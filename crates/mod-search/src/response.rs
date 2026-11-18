//! The answer (S3 r2.7 `SearchResult` and `SearchHit`). STUB.

use serde::Serialize;

/// One hit.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Hit {
    pub id: String,
    pub r#type: String,
    pub summary: String,
    pub rank: f64,
    pub created_at: String,
    pub state: String,
    pub truncated: bool,
}

/// The answer.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SearchResult {
    pub hits: Vec<Hit>,
    pub mode: &'static str,
}
