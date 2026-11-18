//! F1-32's inverted-order control (TESTS r7 §2; the same shape as I1-07's). Its own binary: it
//! flips the process-wide seal switch. Two coordinated sessions: T1 appends (holding
//! `AuditChain`) and then updates row R; T2 updates R and then appends. A barrier makes both hold
//! their first lock before either asks for its second.
//! - **Seal off:** Postgres breaks a real deadlock and that transaction rolls back whole; the
//!   other commits. r2 (mike's F1B-R0-03): `Busy` covers both `40P01` and `55P03`, so it is not
//!   the deadlock witness. The witness is the database's own `pg_stat_database.deadlocks` counter,
//!   which must rise by exactly one, with `lock_timeout` proven off (0) in both sessions, so a lock
//!   timeout cannot pose as the deadlock. The victim's whole transaction is proven rolled back:
//!   neither its append nor its update of R remains.
//! - **Seal on:** T1's update is refused with `LockOrder` before any lock is requested; T2 commits;
//!   no deadlock forms.
mod common;

use std::sync::Arc;

use cortex_db::{AdvisoryClass, DbError, DbPool, ModuleTx};
use cortex_testkit::{module, within};
use tokio::sync::Barrier;

async fn append(tx: &mut ModuleTx, tag: String) -> Result<(), DbError> {
    tx.delegate(&module("test_audit"), async move |t| {
        t.advisory_lock(AdvisoryClass::AuditChain, 77).await?;
        sqlx::query("insert into test_audit.r (v) values ($1)")
            .bind(&tag)
            .execute(t.executor()?)
            .await
            .map(|_| ())
            .map_err(|e| DbError::Other(e.to_string()))
    })
    .await
}

async fn lock_timeout(tx: &mut ModuleTx) -> String {
    sqlx::query_scalar("show lock_timeout")
        .fetch_one(tx.executor().expect("executor"))
        .await
        .expect("lock_timeout")
}

async fn update_r(tx: &mut ModuleTx) -> Result<(), DbError> {
    sqlx::query("update test_a.locks set v = v + 1 where id = 2")
        .execute(tx.executor()?)
        .await
        .map(|_| ())
        .map_err(|e| match cortex_testkit::sqlstate(&e).as_deref() {
            Some("40P01") | Some("55P03") => DbError::Busy,
            _ => DbError::Other(e.to_string()),
        })
}

/// Runs the schedule; returns (T1's outcome, T2's outcome). Each side's append carries
/// `<run>-T1` or `<run>-T2`, so a rolled-back side leaves no row.
async fn schedule(pool: Arc<DbPool>, run: &str) -> (Result<(), DbError>, Result<(), DbError>) {
    let gate = Arc::new(Barrier::new(2));
    let (p1, g1, tag1) = (Arc::clone(&pool), Arc::clone(&gate), format!("{run}-T1"));
    let t1 = tokio::spawn(async move {
        let mut tx = common::begin(&p1, "test_a", None).await;
        assert_eq!(
            lock_timeout(&mut tx).await,
            "0",
            "T1: a lock timeout could pose as a deadlock"
        );
        append(&mut tx, tag1).await.expect("T1 holds AuditChain");
        g1.wait().await;
        let r = update_r(&mut tx).await;
        match r {
            Ok(()) => tx.commit().await,
            Err(e) => {
                tx.rollback().await.ok();
                Err(e)
            }
        }
    });
    let (p2, g2, tag2) = (Arc::clone(&pool), Arc::clone(&gate), format!("{run}-T2"));
    let t2 = tokio::spawn(async move {
        let mut tx = common::begin(&p2, "test_a", None).await;
        assert_eq!(
            lock_timeout(&mut tx).await,
            "0",
            "T2: a lock timeout could pose as a deadlock"
        );
        update_r(&mut tx).await.expect("T2 holds row R");
        g2.wait().await;
        let r = append(&mut tx, tag2).await;
        match r {
            Ok(()) => tx.commit().await,
            Err(e) => {
                tx.rollback().await.ok();
                Err(e)
            }
        }
    });
    (t1.await.expect("T1"), t2.await.expect("T2"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn f1_32_control_the_inverted_order_deadlocks_without_the_seal_and_is_refused_with_it() {
    within(90, async {
        let (_db, pool) = common::pool(2).await;
        let pool = Arc::new(pool);

        let db = cortex_testkit::test_db().await;
        let deadlocks_before = deadlocks(db).await;
        let (t1, t2) = schedule(Arc::clone(&pool), "on").await;
        assert!(
            matches!(t1, Err(DbError::LockOrder { .. })),
            "seal on: T1's update must be refused: {t1:?}"
        );
        assert_eq!(t2, Ok(()), "seal on: T2 must commit");
        assert_eq!(
            deadlocks(db).await,
            deadlocks_before,
            "seal on: a deadlock formed"
        );

        cortex_db::testkit::set_seal_off(true);
        let r_before = r_value(db).await;
        let (t1, t2) = schedule(Arc::clone(&pool), "off").await;
        cortex_db::testkit::set_seal_off(false);
        let busy = [&t1, &t2]
            .iter()
            .filter(|r| matches!(r, Err(DbError::Busy)))
            .count();
        let ok = [&t1, &t2].iter().filter(|r| r.is_ok()).count();
        assert_eq!(
            (busy, ok),
            (1, 1),
            "seal off: one victim and one commit expected: {t1:?} {t2:?}"
        );
        // The deadlock witness: the server's own counter, exactly one more.
        let mut seen = deadlocks(db).await;
        for _ in 0..100 {
            if seen > deadlocks_before {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            seen = deadlocks(db).await;
        }
        assert_eq!(
            seen,
            deadlocks_before + 1,
            "seal off: not exactly one deadlock detected by the server"
        );
        // The victim rolled back whole: its own append is gone, and R moved once (the winner's).
        let (winner, victim) = if t1.is_ok() {
            ("off-T1", "off-T2")
        } else {
            ("off-T2", "off-T1")
        };
        assert_eq!(
            appended(db, winner).await,
            1,
            "the winner's append is missing"
        );
        assert_eq!(
            appended(db, victim).await,
            0,
            "the victim's append survived"
        );
        assert_eq!(r_value(db).await, r_before + 1, "R moved other than once");
    })
    .await
}

/// `pg_stat_database.deadlocks` for this database, fresh (the snapshot cleared first).
async fn deadlocks(db: &cortex_testkit::TestDb) -> i64 {
    let mut admin = db.admin().await;
    sqlx::query("select pg_stat_clear_snapshot()")
        .execute(&mut admin)
        .await
        .expect("clear snapshot");
    sqlx::query_scalar("select deadlocks from pg_stat_database where datname = current_database()")
        .fetch_one(&mut admin)
        .await
        .expect("deadlocks")
}

async fn r_value(db: &cortex_testkit::TestDb) -> i32 {
    let mut admin = db.admin().await;
    sqlx::query_scalar("select v from test_a.locks where id = 2")
        .fetch_one(&mut admin)
        .await
        .expect("R")
}

async fn appended(db: &cortex_testkit::TestDb, tag: &str) -> i64 {
    let mut admin = db.admin().await;
    sqlx::query_scalar("select count(*) from test_audit.r where v = $1")
        .bind(tag)
        .fetch_one(&mut admin)
        .await
        .expect("appends")
}
