// A route built WITHOUT a scope: the scope is the third argument, so this must not compile.
use axum::http::Method;
use cortex_module::{Routes, TargetRule};

fn main() {
    let _ = Routes::new().read(Method::GET, "/v2/x", TargetRule::None, || async { "x" });
}
