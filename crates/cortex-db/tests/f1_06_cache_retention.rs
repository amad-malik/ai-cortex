//! F1-06, cache retention witnessed (vera's Z2-ROLE-N02; TESTS r7). One held session runs the RLS
//! count for project A (3 rows), then B (2 rows), its identity asserted across the change, and
//! `pg_prepared_statements` shows the same statement before and after. Control: with sqlx's
//! statement cache off, the counts are the same and no statement is retained, so that run is
//! never counted as cache evidence. r2 (mike's F1B-R0-04): "the same statement" is its IDENTITY,
//! the prepared name plus `prepare_time`, not its text: deallocating and preparing the same text
//! again would match on text. A guard shows the identity changes on such a re-preparation.
mod common;

use cortex_db::DbPool;
use cortex_testkit::{same_session, session_identity, test_db, within};
use sqlx::Row;

const COUNT: &str = "select count(*) from test_a.t";

/// The identities (name and prepare time) of the session's prepared statements for `text`.
async fn retained_as(conn: &mut sqlx::PgConnection, text: &str) -> Vec<(String, String)> {
    sqlx::query_as(
        "select name, prepare_time::text from pg_prepared_statements where statement = $1 order by 1",
    )
    .bind(text)
    .fetch_all(conn)
    .await
    .expect("witness")
}

async fn retained(conn: &mut sqlx::PgConnection) -> Vec<(String, String)> {
    retained_as(conn, COUNT).await
}

type Run = (i64, i64, Vec<(String, String)>, Vec<(String, String)>);

async fn run(cache: bool) -> Run {
    let db = test_db().await;
    let mut cfg = db.db_config(common::MODULES, common::clock());
    cfg.max_connections = 1;
    cfg.statement_cache = cache;
    let pool = DbPool::connect(&cfg).await.expect("connect");
    let (a, b) = (common::project(), common::project());
    let mut admin = db.admin().await;
    for (p, n) in [(a, 3), (b, 2)] {
        for _ in 0..n {
            sqlx::query("insert into test_a.t (project_id, v) values ($1, 'cache')")
                .bind(common::uuid_of(p))
                .execute(&mut admin)
                .await
                .expect("seed");
        }
    }
    let mut tx = common::begin(&pool, "test_a", Some(a)).await;
    let s_a = session_identity(tx.executor().expect("executor")).await;
    let n_a: i64 = sqlx::query(COUNT)
        .fetch_one(tx.executor().expect("executor"))
        .await
        .expect("count A")
        .get(0);
    let w_a = retained(tx.executor().expect("executor")).await;
    tx.commit().await.expect("commit");
    let mut tx = common::begin(&pool, "test_a", Some(b)).await;
    let s_b = session_identity(tx.executor().expect("executor")).await;
    same_session(&s_a, &s_b).expect("one held session across the project change");
    let n_b: i64 = sqlx::query(COUNT)
        .fetch_one(tx.executor().expect("executor"))
        .await
        .expect("count B")
        .get(0);
    let w_b = retained(tx.executor().expect("executor")).await;
    tx.commit().await.expect("commit");
    (n_a, n_b, w_a, w_b)
}

#[tokio::test]
async fn f1_06_a_cached_statement_follows_the_project_on_one_session() {
    within(60, async {
        let (n_a, n_b, w_a, w_b) = run(true).await;
        assert_eq!(
            (n_a, n_b),
            (3, 2),
            "the cached count did not follow the project"
        );
        assert_eq!(w_a.len(), 1, "not exactly one retained statement: {w_a:?}");
        assert_eq!(
            w_a, w_b,
            "the retained statement's identity changed across the project switch: not the same \
             cached statement"
        );
    })
    .await
}

#[tokio::test]
async fn f1_06_control_without_the_cache_nothing_is_retained() {
    within(60, async {
        let (n_a, n_b, w_a, w_b) = run(false).await;
        assert_eq!((n_a, n_b), (3, 2));
        assert!(
            w_a.is_empty() && w_b.is_empty(),
            "a statement was retained with the cache off: the witness does not discriminate"
        );
    })
    .await
}

/// A SQL-level PREPARE shows its whole `PREPARE … AS …` text, so the guard matches by name.
async fn by_name(conn: &mut sqlx::PgConnection, name: &str) -> Vec<(String, String)> {
    sqlx::query_as("select name, prepare_time::text from pg_prepared_statements where name = $1")
        .bind(name)
        .fetch_all(conn)
        .await
        .expect("witness")
}

/// The witness's own guard: deallocating and preparing the SAME text under the SAME name gives a
/// new identity (prepare_time), so a re-preparation can never pass for a retained statement.
/// Raw SQL on one superuser session; it does not need cortex-db.
#[tokio::test]
async fn f1_06_guard_a_reprepared_statement_has_a_new_identity() {
    within(60, async {
        let db = test_db().await;
        let mut admin = db.admin().await;
        let text = "select 4242";
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "PREPARE f1_06_guard AS {text}"
        )))
        .execute(&mut admin)
        .await
        .expect("prepare");
        let first = by_name(&mut admin, "f1_06_guard").await;
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "SELECT pg_sleep(0.01); DEALLOCATE f1_06_guard; PREPARE f1_06_guard AS {text}"
        )))
        .execute(&mut admin)
        .await
        .expect("reprepare");
        let second = by_name(&mut admin, "f1_06_guard").await;
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1);
        assert_eq!(first[0].0, second[0].0, "the name is the same");
        assert_ne!(
            first, second,
            "a re-prepared statement kept its identity: the witness cannot tell"
        );
    })
    .await
}
