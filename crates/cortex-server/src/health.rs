//! The health document and the readiness decision (contract r0.3 section 2; S3 r2.6 `Health`; BK-08).
//!
//! `LAST_GOOD` is nix's one small JSON file in the dump directory, which the server reads READ-ONLY. This module reads exactly one
//! field of it, `utc` (an RFC 3339 UTC time ending in `Z`, whole seconds or 1 to 6 fraction digits); the other keys belong to piece 12
//! and are not read. The decisions (my reading of BK-08, recorded in the contract r0.4): no file = `none`; a file that cannot be
//! trusted = `stale` with `last_good_dump_at: null` (never `ok`); a time more than 5 minutes ahead of the clock is not believed (it
//! would hide staleness for ever); more than 26 h old = `stale`, exactly 26 h is still `ok`. The backup state is information only:
//! it never makes the server not ready.

use cortex_types::Timestamp;
use serde::Serialize;
use std::io::Read;
use std::path::Path;

/// More than this many seconds since the last good dump is `stale` (26 h, keep-list a).
pub const STALE_AFTER_SECS: i64 = 26 * 3600;
/// A `LAST_GOOD` file larger than this is unusable (it is one small JSON object).
pub const MAX_LAST_GOOD_BYTES: u64 = 4096;
/// A `utc` time this far ahead of the clock is treated as unusable (a far-future time would mask staleness for ever).
pub const FUTURE_SLACK_SECS: i64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Backup {
    Ok,
    Stale,
    None,
}

/// What the read-only `LAST_GOOD` file said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LastGood {
    /// No such file: no good set was ever recorded.
    Missing,
    /// A file that cannot be trusted (empty, not JSON, wrong shape, too large, not a regular file, unreadable).
    Unusable,
    At(Timestamp),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PieceHealth {
    pub name: String,
    pub enabled: bool,
    pub ready: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HealthDoc {
    pub state: &'static str,
    pub version: String,
    pub ready: bool,
    pub pieces: Vec<PieceHealth>,
    pub last_good_dump_at: Option<String>,
    pub backup: Backup,
}

pub struct HealthInput<'a> {
    pub version: &'a str,
    pub database_reachable: bool,
    pub migrations_current: bool,
    pub pieces: &'a [PieceHealth],
    pub last_good: &'a LastGood,
    pub now: Timestamp,
}

/// `YYYY-MM-DDTHH:MM:SS[.f{1,6}]Z` to unix microseconds. Strict: UTC `Z` only, the year 1970 to 9999, a real calendar date.
fn parse_utc(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    let digits = |from: usize, to: usize| -> Option<i64> {
        let part = b.get(from..to)?;
        if part.iter().all(u8::is_ascii_digit) {
            part.iter()
                .try_fold(0i64, |acc, d| Some(acc * 10 + i64::from(d - b'0')))
        } else {
            None
        }
    };
    let sep = |at: usize, c: u8| b.get(at) == Some(&c);
    if !(sep(4, b'-') && sep(7, b'-') && sep(10, b'T') && sep(13, b':') && sep(16, b':')) {
        return None;
    }
    let (year, month, day) = (digits(0, 4)?, digits(5, 7)?, digits(8, 10)?);
    let (hour, minute, second) = (digits(11, 13)?, digits(14, 16)?, digits(17, 19)?);
    let micros_frac = match b.get(19..)? {
        [b'Z'] => 0,
        [b'.', frac @ .., b'Z']
            if (1..=6).contains(&frac.len()) && frac.iter().all(u8::is_ascii_digit) =>
        {
            let mut v = frac
                .iter()
                .fold(0i64, |acc, d| acc * 10 + i64::from(d - b'0'));
            for _ in frac.len()..6 {
                v *= 10;
            }
            v
        }
        _ => return None,
    };
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    if !(1970..=9999).contains(&year)
        || !(1..=days_in_month).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    // Howard Hinnant's days_from_civil.
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 86_400 + hour * 3600 + minute * 60 + second) * 1_000_000) + micros_frac)
}

/// True when both describe the same file on the same device (the inode check that closes the lstat-then-open swap race, hale N-1).
pub fn same_file(checked: &std::fs::Metadata, opened: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    checked.dev() == opened.dev() && checked.ino() == opened.ino()
}

/// Parse the bytes of a `LAST_GOOD` file. Anything that is not a small JSON object with a valid `utc` string is `Unusable`.
pub fn parse_last_good(bytes: &[u8]) -> LastGood {
    if bytes.len() as u64 > MAX_LAST_GOOD_BYTES {
        return LastGood::Unusable;
    }
    let Ok(serde_json::Value::Object(o)) = serde_json::from_slice::<serde_json::Value>(bytes)
    else {
        return LastGood::Unusable;
    };
    match o
        .get("utc")
        .and_then(serde_json::Value::as_str)
        .and_then(parse_utc)
    {
        Some(micros) => LastGood::At(Timestamp::from_unix_micros(micros)),
        None => LastGood::Unusable,
    }
}

/// Read the file at `path` (read only). No such file (or directory) is `Missing`; a symlink, a directory, a file over the size
/// cap or any other read problem is `Unusable`. A symlink is never followed.
pub fn read_last_good(path: &Path) -> LastGood {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return LastGood::Missing,
        Err(_) => return LastGood::Unusable,
    };
    if !meta.file_type().is_file() || meta.len() > MAX_LAST_GOOD_BYTES {
        return LastGood::Unusable;
    }
    read_checked(path, &meta)
}

/// Open `path` and read it ONLY if the opened file is the one `checked` described (hale N-1: between the check and the open the path
/// could have been swapped for a symlink). Public so a test can hand it the metadata of a DIFFERENT file, which is what a swap looks like.
pub fn read_checked(path: &Path, checked: &std::fs::Metadata) -> LastGood {
    // O_NOFOLLOW: a symlink swapped in after the check is refused by the open itself (ELOOP). O_NONBLOCK: a FIFO swapped in would
    // otherwise block the open until a writer appears (vera SERVER-I2-V1); on a regular file it changes nothing.
    use std::os::unix::fs::OpenOptionsExt;
    let Ok(file) = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
    else {
        return LastGood::Unusable;
    };
    match file.metadata() {
        Ok(opened) if opened.file_type().is_file() && same_file(checked, &opened) => {}
        _ => return LastGood::Unusable,
    }
    let mut buf = Vec::new();
    if file
        .take(MAX_LAST_GOOD_BYTES + 1)
        .read_to_end(&mut buf)
        .is_err()
    {
        return LastGood::Unusable;
    }
    parse_last_good(&buf)
}

/// The time to report and the backup state, at `now`.
pub fn backup_state(last: &LastGood, now: Timestamp) -> (Option<Timestamp>, Backup) {
    match *last {
        LastGood::Missing => (None, Backup::None),
        LastGood::Unusable => (None, Backup::Stale),
        LastGood::At(t) => {
            let age_micros = now.unix_micros().saturating_sub(t.unix_micros());
            if age_micros < -(FUTURE_SLACK_SECS * 1_000_000) {
                (None, Backup::Stale)
            } else if age_micros > STALE_AFTER_SECS * 1_000_000 {
                (Some(t), Backup::Stale)
            } else {
                (Some(t), Backup::Ok)
            }
        }
    }
}

/// The open `GET /v2/health` document. `ready` = the database is reachable AND migrations are current AND every ENABLED piece is
/// ready (a disabled piece never blocks). No project data is in it (installation-level state only).
pub fn health_doc(i: &HealthInput<'_>) -> HealthDoc {
    let ready = i.database_reachable
        && i.migrations_current
        && i.pieces.iter().filter(|p| p.enabled).all(|p| p.ready);
    let (at, backup) = backup_state(i.last_good, i.now);
    HealthDoc {
        state: if ready { "ready" } else { "not_ready" },
        version: i.version.to_owned(),
        ready,
        pieces: i.pieces.to_vec(),
        last_good_dump_at: at.map(cortex_http::time::rfc3339),
        backup,
    }
}
