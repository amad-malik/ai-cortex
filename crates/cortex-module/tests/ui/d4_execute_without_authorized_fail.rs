// Delta 4: `execute` takes the `Authorized<T>`; an execute closure that does not (the old one-closure
// shape) does not type-check, so a write cannot run without the proof.
use axum::http::Method;
use cortex_module::{Reply, Routes, Target, TargetRule};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Body {
    n: i64,
}

fn main() {
    let _routes = Routes::new().write(
        Method::POST,
        "/v2/x",
        "x:write",
        TargetRule::None,
        |_tx, ctx, _b: &Body| Box::pin(async move { ctx.authorize(&Target::default(), ()) }),
        |_tx, _ctx, b: Body| Box::pin(async move { Ok(Reply::ok(b.n)) }),
    );
}
