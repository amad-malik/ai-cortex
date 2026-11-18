//! `POST /v2/workers/{id}/tokens` (`tokens.issue`, scope `token:issue`, rule `TokenTarget`): cells TOK-21 to
//! TOK-33 and the randomness condition of cox W1-32. Two-phase (delta 4), like every write of this piece.
//!
//! - **`authorize` (reads only, no lock):** the path id (404 if malformed), the CALLER read again in this changing
//!   transaction (403 if its role, team or state is not what the verifier froze, [`caller_is_current`]), the worker
//!   through `IdentityApi` in the token's project (404 if unknown or another project's), the project lead, and
//!   `Ctx::authorize` with a `Target` built from what was read (403). Nothing from the request body can reach the `Target`.
//! - **`execute` (after the chain's replay lookup):** the per-worker lock, the worker again (404, or 409
//!   `target_retired`), the randomness (before any write: a failure is 500 and stores nothing), the generation
//!   counters, the supersession of the old active token as `reissued`, the new row, the audit record. The reply
//!   sets `Reply.issued`, so the chain stores only a marker naming the token id: **a token is never stored or
//!   replayed** (a retry is 409 `already_issued`).
//!
//! The issue itself is shared with the host path: [`issue_in_host_tx`] (piece 11's bootstrap, inside its own host
//! transaction) and `host::issue_as_host`.

use std::sync::Arc;

use axum::http::Method;
use cortex_db::{Actor, AdvisoryClass, DbError, ModuleTx};
use cortex_module::{
    AuditDetails, AuditSink, AuditTarget, Authorized, Ctx, IdentityApi, PortError, Principal,
    Problem, Reply, Role, Routes, Status, Target, TargetRule, WorkerView,
};
use cortex_types::{Clock, Secret, TokenId, WorkerId};
use serde::{Deserialize, Serialize};

use crate::entropy::Entropy;
use crate::lifetime::{MAX_LIFETIME_DAYS, MIN_LIFETIME_DAYS, expiry};
use crate::problems::TARGET_RETIRED;
use crate::time::rfc3339_micros;
use crate::token::{self, EntropyError};

/// The issue body: nothing. An empty body or `{}` is accepted; any field is an unknown field (422).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoFields {}

/// The 201 body: the token, shown once, its id and its expiry. `Debug` redacts the token.
#[derive(Serialize)]
pub struct IssuedBody {
    token: String,
    token_id: String,
    expires_at: String,
}

impl std::fmt::Debug for IssuedBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IssuedBody")
            .field("token", &"<redacted>")
            .field("token_id", &self.token_id)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// Why an issue stopped. Mapped to a `Problem` for a route, to an error for the host path.
#[derive(Debug)]
pub enum IssueError {
    /// The worker is unknown (or another project's).
    NotFound,
    /// The worker is retired.
    Retired,
    /// A port refused.
    Port(PortError),
    /// The database refused.
    Db(DbError),
    /// The operating system could not supply randomness.
    Entropy,
}

impl From<PortError> for IssueError {
    fn from(e: PortError) -> Self {
        IssueError::Port(e)
    }
}
impl From<DbError> for IssueError {
    fn from(e: DbError) -> Self {
        IssueError::Db(e)
    }
}
impl From<sqlx::Error> for IssueError {
    fn from(e: sqlx::Error) -> Self {
        IssueError::Db(DbError::from(e))
    }
}
impl From<EntropyError> for IssueError {
    fn from(_: EntropyError) -> Self {
        IssueError::Entropy
    }
}

impl From<IssueError> for Problem {
    fn from(e: IssueError) -> Problem {
        match e {
            IssueError::NotFound => Problem::not_found(),
            IssueError::Retired => Problem::new(&TARGET_RETIRED, "That worker is retired."),
            IssueError::Port(p) => p.into(),
            IssueError::Db(d) => d.into(),
            // Fail closed: a fixed internal error, nothing stored, no text from the source.
            IssueError::Entropy => DbError::Other("tokens: no randomness".into()).into(),
        }
    }
}

/// The audit action of a token minted by the issue route AND by the FIRST bootstrap run (S3 r2 section 23, W1-26;
/// mod-audit's registry).
pub const ACTION_ISSUED: &str = "token.issued";

/// The audit action of a token re-minted by a bootstrap RE-RUN only (W1-26). A route issue that supersedes a prior
/// token is still [`ACTION_ISSUED`].
pub const ACTION_REISSUED: &str = "token.reissued";

/// Which bootstrap run a host mint belongs to (W1-26; vera's TOKENS-V2). The host path says it; it is NEVER
/// guessed from whether the worker happens to have an active token. Piece 11's bootstrap takes it from
/// `bootstrap_project`'s `reissued` boolean.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootstrapRun {
    /// The first run: the lead's token is `token.issued`.
    First,
    /// A re-run: the lead's token is `token.reissued`.
    Rerun,
}

impl BootstrapRun {
    /// The audit action this run records.
    pub const fn audit_action(self) -> &'static str {
        match self {
            BootstrapRun::First => ACTION_ISSUED,
            BootstrapRun::Rerun => ACTION_REISSUED,
        }
    }
}

/// Whether the CALLER, as the changing transaction reads it now, is still who the verifier froze (S1 5.3, X-02;
/// vera's TOKENS-V1): the same worker, active, of the same kind and in the same role and team. Anything else, a
/// demotion, a move, a retirement or a missing row, is `false`, and the handler refuses before it asks the policy:
/// the policy was fed the frozen role, so it must not be asked about a role the caller no longer has. Pure.
pub fn caller_is_current(frozen: &Principal, now: Option<&WorkerView>) -> bool {
    now.is_some_and(|v| {
        v.id == frozen.worker && v.active && v.kind == frozen.kind && v.role == frozen.role
    })
}

/// Reads the caller again in THIS transaction and refuses with 403 `target_not_allowed` if [`caller_is_current`] is
/// false. Called by the writes whose authority is the caller's role, before `Ctx::authorize`; it writes nothing.
pub(crate) async fn ensure_caller_current(
    tx: &mut ModuleTx,
    ctx: &Ctx,
    identity: &dyn IdentityApi,
) -> Result<(), Problem> {
    let frozen = ctx.principal();
    let now = identity.worker(tx, &frozen.worker).await?;
    if caller_is_current(frozen, now.as_ref()) {
        Ok(())
    } else {
        Err(Problem::target_not_allowed())
    }
}

/// A new token as the issue returns it.
pub struct Issued {
    pub token: Secret,
    pub id: TokenId,
    pub expires_at_micros: i64,
}

/// What the issue needs besides the transaction.
pub struct Issuing<'a> {
    pub identity: &'a dyn IdentityApi,
    pub audit: &'a dyn AuditSink,
    pub clock: &'a dyn Clock,
    pub entropy: &'a dyn Entropy,
}

/// The team a worker view puts it in.
pub(crate) fn team_of(role: &Role) -> Option<cortex_types::TeamId> {
    match role {
        Role::TeamLead(t) => Some(*t),
        Role::Member(t) => *t,
        Role::ProjectLead => None,
    }
}

/// The `Target` for a worker: only what was read.
pub(crate) fn target_of(view: &WorkerView, lead: WorkerId) -> Target {
    Target {
        worker: Some(view.id),
        kind: Some(view.kind),
        team: team_of(&view.role),
        lead: Some(lead),
        ..Target::default()
    }
}

/// Issues a token for `target` inside `tx` (the chain's, or the host path's) and records `action`. The caller has
/// already decided that this caller may; this function re-reads the worker under its own lock.
pub(crate) async fn issue_in_tx(
    tx: &mut ModuleTx,
    k: &Issuing<'_>,
    target: WorkerId,
    issued_by: serde_json::Value,
    action: &'static str,
) -> Result<Issued, IssueError> {
    // One issue per worker at a time: the Idempotency lock (the chain's) comes first, this gate second.
    let key: i32 = sqlx::query_scalar("SELECT hashtext($1::text)")
        .bind(target.to_string())
        .fetch_one(tx.executor()?)
        .await?;
    tx.advisory_lock(AdvisoryClass::ProducerGate, key).await?;
    let view = k
        .identity
        .worker(tx, &target)
        .await?
        .ok_or(IssueError::NotFound)?;
    if !view.active {
        return Err(IssueError::Retired);
    }
    let project = k.identity.project(tx).await?;
    // The randomness first: a failure stores nothing (W1-32).
    let u = k.entropy.unit()?;
    let generated = token::generate_with(k.clock, &mut |buf: &mut [u8]| k.entropy.fill(buf))?;
    // The injected clock, not `platform.now()`: the host role has no USAGE on the `platform` schema, and the
    // route and the host path must agree on one `now`.
    let now = k.clock.now().unix_micros();
    // The PROJECT's setting (S1 A3, 7 to 365 days; the identity port carries it since wave1/ports3). The database's
    // CHECK keeps it in range; a value outside is refused here too, writing nothing.
    let days = project.token_lifetime_days;
    if !(MIN_LIFETIME_DAYS..=MAX_LIFETIME_DAYS).contains(&days) {
        return Err(IssueError::Port(PortError::Invalid("token lifetime")));
    }
    let (expires, renew) = expiry(now, days, u);
    // The generation counters now; a missing row is generation 0 (DV-4).
    let (install_gen, project_gen): (i64, i64) = sqlx::query_as(
        "SELECT coalesce((SELECT gen FROM tokens.generations WHERE scope = 'install' AND scope_id = $1::uuid), 0), \
                coalesce((SELECT gen FROM tokens.generations WHERE scope = 'project' AND scope_id = $2::uuid), 0)",
    )
    .bind(project.installation.to_string())
    .bind(project.id.to_string())
    .fetch_one(tx.executor()?)
    .await?;
    // Supersede: exactly one active token per worker.
    sqlx::query(
        "UPDATE tokens.tokens \
         SET revoked_at = timestamptz 'epoch' + $2::bigint * interval '1 microsecond', revoke_reason = 'reissued' \
         WHERE worker_id = $1::uuid AND revoked_at IS NULL \
           AND expires_at > timestamptz 'epoch' + $2::bigint * interval '1 microsecond'",
    )
    .bind(target.to_string())
    .bind(now)
    .execute(tx.executor()?)
    .await?;
    sqlx::query(
        "INSERT INTO tokens.tokens (id, worker_id, project_id, digest, install_gen, project_gen, \
                                    issued_at, expires_at, renew_after, issued_by) \
         VALUES ($1::uuid, $2::uuid, $3::uuid, $4, $5, $6, \
                 timestamptz 'epoch' + $7::bigint * interval '1 microsecond', \
                 timestamptz 'epoch' + $8::bigint * interval '1 microsecond', \
                 timestamptz 'epoch' + $9::bigint * interval '1 microsecond', $10::text::jsonb)",
    )
    .bind(generated.id().to_string())
    .bind(target.to_string())
    .bind(project.id.to_string())
    .bind(generated.digest().to_vec())
    .bind(install_gen)
    .bind(project_gen)
    .bind(now)
    .bind(expires)
    .bind(renew)
    .bind(issued_by.to_string())
    .execute(tx.executor()?)
    .await?;
    k.audit
        .record(
            tx,
            action,
            AuditTarget::Token(generated.id()),
            AuditDetails::new(),
        )
        .await?;
    let id = generated.id();
    Ok(Issued {
        token: generated.into_token(),
        id,
        expires_at_micros: expires,
    })
}

/// Issues a token for `target` INSIDE the caller's host transaction (`ModuleTx::begin_host`): piece 11's Minter, in
/// the transaction that also holds the project, the lead and the `project.bootstrapped` row, so all of them commit
/// or roll back together and the secret can be written to its file BEFORE the commit. `issued_by` is
/// `{"kind":"host"}`; the audit action is `run.audit_action()` (`token.issued` or `token.reissued`). The caller
/// commits.
///
/// **On an error the caller MUST roll back and never commit.** This function writes in several steps inside the
/// caller's transaction (the supersession of the old token, the new row, the audit record); an error after the first
/// of them (a failing audit sink, say) leaves those writes pending in that transaction, and nothing here undoes
/// them. The route and `issue_as_host` roll back on every error; a caller that owns the transaction does it itself
/// (cell `tok_32d`). Randomness is drawn before any write, so a randomness failure leaves nothing pending.
pub async fn issue_in_host_tx(
    tx: &mut ModuleTx,
    k: &Issuing<'_>,
    target: WorkerId,
    run: BootstrapRun,
) -> Result<Issued, IssueError> {
    issue_in_tx(
        tx,
        k,
        target,
        serde_json::json!({"kind": "host"}),
        run.audit_action(),
    )
    .await
}

/// Who issued, as the row stores it: the actor's kind and id, never a secret.
fn issued_by(actor: &Actor) -> serde_json::Value {
    match actor {
        Actor::Worker(w) => serde_json::json!({"kind": "worker", "id": w.to_string()}),
        Actor::Service(w) => serde_json::json!({"kind": "service", "id": w.to_string()}),
        Actor::HostOperator { .. } => serde_json::json!({"kind": "host"}),
        Actor::Recovery => serde_json::json!({"kind": "recovery"}),
        Actor::System(_) => serde_json::json!({"kind": "system"}),
    }
}

async fn authorize(
    tx: &mut ModuleTx,
    ctx: &Ctx,
    identity: Arc<dyn IdentityApi>,
    _b: &Option<NoFields>,
) -> Result<Authorized<WorkerId>, Problem> {
    let target: WorkerId = ctx.path_id("id")?;
    // The caller, read again in this changing transaction, before any proof (vera's TOKENS-V1).
    ensure_caller_current(tx, ctx, &*identity).await?;
    let view = identity
        .worker(tx, &target)
        .await?
        .ok_or_else(Problem::not_found)?;
    let project = identity.project(tx).await?;
    ctx.authorize(&target_of(&view, project.lead), target)
}

#[allow(clippy::too_many_arguments)]
async fn execute(
    tx: &mut ModuleTx,
    ctx: &Ctx,
    proof: Authorized<WorkerId>,
    identity: Arc<dyn IdentityApi>,
    audit: Arc<dyn AuditSink>,
    clock: Arc<dyn Clock>,
    entropy: Arc<dyn Entropy>,
) -> Result<Reply<IssuedBody>, Problem> {
    let target = *proof.value();
    let k = Issuing {
        identity: &*identity,
        audit: &*audit,
        clock: &*clock,
        entropy: &*entropy,
    };
    let issued = issue_in_tx(tx, &k, target, issued_by(&ctx.actor()), ACTION_ISSUED).await?;
    Ok(Reply {
        status: Status::Created,
        body: IssuedBody {
            token: issued.token.expose().to_owned(),
            token_id: issued.id.to_string(),
            expires_at: rfc3339_micros(issued.expires_at_micros),
        },
        issued: Some(issued.id),
    })
}

/// Adds `tokens.issue` to `r`.
pub fn add(
    r: Routes,
    identity: Arc<dyn IdentityApi>,
    audit: Arc<dyn AuditSink>,
    clock: Arc<dyn Clock>,
    entropy: Arc<dyn Entropy>,
) -> Routes {
    let id1 = Arc::clone(&identity);
    r.write::<_, _, Option<NoFields>, WorkerId, IssuedBody>(
        Method::POST,
        "/v2/workers/{id}/tokens",
        crate::SCOPE_TOKEN_ISSUE,
        TargetRule::TokenTarget,
        move |tx, ctx, b: &Option<NoFields>| Box::pin(authorize(tx, ctx, Arc::clone(&id1), b)),
        move |tx, ctx, p: Authorized<WorkerId>, _b: Option<NoFields>| {
            Box::pin(execute(
                tx,
                ctx,
                p,
                Arc::clone(&identity),
                Arc::clone(&audit),
                Arc::clone(&clock),
                Arc::clone(&entropy),
            ))
        },
    )
    .max_body(16384)
}
