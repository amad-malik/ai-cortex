//! Wave 0 S0 delta 4 (cox c025db40; kai 68cecbdb): `Ctx::authorize`, the only maker of a proof.
//! DB-free: a spy `ScopePolicy` records what it was asked.
//!
//! - **The rule is the route's.** The chain puts the route's DECLARED rule and the policy into the
//!   `Ctx` (`with_route`); `authorize` takes no rule argument, so a handler cannot prove itself
//!   against a weaker rule (for example `None`). It also makes S3's `x-target` binding at run time.
//! - **False is 403 `target_not_allowed`.** A `Ctx` without a route (one not made by the chain) is
//!   500 `internal_error`, never allowed: fail closed.
//! - `authorize` is pure and synchronous, and takes no lock.
//!
//! RED: `Ctx::authorize` is `todo!()`.
use std::sync::{Arc, Mutex};

use cortex_module::{
    Authorized, Ctx, Principal, Role, ScopePolicy, Target, TargetRule, WorkerKind,
};
use cortex_types::{
    ProjectId, RequestId, ScopeName, SystemClock, TeamId, Timestamp, TokenId, WorkerId,
};

const ALL_RULES: [TargetRule; 8] = [
    TargetRule::None,
    TargetRule::SelfWorker,
    TargetRule::SelfOrProjectLead,
    TargetRule::NotProjectLead,
    TargetRule::RetireTarget,
    TargetRule::TokenTarget,
    TargetRule::Lineage,
    TargetRule::TaskAddressee,
];

/// Answers a fixed bool and remembers every question.
struct Spy {
    answer: bool,
    asked: Mutex<Vec<(WorkerId, TargetRule, Target)>>,
}

impl Spy {
    fn new(answer: bool) -> Arc<Spy> {
        Arc::new(Spy {
            answer,
            asked: Mutex::new(Vec::new()),
        })
    }
    fn asked(&self) -> Vec<(WorkerId, TargetRule, Target)> {
        self.asked.lock().expect("spy lock").clone()
    }
}

impl ScopePolicy for Spy {
    fn allows(&self, _p: &Principal, _scope: &ScopeName) -> bool {
        panic!("authorize must not ask for a scope: the chain's guard did");
    }
    fn target(&self, p: &Principal, rule: TargetRule, t: &Target) -> bool {
        self.asked
            .lock()
            .expect("spy lock")
            .push((p.worker, rule, t.clone()));
        self.answer
    }
}

fn principal() -> Principal {
    let c = SystemClock;
    Principal {
        worker: WorkerId::new(&c),
        project: ProjectId::new(&c),
        kind: WorkerKind::Worker,
        role: Role::TeamLead(TeamId::new(&c)),
        token: TokenId::new(&c),
        expires_at: Timestamp::from_unix_micros(0),
        renew_due: false,
    }
}

fn ctx_with(rule: TargetRule, policy: Arc<Spy>) -> Ctx {
    Ctx::new(principal(), RequestId::new(&SystemClock)).with_route(rule, policy)
}

fn target_of(w: WorkerId) -> Target {
    Target {
        worker: Some(w),
        kind: Some(WorkerKind::Service),
        team: None,
        lead: None,
        consult: false,
    }
}

#[test]
fn d4_ca_1_the_policy_is_asked_with_the_routes_declared_rule_and_the_row_the_handler_read() {
    for rule in ALL_RULES {
        let spy = Spy::new(true);
        let ctx = ctx_with(rule, spy.clone());
        let t = target_of(WorkerId::new(&SystemClock));
        let a = ctx.authorize(&t, 7_u32).expect("allowed");
        assert_eq!(
            *a.value(),
            7,
            "{rule:?}: the value comes back with the proof"
        );
        assert_eq!(
            spy.asked(),
            [(ctx.principal().worker, rule, t)],
            "{rule:?}: asked once, for the caller, with the DECLARED rule and the handler's target"
        );
    }
}

#[test]
fn d4_ca_2_a_refusal_is_403_target_not_allowed_and_asks_once() {
    let spy = Spy::new(false);
    let ctx = ctx_with(TargetRule::TokenTarget, spy.clone());
    let err = ctx
        .authorize(&target_of(WorkerId::new(&SystemClock)), ())
        .expect_err("refused");
    assert_eq!((err.status, err.code), (403, "target_not_allowed"));
    assert_eq!(spy.asked().len(), 1);
}

#[test]
fn d4_ca_3_rule_none_is_not_special_cased_the_policy_decides() {
    // A policy that refuses even `None` refuses: the S0 does not wave `None` through.
    let spy = Spy::new(false);
    let ctx = ctx_with(TargetRule::None, spy.clone());
    let err = ctx.authorize(&Target::default(), ()).expect_err("refused");
    assert_eq!(err.status, 403);
    assert_eq!(spy.asked().len(), 1, "the policy was asked, not skipped");
    // Control: the same call with a policy that allows is allowed.
    let ok = ctx_with(TargetRule::None, Spy::new(true));
    assert!(ok.authorize(&Target::default(), ()).is_ok());
}

#[test]
fn d4_ca_4_a_ctx_without_a_route_fails_closed() {
    // `Ctx::new` alone is not what the chain hands a write: no rule, no policy. Never allowed.
    let ctx = Ctx::new(principal(), RequestId::new(&SystemClock));
    let err = ctx
        .authorize(&Target::default(), ())
        .expect_err("must refuse");
    assert_eq!((err.status, err.code), (500, "internal_error"));
}

#[test]
fn d4_ca_5_a_proof_for_one_ctx_is_not_a_proof_for_another() {
    // Two requests of the same caller: the proof carries the request it was made for. (The erased
    // checks that use this are in route.rs's unit tests; here the public face: Debug shows no value.)
    let spy = Spy::new(true);
    let ctx = ctx_with(TargetRule::SelfWorker, spy);
    let secret = "ctx2_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa.not-a-real-token-but-token-shaped-padding";
    let a: Authorized<String> = ctx
        .authorize(&Target::default(), secret.to_owned())
        .expect("allowed");
    let shown = format!("{a:?}");
    assert!(
        !shown.contains("ctx2_"),
        "Debug must not print the value: {shown}"
    );
    assert_eq!(a.into_value(), secret);
}

#[test]
fn d4_ca_6_the_ctx_stays_comparable_and_cloneable_with_a_route() {
    let spy = Spy::new(true);
    let ctx = ctx_with(TargetRule::Lineage, spy.clone());
    let same = ctx.clone();
    assert_eq!(
        ctx, same,
        "a clone carries the same rule and the same policy"
    );
    let other = Ctx::new(ctx.principal().clone(), ctx.request_id())
        .with_route(TargetRule::Lineage, Spy::new(true));
    assert_ne!(
        ctx, other,
        "another policy instance is another route context"
    );
    let other_rule =
        Ctx::new(ctx.principal().clone(), ctx.request_id()).with_route(TargetRule::None, spy);
    assert_ne!(ctx, other_rule, "another rule is another route context");
}
