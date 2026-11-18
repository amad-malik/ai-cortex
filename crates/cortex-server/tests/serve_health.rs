//! `serve`, part 1: the three OPEN health routes behind the real chain (contract r0.4 section 2; S3 r2.7 `/v2/health`,
//! `/live`, `/ready`). DB-free: the database side is a fake [`Probe`]; the pool never connects. RED first.
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{HeaderMap, Method, Request};
use cortex_db::{DbConfig, DbEndpoint, DbPool, ModuleRegistry};
use cortex_http::chain::{Chain, Limits, LogSink, router_with_open};
use cortex_http::request_id::RequestIds;
use cortex_module::route::AppState;
use cortex_module::{AuthError, Principal, ScopePolicy, Target, TargetRule, TokenVerifier};
use cortex_server::fault::Fault;
use cortex_server::health::PieceHealth;
use cortex_server::health_routes::{HealthState, Probe, ProbeSnapshot, routes};
use cortex_types::{Clock, ModuleId, ScopeName, Secret, SystemClock, Timestamp};
use serde_json::{Value, json};
use tower::ServiceExt;

// 2026-11-06T00:00:00Z, the clock every cell reads.
const T0: i64 = 1_793_923_200;

struct Fixed(Timestamp);
impl Clock for Fixed {
    fn now(&self) -> Timestamp {
        self.0
    }
}

/// The database side, set per cell.
#[derive(Clone)]
struct Fake {
    db: bool,
    migrations: bool,
    pieces: Vec<PieceHealth>,
}

impl Fake {
    fn ready() -> Fake {
        Fake {
            db: true,
            migrations: true,
            pieces: vec![piece("identity", true, true), piece("records", true, true)],
        }
    }
}

fn piece(name: &str, enabled: bool, ready: bool) -> PieceHealth {
    PieceHealth {
        name: name.to_owned(),
        enabled,
        ready,
    }
}

#[async_trait]
impl Probe for Fake {
    async fn snapshot(&self) -> ProbeSnapshot {
        ProbeSnapshot {
            reachable: self.db,
            current: self.migrations,
            pieces: self.pieces.clone(),
        }
    }
}

/// The chain's ports: these routes never reach them (that is the point of an open route).
struct Verifier;
#[async_trait]
impl TokenVerifier for Verifier {
    async fn verify(&self, _bearer: &Secret) -> Result<Principal, AuthError> {
        Err(AuthError::Unknown)
    }
}
struct Policy;
impl ScopePolicy for Policy {
    fn allows(&self, _p: &Principal, _scope: &ScopeName) -> bool {
        false
    }
    fn target(&self, _p: &Principal, _rule: TargetRule, _t: &Target) -> bool {
        false
    }
}

fn dead_state() -> AppState {
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
        registry: ModuleRegistry::new([ModuleId::parse("platform").expect("a module id")]),
        clock: Arc::new(SystemClock),
    };
    AppState {
        pool: Arc::new(DbPool::connect_lazy(&cfg)),
    }
}

fn app_with(fake: Fake, fault: Fault, last_good: Option<PathBuf>) -> Router {
    let state = Arc::new(HealthState {
        version: "0.2.1".to_owned(),
        probe: Arc::new(fake),
        last_good,
        fault,
        clock: Arc::new(Fixed(Timestamp::from_unix_micros(T0 * 1_000_000))),
    });
    let chain = Chain {
        verifier: Arc::new(Verifier),
        policy: Arc::new(Policy),
        log: LogSink::new(|_| {}),
        limits: Limits::default(),
    };
    router_with_open(
        chain,
        dead_state(),
        RequestIds {
            clock: Arc::new(SystemClock),
        },
        Vec::new(),
        routes(state),
    )
    .expect("the router builds")
}

fn app(fake: Fake) -> Router {
    app_with(fake, Fault::None, None)
}

struct Answer {
    status: u16,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl Answer {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|e| panic!("not JSON ({e}): {}", String::from_utf8_lossy(&self.body)))
    }
}

async fn send(app: &Router, method: Method, path: &str, auth: Option<&str>) -> Answer {
    let mut b = Request::builder().method(method).uri(path);
    if let Some(a) = auth {
        b = b.header("authorization", a);
    }
    let resp = app
        .clone()
        .oneshot(b.body(Body::empty()).expect("a request"))
        .await
        .expect("a response");
    let (parts, body) = resp.into_parts();
    let body = axum::body::to_bytes(body, 1 << 20).await.expect("a body");
    Answer {
        status: parts.status.as_u16(),
        headers: parts.headers,
        body: body.to_vec(),
    }
}

async fn get(app: &Router, path: &str) -> Answer {
    send(app, Method::GET, path, None).await
}

const LIVE: &str = "/v2/health/live";
const HEALTH: &str = "/v2/health";
const READY: &str = "/v2/health/ready";

// ---- /live --------------------------------------------------------------------------------

#[tokio::test]
async fn sh1_live_is_200_ok_with_no_token_whatever_the_database_says() {
    let down = Fake {
        db: false,
        migrations: false,
        pieces: vec![piece("identity", true, false)],
    };
    let a = get(&app(down), LIVE).await;
    assert_eq!(a.status, 200);
    assert_eq!(a.json(), json!({"ok": true}));
    assert_eq!(
        a.headers.get("content-type").and_then(|v| v.to_str().ok()),
        Some("application/json")
    );
}

// ---- /v2/health ---------------------------------------------------------------------------

#[tokio::test]
async fn sh2_health_is_the_open_document_with_exactly_the_s3_fields() {
    let a = get(&app(Fake::ready()), HEALTH).await;
    assert_eq!(a.status, 200);
    let v = a.json();
    let mut keys: Vec<&str> = v
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "backup",
            "last_good_dump_at",
            "pieces",
            "ready",
            "state",
            "version"
        ],
        "S3 Health is a closed object"
    );
    assert_eq!(v["state"], "ready");
    assert_eq!(v["ready"], true);
    assert_eq!(v["version"], "0.2.1");
    assert_eq!(v["backup"], "none");
    assert_eq!(v["last_good_dump_at"], Value::Null);
    assert_eq!(
        v["pieces"],
        json!([
            {"name": "identity", "enabled": true, "ready": true},
            {"name": "records", "enabled": true, "ready": true}
        ])
    );
}

#[tokio::test]
async fn sh3_each_reason_for_not_ready_shows_in_the_document_and_in_ready() {
    let reasons: [(&str, Fake); 3] = [
        (
            "the database is unreachable",
            Fake {
                db: false,
                ..Fake::ready()
            },
        ),
        (
            "a migration is not current",
            Fake {
                migrations: false,
                ..Fake::ready()
            },
        ),
        (
            "an enabled piece is not ready",
            Fake {
                pieces: vec![piece("identity", true, true), piece("records", true, false)],
                ..Fake::ready()
            },
        ),
    ];
    for (why, fake) in reasons {
        let a = app(fake);
        let doc = get(&a, HEALTH).await;
        assert_eq!(doc.status, 200, "{why}: the document still answers");
        assert_eq!(doc.json()["state"], "not_ready", "{why}");
        assert_eq!(doc.json()["ready"], false, "{why}");
        assert_eq!(get(&a, READY).await.status, 503, "{why}");
        assert_eq!(get(&a, LIVE).await.status, 200, "{why}: live does not care");
    }
}

#[tokio::test]
async fn sh4_a_disabled_piece_is_listed_and_never_blocks() {
    let fake = Fake {
        pieces: vec![
            piece("identity", true, true),
            piece("records", false, false),
        ],
        ..Fake::ready()
    };
    let a = app(fake);
    let doc = get(&a, HEALTH).await.json();
    assert_eq!(doc["ready"], true);
    assert_eq!(
        doc["pieces"][1],
        json!({"name": "records", "enabled": false, "ready": false})
    );
    assert_eq!(get(&a, READY).await.status, 200);
}

// ---- /ready -------------------------------------------------------------------------------

#[tokio::test]
async fn sh5_ready_is_200_ok_when_ready_and_a_fixed_503_problem_when_not() {
    let ok = get(&app(Fake::ready()), READY).await;
    assert_eq!(ok.status, 200);
    assert_eq!(ok.json(), json!({"ok": true}));

    let not = get(
        &app(Fake {
            db: false,
            ..Fake::ready()
        }),
        READY,
    )
    .await;
    assert_eq!(not.status, 503);
    assert_eq!(
        not.headers
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("application/problem+json")
    );
    assert_eq!(
        not.headers
            .get("cache-control")
            .and_then(|v| v.to_str().ok()),
        Some("no-store")
    );
    assert!(not.headers.contains_key("cortex-request-id"));
    let v = not.json();
    assert_eq!(v["type"], "capability_unavailable");
    assert_eq!(v["status"], 503);
    assert!(v["request_id"].is_string());
    // The body is the same whichever thing is wrong: the route is open, so it says nothing about WHY.
    let other = get(
        &app(Fake {
            migrations: false,
            ..Fake::ready()
        }),
        READY,
    )
    .await
    .json();
    for k in ["type", "title", "status", "detail", "fix"] {
        assert_eq!(v[k], other[k], "{k} must not depend on the reason");
    }
}

#[tokio::test]
async fn sh6_the_test_fault_makes_ready_503_and_nothing_else() {
    let a = app_with(Fake::ready(), Fault::Ready503, None);
    let ready = get(&a, READY).await;
    assert_eq!(ready.status, 503);
    assert_eq!(ready.json()["type"], "capability_unavailable");
    assert_eq!(get(&a, LIVE).await.status, 200);
    let doc = get(&a, HEALTH).await.json();
    assert_eq!(
        doc["ready"], true,
        "the document is not changed by the fault"
    );
    // And with no fault the same state is 200 (the control).
    assert_eq!(get(&app(Fake::ready()), READY).await.status, 200);
}

// ---- no token, whatever it says -----------------------------------------------------------

#[tokio::test]
async fn sh7_the_authorization_header_is_never_read_on_an_open_route() {
    let a = app(Fake::ready());
    for path in [LIVE, HEALTH, READY] {
        for auth in [None, Some("Bearer junk"), Some("Basic nonsense"), Some("")] {
            let r = send(&a, Method::GET, path, auth).await;
            assert_eq!(r.status, 200, "{path} with {auth:?}");
        }
    }
}

// ---- the method check ---------------------------------------------------------------------

#[tokio::test]
async fn sh8_the_wrong_method_is_405_with_allow_and_an_unknown_path_is_404() {
    let a = app(Fake::ready());
    for path in [LIVE, HEALTH, READY] {
        for m in [Method::POST, Method::PUT, Method::DELETE, Method::PATCH] {
            let r = send(&a, m.clone(), path, None).await;
            assert_eq!(r.status, 405, "{m} {path}");
            let allow = r
                .headers
                .get("allow")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_else(|| panic!("{m} {path}: no Allow header"));
            assert!(allow.contains("GET"), "{m} {path}: Allow = {allow}");
            assert_eq!(r.json()["type"], "invalid_request", "{m} {path}");
        }
    }
    let nf = get(&a, "/v2/health/nothing").await;
    assert_eq!(nf.status, 404);
    assert_eq!(nf.json()["type"], "not_found");
}

// ---- the backup state comes through the route ---------------------------------------------

fn last_good(name: &str, utc: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("sh-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).expect("dir");
    let f = d.join("LAST_GOOD");
    fs::write(
        &f,
        format!(r#"{{"set":"s1","utc":"{utc}","bytes":10,"sha256":"00"}}"#),
    )
    .expect("file");
    f
}

#[tokio::test]
async fn sh9_the_last_good_file_decides_backup_through_the_route() {
    // One hour before the fixed clock: ok, and the time is reported.
    let fresh = last_good("fresh", "2026-11-05T23:00:00Z");
    let doc = get(&app_with(Fake::ready(), Fault::None, Some(fresh)), HEALTH)
        .await
        .json();
    assert_eq!(doc["backup"], "ok");
    assert_eq!(doc["last_good_dump_at"], "2026-11-05T23:00:00Z");
    // Two days before: stale. It never makes the server not ready.
    let old = last_good("old", "2026-11-04T00:00:00Z");
    let doc = get(&app_with(Fake::ready(), Fault::None, Some(old)), HEALTH)
        .await
        .json();
    assert_eq!(doc["backup"], "stale");
    assert_eq!(doc["ready"], true);
    // A path with no file behind it: none.
    let gone = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("sh-gone")
        .join("LAST_GOOD");
    let doc = get(&app_with(Fake::ready(), Fault::None, Some(gone)), HEALTH)
        .await
        .json();
    assert_eq!(doc["backup"], "none");
    assert_eq!(doc["last_good_dump_at"], Value::Null);
}

// ---- the declared table ---------------------------------------------------------------------

/// `OpenRoute.method` is metadata the router does not use (the handler's own `get` decides what is allowed), but the route table
/// export (X-07, item F) reads it. So it is checked on its own: exactly the three GET routes, no more.
#[test]
fn sh10_the_declared_open_routes_are_exactly_the_three_get_paths() {
    let state = Arc::new(HealthState {
        version: "0.2.1".to_owned(),
        probe: Arc::new(Fake::ready()),
        last_good: None,
        fault: Fault::None,
        clock: Arc::new(Fixed(Timestamp::from_unix_micros(T0 * 1_000_000))),
    });
    let mut declared: Vec<(String, &str)> = routes(state)
        .iter()
        .map(|r| (r.method.to_string(), r.path))
        .collect();
    declared.sort();
    assert_eq!(
        declared,
        [
            ("GET".to_owned(), "/v2/health"),
            ("GET".to_owned(), "/v2/health/live"),
            ("GET".to_owned(), "/v2/health/ready"),
        ]
    );
}
