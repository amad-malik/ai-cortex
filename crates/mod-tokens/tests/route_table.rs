//! STA-4 (DB-free): the routes the piece mounts are exactly S3's six `tokens` routes, with the twin's method, path,
//! scope, `x-target` and body limit; a write is a write, a list takes a query and nothing else does (default-deny
//! marker). RED: `routes()` mounts nothing.
mod common;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::OnceLock;

use common::audit::{AuditPiece, Fault};
use common::ident::IdentityFixture;
use common::twin;
use cortex_module::{Assembly, Piece, Registry};
use cortex_types::{ModuleId, ProblemCodeMinter};
use mod_tokens::ModTokens;

fn registry() -> Registry {
    static M: OnceLock<ProblemCodeMinter> = OnceLock::new();
    let minter =
        M.get_or_init(|| ProblemCodeMinter::take().expect("the first take in this process"));
    let enabled = [
        ModuleId::parse("tokens").expect("id"),
        ModuleId::parse("identity").expect("id"),
        ModuleId::parse(common::audit::ID).expect("id"),
    ];
    let compiled: Vec<Box<dyn Piece>> = vec![
        Box::new(AuditPiece {
            fault: Fault::new(),
        }),
        Box::new(IdentityFixture),
        Box::new(ModTokens::new()),
    ];
    Registry::assemble(Assembly {
        compiled,
        enabled: &enabled,
        minter,
        chain_codes: &["internal_error", "unavailable", "busy", "conflict"],
    })
    .expect("mod-tokens and the fixtures assemble")
}

#[test]
fn sta_4_the_mounted_routes_equal_the_twin() {
    let t = twin();
    let mut want = BTreeMap::new();
    for (path, ops) in t["paths"].as_object().expect("paths") {
        for (method, op) in ops.as_object().expect("ops") {
            if op["x-piece"] == "tokens" {
                want.insert(
                    (method.to_uppercase(), path.clone()),
                    (
                        op["x-scope"].as_str().expect("scope").to_owned(),
                        op["x-target"].as_str().expect("target").to_owned(),
                    ),
                );
            }
        }
    }
    let reg = registry();
    let mounted: Vec<_> = reg
        .routes()
        .filter(|(piece, _)| *piece == "tokens")
        .flat_map(|(_, r)| r.entries())
        .collect();
    let mut seen = BTreeSet::new();
    for e in mounted {
        let key = (e.method.as_str().to_owned(), e.path.to_owned());
        let (scope, target) = want
            .get(&key)
            .unwrap_or_else(|| panic!("{key:?} is mounted but not in the twin"));
        assert_eq!(e.scope, scope, "{key:?} scope");
        assert_eq!(format!("{:?}", e.target), *target, "{key:?} x-target");
        assert_eq!(e.write, key.0 != "GET", "{key:?}: only GET is a read");
        if e.write {
            let op = &t["paths"][&key.1][key.0.to_lowercase()];
            assert_eq!(op["x-max-body"], 16384);
            assert_eq!(e.max_body, 16384, "{key:?} declares the twin's body limit");
            assert!(!e.query);
        }
        assert_eq!(
            e.query,
            key.1 == "/v2/tokens",
            "{key:?}: only the list takes a query"
        );
        seen.insert(key);
    }
    assert_eq!(seen, want.keys().cloned().collect::<BTreeSet<_>>());
    assert_eq!(seen.len(), 6, "all six, nothing else");
}
