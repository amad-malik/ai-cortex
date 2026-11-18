//! The private stage of the roles step (R-4a; cox W1-65, W1-66, W1-67, W1-91): an engine-created directory that holds the VERIFIED bytes of
//! `db-roles.sh` and `compose.yml` (from [`crate::verify::verify_release_dir_bytes`]: read once, hashed, handed over: nothing is read again) and
//! is what `run_roles` is given as its release directory. `<home>/.roles-stage-<release>`: the directory exactly 0700, the script 0500, `compose.yml`
//! 0400 (the adapter reads it to find the project), all created exclusively and checked as they are on disk; a stale stage is replaced only when it
//! is a real directory of the user's own with mode 0700 (the install's rule for its own stage), anything else at that path stops the build with a
//! plain cause; every directory from `/` to the home, on the CANONICAL path, belongs to root or the user and has no group or other write bit.
//! The stage removes itself when it is dropped (one attempt, a failure is swallowed): on success, on failure and on refusal.

use crate::text::shown;
use crate::unpack::create_private_dir_exact;
use crate::verify::ReleaseBytes;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

const PREFIX: &str = ".roles-stage-";

/// The stage; removed (its files and the directory) when it is dropped.
#[derive(Debug)]
pub struct Stage {
    dir: PathBuf,
}

impl Stage {
    /// The directory to give `run_roles` as its release directory.
    pub fn path(&self) -> &Path {
        &self.dir
    }
}

impl Drop for Stage {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// The effective user: the owner of `/proc/self` (as `members.rs` does; the portable form is M-1d's).
pub(crate) fn my_uid() -> Option<u32> {
    fs::metadata("/proc/self").ok().map(|m| m.uid())
}

/// A release name that can be part of a directory name: a version (`[0-9A-Za-z.+-]`, 1 to 64 bytes, not starting with a dot).
fn release_ok(release: &str) -> bool {
    (1..=64).contains(&release.len())
        && !release.starts_with('.')
        && release
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+'))
}

/// Every directory from `/` to `path`, on the CANONICAL path (links resolved first, so `/home` being a link to `/var/home` is fine): each is a
/// real directory owned by root or the effective user with NO group or other write bit. Returns the canonical path. One plain line, naming the
/// directory and the fix, on the first that fails.
pub fn check_ancestors(path: &Path) -> Result<PathBuf, String> {
    let canonical = fs::canonicalize(path).map_err(|_| {
        format!(
            "the directory {} cannot be resolved; fix: make sure it exists",
            shown(path)
        )
    })?;
    let me = my_uid().ok_or_else(|| "the owner of this process cannot be read".to_owned())?;
    for dir in canonical.ancestors() {
        let md = fs::symlink_metadata(dir)
            .map_err(|_| format!("the directory {} cannot be looked at", shown(dir)))?;
        if !md.file_type().is_dir() {
            return Err(format!(
                "{} is not a directory; fix: it must be a real directory",
                shown(dir)
            ));
        }
        if md.uid() != 0 && md.uid() != me {
            return Err(format!(
                "the directory {} belongs to another user; fix: chown it to root or to you",
                shown(dir)
            ));
        }
        if md.mode() & 0o022 != 0 {
            return Err(format!(
                "the directory {} is writable by group or other; fix: chmod go-w {}",
                shown(dir),
                shown(dir)
            ));
        }
    }
    Ok(canonical)
}

/// Create `path` exclusively with `data`, then set EXACTLY `mode` and check what is on disk: a regular file of this user with that mode.
fn put(path: &Path, data: &[u8], mode: u32, me: u32) -> Result<(), String> {
    let cannot = |why: &str| format!("cannot make {}: {why}", shown(path));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(path)
        .map_err(|e| cannot(&e.to_string()))?;
    file.write_all(data).map_err(|e| cannot(&e.to_string()))?;
    drop(file);
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|e| cannot(&e.to_string()))?;
    let md = fs::symlink_metadata(path).map_err(|e| cannot(&e.to_string()))?;
    if !md.file_type().is_file() || md.uid() != me || md.mode() & 0o7777 != mode {
        return Err(cannot(
            "it is not a regular file of yours with the mode it was given",
        ));
    }
    Ok(())
}

/// Builds `<home>/.roles-stage-<release>` from the verified bytes: see the module comment. The home is checked first (`check_ancestors`), the
/// stage lives under the CANONICAL home. Everything made is removed again when the build fails.
pub fn build_stage(home: &Path, release: &str, bytes: &ReleaseBytes) -> Result<Stage, String> {
    if !release_ok(release) {
        return Err("the stage cannot be built: the release name is not a version".to_owned());
    }
    let home = check_ancestors(home)?;
    let me = my_uid().ok_or_else(|| "the owner of this process cannot be read".to_owned())?;
    let dir = home.join(format!("{PREFIX}{release}"));
    match fs::symlink_metadata(&dir) {
        Ok(md) => {
            if md.file_type().is_dir() && md.uid() == me && md.mode() & 0o7777 == 0o700 {
                fs::remove_dir_all(&dir).map_err(|_| {
                    format!(
                        "the earlier stage {} cannot be removed; fix: remove it by hand",
                        shown(&dir)
                    )
                })?;
            } else {
                return Err(format!(
                    "{} exists and is not a real directory of yours with mode 0700, so the stage will not replace it; fix: remove it by hand",
                    shown(&dir)
                ));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return Err(format!("{} cannot be looked at", shown(&dir))),
    }
    create_private_dir_exact(&dir)
        .map_err(|e| format!("cannot make the stage directory {}: {e}", shown(&dir)))?;
    // from here on the guard removes whatever was made, on any error below
    let stage = Stage { dir: dir.clone() };
    put(&dir.join("db-roles.sh"), &bytes.roles_script, 0o500, me)?;
    put(&dir.join("compose.yml"), &bytes.compose_yml, 0o400, me)?;
    let md = fs::symlink_metadata(&dir).map_err(|_| "the stage cannot be looked at".to_owned())?;
    if !md.file_type().is_dir() || md.uid() != me || md.mode() & 0o7777 != 0o700 {
        return Err(
            "the stage directory is not a real directory of yours with mode 0700".to_owned(),
        );
    }
    Ok(stage)
}
