//! `mod-tokens` (piece 3, Wave 1): the token format and its verification (`TokenVerifier`), and the six routes
//! `tokens.whoami`, `tokens.revoke_self`, `tokens.issue`, `tokens.list`, `tokens.inspect`, `tokens.revoke`.
//!
//! - **Schema:** `tokens`; migration 0001 is the S2 pack file `tokens.sql` (`8357e64a…`, unchanged in revision
//!   7) byte for byte; a test compares it with the pin.
//! - **Port:** [`Verifier`] implements `TokenVerifier` (called by the chain once per request).
//! - **Library functions:** `issue_in_host_tx` for piece 11's bootstrap (inside the caller's host transaction, with the
//!   `BootstrapRun` intent), and `host::issue_as_host` for a caller that wants nothing but a token.
//!

pub mod entropy;
pub mod host;
pub mod issue;
pub mod lifetime;
pub mod problems;
pub mod read;
pub mod revoke;
pub mod store;
pub mod time;
pub mod token;
pub mod verify;

use std::sync::Arc;

use cortex_db::{DbPool, Migration, MigrationSet};
use cortex_module::{
    BoxFuture, CONTRACT_VERSION, DefaultRole, Health, Manifest, Piece, Port, Ports, PortsBuilder,
    Routes, ScopeClass, ScopeDecl,
};
use cortex_types::{Clock, ModuleId, SystemClock};

pub use entropy::{Entropy, OsEntropy};
pub use issue::{BootstrapRun, IssueError, Issued, Issuing, issue_in_host_tx};
pub use store::{Late, SqlStore};
pub use token::{EntropyError, Generated, Malformed, Parsed};
pub use verify::{DigestCompare, StoredToken, TokenState, Verifier, VerifySession, VerifyStore};

/// The piece's id, schema and role suffix (`cortex_m_tokens`).
pub const ID: &str = "tokens";

pub const SCOPE_SELF_READ: &str = "self:read";
pub const SCOPE_SELF_REVOKE: &str = "self:revoke";
pub const SCOPE_TOKEN_ISSUE: &str = "token:issue";
pub const SCOPE_TOKEN_LIST: &str = "token:list";
pub const SCOPE_TOKEN_REVOKE: &str = "token:revoke";

/// The five scopes this piece declares (S3 `x-scopes`, `x-piece: tokens`).
pub static MANIFEST: Manifest = Manifest {
    id: ID,
    version: env!("CARGO_PKG_VERSION"),
    contract: CONTRACT_VERSION,
    core: true,
    schema: ID,
    provides: &[Port::TokenVerifier],
    uses: &[Port::Identity, Port::Audit],
    scopes: &[
        ScopeDecl {
            name: SCOPE_SELF_READ,
            class: ScopeClass::SelfScope,
            default_roles: &[
                DefaultRole::ProjectLead,
                DefaultRole::TeamLead,
                DefaultRole::Member,
            ],
        },
        ScopeDecl {
            name: SCOPE_SELF_REVOKE,
            class: ScopeClass::SelfScope,
            default_roles: &[
                DefaultRole::ProjectLead,
                DefaultRole::TeamLead,
                DefaultRole::Member,
            ],
        },
        ScopeDecl {
            name: SCOPE_TOKEN_ISSUE,
            class: ScopeClass::Admin,
            default_roles: &[DefaultRole::ProjectLead, DefaultRole::TeamLead],
        },
        ScopeDecl {
            name: SCOPE_TOKEN_LIST,
            class: ScopeClass::Admin,
            default_roles: &[DefaultRole::ProjectLead, DefaultRole::TeamLead],
        },
        ScopeDecl {
            name: SCOPE_TOKEN_REVOKE,
            class: ScopeClass::Admin,
            default_roles: &[DefaultRole::ProjectLead, DefaultRole::TeamLead],
        },
    ],
    problems: &[problems::REASON_REQUIRED, problems::TARGET_RETIRED],
};

/// Migration 0001: the S2 pack file `tokens.sql`, unchanged.
pub const MIGRATION_0001_UP: &str = include_str!("../migrations/0001_tokens.up.sql");
pub const MIGRATION_0001_DOWN: &str = include_str!("../migrations/0001_tokens.down.sql");

/// Run by a superuser before the migrations: the piece's role, and `cortex_app`'s membership. Idempotent and
/// safe when two run at once (the plug's form). `cortex_install_admin` and `cortex_owner` are the socket's
/// provisioning, not this piece's.
pub const BOOTSTRAP_SQL: &str = r#"
DO $$ BEGIN
  BEGIN
    CREATE ROLE cortex_m_tokens NOLOGIN;
  EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL;
  END;
  BEGIN
    GRANT cortex_m_tokens TO cortex_app;
  EXCEPTION WHEN unique_violation THEN NULL;
  END;
END $$;
"#;

/// The fixed reason of a piece whose store is not bound yet (no data, no secret).
const NOT_BOUND: &str = "the token store is not bound";

/// The piece.
///
/// **Wiring for the server (piece 11):** the verifier needs the database pool, which the server builds from the
/// registry's module list AFTER assembly, and the `IdentityApi`, which the piece takes from its ports when its
/// routes are built. So the pool is bound with [`ModTokens::bind_pool`] once it exists. Until then, and until the
/// routes are built, every verification is `Unavailable` (503): the piece fails closed, never open.
pub struct ModTokens {
    id: ModuleId,
    late: Arc<Late>,
    clock: Arc<dyn Clock>,
    entropy: Arc<dyn Entropy>,
}

impl ModTokens {
    pub fn new() -> ModTokens {
        ModTokens {
            id: ModuleId::parse(ID).expect("a valid module id"),
            late: Arc::new(Late::default()),
            clock: Arc::new(SystemClock),
            entropy: Arc::new(OsEntropy),
        }
    }

    /// Another clock (the cells inject one).
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> ModTokens {
        self.clock = clock;
        self
    }

    /// Another source of randomness (a cell makes the OS source fail, or forces the stagger).
    pub fn with_entropy(mut self, entropy: Arc<dyn Entropy>) -> ModTokens {
        self.entropy = entropy;
        self
    }

    /// The late-bound slots, for a harness that boxes the piece before the pool exists.
    pub fn late(&self) -> Arc<Late> {
        self.late.clone()
    }

    /// Binds the database pool (once). Returns `false` if one was already bound.
    pub fn bind_pool(&self, pool: Arc<DbPool>) -> bool {
        self.late.bind_pool(pool)
    }
}

impl Default for ModTokens {
    fn default() -> Self {
        Self::new()
    }
}

impl Piece for ModTokens {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn migrations(&self) -> MigrationSet {
        MigrationSet {
            module: self.id.clone(),
            schema: ID,
            steps: vec![Migration {
                version: 1,
                name: "tokens",
                up: MIGRATION_0001_UP,
                down: MIGRATION_0001_DOWN,
            }],
        }
    }

    fn provide(&self, ports: &mut PortsBuilder) {
        let store = SqlStore::late(self.late.clone(), self.clock.clone());
        ports.token_verifier(Arc::new(Verifier::new(Arc::new(store), self.clock.clone())));
    }

    fn routes(&self, ports: &Ports) -> Routes {
        if let Some(identity) = ports.identity() {
            self.late.bind_identity(identity);
        }
        let identity = ports.identity().expect("the registry checked `uses`");
        let audit = ports.audit().expect("the registry checked `uses`");
        let r = issue::add(
            Routes::new(),
            Arc::clone(&identity),
            Arc::clone(&audit),
            self.clock.clone(),
            self.entropy.clone(),
        );
        let r = revoke::add(r, Arc::clone(&identity), audit);
        read::add(r, identity)
    }

    /// Ready only when the store is bound ([`Late::ready`]: the pool AND the identity port). Until then every verification
    /// is `Unavailable` (503), so saying `Ready` would be a lie that a readiness probe believes (cox's row ccfcb9bf).
    fn health(&self) -> BoxFuture<'_, Health> {
        let late = Arc::clone(&self.late);
        Box::pin(async move {
            if late.ready() {
                Health::Ready
            } else {
                Health::NotReady(NOT_BOUND)
            }
        })
    }
}
