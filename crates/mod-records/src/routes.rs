//! The five routes of piece 6 (S3 r2.6): `records.log`, `records.show`, `records.supersede`, `records.invalidate`,
//! `records.section_current`. Each write is the chain's two phases (`Routes::write`): `authorize` reads and decides (step 0 first,
//! then 404, then the 403 target rule; it takes no lock), `execute` takes the locks and writes (the 409 and the write).

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::Method;
use cortex_module::route::AppState;
use cortex_module::{
    AuditSink, Authorized, Ctx, IdentityApi, Port, Ports, Problem, Reply, Role, Routes,
    SectionName, Target, TargetRule,
};
use cortex_types::{ModuleId, RecordId, SystemClock, WorkerId};

use crate::request::{Invalidate, RecordCreate, Supersede};
use crate::store::{self, Actor, Env, LineageTarget, RecordOut, SectionPointer};
use crate::validate::Validated;
use crate::{ID, SCOPE_LINEAGE, SCOPE_READ, SCOPE_WRITE, problems};

/// Body caps of S3 r2.5: the wire limit of `records.log` is 9 MiB (a conversation body of 4 MiB is reserved in 0.2.001, but the
/// 413 runs before parsing and cannot see the type); the lineage bodies are small.
const LOG_MAX_BODY: u64 = 9_437_184;
const LINEAGE_MAX_BODY: u64 = 16_384;

fn actor(ctx: &Ctx) -> Actor {
    let p = ctx.principal();
    Actor {
        worker: p.worker,
        kind: p.kind,
        role: p.role,
    }
}

/// The row the `Lineage` target rule is asked about (S3 R-17): the record's AUTHOR, the author's team and the project lead. A
/// non-worker author (a human or the system) has no worker: only the project lead may act on such a record.
async fn lineage_policy_target(
    tx: &mut cortex_db::ModuleTx,
    identity: &dyn IdentityApi,
    lt: &LineageTarget,
) -> Result<Target, Problem> {
    let project = identity.project(tx).await.map_err(Problem::from)?;
    let author = match lt.author_kind.as_str() {
        "worker" | "service" => WorkerId::parse(&lt.author_id).ok(),
        _ => None,
    };
    let Some(w) = author else {
        return Ok(Target {
            lead: Some(project.lead),
            ..Target::default()
        });
    };
    let view = identity.worker(tx, &w).await.map_err(Problem::from)?;
    Ok(match view {
        Some(v) => Target {
            worker: Some(w),
            kind: Some(v.kind),
            team: match v.role {
                Role::TeamLead(t) | Role::Member(Some(t)) => Some(t),
                Role::ProjectLead | Role::Member(None) => None,
            },
            lead: Some(project.lead),
            consult: false,
        },
        None => Target {
            worker: Some(w),
            lead: Some(project.lead),
            ..Target::default()
        },
    })
}

/// `GET /v2/records/{id}`: one record with its state and lineage; 404 for an id that names nothing in the project.
async fn show_route(State(st): State<AppState>, ctx: Ctx) -> Result<Json<RecordOut>, Problem> {
    let id: RecordId = ctx.path_id("id")?;
    let module = ModuleId::parse(ID).expect("a valid module id");
    let mut tx = ctx.begin(&st.pool, &module).await?;
    let rec = store::show(&mut tx, id).await?;
    tx.commit().await?;
    rec.map(Json).ok_or_else(Problem::not_found)
}

/// `GET /v2/sections/{name}`: the CURRENT revision of a reserved section; 422 `section_invalid` for any other name, 404 when none.
async fn section_route(
    State(st): State<AppState>,
    ctx: Ctx,
) -> Result<Json<SectionPointer>, Problem> {
    let raw = ctx.path_param("name")?;
    let name = SectionName::parse(raw).ok_or_else(|| {
        Problem::new(
            &problems::SECTION_INVALID,
            "The section must be rules-project, rules-worker:<id> or orientation.",
        )
    })?;
    let module = ModuleId::parse(ID).expect("a valid module id");
    let mut tx = ctx.begin(&st.pool, &module).await?;
    let ptr = store::section_current(&mut tx, &name).await?;
    tx.commit().await?;
    ptr.map(Json).ok_or_else(Problem::not_found)
}

/// All five routes. `ports` holds the ports the manifest `uses` (IdentityApi, AuditSink); both are present (the registry checked).
pub fn routes(ports: &Ports) -> Routes {
    let identity = ports.identity().expect("records uses IdentityApi");
    let audit_log: Arc<dyn AuditSink> = ports.audit().expect("records uses AuditSink");
    let (audit_sup, audit_inv) = (Arc::clone(&audit_log), Arc::clone(&audit_log));
    let (identity_sup, identity_inv) = (Arc::clone(&identity), Arc::clone(&identity));
    let _ = Port::Records;

    Routes::new()
        .write(
            Method::POST,
            "/v2/records",
            SCOPE_WRITE,
            TargetRule::None,
            |tx, ctx, b: &RecordCreate| {
                Box::pin(async move {
                    let v = store::authorize_log(tx, &actor(ctx), b).await?;
                    ctx.authorize(&Target::default(), v)
                })
            },
            move |tx, ctx, a: Authorized<Validated>, b: RecordCreate| {
                let audit = Arc::clone(&audit_log);
                Box::pin(async move {
                    let env = Env {
                        project: ctx.project(),
                        clock: &SystemClock,
                        audit: &*audit,
                    };
                    let created = store::execute_log(tx, &env, &actor(ctx), &b, a.value()).await?;
                    Ok(Reply::created(created))
                })
            },
        )
        .max_body(LOG_MAX_BODY)
        .read(
            Method::GET,
            "/v2/records/{id}",
            SCOPE_READ,
            TargetRule::None,
            show_route,
        )
        .write(
            Method::POST,
            "/v2/records/{id}/supersede",
            SCOPE_LINEAGE,
            TargetRule::Lineage,
            move |tx, ctx, b: &Supersede| {
                let identity = Arc::clone(&identity_sup);
                Box::pin(async move {
                    // Step 0 first, even when the path id is malformed (that is a 404, which comes after).
                    let target = match ctx.path_id::<RecordId>("id") {
                        Ok(t) => t,
                        Err(not_found) => {
                            store::step0_invalidate(b.reason.as_deref())?;
                            return Err(not_found);
                        }
                    };
                    let (lt, reason) =
                        store::authorize_supersede(tx, target, b.by, b.reason.as_deref()).await?;
                    let row = lineage_policy_target(tx, &*identity, &lt).await?;
                    ctx.authorize(&row, (lt, reason))
                })
            },
            move |tx, ctx, a: Authorized<(LineageTarget, String)>, b: Supersede| {
                let audit = Arc::clone(&audit_sup);
                Box::pin(async move {
                    let (lt, reason) = a.value();
                    let env = Env {
                        project: ctx.project(),
                        clock: &SystemClock,
                        audit: &*audit,
                    };
                    let out =
                        store::execute_supersede(tx, &env, &actor(ctx), lt, b.by, reason).await?;
                    Ok(Reply::ok(out))
                })
            },
        )
        .max_body(LINEAGE_MAX_BODY)
        .write(
            Method::POST,
            "/v2/records/{id}/invalidate",
            SCOPE_LINEAGE,
            TargetRule::Lineage,
            move |tx, ctx, b: &Invalidate| {
                let identity = Arc::clone(&identity_inv);
                Box::pin(async move {
                    let target = match ctx.path_id::<RecordId>("id") {
                        Ok(t) => t,
                        Err(not_found) => {
                            store::step0_invalidate(b.reason.as_deref())?;
                            return Err(not_found);
                        }
                    };
                    let (lt, reason) =
                        store::authorize_invalidate(tx, target, b.reason.as_deref()).await?;
                    let row = lineage_policy_target(tx, &*identity, &lt).await?;
                    ctx.authorize(&row, (lt, reason))
                })
            },
            move |tx, ctx, a: Authorized<(LineageTarget, String)>, b: Invalidate| {
                let audit = Arc::clone(&audit_inv);
                Box::pin(async move {
                    let (lt, reason) = a.value();
                    let env = Env {
                        project: ctx.project(),
                        clock: &SystemClock,
                        audit: &*audit,
                    };
                    let out = store::execute_invalidate(
                        tx,
                        &env,
                        &actor(ctx),
                        lt,
                        reason,
                        b.undo.unwrap_or(false),
                    )
                    .await?;
                    Ok(Reply::ok(out))
                })
            },
        )
        .max_body(LINEAGE_MAX_BODY)
        .read(
            Method::GET,
            "/v2/sections/{name}",
            SCOPE_READ,
            TargetRule::None,
            section_route,
        )
}
