//! The health document, the readiness decision and the `LAST_GOOD` reader (contract r0.3 section 2; S3 r2.6 `Health`; BK-08).
//! DB-free. RED first.

use cortex_server::health::{
    Backup, FUTURE_SLACK_SECS, HealthInput, LastGood, MAX_LAST_GOOD_BYTES, PieceHealth,
    STALE_AFTER_SECS, backup_state, health_doc, parse_last_good, read_checked, read_last_good,
    same_file,
};
use cortex_types::Timestamp;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::PathBuf;

fn ts(secs: i64) -> Timestamp {
    Timestamp::from_unix_micros(secs * 1_000_000)
}

// 2026-11-06T00:00:00Z
const T0: i64 = 1_793_923_200;

fn dir(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("health-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).expect("dir");
    d
}

fn good(utc: &str) -> String {
    format!(
        r#"{{"set":"20261106T000000Z","utc":"{utc}","bytes":12345,"sha256":"{}"}}"#,
        "ab".repeat(32)
    )
}

#[test]
fn hl_01_the_clock_is_the_26_hour_rule_with_both_edges() {
    let at = LastGood::At(ts(T0));
    assert_eq!(STALE_AFTER_SECS, 26 * 3600);
    assert_eq!(backup_state(&at, ts(T0 + 1)), (Some(ts(T0)), Backup::Ok));
    assert_eq!(
        backup_state(&at, ts(T0 + STALE_AFTER_SECS)),
        (Some(ts(T0)), Backup::Ok),
        "exactly 26 h is still ok: stale is MORE than 26 h"
    );
    assert_eq!(
        backup_state(&at, ts(T0 + STALE_AFTER_SECS + 1)),
        (Some(ts(T0)), Backup::Stale)
    );
    assert_eq!(
        backup_state(&at, ts(T0 + 28 * 3600)),
        (Some(ts(T0)), Backup::Stale)
    );
}

#[test]
fn hl_02_missing_is_none_and_an_unusable_file_is_stale_never_ok() {
    assert_eq!(
        backup_state(&LastGood::Missing, ts(T0)),
        (None, Backup::None)
    );
    assert_eq!(
        backup_state(&LastGood::Unusable, ts(T0)),
        (None, Backup::Stale)
    );
}

#[test]
fn hl_03_a_time_in_the_future_cannot_mask_staleness() {
    let now = ts(T0);
    // a little ahead of the clock (skew) counts as age zero
    assert_eq!(
        backup_state(&LastGood::At(ts(T0 + FUTURE_SLACK_SECS)), now),
        (Some(ts(T0 + FUTURE_SLACK_SECS)), Backup::Ok)
    );
    // far ahead is not believed
    assert_eq!(
        backup_state(&LastGood::At(ts(T0 + FUTURE_SLACK_SECS + 1)), now),
        (None, Backup::Stale)
    );
    assert_eq!(
        backup_state(&LastGood::At(ts(T0 + 400 * 86_400)), now),
        (None, Backup::Stale)
    );
}

#[test]
fn hl_04_parse_accepts_the_marker_and_reads_only_utc() {
    assert_eq!(
        parse_last_good(good("2026-11-06T00:00:00Z").as_bytes()),
        LastGood::At(ts(T0))
    );
    // a fraction of a second is allowed (1 to 6 digits) and truncated to the microsecond
    assert_eq!(
        parse_last_good(good("2026-11-06T00:00:00.5Z").as_bytes()),
        LastGood::At(Timestamp::from_unix_micros(T0 * 1_000_000 + 500_000))
    );
    assert_eq!(
        parse_last_good(good("2026-11-06T00:00:00.123456Z").as_bytes()),
        LastGood::At(Timestamp::from_unix_micros(T0 * 1_000_000 + 123_456))
    );
    // other keys are not read (piece 12 owns the rest of the file)
    assert_eq!(
        parse_last_good(br#"{"utc":"2026-11-06T00:00:00Z"}"#),
        LastGood::At(ts(T0))
    );
    // the leap day of a leap year is a date
    assert!(matches!(
        parse_last_good(good("2028-02-29T12:00:00Z").as_bytes()),
        LastGood::At(_)
    ));
}

#[test]
fn hl_05_parse_refuses_everything_else() {
    let bad: Vec<Vec<u8>> = vec![
        b"".to_vec(),
        b"   ".to_vec(),
        b"[]".to_vec(),
        b"null".to_vec(),
        b"not json".to_vec(),
        b"{}".to_vec(),
        br#"{"utc":5}"#.to_vec(),
        br#"{"utc":null}"#.to_vec(),
        good("2026-11-06T00:00:00+00:00").into_bytes(),
        good("2026-11-06T00:00:00z").into_bytes(),
        good("2026-11-06T00:00:00").into_bytes(),
        good("2026-11-06").into_bytes(),
        good("2026-11-06 00:00:00Z").into_bytes(),
        good("2026-13-06T00:00:00Z").into_bytes(),
        good("2026-00-06T00:00:00Z").into_bytes(),
        good("2026-11-31T00:00:00Z").into_bytes(),
        good("2027-02-29T00:00:00Z").into_bytes(),
        good("2100-02-29T00:00:00Z").into_bytes(),
        good("2026-11-06T24:00:00Z").into_bytes(),
        good("2026-11-06T00:60:00Z").into_bytes(),
        good("2026-11-06T00:00:60Z").into_bytes(),
        good("2026-11-06T00:00:00.Z").into_bytes(),
        good("2026-11-06T00:00:00.1234567Z").into_bytes(),
        good("1969-12-31T23:59:59Z").into_bytes(),
        good(" 2026-11-06T00:00:00Z").into_bytes(),
        good("+2026-11-06T00:00:00Z").into_bytes(),
        vec![0xff, 0xfe, b'{', b'}'],
    ];
    for b in bad {
        assert_eq!(
            parse_last_good(&b),
            LastGood::Unusable,
            "must be unusable: {:?}",
            String::from_utf8_lossy(&b)
        );
    }
    let big = format!(
        r#"{{"utc":"2026-11-06T00:00:00Z","pad":"{}"}}"#,
        "x".repeat(MAX_LAST_GOOD_BYTES as usize)
    );
    assert_eq!(
        parse_last_good(big.as_bytes()),
        LastGood::Unusable,
        "over the size cap"
    );
}

#[test]
fn hl_06_read_missing_unusable_and_good() {
    let d = dir("read");
    assert_eq!(read_last_good(&d.join("LAST_GOOD")), LastGood::Missing);
    assert_eq!(
        read_last_good(&d.join("no-such-dir").join("LAST_GOOD")),
        LastGood::Missing
    );

    fs::write(d.join("LAST_GOOD"), b"").expect("seed");
    assert_eq!(
        read_last_good(&d.join("LAST_GOOD")),
        LastGood::Unusable,
        "empty"
    );

    fs::write(d.join("LAST_GOOD"), good("2026-11-06T00:00:00Z")).expect("seed");
    assert_eq!(read_last_good(&d.join("LAST_GOOD")), LastGood::At(ts(T0)));
}

#[test]
fn hl_07_read_never_follows_a_symlink_or_reads_a_big_file_or_a_directory() {
    let d = dir("odd");
    let real = d.join("real");
    fs::write(&real, good("2026-11-06T00:00:00Z")).expect("seed");
    symlink(&real, d.join("LAST_GOOD")).expect("link");
    assert_eq!(
        read_last_good(&d.join("LAST_GOOD")),
        LastGood::Unusable,
        "a symlink is not followed"
    );

    let d2 = dir("big");
    fs::write(d2.join("LAST_GOOD"), vec![b' '; 1_000_000]).expect("seed");
    assert_eq!(
        read_last_good(&d2.join("LAST_GOOD")),
        LastGood::Unusable,
        "over the cap"
    );

    let d3 = dir("isdir");
    fs::create_dir(d3.join("LAST_GOOD")).expect("dir");
    assert_eq!(
        read_last_good(&d3.join("LAST_GOOD")),
        LastGood::Unusable,
        "a directory"
    );
}

fn piece(name: &str, enabled: bool, ready: bool) -> PieceHealth {
    PieceHealth {
        name: name.into(),
        enabled,
        ready,
    }
}

fn doc(
    db: bool,
    mig: bool,
    pieces: &[PieceHealth],
    last: &LastGood,
    now: i64,
) -> serde_json::Value {
    let d = health_doc(&HealthInput {
        version: "0.2.1",
        database_reachable: db,
        migrations_current: mig,
        pieces,
        last_good: last,
        now: ts(now),
    });
    serde_json::to_value(&d).expect("serialise")
}

#[test]
fn hl_08_the_document_has_exactly_the_s3_fields_and_nothing_else() {
    let v = doc(
        true,
        true,
        &[piece("identity", true, true), piece("search", false, false)],
        &LastGood::At(ts(T0)),
        T0 + 60,
    );
    let mut keys: Vec<&str> = v
        .as_object()
        .expect("object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "backup",
            "last_good_dump_at",
            "pieces",
            "ready",
            "state",
            "version"
        ]
    );
    assert_eq!(v["version"], "0.2.1");
    assert_eq!(v["ready"], true);
    assert_eq!(v["backup"], "ok");
    assert_eq!(v["last_good_dump_at"], "2026-11-06T00:00:00Z");
    assert_eq!(
        v["pieces"][0],
        serde_json::json!({"name":"identity","enabled":true,"ready":true})
    );
    assert_eq!(
        v["pieces"][1],
        serde_json::json!({"name":"search","enabled":false,"ready":false}),
        "the input order is kept, disabled pieces are listed"
    );
}

#[test]
fn hl_09_ready_needs_the_database_the_migrations_and_every_enabled_piece() {
    let ok = [piece("a", true, true), piece("b", false, false)];
    let l = LastGood::At(ts(T0));
    assert_eq!(
        doc(true, true, &ok, &l, T0)["ready"],
        true,
        "a disabled piece that is not ready does not block"
    );
    assert_eq!(doc(false, true, &ok, &l, T0)["ready"], false, "no database");
    assert_eq!(
        doc(true, false, &ok, &l, T0)["ready"],
        false,
        "migrations not current"
    );
    assert_eq!(
        doc(true, true, &[piece("a", true, false)], &l, T0)["ready"],
        false,
        "an enabled piece is not ready"
    );
    assert_eq!(
        doc(true, true, &[], &l, T0)["ready"],
        true,
        "no enabled piece"
    );
}

#[test]
fn hl_10_the_backup_state_is_information_it_never_makes_the_server_not_ready() {
    let ok = [piece("a", true, true)];
    for last in [LastGood::Missing, LastGood::Unusable, LastGood::At(ts(T0))] {
        let v = doc(true, true, &ok, &last, T0 + 40 * 3600);
        assert_eq!(v["ready"], true, "{last:?}");
    }
    let none = doc(true, true, &ok, &LastGood::Missing, T0);
    assert_eq!(
        (none["backup"].clone(), none["last_good_dump_at"].clone()),
        (serde_json::json!("none"), serde_json::Value::Null)
    );
    let stale = doc(true, true, &ok, &LastGood::Unusable, T0);
    assert_eq!(
        (stale["backup"].clone(), stale["last_good_dump_at"].clone()),
        (serde_json::json!("stale"), serde_json::Value::Null)
    );
}

#[test]
fn hl_11_the_state_word_follows_readiness() {
    let ok = [piece("a", true, true)];
    let l = LastGood::Missing;
    assert_eq!(doc(true, true, &ok, &l, T0)["state"], "ready");
    assert_eq!(doc(false, true, &ok, &l, T0)["state"], "not_ready");
}

#[test]
fn hl_12_the_opened_file_must_be_the_file_that_was_checked() {
    // hale N-1: lstat then open leaves a swap race (the path becomes a symlink between the two). Comparing the OPENED file's
    // identity with what lstat saw closes it without libc: a symlink (or any other file) swapped in has another inode.
    let d = dir("samefile");
    let a = d.join("a");
    let b = d.join("b");
    fs::write(&a, b"A").expect("seed");
    fs::write(&b, b"B").expect("seed");
    let ma = fs::symlink_metadata(&a).expect("lstat a");
    let mb = fs::symlink_metadata(&b).expect("lstat b");
    assert!(
        same_file(&ma, &fs::metadata(&a).expect("stat a")),
        "the same file"
    );
    assert!(!same_file(&ma, &mb), "two different files");
    // the swap: the path now names a symlink to B; stat follows it, lstat saw the regular file A earlier
    fs::remove_file(&a).expect("rm");
    symlink(&b, &a).expect("link");
    assert!(
        !same_file(&ma, &fs::metadata(&a).expect("stat swapped")),
        "the swapped-in target is not the file that was checked"
    );
}

#[test]
fn hl_13_the_read_refuses_a_file_other_than_the_one_that_was_checked() {
    // The swap, as a test: the metadata describes file B, the path now names file A (a valid LAST_GOOD). Without the identity check
    // the content of A would be believed.
    let d = dir("swapread");
    let a = d.join("a");
    let b = d.join("b");
    fs::write(&a, good("2026-11-06T00:00:00Z")).expect("seed");
    fs::write(&b, b"other").expect("seed");
    let meta_a = fs::symlink_metadata(&a).expect("lstat a");
    let meta_b = fs::symlink_metadata(&b).expect("lstat b");
    assert_eq!(
        read_checked(&a, &meta_a),
        LastGood::At(ts(T0)),
        "the checked file is read"
    );
    assert_eq!(
        read_checked(&a, &meta_b),
        LastGood::Unusable,
        "a file other than the checked one is not trusted"
    );
}

#[test]
fn hl_14_a_symlink_swapped_in_after_the_check_is_never_followed() {
    // vera SERVER-I2-V1: the checked metadata describes F; the path now names a symlink to F. Following it would open F, the SAME inode,
    // and the identity check alone would accept it. The open itself must refuse a symlink (O_NOFOLLOW).
    let d = dir("nofollow");
    let f = d.join("real");
    fs::write(&f, good("2026-11-06T00:00:00Z")).expect("seed");
    let meta_f = fs::metadata(&f).expect("stat f");
    let link = d.join("LAST_GOOD");
    symlink(&f, &link).expect("link");
    assert_eq!(
        read_checked(&link, &meta_f),
        LastGood::Unusable,
        "a symlink at the path is not followed"
    );
    // control: the real file, opened directly with its own metadata, is read
    assert_eq!(read_checked(&f, &meta_f), LastGood::At(ts(T0)));
}

#[test]
fn hl_15_a_fifo_swapped_in_after_the_check_does_not_block_the_server() {
    // vera SERVER-I2-V1: opening a FIFO for reading blocks until a writer appears; the open must be non-blocking and the file refused.
    let d = dir("fifo");
    let regular = d.join("regular");
    fs::write(&regular, b"x").expect("seed");
    let meta = fs::symlink_metadata(&regular).expect("lstat");
    let fifo = d.join("LAST_GOOD");
    // The workspace forbids unsafe code, so the FIFO is made by coreutils' mkfifo.
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("run mkfifo");
    assert!(made.success(), "mkfifo");
    let (tx, rx) = std::sync::mpsc::channel();
    let p = fifo.clone();
    std::thread::spawn(move || {
        let _ = tx.send(read_checked(&p, &meta));
    });
    let got = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("read_checked blocked on a FIFO: the open must be O_NONBLOCK");
    assert_eq!(got, LastGood::Unusable, "a FIFO is not a LAST_GOOD file");
}

#[test]
fn hl_16_a_checked_fifo_is_still_not_a_regular_file() {
    // The identity check alone is not enough: if the metadata handed in describes a FIFO, the opened FIFO has the SAME inode, so the
    // regular-file test on the opened descriptor is what refuses it (and the open must not block).
    let d = dir("fifo2");
    let fifo = d.join("LAST_GOOD");
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("run mkfifo");
    assert!(made.success(), "mkfifo");
    let meta = fs::symlink_metadata(&fifo).expect("lstat the fifo");
    let (tx, rx) = std::sync::mpsc::channel();
    let p = fifo.clone();
    std::thread::spawn(move || {
        let _ = tx.send(read_checked(&p, &meta));
    });
    let got = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("blocked on a FIFO");
    assert_eq!(got, LastGood::Unusable);
}

#[test]
fn hl_17_a_dangling_symlink_is_unusable_not_missing() {
    // vera SERVER-I2D1-V1 (H06): `metadata` would follow the link and say NotFound (-> `none`, "never dumped"); a symlink at the
    // path is something to distrust (-> `stale`), whatever it points at.
    let d = dir("dangling");
    symlink(d.join("nowhere"), d.join("LAST_GOOD")).expect("link");
    assert_eq!(read_last_good(&d.join("LAST_GOOD")), LastGood::Unusable);
}

#[test]
fn hl_18_a_fifo_with_a_writer_feeding_valid_json_is_never_believed() {
    // vera SERVER-I2D1-V1 (V03): the identity check alone does not refuse a FIFO whose metadata was handed in; the regular-file test on
    // the opened descriptor does. The test keeps one reader open, attaches a writer, and pushes a VALID marker into the pipe buffer BEFORE
    // `read_checked` opens its own end, so a reader that believed a FIFO would read it (deterministic, no race).
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let d = dir("fifowriter");
    let fifo = d.join("LAST_GOOD");
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("run mkfifo");
    assert!(made.success(), "mkfifo");
    let meta = fs::symlink_metadata(&fifo).expect("lstat the fifo");
    let _keep_reader = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(&fifo)
        .expect("keep a reader open");
    let mut writer = fs::OpenOptions::new()
        .write(true)
        .open(&fifo)
        .expect("writer open");
    writer
        .write_all(good("2026-11-06T00:00:00Z").as_bytes())
        .expect("fill the pipe");
    // The writer goes away: the next reader gets the bytes and then EOF (with a writer still attached a non-blocking read would end in
    // EAGAIN, which `read_checked` already treats as unusable by accident).
    drop(writer);
    let (tx, rx) = std::sync::mpsc::channel();
    let p = fifo.clone();
    std::thread::spawn(move || {
        let _ = tx.send(read_checked(&p, &meta));
    });
    let got = rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("blocked on a FIFO");
    assert_ne!(got, LastGood::At(ts(T0)), "a FIFO's content was believed");
    assert_eq!(got, LastGood::Unusable);
}
