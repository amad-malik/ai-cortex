//! Wave 1, mod-handoffs P4f (row 2a86598d): the port boot reads, `HandoffsApi`, after cox W1-33 (sam's BOOT12-01):
//! `open_for` is gone; `claimed_by(w, limit)` returns the worker's CLAIMED handoffs, newest claim first, and
//! `pending_for(w, limit)` the PENDING ones the worker may claim, in HAND-110's order. Each is capped at
//! `HANDOFF_LIST_CAP` (1,000) whatever `limit` asks for. These cells replace the old HAND-110 test and add the
//! flood cell: 1,001 pending handoffs at priority 100 never hide a claimed one.
//!
//! The calls go through the provider the registry hands out, in a module transaction for project A (the project
//! and the actor come from the transaction, as for boot). Run ONLY under `crates/cortex-testkit/supervise.sh`
//! through `cv2-admit`, `CV2_DB=1`, one run at a time.
mod common;

use common::{Seed, Who, project_a, world};
use cortex_db::TxOptions;
use cortex_module::{HANDOFF_LIST_CAP, HandoffKind, HandoffTo, HandoffView};
use cortex_testkit::ctx;
use cortex_types::HandoffId;

fn ids(v: &[HandoffView]) -> Vec<String> {
    v.iter().map(|h| h.id.to_string()).collect()
}

fn of(list: &[HandoffId]) -> Vec<String> {
    list.iter().map(ToString::to_string).collect()
}

async fn tx(w: &common::World) -> cortex_db::ModuleTx {
    cortex_db::ModuleTx::begin(
        &w.pool,
        ctx(common::CALLER_MODULE, Some(project_a())),
        TxOptions::default(),
    )
    .await
    .expect("a transaction")
}

async fn claimed_by(w: &common::World, who: Who, limit: u32) -> Vec<HandoffView> {
    let mut t = tx(w).await;
    let got = w
        .port
        .claimed_by(&mut t, &who.id(), limit)
        .await
        .expect("claimed_by");
    t.rollback().await.expect("rollback");
    got
}

async fn pending_for(w: &common::World, who: Who, limit: u32) -> Vec<HandoffView> {
    let mut t = tx(w).await;
    let got = w
        .port
        .pending_for(&mut t, &who.id(), limit)
        .await
        .expect("pending_for");
    t.rollback().await.expect("rollback");
    got
}

// ---- claimed_by ------------------------------------------------------------------------------------------

/// HAND-110 (claimed): what the worker holds, newest claim first (then id). The order is the order the claims
/// were made through the route, not the priority or the age of the handoff; a returned handoff, another worker's
/// and another project's never appear; what W1 holds does not depend on how it is addressed; `limit` is honoured,
/// and a `limit` of 0 still answers one row (the cap applies from 1).
#[tokio::test]
async fn claimed_by_is_the_workers_claims_newest_first() {
    let w = world("p4f_claimed").await;
    let mut admin = w.admin().await;
    // Created oldest to newest in the order a, b, c, but claimed in the order b, c, a (through the route).
    let a = Seed::pending(Who::L)
        .addressed_to_worker(Who::W1)
        .priority(10)
        .insert(&mut admin)
        .await;
    let b = Seed::pending(Who::L)
        .addressed_to_worker(Who::W1)
        .priority(90)
        .insert(&mut admin)
        .await;
    let c = Seed::pending(Who::L).priority(50).insert(&mut admin).await;
    for id in [b, c, a] {
        let r = w.claim(Who::W1, id).await;
        assert_eq!(r.status.as_u16(), 200, "{}", r.text);
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    // Not W1's list: a claim by W2, a handoff W1 returned, another project's claimed handoff.
    let other = Seed::pending(Who::L)
        .claimed_by(Who::W2)
        .insert(&mut admin)
        .await;
    let mut returned = Seed::pending(Who::L).claimed_by(Who::W1);
    returned.state = "returned";
    returned.insert(&mut admin).await;
    let foreign = Seed::pending(Who::B1)
        .claimed_by(Who::B2)
        .insert(&mut admin)
        .await;
    // Held by W1 though addressed to W3: what is held never depends on addressing (HAND-24).
    let off_address = Seed::pending(Who::L)
        .addressed_to_worker(Who::W3)
        .claimed_by(Who::W1)
        .insert(&mut admin)
        .await;

    let got = claimed_by(&w, Who::W1, 50).await;
    // The seeded off-address claim was written last of all, so it is the newest.
    assert_eq!(
        ids(&got),
        of(&[off_address, a, c, b]),
        "newest claim first, whatever the priority"
    );
    assert!(got.iter().all(|h| h.state == "claimed"));
    assert!(!ids(&got).contains(&other.to_string()));
    assert!(!ids(&got).contains(&foreign.to_string()));

    let one = claimed_by(&w, Who::W1, 1).await;
    assert_eq!(ids(&one), of(&[off_address]), "limit 1: the newest claim");
    let zero = claimed_by(&w, Who::W1, 0).await;
    assert_eq!(zero.len(), 1, "a limit of 0 still answers one row");

    // The view: kind, creator, address, state, priority, summary.
    let h = got.iter().find(|h| h.id == c).expect("c");
    assert_eq!(h.kind, HandoffKind::Task);
    assert_eq!(h.from, Who::L.id());
    assert_eq!(h.to, HandoffTo::JobRole("dev".into()));
    assert_eq!(h.priority, 50);
    assert_eq!(h.summary, "seeded summary");
    let hb = got.iter().find(|h| h.id == b).expect("b");
    assert_eq!(hb.to, HandoffTo::Worker(Who::W1.id()));

    // A worker who holds nothing; a retired worker still sees what it holds.
    assert!(claimed_by(&w, Who::W4, 10).await.is_empty());
    let held_by_retired = Seed::pending(Who::L)
        .claimed_by(Who::W5)
        .insert(&mut admin)
        .await;
    assert_eq!(
        ids(&claimed_by(&w, Who::W5, 10).await),
        of(&[held_by_retired])
    );
}

// ---- pending_for -----------------------------------------------------------------------------------------

/// HAND-110 (pending): what the worker may claim, in priority order (descending), then `created_at` ascending, then
/// id. By worker id and by a held job role; after a rework only the rework holder; never claimed, returned or
/// closed ones; never another worker's, another project's, or a role the worker does not hold; an inactive worker
/// may claim nothing; `limit` is honoured.
#[tokio::test]
async fn pending_for_is_what_the_worker_may_claim_in_order() {
    let w = world("p4f_pending").await;
    let mut admin = w.admin().await;
    let low = Seed::pending(Who::L)
        .addressed_to_worker(Who::W1)
        .priority(10)
        .insert(&mut admin)
        .await;
    let high = Seed::pending(Who::L)
        .addressed_to_worker(Who::W1)
        .priority(90)
        .insert(&mut admin)
        .await;
    // By a held job role (W1 and W2 hold `dev`), the same priority as `tie_b`: created_at breaks the tie.
    let tie_a = Seed::pending(Who::L)
        .priority(50)
        .at("2026-11-01 10:00:00+00")
        .insert(&mut admin)
        .await;
    let tie_b = Seed::pending(Who::L)
        .priority(50)
        .at("2026-11-01 11:00:00+00")
        .insert(&mut admin)
        .await;
    // Not W1's.
    let to_w2 = Seed::pending(Who::L)
        .addressed_to_worker(Who::W2)
        .insert(&mut admin)
        .await;
    let qa = Seed::pending(Who::L)
        .addressed_to_role("qa")
        .insert(&mut admin)
        .await;
    let held = Seed::pending(Who::L)
        .claimed_by(Who::W1)
        .insert(&mut admin)
        .await;
    let mut returned = Seed::pending(Who::L).claimed_by(Who::W1);
    returned.state = "returned";
    returned.insert(&mut admin).await;
    let foreign = Seed::pending(Who::B1)
        .addressed_to_worker(Who::B2)
        .insert(&mut admin)
        .await;
    // After a rework: addressed to W1, but only W2 (who returned it) may claim it.
    let mut reworked = Seed::pending(Who::L).addressed_to_worker(Who::W1);
    reworked.rework_holder = Some(Who::W2);
    reworked.round = 2;
    let reworked = reworked.insert(&mut admin).await;

    let got = pending_for(&w, Who::W1, 50).await;
    assert_eq!(
        ids(&got),
        of(&[high, tie_a, tie_b, low]),
        "priority descending, then created_at ascending"
    );
    assert!(got.iter().all(|h| h.state == "pending"));
    for not_mine in [to_w2, qa, held, foreign, reworked] {
        assert!(
            !ids(&got).contains(&not_mine.to_string()),
            "{not_mine} must not be offered to W1"
        );
    }
    // W2: the role handoffs (the two dated ones first), its own, and the one it must rework, all at priority 50 and
    // so by created_at; none of W1's own.
    let got2 = pending_for(&w, Who::W2, 50).await;
    assert_eq!(
        ids(&got2),
        of(&[tie_a, tie_b, to_w2, reworked]),
        "W2's list, and the rework lock names W2"
    );

    // limit
    assert_eq!(pending_for(&w, Who::W1, 2).await.len(), 2);
    assert_eq!(ids(&pending_for(&w, Who::W1, 1).await), of(&[high]));
    // An inactive (retired) worker may claim nothing; an unknown id gets an empty list.
    assert!(pending_for(&w, Who::W5, 50).await.is_empty());
    assert!(pending_for(&w, Who::W4, 50).await.is_empty());
    let mut t = tx(&w).await;
    let unknown = w
        .port
        .pending_for(
            &mut t,
            &cortex_types::WorkerId::parse("0193e6a0-0000-7000-8000-0000000000ff")
                .expect("a worker id"),
            10,
        )
        .await
        .expect("an unknown worker");
    assert!(unknown.is_empty());
    t.rollback().await.expect("rollback");
}

// ---- the flood -------------------------------------------------------------------------------------------

/// W1-33 / sam BOOT12-01: 1,001 pending handoffs at priority 100 never hide a claimed one. `claimed_by` is its
/// own list: the claimed handoff (priority 1) is there however much pending work is queued; `pending_for` stops at
/// the cap (1,000) however large a limit is asked for, and returns the first 1,000 in order.
#[tokio::test]
async fn a_flood_of_pending_work_never_hides_a_claimed_handoff() {
    let w = world("p4f_flood").await;
    let mut admin = w.admin().await;
    let claimed = Seed::pending(Who::L)
        .priority(1)
        .claimed_by(Who::W1)
        .insert(&mut admin)
        .await;
    for _ in 0..=HANDOFF_LIST_CAP {
        Seed::pending(Who::L)
            .addressed_to_worker(Who::W1)
            .priority(100)
            .insert(&mut admin)
            .await;
    }
    let pending: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM handoffs.handoffs WHERE state = 'pending' AND priority = 100",
    )
    .fetch_one(&mut admin)
    .await
    .expect("count");
    assert_eq!(pending, i64::from(HANDOFF_LIST_CAP) + 1, "1,001 queued");

    let held = claimed_by(&w, Who::W1, 10).await;
    assert_eq!(
        ids(&held),
        of(&[claimed]),
        "the claimed handoff is in its own list"
    );

    let capped = pending_for(&w, Who::W1, 100_000).await;
    assert_eq!(
        capped.len(),
        HANDOFF_LIST_CAP as usize,
        "the cap, per method"
    );
    assert!(
        capped
            .iter()
            .all(|h| h.state == "pending" && h.priority == 100)
    );
    let first_1000: Vec<String> = sqlx::query_scalar(
        "SELECT id::text FROM handoffs.handoffs WHERE state = 'pending' \
         ORDER BY priority DESC, created_at ASC, id ASC LIMIT 1000",
    )
    .fetch_all(&mut admin)
    .await
    .expect("first 1000");
    assert_eq!(ids(&capped), first_1000, "the first 1,000 in order");
    assert!(
        !ids(&capped).contains(&claimed.to_string()),
        "a claimed handoff is never in the pending list"
    );
}
