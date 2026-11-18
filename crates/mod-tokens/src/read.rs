//! `GET /v2/whoami`, `GET /v2/tokens` and `GET /v2/tokens/{id}` (cells TOK-40 to TOK-46, TOK-60 to TOK-63).
//!
//! - **whoami** (`self:read`, rule `None`): the caller's own facts: worker, name, kind, role, team, job roles from
//!   `IdentityApi`, the project's installation, and the PRESENTED token's id and expiry.
//! - **list** (`token:list`, rule `None`): a list, not one target. The project lead sees every token of the
//!   project; a team lead sees the tokens of its own team's workers, **filtered in the query** (the worker ids
//!   come from `IdentityApi::roster`), so a page never holds fewer rows than remain. Keyset on
//!   `(issued_at, id)`; the cursor is a position, never authority: row-level security filters first.
//! - **inspect** (`token:list`, rule `TokenTarget`): one token's metadata; 404 for an id outside the project
//!   (the same answer as an unknown or malformed one), 403 `target_not_allowed` for a team lead outside its team.
//! - **TokenMeta** never carries a digest, a secret, a generation, a project or `issued_by` (TOK-44).

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::http::Method;
use cortex_db::DbError;
use cortex_module::route::{AppState, StrictQuery};
use cortex_module::{Ctx, IdentityApi, Problem, Role, Routes, TargetRule, WorkerKind};
use cortex_types::{ModuleId, Timestamp, TokenId, WorkerId};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use sqlx::postgres::PgRow;

use crate::issue::{target_of, team_of};
use crate::time::{rfc3339, rfc3339_micros};
use crate::verify::state_of;

fn module() -> ModuleId {
    ModuleId::parse(crate::ID).expect("a valid module id")
}

/// `Whoami` (S3).
#[derive(Debug, Serialize)]
pub struct WhoamiBody {
    installation: String,
    project: String,
    worker: String,
    name: String,
    kind: &'static str,
    role: &'static str,
    team: Option<String>,
    job_roles: Vec<String>,
    token_id: String,
    expires_at: String,
    renew_due: bool,
}

/// `TokenMeta` (S3): exactly these keys.
#[derive(Debug, Serialize)]
pub struct TokenMeta {
    token_id: String,
    worker: String,
    issued_at: String,
    expires_at: String,
    last_used_at: Option<String>,
    state: &'static str,
    revoke_reason: Option<String>,
}

/// One page (R-8).
#[derive(Debug, Serialize)]
pub struct Page {
    items: Vec<TokenMeta>,
    next: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListQuery {
    pub limit: Option<u32>,
    pub after: Option<String>,
}

const SELECT_META: &str = "SELECT id::text AS id, worker_id::text AS worker_id, \
        (extract(epoch FROM issued_at) * 1000000)::bigint AS issued_us, \
        (extract(epoch FROM expires_at) * 1000000)::bigint AS expires_us, \
        (extract(epoch FROM last_used_at) * 1000000)::bigint AS last_used_us, \
        (extract(epoch FROM revoked_at) * 1000000)::bigint AS revoked_us, \
        revoke_reason, (extract(epoch FROM platform.now()) * 1000000)::bigint AS now_us \
        FROM tokens.tokens";

fn meta_of(r: &PgRow) -> Result<(TokenMeta, i64), Problem> {
    let e = |e: sqlx::Error| Problem::from(DbError::from(e));
    let micros = |v: i64| Timestamp::from_unix_micros(v);
    let issued: i64 = r.try_get("issued_us").map_err(e)?;
    let expires: i64 = r.try_get("expires_us").map_err(e)?;
    let last: Option<i64> = r.try_get("last_used_us").map_err(e)?;
    let revoked: Option<i64> = r.try_get("revoked_us").map_err(e)?;
    let reason: Option<String> = r.try_get("revoke_reason").map_err(e)?;
    let now: i64 = r.try_get("now_us").map_err(e)?;
    Ok((
        TokenMeta {
            token_id: r.try_get("id").map_err(e)?,
            worker: r.try_get("worker_id").map_err(e)?,
            issued_at: rfc3339_micros(issued),
            expires_at: rfc3339_micros(expires),
            last_used_at: last.map(rfc3339_micros),
            state: state_of(
                revoked.map(micros),
                reason.as_deref(),
                micros(expires),
                micros(now),
            )
            .as_str(),
            revoke_reason: reason,
        },
        issued,
    ))
}

fn role_text(r: &Role) -> &'static str {
    match r {
        Role::ProjectLead => "project_lead",
        Role::TeamLead(_) => "team_lead",
        Role::Member(_) => "member",
    }
}

async fn whoami(
    State(st): State<AppState>,
    ctx: Ctx,
    identity: Arc<dyn IdentityApi>,
) -> Result<Json<WhoamiBody>, Problem> {
    let p = ctx.principal().clone();
    let mut tx = ctx.begin(&st.pool, &module()).await?;
    let view = identity
        .worker(&mut tx, &p.worker)
        .await?
        .ok_or_else(Problem::not_found)?;
    let project = identity.project(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(WhoamiBody {
        installation: project.installation.to_string(),
        project: p.project.to_string(),
        worker: p.worker.to_string(),
        name: view.name,
        kind: if p.kind == WorkerKind::Service {
            "service"
        } else {
            "worker"
        },
        role: role_text(&view.role),
        team: team_of(&view.role).map(|t| t.to_string()),
        job_roles: view.job_roles,
        token_id: p.token.to_string(),
        expires_at: rfc3339(p.expires_at),
        renew_due: p.renew_due,
    }))
}

fn bad_cursor() -> Problem {
    Problem::invalid_request(422, "The cursor is not valid for this list.")
}

/// The first instant a cursor may name: 1970-01-01T00:00:00Z, in microseconds since the epoch (W1-39: year 1970).
pub const CURSOR_MIN_MICROS: i64 = 0;

/// The last instant a cursor may name: 9999-12-31T23:59:59.999999Z (W1-39: year 9999).
pub const CURSOR_MAX_MICROS: i64 = 253_402_300_799_999_999;

/// A list cursor: the last row's `(issued_at, id)`, hex-encoded text. Public for the cells.
pub fn encode_cursor(issued_us: i64, id: &str) -> String {
    format!("{issued_us}:{id}")
        .bytes()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Reads a cursor. The instant must lie in 1970..9999 (W1-39 covers cursors, though they carry microseconds and not
/// text): outside, it is 422 `invalid_request` here, before any SQL, and never a database error answered as 500
/// (vera's TOKENS-V4). Public for the cells.
pub fn decode_cursor(c: &str) -> Result<(i64, TokenId), Problem> {
    if c.is_empty()
        || !c.len().is_multiple_of(2)
        || !c
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(bad_cursor());
    }
    let bytes: Vec<u8> = (0..c.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&c[i..i + 2], 16).map_err(|_| bad_cursor()))
        .collect::<Result<_, _>>()?;
    let text = String::from_utf8(bytes).map_err(|_| bad_cursor())?;
    let (us, id) = text.split_once(':').ok_or_else(bad_cursor)?;
    let us = us.parse::<i64>().map_err(|_| bad_cursor())?;
    if !(CURSOR_MIN_MICROS..=CURSOR_MAX_MICROS).contains(&us) {
        return Err(bad_cursor());
    }
    Ok((us, TokenId::parse(id).map_err(|_| bad_cursor())?))
}

async fn list(
    State(st): State<AppState>,
    ctx: Ctx,
    StrictQuery(q): StrictQuery<ListQuery>,
    identity: Arc<dyn IdentityApi>,
) -> Result<Json<Page>, Problem> {
    let limit = match q.limit {
        None => 50_i64,
        Some(n) if (1..=200).contains(&n) => i64::from(n),
        Some(_) => {
            return Err(Problem::invalid_request(
                422,
                "The limit must be from 1 to 200.",
            ));
        }
    };
    let after = q.after.as_deref().map(decode_cursor).transpose()?;
    let (after_us, after_id) = after.unzip();
    let mut tx = ctx.begin(&st.pool, &module()).await?;
    // A team lead sees its own team's workers' tokens only: the filter is in the query.
    let workers: Option<Vec<String>> = match ctx.principal().role {
        Role::TeamLead(team) => Some(
            identity
                .roster(&mut tx)
                .await?
                .into_iter()
                .filter(|w| team_of(&w.role) == Some(team))
                .map(|w| w.id.to_string())
                .collect(),
        ),
        _ => None,
    };
    let sql = format!(
        "{SELECT_META} \
         WHERE ($1::text[] IS NULL OR worker_id = ANY($1::text[]::uuid[])) \
           AND ($2::bigint IS NULL OR (issued_at, id) > (timestamptz 'epoch' + $2::bigint * interval '1 microsecond', $3::uuid)) \
         ORDER BY issued_at, id LIMIT $4"
    );
    let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(workers)
        .bind(after_us)
        .bind(after_id.map(|i| i.to_string()))
        .bind(limit + 1)
        .fetch_all(tx.executor()?)
        .await
        .map_err(DbError::from)?;
    tx.commit().await?;
    let more = rows.len() as i64 > limit;
    let mut items = Vec::new();
    let mut next = None;
    for r in rows.iter().take(usize::try_from(limit).unwrap_or(50)) {
        let (meta, issued) = meta_of(r)?;
        next = more.then(|| encode_cursor(issued, &meta.token_id));
        items.push(meta);
    }
    Ok(Json(Page { items, next }))
}

async fn inspect(
    State(st): State<AppState>,
    ctx: Ctx,
    identity: Arc<dyn IdentityApi>,
) -> Result<Json<TokenMeta>, Problem> {
    let id: TokenId = ctx.path_id("id")?;
    let mut tx = ctx.begin(&st.pool, &module()).await?;
    let sql = format!("{SELECT_META} WHERE id = $1::uuid");
    let row = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(id.to_string())
        .fetch_optional(tx.executor()?)
        .await
        .map_err(DbError::from)?
        .ok_or_else(Problem::not_found)?;
    let (meta, _) = meta_of(&row)?;
    let worker = WorkerId::parse(&meta.worker).map_err(|_| Problem::not_found())?;
    let view = identity
        .worker(&mut tx, &worker)
        .await?
        .ok_or_else(Problem::not_found)?;
    let project = identity.project(&mut tx).await?;
    tx.commit().await?;
    ctx.authorize(&target_of(&view, project.lead), ())?;
    Ok(Json(meta))
}

/// Adds `tokens.whoami`, `tokens.list` and `tokens.inspect` to `r`.
pub fn add(r: Routes, identity: Arc<dyn IdentityApi>) -> Routes {
    let (i1, i2, i3) = (Arc::clone(&identity), Arc::clone(&identity), identity);
    r.read(
        Method::GET,
        "/v2/whoami",
        crate::SCOPE_SELF_READ,
        TargetRule::None,
        move |st: State<AppState>, ctx: Ctx| whoami(st, ctx, Arc::clone(&i1)),
    )
    .read(
        Method::GET,
        "/v2/tokens",
        crate::SCOPE_TOKEN_LIST,
        TargetRule::None,
        move |st: State<AppState>, ctx: Ctx, q: StrictQuery<ListQuery>| {
            list(st, ctx, q, Arc::clone(&i2))
        },
    )
    .query()
    .read(
        Method::GET,
        "/v2/tokens/{id}",
        crate::SCOPE_TOKEN_LIST,
        TargetRule::TokenTarget,
        move |st: State<AppState>, ctx: Ctx| inspect(st, ctx, Arc::clone(&i3)),
    )
}
