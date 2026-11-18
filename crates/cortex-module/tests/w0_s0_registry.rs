//! Wave 0 S0: the registry's startup checks, the port wiring and piece health (socket contract
//! r4 sections 1.1 to 1.5, with the r5 changes). DB-free: no piece here touches a database.
//!
//! Every test double fails closed: each port method refuses (`Unavailable`, or `false`).
//!
//! RED: `Registry::assemble` and `Registry::health` are `todo!()`.
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::http::Method;
use cortex_db::{MigrationSet, ModuleTx};
use cortex_module::{
    Assembly, AuditDetails, AuditSink, AuditTarget, AuthError, BoxFuture, Ctx, DefaultRole,
    HandoffView, HandoffsApi, Health, IdentityApi, Manifest, Piece, PieceHealth, Port, PortError,
    Ports, PortsBuilder, Principal, Problem, ProblemSpec, ProjectView, RecordCursor, RecordPage,
    RecordView, RecordsApi, Registry, Reply, Role, Routes, ScopeClass, ScopeDecl, ScopePolicy,
    SectionName, SectionView, StartupError, Status, Target, TargetRule, TokenVerifier, WorkerKind,
    WorkerView,
};
use cortex_types::{
    ModuleId, ProblemCodeMinter, ProjectId, RecordId, RequestId, ScopeName, Secret, SystemClock,
    TeamId, Timestamp, TokenId, WorkerId,
};

/// The process's one minter, shared by every test in this binary.
fn minter() -> &'static ProblemCodeMinter {
    static M: OnceLock<ProblemCodeMinter> = OnceLock::new();
    M.get_or_init(|| ProblemCodeMinter::take().expect("the first take in this process"))
}

const CHAIN_CODES: &[&str] = &["internal_error", "unavailable", "busy", "conflict"];

/// A double for all six ports: every call refuses.
struct Refuses;

#[async_trait]
impl TokenVerifier for Refuses {
    async fn verify(&self, _bearer: &Secret) -> Result<Principal, AuthError> {
        Err(AuthError::Unavailable)
    }
}
impl ScopePolicy for Refuses {
    fn allows(&self, _p: &Principal, _scope: &ScopeName) -> bool {
        false
    }
    fn target(&self, _p: &Principal, _rule: TargetRule, _t: &Target) -> bool {
        false
    }
}
#[async_trait]
impl AuditSink for Refuses {
    async fn record(
        &self,
        _tx: &mut ModuleTx,
        _action: &'static str,
        _target: AuditTarget,
        _details: AuditDetails,
    ) -> Result<(), PortError> {
        Err(PortError::Unavailable)
    }
}
#[async_trait]
impl IdentityApi for Refuses {
    async fn project(&self, _tx: &mut ModuleTx) -> Result<ProjectView, PortError> {
        Err(PortError::Unavailable)
    }
    async fn worker(
        &self,
        _tx: &mut ModuleTx,
        _w: &WorkerId,
    ) -> Result<Option<WorkerView>, PortError> {
        Err(PortError::Unavailable)
    }
    async fn roster(&self, _tx: &mut ModuleTx) -> Result<Vec<WorkerView>, PortError> {
        Err(PortError::Unavailable)
    }
}
#[async_trait]
impl RecordsApi for Refuses {
    async fn get(
        &self,
        _tx: &mut ModuleTx,
        _ids: &[RecordId],
    ) -> Result<Vec<RecordView>, PortError> {
        Err(PortError::Unavailable)
    }
    async fn changed_since(
        &self,
        _tx: &mut ModuleTx,
        _after: Option<RecordCursor>,
        _limit: u32,
    ) -> Result<RecordPage, PortError> {
        Err(PortError::Unavailable)
    }
    async fn section_current(
        &self,
        _tx: &mut ModuleTx,
        _name: &SectionName,
    ) -> Result<Option<SectionView>, PortError> {
        Err(PortError::Unavailable)
    }
}
#[async_trait]
impl HandoffsApi for Refuses {
    async fn claimed_by(
        &self,
        _tx: &mut ModuleTx,
        _w: &WorkerId,
        _limit: u32,
    ) -> Result<Vec<HandoffView>, PortError> {
        Err(PortError::Unavailable)
    }
    async fn pending_for(
        &self,
        _tx: &mut ModuleTx,
        _w: &WorkerId,
        _limit: u32,
    ) -> Result<Vec<HandoffView>, PortError> {
        Err(PortError::Unavailable)
    }
}

/// A manifest made at run time and leaked, so each test can vary one key.
#[derive(Clone)]
struct M {
    id: &'static str,
    schema: &'static str,
    core: bool,
    contract: u32,
    provides: Vec<Port>,
    uses: Vec<Port>,
    scopes: Vec<&'static str>,
    problems: Vec<&'static str>,
    /// Whole specs, for the cells that vary one field of a shared code (W1-31).
    specs: Vec<ProblemSpec>,
}

impl M {
    fn new(id: &'static str) -> M {
        M {
            id,
            schema: id,
            core: false,
            contract: 1,
            provides: vec![],
            uses: vec![],
            scopes: vec![],
            problems: vec![],
            specs: vec![],
        }
    }
    fn leak(self) -> &'static Manifest {
        let scopes: Vec<ScopeDecl> = self
            .scopes
            .iter()
            .map(|name| ScopeDecl {
                name,
                class: ScopeClass::Work,
                default_roles: &[DefaultRole::Member],
            })
            .collect();
        let problems: Vec<ProblemSpec> = self
            .problems
            .iter()
            .map(|code| ProblemSpec {
                code,
                status: 409,
                title: "Refused",
                fix: "Try again later.",
            })
            .chain(self.specs.iter().copied())
            .collect();
        Box::leak(Box::new(Manifest {
            id: self.id,
            version: "0.0.1",
            contract: self.contract,
            core: self.core,
            schema: self.schema,
            provides: Box::leak(self.provides.into_boxed_slice()),
            uses: Box::leak(self.uses.into_boxed_slice()),
            scopes: Box::leak(scopes.into_boxed_slice()),
            problems: Box::leak(problems.into_boxed_slice()),
        }))
    }
}

/// A route: method, path, scope, write.
type R = (Method, &'static str, &'static str, bool);

/// A test piece. It provides the ports in `provide` (whatever its manifest says), mounts
/// `routes`, and records which ports its `routes` saw.
struct TestPiece {
    m: &'static Manifest,
    provide: Vec<Port>,
    routes: Vec<R>,
    health: Health,
    hang: bool,
    provided_calls: Arc<AtomicUsize>,
    saw: Arc<Mutex<Option<Vec<Port>>>>,
}

impl TestPiece {
    fn new(m: M) -> TestPiece {
        let provide = m.provides.clone();
        TestPiece {
            m: m.leak(),
            provide,
            routes: vec![],
            health: Health::Ready,
            hang: false,
            provided_calls: Arc::new(AtomicUsize::new(0)),
            saw: Arc::new(Mutex::new(None)),
        }
    }
    fn providing(mut self, ports: &[Port]) -> TestPiece {
        self.provide = ports.to_vec();
        self
    }
    fn route(mut self, r: R) -> TestPiece {
        self.routes.push(r);
        self
    }
    fn boxed(self) -> Box<dyn Piece> {
        Box::new(self)
    }
}

async fn read_handler() -> &'static str {
    "ok"
}

impl Piece for TestPiece {
    fn manifest(&self) -> &'static Manifest {
        self.m
    }
    fn migrations(&self) -> MigrationSet {
        MigrationSet {
            module: ModuleId::parse(self.m.id).expect("a valid id in this test"),
            schema: self.m.schema,
            steps: vec![],
        }
    }
    fn provide(&self, ports: &mut PortsBuilder) {
        self.provided_calls.fetch_add(1, Ordering::SeqCst);
        for p in &self.provide {
            match p {
                Port::Identity => ports.identity(Arc::new(Refuses)),
                Port::Records => ports.records(Arc::new(Refuses)),
                Port::Handoffs => ports.handoffs(Arc::new(Refuses)),
                Port::TokenVerifier => ports.token_verifier(Arc::new(Refuses)),
                Port::ScopePolicy => ports.scope_policy(Arc::new(Refuses)),
                Port::Audit => ports.audit(Arc::new(Refuses)),
            }
        }
    }
    fn routes(&self, ports: &Ports) -> Routes {
        *self.saw.lock().expect("lock") = Some(
            Port::ALL
                .iter()
                .copied()
                .filter(|p| ports.has(*p))
                .collect(),
        );
        let mut r = Routes::new();
        for (method, path, scope, write) in &self.routes {
            r = if *write {
                // The write bound is a boxed future (mira's route.rs; r4's AsyncFn cannot be Send).
                r.write_open(
                    method.clone(),
                    path,
                    scope,
                    |_tx, _ctx, _b: serde_json::Value| {
                        Box::pin(async move {
                            Ok::<_, Problem>(Reply {
                                status: Status::NoContent,
                                body: (),
                                issued: None,
                            })
                        })
                    },
                )
            } else {
                r.read(method.clone(), path, scope, TargetRule::None, read_handler)
            };
        }
        r
    }
    fn health(&self) -> BoxFuture<'_, Health> {
        let (h, hang) = (self.health, self.hang);
        Box::pin(async move {
            if hang {
                std::future::pending::<()>().await;
            }
            h
        })
    }
}

fn ids(v: &[&str]) -> Vec<ModuleId> {
    v.iter()
        .map(|s| ModuleId::parse(s).expect("a valid id"))
        .collect()
}

fn assemble(
    compiled: Vec<Box<dyn Piece>>,
    enabled: &[&str],
) -> Result<Registry, Vec<StartupError>> {
    let enabled = ids(enabled);
    Registry::assemble(Assembly {
        compiled,
        enabled: &enabled,
        minter: minter(),
        chain_codes: CHAIN_CODES,
    })
}

fn errors(r: Result<Registry, Vec<StartupError>>) -> Vec<StartupError> {
    match r {
        Ok(reg) => panic!("want startup errors, got {reg:?}"),
        Err(e) => e,
    }
}

/// The plug's two pieces: identity provides `IdentityApi` and mounts the roster; the consumer
/// uses it and mounts one read and one write.
fn identity() -> TestPiece {
    let mut m = M::new("identity");
    m.core = true;
    m.provides = vec![Port::Identity];
    m.scopes = vec!["identity:read"];
    TestPiece::new(m).route((Method::GET, "/v2/identity/roster", "identity:read", false))
}

fn consumer() -> TestPiece {
    let mut m = M::new("consumer");
    m.uses = vec![Port::Identity];
    m.scopes = vec!["consumer:read", "consumer:write"];
    m.problems = vec!["consumer_full"];
    TestPiece::new(m)
        .route((Method::GET, "/v2/consumer/thing", "consumer:read", false))
        .route((Method::POST, "/v2/consumer/thing", "consumer:write", true))
}

#[test]
fn w0_s0_two_pieces_assemble_and_each_sees_only_the_ports_it_uses() {
    let (id, co) = (identity(), consumer());
    let (id_saw, co_saw) = (id.saw.clone(), co.saw.clone());
    let reg = assemble(vec![id.boxed(), co.boxed()], &["identity", "consumer"])
        .expect("the plug's two pieces assemble");
    let order: Vec<&str> = reg.pieces().map(|p| p.manifest().id).collect();
    assert_eq!(
        order,
        ["identity", "consumer"],
        "migration order is the compiled order"
    );
    assert!(reg.ports().has(Port::Identity));
    assert_eq!(*co_saw.lock().expect("lock"), Some(vec![Port::Identity]));
    assert_eq!(
        *id_saw.lock().expect("lock"),
        Some(vec![]),
        "a piece that uses nothing sees no port"
    );
    let routes: Vec<(&str, usize)> = reg
        .routes()
        .map(|(p, r)| (p, r.entries().count()))
        .collect();
    assert_eq!(routes, [("identity", 1), ("consumer", 2)]);
    let (t, spec) = reg
        .problem("consumer_full")
        .expect("the piece's code is minted");
    assert_eq!((t.code(), spec.status), ("consumer_full", 409));
    assert!(reg.problem("not_declared").is_none());
}

#[test]
fn w0_s0_a_disabled_piece_is_not_wired() {
    let mut m = M::new("extra");
    m.provides = vec![Port::Records];
    m.scopes = vec!["extra:read"];
    let extra = TestPiece::new(m).route((Method::GET, "/v2/extra", "extra:read", false));
    let (calls, saw) = (extra.provided_calls.clone(), extra.saw.clone());
    let reg = assemble(
        vec![identity().boxed(), consumer().boxed(), extra.boxed()],
        &["identity", "consumer"],
    )
    .expect("assembles without the disabled piece");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        0,
        "a disabled piece provides nothing"
    );
    assert!(saw.lock().expect("lock").is_none(), "and mounts no route");
    assert!(!reg.ports().has(Port::Records));
    assert_eq!(reg.pieces().count(), 2);
    // Control: enabled, it is wired.
    let reg = assemble(
        vec![identity().boxed(), consumer().boxed(), {
            let mut m = M::new("extra");
            m.provides = vec![Port::Records];
            TestPiece::new(m).boxed()
        }],
        &["identity", "consumer", "extra"],
    )
    .expect("assembles with it");
    assert!(reg.ports().has(Port::Records));
}

#[test]
fn w0_s0_contract_version_ids_and_the_enabled_list() {
    let mut m = M::new("consumer");
    m.contract = 2;
    let e = errors(assemble(
        vec![identity().boxed(), TestPiece::new(m).boxed()],
        &["identity", "consumer"],
    ));
    assert!(
        e.contains(&StartupError::UnsupportedContract {
            piece: "consumer".into()
        }),
        "{e:?}"
    );

    let e = errors(assemble(
        vec![identity().boxed(), TestPiece::new(M::new("Bad-Id")).boxed()],
        &["identity"],
    ));
    assert!(
        e.contains(&StartupError::BadPieceId {
            piece: "Bad-Id".into()
        }),
        "{e:?}"
    );

    let e = errors(assemble(
        vec![identity().boxed(), identity().boxed()],
        &["identity"],
    ));
    assert!(
        e.contains(&StartupError::DuplicatePiece {
            piece: "identity".into()
        }),
        "{e:?}"
    );

    let e = errors(assemble(vec![identity().boxed()], &["identity", "ghost"]));
    assert_eq!(
        e,
        [StartupError::UnknownPiece {
            piece: "ghost".into()
        }]
    );
}

#[test]
fn w0_s0_a_core_piece_cannot_be_disabled() {
    let e = errors(assemble(
        vec![identity().boxed(), consumer().boxed()],
        &["consumer"],
    ));
    assert!(
        e.contains(&StartupError::CoreDisabled {
            piece: "identity".into()
        }),
        "{e:?}"
    );
    // Control: a non-core piece may be left out.
    assemble(vec![identity().boxed(), consumer().boxed()], &["identity"])
        .expect("the consumer is not core");
}

#[test]
fn w0_s0_a_schema_is_owned_once_and_platform_is_taken() {
    let mut m = M::new("consumer");
    m.schema = "identity";
    let e = errors(assemble(
        vec![identity().boxed(), TestPiece::new(m).boxed()],
        &["identity", "consumer"],
    ));
    assert!(
        e.contains(&StartupError::SchemaOwnedTwice {
            schema: "identity".into()
        }),
        "{e:?}"
    );

    let mut m = M::new("consumer");
    m.schema = "platform";
    let e = errors(assemble(
        vec![identity().boxed(), TestPiece::new(m).boxed()],
        &["identity", "consumer"],
    ));
    assert!(
        e.contains(&StartupError::SchemaOwnedTwice {
            schema: "platform".into()
        }),
        "{e:?}"
    );
}

#[test]
fn w0_s0_ports_are_provided_once_by_the_piece_that_declares_them() {
    // Used, but no enabled piece provides it (the provider is compiled and disabled).
    let mut idm = M::new("identity");
    idm.provides = vec![Port::Identity];
    idm.scopes = vec!["identity:read"];
    let e = errors(assemble(
        vec![TestPiece::new(idm).boxed(), consumer().boxed()],
        &["consumer"],
    ));
    assert!(
        e.contains(&StartupError::MissingPort {
            piece: "consumer".into(),
            port: Port::Identity
        }),
        "{e:?}"
    );

    // Two providers.
    let mut two = M::new("identity_two");
    two.provides = vec![Port::Identity];
    let e = errors(assemble(
        vec![identity().boxed(), TestPiece::new(two).boxed()],
        &["identity", "identity_two"],
    ));
    assert!(
        e.contains(&StartupError::PortProvidedTwice {
            port: Port::Identity
        }),
        "{e:?}"
    );

    // Declared but not provided.
    let mut m = M::new("records");
    m.provides = vec![Port::Records];
    let e = errors(assemble(
        vec![identity().boxed(), TestPiece::new(m).providing(&[]).boxed()],
        &["identity", "records"],
    ));
    assert!(
        e.contains(&StartupError::ProvidesMismatch {
            piece: "records".into(),
            port: Port::Records
        }),
        "{e:?}"
    );

    // Provided but not declared.
    let e = errors(assemble(
        vec![
            identity().boxed(),
            TestPiece::new(M::new("sneaky"))
                .providing(&[Port::Audit])
                .boxed(),
        ],
        &["identity", "sneaky"],
    ));
    assert!(
        e.contains(&StartupError::ProvidesMismatch {
            piece: "sneaky".into(),
            port: Port::Audit
        }),
        "{e:?}"
    );
}

#[test]
fn w0_s0_routes_are_not_asked_for_when_the_wiring_fails() {
    let co = consumer();
    let saw = co.saw.clone();
    let mut idm = M::new("identity");
    idm.provides = vec![Port::Identity];
    let e = errors(assemble(
        vec![TestPiece::new(idm).boxed(), co.boxed()],
        &["consumer"],
    ));
    assert!(!e.is_empty());
    assert!(
        saw.lock().expect("lock").is_none(),
        "routes() ran with a port missing"
    );
}

#[test]
fn w0_s0_scope_names_are_valid_and_declared_once() {
    let mut m = M::new("consumer");
    m.scopes = vec!["Consumer:Read"];
    let e = errors(assemble(
        vec![identity().boxed(), TestPiece::new(m).boxed()],
        &["identity", "consumer"],
    ));
    assert!(
        e.contains(&StartupError::BadScope {
            scope: "Consumer:Read".into()
        }),
        "{e:?}"
    );

    let mut m = M::new("consumer");
    m.scopes = vec!["identity:read"];
    let e = errors(assemble(
        vec![identity().boxed(), TestPiece::new(m).boxed()],
        &["identity", "consumer"],
    ));
    assert!(
        e.contains(&StartupError::ScopeDefinedTwice {
            scope: "identity:read".into()
        }),
        "{e:?}"
    );
}

#[test]
fn w0_s0_problem_codes_are_minted_once() {
    let piece = |id: &'static str, code: &'static str| {
        let mut m = M::new(id);
        m.problems = vec![code];
        TestPiece::new(m).boxed()
    };
    // W1-31 (cox): the SAME code from two pieces with an IDENTICAL spec is accepted now (it was
    // `ProblemCodeTwice` in Wave 0); the cells for it are `w1_s0_shared_problem_codes_*` below. A piece
    // that declares a code twice ITSELF is still refused.
    let mut m = M::new("aaa");
    m.problems = vec!["same_code", "same_code"];
    let e = errors(assemble(
        vec![identity().boxed(), TestPiece::new(m).boxed()],
        &["identity", "aaa"],
    ));
    assert!(
        e.contains(&StartupError::ProblemCodeTwice {
            code: "same_code".into()
        }),
        "a piece declaring a code twice: {e:?}"
    );

    let e = errors(assemble(
        vec![identity().boxed(), piece("aaa", "not_found")],
        &["identity", "aaa"],
    ));
    assert!(
        e.contains(&StartupError::ProblemCodeTwice {
            code: "not_found".into()
        }),
        "a platform code: {e:?}"
    );

    let e = errors(assemble(
        vec![identity().boxed(), piece("aaa", "busy")],
        &["identity", "aaa"],
    ));
    assert!(
        e.contains(&StartupError::ProblemCodeTwice {
            code: "busy".into()
        }),
        "a chain code: {e:?}"
    );

    let e = errors(assemble(
        vec![identity().boxed(), piece("aaa", "Bad Code")],
        &["identity", "aaa"],
    ));
    assert!(
        e.contains(&StartupError::BadProblemCode {
            code: "Bad Code".into()
        }),
        "{e:?}"
    );
}

// ---- W1-31: a problem code several pieces share (S3's text, copied exactly) ------------------------------

fn spec(status: u16, title: &'static str, fix: &'static str) -> ProblemSpec {
    ProblemSpec {
        code: "reason_required",
        status,
        title,
        fix,
    }
}

/// Two pieces, `aaa` and `bbb`, each declaring `reason_required` with the given specs.
fn two_pieces(a: ProblemSpec, b: ProblemSpec) -> Result<Registry, Vec<StartupError>> {
    let (mut ma, mut mb) = (M::new("aaa"), M::new("bbb"));
    ma.specs = vec![a];
    mb.specs = vec![b];
    assemble(
        vec![
            identity().boxed(),
            TestPiece::new(ma).boxed(),
            TestPiece::new(mb).boxed(),
        ],
        &["identity", "aaa", "bbb"],
    )
}

#[test]
fn w1_s0_shared_problem_codes_identical_declarations_are_accepted() {
    let s = spec(
        422,
        "A reason is required",
        "Send a reason of 1 to 512 bytes.",
    );
    let reg = two_pieces(s, s).expect("an identical shared code is accepted");
    let (_, got) = reg.problem("reason_required").expect("the code is minted");
    assert_eq!(got, s, "the one spec both pieces declared");
    // Minted once: the registry holds the code a single time, not once per declaring piece.
    assert_eq!(
        reg.problem_codes()
            .iter()
            .filter(|c| **c == "reason_required")
            .count(),
        1
    );
}

#[test]
fn w1_s0_shared_problem_codes_a_different_status_title_or_fix_is_refused() {
    let base = spec(
        422,
        "A reason is required",
        "Send a reason of 1 to 512 bytes.",
    );
    for (what, other) in [
        ("status", spec(409, base.title, base.fix)),
        ("title", spec(422, "Reason needed", base.fix)),
        ("fix", spec(422, base.title, "Say why.")),
    ] {
        let e = two_pieces(base, other)
            .err()
            .unwrap_or_else(|| panic!("a different {what} was accepted"));
        assert!(
            e.contains(&StartupError::ProblemCodeDiffers {
                code: "reason_required".into(),
                differs: what
            }),
            "{what}: {e:?}"
        );
        // And it is refused in either order.
        let e = two_pieces(other, base)
            .err()
            .unwrap_or_else(|| panic!("a different {what} was accepted"));
        assert!(
            e.contains(&StartupError::ProblemCodeDiffers {
                code: "reason_required".into(),
                differs: what
            }),
            "{what} reversed: {e:?}"
        );
    }
}

/// vera's W1P2-V1: a piece declaring a code twice ITSELF is refused whichever piece declared it first. The first
/// GREEN checked a repeat only against the first global owner, so A:[X] then B:[X, X] accepted B's second X while
/// B:[X, X] then A:[X] refused it.
#[test]
fn w1_s0_shared_problem_codes_a_piece_repeating_a_code_is_refused_in_either_order() {
    let x = spec(
        422,
        "A reason is required",
        "Send a reason of 1 to 512 bytes.",
    );
    let piece = |id: &'static str, specs: Vec<ProblemSpec>| {
        let mut m = M::new(id);
        m.specs = specs;
        TestPiece::new(m).boxed()
    };
    for (what, pieces) in [
        (
            "A:[X] then B:[X, X]",
            vec![piece("aaa", vec![x]), piece("bbb", vec![x, x])],
        ),
        (
            "B:[X, X] then A:[X]",
            vec![piece("bbb", vec![x, x]), piece("aaa", vec![x])],
        ),
        ("A:[X, X] alone", vec![piece("aaa", vec![x, x])]),
        (
            "A:[X] B:[X] then C:[X, X]",
            vec![
                piece("aaa", vec![x]),
                piece("bbb", vec![x]),
                piece("ccc", vec![x, x]),
            ],
        ),
    ] {
        let enabled: Vec<&str> = std::iter::once("identity")
            .chain(pieces.iter().map(|p| p.manifest().id))
            .collect();
        let mut compiled = vec![identity().boxed()];
        compiled.extend(pieces);
        let e = errors(assemble(compiled, &enabled));
        assert!(
            e.contains(&StartupError::ProblemCodeTwice {
                code: "reason_required".into()
            }),
            "{what}: {e:?}"
        );
    }
}

#[test]
fn w1_s0_shared_problem_codes_a_platform_or_chain_code_is_still_refused_even_when_identical() {
    let platform = ProblemSpec {
        code: "not_found",
        status: 404,
        title: "Not found",
        fix: "x",
    };
    let (mut ma, mut mb) = (M::new("aaa"), M::new("bbb"));
    ma.specs = vec![platform];
    mb.specs = vec![platform];
    let e = errors(assemble(
        vec![
            identity().boxed(),
            TestPiece::new(ma).boxed(),
            TestPiece::new(mb).boxed(),
        ],
        &["identity", "aaa", "bbb"],
    ));
    assert!(
        e.contains(&StartupError::ProblemCodeTwice {
            code: "not_found".into()
        }),
        "{e:?}"
    );
}

#[test]
fn w0_s0_the_router_walk_refuses_unknown_scopes_and_twice_mounted_routes() {
    let mut m = M::new("consumer");
    m.scopes = vec!["consumer:read"];
    let p = TestPiece::new(m).route((Method::GET, "/v2/consumer/x", "consumer:nothing", false));
    let e = errors(assemble(
        vec![identity().boxed(), p.boxed()],
        &["identity", "consumer"],
    ));
    assert_eq!(
        e,
        [StartupError::UnknownScope {
            method: "GET".into(),
            path: "/v2/consumer/x".into(),
            scope: "consumer:nothing".into()
        }]
    );

    let mut m = M::new("consumer");
    m.scopes = vec!["consumer:read"];
    let p = TestPiece::new(m).route((Method::GET, "/v2/identity/roster", "consumer:read", false));
    let e = errors(assemble(
        vec![identity().boxed(), p.boxed()],
        &["identity", "consumer"],
    ));
    assert_eq!(
        e,
        [StartupError::RouteTwice {
            method: "GET".into(),
            path: "/v2/identity/roster".into()
        }]
    );

    // Control: one path, two methods.
    let mut m = M::new("consumer");
    m.scopes = vec!["consumer:read"];
    let p = TestPiece::new(m).route((Method::POST, "/v2/identity/roster", "consumer:read", true));
    assemble(
        vec![identity().boxed(), p.boxed()],
        &["identity", "consumer"],
    )
    .expect("another method on the same path mounts");
}

#[test]
fn w0_s0_every_error_is_reported_at_once() {
    let mut a = M::new("aaa");
    a.contract = 9;
    let mut b = M::new("bbb");
    b.scopes = vec!["NoGood"];
    let e = errors(assemble(
        vec![
            identity().boxed(),
            TestPiece::new(a).boxed(),
            TestPiece::new(b).boxed(),
        ],
        &["identity", "aaa", "bbb"],
    ));
    assert!(
        e.contains(&StartupError::UnsupportedContract {
            piece: "aaa".into()
        }),
        "{e:?}"
    );
    assert!(
        e.contains(&StartupError::BadScope {
            scope: "NoGood".into()
        }),
        "{e:?}"
    );
}

#[tokio::test]
async fn w0_s0_piece_health_is_bounded_at_two_seconds() {
    let mut slow = TestPiece::new(M::new("slow"));
    slow.hang = true;
    let mut sick = TestPiece::new(M::new("sick"));
    sick.health = Health::NotReady("index rebuilding");
    let reg = assemble(
        vec![identity().boxed(), slow.boxed(), sick.boxed()],
        &["identity", "slow", "sick"],
    )
    .expect("assembles");
    let t0 = Instant::now();
    let h = reg.health().await;
    let took = t0.elapsed();
    assert_eq!(
        h,
        [
            PieceHealth {
                piece: "identity",
                health: Health::Ready
            },
            PieceHealth {
                piece: "slow",
                health: Health::NotReady("health check timed out")
            },
            PieceHealth {
                piece: "sick",
                health: Health::NotReady("index rebuilding")
            },
        ]
    );
    assert!(
        took >= Duration::from_secs(2) && took < Duration::from_secs(5),
        "{took:?}"
    );
}

#[test]
fn w0_s0_the_ctx_project_is_the_tokens() {
    let clock = SystemClock;
    let p = Principal {
        worker: WorkerId::new(&clock),
        project: ProjectId::new(&clock),
        kind: WorkerKind::Worker,
        role: Role::TeamLead(TeamId::new(&clock)),
        token: TokenId::new(&clock),
        expires_at: Timestamp::from_unix_micros(0),
        renew_due: false,
    };
    let ctx = Ctx::new(p.clone(), RequestId::new(&clock));
    assert_eq!(ctx.project(), p.project);
    assert_eq!(ctx.principal(), &p);
}

#[test]
fn w0_s0_section_names_round_trip() {
    let w = WorkerId::new(&SystemClock);
    for s in [
        "orientation".to_owned(),
        "rules-project".to_owned(),
        format!("rules-worker:{w}"),
    ] {
        let n = SectionName::parse(&s).expect("a section name");
        assert_eq!(n.to_string(), s);
    }
    for bad in [
        "",
        "rules-worker:",
        "rules-worker:not-an-id",
        "Orientation",
        "rules",
    ] {
        assert!(SectionName::parse(bad).is_none(), "{bad:?}");
    }
}
