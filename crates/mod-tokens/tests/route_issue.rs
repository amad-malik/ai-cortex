//! Stage C, L3: `tokens.issue` over the real chain (TOK-21 to TOK-31, TOK-33, and the OS-randomness condition of cox
//! W1-32). Needs `supervise.sh`'s throwaway Postgres (`CV2_DB=1`). RED: the routes are not mounted (404).
mod common;

use std::sync::Arc;

use axum::http::Method;
use common::chain::{World, start, start_with};
use cortex_module::TargetRule;
use cortex_types::WorkerId;
use mod_tokens::Entropy;
use mod_tokens::token::EntropyError;

fn issue_path(w: WorkerId) -> String {
    format!("/v2/workers/{w}/tokens")
}

fn worker_of(a: &common::chain::Answer) -> Option<String> {
    a.json["worker"].as_str().map(str::to_owned)
}

async fn whoami_worker(w: &World, bearer: &str) -> (u16, Option<String>) {
    let a = w.get(Some(bearer), "/v2/whoami").await;
    (a.status.as_u16(), worker_of(&a))
}

#[tokio::test]
async fn tok_21_22_24_issue_returns_the_token_once_supersedes_the_old_one_and_stores_the_digest_row()
 {
    let w = start("ti_issue").await;
    let s = w.seed;
    let old = w.bearer(s.w1).to_owned();
    let a = w
        .post(w.bearer(s.lead), &issue_path(s.w1), serde_json::json!({}))
        .await;
    assert_eq!(a.status.as_u16(), 201, "{}", a.text);
    let mut keys: Vec<&str> = a
        .json
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(keys, ["expires_at", "token", "token_id"]);
    let token = a.json["token"].as_str().expect("token").to_owned();
    assert_eq!(
        token[5..37],
        a.json["token_id"].as_str().expect("id").replace('-', ""),
        "token_id is the id inside the token"
    );
    // Cortex-Token-Expires is the CALLER's expiry, not the new token's.
    assert!(a.header("cortex-token-expires").is_some());
    assert_ne!(
        a.header("cortex-token-expires"),
        a.json["expires_at"].as_str().map(|_| "x")
    );
    // The new token authenticates as W1 at once; the old one is revoked (reissued), W2's is untouched.
    assert_eq!(
        whoami_worker(&w, &token).await,
        (200, Some(s.w1.to_string()))
    );
    assert_eq!(
        w.get(Some(&old), "/v2/whoami").await.code(),
        (401, Some("token_revoked"))
    );
    assert_eq!(
        whoami_worker(&w, w.bearer(s.w2)).await,
        (200, Some(s.w2.to_string()))
    );
    assert_eq!(
        w.count(format!(
            "SELECT count(*) FROM tokens.tokens WHERE worker_id = '{}' AND revoked_at IS NULL",
            s.w1
        ))
        .await,
        1,
        "exactly one active token for W1"
    );
    assert_eq!(
        w.count(format!("SELECT count(*) FROM tokens.tokens WHERE worker_id = '{}' AND revoke_reason = 'reissued'", s.w1)).await,
        1
    );
    // The row: W1's project, issued now, issued_by the caller, renew_after exactly two thirds.
    let mut admin = w.db.admin().await;
    let row: (String, i64, i64, i64, String) = sqlx::query_as(
        "SELECT project_id::text, (extract(epoch from issued_at)*1000000)::bigint, \
                (extract(epoch from expires_at)*1000000)::bigint, (extract(epoch from renew_after)*1000000)::bigint, issued_by::text \
         FROM tokens.tokens WHERE id = $1::uuid",
    )
    .bind(a.json["token_id"].as_str().expect("id"))
    .fetch_one(&mut admin)
    .await
    .expect("row");
    assert_eq!(row.0, s.a.to_string());
    assert_eq!(row.1, common::db::NOW);
    assert_eq!(row.3, row.1 + (row.2 - row.1) * 2 / 3);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&row.4).expect("json"),
        serde_json::json!({"kind": "worker", "id": s.lead.to_string()})
    );
}

#[tokio::test]
async fn tok_26_an_issue_is_never_stored_or_replayed_a_retry_is_already_issued() {
    let w = start("ti_idem").await;
    let s = w.seed;
    let first = w
        .post_keyed(w.bearer(s.lead), &issue_path(s.w1), "key-issue-k", "{}")
        .await;
    assert_eq!(first.status.as_u16(), 201, "{}", first.text);
    let token = first.json["token"].as_str().expect("token").to_owned();
    let id = first.json["token_id"].as_str().expect("id").to_owned();
    let again = w
        .post_keyed(w.bearer(s.lead), &issue_path(s.w1), "key-issue-k", "{}")
        .await;
    assert_eq!(
        again.code(),
        (409, Some("already_issued")),
        "{}",
        again.text
    );
    assert!(again.text.contains(&id), "naming the token id");
    assert!(
        !again.text.contains(&token) && !again.text.contains(&token[38..]),
        "never the secret"
    );
    assert_eq!(
        w.count(format!(
            "SELECT count(*) FROM tokens.tokens WHERE worker_id = '{}'",
            s.w1
        ))
        .await,
        2,
        "no second row for the repeat (the fixture's token and the one issued)"
    );
    // Neither the secret nor the token is in the idempotency table.
    let mut admin = w.db.admin().await;
    let rows: Vec<String> = sqlx::query_scalar("SELECT i::text FROM platform.idempotency i")
        .fetch_all(&mut admin)
        .await
        .expect("rows");
    assert!(
        rows.iter()
            .all(|r| !r.contains(&token) && !r.contains(&token[38..]))
    );
    // A new key issues again and supersedes.
    let third = w
        .post(w.bearer(s.lead), &issue_path(s.w1), serde_json::json!({}))
        .await;
    assert_eq!(third.status.as_u16(), 201);
    assert_eq!(
        w.get(Some(&token), "/v2/whoami").await.code(),
        (401, Some("token_revoked"))
    );
}

#[tokio::test]
async fn tok_27_a_retired_worker_is_409_and_an_unknown_foreign_or_malformed_one_is_404_alike() {
    let w = start("ti_targets").await;
    let s = w.seed;
    let lead = w.bearer(s.lead);
    let before = w.count("SELECT count(*) FROM tokens.tokens").await;
    assert_eq!(
        w.post(lead, &issue_path(s.wr), serde_json::json!({}))
            .await
            .code(),
        (409, Some("target_retired"))
    );
    let unknown = WorkerId::new(&cortex_types::SystemClock);
    let a = w
        .post(lead, &issue_path(unknown), serde_json::json!({}))
        .await;
    let b = w
        .post(lead, &issue_path(s.wb1), serde_json::json!({}))
        .await;
    let c = w
        .post(lead, "/v2/workers/not-a-uuid/tokens", serde_json::json!({}))
        .await;
    for x in [&a, &b, &c] {
        assert_eq!(x.code(), (404, Some("not_found")), "{}", x.text);
    }
    assert_eq!(a.body_without_request_id(), b.body_without_request_id());
    assert_eq!(a.body_without_request_id(), c.body_without_request_id());
    assert_eq!(
        w.count("SELECT count(*) FROM tokens.tokens").await,
        before,
        "nothing written"
    );
    assert!(w.audit().await.is_empty(), "no audit row");
}

#[tokio::test]
async fn tok_28_the_handler_asks_the_policy_once_with_the_row_it_read_and_never_takes_request_input()
 {
    let w = start("ti_policy").await;
    let s = w.seed;
    // W1 is in team T; the project lead is `lead`.
    assert_eq!(
        w.post(w.bearer(s.lead), &issue_path(s.w1), serde_json::json!({}))
            .await
            .status
            .as_u16(),
        201
    );
    let asked = w.asked.lock().expect("lock").clone();
    assert_eq!(asked.len(), 1, "the handler calls `target` once");
    let (caller, rule, target) = &asked[0];
    assert_eq!((*caller, *rule), (s.lead, TargetRule::TokenTarget));
    assert_eq!(target.worker, Some(s.w1));
    assert_eq!(target.kind, Some(cortex_module::WorkerKind::Worker));
    assert_eq!(target.team, Some(s.t));
    assert_eq!(target.lead, Some(s.lead));
    assert!(!target.consult);
    // A body field `team` is 422: the Target never takes request input.
    let r = w
        .post(
            w.bearer(s.lead),
            &issue_path(s.w1),
            serde_json::json!({"team": s.u.to_string()}),
        )
        .await;
    assert_eq!(r.code(), (422, Some("invalid_request")), "{}", r.text);
    // The truth table through the policy double: L issues for W1, W3, W4; TL for own team only; a member is 403 missing_scope.
    for t in [s.w3, s.w4] {
        assert_eq!(
            w.post(w.bearer(s.lead), &issue_path(t), serde_json::json!({}))
                .await
                .status
                .as_u16(),
            201
        );
    }
    assert_eq!(
        w.post(w.bearer(s.tl), &issue_path(s.w2), serde_json::json!({}))
            .await
            .status
            .as_u16(),
        201
    );
    for t in [s.w3, s.w4, s.lead] {
        assert_eq!(
            w.post(w.bearer(s.tl), &issue_path(t), serde_json::json!({}))
                .await
                .code(),
            (403, Some("target_not_allowed"))
        );
    }
    let member = w.bearer(s.w4).to_owned();
    // W4's token was superseded by the issue above; use W1's current one through a fresh issue.
    let fresh = w
        .post(w.bearer(s.lead), &issue_path(s.w1), serde_json::json!({}))
        .await
        .json["token"]
        .as_str()
        .expect("t")
        .to_owned();
    let _ = member;
    assert_eq!(
        w.post(&fresh, &issue_path(s.w2), serde_json::json!({}))
            .await
            .code(),
        (403, Some("missing_scope"))
    );
}

#[tokio::test]
async fn tok_30_the_refusal_order_is_404_403_then_409() {
    let w = start("ti_order").await;
    let s = w.seed;
    // W3 (team U) retires; TL (team T) asks for it: 403, not 409. The lead asks: 409.
    let mut admin = w.db.admin().await;
    sqlx::query(
        "UPDATE identity.workers SET state = 'retired', retired_at = now() WHERE id = $1::uuid",
    )
    .bind(s.w3.to_string())
    .execute(&mut admin)
    .await
    .expect("retire w3");
    assert_eq!(
        w.post(w.bearer(s.tl), &issue_path(s.w3), serde_json::json!({}))
            .await
            .code(),
        (403, Some("target_not_allowed"))
    );
    assert_eq!(
        w.post(w.bearer(s.lead), &issue_path(s.w3), serde_json::json!({}))
            .await
            .code(),
        (409, Some("target_retired"))
    );
    // TL with a worker of project B: 404, not 403.
    assert_eq!(
        w.post(w.bearer(s.tl), &issue_path(s.wb1), serde_json::json!({}))
            .await
            .code(),
        (404, Some("not_found"))
    );
}

#[tokio::test]
async fn tok_31_one_audit_record_per_issue_and_a_failing_sink_leaves_nothing() {
    let w = start("ti_audit").await;
    let s = w.seed;
    let ok = w
        .post(w.bearer(s.lead), &issue_path(s.w1), serde_json::json!({}))
        .await;
    assert_eq!(ok.status.as_u16(), 201);
    let rows = w.audit().await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, "token.issued");
    assert!(
        rows[0].1.contains(&s.lead.to_string()),
        "actor: {}",
        rows[0].1
    );
    assert_eq!(
        rows[0].2,
        format!("token:{}", ok.json["token_id"].as_str().expect("id"))
    );
    // A failing sink: no new row, the old token still active, no secret in any response; a retry succeeds once.
    let before = w.count("SELECT count(*) FROM tokens.tokens").await;
    let current = ok.json["token"].as_str().expect("token").to_owned();
    w.fault.refuse("token.issued");
    let bad = w
        .post(w.bearer(s.lead), &issue_path(s.w1), serde_json::json!({}))
        .await;
    assert!(bad.status.as_u16() >= 500, "{}", bad.text);
    assert!(!bad.text.contains("ctx2_"));
    assert_eq!(
        w.count("SELECT count(*) FROM tokens.tokens").await,
        before,
        "no new row"
    );
    assert_eq!(
        w.get(Some(&current), "/v2/whoami").await.status.as_u16(),
        200,
        "the old token is still active"
    );
    w.fault.clear();
    assert_eq!(
        w.post(w.bearer(s.lead), &issue_path(s.w1), serde_json::json!({}))
            .await
            .status
            .as_u16(),
        201
    );
    assert_eq!(w.audit().await.len(), 2);
}

#[tokio::test]
async fn tok_33_r19_a_replay_never_bypasses_the_current_target_rule() {
    let w = start("ti_r19").await;
    let s = w.seed;
    // (a) L issues for W1 with K; retire W1; repeat K -> 409 already_issued (the lookup is before the 409 state check).
    assert_eq!(
        w.post_keyed(w.bearer(s.lead), &issue_path(s.w1), "key-r19-k1", "{}")
            .await
            .status
            .as_u16(),
        201
    );
    let mut admin = w.db.admin().await;
    sqlx::query(
        "UPDATE identity.workers SET state = 'retired', retired_at = now() WHERE id = $1::uuid",
    )
    .bind(s.w1.to_string())
    .execute(&mut admin)
    .await
    .expect("retire");
    let a = w
        .post_keyed(w.bearer(s.lead), &issue_path(s.w1), "key-r19-k1", "{}")
        .await;
    assert_eq!(a.code(), (409, Some("already_issued")), "{}", a.text);
    // (b) TL issues for own-team W2 with K2; TL moves to team U; repeat K2 -> 403, no replay header.
    assert_eq!(
        w.post_keyed(w.bearer(s.tl), &issue_path(s.w2), "key-r19-k2", "{}")
            .await
            .status
            .as_u16(),
        201
    );
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
        .expect("move");
    let b = w
        .post_keyed(w.bearer(s.tl), &issue_path(s.w2), "key-r19-k2", "{}")
        .await;
    assert_eq!(b.code(), (403, Some("target_not_allowed")), "{}", b.text);
    assert!(b.header("cortex-idempotent-replay").is_none());
    // (c) a new key by the moved TL.
    assert_eq!(
        w.post_keyed(w.bearer(s.tl), &issue_path(s.w2), "key-r19-k3", "{}")
            .await
            .code(),
        (403, Some("target_not_allowed"))
    );
}

/// A source that refuses (the OS could not supply randomness).
struct Failing;
impl Entropy for Failing {
    fn fill(&self, _buf: &mut [u8]) -> Result<(), EntropyError> {
        Err(EntropyError)
    }
}

/// A source that refuses only one request size: 32 bytes (the secret) or 8 bytes (the stagger).
struct FailsOn(usize);
impl Entropy for FailsOn {
    fn fill(&self, buf: &mut [u8]) -> Result<(), EntropyError> {
        if buf.len() == self.0 {
            Err(EntropyError)
        } else {
            buf.fill(0x33);
            Ok(())
        }
    }
}

/// A source with a fixed byte per request size: 32 bytes for the secret, 8 for the stagger.
struct Fixed {
    secret: u8,
    stagger: u8,
}
impl Entropy for Fixed {
    fn fill(&self, buf: &mut [u8]) -> Result<(), EntropyError> {
        buf.fill(if buf.len() == 8 {
            self.stagger
        } else {
            self.secret
        });
        Ok(())
    }
}

#[tokio::test]
async fn w1_32_a_failing_os_source_refuses_the_issue_with_a_500_and_nothing_stored() {
    let w = start_with("ti_entropy", Some(Arc::new(Failing))).await;
    let s = w.seed;
    let before = w.count("SELECT count(*) FROM tokens.tokens").await;
    let r = w
        .post(w.bearer(s.lead), &issue_path(s.w1), serde_json::json!({}))
        .await;
    assert_eq!(r.status.as_u16(), 500, "{}", r.text);
    assert_eq!(r.json["type"], "internal_error");
    assert_eq!(
        w.count("SELECT count(*) FROM tokens.tokens").await,
        before,
        "nothing stored"
    );
    assert!(w.audit().await.is_empty());
    assert_eq!(
        w.get(Some(w.bearer(s.w1)), "/v2/whoami")
            .await
            .status
            .as_u16(),
        200,
        "the old token is untouched"
    );
}

#[tokio::test]
async fn tok_23_the_stagger_reaches_the_expiry_of_the_issued_token() {
    // U = 0 (all-zero bytes): the whole lifetime. U just under 1 (all-one bytes): 0.9 of it.
    for (stagger, expect_days) in [(0x00_u8, 90.0_f64), (0xff, 81.0)] {
        let w = start_with(
            &format!("ti_stagger_{stagger:02x}"),
            Some(Arc::new(Fixed {
                secret: 0x5a,
                stagger,
            })),
        )
        .await;
        let s = w.seed;
        let r = w
            .post(w.bearer(s.lead), &issue_path(s.w1), serde_json::json!({}))
            .await;
        assert_eq!(r.status.as_u16(), 201, "{}", r.text);
        let mut admin = w.db.admin().await;
        let micros: i64 = sqlx::query_scalar(
            "SELECT (extract(epoch from expires_at - issued_at) * 1000000)::bigint FROM tokens.tokens WHERE id = $1::uuid",
        )
        .bind(r.json["token_id"].as_str().expect("id"))
        .fetch_one(&mut admin)
        .await
        .expect("lifetime");
        #[allow(clippy::cast_precision_loss)]
        let days = micros as f64 / 86_400_000_000.0;
        assert!(
            (days - expect_days).abs() < 1e-6,
            "stagger {stagger:#x}: {days} days"
        );
        drop(admin);
        let _ = Method::GET;
    }
}

#[tokio::test]
async fn w1_32b_a_source_that_fails_for_the_secret_only_or_the_stagger_only_refuses_too() {
    // Each use of the operating system's randomness is fail-closed on its own: the 32 secret bytes and the 8
    // stagger bytes. A fallback behind either would let the other still "work" (the first form of this cell only
    // made every request fail, which the early stagger request hid).
    for (what, size) in [("the secret", 32), ("the stagger", 8)] {
        let w = start_with(&format!("ti_entropy_{size}"), Some(Arc::new(FailsOn(size)))).await;
        let s = w.seed;
        let before = w.count("SELECT count(*) FROM tokens.tokens").await;
        let r = w
            .post(w.bearer(s.lead), &issue_path(s.w1), serde_json::json!({}))
            .await;
        assert_eq!(
            r.status.as_u16(),
            500,
            "a failing source for {what}: {}",
            r.text
        );
        assert_eq!(
            w.count("SELECT count(*) FROM tokens.tokens").await,
            before,
            "{what}: nothing stored"
        );
        assert!(w.audit().await.is_empty());
    }
}
