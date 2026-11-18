//! `mod-records` (piece 6): typed, immutable, project-isolated records with lineage and the reserved sections.
//! Contracts: S2 revision 7 (`records.sql`; revision 8 once issued), S3 r2.6 (+ CHANGES sections 19 to 24), `tests/mod-records.md`.
//!
//! **Rule for every migration in schema `records` (cox, W1-14):** a function created by a later step is executable through PUBLIC
//! unless that step REVOKEs EXECUTE; grant back only what a module role needs. `tests/rec_db_schema.rs` db10 fails otherwise.

pub mod problems;
pub mod provider;
pub mod request;
pub mod routes;
pub mod store;
pub mod validate;

use cortex_db::{Migration, MigrationSet};
use cortex_module::{
    BoxFuture, CONTRACT_VERSION, DefaultRole, Health, Manifest, Piece, Port, Routes, ScopeClass,
    ScopeDecl,
};
use cortex_types::ModuleId;

/// The piece's id and schema (the pack's `records` schema; role `cortex_m_records`).
pub const ID: &str = "records";

pub const SCOPE_READ: &str = "records:read";
pub const SCOPE_WRITE: &str = "records:write";
pub const SCOPE_LINEAGE: &str = "records:lineage";

const ALL_ROLES: &[DefaultRole] = &[
    DefaultRole::ProjectLead,
    DefaultRole::TeamLead,
    DefaultRole::Member,
];

/// Run by a superuser before the migrations: the piece's role, and `cortex_app`'s membership (the plug's pattern:
/// roles come from the bootstrap, not from migrations). Idempotent, and safe when two run at once.
pub const BOOTSTRAP_SQL: &str = r#"
DO $$ BEGIN
  BEGIN
    CREATE ROLE cortex_m_records NOLOGIN;
  EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL;
  END;
  -- The migration grants the search views to this role (mod-search's own bootstrap creates it too; either order works).
  BEGIN
    CREATE ROLE cortex_m_search NOLOGIN;
  EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL;
  END;
  BEGIN
    GRANT cortex_m_records TO cortex_app;
  EXCEPTION WHEN unique_violation THEN NULL;
  END;
END $$;
"#;

pub static MANIFEST: Manifest = Manifest {
    id: ID,
    version: env!("CARGO_PKG_VERSION"),
    contract: CONTRACT_VERSION,
    core: false,
    schema: ID,
    provides: &[Port::Records],
    uses: &[Port::Identity, Port::Audit],
    scopes: &[
        ScopeDecl {
            name: SCOPE_READ,
            class: ScopeClass::Work,
            default_roles: ALL_ROLES,
        },
        ScopeDecl {
            name: SCOPE_WRITE,
            class: ScopeClass::Work,
            default_roles: ALL_ROLES,
        },
        ScopeDecl {
            name: SCOPE_LINEAGE,
            class: ScopeClass::Work,
            default_roles: ALL_ROLES,
        },
    ],
    problems: &problems::ALL,
};

/// The piece.
pub struct ModRecords {
    id: ModuleId,
}

impl ModRecords {
    pub fn new() -> ModRecords {
        ModRecords {
            id: ModuleId::parse(ID).expect("a valid module id"),
        }
    }
}

impl Default for ModRecords {
    fn default() -> Self {
        Self::new()
    }
}

impl Piece for ModRecords {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn migrations(&self) -> MigrationSet {
        MigrationSet {
            module: self.id.clone(),
            schema: ID,
            steps: vec![Migration {
                version: 1,
                name: "records",
                up: include_str!("../migrations/0001_records.up.sql"),
                down: include_str!("../migrations/0001_records.down.sql"),
            }],
        }
    }

    fn provide(&self, ports: &mut cortex_module::PortsBuilder) {
        ports.records(std::sync::Arc::new(provider::Provider::new()));
    }

    fn routes(&self, ports: &cortex_module::Ports) -> Routes {
        routes::routes(ports)
    }

    fn health(&self) -> BoxFuture<'_, Health> {
        Box::pin(async { Health::Ready })
    }
}
