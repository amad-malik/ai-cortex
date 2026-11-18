//! RED for piece 6, wiring (DB-free): the piece assembles with an identity provider and an audit sink, its five routes are declared as
//! S3 r2.5 says (method, path, scope, target rule, body cap), its eleven problem codes are minted once, and it registers a
//! `RecordsApi`. A missing provider stops the assembly.

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use axum::http::Method;
use cortex_db::{MigrationSet, ModuleTx};
use cortex_module::{
    Assembly, AuditDetails, AuditSink, AuditTarget, BoxFuture, CONTRACT_VERSION, Health, Manifest,
    Piece, Port, PortError, PortsBuilder, Registry, Routes, StartupError,
};
use cortex_types::{ModuleId, ProblemCodeMinter};
use mod_records::{ID, ModRecords};
use plug_identity::PlugIdentity;

/// An audit provider that records nothing: only its presence matters here.
struct StubAudit;

struct Sink;

#[async_trait]
impl AuditSink for Sink {
    async fn record(
        &self,
        _tx: &mut ModuleTx,
        _a: &'static str,
        _t: AuditTarget,
        _d: AuditDetails,
    ) -> Result<(), PortError> {
        Ok(())
    }
}

static STUB_MANIFEST: Manifest = Manifest {
    id: "stub_audit",
    version: "0.0.0",
    contract: CONTRACT_VERSION,
    core: false,
    schema: "stub_audit",
    provides: &[Port::Audit],
    uses: &[],
    scopes: &[],
    problems: &[],
};

impl Piece for StubAudit {
    fn manifest(&self) -> &'static Manifest {
        &STUB_MANIFEST
    }
    fn migrations(&self) -> MigrationSet {
        MigrationSet {
            module: ModuleId::parse("stub_audit").expect("id"),
            schema: "stub_audit",
            steps: vec![],
        }
    }
    fn provide(&self, ports: &mut PortsBuilder) {
        ports.audit(Arc::new(Sink));
    }
    fn routes(&self, _ports: &cortex_module::Ports) -> Routes {
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
    vec![
        Box::new(PlugIdentity::new()),
        Box::new(StubAudit),
        Box::new(ModRecords::new()),
    ]
}

#[test]
fn w01_the_piece_assembles_and_registers_a_records_provider() {
    let registry = assemble(full()).expect("assembles");
    assert!(
        registry.ports().records().is_some(),
        "RecordsApi is registered"
    );
    assert!(registry.db_modules().iter().any(|m| m.as_str() == ID));
}

#[test]
fn w02_the_five_routes_are_declared_as_s3_says() {
    let registry = assemble(full()).expect("assembles");
    let routes = registry
        .routes()
        .find(|(id, _)| *id == ID)
        .expect("the piece's routes")
        .1;
    // (method, path, scope, target rule, is a write, body cap): the target rule as its Debug name (TargetRule has no Ord).
    type Row = (String, String, String, String, bool, u64);
    let mut got: Vec<Row> = routes
        .entries()
        .map(|e| {
            (
                e.method.to_string(),
                e.path.to_owned(),
                e.scope.to_owned(),
                format!("{:?}", e.target),
                e.write,
                e.max_body,
            )
        })
        .collect();
    got.sort();
    let row = |m: Method, path: &str, scope: &str, target: &str, write: bool, cap: u64| -> Row {
        (
            m.to_string(),
            path.to_owned(),
            scope.to_owned(),
            target.to_owned(),
            write,
            cap,
        )
    };
    let mut want = vec![
        row(
            Method::POST,
            "/v2/records",
            "records:write",
            "None",
            true,
            9_437_184,
        ),
        row(
            Method::GET,
            "/v2/records/{id}",
            "records:read",
            "None",
            false,
            1_048_576,
        ),
        row(
            Method::POST,
            "/v2/records/{id}/supersede",
            "records:lineage",
            "Lineage",
            true,
            16_384,
        ),
        row(
            Method::POST,
            "/v2/records/{id}/invalidate",
            "records:lineage",
            "Lineage",
            true,
            16_384,
        ),
        row(
            Method::GET,
            "/v2/sections/{name}",
            "records:read",
            "None",
            false,
            1_048_576,
        ),
    ];
    want.sort();
    assert_eq!(got, want);
    assert!(
        routes.entries().all(|e| !e.query),
        "no route takes a query string"
    );
}

#[test]
fn w03_the_eleven_problem_codes_are_minted_once() {
    let registry = assemble(full()).expect("assembles");
    for code in [
        "type_reserved",
        "source_required",
        "source_not_allowed",
        "locator_too_long",
        "why_required",
        "files_required",
        "section_invalid",
        "section_reserved",
        "self_supersede",
        "lineage_state_conflict",
        "reason_required",
    ] {
        assert!(registry.problem(code).is_some(), "{code} is registered");
    }
    // The retired codes are not (R-5).
    for retired in ["affects_unknown", "lineage_target_unknown", "body_not_utf8"] {
        assert!(registry.problem(retired).is_none(), "{retired} is retired");
    }
}

#[test]
fn w04_a_missing_identity_or_audit_provider_stops_the_assembly() {
    assert!(
        assemble(vec![Box::new(StubAudit), Box::new(ModRecords::new())]).is_err(),
        "no IdentityApi"
    );
    assert!(
        assemble(vec![
            Box::new(PlugIdentity::new()),
            Box::new(ModRecords::new())
        ])
        .is_err(),
        "no AuditSink"
    );
}

/// The route table equals the pinned S3 r2.7 extract: method, path, scope, target rule and body cap, for all five operations.
#[test]
fn w05_the_route_table_equals_the_pinned_s3_r26_extract() {
    let s3: serde_json::Value =
        serde_json::from_str(include_str!("data/s3-r2.7-records.1d2c4b2f.json")).expect("extract");
    let registry = assemble(full()).expect("assembles");
    let routes = registry
        .routes()
        .find(|(id, _)| *id == ID)
        .expect("routes")
        .1;
    let mut table: Vec<(String, String, String, String, u64)> = routes
        .entries()
        .map(|e| {
            (
                e.method.to_string(),
                e.path.to_owned(),
                e.scope.to_owned(),
                format!("{:?}", e.target),
                e.max_body,
            )
        })
        .collect();
    table.sort();
    let mut spec: Vec<(String, String, String, String, u64)> = s3["operations"]
        .as_object()
        .expect("operations")
        .values()
        .map(|o| {
            (
                o["method"].as_str().expect("m").to_owned(),
                o["path"].as_str().expect("p").to_owned(),
                o["scope"].as_str().expect("s").to_owned(),
                o["target"].as_str().expect("t").to_owned(),
                // S3 states x-max-body on the routes with a cap of their own; the default is 1 MiB.
                o["max_body"].as_u64().unwrap_or(1_048_576),
            )
        })
        .collect();
    spec.sort();
    assert_eq!(table, spec);
}
