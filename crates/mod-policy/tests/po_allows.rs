//! PO-01 to PO-03b: `allows`, the scope-only check (section 3.1 of `mvp/tests/mod-identity-and-mod-policy.md`).
//! DB-free. The expected matrix is typed by hand in `support::MATRIX` from the table; the S3 twin is
//! read at test time and must agree with it, so the table, the twin and the code are three sources.
mod support;

use std::collections::BTreeSet;

use cortex_module::{Role, ScopeClass, ScopePolicy, WorkerKind};
use cortex_types::ScopeName;
use mod_policy::Policy;
use support::*;

fn scope(s: &str) -> ScopeName {
    ScopeName::parse(s).expect("a well-formed scope name")
}

#[test]
fn po01_every_cell_of_the_matrix() {
    let policy = Policy::new();
    let mut wrong = Vec::new();
    for (name, _class, lead_ok, tl_ok, member_ok) in MATRIX {
        let s = scope(name);
        for (who, p, want) in [
            ("project lead", lead(), lead_ok),
            ("team lead", team_lead_1(), tl_ok),
            ("member of a team", member_1(), member_ok),
            ("member in no team", member_0(), member_ok),
        ] {
            if policy.allows(&p, &s) != want {
                wrong.push(format!("{name} for the {who}: expected {want}"));
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:#?}");
}

#[test]
fn po01_the_matrix_the_rows_and_the_twin_agree_on_the_twenty_scopes() {
    let twin = twin();
    let xs = twin["x-scopes"].as_object().expect("x-scopes is an object");
    let twin_names: BTreeSet<&str> = xs.keys().map(String::as_str).collect();
    let matrix_names: BTreeSet<&str> = MATRIX.iter().map(|m| m.0).collect();
    assert_eq!(twin_names.len(), 20, "the twin declares 20 scopes");
    assert_eq!(
        matrix_names, twin_names,
        "the hand-typed matrix and the twin's x-scopes must name the same scopes"
    );

    // The class of each scope: the twin, the matrix and the code's row agree.
    let rows = Policy::rows();
    assert_eq!(rows.len(), 20, "the code's table has exactly 20 rows");
    let row_names: BTreeSet<&str> = rows.iter().map(|r| r.name).collect();
    assert_eq!(
        row_names, twin_names,
        "the code's rows name the twin's scopes"
    );
    for (name, class, lead_ok, tl_ok, member_ok) in MATRIX {
        assert_eq!(
            xs[name][0].as_str(),
            Some(class),
            "{name}: the twin's class"
        );
        let row = rows.iter().find(|r| r.name == name).expect("a row");
        let want = match class {
            "self" => ScopeClass::SelfScope,
            "work" => ScopeClass::Work,
            "admin" => ScopeClass::Admin,
            other => panic!("unexpected class {other}"),
        };
        assert_eq!(row.class, want, "{name}: the row's class");
        assert_eq!(
            (row.project_lead, row.team_lead, row.member),
            (lead_ok, tl_ok, member_ok),
            "{name}: the row's default roles equal the matrix"
        );
    }

    // The twin's own words for the plain rows, as a third check on the typed matrix.
    for (name, class, ..) in MATRIX {
        let text = xs[name][1].as_str().expect("a roles text");
        let (l, t, m) = {
            let row = MATRIX.iter().find(|r| r.0 == name).expect("a row");
            (row.2, row.3, row.4)
        };
        match text {
            "project_lead" | "project_lead only, own project" => {
                assert_eq!((l, t, m), (true, false, false), "{name}: {text}")
            }
            "project_lead, team_lead, member" | "every principal of kind worker (R-12)" => {
                assert_eq!((l, t, m), (true, true, true), "{name}: {text}")
            }
            _ => {} // a rule-dependent text (retire, token:*, handoff:*): the matrix is the authority
        }
        let _ = class;
    }
}

#[test]
fn po02_fail_closed_on_junk_and_on_unknown_scopes_for_every_role() {
    // The junk never parses into a ScopeName, so it can never reach `allows` ...
    for junk in [
        "",
        "x",
        "worker:create ",
        "WORKER:CREATE",
        "worker:*",
        "*",
        "worker:create:extra",
    ] {
        assert!(
            ScopeName::parse(junk).is_err(),
            "{junk:?} must not be a ScopeName"
        );
    }
    // ... and a WELL-FORMED scope the table does not know is false for every role, the lead included.
    let policy = Policy::new();
    let callers = [lead(), team_lead_1(), member_1(), member_0()];
    for unknown in [
        "worker:delete",
        "project:rename",
        "audit:write",
        "self:write",
        "token:mint",
    ] {
        let s = scope(unknown);
        for p in &callers {
            assert!(
                !policy.allows(p, &s),
                "{unknown} is unknown to the table: false for {:?}",
                p.role
            );
        }
    }
}

#[test]
fn po03_a_principal_that_is_not_a_worker_gets_false_for_all_twenty_scopes() {
    let policy = Policy::new();
    for role in [
        Role::ProjectLead,
        Role::TeamLead(team1()),
        Role::Member(Some(team1())),
        Role::Member(None),
    ] {
        let p = principal(LEAD, WorkerKind::Service, role);
        for (name, ..) in MATRIX {
            assert!(
                !policy.allows(&p, &scope(name)),
                "a Service with role {role:?} must not hold {name} (R-12), self:read and self:revoke included"
            );
        }
    }
}

#[test]
fn po03b_a_worker_in_each_role_holds_self_read_and_self_revoke() {
    let policy = Policy::new();
    for p in [lead(), team_lead_1(), member_1(), member_0()] {
        for name in ["self:read", "self:revoke"] {
            assert!(policy.allows(&p, &scope(name)), "{name} for {:?}", p.role);
        }
    }
}
