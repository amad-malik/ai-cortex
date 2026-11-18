//! `plug-identity`: the Wave 0 plug's minimal `IdentityApi` provider (socket contract section 3;
//! sam's checklist section 2). **A fixture, not mod-identity:** two tables, read only, its own
//! schema and role, enough to plug a provider and a consumer through one server, registry and
//! database seam.
//!
//! - **Manifest:** id `plug_identity`, schema `plug_identity`, provides `IdentityApi`, declares
//!   `identity:read`.
//! - **Port:** `project`, `worker`, `roster`, each delegated into the caller's `ModuleTx` under
//!   `cortex_m_plug_identity`; the project is the transaction's (row-level security).
//! - **Route:** `GET /v2/identity/roster` (scope `identity:read`), its own transaction through
//!   `Ctx::begin`.
//! - **Role:** [`BOOTSTRAP_SQL`] creates `cortex_m_plug_identity` and grants it to `cortex_app`.
//!   A superuser runs it before the migrations (finn's recommendation to cox: roles from the
//!   bootstrap, not from migrations).

use std::sync::Arc;

use async_trait::async_trait;
use axum::Json;
use axum::extract::State;
use axum::http::Method;
use cortex_db::{DbError, Migration, MigrationSet, ModuleTx};
use cortex_module::route::AppState;
use cortex_module::{
    BoxFuture, CONTRACT_VERSION, Ctx, DefaultRole, Health, IdentityApi, Manifest, Piece, Port,
    PortError, Ports, PortsBuilder, Problem, ProjectView, Role, Routes, ScopeClass, ScopeDecl,
    TargetRule, WorkerKind, WorkerView,
};
use cortex_types::{InstallationId, ModuleId, ProjectId, TeamId, WorkerId};
use serde::Serialize;

/// The piece's id, schema and role suffix.
pub const ID: &str = "plug_identity";

/// The installation every fixture project belongs to unless a test seeds another: the column's
/// default in the fixture migration (delta 5: `ProjectView.installation`). A UUIDv7 that is not
/// random, so a seed that names no installation (mira's, sam's) is unchanged.
pub const FIXTURE_INSTALLATION: &str = "01930000-0000-7000-8000-000000000001";

/// The scope its route needs.
pub const SCOPE_READ: &str = "identity:read";

pub static MANIFEST: Manifest = Manifest {
    id: ID,
    version: env!("CARGO_PKG_VERSION"),
    contract: CONTRACT_VERSION,
    core: false,
    schema: ID,
    provides: &[Port::Identity],
    uses: &[],
    scopes: &[ScopeDecl {
        name: SCOPE_READ,
        class: ScopeClass::Work,
        default_roles: &[
            DefaultRole::ProjectLead,
            DefaultRole::TeamLead,
            DefaultRole::Member,
        ],
    }],
    problems: &[],
};

/// Run by a superuser before the migrations: the piece's role, and `cortex_app`'s membership.
///
/// Idempotent, and safe when two run at once: roles are cluster-wide, so two bootstraps in two
/// databases can race, and an advisory lock (per database) cannot order them. An existing role
/// (42710) or membership, and the loser of a race (23505), are both fine. (DB run 3 at e55c55b
/// failed on the first form, IF NOT EXISTS then CREATE ROLE, with 23505.)
pub const BOOTSTRAP_SQL: &str = r#"
DO $$ BEGIN
  BEGIN
    CREATE ROLE cortex_m_plug_identity NOLOGIN;
  EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL;
  END;
  BEGIN
    GRANT cortex_m_plug_identity TO cortex_app;
  EXCEPTION WHEN unique_violation THEN NULL;
  END;
END $$;
"#;

/// The piece.
pub struct PlugIdentity {
    id: ModuleId,
}

impl PlugIdentity {
    pub fn new() -> PlugIdentity {
        PlugIdentity {
            id: ModuleId::parse(ID).expect("a valid module id"),
        }
    }
}

impl Default for PlugIdentity {
    fn default() -> Self {
        Self::new()
    }
}

impl Piece for PlugIdentity {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn migrations(&self) -> MigrationSet {
        MigrationSet {
            module: self.id.clone(),
            schema: ID,
            steps: vec![Migration {
                version: 1,
                name: "plug_identity",
                up: include_str!("../migrations/0001_plug_identity.up.sql"),
                down: include_str!("../migrations/0001_plug_identity.down.sql"),
            }],
        }
    }

    fn provide(&self, ports: &mut PortsBuilder) {
        ports.identity(Arc::new(Provider::new()));
    }

    fn routes(&self, _ports: &Ports) -> Routes {
        Routes::new().read(
            Method::GET,
            "/v2/identity/roster",
            SCOPE_READ,
            TargetRule::None,
            roster_route,
        )
    }

    fn health(&self) -> BoxFuture<'_, Health> {
        Box::pin(async { Health::Ready })
    }
}

/// The `IdentityApi` provider: every call runs in the caller's transaction, delegated to
/// `cortex_m_plug_identity`.
pub struct Provider {
    id: ModuleId,
}

impl Provider {
    pub fn new() -> Provider {
        Provider {
            id: ModuleId::parse(ID).expect("a valid module id"),
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

/// A worker, as the roster route answers it.
#[derive(Debug, Serialize)]
pub struct RosterEntry {
    pub id: String,
    pub name: String,
    pub kind: &'static str,
    pub role: &'static str,
    pub job_roles: Vec<String>,
    pub active: bool,
}

/// `GET /v2/identity/roster`: the caller's project's workers, in its own transaction.
async fn roster_route(
    State(st): State<AppState>,
    ctx: Ctx,
) -> Result<Json<Vec<RosterEntry>>, Problem> {
    let id = ModuleId::parse(ID).expect("a valid module id");
    let mut tx = ctx.begin(&st.pool, &id).await?;
    let rows = roster_rows(&mut tx).await?;
    tx.commit().await?;
    Ok(Json(
        rows.into_iter()
            .map(|w| RosterEntry {
                id: w.id.to_string(),
                name: w.name,
                kind: match w.kind {
                    WorkerKind::Worker => "worker",
                    WorkerKind::Service => "service",
                },
                role: match w.role {
                    Role::ProjectLead => "project_lead",
                    Role::TeamLead(_) => "team_lead",
                    Role::Member(_) => "member",
                },
                job_roles: w.job_roles,
                active: w.active,
            })
            .collect(),
    ))
}

type WorkerRow = (
    String,
    String,
    String,
    String,
    Option<String>,
    Vec<String>,
    bool,
);

/// A worker query: literal SQL only (sqlx 0.9's `SqlSafeStr`), with bind parameters.
macro_rules! worker_sql {
    ($tail:literal) => {
        concat!(
            "SELECT id::text, name, kind, role, team_id::text, job_roles, active \
             FROM plug_identity.workers",
            $tail
        )
    };
}

async fn project_row(tx: &mut ModuleTx) -> Result<Option<ProjectView>, DbError> {
    let row: Option<(String, String, String, String, String)> = sqlx::query_as(
        "SELECT id::text, name, state, lead::text, installation_id::text FROM plug_identity.projects",
    )
    .fetch_optional(tx.executor()?)
    .await?;
    row.map(|(id, name, state, lead, installation)| {
        Ok(ProjectView {
            id: ProjectId::parse(&id).map_err(bad_row)?,
            name,
            state,
            lead: WorkerId::parse(&lead).map_err(bad_row)?,
            installation: InstallationId::parse(&installation).map_err(bad_row)?,
            // The plug has no lifetime column: the default (S1 A3).
            token_lifetime_days: 90,
        })
    })
    .transpose()
}

async fn worker_row(tx: &mut ModuleTx, w: WorkerId) -> Result<Option<WorkerView>, DbError> {
    let row: Option<WorkerRow> = sqlx::query_as(worker_sql!(" WHERE id = $1::uuid"))
        .bind(w.to_string())
        .fetch_optional(tx.executor()?)
        .await?;
    row.map(worker_view).transpose()
}

async fn roster_rows(tx: &mut ModuleTx) -> Result<Vec<WorkerView>, DbError> {
    let rows: Vec<WorkerRow> = sqlx::query_as(worker_sql!(" ORDER BY name, id"))
        .fetch_all(tx.executor()?)
        .await?;
    rows.into_iter().map(worker_view).collect()
}

fn worker_view(
    (id, name, kind, role, team, job_roles, active): WorkerRow,
) -> Result<WorkerView, DbError> {
    let team = team
        .map(|t| TeamId::parse(&t).map_err(bad_row))
        .transpose()?;
    Ok(WorkerView {
        id: WorkerId::parse(&id).map_err(bad_row)?,
        name,
        kind: match kind.as_str() {
            "worker" => WorkerKind::Worker,
            "service" => WorkerKind::Service,
            _ => return Err(bad_row(())),
        },
        role: match (role.as_str(), team) {
            ("project_lead", _) => Role::ProjectLead,
            ("team_lead", Some(t)) => Role::TeamLead(t),
            ("member", t) => Role::Member(t),
            _ => return Err(bad_row(())),
        },
        job_roles,
        active,
    })
}

/// A stored value the views cannot carry (an id that is not a UUIDv7, a kind or role the CHECKs
/// should have refused): an internal error, with no row data.
fn bad_row<E>(_e: E) -> DbError {
    DbError::Other("plug_identity: a stored value does not fit its view".into())
}
