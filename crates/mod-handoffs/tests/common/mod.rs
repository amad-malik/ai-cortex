//! Shared fixtures for mod-handoffs' chain-level (L3) tests: the piece mounted behind the real request chain
//! (`cortex-http`), with the Wave 0 plug's identity fixture, an audit fixture that writes a row, a token
//! verifier, and a policy double that follows sophi's table 3.2 for the five rules this piece uses.
//!
//! The pattern is `cortex-http/tests/w0_plug.rs`. Needs the supervisor's Postgres (CV2_PG_PORT,
//! CV2_PG_PASSWORD): run ONLY under `crates/cortex-testkit/supervise.sh`, through `cv2-admit`, with `CV2_DB=1`.
//! Credentials, ids and canaries are synthetic. Each test calls [`world`] with a unique tag and gets a
//! database of its own.
#![allow(dead_code, clippy::too_many_arguments)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::Request;
use axum::http::{HeaderMap, Method, StatusCode};
use cortex_db::migrate::{apply, platform_migrations};
use cortex_db::{DbPool, Migration, MigrationSet, ModuleRegistry, ModuleTx, OwnerConn};
use cortex_http::chain::{Chain, Limits, LogSink, router};
use cortex_http::request_id::RequestIds;
use cortex_module::route::AppState;
use cortex_module::{
    Assembly, AuditDetails, AuditSink, AuditTarget, AuthError, BoxFuture, CONTRACT_VERSION,
    HandoffsApi, Health, Manifest, Piece, Port, PortError, Ports, PortsBuilder, Principal,
    Registry, Role, Routes, ScopePolicy, Target, TargetRule, TokenVerifier, WorkerKind,
};
use cortex_testkit::{TestDb, empty_db};
use cortex_types::{
    HandoffId, ModuleId, ProblemCodeMinter, ProjectId, ScopeName, Secret, SystemClock, TeamId,
    Timestamp, TokenId, WorkerId,
};
use mod_handoffs::Handoffs;
use plug_identity::PlugIdentity;
use tower::ServiceExt;

pub const CALLER_MODULE: &str = "test_a";

fn v7<T>(s: &str, parse: fn(&str) -> Result<T, cortex_types::IdError>) -> T {
    parse(s).expect("a UUIDv7")
}

pub fn project_a() -> ProjectId {
    v7("0193e6a0-0000-7000-8000-00000000a0a0", ProjectId::parse)
}
pub fn project_b() -> ProjectId {
    v7("0193e6a0-0000-7000-8000-00000000b0b0", ProjectId::parse)
}
pub fn team_t() -> TeamId {
    v7("0193e6a0-0000-7000-8000-0000000000a1", TeamId::parse)
}
pub fn team_u() -> TeamId {
    v7("0193e6a0-0000-7000-8000-0000000000a2", TeamId::parse)
}

/// The workers of project A (and B1, B2 of project B). Names are the tokens' suffixes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Who {
    /// The project lead.
    L,
    /// Team lead of T.
    TL,
    /// Team lead of U.
    UL,
    /// Members of T with the job role `dev`.
    W1,
    W2,
    /// Member of U with the job role `qa`.
    W3,
    /// Member with no team and no job role.
    W4,
    /// A retired member of T with the job role `dev`.
    W5,
    /// Project B's lead and a member of B with the job role `dev`.
    B1,
    B2,
}

impl Who {
    pub const ALL: [Who; 10] = [
        Who::L,
        Who::TL,
        Who::UL,
        Who::W1,
        Who::W2,
        Who::W3,
        Who::W4,
        Who::W5,
        Who::B1,
        Who::B2,
    ];

    fn n(self) -> u8 {
        match self {
            Who::L => 1,
            Who::TL => 2,
            Who::UL => 3,
            Who::W1 => 4,
            Who::W2 => 5,
            Who::W3 => 6,
            Who::W4 => 7,
            Who::W5 => 8,
            Who::B1 => 9,
            Who::B2 => 10,
        }
    }

    pub fn id(self) -> WorkerId {
        v7(
            &format!("0193e6a0-0000-7000-8000-0000000002{:02x}", self.n()),
            WorkerId::parse,
        )
    }

    pub fn token(self) -> String {
        format!("tok.{self:?}")
    }

    pub fn project(self) -> ProjectId {
        match self {
            Who::B1 | Who::B2 => project_b(),
            _ => project_a(),
        }
    }

    fn role(self) -> Role {
        match self {
            Who::L | Who::B1 => Role::ProjectLead,
            Who::TL => Role::TeamLead(team_t()),
            Who::UL => Role::TeamLead(team_u()),
            Who::W1 | Who::W2 | Who::W5 => Role::Member(Some(team_t())),
            Who::W3 => Role::Member(Some(team_u())),
            Who::W4 | Who::B2 => Role::Member(None),
        }
    }

    fn job_roles(self) -> &'static str {
        match self {
            Who::W1 | Who::W2 | Who::W5 | Who::B2 => "{dev}",
            Who::W3 => "{qa}",
            _ => "{}",
        }
    }

    fn team(self) -> Option<TeamId> {
        match self.role() {
            Role::TeamLead(t) => Some(t),
            Role::Member(t) => t,
            Role::ProjectLead => None,
        }
    }

    fn sql_role(self) -> &'static str {
        match self.role() {
            Role::ProjectLead => "project_lead",
            Role::TeamLead(_) => "team_lead",
            Role::Member(_) => "member",
        }
    }
}

// ---- the audit fixture ------------------------------------------------------------------------

pub const AUDIT: &str = "fx_audit";
const AUDIT_BOOTSTRAP: &str = "DO $$ BEGIN BEGIN CREATE ROLE cortex_m_fx_audit NOLOGIN; EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL; END; BEGIN GRANT cortex_m_fx_audit TO cortex_app; EXCEPTION WHEN unique_violation THEN NULL; END; END $$;";

const AUDIT_UP: &str = r#"
CREATE SCHEMA fx_audit;
REVOKE ALL ON SCHEMA fx_audit FROM PUBLIC;
CREATE TABLE fx_audit.events (
    id         bigserial PRIMARY KEY,
    project_id uuid NOT NULL,
    action     text NOT NULL,
    target     text NOT NULL,
    details    text NOT NULL
);
ALTER TABLE fx_audit.events ENABLE ROW LEVEL SECURITY;
ALTER TABLE fx_audit.events FORCE ROW LEVEL SECURITY;
CREATE POLICY by_project ON fx_audit.events
    USING (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid)
    WITH CHECK (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid);
GRANT USAGE ON SCHEMA fx_audit TO cortex_m_fx_audit;
GRANT SELECT, INSERT ON fx_audit.events TO cortex_m_fx_audit;
GRANT USAGE ON SEQUENCE fx_audit.events_id_seq TO cortex_m_fx_audit;
"#;

static AUDIT_MANIFEST: Manifest = Manifest {
    id: AUDIT,
    version: "0.0.0",
    contract: CONTRACT_VERSION,
    core: false,
    schema: AUDIT,
    provides: &[Port::Audit],
    uses: &[],
    scopes: &[],
    problems: &[],
};

struct AuditFixture;

struct AuditProvider {
    id: ModuleId,
}

#[async_trait]
impl AuditSink for AuditProvider {
    async fn record(
        &self,
        tx: &mut ModuleTx,
        action: &'static str,
        target: AuditTarget,
        details: AuditDetails,
    ) -> Result<(), PortError> {
        let target = match target {
            AuditTarget::Handoff(h) => format!("handoff:{h}"),
            other => format!("{other:?}"),
        };
        let details = details.render_json();
        tx.delegate(&self.id, async move |t: &mut ModuleTx| {
            sqlx::query(
                "INSERT INTO fx_audit.events (project_id, action, target, details) \
                 VALUES (nullif(current_setting('cortex.project_id', true), '')::uuid, $1, $2, $3)",
            )
            .bind(action)
            .bind(target)
            .bind(details)
            .execute(t.executor()?)
            .await?;
            Ok(())
        })
        .await?;
        Ok(())
    }
}

impl Piece for AuditFixture {
    fn manifest(&self) -> &'static Manifest {
        &AUDIT_MANIFEST
    }
    fn migrations(&self) -> MigrationSet {
        MigrationSet {
            module: ModuleId::parse(AUDIT).expect("an id"),
            schema: AUDIT,
            steps: vec![Migration {
                version: 1,
                name: "fx_audit",
                up: AUDIT_UP,
                down: "DROP SCHEMA fx_audit CASCADE;",
            }],
        }
    }
    fn provide(&self, ports: &mut PortsBuilder) {
        ports.audit(Arc::new(AuditProvider {
            id: ModuleId::parse(AUDIT).expect("an id"),
        }));
    }
    fn routes(&self, _ports: &Ports) -> Routes {
        Routes::new()
    }
    fn health(&self) -> BoxFuture<'_, Health> {
        Box::pin(async { Health::Ready })
    }
}

// ---- the auth doubles -------------------------------------------------------------------------

/// Tokens map to the principals of [`Who`]; a test may change a token's role (a team lead moved to another
/// team keeps its token and scope), which is what R-19's changed-authority replay needs.
struct Verifier {
    roles: Arc<Mutex<HashMap<String, Role>>>,
}

#[async_trait]
impl TokenVerifier for Verifier {
    async fn verify(&self, bearer: &Secret) -> Result<Principal, AuthError> {
        let who = Who::ALL
            .into_iter()
            .find(|w| w.token() == bearer.expose())
            .ok_or(AuthError::Unknown)?;
        Ok(Principal {
            worker: who.id(),
            project: who.project(),
            kind: WorkerKind::Worker,
            role: self
                .roles
                .lock()
                .expect("roles")
                .get(&who.token())
                .copied()
                .unwrap_or_else(|| who.role()),
            token: v7("0193e6a0-0000-7000-8000-0000000002aa", TokenId::parse),
            expires_at: Timestamp::from_unix_micros(4_102_444_799_000_000),
            renew_due: false,
        })
    }
}

/// sophi's table 3.2 (as mod-policy builds it at wave1/mod-policy), for the five rules this piece declares (`None`, `SelfWorker`, `SelfOrProjectLead`,
/// `Lineage`, `TaskAddressee`). The real mod-policy replaces it; a cell that disagrees will say so.
pub struct Policy;

impl ScopePolicy for Policy {
    fn allows(&self, p: &Principal, scope: &ScopeName) -> bool {
        p.kind == WorkerKind::Worker
            && matches!(
                scope.as_str(),
                "handoff:create"
                    | "handoff:read"
                    | "handoff:claim"
                    | "handoff:hold"
                    | "handoff:review"
                    | "handoff:manage"
                    | "identity:read"
            )
    }

    fn target(&self, p: &Principal, rule: TargetRule, t: &Target) -> bool {
        let lead = matches!(p.role, Role::ProjectLead);
        let own = t.worker == Some(p.worker);
        match rule {
            TargetRule::None => true,
            TargetRule::SelfWorker => own,
            TargetRule::SelfOrProjectLead => lead || own,
            TargetRule::Lineage => {
                lead || own
                    || matches!(p.role, Role::TeamLead(team)
                        if t.kind == Some(WorkerKind::Worker) && t.team == Some(team))
            }
            // A consult is open to any principal that passed the scope check (W1-9; mod-policy's
            // `t.consult || may_address_task(p, t)`); a task is the table.
            TargetRule::TaskAddressee => {
                t.consult
                    || lead
                    || matches!(p.role, Role::TeamLead(team) if t.worker.is_some() && t.team == Some(team))
            }
            _ => false,
        }
    }
}

/// One `target` call a route made: the caller, the rule, the row it was asked about.
pub type TargetCall = (WorkerId, TargetRule, Target);

/// A policy that behaves as [`Policy`] and records every `target(principal, rule, T)` call; it can be told to
/// answer `false` to the next one (HAND-116: "the spy answers false once").
#[derive(Default)]
pub struct Spy {
    calls: Mutex<Vec<TargetCall>>,
    refuse_next: std::sync::atomic::AtomicBool,
}

impl Spy {
    pub fn new() -> Arc<Spy> {
        Arc::new(Spy::default())
    }

    /// The calls since the last `take`, in order; empties the record.
    pub fn take(&self) -> Vec<TargetCall> {
        std::mem::take(&mut *self.calls.lock().expect("calls"))
    }

    /// The next `target` call is answered `false`.
    pub fn refuse_next(&self) {
        self.refuse_next.store(true, Ordering::SeqCst);
    }

    /// Whether a refusal is still waiting to be used.
    pub fn refusal_pending(&self) -> bool {
        self.refuse_next.load(Ordering::SeqCst)
    }
}

impl ScopePolicy for Spy {
    fn allows(&self, p: &Principal, scope: &ScopeName) -> bool {
        Policy.allows(p, scope)
    }

    fn target(&self, p: &Principal, rule: TargetRule, t: &Target) -> bool {
        self.calls
            .lock()
            .expect("calls")
            .push((p.worker, rule, t.clone()));
        if self.refuse_next.swap(false, Ordering::SeqCst) {
            return false;
        }
        Policy.target(p, rule, t)
    }
}

// ---- the assembly -----------------------------------------------------------------------------

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
    pub db: TestDb,
    pub app: Router,
    pub pool: Arc<DbPool>,
    /// The piece's own port, as a consumer (boot) reaches it.
    pub port: Arc<dyn HandoffsApi>,
    roles: Arc<Mutex<HashMap<String, Role>>>,
}

/// An empty database with the roles bootstrapped; platform and the three pieces migrated in the registry's
/// order; two projects seeded (A: ten workers' worth of identity; B: two); the routes served through the chain.
pub async fn world(tag: &str) -> World {
    world_with(tag, Arc::new(Policy)).await
}

/// The pieces assembled with NO database: identity, the audit fixture and this piece compiled (plus `extra`),
/// `enabled` of them switched on. For the cells about startup and a disabled piece (HAND-115).
pub fn assemble(
    enabled: &[&str],
    extra: Vec<Box<dyn Piece>>,
) -> Result<Registry, Vec<cortex_module::StartupError>> {
    let enabled: Vec<ModuleId> = enabled
        .iter()
        .map(|e| ModuleId::parse(e).expect("an id"))
        .collect();
    let mut compiled: Vec<Box<dyn Piece>> = vec![
        Box::new(PlugIdentity::new()),
        Box::new(AuditFixture),
        Box::new(Handoffs::new()),
    ];
    compiled.extend(extra);
    Registry::assemble(Assembly {
        compiled,
        enabled: &enabled,
        minter: minter(),
        chain_codes: CHAIN_CODES,
    })
}

/// The same world with a policy of the caller's choice (a spy, in HAND-116 and HAND-121).
pub async fn world_with(tag: &str, policy: Arc<dyn ScopePolicy>) -> World {
    let enabled: Vec<ModuleId> = [mod_handoffs::ID, plug_identity::ID, AUDIT]
        .iter()
        .map(|e| ModuleId::parse(e).expect("an id"))
        .collect();
    let mut registry = Registry::assemble(Assembly {
        compiled: vec![
            Box::new(PlugIdentity::new()) as Box<dyn Piece>,
            Box::new(AuditFixture),
            Box::new(Handoffs::new()),
        ],
        enabled: &enabled,
        minter: minter(),
        chain_codes: CHAIN_CODES,
    })
    .expect("the three pieces assemble");
    let db = empty_db(tag).await;
    let mut admin = db.admin().await;
    for sql in [
        plug_identity::BOOTSTRAP_SQL,
        AUDIT_BOOTSTRAP,
        mod_handoffs::BOOTSTRAP_SQL,
    ] {
        sqlx::raw_sql(sql)
            .execute(&mut admin)
            .await
            .expect("bootstrap");
    }
    let mut owner = OwnerConn::connect(&db.owner_config()).await.expect("owner");
    apply(&mut owner, &platform_migrations())
        .await
        .expect("platform");
    for p in registry.pieces() {
        apply(&mut owner, &p.migrations())
            .await
            .expect("a piece's migrations");
    }
    sqlx::query(
        "INSERT INTO plug_identity.projects (id, name, lead) VALUES ($1::uuid, 'A', $2::uuid), ($3::uuid, 'B', $4::uuid)",
    )
    .bind(project_a().to_string())
    .bind(Who::L.id().to_string())
    .bind(project_b().to_string())
    .bind(Who::B1.id().to_string())
    .execute(&mut admin)
    .await
    .expect("seed projects");
    for who in Who::ALL {
        sqlx::query(
            "INSERT INTO plug_identity.workers (id, project_id, name, kind, role, team_id, job_roles, active) \
             VALUES ($1::uuid, $2::uuid, $3, 'worker', $4, $5::uuid, $6::text[], $7)",
        )
        .bind(who.id().to_string())
        .bind(who.project().to_string())
        .bind(format!("{who:?}").to_lowercase())
        .bind(who.sql_role())
        .bind(who.team().map(|t| t.to_string()))
        .bind(who.job_roles())
        .bind(who != Who::W5)
        .execute(&mut admin)
        .await
        .expect("seed a worker");
    }
    let mut modules = registry.db_modules();
    modules.push(ModuleId::parse(CALLER_MODULE).expect("an id"));
    let mut cfg = db.db_config(&[CALLER_MODULE], Arc::new(SystemClock));
    cfg.registry = ModuleRegistry::new(modules);
    cfg.max_connections = 16;
    let pool = Arc::new(DbPool::connect(&cfg).await.expect("a pool"));
    let port = registry
        .ports()
        .handoffs()
        .expect("the piece provides the port");
    let roles: Arc<Mutex<HashMap<String, Role>>> = Arc::default();
    let app = router(
        Chain {
            verifier: Arc::new(Verifier {
                roles: Arc::clone(&roles),
            }),
            policy,
            log: LogSink::stderr(),
            limits: Limits::default(),
        },
        AppState {
            pool: Arc::clone(&pool),
        },
        RequestIds {
            clock: Arc::new(SystemClock),
        },
        registry.take_routes(),
    )
    .expect("the router");
    World {
        db,
        app,
        pool,
        port,
        roles,
    }
}

// ---- requests ---------------------------------------------------------------------------------

#[derive(Debug)]
pub struct Answer {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub text: String,
    pub json: serde_json::Value,
}

impl Answer {
    /// `(status, problem type)`; the type is `None` for a success body.
    pub fn code(&self) -> (u16, Option<&str>) {
        (self.status.as_u16(), self.json["type"].as_str())
    }
}

static KEY: AtomicU64 = AtomicU64::new(1);

/// A fresh Idempotency-Key (8 to 128 printable ASCII).
pub fn key() -> String {
    format!("bob-test-key-{:08}", KEY.fetch_add(1, Ordering::Relaxed))
}

impl World {
    /// Change the role the verifier gives a worker's token from now on.
    pub fn set_role(&self, who: Who, role: Role) {
        self.roles.lock().expect("roles").insert(who.token(), role);
    }

    pub async fn send(
        &self,
        method: Method,
        path: &str,
        who: Option<Who>,
        key: Option<&str>,
        body: Option<serde_json::Value>,
    ) -> Answer {
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json");
        if let Some(w) = who {
            req = req.header("authorization", format!("Bearer {}", w.token()));
        }
        if let Some(k) = key {
            req = req.header("idempotency-key", k);
        }
        let text = body.map(|b| b.to_string()).unwrap_or_default();
        let resp = self
            .app
            .clone()
            .oneshot(req.body(Body::from(text)).expect("a request"))
            .await
            .expect("a response");
        let (parts, body) = resp.into_parts();
        let text = String::from_utf8(to_bytes(body, 8 << 20).await.expect("a body").to_vec())
            .expect("UTF-8");
        let json = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
        Answer {
            status: parts.status,
            headers: parts.headers,
            text,
            json,
        }
    }

    pub async fn get(&self, who: Who, path: &str) -> Answer {
        self.send(Method::GET, path, Some(who), None, None).await
    }

    /// A write with a fresh key.
    pub async fn post(&self, who: Who, path: &str, body: serde_json::Value) -> Answer {
        self.send(Method::POST, path, Some(who), Some(&key()), Some(body))
            .await
    }

    pub async fn claim(&self, who: Who, id: HandoffId) -> Answer {
        self.send(
            Method::POST,
            &format!("/v2/handoffs/{id}/claim"),
            Some(who),
            Some(&key()),
            None,
        )
        .await
    }

    /// The superuser's view (no row-level security in the way), for what the tests count.
    pub async fn admin(&self) -> sqlx::PgConnection {
        self.db.admin().await
    }

    pub async fn count(&self, sql: &'static str) -> i64 {
        let mut admin = self.admin().await;
        sqlx::query_scalar(sql)
            .fetch_one(&mut admin)
            .await
            .expect("a count")
    }
}

// ---- seeding handoffs (as the superuser; the piece's own writes are the tests' subject) ---------

#[derive(Debug, Clone)]
pub struct Seed {
    pub id: HandoffId,
    pub project: ProjectId,
    pub kind: &'static str,
    pub state: &'static str,
    pub created_by: Who,
    pub to_kind: &'static str,
    pub to_ref: String,
    pub holder: Option<Who>,
    pub rework_holder: Option<Who>,
    pub priority: i32,
    pub round: i32,
    pub summary: String,
    pub verify: Option<String>,
    pub context: Option<String>,
    /// `YYYY-MM-DD HH:MM:SS+00`; `None` = now.
    pub created_at: Option<String>,
    pub closed_at: Option<String>,
    pub close_reason: Option<String>,
    pub incarnation: i64,
}

impl Seed {
    /// A pending task created by `by`, addressed to the job role `dev`.
    pub fn pending(by: Who) -> Seed {
        Seed {
            id: HandoffId::new(&SystemClock),
            project: by.project(),
            kind: "task",
            state: "pending",
            created_by: by,
            to_kind: "job_role",
            to_ref: "dev".to_owned(),
            holder: None,
            rework_holder: None,
            priority: 50,
            round: 1,
            summary: "seeded summary".to_owned(),
            verify: None,
            context: None,
            created_at: None,
            closed_at: None,
            close_reason: None,
            incarnation: 0,
        }
    }

    pub fn addressed_to_worker(mut self, w: Who) -> Seed {
        self.to_kind = "worker";
        self.to_ref = w.id().to_string();
        self
    }

    pub fn addressed_to_role(mut self, role: &str) -> Seed {
        self.to_kind = "job_role";
        self.to_ref = role.to_owned();
        self
    }

    pub fn claimed_by(mut self, w: Who) -> Seed {
        self.state = "claimed";
        self.holder = Some(w);
        self.incarnation = 1;
        self
    }

    pub fn priority(mut self, p: i32) -> Seed {
        self.priority = p;
        self
    }

    pub fn at(mut self, ts: &str) -> Seed {
        self.created_at = Some(ts.to_owned());
        self
    }

    pub fn summary(mut self, s: &str) -> Seed {
        self.summary = s.to_owned();
        self
    }

    pub fn consult(mut self) -> Seed {
        self.kind = "consult";
        self
    }

    /// Insert it (and a creation transition row, so the log starts at 1).
    pub async fn insert(self, admin: &mut sqlx::PgConnection) -> HandoffId {
        sqlx::query(
            "INSERT INTO handoffs.handoffs (id, project_id, kind, state, created_by, to_kind, to_ref, holder, \
             rework_holder, incarnation, priority, summary, next_action, acceptance, verify, context, round, \
             transition_seq, created_at, closed_at, close_reason) \
             VALUES ($1::uuid, $2::uuid, $3, $4, $5::uuid, $6, $7, $8::uuid, $9::uuid, $10, $11, $12, 'next', 'accept', \
             $13, $14, $15, 1, coalesce($16::timestamptz, platform.now()), $17::timestamptz, $18)",
        )
        .bind(self.id.to_string())
        .bind(self.project.to_string())
        .bind(self.kind)
        .bind(self.state)
        .bind(self.created_by.id().to_string())
        .bind(self.to_kind)
        .bind(&self.to_ref)
        .bind(self.holder.map(|w| w.id().to_string()))
        .bind(self.rework_holder.map(|w| w.id().to_string()))
        .bind(self.incarnation)
        .bind(self.priority)
        .bind(&self.summary)
        .bind(&self.verify)
        .bind(&self.context)
        .bind(self.round)
        .bind(&self.created_at)
        .bind(&self.closed_at)
        .bind(&self.close_reason)
        .execute(&mut *admin)
        .await
        .expect("seed a handoff");
        sqlx::query(
            "INSERT INTO handoffs.transitions (id, handoff_id, project_id, seq, from_state, to_state, actor_kind, actor_id, incarnation) \
             VALUES (gen_random_uuid(), $1::uuid, $2::uuid, 1, NULL, 'pending', 'worker', $3, 0)",
        )
        .bind(self.id.to_string())
        .bind(self.project.to_string())
        .bind(self.created_by.id().to_string())
        .execute(&mut *admin)
        .await
        .expect("seed the creation row");
        self.id
    }
}

/// Seed a return of `round` by `author`, with one reference evidence entry.
pub async fn seed_return(
    admin: &mut sqlx::PgConnection,
    h: HandoffId,
    project: ProjectId,
    round: i32,
    author: Who,
    summary: &str,
) {
    let rid: String = sqlx::query_scalar(
        "INSERT INTO handoffs.returns (id, handoff_id, project_id, round, author, incarnation, summary, risks, tests) \
         VALUES (gen_random_uuid(), $1::uuid, $2::uuid, $3, $4::uuid, 1, $5, 'none', '[\"cargo test\"]'::jsonb) RETURNING id::text",
    )
    .bind(h.to_string())
    .bind(project.to_string())
    .bind(round)
    .bind(author.id().to_string())
    .bind(summary)
    .fetch_one(&mut *admin)
    .await
    .expect("seed a return");
    sqlx::query(
        "INSERT INTO handoffs.evidence (id, return_id, project_id, kind, ref, sha256, size) \
         VALUES (gen_random_uuid(), $1::uuid, $2::uuid, 'link', 'https://example.test/run', decode(repeat('ab', 32), 'hex'), 0)",
    )
    .bind(rid)
    .bind(project.to_string())
    .execute(&mut *admin)
    .await
    .expect("seed evidence");
}

/// Seed a review of `round`.
pub async fn seed_review(
    admin: &mut sqlx::PgConnection,
    h: HandoffId,
    project: ProjectId,
    round: i32,
    reviewer: Who,
    verdict: &str,
    notes: Option<&str>,
    self_review: bool,
) {
    sqlx::query(
        "INSERT INTO handoffs.reviews (id, handoff_id, project_id, round, reviewer, verdict, notes, self_review) \
         VALUES (gen_random_uuid(), $1::uuid, $2::uuid, $3, $4::uuid, $5, $6, $7)",
    )
    .bind(h.to_string())
    .bind(project.to_string())
    .bind(round)
    .bind(reviewer.id().to_string())
    .bind(verdict)
    .bind(notes)
    .bind(self_review)
    .execute(&mut *admin)
    .await
    .expect("seed a review");
}

/// Seed a closing transition (`accepted` or `cancelled`) with `seq`, and bump the handoff's counter to it.
pub async fn seed_closing(
    admin: &mut sqlx::PgConnection,
    h: HandoffId,
    project: ProjectId,
    seq: i32,
    from: &str,
    to: &str,
    actor: Who,
    reason: Option<&str>,
) {
    sqlx::query(
        "INSERT INTO handoffs.transitions (id, handoff_id, project_id, seq, from_state, to_state, actor_kind, actor_id, incarnation, reason) \
         VALUES (gen_random_uuid(), $1::uuid, $2::uuid, $3, $4, $5, 'worker', $6, 1, $7)",
    )
    .bind(h.to_string())
    .bind(project.to_string())
    .bind(seq)
    .bind(from)
    .bind(to)
    .bind(actor.id().to_string())
    .bind(reason)
    .execute(&mut *admin)
    .await
    .expect("seed a closing transition");
    sqlx::query("UPDATE handoffs.handoffs SET transition_seq = $2 WHERE id = $1::uuid")
        .bind(h.to_string())
        .bind(seq)
        .execute(&mut *admin)
        .await
        .expect("bump the counter");
}
