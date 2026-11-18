//! F1-32 Lock order and the audit seal. [BASE] (TESTS r7 §2; CONTRACTS §3 "Lock order", r6;
//! mike's R5-AUDIT-01.) A test `AuditSink` double appends through `delegate(test_audit, …)` and
//! takes `AuditChain` exactly as I-1 does. The inverted-order deadlock control is its own binary.
mod common;

use common::{pool, project};
use cortex_db::{AdvisoryClass, DbError, DbPool, LockStep, ModuleTx, TxOptions};
use cortex_testkit::{ctx, module, test_db, within};
use sqlx::Row;

/// The audit double: take the chain's lock as its own statement, then append.
async fn audit_append(tx: &mut ModuleTx, chain: i32) -> Result<(), DbError> {
    tx.delegate(&module("test_audit"), async |t| {
        t.advisory_lock(AdvisoryClass::AuditChain, chain).await?;
        sqlx::query("insert into test_audit.r (v) values ('appended')")
            .execute(t.executor()?)
            .await
            .map(|_| ())
            .map_err(|e| DbError::Other(e.to_string()))
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
async fn f1_32_the_ascending_order_passes() {
    within(60, async {
        let (_db, pool) = pool(1).await;
        let mut tx = common::begin(&pool, "test_a", Some(project())).await;
        tx.advisory_lock(AdvisoryClass::Idempotency, 7)
            .await
            .expect("1");
        tx.advisory_lock(AdvisoryClass::ProducerGate, 7)
            .await
            .expect("3");
        sqlx::query("update test_a.locks set v = v + 1 where id = 1")
            .execute(tx.executor().expect("owner rows"))
            .await
            .expect("owner rows");
        audit_append(&mut tx, 42).await.expect("audit last");
        assert_eq!(
            tx.lock_ledger().steps(),
            &[
                LockStep::Idempotency,
                LockStep::ProducerGate,
                LockStep::AuditChain { chain: 42 }
            ]
        );
        assert!(tx.lock_ledger().sealed());
        tx.commit().await.expect("commit");
    })
    .await
}

#[tokio::test]
async fn f1_32_a_lower_class_after_a_higher_one_is_refused_before_any_sql() {
    within(60, async {
        let (_db, pool) = pool(1).await;
        let mut tx = common::begin(&pool, "test_a", None).await;
        tx.advisory_lock(AdvisoryClass::ProducerGate, 1)
            .await
            .expect("3");
        let e = tx.advisory_lock(AdvisoryClass::Idempotency, 1).await;
        assert_eq!(
            e,
            Err(DbError::LockOrder {
                held: LockStep::ProducerGate,
                wanted: LockStep::Idempotency
            })
        );
        assert_eq!(
            my_advisory_locks(&mut tx, 1).await,
            0,
            "the refused lock reached the server"
        );
        assert_eq!(
            tx.commit().await,
            Err(DbError::DelegationFailed),
            "a LockOrder poisons the transaction"
        );
    })
    .await
}

#[tokio::test]
async fn f1_32_after_an_audit_append_the_transaction_is_sealed() {
    within(60, async {
        let (_db, pool) = pool(1).await;
        // Each refusal in its own transaction: each poisons.
        type Probe = fn(
            &mut ModuleTx,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<(), DbError>> + '_>,
        >;
        let probes: [(&str, Probe); 4] = [
            ("executor", |t| {
                Box::pin(async move { t.executor().map(|_| ()) })
            }),
            ("delegate(test_b)", |t| {
                Box::pin(async move { t.delegate(&module("test_b"), async |_| Ok(())).await })
            }),
            ("advisory_lock(Idempotency)", |t| {
                Box::pin(async move { t.advisory_lock(AdvisoryClass::Idempotency, 9).await })
            }),
            ("a second chain", |t| {
                Box::pin(async move { audit_append(t, 43).await })
            }),
        ];
        for (name, probe) in probes {
            let mut tx = common::begin(&pool, "test_a", Some(project())).await;
            audit_append(&mut tx, 42).await.expect("append");
            let r = probe(&mut tx).await;
            assert!(
                matches!(r, Err(DbError::LockOrder { .. })),
                "{name} after the seal: {r:?}"
            );
            assert_eq!(
                tx.commit().await,
                Err(DbError::DelegationFailed),
                "{name}: the refusal must poison"
            );
        }
        // A second append to the same chain is the audit-scoped path, and succeeds.
        let mut tx = common::begin(&pool, "test_a", Some(project())).await;
        audit_append(&mut tx, 42).await.expect("first append");
        audit_append(&mut tx, 42).await.expect("same chain again");
        tx.commit().await.expect("commit");
    })
    .await
}

#[tokio::test]
async fn f1_32_begin_refuses_anything_but_read_committed() {
    within(60, async {
        let db = test_db().await;
        let mut cfg = db.db_config(common::MODULES, common::clock());
        cfg.app_user = "cortex_app_rr".into(); // default_transaction_isolation = repeatable read
        let pool = DbPool::connect(&cfg).await.expect("connect");
        let r = ModuleTx::begin(&pool, ctx("test_a", None), TxOptions::default()).await;
        assert_eq!(r.err(), Some(DbError::Isolation));
    })
    .await
}
