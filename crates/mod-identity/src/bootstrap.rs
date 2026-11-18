//! `bootstrap_project`: the host-only creation of a project and its lead (cox's piece 11 contract
//! r0.3; W1-3, W1-11, W1-18). There is no HTTP route.
//!
//! The caller is the HOST path, in TWO host transactions (`ModuleTx::begin_host`, role
//! `cortex_install_admin`, actor `HostOperator`):
//!
//! 1. [`find_project`], read-only, in an install-level transaction (`HostContext.project = None`): the
//!    id of the project of that name, if there is one.
//! 2. [`bootstrap_project`], in a transaction whose `HostContext.project` is THAT id (a re-run), or a
//!    fresh id (the first run). In it the host path also mints the lead's token (mod-tokens) and writes
//!    the audit row (`project.bootstrapped`). THIS function writes no audit row and no token.
//!
//! **Why two:** the transaction's project (`cortex.project_id`) is fixed when it begins, and the token
//! and the audit row of the next steps are project-scoped. So `bootstrap_project` succeeds only if the
//! project it resolves IS the transaction's project: a re-run in a transaction that names another id is
//! [`BootstrapError::OtherProject`] (carrying the right id; roll back and begin again), never a success
//! with the wrong project in the context (sam, ID2C-01).
//!
//! - **First run:** creates the installation if none exists, the project (with the transaction's id),
//!   the lead worker (`worker`, `active`, no team, no job roles, mode `model`) and the two current
//!   `names` rows.
//! - **Re-run, same project and lead (compared by name key):** finds them and changes nothing.
//! - **A different lead for an existing project:** [`BootstrapError::Conflict`], nothing changes.
//! - **Another project with its own lead** in the same installation is allowed.
//! - **Concurrency:** every call locks the singleton installation row (`FOR UPDATE`) before it looks,
//!   so two calls cannot both create; no unique violation leaks out. The loser of a race is told
//!   `OtherProject` and repeats with the winner's id.
//! - **Fails closed:** a transaction that is not REALLY a host-path one (its database role is read, not
//!   its context), or that names no project, is refused before any write (sam, ID2C-02).

use cortex_db::{Actor, DbError, HOST_ROLE, ModuleTx};
use cortex_types::{InstallationId, ProjectId, SystemClock, WorkerId};

use crate::labels;

/// What the host path needs back: the ids, and whether this call created them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bootstrapped {
    pub installation: InstallationId,
    pub project: ProjectId,
    pub lead: WorkerId,
    /// True when this call created the project and its lead; false for a re-run.
    pub created: bool,
}

impl Bootstrapped {
    /// A re-run reissues the lead's token (mod-tokens); it creates nothing here.
    pub fn reissued(&self) -> bool {
        !self.created
    }
}

/// Why a bootstrap was refused. No variant carries a name or row data.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum BootstrapError {
    /// A project or lead name outside the name grammar (S3 R-18); nothing was written.
    #[error("invalid name")]
    Invalid,
    /// The project exists with a different lead (409 `conflict`); nothing was changed.
    #[error("the project exists with a different lead")]
    Conflict,
    /// The transaction is not a host-path one, or names no project: refused before any write.
    #[error("not a host-path transaction")]
    Context,
    /// The project exists under THIS id, but the transaction's project is another: nothing was
    /// changed. Roll back and begin again with `HostContext.project = Some(id)` (ID2C-01).
    #[error("the project exists under another id")]
    OtherProject(ProjectId),
    /// The database refused; the caller rolls the transaction back.
    #[error(transparent)]
    Db(#[from] DbError),
}

/// The project of a host-path transaction. The transaction's REAL database role must be the host role
/// (read from the database, not from its context: a module transaction can carry a `HostOperator`
/// actor in a public `TxContext`), and its context must be a host operator naming a project.
async fn require_host(tx: &mut ModuleTx) -> Result<ProjectId, BootstrapError> {
    let ProjectCtx::Named(project) = project_ctx(tx) else {
        return Err(BootstrapError::Context);
    };
    require_host_role(tx).await?;
    Ok(project)
}

enum ProjectCtx {
    Named(ProjectId),
    NotHost,
}

fn project_ctx(tx: &ModuleTx) -> ProjectCtx {
    match (&tx.context().actor, tx.context().project) {
        (Actor::HostOperator { .. }, Some(p)) => ProjectCtx::Named(p),
        _ => ProjectCtx::NotHost,
    }
}

/// A harmless read: the role this transaction runs under. It must be [`HOST_ROLE`].
async fn require_host_role(tx: &mut ModuleTx) -> Result<(), BootstrapError> {
    if !matches!(tx.context().actor, Actor::HostOperator { .. }) {
        return Err(BootstrapError::Context);
    }
    let role: String = sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(tx.executor()?)
        .await
        .map_err(DbError::from)?;
    if role == HOST_ROLE {
        Ok(())
    } else {
        Err(BootstrapError::Context)
    }
}

/// What [`find_project`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FoundProject {
    pub installation: InstallationId,
    pub project: ProjectId,
    pub lead: WorkerId,
}

/// The project of that name in the installation, read-only. Runs in an install-level host-path
/// transaction (`HostContext.project = None`); the host path then begins the transaction of
/// [`bootstrap_project`] with the id it found (a re-run) or a fresh one. Takes no lock.
pub async fn find_project(
    tx: &mut ModuleTx,
    project_name: &str,
) -> Result<Option<FoundProject>, BootstrapError> {
    let Some(project_key) = labels::name_key(project_name) else {
        return Err(BootstrapError::Invalid);
    };
    require_host_role(tx).await?;
    let row: Option<(String, String, String)> = sqlx::query_as(
        "SELECT p.installation_id::text, p.id::text, p.lead_worker_id::text FROM identity.projects p \
         JOIN identity.names n ON n.entity_kind = 'project' AND n.entity_id = p.id AND n.current \
         WHERE n.name_key = $1",
    )
    .bind(project_key)
    .fetch_optional(tx.executor()?)
    .await
    .map_err(DbError::from)?;
    let bad = || DbError::Other("identity: a stored id does not fit".into());
    row.map(|(i, p, l)| {
        Ok(FoundProject {
            installation: InstallationId::parse(&i).map_err(|_| bad())?,
            project: ProjectId::parse(&p).map_err(|_| bad())?,
            lead: WorkerId::parse(&l).map_err(|_| bad())?,
        })
    })
    .transpose()
}

/// Creates or finds the project `project_name` and its lead `lead_name`, inside the caller's
/// host-path transaction. The new project's id is the transaction's own `HostContext.project`.
pub async fn bootstrap_project(
    tx: &mut ModuleTx,
    project_name: &str,
    lead_name: &str,
) -> Result<Bootstrapped, BootstrapError> {
    // Before any SQL: the names (the one validator; a NUL is no name), then the context.
    let (Some(project_key), Some(lead_key)) =
        (labels::name_key(project_name), labels::name_key(lead_name))
    else {
        return Err(BootstrapError::Invalid);
    };
    let new_project = require_host(tx).await?;

    // The singleton installation: create it if there is none (a second concurrent creator waits on the
    // unique index and then does nothing), then LOCK its row. Every bootstrap serialises here, so two
    // calls cannot both create the project and no unique violation can leak out.
    sqlx::query(
        "INSERT INTO identity.installations (id, label) VALUES ($1::uuid, 'installation') ON CONFLICT DO NOTHING",
    )
    .bind(InstallationId::new(&SystemClock).to_string())
    .execute(tx.executor()?)
    .await
    .map_err(DbError::from)?;
    let installation: String =
        sqlx::query_scalar("SELECT id::text FROM identity.installations FOR UPDATE")
            .fetch_one(tx.executor()?)
            .await
            .map_err(DbError::from)?;
    let bad = || DbError::Other("identity: a stored id does not fit".into());
    let installation_id = InstallationId::parse(&installation).map_err(|_| bad())?;

    // Is there a project of this name (compared by key) in the installation?
    let found: Option<(String, String)> = sqlx::query_as(
        "SELECT p.id::text, p.lead_worker_id::text FROM identity.projects p \
         JOIN identity.names n ON n.entity_kind = 'project' AND n.entity_id = p.id AND n.current \
         WHERE p.installation_id = $1::uuid AND n.name_key = $2",
    )
    .bind(&installation)
    .bind(&project_key)
    .fetch_optional(tx.executor()?)
    .await
    .map_err(DbError::from)?;
    if let Some((project, lead)) = found {
        let existing = ProjectId::parse(&project).map_err(|_| bad())?;
        // The transaction's project is fixed; the next steps of the host path run under it.
        if existing != new_project {
            return Err(BootstrapError::OtherProject(existing));
        }
        // A re-run: the same lead (by key) changes nothing; another lead is a conflict.
        let current: Option<String> = sqlx::query_scalar(
            "SELECT name_key FROM identity.names WHERE entity_kind = 'worker' AND entity_id = $1::uuid AND current",
        )
        .bind(&lead)
        .fetch_optional(tx.executor()?)
        .await
        .map_err(DbError::from)?;
        return if current.as_deref() == Some(lead_key.as_str()) {
            Ok(Bootstrapped {
                installation: installation_id,
                project: existing,
                lead: WorkerId::parse(&lead).map_err(|_| bad())?,
                created: false,
            })
        } else {
            Err(BootstrapError::Conflict)
        };
    }

    // First run for this project: the project (its lead key is deferred), the lead, the two names.
    let lead = WorkerId::new(&SystemClock);
    sqlx::query(
        "INSERT INTO identity.projects (id, installation_id, lead_worker_id) VALUES ($1::uuid, $2::uuid, $3::uuid)",
    )
    .bind(new_project.to_string())
    .bind(&installation)
    .bind(lead.to_string())
    .execute(tx.executor()?)
    .await
    .map_err(DbError::from)?;
    sqlx::query(
        "INSERT INTO identity.workers (id, project_id, kind) VALUES ($1::uuid, $2::uuid, 'worker')",
    )
    .bind(lead.to_string())
    .bind(new_project.to_string())
    .execute(tx.executor()?)
    .await
    .map_err(DbError::from)?;
    for (kind, entity, scope, label, key) in [
        (
            "project",
            new_project.to_string(),
            installation.clone(),
            project_name,
            &project_key,
        ),
        (
            "worker",
            lead.to_string(),
            new_project.to_string(),
            lead_name,
            &lead_key,
        ),
    ] {
        sqlx::query(
            "INSERT INTO identity.names (entity_kind, entity_id, scope_id, project_id, name_key, label, current) \
             VALUES ($1, $2::uuid, $3::uuid, $4::uuid, $5, $6, true)",
        )
        .bind(kind)
        .bind(entity)
        .bind(scope)
        .bind(new_project.to_string())
        .bind(key)
        .bind(label)
        .execute(tx.executor()?)
        .await
        .map_err(DbError::from)?;
    }
    Ok(Bootstrapped {
        installation: installation_id,
        project: new_project,
        lead,
        created: true,
    })
}
