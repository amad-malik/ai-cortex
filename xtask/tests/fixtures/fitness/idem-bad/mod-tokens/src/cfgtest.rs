#[cfg(test)]
mod t {
    async fn inside_src_stays_scanned() {
        cortex_db::idempotency::finish().await;
    }
}
