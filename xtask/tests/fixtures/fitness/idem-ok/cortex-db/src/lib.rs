pub mod idempotency {
    pub async fn begin() {}
    pub async fn finish() {}
}
#[cfg(test)]
mod tests {
    async fn own_tests_may_call_both() {
        crate::idempotency::begin().await;
        crate::idempotency::finish().await;
    }
}
