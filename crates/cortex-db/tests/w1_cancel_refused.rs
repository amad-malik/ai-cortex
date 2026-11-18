//! vera's DBCANCEL-L1 (REVIEW.md, 6 Nov): what happens when the cancel connection is REFUSED. `DRAIN_TIMEOUT` bounds the
//! cleanup TASK's wait, not the backend, the locks or the pool place: when the cancel cannot be sent, the final
//! timeout drops the rollback future and sqlx's own pool return (a ping that waits for the statement) takes over. This
//! cell states that fallback with SEPARATE observations of the task and of the resources. The testkit switches are
//! process-wide, so this file is a test binary of its own. DB-backed.
mod common;

use std::time::{Duration, Instant};

use common::{begin, pool};
use cortex_db::testkit::{
    cancelled_backends, finished_cleanups, set_cancel_refused, set_drain_timeout_ms,
};
use cortex_testkit::within;
use sqlx::Row;

async fn running(db: &cortex_testkit::TestDb, pid: i32) -> bool {
    let mut admin = db.admin().await;
    let n: i64 = sqlx::query(
        "select count(*) from pg_stat_activity \
         where pid = $1 and state = 'active' and query like '%pg_sleep(6)%'",
    )
    .bind(pid)
    .fetch_one(&mut admin)
    .await
    .expect("activity")
    .get(0);
    n > 0
}

#[tokio::test]
async fn cd_9_with_the_cancel_refused_the_task_ends_but_the_backend_and_the_pool_place_are_held() {
    within(90, async {
        set_cancel_refused(true);
        set_drain_timeout_ms(1000);
        let (db, pool) = pool(1).await;
        let mut tx = begin(&pool, "test_a", None).await;
        let pid: i32 = sqlx::query_scalar("select pg_backend_pid()")
            .fetch_one(tx.executor().expect("executor"))
            .await
            .expect("pid");
        let r = tokio::time::timeout(
            Duration::from_millis(300),
            sqlx::query("select pg_sleep(6)").execute(tx.executor().expect("executor")),
        )
        .await;
        assert!(r.is_err(), "the 6 s statement must still be running");
        let t0 = Instant::now();
        drop(tx);
        // (1) The TASK ends: grace 100 ms, a refused cancel, then the shortened drain of 1 s.
        while !finished_cleanups().contains(&pid) {
            assert!(
                t0.elapsed() < Duration::from_secs(4),
                "the cleanup task did not end within the shortened drain"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        // (2) No cancel was sent.
        assert!(
            !cancelled_backends().contains(&pid),
            "the cancel was refused"
        );
        // (3) The RESOURCES are not released by the task's end: the backend still runs the statement and the one pool
        // place is still held (sqlx's pool-return ping waits for the statement).
        assert!(
            running(db, pid).await,
            "the backend still runs the statement"
        );
        let held = tokio::time::timeout(
            Duration::from_millis(500),
            cortex_db::ModuleTx::begin(
                &pool,
                cortex_testkit::ctx("test_a", None),
                cortex_db::TxOptions::default(),
            ),
        )
        .await;
        assert!(
            held.is_err(),
            "the pool place is still held after the task has ended"
        );
        // (4) They come back when the statement ends by itself (about 6 s after it started).
        let next = tokio::time::timeout(
            Duration::from_secs(15),
            cortex_db::ModuleTx::begin(
                &pool,
                cortex_testkit::ctx("test_a", None),
                cortex_db::TxOptions::default(),
            ),
        )
        .await
        .expect("the place came back once the statement ended")
        .expect("begin");
        next.rollback().await.expect("rollback");
    })
    .await
}
