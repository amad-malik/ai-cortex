//! R-5 (cox's rulings W1-59, W1-60, W1-61, W1-62): the ten member files that live outside the release directory (`<home>/bin` and the user units) are installed by
//! a PER-ENTRY RECORD (`members.update`, promoted to `members.last` at the end of an upgrade): name, what the destination was (absent, a file with its sha256 and
//! mode, a link with its target) and a state (planned, backed-up, replaced, restored), with from, to and the attempt id in the header. The old file is kept as
//! `<dest>.prev-<id>` by a hard link while the destination stays in place; the new file is renamed over it; a restore trusts a backup only by state and hash, is
//! idempotent, keeps every backup until every entry is restored and verified, and never turns a failure into a warning. DB-free. RED first (the module is a stub).

mod common;

use common::Tmp;
use cortex_upgrade::members::{
    BIN_FILES, DB_CONTAINER, Entry, Inventory, Kind, Layout, Old, RETIRED_MAX, Record, Source,
    State, Trust, UNIT_FILES, check_home, check_unit_dir, discard_plan, forget, info_lines,
    install, inventory, parse, plan, promote, read_last, read_update, recognised_backups, render,
    restore, serialize, sweep_retired, trusted_retired,
};
use sha2::{Digest, Sha256};
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

const FROM: &str = "0.2.1-rc.1";
const TO: &str = "0.2.1-rc.2";
const ID: &str = "00112233445566ff";
const ID2: &str = "aabbccddeeff0011";

struct Setup {
    tmp: Tmp,
    home: PathBuf,
    units: PathBuf,
}

fn bin_text(name: &str, tag: &str) -> Vec<u8> {
    format!("#!/bin/sh\n# {name} {tag}\n").into_bytes()
}

fn unit_template(name: &str, tag: &str) -> String {
    format!(
        "[Unit]\nDescription={name} {tag}\n[Service]\nExecStart=@CORTEX_HOME@/bin/{name} --target @DB_CONTAINER@\n"
    )
}

fn unit_text(name: &str, tag: &str, home: &Path) -> Vec<u8> {
    unit_template(name, tag)
        .replace("@CORTEX_HOME@", home.to_str().unwrap())
        .replace("@DB_CONTAINER@", DB_CONTAINER)
        .into_bytes()
}

/// A release directory `releases/<tag>` holding the ten members (units as TEMPLATES).
fn release(s: &Setup, tag: &str) -> PathBuf {
    let rel = s.home.join("releases").join(tag);
    fs::create_dir_all(rel.join("bin")).unwrap();
    fs::create_dir_all(rel.join("systemd")).unwrap();
    for n in BIN_FILES {
        fs::write(rel.join("bin").join(n), bin_text(n, tag)).unwrap();
    }
    for n in UNIT_FILES {
        fs::write(rel.join("systemd").join(n), unit_template(n, tag)).unwrap();
    }
    rel
}

fn setup(tag: &str) -> Setup {
    let tmp = Tmp::new(tag);
    let home = tmp.path().join("home");
    let units = tmp.path().join("units");
    fs::create_dir_all(home.join("bin")).unwrap();
    fs::create_dir_all(&units).unwrap();
    fs::set_permissions(&units, fs::Permissions::from_mode(0o700)).unwrap();
    Setup { tmp, home, units }
}

fn layout<'a>(s: &'a Setup, rel: &'a Path) -> Layout<'a> {
    Layout {
        release_dir: rel,
        home: &s.home,
        unit_dir: &s.units,
        db_container: DB_CONTAINER,
    }
}

fn dest(s: &Setup, kind: Kind, name: &str) -> PathBuf {
    match kind {
        Kind::Bin => s.home.join("bin").join(name),
        Kind::Unit => s.units.join(name),
    }
}

/// `<dest>.prev-<id>`
fn backup(s: &Setup, kind: Kind, name: &str, id: &str) -> PathBuf {
    let mut p = dest(s, kind, name).into_os_string();
    p.push(format!(".prev-{id}"));
    PathBuf::from(p)
}

fn sibling_of(p: &Path, suffix: &str) -> PathBuf {
    let mut o = p.as_os_str().to_os_string();
    o.push(suffix);
    PathBuf::from(o)
}

fn update_path(s: &Setup) -> PathBuf {
    s.home.join("members.update")
}

fn last_path(s: &Setup) -> PathBuf {
    s.home.join("members.last")
}

fn mode(p: &Path) -> u32 {
    fs::metadata(p).unwrap().permissions().mode() & 0o7777
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn sha(b: &[u8]) -> String {
    hex(&Sha256::digest(b))
}

/// The ten members in the record's order: the five scripts, then the five units.
fn names() -> Vec<(Kind, &'static str)> {
    BIN_FILES
        .iter()
        .map(|n| (Kind::Bin, *n))
        .chain(UNIT_FILES.iter().map(|n| (Kind::Unit, *n)))
        .collect()
}

fn text_of(s: &Setup, kind: Kind, name: &str, tag: &str) -> Vec<u8> {
    match kind {
        Kind::Bin => bin_text(name, tag),
        Kind::Unit => unit_text(name, tag, &s.home),
    }
}

fn mode_of(kind: Kind) -> u32 {
    match kind {
        Kind::Bin => 0o755,
        Kind::Unit => 0o644,
    }
}

/// What an earlier install left: all ten destinations hold the text for `tag`, with our modes.
fn old_set(s: &Setup, tag: &str) {
    for (k, n) in names() {
        let p = dest(s, k, n);
        fs::write(&p, text_of(s, k, n, tag)).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(mode_of(k))).unwrap();
    }
}

/// All ten destinations hold the text for `tag`, with our modes.
fn assert_installed(s: &Setup, tag: &str) {
    for (k, n) in names() {
        let p = dest(s, k, n);
        assert_eq!(fs::read(&p).unwrap(), text_of(s, k, n, tag), "{n}");
        assert_eq!(mode(&p), mode_of(k), "{n}");
    }
}

fn noop(_: &str) {}

/// Run `f` with a crash point that loses the process (a panic) at the boundary named `target`; `None` when the point was reached (the run was lost), the result
/// otherwise.
fn crash_at<R>(target: &str, f: impl FnOnce(&dyn Fn(&str)) -> R) -> Option<R> {
    let target = target.to_string();
    let point = move |n: &str| {
        if n == target {
            panic!("process lost at {n}");
        }
    };
    catch_unwind(AssertUnwindSafe(|| f(&point))).ok()
}

fn planned(s: &Setup, rel: &Path) -> Record {
    plan(&layout(s, rel), FROM, TO, ID, &noop).unwrap()
}

fn states(r: &Record) -> Vec<State> {
    r.entries.iter().map(|e| e.state).collect()
}

fn entry<'a>(r: &'a Record, name: &str) -> &'a Entry {
    r.entries.iter().find(|e| e.name == name).unwrap()
}

fn write_update(s: &Setup, r: &Record) {
    fs::write(update_path(s), serialize(r)).unwrap();
}

fn write_last(s: &Setup, r: &Record) {
    fs::write(last_path(s), serialize(r)).unwrap();
}

fn leftovers(dir: &Path, contains: &str) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.contains(contains))
        .collect();
    v.sort();
    v
}

#[test]
fn mb_01_render_fills_the_two_placeholders_and_refuses_any_other_token() {
    let t = b"A=@CORTEX_HOME@/x B=@DB_CONTAINER@ C=@CORTEX_HOME@\n";
    let out = render(t, "/srv/cortex-v2", "cv2-s5-db").unwrap();
    assert_eq!(out, b"A=/srv/cortex-v2/x B=cv2-s5-db C=/srv/cortex-v2\n");
    assert_eq!(
        render(b"no placeholder\n", "/h", "db").unwrap(),
        b"no placeholder\n"
    );
    // pure: the same input, the same output
    assert_eq!(
        render(t, "/h", "db").unwrap(),
        render(t, "/h", "db").unwrap()
    );
    // the render validates its own arguments: the home alphabet and the database name
    assert!(render(t, "/a b", "db").is_err());
    assert!(render(t, "/a/../b", "db").is_err());
    assert!(render(t, "relative", "db").is_err());
    assert!(render(t, "/h", "DB").is_err());
    assert!(render(t, "/h", "").is_err());
    assert!(render(t, "/h", "db_x").is_err());
    for bad in [
        &b"X=@UNKNOWN@\n"[..],
        b"X=@CORTEX_HOME@ @OTHER_THING@\n",
        b"X=@@\n",
    ] {
        assert!(
            render(bad, "/h", "db").is_err(),
            "{}",
            String::from_utf8_lossy(bad)
        );
    }
}

#[test]
fn mb_02_the_home_alphabet_is_strict() {
    for good in ["/home/a/cortex-v2", "/srv/x.y_z-1", "/a", "/A/B9/c.d"] {
        assert!(check_home(Path::new(good)).is_ok(), "{good}");
    }
    for bad in [
        "",
        "relative/dir",
        "./x",
        "/",
        "/a/",
        "/a//b",
        "/a/../b",
        "/..",
        "/a/..",
        "/a b",
        "/a%b",
        "/a$b",
        "/a\\b",
        "/a'b",
        "/a\"b",
        "/a|b",
        "/a&b",
        "/a\nb",
        "/a\u{1}b",
        "/aéb",
        "/a;b",
        "/a`b",
        "/a(b)",
        "/a*b",
    ] {
        assert!(
            check_home(Path::new(bad)).is_err(),
            "{bad:?} must be refused"
        );
    }
}

#[test]
fn mb_03_the_unit_directory_must_be_a_real_private_directory_of_ours() {
    let s = setup("mb03");
    for ok in [0o700, 0o755] {
        fs::set_permissions(&s.units, fs::Permissions::from_mode(ok)).unwrap();
        assert!(check_unit_dir(&s.units).is_ok(), "{ok:o}");
    }
    for bad in [0o770, 0o702, 0o777] {
        fs::set_permissions(&s.units, fs::Permissions::from_mode(bad)).unwrap();
        assert!(check_unit_dir(&s.units).is_err(), "{bad:o}");
    }
    fs::set_permissions(&s.units, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(check_unit_dir(Path::new("units")).is_err(), "relative");
    // a relative path that EXISTS (cargo runs the tests in the package directory, which has `tests`) is refused for being relative
    assert!(
        check_unit_dir(Path::new("tests")).is_err(),
        "relative and existing"
    );
    assert!(
        check_unit_dir(&s.tmp.path().join("missing")).is_err(),
        "missing"
    );
    let file = s.tmp.path().join("afile");
    fs::write(&file, "x").unwrap();
    assert!(check_unit_dir(&file).is_err(), "a file");
    let link = s.tmp.path().join("link");
    symlink(&s.units, &link).unwrap();
    assert!(
        check_unit_dir(&link).is_err(),
        "a symlink to a real directory"
    );
    // a directory that is not ours (root's /usr), unless this test runs as root
    let me = fs::metadata("/proc/self").unwrap().uid();
    if me != 0 {
        assert!(
            check_unit_dir(Path::new("/usr")).is_err(),
            "a directory of another owner"
        );
    }
}

#[test]
fn mb_32_the_db_container_constant_is_the_one_compose_install_and_db_roles_use() {
    // cox R-5 Q1 (a): a later rename (the cv2-s5-* names are provisional) fails here, loudly, in one place
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("deploy");
    let compose = fs::read_to_string(root.join("compose.yml")).unwrap();
    let install_sh = fs::read_to_string(root.join("install.sh")).unwrap();
    let roles = fs::read_to_string(root.join("db-roles.sh")).unwrap();
    assert!(
        compose
            .lines()
            .any(|l| l.trim() == format!("container_name: {DB_CONTAINER}")),
        "compose.yml names another database container"
    );
    assert!(
        install_sh.contains(&format!("wait_healthy {DB_CONTAINER}"))
            && install_sh.contains(&format!("exec -i --user postgres {DB_CONTAINER} psql")),
        "install.sh uses another database container"
    );
    assert!(
        roles.contains(&format!("db=${{CORTEX_DB_CONTAINER:-{DB_CONTAINER}}}")),
        "db-roles.sh defaults to another database container"
    );
}

fn golden_record() -> (Record, String) {
    let h1 = sha(b"one");
    let h2 = sha(b"two");
    let r = Record {
        from: FROM.to_string(),
        to: TO.to_string(),
        id: ID.to_string(),
        retired: Vec::new(),
        entries: vec![
            Entry {
                kind: Kind::Bin,
                name: "cortex-dump".to_string(),
                old: Old::File {
                    sha256: h1.clone(),
                    mode: 0o755,
                },
                state: State::Planned,
            },
            Entry {
                kind: Kind::Bin,
                name: "cortex-dump-status".to_string(),
                old: Old::Absent,
                state: State::BackedUp,
            },
            Entry {
                kind: Kind::Unit,
                name: "cortex-dump.timer".to_string(),
                old: Old::Link {
                    target_hex: "2e2e2f78".to_string(),
                },
                state: State::Replaced,
            },
            Entry {
                kind: Kind::Unit,
                name: "cortex-dump-alert.service".to_string(),
                old: Old::File {
                    sha256: h2.clone(),
                    mode: 0o600,
                },
                state: State::Restored,
            },
        ],
    };
    let text = format!(
        "members-record 1\nfrom={FROM}\nto={TO}\nid={ID}\nentry bin cortex-dump file:{h1}:0755 planned\nentry bin cortex-dump-status absent backed-up\nentry unit cortex-dump.timer link:2e2e2f78 replaced\nentry unit cortex-dump-alert.service file:{h2}:0600 restored\n"
    );
    (r, text)
}

#[test]
fn mb_04_the_record_has_one_exact_text_form_and_reads_back_to_the_same_record() {
    let (r, text) = golden_record();
    assert_eq!(serialize(&r), text);
    assert_eq!(parse(&text).unwrap(), r);
    // an empty record (an upgrade that changed no member) is a record too
    let empty = Record {
        entries: Vec::new(),
        ..r.clone()
    };
    assert_eq!(parse(&serialize(&empty)).unwrap(), empty);
    for st in [
        State::Planned,
        State::BackedUp,
        State::Replaced,
        State::Restored,
    ] {
        let mut x = r.clone();
        x.entries[0].state = st;
        assert_eq!(parse(&serialize(&x)).unwrap(), x);
    }
}

#[test]
fn mb_05_a_record_that_is_not_exactly_ours_is_refused() {
    let (_, good) = golden_record();
    let h1 = sha(b"one");
    let bad: Vec<(&str, String)> = vec![
        ("empty", String::new()),
        ("no final newline", good.trim_end().to_string()),
        (
            "another version of the format",
            good.replace("members-record 1", "members-record 2"),
        ),
        ("no header", good.replace("members-record 1\n", "")),
        ("from missing", good.replace(&format!("from={FROM}\n"), "")),
        (
            "from is not a version",
            good.replace(&format!("from={FROM}"), "from=0.2.001"),
        ),
        (
            "to is not a version",
            good.replace(&format!("to={TO}"), "to=latest"),
        ),
        (
            "from equals to",
            good.replace(&format!("to={TO}"), &format!("to={FROM}")),
        ),
        ("id of 15 characters", good.replace(ID, "00112233445566f")),
        ("id of 17 characters", good.replace(ID, "00112233445566ff0")),
        ("id in upper case", good.replace(ID, "00112233445566FF")),
        ("id that is not hex", good.replace(ID, "00112233445566fg")),
        (
            "id before to",
            good.replace(
                &format!("to={TO}\nid={ID}\n"),
                &format!("id={ID}\nto={TO}\n"),
            ),
        ),
        (
            "an unknown kind",
            good.replace("entry bin cortex-dump ", "entry dir cortex-dump "),
        ),
        (
            "an unknown name",
            good.replace("entry bin cortex-dump ", "entry bin cortex-other "),
        ),
        (
            "a script named as a unit",
            good.replace("entry bin cortex-dump ", "entry unit cortex-dump "),
        ),
        (
            "the same entry twice",
            format!("{good}entry bin cortex-dump absent planned\n"),
        ),
        (
            "the same entry twice in a row",
            good.replace(
                "entry bin cortex-dump-status absent backed-up\n",
                "entry bin cortex-dump-status absent backed-up\nentry bin cortex-dump-status absent backed-up\n",
            ),
        ),
        (
            "a first field that is not entry",
            good.replace("entry bin cortex-dump-status", "other bin cortex-dump-status"),
        ),
        (
            "a link target of 4097 bytes",
            good.replace("link:2e2e2f78", &format!("link:{}", "61".repeat(4097))),
        ),
        (
            "entries out of order",
            good.replace(
                &format!("entry bin cortex-dump file:{h1}:0755 planned\nentry bin cortex-dump-status absent backed-up\n"),
                &format!("entry bin cortex-dump-status absent backed-up\nentry bin cortex-dump file:{h1}:0755 planned\n"),
            ),
        ),
        (
            "a short hash",
            good.replace(&format!("file:{h1}"), "file:abc"),
        ),
        (
            "an upper-case hash",
            good.replace(&format!("file:{h1}"), &format!("file:{}", h1.to_uppercase())),
        ),
        ("a mode of three digits", good.replace(":0755", ":755")),
        ("a mode that is not octal", good.replace(":0755", ":0958")),
        ("a mode with a type bit", good.replace(":0755", ":10755")),
        (
            "a link target that is not hex",
            good.replace("link:2e2e2f78", "link:xyz"),
        ),
        (
            "an empty link target",
            good.replace("link:2e2e2f78", "link:"),
        ),
        (
            "an odd link target",
            good.replace("link:2e2e2f78", "link:2e2e2f7"),
        ),
        ("an unknown old state", good.replace(" absent ", " gone ")),
        ("an unknown state", good.replace("backed-up", "done")),
        (
            "a field too many",
            good.replace("planned\n", "planned extra\n"),
        ),
        (
            "a field too few",
            good.replace("entry bin cortex-dump-status absent backed-up\n", "entry bin cortex-dump-status absent\n"),
        ),
        ("a blank line", good.replace("\nentry bin", "\n\nentry bin")),
        ("CRLF", good.replace('\n', "\r\n")),
        ("a trailing space", good.replace("planned\n", "planned \n")),
        ("a line that is not an entry", format!("{good}hello\n")),
        ("larger than 16 KiB", format!("{good}{}", "#".repeat(17 * 1024))),
    ];
    // each is refused for ITS OWN reason (a refusal for another reason would prove nothing about the rule)
    let reason = |what: &str| -> &'static str {
        match what {
            "empty" | "no final newline" | "CRLF" => "line feed",
            "another version of the format" | "no header" => "header line",
            "from missing" | "id before to" => "from=, to= and id=",
            "from is not a version" | "to is not a version" => "not both release versions",
            "from equals to" => "the same release",
            w if w.starts_with("id ") => "attempt id",
            "an unknown kind" => "kind that is not bin or unit",
            "an unknown name" | "a script named as a unit" => "not one of the ten",
            "the same entry twice" | "the same entry twice in a row" | "entries out of order" => {
                "fixed order"
            }
            "a field too many"
            | "a field too few"
            | "a blank line"
            | "a trailing space"
            | "a line that is not an entry" => "five fields",
            "an unknown state" => "state that is not",
            "a first field that is not entry" => "not an entry",
            "a link target of 4097 bytes" => "old state",
            "larger than 16 KiB" => "larger than 16 KiB",
            _ => "old state",
        }
    };
    for (what, text) in bad {
        let e = parse(&text).expect_err(what).to_string();
        assert!(e.contains(reason(what)), "{what}: {e}");
    }
    // the limits are exact: a link target of 4096 bytes is a record; a text of exactly 16 KiB is refused for what it holds, not for its size
    let ok = good.replace("link:2e2e2f78", &format!("link:{}", "61".repeat(4096)));
    assert!(parse(&ok).is_ok(), "a link target of 4096 bytes");
    let exact = format!("{good}{}", "x".repeat(16 * 1024 - good.len() - 1) + "\n");
    assert_eq!(exact.len(), 16 * 1024);
    let e = parse(&exact).unwrap_err().to_string();
    assert!(
        !e.contains("larger than 16 KiB"),
        "exactly 16 KiB is not larger: {e}"
    );
}

#[test]
fn mb_06_a_plan_lists_every_changed_entry_with_what_it_was_and_changes_nothing_else() {
    let s = setup("mb06");
    old_set(&s, "v0");
    let rel = release(&s, "v1");
    // one script is already ours (bytes and mode): not listed
    fs::write(
        dest(&s, Kind::Bin, "cortex-dump"),
        bin_text("cortex-dump", "v1"),
    )
    .unwrap();
    // a unit that is absent, a script that is a link, a script with our bytes and another mode, a script with another mode
    fs::remove_file(dest(&s, Kind::Unit, "cortex-dump-alert.service")).unwrap();
    let target = s.tmp.path().join("elsewhere");
    fs::write(&target, "x").unwrap();
    let rc = dest(&s, Kind::Bin, "restore-compare");
    fs::remove_file(&rc).unwrap();
    symlink(&target, &rc).unwrap();
    let before = fs::read(dest(&s, Kind::Bin, "cortex-dump-alert")).unwrap();
    fs::set_permissions(
        dest(&s, Kind::Bin, "cortex-dump-status"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let r = planned(&s, &rel);
    assert_eq!(
        (r.from.as_str(), r.to.as_str(), r.id.as_str()),
        (FROM, TO, ID)
    );
    assert!(
        r.entries.iter().all(|e| e.state == State::Planned),
        "all planned"
    );
    assert!(
        r.entries.iter().all(|e| e.name != "cortex-dump"),
        "ours: not listed"
    );
    assert_eq!(r.entries.len(), 9);
    assert_eq!(
        entry(&r, "cortex-dump-alert").old,
        Old::File {
            sha256: sha(&before),
            mode: 0o755
        }
    );
    assert_eq!(
        entry(&r, "cortex-dump-status").old,
        Old::File {
            sha256: sha(&bin_text("cortex-dump-status", "v0")),
            mode: 0o700
        }
    );
    assert_eq!(entry(&r, "cortex-dump-alert.service").old, Old::Absent);
    assert_eq!(
        entry(&r, "restore-compare").old,
        Old::Link {
            target_hex: hex(target.to_str().unwrap().as_bytes())
        }
    );
    // the record is on disk, durable and private; nothing else changed
    assert_eq!(fs::read_to_string(update_path(&s)).unwrap(), serialize(&r));
    assert_eq!(mode(&update_path(&s)), 0o600);
    assert_eq!(read_update(&s.home).unwrap(), Some(r));
    assert_eq!(
        fs::read(dest(&s, Kind::Bin, "cortex-dump-alert")).unwrap(),
        before
    );
    assert!(leftovers(&s.home.join("bin"), ".prev").is_empty());
    assert!(leftovers(&s.home.join("bin"), ".new").is_empty());
}

#[test]
fn mb_07_a_plan_of_an_install_with_nothing_to_change_is_an_empty_record() {
    let s = setup("mb07");
    let rel = release(&s, "v1");
    let first = planned(&s, &rel);
    install(&layout(&s, &rel), FROM, TO, ID, &noop).unwrap();
    promote(&s.home, FROM, TO, ID, &noop).unwrap();
    assert_installed(&s, "v1");
    // the same release again (an upgrade that changes no member): the record is written, with no entry
    let again = plan(&layout(&s, &rel), TO, "0.2.1-rc.3", ID2, &noop).unwrap();
    assert!(again.entries.is_empty(), "{again:?}");
    assert_eq!(read_update(&s.home).unwrap(), Some(again));
    assert_eq!(first.entries.len(), 10);
}

#[test]
fn mb_08_a_plan_that_cannot_be_trusted_writes_nothing() {
    let s = setup("mb08");
    old_set(&s, "v0");
    let rel = release(&s, "v1");
    let snapshot = |s: &Setup| -> Vec<(String, Vec<u8>)> {
        let mut v = Vec::new();
        for d in [&s.home, &s.home.join("bin"), &s.units] {
            for e in fs::read_dir(d).unwrap().flatten() {
                if e.path().is_file() {
                    v.push((e.path().display().to_string(), fs::read(e.path()).unwrap()));
                }
            }
        }
        v.sort();
        v
    };
    let before = snapshot(&s);
    // a member that the release does not carry
    fs::remove_file(rel.join("bin").join("cortex-restore")).unwrap();
    let e = plan(&layout(&s, &rel), FROM, TO, ID, &noop).unwrap_err();
    assert!(e.to_string().contains("cortex-restore"), "{e}");
    assert_eq!(snapshot(&s), before);
    fs::write(
        rel.join("bin").join("cortex-restore"),
        bin_text("cortex-restore", "v1"),
    )
    .unwrap();
    // a template with a token that is not a placeholder
    fs::write(
        rel.join("systemd").join("cortex-dump.timer"),
        "[Timer]\nX=@NOPE@\n",
    )
    .unwrap();
    let e = plan(&layout(&s, &rel), FROM, TO, ID, &noop).unwrap_err();
    assert!(e.to_string().contains("@NOPE@"), "{e}");
    assert_eq!(snapshot(&s), before);
    fs::write(
        rel.join("systemd").join("cortex-dump.timer"),
        unit_template("cortex-dump.timer", "v1"),
    )
    .unwrap();
    // a directory at a destination
    fs::remove_file(dest(&s, Kind::Unit, "cortex-dump.timer")).unwrap();
    fs::create_dir(dest(&s, Kind::Unit, "cortex-dump.timer")).unwrap();
    let e = plan(&layout(&s, &rel), FROM, TO, ID, &noop).unwrap_err();
    assert!(
        e.to_string().contains("not a file, a link or absent"),
        "{e}"
    );
    assert!(!update_path(&s).exists());
    fs::remove_dir(dest(&s, Kind::Unit, "cortex-dump.timer")).unwrap();
    fs::write(
        dest(&s, Kind::Unit, "cortex-dump.timer"),
        unit_text("cortex-dump.timer", "v0", &s.home),
    )
    .unwrap();
    // a unit directory that is not ours to write into
    fs::set_permissions(&s.units, fs::Permissions::from_mode(0o777)).unwrap();
    let e = plan(&layout(&s, &rel), FROM, TO, ID, &noop).unwrap_err();
    assert!(e.to_string().contains("writable by group or other"), "{e}");
    fs::set_permissions(&s.units, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(!update_path(&s).exists());
    // a version pair or an id that is not valid
    for (f, t, i) in [
        ("0.2.001", TO, ID),
        (FROM, "latest", ID),
        (FROM, FROM, ID),
        (FROM, TO, "short"),
        (FROM, TO, "00112233445566FF"),
    ] {
        assert!(
            plan(&layout(&s, &rel), f, t, i, &noop).is_err(),
            "{f} {t} {i}"
        );
    }
    assert!(!update_path(&s).exists());
}

#[test]
fn mb_09_a_plan_met_again_is_kept_replaced_when_stale_and_refused_when_it_is_not() {
    let s = setup("mb09");
    old_set(&s, "v0");
    let rel = release(&s, "v1");
    let first = planned(&s, &rel);
    // the same attempt asked again (a retry of the plan): the record that is there is returned as it is, whatever state it has reached
    let mut advanced = first.clone();
    advanced.entries[0].state = State::BackedUp;
    write_update(&s, &advanced);
    let again = plan(&layout(&s, &rel), FROM, TO, ID, &noop).unwrap();
    assert_eq!(again, advanced);
    // another attempt's plan in which NOTHING happened (every entry planned): stale, replaced by the new plan
    write_update(&s, &first);
    let other = plan(&layout(&s, &rel), FROM, TO, ID2, &noop).unwrap();
    assert_eq!(other.id, ID2);
    assert_eq!(read_update(&s.home).unwrap().unwrap().id, ID2);
    // another attempt's record in which something happened: never replaced, nothing changed
    write_update(&s, &advanced);
    let bytes = fs::read(update_path(&s)).unwrap();
    assert!(plan(&layout(&s, &rel), FROM, TO, ID2, &noop).is_err());
    assert_eq!(fs::read(update_path(&s)).unwrap(), bytes);
    // a record that cannot be read is not replaced either
    fs::write(update_path(&s), "garbage\n").unwrap();
    assert!(plan(&layout(&s, &rel), FROM, TO, ID2, &noop).is_err());
    assert_eq!(fs::read(update_path(&s)).unwrap(), b"garbage\n");
}

#[test]
fn mb_10_a_plan_leaves_members_last_alone_and_removes_the_leftover_of_a_promotion() {
    let s = setup("mb10");
    old_set(&s, "v0");
    let rel = release(&s, "v1");
    let (r, _) = golden_record();
    write_last(&s, &r);
    fs::write(s.home.join("members.last.new"), "half a promotion").unwrap();
    let last_before = fs::read(last_path(&s)).unwrap();
    planned(&s, &rel);
    assert_eq!(fs::read(last_path(&s)).unwrap(), last_before);
    assert!(!s.home.join("members.last.new").exists());
}

#[test]
fn mb_11_the_forward_protocol_changes_one_entry_at_a_time_and_never_leaves_a_destination_absent() {
    let s = setup("mb11");
    old_set(&s, "v0");
    let rel = release(&s, "v1");
    let inodes: Vec<(Kind, &str, u64)> = names()
        .into_iter()
        .map(|(k, n)| (k, n, fs::metadata(dest(&s, k, n)).unwrap().ino()))
        .collect();
    planned(&s, &rel);
    let up = install(&layout(&s, &rel), FROM, TO, ID, &noop).unwrap();
    assert_installed(&s, "v1");
    assert_eq!(up.entries.len(), 10);
    assert!(up.entries.iter().all(|e| !e.2), "none was created");
    let r = read_update(&s.home).unwrap().unwrap();
    assert!(states(&r).iter().all(|st| *st == State::Replaced));
    for (k, n, ino) in inodes {
        let b = backup(&s, k, n, ID);
        let m = fs::symlink_metadata(&b).unwrap();
        assert!(m.file_type().is_file(), "{n}: the backup is a regular file");
        assert_eq!(
            m.ino(),
            ino,
            "{n}: the backup is a HARD LINK of the old file"
        );
        assert_eq!(
            fs::read(&b).unwrap(),
            text_of(&s, k, n, "v0"),
            "{n}: the backup holds the old bytes"
        );
        assert_ne!(
            fs::metadata(dest(&s, k, n)).unwrap().ino(),
            ino,
            "{n}: the destination is a new file"
        );
    }
    assert!(leftovers(&s.home.join("bin"), ".new").is_empty());
    assert!(leftovers(&s.units, ".new").is_empty());
    assert_eq!(read_update(&s.home).unwrap(), Some(r));
}

#[test]
fn mb_12_a_run_lost_at_any_boundary_of_the_forward_protocol_is_continued_by_the_next_call() {
    let points = [
        "backup-linked",
        "backed-up",
        "new-written",
        "renamed",
        "replaced",
    ];
    for (idx, (kind, name)) in names().into_iter().enumerate() {
        if ![0, 4, 5, 9].contains(&idx) {
            continue;
        }
        for p in points {
            let s = setup(&format!("mb12-{idx}-{p}"));
            old_set(&s, "v0");
            let rel = release(&s, "v1");
            planned(&s, &rel);
            let target = format!("{name}:{p}");
            let outcome = crash_at(&target, |pt| install(&layout(&s, &rel), FROM, TO, ID, pt));
            assert!(outcome.is_none(), "the boundary {target} was reached");
            // at the crash every destination holds the old OR the new content, never nothing
            for (k, n) in names() {
                let b = fs::read(dest(&s, k, n))
                    .unwrap_or_else(|e| panic!("{n} is absent after a loss at {target}: {e}"));
                assert!(
                    b == text_of(&s, k, n, "v0") || b == text_of(&s, k, n, "v1"),
                    "{n} after a loss at {target}"
                );
            }
            // the record can be read, entries after the one lost at are still planned, none is ahead of the disk
            let r = read_update(&s.home).unwrap().unwrap();
            for (i, st) in states(&r).into_iter().enumerate() {
                if i > idx {
                    assert_eq!(st, State::Planned, "entry {i} after a loss at {target}");
                }
            }
            // the next call carries on from where it was lost
            install(&layout(&s, &rel), FROM, TO, ID, &noop)
                .unwrap_or_else(|e| panic!("the retry after a loss at {target}: {e}"));
            assert_installed(&s, "v1");
            let r = read_update(&s.home).unwrap().unwrap();
            assert!(
                states(&r).iter().all(|st| *st == State::Replaced),
                "{target}"
            );
            for (k, n) in names() {
                assert_eq!(
                    fs::read(backup(&s, k, n, ID)).unwrap(),
                    text_of(&s, k, n, "v0"),
                    "{n}: the backup holds v0 after a loss at {target}"
                );
            }
            assert!(
                leftovers(&s.home.join("bin"), ".new").is_empty(),
                "{target}"
            );
            let _ = kind;
        }
    }
}

#[test]
fn mb_13_a_backup_of_another_generation_is_never_taken_for_this_ones() {
    // mike's F1: a crash after the plan and before the backup, with a STALE .prev of an older generation on disk: neither a retry nor a restore may put
    // the wrong generation back
    let s = setup("mb13");
    old_set(&s, "v1");
    for (k, n) in names() {
        fs::write(
            format!("{}.prev", dest(&s, k, n).display()),
            text_of(&s, k, n, "v0"),
        )
        .unwrap();
        fs::write(backup(&s, k, n, ID2), text_of(&s, k, n, "v0")).unwrap();
    }
    let rel = release(&s, "v2");
    planned(&s, &rel);
    // lost right after the plan: restoring leaves every destination alone and uses no backup
    let inv = inventory(&s.home, Some((FROM, TO, ID)), None)
        .unwrap()
        .unwrap();
    restore(&s.home, &s.units, &inv, &noop).unwrap();
    assert_installed(&s, "v1");
    forget(&s.home, &inv).unwrap();
    // plan again and install: the backups are this attempt's own, holding v1
    planned(&s, &rel);
    install(&layout(&s, &rel), FROM, TO, ID, &noop).unwrap();
    assert_installed(&s, "v2");
    let inv = inventory(&s.home, Some((FROM, TO, ID)), None)
        .unwrap()
        .unwrap();
    restore(&s.home, &s.units, &inv, &noop).unwrap();
    assert_installed(&s, "v1");
    // the stale copies were never touched
    for (k, n) in names() {
        assert_eq!(
            fs::read(format!("{}.prev", dest(&s, k, n).display())).unwrap(),
            text_of(&s, k, n, "v0")
        );
        assert_eq!(
            fs::read(backup(&s, k, n, ID2)).unwrap(),
            text_of(&s, k, n, "v0")
        );
    }
}

#[test]
fn mb_14_a_link_an_absent_file_and_a_mode_are_put_back_as_they_were() {
    let s = setup("mb14");
    old_set(&s, "v0");
    let rel = release(&s, "v1");
    let target = s.tmp.path().join("elsewhere");
    fs::write(&target, "x").unwrap();
    let rc = dest(&s, Kind::Bin, "restore-compare");
    fs::remove_file(&rc).unwrap();
    symlink(&target, &rc).unwrap();
    fs::remove_file(dest(&s, Kind::Unit, "cortex-dump-alert.service")).unwrap();
    let odd = dest(&s, Kind::Bin, "cortex-dump-alert");
    fs::set_permissions(&odd, fs::Permissions::from_mode(0o700)).unwrap();
    planned(&s, &rel);
    let up = install(&layout(&s, &rel), FROM, TO, ID, &noop).unwrap();
    assert!(
        up.entries
            .iter()
            .any(|e| e.1 == "cortex-dump-alert.service" && e.2),
        "created"
    );
    assert!(
        !backup(&s, Kind::Bin, "restore-compare", ID).exists(),
        "a link has no backup"
    );
    assert!(
        !backup(&s, Kind::Unit, "cortex-dump-alert.service", ID).exists(),
        "an absent file has no backup"
    );
    assert_installed(&s, "v1");
    let inv = inventory(&s.home, Some((FROM, TO, ID)), None)
        .unwrap()
        .unwrap();
    restore(&s.home, &s.units, &inv, &noop).unwrap();
    assert_eq!(fs::read_link(&rc).unwrap(), target, "the link is back");
    assert!(
        !dest(&s, Kind::Unit, "cortex-dump-alert.service").exists(),
        "the file the upgrade created is gone"
    );
    assert_eq!(mode(&odd), 0o700, "the mode is back");
    assert_eq!(fs::read(&odd).unwrap(), bin_text("cortex-dump-alert", "v0"));
    assert!(leftovers(&s.home.join("bin"), ".restore").is_empty());
    assert!(
        leftovers(&s.home.join("bin"), ".prev-").is_empty(),
        "backups gone once all is restored"
    );
}

#[test]
fn mb_15_an_install_refuses_a_destination_that_changed_after_the_plan() {
    let s = setup("mb15x");
    old_set(&s, "v0");
    let rel = release(&s, "v1");
    planned(&s, &rel);
    let d = dest(&s, Kind::Bin, "cortex-dump-status");
    fs::write(&d, "someone edited this\n").unwrap();
    let e = install(&layout(&s, &rel), FROM, TO, ID, &noop).unwrap_err();
    assert!(e.to_string().contains("cortex-dump-status"), "{e}");
    assert_eq!(
        fs::read(&d).unwrap(),
        b"someone edited this\n",
        "not replaced"
    );
    // a replaced entry whose destination is no longer what was installed
    let s2 = setup("mb15y");
    old_set(&s2, "v0");
    let rel2 = release(&s2, "v1");
    planned(&s2, &rel2);
    install(&layout(&s2, &rel2), FROM, TO, ID, &noop).unwrap();
    let d2 = dest(&s2, Kind::Unit, "cortex-dump.timer");
    fs::write(&d2, "tampered\n").unwrap();
    let e = install(&layout(&s2, &rel2), FROM, TO, ID, &noop).unwrap_err();
    assert!(e.to_string().contains("cortex-dump.timer"), "{e}");
}

#[test]
fn mb_16_install_creates_the_bin_directory_when_the_home_has_none() {
    let s = setup("mb16x");
    fs::remove_dir(s.home.join("bin")).unwrap();
    let rel = release(&s, "v1");
    planned(&s, &rel);
    install(&layout(&s, &rel), FROM, TO, ID, &noop).unwrap();
    assert_installed(&s, "v1");
    assert_eq!(mode(&s.home.join("bin")), 0o755);
}

/// A finished install of release v1 over v0: plan, install, and the inventory a rollback of it would use.
fn installed(tag: &str) -> (Setup, PathBuf) {
    let s = setup(tag);
    old_set(&s, "v0");
    let rel = release(&s, "v1");
    planned(&s, &rel);
    install(&layout(&s, &rel), FROM, TO, ID, &noop).unwrap();
    (s, rel)
}

fn live_inventory(s: &Setup) -> Inventory {
    inventory(&s.home, Some((FROM, TO, ID)), None)
        .unwrap()
        .unwrap()
}

#[test]
fn mb_17_a_restore_puts_every_entry_back_keeps_the_record_and_removes_the_backups_last() {
    let (s, _rel) = installed("mb17");
    let inv = live_inventory(&s);
    assert_eq!(inv.source, Source::Update);
    let done = restore(&s.home, &s.units, &inv, &noop).unwrap();
    assert_eq!(done.len(), 10);
    assert_installed(&s, "v0");
    assert!(leftovers(&s.home.join("bin"), ".prev-").is_empty());
    assert!(leftovers(&s.units, ".prev-").is_empty());
    assert!(leftovers(&s.home.join("bin"), ".restore").is_empty());
    // the record stays (all restored) until the caller has cleared the marker, then `forget` removes it
    let r = read_update(&s.home).unwrap().unwrap();
    assert!(states(&r).iter().all(|st| *st == State::Restored));
    forget(&s.home, &inv).unwrap();
    assert!(!update_path(&s).exists());
    forget(&s.home, &inv).unwrap();
}

#[test]
fn mb_18_a_restore_is_idempotent_and_a_second_one_changes_nothing() {
    let (s, _rel) = installed("mb18");
    let inv = live_inventory(&s);
    restore(&s.home, &s.units, &inv, &noop).unwrap();
    let again = inventory(&s.home, Some((FROM, TO, ID)), None)
        .unwrap()
        .unwrap();
    restore(&s.home, &s.units, &again, &noop).unwrap();
    assert_installed(&s, "v0");
}

#[test]
fn mb_19_a_planned_entry_is_only_verified_and_its_destination_is_never_touched() {
    let s = setup("mb19");
    old_set(&s, "v0");
    let rel = release(&s, "v1");
    planned(&s, &rel);
    // a link of THIS attempt's name that a lost run made before it recorded backed-up, and a stale backup of another id
    let d = dest(&s, Kind::Bin, "cortex-dump");
    fs::hard_link(&d, backup(&s, Kind::Bin, "cortex-dump", ID)).unwrap();
    fs::write(
        backup(&s, Kind::Bin, "cortex-dump-status", ID2),
        "stale generation\n",
    )
    .unwrap();
    // and a staging file of this attempt that a lost restore left for an entry that did not need one
    let stale_restore = sibling_of(
        &dest(&s, Kind::Bin, "cortex-restore"),
        &format!(".restore-{ID}"),
    );
    fs::write(&stale_restore, "left by a lost restore").unwrap();
    let inv = live_inventory(&s);
    restore(&s.home, &s.units, &inv, &noop).unwrap();
    assert_installed(&s, "v0");
    assert!(
        !stale_restore.exists(),
        "the cleanup removes this attempt's staging files"
    );
    assert!(
        !backup(&s, Kind::Bin, "cortex-dump", ID).exists(),
        "this attempt's own leftover link goes with the cleanup"
    );
    assert!(
        backup(&s, Kind::Bin, "cortex-dump-status", ID2).exists(),
        "another attempt's backup is not ours to remove"
    );
    // a planned entry whose destination is NOT what was recorded: a failure, nothing deleted
    let s2 = setup("mb19b");
    old_set(&s2, "v0");
    let rel2 = release(&s2, "v1");
    planned(&s2, &rel2);
    fs::hard_link(
        dest(&s2, Kind::Bin, "cortex-dump"),
        backup(&s2, Kind::Bin, "cortex-dump", ID),
    )
    .unwrap();
    fs::write(
        dest(&s2, Kind::Bin, "cortex-restore"),
        "changed since the plan\n",
    )
    .unwrap();
    let inv2 = live_inventory(&s2);
    let e = restore(&s2.home, &s2.units, &inv2, &noop).unwrap_err();
    assert!(e.to_string().contains("cortex-restore"), "{e}");
    assert!(
        e.to_string().contains("never replaced"),
        "a planned entry is judged as a planned entry: {e}"
    );
    assert!(
        backup(&s2, Kind::Bin, "cortex-dump", ID).exists(),
        "nothing deleted"
    );
    assert!(update_path(&s2).exists(), "the record stays");
}

#[test]
fn mb_20_a_backup_whose_hash_differs_is_a_failure_that_keeps_everything() {
    let (s, _rel) = installed("mb20");
    let victim = backup(&s, Kind::Bin, "cortex-dump-alert", ID);
    fs::remove_file(&victim).unwrap();
    fs::write(&victim, "not the file that was backed up\n").unwrap();
    let inv = live_inventory(&s);
    let e = restore(&s.home, &s.units, &inv, &noop).unwrap_err();
    let text = e.to_string();
    assert!(
        text.contains("cortex-dump-alert") && text.contains(".prev-"),
        "names the entry and the file: {text}"
    );
    // NOTHING deleted: every backup is still there (the entries restored before the failure keep theirs too), the record stays
    for (k, n) in names() {
        assert!(backup(&s, k, n, ID).exists(), "{n}: the backup is kept");
    }
    assert!(update_path(&s).exists());
    // the entries before the failing one are restored, the failing one still holds the NEW file
    assert_eq!(
        fs::read(dest(&s, Kind::Bin, "cortex-dump")).unwrap(),
        bin_text("cortex-dump", "v0")
    );
    assert_eq!(
        fs::read(dest(&s, Kind::Bin, "cortex-dump-alert")).unwrap(),
        bin_text("cortex-dump-alert", "v1")
    );
    // a missing backup of an entry that was replaced is the same failure
    let (s2, _r) = installed("mb20b");
    fs::remove_file(backup(&s2, Kind::Unit, "cortex-dump.timer", ID)).unwrap();
    let inv2 = live_inventory(&s2);
    let e2 = restore(&s2.home, &s2.units, &inv2, &noop).unwrap_err();
    assert!(e2.to_string().contains("cortex-dump.timer"), "{e2}");
    assert!(backup(&s2, Kind::Unit, "cortex-dump-status.timer", ID).exists());
}

#[test]
fn mb_21_a_restore_that_failed_at_one_entry_is_resumed_and_completes_when_the_cause_is_gone() {
    let (s, _rel) = installed("mb21");
    let victim = backup(&s, Kind::Bin, "cortex-restore", ID);
    let good = fs::read(&victim).unwrap();
    fs::remove_file(&victim).unwrap();
    fs::write(&victim, "bad\n").unwrap();
    let inv = live_inventory(&s);
    assert!(restore(&s.home, &s.units, &inv, &noop).is_err());
    // the cause goes (the backup is put back byte for byte); the re-run RESUMES: the entries before it are verified, the rest is done
    fs::remove_file(&victim).unwrap();
    fs::write(&victim, &good).unwrap();
    fs::set_permissions(&victim, fs::Permissions::from_mode(0o755)).unwrap();
    let inv = live_inventory(&s);
    restore(&s.home, &s.units, &inv, &noop).unwrap();
    assert_installed(&s, "v0");
    assert!(leftovers(&s.home.join("bin"), ".prev-").is_empty());
}

#[test]
fn mb_22_a_restore_lost_at_any_boundary_is_finished_by_the_next_call() {
    for (idx, (_k, name)) in names().into_iter().enumerate() {
        if ![0, 4, 5, 9].contains(&idx) {
            continue;
        }
        for p in ["restore-staged", "restore-renamed", "restored"] {
            let (s, _rel) = installed(&format!("mb22-{idx}-{p}"));
            let inv = live_inventory(&s);
            let target = format!("{name}:{p}");
            let outcome = crash_at(&target, |pt| restore(&s.home, &s.units, &inv, pt));
            assert!(outcome.is_none(), "the boundary {target} was reached");
            // every backup is still there and every destination holds the old or the new file
            for (k, n) in names() {
                assert!(
                    backup(&s, k, n, ID).exists(),
                    "{n} after a loss at {target}"
                );
                let b = fs::read(dest(&s, k, n)).unwrap();
                assert!(
                    b == text_of(&s, k, n, "v0") || b == text_of(&s, k, n, "v1"),
                    "{n} after a loss at {target}"
                );
            }
            let inv = live_inventory(&s);
            restore(&s.home, &s.units, &inv, &noop)
                .unwrap_or_else(|e| panic!("the re-run after a loss at {target}: {e}"));
            assert_installed(&s, "v0");
            assert!(
                leftovers(&s.home.join("bin"), ".prev-").is_empty(),
                "{target}"
            );
            assert!(
                leftovers(&s.home.join("bin"), ".restore").is_empty(),
                "{target}"
            );
        }
    }
}

fn record_for(id: &str, from: &str, to: &str, st: State) -> Record {
    Record {
        from: from.to_string(),
        to: to.to_string(),
        id: id.to_string(),
        retired: Vec::new(),
        entries: vec![Entry {
            kind: Kind::Bin,
            name: "cortex-dump".to_string(),
            old: Old::Absent,
            state: st,
        }],
    }
}

fn why(r: Result<Option<Inventory>, cortex_upgrade::members::MemberError>) -> String {
    match r {
        Err(e) => e.to_string(),
        Ok(v) => panic!("expected a refusal, got {v:?}"),
    }
}

#[test]
fn mb_23_the_inventory_of_an_unfinished_upgrade_is_found_by_from_to_and_id() {
    let s = setup("mb23");
    let m = Some((FROM, TO, ID));
    // the marker, the live record: that is the inventory
    write_update(&s, &record_for(ID, FROM, TO, State::Replaced));
    let inv = inventory(&s.home, m, None).unwrap().unwrap();
    assert_eq!(inv.source, Source::Update);
    // the same record and members.last both there (crash after S2, before S3): ONE record
    write_last(&s, &record_for(ID, FROM, TO, State::Replaced));
    assert_eq!(
        inventory(&s.home, m, None).unwrap().unwrap().source,
        Source::Update
    );
    // ... but not if they disagree (never a pick of one)
    let mut other = record_for(ID, FROM, TO, State::Replaced);
    other.entries[0].old = Old::File {
        sha256: sha(b"x"),
        mode: 0o755,
    };
    write_last(&s, &other);
    assert!(why(inventory(&s.home, m, None)).contains("members.last"));
    // a members.last of an OLDER upgrade (another id) next to the live record is the normal state before S2
    write_last(&s, &record_for(ID2, "0.2.0", FROM, State::Replaced));
    assert_eq!(
        inventory(&s.home, m, None).unwrap().unwrap().source,
        Source::Update
    );
    // the live record of ANOTHER attempt (other id) is not this upgrade's
    write_update(&s, &record_for(ID2, FROM, TO, State::Replaced));
    assert!(why(inventory(&s.home, m, None)).contains("members.update"));
    // an unreadable live record is a failure, never a fallback to members.last
    fs::write(update_path(&s), "garbage\n").unwrap();
    assert!(why(inventory(&s.home, m, None)).contains("members.update"));
    // members.update absent: members.last bound to this attempt (crash after S3, before the marker was cleared) IS the inventory, not a lost one
    fs::remove_file(update_path(&s)).unwrap();
    write_last(&s, &record_for(ID, FROM, TO, State::Replaced));
    let inv = inventory(&s.home, m, None).unwrap().unwrap();
    assert_eq!(inv.source, Source::Last);
    // ... but members.last of the same pair and ANOTHER id is a lost inventory (W1-62)
    write_last(&s, &record_for(ID2, FROM, TO, State::Replaced));
    assert!(why(inventory(&s.home, m, None)).contains("lost"));
    // ... and so is one of ANOTHER pair
    write_last(&s, &record_for(ID, "0.2.0", FROM, State::Replaced));
    assert!(why(inventory(&s.home, m, None)).contains("lost"));
    // ... and none at all, or an unreadable one
    fs::remove_file(last_path(&s)).unwrap();
    assert!(why(inventory(&s.home, m, None)).contains("lost"));
    fs::write(last_path(&s), "garbage\n").unwrap();
    assert!(why(inventory(&s.home, m, None)).contains("members.last"));
}

#[test]
fn mb_24_the_inventory_of_a_finished_upgrade_is_members_last_bound_to_the_release_being_left() {
    let s = setup("mb24");
    // no marker, no record at all: nothing is known and nothing is claimed
    assert_eq!(inventory(&s.home, None, Some(TO)).unwrap(), None);
    // members.last whose `to` is the release being left: the inventory
    write_last(&s, &record_for(ID, FROM, TO, State::Replaced));
    let inv = inventory(&s.home, None, Some(TO)).unwrap().unwrap();
    assert_eq!(inv.source, Source::Last);
    // another `to`: not usable, a failure that changes nothing
    assert!(why(inventory(&s.home, None, Some("0.2.1-rc.9"))).contains("members.last"));
    // unreadable
    fs::write(last_path(&s), "garbage\n").unwrap();
    assert!(why(inventory(&s.home, None, Some(TO))).contains("members.last"));
    // a leftover live record with nothing else to do (every entry restored, or never touched) is ignored; one with work in it is a failure
    fs::remove_file(last_path(&s)).unwrap();
    write_update(&s, &record_for(ID, FROM, TO, State::Restored));
    assert_eq!(inventory(&s.home, None, Some(TO)).unwrap(), None);
    write_update(&s, &record_for(ID, FROM, TO, State::Planned));
    assert_eq!(
        inventory(&s.home, None, Some(TO)).unwrap(),
        None,
        "a plan in which nothing happened is not an inventory"
    );
    write_update(&s, &record_for(ID, FROM, TO, State::BackedUp));
    assert!(why(inventory(&s.home, None, Some(TO))).contains("members.update"));
    write_update(&s, &record_for(ID, FROM, TO, State::Replaced));
    assert!(why(inventory(&s.home, None, Some(TO))).contains("members.update"));
}

#[test]
fn mb_25_the_end_of_an_upgrade_promotes_the_record_in_three_durable_steps() {
    let (s, _rel) = installed("mb25");
    let old_last = record_for(ID2, "0.2.0", FROM, State::Replaced);
    write_last(&s, &old_last);
    promote(&s.home, FROM, TO, ID, &noop).unwrap();
    let last = read_last(&s.home).unwrap().unwrap();
    assert_eq!(
        mode(&last_path(&s)),
        0o600,
        "the promoted record is private"
    );
    assert_eq!(
        (last.from.as_str(), last.to.as_str(), last.id.as_str()),
        (FROM, TO, ID)
    );
    assert!(states(&last).iter().all(|st| *st == State::Replaced));
    assert_eq!(last.entries.len(), 10);
    assert!(!update_path(&s).exists(), "S3: members.update is gone");
    assert!(!s.home.join("members.last.new").exists());
    // idempotent: the same call again (the marker was not cleared yet, a resume came back)
    promote(&s.home, FROM, TO, ID, &noop).unwrap();
    assert_eq!(read_last(&s.home).unwrap().unwrap(), last);
}

#[test]
fn mb_26_a_promotion_lost_at_any_step_is_finished_by_the_next_call_and_never_loses_the_older_record()
 {
    for p in ["promote:S1", "promote:S2", "promote:S3"] {
        let (s, _rel) = installed(&format!("mb26-{}", &p[9..]));
        let old_last = record_for(ID2, "0.2.0", FROM, State::Replaced);
        write_last(&s, &old_last);
        let outcome = crash_at(p, |pt| promote(&s.home, FROM, TO, ID, pt));
        assert!(outcome.is_none(), "{p} was reached");
        match p {
            "promote:S1" => {
                assert!(s.home.join("members.last.new").exists());
                assert_eq!(
                    read_last(&s.home).unwrap(),
                    Some(old_last.clone()),
                    "the older record is intact until S2"
                );
                assert!(update_path(&s).exists());
            }
            "promote:S2" => {
                assert!(update_path(&s).exists() && last_path(&s).exists());
                assert_eq!(read_last(&s.home).unwrap().unwrap().id, ID);
            }
            _ => {
                assert!(!update_path(&s).exists());
                assert_eq!(read_last(&s.home).unwrap().unwrap().id, ID);
            }
        }
        promote(&s.home, FROM, TO, ID, &noop).unwrap_or_else(|e| panic!("after {p}: {e}"));
        let last = read_last(&s.home).unwrap().unwrap();
        assert_eq!(last.id, ID);
        assert!(!update_path(&s).exists() && !s.home.join("members.last.new").exists());
    }
}

#[test]
fn mb_27_a_promotion_refuses_what_is_not_finished_or_not_consistent() {
    // not every entry replaced
    let s = setup("mb27");
    old_set(&s, "v0");
    let rel = release(&s, "v1");
    planned(&s, &rel);
    let before = fs::read(update_path(&s)).unwrap();
    assert!(promote(&s.home, FROM, TO, ID, &noop).is_err());
    assert_eq!(fs::read(update_path(&s)).unwrap(), before);
    assert!(!last_path(&s).exists());
    // the record of another attempt
    assert!(promote(&s.home, FROM, TO, ID2, &noop).is_err());
    // a pair or an id that is not valid is refused by name, before the record is looked at
    for (f, t, i) in [
        ("0.2.001", TO, ID),
        (FROM, "latest", ID),
        (FROM, FROM, ID),
        (FROM, TO, "short"),
    ] {
        let e = promote(&s.home, f, t, i, &noop).unwrap_err().to_string();
        assert!(
            e.contains("release versions")
                || e.contains("the same release")
                || e.contains("attempt id"),
            "{f} {t} {i}: {e}"
        );
    }
    // a FULLY installed record asked to be promoted as another attempt: refused on the attempt, nothing changed
    let (s4, _r4) = installed("mb27d");
    let e = promote(&s4.home, FROM, TO, ID2, &noop)
        .unwrap_err()
        .to_string();
    assert!(e.contains("not of attempt"), "{e}");
    assert!(update_path(&s4).exists() && !last_path(&s4).exists());
    // the entries named as not installed are exactly those not replaced
    let s5 = setup("mb27e");
    old_set(&s5, "v0");
    let rel5 = release(&s5, "v1");
    planned(&s5, &rel5);
    let lost = crash_at("cortex-dump-status:replaced", |pt| {
        install(&layout(&s5, &rel5), FROM, TO, ID, pt)
    });
    assert!(lost.is_none());
    let e = promote(&s5.home, FROM, TO, ID, &noop)
        .unwrap_err()
        .to_string();
    let listed: Vec<&str> = e
        .split('(')
        .nth(1)
        .unwrap()
        .split(')')
        .next()
        .unwrap()
        .split(", ")
        .collect();
    assert!(
        !listed.contains(&"cortex-dump") && !listed.contains(&"cortex-dump-status"),
        "{listed:?}"
    );
    assert!(
        listed.contains(&"cortex-dump-alert") && listed.contains(&"restore-compare"),
        "{listed:?}"
    );
    assert_eq!(listed.len(), 8, "{listed:?}");
    // update and last with the SAME id but different entries: never a pick of one
    let (s2, _r) = installed("mb27b");
    let mut other = read_update(&s2.home).unwrap().unwrap();
    other.entries[0].old = Old::Absent;
    write_last(&s2, &other);
    assert!(promote(&s2.home, FROM, TO, ID, &noop).is_err());
    assert!(update_path(&s2).exists(), "nothing removed");
    // members.update absent: members.last of this attempt means it was done already; of another id, or none, it is a lost inventory
    let s3 = setup("mb27c");
    assert!(
        promote(&s3.home, FROM, TO, ID, &noop).is_err(),
        "nothing at all"
    );
    write_last(&s3, &record_for(ID2, FROM, TO, State::Replaced));
    assert!(
        promote(&s3.home, FROM, TO, ID, &noop).is_err(),
        "another id"
    );
    write_last(&s3, &record_for(ID, FROM, TO, State::Replaced));
    promote(&s3.home, FROM, TO, ID, &noop).unwrap();
}

#[test]
fn mb_28_the_sweep_removes_only_the_exact_backup_names_of_the_retired_attempts() {
    let s = setup("mb28");
    let keep = backup(&s, Kind::Bin, "cortex-dump", ID);
    let gone = [
        backup(&s, Kind::Bin, "cortex-dump", ID2),
        backup(&s, Kind::Bin, "cortex-restore", ID2),
        backup(&s, Kind::Unit, "cortex-dump.timer", ID2),
        backup(
            &s,
            Kind::Unit,
            "cortex-dump-alert.service",
            "0000000000000001",
        ),
    ];
    let foreign = [
        dest(&s, Kind::Bin, "cortex-dump").with_file_name("cortex-dump.prev-0011223344556677aa"),
        dest(&s, Kind::Bin, "cortex-dump").with_file_name("cortex-dump.prev-00112233445566f"),
        dest(&s, Kind::Bin, "cortex-dump").with_file_name("cortex-dump.prev-AABBCCDDEEFF0011"),
        dest(&s, Kind::Bin, "cortex-dump").with_file_name(format!("cortex-dump.prev-{ID2}.bak")),
        dest(&s, Kind::Bin, "cortex-dump").with_file_name(format!("other.prev-{ID2}")),
        dest(&s, Kind::Bin, "cortex-dump").with_file_name("cortex-dump.prev"),
        dest(&s, Kind::Bin, "cortex-dump").with_file_name("cortex-dump.new"),
        dest(&s, Kind::Bin, "cortex-dump").with_file_name(format!("cortex-dump.restore-{ID2}")),
        dest(&s, Kind::Unit, "cortex-dump.timer")
            .with_file_name(format!("x-cortex-dump.timer.prev-{ID2}")),
    ];
    for p in std::iter::once(&keep)
        .chain(gone.iter())
        .chain(foreign.iter())
    {
        fs::write(p, "x").unwrap();
    }
    let retired = [ID2, "0000000000000001"];
    let removed = sweep_retired(&s.home, &s.units, &retired, &[], &noop).unwrap();
    assert_eq!(removed.len(), 4, "{removed:?}");
    for p in &gone {
        assert!(!p.exists(), "{} removed", p.display());
    }
    assert!(keep.exists(), "an id that is not named stays");
    for p in &foreign {
        assert!(p.exists(), "{} is not a backup name and stays", p.display());
    }
    assert!(
        sweep_retired(&s.home, &s.units, &retired, &[], &noop)
            .unwrap()
            .is_empty(),
        "idempotent"
    );
    // an id that is named AND protected (members.last's, an unfinished upgrade's, this attempt's) is never removed
    let both = backup(&s, Kind::Bin, "cortex-dump", ID2);
    fs::write(&both, "x").unwrap();
    assert!(
        sweep_retired(&s.home, &s.units, &[ID, ID2], &[ID, ID2], &noop)
            .unwrap()
            .is_empty()
    );
    assert!(both.exists());
}

#[test]
fn mb_29_a_plan_of_an_attempt_that_never_switched_is_discarded_only_while_nothing_happened() {
    let s = setup("mb29");
    old_set(&s, "v0");
    let rel = release(&s, "v1");
    planned(&s, &rel);
    assert!(
        !discard_plan(&s.home, ID2).unwrap(),
        "another attempt's plan is not ours"
    );
    assert!(update_path(&s).exists());
    assert!(discard_plan(&s.home, ID).unwrap());
    assert!(!update_path(&s).exists());
    assert!(
        !discard_plan(&s.home, ID).unwrap(),
        "nothing left to discard"
    );
    // an entry that has moved past planned: refused, kept
    let (s2, _r) = installed("mb29b");
    assert!(discard_plan(&s2.home, ID).is_err());
    assert!(update_path(&s2).exists());
}

#[test]
fn mb_30_only_the_ten_names_and_their_attempt_suffixes_are_ever_written() {
    let (s, rel) = installed("mb30");
    let inv = live_inventory(&s);
    promote(&s.home, FROM, TO, ID, &noop).unwrap();
    let allowed_home = ["bin", "releases", "members.last"];
    for e in fs::read_dir(&s.home).unwrap().flatten() {
        let n = e.file_name().to_string_lossy().to_string();
        assert!(
            allowed_home.contains(&n.as_str()),
            "unexpected {n} in the home"
        );
    }
    for (dir, set) in [
        (s.home.join("bin"), BIN_FILES.to_vec()),
        (s.units.clone(), UNIT_FILES.to_vec()),
    ] {
        for e in fs::read_dir(&dir).unwrap().flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            let base = n.split(".prev-").next().unwrap();
            assert!(set.contains(&base), "unexpected {n} in {}", dir.display());
        }
    }
    let _ = (inv, rel);
}

#[test]
fn mb_31_the_lines_of_the_upgrade_record_name_each_entry_created_or_replaced() {
    let (r, _) = golden_record();
    let lines = info_lines(&r);
    assert_eq!(
        lines,
        vec![
            "bin cortex-dump replaced".to_string(),
            "bin cortex-dump-status created".to_string(),
            "unit cortex-dump.timer replaced".to_string(),
            "unit cortex-dump-alert.service replaced".to_string(),
        ]
    );
}

#[test]
fn mb_33_a_sweep_lost_in_the_middle_is_finished_by_the_next_one_and_keeps_what_it_keeps() {
    let s = setup("mb33");
    let keep = backup(&s, Kind::Bin, "cortex-dump", ID);
    let gone = [
        backup(&s, Kind::Bin, "cortex-dump-status", ID2),
        backup(&s, Kind::Bin, "cortex-restore", ID2),
        backup(&s, Kind::Unit, "cortex-dump.timer", ID2),
    ];
    for p in std::iter::once(&keep).chain(gone.iter()) {
        fs::write(p, "x").unwrap();
    }
    let first = format!(
        "sweep:removed:{}",
        gone[0].file_name().unwrap().to_string_lossy()
    );
    let lost = crash_at(&first, |pt| {
        sweep_retired(&s.home, &s.units, &[ID2], &[], pt)
    });
    assert!(lost.is_none(), "the sweep was lost after its first removal");
    assert!(keep.exists());
    let removed = sweep_retired(&s.home, &s.units, &[ID2], &[], &noop).unwrap();
    assert!(removed.len() <= 3);
    for p in &gone {
        assert!(
            !p.exists(),
            "{} is gone after the second sweep",
            p.display()
        );
    }
    assert!(keep.exists(), "the kept id stays");
}

#[test]
fn mb_34_a_record_file_that_is_not_a_plain_file_of_a_record_size_is_refused() {
    let s = setup("mb34");
    let (r, _) = golden_record();
    // a directory, a link to a good record, a file larger than a record
    fs::create_dir(update_path(&s)).unwrap();
    let e = read_update(&s.home).unwrap_err().to_string();
    assert!(
        e.contains("members.update") && e.contains("plain file"),
        "{e}"
    );
    fs::remove_dir(update_path(&s)).unwrap();
    let real = s.tmp.path().join("real-record");
    fs::write(&real, serialize(&r)).unwrap();
    symlink(&real, update_path(&s)).unwrap();
    let e = read_update(&s.home).unwrap_err().to_string();
    assert!(e.contains("plain file"), "a link is never followed: {e}");
    fs::remove_file(update_path(&s)).unwrap();
    fs::write(update_path(&s), "x".repeat(16 * 1024 + 1)).unwrap();
    assert!(read_update(&s.home).is_err(), "17 KiB");
    // a record of exactly 16 KiB is read (and refused for what it holds), not refused for its size
    fs::write(update_path(&s), "x".repeat(16 * 1024)).unwrap();
    let e = read_update(&s.home).unwrap_err().to_string();
    assert!(
        !e.contains("plain file"),
        "exactly 16 KiB passes the size gate: {e}"
    );
    // not text
    fs::write(update_path(&s), [0xffu8, 0xfe]).unwrap();
    assert!(
        read_update(&s.home)
            .unwrap_err()
            .to_string()
            .contains("not text")
    );
}

#[test]
fn mb_35_a_stale_temporary_and_a_wrong_backup_of_this_attempts_name_do_not_stop_the_install() {
    let s = setup("mb35");
    old_set(&s, "v0");
    let rel = release(&s, "v1");
    // a lost run left the temporary name of the record (this process's pid) and a backup of THIS attempt's name that is not the old file
    fs::write(
        s.home
            .join(format!("members.update.tmp{}", std::process::id())),
        "left by a lost run\n",
    )
    .unwrap();
    planned(&s, &rel);
    fs::write(
        backup(&s, Kind::Bin, "cortex-dump", ID),
        "not the old file\n",
    )
    .unwrap();
    install(&layout(&s, &rel), FROM, TO, ID, &noop).unwrap();
    assert_installed(&s, "v1");
    assert_eq!(
        fs::read(backup(&s, Kind::Bin, "cortex-dump", ID)).unwrap(),
        bin_text("cortex-dump", "v0"),
        "the wrong backup was replaced by the right one"
    );
}

#[test]
fn mb_36_an_install_refuses_another_attempt_another_pair_a_bad_pair_a_bad_home_a_bad_unit_directory()
 {
    let s = setup("mb36");
    old_set(&s, "v0");
    let rel = release(&s, "v1");
    planned(&s, &rel);
    let l = layout(&s, &rel);
    for (f, t, i, why) in [
        (FROM, TO, ID2, "not of attempt"),
        ("0.2.1-rc.0", TO, ID, "not of attempt"),
        (FROM, "0.2.1-rc.3", ID, "not of attempt"),
        (FROM, TO, "short", "attempt id"),
        (FROM, FROM, ID, "the same release"),
    ] {
        let e = install(&l, f, t, i, &noop).unwrap_err().to_string();
        assert!(e.contains(why), "{f} {t} {i}: {e}");
    }
    assert_installed(&s, "v0");
    // a home the units cannot carry, a unit directory that is not ours to write into
    let bad_home = s.tmp.path().join("a home");
    fs::create_dir_all(bad_home.join("bin")).unwrap();
    let lh = Layout {
        release_dir: &rel,
        home: &bad_home,
        unit_dir: &s.units,
        db_container: DB_CONTAINER,
    };
    assert!(
        install(&lh, FROM, TO, ID, &noop)
            .unwrap_err()
            .to_string()
            .contains("holds a character")
    );
    assert!(
        plan(&lh, FROM, TO, ID, &noop)
            .unwrap_err()
            .to_string()
            .contains("holds a character")
    );
    fs::set_permissions(&s.units, fs::Permissions::from_mode(0o777)).unwrap();
    assert!(
        install(&l, FROM, TO, ID, &noop)
            .unwrap_err()
            .to_string()
            .contains("writable by group or other")
    );
    fs::set_permissions(&s.units, fs::Permissions::from_mode(0o700)).unwrap();
    // a plan of the same attempt asked for another pair of releases is refused, not returned
    let e = plan(&l, "0.2.1-rc.0", TO, ID, &noop)
        .unwrap_err()
        .to_string();
    assert!(e.contains("another pair of releases"), "{e}");
}

#[test]
fn mb_37_a_release_member_that_is_a_link_or_too_large_is_refused() {
    let s = setup("mb37");
    old_set(&s, "v0");
    let rel = release(&s, "v1");
    let victim = rel.join("bin").join("cortex-restore");
    fs::write(&victim, vec![b'x'; 256 * 1024 + 1]).unwrap();
    assert!(
        plan(&layout(&s, &rel), FROM, TO, ID, &noop)
            .unwrap_err()
            .to_string()
            .contains("too large")
    );
    fs::write(&victim, vec![b'x'; 256 * 1024]).unwrap();
    plan(&layout(&s, &rel), FROM, TO, ID, &noop).expect("exactly 256 KiB is a member");
    fs::remove_file(update_path(&s)).unwrap();
    let real = s.tmp.path().join("elsewhere");
    fs::write(&real, "x").unwrap();
    fs::remove_file(&victim).unwrap();
    symlink(&real, &victim).unwrap();
    assert!(
        plan(&layout(&s, &rel), FROM, TO, ID, &noop)
            .unwrap_err()
            .to_string()
            .contains("not a regular file")
    );
}

#[test]
fn mb_38_a_backup_that_is_not_a_regular_file_of_the_recorded_mode_is_not_one() {
    for what in ["a link", "a directory", "another mode"] {
        let (s, _rel) = installed(&format!("mb38-{}", what.replace(' ', "-")));
        let b = backup(&s, Kind::Bin, "cortex-dump-alert", ID);
        fs::remove_file(&b).unwrap();
        match what {
            "a link" => symlink(dest(&s, Kind::Bin, "cortex-dump-alert"), &b).unwrap(),
            "a directory" => fs::create_dir(&b).unwrap(),
            _ => {
                fs::write(&b, bin_text("cortex-dump-alert", "v0")).unwrap();
                fs::set_permissions(&b, fs::Permissions::from_mode(0o600)).unwrap();
            }
        }
        let inv = live_inventory(&s);
        let e = restore(&s.home, &s.units, &inv, &noop)
            .unwrap_err()
            .to_string();
        assert!(e.contains("cortex-dump-alert"), "{what}: {e}");
        match what {
            "another mode" => assert!(e.contains("another mode"), "{what}: {e}"),
            _ => assert!(e.contains("not a regular file"), "{what}: {e}"),
        }
    }
}

#[test]
fn mb_39_a_destination_that_changed_after_the_plan_is_refused_by_kind_by_mode_and_in_every_state() {
    // the mode alone changed (the bytes are the recorded ones)
    let s = setup("mb39a");
    old_set(&s, "v0");
    let rel = release(&s, "v1");
    planned(&s, &rel);
    let d = dest(&s, Kind::Bin, "cortex-dump-status");
    fs::set_permissions(&d, fs::Permissions::from_mode(0o700)).unwrap();
    let e = install(&layout(&s, &rel), FROM, TO, ID, &noop)
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("cortex-dump-status") && e.contains("changed after the plan"),
        "{e}"
    );
    // an absent entry that is a file now
    let s = setup("mb39b");
    old_set(&s, "v0");
    let rel = release(&s, "v1");
    let absent = dest(&s, Kind::Unit, "cortex-dump-alert.service");
    fs::remove_file(&absent).unwrap();
    planned(&s, &rel);
    fs::write(&absent, "appeared after the plan\n").unwrap();
    let e = install(&layout(&s, &rel), FROM, TO, ID, &noop)
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("cortex-dump-alert.service") && e.contains("changed after the plan"),
        "{e}"
    );
    // a link entry whose target changed
    let s = setup("mb39d");
    old_set(&s, "v0");
    let rel = release(&s, "v1");
    let rc = dest(&s, Kind::Bin, "restore-compare");
    fs::remove_file(&rc).unwrap();
    let t1 = s.tmp.path().join("t1");
    fs::write(&t1, "x").unwrap();
    symlink(&t1, &rc).unwrap();
    planned(&s, &rel);
    fs::remove_file(&rc).unwrap();
    symlink(s.tmp.path().join("t2"), &rc).unwrap();
    let e = install(&layout(&s, &rel), FROM, TO, ID, &noop)
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("restore-compare") && e.contains("changed after the plan"),
        "{e}"
    );
    // a backed-up entry whose destination is neither the old file nor the new one
    let s = setup("mb39c");
    old_set(&s, "v0");
    let rel = release(&s, "v1");
    planned(&s, &rel);
    let lost = crash_at("cortex-dump:backed-up", |pt| {
        install(&layout(&s, &rel), FROM, TO, ID, pt)
    });
    assert!(lost.is_none());
    fs::write(dest(&s, Kind::Bin, "cortex-dump"), "a third thing\n").unwrap();
    let e = install(&layout(&s, &rel), FROM, TO, ID, &noop)
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("cortex-dump") && e.contains("neither the old file nor the new one"),
        "{e}"
    );
}

#[test]
fn mb_40_a_staging_file_of_a_lost_restore_does_not_stop_a_link_from_being_put_back() {
    let s = setup("mb40");
    old_set(&s, "v0");
    let rel = release(&s, "v1");
    let target = s.tmp.path().join("elsewhere");
    fs::write(&target, "x").unwrap();
    let rc = dest(&s, Kind::Bin, "restore-compare");
    fs::remove_file(&rc).unwrap();
    symlink(&target, &rc).unwrap();
    planned(&s, &rel);
    install(&layout(&s, &rel), FROM, TO, ID, &noop).unwrap();
    // a lost restore left this attempt's staging name for the link in the way
    fs::write(sibling_of(&rc, &format!(".restore-{ID}")), "left behind").unwrap();
    let inv = live_inventory(&s);
    restore(&s.home, &s.units, &inv, &noop).unwrap();
    assert_eq!(fs::read_link(&rc).unwrap(), target, "the link is back");
    assert!(!sibling_of(&rc, &format!(".restore-{ID}")).exists());
}

#[test]
fn mb_41_a_restore_removes_the_new_file_a_lost_install_left_beside_a_destination() {
    let s = setup("mb41");
    old_set(&s, "v0");
    let rel = release(&s, "v1");
    planned(&s, &rel);
    let lost = crash_at("cortex-restore:new-written", |pt| {
        install(&layout(&s, &rel), FROM, TO, ID, pt)
    });
    assert!(lost.is_none());
    let leftover = sibling_of(&dest(&s, Kind::Bin, "cortex-restore"), ".new");
    assert!(leftover.exists(), "the lost install left its .new");
    let inv = live_inventory(&s);
    restore(&s.home, &s.units, &inv, &noop).unwrap();
    assert_installed(&s, "v0");
    assert!(!leftover.exists(), "and the restore removed it");
}

#[test]
fn mb_42_a_live_record_of_another_attempt_or_pair_is_a_lost_inventory_even_with_no_members_last() {
    // MB-1145: mb_23 wrote the other attempt's record while an older members.last (same id) was there, so the refusal came from the "do not agree" rule and the
    // comparison of from, to and id was never the only thing standing; here nothing else is there
    let s = setup("mb42");
    let m = Some((FROM, TO, ID));
    for (label, rec) in [
        ("another id", record_for(ID2, FROM, TO, State::Replaced)),
        ("another from", record_for(ID, "0.2.0", TO, State::Replaced)),
        (
            "another to",
            record_for(ID, FROM, "0.2.1-rc.3", State::Replaced),
        ),
    ] {
        write_update(&s, &rec);
        let text = why(inventory(&s.home, m, None));
        assert!(
            text.contains("not of the unfinished upgrade") && text.contains("is lost"),
            "{label}: {text}"
        );
    }
}

#[test]
fn mb_43_a_backup_with_the_recorded_mode_and_other_bytes_is_refused_by_its_hash_alone() {
    // MB-1086: mb_20 re-created the backup with fs::write, which gives another mode, so the MODE check refused it and the hash check was never alone
    let (s, _rel) = installed("mb43");
    let victim = backup(&s, Kind::Bin, "cortex-dump-alert", ID);
    fs::remove_file(&victim).unwrap();
    fs::write(&victim, "other bytes, the recorded mode\n").unwrap();
    fs::set_permissions(&victim, fs::Permissions::from_mode(mode_of(Kind::Bin))).unwrap();
    assert_eq!(
        mode(&victim),
        mode_of(Kind::Bin),
        "the mode is the recorded one"
    );
    let inv = live_inventory(&s);
    let text = restore(&s.home, &s.units, &inv, &noop)
        .unwrap_err()
        .to_string();
    assert!(
        text.contains("does not hold the bytes that were recorded"),
        "{text}"
    );
    for (k, n) in names() {
        assert!(backup(&s, k, n, ID).exists(), "{n}: the backup is kept");
    }
    assert!(update_path(&s).exists(), "the record stays");
}

#[test]
fn mb_44_forget_removes_the_live_record_and_a_members_last_of_the_same_attempt_and_nothing_else() {
    // MB-1182, MB-1184, MB-1186: nothing read a crash after S2 (members.last of this attempt next to members.update) through forget alone
    let mk = |tag: &str| {
        let s = setup(tag);
        write_update(&s, &record_for(ID, FROM, TO, State::Replaced));
        s
    };
    let m = Some((FROM, TO, ID));
    // (a) members.update and a members.last of the SAME attempt (S2 done, S3 not): both go
    let s = mk("mb44a");
    write_last(&s, &record_for(ID, FROM, TO, State::Replaced));
    let inv = inventory(&s.home, m, None).unwrap().unwrap();
    assert_eq!(inv.source, Source::Update);
    forget(&s.home, &inv).unwrap();
    assert!(!update_path(&s).exists(), "the live record is gone");
    assert!(
        !last_path(&s).exists(),
        "the same attempt's members.last is gone too"
    );
    // (b) a members.last of ANOTHER attempt is the previous upgrade's record and stays untouched
    let s = mk("mb44b");
    let older = record_for(ID2, "0.2.0", FROM, State::Replaced);
    write_last(&s, &older);
    let inv = inventory(&s.home, m, None).unwrap().unwrap();
    forget(&s.home, &inv).unwrap();
    assert!(!update_path(&s).exists());
    assert_eq!(
        read_last(&s.home).unwrap().unwrap(),
        older,
        "another attempt's members.last is kept as it was"
    );
    // (c) the inventory read from members.last (S3 done): that file goes
    let s = setup("mb44c");
    write_last(&s, &record_for(ID, FROM, TO, State::Replaced));
    let inv = inventory(&s.home, m, None).unwrap().unwrap();
    assert_eq!(inv.source, Source::Last);
    forget(&s.home, &inv).unwrap();
    assert!(!last_path(&s).exists());
}

// ---- packet 18 (cox W1-74, W1-75; mike's rework of packet 17): a deletion happens ONLY on a trusted record that POSITIVELY names the backup as retired

fn rid(n: u64) -> String {
    format!("{n:016x}")
}

#[test]
fn mb_45_a_record_carries_its_retired_ids_in_one_exact_form_and_refuses_every_other() {
    let r = Record {
        from: FROM.to_string(),
        to: TO.to_string(),
        id: ID.to_string(),
        retired: vec![rid(1), rid(2)],
        entries: Vec::new(),
    };
    let text = serialize(&r);
    assert_eq!(
        text,
        format!(
            "members-record 1\nfrom={FROM}\nto={TO}\nid={ID}\nretired={}\nretired={}\n",
            rid(1),
            rid(2)
        )
    );
    assert_eq!(parse(&text).unwrap(), r);
    let head = format!("members-record 1\nfrom={FROM}\nto={TO}\nid={ID}\n");
    let entry = "entry bin cortex-dump absent replaced\n";
    let eight: String = (1..=8).map(|n| format!("retired={}\n", rid(n))).collect();
    let nine = format!("{eight}retired={}\n", rid(9));
    assert_eq!(RETIRED_MAX, 8);
    assert!(
        parse(&format!("{head}{eight}")).is_ok(),
        "eight are allowed"
    );
    for (what, bad) in [
        ("nine", format!("{head}{nine}")),
        (
            "an id of 15 characters",
            format!("{head}retired={}\n", &rid(1)[1..]),
        ),
        (
            "an upper-case id",
            format!(
                "{head}retired={}\n",
                rid(0xabc).to_uppercase().replace('0', "A")
            ),
        ),
        (
            "an id that is not hex",
            format!("{head}retired=000000000000000g\n"),
        ),
        ("an empty id", format!("{head}retired=\n")),
        ("the record's own id", format!("{head}retired={ID}\n")),
        (
            "the same id twice",
            format!("{head}retired={}\nretired={}\n", rid(1), rid(1)),
        ),
        (
            "a retired line after an entry",
            format!("{head}{entry}retired={}\n", rid(1)),
        ),
        (
            "a retired line before the id",
            format!(
                "members-record 1\nfrom={FROM}\nto={TO}\nretired={}\nid={ID}\n",
                rid(1)
            ),
        ),
        (
            "a key that is almost retired",
            format!("{head}retired {}\n", rid(1)),
        ),
    ] {
        assert!(parse(&bad).is_err(), "{what} must be refused");
    }
}

#[test]
fn mb_46_a_promotion_retires_the_previous_trusted_attempt_and_names_nothing_else() {
    let prev = |retired: Vec<String>, to: &str, st: State| Record {
        retired,
        ..record_for(ID2, "0.2.0", to, st)
    };
    // a trusted previous record: its own retired ids are carried, its id is added
    let (s, _rel) = installed("mb46a");
    write_last(&s, &prev(vec![rid(9)], FROM, State::Replaced));
    promote(&s.home, FROM, TO, ID, &noop).unwrap();
    let last = read_last(&s.home).unwrap().unwrap();
    assert_eq!(last.id, ID);
    assert_eq!(last.retired, vec![rid(9), ID2.to_string()]);
    // the cap: the oldest falls off the end (its backups are then never named, so kept)
    let (s, _rel) = installed("mb46b");
    let full: Vec<String> = (1..=8).map(rid).collect();
    write_last(&s, &prev(full.clone(), FROM, State::Replaced));
    promote(&s.home, FROM, TO, ID, &noop).unwrap();
    let last = read_last(&s.home).unwrap().unwrap();
    let mut want: Vec<String> = full[1..].to_vec();
    want.push(ID2.to_string());
    assert_eq!(last.retired, want);
    assert_eq!(last.retired.len(), RETIRED_MAX);
    // the new attempt's own id is never in its retired list, whatever the previous record carried
    let (s, _rel) = installed("mb46h");
    write_last(
        &s,
        &prev(vec![ID.to_string(), rid(9)], FROM, State::Replaced),
    );
    promote(&s.home, FROM, TO, ID, &noop).unwrap();
    assert_eq!(
        read_last(&s.home).unwrap().unwrap().retired,
        vec![rid(9), ID2.to_string()]
    );
    // nothing is named when the previous record cannot be relied on: corrupt, absent, not bound to the release being left, not complete
    let (s, _rel) = installed("mb46c");
    fs::write(last_path(&s), "garbage\n").unwrap();
    promote(&s.home, FROM, TO, ID, &noop).unwrap();
    assert!(
        read_last(&s.home).unwrap().unwrap().retired.is_empty(),
        "corrupt"
    );
    let (s, _rel) = installed("mb46d");
    promote(&s.home, FROM, TO, ID, &noop).unwrap();
    assert!(
        read_last(&s.home).unwrap().unwrap().retired.is_empty(),
        "absent"
    );
    let (s, _rel) = installed("mb46e");
    // (a record whose from equals its to is not a valid one: the case must use a VALID record that is bound to another release, or it proves nothing)
    write_last(&s, &prev(vec![rid(9)], "0.2.1-rc.0", State::Replaced));
    assert!(
        read_last(&s.home).is_ok(),
        "the unbound record is a valid one"
    );
    promote(&s.home, FROM, TO, ID, &noop).unwrap();
    assert!(
        read_last(&s.home).unwrap().unwrap().retired.is_empty(),
        "not bound"
    );
    let (s, _rel) = installed("mb46f");
    write_last(&s, &prev(vec![rid(9)], FROM, State::BackedUp));
    promote(&s.home, FROM, TO, ID, &noop).unwrap();
    assert!(
        read_last(&s.home).unwrap().unwrap().retired.is_empty(),
        "not complete"
    );
    // a promotion lost after its rename (S2) and run again keeps what it published: the retired list is NOT recomputed from the record it replaced
    let (s, _rel) = installed("mb46g");
    write_last(&s, &prev(vec![rid(9)], FROM, State::Replaced));
    assert!(crash_at("promote:S2", |pt| promote(&s.home, FROM, TO, ID, pt)).is_none());
    let published = read_last(&s.home).unwrap().unwrap();
    assert_eq!(published.retired, vec![rid(9), ID2.to_string()]);
    assert!(update_path(&s).exists(), "S3 was not reached");
    promote(&s.home, FROM, TO, ID, &noop).unwrap();
    assert_eq!(
        read_last(&s.home).unwrap().unwrap(),
        published,
        "kept as published"
    );
    assert!(!update_path(&s).exists(), "S3 done by the second call");
}

#[test]
fn mb_47_members_last_is_trusted_only_when_present_readable_valid_complete_and_bound() {
    let ok = Record {
        retired: vec![rid(1), rid(2)],
        ..record_for(ID, "0.2.0", FROM, State::Replaced)
    };
    let s = setup("mb47");
    assert_eq!(trusted_retired(&s.home, FROM), Trust::Absent, "no file");
    write_last(&s, &ok);
    assert_eq!(
        trusted_retired(&s.home, FROM),
        Trust::Trusted(vec![rid(1), rid(2)])
    );
    let untrusted = |label: &str| match trusted_retired(&s.home, FROM) {
        Trust::Untrusted(why) => why,
        other => panic!("{label}: expected Untrusted, got {other:?}"),
    };
    // bound to the release being left
    assert!(
        matches!(trusted_retired(&s.home, "0.9.9"), Trust::Untrusted(_)),
        "another release"
    );
    // not complete
    write_last(
        &s,
        &Record {
            entries: record_for(ID, "0.2.0", FROM, State::BackedUp).entries,
            ..ok.clone()
        },
    );
    untrusted("an entry that is not replaced");
    // well formed lines with an id that is not valid
    fs::write(
        last_path(&s),
        format!("members-record 1\nfrom=0.2.0\nto={FROM}\nid=NOT-AN-ID\n"),
    )
    .unwrap();
    untrusted("an invalid id");
    // corrupt
    fs::write(last_path(&s), "garbage\n").unwrap();
    untrusted("corrupt");
    // unreadable by mode
    write_last(&s, &ok);
    fs::set_permissions(last_path(&s), fs::Permissions::from_mode(0o000)).unwrap();
    untrusted("unreadable by mode");
    fs::set_permissions(last_path(&s), fs::Permissions::from_mode(0o600)).unwrap();
    // a link is not a record
    fs::remove_file(last_path(&s)).unwrap();
    symlink("/etc/hostname", last_path(&s)).unwrap();
    untrusted("a link");
}

#[test]
fn mb_48_the_sweep_removes_only_the_ids_a_trusted_record_retired_and_never_a_protected_one() {
    let s = setup("mb48");
    let retired = [
        backup(&s, Kind::Bin, "cortex-dump", ID2),
        backup(&s, Kind::Bin, "cortex-restore", ID2),
        backup(&s, Kind::Unit, "cortex-dump.timer", ID2),
    ];
    let unlisted = [
        backup(&s, Kind::Bin, "cortex-dump", "0000000000000001"),
        backup(
            &s,
            Kind::Unit,
            "cortex-dump-alert.service",
            "0000000000000002",
        ),
    ];
    let protected = backup(&s, Kind::Bin, "cortex-dump-status", ID);
    let foreign = [
        dest(&s, Kind::Bin, "cortex-dump").with_file_name(format!("cortex-dump.prev-{ID2}.bak")),
        dest(&s, Kind::Bin, "cortex-dump").with_file_name("cortex-dump.prev-aabbccddeeff001"),
        dest(&s, Kind::Bin, "cortex-dump").with_file_name(format!("other.prev-{ID2}")),
        dest(&s, Kind::Unit, "cortex-dump.timer")
            .with_file_name(format!("x-cortex-dump.timer.prev-{ID2}")),
    ];
    for p in retired
        .iter()
        .chain(unlisted.iter())
        .chain(foreign.iter())
        .chain(std::iter::once(&protected))
    {
        fs::write(p, "x").unwrap();
    }
    let removed = sweep_retired(&s.home, &s.units, &[ID2, ID], &[ID], &noop).unwrap();
    assert_eq!(removed.len(), 3, "{removed:?}");
    for p in &retired {
        assert!(!p.exists(), "{} was retired and goes", p.display());
    }
    for p in unlisted
        .iter()
        .chain(foreign.iter())
        .chain(std::iter::once(&protected))
    {
        assert!(
            p.exists(),
            "{} is not named (or protected, or not a backup name) and stays",
            p.display()
        );
    }
    assert!(
        sweep_retired(&s.home, &s.units, &[ID2, ID], &[ID], &noop)
            .unwrap()
            .is_empty(),
        "idempotent"
    );
    assert!(
        sweep_retired(&s.home, &s.units, &[], &[], &noop)
            .unwrap()
            .is_empty(),
        "an empty list removes nothing"
    );
    // lost in the middle, finished by the next call, the rest untouched
    let again = [
        backup(&s, Kind::Bin, "cortex-dump-status", ID2),
        backup(&s, Kind::Bin, "cortex-restore", ID2),
        backup(&s, Kind::Unit, "cortex-dump.timer", ID2),
    ];
    for p in &again {
        fs::write(p, "x").unwrap();
    }
    let first = format!(
        "sweep:removed:{}",
        again[0].file_name().unwrap().to_string_lossy()
    );
    assert!(
        crash_at(&first, |pt| sweep_retired(
            &s.home,
            &s.units,
            &[ID2],
            &[],
            pt
        ))
        .is_none()
    );
    sweep_retired(&s.home, &s.units, &[ID2], &[], &noop).unwrap();
    for p in &again {
        assert!(!p.exists(), "{} gone after the second call", p.display());
    }
    for p in &unlisted {
        assert!(p.exists(), "still not named, still there");
    }
}

#[test]
fn mb_49_the_recognised_backups_are_listed_by_their_exact_names_alone() {
    let s = setup("mb49");
    assert!(recognised_backups(&s.home, &s.units).is_empty());
    let a = backup(&s, Kind::Bin, "cortex-dump", ID);
    let b = backup(&s, Kind::Unit, "cortex-dump.timer", ID2);
    fs::write(&a, "x").unwrap();
    fs::write(&b, "x").unwrap();
    fs::write(
        dest(&s, Kind::Bin, "cortex-dump").with_file_name(format!("cortex-dump.prev-{ID}.bak")),
        "x",
    )
    .unwrap();
    fs::write(
        dest(&s, Kind::Bin, "cortex-dump").with_file_name("cortex-dump.new"),
        "x",
    )
    .unwrap();
    let want: Vec<String> = {
        let mut v = vec![
            a.file_name().unwrap().to_string_lossy().to_string(),
            b.file_name().unwrap().to_string_lossy().to_string(),
        ];
        v.sort();
        v
    };
    assert_eq!(recognised_backups(&s.home, &s.units), want);
}

#[test]
fn mb_50_a_restore_into_a_unit_directory_that_is_not_ours_to_write_changes_nothing() {
    // the destination is validated before any write, in a restore as in a plan and an install (cox W1-74 (2) and its deletion table)
    let (s, _rel) = installed("mb50");
    let inv = live_inventory(&s);
    fs::set_permissions(&s.units, fs::Permissions::from_mode(0o775)).unwrap();
    let before: Vec<Vec<u8>> = names()
        .iter()
        .map(|(k, n)| fs::read(dest(&s, *k, n)).unwrap())
        .collect();
    let e = restore(&s.home, &s.units, &inv, &noop)
        .unwrap_err()
        .to_string();
    assert!(e.contains("unit directory") || e.contains("units"), "{e}");
    for (i, (k, n)) in names().iter().enumerate() {
        assert_eq!(
            fs::read(dest(&s, *k, n)).unwrap(),
            before[i],
            "{n} untouched"
        );
        assert!(backup(&s, *k, n, ID).exists(), "{n}: the backup is kept");
    }
    assert!(update_path(&s).exists(), "the record stays");
}

#[test]
fn mb_51_a_restore_with_a_home_the_units_cannot_carry_is_refused_before_it_writes() {
    // PH11: the home is checked first in a restore as in a plan and an install
    let (s, _rel) = installed("mb51");
    let inv = live_inventory(&s);
    let bad_home = PathBuf::from("/nonexistent home with spaces");
    let text = restore(&bad_home, &s.units, &inv, &noop)
        .unwrap_err()
        .to_string();
    assert!(text.contains("holds a character other than"), "{text}");
    for (k, n) in names() {
        assert!(backup(&s, k, n, ID).exists(), "{n}: the backup is kept");
    }
    assert!(update_path(&s).exists(), "the record stays");
}

// ---- packet 19 (cox W1-87; mike's REWORK of packet 18): the same-attempt S2 resume compares STATES and completeness, and a refusal changes nothing

#[test]
fn mb_52_a_same_attempt_members_last_that_is_not_complete_is_never_taken_for_the_published_record()
{
    // mike's probe on packet 18: a same-attempt members.last with ONE entry planned next to a members.update with every entry replaced was "the same record" (same_content
    // ignored the states): promote deleted the good live record and left the incomplete one as the only inventory, which a rollback then refused
    type Mutate = fn(&mut Record);
    let forms: [(&str, Mutate); 5] = [
        ("the first entry planned", |r| {
            r.entries[0].state = State::Planned
        }),
        ("a middle entry backed-up", |r| {
            r.entries[4].state = State::BackedUp
        }),
        ("the last entry restored", |r| {
            r.entries[9].state = State::Restored
        }),
        ("every entry planned", |r| {
            r.entries.iter_mut().for_each(|e| e.state = State::Planned)
        }),
        ("every entry restored", |r| {
            r.entries.iter_mut().for_each(|e| e.state = State::Restored)
        }),
    ];
    for (n, (label, mutate)) in forms.iter().enumerate() {
        let (s, _rel) = installed(&format!("mb52-{n}"));
        let mut last = read_update(&s.home).unwrap().unwrap();
        assert_eq!(
            last.entries.len(),
            10,
            "{label}: the fixture holds the ten members"
        );
        mutate(&mut last);
        write_last(&s, &last);
        let update_before = fs::read(update_path(&s)).unwrap();
        let last_before = fs::read(last_path(&s)).unwrap();
        let e = promote(&s.home, FROM, TO, ID, &noop)
            .unwrap_err()
            .to_string();
        assert!(e.contains("do not agree"), "{label}: {e}");
        assert_eq!(
            fs::read(update_path(&s)).unwrap(),
            update_before,
            "{label}: the live record is untouched"
        );
        assert_eq!(
            fs::read(last_path(&s)).unwrap(),
            last_before,
            "{label}: so is members.last"
        );
        // the rollback is NOT stranded: the inventory is still the live record (every entry replaced) and it puts the old files back
        let inv = live_inventory(&s);
        assert_eq!(inv.source, Source::Update, "{label}");
        restore(&s.home, &s.units, &inv, &noop).unwrap();
        for (k, name) in names() {
            assert_eq!(
                fs::read(dest(&s, k, name)).unwrap(),
                text_of(&s, k, name, "v0"),
                "{label}: {name} is back as it was"
            );
        }
    }
}

#[test]
fn mb_53_a_promotion_that_refuses_changes_nothing_not_even_a_leftover_temporary() {
    // mike's accuracy point on the deletion table: promote removed `members.last.new` BEFORE it read members.update, so a refusal did change something
    let (s, _rel) = installed("mb53");
    let leftover = s.home.join("members.last.new");
    let mut half = read_update(&s.home).unwrap().unwrap();
    half.entries[3].state = State::BackedUp;
    write_update(&s, &half);
    fs::write(&leftover, "a leftover of a lost promotion").unwrap();
    let before = fs::read(update_path(&s)).unwrap();
    let e = promote(&s.home, FROM, TO, ID, &noop)
        .unwrap_err()
        .to_string();
    assert!(e.contains("not every member is installed yet"), "{e}");
    assert!(
        leftover.exists(),
        "a refusal leaves even the temporary where it was"
    );
    assert_eq!(fs::read(update_path(&s)).unwrap(), before);
    assert!(!last_path(&s).exists(), "and publishes nothing");
    // a record of another attempt, an absent record: the same
    let mut other = read_update(&s.home).unwrap().unwrap();
    other.id = ID2.to_string();
    other
        .entries
        .iter_mut()
        .for_each(|e| e.state = State::Replaced);
    write_update(&s, &other);
    assert!(promote(&s.home, FROM, TO, ID, &noop).is_err());
    assert!(
        leftover.exists(),
        "another attempt's record: a refusal, the temporary stays"
    );
    fs::remove_file(update_path(&s)).unwrap();
    assert!(promote(&s.home, FROM, TO, ID, &noop).is_err());
    assert!(
        leftover.exists(),
        "no record at all: a refusal, the temporary stays"
    );
}

#[test]
fn mb_54_a_genuine_resume_after_s2_keeps_the_published_record_and_a_different_record_of_the_same_attempt_is_refused()
 {
    // the combinations of members.last (same attempt) with a complete members.update: equal and complete = finish S3 only and keep what was published (retired list included);
    // a different file hash, mode, or an entry out of the ten = refuse and change nothing
    let (s, _rel) = installed("mb54a");
    write_last(&s, &record_for(ID2, "0.2.0", FROM, State::Replaced));
    assert!(crash_at("promote:S2", |pt| promote(&s.home, FROM, TO, ID, pt)).is_none());
    let published = fs::read(last_path(&s)).unwrap();
    promote(&s.home, FROM, TO, ID, &noop).unwrap();
    assert_eq!(
        fs::read(last_path(&s)).unwrap(),
        published,
        "the published bytes, retired list included, are kept"
    );
    assert!(!update_path(&s).exists());
    let (s, _rel) = installed("mb54b");
    let mut last = read_update(&s.home).unwrap().unwrap();
    last.entries[0].old = Old::File {
        sha256: sha(b"another file"),
        mode: 0o755,
    };
    write_last(&s, &last);
    let (u, l) = (
        fs::read(update_path(&s)).unwrap(),
        fs::read(last_path(&s)).unwrap(),
    );
    let e = promote(&s.home, FROM, TO, ID, &noop)
        .unwrap_err()
        .to_string();
    assert!(e.contains("do not agree"), "{e}");
    assert_eq!(
        (
            fs::read(update_path(&s)).unwrap(),
            fs::read(last_path(&s)).unwrap()
        ),
        (u, l),
        "nothing changed"
    );
}

#[test]
fn mb_55_with_the_live_record_gone_only_a_complete_published_record_of_this_attempt_counts_as_promoted()
 {
    // the S3-done branch of promote: members.update absent. A members.last of this attempt that is complete is "nothing left to do"; one with an entry that is not replaced
    // is not a published record (refused, nothing changed); another attempt's, or none, is a lost inventory
    let (s, _rel) = installed("mb55");
    let mut last = read_update(&s.home).unwrap().unwrap();
    fs::remove_file(update_path(&s)).unwrap();
    write_last(&s, &last);
    promote(&s.home, FROM, TO, ID, &noop).unwrap();
    last.entries[2].state = State::Planned;
    write_last(&s, &last);
    let before = fs::read(last_path(&s)).unwrap();
    let e = promote(&s.home, FROM, TO, ID, &noop)
        .unwrap_err()
        .to_string();
    assert!(e.contains("not complete"), "{e}");
    assert_eq!(fs::read(last_path(&s)).unwrap(), before, "nothing changed");
    last.id = ID2.to_string();
    last.entries[2].state = State::Replaced;
    write_last(&s, &last);
    assert!(
        promote(&s.home, FROM, TO, ID, &noop)
            .unwrap_err()
            .to_string()
            .contains("lost")
    );
    fs::remove_file(last_path(&s)).unwrap();
    assert!(
        promote(&s.home, FROM, TO, ID, &noop)
            .unwrap_err()
            .to_string()
            .contains("lost")
    );
}
