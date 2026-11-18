//! DEFECT S (cox 7615a247, kai's real reboot): `cortex-server migrate` waits for its database. After a reboot the one-shot starts about a second
//! after the database, which still says `the database system is starting up`; today the first failed connect is the answer and the container stays
//! `exited 1` for good. The cells: a database that answers only after migrate started (refused first, and a real 57P03 first), one that never comes
//! (the same line and code after the bound), a refused login or a missing database or role that fails AT ONCE, a ready database that costs nothing, SIGTERM
//! during the wait, and the knob. wt_01, wt_02, wt_04 and wt_05 need `supervise.sh`'s throwaway Postgres; wt_03 and wt_06 to wt_08 are DB-free.
//! RED first. The existing cells of `arms.rs` and `serve_real.rs` are not touched: `migrate_with` keeps its meaning (no wait).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use cortex_config::{Config, Database, DbHost, DbOwner, ModuleList, Paths, Server};
use cortex_db::{DbEndpoint, MigrationSet, ModuleTx};
use cortex_module::{
    AuditDetails, AuditSink, AuditTarget, BoxFuture, CONTRACT_VERSION, Health, Manifest, Piece,
    Port, PortError, Ports, PortsBuilder, Registry, Routes,
};
use cortex_server::assembly::assemble_with;
use cortex_server::cli::{EXIT_FAILURE, EXIT_OK};
use cortex_server::migrate_cmd::{DbWait, migrate_with_wait};
use cortex_testkit::{TestDb, empty_db};
use cortex_types::{ModuleId, ProblemCodeMinter};
use mod_identity::{BOOTSTRAP_SQL, ModIdentity};
use mod_tokens::ModTokens;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpSocket, TcpStream};

/// The one line a connection that never came has always had.
const REFUSED: &str = "migrate failed: could not connect to the database as the owner";
/// The first progress line.
const FIRST_WAIT: &str = "migrate: waiting for the database (it is not ready yet)";
const KNOB: &str = "CORTEX_MIGRATE_DB_WAIT_SECS";
const KNOB_REFUSED: &str =
    "migrate refused: CORTEX_MIGRATE_DB_WAIT_SECS is not a whole number of seconds from 0 to 3600";

/// A policy that runs in milliseconds.
fn fast(bound_ms: u64) -> DbWait {
    DbWait {
        bound: Duration::from_millis(bound_ms),
        first: Duration::from_millis(10),
        cap: Duration::from_millis(50),
        note_every: Duration::from_secs(30),
    }
}

/// The same, with a progress line every `note_ms` of elapsed time.
fn paced(bound_ms: u64, note_ms: u64) -> DbWait {
    DbWait {
        note_every: Duration::from_millis(note_ms),
        ..fast(bound_ms)
    }
}

fn minter() -> &'static ProblemCodeMinter {
    static M: OnceLock<ProblemCodeMinter> = OnceLock::new();
    M.get_or_init(|| ProblemCodeMinter::take().expect("the first take in this process"))
}

// ---- the registry (the one of arms.rs: the real identity and tokens and a fixture audit piece) ---------------------------------

static FX_AUDIT: Manifest = Manifest {
    id: "fx_audit",
    version: "0.1.0",
    contract: CONTRACT_VERSION,
    core: true,
    schema: "fx_audit",
    provides: &[Port::Audit],
    uses: &[],
    scopes: &[],
    problems: &[],
};

struct Sink;

#[async_trait]
impl AuditSink for Sink {
    async fn record(
        &self,
        _tx: &mut ModuleTx,
        _action: &'static str,
        _target: AuditTarget,
        _details: AuditDetails,
    ) -> Result<(), PortError> {
        Ok(())
    }
}

struct FxAudit;

impl Piece for FxAudit {
    fn manifest(&self) -> &'static Manifest {
        &FX_AUDIT
    }
    fn migrations(&self) -> MigrationSet {
        MigrationSet {
            module: ModuleId::parse("fx_audit").expect("a module id"),
            schema: "fx_audit",
            steps: vec![],
        }
    }
    fn provide(&self, ports: &mut PortsBuilder) {
        ports.audit(Arc::new(Sink));
    }
    fn routes(&self, _ports: &Ports) -> Routes {
        Routes::new()
    }
    fn health(&self) -> BoxFuture<'_, Health> {
        Box::pin(async { Health::Ready })
    }
}

fn registry() -> (Registry, Vec<ModuleId>) {
    let pieces: Vec<Box<dyn Piece>> = vec![
        Box::new(FxAudit),
        Box::new(ModIdentity::new()),
        Box::new(ModTokens::new()),
    ];
    let enabled = vec![
        ModuleId::parse("fx_audit").expect("id"),
        ModuleId::parse("identity").expect("id"),
        ModuleId::parse("tokens").expect("id"),
    ];
    let reg = assemble_with(pieces, &enabled, minter()).unwrap_or_else(|e| {
        panic!(
            "the fixture registry must assemble: {}",
            cortex_server::assembly::startup_line(&e)
        )
    });
    (reg, enabled)
}

static BAD_PIECE: Manifest = Manifest {
    id: "zz",
    version: "0.1.0",
    contract: CONTRACT_VERSION,
    core: false,
    schema: "zz",
    provides: &[],
    uses: &[],
    scopes: &[],
    problems: &[],
};

/// A piece whose one migration is not SQL: the connection is made, the migration fails.
struct BadPiece;

impl Piece for BadPiece {
    fn manifest(&self) -> &'static Manifest {
        &BAD_PIECE
    }
    fn migrations(&self) -> MigrationSet {
        MigrationSet {
            module: ModuleId::parse("zz").expect("a module id"),
            schema: "zz",
            steps: vec![cortex_db::Migration {
                version: 1,
                name: "bad",
                up: "NOT SQL",
                down: "",
            }],
        }
    }
    fn provide(&self, _ports: &mut PortsBuilder) {}
    fn routes(&self, _ports: &Ports) -> Routes {
        Routes::new()
    }
    fn health(&self) -> BoxFuture<'_, Health> {
        Box::pin(async { Health::Ready })
    }
}

fn registry_bad() -> (Registry, Vec<ModuleId>) {
    let pieces: Vec<Box<dyn Piece>> = vec![Box::new(FxAudit), Box::new(BadPiece)];
    let enabled = vec![
        ModuleId::parse("fx_audit").expect("id"),
        ModuleId::parse("zz").expect("id"),
    ];
    let reg = assemble_with(pieces, &enabled, minter()).unwrap_or_else(|e| {
        panic!(
            "the failing registry must assemble: {}",
            cortex_server::assembly::startup_line(&e)
        )
    });
    (reg, enabled)
}

// ---- the database and the configs -------------------------------------------------------------------------------------------

async fn setup(tag: &str) -> TestDb {
    let db = empty_db(tag).await;
    let mut admin = db.admin().await;
    for sql in [BOOTSTRAP_SQL, mod_tokens::BOOTSTRAP_SQL] {
        sqlx::raw_sql(sql)
            .execute(&mut admin)
            .await
            .expect("module roles");
    }
    sqlx::raw_sql("DO $$ BEGIN CREATE ROLE cortex_install_admin NOLOGIN; EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL; END $$;")
        .execute(&mut admin)
        .await
        .expect("install admin role");
    db
}

fn pg_port(db: &TestDb) -> u16 {
    match db.owner_config().endpoint {
        DbEndpoint::Tcp { port, .. } => port,
        DbEndpoint::Socket { port, .. } => port,
    }
}

fn dir_for(tag: &str) -> PathBuf {
    let dir =
        PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("mw-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("dir");
    dir
}

/// A config over TCP 127.0.0.1:`port` with both password files readable.
fn cfg_for(
    tag: &str,
    port: u16,
    name: &str,
    owner_user: &str,
    owner_password: &str,
    enabled: &[ModuleId],
) -> Config {
    cfg_host(
        tag,
        "127.0.0.1",
        port,
        name,
        owner_user,
        owner_password,
        enabled,
    )
}

fn cfg_host(
    tag: &str,
    host: &str,
    port: u16,
    name: &str,
    owner_user: &str,
    owner_password: &str,
    enabled: &[ModuleId],
) -> Config {
    let dir = dir_for(tag);
    let owner_pw = dir.join("owner.password");
    fs::write(&owner_pw, format!("{owner_password}\n")).expect("owner file");
    let app_pw = dir.join("app.password");
    fs::write(&app_pw, "app-secret-canary\n").expect("app file");
    Config {
        paths: Paths {
            state_dir: PathBuf::from("/var/lib/cortex"),
            host_socket: PathBuf::from("/run/cortex/admin.sock"),
            worker_socket: PathBuf::from("/run/cortex/api.sock"),
            token_dir: PathBuf::from("/home/x/.config/cortex"),
        },
        modules: ModuleList {
            enabled: enabled.to_vec(),
        },
        database: Some(Database {
            host: DbHost::Tcp(host.into()),
            port,
            name: name.into(),
            app_user: "cortex_app".into(),
            app_password_file: app_pw,
            owner: Some(DbOwner {
                user: owner_user.into(),
                password_file: owner_pw,
            }),
        }),
        server: Server::default(),
    }
}

/// The config of the throwaway database itself, through `port` (its own, or a stand-in's).
fn cfg_via(db: &TestDb, tag: &str, port: u16, enabled: &[ModuleId]) -> Config {
    let oc = db.owner_config();
    cfg_for(
        tag,
        port,
        &oc.database,
        &oc.owner_user,
        oc.owner_password.expose(),
        enabled,
    )
}

async fn ledger_modules(db: &TestDb) -> Vec<String> {
    let mut admin = db.admin().await;
    sqlx::query_scalar("SELECT DISTINCT module FROM platform.migrations ORDER BY 1")
        .fetch_all(&mut admin)
        .await
        .expect("ledger")
}

// ---- the stand-ins ----------------------------------------------------------------------------------------------------------

/// A port with NOBODY listening: a socket bound and never `listen`ed on refuses every connection, and holds the port for us.
fn reserve() -> (TcpSocket, u16) {
    let s = TcpSocket::new_v4().expect("a socket");
    s.set_reuseaddr(true).expect("reuseaddr");
    s.bind("127.0.0.1:0".parse().expect("addr")).expect("bind");
    let port = s.local_addr().expect("local addr").port();
    (s, port)
}

/// One whole PostgreSQL frontend message body (after its length word), the first one without a type byte (startup packets).
async fn read_startup(s: &mut TcpStream) -> std::io::Result<Vec<u8>> {
    let mut len = [0u8; 4];
    s.read_exact(&mut len).await?;
    let n = i32::from_be_bytes(len) as usize;
    let mut body = vec![0u8; n.saturating_sub(4)];
    s.read_exact(&mut body).await?;
    Ok(body)
}

/// What a real PostgreSQL sends a client that connects while it starts: SSLRequest answered `N`, then FATAL 57P03.
async fn starting_up(mut s: TcpStream) {
    let Ok(mut first) = read_startup(&mut s).await else {
        return;
    };
    if first == [0x04, 0xd2, 0x16, 0x2f] {
        let _ = s.write_all(b"N").await;
        first = match read_startup(&mut s).await {
            Ok(b) => b,
            Err(_) => return,
        };
    }
    let _ = first;
    let mut body = Vec::new();
    for (k, v) in [
        (b'S', "FATAL"),
        (b'V', "FATAL"),
        (b'C', "57P03"),
        (b'M', "the database system is starting up"),
    ] {
        body.push(k);
        body.extend_from_slice(v.as_bytes());
        body.push(0);
    }
    body.push(0);
    let mut msg = vec![b'E'];
    msg.extend_from_slice(&(body.len() as i32 + 4).to_be_bytes());
    msg.extend_from_slice(&body);
    let _ = s.write_all(&msg).await;
    let _ = s.shutdown().await;
}

/// Accept on `listener`: the first `starting` connections get 57P03, every later one is forwarded to the real database on `real`.
async fn serve(listener: TcpListener, real: u16, starting: usize, hits: Arc<AtomicUsize>) {
    let mut n = 0usize;
    loop {
        let Ok((client, _)) = listener.accept().await else {
            return;
        };
        n += 1;
        hits.fetch_add(1, Ordering::SeqCst);
        if n <= starting {
            tokio::spawn(starting_up(client));
        } else {
            tokio::spawn(async move {
                let Ok(mut upstream) = TcpStream::connect(("127.0.0.1", real)).await else {
                    return;
                };
                let mut client = client;
                let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
            });
        }
    }
}

/// `migrate_with_wait` under a ceiling: a call that does not return is a FAILED cell, not a hung run.
async fn run_bounded(
    cfg: &Config,
    reg: &Registry,
    wait: DbWait,
    notes: &mut Vec<String>,
) -> cortex_server::bootstrap::Outcome {
    match tokio::time::timeout(
        Duration::from_secs(20),
        migrate_with_wait(cfg, reg, wait, &mut |l| notes.push(l.to_owned())),
    )
    .await
    {
        Ok(o) => o,
        Err(_) => panic!(
            "migrate_with_wait did not return within its ceiling (bound {:?})",
            wait.bound
        ),
    }
}

// ---- the cells --------------------------------------------------------------------------------------------------------------

/// A database that answers only AFTER the command started: nothing listens for a few hundred milliseconds, then the real database is reachable.
#[tokio::test]
async fn wt_01_a_database_that_is_reachable_only_after_migrate_started_is_migrated_and_exits_0() {
    let db = setup("wt01").await;
    let (sock, port) = reserve();
    let (reg, enabled) = registry();
    let cfg = cfg_via(&db, "wt01", port, &enabled);
    let real = pg_port(&db);
    let late = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(400)).await;
        let listener = sock.listen(16).expect("listen");
        serve(listener, real, 0, Arc::default()).await;
    });
    let mut notes: Vec<String> = Vec::new();
    let t0 = Instant::now();
    let o = run_bounded(&cfg, &reg, fast(30_000), &mut notes).await;
    late.abort();
    assert_eq!(
        (o.code, o.stderr.clone()),
        (EXIT_OK, None),
        "{o:?} {notes:?}"
    );
    assert!(
        t0.elapsed() >= Duration::from_millis(350),
        "it cannot have connected before the listener existed: {:?}",
        t0.elapsed()
    );
    assert_eq!(
        notes,
        vec![FIRST_WAIT.to_owned()],
        "one line, at the first failed attempt"
    );
    assert_eq!(
        ledger_modules(&db).await,
        vec![
            "identity".to_owned(),
            "platform".to_owned(),
            "tokens".to_owned()
        ]
    );
}

/// The same, with the bytes a real PostgreSQL sends while it starts: the first two connections get FATAL 57P03.
#[tokio::test]
async fn wt_02_a_database_that_says_it_is_starting_up_is_waited_for_and_then_migrated() {
    let db = setup("wt02").await;
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let (reg, enabled) = registry();
    let cfg = cfg_via(&db, "wt02", port, &enabled);
    let real = pg_port(&db);
    let stand_in = tokio::spawn(serve(listener, real, 2, Arc::default()));
    let mut notes: Vec<String> = Vec::new();
    let o = run_bounded(&cfg, &reg, fast(30_000), &mut notes).await;
    stand_in.abort();
    assert_eq!(
        (o.code, o.stderr.clone()),
        (EXIT_OK, None),
        "{o:?} {notes:?}"
    );
    assert_eq!(notes, vec![FIRST_WAIT.to_owned()]);
    assert_eq!(
        ledger_modules(&db).await,
        vec![
            "identity".to_owned(),
            "platform".to_owned(),
            "tokens".to_owned()
        ]
    );
}

/// A database that never comes: the same line and the same code as ever, after the bound, with at most the first progress line.
#[tokio::test]
async fn wt_03_a_database_that_never_comes_fails_after_the_bound_with_the_same_line_and_code() {
    let (_held, port) = reserve();
    let (reg, enabled) = registry();
    let cfg = cfg_for(
        "wt03",
        port,
        "cortex",
        "cortex_owner",
        "owner-secret-canary",
        &enabled,
    );
    let mut notes: Vec<String> = Vec::new();
    let t0 = Instant::now();
    let o = run_bounded(&cfg, &reg, fast(1_000), &mut notes).await;
    let took = t0.elapsed();
    assert_eq!(o.code, EXIT_FAILURE, "{o:?}");
    assert_eq!(o.stderr.as_deref(), Some(REFUSED), "{o:?}");
    assert!(
        took >= Duration::from_millis(950) && took < Duration::from_secs(10),
        "it waited the bound, not less and not forever: {took:?}"
    );
    assert_eq!(notes, vec![FIRST_WAIT.to_owned()], "{notes:?}");
    for l in notes.iter().chain(o.stderr.iter()) {
        assert!(
            !l.contains("secret-canary")
                && !l.contains("127.0.0.1")
                && !l.contains(&port.to_string()),
            "{l}"
        );
    }
}

/// A refused login, a missing database and a missing role are a misconfiguration: they fail AT ONCE even with a long bound, and say nothing about waiting.
#[tokio::test]
async fn wt_04_a_wrong_password_a_missing_database_and_a_missing_role_fail_at_once() {
    let db = setup("wt04").await;
    let oc = db.owner_config();
    let real = pg_port(&db);
    let (_reg0, enabled) = registry();
    let cases = [
        (
            "password",
            oc.database.clone(),
            oc.owner_user.clone(),
            "not-the-password".to_owned(),
        ),
        (
            "database",
            "no_such_database_wt04".to_owned(),
            oc.owner_user.clone(),
            oc.owner_password.expose().to_owned(),
        ),
        (
            "role",
            oc.database.clone(),
            "no_such_owner_wt04".to_owned(),
            oc.owner_password.expose().to_owned(),
        ),
    ];
    for (what, name, user, pw) in cases {
        let (reg, _) = registry();
        let cfg = cfg_for(&format!("wt04{what}"), real, &name, &user, &pw, &enabled);
        let mut notes: Vec<String> = Vec::new();
        let t0 = Instant::now();
        let o = run_bounded(&cfg, &reg, fast(60_000), &mut notes).await;
        assert_eq!(o.code, EXIT_FAILURE, "{what}: {o:?}");
        assert_eq!(o.stderr.as_deref(), Some(REFUSED), "{what}: {o:?}");
        assert!(
            t0.elapsed() < Duration::from_secs(5),
            "{what}: it must not wait: {:?}",
            t0.elapsed()
        );
        assert!(notes.is_empty(), "{what}: no wait line: {notes:?}");
    }
    // and `connect` itself is as it was: the refused login is `Unavailable`, the reason is not leaked
    let mut oc = db.owner_config();
    oc.owner_password = cortex_types::Secret::new("not-the-password".to_owned());
    assert_eq!(
        cortex_db::OwnerConn::connect(&oc).await.err(),
        Some(cortex_db::DbError::Unavailable)
    );
}

/// The first-install path: a database that is ready costs no wait and prints nothing.
#[tokio::test]
async fn wt_05_a_database_that_is_ready_costs_no_wait_and_prints_no_wait_line() {
    let db = setup("wt05").await;
    let (reg, enabled) = registry();
    let cfg = cfg_via(&db, "wt05", pg_port(&db), &enabled);
    let mut notes: Vec<String> = Vec::new();
    let t0 = Instant::now();
    let o = run_bounded(&cfg, &reg, fast(60_000), &mut notes).await;
    assert_eq!((o.code, o.stderr.clone()), (EXIT_OK, None), "{o:?}");
    assert!(notes.is_empty(), "{notes:?}");
    assert!(t0.elapsed() < Duration::from_secs(20), "{:?}", t0.elapsed());
}

/// An address that accepts the connection and never speaks (a half-open path): one attempt must not outlive the bound.
#[tokio::test]
async fn wt_09_an_address_that_accepts_and_never_answers_still_ends_at_the_bound() {
    let silent = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = silent.local_addr().expect("addr").port();
    let keep = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((s, _)) = silent.accept().await {
            held.push(s);
        }
    });
    let (reg, enabled) = registry();
    let cfg = cfg_for(
        "wt09",
        port,
        "cortex",
        "cortex_owner",
        "owner-secret-canary",
        &enabled,
    );
    let mut notes: Vec<String> = Vec::new();
    let t0 = Instant::now();
    let o = run_bounded(&cfg, &reg, fast(1_000), &mut notes).await;
    keep.abort();
    assert_eq!(o.code, EXIT_FAILURE, "{o:?}");
    assert_eq!(o.stderr.as_deref(), Some(REFUSED), "{o:?}");
    assert!(
        t0.elapsed() >= Duration::from_millis(950) && t0.elapsed() < Duration::from_secs(5),
        "the bound holds even when an attempt never answers: {:?}",
        t0.elapsed()
    );
}

/// The bound is ONE absolute deadline: a sleep that would cross it is cut short, never slept in full.
#[tokio::test]
async fn wt_10_a_sleep_that_would_cross_the_deadline_is_cut_at_the_deadline() {
    let (_held, port) = reserve();
    let (reg, enabled) = registry();
    let cfg = cfg_for("wt10", port, "cortex", "cortex_owner", "x", &enabled);
    let wait = DbWait {
        bound: Duration::from_millis(300),
        first: Duration::from_secs(10),
        cap: Duration::from_secs(10),
        note_every: Duration::from_secs(30),
    };
    let mut notes: Vec<String> = Vec::new();
    let t0 = Instant::now();
    let o = run_bounded(&cfg, &reg, wait, &mut notes).await;
    assert_eq!(o.stderr.as_deref(), Some(REFUSED), "{o:?}");
    assert!(
        t0.elapsed() >= Duration::from_millis(250) && t0.elapsed() < Duration::from_secs(3),
        "the 10 s step is clamped to the remaining time: {:?}",
        t0.elapsed()
    );
}

/// Failures that keep coming back quickly (the database says `starting up` every time) never refresh the deadline.
#[tokio::test]
async fn wt_11_failures_that_keep_coming_never_extend_the_deadline() {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let hits = Arc::new(AtomicUsize::new(0));
    let stand_in = tokio::spawn(serve(listener, 1, usize::MAX, hits.clone()));
    let (reg, enabled) = registry();
    let cfg = cfg_for("wt11", port, "cortex", "cortex_owner", "x", &enabled);
    let mut notes: Vec<String> = Vec::new();
    let t0 = Instant::now();
    let o = run_bounded(&cfg, &reg, fast(1_000), &mut notes).await;
    stand_in.abort();
    assert_eq!(o.stderr.as_deref(), Some(REFUSED), "{o:?}");
    assert!(
        t0.elapsed() >= Duration::from_millis(950) && t0.elapsed() < Duration::from_secs(5),
        "{:?}",
        t0.elapsed()
    );
    assert!(
        hits.load(Ordering::SeqCst) >= 3,
        "it kept trying until the deadline"
    );
}

/// Progress is driven by ELAPSED time: a long pending attempt still produces lines, and the lines name no host, port or user.
#[tokio::test]
async fn wt_12_progress_lines_follow_elapsed_time_even_while_one_attempt_is_pending() {
    let silent = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = silent.local_addr().expect("addr").port();
    let keep = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((s, _)) = silent.accept().await {
            held.push(s);
        }
    });
    let (reg, enabled) = registry();
    let cfg = cfg_for("wt12", port, "cortex", "cortex_owner", "x", &enabled);
    let mut notes: Vec<String> = Vec::new();
    let o = run_bounded(&cfg, &reg, paced(1_000, 300), &mut notes).await;
    keep.abort();
    assert_eq!(o.stderr.as_deref(), Some(REFUSED), "{o:?}");
    assert!(
        (2..=5).contains(&notes.len()),
        "a line at 300, 600 and 900 ms of a 1 s bound: {notes:?}"
    );
    assert_eq!(notes[0], FIRST_WAIT);
    for l in &notes[1..] {
        assert!(
            l.starts_with("migrate: still waiting for the database ("),
            "{l}"
        );
    }
    for l in &notes {
        assert!(
            !l.contains("127.0.0.1")
                && !l.contains(&port.to_string())
                && !l.contains("cortex_owner")
                && !l.contains("57P03"),
            "{l}"
        );
    }
}

/// An authentication failure ends the retries at once, even after transient errors: two `starting up` answers, then the real database refuses the password.
#[tokio::test]
async fn wt_13_a_refused_login_after_transient_errors_ends_the_retries_at_once() {
    let db = setup("wt13").await;
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let hits = Arc::new(AtomicUsize::new(0));
    let stand_in = tokio::spawn(serve(listener, pg_port(&db), 2, hits.clone()));
    let (reg, enabled) = registry();
    let oc = db.owner_config();
    let cfg = cfg_for(
        "wt13",
        port,
        &oc.database,
        &oc.owner_user,
        "not-the-password",
        &enabled,
    );
    let mut notes: Vec<String> = Vec::new();
    let t0 = Instant::now();
    let o = run_bounded(&cfg, &reg, fast(60_000), &mut notes).await;
    stand_in.abort();
    assert_eq!(o.stderr.as_deref(), Some(REFUSED), "{o:?}");
    assert!(t0.elapsed() < Duration::from_secs(10), "{:?}", t0.elapsed());
    assert_eq!(
        hits.load(Ordering::SeqCst),
        3,
        "two transient answers and ONE login attempt: no retry after a classified authentication error"
    );
}

/// After a connection exists nothing is retried: a migration that fails returns at once, over ONE connection, and is never rerun.
#[tokio::test]
async fn wt_14_a_migration_that_fails_after_a_good_connection_is_attempted_once() {
    let db = setup("wt14").await;
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let hits = Arc::new(AtomicUsize::new(0));
    let stand_in = tokio::spawn(serve(listener, pg_port(&db), 0, hits.clone()));
    let (reg, enabled) = registry_bad();
    let cfg = cfg_via(&db, "wt14", port, &enabled);
    let mut notes: Vec<String> = Vec::new();
    let t0 = Instant::now();
    let o = run_bounded(&cfg, &reg, fast(60_000), &mut notes).await;
    stand_in.abort();
    assert_eq!(o.code, EXIT_FAILURE, "{o:?}");
    assert_eq!(
        o.stderr.as_deref(),
        Some("migrate failed: module zz, migration 1"),
        "{o:?}"
    );
    assert!(t0.elapsed() < Duration::from_secs(10), "{:?}", t0.elapsed());
    assert_eq!(
        hits.load(Ordering::SeqCst),
        1,
        "one connection, one attempt"
    );
    assert!(notes.is_empty(), "{notes:?}");
}

/// A host name that does not resolve is the resolver's failure, identified by our own lookup: it is waited for like any other not-ready address.
#[tokio::test]
async fn wt_15_a_host_name_that_does_not_resolve_is_waited_for_up_to_the_bound() {
    let (reg, enabled) = registry();
    // `.invalid` never resolves (RFC 6761)
    let cfg = cfg_host(
        "wt15",
        "wt15.invalid",
        5432,
        "cortex",
        "cortex_owner",
        "x",
        &enabled,
    );
    let mut notes: Vec<String> = Vec::new();
    let t0 = Instant::now();
    let o = run_bounded(&cfg, &reg, fast(1_000), &mut notes).await;
    assert_eq!(o.stderr.as_deref(), Some(REFUSED), "{o:?}");
    assert!(
        t0.elapsed() >= Duration::from_millis(950) && t0.elapsed() < Duration::from_secs(20),
        "an unresolvable name was waited for until the deadline: {:?}",
        t0.elapsed()
    );
    // with no wait it is the same line at once (the old path is unchanged)
    let mut quiet: Vec<String> = Vec::new();
    let o = run_bounded(&cfg, &reg, DbWait::NONE, &mut quiet).await;
    assert_eq!(o.stderr.as_deref(), Some(REFUSED), "{o:?}");
    assert!(quiet.is_empty());
}

/// The runtime of the command is torn down in bounded time even when a blocking task (the resolver's) cannot be cancelled.
#[test]
fn wt_16_the_command_runtime_is_torn_down_in_bounded_time_with_a_blocking_task_pending() {
    let t0 = Instant::now();
    let got = cortex_server::migrate_cmd::block_on_bounded(
        async {
            // a stand-in for the resolver's `spawn_blocking(getaddrinfo)`: it cannot be cancelled and outlives the answer
            tokio::task::spawn_blocking(|| std::thread::sleep(Duration::from_secs(4)));
            7
        },
        Duration::from_millis(300),
    );
    assert_eq!(got, Some(7));
    assert!(
        t0.elapsed() < Duration::from_secs(2),
        "the teardown must not wait for the blocking task: {:?}",
        t0.elapsed()
    );
}

/// A bound of zero is the old behaviour exactly (sam: "zero preserves the one-attempt, no-retry behaviour"): one attempt, which succeeds on a ready
/// database, and on a dead port fails at once with the same line and no progress line.
#[tokio::test]
async fn wt_18_a_bound_of_zero_is_one_attempt_and_no_wait() {
    let db = setup("wt18").await;
    let (reg, enabled) = registry();
    let cfg = cfg_via(&db, "wt18", pg_port(&db), &enabled);
    let mut notes: Vec<String> = Vec::new();
    let o = run_bounded(&cfg, &reg, DbWait::NONE, &mut notes).await;
    assert_eq!((o.code, o.stderr.clone()), (EXIT_OK, None), "{o:?}");
    assert!(notes.is_empty(), "{notes:?}");
    let (_held, port) = reserve();
    let dead = cfg_for("wt18d", port, "cortex", "cortex_owner", "x", &enabled);
    let t0 = Instant::now();
    let o = run_bounded(&dead, &reg, DbWait::NONE, &mut notes).await;
    assert_eq!(o.stderr.as_deref(), Some(REFUSED), "{o:?}");
    assert!(t0.elapsed() < Duration::from_secs(2), "{:?}", t0.elapsed());
    assert!(notes.is_empty(), "{notes:?}");
}

// ---- the real binary (DB-free: the database port is one nobody listens on) --------------------------------------------------

/// A config file for the real `cortex-server migrate`: the database at 127.0.0.1:`port`, which refuses.
fn config_file(tag: &str, port: u16) -> PathBuf {
    let dir = dir_for(tag);
    fs::write(dir.join("owner.password"), "owner-secret-canary\n").expect("owner file");
    fs::write(dir.join("app.password"), "app-secret-canary\n").expect("app file");
    let text = format!(
        "[paths]\nstate_dir = \"/var/lib/cortex\"\nhost_socket = \"/run/cortex/admin.sock\"\nworker_socket = \"/run/cortex/api.sock\"\n\n\
         [modules]\nenabled = [\"audit\", \"identity\", \"tokens\"]\n\n\
         [database]\nhost = \"127.0.0.1\"\nport = {port}\nname = \"cortex\"\napp_user = \"cortex_app\"\napp_password_file = \"{app}\"\n\
         owner_user = \"cortex_owner\"\nowner_password_file = \"{owner}\"\n",
        app = dir.join("app.password").display(),
        owner = dir.join("owner.password").display(),
    );
    let f = dir.join("cortex.toml");
    fs::write(&f, text).expect("config");
    f
}

fn spawn_migrate(config: &Path, knob: Option<&str>) -> Child {
    let mut c = Command::new(env!("CARGO_BIN_EXE_cortex-server"));
    c.args(["migrate", "--config", config.to_str().expect("utf8")])
        .env_clear()
        .env("HOME", "/home/x")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(v) = knob {
        c.env(KNOB, v);
    }
    c.spawn().expect("the binary runs")
}

/// Wait up to `limit` for the child to end; `None` if it is still running.
fn ended_within(child: &mut Child, limit: Duration) -> Option<std::process::ExitStatus> {
    let t0 = Instant::now();
    loop {
        if let Some(s) = child.try_wait().expect("try_wait") {
            return Some(s);
        }
        if t0.elapsed() >= limit {
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn sigterm(child: &Child) {
    let ok = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .expect("kill runs")
        .success();
    assert!(ok, "kill -TERM");
}

/// Never leave a child behind, whatever an assertion did.
struct Reap(Child);
impl Drop for Reap {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn wt_06_a_sigterm_during_the_wait_ends_the_process_at_once_and_non_zero() {
    let (_held, port) = reserve_blocking();
    let cfg = config_file("wt06", port);
    let mut child = Reap(spawn_migrate(&cfg, Some("60")));
    assert!(
        ended_within(&mut child.0, Duration::from_secs(1)).is_none(),
        "with a 60 s bound the process is still waiting after a second"
    );
    sigterm(&child.0);
    let status = ended_within(&mut child.0, Duration::from_secs(2))
        .expect("SIGTERM ends the wait within two seconds");
    assert!(!status.success(), "{status:?}");
}

#[test]
fn wt_07_the_knob_unset_means_the_production_wait_zero_means_none_and_a_bad_value_is_refused() {
    let (_held, port) = reserve_blocking();
    let cfg = config_file("wt07", port);
    // 0: today's immediate failure, the same line
    let out = spawn_migrate(&cfg, Some("0"))
        .wait_with_output()
        .expect("run");
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&out.stderr), format!("{REFUSED}\n"));
    // a value that is not a whole number from 0 to 3600: refused, one line, and NOTHING attempted: a port that LISTENS sees no connection
    let spy = std::net::TcpListener::bind("127.0.0.1:0").expect("a listening port");
    spy.set_nonblocking(true).expect("nonblocking");
    let spied = config_file("wt07spy", spy.local_addr().expect("addr").port());
    for bad in [
        "abc",
        "3601",
        "-1",
        "+5",
        " 5",
        "5 ",
        "1.5",
        "",
        "99999999999999999999",
        "\u{663}",
    ] {
        let mut child = Reap(spawn_migrate(&spied, Some(bad)));
        let status = ended_within(&mut child.0, Duration::from_secs(10))
            .unwrap_or_else(|| panic!("{bad:?}: the process did not end: it is connecting"));
        let mut stderr = String::new();
        std::io::Read::read_to_string(child.0.stderr.as_mut().expect("stderr"), &mut stderr)
            .expect("stderr text");
        assert_eq!(status.code(), Some(1), "{bad:?}");
        assert_eq!(stderr, format!("{KNOB_REFUSED}\n"), "{bad:?}");
        assert!(
            matches!(spy.accept(), Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock),
            "{bad:?}: the refusal came before any connect attempt"
        );
    }
    // unset: the production wait (300 s): still waiting after a second
    let mut child = Reap(spawn_migrate(&cfg, None));
    assert!(
        ended_within(&mut child.0, Duration::from_secs(1)).is_none(),
        "the default is a wait, not an immediate failure"
    );
    sigterm(&child.0);
    assert!(ended_within(&mut child.0, Duration::from_secs(2)).is_some());
}

#[test]
fn wt_17_a_knob_that_is_not_unicode_is_refused_without_a_connection() {
    use std::os::unix::ffi::OsStrExt;
    let spy = std::net::TcpListener::bind("127.0.0.1:0").expect("a listening port");
    spy.set_nonblocking(true).expect("nonblocking");
    let cfg = config_file("wt17", spy.local_addr().expect("addr").port());
    let mut c = Command::new(env!("CARGO_BIN_EXE_cortex-server"));
    c.args(["migrate", "--config", cfg.to_str().expect("utf8")])
        .env_clear()
        .env("HOME", "/home/x")
        .env(KNOB, std::ffi::OsStr::from_bytes(b"\xff\xfe"))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = Reap(c.spawn().expect("runs"));
    let status = ended_within(&mut child.0, Duration::from_secs(10))
        .unwrap_or_else(|| panic!("the process did not end: it is connecting"));
    let mut stderr = String::new();
    std::io::Read::read_to_string(child.0.stderr.as_mut().expect("stderr"), &mut stderr)
        .expect("stderr text");
    assert_eq!(status.code(), Some(1));
    assert_eq!(stderr, format!("{KNOB_REFUSED}\n"));
    assert!(matches!(spy.accept(), Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock));
}

/// `reserve` for the plain (non-async) cells: a tokio socket needs no runtime to be made and bound.
fn reserve_blocking() -> (TcpSocket, u16) {
    reserve()
}
