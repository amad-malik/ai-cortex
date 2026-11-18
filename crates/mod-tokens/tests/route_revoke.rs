//! Stage C, L3: `tokens.revoke` and `tokens.revoke_self` over the real chain (TOK-50 to TOK-59). Needs the throwaway
//! Postgres (`CV2_DB=1`). RED: the routes are not mounted (404).
mod common;

use common::chain::{Answer, World, start};
use cortex_types::{SystemClock, TokenId, WorkerId};

fn revoke_path(id: TokenId) -> String {
    format!("/v2/tokens/{id}/revoke")
}

async fn state_of(w: &World, id: TokenId) -> (Option<String>, bool) {
    let mut admin = w.db.admin().await;
    sqlx::query_as::<_, (Option<String>, bool)>(
        "SELECT revoke_reason, revoked_at IS NOT NULL FROM tokens.tokens WHERE id = $1::uuid",
    )
    .bind(id.to_string())
    .fetch_one(&mut admin)
    .await
    .expect("row")
}

async fn revoke(w: &World, bearer: &str, id: TokenId, body: &str) -> Answer {
    w.call(
        axum::http::Method::POST,
        &revoke_path(id),
        Some(bearer),
        None,
        Some(body),
    )
    .await
}

#[tokio::test]
async fn tok_50_a_revoke_is_ok_true_marks_the_row_audits_once_and_the_next_request_is_401() {
    let w = start("tr_basic").await;
    let s = w.seed;
    let a = revoke(
        &w,
        w.bearer(s.lead),
        w.token_id(s.w1),
        r#"{"reason":"lost"}"#,
    )
    .await;
    assert_eq!(a.status.as_u16(), 200, "{}", a.text);
    assert_eq!(a.json, serde_json::json!({"ok": true}));
    assert_eq!(
        state_of(&w, w.token_id(s.w1)).await,
        (Some("lost".into()), true)
    );
    assert_eq!(
        w.get(Some(w.bearer(s.w1)), "/v2/whoami").await.code(),
        (401, Some("token_revoked"))
    );
    let rows = w.audit().await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, "token.revoked");
    assert_eq!(rows[0].2, format!("token:{}", w.token_id(s.w1)));
    assert_eq!(rows[0].3, r#"{"reason":"lost"}"#);
}

#[tokio::test]
async fn tok_51_the_reason_grammar_is_checked_before_anything_is_read() {
    let w = start("tr_reason").await;
    let s = w.seed;
    let lead = w.bearer(s.lead);
    let foreign = w.token_id(s.wb1); // another project's token: the stateless checks answer first
    let long33 = format!(r#"{{"reason":"{}"}}"#, "a".repeat(33));
    let bad: Vec<(&str, String)> = vec![
        ("null", "null".into()),
        ("an array", "[]".into()),
        ("empty reason", r#"{"reason":""}"#.into()),
        ("capital", r#"{"reason":"Lost"}"#.into()),
        ("space", r#"{"reason":"lost it"}"#.into()),
        ("leading digit", r#"{"reason":"1lost"}"#.into()),
        ("a digit", r#"{"reason":"lost1"}"#.into()),
        ("33 characters", long33),
        ("trailing newline", "{\"reason\":\"lost\\n\"}".into()),
        ("non-ASCII", "{\"reason\":\"l\u{f6}st\"}".into()),
        ("NUL", "{\"reason\":\"lost\\u0000\"}".into()),
        ("an unknown field", r#"{"reason":"lost","note":"x"}"#.into()),
        ("a number", r#"{"reason":5}"#.into()),
    ];
    for (what, body) in &bad {
        for id in [w.token_id(s.w1), foreign] {
            let a = revoke(&w, lead, id, body).await;
            assert_eq!(
                a.code(),
                (422, Some("invalid_request")),
                "{what}: {}",
                a.text
            );
        }
    }
    // `{}` is reason_required, also for a foreign id (stateless first).
    for id in [w.token_id(s.w1), foreign] {
        assert_eq!(
            revoke(&w, lead, id, "{}").await.code(),
            (422, Some("reason_required"))
        );
    }
    // Malformed JSON is 400.
    assert_eq!(
        revoke(&w, lead, w.token_id(s.w1), "{\"reason\":")
            .await
            .status
            .as_u16(),
        400
    );
    // `lost_key` and a 32-character code are accepted.
    assert_eq!(
        revoke(&w, lead, w.token_id(s.w1), r#"{"reason":"lost_key"}"#)
            .await
            .status
            .as_u16(),
        200
    );
    let code32 = format!(r#"{{"reason":"{}"}}"#, "a".repeat(32));
    assert_eq!(
        revoke(&w, lead, w.token_id(s.w2), &code32)
            .await
            .status
            .as_u16(),
        200
    );
    // The property of delta 5: every accepted code is recordable.
    for c in ["lost", "lost_key", "a", "a_b_c", &"z".repeat(32)] {
        assert!(
            cortex_module::AuditDetails::new().code("reason", c).is_ok(),
            "{c}"
        );
    }
}

#[tokio::test]
async fn tok_52_a_second_revoke_is_ok_and_changes_nothing_r14() {
    let w = start("tr_idem").await;
    let s = w.seed;
    let lead = w.bearer(s.lead);
    let id = w.token_id(s.w1);
    assert_eq!(
        revoke(&w, lead, id, r#"{"reason":"lost"}"#)
            .await
            .status
            .as_u16(),
        200
    );
    let audit_before = w.audit().await.len();
    let again = revoke(&w, lead, id, r#"{"reason":"compromised"}"#).await;
    assert_eq!(
        (again.status.as_u16(), again.json),
        (200, serde_json::json!({"ok": true}))
    );
    assert_eq!(
        state_of(&w, id).await.0.as_deref(),
        Some("lost"),
        "the original reason stays"
    );
    assert_eq!(w.audit().await.len(), audit_before, "no new audit row");
}

#[tokio::test]
async fn tok_53_54_targets_system_codes_and_self_revocation() {
    let w = start("tr_targets").await;
    let s = w.seed;
    // TL revokes an own-team token; TL on L's token and a team-U token is 403; B's, a random and a malformed id are 404 alike.
    assert_eq!(
        revoke(&w, w.bearer(s.tl), w.token_id(s.w2), r#"{"reason":"lost"}"#)
            .await
            .status
            .as_u16(),
        200
    );
    for t in [s.lead, s.w3] {
        assert_eq!(
            revoke(&w, w.bearer(s.tl), w.token_id(t), r#"{"reason":"lost"}"#)
                .await
                .code(),
            (403, Some("target_not_allowed"))
        );
    }
    let a = revoke(
        &w,
        w.bearer(s.lead),
        w.token_id(s.wb1),
        r#"{"reason":"lost"}"#,
    )
    .await;
    let b = revoke(
        &w,
        w.bearer(s.lead),
        TokenId::new(&SystemClock),
        r#"{"reason":"lost"}"#,
    )
    .await;
    let c = w
        .post(
            w.bearer(s.lead),
            "/v2/tokens/not-a-uuid/revoke",
            serde_json::json!({"reason": "lost"}),
        )
        .await;
    for x in [&a, &b, &c] {
        assert_eq!(x.code(), (404, Some("not_found")));
    }
    assert_eq!(a.body_without_request_id(), b.body_without_request_id());
    assert_eq!(a.body_without_request_id(), c.body_without_request_id());
    // TOK-54: the four system codes are 422 at step 0, also for an unknown id; `self` and `lost` are accepted.
    for code in ["superseded", "reissued", "retired", "replaced_unused"] {
        let body = format!(r#"{{"reason":"{code}"}}"#);
        assert_eq!(
            revoke(&w, w.bearer(s.lead), w.token_id(s.w1), &body)
                .await
                .code(),
            (422, Some("invalid_request")),
            "{code}"
        );
        assert_eq!(
            revoke(&w, w.bearer(s.lead), TokenId::new(&SystemClock), &body)
                .await
                .code(),
            (422, Some("invalid_request")),
            "{code} on an unknown id"
        );
    }
    assert_eq!(
        revoke(
            &w,
            w.bearer(s.lead),
            w.token_id(s.w4),
            r#"{"reason":"self"}"#
        )
        .await
        .status
        .as_u16(),
        200
    );
    // L revokes the token it is presenting: 200, then 401; the replay of that key is 401, never the stored 200 (R-11).
    let own = w.bearer(s.lead).to_owned();
    let first = w
        .post_keyed(
            &own,
            &revoke_path(w.token_id(s.lead)),
            "key-self-kk",
            r#"{"reason":"lost"}"#,
        )
        .await;
    assert_eq!(first.status.as_u16(), 200);
    assert_eq!(
        w.get(Some(&own), "/v2/whoami").await.code(),
        (401, Some("token_revoked"))
    );
    let replay = w
        .post_keyed(
            &own,
            &revoke_path(w.token_id(s.lead)),
            "key-self-kk",
            r#"{"reason":"lost"}"#,
        )
        .await;
    assert_eq!(replay.code(), (401, Some("token_revoked")));
}

#[tokio::test]
async fn tok_55_revoke_self_by_every_role_reason_self_and_a_repeat_is_401() {
    let w = start("tr_self").await;
    let s = w.seed;
    for (n, who) in [s.w4, s.tl, s.lead].into_iter().enumerate() {
        let bearer = w.bearer(who).to_owned();
        let key = format!("self-key-{n}");
        let a = w
            .post_keyed(&bearer, "/v2/self/token/revoke", &key, "{}")
            .await;
        assert_eq!(
            (a.status.as_u16(), a.json.clone()),
            (200, serde_json::json!({"ok": true})),
            "{}",
            a.text
        );
        assert_eq!(
            w.get(Some(&bearer), "/v2/whoami").await.code(),
            (401, Some("token_revoked"))
        );
        assert_eq!(
            state_of(&w, w.token_id(who)).await.0.as_deref(),
            Some("self")
        );
        let again = w
            .post_keyed(&bearer, "/v2/self/token/revoke", &key, "{}")
            .await;
        assert_eq!(
            again.code(),
            (401, Some("token_revoked")),
            "never the stored 200"
        );
    }
    let rows = w.audit().await;
    assert_eq!(rows.len(), 3);
    assert!(
        rows.iter()
            .all(|r| r.0 == "token.self_revoked" && r.3 == r#"{"reason":"self"}"#),
        "{rows:?}"
    );
    assert_eq!(
        w.get(Some(w.bearer(s.w2)), "/v2/whoami")
            .await
            .status
            .as_u16(),
        200,
        "others are unaffected"
    );
}

#[tokio::test]
async fn tok_56_a_failing_audit_sink_leaves_the_token_active_and_a_retry_succeeds_once() {
    let w = start("tr_fault").await;
    let s = w.seed;
    w.fault.refuse("token.revoked");
    let bad = revoke(
        &w,
        w.bearer(s.lead),
        w.token_id(s.w1),
        r#"{"reason":"lost"}"#,
    )
    .await;
    assert!(bad.status.as_u16() >= 500, "{}", bad.text);
    assert_eq!(
        state_of(&w, w.token_id(s.w1)).await,
        (None, false),
        "nothing written"
    );
    w.fault.clear();
    assert_eq!(
        revoke(
            &w,
            w.bearer(s.lead),
            w.token_id(s.w1),
            r#"{"reason":"lost"}"#
        )
        .await
        .status
        .as_u16(),
        200
    );
    assert_eq!(w.audit().await.len(), 1);
}

#[tokio::test]
async fn tok_57_eight_concurrent_revokes_with_eight_codes_record_one() {
    let w = std::sync::Arc::new(start("tr_race").await);
    let s = w.seed;
    let id = w.token_id(s.w1);
    let codes = ["aa", "bb", "cc", "dd", "ee", "ff", "gg", "hh"];
    let mut tasks = Vec::new();
    for c in codes {
        let w = w.clone();
        let bearer = w.bearer(s.lead).to_owned();
        tasks.push(tokio::spawn(async move {
            revoke(&w, &bearer, id, &format!(r#"{{"reason":"{c}"}}"#))
                .await
                .status
                .as_u16()
        }));
    }
    for t in tasks {
        assert_eq!(t.await.expect("task"), 200);
    }
    let (reason, revoked) = state_of(&w, id).await;
    assert!(revoked && codes.contains(&reason.as_deref().expect("reason")));
    assert_eq!(w.audit().await.len(), 1, "one audit record");
}

#[tokio::test]
async fn tok_58_r19_a_replay_is_the_stored_200_only_while_the_target_rule_still_allows() {
    let w = start("tr_r19").await;
    let s = w.seed;
    let path = revoke_path(w.token_id(s.w1));
    let first = w
        .post_keyed(w.bearer(s.tl), &path, "key-r19-kr", r#"{"reason":"lost"}"#)
        .await;
    assert_eq!(first.status.as_u16(), 200);
    let audit_before = w.audit().await.len();
    let replay = w
        .post_keyed(w.bearer(s.tl), &path, "key-r19-kr", r#"{"reason":"lost"}"#)
        .await;
    assert_eq!(replay.status.as_u16(), 200);
    assert_eq!(replay.header("cortex-idempotent-replay"), Some("true"));
    assert_eq!(w.audit().await.len(), audit_before, "no write, no audit");
    let mut admin = w.db.admin().await;
    // Team U already has its team lead: demote it, so the moved TL is still a team lead (of U).
    sqlx::query("UPDATE identity.team_members SET role = 'member' WHERE worker_id = $1::uuid")
        .bind(s.ul.to_string())
        .execute(&mut admin)
        .await
        .expect("demote UL");
    sqlx::query("UPDATE identity.team_members SET team_id = $1::uuid WHERE worker_id = $2::uuid")
        .bind(s.u.to_string())
        .bind(s.tl.to_string())
        .execute(&mut admin)
        .await
        .expect("move TL");
    let b = w
        .post_keyed(w.bearer(s.tl), &path, "key-r19-kr", r#"{"reason":"lost"}"#)
        .await;
    assert_eq!(b.code(), (403, Some("target_not_allowed")), "{}", b.text);
    assert!(b.header("cortex-idempotent-replay").is_none());
    assert_eq!(
        w.post(w.bearer(s.tl), &path, serde_json::json!({"reason": "lost"}))
            .await
            .code(),
        (403, Some("target_not_allowed"))
    );
}

#[tokio::test]
async fn tok_59_a_key_reused_on_another_token_is_422_only_after_the_target_check_passes() {
    let w = start("tr_reuse").await;
    let s = w.seed;
    let body = r#"{"reason":"lost"}"#;
    assert_eq!(
        w.post_keyed(
            w.bearer(s.lead),
            &revoke_path(w.token_id(s.w1)),
            "key-reuse-kx",
            body
        )
        .await
        .status
        .as_u16(),
        200
    );
    let reused = w
        .post_keyed(
            w.bearer(s.lead),
            &revoke_path(w.token_id(s.w2)),
            "key-reuse-kx",
            body,
        )
        .await;
    assert_eq!(
        reused.code(),
        (422, Some("idempotency_key_reused")),
        "{}",
        reused.text
    );
    // A caller with no access to Y (TL, whose team does not hold W3) learns nothing about the key.
    let outsider = w
        .post_keyed(
            w.bearer(s.tl),
            &revoke_path(w.token_id(s.w3)),
            "key-reuse-kx",
            body,
        )
        .await;
    assert_eq!(outsider.code(), (403, Some("target_not_allowed")));
    // The key first used on B's token id by an outsider: 404, no hint that the key exists.
    let foreign = w
        .post_keyed(
            w.bearer(s.lead),
            &revoke_path(w.token_id(s.wb1)),
            "key-reuse-ky",
            body,
        )
        .await;
    assert_eq!(foreign.code(), (404, Some("not_found")));
    let _: Option<WorkerId> = None;
}
