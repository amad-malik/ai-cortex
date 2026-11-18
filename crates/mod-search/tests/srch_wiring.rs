//! RED for piece 7, wiring (DB-free): the piece assembles with the scope it reads (declared by the records piece, which this crate may not
//! depend on: a stub declares it), its one route is declared as S3 says, it owns no migration step, it uses and provides no port, and its
//! problem code is minted once.

use std::sync::OnceLock;

use axum::http::Method;
use cortex_db::MigrationSet;
use cortex_module::{
    Assembly, BoxFuture, CONTRACT_VERSION, DefaultRole, Health, Manifest, Piece, Ports, Registry,
    Routes, ScopeClass, ScopeDecl, StartupError, TargetRule,
};
use cortex_types::{ModuleId, ProblemCodeMinter};
use mod_search::{ID, MAX_BODY, PATH, SCOPE, Search};

/// Stands in for the records piece: it declares the scope `records:read` that search's route needs.
struct StubRecords;

static STUB: Manifest = Manifest {
    id: "stub_records",
    version: "0.0.0",
    contract: CONTRACT_VERSION,
    core: false,
    schema: "stub_records",
    provides: &[],
    uses: &[],
    scopes: &[ScopeDecl {
        name: "records:read",
        class: ScopeClass::Work,
        default_roles: &[DefaultRole::Member],
    }],
    problems: &[],
};

impl Piece for StubRecords {
    fn manifest(&self) -> &'static Manifest {
        &STUB
    }
    fn migrations(&self) -> MigrationSet {
        MigrationSet {
            module: ModuleId::parse("stub_records").expect("id"),
            schema: "stub_records",
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

fn minter() -> &'static ProblemCodeMinter {
    static M: OnceLock<ProblemCodeMinter> = OnceLock::new();
    M.get_or_init(|| ProblemCodeMinter::take().expect("the first take in this process"))
}

const CHAIN_CODES: &[&str] = &["internal_error", "unavailable", "busy", "conflict"];

fn assemble(compiled: Vec<Box<dyn Piece>>) -> Result<Registry, Vec<StartupError>> {
    let enabled: Vec<ModuleId> = compiled
        .iter()
        .map(|p| ModuleId::parse(p.manifest().id).expect("id"))
        .collect();
    Registry::assemble(Assembly {
        compiled,
        enabled: &enabled,
        minter: minter(),
        chain_codes: CHAIN_CODES,
    })
}

fn full() -> Vec<Box<dyn Piece>> {
    vec![Box::new(StubRecords), Box::new(Search::new())]
}

#[test]
fn w01_the_piece_assembles_and_owns_no_table() {
    let registry = assemble(full()).expect("assembles");
    assert!(
        registry.ports().records().is_none(),
        "search provides no port"
    );
    let m = Search::new().migrations();
    assert_eq!(
        (m.schema, m.steps.len()),
        (ID, 0),
        "an empty schema with no step"
    );
}

#[test]
fn w02_the_one_route_is_declared_as_s3_says() {
    let registry = assemble(full()).expect("assembles");
    let routes = registry
        .routes()
        .find(|(id, _)| *id == ID)
        .expect("the piece's routes")
        .1;
    let rows: Vec<_> = routes
        .entries()
        .map(|e| {
            (
                e.method.clone(),
                e.path,
                e.scope,
                format!("{:?}", e.target),
                e.write,
                e.max_body,
                e.query,
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![(
            Method::POST,
            PATH,
            SCOPE,
            format!("{:?}", TargetRule::None),
            false,
            MAX_BODY,
            false
        )],
        "POST /v2/search is a READ with a body: not a write (no Idempotency-Key), 16384 bytes, no query string"
    );
}

#[test]
fn w03_the_problem_code_is_minted_once_and_the_scope_must_be_declared_by_someone() {
    let registry = assemble(full()).expect("assembles");
    assert!(registry.db_modules().iter().any(|m| m.as_str() == ID));
    // Without the piece that declares records:read the route's scope is unknown: the assembly refuses.
    let only: Vec<Box<dyn Piece>> = vec![Box::new(Search::new())];
    assert!(assemble(only).is_err(), "an undeclared scope stops startup");
}

#[test]
fn w04_the_piece_reports_ready() {
    let r = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    assert_eq!(r.block_on(Search::new().health()), Health::Ready);
}

#[test]
fn w05_a_disabled_piece_has_no_route_so_its_path_is_404_not_503() {
    // MS-18(a): with search not enabled the registry mounts none of its routes (the chain answers 404 not_found for an unknown path;
    // readiness reports a disabled piece only on GET /v2/health/ready). The scope's owner stays enabled.
    let compiled = full();
    let enabled = vec![ModuleId::parse("stub_records").expect("id")];
    let registry = Registry::assemble(Assembly {
        compiled,
        enabled: &enabled,
        minter: minter(),
        chain_codes: CHAIN_CODES,
    })
    .expect("assembles");
    assert!(
        registry.routes().all(|(id, _)| id != ID),
        "no search route is mounted"
    );
    assert!(
        !registry.db_modules().iter().any(|m| m.as_str() == ID),
        "and search gets no database role"
    );
}
