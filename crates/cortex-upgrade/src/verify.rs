//! Read-only verification of a resumed release against its own SHA256SUMS.
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{Read, Result as IoResult};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

const MAX_SUMS: u64 = 1024 * 1024;
const MAX_LINES: usize = 1000;
const REQUIRED_FILES: &[&str] = &[
    "cortex",
    "compose.yml",
    "stack.env",
    "db-roles.sh",
    "cortex.toml.example",
    "manifest.json",
    "bin/cortex-dump",
    "bin/cortex-dump-status",
    "bin/cortex-dump-alert",
    "bin/cortex-restore",
    "bin/restore-compare",
    "systemd/cortex-dump.service",
    "systemd/cortex-dump.timer",
    "systemd/cortex-dump-status.service",
    "systemd/cortex-dump-status.timer",
    "systemd/cortex-dump-alert.service",
];
const REQUIRED_DIRS: &[&str] = &["initdb", "bin", "systemd"];
struct MemberList {
    files: &'static [&'static str],
    dirs: &'static [&'static str],
}
const LIST_2: MemberList = MemberList {
    files: REQUIRED_FILES,
    dirs: REQUIRED_DIRS,
};
// Linux: refuse the final symlink and do not block on a substituted FIFO.
const O_NOFOLLOW: i32 = 0x20000;
const O_NONBLOCK: i32 = 0x800;

/// Refusals carry a rule, never a file body or absolute filesystem path.
#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    #[error("release verification refused ({rule}): {path}")]
    Refused { rule: &'static str, path: String },
}

fn refused(rule: &'static str) -> VerifyError {
    VerifyError::Refused {
        rule,
        path: ".".to_owned(),
    }
}

impl VerifyError {
    fn at(self, path: &str) -> Self {
        let Self::Refused { rule, .. } = self;
        Self::Refused {
            rule,
            path: path.to_owned(),
        }
    }
}

fn metadata(path: &Path) -> Result<Metadata, VerifyError> {
    fs::symlink_metadata(path).map_err(|_| refused("cannot inspect entry"))
}

fn mode(md: &Metadata, expected: u32) -> Result<(), VerifyError> {
    if md.permissions().mode() & 0o7777 != expected {
        return Err(refused("incorrect entry mode"));
    }
    Ok(())
}

fn open_regular(path: &Path, md: &Metadata) -> Result<File, VerifyError> {
    if !md.file_type().is_file() {
        return Err(refused("entry is not a regular file"));
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(O_NOFOLLOW | O_NONBLOCK)
        .open(path)
        .map_err(|_| refused("cannot open regular file"))?;
    let opened = file
        .metadata()
        .map_err(|_| refused("cannot inspect open file"))?;
    if !opened.is_file() || opened.dev() != md.dev() || opened.ino() != md.ino() {
        return Err(refused("entry changed during verification"));
    }
    Ok(file)
}

fn allowed_path(path: &str, list: &MemberList) -> bool {
    if list.files.contains(&path) {
        return true;
    }
    let Some(name) = path.strip_prefix("initdb/") else {
        return false;
    };
    (1..=64).contains(&name.len())
        && name
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn checksums(dir: &Path, list: &MemberList) -> Result<HashMap<String, String>, VerifyError> {
    let path = dir.join("SHA256SUMS");
    let md = metadata(&path)?;
    mode(&md, 0o644)?;
    if md.len() > MAX_SUMS {
        return Err(refused("checksum file too large"));
    }
    let mut bytes = Vec::new();
    open_regular(&path, &md)?
        .take(MAX_SUMS + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| refused("cannot read checksum file"))?;
    if bytes.len() as u64 > MAX_SUMS {
        return Err(refused("checksum file too large"));
    }
    if !bytes.ends_with(b"\n") {
        return Err(refused("checksum file lacks final newline"));
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| refused("invalid checksum line"))?;
    let mut listed = HashMap::new();
    for (index, line) in text.split_terminator('\n').enumerate() {
        if index >= MAX_LINES {
            return Err(refused("too many checksum lines"));
        }
        let raw = line.as_bytes();
        if raw.len() < 67
            || !raw[..64]
                .iter()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
            || raw[64] != b' '
            || !matches!(raw[65], b' ' | b'*')
        {
            return Err(refused("invalid checksum line"));
        }
        let name = line
            .get(66..)
            .ok_or_else(|| refused("invalid checksum line"))?;
        if !allowed_path(name, list) || name == "SHA256SUMS" {
            return Err(refused("invalid checksum path"));
        }
        if listed
            .insert(name.to_owned(), line[..64].to_owned())
            .is_some()
        {
            return Err(refused("duplicate checksum path"));
        }
    }
    for name in list.files {
        if !listed.contains_key(*name) {
            return Err(refused("required member is not listed").at(name));
        }
    }
    Ok(listed)
}

fn hash_file(file: &mut File) -> IoResult<String> {
    let mut hash = Sha256::new();
    let mut buffer = [0u8; 16384];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

/// The two members whose verified bytes are handed over (R-4a): each is read ONCE into one bounded buffer, THAT buffer is hashed, and
/// THAT buffer is kept; every other member is hashed as a stream and kept nowhere.
const CAPTURED: [&str; 2] = ["db-roles.sh", "compose.yml"];

/// Read ONE file whole into a buffer of at most [`STAGED_FILE_MAX_BYTES`] (one byte more is read to tell "too big" from "exactly the limit").
fn read_whole(file: &mut File, rel: &str) -> Result<Vec<u8>, VerifyError> {
    let mut bytes = Vec::new();
    file.take(STAGED_FILE_MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| refused("cannot read entry").at(rel))?;
    if bytes.len() > STAGED_FILE_MAX_BYTES {
        return Err(refused("file exceeds its size limit").at(rel));
    }
    Ok(bytes)
}

fn walk(
    dir: &Path,
    prefix: &str,
    list: &MemberList,
    listed: &HashMap<String, String>,
    seen: &mut HashSet<String>,
    captured: &mut HashMap<String, Vec<u8>>,
) -> Result<(), VerifyError> {
    for entry in fs::read_dir(dir).map_err(|_| refused("cannot read directory"))? {
        let entry = entry.map_err(|_| refused("cannot read directory entry"))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| refused("invalid entry name"))?;
        let rel = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if rel == "SHA256SUMS" {
            continue;
        }
        let md = metadata(&entry.path()).map_err(|e| e.at("<entry>"))?;
        if list.dirs.contains(&rel.as_str()) && md.file_type().is_dir() {
            mode(&md, 0o755).map_err(|e| e.at(&rel))?;
            walk(&entry.path(), &rel, list, listed, seen, captured)?;
            continue;
        }
        if !allowed_path(&rel, list) {
            return Err(refused("invalid entry path"));
        }
        let expected = listed
            .get(&rel)
            .ok_or_else(|| refused("unlisted entry").at(&rel))?;
        let expected_mode = if rel == "cortex" || rel == "db-roles.sh" || rel.starts_with("bin/") {
            0o755
        } else {
            0o644
        };
        mode(&md, expected_mode).map_err(|e| e.at(&rel))?;
        let mut file = open_regular(&entry.path(), &md).map_err(|e| e.at(&rel))?;
        if CAPTURED.contains(&rel.as_str()) {
            // read ONCE; the checksum is of THIS buffer and THIS buffer is kept
            let bytes = read_whole(&mut file, &rel)?;
            if &format!("{:x}", Sha256::digest(&bytes)) != expected {
                return Err(refused("checksum mismatch").at(&rel));
            }
            captured.insert(rel.clone(), bytes);
        } else {
            let actual = hash_file(&mut file).map_err(|_| refused("cannot read entry").at(&rel))?;
            if &actual != expected {
                return Err(refused("checksum mismatch").at(&rel));
            }
        }
        seen.insert(rel);
    }
    Ok(())
}

fn read_bounded(dir: &Path, name: &str, limit: usize) -> Result<Vec<u8>, VerifyError> {
    let path = dir.join(name);
    let md = metadata(&path).map_err(|e| e.at(name))?;
    if md.len() > limit as u64 {
        return Err(refused("file exceeds its size limit").at(name));
    }
    let mut bytes = Vec::new();
    open_regular(&path, &md)
        .map_err(|e| e.at(name))?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| refused("cannot read file").at(name))?;
    if bytes.len() > limit {
        return Err(refused("file exceeds its size limit").at(name));
    }
    Ok(bytes)
}

fn selected_release(dir: &Path) -> Result<crate::manifest::Release, VerifyError> {
    let env_bytes = read_bounded(dir, "stack.env", crate::stack_env::STACK_ENV_MAX_BYTES)?;
    let env_text = std::str::from_utf8(&env_bytes)
        .map_err(|_| refused("invalid stack configuration").at("stack.env"))?;
    let env = crate::stack_env::parse_stack_env(env_text)
        .map_err(|_| refused("invalid stack configuration").at("stack.env"))?;
    let manifest_bytes = read_bounded(dir, "manifest.json", crate::manifest::MANIFEST_MAX_BYTES)?;
    let manifest = crate::manifest::parse(&manifest_bytes)
        .map_err(|_| refused("invalid release manifest").at("manifest.json"))?;
    manifest
        .entries
        .into_iter()
        .find_map(|entry| match entry {
            crate::manifest::Entry::Usable(release) if release.version == env.version => {
                Some(*release)
            }
            _ => None,
        })
        .ok_or_else(|| refused("stack version is absent from manifest").at("manifest.json"))
}

fn verify_roles_against_manifest(
    script: &[u8],
    release: &crate::manifest::Release,
) -> Result<(), VerifyError> {
    let actual = format!("{:x}", Sha256::digest(script));
    let expected = cortex_types::Sha256(release.cortex.roles_script_digest.0).to_hex();
    if actual != expected {
        return Err(refused("manifest roles digest mismatch").at("db-roles.sh"));
    }
    Ok(())
}

/// The most bytes either of the two staged files may have (the adapter reads `compose.yml` with the same limit).
pub const STAGED_FILE_MAX_BYTES: usize = 1024 * 1024;

/// The VERIFIED bytes of the two files the roles step runs from (R-4a): each was read ONCE into one buffer, THAT buffer was hashed against
/// the release's `SHA256SUMS` (and the script's against the manifest's roles digest), and THAT buffer is what is handed over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseBytes {
    pub roles_script: Vec<u8>,
    pub compose_yml: Vec<u8>,
}

/// Verify all listed files, the complete inventory and its modes, without modifying the tree, and return the VERIFIED bytes of `db-roles.sh`
/// and `compose.yml`: each was read once, that buffer was hashed against `SHA256SUMS` (the script's also against the manifest's roles digest) and
/// that buffer is returned, so a caller that writes THESE bytes runs exactly what was verified. A refusal is the refusal of
/// [`verify_release_dir`] (which is this function without the bytes).
pub fn verify_release_dir_bytes(dir: &Path) -> Result<ReleaseBytes, VerifyError> {
    let md = metadata(dir)?;
    if !md.file_type().is_dir() {
        return Err(refused("release is not a real directory"));
    }
    let release = selected_release(dir)?;
    let list = match release.cortex.members {
        2 => &LIST_2,
        _ => return Err(refused("unknown member list version").at("manifest.json")),
    };
    for name in list.dirs {
        let md = metadata(&dir.join(name)).map_err(|e| e.at(name))?;
        if !md.file_type().is_dir() {
            return Err(refused("required member is not a real directory").at(name));
        }
        mode(&md, 0o755).map_err(|e| e.at(name))?;
    }
    let listed = checksums(dir, list).map_err(|e| e.at("SHA256SUMS"))?;
    let mut seen = HashSet::new();
    let mut captured = HashMap::new();
    walk(dir, "", list, &listed, &mut seen, &mut captured)?;
    if seen.len() != listed.len() {
        let missing = listed
            .keys()
            .find(|name| !seen.contains(*name))
            .map_or("<entry>", String::as_str);
        return Err(refused("listed entry is missing").at(missing));
    }
    let roles_script = captured
        .remove("db-roles.sh")
        .ok_or_else(|| refused("cannot read entry").at("db-roles.sh"))?;
    let compose_yml = captured
        .remove("compose.yml")
        .ok_or_else(|| refused("cannot read entry").at("compose.yml"))?;
    verify_roles_against_manifest(&roles_script, &release)?;
    Ok(ReleaseBytes {
        roles_script,
        compose_yml,
    })
}

/// Verify all listed files, the complete inventory and its modes, without modifying the tree.
pub fn verify_release_dir(dir: &Path) -> Result<(), VerifyError> {
    verify_release_dir_bytes(dir).map(|_| ())
}
