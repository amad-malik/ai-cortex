//! The real `Migrator` over the owner connection (contract r0.4 section 5). DB-ONLY: needs `supervise.sh`'s throwaway Postgres. RED first.
//! The migration sets are the REAL platform and identity sets plus fixture sets that fail; the owner password reaches the code through a
//! file, as in production, and the decision of `migrate::run` is the real one.

use async_trait::async_trait;
use cortex_config::{Config, Database, DbHost, DbOwner, ModuleList, Paths, Server};
use cortex_db::migrate::platform_migrations;
use cortex_db::{DbEndpoint, Migration, MigrationSet, OwnerConn};
use cortex_module::Piece;
use cortex_server::cli::{EXIT_FAILURE, EXIT_OK};
use cortex_server::dbowner::owner_config;
use cortex_server::migrate::{Migrator, StepFailed, run};
use cortex_server::migrator::DbMigrator;
use cortex_testkit::{TestDb, empty_db};
use cortex_types::ModuleId;
use mod_identity::{BOOTSTRAP_SQL, ModIdentity};
use std::fs;
use std::path::PathBuf;

async fn setup(tag: &str) -> TestDb {
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
    db
}

/// The config a real install would hold: the database table and an owner password FILE.
fn config_for(db: &TestDb, name: &str, enabled: &[&str]) -> Config {
    let oc = db.owner_config();
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("migrator-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("dir");
    let pw = dir.join("owner.password");
    fs::write(&pw, format!("{}\n", oc.owner_password.expose())).expect("owner file");
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
            enabled: enabled
                .iter()
                .map(|m| ModuleId::parse(m).expect("id"))
                .collect(),
        },
        database: Some(Database {
            host,
            port,
            name: oc.database.clone(),
            app_user: "cortex_app".into(),
            app_password_file: PathBuf::from("/unused/app.password"),
            owner: Some(DbOwner {
                user: oc.owner_user.clone(),
                password_file: pw,
            }),
        }),
        server: Server::default(),
    }
}

async fn migrator(db: &TestDb, name: &str, sets: Vec<MigrationSet>) -> DbMigrator {
    let cfg = config_for(db, name, &[]);
    let oc = owner_config(cfg.database.as_ref().expect("db")).expect("owner config");
    DbMigrator::new(OwnerConn::connect(&oc).await.expect("owner conn"), sets)
}

fn zz(steps: Vec<Migration>) -> MigrationSet {
    MigrationSet {
        module: ModuleId::parse("zz").expect("id"),
        schema: "zz",
        steps,
    }
}

fn step(version: u32, up: &'static str) -> Migration {
    Migration {
        version,
        name: "step",
        up,
        down: "SELECT 1",
    }
}

#[tokio::test]
async fn mi_01_the_first_run_applies_every_step_and_a_second_run_is_a_no_op() {
    let db = setup("mi01").await;
    let sets = vec![platform_migrations(), ModIdentity::new().migrations()];
    let platform_steps = sets[0].steps.len() as u32;
    let identity_steps = sets[1].steps.len() as u32;
    let mut m = migrator(&db, "mi01", sets).await;
    assert_eq!(
        m.apply("platform").await,
        Ok(platform_steps),
        "every step of platform is new"
    );
    assert_eq!(m.apply("identity").await, Ok(identity_steps));
    assert_eq!(
        m.apply("platform").await,
        Ok(0),
        "a current schema is a no-op"
    );
    assert_eq!(m.apply("identity").await, Ok(0));
}

#[tokio::test]
async fn mi_02_a_failing_step_names_its_version_and_the_steps_before_it_stay_applied() {
    let db = setup("mi02").await;
    let mut m = migrator(
        &db,
        "mi02",
        vec![
            platform_migrations(),
            zz(vec![
                step(1, "CREATE SCHEMA zz"),
                step(2, "THIS IS NOT SQL"),
            ]),
        ],
    )
    .await;
    assert!(m.apply("platform").await.is_ok());
    assert_eq!(m.apply("zz").await, Err(StepFailed(2)));
    let mut admin = db.admin().await;
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM platform.migrations WHERE module = 'zz'")
        .fetch_one(&mut admin)
        .await
        .expect("count");
    assert_eq!(n, 1, "step 1 is recorded, step 2 is not");
}

#[tokio::test]
async fn mi_03_a_module_the_migrator_does_not_know_is_refused_and_applies_nothing() {
    let db = setup("mi03").await;
    let mut m = migrator(&db, "mi03", vec![platform_migrations()]).await;
    assert!(m.apply("platform").await.is_ok());
    assert_eq!(m.apply("nosuch").await, Err(StepFailed(0)));
}

#[tokio::test]
async fn mi_04_a_changed_step_stops_with_its_version() {
    let db = setup("mi04").await;
    let mut first = migrator(
        &db,
        "mi04a",
        vec![platform_migrations(), zz(vec![step(1, "CREATE SCHEMA zz")])],
    )
    .await;
    assert!(first.apply("platform").await.is_ok());
    assert_eq!(first.apply("zz").await, Ok(1));
    let mut second = migrator(
        &db,
        "mi04b",
        vec![
            platform_migrations(),
            zz(vec![step(
                1,
                "CREATE SCHEMA zz; -- edited after it was applied",
            )]),
        ],
    )
    .await;
    assert_eq!(
        second.apply("zz").await,
        Err(StepFailed(1)),
        "the recorded checksum no longer matches"
    );
}

#[tokio::test]
async fn mi_05_migrate_run_end_to_end_over_the_real_owner_config() {
    let db = setup("mi05").await;
    let cfg = config_for(&db, "mi05", &["identity"]);
    let order: Vec<ModuleId> = ["platform", "identity"]
        .iter()
        .map(|m| ModuleId::parse(m).expect("id"))
        .collect();
    let mut m = migrator(
        &db,
        "mi05",
        vec![platform_migrations(), ModIdentity::new().migrations()],
    )
    .await;
    let o = run(&cfg, &order, &mut m).await;
    assert_eq!((o.code, o.stderr.clone()), (EXIT_OK, None), "{o:?}");
    let o2 = run(&cfg, &order, &mut m).await;
    assert_eq!(
        o2.code, EXIT_OK,
        "the second run is a no-op that still succeeds"
    );
    // a broken set fails the whole run, names module and version, and prints nothing secret
    let mut bad = migrator(
        &db,
        "mi05b",
        vec![
            platform_migrations(),
            ModIdentity::new().migrations(),
            zz(vec![step(1, "NOT SQL")]),
        ],
    )
    .await;
    let cfg2 = config_for(&db, "mi05c", &["identity", "zz"]);
    let order2: Vec<ModuleId> = ["platform", "identity", "zz"]
        .iter()
        .map(|m| ModuleId::parse(m).expect("id"))
        .collect();
    let o3 = run(&cfg2, &order2, &mut bad).await;
    assert_eq!(o3.code, EXIT_FAILURE);
    assert_eq!(
        o3.stderr.as_deref(),
        Some("migrate failed: module zz, migration 1")
    );
}

// keep the unused-import lint quiet for the trait that makes `apply` callable
#[allow(dead_code)]
fn _uses(_: &dyn Migrator, _: &dyn Fn() -> Box<dyn Send>) {}
#[allow(dead_code)]
struct _Unused;
#[async_trait]
impl Migrator for _Unused {
    async fn apply(&mut self, _m: &str) -> Result<u32, StepFailed> {
        Ok(0)
    }
}

/// hale F-1..F-3 (packet 4): her probes, adopted. mp_a: a set that grew reports only the new steps; mp_b: the failing step of three is
/// named and the step after it is not applied; mp_c: when the connection dies inside a step the line never names an applied step (F-1).
#[tokio::test]
async fn mp_a_a_set_that_grew_reports_only_the_new_steps() {
    let db = setup("mpa").await;
    let mut m1 = migrator(
        &db,
        "mpa1",
        vec![platform_migrations(), zz(vec![step(1, "CREATE SCHEMA zz")])],
    )
    .await;
    assert!(m1.apply("platform").await.is_ok());
    assert_eq!(m1.apply("zz").await, Ok(1));
    let mut m2 = migrator(
        &db,
        "mpa2",
        vec![
            platform_migrations(),
            zz(vec![
                step(1, "CREATE SCHEMA zz"),
                step(2, "CREATE TABLE zz.t (id int)"),
                step(3, "CREATE TABLE zz.u (id int)"),
            ]),
        ],
    )
    .await;
    let r = m2.apply("zz").await;
    assert_eq!(r, Ok(2));
}

#[tokio::test]
async fn mp_b_the_failing_step_of_three_is_named_and_the_step_after_it_is_not_applied() {
    let db = setup("mpb").await;
    let mut m = migrator(
        &db,
        "mpb",
        vec![
            platform_migrations(),
            zz(vec![
                step(1, "CREATE SCHEMA zz"),
                step(2, "THIS IS NOT SQL"),
                step(3, "CREATE TABLE zz.t (id int)"),
            ]),
        ],
    )
    .await;
    assert!(m.apply("platform").await.is_ok());
    let r = m.apply("zz").await;
    let mut admin = db.admin().await;
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM platform.migrations WHERE module = 'zz'")
        .fetch_one(&mut admin)
        .await
        .expect("count");
    let t: bool = sqlx::query_scalar("SELECT to_regclass('zz.t') IS NOT NULL")
        .fetch_one(&mut admin)
        .await
        .expect("regclass");
    assert_eq!(r, Err(StepFailed(2)));
    assert_eq!((n, t), (1, false));
}

#[tokio::test]
async fn mp_c_when_the_connection_dies_inside_a_step_the_version_is_not_a_wrong_one() {
    // Step 2 terminates its own session. The error names no step, and the ledger cannot be read afterwards either. The line must then say 2 (the
    // step that was running) or 0 (unknown), never 1, the first step of the set, which was applied and committed.
    let db = setup("mpc").await;
    let mut m = migrator(
        &db,
        "mpc",
        vec![
            platform_migrations(),
            zz(vec![
                step(1, "CREATE SCHEMA zz"),
                step(2, "SELECT pg_terminate_backend(pg_backend_pid())"),
                step(3, "CREATE TABLE zz.t (id int)"),
            ]),
        ],
    )
    .await;
    assert!(m.apply("platform").await.is_ok());
    let r = m.apply("zz").await;
    assert!(
        matches!(r, Err(StepFailed(2)) | Err(StepFailed(0))),
        "{r:?}"
    );
}
