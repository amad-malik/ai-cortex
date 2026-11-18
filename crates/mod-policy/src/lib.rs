//! `mod-policy` (piece 4, Wave 1): roles to scopes, and the pure target rules. Fails closed.
//!
//! - **`allows(principal, scope)`:** the scope-only check the chain runs (cox C7). A principal whose
//!   kind is not `Worker` gets `false` for every scope (S3 R-12); an unknown scope gets `false`.
//! - **`target(principal, rule, target)`:** pure, synchronous, no I/O (`mvp/tests/mod-identity-and-mod-policy.md`
//!   section 3.2). Every `None` field the rule needs fails closed.
//! - **`check_manifests`:** the startup check the registry cannot make: every scope an enabled
//!   manifest declares must match this crate's table (class and default roles), and no `admin`
//!   scope may be granted to a member.
//!
//! Wave 1, row 7a08bf86. The table below is section 3.1 of `mvp/tests/mod-identity-and-mod-policy.md`;
//! the rules are section 3.2. The tests write their own oracle from the same text and never call
//! [`Policy::rows`] to decide an expected answer.

use std::sync::Arc;

use cortex_db::MigrationSet;
use cortex_module::{
    BoxFuture, CONTRACT_VERSION, DefaultRole, Health, Manifest, Piece, Port, Ports, PortsBuilder,
    Principal, Role, Routes, ScopeClass, ScopePolicy, Target, TargetRule, WorkerKind,
};
use cortex_types::{ModuleId, ScopeName};

/// The piece's id and schema (it owns no tables).
pub const ID: &str = "policy";

pub static MANIFEST: Manifest = Manifest {
    id: ID,
    version: env!("CARGO_PKG_VERSION"),
    contract: CONTRACT_VERSION,
    core: true,
    schema: ID,
    provides: &[Port::ScopePolicy],
    uses: &[],
    scopes: &[],
    problems: &[],
};

/// One scope's row: its class and the roles that hold it by default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScopeRow {
    pub name: &'static str,
    pub class: ScopeClass,
    pub project_lead: bool,
    pub team_lead: bool,
    pub member: bool,
}

/// Why `check_manifests` refused. Scope names only, never data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyError {
    /// A manifest declares a scope this table has no row for.
    UnknownToPolicy { scope: String },
    /// A manifest declares the scope in a different class than the table.
    ClassMismatch { scope: String },
    /// A manifest's default roles differ from the table's.
    RolesMismatch { scope: String },
    /// An `admin` scope whose default roles include `member`.
    AdminGrantsMember { scope: String },
}

/// The `ScopePolicy` provider: stateless.
#[derive(Debug, Default, Clone, Copy)]
pub struct Policy;

impl Policy {
    pub const fn new() -> Policy {
        Policy
    }

    /// The table, in a stable order: 20 scopes (S1 5.3; S3 `x-scopes`).
    pub fn rows() -> &'static [ScopeRow] {
        &ROWS
    }

    /// Checks every declared scope of the enabled manifests against the table and reports ALL the
    /// refusals, in manifest order then declaration order. Errors name scopes only.
    pub fn check_manifests<'a>(
        manifests: impl IntoIterator<Item = &'a Manifest>,
    ) -> Result<(), Vec<PolicyError>> {
        let mut errors = Vec::new();
        for manifest in manifests {
            for decl in manifest.scopes {
                let scope = decl.name.to_owned();
                let Some(row) = ROWS.iter().find(|r| r.name == decl.name) else {
                    errors.push(PolicyError::UnknownToPolicy { scope });
                    continue;
                };
                let holds = |role: DefaultRole| decl.default_roles.contains(&role);
                let declared = (
                    holds(DefaultRole::ProjectLead),
                    holds(DefaultRole::TeamLead),
                    holds(DefaultRole::Member),
                );
                // An admin scope may never be granted to a member: reported INSTEAD of a roles mismatch.
                if decl.class == ScopeClass::Admin && declared.2 {
                    errors.push(PolicyError::AdminGrantsMember { scope });
                    continue;
                }
                if decl.class != row.class {
                    errors.push(PolicyError::ClassMismatch {
                        scope: scope.clone(),
                    });
                }
                if declared != (row.project_lead, row.team_lead, row.member) {
                    errors.push(PolicyError::RolesMismatch { scope });
                }
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

const fn row(
    name: &'static str,
    class: ScopeClass,
    project_lead: bool,
    team_lead: bool,
    member: bool,
) -> ScopeRow {
    ScopeRow {
        name,
        class,
        project_lead,
        team_lead,
        member,
    }
}

/// Section 3.1: the scope each role holds by default. A scope that a role holds here may still be
/// refused by its handler's target rule; that is `target`'s, not this table's.
const ROWS: [ScopeRow; 20] = [
    row("self:read", ScopeClass::SelfScope, true, true, true),
    row("self:revoke", ScopeClass::SelfScope, true, true, true),
    row("roster:read", ScopeClass::Work, true, true, true),
    row("team:create", ScopeClass::Admin, true, false, false),
    row("team:assign", ScopeClass::Admin, true, false, false),
    row("worker:create", ScopeClass::Admin, true, false, false),
    row("worker:retire", ScopeClass::Admin, true, true, false),
    row("token:issue", ScopeClass::Admin, true, true, false),
    row("token:list", ScopeClass::Admin, true, true, false),
    row("token:revoke", ScopeClass::Admin, true, true, false),
    row("audit:read", ScopeClass::Admin, true, false, false),
    row("records:read", ScopeClass::Work, true, true, true),
    row("records:write", ScopeClass::Work, true, true, true),
    row("records:lineage", ScopeClass::Work, true, true, true),
    row("handoff:create", ScopeClass::Work, true, true, true),
    row("handoff:read", ScopeClass::Work, true, true, true),
    row("handoff:claim", ScopeClass::Work, true, true, true),
    row("handoff:hold", ScopeClass::Work, true, true, true),
    row("handoff:review", ScopeClass::Work, true, true, true),
    row("handoff:manage", ScopeClass::Work, true, true, true),
];

impl ScopePolicy for Policy {
    fn allows(&self, p: &Principal, scope: &ScopeName) -> bool {
        // R-12: the kind check comes first; a Service holds no scope, `self:*` included.
        if p.kind != WorkerKind::Worker {
            return false;
        }
        // An exact-name lookup: no wildcard, no inheritance, an unknown name is false.
        let Some(row) = ROWS.iter().find(|r| r.name == scope.as_str()) else {
            return false;
        };
        match p.role {
            Role::ProjectLead => row.project_lead,
            Role::TeamLead(_) => row.team_lead,
            Role::Member(_) => row.member,
        }
    }

    fn target(&self, p: &Principal, rule: TargetRule, t: &Target) -> bool {
        // ONE match, NO wildcard arm: a ninth variant breaks the build here (PO-05, PO-14).
        match rule {
            TargetRule::None => true,
            TargetRule::SelfWorker => t.worker == Some(p.worker),
            TargetRule::SelfOrProjectLead => is_project_lead(p) || t.worker == Some(p.worker),
            TargetRule::NotProjectLead => {
                matches!((t.worker, t.lead), (Some(w), Some(l)) if w != l)
            }
            TargetRule::RetireTarget => may_retire(p, t),
            TargetRule::TokenTarget => may_touch_tokens(p, t),
            TargetRule::Lineage => may_change_lineage(p, t),
            // A consult is open to any principal that passed the scope check (W1-9); a task is the table.
            TargetRule::TaskAddressee => t.consult || may_address_task(p, t),
        }
    }
}

fn is_project_lead(p: &Principal) -> bool {
    matches!(p.role, Role::ProjectLead)
}

/// `worker:retire`. T.worker AND T.lead are needed for EVERY caller (fail closed): never oneself,
/// never the lead; then the project lead passes, a team lead only for a worker of its own team.
fn may_retire(p: &Principal, t: &Target) -> bool {
    let (Some(worker), Some(lead)) = (t.worker, t.lead) else {
        return false;
    };
    if worker == p.worker || worker == lead {
        return false;
    }
    match p.role {
        Role::ProjectLead => true,
        Role::TeamLead(team) => t.team == Some(team),
        Role::Member(_) => false,
    }
}

/// `token:issue`, `token:revoke`, `GET /v2/tokens/{id}`. The project lead: any worker with an id. A
/// team lead: a WORKER (not a service) of its own team. A member: never.
fn may_touch_tokens(p: &Principal, t: &Target) -> bool {
    if t.worker.is_none() {
        return false;
    }
    match p.role {
        Role::ProjectLead => true,
        Role::TeamLead(team) => t.team == Some(team) && t.kind == Some(WorkerKind::Worker),
        Role::Member(_) => false,
    }
}

/// `records:lineage` (supersede, invalidate) and `handoff:manage` (cancel): the project lead; the
/// author (T.worker is the caller); a team lead of the author's team when the author is a worker.
/// An author that is not a worker (T.worker None): the project lead only.
fn may_change_lineage(p: &Principal, t: &Target) -> bool {
    if is_project_lead(p) || t.worker == Some(p.worker) {
        return true;
    }
    match p.role {
        Role::TeamLead(team) => {
            t.worker.is_some() && t.kind == Some(WorkerKind::Worker) && t.team == Some(team)
        }
        Role::ProjectLead | Role::Member(_) => false,
    }
}

/// `handoff:create` for a TASK (a consult never reaches here: `target` answers it first): the
/// project lead, any addressee, a job-role one included; a team lead, a worker addressee of its own team.
fn may_address_task(p: &Principal, t: &Target) -> bool {
    match p.role {
        Role::ProjectLead => true,
        Role::TeamLead(team) => t.worker.is_some() && t.team == Some(team),
        Role::Member(_) => false,
    }
}

/// The piece.
pub struct ModPolicy {
    id: ModuleId,
}

impl ModPolicy {
    pub fn new() -> ModPolicy {
        ModPolicy {
            id: ModuleId::parse(ID).expect("a valid module id"),
        }
    }
}

impl Default for ModPolicy {
    fn default() -> Self {
        Self::new()
    }
}

impl Piece for ModPolicy {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn migrations(&self) -> MigrationSet {
        MigrationSet {
            module: self.id.clone(),
            schema: ID,
            steps: vec![],
        }
    }

    fn provide(&self, ports: &mut PortsBuilder) {
        ports.scope_policy(Arc::new(Policy::new()));
    }

    fn routes(&self, _ports: &Ports) -> Routes {
        Routes::new()
    }

    fn health(&self) -> BoxFuture<'_, Health> {
        Box::pin(async { Health::Ready })
    }
}
