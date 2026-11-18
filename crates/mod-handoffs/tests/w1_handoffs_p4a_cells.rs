//! Wave 1, mod-handoffs P4a (row 2a86598d): the cells of `mvp/tests/mod-handoffs.md` r3.4 that P2 and P3 did not
//! reach and that need no second connection: HAND-14 (no incarnation), HAND-34 (the lost race's body), HAND-35
//! (no lease expiry), HAND-57 and HAND-58 (accept), HAND-63 and HAND-64 (release), HAND-82, HAND-85 and
//! HAND-86 (the key), HAND-111 and HAND-112 (audit). Handoffs are seeded as the superuser; the moves go through
//! the routes behind the real chain.
//!
//! Run ONLY under `crates/cortex-testkit/supervise.sh` through `cv2-admit`, `CV2_DB=1`, one run at a time.
mod common;

use axum::http::Method;
use common::{Answer, Seed, Who, key, project_a, seed_return, world};
use cortex_types::HandoffId;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn path(id: HandoffId, verb: &str) -> String {
    format!("/v2/handoffs/{id}/{verb}")
}

fn link(r: &str) -> Value {
    let sha: String = Sha256::digest(r.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    json!({ "kind": "link", "ref": r, "sha256": sha })
}

fn report() -> Value {
    json!({ "summary": "done", "evidence": [link("r")], "risks": "none", "tests": ["cargo test"] })
}

fn code(a: &Answer) -> (u16, Option<String>) {
    (
        a.status.as_u16(),
        a.json["type"].as_str().map(str::to_owned),
    )
}

async fn claimed(w: &common::World, creator: Who, holder: Who) -> HandoffId {
    let mut admin = w.admin().await;
    Seed::pending(creator)
        .claimed_by(holder)
        .insert(&mut admin)
        .await
}

async fn returned(w: &common::World, creator: Who, author: Who) -> HandoffId {
    let mut admin = w.admin().await;
    let mut s = Seed::pending(creator).claimed_by(author);
    s.state = "returned";
    let id = s.insert(&mut admin).await;
    seed_return(&mut admin, id, project_a(), 1, author, "report").await;
    id
}

/// A handoff in each of the five states, created by `creator`, addressed to role `dev`, held by W1 when held.
async fn in_state(w: &common::World, creator: Who, state: &'static str) -> HandoffId {
    match state {
        "pending" => {
            let mut admin = w.admin().await;
            Seed::pending(creator).insert(&mut admin).await
        }
        "claimed" => claimed(w, creator, Who::W1).await,
        "returned" => returned(w, creator, Who::W1).await,
        _ => {
            let mut admin = w.admin().await;
            let mut s = Seed::pending(creator).claimed_by(Who::W1);
            s.state = state;
            s.closed_at = Some("2026-11-02 00:00:00+00".to_owned());
            if state == "cancelled" {
                s.close_reason = Some("no longer needed".to_owned());
            }
            s.insert(&mut admin).await
        }
    }
}

async fn state_of(w: &common::World, id: HandoffId) -> String {
    let mut admin = w.admin().await;
    sqlx::query_scalar("SELECT state FROM handoffs.handoffs WHERE id = $1::uuid")
        .bind(id.to_string())
        .fetch_one(&mut admin)
        .await
        .expect("a state")
}

async fn post_with(w: &common::World, who: Who, p: &str, k: &str, body: Option<Value>) -> Answer {
    w.send(Method::POST, p, Some(who), Some(k), body).await
}

fn replayed(a: &Answer) -> bool {
    a.headers.contains_key("cortex-idempotent-replay")
}

// ---- HAND-14 ---------------------------------------------------------------------------------------------

/// HAND-14: no response of any handoff route carries `incarnation` or a lease field, and no request body
/// accepts one. The column is kept by the server (+1 on each claim) and read here from the table only.
#[tokio::test]
async fn no_response_carries_incarnation_and_no_request_accepts_one() {
    let w = world("p4_incarnation").await;
    let mut seen: Vec<String> = Vec::new();
    let created = w
        .post(
            Who::L,
            "/v2/handoffs",
            json!({ "kind": "task", "to": { "kind": "job_role", "ref": "dev" },
                    "summary": "s", "next_action": "n", "acceptance": "a" }),
        )
        .await;
    assert_eq!(created.status.as_u16(), 201, "{}", created.text);
    seen.push(created.text.clone());
    let id = HandoffId::parse(created.json["id"].as_str().expect("id")).expect("an id");
    for (who, verb, body) in [
        (Who::W1, "claim", None),
        (Who::W1, "release", Some(json!({}))),
        (Who::W1, "claim", None),
        (Who::W1, "return", Some(report())),
        (Who::L, "accept", Some(json!({}))),
    ] {
        let a = w
            .send(Method::POST, &path(id, verb), Some(who), Some(&key()), body)
            .await;
        assert_eq!(a.status.as_u16(), 200, "{verb}: {}", a.text);
        seen.push(a.text);
    }
    seen.push(w.get(Who::W1, &format!("/v2/handoffs/{id}")).await.text);
    seen.push(w.get(Who::W1, "/v2/handoffs?mine=true").await.text);
    seen.push(w.get(Who::L, "/v2/handoffs?state=all").await.text);
    for t in &seen {
        let lower = t.to_lowercase();
        assert!(
            !lower.contains("incarnation") && !lower.contains("lease"),
            "{t}"
        );
    }
    let mut admin = w.admin().await;
    let inc: i64 =
        sqlx::query_scalar("SELECT incarnation FROM handoffs.handoffs WHERE id = $1::uuid")
            .bind(id.to_string())
            .fetch_one(&mut admin)
            .await
            .expect("incarnation");
    assert_eq!(inc, 2, "two claims, +1 each, in the table only");

    // No request accepts one: step 0, 422 invalid_request, on every route that has a body.
    let held = claimed(&w, Who::L, Who::W1).await;
    let back = returned(&w, Who::L, Who::W1).await;
    let mut ret = report();
    ret["incarnation"] = json!(1);
    for (who, p, body) in [
        (
            Who::L,
            "/v2/handoffs".to_owned(),
            json!({ "kind": "task", "to": { "kind": "job_role", "ref": "dev" }, "summary": "s",
                    "next_action": "n", "acceptance": "a", "incarnation": 1 }),
        ),
        (Who::W1, path(held, "claim"), json!({ "incarnation": 1 })),
        (Who::W1, path(held, "release"), json!({ "incarnation": 1 })),
        (Who::W1, path(held, "return"), ret),
        (Who::L, path(back, "accept"), json!({ "incarnation": 1 })),
        (
            Who::L,
            path(back, "rework"),
            json!({ "notes": "n", "incarnation": 1 }),
        ),
        (
            Who::L,
            path(held, "cancel"),
            json!({ "reason": "r", "incarnation": 1 }),
        ),
    ] {
        let a = w.post(who, &p, body).await;
        assert_eq!(
            code(&a),
            (422, Some("invalid_request".into())),
            "{p}: {}",
            a.text
        );
    }
}

// ---- HAND-34 ---------------------------------------------------------------------------------------------

/// HAND-34: the 409 for a lost race is the fixed problem. It holds no text from the handoff's packet, none from
/// project B, no token, and no worker id (the addressee reads the holder with `show`, HAND-10; S3 defines no
/// holder member on this problem and this piece's problem details are fixed words, never a row).
#[tokio::test]
async fn the_lost_race_answers_a_fixed_problem_and_the_holder_is_read_with_show() {
    let w = world("p4_lostrace").await;
    let mut admin = w.admin().await;
    let packet = Seed::pending(Who::L)
        .summary("CANARY-PACKET-7d3f91")
        .claimed_by(Who::W1)
        .insert(&mut admin)
        .await;
    Seed::pending(Who::B1)
        .summary("CANARY-PROJECT-B-91ac04")
        .insert(&mut admin)
        .await;
    let lost = w.claim(Who::W2, packet).await;
    assert_eq!(code(&lost), (409, Some("already_claimed".into())));
    for needle in [
        "CANARY-PACKET-7d3f91",
        "CANARY-PROJECT-B-91ac04",
        &Who::W1.id().to_string(),
        &Who::W2.id().to_string(),
        &Who::B1.id().to_string(),
        "tok.",
        "Bearer",
    ] {
        assert!(!lost.text.contains(needle), "{needle} in {}", lost.text);
    }
    let shown = w.get(Who::W2, &format!("/v2/handoffs/{packet}")).await;
    assert_eq!(shown.json["holder"], Who::W1.id().to_string());
}

// ---- HAND-35 ---------------------------------------------------------------------------------------------

/// HAND-35: no silent expiry. A claim whose lease columns say it lapsed two days ago is still held, and the
/// holder's return succeeds. .001 has no lease expiry; this makes that visible.
#[tokio::test]
async fn a_claim_has_no_lease_expiry() {
    let w = world("p4_nolease").await;
    let id = claimed(&w, Who::L, Who::W1).await;
    let mut admin = w.admin().await;
    sqlx::query(
        "UPDATE handoffs.handoffs SET lease_ttl_s = 300, lease_renewed_at = platform.now() - interval '2 days' \
         WHERE id = $1::uuid",
    )
    .bind(id.to_string())
    .execute(&mut admin)
    .await
    .expect("age the lease");
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    assert_eq!(state_of(&w, id).await, "claimed");
    let shown = w.get(Who::W1, &format!("/v2/handoffs/{id}")).await;
    assert_eq!(shown.json["holder"], Who::W1.id().to_string());
    let back = w.post(Who::W1, &path(id, "return"), report()).await;
    assert_eq!(back.status.as_u16(), 200, "{}", back.text);
    assert_eq!(state_of(&w, id).await, "returned");
}

// ---- HAND-57, HAND-58 ------------------------------------------------------------------------------------

/// HAND-57: accept's effects, and nothing else changes.
#[tokio::test]
async fn accept_closes_the_handoff_and_changes_nothing_else() {
    let w = world("p4_accept").await;
    let id = returned(&w, Who::L, Who::W1).await;
    let before = (
        w.count("SELECT count(*) FROM handoffs.handoffs").await,
        w.count("SELECT count(*) FROM handoffs.returns").await,
        w.count("SELECT count(*) FROM handoffs.evidence").await,
    );
    let a = w.post(Who::L, &path(id, "accept"), json!({})).await;
    assert_eq!(
        a.json,
        json!({ "id": id.to_string(), "state": "accepted", "round": 1 }),
        "{}",
        a.text
    );
    let after = (
        w.count("SELECT count(*) FROM handoffs.handoffs").await,
        w.count("SELECT count(*) FROM handoffs.returns").await,
        w.count("SELECT count(*) FROM handoffs.evidence").await,
    );
    assert_eq!(
        before, after,
        "no table but reviews, transitions and audit grows"
    );
    let mut admin = w.admin().await;
    let (state, closed, holder, round): (String, bool, Option<String>, i32) = sqlx::query_as(
        "SELECT state, closed_at IS NOT NULL, holder::text, round FROM handoffs.handoffs WHERE id = $1::uuid",
    )
    .bind(id.to_string())
    .fetch_one(&mut admin)
    .await
    .expect("row");
    assert_eq!(
        (state.as_str(), closed, holder, round),
        ("accepted", true, Some(Who::W1.id().to_string()), 1)
    );
    let review: (String, String, bool, i32) = sqlx::query_as(
        "SELECT verdict, reviewer::text, self_review, round FROM handoffs.reviews WHERE handoff_id = $1::uuid",
    )
    .bind(id.to_string())
    .fetch_one(&mut admin)
    .await
    .expect("the one review");
    assert_eq!(review, ("accept".into(), Who::L.id().to_string(), false, 1));
    let t: (String, String, String) = sqlx::query_as(
        "SELECT from_state, to_state, actor_id FROM handoffs.transitions WHERE handoff_id = $1::uuid AND seq = 2",
    )
    .bind(id.to_string())
    .fetch_one(&mut admin)
    .await
    .expect("the log row");
    assert_eq!(
        t,
        (
            "returned".into(),
            "accepted".into(),
            Who::L.id().to_string()
        )
    );
    assert_eq!(
        w.count("SELECT count(*) FROM fx_audit.events WHERE action = 'handoff.accepted'")
            .await,
        1
    );
}

/// HAND-58: accept and rework on pending, claimed, accepted and cancelled are 409 `invalid_transition`; nothing moves.
#[tokio::test]
async fn accept_and_rework_refuse_every_state_but_returned() {
    let w = world("p4_review_states").await;
    for state in ["pending", "claimed", "accepted", "cancelled"] {
        let id = in_state(&w, Who::L, state).await;
        for (verb, body) in [("accept", json!({})), ("rework", json!({ "notes": "n" }))] {
            let a = w.post(Who::L, &path(id, verb), body).await;
            assert_eq!(
                code(&a),
                (409, Some("invalid_transition".into())),
                "{verb} on {state}: {}",
                a.text
            );
        }
        assert_eq!(state_of(&w, id).await, state);
    }
    assert_eq!(
        w.count("SELECT count(*) FROM handoffs.reviews").await,
        0,
        "no review row from a refusal"
    );
    assert_eq!(
        w.count("SELECT count(*) FROM handoffs.transitions WHERE seq > 1")
            .await,
        0
    );
    assert_eq!(w.count("SELECT count(*) FROM fx_audit.events").await, 0);
}

// ---- HAND-63, HAND-64 ------------------------------------------------------------------------------------

/// HAND-63: on a claim held by W1, the creator (a consult from W3, so not the lead), the addressee W2 and the
/// stranger W4 each get 409 `not_holder` from `release`. Only the holder and the project lead may release.
#[tokio::test]
async fn release_by_anyone_but_the_holder_or_the_lead_is_not_holder() {
    let w = world("p4_release_notholder").await;
    let mut admin = w.admin().await;
    let id = Seed::pending(Who::W3)
        .consult()
        .claimed_by(Who::W1)
        .insert(&mut admin)
        .await;
    for who in [Who::W3, Who::W2, Who::W4] {
        let a = w.post(who, &path(id, "release"), json!({})).await;
        assert_eq!(
            code(&a),
            (409, Some("not_holder".into())),
            "{who:?}: {}",
            a.text
        );
    }
    assert_eq!(state_of(&w, id).await, "claimed");
    assert_eq!(
        w.count("SELECT count(*) FROM handoffs.handoffs WHERE holder IS NOT NULL")
            .await,
        1
    );
    assert_eq!(
        w.count("SELECT count(*) FROM handoffs.transitions WHERE seq > 1")
            .await,
        0
    );
}

/// HAND-64: release on pending, returned, accepted and cancelled is 409 `invalid_transition` (v1 said 404).
#[tokio::test]
async fn release_refuses_every_state_but_claimed() {
    let w = world("p4_release_states").await;
    for state in ["pending", "returned", "accepted", "cancelled"] {
        let id = in_state(&w, Who::L, state).await;
        for who in [Who::L, Who::W1] {
            let a = w.post(who, &path(id, "release"), json!({})).await;
            assert_eq!(
                code(&a),
                (409, Some("invalid_transition".into())),
                "{who:?} on {state}: {}",
                a.text
            );
        }
        assert_eq!(state_of(&w, id).await, state);
    }
    assert_eq!(
        w.count("SELECT count(*) FROM handoffs.transitions WHERE seq > 1")
            .await,
        0
    );
}

// ---- HAND-82, HAND-85, HAND-86 ---------------------------------------------------------------------------

/// HAND-82: one key with a changed body is 422 `idempotency_key_reused`; so is one key on a different handoff,
/// for a caller who passes the target check of the new id; W4, who does not, learns nothing: 403.
#[tokio::test]
async fn a_key_reused_for_another_body_or_another_handoff_is_refused() {
    let w = world("p4_key_reused").await;
    let mut admin = w.admin().await;
    let h1 = Seed::pending(Who::L)
        .addressed_to_worker(Who::W1)
        .insert(&mut admin)
        .await;
    let h2 = Seed::pending(Who::L)
        .addressed_to_worker(Who::W1)
        .insert(&mut admin)
        .await;
    let h3 = Seed::pending(Who::L)
        .addressed_to_worker(Who::W1)
        .insert(&mut admin)
        .await;

    // A changed body: cancel h3 with reason "a", then the same key with reason "b".
    let k = key();
    let first = post_with(
        &w,
        Who::L,
        &path(h3, "cancel"),
        &k,
        Some(json!({ "reason": "a" })),
    )
    .await;
    assert_eq!(first.status.as_u16(), 200, "{}", first.text);
    let again = post_with(
        &w,
        Who::L,
        &path(h3, "cancel"),
        &k,
        Some(json!({ "reason": "a" })),
    )
    .await;
    assert!(
        replayed(&again) && again.json == first.json,
        "same body replays"
    );
    let changed = post_with(
        &w,
        Who::L,
        &path(h3, "cancel"),
        &k,
        Some(json!({ "reason": "b" })),
    )
    .await;
    assert_eq!(code(&changed), (422, Some("idempotency_key_reused".into())));
    let mut admin = w.admin().await;
    let reason: Option<String> =
        sqlx::query_scalar("SELECT close_reason FROM handoffs.handoffs WHERE id = $1::uuid")
            .bind(h3.to_string())
            .fetch_one(&mut admin)
            .await
            .expect("reason");
    assert_eq!(reason.as_deref(), Some("a"), "nothing happened");

    // A different handoff: claim h1 with K2, then h2 with K2.
    let k2 = key();
    let c1 = post_with(&w, Who::W1, &path(h1, "claim"), &k2, None).await;
    assert_eq!(c1.status.as_u16(), 200, "{}", c1.text);
    let c2 = post_with(&w, Who::W1, &path(h2, "claim"), &k2, None).await;
    assert_eq!(code(&c2), (422, Some("idempotency_key_reused".into())));
    assert_eq!(state_of(&w, h2).await, "pending");
    let c3 = post_with(&w, Who::W4, &path(h2, "claim"), &k2, None).await;
    assert_eq!(
        code(&c3),
        (403, Some("target_not_allowed".into())),
        "W4 learns nothing about the key"
    );
}

/// HAND-85: the same key string used by W1 and W2, and in projects A and B, is independent: no replay crosses
/// a principal or a project.
#[tokio::test]
async fn one_key_string_is_independent_across_principals_and_projects() {
    let w = world("p4_key_scope").await;
    let mut admin = w.admin().await;
    let a1 = Seed::pending(Who::L)
        .addressed_to_worker(Who::W1)
        .insert(&mut admin)
        .await;
    let a2 = Seed::pending(Who::L)
        .addressed_to_worker(Who::W2)
        .insert(&mut admin)
        .await;
    let b1 = Seed::pending(Who::B1)
        .addressed_to_worker(Who::B2)
        .insert(&mut admin)
        .await;
    let k = key();
    for (who, id) in [(Who::W1, a1), (Who::W2, a2), (Who::B2, b1)] {
        let a = post_with(&w, who, &path(id, "claim"), &k, None).await;
        assert_eq!(a.status.as_u16(), 200, "{who:?}: {}", a.text);
        assert!(!replayed(&a), "{who:?} executed, it did not replay");
    }
    assert_eq!(
        w.count("SELECT count(*) FROM handoffs.transitions WHERE to_state = 'claimed'")
            .await,
        3
    );
}

/// HAND-86: a refused call stores no reply and no reservation. The same key, once the call would succeed,
/// executes fresh; while it would still be refused it is refused again, with the same status.
#[tokio::test]
async fn a_refusal_stores_nothing_so_the_same_key_executes_once_it_would_succeed() {
    let w = world("p4_key_refusal").await;
    let held = claimed(&w, Who::L, Who::W1).await;
    let k = key();
    for _ in 0..2 {
        let a = post_with(&w, Who::W2, &path(held, "claim"), &k, None).await;
        assert_eq!(code(&a), (409, Some("already_claimed".into())));
        assert!(!replayed(&a), "a refusal is never replayed");
    }
    let mut admin = w.admin().await;
    let pending = Seed::pending(Who::L).insert(&mut admin).await;
    let k4 = key();
    for _ in 0..2 {
        let a = post_with(&w, Who::W4, &path(pending, "claim"), &k4, None).await;
        assert_eq!(code(&a), (403, Some("target_not_allowed".into())));
        assert!(!replayed(&a));
    }
    // W1 lets go; the same key now wins for W2.
    let rel = w.post(Who::W1, &path(held, "release"), json!({})).await;
    assert_eq!(rel.status.as_u16(), 200, "{}", rel.text);
    let won = post_with(&w, Who::W2, &path(held, "claim"), &k, None).await;
    assert_eq!(won.status.as_u16(), 200, "{}", won.text);
    assert!(!replayed(&won), "fresh, not a stored refusal");
    assert_eq!(state_of(&w, held).await, "claimed");
}

/// HAND-119 for `claim` (R-19): a replay never bypasses authorization. W1 claims a handoff addressed to role `dev`
/// with key K and K replays while W1 still holds the role; then W1 loses the role, and the same K is refused exactly
/// as a first call would be: 403 `target_not_allowed`, no stored body, no replay header. The claim itself stands.
/// (Found by the mutation check: with the address check removed from the first phase a plain claim was still 403,
/// because the second phase checks again, but this replay answered 200.)
#[tokio::test]
async fn a_claim_replay_is_refused_when_the_role_is_gone() {
    let w = world("p4_claim_replay_auth").await;
    let mut admin = w.admin().await;
    let id = Seed::pending(Who::L).insert(&mut admin).await;
    let k = key();
    let first = post_with(&w, Who::W1, &path(id, "claim"), &k, None).await;
    assert_eq!(first.status.as_u16(), 200, "{}", first.text);
    assert!(!replayed(&first));
    let before = post_with(&w, Who::W1, &path(id, "claim"), &k, None).await;
    assert!(
        replayed(&before) && before.json == first.json,
        "while W1 holds the role the same key replays: {}",
        before.text
    );

    sqlx::query("UPDATE plug_identity.workers SET job_roles = '{}'::text[] WHERE id = $1::uuid")
        .bind(Who::W1.id().to_string())
        .execute(&mut admin)
        .await
        .expect("W1 loses the role");
    let after = post_with(&w, Who::W1, &path(id, "claim"), &k, None).await;
    assert_eq!(
        code(&after),
        (403, Some("target_not_allowed".into())),
        "{}",
        after.text
    );
    assert!(!replayed(&after), "a refusal is never a replay");
    assert_eq!(state_of(&w, id).await, "claimed", "the claim stands");
}

/// HAND-119 (e): a key first used on a handoff of project B, then sent by an A caller on that id, is the same 404 as
/// an id that exists nowhere, with no hint that the key exists (the project check comes before the lookup, R-19).
#[tokio::test]
async fn a_key_used_in_another_project_is_a_plain_404() {
    let w = world("p4_key_foreign").await;
    let mut admin = w.admin().await;
    let b = Seed::pending(Who::B1)
        .addressed_to_worker(Who::B2)
        .insert(&mut admin)
        .await;
    let k = key();
    let first = post_with(&w, Who::B2, &path(b, "claim"), &k, None).await;
    assert_eq!(first.status.as_u16(), 200, "{}", first.text);

    let from_a = post_with(&w, Who::W1, &path(b, "claim"), &k, None).await;
    let nowhere = HandoffId::new(&cortex_types::SystemClock);
    let missing = post_with(&w, Who::W1, &path(nowhere, "claim"), &key(), None).await;
    assert_eq!(code(&from_a), (404, Some("not_found".into())));
    assert!(!replayed(&from_a), "no replay across projects");
    let strip = |a: &Answer| {
        let mut j = a.json.clone();
        j.as_object_mut().expect("a problem").remove("request_id");
        j
    };
    assert_eq!(
        strip(&from_a),
        strip(&missing),
        "the same body as an id that exists nowhere"
    );
}

/// HAND-119 (g): the success reply of every mutating route holds only fields the contract allows (`id`, `ok`,
/// `state`, `round`, `created_at`, `section_revision`); this piece's are `{id, state}` for create and
/// `{id, state, round}` for the six moves, whatever the handoff holds.
#[tokio::test]
async fn every_success_reply_holds_only_the_contracts_fields() {
    let w = world("p4_reply_shape").await;
    let allowed = [
        "id",
        "ok",
        "state",
        "round",
        "created_at",
        "section_revision",
    ];
    let keys = |a: &Answer| -> Vec<String> {
        let mut k: Vec<String> = a
            .json
            .as_object()
            .expect("an object")
            .keys()
            .cloned()
            .collect();
        k.sort();
        k
    };
    let created = w
        .post(
            Who::L,
            "/v2/handoffs",
            json!({ "kind": "task", "to": { "kind": "job_role", "ref": "dev" },
                    "summary": "CANARY-REPLY-SHAPE", "next_action": "n", "acceptance": "a",
                    "verify": "v", "context": "c", "files": ["f"] }),
        )
        .await;
    assert_eq!(created.status.as_u16(), 201, "{}", created.text);
    assert_eq!(keys(&created), ["id", "state"]);
    let id = HandoffId::parse(created.json["id"].as_str().expect("id")).expect("an id");
    let mut replies = Vec::new();
    for (who, verb, body) in [
        (Who::W1, "claim", None),
        (Who::W1, "release", Some(json!({}))),
        (Who::W1, "claim", None),
        (Who::W1, "return", Some(report())),
        (Who::L, "rework", Some(json!({ "notes": "again" }))),
        (Who::W1, "claim", None),
        (Who::W1, "return", Some(report())),
        (Who::L, "accept", Some(json!({}))),
    ] {
        let a = w
            .send(Method::POST, &path(id, verb), Some(who), Some(&key()), body)
            .await;
        assert_eq!(a.status.as_u16(), 200, "{verb}: {}", a.text);
        replies.push((verb, a));
    }
    let second = w
        .post(
            Who::L,
            "/v2/handoffs",
            json!({ "kind": "task", "to": { "kind": "job_role", "ref": "dev" },
                    "summary": "s", "next_action": "n", "acceptance": "a" }),
        )
        .await;
    let id2 = HandoffId::parse(second.json["id"].as_str().expect("id")).expect("an id");
    replies.push((
        "cancel",
        w.post(Who::L, &path(id2, "cancel"), json!({ "reason": "r" }))
            .await,
    ));
    for (verb, a) in &replies {
        assert_eq!(keys(a), ["id", "round", "state"], "{verb}: {}", a.text);
        assert!(
            keys(a).iter().all(|k| allowed.contains(&k.as_str())),
            "{verb}"
        );
        assert!(
            !a.text.contains("CANARY-REPLY-SHAPE"),
            "{verb}: no packet text"
        );
    }
}

// ---- HAND-111, HAND-112 ----------------------------------------------------------------------------------

/// HAND-111: one audit row per successful move, in the move's own transaction. With the audit sink broken, no
/// move survives (no row, no log entry, no stored reply); mended, the same key executes. A refusal adds none.
#[tokio::test]
async fn a_failing_audit_write_rolls_the_whole_move_back() {
    let w = world("p4_audit_atomic").await;
    let pending = {
        let mut admin = w.admin().await;
        Seed::pending(Who::L)
            .addressed_to_worker(Who::W1)
            .insert(&mut admin)
            .await
    };
    let held = claimed(&w, Who::L, Who::W1).await;
    let back = returned(&w, Who::L, Who::W1).await;
    let mut admin = w.admin().await;
    sqlx::query(
        "CREATE FUNCTION fx_audit.boom() RETURNS trigger LANGUAGE plpgsql AS \
         'BEGIN RAISE EXCEPTION ''audit sink down''; END'",
    )
    .execute(&mut admin)
    .await
    .expect("the function");
    sqlx::query(
        "CREATE TRIGGER boom BEFORE INSERT ON fx_audit.events FOR EACH ROW EXECUTE FUNCTION fx_audit.boom()",
    )
    .execute(&mut admin)
    .await
    .expect("the trigger");

    let k = key();
    let moves: Vec<(Who, String, Option<Value>)> = vec![
        (Who::W1, path(pending, "claim"), None),
        (Who::W1, path(held, "release"), Some(json!({}))),
        (Who::W1, path(held, "return"), Some(report())),
        (Who::L, path(back, "accept"), Some(json!({}))),
        (Who::L, path(back, "rework"), Some(json!({ "notes": "n" }))),
        (
            Who::L,
            path(pending, "cancel"),
            Some(json!({ "reason": "r" })),
        ),
    ];
    for (who, p, body) in &moves {
        let a = post_with(&w, *who, p, &k, body.clone()).await;
        assert!(a.status.is_server_error(), "{p}: {} {}", a.status, a.text);
    }
    let created = post_with(
        &w,
        Who::L,
        "/v2/handoffs",
        &k,
        Some(
            json!({ "kind": "task", "to": { "kind": "job_role", "ref": "dev" },
                     "summary": "s", "next_action": "n", "acceptance": "a" }),
        ),
    )
    .await;
    assert!(created.status.is_server_error(), "{}", created.text);
    assert_eq!(state_of(&w, pending).await, "pending");
    assert_eq!(state_of(&w, held).await, "claimed");
    assert_eq!(state_of(&w, back).await, "returned");
    assert_eq!(w.count("SELECT count(*) FROM handoffs.handoffs").await, 3);
    assert_eq!(
        w.count("SELECT count(*) FROM handoffs.transitions WHERE seq > 1")
            .await,
        0
    );
    assert_eq!(w.count("SELECT count(*) FROM handoffs.returns").await, 1);
    assert_eq!(w.count("SELECT count(*) FROM handoffs.reviews").await, 0);

    // Mended: the same key now executes (the failed tries stored no reply, R-11).
    sqlx::query("DROP TRIGGER boom ON fx_audit.events")
        .execute(&mut admin)
        .await
        .expect("mend");
    let ok = post_with(&w, Who::W1, &path(pending, "claim"), &k, None).await;
    assert_eq!(ok.status.as_u16(), 200, "{}", ok.text);
    assert!(!replayed(&ok));
    assert_eq!(
        w.count("SELECT count(*) FROM fx_audit.events").await,
        1,
        "one audit row for the one success"
    );
}

/// HAND-112: the audit rows hold the action code, the handoff id, the actor and the project, and the boolean
/// `self_review` on a self-review. No summary, acceptance, notes, evidence or reason text. Action names are the
/// seven this piece mints.
#[tokio::test]
async fn audit_rows_carry_codes_and_ids_only() {
    let w = world("p4_audit_content").await;
    let c = |what: &str| format!("CANARY-{what}-4f2a9c");
    let created = w
        .post(
            Who::L,
            "/v2/handoffs",
            json!({ "kind": "task", "to": { "kind": "worker", "ref": Who::W1.id().to_string() },
                    "summary": c("SUMMARY"), "next_action": c("NEXT"), "acceptance": c("ACCEPT"),
                    "verify": c("VERIFY"), "context": c("CONTEXT"), "files": [c("FILE")],
                    "blockers": [c("BLOCKER")], "decisions": [c("DECISION")] }),
        )
        .await;
    assert_eq!(created.status.as_u16(), 201, "{}", created.text);
    let id = HandoffId::parse(created.json["id"].as_str().expect("id")).expect("an id");
    let ret = json!({ "summary": c("RETURN"), "evidence": [link(&c("EVIDENCE"))],
                          "risks": c("RISKS"), "tests": [c("TEST")] });
    for (who, verb, body) in [
        (Who::W1, "claim", None),
        (Who::W1, "release", Some(json!({ "reason": c("RELEASE") }))),
        (Who::W1, "claim", None),
        (Who::W1, "return", Some(ret.clone())),
        (Who::L, "rework", Some(json!({ "notes": c("REWORK") }))),
        (Who::W1, "claim", None),
        (Who::W1, "return", Some(ret.clone())),
        (
            Who::L,
            "accept",
            Some(json!({ "notes": c("ACCEPT-NOTES") })),
        ),
    ] {
        let a = w
            .send(Method::POST, &path(id, verb), Some(who), Some(&key()), body)
            .await;
        assert_eq!(a.status.as_u16(), 200, "{verb}: {}", a.text);
    }
    let second = w
        .post(
            Who::L,
            "/v2/handoffs",
            json!({ "kind": "task", "to": { "kind": "job_role", "ref": "dev" }, "summary": c("S2"),
                    "next_action": "n", "acceptance": "a" }),
        )
        .await;
    let id2 = HandoffId::parse(second.json["id"].as_str().expect("id")).expect("an id");
    let cancel = w
        .post(
            Who::L,
            &path(id2, "cancel"),
            json!({ "reason": c("CANCEL") }),
        )
        .await;
    assert_eq!(cancel.status.as_u16(), 200, "{}", cancel.text);
    // A self-review: L creates, holds, returns and accepts its own handoff.
    let mut admin = w.admin().await;
    let mut own = Seed::pending(Who::L).claimed_by(Who::L);
    own.state = "returned";
    let own = own.insert(&mut admin).await;
    seed_return(&mut admin, own, project_a(), 1, Who::L, "mine").await;
    let sr = w.post(Who::L, &path(own, "accept"), json!({})).await;
    assert_eq!(sr.status.as_u16(), 200, "{}", sr.text);

    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT action, target::text, coalesce(details::text, '') FROM fx_audit.events ORDER BY action, target",
    )
    .fetch_all(&mut admin)
    .await
    .expect("audit rows");
    let names = [
        "handoff.created",
        "handoff.claimed",
        "handoff.released",
        "handoff.returned",
        "handoff.accepted",
        "handoff.reworked",
        "handoff.cancelled",
    ];
    // id: created + 8 moves = 9; id2: created + cancelled = 2; the self-review accept = 1.
    assert_eq!(rows.len(), 12, "{rows:?}");
    for (action, target, details) in &rows {
        assert!(names.contains(&action.as_str()), "{action}");
        let all = format!("{action} {target} {details}");
        assert!(!all.contains("CANARY"), "a canary in an audit row: {all}");
        assert!(
            target.contains(&id.to_string())
                || target.contains(&id2.to_string())
                || target.contains(&own.to_string()),
            "{target}"
        );
        let is_self_review = target.contains(&own.to_string()) && action == "handoff.accepted";
        assert_eq!(
            details.contains("self_review"),
            is_self_review,
            "{action} {target} {details}"
        );
    }
}
