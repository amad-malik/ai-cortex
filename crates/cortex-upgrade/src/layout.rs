//! The release layout of one `$CORTEX_HOME` (design §2 and §5): the symlinks, the lock, the record,
//! the prune. Only `std`. Nothing here runs a process or touches the network.

use crate::manifest::{self, Entry, MANIFEST_MAX_BYTES, MigrationClass};
use crate::stack_env::{STACK_ENV_MAX_BYTES, StackEnv, StackEnvError, parse_stack_env};
use crate::version::{Version, parse_version};
use cortex_types::{Clock, Timestamp};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, symlink};
use std::path::{Path, PathBuf};

/// Where a rollback (`flip_back_with`) can be interrupted (a test hook).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlipStep {
    IntentWritten,
    PreviousSet,
    CurrentSet,
    /// Only for a marked rollback ([`Home::begin_rollback_with`]): the marker is written, the intent not yet removed.
    MarkerWritten,
}

/// Where a `commit` can be interrupted (a test hook: the real caller never stops early).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitStep {
    DirRenamed,
    PreviousSet,
    CurrentSet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RollbackRefusal {
    #[error("there is no previous release")]
    NoPrevious,
    #[error("the previous release is the current one (an interrupted switch)")]
    PreviousIsCurrent,
    #[error("the release being left has migration_class contract")]
    ContractRelease,
    #[error("the manifest copy of the release being left cannot be read or does not list it")]
    ManifestUnreadable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LayoutError {
    #[error("layout: {what}: {kind}")]
    Io {
        what: &'static str,
        kind: std::io::ErrorKind,
    },
    #[error("layout: `{link}` does not point at a release directory of this home")]
    BadLink { link: &'static str },
    #[error("layout: there is no staged release for that version")]
    StagingMissing,
    #[error("layout: that release directory already exists next to a staged one")]
    ReleaseExists,
    #[error("layout: there is no current release")]
    NoCurrent,
    #[error("layout: rollback refused: {0}")]
    RollbackRefused(RollbackRefusal),
    #[error("layout: interrupted after {0:?}")]
    Interrupted(CommitStep),
    #[error("layout: rollback interrupted after {0:?}")]
    FlipInterrupted(FlipStep),
    /// An I/O error AFTER the rollback intent was published: the way back is written down, the links may be half way, and the
    /// intent is kept. (An I/O error before that is a plain `Io`: nothing was changed.)
    #[error("layout: the rollback was started and stopped: {what}: {kind}")]
    FlipFailed {
        what: &'static str,
        kind: std::io::ErrorKind,
    },
    #[error("layout: a rollback was interrupted and must be recovered first")]
    RecoveryPending,
    #[error("layout: the rollback intent file is not valid")]
    BadIntent,
    #[error(
        "layout: upgrade.incomplete is not a marker this engine wrote, or does not fit the links"
    )]
    BadIncomplete,
    #[error("layout: {0}")]
    StackEnv(#[from] crate::stack_env::StackEnvError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LockError {
    #[error("another upgrade is running (upgrade.lock is held)")]
    Busy,
    #[error("upgrade.lock could not be opened: {0}")]
    Io(std::io::ErrorKind),
}

/// Held while one upgrade runs; the kernel drops it with the process, so a crash never leaves it held.
#[derive(Debug)]
pub struct UpgradeLock {
    _file: std::fs::File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordResult {
    Done,
    RolledBack,
    RollbackFailed,
    /// The run stopped and the old release kept running; nothing needed rolling back.
    Failed,
    /// The module-roles step and its one confirmation both timed out (cox W1-49 B): whether the roles were applied is UNKNOWN, and nothing was
    /// rolled back; the install stays on the new release for the operator.
    RolesUnknown,
    /// A RESUMED upgrade (a marker stood) failed again at step 9 or 10: nothing was rolled back, the marker stays, the operator decides (cox W1-56 B).
    ResumeFailed,
}

/// What the `previous` link was when the upgrade began (mike's F2 on packet 15 r1): the half-done flip of a lost process overwrites it, and the stale
/// marker is the only record of what to put back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrevLink {
    /// There was no `previous` link.
    Absent,
    /// It pointed at this release.
    Release(Version),
}

impl std::fmt::Display for PrevLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PrevLink::Absent => f.write_str("none"),
            PrevLink::Release(v) => write!(f, "{v}"),
        }
    }
}

/// An upgrade that was started (the marker is written AHEAD of the switch of step 8) and did not finish (steps 8 to 11): `<home>/upgrade.incomplete`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Incomplete {
    pub from: Version,
    pub to: Version,
    /// The pre-upgrade dump set the upgrade made.
    pub set: String,
    /// The `previous` link before the upgrade.
    pub prev: PrevLink,
    /// One plain sentence: why it did not finish.
    pub why: String,
    /// The id of THIS attempt (W1-62): 16 lowercase hex characters, new for every attempt, written ahead of the switch with the rest; the member records and
    /// the backup names carry the same id.
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepRecord {
    pub step: u8,
    pub name: String,
    pub ok: bool,
}

/// One attempt, written to `upgrades/<UTC stamp>.json`. Typed fields only: no token, no server text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub from: String,
    pub to: String,
    pub result: RecordResult,
    pub steps: Vec<StepRecord>,
    pub dump_set: Option<String>,
    /// The backup scripts and units this attempt changed (R-5), one `<kind> <name> <created|replaced>` line each; absent when it changed none. A later
    /// `--rollback` of this upgrade puts them back from their `.prev` copies.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<String>,
}

/// What a rollback did: the release it left and the one it went back to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Flip {
    pub left: Version,
    pub now: Version,
}

#[derive(Debug, Clone)]
pub struct Home {
    root: PathBuf,
}

/// An I/O error that happens after a rollback was started is `FlipFailed`, not a plain `Io` (mike, packet 8).
fn started(e: LayoutError) -> LayoutError {
    match e {
        LayoutError::Io { what, kind } => LayoutError::FlipFailed { what, kind },
        other => other,
    }
}

fn io_err(what: &'static str, e: std::io::Error) -> LayoutError {
    LayoutError::Io {
        what,
        kind: e.kind(),
    }
}

/// A directory, not a link to one.
fn is_real_dir(p: &Path) -> bool {
    fs::symlink_metadata(p).map(|m| m.is_dir()).unwrap_or(false)
}

fn sync_dir(p: &Path) -> Result<(), LayoutError> {
    fs::File::open(p)
        .and_then(|f| f.sync_all())
        .map_err(|e| io_err("sync directory", e))
}

/// UTC `YYYYMMDDTHHMMSSZ` of a timestamp (civil-from-days, proleptic Gregorian).
fn stamp(t: Timestamp) -> String {
    let secs = t.unix_micros().div_euclid(1_000_000);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}{m:02}{d:02}T{:02}{:02}{:02}Z",
        rem / 3_600,
        rem % 3_600 / 60,
        rem % 60
    )
}

impl Home {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn release_dir(&self, v: &Version) -> PathBuf {
        self.root.join("releases").join(v.to_string())
    }
    pub fn staging_dir(&self, v: &Version) -> PathBuf {
        self.root.join("releases").join(format!("{v}.part"))
    }

    /// Where the link `name` points, as a version; `None` when there is no such link. A link that is
    /// anything but exactly `releases/<version>` is refused, never followed.
    fn link_version(&self, name: &'static str) -> Result<Option<Version>, LayoutError> {
        match fs::read_link(self.root.join(name)) {
            Ok(target) => target
                .to_str()
                .and_then(|t| t.strip_prefix("releases/"))
                .and_then(|rest| parse_version(rest).ok())
                .map(Some)
                .ok_or(LayoutError::BadLink { link: name }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) if e.kind() == std::io::ErrorKind::InvalidInput => {
                Err(LayoutError::BadLink { link: name })
            }
            Err(e) => Err(io_err("read link", e)),
        }
    }
    pub fn current(&self) -> Result<Option<Version>, LayoutError> {
        self.link_version("current")
    }
    pub fn previous(&self) -> Result<Option<Version>, LayoutError> {
        self.link_version("previous")
    }

    /// Point `name` at `releases/<v>` in one atomic step: a new link under a temporary name, then
    /// `rename(2)` over the old one. There is no moment when `name` is missing or dangling.
    fn set_link(&self, name: &'static str, v: &Version) -> Result<(), LayoutError> {
        let tmp = self.root.join(format!(".{name}.tmp{}", std::process::id()));
        match fs::remove_file(&tmp) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_err("remove stale link", e)),
        }
        symlink(format!("releases/{v}"), &tmp).map_err(|e| io_err("create link", e))?;
        fs::rename(&tmp, self.root.join(name)).map_err(|e| io_err("replace link", e))
    }

    /// Forget the `previous` link. After a rollback it names the release that was LEFT, and a second rollback would flip
    /// forward to it again, so a completed rollback ends here (the abandoned release directory stays on disk for
    /// inspection until a later upgrade prunes it). No link is fine. Refused, with nothing changed, while a rollback
    /// intent is pending, and when `previous` is not exactly a link `releases/<version>` (a directory or an odd link
    /// there is not ours to remove).
    pub fn drop_previous(&self) -> Result<(), LayoutError> {
        self.refuse_if_pending()?;
        if self.previous()?.is_none() {
            return Ok(());
        }
        fs::remove_file(self.root.join("previous")).map_err(|e| io_err("remove link", e))?;
        sync_dir(&self.root)
    }

    /// Take `upgrade.lock` (`flock`) without waiting: one upgrade at a time, and a second one is told so.
    pub fn lock(&self) -> Result<UpgradeLock, LockError> {
        let path = self.root.join("upgrade.lock");
        if fs::symlink_metadata(&path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            return Err(LockError::Io(std::io::ErrorKind::InvalidInput));
        }
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| LockError::Io(e.kind()))?;
        match file.try_lock() {
            Ok(()) => Ok(UpgradeLock { _file: file }),
            Err(fs::TryLockError::WouldBlock) => Err(LockError::Busy),
            Err(fs::TryLockError::Error(e)) => Err(LockError::Io(e.kind())),
        }
    }

    pub fn commit(&self, new: &Version) -> Result<(), LayoutError> {
        self.commit_with(new, &mut |_| true)
    }

    /// Switch to `new` (step 8): the staged directory becomes the release, `previous` is set to the
    /// release that was current, then `current` to the new one: `previous` first, so a stop between the
    /// two leaves `current` on a working release. Safe to call again after any stop. `after` is called
    /// after each of the three effects; returning `false` stops there with `Interrupted` and leaves the
    /// tree exactly as it is (a test hook; the real caller always continues).
    pub fn commit_with(
        &self,
        new: &Version,
        after: &mut dyn FnMut(CommitStep) -> bool,
    ) -> Result<(), LayoutError> {
        self.refuse_if_pending()?;
        let old = self.current()?;
        let (staging, target) = (self.staging_dir(new), self.release_dir(new));
        let (staged, released) = (is_real_dir(&staging), is_real_dir(&target));
        if old.as_ref() == Some(new) {
            return if staged {
                Err(LayoutError::ReleaseExists)
            } else {
                Ok(())
            };
        }
        match (staged, released) {
            (true, true) => return Err(LayoutError::ReleaseExists),
            (false, false) => return Err(LayoutError::StagingMissing),
            (true, false) => {
                fs::rename(&staging, &target).map_err(|e| io_err("rename staging", e))?;
                sync_dir(&self.root.join("releases"))?;
            }
            (false, true) => {} // resuming after a stop
        }
        if !after(CommitStep::DirRenamed) {
            return Err(LayoutError::Interrupted(CommitStep::DirRenamed));
        }
        if let Some(old) = &old {
            self.set_link("previous", old)?;
        }
        if !after(CommitStep::PreviousSet) {
            return Err(LayoutError::Interrupted(CommitStep::PreviousSet));
        }
        self.set_link("current", new)?;
        sync_dir(&self.root)?;
        if !after(CommitStep::CurrentSet) {
            return Err(LayoutError::Interrupted(CommitStep::CurrentSet));
        }
        Ok(())
    }

    /// The release entry that the manifest copy inside release `v` lists for `v` itself (what step 5 wrote there), for a retry that finishes an
    /// interrupted upgrade. `None` when the copy cannot be read, does not parse, or does not list `v` as a usable release.
    pub fn read_release(&self, v: &Version) -> Option<manifest::Release> {
        let mut body = Vec::new();
        fs::File::open(self.release_dir(v).join("manifest.json"))
            .ok()?
            .take(MANIFEST_MAX_BYTES as u64 + 1)
            .read_to_end(&mut body)
            .ok()?;
        manifest::parse(&body)
            .ok()?
            .entries
            .into_iter()
            .find_map(|e| match e {
                Entry::Usable(r) if r.version == *v => Some(*r),
                _ => None,
            })
    }

    /// The migration class the manifest copy inside release `v` gives to `v` itself.
    fn class_of(&self, v: &Version) -> Option<MigrationClass> {
        let mut body = Vec::new();
        fs::File::open(self.release_dir(v).join("manifest.json"))
            .ok()?
            .take(MANIFEST_MAX_BYTES as u64 + 1)
            .read_to_end(&mut body)
            .ok()?;
        manifest::parse(&body)
            .ok()?
            .entries
            .into_iter()
            .find_map(|e| match e {
                Entry::Usable(r) if r.version == *v => Some(r.cortex.migration_class),
                _ => None,
            })
    }

    /// Rollback (design §5): `current` and `previous` change places, `previous` first. Refused, with
    /// nothing changed, when there is no previous release, when the previous one is the current one,
    /// when the manifest copy of the release being left cannot be read or does not list it, or when
    /// that release has `migration_class = contract`.
    pub fn flip_back(&self) -> Result<Flip, LayoutError> {
        self.flip_back_with(&mut |_| true)
    }

    /// The rollback with a durable record of the way back. `previous` is replaced first and `current` second, and between
    /// the two renames both links name the release being left; so BEFORE the first rename the pair `(left, back)` is written
    /// to `rollback.intent` (private file, written to a temporary name, synced, renamed, directory synced). If anything
    /// stops after that, [`Home::recover`] finishes the job from the file, and until it has, `commit`, `prune` and another
    /// rollback refuse (`RecoveryPending`), so the release the links no longer name is not deleted. (mike, packet 2.)
    /// `after` is called after each of the three effects; `false` stops there (a test hook; the real caller continues).
    pub fn flip_back_with(
        &self,
        after: &mut dyn FnMut(FlipStep) -> bool,
    ) -> Result<Flip, LayoutError> {
        self.flip_with(false, after)
    }

    /// The rollback the engine uses: the same flip, and it ends with a durable marker, `rollback.unfinished`, which stays until
    /// the old release's server has been started and checked and [`Home::finish_rollback`] has forgotten `previous` (mike,
    /// packet 7). While the marker stands, `commit`, `prune`, `drop_previous` and every other flip refuse (`RecoveryPending`), so a
    /// crash or a failed start cannot leave a pair of links that a later `--rollback` would flip FORWARD. The intent file says
    /// `tail=1`, so [`Home::recover`] writes the marker too when this flip is interrupted.
    pub fn begin_rollback(&self) -> Result<Flip, LayoutError> {
        self.begin_rollback_with(&mut |_| true)
    }

    /// [`Home::begin_rollback`] with the test hook of [`Home::flip_back_with`] (and one more step, `MarkerWritten`).
    pub fn begin_rollback_with(
        &self,
        after: &mut dyn FnMut(FlipStep) -> bool,
    ) -> Result<Flip, LayoutError> {
        self.flip_with(true, after)
    }

    fn flip_with(
        &self,
        mark: bool,
        after: &mut dyn FnMut(FlipStep) -> bool,
    ) -> Result<Flip, LayoutError> {
        self.refuse_if_pending()?;
        let cur = self.current()?.ok_or(LayoutError::NoCurrent)?;
        let prev = self
            .previous()?
            .ok_or(LayoutError::RollbackRefused(RollbackRefusal::NoPrevious))?;
        if prev == cur {
            return Err(LayoutError::RollbackRefused(
                RollbackRefusal::PreviousIsCurrent,
            ));
        }
        match self.class_of(&cur) {
            None => {
                return Err(LayoutError::RollbackRefused(
                    RollbackRefusal::ManifestUnreadable,
                ));
            }
            Some(MigrationClass::Contract) => {
                return Err(LayoutError::RollbackRefused(
                    RollbackRefusal::ContractRelease,
                ));
            }
            Some(MigrationClass::Expand) => {}
        }
        if !is_real_dir(&self.release_dir(&prev)) {
            return Err(LayoutError::BadLink { link: "previous" });
        }
        self.write_pair(&self.intent_path(), ".rollback.intent", &cur, &prev, mark)?;
        if !after(FlipStep::IntentWritten) {
            return Err(LayoutError::FlipInterrupted(FlipStep::IntentWritten));
        }
        self.set_link("previous", &cur).map_err(started)?;
        if !after(FlipStep::PreviousSet) {
            return Err(LayoutError::FlipInterrupted(FlipStep::PreviousSet));
        }
        self.set_link("current", &prev).map_err(started)?;
        sync_dir(&self.root).map_err(started)?;
        if !after(FlipStep::CurrentSet) {
            return Err(LayoutError::FlipInterrupted(FlipStep::CurrentSet));
        }
        if mark {
            self.write_pair(
                &self.marker_path(),
                ".rollback.unfinished",
                &cur,
                &prev,
                false,
            )
            .map_err(started)?;
            if !after(FlipStep::MarkerWritten) {
                return Err(LayoutError::FlipInterrupted(FlipStep::MarkerWritten));
            }
        }
        self.remove_intent().map_err(started)?;
        Ok(Flip {
            left: cur,
            now: prev,
        })
    }

    /// Put the `previous` link back as it was before a commit that failed half way (`was` is what [`Home::previous`] said before): `None`
    /// removes the link, `Some(v)` points it at `releases/<v>` when that is a real directory. Nothing to do when it is as it was. Refused,
    /// with nothing changed, while a rollback is pending, and when `previous` is not a plain release link (not ours to touch).
    pub fn restore_previous(&self, was: Option<&Version>) -> Result<(), LayoutError> {
        self.refuse_if_pending()?;
        match was {
            None => self.drop_previous(),
            Some(v) => {
                if !is_real_dir(&self.release_dir(v)) {
                    return Err(LayoutError::BadLink { link: "previous" });
                }
                if self.previous()?.as_ref() == Some(v) {
                    return Ok(());
                }
                self.set_link("previous", v)?;
                sync_dir(&self.root)
            }
        }
    }

    /// The marker file of an upgrade that did not finish.
    fn incomplete_path(&self) -> PathBuf {
        self.root.join("upgrade.incomplete")
    }

    /// Write the marker of an upgrade that is about to switch (step 8) and has not finished (steps 8 to 11), durably: a temporary file, fsync, rename,
    /// directory sync (like `rollback.intent`). Overwrites an older marker. `set` and `why` are one plain line each.
    pub fn mark_incomplete(&self, i: &Incomplete) -> Result<(), LayoutError> {
        for v in [&i.set, &i.why] {
            if v.is_empty() || v.chars().any(char::is_control) {
                return Err(LayoutError::BadIncomplete);
            }
        }
        if !valid_attempt_id(&i.id) {
            return Err(LayoutError::BadIncomplete);
        }
        let path = self.incomplete_path();
        let tmp = self
            .root
            .join(format!("upgrade.incomplete.tmp{}", std::process::id()));
        match fs::remove_file(&tmp) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_err("remove stale marker", e)),
        }
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(|e| io_err("create marker", e))?;
        let text = format!(
            "from={}\nto={}\nset={}\nprev={}\nwhy={}\nid={}\n",
            i.from, i.to, i.set, i.prev, i.why, i.id
        );
        f.write_all(text.as_bytes())
            .and_then(|()| f.sync_all())
            .map_err(|e| io_err("write marker", e))?;
        drop(f);
        fs::rename(&tmp, &path).map_err(|e| io_err("publish marker", e))?;
        sync_dir(&self.root)
    }

    /// The marker of an unfinished upgrade. `Ok(None)` when there is none. Exactly six lines, `from=`, `to=`, `set=`, `prev=`, `why=`, `id=`, in that order,
    /// each ended by a newline, nothing else, at most 4 KiB; two different SemVer versions; `to` must be the CURRENT release and `from` the
    /// PREVIOUS one (the links of an upgrade that was switched); `current` still the `from` release: the marker is stale, `Ok(None)` (the file is left
    /// alone; the next upgrade overwrites it). Anything else is `BadIncomplete` and the file is left alone.
    pub fn incomplete(&self) -> Result<Option<Incomplete>, LayoutError> {
        Ok(self
            .incomplete_state()?
            .and_then(|(marker, stale)| (!stale).then_some(marker)))
    }

    /// The marker and whether it is stale; everything `incomplete` says about the file.
    pub fn incomplete_state(&self) -> Result<Option<(Incomplete, bool)>, LayoutError> {
        let mut text = String::new();
        match fs::File::open(self.incomplete_path()) {
            Ok(f) => {
                f.take(4097)
                    .read_to_string(&mut text)
                    .map_err(|_| LayoutError::BadIncomplete)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(io_err("open marker", e)),
        }
        if text.len() > 4096 || !text.ends_with('\n') {
            return Err(LayoutError::BadIncomplete);
        }
        let lines: Vec<&str> = text[..text.len() - 1].split('\n').collect();
        let key = |line: Option<&&str>, k: &str| -> Option<String> {
            line.and_then(|l| l.strip_prefix(k)).map(str::to_string)
        };
        let (Some(from), Some(to), Some(set), Some(prev), Some(why), Some(id), 6) = (
            key(lines.first(), "from="),
            key(lines.get(1), "to="),
            key(lines.get(2), "set="),
            key(lines.get(3), "prev="),
            key(lines.get(4), "why="),
            key(lines.get(5), "id="),
            lines.len(),
        ) else {
            return Err(LayoutError::BadIncomplete);
        };
        if !valid_attempt_id(&id) {
            return Err(LayoutError::BadIncomplete);
        }
        let (Ok(from), Ok(to)) = (parse_version(&from), parse_version(&to)) else {
            return Err(LayoutError::BadIncomplete);
        };
        let prev = match prev.as_str() {
            "none" => PrevLink::Absent,
            v => match parse_version(v) {
                Ok(v) => PrevLink::Release(v),
                Err(_) => return Err(LayoutError::BadIncomplete),
            },
        };
        let one_line = |v: &str| !v.is_empty() && !v.chars().any(char::is_control);
        if from == to || !one_line(&set) || !one_line(&why) {
            return Err(LayoutError::BadIncomplete);
        }
        // the marker is written AHEAD of the switch. `current` is still `from`: the switch never happened (the run was lost between the marker and
        // the flip, or the switch failed and was undone): the marker is STALE and there is no unfinished upgrade
        let current = self.current()?;
        if current.as_ref() == Some(&from) {
            return Ok(Some((
                Incomplete {
                    from,
                    to,
                    set,
                    prev,
                    why,
                    id,
                },
                true,
            )));
        }
        // otherwise it must fit the links of a switched upgrade: current is `to`, previous is `from`
        if current.as_ref() != Some(&to) || self.previous()?.as_ref() != Some(&from) {
            return Err(LayoutError::BadIncomplete);
        }
        Ok(Some((
            Incomplete {
                from,
                to,
                set,
                prev,
                why,
                id,
            },
            false,
        )))
    }

    /// Remove the marker (durably). Nothing to remove is not an error.
    pub fn clear_incomplete(&self) -> Result<(), LayoutError> {
        match fs::remove_file(self.incomplete_path()) {
            Ok(()) => sync_dir(&self.root),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(io_err("remove marker", e)),
        }
    }

    /// Whether a rollback is unfinished: its intent (the flip was interrupted) or its marker (the old release was not confirmed running).
    pub fn has_unfinished_rollback(&self) -> bool {
        self.rollback_pending()
    }

    fn intent_path(&self) -> PathBuf {
        self.root.join("rollback.intent")
    }

    fn marker_path(&self) -> PathBuf {
        self.root.join("rollback.unfinished")
    }

    /// Whether a rollback intent file or an unfinished-rollback marker is there (valid or not).
    fn rollback_pending(&self) -> bool {
        fs::symlink_metadata(self.intent_path()).is_ok()
            || fs::symlink_metadata(self.marker_path()).is_ok()
    }

    fn refuse_if_pending(&self) -> Result<(), LayoutError> {
        if self.rollback_pending() {
            Err(LayoutError::RecoveryPending)
        } else {
            Ok(())
        }
    }

    /// Write `left=<v>`, `back=<v>` (and `tail=1` when `tail`) to `path` through a private temporary file: synced, renamed,
    /// directory synced. `tmp_stem` names the temporary file.
    fn write_pair(
        &self,
        path: &Path,
        tmp_stem: &str,
        left: &Version,
        back: &Version,
        tail: bool,
    ) -> Result<(), LayoutError> {
        let tmp = self
            .root
            .join(format!("{tmp_stem}.tmp{}", std::process::id()));
        match fs::remove_file(&tmp) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_err("remove stale intent", e)),
        }
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(|e| io_err("create intent", e))?;
        let text = format!(
            "left={left}\nback={back}\n{}",
            if tail { "tail=1\n" } else { "" }
        );
        f.write_all(text.as_bytes())
            .and_then(|()| f.sync_all())
            .map_err(|e| io_err("write intent", e))?;
        drop(f);
        fs::rename(&tmp, path).map_err(|e| io_err("publish intent", e))?;
        // the file is published now: an error from here on is "started" (the file stays)
        sync_dir(&self.root).map_err(started)
    }

    /// The unlink must succeed (a failure leaves the intent, which is what `FlipFailed` promises). The directory sync after it is best
    /// effort: the intent is gone from the running system, and if a crash brings the file back, `recover` redoes the same links (it is
    /// idempotent), so a failed sync is not a failure of the rollback and cannot honestly be reported as one. This boundary cannot be
    /// forced on a real file system and has no test (mike, packet 9).
    fn remove_intent(&self) -> Result<(), LayoutError> {
        fs::remove_file(self.intent_path()).map_err(|e| io_err("remove intent", e))?;
        let _ = sync_dir(&self.root);
        Ok(())
    }

    /// The pair in `path`: exactly `left=<version>` and `back=<version>` (and, when `allow_tail`, a last line `tail=1`), two
    /// different versions whose release directories exist, in a file of at most 512 bytes. Anything else is `BadIntent` and the
    /// file is left alone. The flag says whether `tail=1` was there.
    fn read_pair(
        &self,
        path: &Path,
        allow_tail: bool,
    ) -> Result<Option<(Version, Version, bool)>, LayoutError> {
        let mut text = String::new();
        match fs::File::open(path) {
            Ok(f) => {
                f.take(513)
                    .read_to_string(&mut text)
                    .map_err(|_| LayoutError::BadIntent)?;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(io_err("open intent", e)),
        }
        let mut lines = text.split('\n');
        let (l1, l2, l3, l4) = (lines.next(), lines.next(), lines.next(), lines.next());
        let version = |line: Option<&str>, key: &str| {
            line.and_then(|l| l.strip_prefix(key))
                .and_then(|v| parse_version(v).ok())
        };
        let tail = match (l3, l4) {
            (Some(""), None) => false,
            (Some("tail=1"), Some("")) if allow_tail => true,
            _ => return Err(LayoutError::BadIntent),
        };
        let (Some(left), Some(back)) = (version(l1, "left="), version(l2, "back=")) else {
            return Err(LayoutError::BadIntent);
        };
        if lines.next().is_some()
            || text.len() > 512
            || left == back
            || !is_real_dir(&self.release_dir(&left))
            || !is_real_dir(&self.release_dir(&back))
        {
            return Err(LayoutError::BadIntent);
        }
        Ok(Some((left, back, tail)))
    }

    fn read_intent(&self) -> Result<Option<(Version, Version, bool)>, LayoutError> {
        self.read_pair(&self.intent_path(), true)
    }

    /// The marker of a rollback that was started and not finished: the release that was left and the one rolled back to. The
    /// links must agree with it (`current` is the one rolled back to); anything else is `BadIntent`, never guessed.
    fn read_marker(&self) -> Result<Option<(Version, Version)>, LayoutError> {
        let Some((left, back, _)) = self.read_pair(&self.marker_path(), false)? else {
            return Ok(None);
        };
        if self.current()?.as_ref() != Some(&back) {
            return Err(LayoutError::BadIntent);
        }
        Ok(Some((left, back)))
    }

    /// The rollback that was started (its links are flipped) and not finished: `Ok(None)` when there is none. A marker that
    /// cannot be used is `BadIntent`.
    pub fn unfinished_rollback(&self) -> Result<Option<Flip>, LayoutError> {
        Ok(self.read_marker()?.map(|(left, now)| Flip { left, now }))
    }

    /// The last step of a rollback, after the old release's server was started and checked: forget `previous` (it names the
    /// release that was left, and another rollback would flip forward to it), then the marker. Safe to repeat; nothing when
    /// no rollback is unfinished. Refused while a flip is still pending (`RecoveryPending`), when the marker is not usable
    /// (`BadIntent`), and when `previous` is not exactly a link to the release that was left (`BadLink`): then NOTHING is
    /// changed and the marker stays, so everything keeps refusing.
    pub fn finish_rollback(&self) -> Result<(), LayoutError> {
        if fs::symlink_metadata(self.intent_path()).is_ok() {
            return Err(LayoutError::RecoveryPending);
        }
        let Some((left, _)) = self.read_marker()? else {
            return Ok(());
        };
        match self.previous()? {
            None => {}
            Some(p) if p == left => {
                fs::remove_file(self.root.join("previous"))
                    .map_err(|e| io_err("remove link", e))?;
                sync_dir(&self.root)?;
            }
            Some(_) => return Err(LayoutError::BadLink { link: "previous" }),
        }
        fs::remove_file(self.marker_path()).map_err(|e| io_err("remove marker", e))?;
        sync_dir(&self.root)?;
        // a finished rollback is also the way out of an unfinished upgrade: its marker goes
        self.clear_incomplete()
    }

    /// Finish a rollback that was interrupted: `previous` is the release that was left, `current` the one rolled back to,
    /// then the intent file is removed. Safe to call again after any stop. `Ok(None)` when no rollback is pending (and
    /// nothing is written).
    pub fn recover(&self) -> Result<Option<Flip>, LayoutError> {
        let Some((left, back, tail)) = self.read_intent()? else {
            return Ok(None);
        };
        self.set_link("previous", &left).map_err(started)?;
        self.set_link("current", &back).map_err(started)?;
        sync_dir(&self.root).map_err(started)?;
        if tail {
            self.write_pair(
                &self.marker_path(),
                ".rollback.unfinished",
                &left,
                &back,
                false,
            )
            .map_err(started)?;
        }
        self.remove_intent().map_err(started)?;
        Ok(Some(Flip { left, now: back }))
    }

    /// Remove every release directory that is neither `current` nor `previous`. Only real directories
    /// whose name is a version are considered: staging directories (`*.part`), links, files and anything
    /// else in `releases/` are left alone, and nothing outside `releases/` is ever touched. Returns what
    /// it removed.
    pub fn prune(&self) -> Result<Vec<Version>, LayoutError> {
        self.refuse_if_pending()?;
        let keep: Vec<Version> = [self.current()?, self.previous()?]
            .into_iter()
            .flatten()
            .collect();
        let dir = self.root.join("releases");
        let mut removed = Vec::new();
        for entry in fs::read_dir(&dir).map_err(|e| io_err("list releases", e))? {
            let entry = entry.map_err(|e| io_err("list releases", e))?;
            let Some(version) = entry
                .file_name()
                .to_str()
                .and_then(|n| parse_version(n).ok())
            else {
                continue;
            };
            if keep.contains(&version) || !is_real_dir(&entry.path()) {
                continue;
            }
            fs::remove_dir_all(entry.path()).map_err(|e| io_err("remove release", e))?;
            removed.push(version);
        }
        removed.sort();
        Ok(removed)
    }

    /// Write `upgrades/<UTC stamp>.json` (0600), never over an existing file: a second record in the
    /// same second is `<stamp>-2.json`, and so on.
    pub fn write_record(&self, clock: &dyn Clock, record: &Record) -> Result<PathBuf, LayoutError> {
        let dir = self.root.join("upgrades");
        fs::create_dir_all(&dir).map_err(|e| io_err("create upgrades", e))?;
        let body = serde_json::to_vec_pretty(record).map_err(|_| LayoutError::Io {
            what: "encode record",
            kind: std::io::ErrorKind::InvalidData,
        })?;
        let stamp = stamp(clock.now());
        for n in 1..=99 {
            let name = if n == 1 {
                format!("{stamp}.json")
            } else {
                format!("{stamp}-{n}.json")
            };
            let path = dir.join(name);
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
            {
                Ok(mut f) => {
                    f.write_all(&body)
                        .and_then(|()| f.sync_all())
                        .map_err(|e| io_err("write record", e))?;
                    return Ok(path);
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(io_err("create record", e)),
            }
        }
        Err(LayoutError::Io {
            what: "too many records in one second",
            kind: std::io::ErrorKind::AlreadyExists,
        })
    }

    /// The `stack.env` of release `v`; its `CORTEX_VERSION` must be `v`.
    pub fn read_stack_env(&self, v: &Version) -> Result<StackEnv, LayoutError> {
        let mut text = String::new();
        fs::File::open(self.release_dir(v).join("stack.env"))
            .map_err(|e| io_err("open stack.env", e))?
            .take(STACK_ENV_MAX_BYTES as u64 + 1)
            .read_to_string(&mut text)
            .map_err(|e| io_err("read stack.env", e))?;
        let env = parse_stack_env(&text)?;
        if env.version != *v {
            return Err(StackEnvError::Invalid {
                key: "CORTEX_VERSION",
            }
            .into());
        }
        Ok(env)
    }
}

/// The id of an upgrade attempt (cox W1-62): 16 lowercase hex characters.
pub fn valid_attempt_id(id: &str) -> bool {
    id.len() == 16
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// A NEW id for an attempt (W1-62): 8 bytes of the operating system's random source, as 16 lowercase hex characters. Uniqueness is the requirement, not secrecy;
/// a source that cannot be read stops the attempt before anything is written.
pub fn new_attempt_id() -> Result<String, LayoutError> {
    let mut bytes = [0u8; 8];
    fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|e| io_err("read the random source for the attempt id", e))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
