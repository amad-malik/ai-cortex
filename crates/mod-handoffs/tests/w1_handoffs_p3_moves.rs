//! Wave 1, mod-handoffs P3 (row 2a86598d): `return`, `accept`, `rework`, `release` and `cancel` behind the real
//! chain, on a real database. Test definitions: `mvp/tests/mod-handoffs.md` r3.4, HAND-40 to HAND-49 (return),
//! HAND-50 to HAND-60 (review), HAND-61 to HAND-65 (release), HAND-70 to HAND-75 and HAND-117, HAND-118
//! (cancel and who sees it), HAND-56 (the full cycle), and the replay cells of HAND-119 that the chain at the
//! pin can answer. Handoffs are seeded as the superuser; the moves go through the routes.
//!
//! Run ONLY under `crates/cortex-testkit/supervise.sh` through `cv2-admit`, `CV2_DB=1`, one run at a time.
mod common;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use common::{Seed, Who, project_a, seed_return, world};
use cortex_types::HandoffId;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// One stored evidence row: kind, ref, inline bytes, sha256 (hex), size.
type EvidenceRow = (String, Option<String>, Option<Vec<u8>>, String, i32);

fn hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn inline(data: &[u8]) -> Value {
    json!({ "kind": "output", "inline_b64": B64.encode(data), "sha256": hex(data) })
}

fn link(r: &str) -> Value {
    json!({ "kind": "link", "ref": r, "sha256": hex(r.as_bytes()) })
}

fn report(evidence: Vec<Value>) -> Value {
    json!({ "summary": "done", "evidence": evidence, "risks": "none", "tests": ["cargo test"] })
}

fn path(id: HandoffId, verb: &str) -> String {
    format!("/v2/handoffs/{id}/{verb}")
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

async fn state_of(w: &common::World, id: HandoffId) -> String {
    let mut admin = w.admin().await;
    sqlx::query_scalar("SELECT state FROM handoffs.handoffs WHERE id = $1::uuid")
        .bind(id.to_string())
        .fetch_one(&mut admin)
        .await
        .expect("state")
}

// ---- return (HAND-40 to HAND-49) ----

/// HAND-40: a return stores the report and its evidence, moves the handoff, and logs and audits it once.
#[tokio::test]
async fn return_stores_the_report_and_its_evidence() {
    let w = world("p3_return").await;
    let id = claimed(&w, Who::L, Who::W1).await;
    let data = b"hello evidence";
    let a = w
        .post(
            Who::W1,
            &path(id, "return"),
            report(vec![inline(data), link("https://example.test/run/1")]),
        )
        .await;
    assert_eq!(a.status.as_u16(), 200, "{}", a.text);
    assert_eq!(
        a.json,
        json!({ "id": id.to_string(), "state": "returned", "round": 1 })
    );

    let mut admin = w.admin().await;
    let (state, holder): (String, Option<String>) =
        sqlx::query_as("SELECT state, holder::text FROM handoffs.handoffs WHERE id = $1::uuid")
            .bind(id.to_string())
            .fetch_one(&mut admin)
            .await
            .expect("row");
    assert_eq!(
        (state.as_str(), holder),
        ("returned", Some(Who::W1.id().to_string())),
        "the holder stays"
    );
    let (round, author, incarnation, risks): (i32, String, i64, String) =
        sqlx::query_as("SELECT round, author::text, incarnation, risks FROM handoffs.returns")
            .fetch_one(&mut admin)
            .await
            .expect("the one return");
    assert_eq!(
        (round, author, incarnation, risks.as_str()),
        (1, Who::W1.id().to_string(), 1, "none")
    );
    let stored: Vec<EvidenceRow> = sqlx::query_as(
        "SELECT kind, ref, inline, encode(sha256, 'hex'), size FROM handoffs.evidence ORDER BY id",
    )
    .fetch_all(&mut admin)
    .await
    .expect("evidence");
    assert_eq!(stored.len(), 2);
    assert_eq!(
        stored[0],
        (
            "output".to_owned(),
            None,
            Some(data.to_vec()),
            hex(data),
            data.len() as i32
        )
    );
    assert_eq!(stored[1].0, "link");
    assert_eq!(
        (stored[1].1.as_deref(), stored[1].2.as_deref(), stored[1].4),
        (Some("https://example.test/run/1"), None, 0)
    );
    let t: (i32, String, String, String) = sqlx::query_as(
        "SELECT seq, from_state, to_state, actor_id FROM handoffs.transitions WHERE to_state = 'returned'",
    )
    .fetch_one(&mut admin)
    .await
    .expect("the log row");
    assert_eq!(
        (t.0, t.1.as_str(), t.2.as_str(), t.3),
        (2, "claimed", "returned", Who::W1.id().to_string())
    );
    assert_eq!(
        w.count("SELECT count(*) FROM fx_audit.events WHERE action = 'handoff.returned'")
            .await,
        1
    );

    // The reviewer sees what was returned, with no inline bytes.
    let shown = w.get(Who::L, &format!("/v2/handoffs/{id}")).await;
    let lr = &shown.json["latest_return"];
    assert_eq!(lr["round"], 1);
    assert_eq!(lr["evidence"][0]["size"], data.len());
    assert_eq!(lr["evidence"][0]["sha256"], hex(data));
    assert!(!shown.text.contains("inline"));
}

/// HAND-45, HAND-46: only the holder returns, and only a claimed handoff.
#[tokio::test]
async fn return_refusals() {
    let w = world("p3_returnrefuse").await;
    let held = claimed(&w, Who::W3, Who::W1).await;
    let good = report(vec![link("r")]);
    for who in [Who::W2, Who::W3, Who::L, Who::W4] {
        let a = w.post(who, &path(held, "return"), good.clone()).await;
        assert_eq!(a.code(), (409, Some("not_holder")), "{who:?}: {}", a.text);
    }
    let mut admin = w.admin().await;
    let pending = Seed::pending(Who::L).insert(&mut admin).await;
    let again = returned(&w, Who::L, Who::W1).await;
    let mut done = Seed::pending(Who::L).claimed_by(Who::W1);
    done.state = "accepted";
    done.closed_at = Some("2026-11-02 00:00:00+00".to_owned());
    let done = done.insert(&mut admin).await;
    let mut cancelled = Seed::pending(Who::L);
    cancelled.state = "cancelled";
    cancelled.closed_at = Some("2026-11-02 00:00:00+00".to_owned());
    cancelled.close_reason = Some("no".to_owned());
    let cancelled = cancelled.insert(&mut admin).await;
    for id in [pending, again, done, cancelled] {
        let a = w.post(Who::W1, &path(id, "return"), good.clone()).await;
        assert_eq!(a.code(), (409, Some("invalid_transition")), "{}", a.text);
    }
    assert_eq!(
        w.count("SELECT count(*) FROM handoffs.returns WHERE summary = 'done'")
            .await,
        0
    );
    assert_eq!(w.count("SELECT count(*) FROM fx_audit.events").await, 0);
    assert_eq!(state_of(&w, held).await, "claimed");
}

/// HAND-41 to HAND-44 and HAND-103 (b): the body is checked before anything is read.
#[tokio::test]
async fn return_checks_the_body_first() {
    let w = world("p3_returnbody").await;
    let id = claimed(&w, Who::L, Who::W1).await;
    let code = |a: &common::Answer| {
        (
            a.status.as_u16(),
            a.json["type"].as_str().map(str::to_owned),
        )
    };
    let mut wrong = inline(b"abc");
    wrong["sha256"] = json!(hex(b"other"));
    let both =
        json!({ "kind": "link", "ref": "r", "inline_b64": B64.encode(b"x"), "sha256": hex(b"x") });
    for (name, body, want) in [
        (
            "no evidence",
            json!({ "summary": "s", "risks": "none" }),
            "evidence_required",
        ),
        ("empty evidence", report(vec![]), "evidence_required"),
        (
            "wrong digest",
            report(vec![wrong]),
            "evidence_hash_mismatch",
        ),
        (
            "both sources",
            report(vec![both]),
            "evidence_source_invalid",
        ),
        (
            "no risks",
            json!({ "summary": "s", "evidence": [link("r")] }),
            "risks_required",
        ),
        (
            "blank risks",
            json!({ "summary": "s", "evidence": [link("r")], "risks": "  " }),
            "risks_required",
        ),
    ] {
        let a = w.post(Who::W1, &path(id, "return"), body).await;
        assert_eq!(code(&a), (422, Some(want.to_owned())), "{name}: {}", a.text);
    }
    // Two faults: a stranger with no evidence gets the stateless 422, not the 409.
    let a = w.post(Who::W4, &path(id, "return"), report(vec![])).await;
    assert_eq!(code(&a), (422, Some("evidence_required".into())));
    assert_eq!(state_of(&w, id).await, "claimed");
    assert_eq!(w.count("SELECT count(*) FROM handoffs.returns").await, 0);
}

/// HAND-47: a fault in the middle of a return takes everything back; the same key then runs fresh (R-11).
#[tokio::test]
async fn return_is_atomic_and_a_failed_try_leaves_no_replay() {
    let w = world("p3_atomic").await;
    let id = claimed(&w, Who::L, Who::W1).await;
    let mut admin = w.admin().await;
    sqlx::raw_sql(
        "CREATE FUNCTION public.fx_fail() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'injected'; END $$; \
         CREATE TRIGGER fx_fail_log BEFORE INSERT ON handoffs.transitions FOR EACH ROW WHEN (NEW.to_state = 'returned') \
         EXECUTE FUNCTION public.fx_fail();",
    )
    .execute(&mut admin)
    .await
    .expect("the fault");
    let k = common::key();
    let body = report(vec![inline(b"abc"), link("r")]);
    let a = w
        .send(
            axum::http::Method::POST,
            &path(id, "return"),
            Some(Who::W1),
            Some(&k),
            Some(body.clone()),
        )
        .await;
    assert!(
        a.status.is_server_error(),
        "an error status, never 200: {}",
        a.text
    );
    assert_eq!(state_of(&w, id).await, "claimed");
    let returns: i64 = sqlx::query_scalar("SELECT count(*) FROM handoffs.returns")
        .fetch_one(&mut admin)
        .await
        .expect("count");
    let evidence: i64 = sqlx::query_scalar("SELECT count(*) FROM handoffs.evidence")
        .fetch_one(&mut admin)
        .await
        .expect("count");
    assert_eq!((returns, evidence), (0, 0), "nothing survives the fault");
    assert_eq!(
        w.count("SELECT count(*) FROM handoffs.transitions WHERE seq > 1")
            .await,
        0
    );
    assert_eq!(w.count("SELECT count(*) FROM fx_audit.events").await, 0);
    // Remove the fault: the same key now runs again, and succeeds.
    sqlx::raw_sql("DROP TRIGGER fx_fail_log ON handoffs.transitions")
        .execute(&mut admin)
        .await
        .expect("remove the fault");
    let b = w
        .send(
            axum::http::Method::POST,
            &path(id, "return"),
            Some(Who::W1),
            Some(&k),
            Some(body),
        )
        .await;
    assert_eq!(b.status.as_u16(), 200, "{}", b.text);
    assert!(
        !b.headers.contains_key("cortex-idempotent-replay"),
        "a fresh run, not a replay"
    );
    assert_eq!(state_of(&w, id).await, "returned");
}

// ---- review (HAND-50 to HAND-60) ----

/// HAND-50, HAND-51, HAND-52: who may review, with the right codes, and the self-review flag.
#[tokio::test]
async fn review_follows_the_reviewer_table() {
    let w = world("p3_review").await;
    let code = |a: &common::Answer| {
        (
            a.status.as_u16(),
            a.json["type"].as_str().map(str::to_owned),
        )
    };
    let nta = (403, Some("target_not_allowed".to_owned()));
    let barred = (403, Some("self_review_forbidden".to_owned()));
    // (name, creator, author, callers with the code each gets)
    #[allow(clippy::type_complexity)]
    let cases: Vec<(&str, Who, Who, Vec<(Who, Option<(u16, Option<String>)>)>)> = vec![
        (
            "a",
            Who::L,
            Who::W1,
            vec![
                (Who::L, None),
                (Who::W1, Some(nta.clone())),
                (Who::W2, Some(nta.clone())),
            ],
        ),
        (
            "b",
            Who::W3,
            Who::W1,
            vec![
                (Who::W3, None),
                (Who::L, None),
                (Who::W1, Some(nta.clone())),
            ],
        ),
        (
            "c",
            Who::W3,
            Who::W3,
            vec![(Who::L, None), (Who::W3, Some(barred.clone()))],
        ),
        (
            "d",
            Who::W3,
            Who::L,
            vec![(Who::W3, None), (Who::L, Some(barred.clone()))],
        ),
        ("e", Who::L, Who::L, vec![(Who::L, None)]),
        (
            "f",
            Who::TL,
            Who::W1,
            vec![
                (Who::TL, None),
                (Who::L, None),
                (Who::W1, Some(nta.clone())),
            ],
        ),
    ];
    for (name, creator, author, callers) in cases {
        for (caller, want) in callers {
            for verb in ["accept", "rework"] {
                let id = returned(&w, creator, author).await;
                let body = if verb == "rework" {
                    json!({ "notes": "change it" })
                } else {
                    json!({})
                };
                let a = w.post(caller, &path(id, verb), body).await;
                match &want {
                    None => assert!(
                        a.status.is_success(),
                        "case {name} / {verb} / {caller:?}: {}",
                        a.text
                    ),
                    Some(w2) => assert_eq!(
                        &code(&a),
                        w2,
                        "case {name} / {verb} / {caller:?}: {}",
                        a.text
                    ),
                }
                if want.is_some() {
                    assert_eq!(
                        state_of(&w, id).await,
                        "returned",
                        "a refusal changes nothing"
                    );
                }
            }
        }
    }
    // The self-review marker: true only for case e.
    let mut admin = w.admin().await;
    let flags: Vec<(String, bool)> = sqlx::query_as(
        "SELECT verdict, self_review FROM handoffs.reviews WHERE self_review ORDER BY verdict",
    )
    .fetch_all(&mut admin)
    .await
    .expect("flags");
    assert_eq!(
        flags,
        [("accept".to_owned(), true), ("rework".to_owned(), true)],
        "case e only"
    );
    let audit: i64 = w
        .count("SELECT count(*) FROM fx_audit.events WHERE details LIKE '%self_review%'")
        .await;
    assert_eq!(audit, 2, "the audit rows of the two self-reviews say so");
}

/// HAND-53, HAND-54: rework needs notes; it sends the handoff back to the same worker, in round 2.
#[tokio::test]
async fn rework_needs_notes_and_goes_back_to_the_same_worker() {
    let w = world("p3_rework").await;
    let id = returned(&w, Who::L, Who::W1).await;
    let code = |a: &common::Answer| {
        (
            a.status.as_u16(),
            a.json["type"].as_str().map(str::to_owned),
        )
    };
    for bad in [json!({}), json!({ "notes": "" }), json!({ "notes": "   " })] {
        let a = w.post(Who::L, &path(id, "rework"), bad).await;
        assert_eq!(code(&a), (422, Some("notes_required".into())), "{}", a.text);
    }
    assert_eq!(state_of(&w, id).await, "returned");
    let a = w
        .post(
            Who::L,
            &path(id, "rework"),
            json!({ "notes": "add a test" }),
        )
        .await;
    assert_eq!(a.status.as_u16(), 200, "{}", a.text);
    assert_eq!(
        a.json,
        json!({ "id": id.to_string(), "state": "pending", "round": 2 })
    );

    let mut admin = w.admin().await;
    let (state, holder, rework, round): (String, Option<String>, Option<String>, i32) = sqlx::query_as(
        "SELECT state, holder::text, rework_holder::text, round FROM handoffs.handoffs WHERE id = $1::uuid",
    )
    .bind(id.to_string())
    .fetch_one(&mut admin)
    .await
    .expect("row");
    assert_eq!(
        (state.as_str(), holder, rework, round),
        ("pending", None, Some(Who::W1.id().to_string()), 2)
    );
    // The original addressing is untouched; the worker who returned it is the only one who may claim.
    let shown = w.get(Who::W4, &format!("/v2/handoffs/{id}")).await;
    assert_eq!(
        shown.json["to"],
        json!({ "kind": "job_role", "ref": "dev" })
    );
    assert_eq!(shown.json["latest_review"]["decision"], "rework");
    assert_eq!(shown.json["latest_review"]["notes"], "add a test");
    assert_eq!(
        w.claim(Who::W2, id).await.code(),
        (403, Some("target_not_allowed"))
    );
    assert_eq!(w.claim(Who::W1, id).await.status.as_u16(), 200);
    assert_eq!(w.count("SELECT count(*) FROM handoffs.reviews").await, 1);
}

/// HAND-56: create, claim, return, rework, claim, return, accept: the whole loop, and what it leaves.
#[tokio::test]
async fn the_full_cycle_leaves_a_consistent_log() {
    let w = world("p3_cycle").await;
    let a = w
        .post(
            Who::L,
            "/v2/handoffs",
            json!({ "kind": "task", "to": { "kind": "job_role", "ref": "dev" }, "summary": "s", "next_action": "n", "acceptance": "a" }),
        )
        .await;
    assert_eq!(a.status.as_u16(), 201, "{}", a.text);
    let id: HandoffId = HandoffId::parse(a.json["id"].as_str().expect("id")).expect("an id");
    let step = |r: common::Answer, want: &str| {
        assert_eq!(r.status.as_u16(), 200, "{}", r.text);
        assert_eq!(r.json["state"], want, "{}", r.text);
    };
    step(w.claim(Who::W1, id).await, "claimed");
    step(
        w.post(Who::W1, &path(id, "return"), report(vec![link("r1")]))
            .await,
        "returned",
    );
    step(
        w.post(Who::L, &path(id, "rework"), json!({ "notes": "again" }))
            .await,
        "pending",
    );
    step(w.claim(Who::W1, id).await, "claimed");
    step(
        w.post(Who::W1, &path(id, "return"), report(vec![link("r2")]))
            .await,
        "returned",
    );
    step(
        w.post(Who::L, &path(id, "accept"), json!({})).await,
        "accepted",
    );

    let mut admin = w.admin().await;
    let log: Vec<(i32, Option<String>, String)> = sqlx::query_as(
        "SELECT seq, from_state, to_state FROM handoffs.transitions WHERE handoff_id = $1::uuid ORDER BY seq",
    )
    .bind(id.to_string())
    .fetch_all(&mut admin)
    .await
    .expect("log");
    let want: Vec<(i32, Option<String>, String)> = [
        (1, None, "pending"),
        (2, Some("pending"), "claimed"),
        (3, Some("claimed"), "returned"),
        (4, Some("returned"), "pending"),
        (5, Some("pending"), "claimed"),
        (6, Some("claimed"), "returned"),
        (7, Some("returned"), "accepted"),
    ]
    .iter()
    .map(|(s, f, t)| (*s, f.map(str::to_owned), (*t).to_owned()))
    .collect();
    assert_eq!(log, want, "seven transitions, a chain");
    assert_eq!(w.count("SELECT count(*) FROM handoffs.returns").await, 2);
    assert_eq!(w.count("SELECT count(*) FROM handoffs.reviews").await, 2);
    let actions: Vec<String> = sqlx::query_scalar("SELECT action FROM fx_audit.events ORDER BY id")
        .fetch_all(&mut admin)
        .await
        .expect("audit");
    assert_eq!(
        actions,
        [
            "handoff.created",
            "handoff.claimed",
            "handoff.returned",
            "handoff.reworked",
            "handoff.claimed",
            "handoff.returned",
            "handoff.accepted"
        ]
    );
    let shown = w.get(Who::W4, &format!("/v2/handoffs/{id}")).await;
    assert_eq!(shown.json["state"], "accepted");
    assert_eq!(shown.json["latest_return"]["round"], 2);
    assert_eq!(
        shown.json["closed_by"],
        json!({ "kind": "worker", "id": Who::L.id().to_string() })
    );
    assert!(shown.json["closed_at"].is_string());
    // Terminal: every move is refused.
    for verb in ["release", "return", "accept", "rework", "cancel"] {
        let body = match verb {
            "return" => report(vec![link("r")]),
            "rework" => json!({ "notes": "n" }),
            "cancel" => json!({ "reason": "r" }),
            _ => json!({}),
        };
        let a = w.post(Who::L, &path(id, verb), body).await;
        assert_eq!(
            a.code(),
            (409, Some("invalid_transition")),
            "{verb}: {}",
            a.text
        );
    }
}

// ---- release (HAND-61 to HAND-65) ----

#[tokio::test]
async fn release_by_the_holder_or_the_lead_and_a_release_clears_the_rework_lock() {
    let w = world("p3_release").await;
    let code = |a: &common::Answer| {
        (
            a.status.as_u16(),
            a.json["type"].as_str().map(str::to_owned),
        )
    };
    // The holder releases with `{}` and with a reason.
    for body in [json!({}), json!({ "reason": "moving on" })] {
        let id = claimed(&w, Who::L, Who::W1).await;
        let a = w.post(Who::W1, &path(id, "release"), body).await;
        assert_eq!(
            a.json,
            json!({ "id": id.to_string(), "state": "pending", "round": 1 }),
            "{}",
            a.text
        );
        assert_eq!(state_of(&w, id).await, "pending");
    }
    // The lead needs a reason; with one it may release another worker's claim.
    let id = claimed(&w, Who::L, Who::W1).await;
    for bad in [
        json!({}),
        json!({ "reason": "" }),
        json!({ "reason": "  " }),
    ] {
        let a = w.post(Who::L, &path(id, "release"), bad).await;
        assert_eq!(
            code(&a),
            (422, Some("reason_required".into())),
            "{}",
            a.text
        );
    }
    assert_eq!(
        w.post(
            Who::L,
            &path(id, "release"),
            json!({ "reason": "W1 is away" })
        )
        .await
        .status
        .as_u16(),
        200
    );
    let mut admin = w.admin().await;
    let reason: Option<String> = sqlx::query_scalar(
        "SELECT reason FROM handoffs.transitions WHERE handoff_id = $1::uuid AND to_state = 'pending' AND seq = 2",
    )
    .bind(id.to_string())
    .fetch_one(&mut admin)
    .await
    .expect("the log row");
    assert_eq!(reason.as_deref(), Some("W1 is away"));
    // Nobody else may release; the wrong states are 409.
    let held = claimed(&w, Who::W3, Who::W1).await;
    for who in [Who::W2, Who::W3, Who::W4] {
        let a = w
            .post(who, &path(held, "release"), json!({ "reason": "x" }))
            .await;
        assert_eq!(code(&a), (409, Some("not_holder".into())), "{who:?}");
    }
    let pending = Seed::pending(Who::L).insert(&mut admin).await;
    assert_eq!(
        code(&w.post(Who::L, &path(pending, "release"), json!({})).await),
        (409, Some("invalid_transition".into()))
    );
    assert_eq!(
        code(
            &w.post(Who::W1, &path(pending, "release"), json!({ "note": "n" }))
                .await
        ),
        (422, Some("invalid_request".into()))
    );

    // Q-17: a release after a rework clears the rework lock, so the original address applies again.
    let rework = returned(&w, Who::L, Who::W1).await;
    assert_eq!(
        w.post(Who::L, &path(rework, "rework"), json!({ "notes": "again" }))
            .await
            .status
            .as_u16(),
        200
    );
    assert_eq!(w.claim(Who::W1, rework).await.status.as_u16(), 200);
    assert_eq!(
        w.post(Who::W1, &path(rework, "release"), json!({}))
            .await
            .status
            .as_u16(),
        200
    );
    let lock: Option<String> =
        sqlx::query_scalar("SELECT rework_holder::text FROM handoffs.handoffs WHERE id = $1::uuid")
            .bind(rework.to_string())
            .fetch_one(&mut admin)
            .await
            .expect("lock");
    assert_eq!(lock, None, "the release cleared it");
    assert_eq!(
        w.claim(Who::W2, rework).await.status.as_u16(),
        200,
        "W2 holds the role `dev` and may claim again"
    );
}

// ---- cancel (HAND-70 to HAND-75, HAND-117, HAND-118) ----

/// HAND-70, HAND-117: who may cancel, from which open state.
#[tokio::test]
async fn cancel_is_for_the_creator_the_lead_and_the_team_lead_concerned() {
    let w = world("p3_cancel").await;
    let code = |a: &common::Answer| {
        (
            a.status.as_u16(),
            a.json["type"].as_str().map(str::to_owned),
        )
    };
    let nta = (403, Some("target_not_allowed".to_owned()));
    for state in ["pending", "claimed", "returned"] {
        let make = |creator: Who, to: Who| {
            let w = &w;
            async move {
                let mut admin = w.admin().await;
                let mut s = Seed::pending(creator).addressed_to_worker(to);
                if state != "pending" {
                    s = s.claimed_by(to);
                    s.state = if state == "claimed" {
                        "claimed"
                    } else {
                        "returned"
                    };
                }
                s.insert(&mut admin).await
            }
        };
        // A task L created for W1 (team T): the creator, the lead and TL (the team lead concerned) may cancel.
        for (who, ok) in [
            (Who::L, true),
            (Who::TL, true),
            (Who::UL, false),
            (Who::W2, false),
            (Who::W1, false),
            (Who::W4, false),
        ] {
            let id = make(Who::L, Who::W1).await;
            let a = w
                .post(
                    who,
                    &path(id, "cancel"),
                    json!({ "reason": "no longer needed" }),
                )
                .await;
            if ok {
                assert_eq!(a.status.as_u16(), 200, "{state} / {who:?}: {}", a.text);
                assert_eq!(state_of(&w, id).await, "cancelled");
            } else {
                assert_eq!(code(&a), nta, "{state} / {who:?}: {}", a.text);
                assert_eq!(state_of(&w, id).await, state);
            }
        }
        // A task addressed to a job role: a team lead who did not create it may not; the creator may.
        let mut admin = w.admin().await;
        let role_task = Seed::pending(Who::L).insert(&mut admin).await;
        assert_eq!(
            code(
                &w.post(
                    Who::TL,
                    &path(role_task, "cancel"),
                    json!({ "reason": "r" })
                )
                .await
            ),
            nta
        );
        assert_eq!(
            code(
                &w.post(
                    Who::UL,
                    &path(role_task, "cancel"),
                    json!({ "reason": "r" })
                )
                .await
            ),
            nta
        );
    }
}

/// HAND-71, HAND-72, HAND-73, HAND-75: the reason, the effects, and the terminal states.
#[tokio::test]
async fn cancel_needs_a_reason_records_who_and_why_and_is_terminal() {
    let w = world("p3_cancel2").await;
    let code = |a: &common::Answer| {
        (
            a.status.as_u16(),
            a.json["type"].as_str().map(str::to_owned),
        )
    };
    let id = claimed(&w, Who::L, Who::W1).await;
    for bad in [
        json!({}),
        json!({ "reason": "" }),
        json!({ "reason": "  " }),
    ] {
        assert_eq!(
            code(&w.post(Who::L, &path(id, "cancel"), bad).await),
            (422, Some("reason_required".into()))
        );
    }
    for bad in [
        json!({ "reason": "x".repeat(513) }),
        json!({ "reason": "é".repeat(300) }),
    ] {
        assert_eq!(
            code(&w.post(Who::L, &path(id, "cancel"), bad).await),
            (422, Some("invalid_request".into()))
        );
    }
    // Two faults: no reason on an unknown id and on another project's id is the stateless 422, not 404.
    let missing = HandoffId::new(&cortex_types::SystemClock);
    assert_eq!(
        code(&w.post(Who::L, &path(missing, "cancel"), json!({})).await),
        (422, Some("reason_required".into()))
    );
    assert_eq!(state_of(&w, id).await, "claimed");

    let a = w
        .post(
            Who::L,
            &path(id, "cancel"),
            json!({ "reason": "x".repeat(512) }),
        )
        .await;
    assert_eq!(
        a.json,
        json!({ "id": id.to_string(), "state": "cancelled", "round": 1 }),
        "{}",
        a.text
    );
    let mut admin = w.admin().await;
    let (closed, reason, holder): (bool, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT closed_at IS NOT NULL, close_reason, holder::text FROM handoffs.handoffs WHERE id = $1::uuid",
    )
    .bind(id.to_string())
    .fetch_one(&mut admin)
    .await
    .expect("row");
    assert_eq!(
        (closed, reason.map(|r| r.len()), holder),
        (true, Some(512), Some(Who::W1.id().to_string()))
    );
    // The seed holds one creation row (seq 1) and `transition_seq = 1`, so the cancel is the next number: 2.
    let t: (String, String, String, Option<String>) = sqlx::query_as(
        "SELECT from_state, to_state, actor_id, reason FROM handoffs.transitions WHERE handoff_id = $1::uuid AND seq = 2",
    )
    .bind(id.to_string())
    .fetch_one(&mut admin)
    .await
    .expect("the log row");
    assert_eq!(
        (t.0.as_str(), t.1.as_str(), t.2),
        ("claimed", "cancelled", Who::L.id().to_string())
    );
    assert_eq!(t.3.map(|r| r.len()), Some(512));
    assert_eq!(
        w.count("SELECT count(*) FROM fx_audit.events WHERE action = 'handoff.cancelled'")
            .await,
        1
    );
    // A claim is answered by the state only for someone it is addressed to: the seed's address is job role
    // `dev`, which W1 holds and the lead does not (a non-addressee is 403 first, R-16), so W1 claims.
    for (who, verb, body) in [
        (Who::W1, "claim", json!({})),
        (Who::L, "release", json!({})),
        (Who::L, "return", report(vec![link("r")])),
        (Who::L, "accept", json!({})),
        (Who::L, "rework", json!({ "notes": "n" })),
        (Who::L, "cancel", json!({ "reason": "again" })),
    ] {
        let a = w.post(who, &path(id, verb), body).await;
        assert_eq!(
            code(&a),
            (409, Some("invalid_transition".into())),
            "{verb}: {}",
            a.text
        );
    }
    let mut accepted = Seed::pending(Who::L).claimed_by(Who::W1);
    accepted.state = "accepted";
    accepted.closed_at = Some("2026-11-02 00:00:00+00".to_owned());
    let accepted = accepted.insert(&mut admin).await;
    assert_eq!(
        code(
            &w.post(Who::L, &path(accepted, "cancel"), json!({ "reason": "r" }))
                .await
        ),
        (409, Some("invalid_transition".into()))
    );
}

/// HAND-118: the creator sees a cancellation by someone else for 7 days; not one it made; `show` always has it.
#[tokio::test]
async fn the_creator_sees_a_cancellation_by_someone_else_for_seven_days() {
    let w = world("p3_notice").await;
    let ids = |a: &common::Answer| -> Vec<String> {
        a.json["items"]
            .as_array()
            .expect("items")
            .iter()
            .map(|h| h["id"].as_str().expect("id").to_owned())
            .collect()
    };
    let by_tl = {
        let mut admin = w.admin().await;
        Seed::pending(Who::L)
            .addressed_to_worker(Who::W1)
            .insert(&mut admin)
            .await
    };
    assert_eq!(
        w.post(
            Who::TL,
            &path(by_tl, "cancel"),
            json!({ "reason": "wrong team" })
        )
        .await
        .status
        .as_u16(),
        200
    );
    let by_self = {
        let mut admin = w.admin().await;
        Seed::pending(Who::L).insert(&mut admin).await
    };
    assert_eq!(
        w.post(
            Who::L,
            &path(by_self, "cancel"),
            json!({ "reason": "mine" })
        )
        .await
        .status
        .as_u16(),
        200
    );

    let mine = w.get(Who::L, "/v2/handoffs?mine=true").await;
    assert_eq!(
        ids(&mine),
        [by_tl.to_string()],
        "L sees TL's cancellation, not its own"
    );
    let item = &mine.json["items"][0];
    assert_eq!(item["state"], "cancelled");
    assert_eq!(
        item["closed_by"],
        json!({ "kind": "worker", "id": Who::TL.id().to_string() })
    );
    assert_eq!(item["close_reason"], "wrong team");
    for other in [Who::TL, Who::W1, Who::W4] {
        assert!(
            ids(&w.get(other, "/v2/handoffs?mine=true").await).is_empty(),
            "{other:?} does not see it"
        );
    }
    assert_eq!(
        ids(&w
            .get(Who::L, "/v2/handoffs?mine=true&state=cancelled")
            .await),
        [by_tl.to_string()]
    );
    assert!(ids(&w.get(Who::L, "/v2/handoffs?mine=true&state=pending").await).is_empty());

    // At closed_at + 7 days - 1 s it is still listed; at + 1 s it has left `mine`, but `show` has it.
    let mut admin = w.admin().await;
    for (age, listed) in [("6 days 23:59:59", true), ("7 days 00:00:01", false)] {
        sqlx::query(
            "UPDATE handoffs.handoffs SET closed_at = platform.now() - $2::interval WHERE id = $1::uuid",
        )
        .bind(by_tl.to_string())
        .bind(age)
        .execute(&mut admin)
        .await
        .expect("move closed_at");
        let got = ids(&w.get(Who::L, "/v2/handoffs?mine=true").await).contains(&by_tl.to_string());
        assert_eq!(got, listed, "closed {age} ago");
    }
    let shown = w.get(Who::L, &format!("/v2/handoffs/{by_tl}")).await;
    assert_eq!(shown.json["closed_by"]["id"], Who::TL.id().to_string());
}

/// HAND-117 (h): a team lead who is the ADDRESSEE of a task someone else created may cancel it.
#[tokio::test]
async fn a_team_lead_may_cancel_a_task_addressed_to_itself_a_plain_addressee_may_not() {
    let w = world("p3_cancel_self").await;
    let mut admin = w.admin().await;
    let to_tl = Seed::pending(Who::L)
        .addressed_to_worker(Who::TL)
        .claimed_by(Who::TL)
        .insert(&mut admin)
        .await;
    let to_w1 = Seed::pending(Who::L)
        .addressed_to_worker(Who::W1)
        .claimed_by(Who::W1)
        .insert(&mut admin)
        .await;
    assert_eq!(
        w.post(Who::TL, &path(to_tl, "cancel"), json!({ "reason": "r" }))
            .await
            .status
            .as_u16(),
        200
    );
    let a = w
        .post(Who::W1, &path(to_w1, "cancel"), json!({ "reason": "r" }))
        .await;
    assert_eq!(a.code(), (403, Some("target_not_allowed")));
}

// ---- replay (HAND-81, HAND-83, HAND-119) ----

/// HAND-81, HAND-83: a repeated key replays the stored reply; a new key sees the new state.
#[tokio::test]
async fn a_repeated_key_replays_and_a_new_key_sees_the_new_state() {
    let w = world("p3_replay").await;
    let mut admin = w.admin().await;
    let id = Seed::pending(Who::L).insert(&mut admin).await;
    let k = common::key();
    let send = |k: String| {
        let w = &w;
        async move {
            w.send(
                axum::http::Method::POST,
                &path(id, "claim"),
                Some(Who::W1),
                Some(&k),
                None,
            )
            .await
        }
    };
    let first = send(k.clone()).await;
    let second = send(k.clone()).await;
    assert_eq!((first.status.as_u16(), second.status.as_u16()), (200, 200));
    assert_eq!(first.json, second.json);
    assert!(
        second.headers.contains_key("cortex-idempotent-replay")
            && !first.headers.contains_key("cortex-idempotent-replay")
    );
    assert_eq!(
        w.count("SELECT count(*) FROM handoffs.transitions WHERE to_state = 'claimed'")
            .await,
        1,
        "once"
    );
    // The handoff moves on; the same key still replays; a new key is refused by the new state.
    assert_eq!(
        w.post(Who::W1, &path(id, "return"), report(vec![link("r")]))
            .await
            .status
            .as_u16(),
        200
    );
    assert_eq!(
        send(k).await.status.as_u16(),
        200,
        "the stored reply, though the handoff is returned"
    );
    assert_eq!(
        send(common::key()).await.code(),
        (409, Some("invalid_transition"))
    );
}

/// HAND-119 (a), (b), (f): a replay never bypasses authorization (R-19). A team lead cancels, then is moved to
/// another team (token and scope unchanged): the same key is refused exactly as a first call would be, with no
/// stored body and no replay header; a retry before the move is the stored 200.
#[tokio::test]
async fn a_replay_is_refused_when_the_target_rule_no_longer_holds() {
    let w = world("p3_replay_auth").await;
    let mut admin = w.admin().await;
    let id = Seed::pending(Who::L)
        .addressed_to_worker(Who::W1)
        .insert(&mut admin)
        .await;
    let k = common::key();
    let cancel = |k: String| {
        let w = &w;
        async move {
            w.send(
                axum::http::Method::POST,
                &path(id, "cancel"),
                Some(Who::TL),
                Some(&k),
                Some(json!({ "reason": "r" })),
            )
            .await
        }
    };
    let first = cancel(k.clone()).await;
    assert_eq!(first.status.as_u16(), 200, "{}", first.text);
    let before = cancel(k.clone()).await;
    assert_eq!(before.status.as_u16(), 200);
    assert!(
        before.headers.contains_key("cortex-idempotent-replay"),
        "before the move: the stored 200"
    );
    w.set_role(Who::TL, cortex_module::Role::TeamLead(common::team_u()));
    let after = cancel(k).await;
    assert_eq!(
        after.code(),
        (403, Some("target_not_allowed")),
        "{}",
        after.text
    );
    assert!(
        !after.headers.contains_key("cortex-idempotent-replay"),
        "no stored body for a refused caller"
    );
    assert_eq!(
        cancel(common::key()).await.code(),
        (403, Some("target_not_allowed")),
        "a new key is refused too"
    );
    assert_eq!(
        w.count("SELECT count(*) FROM handoffs.transitions WHERE to_state = 'cancelled'")
            .await,
        1,
        "cancelled once"
    );
}
