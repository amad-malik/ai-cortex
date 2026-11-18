//! `serve_with` against a real database (piece 11 `serve` S4c, mira). DB-ONLY: needs `supervise.sh`'s throwaway Postgres. RED first.
//! The migration sets are the REAL platform set plus a fixture piece's; the pool is the real `cortex_app` pool; the HTTP client is a raw TCP
//! socket against the real listener, so what these cells see is what an operator's `curl` sees.
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::http::Method;
use cortex_config::{Config, Database, DbHost, DbOwner, ModuleList, Paths, Server};
use cortex_db::migrate::{apply, platform_migrations};
use cortex_db::{DbEndpoint, Migration, MigrationSet, OwnerConn};
use cortex_module::{
    AuthError, BoxFuture, CONTRACT_VERSION, DefaultRole, Health, Manifest, Piece, Port, Ports,
    PortsBuilder, Principal, Routes, ScopeClass, ScopeDecl, ScopePolicy, Target, TargetRule,
    TokenVerifier,
};
use cortex_server::assembly::{Wiring, assemble_with};
use cortex_server::fault::Fault;
use cortex_server::serve::{Refused, ServeOptions, no_checks, serve_with};
use cortex_testkit::{TestDb, empty_db};
use cortex_types::{ModuleId, ProblemCodeMinter, ScopeName, Secret, SystemClock};
use mod_identity::BOOTSTRAP_SQL;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::oneshot;

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
    scopes: &[ScopeDecl {
        name: "fx:read",
        class: ScopeClass::Work,
        default_roles: &[
            DefaultRole::ProjectLead,
            DefaultRole::TeamLead,
            DefaultRole::Member,
        ],
    }],
    problems: &[],
};

/// The fixture piece: its migration steps are fixed per test, its health is chosen.
struct Auth {
    steps: &'static [&'static str],
    ready: bool,
}

impl Piece for Auth {
    fn manifest(&self) -> &'static Manifest {
        &AUTH
    }
    fn migrations(&self) -> MigrationSet {
        set_of(self.steps)
    }
    fn provide(&self, ports: &mut PortsBuilder) {
        ports.token_verifier(Arc::new(Verifier));
        ports.scope_policy(Arc::new(Policy));
    }
    fn routes(&self, _ports: &Ports) -> Routes {
        Routes::new().read(
            Method::GET,
            "/v2/fx/ping",
            "fx:read",
            TargetRule::None,
            || async { "pong" },
        )
    }
    fn health(&self) -> BoxFuture<'_, Health> {
        let ready = self.ready;
        Box::pin(async move {
            if ready {
                Health::Ready
            } else {
                Health::NotReady("fixture not ready")
            }
        })
    }
}

fn set_of(steps: &'static [&'static str]) -> MigrationSet {
    MigrationSet {
        module: ModuleId::parse("fx_auth").expect("a module id"),
        schema: "fx_auth",
        steps: steps
            .iter()
            .enumerate()
            .map(|(i, up)| Migration {
                version: i as u32 + 1,
                name: "step",
                up,
                down: "SELECT 1",
            })
            .collect(),
    }
}

/// Roles bootstrapped (as install does); the platform set and `applied` steps of the fixture piece migrated.
async fn database(tag: &str, applied: &'static [&'static str]) -> TestDb {
    let db = empty_db(tag).await;
    let mut admin = db.admin().await;
    sqlx::raw_sql(BOOTSTRAP_SQL)
        .execute(&mut admin)
        .await
        .expect("bootstrap roles");
    sqlx::raw_sql("DO $$ BEGIN CREATE ROLE cortex_install_admin NOLOGIN; EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL; END $$;")
        .execute(&mut admin)
        .await
        .expect("install admin role");
    let mut owner = OwnerConn::connect(&db.owner_config()).await.expect("owner");
    apply(&mut owner, &platform_migrations())
        .await
        .expect("platform");
    if !applied.is_empty() {
        apply(&mut owner, &set_of(applied))
            .await
            .expect("fixture steps");
    }
    db
}

/// The config a real install would hold, with the listener on a free loopback port.
fn config_for(db: &TestDb, name: &str) -> Config {
    let oc = db.owner_config();
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("up-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("dir");
    let pw = dir.join("app.password");
    // The testkit gives the app and the owner role one password.
    fs::write(&pw, format!("{}\n", oc.owner_password.expose())).expect("app password file");
    let (host, port) = match &oc.endpoint {
        DbEndpoint::Socket { dir, port } => (DbHost::SocketDir(dir.clone()), *port),
        DbEndpoint::Tcp { host, port } => (DbHost::Tcp(host.clone()), *port),
    };
    Config {
        paths: Paths {
            state_dir: PathBuf::from("/var/lib/cortex"),
            host_socket: PathBuf::from("/run/cortex/admin.sock"),
            worker_socket: PathBuf::from("/run/cortex/api.sock"),
            token_dir: PathBuf::from("/home/x/.config/cortex"),
        },
        modules: ModuleList {
            enabled: vec![ModuleId::parse("fx_auth").expect("id")],
        },
        database: Some(Database {
            host,
            port,
            name: oc.database.clone(),
            app_user: "cortex_app".into(),
            app_password_file: pw,
            owner: Some(DbOwner {
                user: oc.owner_user.clone(),
                password_file: PathBuf::from("/unused/owner.password"),
            }),
        }),
        server: Server {
            listen: "127.0.0.1:0".parse().expect("an address"),
        },
    }
}

/// A running server, or the refusal it stopped with.
struct Up {
    addr: SocketAddr,
    stop: Option<oneshot::Sender<()>>,
    handle: tokio::task::JoinHandle<Result<(), Refused>>,
    lines: Arc<Mutex<Vec<String>>>,
}

async fn start(
    cfg: Config,
    steps: &'static [&'static str],
    piece_ready: bool,
) -> Result<Up, String> {
    start_with(cfg, steps, piece_ready, Fault::None, None).await
}

async fn start_with(
    cfg: Config,
    steps: &'static [&'static str],
    piece_ready: bool,
    fault: Fault,
    last_good: Option<PathBuf>,
) -> Result<Up, String> {
    let registry = assemble_with(
        vec![Box::new(Auth {
            steps,
            ready: piece_ready,
        })],
        &[ModuleId::parse("fx_auth").expect("id")],
        minter(),
    )
    .expect("assembles");
    let lines = Arc::new(Mutex::new(Vec::new()));
    let sink = lines.clone();
    let (addr_tx, addr_rx) = oneshot::channel();
    let (stop_tx, stop_rx) = oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        serve_with(
            &cfg,
            registry,
            vec!["fx_auth"],
            ServeOptions {
                fault,
                version: "0.0.0-test".to_owned(),
                last_good,
                clock: Arc::new(SystemClock),
                checks: no_checks,
                wiring: Wiring::none(),
            },
            async move {
                let _ = stop_rx.await;
            },
            move |l| sink.lock().expect("lines").push(l.to_owned()),
            move |a| {
                let _ = addr_tx.send(a);
            },
        )
        .await
    });
    match tokio::time::timeout(Duration::from_secs(30), addr_rx).await {
        Ok(Ok(addr)) => Ok(Up {
            addr,
            stop: Some(stop_tx),
            handle,
            lines,
        }),
        _ => {
            assert!(
                handle.is_finished(),
                "the server neither bound nor stopped within 30 s"
            );
            match handle.await {
                Ok(Err(refused)) => Err(refused.to_string()),
                other => panic!("neither bound nor refused: {other:?}"),
            }
        }
    }
}

impl Up {
    async fn stop(mut self) -> Result<(), Refused> {
        let _ = self.stop.take().expect("stop once").send(());
        tokio::time::timeout(Duration::from_secs(15), self.handle)
            .await
            .expect("the server stopped")
            .expect("the task")
    }
    fn lines(&self) -> Vec<String> {
        self.lines.lock().expect("lines").clone()
    }
}

async fn get(addr: SocketAddr, path: &str) -> (u16, String) {
    let mut s = TcpStream::connect(addr).await.expect("connect");
    s.write_all(format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").as_bytes())
        .await
        .expect("write");
    let mut raw = Vec::new();
    s.read_to_end(&mut raw).await.expect("read");
    let text = String::from_utf8_lossy(&raw).into_owned();
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .expect("a status line");
    let body = text.split("\r\n\r\n").nth(1).unwrap_or_default().to_owned();
    (status, body)
}

const STEPS: &[&str] = &["SELECT 1", "SELECT 2"];
const STEPS_CHANGED: &[&str] = &["SELECT 1", "SELECT 99"];
const STEPS_FEWER: &[&str] = &["SELECT 1"];

#[tokio::test]
async fn su1_a_migrated_database_serves_ready_and_stops_cleanly_on_the_signal() {
    let db = database("su1", STEPS).await;
    let up = start(config_for(&db, "su1"), STEPS, true)
        .await
        .expect("serves");
    assert_eq!(
        get(up.addr, "/v2/health/live").await,
        (200, r#"{"ok":true}"#.to_owned())
    );
    assert_eq!(
        get(up.addr, "/v2/health/ready").await,
        (200, r#"{"ok":true}"#.to_owned())
    );
    let (status, body) = get(up.addr, "/v2/health").await;
    assert_eq!(status, 200);
    let doc: Value = serde_json::from_str(&body).expect("the health document");
    assert_eq!(doc["ready"], true);
    assert_eq!(doc["state"], "ready");
    assert_eq!(doc["version"], "0.0.0-test");
    // The piece's route is mounted behind the token check; a path nobody serves is 404.
    assert_eq!(get(up.addr, "/v2/fx/ping").await.0, 401);
    assert_eq!(get(up.addr, "/v2/fx/nothing").await.0, 404);
    assert!(
        doc["last_good_dump_at"].is_null(),
        "no LAST_GOOD configured"
    );
    assert_eq!(doc["backup"], "none");
    assert_eq!(doc["pieces"][0]["name"], "fx_auth");
    assert_eq!(doc["pieces"][0]["enabled"], true);
    assert_eq!(doc["pieces"][0]["ready"], true);
    assert!(
        up.lines().is_empty(),
        "a current server says nothing: {:?}",
        up.lines()
    );
    let addr = up.addr;
    up.stop().await.expect("a clean stop");
    assert!(
        TcpStream::connect(addr).await.is_err(),
        "the listener is closed after the stop"
    );
}

#[tokio::test]
async fn su2_an_unapplied_step_starts_the_server_not_ready_and_says_so_once() {
    let db = database("su2", &[]).await;
    let up = start(config_for(&db, "su2"), STEPS, true)
        .await
        .expect("serves, not ready");
    assert_eq!(
        up.lines(),
        ["serve: starting NOT READY: migration steps are not applied yet"]
    );
    assert_eq!(get(up.addr, "/v2/health/live").await.0, 200, "alive");
    let (status, body) = get(up.addr, "/v2/health/ready").await;
    assert_eq!(status, 503);
    let v: Value = serde_json::from_str(&body).expect("a problem body");
    assert_eq!(v["type"], "capability_unavailable");
    let (_, doc) = get(up.addr, "/v2/health").await;
    let doc: Value = serde_json::from_str(&doc).expect("the health document");
    assert_eq!(doc["ready"], false);
    up.stop().await.expect("a clean stop");
}

#[tokio::test]
async fn su3_a_changed_step_stops_startup_before_anything_listens() {
    let db = database("su3", STEPS).await;
    let e = start(config_for(&db, "su3"), STEPS_CHANGED, true)
        .await
        .err()
        .expect("refused");
    assert_eq!(
        e,
        "serve refused: migration 2 of fx_auth was changed after it was applied"
    );
}

#[tokio::test]
async fn su4_a_database_ahead_of_the_binary_starts_not_ready() {
    let db = database("su4", STEPS).await;
    let up = start(config_for(&db, "su4"), STEPS_FEWER, true)
        .await
        .expect("serves, not ready");
    assert_eq!(
        up.lines(),
        ["serve: starting NOT READY: the database holds migration steps this build does not know"]
    );
    assert_eq!(get(up.addr, "/v2/health/ready").await.0, 503);
    up.stop().await.expect("a clean stop");
}

#[tokio::test]
async fn su5_the_server_becomes_ready_when_the_missing_step_is_applied() {
    let db = database("su5", &[]).await;
    let up = start(config_for(&db, "su5"), STEPS, true)
        .await
        .expect("serves, not ready");
    assert_eq!(get(up.addr, "/v2/health/ready").await.0, 503);
    let mut owner = OwnerConn::connect(&db.owner_config()).await.expect("owner");
    apply(&mut owner, &set_of(STEPS)).await.expect("the steps");
    // The answer is reused for one second; after that the ledger is read again.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if get(up.addr, "/v2/health/ready").await.0 == 200 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "still not ready 10 s after the step was applied"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    up.stop().await.expect("a clean stop");
}

#[tokio::test]
async fn su6_a_piece_that_is_not_ready_keeps_the_server_not_ready() {
    let db = database("su6", STEPS).await;
    let up = start(config_for(&db, "su6"), STEPS, false)
        .await
        .expect("serves");
    assert_eq!(get(up.addr, "/v2/health/ready").await.0, 503);
    let (_, doc) = get(up.addr, "/v2/health").await;
    let doc: Value = serde_json::from_str(&doc).expect("the health document");
    assert_eq!(doc["pieces"][0]["enabled"], true);
    assert_eq!(doc["pieces"][0]["ready"], false);
    assert_eq!(get(up.addr, "/v2/health/live").await.0, 200);
    up.stop().await.expect("a clean stop");
}

#[tokio::test]
async fn su7_the_test_fault_of_the_build_reaches_the_ready_route_and_only_that() {
    let db = database("su7", STEPS).await;
    let up = start_with(config_for(&db, "su7"), STEPS, true, Fault::Ready503, None)
        .await
        .expect("serves");
    assert_eq!(get(up.addr, "/v2/health/live").await.0, 200);
    assert_eq!(get(up.addr, "/v2/health/ready").await.0, 503);
    assert_eq!(get(up.addr, "/v2/health").await.0, 200);
    up.stop().await.expect("a clean stop");
}

#[tokio::test]
async fn su8_the_last_good_file_that_is_given_reaches_the_health_document() {
    let db = database("su8", STEPS).await;
    let file = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("up-lastgood-{}", std::process::id()));
    fs::write(&file, r#"{"utc":"2026-11-06T10:00:00Z"}"#).expect("LAST_GOOD");
    let up = start_with(config_for(&db, "su8"), STEPS, true, Fault::None, Some(file))
        .await
        .expect("serves");
    let (_, body) = get(up.addr, "/v2/health").await;
    let doc: Value = serde_json::from_str(&body).expect("the health document");
    assert!(
        doc["last_good_dump_at"]
            .as_str()
            .is_some_and(|t| t.starts_with("2026-11-06T10:00:00")),
        "{doc}"
    );
    assert_ne!(doc["backup"], "none");
    up.stop().await.expect("a clean stop");
}

#[tokio::test]
async fn su9_the_configured_listen_address_is_the_one_bound() {
    let db = database("su9", STEPS).await;
    // A port that was free a moment ago.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("a free port")
        .local_addr()
        .expect("addr")
        .port();
    let mut cfg = config_for(&db, "su9");
    cfg.server.listen = format!("127.0.0.1:{port}").parse().expect("an address");
    let up = start(cfg, STEPS, true).await.expect("serves");
    assert_eq!(
        up.addr,
        format!("127.0.0.1:{port}").parse().expect("an address")
    );
    assert_eq!(get(up.addr, "/v2/health/live").await.0, 200);
    up.stop().await.expect("a clean stop");
}

#[tokio::test]
async fn su10_a_platform_step_changed_after_it_was_applied_stops_startup_too() {
    let db = database("su10", STEPS).await;
    let mut admin = db.admin().await;
    sqlx::raw_sql("UPDATE platform.migrations SET checksum = 'tampered' WHERE module = 'platform' AND version = 1")
        .execute(&mut admin)
        .await
        .expect("tamper with the ledger");
    let e = start(config_for(&db, "su10"), STEPS, true)
        .await
        .err()
        .expect("refused");
    assert_eq!(
        e,
        "serve refused: migration 1 of platform was changed after it was applied"
    );
}
