//! 2b, through the real chain: `POST /v2/workers/{id}/retire`.
//! Rows: ID-17, ID-17b, ID-18, ID-19 (history), ID-51 (no reason in audit), ID-53, ID-54 (order).
//! Needs `supervise.sh`'s throwaway Postgres. RED: `authorize` refuses with a 500.
mod common;

use axum::http::Method;
use common::chain::{A_LEAD, A_M1, A_TL, B_LEAD, World, start};
use common::seed::{add_member, add_team, add_worker};
use cortex_testkit::within;
use cortex_types::{SystemClock, TeamId, WorkerId};
use serde_json::json;
use sqlx::Connection;

fn path(w: WorkerId) -> String {
    format!("/v2/workers/{w}/retire")
}

async fn footprint(w: &World) -> (i64, i64, usize) {
    (
        w.count("SELECT count(*) FROM identity.workers WHERE state = 'retired'")
            .await,
        w.count("SELECT count(*) FROM identity.team_members").await,
        w.audit().await.len(),
    )
}

/// Team Ops (t2) with a member `erin`, and a fresh member `frank` of team t1, as the superuser.
async fn extra(w: &World) -> (TeamId, WorkerId, WorkerId) {
    let t2 = TeamId::new(&SystemClock);
    let erin = WorkerId::new(&SystemClock);
    let frank = WorkerId::new(&SystemClock);
    let mut admin = w.db.admin().await;
    let mut tx = admin.begin().await.expect("tx");
    add_team(&mut tx, t2, w.f.a, "Ops").await;
    add_worker(&mut tx, erin, w.f.a, "erin", "worker", false, &[]).await;
    add_worker(&mut tx, frank, w.f.a, "frank", "worker", false, &[]).await;
    add_member(&mut tx, erin, t2, w.f.a, "member").await;
    add_member(&mut tx, frank, w.f.t1, w.f.a, "member").await;
    tx.commit().await.expect("commit");
    (t2, erin, frank)
}

#[tokio::test]
async fn id_17_the_lead_retires_a_member_and_nothing_but_codes_reach_the_audit() {
    within(120, async {
        let w = start("mi_r_ok").await;
        let reason = "token sk-test-0123456789 left the team";
        let before = footprint(&w).await;
        let a = w
            .post(A_LEAD, &path(w.f.a_m1), json!({"reason": reason}))
            .await;
        assert_eq!(a.status.as_u16(), 200, "{}", a.text);
        assert_eq!(a.json, json!({"ok": true}));
        let (state, retired_at, stored, perm_gen): (String, Option<String>, Option<String>, i64) = {
            let mut admin = w.db.admin().await;
            sqlx::query_as("SELECT state, retired_at::text, retired_reason, perm_gen FROM identity.workers WHERE id = $1::uuid")
                .bind(w.f.a_m1.to_string())
                .fetch_one(&mut admin)
                .await
                .expect("row")
        };
        assert_eq!(state, "retired");
        assert!(retired_at.is_some());
        assert_eq!(stored.as_deref(), Some(reason), "the reason is stored byte for byte");
        let _ = perm_gen;
        // History survives (ID-19): the name row is current, the membership row is kept.
        assert_eq!(
            w.count("SELECT count(*) FROM identity.names WHERE entity_kind = 'worker' AND current AND label = 'Bob'").await,
            1
        );
        let after = footprint(&w).await;
        assert_eq!((after.0, after.1, after.2), (before.0 + 1, before.1, before.2 + 1));
        let rows = w.audit().await;
        let last = rows.last().expect("audit row");
        assert_eq!(
            (last.0.as_str(), last.2.as_str(), last.3.as_str()),
            ("worker.retired", format!("worker:{}", w.f.a_m1).as_str(), "{}")
        );
        // The reason text is in workers.retired_reason and in no audit column (ID-51).
        for (a, ac, t, d) in &rows {
            for field in [a, ac, t, d] {
                assert!(!field.contains("sk-test") && !field.contains("left the team"), "{field}");
            }
        }
        // The roster no longer lists the worker; a second retire is 409 target_retired.
        let list = w.get(Some(A_LEAD), "/v2/workers?name=bob").await;
        assert_eq!(list.json["items"].as_array().map(Vec::len), Some(0));
        let again = w.post(A_LEAD, &path(w.f.a_m1), json!({"reason": "again"})).await;
        assert_eq!(again.code(), (409, Some("target_retired")), "{}", again.text);
        assert_eq!(footprint(&w).await, after, "a refusal changes nothing and leaves no audit row");
    })
    .await
}

#[tokio::test]
async fn id_17b_the_reason_is_checked_first_and_stored_exactly() {
    within(120, async {
        let w = start("mi_r_reason").await;
        let (_, _, frank) = extra(&w).await;
        let before = footprint(&w).await;
        let unknown = WorkerId::new(&SystemClock);
        // Absent, empty, whitespace-only: 422 reason_required even for an unknown id, the lead, a
        // retired worker (step 0 comes before 404, 409 and 403).
        for target in [unknown, w.f.a_lead, w.f.a_ret, frank] {
            for body in [
                json!({}),
                json!({"reason": ""}),
                json!({"reason": "   \t\n"}),
            ] {
                let r = w.post(A_LEAD, &path(target), body.clone()).await;
                assert_eq!(
                    r.code(),
                    (422, Some("reason_required")),
                    "{target} {body}: {}",
                    r.text
                );
            }
        }
        // Over 512 bytes, a NUL, and bodies that are not an object: 422 invalid_request.
        let big = "x".repeat(513);
        for body in [
            json!({"reason": big}),
            json!({"reason": "a\u{0}b"}),
            json!([]),
            json!("x"),
            json!({"reason": 5}),
            json!({"reason": "ok", "extra": 1}),
        ] {
            let r = w.post(A_LEAD, &path(frank), body.clone()).await;
            assert_eq!(
                r.code(),
                (422, Some("invalid_request")),
                "{body}: {}",
                r.text
            );
        }
        let r = w
            .call(Method::POST, &path(frank), Some(A_LEAD), None, Some(""))
            .await;
        assert_eq!(
            r.code(),
            (422, Some("invalid_request")),
            "no body: {}",
            r.text
        );
        assert_eq!(footprint(&w).await, before);
        // 512 bytes exactly is accepted, and the text is kept as sent (whitespace included).
        let exact = format!("  {}  ", "é".repeat(254)); // 2 + 508 + 2 = 512 bytes
        assert_eq!(exact.len(), 512);
        let ok = w.post(A_LEAD, &path(frank), json!({"reason": exact})).await;
        assert_eq!(ok.status.as_u16(), 200, "{}", ok.text);
        let stored: String = {
            let mut admin = w.db.admin().await;
            sqlx::query_scalar("SELECT retired_reason FROM identity.workers WHERE id = $1::uuid")
                .bind(frank.to_string())
                .fetch_one(&mut admin)
                .await
                .expect("reason")
        };
        assert_eq!(stored, exact);
        // ID-53: a body over 16384 bytes is 413.
        let huge = format!(r#"{{"reason":"{}"}}"#, "a".repeat(16400));
        let r = w
            .call(
                Method::POST,
                &path(w.f.a_m2),
                Some(A_LEAD),
                None,
                Some(&huge),
            )
            .await;
        assert_eq!(r.code(), (413, Some("body_too_large")));
    })
    .await
}

#[tokio::test]
async fn id_18_who_may_retire_whom_and_the_order_of_refusals() {
    within(120, async {
        let w = start("mi_r_rules").await;
        let (_t2, erin, frank) = extra(&w).await;
        let reason = json!({"reason": "because"});
        let before = footprint(&w).await;
        // The lead retiring the lead (itself or by id): 409 lead_permanent, for any caller with the scope.
        for token in [A_LEAD, A_TL] {
            let r = w.post(token, &path(w.f.a_lead), reason.clone()).await;
            assert_eq!(r.code(), (409, Some("lead_permanent")), "{token}: {}", r.text);
        }
        // A team lead: another team's member, a worker in no team, itself: 403 target_not_allowed.
        for target in [erin, w.f.a_m2, w.f.a_tl] {
            let r = w.post(A_TL, &path(target), reason.clone()).await;
            assert_eq!(r.code(), (403, Some("target_not_allowed")), "{target}: {}", r.text);
        }
        // A member: 403 missing_scope.
        let r = w.post(A_M1, &path(frank), reason.clone()).await;
        assert_eq!(r.code(), (403, Some("missing_scope")), "{}", r.text);
        // A malformed, an unknown and another project's id: 404, one body.
        let bad = w.post(A_LEAD, "/v2/workers/abc/retire", reason.clone()).await;
        let unknown = w.post(A_LEAD, &path(WorkerId::new(&SystemClock)), reason.clone()).await;
        let foreign = w.post(A_LEAD, &path(w.f.b_worker), reason.clone()).await;
        for r in [&bad, &unknown, &foreign] {
            assert_eq!(r.code(), (404, Some("not_found")), "{}", r.text);
        }
        assert_eq!(bad.body_without_request_id(), unknown.body_without_request_id());
        assert_eq!(bad.body_without_request_id(), foreign.body_without_request_id());
        assert_eq!(footprint(&w).await, before, "no refusal wrote anything");
        // A team lead retiring an own-team member: 200. The lead retiring any member: 200.
        let ok = w.post(A_TL, &path(frank), reason.clone()).await;
        assert_eq!(ok.status.as_u16(), 200, "{}", ok.text);
        let ok = w.post(A_LEAD, &path(erin), reason.clone()).await;
        assert_eq!(ok.status.as_u16(), 200, "{}", ok.text);
        // B's lead cannot reach A's worker.
        let r = w.post(B_LEAD, &path(w.f.a_m2), reason.clone()).await;
        assert_eq!(r.code(), (404, Some("not_found")));
        // The retired worker's membership row is kept (ID-19).
        assert_eq!(
            w.count("SELECT count(*) FROM identity.team_members WHERE worker_id IN (SELECT id FROM identity.workers WHERE state = 'retired')").await,
            3,
            "dave (seed), frank and erin keep their membership rows"
        );
    })
    .await
}

#[tokio::test]
async fn id_16_a_fault_in_the_audit_sink_leaves_the_worker_active() {
    within(120, async {
        let w = start("mi_r_atomic").await;
        w.fault.refuse("worker.retired");
        let before = footprint(&w).await;
        let r = w
            .post(A_LEAD, &path(w.f.a_m1), json!({"reason": "x"}))
            .await;
        assert_eq!(r.code(), (503, Some("unavailable")), "{}", r.text);
        assert_eq!(footprint(&w).await, before);
        let state: String = {
            let mut admin = w.db.admin().await;
            sqlx::query_scalar("SELECT state FROM identity.workers WHERE id = $1::uuid")
                .bind(w.f.a_m1.to_string())
                .fetch_one(&mut admin)
                .await
                .expect("state")
        };
        assert_eq!(state, "active");
    })
    .await
}

/// PO-17 for the identity writes: the rule each route declares and the Target its handler builds.
#[tokio::test]
async fn po_17_spy_the_target_each_identity_handler_builds() {
    use cortex_module::{Target, TargetRule, WorkerKind};
    within(120, async {
        let w = start("mi_po17").await;
        let (_t2, erin, frank) = extra(&w).await;
        let asked = |w: &World| w.asked.lock().expect("asked lock").clone();
        // Create routes: rule None, the empty Target.
        w.post(A_LEAD, "/v2/teams", json!({"name": "Spied"})).await;
        w.post(
            A_LEAD,
            "/v2/workers",
            json!({"name": "Spied", "kind": "worker"}),
        )
        .await;
        assert_eq!(
            asked(&w),
            vec![(w.f.a_lead, TargetRule::None, Target::default()); 2],
            "both creates ask rule None with an empty Target"
        );
        // Retire a member of t1: RetireTarget with worker, kind, the target's team and the lead.
        w.post(A_LEAD, &path(frank), json!({"reason": "x"})).await;
        let last = asked(&w).pop().expect("asked");
        assert_eq!(
            last,
            (
                w.f.a_lead,
                TargetRule::RetireTarget,
                Target {
                    worker: Some(frank),
                    kind: Some(WorkerKind::Worker),
                    team: Some(w.f.t1),
                    lead: Some(w.f.a_lead),
                    ..Target::default()
                }
            )
        );
        // A worker in no team: team is None. A service is shown as a service.
        w.post(A_LEAD, &path(w.f.a_m2), json!({"reason": "x"}))
            .await;
        let last = asked(&w).pop().expect("asked");
        assert_eq!(
            (last.1, last.2.team, last.2.worker),
            (TargetRule::RetireTarget, None, Some(w.f.a_m2))
        );
        w.post(A_LEAD, &path(w.f.a_svc), json!({"reason": "x"}))
            .await;
        assert_eq!(
            asked(&w).pop().expect("asked").2.kind,
            Some(WorkerKind::Service)
        );
        // Assign: NotProjectLead with the path worker and the lead.
        w.put(
            A_LEAD,
            &format!("/v2/workers/{erin}/team"),
            json!({"team": w.f.t1.to_string(), "role": "member"}),
        )
        .await;
        let last = asked(&w).pop().expect("asked");
        assert_eq!(
            last,
            (
                w.f.a_lead,
                TargetRule::NotProjectLead,
                Target {
                    worker: Some(erin),
                    kind: Some(WorkerKind::Worker),
                    team: None,
                    lead: Some(w.f.a_lead),
                    ..Target::default()
                }
            )
        );
        // The policy is NOT asked when the 404 or the lead_permanent 409 comes first (R-16 order).
        let n = asked(&w).len();
        w.post(
            A_LEAD,
            &path(WorkerId::new(&SystemClock)),
            json!({"reason": "x"}),
        )
        .await;
        w.post(A_LEAD, &path(w.f.a_lead), json!({"reason": "x"}))
            .await;
        w.put(
            A_LEAD,
            &format!("/v2/workers/{}/team", WorkerId::new(&SystemClock)),
            json!({"team": w.f.t1.to_string(), "role": "member"}),
        )
        .await;
        assert_eq!(
            asked(&w).len(),
            n,
            "no target check before the 404 and the 409 lead_permanent"
        );
    })
    .await
}

/// ID2B-01 (sam): the retire authorization was made on a team read BEFORE the worker lock. A move
/// committed in between must make the retire a 403, not retire a worker the team lead no longer leads.
#[tokio::test]
async fn id_18_a_move_committed_between_authorize_and_execute_refuses_the_team_leads_retire() {
    within(120, async {
        let w = start("mi_r_stale").await;
        let (t2, _erin, frank) = extra(&w).await; // frank: a member of t1, which A_TL leads
        {
            // Hold the worker row for 0.5 s inside the assign's UPDATE, so the retire's authorize runs
            // while the move is NOT yet committed and its execute then waits on the row lock.
            let mut admin = w.db.admin().await;
            sqlx::raw_sql(
                "CREATE FUNCTION identity.slow() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_sleep(0.5); RETURN NEW; END $$; \
                 CREATE TRIGGER slow BEFORE UPDATE ON identity.workers FOR EACH ROW EXECUTE FUNCTION identity.slow();",
            )
            .execute(&mut admin)
            .await
            .expect("slow trigger");
        }
        let assign_path = format!("/v2/workers/{frank}/team");
        let retire_path = path(frank);
        let rows0 = w.audit().await.len();
        let (assign, retire) = tokio::join!(
            w.put(A_LEAD, &assign_path, json!({"team": t2.to_string(), "role": "member"})),
            async {
                tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                w.post(A_TL, &retire_path, json!({"reason": "stale"})).await
            }
        );
        assert_eq!(assign.status.as_u16(), 200, "{}", assign.text);
        assert_eq!(retire.code(), (403, Some("target_not_allowed")), "{}", retire.text);
        let state: String = {
            let mut admin = w.db.admin().await;
            sqlx::query_scalar("SELECT state FROM identity.workers WHERE id = $1::uuid")
                .bind(frank.to_string())
                .fetch_one(&mut admin)
                .await
                .expect("state")
        };
        assert_eq!(state, "active", "the worker was moved to another team and must not be retired by the old team's lead");
        let rows = w.audit().await;
        assert_eq!(rows.len(), rows0 + 1, "only the assign left a record");
        assert_eq!(rows.last().map(|r| r.0.as_str()), Some("worker.team_assigned"));
    })
    .await
}

/// ID2B-03 (sam): the byte limit comes before the requiredness (length, NUL, then the own code).
#[tokio::test]
async fn id_17b_the_reason_length_is_checked_before_it_is_required() {
    within(120, async {
        let w = start("mi_r_reason_order").await;
        let before = footprint(&w).await;
        let target = path(w.f.a_m1);
        for reason in [
            " ".repeat(513),
            "\t".repeat(513),
            format!("{}\n", " ".repeat(512)),
            "x".repeat(513),
        ] {
            let r = w
                .post(A_LEAD, &target, json!({"reason": reason.clone()}))
                .await;
            assert_eq!(
                r.code(),
                (422, Some("invalid_request")),
                "{} bytes: {}",
                reason.len(),
                r.text
            );
        }
        // At the limit, whitespace-only is still "required".
        for reason in [" ".repeat(512), "\t".repeat(512)] {
            let r = w
                .post(A_LEAD, &target, json!({"reason": reason.clone()}))
                .await;
            assert_eq!(
                r.code(),
                (422, Some("reason_required")),
                "{} bytes: {}",
                reason.len(),
                r.text
            );
        }
        // ID2B-02: an explicit null is not an absent reason; it is a decoding error.
        let r = w.post(A_LEAD, &target, json!({"reason": null})).await;
        assert_eq!(r.code(), (422, Some("invalid_request")), "{}", r.text);
        assert_eq!(footprint(&w).await, before);
    })
    .await
}

// ---- W1-51 (cox 7 Nov 12:56:50Z): a worker who leads a team cannot be retired while leading --------
// Refusal order on a retire: 422, 404, 409 lead_permanent, 403 target, 409 target_retired, 409
// team_lead_in_post. Cells: id_20 (the refusal), id_20b (the fix text works), id_20c (the order),
// id_20d (a promotion committed between authorize and execute).

const IN_POST_TITLE: &str = "A team lead cannot be retired";
const IN_POST_DETAIL: &str = "That worker leads a team.";
const IN_POST_FIX: &str =
    "Move the worker to another team or make the worker a member, then retire.";

fn assign_path(w: WorkerId) -> String {
    format!("/v2/workers/{w}/team")
}

/// The worker's membership as the superuser reads it: (team, role), or None.
async fn membership(w: &World, id: WorkerId) -> Option<(String, String)> {
    let mut admin = w.db.admin().await;
    sqlx::query_as(
        "SELECT team_id::text, role FROM identity.team_members WHERE worker_id = $1::uuid",
    )
    .bind(id.to_string())
    .fetch_optional(&mut admin)
    .await
    .expect("membership")
}

/// (state, retired_at is set, retired_reason) of one worker, as the superuser reads it.
async fn life(w: &World, id: WorkerId) -> (String, bool, Option<String>) {
    let mut admin = w.db.admin().await;
    sqlx::query_as(
        "SELECT state, retired_at IS NOT NULL, retired_reason FROM identity.workers WHERE id = $1::uuid",
    )
    .bind(id.to_string())
    .fetch_one(&mut admin)
    .await
    .expect("life")
}

#[tokio::test]
async fn id_20_a_lead_in_post_is_refused_and_nothing_changes() {
    within(120, async {
        let w = start("mi_r_inpost").await;
        let before = footprint(&w).await;
        let audit0 = w.audit().await;
        let r = w
            .post(A_LEAD, &path(w.f.a_tl), json!({"reason": "moving on"}))
            .await;
        assert_eq!(r.code(), (409, Some("team_lead_in_post")), "{}", r.text);
        assert_eq!(r.json["status"], 409, "{}", r.text);
        assert_eq!(r.json["title"], IN_POST_TITLE, "{}", r.text);
        assert_eq!(r.json["fix"], IN_POST_FIX, "{}", r.text);
        // A fixed text: no request value and no row data in the detail.
        assert_eq!(r.json["detail"], IN_POST_DETAIL, "{}", r.text);
        // Nothing changes: the worker is active, still the team lead, no retire time or reason, no
        // audit row (the whole audit list is equal, not only its length), no membership moved.
        assert_eq!(life(&w, w.f.a_tl).await, ("active".to_owned(), false, None));
        assert_eq!(
            membership(&w, w.f.a_tl).await,
            Some((w.f.t1.to_string(), "team_lead".to_owned()))
        );
        assert_eq!(footprint(&w).await, before);
        assert_eq!(w.audit().await, audit0, "a refusal leaves no audit row");
        // The refusal is the same on a second try (no state was recorded by the first).
        let again = w
            .post(A_LEAD, &path(w.f.a_tl), json!({"reason": "moving on"}))
            .await;
        assert_eq!(
            again.code(),
            (409, Some("team_lead_in_post")),
            "{}",
            again.text
        );
        assert_eq!(footprint(&w).await, before);
    })
    .await
}

#[tokio::test]
async fn id_20b_move_or_demote_first_then_the_retire_works_and_the_slot_is_free() {
    within(120, async {
        let w = start("mi_r_fixtext").await;
        let (t2, _erin, frank) = extra(&w).await; // frank: a member of t1; t2 has a member and no lead
        let reason = json!({"reason": "left"});
        // The fix text, first half: demote the lead of t1 to a member, then retire it.
        let d = w
            .put(
                A_LEAD,
                &assign_path(w.f.a_tl),
                json!({"team": w.f.t1.to_string(), "role": "member"}),
            )
            .await;
        assert_eq!(d.status.as_u16(), 200, "{}", d.text);
        let r = w.post(A_LEAD, &path(w.f.a_tl), reason.clone()).await;
        assert_eq!(r.status.as_u16(), 200, "{}", r.text);
        assert_eq!(life(&w, w.f.a_tl).await.0, "retired");
        // The slot of t1 is free: a new team lead can be assigned (the trap W1-51 closes is a slot
        // that stays occupied by a worker nobody can move any more).
        let p = w
            .put(
                A_LEAD,
                &assign_path(frank),
                json!({"team": w.f.t1.to_string(), "role": "team_lead"}),
            )
            .await;
        assert_eq!(p.status.as_u16(), 200, "{}", p.text);
        // The fix text, second half: move the lead to another team (as a member), then retire it.
        let m = w
            .put(
                A_LEAD,
                &assign_path(frank),
                json!({"team": t2.to_string(), "role": "member"}),
            )
            .await;
        assert_eq!(m.status.as_u16(), 200, "{}", m.text);
        let r = w.post(A_LEAD, &path(frank), reason.clone()).await;
        assert_eq!(r.status.as_u16(), 200, "{}", r.text);
        assert_eq!(life(&w, frank).await.0, "retired");
        // t1 has no lead again, and a member can take the slot.
        let p = w
            .put(
                A_LEAD,
                &assign_path(w.f.a_m1),
                json!({"team": w.f.t1.to_string(), "role": "team_lead"}),
            )
            .await;
        assert_eq!(p.status.as_u16(), 200, "{}", p.text);
        assert_eq!(
            w.count("SELECT count(*) FROM identity.team_members WHERE role = 'team_lead'")
                .await,
            1,
            "one team lead in all: Bob leads t1"
        );
    })
    .await
}

#[tokio::test]
async fn id_20c_the_refusal_comes_after_the_target_authority_and_the_other_refusals() {
    within(120, async {
        let w = start("mi_r_inpost_order").await;
        // t2 is led by gina; t4 was led by hank, who is RETIRED (data from before W1-51: a retired
        // worker that still holds a lead row). The project lead cannot also lead a team: the schema
        // refuses it (trigger `refuse_lead_in_team`), so lead_permanent and team_lead_in_post can
        // never apply to one worker and their relative order cannot be observed.
        let (gina, hank) = {
            let (t2, t4) = (TeamId::new(&SystemClock), TeamId::new(&SystemClock));
            let (gina, hank) = (WorkerId::new(&SystemClock), WorkerId::new(&SystemClock));
            let mut admin = w.db.admin().await;
            let mut tx = admin.begin().await.expect("tx");
            add_team(&mut tx, t2, w.f.a, "Ops").await;
            add_team(&mut tx, t4, w.f.a, "Legacy").await;
            add_worker(&mut tx, gina, w.f.a, "gina", "worker", false, &[]).await;
            add_worker(&mut tx, hank, w.f.a, "hank", "worker", true, &[]).await;
            add_member(&mut tx, gina, t2, w.f.a, "team_lead").await;
            add_member(&mut tx, hank, t4, w.f.a, "team_lead").await;
            tx.commit().await.expect("commit");
            (gina, hank)
        };
        let reason = json!({"reason": "because"});
        let before = footprint(&w).await;
        let audit0 = w.audit().await;
        // A caller without authority over the target learns nothing about who leads a team:
        // a team lead of t1 retiring the lead of t2 and itself (the lead of t1): 403, never 409.
        for target in [gina, w.f.a_tl] {
            let r = w.post(A_TL, &path(target), reason.clone()).await;
            assert_eq!(
                r.code(),
                (403, Some("target_not_allowed")),
                "{target}: {}",
                r.text
            );
        }
        // A member has no scope: 403 missing_scope, for a lead in post as for any target.
        let r = w.post(A_M1, &path(w.f.a_tl), reason.clone()).await;
        assert_eq!(r.code(), (403, Some("missing_scope")), "{}", r.text);
        // 404 comes first: an unknown id and another project's id give the one body.
        let unknown = w
            .post(A_LEAD, &path(WorkerId::new(&SystemClock)), reason.clone())
            .await;
        let foreign = w.post(A_LEAD, &path(w.f.b_worker), reason.clone()).await;
        for r in [&unknown, &foreign] {
            assert_eq!(r.code(), (404, Some("not_found")), "{}", r.text);
        }
        assert_eq!(
            unknown.body_without_request_id(),
            foreign.body_without_request_id()
        );
        // The project lead: lead_permanent (it is in no team, so never team_lead_in_post).
        let r = w.post(A_LEAD, &path(w.f.a_lead), reason.clone()).await;
        assert_eq!(r.code(), (409, Some("lead_permanent")), "{}", r.text);
        // A retired worker that still holds a lead row: target_retired, not team_lead_in_post.
        let r = w.post(A_LEAD, &path(hank), reason.clone()).await;
        assert_eq!(r.code(), (409, Some("target_retired")), "{}", r.text);
        // 422 comes before all of them: no reason, for a lead in post.
        let r = w.post(A_LEAD, &path(gina), json!({})).await;
        assert_eq!(r.code(), (422, Some("reason_required")), "{}", r.text);
        assert_eq!(footprint(&w).await, before);
        assert_eq!(w.audit().await, audit0, "no refusal left an audit row");
        // And with the authority, the lead in post of another team is refused (409), not retired.
        let r = w.post(A_LEAD, &path(gina), reason.clone()).await;
        assert_eq!(r.code(), (409, Some("team_lead_in_post")), "{}", r.text);
        assert_eq!(footprint(&w).await, before);
        assert_eq!(w.audit().await, audit0);
    })
    .await
}

/// The race: a promotion to team lead committed between the retire's authorize (the worker is still
/// a plain member) and its execute (the worker row lock is then granted) must make the retire a 409.
/// Same technique as `id_18_a_move_committed_between_authorize_and_execute_refuses_the_team_leads_retire`.
#[tokio::test]
async fn id_20d_a_promotion_committed_between_authorize_and_execute_is_refused() {
    within(120, async {
        let w = start("mi_r_promoted").await;
        let (t2, _erin, frank) = extra(&w).await; // frank: a member of t1; t2 has no lead
        {
            // Hold the worker row for 0.5 s inside the assign's UPDATE (it always updates `workers`:
            // perm_gen rises on a membership change), so the retire's authorize runs while the
            // promotion is NOT yet committed and its execute then waits on the row lock.
            let mut admin = w.db.admin().await;
            sqlx::raw_sql(
                "CREATE FUNCTION identity.slow() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_sleep(0.5); RETURN NEW; END $$; \
                 CREATE TRIGGER slow BEFORE UPDATE ON identity.workers FOR EACH ROW EXECUTE FUNCTION identity.slow();",
            )
            .execute(&mut admin)
            .await
            .expect("slow trigger");
        }
        let (assign_to, retire_to) = (assign_path(frank), path(frank));
        let rows0 = w.audit().await.len();
        let (assign, retire) = tokio::join!(
            w.put(A_LEAD, &assign_to, json!({"team": t2.to_string(), "role": "team_lead"})),
            async {
                tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                // The project lead retires: its authority over the target does not depend on a team.
                w.post(A_LEAD, &retire_to, json!({"reason": "stale"})).await
            }
        );
        assert_eq!(assign.status.as_u16(), 200, "{}", assign.text);
        assert_eq!(retire.code(), (409, Some("team_lead_in_post")), "{}", retire.text);
        assert_eq!(life(&w, frank).await, ("active".to_owned(), false, None));
        assert_eq!(
            membership(&w, frank).await,
            Some((t2.to_string(), "team_lead".to_owned())),
            "the promotion stands: the worker leads t2"
        );
        let rows = w.audit().await;
        assert_eq!(rows.len(), rows0 + 1, "only the assign left a record");
        assert_eq!(rows.last().map(|r| r.0.as_str()), Some("worker.team_assigned"));
    })
    .await
}
