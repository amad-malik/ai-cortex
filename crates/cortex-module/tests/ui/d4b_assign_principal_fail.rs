// Delta 4b: a clone cannot carry an altered principal: the field is private.
use cortex_module::{Ctx, Role};

fn alter(ctx: &Ctx) -> Ctx {
    let mut c = ctx.clone();
    c.principal.role = Role::ProjectLead;
    c
}

fn main() {
    let _ = alter;
}
