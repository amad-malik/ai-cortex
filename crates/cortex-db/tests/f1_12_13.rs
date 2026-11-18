//! F1-12 Ended connections and statement deadlines. [BASE] (S2 §9.1; S1 §9.)
//! F1-13 Migrations. [BASE] (S1 §2.2.)
mod common;

use std::time::Duration;

use common::{begin, pool, project};
use cortex_db::migrate::{apply, platform_migrations, revert_last};
use cortex_db::{MigrateError, Migration, MigrationSet, ModuleTx, OwnerConn, TxOptions};
use cortex_testkit::{ctx, module, session_identity, sqlstate, test_db, within};
use sqlx::Row;

#[tokio::test]
async fn f1_12_a_connection_the_server_ends_is_replaced() {
    within(60, async {
        let (db, pool) = pool(1).await;
        let mut tx = begin(&pool, "test_a", None).await;
        let s = session_identity(tx.executor().expect("executor")).await;
        tx.commit().await.expect("commit");
        let mut admin = db.admin().await;
        sqlx::query("select pg_terminate_backend($1)")
            .bind(s.pid)
            .execute(&mut admin)
            .await
            .expect("terminate");
        // The next ModuleTx gets a working connection (at most one failed attempt is tolerated).
        let mut ok = false;
        for _ in 0..2 {
            if let Ok(mut tx) =
                ModuleTx::begin(&pool, ctx("test_a", None), TxOptions::default()).await
                && sqlx::query("select 1")
                    .execute(tx.executor().expect("executor"))
                    .await
                    .is_ok()
            {
                ok = true;
                tx.rollback().await.ok();
                break;
            }
        }
        assert!(
            ok,
            "no working connection after the server ended the session"
        );
    })
    .await
}

#[tokio::test]
async fn f1_12_a_statement_deadline_ends_the_statement_and_the_transaction() {
    within(60, async {
        let (db, pool) = pool(1).await;
        let p = project();
        let opts = TxOptions {
            statement_timeout: Some(Duration::from_millis(500)),
        };
        let mut tx = ModuleTx::begin(&pool, ctx("test_a", Some(p)), opts)
            .await
            .expect("begin");
        sqlx::query("insert into test_a.t (project_id, v) values ($1, 'f1-12')")
            .bind(common::uuid_of(p))
            .execute(tx.executor().expect("executor"))
            .await
            .expect("insert");
        let e = sqlx::query("select pg_sleep(3)")
            .execute(tx.executor().expect("executor"))
            .await
            .expect_err("the deadline");
        assert_eq!(sqlstate(&e).as_deref(), Some("57014"));
        tx.rollback().await.ok();
        assert_eq!(
            common::count_as_admin(db, "test_a.t", p).await,
            0,
            "the transaction did not roll back"
        );
        // Control: with no deadline option, the same statement runs past it.
        let mut tx = begin(&pool, "test_a", None).await;
        sqlx::query("select pg_sleep(1)")
            .execute(tx.executor().expect("executor"))
            .await
            .expect("no deadline: completes");
        tx.rollback().await.ok();
    })
    .await
}

fn set(module_id: &str, schema: &'static str, steps: Vec<Migration>) -> MigrationSet {
    MigrationSet {
        module: module(module_id),
        schema,
        steps,
    }
}

fn test_m_steps() -> Vec<Migration> {
    vec![
        Migration {
            version: 1,
            name: "create",
            up: "CREATE SCHEMA test_m; CREATE TABLE test_m.a (id int PRIMARY KEY)",
            down: "DROP TABLE test_m.a; DROP SCHEMA test_m",
        },
        Migration {
            version: 2,
            name: "add_b",
            up: "ALTER TABLE test_m.a ADD COLUMN b int",
            down: "ALTER TABLE test_m.a DROP COLUMN b",
        },
    ]
}

/// The catalogue of `test_m`: its relations and their columns, in a stable order.
async fn catalogue(admin: &mut sqlx::PgConnection) -> Vec<String> {
    sqlx::query(
        "select c.relname || '.' || a.attname || ':' || format_type(a.atttypid, a.atttypmod) \
         from pg_class c join pg_namespace n on n.oid = c.relnamespace join pg_attribute a on a.attrelid = c.oid \
         where n.nspname = 'test_m' and a.attnum > 0 and not a.attisdropped order by 1",
    )
    .fetch_all(admin)
    .await
    .expect("catalogue")
    .into_iter()
    .map(|r| r.get(0))
    .collect()
}

#[tokio::test]
async fn f1_13_migrations_run_in_order_and_round_trip() {
    within(120, async {
        let db = test_db().await;
        let mut owner = OwnerConn::connect(&db.owner_config()).await.expect("owner");
        apply(&mut owner, &platform_migrations())
            .await
            .expect("platform migrations");
        let m = set("test_m", "test_m", test_m_steps());
        apply(&mut owner, &m).await.expect("up");
        let mut admin = db.admin().await;
        let first = catalogue(&mut admin).await;
        assert_eq!(
            first,
            vec![
                "a.b:integer".to_owned(),
                "a.id:integer".to_owned(),
                "a_pkey.id:integer".to_owned()
            ]
        );
        revert_last(&mut owner, &m).await.expect("down 2");
        revert_last(&mut owner, &m).await.expect("down 1");
        assert!(catalogue(&mut admin).await.is_empty(), "down left objects");
        apply(&mut owner, &m).await.expect("up again");
        assert_eq!(
            catalogue(&mut admin).await,
            first,
            "up, down, up changed the catalogue"
        );
        let ran_as: Vec<String> = sqlx::query(
            "select applied_by from platform.migrations where module = 'test_m' order by version",
        )
        .fetch_all(&mut admin)
        .await
        .expect("ledger")
        .into_iter()
        .map(|r| r.get(0))
        .collect();
        assert_eq!(
            ran_as,
            vec!["cortex_owner".to_owned(), "cortex_owner".to_owned()],
            "migrations run as cortex_owner"
        );

        // A changed checksum on an applied step stops startup.
        let mut changed = test_m_steps();
        changed[1].up = "ALTER TABLE test_m.a ADD COLUMN b bigint";
        assert_eq!(
            apply(&mut owner, &set("test_m", "test_m", changed)).await,
            Err(MigrateError::ChecksumChanged { version: 2 })
        );
    })
    .await
}

/// D11 (mike's R0): out-of-order and duplicate versions are refused with `OutOfOrder` before any
/// step runs, so the planned "accept out-of-order" mutation at GREEN has a discriminating input.
#[tokio::test]
async fn f1_13_out_of_order_or_duplicate_versions_are_refused_before_any_step() {
    within(120, async {
        let db = test_db().await;
        let mut owner = OwnerConn::connect(&db.owner_config()).await.expect("owner");
        apply(&mut owner, &platform_migrations())
            .await
            .expect("platform migrations");
        let step = |version: u32, name: &'static str, up: &'static str| Migration {
            version,
            name,
            up,
            down: "SELECT 1",
        };
        for (what, steps) in [
            (
                "out of order",
                vec![
                    step(2, "second", "CREATE TABLE test_m_order.b (id int)"),
                    step(1, "first", "CREATE SCHEMA test_m_order"),
                ],
            ),
            (
                "duplicate",
                vec![
                    step(1, "first", "CREATE SCHEMA test_m_order"),
                    step(1, "again", "CREATE TABLE test_m_order.b (id int)"),
                ],
            ),
        ] {
            assert_eq!(
                apply(&mut owner, &set("test_m", "test_m_order", steps)).await,
                Err(MigrateError::OutOfOrder),
                "{what}"
            );
            let mut admin = db.admin().await;
            let (schemas, ledger): (i64, i64) = sqlx::query_as(
                "select (select count(*) from pg_namespace where nspname = 'test_m_order'), \
                 (select count(*) from platform.migrations where module = 'test_m' and name in ('first', 'second', 'again'))",
            )
            .fetch_one(&mut admin)
            .await
            .expect("probe");
            assert_eq!((schemas, ledger), (0, 0), "{what}: a step ran");
        }
    })
    .await
}

/// F1-13's control: a step in `test_a`'s folder that creates `test_b.x` is refused, names the
/// object, and is rolled back.
#[tokio::test]
async fn f1_13_control_a_step_outside_its_schema_is_refused_and_rolled_back() {
    within(120, async {
        let db = test_db().await;
        let mut owner = OwnerConn::connect(&db.owner_config()).await.expect("owner");
        apply(&mut owner, &platform_migrations()).await.expect("platform migrations");
        let bad = set("test_a", "test_a", vec![Migration { version: 1, name: "escape", up: "CREATE TABLE test_b.x (id int)", down: "DROP TABLE test_b.x" }]);
        assert_eq!(apply(&mut owner, &bad).await, Err(MigrateError::OutsideSchema { version: 1, object: "test_b.x".into() }));
        let mut admin = db.admin().await;
        let n: i64 = sqlx::query("select count(*) from pg_class c join pg_namespace n on n.oid = c.relnamespace where n.nspname = 'test_b' and c.relname = 'x'")
            .fetch_one(&mut admin)
            .await
            .expect("probe")
            .get(0);
        assert_eq!(n, 0, "test_b.x survived the refusal");
    })
    .await
}
