//! F-1b rework 6 (kai's row aaa73c39; vera's F1B-RW5-TEMP-INHERITANCE-TOPOLOGY on 8ef637d). A
//! guarded step may create a TEMP table that inherits an own permanent table: the guard counts
//! temporary schemas as inside, so the step passes and leaves a `pg_temp_N -> test_a` link, which
//! ruling T-1 excludes; only the NEXT step is refused, by the start check. The author reproduced it
//! on PostgreSQL 18.4 (runs-rw6). The topology rule is now a postcondition as well: after the
//! step's statements, before anything is recorded, a link across schemas refuses the step and
//! rolls it back, naming the link. Paired controls: a TEMP table with no inheritance, same-schema
//! inheritance, and a same-schema partitioned table with its DEFAULT partition are accepted.
//!
//! Each negative case has a database of its own (`isolated_db`). The new error is matched by its
//! `Debug` form, so these tests-only cases compile before the variant exists; the temporary
//! schema's number is the session's, so only its digits are free.
use cortex_db::OwnerConn;
use cortex_db::migrate::{
    MigrateError, Migration, MigrationSet, apply, platform_migrations, revert_last,
};
use cortex_testkit::{TestDb, isolated_db, module, test_db, within};

fn one(module_id: &str, name: &'static str, up: &'static str, down: &'static str) -> MigrationSet {
    MigrationSet {
        module: module(module_id),
        schema: "test_a",
        steps: vec![Migration {
            version: 1,
            name,
            up,
            down,
        }],
    }
}

async fn ready_on(db: &TestDb) -> OwnerConn {
    let mut owner = OwnerConn::connect(&db.owner_config()).await.expect("owner");
    apply(&mut owner, &platform_migrations())
        .await
        .expect("platform migrations");
    owner
}

async fn count(db: &TestDb, sql: &'static str, bind: &'static str) -> i64 {
    let mut admin = db.admin().await;
    sqlx::query_scalar(sql)
        .bind(bind)
        .fetch_one(&mut admin)
        .await
        .expect("probe")
}

async fn ledger(db: &TestDb, module_id: &'static str) -> i64 {
    count(
        db,
        "select count(*) from platform.migrations where module = $1",
        module_id,
    )
    .await
}

/// Relations of this name in any schema, temporary ones included, seen from another session.
async fn relations(db: &TestDb, name: &'static str) -> i64 {
    count(db, "select count(*) from pg_class where relname = $1", name).await
}

/// The step was refused AFTER it ran, naming the link it left: `pg_temp_<n>.<child>` under `parent`.
fn left_temp_link(result: Result<(), MigrateError>, child: &str, parent: &str) {
    let err = result.expect_err("a step that leaves a cross-schema link is refused");
    let debug = format!("{err:?}");
    let digits = debug
        .strip_prefix("StepLeftTopologyNotSchemaClosed { version: 1, link: \"pg_temp_")
        .and_then(|r| {
            r.strip_suffix(&format!(
                ".{child} is an inheritance child of {parent}\" }}"
            ))
        });
    assert!(
        digits.is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit())),
        "the postcondition's refusal, naming the temporary link: {debug}"
    );
}

/// vera's case under `apply`: refused, rolled back (no temporary table, no link, not recorded),
/// and the NEXT step on the same session runs.
#[tokio::test]
async fn rw6_a_step_leaving_a_temp_child_of_an_own_table_is_refused_and_rolled_back() {
    within(120, async {
        let db = isolated_db("rw6_temp_apply").await;
        let mut owner = ready_on(&db).await;
        let mut raw = db.owner().await;
        sqlx::raw_sql("CREATE TABLE test_a.rw6_base (k int);")
            .execute(&mut raw)
            .await
            .expect("an ordinary owner-owned test_a table");
        let temp = one(
            "rw6_temp",
            "temp_child_of_own_table",
            "CREATE TEMP TABLE rw6_tmp () INHERITS (test_a.rw6_base) ON COMMIT PRESERVE ROWS;",
            "DROP TABLE IF EXISTS pg_temp.rw6_tmp;",
        );
        left_temp_link(apply(&mut owner, &temp).await, "rw6_tmp", "test_a.rw6_base");
        assert_eq!(
            relations(&db, "rw6_tmp").await,
            0,
            "rolled back: no temporary table"
        );
        assert_eq!(
            count(
                &db,
                "select count(*) from pg_inherits where inhparent = $1::regclass",
                "test_a.rw6_base"
            )
            .await,
            0,
            "rolled back: no link"
        );
        assert_eq!(ledger(&db, "rw6_temp").await, 0, "not recorded");
        let next = one(
            "rw6_next",
            "an_unrelated_own_table",
            "CREATE TABLE test_a.rw6_next (k int);",
            "DROP TABLE test_a.rw6_next;",
        );
        apply(&mut owner, &next)
            .await
            .expect("the next step on the same session is not blocked");
        assert_eq!(
            ledger(&db, "rw6_next").await,
            1,
            "the next step is recorded"
        );
    })
    .await
}

/// The same under `revert_last`: a `down` that leaves the link is refused, and the ledger row it
/// would remove stays; the next step runs.
#[tokio::test]
async fn rw6_a_revert_leaving_a_temp_child_of_an_own_table_is_refused_and_rolled_back() {
    within(120, async {
        let db = isolated_db("rw6_temp_revert").await;
        let mut owner = ready_on(&db).await;
        let set = one(
            "rw6_revert",
            "own_table_whose_down_links_a_temp_child",
            "CREATE TABLE test_a.rw6_rv (k int);",
            "CREATE TEMP TABLE rw6_rtmp () INHERITS (test_a.rw6_rv) ON COMMIT PRESERVE ROWS;",
        );
        apply(&mut owner, &set).await.expect("the up step");
        assert_eq!(ledger(&db, "rw6_revert").await, 1, "premise: applied");
        left_temp_link(
            revert_last(&mut owner, &set).await,
            "rw6_rtmp",
            "test_a.rw6_rv",
        );
        assert_eq!(
            relations(&db, "rw6_rtmp").await,
            0,
            "rolled back: no temporary table"
        );
        assert_eq!(
            ledger(&db, "rw6_revert").await,
            1,
            "rolled back: still recorded"
        );
        let next = one(
            "rw6_next",
            "an_unrelated_own_table",
            "CREATE TABLE test_a.rw6_next (k int);",
            "DROP TABLE test_a.rw6_next;",
        );
        apply(&mut owner, &next)
            .await
            .expect("the next step on the same session is not blocked");
    })
    .await
}

/// Controls, accepted before and after the change: a TEMP table with no inheritance (and the next
/// step on that session), same-schema inheritance, and a same-schema partitioned table with a
/// partition and its DEFAULT partition.
#[tokio::test]
async fn rw6_temp_tables_and_same_schema_links_are_accepted() {
    within(120, async {
        let db = test_db().await;
        let mut owner = ready_on(db).await;
        let plain = one(
            "rw6_ctl_temp",
            "a_temp_table_with_no_inheritance",
            "CREATE TEMP TABLE rw6_plain (k int) ON COMMIT PRESERVE ROWS;",
            "DROP TABLE IF EXISTS pg_temp.rw6_plain;",
        );
        apply(&mut owner, &plain)
            .await
            .expect("a temporary table with no inheritance");
        let inherits = one(
            "rw6_ctl_inherits",
            "same_schema_inheritance",
            "CREATE TABLE test_a.rw6_ibase (k int); \
             CREATE TABLE test_a.rw6_ichild () INHERITS (test_a.rw6_ibase);",
            "DROP TABLE test_a.rw6_ichild; DROP TABLE test_a.rw6_ibase;",
        );
        apply(&mut owner, &inherits)
            .await
            .expect("same-schema inheritance, on the session holding a temporary table");
        let partitioned = one(
            "rw6_ctl_default",
            "same_schema_partition_and_default",
            "CREATE TABLE test_a.rw6_pp (k int) PARTITION BY LIST (k); \
             CREATE TABLE test_a.rw6_pp_1 PARTITION OF test_a.rw6_pp FOR VALUES IN (1); \
             CREATE TABLE test_a.rw6_pp_d PARTITION OF test_a.rw6_pp DEFAULT;",
            "DROP TABLE test_a.rw6_pp;",
        );
        apply(&mut owner, &partitioned)
            .await
            .expect("a same-schema partition and DEFAULT partition");
        for m in ["rw6_ctl_temp", "rw6_ctl_inherits", "rw6_ctl_default"] {
            assert_eq!(ledger(db, m).await, 1, "{m}: recorded");
        }
    })
    .await
}
