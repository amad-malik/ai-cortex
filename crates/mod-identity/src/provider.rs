//! The `IdentityApi` provider over the real `identity` tables (S2 revision 6).
//!
//! Every call runs in the CALLER's `ModuleTx`, delegated to `cortex_m_identity`: it sees the
//! caller's own earlier writes, rolls back with them, and opens no second transaction. The project
//! is the transaction's (row-level security); a worker of another project is `None`, the same
//! answer as a random id. Errors are codes only (`PortError`), never row text.
//!
//! - `project`: the current name as typed, the state, the lead and `projects.installation_id`
//!   (never `identity.installations`, which only host paths may read; delta 5, ID-59).
//! - `worker` / `roster`: the role comes from `projects.lead_worker_id` and `team_members`; a worker
//!   in no team is `Member(None)`. `roster` lists ACTIVE workers only, ordered by the BYTE order of `name_key` (`COLLATE "C"`, so the database locale cannot
//!   reorder punctuation) then id (ID-46); `worker` also shows a retired one, with `active = false` (ID-45).

use async_trait::async_trait;
use cortex_db::{DbError, ModuleTx};
use cortex_module::{IdentityApi, PortError, ProjectView, Role, WorkerKind, WorkerView};
use cortex_types::{InstallationId, ModuleId, ProjectId, TeamId, WorkerId};

/// The `IdentityApi` provider: stateless apart from its module id.
pub struct Provider {
    id: ModuleId,
}

impl Provider {
    pub fn new() -> Provider {
        Provider {
            id: ModuleId::parse(crate::ID).expect("a valid module id"),
        }
    }
}

impl Default for Provider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl IdentityApi for Provider {
    async fn project(&self, tx: &mut ModuleTx) -> Result<ProjectView, PortError> {
        tx.delegate(&self.id, async |t: &mut ModuleTx| project_row(t).await)
            .await?
            .ok_or(PortError::NotFound)
    }

    async fn worker(
        &self,
        tx: &mut ModuleTx,
        w: &WorkerId,
    ) -> Result<Option<WorkerView>, PortError> {
        let w = *w;
        Ok(tx
            .delegate(&self.id, async move |t: &mut ModuleTx| {
                worker_row(t, w).await
            })
            .await?)
    }

    async fn roster(&self, tx: &mut ModuleTx) -> Result<Vec<WorkerView>, PortError> {
        Ok(tx
            .delegate(&self.id, async |t: &mut ModuleTx| roster_rows(t).await)
            .await?)
    }
}

/// id, label, kind, state, job roles, team-member role, team id, is the project lead.
pub(crate) type WorkerRow = (
    String,
    String,
    String,
    String,
    Vec<String>,
    Option<String>,
    Option<String>,
    bool,
);

/// A worker query: literal SQL only (sqlx 0.9's `SqlSafeStr`), with bind parameters.
macro_rules! worker_sql {
    ($tail:literal) => {
        concat!(
            "SELECT w.id::text, n.label, w.kind, w.state, w.job_roles, \
                    tm.role, tm.team_id::text, (p.lead_worker_id = w.id) \
             FROM identity.workers w \
             JOIN identity.names n \
               ON n.entity_kind = 'worker' AND n.entity_id = w.id AND n.current \
             JOIN identity.projects p ON p.id = w.project_id \
             LEFT JOIN identity.team_members tm ON tm.worker_id = w.id",
            $tail
        )
    };
}

async fn project_row(tx: &mut ModuleTx) -> Result<Option<ProjectView>, DbError> {
    let row: Option<(String, String, String, String, String, i32)> = sqlx::query_as(
        "SELECT p.id::text, n.label, p.state, p.lead_worker_id::text, p.installation_id::text, \
                p.token_lifetime_days \
         FROM identity.projects p \
         JOIN identity.names n \
           ON n.entity_kind = 'project' AND n.entity_id = p.id AND n.current",
    )
    .fetch_optional(tx.executor()?)
    .await?;
    row.map(|(id, name, state, lead, installation, days)| {
        Ok(ProjectView {
            id: ProjectId::parse(&id).map_err(bad_row)?,
            name,
            state,
            lead: WorkerId::parse(&lead).map_err(bad_row)?,
            installation: InstallationId::parse(&installation).map_err(bad_row)?,
            token_lifetime_days: days,
        })
    })
    .transpose()
}

async fn worker_row(tx: &mut ModuleTx, w: WorkerId) -> Result<Option<WorkerView>, DbError> {
    let row: Option<WorkerRow> = sqlx::query_as(worker_sql!(" WHERE w.id = $1::uuid"))
        .bind(w.to_string())
        .fetch_optional(tx.executor()?)
        .await?;
    row.map(worker_view).transpose()
}

async fn roster_rows(tx: &mut ModuleTx) -> Result<Vec<WorkerView>, DbError> {
    let rows: Vec<WorkerRow> = sqlx::query_as(worker_sql!(
        " WHERE w.state = 'active' ORDER BY n.name_key COLLATE \"C\", w.id"
    ))
    .fetch_all(tx.executor()?)
    .await?;
    rows.into_iter().map(worker_view).collect()
}

pub(crate) fn worker_view(
    (id, name, kind, state, job_roles, member_role, team, is_lead): WorkerRow,
) -> Result<WorkerView, DbError> {
    let team = team
        .map(|t| TeamId::parse(&t).map_err(bad_row))
        .transpose()?;
    let role = match (is_lead, member_role.as_deref(), team) {
        (true, _, _) => Role::ProjectLead,
        (false, Some("team_lead"), Some(t)) => Role::TeamLead(t),
        (false, Some("member"), t) => Role::Member(t),
        (false, None, _) => Role::Member(None),
        _ => return Err(bad_row(())),
    };
    Ok(WorkerView {
        id: WorkerId::parse(&id).map_err(bad_row)?,
        name,
        kind: match kind.as_str() {
            "worker" => WorkerKind::Worker,
            "service" => WorkerKind::Service,
            _ => return Err(bad_row(())),
        },
        role,
        job_roles,
        active: state == "active",
    })
}

/// A stored value the views cannot carry (an id that is not a UUIDv7, a kind or role the CHECKs
/// should have refused): an internal error, with no row data.
fn bad_row<E>(_e: E) -> DbError {
    DbError::Other("identity: a stored value does not fit its view".into())
}
