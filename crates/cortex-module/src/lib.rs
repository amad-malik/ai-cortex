//! `cortex-module` (S0): the socket every piece plugs into (Wave 0; socket contract
//! `mvp/socket/S0-S1-SOCKET-CONTRACT.md`, r4 plus the r5 changes named below).
//!
//! S0 is **only** registration, routes, scopes, health, migration metadata and the trait
//! signatures. It holds no business rule and opens no listener.
//!
//! - [`Piece`]: what a piece is (its static [`Manifest`], its migrations, the ports it provides,
//!   its routes, its health).
//! - [`Registry`]: `cortex-server` assembles the compiled pieces, keeps the enabled ones and runs
//!   every startup check; each error names pieces, ports, scopes or paths only.
//! - [`ports`]: the six ports and their view types. A consumer never depends on a provider's
//!   crate. A port runs inside the caller's `ModuleTx`, takes its project and actor from
//!   `tx.context()` only, and returns owned values.
//! - [`route`]: the route table, `Ctx`, `Reply`, `Problem` and `ProblemSpec`. mira's `cortex-http`
//!   owns the chain that runs them (her file `mvp/socket/cortex-http-signatures.md`).
//!
//! **r5 changes over the contract's r4 text** (sophi's F-3 and F-5; finn's answers of 5 Nov):
//! `Principal` carries the token's `project`; `Role` carries the caller's team;
//! `Target` and `WorkerView` carry the worker's `kind`.

pub mod manifest;
pub mod piece;
pub mod ports;
pub mod registry;
pub mod route;
pub mod strict;

pub use manifest::{CONTRACT_VERSION, DefaultRole, Manifest, Port, ScopeClass, ScopeDecl};
pub use piece::{BoxFuture, Health, Piece};
pub use ports::{
    AuditDetails, AuditSink, AuditTarget, AuthError, DetailCode, DetailRef, DetailValue,
    HANDOFF_LIST_CAP, HandoffKind, HandoffTo, HandoffView, HandoffsApi, IdentityApi, PortError,
    Ports, PortsBuilder, Principal, ProjectView, RecordCursor, RecordPage, RecordState, RecordView,
    RecordsApi, Role, ScopePolicy, SectionName, SectionView, SourceView, Target, TargetRule,
    TokenVerifier, WorkerKind, WorkerView,
};
pub use registry::{Assembly, HEALTH_TIMEOUT, PieceHealth, Registry, StartupError};
pub use route::{
    Authorization, Authorized, Ctx, Problem, ProblemSpec, Reply, RouteInfo, Routes, Status,
    StrictQuery,
};
pub use strict::{StrictError, strict_from_query, strict_from_slice};
