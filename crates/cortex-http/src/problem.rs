//! problem+json: the one body every refusal uses (CONTRACTS r7 :420; signatures r2 §3), and the
//! mapping of database and idempotency errors to it. No refusal ever carries a request value, a
//! token, row data, a constraint name or a database message (CONTRACTS §3; H-18).
use std::borrow::Cow;

use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE, RETRY_AFTER, WWW_AUTHENTICATE};
use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use cortex_db::{DbError, IdemError};
use cortex_module::AuthError;
use cortex_module::route::Problem;
use cortex_types::{ProblemType, RequestId};
use serde::Serialize;

/// The four codes cortex-http mints for itself (signatures r2 §3; cox's Q3). They are minted at
/// startup through the one `ProblemCodeMinter` with every piece's codes, by S0's registry.
pub const INTERNAL_ERROR: &str = "internal_error";
pub const UNAVAILABLE: &str = "unavailable";
pub const BUSY: &str = "busy";
pub const CONFLICT: &str = "conflict";

/// The content type of every refusal.
pub const PROBLEM_JSON: &str = "application/problem+json";

/// The idempotency key's grammar, as every key refusal states it.
const KEY_FIX: &str = "Send an Idempotency-Key of 8 to 128 printable ASCII characters; the CLI sends a UUIDv7 per operation.";

/// A refusal, before it meets its request id. `detail` and `fix` are fixed texts chosen here or by
/// the piece; they never quote the request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub status: StatusCode,
    pub code: &'static str,
    pub title: &'static str,
    pub detail: Cow<'static, str>,
    pub fix: &'static str,
    /// `install` or `project`, only on an admission 503 (F1-24).
    pub scope: Option<&'static str>,
    /// A 5xx's step and internal cause, for the log line only. Never rendered into the body or a
    /// header (signatures r2 section 3); a 4xx has none. Boxed so a `Refusal` stays small enough to
    /// return by value (clippy `result_large_err`).
    pub internal: Option<Box<(&'static str, String)>>,
}

/// The wire body. Its field order is the contract (CONTRACTS r7 :420): a plain struct, never a
/// map, so equal refusals are byte-identical apart from the request id (F1-24).
#[derive(Serialize)]
struct Wire<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    title: &'a str,
    status: u16,
    detail: &'a str,
    fix: &'a str,
    request_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    scope: Option<&'a str>,
}

impl Refusal {
    fn new(
        status: StatusCode,
        code: &'static str,
        title: &'static str,
        detail: impl Into<Cow<'static, str>>,
        fix: &'static str,
    ) -> Refusal {
        Refusal {
            status,
            code,
            title,
            detail: detail.into(),
            fix,
            scope: None,
            internal: None,
        }
    }

    /// The body, fields in the contract's order: type, title, status, detail, fix, request_id,
    /// then scope only when set.
    pub fn body(&self, request_id: &RequestId) -> Vec<u8> {
        let id = request_id.to_string();
        let wire = Wire {
            kind: self.code,
            title: self.title,
            status: self.status.as_u16(),
            detail: &self.detail,
            fix: self.fix,
            request_id: &id,
            scope: self.scope,
        };
        serde_json::to_vec(&wire).expect("a struct of strings and an integer always serialises")
    }

    /// The response: the body, `Content-Type: application/problem+json`, `Cache-Control:
    /// no-store`, `WWW-Authenticate: Bearer` on a 401, and `Retry-After: 1` on a 429 (busy). The request-id layer adds
    /// `Cortex-Request-Id`.
    pub fn into_response_with(self, request_id: &RequestId) -> Response {
        let mut r = Response::new(axum::body::Body::from(self.body(request_id)));
        *r.status_mut() = self.status;
        let h = r.headers_mut();
        h.insert(CONTENT_TYPE, HeaderValue::from_static(PROBLEM_JSON));
        h.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
        if self.status == StatusCode::UNAUTHORIZED {
            h.insert(WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }
        // cox's Q3: busy (429) tells the client when to retry the same key.
        if self.status == StatusCode::TOO_MANY_REQUESTS {
            h.insert(RETRY_AFTER, HeaderValue::from_static("1"));
        }
        // For the log layer (delta 4): the code, and a 5xx's step and cause. Extensions are never
        // sent, so neither reaches the client.
        let facts = crate::log::facts_mut(&mut r);
        facts.code = Some(self.code);
        facts.internal = self.internal.map(|b| *b);
        r
    }
}

pub fn token_missing() -> Refusal {
    Refusal::new(
        StatusCode::UNAUTHORIZED,
        ProblemType::TOKEN_MISSING.code(),
        "Token missing",
        "This request carries no bearer token.",
        "Send Authorization: Bearer <token>; the CLI reads your token file.",
    )
}

/// One body for a bad shape, an unknown token and a wrong secret (S1 A2; C5).
pub fn token_invalid() -> Refusal {
    Refusal::new(
        StatusCode::UNAUTHORIZED,
        ProblemType::TOKEN_INVALID.code(),
        "Token invalid",
        "The bearer token is not valid.",
        "Ask your project lead to reissue your token.",
    )
}

/// A token whose secret matched but which cannot be used (S1 section 5.4). These three codes are
/// mod-tokens' (its `ProblemSpec`s); the chain renders them from `AuthError`.
pub fn token_expired() -> Refusal {
    Refusal::new(
        StatusCode::UNAUTHORIZED,
        "token_expired",
        "Token expired",
        "The bearer token has expired.",
        "Ask your project lead to reissue your token.",
    )
}

pub fn token_revoked() -> Refusal {
    Refusal::new(
        StatusCode::UNAUTHORIZED,
        "token_revoked",
        "Token revoked",
        "The bearer token was revoked.",
        "Ask your project lead to reissue your token.",
    )
}

pub fn worker_retired() -> Refusal {
    Refusal::new(
        StatusCode::UNAUTHORIZED,
        "worker_retired",
        "AI Worker retired",
        "The AI Worker this token belongs to is retired.",
        "Ask your project lead.",
    )
}

/// `TokenVerifier`'s refusal (C5): a bad shape and an unknown token (a wrong secret included) are
/// one `token_invalid`, so the answer never says which.
pub fn from_auth(e: &AuthError) -> Refusal {
    match e {
        AuthError::Missing => token_missing(),
        AuthError::Malformed | AuthError::Unknown => token_invalid(),
        AuthError::Expired => token_expired(),
        AuthError::Revoked => token_revoked(),
        AuthError::Retired => worker_retired(),
        AuthError::Unavailable => unavailable(),
    }
}

/// A piece's `Problem` (S0), rendered like every other refusal.
impl From<&Problem> for Refusal {
    fn from(p: &Problem) -> Refusal {
        Refusal {
            status: StatusCode::from_u16(p.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR),
            code: p.code,
            title: p.title,
            detail: p.detail.clone(),
            fix: p.fix,
            scope: p.scope,
            internal: None,
        }
    }
}

pub fn missing_scope(scope: &str) -> Refusal {
    Refusal::new(
        StatusCode::FORBIDDEN,
        ProblemType::MISSING_SCOPE.code(),
        "Missing scope",
        format!("This operation needs the scope {scope}."),
        "Ask your project lead for a role that grants this scope.",
    )
}

pub fn body_too_large(max: u64) -> Refusal {
    Refusal::new(
        StatusCode::PAYLOAD_TOO_LARGE,
        ProblemType::BODY_TOO_LARGE.code(),
        "Body too large",
        format!("The body is over this operation's limit of {max} bytes."),
        "Send a smaller body.",
    )
}

pub fn not_found() -> Refusal {
    Refusal::new(
        StatusCode::NOT_FOUND,
        ProblemType::NOT_FOUND.code(),
        "Not found",
        "Nothing was found at this address in your project.",
        "Check the address and the ID.",
    )
}

/// 400, 405, 415 or 422 `invalid_request`, with a fixed detail.
pub fn invalid_request(status: StatusCode, detail: &'static str) -> Refusal {
    Refusal::new(
        status,
        ProblemType::INVALID_REQUEST.code(),
        "Invalid request",
        detail,
        "Correct the request; the detail says what is wrong.",
    )
}

/// 500 `internal_error`: S0's text for an internal database error (one home: finn's 16d227f).
pub fn internal_error() -> Refusal {
    from_db(DbError::Other(String::new()))
}

/// 503 `unavailable`: no `scope`, and not a code the client retries on its own (signatures r2 §3).
pub fn unavailable() -> Refusal {
    from_db(DbError::Unavailable)
}

/// A database error, by kind only (signatures r2 §3): no message, no constraint name. The mapping
/// and its texts live in S0 (`impl From<DbError> for Problem`, finn's 16d227f), so a read handler
/// that opens its own transaction refuses exactly as the chain does.
pub fn from_db(e: DbError) -> Refusal {
    Refusal::from(&Problem::from(e))
}

/// An idempotency error (S2 §5; signatures r2 §4).
pub fn from_idem(e: IdemError) -> Refusal {
    match e {
        IdemError::KeyRequired => Refusal::new(
            StatusCode::PRECONDITION_REQUIRED,
            ProblemType::IDEMPOTENCY_KEY_REQUIRED.code(),
            "Idempotency key required",
            "This operation changes data and needs an Idempotency-Key header.",
            KEY_FIX,
        ),
        IdemError::KeyReused => Refusal::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            ProblemType::IDEMPOTENCY_KEY_REUSED.code(),
            "Idempotency key reused",
            "This key was already used for a different request.",
            "Use a new key for a new operation; reuse a key only to retry the same request.",
        ),
        IdemError::AlreadyIssued { token_id } => Refusal::new(
            StatusCode::CONFLICT,
            ProblemType::ALREADY_ISSUED.code(),
            "Already issued",
            format!("Token {token_id} was issued by the first request; its secret is not stored."),
            "Revoke that token and issue a new one.",
        ),
        IdemError::BadKey => invalid_key(),
        IdemError::Db(d) => from_db(d),
    }
}

/// 400: a key outside the grammar, two keys, or a key that is not visible ASCII.
pub(crate) fn invalid_key() -> Refusal {
    Refusal::new(
        StatusCode::BAD_REQUEST,
        ProblemType::INVALID_REQUEST.code(),
        "Invalid request",
        "The Idempotency-Key is not one value of 8 to 128 printable ASCII characters.",
        KEY_FIX,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::header;
    use cortex_types::{SystemClock, TokenId};

    fn rid() -> RequestId {
        RequestId::new(&SystemClock)
    }

    fn text(r: &Refusal, id: &RequestId) -> String {
        String::from_utf8(r.body(id)).expect("UTF-8")
    }

    #[test]
    fn h15_fields_are_in_the_contract_order_and_scope_only_when_set() {
        let id = RequestId::parse("0193e6a0-0000-7000-8000-000000000001").expect("a v7 id");
        let r = missing_scope("handoffs:claim");
        let t = text(&r, &id);
        let order = [
            "\"type\"",
            "\"title\"",
            "\"status\"",
            "\"detail\"",
            "\"fix\"",
            "\"request_id\"",
        ];
        let mut at = 0;
        for f in order {
            let i = t[at..]
                .find(f)
                .unwrap_or_else(|| panic!("{f} missing or out of order in {t}"));
            at += i + f.len();
        }
        assert!(!t.contains("\"scope\""), "no scope on a 403: {t}");
        assert!(t.starts_with("{\"type\":\"missing_scope\","), "{t}");
        assert!(
            t.contains("\"request_id\":\"0193e6a0-0000-7000-8000-000000000001\""),
            "{t}"
        );
        let mut s = unavailable();
        s.scope = Some("project");
        assert!(text(&s, &id).ends_with(",\"scope\":\"project\"}"));
        let v: serde_json::Value = serde_json::from_slice(&r.body(&id)).expect("JSON");
        assert_eq!(
            v.as_object().expect("an object").len(),
            6,
            "no other field: {t}"
        );
    }

    #[test]
    fn h15_every_constructor_has_a_fix_and_a_bare_code() {
        let all = [
            token_missing(),
            token_invalid(),
            missing_scope("x:y"),
            body_too_large(1024),
            not_found(),
            invalid_request(
                StatusCode::UNPROCESSABLE_ENTITY,
                "the body does not match the operation",
            ),
            internal_error(),
            unavailable(),
        ];
        for r in all {
            assert!(!r.fix.is_empty(), "{} has no fix", r.code);
            assert!(
                r.code.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'),
                "{}",
                r.code
            );
        }
    }

    #[test]
    fn h16_equal_refusals_are_byte_identical_apart_from_the_request_id() {
        let (a, b) = (rid(), rid());
        let ta = text(&token_invalid(), &a).replace(&a.to_string(), "ID");
        let tb = text(&token_invalid(), &b).replace(&b.to_string(), "ID");
        assert_eq!(ta, tb);
        let s1 = text(&missing_scope("a:read"), &a);
        let s2 = text(&missing_scope("b:read"), &a);
        assert_ne!(s1, s2, "the context differs, so the detail differs");
    }

    #[tokio::test]
    async fn h06_refusal_headers() {
        let r = token_missing().into_response_with(&rid());
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(r.headers()[header::CONTENT_TYPE], PROBLEM_JSON);
        assert_eq!(r.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(r.headers()[header::WWW_AUTHENTICATE], "Bearer");
        let f = missing_scope("a:b").into_response_with(&rid());
        assert_eq!(f.status(), StatusCode::FORBIDDEN);
        assert!(f.headers().get(header::WWW_AUTHENTICATE).is_none());
        assert_eq!(f.headers()[header::CACHE_CONTROL], "no-store");
    }

    #[test]
    fn h17_database_errors_by_kind() {
        use cortex_db::LockStep;
        let cases: Vec<(DbError, u16, &str)> = vec![
            (DbError::PermissionDenied, 500, INTERNAL_ERROR),
            (DbError::Busy, 429, BUSY),
            (DbError::StatementTimeout, 503, UNAVAILABLE),
            (DbError::SessionEnded, 503, UNAVAILABLE),
            (DbError::Unavailable, 503, UNAVAILABLE),
            (DbError::Constraint { name: "c".into() }, 409, CONFLICT),
            (DbError::DelegationFailed, 500, INTERNAL_ERROR),
            (
                DbError::LockOrder {
                    held: LockStep::OwnerRows,
                    wanted: LockStep::Idempotency,
                },
                500,
                INTERNAL_ERROR,
            ),
            (DbError::UnknownModule, 500, INTERNAL_ERROR),
            (DbError::Isolation, 500, INTERNAL_ERROR),
            (DbError::Other("x".into()), 500, INTERNAL_ERROR),
        ];
        for (e, status, code) in cases {
            let label = format!("{e:?}");
            let r = from_db(e);
            assert_eq!((r.status.as_u16(), r.code), (status, code), "{label}");
            assert!(r.scope.is_none(), "{label}: unavailable carries no scope");
        }
    }

    #[test]
    fn h17_idempotency_errors() {
        let t = TokenId::new(&SystemClock);
        let cases: Vec<(IdemError, u16, &str)> = vec![
            (IdemError::KeyRequired, 428, "idempotency_key_required"),
            (IdemError::KeyReused, 422, "idempotency_key_reused"),
            (
                IdemError::AlreadyIssued { token_id: t },
                409,
                "already_issued",
            ),
            (IdemError::BadKey, 400, "invalid_request"),
            (IdemError::Db(DbError::Busy), 429, BUSY),
        ];
        for (e, status, code) in cases {
            let label = format!("{e:?}");
            let r = from_idem(e);
            assert_eq!((r.status.as_u16(), r.code), (status, code), "{label}");
        }
        let issued = from_idem(IdemError::AlreadyIssued { token_id: t });
        assert!(
            issued.detail.contains(&t.to_string()),
            "409 names the token id"
        );
        assert!(from_idem(IdemError::BadKey).fix.contains("8 to 128"));
    }

    /// cox's Q3 (45e33f74): busy is 429 with Retry-After, so a client retries the same key after
    /// a second. Every 429 refusal carries it (the header follows the status); busy is the only
    /// 429 the chain makes today, and no other status carries Retry-After.
    #[tokio::test]
    async fn h17_busy_is_429_with_retry_after() {
        let r = from_db(DbError::Busy).into_response_with(&rid());
        assert_eq!(r.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            r.headers().get(header::RETRY_AFTER).map(|v| v.as_bytes()),
            Some(&b"1"[..])
        );
        for other in [missing_scope("a:b"), unavailable(), internal_error()] {
            let r = other.into_response_with(&rid());
            assert!(
                r.headers().get(header::RETRY_AFTER).is_none(),
                "{}",
                r.status()
            );
        }
    }

    #[test]
    fn h18_no_database_text_reaches_the_body() {
        let id = rid();
        for e in [
            DbError::Other("canary_7f3e".into()),
            DbError::Constraint {
                name: "canary_7f3e_key".into(),
            },
        ] {
            let t = text(&from_db(e), &id);
            assert!(!t.contains("canary_7f3e"), "{t}");
        }
    }
}
