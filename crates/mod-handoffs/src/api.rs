//! The routes of the read side and the claim (P2a): `GET /v2/handoffs`, `GET /v2/handoffs/{id}` and
//! `POST /v2/handoffs/{id}/claim`, and the `HandoffsApi` provider. The bodies follow S3 r2.5's `Handoff`,
//! `HandoffPage` and `HandoffState`; the order of refusals is R-16 with R-19's lookup between the 403 and the
//! 409 checks, which is why a write route is two closures (`authorize`, then `execute`).

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use axum::Json;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64URL;
use cortex_db::{DbError, ModuleTx};
use cortex_module::route::AppState;
use cortex_module::{
    AuditDetails, AuditSink, AuditTarget, Authorized, Ctx, HANDOFF_LIST_CAP, HandoffKind,
    HandoffTo, HandoffView, HandoffsApi, IdentityApi, PortError, Problem, Reply, Role, Target,
    WorkerKind, WorkerView,
};
use cortex_types::{HandoffId, ModuleId, Timestamp, WorkerId};
use serde::{Deserialize, Serialize};

use crate::body::{CancelBody, CreateBody, Kind, ReleaseBody, ReturnBody, ReviewBody};
use crate::problem::{invalid, own, refusal};
use crate::rules::{Allowed, Caller, Facts, State as St, Verb, authority, may_claim, transition};
use crate::store::{self, Row_};
use crate::validate::{
    CreateOk, ReturnOk, To, validate_accept, validate_cancel, validate_create, validate_release,
    validate_return, validate_rework,
};

/// The ports the handlers use, captured when the routes are built.
#[derive(Clone)]
pub struct Deps {
    pub identity: Arc<dyn IdentityApi>,
    pub audit: Arc<dyn AuditSink>,
}

fn module_id() -> ModuleId {
    ModuleId::parse(crate::ID).expect("a valid module id")
}

// ---- the output types (S3 r2.5) ----

#[derive(Serialize)]
pub struct ToOut {
    pub kind: &'static str,
    pub r#ref: String,
}

#[derive(Serialize)]
pub struct EvidenceRefOut {
    pub kind: String,
    pub r#ref: Option<String>,
    pub sha256: String,
    pub size: i32,
}

#[derive(Serialize)]
pub struct ReturnReportOut {
    pub round: i32,
    pub author: String,
    pub summary: String,
    pub risks: String,
    pub tests: Vec<String>,
    pub evidence: Vec<EvidenceRefOut>,
    pub at: String,
}

#[derive(Serialize)]
pub struct ReviewOut {
    pub decision: String,
    pub by: String,
    pub at: String,
    pub notes: Option<String>,
    pub self_review: bool,
}

#[derive(Serialize)]
pub struct ClosedByOut {
    pub kind: String,
    pub id: String,
}

#[derive(Serialize)]
pub struct HandoffOut {
    pub id: String,
    pub kind: &'static str,
    pub state: &'static str,
    pub to: ToOut,
    pub summary: String,
    pub next_action: String,
    pub acceptance: String,
    pub verify: Option<String>,
    pub context: Option<String>,
    pub files: Vec<String>,
    pub blockers: Vec<String>,
    pub decisions: Vec<String>,
    pub priority: i32,
    pub due_at: Option<String>,
    pub holder: Option<String>,
    pub created_by: String,
    pub created_at: String,
    pub round: i32,
    pub closed_at: Option<String>,
    pub close_reason: Option<String>,
    pub closed_by: Option<ClosedByOut>,
    pub latest_return: Option<ReturnReportOut>,
    pub latest_review: Option<ReviewOut>,
}

#[derive(Serialize)]
pub struct PageOut {
    pub items: Vec<HandoffOut>,
    pub next: Option<String>,
}

/// `HandoffState`: the answer of the moves that change a handoff.
#[derive(Serialize)]
pub struct StateOut {
    pub id: String,
    pub state: &'static str,
    pub round: i32,
}

/// A handoff row and its history, as S3's `Handoff`.
async fn build(tx: &mut ModuleTx, row: Row_) -> Result<HandoffOut, DbError> {
    let latest_return = store::latest_return(tx, row.id).await?;
    let latest_review = store::latest_review(tx, row.id).await?;
    let closed_by = if row.state.is_open() {
        None
    } else {
        store::closed_by(tx, row.id).await?
    };
    Ok(HandoffOut {
        id: row.id.to_string(),
        kind: row.kind.as_str(),
        state: row.state.as_str(),
        to: match &row.to {
            To::Worker(w) => ToOut {
                kind: "worker",
                r#ref: w.to_string(),
            },
            To::JobRole(r) => ToOut {
                kind: "job_role",
                r#ref: r.clone(),
            },
        },
        summary: row.summary,
        next_action: row.next_action,
        acceptance: row.acceptance,
        verify: row.verify,
        context: row.context,
        files: row.files,
        blockers: row.blockers,
        decisions: row.decisions,
        priority: row.priority,
        due_at: row.due_at,
        holder: row.holder.map(|w| w.to_string()),
        created_by: row.created_by.to_string(),
        created_at: row.created_at,
        round: row.round,
        closed_at: row.closed_at,
        close_reason: row.close_reason,
        closed_by: closed_by.map(|(kind, id)| ClosedByOut { kind, id }),
        latest_return: latest_return.map(|r| ReturnReportOut {
            round: r.round,
            author: r.author.to_string(),
            summary: r.summary,
            risks: r.risks,
            tests: r.tests,
            evidence: r
                .evidence
                .into_iter()
                .map(|e| EvidenceRefOut {
                    kind: e.kind,
                    r#ref: e.reference,
                    sha256: e.sha256,
                    size: e.size,
                })
                .collect(),
            at: r.at,
        }),
        latest_review: latest_review.map(|v| ReviewOut {
            decision: v.decision,
            by: v.by.to_string(),
            at: v.at,
            notes: v.notes,
            self_review: v.self_review,
        }),
    })
}

// ---- show ----

/// `GET /v2/handoffs/{id}`: every packet field, the latest return and review, and who closed it. Any
/// worker with `handoff:read` may read any handoff of the project; another project's is a 404.
pub async fn show(st: &AppState, ctx: &Ctx) -> Result<Json<HandoffOut>, Problem> {
    let id: HandoffId = ctx.path_id("id")?;
    let mut tx = ctx.begin(&st.pool, &module_id()).await?;
    let row = store::load(&mut tx, id)
        .await?
        .ok_or_else(Problem::not_found)?;
    let out = build(&mut tx, row).await?;
    tx.rollback().await?;
    Ok(Json(out))
}

// ---- list and mine ----

/// `GET /v2/handoffs`: strict, so an unknown parameter is 422 (R-4; every query struct derives
/// `deny_unknown_fields`, vera r5).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListQuery {
    pub mine: Option<bool>,
    pub state: Option<String>,
    pub holder: Option<String>,
    pub created_by: Option<String>,
    pub limit: Option<i64>,
    pub after: Option<String>,
}

const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;
const CURSOR_TAG: &str = "h1";

fn cursor_encode(priority: i32, created_at: &str, id: HandoffId) -> String {
    B64URL.encode(format!("{CURSOR_TAG}|{priority}|{created_at}|{id}"))
}

/// A cursor is a position and nothing else; one that does not decode, or was not issued by this list, is 422.
fn cursor_decode(c: &str) -> Result<(i32, String, HandoffId), Problem> {
    let bad = || invalid("after", "not a cursor of this list");
    let raw = B64URL.decode(c).map_err(|_| bad())?;
    let text = String::from_utf8(raw).map_err(|_| bad())?;
    let mut parts = text.splitn(4, '|');
    let (Some(tag), Some(p), Some(at), Some(id)) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(bad());
    };
    if tag != CURSOR_TAG {
        return Err(bad());
    }
    let priority = p.parse::<i32>().map_err(|_| bad())?;
    if !crate::validate::is_rfc3339(at) {
        return Err(bad());
    }
    let id = HandoffId::parse(id).map_err(|_| bad())?;
    Ok((priority, at.to_owned(), id))
}

fn uuid_filter(field: &str, v: &Option<String>) -> Result<Option<WorkerId>, Problem> {
    v.as_deref()
        .map(|s| WorkerId::parse(s).map_err(|_| invalid(field, "not a worker id")))
        .transpose()
}

/// The caller as the lists and the claim see them: active, with the job roles they hold.
async fn caller_view(d: &Deps, tx: &mut ModuleTx, ctx: &Ctx) -> Result<WorkerView, Problem> {
    d.identity
        .worker(tx, &ctx.principal().worker)
        .await
        .map_err(Problem::from)?
        .ok_or_else(Problem::not_found)
}

pub async fn list(
    d: &Deps,
    st: &AppState,
    ctx: &Ctx,
    q: ListQuery,
) -> Result<Json<PageOut>, Problem> {
    // Step 0: the query, all of it, before any read.
    let limit = match q.limit {
        None => DEFAULT_LIMIT,
        Some(n) if (1..=MAX_LIMIT).contains(&n) => n,
        Some(_) => return Err(invalid("limit", "must be 1 to 200")),
    };
    let state = match q.state.as_deref() {
        None | Some("all") => None,
        Some(s) if St::parse(s).is_some() => Some(s.to_owned()),
        Some(_) => return Err(invalid("state", "not a state")),
    };
    let holder = uuid_filter("holder", &q.holder)?;
    let created_by = uuid_filter("created_by", &q.created_by)?;
    let after = q.after.as_deref().map(cursor_decode).transpose()?;

    let mut tx = ctx.begin(&st.pool, &module_id()).await?;
    let me = caller_view(d, &mut tx, ctx).await?;
    let lead = d
        .identity
        .project(&mut tx)
        .await
        .map_err(Problem::from)?
        .lead;
    let rows = store::list(
        &mut tx,
        &store::ListFilter {
            state: state.as_deref(),
            holder,
            created_by,
            mine: q.mine.unwrap_or(false),
            me: me.id,
            me_active: me.active,
            roles: &me.job_roles,
            lead,
            after: after.as_ref().map(|(p, c, i)| (*p, c.as_str(), *i)),
            limit: limit + 1,
        },
    )
    .await?;
    let more = rows.len() as i64 > limit;
    let rows: Vec<Row_> = rows.into_iter().take(limit as usize).collect();
    let next = if more {
        rows.last()
            .map(|r| cursor_encode(r.priority, &r.created_at, r.id))
    } else {
        None
    };
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        items.push(build(&mut tx, row).await?);
    }
    tx.rollback().await?;
    Ok(Json(PageOut { items, next }))
}

// ---- claim ----

/// A claim takes no body (S3 declares none); `{}` is accepted, anything else is refused.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoBody {}

/// The first phase: step 0 (nothing to check), the read (404), the target rule (403). It reads, takes no
/// lock and writes nothing.
pub async fn claim_authorize(
    d: &Deps,
    tx: &mut ModuleTx,
    ctx: &Ctx,
) -> Result<Authorized<HandoffId>, Problem> {
    let id: HandoffId = ctx.path_id("id")?;
    let row = store::load(tx, id).await?.ok_or_else(Problem::not_found)?;
    let me = caller_view(d, tx, ctx).await?;
    let caller = Caller {
        id: me.id,
        job_roles: me.job_roles.clone(),
        active: me.active,
    };
    if !may_claim(&row.to, row.rework_holder, &caller) {
        return Err(Problem::target_not_allowed());
    }
    // The declared rule is `SelfWorker`: the worker the handoff is addressed to (after a rework, the
    // worker who returned it). A job-role address was matched above by the shared predicate, so the
    // caller is the worker the rule is asked about.
    let worker = match (&row.to, row.rework_holder) {
        (_, Some(returner)) => returner,
        (To::Worker(w), None) => *w,
        (To::JobRole(_), None) => caller.id,
    };
    let target = Target {
        worker: Some(worker),
        kind: Some(WorkerKind::Worker),
        ..Target::default()
    };
    ctx.authorize(&target, id)
}

/// The second phase: the 409 checks under the row lock, then the write, the log row and the audit row.
pub async fn claim_execute(
    d: &Deps,
    tx: &mut ModuleTx,
    ctx: &Ctx,
    auth: Authorized<HandoffId>,
) -> Result<Reply<StateOut>, Problem> {
    let id = auth.into_value();
    let caller = ctx.principal().worker;
    let row = store::load_for_update(tx, id)
        .await?
        .ok_or_else(Problem::not_found)?;
    // The address is checked again on the locked row: the role may have gone since the first phase.
    let me = caller_view(d, tx, ctx).await?;
    let c = Caller {
        id: me.id,
        job_roles: me.job_roles,
        active: me.active,
    };
    if !may_claim(&row.to, row.rework_holder, &c) {
        return Err(Problem::target_not_allowed());
    }
    let facts = Facts {
        state: row.state,
        caller,
        creator: row.created_by,
        holder: row.holder,
        author: None,
        // Not used by a claim.
        project_lead: caller,
    };
    transition(Verb::Claim, &facts, false).map_err(refusal)?;
    let (seq, incarnation) = store::claim_write(tx, id, caller).await?;
    store::log_transition(
        tx,
        &store::LogRow {
            id,
            seq,
            from: Some("pending"),
            to: "claimed",
            actor: caller,
            incarnation,
            reason: None,
        },
    )
    .await?;
    d.audit
        .record(
            tx,
            "handoff.claimed",
            AuditTarget::Handoff(id),
            AuditDetails::new(),
        )
        .await
        .map_err(Problem::from)?;
    Ok(Reply::ok(StateOut {
        id: id.to_string(),
        state: "claimed",
        round: row.round,
    }))
}

// ---- create ----

/// `HandoffRef`: the answer of a create.
#[derive(Serialize)]
pub struct RefOut {
    pub id: String,
    pub state: &'static str,
}

/// What the first phase learned and the second phase needs: the validated packet and, for a worker
/// addressee, its view as read.
pub struct CreatePlan {
    ok: CreateOk,
    addressee: Option<WorkerView>,
}

/// The team a worker belongs to, from its role.
fn team_of(w: &WorkerView) -> Option<cortex_types::TeamId> {
    match w.role {
        Role::TeamLead(t) => Some(t),
        Role::Member(t) => t,
        Role::ProjectLead => None,
    }
}

/// The Target of a consult (cox W1-9): `consult: true` and every other field at its default. It is set from the
/// TOP-LEVEL `kind` of the body only (never from `to.kind`), and by this route alone; every task is `false`.
fn consult_target() -> Target {
    Target {
        consult: true,
        ..Target::default()
    }
}

/// The first phase of `POST /v2/handoffs`: step 0 (the whole body), the read of a worker addressee (404),
/// the target rule (403). It reads, takes no lock and writes nothing.
pub async fn create_authorize(
    d: &Deps,
    tx: &mut ModuleTx,
    ctx: &Ctx,
    body: &CreateBody,
) -> Result<Authorized<CreatePlan>, Problem> {
    let ok = validate_create(body.clone())?;
    // 404: a worker addressee must be a worker of this project (another project's is invisible).
    let addressee = match &ok.to {
        To::Worker(w) => {
            let v = d
                .identity
                .worker(tx, w)
                .await
                .map_err(Problem::from)?
                .ok_or_else(Problem::not_found)?;
            if v.kind != WorkerKind::Worker {
                return Err(Problem::not_found());
            }
            Some(v)
        }
        To::JobRole(_) => None,
    };
    // 403: a consult is open to any worker; a task is the policy's `TaskAddressee`.
    let target = match ok.kind {
        Kind::Consult => consult_target(),
        Kind::Task => {
            let lead = d.identity.project(tx).await.map_err(Problem::from)?.lead;
            Target {
                worker: addressee.as_ref().map(|v| v.id),
                kind: addressee.as_ref().map(|v| v.kind),
                team: addressee.as_ref().and_then(team_of),
                lead: Some(lead),
                // A task is never a consult, whatever its address says (cox W1-9).
                consult: false,
            }
        }
    };
    ctx.authorize(&target, CreatePlan { ok, addressee })
}

/// The second phase: the 409 (a retired addressee), the 422 that needs a read (a role nobody holds), then
/// the write, the log row and the audit row.
pub async fn create_execute(
    d: &Deps,
    tx: &mut ModuleTx,
    ctx: &Ctx,
    auth: Authorized<CreatePlan>,
) -> Result<Reply<RefOut>, Problem> {
    let CreatePlan { ok, addressee } = auth.into_value();
    if let Some(v) = addressee {
        // Read again under this transaction: the addressee may have been retired since the first phase.
        let now = d
            .identity
            .worker(tx, &v.id)
            .await
            .map_err(Problem::from)?
            .ok_or_else(Problem::not_found)?;
        if !now.active {
            return Err(own(
                "target_retired",
                "The addressee has been retired; address an active worker or a job role.",
            ));
        }
    }
    if let To::JobRole(role) = &ok.to {
        let roster = d.identity.roster(tx).await.map_err(Problem::from)?;
        if !roster
            .iter()
            .any(|w| w.active && w.job_roles.iter().any(|r| r == role))
        {
            return Err(own(
                "unknown_addressee",
                "No active worker holds that job role.",
            ));
        }
    }
    let creator = ctx.principal().worker;
    let id = HandoffId::new(&cortex_types::SystemClock);
    store::insert_handoff(tx, id, creator, &ok).await?;
    store::log_transition(
        tx,
        &store::LogRow {
            id,
            seq: 1,
            from: None,
            to: "pending",
            actor: creator,
            incarnation: 0,
            reason: None,
        },
    )
    .await?;
    d.audit
        .record(
            tx,
            "handoff.created",
            AuditTarget::Handoff(id),
            AuditDetails::new(),
        )
        .await
        .map_err(Problem::from)?;
    Ok(Reply::created(RefOut {
        id: id.to_string(),
        state: "pending",
    }))
}

// ---- the moves on a handoff that is already there: return, accept, rework, release, cancel ----

/// The facts a decision needs, read from the locked (or, in the first phase, unlocked) row.
async fn facts(d: &Deps, tx: &mut ModuleTx, ctx: &Ctx, row: &Row_) -> Result<Facts, Problem> {
    let author = if row.state == St::Returned {
        store::latest_return_author(tx, row.id).await?
    } else {
        None
    };
    let project_lead = d.identity.project(tx).await.map_err(Problem::from)?.lead;
    Ok(Facts {
        state: row.state,
        caller: ctx.principal().worker,
        creator: row.created_by,
        holder: row.holder,
        author,
        project_lead,
    })
}

/// The log row and the audit row every move leaves, written last, in the same transaction.
#[allow(clippy::too_many_arguments)]
async fn leave_trace(
    d: &Deps,
    tx: &mut ModuleTx,
    id: HandoffId,
    seq: i32,
    from: &str,
    to: &str,
    actor: WorkerId,
    incarnation: i64,
    reason: Option<&str>,
    action: &'static str,
    details: AuditDetails,
) -> Result<(), Problem> {
    store::log_transition(
        tx,
        &store::LogRow {
            id,
            seq,
            from: Some(from),
            to,
            actor,
            incarnation,
            reason,
        },
    )
    .await?;
    d.audit
        .record(tx, action, AuditTarget::Handoff(id), details)
        .await
        .map_err(Problem::from)
}

fn state_out(id: HandoffId, state: &'static str, round: i32) -> Reply<StateOut> {
    Reply::ok(StateOut {
        id: id.to_string(),
        state,
        round,
    })
}

/// The first phase of a move whose declared rule is `None` (release, return): the 404, then the proof.
async fn authorize_open<T>(
    ctx: &Ctx,
    tx: &mut ModuleTx,
    value: impl FnOnce(HandoffId) -> T,
) -> Result<(HandoffId, Authorized<T>), Problem> {
    let id: HandoffId = ctx.path_id("id")?;
    store::load(tx, id).await?.ok_or_else(Problem::not_found)?;
    let proof = ctx.authorize(&Target::default(), value(id))?;
    Ok((id, proof))
}

// -- return --

pub struct ReturnPlan {
    id: HandoffId,
    ok: ReturnOk,
}

pub async fn return_authorize(
    _d: &Deps,
    tx: &mut ModuleTx,
    ctx: &Ctx,
    body: &ReturnBody,
) -> Result<Authorized<ReturnPlan>, Problem> {
    let ok = validate_return(body.clone())?;
    let (_, proof) = authorize_open(ctx, tx, |id| ReturnPlan { id, ok }).await?;
    Ok(proof)
}

pub async fn return_execute(
    d: &Deps,
    tx: &mut ModuleTx,
    ctx: &Ctx,
    auth: Authorized<ReturnPlan>,
) -> Result<Reply<StateOut>, Problem> {
    let ReturnPlan { id, ok } = auth.into_value();
    let caller = ctx.principal().worker;
    let row = store::load_for_update(tx, id)
        .await?
        .ok_or_else(Problem::not_found)?;
    let f = facts(d, tx, ctx, &row).await?;
    transition(Verb::Return, &f, false).map_err(refusal)?;
    let incarnation = incarnation_of(tx, id).await?;
    let seq = store::return_write(tx, &row, caller, incarnation, &ok).await?;
    leave_trace(
        d,
        tx,
        id,
        seq,
        "claimed",
        "returned",
        caller,
        incarnation,
        None,
        "handoff.returned",
        AuditDetails::new(),
    )
    .await?;
    Ok(state_out(id, "returned", row.round))
}

async fn incarnation_of(tx: &mut ModuleTx, id: HandoffId) -> Result<i64, DbError> {
    Ok(
        sqlx::query_scalar("SELECT incarnation FROM handoffs.handoffs WHERE id = $1::uuid")
            .bind(id.to_string())
            .fetch_one(tx.executor()?)
            .await?,
    )
}

// -- accept and rework --

pub struct ReviewPlan {
    id: HandoffId,
    notes: Option<String>,
}

/// The first phase of a review: step 0 (the notes), the 404, the target rule `SelfOrProjectLead` with the
/// creator as the target worker (403), then "never the author" (403 `self_review_forbidden`).
async fn review_authorize(
    d: &Deps,
    tx: &mut ModuleTx,
    ctx: &Ctx,
    verb: Verb,
    notes: Option<String>,
) -> Result<Authorized<ReviewPlan>, Problem> {
    let id: HandoffId = ctx.path_id("id")?;
    let row = store::load(tx, id).await?.ok_or_else(Problem::not_found)?;
    let target = Target {
        worker: Some(row.created_by),
        kind: Some(WorkerKind::Worker),
        ..Target::default()
    };
    let proof = ctx.authorize(&target, ReviewPlan { id, notes })?;
    let f = facts(d, tx, ctx, &row).await?;
    authority(verb, &f, true).map_err(refusal)?;
    Ok(proof)
}

pub async fn accept_authorize(
    d: &Deps,
    tx: &mut ModuleTx,
    ctx: &Ctx,
    body: &ReviewBody,
) -> Result<Authorized<ReviewPlan>, Problem> {
    let notes = validate_accept(body.clone())?;
    review_authorize(d, tx, ctx, Verb::Accept, notes).await
}

pub async fn rework_authorize(
    d: &Deps,
    tx: &mut ModuleTx,
    ctx: &Ctx,
    body: &ReviewBody,
) -> Result<Authorized<ReviewPlan>, Problem> {
    let notes = validate_rework(body.clone())?;
    review_authorize(d, tx, ctx, Verb::Rework, Some(notes)).await
}

async fn review_execute(
    d: &Deps,
    tx: &mut ModuleTx,
    ctx: &Ctx,
    verb: Verb,
    plan: ReviewPlan,
) -> Result<Reply<StateOut>, Problem> {
    let ReviewPlan { id, notes } = plan;
    let caller = ctx.principal().worker;
    let row = store::load_for_update(tx, id)
        .await?
        .ok_or_else(Problem::not_found)?;
    let f = facts(d, tx, ctx, &row).await?;
    // The same two halves again, on the locked row: the 403 half cannot have changed for the worse, but the
    // 409 half is decided only here.
    let allowed = authority(verb, &f, true).map_err(refusal)?;
    transition(verb, &f, false).map_err(refusal)?;
    let self_review = allowed == Allowed::SelfReview;
    let incarnation = incarnation_of(tx, id).await?;
    let details = if self_review {
        AuditDetails::new()
            .flag("self_review", true)
            .map_err(Problem::from)?
    } else {
        AuditDetails::new()
    };
    match verb {
        Verb::Accept => {
            store::review_write(
                tx,
                id,
                row.round,
                caller,
                "accept",
                notes.as_deref(),
                self_review,
            )
            .await?;
            let seq = store::accept_write(tx, id).await?;
            leave_trace(
                d,
                tx,
                id,
                seq,
                "returned",
                "accepted",
                caller,
                incarnation,
                None,
                "handoff.accepted",
                details,
            )
            .await?;
            Ok(state_out(id, "accepted", row.round))
        }
        _ => {
            // The worker who returned it is the holder, and it is the only one who may claim round + 1.
            let returner = row.holder.ok_or_else(|| {
                Problem::from(DbError::Other("a returned handoff has a holder".into()))
            })?;
            store::review_write(
                tx,
                id,
                row.round,
                caller,
                "rework",
                notes.as_deref(),
                self_review,
            )
            .await?;
            let (seq, round) = store::rework_write(tx, id, returner).await?;
            leave_trace(
                d,
                tx,
                id,
                seq,
                "returned",
                "pending",
                caller,
                incarnation,
                None,
                "handoff.reworked",
                details,
            )
            .await?;
            Ok(state_out(id, "pending", round))
        }
    }
}

pub async fn accept_execute(
    d: &Deps,
    tx: &mut ModuleTx,
    ctx: &Ctx,
    auth: Authorized<ReviewPlan>,
) -> Result<Reply<StateOut>, Problem> {
    review_execute(d, tx, ctx, Verb::Accept, auth.into_value()).await
}

pub async fn rework_execute(
    d: &Deps,
    tx: &mut ModuleTx,
    ctx: &Ctx,
    auth: Authorized<ReviewPlan>,
) -> Result<Reply<StateOut>, Problem> {
    review_execute(d, tx, ctx, Verb::Rework, auth.into_value()).await
}

// -- release --

pub struct ReleasePlan {
    id: HandoffId,
    reason: Option<String>,
}

pub async fn release_authorize(
    _d: &Deps,
    tx: &mut ModuleTx,
    ctx: &Ctx,
    body: &ReleaseBody,
) -> Result<Authorized<ReleasePlan>, Problem> {
    let reason = validate_release(body.clone())?;
    let (_, proof) = authorize_open(ctx, tx, |id| ReleasePlan { id, reason }).await?;
    Ok(proof)
}

pub async fn release_execute(
    d: &Deps,
    tx: &mut ModuleTx,
    ctx: &Ctx,
    auth: Authorized<ReleasePlan>,
) -> Result<Reply<StateOut>, Problem> {
    let ReleasePlan { id, reason } = auth.into_value();
    let caller = ctx.principal().worker;
    let row = store::load_for_update(tx, id)
        .await?
        .ok_or_else(Problem::not_found)?;
    let f = facts(d, tx, ctx, &row).await?;
    transition(Verb::Release, &f, reason.is_some()).map_err(refusal)?;
    let incarnation = incarnation_of(tx, id).await?;
    let seq = store::release_write(tx, id).await?;
    leave_trace(
        d,
        tx,
        id,
        seq,
        "claimed",
        "pending",
        caller,
        incarnation,
        reason.as_deref(),
        "handoff.released",
        AuditDetails::new(),
    )
    .await?;
    Ok(state_out(id, "pending", row.round))
}

// -- cancel --

pub struct CancelPlan {
    id: HandoffId,
    reason: String,
}

/// The first phase of a cancel: step 0 (the reason, even for an id that does not exist), the 404, and the
/// target rule `Lineage` with the creator as the target worker and the WORKER addressee's team at this
/// moment (none for a job-role address): the creator, the project lead, or the team lead concerned.
pub async fn cancel_authorize(
    d: &Deps,
    tx: &mut ModuleTx,
    ctx: &Ctx,
    body: &CancelBody,
) -> Result<Authorized<CancelPlan>, Problem> {
    let reason = validate_cancel(body.clone())?;
    let id: HandoffId = ctx.path_id("id")?;
    let row = store::load(tx, id).await?.ok_or_else(Problem::not_found)?;
    let team = match &row.to {
        To::Worker(w) => d
            .identity
            .worker(tx, w)
            .await
            .map_err(Problem::from)?
            .as_ref()
            .and_then(team_of),
        To::JobRole(_) => None,
    };
    let lead = d.identity.project(tx).await.map_err(Problem::from)?.lead;
    let target = Target {
        worker: Some(row.created_by),
        kind: Some(WorkerKind::Worker),
        team,
        lead: Some(lead),
        ..Target::default()
    };
    ctx.authorize(&target, CancelPlan { id, reason })
}

pub async fn cancel_execute(
    d: &Deps,
    tx: &mut ModuleTx,
    ctx: &Ctx,
    auth: Authorized<CancelPlan>,
) -> Result<Reply<StateOut>, Problem> {
    let CancelPlan { id, reason } = auth.into_value();
    let caller = ctx.principal().worker;
    let row = store::load_for_update(tx, id)
        .await?
        .ok_or_else(Problem::not_found)?;
    let f = facts(d, tx, ctx, &row).await?;
    transition(Verb::Cancel, &f, false).map_err(refusal)?;
    let incarnation = incarnation_of(tx, id).await?;
    let from = row.state.as_str();
    let seq = store::cancel_write(tx, id, &reason).await?;
    leave_trace(
        d,
        tx,
        id,
        seq,
        from,
        "cancelled",
        caller,
        incarnation,
        Some(&reason),
        "handoff.cancelled",
        AuditDetails::new(),
    )
    .await?;
    Ok(state_out(id, "cancelled", row.round))
}

// ---- the port ----

/// The `HandoffsApi` provider: what a worker holds and may claim, for boot. It needs `IdentityApi` to know the
/// caller's job roles; the ports only exist once every piece has provided, so the registry hands it over in
/// `Piece::routes` ([`Provider::attach`]).
pub struct Provider {
    id: ModuleId,
    identity: OnceLock<Arc<dyn IdentityApi>>,
}

impl Provider {
    pub fn new() -> Provider {
        Provider {
            id: module_id(),
            identity: OnceLock::new(),
        }
    }

    pub fn attach(&self, identity: Arc<dyn IdentityApi>) {
        let _ = self.identity.set(identity);
    }
}

impl Default for Provider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl HandoffsApi for Provider {
    async fn claimed_by(
        &self,
        tx: &mut ModuleTx,
        w: &WorkerId,
        limit: u32,
    ) -> Result<Vec<HandoffView>, PortError> {
        // What a worker holds never depends on addressing or on being active (HAND-24): no identity read.
        let (w, limit) = (*w, i64::from(limit.clamp(1, HANDOFF_LIST_CAP)));
        let rows = tx
            .delegate(&self.id, async move |t: &mut ModuleTx| {
                store::claimed_by(t, w, limit).await
            })
            .await?;
        Ok(rows.into_iter().map(view).collect())
    }

    async fn pending_for(
        &self,
        tx: &mut ModuleTx,
        w: &WorkerId,
        limit: u32,
    ) -> Result<Vec<HandoffView>, PortError> {
        let identity = self.identity.get().ok_or(PortError::Unavailable)?;
        let Some(who) = identity.worker(tx, w).await? else {
            return Ok(Vec::new());
        };
        let (w, limit) = (*w, i64::from(limit.clamp(1, HANDOFF_LIST_CAP)));
        let rows = tx
            .delegate(&self.id, async move |t: &mut ModuleTx| {
                store::pending_for(t, w, who.active, &who.job_roles, limit).await
            })
            .await?;
        Ok(rows.into_iter().map(view).collect())
    }
}

/// A row as boot sees it: one briefing line, not the packet.
fn view((r, micros): (Row_, i64)) -> HandoffView {
    HandoffView {
        id: r.id,
        kind: match r.kind {
            Kind::Task => HandoffKind::Task,
            Kind::Consult => HandoffKind::Consult,
        },
        from: r.created_by,
        to: match r.to {
            To::Worker(w) => HandoffTo::Worker(w),
            To::JobRole(label) => HandoffTo::JobRole(label),
        },
        state: r.state.as_str().to_owned(),
        priority: r.priority,
        summary: r.summary,
        created_at: Timestamp::from_unix_micros(micros),
    }
}
