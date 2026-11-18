//! Request bodies (S3 r2.5). Strict decoding: an unknown field, a wrong type or a value outside an enum is refused by
//! serde (422 `invalid_request`, R-3); bounds, patterns and the piece's own codes are the handler's (`validate`).

use cortex_types::{HandoffId, RecordId};
use serde::Deserialize;

/// The twelve record types of S2 r9 section 3.1. `human-message`, `drill-canary`, `drill` and `conversation` are RESERVED:
/// they decode, then the handler answers 422 `type_reserved`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RecordType {
    Decision,
    Lesson,
    Note,
    Status,
    Question,
    Blocker,
    Knowledge,
    Conversation,
    WorkProduct,
    HumanMessage,
    DrillCanary,
    Drill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceKind {
    Conversation,
    File,
    Url,
    Worker,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub kind: SourceKind,
    #[serde(rename = "ref")]
    pub reference: String,
    #[serde(default)]
    pub span: Option<String>,
}

/// `POST /v2/records`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecordCreate {
    #[serde(rename = "type")]
    pub kind: RecordType,
    pub summary: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub fields: Option<serde_json::Map<String, serde_json::Value>>,
    #[serde(default)]
    pub source: Option<Source>,
    #[serde(default)]
    pub affects: Option<Vec<RecordId>>,
    #[serde(default)]
    pub reply_to: Option<RecordId>,
    #[serde(default)]
    pub handoff_id: Option<HandoffId>,
    #[serde(default)]
    pub section: Option<String>,
    #[serde(default)]
    pub valid_until: Option<String>,
}

/// `POST /v2/records/{id}/supersede`. `reason` is handler-required (`reason_required`), so it is optional to serde.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Supersede {
    pub by: RecordId,
    #[serde(default)]
    pub reason: Option<String>,
}

/// `POST /v2/records/{id}/invalidate`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invalidate {
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub undo: Option<bool>,
}
