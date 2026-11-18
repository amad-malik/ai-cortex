//! 2b, through the real chain: `POST /v2/teams` and `POST /v2/workers`.
//! Rows: ID-30, ID-10, ID-11, ID-12a/b, ID-13, ID-14, ID-15, ID-16, ID-03/04/05 (names), ID-54 (order),
//! ID-51 (audit), ID-53 (body limit). Needs `supervise.sh`'s throwaway Postgres. RED: `authorize` refuses
//! with a 500.
mod common;

use common::chain::{A_LEAD, A_M1, A_TL, B_LEAD, World, start};
use cortex_testkit::within;
use cortex_types::{SystemClock, TeamId};
use serde_json::json;

fn id_of(a: &common::chain::Answer) -> String {
    a.json["id"].as_str().expect("an id").to_owned()
}

const NAMES_OF: &str = "SELECT count(*) FROM identity.names";
const WORKERS: &str = "SELECT count(*) FROM identity.workers";
const TEAMS: &str = "SELECT count(*) FROM identity.teams";
const MEMBERS: &str = "SELECT count(*) FROM identity.team_members";

/// (workers, teams, team_members, names, audit rows): the whole footprint of a request.
async fn footprint(w: &World) -> (i64, i64, i64, i64, usize) {
    (
        w.count(WORKERS).await,
        w.count(TEAMS).await,
        w.count(MEMBERS).await,
        w.count(NAMES_OF).await,
        w.audit().await.len(),
    )
}

#[tokio::test]
async fn id_30_team_create_audit_names_and_roles() {
    within(120, async {
        let w = start("mi_w_team").await;
        let before = footprint(&w).await;
        let a = w.post(A_LEAD, "/v2/teams", json!({"name": "Ops"})).await;
        assert_eq!(a.status.as_u16(), 201, "{}", a.text);
        assert_eq!(a.json.as_object().expect("object").len(), 1, "exactly {{id}}");
        let id = id_of(&a);
        let after = footprint(&w).await;
        assert_eq!((after.1, after.3, after.4), (before.1 + 1, before.3 + 1, before.4 + 1));
        let (state, key, label, current): (String, String, String, bool) = {
            let mut admin = w.db.admin().await;
            sqlx::query_as(
                "SELECT t.state, n.name_key, n.label, n.current FROM identity.teams t \
                 JOIN identity.names n ON n.entity_kind = 'team' AND n.entity_id = t.id WHERE t.id = $1::uuid",
            )
            .bind(&id)
            .fetch_one(&mut admin)
            .await
            .expect("team row")
        };
        assert_eq!((state.as_str(), key.as_str(), label.as_str(), current), ("active", "ops", "Ops", true));
        let rows = w.audit().await;
        let last = rows.last().expect("an audit row");
        assert_eq!(last.0, "team.created");
        assert_eq!(last.2, format!("team:{id}"));
        assert_eq!(last.3, "{}", "no details");
        assert!(last.1.contains(&w.f.a_lead.to_string()), "the caller is the actor: {}", last.1);
        // A duplicate by key: 409 name_taken, nothing written. A worker and a team may share a name.
        let dup = w.post(A_LEAD, "/v2/teams", json!({"name": "OPS"})).await;
        assert_eq!(dup.code(), (409, Some("name_taken")), "{}", dup.text);
        assert_eq!(footprint(&w).await, after, "a refusal leaves nothing, no audit row either");
        let shared = w.post(A_LEAD, "/v2/teams", json!({"name": "alice"})).await;
        assert_eq!(shared.status.as_u16(), 201, "{}", shared.text);
        // The same name in another project is accepted.
        let other = w.post(B_LEAD, "/v2/teams", json!({"name": "Ops"})).await;
        assert_eq!(other.status.as_u16(), 201, "{}", other.text);
        // A team lead and a member lack the scope.
        for t in [A_TL, A_M1] {
            let r = w.post(t, "/v2/teams", json!({"name": "Nope"})).await;
            assert_eq!(r.code(), (403, Some("missing_scope")), "{}", r.text);
        }
    })
    .await
}

#[tokio::test]
async fn id_30_team_create_refusals_write_nothing() {
    within(120, async {
        let w = start("mi_w_team_bad").await;
        let before = footprint(&w).await;
        let long = "a".repeat(33);
        for bad in [
            "",
            long.as_str(),
            "1abc",
            "-abc",
            "ab_c",
            "ab c",
            "sophi\n",
            "\u{212A}evin",
        ] {
            let r = w.post(A_LEAD, "/v2/teams", json!({"name": bad})).await;
            assert_eq!(
                r.code(),
                (422, Some("name_invalid")),
                "name {bad:?}: {}",
                r.text
            );
        }
        // Decoding: unknown or mass-assigned fields, a missing name, a non-string name, a NUL.
        for body in [
            json!({"name": "Ok", "id": "x"}),
            json!({"name": "Ok", "project_id": "x"}),
            json!({"name": "Ok", "state": "archived"}),
            json!({}),
            json!({"name": 7}),
            json!({"name": null}),
            json!({"name": "a\u{0}b"}),
        ] {
            let r = w.post(A_LEAD, "/v2/teams", body.clone()).await;
            assert_eq!(
                r.code(),
                (422, Some("invalid_request")),
                "{body}: {}",
                r.text
            );
        }
        assert_eq!(footprint(&w).await, before);
        // ID-53: a body over 16384 bytes is 413 before parsing.
        let big = format!(r#"{{"name":"{}"}}"#, "a".repeat(16400));
        let r = w
            .call(
                axum::http::Method::POST,
                "/v2/teams",
                Some(A_LEAD),
                None,
                Some(&big),
            )
            .await;
        assert_eq!(r.code(), (413, Some("body_too_large")), "{}", r.text);
        assert_eq!(footprint(&w).await, before);
    })
    .await
}

#[tokio::test]
async fn id_10_11_14_worker_create_row_defaults_project_and_audit() {
    within(120, async {
        let w = start("mi_w_worker").await;
        let before = footprint(&w).await;
        let b_workers = {
            let mut admin = w.db.admin().await;
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM identity.workers WHERE project_id = $1::uuid")
                .bind(w.f.b.to_string())
                .fetch_one(&mut admin)
                .await
                .expect("B count")
        };
        let a = w
            .call(
                axum::http::Method::POST,
                "/v2/workers",
                Some(A_LEAD),
                None,
                Some(&json!({"name": "Newbie", "kind": "worker", "job_roles": ["dev", "qa"], "mode": "deterministic"}).to_string()),
            )
            .await;
        assert_eq!(a.status.as_u16(), 201, "{}", a.text);
        assert_eq!(a.json.as_object().expect("object").len(), 1, "exactly {{id}}");
        let id = id_of(&a);
        let (state, retired, perm_gen, mode, roles, project, kind): (String, Option<String>, i64, String, Vec<String>, String, String) = {
            let mut admin = w.db.admin().await;
            sqlx::query_as(
                "SELECT state, retired_at::text, perm_gen, mode, job_roles, project_id::text, kind \
                 FROM identity.workers WHERE id = $1::uuid",
            )
            .bind(&id)
            .fetch_one(&mut admin)
            .await
            .expect("worker row")
        };
        assert_eq!(
            (state.as_str(), retired, perm_gen, mode.as_str(), roles, project, kind.as_str()),
            ("active", None, 0, "deterministic", vec!["dev".to_owned(), "qa".to_owned()], w.f.a.to_string(), "worker")
        );
        let (names_for_it, label, key): (i64, String, String) = {
            let mut admin = w.db.admin().await;
            sqlx::query_as(
                "SELECT count(*), max(label), max(name_key) FROM identity.names WHERE entity_kind = 'worker' AND entity_id = $1::uuid AND current",
            )
            .bind(&id)
            .fetch_one(&mut admin)
            .await
            .expect("name")
        };
        assert_eq!((names_for_it, label.as_str(), key.as_str()), (1, "Newbie", "newbie"));
        let after = footprint(&w).await;
        assert_eq!((after.0, after.2, after.3, after.4), (before.0 + 1, before.2, before.3 + 1, before.4 + 1));
        let rows = w.audit().await;
        let last = rows.last().expect("audit row");
        assert_eq!((last.0.as_str(), last.2.as_str(), last.3.as_str()), ("worker.created", format!("worker:{id}").as_str(), "{}"));
        // ID-11 defaults; ID-14 the token's project wins over any header or body.
        let d = w
            .call(
                axum::http::Method::POST,
                "/v2/workers",
                Some(A_LEAD),
                None,
                Some(&json!({"name": "Plain", "kind": "worker"}).to_string()),
            )
            .await;
        assert_eq!(d.status.as_u16(), 201, "{}", d.text);
        let (mode, roles): (String, Vec<String>) = {
            let mut admin = w.db.admin().await;
            sqlx::query_as("SELECT mode, job_roles FROM identity.workers WHERE id = $1::uuid")
                .bind(id_of(&d))
                .fetch_one(&mut admin)
                .await
                .expect("defaults")
        };
        assert_eq!((mode.as_str(), roles.len()), ("model", 0));
        let sneaky = w.f.b.to_string();
        let r = {
            let req = axum::http::Request::builder()
                .method("POST")
                .uri("/v2/workers")
                .header("authorization", format!("Bearer {A_LEAD}"))
                .header("idempotency-key", "idem-key-sneaky-1")
                .header("content-type", "application/json")
                .header("x-project", &sneaky)
                .body(axum::body::Body::from(json!({"name": "Sneaky", "kind": "worker"}).to_string()))
                .expect("req");
            tower::ServiceExt::oneshot(w.app.clone(), req).await.expect("resp")
        };
        assert_eq!(r.status().as_u16(), 201);
        let b_after = {
            let mut admin = w.db.admin().await;
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM identity.workers WHERE project_id = $1::uuid")
                .bind(w.f.b.to_string())
                .fetch_one(&mut admin)
                .await
                .expect("B count")
        };
        assert_eq!(b_after, b_workers, "B gained nothing");
        // The roster shows the new worker.
        let list = w.get(Some(A_M1), "/v2/workers?name=NEWBIE").await;
        assert_eq!(list.json["items"].as_array().map(Vec::len), Some(1), "{}", list.text);
        // The names stay taken: a duplicate by key, a retired worker's name, the other project is free.
        for n in ["newbie", "NEWBIE", "dave"] {
            let r = w.post(A_LEAD, "/v2/workers", json!({"name": n, "kind": "worker"})).await;
            assert_eq!(r.code(), (409, Some("name_taken")), "{n}: {}", r.text);
        }
        let ok = w.post(B_LEAD, "/v2/workers", json!({"name": "Newbie", "kind": "worker"})).await;
        assert_eq!(ok.status.as_u16(), 201, "{}", ok.text);
        for t in [A_TL, A_M1] {
            let r = w.post(t, "/v2/workers", json!({"name": "Nope", "kind": "worker"})).await;
            assert_eq!(r.code(), (403, Some("missing_scope")), "{}", r.text);
        }
    })
    .await
}

#[tokio::test]
async fn id_12_13_worker_create_refusals_write_nothing() {
    within(120, async {
        let w = start("mi_w_worker_bad").await;
        let before = footprint(&w).await;
        let many: Vec<String> = (0..9).map(|i| format!("r{i}")).collect();
        let thirty_three = "a".repeat(33);
        let decoding = [
            json!({"name": "X1", "kind": "worker", "mode": "fast"}),
            json!({"name": "X1", "kind": "service"}),
            json!({"name": "X1"}),
            json!({"name": "X1", "kind": "worker", "job_roles": [null]}),
            json!({"name": "X1", "kind": "worker", "job_roles": ["a", 1]}),
            json!({"name": "X1", "kind": "worker", "job_roles": "dev"}),
            json!({"name": "X1", "kind": "worker", "foo": 1}),
            json!({"name": "X1", "kind": "worker", "id": "x"}),
            json!({"name": "X1", "kind": "worker", "project_id": "x"}),
            json!({"name": "X1", "kind": "worker", "state": "retired"}),
            json!({"name": "X1", "kind": "worker", "perm_gen": 5}),
            json!({"name": "X1", "kind": "worker", "retired_at": "2026-11-06T00:00:00Z"}),
            json!({"name": "X1", "kind": "worker", "lead": "x"}),
            json!({"name": "X1", "kind": "worker", "token_path_hint": "/x"}),
            json!({"name": "X1", "kind": "worker", "team": "abc"}),
            json!({"name": "X1", "kind": "worker", "job_roles": ["a\u{0}b"]}),
        ];
        for body in decoding {
            let r = w.post(A_LEAD, "/v2/workers", body.clone()).await;
            assert_eq!(
                r.code(),
                (422, Some("invalid_request")),
                "{body}: {}",
                r.text
            );
        }
        let roles_bad = [
            json!(["QA"]),
            json!(["1qa"]),
            json!(["a b"]),
            json!(["a_b"]),
            json!([thirty_three]),
            json!(["dev", "dev"]),
            json!(many),
            json!(["qa,dev"]),
            json!([""]),
        ];
        for roles in roles_bad {
            let r = w
                .post(
                    A_LEAD,
                    "/v2/workers",
                    json!({"name": "X1", "kind": "worker", "job_roles": roles.clone()}),
                )
                .await;
            assert_eq!(
                r.code(),
                (422, Some("job_roles_invalid")),
                "{roles}: {}",
                r.text
            );
        }
        for bad in ["", "1abc", "ab c", "sophi\n"] {
            let r = w
                .post(
                    A_LEAD,
                    "/v2/workers",
                    json!({"name": bad, "kind": "worker"}),
                )
                .await;
            assert_eq!(r.code(), (422, Some("name_invalid")), "{bad:?}: {}", r.text);
        }
        assert_eq!(
            footprint(&w).await,
            before,
            "nothing written by any refusal"
        );
        // The boundaries are accepted: 8 roles, a 32-character role, a 32-character name.
        let eight: Vec<String> = (0..8).map(|i| format!("r{i}")).collect();
        let ok = w
            .post(
                A_LEAD,
                "/v2/workers",
                json!({"name": "a".repeat(32), "kind": "worker", "job_roles": eight}),
            )
            .await;
        assert_eq!(ok.status.as_u16(), 201, "{}", ok.text);
        let ok = w
            .post(
                A_LEAD,
                "/v2/workers",
                json!({"name": "Okay", "kind": "worker", "job_roles": ["a".repeat(32)]}),
            )
            .await;
        assert_eq!(ok.status.as_u16(), 201, "{}", ok.text);
    })
    .await
}

#[tokio::test]
async fn id_15_54_worker_create_team_404_and_the_refusal_order() {
    within(120, async {
        let w = start("mi_w_worker_team").await;
        let before = footprint(&w).await;
        // With an own team: a team_members row with role member, same transaction.
        let a = w
            .post(A_LEAD, "/v2/workers", json!({"name": "Joiner", "kind": "worker", "team": w.f.t1.to_string()}))
            .await;
        assert_eq!(a.status.as_u16(), 201, "{}", a.text);
        let (team, role): (String, String) = {
            let mut admin = w.db.admin().await;
            sqlx::query_as("SELECT team_id::text, role FROM identity.team_members WHERE worker_id = $1::uuid")
                .bind(id_of(&a))
                .fetch_one(&mut admin)
                .await
                .expect("membership")
        };
        assert_eq!((team, role.as_str()), (w.f.t1.to_string(), "member"));
        let mid = footprint(&w).await;
        assert_eq!((mid.0, mid.2), (before.0 + 1, before.2 + 1));
        // Unknown team and another project's team: one 404 body for both; nothing written.
        let unknown = w
            .post(A_LEAD, "/v2/workers", json!({"name": "Ghost", "kind": "worker", "team": TeamId::new(&SystemClock).to_string()}))
            .await;
        let foreign = w
            .post(A_LEAD, "/v2/workers", json!({"name": "Ghost", "kind": "worker", "team": w.f.tb.to_string()}))
            .await;
        assert_eq!(unknown.code(), (404, Some("not_found")), "{}", unknown.text);
        assert_eq!(foreign.code(), (404, Some("not_found")), "{}", foreign.text);
        assert_eq!(unknown.body_without_request_id(), foreign.body_without_request_id());
        assert_eq!(footprint(&w).await, mid);
        // ID-54: an invalid name with an unknown team: name_invalid; duplicate roles with a taken name:
        // job_roles_invalid; a valid taken name with an unknown team: 404; a valid taken name alone: 409.
        let ghost = TeamId::new(&SystemClock).to_string();
        let r = w.post(A_LEAD, "/v2/workers", json!({"name": "1bad", "kind": "worker", "team": ghost})).await;
        assert_eq!(r.code(), (422, Some("name_invalid")));
        let r = w.post(A_LEAD, "/v2/workers", json!({"name": "Joiner", "kind": "worker", "job_roles": ["a", "a"]})).await;
        assert_eq!(r.code(), (422, Some("job_roles_invalid")));
        let r = w.post(A_LEAD, "/v2/workers", json!({"name": "Joiner", "kind": "worker", "team": ghost})).await;
        assert_eq!(r.code(), (404, Some("not_found")));
        let r = w.post(A_LEAD, "/v2/workers", json!({"name": "Joiner", "kind": "worker"})).await;
        assert_eq!(r.code(), (409, Some("name_taken")));
        // Per-field before cross-field (CHANGES s17): a bad name together with duplicate roles is name_invalid.
        let r = w.post(A_LEAD, "/v2/workers", json!({"name": "1bad", "kind": "worker", "job_roles": ["a", "a"]})).await;
        assert_eq!(r.code(), (422, Some("name_invalid")));
        assert_eq!(footprint(&w).await, mid);
    })
    .await
}

#[tokio::test]
async fn id_16_a_fault_in_the_audit_sink_leaves_no_rows() {
    within(120, async {
        let w = start("mi_w_atomic").await;
        let before = footprint(&w).await;
        w.fault.refuse("worker.created");
        let r = w
            .post(A_LEAD, "/v2/workers", json!({"name": "Doomed", "kind": "worker", "team": w.f.t1.to_string(), "job_roles": ["dev"]}))
            .await;
        assert_eq!(r.code(), (503, Some("unavailable")), "{}", r.text);
        assert_eq!(footprint(&w).await, before, "no worker, name, membership or audit row survives");
        w.fault.refuse("team.created");
        let r = w.post(A_LEAD, "/v2/teams", json!({"name": "Doomed"})).await;
        assert_eq!(r.code(), (503, Some("unavailable")), "{}", r.text);
        assert_eq!(footprint(&w).await, before, "no team, name or audit row survives");
        // The same requests succeed once the fault is cleared: nothing was half-written.
        w.fault.clear();
        let r = w.post(A_LEAD, "/v2/workers", json!({"name": "Doomed", "kind": "worker"})).await;
        assert_eq!(r.status.as_u16(), 201, "{}", r.text);
        let r = w.post(A_LEAD, "/v2/teams", json!({"name": "Doomed"})).await;
        assert_eq!(r.status.as_u16(), 201, "{}", r.text);
    })
    .await
}

/// Cox's ruling on NUL (6 Nov 12:32Z): a NUL in ANY string is 422 `invalid_request`, before the field's
/// own code. Boundary cells (alone, in the middle, at the end) and a control: an invalid character that
/// is NOT a NUL still answers the field's own code.
#[tokio::test]
async fn id_02_12b_nul_is_invalid_request_before_the_own_code_and_the_control_keeps_it() {
    within(120, async {
        let w = start("mi_w_nul").await;
        let before = footprint(&w).await;
        for nul in ["\u{0}", "a\u{0}b", "ab\u{0}", "\u{0}ab"] {
            let t = w.post(A_LEAD, "/v2/teams", json!({"name": nul})).await;
            assert_eq!(
                t.code(),
                (422, Some("invalid_request")),
                "team name {nul:?}: {}",
                t.text
            );
            let r = w
                .post(
                    A_LEAD,
                    "/v2/workers",
                    json!({"name": nul, "kind": "worker"}),
                )
                .await;
            assert_eq!(
                r.code(),
                (422, Some("invalid_request")),
                "worker name {nul:?}: {}",
                r.text
            );
            let r = w
                .post(
                    A_LEAD,
                    "/v2/workers",
                    json!({"name": "Okay", "kind": "worker", "job_roles": [nul]}),
                )
                .await;
            assert_eq!(
                r.code(),
                (422, Some("invalid_request")),
                "job role {nul:?}: {}",
                r.text
            );
        }
        // The control: an invalid character that is not a NUL keeps the field's own code.
        for bad in ["ab c", "a_b", "a.b", "\u{7f}x", "a\u{1}b"] {
            let t = w.post(A_LEAD, "/v2/teams", json!({"name": bad})).await;
            assert_eq!(
                t.code(),
                (422, Some("name_invalid")),
                "team name {bad:?}: {}",
                t.text
            );
            let r = w
                .post(
                    A_LEAD,
                    "/v2/workers",
                    json!({"name": bad, "kind": "worker"}),
                )
                .await;
            assert_eq!(
                r.code(),
                (422, Some("name_invalid")),
                "worker name {bad:?}: {}",
                r.text
            );
        }
        for bad in ["a b", "A", "a_b"] {
            let r = w
                .post(
                    A_LEAD,
                    "/v2/workers",
                    json!({"name": "Okay", "kind": "worker", "job_roles": [bad]}),
                )
                .await;
            assert_eq!(
                r.code(),
                (422, Some("job_roles_invalid")),
                "job role {bad:?}: {}",
                r.text
            );
        }
        assert_eq!(
            footprint(&w).await,
            before,
            "nothing written by any of them"
        );
    })
    .await
}

/// ID2B-02 (sam): an optional property may be omitted but not null (S3 R-3); null is a decoding error,
/// not a quiet default.
#[tokio::test]
async fn id_12a_an_explicit_null_for_an_optional_property_is_refused() {
    within(120, async {
        let w = start("mi_w_null").await;
        let before = footprint(&w).await;
        for field in ["job_roles", "mode", "team"] {
            let mut body = json!({"name": "Nulls", "kind": "worker"});
            body[field] = serde_json::Value::Null;
            let r = w.post(A_LEAD, "/v2/workers", body.clone()).await;
            assert_eq!(r.code(), (422, Some("invalid_request")), "{body}: {}", r.text);
        }
        let body = json!({"name": "Nulls", "kind": "worker", "job_roles": null, "mode": null, "team": null});
        let r = w.post(A_LEAD, "/v2/workers", body.clone()).await;
        assert_eq!(r.code(), (422, Some("invalid_request")), "{body}: {}", r.text);
        assert_eq!(footprint(&w).await, before, "nothing was created with defaults");
        // Omission is fine: the same request without the three properties creates the worker.
        let r = w.post(A_LEAD, "/v2/workers", json!({"name": "Nulls", "kind": "worker"})).await;
        assert_eq!(r.status.as_u16(), 201, "{}", r.text);
    })
    .await
}
