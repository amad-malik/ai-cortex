// Delta 4: how a piece writes a route. The legal two-closure form, and write_open, compile.
use axum::http::Method;
use cortex_module::{Authorized, Reply, Routes, Target, TargetRule};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Body {
    n: i64,
}

struct Decided {
    n: i64,
}

fn main() {
    let _routes = Routes::new()
        .write(
            Method::POST,
            "/v2/x/{id}",
            "x:write",
            TargetRule::SelfWorker,
            |_tx, ctx, b: &Body| {
                Box::pin(async move { ctx.authorize(&Target::default(), Decided { n: b.n }) })
            },
            |_tx, _ctx, a: Authorized<Decided>, b: Body| {
                Box::pin(async move { Ok(Reply::ok(a.value().n + b.n)) })
            },
        )
        .write_open(Method::POST, "/v2/y", "y:write", |_tx, _ctx, b: Body| {
            Box::pin(async move { Ok(Reply::ok(b.n)) })
        });
}
