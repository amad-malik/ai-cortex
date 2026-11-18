//! `mod-identity` (piece 2, Wave 1): projects, teams, workers, names and the roster.
//!
//! - **Schema:** `identity`, migration 0001 is the S2 revision 6 pack file `identity.sql`
//!   (`c08fab0dc0d0…`) byte for byte; a test compares it with the pin.
//! - **Port:** [`Provider`] implements `IdentityApi` (`project`, `worker`, `roster`) in the caller's
//!   `ModuleTx`, under `cortex_m_identity`, inside the caller's project only.
//! - **Routes (2a, the reads):** `GET /v2/projects/{id}`, `GET /v2/teams`, `GET /v2/workers`
//!   (scope `roster:read`). The four writes and the audit records are 2b.
//! - **Labels:** [`labels`] is the ONE validator for names and job roles (S3 R-18).
//!
//! STUB (Wave 1, row 7a08bf86): the API exists, the bodies are `todo!()`.

pub mod bootstrap;
pub mod labels;
mod problems;
mod provider;
pub mod read;
pub mod write;

use cortex_db::{Migration, MigrationSet};
use cortex_module::{
    BoxFuture, CONTRACT_VERSION, DefaultRole, Health, Manifest, Piece, Port, Ports, PortsBuilder,
    Routes, ScopeClass, ScopeDecl,
};
use cortex_types::ModuleId;
use std::sync::Arc;

pub use bootstrap::{BootstrapError, Bootstrapped, FoundProject, bootstrap_project, find_project};
pub use provider::Provider;

/// The piece's id, schema and role suffix (`cortex_m_identity`).
pub const ID: &str = "identity";

pub const SCOPE_ROSTER_READ: &str = "roster:read";
pub const SCOPE_TEAM_CREATE: &str = "team:create";
pub const SCOPE_TEAM_ASSIGN: &str = "team:assign";
pub const SCOPE_WORKER_CREATE: &str = "worker:create";
pub const SCOPE_WORKER_RETIRE: &str = "worker:retire";

/// The five scopes this piece declares (S3 r2.5 `x-scopes`, `x-piece: identity`). Renames are cut.
pub static MANIFEST: Manifest = Manifest {
    id: ID,
    version: env!("CARGO_PKG_VERSION"),
    contract: CONTRACT_VERSION,
    core: true,
    schema: ID,
    provides: &[Port::Identity],
    uses: &[Port::Audit],
    scopes: &[
        ScopeDecl {
            name: SCOPE_ROSTER_READ,
            class: ScopeClass::Work,
            default_roles: &[
                DefaultRole::ProjectLead,
                DefaultRole::TeamLead,
                DefaultRole::Member,
            ],
        },
        ScopeDecl {
            name: SCOPE_TEAM_CREATE,
            class: ScopeClass::Admin,
            default_roles: &[DefaultRole::ProjectLead],
        },
        ScopeDecl {
            name: SCOPE_TEAM_ASSIGN,
            class: ScopeClass::Admin,
            default_roles: &[DefaultRole::ProjectLead],
        },
        ScopeDecl {
            name: SCOPE_WORKER_CREATE,
            class: ScopeClass::Admin,
            default_roles: &[DefaultRole::ProjectLead],
        },
        ScopeDecl {
            name: SCOPE_WORKER_RETIRE,
            class: ScopeClass::Admin,
            default_roles: &[DefaultRole::ProjectLead, DefaultRole::TeamLead],
        },
    ],
    problems: &[
        problems::NAME_INVALID,
        problems::NAME_TAKEN,
        problems::JOB_ROLES_INVALID,
        problems::LEAD_PERMANENT,
        problems::TARGET_RETIRED,
        problems::TEAM_LEAD_TAKEN,
        problems::TEAM_LEAD_IN_POST,
        problems::REASON_REQUIRED,
    ],
};

/// Migration 0001: the S2 pack file `identity.sql`, unchanged.
pub const MIGRATION_0001_UP: &str = include_str!("../migrations/0001_identity.up.sql");
pub const MIGRATION_0001_DOWN: &str = include_str!("../migrations/0001_identity.down.sql");

/// Run by a superuser before the migrations: the piece's role, and `cortex_app`'s membership.
/// Idempotent and safe when two run at once (the plug's form: an existing role or a lost race is
/// fine). `cortex_install_admin` and `cortex_owner` are the socket's provisioning, not this piece's.
pub const BOOTSTRAP_SQL: &str = r#"
DO $$ BEGIN
  BEGIN
    CREATE ROLE cortex_m_identity NOLOGIN;
  EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL;
  END;
  BEGIN
    GRANT cortex_m_identity TO cortex_app;
  EXCEPTION WHEN unique_violation THEN NULL;
  END;
END $$;
"#;

/// The piece.
pub struct ModIdentity {
    id: ModuleId,
}

impl ModIdentity {
    pub fn new() -> ModIdentity {
        ModIdentity {
            id: ModuleId::parse(ID).expect("a valid module id"),
        }
    }
}

impl Default for ModIdentity {
    fn default() -> Self {
        Self::new()
    }
}

impl Piece for ModIdentity {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn migrations(&self) -> MigrationSet {
        MigrationSet {
            module: self.id.clone(),
            schema: ID,
            steps: vec![Migration {
                version: 1,
                name: "identity",
                up: MIGRATION_0001_UP,
                down: MIGRATION_0001_DOWN,
            }],
        }
    }

    fn provide(&self, ports: &mut PortsBuilder) {
        ports.identity(Arc::new(Provider::new()));
    }

    fn routes(&self, ports: &Ports) -> Routes {
        let audit = ports.audit().expect("the registry checked `uses`");
        write::add(read::add(Routes::new()), audit)
    }

    fn health(&self) -> BoxFuture<'_, Health> {
        Box::pin(async { Health::Ready })
    }
}
