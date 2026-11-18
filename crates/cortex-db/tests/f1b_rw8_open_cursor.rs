//! F-1b rework 8, part 2 (kai's row 55963949; F1B-RW6-DEFERRED-FINALIZATION), the enumeration's
//! last portal case. COMMIT drops every non-holdable cursor the transaction left open, and
//! PostgreSQL 18.4 says that drop "potentially runs user-defined code" (`PortalDrop`, shutting down
//! a started executor). The guard refuses such a cursor too, naming it, rather than argue it
//! benign. The new error is matched by its `Debug` form, so this tests-only case compiles before
//! the variant exists.
use cortex_db::OwnerConn;
use cortex_db::migrate::{Migration, MigrationSet, apply, platform_migrations};
use cortex_testkit::{isolated_db, module, within};

#[tokio::test]
async fn rw8_an_open_non_holdable_cursor_is_refused() {
    within(120, async {
        let db = isolated_db("rw8_open_cursor").await;
        let mut owner = OwnerConn::connect(&db.owner_config()).await.expect("owner");
        apply(&mut owner, &platform_migrations())
            .await
            .expect("platform migrations");
        let set = MigrationSet {
            module: module("rw8_open"),
            schema: "test_a",
            steps: vec![Migration {
                version: 1,
                name: "open_cursor",
                up: "CREATE TABLE test_a.rw8_t (k int); \
                     DECLARE rw8_plain CURSOR FOR SELECT k FROM test_a.rw8_t;",
                down: "DROP TABLE test_a.rw8_t;",
            }],
        };
        let err = apply(&mut owner, &set)
            .await
            .expect_err("a step that leaves a cursor open is refused");
        assert_eq!(
            format!("{err:?}"),
            "StepLeftOpenCursor { version: 1, cursor: \"rw8_plain\" }"
        );
        let mut admin = db.admin().await;
        let recorded: i64 = sqlx::query_scalar(
            "select count(*) from platform.migrations where module = 'rw8_open'",
        )
        .fetch_one(&mut admin)
        .await
        .expect("ledger");
        assert_eq!(recorded, 0, "not recorded");
        let table: String =
            sqlx::query_scalar("select coalesce(to_regclass('test_a.rw8_t')::text, 'absent')")
                .fetch_one(&mut admin)
                .await
                .expect("probe");
        assert_eq!(table, "absent", "rolled back");
        // Control, same session: the same step without the cursor runs.
        let ok = MigrationSet {
            module: module("rw8_open"),
            schema: "test_a",
            steps: vec![Migration {
                version: 1,
                name: "no_cursor",
                up: "CREATE TABLE test_a.rw8_t (k int);",
                down: "DROP TABLE test_a.rw8_t;",
            }],
        };
        apply(&mut owner, &ok)
            .await
            .expect("the same step without a cursor");
    })
    .await
}
