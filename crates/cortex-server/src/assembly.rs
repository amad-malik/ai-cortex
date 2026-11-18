//! The compile-time piece list and the assembly of the registry, SHARED by `serve`, `migrate` and `bootstrap` (piece 11; cox's request of
//! 15:2xZ: one assembly, so every command sees the same pieces in the same order, and `routes(&Ports)` has run before the first audit row, W1-37).
//!
//! - [`compiled`]: every piece this binary compiles in, in migration order (dependencies before dependents; `audit` and `tokens` join when
//!   they merge, `audit` before `identity`). `identity` is core and always in; each other piece is ONE `#[cfg(feature = "mod-x")]` line.
//!   There is NO default feature.
//! - [`Wiring`], [`compiled_with_wiring`], [`assemble_wired`]: what the server hands to a piece once its pool exists (today `mod-tokens`'
//!   verifier, which fails closed until bound); `serve` binds it, `migrate` and `bootstrap` do not need it.
//! - [`assemble`]: `Registry::assemble` over them with the config's `[modules] enabled`. The pool is NOT an input: its module registry is
//!   derived FROM the assembled registry ([`db_config`], contract section 4b), so the order is assemble, then the pool.
//! - [`ordered_modules`]: `platform` first, then every enabled piece in migration order: what `migrate::run` takes.
//!
//! Errors are the raw [`StartupError`]s (names only); each command adds its own prefix and joins them with [`startup_line`].

use std::collections::BTreeSet;
use std::sync::Arc;

use cortex_config::{Config, DbHost};
use cortex_db::{DbConfig, DbEndpoint, DbPool, ModuleRegistry};
use cortex_module::{Assembly, Piece, Registry, StartupError};
use cortex_types::{Clock, ModuleId, ProblemCodeMinter};

use crate::secretfile::read_secret_file;

/// Policy's STARTUP check over the enabled manifests (S3 `x-scopes-rule`: mod-policy refuses startup on a scope it does not know, a
/// wrong class, different default roles, or an admin scope granted to a member). `Registry::assemble` cannot make it, and until this
/// call nothing did. ONE line for ALL the refusals, joined with "; ", in manifest order then declaration order; each names the SCOPE
/// and nothing else (the scope names are compiled-in text, never operator input).
///
/// Production path only: `assemble_with` (fixture pieces) does not call it, because a fixture's scopes are not in the real table.
#[cfg(feature = "mod-policy")]
pub fn check_policy_manifests<'a>(
    manifests: impl IntoIterator<Item = &'a cortex_module::Manifest>,
) -> Result<(), String> {
    use mod_policy::{Policy, PolicyError};
    Policy::check_manifests(manifests).map_err(|errors| {
        errors
            .iter()
            .map(|e| match e {
                PolicyError::UnknownToPolicy { scope } => {
                    format!("the policy has no row for scope {scope}")
                }
                PolicyError::ClassMismatch { scope } => {
                    format!(
                        "scope {scope} is declared in a different class than the policy's table"
                    )
                }
                PolicyError::RolesMismatch { scope } => {
                    format!("scope {scope} has default roles that differ from the policy's table")
                }
                PolicyError::AdminGrantsMember { scope } => {
                    format!("admin scope {scope} is granted to members")
                }
            })
            .collect::<Vec<_>>()
            .join("; ")
    })
}

/// [`check_policy_manifests`] over the pieces an assembled registry enables (a piece compiled in but not enabled is not held by the
/// registry, so it is not checked).
#[cfg(feature = "mod-policy")]
pub fn check_policy(registry: &Registry) -> Result<(), String> {
    check_policy_manifests(registry.pieces().map(|p| p.manifest()))
}

/// The codes `cortex-http` mints for itself: no piece may declare them (`Assembly.chain_codes`).
pub const CHAIN_CODES: &[&str] = &[
    cortex_http::problem::INTERNAL_ERROR,
    cortex_http::problem::UNAVAILABLE,
    cortex_http::problem::BUSY,
    cortex_http::problem::CONFLICT,
];

/// The size of the `cortex_app` pool. Not a config key in v0.2.001.
pub const POOL_SIZE: u32 = 10;

/// Why the pool's configuration could not be made. Fixed text, no path, no value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DbConfigError {
    /// There is no `[database]` table.
    NoDatabaseTable,
    /// The app password file could not be read.
    PasswordUnreadable,
}

impl std::fmt::Display for DbConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            DbConfigError::NoDatabaseTable => "missing: database",
            DbConfigError::PasswordUnreadable => "the database password file could not be read",
        })
    }
}

/// What the server must hand to a piece AFTER the pool exists. A piece that needs the pool before any request opens a transaction (today
/// only `mod-tokens`' verifier: it runs in the chain BEFORE the request's transaction, so it cannot use the request's) is boxed as a
/// `dyn Piece`, and the server could no longer reach it; so [`compiled_with_wiring`] keeps a binder per such piece, and `serve_with` runs
/// them once the pool is built (the contract in `mod-tokens`: "Wiring for the server (piece 11)"). Until a piece is bound, it fails CLOSED.
#[derive(Clone, Default)]
pub struct Wiring {
    binders: Vec<Binder>,
}

/// What hands the pool to one piece.
type Binder = Arc<dyn Fn(&Arc<DbPool>) + Send + Sync>;

impl Wiring {
    /// Nothing to bind (fixture pieces).
    pub fn none() -> Wiring {
        Wiring::default()
    }

    /// One more piece to bind.
    pub fn with(mut self, binder: impl Fn(&Arc<DbPool>) + Send + Sync + 'static) -> Wiring {
        self.binders.push(Arc::new(binder));
        self
    }

    /// How many pieces are bound when the pool exists.
    pub fn len(&self) -> usize {
        self.binders.len()
    }

    pub fn is_empty(&self) -> bool {
        self.binders.is_empty()
    }

    /// Hand the pool to every piece that needs it.
    pub fn bind_pool(&self, pool: &Arc<DbPool>) {
        for binder in &self.binders {
            binder(pool);
        }
    }
}

/// The pieces compiled into this binary, in migration order.
pub fn compiled() -> Vec<Box<dyn Piece>> {
    compiled_with_wiring().0
}

/// [`compiled`] and the [`Wiring`] of the pieces that need the pool (a new list each call: a piece and its binder belong together).
#[allow(clippy::vec_init_then_push)]
pub fn compiled_with_wiring() -> (Vec<Box<dyn Piece>>, Wiring) {
    // One `cfg` line per piece: attributes cannot sit on `vec!` elements, so this is `push` after `new` on purpose.
    #[allow(unused_mut)]
    let mut pieces: Vec<Box<dyn Piece>> = Vec::new();
    let mut wiring = Wiring::none();
    // audit is core and always compiled in, FIRST: identity writes its audit rows through the audit sink, so the audit schema exists before
    // identity's (cox 20:07Z; marv's pin 6eeccbd). No feature.
    pieces.push(Box::new(mod_audit::AuditPiece::new()));
    // identity is core and always compiled in (`bootstrap` calls it directly): no feature.
    pieces.push(Box::new(mod_identity::ModIdentity::new()));
    // tokens is core and always compiled in (cox 19:41Z: `TokenMinter` calls it, so it is a non-optional dependency): no feature. It uses identity
    // and audit, so it comes AFTER both (audit, identity, tokens).
    // Its verifier fails closed until the pool is bound, so the binder is kept: the box below hides the piece from the server.
    let tokens = mod_tokens::ModTokens::new();
    let late = tokens.late();
    wiring = wiring.with(move |pool| {
        late.bind_pool(pool.clone());
    });
    pieces.push(Box::new(tokens));
    #[cfg(feature = "mod-policy")]
    pieces.push(Box::new(mod_policy::ModPolicy::new()));
    #[cfg(feature = "mod-records")]
    pieces.push(Box::new(mod_records::ModRecords::new()));
    // search reads the records views, so it comes after records.
    #[cfg(feature = "mod-search")]
    pieces.push(Box::new(mod_search::Search::new()));
    #[cfg(feature = "mod-handoffs")]
    pieces.push(Box::new(mod_handoffs::Handoffs::new()));
    #[cfg(feature = "mod-boot")]
    pieces.push(Box::new(mod_boot::Boot::new()));
    (pieces, wiring)
}

/// The first module id that `[modules] enabled` names twice, if any (`serve` refuses such a config).
pub fn duplicate_enabled(enabled: &[ModuleId]) -> Option<&ModuleId> {
    let mut seen = BTreeSet::new();
    enabled.iter().find(|id| !seen.insert(*id))
}

/// `Registry::assemble` over `compiled` with a caller-held minter (a test shares one per binary).
pub fn assemble_with(
    compiled: Vec<Box<dyn Piece>>,
    enabled: &[ModuleId],
    minter: &ProblemCodeMinter,
) -> Result<Registry, Vec<StartupError>> {
    Registry::assemble(Assembly {
        compiled,
        enabled,
        minter,
        chain_codes: CHAIN_CODES,
    })
}

/// The production entry: this binary's [`compiled`] pieces, the config's enabled set, the process's ONE minter.
pub fn assemble(cfg: &Config) -> Result<Registry, Vec<StartupError>> {
    // The process's ONE minter: a command assembles once per process.
    let minter = ProblemCodeMinter::take().expect("the registry is assembled once per process");
    assemble_with(compiled(), &cfg.modules.enabled, &minter)
}

/// [`assemble`] and the [`Wiring`] the server binds once its pool exists (`serve` uses this; `migrate` and `bootstrap` have no use for the
/// wiring and keep [`assemble`]).
pub fn assemble_wired(cfg: &Config) -> Result<(Registry, Wiring), Vec<StartupError>> {
    let minter = ProblemCodeMinter::take().expect("the registry is assembled once per process");
    let (pieces, wiring) = compiled_with_wiring();
    assemble_with(pieces, &cfg.modules.enabled, &minter).map(|registry| (registry, wiring))
}

/// Every startup error on ONE line (`; ` between), names only.
pub fn startup_line(errors: &[StartupError]) -> String {
    errors
        .iter()
        .map(|e| e.to_string().replace('\n', " "))
        .collect::<Vec<_>>()
        .join("; ")
}

/// `platform`, then every enabled piece in migration order.
pub fn ordered_modules(registry: &Registry) -> Vec<ModuleId> {
    // `Registry::db_modules` is `platform` first, then every enabled piece in migration order.
    registry.db_modules()
}

/// The `cortex_app` pool's configuration: the config's `[database]`, the app password read from its file, and the module registry
/// of the ASSEMBLED registry (never a hand-built list). Statement cache on (production).
pub fn db_config(
    cfg: &Config,
    registry: &Registry,
    clock: Arc<dyn Clock>,
) -> Result<DbConfig, DbConfigError> {
    let db = cfg
        .database
        .as_ref()
        .ok_or(DbConfigError::NoDatabaseTable)?;
    // The strict reader: a regular file, no symlink, no FIFO block, at most 4096 bytes, UTF-8 without NUL; only one final line ending is not
    // part of the password (it may hold spaces); an empty password is refused. Its error names neither the path nor any content.
    let password =
        read_secret_file(&db.app_password_file).map_err(|_| DbConfigError::PasswordUnreadable)?;
    let endpoint = match &db.host {
        DbHost::SocketDir(dir) => DbEndpoint::Socket {
            dir: dir.clone(),
            port: db.port,
        },
        DbHost::Tcp(host) => DbEndpoint::Tcp {
            host: host.clone(),
            port: db.port,
        },
    };
    Ok(DbConfig {
        endpoint,
        database: db.name.clone(),
        app_user: db.app_user.clone(),
        app_password: password,
        max_connections: POOL_SIZE,
        statement_cache: true,
        registry: ModuleRegistry::new(registry.db_modules()),
        clock,
    })
}
