//! Wave 1, mod-handoffs P4e (row 2a86598d): what each route asks the policy, observed with a spy `ScopePolicy`
//! behind the real chain. `mvp/tests/mod-handoffs.md` r3.4: HAND-116 (each route passes its declared target rule,
//! R-17, and ignores nothing the spy says) and HAND-121 (c), (d), (e) (the consult flag comes only from the
//! top-level `kind`; hale verifies it). HAND-121 (a) and (b) are in `w1_handoffs_p2_create.rs`.
//!
//! The spy behaves as the policy double in `tests/common` (sophi's table 3.2) and records every
//! `target(principal, rule, T)` call; it can answer `false` once. Run ONLY under
//! `crates/cortex-testkit/supervise.sh` through `cv2-admit`, `CV2_DB=1`, one run at a time.
mod common;

use axum::http::Method;
use common::{Answer, Seed, Spy, TargetCall, Who, key, project_a, seed_return, team_t, world_with};
use cortex_module::{Target, TargetRule, WorkerKind};
use cortex_types::HandoffId;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn path(id: HandoffId, verb: &str) -> String {
    format!("/v2/handoffs/{id}/{verb}")
}

fn task(to: Value) -> Value {
    json!({ "kind": "task", "to": to, "summary": "s", "next_action": "n", "acceptance": "a" })
}

fn consult(to: Value) -> Value {
    json!({ "kind": "consult", "to": to, "summary": "s", "next_action": "n", "acceptance": "a" })
}

fn role(r: &str) -> Value {
    json!({ "kind": "job_role", "ref": r })
}

fn worker(w: Who) -> Value {
    json!({ "kind": "worker", "ref": w.id().to_string() })
}

fn report() -> Value {
    let sha: String = Sha256::digest(b"r")
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    json!({ "summary": "done", "risks": "none", "tests": ["t"],
            "evidence": [{ "kind": "link", "ref": "r", "sha256": sha }] })
}

fn code(a: &Answer) -> (u16, Option<String>) {
    (
        a.status.as_u16(),
        a.json["type"].as_str().map(str::to_owned),
    )
}

/// The worker row a rule is asked about: the worker and its kind.
fn worker_target(w: Who) -> Target {
    Target {
        worker: Some(w.id()),
        kind: Some(WorkerKind::Worker),
        ..Target::default()
    }
}

fn call(who: Who, rule: TargetRule, t: Target) -> TargetCall {
    (who.id(), rule, t)
}

/// A route whose declared rule is `None` asks nothing, or asks `None` about an empty target.
fn asked_nothing_that_matters(calls: &[TargetCall]) -> bool {
    calls
        .iter()
        .all(|(_, rule, t)| *rule == TargetRule::None && *t == Target::default())
}

/// Every handoff's id, state and holder, in one string: if a refused call moved anything this changes.
async fn states(w: &common::World) -> String {
    let mut c = w.admin().await;
    sqlx::query_scalar::<_, String>(
        "SELECT string_agg(id::text || state || coalesce(holder::text, ''), ',' ORDER BY id) FROM handoffs.handoffs",
    )
    .fetch_one(&mut c)
    .await
    .expect("states")
}

async fn send(w: &common::World, who: Who, verb_path: &str, body: Option<Value>) -> Answer {
    w.send(Method::POST, verb_path, Some(who), Some(&key()), body)
        .await
}

// ---- HAND-116 --------------------------------------------------------------------------------------------

/// HAND-116: every route, run once and allowed, makes exactly the policy calls its declared rule says, no more.
#[tokio::test]
async fn each_route_asks_the_policy_exactly_its_declared_question() {
    let spy = Spy::new();
    let w = world_with("p4e_spy", spy.clone()).await;
    let mut admin = w.admin().await;

    // create: a task to a worker, a task to a job role, a consult (to a worker and to a role).
    spy.take();
    let a = send(&w, Who::L, "/v2/handoffs", Some(task(worker(Who::W1)))).await;
    assert_eq!(a.status.as_u16(), 201, "{}", a.text);
    assert_eq!(
        spy.take(),
        vec![call(
            Who::L,
            TargetRule::TaskAddressee,
            Target {
                worker: Some(Who::W1.id()),
                kind: Some(WorkerKind::Worker),
                team: Some(team_t()),
                lead: Some(Who::L.id()),
                consult: false,
            }
        )],
        "create, a task to a worker"
    );
    let a = send(&w, Who::L, "/v2/handoffs", Some(task(role("dev")))).await;
    assert_eq!(a.status.as_u16(), 201, "{}", a.text);
    assert_eq!(
        spy.take(),
        vec![call(
            Who::L,
            TargetRule::TaskAddressee,
            Target {
                lead: Some(Who::L.id()),
                ..Target::default()
            }
        )],
        "create, a task to a job role: everything absent but the lead"
    );
    for (name, to) in [("to a worker", worker(Who::W2)), ("to a role", role("dev"))] {
        let a = send(&w, Who::W1, "/v2/handoffs", Some(consult(to))).await;
        assert_eq!(a.status.as_u16(), 201, "{name}: {}", a.text);
        assert_eq!(
            spy.take(),
            vec![call(
                Who::W1,
                TargetRule::TaskAddressee,
                Target {
                    consult: true,
                    ..Target::default()
                }
            )],
            "create, a consult {name}: the flag and every other field at its default (W1-9)"
        );
    }

    // claim: worker-addressed, job-role, and a round-2 handoff after a rework.
    let to_w1 = Seed::pending(Who::L)
        .addressed_to_worker(Who::W1)
        .insert(&mut admin)
        .await;
    let a = w.claim(Who::W1, to_w1).await;
    assert_eq!(a.status.as_u16(), 200, "{}", a.text);
    assert_eq!(
        spy.take(),
        vec![call(
            Who::W1,
            TargetRule::SelfWorker,
            worker_target(Who::W1)
        )],
        "claim, worker-addressed: T.worker is the addressee"
    );
    let to_role = Seed::pending(Who::L).insert(&mut admin).await;
    let a = w.claim(Who::W2, to_role).await;
    assert_eq!(a.status.as_u16(), 200, "{}", a.text);
    assert_eq!(
        spy.take(),
        vec![call(
            Who::W2,
            TargetRule::SelfWorker,
            worker_target(Who::W2)
        )],
        "claim, job-role: SelfWorker about the caller, asked only after the handler matched the caller's own role"
    );
    let mut round2 = Seed::pending(Who::L).addressed_to_worker(Who::W3);
    round2.rework_holder = Some(Who::W1);
    round2.round = 2;
    let round2 = round2.insert(&mut admin).await;
    let a = w.claim(Who::W1, round2).await;
    assert_eq!(a.status.as_u16(), 200, "{}", a.text);
    assert_eq!(
        spy.take(),
        vec![call(
            Who::W1,
            TargetRule::SelfWorker,
            worker_target(Who::W1)
        )],
        "claim after a rework: T.worker is the rework_holder, not the original address"
    );

    // accept and rework: SelfOrProjectLead about the creator.
    for verb in ["accept", "rework"] {
        let mut s = Seed::pending(Who::TL).claimed_by(Who::W1);
        s.state = "returned";
        let id = s.insert(&mut admin).await;
        seed_return(&mut admin, id, project_a(), 1, Who::W1, "report").await;
        let body = if verb == "accept" {
            json!({})
        } else {
            json!({ "notes": "again" })
        };
        let a = send(&w, Who::L, &path(id, verb), Some(body)).await;
        assert_eq!(a.status.as_u16(), 200, "{verb}: {}", a.text);
        assert_eq!(
            spy.take(),
            vec![call(
                Who::L,
                TargetRule::SelfOrProjectLead,
                worker_target(Who::TL)
            )],
            "{verb}: T.worker is the creator"
        );
    }

    // cancel: Lineage about the creator, the worker addressee's team (none for a job role) and the lead.
    let worker_task = Seed::pending(Who::TL)
        .addressed_to_worker(Who::W1)
        .insert(&mut admin)
        .await;
    let a = send(
        &w,
        Who::L,
        &path(worker_task, "cancel"),
        Some(json!({ "reason": "r" })),
    )
    .await;
    assert_eq!(a.status.as_u16(), 200, "{}", a.text);
    assert_eq!(
        spy.take(),
        vec![call(
            Who::L,
            TargetRule::Lineage,
            Target {
                team: Some(team_t()),
                lead: Some(Who::L.id()),
                ..worker_target(Who::TL)
            }
        )],
        "cancel, a worker addressee: the creator, its team and the lead"
    );
    let role_task = Seed::pending(Who::TL).insert(&mut admin).await;
    let a = send(
        &w,
        Who::L,
        &path(role_task, "cancel"),
        Some(json!({ "reason": "r" })),
    )
    .await;
    assert_eq!(a.status.as_u16(), 200, "{}", a.text);
    assert_eq!(
        spy.take(),
        vec![call(
            Who::L,
            TargetRule::Lineage,
            Target {
                team: None,
                lead: Some(Who::L.id()),
                ..worker_target(Who::TL)
            }
        )],
        "cancel, a job-role addressee: no team"
    );

    // release, return, show, list: rule None, so no call or only an empty `None`.
    let held = Seed::pending(Who::L)
        .claimed_by(Who::W1)
        .insert(&mut admin)
        .await;
    spy.take();
    let a = send(&w, Who::W1, &path(held, "release"), Some(json!({}))).await;
    assert_eq!(a.status.as_u16(), 200, "{}", a.text);
    assert!(asked_nothing_that_matters(&spy.take()), "release");
    let a = w.claim(Who::W1, held).await;
    assert_eq!(a.status.as_u16(), 200, "{}", a.text);
    spy.take();
    let a = send(&w, Who::W1, &path(held, "return"), Some(report())).await;
    assert_eq!(a.status.as_u16(), 200, "{}", a.text);
    assert!(asked_nothing_that_matters(&spy.take()), "return");
    for p in [format!("/v2/handoffs/{held}"), "/v2/handoffs".to_owned()] {
        let a = w.get(Who::W1, &p).await;
        assert_eq!(a.status.as_u16(), 200, "{p}: {}", a.text);
        assert!(asked_nothing_that_matters(&spy.take()), "{p}");
    }
}

/// HAND-116, the second half: a route that ignores a `false` from the spy fails. The spy answers `false` once;
/// each route with a target rule answers 403 `target_not_allowed` and changes nothing.
#[tokio::test]
async fn a_refusal_from_the_policy_is_a_403_and_changes_nothing() {
    let spy = Spy::new();
    let w = world_with("p4e_spy_refuse", spy.clone()).await;
    let mut admin = w.admin().await;
    let pending = Seed::pending(Who::L)
        .addressed_to_worker(Who::W1)
        .insert(&mut admin)
        .await;
    let mut returned = Seed::pending(Who::L).claimed_by(Who::W1);
    returned.state = "returned";
    let returned = returned.insert(&mut admin).await;
    seed_return(&mut admin, returned, project_a(), 1, Who::W1, "report").await;
    let held = Seed::pending(Who::L)
        .claimed_by(Who::W2)
        .insert(&mut admin)
        .await;

    let cases: Vec<(&str, Who, String, Option<Value>)> = vec![
        (
            "create",
            Who::L,
            "/v2/handoffs".to_owned(),
            Some(task(role("dev"))),
        ),
        ("claim", Who::W1, path(pending, "claim"), None),
        ("accept", Who::L, path(returned, "accept"), Some(json!({}))),
        (
            "rework",
            Who::L,
            path(returned, "rework"),
            Some(json!({ "notes": "n" })),
        ),
        (
            "cancel",
            Who::L,
            path(held, "cancel"),
            Some(json!({ "reason": "r" })),
        ),
    ];
    for (name, who, p, body) in cases {
        let rows = |sql: &'static str| {
            let w = &w;
            async move { w.count(sql).await }
        };
        let before = (
            rows("SELECT count(*) FROM handoffs.handoffs").await,
            rows("SELECT count(*) FROM handoffs.transitions").await,
            rows("SELECT count(*) FROM handoffs.reviews").await,
            rows("SELECT count(*) FROM fx_audit.events").await,
        );
        let state_before = states(&w).await;
        spy.take();
        spy.refuse_next();
        let a = send(&w, who, &p, body).await;
        assert_eq!(
            code(&a),
            (403, Some("target_not_allowed".into())),
            "{name}: {}",
            a.text
        );
        assert!(!spy.refusal_pending(), "{name}: the route asked the policy");
        let after = (
            rows("SELECT count(*) FROM handoffs.handoffs").await,
            rows("SELECT count(*) FROM handoffs.transitions").await,
            rows("SELECT count(*) FROM handoffs.reviews").await,
            rows("SELECT count(*) FROM fx_audit.events").await,
        );
        assert_eq!(before, after, "{name}: no row, log, review or audit row");
        assert_eq!(state_before, states(&w).await, "{name}: no handoff moved");
    }
}

// ---- HAND-121 (c), (d), (e) ------------------------------------------------------------------------------

/// HAND-121 (c): an absent or unknown top-level `kind` is 422 `invalid_request` at decoding and stores nothing; it
/// never defaults to a consult.
#[tokio::test]
async fn an_absent_or_unknown_kind_is_422_and_never_a_consult() {
    let spy = Spy::new();
    let w = world_with("p4e_kind", spy.clone()).await;
    let base = consult(worker(Who::W2));
    let mut absent = base.clone();
    absent.as_object_mut().expect("object").remove("kind");
    let mut cases = vec![("absent", absent)];
    for (label, v) in [
        ("Consult", json!("Consult")),
        ("consult with a space", json!("consult ")),
        ("note", json!("note")),
        ("null", Value::Null),
        ("an array", json!(["consult"])),
    ] {
        let mut b = base.clone();
        b["kind"] = v;
        cases.push((label, b));
    }
    for (label, body) in cases {
        spy.take();
        let a = send(&w, Who::W1, "/v2/handoffs", Some(body)).await;
        assert_eq!(
            code(&a),
            (422, Some("invalid_request".into())),
            "{label}: {}",
            a.text
        );
        assert!(
            spy.take()
                .iter()
                .all(|(_, rule, _)| *rule == TargetRule::None),
            "{label}: decoding refuses before any target rule is asked"
        );
    }
    assert_eq!(
        w.count("SELECT count(*) FROM handoffs.handoffs").await,
        0,
        "nothing stored"
    );

    // The same wrong types on the other word fields: `to.kind` on create and an evidence `kind` on return. A
    // `null`, an array or a number in an enum field is a type error (422), not "invalid JSON" (400).
    for (label, v) in [
        ("to.kind null", Value::Null),
        ("to.kind array", json!(["worker"])),
        ("to.kind number", json!(7)),
        ("to.kind team", json!("team")),
        ("to.kind human", json!("human")),
    ] {
        let mut b = task(worker(Who::W2));
        b["to"]["kind"] = v;
        let a = send(&w, Who::L, "/v2/handoffs", Some(b)).await;
        assert_eq!(
            code(&a),
            (422, Some("invalid_request".into())),
            "{label}: {}",
            a.text
        );
    }
    let mut admin = w.admin().await;
    let held = Seed::pending(Who::L)
        .claimed_by(Who::W1)
        .insert(&mut admin)
        .await;
    for (label, v) in [
        ("evidence kind null", Value::Null),
        ("evidence kind array", json!(["link"])),
        ("evidence kind unknown", json!("screenshot")),
    ] {
        let mut b = report();
        b["evidence"][0]["kind"] = v;
        let a = send(&w, Who::W1, &path(held, "return"), Some(b)).await;
        assert_eq!(
            code(&a),
            (422, Some("invalid_request".into())),
            "{label}: {}",
            a.text
        );
    }
    assert_eq!(
        w.count("SELECT count(*) FROM handoffs.returns").await,
        0,
        "no return stored"
    );
}

/// HAND-121 (d): both kinds of address reach the spy with `consult` false for a task; only a request whose
/// top-level `kind` is `consult` reaches it with `consult` true (PO-18 (e)).
#[tokio::test]
async fn only_the_top_level_kind_sets_the_flag() {
    let spy = Spy::new();
    let w = world_with("p4e_flag", spy.clone()).await;
    for (label, body, want) in [
        ("task to a worker", task(worker(Who::W2)), false),
        ("task to a job role", task(role("dev")), false),
        ("consult to a worker", consult(worker(Who::W2)), true),
        ("consult to a job role", consult(role("dev")), true),
    ] {
        spy.take();
        let a = send(&w, Who::W1, "/v2/handoffs", Some(body)).await;
        let calls = spy.take();
        assert_eq!(calls.len(), 1, "{label}: {calls:?}");
        assert_eq!(calls[0].1, TargetRule::TaskAddressee, "{label}");
        assert_eq!(calls[0].2.consult, want, "{label}");
        let expect = if want { 201 } else { 403 };
        assert_eq!(a.status.as_u16(), expect, "{label}: {}", a.text);
    }
}

/// HAND-121 (e): there is no route that turns a consult into a task. `PUT` and `PATCH` on a handoff are 405 with
/// an `Allow` header (R-3); the database refuses the `UPDATE` of `kind` (HAND-95, `w1_handoffs_p4c_db_guarantees.rs`).
#[tokio::test]
async fn no_route_changes_the_kind() {
    let spy = Spy::new();
    let w = world_with("p4e_nokind", spy.clone()).await;
    let mut admin = w.admin().await;
    let id = Seed::pending(Who::W1).consult().insert(&mut admin).await;
    for method in [Method::PUT, Method::PATCH] {
        let a = w
            .send(
                method.clone(),
                &format!("/v2/handoffs/{id}"),
                Some(Who::L),
                Some(&key()),
                Some(json!({ "kind": "task" })),
            )
            .await;
        assert_eq!(a.status.as_u16(), 405, "{method}: {}", a.text);
        let allow = a
            .headers
            .get("allow")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        assert!(allow.contains("GET"), "{method}: Allow is `{allow}`");
        assert!(
            !allow.contains("PUT") && !allow.contains("PATCH"),
            "{method}: Allow is `{allow}`"
        );
    }
    let kind: String = sqlx::query_scalar("SELECT kind FROM handoffs.handoffs WHERE id = $1::uuid")
        .bind(id.to_string())
        .fetch_one(&mut admin)
        .await
        .expect("kind");
    assert_eq!(kind, "consult");
}
