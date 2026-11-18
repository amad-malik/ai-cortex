//! The three read routes (S3 r2.5): `GET /v2/projects/{id}`, `GET /v2/teams`, `GET /v2/workers`.
//! All `roster:read`, rule `None`, each in its own transaction (`Ctx::begin`) under `cortex_m_identity`
//! and the token's project (row-level security).
//!
//! - **Step 0 (stateless 422s)** runs before any read: `limit` outside 1 to 200, a `name` outside the
//!   name grammar (a NUL or a trailing newline included), a cursor that does not decode or belongs to
//!   another list. Unknown query parameters are refused by `StrictQuery` and the default-deny marker.
//! - **Paging (R-8):** keyset on `(name_key, id)` in BYTE order (`COLLATE "C"`); `next` is the last
//!   returned row's cursor when more rows exist, else `null`. A cursor is a POSITION, never authority:
//!   it cannot select a row outside the token's project, because RLS filters first.
//! - **Cursor:** lowercase hex of `<tag>:<name_key>:<id>`, tag `w` for workers and `t` for teams.

use axum::Json;
use axum::extract::State;
use axum::http::Method;
use cortex_module::route::{AppState, StrictQuery};
use cortex_module::{Ctx, Problem, Routes, TargetRule};
use cortex_types::{Id, ModuleId, ProjectId, TeamId, WorkerId};
use serde::{Deserialize, Serialize};

/// `GET /v2/projects/{id}`: `{id, name}` and nothing else.
#[derive(Debug, Serialize)]
pub struct ProjectBody {
    pub id: String,
    pub name: String,
}

/// One page (R-8): `{items, next}`.
#[derive(Debug, Serialize)]
pub struct Page<T: Serialize> {
    pub items: Vec<T>,
    pub next: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TeamBody {
    pub id: String,
    pub name: String,
    pub lead: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct WorkerBody {
    pub id: String,
    pub name: String,
    pub kind: &'static str,
    pub state: &'static str,
    pub team: Option<String>,
    pub role: &'static str,
    pub job_roles: Vec<String>,
    pub mode: &'static str,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeamsQuery {
    pub limit: Option<u32>,
    pub after: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkersQuery {
    pub name: Option<String>,
    pub limit: Option<u32>,
    pub after: Option<String>,
}

/// Adds the three read routes to `r`.
pub fn add(r: Routes) -> Routes {
    r.read(
        Method::GET,
        "/v2/projects/{id}",
        crate::SCOPE_ROSTER_READ,
        TargetRule::None,
        project_get,
    )
    .read(
        Method::GET,
        "/v2/teams",
        crate::SCOPE_ROSTER_READ,
        TargetRule::None,
        teams_list,
    )
    .query()
    .read(
        Method::GET,
        "/v2/workers",
        crate::SCOPE_ROSTER_READ,
        TargetRule::None,
        workers_list,
    )
    .query()
}

/// The page size: default 50, 1 to 200 (R-8).
fn limit_of(limit: Option<u32>) -> Result<i64, Problem> {
    match limit {
        None => Ok(50),
        Some(n) if (1..=200).contains(&n) => Ok(i64::from(n)),
        Some(_) => Err(Problem::invalid_request(
            422,
            "The limit must be from 1 to 200.",
        )),
    }
}

fn bad_cursor() -> Problem {
    Problem::invalid_request(422, "The cursor is not valid for this list.")
}

fn encode_cursor(tag: char, key: &str, id: &str) -> String {
    format!("{tag}:{key}:{id}")
        .bytes()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// A cursor of the right list, with a key of the label grammar and an id of the right kind;
/// anything else is 422. Returns the key and the id text.
fn decode_cursor<I: Id>(after: &str, tag: char) -> Result<(String, String), Problem> {
    let b = after.as_bytes();
    if b.is_empty() || !b.len().is_multiple_of(2) || !b.iter().all(u8::is_ascii_hexdigit) {
        return Err(bad_cursor());
    }
    let raw: Vec<u8> = b
        .chunks(2)
        .map(|p| {
            let hex = std::str::from_utf8(p).unwrap_or("zz");
            u8::from_str_radix(hex, 16).unwrap_or(0)
        })
        .collect();
    let text = String::from_utf8(raw).map_err(|_| bad_cursor())?;
    let mut parts = text.splitn(3, ':');
    let (t, key, id) = (parts.next(), parts.next(), parts.next());
    match (t, key, id) {
        (Some(t), Some(key), Some(id))
            if t.len() == 1
                && t.starts_with(tag)
                && crate::labels::name_valid(key)
                && key == key.to_ascii_lowercase()
                && I::parse(id).is_ok() =>
        {
            Ok((key.to_owned(), id.to_owned()))
        }
        _ => Err(bad_cursor()),
    }
}

fn id_module() -> ModuleId {
    ModuleId::parse(crate::ID).expect("a valid module id")
}

async fn project_get(State(st): State<AppState>, ctx: Ctx) -> Result<Json<ProjectBody>, Problem> {
    // The token's own project only; anything else (another project, unknown, malformed) is the same 404.
    let id: ProjectId = ctx.path_id("id")?;
    if id != ctx.project() {
        return Err(Problem::not_found());
    }
    let mut tx = ctx.begin(&st.pool, &id_module()).await?;
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT p.id::text, n.label FROM identity.projects p \
         JOIN identity.names n ON n.entity_kind = 'project' AND n.entity_id = p.id AND n.current",
    )
    .fetch_optional(tx.executor()?)
    .await
    .map_err(cortex_db::DbError::from)?;
    tx.commit().await?;
    let (id, name) = row.ok_or_else(Problem::not_found)?;
    Ok(Json(ProjectBody { id, name }))
}

async fn teams_list(
    State(st): State<AppState>,
    ctx: Ctx,
    StrictQuery(q): StrictQuery<TeamsQuery>,
) -> Result<Json<Page<TeamBody>>, Problem> {
    let limit = limit_of(q.limit)?;
    let after = q
        .after
        .as_deref()
        .map(|c| decode_cursor::<TeamId>(c, 't'))
        .transpose()?;
    let (after_key, after_id) = after.unzip();
    let mut tx = ctx.begin(&st.pool, &id_module()).await?;
    let rows: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT t.id::text, n.label, n.name_key, \
                (SELECT tm.worker_id::text FROM identity.team_members tm \
                  WHERE tm.team_id = t.id AND tm.role = 'team_lead') \
         FROM identity.teams t \
         JOIN identity.names n ON n.entity_kind = 'team' AND n.entity_id = t.id AND n.current \
         WHERE ($1::text IS NULL \
                OR n.name_key COLLATE \"C\" > $1::text COLLATE \"C\" \
                OR (n.name_key = $1::text AND t.id > $2::uuid)) \
         ORDER BY n.name_key COLLATE \"C\", t.id \
         LIMIT $3",
    )
    .bind(after_key)
    .bind(after_id)
    .bind(limit + 1)
    .fetch_all(tx.executor()?)
    .await
    .map_err(cortex_db::DbError::from)?;
    tx.commit().await?;
    let more = rows.len() as i64 > limit;
    let mut next = None;
    let items: Vec<TeamBody> = rows
        .into_iter()
        .take(limit as usize)
        .map(|(id, name, key, lead)| {
            next = more.then(|| encode_cursor('t', &key, &id));
            TeamBody { id, name, lead }
        })
        .collect();
    Ok(Json(Page { items, next }))
}

/// id, label, key, kind, state, job roles, mode, team-member role, team id, is the project lead.
type ListRow = (
    String,
    String,
    String,
    String,
    String,
    Vec<String>,
    String,
    Option<String>,
    Option<String>,
    bool,
);

fn static_of(s: &str, options: &[&'static str]) -> Result<&'static str, Problem> {
    options.iter().copied().find(|o| *o == s).ok_or_else(|| {
        cortex_db::DbError::Other("identity: a stored value does not fit its body".into()).into()
    })
}

async fn workers_list(
    State(st): State<AppState>,
    ctx: Ctx,
    StrictQuery(q): StrictQuery<WorkersQuery>,
) -> Result<Json<Page<WorkerBody>>, Problem> {
    let limit = limit_of(q.limit)?;
    let name_key = q
        .name
        .as_deref()
        .map(|n| {
            crate::labels::name_key(n)
                .ok_or_else(|| Problem::invalid_request(422, "The name is not a valid name."))
        })
        .transpose()?;
    let after = q
        .after
        .as_deref()
        .map(|c| decode_cursor::<WorkerId>(c, 'w'))
        .transpose()?;
    let (after_key, after_id) = after.unzip();
    let mut tx = ctx.begin(&st.pool, &id_module()).await?;
    let rows: Vec<ListRow> = sqlx::query_as(
        "SELECT w.id::text, n.label, n.name_key, w.kind, w.state, w.job_roles, w.mode, \
                tm.role, tm.team_id::text, (p.lead_worker_id = w.id) \
         FROM identity.workers w \
         JOIN identity.names n ON n.entity_kind = 'worker' AND n.entity_id = w.id AND n.current \
         JOIN identity.projects p ON p.id = w.project_id \
         LEFT JOIN identity.team_members tm ON tm.worker_id = w.id \
         WHERE w.state = 'active' \
           AND ($1::text IS NULL OR n.name_key = $1::text) \
           AND ($2::text IS NULL \
                OR n.name_key COLLATE \"C\" > $2::text COLLATE \"C\" \
                OR (n.name_key = $2::text AND w.id > $3::uuid)) \
         ORDER BY n.name_key COLLATE \"C\", w.id \
         LIMIT $4",
    )
    .bind(name_key)
    .bind(after_key)
    .bind(after_id)
    .bind(limit + 1)
    .fetch_all(tx.executor()?)
    .await
    .map_err(cortex_db::DbError::from)?;
    tx.commit().await?;
    let more = rows.len() as i64 > limit;
    let mut next = None;
    let mut items = Vec::new();
    for (id, name, key, kind, state, job_roles, mode, member_role, team, is_lead) in
        rows.into_iter().take(limit as usize)
    {
        next = more.then(|| encode_cursor('w', &key, &id));
        items.push(WorkerBody {
            id,
            name,
            kind: static_of(&kind, &["worker", "service"])?,
            state: static_of(&state, &["active", "retired"])?,
            role: if is_lead {
                "project_lead"
            } else if member_role.as_deref() == Some("team_lead") {
                "team_lead"
            } else {
                "member"
            },
            team,
            job_roles,
            mode: static_of(&mode, &["model", "deterministic"])?,
        });
    }
    Ok(Json(Page { items, next }))
}
