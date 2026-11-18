//! Shared test helpers. DB-free doubles live in [`mem`].
#![allow(dead_code)]

pub mod mem;

use std::path::PathBuf;

const MVP: &str = "/path/to/docs/mvp";

/// The S3 r2.6 JSON twin (`a31fa244…`). The tokens operations and schemas are the same as in r2.5 (`bb8775e8…`),
/// which the r5 cell file was written against. `CORTEX_S3_TWIN` overrides the pinned path.
pub fn twin() -> serde_json::Value {
    let p = std::env::var("CORTEX_S3_TWIN").unwrap_or_else(|_| {
        format!("{MVP}/wave1-pins-20261106-S3r26/S3/cortex-v2-skeleton.openapi.r2.6-a31fa244.json")
    });
    let text = std::fs::read_to_string(PathBuf::from(&p)).expect("the S3 twin is readable");
    serde_json::from_str(&text).expect("the S3 twin is JSON")
}

/// The S2 `tokens.sql` (`8357e64a…`, unchanged in revisions 6 and 7). `CORTEX_S2_TOKENS_SQL` overrides the path.
pub fn s2_tokens_sql() -> String {
    let p = std::env::var("CORTEX_S2_TOKENS_SQL")
        .unwrap_or_else(|_| format!("{MVP}/wave1-pins-20261106-S2r7/S2/tokens.sql"));
    std::fs::read_to_string(PathBuf::from(&p)).expect("the S2 pin is readable")
}

/// The S2 `identity.sql` (`c08fab0d…`, the same bytes in revisions 6 and 7), which `tokens.sql` needs first.
/// `CORTEX_S2_IDENTITY_SQL` overrides the path.
pub fn s2_identity_sql() -> String {
    let p = std::env::var("CORTEX_S2_IDENTITY_SQL")
        .unwrap_or_else(|_| format!("{MVP}/wave1-pins-20261106-S2r7/S2/identity.sql"));
    std::fs::read_to_string(PathBuf::from(&p)).expect("the S2 identity pin is readable")
}

/// cox's one source of the title and fix of the shared problem codes (W1-34).
pub fn shared_problem_codes() -> serde_json::Value {
    let p = std::env::var("CORTEX_SHARED_PROBLEM_CODES")
        .unwrap_or_else(|_| format!("{MVP}/api/shared-problem-codes.json"));
    let text =
        std::fs::read_to_string(PathBuf::from(&p)).expect("the shared codes file is readable");
    serde_json::from_str(&text).expect("JSON")
}

/// The crate's own directory.
pub fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

pub mod db;

pub mod audit;
pub mod chain;
pub mod ident;
