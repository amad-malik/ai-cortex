// A `match` on TargetRule that forgets a variant (TaskAddressee): non-exhaustive patterns, so a new
// variant added to the enum without an arm breaks the build of every such match.
use cortex_module::TargetRule;

fn arm(rule: TargetRule) -> u8 {
    match rule {
        TargetRule::None => 0,
        TargetRule::SelfWorker => 1,
        TargetRule::SelfOrProjectLead => 2,
        TargetRule::NotProjectLead => 3,
        TargetRule::RetireTarget => 4,
        TargetRule::TokenTarget => 5,
        TargetRule::Lineage => 6,
    }
}

fn main() {
    let _ = arm(TargetRule::None);
}
