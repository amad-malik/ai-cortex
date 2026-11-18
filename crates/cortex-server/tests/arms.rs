//! The `migrate` and `bootstrap` command arms over an ASSEMBLED registry and a real database (contract r0.5 section 6; W1-37). DB-ONLY.
//! The registry holds the REAL mod-identity and mod-tokens and a FIXTURE audit piece (mod-audit is not merged yet); the owner and app
//! passwords reach the code through files, as in production. RED first.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use cortex_config::{Config, Database, DbHost, DbOwner, ModuleList, Paths, Server};
use cortex_db::{DbEndpoint, MigrationSet, ModuleTx};
use cortex_module::{
    AuditDetails, AuditSink, AuditTarget, BoxFuture, CONTRACT_VERSION, Health, Manifest, Piece,
    Port, PortError, Ports, PortsBuilder, Registry, Routes,
};
use cortex_server::assembly::assemble_with;
use cortex_server::bootstrap_cmd::bootstrap_with;
use cortex_server::cli::{BootstrapArgs, EXIT_FAILURE, EXIT_OK};
use cortex_server::migrate_cmd::migrate_with;
use cortex_testkit::{TestDb, empty_db};
use cortex_types::{ModuleId, ProblemCodeMinter, SystemClock};
use mod_identity::{BOOTSTRAP_SQL, ModIdentity};
use mod_tokens::ModTokens;

fn minter() -> &'static ProblemCodeMinter {
    static M: OnceLock<ProblemCodeMinter> = OnceLock::new();
    M.get_or_init(|| ProblemCodeMinter::take().expect("the first take in this process"))
}

// ---- the fixture audit piece: provides the AuditSink port and records the action names -------------------------------------

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

#[derive(Default)]
struct Seen(Mutex<Vec<String>>);

struct Sink(Arc<Seen>);

#[async_trait]
impl AuditSink for Sink {
    async fn record(
        &self,
        _tx: &mut ModuleTx,
        action: &'static str,
        _target: AuditTarget,
        _details: AuditDetails,
    ) -> Result<(), PortError> {
        self.0.0.lock().unwrap().push(action.to_owned());
        Ok(())
    }
}

struct FxAudit(Arc<Seen>);

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
        ports.audit(Arc::new(Sink(self.0.clone())));
    }
    fn routes(&self, _ports: &Ports) -> Routes {
        Routes::new()
    }
    fn health(&self) -> BoxFuture<'_, Health> {
        Box::pin(async { Health::Ready })
    }
}

/// The registry of the real identity and tokens plus the fixture audit.
fn registry(seen: &Arc<Seen>) -> (Registry, Vec<ModuleId>) {
    let pieces: Vec<Box<dyn Piece>> = vec![
        Box::new(FxAudit(seen.clone())),
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

// ---- the database and the config ------------------------------------------------------------------------------------------

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

/// The config a real install holds: the database table with BOTH password files.
fn config_for(db: &TestDb, name: &str, enabled: &[ModuleId]) -> (Config, PathBuf) {
    let oc = db.owner_config();
    let app = db.db_config(&[], Arc::new(SystemClock));
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("arms-{name}-{}", std::process::id()));
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
    let cfg = Config {
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
        server: Server::default(),
    };
    (cfg, dir)
}

async fn ledger_modules(db: &TestDb) -> Vec<String> {
    let mut admin = db.admin().await;
    sqlx::query_scalar("SELECT DISTINCT module FROM platform.migrations ORDER BY 1")
        .fetch_all(&mut admin)
        .await
        .expect("ledger")
}

/// The modules of the ledger in the order of their FIRST applied step (each step is its own transaction, so `applied_at` increases).
async fn ledger_first_applied_order(db: &TestDb) -> Vec<String> {
    let mut admin = db.admin().await;
    sqlx::query_scalar(
        "SELECT module FROM platform.migrations GROUP BY module ORDER BY min(applied_at)",
    )
    .fetch_all(&mut admin)
    .await
    .expect("ledger order")
}

async fn ledger_rows(db: &TestDb) -> i64 {
    let mut admin = db.admin().await;
    sqlx::query_scalar("SELECT count(*) FROM platform.migrations")
        .fetch_one(&mut admin)
        .await
        .expect("count")
}

async fn counts(db: &TestDb) -> (i64, i64, i64) {
    let mut admin = db.admin().await;
    let p: i64 = sqlx::query_scalar("SELECT count(*) FROM identity.projects")
        .fetch_one(&mut admin)
        .await
        .expect("projects");
    let t: i64 = sqlx::query_scalar("SELECT count(*) FROM tokens.tokens")
        .fetch_one(&mut admin)
        .await
        .expect("tokens");
    let a: i64 = sqlx::query_scalar("SELECT count(*) FROM tokens.tokens WHERE revoked_at IS NULL")
        .fetch_one(&mut admin)
        .await
        .expect("active");
    (p, t, a)
}

fn args(cfg_dir: &Path, token_name: &str, require_existing: bool) -> BootstrapArgs {
    BootstrapArgs {
        config: cfg_dir.join("cortex.toml"),
        project: "zz-arms-project".into(),
        lead: "zz-arms-lead".into(),
        token_file: cfg_dir.join(token_name),
        require_existing,
    }
}

// ---- the cells -------------------------------------------------------------------------------------------------------------

#[tokio::test]
async fn ar_01_migrate_applies_platform_then_every_enabled_piece_and_a_second_run_changes_nothing()
{
    let db = setup("ar01").await;
    let seen = Arc::new(Seen::default());
    let (reg, enabled) = registry(&seen);
    // hale (packet 6): the CONFIG lists the pieces in the reverse of their dependency order; only the registry's order may decide.
    let mut config_order = enabled.clone();
    config_order.reverse();
    let (cfg, _dir) = config_for(&db, "ar01", &config_order);
    let o = migrate_with(&cfg, &reg).await;
    assert_eq!((o.code, o.stderr.clone()), (EXIT_OK, None), "{o:?}");
    let mods = ledger_modules(&db).await;
    assert_eq!(
        mods,
        vec![
            "identity".to_owned(),
            "platform".to_owned(),
            "tokens".to_owned()
        ],
        "platform and the two real pieces with migrations are recorded (the fixture has none)"
    );
    assert_eq!(
        ledger_first_applied_order(&db).await,
        vec![
            "platform".to_owned(),
            "identity".to_owned(),
            "tokens".to_owned()
        ],
        "platform first, identity before tokens (the registry's order), whatever order the config lists them in"
    );
    let rows = ledger_rows(&db).await;
    assert!(rows >= 3, "at least one step each: {rows}");
    let again = migrate_with(&cfg, &reg).await;
    assert_eq!(
        (again.code, again.stderr.clone()),
        (EXIT_OK, None),
        "{again:?}"
    );
    assert_eq!(ledger_rows(&db).await, rows, "a current schema is a no-op");
}

#[tokio::test]
async fn ar_02_migrate_refusals_name_a_key_or_say_the_file_could_not_be_read_and_apply_nothing() {
    let db = setup("ar02").await;
    let seen = Arc::new(Seen::default());
    let (reg, enabled) = registry(&seen);
    let (mut cfg, _dir) = config_for(&db, "ar02", &enabled);
    // no owner keys
    let owner = cfg.database.as_mut().expect("db").owner.take();
    let o = migrate_with(&cfg, &reg).await;
    assert_eq!(o.code, EXIT_FAILURE);
    assert_eq!(
        o.stderr.as_deref(),
        Some("migrate refused: missing: database.owner_user")
    );
    // an unreadable owner password file
    let mut owner = owner.expect("owner");
    owner.password_file = PathBuf::from("/nonexistent/owner.password");
    cfg.database.as_mut().expect("db").owner = Some(owner);
    let o = migrate_with(&cfg, &reg).await;
    assert_eq!(o.code, EXIT_FAILURE);
    assert_eq!(
        o.stderr.as_deref(),
        Some("migrate refused: the owner password file could not be read")
    );
    // no [database] table
    cfg.database = None;
    let o = migrate_with(&cfg, &reg).await;
    assert_eq!(o.code, EXIT_FAILURE);
    assert_eq!(
        o.stderr.as_deref(),
        Some("migrate refused: the config has no [database] table")
    );
    // nothing was applied by any of the three refusals
    let mut admin = db.admin().await;
    let has_ledger: bool =
        sqlx::query_scalar("SELECT to_regclass('platform.migrations') IS NOT NULL")
            .fetch_one(&mut admin)
            .await
            .expect("regclass");
    assert!(!has_ledger, "the ledger table does not exist: nothing ran");
}

#[tokio::test]
async fn ar_03_bootstrap_creates_the_project_the_lead_the_token_and_the_file_then_a_rerun_reissues()
{
    let db = setup("ar03").await;
    let seen = Arc::new(Seen::default());
    let (reg, enabled) = registry(&seen);
    let (cfg, dir) = config_for(&db, "ar03", &enabled);
    let m = migrate_with(&cfg, &reg).await;
    assert_eq!(m.code, EXIT_OK, "{m:?}");

    let a1 = args(&dir, "first.token", false);
    let o = bootstrap_with(&cfg, &reg, &a1).await;
    assert_eq!((o.code, o.stderr.clone()), (EXIT_OK, None), "{o:?}");
    let first = fs::read(&a1.token_file).expect("the token file");
    assert!(!first.is_empty());
    assert_eq!(
        fs::metadata(&a1.token_file)
            .expect("meta")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        counts(&db).await,
        (1, 1, 1),
        "one project, one token, active"
    );
    let actions = seen.0.lock().unwrap().clone();
    assert!(
        actions.iter().any(|a| a == "token.issued")
            && actions.iter().any(|a| a == "project.bootstrapped"),
        "{actions:?}"
    );

    let a2 = args(&dir, "second.token", true);
    let o = bootstrap_with(&cfg, &reg, &a2).await;
    assert_eq!((o.code, o.stderr.clone()), (EXIT_OK, None), "{o:?}");
    assert_ne!(fs::read(&a2.token_file).expect("second file"), first);
    assert_eq!(
        counts(&db).await,
        (1, 2, 1),
        "the re-run superseded the first token: two rows, one active, still one project"
    );
}

#[tokio::test]
async fn ar_04_bootstrap_on_a_database_that_was_not_migrated_fails_with_one_line_and_leaves_no_file()
 {
    let db = setup("ar04").await;
    let seen = Arc::new(Seen::default());
    let (reg, enabled) = registry(&seen);
    let (cfg, dir) = config_for(&db, "ar04", &enabled);
    let a = args(&dir, "never.token", false);
    let o = bootstrap_with(&cfg, &reg, &a).await;
    assert_eq!(o.code, EXIT_FAILURE, "{o:?}");
    let line = o.stderr.expect("a line");
    assert!(
        line.starts_with("bootstrap failed:") && !line.contains('\n'),
        "{line}"
    );
    assert!(!a.token_file.exists(), "no token file");
}

#[tokio::test]
async fn ar_05_a_registry_without_an_audit_provider_does_not_assemble_so_bootstrap_never_runs_without_one()
 {
    // identity USES the audit port, so the registry refuses the assembly by itself (W1-37); `bootstrap_with` keeps a defensive refusal for a
    // registry that somehow lacks the ports, but the production path cannot reach it.
    let e = match assemble_with(
        vec![Box::new(ModIdentity::new())],
        &[ModuleId::parse("identity").expect("id")],
        minter(),
    ) {
        Err(e) => e,
        Ok(_) => panic!("identity alone must not assemble"),
    };
    let line = cortex_server::assembly::startup_line(&e);
    assert!(line.contains("AuditSink"), "{line}");
}

/// A config whose database port has nobody listening, with both password files readable: the connection itself fails.
fn dead_config(name: &str, enabled: &[ModuleId]) -> (Config, PathBuf) {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("arms-dead-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("dir");
    let owner_pw = dir.join("owner.password");
    fs::write(&owner_pw, "owner-secret-canary\n").expect("owner file");
    let app_pw = dir.join("app.password");
    fs::write(&app_pw, "app-secret-canary\n").expect("app file");
    let cfg = Config {
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
            host: DbHost::Tcp("127.0.0.1".into()),
            port: 1,
            name: "cortex".into(),
            app_user: "cortex_app".into(),
            app_password_file: app_pw,
            owner: Some(DbOwner {
                user: "cortex_owner".into(),
                password_file: owner_pw,
            }),
        }),
        server: Server::default(),
    };
    (cfg, dir)
}

/// hale (packet 6): the connection-failure lines of both arms had no cell. DB-free: nobody listens on the port.
#[tokio::test]
async fn ar_06_a_database_that_cannot_be_reached_is_one_fixed_line_without_the_secret_in_both_arms()
{
    let seen = Arc::new(Seen::default());
    let (reg, enabled) = registry(&seen);
    let (cfg, dir) = dead_config("conn", &enabled);
    let m = migrate_with(&cfg, &reg).await;
    assert_eq!(m.code, EXIT_FAILURE);
    assert_eq!(
        m.stderr.as_deref(),
        Some("migrate failed: could not connect to the database as the owner")
    );
    let a = args(&dir, "dead.token", false);
    let b = bootstrap_with(&cfg, &reg, &a).await;
    assert_eq!(b.code, EXIT_FAILURE);
    assert_eq!(
        b.stderr.as_deref(),
        Some("bootstrap failed: could not connect to the database")
    );
    for line in [m.stderr.unwrap_or_default(), b.stderr.unwrap_or_default()] {
        assert!(
            !line.contains("secret-canary") && !line.contains('\n'),
            "{line}"
        );
    }
    assert!(!a.token_file.exists(), "no token file");
}

// ---- cells adopted from hale's packet 6 review probes (mvp/reviews/piece11-cortex-server-hale-receipts/p6-probes-arms.rs pa_5, pa_6 and
// p6-probes-arms-pa7.rs pa_7): the app-config refusal of `bootstrap`, the audit actions of BOTH runs in order, and the actor of the host
// transaction. Each one kills a mutant that survived the cells above (H06, the exact sequence, A09). -----------------------------------

#[tokio::test]
async fn ar_07_bootstrap_with_an_unreadable_app_password_file_refuses_with_one_line_and_writes_no_file()
 {
    let db = setup("ar07").await;
    let seen = Arc::new(Seen::default());
    let (reg, enabled) = registry(&seen);
    let (mut cfg, dir) = config_for(&db, "ar07", &enabled);
    let m = migrate_with(&cfg, &reg).await;
    assert_eq!(m.code, EXIT_OK, "{m:?}");
    cfg.database.as_mut().expect("db").app_password_file =
        PathBuf::from("/nonexistent/app.password");
    let a = args(&dir, "never.token", false);
    let o = bootstrap_with(&cfg, &reg, &a).await;
    assert_eq!(o.code, EXIT_FAILURE, "{o:?}");
    let line = o.stderr.expect("a line");
    assert!(
        line.starts_with("bootstrap refused:") && !line.contains('\n'),
        "{line}"
    );
    assert!(
        !line.contains("/nonexistent"),
        "no path in the line: {line}"
    );
    assert!(!a.token_file.exists(), "no token file");
}

#[tokio::test]
async fn ar_08_both_bootstrap_runs_record_their_audit_actions_in_order() {
    let db = setup("ar08").await;
    let seen = Arc::new(Seen::default());
    let (reg, enabled) = registry(&seen);
    let (cfg, dir) = config_for(&db, "ar08", &enabled);
    let m = migrate_with(&cfg, &reg).await;
    assert_eq!(m.code, EXIT_OK, "{m:?}");
    let o = bootstrap_with(&cfg, &reg, &args(&dir, "first.token", false)).await;
    assert_eq!(o.code, EXIT_OK, "{o:?}");
    let first = seen.0.lock().unwrap().clone();
    let o = bootstrap_with(&cfg, &reg, &args(&dir, "second.token", true)).await;
    assert_eq!(o.code, EXIT_OK, "{o:?}");
    let all = seen.0.lock().unwrap().clone();
    let second = &all[first.len()..];
    assert!(first.iter().any(|a| a == "token.issued"), "{first:?}");
    assert!(
        first.iter().any(|a| a == "project.bootstrapped"),
        "{first:?}"
    );
    assert!(second.iter().any(|a| a == "token.reissued"), "{second:?}");
    assert!(
        !second.iter().any(|a| a == "token.issued"),
        "the re-run does not issue a first token: {second:?}"
    );
}

/// A sink that records the host transaction's ACTOR with each action (the production `process_uid()` is the owner of `/proc/self`).
struct ActorSink(Arc<Seen>);

#[async_trait]
impl AuditSink for ActorSink {
    async fn record(
        &self,
        tx: &mut ModuleTx,
        action: &'static str,
        _target: AuditTarget,
        _details: AuditDetails,
    ) -> Result<(), PortError> {
        self.0
            .0
            .lock()
            .unwrap()
            .push(format!("{action}|{:?}", tx.context().actor));
        Ok(())
    }
}

struct ActorAudit(Arc<Seen>);

impl Piece for ActorAudit {
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
        ports.audit(Arc::new(ActorSink(self.0.clone())));
    }
    fn routes(&self, _ports: &Ports) -> Routes {
        Routes::new()
    }
    fn health(&self) -> BoxFuture<'_, Health> {
        Box::pin(async { Health::Ready })
    }
}

/// The effective uid from `/proc/self/status`: a source independent of the `/proc/self` ownership the command reads.
fn effective_uid() -> u32 {
    let status = fs::read_to_string("/proc/self/status").expect("status");
    let line = status
        .lines()
        .find(|l| l.starts_with("Uid:"))
        .expect("Uid line");
    line.split_whitespace()
        .nth(2)
        .expect("effective uid")
        .parse()
        .expect("number")
}

#[tokio::test]
async fn ar_09_the_audit_records_of_bootstrap_carry_the_operators_uid_as_the_host_actor() {
    let db = setup("ar09").await;
    let seen = Arc::new(Seen::default());
    let pieces: Vec<Box<dyn Piece>> = vec![
        Box::new(ActorAudit(seen.clone())),
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
            "must assemble: {}",
            cortex_server::assembly::startup_line(&e)
        )
    });
    let (cfg, dir) = config_for(&db, "ar09", &enabled);
    let m = migrate_with(&cfg, &reg).await;
    assert_eq!(m.code, EXIT_OK, "{m:?}");
    let o = bootstrap_with(&cfg, &reg, &args(&dir, "first.token", false)).await;
    assert_eq!(o.code, EXIT_OK, "{o:?}");
    let want = format!("HostOperator {{ uid: {} }}", effective_uid());
    let recorded = seen.0.lock().unwrap().clone();
    assert!(recorded.len() >= 2, "{recorded:?}");
    for r in &recorded {
        assert!(r.ends_with(&want), "the actor of {r} is not {want}");
    }
}
