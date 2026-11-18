// only idem.rs is the wrapper: another file of cortex-http is flagged
pub async fn not_the_wrapper() { cortex_db::idempotency::finish().await; }
