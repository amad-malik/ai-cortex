//! F-1b rework 1 (kai's row 4598a4ed; vera's R2 of a1d1171, REVIEW.md 2e954d0e): three
//! `ModuleTx` findings, each RED on a1d1171.
//! - **F1B-GREEN-DELEGATE-CANCEL:** a `delegate` future dropped at the role switch, at the
//!   callback's await, or at the restore, while the caller keeps its `ModuleTx`. Afterwards
//!   `executor`, `delegate` and `advisory_lock` refuse with `DelegationFailed`, `commit` refuses
//!   with `DelegationFailed`, and nothing the transaction wrote persists.
//! - **F1B-GREEN-DELEGATE-PREFLIGHT:** an owner write, then a delegation to an unregistered
//!   module: `UnknownModule`, and `commit` refuses with `DelegationFailed`; the row is gone.
//! - **F1B-GREEN-OWNER-AUDIT-SEAL:** `AuditChain` taken directly in the owner's scope is refused
//!   before any SQL (`LockOrder { held: OwnerRows, wanted: AuditChain }`) and poisons; and when the
//!   owner's module is also the audit module, the owner's executor after the audit delegation
//!   is refused, because the audit scope is the delegation, not the module id.
mod common;

use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::task::Poll;

use common::{begin, count_as_admin, pool, project, uuid_of};
use cortex_db::{AdvisoryClass, DbError, DbPool, LockStep, ModuleTx};
use cortex_testkit::{TestDb, module, within};
use cortex_types::ProjectId;
use sqlx::Row;

/// Where a delegation is dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum At {
    /// While the switch to the target role is awaited, before the callback starts.
    RoleSwitch,
    /// While the callback awaits, after it has written a row as the target.
    Callback,
    /// While the restore of the owner's role is awaited, after the callback returned.
    Restore,
}

impl At {
    /// The callback's stage at this boundary: 0 not started, 1 awaiting inside, 2 returned.
    fn stage(self) -> u8 {
        match self {
            At::RoleSwitch => 0,
            At::Callback => 1,
            At::Restore => 2,
        }
    }
}

/// Polls `delegate(test_audit, …)` and drops it at the first `Pending` seen at `at`, keeping the
/// `ModuleTx`. True when it was dropped there; false when the boundary was passed without a
/// `Pending` (the caller then tries again in a fresh transaction).
async fn drop_delegation_at(tx: &mut ModuleTx, p: ProjectId, at: At) -> bool {
    let stage = Arc::new(AtomicU8::new(0));
    let inside = stage.clone();
    let project = uuid_of(p);
    let target = module("test_audit");
    let delegation = tx.delegate(&target, async move |t| {
        if at == At::Callback {
            sqlx::query("insert into test_audit.r (project_id, v) values ($1, 'rw1-cancel')")
                .bind(project)
                .execute(t.executor()?)
                .await
                .map_err(|e| DbError::Other(e.to_string()))?;
        }
        if at == At::Restore {
            inside.store(2, Ordering::SeqCst);
            return Ok(());
        }
        inside.store(1, Ordering::SeqCst);
        std::future::pending::<()>().await;
        Ok(())
    });
    let mut delegation = pin!(delegation);
    std::future::poll_fn(|cx| match delegation.as_mut().poll(cx) {
        Poll::Ready(_) => Poll::Ready(false),
        Poll::Pending => {
            let now = stage.load(Ordering::SeqCst);
            if now == at.stage() {
                Poll::Ready(true)
            } else if now > at.stage() {
                Poll::Ready(false)
            } else {
                Poll::Pending
            }
        }
    })
    .await
}

/// An owner row in `test_a.t` for `p`.
async fn owner_write(tx: &mut ModuleTx, p: ProjectId, v: &str) {
    sqlx::query("insert into test_a.t (project_id, v) values ($1, $2)")
        .bind(uuid_of(p))
        .bind(v)
        .execute(tx.executor().expect("executor before any delegation"))
        .await
        .expect("owner insert");
}

/// Begins as `test_a`, writes an owner row, and drops a delegation at `at`. Retries in a fresh
/// transaction (up to 20 times) when the boundary is passed without a `Pending`.
async fn abandoned_at(db: &TestDb, pool: &DbPool, at: At) -> (ModuleTx, ProjectId) {
    for _ in 0..20 {
        let p = project();
        let mut tx = begin(pool, "test_a", Some(p)).await;
        owner_write(&mut tx, p, "rw1-owner").await;
        if drop_delegation_at(&mut tx, p, at).await {
            return (tx, p);
        }
        tx.rollback().await.ok();
        assert_eq!(count_as_admin(db, "test_a.t", p).await, 0);
    }
    panic!("the {at:?} boundary was never reached in 20 attempts");
}

async fn everything_refuses_after(at: At) {
    let (db, pool) = pool(1).await;
    let (mut tx, p) = abandoned_at(db, &pool, at).await;
    assert_eq!(
        tx.executor().err(),
        Some(DbError::DelegationFailed),
        "{at:?}: the owner's executor after an abandoned delegation"
    );
    assert_eq!(
        tx.delegate(&module("test_audit"), async |_t| Ok(())).await,
        Err(DbError::DelegationFailed),
        "{at:?}: a new delegation after an abandoned one"
    );
    assert_eq!(
        tx.advisory_lock(AdvisoryClass::Idempotency, 1).await,
        Err(DbError::DelegationFailed),
        "{at:?}: an advisory lock after an abandoned delegation"
    );
    assert_eq!(
        tx.commit().await,
        Err(DbError::DelegationFailed),
        "{at:?}: commit after an abandoned delegation"
    );
    assert_eq!(
        count_as_admin(db, "test_a.t", p).await,
        0,
        "{at:?}: the owner's row persisted"
    );
    assert_eq!(
        count_as_admin(db, "test_audit.r", p).await,
        0,
        "{at:?}: the delegated row persisted"
    );
}

#[tokio::test]
async fn rw1_cancel_at_the_role_switch_fails_closed() {
    within(120, everything_refuses_after(At::RoleSwitch)).await
}

#[tokio::test]
async fn rw1_cancel_at_the_callback_await_fails_closed() {
    within(120, everything_refuses_after(At::Callback)).await
}

#[tokio::test]
async fn rw1_cancel_at_the_restore_fails_closed() {
    within(120, everything_refuses_after(At::Restore)).await
}

/// Control: a delegation that runs to the end leaves the transaction usable and committable.
#[tokio::test]
async fn rw1_control_a_completed_delegation_commits() {
    within(60, async {
        let (db, pool) = pool(1).await;
        let p = project();
        let mut tx = begin(&pool, "test_a", Some(p)).await;
        owner_write(&mut tx, p, "rw1-control").await;
        let project = uuid_of(p);
        tx.delegate(&module("test_audit"), async move |t| {
            sqlx::query("insert into test_audit.r (project_id, v) values ($1, 'rw1-control')")
                .bind(project)
                .execute(t.executor()?)
                .await
                .map(|_| ())
                .map_err(|e| DbError::Other(e.to_string()))
        })
        .await
        .expect("delegate");
        tx.executor()
            .expect("the owner's executor after a completed delegation");
        tx.commit().await.expect("commit");
        assert_eq!(count_as_admin(db, "test_a.t", p).await, 1);
        assert_eq!(count_as_admin(db, "test_audit.r", p).await, 1);
    })
    .await
}

#[tokio::test]
async fn rw1_an_unknown_target_after_an_owner_write_poisons_the_transaction() {
    within(60, async {
        let (db, pool) = pool(1).await;
        let p = project();
        let mut tx = begin(&pool, "test_a", Some(p)).await;
        owner_write(&mut tx, p, "rw1-preflight").await;
        let mut ran = false;
        let r = tx
            .delegate(&module("nowhere"), async |_t| {
                ran = true;
                Ok(())
            })
            .await;
        assert_eq!(r, Err(DbError::UnknownModule));
        assert!(!ran, "the closure ran for an unregistered module");
        assert_eq!(
            tx.commit().await,
            Err(DbError::DelegationFailed),
            "commit after a refused delegation"
        );
        assert_eq!(
            count_as_admin(db, "test_a.t", p).await,
            0,
            "the owner's row persisted after a refused delegation"
        );
    })
    .await
}

async fn my_advisory_locks(tx: &mut ModuleTx, class: i32) -> i64 {
    sqlx::query("select count(*) from pg_locks where locktype = 'advisory' and pid = pg_backend_pid() and classid = $1::oid")
        .bind(class)
        .fetch_one(tx.executor().expect("executor"))
        .await
        .expect("pg_locks")
        .get(0)
}

#[tokio::test]
async fn rw1_audit_chain_in_the_owner_scope_is_refused_before_any_sql() {
    within(60, async {
        let (_db, pool) = pool(1).await;
        let mut tx = begin(&pool, "test_a", Some(project())).await;
        assert_eq!(
            tx.advisory_lock(AdvisoryClass::AuditChain, 42).await,
            Err(DbError::LockOrder {
                held: LockStep::OwnerRows,
                wanted: LockStep::AuditChain { chain: 42 },
            }),
            "AuditChain taken directly as the owner"
        );
        assert!(
            !tx.lock_ledger().sealed(),
            "a refused lock sealed the ledger"
        );
        assert_eq!(
            my_advisory_locks(&mut tx, AdvisoryClass::AuditChain as i32).await,
            0,
            "the refused lock reached the server"
        );
        assert_eq!(
            tx.commit().await,
            Err(DbError::DelegationFailed),
            "the refusal must poison"
        );
    })
    .await
}

#[tokio::test]
async fn rw1_the_audit_scope_is_the_delegation_not_the_module_id() {
    within(60, async {
        let (_db, pool) = pool(1).await;
        // The owner IS the audit module: after its audit delegation, the owner's own executor is
        // outside the audit scope although the module ids are equal.
        let mut tx = begin(&pool, "test_audit", Some(project())).await;
        tx.delegate(&module("test_audit"), async |t| {
            t.advisory_lock(AdvisoryClass::AuditChain, 43).await?;
            sqlx::query("insert into test_audit.r (v) values ('rw1-scope')")
                .execute(t.executor()?)
                .await
                .map(|_| ())
                .map_err(|e| DbError::Other(e.to_string()))
        })
        .await
        .expect("the append inside the audit delegation");
        assert!(tx.lock_ledger().sealed());
        assert_eq!(
            tx.executor().err(),
            Some(DbError::LockOrder {
                held: LockStep::AuditChain { chain: 43 },
                wanted: LockStep::OwnerRows,
            }),
            "the owner's executor after the seal"
        );
        assert_eq!(tx.commit().await, Err(DbError::DelegationFailed));

        // Control: the same-chain append again, inside the audit delegation, still passes.
        let mut tx = begin(&pool, "test_audit", Some(project())).await;
        for _ in 0..2 {
            tx.delegate(&module("test_audit"), async |t| {
                t.advisory_lock(AdvisoryClass::AuditChain, 44).await?;
                sqlx::query("insert into test_audit.r (v) values ('rw1-scope-again')")
                    .execute(t.executor()?)
                    .await
                    .map(|_| ())
                    .map_err(|e| DbError::Other(e.to_string()))
            })
            .await
            .expect("the same chain, inside the audit delegation");
        }
        tx.commit().await.expect("commit");
    })
    .await
}
