//! The Cortex HTTP client (Wave 1 piece 10; the skeleton). Contract: S3 r2.7 (the twin `1d2c4b2f`), S1 4.5, cox's
//! W1-43. Test definitions: `mvp/tests/cortex-client-and-cli.md`. The token is never printed, put in an argument
//! list or kept in an error text; the only reader of the process environment is [`env`].

pub mod client;
pub mod display;
pub mod endpoint;
pub mod env;
pub mod error;
pub mod exit;
pub mod idempotency;
pub mod request;
pub mod retry;
pub mod token;

pub use client::{Client, ClientConfig, Response};
pub use endpoint::{Endpoint, EndpointError};
pub use error::{ClientError, Problem, ProblemKind, TransportKind};
pub use idempotency::{IdempotencyKey, KeyError};
pub use request::{Auth, Method, Mutates, PathId, Request};
pub use retry::{RetryPolicy, Sleeper, ThreadSleeper};
pub use token::{Token, TokenError, read_token_file};
