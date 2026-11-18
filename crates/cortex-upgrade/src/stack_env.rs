//! `stack.env` (design §2): the three values a release's `compose.yml` is started with. No secrets.

use crate::version::Version;

/// The longest `stack.env` accepted.
pub const STACK_ENV_MAX_BYTES: usize = 8 * 1024;

/// `name@sha256:<64 lowercase hex>`; the digest is what is pinned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRef(pub String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackEnv {
    pub version: Version,
    pub server_image: ImageRef,
    pub db_image: ImageRef,
}

/// A refusal names a line number and a rule, never the text of the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StackEnvError {
    #[error("stack.env refused: larger than {STACK_ENV_MAX_BYTES} bytes")]
    TooLarge,
    #[error("stack.env refused: line {line} is not `KEY=VALUE`, a comment or blank")]
    BadLine { line: usize },
    #[error("stack.env refused: line {line} names a key this file may not hold")]
    UnknownKey { line: usize },
    #[error("stack.env refused: line {line} repeats a key")]
    Duplicate { line: usize },
    #[error("stack.env refused: line {line} has a value outside the allowed characters or length")]
    BadValue { line: usize },
    #[error("stack.env refused: `{key}` is missing")]
    Missing { key: &'static str },
    #[error("stack.env refused: `{key}` is not valid")]
    Invalid { key: &'static str },
}

/// The longest value accepted.
const VALUE_MAX_BYTES: usize = 300;

fn value_char_ok(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | ':' | '/' | '@' | '-')
}

/// `name@sha256:` and exactly 64 lowercase hex digits; `name` is not empty and holds no second `@`.
fn parse_image(v: &str) -> Option<ImageRef> {
    let (name, hex) = v.rsplit_once("@sha256:")?;
    let hex_ok = hex.len() == 64
        && hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    (hex_ok && !name.is_empty() && !name.contains('@')).then(|| ImageRef(v.to_owned()))
}

/// Three keys, each exactly once, in any order; blank lines and `#` comments are allowed. Nothing else:
/// an unknown key (a password, say) is refused whatever its value. A refusal names the line, never its text.
pub fn parse_stack_env(text: &str) -> Result<StackEnv, StackEnvError> {
    if text.len() > STACK_ENV_MAX_BYTES {
        return Err(StackEnvError::TooLarge);
    }
    let (mut version, mut server, mut db) = (None, None, None);
    for (i, raw) in text.split('\n').enumerate() {
        let line = i + 1;
        if raw.trim().is_empty() || raw.trim_start().starts_with('#') {
            continue;
        }
        let Some((key, value)) = raw.split_once('=') else {
            return Err(StackEnvError::BadLine { line });
        };
        let slot = match key {
            "CORTEX_VERSION" => &mut version,
            "CORTEX_SERVER_IMAGE" => &mut server,
            "CORTEX_DB_IMAGE" => &mut db,
            _ => return Err(StackEnvError::UnknownKey { line }),
        };
        if slot.is_some() {
            return Err(StackEnvError::Duplicate { line });
        }
        if value.is_empty() || value.len() > VALUE_MAX_BYTES || !value.chars().all(value_char_ok) {
            return Err(StackEnvError::BadValue { line });
        }
        *slot = Some(value);
    }
    let version = version.ok_or(StackEnvError::Missing {
        key: "CORTEX_VERSION",
    })?;
    let server = server.ok_or(StackEnvError::Missing {
        key: "CORTEX_SERVER_IMAGE",
    })?;
    let db = db.ok_or(StackEnvError::Missing {
        key: "CORTEX_DB_IMAGE",
    })?;
    Ok(StackEnv {
        version: crate::version::parse_version(version).map_err(|_| StackEnvError::Invalid {
            key: "CORTEX_VERSION",
        })?,
        server_image: parse_image(server).ok_or(StackEnvError::Invalid {
            key: "CORTEX_SERVER_IMAGE",
        })?,
        db_image: parse_image(db).ok_or(StackEnvError::Invalid {
            key: "CORTEX_DB_IMAGE",
        })?,
    })
}
