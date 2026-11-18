// Delta 4b: the identity is read through accessors, not fields (one source: the authority).
use cortex_module::Ctx;

fn worker(ctx: &Ctx) {
    let _ = ctx.principal.worker;
    let _ = ctx.project;
    let _ = ctx.request_id;
}

fn main() {
    let _ = worker;
}
