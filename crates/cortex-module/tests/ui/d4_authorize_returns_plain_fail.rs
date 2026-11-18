// Delta 4: the `authorize` closure must return an `Authorized<T>`; returning the bare value (or
// `Ok(())`) skips the check and does not type-check.
use axum::http::Method;
use cortex_module::{Authorized, Reply, Routes, TargetRule};
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
        |_tx, _ctx, _b: &Body| Box::pin(async move { Ok(()) }),
        |_tx, _ctx, _a: Authorized<()>, b: Body| Box::pin(async move { Ok(Reply::ok(b.n)) }),
    );
}
