//! Wave 0 S0 delta 4 (cox c025db40; kai 68cecbdb), DB-backed: the erased `authorize` and `execute`
//! closures of a two-phase write route run with a REAL `ModuleTx`, through the plug's identity
//! provider. This is delta 4's ONE database run (S0's half; mira's chain cells are hers).
//!
//! What it shows (the chain's assumptions about the S0 closures):
//! - `authorize` reads through `IdentityApi` inside the caller's transaction and takes **no lock**:
//!   the lock ledger is empty afterwards (the chain asserts this; a lock taken in `authorize` would
//!   make the later Idempotency lock fail with `LockOrder`);
//! - a control: an `authorize` that DOES take an advisory lock leaves a step in the ledger, so the
//!   chain's check can see it;
//! - the order inside `authorize` is the piece's: a target of another project is 404 before the
//!   policy is asked; a refusal is 403 `target_not_allowed` and `execute` cannot run (no proof);
//! - the rule the policy sees is the route's DECLARED one; `write_open` is rule `None`;
//! - an `Authorization` made for one request is refused by `execute` under another (500);
//! - (rework, vera D4-V1) a handler that REPLACES the route authority (another rule and policy, the
//!   same rule with a permissive policy, a `Ctx` rebuilt with `Ctx::new`) cannot mint a proof the
//!   chain accepts (500 from the erased `authorize`, so neither replay nor `execute` can follow), and
//!   an altered `principal` field in a clone changes nothing the policy sees (403 from the original).
//!
//! Needs `supervise.sh`'s throwaway Postgres (CV2_PG_PORT, CV2_PG_PASSWORD). Default features.
//! RED: does not compile (`Routes::write` takes one closure; `Ctx::with_route`'s proof types are stubs).
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::http::Method;
use cortex_db::migrate::{apply, platform_migrations};
use cortex_db::{AdvisoryClass, DbPool, LockStep, ModuleTx, OwnerConn};
use cortex_module::route::{AuthorizeFn, DecodeFn, ExecuteFn, Handler};
use cortex_module::{
    Authorized, Ctx, IdentityApi, Piece, Principal, Problem, Reply, Role, Routes, ScopePolicy,
    Target, TargetRule, WorkerKind,
};
use cortex_testkit::empty_db;
use cortex_types::{
    ModuleId, ProjectId, RequestId, ScopeName, SystemClock, Timestamp, TokenId, WorkerId,
};
use plug_identity::{BOOTSTRAP_SQL, PlugIdentity, Provider};
use serde::Deserialize;

/// A caller module from the testkit's roles (`cortex_m_test_a`).
const CALLER: &str = "test_a";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Touch {
    worker: WorkerId,
}

struct Fixture {
    a: ProjectId,
    b: ProjectId,
    a_lead: WorkerId,
    b_worker: WorkerId,
}

/// Answers a fixed bool and remembers every question (rule and target).
struct Spy {
    answer: bool,
    asked: Mutex<Vec<(TargetRule, Target)>>,
}

impl Spy {
    fn new(answer: bool) -> Arc<Spy> {
        Arc::new(Spy {
            answer,
            asked: Mutex::new(Vec::new()),
        })
    }
    fn asked(&self) -> Vec<(TargetRule, Target)> {
        self.asked.lock().expect("spy lock").clone()
    }
}

impl ScopePolicy for Spy {
    fn allows(&self, _p: &Principal, _scope: &ScopeName) -> bool {
        true
    }
    fn target(&self, _p: &Principal, rule: TargetRule, t: &Target) -> bool {
        self.asked.lock().expect("spy lock").push((rule, t.clone()));
        self.answer
    }
}

async fn setup(tag: &str) -> (cortex_testkit::TestDb, Fixture, DbPool, Arc<Provider>) {
    let db = empty_db(tag).await;
    let mut admin = db.admin().await;
    sqlx::raw_sql(BOOTSTRAP_SQL)
        .execute(&mut admin)
        .await
        .expect("bootstrap");
    let mut owner = OwnerConn::connect(&db.owner_config()).await.expect("owner");
    apply(&mut owner, &platform_migrations())
        .await
        .expect("platform");
    apply(&mut owner, &PlugIdentity::new().migrations())
        .await
        .expect("plug_identity migrations");
    let c = SystemClock;
    let f = Fixture {
        a: ProjectId::new(&c),
        b: ProjectId::new(&c),
        a_lead: WorkerId::new(&c),
        b_worker: WorkerId::new(&c),
    };
    sqlx::query(
        "INSERT INTO plug_identity.projects (id, name, lead) VALUES ($1::uuid, 'A', $2::uuid), ($3::uuid, 'B', $4::uuid)",
    )
    .bind(f.a.to_string())
    .bind(f.a_lead.to_string())
    .bind(f.b.to_string())
    .bind(f.b_worker.to_string())
    .execute(&mut admin)
    .await
    .expect("seed projects");
    sqlx::query(
        "INSERT INTO plug_identity.workers (id, project_id, name, kind, role, team_id, job_roles) VALUES \
         ($1::uuid, $2::uuid, 'a-lead', 'worker', 'project_lead', NULL, '{lead}'), \
         ($3::uuid, $4::uuid, 'b-worker', 'worker', 'project_lead', NULL, '{}')",
    )
    .bind(f.a_lead.to_string())
    .bind(f.a.to_string())
    .bind(f.b_worker.to_string())
    .bind(f.b.to_string())
    .execute(&mut admin)
    .await
    .expect("seed workers");
    let pool = DbPool::connect(&db.db_config(&[CALLER, plug_identity::ID], Arc::new(SystemClock)))
        .await
        .expect("pool");
    (db, f, pool, Arc::new(Provider::new()))
}

fn principal(f: &Fixture) -> Principal {
    Principal {
        worker: f.a_lead,
        project: f.a,
        kind: WorkerKind::Worker,
        role: Role::ProjectLead,
        token: TokenId::new(&SystemClock),
        expires_at: Timestamp::from_unix_micros(0),
        renew_due: false,
    }
}

fn ctx_for(f: &Fixture, rule: TargetRule, policy: Arc<Spy>) -> Ctx {
    Ctx::new(principal(f), RequestId::new(&SystemClock)).with_route(rule, policy)
}

async fn begin(ctx: &Ctx, pool: &DbPool) -> ModuleTx {
    ctx.begin(pool, &ModuleId::parse(CALLER).expect("module id"))
        .await
        .expect("begin")
}

/// A piece's two-phase route: `authorize` reads the target through `IdentityApi` (404 if absent in
/// this project) and asks `ctx.authorize`; `execute` counts and answers 201 with the worker's id.
/// `lock_first` makes `authorize` take an advisory lock first (the control).
fn touch_routes(p: Arc<Provider>, executed: Arc<AtomicUsize>, lock_first: bool) -> Routes {
    Routes::new().write(
        Method::POST,
        "/v2/d4/touch",
        "d4:write",
        TargetRule::SelfOrProjectLead,
        move |tx, ctx, b: &Touch| {
            let p = Arc::clone(&p);
            let w = b.worker;
            Box::pin(async move {
                if lock_first {
                    tx.advisory_lock(AdvisoryClass::ProducerGate, 1)
                        .await
                        .map_err(Problem::from)?;
                }
                let row = p.worker(tx, &w).await?.ok_or_else(Problem::not_found)?;
                ctx.authorize(
                    &Target {
                        worker: Some(row.id),
                        kind: Some(row.kind),
                        team: None,
                        lead: None,
                        consult: false,
                    },
                    row.id,
                )
            })
        },
        {
            let executed = Arc::clone(&executed);
            move |_tx, _ctx, a: Authorized<WorkerId>, _b: Touch| {
                let executed = Arc::clone(&executed);
                Box::pin(async move {
                    executed.fetch_add(1, Ordering::SeqCst);
                    Ok(Reply::created(a.into_value().to_string()))
                })
            }
        },
    )
}

fn parts(r: Routes) -> (DecodeFn, AuthorizeFn, ExecuteFn, TargetRule) {
    r.into_entries()
        .into_iter()
        .find_map(|e| match e.handler {
            Handler::Write {
                decode,
                authorize,
                execute,
            } => Some((decode, authorize, execute, e.target)),
            Handler::Read(_) => None,
        })
        .expect("a write route")
}

fn body(w: WorkerId) -> Vec<u8> {
    format!(r#"{{"worker":"{w}"}}"#).into_bytes()
}

#[tokio::test]
async fn w0_d4_authorize_reads_without_a_lock_and_execute_runs_only_with_the_proof() {
    let (_db, f, pool, p) = setup("d4_flow").await;
    let spy = Spy::new(true);
    let executed = Arc::new(AtomicUsize::new(0));
    let (decode, authorize, execute, declared) =
        parts(touch_routes(p, Arc::clone(&executed), false));
    let ctx = ctx_for(&f, declared, spy.clone());
    let mut tx = begin(&ctx, &pool).await;

    let decoded = decode(&body(f.a_lead)).expect("decodes");
    let auth = authorize(&mut tx, &ctx, &decoded)
        .await
        .expect("authorized");
    assert!(
        tx.lock_ledger().steps().is_empty(),
        "authorize took no advisory lock: {:?}",
        tx.lock_ledger().steps()
    );
    assert!(auth.belongs_to(&ctx));
    assert_eq!(
        auth.rule(),
        TargetRule::SelfOrProjectLead,
        "the route's declared rule"
    );
    assert_eq!((auth.worker(), auth.project()), (f.a_lead, f.a));
    let asked = spy.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].0, TargetRule::SelfOrProjectLead);
    assert_eq!(asked[0].1.worker, Some(f.a_lead), "the row the piece read");

    let reply = execute(&mut tx, &ctx, auth, decoded)
        .await
        .expect("executed");
    assert_eq!(executed.load(Ordering::SeqCst), 1);
    assert_eq!(reply.body.as_str(), Some(f.a_lead.to_string().as_str()));
    assert_eq!(reply.status.code(), 201);
    tx.rollback().await.expect("rollback");
}

#[tokio::test]
async fn w0_d4_a_target_of_another_project_is_404_before_the_policy_is_asked() {
    let (_db, f, pool, p) = setup("d4_404").await;
    let spy = Spy::new(true);
    let (decode, authorize, _execute, declared) =
        parts(touch_routes(p, Arc::new(AtomicUsize::new(0)), false));
    let ctx = ctx_for(&f, declared, spy.clone());
    let mut tx = begin(&ctx, &pool).await;
    let decoded = decode(&body(f.b_worker)).expect("decodes");
    let err = authorize(&mut tx, &ctx, &decoded)
        .await
        .expect_err("refused");
    assert_eq!((err.status, err.code), (404, "not_found"));
    assert!(
        spy.asked().is_empty(),
        "404 comes first: the policy was not asked"
    );
    tx.rollback().await.expect("rollback");
}

#[tokio::test]
async fn w0_d4_a_refused_target_is_403_and_there_is_no_proof_to_execute_with() {
    let (_db, f, pool, p) = setup("d4_403").await;
    let spy = Spy::new(false);
    let executed = Arc::new(AtomicUsize::new(0));
    let (decode, authorize, _execute, declared) =
        parts(touch_routes(p, Arc::clone(&executed), false));
    let ctx = ctx_for(&f, declared, spy.clone());
    let mut tx = begin(&ctx, &pool).await;
    let decoded = decode(&body(f.a_lead)).expect("decodes");
    let err = authorize(&mut tx, &ctx, &decoded)
        .await
        .expect_err("refused");
    assert_eq!((err.status, err.code), (403, "target_not_allowed"));
    assert_eq!(spy.asked().len(), 1);
    assert_eq!(executed.load(Ordering::SeqCst), 0);
    tx.rollback().await.expect("rollback");
}

#[tokio::test]
async fn w0_d4_the_ledger_sees_a_lock_taken_in_authorize_so_the_chains_check_can_fire() {
    let (_db, f, pool, p) = setup("d4_ledger").await;
    let (decode, authorize, _execute, declared) =
        parts(touch_routes(p, Arc::new(AtomicUsize::new(0)), true));
    let ctx = ctx_for(&f, declared, Spy::new(true));
    let mut tx = begin(&ctx, &pool).await;
    let decoded = decode(&body(f.a_lead)).expect("decodes");
    let _auth = authorize(&mut tx, &ctx, &decoded)
        .await
        .expect("authorized");
    assert_eq!(
        tx.lock_ledger().steps(),
        [LockStep::ProducerGate],
        "the control: a lock in authorize is visible to the chain"
    );
    tx.rollback().await.expect("rollback");
}

#[tokio::test]
async fn w0_d4_a_proof_made_for_one_request_is_refused_by_execute_under_another() {
    let (_db, f, pool, p) = setup("d4_stash").await;
    let executed = Arc::new(AtomicUsize::new(0));
    let (decode, authorize, execute, declared) =
        parts(touch_routes(p, Arc::clone(&executed), false));
    let first = ctx_for(&f, declared, Spy::new(true));
    let second = ctx_for(&f, declared, Spy::new(true));
    let mut tx = begin(&first, &pool).await;
    let decoded = decode(&body(f.a_lead)).expect("decodes");
    let stashed = authorize(&mut tx, &first, &decoded)
        .await
        .expect("authorized");
    let err = execute(&mut tx, &second, stashed, decoded)
        .await
        .expect_err("refused");
    assert_eq!((err.status, err.code), (500, "internal_error"));
    assert_eq!(
        executed.load(Ordering::SeqCst),
        0,
        "execute's body did not run"
    );
    tx.rollback().await.expect("rollback");
}

#[tokio::test]
async fn w0_d4_write_open_is_rule_none_and_its_proof_is_the_default_check() {
    let (_db, f, pool, _p) = setup("d4_open").await;
    let ran = Arc::new(AtomicUsize::new(0));
    let r = {
        let ran = Arc::clone(&ran);
        Routes::new().write_open(
            Method::POST,
            "/v2/d4/open",
            "d4:write",
            move |_tx, _ctx, _b: Touch| {
                let ran = Arc::clone(&ran);
                Box::pin(async move {
                    ran.fetch_add(1, Ordering::SeqCst);
                    Ok(Reply::ok(()))
                })
            },
        )
    };
    let (decode, authorize, execute, declared) = parts(r);
    assert_eq!(declared, TargetRule::None);
    let spy = Spy::new(true);
    let ctx = ctx_for(&f, declared, spy.clone());
    let mut tx = begin(&ctx, &pool).await;
    let decoded = decode(&body(f.a_lead)).expect("decodes");
    let auth = authorize(&mut tx, &ctx, &decoded)
        .await
        .expect("authorized");
    assert_eq!(auth.rule(), TargetRule::None);
    assert!(tx.lock_ledger().steps().is_empty());
    assert_eq!(spy.asked(), [(TargetRule::None, Target::default())]);
    execute(&mut tx, &ctx, auth, decoded)
        .await
        .expect("executed");
    assert_eq!(ran.load(Ordering::SeqCst), 1);
    tx.rollback().await.expect("rollback");
}

// ---- delta 4 rework, D4-V1: a malicious `authorize` closure, through Handler::Write (public API) ----

/// A route whose `authorize` builds the `Ctx` it authorizes with by `forge`, from the one the chain
/// gave it, and whose `execute` counts.
fn forging_routes(
    forge: Arc<dyn Fn(&Ctx) -> Ctx + Send + Sync>,
    executed: Arc<AtomicUsize>,
) -> Routes {
    Routes::new().write(
        Method::POST,
        "/v2/d4/forge",
        "d4:write",
        TargetRule::SelfOrProjectLead,
        move |_tx, ctx, _b: &Touch| {
            let forge = Arc::clone(&forge);
            Box::pin(async move { forge(ctx).authorize(&Target::default(), ()) })
        },
        move |_tx, _ctx, _a: Authorized<()>, _b: Touch| {
            let executed = Arc::clone(&executed);
            Box::pin(async move {
                executed.fetch_add(1, Ordering::SeqCst);
                Ok(Reply::ok(()))
            })
        },
    )
}

/// Runs the erased `authorize` of a forging route under the chain's `Ctx` (a denying original policy)
/// and returns the refusal's (status, code), whether the lock ledger stayed empty, and how many times
/// `execute` ran (never: there is no proof to run it with).
async fn run_forging(
    tag: &str,
    member: bool,
    original: Arc<Spy>,
    forge: Arc<dyn Fn(&Ctx) -> Ctx + Send + Sync>,
) -> ((u16, &'static str), usize) {
    let (_db, f, pool, _p) = setup(tag).await;
    let executed = Arc::new(AtomicUsize::new(0));
    let (decode, authorize, _execute, declared) =
        parts(forging_routes(forge, Arc::clone(&executed)));
    let mut principal = principal(&f);
    if member {
        principal.role = Role::Member(None);
    }
    let ctx = Ctx::new(principal, RequestId::new(&SystemClock)).with_route(declared, original);
    let mut tx = begin(&ctx, &pool).await;
    let decoded = decode(&body(f.a_lead)).expect("decodes");
    let err = authorize(&mut tx, &ctx, &decoded)
        .await
        .expect_err("a forged proof must be refused by the chain");
    assert!(tx.lock_ledger().steps().is_empty());
    tx.rollback().await.expect("rollback");
    ((err.status, err.code), executed.load(Ordering::SeqCst))
}

#[tokio::test]
async fn w0_d4_v1_a_replacement_rule_and_policy_cannot_mint_a_proof_the_chain_accepts() {
    let original = Spy::new(false);
    let replacement = Spy::new(true);
    let r = Arc::clone(&replacement);
    let (refusal, executed) = run_forging(
        "d4_v1_rule",
        false,
        Arc::clone(&original),
        Arc::new(move |c: &Ctx| c.clone().with_route(TargetRule::None, r.clone())),
    )
    .await;
    assert_eq!(refusal, (500, "internal_error"));
    assert_eq!(executed, 0, "execute cannot follow a refused authorize");
    assert_eq!(
        replacement.asked().len(),
        1,
        "the replacement did decide, uselessly"
    );
    assert!(
        original.asked().is_empty(),
        "the original policy was never reached by the forgery"
    );
}

#[tokio::test]
async fn w0_d4_v1_the_same_rule_with_a_permissive_policy_is_refused_too() {
    let replacement = Spy::new(true);
    let r = Arc::clone(&replacement);
    let (refusal, executed) = run_forging(
        "d4_v1_policy",
        false,
        Spy::new(false),
        Arc::new(move |c: &Ctx| {
            c.clone()
                .with_route(TargetRule::SelfOrProjectLead, r.clone())
        }),
    )
    .await;
    assert_eq!(refusal, (500, "internal_error"));
    assert_eq!(executed, 0);
}

#[tokio::test]
async fn w0_d4_v1_a_ctx_rebuilt_with_ctx_new_is_refused_even_with_the_same_ids() {
    let replacement = Spy::new(true);
    let r = Arc::clone(&replacement);
    let (refusal, executed) = run_forging(
        "d4_v1_rebuilt",
        false,
        Spy::new(false),
        Arc::new(move |c: &Ctx| {
            Ctx::new(c.principal().clone(), c.request_id())
                .with_route(TargetRule::SelfOrProjectLead, r.clone())
        }),
    )
    .await;
    assert_eq!(refusal, (500, "internal_error"));
    assert_eq!(executed, 0);
}

#[tokio::test]
async fn w0_d4b_a_clone_inside_authorize_is_the_chains_authority_and_one_identity_everywhere() {
    // Delta 4b: the `Ctx` has no public field, so a clone cannot carry another principal, project or
    // request id (trybuild fixtures d4b_assign_*_fail). What a handler's clone does is the same
    // authority: its proof is accepted, and the identity the accessors, the proof and the
    // transaction's context report is the chain's.
    let (_db, f, pool, _p) = setup("d4b_clone").await;
    let spy = Spy::new(true);
    let executed = Arc::new(AtomicUsize::new(0));
    let (decode, authorize, execute, declared) = parts(forging_routes(
        Arc::new(|c: &Ctx| c.clone()),
        Arc::clone(&executed),
    ));
    let ctx = Ctx::new(principal(&f), RequestId::new(&SystemClock)).with_route(declared, spy);
    let mut tx = begin(&ctx, &pool).await;
    assert_eq!(
        tx.context().project,
        Some(f.a),
        "Ctx::begin reads the one authority"
    );
    assert_eq!(tx.context().actor, cortex_db::Actor::Worker(f.a_lead));
    assert_eq!(tx.context().request_id, ctx.request_id());
    let decoded = decode(&body(f.a_lead)).expect("decodes");
    let auth = authorize(&mut tx, &ctx, &decoded)
        .await
        .expect("a clone is the same authority");
    assert!(auth.belongs_to(&ctx));
    assert_eq!(
        (auth.worker(), auth.project()),
        (ctx.principal().worker, ctx.project())
    );
    assert_eq!((ctx.principal().worker, ctx.project()), (f.a_lead, f.a));
    execute(&mut tx, &ctx, auth, decoded)
        .await
        .expect("executed");
    assert_eq!(executed.load(Ordering::SeqCst), 1);
    tx.rollback().await.expect("rollback");
}
