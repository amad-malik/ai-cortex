//! vera's TOKENS-V1 (REVIEW.md, 6 Nov): the CALLER's membership is read again in the CHANGING transaction (S1 5.3,
//! X-02, TOK-29). The chain verifies a token in one transaction and gives the handler the principal it froze; a
//! demotion that commits AFTER the verification and BEFORE the handler's transaction used to be invisible: the
//! target's data was current, the policy still saw `TeamLead(T)`.
//!
//! The schedule is made deterministic by `common::chain::Gate`: the verifier answers, the request is HELD, the test
//! changes the caller's membership and commits, the request goes on. Needs the throwaway Postgres (`CV2_DB=1`).
//!
//! Expectations: 403 `target_not_allowed`, and NOTHING written: no token row, no audit row, no idempotency
//! reservation, no revocation.
mod common;

use common::chain::{Answer, World, start};
use cortex_types::WorkerId;

async fn admin_exec(w: &World, sql: &str, who: WorkerId) {
    let mut admin = w.db.admin().await;
    sqlx::query(sqlx::AssertSqlSafe(sql.to_owned()))
        .bind(who.to_string())
        .execute(&mut admin)
        .await
        .expect("admin exec");
}

async fn set_role(w: &World, who: WorkerId, role: &str) {
    let mut admin = w.db.admin().await;
    sqlx::query("UPDATE identity.team_members SET role = $1 WHERE worker_id = $2::uuid")
        .bind(role)
        .bind(who.to_string())
        .execute(&mut admin)
        .await
        .expect("role");
}

async fn retire(w: &World, who: WorkerId) {
    admin_exec(
        w,
        "UPDATE identity.workers SET state = 'retired', retired_at = now() WHERE id = $1::uuid",
        who,
    )
    .await;
}

/// Everything a refused request must leave alone: tokens, audit rows, idempotency reservations, active tokens.
async fn written(w: &World) -> (i64, i64, i64, i64) {
    (
        w.count("SELECT count(*) FROM tokens.tokens").await,
        w.count("SELECT count(*) FROM plug_audit.events").await,
        w.count("SELECT count(*) FROM platform.idempotency").await,
        w.count("SELECT count(*) FROM tokens.tokens WHERE revoked_at IS NULL")
            .await,
    )
}

fn issue_path(w: WorkerId) -> String {
    format!("/v2/workers/{w}/tokens")
}

/// Run `request` held after verification; `change` happens while it is held.
async fn held(
    w: &World,
    caller: WorkerId,
    path: String,
    body: serde_json::Value,
    change: impl std::future::Future<Output = ()>,
) -> Answer {
    w.gate.arm(caller);
    let handle = w.spawn_post(w.bearer(caller), &path, body);
    w.gate.wait_reached().await;
    change.await;
    w.gate.release();
    handle.await.expect("the held request finishes")
}

#[tokio::test]
async fn rw_10_a_demotion_between_verification_and_issue_is_403_and_writes_nothing() {
    let w = start("rf_issue_demote").await;
    let s = w.seed;
    let before = written(&w).await;
    let a = held(
        &w,
        s.tl,
        issue_path(s.w1),
        serde_json::json!({}),
        set_role(&w, s.tl, "member"),
    )
    .await;
    assert_eq!(a.code(), (403, Some("target_not_allowed")), "{}", a.text);
    assert_eq!(written(&w).await, before, "no token, audit or reservation");
}

#[tokio::test]
async fn rw_11_a_demotion_between_verification_and_revoke_is_403_and_the_token_stays_active() {
    let w = start("rf_revoke_demote").await;
    let s = w.seed;
    let before = written(&w).await;
    let target = w.token_id(s.w1);
    let a = held(
        &w,
        s.tl,
        format!("/v2/tokens/{target}/revoke"),
        serde_json::json!({"reason": "lost"}),
        set_role(&w, s.tl, "member"),
    )
    .await;
    assert_eq!(a.code(), (403, Some("target_not_allowed")), "{}", a.text);
    assert_eq!(written(&w).await, before);
    assert_eq!(
        w.count(format!(
            "SELECT count(*) FROM tokens.tokens WHERE id = '{target}' AND revoked_at IS NULL"
        ))
        .await,
        1,
        "W1's token is still active"
    );
}

#[tokio::test]
async fn rw_12_a_caller_retired_between_verification_and_the_change_is_403_and_writes_nothing() {
    let w = start("rf_retired").await;
    let s = w.seed;
    let before = written(&w).await;
    let a = held(
        &w,
        s.tl,
        issue_path(s.w1),
        serde_json::json!({}),
        retire(&w, s.tl),
    )
    .await;
    assert_eq!(a.code(), (403, Some("target_not_allowed")), "{}", a.text);
    assert_eq!(written(&w).await, before);
}

#[tokio::test]
async fn rw_13_a_move_to_another_team_between_verification_and_issue_is_403_too() {
    let w = start("rf_move").await;
    let s = w.seed;
    let before = written(&w).await;
    // TL leads team T. While the request is held TL becomes the lead of team U instead (UL steps down first, so the
    // one-lead-per-team rule holds): the frozen principal says TeamLead(T), the truth says TeamLead(U).
    let u = s.u;
    let change = async {
        let mut admin = w.db.admin().await;
        sqlx::query("UPDATE identity.team_members SET role = 'member' WHERE worker_id = $1::uuid")
            .bind(s.ul.to_string())
            .execute(&mut admin)
            .await
            .expect("UL steps down");
        sqlx::query(
            "UPDATE identity.team_members SET team_id = $2::uuid WHERE worker_id = $1::uuid",
        )
        .bind(s.tl.to_string())
        .bind(u.to_string())
        .execute(&mut admin)
        .await
        .expect("TL moves");
    };
    let a = held(&w, s.tl, issue_path(s.w1), serde_json::json!({}), change).await;
    assert_eq!(a.code(), (403, Some("target_not_allowed")), "{}", a.text);
    assert_eq!(written(&w).await, before);
}

#[tokio::test]
async fn rw_14_control_nothing_changes_during_the_pause_and_the_issue_succeeds_once() {
    let w = start("rf_control").await;
    let s = w.seed;
    let before = written(&w).await;
    let a = held(&w, s.tl, issue_path(s.w1), serde_json::json!({}), async {}).await;
    assert_eq!(a.status.as_u16(), 201, "{}", a.text);
    let after = written(&w).await;
    // One new token row, one audit row (`token.issued`), W1's old token superseded.
    assert_eq!(after.0, before.0 + 1);
    assert_eq!(after.1, before.1 + 1);
    let audit = w.audit().await;
    assert_eq!(audit.last().map(|r| r.0.as_str()), Some("token.issued"));
}

#[tokio::test]
async fn rw_15_a_demoted_caller_can_still_revoke_its_own_token() {
    // `revoke_self` needs no role: the caller's authority is the token it holds. A demotion during the pause must
    // not turn it into a 403 (the fix must not over-block).
    let w = start("rf_self").await;
    let s = w.seed;
    let own = w.token_id(s.tl);
    let a = held(
        &w,
        s.tl,
        "/v2/self/token/revoke".to_owned(),
        serde_json::json!({}),
        set_role(&w, s.tl, "member"),
    )
    .await;
    assert_eq!(a.status.as_u16(), 200, "{}", a.text);
    assert_eq!(
        w.count(format!(
            "SELECT count(*) FROM tokens.tokens WHERE id = '{own}' AND revoked_at IS NOT NULL"
        ))
        .await,
        1
    );
}
