//! The request (S3 r2.7 `SearchRequest`): a closed object. A property that is present is never `null` (none is nullable), so every
//! optional one is read through [`present`].

use serde::{Deserialize, Deserializer};

/// The only mode in 0.2.001. Read from a STRING only: serde's default enum decoder also takes the object form `{"keyword": null}`, which
/// S3 does not allow (sam MS7-01), so the decoder goes through `String` and rejects every other JSON type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub enum Mode {
    Keyword,
}

impl TryFrom<String> for Mode {
    type Error = &'static str;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        match s.as_str() {
            "keyword" => Ok(Mode::Keyword),
            _ => Err("mode must be \"keyword\""),
        }
    }
}

/// The twelve record types of the S3 enum (S2 §3.1 plus the reserved ones). Read from a STRING only (see [`Mode`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(try_from = "String")]
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

impl RecordType {
    /// Every type, in the S3 enum's order.
    pub const ALL: [RecordType; 12] = [
        RecordType::Decision,
        RecordType::Lesson,
        RecordType::Note,
        RecordType::Status,
        RecordType::Question,
        RecordType::Blocker,
        RecordType::Knowledge,
        RecordType::Conversation,
        RecordType::WorkProduct,
        RecordType::HumanMessage,
        RecordType::DrillCanary,
        RecordType::Drill,
    ];
}

impl TryFrom<String> for RecordType {
    type Error = &'static str;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        RecordType::ALL
            .into_iter()
            .find(|t| t.as_str() == s)
            .ok_or("not a record type")
    }
}

impl RecordType {
    /// The stored spelling (S2 `records.records.type`).
    pub fn as_str(self) -> &'static str {
        match self {
            RecordType::Decision => "decision",
            RecordType::Lesson => "lesson",
            RecordType::Note => "note",
            RecordType::Status => "status",
            RecordType::Question => "question",
            RecordType::Blocker => "blocker",
            RecordType::Knowledge => "knowledge",
            RecordType::Conversation => "conversation",
            RecordType::WorkProduct => "work-product",
            RecordType::HumanMessage => "human-message",
            RecordType::DrillCanary => "drill-canary",
            RecordType::Drill => "drill",
        }
    }
}

/// `Some(value)` when the property is present, and an error (not `None`) when it is `null`.
fn present<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(d).map(Some)
}

/// `POST /v2/search`'s body.
#[derive(Debug, Clone, Deserialize)]
pub struct SearchRequest {
    pub query: String,
    #[serde(default, deserialize_with = "present")]
    pub mode: Option<Mode>,
    #[serde(default, deserialize_with = "present")]
    pub types: Option<Vec<RecordType>>,
    #[serde(default, deserialize_with = "present")]
    pub since: Option<String>,
    #[serde(default, deserialize_with = "present")]
    pub until: Option<String>,
    #[serde(default, deserialize_with = "present")]
    pub limit: Option<i64>,
    #[serde(default, deserialize_with = "present")]
    pub include_history: Option<bool>,
}
