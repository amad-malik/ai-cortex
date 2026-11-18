//! F-1b rework 1 (kai's row 4598a4ed; vera's F1B-GREEN-MIGRATION-CONFINEMENT on a1d1171): two
//! outside-schema changes the migrator's guard missed, under F1-13's contract (refused with
//! `OutsideSchema`, the object named, the step rolled back), each with an in-schema control.
//! - a catalogue write made in a CHILD transaction (a PL/pgSQL exception block);
//! - the loss of one of two OVERLOADED functions (the name `schema.f` survives).
//!
//! The guard catches mistakes in trusted release migrations over the catalogues it reads; it is
//! not full confinement, and no hostile-migration containment is claimed. Each case has a module id
//! of its own, so an accepted step (on a1d1171) records nothing another case reads.

use cortex_db::OwnerConn;
use cortex_db::migrate::{MigrateError, Migration, MigrationSet, apply, platform_migrations};
use cortex_testkit::{TestDb, module, test_db, within};
use sqlx::Row;

fn set(module_id: &str, schema: &'static str, steps: Vec<Migration>) -> MigrationSet {
    MigrationSet {
        module: module(module_id),
        schema,
        steps,
    }
}

async fn ready() -> (&'static TestDb, OwnerConn) {
    let db = test_db().await;
    let mut owner = OwnerConn::connect(&db.owner_config()).await.expect("owner");
    apply(&mut owner, &platform_migrations())
        .await
        .expect("platform migrations");
    (db, owner)
}

async fn relations(db: &TestDb, schema: &str, name: &str) -> i64 {
    let mut admin = db.admin().await;
    sqlx::query("select count(*) from pg_class c join pg_namespace n on n.oid = c.relnamespace where n.nspname = $1 and c.relname = $2")
        .bind(schema)
        .bind(name)
        .fetch_one(&mut admin)
        .await
        .expect("pg_class")
        .get(0)
}

/// The identities of `schema.name`'s overloads, e.g. `["integer", "text"]`.
async fn overloads(db: &TestDb, schema: &str, name: &str) -> Vec<String> {
    let mut admin = db.admin().await;
    sqlx::query("select pg_get_function_identity_arguments(p.oid) from pg_proc p join pg_namespace n on n.oid = p.pronamespace where n.nspname = $1 and p.proname = $2 order by 1")
        .bind(schema)
        .bind(name)
        .fetch_all(&mut admin)
        .await
        .expect("pg_proc")
        .into_iter()
        .map(|r| r.get(0))
        .collect()
}

/// Two overloads of `schema.name`, made and committed by the owner.
async fn make_overloads(db: &TestDb, schema: &str, name: &str) {
    let mut owner = db.owner().await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE FUNCTION {schema}.{name}(integer) RETURNS integer LANGUAGE sql AS $$ SELECT 1 $$; \
         CREATE FUNCTION {schema}.{name}(text) RETURNS integer LANGUAGE sql AS $$ SELECT 2 $$;"
    )))
    .execute(&mut owner)
    .await
    .expect("the two overloads");
}

#[tokio::test]
async fn rw1_a_child_transaction_write_outside_the_schema_is_refused_and_rolled_back() {
    within(120, async {
        let (db, mut owner) = ready().await;
        let bad = set(
            "rw1_bad_sub",
            "test_a",
            vec![Migration {
                version: 1,
                name: "escape_in_a_child_transaction",
                up: "DO $$ BEGIN CREATE TABLE test_b.rw1_sub (id int); \
                     EXCEPTION WHEN duplicate_table THEN NULL; END $$",
                down: "DROP TABLE test_b.rw1_sub",
            }],
        );
        assert_eq!(
            apply(&mut owner, &bad).await,
            Err(MigrateError::OutsideSchema {
                version: 1,
                object: "test_b.rw1_sub".into()
            })
        );
        assert_eq!(
            relations(db, "test_b", "rw1_sub").await,
            0,
            "test_b.rw1_sub survived the refusal"
        );
    })
    .await
}

#[tokio::test]
async fn rw1_control_a_child_transaction_write_inside_the_schema_is_accepted() {
    within(120, async {
        let (db, mut owner) = ready().await;
        let good = set(
            "rw1_ctl_sub",
            "test_a",
            vec![Migration {
                version: 1,
                name: "create_in_a_child_transaction",
                up: "DO $$ BEGIN CREATE TABLE test_a.rw1_sub_ok (id int); \
                     EXCEPTION WHEN duplicate_table THEN NULL; END $$",
                down: "DROP TABLE test_a.rw1_sub_ok",
            }],
        );
        apply(&mut owner, &good)
            .await
            .expect("a child-transaction write inside the schema");
        assert_eq!(relations(db, "test_a", "rw1_sub_ok").await, 1);
    })
    .await
}

#[tokio::test]
async fn rw1_dropping_one_overload_outside_the_schema_is_refused_and_rolled_back() {
    within(120, async {
        let (db, mut owner) = ready().await;
        make_overloads(db, "test_b", "rw1_ovl").await;
        let bad = set(
            "rw1_bad_ovl",
            "test_a",
            vec![Migration {
                version: 1,
                name: "drop_an_outside_overload",
                up: "DROP FUNCTION test_b.rw1_ovl(integer)",
                down: "CREATE FUNCTION test_b.rw1_ovl(integer) RETURNS integer LANGUAGE sql AS $$ SELECT 1 $$",
            }],
        );
        assert_eq!(
            apply(&mut owner, &bad).await,
            Err(MigrateError::OutsideSchema {
                version: 1,
                object: "test_b.rw1_ovl(integer)".into()
            })
        );
        assert_eq!(
            overloads(db, "test_b", "rw1_ovl").await,
            ["integer", "text"],
            "the dropped overload was not restored"
        );
    })
    .await
}

#[tokio::test]
async fn rw1_control_dropping_one_overload_inside_the_schema_is_accepted() {
    within(120, async {
        let (db, mut owner) = ready().await;
        make_overloads(db, "test_a", "rw1_ovl_ok").await;
        let good = set(
            "rw1_ctl_ovl",
            "test_a",
            vec![Migration {
                version: 1,
                name: "drop_an_own_overload",
                up: "DROP FUNCTION test_a.rw1_ovl_ok(integer)",
                down: "CREATE FUNCTION test_a.rw1_ovl_ok(integer) RETURNS integer LANGUAGE sql AS $$ SELECT 1 $$",
            }],
        );
        apply(&mut owner, &good)
            .await
            .expect("dropping an overload inside the schema");
        assert_eq!(overloads(db, "test_a", "rw1_ovl_ok").await, ["text"]);
    })
    .await
}
