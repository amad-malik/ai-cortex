//! `cortex --version`: the crate version and the git sha the build was given (W1-43 Q-3). No `build.rs`.

/// `CORTEX_GIT_SHA` at BUILD time (the S5 Containerfile passes it), or `unknown`.
pub const GIT_SHA: &str = match option_env!("CORTEX_GIT_SHA") {
    Some(sha) => sha,
    None => "unknown",
};

/// `0.1.0 (unknown)`; clap prefixes the command name. Built once, `'static` (clap's `version` wants a `'static` str).
pub fn line() -> &'static str {
    static LINE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    LINE.get_or_init(|| format!("{} ({})", env!("CARGO_PKG_VERSION"), GIT_SHA))
        .as_str()
}
