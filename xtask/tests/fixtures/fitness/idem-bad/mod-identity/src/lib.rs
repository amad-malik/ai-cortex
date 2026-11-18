use cortex_db::idempotency::{begin as b, finish};
pub fn x() { let _ = (b, finish); }
mod multi;
