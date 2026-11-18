//! Wave 0 S0: `Registry::db_modules()`, the module list `cortex-db`'s `DbConfig.registry` is built
//! from (cox's row 17863602; piece 11 contract s4b: never a hand-built list). It is `platform`
//! (the idempotency helper delegates to it) followed by every ENABLED piece, in migration order.
//! Run 5 (mira's w0_plug at f7a450b) failed every idempotent write because a hand-built list
//! left `platform` out. DB-free.
//!
//! RED: `db_modules` is `todo!()`.
use std::sync::OnceLock;

use cortex_db::{MigrationSet, ModuleRegistry};
use cortex_module::{Assembly, BoxFuture, Health, Manifest, Piece, Ports, Registry, Routes};
use cortex_types::{ModuleId, ProblemCodeMinter};

fn minter() -> &'static ProblemCodeMinter {
    static M: OnceLock<ProblemCodeMinter> = OnceLock::new();
    M.get_or_init(|| ProblemCodeMinter::take().expect("the first take in this process"))
}

struct P(&'static Manifest);

impl Piece for P {
    fn manifest(&self) -> &'static Manifest {
        self.0
    }
    fn migrations(&self) -> MigrationSet {
        MigrationSet {
            module: ModuleId::parse(self.0.id).expect("id"),
            schema: self.0.schema,
            steps: vec![],
        }
    }
    fn routes(&self, _ports: &Ports) -> Routes {
        Routes::new()
    }
    fn health(&self) -> BoxFuture<'_, Health> {
        Box::pin(async { Health::Ready })
    }
}

const fn manifest(id: &'static str) -> Manifest {
    Manifest {
        id,
        version: "0.0.1",
        contract: 1,
        core: false,
        schema: id,
        provides: &[],
        uses: &[],
        scopes: &[],
        problems: &[],
    }
}

static ALPHA: Manifest = manifest("alpha");
static BETA: Manifest = manifest("beta");
static GAMMA: Manifest = manifest("gamma");

fn ids(v: &[&str]) -> Vec<ModuleId> {
    v.iter().map(|s| ModuleId::parse(s).expect("id")).collect()
}

#[test]
fn w0_s0_db_modules_is_platform_then_the_enabled_pieces_in_migration_order() {
    let enabled = ids(&["gamma", "alpha"]);
    let reg = Registry::assemble(Assembly {
        compiled: vec![Box::new(P(&ALPHA)), Box::new(P(&BETA)), Box::new(P(&GAMMA))],
        enabled: &enabled,
        minter: minter(),
        chain_codes: &[],
    })
    .expect("assembles");
    let m = reg.db_modules();
    assert_eq!(m, ids(&["platform", "alpha", "gamma"]), "beta is disabled");
    // It builds the pool's registry directly, and the registry knows platform.
    let r = ModuleRegistry::new(m);
    assert!(r.contains(&ModuleId::parse("platform").expect("id")));
    assert!(!r.contains(&ModuleId::parse("beta").expect("id")));
}

#[test]
fn w0_s0_db_modules_has_platform_even_with_no_piece() {
    let reg = Registry::assemble(Assembly {
        compiled: vec![Box::new(P(&ALPHA))],
        enabled: &[],
        minter: minter(),
        chain_codes: &[],
    })
    .expect("assembles with nothing enabled");
    assert_eq!(reg.db_modules(), ids(&["platform"]));
}
