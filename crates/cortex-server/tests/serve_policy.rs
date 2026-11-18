//! The policy STARTUP check in the server's production assembly (piece 11 `serve`, mira; cox 19:33Z: YES). DB-free. RED first.
//! S3 `x-scopes-rule`: "mod-policy REFUSES STARTUP" on a scope it does not know, a wrong class, different default roles, or an admin scope
//! granted to a member. `Registry::assemble` does not make that check and nothing in the server called `Policy::check_manifests`. These cells
//! pin the check and its ONE-line, scope-names-only refusal text. They need the `mod-policy` feature (the policy is only compiled in with it);
//! without it the file is empty, and the feature runs are their own receipts (as the per-feature runs of the assembly cells were).
#![cfg(feature = "mod-policy")]
use std::sync::OnceLock;

use cortex_db::MigrationSet;
use cortex_module::{
    BoxFuture, CONTRACT_VERSION, DefaultRole, Health, Manifest, Piece, Ports, PortsBuilder, Routes,
    ScopeClass, ScopeDecl,
};
use cortex_server::assembly::{assemble_with, check_policy, check_policy_manifests, compiled};
use cortex_server::serve::production_checks;
use cortex_types::{ModuleId, ProblemCodeMinter};

const ALL: &[DefaultRole] = &[
    DefaultRole::ProjectLead,
    DefaultRole::TeamLead,
    DefaultRole::Member,
];

fn manifest(id: &'static str, scopes: &'static [ScopeDecl]) -> Manifest {
    Manifest {
        id,
        version: "0.1.0",
        contract: CONTRACT_VERSION,
        core: false,
        schema: id,
        provides: &[],
        uses: &[],
        scopes,
        problems: &[],
    }
}

// One scope per wrong manifest. The table (S1 5.3): `roster:read` is Work for all three roles; `records:read` is Work for all three;
// `audit:read` is Admin for the project lead only.
static UNKNOWN: &[ScopeDecl] = &[ScopeDecl {
    name: "fx:read",
    class: ScopeClass::Work,
    default_roles: ALL,
}];
static WRONG_CLASS: &[ScopeDecl] = &[ScopeDecl {
    name: "roster:read",
    class: ScopeClass::SelfScope,
    default_roles: ALL,
}];
static WRONG_ROLES: &[ScopeDecl] = &[ScopeDecl {
    name: "records:read",
    class: ScopeClass::Work,
    default_roles: &[DefaultRole::ProjectLead],
}];
static ADMIN_TO_MEMBER: &[ScopeDecl] = &[ScopeDecl {
    name: "audit:read",
    class: ScopeClass::Admin,
    default_roles: &[DefaultRole::ProjectLead, DefaultRole::Member],
}];
static RIGHT: &[ScopeDecl] = &[ScopeDecl {
    name: "roster:read",
    class: ScopeClass::Work,
    default_roles: ALL,
}];

fn one(id: &'static str, scopes: &'static [ScopeDecl]) -> Result<(), String> {
    let m = manifest(id, scopes);
    check_policy_manifests([&m])
}

#[test]
fn sp1_the_real_compiled_set_passes_the_policy_check() {
    let pieces = compiled();
    let manifests: Vec<&Manifest> = pieces.iter().map(|p| p.manifest()).collect();
    // The "real set" is every piece this build compiled in, whatever the feature mix: say which, so the control is not vacuous.
    for (on, id) in [
        (true, "audit"),
        (true, "identity"),
        (true, "tokens"),
        (true, "policy"),
        (cfg!(feature = "mod-records"), "records"),
        (cfg!(feature = "mod-search"), "search"),
        (cfg!(feature = "mod-handoffs"), "handoffs"),
        (cfg!(feature = "mod-boot"), "boot"),
    ] {
        assert_eq!(
            manifests.iter().any(|m| m.id == id),
            on,
            "{id} is in the compiled set exactly when its feature is on"
        );
    }
    assert_eq!(check_policy_manifests(manifests), Ok(()));
    // A manifest that declares a table scope exactly as the table says is fine.
    assert_eq!(one("fx_right", RIGHT), Ok(()));
    assert_eq!(check_policy_manifests(Vec::<&Manifest>::new()), Ok(()));
}

#[test]
fn sp2_each_policy_error_is_one_fixed_line_naming_the_scope() {
    assert_eq!(
        one("fx_unknown", UNKNOWN),
        Err("the policy has no row for scope fx:read".to_owned())
    );
    assert_eq!(
        one("fx_class", WRONG_CLASS),
        Err(
            "scope roster:read is declared in a different class than the policy's table".to_owned()
        )
    );
    assert_eq!(
        one("fx_roles", WRONG_ROLES),
        Err("scope records:read has default roles that differ from the policy's table".to_owned())
    );
    assert_eq!(
        one("fx_admin", ADMIN_TO_MEMBER),
        Err("admin scope audit:read is granted to members".to_owned())
    );
}

#[test]
fn sp3_all_the_refusals_come_on_one_line_in_manifest_order() {
    let a = manifest("fx_unknown", UNKNOWN);
    let ok = manifest("fx_right", RIGHT);
    let b = manifest("fx_roles", WRONG_ROLES);
    let c = manifest("fx_admin", ADMIN_TO_MEMBER);
    let line = check_policy_manifests([&a, &ok, &b, &c]).expect_err("three refusals");
    assert_eq!(
        line,
        "the policy has no row for scope fx:read; scope records:read has default roles that differ from the policy's table; admin scope audit:read is granted to members"
    );
    assert!(!line.contains('\n'));
    // The good manifest in between is not mentioned.
    assert!(!line.contains("roster:read"));
}

fn minter() -> &'static ProblemCodeMinter {
    static M: OnceLock<ProblemCodeMinter> = OnceLock::new();
    M.get_or_init(|| ProblemCodeMinter::take().expect("the first take in this process"))
}

struct Fx(&'static Manifest);

impl Piece for Fx {
    fn manifest(&self) -> &'static Manifest {
        self.0
    }
    fn migrations(&self) -> MigrationSet {
        MigrationSet {
            module: ModuleId::parse(self.0.id).expect("a module id"),
            schema: self.0.schema,
            steps: vec![],
        }
    }
    fn provide(&self, _ports: &mut PortsBuilder) {}
    fn routes(&self, _ports: &Ports) -> Routes {
        Routes::new()
    }
    fn health(&self) -> BoxFuture<'_, Health> {
        Box::pin(async { Health::Ready })
    }
}

fn leak(m: Manifest) -> &'static Manifest {
    Box::leak(Box::new(m))
}

#[test]
fn sp4_the_registry_form_checks_the_pieces_the_registry_enabled() {
    let bad = leak(manifest("fx_unknown", UNKNOWN));
    let good = leak(manifest("fx_right", RIGHT));
    let enabled = |ids: &[&str]| -> Vec<ModuleId> {
        ids.iter()
            .map(|i| ModuleId::parse(i).expect("a module id"))
            .collect()
    };
    // Compiled in AND enabled: refused.
    let reg = assemble_with(
        vec![Box::new(Fx(bad)), Box::new(Fx(good))],
        &enabled(&["fx_unknown", "fx_right"]),
        minter(),
    )
    .expect("assembles");
    assert_eq!(
        check_policy(&reg),
        Err("the policy has no row for scope fx:read".to_owned())
    );
    // Compiled in but NOT enabled: the registry does not hold it, so the check does not see it.
    let reg = assemble_with(
        vec![Box::new(Fx(bad)), Box::new(Fx(good))],
        &enabled(&["fx_right"]),
        minter(),
    )
    .expect("assembles");
    assert_eq!(check_policy(&reg), Ok(()));
}

#[test]
fn sp5_the_production_checks_refuse_with_the_serve_refused_line_and_pass_a_clean_registry() {
    let bad = leak(manifest("fx_unknown", UNKNOWN));
    let good = leak(manifest("fx_right", RIGHT));
    let ids = |v: &[&str]| -> Vec<ModuleId> {
        v.iter()
            .map(|i| ModuleId::parse(i).expect("a module id"))
            .collect()
    };
    let reg =
        assemble_with(vec![Box::new(Fx(bad))], &ids(&["fx_unknown"]), minter()).expect("assembles");
    let refused = production_checks(&reg).expect_err("a scope the policy does not know");
    assert_eq!(
        refused.to_string(),
        "serve refused: the policy has no row for scope fx:read"
    );
    let reg =
        assemble_with(vec![Box::new(Fx(good))], &ids(&["fx_right"]), minter()).expect("assembles");
    assert!(production_checks(&reg).is_ok());
}
