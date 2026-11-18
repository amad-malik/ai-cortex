//! The ONE validator for names and job roles (S3 R-18; `x-label-grammar`).
//!
//! - **name:** `^[A-Za-z][A-Za-z0-9-]{0,31}$`, at most 32 bytes. The key is its ASCII lowercase.
//! - **job role:** `^[a-z][a-z0-9-]{0,31}$`, at most 32 bytes; a list holds at most 8, no duplicates.
//!
//! Matching is to the END of the string: a trailing newline is not valid (the Python `$` trap,
//! F-11). No regular expression is used; the grammar is written out byte by byte.
//!

/// The longest name or job role, in bytes.
pub const LABEL_MAX: usize = 32;
/// The most job roles one worker may hold.
pub const JOB_ROLES_MAX: usize = 8;

/// Why a job-role list was refused (S3 `job_roles_invalid`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobRolesError {
    /// An element breaks the job-role grammar.
    Grammar,
    /// Two elements are equal.
    Duplicate,
    /// More than [`JOB_ROLES_MAX`] elements.
    TooMany,
}

/// One grammar: a first byte from `first`, then up to `LABEL_MAX - 1` bytes from `rest`. Bytes, not
/// characters: every allowed byte is ASCII, so any multi-byte character fails on its first byte.
fn matches(s: &str, first: fn(u8) -> bool, rest: fn(u8) -> bool) -> bool {
    let b = s.as_bytes();
    (1..=LABEL_MAX).contains(&b.len()) && first(b[0]) && b[1..].iter().all(|c| rest(*c))
}

fn name_first(c: u8) -> bool {
    c.is_ascii_alphabetic()
}

fn name_rest(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'-'
}

fn role_first(c: u8) -> bool {
    c.is_ascii_lowercase()
}

fn role_rest(c: u8) -> bool {
    c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-'
}

/// Is `s` a valid name?
pub fn name_valid(s: &str) -> bool {
    matches(s, name_first, name_rest)
}

/// The name's key (its ASCII lowercase), or `None` if the name is invalid. The grammar is checked
/// on the name as typed, BEFORE lowercasing (F-10): the Kelvin sign never becomes a `k`.
pub fn name_key(s: &str) -> Option<String> {
    name_valid(s).then(|| s.to_ascii_lowercase())
}

/// Is `s` a valid job role?
pub fn job_role_valid(s: &str) -> bool {
    matches(s, role_first, role_rest)
}

/// Checks a whole job-role list: the count, each element's grammar, then duplicates.
pub fn job_roles_check(roles: &[String]) -> Result<(), JobRolesError> {
    if roles.len() > JOB_ROLES_MAX {
        return Err(JobRolesError::TooMany);
    }
    if !roles.iter().all(|r| job_role_valid(r)) {
        return Err(JobRolesError::Grammar);
    }
    let mut seen: Vec<&str> = Vec::with_capacity(roles.len());
    for r in roles {
        if seen.contains(&r.as_str()) {
            return Err(JobRolesError::Duplicate);
        }
        seen.push(r);
    }
    Ok(())
}
