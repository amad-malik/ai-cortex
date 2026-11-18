//! Wave 1 item F (vera HTTP-F1): the exported statuses against the REAL chain. DB-free.
//!
//! `table::export` claims, for each kind of route, the problem statuses the CHAIN ITSELF can answer
//! (the guard's, and the write path's decode, key and transaction steps; not a handler's own refusals,
//! and not the router's 404 and 405 for a path). These cells drive a plain read, a query read and a
//! write through the real router, built with `chain::router`, and compare what comes back with what
//! `export` says, in both directions:
//!
//! - every status a request produced must be in the export's list (nothing the export does not claim);
//! - every status in the export's list must have been produced here, EXCEPT the ones that need a
//!   database, which are named below with the cell that covers them elsewhere.
//!
//! Each probe also asserts the status it was written to produce, so a probe cannot pass by landing on
//! another status of the same set. The pool never connects (port 9), as in `chain.rs`'s own cells: a
//! write that reaches `begin` gets the chain's 503.
//!
//! DB-ONLY for a WRITE, and why:
//! - 409 `already_issued` (the idempotency lookup): `h_db.rs` `h27_a_reply_with_a_secret_is_never_stored_and_a_retry_is_409`
//!   and `d4_reused_and_already_issued_land_after_authorize`.
//! - 429 `busy` (a lock held by another request, `DbError::Busy`; "the only 429 the chain makes today",
//!   `problem.rs`): the mapping and its `Retry-After` are `problem.rs` `h17_busy_is_429_with_retry_after`;
//!   there is NO request-level database cell for it. That is a gap in what is covered, stated here.
use std::collections::BTreeSet;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request};
use bytes::Bytes;
use cortex_db::{DbConfig, DbEndpoint, DbPool, ModuleRegistry};
use cortex_http::chain::{Chain, Limits, LogSink, router};
use cortex_http::request_id::RequestIds;
use cortex_http::table::export;
use cortex_module::route::AppState;
use cortex_module::{
    AuthError, Authorized, Principal, Reply, Role, Routes, ScopePolicy, StrictQuery, Target,
    TargetRule, TokenVerifier, WorkerKind,
};
use cortex_types::{
    ModuleId, ProjectId, ScopeName, Secret, SystemClock, Timestamp, TokenId, WorkerId,
};
use serde::Deserialize;
use serde_json::json;
use tower::ServiceExt;

const PLAIN: &str = "/v2/t/plain";
const LIST: &str = "/v2/t/list";
const DOC: &str = "/v2/t/doc";
/// The route's own body cap in these cells (small, so an oversized request is cheap to make).
const CAP: u64 = 64;
/// A write's statuses that need a database; see the module doc.
const WRITE_DB_ONLY: [u16; 2] = [409, 429];

fn v7<T>(s: &str, parse: fn(&str) -> Result<T, cortex_types::IdError>) -> T {
    parse(s).expect("a UUIDv7")
}

fn worker(n: u8) -> WorkerId {
    v7(
        &format!("0193e6a0-0000-7000-8000-0000000003{n:02}"),
        WorkerId::parse,
    )
}

struct Verifier;

#[async_trait]
impl TokenVerifier for Verifier {
    async fn verify(&self, bearer: &Secret) -> Result<Principal, AuthError> {
        let who = match bearer.expose() {
            "ok" => worker(1),
            "noscope" => worker(2),
            "expired" => return Err(AuthError::Expired),
            "down" => return Err(AuthError::Unavailable),
            "slow" => {
                tokio::time::sleep(Duration::from_secs(5)).await;
                return Err(AuthError::Unavailable);
            }
            "boom" => panic!("a verifier that panics"),
            _ => return Err(AuthError::Unknown),
        };
        Ok(Principal {
            worker: who,
            project: v7("0193e6a0-0000-7000-8000-00000000a0a0", ProjectId::parse),
            kind: WorkerKind::Worker,
            role: Role::Member(None),
            token: v7("0193e6a0-0000-7000-8000-0000000003aa", TokenId::parse),
            expires_at: Timestamp::from_unix_micros(4_102_444_799_000_000),
            renew_due: false,
        })
    }
}

struct Policy;

impl ScopePolicy for Policy {
    fn allows(&self, p: &Principal, scope: &ScopeName) -> bool {
        p.worker != worker(2) && matches!(scope.as_str(), "t:read" | "t:write")
    }
    fn target(&self, _p: &Principal, rule: TargetRule, _t: &Target) -> bool {
        matches!(rule, TargetRule::None)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListQ {
    limit: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Doc {
    #[allow(dead_code)]
    name: String,
}

/// The three routes, built fresh for each use (the router takes them by value).
fn routes() -> Routes {
    Routes::new()
        .read(Method::GET, PLAIN, "t:read", TargetRule::None, || async {
            "ok"
        })
        .max_body(CAP)
        .read(
            Method::GET,
            LIST,
            "t:read",
            TargetRule::None,
            |StrictQuery(q): StrictQuery<ListQ>| async move { q.limit.unwrap_or(0).to_string() },
        )
        .query()
        .max_body(CAP)
        .write(
            Method::POST,
            DOC,
            "t:write",
            TargetRule::None,
            |_tx, ctx, _b: &Doc| Box::pin(async move { ctx.authorize(&Target::default(), ()) }),
            |_tx, _ctx, _a: Authorized<()>, _b: Doc| {
                Box::pin(async move { Ok(Reply::ok(json!({}))) })
            },
        )
        .max_body(CAP)
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
        registry: ModuleRegistry::new([ModuleId::parse("t_probe").expect("a module id")]),
        clock: Arc::new(SystemClock),
    };
    AppState {
        pool: Arc::new(DbPool::connect_lazy(&cfg)),
    }
}

fn app() -> Router {
    let chain = Chain {
        verifier: Arc::new(Verifier),
        policy: Arc::new(Policy),
        log: LogSink::new(|_| {}),
        limits: Limits {
            max_in_flight: 128,
            request_timeout: Duration::from_millis(300),
        },
    };
    router(
        chain,
        dead_state(),
        RequestIds {
            clock: Arc::new(SystemClock),
        },
        vec![("t_probe", routes())],
    )
    .expect("the router builds")
}

/// One request: the token, the idempotency key, the body and a declared length, if any.
struct Probe {
    label: &'static str,
    method: Method,
    uri: String,
    token: Option<&'static str>,
    key: Option<&'static str>,
    body: Option<&'static str>,
    declared: Option<u64>,
    /// A body that fails while it is being read.
    broken_body: bool,
    expect: u16,
}

impl Probe {
    fn new(label: &'static str, method: &Method, uri: &str, expect: u16) -> Probe {
        Probe {
            label,
            method: method.clone(),
            uri: uri.to_owned(),
            token: Some("ok"),
            key: None,
            body: None,
            declared: None,
            broken_body: false,
            expect,
        }
    }
    fn token(mut self, t: Option<&'static str>) -> Probe {
        self.token = t;
        self
    }
    fn key(mut self, k: &'static str) -> Probe {
        self.key = Some(k);
        self
    }
    fn body(mut self, b: &'static str) -> Probe {
        self.body = Some(b);
        self
    }
    fn declared(mut self, n: u64) -> Probe {
        self.declared = Some(n);
        self
    }
    fn broken(mut self) -> Probe {
        self.broken_body = true;
        self
    }

    fn request(&self) -> Request<Body> {
        let mut b = Request::builder()
            .method(self.method.clone())
            .uri(&self.uri);
        if let Some(t) = self.token {
            b = b.header("authorization", format!("Bearer {t}"));
        }
        if let Some(k) = self.key {
            b = b.header("idempotency-key", k);
        }
        if let Some(n) = self.declared {
            b = b.header("content-length", n.to_string());
        }
        let body = if self.broken_body {
            Body::from_stream(futures_util::stream::iter(vec![Err::<Bytes, io::Error>(
                io::Error::other("the connection broke"),
            )]))
        } else {
            Body::from(self.body.unwrap_or(""))
        };
        b.body(body).expect("a request")
    }
}

/// The guard's refusals, the same for every kind of authenticated route.
fn guard_probes(method: &Method, path: &str) -> Vec<Probe> {
    vec![
        Probe::new("no token", method, path, 401).token(None),
        Probe::new("an expired token", method, path, 401).token(Some("expired")),
        Probe::new("a verifier that says unavailable", method, path, 503).token(Some("down")),
        Probe::new(
            "a verifier slower than the request timeout",
            method,
            path,
            503,
        )
        .token(Some("slow")),
        Probe::new("a verifier that panics", method, path, 500).token(Some("boom")),
        Probe::new("a token without the scope", method, path, 403).token(Some("noscope")),
        Probe::new("a declared length over the cap", method, path, 413).declared(CAP + 1000),
        Probe::new("a body that fails while it is read", method, path, 400).broken(),
        Probe::new(
            "a query string the route does not take",
            method,
            &format!("{path}?x=1"),
            422,
        ),
    ]
}

async fn run(app: &Router, probes: Vec<Probe>) -> BTreeSet<u16> {
    let mut seen = BTreeSet::new();
    for p in probes {
        let got = app
            .clone()
            .oneshot(p.request())
            .await
            .expect("a response")
            .status()
            .as_u16();
        assert_eq!(
            got, p.expect,
            "probe `{}` on {} {}",
            p.label, p.method, p.uri
        );
        if got >= 400 {
            seen.insert(got);
        }
    }
    seen
}

/// The export's claimed statuses for the route at `path`.
fn claimed(path: &str) -> BTreeSet<u16> {
    let rows = export(&[("t_probe", &routes())], &[]);
    rows.iter()
        .find(|r| r.path == path)
        .unwrap_or_else(|| panic!("the export has no row for {path}"))
        .statuses
        .iter()
        .copied()
        .collect()
}

/// Both directions, with the database-only statuses named.
fn agree(what: &str, claimed: &BTreeSet<u16>, observed: &BTreeSet<u16>, db_only: &[u16]) {
    let unclaimed: Vec<u16> = observed.difference(claimed).copied().collect();
    assert!(
        unclaimed.is_empty(),
        "{what}: the chain answered {unclaimed:?} and the export does not claim it (export: {claimed:?}, observed: {observed:?})"
    );
    let db_only: BTreeSet<u16> = db_only.iter().copied().collect();
    let unproven: Vec<u16> = claimed
        .difference(observed)
        .copied()
        .filter(|s| !db_only.contains(s))
        .collect();
    assert!(
        unproven.is_empty(),
        "{what}: the export claims {unproven:?} and no DB-free request produced it (export: {claimed:?}, observed: {observed:?})"
    );
    let missing: Vec<u16> = db_only.difference(claimed).copied().collect();
    assert!(
        missing.is_empty(),
        "{what}: the database-only statuses {missing:?} are not in the export"
    );
}

#[tokio::test]
async fn fc1_a_plain_read_answers_what_the_export_claims() {
    let app = app();
    let mut probes = guard_probes(&Method::GET, PLAIN);
    probes.push(Probe::new("the success control", &Method::GET, PLAIN, 200));
    let seen = run(&app, probes).await;
    agree("a plain read", &claimed(PLAIN), &seen, &[]);
}

#[tokio::test]
async fn fc2_a_query_read_answers_what_the_export_claims() {
    let app = app();
    let mut probes = guard_probes(&Method::GET, LIST);
    probes.push(Probe::new(
        "the success control",
        &Method::GET,
        &format!("{LIST}?limit=1"),
        200,
    ));
    probes.push(Probe::new(
        "a query parameter the route did not declare",
        &Method::GET,
        &format!("{LIST}?bogus=1"),
        422,
    ));
    let seen = run(&app, probes).await;
    agree("a query read", &claimed(LIST), &seen, &[]);
}

#[tokio::test]
async fn fc3_a_write_answers_what_the_export_claims() {
    let app = app();
    let m = Method::POST;
    let mut probes = guard_probes(&m, DOC);
    probes.extend([
        Probe::new("a body that is not JSON", &m, DOC, 400)
            .key("0193e6a0-key-a")
            .body("{"),
        Probe::new("a body of the wrong shape", &m, DOC, 422)
            .key("0193e6a0-key-a")
            .body("{}"),
        Probe::new("a valid body and no key", &m, DOC, 428).body(r#"{"name":"x"}"#),
        Probe::new("a key outside the grammar", &m, DOC, 400)
            .key("short")
            .body(r#"{"name":"x"}"#),
        Probe::new("everything valid, no database", &m, DOC, 503)
            .key("0193e6a0-key-a")
            .body(r#"{"name":"x"}"#),
    ]);
    let seen = run(&app, probes).await;
    agree("a write", &claimed(DOC), &seen, &WRITE_DB_ONLY);
}
