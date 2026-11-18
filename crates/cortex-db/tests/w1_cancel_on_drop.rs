//! Lane A, cancel-on-drop (cox 14:48Z, nemo's finding): dropping a `ModuleTx` while a statement is in flight must
//! CANCEL the statement. Before: the backend kept running until the statement ended or `statement_timeout` fired
//! (a module role cannot `pg_cancel_backend`: the target's session role is `cortex_app`), and the pool kept the
//! connection busy, so a pool place stayed held and locks stayed held. No API change.
//!
//! The fix: a dropped `ModuleTx` hands its connection to a cleanup task that rolls back; if the rollback does not
//! finish within a short grace the connection is still busy, and the task, which STILL OWNS the connection, sends
//! `pg_cancel_backend(pid)` from a separate connection of the pool's own login role, then lets the rollback finish.
//! The connection is never in the pool while the cancel is sent, so the cancel cannot hit another request.
//!
//! DB-backed (run under `supervise.sh`). RED: dropping does nothing about the statement.
mod common;

use std::time::{Duration, Instant};

use common::{begin, pool};
use cortex_db::testkit::cancelled_backends;
use cortex_testkit::within;
use sqlx::Row;

/// Starts `select pg_sleep(60)` on the transaction and drops the statement's future after `after`, as a handler
/// timeout does; the statement is then in flight on a connection nobody is reading.
async fn leave_a_statement_running(tx: &mut cortex_db::ModuleTx, after: Duration) -> i32 {
    let pid: i32 = sqlx::query_scalar("select pg_backend_pid()")
        .fetch_one(tx.executor().expect("executor"))
        .await
        .expect("pid");
    let r = tokio::time::timeout(
        after,
        sqlx::query("select pg_sleep(60)").execute(tx.executor().expect("executor")),
    )
    .await;
    assert!(r.is_err(), "the 60 s statement must still be running");
    pid
}

/// Whether the backend is still running the long statement, read by the superuser.
async fn still_sleeping(db: &cortex_testkit::TestDb, pid: i32) -> bool {
    let mut admin = db.admin().await;
    let n: i64 = sqlx::query(
        "select count(*) from pg_stat_activity \
         where pid = $1 and state = 'active' and query like '%pg_sleep(60)%'",
    )
    .bind(pid)
    .fetch_one(&mut admin)
    .await
    .expect("activity")
    .get(0);
    n > 0
}

#[tokio::test]
async fn cd_1_dropping_a_transaction_mid_statement_cancels_the_backend() {
    within(60, async {
        let (db, pool) = pool(2).await;
        let mut tx = begin(&pool, "test_a", None).await;
        let pid = leave_a_statement_running(&mut tx, Duration::from_millis(300)).await;
        assert!(still_sleeping(db, pid).await, "before the drop it runs");
        let t0 = Instant::now();
        drop(tx);
        while still_sleeping(db, pid).await {
            assert!(
                t0.elapsed() < Duration::from_secs(5),
                "the statement still runs {:?} after the drop",
                t0.elapsed()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(
            cancelled_backends().contains(&pid),
            "the drop sent a cancel for this backend"
        );
    })
    .await
}

#[tokio::test]
async fn cd_2_the_pool_place_is_free_again_soon_after_the_drop() {
    within(60, async {
        // One place: the next `begin` can only be served by the connection the dropped transaction held.
        let (_db, pool) = pool(1).await;
        let mut tx = begin(&pool, "test_a", None).await;
        let _pid = leave_a_statement_running(&mut tx, Duration::from_millis(300)).await;
        drop(tx);
        let t0 = Instant::now();
        let next = tokio::time::timeout(
            Duration::from_secs(5),
            cortex_db::ModuleTx::begin(
                &pool,
                cortex_testkit::ctx("test_a", None),
                cortex_db::TxOptions::default(),
            ),
        )
        .await
        .expect("the place came back within 5 s (the statement would run for 60 s)")
        .expect("begin");
        assert!(t0.elapsed() < Duration::from_secs(5));
        next.rollback().await.expect("rollback");
    })
    .await
}

#[tokio::test]
async fn cd_3_a_lock_held_by_the_dropped_statement_is_released() {
    within(60, async {
        let (db, pool) = pool(2).await;
        let mut tx = begin(&pool, "test_a", None).await;
        sqlx::query("select pg_advisory_xact_lock(7770042)")
            .execute(tx.executor().expect("executor"))
            .await
            .expect("lock");
        let _pid = leave_a_statement_running(&mut tx, Duration::from_millis(300)).await;
        drop(tx);
        let mut admin = db.admin().await;
        let t0 = Instant::now();
        loop {
            let got: bool = sqlx::query_scalar("select pg_try_advisory_lock(7770042)")
                .fetch_one(&mut admin)
                .await
                .expect("try");
            if got {
                sqlx::query("select pg_advisory_unlock(7770042)")
                    .execute(&mut admin)
                    .await
                    .expect("unlock");
                break;
            }
            assert!(
                t0.elapsed() < Duration::from_secs(5),
                "the lock is still held"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
}

#[tokio::test]
async fn cd_4_a_normal_drop_sends_no_cancel_and_keeps_the_connection() {
    within(60, async {
        let (_db, pool) = pool(1).await;
        let mut tx = begin(&pool, "test_a", None).await;
        let pid: i32 = sqlx::query_scalar("select pg_backend_pid()")
            .fetch_one(tx.executor().expect("executor"))
            .await
            .expect("pid");
        // An error path: the transaction is dropped with nothing in flight.
        drop(tx);
        let mut tx = begin(&pool, "test_a", None).await;
        let again: i32 = sqlx::query_scalar("select pg_backend_pid()")
            .fetch_one(tx.executor().expect("executor"))
            .await
            .expect("pid");
        assert_eq!(
            pid, again,
            "the same connection served the next transaction"
        );
        // A cancel for an idle drop would be sent from a separate connection a few milliseconds after the drop and
        // recorded when it returns: wait well past that before saying none was sent. (The no-grace mutant LA-M4b
        // cancels after the rollback and was killed by cd_5, not by this cell, until it waited.)
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(
            !cancelled_backends().contains(&pid),
            "no cancel for an idle drop"
        );
        tx.rollback().await.expect("rollback");
    })
    .await
}

#[tokio::test]
async fn cd_5_a_cancel_never_hits_the_next_request_on_the_same_connection() {
    within(120, async {
        let (_db, pool) = pool(1).await;
        // Statements that end about when their future is dropped, so the cancel races the end; the next
        // transaction on the one connection then runs a 250 ms statement that must not be cancelled.
        for i in 0..40_u64 {
            let mut tx = begin(&pool, "test_a", None).await;
            let ms = 20 + (i % 8) * 5;
            let r = tokio::time::timeout(
                Duration::from_millis(30),
                sqlx::query(sqlx::AssertSqlSafe(format!(
                    "select pg_sleep({})",
                    ms as f64 / 1000.0
                )))
                .execute(tx.executor().expect("executor")),
            )
            .await;
            let _ = r;
            drop(tx);
            let mut next = begin(&pool, "test_a", None).await;
            let r = sqlx::query("select pg_sleep(0.25)")
                .execute(next.executor().expect("executor"))
                .await;
            assert!(
                r.is_ok(),
                "round {i}: the next statement was cancelled: {r:?}"
            );
            next.rollback().await.expect("rollback");
        }
    })
    .await
}

// ---- vera's DBCANCEL-V1 (REVIEW.md, 6 Nov): a terminal operation that is itself cancelled -------------------------
//
// `rollback` and a poisoned `commit` used to take the transaction out of the `ModuleTx` before awaiting it. If THAT
// future was cancelled while an earlier abandoned statement still ran, the `ModuleTx` was empty, its drop installed no
// cleanup, and sqlx's old behaviour (a queued rollback, a pool-return ping that waits for the statement) returned.

/// Everything the dropped transaction must release, observed separately: the backend stops, its pid is in the
/// oracle, the advisory lock it held is free, and the ONE pool place serves a new transaction, each within 5 s.
async fn expect_everything_released(
    db: &cortex_testkit::TestDb,
    pool: &cortex_db::DbPool,
    pid: i32,
    lock: i64,
) {
    let t0 = Instant::now();
    while still_sleeping(db, pid).await {
        assert!(
            t0.elapsed() < Duration::from_secs(5),
            "the statement still runs {:?} after the cancelled terminal operation",
            t0.elapsed()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        cancelled_backends().contains(&pid),
        "a cancel was sent for this backend"
    );
    let mut admin = db.admin().await;
    let got: bool = sqlx::query_scalar("select pg_try_advisory_lock($1)")
        .bind(lock)
        .fetch_one(&mut admin)
        .await
        .expect("try");
    assert!(got, "the advisory lock is released");
    sqlx::query("select pg_advisory_unlock($1)")
        .bind(lock)
        .execute(&mut admin)
        .await
        .expect("unlock");
    let next = tokio::time::timeout(
        Duration::from_secs(5),
        cortex_db::ModuleTx::begin(
            pool,
            cortex_testkit::ctx("test_a", None),
            cortex_db::TxOptions::default(),
        ),
    )
    .await
    .expect("the pool place came back within 5 s")
    .expect("begin");
    next.rollback().await.expect("rollback");
}

#[tokio::test]
async fn cd_6_a_cancelled_explicit_rollback_still_cancels_the_abandoned_statement() {
    within(60, async {
        let (db, pool) = pool(1).await;
        let mut tx = begin(&pool, "test_a", None).await;
        sqlx::query("select pg_advisory_xact_lock(7770061)")
            .execute(tx.executor().expect("executor"))
            .await
            .expect("lock");
        let pid = leave_a_statement_running(&mut tx, Duration::from_millis(300)).await;
        // The explicit rollback queues behind the running statement; its own future is cancelled too.
        let r = tokio::time::timeout(Duration::from_millis(300), tx.rollback()).await;
        assert!(
            r.is_err(),
            "the rollback waits behind the running statement"
        );
        expect_everything_released(db, &pool, pid, 7770061).await;
    })
    .await
}

#[tokio::test]
async fn cd_7_a_cancelled_poisoned_commit_still_cancels_the_abandoned_statement() {
    within(60, async {
        let (db, pool) = pool(1).await;
        let mut tx = begin(&pool, "test_a", None).await;
        sqlx::query("select pg_advisory_xact_lock(7770062)")
            .execute(tx.executor().expect("executor"))
            .await
            .expect("lock");
        let pid: i32 = sqlx::query_scalar("select pg_backend_pid()")
            .fetch_one(tx.executor().expect("executor"))
            .await
            .expect("pid");
        // A delegation dropped mid-statement abandons the transaction (the statement keeps running), so `commit`
        // takes its poisoned path: a rollback behind the running statement.
        let r = tokio::time::timeout(
            Duration::from_millis(300),
            tx.delegate(&cortex_testkit::module("test_audit"), async |t| {
                sqlx::query("select pg_sleep(60)")
                    .execute(t.executor()?)
                    .await?;
                Ok(())
            }),
        )
        .await;
        assert!(r.is_err(), "the delegated statement was still running");
        let r = tokio::time::timeout(Duration::from_millis(300), tx.commit()).await;
        assert!(
            r.is_err(),
            "the poisoned commit waits behind the running statement"
        );
        expect_everything_released(db, &pool, pid, 7770062).await;
    })
    .await
}

#[tokio::test]
async fn cd_8_a_successful_commit_or_rollback_sends_no_cancel_and_keeps_the_connection() {
    within(60, async {
        let (_db, pool) = pool(1).await;
        let mut seen: Vec<i32> = Vec::new();
        for round in 0..6 {
            let mut tx = begin(&pool, "test_a", None).await;
            let pid: i32 = sqlx::query_scalar("select pg_backend_pid()")
                .fetch_one(tx.executor().expect("executor"))
                .await
                .expect("pid");
            seen.push(pid);
            if round % 2 == 0 {
                tx.commit().await.expect("commit");
            } else {
                tx.rollback().await.expect("rollback");
            }
        }
        assert!(
            seen.iter().all(|p| *p == seen[0]),
            "one connection served every round: {seen:?}"
        );
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(
            !cancelled_backends().contains(&seen[0]),
            "no cancel for a terminal operation that completed"
        );
    })
    .await
}

/// vera's DBCANCEL-L2: a deterministic entry into the GENUINE cancel path. Each round leaves `pg_sleep(60)` running
/// (so the grace of 100 ms expires with the statement in flight), drops the transaction, and takes the connection
/// again: the connection is available only after the cleanup task is done, so by then the oracle has recorded THIS
/// round's cancel (exactly one more for the pid), taken while the task still owned the connection; the next
/// statement on the connection must then run to its end.
#[tokio::test]
async fn cd_5b_after_a_genuine_cancel_the_next_request_on_the_connection_is_untouched() {
    within(120, async {
        let (_db, pool) = pool(1).await;
        for round in 0..8 {
            let mut tx = begin(&pool, "test_a", None).await;
            let pid = leave_a_statement_running(&mut tx, Duration::from_millis(50)).await;
            let before = cancelled_backends().iter().filter(|p| **p == pid).count();
            drop(tx);
            let mut next = begin(&pool, "test_a", None).await;
            let after = cancelled_backends().iter().filter(|p| **p == pid).count();
            assert_eq!(
                after,
                before + 1,
                "round {round}: one cancel, sent before the connection came back"
            );
            // The SAME backend serves it (a pool of one could have replaced a connection; vera's R1): the cancel was
            // sent for the connection the next request actually uses.
            let again: i32 = sqlx::query_scalar("select pg_backend_pid()")
                .fetch_one(next.executor().expect("executor"))
                .await
                .expect("pid");
            assert_eq!(
                again, pid,
                "round {round}: the next request runs on the cancelled connection"
            );
            let r = sqlx::query("select pg_sleep(0.25)")
                .execute(next.executor().expect("executor"))
                .await;
            assert!(
                r.is_ok(),
                "round {round}: the next statement was disturbed: {r:?}"
            );
            next.rollback().await.expect("rollback");
        }
    })
    .await
}

/// vera's R2: what a CANCELLED `commit` does. It is not rolled back: its future continues in the cleanup task, and
/// the COMMIT (already queued behind the earlier work) is executed by the server when that work ends by itself within
/// the grace, so the data IS committed. (After a cancel of the earlier work the transaction is aborted and COMMIT
/// acts as ROLLBACK, as `cd_7` shows for the poisoned path.) A caller that times out around `commit` must treat the
/// outcome as unknown.
#[tokio::test]
async fn cd_10_a_cancelled_commit_is_continued_and_commits_when_the_earlier_work_ends_by_itself() {
    within(60, async {
        let (db, pool) = pool(1).await;
        let p = common::project();
        let mut tx = begin(&pool, "test_a", Some(p)).await;
        sqlx::query("insert into test_a.t (project_id, v) values ($1, 'cd10')")
            .bind(common::uuid_of(p))
            .execute(tx.executor().expect("executor"))
            .await
            .expect("insert");
        // An earlier statement that ends by itself (about 60 ms), abandoned at 20 ms.
        let r = tokio::time::timeout(
            Duration::from_millis(20),
            sqlx::query("select pg_sleep(0.06)").execute(tx.executor().expect("executor")),
        )
        .await;
        assert!(r.is_err(), "the earlier statement was still running");
        // The commit is cancelled after 10 ms, while it waits behind that statement.
        let r = tokio::time::timeout(Duration::from_millis(10), tx.commit()).await;
        assert!(r.is_err(), "the commit was still waiting");
        let t0 = Instant::now();
        loop {
            if common::count_as_admin(db, "test_a.t", p).await == 1 {
                break;
            }
            assert!(
                t0.elapsed() < Duration::from_secs(5),
                "the cancelled commit was not continued"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
}
