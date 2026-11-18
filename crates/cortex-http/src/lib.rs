//! `cortex-http`: the request chain every route passes through (v0.2.001, piece 1).
//!
//! Contract: `mvp/socket/cortex-http-signatures.md` r2 (the chain, problem+json, idempotency) and
//! finn's `mvp/socket/S0-S1-SOCKET-CONTRACT.md` r4 §1.4 (the `Routes` builder and `Ctx`). Test
//! definitions: `mvp/socket/cortex-http-TESTS.md`; every test here names its H-id.
//!
//! `chain` mounts S0's routes behind steps 3 to 10 and builds the whole router. The other modules
//! need nothing from S0: the bearer parser, the problem+json body and the error mapping, the
//! idempotency inputs, the request-id step, RFC 3339 time, the layer that turns any framework
//! error into problem+json, and the layer that writes one log line per refusal.

pub mod bearer;
pub mod chain;
pub mod idem;
pub(crate) mod log;
pub mod normalise;
pub mod problem;
pub mod request_id;
pub mod table;
pub mod time;
