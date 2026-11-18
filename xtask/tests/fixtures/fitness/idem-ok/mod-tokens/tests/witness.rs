// A test may call the store directly: tests are not linked into the binary (cox's boundary ruling).
use cortex_db::idempotency::{begin as b, finish};
async fn real_calls() {
    cortex_db::idempotency::begin().await;
    cortex_db::idempotency::finish().await;
}
