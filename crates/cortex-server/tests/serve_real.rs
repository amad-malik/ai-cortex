//! `migrate` and `serve` over the REAL pieces and a real database (piece 11 `serve`, mira; cox 7 Nov 12:52Z, packet 6 merged: "migrate the
//! throwaway database with `migrate_with`, then `serve_with` on it"). DB-ONLY: needs `supervise.sh`'s throwaway Postgres.
//!
//! The registry here is `compiled()` itself: every piece this build compiles in (audit, identity and tokens always; the optional ones with
//! their features), enabled all together, assembled by `assemble_with` with the process's one minter. The database is built by the SAME
//! function the `migrate` command runs (`migrate_with`), from the owner password file, and the server is the SAME function the `serve`
//! command runs (`serve_with`) with the PRODUCTION checks, on the `cortex_app` pool from the app password file. So what these cells see is the
//! first time the real set is migrated by the command and served ready end to end.
//!
//! They need the `mod-policy` feature: the policy piece is the one that provides the scope policy, and `Registry::assemble` refuses a set
//! in which nothing does ("no piece provides the scope policy"). The core three (audit, identity, tokens) alone are therefore NOT a servable
//! set: the first run of these cells without the feature was refused with exactly that line (receipt
//! `serve-real-RED-default-features-3f4eb1f.txt`). Without the feature this file is empty, and the feature runs are their own receipts (as for
//! `serve_policy.rs`); the release build names its features.
#![cfg(feature = "mod-policy")]
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use cortex_config::{Config, Database, DbHost, DbOwner, ModuleList, Paths, Server};
use cortex_db::migrate::{apply, platform_migrations};
use cortex_db::{DbEndpoint, OwnerConn};
use cortex_module::Registry;
use cortex_server::assembly::{Wiring, assemble_with, compiled, compiled_with_wiring};
use cortex_server::bootstrap_cmd::bootstrap_with;
use cortex_server::cli::{BootstrapArgs, EXIT_OK};
use cortex_server::migrate_cmd::migrate_with;
use cortex_server::serve::{Refused, ServeOptions, production_checks, serve_with};
use cortex_testkit::{TestDb, empty_db};
use cortex_types::{ModuleId, ProblemCodeMinter, SystemClock};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::oneshot;

fn minter() -> &'static ProblemCodeMinter {
    static M: OnceLock<ProblemCodeMinter> = OnceLock::new();
    M.get_or_init(|| ProblemCodeMinter::take().expect("the first take in this process"))
}

/// The id of every compiled piece, in migration order.
fn ids() -> Vec<&'static str> {
    compiled().iter().map(|p| p.manifest().id).collect()
}

/// The real set, all enabled, assembled (a fresh assembly per call; the minter is shared), and the wiring the server binds once its pool
/// exists (what `serve` command gets from `assemble_wired`).
fn wired() -> (Registry, Wiring) {
    let enabled: Vec<ModuleId> = ids()
        .iter()
        .map(|id| ModuleId::parse(id).expect("a module id"))
        .collect();
    let (pieces, wiring) = compiled_with_wiring();
    let registry = assemble_with(pieces, &enabled, minter()).unwrap_or_else(|e| {
        panic!(
            "the real set must assemble: {}",
            cortex_server::assembly::startup_line(&e)
        )
    });
    (registry, wiring)
}

/// The real set without its wiring: what `migrate` and `bootstrap` use.
fn registry() -> Registry {
    wired().0
}

/// An empty database with what the installer provisions before `migrate`: one NOLOGIN role per compiled piece, granted to `cortex_app`
/// (the form of `deploy/db-roles.sh`: an existing role or a lost race is fine), and `cortex_install_admin`.
async fn setup(tag: &str) -> TestDb {
    let db = empty_db(tag).await;
    let mut admin = db.admin().await;
    for id in ids() {
        let sql = format!(
            "DO $$ BEGIN \
               BEGIN CREATE ROLE cortex_m_{id} NOLOGIN; EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL; END; \
               BEGIN GRANT cortex_m_{id} TO cortex_app; EXCEPTION WHEN unique_violation THEN NULL; END; \
             END $$;"
        );
        // The id is a compiled-in manifest id (a validated module id), never operator input.
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
            .execute(&mut admin)
            .await
            .expect("a module role");
    }
    sqlx::raw_sql("DO $$ BEGIN CREATE ROLE cortex_install_admin NOLOGIN; EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL; END $$;")
        .execute(&mut admin)
        .await
        .expect("install admin role");
    db
}

/// The config a real install holds: the database table with BOTH password files, every compiled piece enabled, the listener on a free
/// loopback port.
fn config_for(db: &TestDb, name: &str) -> Config {
    let oc = db.owner_config();
    let app = db.db_config(&[], Arc::new(SystemClock));
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("real-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("dir");
    let owner_pw = dir.join("owner.password");
    fs::write(&owner_pw, format!("{}\n", oc.owner_password.expose())).expect("owner file");
    let app_pw = dir.join("app.password");
    fs::write(&app_pw, format!("{}\n", app.app_password.expose())).expect("app file");
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
            enabled: ids()
                .iter()
                .map(|id| ModuleId::parse(id).expect("a module id"))
                .collect(),
        },
        database: Some(Database {
            host,
            port,
            name: oc.database.clone(),
            app_user: app.app_user.clone(),
            app_password_file: app_pw,
            owner: Some(DbOwner {
                user: oc.owner_user.clone(),
                password_file: owner_pw,
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

/// `serve_with` as the command runs it: the real set, the PRODUCTION checks, the system clock, no test fault.
async fn start(cfg: Config) -> Result<Up, String> {
    start_with(cfg, false).await
}

/// The same, with the wiring left out when `unbound` (a server that never hands the pool to the token verifier: the defect sr3 found).
async fn start_with(cfg: Config, unbound: bool) -> Result<Up, String> {
    let (registry, wiring) = wired();
    let wiring = if unbound { Wiring::none() } else { wiring };
    let lines = Arc::new(Mutex::new(Vec::new()));
    let sink = lines.clone();
    let (addr_tx, addr_rx) = oneshot::channel();
    let (stop_tx, stop_rx) = oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        serve_with(
            &cfg,
            registry,
            ids(),
            ServeOptions {
                fault: cortex_server::fault::Fault::None,
                version: "0.0.0-test".to_owned(),
                last_good: None,
                clock: Arc::new(SystemClock),
                checks: production_checks,
                wiring,
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
    match tokio::time::timeout(Duration::from_secs(60), addr_rx).await {
        Ok(Ok(addr)) => Ok(Up {
            addr,
            stop: Some(stop_tx),
            handle,
            lines,
        }),
        _ => {
            assert!(
                handle.is_finished(),
                "the server neither bound nor stopped within 60 s"
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
    get_with(addr, path, None).await
}

/// `bearer` is the token the bootstrap wrote; it is only ever put on the wire, never printed or asserted on.
async fn get_with(addr: SocketAddr, path: &str, bearer: Option<&str>) -> (u16, String) {
    let auth = bearer
        .map(|t| format!("Authorization: Bearer {t}\r\n"))
        .unwrap_or_default();
    let mut s = TcpStream::connect(addr).await.expect("connect");
    s.write_all(
        format!("GET {path} HTTP/1.1\r\nHost: x\r\n{auth}Connection: close\r\n\r\n").as_bytes(),
    )
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

async fn document(addr: SocketAddr) -> Value {
    let (status, body) = get(addr, "/v2/health").await;
    assert_eq!(status, 200, "the health document answers");
    serde_json::from_str(&body).expect("the health document")
}

#[tokio::test]
async fn sr1_the_real_set_migrated_by_the_migrate_command_serves_ready_and_stops_cleanly() {
    let db = setup("sr1").await;
    let cfg = config_for(&db, "sr1");
    let o = migrate_with(&cfg, &registry()).await;
    assert_eq!((o.code, o.stderr.clone()), (EXIT_OK, None), "{o:?}");
    let up = start(cfg).await.expect("serves");
    assert_eq!(
        get(up.addr, "/v2/health/live").await,
        (200, r#"{"ok":true}"#.to_owned())
    );
    assert_eq!(
        get(up.addr, "/v2/health/ready").await,
        (200, r#"{"ok":true}"#.to_owned()),
        "the ledger is current for every compiled set and every piece is ready"
    );
    let doc = document(up.addr).await;
    assert_eq!(doc["ready"], true);
    assert_eq!(doc["state"], "ready");
    let pieces: Vec<(String, bool, bool)> = doc["pieces"]
        .as_array()
        .expect("the pieces")
        .iter()
        .map(|p| {
            (
                p["name"].as_str().expect("a name").to_owned(),
                p["enabled"] == true,
                p["ready"] == true,
            )
        })
        .collect();
    let want: Vec<(String, bool, bool)> = ids()
        .iter()
        .map(|id| ((*id).to_owned(), true, true))
        .collect();
    assert!(
        want.len() >= 4,
        "audit, identity, tokens and policy are always in this build: {want:?}"
    );
    assert_eq!(
        pieces, want,
        "every compiled piece is listed, enabled and ready, in migration order"
    );
    // The real audit route is mounted behind the token check: no token is 401, and a path nobody serves is 404.
    assert_eq!(get(up.addr, "/v2/audit").await.0, 401);
    assert_eq!(get(up.addr, "/v2/nothing").await.0, 404);
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
async fn sr2_a_platform_only_database_starts_not_ready_and_becomes_ready_when_migrate_runs() {
    let db = setup("sr2").await;
    let cfg = config_for(&db, "sr2");
    // What the installer leaves before `migrate`: the platform set only.
    let mut owner = OwnerConn::connect(&db.owner_config()).await.expect("owner");
    apply(&mut owner, &platform_migrations())
        .await
        .expect("platform");
    // The starting state is what the cell says it is: the platform's steps are in the ledger and no piece's are.
    let mut admin = db.admin().await;
    let (all, pieces): (i64, i64) = sqlx::query_as(
        "SELECT count(*), count(*) FILTER (WHERE module <> 'platform') FROM platform.migrations",
    )
    .fetch_one(&mut admin)
    .await
    .expect("the ledger");
    assert!(
        all > 0 && pieces == 0,
        "platform steps only: {all} rows, {pieces} of pieces"
    );
    let up = start(cfg.clone()).await.expect("serves, not ready");
    assert_eq!(
        up.lines(),
        ["serve: starting NOT READY: migration steps are not applied yet"]
    );
    assert_eq!(get(up.addr, "/v2/health/live").await.0, 200, "alive");
    assert_eq!(get(up.addr, "/v2/health/ready").await.0, 503, "not ready");
    let o = migrate_with(&cfg, &registry()).await;
    assert_eq!((o.code, o.stderr.clone()), (EXIT_OK, None), "{o:?}");
    // The answer is reused for one second; after that the ledger is read again.
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if get(up.addr, "/v2/health/ready").await.0 == 200 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "still not ready 15 s after migrate ran"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert_eq!(document(up.addr).await["state"], "ready");
    up.stop().await.expect("a clean stop");
}

#[tokio::test]
async fn sr3_the_lead_token_that_bootstrap_wrote_reads_the_audit_trail_through_the_real_tokens_policy_and_audit()
 {
    let db = setup("sr3").await;
    let cfg = config_for(&db, "sr3");
    let o = migrate_with(&cfg, &registry()).await;
    assert_eq!((o.code, o.stderr.clone()), (EXIT_OK, None), "{o:?}");
    let dir = cfg
        .database
        .as_ref()
        .expect("a database table")
        .app_password_file
        .parent()
        .expect("the config directory")
        .to_path_buf();
    let args = BootstrapArgs {
        config: dir.join("cortex.toml"),
        project: "zz-real-project".into(),
        lead: "zz-real-lead".into(),
        token_file: dir.join("lead.token"),
        require_existing: false,
    };
    let o = bootstrap_with(&cfg, &registry(), &args).await;
    assert_eq!((o.code, o.stderr.clone()), (EXIT_OK, None), "{o:?}");
    let token = fs::read_to_string(&args.token_file).expect("the token file");
    let token = token.trim();
    assert!(!token.is_empty(), "the token file holds a token");
    let up = start(cfg).await.expect("serves");
    // Readiness and verification agree: a server that says ready can verify the token it was just given (see sr5 for the other way round).
    assert_eq!(get(up.addr, "/v2/health/ready").await.0, 200, "ready");
    // The real tokens piece verifies the bearer, the real policy decides `audit:read` for the project lead, the real audit piece reads its
    // own table through its own role.
    assert_eq!(get(up.addr, "/v2/audit").await.0, 401, "no token");
    assert_eq!(
        get_with(up.addr, "/v2/audit", Some("junk")).await.0,
        401,
        "a token nobody issued"
    );
    let (status, body) = get_with(up.addr, "/v2/audit", Some(token)).await;
    assert_eq!(status, 200, "the lead reads the audit trail: {body}");
    let doc: Value = serde_json::from_str(&body).expect("a JSON body");
    let text = doc.to_string();
    for action in ["project.bootstrapped", "token.issued"] {
        assert!(
            text.contains(action),
            "the trail holds {action} (bootstrap wrote it through the real audit piece): {text}"
        );
    }
    up.stop().await.expect("a clean stop");
}

/// The server PROCESS, stopped through its exact pid if the cell ends early (SIGTERM, never SIGKILL, never by name).
struct Proc(Child);

impl Drop for Proc {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = Command::new("kill")
                .arg("-TERM")
                .arg(self.0.id().to_string())
                .status();
            for _ in 0..100 {
                if self.0.try_wait().ok().flatten().is_some() {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

#[tokio::test]
async fn sr4_the_real_serve_process_serves_the_migrated_real_set_to_the_bootstrap_token_and_stops_on_sigterm()
 {
    let db = setup("sr4").await;
    let cfg = config_for(&db, "sr4");
    let o = migrate_with(&cfg, &registry()).await;
    assert_eq!((o.code, o.stderr.clone()), (EXIT_OK, None), "{o:?}");
    let d = cfg.database.as_ref().expect("a database table").clone();
    let dir = d
        .app_password_file
        .parent()
        .expect("the config directory")
        .to_path_buf();
    let args = BootstrapArgs {
        config: dir.join("cortex.toml"),
        project: "zz-real-project".into(),
        lead: "zz-real-lead".into(),
        token_file: dir.join("lead.token"),
        require_existing: false,
    };
    let o = bootstrap_with(&cfg, &registry(), &args).await;
    assert_eq!((o.code, o.stderr.clone()), (EXIT_OK, None), "{o:?}");
    let token = fs::read_to_string(&args.token_file).expect("the token file");
    let token = token.trim().to_owned();

    // The config file the process reads (names and file paths only, as in an install), the listener on a free loopback port.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .expect("a free port")
        .local_addr()
        .expect("its address")
        .port();
    let host = match &d.host {
        DbHost::SocketDir(p) => p.display().to_string(),
        DbHost::Tcp(h) => h.clone(),
    };
    let enabled = ids()
        .iter()
        .map(|id| format!("{id:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    let toml = format!(
        "[modules]\nenabled = [{enabled}]\n\n[database]\nhost = {host:?}\nport = {}\nname = {:?}\napp_user = {:?}\napp_password_file = {:?}\n\n[server]\nlisten = \"127.0.0.1:{port}\"\n",
        d.port,
        d.name,
        d.app_user,
        d.app_password_file.display().to_string(),
    );
    let conf = dir.join("serve.toml");
    fs::write(&conf, toml).expect("the config file");
    let stderr_path = dir.join("serve.stderr");
    let mut proc = Proc(
        Command::new(env!("CARGO_BIN_EXE_cortex-server"))
            .arg("serve")
            .arg("--config")
            .arg(&conf)
            .env_clear()
            .env("HOME", &dir)
            .stdout(Stdio::null())
            .stderr(Stdio::from(
                fs::File::create(&stderr_path).expect("a stderr file"),
            ))
            .spawn()
            .expect("the binary starts"),
    );
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().expect("an address");
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(code) = proc.0.try_wait().expect("the process state") {
            panic!(
                "the process stopped before it was ready ({code}): {}",
                fs::read_to_string(&stderr_path).unwrap_or_default()
            );
        }
        if TcpStream::connect(addr).await.is_ok() && get(addr, "/v2/health/ready").await.0 == 200 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "not ready within 60 s: {}",
            fs::read_to_string(&stderr_path).unwrap_or_default()
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let (status, body) = get_with(addr, "/v2/audit", Some(&token)).await;
    assert_eq!(
        status,
        200,
        "the bootstrap token reads the audit trail through the real process: {body} {}",
        fs::read_to_string(&stderr_path).unwrap_or_default()
    );
    assert!(body.contains("project.bootstrapped"), "{body}");
    // SIGTERM to the exact pid of our own child: a clean stop, exit code 0.
    let killed = Command::new("kill")
        .arg("-TERM")
        .arg(proc.0.id().to_string())
        .status()
        .expect("kill runs");
    assert!(killed.success(), "kill -TERM on the child's own pid");
    let deadline = Instant::now() + Duration::from_secs(20);
    let code = loop {
        if let Some(code) = proc.0.try_wait().expect("the process state") {
            break code;
        }
        assert!(
            Instant::now() < deadline,
            "the process did not stop on SIGTERM"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert_eq!(code.code(), Some(0), "a clean stop: {code}");
    assert!(
        TcpStream::connect(addr).await.is_err(),
        "the listener is closed"
    );
}

#[tokio::test]
async fn sr5_readiness_is_never_200_while_the_token_verifier_cannot_verify() {
    let db = setup("sr5").await;
    let cfg = config_for(&db, "sr5");
    let o = migrate_with(&cfg, &registry()).await;
    assert_eq!((o.code, o.stderr.clone()), (EXIT_OK, None), "{o:?}");
    let dir = cfg
        .database
        .as_ref()
        .expect("a database table")
        .app_password_file
        .parent()
        .expect("the config directory")
        .to_path_buf();
    let args = BootstrapArgs {
        config: dir.join("cortex.toml"),
        project: "zz-real-project".into(),
        lead: "zz-real-lead".into(),
        token_file: dir.join("lead.token"),
        require_existing: false,
    };
    let o = bootstrap_with(&cfg, &registry(), &args).await;
    assert_eq!((o.code, o.stderr.clone()), (EXIT_OK, None), "{o:?}");
    let token = fs::read_to_string(&args.token_file).expect("the token file");
    let token = token.trim();
    // A server that never binds the pool into the verifier: the defect sr3 found, put back on purpose.
    let up = start_with(cfg, true).await.expect("serves");
    let (ready, _) = get(up.addr, "/v2/health/ready").await;
    let (auth, body) = get_with(up.addr, "/v2/audit", Some(token)).await;
    // Control: with the pool unbound the verifier really cannot verify (else this cell proves nothing).
    assert_eq!(
        auth, 503,
        "control: the unbound verifier fails closed: {body}"
    );
    assert!(
        ready != 200,
        "readiness says ready ({ready}) while every verification fails ({auth}): the piece must report NotReady until its store is bound"
    );
    up.stop().await.expect("a clean stop");
}
