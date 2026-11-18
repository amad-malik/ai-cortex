//! R-5 (cox's rulings W1-59, W1-60, W1-61 and W1-62): the ten member files of a release that live OUTSIDE the release directory, the five scripts of
//! `<home>/bin` and the five user units. The state is a PER-ENTRY RECORD (`<home>/members.update`, later `<home>/members.last`): for every entry its name, what
//! the destination was (absent, a file with its sha256 and mode, or a link with its target) and a STATE (planned, backed-up, replaced, restored), with the
//! upgrade's from, to and attempt id in the header. The record is written durably BEFORE the first change and rewritten atomically at every transition. The old
//! file is kept as `<dest>.prev-<id>` by a HARD LINK while the destination stays in place (a verified copy where a link is impossible); the new file is written
//! to `<dest>.new` and renamed over the destination, so the destination is never absent. A restore trusts a backup only by state AND hash, is idempotent, keeps
//! every backup until EVERY entry is restored and verified, and never turns a failure into a warning. Nothing here calls `systemctl` (cox R-5 Q2 (b)).

use crate::layout::valid_attempt_id;
use crate::version::parse_version;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};

/// The database container name the units are rendered with (cox R-5 Q1 (a)): one constant, tied by one cell to compose.yml, install.sh and db-roles.sh.
pub const DB_CONTAINER: &str = "cv2-s5-db";

/// The five scripts of `<home>/bin` and the five user units, by file name.
pub const BIN_FILES: [&str; 5] = [
    "cortex-dump",
    "cortex-dump-status",
    "cortex-dump-alert",
    "cortex-restore",
    "restore-compare",
];
pub const UNIT_FILES: [&str; 5] = [
    "cortex-dump.service",
    "cortex-dump.timer",
    "cortex-dump-status.service",
    "cortex-dump-status.timer",
    "cortex-dump-alert.service",
];

/// A member is a small text file; more than this is not one of ours.
const MEMBER_MAX_BYTES: u64 = 256 * 1024;
const UPDATE: &str = "members.update";
const LAST: &str = "members.last";
const LAST_NEW: &str = "members.last.new";
/// A record is a few lines; more than this is not one of ours.
const RECORD_MAX_BYTES: usize = 16 * 1024;
const RECORD_HEADER: &str = "members-record 1";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MemberError {
    #[error("{0}")]
    Refused(String),
}

fn refuse<T>(text: impl Into<String>) -> Result<T, MemberError> {
    Err(MemberError::Refused(text.into()))
}

fn io(what: &str, path: &Path, e: &std::io::Error) -> MemberError {
    MemberError::Refused(format!("{what} {}: {e}", path.display()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Bin,
    Unit,
}

impl Kind {
    fn word(self) -> &'static str {
        match self {
            Kind::Bin => "bin",
            Kind::Unit => "unit",
        }
    }
    fn mode(self) -> u32 {
        match self {
            Kind::Bin => 0o755,
            Kind::Unit => 0o644,
        }
    }
}

pub struct Layout<'a> {
    pub release_dir: &'a Path,
    pub home: &'a Path,
    pub unit_dir: &'a Path,
    pub db_container: &'a str,
}

/// The home directory the units are rendered with: absolute, only letters, digits and `. _ / -` (it is written into `ExecStart` lines, which systemd splits on
/// spaces and expands for `%` and `$`; install.sh passes it through `sed`), no empty component (a double slash, a trailing slash, the root alone) and no `..`.
pub fn check_home(home: &Path) -> Result<(), MemberError> {
    let Some(s) = home.to_str() else {
        return refuse("the home directory is not valid UTF-8");
    };
    let bad = |why: &str| refuse(format!("the home directory {s:?} {why}"));
    if !s.starts_with('/') {
        return bad("is not an absolute path");
    }
    if !s
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'/' | b'-'))
    {
        return bad(
            "holds a character other than letters, digits and . _ / - (it is written into systemd units)",
        );
    }
    let parts: Vec<&str> = s[1..].split('/').collect();
    if parts.iter().any(|p| p.is_empty()) {
        return bad("has an empty component (a double slash, a trailing slash or the root alone)");
    }
    if parts.contains(&"..") {
        return bad("has a .. component");
    }
    Ok(())
}

/// The directory of the user units: absolute, a real directory (not a link), owned by the user this process runs as, not writable by group or other.
pub fn check_unit_dir(dir: &Path) -> Result<(), MemberError> {
    if !dir.is_absolute() {
        return refuse(format!(
            "the unit directory {} is not an absolute path",
            dir.display()
        ));
    }
    let m =
        fs::symlink_metadata(dir).map_err(|e| io("cannot look at the unit directory", dir, &e))?;
    if !m.file_type().is_dir() {
        return refuse(format!(
            "the unit directory {} is not a real directory (a link or a file is not accepted)",
            dir.display()
        ));
    }
    let me = fs::metadata("/proc/self")
        .map_err(|e| {
            io(
                "cannot read the owner of this process",
                Path::new("/proc/self"),
                &e,
            )
        })?
        .uid();
    if m.uid() != me {
        return refuse(format!(
            "the unit directory {} belongs to another user",
            dir.display()
        ));
    }
    if m.mode() & 0o022 != 0 {
        return refuse(format!(
            "the unit directory {} is writable by group or other",
            dir.display()
        ));
    }
    Ok(())
}

/// The unit template with `@CORTEX_HOME@` and `@DB_CONTAINER@` filled in. Pure. Any other `@name@` token is refused (nothing else is a placeholder), the home
/// must pass `check_home` and the database container is `[a-z0-9-]` only.
pub fn render(bytes: &[u8], home: &str, db: &str) -> Result<Vec<u8>, MemberError> {
    check_home(Path::new(home))?;
    if db.is_empty()
        || !db
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return refuse(format!(
            "the database container name {db:?} is not [a-z0-9-]"
        ));
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return refuse("a unit template is not valid UTF-8");
    };
    let mut out = String::with_capacity(text.len() + 64);
    let mut rest = text;
    while let Some(at) = rest.find('@') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let name_len = after
            .bytes()
            .take_while(|b| b.is_ascii_alphanumeric() || *b == b'_')
            .count();
        if after.as_bytes().get(name_len) == Some(&b'@') {
            match &after[..name_len] {
                "CORTEX_HOME" => out.push_str(home),
                "DB_CONTAINER" => out.push_str(db),
                other => {
                    return refuse(format!(
                        "a unit template holds the token @{other}@, which is not a placeholder"
                    ));
                }
            }
            rest = &after[name_len + 1..];
        } else {
            out.push('@');
            rest = after;
        }
    }
    out.push_str(rest);
    Ok(out.into_bytes())
}

// ---- the record -------------------------------------------------------------------------------------------------------

/// What a destination was before the upgrade touched it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Old {
    Absent,
    /// A regular file: its sha256 (64 lowercase hex) and its mode (the permission bits).
    File {
        sha256: String,
        mode: u32,
    },
    /// A symbolic link: its target text, hex encoded (bytes, no assumption about the characters).
    Link {
        target_hex: String,
    },
}

/// Where an entry stands: planned (nothing changed), backed-up (the backup is verified, the destination may still be the old file), replaced (the new file is in
/// place), restored (the old state is back).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Planned,
    BackedUp,
    Replaced,
    Restored,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub kind: Kind,
    pub name: String,
    pub old: Old,
    pub state: State,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub from: String,
    pub to: String,
    pub id: String,
    /// The attempt ids whose backups a TRUSTED promotion has retired (W1-75): only a `members.last` carries any; a live `members.update` never does.
    pub retired: Vec<String>,
    pub entries: Vec<Entry>,
}

/// What an install did, for the lines the engine prints: kind, name and whether the file was CREATED (the destination was absent).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    pub entries: Vec<(Kind, String, bool)>,
}

/// A crash point: called with a name at every boundary of the protocol. Production passes a function that does nothing; a test passes one that panics at the
/// boundary it wants to lose the process at.
pub type Point<'a> = &'a dyn Fn(&str);

/// Where an inventory was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Update,
    Last,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inventory {
    pub record: Record,
    pub source: Source,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if text.is_empty()
        || !text.len().is_multiple_of(2)
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).ok())
        .collect()
}

fn sha_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

impl Old {
    fn word(&self) -> String {
        match self {
            Old::Absent => "absent".to_string(),
            Old::File { sha256, mode } => format!("file:{sha256}:{mode:04o}"),
            Old::Link { target_hex } => format!("link:{target_hex}"),
        }
    }

    fn parse(word: &str) -> Option<Old> {
        if word == "absent" {
            return Some(Old::Absent);
        }
        if let Some(rest) = word.strip_prefix("file:") {
            let (sha, mode) = rest.split_once(':')?;
            let lower_hex = |t: &str| {
                t.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            };
            if sha.len() != 64
                || !lower_hex(sha)
                || mode.len() != 4
                || !mode.bytes().all(|b| (b'0'..=b'7').contains(&b))
            {
                return None;
            }
            return Some(Old::File {
                sha256: sha.to_string(),
                mode: u32::from_str_radix(mode, 8).ok()?,
            });
        }
        let target = word.strip_prefix("link:")?;
        unhex(target)?;
        if target.len() > 2 * 4096 {
            return None;
        }
        Some(Old::Link {
            target_hex: target.to_string(),
        })
    }
}

impl State {
    fn word(self) -> &'static str {
        match self {
            State::Planned => "planned",
            State::BackedUp => "backed-up",
            State::Replaced => "replaced",
            State::Restored => "restored",
        }
    }

    fn parse(word: &str) -> Option<State> {
        Some(match word {
            "planned" => State::Planned,
            "backed-up" => State::BackedUp,
            "replaced" => State::Replaced,
            "restored" => State::Restored,
            _ => return None,
        })
    }
}

/// The position of a member in the record: the five scripts, then the five units.
fn canon_index(kind: Kind, name: &str) -> Option<usize> {
    match kind {
        Kind::Bin => BIN_FILES.iter().position(|n| *n == name),
        Kind::Unit => UNIT_FILES
            .iter()
            .position(|n| *n == name)
            .map(|i| BIN_FILES.len() + i),
    }
}

fn valid_pair(from: &str, to: &str, id: &str) -> Result<(), MemberError> {
    let (Ok(f), Ok(t)) = (parse_version(from), parse_version(to)) else {
        return refuse("the upgrade's from and to are not both release versions");
    };
    if f == t {
        return refuse("the upgrade's from and to are the same release");
    }
    if !valid_attempt_id(id) {
        return refuse("the attempt id is not 16 lowercase hex characters");
    }
    Ok(())
}

/// The text form of a record (one exact form; `parse` reads exactly this).
pub fn serialize(r: &Record) -> String {
    let mut out = format!(
        "{RECORD_HEADER}\nfrom={}\nto={}\nid={}\n",
        r.from, r.to, r.id
    );
    for id in &r.retired {
        out.push_str(&format!("retired={id}\n"));
    }
    for e in &r.entries {
        out.push_str(&format!(
            "entry {} {} {} {}\n",
            e.kind.word(),
            e.name,
            e.old.word(),
            e.state.word()
        ));
    }
    out
}

/// Read a record: exactly the form `serialize` writes, nothing else (at most 16 KiB, LF only, header, from, to, id, then the entries in the fixed order, each
/// member at most once).
pub fn parse(text: &str) -> Result<Record, MemberError> {
    fn bad<T>(why: &str) -> Result<T, MemberError> {
        refuse(format!("not a member record this engine wrote: {why}"))
    }
    if text.len() > RECORD_MAX_BYTES {
        return bad("it is larger than 16 KiB");
    }
    if !text.ends_with('\n') || text.contains('\r') {
        return bad("it does not end with exactly one line feed, or has a carriage return");
    }
    let lines: Vec<&str> = text[..text.len() - 1].split('\n').collect();
    if lines.first() != Some(&RECORD_HEADER) {
        return bad("the header line is not the one of this format");
    }
    let key = |n: usize, k: &str| lines.get(n).and_then(|l| l.strip_prefix(k));
    let (Some(from), Some(to), Some(id)) = (key(1, "from="), key(2, "to="), key(3, "id=")) else {
        return bad("from=, to= and id= are not the lines after the header");
    };
    valid_pair(from, to, id).or_else(|e| bad(&e.to_string()))?;
    // zero to RETIRED_MAX `retired=<id>` lines straight after id= (W1-75): each a valid attempt id, none the record's own, none twice
    let mut retired: Vec<String> = Vec::new();
    let mut next = 4;
    while let Some(r) = lines.get(next).and_then(|l| l.strip_prefix("retired=")) {
        if !valid_attempt_id(r) {
            return bad("a retired id is not 16 lowercase hex characters");
        }
        if r == id {
            return bad("a record retires its own attempt");
        }
        if retired.iter().any(|x| x == r) {
            return bad("an id is retired twice");
        }
        if retired.len() >= RETIRED_MAX {
            return bad("more retired ids than a record may carry");
        }
        retired.push(r.to_string());
        next += 1;
    }
    let mut entries = Vec::new();
    let mut last_index: Option<usize> = None;
    for line in &lines[next..] {
        let parts: Vec<&str> = line.split(' ').collect();
        let [word, kind, name, old, state] = parts[..] else {
            return bad("an entry line does not have its five fields");
        };
        if word != "entry" {
            return bad("a line that is not an entry");
        }
        let kind = match kind {
            "bin" => Kind::Bin,
            "unit" => Kind::Unit,
            _ => return bad("an entry names a kind that is not bin or unit"),
        };
        let Some(index) = canon_index(kind, name) else {
            return bad("an entry names a file that is not one of the ten");
        };
        if last_index.is_some_and(|l| index <= l) {
            return bad("the entries are not in the fixed order, or one is there twice");
        }
        last_index = Some(index);
        let Some(old) = Old::parse(old) else {
            return bad(
                "an entry has an old state that is not absent, file:<sha256>:<mode> or link:<hex>",
            );
        };
        let Some(state) = State::parse(state) else {
            return bad(
                "an entry has a state that is not planned, backed-up, replaced or restored",
            );
        };
        entries.push(Entry {
            kind,
            name: name.to_string(),
            old,
            state,
        });
    }
    Ok(Record {
        from: from.to_string(),
        to: to.to_string(),
        id: id.to_string(),
        retired,
        entries,
    })
}

fn update_path(home: &Path) -> PathBuf {
    home.join(UPDATE)
}

fn last_path(home: &Path) -> PathBuf {
    home.join(LAST)
}

fn read_record_at(path: &Path, label: &str) -> Result<Option<Record>, MemberError> {
    let m = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(io(&format!("cannot look at {label}"), path, &e)),
    };
    if !m.file_type().is_file() || m.len() > RECORD_MAX_BYTES as u64 {
        return refuse(format!(
            "{label} cannot be read: it is not a plain file of the size of a record ({})",
            path.display()
        ));
    }
    let mut bytes = Vec::new();
    File::open(path)
        .and_then(|f| f.take(RECORD_MAX_BYTES as u64 + 1).read_to_end(&mut bytes))
        .map_err(|e| io(&format!("cannot read {label}"), path, &e))?;
    let text = String::from_utf8(bytes).map_err(|_| {
        MemberError::Refused(format!(
            "{label} cannot be read: it is not text ({})",
            path.display()
        ))
    })?;
    parse(&text).map(Some).map_err(|e| {
        MemberError::Refused(format!("{label} cannot be used ({}): {e}", path.display()))
    })
}

/// `members.update` of the home: `Ok(None)` when there is none, a refusal when it cannot be used.
pub fn read_update(home: &Path) -> Result<Option<Record>, MemberError> {
    read_record_at(&update_path(home), UPDATE)
}

/// `members.last` of the home: `Ok(None)` when there is none, a refusal when it cannot be used.
pub fn read_last(home: &Path) -> Result<Option<Record>, MemberError> {
    read_record_at(&last_path(home), LAST)
}

fn sync_dir(dir: &Path) -> Result<(), MemberError> {
    File::open(dir)
        .and_then(|d| d.sync_all())
        .map_err(|e| io("cannot sync the directory", dir, &e))
}

fn remove_if_there(path: &Path) -> Result<(), MemberError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(io("cannot remove", path, &e)),
    }
}

fn sibling(dest: &Path, suffix: &str) -> PathBuf {
    let mut s = dest.as_os_str().to_os_string();
    s.push(suffix);
    PathBuf::from(s)
}

fn backup_of(dest: &Path, id: &str) -> PathBuf {
    sibling(dest, &format!(".prev-{id}"))
}

fn dest_of(kind: Kind, name: &str, home: &Path, unit_dir: &Path) -> PathBuf {
    match kind {
        Kind::Bin => home.join("bin").join(name),
        Kind::Unit => unit_dir.join(name),
    }
}

/// A small file written whole: a temporary name (`create_new`), the bytes, fsync, the mode, then the rename over `path` and a sync of its directory.
fn write_durably(path: &Path, bytes: &[u8], mode: u32) -> Result<(), MemberError> {
    let tmp = sibling(path, &format!(".tmp{}", std::process::id()));
    remove_if_there(&tmp)?;
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(&tmp)
        .map_err(|e| io("cannot create", &tmp, &e))?;
    f.write_all(bytes)
        .and_then(|()| f.sync_all())
        .map_err(|e| io("cannot write", &tmp, &e))?;
    drop(f);
    fs::set_permissions(&tmp, fs::Permissions::from_mode(mode))
        .map_err(|e| io("cannot set the mode of", &tmp, &e))?;
    fs::rename(&tmp, path).map_err(|e| io("cannot publish", path, &e))?;
    sync_dir(path.parent().unwrap_or(Path::new("/")))
}

fn persist(path: &Path, r: &Record) -> Result<(), MemberError> {
    write_durably(path, serialize(r).as_bytes(), 0o600)
}

fn read_regular(path: &Path, what: &str) -> Result<Vec<u8>, MemberError> {
    let m = fs::symlink_metadata(path).map_err(|e| io(what, path, &e))?;
    if !m.file_type().is_file() {
        return refuse(format!("{what} {} is not a regular file", path.display()));
    }
    if m.len() > MEMBER_MAX_BYTES {
        return refuse(format!("{what} {} is too large", path.display()));
    }
    let mut buf = Vec::new();
    File::open(path)
        .and_then(|f| f.take(MEMBER_MAX_BYTES + 1).read_to_end(&mut buf))
        .map_err(|e| io(what, path, &e))?;
    Ok(buf)
}

/// What a destination is now.
#[derive(Debug, PartialEq, Eq)]
enum Dest {
    Absent,
    File { sha256: String, mode: u32 },
    Link { target: Vec<u8> },
}

fn dest_now(path: &Path) -> Result<Dest, MemberError> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Dest::Absent),
        Err(e) => Err(io("cannot look at", path, &e)),
        Ok(m) if m.file_type().is_symlink() => {
            let target = fs::read_link(path).map_err(|e| io("cannot read the link", path, &e))?;
            Ok(Dest::Link {
                target: target.into_os_string().into_vec(),
            })
        }
        Ok(m) if m.file_type().is_file() => Ok(Dest::File {
            sha256: sha_hex(&read_regular(path, "the installed file")?),
            mode: m.permissions().mode() & 0o7777,
        }),
        Ok(_) => refuse(format!(
            "{} is not a file, a link or absent (a directory is not one of our files)",
            path.display()
        )),
    }
}

fn is_old(now: &Dest, old: &Old) -> bool {
    match (now, old) {
        (Dest::Absent, Old::Absent) => true,
        (Dest::File { sha256, mode }, Old::File { sha256: s, mode: m }) => sha256 == s && mode == m,
        (Dest::Link { target }, Old::Link { target_hex }) => hex(target) == *target_hex,
        _ => false,
    }
}

/// One member of a release, read and rendered: its kind, its name and the bytes it must have.
type Wanted = (Kind, &'static str, Vec<u8>);

/// The ten members of release `l.release_dir`, read and rendered, in the fixed order.
fn wanted(l: &Layout) -> Result<Vec<Wanted>, MemberError> {
    let home = l.home.to_str().unwrap_or_default();
    let mut out = Vec::with_capacity(10);
    for name in BIN_FILES {
        let want = read_regular(&l.release_dir.join("bin").join(name), "the release member")?;
        out.push((Kind::Bin, name, want));
    }
    for name in UNIT_FILES {
        let template = read_regular(
            &l.release_dir.join("systemd").join(name),
            "the release member",
        )?;
        out.push((Kind::Unit, name, render(&template, home, l.db_container)?));
    }
    Ok(out)
}

fn all_in(r: &Record, states: &[State]) -> bool {
    r.entries.iter().all(|e| states.contains(&e.state))
}

/// Plan the install of release `l.release_dir`: everything is read and checked BEFORE anything is written (a missing member, a bad home or unit directory, a
/// template with a token that is not a placeholder, a directory at a destination stop with nothing changed); the changes are listed in `members.update`, every
/// entry `planned` with what its destination is now, written durably. A plan of the SAME attempt that is already there is returned as it is; a plan of another
/// attempt in which nothing happened is stale and replaced; one in which something happened, or one that cannot be read, is never replaced.
pub fn plan(
    l: &Layout,
    from: &str,
    to: &str,
    id: &str,
    point: Point,
) -> Result<Record, MemberError> {
    valid_pair(from, to, id)?;
    check_home(l.home)?;
    check_unit_dir(l.unit_dir)?;
    let members = wanted(l)?;
    let mut entries = Vec::new();
    for (kind, name, want) in &members {
        let dest = dest_of(*kind, name, l.home, l.unit_dir);
        let old = match dest_now(&dest)? {
            Dest::Absent => Old::Absent,
            Dest::Link { target } => Old::Link {
                target_hex: hex(&target),
            },
            Dest::File { sha256, mode } => {
                if sha256 == sha_hex(want) && mode == kind.mode() {
                    continue;
                }
                Old::File { sha256, mode }
            }
        };
        entries.push(Entry {
            kind: *kind,
            name: name.to_string(),
            old,
            state: State::Planned,
        });
    }
    match read_update(l.home)? {
        Some(there) if there.id == id => {
            if there.from == from && there.to == to {
                return Ok(there);
            }
            return refuse(format!(
                "members.update is of attempt {id} but of another pair of releases ({} to {}); nothing was changed",
                there.from, there.to
            ));
        }
        Some(there) if !all_in(&there, &[State::Planned, State::Restored]) => {
            return refuse(format!(
                "members.update of attempt {} has work in it (an entry past planned); nothing was changed: it is that attempt's inventory",
                there.id
            ));
        }
        _ => {}
    }
    // the leftover of a promotion that was lost before its rename: only ever a temporary
    remove_if_there(&l.home.join(LAST_NEW))?;
    let record = Record {
        from: from.to_string(),
        to: to.to_string(),
        id: id.to_string(),
        retired: Vec::new(),
        entries,
    };
    persist(&update_path(l.home), &record)?;
    point("plan:written");
    Ok(record)
}

fn entry_fail(name: &str, what: impl std::fmt::Display) -> MemberError {
    MemberError::Refused(format!("{name}: {what}"))
}

/// A backup of `dest` as it is recorded: the file at `backup` is a regular file with the recorded hash and mode.
fn backup_is(backup: &Path, sha256: &str, mode: u32) -> Result<(), String> {
    let m = fs::symlink_metadata(backup).map_err(|e| format!("cannot be looked at ({e})"))?;
    if !m.file_type().is_file() {
        return Err("is not a regular file".to_string());
    }
    if m.permissions().mode() & 0o7777 != mode {
        return Err("has another mode than the one recorded".to_string());
    }
    let bytes = read_regular(backup, "the backup").map_err(|e| e.to_string())?;
    if sha_hex(&bytes) != sha256 {
        return Err("does not hold the bytes that were recorded".to_string());
    }
    Ok(())
}

/// A verified COPY of `from` at `to`, for where a hard link is impossible: a temporary name, the bytes, fsync, the mode, a rename.
fn copy_file(from: &Path, to: &Path, mode: u32) -> Result<(), MemberError> {
    let bytes = read_regular(from, "the file to copy")?;
    write_durably(to, &bytes, mode)
}

/// A hard link of `from` at `to` (the same bytes, no copy), or a copy where the link is refused.
fn link_or_copy(from: &Path, to: &Path, mode: u32) -> Result<(), MemberError> {
    remove_if_there(to)?;
    match fs::hard_link(from, to) {
        Ok(()) => Ok(()),
        Err(_) => copy_file(from, to, mode),
    }
}

/// Keep the old file of a `planned` entry as `<dest>.prev-<id>` while the destination STAYS in place: verified against the record.
fn make_backup(
    name: &str,
    dest: &Path,
    backup: &Path,
    sha256: &str,
    mode: u32,
) -> Result<(), MemberError> {
    // the destination is what the plan recorded, or this entry is not ours to back up
    match dest_now(dest)? {
        Dest::File { sha256: s, mode: m } if s == sha256 && m == mode => {}
        _ => {
            return Err(entry_fail(
                name,
                format!(
                    "{} is not what the plan recorded (it changed after the plan); nothing was replaced",
                    dest.display()
                ),
            ));
        }
    }
    // a backup of THIS attempt's name that is already right (a lost run made it) stays; one that is not is replaced
    if backup_is(backup, sha256, mode).is_err() {
        link_or_copy(dest, backup, mode)?;
    }
    sync_dir(dest.parent().unwrap_or(Path::new("/")))?;
    backup_is(backup, sha256, mode).map_err(|why| {
        entry_fail(
            name,
            format!(
                "the backup {} {why}; nothing was replaced",
                backup.display()
            ),
        )
    })
}

/// The new file: `<dest>.new` with OUR mode, fsynced, renamed over the destination (a link is replaced, never followed), the directory synced.
fn put_new(dest: &Path, want: &[u8], mode: u32) -> Result<(), MemberError> {
    let new = sibling(dest, ".new");
    remove_if_there(&new)?;
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(&new)
        .map_err(|e| io("cannot create", &new, &e))?;
    f.write_all(want)
        .and_then(|()| f.sync_all())
        .map_err(|e| io("cannot write", &new, &e))?;
    drop(f);
    // the process umask may have cleared bits of the mode: it is OURS to set
    fs::set_permissions(&new, fs::Permissions::from_mode(mode))
        .map_err(|e| io("cannot set the mode of", &new, &e))?;
    Ok(())
}

fn load_for(home: &Path, from: &str, to: &str, id: &str) -> Result<Record, MemberError> {
    let Some(rec) = read_update(home)? else {
        return refuse("there is no members.update: the install was never planned");
    };
    if rec.id != id || rec.from != from || rec.to != to {
        return refuse(format!(
            "members.update is of attempt {} ({} to {}), not of attempt {id} ({from} to {to})",
            rec.id, rec.from, rec.to
        ));
    }
    Ok(rec)
}

/// Carry the plan out, entry by entry, by the states of `members.update`. Idempotent: a run that was lost anywhere is continued by the next call with the same
/// id. Forward, per entry: a HARD LINK `<dest>.prev-<id>` while the destination stays in place (nothing for a link or an absent file: the record holds it),
/// verified; THEN `backed-up` is recorded durably; only then `<dest>.new` is written and renamed over the destination (never absent), the directory synced,
/// `replaced` recorded. At every boundary the destination holds the old OR the new file. A failure stops here and changes the record no further: the caller
/// restores from it.
pub fn install(
    l: &Layout,
    from: &str,
    to: &str,
    id: &str,
    point: Point,
) -> Result<Update, MemberError> {
    valid_pair(from, to, id)?;
    check_home(l.home)?;
    check_unit_dir(l.unit_dir)?;
    let members = wanted(l)?;
    let mut rec = load_for(l.home, from, to, id)?;
    let path = update_path(l.home);
    // a home that has no bin directory yet (the installer makes it; an upgrade of an older layout may not have it)
    let bin = l.home.join("bin");
    if matches!(fs::symlink_metadata(&bin), Err(e) if e.kind() == std::io::ErrorKind::NotFound) {
        fs::create_dir(&bin).map_err(|e| io("cannot create", &bin, &e))?;
        fs::set_permissions(&bin, fs::Permissions::from_mode(0o755))
            .map_err(|e| io("cannot set the mode of", &bin, &e))?;
    }
    for i in 0..rec.entries.len() {
        let e = rec.entries[i].clone();
        let Some((_, _, want)) = members
            .iter()
            .find(|(k, n, _)| *k == e.kind && *n == e.name)
        else {
            return Err(entry_fail(
                &e.name,
                "the release does not carry this member",
            ));
        };
        let dest = dest_of(e.kind, &e.name, l.home, l.unit_dir);
        let backup = backup_of(&dest, id);
        let is_new = |d: &Dest| matches!(d, Dest::File { sha256, mode } if *sha256 == sha_hex(want) && *mode == e.kind.mode());
        let mut state = e.state;
        if state == State::Planned {
            match &e.old {
                Old::File { sha256, mode } => {
                    make_backup(&e.name, &dest, &backup, sha256, *mode)?;
                    point(&format!("{}:backup-linked", e.name));
                }
                Old::Absent | Old::Link { .. } => {
                    let now = dest_now(&dest)?;
                    if !is_old(&now, &e.old) {
                        return Err(entry_fail(
                            &e.name,
                            format!(
                                "{} is not what the plan recorded (it changed after the plan); nothing was replaced",
                                dest.display()
                            ),
                        ));
                    }
                }
            }
            rec.entries[i].state = State::BackedUp;
            persist(&path, &rec)?;
            point(&format!("{}:backed-up", e.name));
            state = State::BackedUp;
        }
        match state {
            State::BackedUp => {
                let now = dest_now(&dest)?;
                if !is_new(&now) {
                    // not replaced yet: the destination must be the old state, and the backup (when there is one) must be right
                    if !is_old(&now, &e.old) {
                        return Err(entry_fail(
                            &e.name,
                            format!(
                                "{} is neither the old file nor the new one; nothing was replaced",
                                dest.display()
                            ),
                        ));
                    }
                    if let Old::File { sha256, mode } = &e.old {
                        backup_is(&backup, sha256, *mode).map_err(|why| {
                            entry_fail(
                                &e.name,
                                format!(
                                    "the backup {} {why}; nothing was replaced",
                                    backup.display()
                                ),
                            )
                        })?;
                    }
                    put_new(&dest, want, e.kind.mode())?;
                    point(&format!("{}:new-written", e.name));
                    fs::rename(sibling(&dest, ".new"), &dest)
                        .map_err(|er| io("cannot replace", &dest, &er))?;
                    sync_dir(dest.parent().unwrap_or(Path::new("/")))?;
                }
                point(&format!("{}:renamed", e.name));
                rec.entries[i].state = State::Replaced;
                persist(&path, &rec)?;
                point(&format!("{}:replaced", e.name));
            }
            State::Replaced => {
                if !is_new(&dest_now(&dest)?) {
                    return Err(entry_fail(
                        &e.name,
                        format!(
                            "{} is no longer the file that was installed",
                            dest.display()
                        ),
                    ));
                }
            }
            State::Restored => {
                return refuse(
                    "members.update says an entry was restored: it is being rolled back, not installed",
                );
            }
            State::Planned => {}
        }
    }
    Ok(Update {
        entries: rec
            .entries
            .iter()
            .map(|e| (e.kind, e.name.clone(), e.old == Old::Absent))
            .collect(),
    })
}

fn same_content(a: &Record, b: &Record) -> bool {
    a.from == b.from
        && a.to == b.to
        && a.id == b.id
        && a.entries.len() == b.entries.len()
        && a.entries
            .iter()
            .zip(&b.entries)
            .all(|(x, y)| x.kind == y.kind && x.name == y.name && x.old == y.old)
}

/// The inventory a rollback or a resume works from. `marker` is the unfinished upgrade's `(from, to, id)` (from `upgrade.incomplete`), `left` the release a
/// rollback of a FINISHED upgrade leaves. With a marker: the live `members.update` of that attempt (and, when `members.last` is of the same attempt too, the
/// two must agree: never a pick of one); with `members.update` absent, `members.last` bound to that same from, to and id IS the unfinished upgrade's inventory
/// (the promotion got as far as removing the live record); anything else is a LOST inventory. Without a marker: `members.last` when its `to` is the release
/// being left, none when there is no record at all, a refusal when the record is not usable.
pub fn inventory(
    home: &Path,
    marker: Option<(&str, &str, &str)>,
    left: Option<&str>,
) -> Result<Option<Inventory>, MemberError> {
    let update = read_update(home)?;
    match marker {
        Some((from, to, id)) => match update {
            Some(u) => {
                if (u.from.as_str(), u.to.as_str(), u.id.as_str()) != (from, to, id) {
                    return refuse(format!(
                        "members.update is of attempt {} ({} to {}), not of the unfinished upgrade (attempt {id}, {from} to {to}): its inventory is lost",
                        u.id, u.from, u.to
                    ));
                }
                if let Ok(Some(l)) = read_last(home)
                    && l.id == u.id
                    && !same_content(&l, &u)
                {
                    return refuse(
                        "members.update and members.last are of the same attempt and do not agree (their entries differ): neither is used",
                    );
                }
                Ok(Some(Inventory {
                    record: u,
                    source: Source::Update,
                }))
            }
            None => match read_last(home)? {
                Some(l) if (l.from.as_str(), l.to.as_str(), l.id.as_str()) == (from, to, id) => {
                    Ok(Some(Inventory {
                        record: l,
                        source: Source::Last,
                    }))
                }
                Some(l) => refuse(format!(
                    "the inventory of the unfinished upgrade (attempt {id}, {from} to {to}) is lost: members.update is absent and members.last is of attempt {} ({} to {})",
                    l.id, l.from, l.to
                )),
                None => refuse(format!(
                    "the inventory of the unfinished upgrade (attempt {id}, {from} to {to}) is lost: members.update is absent and there is no members.last"
                )),
            },
        },
        None => {
            if let Some(u) = update
                && !all_in(&u, &[State::Planned, State::Restored])
            {
                return refuse(format!(
                    "members.update of attempt {} has work in it but no upgrade is unfinished; nothing was changed",
                    u.id
                ));
            }
            match (read_last(home)?, left) {
                (Some(l), Some(left)) if l.to == left => Ok(Some(Inventory {
                    record: l,
                    source: Source::Last,
                })),
                (Some(l), _) => refuse(format!(
                    "members.last is of attempt {} ({} to {}), not of the release being left; nothing was changed",
                    l.id, l.from, l.to
                )),
                (None, _) => Ok(None),
            }
        }
    }
}

/// Put one entry back by what the record says it was; idempotent.
fn restore_entry(
    e: &Entry,
    dest: &Path,
    backup: &Path,
    id: &str,
    point: Point,
) -> Result<(), MemberError> {
    let staged = sibling(dest, &format!(".restore-{id}"));
    let dir = dest.parent().unwrap_or(Path::new("/"));
    let now = dest_now(dest)?;
    if is_old(&now, &e.old) {
        return Ok(());
    }
    match &e.old {
        Old::Absent => {
            // a file or a link the upgrade put there
            remove_if_there(dest)?;
            sync_dir(dir)?;
        }
        Old::File { sha256, mode } => {
            backup_is(backup, sha256, *mode).map_err(|why| {
                entry_fail(
                    &e.name,
                    format!(
                        "the backup {} {why}; this entry was not put back and every backup is kept",
                        backup.display()
                    ),
                )
            })?;
            link_or_copy(backup, &staged, *mode)?;
            point(&format!("{}:restore-staged", e.name));
            fs::rename(&staged, dest).map_err(|er| io("cannot put back", dest, &er))?;
            sync_dir(dir)?;
        }
        Old::Link { target_hex } => {
            let target = unhex(target_hex)
                .ok_or_else(|| entry_fail(&e.name, "the recorded link target is not readable"))?;
            remove_if_there(&staged)?;
            symlink(std::ffi::OsString::from_vec(target), &staged)
                .map_err(|er| io("cannot recreate the link", &staged, &er))?;
            fs::rename(&staged, dest).map_err(|er| io("cannot put back", dest, &er))?;
            sync_dir(dir)?;
        }
    }
    if !is_old(&dest_now(dest)?, &e.old) {
        return Err(entry_fail(
            &e.name,
            format!(
                "{} is not what it was after it was put back",
                dest.display()
            ),
        ));
    }
    point(&format!("{}:restore-renamed", e.name));
    Ok(())
}

/// Put every entry of the inventory back, by its state and hash (never a guess): a `planned` entry is only VERIFIED (its destination was never touched; any
/// backup there is not used), the others are put back through `<dest>.restore-<id>` renamed over the destination, verified, and recorded `restored` durably.
/// EVERY backup stays until EVERY entry is restored and verified; only then the backups of this attempt go. A failure at entry N leaves 1 to N-1 restored and
/// every backup in place; the next call resumes. Returns the names put back.
pub fn restore(
    home: &Path,
    unit_dir: &Path,
    inv: &Inventory,
    point: Point,
) -> Result<Vec<String>, MemberError> {
    // the destination is validated before any write, in a restore as in a plan and an install (cox W1-74 (2)): a refusal changes nothing and every backup stays
    check_home(home)?;
    check_unit_dir(unit_dir)?;
    let mut rec = inv.record.clone();
    let path = match inv.source {
        Source::Update => update_path(home),
        Source::Last => last_path(home),
    };
    let id = rec.id.clone();
    let mut done = Vec::new();
    for i in 0..rec.entries.len() {
        let e = rec.entries[i].clone();
        let dest = dest_of(e.kind, &e.name, home, unit_dir);
        remove_if_there(&sibling(&dest, ".new"))?;
        if e.state == State::Planned {
            // never touched: it must still be what the plan recorded, and nothing else is looked at
            if !is_old(&dest_now(&dest)?, &e.old) {
                return Err(entry_fail(
                    &e.name,
                    format!(
                        "{} is not what the plan recorded and the entry was never replaced; nothing was changed, every backup is kept",
                        dest.display()
                    ),
                ));
            }
        } else {
            restore_entry(&e, &dest, &backup_of(&dest, &id), &id, point)?;
        }
        rec.entries[i].state = State::Restored;
        persist(&path, &rec)?;
        point(&format!("{}:restored", e.name));
        done.push(e.name.clone());
    }
    point("restore:all-restored");
    for e in &rec.entries {
        let dest = dest_of(e.kind, &e.name, home, unit_dir);
        remove_if_there(&backup_of(&dest, &id))?;
        remove_if_there(&sibling(&dest, &format!(".restore-{id}")))?;
        sync_dir(dest.parent().unwrap_or(Path::new("/")))?;
    }
    point("restore:backups-removed");
    Ok(done)
}

/// Remove the record the inventory came from (after the marker was cleared); when `members.last` is the SAME attempt's copy of it (a promotion that stopped
/// after S2), that goes too.
pub fn forget(home: &Path, inv: &Inventory) -> Result<(), MemberError> {
    let path = match inv.source {
        Source::Update => update_path(home),
        Source::Last => last_path(home),
    };
    remove_if_there(&path)?;
    if inv.source == Source::Update
        && let Ok(Some(l)) = read_last(home)
        && l.id == inv.record.id
    {
        remove_if_there(&last_path(home))?;
    }
    sync_dir(home)
}

/// S1 to S3 of the end of an upgrade: `members.last.new` from `members.update` (every entry replaced), synced; its rename over `members.last` (the prior
/// completed record is replaced only here), the directory synced; the removal of `members.update`. Idempotent: with `members.update` absent and `members.last`
/// of this attempt there is nothing left to do; with both there and of the same attempt they must agree.
pub fn promote(
    home: &Path,
    from: &str,
    to: &str,
    id: &str,
    point: Point,
) -> Result<(), MemberError> {
    valid_pair(from, to, id)?;
    let last = read_last(home).ok().flatten();
    let Some(update) = read_update(home)? else {
        return match last {
            // S3 was done: the published record must be COMPLETE (every entry replaced), or it is not the record of a finished promotion (packet 19: nothing was changed)
            Some(l)
                if (l.from.as_str(), l.to.as_str(), l.id.as_str()) == (from, to, id)
                    && all_in(&l, &[State::Replaced]) =>
            {
                Ok(())
            }
            Some(l) if (l.from.as_str(), l.to.as_str(), l.id.as_str()) == (from, to, id) => refuse(
                "members.update is absent and members.last is of this attempt but not complete (an entry is not replaced): it is not a published record; nothing was changed",
            ),
            _ => refuse(
                "members.update is absent and members.last is not of this attempt: the inventory is lost; nothing was changed",
            ),
        };
    };
    if (update.from.as_str(), update.to.as_str(), update.id.as_str()) != (from, to, id) {
        return refuse(format!(
            "members.update is of attempt {} ({} to {}), not of attempt {id}; nothing was changed",
            update.id, update.from, update.to
        ));
    }
    if !all_in(&update, &[State::Replaced]) {
        let names: Vec<&str> = update
            .entries
            .iter()
            .filter(|e| e.state != State::Replaced)
            .map(|e| e.name.as_str())
            .collect();
        return refuse(format!(
            "not every member is installed yet ({}): the record is not promoted",
            names.join(", ")
        ));
    }
    if let Some(l) = &last
        && l.id == update.id
    {
        // `same_content` compares the entries' kinds, names and old states and NOT their states: the published record must also be COMPLETE (every entry replaced, as the
        // update is, or it is not what an S2 published) and must carry the same states, else the two do not agree and NEITHER is touched (packet 19, mike's probe)
        if !same_content(l, &update) || !all_in(l, &[State::Replaced]) {
            return refuse(
                "members.update and members.last are of the same attempt and do not agree (their entries or their states differ, or members.last is not complete); nothing was changed",
            );
        }
        // S1 and S2 were done by a call that was lost before S3: what it published (with its retired list, computed from the record it replaced) stays as it is
        remove_if_there(&update_path(home))?;
        sync_dir(home)?;
        point("promote:S3");
        return Ok(());
    }
    // W1-75: the previous record names nobody unless it is TRUSTED (complete and bound to the release this attempt left): then its id and the ids it had retired
    // are retired, the oldest falling off the end of RETIRED_MAX (an id nobody names is never swept)
    let retired = match &last {
        Some(l) if l.to == from && all_in(l, &[State::Replaced]) => {
            let mut v: Vec<String> = l.retired.clone();
            v.push(l.id.clone());
            v.retain(|x| *x != update.id);
            if v.len() > RETIRED_MAX {
                v.drain(..v.len() - RETIRED_MAX);
            }
            v
        }
        _ => Vec::new(),
    };
    let promoted = Record {
        retired,
        ..update.clone()
    };
    // S1: members.last.new, synced (the leftover of a lost S1 goes HERE, after every check has passed: a refusal above changed nothing)
    let new = home.join(LAST_NEW);
    remove_if_there(&new)?;
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&new)
        .map_err(|e| io("cannot create", &new, &e))?;
    f.write_all(serialize(&promoted).as_bytes())
        .and_then(|()| f.sync_all())
        .map_err(|e| io("cannot write", &new, &e))?;
    drop(f);
    sync_dir(home)?;
    point("promote:S1");
    // S2: the rename; the older completed record is replaced only here
    fs::rename(&new, last_path(home)).map_err(|e| io("cannot publish", &last_path(home), &e))?;
    sync_dir(home)?;
    point("promote:S2");
    // S3: the live record goes
    remove_if_there(&update_path(home))?;
    sync_dir(home)?;
    point("promote:S3");
    Ok(())
}

/// The most retired ids one `members.last` carries (W1-75): an id that falls off the end is simply never named again, so its backups are KEPT, never deleted.
pub const RETIRED_MAX: usize = 8;

/// What the sweep may rely on at the start of an upgrade or after a promotion (W1-74, W1-75).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trust {
    /// `members.last` is present, readable, valid, complete (every entry replaced) and bound to the release in question: these ids are retired.
    Trusted(Vec<String>),
    /// There is no `members.last`.
    Absent,
    /// There is one and it cannot be relied on: why.
    Untrusted(String),
}

/// The retired ids of a TRUSTED `members.last`: present, readable, valid, complete (every entry replaced) and bound to `release` (its `to`: the release being left
/// by an upgrade that starts, or the one just reached by a promotion). Anything else is not trusted and names nobody.
pub fn trusted_retired(home: &Path, release: &str) -> Trust {
    match read_last(home) {
        Ok(None) => Trust::Absent,
        Err(e) => Trust::Untrusted(e.to_string()),
        Ok(Some(l)) => {
            if l.to != release {
                return Trust::Untrusted(format!(
                    "it is the record of the upgrade to {}, not of the release {release}",
                    l.to
                ));
            }
            if !all_in(&l, &[State::Replaced]) {
                return Trust::Untrusted(
                    "it is not a completed record (an entry is not replaced)".to_string(),
                );
            }
            Trust::Trusted(l.retired)
        }
    }
}

/// The id of a recognised backup file name (`<one of the ten>.prev-<16 lowercase hex>`, nothing else), if it is one.
fn backup_id<'a>(file: &'a str, names: &[&str]) -> Option<&'a str> {
    names.iter().find_map(|n| {
        file.strip_prefix(&format!("{n}.prev-"))
            .filter(|id| valid_attempt_id(id))
    })
}

/// The file names of every recognised backup in the two member directories, sorted (regular files only).
pub fn recognised_backups(home: &Path, unit_dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    for (dir, names) in [
        (home.join("bin"), &BIN_FILES[..]),
        (unit_dir.to_path_buf(), &UNIT_FILES[..]),
    ] {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            let file = entry.file_name().to_string_lossy().to_string();
            if backup_id(&file, names).is_some() && entry.file_type().is_ok_and(|t| t.is_file()) {
                found.push(file);
            }
        }
    }
    found.sort();
    found
}

/// S5 and the start of an upgrade: remove every recognised backup (one of the ten names, `.prev-`, 16 lowercase hex characters, nothing else) whose id is in
/// `retired` and NOT in `protect`, in the two member directories. A backup that is not named is never touched, whatever else is true of it (W1-75: unknown means
/// keep). By the name rule alone; a name that is almost one is left alone. Idempotent. Returns the file names removed.
pub fn sweep_retired(
    home: &Path,
    unit_dir: &Path,
    retired: &[&str],
    protect: &[&str],
    point: Point,
) -> Result<Vec<String>, MemberError> {
    let mut removed = Vec::new();
    for (dir, names) in [
        (home.join("bin"), &BIN_FILES[..]),
        (unit_dir.to_path_buf(), &UNIT_FILES[..]),
    ] {
        let Ok(rd) = fs::read_dir(&dir) else { continue };
        for entry in rd.flatten() {
            let file = entry.file_name().to_string_lossy().to_string();
            let named = backup_id(&file, names)
                .is_some_and(|id| retired.contains(&id) && !protect.contains(&id));
            if named && entry.file_type().is_ok_and(|t| t.is_file()) {
                remove_if_there(&entry.path())?;
                point(&format!("sweep:removed:{file}"));
                removed.push(file);
            }
        }
        sync_dir(&dir)?;
    }
    removed.sort();
    Ok(removed)
}

/// A plan of an attempt that never switched (a stale marker): remove its `members.update` when every entry is still `planned` (nothing happened). `Ok(false)`
/// when there is none or it is of another attempt; a refusal when it has work in it.
pub fn discard_plan(home: &Path, id: &str) -> Result<bool, MemberError> {
    let Some(r) = read_update(home)? else {
        return Ok(false);
    };
    if r.id != id {
        return Ok(false);
    }
    if !all_in(&r, &[State::Planned]) {
        return refuse(format!(
            "members.update of attempt {id} has work in it (an entry past planned) though the upgrade never switched; nothing was changed"
        ));
    }
    remove_if_there(&update_path(home))?;
    sync_dir(home)?;
    Ok(true)
}

/// The lines of the upgrade record: `<kind> <name> <created|replaced>`, one per entry.
pub fn info_lines(r: &Record) -> Vec<String> {
    r.entries
        .iter()
        .map(|e| {
            format!(
                "{} {} {}",
                e.kind.word(),
                e.name,
                if e.old == Old::Absent {
                    "created"
                } else {
                    "replaced"
                }
            )
        })
        .collect()
}
