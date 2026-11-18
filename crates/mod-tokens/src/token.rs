//! The token's format, generation and digest (S1 4.1; cells TOK-01 to TOK-04, TOK-72, TOK-73).
//!
//! A token is `ctx2_<32 hex>.<43 base64url>`: the hex part is the token's id (a UUIDv7 without hyphens), the
//! part after the dot is 32 bytes of OS randomness in base64url without padding. Only the SHA-256 of the WHOLE
//! string is stored. [`parse`] is pure: it never touches a database, so a malformed bearer costs no lookup.
//!
//! **The secret's one source (cox W1-32):** [`generate`] takes its 32 bytes from `getrandom` (the operating
//! system) and from nothing else. A failure of the OS source is an [`EntropyError`] that refuses the issue
//! (fail closed, 500, nothing stored). There is NO fallback generator.
//!
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use cortex_types::{Clock, Secret, TokenId};
use sha2::{Digest, Sha256};

/// The fixed prefix of every token.
pub const PREFIX: &str = "ctx2_";
/// The secret's size in bytes (256 bits).
pub const SECRET_BYTES: usize = 32;
/// The length of a whole token: `ctx2_` (5) + 32 hex + `.` + 43 base64url.
pub const TOKEN_LEN: usize = 5 + 32 + 1 + 43;

/// The operating system could not supply randomness. The issue is refused: fail closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the operating system could not supply randomness")]
pub struct EntropyError;

/// A bearer that is not a token. Carries nothing: not a character of what was presented.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("malformed token")]
pub struct Malformed;

/// A freshly generated token. The whole string is in a [`Secret`]; `Debug` redacts it.
pub struct Generated {
    id: TokenId,
    token: Secret,
    digest: [u8; 32],
}

impl Generated {
    /// The token's id (the hex part of the string).
    pub fn id(&self) -> TokenId {
        self.id
    }
    /// The whole token, shown once to the caller.
    pub fn token(&self) -> &Secret {
        &self.token
    }
    /// SHA-256 of the whole token string: the only thing stored.
    pub fn digest(&self) -> [u8; 32] {
        self.digest
    }
    /// Takes the whole token out (for the one response that carries it).
    pub fn into_token(self) -> Secret {
        self.token
    }
}

impl std::fmt::Debug for Generated {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let _ = (&self.token, &self.digest);
        f.debug_struct("Generated")
            .field("id", &self.id)
            .field("token", &"<redacted>")
            .field("digest", &"<digest>")
            .finish()
    }
}

/// A bearer that parsed: the id to look up and the digest of the whole presented string.
pub struct Parsed {
    id: TokenId,
    digest: [u8; 32],
}

impl Parsed {
    pub fn id(&self) -> TokenId {
        self.id
    }
    pub fn digest(&self) -> [u8; 32] {
        self.digest
    }
}

impl std::fmt::Debug for Parsed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let _ = &self.digest;
        f.debug_struct("Parsed")
            .field("id", &self.id)
            .field("digest", &"<digest>")
            .finish()
    }
}

/// A new token: the id from `clock`, the secret from the operating system.
pub fn generate(clock: &dyn Clock) -> Result<Generated, EntropyError> {
    generate_with(clock, &mut os_fill)
}

/// THE call of the operating system's randomness in this crate (cox W1-32; STA-2 counts it): fills `buf` or fails.
/// Everything else (`OsEntropy`, the stagger) goes through here.
pub fn os_fill(buf: &mut [u8]) -> Result<(), EntropyError> {
    getrandom::fill(buf).map_err(|_| EntropyError)
}

/// [`generate`] with the randomness source injected, for the cells that make it fail or short. `fill` must fill
/// the whole slice or return an error.
pub fn generate_with(
    clock: &dyn Clock,
    fill: &mut dyn FnMut(&mut [u8]) -> Result<(), EntropyError>,
) -> Result<Generated, EntropyError> {
    let id = TokenId::new(clock);
    let mut secret = [0_u8; SECRET_BYTES];
    fill(&mut secret)?;
    let encoded = URL_SAFE_NO_PAD.encode(secret);
    // Best effort: the raw bytes are not kept past the encoding.
    secret.fill(0);
    let full = format!("{PREFIX}{}.{encoded}", id.to_string().replace('-', ""));
    let digest = digest_of(&full);
    Ok(Generated {
        id,
        token: Secret::new(full),
        digest,
    })
}

/// Parses a bearer. `ctx2_`, 32 lowercase hex that form a UUIDv7, a dot, 43 base64url characters that decode to
/// exactly 32 bytes. Anything else is [`Malformed`], and nothing was looked up.
pub fn parse(bearer: &str) -> Result<Parsed, Malformed> {
    let b = bearer.as_bytes();
    if b.len() != TOKEN_LEN || !bearer.is_ascii() || !bearer.starts_with(PREFIX) || b[37] != b'.' {
        return Err(Malformed);
    }
    let hex = &bearer[5..37];
    if !hex
        .bytes()
        .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Err(Malformed);
    }
    let secret = &bearer[38..];
    if !secret
        .bytes()
        .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
    {
        return Err(Malformed);
    }
    // 43 characters of base64url decode to exactly 32 bytes, and only the canonical encoding (no stray bits).
    match URL_SAFE_NO_PAD.decode(secret) {
        // Exactly 32 bytes, and only the CANONICAL spelling: re-encoding must give back what was presented, so a
        // bearer has one spelling whatever the decoder's configuration says about stray trailing bits.
        Ok(bytes) if bytes.len() == SECRET_BYTES && URL_SAFE_NO_PAD.encode(&bytes) == secret => {}
        _ => return Err(Malformed),
    }
    let hyphenated = format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    );
    // A UUIDv7 in the RFC variant, or it is not a token id.
    let id = TokenId::parse(&hyphenated).map_err(|_| Malformed)?;
    Ok(Parsed {
        id,
        digest: digest_of(bearer),
    })
}

/// SHA-256 of a whole token string.
pub fn digest_of(full: &str) -> [u8; 32] {
    Sha256::digest(full.as_bytes()).into()
}
