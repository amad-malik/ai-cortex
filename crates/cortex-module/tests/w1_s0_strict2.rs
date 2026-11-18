//! Wave 1, strict2 (cox W1-35 B; sophi's row c6490329; bob's and sophi's findings of 6 Nov): two decode defects
//! in the write body, fixed once in `strict_from_slice` (`strict.rs`) and in `Routes::write`'s decode.
//!
//! Both are bugs against S3 (every one of the 15 request bodies in the r2.6 twin is a JSON OBJECT; 400 is for
//! SYNTAX only, 422 for shape):
//!
//! 1. A field whose type is an ENUM and whose value is not a string (`{"kind":null}`, `5`, `true`, `[]`, `{}`)
//!    was answered 400 "not valid JSON", because serde_json reports that failure in its syntax category. Now:
//!    the whole body must FIRST be syntactically valid JSON (400); after that ANY error from deserializing the
//!    body type is 422 (rules 1 and 2).
//! 2. A body that is a JSON ARRAY was accepted by every plain struct (serde's derive reads the sequence form:
//!    `["Ops"]` created a team). Now: a non-empty body whose first non-space byte is not `{` is 422, AFTER the
//!    syntax check (rule 3). An EMPTY body stays as today (it decodes as `null`).
//!
//! `strict_from_query` is unchanged. The refusal never carries the key or the value (the 4xx text is fixed).
//!
//! RED: `strict_from_slice` classifies serde_json's error category, and the route decode has no object rule.
use axum::http::Method;
use cortex_module::route::{DecodeFn, Handler};
use cortex_module::{Reply, Routes, StrictError, strict_from_slice};
use serde::Deserialize;
use serde::de::DeserializeOwned;

/// A plain enum, serde's default (externally tagged): the type the defect is about.
#[derive(Debug, Deserialize, PartialEq)]
enum Kind {
    Ops,
    Dev,
}

#[derive(Debug, Deserialize, PartialEq)]
struct Pick {
    kind: Kind,
}

#[derive(Debug, Deserialize, PartialEq)]
struct Outer {
    inner: Pick,
    list: Vec<Pick>,
}

#[derive(Debug, Deserialize, PartialEq)]
struct Plain {
    name: String,
    n: Option<u32>,
}

const TOKENISH: &str =
    "ctx2_0123456789abcdef0123456789abcdef.AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

fn decode_for<B: DeserializeOwned + Send + Sync + 'static>() -> DecodeFn {
    Routes::new()
        .write_open(Method::POST, "/v2/x", "x:write", |_tx, _ctx, _b: B| {
            Box::pin(async move { Ok(Reply::ok(1_u8)) })
        })
        .into_entries()
        .into_iter()
        .find_map(|e| match e.handler {
            Handler::Write { decode, .. } => Some(decode),
            Handler::Read(_) => None,
        })
        .expect("a write route")
}

/// `None` when the decode accepts, else the refusal's status and code.
fn refusal(d: &DecodeFn, raw: &[u8]) -> Option<(u16, &'static str)> {
    d(raw).err().map(|p| (p.status, p.code))
}

const SYNTAX: Option<(u16, &str)> = Some((400, "invalid_request"));
const SHAPE: Option<(u16, &str)> = Some((422, "invalid_request"));

// ---- rules 1 and 2, in the helper --------------------------------------------------------------------

#[test]
fn w1_s2_1_an_enum_field_with_a_value_that_is_not_a_string_is_shape_not_syntax() {
    for (what, raw) in [
        ("null", &br#"{"kind":null}"#[..]),
        ("number", br#"{"kind":5}"#),
        ("true", br#"{"kind":true}"#),
        ("false", br#"{"kind":false}"#),
        ("empty array", br#"{"kind":[]}"#),
        ("array of the variant", br#"{"kind":["Ops"]}"#),
        ("empty object", br#"{"kind":{}}"#),
        (
            "object with two keys",
            br#"{"kind":{"Ops":null,"Dev":null}}"#,
        ),
        ("unknown variant", br#"{"kind":"Nope"}"#),
        (
            "variant with a payload it does not take",
            br#"{"kind":{"Ops":5}}"#,
        ),
    ] {
        assert_eq!(
            strict_from_slice::<Pick>(raw),
            Err(StrictError::Shape),
            "{what}"
        );
    }
    // The control: a real variant decodes.
    assert_eq!(
        strict_from_slice::<Pick>(br#"{"kind":"Ops"}"#),
        Ok(Pick { kind: Kind::Ops })
    );
}

#[test]
fn w1_s2_2_a_nested_enum_field_is_shape_too() {
    for (what, raw) in [
        ("in an object", &br#"{"inner":{"kind":null},"list":[]}"#[..]),
        (
            "in a list",
            br#"{"inner":{"kind":"Ops"},"list":[{"kind":"Dev"},{"kind":5}]}"#,
        ),
        (
            "in a list, true",
            br#"{"inner":{"kind":"Ops"},"list":[{"kind":true}]}"#,
        ),
    ] {
        assert_eq!(
            strict_from_slice::<Outer>(raw),
            Err(StrictError::Shape),
            "{what}"
        );
    }
    assert!(
        strict_from_slice::<Outer>(br#"{"inner":{"kind":"Ops"},"list":[{"kind":"Dev"}]}"#).is_ok()
    );
}

#[test]
fn w1_s2_3_the_whole_body_must_be_json_before_any_shape_is_asked_of_it() {
    // A data error comes first in the text, a syntax error after it: the body is not JSON, so 400 (rule 1).
    // (Before: the type error at `5` ended the decode and the tail was never read.)
    for (what, raw) in [
        ("wrong type, then junk", &br#"{"name":5, junk"#[..]),
        ("wrong type, then truncated", br#"{"name":5,"n":"#),
        ("enum null, then junk", br#"{"kind":null, junk"#),
        ("enum number, then trailing characters", br#"{"kind":5} x"#),
        ("wrong type, then trailing characters", br#"{"name":5} x"#),
        (
            "wrong type, then a second value",
            br#"{"name":5}{"name":6}"#,
        ),
    ] {
        let as_plain = strict_from_slice::<Plain>(raw);
        let as_pick = strict_from_slice::<Pick>(raw);
        assert_eq!(as_plain, Err(StrictError::Syntax), "Plain: {what}");
        assert_eq!(as_pick, Err(StrictError::Syntax), "Pick: {what}");
    }
}

#[test]
fn w1_s2_4_a_syntax_error_alone_is_still_syntax() {
    for (what, raw) in [
        ("truncated", &br#"{"name":"a""#[..]),
        ("not json", b"name=a"),
        ("trailing characters", br#"{"name":"a"} x"#),
        ("value missing", br#"{"kind":}"#),
        ("bare word", br#"{"kind":nul}"#),
        ("unquoted key", br#"{kind:"Ops"}"#),
        ("single quotes", br#"{'kind':'Ops'}"#),
        ("empty", b""),
        ("only space", b"   "),
        ("a byte order mark", b"\xEF\xBB\xBF{\"kind\":\"Ops\"}"),
    ] {
        assert_eq!(
            strict_from_slice::<Pick>(raw),
            Err(StrictError::Syntax),
            "Pick: {what}"
        );
    }
}

#[test]
fn w1_s2_5_after_the_syntax_check_every_other_failure_is_shape_or_unknown() {
    for (what, raw, want) in [
        ("missing field", &br#"{"n":1}"#[..], StrictError::Shape),
        (
            "repeated field",
            br#"{"name":"a","name":"b"}"#,
            StrictError::Shape,
        ),
        ("wrong type", br#"{"name":5}"#, StrictError::Shape),
        ("null for a struct", b"null", StrictError::Shape),
        ("array for a struct", b"[]", StrictError::Shape),
        (
            "unknown field",
            br#"{"name":"a","extra":1}"#,
            StrictError::Unknown,
        ),
    ] {
        assert_eq!(strict_from_slice::<Plain>(raw), Err(want), "{what}");
    }
    assert!(strict_from_slice::<Plain>(br#"{"name":"a"}"#).is_ok());
    assert!(strict_from_slice::<Plain>(b" \t\r\n{\"name\":\"a\"}\n").is_ok());
}

// ---- rules 1, 2 and 3 on a route ---------------------------------------------------------------------

#[test]
fn w1_s2_6_on_a_route_an_enum_field_with_a_value_that_is_not_a_string_is_422() {
    let d = decode_for::<Pick>();
    for raw in [
        &br#"{"kind":null}"#[..],
        br#"{"kind":5}"#,
        br#"{"kind":true}"#,
        br#"{"kind":[]}"#,
        br#"{"kind":{}}"#,
        br#"{"kind":"Nope"}"#,
    ] {
        assert_eq!(refusal(&d, raw), SHAPE, "{}", String::from_utf8_lossy(raw));
    }
    assert_eq!(refusal(&d, br#"{"kind":"Dev"}"#), None);
    let nested = decode_for::<Outer>();
    assert_eq!(
        refusal(&nested, br#"{"inner":{"kind":null},"list":[]}"#),
        SHAPE
    );
    assert_eq!(
        refusal(&nested, br#"{"inner":{"kind":"Ops"},"list":[{"kind":5}]}"#),
        SHAPE
    );
}

#[test]
fn w1_s2_7_on_a_route_a_body_that_is_not_an_object_is_422_and_never_builds_a_struct() {
    let pick = decode_for::<Pick>();
    let plain = decode_for::<Plain>();
    // The sequence form of a struct: serde's derive accepts it, S3 does not (the team that `["Ops"]` created).
    assert_eq!(refusal(&pick, br#"["Ops"]"#), SHAPE);
    assert_eq!(refusal(&plain, br#"["a",3]"#), SHAPE);
    assert_eq!(refusal(&plain, br#"["a"]"#), SHAPE);
    assert_eq!(refusal(&plain, b" \n\t\r [\"a\",3]"), SHAPE);
    for raw in [
        &b"[]"[..],
        b"null",
        b"5",
        b"true",
        br#""x""#,
        b"-1",
        b"1.5e3",
    ] {
        assert_eq!(
            refusal(&plain, raw),
            SHAPE,
            "{}",
            String::from_utf8_lossy(raw)
        );
        assert_eq!(
            refusal(&pick, raw),
            SHAPE,
            "{}",
            String::from_utf8_lossy(raw)
        );
    }
}

#[test]
fn w1_s2_8_on_a_route_the_syntax_check_comes_before_the_object_rule() {
    let plain = decode_for::<Plain>();
    // Not JSON at all: 400, even though the first byte is not `{`.
    for raw in [
        &b"[1,2"[..],
        b"[",
        b"]",
        b"nul",
        b"tru",
        b"'x'",
        b"name=a",
        b"[1,2] x",
        b"[1,2][3]",
        b"   ",
        b"\xEF\xBB\xBF{}",
        b"\xFF",
    ] {
        assert_eq!(
            refusal(&plain, raw),
            SYNTAX,
            "{}",
            String::from_utf8_lossy(raw)
        );
    }
    // The same text, well-formed: 422.
    assert_eq!(refusal(&plain, b"[1,2]"), SHAPE);
    let p = plain(b"[1,2").expect_err("not JSON");
    assert_eq!(&*p.detail, "The body is not valid JSON.");
}

#[test]
fn w1_s2_9_on_a_route_an_object_still_passes_and_the_old_refusals_hold() {
    let plain = decode_for::<Plain>();
    assert_eq!(refusal(&plain, br#"{"name":"a"}"#), None);
    assert_eq!(refusal(&plain, br#"{"name":"a","n":3}"#), None);
    // Every kind of JSON space before the brace.
    for lead in [" ", "\t", "\r", "\n", " \t\r\n ", "\n\n\n"] {
        let body = format!(r#"{lead}{{"name":"a"}}"#);
        assert_eq!(refusal(&plain, body.as_bytes()), None, "{lead:?}");
    }
    assert_eq!(refusal(&plain, br#"{"name":"a","extra":1}"#), SHAPE);
    assert_eq!(refusal(&plain, br#"{"name":5}"#), SHAPE);
    assert_eq!(refusal(&plain, br#"{"name":"a","name":"b"}"#), SHAPE);
    assert_eq!(refusal(&plain, br#"{"name":"a""#), SYNTAX);
    assert_eq!(refusal(&plain, br#"{"name":5, junk"#), SYNTAX);
    assert_eq!(refusal(&plain, b"{"), SYNTAX);
    assert_eq!(refusal(&plain, b"{}"), SHAPE);
}

#[test]
fn w1_s2_10_an_empty_body_stays_as_it_was_and_a_body_of_null_is_not_empty() {
    // Empty decodes as `null` (unchanged): a unit body accepts it, a struct refuses it as a shape error.
    let unit = decode_for::<()>();
    let plain = decode_for::<Plain>();
    assert_eq!(refusal(&unit, b""), None, "an empty body for a unit body");
    assert_eq!(refusal(&plain, b""), SHAPE, "an empty body for a struct");
    // A body that is the text `null` is NOT empty: it is not an object (rule 3).
    assert_eq!(refusal(&unit, b"null"), SHAPE);
    assert_eq!(refusal(&unit, b"  null"), SHAPE);
    assert_eq!(
        refusal(&unit, b"{}"),
        SHAPE,
        "an object is not a unit either"
    );
}

#[test]
fn w1_s2_11_a_refusal_never_carries_the_key_or_the_value() {
    let pick = decode_for::<Pick>();
    let plain = decode_for::<Plain>();
    let cases = [
        format!(r#"{{"kind":"{TOKENISH}"}}"#),
        format!(r#"{{"kind":["{TOKENISH}"]}}"#),
        format!(r#"["{TOKENISH}"]"#),
        format!(r#"{{"{TOKENISH}":"{TOKENISH}"}}"#),
        format!(r#"["{TOKENISH}""#),
    ];
    for body in &cases {
        for d in [&pick, &plain] {
            let p = d(body.as_bytes()).expect_err("refused");
            let shown = format!("{p:?} | {} | {}", p.title, p.detail);
            assert!(
                !shown.contains("ctx2_") && !shown.contains("AAAA"),
                "{shown}"
            );
        }
    }
}

// ---- vera's STRICT2-V1 (REVIEW.md, 6 Nov): "the whole input must be well-formed JSON" includes UTF-8 ----------

/// A target with an `IgnoredAny` field: a value serde_json skips without converting its strings to `str`.
#[derive(Debug, Deserialize)]
struct Ignoring {
    #[allow(dead_code)]
    name: String,
    #[allow(dead_code)]
    extra: serde::de::IgnoredAny,
}

/// Invalid UTF-8 inside a JSON string: a lone continuation byte, a truncated sequence, an overlong form, an encoded
/// surrogate and a byte that never occurs.
const BAD_STRINGS: [&[u8]; 5] = [b"\xff", b"\xc3", b"\xc0\x80", b"\xed\xa0\x80", b"a\x80b"];

fn with_bad(prefix: &str, bad: &[u8], suffix: &str) -> Vec<u8> {
    let mut v = prefix.as_bytes().to_vec();
    v.extend_from_slice(bad);
    v.extend_from_slice(suffix.as_bytes());
    v
}

#[test]
fn w1_s2_12_invalid_utf8_inside_a_string_is_syntax_whether_the_string_is_read_skipped_or_ignored() {
    for bad in BAD_STRINGS {
        // Read: a known `String` field (serde_json refuses the bytes itself).
        let known = with_bad(r#"{"name":""#, bad, r#""}"#);
        assert_eq!(
            strict_from_slice::<Plain>(&known),
            Err(StrictError::Syntax),
            "known {bad:?}"
        );
        // Skipped: an unknown field's string value (before: `Unknown`, 422, for a body that is not JSON).
        let unknown = with_bad(r#"{"name":"a","extra":""#, bad, r#""}"#);
        assert_eq!(
            strict_from_slice::<Plain>(&unknown),
            Err(StrictError::Syntax),
            "unknown {bad:?}"
        );
        // Skipped, nested, and as an object key.
        let nested = with_bad(r#"{"name":"a","extra":{"k":[""#, bad, r#""]}}"#);
        assert_eq!(
            strict_from_slice::<Plain>(&nested),
            Err(StrictError::Syntax),
            "nested {bad:?}"
        );
        let key = with_bad(r#"{"name":"a",""#, bad, r#"":1}"#);
        assert_eq!(
            strict_from_slice::<Plain>(&key),
            Err(StrictError::Syntax),
            "key {bad:?}"
        );
        // Ignored by the TARGET (no unknown field is flagged, the typed decode succeeds).
        let ignored = with_bad(r#"{"name":"a","extra":""#, bad, r#""}"#);
        assert!(
            matches!(
                strict_from_slice::<Ignoring>(&ignored),
                Err(StrictError::Syntax)
            ),
            "ignored {bad:?}"
        );
        // After a data error, too: the typed failure must not hide it (wrong type first).
        let after = with_bad(r#"{"name":5,"extra":""#, bad, r#""}"#);
        assert_eq!(
            strict_from_slice::<Plain>(&after),
            Err(StrictError::Syntax),
            "after {bad:?}"
        );
    }
    // The controls: valid multibyte text is JSON.
    assert!(strict_from_slice::<Plain>("{\"name\":\"caf\u{e9} \u{1F600}\"}".as_bytes()).is_ok());
    assert_eq!(
        strict_from_slice::<Plain>("{\"name\":\"a\",\"extra\":\"caf\u{e9}\"}".as_bytes()),
        Err(StrictError::Unknown)
    );
}

#[test]
fn w1_s2_13_on_a_route_invalid_utf8_inside_a_string_is_400() {
    let plain = decode_for::<Plain>();
    let pick = decode_for::<Pick>();
    for bad in BAD_STRINGS {
        for (what, body) in [
            ("known", with_bad(r#"{"name":""#, bad, r#""}"#)),
            (
                "unknown",
                with_bad(r#"{"name":"a","extra":""#, bad, r#""}"#),
            ),
            (
                "after a data error",
                with_bad(r#"{"name":5,"extra":""#, bad, r#""}"#),
            ),
        ] {
            assert_eq!(refusal(&plain, &body), SYNTAX, "{what} {bad:?}");
        }
        // An enum field's string with bad bytes: the variant name cannot match, but the body is not JSON.
        let kind = with_bad(r#"{"kind":""#, bad, r#""}"#);
        assert_eq!(refusal(&pick, &kind), SYNTAX, "enum {bad:?}");
    }
}

/// Depth and numbers: a body the parser cannot read is not JSON to it (400), wherever the value sits. Before: a
/// skipped unknown value was never depth-checked or number-checked, and the answer was 422 `Unknown`.
#[test]
fn w1_s2_14_depth_and_number_range_inside_a_skipped_value_are_syntax_like_everywhere_else() {
    let deep = |n: usize| format!("{}{}", "[".repeat(n), "]".repeat(n));
    // 100 levels are fine for the parser (limit 128): a valid body with an unknown field.
    let ok = format!(r#"{{"name":"a","extra":{}}}"#, deep(100));
    assert_eq!(
        strict_from_slice::<Plain>(ok.as_bytes()),
        Err(StrictError::Unknown)
    );
    // 200 levels are past the limit: not parseable, so Syntax, in an unknown field and in a known one.
    let too_deep = format!(r#"{{"name":"a","extra":{}}}"#, deep(200));
    assert_eq!(
        strict_from_slice::<Plain>(too_deep.as_bytes()),
        Err(StrictError::Syntax)
    );
    let known_deep = format!(r#"{{"name":{}}}"#, deep(200));
    assert_eq!(
        strict_from_slice::<Plain>(known_deep.as_bytes()),
        Err(StrictError::Syntax)
    );
    // A number the parser refuses (out of range), skipped or typed.
    for body in [
        r#"{"name":"a","extra":1e999}"#,
        r#"{"name":"a","n":1e999}"#,
        r#"{"name":"a","extra":-1e999}"#,
    ] {
        assert_eq!(
            strict_from_slice::<Plain>(body.as_bytes()),
            Err(StrictError::Syntax),
            "{body}"
        );
    }
    // The route decode says the same.
    let plain = decode_for::<Plain>();
    assert_eq!(refusal(&plain, too_deep.as_bytes()), SYNTAX);
    assert_eq!(refusal(&plain, ok.as_bytes()), SHAPE);
    assert_eq!(refusal(&plain, br#"{"name":"a","extra":1e999}"#), SYNTAX);
}

/// vera's R2 (ADDENDUM1-REVIEW): a target that takes its string as BYTES. serde_json hands the raw bytes to
/// `visit_bytes` with no UTF-8 conversion, so the typed decode SUCCEEDS on invalid UTF-8, nothing is flagged unknown,
/// and only the whole-input gate in `strict_from_slice` refuses the body. (Before the gate this decoded.)
#[derive(Debug)]
struct RawBytes(#[allow(dead_code)] Vec<u8>);

impl<'de> Deserialize<'de> for RawBytes {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl serde::de::Visitor<'_> for V {
            type Value = RawBytes;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("bytes")
            }
            fn visit_bytes<E>(self, v: &[u8]) -> Result<RawBytes, E> {
                Ok(RawBytes(v.to_vec()))
            }
        }
        d.deserialize_bytes(V)
    }
}

#[derive(Debug, Deserialize)]
struct WithBytes {
    #[allow(dead_code)]
    raw: RawBytes,
}

#[test]
fn w1_s2_15_a_byte_typed_field_cannot_carry_invalid_utf8_past_the_whole_input_gate() {
    for bad in BAD_STRINGS {
        let body = with_bad(r#"{"raw":""#, bad, r#""}"#);
        assert!(
            matches!(
                strict_from_slice::<WithBytes>(&body),
                Err(StrictError::Syntax)
            ),
            "bytes {bad:?}"
        );
    }
    // The control: valid text in a byte field decodes.
    assert!(strict_from_slice::<WithBytes>(br#"{"raw":"abc"}"#).is_ok());
    assert!(strict_from_slice::<WithBytes>("{\"raw\":\"caf\u{e9}\"}".as_bytes()).is_ok());
}
