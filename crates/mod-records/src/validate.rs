//! Step 0 of `records.log`: validation that needs no database. Inside step 0 the PER-FIELD checks come BEFORE the
//! cross-field rules (S3 CHANGES section 17; R-16). Every bound is in UTF-8 BYTES (R-1) and a NUL is refused (R-2).
//! A shape or enum violation never gets here: serde refuses it first (422 `invalid_request`, R-3).

use cortex_module::{Problem, SectionName};
use serde_json::{Map, Value};

use crate::problems;
use cortex_types::RecordId;

use crate::request::{RecordCreate, RecordType, Source, SourceKind};

/// Bounds of S2 r9 section 3.1 / S3 r2.5, in bytes.
pub const SUMMARY_MAX: usize = 512;
pub const BODY_MAX: usize = 262_144;
/// The bound on `fields` is on its STORED form (jsonb text, with `": "` and `", "` separators).
pub const FIELDS_MAX: usize = 65_536;
pub const SOURCE_REF_MAX: usize = 2048;
pub const SOURCE_SPAN_MAX: usize = 512;
pub const SECTION_MAX: usize = 128;
pub const ORIENTATION_BODY_MAX: usize = 3072;
pub const READING_PATH_MIN: usize = 5;
pub const READING_PATH_MAX: usize = 10;
pub const LOCATOR_MAX: usize = 256;
/// A `work-product` file item (cox W1-15): a string of 1 to 1024 bytes, no control character.
pub const FILE_ITEM_MAX: usize = 1024;
/// `affects` (cox W1-19): at most 256 items, no duplicates (platform.refs is keyed (from_id, to_id, kind)).
pub const AFFECTS_MAX: usize = 256;

/// A request that passed step 0.
#[derive(Debug, Clone, PartialEq)]
pub struct Validated {
    /// The reserved section named by the request, when there is one.
    pub section: Option<SectionName>,
}

fn invalid(detail: &'static str) -> Problem {
    Problem::invalid_request(422, detail)
}

fn has_nul(s: &str) -> bool {
    s.contains('\0')
}

/// True when a string, a key or any nested value holds a NUL character.
fn value_has_nul(v: &Value) -> bool {
    match v {
        Value::String(s) => has_nul(s),
        Value::Array(a) => a.iter().any(value_has_nul),
        Value::Object(m) => m.iter().any(|(k, v)| has_nul(k) || value_has_nul(v)),
        _ => false,
    }
}

/// The length in bytes of a JSON value in its STORED form: Postgres `jsonb` text puts a space after every `:` and
/// every `,` (S2 pack: `octet_length(fields::text) <= 65536`). Strings are escaped as `serde_json` does, which is
/// what `jsonb` prints for them (control characters as `\u00XX`, no escaping of `/` or DEL).
pub fn stored_len(v: &Value) -> usize {
    match v {
        Value::Null => 4,
        Value::Bool(true) => 4,
        Value::Bool(false) => 5,
        Value::Number(n) => n.to_string().len(),
        Value::String(s) => serde_json::to_string(s).map_or(usize::MAX, |t| t.len()),
        Value::Array(a) => {
            let inner: usize = a.iter().map(stored_len).sum();
            2 + inner + 2 * a.len().saturating_sub(1)
        }
        Value::Object(m) => {
            let inner: usize = m
                .iter()
                .map(|(k, v)| {
                    serde_json::to_string(k).map_or(usize::MAX, |t| t.len()) + 2 + stored_len(v)
                })
                .sum();
            2 + inner + 2 * m.len().saturating_sub(1)
        }
    }
}

/// The reserved types a worker may not write (S2 r9 3.1; `conversation` joins them in 0.2.001, CHANGES section 19).
fn is_reserved(t: RecordType) -> bool {
    matches!(
        t,
        RecordType::HumanMessage
            | RecordType::DrillCanary
            | RecordType::Drill
            | RecordType::Conversation
    )
}

/// `rules-project`, `rules-worker:<36 characters of [0-9a-f-]>` or `orientation`; anything else is `section_invalid`.
fn parse_section(s: &str) -> Result<SectionName, Problem> {
    let bad = || {
        Problem::new(
            &problems::SECTION_INVALID,
            "The section must be rules-project, rules-worker:<id> or orientation.",
        )
    };
    if s.len() > SECTION_MAX {
        return Err(bad());
    }
    if let Some(id) = s.strip_prefix("rules-worker:")
        && !(id.len() == 36
            && id
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase() || b == b'-'))
    {
        return Err(bad());
    }
    SectionName::parse(s).ok_or_else(bad)
}

fn check_source(src: &Source) -> Result<(), Problem> {
    if src.kind == SourceKind::Conversation && RecordId::parse(&src.reference).is_err() {
        return Err(invalid("A conversation source names a record id."));
    }
    if src.reference.is_empty() || src.reference.len() > SOURCE_REF_MAX {
        return Err(invalid("source.ref must be 1 to 2048 bytes."));
    }
    if has_nul(&src.reference) {
        return Err(invalid("source.ref must not contain a NUL character."));
    }
    if let Some(span) = &src.span
        && (span.is_empty() || span.len() > SOURCE_SPAN_MAX || has_nul(span))
    {
        return Err(invalid("source.span must be 1 to 512 bytes."));
    }
    Ok(())
}

/// The orientation section's own rules (S2 r9 section 3; S3 r2.5 conditional): a body of at most 3072 bytes and
/// `fields.reading_path[]` with 5 to 10 entries of at most 256 bytes, starting with `/`, with no control character.
fn check_orientation(req: &RecordCreate) -> Result<(), Problem> {
    let body = req
        .body
        .as_deref()
        .ok_or_else(|| invalid("An orientation record needs a body."))?;
    if body.len() > ORIENTATION_BODY_MAX {
        return Err(invalid("An orientation body is at most 3072 bytes."));
    }
    let fields: &Map<String, Value> = req
        .fields
        .as_ref()
        .ok_or_else(|| invalid("An orientation record needs fields.reading_path."))?;
    let path = fields
        .get("reading_path")
        .and_then(Value::as_array)
        .ok_or_else(|| invalid("fields.reading_path must be an array of 5 to 10 entries."))?;
    if !(READING_PATH_MIN..=READING_PATH_MAX).contains(&path.len()) {
        return Err(invalid("fields.reading_path must have 5 to 10 entries."));
    }
    for entry in path {
        let e = entry
            .as_str()
            .ok_or_else(|| invalid("Each reading_path entry must be a string."))?;
        // Inside one field the order is length, then pattern (R-16, CHANGES section 22).
        if e.len() > LOCATOR_MAX {
            return Err(Problem::new(
                &problems::LOCATOR_TOO_LONG,
                "A reading_path entry is at most 256 bytes.",
            ));
        }
        if !e.starts_with('/') || e.chars().any(|c| (c as u32) < 0x20 || c == '\u{7f}') {
            return Err(invalid(
                "Each reading_path entry must start with / and hold no control character.",
            ));
        }
    }
    Ok(())
}

fn is_leap(y: u32) -> bool {
    (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400)
}

fn days_in_month(y: u32, m: u32) -> u32 {
    match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if is_leap(y) => 29,
        _ => 28,
    }
}

/// An RFC 3339 `date-time` (S3 `format: date-time`) and nothing else (hale F-1, cox W1-36 and W1-39, the one rule for every date-time
/// input; shared vectors `mvp/api/rfc3339-vectors.json`): `YYYY-MM-DDTHH:MM:SS[.fraction]` with `Z` or `+HH:MM` / `-HH:MM`; a real
/// calendar date; year 1970 to 9999; a fraction of 1 to 9 digits; seconds 00 to 59 (Postgres has no leap second); `T` and `Z` in
/// capitals; an offset of minutes 0 to 59 within -12:00 to +14:00 (the real range). Postgres' own `timestamptz` input would also take
/// `infinity`, `epoch`, `now`, a date alone, a space for the `T`, month names; none of those is a date-time, and `infinity` would break
/// every reader of the record. The value is checked, never normalised.
pub fn is_rfc3339(s: &str) -> bool {
    let b = s.as_bytes();
    let num = |from: usize, len: usize| -> Option<u32> {
        let part = b.get(from..from + len)?;
        part.iter()
            .all(u8::is_ascii_digit)
            .then(|| part.iter().fold(0, |n, d| n * 10 + u32::from(d - b'0')))
    };
    let (Some(year), Some(month), Some(day), Some(hour), Some(minute), Some(second)) = (
        num(0, 4),
        num(5, 2),
        num(8, 2),
        num(11, 2),
        num(14, 2),
        num(17, 2),
    ) else {
        return false;
    };
    if b.get(4) != Some(&b'-')
        || b.get(7) != Some(&b'-')
        || b.get(10) != Some(&b'T')
        || b.get(13) != Some(&b':')
        || b.get(16) != Some(&b':')
    {
        return false;
    }
    if !(1970..=9999).contains(&year)
        || !(1..=12).contains(&month)
        || day < 1
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return false;
    }
    let mut i = 19;
    if b.get(i) == Some(&b'.') {
        let start = i + 1;
        i = start;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if !(1..=9).contains(&(i - start)) {
            return false;
        }
    }
    match b.get(i) {
        Some(b'Z') => i + 1 == b.len(),
        Some(sign @ (b'+' | b'-')) => {
            // -12:00 to +14:00: the earliest and the latest offset in use. Hours are checked as a pair with the minutes.
            let max_minutes = if *sign == b'+' { 14 * 60 } else { 12 * 60 };
            b.len() == i + 6
                && b.get(i + 3) == Some(&b':')
                && num(i + 4, 2).is_some_and(|m| m <= 59)
                && num(i + 1, 2)
                    .zip(num(i + 4, 2))
                    .is_some_and(|(h, m)| h * 60 + m <= max_minutes)
        }
        _ => false,
    }
}

/// `fields.files` of a work-product (cox W1-15, S3 r2.6): when present it is an array of strings, each 1 to 1024 bytes with no
/// control character; an item over 1024 bytes is `locator_too_long` (length is checked before the pattern). Absent or empty is
/// `files_required`, decided later with the cross-field rules.
fn check_files(req: &RecordCreate) -> Result<(), Problem> {
    let Some(files) = req.fields.as_ref().and_then(|f| f.get("files")) else {
        return Ok(());
    };
    let items = files
        .as_array()
        .ok_or_else(|| invalid("fields.files must be an array of strings."))?;
    for item in items {
        let s = item
            .as_str()
            .ok_or_else(|| invalid("Each fields.files item must be a string."))?;
        if s.len() > FILE_ITEM_MAX {
            return Err(Problem::new(
                &problems::LOCATOR_TOO_LONG,
                "A fields.files item is at most 1024 bytes.",
            ));
        }
        if s.is_empty() || s.chars().any(|c| (c as u32) < 0x20 || c == '\u{7f}') {
            return Err(invalid(
                "Each fields.files item must be 1 to 1024 bytes with no control character.",
            ));
        }
    }
    Ok(())
}

/// Validate a decoded `RecordCreate`.
pub fn validate_create(req: &RecordCreate) -> Result<Validated, Problem> {
    // ---- per-field checks, in the order of the body's fields ----
    if is_reserved(req.kind) {
        return Err(Problem::new(
            &problems::TYPE_RESERVED,
            "This record type is reserved and cannot be written here.",
        ));
    }
    if req.summary.is_empty() || req.summary.len() > SUMMARY_MAX {
        return Err(invalid("summary must be 1 to 512 bytes."));
    }
    if has_nul(&req.summary) {
        return Err(invalid("summary must not contain a NUL character."));
    }
    if let Some(body) = &req.body {
        if body.len() > BODY_MAX {
            return Err(invalid("body is at most 262144 bytes."));
        }
        if has_nul(body) {
            return Err(invalid("body must not contain a NUL character."));
        }
    }
    if let Some(fields) = &req.fields {
        let v = Value::Object(fields.clone());
        if value_has_nul(&v) {
            return Err(invalid("fields must not contain a NUL character."));
        }
        if stored_len(&v) > FIELDS_MAX {
            return Err(invalid("fields is at most 65536 bytes as stored."));
        }
    }
    if let Some(src) = &req.source {
        check_source(src)?;
    }
    if let Some(affects) = &req.affects {
        // Count first, then (serde already checked each id's format), then uniqueness.
        if affects.len() > AFFECTS_MAX {
            return Err(invalid("affects names at most 256 records."));
        }
        let mut seen = std::collections::BTreeSet::new();
        if !affects.iter().all(|a| seen.insert(a.to_string())) {
            return Err(invalid("affects must not name a record twice."));
        }
    }
    let section = match req.section.as_deref() {
        Some(s) if has_nul(s) => {
            return Err(Problem::new(
                &problems::SECTION_INVALID,
                "The section must not contain a NUL character.",
            ));
        }
        Some(s) => Some(parse_section(s)?),
        None => None,
    };
    if let Some(valid_until) = &req.valid_until
        && !is_rfc3339(valid_until)
    {
        return Err(invalid(
            "valid_until must be an RFC 3339 date-time such as 2030-01-01T00:00:00Z.",
        ));
    }
    if section == Some(SectionName::Orientation) {
        check_orientation(req)?;
    }
    if req.kind == RecordType::WorkProduct {
        check_files(req)?;
    }

    // ---- cross-field rules ----
    if section.is_some() && req.kind != RecordType::Knowledge {
        return Err(invalid("A record in a section must be of type knowledge."));
    }
    match (req.kind, req.source.is_some(), section.is_some()) {
        // The reserved sections waive the source (ruling Q-2); every section name that parses is reserved.
        (RecordType::Knowledge, false, false) => {
            return Err(Problem::new(
                &problems::SOURCE_REQUIRED,
                "A knowledge record needs a source.",
            ));
        }
        (RecordType::Knowledge, _, _) => {}
        (_, true, _) => {
            return Err(Problem::new(
                &problems::SOURCE_NOT_ALLOWED,
                "Only a knowledge record carries a source.",
            ));
        }
        _ => {}
    }
    let fields = req.fields.as_ref();
    match req.kind {
        RecordType::Decision => {
            let why = fields.and_then(|f| f.get("why")).and_then(Value::as_str);
            if why.is_none_or(|w| w.trim().is_empty()) {
                return Err(Problem::new(
                    &problems::WHY_REQUIRED,
                    "A decision needs fields.why.",
                ));
            }
        }
        RecordType::WorkProduct => {
            let files = fields
                .and_then(|f| f.get("files"))
                .and_then(Value::as_array);
            if files.is_none_or(Vec::is_empty) {
                return Err(Problem::new(
                    &problems::FILES_REQUIRED,
                    "A work-product needs fields.files with at least one file.",
                ));
            }
        }
        _ => {}
    }
    Ok(Validated { section })
}
