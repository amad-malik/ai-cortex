//! `serve::serve_with`: what it refuses before anything listens, without a database (piece 11 `serve` S4c, mira). DB-free. RED first.
//! A fixture registry stands in for the real pieces (which cannot assemble until mod-audit and mod-tokens merge).
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use cortex_config::{Env, load};
use cortex_db::MigrationSet;
use cortex_module::{
    AuthError, BoxFuture, CONTRACT_VERSION, Health, Manifest, Piece, Port, Ports, PortsBuilder,
    Principal, Registry, Routes, ScopePolicy, Target, TargetRule, TokenVerifier,
};
use cortex_server::assembly::{Wiring, assemble_with};
use cortex_server::fault::Fault;
use cortex_server::serve::{Refused, ServeOptions, no_checks, serve_with};
use cortex_types::{ModuleId, ProblemCodeMinter, ScopeName, Secret, SystemClock};

fn minter() -> &'static ProblemCodeMinter {
    static M: OnceLock<ProblemCodeMinter> = OnceLock::new();
    M.get_or_init(|| ProblemCodeMinter::take().expect("the first take in this process"))
}

struct Verifier;
#[async_trait]
impl TokenVerifier for Verifier {
    async fn verify(&self, _bearer: &Secret) -> Result<Principal, AuthError> {
        Err(AuthError::Unknown)
    }
}
struct Policy;
impl ScopePolicy for Policy {
    fn allows(&self, _p: &Principal, _scope: &ScopeName) -> bool {
        false
    }
    fn target(&self, _p: &Principal, _rule: TargetRule, _t: &Target) -> bool {
        false
    }
}

static AUTH: Manifest = Manifest {
    id: "fx_auth",
    version: "0.1.0",
    contract: CONTRACT_VERSION,
    core: true,
    schema: "fx_auth",
    provides: &[Port::TokenVerifier, Port::ScopePolicy],
    uses: &[],
    scopes: &[],
    problems: &[],
};

struct Auth;
impl Piece for Auth {
    fn manifest(&self) -> &'static Manifest {
        &AUTH
    }
    fn migrations(&self) -> MigrationSet {
        MigrationSet {
            module: ModuleId::parse("fx_auth").expect("a module id"),
            schema: "fx_auth",
            steps: vec![],
        }
    }
    fn provide(&self, ports: &mut PortsBuilder) {
        ports.token_verifier(Arc::new(Verifier));
        ports.scope_policy(Arc::new(Policy));
    }
    fn routes(&self, _ports: &Ports) -> Routes {
        Routes::new()
    }
    fn health(&self) -> BoxFuture<'_, Health> {
        Box::pin(async { Health::Ready })
    }
}

fn registry() -> Registry {
    let enabled = [ModuleId::parse("fx_auth").expect("a module id")];
    assemble_with(vec![Box::new(Auth)], &enabled, minter()).expect("assembles")
}

fn options() -> ServeOptions {
    ServeOptions {
        fault: Fault::None,
        version: "0.0.0-test".to_owned(),
        last_good: None,
        clock: Arc::new(SystemClock),
        checks: no_checks,
        wiring: Wiring::none(),
    }
}

fn dir(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("sw-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).expect("dir");
    d
}

fn config(name: &str, database: Option<(&str, u16, &PathBuf)>) -> cortex_config::Config {
    let d = dir(name);
    let mut text = String::from("[modules]\nenabled = [\"fx_auth\"]\n");
    if let Some((host, port, pw)) = database {
        text += &format!(
            "[database]\nhost = \"{host}\"\nport = {port}\nname = \"cortex\"\napp_user = \"cortex_app\"\napp_password_file = \"{}\"\n",
            pw.display()
        );
    }
    let f = d.join("cortex.toml");
    fs::write(&f, text).expect("config");
    load(&f, &Env::from_pairs(&[("HOME", "/home/x")])).expect("a valid config")
}

/// Runs `serve_with` and counts what it told the caller.
async fn run(cfg: &cortex_config::Config) -> (Result<(), String>, usize, usize) {
    let lines = Arc::new(AtomicUsize::new(0));
    let bounds = Arc::new(AtomicUsize::new(0));
    let (l, b) = (lines.clone(), bounds.clone());
    let r = serve_with(
        cfg,
        registry(),
        vec!["fx_auth"],
        options(),
        std::future::pending::<()>(),
        move |_| {
            l.fetch_add(1, Ordering::SeqCst);
        },
        move |_| {
            b.fetch_add(1, Ordering::SeqCst);
        },
    )
    .await
    .map_err(|e| e.to_string());
    (
        r,
        lines.load(Ordering::SeqCst),
        bounds.load(Ordering::SeqCst),
    )
}

#[tokio::test]
async fn sw1_a_config_without_a_database_table_is_refused_before_anything_listens() {
    let (r, lines, bounds) = run(&config("nodb", None)).await;
    assert_eq!(r, Err("serve refused: missing: database".to_owned()));
    assert_eq!((lines, bounds), (0, 0));
}

#[tokio::test]
async fn sw2_an_unreadable_password_file_is_refused_without_the_path() {
    let missing = dir("pwmissing").join("canary-pw-dir/app.pw");
    let (r, _, bounds) = run(&config("pw", Some(("127.0.0.1", 1, &missing)))).await;
    let e = r.expect_err("refused");
    assert_eq!(
        e,
        "serve refused: the database password file could not be read"
    );
    assert!(!e.contains("canary-pw-dir"));
    assert_eq!(bounds, 0);
}

#[tokio::test]
async fn sw3_a_database_nothing_listens_on_is_refused_without_the_host_or_the_password() {
    let pw = dir("pwok").join("app.pw");
    fs::write(&pw, "canary-db-password\n").expect("pw");
    // Port 1 on loopback: the connection is refused at once.
    let started = std::time::Instant::now();
    let (r, lines, bounds) = run(&config("dead", Some(("127.0.0.1", 1, &pw)))).await;
    let e = r.expect_err("refused");
    assert_eq!(e, "serve refused: the database could not be reached");
    assert!(!e.contains("canary-db-password") && !e.contains("127.0.0.1"));
    assert_eq!((lines, bounds), (0, 0), "nothing was announced or bound");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(30),
        "refused, not hung"
    );
}

fn refuse(_registry: &Registry) -> Result<(), Refused> {
    Err(Refused("serve refused: the check said no".to_owned()))
}

/// Like `run`, with the options' checks replaced.
async fn run_with_checks(
    cfg: &cortex_config::Config,
    checks: fn(&Registry) -> Result<(), Refused>,
) -> (Result<(), String>, usize, usize) {
    let lines = Arc::new(AtomicUsize::new(0));
    let bounds = Arc::new(AtomicUsize::new(0));
    let (l, b) = (lines.clone(), bounds.clone());
    let mut opts = options();
    opts.checks = checks;
    let r = serve_with(
        cfg,
        registry(),
        vec!["fx_auth"],
        opts,
        std::future::pending::<()>(),
        move |_| {
            l.fetch_add(1, Ordering::SeqCst);
        },
        move |_| {
            b.fetch_add(1, Ordering::SeqCst);
        },
    )
    .await
    .map_err(|e| e.to_string());
    (
        r,
        lines.load(Ordering::SeqCst),
        bounds.load(Ordering::SeqCst),
    )
}

#[tokio::test]
async fn sw4_the_checks_of_the_options_run_first_and_a_refusal_stops_everything_after() {
    // The config has NO [database] table: after a PASSING check the next refusal is `missing: database`; a REFUSING check comes before it.
    let cfg = config("checks", None);
    let (r, lines, bounds) = run_with_checks(&cfg, no_checks).await;
    assert_eq!(
        r,
        Err("serve refused: missing: database".to_owned()),
        "a passing check lets the start go on"
    );
    assert_eq!((lines, bounds), (0, 0));
    let (r, lines, bounds) = run_with_checks(&cfg, refuse).await;
    assert_eq!(
        r,
        Err("serve refused: the check said no".to_owned()),
        "the check's refusal comes BEFORE the pool configuration"
    );
    assert_eq!((lines, bounds), (0, 0), "nothing announced, nothing bound");
}
