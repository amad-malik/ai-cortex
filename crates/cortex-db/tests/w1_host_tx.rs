//! Wave 1 ports (d), W1-11: the HOST-PATH transaction. DB-backed (run under `supervise.sh`).
//!
//! `ModuleTx::begin_host` opens a transaction under the constant role `cortex_install_admin` with actor
//! `HostOperator{uid}`, for the host-socket paths (create the install, create a project) that no module role
//! may do. `delegate` out of it comes back to the HOST role, not to a `cortex_m_<x>`. No module id reaches it
//! (DB-free proofs in `pool.rs`'s `w1_host_role_tests`; here the live side).
//!
//! RED: `begin_host` is `todo!()`.
mod common;

use common::{begin, pool, project, who};
use cortex_db::{Actor, HOST_ROLE, HostContext, ModuleTx, TxOptions};
use cortex_testkit::{module, within};
use cortex_types::{RequestId, SystemClock};

fn host(uid: u32, project: Option<cortex_types::ProjectId>) -> HostContext {
    HostContext {
        uid,
        project,
        request_id: RequestId::new(&SystemClock),
    }
}

#[tokio::test]
async fn w1_h_3_a_host_transaction_runs_as_install_admin_with_a_host_operator_actor() {
    within(60, async {
        let (_db, pool) = pool(1).await;
        let mut tx = ModuleTx::begin_host(&pool, host(1000, None), TxOptions::default())
            .await
            .expect("begin_host");
        assert_eq!(tx.context().actor, Actor::HostOperator { uid: 1000 });
        assert_eq!(tx.context().project, None);
        assert_eq!(
            who(tx.executor().expect("executor")).await,
            (HOST_ROLE.to_owned(), None),
            "install-level: no project setting"
        );
        tx.rollback().await.expect("rollback");

        // An action that creates a project may name it before it exists.
        let p = project();
        let mut tx = ModuleTx::begin_host(&pool, host(0, Some(p)), TxOptions::default())
            .await
            .expect("begin_host with a project");
        assert_eq!(
            who(tx.executor().expect("executor")).await,
            (HOST_ROLE.to_owned(), Some(p.to_string()))
        );
        tx.rollback().await.expect("rollback");
    })
    .await
}

#[tokio::test]
async fn w1_h_4_delegate_out_of_a_host_transaction_comes_back_to_the_host_role() {
    within(60, async {
        let (_db, pool) = pool(1).await;
        let mut tx = ModuleTx::begin_host(&pool, host(1000, None), TxOptions::default())
            .await
            .expect("begin_host");
        let inside = tx
            .delegate(
                &module("test_audit"),
                async |t| Ok(who(t.executor()?).await),
            )
            .await
            .expect("delegate");
        assert_eq!(inside.0, "cortex_m_test_audit");
        assert_eq!(
            who(tx.executor().expect("executor")).await.0,
            HOST_ROLE,
            "the role came back to the HOST role, not to a module's"
        );
        assert_eq!(tx.context().actor, Actor::HostOperator { uid: 1000 });
        tx.rollback().await.expect("rollback");
    })
    .await
}

#[tokio::test]
async fn w1_h_5_a_module_transaction_is_never_the_host_role() {
    within(60, async {
        let (_db, pool) = pool(1).await;
        for m in ["test_a", "test_b", "test_audit", "test_m"] {
            let mut tx = begin(&pool, m, Some(project())).await;
            let (role, _) = who(tx.executor().expect("executor")).await;
            assert_eq!(role, format!("cortex_m_{m}"));
            assert_ne!(role, HOST_ROLE);
            tx.rollback().await.expect("rollback");
        }
        // A module id that looks like the role is not in the registry: no role, no transaction.
        let r = ModuleTx::begin(
            &pool,
            cortex_testkit::ctx("install_admin", None),
            TxOptions::default(),
        )
        .await;
        assert_eq!(r.err(), Some(cortex_db::DbError::UnknownModule));
    })
    .await
}
