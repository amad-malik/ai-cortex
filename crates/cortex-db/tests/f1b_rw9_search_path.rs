//! F-1b rework 9, item 1 (kai's row 61c6f360 and T-2 Addendum 3). The guard's own observation SQL
//! resolves operators (and cast types) through the session's search_path. A step that creates an
//! always-false `<>` on oid in its own schema and puts its schema ahead of pg_catalog hides the
//! link it then makes from the topology checks: the author reproduced it on PostgreSQL 18.4
//! (runs-rw9), and the session kept the step's path for the next step too. The guard now resets
//! the step's settings and pins search_path to pg_catalog for every observation, under apply and
//! revert_last. Control: a harmless step with its own search_path passes.
//!
//! Each case has a database of its own. A refusal is matched by its `Debug` form; the temporary
//! schema's number is the session's, so only its digits are free.
use cortex_db::OwnerConn;
use cortex_db::migrate::{
    MigrateError, Migration, MigrationSet, apply, platform_migrations, revert_last,
};
use cortex_testkit::{TestDb, isolated_db, module, within};

fn one(module_id: &str, up: &'static str, down: &'static str) -> MigrationSet {
    MigrationSet {
        module: module(module_id),
        schema: "test_a",
        steps: vec![Migration {
            version: 1,
            name: "path",
            up,
            down,
        }],
    }
}

async fn ready(tag: &str) -> (TestDb, OwnerConn) {
    let db = isolated_db(tag).await;
    let mut owner = OwnerConn::connect(&db.owner_config()).await.expect("owner");
    apply(&mut owner, &platform_migrations())
        .await
        .expect("platform migrations");
    (db, owner)
}

async fn ledger(db: &TestDb, module_id: &'static str) -> i64 {
    let mut admin = db.admin().await;
    sqlx::query_scalar("select count(*) from platform.migrations where module = $1")
        .bind(module_id)
        .fetch_one(&mut admin)
        .await
        .expect("ledger")
}

/// Links across schemas, seen from another session with pg_catalog's own operator.
async fn links(db: &TestDb) -> i64 {
    let mut admin = db.admin().await;
    sqlx::query_scalar(
        "select count(*) from pg_catalog.pg_inherits i \
         join pg_catalog.pg_class c on c.oid operator(pg_catalog.=) i.inhrelid \
         join pg_catalog.pg_class p on p.oid operator(pg_catalog.=) i.inhparent \
         where c.relnamespace operator(pg_catalog.<>) p.relnamespace",
    )
    .fetch_one(&mut admin)
    .await
    .expect("links")
}

/// An always-false `<>` on oid, in the step's own schema, and an own table.
const SHADOW: &str = "CREATE FUNCTION test_a.rw9_never(oid, oid) RETURNS boolean LANGUAGE sql IMMUTABLE AS 'SELECT false'; \
     CREATE OPERATOR test_a.<> (LEFTARG = oid, RIGHTARG = oid, FUNCTION = test_a.rw9_never); \
     CREATE TABLE test_a.rw9_base (k int);";
/// The path set ahead of pg_catalog, then a temporary child of the own table.
const HIDE_LINK: &str = "SET search_path = test_a, pg_catalog; \
     CREATE TEMP TABLE rw9_tmp () INHERITS (test_a.rw9_base);";

fn left_temp_link(result: Result<(), MigrateError>) {
    let err =
        result.expect_err("a step that hides its link behind a shadowing operator is refused");
    let debug = format!("{err:?}");
    let digits = debug
        .strip_prefix("StepLeftTopologyNotSchemaClosed { version: 1, link: \"pg_temp_")
        .and_then(|r| r.strip_suffix(".rw9_tmp is an inheritance child of test_a.rw9_base\" }"));
    assert!(
        digits.is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit())),
        "the postcondition sees the link through pg_catalog's operator: {debug}"
    );
}

async fn next_runs(db: &TestDb, owner: &mut OwnerConn) {
    let next = one(
        "rw9_next",
        "CREATE TABLE test_a.rw9_next (k int);",
        "DROP TABLE test_a.rw9_next;",
    );
    apply(owner, &next)
        .await
        .expect("the next step on the same session");
    assert_eq!(ledger(db, "rw9_next").await, 1);
}

#[tokio::test]
async fn rw9_a_shadowing_operator_cannot_hide_a_link_under_apply() {
    within(120, async {
        let (db, mut owner) = ready("rw9_apply").await;
        let up: &'static str = Box::leak(format!("{SHADOW} {HIDE_LINK}").into_boxed_str());
        left_temp_link(apply(&mut owner, &one("rw9_shadow", up, "SELECT 1;")).await);
        assert_eq!(ledger(&db, "rw9_shadow").await, 0, "not recorded");
        assert_eq!(links(&db).await, 0, "no link after the refusal");
        next_runs(&db, &mut owner).await;
    })
    .await
}

#[tokio::test]
async fn rw9_a_shadowing_operator_cannot_hide_a_link_under_revert() {
    within(120, async {
        let (db, mut owner) = ready("rw9_revert").await;
        let set = one("rw9_shadow", SHADOW, HIDE_LINK);
        apply(&mut owner, &set).await.expect("the up step");
        assert_eq!(ledger(&db, "rw9_shadow").await, 1, "premise: applied");
        left_temp_link(revert_last(&mut owner, &set).await);
        assert_eq!(
            ledger(&db, "rw9_shadow").await,
            1,
            "rolled back: still recorded"
        );
        assert_eq!(links(&db).await, 0, "no link after the refusal");
        next_runs(&db, &mut owner).await;
    })
    .await
}

/// A step's own settings do not outlive it: the next step, and the guard observing it, start
/// from the session's defaults.
#[tokio::test]
async fn rw9_a_steps_own_settings_do_not_outlive_it() {
    within(120, async {
        let (_db, mut owner) = ready("rw9_reset").await;
        let before: String = sqlx::query_scalar("SHOW search_path")
            .fetch_one(owner.conn())
            .await
            .expect("path");
        apply(
            &mut owner,
            &one(
                "rw9_sets",
                "SET search_path = test_a, pg_catalog; CREATE TABLE rw9_set (k int);",
                "DROP TABLE test_a.rw9_set;",
            ),
        )
        .await
        .expect("a step with its own search_path");
        let after: String = sqlx::query_scalar("SHOW search_path")
            .fetch_one(owner.conn())
            .await
            .expect("path");
        assert_eq!(after, before, "the step's search_path did not outlive it");
    })
    .await
}

/// Control: a harmless step with its own search_path (an unqualified table lands in its schema)
/// passes and is recorded.
#[tokio::test]
async fn rw9_control_a_harmless_step_with_its_own_search_path_passes() {
    within(120, async {
        let (db, mut owner) = ready("rw9_harmless").await;
        apply(
            &mut owner,
            &one(
                "rw9_harmless",
                "SET search_path = test_a, pg_catalog; CREATE TABLE rw9_ok (k int);",
                "DROP TABLE test_a.rw9_ok;",
            ),
        )
        .await
        .expect("a harmless step with its own search_path");
        assert_eq!(ledger(&db, "rw9_harmless").await, 1, "recorded");
        let mut admin = db.admin().await;
        let table: String = sqlx::query_scalar(
            "select coalesce(pg_catalog.to_regclass('test_a.rw9_ok')::text, 'absent')",
        )
        .fetch_one(&mut admin)
        .await
        .expect("probe");
        assert_eq!(table, "test_a.rw9_ok");
    })
    .await
}
