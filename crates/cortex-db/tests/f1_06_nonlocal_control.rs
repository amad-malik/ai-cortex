//! F1-06's nonlocal control, qualified (vera's Z2-ROLE-N01; TESTS r7). Its own binary: it sets
//! the process-wide testkit switch. In the chosen form, after `Commit`, on a session proven to be
//! the same one: without the switch the next transaction is clean; with it, the case asserts
//! **separately** that the role leaked and that the project leaked. Rollback, error and drop are
//! not control endings (a session `SET` inside them rolls back too).
mod common;

use common::{begin, pool, project, who};
use cortex_testkit::{same_session, session_identity, within};

#[tokio::test]
async fn f1_06_nonlocal_switch_leaks_role_and_project_on_the_same_session() {
    within(60, async {
        let (_db, pool) = pool(1).await;
        let raw = cortex_db::testkit::raw_pool(&pool);

        // The normal case, on this session.
        let mut tx = begin(&pool, "test_a", Some(project())).await;
        let s0 = session_identity(tx.executor().expect("executor")).await;
        tx.commit().await.expect("commit");
        let mut next = raw.begin().await.expect("next");
        same_session(&s0, &session_identity(&mut next).await)
            .expect("the normal case reuses the session");
        let (user, proj) = who(&mut next).await;
        assert_eq!(user, "cortex_app", "the normal case leaked the role");
        assert!(
            proj.as_deref().unwrap_or("").is_empty(),
            "the normal case leaked the project"
        );
        next.rollback().await.ok();

        // The control: session scope.
        cortex_db::testkit::set_nonlocal(true);
        let p = project();
        let mut tx = begin(&pool, "test_a", Some(p)).await;
        let s1 = session_identity(tx.executor().expect("executor")).await;
        tx.commit().await.expect("commit");
        let mut next = raw.begin().await.expect("next");
        same_session(&s1, &session_identity(&mut next).await)
            .expect("the control must run on the same session");
        let (user, proj) = who(&mut next).await;
        cortex_db::testkit::set_nonlocal(false);
        assert_eq!(
            user, "cortex_m_test_a",
            "the control did not leak the role: the leak check would not see it"
        );
        assert_eq!(
            proj,
            Some(p.to_string()),
            "the control did not leak the project: the leak check would not see it"
        );
        next.rollback().await.ok();
    })
    .await
}
