// Delta 4: `Ctx::authorize` takes no rule: the rule is the route's declared one, so a handler
// cannot prove itself against a weaker rule.
use cortex_module::{Ctx, Target, TargetRule};

fn weaker(ctx: &Ctx) {
    let _ = ctx.authorize(&Target::default(), TargetRule::None, ());
}

fn main() {
    let _ = weaker;
}
