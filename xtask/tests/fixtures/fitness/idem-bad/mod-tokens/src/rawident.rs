pub async fn raw() {
    cortex_db::idempotency::r#begin().await;
    cortex_db::idempotency::r#finish().await;
}
