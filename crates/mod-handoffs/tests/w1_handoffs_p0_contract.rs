//! Wave 1, mod-handoffs P0 (row 2a86598d): the piece registers through S0 and declares exactly what S3 r2.5
//! and S2 revision 6 say. DB-free. Test definitions: `mvp/tests/mod-handoffs.md` r3.2, HAND-113 (the
//! migration, the bootstrap role), HAND-114 (the manifest, the scopes, the minted codes) and the route
//! table of S3 r2.5 (nine routes, one scope and one target rule each, the body limits).
//!
//! RED against the stub: the manifest is empty and the piece mounts no route and has no migration.
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use axum::http::Method;
use cortex_db::{MigrationSet, ModuleTx};
use cortex_module::{
    Assembly, AuditDetails, AuditSink, AuditTarget, BoxFuture, CONTRACT_VERSION, DefaultRole,
    Health, Manifest, Piece, Port, PortError, Ports, PortsBuilder, Registry, Routes, ScopeClass,
    ScopeDecl, TargetRule,
};
use cortex_types::{ModuleId, ProblemCodeMinter};
use mod_handoffs::{BOOTSTRAP_SQL, Handoffs, ID, MANIFEST};
use plug_identity::PlugIdentity;
use sha2::{Digest, Sha256};

/// The SHA-256 of the pinned S2 revision 6 `handoffs.sql` (`wave0-partB-pins-20261106-S2r6/S2/handoffs.sql`,
/// prefix `8f7c2ab5`). The migration is that file, byte for byte.
const PINNED_HANDOFFS_SQL_SHA256: &str =
    "8f7c2ab56c98b039621b8975266e2b552bfeb5601e0f39fded442c3a274d853d";

/// S3 r2.5, the handoffs piece: the thirteen codes it mints and their statuses.
const PIECE_CODES: [(&str, u16); 13] = [
    ("already_claimed", 409),
    ("evidence_hash_mismatch", 422),
    ("evidence_required", 422),
    ("evidence_source_invalid", 422),
    ("invalid_transition", 409),
    ("not_holder", 409),
    ("notes_required", 422),
    ("packet_incomplete", 422),
    ("reason_required", 422),
    ("risks_required", 422),
    ("self_review_forbidden", 403),
    ("target_retired", 409),
    ("unknown_addressee", 422),
];

/// S3 r2.5 `x-scopes` for the handoffs piece.
const SCOPES: [&str; 6] = [
    "handoff:claim",
    "handoff:create",
    "handoff:hold",
    "handoff:manage",
    "handoff:read",
    "handoff:review",
];

/// One route of the S3 table: method, path, scope, declared target rule, write, body limit, takes a query.
type Row = (
    &'static str,
    &'static str,
    &'static str,
    TargetRule,
    bool,
    u64,
    bool,
);

const DEFAULT_BODY: u64 = 1024 * 1024;

fn s3_routes() -> Vec<Row> {
    vec![
        (
            "GET",
            "/v2/handoffs",
            "handoff:read",
            TargetRule::None,
            false,
            DEFAULT_BODY,
            true,
        ),
        (
            "GET",
            "/v2/handoffs/{id}",
            "handoff:read",
            TargetRule::None,
            false,
            DEFAULT_BODY,
            false,
        ),
        (
            "POST",
            "/v2/handoffs",
            "handoff:create",
            TargetRule::TaskAddressee,
            true,
            262_144,
            false,
        ),
        (
            "POST",
            "/v2/handoffs/{id}/claim",
            "handoff:claim",
            TargetRule::SelfWorker,
            true,
            16_384,
            false,
        ),
        (
            "POST",
            "/v2/handoffs/{id}/release",
            "handoff:hold",
            TargetRule::None,
            true,
            16_384,
            false,
        ),
        (
            "POST",
            "/v2/handoffs/{id}/return",
            "handoff:hold",
            TargetRule::None,
            true,
            2_097_152,
            false,
        ),
        (
            "POST",
            "/v2/handoffs/{id}/accept",
            "handoff:review",
            TargetRule::SelfOrProjectLead,
            true,
            16_384,
            false,
        ),
        (
            "POST",
            "/v2/handoffs/{id}/rework",
            "handoff:review",
            TargetRule::SelfOrProjectLead,
            true,
            16_384,
            false,
        ),
        (
            "POST",
            "/v2/handoffs/{id}/cancel",
            "handoff:manage",
            TargetRule::Lineage,
            true,
            16_384,
            false,
        ),
    ]
}

/// The audit port has no provider yet (mod-audit is Wave 1): a fixture piece that provides it and records nothing.
struct NoAudit;

#[async_trait]
impl AuditSink for NoAudit {
    async fn record(
        &self,
        _tx: &mut ModuleTx,
        _action: &'static str,
        _target: AuditTarget,
        _details: AuditDetails,
    ) -> Result<(), PortError> {
        Ok(())
    }
}

static AUDIT_FIXTURE: Manifest = Manifest {
    id: "fx_audit",
    version: "0.0.0",
    contract: CONTRACT_VERSION,
    core: false,
    schema: "fx_audit",
    provides: &[Port::Audit],
    uses: &[],
    scopes: &[] as &[ScopeDecl],
    problems: &[],
};

struct AuditFixture;

impl Piece for AuditFixture {
    fn manifest(&self) -> &'static Manifest {
        &AUDIT_FIXTURE
    }
    fn migrations(&self) -> MigrationSet {
        MigrationSet {
            module: ModuleId::parse("fx_audit").expect("id"),
            schema: "fx_audit",
            steps: Vec::new(),
        }
    }
    fn provide(&self, ports: &mut PortsBuilder) {
        ports.audit(Arc::new(NoAudit));
    }
    fn routes(&self, _ports: &Ports) -> Routes {
        Routes::new()
    }
    fn health(&self) -> BoxFuture<'_, Health> {
        Box::pin(async { Health::Ready })
    }
}

/// What one assembly of the handoffs piece shows; built once per process (the minter can be taken once).
struct Assembled {
    routes: Vec<Row>,
    codes: Vec<(String, u16)>,
    provides_handoffs: bool,
}

fn assembled() -> &'static Assembled {
    static A: OnceLock<Assembled> = OnceLock::new();
    A.get_or_init(|| {
        let minter = ProblemCodeMinter::take().expect("the first take in this process");
        let enabled = [
            ModuleId::parse(ID).expect("id"),
            ModuleId::parse(plug_identity::ID).expect("id"),
            ModuleId::parse("fx_audit").expect("id"),
        ];
        let reg = Registry::assemble(Assembly {
            compiled: vec![
                Box::new(Handoffs::new()) as Box<dyn Piece>,
                Box::new(PlugIdentity::new()),
                Box::new(AuditFixture),
            ],
            enabled: &enabled,
            minter: &minter,
            chain_codes: &["internal_error", "unavailable", "busy", "conflict"],
        })
        .expect("the handoffs piece assembles with an identity and an audit provider");
        let mut routes: Vec<Row> = reg
            .routes()
            .filter(|(piece, _)| *piece == ID)
            .flat_map(|(_, r)| {
                r.entries()
                    .map(|e| {
                        (
                            method_name(e.method),
                            e.path,
                            e.scope,
                            e.target,
                            e.write,
                            e.max_body,
                            e.query,
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        routes.sort_by(|a, b| (a.1, a.0).cmp(&(b.1, b.0)));
        let mut codes: Vec<(String, u16)> = PIECE_CODES
            .iter()
            .filter_map(|(c, _)| {
                reg.problem(c)
                    .map(|(_, spec)| (spec.code.to_owned(), spec.status))
            })
            .collect();
        codes.sort();
        Assembled {
            routes,
            codes,
            provides_handoffs: reg.ports().has(Port::Handoffs),
        }
    })
}

fn method_name(m: &Method) -> &'static str {
    match m.as_str() {
        "GET" => "GET",
        "POST" => "POST",
        _ => "OTHER",
    }
}

/// HAND-114: the manifest.
#[test]
fn w1_handoffs_manifest_declares_the_piece_its_ports_and_six_scopes() {
    assert_eq!(MANIFEST.id, "handoffs");
    assert_eq!(MANIFEST.schema, "handoffs");
    assert_eq!(MANIFEST.contract, CONTRACT_VERSION);
    assert!(
        !MANIFEST.core,
        "mod-handoffs can be disabled; boot then lacks sections 4 and 5"
    );
    assert_eq!(MANIFEST.provides, &[Port::Handoffs]);
    let mut uses = MANIFEST.uses.to_vec();
    uses.sort();
    assert_eq!(uses, [Port::Identity, Port::Audit]);

    let mut names: Vec<&str> = MANIFEST.scopes.iter().map(|s| s.name).collect();
    names.sort();
    assert_eq!(names, SCOPES);
    for s in MANIFEST.scopes {
        assert_eq!(s.class, ScopeClass::Work, "{}: a work scope", s.name);
        assert_eq!(
            s.default_roles,
            &[
                DefaultRole::ProjectLead,
                DefaultRole::TeamLead,
                DefaultRole::Member
            ],
            "{}: every role holds it; the target rule narrows it (S3 x-scopes)",
            s.name
        );
    }
}

/// HAND-114: the thirteen codes are declared, each with a title and a fix, and are minted without a clash.
#[test]
fn w1_handoffs_manifest_declares_the_thirteen_codes_and_the_registry_mints_them() {
    let mut declared: Vec<(&str, u16)> = MANIFEST
        .problems
        .iter()
        .map(|p| (p.code, p.status))
        .collect();
    declared.sort();
    assert_eq!(declared, PIECE_CODES);
    for p in MANIFEST.problems {
        assert!(
            !p.title.is_empty() && !p.fix.is_empty(),
            "{}: a title and a fix",
            p.code
        );
    }
    let minted = &assembled().codes;
    assert_eq!(
        minted
            .iter()
            .map(|(c, s)| (c.as_str(), *s))
            .collect::<Vec<_>>(),
        PIECE_CODES,
        "the registry minted all thirteen, none refused as a clash with a platform code"
    );
}

/// S3 r2.5: nine routes, one scope and one declared target rule each, the body limits of `x-max-body`.
#[test]
fn w1_handoffs_mounts_the_nine_routes_of_s3() {
    let got = &assembled().routes;
    let mut want = s3_routes();
    want.sort_by(|a, b| (a.1, a.0).cmp(&(b.1, b.0)));
    assert_eq!(got.len(), 9, "nine routes: {got:?}");
    for (g, w) in got.iter().zip(&want) {
        assert_eq!(g.0, w.0, "method of {}", w.1);
        assert_eq!(g.1, w.1, "path");
        assert_eq!(g.2, w.2, "scope of {} {}", w.0, w.1);
        assert_eq!(g.3, w.3, "declared target rule of {} {}", w.0, w.1);
        assert_eq!(g.4, w.4, "write flag of {} {}", w.0, w.1);
        if w.4 {
            assert_eq!(g.5, w.5, "body limit of {} {}", w.0, w.1);
        }
        assert_eq!(g.6, w.6, "query marker of {} {}", w.0, w.1);
    }
}

#[test]
fn w1_handoffs_provides_the_handoffs_port() {
    assert!(assembled().provides_handoffs);
}

/// HAND-113: the migration is the pinned S2 revision 6 file, byte for byte; its down drops the schema.
#[test]
fn w1_handoffs_migration_is_the_pinned_s2_file() {
    let m = Handoffs::new().migrations();
    assert_eq!((m.schema, m.steps.len()), ("handoffs", 1));
    let step = &m.steps[0];
    assert_eq!((step.version, step.name), (1, "handoffs"));
    let digest = Sha256::digest(step.up.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(
        hex, PINNED_HANDOFFS_SQL_SHA256,
        "the up step is S2's handoffs.sql"
    );
    assert!(step.down.contains("DROP SCHEMA handoffs CASCADE"));
}

/// HAND-113: the bootstrap creates the piece's role and grants it to the application role, idempotently.
#[test]
fn w1_handoffs_bootstrap_creates_the_piece_role() {
    assert!(BOOTSTRAP_SQL.contains("CREATE ROLE cortex_m_handoffs NOLOGIN"));
    assert!(BOOTSTRAP_SQL.contains("GRANT cortex_m_handoffs TO cortex_app"));
    assert!(
        BOOTSTRAP_SQL.contains("duplicate_object") && BOOTSTRAP_SQL.contains("unique_violation"),
        "safe to run twice and when two bootstraps race (the plug's DB run 3 lesson)"
    );
}
