//! PO-18: the consult flag (cox W1-9, binding for Wave 1; `Target.consult` is a plain `bool` on finn's
//! `wave1/ports`, default false). DB-free.
//!   (a) `TaskAddressee` with `consult` true is true for every caller and every other field, all
//!       `None` included, a `Service` principal too (a Service is stopped by `allows`, PO-03, not by
//!       `target`; both must hold, and this file says so);
//!   (b) with `consult` false the table is unchanged (PO-10, on the same oracle);
//!   (c) EVERY OTHER rule ignores `consult`: exhaustive, so a handler that sets the flag on a retire or a
//!       token target gains nothing;
//!   (d) `Target::default().consult` is false, so a handler that forgets the field fails closed.
//! The handler-level spy cell (consult is true ONLY for a consult create) belongs to `mod-handoffs`
//! and uses `cortex_testkit::spy::SpyPolicy`.
mod support;

use cortex_module::{Role, ScopePolicy, Target, TargetRule, WorkerKind};
use cortex_types::ScopeName;
use mod_policy::Policy;
use support::*;

fn run(p: &cortex_module::Principal, rule: TargetRule, t: &Target) -> bool {
    Policy::new().target(p, rule, t)
}

fn with_consult(t: &Target, consult: bool) -> Target {
    Target {
        consult,
        ..t.clone()
    }
}

/// The same five callers, and the same five as Services: the flag must not depend on either.
fn every_caller() -> Vec<(String, cortex_module::Principal)> {
    let mut out: Vec<(String, cortex_module::Principal)> = callers()
        .into_iter()
        .map(|(who, p)| (who.to_owned(), p))
        .collect();
    for (who, p) in callers() {
        let mut service = p;
        service.kind = WorkerKind::Service;
        out.push((format!("{who}, as a Service"), service));
    }
    out
}

#[test]
fn po18a_a_consult_is_true_for_every_caller_and_every_target() {
    let mut wrong = Vec::new();
    let mut n = 0usize;
    for (who, p) in every_caller() {
        for t in domain(&p) {
            n += 1;
            let consult = with_consult(&t, true);
            if !run(&p, TargetRule::TaskAddressee, &consult) {
                wrong.push(format!("{who}, {consult:?}: a consult must be true"));
            }
        }
    }
    assert_eq!(
        n,
        10 * 108,
        "ten callers (five, and the same five as Services) x 108 targets"
    );
    assert!(
        wrong.is_empty(),
        "{} wrong; first ten:\n{:#?}",
        wrong.len(),
        &wrong[..wrong.len().min(10)]
    );
    // All fields None is a consult too (a job-role addressee does not exist for a consult).
    let nothing = Target {
        consult: true,
        ..Target::default()
    };
    for (who, p) in every_caller() {
        assert!(
            run(&p, TargetRule::TaskAddressee, &nothing),
            "{who}: an empty consult target"
        );
    }
}

#[test]
fn po18a_the_service_is_stopped_by_allows_not_by_target() {
    // Both must hold: `target` says yes to a consult for a Service, and `allows` says no to a Service.
    let policy = Policy::new();
    let service = principal(LEAD, WorkerKind::Service, Role::ProjectLead);
    let consult = Target {
        consult: true,
        ..Target::default()
    };
    assert!(policy.target(&service, TargetRule::TaskAddressee, &consult));
    assert!(!policy.allows(
        &service,
        &ScopeName::parse("handoff:create").expect("a scope")
    ));
}

#[test]
fn po18b_with_consult_false_the_table_is_unchanged() {
    // The PO-10 oracle (consult false), explicitly with the flag spelled out.
    let mut wrong = Vec::new();
    for (who, p) in callers() {
        for t in domain(&p) {
            let t = with_consult(&t, false);
            if run(&p, TargetRule::TaskAddressee, &t) != oracle(&p, TargetRule::TaskAddressee, &t) {
                wrong.push(format!("{who}, {t:?}"));
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
    // A member is still false for a task, whatever the target holds.
    let task = tg(Some(OTHER), Some(WorkerKind::Worker), Some(1), None);
    assert!(!run(&member_1(), TargetRule::TaskAddressee, &task));
    assert!(!run(
        &member_0(),
        TargetRule::TaskAddressee,
        &Target::default()
    ));
}

#[test]
fn po18c_every_other_rule_ignores_consult() {
    let mut differ = Vec::new();
    let mut n = 0usize;
    for (who, p) in callers() {
        for t in domain(&p) {
            for rule in RULES {
                if rule == TargetRule::TaskAddressee {
                    continue;
                }
                n += 1;
                let off = run(&p, rule, &with_consult(&t, false));
                let on = run(&p, rule, &with_consult(&t, true));
                if off != on {
                    differ.push(format!(
                        "{who}, {}, {t:?}: consult false {off}, true {on}",
                        rule_name(rule)
                    ));
                }
            }
        }
    }
    assert_eq!(
        n,
        5 * 108 * 7,
        "5 callers x 108 targets x the 7 other rules"
    );
    assert!(
        differ.is_empty(),
        "{} differ; first ten:\n{:#?}",
        differ.len(),
        &differ[..differ.len().min(10)]
    );
}

#[test]
fn po18d_the_default_target_is_not_a_consult_so_a_forgotten_field_fails_closed() {
    assert!(!Target::default().consult);
    // A handler that builds a Target and forgets `consult` is a task: a member is refused.
    let forgotten = Target {
        worker: Some(wid(OTHER)),
        kind: Some(WorkerKind::Worker),
        team: Some(team1()),
        lead: None,
        ..Target::default()
    };
    assert!(!run(&member_1(), TargetRule::TaskAddressee, &forgotten));
}
