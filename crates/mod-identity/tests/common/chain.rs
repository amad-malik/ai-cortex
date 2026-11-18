//! The real chain (cortex-http) in front of mod-identity and an audit fixture, with test doubles for
//! tokens and policy (the Wave 0 plug's pattern, `cortex-http/tests/w0_plug.rs`).
//!
//! The policy double is NOT mod-policy (mira's piece). It grants scopes by role (S1 5.3) and answers
//! the two target rules the identity writes use, `RetireTarget` and `NotProjectLead`, from the
//! truth table in `mvp/tests/mod-identity-and-mod-policy.md` 3.2; every other rule is `true`.
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
    ModuleId, ProblemCodeMinter, ProjectId, ScopeName, Secret, SystemClock, Timestamp, TokenId,
    WorkerId,
};
use mod_identity::ModIdentity;
use tower::ServiceExt;

use super::audit::{AuditPiece, Fault};
use super::seed::{F, migrate_and_seed};

pub const A_LEAD: &str = "ctx2_t.alead";
pub const A_TL: &str = "ctx2_t.atl";
pub const A_M1: &str = "ctx2_t.am1";
pub const B_LEAD: &str = "ctx2_t.blead";
pub const C_LEAD: &str = "ctx2_t.clead";

type Tokens = Arc<Mutex<HashMap<String, (WorkerId, ProjectId, Role)>>>;

struct Verifier(Tokens);

#[async_trait]
impl TokenVerifier for Verifier {
    async fn verify(&self, bearer: &Secret) -> Result<Principal, AuthError> {
        let (worker, project, role) = self
            .0
            .lock()
            .expect("tokens lock")
            .get(bearer.expose())
            .copied()
            .ok_or(AuthError::Unknown)?;
        Ok(Principal {
            worker,
            project,
            kind: WorkerKind::Worker,
            role,
            token: TokenId::new(&SystemClock),
            expires_at: Timestamp::from_unix_micros(4_102_444_799_000_000),
            renew_due: false,
        })
    }
}

/// Every `(caller, rule, target)` the handlers asked the policy (PO-17).
pub type Asked = Arc<Mutex<Vec<(WorkerId, TargetRule, Target)>>>;

struct Policy(Asked);

impl ScopePolicy for Policy {
    fn allows(&self, p: &Principal, scope: &ScopeName) -> bool {
        if p.kind != WorkerKind::Worker {
            return false;
        }
        match scope.as_str() {
            "roster:read" => true,
            "team:create" | "team:assign" | "worker:create" => p.role == Role::ProjectLead,
            "worker:retire" => matches!(p.role, Role::ProjectLead | Role::TeamLead(_)),
            _ => false,
        }
    }

    fn target(&self, p: &Principal, rule: TargetRule, t: &Target) -> bool {
        self.0
            .lock()
            .expect("asked lock")
            .push((p.worker, rule, t.clone()));
        match rule {
            TargetRule::NotProjectLead => {
                t.worker.is_some() && t.lead.is_some() && t.worker != t.lead
            }
            TargetRule::RetireTarget => {
                let base = t.worker.is_some()
                    && t.lead.is_some()
                    && t.worker != Some(p.worker)
                    && t.worker != t.lead;
                base && match p.role {
                    Role::ProjectLead => true,
                    Role::TeamLead(team) => t.team == Some(team),
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

pub struct World {
    pub db: cortex_testkit::TestDb,
    pub f: F,
    pub app: Router,
    pub tokens: Tokens,
    pub fault: Arc<Fault>,
    pub asked: Asked,
}

/// Migrate and seed, assemble mod-identity with the audit fixture, and serve both behind the chain.
pub async fn start(tag: &str) -> World {
    let (db, f) = migrate_and_seed(tag).await;
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
    let enabled = [
        ModuleId::parse(mod_identity::ID).expect("id"),
        ModuleId::parse(super::audit::ID).expect("id"),
    ];
    let compiled: Vec<Box<dyn Piece>> = vec![
        Box::new(AuditPiece {
            fault: Arc::clone(&fault),
        }),
        Box::new(ModIdentity::new()),
    ];
    let mut registry = Registry::assemble(Assembly {
        compiled,
        enabled: &enabled,
        minter: minter(),
        chain_codes: CHAIN_CODES,
    })
    .expect("mod-identity and the audit fixture assemble");
    let mut cfg = db.db_config(&[], Arc::new(SystemClock));
    cfg.registry = ModuleRegistry::new(registry.db_modules());
    let pool = cortex_db::DbPool::connect(&cfg).await.expect("a pool");
    let tokens: Tokens = Arc::new(Mutex::new(HashMap::from([
        (A_LEAD.to_owned(), (f.a_lead, f.a, Role::ProjectLead)),
        (A_TL.to_owned(), (f.a_tl, f.a, Role::TeamLead(f.t1))),
        (A_M1.to_owned(), (f.a_m1, f.a, Role::Member(Some(f.t1)))),
        (B_LEAD.to_owned(), (f.b_lead, f.b, Role::ProjectLead)),
        (C_LEAD.to_owned(), (f.c_lead, f.c, Role::ProjectLead)),
    ])));
    let asked: Asked = Arc::new(Mutex::new(Vec::new()));
    let chain = Chain {
        verifier: Arc::new(Verifier(Arc::clone(&tokens))),
        policy: Arc::new(Policy(Arc::clone(&asked))),
        log: LogSink::stderr(),
        limits: Limits::default(),
    };
    let app = router(
        chain,
        AppState {
            pool: Arc::new(pool),
        },
        RequestIds {
            clock: Arc::new(SystemClock),
        },
        registry.take_routes(),
    )
    .expect("the router");
    World {
        db,
        f,
        app,
        tokens,
        fault,
        asked,
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

    /// The body with the request id removed, for "byte-identical apart from the request id".
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

impl World {
    /// Any request. A write gets a fresh idempotency key unless `key` is given.
    pub async fn call(
        &self,
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
        let resp = self
            .app
            .clone()
            .oneshot(
                req.body(Body::from(body.unwrap_or("").to_owned()))
                    .expect("a request"),
            )
            .await
            .expect("a response");
        let (parts, body) = resp.into_parts();
        let text = String::from_utf8(to_bytes(body, 1 << 20).await.expect("a body").to_vec())
            .expect("UTF-8");
        let json = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
        Answer {
            status: parts.status,
            headers: parts.headers,
            text,
            json,
        }
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

    pub async fn put(&self, token: &str, path: &str, body: serde_json::Value) -> Answer {
        self.call(
            Method::PUT,
            path,
            Some(token),
            None,
            Some(&body.to_string()),
        )
        .await
    }

    /// A token for a worker the test created.
    pub fn token_for(
        &self,
        name: &str,
        worker: WorkerId,
        project: ProjectId,
        role: Role,
    ) -> String {
        let t = format!("ctx2_t.{name}");
        self.tokens
            .lock()
            .expect("tokens lock")
            .insert(t.clone(), (worker, project, role));
        t
    }

    /// A scalar, read as the superuser (RLS out of the way).
    pub async fn count(&self, sql: &'static str) -> i64 {
        let mut admin = self.db.admin().await;
        sqlx::query_scalar(sql)
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
