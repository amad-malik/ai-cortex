//! F-1b rework 8, part 2 (kai's row 55963949 and the T-2 Addendum 2; vera's and sam's
//! F1B-RW6-DEFERRED-FINALIZATION). PostgreSQL's COMMIT runs user code after the guard's last
//! observation: pending deferred triggers, and the freezing of holdable cursors (which runs the
//! rest of their query). A step the guard accepted could so make, at COMMIT, a cross-schema link
//! or a table in another schema, and stay recorded; the author reproduced all four on
//! PostgreSQL 18.4 (runs-rw8). The guard now fires the deferred triggers before its observations
//! (`SET CONSTRAINTS ALL IMMEDIATE`) and refuses an open holdable cursor.
//!
//! Each negative case has a database of its own. A refusal from a NEW error is matched by its
//! `Debug` form, so these tests-only cases compile before the variant exists. Control: a harmless
//! deferred trigger runs, and the step passes. The earlier tests are unchanged.
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
            name: "late",
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

async fn scalar(db: &TestDb, sql: &'static str) -> String {
    let mut admin = db.admin().await;
    sqlx::query_scalar(sql)
        .fetch_one(&mut admin)
        .await
        .expect("probe")
}

async fn ledger(db: &TestDb, module_id: &'static str) -> i64 {
    let mut admin = db.admin().await;
    sqlx::query_scalar("select count(*) from platform.migrations where module = $1")
        .bind(module_id)
        .fetch_one(&mut admin)
        .await
        .expect("ledger")
}

/// Links across schemas, seen from another session.
const LINKS: &str = "select count(*)::text from pg_inherits i join pg_class c on c.oid = i.inhrelid \
     join pg_class p on p.oid = i.inhparent where c.relnamespace <> p.relnamespace";

/// The next, unrelated step on the same session runs and is recorded.
async fn next_runs(db: &TestDb, owner: &mut OwnerConn) {
    let next = one(
        "rw8_next",
        "CREATE TABLE test_a.rw8_next (k int);",
        "DROP TABLE test_a.rw8_next;",
    );
    apply(owner, &next)
        .await
        .expect("the next step on the same session is not blocked");
    assert_eq!(ledger(db, "rw8_next").await, 1);
}

/// Refused after the step ran, naming the temporary link it left: `pg_temp_<n>.rw8_tmp` under
/// `test_a.rw8_base`.
fn left_temp_link(result: Result<(), MigrateError>) {
    let err = result.expect_err("a step whose deferred work links a temporary child is refused");
    let debug = format!("{err:?}");
    let digits = debug
        .strip_prefix("StepLeftTopologyNotSchemaClosed { version: 1, link: \"pg_temp_")
        .and_then(|r| r.strip_suffix(".rw8_tmp is an inheritance child of test_a.rw8_base\" }"));
    assert!(
        digits.is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit())),
        "the postcondition's refusal, after the deferred work ran: {debug}"
    );
}

/// An own table, a function that links a temporary child to it, and a deferred constraint
/// trigger on another own table that calls the function; nothing is queued yet.
const DEFERRED_LINK: &str = "CREATE TABLE test_a.rw8_base (k int); \
     CREATE FUNCTION test_a.rw8_late() RETURNS trigger LANGUAGE plpgsql AS $f$ BEGIN \
       CREATE TEMP TABLE rw8_tmp () INHERITS (test_a.rw8_base); RETURN NULL; END $f$; \
     CREATE TABLE test_a.rw8_q (k int); \
     CREATE CONSTRAINT TRIGGER rw8_at_commit AFTER INSERT ON test_a.rw8_q \
       DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION test_a.rw8_late();";

#[tokio::test]
async fn rw8_a_deferred_trigger_that_links_a_temp_child_at_commit_is_refused() {
    within(120, async {
        let (db, mut owner) = ready("rw8_trigger_link").await;
        let up: &'static str = Box::leak(
            format!("{DEFERRED_LINK} INSERT INTO test_a.rw8_q VALUES (1);").into_boxed_str(),
        );
        left_temp_link(apply(&mut owner, &one("rw8_late", up, "SELECT 1;")).await);
        assert_eq!(ledger(&db, "rw8_late").await, 0, "not recorded");
        assert_eq!(scalar(&db, LINKS).await, "0", "no link after the refusal");
        assert_eq!(
            scalar(
                &db,
                "select coalesce(to_regclass('test_a.rw8_base')::text, 'absent')"
            )
            .await,
            "absent",
            "the whole step rolled back"
        );
        next_runs(&db, &mut owner).await;
    })
    .await
}

/// The same under `revert_last`: the `down` queues the deferred work; refused, and the ledger
/// row it would remove stays.
#[tokio::test]
async fn rw8_a_revert_whose_deferred_trigger_links_a_temp_child_is_refused() {
    within(120, async {
        let (db, mut owner) = ready("rw8_revert_link").await;
        let set = one(
            "rw8_late",
            DEFERRED_LINK,
            "INSERT INTO test_a.rw8_q VALUES (1);",
        );
        apply(&mut owner, &set)
            .await
            .expect("the up step queues nothing");
        assert_eq!(ledger(&db, "rw8_late").await, 1, "premise: applied");
        left_temp_link(revert_last(&mut owner, &set).await);
        assert_eq!(
            ledger(&db, "rw8_late").await,
            1,
            "rolled back: still recorded"
        );
        assert_eq!(scalar(&db, LINKS).await, "0", "no link after the refusal");
        next_runs(&db, &mut owner).await;
    })
    .await
}

#[tokio::test]
async fn rw8_a_deferred_trigger_that_creates_an_outside_table_at_commit_is_refused() {
    within(120, async {
        let (db, mut owner) = ready("rw8_trigger_outside").await;
        let up = "CREATE FUNCTION test_a.rw8_late() RETURNS trigger LANGUAGE plpgsql AS $f$ BEGIN \
               CREATE TABLE test_b.rw8_outside (k int); RETURN NULL; END $f$; \
             CREATE TABLE test_a.rw8_q (k int); \
             CREATE CONSTRAINT TRIGGER rw8_at_commit AFTER INSERT ON test_a.rw8_q \
               DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION test_a.rw8_late(); \
             INSERT INTO test_a.rw8_q VALUES (1);";
        let err = apply(&mut owner, &one("rw8_late", up, "SELECT 1;"))
            .await
            .expect_err("a step whose deferred work writes outside its schema is refused");
        assert!(
            matches!(&err, MigrateError::OutsideSchema { version: 1, object } if object.contains("test_b.rw8_outside")),
            "the row check's refusal, after the deferred work ran: {err:?}"
        );
        assert_eq!(ledger(&db, "rw8_late").await, 0, "not recorded");
        assert_eq!(
            scalar(&db, "select coalesce(to_regclass('test_b.rw8_outside')::text, 'absent')").await,
            "absent",
            "rolled back: no outside table"
        );
        next_runs(&db, &mut owner).await;
    })
    .await
}

/// A holdable cursor whose remaining rows, read when COMMIT freezes it, would link a temporary
/// child: refused before COMMIT, naming the cursor.
#[tokio::test]
async fn rw8_a_holdable_cursor_is_refused() {
    within(120, async {
        let (db, mut owner) = ready("rw8_cursor").await;
        let up = "CREATE TABLE test_a.rw8_base (k int); \
             CREATE FUNCTION test_a.rw8_late() RETURNS int LANGUAGE plpgsql AS $f$ BEGIN \
               CREATE TEMP TABLE rw8_tmp () INHERITS (test_a.rw8_base); RETURN 1; END $f$; \
             DECLARE rw8_cur CURSOR WITH HOLD FOR SELECT test_a.rw8_late();";
        let err = apply(&mut owner, &one("rw8_late", up, "SELECT 1;"))
            .await
            .expect_err("a step that leaves a holdable cursor is refused");
        assert_eq!(
            format!("{err:?}"),
            "StepLeftHoldableCursor { version: 1, cursor: \"rw8_cur\" }"
        );
        assert_eq!(ledger(&db, "rw8_late").await, 0, "not recorded");
        assert_eq!(scalar(&db, LINKS).await, "0", "no link");
        next_runs(&db, &mut owner).await;
    })
    .await
}

/// Control: a harmless deferred trigger (a row in an own table) runs, and the step passes and is
/// recorded.
#[tokio::test]
async fn rw8_control_a_harmless_deferred_trigger_runs_and_the_step_passes() {
    within(120, async {
        let (db, mut owner) = ready("rw8_harmless").await;
        let up = "CREATE TABLE test_a.rw8_log (v text); \
             CREATE FUNCTION test_a.rw8_note() RETURNS trigger LANGUAGE plpgsql AS $f$ BEGIN \
               INSERT INTO test_a.rw8_log VALUES ('ran'); RETURN NULL; END $f$; \
             CREATE TABLE test_a.rw8_q (k int); \
             CREATE CONSTRAINT TRIGGER rw8_at_commit AFTER INSERT ON test_a.rw8_q \
               DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION test_a.rw8_note(); \
             INSERT INTO test_a.rw8_q VALUES (1);";
        apply(&mut owner, &one("rw8_harmless", up, "SELECT 1;"))
            .await
            .expect("a harmless deferred trigger");
        assert_eq!(ledger(&db, "rw8_harmless").await, 1, "recorded");
        assert_eq!(
            scalar(&db, "select string_agg(v, ',') from test_a.rw8_log").await,
            "ran",
            "the deferred trigger ran"
        );
    })
    .await
}
