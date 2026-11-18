//! R-6, stage C (cox W1-68, W1-91 Q1): `read_compose_provider(home)`: the verified ABSOLUTE compose provider recorded by `install.sh` in
//! `<home>/compose-provider` (one line, mode 0644, the user's own). It is read ONCE (by the entry point, which hands the path to the adapter and
//! to the engine's `Env`), and anything but a regular executable file whose every parent directory belongs to root or the user and is not
//! writable by group or other is ONE plain refusal, before any compose call. DB-free; no container, no `podman`, no network.
mod common;

use common::Tmp;
use cortex_upgrade::provider::{RECORD, RECORD_MAX_BYTES, read_compose_provider};
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn put(p: &Path, bytes: &[u8], mode: u32) {
    // replaced, not overwritten: the file a loop made before may have a mode that forbids writing
    let _ = fs::remove_file(p);
    fs::write(p, bytes).unwrap();
    fs::set_permissions(p, fs::Permissions::from_mode(mode)).unwrap();
}

fn dir(p: &Path, mode: u32) -> PathBuf {
    fs::create_dir_all(p).unwrap();
    fs::set_permissions(p, fs::Permissions::from_mode(mode)).unwrap();
    p.to_path_buf()
}

fn mkfifo(p: &Path) {
    assert!(
        std::process::Command::new("mkfifo")
            .arg(p)
            .status()
            .unwrap()
            .success()
    );
}

/// A fresh private home and, beside it, a good provider (a script, 0755) in a good directory; returns (tmp, home, provider).
fn rig(tag: &str) -> (Tmp, PathBuf, PathBuf) {
    let t = Tmp::new(tag);
    let home = dir(&t.path().join("home"), 0o700);
    let bin = dir(&t.path().join("bin"), 0o755);
    let provider = bin.join("podman-compose");
    put(&provider, b"#!/bin/sh\nexit 0\n", 0o755);
    (t, home, provider)
}

fn record(home: &Path, text: &[u8]) {
    put(&home.join(RECORD), text, 0o644);
}

fn text_of(p: &Path) -> Vec<u8> {
    format!("{}\n", p.display()).into_bytes()
}

fn refusal(r: Result<PathBuf, String>) -> String {
    match r {
        Ok(p) => panic!("expected a refusal, got {}", p.display()),
        Err(e) => e,
    }
}

fn no_control(m: &str) -> bool {
    !m.chars().any(char::is_control)
}

#[test]
fn pv_01_a_recorded_regular_executable_in_good_directories_is_returned_as_its_canonical_path() {
    let (t, home, provider) = rig("pv01");
    // one trailing newline, as install.sh writes it
    record(&home, &text_of(&provider));
    let got = read_compose_provider(&home).expect("a valid record");
    assert_eq!(
        got,
        fs::canonicalize(t.path().join("bin"))
            .unwrap()
            .join("podman-compose")
    );
    assert!(got.is_absolute());
    // and without the newline
    record(&home, provider.as_os_str().to_str().unwrap().as_bytes());
    assert_eq!(read_compose_provider(&home).unwrap(), got);
    // the mode of the record: private or group readable are fine
    for mode in [0o600, 0o640, 0o644] {
        put(&home.join(RECORD), &text_of(&provider), mode);
        assert_eq!(read_compose_provider(&home).unwrap(), got, "{mode:o}");
    }
}

#[test]
fn pv_02_a_record_that_is_not_exactly_one_absolute_path_is_refused_and_never_echoed() {
    let (_t, home, provider) = rig("pv02");
    let p = provider.display().to_string();
    let big = vec![b'a'; RECORD_MAX_BYTES + 1];
    let cases: Vec<(&str, Vec<u8>, &str)> = vec![
        ("empty", b"".to_vec(), "is empty"),
        ("a newline only", b"\n".to_vec(), "is empty"),
        (
            "two lines",
            format!("{p}\n{p}\n").into_bytes(),
            "more than one line",
        ),
        (
            "a second empty line",
            format!("{p}\n\n").into_bytes(),
            "more than one line",
        ),
        (
            "a relative path",
            b"podman-compose\n".to_vec(),
            "not an absolute path",
        ),
        (
            "a bare name",
            b"docker-compose".to_vec(),
            "not an absolute path",
        ),
        (
            "a .. component",
            b"/usr/../bin/podman-compose\n".to_vec(),
            "component",
        ),
        (
            "a . component",
            b"/usr/./bin/podman-compose\n".to_vec(),
            "component",
        ),
        (
            "an empty component",
            b"/usr//bin/podman-compose\n".to_vec(),
            "component",
        ),
        ("a trailing slash", b"/usr/bin/\n".to_vec(), "component"),
        ("the root alone", b"/\n".to_vec(), "component"),
        ("a NUL", b"/usr/bin/a\0b\n".to_vec(), "control character"),
        ("a CR", b"/usr/bin/a\rb\n".to_vec(), "control character"),
        ("a tab", b"/usr/bin/a\tb\n".to_vec(), "control character"),
        (
            "an escape sequence",
            b"/usr/bin/\x1b[31mx\n".to_vec(),
            "control character",
        ),
        (
            "bytes that are not text",
            b"/usr/bin/\xff\xfe\n".to_vec(),
            "not text",
        ),
        ("more than the limit", big, "longer than 4096 bytes"),
    ];
    for (what, bytes, phrase) in cases {
        record(&home, &bytes);
        let m = refusal(read_compose_provider(&home));
        assert!(m.contains(phrase), "{what}: {m}");
        assert!(m.contains(RECORD), "{what}: the line names the record: {m}");
        assert!(m.contains("fix:"), "{what}: and says the fix: {m}");
        assert!(
            no_control(&m),
            "{what}: a refusal never echoes the record's bytes: {m:?}"
        );
    }
    // exactly the limit is not refused for its length (it fails later, as a name that cannot exist)
    let mut exact = vec![b'a'; RECORD_MAX_BYTES];
    exact[0] = b'/';
    record(&home, &exact);
    let m = refusal(read_compose_provider(&home));
    assert!(
        !m.contains("longer than"),
        "exactly {RECORD_MAX_BYTES} bytes is within the limit: {m}"
    );
    // a record far over the limit is refused without being read whole (a bounded read)
    record(&home, &vec![b'a'; 10 * 1024 * 1024]);
    let m = refusal(read_compose_provider(&home));
    assert!(m.contains("longer than 4096 bytes"), "{m}");
}

#[test]
fn pv_03_the_record_must_be_a_regular_file_of_the_user_that_group_and_other_cannot_write() {
    let (t, home, provider) = rig("pv03");
    // missing
    let m = refusal(read_compose_provider(&home));
    assert!(m.contains("no compose provider is recorded"), "{m}");
    assert!(m.contains(RECORD) && m.contains("fix:"), "{m}");
    // a directory at the name
    fs::create_dir(home.join(RECORD)).unwrap();
    let m = refusal(read_compose_provider(&home));
    assert!(m.contains("not a regular file"), "{m}");
    fs::remove_dir(home.join(RECORD)).unwrap();
    // a link to a valid record is not followed
    let real = t.path().join("elsewhere");
    put(&real, &text_of(&provider), 0o644);
    symlink(&real, home.join(RECORD)).unwrap();
    let m = refusal(read_compose_provider(&home));
    assert!(m.contains("not a regular file"), "{m}");
    fs::remove_file(home.join(RECORD)).unwrap();
    // a FIFO is refused promptly, not waited for
    mkfifo(&home.join(RECORD));
    let start = Instant::now();
    let m = refusal(read_compose_provider(&home));
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "a FIFO must not block"
    );
    assert!(m.contains("not a regular file"), "{m}");
    fs::remove_file(home.join(RECORD)).unwrap();
    // writable by group or other
    for mode in [0o664, 0o646, 0o666, 0o620, 0o602] {
        put(&home.join(RECORD), &text_of(&provider), mode);
        let m = refusal(read_compose_provider(&home));
        assert!(m.contains("writable by group or other"), "{mode:o}: {m}");
    }
}

#[test]
fn pv_04_the_provider_must_be_a_regular_executable_file_that_group_and_other_cannot_write() {
    let (t, home, provider) = rig("pv04");
    let bin = provider.parent().unwrap().to_path_buf();
    let point = |p: &Path| record(&home, &text_of(p));
    // missing
    point(&bin.join("nothing"));
    let m = refusal(read_compose_provider(&home));
    assert!(m.contains("does not exist"), "{m}");
    // a directory
    dir(&bin.join("adir"), 0o755);
    point(&bin.join("adir"));
    let m = refusal(read_compose_provider(&home));
    assert!(m.contains("not a regular file"), "{m}");
    // a link, even to a good executable: not followed
    symlink(&provider, bin.join("alink")).unwrap();
    point(&bin.join("alink"));
    let m = refusal(read_compose_provider(&home));
    assert!(m.contains("not a regular file"), "{m}");
    // a FIFO: refused promptly
    mkfifo(&bin.join("afifo"));
    point(&bin.join("afifo"));
    let start = Instant::now();
    let m = refusal(read_compose_provider(&home));
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "a FIFO must not block"
    );
    assert!(m.contains("not a regular file"), "{m}");
    // not executable for the owner
    for mode in [0o644, 0o600, 0o444, 0o640, 0o611, 0o610, 0o601] {
        put(&bin.join("plain"), b"#!/bin/sh\n", mode);
        point(&bin.join("plain"));
        let m = refusal(read_compose_provider(&home));
        assert!(m.contains("not executable"), "{mode:o}: {m}");
    }
    // a provider its owner cannot even read cannot be opened to be checked: refused too, in one plain line
    for mode in [0o055, 0o044, 0o000] {
        put(&bin.join("unreadable"), b"#!/bin/sh\n", mode);
        point(&bin.join("unreadable"));
        let m = refusal(read_compose_provider(&home));
        assert!(
            m.contains("cannot be opened") && no_control(&m),
            "{mode:o}: {m}"
        );
    }
    // writable by group or other
    for mode in [0o775, 0o757, 0o777, 0o722] {
        put(&bin.join("loose"), b"#!/bin/sh\n", mode);
        point(&bin.join("loose"));
        let m = refusal(read_compose_provider(&home));
        assert!(m.contains("writable by group or other"), "{mode:o}: {m}");
    }
    // owner-executable modes are accepted
    for mode in [0o755, 0o700, 0o500, 0o555] {
        put(&bin.join("good"), b"#!/bin/sh\n", mode);
        point(&bin.join("good"));
        assert!(read_compose_provider(&home).is_ok(), "{mode:o}");
    }
    drop(t);
}

#[test]
fn pv_05_every_parent_directory_of_the_provider_must_belong_to_root_or_the_user_and_not_be_group_or_other_writable()
 {
    let (t, home, _provider) = rig("pv05");
    for (mode, what) in [
        (0o775, "group"),
        (0o757, "other"),
        (0o777, "group and other"),
    ] {
        let parent = dir(&t.path().join(format!("p-{mode:o}")), 0o755);
        let inner = dir(&parent.join("inner"), 0o755);
        let provider = inner.join("podman-compose");
        put(&provider, b"#!/bin/sh\n", 0o755);
        record(&home, &text_of(&provider));
        assert!(read_compose_provider(&home).is_ok(), "{what}: good before");
        // the DIRECT parent
        fs::set_permissions(&inner, fs::Permissions::from_mode(mode)).unwrap();
        let canonical = fs::canonicalize(&inner).unwrap();
        let m = refusal(read_compose_provider(&home));
        assert!(
            m.contains(canonical.to_str().unwrap()),
            "{what}: names the directory: {m}"
        );
        assert!(m.contains("chmod"), "{what}: and the fix: {m}");
        fs::set_permissions(&inner, fs::Permissions::from_mode(0o755)).unwrap();
        // a directory ABOVE it
        fs::set_permissions(&parent, fs::Permissions::from_mode(mode)).unwrap();
        let canonical = fs::canonicalize(&parent).unwrap();
        let m = refusal(read_compose_provider(&home));
        assert!(
            m.contains(canonical.to_str().unwrap()),
            "{what}: a grandparent: {m}"
        );
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(read_compose_provider(&home).is_ok(), "{what}: good after");
    }
    // a parent that does not exist is a refusal that names it
    record(
        &home,
        &text_of(&t.path().join("no-such-dir").join("podman-compose")),
    );
    let m = refusal(read_compose_provider(&home));
    assert!(m.contains("no-such-dir"), "{m}");
}

#[test]
fn pv_06_the_home_itself_must_be_trustworthy_or_the_record_is_not_read() {
    let (t, home, provider) = rig("pv06");
    record(&home, &text_of(&provider));
    assert!(read_compose_provider(&home).is_ok());
    // an ancestor of the home writable by group or other
    let outer = dir(&t.path().join("outer"), 0o755);
    let inner_home = dir(&outer.join("home"), 0o700);
    record(&inner_home, &text_of(&provider));
    assert!(read_compose_provider(&inner_home).is_ok());
    fs::set_permissions(&outer, fs::Permissions::from_mode(0o775)).unwrap();
    let canonical = fs::canonicalize(&outer).unwrap();
    let m = refusal(read_compose_provider(&inner_home));
    assert!(
        m.contains(canonical.to_str().unwrap()) && m.contains("chmod"),
        "{m}"
    );
    fs::set_permissions(&outer, fs::Permissions::from_mode(0o755)).unwrap();
    // a home that does not exist
    let m = refusal(read_compose_provider(&t.path().join("no-home")));
    assert!(m.contains("no-home"), "{m}");
}

#[test]
fn pv_07_the_answer_is_the_file_that_was_checked_not_a_path_that_names_it_through_a_link() {
    let (t, home, provider) = rig("pv07");
    // the provider is named through a link in the middle of its path: the answer is the canonical path of the real directory
    let link = t.path().join("bin-link");
    symlink(provider.parent().unwrap(), &link).unwrap();
    record(&home, &text_of(&link.join("podman-compose")));
    let got = read_compose_provider(&home).expect("a link in a parent is resolved, not refused");
    assert_eq!(
        fs::canonicalize(&got).unwrap(),
        got,
        "no link is left in the answer"
    );
    assert!(got.starts_with(fs::canonicalize(t.path()).unwrap()));
    assert_eq!(got.file_name().unwrap(), "podman-compose");
    assert!(
        fs::symlink_metadata(&got).unwrap().file_type().is_file(),
        "and it is the regular file"
    );
}

#[test]
fn pv_08_every_refusal_is_one_plain_line() {
    let (_t, home, provider) = rig("pv08");
    let mut seen = Vec::new();
    seen.push(refusal(read_compose_provider(&home))); // no record
    for bytes in [
        b"".to_vec(),
        b"x".to_vec(),
        b"/a/../b".to_vec(),
        b"/a\0".to_vec(),
    ] {
        record(&home, &bytes);
        seen.push(refusal(read_compose_provider(&home)));
    }
    record(&home, &text_of(&provider.parent().unwrap().join("missing")));
    seen.push(refusal(read_compose_provider(&home)));
    put(&provider, b"#!/bin/sh\n", 0o666);
    record(&home, &text_of(&provider));
    seen.push(refusal(read_compose_provider(&home)));
    for m in &seen {
        assert!(
            !m.is_empty() && !m.contains('\n') && no_control(m),
            "one plain line: {m:?}"
        );
        assert!(m.len() < 600, "short: {m}");
    }
    assert_eq!(seen.len(), 7);
}

#[test]
fn pv_09_a_root_owned_system_executable_is_accepted_the_real_case() {
    use std::os::unix::fs::MetadataExt;
    // the provider of a real install is a root-owned regular file in a root-owned directory (an RPM): any such file does for the cell
    let Some(root_owned) = ["/usr/bin/env", "/bin/env", "/usr/bin/true", "/bin/true"]
        .iter()
        .map(Path::new)
        .find(|p| {
            fs::symlink_metadata(p)
                .map(|m| m.file_type().is_file() && m.uid() == 0 && m.mode() & 0o022 == 0)
                .unwrap_or(false)
        })
    else {
        eprintln!(
            "pv_09: no root-owned regular system executable on this host; the cell has nothing to try"
        );
        return;
    };
    let (_t, home, _provider) = rig("pv09");
    record(&home, &text_of(root_owned));
    let got =
        read_compose_provider(&home).expect("a root-owned executable in a root-owned directory");
    assert_eq!(got.file_name(), root_owned.file_name());
    assert!(got.is_absolute());
}

/// The names in `dir` (sorted): a refusal changes nothing in the home.
fn names_in(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

/// R6A-01 (vera 9 Nov, found by mike): the TEXT of the record is checked for control characters, but the CANONICAL path the reader returns (the parent
/// resolved through every link, then the leaf) was not: a plain-looking record through a directory alias whose real name holds a control byte returned a
/// path that the engine's absolute-path guard accepts and that `ComposeEnv::vars` refuses only at `compose_up`, after the switch and the member steps. The
/// reader must refuse it itself, with the compose environment's own shape (a byte below 0x20 or 0x7f), before it returns, and change nothing.
#[test]
fn pv_10_a_provider_whose_canonical_path_holds_a_control_byte_is_refused_before_it_is_returned() {
    for byte in [0x01u8, 0x09, 0x0a, 0x1f, 0x7f] {
        let (t, home, _good) = rig(&format!("pv10-{byte:02x}"));
        let real = dir(&t.path().join(format!("ctl{}dir", byte as char)), 0o755);
        put(&real.join("podman-compose"), b"#!/bin/sh\nexit 0\n", 0o755);
        let alias = t.path().join("plain-alias");
        symlink(&real, &alias).unwrap();
        record(&home, &text_of(&alias.join("podman-compose")));
        let written = fs::read(home.join(RECORD)).unwrap();
        assert!(
            no_control(
                std::str::from_utf8(&written)
                    .unwrap()
                    .trim_end_matches('\n')
            ),
            "{byte:#04x}: the record's own text is plain"
        );
        let before = names_in(&home);
        let m = refusal(read_compose_provider(&home));
        assert!(
            m.contains("canonical path holds a control character"),
            "{byte:#04x}: {m}"
        );
        assert!(
            m.contains(RECORD),
            "{byte:#04x}: the line names the record: {m}"
        );
        assert!(m.contains("fix:"), "{byte:#04x}: and says the fix: {m}");
        assert!(
            no_control(&m),
            "{byte:#04x}: a refusal never echoes the bytes: {m:?}"
        );
        assert_eq!(
            names_in(&home),
            before,
            "{byte:#04x}: nothing changed in the home"
        );
        assert_eq!(
            fs::read(home.join(RECORD)).unwrap(),
            written,
            "{byte:#04x}: nor the record"
        );
    }
}

/// The control of pv_10: the SAME alias to a directory whose name holds characters the compose environment accepts (a space, quotes, a dash, a UTF-8 letter)
/// still reads (the name also holds U+0085, a C1 control the environment's byte test does not refuse), and answers the canonical path: the check refuses ONLY
/// the compose environment's shape (a byte below 0x20 or 0x7f), nothing more.
#[test]
fn pv_11_a_canonical_path_with_ordinary_odd_characters_is_still_read() {
    let (t, home, _good) = rig("pv11");
    let real = dir(&t.path().join("bin dir 'q' \"d\" -x \u{e9}\u{85}"), 0o755);
    put(&real.join("podman-compose"), b"#!/bin/sh\nexit 0\n", 0o755);
    let alias = t.path().join("plain-alias");
    symlink(&real, &alias).unwrap();
    record(&home, &text_of(&alias.join("podman-compose")));
    let got = read_compose_provider(&home).expect("a name without a control byte is not refused");
    assert_eq!(got, fs::canonicalize(&real).unwrap().join("podman-compose"));
}

/// R6A-01, mike's second finding (kai's row f3654d2b): every refusal of the reader interpolates the CANONICAL path of the record (the home resolved through its
/// links), so a plain CORTEX_HOME alias whose real directory name holds a control byte put that byte, RAW, into the operator's terminal. A message shows a path
/// with every control character escaped (a `\u{..}` form), never raw, and still names the record. The characters: a C0 control, a newline, ESC, DEL and a C1 control
/// (U+009B, the one-character CSI).
const ODD: [char; 5] = ['\u{1}', '\n', '\u{1b}', '\u{7f}', '\u{9b}'];

/// A plain alias to a directory whose real name holds `c`: (tmp, the alias, the real directory).
fn odd_home(tag: &str, c: char) -> (Tmp, PathBuf, PathBuf) {
    let t = Tmp::new(tag);
    let real = dir(&t.path().join(format!("odd{c}home")), 0o700);
    let alias = t.path().join("plain-home");
    symlink(&real, &alias).unwrap();
    (t, alias, real)
}

fn escaped(c: char) -> String {
    c.escape_debug().to_string()
}

#[test]
fn pv_12_a_home_alias_to_a_directory_with_a_control_character_never_puts_it_raw_into_a_refusal() {
    for c in ODD {
        // no record at all: the reader's first refusal names the canonical record
        let (_t, alias, _real) = odd_home("pv12", c);
        let m = refusal(read_compose_provider(&alias));
        assert!(
            no_control(&m),
            "{c:?}: a refusal never carries a raw control character: {m:?}"
        );
        assert!(
            m.contains(RECORD),
            "{c:?}: and it still names the record: {m}"
        );
        assert!(
            m.contains(&escaped(c)),
            "{c:?}: the character is SHOWN, escaped as {}: {m}",
            escaped(c)
        );
    }
}

#[test]
fn pv_13_the_canonical_path_refusal_itself_never_carries_the_control_character_of_the_home() {
    // mike's exact case: the home alias resolves to a control-byte directory AND the provider is named through an alias whose real name holds one: the new
    // refusal (R6A-01) interpolated the canonical record path and so the home's byte
    for c in ODD {
        let (t, alias, real) = odd_home("pv13", c);
        // the provider's own directory holds 0x01 (a byte the environment refuses, so the new refusal fires); the HOME holds `c`, any of the five
        let bin = dir(&t.path().join("bin\u{1}dir"), 0o755);
        put(&bin.join("podman-compose"), b"#!/bin/sh\nexit 0\n", 0o755);
        let link = t.path().join("plain-bin");
        symlink(&bin, &link).unwrap();
        record(&real, &text_of(&link.join("podman-compose")));
        let m = refusal(read_compose_provider(&alias));
        assert!(
            m.contains("canonical path holds a control character"),
            "{c:?}: {m}"
        );
        assert!(
            no_control(&m),
            "{c:?}: no raw control character in the refusal: {m:?}"
        );
        assert!(
            m.contains(&escaped(c)),
            "{c:?}: the home's character is shown escaped: {m}"
        );
    }
}

#[test]
fn pv_14_a_directory_refusal_of_the_ancestor_rule_shows_its_control_character_escaped() {
    // the ancestor rule's own lines (check_ancestors) name the directory: a group-writable directory whose real name holds a control character
    for c in ODD {
        let (t, home, _good) = rig(&format!("pv14-{}", c as u32));
        let bin = dir(&t.path().join(format!("bin{c}dir")), 0o775);
        put(&bin.join("podman-compose"), b"#!/bin/sh\nexit 0\n", 0o755);
        let link = t.path().join("plain-bin");
        symlink(&bin, &link).unwrap();
        record(&home, &text_of(&link.join("podman-compose")));
        let m = refusal(read_compose_provider(&home));
        assert!(m.contains("writable by group or other"), "{c:?}: {m}");
        assert!(
            no_control(&m),
            "{c:?}: no raw control character in the refusal: {m:?}"
        );
        assert!(
            m.contains(&escaped(c)),
            "{c:?}: the directory's character is shown escaped: {m}"
        );
    }
}

/// The control of pv_12 (cox's row 1aa8ba41: "and its control"): the SAME home alias to an ordinary directory, whose name holds characters that are printable but
/// that an escape routine could mangle (a space, quotes, a backslash, a UTF-8 letter): the refusal shows the canonical record path EXACTLY as before, unescaped.
#[test]
fn pv_15_a_home_alias_to_an_ordinary_directory_shows_the_canonical_record_path_unchanged() {
    let t = Tmp::new("pv15");
    let real = dir(&t.path().join("real home 'q' \"d\" \\b \u{e9}"), 0o700);
    let alias = t.path().join("plain-home");
    symlink(&real, &alias).unwrap();
    let m = refusal(read_compose_provider(&alias));
    let want = fs::canonicalize(&real).unwrap().join(RECORD);
    assert!(
        m.contains(&want.display().to_string()),
        "the canonical record path is shown exactly: {m}"
    );
    assert!(no_control(&m), "{m:?}");
}
