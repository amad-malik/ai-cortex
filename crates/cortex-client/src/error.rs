//! Typed errors: a decoded Problem, an answer that is not a Problem, local refusals and transport failures.
//! No variant carries token bytes; strings that come from the server are redacted against the token first.

use crate::display::{BUDGET_DETAIL, BUDGET_FIX, BUDGET_IDENT, BUDGET_TITLE, safe_text};
use crate::endpoint::EndpointError;
use crate::idempotency::KeyError;
use crate::token::TokenError;

/// The 20 platform and chain codes of the twin's `x-error-codes`, plus `Other` for every code the client does not know
/// (a code the client does not know is never an error of its own).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProblemKind {
    TokenMissing,
    TokenInvalid,
    TokenExpired,
    TokenRevoked,
    WorkerRetired,
    MissingScope,
    TargetNotAllowed,
    NotFound,
    RestoreInProgress,
    Maintenance,
    CapabilityUnavailable,
    IdempotencyKeyRequired,
    IdempotencyKeyReused,
    AlreadyIssued,
    BodyTooLarge,
    InvalidRequest,
    InternalError,
    Unavailable,
    Busy,
    Conflict,
    Other(String),
}

impl ProblemKind {
    pub fn from_code(code: &str) -> ProblemKind {
        match code {
            "token_missing" => ProblemKind::TokenMissing,
            "token_invalid" => ProblemKind::TokenInvalid,
            "token_expired" => ProblemKind::TokenExpired,
            "token_revoked" => ProblemKind::TokenRevoked,
            "worker_retired" => ProblemKind::WorkerRetired,
            "missing_scope" => ProblemKind::MissingScope,
            "target_not_allowed" => ProblemKind::TargetNotAllowed,
            "not_found" => ProblemKind::NotFound,
            "restore_in_progress" => ProblemKind::RestoreInProgress,
            "maintenance" => ProblemKind::Maintenance,
            "capability_unavailable" => ProblemKind::CapabilityUnavailable,
            "idempotency_key_required" => ProblemKind::IdempotencyKeyRequired,
            "idempotency_key_reused" => ProblemKind::IdempotencyKeyReused,
            "already_issued" => ProblemKind::AlreadyIssued,
            "body_too_large" => ProblemKind::BodyTooLarge,
            "invalid_request" => ProblemKind::InvalidRequest,
            "internal_error" => ProblemKind::InternalError,
            "unavailable" => ProblemKind::Unavailable,
            "busy" => ProblemKind::Busy,
            "conflict" => ProblemKind::Conflict,
            other => ProblemKind::Other(other.to_owned()),
        }
    }

    /// The HTTP statuses the twin gives this code (`invalid_request` has four). Empty for `Other`.
    pub fn statuses(&self) -> &'static [u16] {
        match self {
            ProblemKind::TokenMissing
            | ProblemKind::TokenInvalid
            | ProblemKind::TokenExpired
            | ProblemKind::TokenRevoked
            | ProblemKind::WorkerRetired => &[401],
            ProblemKind::MissingScope | ProblemKind::TargetNotAllowed => &[403],
            ProblemKind::NotFound => &[404],
            ProblemKind::RestoreInProgress
            | ProblemKind::Maintenance
            | ProblemKind::CapabilityUnavailable
            | ProblemKind::Unavailable => &[503],
            ProblemKind::IdempotencyKeyRequired => &[428],
            ProblemKind::IdempotencyKeyReused => &[422],
            ProblemKind::AlreadyIssued | ProblemKind::Conflict => &[409],
            ProblemKind::BodyTooLarge => &[413],
            ProblemKind::InvalidRequest => &[400, 405, 415, 422],
            ProblemKind::InternalError => &[500],
            ProblemKind::Busy => &[429],
            ProblemKind::Other(_) => &[],
        }
    }
}

/// An `application/problem+json` answer (R-13), decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// The HTTP status of the answer (it wins over `status` in the body).
    pub status: u16,
    /// The `status` member of the body when it differs from the HTTP status.
    pub body_status: Option<u16>,
    /// The bare code (`not_found`).
    pub code: String,
    pub title: String,
    pub detail: Option<String>,
    pub fix: String,
    pub request_id: String,
    /// `install` or `project` on a scoped 503, as sent.
    pub scope: Option<String>,
    /// `Cortex-Token-Expires` as sent (W1-25: also on refusals after authentication).
    pub token_expires: Option<String>,
    pub token_renew_due: bool,
}

impl Problem {
    pub fn kind(&self) -> ProblemKind {
        ProblemKind::from_code(&self.code)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportKind {
    Refused,
    Timeout,
    Closed,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClientError {
    #[error(transparent)]
    Token(#[from] TokenError),
    #[error(transparent)]
    Endpoint(#[from] EndpointError),
    #[error(transparent)]
    Key(#[from] KeyError),
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("the request body is {size} bytes, over the limit of {cap}")]
    RequestTooLarge { size: usize, cap: usize },
    #[error("{}", problem_text(.0))]
    Problem(Box<Problem>),
    /// An error status whose body is not a Problem (a proxy page, empty, plain text, truncated JSON).
    #[error("HTTP {status}: {snippet}")]
    Http {
        status: u16,
        snippet: String,
        request_id: Option<String>,
    },
    #[error("the answer cannot be read: {what}")]
    Decode {
        what: String,
        request_id: Option<String>,
    },
    #[error("the answer is larger than {cap} bytes")]
    ResponseTooLarge { cap: usize },
    #[error("the server answered HTTP {status}, a redirect; redirects are never followed")]
    UnexpectedRedirect { status: u16 },
    /// 429 `busy` on every allowed attempt; `last` is the final answer (it carries the token headers, W1-25).
    #[error("the server is busy: {attempts} attempt(s), each answered 429")]
    Busy {
        attempts: u32,
        retry_after_secs: u64,
        last: Option<Box<Problem>>,
    },
    #[error("{}", transport_text(*kind, addr, key.as_deref()))]
    Transport {
        kind: TransportKind,
        addr: String,
        key: Option<String>,
    },
}

/// The display boundary (ellis F-1; cox's decisions of 6 Nov 18:37Z): what a server said is shown without control characters
/// and within a budget; the structured fields of the `Problem` keep what the server sent.
fn problem_text(p: &Problem) -> String {
    let mut s = format!(
        "{} ({}, HTTP {})",
        safe_text(&p.title, BUDGET_TITLE),
        safe_text(&p.code, BUDGET_IDENT),
        p.status
    );
    if let Some(d) = &p.detail {
        s.push_str(": ");
        s.push_str(&safe_text(d, BUDGET_DETAIL));
    }
    s.push_str(" Fix: ");
    s.push_str(&safe_text(&p.fix, BUDGET_FIX));
    s.push_str(&format!(
        " [request {}]",
        safe_text(&p.request_id, BUDGET_IDENT)
    ));
    s
}

fn transport_text(kind: TransportKind, addr: &str, key: Option<&str>) -> String {
    let mut s = match kind {
        TransportKind::Refused => format!("cannot reach {addr}: connection refused"),
        TransportKind::Timeout => format!("no complete answer from {addr} in time"),
        TransportKind::Closed => format!("{addr} closed the connection before a complete answer"),
        TransportKind::Other => format!("transport error talking to {addr}"),
    };
    // Only a request that MAY have been applied needs the advice: a refused connection sent nothing (ellis F-2).
    if let Some(k) = key
        && kind != TransportKind::Refused
    {
        s.push_str(&format!(
            "; the request may have been applied. Repeat it with the same Idempotency-Key {k}"
        ));
    }
    s
}
