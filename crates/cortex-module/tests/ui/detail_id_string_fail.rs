// Delta 5: a string is not an id: `id` takes `impl Into<DetailRef>`, which only the six id types
// implement, so a secret cannot be passed as an "id".
use cortex_module::AuditDetails;

fn main() {
    let _ = AuditDetails::new().id("worker", "ctx2_not_an_id");
}
