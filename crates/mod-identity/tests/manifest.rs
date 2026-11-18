//! The manifest, the migration and the route table against the frozen S3 twin and the S2 pin
//! (ID-12-adjacent scope declarations, X-07 for the 2a routes, ID-52's R-4 obligation, the S2 pin).
//! DB-free. RED: the read handlers and the label validators are `todo!()`; the tests below that
//! only read static data may already pass.
mod common;

use std::collections::{BTreeMap, BTreeSet};

use common::audit::{AuditPiece, Fault};
use common::{s2_identity_sql, twin};
use cortex_module::{Assembly, DefaultRole, Piece, Registry, ScopeClass};
use cortex_types::{ModuleId, ProblemCodeMinter};
use mod_identity::read::{TeamsQuery, WorkersQuery};
use mod_identity::{MANIFEST, MIGRATION_0001_DOWN, MIGRATION_0001_UP, ModIdentity};

/// mod-identity and the audit fixture, assembled (no database).
fn assemble() -> Registry {
    let minter = Box::leak(Box::new(
        ProblemCodeMinter::take().expect("the first take in this process"),
    ));
    let enabled = [
        ModuleId::parse(mod_identity::ID).expect("id"),
        ModuleId::parse(common::audit::ID).expect("id"),
    ];
    let compiled: Vec<Box<dyn Piece>> = vec![
        Box::new(AuditPiece {
            fault: Fault::new(),
        }),
        Box::new(ModIdentity::new()),
    ];
    Registry::assemble(Assembly {
        compiled,
        enabled: &enabled,
        minter,
        chain_codes: &["internal_error", "unavailable", "busy", "conflict"],
    })
    .expect("mod-identity and the audit fixture assemble")
}

/// `(method, path) -> (scope, x-target)` for every identity operation in the twin.
fn twin_identity_routes() -> BTreeMap<(String, String), (String, String, String)> {
    let t = twin();
    let mut out = BTreeMap::new();
    for (path, ops) in t["paths"].as_object().expect("paths") {
        for (method, op) in ops.as_object().expect("ops") {
            if op["x-piece"] == "identity" {
                out.insert(
                    (method.to_uppercase(), path.clone()),
                    (
                        op["x-scope"].as_str().expect("scope").to_owned(),
                        op["x-target"].as_str().expect("target").to_owned(),
                        op["operationId"].as_str().expect("id").to_owned(),
                    ),
                );
            }
        }
    }
    out
}

#[test]
fn the_manifest_declares_exactly_the_scopes_the_twin_gives_to_identity() {
    let t = twin();
    let from_twin: BTreeSet<String> = twin_identity_routes()
        .values()
        .map(|(scope, _, _)| scope.clone())
        .collect();
    let declared: BTreeSet<String> = MANIFEST.scopes.iter().map(|s| s.name.to_owned()).collect();
    assert_eq!(
        declared, from_twin,
        "declared scopes equal the identity operations' scopes"
    );
    assert_eq!(declared.len(), 5);
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
fn default_roles_follow_s1_5_3_and_no_admin_scope_goes_to_a_member() {
    use DefaultRole::{Member, ProjectLead, TeamLead};
    let expect: BTreeMap<&str, Vec<DefaultRole>> = BTreeMap::from([
        ("roster:read", vec![ProjectLead, TeamLead, Member]),
        ("team:create", vec![ProjectLead]),
        ("team:assign", vec![ProjectLead]),
        ("worker:create", vec![ProjectLead]),
        ("worker:retire", vec![ProjectLead, TeamLead]),
    ]);
    for d in MANIFEST.scopes {
        assert_eq!(d.default_roles.to_vec(), expect[d.name], "{}", d.name);
        if matches!(d.class, ScopeClass::Admin) {
            assert!(!d.default_roles.contains(&Member), "{} is admin", d.name);
        }
    }
}

#[test]
fn x_07_the_mounted_routes_equal_the_twin() {
    let twin_routes = twin_identity_routes();
    let t = twin();
    // The routes need the audit port: assemble mod-identity with the audit fixture (DB-free).
    let registry = assemble();
    let routes: Vec<_> = registry
        .routes()
        .filter(|(piece, _)| *piece == mod_identity::ID)
        .flat_map(|(_, r)| r.entries())
        .collect();
    let mut seen = BTreeSet::new();
    for e in routes.iter().copied() {
        let key = (e.method.as_str().to_owned(), e.path.to_owned());
        let (scope, target, _) = twin_routes
            .get(&key)
            .unwrap_or_else(|| panic!("{key:?} is mounted but not in the twin"));
        assert_eq!(e.scope, scope, "{key:?} scope");
        assert_eq!(format!("{:?}", e.target), *target, "{key:?} x-target");
        assert_eq!(e.write, key.0 != "GET", "{key:?}: writes are POST and PUT");
        if e.write {
            let op = &t["paths"][&key.1][key.0.to_lowercase()];
            assert_eq!(op["x-max-body"], 16384, "{key:?} twin x-max-body");
            assert_eq!(
                e.max_body, 16384,
                "{key:?} declares the 16384-byte limit (ID-53)"
            );
            assert!(!e.query, "{key:?}: a write takes no query");
        }
        seen.insert(key);
    }
    // All seven identity operations of the twin are mounted, and nothing else.
    assert_eq!(seen, twin_routes.keys().cloned().collect::<BTreeSet<_>>());
    assert_eq!(seen.len(), 7);
    // R-4: the two lists declare a query; project_get does not (default-deny elsewhere).
    for e in routes.iter().filter(|e| !e.write) {
        assert_eq!(
            e.query,
            e.path != "/v2/projects/{id}",
            "{} query marker",
            e.path
        );
    }
}

#[test]
fn the_migration_is_the_s2_revision_6_file_byte_for_byte() {
    assert_eq!(MIGRATION_0001_UP, s2_identity_sql());
    assert_eq!(MIGRATION_0001_DOWN.trim(), "DROP SCHEMA identity CASCADE;");
    let set = ModIdentity::new().migrations();
    assert_eq!(set.schema, "identity");
    assert_eq!(set.steps.len(), 1);
    assert_eq!(set.steps[0].version, 1);
}

#[test]
fn id_52_query_structs_refuse_unknown_parameters() {
    let ok = serde_json::json!({"limit": 5, "after": "x"});
    assert!(serde_json::from_value::<TeamsQuery>(ok.clone()).is_ok());
    assert!(
        serde_json::from_value::<TeamsQuery>(serde_json::json!({"limit": 5, "name": "x"})).is_err()
    );
    assert!(
        serde_json::from_value::<TeamsQuery>(serde_json::json!({"include_retired": true})).is_err()
    );
    let ok = serde_json::json!({"name": "sophi", "limit": 5, "after": "x"});
    assert!(serde_json::from_value::<WorkersQuery>(ok).is_ok());
    assert!(
        serde_json::from_value::<WorkersQuery>(serde_json::json!({"include_retired": true}))
            .is_err()
    );
    assert!(serde_json::from_value::<WorkersQuery>(serde_json::json!({"limit": -1})).is_err());
}

/// A code this piece declares BEFORE the S3 twin has it (W1-51: `team_lead_in_post`, S3 r2.8 pending
/// item 5, `mvp/api/S3-r2.8-PENDING.md`). A RATCHET, not an exemption: while the twin lacks the code the
/// entry is accepted (its status must equal the manifest's); the moment the twin has it, `ratchet`
/// REFUSES the entry ("remove the pending entry") and the ordinary checks apply (same status, and an
/// operation's `x-errors` names it). An entry whose code the manifest does not declare is refused too.
struct Pending {
    code: &'static str,
    status: u16,
    source: &'static str,
}

const PENDING: &[Pending] = &[Pending {
    code: "team_lead_in_post",
    status: 409,
    source: "W1-51; S3 r2.8 pending item 5",
}];

/// `Ok` when every declared `(code, status)` is in the twin's `x-error-codes.pieces` with the same
/// status, or is a `pending` entry the twin lacks; and every pending entry names a declared code.
fn ratchet(
    pieces: &serde_json::Value,
    declared: &[(&str, u16)],
    pending: &[Pending],
) -> Result<(), String> {
    for (code, status) in declared {
        let in_twin = pieces[*code].as_u64();
        let entry = pending.iter().find(|p| p.code == *code);
        match (in_twin, entry) {
            (Some(s), None) if s == u64::from(*status) => {}
            (Some(s), None) => {
                return Err(format!("{code}: status {status} here, {s} in the twin"));
            }
            (Some(_), Some(p)) => {
                return Err(format!(
                    "{code} is in the twin now: remove the pending entry ({})",
                    p.source
                ));
            }
            (None, Some(p)) if p.status == *status => {}
            (None, Some(p)) => {
                return Err(format!(
                    "{code}: the pending status {} differs from the manifest's {status}",
                    p.status
                ));
            }
            (None, None) => return Err(format!("{code} is not in the twin")),
        }
    }
    for p in pending {
        if !declared.iter().any(|(c, _)| *c == p.code) {
            return Err(format!(
                "the pending entry {} names no code of the manifest: remove it ({})",
                p.code, p.source
            ));
        }
    }
    Ok(())
}

/// Cox W1-31: a code several pieces declare must be identical in (code, status, title, fix). S3 r2.6
/// gives the STATUS only (`x-error-codes.pieces`), so this cell checks what S3 pins (every manifest code
/// is in the twin with the same status, every identity operation's `x-errors` pair resolves to my code or
/// a platform one), and reads the ONE file of shared texts (cox W1-34). A code the twin does not have yet
/// goes through the ratchet above (W1-51).
#[test]
fn the_problem_codes_equal_the_twin_and_the_shared_texts_are_pinned() {
    let t = twin();
    let pieces = &t["x-error-codes"]["pieces"];
    let platform = &t["x-error-codes"]["platform_and_chain"];
    let declared: Vec<(&str, u16)> = MANIFEST
        .problems
        .iter()
        .map(|s| (s.code, s.status))
        .collect();
    ratchet(pieces, &declared, PENDING).unwrap_or_else(|e| panic!("{e}"));
    let mine: BTreeSet<&str> = MANIFEST.problems.iter().map(|p| p.code).collect();
    // A pending code is not named by any operation of the twin yet; once the twin has it the entry is
    // gone (the ratchet) and the code is held to the same "named by an operation" rule as the others.
    let accepted_pending: BTreeSet<&str> = PENDING
        .iter()
        .filter(|p| pieces[p.code].is_null())
        .map(|p| p.code)
        .collect();
    let mut used = BTreeSet::new();
    for (key, (_, _, op_id)) in twin_identity_routes() {
        let op = &t["paths"][&key.1][key.0.to_lowercase()];
        assert_eq!(op["operationId"], op_id.as_str());
        for e in op["x-errors"].as_array().into_iter().flatten() {
            let e = e.as_str().expect("an x-errors entry");
            let (status, code) = e.split_once(' ').expect("'status code'");
            if mine.contains(code) {
                assert_eq!(pieces[code].to_string(), status, "{op_id}: {e}");
                used.insert(code.to_owned());
            } else {
                assert!(
                    platform.get(code).is_some() || pieces.get(code).is_some(),
                    "{op_id}: {e} resolves nowhere in x-error-codes"
                );
            }
        }
    }
    let all: BTreeSet<String> = mine
        .iter()
        .filter(|c| !accepted_pending.contains(*c))
        .map(|c| (*c).to_owned())
        .collect();
    assert_eq!(
        used, all,
        "every code in my manifest is named by one of my operations"
    );
    // W1-34: the title and fix of every code more than one piece declares come from ONE file, and a
    // piece declares them byte for byte. Both shared codes are in my manifest.
    let shared = common::shared_problem_codes();
    let entries = shared["codes"].as_array().expect("codes");
    let mut checked = 0;
    for e in entries {
        let code = e["code"].as_str().expect("code");
        if let Some(spec) = MANIFEST.problems.iter().find(|p| p.code == code) {
            assert_eq!(
                u64::from(spec.status),
                e["status"].as_u64().expect("status"),
                "{code}: status"
            );
            assert_eq!(
                spec.title,
                e["title"].as_str().expect("title"),
                "{code}: title"
            );
            assert_eq!(spec.fix, e["fix"].as_str().expect("fix"), "{code}: fix");
            checked += 1;
        }
    }
    assert_eq!(
        checked, 2,
        "reason_required and target_retired are both declared here and both match the file"
    );
}

/// W1-51: the code, its status and its fixed texts. It is this piece's alone (not a shared code).
#[test]
fn w1_51_team_lead_in_post_is_declared_with_fixed_texts_and_is_not_a_shared_code() {
    let spec = MANIFEST
        .problems
        .iter()
        .find(|p| p.code == "team_lead_in_post")
        .expect("team_lead_in_post is declared in the manifest");
    assert_eq!(
        (spec.status, spec.title, spec.fix),
        (
            409,
            "A team lead cannot be retired",
            "Move the worker to another team or make the worker a member, then retire."
        )
    );
    let shared = common::shared_problem_codes();
    assert!(
        !shared["codes"]
            .as_array()
            .expect("codes")
            .iter()
            .any(|e| e["code"] == "team_lead_in_post"),
        "no other piece declares it, so it is not in the shared texts file"
    );
}

/// The ratchet itself, on synthetic twins (DB-free): it accepts a code the twin lacks only while the
/// entry is pending and agrees in status, and it refuses an entry once the twin has the code.
#[test]
fn the_pending_ratchet_accepts_a_code_the_twin_lacks_and_refuses_what_it_should() {
    let lacking = serde_json::json!({"team_lead_taken": 409});
    let having = serde_json::json!({"team_lead_taken": 409, "team_lead_in_post": 409});
    let other = serde_json::json!({"team_lead_taken": 409, "team_lead_in_post": 422});
    let declared = [("team_lead_taken", 409_u16), ("team_lead_in_post", 409_u16)];
    // The twin lacks the code and the entry is pending with the same status: accepted.
    assert_eq!(ratchet(&lacking, &declared, PENDING), Ok(()));
    // The twin has the code and the entry is still there: refused, with the instruction.
    let e = ratchet(&having, &declared, PENDING).expect_err("the entry must go");
    assert!(e.contains("remove the pending entry"), "{e}");
    // The entry removed and the twin has it with the same status: accepted by the ordinary rule.
    assert_eq!(ratchet(&having, &declared, &[]), Ok(()));
    // The twin has it with another status: refused.
    let e = ratchet(&other, &declared, &[]).expect_err("status differs");
    assert!(e.contains("status 409 here, 422 in the twin"), "{e}");
    // The twin lacks it and there is no entry: refused.
    let e = ratchet(&lacking, &declared, &[]).expect_err("not in the twin");
    assert!(e.contains("team_lead_in_post is not in the twin"), "{e}");
    // The entry's status differs from the manifest's: refused.
    let wrong = [Pending {
        code: "team_lead_in_post",
        status: 422,
        source: "test",
    }];
    let e = ratchet(&lacking, &declared, &wrong).expect_err("pending status differs");
    assert!(e.contains("differs from the manifest's 409"), "{e}");
    // A stale entry (the manifest does not declare the code): refused.
    let e = ratchet(&lacking, &[("team_lead_taken", 409_u16)], PENDING).expect_err("stale entry");
    assert!(e.contains("names no code of the manifest"), "{e}");
}
