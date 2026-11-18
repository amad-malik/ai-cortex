//! Step 0 of S3's R-16: the checks that need no database read. `tests/w1_handoffs_p1_validate.rs` is the
//! spec (HAND-02, HAND-04, HAND-41 to HAND-44, HAND-53, HAND-62, HAND-71, HAND-104, HAND-120).
//!
//! Order (S3-r2-CHANGES section 17): inside step 0 every PER-FIELD check (length in bytes, NUL, pattern,
//! range, the field's own code) comes before any CROSS-FIELD rule (Evidence's `ref` or `inline_b64`, the
//! digest, the aggregate limits); the first refusal wins, and within a group the order is the schema's.
//! Nothing here reads a row, so a refusal depends on no state, and none of them carries a request value.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use cortex_module::Problem;
use cortex_types::{Sha256, WorkerId};
use sha2::Digest as _;

use crate::body::{
    AddresseeKind, CancelBody, CreateBody, EvidenceBody, EvidenceKind, Kind, ReleaseBody,
    ReturnBody, ReviewBody,
};
use crate::problem::{invalid, own};

pub const MAX_SUMMARY: usize = 2048;
pub const MAX_PACKET_TEXT: usize = 8192;
pub const MAX_AGGREGATE: usize = 65_536;
pub const MAX_RETURN_SUMMARY: usize = 32_768;
pub const MAX_RISKS: usize = 8192;
pub const MAX_NOTES: usize = 8192;
pub const MAX_REASON: usize = 512;
pub const MAX_EVIDENCE: usize = 64;
pub const MAX_EVIDENCE_REF: usize = 2048;
/// Decoded bytes, per entry and per return.
pub const MAX_INLINE: usize = 1024 * 1024;

/// Who a new handoff is addressed to, once validated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum To {
    Worker(WorkerId),
    JobRole(String),
}

/// A create body that passed step 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateOk {
    pub kind: Kind,
    pub to: To,
    pub summary: String,
    pub next_action: String,
    pub acceptance: String,
    pub verify: Option<String>,
    pub context: Option<String>,
    pub files: Vec<String>,
    pub blockers: Vec<String>,
    pub decisions: Vec<String>,
    pub priority: i32,
    pub due_at: Option<String>,
}

/// One evidence entry that passed step 0: exactly one of `reference` and `inline`; the digest is the
/// one stored (for inline bytes, the one the server computed and found equal to the one sent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceOk {
    pub kind: EvidenceKind,
    pub reference: Option<String>,
    pub inline: Option<Vec<u8>>,
    pub sha256: Sha256,
    /// Decoded length for inline bytes, 0 for a reference.
    pub size: usize,
}

/// A return body that passed step 0.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReturnOk {
    pub summary: String,
    pub evidence: Vec<EvidenceOk>,
    pub risks: String,
    pub tests: Vec<String>,
}

// ---- text ----

fn is_blank(s: &str) -> bool {
    s.chars().all(char::is_whitespace)
}

/// A text field's bounds, in the order of R-16: its length in BYTES, then NUL.
fn bounded(field: &str, s: &str, max: usize) -> Result<(), Problem> {
    if s.len() > max {
        return Err(invalid(field, "longer than its limit in bytes"));
    }
    if s.contains('\0') {
        return Err(invalid(field, "contains a NUL character"));
    }
    Ok(())
}

/// A handler-required text: absent, or empty or whitespace-only, is the field's own code, last.
fn required<'a>(
    field: &str,
    value: &'a Option<String>,
    max: usize,
    code: &'static str,
    detail: &'static str,
) -> Result<&'a str, Problem> {
    match value {
        None => Err(own(code, detail)),
        Some(s) => {
            bounded(field, s, max)?;
            if is_blank(s) {
                Err(own(code, detail))
            } else {
                Ok(s)
            }
        }
    }
}

/// `true` when `s` is a job-role label: `^[a-z][a-z0-9-]{0,31}$`, anchored on the WHOLE string (S3 R-18,
/// F-11: a trailing newline does not match).
pub fn is_job_role_label(s: &str) -> bool {
    let b = s.as_bytes();
    if b.is_empty() || b.len() > 32 || !b[0].is_ascii_lowercase() {
        return false;
    }
    b[1..]
        .iter()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// The ONE rule for every date-time input of a body, a query or a cursor (cox W1-39, answering hale's F-1):
/// RFC 3339 `YYYY-MM-DDTHH:MM:SS[.f{1,9}](Z|+HH:MM|-HH:MM)`, upper-case `T` and `Z`, a real calendar date, year 1970 to
/// 9999, hour 0-23, minute 0-59, second 0-59 (no leap second), offset minutes 0-59 and the offset within -12:00 ..
/// +14:00 (the real range). Anything else, NUL included, is refused. It is refused, not normalised to UTC. Every value
/// it accepts is one Postgres can hold as a `timestamptz`; `tests/w1_handoffs_p5_rfc3339_db.rs` checks that against
/// the database.
pub(crate) fn is_rfc3339(s: &str) -> bool {
    let b = s.as_bytes();
    if !s.is_ascii() || b.len() < 20 {
        return false;
    }
    let num = |from: usize, to: usize| -> Option<u32> {
        let part = &s[from..to];
        if part.bytes().all(|c| c.is_ascii_digit()) {
            part.parse().ok()
        } else {
            None
        }
    };
    let (Some(y), Some(mo), Some(d), Some(h), Some(mi), Some(se)) = (
        num(0, 4),
        num(5, 7),
        num(8, 10),
        num(11, 13),
        num(14, 16),
        num(17, 19),
    ) else {
        return false;
    };
    if b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' || b[16] != b':' {
        return false;
    }
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days = match mo {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    if !(1970..=9999).contains(&y) || d == 0 || d > days || h > 23 || mi > 59 || se > 59 {
        return false;
    }
    let mut i = 19;
    if b[i] == b'.' {
        let start = i + 1;
        i = start;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == start || i - start > 9 {
            return false;
        }
    }
    match &s[i..] {
        "Z" => true,
        off if off.len() == 6 && (off.starts_with('+') || off.starts_with('-')) => {
            let ob = off.as_bytes();
            let digits = |from: usize, to: usize| -> Option<u32> {
                let part = &off[from..to];
                if part.bytes().all(|c| c.is_ascii_digit()) {
                    part.parse().ok()
                } else {
                    None
                }
            };
            let (Some(oh), Some(om)) = (digits(1, 3), digits(4, 6)) else {
                return false;
            };
            // East to +14:00, west to -12:00: the real range of offsets.
            let limit = if ob[0] == b'+' { 14 * 60 } else { 12 * 60 };
            ob[3] == b':' && om <= 59 && oh * 60 + om <= limit
        }
        _ => false,
    }
}

/// The length of an array as S2's table CHECK counts it: the `jsonb` text of a non-empty array
/// (`["a", "b"]`: brackets, each element JSON-escaped and quoted, `, ` between); an empty array is stored
/// as NULL and counts nothing.
fn jsonb_text_len(items: &[String]) -> usize {
    if items.is_empty() {
        return 0;
    }
    let mut n = 2;
    for (i, s) in items.iter().enumerate() {
        if i > 0 {
            n += 2;
        }
        n += serde_json::to_string(s).map_or(usize::MAX / 8, |j| j.len());
    }
    n
}

// ---- create ----

pub fn validate_create(b: CreateBody) -> Result<CreateOk, Problem> {
    // Per-field checks, in the schema's order: kind (decoded), to, summary, next_action, acceptance,
    // verify, context, files, blockers, decisions, priority, due_at.
    let to = match b.to.kind {
        AddresseeKind::JobRole => {
            bounded("to.ref", &b.to.r#ref, 256)?;
            if !is_job_role_label(&b.to.r#ref) {
                return Err(invalid("to.ref", "not a job-role label"));
            }
            To::JobRole(b.to.r#ref.clone())
        }
        AddresseeKind::Worker => {
            bounded("to.ref", &b.to.r#ref, 256)?;
            To::Worker(
                WorkerId::parse(&b.to.r#ref).map_err(|_| invalid("to.ref", "not a worker id"))?,
            )
        }
    };
    let summary = required(
        "summary",
        &b.summary,
        MAX_SUMMARY,
        "packet_incomplete",
        "A handoff needs a summary, a next action and acceptance criteria.",
    )?
    .to_owned();
    let next_action = required(
        "next_action",
        &b.next_action,
        MAX_PACKET_TEXT,
        "packet_incomplete",
        "A handoff needs a summary, a next action and acceptance criteria.",
    )?
    .to_owned();
    let acceptance = required(
        "acceptance",
        &b.acceptance,
        MAX_PACKET_TEXT,
        "packet_incomplete",
        "A handoff needs a summary, a next action and acceptance criteria.",
    )?
    .to_owned();
    if let Some(v) = &b.verify {
        bounded("verify", v, MAX_PACKET_TEXT)?;
    }
    if let Some(c) = &b.context {
        bounded("context", c, MAX_PACKET_TEXT)?;
    }
    for (field, items) in [
        ("files", &b.files),
        ("blockers", &b.blockers),
        ("decisions", &b.decisions),
    ] {
        for item in items {
            if item.contains('\0') {
                return Err(invalid(field, "contains a NUL character"));
            }
        }
    }
    let priority = match b.priority {
        None => 50,
        Some(p) if (1..=100).contains(&p) => i32::try_from(p).expect("1 to 100 fits"),
        Some(_) => return Err(invalid("priority", "must be 1 to 100")),
    };
    if let Some(d) = &b.due_at
        && !is_rfc3339(d)
    {
        return Err(invalid("due_at", "not an RFC 3339 date-time"));
    }
    // Cross-field: `files` + `blockers` + `decisions` together (S2's table CHECK, S3 x-aggregate-max-bytes).
    let total =
        jsonb_text_len(&b.files) + jsonb_text_len(&b.blockers) + jsonb_text_len(&b.decisions);
    if total > MAX_AGGREGATE {
        return Err(invalid(
            "files",
            "files, blockers and decisions together are too long",
        ));
    }
    Ok(CreateOk {
        kind: b.kind,
        to,
        summary,
        next_action,
        acceptance,
        verify: b.verify,
        context: b.context,
        files: b.files,
        blockers: b.blockers,
        decisions: b.decisions,
        priority,
        due_at: b.due_at,
    })
}

// ---- return ----

/// 64 lowercase hex characters to the 32 bytes they say.
fn digest_from_hex(s: &str) -> Option<Sha256> {
    let b = s.as_bytes();
    if b.len() != 64
        || !b
            .iter()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(c))
    {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(Sha256(out))
}

/// An entry after its per-field checks: what is left for the cross-field rules.
struct Staged<'a> {
    body: &'a EvidenceBody,
    digest: Sha256,
    decoded: Option<Vec<u8>>,
}

fn stage(e: &EvidenceBody) -> Result<Staged<'_>, Problem> {
    if let Some(r) = &e.r#ref {
        if r.is_empty() {
            return Err(invalid("evidence.ref", "must not be empty"));
        }
        bounded("evidence.ref", r, MAX_EVIDENCE_REF)?;
    }
    let digest = digest_from_hex(&e.sha256)
        .ok_or_else(|| invalid("evidence.sha256", "must be 64 lowercase hex characters"))?;
    let decoded = match &e.inline_b64 {
        None => None,
        Some(text) => {
            // Refuse before decoding what could not fit, so a huge string costs nothing.
            if text.len() > (MAX_INLINE / 3 + 1) * 4 {
                return Err(invalid("evidence.inline_b64", "longer than its limit"));
            }
            let bytes = B64
                .decode(text)
                .map_err(|_| invalid("evidence.inline_b64", "not valid base64"))?;
            if bytes.len() > MAX_INLINE {
                return Err(invalid("evidence.inline_b64", "longer than its limit"));
            }
            Some(bytes)
        }
    };
    Ok(Staged {
        body: e,
        digest,
        decoded,
    })
}

pub fn validate_return(b: ReturnBody) -> Result<ReturnOk, Problem> {
    // Per-field checks, in the schema's order: summary, evidence, risks, tests.
    if b.summary.is_empty() {
        return Err(invalid("summary", "must not be empty"));
    }
    bounded("summary", &b.summary, MAX_RETURN_SUMMARY)?;
    let entries = match &b.evidence {
        Some(list) if !list.is_empty() => list,
        _ => {
            return Err(own(
                "evidence_required",
                "A return needs at least one evidence entry.",
            ));
        }
    };
    if entries.len() > MAX_EVIDENCE {
        return Err(invalid("evidence", "more entries than the limit"));
    }
    let staged = entries.iter().map(stage).collect::<Result<Vec<_>, _>>()?;
    let risks = required(
        "risks",
        &b.risks,
        MAX_RISKS,
        "risks_required",
        "A return states its risks, or the word none.",
    )?
    .to_owned();
    for t in &b.tests {
        if t.contains('\0') {
            return Err(invalid("tests", "contains a NUL character"));
        }
    }

    // Cross-field: exactly one of `ref` and `inline_b64` per entry (S3: evidence_source_invalid)...
    for s in &staged {
        if s.body.r#ref.is_some() == s.decoded.is_some() {
            return Err(own(
                "evidence_source_invalid",
                "Give each evidence entry exactly one of ref and inline_b64.",
            ));
        }
    }
    // ...then the digest of the bytes the server holds...
    let mut total = 0usize;
    let mut evidence = Vec::with_capacity(staged.len());
    for s in staged {
        let (reference, inline, size) = match (&s.body.r#ref, s.decoded) {
            (Some(r), None) => (Some(r.clone()), None, 0),
            (None, Some(bytes)) => {
                let computed: [u8; 32] = sha2::Sha256::digest(&bytes).into();
                if Sha256(computed) != s.digest {
                    return Err(own(
                        "evidence_hash_mismatch",
                        "The digest does not match the attached bytes.",
                    ));
                }
                total += bytes.len();
                let len = bytes.len();
                (None, Some(bytes), len)
            }
            _ => unreachable!("exactly one source was checked above"),
        };
        evidence.push(EvidenceOk {
            kind: s.body.kind,
            reference,
            inline,
            sha256: s.digest,
            size,
        });
    }
    // ...and the inline bytes of the whole return.
    if total > MAX_INLINE {
        return Err(invalid("evidence", "inline bytes together are too long"));
    }
    Ok(ReturnOk {
        summary: b.summary,
        evidence,
        risks,
        tests: b.tests,
    })
}

// ---- reasons and notes ----

/// Cancel: `reason` is required, 1 to 512 bytes (stateless, so it fires before the 404 and the 409).
pub fn validate_cancel(b: CancelBody) -> Result<String, Problem> {
    required(
        "reason",
        &b.reason,
        MAX_REASON,
        "reason_required",
        "A cancel needs a reason of 1 to 512 bytes.",
    )
    .map(str::to_owned)
}

/// An optional text: absent or blank is `None`; the bounds hold when there is text.
fn optional(field: &str, value: Option<String>, max: usize) -> Result<Option<String>, Problem> {
    match value {
        None => Ok(None),
        Some(s) => {
            bounded(field, &s, max)?;
            Ok(if is_blank(&s) { None } else { Some(s) })
        }
    }
}

/// Release: `reason` is optional here; its bounds are checked when present. Whether the lead needs one
/// is decided later, with the caller's role (it needs a read).
pub fn validate_release(b: ReleaseBody) -> Result<Option<String>, Problem> {
    optional("reason", b.reason, MAX_REASON)
}

/// Accept: `notes` optional, at most 8,192 bytes.
pub fn validate_accept(b: ReviewBody) -> Result<Option<String>, Problem> {
    optional("notes", b.notes, MAX_NOTES)
}

/// Rework: `notes` required (`notes_required`), at most 8,192 bytes.
pub fn validate_rework(b: ReviewBody) -> Result<String, Problem> {
    required(
        "notes",
        &b.notes,
        MAX_NOTES,
        "notes_required",
        "A rework needs notes saying what to change.",
    )
    .map(str::to_owned)
}
