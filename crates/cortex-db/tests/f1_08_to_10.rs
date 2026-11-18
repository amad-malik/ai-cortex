//! F1-08 A failed delegated write rolls back the owner. [BASE]
//! F1-09 A module cannot read another's tables. [BASE]
//! F1-10 Row security fails closed. [RULED(L-7)] (TESTS r7 §2; S1 §9, A8, A11; S2 §9 D10.)
mod common;

use common::{begin, pool, project};
use cortex_db::DbError;
use cortex_testkit::{module, sqlstate, within};
use sqlx::Row;

async fn owner_insert_then_failing_audit(
    tx: &mut cortex_db::ModuleTx,
    p: cortex_types::ProjectId,
) -> Result<(), DbError> {
    sqlx::query("insert into test_a.t (project_id, v) values ($1, 'f1-08 owner')")
        .bind(common::uuid_of(p))
        .execute(tx.executor()?)
        .await
        .map_err(|e| DbError::Other(e.to_string()))?;
    tx.delegate(&module("test_audit"), async |t| {
        sqlx::query("insert into test_audit.r (project_id, v) values ($1, 'boom')")
            .bind(common::uuid_of(p))
            .execute(t.executor()?)
            .await
            .map(|_| ())
            .map_err(|e| DbError::Constraint {
                name: sqlstate(&e).unwrap_or_default(),
            })
    })
    .await
}

#[tokio::test]
async fn f1_08_a_failed_delegated_write_leaves_no_row_from_either() {
    within(60, async {
        let (db, pool) = pool(1).await;
        let p = project();
        let mut tx = begin(&pool, "test_a", Some(p)).await;
        assert!(
            owner_insert_then_failing_audit(&mut tx, p).await.is_err(),
            "the injected failure did not fail"
        );
        assert_eq!(tx.commit().await, Err(DbError::DelegationFailed));
        assert_eq!(
            common::count_as_admin(db, "test_a.t", p).await,
            0,
            "the owner's row survived"
        );
        assert_eq!(
            common::count_as_admin(db, "test_audit.r", p).await,
            0,
            "the audit row survived"
        );
    })
    .await
}

/// F1-08's control: the owner's write committed before the delegation survives it, which is what
/// the assertion above would see if the two were not one transaction.
#[tokio::test]
async fn f1_08_control_a_separately_committed_owner_row_survives() {
    within(60, async {
        let (db, pool) = pool(1).await;
        let p = project();
        let mut tx = begin(&pool, "test_a", Some(p)).await;
        sqlx::query("insert into test_a.t (project_id, v) values ($1, 'f1-08 control')")
            .bind(common::uuid_of(p))
            .execute(tx.executor().expect("executor"))
            .await
            .expect("owner insert");
        tx.commit().await.expect("commit first");
        let mut tx = begin(&pool, "test_a", Some(p)).await;
        let r = tx
            .delegate(&module("test_audit"), async |t| {
                sqlx::query("insert into test_audit.r (project_id, v) values ($1, 'boom')")
                    .bind(common::uuid_of(p))
                    .execute(t.executor()?)
                    .await
                    .map(|_| ())
                    .map_err(|e| DbError::Other(e.to_string()))
            })
            .await;
        assert!(r.is_err());
        tx.rollback().await.ok();
        assert_eq!(
            common::count_as_admin(db, "test_a.t", p).await,
            1,
            "the control's committed owner row must survive"
        );
    })
    .await
}

#[tokio::test]
async fn f1_09_a_module_cannot_read_another_modules_tables() {
    within(60, async {
        let (_db, pool) = pool(1).await;
        let mut tx = begin(&pool, "test_a", Some(project())).await;
        let e = sqlx::query("select count(*) from test_b.t")
            .fetch_one(tx.executor().expect("executor"))
            .await
            .expect_err("cross-module read");
        assert_eq!(sqlstate(&e).as_deref(), Some("42501"));
        tx.rollback().await.ok();
    })
    .await
}

async fn select_in(tx: &mut cortex_db::ModuleTx, table: &str) -> Result<i64, sqlx::Error> {
    let q = format!("select count(*) from {table}");
    sqlx::query(sqlx::AssertSqlSafe(q))
        .fetch_one(tx.executor().expect("executor"))
        .await
        .map(|r| r.get(0))
}

async fn setting(tx: &mut cortex_db::ModuleTx) -> Option<String> {
    sqlx::query("select current_setting('cortex.project_id', true)")
        .fetch_one(tx.executor().expect("executor"))
        .await
        .expect("setting")
        .get(0)
}

/// F1-10: foreign rows hidden; both empty states succeed with zero rows (never set, and set then
/// reverted); the ruled `nullif` form. Controls: the bare cast errors once reverted; the
/// no-missing-ok form errors when never set. Each state's raw setting is recorded.
#[tokio::test]
async fn f1_10_row_security_fails_closed_in_both_empty_states() {
    within(60, async {
        let (db, pool) = pool(1).await;
        let (a, b) = (project(), project());
        let mut admin = db.admin().await;
        for (p, table) in [
            (a, "test_a.t"),
            (b, "test_a.t"),
            (a, "test_a.t_bare_cast"),
            (a, "test_a.t_strict"),
        ] {
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "insert into {table} (project_id, v) values ($1, 'f1-10')"
            )))
            .bind(common::uuid_of(p))
            .execute(&mut admin)
            .await
            .expect("seed");
        }
        // Never set on this connection (the pool's one session, fresh).
        let mut tx = begin(&pool, "test_a", None).await;
        let never = setting(&mut tx).await;
        assert_eq!(
            select_in(&mut tx, "test_a.t")
                .await
                .expect("never-set state must succeed"),
            0
        );
        let strict_never = select_in(&mut tx, "test_a.t_strict").await;
        tx.rollback().await.ok();
        // Set for A, then reverted.
        let mut tx = begin(&pool, "test_a", Some(a)).await;
        let n_a = select_in(&mut tx, "test_a.t").await.expect("A");
        tx.commit().await.expect("commit");
        let mut tx = begin(&pool, "test_a", None).await;
        let reverted = setting(&mut tx).await;
        assert_eq!(
            select_in(&mut tx, "test_a.t")
                .await
                .expect("reverted state must succeed"),
            0
        );
        let bare_reverted = select_in(&mut tx, "test_a.t_bare_cast").await;
        tx.rollback().await.ok();

        println!("F1-10 settings: never-set {never:?}, reverted {reverted:?}");
        assert_eq!(n_a, 1, "project A sees only its own row, none of B's");
        assert!(
            bare_reverted.is_err(),
            "control: the bare cast must error in the reverted state"
        );
        assert!(
            strict_never.is_err(),
            "control: the setting without missing_ok must error when never set"
        );
    })
    .await
}
