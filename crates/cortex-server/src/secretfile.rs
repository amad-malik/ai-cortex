//! A secret read from a FILE (the database passwords).
//!
//! The rules: a REGULAR file, opened without following a symlink (`O_NOFOLLOW`) and without blocking on a FIFO (`O_NONBLOCK`), at most
//! [`MAX_SECRET_BYTES`], valid UTF-8 without NUL; only ONE final line ending is not part of the secret (a password may hold spaces);
//! an empty secret is an error. The error is one fixed text: it names neither the path nor any content.

use cortex_types::Secret;
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use zeroize::Zeroizing;

/// Why a secret file was refused. Fixed text: no path, no content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecretFileError;

impl std::fmt::Display for SecretFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the secret file could not be read")
    }
}

/// The largest secret file read.
pub const MAX_SECRET_BYTES: u64 = 4096;

pub fn read_secret_file(path: &Path) -> Result<Secret, SecretFileError> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| SecretFileError)?;
    let meta = file.metadata().map_err(|_| SecretFileError)?;
    if !meta.file_type().is_file() || meta.len() > MAX_SECRET_BYTES {
        return Err(SecretFileError);
    }
    read_secret_from(file)
}

/// The same rules over any reader (the seam that lets a test see exactly which buffers the reader is handed).
pub fn read_secret_from(mut reader: impl Read) -> Result<Secret, SecretFileError> {
    // The plaintext passes through ONE buffer: initialised at its full size (cap plus one byte, to see an over-long input), handed to the
    // reader directly (never `read_to_end`: its default helper reads a short tail into a 32-byte stack probe and copies it, a second
    // un-zeroised copy; vera SERVER-I4-V1), zeroised on EVERY exit, and never turned into a `String` (a `String` or a `FromUtf8Error`
    // would carry the bytes out). `ss_01` records where every read lands.
    let mut buf = Zeroizing::new(vec![0u8; MAX_SECRET_BYTES as usize + 1]);
    let mut len = 0usize;
    while len < buf.len() {
        match reader.read(&mut buf[len..]) {
            Ok(0) => break,
            Ok(n) => len += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => return Err(SecretFileError),
        }
    }
    if len as u64 > MAX_SECRET_BYTES {
        return Err(SecretFileError);
    }
    let text = std::str::from_utf8(&buf[..len]).map_err(|_| SecretFileError)?;
    if text.contains('\0') {
        return Err(SecretFileError);
    }
    let secret = text
        .strip_suffix("\r\n")
        .or_else(|| text.strip_suffix('\n'))
        .unwrap_or(text);
    if secret.is_empty() {
        return Err(SecretFileError);
    }
    Ok(Secret::new(secret))
}
