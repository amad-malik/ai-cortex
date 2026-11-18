//! `serve`, part 2: the router over the assembled registry, the listener and the graceful stop (contract r0.4 sections 1 and 2).
//! DB-free (the pool never connects). Fixture pieces stand in for the real ones. RED first.
use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request};
use cortex_db::{DbConfig, DbEndpoint, DbPool, MigrationSet, ModuleRegistry};
use cortex_module::route::AppState;
use cortex_module::{
    AuthError, BoxFuture, CONTRACT_VERSION, DefaultRole, Health, Manifest, Piece, Port, Ports,
    PortsBuilder, Principal, Registry, Role, Routes, ScopeClass, ScopeDecl, ScopePolicy, Target,
    TargetRule, TokenVerifier, WorkerKind,
};
use cortex_server::assembly::assemble_with;
use cortex_server::fault::{WARN_IGNORED, WARN_UNKNOWN};
use cortex_server::health::PieceHealth;
use cortex_server::health_routes::{HealthState, Probe, ProbeSnapshot};
use cortex_server::serve::{self, Refused};
use cortex_types::{
    Clock, ModuleId, ProblemCodeMinter, ProjectId, ScopeName, Secret, SystemClock, Timestamp,
    TokenId, WorkerId,
};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tower::ServiceExt;

fn minter() -> &'static ProblemCodeMinter {
    static M: OnceLock<ProblemCodeMinter> = OnceLock::new();
    M.get_or_init(|| ProblemCodeMinter::take().expect("the first take in this process"))
}

// ---- the chain's ports ------------------------------------------------------------------------

fn v7<T>(s: &str, parse: fn(&str) -> Result<T, cortex_types::IdError>) -> T {
    parse(s).expect("a UUIDv7")
}

struct Verifier;
#[async_trait]
impl TokenVerifier for Verifier {
    async fn verify(&self, bearer: &Secret) -> Result<Principal, AuthError> {
        if bearer.expose() != "ok" {
            return Err(AuthError::Unknown);
        }
        Ok(Principal {
            worker: v7("0193e6a0-0000-7000-8000-000000000401", WorkerId::parse),
            project: v7("0193e6a0-0000-7000-8000-00000000a0a0", ProjectId::parse),
            kind: WorkerKind::Worker,
            role: Role::Member(None),
            token: v7("0193e6a0-0000-7000-8000-0000000004aa", TokenId::parse),
            expires_at: Timestamp::from_unix_micros(4_102_444_799_000_000),
            renew_due: false,
        })
    }
}

struct Policy;
impl ScopePolicy for Policy {
    fn allows(&self, _p: &Principal, scope: &ScopeName) -> bool {
        matches!(scope.as_str(), "fx:read" | "fxo:read")
    }
    fn target(&self, _p: &Principal, rule: TargetRule, _t: &Target) -> bool {
        matches!(rule, TargetRule::None)
    }
}

// ---- fixture pieces ---------------------------------------------------------------------------

static AUTH: Manifest = Manifest {
    id: "fx_auth",
    version: "0.1.0",
    contract: CONTRACT_VERSION,
    core: true,
    schema: "fx_auth",
    provides: &[Port::TokenVerifier, Port::ScopePolicy],
    uses: &[],
    scopes: &[],
    problems: &[],
};
static HALF: Manifest = Manifest {
    id: "fx_half",
    version: "0.1.0",
    contract: CONTRACT_VERSION,
    core: true,
    schema: "fx_half",
    provides: &[Port::TokenVerifier],
    uses: &[],
    scopes: &[],
    problems: &[],
};
static NOAUTH: Manifest = Manifest {
    id: "fx_noauth",
    version: "0.1.0",
    contract: CONTRACT_VERSION,
    core: true,
    schema: "fx_noauth",
    provides: &[],
    uses: &[],
    scopes: &[],
    problems: &[],
};

const ROLES: &[DefaultRole] = &[
    DefaultRole::ProjectLead,
    DefaultRole::TeamLead,
    DefaultRole::Member,
];

static THING: Manifest = Manifest {
    id: "fx_thing",
    version: "0.1.0",
    contract: CONTRACT_VERSION,
    core: false,
    schema: "fx_thing",
    provides: &[],
    uses: &[],
    scopes: &[ScopeDecl {
        name: "fx:read",
        class: ScopeClass::Work,
        default_roles: ROLES,
    }],
    problems: &[],
};
static OTHER: Manifest = Manifest {
    id: "fx_other",
    version: "0.1.0",
    contract: CONTRACT_VERSION,
    core: false,
    schema: "fx_other",
    provides: &[],
    uses: &[],
    scopes: &[ScopeDecl {
        name: "fxo:read",
        class: ScopeClass::Work,
        default_roles: ROLES,
    }],
    problems: &[],
};

#[derive(Clone, Copy)]
enum Kind {
    Auth,
    Half,
    NoAuth,
    Thing,
    Other,
}

struct Fx(Kind);

impl Piece for Fx {
    fn manifest(&self) -> &'static Manifest {
        match self.0 {
            Kind::Auth => &AUTH,
            Kind::Half => &HALF,
            Kind::NoAuth => &NOAUTH,
            Kind::Thing => &THING,
            Kind::Other => &OTHER,
        }
    }
    fn migrations(&self) -> MigrationSet {
        MigrationSet {
            module: ModuleId::parse(self.manifest().id).expect("a module id"),
            schema: self.manifest().schema,
            steps: vec![],
        }
    }
    fn provide(&self, ports: &mut PortsBuilder) {
        match self.0 {
            Kind::Auth => {
                ports.token_verifier(Arc::new(Verifier));
                ports.scope_policy(Arc::new(Policy));
            }
            Kind::Half => ports.token_verifier(Arc::new(Verifier)),
            _ => {}
        }
    }
    fn routes(&self, _ports: &Ports) -> Routes {
        match self.0 {
            Kind::Thing => Routes::new()
                .read(
                    Method::GET,
                    "/v2/fx/thing",
                    "fx:read",
                    TargetRule::None,
                    || async { "thing" },
                )
                .read(
                    Method::GET,
                    "/v2/fx/slow",
                    "fx:read",
                    TargetRule::None,
                    || async {
                        tokio::time::sleep(Duration::from_millis(400)).await;
                        "slow"
                    },
                ),
            Kind::Other => Routes::new().read(
                Method::GET,
                "/v2/fx/other",
                "fxo:read",
                TargetRule::None,
                || async { "other" },
            ),
            _ => Routes::new(),
        }
    }
    fn health(&self) -> BoxFuture<'_, Health> {
        Box::pin(async { Health::Ready })
    }
}

fn piece(k: Kind) -> Box<dyn Piece> {
    Box::new(Fx(k))
}
fn ids(v: &[&str]) -> Vec<ModuleId> {
    v.iter()
        .map(|s| ModuleId::parse(s).expect("a module id"))
        .collect()
}

// ---- the health side --------------------------------------------------------------------------

struct Fake;
#[async_trait]
impl Probe for Fake {
    async fn snapshot(&self) -> ProbeSnapshot {
        ProbeSnapshot {
            reachable: true,
            current: true,
            pieces: vec![PieceHealth {
                name: "fx_thing".to_owned(),
                enabled: true,
                ready: true,
            }],
        }
    }
}
struct Fixed;
impl Clock for Fixed {
    fn now(&self) -> Timestamp {
        Timestamp::from_unix_micros(1_793_923_200 * 1_000_000)
    }
}

fn health() -> Arc<HealthState> {
    Arc::new(HealthState {
        version: "0.2.1".to_owned(),
        probe: Arc::new(Fake),
        last_good: None,
        fault: cortex_server::fault::Fault::None,
        clock: Arc::new(Fixed),
    })
}

fn state_for(registry: &Registry) -> AppState {
    let cfg = DbConfig {
        endpoint: DbEndpoint::Tcp {
            host: "127.0.0.1".into(),
            port: 9,
        },
        database: "none".into(),
        app_user: "none".into(),
        app_password: Secret::new("none"),
        max_connections: 1,
        statement_cache: false,
        registry: ModuleRegistry::new(registry.db_modules()),
        clock: Arc::new(SystemClock),
    };
    AppState {
        pool: Arc::new(DbPool::connect_lazy(&cfg)),
    }
}

/// auth + thing enabled, other compiled but disabled.
fn router() -> Router {
    let mut registry = assemble_with(
        vec![piece(Kind::Auth), piece(Kind::Thing), piece(Kind::Other)],
        &ids(&["fx_auth", "fx_thing"]),
        minter(),
    )
    .expect("assembles");
    let state = state_for(&registry);
    serve::build_router(&mut registry, health(), state).expect("the router builds")
}

async fn get(app: &Router, path: &str, token: Option<&str>) -> (u16, Vec<u8>) {
    let mut b = Request::builder().method(Method::GET).uri(path);
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    let resp = app
        .clone()
        .oneshot(b.body(Body::empty()).expect("a request"))
        .await
        .expect("a response");
    let status = resp.status().as_u16();
    let body = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .expect("a body");
    (status, body.to_vec())
}

// ---- the router -------------------------------------------------------------------------------

#[tokio::test]
async fn rt1_the_token_routes_and_the_open_health_routes_share_one_router() {
    let app = router();
    assert_eq!(
        get(&app, "/v2/fx/thing", None).await.0,
        401,
        "a token route needs a token"
    );
    let (status, body) = get(&app, "/v2/fx/thing", Some("ok")).await;
    assert_eq!((status, body.as_slice()), (200, &b"thing"[..]));
    assert_eq!(get(&app, "/v2/fx/thing", Some("wrong")).await.0, 401);
    for path in ["/v2/health/live", "/v2/health", "/v2/health/ready"] {
        assert_eq!(get(&app, path, None).await.0, 200, "{path} is open");
    }
}

#[tokio::test]
async fn rt2_a_disabled_pieces_paths_are_404_not_found_with_or_without_a_token() {
    let app = router();
    for token in [Some("ok"), None] {
        let (status, body) = get(&app, "/v2/fx/other", token).await;
        assert_eq!(status, 404, "token {token:?}");
        let v: Value = serde_json::from_slice(&body).expect("a problem body");
        assert_eq!(v["type"], "not_found");
    }
}

#[test]
fn rt3_a_registry_without_a_token_verifier_or_a_scope_policy_is_refused_naming_which() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("rt");
    rt.block_on(async {
        for (k, what) in [
            (Kind::NoAuth, "token verifier"),
            (Kind::Half, "scope policy"),
        ] {
            let mut registry = assemble_with(
                vec![piece(k), piece(Kind::Thing)],
                &ids(&[Fx(k).manifest().id, "fx_thing"]),
                minter(),
            )
            .expect("assembles");
            let state = state_for(&registry);
            let e = serve::build_router(&mut registry, health(), state).expect_err("refused");
            assert!(e.0.starts_with("serve refused: "), "{e}");
            assert!(e.0.contains(what), "{e}");
            assert!(!e.0.contains('\n'));
        }
    });
}

// ---- the listener -----------------------------------------------------------------------------

#[tokio::test]
async fn rt4_bind_takes_loopback_or_the_wildcard_and_refuses_a_busy_address_without_naming_it() {
    let loopback = serve::bind("127.0.0.1:0".parse().unwrap())
        .await
        .expect("loopback");
    assert!(loopback.local_addr().unwrap().ip().is_loopback());
    let wildcard = serve::bind("0.0.0.0:0".parse().unwrap())
        .await
        .expect("the container's address");
    assert!(wildcard.local_addr().unwrap().ip().is_unspecified());
    // A busy address.
    let taken = loopback.local_addr().unwrap();
    let e: Refused = serve::bind(taken).await.expect_err("busy");
    assert_eq!(
        e.0,
        "serve refused: could not listen on the configured address"
    );
    assert!(
        !e.0.contains(&taken.port().to_string()),
        "the address is not echoed: {e}"
    );
}

/// A bare HTTP/1.1 GET over a real socket.
async fn raw_get(addr: SocketAddr, path: &str, token: Option<&str>) -> (u16, String) {
    let mut s = TcpStream::connect(addr).await.expect("connect");
    let auth = token
        .map(|t| format!("Authorization: Bearer {t}\r\n"))
        .unwrap_or_default();
    s.write_all(
        format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n{auth}\r\n").as_bytes(),
    )
    .await
    .expect("write");
    let mut out = Vec::new();
    s.read_to_end(&mut out).await.expect("read");
    let text = String::from_utf8_lossy(&out).into_owned();
    let status: u16 = text
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .expect("a status line");
    let body = text.split("\r\n\r\n").nth(1).unwrap_or_default().to_owned();
    (status, body)
}

#[tokio::test]
async fn rt5_a_real_socket_serves_and_stops_gracefully_letting_a_request_in_flight_finish() {
    let listener = serve::bind("127.0.0.1:0".parse().unwrap())
        .await
        .expect("bind");
    let addr = listener.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(serve::run(listener, router(), async move {
        let _ = stopped.await;
    }));

    let (status, body) = raw_get(addr, "/v2/health/live", None).await;
    assert_eq!(status, 200);
    assert!(body.contains(r#""ok":true"#), "{body}");

    // A slow request is under way when the stop comes: it still gets its answer.
    let slow = tokio::spawn(async move { raw_get(addr, "/v2/fx/slow", Some("ok")).await });
    tokio::time::sleep(Duration::from_millis(120)).await;
    stop.send(()).expect("stop");
    let (status, body) = slow.await.expect("the slow request task");
    assert_eq!(status, 200, "the request in flight finished");
    assert!(body.contains("slow"), "{body}");

    let done = tokio::time::timeout(Duration::from_secs(5), server)
        .await
        .expect("run returns after the stop")
        .expect("the server task");
    assert_eq!(done, Ok(()));
    // And nothing listens any more.
    assert!(
        TcpStream::connect(addr).await.is_err(),
        "the listener is gone"
    );
}

// ---- the startup warnings ---------------------------------------------------------------------

#[test]
fn rt6_the_startup_warnings_name_the_problem_and_never_echo_the_value() {
    assert!(serve::startup_warnings(None).is_empty());
    assert!(serve::startup_warnings(Some("")).is_empty());
    let with_feature = cfg!(feature = "test-faults");
    let unknown = serve::startup_warnings(Some("canary-fault-name"));
    assert_eq!(
        unknown,
        [if with_feature {
            WARN_UNKNOWN
        } else {
            WARN_IGNORED
        }]
    );
    let ready = serve::startup_warnings(Some("ready-503"));
    assert_eq!(
        ready.is_empty(),
        with_feature,
        "a known fault with the feature on is silent; without it, ignored"
    );
    for line in unknown.iter().chain(ready.iter()) {
        assert!(
            !line.contains("canary-fault-name") && !line.contains("ready-503"),
            "{line}"
        );
    }
}

#[test]
fn rt7_the_serving_loops_own_failure_has_a_fixed_one_line_text() {
    // An accept error cannot be provoked here, so the wording is pinned on its own.
    let e = serve::stopped_with_error();
    assert_eq!(e.0, "serve refused: the server stopped with an error");
    assert!(!e.0.contains('\n'));
}
