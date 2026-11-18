//! A request: method, a path from a template with typed ids, a form-encoded query, an optional JSON body, and whether it
//! mutates (the caller says so; the method does not decide: `POST /v2/search` is a POST that mutates nothing).

use crate::error::ClientError;
use crate::idempotency::IdempotencyKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
    Put,
    Delete,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Method::Get => "GET",
            Method::Post => "POST",
            Method::Put => "PUT",
            Method::Delete => "DELETE",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mutates {
    Yes,
    No,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Auth {
    /// `Authorization: Bearer <token>`.
    Token,
    /// The open routes (the three health routes): no Authorization header at all.
    None,
}

/// An id that is a lower-case hyphenated UUID and nothing else (`../x`, `%2f`, an upper-case or braced UUID cannot be built).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathId(String);

impl PathId {
    pub fn parse(text: &str) -> Result<PathId, ClientError> {
        let canonical = text.len() == 36
            && uuid::Uuid::parse_str(text).is_ok_and(|u| u.hyphenated().to_string() == text);
        if canonical {
            Ok(PathId(text.to_owned()))
        } else {
            Err(ClientError::BadRequest(
                "an id is a lower-case hyphenated UUID".to_owned(),
            ))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub struct Request {
    pub(crate) method: Method,
    pub(crate) path: String,
    pub(crate) query: Vec<(String, String)>,
    pub(crate) body: Option<Vec<u8>>,
    pub(crate) mutates: Mutates,
    pub(crate) auth: Auth,
    pub(crate) key: Option<IdempotencyKey>,
}

impl Request {
    /// `template` has `{name}` placeholders, filled in order from `ids`; counts must match.
    pub fn new(
        method: Method,
        template: &str,
        ids: &[PathId],
        mutates: Mutates,
    ) -> Result<Request, ClientError> {
        let bad = |why: &str| Err(ClientError::BadRequest(why.to_owned()));
        if !template.starts_with('/')
            || !template.bytes().all(|b| {
                b.is_ascii_alphanumeric() || matches!(b, b'/' | b'_' | b'-' | b'.' | b'{' | b'}')
            })
        {
            return bad("a path template starts with / and holds letters, digits and / _ - . { }");
        }
        let mut path = String::with_capacity(template.len() + 36 * ids.len());
        let mut rest = template;
        let mut used = 0usize;
        while let Some(open) = rest.find('{') {
            let close = match rest[open..].find('}') {
                Some(c) => open + c,
                None => return bad("a path template has an unclosed {"),
            };
            if rest[open + 1..close].is_empty() || rest[open + 1..close].contains('{') {
                return bad("a path template has an empty or nested placeholder");
            }
            let Some(id) = ids.get(used) else {
                return bad("a path template has more placeholders than ids");
            };
            path.push_str(&rest[..open]);
            path.push_str(id.as_str());
            used += 1;
            rest = &rest[close + 1..];
        }
        if rest.contains('}') {
            return bad("a path template has a stray }");
        }
        path.push_str(rest);
        if used != ids.len() {
            return bad("a path template has fewer placeholders than ids");
        }
        Ok(Request {
            method,
            path,
            query: Vec::new(),
            body: None,
            mutates,
            auth: Auth::Token,
            key: None,
        })
    }

    pub fn query(mut self, name: &str, value: &str) -> Request {
        self.query.push((name.to_owned(), value.to_owned()));
        self
    }

    /// A JSON object body.
    pub fn json_body(mut self, bytes: Vec<u8>) -> Request {
        self.body = Some(bytes);
        self
    }

    /// For the open routes.
    pub fn open(mut self) -> Request {
        self.auth = Auth::None;
        self
    }

    /// A key chosen by the caller (it retries across process runs): validated by `IdempotencyKey::parse`.
    pub fn with_key(mut self, key: IdempotencyKey) -> Request {
        self.key = Some(key);
        self
    }
}

/// Percent-encode one query component: unreserved characters (RFC 3986) stay, every other byte becomes `%XX`.
pub(crate) fn encode_component(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
