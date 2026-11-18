//! The manifest, the migration and the shared problem codes against the frozen S3 twin and the S2 pin (DB-free).
//! Stage A cells: the five scopes (X-07's scope half), the migration byte for byte, STA-5 and the shared texts
//! of cox's W1-34. The route table against the twin (STA-4) is a stage C cell: it needs the routes.
//! RED: these read static data, so they may already pass; `ModTokens::routes` is `todo!()`.
mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::{s2_tokens_sql, shared_problem_codes, twin};
use cortex_module::{DefaultRole, Piece, Port, ScopeClass};
use mod_tokens::{BOOTSTRAP_SQL, MANIFEST, MIGRATION_0001_DOWN, MIGRATION_0001_UP, ModTokens};
use sha2::{Digest, Sha256};

/// `(method, path) -> (scope, x-target)` for every tokens operation in the twin.
fn twin_token_routes() -> BTreeMap<(String, String), (String, String)> {
    let t = twin();
    let mut out = BTreeMap::new();
    for (path, ops) in t["paths"].as_object().expect("paths") {
        for (method, op) in ops.as_object().expect("ops") {
            if op["x-piece"] == "tokens" {
                out.insert(
                    (method.to_uppercase(), path.clone()),
                    (
                        op["x-scope"].as_str().expect("scope").to_owned(),
                        op["x-target"].as_str().expect("target").to_owned(),
                    ),
                );
            }
        }
    }
    out
}

#[test]
fn the_twin_has_six_token_routes_and_the_manifest_declares_exactly_their_scopes() {
    let routes = twin_token_routes();
    assert_eq!(routes.len(), 6, "S3 r2.6 gives the tokens piece six routes");
    let from_twin: BTreeSet<String> = routes.values().map(|(s, _)| s.clone()).collect();
    let declared: BTreeSet<String> = MANIFEST.scopes.iter().map(|s| s.name.to_owned()).collect();
    assert_eq!(declared, from_twin);
    assert_eq!(declared.len(), 5);
    let t = twin();
    for d in MANIFEST.scopes {
        let class = t["x-scopes"][d.name][0]
            .as_str()
            .expect("class in x-scopes");
        let ours = match d.class {
            ScopeClass::SelfScope => "self",
            ScopeClass::Work => "work",
            ScopeClass::Admin => "admin",
            ScopeClass::Service => "service",
        };
        assert_eq!(ours, class, "class of {}", d.name);
    }
}

#[test]
fn default_roles_follow_the_twin_and_no_admin_scope_goes_to_a_member() {
    use DefaultRole::{Member, ProjectLead, TeamLead};
    let expect: BTreeMap<&str, Vec<DefaultRole>> = BTreeMap::from([
        ("self:read", vec![ProjectLead, TeamLead, Member]),
        ("self:revoke", vec![ProjectLead, TeamLead, Member]),
        ("token:issue", vec![ProjectLead, TeamLead]),
        ("token:list", vec![ProjectLead, TeamLead]),
        ("token:revoke", vec![ProjectLead, TeamLead]),
    ]);
    for d in MANIFEST.scopes {
        assert_eq!(d.default_roles.to_vec(), expect[d.name], "{}", d.name);
        if matches!(d.class, ScopeClass::Admin) {
            assert!(!d.default_roles.contains(&Member), "{} is admin", d.name);
        }
    }
}

#[test]
fn the_manifest_provides_the_verifier_and_uses_identity_and_audit() {
    assert_eq!(MANIFEST.id, "tokens");
    assert_eq!(MANIFEST.schema, "tokens");
    assert_eq!(MANIFEST.provides, &[Port::TokenVerifier]);
    assert_eq!(MANIFEST.uses, &[Port::Identity, Port::Audit]);
}

#[test]
fn the_migration_is_the_s2_tokens_sql_byte_for_byte() {
    assert_eq!(MIGRATION_0001_UP, s2_tokens_sql());
    let digest = Sha256::digest(MIGRATION_0001_UP.as_bytes());
    assert_eq!(
        digest
            .iter()
            .take(4)
            .map(|b| format!("{b:02x}"))
            .collect::<String>(),
        "8357e64a",
        "the pinned file"
    );
    assert_eq!(MIGRATION_0001_DOWN.trim(), "DROP SCHEMA tokens CASCADE;");
    let set = ModTokens::new().migrations();
    assert_eq!(set.schema, "tokens");
    assert_eq!(set.steps.len(), 1);
    assert_eq!(set.steps[0].version, 1);
}

#[test]
fn the_bootstrap_creates_only_this_pieces_role_idempotently() {
    assert!(BOOTSTRAP_SQL.contains("cortex_m_tokens"));
    assert!(
        !BOOTSTRAP_SQL.contains("cortex_install_admin"),
        "the host role is not this piece's to touch"
    );
    assert!(BOOTSTRAP_SQL.contains("duplicate_object"), "idempotent");
}

/// STA-5 and W1-34: every problem code the piece mints is in S3's `x-error-codes.pieces`, with that status, and the
/// two SHARED codes carry the title and fix of `shared-problem-codes.json` word for word (the registry refuses a
/// difference between pieces).
#[test]
fn sta_5_the_problem_codes_are_s3s_and_the_shared_texts_are_the_shared_files() {
    let t = twin();
    let shared = shared_problem_codes();
    let codes: BTreeSet<&str> = MANIFEST.problems.iter().map(|p| p.code).collect();
    assert_eq!(codes, BTreeSet::from(["reason_required", "target_retired"]));
    for p in MANIFEST.problems {
        assert_eq!(
            t["x-error-codes"]["pieces"][p.code].as_u64(),
            Some(u64::from(p.status)),
            "{}: S3's status",
            p.code
        );
        let entry = shared["codes"]
            .as_array()
            .expect("codes")
            .iter()
            .find(|c| c["code"] == p.code)
            .unwrap_or_else(|| panic!("{} is in the shared file", p.code));
        assert_eq!(
            entry["status"].as_u64(),
            Some(u64::from(p.status)),
            "{} status",
            p.code
        );
        assert_eq!(entry["title"].as_str(), Some(p.title), "{} title", p.code);
        assert_eq!(entry["fix"].as_str(), Some(p.fix), "{} fix", p.code);
    }
}
