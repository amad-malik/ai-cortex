// Delta 4b: a clone cannot carry another project into `Ctx::begin`: the field is private.
use cortex_module::Ctx;
use cortex_types::{ProjectId, SystemClock};

fn alter(ctx: &Ctx) -> Ctx {
    let mut c = ctx.clone();
    c.project = ProjectId::new(&SystemClock);
    c
}

fn main() {
    let _ = alter;
}
