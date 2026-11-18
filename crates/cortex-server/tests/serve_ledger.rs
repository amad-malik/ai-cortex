//! The real `Ledger` over a real database (piece 11 `serve` S4b, mira). DB-ONLY: needs `supervise.sh`'s throwaway Postgres. RED first.
//! The sets are the REAL platform and identity sets plus a fixture set `zz`; the pool is the real `cortex_app` pool, so what `serve`
//! can read is what these cells can read.
use std::sync::Arc;
use std::time::Duration;

use cortex_db::migrate::{LedgerState, apply, platform_migrations};
use cortex_db::{DbPool, Migration, MigrationSet, OwnerConn};
use cortex_module::Piece;
use cortex_server::ledger::DbLedger;
use cortex_server::probe::{Ledger, LedgerFault, LedgerRead, Startup, startup_check};
use cortex_testkit::{TestDb, empty_db};
use cortex_types::{ModuleId, SystemClock};
use mod_identity::{BOOTSTRAP_SQL, ModIdentity};

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

fn sets_with(zz_steps: Vec<Migration>) -> Vec<MigrationSet> {
    vec![
        platform_migrations(),
        ModIdentity::new().migrations(),
        zz(zz_steps),
    ]
}

/// Roles bootstrapped (as install does); nothing migrated.
async fn roles_only(tag: &str) -> TestDb {
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

/// The sets applied under the owner role, in order.
async fn migrated(tag: &str, applied: &[MigrationSet]) -> TestDb {
    let db = roles_only(tag).await;
    let mut owner = OwnerConn::connect(&db.owner_config()).await.expect("owner");
    for set in applied {
        apply(&mut owner, set).await.expect("apply");
    }
    db
}

async fn pool(db: &TestDb) -> Arc<DbPool> {
    Arc::new(
        DbPool::connect(&db.db_config(&["platform", "identity", "zz"], Arc::new(SystemClock)))
            .await
            .expect("pool"),
    )
}

fn answered(read: LedgerRead) -> Vec<(String, Result<LedgerState, LedgerFault>)> {
    match read {
        LedgerRead::Answered(s) => s,
        LedgerRead::Unreachable => panic!("the database was reachable"),
        LedgerRead::PlatformNotRegistered => panic!("the registry held platform"),
    }
}

fn state(
    s: &[(String, Result<LedgerState, LedgerFault>)],
    module: &str,
) -> Result<LedgerState, LedgerFault> {
    s.iter()
        .find(|(m, _)| m == module)
        .unwrap_or_else(|| panic!("no entry for {module}: {s:?}"))
        .1
        .clone()
}

fn modules(s: &[(String, Result<LedgerState, LedgerFault>)]) -> Vec<&str> {
    s.iter().map(|(m, _)| m.as_str()).collect()
}

#[tokio::test]
async fn lr1_a_migrated_database_is_current_for_every_set_in_the_order_given() {
    let sets = sets_with(vec![step(1, "SELECT 1"), step(2, "SELECT 2")]);
    let db = migrated("lr1", &sets).await;
    let ledger = DbLedger::new(pool(&db).await, sets);
    let got = answered(ledger.read().await);
    assert_eq!(modules(&got), ["platform", "identity", "zz"]);
    for (m, st) in &got {
        assert_eq!(st, &Ok(LedgerState::Current), "{m}");
    }
    assert_eq!(
        startup_check(&LedgerRead::Answered(got)),
        Ok(Startup::Current)
    );
}

#[tokio::test]
async fn lr2_a_database_with_roles_but_no_migrations_has_every_step_unapplied() {
    let sets = sets_with(vec![step(1, "SELECT 1")]);
    let db = roles_only("lr2").await;
    let ledger = DbLedger::new(pool(&db).await, sets.clone());
    let got = answered(ledger.read().await);
    for set in &sets {
        let want: Vec<u32> = set.steps.iter().map(|s| s.version).collect();
        assert_eq!(
            state(&got, set.module.as_str()),
            Ok(LedgerState::Unapplied { steps: want }),
            "{}",
            set.module.as_str()
        );
    }
    assert!(matches!(
        startup_check(&LedgerRead::Answered(got)),
        Ok(Startup::NotReady(_))
    ));
}

#[tokio::test]
async fn lr3_one_unapplied_step_makes_only_that_set_unapplied() {
    let applied = sets_with(vec![step(1, "SELECT 1")]);
    let db = migrated("lr3", &applied).await;
    let now = sets_with(vec![step(1, "SELECT 1"), step(2, "SELECT 2")]);
    let ledger = DbLedger::new(pool(&db).await, now);
    let got = answered(ledger.read().await);
    assert_eq!(state(&got, "platform"), Ok(LedgerState::Current));
    assert_eq!(state(&got, "identity"), Ok(LedgerState::Current));
    assert_eq!(
        state(&got, "zz"),
        Ok(LedgerState::Unapplied { steps: vec![2] })
    );
}

#[tokio::test]
async fn lr4_a_changed_step_is_reported_with_its_version_and_startup_refuses_naming_it() {
    let applied = sets_with(vec![step(1, "SELECT 1")]);
    let db = migrated("lr4", &applied).await;
    let now = sets_with(vec![step(1, "SELECT 2")]);
    let ledger = DbLedger::new(pool(&db).await, now);
    let read = ledger.read().await;
    let got = answered(read.clone());
    assert_eq!(
        state(&got, "zz"),
        Ok(LedgerState::ChecksumChanged { version: 1 })
    );
    assert_eq!(state(&got, "platform"), Ok(LedgerState::Current));
    assert_eq!(state(&got, "identity"), Ok(LedgerState::Current));
    let line = startup_check(&read).expect_err("refused").to_string();
    assert_eq!(
        line,
        "serve refused: migration 1 of zz was changed after it was applied"
    );
}

#[tokio::test]
async fn lr5_steps_the_binary_does_not_know_are_db_ahead() {
    let applied = sets_with(vec![step(1, "SELECT 1"), step(2, "SELECT 2")]);
    let db = migrated("lr5", &applied).await;
    let now = sets_with(vec![step(1, "SELECT 1")]);
    let ledger = DbLedger::new(pool(&db).await, now);
    let got = answered(ledger.read().await);
    assert_eq!(state(&got, "zz"), Ok(LedgerState::DbAhead));
    assert_eq!(state(&got, "platform"), Ok(LedgerState::Current));
}

#[tokio::test]
async fn lr6_a_database_nothing_listens_on_is_unreachable() {
    let db = roles_only("lr6").await;
    let dead = Arc::new(DbPool::connect_lazy(
        &db.dead_db_config(&["platform", "identity", "zz"], Arc::new(SystemClock)),
    ));
    let ledger = DbLedger::new(dead, sets_with(vec![]));
    assert_eq!(ledger.read().await, LedgerRead::Unreachable);
}

#[tokio::test]
async fn lr7a_a_pool_registry_without_platform_is_a_wiring_defect_not_an_outage() {
    let sets = sets_with(vec![]);
    let db = migrated("lr7a", &sets).await;
    // The registry the server hands the pool must hold `platform` (contract 4b: `Registry::db_modules`); this one does not.
    let pool = Arc::new(
        DbPool::connect(&db.db_config(&["identity", "zz"], Arc::new(SystemClock)))
            .await
            .expect("pool"),
    );
    let ledger = DbLedger::new(pool, sets);
    let read = ledger.read().await;
    assert_eq!(read, LedgerRead::PlatformNotRegistered);
    assert_eq!(
        startup_check(&read).expect_err("refused").to_string(),
        "serve refused: the database module registry does not hold platform"
    );
}

#[tokio::test]
async fn lr7b_a_database_that_answers_but_will_not_let_platform_in_is_reachable_and_unchecked() {
    let sets = sets_with(vec![]);
    let db = migrated("lr7b", &sets).await;
    // A login role that is a member of no module role: it connects, but `SET ROLE cortex_m_platform` is refused.
    let login = format!("lr7b_login_{}", std::process::id());
    let mut admin = db.admin().await;
    // The name is built here from our own pid: nothing outside the test reaches it.
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE ROLE {login} LOGIN PASSWORD 'lr7b-pw'"
    )))
    .execute(&mut admin)
    .await
    .expect("create the login role");
    let mut cfg = db.db_config(&["platform", "identity", "zz"], Arc::new(SystemClock));
    // The test database is closed to PUBLIC: only named roles may connect.
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "GRANT CONNECT ON DATABASE {} TO {login}",
        cfg.database
    )))
    .execute(&mut admin)
    .await
    .expect("let the login role connect");
    cfg.app_user = login.clone();
    cfg.app_password = cortex_types::Secret::new("lr7b-pw");
    let pool = Arc::new(
        DbPool::connect(&cfg)
            .await
            .expect("the login role connects"),
    );
    let ledger = DbLedger::new(pool, sets.clone());
    let got = answered(ledger.read().await);
    assert_eq!(got.len(), sets.len(), "one entry per set");
    for (m, st) in &got {
        assert_eq!(st, &Err(LedgerFault::Database), "{m}");
    }
    drop(ledger);
    let _ = sqlx::raw_sql(sqlx::AssertSqlSafe(format!("DROP ROLE {login}")))
        .execute(&mut admin)
        .await;
}

#[tokio::test]
async fn lr8_a_read_changes_nothing_and_gives_its_connection_back() {
    let sets = sets_with(vec![step(1, "SELECT 1")]);
    let db = migrated("lr8", &sets).await;
    let mut admin = db.admin().await;
    let fingerprint = "SELECT count(*)::text || ':' || coalesce(md5(string_agg(module || version::text || checksum, ',' ORDER BY module, version)), '') FROM platform.migrations";
    let before: String = sqlx::query_scalar(fingerprint)
        .fetch_one(&mut admin)
        .await
        .expect("fingerprint");
    let ledger = DbLedger::new(pool(&db).await, sets);
    // More reads than the pool has connections (the testkit pool holds 4): each must give its connection back.
    let all = async {
        for _ in 0..12 {
            assert!(matches!(ledger.read().await, LedgerRead::Answered(_)));
        }
    };
    tokio::time::timeout(Duration::from_secs(60), all)
        .await
        .expect("twelve reads on a pool of four returned");
    let after: String = sqlx::query_scalar(fingerprint)
        .fetch_one(&mut admin)
        .await
        .expect("fingerprint");
    assert_eq!(before, after, "the ledger is untouched");
    let idle: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_stat_activity WHERE datname = current_database() AND state = 'idle in transaction'",
    )
    .fetch_one(&mut admin)
    .await
    .expect("activity");
    assert_eq!(idle, 0, "no transaction was left open");
}

#[tokio::test]
async fn lr9_a_read_that_cannot_get_the_ledger_gives_up_at_its_timeout() {
    let sets = sets_with(vec![step(1, "SELECT 1")]);
    let db = migrated("lr9", &sets).await;
    // A lock on the ledger that is never released during the read.
    let mut holder = db.admin().await;
    sqlx::raw_sql("BEGIN; LOCK TABLE platform.migrations IN ACCESS EXCLUSIVE MODE")
        .execute(&mut holder)
        .await
        .expect("lock the ledger");
    let ledger = DbLedger::with_timeout(pool(&db).await, sets.clone(), Duration::from_millis(300));
    let started = std::time::Instant::now();
    let read = tokio::time::timeout(Duration::from_secs(20), ledger.read())
        .await
        .expect("the read returned instead of waiting for the lock");
    // The 300 ms asked for, not the 2 s default: a stuck database must not hold an open route for long.
    assert!(
        started.elapsed() < Duration::from_millis(1500),
        "gave up after {:?}",
        started.elapsed()
    );
    let got = answered(read);
    assert_eq!(got.len(), sets.len());
    assert!(
        got.iter().any(|(_, st)| st == &Err(LedgerFault::Database)),
        "the timeout is a database fault, not a stop and not 'current': {got:?}"
    );
    assert!(
        got.iter().all(|(_, st)| st != &Ok(LedgerState::Current)),
        "{got:?}"
    );
    sqlx::raw_sql("ROLLBACK")
        .execute(&mut holder)
        .await
        .expect("release");
}

#[tokio::test]
async fn lr10_a_set_whose_versions_do_not_increase_is_a_fault_and_startup_refuses_naming_it() {
    let sets = sets_with(vec![step(1, "SELECT 1")]);
    let db = migrated("lr10", &sets).await;
    let bad = sets_with(vec![step(2, "SELECT 2"), step(1, "SELECT 1")]);
    let ledger = DbLedger::new(pool(&db).await, bad);
    let read = ledger.read().await;
    let got = answered(read.clone());
    assert_eq!(state(&got, "zz"), Err(LedgerFault::OutOfOrder));
    assert_eq!(state(&got, "platform"), Ok(LedgerState::Current));
    let line = startup_check(&read).expect_err("refused").to_string();
    assert_eq!(
        line,
        "serve refused: the migration steps of zz are not in order"
    );
}
