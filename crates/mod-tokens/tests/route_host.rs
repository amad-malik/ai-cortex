//! Stage D, L3 + library: `issue_as_host` (TOK-32) and the concurrency of `tokens.issue` (TOK-25). Needs the
//! throwaway Postgres (`CV2_DB=1`).
mod common;

use std::sync::Arc;

use common::chain::{World, start};
use cortex_db::HostContext;
use cortex_module::TokenVerifier;
use cortex_types::{ProjectId, RequestId, Secret, SystemClock, WorkerId};
use mod_tokens::host::{HostIssueError, HostIssued, issue_as_host};
use mod_tokens::{BootstrapRun, Issuing, OsEntropy, issue_in_host_tx};

async fn host_issue(
    w: &World,
    project: ProjectId,
    worker: WorkerId,
) -> Result<HostIssued, HostIssueError> {
    host_issue_as(w, project, worker, BootstrapRun::First).await
}

async fn host_issue_as(
    w: &World,
    project: ProjectId,
    worker: WorkerId,
    run: BootstrapRun,
) -> Result<HostIssued, HostIssueError> {
    issue_as_host(
        &w.pool,
        &*w.identity,
        &*w.audit_sink,
        &*w.clock,
        &OsEntropy,
        HostContext {
            uid: 1000,
            project: Some(project),
            request_id: RequestId::new(&SystemClock),
        },
        worker,
        run,
    )
    .await
}

#[tokio::test]
async fn tok_32_the_host_path_issues_once_supersedes_and_an_unknown_worker_writes_nothing() {
    let w = start("th_host").await;
    let s = w.seed;
    let first = host_issue(&w, s.a, s.lead)
        .await
        .expect("issued for the lead");
    // The secret authenticates; the lead's fixture token was superseded.
    let a = w.get(Some(first.token.expose()), "/v2/whoami").await;
    assert_eq!(
        (a.status.as_u16(), a.json["worker"].as_str()),
        (200, Some(s.lead.to_string().as_str()))
    );
    assert_eq!(
        w.get(Some(w.bearer(s.lead)), "/v2/whoami").await.code(),
        (401, Some("token_revoked"))
    );
    // issued_by is {"kind":"host"}; exactly one token.issued record, by the host operator.
    let mut admin = w.db.admin().await;
    let by: String =
        sqlx::query_scalar("SELECT issued_by::text FROM tokens.tokens WHERE id = $1::uuid")
            .bind(first.token_id.to_string())
            .fetch_one(&mut admin)
            .await
            .expect("row");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&by).expect("json"),
        serde_json::json!({"kind": "host"})
    );
    let rows = w.audit().await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, "token.issued");
    assert!(rows[0].1.contains("HostOperator"), "actor: {}", rows[0].1);
    // A second call supersedes the first.
    let second = host_issue(&w, s.a, s.lead).await.expect("issued again");
    assert_eq!(
        w.get(Some(first.token.expose()), "/v2/whoami").await.code(),
        (401, Some("token_revoked"))
    );
    assert_eq!(
        w.get(Some(second.token.expose()), "/v2/whoami")
            .await
            .status
            .as_u16(),
        200
    );
    // Unknown, foreign and retired workers are errors and write nothing.
    let (tokens, audits) = (
        w.count("SELECT count(*) FROM tokens.tokens").await,
        w.audit().await.len(),
    );
    assert!(matches!(
        host_issue(&w, s.a, WorkerId::new(&SystemClock)).await,
        Err(HostIssueError::UnknownWorker)
    ));
    assert!(matches!(
        host_issue(&w, s.a, s.wb1).await,
        Err(HostIssueError::UnknownWorker)
    ));
    assert!(matches!(
        host_issue(&w, s.a, s.wr).await,
        Err(HostIssueError::Retired)
    ));
    assert_eq!(w.count("SELECT count(*) FROM tokens.tokens").await, tokens);
    assert_eq!(w.audit().await.len(), audits);
    // The returned secret is not in any stored row.
    let all: Vec<String> = sqlx::query_scalar("SELECT t::text FROM tokens.tokens t")
        .fetch_all(&mut admin)
        .await
        .expect("rows");
    assert!(
        all.iter().all(
            |r| !r.contains(second.token.expose()) && !r.contains(&second.token.expose()[38..])
        )
    );
    let _: Option<Secret> = None;
}

#[tokio::test]
async fn tok_25_concurrent_issues_for_one_worker_leave_exactly_one_active_token() {
    let w = Arc::new(start("th_race").await);
    let s = w.seed;
    // 16 concurrent issues for W1, distinct keys: each 201 or 429, never another 5xx.
    let mut tasks = Vec::new();
    for _ in 0..16 {
        let w = w.clone();
        let lead = w.bearer(s.lead).to_owned();
        tasks.push(tokio::spawn(async move {
            w.post(
                &lead,
                &format!("/v2/workers/{}/tokens", s.w1),
                serde_json::json!({}),
            )
            .await
        }));
    }
    let mut secrets = Vec::new();
    for t in tasks {
        let a = t.await.expect("task");
        assert!([201, 429].contains(&a.status.as_u16()), "{}", a.text);
        if a.status.as_u16() == 201 {
            secrets.push(a.json["token"].as_str().expect("token").to_owned());
        }
    }
    assert!(!secrets.is_empty());
    assert_eq!(
        w.count(format!(
            "SELECT count(*) FROM tokens.tokens WHERE worker_id = '{}' AND revoked_at IS NULL",
            s.w1
        ))
        .await,
        1,
        "exactly one active token afterwards"
    );
    let mut ok = 0;
    for t in &secrets {
        if w.get(Some(t), "/v2/whoami").await.status.as_u16() == 200 {
            ok += 1;
        }
    }
    assert_eq!(ok, 1, "of the returned secrets exactly one verifies");
    // Different workers do not serialise on each other: six concurrent issues, six answers of 201.
    let mut tasks = Vec::new();
    for t in [s.w2, s.w3, s.w4, s.tl, s.ul] {
        let w = w.clone();
        let lead = w.bearer(s.lead).to_owned();
        tasks.push(tokio::spawn(async move {
            w.post(
                &lead,
                &format!("/v2/workers/{t}/tokens"),
                serde_json::json!({}),
            )
            .await
            .status
            .as_u16()
        }));
    }
    for t in tasks {
        assert_eq!(t.await.expect("task"), 201);
    }
    let _ = Arc::strong_count(&w);
    let _: Option<Box<dyn TokenVerifier>> = None;
}

async fn last_action(w: &World) -> Option<String> {
    w.audit().await.last().map(|r| r.0.clone())
}

/// vera's TOKENS-V2 and W1-26: the host mint carries an EXPLICIT intent. A first bootstrap run records
/// `token.issued`, a re-run `token.reissued`; the mode is never guessed from whether an old token exists, and a
/// route issue that supersedes a token stays `token.issued`.
#[tokio::test]
async fn tok_32b_the_audit_action_follows_the_mint_intent_and_not_the_presence_of_an_old_token() {
    let w = start("th_intent").await;
    let s = w.seed;
    // First run, although the lead already has an active token: token.issued.
    host_issue_as(&w, s.a, s.lead, BootstrapRun::First)
        .await
        .expect("first run");
    assert_eq!(last_action(&w).await.as_deref(), Some("token.issued"));
    // Re-run: token.reissued. (The lead's fixture token and the first run's token are both superseded now: the
    // newest secret is the only one that still authenticates.)
    let lead = host_issue_as(&w, s.a, s.lead, BootstrapRun::Rerun)
        .await
        .expect("re-run");
    assert_eq!(last_action(&w).await.as_deref(), Some("token.reissued"));
    // A re-run for a worker with NO active token is still token.reissued (the intent decides).
    let mut admin = w.db.admin().await;
    sqlx::query("UPDATE tokens.tokens SET revoked_at = now(), revoke_reason = 'lost' WHERE worker_id = $1::uuid AND revoked_at IS NULL")
        .bind(s.w1.to_string())
        .execute(&mut admin)
        .await
        .expect("revoke W1's token");
    host_issue_as(&w, s.a, s.w1, BootstrapRun::Rerun)
        .await
        .expect("re-run without an old token");
    assert_eq!(last_action(&w).await.as_deref(), Some("token.reissued"));
    // A route issue that supersedes W2's token is token.issued.
    let r = w
        .post(
            lead.token.expose(),
            &format!("/v2/workers/{}/tokens", s.w2),
            serde_json::json!({}),
        )
        .await;
    assert_eq!(r.status.as_u16(), 201, "{}", r.text);
    assert_eq!(last_action(&w).await.as_deref(), Some("token.issued"));
    // Every action written is one of the registry's four (the fixture refuses anything else).
    for (action, ..) in w.audit().await {
        assert!(
            common::audit::TOKEN_ACTIONS.contains(&action.as_str()),
            "{action}"
        );
    }
}

/// `issue_in_host_tx` runs INSIDE the caller's host transaction (piece 11's Minter): the token row and the audit
/// row commit or roll back with it, and nothing is visible from outside before the commit.
#[tokio::test]
async fn tok_32c_issue_in_host_tx_is_atomic_with_the_callers_host_transaction() {
    let w = start("th_in_tx").await;
    let s = w.seed;
    let k = Issuing {
        identity: &*w.identity,
        audit: &*w.audit_sink,
        clock: &*w.clock,
        entropy: &OsEntropy,
    };
    let host = || HostContext {
        uid: 1000,
        project: Some(s.a),
        request_id: RequestId::new(&SystemClock),
    };
    let (tokens, audits) = (
        w.count("SELECT count(*) FROM tokens.tokens").await,
        w.count("SELECT count(*) FROM plug_audit.events").await,
    );
    // Rolled back: nothing.
    let mut tx = cortex_db::ModuleTx::begin_host(&w.pool, host(), cortex_db::TxOptions::default())
        .await
        .expect("begin_host");
    let issued = issue_in_host_tx(&mut tx, &k, s.w3, BootstrapRun::First)
        .await
        .expect("issued inside the transaction");
    assert_eq!(
        w.count("SELECT count(*) FROM tokens.tokens").await,
        tokens,
        "nothing is visible before the commit"
    );
    tx.rollback().await.expect("rollback");
    assert_eq!(w.count("SELECT count(*) FROM tokens.tokens").await, tokens);
    assert_eq!(
        w.count("SELECT count(*) FROM plug_audit.events").await,
        audits
    );
    assert_eq!(
        w.get(Some(issued.token.expose()), "/v2/whoami")
            .await
            .code(),
        (401, Some("token_invalid")),
        "a rolled-back secret authenticates nothing"
    );
    // Committed: one token, one `token.issued` row, by the host operator, and the secret works.
    let mut tx = cortex_db::ModuleTx::begin_host(&w.pool, host(), cortex_db::TxOptions::default())
        .await
        .expect("begin_host");
    let issued = issue_in_host_tx(&mut tx, &k, s.w3, BootstrapRun::First)
        .await
        .expect("issued");
    tx.commit().await.expect("commit");
    assert_eq!(
        w.count("SELECT count(*) FROM tokens.tokens").await,
        tokens + 1
    );
    let rows = w.audit().await;
    assert_eq!(rows.len() as i64, audits + 1);
    let row = rows.last().expect("a row");
    assert_eq!(row.0, "token.issued");
    assert!(row.1.contains("HostOperator"), "{}", row.1);
    assert_eq!(
        w.get(Some(issued.token.expose()), "/v2/whoami")
            .await
            .status
            .as_u16(),
        200
    );
}

async fn counts(w: &World, active_sql: &str) -> (i64, i64, i64) {
    (
        w.count("SELECT count(*) FROM tokens.tokens").await,
        w.count("SELECT count(*) FROM plug_audit.events").await,
        w.count(active_sql).await,
    )
}

/// vera's TOKENS-R1: `issue_in_host_tx` writes in several steps in the CALLER'S transaction, so an error after the
/// first leaves them pending there. Here the audit sink fails AFTER its row is written (the fixture's fault): the
/// supersession and the new row are pending in the transaction, the function returns `Err`, and the caller's
/// rollback leaves NOTHING behind: no token, no audit row, the old token still active. (Committing after the `Err`
/// is what the doc forbids.)
#[tokio::test]
async fn tok_32d_an_error_inside_the_callers_host_transaction_leaves_nothing_once_the_caller_rolls_back()
 {
    let w = start("th_err_tx").await;
    let s = w.seed;
    let k = Issuing {
        identity: &*w.identity,
        audit: &*w.audit_sink,
        clock: &*w.clock,
        entropy: &OsEntropy,
    };
    let active_sql = format!(
        "SELECT count(*) FROM tokens.tokens WHERE worker_id = '{}' AND revoked_at IS NULL",
        s.lead
    );
    let before = counts(&w, &active_sql).await;
    assert_eq!(before.2, 1, "the lead has its one fixture token");
    // The sink refuses after writing its row: the issue has already superseded the old token and inserted the new.
    w.fault.refuse("token.issued");
    let mut tx = cortex_db::ModuleTx::begin_host(
        &w.pool,
        HostContext {
            uid: 1000,
            project: Some(s.a),
            request_id: RequestId::new(&SystemClock),
        },
        cortex_db::TxOptions::default(),
    )
    .await
    .expect("begin_host");
    let r = issue_in_host_tx(&mut tx, &k, s.lead, BootstrapRun::First).await;
    assert!(r.is_err(), "the failing sink is an error");
    // The caller rolls back, as the doc requires.
    tx.rollback().await.expect("rollback");
    w.fault.clear();
    assert_eq!(
        counts(&w, &active_sql).await,
        before,
        "nothing is left behind"
    );
    assert_eq!(
        w.get(Some(w.bearer(s.lead)), "/v2/whoami")
            .await
            .status
            .as_u16(),
        200,
        "the old token still authenticates"
    );
}
