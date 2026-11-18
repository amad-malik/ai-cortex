//! Unpacking a release archive (design §4 step 5, UP-04). A strict reader of a plain `ustar` stream: only regular
//! files and directories, only the names a release may hold, modes set by us, sizes capped. Nothing is ever
//! created outside the directory it is given. Decompression (gzip) is the caller's layer.

use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnpackLimits {
    pub max_entries: usize,
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
}

impl Default for UnpackLimits {
    fn default() -> Self {
        Self {
            max_entries: 256,
            max_file_bytes: 1 << 30,
            max_total_bytes: 1 << 30,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum UnpackError {
    #[error("unpack refused: the target directory already exists")]
    Exists,
    #[error("unpack refused: the archive is damaged or not a plain tar")]
    Corrupt,
    #[error("unpack refused: entry {index} is not allowed in a release (type or name)")]
    Forbidden { index: usize },
    #[error("unpack refused: entry {index} repeats a name")]
    Duplicate { index: usize },
    #[error("unpack refused: the archive is larger than the limits")]
    TooLarge,
    #[error("unpack refused: the release lacks `{name}`")]
    Missing { name: &'static str },
    #[error("unpack failed: local file error: {0}")]
    Io(std::io::ErrorKind),
}

const BLOCK: usize = 512;

/// `None` at a clean end of the stream (no byte read); a block cut short is `Corrupt`.
fn read_block(r: &mut impl Read) -> Result<Option<[u8; BLOCK]>, UnpackError> {
    let mut b = [0u8; BLOCK];
    let mut filled = 0;
    while filled < BLOCK {
        match r.read(&mut b[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(io(e)),
        }
    }
    match filled {
        0 => Ok(None),
        BLOCK => Ok(Some(b)),
        _ => Err(UnpackError::Corrupt),
    }
}

/// An octal field: digits only, ended by a NUL or a space; nothing else (no base-256, no sign, no blanks first).
fn octal(field: &[u8]) -> Result<u64, UnpackError> {
    let end = field
        .iter()
        .position(|b| *b == 0 || *b == b' ')
        .unwrap_or(field.len());
    let digits = &field[..end];
    if digits.is_empty() || digits.len() > 11 || !digits.iter().all(|b| (b'0'..=b'7').contains(b)) {
        return Err(UnpackError::Corrupt);
    }
    Ok(digits
        .iter()
        .fold(0u64, |acc, b| acc * 8 + u64::from(b - b'0')))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    File(u32),
    Dir,
}

/// The member list this unpacker knows (cox R-4: the manifest says `cortex.members = N`, and an engine refuses a release whose list is newer than
/// the one it knows). List 1 was today's five files and `initdb/`; list 2 adds the example config, the backup scripts and the user units
/// (`pieces/14-cortex-upgrade-MEMBER-LIST-v2-r0.md`). 0.2.1 is the first release, so list 2 is the list from day one.
pub const MEMBER_LIST: u32 = 2;

/// The directories of a release. Each may be an entry of its own or implied by a file under it; each is required.
const DIRS: [&str; 3] = ["initdb", "bin", "systemd"];
/// The backup scripts (0755) and the user units (0644) of list 2: EXACT names, no pattern, all required.
const BIN_MEMBERS: [&str; 5] = [
    "bin/cortex-dump",
    "bin/cortex-dump-status",
    "bin/cortex-dump-alert",
    "bin/cortex-restore",
    "bin/restore-compare",
];
const UNIT_MEMBERS: [&str; 5] = [
    "systemd/cortex-dump.service",
    "systemd/cortex-dump.timer",
    "systemd/cortex-dump-status.service",
    "systemd/cortex-dump-status.timer",
    "systemd/cortex-dump-alert.service",
];

/// The only names a release may hold, with the mode WE give them. `None` means forbidden.
fn classify(name: &str, is_dir: bool) -> Option<Kind> {
    if is_dir {
        return DIRS
            .iter()
            .any(|d| name == *d || name.strip_suffix('/') == Some(*d))
            .then_some(Kind::Dir);
    }
    match name {
        "cortex" => Some(Kind::File(0o755)),
        "compose.yml" | "stack.env" | "cortex.toml.example" => Some(Kind::File(0o644)),
        "db-roles.sh" => Some(Kind::File(0o755)),
        _ if BIN_MEMBERS.contains(&name) => Some(Kind::File(0o755)),
        _ if UNIT_MEMBERS.contains(&name) => Some(Kind::File(0o644)),
        _ => {
            let f = name.strip_prefix("initdb/")?;
            let ok = (1..=64).contains(&f.len())
                && !f.starts_with('.')
                && f.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'));
            ok.then_some(Kind::File(0o644))
        }
    }
}

/// The directory a file name lives in, when it is one of the release's directories.
fn dir_of(key: &str) -> Option<&'static str> {
    DIRS.iter().copied().find(|d| {
        key.strip_prefix(*d)
            .is_some_and(|rest| rest.starts_with('/'))
    })
}

/// A local file error stays an `Io` error; a stream that is damaged (a bad gzip header or checksum, a cut stream) is
/// `Corrupt`: the decoder reports those as these kinds.
fn io(e: std::io::Error) -> UnpackError {
    use std::io::ErrorKind::{InvalidData, InvalidInput, UnexpectedEof};
    match e.kind() {
        InvalidData | InvalidInput | UnexpectedEof => UnpackError::Corrupt,
        kind => UnpackError::Io(kind),
    }
}

/// Create the NEW directory `path` with mode 0700 given to the `mkdir` itself, so that it is never visible with a
/// wider mode (a create under the process umask followed by a chmod would be). The kernel still takes the umask's bits
/// away, so a very strict umask can leave it narrower than 0700; the caller sets exactly 0700 afterwards. An existing
/// path of any kind is `Exists` and is left alone.
pub fn create_private_dir(path: &Path) -> Result<(), UnpackError> {
    use std::os::unix::fs::DirBuilderExt;
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(UnpackError::Exists),
        Err(e) => Err(io(e)),
    }
}

/// [`create_private_dir`], and then EXACTLY 0700: a strict umask (277, 377) takes owner bits away from the `mkdir`, and a
/// directory the owner cannot enter or write is useless to the caller. The directory is never visible with more than
/// 0700. If the mode cannot be set the new directory is removed again.
pub fn create_private_dir_exact(path: &Path) -> Result<(), UnpackError> {
    create_private_dir(path)?;
    if let Err(e) = fs::set_permissions(path, fs::Permissions::from_mode(0o700)) {
        let _ = fs::remove_dir(path);
        return Err(io(e));
    }
    Ok(())
}

/// Unpack into the NEW directory `into` (created 0700; its parent must exist; an existing path of any kind is
/// refused and left alone). Only these may be in the archive: regular files `cortex`, `compose.yml`, `stack.env`,
/// `db-roles.sh` (required, mode 0755; the module-roles script, cox 14:30Z), `cortex.toml.example`, and list 2's directories `initdb`, `bin`
/// and `systemd` (each required: its entry, or a file under it, must be there) with `initdb/<name>`, the five `bin/` scripts and the five `systemd/` units;
/// each is written with our own mode. Returns the names written, in
/// archive order. On any error `into` is removed. The stream after the end marker is not read.
pub fn unpack_release(
    mut tar: impl Read,
    into: &Path,
    limits: &UnpackLimits,
) -> Result<Vec<String>, UnpackError> {
    create_private_dir_exact(into)?;
    let result = extract(&mut tar, into, limits);
    if result.is_err() {
        let _ = fs::remove_dir_all(into);
    }
    result
}

fn extract(
    tar: &mut impl Read,
    into: &Path,
    limits: &UnpackLimits,
) -> Result<Vec<String>, UnpackError> {
    let (mut names, mut seen, mut total) = (Vec::new(), HashSet::new(), 0u64);
    let mut dirs_made: Vec<&'static str> = Vec::new();
    for index in 0.. {
        let block = match read_block(tar)? {
            None => return Err(UnpackError::Corrupt), // no end marker
            Some(b) => b,
        };
        if block.iter().all(|b| *b == 0) {
            // the end marker is two zero blocks
            return match read_block(tar)? {
                Some(b) if b.iter().all(|x| *x == 0) => finish(&seen, names),
                _ => Err(UnpackError::Corrupt),
            };
        }
        if &block[257..262] != b"ustar" {
            return Err(UnpackError::Corrupt);
        }
        let mut for_sum = block;
        for_sum[148..156].fill(b' ');
        let sum: u64 = for_sum.iter().map(|b| u64::from(*b)).sum();
        if octal(&block[148..156])? != sum {
            return Err(UnpackError::Corrupt);
        }
        let size = octal(&block[124..136])?;
        if index >= limits.max_entries {
            return Err(UnpackError::TooLarge);
        }
        let forbidden = UnpackError::Forbidden { index };
        if block[345..500].iter().any(|b| *b != 0) {
            return Err(forbidden);
        }
        let is_dir = match block[156] {
            b'0' | 0 => false,
            b'5' => true,
            _ => return Err(forbidden),
        };
        let raw = &block[..100];
        let name = std::str::from_utf8(&raw[..raw.iter().position(|b| *b == 0).unwrap_or(100)])
            .map_err(|_| forbidden)?;
        let kind = classify(name, is_dir).ok_or(forbidden)?;
        if is_dir && size != 0 {
            return Err(forbidden);
        }
        let key = name.trim_end_matches('/').to_string();
        if !seen.insert(key.clone()) {
            return Err(UnpackError::Duplicate { index });
        }
        if size > limits.max_file_bytes || total.saturating_add(size) > limits.max_total_bytes {
            return Err(UnpackError::TooLarge);
        }
        total += size;
        match kind {
            Kind::Dir => {
                if let Some(d) = DIRS.iter().copied().find(|d| *d == key) {
                    make_dir(into, d, &mut dirs_made)?;
                }
                names.push(key);
            }
            Kind::File(mode) => {
                if let Some(d) = dir_of(&key)
                    && make_dir(into, d, &mut dirs_made)?
                {
                    names.push(d.to_string());
                }
                write_file(tar, &into.join(&key), size, mode)?;
                names.push(key);
            }
        }
    }
    unreachable!("the loop ends by return")
}

/// Creates the release directory `name` once (0755). Returns whether this call created it.
fn make_dir(
    into: &Path,
    name: &'static str,
    made: &mut Vec<&'static str>,
) -> Result<bool, UnpackError> {
    if made.contains(&name) {
        return Ok(false);
    }
    let dir = into.join(name);
    fs::create_dir(&dir).map_err(io)?;
    fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).map_err(io)?;
    made.push(name);
    Ok(true)
}

fn write_file(tar: &mut impl Read, path: &Path, size: u64, mode: u32) -> Result<(), UnpackError> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(path)
        .map_err(io)?;
    let mut left = size;
    let mut buf = vec![0u8; 64 * 1024];
    while left > 0 {
        let want = usize::try_from(left.min(buf.len() as u64)).unwrap_or(buf.len());
        let n = tar.read(&mut buf[..want]).map_err(io)?;
        if n == 0 {
            return Err(UnpackError::Corrupt); // the archive ends inside the data
        }
        file.write_all(&buf[..n]).map_err(io)?;
        left -= n as u64;
    }
    file.sync_all().map_err(io)?;
    drop(file);
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(io)?;
    let pad = (BLOCK as u64 - size % BLOCK as u64) % BLOCK as u64;
    let mut skip = vec![0u8; usize::try_from(pad).unwrap_or(0)];
    tar.read_exact(&mut skip).map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            UnpackError::Corrupt
        } else {
            io(e)
        }
    })
}

fn finish(seen: &HashSet<String>, names: Vec<String>) -> Result<Vec<String>, UnpackError> {
    for name in ["cortex", "compose.yml", "stack.env", "db-roles.sh"] {
        if !seen.contains(name) {
            return Err(UnpackError::Missing { name });
        }
    }
    // list 2 (R-4): the example config, and the three directories, each as an entry of its own or implied by a file under it. `initdb` since R-7
    // (the release compose bind-mounts ./initdb into the database), `bin` and `systemd` since list 2 (the installer takes the backup scripts and
    // the user units from them); the verifier of a resumed release agrees.
    if !seen.contains("cortex.toml.example") {
        return Err(UnpackError::Missing {
            name: "cortex.toml.example",
        });
    }
    for name in DIRS {
        if !seen.contains(name)
            && !seen
                .iter()
                .any(|k| k.strip_prefix(name).is_some_and(|r| r.starts_with('/')))
        {
            return Err(UnpackError::Missing { name });
        }
    }
    for name in BIN_MEMBERS.into_iter().chain(UNIT_MEMBERS) {
        if !seen.contains(name) {
            return Err(UnpackError::Missing { name });
        }
    }
    Ok(names)
}

/// How much of the gzip member may follow the tar end marker (tar pads to a block size; this is generous).
const GZ_TAIL_MAX: u64 = 1 << 20;

/// [`unpack_release`] over a gzip stream (the release archive is `.tar.gz`). A damaged or truncated gzip stream is
/// `Corrupt`; the caps apply to the DECOMPRESSED bytes, so a gzip bomb is `TooLarge`, not an out-of-memory. The tar
/// reader stops at its end marker; this then reads the rest of the gzip member (at most 1 MiB of it) so that the
/// gzip checksum and length are checked too: a cut trailer, or a long tail, is `Corrupt` and `into` is removed.
/// Bytes after the gzip member are not decoded (the decoder's own buffer may have taken some of them in).
pub fn unpack_release_gz(
    gz: impl Read,
    into: &Path,
    limits: &UnpackLimits,
) -> Result<Vec<String>, UnpackError> {
    let mut decoder = flate2::read::GzDecoder::new(gz);
    let names = unpack_release(&mut decoder, into, limits)?;
    let drained = std::io::copy(&mut (&mut decoder).take(GZ_TAIL_MAX), &mut std::io::sink());
    if !matches!(drained, Ok(n) if n < GZ_TAIL_MAX) {
        let _ = fs::remove_dir_all(into);
        return Err(UnpackError::Corrupt);
    }
    Ok(names)
}
