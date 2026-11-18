// Delta 4b: a clone cannot carry another request id: the field is private.
use cortex_module::Ctx;
use cortex_types::{RequestId, SystemClock};

fn alter(ctx: &Ctx) -> Ctx {
    let mut c = ctx.clone();
    c.request_id = RequestId::new(&SystemClock);
    c
}

fn main() {
    let _ = alter;
}
