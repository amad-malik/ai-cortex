//! Wave 1, mod-handoffs follow-up after the merge (hale's note N-1 on re-read r1; cox: "after the merge, test-only"):
//! the SECOND line of W1-39 reached with a REAL Postgres error.
//!
//! The first line (the grammar in `validate.rs`) leaves no HTTP input that can reach a database time error, so the
//! cells in `w1_handoffs_p5_rfc3339_db.rs` cannot show that `time_db` is wired into the two statements that carry a
//! request date-time. These cells call the public store functions directly with a value the validator would refuse,
//! in a module transaction for `handoffs`, and the answer must be a 422 `invalid_request` from Postgres's own 22009
//! (time zone displacement out of range). With `time_db` unwired from either statement the answer is a 500.
//!
//! The two cells are hale's (her probe `rework-second-line-probe.sh`, `mvp/reviews/piece8-mod-handoffs-hale-receipts/`),
//! adopted with a positive control each: the same call with a value the store CAN hold succeeds, so the 422 is the
//! time and nothing else.
//!
//! Run ONLY under `crates/cortex-testkit/supervise.sh` through `cv2-admit`, `CV2_DB=1`, one run at a time.
mod common;

use common::{Seed, Who, project_a, world};
use cortex_db::TxOptions;
use cortex_testkit::ctx;
use cortex_types::{HandoffId, SystemClock};
use mod_handoffs::body::Kind;
use mod_handoffs::store::{self, ListFilter};
use mod_handoffs::validate::{CreateOk, To};

async fn tx(w: &common::World) -> cortex_db::ModuleTx {
    cortex_db::ModuleTx::begin(
        &w.pool,
        ctx("handoffs", Some(project_a())),
        TxOptions::default(),
    )
    .await
    .expect("a transaction")
}

fn create_ok(due_at: &str) -> CreateOk {
    CreateOk {
        kind: Kind::Task,
        to: To::JobRole("dev".into()),
        summary: "s".into(),
        next_action: "n".into(),
        acceptance: "a".into(),
        verify: None,
        context: None,
        files: vec![],
        blockers: vec![],
        decisions: vec![],
        priority: 50,
        due_at: Some(due_at.into()),
    }
}

/// `store::list` with a cursor time Postgres refuses (22009) is 422; with a time it holds, a page.
#[tokio::test]
async fn the_list_statement_maps_a_database_time_error_to_422() {
    let w = world("p6_list").await;
    // A row must exist: with none, Postgres never evaluates the cursor's timestamp.
    Seed::pending(Who::L).insert(&mut w.admin().await).await;
    let id = HandoffId::new(&SystemClock);
    let list = |at: &'static str| {
        let w = &w;
        async move {
            let mut t = tx(w).await;
            let f = ListFilter {
                state: None,
                holder: None,
                created_by: None,
                mine: false,
                me: Who::L.id(),
                me_active: true,
                roles: &[],
                lead: Who::L.id(),
                after: Some((50, at, id)),
                limit: 10,
            };
            let r = store::list(&mut t, &f).await;
            let _ = t.rollback().await;
            r.map(|rows| rows.len()).map_err(|p| (p.status, p.code))
        }
    };
    assert_eq!(
        list("2030-01-01T00:00:00+23:00").await,
        Err((422, "invalid_request")),
        "a cursor time the store cannot hold"
    );
    assert!(
        list("2030-01-01T00:00:00+14:00").await.is_ok(),
        "the control: a time the store holds is a page"
    );
}

/// `store::insert_handoff` with a `due_at` Postgres refuses (22009) is 422; with one it holds, the row is written.
#[tokio::test]
async fn the_insert_statement_maps_a_database_time_error_to_422() {
    let w = world("p6_insert").await;
    let insert = |due_at: &'static str| {
        let w = &w;
        async move {
            let mut t = tx(w).await;
            let r = store::insert_handoff(
                &mut t,
                HandoffId::new(&SystemClock),
                Who::L.id(),
                &create_ok(due_at),
            )
            .await;
            let _ = t.rollback().await;
            r.map_err(|p| (p.status, p.code))
        }
    };
    assert_eq!(
        insert("2030-01-01T00:00:00+23:00").await,
        Err((422, "invalid_request")),
        "a due_at the store cannot hold"
    );
    assert_eq!(
        insert("2030-01-01T00:00:00+14:00").await,
        Ok(()),
        "the control: a due_at the store holds is written"
    );
}
