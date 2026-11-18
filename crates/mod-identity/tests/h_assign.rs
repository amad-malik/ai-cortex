//! 2b, through the real chain: `PUT /v2/workers/{id}/team`.
//! Rows: ID-31, ID-31b (the no-op and the job-role change), ID-32 (one team lead per team, also
//! under a race), ID-33 (rules and order), ID-23 (`perm_gen`), ID-51 (audit details), ID-53.
//! Needs `supervise.sh`'s throwaway Postgres. RED: `authorize` refuses with a 500.
mod common;

use common::chain::{A_LEAD, A_M1, A_TL, B_LEAD, World, start};
use common::seed::add_team;
use cortex_testkit::within;
use cortex_types::{SystemClock, TeamId, WorkerId};
use serde_json::{Value, json};
use sqlx::Connection;

fn path(w: WorkerId) -> String {
    format!("/v2/workers/{w}/team")
}

async fn perm_gen(w: &World, id: WorkerId) -> i64 {
    let mut admin = w.db.admin().await;
    sqlx::query_scalar("SELECT perm_gen FROM identity.workers WHERE id = $1::uuid")
        .bind(id.to_string())
        .fetch_one(&mut admin)
        .await
        .expect("perm_gen")
}

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

async fn roles(w: &World, id: WorkerId) -> Vec<String> {
    let mut admin = w.db.admin().await;
    sqlx::query_scalar("SELECT job_roles FROM identity.workers WHERE id = $1::uuid")
        .bind(id.to_string())
        .fetch_one(&mut admin)
        .await
        .expect("roles")
}

/// A second team `Ops` (t2) in project A.
async fn team2(w: &World) -> TeamId {
    let t2 = TeamId::new(&SystemClock);
    let mut admin = w.db.admin().await;
    let mut tx = admin.begin().await.expect("tx");
    add_team(&mut tx, t2, w.f.a, "Ops").await;
    tx.commit().await.expect("commit");
    t2
}

fn details(w: Option<&(String, String, String, String)>) -> Value {
    serde_json::from_str(&w.expect("an audit row").3).expect("details are JSON")
}

#[tokio::test]
async fn id_31_join_move_promote_demote_with_perm_gen_and_audit() {
    within(120, async {
        let w = start("mi_a_basic").await;
        let t2 = team2(&w).await;
        let (t1s, t2s) = (w.f.t1.to_string(), t2.to_string());
        // Join: a worker in no team (carol) becomes a member of t1.
        let g0 = perm_gen(&w, w.f.a_m2).await;
        let r = w.put(A_LEAD, &path(w.f.a_m2), json!({"team": t1s, "role": "member"})).await;
        assert_eq!(r.json, json!({"ok": true}), "{}", r.text);
        assert_eq!(membership(&w, w.f.a_m2).await, Some((t1s.clone(), "member".into())));
        assert_eq!(perm_gen(&w, w.f.a_m2).await, g0 + 1);
        let rows = w.audit().await;
        let d = details(rows.last());
        assert_eq!(rows.last().map(|r| r.0.as_str()), Some("worker.team_assigned"));
        assert_eq!(rows.last().map(|r| r.2.clone()), Some(format!("worker:{}", w.f.a_m2)));
        assert_eq!(d, json!({"team": t1s, "role": "member", "job_roles_changed": false}), "no from_* for a first team");
        // Move: bob (member of t1) to t2. The row is updated, never duplicated.
        let g = perm_gen(&w, w.f.a_m1).await;
        let r = w.put(A_LEAD, &path(w.f.a_m1), json!({"team": t2s, "role": "member"})).await;
        assert_eq!(r.status.as_u16(), 200, "{}", r.text);
        assert_eq!(membership(&w, w.f.a_m1).await, Some((t2s.clone(), "member".into())));
        assert_eq!(w.count("SELECT count(*) FROM identity.team_members").await, 4, "3 seeded + carol; bob moved, not added");
        assert_eq!(perm_gen(&w, w.f.a_m1).await, g + 1);
        let d = details(w.audit().await.last());
        assert_eq!(d, json!({"team": t2s, "role": "member", "from_team": t1s, "from_role": "member", "job_roles_changed": false}));
        // Promote bob in t2 (no lead there): 200; demote: 200.
        let r = w.put(A_LEAD, &path(w.f.a_m1), json!({"team": t2s, "role": "team_lead"})).await;
        assert_eq!(r.status.as_u16(), 200, "{}", r.text);
        assert_eq!(perm_gen(&w, w.f.a_m1).await, g + 2);
        let r = w.put(A_LEAD, &path(w.f.a_m1), json!({"team": t2s, "role": "member"})).await;
        assert_eq!(r.status.as_u16(), 200, "{}", r.text);
        assert_eq!(perm_gen(&w, w.f.a_m1).await, g + 3);
        // The lead is refused by the target rule, a team lead and a member by the scope.
        for t in [A_TL, A_M1] {
            let r = w.put(t, &path(w.f.a_m2), json!({"team": t1s, "role": "member"})).await;
            assert_eq!(r.code(), (403, Some("missing_scope")), "{}", r.text);
        }
    })
    .await
}

#[tokio::test]
async fn id_31b_the_no_op_and_the_job_role_change() {
    within(120, async {
        let w = start("mi_a_noop").await;
        let (alice, t1s) = (w.f.a_tl, w.f.t1.to_string()); // alice: team lead of t1, roles [builder, reviewer]
        let same = json!({"team": t1s, "role": "team_lead"});
        let rows0 = w.audit().await.len();
        let g0 = perm_gen(&w, alice).await;
        // (1) same (team, role), roles absent: a no-op. (2) roles equal: a no-op.
        let r = w.put(A_LEAD, &path(alice), same.clone()).await;
        assert_eq!(r.json, json!({"ok": true}), "{}", r.text);
        let r = w
            .put(A_LEAD, &path(alice), json!({"team": t1s, "role": "team_lead", "job_roles": ["builder", "reviewer"]}))
            .await;
        assert_eq!(r.status.as_u16(), 200, "{}", r.text);
        assert_eq!((w.audit().await.len(), perm_gen(&w, alice).await), (rows0, g0), "no audit row, no perm_gen");
        // (3) different roles: a change; perm_gen does not rise; one record with the flag.
        let r = w
            .put(A_LEAD, &path(alice), json!({"team": t1s, "role": "team_lead", "job_roles": ["builder", "reviewer", "qa"]}))
            .await;
        assert_eq!(r.status.as_u16(), 200, "{}", r.text);
        assert_eq!(roles(&w, alice).await, ["builder", "reviewer", "qa"]);
        assert_eq!(perm_gen(&w, alice).await, g0, "a job-role-only change does not raise perm_gen");
        let rows = w.audit().await;
        assert_eq!(rows.len(), rows0 + 1);
        assert_eq!(
            details(rows.last()),
            json!({"team": t1s, "role": "team_lead", "from_team": t1s, "from_role": "team_lead", "job_roles_changed": true})
        );
        // (4) a reorder is a change.
        let r = w
            .put(A_LEAD, &path(alice), json!({"team": t1s, "role": "team_lead", "job_roles": ["qa", "builder", "reviewer"]}))
            .await;
        assert_eq!(r.status.as_u16(), 200, "{}", r.text);
        assert_eq!(roles(&w, alice).await, ["qa", "builder", "reviewer"]);
        assert_eq!(w.audit().await.len(), rows0 + 2);
        // (5) cleared by []: a change; [] on an empty array: a no-op.
        let r = w.put(A_LEAD, &path(alice), json!({"team": t1s, "role": "team_lead", "job_roles": []})).await;
        assert_eq!(r.status.as_u16(), 200, "{}", r.text);
        assert!(roles(&w, alice).await.is_empty());
        assert_eq!(w.audit().await.len(), rows0 + 3);
        let r = w.put(A_LEAD, &path(alice), json!({"team": t1s, "role": "team_lead", "job_roles": []})).await;
        assert_eq!(r.status.as_u16(), 200, "{}", r.text);
        assert_eq!(w.audit().await.len(), rows0 + 3, "[] on [] is a no-op");
        // (6) a move together with different roles: perm_gen +1 once, ONE record, flag true.
        let t2 = team2(&w).await;
        let t2s = t2.to_string();
        let m1 = w.f.a_m1;
        let g = perm_gen(&w, m1).await;
        let r = w.put(A_LEAD, &path(m1), json!({"team": t2s, "role": "member", "job_roles": ["dev"]})).await;
        assert_eq!(r.status.as_u16(), 200, "{}", r.text);
        assert_eq!(perm_gen(&w, m1).await, g + 1);
        assert_eq!(w.audit().await.len(), rows0 + 4);
        assert_eq!(details(w.audit().await.last())["job_roles_changed"], json!(true));
        // (7) an invalid list is refused at step 0, before any lookup, even for an unknown worker.
        let r = w
            .put(A_LEAD, &path(WorkerId::new(&SystemClock)), json!({"team": t2s, "role": "member", "job_roles": ["a", "a"]}))
            .await;
        assert_eq!(r.code(), (422, Some("job_roles_invalid")), "{}", r.text);
        assert_eq!(w.audit().await.len(), rows0 + 4);
    })
    .await
}

#[tokio::test]
async fn id_32_one_team_lead_per_team_also_under_a_race() {
    within(120, async {
        let w = start("mi_a_lead").await;
        let t2 = team2(&w).await;
        let t2s = t2.to_string();
        // The first promotion in an empty team succeeds; a second one is 409 team_lead_taken.
        let r = w.put(A_LEAD, &path(w.f.a_m1), json!({"team": t2s, "role": "team_lead"})).await;
        assert_eq!(r.status.as_u16(), 200, "{}", r.text);
        let rows = w.audit().await.len();
        let r = w.put(A_LEAD, &path(w.f.a_m2), json!({"team": t2s, "role": "team_lead"})).await;
        assert_eq!(r.code(), (409, Some("team_lead_taken")), "{}", r.text);
        assert_eq!(w.audit().await.len(), rows, "a refusal leaves no audit row");
        assert_eq!(membership(&w, w.f.a_m2).await, None, "the refused worker stayed out of the team");
        // Demote the lead, then promote another: 200.
        let r = w.put(A_LEAD, &path(w.f.a_m1), json!({"team": t2s, "role": "member"})).await;
        assert_eq!(r.status.as_u16(), 200, "{}", r.text);
        let r = w.put(A_LEAD, &path(w.f.a_m2), json!({"team": t2s, "role": "team_lead"})).await;
        assert_eq!(r.status.as_u16(), 200, "{}", r.text);
        // Moving the lead to another team as a member leaves the first team without a lead.
        let r = w.put(A_LEAD, &path(w.f.a_m2), json!({"team": w.f.t1.to_string(), "role": "member"})).await;
        assert_eq!(r.status.as_u16(), 200, "{}", r.text);
        let leads: i64 = {
            let mut admin = w.db.admin().await;
            sqlx::query_scalar(
                "SELECT count(*) FROM identity.team_members WHERE team_id = $1::uuid AND role = 'team_lead'",
            )
            .bind(t2s.clone())
            .fetch_one(&mut admin)
            .await
            .expect("count")
        };
        assert_eq!(leads, 0);
        // Two simultaneous promotions into the empty t2: one 200, one 409, never a 500.
        let (p1, p2) = (path(w.f.a_m1), path(w.f.a_m2));
        let body = json!({"team": t2s, "role": "team_lead"});
        let (a, b) = tokio::join!(w.put(A_LEAD, &p1, body.clone()), w.put(A_LEAD, &p2, body.clone()));
        let mut got = [a.status.as_u16(), b.status.as_u16()];
        got.sort_unstable();
        assert_eq!(got, [200, 409], "{} / {}", a.text, b.text);
        assert_eq!(
            [a.json["type"].as_str(), b.json["type"].as_str()].iter().filter(|t| **t == Some("team_lead_taken")).count(),
            1
        );
    })
    .await
}

#[tokio::test]
async fn id_33_target_rules_and_the_order_of_refusals() {
    within(120, async {
        let w = start("mi_a_rules").await;
        let ok_body = json!({"team": w.f.t1.to_string(), "role": "member"});
        let rows = w.audit().await.len();
        // The project lead as target: 403 target_not_allowed. A retired worker: 409 target_retired.
        let r = w.put(A_LEAD, &path(w.f.a_lead), ok_body.clone()).await;
        assert_eq!(r.code(), (403, Some("target_not_allowed")), "{}", r.text);
        let r = w.put(A_LEAD, &path(w.f.a_ret), ok_body.clone()).await;
        assert_eq!(r.code(), (409, Some("target_retired")), "{}", r.text);
        // Malformed, unknown and another project's path id; an unknown and another project's body team: 404.
        let bad = w.put(A_LEAD, "/v2/workers/abc/team", ok_body.clone()).await;
        let unknown = w
            .put(A_LEAD, &path(WorkerId::new(&SystemClock)), ok_body.clone())
            .await;
        let foreign = w.put(A_LEAD, &path(w.f.b_worker), ok_body.clone()).await;
        for r in [&bad, &unknown, &foreign] {
            assert_eq!(r.code(), (404, Some("not_found")), "{}", r.text);
        }
        assert_eq!(
            bad.body_without_request_id(),
            unknown.body_without_request_id()
        );
        assert_eq!(
            bad.body_without_request_id(),
            foreign.body_without_request_id()
        );
        let ghost = w
            .put(
                A_LEAD,
                &path(w.f.a_m2),
                json!({"team": TeamId::new(&SystemClock).to_string(), "role": "member"}),
            )
            .await;
        let theirs = w
            .put(
                A_LEAD,
                &path(w.f.a_m2),
                json!({"team": w.f.tb.to_string(), "role": "member"}),
            )
            .await;
        assert_eq!(ghost.code(), (404, Some("not_found")));
        assert_eq!(theirs.code(), (404, Some("not_found")));
        assert_eq!(
            ghost.body_without_request_id(),
            theirs.body_without_request_id()
        );
        // Decoding first: a malformed team, a role outside the enum, an unknown field: 422 invalid_request,
        // even with an unknown worker id.
        for body in [
            json!({"team": "abc", "role": "member"}),
            json!({"team": w.f.t1.to_string(), "role": "owner"}),
            json!({"team": w.f.t1.to_string(), "role": "member", "extra": 1}),
            json!({"role": "member"}),
            json!({"team": w.f.t1.to_string()}),
        ] {
            let r = w
                .put(A_LEAD, &path(WorkerId::new(&SystemClock)), body.clone())
                .await;
            assert_eq!(
                r.code(),
                (422, Some("invalid_request")),
                "{body}: {}",
                r.text
            );
        }
        // Valid roles with the three target problems: 404, 403, 409 (the stateless 422 came first above).
        let roles = |extra: Value| {
            let mut b = ok_body.clone();
            b["job_roles"] = extra;
            b
        };
        let r = w
            .put(A_LEAD, &path(w.f.a_lead), roles(json!(["a", "a"])))
            .await;
        assert_eq!(
            r.code(),
            (422, Some("job_roles_invalid")),
            "step 0 before the 403: {}",
            r.text
        );
        let r = w
            .put(A_LEAD, &path(w.f.a_ret), roles(json!(["a", "a"])))
            .await;
        assert_eq!(
            r.code(),
            (422, Some("job_roles_invalid")),
            "step 0 before the 409: {}",
            r.text
        );
        // ID-53: a body over 16384 bytes.
        let huge = format!(
            r#"{{"team":"{}","role":"member","pad":"{}"}}"#,
            w.f.t1,
            "a".repeat(16400)
        );
        let r = w
            .call(
                axum::http::Method::PUT,
                &path(w.f.a_m2),
                Some(A_LEAD),
                None,
                Some(&huge),
            )
            .await;
        assert_eq!(r.code(), (413, Some("body_too_large")));
        // B's lead cannot reach A's worker or A's team.
        let r = w.put(B_LEAD, &path(w.f.a_m2), ok_body.clone()).await;
        assert_eq!(r.code(), (404, Some("not_found")));
        assert_eq!(w.audit().await.len(), rows, "no refusal left an audit row");
    })
    .await
}

#[tokio::test]
async fn id_16_a_fault_in_the_audit_sink_leaves_the_membership_unchanged() {
    within(120, async {
        let w = start("mi_a_atomic").await;
        w.fault.refuse("worker.team_assigned");
        let before = (
            membership(&w, w.f.a_m1).await,
            perm_gen(&w, w.f.a_m1).await,
            roles(&w, w.f.a_m1).await,
            w.audit().await.len(),
        );
        let t2 = team2(&w).await;
        let r = w
            .put(
                A_LEAD,
                &path(w.f.a_m1),
                json!({"team": t2.to_string(), "role": "member", "job_roles": ["dev"]}),
            )
            .await;
        assert_eq!(r.code(), (503, Some("unavailable")), "{}", r.text);
        let after = (
            membership(&w, w.f.a_m1).await,
            perm_gen(&w, w.f.a_m1).await,
            roles(&w, w.f.a_m1).await,
            w.audit().await.len(),
        );
        assert_eq!(
            before, after,
            "membership, perm_gen, job roles and audit all rolled back"
        );
    })
    .await
}

#[tokio::test]
async fn id_31b_9_two_simultaneous_identical_changes_give_one_record() {
    within(120, async {
        let w = start("mi_a_race").await;
        let (alice, t1s) = (w.f.a_tl, w.f.t1.to_string());
        let rows0 = w.audit().await.len();
        let p = path(alice);
        // Widen the window: every UPDATE of a worker sleeps while it holds the row. Without the row lock
        // in `execute`, the second request would read the OLD job roles during that sleep and write a
        // second record; with it, the second request waits and finds the work done.
        {
            let mut admin = w.db.admin().await;
            sqlx::raw_sql(
                "CREATE FUNCTION identity.slow() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN PERFORM pg_sleep(0.5); RETURN NEW; END $$; \
                 CREATE TRIGGER slow BEFORE UPDATE ON identity.workers FOR EACH ROW EXECUTE FUNCTION identity.slow();",
            )
            .execute(&mut admin)
            .await
            .expect("slow trigger");
        }
        let body =
            json!({"team": t1s, "role": "team_lead", "job_roles": ["builder", "reviewer", "qa"]});
        // Different keys, so the chain's replay does not hide the race; the row lock decides.
        let (a, b) = tokio::join!(
            w.put(A_LEAD, &p, body.clone()),
            w.put(A_LEAD, &p, body.clone())
        );
        assert_eq!(
            (a.status.as_u16(), b.status.as_u16()),
            (200, 200),
            "{} / {}",
            a.text,
            b.text
        );
        assert_eq!(roles(&w, alice).await, ["builder", "reviewer", "qa"]);
        assert_eq!(
            w.audit().await.len(),
            rows0 + 1,
            "one write, one record; the second saw the first and was a no-op"
        );
    })
    .await
}

/// ID2B-02 (sam): `job_roles: null` is a decoding error, not "leave the roles as they are".
#[tokio::test]
async fn id_31_an_explicit_null_job_roles_is_refused() {
    within(120, async {
        let w = start("mi_a_null").await;
        let before = (
            membership(&w, w.f.a_m2).await,
            roles(&w, w.f.a_m2).await,
            w.audit().await.len(),
        );
        let r = w
            .put(
                A_LEAD,
                &path(w.f.a_m2),
                json!({"team": w.f.t1.to_string(), "role": "member", "job_roles": null}),
            )
            .await;
        assert_eq!(r.code(), (422, Some("invalid_request")), "{}", r.text);
        let after = (
            membership(&w, w.f.a_m2).await,
            roles(&w, w.f.a_m2).await,
            w.audit().await.len(),
        );
        assert_eq!(before, after);
    })
    .await
}
