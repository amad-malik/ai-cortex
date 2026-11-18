//! F1-06 Role and project per transaction, with no leak. [BASE] (lane A TESTS r7 §2; S1 §9.)
//! Z2-1 and Z2-2 over `ModuleTx`, in Z-2's form (param, provisional). The reuse oracle is the
//! session's backend pid plus a random session token, as TESTS r7 F1-06 and Z-2 have it. r7: the pre-connection refusal. The nonlocal
//! control and the cache witness are their own binaries (process-wide switch; pool shape).
//! RED: `DbPool::connect`, `ModuleTx::begin` and the rest are `todo!()`.
mod common;

use common::{begin, pool, project, who};
use cortex_db::{DbError, DbPool, ModuleTx, TxOptions};
use cortex_testkit::{ctx, module, same_session, session_identity, test_db, within};

#[tokio::test]
async fn f1_06_role_and_project_are_set_for_the_transaction() {
    within(60, async {
        let (_db, pool) = pool(2).await;
        let p = project();
        let mut tx = begin(&pool, "test_a", Some(p)).await;
        let (user, proj) = who(tx.executor().expect("executor")).await;
        assert_eq!(user, "cortex_m_test_a");
        assert_eq!(proj, Some(p.to_string()));
        tx.rollback().await.expect("rollback");
    })
    .await
}

#[derive(Clone, Copy, Debug)]
enum End {
    Commit,
    Rollback,
    Error,
    Dropped,
}

/// Z2-2: after each ending, the next transaction on the **same** session (proven by the oracle)
/// runs as `cortex_app` with no project. A replacement fails the case; it is never counted as reuse.
#[tokio::test]
async fn f1_06_nothing_leaks_to_the_next_transaction_on_a_proven_reused_session() {
    within(60, async {
        let (_db, pool) = pool(1).await;
        let raw = cortex_db::testkit::raw_pool(&pool);
        for end in [End::Commit, End::Rollback, End::Error, End::Dropped] {
            let p = project();
            let mut tx = begin(&pool, "test_a", Some(p)).await;
            let before = session_identity(tx.executor().expect("executor")).await;
            sqlx::query("insert into test_a.t (project_id, v) values ($1, 'f1-06')")
                .bind(common::uuid_of(p))
                .execute(tx.executor().expect("executor"))
                .await
                .expect("owner insert");
            match end {
                End::Commit => tx.commit().await.expect("commit"),
                End::Rollback => tx.rollback().await.expect("rollback"),
                End::Error => {
                    assert!(
                        sqlx::query("select 1/0")
                            .execute(tx.executor().expect("executor"))
                            .await
                            .is_err()
                    );
                    tx.rollback().await.expect("rollback after an error");
                }
                End::Dropped => drop(tx),
            }
            let mut next = raw.begin().await.expect("next transaction");
            let after = session_identity(&mut next).await;
            if let Err(why) = same_session(&before, &after) {
                panic!("after {end:?}: reuse not proven: {why}");
            }
            let (user, proj) = who(&mut next).await;
            assert_eq!(user, "cortex_app", "after {end:?}: the role leaked");
            assert!(
                proj.as_deref().unwrap_or("").is_empty(),
                "after {end:?}: the project leaked"
            );
            next.rollback().await.ok();
        }
    })
    .await
}

/// The oracle's control: a session ended from outside is reported as replaced, although the role
/// and project checks pass on the fresh one.
#[tokio::test]
async fn f1_06_control_a_forced_replacement_is_detected() {
    within(60, async {
        let (db, pool) = pool(1).await;
        let raw = cortex_db::testkit::raw_pool(&pool);
        let mut tx = begin(&pool, "test_a", Some(project())).await;
        let before = session_identity(tx.executor().expect("executor")).await;
        tx.commit().await.expect("commit");
        let mut admin = db.admin().await;
        sqlx::query("select pg_terminate_backend($1)")
            .bind(before.pid)
            .execute(&mut admin)
            .await
            .expect("terminate");
        let mut next = loop {
            if let Ok(mut t) = raw.begin().await
                && sqlx::query("select 1").execute(&mut *t).await.is_ok()
            {
                break t;
            }
        };
        let after = session_identity(&mut next).await;
        assert_eq!(
            who(&mut next).await.0,
            "cortex_app",
            "a fresh session is clean"
        );
        assert!(
            same_session(&before, &after).is_err(),
            "the oracle accepted a replaced session: {before:?} -> {after:?}"
        );
        next.rollback().await.ok();
    })
    .await
}

/// r7: an unregistered module is refused before a connection is taken (a dead pool shows it), and
/// `delegate` refuses one before any delegated SQL. Control: a registered module on the dead pool
/// tries to connect, and fails with a connection error.
#[tokio::test]
async fn f1_06_an_unregistered_module_is_refused_before_a_connection() {
    within(60, async {
        let db = test_db().await;
        let dead = DbPool::connect_lazy(&db.dead_db_config(common::MODULES, common::clock()));
        let e = ModuleTx::begin(&dead, ctx("not_registered", None), TxOptions::default())
            .await
            .err();
        assert_eq!(
            e,
            Some(DbError::UnknownModule),
            "begin did not refuse before connecting"
        );
        let c = ModuleTx::begin(&dead, ctx("test_a", None), TxOptions::default())
            .await
            .err()
            .expect("the dead pool connected");
        // r2 (mike's F1B-R0-04): exactly the acquisition failure, so an early isolation,
        // permission or lock refusal cannot pass for "it tried to connect".
        assert_eq!(
            c,
            DbError::Unavailable,
            "a registered module on the dead pool must fail at acquisition"
        );

        let (_db, pool) = pool(1).await;
        let mut tx = begin(&pool, "test_a", None).await;
        let mut ran = false;
        let r = tx
            .delegate(&module("nowhere"), async |_t| {
                ran = true;
                Ok(())
            })
            .await;
        assert_eq!(r, Err(DbError::UnknownModule));
        assert!(!ran, "the delegated closure ran for an unregistered module");
        tx.rollback().await.ok();
    })
    .await
}
