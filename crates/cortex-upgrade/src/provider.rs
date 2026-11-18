//! R-6 (cox W1-68, W1-91 Q1): the verified ABSOLUTE compose provider. `install.sh` records the path it verified in `<home>/compose-provider` (ONE line,
//! the absolute path, the user's own file, not writable by group or other); [`read_compose_provider`] reads it ONCE, at the process entry (the adapter is
//! built before the engine runs and the ports are frozen, so the entry hands the path to both), and refuses with ONE plain line, before any compose
//! call, anything that is not exactly that: the record (a regular file read through its open handle, no link followed, bounded, one absolute path
//! without `.`, `..` or empty components, no control character), the provider (a regular file opened the same way, owned by root or the user, not
//! writable by group or other, executable by its owner) and every directory above the provider and above the home (the stage's rule: root or the user,
//! no group or other write bit, on the canonical path). A refusal never echoes the record's bytes. The answer is the CANONICAL path of the provider
//! that was checked.

use crate::stage::{check_ancestors, my_uid};
use crate::text::shown;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::Read;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

/// The name of the record in the install's home.
pub const RECORD: &str = "compose-provider";

/// The most bytes the record may hold (a path is at most this long on Linux).
pub const RECORD_MAX_BYTES: usize = 4096;

/// THE shape the compose environment accepts for the provider path (R6A-01): absolute, and no byte below 0x20 or equal to 0x7f. ONE predicate, called by
/// [`read_compose_provider`] on the CANONICAL path it is about to return and by `ComposeEnv::vars` (`PODMAN_COMPOSE_PROVIDER`), so that the two cannot drift
/// and a path the reader returns is never one the environment refuses later, at `compose_up`, after the switch and the member steps.
pub(crate) fn compose_env_path_ok(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    path.is_absolute()
        && !path
            .as_os_str()
            .as_bytes()
            .iter()
            .any(|b| *b < 0x20 || *b == 0x7f)
}

// Linux values, as in `verify.rs`: a link in the last component is refused instead of followed, and opening a FIFO does not wait for a writer.
const O_NOFOLLOW: i32 = 0x20000;
const O_NONBLOCK: i32 = 0x800;

/// Opens `path` as what it must be: looked at first (a missing file says `missing`, anything but a regular file is refused without being opened, so a
/// FIFO never blocks), then opened without following a link and compared with what was looked at; returns the handle and ITS metadata.
fn open_regular(path: &Path, what: &str, missing: &str) -> Result<(File, Metadata), String> {
    let shown = shown(path);
    let md = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(missing.to_owned()),
        Err(_) => return Err(format!("{what} {shown} cannot be looked at")),
    };
    if !md.file_type().is_file() {
        return Err(format!(
            "{what} {shown} is not a regular file (a link, a directory or any other kind is not accepted); fix: run install.sh again"
        ));
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(O_NOFOLLOW | O_NONBLOCK)
        .open(path)
        .map_err(|_| format!("{what} {shown} cannot be opened"))?;
    let opened = file
        .metadata()
        .map_err(|_| format!("{what} {shown} cannot be inspected"))?;
    if !opened.is_file() || opened.dev() != md.dev() || opened.ino() != md.ino() {
        return Err(format!("{what} {shown} changed while it was checked"));
    }
    Ok((file, opened))
}

/// Reads and checks the compose provider recorded in `<home>/compose-provider`; see the module comment. `Ok` is the canonical absolute path of the
/// regular executable file that was checked.
pub fn read_compose_provider(home: &Path) -> Result<PathBuf, String> {
    let home = check_ancestors(home)?;
    let me = my_uid().ok_or_else(|| "the owner of this process cannot be read".to_owned())?;
    let record = home.join(RECORD);
    let bad = |why: &str| {
        format!(
            "the compose provider record {} must hold one absolute path: {why}; fix: run install.sh again",
            shown(&record)
        )
    };
    // the record: ONE read, through the open handle
    let missing = format!(
        "no compose provider is recorded: {} does not exist; fix: run install.sh again (it records the verified provider)",
        shown(&record)
    );
    let (file, md) = open_regular(&record, "the compose provider record", &missing)?;
    if md.uid() != me {
        return Err(format!(
            "the compose provider record {} belongs to another user; fix: remove it and run install.sh again",
            shown(&record)
        ));
    }
    if md.mode() & 0o022 != 0 {
        return Err(format!(
            "the compose provider record {} is writable by group or other; fix: chmod go-w {} (or run install.sh again)",
            shown(&record),
            shown(&record)
        ));
    }
    let mut bytes = Vec::new();
    file.take(RECORD_MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| {
            format!(
                "the compose provider record {} cannot be read",
                shown(&record)
            )
        })?;
    if bytes.len() > RECORD_MAX_BYTES {
        return Err(bad("it is longer than 4096 bytes"));
    }
    // exactly one line, with at most the one newline that ends it
    let body = bytes.strip_suffix(b"\n").unwrap_or(&bytes);
    if body.contains(&b'\n') {
        return Err(bad("it holds more than one line"));
    }
    if body.is_empty() {
        return Err(bad("it is empty"));
    }
    let text = std::str::from_utf8(body).map_err(|_| bad("it is not text"))?;
    if text.chars().any(char::is_control) {
        return Err(bad("it holds a control character or NUL"));
    }
    if !text.starts_with('/') {
        return Err(bad("it is not an absolute path"));
    }
    if text[1..].split('/').any(|c| matches!(c, "" | "." | "..")) {
        return Err(bad("it has an empty, . or .. component"));
    }
    // the provider: the directories above it first (canonical), then the file itself
    let given = PathBuf::from(text);
    let (Some(parent), Some(leaf)) = (given.parent(), given.file_name()) else {
        return Err(bad("it names no file"));
    };
    let path = check_ancestors(parent)?.join(leaf);
    // R6A-01 (vera 9 Nov, found by mike): the text check above cannot see a control byte in the name of a directory that a link in the record resolves to;
    // the CANONICAL path is what is returned and what reaches the compose environment, so it has the environment's own shape, here, before anything is
    // opened or returned. The line names the record and never echoes the path.
    if !compose_env_path_ok(&path) {
        return Err(format!(
            "the compose provider record {} names a provider whose canonical path holds a control character; fix: install the provider where no directory name holds one, then run install.sh again",
            shown(&record)
        ));
    }
    let missing = format!(
        "the compose provider {} does not exist; fix: run install.sh again",
        shown(&path)
    );
    let (_file, md) = open_regular(&path, "the compose provider", &missing)?;
    if md.uid() != 0 && md.uid() != me {
        return Err(format!(
            "the compose provider {} belongs to another user; fix: install the provider as root",
            shown(&path)
        ));
    }
    if md.mode() & 0o022 != 0 {
        return Err(format!(
            "the compose provider {} is writable by group or other; fix: chmod go-w {}",
            shown(&path),
            shown(&path)
        ));
    }
    if md.mode() & 0o100 == 0 {
        return Err(format!(
            "the compose provider {} is not executable; fix: chmod u+x {}",
            shown(&path),
            shown(&path)
        ));
    }
    Ok(path)
}
