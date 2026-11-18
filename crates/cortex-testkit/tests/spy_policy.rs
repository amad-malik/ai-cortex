//! `SpyPolicy` (PO-17's double; W1-22). DB-free. The cells that prove it records are RED against the
//! stub that only answers.
use std::sync::Arc;

use cortex_module::{Principal, Role, ScopePolicy, Target, TargetRule, WorkerKind};
use cortex_testkit::spy::{SpyPolicy, TargetCall};
use cortex_types::{ProjectId, ScopeName, Timestamp, TokenId, WorkerId};

fn wid(n: u8) -> WorkerId {
    WorkerId::parse(&format!("0193e6a0-0000-7000-8000-0000000001{n:02}")).expect("a v7 id")
}

fn caller() -> Principal {
    Principal {
        worker: wid(1),
        project: ProjectId::parse("0193e6a0-0000-7000-8000-0000000000a1").expect("a v7 id"),
        kind: WorkerKind::Worker,
        role: Role::ProjectLead,
        token: TokenId::parse("0193e6a0-0000-7000-8000-0000000000aa").expect("a v7 id"),
        expires_at: Timestamp::from_unix_micros(4_102_444_799_000_000),
        renew_due: false,
    }
}

fn target(n: u8) -> Target {
    Target {
        worker: Some(wid(n)),
        kind: Some(WorkerKind::Worker),
        team: None,
        lead: Some(wid(1)),
        consult: false,
    }
}

#[test]
fn it_records_each_call_in_order_with_its_rule_and_its_target() {
    let spy = SpyPolicy::allowing();
    spy.target(&caller(), TargetRule::RetireTarget, &target(2));
    spy.target(&caller(), TargetRule::NotProjectLead, &target(3));
    assert_eq!(
        spy.calls(),
        vec![
            TargetCall {
                rule: TargetRule::RetireTarget,
                target: target(2)
            },
            TargetCall {
                rule: TargetRule::NotProjectLead,
                target: target(3)
            },
        ]
    );
}

#[test]
fn it_answers_what_it_was_built_to_answer_and_records_either_way() {
    let yes = SpyPolicy::allowing();
    let no = SpyPolicy::denying();
    assert!(yes.target(&caller(), TargetRule::SelfWorker, &target(2)));
    assert!(!no.target(&caller(), TargetRule::SelfWorker, &target(2)));
    assert_eq!(yes.calls().len(), 1);
    assert_eq!(no.calls().len(), 1, "a refused call is recorded too");
}

#[test]
fn calls_leaves_the_record_and_take_empties_it() {
    let spy = SpyPolicy::allowing();
    spy.target(&caller(), TargetRule::Lineage, &target(2));
    assert_eq!(spy.calls().len(), 1);
    assert_eq!(spy.calls().len(), 1, "calls() does not drain");
    assert_eq!(spy.take().len(), 1);
    assert!(spy.calls().is_empty(), "take() emptied the record");
    assert!(spy.take().is_empty());
}

#[test]
fn allows_is_true_for_any_scope_so_only_the_target_is_under_test() {
    let spy = SpyPolicy::denying();
    let s = ScopeName::parse("worker:retire").expect("a scope name");
    assert!(spy.allows(&caller(), &s));
    let mut service = caller();
    service.kind = WorkerKind::Service;
    assert!(spy.allows(&service, &s), "the spy has no policy of its own");
    assert!(spy.calls().is_empty(), "allows records nothing");
}

#[test]
fn it_is_usable_as_the_chains_policy_and_from_several_threads() {
    let spy = Arc::new(SpyPolicy::allowing());
    let as_policy: Arc<dyn ScopePolicy> = spy.clone();
    let handles: Vec<_> = (0..4)
        .map(|i| {
            let p = Arc::clone(&as_policy);
            std::thread::spawn(move || {
                for _ in 0..25 {
                    p.target(&caller(), TargetRule::TokenTarget, &target(2 + i));
                }
            })
        })
        .collect();
    for h in handles {
        h.join().expect("a thread");
    }
    assert_eq!(
        spy.calls().len(),
        100,
        "every call from every thread is recorded"
    );
}
