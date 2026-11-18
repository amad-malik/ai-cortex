//! The real chain (cortex-http) in front of mod-tokens, an identity fixture and an audit fixture. The verifier IS
//! mod-tokens' own (the registry's `TokenVerifier` port); the policy is a double that grants the S3 default roles
//! and answers `TokenTarget` from sophi's truth table, and records every `(caller, rule, target)` it was asked.
#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::Request;
use axum::http::{HeaderMap, Method, StatusCode};
use cortex_db::ModuleRegistry;
use cortex_http::chain::{Chain, Limits, LogSink, router};
use cortex_http::request_id::RequestIds;
use cortex_module::route::AppState;
use cortex_module::{
    Assembly, AuthError, Piece, Principal, Registry, Role, ScopePolicy, Target, TargetRule,
    TokenVerifier, WorkerKind,
};
use cortex_types::{
    ModuleId, ProblemCodeMinter, ScopeName, SystemClock, TestClock, Timestamp, TokenId, WorkerId,
};
use mod_tokens::{Entropy, ModTokens};
use tower::ServiceExt;

use super::audit::{AuditPiece, Fault};
use super::db::{NOW, Seed, bootstrap, insert_token};
use super::ident::IdentityFixture;

/// Every `(caller, rule, target)` the handlers asked the policy.
pub type Asked = Arc<Mutex<Vec<(WorkerId, TargetRule, Target)>>>;

struct Policy(Asked);

impl ScopePolicy for Policy {
    fn allows(&self, p: &Principal, scope: &ScopeName) -> bool {
        if p.kind != WorkerKind::Worker {
            return false;
        }
        match scope.as_str() {
            "self:read" | "self:revoke" => true,
            "token:issue" | "token:list" | "token:revoke" => {
                matches!(p.role, Role::ProjectLead | Role::TeamLead(_))
            }
            _ => false,
        }
    }

    fn target(&self, p: &Principal, rule: TargetRule, t: &Target) -> bool {
        self.0
            .lock()
            .expect("asked lock")
            .push((p.worker, rule, t.clone()));
        match rule {
            // sophi's table 3.2: the lead when the target worker is known; a team lead when the target worker is
            // a worker of its own team; nobody else.
            TargetRule::TokenTarget => {
                t.worker.is_some()
                    && match p.role {
                        Role::ProjectLead => true,
                        Role::TeamLead(team) => {
                            t.team == Some(team) && t.kind == Some(WorkerKind::Worker)
                        }
                        Role::Member(_) => false,
                    }
            }
            _ => true,
        }
    }
}

fn minter() -> &'static ProblemCodeMinter {
    static M: OnceLock<ProblemCodeMinter> = OnceLock::new();
    M.get_or_init(|| ProblemCodeMinter::take().expect("the first take in this process"))
}

const CHAIN_CODES: &[&str] = &[
    cortex_http::problem::INTERNAL_ERROR,
    cortex_http::problem::UNAVAILABLE,
    cortex_http::problem::BUSY,
    cortex_http::problem::CONFLICT,
];

/// A pause between the chain's verification and the handler (the schedule of vera's TOKENS-V1): armed for one
/// worker's NEXT verified request, it tells the test that the verifier has answered, then holds the request until
/// the test releases it. A demotion committed during the pause is the "changed membership" the handler must catch.
pub struct Gate {
    armed: Mutex<Option<WorkerId>>,
    reached: tokio::sync::Semaphore,
    release: tokio::sync::Semaphore,
}

impl Gate {
    pub fn new() -> Gate {
        Gate {
            armed: Mutex::new(None),
            reached: tokio::sync::Semaphore::new(0),
            release: tokio::sync::Semaphore::new(0),
        }
    }

    /// Hold the next request whose principal is `who` after it is verified.
    pub fn arm(&self, who: WorkerId) {
        *self.armed.lock().expect("gate lock") = Some(who);
    }

    /// Wait until a request has been verified and is held.
    pub async fn wait_reached(&self) {
        self.reached
            .acquire()
            .await
            .expect("the gate stays open")
            .forget();
    }

    /// Let the held request go on.
    pub fn release(&self) {
        self.release.add_permits(1);
    }
}

/// The real verifier, with the [`Gate`] after it.
struct GatedVerifier {
    inner: Arc<dyn TokenVerifier>,
    gate: Arc<Gate>,
}

#[async_trait]
impl TokenVerifier for GatedVerifier {
    async fn verify(&self, bearer: &cortex_types::Secret) -> Result<Principal, AuthError> {
        let principal = self.inner.verify(bearer).await?;
        let hold = {
            let mut armed = self.gate.armed.lock().expect("gate lock");
            if *armed == Some(principal.worker) {
                armed.take();
                true
            } else {
                false
            }
        };
        if hold {
            self.gate.reached.add_permits(1);
            self.gate
                .release
                .acquire()
                .await
                .expect("the gate stays open")
                .forget();
        }
        Ok(principal)
    }
}

pub struct World {
    pub db: cortex_testkit::TestDb,
    pub seed: Seed,
    pub pool: Arc<cortex_db::DbPool>,
    pub app: Router,
    pub clock: Arc<TestClock>,
    pub fault: Arc<Fault>,
    pub asked: Asked,
    /// The chain's log lines, in order.
    pub logs: Arc<Mutex<Vec<String>>>,
    /// Pauses a verified request before its handler.
    pub gate: Arc<Gate>,
    pub identity: Arc<dyn cortex_module::IdentityApi>,
    pub audit_sink: Arc<dyn cortex_module::AuditSink>,
    /// worker -> (bearer, token id), one token each for the fixture workers.
    pub tokens: HashMap<WorkerId, (String, TokenId)>,
}

pub async fn start(tag: &str) -> World {
    start_with(tag, None).await
}

/// Migrate and seed, assemble mod-tokens with the fixtures, give each fixture worker a token, serve behind the
/// chain. `entropy` replaces the OS source of the issue path when given.
pub async fn start_with(tag: &str, entropy: Option<Arc<dyn Entropy>>) -> World {
    let (db, s) = bootstrap(tag).await;
    let mut admin = db.admin().await;
    sqlx::raw_sql(super::audit::BOOTSTRAP_SQL)
        .execute(&mut admin)
        .await
        .expect("audit role");
    let mut owner = cortex_db::OwnerConn::connect(&db.owner_config())
        .await
        .expect("owner");
    let fault = Fault::new();
    cortex_db::migrate::apply(
        &mut owner,
        &AuditPiece {
            fault: Arc::clone(&fault),
        }
        .migrations(),
    )
    .await
    .expect("audit fixture migration");
    let clock = Arc::new(TestClock::at(Timestamp::from_unix_micros(NOW)));
    let mut piece = ModTokens::new().with_clock(clock.clone());
    if let Some(e) = entropy {
        piece = piece.with_entropy(e);
    }
    let late = piece.late();
    let enabled = [
        ModuleId::parse("tokens").expect("id"),
        ModuleId::parse("identity").expect("id"),
        ModuleId::parse(super::audit::ID).expect("id"),
    ];
    let compiled: Vec<Box<dyn Piece>> = vec![
        Box::new(AuditPiece {
            fault: Arc::clone(&fault),
        }),
        Box::new(IdentityFixture),
        Box::new(piece),
    ];
    let mut registry = Registry::assemble(Assembly {
        compiled,
        enabled: &enabled,
        minter: minter(),
        chain_codes: CHAIN_CODES,
    })
    .expect("mod-tokens and the fixtures assemble");
    let mut cfg = db.db_config(&[], clock.clone());
    let mut modules = registry.db_modules();
    let platform = ModuleId::parse("platform").expect("id");
    if !modules.contains(&platform) {
        modules.push(platform);
    }
    cfg.registry = ModuleRegistry::new(modules);
    cfg.max_connections = 24;
    let pool = Arc::new(cortex_db::DbPool::connect(&cfg).await.expect("a pool"));
    assert!(late.bind_pool(pool.clone()), "the pool binds once");

    let mut tokens = HashMap::new();
    for (w, p) in [
        (s.lead, s.a),
        (s.tl, s.a),
        (s.ul, s.a),
        (s.w1, s.a),
        (s.w2, s.a),
        (s.w3, s.a),
        (s.w4, s.a),
        (s.lb, s.b),
        (s.wb1, s.b),
    ] {
        tokens.insert(w, insert_token(&pool, &*clock, w, p, 0, 0).await);
    }
    let asked: Asked = Arc::new(Mutex::new(Vec::new()));
    let verifier = registry
        .ports()
        .token_verifier()
        .expect("mod-tokens provides the verifier");
    let logs: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&logs);
    let identity = registry.ports().identity().expect("identity port");
    let audit_sink = registry.ports().audit().expect("audit port");
    let gate = Arc::new(Gate::new());
    let verifier: Arc<dyn TokenVerifier> = Arc::new(GatedVerifier {
        inner: verifier,
        gate: Arc::clone(&gate),
    });
    let chain = Chain {
        verifier,
        policy: Arc::new(Policy(Arc::clone(&asked))),
        log: LogSink::new(move |line| sink.lock().expect("log lock").push(line.to_owned())),
        limits: Limits::default(),
    };
    let app = router(
        chain,
        AppState { pool: pool.clone() },
        RequestIds {
            clock: Arc::new(SystemClock),
        },
        registry.take_routes(),
    )
    .expect("the router");
    World {
        db,
        seed: s,
        pool,
        app,
        clock,
        fault,
        asked,
        logs,
        gate,
        identity,
        audit_sink,
        tokens,
    }
}

#[derive(Debug)]
pub struct Answer {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub text: String,
    pub json: serde_json::Value,
}

impl Answer {
    pub fn code(&self) -> (u16, Option<&str>) {
        (self.status.as_u16(), self.json["type"].as_str())
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }

    /// The body PARSED as JSON with the volatile keys removed (every top-level key whose lower-case name contains
    /// `request`, and `instance`), re-serialised by `serde_json`: for "the same answer apart from the request id". It is
    /// not the raw bytes.
    pub fn body_without_request_id(&self) -> String {
        let mut v = self.json.clone();
        if let Some(o) = v.as_object_mut() {
            o.retain(|k, _| !k.to_lowercase().contains("request"));
            o.remove("instance");
        }
        v.to_string()
    }
}

fn next_key() -> String {
    static N: AtomicU64 = AtomicU64::new(1);
    format!("idem-key-{:08}", N.fetch_add(1, Ordering::SeqCst))
}

/// One request through the router; a write gets a fresh idempotency key unless `key` is given.
pub async fn send(
    app: Router,
    method: Method,
    path: &str,
    token: Option<&str>,
    key: Option<&str>,
    body: Option<&str>,
) -> Answer {
    let mut req = Request::builder().method(method.clone()).uri(path);
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    if method != Method::GET {
        let k = key.map(str::to_owned).unwrap_or_else(next_key);
        req = req
            .header("idempotency-key", k)
            .header("content-type", "application/json");
    }
    let resp = app
        .oneshot(
            req.body(Body::from(body.unwrap_or("").to_owned()))
                .expect("a request"),
        )
        .await
        .expect("a response");
    let (parts, body) = resp.into_parts();
    let text =
        String::from_utf8(to_bytes(body, 1 << 20).await.expect("a body").to_vec()).expect("UTF-8");
    let json = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
    Answer {
        status: parts.status,
        headers: parts.headers,
        text,
        json,
    }
}

impl World {
    pub fn bearer(&self, w: WorkerId) -> &str {
        &self.tokens[&w].0
    }

    pub fn token_id(&self, w: WorkerId) -> TokenId {
        self.tokens[&w].1
    }

    /// Any request. A write gets a fresh idempotency key unless `key` is given.
    pub async fn call(
        &self,
        method: Method,
        path: &str,
        token: Option<&str>,
        key: Option<&str>,
        body: Option<&str>,
    ) -> Answer {
        send(self.app.clone(), method, path, token, key, body).await
    }

    /// A POST that runs on its own task, for a schedule that needs the test to act while it is held.
    pub fn spawn_post(
        &self,
        token: &str,
        path: &str,
        body: serde_json::Value,
    ) -> tokio::task::JoinHandle<Answer> {
        let app = self.app.clone();
        let (token, path, body) = (token.to_owned(), path.to_owned(), body.to_string());
        tokio::spawn(async move {
            send(app, Method::POST, &path, Some(&token), None, Some(&body)).await
        })
    }

    pub async fn get(&self, token: Option<&str>, path: &str) -> Answer {
        self.call(Method::GET, path, token, None, None).await
    }

    pub async fn post(&self, token: &str, path: &str, body: serde_json::Value) -> Answer {
        self.call(
            Method::POST,
            path,
            Some(token),
            None,
            Some(&body.to_string()),
        )
        .await
    }

    pub async fn post_keyed(&self, token: &str, path: &str, key: &str, body: &str) -> Answer {
        self.call(Method::POST, path, Some(token), Some(key), Some(body))
            .await
    }

    /// A scalar read as the superuser (RLS out of the way).
    pub async fn count(&self, sql: impl Into<String>) -> i64 {
        let mut admin = self.db.admin().await;
        sqlx::query_scalar(sqlx::AssertSqlSafe(sql.into()))
            .fetch_one(&mut admin)
            .await
            .expect("count")
    }

    /// The audit rows (action, actor, target, details) in write order.
    pub async fn audit(&self) -> Vec<(String, String, String, String)> {
        let mut admin = self.db.admin().await;
        sqlx::query_as("SELECT action, actor, target, details FROM plug_audit.events ORDER BY id")
            .fetch_all(&mut admin)
            .await
            .expect("audit rows")
    }
}
