//! `cortex-db`: one transaction per module operation, under that module's role and project,
//! with synchronous delegation (lane A CONTRACTS §3, r7; S1 §2.4, §9; S2 §5, §6).
//!
//! F-1b RED (b1, the core): every behaviour under test is a `todo!()` stub. The L-8 runtime-DDL
//! machinery (F1-31) is b2: `maintenance`, whose stubs are `todo!()` too.
//!
//! - **The role comes only from the trusted `ModuleRegistry`** (`cortex_m_<module>`), and an
//!   unregistered module is refused with `UnknownModule` before a connection is taken (r7).
//! - **Z-2's form, provisionally `param`** (kai, row 935ae6df): `BEGIN`, then one
//!   `SELECT set_config('role', $1, true), set_config('cortex.project_id', $2, true), …` round trip.
//! - **READ COMMITTED only** (r6).
//! - **The lock order and the audit seal** (r6, §3 "Lock order"): `ModuleTx` keeps a `LockLedger`.

mod error;
/// The idempotency helper (S2 §5; L-7: its rows are held by `cortex_m_platform`). CONTRACTS §3
/// names this module `idempotency`.
pub mod idempotency;
/// Runtime DDL only through maintenance functions (L-8; F1-31; b2). Banked for v0.2.001: built
/// only with the `maintenance` feature (the walking-skeleton plan; Wave 0's S1).
#[cfg(feature = "maintenance")]
pub mod maintenance;
/// The migration runner (S1 §2.2). CONTRACTS §3 names this module `migrate`.
pub mod migrate;
/// The migration guard (F1-13; reworks 2 to 9). Banked for v0.2.001: built only with the `guard`
/// feature; without it, release migrations are trusted (C-32).
#[cfg(feature = "guard")]
mod migrate_guard;
mod platform;
mod pool;
mod tx;

#[cfg(feature = "testkit")]
pub mod testkit;

pub use error::DbError;
pub use idempotency::{Idem, IdemError, IdemKey, IdemToken, StoredResponse};
pub use migrate::{MigrateError, Migration, MigrationSet};
pub use pool::{
    ConnectFailure, DbConfig, DbEndpoint, DbPool, ModuleRegistry, OwnerConfig, OwnerConn, Session,
};
pub use tx::{
    Actor, AdvisoryClass, HOST_ROLE, HostContext, LockLedger, LockStep, ModuleTx, TxContext,
    TxOptions,
};

/// `platform.objects` and `platform.refs` (S2 §6).
pub mod objects {
    pub use crate::platform::insert_object as insert;
}
pub mod refs {
    pub use crate::platform::{insert_ref as insert, refs_to as to};
}
