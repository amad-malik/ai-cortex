// The control: a `match` on every TargetRule variant, and a route built WITH a scope and a rule.
use axum::http::Method;
use cortex_module::{Routes, TargetRule};

fn arm(rule: TargetRule) -> u8 {
    match rule {
        TargetRule::None => 0,
        TargetRule::SelfWorker => 1,
        TargetRule::SelfOrProjectLead => 2,
        TargetRule::NotProjectLead => 3,
        TargetRule::RetireTarget => 4,
        TargetRule::TokenTarget => 5,
        TargetRule::Lineage => 6,
        TargetRule::TaskAddressee => 7,
    }
}

fn main() {
    let _ = Routes::new().read(Method::GET, "/v2/x", "x:read", TargetRule::None, || async { "x" });
    let _ = arm(TargetRule::Lineage);
}
