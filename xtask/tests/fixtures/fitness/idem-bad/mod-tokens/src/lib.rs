// idempotency::begin( in a comment is not a call
pub async fn sneaky() {
    cortex_db::idempotency::begin().await;
}
mod selfl;
mod cfgtest;
mod rawident;
