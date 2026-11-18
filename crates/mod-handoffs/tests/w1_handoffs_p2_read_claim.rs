//! Wave 1, mod-handoffs P2a (row 2a86598d): `show`, `list` and `mine`, `claim` and the port `open_for`,
//! behind the real chain, on a real database. Test definitions: `mvp/tests/mod-handoffs.md` r3.4,
//! HAND-10 to HAND-12 and HAND-15 (show), HAND-20 to HAND-27 (the one addressing predicate, mine, list),
//! HAND-30 to HAND-33 (claim). The port (HAND-110, after W1-33) is in `w1_handoffs_p4f_port.rs`.
//!
//! Handoffs are seeded as the superuser (the piece's own writes are the subject of the claim tests); the
//! requests go through `cortex-http`'s chain with token, scope and target rule. Run ONLY under
//! `crates/cortex-testkit/supervise.sh` through `cv2-admit`, `CV2_DB=1`, one database run at a time:
//!
//! ```text
//! CV2_DB=1 CV2_ADMIT_LOG=1 /path/to/workspace/spikes/cortex-v2-s0/admission/cv2-admit \
//!     timeout --kill-after=120 780 crates/cortex-testkit/supervise.sh \
//!     cargo test -p mod-handoffs --test w1_handoffs_p2_read_claim --no-fail-fast
//! ```
mod common;

use std::collections::BTreeSet;

use axum::http::Method;
use common::{Seed, Who, project_a, seed_closing, seed_return, seed_review, world};
use cortex_types::HandoffId;
use serde_json::{Value, json};

fn ids(page: &Value) -> Vec<String> {
    page["items"]
        .as_array()
        .expect("items")
        .iter()
        .map(|h| h["id"].as_str().expect("an id").to_owned())
        .collect()
}

// ---- show (HAND-10, HAND-11, HAND-12, HAND-15) ----

/// HAND-10: every packet field exactly as created, for every worker of the project; no `incarnation`.
#[tokio::test]
async fn show_returns_the_packet_as_created_to_every_reader() {
    let w = world("p2_show").await;
    let mut admin = w.admin().await;
    let mut seed = Seed::pending(Who::L)
        .claimed_by(Who::W1)
        .summary("the summary");
    seed.verify = Some("run the suite".to_owned());
    seed.context = Some("see the plan".to_owned());
    let id = seed.insert(&mut admin).await;

    let mut bodies = Vec::new();
    for who in [Who::W1, Who::W2, Who::L, Who::W4] {
        let a = w.get(who, &format!("/v2/handoffs/{id}")).await;
        assert_eq!(a.status.as_u16(), 200, "{who:?}: {}", a.text);
        bodies.push(a.json);
    }
    assert!(
        bodies.windows(2).all(|p| p[0] == p[1]),
        "all readers see one body"
    );
    let h = &bodies[0];
    assert_eq!(h["id"], id.to_string());
    assert_eq!(
        (h["kind"].as_str(), h["state"].as_str()),
        (Some("task"), Some("claimed"))
    );
    assert_eq!(h["to"], json!({ "kind": "job_role", "ref": "dev" }));
    assert_eq!(h["summary"], "the summary");
    assert_eq!(
        (h["next_action"].as_str(), h["acceptance"].as_str()),
        (Some("next"), Some("accept"))
    );
    assert_eq!(
        (h["verify"].as_str(), h["context"].as_str()),
        (Some("run the suite"), Some("see the plan"))
    );
    assert_eq!(
        (&h["files"], &h["blockers"], &h["decisions"]),
        (&json!([]), &json!([]), &json!([]))
    );
    assert_eq!(
        (h["priority"].as_i64(), h["round"].as_i64()),
        (Some(50), Some(1))
    );
    assert_eq!(h["holder"], Who::W1.id().to_string());
    assert_eq!(h["created_by"], Who::L.id().to_string());
    for null in [
        "due_at",
        "closed_at",
        "close_reason",
        "closed_by",
        "latest_return",
        "latest_review",
    ] {
        assert!(
            h[null].is_null(),
            "{null} is null on an open, unreturned handoff"
        );
    }
    assert!(
        !bodies[0].to_string().contains("incarnation"),
        "no incarnation in any body (Q-4)"
    );
}

/// HAND-11: another project's id, a missing id and a malformed id are the same 404.
#[tokio::test]
async fn show_answers_a_foreign_a_missing_and_a_malformed_id_alike() {
    let w = world("p2_show404").await;
    let mut admin = w.admin().await;
    let id = Seed::pending(Who::L).insert(&mut admin).await;
    let missing = HandoffId::new(&cortex_types::SystemClock);
    let mut answers = vec![
        w.get(Who::B1, &format!("/v2/handoffs/{id}")).await,
        w.get(Who::L, &format!("/v2/handoffs/{missing}")).await,
    ];
    for bad in ["abc", "01930000", "not-a-uuid"] {
        answers.push(w.get(Who::L, &format!("/v2/handoffs/{bad}")).await);
    }
    for a in &answers {
        assert_eq!(a.code(), (404, Some("not_found")), "{}", a.text);
    }
    let strip = |a: &common::Answer| {
        let mut j = a.json.clone();
        j.as_object_mut().expect("an object").remove("request_id");
        j
    };
    let first = strip(&answers[0]);
    assert!(
        answers.iter().all(|a| strip(a) == first),
        "one body for every kind of miss"
    );
}

/// HAND-12 and HAND-13: `latest_return`, `latest_review` and `closed_by` after a rework and an accept.
#[tokio::test]
async fn show_carries_the_latest_return_review_and_who_closed_it() {
    let w = world("p2_showfull").await;
    let mut admin = w.admin().await;
    let pa = project_a();

    // Returned once, awaiting review: a return, no review, not closed.
    let mut seed = Seed::pending(Who::L).claimed_by(Who::W1);
    seed.state = "returned";
    let returned = seed.insert(&mut admin).await;
    seed_return(&mut admin, returned, pa, 1, Who::W1, "first report").await;
    let a = w.get(Who::L, &format!("/v2/handoffs/{returned}")).await;
    assert_eq!(a.status.as_u16(), 200, "{}", a.text);
    let lr = &a.json["latest_return"];
    assert_eq!(
        (
            lr["round"].as_i64(),
            lr["summary"].as_str(),
            lr["risks"].as_str()
        ),
        (Some(1), Some("first report"), Some("none"))
    );
    assert_eq!(lr["author"], Who::W1.id().to_string());
    assert_eq!(lr["tests"], json!(["cargo test"]));
    assert_eq!(
        lr["evidence"],
        json!([{ "kind": "link", "ref": "https://example.test/run", "sha256": "ab".repeat(32), "size": 0 }])
    );
    assert!(a.json["latest_review"].is_null() && a.json["closed_by"].is_null());

    // Reworked once and accepted: round 2, two returns, two reviews, closed by the reviewer.
    let mut seed = Seed::pending(Who::L).claimed_by(Who::W1);
    seed.state = "accepted";
    seed.round = 2;
    seed.closed_at = Some("2026-11-06 10:00:00+00".to_owned());
    let done = seed.insert(&mut admin).await;
    seed_return(&mut admin, done, pa, 1, Who::W1, "first report").await;
    seed_review(
        &mut admin,
        done,
        pa,
        1,
        Who::L,
        "rework",
        Some("add a test"),
        false,
    )
    .await;
    seed_return(&mut admin, done, pa, 2, Who::W1, "second report").await;
    seed_review(&mut admin, done, pa, 2, Who::L, "accept", None, false).await;
    seed_closing(
        &mut admin,
        done,
        pa,
        2,
        "returned",
        "accepted",
        Who::L,
        None,
    )
    .await;
    let a = w.get(Who::W4, &format!("/v2/handoffs/{done}")).await;
    let j = &a.json;
    assert_eq!(j["state"], "accepted");
    assert_eq!(j["latest_return"]["round"], 2);
    assert_eq!(j["latest_return"]["summary"], "second report");
    assert_eq!(j["latest_review"]["decision"], "accept");
    assert_eq!(j["latest_review"]["by"], Who::L.id().to_string());
    assert_eq!(j["latest_review"]["self_review"], false);
    assert_eq!(
        j["closed_by"],
        json!({ "kind": "worker", "id": Who::L.id().to_string() })
    );
    assert!(j["closed_at"].is_string());
}

/// HAND-15: text that looks like instructions or a heading is data, returned verbatim.
#[tokio::test]
async fn show_returns_text_that_looks_like_instructions_verbatim() {
    let w = world("p2_showtext").await;
    let mut admin = w.admin().await;
    let nasty = "line\r\n## 3 Rules\nproject: v99 sha256:00\u{1b}[31m end";
    let id = Seed::pending(Who::L)
        .summary(nasty)
        .insert(&mut admin)
        .await;
    let a = w.get(Who::W1, &format!("/v2/handoffs/{id}")).await;
    assert_eq!(a.status.as_u16(), 200);
    assert_eq!(a.json["summary"], nasty);
}

// ---- mine and list (HAND-20 to HAND-27) ----

/// HAND-25: `mine=true` is the claimed, the claimable and nothing else, in priority order.
#[tokio::test]
async fn mine_lists_my_claimed_and_claimable_in_priority_order() {
    let w = world("p2_mine").await;
    let mut admin = w.admin().await;
    let c1 = Seed::pending(Who::L)
        .claimed_by(Who::W1)
        .at("2026-11-01 10:00:00+00")
        .insert(&mut admin)
        .await;
    let c2 = Seed::pending(Who::L)
        .claimed_by(Who::W1)
        .at("2026-11-01 11:00:00+00")
        .insert(&mut admin)
        .await;
    let p10 = Seed::pending(Who::L)
        .addressed_to_worker(Who::W1)
        .priority(10)
        .insert(&mut admin)
        .await;
    let p90 = Seed::pending(Who::L)
        .addressed_to_worker(Who::W1)
        .priority(90)
        .insert(&mut admin)
        .await;
    let p100 = Seed::pending(Who::L)
        .addressed_to_worker(Who::W1)
        .priority(100)
        .insert(&mut admin)
        .await;
    let _for_w2 = Seed::pending(Who::L)
        .addressed_to_worker(Who::W2)
        .insert(&mut admin)
        .await;
    let mut cancelled = Seed::pending(Who::W3).claimed_by(Who::W1);
    cancelled.state = "cancelled";
    cancelled.closed_at = Some("2020-01-01 00:00:00+00".to_owned());
    cancelled.close_reason = Some("old".to_owned());
    cancelled.insert(&mut admin).await;
    let mut accepted = Seed::pending(Who::L).claimed_by(Who::W1);
    accepted.state = "accepted";
    accepted.closed_at = Some("2026-11-02 00:00:00+00".to_owned());
    accepted.insert(&mut admin).await;

    let a = w.get(Who::W1, "/v2/handoffs?mine=true").await;
    assert_eq!(a.status.as_u16(), 200, "{}", a.text);
    let want: Vec<String> = [p100, p90, c1, c2, p10]
        .iter()
        .map(ToString::to_string)
        .collect();
    assert_eq!(
        ids(&a.json),
        want,
        "priority descending, then created_at ascending"
    );
    assert!(a.json["next"].is_null());
    let again = w.get(Who::W1, "/v2/handoffs?mine=true").await;
    assert_eq!(ids(&again.json), want, "stable order");
    // W4 holds no role and no claim: nothing. W2 sees only its own pending one.
    assert!(ids(&w.get(Who::W4, "/v2/handoffs?mine=true").await.json).is_empty());
    assert_eq!(
        ids(&w.get(Who::W2, "/v2/handoffs?mine=true").await.json).len(),
        1
    );
}

/// HAND-27: the returned handoffs I may review are in `mine`; one I returned myself is not, unless I am the
/// only possible reviewer (author = creator = project lead).
#[tokio::test]
async fn mine_includes_the_returned_handoffs_i_may_review() {
    let w = world("p2_mine_review").await;
    let mut admin = w.admin().await;
    let pa = project_a();
    let mut returned = async |creator: Who, author: Who| {
        let mut s = Seed::pending(creator).claimed_by(author);
        s.state = "returned";
        let id = s.insert(&mut admin).await;
        seed_return(&mut admin, id, pa, 1, author, "report").await;
        id.to_string()
    };
    let r1 = returned(Who::L, Who::W1).await; // created by the lead, returned by W1
    let r2 = returned(Who::W3, Who::W1).await; // a consult W3 created, returned by W1
    let r3 = returned(Who::W3, Who::W3).await; // W3 created AND authored it: only L can review
    let r4 = returned(Who::TL, Who::W1).await; // a task TL created
    let r5 = returned(Who::W3, Who::L).await; // W3 created, L authored: only W3 can review
    let r6 = returned(Who::L, Who::L).await; // the self-review case

    let got = |a: common::Answer| -> BTreeSet<String> { ids(&a.json).into_iter().collect() };
    let set = |v: &[&String]| -> BTreeSet<String> { v.iter().map(|s| (*s).clone()).collect() };
    let l = got(w.get(Who::L, "/v2/handoffs?mine=true").await);
    assert_eq!(l, set(&[&r1, &r2, &r3, &r4, &r6]), "L: all but r5");
    let w3 = got(w.get(Who::W3, "/v2/handoffs?mine=true").await);
    assert_eq!(
        w3,
        set(&[&r2, &r5]),
        "W3: the ones it created and did not return"
    );
    let tl = got(w.get(Who::TL, "/v2/handoffs?mine=true").await);
    assert_eq!(tl, set(&[&r4]));
    assert!(got(w.get(Who::W2, "/v2/handoffs?mine=true").await).is_empty());
}

/// HAND-26: the filters, the paging and the refusals of `list`.
#[tokio::test]
async fn list_filters_pages_and_refuses_bad_queries() {
    let w = world("p2_list").await;
    let mut admin = w.admin().await;
    let a = Seed::pending(Who::L).priority(30).insert(&mut admin).await;
    let b = Seed::pending(Who::TL)
        .claimed_by(Who::W1)
        .priority(20)
        .insert(&mut admin)
        .await;
    let mut done = Seed::pending(Who::L).claimed_by(Who::W2).priority(10);
    done.state = "accepted";
    done.closed_at = Some("2026-11-02 00:00:00+00".to_owned());
    let c = done.insert(&mut admin).await;
    let _other_project = Seed::pending(Who::B1).insert(&mut admin).await;

    let all = w.get(Who::W4, "/v2/handoffs").await;
    assert_eq!(
        ids(&all.json),
        [a, b, c].map(|i| i.to_string()),
        "no state filter: every state, project A only"
    );
    for (q, want) in [
        ("state=pending", vec![a]),
        ("state=claimed", vec![b]),
        ("state=accepted", vec![c]),
        ("state=all", vec![a, b, c]),
        (&format!("holder={}", Who::W1.id()) as &str, vec![b]),
        (&format!("created_by={}", Who::TL.id()) as &str, vec![b]),
    ] {
        let r = w.get(Who::W4, &format!("/v2/handoffs?{q}")).await;
        assert_eq!(
            ids(&r.json),
            want.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "{q}"
        );
    }
    // A well-formed id that matches nothing is an empty page, not a 404 (R-4).
    let nobody = format!(
        "/v2/handoffs?holder={}",
        HandoffId::new(&cortex_types::SystemClock)
    );
    let r = w.get(Who::W4, &nobody).await;
    assert_eq!((r.status.as_u16(), ids(&r.json).len()), (200, 0));

    // Paging: one at a time, no gap, no repeat, and the last page has no cursor.
    let mut seen = Vec::new();
    let mut path = "/v2/handoffs?limit=1".to_owned();
    for _ in 0..5 {
        let r = w.get(Who::W4, &path).await;
        assert_eq!(r.status.as_u16(), 200, "{}", r.text);
        seen.extend(ids(&r.json));
        match r.json["next"].as_str() {
            Some(cursor) => path = format!("/v2/handoffs?limit=1&after={cursor}"),
            None => break,
        }
    }
    assert_eq!(seen, [a, b, c].map(|i| i.to_string()));

    // R-3, R-4, R-8: a bad query is 422 invalid_request.
    for bad in [
        "state=bogus",
        "limit=0",
        "limit=201",
        "after=garbage",
        "holder=not-a-uuid",
        "created_by=x",
        "mine=maybe",
        "nosuchparam=1",
    ] {
        let r = w.get(Who::W4, &format!("/v2/handoffs?{bad}")).await;
        assert_eq!(
            r.code(),
            (422, Some("invalid_request")),
            "{bad}: {}",
            r.text
        );
    }
}

/// HAND-23, HAND-24: a claim shows up in my `mine` as claimed, and stays there when I lose the role.
#[tokio::test]
async fn a_claim_appears_in_mine_and_survives_losing_the_role() {
    let w = world("p2_roundtrip").await;
    let mut admin = w.admin().await;
    let id = Seed::pending(Who::L).insert(&mut admin).await;
    assert_eq!(
        ids(&w.get(Who::W1, "/v2/handoffs?mine=true").await.json),
        [id.to_string()]
    );
    assert_eq!(w.claim(Who::W1, id).await.status.as_u16(), 200);
    let mine = w.get(Who::W1, "/v2/handoffs?mine=true").await;
    assert_eq!(ids(&mine.json), [id.to_string()]);
    assert_eq!(mine.json["items"][0]["state"], "claimed");
    assert!(
        ids(&w.get(Who::W2, "/v2/handoffs?mine=true").await.json).is_empty(),
        "W2 no longer sees it"
    );
    // W1 loses the role `dev`; the claimed row still resurfaces for it (the claimed-by-me clause stands alone).
    sqlx::query("UPDATE plug_identity.workers SET job_roles = '{}' WHERE id = $1::uuid")
        .bind(Who::W1.id().to_string())
        .execute(&mut admin)
        .await
        .expect("remove the role");
    assert_eq!(
        ids(&w.get(Who::W1, "/v2/handoffs?mine=true").await.json),
        [id.to_string()]
    );
}

/// HAND-20, HAND-21, HAND-55: `mine` and `claim` agree for every handoff and every worker.
#[tokio::test]
async fn mine_and_claim_use_one_addressing_predicate() {
    let w = world("p2_parity").await;
    let mut admin = w.admin().await;
    type Make = fn() -> Seed;
    let cases: Vec<(&str, Make)> = vec![
        ("to W1", || {
            Seed::pending(Who::L).addressed_to_worker(Who::W1)
        }),
        ("to W3", || {
            Seed::pending(Who::L).addressed_to_worker(Who::W3)
        }),
        ("to role dev", || Seed::pending(Who::L)),
        ("to role qa", || {
            Seed::pending(Who::L).addressed_to_role("qa")
        }),
        ("to role Dev (no one)", || {
            Seed::pending(Who::L).addressed_to_role("nobody")
        }),
        ("to retired W5", || {
            Seed::pending(Who::L).addressed_to_worker(Who::W5)
        }),
        ("dev, reworked to W2", || {
            let mut s = Seed::pending(Who::L);
            s.round = 2;
            s.rework_holder = Some(Who::W2);
            s
        }),
    ];
    let mut cells = 0;
    for (name, make) in &cases {
        for who in [Who::L, Who::TL, Who::UL, Who::W1, Who::W2, Who::W3, Who::W4] {
            let id = make().insert(&mut admin).await;
            let listed =
                ids(&w.get(who, "/v2/handoffs?mine=true").await.json).contains(&id.to_string());
            let claim = w.claim(who, id).await;
            let claimed = claim.status.as_u16() == 200;
            assert_eq!(
                listed, claimed,
                "{name} / {who:?}: in mine = {listed}, claim = {} ({})",
                claim.status, claim.text
            );
            cells += 1;
        }
    }
    assert_eq!(cells, 49);
}

// ---- claim (HAND-30 to HAND-33) ----

/// HAND-32: claim effects, in the table, the log and the audit.
#[tokio::test]
async fn claim_moves_the_handoff_and_leaves_one_row_in_the_log_and_the_audit() {
    let w = world("p2_claim").await;
    let mut admin = w.admin().await;
    let id = Seed::pending(Who::L).insert(&mut admin).await;
    let a = w.claim(Who::W1, id).await;
    assert_eq!(a.status.as_u16(), 200, "{}", a.text);
    assert_eq!(
        a.json,
        json!({ "id": id.to_string(), "state": "claimed", "round": 1 })
    );

    let (state, holder, incarnation, seq): (String, String, i64, i32) = sqlx::query_as(
        "SELECT state, holder::text, incarnation, transition_seq FROM handoffs.handoffs WHERE id = $1::uuid",
    )
    .bind(id.to_string())
    .fetch_one(&mut admin)
    .await
    .expect("the row");
    assert_eq!(
        (state.as_str(), holder, incarnation, seq),
        ("claimed", Who::W1.id().to_string(), 1, 2)
    );
    let t: (i32, String, String, String, String) = sqlx::query_as(
        "SELECT seq, from_state, to_state, actor_kind, actor_id FROM handoffs.transitions WHERE handoff_id = $1::uuid AND seq = 2",
    )
    .bind(id.to_string())
    .fetch_one(&mut admin)
    .await
    .expect("the log row");
    assert_eq!(
        (t.0, t.1.as_str(), t.2.as_str(), t.3.as_str(), t.4),
        (2, "pending", "claimed", "worker", Who::W1.id().to_string())
    );
    let audit: Vec<(String, String)> =
        sqlx::query_as("SELECT action, target FROM fx_audit.events ORDER BY id")
            .fetch_all(&mut admin)
            .await
            .expect("audit");
    assert_eq!(
        audit,
        [("handoff.claimed".to_owned(), format!("handoff:{id}"))]
    );
}

/// HAND-22, HAND-31, HAND-33: who may not claim, and what state refuses.
#[tokio::test]
async fn claim_refusals() {
    let w = world("p2_claimrefuse").await;
    let mut admin = w.admin().await;
    let pending = Seed::pending(Who::L).insert(&mut admin).await;
    let held = Seed::pending(Who::L)
        .claimed_by(Who::W1)
        .insert(&mut admin)
        .await;
    let mut returned = Seed::pending(Who::L).claimed_by(Who::W1);
    returned.state = "returned";
    let returned = returned.insert(&mut admin).await;
    let mut accepted = Seed::pending(Who::L).claimed_by(Who::W1);
    accepted.state = "accepted";
    accepted.closed_at = Some("2026-11-02 00:00:00+00".to_owned());
    let accepted = accepted.insert(&mut admin).await;
    let mut cancelled = Seed::pending(Who::L);
    cancelled.state = "cancelled";
    cancelled.closed_at = Some("2026-11-02 00:00:00+00".to_owned());
    cancelled.close_reason = Some("no longer needed".to_owned());
    let cancelled = cancelled.insert(&mut admin).await;
    let missing = HandoffId::new(&cortex_types::SystemClock);

    let code = |a: &common::Answer| {
        (
            a.status.as_u16(),
            a.json["type"].as_str().map(str::to_owned),
        )
    };
    // Not an addressee (no role `dev`): 403, and the handoff is unchanged.
    assert_eq!(
        code(&w.claim(Who::W4, pending).await),
        (403, Some("target_not_allowed".into()))
    );
    assert_eq!(
        code(&w.claim(Who::L, pending).await),
        (403, Some("target_not_allowed".into()))
    );
    // Another project's, and a missing id: the same 404.
    assert_eq!(
        code(&w.claim(Who::B2, pending).await),
        (404, Some("not_found".into()))
    );
    assert_eq!(
        code(&w.claim(Who::W1, missing).await),
        (404, Some("not_found".into()))
    );
    // A claimed one: already_claimed, for another worker and for the holder itself.
    assert_eq!(
        code(&w.claim(Who::W2, held).await),
        (409, Some("already_claimed".into()))
    );
    assert_eq!(
        code(&w.claim(Who::W1, held).await),
        (409, Some("already_claimed".into()))
    );
    // The other states: invalid_transition.
    for (name, id) in [
        ("returned", returned),
        ("accepted", accepted),
        ("cancelled", cancelled),
    ] {
        assert_eq!(
            code(&w.claim(Who::W1, id).await),
            (409, Some("invalid_transition".into())),
            "{name}"
        );
    }
    let still_pending: i64 = w
        .count("SELECT count(*) FROM handoffs.handoffs WHERE state = 'pending' AND holder IS NULL")
        .await;
    assert_eq!(still_pending, 1, "the refusals changed nothing");
    assert_eq!(
        w.count("SELECT count(*) FROM fx_audit.events").await,
        0,
        "a refusal is never audited"
    );
    assert_eq!(
        w.count("SELECT count(*) FROM handoffs.transitions WHERE seq > 1")
            .await,
        0
    );
}

/// HAND-30: eight claims at once, one winner, seven `already_claimed`, no 429, 50 rounds in 10 here.
#[tokio::test]
async fn claim_has_one_winner_under_concurrency() {
    let w = world("p2_race").await;
    let mut admin = w.admin().await;
    for round in 0..10 {
        let id = Seed::pending(Who::L).insert(&mut admin).await;
        let mut set = tokio::task::JoinSet::new();
        for i in 0..8 {
            let world_app = w.app.clone();
            let who = if i % 2 == 0 { Who::W1 } else { Who::W2 };
            let path = format!("/v2/handoffs/{id}/claim");
            set.spawn(async move {
                use tower::ServiceExt;
                let req = axum::extract::Request::builder()
                    .method(Method::POST)
                    .uri(path)
                    .header("authorization", format!("Bearer {}", who.token()))
                    .header("idempotency-key", common::key())
                    .body(axum::body::Body::empty())
                    .expect("a request");
                let resp = world_app.oneshot(req).await.expect("a response");
                let status = resp.status().as_u16();
                let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
                    .await
                    .expect("a body");
                let json: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
                (who, status, json["type"].as_str().map(str::to_owned))
            });
        }
        let mut wins = Vec::new();
        let mut losses = 0;
        while let Some(r) = set.join_next().await {
            let (who, status, ty) = r.expect("a task");
            match (status, ty.as_deref()) {
                (200, _) => wins.push(who),
                (409, Some("already_claimed")) => losses += 1,
                other => panic!("round {round}: neither a win nor already_claimed: {other:?}"),
            }
        }
        assert_eq!((wins.len(), losses), (1, 7), "round {round}");
        let holder: String =
            sqlx::query_scalar("SELECT holder::text FROM handoffs.handoffs WHERE id = $1::uuid")
                .bind(id.to_string())
                .fetch_one(&mut admin)
                .await
                .expect("holder");
        assert_eq!(holder, wins[0].id().to_string());
        let moves: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM handoffs.transitions WHERE handoff_id = $1::uuid AND to_state = 'claimed'",
        )
        .bind(id.to_string())
        .fetch_one(&mut admin)
        .await
        .expect("count");
        assert_eq!(moves, 1, "exactly one claim transition");
    }
}
