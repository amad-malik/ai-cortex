pub async fn lookup() {
    cortex_db::idempotency::begin().await;
}
pub async fn finish() {
    cortex_db::idempotency::finish().await;
}
