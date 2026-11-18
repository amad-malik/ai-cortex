//! Decoding and step-0 validation of the request (S3 r2.7 R-1 to R-3 and R-16; the date-time rule W1-39). Everything here needs no database
//! read: a refusal costs no slot of the cap and no statement. Per field in schema order (`query`, `mode`, `types`, `since`, `until`, `limit`,
//! `include_history`), then the one cross-field rule (`since` after `until`).

use cortex_module::{Problem, StrictError, strict_from_slice};
use serde_json::Value;

use crate::QUERY_TOO_LONG;
use crate::request::{RecordType, SearchRequest};

/// `query` is 1 to 1000 CHARACTERS (R-1: the one limit counted in characters).
const QUERY_MAX_CHARS: usize = 1000;
/// `limit`: 1 to 50, default 10 (never clamped).
const LIMIT_MAX: i64 = 50;
const LIMIT_DEFAULT: u32 = 10;

/// A checked request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Validated {
    pub query: String,
    /// Empty means no filter.
    pub types: Vec<RecordType>,
    /// The checked text, kept as sent (never normalised); the database parses it.
    pub since: Option<String>,
    pub until: Option<String>,
    pub limit: u32,
    pub history: bool,
    since_ns: Option<i128>,
    until_ns: Option<i128>,
}

impl Validated {
    /// `since` equal to `until` (as instants) is an empty interval: the answer is an empty list with no database read.
    pub fn empty_interval(&self) -> bool {
        matches!((self.since_ns, self.until_ns), (Some(s), Some(u)) if s == u)
    }
}

fn invalid(status: u16, detail: &'static str) -> Problem {
    Problem::invalid_request(status, detail)
}

/// Whether the `Content-Type` header names `application/json` (the media type alone is compared, case-insensitively).
fn is_json(content_type: Option<&str>) -> bool {
    content_type
        .and_then(|v| v.split(';').next())
        .is_some_and(|m| m.trim().eq_ignore_ascii_case("application/json"))
}

/// Decode the body: 415 when it is not `application/json`, 400 for bad syntax or invalid UTF-8, 422 for a body that is not an object, a
/// wrong type, a repeated or unknown field or a value outside an enum.
pub fn decode(content_type: Option<&str>, raw: &[u8]) -> Result<SearchRequest, Problem> {
    if !is_json(content_type) {
        return Err(invalid(415, "Send the body as application/json."));
    }
    // Syntax first (400), on a value: invalid UTF-8, a lone surrogate escape, trailing text, a truncated body.
    let value: Value = serde_json::from_slice(raw)
        .map_err(|_| invalid(400, "The body is not well-formed JSON."))?;
    // The strict decoder would accept the sequence form of a struct, so the object check is ours (R-3, W1-35 B).
    if !value.is_object() {
        return Err(invalid(422, "The body must be a JSON object."));
    }
    // The text is well-formed JSON now (the value above parsed), so whatever the typed decode refuses is the SHAPE: a wrong type, a repeated
    // or unknown field, a value outside an enum. serde_json reports a non-string value for an enum as a syntax-category error, which the
    // strict helper would call `Syntax` (400); S3 R-3 says such a value is 422, never 400.
    strict_from_slice::<SearchRequest>(raw).map_err(|e| match e {
        StrictError::Syntax | StrictError::Shape | StrictError::Unknown => {
            invalid(422, "The body does not match this operation's request.")
        }
    })
}

/// Step 0: every check that needs no database read, per field before the cross-field rule.
pub fn validate(req: &SearchRequest) -> Result<Validated, Problem> {
    let chars = req.query.chars().count();
    if chars == 0 {
        return Err(invalid(422, "The query must not be empty."));
    }
    if chars > QUERY_MAX_CHARS {
        return Err(Problem::new(
            &QUERY_TOO_LONG,
            "The query is longer than 1000 characters.",
        ));
    }
    if req.query.contains('\0') {
        return Err(invalid(
            422,
            "A text value must not contain a NUL character.",
        ));
    }
    // `mode` is read as the one-value enum: any other value never decodes.
    let types = req.types.clone().unwrap_or_default();
    for (i, t) in types.iter().enumerate() {
        if types[..i].contains(t) {
            return Err(invalid(422, "A record type may be named once."));
        }
    }
    let since_ns = bound(req.since.as_deref())?;
    let until_ns = bound(req.until.as_deref())?;
    let limit = match req.limit {
        None => LIMIT_DEFAULT,
        Some(n) if (1..=LIMIT_MAX).contains(&n) => u32::try_from(n).unwrap_or(LIMIT_DEFAULT),
        Some(_) => return Err(invalid(422, "The limit must be 1 to 50.")),
    };
    if let (Some(s), Some(u)) = (since_ns, until_ns)
        && s > u
    {
        return Err(invalid(422, "since must not be after until."));
    }
    Ok(Validated {
        query: req.query.clone(),
        types,
        since: req.since.clone(),
        until: req.until.clone(),
        limit,
        history: req.include_history.unwrap_or(false),
        since_ns,
        until_ns,
    })
}

/// A bound of the interval: absent, or a W1-39 date-time as nanoseconds since the epoch (for the cross-field comparison).
fn bound(v: Option<&str>) -> Result<Option<i128>, Problem> {
    let Some(s) = v else { return Ok(None) };
    match is_rfc3339(s).then(|| to_nanos(s)).flatten() {
        Some(n) => Ok(Some(n)),
        None => Err(invalid(
            422,
            "A date-time must be RFC 3339, for example 2026-11-06T13:00:00Z.",
        )),
    }
}

/// Decode then validate.
pub fn check(content_type: Option<&str>, raw: &[u8]) -> Result<Validated, Problem> {
    validate(&decode(content_type, raw)?)
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

/// W1-39 (the one rule for every date-time input of a body, a query or a cursor; shared vectors `mvp/api/rfc3339-vectors.json`, 14 accept
/// and 48 reject): RFC 3339 `YYYY-MM-DDTHH:MM:SS[.f{1,9}](Z|+HH:MM|-HH:MM)`, capital T and Z, a real calendar date, year 1970 to 9999, hour
/// 0-23, minute 0-59, second 0-59 (no leap second), an offset of minutes 0-59 within -12:00 to +14:00, nothing else anywhere. Checked, never
/// normalised.
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

/// Days from 1970-01-01 to the civil date (proleptic Gregorian; the algorithm of H. Hinnant).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The instant of a string [`is_rfc3339`] accepted, in nanoseconds since the epoch. `None` for anything else.
fn to_nanos(s: &str) -> Option<i128> {
    let b = s.as_bytes();
    let num = |from: usize, len: usize| -> Option<i64> {
        let part = b.get(from..from + len)?;
        part.iter()
            .all(u8::is_ascii_digit)
            .then(|| part.iter().fold(0i64, |n, d| n * 10 + i64::from(d - b'0')))
    };
    let days = days_from_civil(num(0, 4)?, num(5, 2)?, num(8, 2)?);
    let secs = days * 86_400 + num(11, 2)? * 3_600 + num(14, 2)? * 60 + num(17, 2)?;
    let mut i = 19;
    let mut frac: i128 = 0;
    if b.get(i) == Some(&b'.') {
        let start = i + 1;
        i = start;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        let digits = &s[start..i];
        frac = digits.parse::<i128>().ok()? * 10_i128.pow(u32::try_from(9 - digits.len()).ok()?);
    }
    let offset = match b.get(i)? {
        b'Z' => 0,
        sign @ (b'+' | b'-') => {
            let o = num(i + 1, 2)? * 3_600 + num(i + 4, 2)? * 60;
            if *sign == b'+' { o } else { -o }
        }
        _ => return None,
    };
    Some(i128::from(secs - offset) * 1_000_000_000 + frac)
}
