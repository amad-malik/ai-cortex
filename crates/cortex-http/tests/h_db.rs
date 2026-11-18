//! The chain's database cases, through the real chain and a real `ModuleTx`: H-14, H-22 to H-29
//! (`mvp/socket/cortex-http-TESTS.md`), which cover sam's plug cases P-06 to P-10.
//!
//! They need the supervisor's Postgres. Run them ONLY under `crates/cortex-testkit/supervise.sh`
//! (L-9) through `cv2-admit`; for the plug, finn runs them (cox 0df539d3):
//!
//! ```text
//! CV2_ADMIT_LOG=1 /path/to/workspace/spikes/cortex-v2-s0/admission/cv2-admit \
//!     timeout --kill-after=120 780 crates/cortex-testkit/supervise.sh \
//!     cargo test -p cortex-http --test h_db --no-fail-fast
//! ```
//!
//! The probe piece runs as the testkit's fixture module `test_a` (role `cortex_m_test_a`; table
//! `test_a.t` under FORCE row-level security by `cortex.project_id`), so no role is added to the
//! testkit. Each test writes rows marked with its own `v`, so tests share one database safely.
//! Credentials are synthetic.
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, Method, StatusCode};
use cortex_db::idempotency::{begin as idem_begin, finish as idem_finish};
use cortex_db::migrate::{apply, platform_migrations};
use cortex_db::{
    AdvisoryClass, DbPool, Idem, IdemKey, ModuleTx, OwnerConn, StoredResponse, TxOptions,
};
use cortex_http::chain::{Chain, Limits, LogSink, REPLAY_HEADER, router};
use cortex_http::idem;
use cortex_http::request_id::RequestIds;
use cortex_module::route::{AppState, Ctx, Problem, ProblemSpec, Reply, Routes, Status};
use cortex_module::{
    AuthError, Authorized, Principal, Role, ScopePolicy, Target, TargetRule, TokenVerifier,
    WorkerKind,
};
use cortex_testkit::{TestDb, ctx, test_db, within};
use cortex_types::{
    HandoffId, ModuleId, ProjectId, ScopeName, Secret, SystemClock, Timestamp, TokenId, WorkerId,
};
use serde::Deserialize;
use serde_json::json;
use sqlx::Row;
use tower::ServiceExt;

const MODULE: &str = "test_a";

/// Values whose handler REACHED its post-insert wait (`wait_ms`): the barrier for the timeout cell
/// (vera's Low note: without it, a deadline that fired before the insert would make "no row" true
/// for the wrong reason).
static REACHED: Mutex<Vec<String>> = Mutex::new(Vec::new());
const A1: &str = "ctx2_db.a1";
const A3: &str = "ctx2_db.a3";
const B4: &str = "ctx2_db.b4";
const SECRET_CANARY: &str = "canary-secret-issued-5f2e";

fn id<T>(s: &str, parse: fn(&str) -> Result<T, cortex_types::IdError>) -> T {
    parse(s).expect("a UUIDv7")
}
fn project_a() -> ProjectId {
    id("0193e6a0-0000-7000-8000-0000000000a1", ProjectId::parse)
}
fn project_b() -> ProjectId {
    id("0193e6a0-0000-7000-8000-0000000000b1", ProjectId::parse)
}
fn worker(n: u8) -> WorkerId {
    id(
        &format!("0193e6a0-0000-7000-8000-0000000001{n:02}"),
        WorkerId::parse,
    )
}
fn issued_token() -> TokenId {
    id("0193e6a0-0000-7000-8000-0000000000ee", TokenId::parse)
}

/// What each test may switch: a worker's token revoked, its scopes removed, or its authority over
/// the `SelfWorker` target closed (R-19: the gate a replay must not bypass).
#[derive(Default)]
struct World {
    revoked: Mutex<HashSet<WorkerId>>,
    no_scope: Mutex<HashSet<WorkerId>>,
    gate_closed: Mutex<HashSet<WorkerId>>,
}

struct Verifier(Arc<World>);
#[async_trait]
impl TokenVerifier for Verifier {
    async fn verify(&self, bearer: &Secret) -> Result<Principal, AuthError> {
        let (w, project) = match bearer.expose() {
            A1 => (worker(1), project_a()),
            A3 => (worker(3), project_a()),
            B4 => (worker(4), project_b()),
            _ => return Err(AuthError::Unknown),
        };
        if self.0.revoked.lock().expect("lock").contains(&w) {
            return Err(AuthError::Revoked);
        }
        Ok(Principal {
            worker: w,
            project,
            kind: WorkerKind::Worker,
            role: Role::Member(None),
            token: id("0193e6a0-0000-7000-8000-0000000000aa", TokenId::parse),
            expires_at: Timestamp::from_unix_micros(4_102_444_799_000_000),
            renew_due: false,
        })
    }
}

struct Policy(Arc<World>);
impl ScopePolicy for Policy {
    fn allows(&self, p: &Principal, scope: &ScopeName) -> bool {
        !self.0.no_scope.lock().expect("lock").contains(&p.worker)
            && matches!(scope.as_str(), "probe:read" | "probe:write")
    }
    // The real policy answers true for rule None (every write_open route asks it); SelfWorker is
    // allowed unless the world closed the gate for that worker; mod-policy's other rules are not
    // modelled here.
    fn target(&self, p: &Principal, rule: TargetRule, _t: &Target) -> bool {
        match rule {
            TargetRule::None => true,
            TargetRule::SelfWorker => !self.0.gate_closed.lock().expect("lock").contains(&p.worker),
            _ => false,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Touch {
    v: String,
    #[serde(default)]
    fail: bool,
    #[serde(default)]
    wait_ms: u64,
    /// Wave 1 item C (H-19): panic after the write.
    #[serde(default)]
    panic: bool,
}

const PROBE_FAILED: ProblemSpec = ProblemSpec {
    code: "probe_failed",
    status: 409,
    title: "Probe failed",
    fix: "Send the request without fail.",
};

/// A step-0 refusal from a gated route's `authorize` (422), before any target read.
const PROBE_SHAPE: ProblemSpec = ProblemSpec {
    code: "probe_shape",
    status: 422,
    title: "Probe shape",
    fix: "Send a v that is not bad-shape.",
};

/// A gated route's `authorize` (delta 4): step-0 422, then the target check against the caller
/// itself (rule `SelfWorker`), which the world can close between two calls. It only reads.
fn gate(ctx: &Ctx, b: &Touch) -> Result<Authorized<()>, Problem> {
    if b.v == "bad-shape" {
        return Err(Problem::new(
            &PROBE_SHAPE,
            "The v field has the refused shape.",
        ));
    }
    ctx.authorize(
        &Target {
            worker: Some(ctx.principal().worker),
            ..Target::default()
        },
        (),
    )
}

fn broken() -> Problem {
    Problem {
        code: "internal_error",
        status: 500,
        title: "Internal error",
        detail: "The probe's SQL failed.".into(),
        fix: "Report the request ID.",
        scope: None,
    }
}

/// One row in `test_a.t` under the transaction's project (row-level security checks it).
async fn insert(tx: &mut ModuleTx, v: &str) -> Result<i64, Problem> {
    let conn = tx.executor().map_err(|_| broken())?;
    sqlx::query_scalar(
        "INSERT INTO test_a.t (project_id, v) \
         VALUES (nullif(current_setting('cortex.project_id', true), '')::uuid, $1) RETURNING id",
    )
    .bind(v)
    .fetch_one(conn)
    .await
    .map_err(|_| broken())
}

fn probe() -> Routes {
    Routes::new()
        .write_open(Method::POST, "/v2/probe/{id}/touch", "probe:write", |tx, _ctx, b: Touch| {
            Box::pin(async move {
                let row = insert(tx, &b.v).await?;
                if b.wait_ms > 0 {
                    REACHED.lock().expect("lock").push(b.v.clone());
                    tokio::time::sleep(Duration::from_millis(b.wait_ms)).await;
                }
                if b.fail {
                    return Err(Problem::new(&PROBE_FAILED, "The probe was told to fail after its write."));
                }
                if b.panic {
                    panic!("canary_h19_write_do_not_leak");
                }
                Ok(Reply::created(json!({ "row": row, "v": b.v })))
            })
        })
        .max_body(1024)
        // R-5 for writes: the handler reads its path id; malformed is 404 before any write.
        .write_open(Method::POST, "/v2/probe/item/{id}/mark", "probe:write", |tx, ctx, b: Touch| {
            Box::pin(async move {
                let id = ctx.path_id::<HandoffId>("id")?;
                insert(tx, &b.v).await?;
                Ok(Reply::created(json!({ "id": id.to_string(), "v": b.v })))
            })
        })
        .write_open(Method::POST, "/v2/probe/issue", "probe:write", |tx, _ctx, b: Touch| {
            Box::pin(async move {
                insert(tx, &b.v).await?;
                Ok(Reply {
                    status: Status::Created,
                    body: json!({ "secret": SECRET_CANARY, "token_id": issued_token().to_string() }),
                    issued: Some(issued_token()),
                })
            })
        })
        // Delta 4 (R-19): gated writes. `authorize` reads and decides; `execute` writes.
        .write(
            Method::POST,
            "/v2/gated/touch",
            "probe:write",
            TargetRule::SelfWorker,
            |_tx, ctx, b: &Touch| Box::pin(async move { gate(ctx, b) }),
            |tx, _ctx, _auth: Authorized<()>, b: Touch| {
                Box::pin(async move {
                    let row = insert(tx, &b.v).await?;
                    Ok(Reply::created(json!({ "row": row, "v": b.v })))
                })
            },
        )
        .write(
            Method::POST,
            "/v2/gated/issue",
            "probe:write",
            TargetRule::SelfWorker,
            |_tx, ctx, b: &Touch| Box::pin(async move { gate(ctx, b) }),
            |tx, _ctx, _auth: Authorized<()>, b: Touch| {
                Box::pin(async move {
                    insert(tx, &b.v).await?;
                    Ok(Reply {
                        status: Status::Created,
                        body: json!({ "secret": SECRET_CANARY, "token_id": issued_token().to_string() }),
                        issued: Some(issued_token()),
                    })
                })
            },
        )
        // A BREACH of the rule that `authorize` takes no lock: it takes an advisory lock, which the
        // chain's ledger check must refuse as a 500 (it would otherwise make the lookup's
        // Idempotency lock fail on the lock order).
        .write(
            Method::POST,
            "/v2/gated/locks",
            "probe:write",
            TargetRule::SelfWorker,
            |tx, ctx, b: &Touch| {
                Box::pin(async move {
                    tx.advisory_lock(AdvisoryClass::ProducerGate, 7)
                        .await
                        .map_err(Problem::from)?;
                    gate(ctx, b)
                })
            },
            |tx, _ctx, _auth: Authorized<()>, b: Touch| {
                Box::pin(async move {
                    insert(tx, &b.v).await?;
                    Ok(Reply::created(json!({ "v": b.v })))
                })
            },
        )
        .read(
            Method::GET,
            "/v2/probe/count/{v}",
            "probe:read",
            TargetRule::None,
            |State(st): State<AppState>, ctx: Ctx, Path(v): Path<String>| async move {
                let module = ModuleId::parse(MODULE).expect("a module id");
                let mut tx = ctx.begin(&st.pool, &module).await.map_err(|_| broken())?;
                let n: i64 = sqlx::query_scalar("SELECT count(*) FROM test_a.t WHERE v = $1")
                    .bind(&v)
                    .fetch_one(tx.executor().map_err(|_| broken())?)
                    .await
                    .map_err(|_| broken())?;
                tx.rollback().await.ok();
                Ok::<String, Problem>(n.to_string())
            },
        )
}

async fn ready() -> &'static TestDb {
    let db = test_db().await;
    let mut owner = OwnerConn::connect(&db.owner_config()).await.expect("owner");
    apply(&mut owner, &platform_migrations())
        .await
        .expect("platform migrations");
    db
}

async fn pool(db: &TestDb) -> DbPool {
    DbPool::connect(&db.db_config(cortex_testkit::pg::MODULES, Arc::new(SystemClock)))
        .await
        .expect("a pool")
}

struct Harness {
    app: Router,
    world: Arc<World>,
    db: &'static TestDb,
    /// The chain's log lines, captured (delta 4: one line per refusal of 400 or more).
    log: Arc<Mutex<Vec<String>>>,
}

#[derive(Debug)]
struct Answer {
    status: StatusCode,
    headers: HeaderMap,
    text: String,
    json: serde_json::Value,
}

impl Harness {
    async fn new() -> Harness {
        Harness::with_limits(Limits::default()).await
    }

    async fn with_limits(limits: Limits) -> Harness {
        let db = ready().await;
        let world = Arc::new(World::default());
        let log: Arc<Mutex<Vec<String>>> = Arc::default();
        let into = log.clone();
        let chain = Chain {
            verifier: Arc::new(Verifier(world.clone())),
            policy: Arc::new(Policy(world.clone())),
            log: LogSink::new(move |l| into.lock().expect("lock").push(l.to_owned())),
            limits,
        };
        let state = AppState {
            pool: Arc::new(pool(db).await),
        };
        let ids = RequestIds {
            clock: Arc::new(SystemClock),
        };
        let app = router(chain, state, ids, vec![(MODULE, probe())]).expect("a router");
        Harness {
            app,
            world,
            db,
            log,
        }
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        token: &str,
        key: Option<&str>,
        body: String,
    ) -> Answer {
        let mut req = Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json");
        if let Some(k) = key {
            req = req.header(idem::HEADER, k);
        }
        let resp = self
            .app
            .clone()
            .oneshot(req.body(Body::from(body)).expect("a request"))
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

    async fn touch(&self, object: &str, token: &str, key: &str, body: serde_json::Value) -> Answer {
        self.call(
            Method::POST,
            &format!("/v2/probe/{object}/touch"),
            token,
            Some(key),
            body.to_string(),
        )
        .await
    }

    /// Rows with this marker, read as the superuser (no row-level security in the way).
    async fn rows(&self, v: &str) -> i64 {
        let mut admin = self.db.admin().await;
        sqlx::query("SELECT count(*) FROM test_a.t WHERE v = $1")
            .bind(v)
            .fetch_one(&mut admin)
            .await
            .expect("count")
            .get(0)
    }

    /// How many idempotency rows (reservations or finished) exist for a key. A direct count, not
    /// `stored()`, which cannot tell a missing row from a reserved one whose response is NULL.
    async fn reservations(&self, key: &str) -> i64 {
        let mut admin = self.db.admin().await;
        sqlx::query("SELECT count(*) FROM platform.idempotency WHERE key = $1")
            .bind(key)
            .fetch_one(&mut admin)
            .await
            .expect("idempotency count")
            .get(0)
    }

    /// How many of a key's idempotency rows are still PENDING: reserved, response NULL. With
    /// `reservations()` this separates the three states a key can be in (Wave 1 item A, delta 2
    /// limit a): no row, a pending row, a finished row.
    async fn pending(&self, key: &str) -> i64 {
        let mut admin = self.db.admin().await;
        sqlx::query("SELECT count(*) FROM platform.idempotency WHERE key = $1 AND response IS NULL")
            .bind(key)
            .fetch_one(&mut admin)
            .await
            .expect("idempotency pending count")
            .get(0)
    }

    /// The log lines so far.
    fn lines(&self) -> Vec<String> {
        self.log.lock().expect("lock").clone()
    }

    /// The stored idempotency response for a key, as text, if any.
    async fn stored(&self, key: &str) -> Option<String> {
        let mut admin = self.db.admin().await;
        sqlx::query("SELECT response::text FROM platform.idempotency WHERE key = $1")
            .bind(key)
            .fetch_optional(&mut admin)
            .await
            .expect("idempotency rows")
            .and_then(|r| r.get::<Option<String>, _>(0))
    }
}

const OBJ_A: &str = "0193e6a0-0000-7000-8000-00000000c0a1";
const OBJ_B: &str = "0193e6a0-0000-7000-8000-00000000c0b1";

fn replayed(a: &Answer) -> bool {
    a.headers.get(REPLAY_HEADER).is_some_and(|v| v == "true")
}

fn request_id(a: &Answer) -> String {
    a.headers["cortex-request-id"]
        .to_str()
        .expect("ASCII")
        .to_owned()
}

#[tokio::test]
async fn h22_a_committed_write_replays_with_a_new_request_id_and_runs_once() {
    within(60, async {
        let h = Harness::new().await;
        let first = h
            .touch(OBJ_A, A1, "h22-key-00000001", json!({ "v": "h22" }))
            .await;
        assert_eq!(first.status, StatusCode::CREATED, "{}", first.text);
        assert!(!replayed(&first));
        let again = h
            .touch(OBJ_A, A1, "h22-key-00000001", json!({ "v": "h22" }))
            .await;
        assert_eq!(
            (again.status, &again.json),
            (StatusCode::CREATED, &first.json)
        );
        assert!(replayed(&again), "the second answer is a replay");
        assert_ne!(request_id(&first), request_id(&again));
        assert_eq!(h.rows("h22").await, 1, "the handler ran once");
    })
    .await
}

#[tokio::test]
async fn h23_the_same_value_re_serialised_replays_and_another_value_is_422() {
    within(60, async {
        let h = Harness::new().await;
        let key = "h23-key-00000001";
        let path = format!("/v2/probe/{OBJ_A}/touch");
        let first = h
            .call(
                Method::POST,
                &path,
                A1,
                Some(key),
                r#"{"v":"h23","fail":false}"#.into(),
            )
            .await;
        assert_eq!(first.status, StatusCode::CREATED, "{}", first.text);
        let same = h
            .call(
                Method::POST,
                &path,
                A1,
                Some(key),
                " { \"fail\" : false ,\n \"v\" : \"h23\" } ".into(),
            )
            .await;
        assert!(
            replayed(&same) && same.status == StatusCode::CREATED,
            "{same:?}"
        );
        let other = h
            .call(
                Method::POST,
                &path,
                A1,
                Some(key),
                r#"{"v":"h23-other"}"#.into(),
            )
            .await;
        assert_eq!(
            (other.status.as_u16(), other.json["type"].as_str()),
            (422, Some("idempotency_key_reused"))
        );
        assert_eq!((h.rows("h23").await, h.rows("h23-other").await), (1, 0));
    })
    .await
}

#[tokio::test]
async fn h24_one_key_on_two_objects_is_refused_not_answered_with_the_first() {
    within(60, async {
        let h = Harness::new().await;
        let key = "h24-key-00000001";
        let a = h.touch(OBJ_A, A1, key, json!({ "v": "h24" })).await;
        assert_eq!(a.status, StatusCode::CREATED, "{}", a.text);
        let b = h.touch(OBJ_B, A1, key, json!({ "v": "h24" })).await;
        assert_eq!(
            (b.status.as_u16(), b.json["type"].as_str()),
            (422, Some("idempotency_key_reused")),
            "{}",
            b.text
        );
        assert_eq!(h.rows("h24").await, 1);
    })
    .await
}

/// H-24's control, shown rather than mutated: bound to the TEMPLATE, the same key and body on a
/// second object WOULD replay the first object's reply. This is why the chain binds the concrete path.
#[tokio::test]
async fn h24_control_a_template_binding_would_replay_the_other_objects_reply() {
    within(60, async {
        let db = ready().await;
        let pool = pool(db).await;
        let key = IdemKey::parse("h24-control-0001").expect("a key");
        let (_, hash) = idem::body_hash(br#"{"v":"h24c"}"#).expect("JSON");
        let template = "POST /v2/probe/{id}/touch";
        let mut tx = ModuleTx::begin(&pool, ctx(MODULE, Some(project_a())), TxOptions::default())
            .await
            .expect("begin");
        let Idem::Proceed(t) = idem_begin(&mut tx, worker(1), &key, template, hash)
            .await
            .expect("first")
        else {
            panic!("a fresh key proceeds")
        };
        idem_finish(
            &mut tx,
            t,
            201,
            StoredResponse::Body(json!({ "object": "A" })),
        )
        .await
        .expect("finish");
        tx.commit().await.expect("commit");
        let mut tx = ModuleTx::begin(&pool, ctx(MODULE, Some(project_a())), TxOptions::default())
            .await
            .expect("begin");
        let second = idem_begin(&mut tx, worker(1), &key, template, hash)
            .await
            .expect("second");
        tx.rollback().await.ok();
        assert!(
            matches!(second, Idem::Replay { .. }),
            "a template binding replays across objects: {second:?}"
        );
    })
    .await
}

#[tokio::test]
async fn h25_a_replay_passes_the_token_and_scope_checks_again() {
    within(60, async {
        let h = Harness::new().await;
        let key = "h25-key-00000001";
        let body = json!({ "v": "h25" });
        assert_eq!(
            h.touch(OBJ_A, A1, key, body.clone()).await.status,
            StatusCode::CREATED
        );
        h.world.no_scope.lock().expect("lock").insert(worker(1));
        let denied = h.touch(OBJ_A, A1, key, body.clone()).await;
        assert_eq!(
            (denied.status.as_u16(), denied.json["type"].as_str()),
            (403, Some("missing_scope")),
            "not the stored reply"
        );
        h.world.no_scope.lock().expect("lock").clear();
        h.world.revoked.lock().expect("lock").insert(worker(1));
        let revoked = h.touch(OBJ_A, A1, key, body.clone()).await;
        assert_eq!(
            (revoked.status.as_u16(), revoked.json["type"].as_str()),
            (401, Some("token_revoked"))
        );
        h.world.revoked.lock().expect("lock").clear();
        let back = h.touch(OBJ_A, A1, key, body).await;
        assert!(
            back.status == StatusCode::CREATED && replayed(&back),
            "{back:?}"
        );
        assert_eq!(h.rows("h25").await, 1);
    })
    .await
}

#[tokio::test]
async fn h26_two_requests_at_once_with_one_key_have_one_winner() {
    within(60, async {
        let h = Harness::new().await;
        let body = json!({ "v": "h26", "wait_ms": 300 });
        let (x, y) = tokio::join!(
            h.touch(OBJ_A, A1, "h26-key-00000001", body.clone()),
            h.touch(OBJ_A, A1, "h26-key-00000001", body.clone()),
        );
        assert_eq!(
            (x.status, y.status),
            (StatusCode::CREATED, StatusCode::CREATED),
            "{x:?} {y:?}"
        );
        assert_eq!(x.json, y.json, "both get the winner's reply");
        assert_eq!(
            u8::from(replayed(&x)) + u8::from(replayed(&y)),
            1,
            "exactly one replay"
        );
        assert_eq!(h.rows("h26").await, 1, "one effect");
        let other = json!({ "v": "h26-two" });
        h.touch(OBJ_A, A1, "h26-key-00000002", other.clone()).await;
        h.touch(OBJ_A, A1, "h26-key-00000003", other).await;
        assert_eq!(h.rows("h26-two").await, 2, "two keys are two operations");
    })
    .await
}

#[tokio::test]
async fn h27_a_reply_with_a_secret_is_never_stored_and_a_retry_is_409() {
    within(60, async {
        let h = Harness::new().await;
        let key = "h27-key-00000001";
        let path = "/v2/probe/issue";
        let first = h
            .call(
                Method::POST,
                path,
                A1,
                Some(key),
                json!({ "v": "h27" }).to_string(),
            )
            .await;
        assert_eq!(first.status, StatusCode::CREATED, "{}", first.text);
        assert_eq!(
            first.json["secret"], SECRET_CANARY,
            "the first answer carries it once"
        );
        let stored = h.stored(key).await.expect("a stored row");
        assert!(
            stored.contains("secret_not_stored") && !stored.contains(SECRET_CANARY),
            "{stored}"
        );
        let retry = h
            .call(
                Method::POST,
                path,
                A1,
                Some(key),
                json!({ "v": "h27" }).to_string(),
            )
            .await;
        assert_eq!(
            (retry.status.as_u16(), retry.json["type"].as_str()),
            (409, Some("already_issued"))
        );
        assert!(
            retry.json["detail"]
                .as_str()
                .unwrap_or("")
                .contains(&issued_token().to_string())
        );
        assert!(!retry.text.contains(SECRET_CANARY));
    })
    .await
}

#[tokio::test]
async fn h28_h14_a_refused_write_leaves_no_row_and_no_reservation() {
    within(60, async {
        let h = Harness::new().await;
        let key = "h28-key-00000001";
        let failed = h
            .touch(OBJ_A, A1, key, json!({ "v": "h28", "fail": true }))
            .await;
        assert_eq!(
            (failed.status.as_u16(), failed.json["type"].as_str()),
            (409, Some("probe_failed")),
            "{}",
            failed.text
        );
        assert_eq!(
            h.rows("h28").await,
            0,
            "the handler's write rolled back (H-14)"
        );
        assert_eq!(h.stored(key).await, None, "no reservation was kept");
        assert_eq!(
            h.reservations(key).await,
            0,
            "no idempotency row at all (direct count)"
        );
        // A different body with the same key runs, instead of 422: nothing was bound to the key.
        let retry = h.touch(OBJ_A, A1, key, json!({ "v": "h28" })).await;
        assert!(
            retry.status == StatusCode::CREATED && !replayed(&retry),
            "{retry:?}"
        );
        assert_eq!(h.rows("h28").await, 1);
    })
    .await
}

#[tokio::test]
async fn h29_a_key_belongs_to_its_principal() {
    within(60, async {
        let h = Harness::new().await;
        let key = "h29-key-00000001";
        let one = h.touch(OBJ_A, A1, key, json!({ "v": "h29" })).await;
        let two = h.touch(OBJ_A, A3, key, json!({ "v": "h29" })).await;
        assert_eq!(
            (one.status, two.status),
            (StatusCode::CREATED, StatusCode::CREATED)
        );
        assert!(!replayed(&one) && !replayed(&two));
        assert_eq!(h.rows("h29").await, 2);
    })
    .await
}

#[tokio::test]
async fn p06_another_projects_token_sees_none_of_the_rows() {
    within(60, async {
        let h = Harness::new().await;
        assert_eq!(
            h.touch(OBJ_A, A1, "p06-key-00000001", json!({ "v": "p06" }))
                .await
                .status,
            StatusCode::CREATED
        );
        let mine = h
            .call(Method::GET, "/v2/probe/count/p06", A1, None, String::new())
            .await;
        let theirs = h
            .call(Method::GET, "/v2/probe/count/p06", B4, None, String::new())
            .await;
        assert_eq!((mine.status, mine.text.as_str()), (StatusCode::OK, "1"));
        assert_eq!(
            (theirs.status, theirs.text.as_str()),
            (StatusCode::OK, "0"),
            "row-level security hides A's row from B"
        );
    })
    .await
}

#[tokio::test]
async fn r5_a_write_handler_sees_its_path_id_and_a_malformed_one_is_404() {
    within(60, async {
        let h = Harness::new().await;
        let good = "0193e6a0-0000-7000-8000-0000000000e1";
        let ok = h
            .call(
                Method::POST,
                &format!("/v2/probe/item/{good}/mark"),
                A1,
                Some("r5w-key-00000001"),
                json!({ "v": "r5w" }).to_string(),
            )
            .await;
        assert_eq!(ok.status, StatusCode::CREATED, "{}", ok.text);
        assert_eq!(ok.json["id"], good, "the handler saw its path id");
        assert_eq!(h.rows("r5w").await, 1);
        let bad = h
            .call(
                Method::POST,
                "/v2/probe/item/abc/mark",
                A1,
                Some("r5w-key-00000002"),
                json!({ "v": "r5w-bad" }).to_string(),
            )
            .await;
        assert_eq!(
            (bad.status.as_u16(), bad.json["type"].as_str()),
            (404, Some("not_found")),
            "{}",
            bad.text
        );
        assert_eq!(h.rows("r5w-bad").await, 0, "nothing written");
        assert_eq!(
            h.stored("r5w-key-00000002").await,
            None,
            "no reservation kept"
        );
        assert_eq!(
            h.reservations("r5w-key-00000002").await,
            0,
            "no idempotency row at all (direct count; cox 88f4f430)"
        );
    })
    .await
}

// ---- Delta 4 (R-19, R-16): authorize BEFORE the lookup. The chain half; finn runs these. --------
//
// Order under test: decode, key (428/400, before any transaction), begin tx, AUTHORIZE (step-0 422,
// 404, 403), the lock ledger must be empty, LOOKUP (replay, 422 reused, 409 already_issued),
// execute, finish, commit. The gated routes' target is the caller itself (`SelfWorker`); the world
// closes the gate for one worker between two calls.

impl Harness {
    async fn gated(&self, route: &str, token: &str, key: &str, body: serde_json::Value) -> Answer {
        self.call(
            Method::POST,
            &format!("/v2/gated/{route}"),
            token,
            Some(key),
            body.to_string(),
        )
        .await
    }

    fn close_gate(&self, w: WorkerId) {
        self.world.gate_closed.lock().expect("lock").insert(w);
    }

    fn open_gate(&self, w: WorkerId) {
        self.world.gate_closed.lock().expect("lock").remove(&w);
    }
}

fn type_of(a: &Answer) -> Option<&str> {
    a.json["type"].as_str()
}

#[tokio::test]
async fn d4_r19_a_replay_never_bypasses_authorization() {
    within(60, async {
        let h = Harness::new().await;
        let key = "d4-r19-key-0000001";
        let first = h.gated("touch", A1, key, json!({ "v": "d4r19" })).await;
        assert_eq!(first.status, StatusCode::CREATED, "{}", first.text);
        assert_eq!(h.rows("d4r19").await, 1);
        assert!(h.lines().is_empty(), "a success writes no line");

        // The authority is removed AFTER the reply was stored. The same key and body must now be
        // 403, not the stored 201 (R-19; mike f923262d).
        h.close_gate(worker(1));
        let again = h.gated("touch", A1, key, json!({ "v": "d4r19" })).await;
        assert_eq!(
            (again.status.as_u16(), type_of(&again)),
            (403, Some("target_not_allowed")),
            "a replay must not bypass authorization: {}",
            again.text
        );
        assert!(!replayed(&again), "no stored reply came back");
        assert!(
            !again.text.contains("d4r19"),
            "nothing of the stored reply: {}",
            again.text
        );
        assert_eq!(h.rows("d4r19").await, 1, "nothing ran again");
        assert_eq!(
            h.reservations(key).await,
            1,
            "the finished row is untouched by the refusal"
        );

        // The refusal's log line, on the write path: the worker, the TEMPLATE, status and code.
        let lines = h.lines();
        assert_eq!(lines.len(), 1, "one line for the one refusal: {lines:?}");
        for want in [
            format!("principal={}", worker(1)),
            "method=POST".to_owned(),
            "path=/v2/gated/touch".to_owned(),
            "status=403".to_owned(),
            "code=target_not_allowed".to_owned(),
        ] {
            assert!(lines[0].contains(&want), "{want} missing from {}", lines[0]);
        }

        // MUTATION WITNESS: the primitive replays whatever the caller's authority, so the chain's
        // ORDER is the only guard. A chain that looked up before authorize would return this.
        let pool = pool(h.db).await;
        let (_, hash) =
            idem::body_hash(json!({ "v": "d4r19" }).to_string().as_bytes()).expect("JSON");
        let route = idem::route(&Method::POST, "/v2/gated/touch");
        let mut tx = ModuleTx::begin(&pool, ctx(MODULE, Some(project_a())), TxOptions::default())
            .await
            .expect("begin");
        let found = idem_begin(
            &mut tx,
            worker(1),
            &IdemKey::parse(key).expect("a key"),
            &route,
            hash,
        )
        .await
        .expect("lookup");
        tx.rollback().await.ok();
        assert!(
            matches!(found, Idem::Replay { .. }),
            "the lookup alone would replay: {found:?}"
        );

        // The authority comes back: the stored reply is still there and replays.
        h.open_gate(worker(1));
        let back = h.gated("touch", A1, key, json!({ "v": "d4r19" })).await;
        assert!(
            back.status == StatusCode::CREATED && replayed(&back),
            "{back:?}"
        );
    })
    .await
}

#[tokio::test]
async fn d4_an_authorize_refusal_leaves_no_reservation_and_no_row() {
    within(60, async {
        let h = Harness::new().await;
        // Step-0 422, before any target read.
        let k1 = "d4-shape-key-000001";
        let shaped = h.gated("touch", A1, k1, json!({ "v": "bad-shape" })).await;
        assert_eq!(
            (shaped.status.as_u16(), type_of(&shaped)),
            (422, Some("probe_shape")),
            "{}",
            shaped.text
        );
        assert_eq!(
            h.reservations(k1).await,
            0,
            "no reservation after a step-0 422"
        );
        assert_eq!(h.rows("bad-shape").await, 0);

        // 403 from the target check.
        h.close_gate(worker(3));
        let k2 = "d4-gate-key-0000001";
        let denied = h.gated("touch", A3, k2, json!({ "v": "d4gate" })).await;
        assert_eq!(
            (denied.status.as_u16(), type_of(&denied)),
            (403, Some("target_not_allowed")),
            "{}",
            denied.text
        );
        assert_eq!(h.reservations(k2).await, 0, "no reservation after a 403");
        assert_eq!(h.rows("d4gate").await, 0);

        // The key was never bound: with the authority back, a DIFFERENT body runs on it. A leftover
        // reservation would make this a 422 idempotency_key_reused.
        h.open_gate(worker(3));
        let ok = h.gated("touch", A3, k2, json!({ "v": "d4gate-ok" })).await;
        assert!(ok.status == StatusCode::CREATED && !replayed(&ok), "{ok:?}");
        assert_eq!(h.rows("d4gate-ok").await, 1);
    })
    .await
}

#[tokio::test]
async fn d4_reused_and_already_issued_land_after_authorize() {
    within(60, async {
        let h = Harness::new().await;
        // idempotency_key_reused (422): authorized, it is 422; once de-authorized it is 403.
        let k = "d4-reuse-key-000001";
        let a = h.gated("touch", A1, k, json!({ "v": "d4re1" })).await;
        assert_eq!(a.status, StatusCode::CREATED, "{}", a.text);
        let b = h.gated("touch", A1, k, json!({ "v": "d4re2" })).await;
        assert_eq!(
            (b.status.as_u16(), type_of(&b)),
            (422, Some("idempotency_key_reused")),
            "{}",
            b.text
        );
        h.close_gate(worker(1));
        let c = h.gated("touch", A1, k, json!({ "v": "d4re2" })).await;
        assert_eq!(
            (c.status.as_u16(), type_of(&c)),
            (403, Some("target_not_allowed")),
            "reused only AFTER the target check (kai): {}",
            c.text
        );
        assert_eq!(h.rows("d4re2").await, 0);

        // already_issued (409): the secret-issuing route, on another worker.
        let ki = "d4-issue-key-000001";
        let i1 = h.gated("issue", A3, ki, json!({ "v": "d4is" })).await;
        assert_eq!(i1.status, StatusCode::CREATED, "{}", i1.text);
        assert!(
            i1.text.contains(SECRET_CANARY),
            "the first answer carries the secret"
        );
        let i2 = h.gated("issue", A3, ki, json!({ "v": "d4is" })).await;
        assert_eq!(
            (i2.status.as_u16(), type_of(&i2)),
            (409, Some("already_issued")),
            "{}",
            i2.text
        );
        assert!(
            !i2.text.contains(SECRET_CANARY),
            "the secret is never stored or replayed"
        );
        h.close_gate(worker(3));
        let i3 = h.gated("issue", A3, ki, json!({ "v": "d4is" })).await;
        assert_eq!(
            (i3.status.as_u16(), type_of(&i3)),
            (403, Some("target_not_allowed")),
            "403 comes before the 409: {}",
            i3.text
        );
    })
    .await
}

#[tokio::test]
async fn d4_a_lock_taken_in_authorize_is_a_500_with_a_log_line_naming_it() {
    within(60, async {
        let h = Harness::new().await;
        let key = "d4-lock-key-0000001";
        let r = h.gated("locks", A1, key, json!({ "v": "d4lock" })).await;
        assert_eq!(
            (r.status.as_u16(), type_of(&r)),
            (500, Some("internal_error")),
            "a lock in authorize is a bug the chain refuses: {}",
            r.text
        );
        assert_eq!(h.rows("d4lock").await, 0, "nothing was written");
        assert_eq!(h.reservations(key).await, 0, "no reservation");
        let lines = h.lines();
        assert_eq!(lines.len(), 1, "{lines:?}");
        for want in ["status=500", "path=/v2/gated/locks", "ProducerGate"] {
            assert!(lines[0].contains(want), "{want} missing from {}", lines[0]);
        }
    })
    .await
}

// ---- Wave 1, item A (delta 2 limit a): the direct reservation-row assertion ----------------------
//
// `stored(key)` answers `None` both for NO row and for a row that is reserved but not finished (its
// response is NULL), so an assertion on it alone cannot say "nothing was reserved". The direct
// counts can. This cell makes the three states of a key observable and shows the old helper's blind
// spot as a control: the same `None` for two different states.

#[tokio::test]
async fn w1_a_the_row_counts_tell_no_row_from_a_pending_row_from_a_finished_one() {
    within(60, async {
        let h = Harness::new().await;

        // 1. NO ROW.
        let none = "w1a-none-key-0000001";
        assert_eq!(
            (
                h.reservations(none).await,
                h.pending(none).await,
                h.stored(none).await
            ),
            (0, 0, None),
            "a key nobody used"
        );

        // 2. A PENDING ROW: reserved and committed WITHOUT finish. The chain never leaves one (a
        // refusal rolls the reservation back), so the cell makes one by hand.
        let pending = "w1a-pending-key-00001";
        let pool = pool(h.db).await;
        let (_, hash) =
            idem::body_hash(json!({ "v": "w1a" }).to_string().as_bytes()).expect("JSON");
        let route = idem::route(&Method::POST, "/v2/probe/item/w1a/touch");
        let mut tx = ModuleTx::begin(&pool, ctx(MODULE, Some(project_a())), TxOptions::default())
            .await
            .expect("begin");
        let Idem::Proceed(_unfinished) = idem_begin(
            &mut tx,
            worker(1),
            &IdemKey::parse(pending).expect("a key"),
            &route,
            hash,
        )
        .await
        .expect("reserve") else {
            panic!("a fresh key proceeds")
        };
        tx.commit()
            .await
            .expect("commit the reservation without finishing it");
        assert_eq!(
            (h.reservations(pending).await, h.pending(pending).await),
            (1, 1),
            "the reservation exists and is pending"
        );
        // THE CONTROL: the old helper cannot tell this row from state 1.
        assert_eq!(
            h.stored(pending).await,
            h.stored(none).await,
            "stored() answers None for a pending row exactly as for no row: it is blind here"
        );

        // 3. A FINISHED ROW: a committed write through the chain.
        let finished = "w1a-finished-key-0001";
        let r = h
            .touch(OBJ_A, A1, finished, json!({ "v": "w1a-fin" }))
            .await;
        assert_eq!(r.status, StatusCode::CREATED, "{}", r.text);
        assert_eq!(
            (h.reservations(finished).await, h.pending(finished).await),
            (1, 0),
            "one row, and it is not pending"
        );
        assert!(h.stored(finished).await.is_some(), "its reply is stored");
    })
    .await
}

// ---- Wave 1 item C: H-19 on the write path (DB) ---------------------------------------------------

#[tokio::test]
async fn h19_a_write_handler_that_panics_leaves_no_row_and_no_reservation_and_the_next_request_is_served()
 {
    within(60, async {
        let h = Harness::new().await;
        let key = "h19-panic-key-000001";
        let r = h
            .touch(OBJ_A, A1, key, json!({ "v": "h19", "panic": true }))
            .await;
        assert_eq!(
            (r.status.as_u16(), r.json["type"].as_str()),
            (500, Some("internal_error")),
            "{}",
            r.text
        );
        assert!(
            !r.text.contains("canary_h19"),
            "the panic text reached the client: {}",
            r.text
        );
        assert_eq!(h.rows("h19").await, 0, "the row the handler wrote is gone");
        assert_eq!(
            h.reservations(key).await,
            0,
            "no reservation survives a panic"
        );

        // The next request is served, on the same pool and the same router.
        let next = h
            .touch(OBJ_A, A1, "h19-next-key-000001", json!({ "v": "h19-next" }))
            .await;
        assert_eq!(next.status, StatusCode::CREATED, "{}", next.text);
        assert_eq!(h.rows("h19-next").await, 1);

        let lines = h.lines();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].contains("status=500") && lines[0].contains(" step=handler cause=panic"),
            "{}",
            lines[0]
        );
        assert!(
            !lines[0].contains("canary_h19"),
            "the panic text reached the log: {}",
            lines[0]
        );
    })
    .await
}

// ---- Wave 1 item E: the request timeout on the write path (DB) ------------------------------------

#[tokio::test]
async fn e_a_write_past_the_timeout_leaves_no_row_and_no_reservation_and_the_next_is_served() {
    within(60, async {
        let h = Harness::with_limits(Limits {
            request_timeout: Duration::from_millis(1000),
            ..Limits::default()
        })
        .await;
        let key = "e-timeout-key-000001";
        // The handler writes its row, then sleeps 5 s: the timeout (1 s) cancels it mid-transaction.
        let r = h
            .touch(OBJ_A, A1, key, json!({ "v": "e-slow", "wait_ms": 5000 }))
            .await;
        assert_eq!(
            (r.status.as_u16(), r.json["type"].as_str()),
            (503, Some("unavailable")),
            "{}",
            r.text
        );
        // THE BARRIER: the handler did reach its wait, AFTER its insert, before the deadline.
        assert!(
            REACHED.lock().expect("lock").iter().any(|v| v == "e-slow"),
            "the handler never reached its post-insert wait: 'no row' would be true for the wrong reason"
        );
        assert_eq!(
            h.rows("e-slow").await,
            0,
            "the row the handler wrote is gone"
        );
        assert_eq!(
            h.reservations(key).await,
            0,
            "no reservation survives a timeout"
        );
        let lines = h.lines();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(
            lines[0].contains("status=503")
                && lines[0].contains(" step=timeout cause=request_timeout"),
            "{}",
            lines[0]
        );

        // The next request is served on the same pool.
        let next = h
            .touch(OBJ_A, A1, "e-next-key-0000001", json!({ "v": "e-next" }))
            .await;
        assert_eq!(next.status, StatusCode::CREATED, "{}", next.text);
        assert_eq!(h.rows("e-next").await, 1);
    })
    .await
}
