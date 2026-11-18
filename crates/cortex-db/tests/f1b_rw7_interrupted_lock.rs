//! F-1b rework 7, fix A (kai's row 690c50a0 and ruling T-2; sam's SAM-R2-02, combined by vera).
//! An audit delegate awaits only `advisory_lock(AuditChain, key)` under an ordinary timeout and
//! catches `Elapsed`. When the inner request was already sent and is waiting (a peer holds the
//! chain), dropping its future does not withdraw it: the outer restore drains it, the server
//! grants it once the peer releases, and the retained transaction holds `AuditChain` with an empty
//! ledger and no seal. The author reproduced that on PostgreSQL 18.4 (runs-rw7). An interrupted
//! acquisition now abandons the transaction, as a dropped delegation does: each entry, in a fresh
//! transaction, refuses with `DelegationFailed`, the owner's earlier row is rolled back, the lock is
//! released and the connection serves the next transaction. The boundary is confirmed each time:
//! the request is seen waiting in `pg_locks` before its future times out.
//!
//! Controls, passing before and after: a never-polled acquisition; a lock that fails in SQL (the
//! statement timeout); a callback that propagates the timeout; a healthy acquisition under a
//! timeout. The whole-delegate drops and the same-chain re-take stay rework 1's
//! (`f1b_rw1_delegate.rs`), unchanged.
mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use common::{begin, count_as_admin, pool, project, uuid_of};
use cortex_db::{AdvisoryClass, DbError, DbPool, LockStep, ModuleTx, TxOptions};
use cortex_testkit::{TestDb, ctx, module, within};
use cortex_types::ProjectId;
use sqlx::PgConnection;

const AUDIT: i32 = AdvisoryClass::AuditChain as i32;
const GATE: i32 = AdvisoryClass::ProducerGate as i32;

/// Advisory locks `(class, key)` on the server, `granted` or waiting, of any session (`pid` 0) or
/// of one.
async fn locks(db: &TestDb, pid: i32, class: i32, key: i32, granted: bool) -> i64 {
    let mut admin = db.admin().await;
    sqlx::query_scalar(
        "select count(*) from pg_locks where locktype = 'advisory' and ($1 = 0 or pid = $1) \
         and classid = $2::oid and objid = $3::oid and objsubid = 2 and granted = $4",
    )
    .bind(pid)
    .bind(class)
    .bind(key)
    .bind(granted)
    .fetch_one(&mut admin)
    .await
    .expect("pg_locks")
}

/// A peer session holding `(class, key)` in an open transaction.
async fn peer_holding(db: &TestDb, class: i32, key: i32) -> PgConnection {
    let mut peer = db.admin().await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "BEGIN; SELECT pg_advisory_xact_lock({class}, {key});"
    )))
    .execute(&mut peer)
    .await
    .expect("the peer takes the lock");
    peer
}

async fn owner_write(tx: &mut ModuleTx, p: ProjectId) -> i32 {
    sqlx::query("insert into test_a.t (project_id, v) values ($1, 'rw7a-owner')")
        .bind(uuid_of(p))
        .execute(tx.executor().expect("executor"))
        .await
        .expect("owner insert");
    sqlx::query_scalar("select pg_backend_pid()")
        .fetch_one(tx.executor().expect("executor"))
        .await
        .expect("pid")
}

/// The schedule, in a fresh transaction after an owner row: a peer holds `AuditChain(key)`; inside
/// an audit delegate (nested in a second one when `nested`), the request is sent and seen WAITING,
/// its future times out while pending, the callback returns `Ok`; the peer then releases, and the
/// outer restore drains the late grant. Returns the transaction, its project and backend pid.
async fn interrupted(
    db: &'static TestDb,
    pool: &DbPool,
    key: i32,
    nested: bool,
) -> (ModuleTx, ProjectId, i32) {
    let p = project();
    let mut tx = begin(pool, "test_a", Some(p)).await;
    let pid = owner_write(&mut tx, p).await;
    let mut peer = peer_holding(db, AUDIT, key).await;
    let (dropped, dropped_rx) = tokio::sync::oneshot::channel::<()>();
    let observer = tokio::spawn(async move {
        let mut seen = false;
        for _ in 0..500 {
            if locks(db, pid, AUDIT, key, false).await == 1 {
                seen = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        dropped_rx.await.ok();
        sqlx::raw_sql("COMMIT")
            .execute(&mut peer)
            .await
            .expect("the peer releases");
        seen
    });
    let elapsed = Arc::new(AtomicBool::new(false));
    let flag = elapsed.clone();
    let inner = async move |t: &mut ModuleTx| {
        let r = tokio::time::timeout(
            Duration::from_millis(400),
            t.advisory_lock(AdvisoryClass::AuditChain, key),
        )
        .await;
        flag.store(r.is_err(), Ordering::SeqCst);
        dropped.send(()).ok();
        Ok::<(), DbError>(())
    };
    let r = if nested {
        tx.delegate(&module("test_audit"), async move |t| {
            t.delegate(&module("test_audit"), inner).await
        })
        .await
    } else {
        tx.delegate(&module("test_audit"), inner).await
    };
    assert!(
        observer.await.expect("observer"),
        "boundary: the request was seen waiting"
    );
    assert!(
        elapsed.load(Ordering::SeqCst),
        "boundary: its future timed out while pending"
    );
    assert_eq!(
        r,
        Ok(()),
        "the callback caught the timeout, so the delegation returns Ok"
    );
    assert_eq!(
        locks(db, pid, AUDIT, key, true).await,
        1,
        "the server granted the interrupted request afterwards"
    );
    (tx, p, pid)
}

/// After the entry was refused: commit refuses and rolls back, the owner's row is gone, the chain
/// lock is released, and the pool's one connection serves the next transaction.
async fn closed_and_clean(
    db: &TestDb,
    pool: &DbPool,
    tx: Option<ModuleTx>,
    p: ProjectId,
    key: i32,
) {
    if let Some(tx) = tx {
        assert_eq!(
            tx.commit().await,
            Err(DbError::DelegationFailed),
            "commit afterwards"
        );
    }
    assert_eq!(
        count_as_admin(db, "test_a.t", p).await,
        0,
        "the owner's row rolled back"
    );
    assert_eq!(
        locks(db, 0, AUDIT, key, true).await,
        0,
        "the chain lock is released"
    );
    let mut next = begin(pool, "test_a", Some(project())).await;
    sqlx::query("select 1")
        .execute(next.executor().expect("executor"))
        .await
        .expect("the connection serves the next transaction");
    next.commit().await.expect("the next transaction commits");
}

#[tokio::test]
async fn rw7_interrupted_audit_lock_then_commit_first_fails_closed() {
    within(60, async {
        let (db, pool) = pool(1).await;
        let (tx, p, _) = interrupted(db, &pool, 7201, false).await;
        assert_eq!(
            tx.commit().await,
            Err(DbError::DelegationFailed),
            "commit first"
        );
        closed_and_clean(db, &pool, None, p, 7201).await;
    })
    .await
}

#[tokio::test]
async fn rw7_interrupted_audit_lock_then_executor_first_fails_closed() {
    within(60, async {
        let (db, pool) = pool(1).await;
        let (mut tx, p, _) = interrupted(db, &pool, 7202, false).await;
        assert_eq!(
            tx.executor().err(),
            Some(DbError::DelegationFailed),
            "executor first"
        );
        closed_and_clean(db, &pool, Some(tx), p, 7202).await;
    })
    .await
}

#[tokio::test]
async fn rw7_interrupted_audit_lock_then_a_lower_class_first_fails_closed() {
    within(60, async {
        let (db, pool) = pool(1).await;
        let (mut tx, p, pid) = interrupted(db, &pool, 7203, false).await;
        assert_eq!(
            tx.advisory_lock(AdvisoryClass::Idempotency, 7203).await,
            Err(DbError::DelegationFailed),
            "a lower class first"
        );
        assert_eq!(
            locks(db, pid, AdvisoryClass::Idempotency as i32, 7203, true).await,
            0,
            "the lower class never reached the server"
        );
        closed_and_clean(db, &pool, Some(tx), p, 7203).await;
    })
    .await
}

#[tokio::test]
async fn rw7_interrupted_audit_lock_then_another_delegate_first_fails_closed() {
    within(60, async {
        let (db, pool) = pool(1).await;
        let (mut tx, p, _) = interrupted(db, &pool, 7204, false).await;
        let mut ran = false;
        let r = tx
            .delegate(&module("test_b"), async |_t| {
                ran = true;
                Ok(())
            })
            .await;
        assert_eq!(r, Err(DbError::DelegationFailed), "another delegate first");
        assert!(!ran, "the other delegate's callback ran");
        closed_and_clean(db, &pool, Some(tx), p, 7204).await;
    })
    .await
}

/// Nested: the interrupted acquisition in an inner delegate stays abandoned after both return.
#[tokio::test]
async fn rw7_interrupted_audit_lock_in_a_nested_delegate_stays_abandoned() {
    within(60, async {
        let (db, pool) = pool(1).await;
        let (mut tx, p, _) = interrupted(db, &pool, 7205, true).await;
        assert_eq!(
            tx.executor().err(),
            Some(DbError::DelegationFailed),
            "executor first"
        );
        closed_and_clean(db, &pool, Some(tx), p, 7205).await;
    })
    .await
}

/// Control: an acquisition future dropped before it was ever polled sends nothing and abandons
/// nothing; the same lock taken normally afterwards seals and commits.
#[tokio::test]
async fn rw7_control_a_never_polled_acquisition_abandons_nothing() {
    within(60, async {
        let (db, pool) = pool(1).await;
        let p = project();
        let mut tx = begin(&pool, "test_a", Some(p)).await;
        let pid = owner_write(&mut tx, p).await;
        tx.delegate(&module("test_audit"), async |t| {
            let never = t.advisory_lock(AdvisoryClass::AuditChain, 7206);
            drop(never);
            t.advisory_lock(AdvisoryClass::AuditChain, 7206).await
        })
        .await
        .expect("the delegation");
        assert_eq!(locks(db, pid, AUDIT, 7206, true).await, 1);
        assert_eq!(
            tx.lock_ledger().steps(),
            [LockStep::AuditChain { chain: 7206 }]
        );
        assert!(tx.lock_ledger().sealed());
        tx.commit().await.expect("commit");
        assert_eq!(count_as_admin(db, "test_a.t", p).await, 1);
    })
    .await
}

/// Control: a lock that fails in SQL (the statement timeout, while a peer holds it) returns that
/// error and poisons, but abandons nothing: the executor is still handed out, and commit refuses
/// because of the poison.
#[tokio::test]
async fn rw7_control_a_lock_failing_in_sql_returns_its_error_and_abandons_nothing() {
    within(60, async {
        let (db, pool) = pool(1).await;
        let p = project();
        let opts = TxOptions {
            statement_timeout: Some(Duration::from_millis(300)),
        };
        let mut tx = ModuleTx::begin(&pool, ctx("test_a", Some(p)), opts)
            .await
            .expect("begin");
        owner_write(&mut tx, p).await;
        let mut peer = peer_holding(db, GATE, 7207).await;
        assert_eq!(
            tx.advisory_lock(AdvisoryClass::ProducerGate, 7207).await,
            Err(DbError::StatementTimeout),
            "the SQL error is returned"
        );
        assert!(
            tx.executor().is_ok(),
            "an SQL failure is not an abandonment"
        );
        assert_eq!(
            tx.commit().await,
            Err(DbError::DelegationFailed),
            "poisoned"
        );
        sqlx::raw_sql("COMMIT")
            .execute(&mut peer)
            .await
            .expect("peer");
        assert_eq!(count_as_admin(db, "test_a.t", p).await, 0);
    })
    .await
}

/// Control: a callback that propagates the inner timeout as its error: the delegation returns that
/// error, and the transaction cannot commit.
#[tokio::test]
async fn rw7_control_a_propagated_timeout_returns_its_error() {
    within(60, async {
        let (db, pool) = pool(1).await;
        let p = project();
        let mut tx = begin(&pool, "test_a", Some(p)).await;
        owner_write(&mut tx, p).await;
        let mut peer = peer_holding(db, AUDIT, 7208).await;
        let releaser = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(800)).await;
            sqlx::raw_sql("COMMIT")
                .execute(&mut peer)
                .await
                .expect("peer");
        });
        let r = tx
            .delegate(&module("test_audit"), async |t| {
                tokio::time::timeout(
                    Duration::from_millis(300),
                    t.advisory_lock(AdvisoryClass::AuditChain, 7208),
                )
                .await
                .map_err(|_| DbError::Other("rw7: the chain lock timed out".into()))?
            })
            .await;
        releaser.await.expect("releaser");
        assert_eq!(
            r,
            Err(DbError::Other("rw7: the chain lock timed out".into())),
            "the callback's error"
        );
        assert_eq!(tx.commit().await, Err(DbError::DelegationFailed));
        assert_eq!(count_as_admin(db, "test_a.t", p).await, 0);
    })
    .await
}

/// Control: a healthy acquisition under a timeout completes, is recorded and sealed, and commits.
#[tokio::test]
async fn rw7_control_a_healthy_acquisition_under_a_timeout_commits() {
    within(60, async {
        let (db, pool) = pool(1).await;
        let p = project();
        let mut tx = begin(&pool, "test_a", Some(p)).await;
        owner_write(&mut tx, p).await;
        let project = uuid_of(p);
        tx.delegate(&module("test_audit"), async move |t| {
            tokio::time::timeout(
                Duration::from_secs(10),
                t.advisory_lock(AdvisoryClass::AuditChain, 7209),
            )
            .await
            .expect("no timeout")?;
            sqlx::query("insert into test_audit.r (project_id, v) values ($1, 'rw7a-healthy')")
                .bind(project)
                .execute(t.executor()?)
                .await
                .map(|_| ())
                .map_err(|e| DbError::Other(e.to_string()))
        })
        .await
        .expect("the delegation");
        assert_eq!(
            tx.lock_ledger().steps(),
            [LockStep::AuditChain { chain: 7209 }]
        );
        assert!(tx.lock_ledger().sealed());
        tx.commit().await.expect("commit");
        assert_eq!(count_as_admin(db, "test_a.t", p).await, 1);
        assert_eq!(count_as_admin(db, "test_audit.r", p).await, 1);
    })
    .await
}
