// A route built WITHOUT a TargetRule: `Routes::read` takes one, so this must not compile.
use axum::http::Method;
use cortex_module::Routes;

fn main() {
    let _ = Routes::new().read(Method::GET, "/v2/x", "x:read", || async { "x" });
}
