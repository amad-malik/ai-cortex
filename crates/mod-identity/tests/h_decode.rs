//! ID2B-02 (sam) and R-3, R-7: how a body is READ. Through the real chain.
//! - A body must be a JSON object: a JSON array, a string, a number, `true` and `null` are 422
//!   `invalid_request` on all four writes (serde alone would build a struct from an array).
//! - An enum property (`kind`, `mode`, `role`) given a non-string value is 422, not 400 (serde_json
//!   reports a non-string value for a derived enum as a syntax error).
//!
//! Needs `supervise.sh`'s throwaway Postgres.
mod common;

use axum::http::Method;
use common::chain::{A_LEAD, start};
use cortex_testkit::within;
use serde_json::{Value, json};

#[tokio::test]
async fn a_body_that_is_not_an_object_is_refused_on_all_four_writes() {
    within(120, async {
        let w = start("mi_d_object").await;
        let routes = [
            (Method::POST, "/v2/teams".to_owned()),
            (Method::POST, "/v2/workers".to_owned()),
            (Method::POST, format!("/v2/workers/{}/retire", w.f.a_m1)),
            (Method::PUT, format!("/v2/workers/{}/team", w.f.a_m1)),
        ];
        let rows0 = w.audit().await.len();
        let counts0 = (
            w.count("SELECT count(*) FROM identity.teams").await,
            w.count("SELECT count(*) FROM identity.workers").await,
            w.count("SELECT count(*) FROM identity.workers WHERE state = 'retired'")
                .await,
            w.count("SELECT count(*) FROM identity.team_members").await,
        );
        let t1 = w.f.t1.to_string();
        for (m, p) in routes {
            for body in [
                "[]".to_owned(),
                r#"["Ops"]"#.to_owned(),
                format!(r#"["x", "{t1}"]"#),
                r#""Ops""#.to_owned(),
                "5".to_owned(),
                "true".to_owned(),
                "null".to_owned(),
            ] {
                let r = w.call(m.clone(), &p, Some(A_LEAD), None, Some(&body)).await;
                assert_eq!(
                    r.code(),
                    (422, Some("invalid_request")),
                    "{m} {p} {body}: {}",
                    r.text
                );
            }
        }
        assert_eq!(w.audit().await.len(), rows0);
        let counts1 = (
            w.count("SELECT count(*) FROM identity.teams").await,
            w.count("SELECT count(*) FROM identity.workers").await,
            w.count("SELECT count(*) FROM identity.workers WHERE state = 'retired'")
                .await,
            w.count("SELECT count(*) FROM identity.team_members").await,
        );
        assert_eq!(
            counts0, counts1,
            "nothing was created, retired or moved by any of them"
        );
    })
    .await
}

#[tokio::test]
async fn an_enum_property_with_a_non_string_value_is_422_not_400() {
    within(120, async {
        let w = start("mi_d_enum").await;
        let wrong: [Value; 5] = [json!(null), json!(5), json!(true), json!([]), json!({})];
        for v in &wrong {
            for field in ["kind", "mode"] {
                let mut body = json!({"name": "Enums", "kind": "worker"});
                body[field] = v.clone();
                let r = w.post(A_LEAD, "/v2/workers", body.clone()).await;
                assert_eq!(
                    r.code(),
                    (422, Some("invalid_request")),
                    "{body}: {}",
                    r.text
                );
            }
            let r = w
                .put(
                    A_LEAD,
                    &format!("/v2/workers/{}/team", w.f.a_m2),
                    json!({"team": w.f.t1.to_string(), "role": v.clone()}),
                )
                .await;
            assert_eq!(
                r.code(),
                (422, Some("invalid_request")),
                "role {v}: {}",
                r.text
            );
        }
        // The control: a string that is not a variant is 422 too, and a real value still works.
        let r = w
            .post(
                A_LEAD,
                "/v2/workers",
                json!({"name": "Enums", "kind": "worker", "mode": "fast"}),
            )
            .await;
        assert_eq!(r.code(), (422, Some("invalid_request")));
        let r = w
            .post(
                A_LEAD,
                "/v2/workers",
                json!({"name": "Enums", "kind": "worker", "mode": "deterministic"}),
            )
            .await;
        assert_eq!(r.status.as_u16(), 201, "{}", r.text);
    })
    .await
}
