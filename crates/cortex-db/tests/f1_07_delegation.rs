//! F1-07 Delegation comes back. [BASE] (TESTS r7 §2; S1 §2.4; Z-2 RESULT, Z2-4.)
mod common;

use common::{begin, pool, project, who};
use cortex_db::DbError;
use cortex_testkit::{module, test_db, within};

#[tokio::test]
async fn f1_07_delegate_runs_as_the_target_and_comes_back() {
    within(60, async {
        let (_db, pool) = pool(1).await;
        let p = project();
        let mut tx = begin(&pool, "test_a", Some(p)).await;
        let inside = tx
            .delegate(
                &module("test_audit"),
                async |t| Ok(who(t.executor()?).await),
            )
            .await
            .expect("delegate");
        assert_eq!(
            inside,
            ("cortex_m_test_audit".to_owned(), Some(p.to_string())),
            "delegated role and kept project"
        );
        assert_eq!(
            who(tx.executor().expect("executor")).await,
            ("cortex_m_test_a".to_owned(), Some(p.to_string()))
        );
        tx.rollback().await.expect("rollback");
    })
    .await
}

#[tokio::test]
async fn f1_07_a_failed_delegation_comes_back_and_poisons_the_transaction() {
    within(60, async {
        let (db, pool) = pool(1).await;
        let p = project();
        let mut tx = begin(&pool, "test_a", Some(p)).await;
        sqlx::query("insert into test_a.t (project_id, v) values ($1, 'f1-07')")
            .bind(common::uuid_of(p))
            .execute(tx.executor().expect("executor"))
            .await
            .expect("owner insert");
        let r: Result<(), DbError> = tx
            .delegate(&module("test_audit"), async |_t| {
                Err(DbError::Other("injected".into()))
            })
            .await;
        assert!(r.is_err(), "the failed closure's error is returned");
        assert_eq!(
            who(tx.executor().expect("executor")).await.0,
            "cortex_m_test_a",
            "the role came back after the failure"
        );
        assert_eq!(
            tx.commit().await,
            Err(DbError::DelegationFailed),
            "commit after a failed delegation must refuse"
        );
        assert_eq!(
            common::count_as_admin(db, "test_a.t", p).await,
            0,
            "the transaction was not rolled back"
        );
    })
    .await
}

/// Z2-4 carried into F-1b: a statement prepared and run inside `delegate(test_audit, …)`, run
/// again by the owner in the same transaction, is checked under the owner's role (42501).
#[tokio::test]
async fn f1_07_a_cached_statement_is_checked_under_the_current_role() {
    within(60, async {
        const READ: &str = "select count(*) from test_audit.r where project_id = $1";
        let (_db, pool) = pool(1).await;
        let p = project();
        let mut tx = begin(&pool, "test_a", Some(p)).await;
        tx.delegate(&module("test_audit"), async |t| {
            sqlx::query(READ)
                .bind(common::uuid_of(p))
                .fetch_one(t.executor()?)
                .await
                .map_err(|e| DbError::Other(e.to_string()))?;
            Ok(())
        })
        .await
        .expect("prepared and run under cortex_m_test_audit");
        let e = sqlx::query(READ)
            .bind(common::uuid_of(p))
            .fetch_one(tx.executor().expect("executor"))
            .await
            .expect_err("the owner must not read test_audit.r");
        assert_eq!(cortex_testkit::sqlstate(&e).as_deref(), Some("42501"));
        tx.rollback().await.ok();
    })
    .await
}

/// Its positive control: one role holding both grants runs the same read twice.
#[tokio::test]
async fn f1_07_control_one_role_with_both_grants_reads_twice() {
    within(60, async {
        let db = test_db().await;
        let mut base = db.base().await;
        for _ in 0..2 {
            sqlx::query("select count(*) from test_audit.r where project_id = $1")
                .bind(common::uuid_of(project()))
                .fetch_one(&mut base)
                .await
                .expect("the both-grants role reads test_audit.r");
        }
    })
    .await
}
