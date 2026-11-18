//! Wave 0 freeze bar, P-03 (cox's row d5b02965): `empty_db()` gives a database of its own with
//! the roles and NO precreated schema, so a piece's migrations are proved on a truly empty
//! database. Control: `isolated_db()` keeps F-1b's fixture schemas.
//!
//! Needs `supervise.sh`'s throwaway Postgres.
//!
//! RED: `empty_db` is `todo!()`.
use cortex_testkit::{empty_db, isolated_db};

/// Every schema a user made: everything but the system ones and the default `public`.
const USER_SCHEMAS: &str = "SELECT nspname::text FROM pg_namespace \
     WHERE nspname NOT LIKE 'pg\\_%' AND nspname NOT IN ('information_schema', 'public') \
     ORDER BY nspname";

/// Every relation outside the system schemas.
const USER_RELATIONS: &str = "SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
     WHERE n.nspname NOT LIKE 'pg\\_%' AND n.nspname <> 'information_schema'";

#[tokio::test]
async fn w0_empty_db_has_no_schema_and_no_relation() {
    let db = empty_db("w0_empty").await;
    let mut admin = db.admin().await;
    let schemas: Vec<String> = sqlx::query_scalar(USER_SCHEMAS)
        .fetch_all(&mut admin)
        .await
        .expect("schemas");
    assert_eq!(schemas, Vec::<String>::new(), "no precreated schema");
    let relations: i64 = sqlx::query_scalar(USER_RELATIONS)
        .fetch_one(&mut admin)
        .await
        .expect("relations");
    assert_eq!(relations, 0, "no table, view or sequence");
    // The roles are there, and the owner can migrate into it.
    let mut owner = db.owner().await;
    sqlx::raw_sql("CREATE SCHEMA w0_probe; DROP SCHEMA w0_probe;")
        .execute(&mut owner)
        .await
        .expect("cortex_owner owns the database");
}

#[tokio::test]
async fn w0_isolated_db_keeps_the_fixture_schemas() {
    let db = isolated_db("w0_fixture").await;
    let mut admin = db.admin().await;
    let schemas: Vec<String> = sqlx::query_scalar(USER_SCHEMAS)
        .fetch_all(&mut admin)
        .await
        .expect("schemas");
    assert_eq!(schemas, ["test_a", "test_audit", "test_b"]);
}
