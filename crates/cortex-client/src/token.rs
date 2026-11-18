//! The worker token: a redacted, zeroized value and the one reader of a token file (TF-01 to TF-11).

use std::fmt;
use std::fs;
use std::io::Read;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use zeroize::{ZeroizeOnDrop, Zeroizing};

/// The longest token the client accepts, in bytes (W1-43 Q-7).
pub const MAX_TOKEN_BYTES: usize = 4096;

/// A worker token. No `Clone`, no `Display`, no `Serialize`; `Debug` prints `Token(<redacted>)`; the bytes are
/// wiped on drop. [`Token::expose`] is the single way to read it (the `Authorization` header).
#[derive(ZeroizeOnDrop)]
pub struct Token(String);

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Token(<redacted>)")
    }
}

/// Why a token could not be obtained. Never carries token bytes; paths and octal modes only.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TokenError {
    /// The token file has group or other permission bits (S1 4.5; W1-43 Q-7).
    #[error(
        "token file {path} has mode {mode:04o}: it must not be accessible to group or others (chmod 600)"
    )]
    FileMode { path: String, mode: u32 },
    /// The directory that holds the token file has group or other permission bits.
    #[error(
        "token directory {path} has mode {mode:04o}: it must not be accessible to group or others (chmod 700)"
    )]
    DirMode { path: String, mode: u32 },
    #[error("token file {path} is a symbolic link: name the file itself")]
    Symlink { path: String },
    #[error("token file {path} is not a regular file")]
    NotRegular { path: String },
    #[error("token file {path} does not exist")]
    NotFound { path: String },
    #[error("token file {path} cannot be read: permission denied")]
    PermissionDenied { path: String },
    #[error("token path {path} is a directory, not a file")]
    IsDirectory { path: String },
    #[error("token file {path} cannot be read: {kind}")]
    Io { path: String, kind: String },
    /// A source that was set but holds nothing (a set-but-empty variable does not fall through).
    #[error("the token from {source_name} is empty")]
    Empty { source_name: String },
    #[error("the token is longer than {MAX_TOKEN_BYTES} bytes")]
    TooLong,
    /// A byte outside printable ASCII `0x21..=0x7E` (a space, a newline, NUL, a high byte).
    #[error("the token holds a byte that is not printable ASCII (at offset {offset})")]
    BadByte { offset: usize },
    #[error("the token source is not valid UTF-8")]
    NotUtf8,
    /// No `--token-file`, no `CORTEX_TOKEN`, no `CORTEX_TOKEN_FILE`.
    #[error("no token: give --token-file PATH, or set CORTEX_TOKEN, or set CORTEX_TOKEN_FILE")]
    NoSource,
}

impl Token {
    /// Check the grammar (printable ASCII `0x21..=0x7E`, 1 to [`MAX_TOKEN_BYTES`]) of an already trimmed token.
    pub fn parse(text: &str, source_name: &str) -> Result<Token, TokenError> {
        Token::parse_bytes(text.as_bytes(), source_name)
    }

    fn parse_bytes(bytes: &[u8], source_name: &str) -> Result<Token, TokenError> {
        if bytes.is_empty() {
            return Err(TokenError::Empty {
                source_name: source_name.to_owned(),
            });
        }
        if bytes.len() > MAX_TOKEN_BYTES {
            return Err(TokenError::TooLong);
        }
        if let Some(offset) = bytes.iter().position(|b| !(0x21..=0x7e).contains(b)) {
            return Err(TokenError::BadByte { offset });
        }
        // Printable ASCII is valid UTF-8, so this cannot fail; the error arm keeps the function total.
        String::from_utf8(bytes.to_vec())
            .map(Token)
            .map_err(|_| TokenError::NotUtf8)
    }

    /// The token text, for the `Authorization` header and nowhere else.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

fn io_error(path: &str, e: &std::io::Error) -> TokenError {
    match e.kind() {
        std::io::ErrorKind::NotFound => TokenError::NotFound {
            path: path.to_owned(),
        },
        std::io::ErrorKind::PermissionDenied => TokenError::PermissionDenied {
            path: path.to_owned(),
        },
        // The kind only: an OS error text can name a path or a value.
        kind => TokenError::Io {
            path: path.to_owned(),
            kind: format!("{kind:?}"),
        },
    }
}

/// Read a token file: refuses a symbolic link as the last component, a non-regular file, a directory or file with
/// group or other permission bits (the mode of the OPEN file), an empty file and a token outside the grammar.
///
/// The path is examined with `lstat` first (so a FIFO is refused without an `open` that could block), the directory
/// that holds the file is checked, the file is opened, and the OPEN file is checked again (`fstat`: a regular file,
/// the same device and inode as the `lstat`, no group or other bits) before one byte is read.
pub fn read_token_file(path: &Path) -> Result<Token, TokenError> {
    let shown = path.display().to_string();
    let before = fs::symlink_metadata(path).map_err(|e| io_error(&shown, &e))?;
    let kind = before.file_type();
    if kind.is_symlink() {
        return Err(TokenError::Symlink { path: shown });
    }
    if kind.is_dir() {
        return Err(TokenError::IsDirectory { path: shown });
    }
    if !kind.is_file() {
        return Err(TokenError::NotRegular { path: shown });
    }

    let parent = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let parent_meta =
        fs::metadata(parent).map_err(|e| io_error(&parent.display().to_string(), &e))?;
    if parent_meta.mode() & 0o077 != 0 {
        return Err(TokenError::DirMode {
            path: parent.display().to_string(),
            mode: parent_meta.mode() & 0o777,
        });
    }
    if before.mode() & 0o077 != 0 {
        return Err(TokenError::FileMode {
            path: shown,
            mode: before.mode() & 0o777,
        });
    }

    let file = fs::File::open(path).map_err(|e| io_error(&shown, &e))?;
    let open = file.metadata().map_err(|e| io_error(&shown, &e))?;
    if !open.file_type().is_file() || open.dev() != before.dev() || open.ino() != before.ino() {
        return Err(TokenError::NotRegular { path: shown });
    }
    if open.mode() & 0o077 != 0 {
        return Err(TokenError::FileMode {
            path: shown,
            mode: open.mode() & 0o777,
        });
    }

    // Room for the token and its line ending, and one byte more to see that the file is too long.
    let mut buf = Zeroizing::new(Vec::with_capacity(MAX_TOKEN_BYTES + 3));
    file.take(MAX_TOKEN_BYTES as u64 + 3)
        .read_to_end(&mut buf)
        .map_err(|e| io_error(&shown, &e))?;
    if buf.len() > MAX_TOKEN_BYTES + 2 {
        return Err(TokenError::TooLong);
    }
    if buf
        .iter()
        .all(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
    {
        return Err(TokenError::Empty { source_name: shown });
    }
    let mut end = buf.len();
    if buf[..end].ends_with(b"\n") {
        end -= 1;
        if buf[..end].ends_with(b"\r") {
            end -= 1;
        }
    }
    Token::parse_bytes(&buf[..end], &shown)
}
