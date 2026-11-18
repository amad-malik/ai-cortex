//! PO-04: the startup check `Policy::check_manifests` (handover decision 7). DB-free.
//!
//! The registry already refuses a malformed scope name, a scope declared twice and a route naming a
//! scope no manifest declares (finn's `cortex-module/tests/w0_s0_registry.rs`); this file does NOT
//! repeat them. It tests what only the table can know: every scope a manifest declares must have a
//! row (`UnknownToPolicy`), the class must match (`ClassMismatch`), the default roles must match
//! (`RolesMismatch`), and an `admin` scope that lists `Member` is `AdminGrantsMember`, reported
//! INSTEAD of `RolesMismatch`. Errors name scopes only. All errors are reported at once.
mod support;

use cortex_module::{CONTRACT_VERSION, DefaultRole, Manifest, ScopeClass, ScopeDecl};
use mod_policy::{Policy, PolicyError};
use support::*;

fn class_of(name: &str) -> ScopeClass {
    match name {
        "self" => ScopeClass::SelfScope,
        "work" => ScopeClass::Work,
        "admin" => ScopeClass::Admin,
        other => panic!("unexpected class {other}"),
    }
}

fn roles(lead: bool, team_lead: bool, member: bool) -> &'static [DefaultRole] {
    let mut v = Vec::new();
    if lead {
        v.push(DefaultRole::ProjectLead);
    }
    if team_lead {
        v.push(DefaultRole::TeamLead);
    }
    if member {
        v.push(DefaultRole::Member);
    }
    Box::leak(v.into_boxed_slice())
}

/// The correct declaration of one scope, from the hand-typed matrix.
fn good(name: &'static str) -> ScopeDecl {
    let m = MATRIX.iter().find(|m| m.0 == name).expect("a matrix row");
    ScopeDecl {
        name,
        class: class_of(m.1),
        default_roles: roles(m.2, m.3, m.4),
    }
}

fn manifest(id: &'static str, scopes: Vec<ScopeDecl>) -> &'static Manifest {
    Box::leak(Box::new(Manifest {
        id,
        version: "0.0.1",
        contract: CONTRACT_VERSION,
        core: false,
        schema: id,
        provides: &[],
        uses: &[],
        scopes: Box::leak(scopes.into_boxed_slice()),
        problems: &[],
    }))
}

fn all_good() -> Vec<ScopeDecl> {
    MATRIX.iter().map(|m| good(m.0)).collect()
}

#[test]
fn po04_the_table_accepts_its_own_twenty_scopes_and_any_subset_a_piece_declares() {
    assert_eq!(
        Policy::check_manifests([manifest("everything", all_good())]),
        Ok(())
    );
    // Pieces declare only their own scopes: two manifests, disjoint.
    let a = manifest("pa", vec![good("worker:create"), good("team:create")]);
    let b = manifest("pb", vec![good("records:read"), good("self:read")]);
    assert_eq!(Policy::check_manifests([a, b]), Ok(()));
    // No manifest at all, or one with no scopes, is fine.
    assert_eq!(
        Policy::check_manifests(std::iter::empty::<&Manifest>()),
        Ok(())
    );
    assert_eq!(
        Policy::check_manifests([manifest("noscopes", vec![])]),
        Ok(())
    );
}

#[test]
fn po04_a_scope_the_table_has_no_row_for_is_unknown_to_policy() {
    let m = manifest(
        "pc",
        vec![ScopeDecl {
            name: "worker:delete",
            class: ScopeClass::Admin,
            default_roles: roles(true, false, false),
        }],
    );
    assert_eq!(
        Policy::check_manifests([m]),
        Err(vec![PolicyError::UnknownToPolicy {
            scope: "worker:delete".to_owned()
        }])
    );
}

#[test]
fn po04_a_class_that_differs_from_the_table_is_a_class_mismatch() {
    let m = manifest(
        "pd",
        vec![ScopeDecl {
            name: "worker:create",
            class: ScopeClass::Work, // the table says admin
            default_roles: roles(true, false, false),
        }],
    );
    assert_eq!(
        Policy::check_manifests([m]),
        Err(vec![PolicyError::ClassMismatch {
            scope: "worker:create".to_owned()
        }])
    );
}

#[test]
fn po04_default_roles_that_differ_from_the_table_are_a_roles_mismatch() {
    let m = manifest(
        "pe",
        vec![ScopeDecl {
            name: "records:read",
            class: ScopeClass::Work,
            default_roles: roles(true, false, false), // the table says lead, team lead and member
        }],
    );
    assert_eq!(
        Policy::check_manifests([m]),
        Err(vec![PolicyError::RolesMismatch {
            scope: "records:read".to_owned()
        }])
    );
}

#[test]
fn po04_an_admin_scope_that_lists_member_is_admin_grants_member_and_only_that() {
    let m = manifest(
        "pf",
        vec![ScopeDecl {
            name: "audit:read",
            class: ScopeClass::Admin,
            default_roles: roles(true, false, true), // the lead (correct) and a member (never for admin)
        }],
    );
    assert_eq!(
        Policy::check_manifests([m]),
        Err(vec![PolicyError::AdminGrantsMember {
            scope: "audit:read".to_owned()
        }]),
        "reported INSTEAD of RolesMismatch, not as well as it"
    );
}

#[test]
fn po04_every_error_is_reported_at_once_and_names_scopes_only() {
    let m = manifest(
        "canary_manifest_id_do_not_echo",
        vec![
            ScopeDecl {
                name: "worker:delete",
                class: ScopeClass::Admin,
                default_roles: roles(true, false, false),
            },
            ScopeDecl {
                name: "worker:create",
                class: ScopeClass::Work,
                default_roles: roles(true, false, false),
            },
            ScopeDecl {
                name: "records:read",
                class: ScopeClass::Work,
                default_roles: roles(true, true, false),
            },
            ScopeDecl {
                name: "audit:read",
                class: ScopeClass::Admin,
                default_roles: roles(true, false, true),
            },
            good("self:read"),
        ],
    );
    let errs = Policy::check_manifests([m]).expect_err("four scopes are wrong");
    assert_eq!(errs.len(), 4, "all four, not the first: {errs:?}");
    let text = format!("{errs:?}");
    assert!(
        !text.contains("canary_manifest_id_do_not_echo"),
        "the manifest id is not data an error carries: {text}"
    );
    for scope in [
        "worker:delete",
        "worker:create",
        "records:read",
        "audit:read",
    ] {
        assert!(text.contains(scope), "{scope} is named: {text}");
    }
    assert!(
        !text.contains("self:read"),
        "a correct scope is not named: {text}"
    );
}
