//! The Wave 0 plug, at one combined pin (cox's `mvp/WAVE0-FREEZE-BAR.md` r1, bc0eed84): three
//! pieces assembled through `Registry::assemble` and served through `chain::router`.
//!
//! - finn's `plug_identity` provides `IdentityApi` and mounts `GET /v2/identity/roster`;
//! - this file's `plug_audit` provides `AuditSink` (a plain INSERT in the caller's transaction) and
//!   can be told to fail after its write;
//! - this file's `probe` USES both: its write calls `IdentityApi` and `AuditSink` inside the chain's
//!   own transaction, then the chain stores the reply and commits, or nothing is kept.
//!
//! Rows: P-01, P-02 (the registry's refusals and a route mounted past the chain), P-04, P-05 and
//! P-06 (NEG-00 to NEG-09 for every route the plug mounts), P-07 (a handler Problem, and an
//! AuditSink failure, roll back owner, audit and reservation), P-12 (a disabled piece's route is a
//! 404 problem through the chain). P-10: the plug mounts NO secret-issuing route (that case, H-27,
//! stays in `h_db.rs`). P-08 and P-09 are `h_db.rs`, re-run at the same pin.
//!
//! The DB cases need the supervisor's Postgres: run them ONLY under
//! `crates/cortex-testkit/supervise.sh`, through `cv2-admit`; finn runs them (cox 0df539d3):
//!
//! ```text
//! CV2_DB=1 CV2_ADMIT_LOG=1 /path/to/workspace/spikes/cortex-v2-s0/admission/cv2-admit \
//!     timeout --kill-after=120 780 crates/cortex-testkit/supervise.sh \
//!     cargo test -p cortex-http --test w0_plug --no-fail-fast
//! ```
//!
//! `p01`, `p02` and `p12` need no database. Credentials, ids and canaries are synthetic.
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, Method, StatusCode};
use cortex_db::migrate::{apply, platform_migrations};
use cortex_db::{
    DbConfig, DbEndpoint, DbError, DbPool, Migration, MigrationSet, ModuleRegistry, ModuleTx,
    OwnerConn,
};
use cortex_http::chain::{Chain, ChainError, Limits, LogSink, router};
use cortex_http::request_id::RequestIds;
use cortex_module::route::{AppState, Handler};
use cortex_module::{
    Assembly, AuditDetails, AuditSink, AuditTarget, AuthError, BoxFuture, CONTRACT_VERSION, Ctx,
    DefaultRole, Health, Manifest, Piece, Port, PortError, Ports, PortsBuilder, Principal, Problem,
    ProblemSpec, Registry, Reply, Role, Routes, ScopeClass, ScopeDecl, ScopePolicy, StartupError,
    Target, TargetRule, TokenVerifier, WorkerKind,
};
use cortex_testkit::{TestDb, empty_db, within};
use cortex_types::{
    ModuleId, ProblemCodeMinter, ProjectId, ScopeName, Secret, SystemClock, Timestamp, TokenId,
    WorkerId,
};
use plug_identity::PlugIdentity;
use serde::Deserialize;
use serde_json::json;
use tower::ServiceExt;

/// A race-safe bootstrap for a fixture role, the same form as `plug_identity::BOOTSTRAP_SQL`.
macro_rules! bootstrap_sql {
    ($role:literal) => {
        concat!(
            "DO $$ BEGIN\n  BEGIN\n    CREATE ROLE ",
            $role,
            " NOLOGIN;\n  EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL;\n  END;\n  BEGIN\n    GRANT ",
            $role,
            " TO cortex_app;\n  EXCEPTION WHEN unique_violation THEN NULL;\n  END;\nEND $$;"
        )
    };
}

// ---- plug_audit: the AuditSink fixture ----------------------------------------------------------

const AUDIT: &str = "plug_audit";
/// The action the fixture refuses AFTER writing its row, so a rollback must take that row too.
const FAIL_ACTION: &str = "probe.touch.fail_audit";

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

const AUDIT_BOOTSTRAP: &str = bootstrap_sql!("cortex_m_plug_audit");

const AUDIT_UP: &str = r#"
CREATE SCHEMA plug_audit;
REVOKE ALL ON SCHEMA plug_audit FROM PUBLIC;
CREATE TABLE plug_audit.events (
    id         bigserial PRIMARY KEY,
    project_id uuid NOT NULL,
    action     text NOT NULL
);
ALTER TABLE plug_audit.events ENABLE ROW LEVEL SECURITY;
ALTER TABLE plug_audit.events FORCE ROW LEVEL SECURITY;
CREATE POLICY by_project ON plug_audit.events
    USING (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid)
    WITH CHECK (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid);
GRANT USAGE ON SCHEMA plug_audit TO cortex_m_plug_audit;
GRANT SELECT, INSERT ON plug_audit.events TO cortex_m_plug_audit;
GRANT USAGE ON SEQUENCE plug_audit.events_id_seq TO cortex_m_plug_audit;
"#;

struct PlugAudit;

struct AuditProvider {
    id: ModuleId,
}

#[async_trait]
impl AuditSink for AuditProvider {
    async fn record(
        &self,
        tx: &mut ModuleTx,
        action: &'static str,
        _target: AuditTarget,
        _details: AuditDetails,
    ) -> Result<(), PortError> {
        tx.delegate(&self.id, async move |t: &mut ModuleTx| {
            sqlx::query(
                "INSERT INTO plug_audit.events (project_id, action) \
                 VALUES (nullif(current_setting('cortex.project_id', true), '')::uuid, $1)",
            )
            .bind(action)
            .execute(t.executor()?)
            .await?;
            Ok(())
        })
        .await?;
        if action == FAIL_ACTION {
            return Err(PortError::Unavailable);
        }
        Ok(())
    }
}

impl Piece for PlugAudit {
    fn manifest(&self) -> &'static Manifest {
        &AUDIT_MANIFEST
    }
    fn migrations(&self) -> MigrationSet {
        MigrationSet {
            module: ModuleId::parse(AUDIT).expect("an id"),
            schema: AUDIT,
            steps: vec![Migration {
                version: 1,
                name: "plug_audit",
                up: AUDIT_UP,
                down: "DROP SCHEMA plug_audit CASCADE;",
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

// ---- probe: the consumer ---------------------------------------------------------------------

const PROBE: &str = "probe";

const PROBE_FAILED: ProblemSpec = ProblemSpec {
    code: "probe_failed",
    status: 409,
    title: "Probe failed",
    fix: "Send the request without fail.",
};

static PROBE_MANIFEST: Manifest = Manifest {
    id: PROBE,
    version: "0.0.0",
    contract: CONTRACT_VERSION,
    core: false,
    schema: PROBE,
    provides: &[],
    uses: &[Port::Identity, Port::Audit],
    scopes: &[
        ScopeDecl {
            name: "probe:read",
            class: ScopeClass::Work,
            default_roles: &[DefaultRole::Member],
        },
        ScopeDecl {
            name: "probe:write",
            class: ScopeClass::Work,
            default_roles: &[DefaultRole::Member],
        },
    ],
    problems: &[PROBE_FAILED],
};

const PROBE_BOOTSTRAP: &str = bootstrap_sql!("cortex_m_probe");

const PROBE_UP: &str = r#"
CREATE SCHEMA probe;
REVOKE ALL ON SCHEMA probe FROM PUBLIC;
CREATE TABLE probe.touches (
    id          bigserial PRIMARY KEY,
    project_id  uuid NOT NULL,
    v           text NOT NULL,
    worker_name text NOT NULL
);
ALTER TABLE probe.touches ENABLE ROW LEVEL SECURITY;
ALTER TABLE probe.touches FORCE ROW LEVEL SECURITY;
CREATE POLICY by_project ON probe.touches
    USING (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid)
    WITH CHECK (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid);
GRANT USAGE ON SCHEMA probe TO cortex_m_probe;
GRANT SELECT, INSERT ON probe.touches TO cortex_m_probe;
GRANT USAGE ON SEQUENCE probe.touches_id_seq TO cortex_m_probe;
"#;

/// Strict: an unknown field (a `project`, say) is 422.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Touch {
    v: String,
    #[serde(default)]
    fail: bool,
    #[serde(default)]
    fail_audit: bool,
    /// A nested reference to another probe row: it must be visible in the caller's project.
    #[serde(default)]
    ref_row: Option<i64>,
}

fn probe_id() -> ModuleId {
    ModuleId::parse(PROBE).expect("an id")
}

struct ProbePiece;

impl Piece for ProbePiece {
    fn manifest(&self) -> &'static Manifest {
        &PROBE_MANIFEST
    }

    fn migrations(&self) -> MigrationSet {
        MigrationSet {
            module: probe_id(),
            schema: PROBE,
            steps: vec![Migration {
                version: 1,
                name: "probe",
                up: PROBE_UP,
                down: "DROP SCHEMA probe CASCADE;",
            }],
        }
    }

    fn routes(&self, ports: &Ports) -> Routes {
        let identity = ports.identity().expect("the registry checked `uses`");
        let audit = ports.audit().expect("the registry checked `uses`");
        Routes::new()
            .write_open(Method::POST, "/v2/probe/touch", "probe:write", move |tx, ctx, b: Touch| {
                let (identity, audit) = (Arc::clone(&identity), Arc::clone(&audit));
                Box::pin(async move {
                    // P-04: the port in THIS transaction, under plug_identity's role, in the token's
                    // project; an AI Worker unknown there is 404.
                    let me = identity
                        .worker(tx, &ctx.principal().worker)
                        .await
                        .map_err(Problem::from)?
                        .ok_or_else(Problem::not_found)?;
                    // NEG-02: a nested reference must be visible in the caller's project.
                    if let Some(r) = b.ref_row {
                        let seen: Option<i64> = sqlx::query_scalar("SELECT id FROM probe.touches WHERE id = $1")
                            .bind(r)
                            .fetch_optional(tx.executor()?)
                            .await
                            .map_err(DbError::from)?;
                        seen.ok_or_else(Problem::not_found)?;
                    }
                    // The owner's write, as cortex_m_probe (42501 if the delegation kept its role).
                    let row: i64 = sqlx::query_scalar(
                        "INSERT INTO probe.touches (project_id, v, worker_name) \
                         VALUES (nullif(current_setting('cortex.project_id', true), '')::uuid, $1, $2) RETURNING id",
                    )
                    .bind(&b.v)
                    .bind(&me.name)
                    .fetch_one(tx.executor()?)
                    .await
                    .map_err(DbError::from)?;
                    // P-07: the audit row in the same transaction; a refusal takes both back.
                    let action = if b.fail_audit { FAIL_ACTION } else { "probe.touch" };
                    audit
                        .record(
                            tx,
                            action,
                            AuditTarget::Worker(ctx.principal().worker),
                            AuditDetails::new(),
                        )
                        .await
                        .map_err(Problem::from)?;
                    if b.fail {
                        return Err(Problem::new(&PROBE_FAILED, "The probe was told to fail after its writes."));
                    }
                    Ok(Reply::created(json!({ "row": row, "v": b.v, "worker": me.name })))
                })
            })
            .read(
                Method::GET,
                "/v2/probe/count/{v}",
                "probe:read",
                TargetRule::None,
                |State(st): State<AppState>, ctx: Ctx, Path(v): Path<String>| async move {
                    let mut tx = ctx.begin(&st.pool, &probe_id()).await?;
                    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM probe.touches WHERE v = $1")
                        .bind(&v)
                        .fetch_one(tx.executor()?)
                        .await
                        .map_err(DbError::from)?;
                    tx.rollback().await?;
                    Ok::<String, Problem>(n.to_string())
                },
            )
            .read(
                Method::GET,
                "/v2/probe/row/{id}",
                "probe:read",
                TargetRule::None,
                |State(st): State<AppState>, ctx: Ctx, Path(id): Path<i64>| async move {
                    let mut tx = ctx.begin(&st.pool, &probe_id()).await?;
                    let v: Option<String> = sqlx::query_scalar("SELECT v FROM probe.touches WHERE id = $1")
                        .bind(id)
                        .fetch_optional(tx.executor()?)
                        .await
                        .map_err(DbError::from)?;
                    tx.rollback().await?;
                    // Another project's row looks exactly like a missing one.
                    let v = v.ok_or_else(Problem::not_found)?;
                    Ok::<axum::Json<serde_json::Value>, Problem>(axum::Json(json!({ "row": id, "v": v })))
                },
            )
    }

    fn health(&self) -> BoxFuture<'_, Health> {
        Box::pin(async { Health::Ready })
    }
}

/// P-02 fixtures: a piece that mounts a route another piece already mounts, and one whose route
/// names a scope no manifest declares. No migrations; never reached past startup.
struct Extra {
    manifest: &'static Manifest,
    route: (Method, &'static str, &'static str),
}

static TWICE_MANIFEST: Manifest = Manifest {
    id: "probe_twice",
    version: "0.0.0",
    contract: CONTRACT_VERSION,
    core: false,
    schema: "probe_twice",
    provides: &[],
    uses: &[],
    scopes: &[ScopeDecl {
        name: "twice:write",
        class: ScopeClass::Work,
        default_roles: &[DefaultRole::Member],
    }],
    problems: &[],
};

static UNKNOWN_SCOPE_MANIFEST: Manifest = Manifest {
    id: "probe_unknown",
    version: "0.0.0",
    contract: CONTRACT_VERSION,
    core: false,
    schema: "probe_unknown",
    provides: &[],
    uses: &[],
    scopes: &[],
    problems: &[],
};

impl Piece for Extra {
    fn manifest(&self) -> &'static Manifest {
        self.manifest
    }
    fn migrations(&self) -> MigrationSet {
        MigrationSet {
            module: ModuleId::parse(self.manifest.id).expect("an id"),
            schema: self.manifest.schema,
            steps: vec![],
        }
    }
    fn routes(&self, _ports: &Ports) -> Routes {
        let (m, path, scope) = self.route.clone();
        Routes::new().read(m, path, scope, TargetRule::None, || async { "never" })
    }
    fn health(&self) -> BoxFuture<'_, Health> {
        Box::pin(async { Health::Ready })
    }
}

// ---- the auth doubles --------------------------------------------------------------------------

const A1: &str = "ctx2_plug.a1";
const A2: &str = "ctx2_plug.a2";
const B1: &str = "ctx2_plug.b1";
const A_STRANGER: &str = "ctx2_plug.ax";
const A_NOSCOPE: &str = "ctx2_plug.ans";
const A_REVOKED: &str = "ctx2_plug.arev";
const A_EXPIRED: &str = "ctx2_plug.aexp";
const BAD: &str = "ctx2_plug.unknown";

fn v7<T>(s: &str, parse: fn(&str) -> Result<T, cortex_types::IdError>) -> T {
    parse(s).expect("a UUIDv7")
}
fn project_a() -> ProjectId {
    v7("0193e6a0-0000-7000-8000-00000000a0a0", ProjectId::parse)
}
fn project_b() -> ProjectId {
    v7("0193e6a0-0000-7000-8000-00000000b0b0", ProjectId::parse)
}
fn w(n: u8) -> WorkerId {
    v7(
        &format!("0193e6a0-0000-7000-8000-0000000002{n:02}"),
        WorkerId::parse,
    )
}

struct Verifier;
#[async_trait]
impl TokenVerifier for Verifier {
    async fn verify(&self, bearer: &Secret) -> Result<Principal, AuthError> {
        let (worker, project) = match bearer.expose() {
            A1 => (w(1), project_a()),
            A2 => (w(2), project_a()),
            B1 => (w(3), project_b()),
            A_STRANGER => (w(9), project_a()),
            A_NOSCOPE => (w(8), project_a()),
            A_REVOKED => return Err(AuthError::Revoked),
            A_EXPIRED => return Err(AuthError::Expired),
            _ => return Err(AuthError::Unknown),
        };
        Ok(Principal {
            worker,
            project,
            kind: WorkerKind::Worker,
            role: Role::Member(None),
            token: v7("0193e6a0-0000-7000-8000-0000000002aa", TokenId::parse),
            expires_at: Timestamp::from_unix_micros(4_102_444_799_000_000),
            renew_due: false,
        })
    }
}

struct Policy;
impl ScopePolicy for Policy {
    fn allows(&self, p: &Principal, scope: &ScopeName) -> bool {
        p.worker != w(8)
            && matches!(
                scope.as_str(),
                "probe:read" | "probe:write" | "identity:read"
            )
    }
    // The real policy answers true for rule None (a write_open route asks it); every other rule
    // is mod-policy's truth table, which this fixture does not model.
    fn target(&self, _p: &Principal, rule: TargetRule, _t: &Target) -> bool {
        matches!(rule, TargetRule::None)
    }
}

// ---- the assembly ----------------------------------------------------------------------------

/// The process's one minter, shared by every test in this binary (as finn's registry tests do).
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

fn compiled() -> Vec<Box<dyn Piece>> {
    vec![
        Box::new(PlugIdentity::new()),
        Box::new(PlugAudit),
        Box::new(ProbePiece),
    ]
}

fn assemble_from(
    compiled: Vec<Box<dyn Piece>>,
    enabled: &[&str],
) -> Result<Registry, Vec<StartupError>> {
    let enabled: Vec<ModuleId> = enabled
        .iter()
        .map(|e| ModuleId::parse(e).expect("an id"))
        .collect();
    Registry::assemble(Assembly {
        compiled,
        enabled: &enabled,
        minter: minter(),
        chain_codes: CHAIN_CODES,
    })
}

fn assemble(enabled: &[&str]) -> Result<Registry, Vec<StartupError>> {
    assemble_from(compiled(), enabled)
}

const ALL: &[&str] = &[plug_identity::ID, AUDIT, PROBE];

fn chain() -> Chain {
    Chain {
        verifier: Arc::new(Verifier),
        policy: Arc::new(Policy),
        log: LogSink::stderr(),
        limits: Limits::default(),
    }
}

fn ids() -> RequestIds {
    RequestIds {
        clock: Arc::new(SystemClock),
    }
}

/// A pool that never connects (port 9), for the DB-free cases.
fn dead_state(registry: &Registry) -> AppState {
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

struct Plug {
    db: TestDb,
    app: Router,
}

/// On an empty database: bootstrap the three roles, migrate platform then the pieces in the
/// registry's order, seed
/// identity (A: two AI Workers; B: one), and serve the registry's routes through the chain.
async fn plug(tag: &str) -> Plug {
    let mut registry = assemble(ALL).expect("the three pieces assemble");
    // P-03: a TRULY EMPTY database (the roles, no fixture schema): platform and the three pieces'
    // migrations are the only schema there is.
    let db = empty_db(tag).await;
    let mut admin = db.admin().await;
    for sql in [
        plug_identity::BOOTSTRAP_SQL,
        AUDIT_BOOTSTRAP,
        PROBE_BOOTSTRAP,
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
    .bind(w(1).to_string())
    .bind(project_b().to_string())
    .bind(w(3).to_string())
    .execute(&mut admin)
    .await
    .expect("seed projects");
    sqlx::query(
        "INSERT INTO plug_identity.workers (id, project_id, name, kind, role) VALUES \
         ($1::uuid, $2::uuid, 'a-one', 'worker', 'project_lead'), \
         ($3::uuid, $2::uuid, 'a-two', 'worker', 'member'), \
         ($4::uuid, $5::uuid, 'b-one', 'worker', 'project_lead')",
    )
    .bind(w(1).to_string())
    .bind(project_a().to_string())
    .bind(w(2).to_string())
    .bind(w(3).to_string())
    .bind(project_b().to_string())
    .execute(&mut admin)
    .await
    .expect("seed workers");
    // The pool knows exactly the registry's modules: platform, then the enabled pieces. Never a
    // hand-built list (cox 17863602; piece 11 contract s4b).
    let mut cfg = db.db_config(&[], Arc::new(SystemClock));
    cfg.registry = ModuleRegistry::new(registry.db_modules());
    let pool = DbPool::connect(&cfg).await.expect("a pool");
    let app = router(
        chain(),
        AppState {
            pool: Arc::new(pool),
        },
        ids(),
        registry.take_routes(),
    )
    .expect("the router");
    Plug { db, app }
}

#[derive(Debug)]
struct Answer {
    status: StatusCode,
    headers: HeaderMap,
    text: String,
    json: serde_json::Value,
}

impl Answer {
    fn code(&self) -> (u16, Option<&str>) {
        (self.status.as_u16(), self.json["type"].as_str())
    }
}

async fn send(
    app: &Router,
    method: Method,
    path: &str,
    token: Option<&str>,
    extra: &[(&str, &str)],
    body: String,
) -> Answer {
    let mut req = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", "application/json");
    if let Some(t) = token {
        req = req.header("authorization", format!("Bearer {t}"));
    }
    for (k, v) in extra {
        req = req.header(*k, *v);
    }
    let resp = app
        .clone()
        .oneshot(req.body(Body::from(body)).expect("a request"))
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

impl Plug {
    async fn call(
        &self,
        method: Method,
        path: &str,
        token: Option<&str>,
        extra: &[(&str, &str)],
        body: String,
    ) -> Answer {
        send(&self.app, method, path, token, extra, body).await
    }

    async fn touch(&self, token: &str, key: &str, body: serde_json::Value) -> Answer {
        self.call(
            Method::POST,
            "/v2/probe/touch",
            Some(token),
            &[("idempotency-key", key)],
            body.to_string(),
        )
        .await
    }

    async fn get(&self, token: Option<&str>, path: &str) -> Answer {
        self.call(Method::GET, path, token, &[], String::new())
            .await
    }

    /// Probe rows with this marker in one project, read as the superuser (no RLS in the way).
    async fn rows(&self, project: ProjectId, v: &str) -> i64 {
        let mut admin = self.db.admin().await;
        sqlx::query_scalar(
            "SELECT count(*) FROM probe.touches WHERE project_id = $1::uuid AND v = $2",
        )
        .bind(project.to_string())
        .bind(v)
        .fetch_one(&mut admin)
        .await
        .expect("count")
    }

    async fn audit_rows(&self, action: &str) -> i64 {
        let mut admin = self.db.admin().await;
        sqlx::query_scalar("SELECT count(*) FROM plug_audit.events WHERE action = $1")
            .bind(action)
            .fetch_one(&mut admin)
            .await
            .expect("count")
    }

    async fn stored(&self, key: &str) -> i64 {
        let mut admin = self.db.admin().await;
        sqlx::query_scalar("SELECT count(*) FROM platform.idempotency WHERE key = $1")
            .bind(key)
            .fetch_one(&mut admin)
            .await
            .expect("count")
    }
}

// ---- P-01, P-02, P-12: no database -----------------------------------------------------------

/// P-01: the three register once; providers before the consumer; the pool's modules; the mounted
/// routes are exactly the declared ones; a missing provider refuses.
#[test]
fn p01_three_pieces_assemble_wire_and_mount_and_a_missing_provider_refuses() {
    let r = assemble(ALL).expect("assembled");
    let order: Vec<&str> = r.pieces().map(|p| p.manifest().id).collect();
    assert_eq!(order, [plug_identity::ID, AUDIT, PROBE], "providers first");
    let db: Vec<String> = r
        .db_modules()
        .iter()
        .map(|m| m.as_str().to_owned())
        .collect();
    assert_eq!(
        db,
        ["platform", plug_identity::ID, AUDIT, PROBE],
        "the pool's modules"
    );
    assert!(r.ports().has(Port::Identity) && r.ports().has(Port::Audit));
    let mut mounted: Vec<(String, &str, &str)> = r
        .routes()
        .flat_map(|(_, routes)| {
            routes
                .entries()
                .map(|e| (e.method.to_string(), e.path, e.scope))
                .collect::<Vec<_>>()
        })
        .collect();
    mounted.sort();
    assert_eq!(
        mounted,
        [
            ("GET".to_owned(), "/v2/identity/roster", "identity:read"),
            ("GET".to_owned(), "/v2/probe/count/{v}", "probe:read"),
            ("GET".to_owned(), "/v2/probe/row/{id}", "probe:read"),
            ("POST".to_owned(), "/v2/probe/touch", "probe:write"),
        ]
    );
    let no_audit = assemble(&[plug_identity::ID, PROBE]).expect_err("probe without AuditSink");
    assert!(
        no_audit.iter().any(|e| matches!(e, StartupError::MissingPort { piece, port: Port::Audit } if piece.as_str() == PROBE)),
        "{no_audit:?}"
    );
    let alone = assemble(&[PROBE]).expect_err("probe without either provider");
    assert!(
        alone.iter().any(|e| matches!(e, StartupError::MissingPort { piece, port: Port::Identity } if piece.as_str() == PROBE)),
        "{alone:?}"
    );
}

/// P-02 at the pin: the registry refuses a route mounted twice and a scope nobody declares; the
/// chain refuses the same table; and a route mounted PAST the chain cannot run its handler (its
/// `Ctx` is missing, so it fails closed with a 500 problem) (H-02).
#[tokio::test]
async fn p02_startup_refuses_and_a_route_past_the_chain_cannot_run() {
    let mut c = compiled();
    c.push(Box::new(Extra {
        manifest: &TWICE_MANIFEST,
        route: (Method::POST, "/v2/probe/touch", "twice:write"),
    }));
    let twice = assemble_from(c, &[plug_identity::ID, AUDIT, PROBE, "probe_twice"])
        .expect_err("one route, two pieces");
    assert!(
        twice
            .iter()
            .any(|e| matches!(e, StartupError::RouteTwice { .. })),
        "{twice:?}"
    );

    let mut c = compiled();
    c.push(Box::new(Extra {
        manifest: &UNKNOWN_SCOPE_MANIFEST,
        route: (Method::GET, "/v2/probe/nowhere", "nowhere:read"),
    }));
    let unknown = assemble_from(c, &[plug_identity::ID, AUDIT, PROBE, "probe_unknown"])
        .expect_err("an undeclared scope");
    assert!(
        unknown
            .iter()
            .any(|e| matches!(e, StartupError::UnknownScope { .. })),
        "{unknown:?}"
    );

    // The chain refuses the same table on its own (a server that skipped the registry's check).
    let mut r = assemble(ALL).expect("assembled");
    let state = dead_state(&r);
    let mut routes = r.take_routes();
    routes.push((PROBE, ProbePiece.routes(r.ports())));
    assert!(matches!(
        router(chain(), state.clone(), ids(), routes),
        Err(ChainError::RouteTwice { .. })
    ));

    // A read handler mounted on a raw axum router, past the chain: no Ctx, so it refuses closed.
    let probe = ProbePiece.routes(r.ports());
    let raw = probe
        .into_entries()
        .into_iter()
        .find_map(|e| match e.handler {
            Handler::Read(mr) if e.path == "/v2/probe/count/{v}" => Some(*mr),
            _ => None,
        })
        .expect("the count route");
    let raw_app: Router = Router::new()
        .route("/v2/probe/count/{v}", raw)
        .with_state(state);
    let a = send(
        &raw_app,
        Method::GET,
        "/v2/probe/count/x",
        Some(A1),
        &[],
        String::new(),
    )
    .await;
    assert_eq!(
        a.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "a handler past the chain must not run: {}",
        a.text
    );
    assert_ne!(a.text, "0", "no count was read");
}

/// P-12: a disabled piece is not mounted; its route answers a 404 problem through the chain, while
/// an enabled piece's route is still there.
#[tokio::test]
async fn p12_a_disabled_pieces_route_is_a_404_problem_through_the_chain() {
    let mut r = assemble(&[plug_identity::ID, AUDIT]).expect("probe disabled");
    let state = dead_state(&r);
    let app = router(chain(), state, ids(), r.take_routes()).expect("the router");
    let gone = send(
        &app,
        Method::POST,
        "/v2/probe/touch",
        Some(A1),
        &[("idempotency-key", "p12-key-0000001")],
        r#"{"v":"x"}"#.into(),
    )
    .await;
    assert_eq!(gone.code(), (404, Some("not_found")), "{}", gone.text);
    assert_eq!(gone.headers["content-type"], "application/problem+json");
    let roster = send(
        &app,
        Method::GET,
        "/v2/identity/roster",
        None,
        &[],
        String::new(),
    )
    .await;
    assert_eq!(
        roster.code(),
        (401, Some("token_missing")),
        "the enabled piece's route is mounted"
    );
}

// ---- P-04, P-05/P-06, P-07: the database -----------------------------------------------------

/// P-04: both ports run inside the chain's transaction for the write; the caller's role is back
/// after them (its own insert succeeds); a handler refusal after all three writes keeps none; an
/// AI Worker unknown to identity is 404.
#[tokio::test]
async fn p04_the_ports_run_in_the_callers_transaction() {
    within(120, async {
        let p = plug("w0plug_p04").await;
        let ok = p.touch(A1, "p04-key-00000001", json!({ "v": "p04" })).await;
        assert_eq!(ok.status, StatusCode::CREATED, "{}", ok.text);
        assert_eq!(
            ok.json["worker"], "a-one",
            "the name came through IdentityApi"
        );
        assert_eq!(p.rows(project_a(), "p04").await, 1);
        assert_eq!(
            p.audit_rows("probe.touch").await,
            1,
            "the audit row committed with it"
        );
        let failed = p
            .touch(
                A1,
                "p04-key-00000002",
                json!({ "v": "p04-fail", "fail": true }),
            )
            .await;
        assert_eq!(
            failed.code(),
            (409, Some("probe_failed")),
            "{}",
            failed.text
        );
        assert_eq!(
            p.rows(project_a(), "p04-fail").await,
            0,
            "the owner's write rolled back"
        );
        assert_eq!(p.audit_rows("probe.touch").await, 1, "and its audit row");
        assert_eq!(p.stored("p04-key-00000002").await, 0, "and the reservation");
        let stranger = p
            .touch(
                A_STRANGER,
                "p04-key-00000003",
                json!({ "v": "p04-stranger" }),
            )
            .await;
        assert_eq!(stranger.code(), (404, Some("not_found")));
        assert_eq!(p.rows(project_a(), "p04-stranger").await, 0);
    })
    .await
}

/// P-07: an AuditSink failure AFTER its own insert rolls back the audit row, the owner's row and
/// the idempotency reservation; the refusal is the port's problem (503 unavailable).
#[tokio::test]
async fn p07_an_audit_failure_rolls_back_owner_audit_and_reservation() {
    within(120, async {
        let p = plug("w0plug_p07").await;
        let key = "p07-key-00000001";
        let r = p
            .touch(A1, key, json!({ "v": "p07", "fail_audit": true }))
            .await;
        assert_eq!(r.code(), (503, Some("unavailable")), "{}", r.text);
        assert_eq!(
            p.rows(project_a(), "p07").await,
            0,
            "the owner's row is gone"
        );
        assert_eq!(
            p.audit_rows(FAIL_ACTION).await,
            0,
            "the audit row it wrote is gone"
        );
        assert_eq!(p.stored(key).await, 0, "no reservation was kept");
        let again = p.touch(A1, key, json!({ "v": "p07" })).await;
        assert!(
            again.status == StatusCode::CREATED
                && again.headers.get("cortex-idempotent-replay").is_none(),
            "{again:?}"
        );
        assert_eq!(
            (
                p.rows(project_a(), "p07").await,
                p.audit_rows("probe.touch").await
            ),
            (1, 1)
        );
    })
    .await
}

/// P-05 and P-06: NEG-00 to NEG-09 for every route the plug mounts (the roster, probe's count,
/// row and touch), with project B's data marked by a canary that A must never see, and both
/// projects read back after every refusal.
#[tokio::test]
async fn p05_p06_the_negative_matrix_for_every_plug_route() {
    within(240, async {
        let p = plug("w0plug_neg").await;
        const A_CANARY: &str = "canary-a-91d3";
        const B_CANARY: &str = "canary-b-47c8";
        // NEG-00 and NEG-07: the positive controls, one write in each project.
        let a = p
            .touch(A1, "neg-key-a0000001", json!({ "v": A_CANARY }))
            .await;
        let b = p
            .touch(B1, "neg-key-b0000001", json!({ "v": B_CANARY }))
            .await;
        assert_eq!(
            (a.status, b.status),
            (StatusCode::CREATED, StatusCode::CREATED),
            "{a:?} {b:?}"
        );
        let (a_row, b_row) = (
            a.json["row"].as_i64().expect("a row"),
            b.json["row"].as_i64().expect("a row"),
        );
        let a_path = format!("/v2/probe/row/{a_row}");
        let b_path = format!("/v2/probe/row/{b_row}");
        assert_eq!(p.get(Some(A1), &a_path).await.json["v"], A_CANARY);
        assert_eq!(p.get(Some(B1), &b_path).await.json["v"], B_CANARY);
        let roster_a = p.get(Some(A1), "/v2/identity/roster").await;
        assert!(
            roster_a.status == StatusCode::OK
                && roster_a.text.contains("a-one")
                && !roster_a.text.contains("b-one")
        );

        // NEG-01 (A at B through each selector) and NEG-08 (B at A): refused or empty, no canary.
        for (token, path, foreign) in [
            (A1, b_path.clone(), B_CANARY),
            (B1, a_path.clone(), A_CANARY),
        ] {
            let r = p.get(Some(token), &path).await;
            assert_eq!(r.code(), (404, Some("not_found")), "path selector {path}");
            assert!(!r.text.contains(foreign));
        }
        // The count route over HTTP: POSITIVE controls (each project sees its own row) beside the
        // foreign = 0 negatives, so an always-zero count cannot pass (vera's W0-E2).
        let count = |token: &'static str, v: &'static str| {
            let path = format!("/v2/probe/count/{v}");
            let p = &p;
            async move { p.get(Some(token), &path).await.text }
        };
        let (a_own, b_own) = (count(A1, A_CANARY).await, count(B1, B_CANARY).await);
        let (a_sees_b, b_sees_a) = (count(A1, B_CANARY).await, count(B1, A_CANARY).await);
        count_controls(&a_own, &b_own, &a_sees_b, &b_sees_a).expect("the count route's controls");
        let roster_b = p.get(Some(B1), "/v2/identity/roster").await;
        assert!(!roster_b.text.contains("a-one") && roster_b.text.contains("b-one"));
        // A header naming B: the operation stays in A (context comes from the token only).
        let hinted = p
            .call(
                Method::POST,
                "/v2/probe/touch",
                Some(A1),
                &[
                    ("idempotency-key", "neg-key-h0000001"),
                    ("x-project", &project_b().to_string()),
                ],
                json!({ "v": "neg-header" }).to_string(),
            )
            .await;
        assert_eq!(hinted.status, StatusCode::CREATED, "{}", hinted.text);
        assert_eq!(
            (
                p.rows(project_a(), "neg-header").await,
                p.rows(project_b(), "neg-header").await
            ),
            (1, 0)
        );
        // A body naming B: refused by strict decoding, nothing written in either project.
        let body = p
            .touch(
                A1,
                "neg-key-y0000001",
                json!({ "v": "neg-body", "project": project_b().to_string() }),
            )
            .await;
        assert_eq!(body.code(), (422, Some("invalid_request")));
        assert_eq!(
            (
                p.rows(project_a(), "neg-body").await,
                p.rows(project_b(), "neg-body").await
            ),
            (0, 0)
        );

        // NEG-02: a nested reference to B's row from A is refused before any write.
        let nested = p
            .touch(
                A1,
                "neg-key-n0000001",
                json!({ "v": "neg-nested", "ref_row": b_row }),
            )
            .await;
        assert_eq!(nested.code(), (404, Some("not_found")));
        assert_eq!(
            (
                p.rows(project_a(), "neg-nested").await,
                p.rows(project_b(), "neg-nested").await
            ),
            (0, 0)
        );

        // NEG-03 to NEG-06 on every route: no token, revoked, no scope, unknown, expired.
        let routes: Vec<(Method, String, String)> = vec![
            (Method::GET, "/v2/identity/roster".into(), String::new()),
            (
                Method::GET,
                format!("/v2/probe/count/{A_CANARY}"),
                String::new(),
            ),
            (Method::GET, a_path.clone(), String::new()),
            (
                Method::POST,
                "/v2/probe/touch".into(),
                json!({ "v": "neg-denied" }).to_string(),
            ),
        ];
        for (method, path, body) in &routes {
            for (token, want) in [
                (None, (401, Some("token_missing"))),
                (Some(A_REVOKED), (401, Some("token_revoked"))),
                (Some(A_NOSCOPE), (403, Some("missing_scope"))),
                (Some(BAD), (401, Some("token_invalid"))),
                (Some(A_EXPIRED), (401, Some("token_expired"))),
            ] {
                let r = p
                    .call(
                        method.clone(),
                        path,
                        token,
                        &[("idempotency-key", "neg-key-d0000001")],
                        body.clone(),
                    )
                    .await;
                assert_eq!(r.code(), want, "{method} {path} with {token:?}");
                assert!(
                    !r.text.contains(A_CANARY) && !r.text.contains(B_CANARY),
                    "{method} {path}: a canary leaked"
                );
            }
        }
        assert_eq!(
            (
                p.rows(project_a(), "neg-denied").await,
                p.rows(project_b(), "neg-denied").await
            ),
            (0, 0)
        );
        assert_eq!(
            p.stored("neg-key-d0000001").await,
            0,
            "no denied call reserved the key"
        );

        // NEG-09: A's committed key, retried by a revoked token, a scope-less token and B's token.
        let key = "neg-key-r0000001";
        let first = p.touch(A1, key, json!({ "v": "neg-replay" })).await;
        assert_eq!(first.status, StatusCode::CREATED);
        assert_eq!(
            p.touch(A_REVOKED, key, json!({ "v": "neg-replay" }))
                .await
                .code(),
            (401, Some("token_revoked"))
        );
        assert_eq!(
            p.touch(A_NOSCOPE, key, json!({ "v": "neg-replay" }))
                .await
                .code(),
            (403, Some("missing_scope"))
        );
        let theirs = p.touch(B1, key, json!({ "v": "neg-replay" })).await;
        assert_eq!(
            theirs.status,
            StatusCode::CREATED,
            "B's key is B's own operation"
        );
        assert!(
            theirs.headers.get("cortex-idempotent-replay").is_none(),
            "never A's stored reply"
        );
        assert_eq!(theirs.json["worker"], "b-one");
        assert_eq!(
            (
                p.rows(project_a(), "neg-replay").await,
                p.rows(project_b(), "neg-replay").await
            ),
            (1, 1)
        );
        // The positive rows are unchanged by everything above.
        assert_eq!(
            (
                p.rows(project_a(), A_CANARY).await,
                p.rows(project_b(), B_CANARY).await
            ),
            (1, 1)
        );
    })
    .await
}

/// The count route's four observations: each project sees its own canary row once, and never the
/// other project's. Positive and negative together, so an always-zero count fails (vera's W0-E2).
fn count_controls(a_own: &str, b_own: &str, a_sees_b: &str, b_sees_a: &str) -> Result<(), String> {
    let checks = [
        ("A's token on A's own row", a_own, "1"),
        ("B's token on B's own row", b_own, "1"),
        ("A's token on B's row", a_sees_b, "0"),
        ("B's token on A's row", b_sees_a, "0"),
    ];
    let wrong: Vec<String> = checks
        .iter()
        .filter(|(_, got, want)| got != want)
        .map(|(what, got, want)| format!("{what}: got {got:?}, want {want:?}"))
        .collect();
    if wrong.is_empty() {
        Ok(())
    } else {
        Err(wrong.join("; "))
    }
}

/// The controls' own check, without a database: a stubbed always-zero count must FAIL them, the
/// expected observations must pass, and a leak either way must fail.
#[test]
fn p06_count_controls_refuse_an_always_zero_count() {
    assert!(
        count_controls("0", "0", "0", "0").is_err(),
        "an always-zero count passed the controls"
    );
    assert!(count_controls("1", "1", "0", "0").is_ok());
    assert!(
        count_controls("1", "1", "1", "0").is_err(),
        "A seeing B's row passed"
    );
    assert!(
        count_controls("1", "1", "0", "1").is_err(),
        "B seeing A's row passed"
    );
    assert!(
        count_controls("1", "0", "0", "0").is_err(),
        "B's own row missing passed"
    );
}

/// The roster through the chain: each token's project only; 401 without a token; 403 without the
/// scope.
#[tokio::test]
async fn roster_through_the_chain_is_the_tokens_project_only() {
    within(120, async {
        let p = plug("w0plug_roster").await;
        let names = |a: &Answer| -> Vec<String> {
            a.json
                .as_array()
                .map(|v| {
                    v.iter()
                        .filter_map(|w| w["name"].as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default()
        };
        let a = p.get(Some(A2), "/v2/identity/roster").await;
        assert_eq!(a.status, StatusCode::OK, "{}", a.text);
        assert_eq!(names(&a), ["a-one", "a-two"]);
        let b = p
            .call(
                Method::GET,
                "/v2/identity/roster",
                Some(B1),
                &[("x-project", &project_a().to_string())],
                String::new(),
            )
            .await;
        assert_eq!(names(&b), ["b-one"], "a header naming A changes nothing");
        assert_eq!(
            p.get(None, "/v2/identity/roster").await.code(),
            (401, Some("token_missing"))
        );
        assert_eq!(
            p.get(Some(A_NOSCOPE), "/v2/identity/roster").await.code(),
            (403, Some("missing_scope"))
        );
    })
    .await
}
