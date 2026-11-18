//! The strict decoders (Wave 1; kai d2a88cf7 and cox abde4db0): [`strict_from_slice`] for a JSON body and
//! [`strict_from_query`] for a query string. ONE helper, shared by `Routes::write`'s body decode, by
//! [`StrictQuery`](crate::StrictQuery) and by `cortex-http`.
//!
//! Both read the input through `serde_ignored`, so an unknown field is refused for EVERY plain struct and
//! every externally tagged enum (serde's default), including one a piece forgot to mark
//! `#[serde(deny_unknown_fields)]` (`w1_s_2`, `w1_s_12`). The refusal is a [`StrictError`], which carries no key,
//! no value and no serde message: they come from the request, and the 4xx text is fixed.
//!
//! **Limit 1 (found by test `w1_s_4` and, wider, by vera's W1S-V1 and `w1_s_9` to `w1_s_11`, 6 Nov): the
//! BUFFERING serde attributes.** serde reads the input into a buffer, and deserializes from the buffer with a
//! deserializer `serde_ignored` never wraps, for `#[serde(flatten)]`, for an `untagged` enum, for an internally
//! tagged one (`tag = ..`) and for an adjacently tagged one when the content comes BEFORE the tag (the order of
//! the request's keys decides). An unknown field inside such a payload is neither reported here nor, next to a
//! `flatten`, by `deny_unknown_fields` (serde does not support that pair). `deny_unknown_fields` on the INNER
//! struct of an enum does work through the buffer (`w1_s_13`) and shows as `Shape`. So a request or query type
//! is a plain struct or a default enum, and `xtask fitness` forbids the four attributes in product source
//! (`Rule::BufferedSerdeInProductSource`, cox W1-21). A piece that needs one asks cox: a reviewed delta with
//! `deny_unknown_fields` on every payload type, a negative cell in the piece, vera's pass. A hand-written
//! `Deserialize` or a `deserialize_with` function that buffers is outside the rule (stated).
//!
//! **Syntax first, then shape (wave1/strict2, cox W1-35 B; bob's and sophi's findings of 6 Nov).** 400 is for SYNTAX
//! only, 422 for shape (S3). serde_json reports some shape failures in its syntax category (an enum field given
//! `null`, a number, `true`, an array or an object is `ExpectedSomeValue`), so the category cannot decide. Rule 1:
//! the whole input must be well-formed JSON; if it is not, [`StrictError::Syntax`], whatever the decode said. Rule 2:
//! once it is, ANY failure to build the target is [`StrictError::Shape`] (an unknown field stays
//! [`StrictError::Unknown`]). The well-formedness probe (a `serde_json::Value` parse of the whole input, the chain's
//! own test of what is JSON; UTF-8 and number range and nesting depth included) runs only when the typed decode failed or
//! flagged an unknown field, so a good body is read once and a repeated field is still refused by the typed decode. A
//! UTF-8 check of the whole input comes first on every body (vera's STRICT2-V1).
//!
//! **A request body is an object (rule 3, [`strict_body_from_slice`]).** serde's derive also builds a struct from its
//! SEQUENCE form (`["Ops"]` for `{name}`), which S3 does not allow: every request body in the twin is a JSON object.
//! The route decode therefore refuses a non-empty body whose first non-space byte is not `{` as a shape error (422),
//! AFTER the syntax check (an unterminated `[` is still 400). An EMPTY body is unchanged: the route decodes it as
//! `null`. [`strict_from_slice`] itself stays free of this rule, because it cannot tell whether a target type is
//! meant to be an object (a `Vec<T>` is not).

use serde::de::DeserializeOwned;

/// Why a strict decode refused. A fixed meaning, never the key, the value or serde's message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StrictError {
    /// The input is not well-formed (not JSON, trailing characters, truncated): 400 for a body.
    #[error("malformed input")]
    Syntax,
    /// Well-formed but not this shape (a wrong type, a missing or repeated field): 422.
    #[error("input does not match the expected shape")]
    Shape,
    /// A field the target type does not read: 422.
    #[error("input names a field the operation does not take")]
    Unknown,
}

/// Whether `raw` is exactly one JSON value to the parser: UTF-8 everywhere, every number in range, within
/// serde_json's nesting limit (128), nothing after the value. It parses a `serde_json::Value`, which is the same test
/// the chain's idempotency body hash applies, so the two never disagree about what is JSON. (It is not
/// `serde::de::IgnoredAny`: that skips string contents without checking their UTF-8, skips numbers without
/// range-checking them and skips nesting without a depth limit, so a body that is not JSON passed it: vera's
/// STRICT2-V1.) Runs only when the typed decode failed or flagged an unknown field, never on a good body.
fn is_well_formed(raw: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(raw).is_ok()
}

/// The refusal for a decode that failed: syntax if the input is not JSON at all (rule 1), else shape (rule 2).
fn refusal(raw: &[u8]) -> StrictError {
    if is_well_formed(raw) {
        StrictError::Shape
    } else {
        StrictError::Syntax
    }
}

/// Decode a JSON body strictly. An empty slice is `Syntax`; the route decode maps an empty body to `null`
/// before calling.
pub fn strict_from_slice<T: DeserializeOwned>(raw: &[u8]) -> Result<T, StrictError> {
    // JSON text is UTF-8 (RFC 8259). The typed decode converts only the strings it reads, so bytes in a string it
    // skips or ignores would otherwise pass: the whole input is checked first, which is cheap.
    if std::str::from_utf8(raw).is_err() {
        return Err(StrictError::Syntax);
    }
    let mut de = serde_json::Deserializer::from_slice(raw);
    let mut unknown = false;
    let value: T = match serde_ignored::deserialize(&mut de, |_path| unknown = true) {
        Ok(v) => v,
        Err(_) => return Err(refusal(raw)),
    };
    // Trailing characters after the value are a syntax error, as in `serde_json::from_slice`.
    de.end().map_err(|_| StrictError::Syntax)?;
    if unknown {
        // An unknown field's value was skipped without being read: it must still be JSON (rule 1).
        Err(if is_well_formed(raw) {
            StrictError::Unknown
        } else {
            StrictError::Syntax
        })
    } else {
        Ok(value)
    }
}

/// Decode a REQUEST BODY strictly: [`strict_from_slice`], and the body must be a JSON object (rule 3). A non-empty
/// body whose first non-space byte is not `{` is `Shape`, once it is known to be JSON at all (`Syntax` otherwise);
/// a body of white space only is `Syntax`. The caller maps an empty body to `null` first and does not come here.
pub(crate) fn strict_body_from_slice<T: DeserializeOwned>(raw: &[u8]) -> Result<T, StrictError> {
    match raw
        .iter()
        .find(|b| !matches!(**b, b' ' | b'\t' | b'\n' | b'\r'))
    {
        Some(b'{') => strict_from_slice(raw),
        Some(_) => Err(refusal(raw)),
        None => Err(StrictError::Syntax),
    }
}

/// Decode a query string strictly (percent-decoded as axum's `Query` does). A bad value or a missing
/// parameter is `Shape`, an unknown parameter is `Unknown`; a query has no syntax error of its own.
pub fn strict_from_query<T: DeserializeOwned>(query: &str) -> Result<T, StrictError> {
    let de = serde_urlencoded::Deserializer::new(form_urlencoded::parse(query.as_bytes()));
    let mut unknown = false;
    let value: T =
        serde_ignored::deserialize(de, |_path| unknown = true).map_err(|_| StrictError::Shape)?;
    if unknown {
        Err(StrictError::Unknown)
    } else {
        Ok(value)
    }
}
