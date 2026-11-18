//! The four write routes (S3 r2.5): `POST /v2/teams`, `POST /v2/workers`,
//! `POST /v2/workers/{id}/retire`, `PUT /v2/workers/{id}/team`.
//!
//! Each is two-phase (delta 4): `authorize` does step 0 (the stateless 422s), reads the target (404)
//! and asks `Ctx::authorize` (403); `execute` does the 409 checks, the write and the audit record,
//! in the chain's transaction under `cortex_m_identity`.
//!
//! - **Order (R-16):** step 0 (bounds, grammar, uniqueness in the body, `reason_required`), then 404
//!   (the path id, the body team: another project's looks exactly like an unknown one), then 409
//!   `lead_permanent` for a retire of the lead (exception (a)), then 403 (the declared target rule),
//!   then, in `execute` after the chain's replay lookup (R-19), 409 and the write: `target_retired`,
//!   then `team_lead_in_post` (W1-51: a worker who leads a team is not retired; it comes after the
//!   403 so a caller without authority over the target cannot learn who leads a team).
//! - **Audit:** one record per COMMITTED change, in the same transaction, from marv's registry
//!   (`team.created`, `worker.created`, `worker.retired`, `worker.team_assigned`); details are ids,
//!   codes and flags only (delta 5). A refusal, a replay and a no-op write none.
//! - **Races:** the database is the guard. A taken name is `names_unique` (23505) and a second team
//!   lead is `one_team_lead` (23505); both are mapped to their 409, never a 500. `execute` locks the
//!   worker row (`FOR UPDATE`) before it decides, so two identical changes give one record.

use std::sync::Arc;

use axum::http::Method;
use cortex_db::{DbError, ModuleTx};
use cortex_module::{
    AuditDetails, AuditSink, AuditTarget, Authorized, Ctx, DetailRef, Problem, Reply, Routes,
    Target, TargetRule, WorkerKind,
};
use cortex_types::{SystemClock, TeamId, WorkerId};
use serde::{Deserialize, Serialize};

use crate::labels;
use crate::problems::{
    JOB_ROLES_INVALID, LEAD_PERMANENT, NAME_INVALID, NAME_TAKEN, REASON_REQUIRED, TARGET_RETIRED,
    TEAM_LEAD_IN_POST, TEAM_LEAD_TAKEN,
};

/// An optional property may be omitted but not null (S3 R-3): used with `#[serde(default)]`, a missing
/// key is `None`, and a present key must be a real value, so an explicit `null` is a decoding error
/// (422 `invalid_request`) and not a quiet default. It does not buffer the input (no `flatten`, no
/// untagged enum: W1-16, W1-21).
fn present<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(d).map(Some)
}

/// `POST /v2/teams`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeamCreate {
    pub name: String,
}

/// A body must be a JSON OBJECT. serde would also build a struct from a JSON array (its fields in
/// order), so `["Ops"]` would create the team `Ops`; `deserialize_map` refuses every other shape
/// as a type error (422 `invalid_request`), and hands the inner struct a map-only deserializer, so
/// its `deny_unknown_fields` still applies. It buffers nothing (W1-16, W1-21).
#[derive(Debug)]
pub struct ObjectOnly<T>(pub T);

impl<'de, T: Deserialize<'de>> Deserialize<'de> for ObjectOnly<T> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<T>(std::marker::PhantomData<T>);
        impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for V<T> {
            type Value = ObjectOnly<T>;
            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a JSON object")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                map: A,
            ) -> Result<Self::Value, A::Error> {
                T::deserialize(serde::de::value::MapAccessDeserializer::new(map)).map(ObjectOnly)
            }
        }
        d.deserialize_map(V(std::marker::PhantomData))
    }
}

/// An enum read as a STRING: serde_json reports a non-string value for a derived enum (null, a
/// number, `true`) as a syntax error, which the chain answers 400; read as a string it is a type
/// error, 422, like every other wrong type (S3 R-3). Unknown names are 422 too.
macro_rules! string_enum {
    ($(#[$m:meta])* $name:ident { $($text:literal => $variant:ident),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy)]
        pub enum $name { $($variant),+ }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let s = <std::borrow::Cow<'de, str> as Deserialize>::deserialize(d)?;
                match s.as_ref() {
                    $($text => Ok($name::$variant),)+
                    _ => Err(serde::de::Error::unknown_variant(&s, &[$($text),+])),
                }
            }
        }
    };
}

string_enum!(
    /// `kind` of a created worker: `worker` only in 0.2.001 (R-12).
    NewKind { "worker" => Worker }
);
string_enum!(Mode { "model" => Model, "deterministic" => Deterministic });
string_enum!(TeamRole { "member" => Member, "team_lead" => TeamLead });

/// `POST /v2/workers`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerCreate {
    pub name: String,
    pub kind: NewKind,
    #[serde(default, deserialize_with = "present")]
    pub job_roles: Option<Vec<String>>,
    #[serde(default, deserialize_with = "present")]
    pub mode: Option<Mode>,
    #[serde(default, deserialize_with = "present")]
    pub team: Option<TeamId>,
}

/// The retire body: `reason` is required by the handler, not by decoding (R-6, `reason_required`).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Retire {
    #[serde(default, deserialize_with = "present")]
    pub reason: Option<String>,
}

/// `PUT /v2/workers/{id}/team`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeamAssign {
    pub team: TeamId,
    pub role: TeamRole,
    #[serde(default, deserialize_with = "present")]
    pub job_roles: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct Created {
    pub id: String,
}

#[derive(Debug, Serialize)]
pub struct OkBody {
    pub ok: bool,
}

/// The longest retire reason, in bytes (S3 `Reason`).
const REASON_MAX: usize = 512;

fn invalid(detail: &'static str) -> Problem {
    Problem::invalid_request(422, detail)
}

/// A statement's failure as the chain would render it, except that the two constraints this piece
/// owns become their 409: `names_unique` is a taken name, `one_team_lead` a second team lead.
fn db_problem(e: sqlx::Error) -> Problem {
    match DbError::from(e) {
        DbError::Constraint { name } if name == "names_unique" => {
            Problem::new(&NAME_TAKEN, "That name is already taken in this project.")
        }
        DbError::Constraint { name } if name == "one_team_lead" => {
            Problem::new(&TEAM_LEAD_TAKEN, "That team already has a team lead.")
        }
        other => other.into(),
    }
}

/// R-2: a NUL character in any string is 422 `invalid_request`. It is checked BEFORE the label
/// grammar, so a name or a role with a NUL answers `invalid_request`, not the field's own code (a
/// reading of R-2 against CHANGES section 22's in-field order; put to cox).
fn no_nul(s: &str) -> Result<(), Problem> {
    if s.contains('\0') {
        Err(invalid("A string must not contain a NUL character."))
    } else {
        Ok(())
    }
}

fn check_name(name: &str) -> Result<(), Problem> {
    no_nul(name)?;
    if labels::name_valid(name) {
        Ok(())
    } else {
        Err(Problem::new(
            &NAME_INVALID,
            "The name must be a letter, then up to 31 letters, digits or hyphens.",
        ))
    }
}

fn check_job_roles(roles: &[String]) -> Result<(), Problem> {
    for r in roles {
        no_nul(r)?;
    }
    labels::job_roles_check(roles).map_err(|_| {
        Problem::new(
            &JOB_ROLES_INVALID,
            "Job roles must be at most 8 different lowercase labels.",
        )
    })
}

/// A team of the caller's project (row-level security hides every other project's): 404 if absent.
async fn require_team(tx: &mut ModuleTx, team: TeamId) -> Result<(), Problem> {
    let seen: Option<i32> = sqlx::query_scalar("SELECT 1 FROM identity.teams WHERE id = $1::uuid")
        .bind(team.to_string())
        .fetch_optional(tx.executor()?)
        .await
        .map_err(db_problem)?;
    seen.map(|_| ()).ok_or_else(Problem::not_found)
}

/// What the handlers need to know about a target worker.
struct TargetRow {
    kind: WorkerKind,
    team: Option<TeamId>,
    lead: WorkerId,
}

async fn read_target(tx: &mut ModuleTx, id: WorkerId) -> Result<TargetRow, Problem> {
    let row: Option<(String, String, Option<String>, String)> = sqlx::query_as(
        "SELECT w.kind, w.state, tm.team_id::text, p.lead_worker_id::text \
         FROM identity.workers w \
         JOIN identity.projects p ON p.id = w.project_id \
         LEFT JOIN identity.team_members tm ON tm.worker_id = w.id \
         WHERE w.id = $1::uuid",
    )
    .bind(id.to_string())
    .fetch_optional(tx.executor()?)
    .await
    .map_err(db_problem)?;
    let (kind, _state, team, lead) = row.ok_or_else(Problem::not_found)?;
    let bad =
        || -> Problem { DbError::Other("identity: a stored value does not fit".into()).into() };
    Ok(TargetRow {
        kind: match kind.as_str() {
            "worker" => WorkerKind::Worker,
            "service" => WorkerKind::Service,
            _ => return Err(bad()),
        },
        team: team
            .map(|t| TeamId::parse(&t))
            .transpose()
            .map_err(|_| bad())?,
        lead: WorkerId::parse(&lead).map_err(|_| bad())?,
    })
}

async fn insert_name(
    tx: &mut ModuleTx,
    kind: &'static str,
    entity: &str,
    project: &str,
    label: &str,
) -> Result<(), Problem> {
    let key = labels::name_key(label).ok_or_else(|| {
        Problem::new(
            &NAME_INVALID,
            "The name must be a letter, then up to 31 letters, digits or hyphens.",
        )
    })?;
    sqlx::query(
        "INSERT INTO identity.names (entity_kind, entity_id, scope_id, project_id, name_key, label, current) \
         VALUES ($1, $2::uuid, $3::uuid, $3::uuid, $4, $5, true)",
    )
    .bind(kind)
    .bind(entity)
    .bind(project)
    .bind(key)
    .bind(label)
    .execute(tx.executor()?)
    .await
    .map_err(db_problem)?;
    Ok(())
}

// ---- POST /v2/teams ----------------------------------------------------------------------------

async fn team_create_authorize(
    _tx: &mut ModuleTx,
    ctx: &Ctx,
    b: &TeamCreate,
) -> Result<Authorized<()>, Problem> {
    check_name(&b.name)?;
    ctx.authorize(&Target::default(), ())
}

async fn team_create_execute(
    tx: &mut ModuleTx,
    ctx: &Ctx,
    _proof: Authorized<()>,
    b: TeamCreate,
    audit: Arc<dyn AuditSink>,
) -> Result<Reply<Created>, Problem> {
    let id = TeamId::new(&SystemClock);
    let project = ctx.project().to_string();
    sqlx::query("INSERT INTO identity.teams (id, project_id) VALUES ($1::uuid, $2::uuid)")
        .bind(id.to_string())
        .bind(&project)
        .execute(tx.executor()?)
        .await
        .map_err(db_problem)?;
    insert_name(tx, "team", &id.to_string(), &project, &b.name).await?;
    audit
        .record(
            tx,
            "team.created",
            AuditTarget::Team(id),
            AuditDetails::new(),
        )
        .await?;
    Ok(Reply::created(Created { id: id.to_string() }))
}

// ---- POST /v2/workers --------------------------------------------------------------------------

async fn worker_create_authorize(
    tx: &mut ModuleTx,
    ctx: &Ctx,
    b: &WorkerCreate,
) -> Result<Authorized<()>, Problem> {
    check_name(&b.name)?;
    if let Some(roles) = &b.job_roles {
        check_job_roles(roles)?;
    }
    if let Some(team) = b.team {
        require_team(tx, team).await?;
    }
    ctx.authorize(&Target::default(), ())
}

async fn worker_create_execute(
    tx: &mut ModuleTx,
    ctx: &Ctx,
    _proof: Authorized<()>,
    b: WorkerCreate,
    audit: Arc<dyn AuditSink>,
) -> Result<Reply<Created>, Problem> {
    let id = WorkerId::new(&SystemClock);
    let project = ctx.project().to_string();
    let mode = match b.mode.unwrap_or(Mode::Model) {
        Mode::Model => "model",
        Mode::Deterministic => "deterministic",
    };
    let roles = b.job_roles.unwrap_or_default();
    // Kind is `worker`: `NewKind` has no other value in 0.2.001 (R-12).
    let NewKind::Worker = b.kind;
    sqlx::query(
        "INSERT INTO identity.workers (id, project_id, kind, job_roles, mode) \
         VALUES ($1::uuid, $2::uuid, 'worker', $3::text[], $4)",
    )
    .bind(id.to_string())
    .bind(&project)
    .bind(&roles)
    .bind(mode)
    .execute(tx.executor()?)
    .await
    .map_err(db_problem)?;
    insert_name(tx, "worker", &id.to_string(), &project, &b.name).await?;
    if let Some(team) = b.team {
        sqlx::query(
            "INSERT INTO identity.team_members (worker_id, team_id, project_id, role) \
             VALUES ($1::uuid, $2::uuid, $3::uuid, 'member')",
        )
        .bind(id.to_string())
        .bind(team.to_string())
        .bind(&project)
        .execute(tx.executor()?)
        .await
        .map_err(db_problem)?;
    }
    audit
        .record(
            tx,
            "worker.created",
            AuditTarget::Worker(id),
            AuditDetails::new(),
        )
        .await?;
    Ok(Reply::created(Created { id: id.to_string() }))
}

// ---- POST /v2/workers/{id}/retire --------------------------------------------------------------

async fn retire_authorize(
    tx: &mut ModuleTx,
    ctx: &Ctx,
    b: &Retire,
) -> Result<Authorized<WorkerId>, Problem> {
    // Step 0, before any read or any 404 (R-16, Q-10): the reason. In-field order (CHANGES s22, cox's
    // NUL ruling): the byte limit, then NUL (both `invalid_request`), then the field's own code.
    let reason = b.reason.as_deref().unwrap_or("");
    if reason.len() > REASON_MAX {
        return Err(invalid("The reason must be at most 512 bytes of text."));
    }
    no_nul(reason)?;
    if reason.trim().is_empty() {
        return Err(Problem::new(
            &REASON_REQUIRED,
            "A reason of 1 to 512 bytes is required.",
        ));
    }
    let target: WorkerId = ctx.path_id("id")?;
    let row = read_target(tx, target).await?;
    // Exception (a): the lead is permanent, for any caller who holds the scope, before the 403.
    if target == row.lead {
        return Err(Problem::new(
            &LEAD_PERMANENT,
            "The project lead cannot be retired.",
        ));
    }
    ctx.authorize(
        &Target {
            worker: Some(target),
            kind: Some(row.kind),
            team: row.team,
            lead: Some(row.lead),
            ..Target::default()
        },
        target,
    )
}

/// Whether the worker holds the `team_lead` seat of a team (W1-51). Read under the worker's row lock.
async fn leads_a_team(tx: &mut ModuleTx, worker: WorkerId) -> Result<bool, Problem> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM identity.team_members \
         WHERE worker_id = $1::uuid AND role = 'team_lead')",
    )
    .bind(worker.to_string())
    .fetch_one(tx.executor()?)
    .await
    .map_err(db_problem)
}

async fn retire_execute(
    tx: &mut ModuleTx,
    ctx: &Ctx,
    proof: Authorized<WorkerId>,
    b: Retire,
    audit: Arc<dyn AuditSink>,
) -> Result<Reply<OkBody>, Problem> {
    let target = *proof.value();
    // The row lock first: a concurrent assignment holds it while it moves the worker, and two retires
    // of one worker cannot both pass the checks below.
    let state: Option<String> =
        sqlx::query_scalar("SELECT state FROM identity.workers WHERE id = $1::uuid FOR UPDATE")
            .bind(target.to_string())
            .fetch_optional(tx.executor()?)
            .await
            .map_err(db_problem)?;
    let state = state.ok_or_else(Problem::not_found)?;
    // The proof carries an id, not trusted state: what `authorize` read had no lock, so the worker may
    // have been moved to another team since. Re-read the facts the rule uses, now that the worker is
    // locked and its membership cannot change, and apply the declared rule again (403 before 409).
    let row = read_target(tx, target).await?;
    ctx.authorize(
        &Target {
            worker: Some(target),
            kind: Some(row.kind),
            team: row.team,
            lead: Some(row.lead),
            ..Target::default()
        },
        (),
    )?;
    if state == "retired" {
        return Err(Problem::new(
            &TARGET_RETIRED,
            "That worker is already retired.",
        ));
    }
    // W1-51: the last refusal, after the target authority and `target_retired`. The worker's row is
    // locked (a promotion holds the same lock), so a promotion committed since `authorize` is seen
    // here. Nothing has been written yet; the refusal ends the transaction with no audit record.
    if leads_a_team(tx, target).await? {
        return Err(Problem::new(
            &TEAM_LEAD_IN_POST,
            "That worker leads a team.",
        ));
    }
    sqlx::query(
        "UPDATE identity.workers \
         SET state = 'retired', retired_at = platform.now(), retired_reason = $2 \
         WHERE id = $1::uuid",
    )
    .bind(target.to_string())
    .bind(b.reason.unwrap_or_default())
    .execute(tx.executor()?)
    .await
    .map_err(db_problem)?;
    audit
        .record(
            tx,
            "worker.retired",
            AuditTarget::Worker(target),
            AuditDetails::new(),
        )
        .await?;
    Ok(Reply::ok(OkBody { ok: true }))
}

// ---- PUT /v2/workers/{id}/team -----------------------------------------------------------------

async fn assign_authorize(
    tx: &mut ModuleTx,
    ctx: &Ctx,
    b: &TeamAssign,
) -> Result<Authorized<WorkerId>, Problem> {
    if let Some(roles) = &b.job_roles {
        check_job_roles(roles)?;
    }
    let target: WorkerId = ctx.path_id("id")?;
    let row = read_target(tx, target).await?;
    require_team(tx, b.team).await?;
    ctx.authorize(
        &Target {
            worker: Some(target),
            kind: Some(row.kind),
            team: None,
            lead: Some(row.lead),
            ..Target::default()
        },
        target,
    )
}

fn role_code(r: TeamRole) -> &'static str {
    match r {
        TeamRole::Member => "member",
        TeamRole::TeamLead => "team_lead",
    }
}

async fn assign_execute(
    tx: &mut ModuleTx,
    ctx: &Ctx,
    proof: Authorized<WorkerId>,
    b: TeamAssign,
    audit: Arc<dyn AuditSink>,
) -> Result<Reply<OkBody>, Problem> {
    let target = *proof.value();
    let project = ctx.project().to_string();
    // Lock the worker's row first: the decision below and the write cannot interleave with another
    // identical change (one record, not two), and a retire cannot slip in between.
    let row: Option<(String, Vec<String>)> = sqlx::query_as(
        "SELECT state, job_roles FROM identity.workers WHERE id = $1::uuid FOR UPDATE",
    )
    .bind(target.to_string())
    .fetch_optional(tx.executor()?)
    .await
    .map_err(db_problem)?;
    let (state, stored_roles) = row.ok_or_else(Problem::not_found)?;
    if state == "retired" {
        return Err(Problem::new(&TARGET_RETIRED, "That worker is retired."));
    }
    let current: Option<(String, String)> = sqlx::query_as(
        "SELECT team_id::text, role FROM identity.team_members WHERE worker_id = $1::uuid FOR UPDATE",
    )
    .bind(target.to_string())
    .fetch_optional(tx.executor()?)
    .await
    .map_err(db_problem)?;
    let want_role = role_code(b.role);
    let membership_same = current
        .as_ref()
        .is_some_and(|(t, r)| *t == b.team.to_string() && r == want_role);
    let roles_changed = b.job_roles.as_ref().is_some_and(|r| *r != stored_roles);
    if membership_same && !roles_changed {
        // The no-op (ID-31b): 200, no write, no audit record.
        return Ok(Reply::ok(OkBody { ok: true }));
    }
    if !membership_same {
        if current.is_some() {
            sqlx::query(
                "UPDATE identity.team_members SET team_id = $2::uuid, role = $3 WHERE worker_id = $1::uuid",
            )
            .bind(target.to_string())
            .bind(b.team.to_string())
            .bind(want_role)
            .execute(tx.executor()?)
            .await
            .map_err(db_problem)?;
        } else {
            sqlx::query(
                "INSERT INTO identity.team_members (worker_id, team_id, project_id, role) \
                 VALUES ($1::uuid, $2::uuid, $3::uuid, $4)",
            )
            .bind(target.to_string())
            .bind(b.team.to_string())
            .bind(&project)
            .bind(want_role)
            .execute(tx.executor()?)
            .await
            .map_err(db_problem)?;
        }
    }
    // perm_gen rises once per membership change and never for a job-role-only change.
    sqlx::query(
        "UPDATE identity.workers \
         SET perm_gen = perm_gen + CASE WHEN $2 THEN 1 ELSE 0 END, \
             job_roles = CASE WHEN $3 THEN $4::text[] ELSE job_roles END \
         WHERE id = $1::uuid",
    )
    .bind(target.to_string())
    .bind(!membership_same)
    .bind(roles_changed)
    .bind(b.job_roles.clone().unwrap_or_default())
    .execute(tx.executor()?)
    .await
    .map_err(db_problem)?;
    let mut details = AuditDetails::new()
        .id("team", DetailRef::Team(b.team))?
        .code("role", want_role)?;
    if let Some((from_team, from_role)) = &current {
        let from_team = TeamId::parse(from_team)
            .map_err(|_| Problem::from(DbError::Other("identity: a stored team id".into())))?;
        details = details
            .id("from_team", DetailRef::Team(from_team))?
            .code("from_role", from_role)?;
    }
    details = details.flag("job_roles_changed", roles_changed)?;
    audit
        .record(
            tx,
            "worker.team_assigned",
            AuditTarget::Worker(target),
            details,
        )
        .await?;
    Ok(Reply::ok(OkBody { ok: true }))
}

// ---- the route table ---------------------------------------------------------------------------

/// Adds the four write routes to `r`, with the audit sink they record to.
pub fn add(r: Routes, audit: Arc<dyn AuditSink>) -> Routes {
    let (a1, a2, a3, a4) = (
        Arc::clone(&audit),
        Arc::clone(&audit),
        Arc::clone(&audit),
        audit,
    );
    r.write::<_, _, ObjectOnly<TeamCreate>, (), Created>(
        Method::POST,
        "/v2/teams",
        crate::SCOPE_TEAM_CREATE,
        TargetRule::None,
        |tx, ctx, b: &ObjectOnly<TeamCreate>| Box::pin(team_create_authorize(tx, ctx, &b.0)),
        move |tx, ctx, p: Authorized<()>, b: ObjectOnly<TeamCreate>| {
            Box::pin(team_create_execute(tx, ctx, p, b.0, Arc::clone(&a1)))
        },
    )
    .max_body(WRITE_BODY_MAX)
    .write::<_, _, ObjectOnly<WorkerCreate>, (), Created>(
        Method::POST,
        "/v2/workers",
        crate::SCOPE_WORKER_CREATE,
        TargetRule::None,
        |tx, ctx, b: &ObjectOnly<WorkerCreate>| Box::pin(worker_create_authorize(tx, ctx, &b.0)),
        move |tx, ctx, p: Authorized<()>, b: ObjectOnly<WorkerCreate>| {
            Box::pin(worker_create_execute(tx, ctx, p, b.0, Arc::clone(&a2)))
        },
    )
    .max_body(WRITE_BODY_MAX)
    .write::<_, _, ObjectOnly<Retire>, WorkerId, OkBody>(
        Method::POST,
        "/v2/workers/{id}/retire",
        crate::SCOPE_WORKER_RETIRE,
        TargetRule::RetireTarget,
        |tx, ctx, b: &ObjectOnly<Retire>| Box::pin(retire_authorize(tx, ctx, &b.0)),
        move |tx, ctx, p: Authorized<WorkerId>, b: ObjectOnly<Retire>| {
            Box::pin(retire_execute(tx, ctx, p, b.0, Arc::clone(&a3)))
        },
    )
    .max_body(WRITE_BODY_MAX)
    .write::<_, _, ObjectOnly<TeamAssign>, WorkerId, OkBody>(
        Method::PUT,
        "/v2/workers/{id}/team",
        crate::SCOPE_TEAM_ASSIGN,
        TargetRule::NotProjectLead,
        |tx, ctx, b: &ObjectOnly<TeamAssign>| Box::pin(assign_authorize(tx, ctx, &b.0)),
        move |tx, ctx, p: Authorized<WorkerId>, b: ObjectOnly<TeamAssign>| {
            Box::pin(assign_execute(tx, ctx, p, b.0, Arc::clone(&a4)))
        },
    )
    .max_body(WRITE_BODY_MAX)
}

/// Every write of this piece declares the S3 limit: 16384 bytes (`x-max-body`, ID-53).
const WRITE_BODY_MAX: u64 = 16384;
