//! 2b, through the real chain: idempotency and replay on the four writes (S3 R-11, R-19).
//! Rows: ID-50. A replay never bypasses authorization; the lookup comes after 404 and 403 and before
//! 409; a refusal is never recorded; a key is bound to project, principal, method, path and body.
//! Needs `supervise.sh`'s throwaway Postgres. RED: `authorize` refuses with a 500.
mod common;

use axum::http::Method;
use common::chain::{A_LEAD, A_M1, B_LEAD, start};
use cortex_testkit::within;
use serde_json::json;

fn replayed(a: &common::chain::Answer) -> bool {
    a.headers
        .get("cortex-idempotent-replay")
        .and_then(|v| v.to_str().ok())
        == Some("true")
}

#[tokio::test]
async fn id_50_replay_of_a_create_is_the_stored_reply_and_one_row() {
    within(120, async {
        let w = start("mi_i_create").await;
        let body = json!({"name": "Replayed", "kind": "worker"}).to_string();
        let k = "idem-key-replay-0001";
        let first = w.call(Method::POST, "/v2/workers", Some(A_LEAD), Some(k), Some(&body)).await;
        assert_eq!(first.status.as_u16(), 201, "{}", first.text);
        assert!(!replayed(&first));
        let again = w.call(Method::POST, "/v2/workers", Some(A_LEAD), Some(k), Some(&body)).await;
        assert_eq!(again.status.as_u16(), 201, "{}", again.text);
        assert!(replayed(&again), "the replay is flagged");
        assert_eq!(again.json, first.json, "the stored reply, the same id");
        assert_eq!(
            w.count("SELECT count(*) FROM identity.workers WHERE id::text = (SELECT entity_id::text FROM identity.names WHERE label = 'Replayed')").await,
            1
        );
        assert_eq!(w.count("SELECT count(*) FROM identity.names WHERE label = 'Replayed'").await, 1);
        assert_eq!(w.audit().await.iter().filter(|r| r.0 == "worker.created").count(), 1, "a replay writes no second audit row");
        // The same key with a different body: 422 idempotency_key_reused.
        let other = json!({"name": "Different", "kind": "worker"}).to_string();
        let r = w.call(Method::POST, "/v2/workers", Some(A_LEAD), Some(k), Some(&other)).await;
        assert_eq!(r.code(), (422, Some("idempotency_key_reused")), "{}", r.text);
        // A key bound to another path: the same key and body on /v2/teams is also reuse, not a replay.
        let r = w.call(Method::POST, "/v2/teams", Some(A_LEAD), Some(k), Some(&json!({"name": "Replayed"}).to_string())).await;
        assert_ne!(r.status.as_u16(), 201, "{}", r.text);
    })
    .await
}

#[tokio::test]
async fn id_50_a_refusal_is_never_recorded_and_a_replay_never_bypasses_authorization() {
    within(120, async {
        let w = start("mi_i_auth").await;
        // A refused call (422) does not use up its key.
        let k = "idem-key-refused-0001";
        let bad = w
            .call(
                Method::POST,
                "/v2/teams",
                Some(A_LEAD),
                Some(k),
                Some(&json!({"name": "1bad"}).to_string()),
            )
            .await;
        assert_eq!(bad.code(), (422, Some("name_invalid")), "{}", bad.text);
        let good = w
            .call(
                Method::POST,
                "/v2/teams",
                Some(A_LEAD),
                Some(k),
                Some(&json!({"name": "Good"}).to_string()),
            )
            .await;
        assert_eq!(
            good.status.as_u16(),
            201,
            "the key was not recorded by the refusal: {}",
            good.text
        );
        // The lead creates with key K; a member sends the same key and body: 403, not the stored reply.
        let body = json!({"name": "Guarded"}).to_string();
        let k = "idem-key-guarded-001";
        let ok = w
            .call(
                Method::POST,
                "/v2/teams",
                Some(A_LEAD),
                Some(k),
                Some(&body),
            )
            .await;
        assert_eq!(ok.status.as_u16(), 201, "{}", ok.text);
        let denied = w
            .call(Method::POST, "/v2/teams", Some(A_M1), Some(k), Some(&body))
            .await;
        assert_eq!(
            denied.code(),
            (403, Some("missing_scope")),
            "{}",
            denied.text
        );
        assert!(!replayed(&denied));
        // Another project's lead with the same key and body is a fresh request in its own project.
        let b = w
            .call(
                Method::POST,
                "/v2/teams",
                Some(B_LEAD),
                Some(k),
                Some(&body),
            )
            .await;
        assert_eq!(b.status.as_u16(), 201, "{}", b.text);
        assert!(!replayed(&b));
        assert_ne!(b.json["id"], ok.json["id"]);
        // A write without a key: 428 idempotency_key_required, on all four routes. The body must be valid:
        // the chain decodes the body BEFORE it looks at the key (R-16), so `{}` would be a 422 first.
        let t1 = w.f.t1.to_string();
        for (m, p, body) in [
            (
                Method::POST,
                "/v2/teams".to_owned(),
                json!({"name": "Keyless"}),
            ),
            (
                Method::POST,
                "/v2/workers".to_owned(),
                json!({"name": "Keyless", "kind": "worker"}),
            ),
            (
                Method::POST,
                format!("/v2/workers/{}/retire", w.f.a_m1),
                json!({"reason": "x"}),
            ),
            (
                Method::PUT,
                format!("/v2/workers/{}/team", w.f.a_m1),
                json!({"team": t1, "role": "member"}),
            ),
        ] {
            let req = axum::http::Request::builder()
                .method(m.clone())
                .uri(&p)
                .header("authorization", format!("Bearer {A_LEAD}"))
                .header("content-type", "application/json")
                .body(axum::body::Body::from(body.to_string()))
                .expect("req");
            let resp = tower::ServiceExt::oneshot(w.app.clone(), req)
                .await
                .expect("resp");
            assert_eq!(resp.status().as_u16(), 428, "{m} {p}");
        }
    })
    .await
}

#[tokio::test]
async fn id_50_replay_comes_before_the_409_and_a_new_key_gets_the_409() {
    within(120, async {
        let w = start("mi_i_order").await;
        // Retire: the stored 200 is replayed although the worker is now retired (409 target_retired for a new key).
        let path = format!("/v2/workers/{}/retire", w.f.a_m1);
        let body = json!({"reason": "done"}).to_string();
        let k = "idem-key-retire-0001";
        let first = w
            .call(Method::POST, &path, Some(A_LEAD), Some(k), Some(&body))
            .await;
        assert_eq!(first.status.as_u16(), 200, "{}", first.text);
        let rows = w.audit().await.len();
        let again = w
            .call(Method::POST, &path, Some(A_LEAD), Some(k), Some(&body))
            .await;
        assert_eq!(again.status.as_u16(), 200, "{}", again.text);
        assert!(replayed(&again));
        let fresh = w
            .call(Method::POST, &path, Some(A_LEAD), None, Some(&body))
            .await;
        assert_eq!(
            fresh.code(),
            (409, Some("target_retired")),
            "{}",
            fresh.text
        );
        assert_eq!(
            w.audit().await.len(),
            rows,
            "neither the replay nor the refusal wrote a row"
        );
        // Assign: the same.
        let t1 = w.f.t1.to_string();
        let path = format!("/v2/workers/{}/team", w.f.a_m2);
        let body = json!({"team": t1, "role": "member"}).to_string();
        let k = "idem-key-assign-0001";
        let first = w
            .call(Method::PUT, &path, Some(A_LEAD), Some(k), Some(&body))
            .await;
        assert_eq!(first.status.as_u16(), 200, "{}", first.text);
        let rows = w.audit().await.len();
        let again = w
            .call(Method::PUT, &path, Some(A_LEAD), Some(k), Some(&body))
            .await;
        assert_eq!(again.status.as_u16(), 200, "{}", again.text);
        assert!(replayed(&again));
        assert_eq!(w.audit().await.len(), rows);
        // A repeat with a NEW key is the no-op of ID-31b: 200, no row, not flagged as a replay.
        let noop = w
            .call(Method::PUT, &path, Some(A_LEAD), None, Some(&body))
            .await;
        assert_eq!(noop.status.as_u16(), 200, "{}", noop.text);
        assert!(!replayed(&noop));
        assert_eq!(w.audit().await.len(), rows);
    })
    .await
}
