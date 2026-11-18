//! `POST /v2/tokens/{id}/revoke` (`tokens.revoke`, scope `token:revoke`, rule `TokenTarget`) and
//! `POST /v2/self/token/revoke` (`tokens.revoke_self`, scope `self:revoke`, rule `None`): cells TOK-50 to TOK-59.
//!
//! - **Step 0, stateless, before any read (R-16, R-3):** the reason. Absent is 422 `reason_required`; a value off
//!   the code grammar `^[a-z][a-z_]{0,31}$` (no digit, 32 at most) is 422 `invalid_request`; so are the four SYSTEM
//!   codes (`superseded`, `reissued`, `retired`, `replaced_unused`), which only the server assigns, so a caller
//!   cannot forge a token's `state`. These answer even for an unknown or foreign token id.
//! - **Revoking an ended token is a no-op that answers 200 (R-14):** the UPDATE only touches a token that is not
//!   revoked and not expired, so the original reason and time stay and no audit row is written. Two revokes of one
//!   token race on the row lock: one wins, the other is the no-op.
//! - **Audit:** one `token.revoked` (or `token.self_revoked`, reason `self`) record per COMMITTED revocation, in the
//!   same transaction, with the reason code as its only detail.

use std::sync::Arc;

use axum::http::Method;
use cortex_db::{DbError, ModuleTx};
use cortex_module::{
    AuditDetails, AuditSink, AuditTarget, Authorized, Ctx, IdentityApi, Problem, Reply, Routes,
    TargetRule,
};
use cortex_types::{TokenId, WorkerId};
use serde::{Deserialize, Serialize};

use crate::issue::{NoFields, ensure_caller_current, target_of};
use crate::problems::REASON_REQUIRED;

/// The revoke body.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevokeBody {
    pub reason: Option<String>,
}

/// `{"ok": true}`.
#[derive(Debug, Serialize)]
pub struct OkBody {
    pub ok: bool,
}

/// The codes only the server assigns (OQ-T4).
const SYSTEM_CODES: [&str; 4] = ["superseded", "reissued", "retired", "replaced_unused"];

/// `^[a-z][a-z_]{0,31}$`.
fn is_code(s: &str) -> bool {
    let b = s.as_bytes();
    (1..=32).contains(&b.len())
        && b[0].is_ascii_lowercase()
        && b.iter().all(|c| c.is_ascii_lowercase() || *c == b'_')
}

/// Step 0: the reason, validated.
pub(crate) fn reason_of(b: &RevokeBody) -> Result<&str, Problem> {
    let Some(reason) = b.reason.as_deref() else {
        return Err(Problem::new(
            &REASON_REQUIRED,
            "Send a reason code, for example `lost`.",
        ));
    };
    if !is_code(reason) {
        return Err(Problem::invalid_request(
            422,
            "The reason must be a code: lowercase letters and underscores, 1 to 32 characters.",
        ));
    }
    if SYSTEM_CODES.contains(&reason) {
        return Err(Problem::invalid_request(
            422,
            "That reason code is reserved for the system.",
        ));
    }
    Ok(reason)
}

/// The audit action of a committed revocation by someone else (S3 r2 section 23; mod-audit's registry). Details: the
/// reason code, nothing else.
pub const ACTION_REVOKED: &str = "token.revoked";

/// The audit action of a token revoking itself (reason `self`).
pub const ACTION_SELF_REVOKED: &str = "token.self_revoked";

async fn authorize(
    tx: &mut ModuleTx,
    ctx: &Ctx,
    identity: Arc<dyn IdentityApi>,
    b: &RevokeBody,
) -> Result<Authorized<TokenId>, Problem> {
    reason_of(b)?;
    let id: TokenId = ctx.path_id("id")?;
    // The caller, read again in this changing transaction, before any proof (vera's TOKENS-V1).
    ensure_caller_current(tx, ctx, &*identity).await?;
    // Another project's token is invisible (row-level security): the same 404 as an unknown id.
    let worker: Option<String> =
        sqlx::query_scalar("SELECT worker_id::text FROM tokens.tokens WHERE id = $1::uuid")
            .bind(id.to_string())
            .fetch_optional(tx.executor()?)
            .await
            .map_err(DbError::from)?;
    let worker = worker
        .and_then(|w| WorkerId::parse(&w).ok())
        .ok_or_else(Problem::not_found)?;
    let view = identity
        .worker(tx, &worker)
        .await?
        .ok_or_else(Problem::not_found)?;
    let project = identity.project(tx).await?;
    ctx.authorize(&target_of(&view, project.lead), id)
}

/// Revokes `id` if it is neither revoked nor expired. `true` if this call did it.
async fn revoke_row(tx: &mut ModuleTx, id: TokenId, reason: &str) -> Result<bool, Problem> {
    let n = sqlx::query(
        "UPDATE tokens.tokens SET revoked_at = platform.now(), revoke_reason = $2 \
         WHERE id = $1::uuid AND revoked_at IS NULL AND expires_at > platform.now()",
    )
    .bind(id.to_string())
    .bind(reason)
    .execute(tx.executor()?)
    .await
    .map_err(DbError::from)?
    .rows_affected();
    Ok(n == 1)
}

async fn execute(
    tx: &mut ModuleTx,
    proof: Authorized<TokenId>,
    b: RevokeBody,
    audit: Arc<dyn AuditSink>,
) -> Result<Reply<OkBody>, Problem> {
    let id = *proof.value();
    let reason = reason_of(&b)?.to_owned();
    if revoke_row(tx, id, &reason).await? {
        audit
            .record(
                tx,
                ACTION_REVOKED,
                AuditTarget::Token(id),
                AuditDetails::new().code("reason", &reason)?,
            )
            .await?;
    }
    Ok(Reply::ok(OkBody { ok: true }))
}

async fn execute_self(
    tx: &mut ModuleTx,
    ctx: &Ctx,
    audit: Arc<dyn AuditSink>,
) -> Result<Reply<OkBody>, Problem> {
    let id = ctx.principal().token;
    if revoke_row(tx, id, "self").await? {
        audit
            .record(
                tx,
                ACTION_SELF_REVOKED,
                AuditTarget::Token(id),
                AuditDetails::new().code("reason", "self")?,
            )
            .await?;
    }
    Ok(Reply::ok(OkBody { ok: true }))
}

/// Adds `tokens.revoke` and `tokens.revoke_self` to `r`.
pub fn add(r: Routes, identity: Arc<dyn IdentityApi>, audit: Arc<dyn AuditSink>) -> Routes {
    let audit2 = Arc::clone(&audit);
    r.write::<_, _, RevokeBody, TokenId, OkBody>(
        Method::POST,
        "/v2/tokens/{id}/revoke",
        crate::SCOPE_TOKEN_REVOKE,
        TargetRule::TokenTarget,
        move |tx, ctx, b: &RevokeBody| Box::pin(authorize(tx, ctx, Arc::clone(&identity), b)),
        move |tx, _ctx, p: Authorized<TokenId>, b: RevokeBody| {
            Box::pin(execute(tx, p, b, Arc::clone(&audit)))
        },
    )
    .max_body(16384)
    .write_open::<_, Option<NoFields>, OkBody>(
        Method::POST,
        "/v2/self/token/revoke",
        crate::SCOPE_SELF_REVOKE,
        move |tx, ctx, _b: Option<NoFields>| Box::pin(execute_self(tx, ctx, Arc::clone(&audit2))),
    )
    .max_body(16384)
}
