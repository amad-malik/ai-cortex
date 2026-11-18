//! Piece 14, test definitions r4: UP-04 (hostile archives) and the unpack rules of design section 4 step 5. The
//! archives are built here byte by byte (no tar crate): a strict reader must be tested with hand-made headers.

mod common;

use common::*;
use cortex_upgrade::unpack::{UnpackError, UnpackLimits, unpack_release, unpack_release_gz};
use std::fs;
use std::io::Cursor;
use std::os::unix::fs::PermissionsExt;

// ---------------------------------------------------------------- a tiny tar writer

fn octal(buf: &mut [u8], v: u64) {
    let s = format!("{:0width$o}\0", v, width = buf.len() - 1);
    buf.copy_from_slice(s.as_bytes());
}

fn header(name: &[u8], ty: u8, size: u64, mode: u32, link: &[u8], prefix: &[u8]) -> [u8; 512] {
    let mut h = [0u8; 512];
    h[..name.len()].copy_from_slice(name);
    octal(&mut h[100..108], u64::from(mode));
    octal(&mut h[108..116], 0);
    octal(&mut h[116..124], 0);
    octal(&mut h[124..136], size);
    octal(&mut h[136..148], 0);
    h[156] = ty;
    h[157..157 + link.len()].copy_from_slice(link);
    h[257..263].copy_from_slice(b"ustar\0");
    h[263..265].copy_from_slice(b"00");
    h[345..345 + prefix.len()].copy_from_slice(prefix);
    seal(&mut h);
    h
}

/// Checksum: the sum of all bytes with the checksum field read as spaces.
fn seal(h: &mut [u8; 512]) {
    h[148..156].fill(b' ');
    let sum: u32 = h.iter().map(|b| u32::from(*b)).sum();
    let s = format!("{sum:06o}\0 ");
    h[148..156].copy_from_slice(s.as_bytes());
}

fn entry(h: [u8; 512], data: &[u8]) -> Vec<u8> {
    let mut v = h.to_vec();
    v.extend_from_slice(data);
    v.resize(v.len().div_ceil(512) * 512, 0);
    v
}

fn file(name: &str, data: &[u8]) -> Vec<u8> {
    entry(
        header(name.as_bytes(), b'0', data.len() as u64, 0o644, b"", b""),
        data,
    )
}
fn file_mode(name: &str, data: &[u8], mode: u32) -> Vec<u8> {
    entry(
        header(name.as_bytes(), b'0', data.len() as u64, mode, b"", b""),
        data,
    )
}
fn dir(name: &str) -> Vec<u8> {
    entry(header(name.as_bytes(), b'5', 0, 0o755, b"", b""), b"")
}
fn typed(name: &str, ty: u8) -> Vec<u8> {
    entry(
        header(name.as_bytes(), ty, 0, 0o644, b"/etc/passwd", b""),
        b"",
    )
}

fn finish(entries: &[Vec<u8>]) -> Vec<u8> {
    let mut v: Vec<u8> = entries.concat();
    v.extend_from_slice(&[0u8; 1024]);
    v
}

/// The members of list 2 that list 1 did not have (cox R-1/R-4, `pieces/14-cortex-upgrade-MEMBER-LIST-v2-r0.md`): the example config, the backup
/// scripts in `bin/` (0755) and the user units in `systemd/` (0644). EXACT names, no pattern.
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

/// All 13 new entries, in this order: the example, `bin/` and its five files, `systemd/` and its five files; every file holds `data`.
fn new_members_with(data: &[u8]) -> Vec<Vec<u8>> {
    let mut v = vec![file("cortex.toml.example", data), dir("bin/")];
    v.extend(BIN_MEMBERS.iter().map(|n| file_mode(n, data, 0o755)));
    v.push(dir("systemd/"));
    v.extend(UNIT_MEMBERS.iter().map(|n| file(n, data)));
    v
}

/// The names `unpack` returns for those 13 entries.
const NEW_NAMES: [&str; 13] = [
    "cortex.toml.example",
    "bin",
    "bin/cortex-dump",
    "bin/cortex-dump-status",
    "bin/cortex-dump-alert",
    "bin/cortex-restore",
    "bin/restore-compare",
    "systemd",
    "systemd/cortex-dump.service",
    "systemd/cortex-dump.timer",
    "systemd/cortex-dump-status.service",
    "systemd/cortex-dump-status.timer",
    "systemd/cortex-dump-alert.service",
];

fn good() -> Vec<Vec<u8>> {
    let mut v = vec![
        file_mode("cortex", b"#!/bin/sh\necho cortex\n", 0o755),
        file("compose.yml", b"services: {}\n"),
        file("stack.env", b"CORTEX_VERSION=0.2.1-rc.2\n"),
        dir("initdb/"),
        file("initdb/00-cortex-roles.sh", b"#!/bin/sh\n:\n"),
    ];
    v.extend(new_members_with(b"x\n")); // list 2: before db-roles.sh, which stays the last entry
    v.push(file_mode("db-roles.sh", b"#!/bin/sh\n:\n", 0o755));
    v
}

/// A parent directory (so that "nothing else was created" is a plain listing) and the target inside it.
fn target() -> (Tmp, std::path::PathBuf) {
    let t = Tmp::new("unpack");
    let into = t.path().join("stage");
    (t, into)
}

fn unpack(bytes: &[u8], into: &std::path::Path) -> Result<Vec<String>, UnpackError> {
    unpack_release(Cursor::new(bytes.to_vec()), into, &UnpackLimits::default())
}

fn parent_is_empty(t: &Tmp) -> bool {
    fs::read_dir(t.path()).unwrap().next().is_none()
}

// ---------------------------------------------------------------- the good case

#[test]
fn un_01_a_good_release_is_unpacked_with_modes_set_by_us() {
    let (t, into) = target();
    let names = unpack(&finish(&good()), &into).unwrap();
    assert_eq!(
        names,
        [
            [
                "cortex",
                "compose.yml",
                "stack.env",
                "initdb",
                "initdb/00-cortex-roles.sh"
            ]
            .as_slice(),
            NEW_NAMES.as_slice(),
            ["db-roles.sh"].as_slice(),
        ]
        .concat()
    );
    let mode = |p: &str| fs::metadata(into.join(p)).unwrap().permissions().mode() & 0o7777;
    assert_eq!(
        fs::read(into.join("cortex")).unwrap(),
        b"#!/bin/sh\necho cortex\n"
    );
    assert_eq!(
        fs::read(into.join("initdb/00-cortex-roles.sh")).unwrap(),
        b"#!/bin/sh\n:\n"
    );
    assert_eq!(
        (
            mode("cortex"),
            mode("compose.yml"),
            mode("stack.env"),
            mode("initdb"),
            mode("initdb/00-cortex-roles.sh"),
            mode("db-roles.sh")
        ),
        (0o755, 0o644, 0o644, 0o755, 0o644, 0o755)
    );
    assert_eq!(
        fs::metadata(&into).unwrap().permissions().mode() & 0o7777,
        0o700,
        "the staging directory is 0700"
    );
    assert_eq!(fs::read_dir(t.path()).unwrap().count(), 1);
}

#[test]
fn un_02_the_archive_cannot_choose_its_modes_and_initdb_is_made_0755() {
    let (_t, into) = target();
    let mut es = vec![
        file_mode("cortex", b"x", 0o4755),
        file_mode("compose.yml", b"x", 0o777),
        file_mode("stack.env", b"x", 0o600),
        file_mode("db-roles.sh", b"x", 0o4777),
        dir("initdb/"), // R-7: the directory is required; an entry of its own is enough
    ];
    es.extend(new_members_with(b"x")); // list 2 (R-4)
    unpack(&finish(&es), &into).unwrap();
    let mode = |p: &str| fs::metadata(into.join(p)).unwrap().permissions().mode() & 0o7777;
    assert_eq!(
        (
            mode("cortex"),
            mode("compose.yml"),
            mode("stack.env"),
            mode("db-roles.sh")
        ),
        (0o755, 0o644, 0o644, 0o755),
        "no setuid, no group or world write"
    );
    assert!(into.join("initdb").is_dir());
    assert_eq!(
        fs::metadata(into.join("initdb"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o755
    );
    // an initdb file without a directory entry creates the directory
    let (_t, into) = target();
    let mut es = vec![
        file("cortex", b"x"),
        file("compose.yml", b"x"),
        file("stack.env", b"x"),
        file("db-roles.sh", b"x"),
    ];
    es.extend(new_members_with(b"x"));
    es.push(file("initdb/a.sh", b"x"));
    unpack(&finish(&es), &into).unwrap();
    assert_eq!(
        fs::metadata(into.join("initdb"))
            .unwrap()
            .permissions()
            .mode()
            & 0o7777,
        0o755
    );
}

// ---------------------------------------------------------------- R-7 (marv's row 3a900039): initdb is a required member

/// `good()` without its two `initdb` entries (the directory and the file in it).
fn without_initdb() -> Vec<Vec<u8>> {
    let mut es = good();
    es.retain(|e| !e.starts_with(b"initdb"));
    es
}

#[test]
fn un_22_a_release_without_initdb_is_refused_and_the_message_names_it() {
    // the compose file bind-mounts ./initdb into the database, so a release without it could not start: the unpacker and
    // the resumed-release verifier must agree
    let (t, into) = target();
    let es = without_initdb();
    assert_eq!(
        es.len(),
        good().len() - 2,
        "the helper removed exactly the two initdb entries"
    );
    assert_eq!(
        unpack(&finish(&es), &into),
        Err(UnpackError::Missing { name: "initdb" })
    );
    assert!(!into.exists() && parent_is_empty(&t));
    assert_eq!(
        UnpackError::Missing { name: "initdb" }.to_string(),
        "unpack refused: the release lacks `initdb`"
    );
    // control 1 (accepted already): the directory entry alone is enough, and the directory is made empty
    let (_t, into) = target();
    let mut es = without_initdb();
    es.push(dir("initdb/"));
    unpack(&finish(&es), &into).unwrap();
    assert!(into.join("initdb").is_dir());
    assert_eq!(fs::read_dir(into.join("initdb")).unwrap().count(), 0);
    // control 2 (accepted already): a file under initdb/ without a directory entry also makes it
    let (_t, into) = target();
    let mut es = without_initdb();
    es.push(file("initdb/00-cortex-roles.sh", b"x"));
    unpack(&finish(&es), &into).unwrap();
    assert!(into.join("initdb/00-cortex-roles.sh").is_file());
}

// ---------------------------------------------------------------- list 2 (cox R-1/R-4; pieces/14-cortex-upgrade-MEMBER-LIST-v2-r0.md)

/// `good()` without one entry (or, for a directory, without it and every file under it).
fn good_without(name: &str) -> Vec<Vec<u8>> {
    let dir_prefix = format!("{name}/");
    let mut keep = Vec::new();
    for (i, e) in good().into_iter().enumerate() {
        let n = std::str::from_utf8(&e[..100])
            .unwrap()
            .trim_end_matches('\0')
            .to_string();
        let _ = i;
        let is_it = n == name || n == dir_prefix || n.starts_with(&dir_prefix);
        if !is_it {
            keep.push(e);
        }
    }
    keep
}

#[test]
fn un_23_every_member_of_list_2_is_accepted_and_gets_its_mode_from_us() {
    // hostile archive modes on every new member: the unpacker writes bin/ 0755, systemd/ 0644, the example 0644, the directories 0755
    let (_t, into) = target();
    let mut es = vec![
        file("cortex", b"x"),
        file("compose.yml", b"x"),
        file("stack.env", b"x"),
        file("db-roles.sh", b"x"),
        dir("initdb/"),
        file_mode("cortex.toml.example", b"x", 0o4777),
        dir("bin/"),
    ];
    es.extend(BIN_MEMBERS.iter().map(|n| file_mode(n, b"x", 0o600)));
    es.push(dir("systemd/"));
    es.extend(UNIT_MEMBERS.iter().map(|n| file_mode(n, b"x", 0o777)));
    let names = unpack(&finish(&es), &into).unwrap();
    for n in NEW_NAMES {
        assert!(names.iter().any(|x| x == n), "{n} in {names:?}");
    }
    let mode = |p: &str| fs::metadata(into.join(p)).unwrap().permissions().mode() & 0o7777;
    assert_eq!(mode("cortex.toml.example"), 0o644);
    assert_eq!((mode("bin"), mode("systemd")), (0o755, 0o755));
    for n in BIN_MEMBERS {
        assert_eq!(mode(n), 0o755, "{n}");
    }
    for n in UNIT_MEMBERS {
        assert_eq!(mode(n), 0o644, "{n}");
    }
}

#[test]
fn un_24_every_missing_member_of_list_2_is_refused_and_the_message_names_it() {
    // thirteen names, thirteen refusals; a directory counts as missing only when neither it nor a file under it is there
    let required = [
        "cortex.toml.example",
        "bin",
        "systemd",
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
    for name in required {
        let (t, into) = target();
        let es = good_without(name);
        assert!(
            es.len() < good().len(),
            "{name}: the helper removed something"
        );
        let want = required.iter().copied().find(|n| *n == name).unwrap();
        assert_eq!(
            unpack(&finish(&es), &into),
            Err(UnpackError::Missing { name: want }),
            "{name}"
        );
        assert!(
            !into.exists() && parent_is_empty(&t),
            "{name}: nothing left"
        );
    }
    assert_eq!(
        UnpackError::Missing {
            name: "bin/cortex-dump"
        }
        .to_string(),
        "unpack refused: the release lacks `bin/cortex-dump`"
    );
}

#[test]
fn un_25_a_name_that_list_2_does_not_have_is_refused_even_next_to_a_good_member() {
    for extra in [
        "bin/other",
        "bin/cortex-dump.sh",
        "bin/CORTEX-DUMP",
        "bin/cortex-dump/x",
        "bin/.hidden",
        "systemd/other.service",
        "systemd/cortex-dump.timer.bak",
        "systemd/x/y",
        "cortex.toml",
        "cortex.toml.example.bak",
        "lib/x",
        "scripts/cortex-dump",
        "install.sh",
    ] {
        let (t, into) = target();
        let mut es = good();
        let at = es.len(); // the offending entry is the one pushed here, not an earlier member
        es.push(file(extra, b"x"));
        assert_eq!(
            unpack(&finish(&es), &into),
            Err(UnpackError::Forbidden { index: at }),
            "{extra}"
        );
        assert!(
            !into.exists() && parent_is_empty(&t),
            "{extra}: nothing left"
        );
    }
}

#[test]
fn un_25b_a_directory_that_list_2_does_not_have_is_refused() {
    for extra in [
        "lib/",
        "bin/x/",
        "systemd/x/",
        "initdb/x/",
        "bins/",
        "initdb2/",
    ] {
        let (t, into) = target();
        let mut es = good();
        let at = es.len();
        es.push(dir(extra));
        assert_eq!(
            unpack(&finish(&es), &into),
            Err(UnpackError::Forbidden { index: at }),
            "{extra}"
        );
        assert!(
            !into.exists() && parent_is_empty(&t),
            "{extra}: nothing left"
        );
    }
}

#[test]
fn un_26_a_member_of_the_wrong_type_is_refused() {
    // a directory where a file belongs, a file where a directory belongs
    for (name, as_dir) in [
        ("bin/cortex-dump", true),
        ("systemd/cortex-dump.timer", true),
        ("cortex.toml.example", true),
        ("bin", false),
        ("systemd", false),
    ] {
        let (t, into) = target();
        let mut es = good_without(name);
        let at = es.len(); // the wrong-typed entry is the one pushed here
        es.push(if as_dir {
            dir(&format!("{name}/"))
        } else {
            file(name, b"x")
        });
        assert_eq!(
            unpack(&finish(&es), &into),
            Err(UnpackError::Forbidden { index: at }),
            "{name}"
        );
        assert!(
            !into.exists() && parent_is_empty(&t),
            "{name}: nothing left"
        );
    }
}

#[test]
fn un_27_the_directories_of_list_2_may_be_implied_by_a_file_under_them() {
    // as initdb already may: no entry for bin/ or systemd/, only their files
    let (_t, into) = target();
    let mut es = vec![
        file("cortex", b"x"),
        file("compose.yml", b"x"),
        file("stack.env", b"x"),
        file("db-roles.sh", b"x"),
        dir("initdb/"),
        file("cortex.toml.example", b"x"),
    ];
    es.extend(BIN_MEMBERS.iter().map(|n| file(n, b"x")));
    es.extend(UNIT_MEMBERS.iter().map(|n| file(n, b"x")));
    let names = unpack(&finish(&es), &into).unwrap();
    assert!(
        names.iter().any(|n| n == "bin") && names.iter().any(|n| n == "systemd"),
        "{names:?}"
    );
    let mode = |p: &str| fs::metadata(into.join(p)).unwrap().permissions().mode() & 0o7777;
    assert_eq!((mode("bin"), mode("systemd")), (0o755, 0o755));
    assert!(
        into.join("bin/cortex-dump").is_file() && into.join("systemd/cortex-dump.timer").is_file()
    );
}

#[test]
fn un_28_a_repeated_new_member_is_refused() {
    for name in [
        "cortex.toml.example",
        "bin/cortex-dump",
        "systemd/cortex-dump.timer",
    ] {
        let (t, into) = target();
        let mut es = good();
        es.push(file(name, b"second"));
        assert_eq!(
            unpack(&finish(&es), &into),
            Err(UnpackError::Duplicate {
                index: good().len()
            }),
            "{name}"
        );
        assert!(parent_is_empty(&t));
    }
}

// ---------------------------------------------------------------- UP-04: hostile archives

#[test]
fn un_03_only_regular_files_and_directories_are_allowed() {
    for &ty in b"123467xgLKSDMNVA" {
        let (t, into) = target();
        let mut es = good();
        es.insert(1, typed("link", ty));
        let e = unpack(&finish(&es), &into).expect_err(&format!("type {}", ty as char));
        assert_eq!(
            e,
            UnpackError::Forbidden { index: 1 },
            "type {}",
            ty as char
        );
        assert!(!into.exists(), "the target is removed after a refusal");
        assert!(parent_is_empty(&t), "nothing else was created");
    }
    // a symlink named like an allowed file is still a symlink
    let (t, into) = target();
    let es = vec![
        typed("cortex", b'2'),
        file("compose.yml", b"x"),
        file("stack.env", b"x"),
    ];
    assert_eq!(
        unpack(&finish(&es), &into),
        Err(UnpackError::Forbidden { index: 0 })
    );
    assert!(parent_is_empty(&t));
    // wrong kind for an allowed name
    for bad in [dir("cortex/"), dir("compose.yml"), file("initdb", b"x")] {
        let (t, into) = target();
        let mut es = good();
        es[0] = bad;
        assert!(matches!(
            unpack(&finish(&es), &into),
            Err(UnpackError::Forbidden { .. } | UnpackError::Missing { .. })
        ));
        assert!(parent_is_empty(&t));
    }
}

#[test]
fn un_04_only_the_names_a_release_may_hold_are_allowed() {
    let long = format!("initdb/{}", "a".repeat(93));
    let names = [
        "../x",
        "/etc/passwd",
        "initdb/../../x",
        "initdb/../x",
        "evil",
        "./cortex",
        "cortex/",
        "initdb/sub/deep",
        "initdb/.hidden",
        "initdb/a b",
        "INITDB/x",
        "compose.yml.bak",
        "a\\b",
        "initdb/",
        "initdb//x",
        "initdb/x\n",
        "",
        "initdb/é",
        &long,
        "initdb/..",
        "initdb/.",
        ".",
        "..",
        "stack.env/x",
        "manifest.json",
        "SHA256SUMS",
    ];
    for name in names {
        let (t, into) = target();
        let mut es = good();
        es.insert(2, file(name, b"x"));
        let r = unpack(&finish(&es), &into);
        assert!(
            matches!(
                r,
                Err(UnpackError::Forbidden { index: 2 }) | Err(UnpackError::Duplicate { index: 2 })
            ),
            "{name:?}: {r:?}"
        );
        assert!(!into.exists(), "{name:?}");
        assert!(parent_is_empty(&t), "{name:?}: nothing outside the target");
    }
    // a name that is not UTF-8
    let (t, into) = target();
    let mut es = good();
    es.insert(
        2,
        entry(header(b"initdb/\xff\xfe", b'0', 1, 0o644, b"", b""), b"x"),
    );
    assert_eq!(
        unpack(&finish(&es), &into),
        Err(UnpackError::Forbidden { index: 2 })
    );
    assert!(parent_is_empty(&t));
    // the ustar prefix field must be empty (it would extend the name)
    let (t, into) = target();
    let mut es = good();
    es.insert(
        2,
        entry(
            header(b"initdb/extra.sh", b'0', 1, 0o644, b"", b"../.."),
            b"x",
        ),
    );
    assert_eq!(
        unpack(&finish(&es), &into),
        Err(UnpackError::Forbidden { index: 2 })
    );
    assert!(parent_is_empty(&t));
}

#[test]
fn un_05_a_repeated_name_and_a_missing_required_file_are_refused() {
    let (t, into) = target();
    let mut es = good();
    es.push(file("compose.yml", b"second"));
    assert_eq!(
        unpack(&finish(&es), &into),
        Err(UnpackError::Duplicate {
            index: good().len()
        })
    );
    assert!(parent_is_empty(&t));
    for (skip, name) in [(0, "cortex"), (1, "compose.yml"), (2, "stack.env")] {
        let (t, into) = target();
        let mut es = good();
        es.remove(skip);
        assert_eq!(
            unpack(&finish(&es), &into),
            Err(UnpackError::Missing { name }),
            "{name}"
        );
        assert!(!into.exists() && parent_is_empty(&t));
    }
}

#[test]
fn un_06_sizes_and_counts_are_capped_before_anything_large_is_read() {
    let lim = UnpackLimits {
        max_entries: 4,
        max_file_bytes: 100,
        max_total_bytes: 150,
    };
    let (t, into) = target();
    // too many entries
    let es = vec![
        file("cortex", b"x"),
        file("compose.yml", b"x"),
        file("stack.env", b"x"),
        dir("initdb/"),
        file("initdb/a", b"x"),
    ];
    assert_eq!(
        unpack_release(Cursor::new(finish(&es)), &into, &lim),
        Err(UnpackError::TooLarge)
    );
    // one file over the file cap (declared in the header; only a few bytes follow)
    let h = header(b"cortex", b'0', 101, 0o755, b"", b"");
    let es = vec![
        entry(h, &[b'x'; 101]),
        file("compose.yml", b"x"),
        file("stack.env", b"x"),
    ];
    assert_eq!(
        unpack_release(Cursor::new(finish(&es)), &into, &lim),
        Err(UnpackError::TooLarge)
    );
    // the total over the total cap
    let es = vec![
        file("cortex", &[b'x'; 80]),
        file("compose.yml", &[b'x'; 80]),
        file("stack.env", b"x"),
    ];
    assert_eq!(
        unpack_release(Cursor::new(finish(&es)), &into, &lim),
        Err(UnpackError::TooLarge)
    );
    // exactly the caps are fine (R-7: the initdb entry is a fifth entry, so the entry cap is 5 here, and it is met exactly)
    let mut es = vec![
        file("cortex", &[b'x'; 100]),
        file("compose.yml", &[b'x'; 49]),
        file("stack.env", b"x"),
        file("db-roles.sh", b""),
        dir("initdb/"),
    ];
    es.extend(new_members_with(b"")); // list 2 (R-4): 13 more entries, all empty
    let exact = UnpackLimits {
        max_entries: 18,
        ..lim
    };
    unpack_release(Cursor::new(finish(&es)), &into, &exact).unwrap();
    drop(t);
    // a header that declares the largest size its 11 octal digits allow (8 GiB) with no data behind it is TooLarge, not an attempt to read it
    let (t, into) = target();
    let h = header(b"cortex", b'0', (1u64 << 33) - 1, 0o755, b"", b"");
    let r = unpack(&finish(&[entry(h, b"")]), &into);
    assert_eq!(r, Err(UnpackError::TooLarge));
    assert!(parent_is_empty(&t));
}

#[test]
fn un_07_damaged_or_incomplete_archives_are_refused() {
    let full = finish(&good());
    {
        let (_t, into) = target();
        assert!(
            unpack(&full, &into).is_ok(),
            "control: the undamaged archive unpacks"
        );
    }
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("empty", vec![]),
        ("one zero block", vec![0u8; 512]),
        ("no end marker", good().concat()),
        ("cut inside a header", full[..300].to_vec()),
        ("cut inside the data", full[..512 + 5].to_vec()),
        ("bad checksum", {
            let mut v = full.clone();
            v[10] ^= 1;
            v
        }),
        ("size with a non-octal digit", {
            let mut h = header(b"cortex", b'0', 1, 0o755, b"", b"");
            h[130] = b'9';
            seal(&mut h);
            finish(&[entry(h, b"x")])
        }),
        ("base-256 size", {
            let mut h = header(b"cortex", b'0', 1, 0o755, b"", b"");
            h[124] = 0x80;
            seal(&mut h);
            finish(&[entry(h, b"x")])
        }),
        ("only one terminating block", {
            let mut v = good().concat();
            v.extend_from_slice(&[0u8; 512]);
            v
        }),
    ];
    for (what, bytes) in cases {
        let (t, into) = target();
        let r = unpack(&bytes, &into);
        assert_eq!(r, Err(UnpackError::Corrupt), "{what}");
        assert!(!into.exists() && parent_is_empty(&t), "{what}");
    }
}

#[test]
fn un_08_an_existing_target_is_never_merged_into_and_a_missing_parent_is_an_error() {
    let t = Tmp::new("unpack");
    let into = t.path().join("stage");
    fs::create_dir(&into).unwrap();
    fs::write(into.join("KEEP"), "keep").unwrap();
    assert_eq!(unpack(&finish(&good()), &into), Err(UnpackError::Exists));
    assert_eq!(
        fs::read_to_string(into.join("KEEP")).unwrap(),
        "keep",
        "an existing directory is left alone"
    );
    assert!(!into.join("cortex").exists());
    // a link in the way is also "exists", and is not followed
    let link = t.path().join("lnk");
    std::os::unix::fs::symlink(t.path(), &link).unwrap();
    assert_eq!(unpack(&finish(&good()), &link), Err(UnpackError::Exists));
    assert!(!t.path().join("cortex").exists());
    let r = unpack(&finish(&good()), &t.path().join("no/such/parent/stage"));
    assert_eq!(r, Err(UnpackError::Io(std::io::ErrorKind::NotFound)));
}

#[test]
fn un_09_the_reader_stops_at_the_end_marker_and_does_not_read_what_follows() {
    struct Tripwire {
        data: Cursor<Vec<u8>>,
        end: usize,
    }
    impl std::io::Read for Tripwire {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            assert!(
                (self.data.position() as usize) <= self.end,
                "read past the end marker"
            );
            let room = self.end - self.data.position() as usize;
            if room == 0 {
                return Err(std::io::Error::other("read at or past the end marker"));
            }
            let n = room.min(buf.len());
            self.data.read(&mut buf[..n])
        }
    }
    let mut bytes = finish(&good());
    let end = bytes.len();
    bytes.extend_from_slice(&[b'J'; 4096]); // junk after the marker
    let (_t, into) = target();
    let r = unpack_release(
        Tripwire {
            data: Cursor::new(bytes),
            end,
        },
        &into,
        &UnpackLimits::default(),
    );
    assert!(r.is_ok(), "{r:?}");
}

#[test]
fn un_10_a_directory_entry_that_carries_data_is_refused() {
    let (t, into) = target();
    let mut es = good();
    es[3] = entry(header(b"initdb/", b'5', 512, 0o755, b"", b""), &[b'x'; 512]);
    assert_eq!(
        unpack(&finish(&es), &into),
        Err(UnpackError::Forbidden { index: 3 })
    );
    assert!(parent_is_empty(&t));
}

// ---------------------------------------------------------------- gzip

fn gz(bytes: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(bytes).unwrap();
    e.finish().unwrap()
}

fn unpack_gz(
    bytes: &[u8],
    into: &std::path::Path,
    limits: &UnpackLimits,
) -> Result<Vec<String>, UnpackError> {
    unpack_release_gz(Cursor::new(bytes.to_vec()), into, limits)
}

#[test]
fn un_11_a_gzip_release_unpacks_like_a_plain_one() {
    let (_t, into) = target();
    let names = unpack_gz(&gz(&finish(&good())), &into, &UnpackLimits::default()).unwrap();
    assert_eq!(
        names,
        [
            [
                "cortex",
                "compose.yml",
                "stack.env",
                "initdb",
                "initdb/00-cortex-roles.sh"
            ]
            .as_slice(),
            NEW_NAMES.as_slice(),
            ["db-roles.sh"].as_slice(),
        ]
        .concat()
    );
    assert_eq!(
        fs::read(into.join("compose.yml")).unwrap(),
        b"services: {}\n"
    );
}

#[test]
fn un_12_damaged_gzip_is_corrupt_and_leaves_nothing() {
    let full = gz(&finish(&good()));
    {
        let (_t, into) = target();
        assert!(
            unpack_gz(&full, &into, &UnpackLimits::default()).is_ok(),
            "control: the undamaged stream unpacks"
        );
    }
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("not gzip", b"this is not a gzip stream at all".to_vec()),
        ("empty", vec![]),
        ("a plain tar", finish(&good())),
        ("cut in the middle", full[..full.len() / 2].to_vec()),
        ("cut before the trailer", full[..full.len() - 6].to_vec()),
        ("a flipped byte in the body", {
            let mut v = full.clone();
            let m = v.len() / 2;
            v[m] ^= 0xff;
            v
        }),
    ];
    for (what, bytes) in cases {
        let (t, into) = target();
        let r = unpack_gz(&bytes, &into, &UnpackLimits::default());
        assert_eq!(r, Err(UnpackError::Corrupt), "{what}");
        assert!(!into.exists() && parent_is_empty(&t), "{what}");
    }
}

#[test]
fn un_13_a_gzip_bomb_is_stopped_by_the_caps_on_the_decompressed_size() {
    // 64 MiB of zeros inside a file entry compresses to a few tens of KiB
    let big = vec![0u8; 64 << 20];
    let es = vec![
        file_mode("cortex", &big, 0o755),
        file("compose.yml", b"x"),
        file("stack.env", b"x"),
    ];
    let bomb = gz(&finish(&es));
    assert!(
        bomb.len() < 200 * 1024,
        "the fixture is really a bomb: {} bytes",
        bomb.len()
    );
    let (t, into) = target();
    let lim = UnpackLimits {
        max_entries: 16,
        max_file_bytes: 1 << 20,
        max_total_bytes: 2 << 20,
    };
    let started = std::time::Instant::now();
    assert_eq!(unpack_gz(&bomb, &into, &lim), Err(UnpackError::TooLarge));
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    assert!(!into.exists() && parent_is_empty(&t));
    // trailing garbage after the gzip member is not read into the archive
    let mut ok = gz(&finish(&good()));
    ok.extend_from_slice(&[0xAB; 4096]);
    let (_t2, into2) = target();
    assert!(unpack_gz(&ok, &into2, &UnpackLimits::default()).is_ok());
}

#[test]
fn un_14_a_long_tail_after_the_end_marker_inside_the_gzip_member_is_refused() {
    // 2 MiB of zeros after the tar end marker, still inside the one gzip member: more than the allowed tail
    let mut tar = finish(&good());
    tar.extend_from_slice(&vec![0u8; 2 << 20]);
    let (t, into) = target();
    assert_eq!(
        unpack_gz(&gz(&tar), &into, &UnpackLimits::default()),
        Err(UnpackError::Corrupt)
    );
    assert!(!into.exists() && parent_is_empty(&t));
    // a short tail (tar padding) is fine
    let mut ok = finish(&good());
    ok.extend_from_slice(&vec![0u8; 8192]);
    let (_t2, into2) = target();
    assert!(unpack_gz(&gz(&ok), &into2, &UnpackLimits::default()).is_ok());
}

// ---------------------------------------------------------------- mike, packet 3: the directory is 0700 when it appears

fn mode_of(p: &std::path::Path) -> u32 {
    fs::metadata(p).unwrap().permissions().mode() & 0o7777
}

/// The child half of un_15: it runs under a umask the parent chose (through `sh`; the test process itself never
/// changes its umask), does one thing and prints what it saw.
fn umask_child(arg: &str) {
    use cortex_upgrade::unpack::{create_private_dir, create_private_dir_exact};
    // "<case> <directory>": the parent made the directory under its own umask (under a strict one the child could not
    // even create its working directory)
    let (case, dir) = arg.split_once(' ').unwrap();
    let dir = std::path::Path::new(dir);
    match case {
        "create" => {
            let d = dir.join("d");
            create_private_dir(&d).unwrap();
            println!("CHILD-CREATE-MODE {:o}", mode_of(&d));
            // an existing path of any kind is refused and left alone
            println!(
                "CHILD-AGAIN {:?}",
                create_private_dir(&d).map_err(|e| e.to_string())
            );
            println!(
                "CHILD-MISSING-PARENT {}",
                create_private_dir(&dir.join("nope/d")).is_err()
            );
        }
        "exact" => {
            let d = dir.join("e");
            create_private_dir_exact(&d).unwrap();
            let writable = fs::write(d.join("f"), "x").is_ok();
            println!("CHILD-EXACT-MODE {:o} writable={writable}", mode_of(&d));
            println!(
                "CHILD-EXACT-AGAIN {:?}",
                create_private_dir_exact(&d).map_err(|e| e.to_string())
            );
        }
        _ => {
            let into = dir.join("stage");
            let r = unpack(&finish(&good()), &into);
            println!(
                "CHILD-UNPACK {} {:o} {:o}",
                r.is_ok(),
                mode_of(&into),
                mode_of(&into.join("initdb"))
            );
        }
    }
}

#[test]
fn un_15_the_staging_directory_is_created_0700_by_the_mkdir_itself_whatever_the_umask() {
    if let Ok(case) = std::env::var("CORTEX_TEST_UMASK_CHILD") {
        return umask_child(&case);
    }
    let name = "un_15_the_staging_directory_is_created_0700_by_the_mkdir_itself_whatever_the_umask";
    let run = |case: &str, mask: &str| {
        let t = Tmp::new("unpack-umask");
        let arg = format!("{case} {}", t.path().display());
        let out = rerun_self_umask(name, ("CORTEX_TEST_UMASK_CHILD", &arg), mask);
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    // umask 000 would make a plain mkdir 0777; umask 077 and 022 are the usual ones
    for mask in ["000", "022", "077"] {
        let text = run("create", mask);
        assert!(
            text.contains("CHILD-CREATE-MODE 700\n"),
            "umask {mask}: the directory was not 0700 when it was created: {text}"
        );
        assert!(text.contains("CHILD-AGAIN Err(\"") && text.contains("CHILD-MISSING-PARENT true"));
    }
    // a umask that takes owner bits away must not leave the staging directory unusable: the unpack ends with exactly
    // 0700 (and the directory made by the archive's own entry 0755)
    for mask in ["000", "022", "377"] {
        let text = run("unpack", mask);
        assert!(
            text.contains("CHILD-UNPACK true 700 755\n")
                || text.contains("CHILD-UNPACK true 700 755\r\n"),
            "umask {mask}: {text}"
        );
    }
}

// ---------------------------------------------------------------- mike, packet 3: archives made by the system tools

#[test]
fn un_16_a_tar_and_a_tar_gz_made_by_the_system_tools_are_read() {
    use std::process::Command;
    let t = Tmp::new("unpack-sys");
    let src = t.path().join("src");
    fs::create_dir_all(src.join("initdb")).unwrap();
    fs::create_dir_all(src.join("bin")).unwrap();
    fs::create_dir_all(src.join("systemd")).unwrap();
    for n in BIN_MEMBERS.iter().chain(UNIT_MEMBERS.iter()) {
        fs::write(src.join(n), b"x\n").unwrap();
    }
    fs::write(src.join("cortex.toml.example"), b"[modules]\n").unwrap();
    fs::write(src.join("cortex"), b"#!/bin/sh\n:\n").unwrap();
    fs::write(src.join("compose.yml"), b"name: x\n").unwrap();
    fs::write(src.join("stack.env"), b"CORTEX_VERSION=0.2.1\n").unwrap();
    fs::write(src.join("db-roles.sh"), b"#!/bin/sh\n:\n").unwrap();
    fs::write(src.join("initdb/00-cortex-roles.sh"), b"#!/bin/sh\n:\n").unwrap();
    for (flag, name) in [("-cf", "r.tar"), ("-czf", "r.tgz")] {
        let archive = t.path().join(name);
        let status = Command::new("tar")
            .current_dir(&src)
            .args(["--format=ustar", flag])
            .arg(&archive)
            .args([
                "cortex",
                "compose.yml",
                "stack.env",
                "db-roles.sh",
                "cortex.toml.example",
                "initdb",
                "bin",
                "systemd",
            ])
            .status()
            .expect("the system tar runs");
        assert!(status.success(), "tar {flag}");
        let bytes = fs::read(&archive).unwrap();
        let into = t.path().join(format!("out-{name}"));
        let names = if flag == "-czf" {
            unpack_gz(&bytes, &into, &UnpackLimits::default())
        } else {
            unpack(&bytes, &into)
        }
        .unwrap_or_else(|e| panic!("{name}: {e:?}"));
        for want in [
            "cortex",
            "compose.yml",
            "stack.env",
            "db-roles.sh",
            "initdb/00-cortex-roles.sh",
        ] {
            assert!(
                names.iter().any(|n| n == want),
                "{name}: {want} in {names:?}"
            );
        }
        assert_eq!(fs::read(into.join("compose.yml")).unwrap(), b"name: x\n");
        assert_eq!(
            (
                mode_of(&into),
                mode_of(&into.join("cortex")),
                mode_of(&into.join("initdb")),
                mode_of(&into.join("db-roles.sh"))
            ),
            (0o700, 0o755, 0o755, 0o755),
            "{name}: modes are ours, not the archive's"
        );
    }
}

// ---------------------------------------------------------------- cox 14:30Z, Q-7: db-roles.sh is part of every release

#[test]
fn un_17_db_roles_sh_is_required_and_the_message_names_it() {
    let (t, into) = target();
    let mut es = good();
    let at = es.len() - 1;
    es.remove(at); // the db-roles.sh entry is the last one in good()
    assert_eq!(
        unpack(&finish(&es), &into),
        Err(UnpackError::Missing {
            name: "db-roles.sh"
        })
    );
    assert!(!into.exists() && parent_is_empty(&t));
    assert_eq!(
        UnpackError::Missing {
            name: "db-roles.sh"
        }
        .to_string(),
        "unpack refused: the release lacks `db-roles.sh`"
    );
    // control: with it, the same archive is accepted
    let (_t, into) = target();
    let names = unpack(&finish(&good()), &into).unwrap();
    assert!(names.iter().any(|n| n == "db-roles.sh"), "{names:?}");
}

#[test]
fn un_18_db_roles_sh_gets_mode_0755_whatever_the_archive_says_and_keeps_its_bytes() {
    for archive_mode in [0o4777, 0o600, 0o444, 0o777, 0o000] {
        let (_t, into) = target();
        let mut es = good();
        let at = es.len() - 1;
        es[at] = file_mode("db-roles.sh", b"#!/bin/sh\necho roles\n", archive_mode);
        unpack(&finish(&es), &into).unwrap();
        assert_eq!(
            mode_of(&into.join("db-roles.sh")),
            0o755,
            "archive said {archive_mode:o}"
        );
        assert_eq!(
            fs::read(into.join("db-roles.sh")).unwrap(),
            b"#!/bin/sh\necho roles\n"
        );
    }
}

#[test]
fn un_19_only_the_one_exact_name_is_db_roles_sh() {
    for name in [
        "db-roles.sh.bak",
        "DB-ROLES.SH",
        "Db-Roles.sh",
        "db-roles",
        "db-roles.sh ",
        "db-roles.sh\n",
        "./db-roles.sh",
        "db-roles.sh/",
        "sub/db-roles.sh",
        "initdb/sub/db-roles.sh",
        "db-roles.sh/x",
        "xdb-roles.sh",
        "db_roles.sh",
    ] {
        let (t, into) = target();
        let mut es = good();
        es.insert(2, file(name, b"x"));
        let r = unpack(&finish(&es), &into);
        assert!(
            matches!(r, Err(UnpackError::Forbidden { index: 2 })),
            "{name:?}: {r:?}"
        );
        assert!(!into.exists() && parent_is_empty(&t), "{name:?}");
    }
    // a link or a directory of that name is no file
    for ty in *b"21346" {
        let (t, into) = target();
        let mut es = good();
        es.insert(2, typed("db-roles.sh", ty));
        let r = unpack(&finish(&es), &into);
        assert!(
            matches!(r, Err(UnpackError::Forbidden { index: 2 })),
            "type {}: {r:?}",
            ty as char
        );
        assert!(!into.exists() && parent_is_empty(&t));
    }
    let (t, into) = target();
    let mut es = good();
    es.insert(2, dir("db-roles.sh"));
    assert!(matches!(
        unpack(&finish(&es), &into),
        Err(UnpackError::Forbidden { index: 2 })
    ));
    assert!(!into.exists() && parent_is_empty(&t));
    // twice is a repeated name
    let (t, into) = target();
    let mut es = good();
    es.push(file("db-roles.sh", b"second"));
    assert_eq!(
        unpack(&finish(&es), &into),
        Err(UnpackError::Duplicate {
            index: good().len()
        })
    );
    assert!(!into.exists() && parent_is_empty(&t));
    // control: a file of that name INSIDE initdb is an initdb file (mode 0644), not the roles script
    let (_t, into) = target();
    let mut es = good();
    es.insert(5, file("initdb/db-roles.sh", b"x"));
    unpack(&finish(&es), &into).unwrap();
    assert_eq!(mode_of(&into.join("initdb/db-roles.sh")), 0o644);
    assert_eq!(mode_of(&into.join("db-roles.sh")), 0o755);
}

#[test]
fn un_20_db_roles_sh_counts_against_the_size_caps_like_any_file() {
    let lim = UnpackLimits {
        max_entries: 30,
        max_file_bytes: 100,
        max_total_bytes: 150,
    };
    let (_t, into) = target();
    let h = header(b"db-roles.sh", b'0', 101, 0o755, b"", b"");
    let es = vec![
        file("cortex", b"x"),
        file("compose.yml", b"x"),
        file("stack.env", b"x"),
        entry(h, &[b'x'; 101]),
    ];
    assert_eq!(
        unpack_release(Cursor::new(finish(&es)), &into, &lim),
        Err(UnpackError::TooLarge)
    );
    // exactly the file cap is fine
    let (_t, into) = target();
    let mut es = vec![
        file("cortex", b"x"),
        file("compose.yml", b"x"),
        file("stack.env", b"x"),
        file("db-roles.sh", &[b'x'; 100]),
        dir("initdb/"), // R-7: required
    ];
    es.extend(new_members_with(b"")); // list 2 (R-4)
    unpack_release(Cursor::new(finish(&es)), &into, &lim).unwrap();
    // the total cap counts it
    let (_t, into) = target();
    let es = vec![
        file("cortex", &[b'x'; 60]),
        file("compose.yml", &[b'x'; 50]),
        file("stack.env", b"x"),
        file("db-roles.sh", &[b'x'; 50]),
    ];
    assert_eq!(
        unpack_release(Cursor::new(finish(&es)), &into, &lim),
        Err(UnpackError::TooLarge)
    );
}

// ---------------------------------------------------------------- mike, packet 6: a strict umask must not strip the owner's bits

#[test]
fn un_21_create_private_dir_exact_is_0700_and_usable_under_any_umask() {
    if let Ok(arg) = std::env::var("CORTEX_TEST_UMASK_CHILD") {
        return umask_child(&arg);
    }
    let name = "un_21_create_private_dir_exact_is_0700_and_usable_under_any_umask";
    for mask in ["000", "022", "277", "377"] {
        let t = Tmp::new("unpack-exact");
        let arg = format!("exact {}", t.path().display());
        let out = rerun_self_umask(name, ("CORTEX_TEST_UMASK_CHILD", &arg), mask);
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            text.contains("CHILD-EXACT-MODE 700 writable=true\n"),
            "umask {mask}: {text}"
        );
        assert!(
            text.contains("CHILD-EXACT-AGAIN Err(\"") && text.contains("already exists"),
            "umask {mask}: an existing path is still refused: {text}"
        );
    }
}
