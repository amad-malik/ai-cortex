fn main() { let _ = "a string"; futures::executor::block_on(cortex_db::idempotency::begin()); }
