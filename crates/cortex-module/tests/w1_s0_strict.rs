//! Wave 1, step 1 (cox f28f6ff8; kai d2a88cf7; cox abde4db0): the strict decoders, `strict_from_slice` for a
//! JSON body and `strict_from_query` for a query string, ONE shared helper over `serde_ignored`.
//!
//! The point: an unknown field is refused for every plain struct. `#[serde(deny_unknown_fields)]` is
//! optional in a piece's body struct (a piece can forget it). The helper makes the refusal independent of the
//! struct's attributes. STATED LIMIT (w1_s_4, and vera's W1S-V1 in w1_s_9 to w1_s_11): serde BUFFERS the input of
//! `flatten`, of an `untagged` enum, of an internally tagged one and of an adjacently tagged one whose content
//! comes before its tag, and an unknown field inside such a payload is not seen. A request type must not use
//! them (xtask forbids them in product source, cox W1-21); the externally tagged default IS checked (w1_s_12);
//! `deny_unknown_fields` on the inner struct works through the buffer (w1_s_13).
//!
//! - Syntax (not JSON, truncated, trailing characters) is `Syntax` (400 for a body); a wrong type, a
//!   missing or repeated field is `Shape` and an unknown field is `Unknown` (both 422).
//! - The refusal NEVER carries the key or the value: they come from the request.
//! - `Routes::write`'s decode and `StrictQuery` use the helper, so the same holds on a route.
//!
//! RED: the helper is `todo!()`, and `Routes::write` and `StrictQuery` still use serde's plain decode.
use axum::extract::FromRequestParts;
use axum::http::{Method, Request};
use cortex_module::route::Handler;
use cortex_module::{
    Reply, Routes, StrictError, StrictQuery, strict_from_query, strict_from_slice,
};
use serde::Deserialize;

/// NO `deny_unknown_fields`: the case the helper exists for.
#[derive(Debug, Deserialize, PartialEq)]
struct Plain {
    name: String,
    n: Option<u32>,
}

#[derive(Debug, Deserialize, PartialEq)]
struct Inner {
    v: u8,
}

#[derive(Debug, Deserialize, PartialEq)]
struct Nested {
    inner: Inner,
    list: Vec<Inner>,
}

#[derive(Debug, Deserialize, PartialEq)]
struct Base {
    id: u8,
}

/// `flatten`: serde's `deny_unknown_fields` cannot be used here.
#[derive(Debug, Deserialize, PartialEq)]
struct Flat {
    name: String,
    #[serde(flatten)]
    base: Base,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Denying {
    name: String,
}

const TOKENISH: &str =
    "ctx2_0123456789abcdef0123456789abcdef.AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

#[test]
fn w1_s_1_a_body_that_names_only_known_fields_decodes() {
    assert_eq!(
        strict_from_slice::<Plain>(br#"{"name":"a","n":3}"#),
        Ok(Plain {
            name: "a".into(),
            n: Some(3)
        })
    );
    assert_eq!(
        strict_from_slice::<Plain>(br#"{"name":"a"}"#),
        Ok(Plain {
            name: "a".into(),
            n: None
        })
    );
    // Whitespace around the value is fine.
    assert!(strict_from_slice::<Plain>(b" \n{\"name\":\"a\"}\n ").is_ok());
}

#[test]
fn w1_s_2_an_unknown_field_is_refused_even_without_deny_unknown_fields() {
    assert_eq!(
        strict_from_slice::<Plain>(br#"{"name":"a","extra":1}"#),
        Err(StrictError::Unknown)
    );
    // The control: serde's own decode ACCEPTS it, which is why the helper exists.
    assert!(serde_json::from_slice::<Plain>(br#"{"name":"a","extra":1}"#).is_ok());
}

#[test]
fn w1_s_3_an_unknown_field_is_found_at_any_depth() {
    assert_eq!(
        strict_from_slice::<Nested>(br#"{"inner":{"v":1,"x":2},"list":[]}"#),
        Err(StrictError::Unknown),
        "inside an object"
    );
    assert_eq!(
        strict_from_slice::<Nested>(br#"{"inner":{"v":1},"list":[{"v":1},{"v":2,"x":3}]}"#),
        Err(StrictError::Unknown),
        "inside an array element"
    );
    assert!(strict_from_slice::<Nested>(br#"{"inner":{"v":1},"list":[{"v":1},{"v":2}]}"#).is_ok());
}

/// A STATED LIMIT, found by this test when it was written the other way round: `serde_ignored` does not see
/// an unknown field next to `#[serde(flatten)]` (serde buffers the flattened input and ignores the rest with
/// its own `IgnoredAny`). Neither does serde's own decode, and `deny_unknown_fields` does not work with
/// `flatten`. So a request type must not use `flatten`. If a later `serde_ignored` closes this, this test
/// fails on purpose and the limit comes out of `strict.rs`'s docs.
#[test]
fn w1_s_4_flatten_is_a_stated_limit_an_unknown_field_next_to_it_is_not_seen() {
    assert_eq!(
        strict_from_slice::<Flat>(br#"{"name":"a","id":1,"extra":true}"#),
        Ok(Flat {
            name: "a".into(),
            base: Base { id: 1 }
        }),
        "the limit: the extra field passes next to flatten"
    );
    assert!(strict_from_slice::<Flat>(br#"{"name":"a","id":1}"#).is_ok());
    // The control: serde's plain decode accepts it too, so the helper is no worse.
    assert!(serde_json::from_slice::<Flat>(br#"{"name":"a","id":1,"extra":true}"#).is_ok());
    // A MISSING flattened field is still caught.
    assert_eq!(
        strict_from_slice::<Flat>(br#"{"name":"a"}"#),
        Err(StrictError::Shape)
    );
}

// ---- W1S-V1 (vera's REWORK, cox W1-21): the hole is wider than `flatten` -------------------------------------
//
// serde BUFFERS the input of an UNTAGGED enum, of an INTERNALLY tagged one and of an ADJACENTLY tagged one when
// the content comes before the tag (it reads the map into `Content`, then deserializes the chosen variant from
// that buffer with `ContentRefDeserializer`, which `serde_ignored` never wraps). An unknown field inside such a
// payload is therefore accepted by `strict_from_slice` unless the INNER struct says `deny_unknown_fields`. An
// EXTERNALLY tagged enum (the default) is read straight from the input and IS checked. These cells witness
// both sides, with the same purpose as `w1_s_4`: if a later `serde_ignored` or serde closes a limit, the cell
// fails on purpose and the limit comes out of the docs. The xtask rule `BufferedSerdeInProductSource` (cox
// W1-21) forbids the four attributes (`flatten`, `untagged`, `tag =`, `content =`) in product source, so a
// request or query type is externally tagged (or a plain struct) and the helper's refusal holds for it.

#[derive(Debug, Deserialize, PartialEq)]
#[serde(untagged)]
enum Untagged {
    Named { name: String },
    Numbered { id: u8 },
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Internal {
    Create { name: String },
    Drop { id: u8 },
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "t", content = "c")]
enum Adjacent {
    Create { name: String },
    Drop { id: u8 },
}

/// The default: externally tagged.
#[derive(Debug, Deserialize, PartialEq)]
enum External {
    Create { name: String },
    Drop { id: u8 },
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct DenyNamed {
    name: String,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(untagged)]
enum UntaggedDenying {
    Named(DenyNamed),
}

#[test]
fn w1_s_9_an_untagged_enum_hides_an_unknown_field_a_stated_limit() {
    assert_eq!(
        strict_from_slice::<Untagged>(br#"{"name":"a","extra":true}"#),
        Ok(Untagged::Named { name: "a".into() }),
        "the limit: the extra field passes inside an untagged payload"
    );
    assert!(strict_from_slice::<Untagged>(br#"{"id":3}"#).is_ok());
    // The control: serde's plain decode accepts it too, so the helper is no worse.
    assert!(serde_json::from_slice::<Untagged>(br#"{"name":"a","extra":true}"#).is_ok());
    // A body no variant matches is still a shape error.
    assert_eq!(
        strict_from_slice::<Untagged>(br#"{"other":1}"#),
        Err(StrictError::Shape)
    );
}

#[test]
fn w1_s_10_an_internally_tagged_enum_hides_an_unknown_field_a_stated_limit() {
    assert_eq!(
        strict_from_slice::<Internal>(br#"{"kind":"create","name":"a","extra":true}"#),
        Ok(Internal::Create { name: "a".into() }),
        "the limit: the extra field passes inside an internally tagged payload"
    );
    assert_eq!(
        strict_from_slice::<Internal>(br#"{"kind":"drop","id":7,"extra":1}"#),
        Ok(Internal::Drop { id: 7 })
    );
    assert!(
        serde_json::from_slice::<Internal>(br#"{"kind":"create","name":"a","extra":true}"#).is_ok()
    );
}

#[test]
fn w1_s_11_an_adjacently_tagged_enum_hides_an_unknown_field_when_the_content_comes_before_the_tag()
{
    // Content before tag: serde buffers the content, so the helper does not see `extra`.
    assert_eq!(
        strict_from_slice::<Adjacent>(br#"{"c":{"name":"a","extra":true},"t":"Create"}"#),
        Ok(Adjacent::Create { name: "a".into() }),
        "the limit: content before tag is buffered"
    );
    // Tag before content: serde reads the content straight from the input, so the helper IS told.
    assert_eq!(
        strict_from_slice::<Adjacent>(br#"{"t":"Create","c":{"name":"a","extra":true}}"#),
        Err(StrictError::Unknown),
        "tag before content is not buffered; the order of keys in the request decides"
    );
    assert_eq!(
        strict_from_slice::<Adjacent>(br#"{"t":"Drop","c":{"id":2}}"#),
        Ok(Adjacent::Drop { id: 2 })
    );
}

#[test]
fn w1_s_12_the_positive_control_an_externally_tagged_enum_is_checked() {
    assert_eq!(
        strict_from_slice::<External>(br#"{"Create":{"name":"a"}}"#),
        Ok(External::Create { name: "a".into() })
    );
    assert_eq!(
        strict_from_slice::<External>(br#"{"Create":{"name":"a","extra":true}}"#),
        Err(StrictError::Unknown),
        "the default enum representation: an unknown field inside the variant is refused"
    );
    assert_eq!(
        strict_from_slice::<External>(br#"{"Drop":{"id":1,"extra":[1,2]}}"#),
        Err(StrictError::Unknown)
    );
    // A unit-like payload and a wrong variant name are shape errors, not unknown fields.
    assert_eq!(
        strict_from_slice::<External>(br#"{"Nope":{"id":1}}"#),
        Err(StrictError::Shape)
    );
}

#[test]
fn w1_s_13_deny_unknown_fields_on_the_inner_struct_works_through_a_buffered_enum() {
    // The reviewed-delta route of W1-21: a payload type that denies unknown fields refuses inside an untagged
    // enum, because the inner struct's own visitor sees the buffered map. It surfaces as a shape error (no
    // variant matched), not as `Unknown`.
    assert_eq!(
        strict_from_slice::<UntaggedDenying>(br#"{"name":"a"}"#),
        Ok(UntaggedDenying::Named(DenyNamed { name: "a".into() }))
    );
    assert_eq!(
        strict_from_slice::<UntaggedDenying>(br#"{"name":"a","extra":true}"#),
        Err(StrictError::Shape)
    );
}

// ---- W1S-V1b (vera): serde accepts the other two list delimiters, so the buffering attributes can be spelled
// `#[serde { untagged }]` and `#[serde [tag = ".."]]`. These compiled types prove the spellings are REAL (the
// derive takes them and buffers exactly as the parenthesised form does), which is why the xtask rule reads all
// three delimiter pairs.

#[derive(Debug, Deserialize, PartialEq)]
#[serde { untagged }]
enum UntaggedBrace {
    Named { name: String },
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde [tag = "kind", rename_all = "snake_case"]]
enum InternalBracket {
    Create { name: String },
}

#[test]
fn w1_s_14_the_brace_and_bracket_spellings_buffer_like_the_parenthesised_one() {
    assert_eq!(
        strict_from_slice::<UntaggedBrace>(br#"{"name":"a","extra":true}"#),
        Ok(UntaggedBrace::Named { name: "a".into() }),
        "`#[serde {{ untagged }}]` hides an unknown field"
    );
    assert_eq!(
        strict_from_slice::<InternalBracket>(br#"{"kind":"create","name":"a","extra":true}"#),
        Ok(InternalBracket::Create { name: "a".into() }),
        "`#[serde [tag = ..]]` hides an unknown field"
    );
    // The control: the same input in the default representation IS refused (w1_s_12).
    assert_eq!(
        strict_from_slice::<External>(br#"{"Create":{"name":"a","extra":true}}"#),
        Err(StrictError::Unknown)
    );
}

#[test]
fn w1_s_5_malformed_input_is_syntax() {
    for (what, raw) in [
        ("truncated", &br#"{"name":"a""#[..]),
        ("not json", b"name=a"),
        ("trailing characters", br#"{"name":"a"} x"#),
        ("two values", br#"{"name":"a"}{"name":"b"}"#),
        ("empty", b""),
    ] {
        assert_eq!(
            strict_from_slice::<Plain>(raw),
            Err(StrictError::Syntax),
            "{what}"
        );
    }
}

#[test]
fn w1_s_6_a_wrong_type_or_a_missing_or_repeated_field_is_shape() {
    for (what, raw) in [
        ("wrong type", &br#"{"name":5}"#[..]),
        ("missing field", br#"{"n":1}"#),
        ("repeated field", br#"{"name":"a","name":"b"}"#),
        ("null for a struct", b"null"),
        ("array for a struct", b"[]"),
    ] {
        assert_eq!(
            strict_from_slice::<Plain>(raw),
            Err(StrictError::Shape),
            "{what}"
        );
    }
}

#[test]
fn w1_s_7_a_struct_that_denies_unknown_fields_still_refuses() {
    assert!(strict_from_slice::<Denying>(br#"{"name":"a","extra":1}"#).is_err());
    assert!(strict_from_slice::<Denying>(br#"{"name":"a"}"#).is_ok());
}

#[test]
fn w1_s_8_a_refusal_never_carries_the_key_or_the_value() {
    let body = format!(r#"{{"name":"a","{TOKENISH}":"{TOKENISH}"}}"#);
    let err = strict_from_slice::<Plain>(body.as_bytes()).expect_err("an unknown field");
    let shown = format!("{err} | {err:?}");
    assert!(
        !shown.contains("ctx2_") && !shown.contains("AAAA"),
        "{shown}"
    );
    let wrong = format!(r#"{{"name":["{TOKENISH}"]}}"#);
    let err = strict_from_slice::<Plain>(wrong.as_bytes()).expect_err("a wrong type");
    let shown = format!("{err} | {err:?}");
    assert!(
        !shown.contains("ctx2_") && !shown.contains("AAAA"),
        "{shown}"
    );
}

#[test]
fn w1_q_1_a_query_with_known_parameters_decodes_and_percent_decodes() {
    assert_eq!(
        strict_from_query::<Plain>("name=a%20b&n=7"),
        Ok(Plain {
            name: "a b".into(),
            n: Some(7)
        })
    );
    assert_eq!(
        strict_from_query::<Plain>("name=a"),
        Ok(Plain {
            name: "a".into(),
            n: None
        })
    );
}

#[test]
fn w1_q_2_an_unknown_parameter_is_refused_even_without_deny_unknown_fields() {
    assert_eq!(
        strict_from_query::<Plain>("name=a&extra=1"),
        Err(StrictError::Unknown)
    );
    assert_eq!(
        strict_from_query::<Plain>("extra=1&name=a"),
        Err(StrictError::Unknown)
    );
    // The control: axum's own decoding accepts it.
    assert!(serde_urlencoded_accepts("name=a&extra=1"));
}

fn serde_urlencoded_accepts(q: &str) -> bool {
    axum::extract::Query::<Plain>::try_from_uri(&format!("/x?{q}").parse().expect("uri")).is_ok()
}

#[test]
fn w1_q_3_a_bad_value_or_a_missing_parameter_is_shape() {
    assert_eq!(
        strict_from_query::<Plain>("name=a&n=abc"),
        Err(StrictError::Shape)
    );
    assert_eq!(strict_from_query::<Plain>("n=1"), Err(StrictError::Shape));
    assert_eq!(strict_from_query::<Plain>(""), Err(StrictError::Shape));
}

#[test]
fn w1_q_4_a_query_refusal_never_carries_the_key_or_the_value() {
    let q = format!("name=a&{TOKENISH}={TOKENISH}");
    let err = strict_from_query::<Plain>(&q).expect_err("an unknown parameter");
    let shown = format!("{err} | {err:?}");
    assert!(
        !shown.contains("ctx2_") && !shown.contains("AAAA"),
        "{shown}"
    );
}

fn decode_of(r: Routes) -> cortex_module::route::DecodeFn {
    r.into_entries()
        .into_iter()
        .find_map(|e| match e.handler {
            Handler::Write { decode, .. } => Some(decode),
            Handler::Read(_) => None,
        })
        .expect("a write route")
}

#[test]
fn w1_r_1_a_write_routes_body_decode_refuses_an_unknown_field_without_deny_unknown_fields() {
    let d = decode_of(Routes::new().write_open(
        Method::POST,
        "/v2/x",
        "x:write",
        |_tx, _ctx, b: Plain| Box::pin(async move { Ok(Reply::ok(b.name)) }),
    ));
    assert!(d(br#"{"name":"a"}"#).is_ok());
    assert_eq!(
        d(br#"{"name":"a","extra":1}"#)
            .err()
            .map(|p| (p.status, p.code)),
        Some((422, "invalid_request")),
        "an unknown field is 422"
    );
    assert_eq!(
        d(br#"{"name":5}"#).err().map(|p| (p.status, p.code)),
        Some((422, "invalid_request")),
        "a wrong type is 422"
    );
    assert_eq!(
        d(b"{").err().map(|p| (p.status, p.code)),
        Some((400, "invalid_request")),
        "malformed JSON is 400"
    );
    assert_eq!(
        d(b"").err().map(|p| p.status),
        Some(422),
        "an empty body decodes as null, which is not a Plain"
    );
}

async fn query_status(uri: &str) -> Result<Plain, (u16, &'static str)> {
    let (mut parts, ()) = Request::builder()
        .uri(uri)
        .body(())
        .expect("request")
        .into_parts();
    StrictQuery::<Plain>::from_request_parts(&mut parts, &())
        .await
        .map(|q| q.0)
        .map_err(|p| (p.status, p.code))
}

#[tokio::test]
async fn w1_r_2_strict_query_refuses_an_unknown_parameter_without_deny_unknown_fields() {
    assert!(query_status("/x?name=a").await.is_ok());
    assert_eq!(
        query_status("/x?name=a&extra=1").await,
        Err((422, "invalid_request"))
    );
    assert_eq!(
        query_status("/x?name=a&n=abc").await,
        Err((422, "invalid_request"))
    );
}
