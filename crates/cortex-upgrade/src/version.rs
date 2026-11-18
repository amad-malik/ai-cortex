//! Release versions (design §3): strict SemVer 2.0, no build metadata. `0.2.001` is a display
//! label, not a version, and is never guessed into one.

pub use semver::Version;

/// The longest version string accepted (a bound, so a hostile value cannot be large).
pub const VERSION_MAX_LEN: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VersionError {
    /// `cut` is at most 40 characters of the input, escaped; never the whole value.
    #[error("version is not SemVer 2.0 without build metadata (got: {cut})")]
    NotSemver { cut: String },
}

/// Strict SemVer 2.0 without build metadata, at most [`VERSION_MAX_LEN`] bytes, ASCII only.
/// The value is later put into file names, a command's argument vector and a label, so what is
/// accepted is fixed here and nowhere else. `0.2.001` and `1.0` are not versions.
pub fn parse_version(s: &str) -> Result<Version, VersionError> {
    let refuse = || VersionError::NotSemver {
        cut: crate::text::cut(s),
    };
    if s.len() > VERSION_MAX_LEN {
        return Err(refuse());
    }
    // `releases/<v>.part` is a staging directory: no release may carry that ending in its own name.
    if s.ends_with(".part") {
        return Err(refuse());
    }
    if !s
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+'))
    {
        return Err(refuse());
    }
    let v = Version::parse(s).map_err(|_| refuse())?;
    if !v.build.is_empty() {
        return Err(refuse());
    }
    Ok(v)
}
