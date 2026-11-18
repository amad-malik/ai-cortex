//! The manifest: a Rust value, checked at compile time (contract r4 ask 3; section 1.1). r7's TOML
//! keys map one to one; the cut keys return with their pieces.

use crate::route::ProblemSpec;

/// The socket's contract version. v0.2.001's server accepts exactly this one.
pub const CONTRACT_VERSION: u32 = 1;

/// A port: one of the six traits pieces reach each other through (section 1.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Port {
    /// `IdentityApi` (mod-identity).
    Identity,
    /// `RecordsApi` (mod-records).
    Records,
    /// `HandoffsApi` (mod-handoffs).
    Handoffs,
    /// `TokenVerifier` (mod-tokens); called by the chain.
    TokenVerifier,
    /// `ScopePolicy` (mod-policy); called by the chain and by handlers.
    ScopePolicy,
    /// `AuditSink` (mod-audit).
    Audit,
}

impl Port {
    pub const ALL: [Port; 6] = [
        Port::Identity,
        Port::Records,
        Port::Handoffs,
        Port::TokenVerifier,
        Port::ScopePolicy,
        Port::Audit,
    ];

    /// The trait's name, for startup errors.
    pub fn name(self) -> &'static str {
        match self {
            Port::Identity => "IdentityApi",
            Port::Records => "RecordsApi",
            Port::Handoffs => "HandoffsApi",
            Port::TokenVerifier => "TokenVerifier",
            Port::ScopePolicy => "ScopePolicy",
            Port::Audit => "AuditSink",
        }
    }
}

impl std::fmt::Display for Port {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// A scope's class (CONTRACTS r7 section 4: `self`, `work`, `admin`, `service`). The class
/// ceilings are mod-policy's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScopeClass {
    /// `self`: the caller's own things.
    SelfScope,
    Work,
    Admin,
    Service,
}

/// A role a scope is granted to by default (CONTRACTS r7 section 4). Expansion is mod-policy's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DefaultRole {
    ProjectLead,
    TeamLead,
    Member,
}

/// One `[[scopes]]` entry: a scope this piece declares, owned by it alone.
#[derive(Debug)]
pub struct ScopeDecl {
    /// `<resource>:<action>` (`ScopeName`'s grammar), unique across the enabled pieces.
    pub name: &'static str,
    pub class: ScopeClass,
    pub default_roles: &'static [DefaultRole],
}

/// A piece's manifest (section 1.1).
#[derive(Debug)]
pub struct Manifest {
    /// `ModuleId` grammar, `^[a-z][a-z0-9_]{0,31}$`; the role is `cortex_m_<id>`.
    pub id: &'static str,
    /// The piece's own version, reported only.
    pub version: &'static str,
    /// The socket's contract version; must be [`CONTRACT_VERSION`].
    pub contract: u32,
    /// A core piece cannot be disabled.
    pub core: bool,
    /// The one schema it owns and migrates.
    pub schema: &'static str,
    /// The ports it implements. Each must be provided in `Piece::provide`, and nothing else.
    pub provides: &'static [Port],
    /// The ports it calls. Its `routes` sees only these.
    pub uses: &'static [Port],
    pub scopes: &'static [ScopeDecl],
    /// Its problem codes, minted once at startup.
    pub problems: &'static [ProblemSpec],
}
