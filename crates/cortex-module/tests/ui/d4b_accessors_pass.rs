// Delta 4b: a handler reads the identity through accessors.
use cortex_module::Ctx;

fn who(ctx: &Ctx) -> String {
    format!(
        "{} {} {} {:?}",
        ctx.principal().worker,
        ctx.project(),
        ctx.request_id(),
        ctx.actor()
    )
}

fn main() {
    let _ = who;
}
