//! Step 9's inputs (S2 §5; signatures r2 §4): the key from `Idempotency-Key`, the route the key
//! is bound to (the method and the CONCRETE path, never the template), and the body's hash over
//! its RFC 8785 canonical form. The lock, the lookup and the stored reply are `cortex-db`'s.
use axum::http::{HeaderMap, Method, StatusCode};
use cortex_db::{Idem, IdemError, IdemKey, IdemToken, ModuleTx, StoredResponse, idempotency};
use cortex_module::Authorization;
use cortex_module::route::Ctx;
use cortex_types::{Sha256, canonical_json};
use sha2::Digest;

use crate::problem::{Refusal, from_idem, invalid_key, invalid_request};

/// The request header.
pub const HEADER: &str = "idempotency-key";

/// The key: missing gives 428 `idempotency_key_required`; two headers, a value that is not
/// visible ASCII, or a key outside 8 to 128 printable ASCII characters gives 400.
pub fn key(headers: &HeaderMap) -> Result<IdemKey, Refusal> {
    let mut all = headers.get_all(HEADER).iter();
    let Some(first) = all.next() else {
        return Err(from_idem(IdemError::KeyRequired));
    };
    if all.next().is_some() {
        return Err(invalid_key());
    }
    let text = first.to_str().map_err(|_| invalid_key())?;
    IdemKey::parse(text).map_err(|_| invalid_key())
}

/// The route a key is bound to: `<METHOD> <concrete path>`, so one key reused on two resources of
/// one template is refused, never answered with the other resource's reply (H-24).
pub fn route(method: &Method, path: &str) -> String {
    format!("{method} {path}")
}

/// The lookup (S3 R-19, delta 4): takes the Idempotency lock, then replays, refuses (422 reused,
/// 409 already_issued) or reserves. It is the ONLY place in this crate that calls
/// `idempotency::begin`, and it takes the erased `&Authorization`. An `Authorization` cannot be
/// made outside S0's `Ctx::authorize`, so no code path can return a stored reply to a caller whose
/// target check did not pass THIS time. A fitness test (`tests/fitness.rs`) and finn's xtask rule
/// keep every other caller out.
pub(crate) async fn lookup(
    tx: &mut ModuleTx,
    ctx: &Ctx,
    key: &IdemKey,
    route: &str,
    hash: Sha256,
    _proof: &Authorization,
) -> Result<Idem, IdemError> {
    idempotency::begin(tx, ctx.principal().worker, key, route, hash).await
}

/// Stores the reply with the effect, in the same transaction. The only place that calls
/// `idempotency::finish`; the reservation token only comes from `lookup`.
pub(crate) async fn finish(
    tx: &mut ModuleTx,
    token: IdemToken,
    status: u16,
    stored: StoredResponse,
) -> Result<(), IdemError> {
    idempotency::finish(tx, token, status, stored).await
}

/// The decoded body and its hash. An empty body is JSON `null`. Bad syntax is 400
/// `invalid_request`; a value RFC 8785 cannot represent is 422.
pub fn body_hash(body: &[u8]) -> Result<(serde_json::Value, Sha256), Refusal> {
    let value: serde_json::Value = if body.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(body)
            .map_err(|_| invalid_request(StatusCode::BAD_REQUEST, "The body is not valid JSON."))?
    };
    let canonical = canonical_json(&value).map_err(|_| {
        invalid_request(
            StatusCode::UNPROCESSABLE_ENTITY,
            "The body holds a value that has no canonical JSON form.",
        )
    })?;
    let digest: [u8; 32] = sha2::Sha256::digest(&canonical).into();
    Ok((value, Sha256(digest)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn h(values: &[&[u8]]) -> HeaderMap {
        let mut m = HeaderMap::new();
        for v in values {
            m.append(HEADER, HeaderValue::from_bytes(v).expect("a header value"));
        }
        m
    }

    fn status(r: Result<IdemKey, Refusal>) -> u16 {
        match r {
            Ok(_) => 200,
            Err(e) => e.status.as_u16(),
        }
    }

    #[test]
    fn h20_a_missing_key_is_428() {
        let e = key(&HeaderMap::new()).expect_err("no key");
        assert_eq!(
            (e.status.as_u16(), e.code),
            (428, "idempotency_key_required")
        );
    }

    #[test]
    fn h21_the_key_grammar_is_8_to_128_printable_ascii() {
        let k = |n: usize| "k".repeat(n);
        assert_eq!(status(key(&h(&[k(7).as_bytes()]))), 400);
        assert_eq!(status(key(&h(&[k(8).as_bytes()]))), 200);
        assert_eq!(status(key(&h(&[k(128).as_bytes()]))), 200);
        assert_eq!(status(key(&h(&[k(129).as_bytes()]))), 400);
        assert_eq!(
            status(key(&h(&[b"has a space in it"]))),
            200,
            "0x20 is printable"
        );
        assert_eq!(status(key(&h(&[b"tab\there12"]))), 400, "a tab is not");
        assert_eq!(status(key(&h(&[b"caf\xc3\xa9-key-1"]))), 400, "non-ASCII");
        assert_eq!(
            status(key(&h(&[b"0193e6a0-key-a", b"0193e6a0-key-b"]))),
            400,
            "two keys"
        );
        let e = key(&h(&[k(7).as_bytes()])).expect_err("too short");
        assert_eq!(e.code, "invalid_request");
        assert!(e.fix.contains("8 to 128"), "the fix states the grammar");
    }

    #[test]
    fn h24_the_route_is_the_method_and_the_concrete_path() {
        let a = route(
            &Method::POST,
            "/v2/probe/0193e6a0-0000-7000-8000-00000000000a/touch",
        );
        let b = route(
            &Method::POST,
            "/v2/probe/0193e6a0-0000-7000-8000-00000000000b/touch",
        );
        assert_ne!(a, b);
        assert_eq!(
            a,
            "POST /v2/probe/0193e6a0-0000-7000-8000-00000000000a/touch"
        );
        assert_ne!(route(&Method::PUT, "/v2/x"), route(&Method::POST, "/v2/x"));
    }

    #[test]
    fn h23_the_hash_is_over_the_canonical_form() {
        let (_, a) = body_hash(br#"{"a":1,"b":[1,2]}"#).expect("JSON");
        let (_, b) = body_hash(b" { \"b\" : [1,2] ,\n \"a\" : 1 } ").expect("JSON");
        let (_, c) = body_hash(br#"{"a":2,"b":[1,2]}"#).expect("JSON");
        assert_eq!(a, b, "the same value re-serialised has one hash");
        assert_ne!(a, c, "a different value has another");
        let (v, n) = body_hash(b"").expect("empty is null");
        assert_eq!(v, serde_json::Value::Null);
        assert_eq!(n, body_hash(b"null").expect("null").1);
        let e = body_hash(b"{\"a\":").expect_err("bad syntax");
        assert_eq!((e.status.as_u16(), e.code), (400, "invalid_request"));
    }
}
