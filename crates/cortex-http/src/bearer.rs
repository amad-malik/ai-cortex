//! Step 3's input: the bearer value from `Authorization`, parsed as strictly as v1's
//! `service_auth_http.py` (`_BEARER.fullmatch`, case-insensitive scheme). The token's own shape
//! (`ctx2_…`) is the verifier's (mod-tokens) to judge, not this parser's.
use axum::http::HeaderMap;
use axum::http::header::AUTHORIZATION;
use cortex_types::Secret;

/// Why there is no bearer value. Both refuse with 401; `Invalid` is the same body as an unknown
/// token or a wrong secret (C5, S1 A2), so the parser reveals nothing about the cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BearerError {
    /// No `Authorization` header: 401 `token_missing`.
    Missing,
    /// Anything else that is not exactly one `Bearer <token>`: 401 `token_invalid`.
    Invalid,
}

/// The bearer value as a `Secret`, at once, so it cannot be formatted into a log or an error.
pub fn bearer(headers: &HeaderMap) -> Result<Secret, BearerError> {
    let mut all = headers.get_all(AUTHORIZATION).iter();
    let Some(first) = all.next() else {
        return Err(BearerError::Missing);
    };
    if all.next().is_some() {
        return Err(BearerError::Invalid);
    }
    // Visible ASCII only (a tab passes here and is refused below).
    let value = first.to_str().map_err(|_| BearerError::Invalid)?;
    let (scheme, token) = value.split_once(' ').ok_or(BearerError::Invalid)?;
    if !scheme.eq_ignore_ascii_case("bearer")
        || token.is_empty()
        || token.bytes().any(|b| b.is_ascii_whitespace() || b == b',')
    {
        return Err(BearerError::Invalid);
    }
    Ok(Secret::new(token))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn with(values: &[&[u8]]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for v in values {
            h.append(
                AUTHORIZATION,
                HeaderValue::from_bytes(v).expect("a header value"),
            );
        }
        h
    }

    fn parsed(values: &[&[u8]]) -> Result<String, BearerError> {
        bearer(&with(values)).map(|s| s.expose().to_owned())
    }

    #[test]
    fn h07_no_header_is_missing() {
        assert_eq!(parsed(&[]), Err(BearerError::Missing));
    }

    #[test]
    fn h08_one_bearer_token_is_taken_and_the_scheme_is_case_insensitive() {
        assert_eq!(
            parsed(&[b"Bearer ctx2_abc.def"]),
            Ok("ctx2_abc.def".to_owned())
        );
        assert_eq!(
            parsed(&[b"bearer ctx2_abc.def"]),
            Ok("ctx2_abc.def".to_owned())
        );
        assert_eq!(parsed(&[b"BEARER x"]), Ok("x".to_owned()));
    }

    #[test]
    fn h08_every_other_shape_is_the_same_invalid() {
        let cases: &[&[&[u8]]] = &[
            &[b"Basic x"],
            &[b"Bearer"],
            &[b"Bearer "],
            &[b"Bearer  x"],
            &[b"Bearer a b"],
            &[b"Bearer a,b"],
            &[b"Bearer a\tb"],
            &[b"Bearerx"],
            &[b"x Bearer y"],
            &[b"Bearer \xc3\xa9t\xc3\xa9"],
            &[b"Bearer a", b"Bearer b"],
            &[b"Bearer a", b"Bearer a"],
        ];
        for c in cases {
            assert_eq!(parsed(c), Err(BearerError::Invalid), "case {c:?}");
        }
    }
}
