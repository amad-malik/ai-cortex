//! Wave 1, mod-handoffs P2b (row 2a86598d): `POST /v2/handoffs` behind the real chain, on a real database.
//! Test definitions: `mvp/tests/mod-handoffs.md` r3.4, HAND-01 to HAND-09 (create), HAND-80 and HAND-81
//! (the key), the create cells of HAND-103, HAND-104 and HAND-121.
//!
//! **Pending finn's `wave1/ports`:** the consult flag (`Target.consult`, cox W1-9) does not exist at the pin,
//! so the two cells that need it (a member's consult is 201; the flag comes from the top-level `kind`) are
//! `consult_*` and stay RED until that branch is merged into this lane. Run ONLY under
//! `crates/cortex-testkit/supervise.sh` through `cv2-admit`, `CV2_DB=1`, one database run at a time.
mod common;

use axum::http::Method;
use common::{Who, key, world};
use serde_json::{Value, json};

fn role(r: &str) -> Value {
    json!({ "kind": "job_role", "ref": r })
}

fn worker(w: Who) -> Value {
    json!({ "kind": "worker", "ref": w.id().to_string() })
}

fn task(to: Value) -> Value {
    json!({ "kind": "task", "to": to, "summary": "s", "next_action": "n", "acceptance": "a" })
}

fn consult(to: Value) -> Value {
    json!({ "kind": "consult", "to": to, "summary": "s", "next_action": "n", "acceptance": "a" })
}

fn with(mut v: Value, k: &str, x: Value) -> Value {
    v[k] = x;
    v
}

fn without(mut v: Value, k: &str) -> Value {
    v.as_object_mut().expect("an object").remove(k);
    v
}

/// HAND-01 and HAND-09: a complete packet is stored as sent; one row in the table, the log and the audit.
#[tokio::test]
async fn create_stores_the_packet_and_leaves_one_row_in_the_log_and_the_audit() {
    let w = world("p2b_create").await;
    let body = json!({
        "kind": "task",
        "to": role("dev"),
        "summary": "the summary",
        "next_action": "the next action",
        "acceptance": "the acceptance criteria",
        "verify": "run the suite",
        "context": "see the plan",
        "files": ["a.rs", "b.rs"],
        "blockers": ["waiting for the freeze"],
        "decisions": ["use the existing predicate"],
        "priority": 90,
        "due_at": "2026-12-01T09:30:00.250000Z"
    });
    let a = w.post(Who::L, "/v2/handoffs", body.clone()).await;
    assert_eq!(a.status.as_u16(), 201, "{}", a.text);
    let id = a.json["id"].as_str().expect("an id").to_owned();
    assert_eq!(a.json, json!({ "id": id, "state": "pending" }));

    let shown = w.get(Who::W4, &format!("/v2/handoffs/{id}")).await;
    assert_eq!(shown.status.as_u16(), 200, "{}", shown.text);
    let h = &shown.json;
    for k in [
        "kind",
        "summary",
        "next_action",
        "acceptance",
        "verify",
        "context",
        "files",
        "blockers",
        "decisions",
        "priority",
        "to",
    ] {
        assert_eq!(h[k], body[k], "{k} is returned exactly as created");
    }
    assert_eq!(h["state"], "pending");
    assert_eq!(h["due_at"], "2026-12-01T09:30:00.250000Z");
    assert_eq!(
        (h["round"].as_i64(), h["created_by"].as_str()),
        (Some(1), Some(Who::L.id().to_string().as_str()))
    );
    assert!(h["holder"].is_null() && h["latest_return"].is_null() && h["closed_by"].is_null());

    assert_eq!(w.count("SELECT count(*) FROM handoffs.handoffs").await, 1);
    let mut admin = w.admin().await;
    let (seq, from, to, kind, actor): (i32, Option<String>, String, String, String) =
        sqlx::query_as(
            "SELECT seq, from_state, to_state, actor_kind, actor_id FROM handoffs.transitions",
        )
        .fetch_one(&mut admin)
        .await
        .expect("the one log row");
    assert_eq!(
        (seq, from, to.as_str(), kind.as_str(), actor),
        (1, None, "pending", "worker", Who::L.id().to_string())
    );
    let audit: Vec<(String, String)> = sqlx::query_as("SELECT action, target FROM fx_audit.events")
        .fetch_all(&mut admin)
        .await
        .expect("audit");
    assert_eq!(
        audit,
        [("handoff.created".to_owned(), format!("handoff:{id}"))]
    );
    let (incarnation, seq_counter, holder): (i64, i32, Option<String>) =
        sqlx::query_as("SELECT incarnation, transition_seq, holder::text FROM handoffs.handoffs")
            .fetch_one(&mut admin)
            .await
            .expect("the row");
    assert_eq!((incarnation, seq_counter, holder), (0, 1, None));
}

/// HAND-05: who may create what (the policy double follows sophi's table 3.2).
#[tokio::test]
async fn create_follows_the_target_rule() {
    let w = world("p2b_who").await;
    let ok = |who: Who, body: Value| {
        let w = &w;
        async move { w.post(who, "/v2/handoffs", body).await.status.as_u16() }
    };
    // The project lead: any addressee.
    assert_eq!(ok(Who::L, task(worker(Who::W1))).await, 201);
    assert_eq!(ok(Who::L, task(worker(Who::W3))).await, 201);
    assert_eq!(ok(Who::L, task(role("dev"))).await, 201);
    // A team lead: a worker of its own team only.
    assert_eq!(ok(Who::TL, task(worker(Who::W1))).await, 201);
    assert_eq!(
        ok(Who::TL, task(worker(Who::W3))).await,
        403,
        "a worker of team U"
    );
    assert_eq!(
        ok(Who::TL, task(role("dev"))).await,
        403,
        "a job-role task needs the project lead"
    );
    // A member creates no task.
    assert_eq!(ok(Who::W1, task(worker(Who::W2))).await, 403);
    assert_eq!(ok(Who::W1, task(role("dev"))).await, 403);
    // 403 is target_not_allowed and leaves no row.
    let a = w.post(Who::W1, "/v2/handoffs", task(role("dev"))).await;
    assert_eq!(a.code(), (403, Some("target_not_allowed")), "{}", a.text);
    assert_eq!(w.count("SELECT count(*) FROM handoffs.handoffs").await, 4);
}

/// HAND-121 (a), (b): the consult flag comes only from the top-level `kind`. RED until `wave1/ports`.
#[tokio::test]
async fn consult_by_a_member_is_201_and_the_same_body_as_a_task_is_403() {
    let w = world("p2b_consult").await;
    for to in [worker(Who::W2), role("dev")] {
        let a = w.post(Who::W1, "/v2/handoffs", task(to.clone())).await;
        assert_eq!(
            a.code(),
            (403, Some("target_not_allowed")),
            "a member's task: {}",
            a.text
        );
        let b = w.post(Who::W1, "/v2/handoffs", consult(to)).await;
        assert_eq!(b.status.as_u16(), 201, "a member's consult: {}", b.text);
    }
}

/// HAND-04: the addressee is read (404), retired (409) or must hold the role (422).
#[tokio::test]
async fn create_checks_the_addressee() {
    let w = world("p2b_addressee").await;
    let code = |a: &common::Answer| {
        (
            a.status.as_u16(),
            a.json["type"].as_str().map(str::to_owned),
        )
    };
    // An unknown worker and another project's worker: the same 404.
    let unknown = json!({ "kind": "worker", "ref": "01930000-0000-7000-8000-0000000000aa" });
    let a = w.post(Who::L, "/v2/handoffs", task(unknown)).await;
    let b = w.post(Who::L, "/v2/handoffs", task(worker(Who::B2))).await;
    assert_eq!(code(&a), (404, Some("not_found".into())));
    assert_eq!(code(&b), (404, Some("not_found".into())));
    let strip = |a: &common::Answer| {
        let mut j = a.json.clone();
        j.as_object_mut().expect("an object").remove("request_id");
        j
    };
    assert_eq!(
        strip(&a),
        strip(&b),
        "no way to tell `exists in B` from `does not exist`"
    );
    // A retired worker of this project: 409 target_retired.
    let r = w.post(Who::L, "/v2/handoffs", task(worker(Who::W5))).await;
    assert_eq!(code(&r), (409, Some("target_retired".into())));
    // A role some active worker holds is accepted; a role nobody holds is 422 unknown_addressee.
    assert_eq!(
        w.post(Who::L, "/v2/handoffs", task(role("qa")))
            .await
            .status
            .as_u16(),
        201
    );
    let none = w.post(Who::L, "/v2/handoffs", task(role("nobody"))).await;
    assert_eq!(code(&none), (422, Some("unknown_addressee".into())));
    // Step 0: a malformed worker ref and a role outside the grammar are invalid_request, before any read.
    for bad in [
        json!({ "kind": "worker", "ref": "abc" }),
        role("Dev"),
        role("1qa"),
        role("a b"),
        role(&"a".repeat(33)),
        role("dev\n"),
    ] {
        let a = w.post(Who::L, "/v2/handoffs", task(bad.clone())).await;
        assert_eq!(
            code(&a),
            (422, Some("invalid_request".into())),
            "{bad}: {}",
            a.text
        );
    }
    assert_eq!(
        w.count("SELECT count(*) FROM handoffs.handoffs").await,
        1,
        "only the `qa` task was stored"
    );
}

/// HAND-103 (e) to (g), HAND-104: when two refusals apply, the first in R-16's order is the answer.
#[tokio::test]
async fn create_reports_the_first_refusal_in_r16_order() {
    let w = world("p2b_order").await;
    let code = |a: &common::Answer| {
        (
            a.status.as_u16(),
            a.json["type"].as_str().map(str::to_owned),
        )
    };
    // (e) a member's task with an over-long summary: step 0 (422) before the target rule (403).
    let big = with(task(role("dev")), "summary", json!("x".repeat(2049)));
    assert_eq!(
        code(&w.post(Who::W1, "/v2/handoffs", big).await),
        (422, Some("invalid_request".into()))
    );
    // (f) no summary to an unknown worker: packet_incomplete before the 404.
    let unknown = json!({ "kind": "worker", "ref": "01930000-0000-7000-8000-0000000000aa" });
    let no_summary = without(task(unknown), "summary");
    assert_eq!(
        code(&w.post(Who::L, "/v2/handoffs", no_summary).await),
        (422, Some("packet_incomplete".into()))
    );
    // (g) a team lead's task to a role nobody holds: the target rule (403) before unknown_addressee.
    assert_eq!(
        code(&w.post(Who::TL, "/v2/handoffs", task(role("nobody"))).await),
        (403, Some("target_not_allowed".into()))
    );
    // HAND-104 (a): a member's task to `QA`: the grammar (422) before the target rule (403).
    assert_eq!(
        code(&w.post(Who::W1, "/v2/handoffs", task(role("QA"))).await),
        (422, Some("invalid_request".into()))
    );
    assert_eq!(w.count("SELECT count(*) FROM handoffs.handoffs").await, 0);
}

/// HAND-02: the aggregate limit on `files`, `blockers` and `decisions` agrees with S2's table CHECK.
#[tokio::test]
async fn create_aggregate_limit_agrees_with_the_database_check() {
    let w = world("p2b_aggregate").await;
    let big = |n: usize| json!(["x".repeat(n)]);
    // One array: 65,532 bytes of element is exactly 65,536 bytes of jsonb text.
    let a = w
        .post(
            Who::L,
            "/v2/handoffs",
            with(task(role("dev")), "files", big(65_532)),
        )
        .await;
    assert_eq!(a.status.as_u16(), 201, "{}", a.text);
    let b = w
        .post(
            Who::L,
            "/v2/handoffs",
            with(task(role("dev")), "files", big(65_533)),
        )
        .await;
    assert_eq!(b.code(), (422, Some("invalid_request")), "{}", b.text);
    // Across the three arrays.
    let spread = |n: usize| {
        with(
            with(with(task(role("dev")), "files", big(n)), "blockers", big(n)),
            "decisions",
            big(n),
        )
    };
    let c = w.post(Who::L, "/v2/handoffs", spread(21_841)).await;
    assert_eq!(c.status.as_u16(), 201, "{}", c.text);
    let d = w.post(Who::L, "/v2/handoffs", spread(21_842)).await;
    assert_eq!(d.code(), (422, Some("invalid_request")), "{}", d.text);
    assert_eq!(
        w.count("SELECT count(*) FROM handoffs.handoffs").await,
        2,
        "the accepted two, no 500 in between"
    );
}

/// HAND-03, HAND-06: strict decoding, and no addressee kinds beyond worker and job_role.
#[tokio::test]
async fn create_refuses_unknown_fields_and_other_addressee_kinds() {
    let w = world("p2b_strict").await;
    for bad in [
        with(
            task(role("dev")),
            "created_by",
            json!(Who::W1.id().to_string()),
        ),
        with(task(role("dev")), "incarnation", json!(1)),
        with(task(role("dev")), "state", json!("claimed")),
        task(json!({ "kind": "team", "ref": "x" })),
        task(json!({ "kind": "human", "ref": "x" })),
        without(task(role("dev")), "kind"),
        with(task(role("dev")), "kind", json!("Consult")),
        with(task(role("dev")), "kind", json!("note")),
    ] {
        let a = w.post(Who::L, "/v2/handoffs", bad.clone()).await;
        assert_eq!(
            a.code(),
            (422, Some("invalid_request")),
            "{bad}: {}",
            a.text
        );
    }
    assert_eq!(w.count("SELECT count(*) FROM handoffs.handoffs").await, 0);
}

/// HAND-07, HAND-08, HAND-80, HAND-81: no quota; the size limit; the key.
#[tokio::test]
async fn create_has_no_quota_a_body_limit_and_needs_a_key() {
    let w = world("p2b_misc").await;
    // No quota in 0.2.001: many creates by one creator, all 201.
    for i in 0..25 {
        let a = w
            .post(
                Who::L,
                "/v2/handoffs",
                with(task(role("dev")), "summary", json!(format!("s{i}"))),
            )
            .await;
        assert_eq!(a.status.as_u16(), 201, "create {i}: {}", a.text);
    }
    // The body limit is 262,144 bytes: one byte over is 413 before parsing.
    let huge = with(task(role("dev")), "context", json!("x".repeat(262_144)));
    let a = w.post(Who::L, "/v2/handoffs", huge).await;
    assert_eq!(a.code(), (413, Some("body_too_large")), "{}", a.text);
    // No Idempotency-Key: 428.
    let a = w
        .send(
            Method::POST,
            "/v2/handoffs",
            Some(Who::L),
            None,
            Some(task(role("dev"))),
        )
        .await;
    assert_eq!(a.code(), (428, Some("idempotency_key_required")));
    // The same key and body twice: one handoff, the same answer, the replay header on the second.
    let k = key();
    let first = w
        .send(
            Method::POST,
            "/v2/handoffs",
            Some(Who::L),
            Some(&k),
            Some(task(role("qa"))),
        )
        .await;
    let second = w
        .send(
            Method::POST,
            "/v2/handoffs",
            Some(Who::L),
            Some(&k),
            Some(task(role("qa"))),
        )
        .await;
    assert_eq!((first.status.as_u16(), second.status.as_u16()), (201, 201));
    assert_eq!(first.json, second.json, "the stored reply");
    assert!(
        second.headers.contains_key("cortex-idempotent-replay")
            && !first.headers.contains_key("cortex-idempotent-replay")
    );
    assert_eq!(
        w.count("SELECT count(*) FROM handoffs.handoffs").await,
        26,
        "25 + the one replayed key"
    );
}
