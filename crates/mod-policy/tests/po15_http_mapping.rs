//! PO-15: the HTTP mapping of the REAL policy behind the REAL chain (`cortex-http`), DB-free.
//! `allows` false is 403 `missing_scope`; `target` false is 403 `target_not_allowed`; a row the caller
//! cannot see (another project) is 404 BEFORE the target check; a 403 body names no other scope and
//! no token data. The row store is an in-memory, per-project stand-in for row-level security (which
//! `cortex-db`'s own cells and `cortex-http`'s `p06` prove against a real database); what this file
//! proves is the chain's mapping and its ORDER with the real policy, counted by a wrapper around it.
mod support;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::Request;
use axum::http::{Method, StatusCode};
use cortex_db::{DbConfig, DbEndpoint, DbPool, ModuleRegistry};
use cortex_http::chain::{Chain, Limits, LogSink, router};
use cortex_http::request_id::RequestIds;
use cortex_module::route::{AppState, Ctx, Problem, Routes};
use cortex_module::{
    AuthError, Principal, Role, ScopePolicy, Target, TargetRule, TokenVerifier, WorkerKind,
};
use cortex_types::{ModuleId, ProjectId, Secret, SystemClock, WorkerId};
use mod_policy::Policy;
use support::*;
use tower::ServiceExt;

// Synthetic credentials; none is a real token.
const LEAD_A: &str = "ctx2_p15.lead-a";
const TEAM_LEAD_A: &str = "ctx2_p15.teamlead-a";
const MEMBER_A: &str = "ctx2_p15.member-a";
const SERVICE_A: &str = "ctx2_p15.service-a";
const LEAD_B: &str = "ctx2_p15.lead-b";

fn project_a() -> ProjectId {
    ProjectId::parse("0193e6a0-0000-7000-8000-0000000000a1").expect("a v7 id")
}
fn project_b() -> ProjectId {
    ProjectId::parse("0193e6a0-0000-7000-8000-0000000000b1").expect("a v7 id")
}

fn in_project(mut p: Principal, project: ProjectId) -> Principal {
    p.project = project;
    p
}

struct Verifier;

#[async_trait]
impl TokenVerifier for Verifier {
    async fn verify(&self, bearer: &Secret) -> Result<Principal, AuthError> {
        match bearer.expose() {
            LEAD_A => Ok(lead()),
            TEAM_LEAD_A => Ok(team_lead_1()),
            MEMBER_A => Ok(member_1()),
            SERVICE_A => Ok(principal(LEAD, WorkerKind::Service, Role::ProjectLead)),
            LEAD_B => Ok(in_project(
                principal(OTHER, WorkerKind::Worker, Role::ProjectLead),
                project_b(),
            )),
            _ => Err(AuthError::Unknown),
        }
    }
}

/// The REAL policy, with a count of the `target` questions it was asked.
struct Counting {
    real: Policy,
    asked: AtomicUsize,
}

impl ScopePolicy for Counting {
    fn allows(&self, p: &Principal, scope: &cortex_types::ScopeName) -> bool {
        self.real.allows(p, scope)
    }

    fn target(&self, p: &Principal, rule: TargetRule, t: &Target) -> bool {
        self.asked.fetch_add(1, Ordering::SeqCst);
        self.real.target(p, rule, t)
    }
}

/// The rows a project can see: (project, worker) -> the Target the handler would build from it.
type Rows = HashMap<(ProjectId, WorkerId), Target>;

fn rows() -> Rows {
    let mut r = HashMap::new();
    // Project A: a member of team 1 (retirable by the lead), and the lead's own row.
    r.insert(
        (project_a(), wid(M1)),
        tg(Some(M1), Some(WorkerKind::Worker), Some(1), Some(LEAD)),
    );
    r.insert(
        (project_a(), wid(LEAD)),
        tg(Some(LEAD), Some(WorkerKind::Worker), None, Some(LEAD)),
    );
    // A team lead's own row, in its own team: only "never oneself" can refuse its owner (the lead's
    // own row is ALSO the project lead, so "never the lead" would refuse it by itself).
    r.insert(
        (project_a(), wid(TL1)),
        tg(Some(TL1), Some(WorkerKind::Worker), Some(1), Some(LEAD)),
    );
    // Project B: its own worker.
    r.insert(
        (project_b(), wid(OTHER)),
        tg(Some(OTHER), Some(WorkerKind::Worker), None, Some(OTHER)),
    );
    r
}

fn probe(rows: Arc<Rows>) -> Routes {
    Routes::new()
        // An admin scope with no row target.
        .read(
            Method::GET,
            "/v2/p15/create",
            "worker:create",
            TargetRule::None,
            || async { "created" },
        )
        // A self scope.
        .read(
            Method::GET,
            "/v2/p15/me",
            "self:read",
            TargetRule::None,
            || async { "me" },
        )
        // A row-targeted route: the handler reads the row (project-scoped), then asks the rule.
        .read(
            Method::GET,
            "/v2/p15/retire/{id}",
            "worker:retire",
            TargetRule::RetireTarget,
            move |ctx: Ctx| {
                let rows = Arc::clone(&rows);
                async move {
                    let id = ctx.path_id::<WorkerId>("id")?;
                    // 404 for a row this caller's project cannot see, BEFORE any target check.
                    let Some(row) = rows.get(&(ctx.project(), id)) else {
                        return Err(Problem::not_found());
                    };
                    ctx.authorize(row, ()).map(|_| "retired")
                }
            },
        )
}

fn state() -> AppState {
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
        registry: ModuleRegistry::new([ModuleId::parse("probe").expect("a module id")]),
        clock: Arc::new(SystemClock),
    };
    AppState {
        pool: Arc::new(DbPool::connect_lazy(&cfg)),
    }
}

fn app() -> (Router, Arc<Counting>) {
    let policy = Arc::new(Counting {
        real: Policy::new(),
        asked: AtomicUsize::new(0),
    });
    let chain = Chain {
        verifier: Arc::new(Verifier),
        policy: policy.clone(),
        log: LogSink::stderr(),
        // `Chain.limits` came with cortex-http's item D/E (wave1/integration 40066de); the default is what
        // this test needs (it neither fills the in-flight cap nor waits out the timeout).
        limits: Limits::default(),
    };
    let ids = RequestIds {
        clock: Arc::new(SystemClock),
    };
    let app = router(
        chain,
        state(),
        ids,
        vec![("probe", probe(Arc::new(rows())))],
    )
    .expect("a router");
    (app, policy)
}

struct Reply {
    status: StatusCode,
    text: String,
    json: serde_json::Value,
}

async fn get(app: &Router, path: &str, token: &str) -> Reply {
    let req = Request::builder()
        .method(Method::GET)
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .expect("a request");
    let resp = app.clone().oneshot(req).await.expect("a response");
    let (parts, body) = resp.into_parts();
    let text =
        String::from_utf8(to_bytes(body, 1 << 20).await.expect("a body").to_vec()).expect("UTF-8");
    let json = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
    Reply {
        status: parts.status,
        text,
        json,
    }
}

fn kind(r: &Reply) -> (u16, Option<&str>) {
    (r.status.as_u16(), r.json["type"].as_str())
}

fn retire(id: u8) -> String {
    format!("/v2/p15/retire/{}", wid(id))
}

#[tokio::test]
async fn po15_allows_false_is_403_missing_scope_and_the_target_rule_is_never_asked() {
    let (app, policy) = app();
    // A member lacks an admin scope; the project lead holds it.
    let r = get(&app, "/v2/p15/create", MEMBER_A).await;
    assert_eq!(kind(&r), (403, Some("missing_scope")), "{}", r.text);
    assert_eq!(
        get(&app, "/v2/p15/create", LEAD_A).await.status,
        StatusCode::OK
    );
    // A Service holds NO scope, self:read included (R-12), even with the lead's role.
    let r = get(&app, "/v2/p15/me", SERVICE_A).await;
    assert_eq!(kind(&r), (403, Some("missing_scope")), "{}", r.text);
    // A member on a row-targeted admin route is refused at the scope step: `target` is never asked.
    let r = get(&app, &retire(M1), MEMBER_A).await;
    assert_eq!(kind(&r), (403, Some("missing_scope")), "{}", r.text);
    assert_eq!(
        policy.asked.load(Ordering::SeqCst),
        0,
        "a missing scope must not reach the target rule"
    );
}

#[tokio::test]
async fn po15_target_false_is_403_target_not_allowed_and_true_is_served() {
    let (app, policy) = app();
    // The lead retires a member of team 1: the rule allows it.
    let ok = get(&app, &retire(M1), LEAD_A).await;
    assert_eq!(ok.status, StatusCode::OK, "{}", ok.text);
    assert_eq!(policy.asked.load(Ordering::SeqCst), 1);
    // The lead retires itself: the scope is held, the TARGET rule refuses.
    let r = get(&app, &retire(LEAD), LEAD_A).await;
    assert_eq!(kind(&r), (403, Some("target_not_allowed")), "{}", r.text);
    assert_eq!(policy.asked.load(Ordering::SeqCst), 2);
    // A team lead holds the scope too, and the rule lets it retire a member of its own team ...
    assert_eq!(
        get(&app, &retire(M1), TEAM_LEAD_A).await.status,
        StatusCode::OK
    );
    // ... but never ITSELF, though its own row is in its own team (this isolates "never oneself").
    let r = get(&app, &retire(TL1), TEAM_LEAD_A).await;
    assert_eq!(kind(&r), (403, Some("target_not_allowed")), "{}", r.text);
}

#[tokio::test]
async fn po15_a_row_in_another_project_is_404_before_the_target_check() {
    let (app, policy) = app();
    // Project B's lead names a worker that exists only in project A: 404, and `target` is NOT asked.
    let r = get(&app, &retire(M1), LEAD_B).await;
    assert_eq!(kind(&r), (404, Some("not_found")), "{}", r.text);
    assert_eq!(
        policy.asked.load(Ordering::SeqCst),
        0,
        "the 404 comes BEFORE the target check"
    );
    // The control: project A's lead on the same id is served, so the id is real and it is the project that hides it.
    assert_eq!(get(&app, &retire(M1), LEAD_A).await.status, StatusCode::OK);
    assert_eq!(policy.asked.load(Ordering::SeqCst), 1);
    // And an id that exists nowhere is the same 404.
    let nowhere = get(&app, &retire(60), LEAD_A).await;
    assert_eq!(kind(&nowhere), (404, Some("not_found")));
}

#[tokio::test]
async fn po15_a_403_names_no_other_scope_and_no_token_data() {
    let (app, _policy) = app();
    let scope_refusal = get(&app, "/v2/p15/create", MEMBER_A).await;
    let target_refusal = get(&app, &retire(LEAD), LEAD_A).await;
    assert_eq!(kind(&scope_refusal), (403, Some("missing_scope")));
    assert_eq!(kind(&target_refusal), (403, Some("target_not_allowed")));
    // The missing-scope refusal names the one scope the operation needs ...
    assert!(
        scope_refusal.text.contains("worker:create"),
        "{}",
        scope_refusal.text
    );
    for r in [&scope_refusal, &target_refusal] {
        // ... and nothing else: no other scope of the matrix, no token, no token id, no worker id.
        for (name, ..) in MATRIX {
            if name == "worker:create" {
                continue;
            }
            assert!(
                !r.text.contains(name),
                "a 403 must not list the scope {name}: {}",
                r.text
            );
        }
        for secret in [
            MEMBER_A,
            LEAD_A,
            "0193e6a0-0000-7000-8000-0000000000aa",
            &wid(LEAD).to_string(),
            &wid(M1).to_string(),
        ] {
            assert!(
                !r.text.contains(secret),
                "a 403 must not carry {secret}: {}",
                r.text
            );
        }
    }
}
