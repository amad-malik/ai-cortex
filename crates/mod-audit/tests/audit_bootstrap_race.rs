//! Deterministic overlapping role creation control for AUD-SAM-N02.
use std::time::Duration;

use cortex_testkit::empty_db;
use sqlx::Connection;

#[tokio::test]
async fn audit_bootstrap_tolerates_concurrent_role_creation() {
    let db = empty_db("audit_role_race").await;
    let mut creator = db.admin().await;
    let mut competing = db.admin().await;
    let mut observer = db.admin().await;
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut competing)
        .await
        .unwrap();
    let source = include_str!("audit_acl.rs");
    let (_, after) = source.split_once("sqlx::raw_sql(\"").expect("fixture SQL");
    let (bootstrap, _) = after.split_once("\")").expect("fixture SQL end");
    let bootstrap = bootstrap.replace("cortex_m_audit", "cortex_m_audit_race");

    let mut tx = creator.begin().await.unwrap();
    sqlx::query("CREATE ROLE cortex_m_audit_race NOLOGIN")
        .execute(&mut *tx)
        .await
        .expect("race-control role must not already exist");
    let task = tokio::spawn(async move {
        sqlx::raw_sql(sqlx::AssertSqlSafe(bootstrap))
            .execute(&mut competing)
            .await
            .map(|_| ())
    });
    let mut blocked = false;
    for _ in 0..250 {
        let waiting: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE pid = $1 AND wait_event_type = 'Lock')",
        )
        .bind(pid)
        .fetch_one(&mut observer)
        .await
        .unwrap();
        if waiting {
            blocked = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        blocked,
        "competing bootstrap did not reach the catalog lock"
    );
    tx.commit().await.unwrap();
    task.await
        .unwrap()
        .expect("concurrent creator must be tolerated");
}
