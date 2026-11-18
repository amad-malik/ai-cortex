//! PO-10 to PO-14 and PO-16: the pure `target` rules (section 3.2 of `mvp/tests/mod-identity-and-mod-policy.md`).
//! DB-free. Three layers: (1) every cell the table names, listed by hand with its words; (2) the
//! EXHAUSTIVE comparison with the independent oracle in `support` (5 callers x 108 targets x 8 rules);
//! (3) the properties the table states (fail closed, a member never, no panic, deterministic).
//! `consult` is a field of `Target` that arrives with finn's `wave1/ports`; PO-18 is written then.
mod support;

use cortex_module::{Role, ScopePolicy, Target, TargetRule, WorkerKind};
use mod_policy::Policy;
use support::*;

const W: Option<WorkerKind> = Some(WorkerKind::Worker);
const S: Option<WorkerKind> = Some(WorkerKind::Service);

fn run(p: &cortex_module::Principal, rule: TargetRule, t: &Target) -> bool {
    Policy::new().target(p, rule, t)
}

/// One cell: a label (the table's words), the caller, the rule, the target, the expected answer.
struct Cell(
    &'static str,
    cortex_module::Principal,
    TargetRule,
    Target,
    bool,
);

fn cells() -> Vec<Cell> {
    use TargetRule::*;
    vec![
        // ---- None: always.
        Cell(
            "None: the lead",
            lead(),
            None,
            tg(Option::None, Option::None, Option::None, Option::None),
            true,
        ),
        Cell(
            "None: a member in no team",
            member_0(),
            None,
            tg(Option::None, Option::None, Option::None, Option::None),
            true,
        ),
        // ---- SelfWorker (handoff:claim, a worker addressee).
        Cell(
            "SelfWorker: the target is the caller",
            member_1(),
            SelfWorker,
            tg(Some(M1), W, Some(1), Option::None),
            true,
        ),
        Cell(
            "SelfWorker: the target is another worker",
            member_1(),
            SelfWorker,
            tg(Some(OTHER), W, Some(1), Option::None),
            false,
        ),
        Cell(
            "SelfWorker: T.worker is None",
            member_1(),
            SelfWorker,
            tg(Option::None, Option::None, Option::None, Option::None),
            false,
        ),
        Cell(
            "SelfWorker: the project lead has no override",
            lead(),
            SelfWorker,
            tg(Some(OTHER), W, Option::None, Option::None),
            false,
        ),
        // ---- SelfOrProjectLead (accept, rework: T.worker = the creator).
        Cell(
            "SelfOrProjectLead: the lead, another creator",
            lead(),
            SelfOrProjectLead,
            tg(Some(OTHER), W, Option::None, Option::None),
            true,
        ),
        Cell(
            "SelfOrProjectLead: the lead, T.worker None",
            lead(),
            SelfOrProjectLead,
            tg(Option::None, Option::None, Option::None, Option::None),
            true,
        ),
        Cell(
            "SelfOrProjectLead: a member who is the creator",
            member_1(),
            SelfOrProjectLead,
            tg(Some(M1), W, Some(1), Option::None),
            true,
        ),
        Cell(
            "SelfOrProjectLead: a member, another creator",
            member_1(),
            SelfOrProjectLead,
            tg(Some(OTHER), W, Some(1), Option::None),
            false,
        ),
        Cell(
            "SelfOrProjectLead: T.worker None and not the lead",
            member_1(),
            SelfOrProjectLead,
            tg(Option::None, Option::None, Option::None, Option::None),
            false,
        ),
        Cell(
            "SelfOrProjectLead: a team lead is no override",
            team_lead_1(),
            SelfOrProjectLead,
            tg(Some(OTHER), W, Some(1), Option::None),
            false,
        ),
        // ---- NotProjectLead (team:assign): T.worker and T.lead Some and different; the caller is ignored.
        Cell(
            "NotProjectLead: different",
            member_1(),
            NotProjectLead,
            tg(Some(OTHER), W, Option::None, Some(LEAD)),
            true,
        ),
        Cell(
            "NotProjectLead: equal",
            lead(),
            NotProjectLead,
            tg(Some(LEAD), W, Option::None, Some(LEAD)),
            false,
        ),
        Cell(
            "NotProjectLead: T.worker None",
            lead(),
            NotProjectLead,
            tg(Option::None, Option::None, Option::None, Some(LEAD)),
            false,
        ),
        Cell(
            "NotProjectLead: T.lead None",
            lead(),
            NotProjectLead,
            tg(Some(OTHER), W, Option::None, Option::None),
            false,
        ),
        // ---- RetireTarget (worker:retire): T.worker AND T.lead Some for EVERY caller.
        Cell(
            "RetireTarget: the lead retires a member of team 1",
            lead(),
            RetireTarget,
            tg(Some(M1), W, Some(1), Some(LEAD)),
            true,
        ),
        Cell(
            "RetireTarget: the lead retires a service",
            lead(),
            RetireTarget,
            tg(Some(OTHER), S, Option::None, Some(LEAD)),
            true,
        ),
        Cell(
            "RetireTarget: the lead retires itself",
            lead(),
            RetireTarget,
            tg(Some(LEAD), W, Option::None, Some(LEAD)),
            false,
        ),
        Cell(
            "RetireTarget: the lead retires T.lead",
            lead(),
            RetireTarget,
            tg(Some(OTHER), W, Option::None, Some(OTHER)),
            false,
        ),
        Cell(
            "RetireTarget: T.lead None is false for the lead too",
            lead(),
            RetireTarget,
            tg(Some(M1), W, Some(1), Option::None),
            false,
        ),
        Cell(
            "RetireTarget: a team lead retires a member of its own team",
            team_lead_1(),
            RetireTarget,
            tg(Some(M1), W, Some(1), Some(LEAD)),
            true,
        ),
        Cell(
            "RetireTarget: a team lead, a worker of another team",
            team_lead_1(),
            RetireTarget,
            tg(Some(OTHER), W, Some(2), Some(LEAD)),
            false,
        ),
        Cell(
            "RetireTarget: a team lead, a worker in no team",
            team_lead_1(),
            RetireTarget,
            tg(Some(OTHER), W, Option::None, Some(LEAD)),
            false,
        ),
        Cell(
            "RetireTarget: a team lead retires itself",
            team_lead_1(),
            RetireTarget,
            tg(Some(TL1), W, Some(1), Some(LEAD)),
            false,
        ),
        Cell(
            "RetireTarget: a team lead retires T.lead",
            team_lead_1(),
            RetireTarget,
            tg(Some(LEAD), W, Some(1), Some(LEAD)),
            false,
        ),
        Cell(
            "RetireTarget: a team lead with T.lead None",
            team_lead_1(),
            RetireTarget,
            tg(Some(M1), W, Some(1), Option::None),
            false,
        ),
        Cell(
            "RetireTarget: a member never",
            member_1(),
            RetireTarget,
            tg(Some(OTHER), W, Some(1), Some(LEAD)),
            false,
        ),
        // ---- TokenTarget (token:issue, token:revoke, GET /v2/tokens/{id}).
        Cell(
            "TokenTarget: the lead, any worker, a service too",
            lead(),
            TokenTarget,
            tg(Some(OTHER), S, Option::None, Option::None),
            true,
        ),
        Cell(
            "TokenTarget: the lead, T.worker None",
            lead(),
            TokenTarget,
            tg(Option::None, W, Some(1), Option::None),
            false,
        ),
        Cell(
            "TokenTarget: a team lead, a worker of its team",
            team_lead_1(),
            TokenTarget,
            tg(Some(M1), W, Some(1), Option::None),
            true,
        ),
        Cell(
            "TokenTarget: a team lead, a SERVICE of its team",
            team_lead_1(),
            TokenTarget,
            tg(Some(OTHER), S, Some(1), Option::None),
            false,
        ),
        Cell(
            "TokenTarget: a team lead, a worker of another team",
            team_lead_1(),
            TokenTarget,
            tg(Some(OTHER), W, Some(2), Option::None),
            false,
        ),
        Cell(
            "TokenTarget: a team lead, T.team None",
            team_lead_1(),
            TokenTarget,
            tg(Some(OTHER), W, Option::None, Option::None),
            false,
        ),
        Cell(
            "TokenTarget: a team lead, T.kind None",
            team_lead_1(),
            TokenTarget,
            tg(Some(OTHER), Option::None, Some(1), Option::None),
            false,
        ),
        Cell(
            "TokenTarget: a member never",
            member_1(),
            TokenTarget,
            tg(Some(M1), W, Some(1), Option::None),
            false,
        ),
        // ---- Lineage (supersede, invalidate, cancel): T.worker = the author, T.team = the author's team.
        Cell(
            "Lineage: the author, a member",
            member_1(),
            Lineage,
            tg(Some(M1), W, Some(1), Option::None),
            true,
        ),
        Cell(
            "Lineage: the lead, any author",
            lead(),
            Lineage,
            tg(Some(OTHER), W, Some(2), Option::None),
            true,
        ),
        Cell(
            "Lineage: the lead, an author that is not a worker",
            lead(),
            Lineage,
            tg(Option::None, Option::None, Option::None, Option::None),
            true,
        ),
        Cell(
            "Lineage: a team lead, an author of its team",
            team_lead_1(),
            Lineage,
            tg(Some(M1), W, Some(1), Option::None),
            true,
        ),
        Cell(
            "Lineage: a team lead, an author of another team",
            team_lead_1(),
            Lineage,
            tg(Some(OTHER), W, Some(2), Option::None),
            false,
        ),
        Cell(
            "Lineage: a team lead, an author that is NOT a worker, even if T.team is set",
            team_lead_1(),
            Lineage,
            tg(Option::None, Option::None, Some(1), Option::None),
            false,
        ),
        Cell(
            "Lineage: a team lead, a service author of its team",
            team_lead_1(),
            Lineage,
            tg(Some(OTHER), S, Some(1), Option::None),
            false,
        ),
        Cell(
            "Lineage: a member who is not the author",
            member_1(),
            Lineage,
            tg(Some(OTHER), W, Some(1), Option::None),
            false,
        ),
        Cell(
            "Lineage: a member in no team, a non-worker author",
            member_0(),
            Lineage,
            tg(Option::None, Option::None, Option::None, Option::None),
            false,
        ),
        // ---- TaskAddressee (a task, consult false).
        Cell(
            "TaskAddressee: the lead, a job-role addressee",
            lead(),
            TaskAddressee,
            tg(Option::None, Option::None, Option::None, Option::None),
            true,
        ),
        Cell(
            "TaskAddressee: a team lead, a worker of its team",
            team_lead_1(),
            TaskAddressee,
            tg(Some(M1), W, Some(1), Option::None),
            true,
        ),
        Cell(
            "TaskAddressee: a team lead, a job-role addressee",
            team_lead_1(),
            TaskAddressee,
            tg(Option::None, Option::None, Option::None, Option::None),
            false,
        ),
        Cell(
            "TaskAddressee: a team lead, a worker of another team",
            team_lead_1(),
            TaskAddressee,
            tg(Some(OTHER), W, Some(2), Option::None),
            false,
        ),
        Cell(
            "TaskAddressee: a team lead, the project lead as addressee (no team)",
            team_lead_1(),
            TaskAddressee,
            tg(Some(LEAD), W, Option::None, Option::None),
            false,
        ),
        Cell(
            "TaskAddressee: a member, a worker of its own team",
            member_1(),
            TaskAddressee,
            tg(Some(OTHER), W, Some(1), Option::None),
            false,
        ),
        // ---- PO-16: cancel through Lineage (T.worker = the creator, T.team = the worker addressee's team).
        Cell(
            "PO-16: the creator, whatever the addressee's team",
            member_1(),
            Lineage,
            tg(Some(M1), W, Some(2), Option::None),
            true,
        ),
        Cell(
            "PO-16: the team lead of T.team, not the creator",
            team_lead_1(),
            Lineage,
            tg(Some(OTHER), W, Some(1), Option::None),
            true,
        ),
        Cell(
            "PO-16: the team lead of another team",
            team_lead_2(),
            Lineage,
            tg(Some(OTHER), W, Some(1), Option::None),
            false,
        ),
        Cell(
            "PO-16: a team lead with T.team None (job-role addressee), not the creator",
            team_lead_1(),
            Lineage,
            tg(Some(OTHER), W, Option::None, Option::None),
            false,
        ),
        Cell(
            "PO-16: a member who is not the creator, even in T.team",
            member_1(),
            Lineage,
            tg(Some(OTHER), W, Some(1), Option::None),
            false,
        ),
        Cell(
            "PO-16: the project lead",
            lead(),
            Lineage,
            tg(Some(OTHER), W, Option::None, Option::None),
            true,
        ),
        Cell(
            "PO-16: T.worker None gives the project lead only (team lead)",
            team_lead_1(),
            Lineage,
            tg(Option::None, W, Some(1), Option::None),
            false,
        ),
        Cell(
            "PO-16 Q-13: a team lead who is the addressee of a task created by someone else",
            team_lead_1(),
            Lineage,
            tg(Some(OTHER), W, Some(1), Option::None),
            true,
        ),
    ]
}

#[test]
fn po10_every_cell_the_table_names() {
    let mut wrong = Vec::new();
    for Cell(label, p, rule, t, want) in cells() {
        let got = run(&p, rule, &t);
        if got != want {
            wrong.push(format!("{label}: expected {want}, got {got}"));
        }
    }
    assert!(wrong.is_empty(), "{} cells wrong:\n{wrong:#?}", wrong.len());
}

#[test]
fn po10_the_table_lists_at_least_one_cell_for_every_rule() {
    // Guards the cell list itself: a rule with no hand-listed cell would hide behind the oracle.
    for rule in RULES {
        let n = cells().iter().filter(|c| c.2 == rule).count();
        assert!(n >= 2, "{} has {n} hand-listed cells", rule_name(rule));
    }
}

#[test]
fn po10_exhaustive_against_the_independent_oracle() {
    let mut wrong = Vec::new();
    let mut evaluated = 0usize;
    for (who, p) in callers() {
        for t in domain(&p) {
            for rule in RULES {
                evaluated += 1;
                let got = run(&p, rule, &t);
                let want = oracle(&p, rule, &t);
                if got != want {
                    wrong.push(format!(
                        "{who}, {}, {t:?}: expected {want}, got {got}",
                        rule_name(rule)
                    ));
                }
            }
        }
    }
    assert_eq!(
        evaluated,
        5 * 108 * 8,
        "the domain is 5 callers x 108 targets x 8 rules"
    );
    assert!(
        wrong.is_empty(),
        "{} of {evaluated} evaluations differ; first ten:\n{:#?}",
        wrong.len(),
        &wrong[..wrong.len().min(10)]
    );
}

#[test]
fn po11_a_rule_that_needs_a_field_and_finds_none_is_false() {
    for (who, p) in callers() {
        for t in domain(&p) {
            // Needs T.worker: false for every caller when it is None.
            if t.worker.is_none() {
                for rule in [
                    TargetRule::SelfWorker,
                    TargetRule::TokenTarget,
                    TargetRule::NotProjectLead,
                    TargetRule::RetireTarget,
                ] {
                    assert!(
                        !run(&p, rule, &t),
                        "{who}: {} with T.worker None must be false",
                        rule_name(rule)
                    );
                }
                // The lead alone passes these when T.worker is None.
                for rule in [
                    TargetRule::SelfOrProjectLead,
                    TargetRule::Lineage,
                    TargetRule::TaskAddressee,
                ] {
                    let is_lead = matches!(p.role, Role::ProjectLead);
                    assert_eq!(
                        run(&p, rule, &t),
                        is_lead,
                        "{who}: {} with T.worker None is the lead's alone",
                        rule_name(rule)
                    );
                }
            }
            // Needs T.lead: NotProjectLead and RetireTarget are false for EVERY caller when it is None.
            if t.lead.is_none() {
                for rule in [TargetRule::NotProjectLead, TargetRule::RetireTarget] {
                    assert!(
                        !run(&p, rule, &t),
                        "{who}: {} with T.lead None must be false, the lead included",
                        rule_name(rule)
                    );
                }
            }
            // A team lead needs T.team for the team-bound rules.
            if matches!(p.role, Role::TeamLead(_)) && t.team.is_none() {
                for rule in [
                    TargetRule::RetireTarget,
                    TargetRule::TokenTarget,
                    TargetRule::TaskAddressee,
                ] {
                    assert!(
                        !run(&p, rule, &t),
                        "{who}: {} with T.team None must be false",
                        rule_name(rule)
                    );
                }
                if t.worker != Some(p.worker) {
                    assert!(
                        !run(&p, TargetRule::Lineage, &t),
                        "{who}: Lineage with T.team None, not the author, must be false"
                    );
                }
            }
        }
    }
}

#[test]
fn po12_a_member_never_gets_the_admin_rules_and_nothing_panics() {
    for p in [member_1(), member_0()] {
        for t in domain(&p) {
            for rule in [
                TargetRule::RetireTarget,
                TargetRule::TokenTarget,
                TargetRule::TaskAddressee,
            ] {
                assert!(
                    !run(&p, rule, &t),
                    "a member must be false on {} for {t:?}",
                    rule_name(rule)
                );
            }
        }
    }
    // NotProjectLead with equal fields is false for every caller.
    for (who, p) in callers() {
        let equal = tg(Some(OTHER), W, Option::None, Some(OTHER));
        assert!(
            !run(&p, TargetRule::NotProjectLead, &equal),
            "{who}: equal worker and lead"
        );
    }
    // Deterministic: the same inputs give the same answer.
    for (_, p) in callers() {
        for t in domain(&p).iter().take(40) {
            for rule in RULES {
                assert_eq!(run(&p, rule, t), run(&p, rule, t));
            }
        }
    }
}

#[test]
fn po13_target_is_a_plain_function_with_no_state_and_no_io() {
    // A plain `fn`: not async, no transaction, no clock. The trait method must coerce to a fn pointer.
    let f: fn(&Policy, &cortex_module::Principal, TargetRule, &Target) -> bool =
        <Policy as ScopePolicy>::target;
    assert!(f(
        &Policy::new(),
        &lead(),
        TargetRule::None,
        &Target::default()
    ));
    assert_eq!(
        std::mem::size_of::<Policy>(),
        0,
        "the policy holds no state"
    );

    // And the SOURCE of `allows` and `target` mentions no async, no clock, no I/O.
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs"))
        .expect("src/lib.rs");
    for name in ["allows", "target"] {
        let body = fn_body(&src, name);
        for banned in [
            "async",
            ".await",
            "std::fs",
            "std::net",
            "std::time",
            "SystemTime",
            "Instant",
            "tokio",
            "sqlx",
            "ModuleTx",
            "Clock",
            "println!",
            "eprintln!",
        ] {
            assert!(
                !body.contains(banned),
                "`fn {name}` must not mention {banned}"
            );
        }
    }
}

/// The text of `fn <name>(...) { ... }` in an `impl` block: from its line to the first line that is
/// exactly four spaces and `}`.
fn fn_body(src: &str, name: &str) -> String {
    let marker = format!("fn {name}(");
    let start = src
        .find(&marker)
        .unwrap_or_else(|| panic!("`{marker}` not found in src/lib.rs"));
    let rest = &src[start..];
    let end = rest
        .find("\n    }\n")
        .unwrap_or_else(|| panic!("the end of `fn {name}` not found"));
    rest[..end].to_owned()
}

#[test]
fn po14_every_rule_variant_has_a_fixture_and_the_set_equals_the_twins() {
    // `rule_name` and `RULES` are in support: a ninth variant stops this file compiling there.
    let mut names: Vec<&str> = RULES.iter().map(|r| rule_name(*r)).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), 8, "eight distinct variants");

    let twin = twin();
    let mut in_twin = std::collections::BTreeSet::new();
    for item in twin["paths"].as_object().expect("paths").values() {
        for op in item.as_object().expect("a path item").values() {
            if let Some(rule) = op.get("x-target").and_then(|v| v.as_str()) {
                in_twin.insert(rule.to_owned());
            }
        }
    }
    let ours: std::collections::BTreeSet<String> = names.iter().map(|s| (*s).to_owned()).collect();
    assert_eq!(
        ours, in_twin,
        "the variants equal the set of x-target values in the twin"
    );
}

#[test]
fn po05_target_is_one_match_with_no_wildcard_arm() {
    let src = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/src/lib.rs"))
        .expect("src/lib.rs");
    let body = fn_body(&src, "target");
    assert_eq!(
        body.matches("match rule").count(),
        1,
        "`target` is ONE match on the rule"
    );
    assert!(
        !body.contains("_ =>"),
        "no wildcard arm: a new variant must break the build"
    );
    for rule in RULES {
        assert!(
            body.contains(&format!("TargetRule::{}", rule_name(rule))),
            "`target` has an arm for {}",
            rule_name(rule)
        );
    }
}
