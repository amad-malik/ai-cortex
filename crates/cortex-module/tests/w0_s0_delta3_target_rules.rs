//! Known delta 3 (cox's row c9a20f06): `TargetRule` is exactly sophi's eight variants (table 3.2 in
//! `mvp/tests/mod-identity-and-mod-policy.md`; S3 R-17, `x-target` on every operation). The truth
//! table is mod-policy's; S0 only names the rules. DB-free.
//!
//! The exhaustive `match` below pins the set: adding a variant (the match is no longer
//! exhaustive) or removing one (an unknown variant) fails to compile.
//!
//! RED: `TargetRule` has only `None`, so this file does not compile (E0599 on the seven missing
//! variants).
use std::collections::HashSet;

use cortex_module::TargetRule;

/// The pin, in table 3.2's order.
fn sophi_name(r: TargetRule) -> &'static str {
    match r {
        TargetRule::None => "None",
        TargetRule::SelfWorker => "SelfWorker",
        TargetRule::SelfOrProjectLead => "SelfOrProjectLead",
        TargetRule::NotProjectLead => "NotProjectLead",
        TargetRule::RetireTarget => "RetireTarget",
        TargetRule::TokenTarget => "TokenTarget",
        TargetRule::Lineage => "Lineage",
        TargetRule::TaskAddressee => "TaskAddressee",
    }
}

const ALL: [TargetRule; 8] = [
    TargetRule::None,
    TargetRule::SelfWorker,
    TargetRule::SelfOrProjectLead,
    TargetRule::NotProjectLead,
    TargetRule::RetireTarget,
    TargetRule::TokenTarget,
    TargetRule::Lineage,
    TargetRule::TaskAddressee,
];

#[test]
fn w0_d3_the_target_rules_are_exactly_sophis_eight() {
    let names: Vec<&str> = ALL.iter().map(|r| sophi_name(*r)).collect();
    assert_eq!(
        names,
        [
            "None",
            "SelfWorker",
            "SelfOrProjectLead",
            "NotProjectLead",
            "RetireTarget",
            "TokenTarget",
            "Lineage",
            "TaskAddressee",
        ]
    );
    let distinct: HashSet<TargetRule> = ALL.iter().copied().collect();
    assert_eq!(distinct.len(), 8, "eight distinct rules");
    // A log line names a rule exactly as the table and the S3 YAML's x-target do.
    for r in ALL {
        assert_eq!(format!("{r:?}"), sophi_name(r));
    }
}
