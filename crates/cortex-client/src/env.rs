//! The ONE place that reads the process environment (EN-01). Everything else gets the lookup as a value.

use std::ffi::OsString;
use std::path::Path;

use crate::endpoint::{DEFAULT_SERVER, Endpoint, EndpointError};
use crate::token::{Token, TokenError, read_token_file};

pub const VAR_TOKEN: &str = "CORTEX_TOKEN";
pub const VAR_TOKEN_FILE: &str = "CORTEX_TOKEN_FILE";
pub const VAR_SERVER: &str = "CORTEX_SERVER";

/// A variable lookup. Production passes [`process_env`]; a test passes a map.
pub type Lookup<'a> = &'a dyn Fn(&str) -> Option<OsString>;

/// The process environment.
pub fn process_env(name: &str) -> Option<OsString> {
    std::env::var_os(name)
}

/// The token, from the first source present: `--token-file`, then `CORTEX_TOKEN`, then `CORTEX_TOKEN_FILE`.
///
/// The first source that is PRESENT decides: a set-but-empty source is an error, a broken file is an error, and
/// neither falls through to the next source (fail closed on ambiguity; W1-43 Q-1, Q-7).
pub fn resolve_token(flag: Option<&Path>, env: Lookup<'_>) -> Result<Token, TokenError> {
    if let Some(path) = flag {
        if path.as_os_str().is_empty() {
            return Err(TokenError::Empty {
                source_name: "--token-file".to_owned(),
            });
        }
        return read_token_file(path);
    }
    if let Some(value) = env(VAR_TOKEN) {
        let text = value.to_str().ok_or(TokenError::NotUtf8)?;
        return Token::parse(text, VAR_TOKEN);
    }
    if let Some(value) = env(VAR_TOKEN_FILE) {
        if value.is_empty() {
            return Err(TokenError::Empty {
                source_name: VAR_TOKEN_FILE.to_owned(),
            });
        }
        return read_token_file(Path::new(&value));
    }
    Err(TokenError::NoSource)
}

/// The server: `--server`, then `CORTEX_SERVER`, then the default.
///
/// A source that is present but invalid is an error; it does not fall back to the default.
pub fn resolve_server(flag: Option<&str>, env: Lookup<'_>) -> Result<Endpoint, EndpointError> {
    if let Some(text) = flag {
        return Endpoint::parse(text);
    }
    if let Some(value) = env(VAR_SERVER) {
        return Endpoint::parse(value.to_str().ok_or(EndpointError::Shape)?);
    }
    Endpoint::parse(DEFAULT_SERVER)
}
