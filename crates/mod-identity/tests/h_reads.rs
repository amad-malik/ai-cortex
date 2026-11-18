//! 2a, through the real chain: the three read routes (S3 r2.5).
//! Rows: ID-40 (project_get), ID-41 (teams_list), ID-42 (workers_list), ID-06 (`?name=`), ID-52 (R-8
//! lists, R-4 strict queries), ID-49 (codes only), X-04 for the reads (another project's ids).
//! Needs `supervise.sh`'s throwaway Postgres. RED: the handlers are `todo!()`.
mod common;

use std::collections::BTreeSet;

use common::chain::{A_LEAD, A_M1, A_TL, Answer, B_LEAD, C_LEAD, World, start};
use common::seed::{add_team, add_worker};
use cortex_testkit::within;
use cortex_types::{SystemClock, TeamId, WorkerId};
use serde_json::{Value, json};
use sqlx::Connection;

fn items(a: &Answer) -> Vec<Value> {
    a.json["items"].as_array().cloned().unwrap_or_default()
}

fn names(a: &Answer) -> Vec<String> {
    items(a)
        .iter()
        .filter_map(|w| w["name"].as_str().map(str::to_owned))
        .collect()
}

#[tokio::test]
async fn id_40_project_get_own_id_and_the_three_404s_are_identical() {
    within(120, async {
        let w = start("mi_h_project").await;
        let own = w
            .get(Some(A_LEAD), &format!("/v2/projects/{}", w.f.a))
            .await;
        assert_eq!(own.status.as_u16(), 200, "{}", own.text);
        assert_eq!(
            own.json,
            json!({"id": w.f.a.to_string(), "name": "Alpha"}),
            "exactly {{id, name}}"
        );
        let other = w
            .get(Some(A_LEAD), &format!("/v2/projects/{}", w.f.b))
            .await;
        let random = w
            .get(
                Some(A_LEAD),
                &format!("/v2/projects/{}", WorkerId::new(&SystemClock)),
            )
            .await;
        let malformed = w.get(Some(A_LEAD), "/v2/projects/abc").await;
        for a in [&other, &random, &malformed] {
            assert_eq!(a.code(), (404, Some("not_found")), "{}", a.text);
        }
        assert_eq!(
            other.body_without_request_id(),
            random.body_without_request_id()
        );
        assert_eq!(
            other.body_without_request_id(),
            malformed.body_without_request_id()
        );
        assert!(!other.text.contains("Beta"), "no row text in a 404");
        // B's lead reads B's own project.
        let b = w
            .get(Some(B_LEAD), &format!("/v2/projects/{}", w.f.b))
            .await;
        assert_eq!(b.json["name"], "Beta");
        // A route that declares no query refuses one (R-4 default-deny).
        let q = w
            .get(Some(A_LEAD), &format!("/v2/projects/{}?x=1", w.f.a))
            .await;
        assert_eq!(q.code(), (422, Some("invalid_request")), "{}", q.text);
        assert_eq!(
            w.get(None, &format!("/v2/projects/{}", w.f.a))
                .await
                .code()
                .0,
            401
        );
    })
    .await
}

#[tokio::test]
async fn id_41_teams_list_is_the_tokens_project_with_its_lead_or_null() {
    within(120, async {
        let w = start("mi_h_teams").await;
        let a = w.get(Some(A_M1), "/v2/teams").await;
        assert_eq!(a.status.as_u16(), 200, "{}", a.text);
        assert_eq!(
            a.json,
            json!({"items": [{"id": w.f.t1.to_string(), "name": "Platform", "lead": w.f.a_tl.to_string()}], "next": null})
        );
        // B's team has no team lead: lead is null. C has no team: an empty page.
        let b = w.get(Some(B_LEAD), "/v2/teams").await;
        assert_eq!(
            b.json,
            json!({"items": [{"id": w.f.tb.to_string(), "name": "Core", "lead": null}], "next": null})
        );
        let c = w.get(Some(C_LEAD), "/v2/teams").await;
        assert_eq!(c.json, json!({"items": [], "next": null}));
    })
    .await
}

#[tokio::test]
async fn id_42_workers_list_fields_order_and_what_is_never_shown() {
    within(120, async {
        let w = start("mi_h_workers").await;
        let a = w.get(Some(A_TL), "/v2/workers").await;
        assert_eq!(a.status.as_u16(), 200, "{}", a.text);
        // name_key order; the retired `dave` is never listed (Q-2).
        assert_eq!(names(&a), ["alice", "Bob", "carol", "svc-1", "Zed"]);
        assert_eq!(a.json["next"], Value::Null);
        let by = |n: &str| {
            items(&a)
                .into_iter()
                .find(|v| v["name"] == n)
                .expect("item")
        };
        let lead = by("Zed");
        assert_eq!(
            lead,
            json!({
                "id": w.f.a_lead.to_string(), "name": "Zed", "kind": "worker", "state": "active",
                "team": null, "role": "project_lead", "job_roles": ["lead"], "mode": "model"
            })
        );
        let tl = by("alice");
        assert_eq!(
            (tl["role"].as_str(), tl["team"].as_str()),
            (Some("team_lead"), Some(w.f.t1.to_string().as_str()))
        );
        assert_eq!(by("svc-1")["kind"], "service");
        assert_eq!(by("carol")["team"], Value::Null);
        for item in items(&a) {
            let keys: BTreeSet<&str> = item
                .as_object()
                .expect("object")
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(
                keys,
                BTreeSet::from([
                    "id",
                    "name",
                    "kind",
                    "state",
                    "team",
                    "role",
                    "job_roles",
                    "mode"
                ]),
                "no perm_gen, project_id or token_path_hint"
            );
        }
        let r = w.get(Some(A_TL), "/v2/workers?include_retired=true").await;
        assert_eq!(r.code(), (422, Some("invalid_request")), "{}", r.text);
        // Project B's token sees B's two workers only.
        assert_eq!(
            names(&w.get(Some(B_LEAD), "/v2/workers").await),
            ["bl", "bw"]
        );
    })
    .await
}

#[tokio::test]
async fn id_06_the_name_filter_matches_the_key_and_refuses_what_is_no_name() {
    within(120, async {
        let w = start("mi_h_name").await;
        for q in ["alice", "ALICE", "Alice"] {
            let a = w.get(Some(A_LEAD), &format!("/v2/workers?name={q}")).await;
            assert_eq!(names(&a), ["alice"], "?name={q}");
        }
        // Retired `dave` and an unknown name: an empty page, not 404.
        for q in ["dave", "nobody"] {
            let a = w.get(Some(A_LEAD), &format!("/v2/workers?name={q}")).await;
            assert_eq!(a.json, json!({"items": [], "next": null}), "?name={q}");
        }
        // Outside the grammar: 422 invalid_request at step 0, never an empty page.
        let long = "a".repeat(33);
        for q in [
            long.as_str(),
            "a%20b",
            "1abc",
            "sophi%0A",
            "a%00b",
            "-a",
            "caf%C3%A9",
        ] {
            let a = w.get(Some(A_LEAD), &format!("/v2/workers?name={q}")).await;
            assert_eq!(
                a.code(),
                (422, Some("invalid_request")),
                "?name={q}: {}",
                a.text
            );
        }
        // 32 characters is a valid name that matches nobody.
        let ok = "a".repeat(32);
        let a = w.get(Some(A_LEAD), &format!("/v2/workers?name={ok}")).await;
        assert_eq!(a.json, json!({"items": [], "next": null}));
    })
    .await
}

/// 120 more workers in A, named `w000`..`w119`, as the superuser.
async fn add_many(w: &World) {
    let mut admin = w.db.admin().await;
    let mut tx = admin.begin().await.expect("tx");
    for i in 0..120 {
        add_worker(
            &mut tx,
            WorkerId::new(&SystemClock),
            w.f.a,
            &format!("w{i:03}"),
            "worker",
            false,
            &[],
        )
        .await;
    }
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn id_52_limits_unknown_parameters_and_cursors() {
    within(120, async {
        let w = start("mi_h_lists").await;
        for q in [
            "limit=0",
            "limit=201",
            "limit=-1",
            "limit=abc",
            "bogus=1",
            "limit=1&bogus=1",
        ] {
            for path in ["/v2/workers", "/v2/teams"] {
                let a = w.get(Some(A_LEAD), &format!("{path}?{q}")).await;
                assert_eq!(
                    a.code(),
                    (422, Some("invalid_request")),
                    "{path}?{q}: {}",
                    a.text
                );
            }
        }
        assert_eq!(
            names(&w.get(Some(A_LEAD), "/v2/workers?limit=200").await).len(),
            5
        );
        assert_eq!(
            names(&w.get(Some(A_LEAD), "/v2/workers?limit=1").await),
            ["alice"]
        );
        // A cursor that does not decode: 422.
        for bad in ["zzz", "", "00", "%00"] {
            let a = w
                .get(Some(A_LEAD), &format!("/v2/workers?after={bad}"))
                .await;
            assert_eq!(
                a.code(),
                (422, Some("invalid_request")),
                "after={bad:?}: {}",
                a.text
            );
        }
        // A teams cursor on the workers list: 422 (a cursor is bound to its list). A has one team,
        // so add a second to get a `next`.
        {
            let mut admin = w.db.admin().await;
            let mut tx = admin.begin().await.expect("tx");
            add_team(&mut tx, TeamId::new(&SystemClock), w.f.a, "Zeta").await;
            tx.commit().await.expect("commit");
        }
        let page = w.get(Some(A_LEAD), "/v2/teams?limit=1").await;
        let cursor = page.json["next"]
            .as_str()
            .expect("a next cursor")
            .to_owned();
        assert_eq!(names(&page), ["Platform"]);
        let rest = w
            .get(Some(A_LEAD), &format!("/v2/teams?limit=1&after={cursor}"))
            .await;
        assert_eq!(
            (names(&rest), rest.json["next"].clone()),
            (vec!["Zeta".to_owned()], Value::Null)
        );
        let cross = w
            .get(Some(A_LEAD), &format!("/v2/workers?after={cursor}"))
            .await;
        assert_eq!(
            cross.code(),
            (422, Some("invalid_request")),
            "{}",
            cross.text
        );
    })
    .await
}

#[tokio::test]
async fn id_52_paging_has_no_gaps_and_no_repeats_even_when_rows_arrive_between_calls() {
    within(180, async {
        let w = start("mi_h_paging").await;
        add_many(&w).await; // A now has 5 + 120 = 125 active workers
        let p1 = w.get(Some(A_LEAD), "/v2/workers?limit=50").await;
        assert_eq!(items(&p1).len(), 50, "{}", p1.text);
        let c1 = p1.json["next"]
            .as_str()
            .expect("next after page 1")
            .to_owned();
        // A worker whose key sorts BEFORE the cursor arrives between the calls: it must neither
        // repeat an existing worker nor hide one.
        {
            let mut admin = w.db.admin().await;
            let mut tx = admin.begin().await.expect("tx");
            add_worker(
                &mut tx,
                WorkerId::new(&SystemClock),
                w.f.a,
                "aaa-early",
                "worker",
                false,
                &[],
            )
            .await;
            tx.commit().await.expect("commit");
        }
        let p2 = w
            .get(Some(A_LEAD), &format!("/v2/workers?limit=50&after={c1}"))
            .await;
        assert_eq!(items(&p2).len(), 50, "{}", p2.text);
        let c2 = p2.json["next"]
            .as_str()
            .expect("next after page 2")
            .to_owned();
        // One that sorts AFTER the cursor arrives: it is seen on a later page.
        {
            let mut admin = w.db.admin().await;
            let mut tx = admin.begin().await.expect("tx");
            add_worker(
                &mut tx,
                WorkerId::new(&SystemClock),
                w.f.a,
                "zzz-late",
                "worker",
                false,
                &[],
            )
            .await;
            tx.commit().await.expect("commit");
        }
        let p3 = w
            .get(Some(A_LEAD), &format!("/v2/workers?limit=50&after={c2}"))
            .await;
        assert_eq!(
            p3.json["next"],
            Value::Null,
            "the last page has next = null"
        );
        let all: Vec<String> = [&p1, &p2, &p3].iter().flat_map(|p| names(p)).collect();
        let unique: BTreeSet<&String> = all.iter().collect();
        assert_eq!(all.len(), unique.len(), "no worker appears twice");
        let originals: BTreeSet<String> = ["alice", "Bob", "carol", "svc-1", "Zed"]
            .iter()
            .map(|s| (*s).to_owned())
            .chain((0..120).map(|i| format!("w{i:03}")))
            .collect();
        for o in &originals {
            assert!(unique.contains(o), "{o} is not hidden");
        }
        assert!(
            unique.contains(&"zzz-late".to_owned()),
            "a row after the cursor is seen"
        );
        assert!(
            !unique.contains(&"aaa-early".to_owned()),
            "a row before the cursor is not"
        );
        // Stable order: the concatenation is sorted by name key.
        let keys: Vec<String> = all.iter().map(|n| n.to_lowercase()).collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
        // A's cursor with B's token returns none of A's rows.
        let b = w
            .get(Some(B_LEAD), &format!("/v2/workers?after={c1}"))
            .await;
        for n in names(&b) {
            assert!(["bl", "bw"].contains(&n.as_str()), "only B's rows, got {n}");
        }
    })
    .await
}

#[tokio::test]
async fn id_49_problem_bodies_carry_codes_not_rows() {
    within(120, async {
        let w = start("mi_h_codes").await;
        let a = w.get(Some(A_LEAD), "/v2/workers?name=a%20b").await;
        let b = w
            .get(Some(A_LEAD), &format!("/v2/projects/{}", w.f.b))
            .await;
        for body in [&a.text, &b.text] {
            for banned in ["Alpha", "Beta", "alice", "identity.", "SELECT", "sqlstate"] {
                assert!(
                    !body.contains(banned),
                    "{banned:?} must not appear in {body}"
                );
            }
        }
    })
    .await
}
