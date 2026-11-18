//! The request bodies of the nine routes (S3 r2.5 schemas), as `serde` types.
//!
//! Every body derives `deny_unknown_fields` (R-3: an unknown field is 422 `invalid_request` at
//! decoding). A field that S3 marks *handler-required* (R-6: `packet_incomplete`, `evidence_required`,
//! `risks_required`, `notes_required`, `reason_required`) is an `Option` here: serde must not refuse its
//! absence, because the field's own code has to fire instead of `invalid_request`. The bounds (bytes,
//! patterns, ranges) are the handler's, in [`crate::validate`]; serde only refuses a wrong type, an
//! unknown field, a missing schema-required field and a value outside an enum.

use serde::Deserialize;

/// Decode a word enum from a JSON STRING. `serde_json` answers an enum that meets `null`, an array or a number
/// with an "expected value" error that it classifies as SYNTAX, which the chain would answer 400 "not valid JSON"
/// for a body that is valid JSON; S3 R-3 says a wrong type is 422. Going through a `String` makes every wrong
/// type and every unknown word an ordinary data error (422). The error text is never shown to the caller.
macro_rules! word_enum {
    ($t:ident { $($word:literal => $variant:ident),+ $(,)? }) => {
        impl TryFrom<String> for $t {
            type Error = String;
            fn try_from(s: String) -> Result<Self, String> {
                match s.as_str() {
                    $($word => Ok($t::$variant),)+
                    other => Err(format!("unknown variant `{other}`")),
                }
            }
        }
    };
}

/// What a handoff is for. Decoded from the top-level `kind` only; an absent, misspelt or mis-cased
/// value fails decoding (422) and never defaults to a consult (W1-9; hale's check 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub enum Kind {
    Task,
    Consult,
}

word_enum!(Kind { "task" => Task, "consult" => Consult });

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Task => "task",
            Kind::Consult => "consult",
        }
    }
}

/// `Addressee.kind` in 0.2.001: `team` and `human` fail decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub enum AddresseeKind {
    Worker,
    JobRole,
}

word_enum!(AddresseeKind { "worker" => Worker, "job_role" => JobRole });

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddresseeBody {
    pub kind: AddresseeKind,
    #[serde(rename = "ref")]
    pub r#ref: String,
}

/// `POST /v2/handoffs`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateBody {
    pub kind: Kind,
    pub to: AddresseeBody,
    // Handler-required (R-6): absent, empty or whitespace-only is `packet_incomplete`.
    pub summary: Option<String>,
    pub next_action: Option<String>,
    pub acceptance: Option<String>,
    pub verify: Option<String>,
    pub context: Option<String>,
    #[serde(default)]
    pub files: Vec<String>,
    #[serde(default)]
    pub blockers: Vec<String>,
    #[serde(default)]
    pub decisions: Vec<String>,
    /// 1 to 100, default 50. Wider than `i32` so that an out-of-range value reaches the bound check.
    pub priority: Option<i64>,
    /// RFC 3339 date-time; the handler checks the grammar, Postgres parses it.
    pub due_at: Option<String>,
}

/// The kind of an evidence entry (S3 `Evidence.kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub enum EvidenceKind {
    Test,
    Output,
    Review,
    Artifact,
    Link,
}

word_enum!(EvidenceKind {
    "test" => Test,
    "output" => Output,
    "review" => Review,
    "artifact" => Artifact,
    "link" => Link,
});

impl EvidenceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EvidenceKind::Test => "test",
            EvidenceKind::Output => "output",
            EvidenceKind::Review => "review",
            EvidenceKind::Artifact => "artifact",
            EvidenceKind::Link => "link",
        }
    }
}

/// One evidence entry as sent (S3 `Evidence`): `kind` and `sha256` are schema-required.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvidenceBody {
    pub kind: EvidenceKind,
    #[serde(rename = "ref")]
    pub r#ref: Option<String>,
    pub inline_b64: Option<String>,
    pub sha256: String,
}

/// `POST /v2/handoffs/{id}/return`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReturnBody {
    /// Schema-required (`minLength` 1): an absent one fails decoding, an empty one is the handler's.
    pub summary: String,
    // Handler-required (R-6): absent or `[]` is `evidence_required`.
    pub evidence: Option<Vec<EvidenceBody>>,
    // Handler-required (R-6): absent, empty or whitespace-only is `risks_required`.
    pub risks: Option<String>,
    #[serde(default)]
    pub tests: Vec<String>,
}

/// `POST /v2/handoffs/{id}/release`: `reason` is optional for the holder, required from the lead.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseBody {
    pub reason: Option<String>,
}

/// `POST /v2/handoffs/{id}/cancel`: `reason` is handler-required (R-6).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CancelBody {
    pub reason: Option<String>,
}

/// `POST /v2/handoffs/{id}/accept` and `/rework`: `notes` is optional for accept, handler-required
/// for rework (R-6, `notes_required`).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewBody {
    pub notes: Option<String>,
}
