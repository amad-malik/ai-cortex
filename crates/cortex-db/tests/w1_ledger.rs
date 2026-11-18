//! Wave 1 W1-41 (cox, 14:35Z; piece 11 `serve`, W1-38 B): the migration ledger can be READ by exactly one role, `cortex_m_platform`,
//! and `cortex_db::migrate::verify` compares a module's migration set with it, read-only, with the SAME checksum `apply` writes.
//! DB-backed (run under `crates/cortex-testkit/supervise.sh`). RED first.
//!
//! lg1 the grants; lg2 and lg3 the exact-set read-back in the platform step (a missing grant and an extra grant are both refused);
//! lg4 the four states and their precedence; lg5 the shared checksum formula; lg6 read-only; lg7 no ledger yet; lg8 the role
//! cannot write the ledger and no other module role can read it.
mod common;

use cortex_db::migrate::{LedgerState, apply, platform_migrations, verify};
use cortex_db::{DbPool, Migration, MigrationSet, OwnerConn};
use cortex_testkit::{empty_db, sqlstate, within};
use cortex_types::ModuleId;
use sqlx::Row;

fn step(version: u32, name: &'static str, up: &'static str, down: &'static str) -> Migration {
    Migration {
        version,
        name,
        up,
        down,
    }
}

fn set(module: &str, schema: &'static str, steps: Vec<Migration>) -> MigrationSet {
    MigrationSet {
        module: ModuleId::parse(module).expect("a module id"),
        schema,
        steps,
    }
}

/// An empty database with the platform step applied (as the owner), and a pool that can begin a `platform` transaction.
async fn platform_db(tag: &str) -> (&'static cortex_testkit::TestDb, OwnerConn, DbPool) {
    let db = Box::leak(Box::new(empty_db(tag).await));
    let mut owner = OwnerConn::connect(&db.owner_config()).await.expect("owner");
    apply(&mut owner, &platform_migrations())
        .await
        .expect("platform migrations");
    let pool = DbPool::connect(&db.db_config(&["platform", "test_a"], common::clock()))
        .await
        .expect("pool");
    (db, owner, pool)
}

async fn state(pool: &DbPool, s: &MigrationSet) -> LedgerState {
    let mut tx = common::begin(pool, "platform", None).await;
    let r = verify(&mut tx, s).await.expect("verify");
    tx.rollback().await.ok();
    r
}

// ---- the grants -----------------------------------------------------------------------------

#[tokio::test]
async fn lg1_only_cortex_m_platform_can_select_the_ledger_and_nothing_else_is_granted() {
    within(120, async {
        let (db, _owner, _pool) = platform_db("lg1").await;
        let mut admin = db.admin().await;
        let can = |role: &'static str, privilege: &'static str| {
            format!("select has_table_privilege('{role}', 'platform.migrations', '{privilege}')")
        };
        // The one reader.
        let selects: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(can("cortex_m_platform", "SELECT")))
            .fetch_one(&mut admin)
            .await
            .expect("privilege");
        assert!(selects, "cortex_m_platform must be able to SELECT the ledger");
        // ... and only to read it.
        for p in ["INSERT", "UPDATE", "DELETE", "TRUNCATE", "REFERENCES", "TRIGGER"] {
            let got: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(can("cortex_m_platform", p)))
                .fetch_one(&mut admin)
                .await
                .expect("privilege");
            assert!(!got, "cortex_m_platform must not have {p} on the ledger");
        }
        // Nobody else: another module role, the login role, and PUBLIC.
        for role in ["cortex_m_test_a", "cortex_m_test_b", "cortex_app"] {
            let got: bool = sqlx::query_scalar(sqlx::AssertSqlSafe(can(role, "SELECT")))
                .fetch_one(&mut admin)
                .await
                .expect("privilege");
            assert!(!got, "{role} must not be able to SELECT the ledger");
        }
        let public: i64 = sqlx::query_scalar(
            "select count(*) from pg_class c, aclexplode(c.relacl) a \
             where c.oid = 'platform.migrations'::regclass and a.grantee = 0",
        )
        .fetch_one(&mut admin)
        .await
        .expect("acl");
        assert_eq!(public, 0, "PUBLIC must hold nothing on the ledger");
        // The ACL is exactly that one grant, not grantable.
        let acl: Vec<(String, String, bool)> = sqlx::query(
            "select a.grantee::regrole::text, a.privilege_type, a.is_grantable \
             from pg_class c, aclexplode(c.relacl) a \
             where c.oid = 'platform.migrations'::regclass and a.grantee <> c.relowner order by 1, 2",
        )
        .fetch_all(&mut admin)
        .await
        .expect("acl")
        .into_iter()
        .map(|r| (r.get(0), r.get(1), r.get(2)))
        .collect();
        assert_eq!(
            acl,
            vec![("cortex_m_platform".to_owned(), "SELECT".to_owned(), false)]
        );
    })
    .await
}

// ---- the exact-set read-back ----------------------------------------------------------------

const THE_GRANT: &str = "GRANT SELECT ON platform.migrations TO cortex_m_platform;\n";

/// The platform step with `edit` applied to its SQL, on an empty database: does it apply?
async fn applies_with(tag: &str, edit: impl FnOnce(&str) -> String) -> bool {
    let db = empty_db(tag).await;
    let mut owner = OwnerConn::connect(&db.owner_config()).await.expect("owner");
    let real = platform_migrations();
    let up: &'static str = Box::leak(edit(real.steps[0].up).into_boxed_str());
    let variant = MigrationSet {
        steps: vec![step(1, real.steps[0].name, up, real.steps[0].down)],
        ..real
    };
    let applied = apply(&mut owner, &variant).await.is_ok();
    if !applied {
        // The failed step rolled back: no ledger, no schema.
        let mut admin = db.admin().await;
        let left: bool =
            sqlx::query_scalar("select to_regclass('platform.migrations') is not null")
                .fetch_one(&mut admin)
                .await
                .expect("probe");
        assert!(
            !left,
            "{tag}: a refused platform step left the ledger behind"
        );
    }
    applied
}

#[tokio::test]
async fn lg2_the_read_back_refuses_a_platform_step_that_lacks_the_grant() {
    within(120, async {
        let real = platform_migrations().steps[0].up;
        assert_eq!(
            real.matches(THE_GRANT).count(),
            1,
            "the platform step must grant SELECT on the ledger to cortex_m_platform, once"
        );
        assert!(
            !applies_with("lg2", |up| up.replace(THE_GRANT, "")).await,
            "the read-back must refuse a step without the grant"
        );
        // The control: the step as it is applies.
        assert!(applies_with("lg2c", str::to_owned).await);
    })
    .await
}

#[tokio::test]
async fn lg3_the_read_back_refuses_an_extra_grant_on_the_ledger() {
    within(180, async {
        for (i, (what, extra)) in [
            (
                "to the login role",
                "GRANT SELECT ON platform.migrations TO cortex_app;\n",
            ),
            (
                "to PUBLIC",
                "GRANT SELECT ON platform.migrations TO PUBLIC;\n",
            ),
            (
                "INSERT to the same role",
                "GRANT INSERT ON platform.migrations TO cortex_m_platform;\n",
            ),
            (
                "to another module role",
                "GRANT SELECT ON platform.migrations TO cortex_m_test_a;\n",
            ),
            (
                "WITH GRANT OPTION",
                "GRANT SELECT ON platform.migrations TO cortex_m_test_b WITH GRANT OPTION;\n",
            ),
        ]
        .into_iter()
        .enumerate()
        {
            assert!(
                !applies_with(&format!("lg3_{i}"), |up| up
                    .replace(THE_GRANT, &format!("{THE_GRANT}{extra}")))
                .await,
                "the read-back must refuse an extra grant {what}"
            );
        }
    })
    .await
}

// ---- verify: the four states ----------------------------------------------------------------

const S1_UP: &str = "CREATE SCHEMA lg_a";
const S2_UP: &str = "CREATE TABLE lg_a.t (id int)";

fn base() -> MigrationSet {
    set(
        "lg_a",
        "lg_a",
        vec![
            step(1, "schema", S1_UP, "DROP SCHEMA lg_a"),
            step(2, "table", S2_UP, "DROP TABLE lg_a.t"),
        ],
    )
}

#[tokio::test]
async fn lg4_verify_says_current_unapplied_checksum_changed_or_db_ahead() {
    within(180, async {
        let (_db, mut owner, pool) = platform_db("lg4").await;
        apply(&mut owner, &base())
            .await
            .expect("apply the base set");

        // Current: the set as applied.
        assert_eq!(state(&pool, &base()).await, LedgerState::Current);

        // Unapplied: steps the ledger has not seen, ascending, and only those.
        let mut more = base();
        more.steps.push(step(
            3,
            "c",
            "CREATE TABLE lg_a.u (id int)",
            "DROP TABLE lg_a.u",
        ));
        more.steps.push(step(
            4,
            "d",
            "CREATE TABLE lg_a.v (id int)",
            "DROP TABLE lg_a.v",
        ));
        assert_eq!(
            state(&pool, &more).await,
            LedgerState::Unapplied { steps: vec![3, 4] }
        );
        // A module the ledger has never heard of: every step.
        let fresh = set(
            "lg_b",
            "lg_b",
            vec![
                step(1, "a", "SELECT 1", "SELECT 1"),
                step(2, "b", "SELECT 2", "SELECT 2"),
            ],
        );
        assert_eq!(
            state(&pool, &fresh).await,
            LedgerState::Unapplied { steps: vec![1, 2] }
        );

        // ChecksumChanged: the UP text of an applied step, and its DOWN text, each alone.
        let mut up_changed = base();
        up_changed.steps[1].up = "CREATE TABLE lg_a.t (id bigint)";
        assert_eq!(
            state(&pool, &up_changed).await,
            LedgerState::ChecksumChanged { version: 2 }
        );
        let mut down_changed = base();
        down_changed.steps[0].down = "DROP SCHEMA lg_a CASCADE";
        assert_eq!(
            state(&pool, &down_changed).await,
            LedgerState::ChecksumChanged { version: 1 }
        );

        // DbAhead: the ledger has a step this set does not know.
        let mut older = base();
        older.steps.truncate(1);
        assert_eq!(state(&pool, &older).await, LedgerState::DbAhead);

        // Precedence: a changed checksum beats everything, a DbAhead beats an unapplied step.
        let mut changed_and_ahead = base();
        changed_and_ahead.steps.truncate(1);
        changed_and_ahead.steps[0].up = "CREATE SCHEMA lg_a_changed";
        assert_eq!(
            state(&pool, &changed_and_ahead).await,
            LedgerState::ChecksumChanged { version: 1 },
            "the changed checksum is reported although the ledger is also ahead"
        );
        let mut ahead_and_unapplied = base();
        ahead_and_unapplied.steps.remove(1);
        ahead_and_unapplied.steps.push(step(
            3,
            "c",
            "CREATE TABLE lg_a.u (id int)",
            "DROP TABLE lg_a.u",
        ));
        assert_eq!(
            state(&pool, &ahead_and_unapplied).await,
            LedgerState::DbAhead,
            "step 2 is applied but unknown, step 3 is unapplied: ahead wins"
        );
    })
    .await
}

// ---- verify: the checksum formula is the one apply writes -------------------------------------

#[tokio::test]
async fn lg5_verify_agrees_with_apply_on_awkward_text_and_sees_a_one_character_change() {
    within(180, async {
        let (_db, mut owner, pool) = platform_db("lg5").await;
        // Text a hand-written formula would get wrong: non-ASCII, quotes, backslashes, CR LF, tabs, trailing spaces.
        let awkward: &'static str =
            "CREATE SCHEMA lg_f; COMMENT ON SCHEMA lg_f IS $x$é€漢字 \\ ' \" \r\n\t  $x$   ";
        let down: &'static str = "DROP SCHEMA lg_f  \r\n";
        let f = set("lg_f", "lg_f", vec![step(1, "awkward", awkward, down)]);
        apply(&mut owner, &f).await.expect("apply");
        assert_eq!(state(&pool, &f).await, LedgerState::Current);
        // One character of the up text.
        let mut a = f.clone();
        a.steps[0].up =
            "CREATE SCHEMA lg_f; COMMENT ON SCHEMA lg_f IS $x$é€漢字 \\ ' \" \r\n\t  $x$    ";
        assert_eq!(
            state(&pool, &a).await,
            LedgerState::ChecksumChanged { version: 1 }
        );
        // One character of the down text.
        let mut b = f.clone();
        b.steps[0].down = "DROP SCHEMA lg_f  \r\n ";
        assert_eq!(
            state(&pool, &b).await,
            LedgerState::ChecksumChanged { version: 1 }
        );
        // The boundary between them is part of the sum: moving a character from one to the other changes it.
        let mut c = f.clone();
        c.steps[0].up =
            "CREATE SCHEMA lg_f; COMMENT ON SCHEMA lg_f IS $x$é€漢字 \\ ' \" \r\n\t  $x$   D";
        c.steps[0].down = "ROP SCHEMA lg_f  \r\n";
        assert_eq!(
            state(&pool, &c).await,
            LedgerState::ChecksumChanged { version: 1 }
        );
    })
    .await
}

// ---- verify reads and nothing else ------------------------------------------------------------

#[tokio::test]
async fn lg6_verify_writes_no_row_and_takes_no_advisory_lock() {
    within(120, async {
        let (db, mut owner, pool) = platform_db("lg6").await;
        apply(&mut owner, &base()).await.expect("apply");
        let mut admin = db.admin().await;
        let before: i64 = sqlx::query_scalar("select count(*) from platform.migrations")
            .fetch_one(&mut admin)
            .await
            .expect("count");
        let mut tx = common::begin(&pool, "platform", None).await;
        assert_eq!(
            verify(&mut tx, &base()).await.expect("verify"),
            LedgerState::Current
        );
        assert!(
            tx.lock_ledger().steps().is_empty(),
            "verify must take no advisory lock (a handler's authorize may not either)"
        );
        tx.commit().await.expect("commit");
        let after: i64 = sqlx::query_scalar("select count(*) from platform.migrations")
            .fetch_one(&mut admin)
            .await
            .expect("count");
        assert_eq!(before, after);
    })
    .await
}

#[tokio::test]
async fn lg7_with_no_ledger_yet_every_step_is_unapplied_not_an_error() {
    within(120, async {
        let db = empty_db("lg7").await;
        let pool = DbPool::connect(&db.db_config(&["platform"], common::clock()))
            .await
            .expect("pool");
        assert_eq!(
            state(&pool, &platform_migrations()).await,
            LedgerState::Unapplied { steps: vec![1] },
            "even the platform set itself, before its own step has run"
        );
        assert_eq!(
            state(&pool, &base()).await,
            LedgerState::Unapplied { steps: vec![1, 2] }
        );
    })
    .await
}

// ---- who can do what ------------------------------------------------------------------------

#[tokio::test]
async fn lg8_the_platform_role_cannot_write_the_ledger_and_another_module_role_cannot_read_it() {
    within(120, async {
        let (_db, mut owner, pool) = platform_db("lg8").await;
        apply(&mut owner, &base()).await.expect("apply");
        // platform: reads, never writes.
        for (what, sql) in [
            ("insert", "insert into platform.migrations (module, version, name, checksum) values ('x', 1, 'x', 'x')"),
            ("update", "update platform.migrations set name = name"),
            ("delete", "delete from platform.migrations"),
        ] {
            let mut tx = common::begin(&pool, "platform", None).await;
            let e = sqlx::query(sql)
                .execute(tx.executor().expect("executor"))
                .await
                .expect_err(what);
            assert_eq!(sqlstate(&e).as_deref(), Some("42501"), "platform {what}");
            tx.rollback().await.ok();
        }
        let mut tx = common::begin(&pool, "platform", None).await;
        let n: i64 = sqlx::query_scalar("select count(*) from platform.migrations")
            .fetch_one(tx.executor().expect("executor"))
            .await
            .expect("the platform role reads the ledger");
        assert!(n >= 1);
        tx.rollback().await.ok();
        // another module role: not even a read.
        let mut tx = common::begin(&pool, "test_a", None).await;
        let e = sqlx::query("select count(*) from platform.migrations")
            .fetch_one(tx.executor().expect("executor"))
            .await
            .expect_err("test_a must not read the ledger");
        assert_eq!(sqlstate(&e).as_deref(), Some("42501"));
        tx.rollback().await.ok();
    })
    .await
}

// ---- verify: the same refusals and the same choice as apply ------------------------------------

#[tokio::test]
async fn lg9_verify_refuses_an_out_of_order_set_and_reports_the_lowest_changed_step() {
    within(180, async {
        let (_db, mut owner, pool) = platform_db("lg9").await;
        apply(&mut owner, &base()).await.expect("apply");
        // As `apply`: versions that do not increase are OutOfOrder, before the ledger is read.
        for (what, steps) in [
            (
                "out of order",
                vec![step(2, "b", S2_UP, "x"), step(1, "a", S1_UP, "y")],
            ),
            (
                "duplicate",
                vec![step(1, "a", S1_UP, "y"), step(1, "again", S2_UP, "x")],
            ),
        ] {
            let mut tx = common::begin(&pool, "platform", None).await;
            let r = verify(&mut tx, &set("lg_a", "lg_a", steps)).await;
            tx.rollback().await.ok();
            assert!(
                matches!(r, Err(cortex_db::MigrateError::OutOfOrder)),
                "{what}: {r:?}"
            );
        }
        // Two applied steps both changed: the LOWEST version is reported.
        let mut both = base();
        both.steps[0].down = "DROP SCHEMA lg_a CASCADE";
        both.steps[1].up = "CREATE TABLE lg_a.t (id bigint)";
        assert_eq!(
            state(&pool, &both).await,
            LedgerState::ChecksumChanged { version: 1 }
        );
        // And the unapplied steps are listed in ascending order whatever else is true of the ledger.
        let mut gaps = set("lg_g", "lg_g", vec![step(1, "a", "SELECT 1", "SELECT 1")]);
        apply(&mut owner, &gaps).await.expect("apply g");
        gaps.steps.push(step(5, "e", "SELECT 5", "SELECT 5"));
        gaps.steps.push(step(9, "i", "SELECT 9", "SELECT 9"));
        assert_eq!(
            state(&pool, &gaps).await,
            LedgerState::Unapplied { steps: vec![5, 9] }
        );
    })
    .await
}
