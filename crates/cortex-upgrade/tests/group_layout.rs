//! Piece 14, test definitions r4: the layout part of UP-07, UP-11, UP-12, UP-16 and UP-22, plus the
//! `stack.env` rules of design section 2. Only the file system; no process, no network.
//! (A "release" here is a synthetic directory; nothing is installed or run.)

mod common;

use common::*;
use cortex_upgrade::layout::{
    CommitStep, Home, LayoutError, LockError, Record, RecordResult, RollbackRefusal, StepRecord,
};
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::time::{Duration, Instant};

const RC1: &str = "0.2.1-rc.1";
const RC2: &str = "0.2.1-rc.2";
const RC3: &str = "0.2.1-rc.3";

fn names(dir: &std::path::Path) -> Vec<String> {
    let mut n: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    n.sort();
    n
}

/// Home with rc.1 current and rc.2 staged.
fn home_with_staged() -> (Tmp, Home) {
    let t = Tmp::new("layout");
    install(t.path(), RC1);
    stage(t.path(), RC2, "expand");
    let h = Home::new(t.path());
    (t, h)
}

// ---------------------------------------------------------------- UP-07 (layout part): commit

#[test]
fn lay_01_commit_switches_current_and_previous_and_touches_nothing_else() {
    let (t, home) = home_with_staged();
    let p = t.path();
    assert_eq!(
        home.current().unwrap(),
        Some(v(RC1)),
        "control: rc.1 is current before"
    );
    let (toml, rc1, dumps, token) = (
        tree_hash(&p.join("cortex.toml")),
        tree_hash(&p.join("releases").join(RC1)),
        tree_hash(&p.join("dumps")),
        tree_hash(&p.join("operator.token")),
    );

    home.commit(&v(RC2)).unwrap();

    assert_eq!(home.current().unwrap(), Some(v(RC2)));
    assert_eq!(home.previous().unwrap(), Some(v(RC1)));
    assert_eq!(
        fs::read_link(p.join("current")).unwrap().to_str().unwrap(),
        format!("releases/{RC2}"),
        "links are relative to the home"
    );
    assert_eq!(
        fs::read_link(p.join("previous")).unwrap().to_str().unwrap(),
        format!("releases/{RC1}")
    );
    assert!(
        fs::read_to_string(p.join("bin/cortex"))
            .unwrap()
            .contains(RC2),
        "bin/cortex follows current"
    );
    assert!(
        !p.join("releases").join(format!("{RC2}.part")).exists(),
        "the staging directory became the release"
    );
    assert!(p.join("releases").join(RC2).join("compose.yml").is_file());
    assert_eq!(
        tree_hash(&p.join("cortex.toml")),
        toml,
        "cortex.toml untouched"
    );
    assert_eq!(
        tree_hash(&p.join("releases").join(RC1)),
        rc1,
        "the old release is byte-identical"
    );
    assert_eq!(tree_hash(&p.join("dumps")), dumps);
    assert_eq!(tree_hash(&p.join("operator.token")), token);
    let left: Vec<String> = names(p).into_iter().filter(|n| n.contains("tmp")).collect();
    assert!(left.is_empty(), "no temporary link left behind: {left:?}");
}

#[test]
fn lay_02_commit_needs_exactly_one_of_staging_or_release() {
    let t = Tmp::new("layout");
    install(t.path(), RC1);
    let home = Home::new(t.path());
    let before = tree_hash(t.path());
    // neither
    assert_eq!(home.commit(&v(RC2)), Err(LayoutError::StagingMissing));
    assert_eq!(tree_hash(t.path()), before);
    // both
    stage(t.path(), RC2, "expand");
    write_release_files(&t.path().join("releases").join(RC2), RC2, "expand");
    let before = tree_hash(t.path());
    assert_eq!(home.commit(&v(RC2)), Err(LayoutError::ReleaseExists));
    assert_eq!(tree_hash(t.path()), before);
}

#[test]
fn lay_03_commit_of_the_current_release_changes_nothing() {
    let t = Tmp::new("layout");
    install(t.path(), RC1);
    let home = Home::new(t.path());
    let before = tree_hash(t.path());
    assert_eq!(home.current().unwrap(), Some(v(RC1)));
    home.commit(&v(RC1)).unwrap();
    assert_eq!(tree_hash(t.path()), before);
}

#[test]
fn lay_04_a_link_that_is_not_a_release_is_refused_not_followed() {
    let t = Tmp::new("layout");
    install(t.path(), RC1);
    let home = Home::new(t.path());
    for bad in [
        "/etc",
        "../outside",
        "releases/not-a-version",
        "releases/0.2.001",
        "releases/0.2.1-rc.1/../0.2.1-rc.1",
        "releases/0.2.1-rc.1/",
        "releases/0.2.1-rc.1.part",
    ] {
        fs::remove_file(t.path().join("current")).unwrap();
        symlink(bad, t.path().join("current")).unwrap();
        assert_eq!(
            home.current(),
            Err(LayoutError::BadLink { link: "current" }),
            "{bad}"
        );
    }
    // a regular file where the link should be is refused too
    fs::remove_file(t.path().join("current")).unwrap();
    fs::write(t.path().join("current"), "releases/0.2.1-rc.1").unwrap();
    assert_eq!(
        home.current(),
        Err(LayoutError::BadLink { link: "current" }),
        "a file named current"
    );
    // a missing link is "no current", not an error
    fs::remove_file(t.path().join("current")).unwrap();
    assert_eq!(home.current().unwrap(), None);
}

// ---------------------------------------------------------------- UP-16: a crash never leaves a bad link

#[test]
fn lay_05_interrupted_at_every_step_never_leaves_a_missing_or_dangling_link() {
    // (stop after, current, previous) as they must stand at that point
    let cases = [
        (CommitStep::DirRenamed, RC1, None),
        (CommitStep::PreviousSet, RC1, Some(RC1)),
        (CommitStep::CurrentSet, RC2, Some(RC1)),
    ];
    for (stop, cur, prev) in cases {
        let (t, home) = home_with_staged();
        let r = home.commit_with(&v(RC2), &mut |s| s != stop);
        assert_eq!(r, Err(LayoutError::Interrupted(stop)), "{stop:?}");
        let p = t.path();
        assert_eq!(
            home.current().unwrap(),
            Some(v(cur)),
            "current after {stop:?}"
        );
        assert_eq!(
            home.previous().unwrap(),
            prev.map(v),
            "previous after {stop:?}"
        );
        assert!(
            resolves_to_a_release(&p.join("current")),
            "current resolves after {stop:?}"
        );
        if prev.is_some() {
            assert!(
                resolves_to_a_release(&p.join("previous")),
                "previous resolves after {stop:?}"
            );
        }
        assert!(
            fs::read_to_string(p.join("bin/cortex")).is_ok(),
            "bin/cortex readable after {stop:?}"
        );

        // and the next attempt completes it, to the same final state every time
        home.commit(&v(RC2)).unwrap();
        assert_eq!(home.current().unwrap(), Some(v(RC2)));
        assert_eq!(home.previous().unwrap(), Some(v(RC1)));
        assert!(!p.join("releases").join(format!("{RC2}.part")).exists());
    }
}

#[test]
fn lay_06_a_resumed_commit_twice_is_the_same_as_once() {
    let (t, home) = home_with_staged();
    let _ = home.commit_with(&v(RC2), &mut |s| s != CommitStep::PreviousSet);
    home.commit(&v(RC2)).unwrap();
    let once = tree_hash(t.path());
    home.commit(&v(RC2)).unwrap();
    assert_eq!(tree_hash(t.path()), once);
}

// ---------------------------------------------------------------- UP-11: the flip back

#[test]
fn lay_07_flip_back_swaps_the_links_and_leaves_data_alone() {
    let (t, home) = home_with_staged();
    home.commit(&v(RC2)).unwrap();
    let p = t.path();
    let (toml, dumps, rc2) = (
        tree_hash(&p.join("cortex.toml")),
        tree_hash(&p.join("dumps")),
        tree_hash(&p.join("releases").join(RC2)),
    );
    let flip = home.flip_back().unwrap();
    assert_eq!((flip.left, flip.now), (v(RC2), v(RC1)));
    assert_eq!(home.current().unwrap(), Some(v(RC1)));
    assert_eq!(home.previous().unwrap(), Some(v(RC2)));
    assert!(
        fs::read_to_string(p.join("bin/cortex"))
            .unwrap()
            .contains(RC1)
    );
    assert_eq!(
        (
            tree_hash(&p.join("cortex.toml")),
            tree_hash(&p.join("dumps")),
            tree_hash(&p.join("releases").join(RC2))
        ),
        (toml, dumps, rc2)
    );
}

#[test]
fn lay_08_flip_back_is_refused_for_a_contract_release() {
    let t = Tmp::new("layout");
    install(t.path(), RC1);
    stage(t.path(), RC2, "contract");
    let home = Home::new(t.path());
    home.commit(&v(RC2)).unwrap();
    let before = tree_hash(t.path());
    assert_eq!(
        home.flip_back(),
        Err(LayoutError::RollbackRefused(
            RollbackRefusal::ContractRelease
        ))
    );
    assert_eq!(tree_hash(t.path()), before, "a refusal changes nothing");
    assert_eq!(home.current().unwrap(), Some(v(RC2)));
}

#[test]
fn lay_09_flip_back_is_refused_when_it_cannot_be_sure() {
    // no previous
    let t = Tmp::new("layout");
    install(t.path(), RC1);
    let home = Home::new(t.path());
    let before = tree_hash(t.path());
    assert_eq!(
        home.flip_back(),
        Err(LayoutError::RollbackRefused(RollbackRefusal::NoPrevious))
    );
    assert_eq!(tree_hash(t.path()), before);

    // previous is the current release (an interrupted switch)
    let (t, home) = home_with_staged();
    let _ = home.commit_with(&v(RC2), &mut |s| s != CommitStep::PreviousSet);
    let before = tree_hash(t.path());
    assert_eq!(
        home.flip_back(),
        Err(LayoutError::RollbackRefused(
            RollbackRefusal::PreviousIsCurrent
        ))
    );
    assert_eq!(tree_hash(t.path()), before);

    // the manifest copy of the release being left is missing, unreadable or does not list it
    for damage in ["missing", "garbage", "other-version"] {
        let (t, home) = home_with_staged();
        home.commit(&v(RC2)).unwrap();
        let m = t.path().join("releases").join(RC2).join("manifest.json");
        match damage {
            "missing" => fs::remove_file(&m).unwrap(),
            "garbage" => fs::write(&m, b"not json").unwrap(),
            _ => write_release_files(&t.path().join("releases").join(RC2), RC3, "expand"),
        }
        let before = tree_hash(t.path());
        assert_eq!(
            home.flip_back(),
            Err(LayoutError::RollbackRefused(
                RollbackRefusal::ManifestUnreadable
            )),
            "{damage}"
        );
        assert_eq!(tree_hash(t.path()), before, "{damage}");
    }
}

// ---------------------------------------------------------------- UP-22: prune never touches data

#[test]
fn lay_10_prune_keeps_current_and_previous_and_touches_nothing_else() {
    let t = Tmp::new("layout");
    install(t.path(), RC1);
    stage(t.path(), RC2, "expand");
    let home = Home::new(t.path());
    home.commit(&v(RC2)).unwrap();
    stage(t.path(), RC3, "expand");
    home.commit(&v(RC3)).unwrap();
    let p = t.path();
    // things that are not releases of this home must be left alone
    fs::create_dir_all(p.join("releases/not-a-version")).unwrap();
    fs::write(p.join("releases/not-a-version/x"), "x").unwrap();
    fs::create_dir_all(p.join("releases/0.2.1-rc.9.part")).unwrap();
    fs::write(p.join("releases/README"), "r").unwrap();
    symlink("../dumps/set-1", p.join("releases/0.2.1-rc.8")).unwrap(); // a link NAMED like a version, pointing at a dump set
    let (dumps, upgrades, toml, rc2, rc3) = (
        tree_hash(&p.join("dumps")),
        tree_hash(&p.join("upgrades")),
        tree_hash(&p.join("cortex.toml")),
        tree_hash(&p.join("releases").join(RC2)),
        tree_hash(&p.join("releases").join(RC3)),
    );

    assert_eq!(home.prune().unwrap(), vec![v(RC1)]);

    assert!(!p.join("releases").join(RC1).exists());
    assert_eq!(tree_hash(&p.join("releases").join(RC2)), rc2);
    assert_eq!(tree_hash(&p.join("releases").join(RC3)), rc3);
    assert_eq!(
        (
            tree_hash(&p.join("dumps")),
            tree_hash(&p.join("upgrades")),
            tree_hash(&p.join("cortex.toml"))
        ),
        (dumps, upgrades, toml)
    );
    assert!(
        p.join("releases/not-a-version/x").is_file()
            && p.join("releases/0.2.1-rc.9.part").is_dir()
            && p.join("releases/README").is_file()
    );
    assert!(
        fs::symlink_metadata(p.join("releases/0.2.1-rc.8"))
            .unwrap()
            .file_type()
            .is_symlink(),
        "the link was not followed or removed"
    );
    assert!(p.join("dumps/set-1/dump.sql").is_file());
    assert_eq!(
        home.prune().unwrap(),
        Vec::<semver::Version>::new(),
        "a second prune has nothing to do"
    );
}

#[test]
fn lay_11_after_a_rollback_prune_keeps_the_older_current_and_the_newer_previous() {
    let t = Tmp::new("layout");
    install(t.path(), "0.2.1-rc.0");
    for (n, ver) in [(1, RC1), (2, RC2)] {
        let _ = n;
        stage(t.path(), ver, "expand");
        Home::new(t.path()).commit(&v(ver)).unwrap();
    }
    let home = Home::new(t.path());
    home.flip_back().unwrap(); // current rc.1, previous rc.2
    assert_eq!(home.prune().unwrap(), vec![v("0.2.1-rc.0")]);
    assert_eq!(home.current().unwrap(), Some(v(RC1)));
    assert!(
        t.path().join("releases").join(RC1).is_dir()
            && t.path().join("releases").join(RC2).is_dir()
    );
}

// ---------------------------------------------------------------- UP-12: one upgrade at a time

/// `home.lock()` on another thread, waited for at most `limit`; `None` means it was still waiting.
fn lock_within(
    home: &Home,
    limit: Duration,
) -> Option<Result<cortex_upgrade::layout::UpgradeLock, LockError>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let h = home.clone();
    std::thread::spawn(move || {
        let _ = tx.send(h.lock());
    });
    rx.recv_timeout(limit).ok()
}

#[test]
fn lay_12_the_lock_is_refused_at_once_when_held_and_free_after_drop() {
    let t = Tmp::new("layout");
    install(t.path(), RC1);
    let home = Home::new(t.path());
    let first = home.lock().expect("control: the first lock is granted");
    let started = Instant::now();
    let second = lock_within(&home, Duration::from_secs(1));
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!(
        second.map(|r| r.err()),
        Some(Some(LockError::Busy)),
        "refused at once, not waited for"
    );
    let third = lock_within(&home, Duration::from_secs(1));
    assert_eq!(
        third.map(|r| r.err()),
        Some(Some(LockError::Busy)),
        "a refused attempt does not release or break the lock"
    );
    let lock_file = t.path().join("upgrade.lock");
    assert!(lock_file.is_file());
    assert_eq!(
        fs::metadata(&lock_file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    drop(first);
    let again = home.lock().expect("free after the holder is dropped");
    drop(again);
    assert!(
        lock_file.is_file(),
        "the lock file stays; the lock is the flock, not the file"
    );
}

#[test]
fn lay_13_a_lock_held_by_someone_else_is_respected() {
    let t = Tmp::new("layout");
    install(t.path(), RC1);
    drop(
        Home::new(t.path())
            .lock()
            .expect("control: with nobody holding it, the lock is granted"),
    );
    let outside = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(t.path().join("upgrade.lock"))
        .unwrap();
    outside.lock().unwrap();
    let r = lock_within(&Home::new(t.path()), Duration::from_secs(1));
    assert_eq!(
        r.map(|r| r.err()),
        Some(Some(LockError::Busy)),
        "a lock held by another descriptor is respected, not waited for"
    );
}

#[test]
fn lay_14_a_lock_file_that_is_a_link_is_refused_and_its_target_is_not_touched() {
    let t = Tmp::new("layout");
    install(t.path(), RC1);
    fs::write(t.path().join("victim"), "VICTIM\n").unwrap();
    symlink("victim", t.path().join("upgrade.lock")).unwrap();
    let r = Home::new(t.path()).lock();
    assert!(matches!(r, Err(LockError::Io(_))), "{r:?}");
    assert_eq!(
        fs::read_to_string(t.path().join("victim")).unwrap(),
        "VICTIM\n"
    );
}

// ---------------------------------------------------------------- the record

#[test]
fn lay_15_a_record_is_written_once_per_attempt_and_never_overwritten() {
    let t = Tmp::new("layout");
    install(t.path(), RC1);
    let home = Home::new(t.path());
    let rec = Record {
        from: RC1.into(),
        to: RC2.into(),
        result: RecordResult::RolledBack,
        steps: vec![
            StepRecord {
                step: 6,
                name: "dump".into(),
                ok: true,
            },
            StepRecord {
                step: 9,
                name: "migrate".into(),
                ok: false,
            },
        ],
        dump_set: Some("dumps/set-1".into()),
        members: Vec::new(),
    };
    let clock = FixedClock(1_793_968_496); // 2026-11-06T12:34:56Z
    let a = home.write_record(&clock, &rec).unwrap();
    assert_eq!(a, t.path().join("upgrades/20261106T123456Z.json"));
    assert_eq!(
        fs::metadata(&a).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let first_bytes = fs::read(&a).unwrap();
    let back: Record = serde_json::from_slice(&first_bytes).unwrap();
    assert_eq!(back, rec);
    let keys: std::collections::BTreeSet<String> =
        serde_json::from_slice::<serde_json::Value>(&first_bytes)
            .unwrap()
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
    assert_eq!(
        keys,
        ["dump_set", "from", "result", "steps", "to"]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        "typed fields only"
    );

    // the same second again: a new file, the first one untouched
    let b = home.write_record(&clock, &rec).unwrap();
    assert_eq!(b, t.path().join("upgrades/20261106T123456Z-2.json"));
    assert_eq!(fs::read(&a).unwrap(), first_bytes);
    let c = home.write_record(&clock, &rec).unwrap();
    assert_eq!(c, t.path().join("upgrades/20261106T123456Z-3.json"));
}

#[test]
fn lay_16_the_stamp_is_utc_and_right_on_leap_days_and_the_epoch() {
    for (secs, want) in [
        (0, "19700101T000000Z"),
        (951_868_799, "20000229T235959Z"),
        (1_000_000_000, "20010909T014640Z"),
        (4_107_542_400, "21000301T000000Z"),
    ] {
        let t = Tmp::new("layout");
        let home = Home::new(t.path());
        let rec = Record {
            from: RC1.into(),
            to: RC2.into(),
            result: RecordResult::Done,
            steps: vec![],
            dump_set: None,
            members: Vec::new(),
        };
        let path = home.write_record(&FixedClock(secs), &rec).unwrap();
        assert_eq!(
            path.file_name().unwrap().to_str().unwrap(),
            format!("{want}.json"),
            "{secs}"
        );
    }
}

// ---------------------------------------------------------------- stack.env as read from a release

#[test]
fn lay_17_stack_env_is_read_from_the_release_and_must_name_that_release() {
    let t = Tmp::new("layout");
    install(t.path(), RC1);
    let home = Home::new(t.path());
    let se = home.read_stack_env(&v(RC1)).unwrap();
    assert_eq!(se.version, v(RC1));
    assert!(
        se.server_image
            .0
            .starts_with("localhost/cortex-server@sha256:")
    );
    // a stack.env that says another version than its directory is refused
    let p = t.path().join("releases").join(RC1).join("stack.env");
    let text = fs::read_to_string(&p).unwrap().replace(
        &format!("CORTEX_VERSION={RC1}"),
        &format!("CORTEX_VERSION={RC2}"),
    );
    fs::write(&p, text).unwrap();
    assert!(matches!(
        home.read_stack_env(&v(RC1)),
        Err(LayoutError::StackEnv(_))
    ));
    // a missing file is an I/O error, not a panic
    fs::remove_file(&p).unwrap();
    assert!(matches!(
        home.read_stack_env(&v(RC1)),
        Err(LayoutError::Io { .. })
    ));
}

// ---------------------------------------------------------------- the swap is atomic for a reader

#[test]
fn lay_18_a_reader_never_finds_current_missing_while_the_links_are_swapped() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let (t, home) = home_with_staged();
    home.commit(&v(RC2)).unwrap();
    let (stop, missing, reads) = (
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicUsize::new(0)),
        Arc::new(AtomicUsize::new(0)),
    );
    let reader = {
        let (stop, missing, reads, p) = (
            stop.clone(),
            missing.clone(),
            reads.clone(),
            t.path().to_path_buf(),
        );
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                if fs::read_to_string(p.join("current/compose.yml")).is_err() {
                    missing.fetch_add(1, Ordering::Relaxed);
                }
                reads.fetch_add(1, Ordering::Relaxed);
            }
        })
    };
    for _ in 0..60 {
        home.flip_back().unwrap(); // swaps current and previous; both releases are `expand`
    }
    stop.store(true, Ordering::Relaxed);
    reader.join().unwrap();
    assert!(
        reads.load(Ordering::Relaxed) > 1000,
        "the reader really ran"
    );
    assert_eq!(
        missing.load(Ordering::Relaxed),
        0,
        "current was missing for a reader"
    );
}

// ---------------------------------------------------------------- mike's finding on packet 2: an interrupted rollback

use cortex_upgrade::layout::FlipStep;

fn files_named_like_temp(p: &std::path::Path) -> Vec<String> {
    names(p).into_iter().filter(|n| n.contains("tmp")).collect()
}

/// rc.1 installed, rc.2 committed: `current` is rc.2 and `previous` is rc.1.
fn after_commit() -> (Tmp, Home) {
    let (t, home) = home_with_staged();
    home.commit(&v(RC2)).unwrap();
    (t, home)
}

#[test]
fn lay_19_a_rollback_interrupted_at_every_step_loses_nothing_and_recover_finishes_it() {
    for stop in [
        FlipStep::IntentWritten,
        FlipStep::PreviousSet,
        FlipStep::CurrentSet,
    ] {
        let (t, home) = after_commit();
        let p = t.path();
        let r = home.flip_back_with(&mut |s| s != stop);
        assert_eq!(r, Err(LayoutError::FlipInterrupted(stop)), "{stop:?}");
        // at no point is a link missing or dangling
        assert!(
            resolves_to_a_release(&p.join("current")),
            "current after {stop:?}"
        );
        assert!(
            resolves_to_a_release(&p.join("previous")),
            "previous after {stop:?}"
        );
        // the way back is written down durably, and its mode is private
        let intent = p.join("rollback.intent");
        assert!(intent.is_file(), "an intent file after {stop:?}");
        assert_eq!(
            fs::metadata(&intent).unwrap().permissions().mode() & 0o777,
            0o600
        );
        // while it is pending, nothing may delete the rollback target or start something else
        assert_eq!(
            home.prune(),
            Err(LayoutError::RecoveryPending),
            "prune after {stop:?}"
        );
        assert_eq!(
            home.flip_back(),
            Err(LayoutError::RecoveryPending),
            "a second rollback after {stop:?}"
        );
        stage(p, RC3, "expand");
        assert_eq!(
            home.commit(&v(RC3)),
            Err(LayoutError::RecoveryPending),
            "a commit after {stop:?}"
        );
        assert!(
            p.join("releases").join(RC1).is_dir(),
            "the rollback target still exists after {stop:?}"
        );
        // recover completes the rollback to the same final state every time
        let flip = home.recover().unwrap().expect("a pending rollback");
        assert_eq!((flip.left, flip.now), (v(RC2), v(RC1)), "{stop:?}");
        assert_eq!(
            (home.current().unwrap(), home.previous().unwrap()),
            (Some(v(RC1)), Some(v(RC2))),
            "{stop:?}"
        );
        assert!(
            !intent.exists() && files_named_like_temp(p).is_empty(),
            "no intent or temporary file left after {stop:?}"
        );
        assert_eq!(
            home.recover(),
            Ok(None),
            "a second recover has nothing to do"
        );
        // and the home works again
        home.commit(&v(RC3)).unwrap();
    }
}

#[test]
fn lay_20_a_normal_rollback_leaves_no_intent_and_recover_without_one_changes_nothing() {
    let (t, home) = after_commit();
    let before = tree_hash(t.path());
    assert_eq!(home.recover(), Ok(None));
    assert_eq!(
        tree_hash(t.path()),
        before,
        "recover with no intent writes nothing"
    );
    home.flip_back().unwrap();
    assert!(!t.path().join("rollback.intent").exists());
    assert!(files_named_like_temp(t.path()).is_empty());
    assert_eq!(
        (home.current().unwrap(), home.previous().unwrap()),
        (Some(v(RC1)), Some(v(RC2)))
    );
}

#[test]
fn lay_21_an_intent_file_that_is_not_valid_is_refused_and_left_alone() {
    let (t, home) = after_commit();
    let p = t.path();
    let ok_text = format!("left={RC2}\nback={RC1}\n");
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("empty", vec![]),
        ("garbage", b"hello".to_vec()),
        ("one line", format!("left={RC2}\n").into_bytes()),
        (
            "swapped keys",
            format!("back={RC1}\nleft={RC2}\n").into_bytes(),
        ),
        ("a third line", format!("{ok_text}extra=1\n").into_bytes()),
        (
            "a path as a version",
            format!("left=../{RC2}\nback={RC1}\n").into_bytes(),
        ),
        (
            "not a version",
            format!("left=0.2.001\nback={RC1}\n").into_bytes(),
        ),
        (
            "both the same",
            format!("left={RC1}\nback={RC1}\n").into_bytes(),
        ),
        (
            "a release that is not there",
            "left=0.2.1-rc.7\nback=0.2.1-rc.8\n".as_bytes().to_vec(),
        ),
        (
            "only the first release is not there",
            format!("left=0.2.1-rc.7\nback={RC1}\n").into_bytes(),
        ),
        (
            "only the second release is not there",
            format!("left={RC2}\nback=0.2.1-rc.8\n").into_bytes(),
        ),
        ("too large", vec![b'x'; 5000]),
        ("not text", vec![0xff, 0xfe, 0x00]),
    ];
    for (what, bytes) in cases {
        fs::write(p.join("rollback.intent"), &bytes).unwrap();
        let before = tree_hash(p);
        assert_eq!(home.recover(), Err(LayoutError::BadIntent), "{what}");
        assert_eq!(tree_hash(p), before, "{what}: nothing was changed");
        assert_eq!(
            home.prune(),
            Err(LayoutError::RecoveryPending),
            "{what}: still fail closed"
        );
    }
    // control: the valid text is accepted (and finishes the rollback)
    fs::write(p.join("rollback.intent"), ok_text).unwrap();
    assert!(home.recover().unwrap().is_some());
}

// ---------------------------------------------------------------- engine step: after a rollback `previous` is forgotten

#[test]
fn lay_22_drop_previous_removes_the_link_and_nothing_else_and_is_safe_to_repeat() {
    let (t, home) = after_commit();
    let p = t.path();
    let before_releases = tree_hash(&p.join("releases"));
    assert_eq!(home.previous().unwrap(), Some(v(RC1)));
    home.drop_previous().unwrap();
    assert_eq!(home.previous().unwrap(), None);
    assert!(
        fs::symlink_metadata(p.join("previous")).is_err(),
        "the link is gone"
    );
    assert_eq!(
        home.current().unwrap(),
        Some(v(RC2)),
        "current is untouched"
    );
    assert_eq!(
        tree_hash(&p.join("releases")),
        before_releases,
        "no release directory is touched"
    );
    assert!(files_named_like_temp(p).is_empty());
    // again: nothing to do, nothing wrong
    home.drop_previous().unwrap();
    assert_eq!(home.previous().unwrap(), None);
    // with no previous a rollback is refused, which is the point
    assert!(home.flip_back().is_err());
}

#[test]
fn lay_23_drop_previous_refuses_while_a_rollback_is_pending_and_leaves_odd_things_alone() {
    // a rollback stopped half way: nothing may be removed until it is finished
    let (t, home) = after_commit();
    assert!(
        home.flip_back_with(&mut |s| s != FlipStep::IntentWritten)
            .is_err()
    );
    let before = tree_hash(t.path());
    assert_eq!(home.drop_previous(), Err(LayoutError::RecoveryPending));
    assert_eq!(tree_hash(t.path()), before);
    // a `previous` that is not exactly releases/<version> is not ours to remove
    let (t, home) = after_commit();
    fs::remove_file(t.path().join("previous")).unwrap();
    symlink("../outside", t.path().join("previous")).unwrap();
    let before = tree_hash(t.path());
    assert!(matches!(
        home.drop_previous(),
        Err(LayoutError::BadLink { .. })
    ));
    assert_eq!(tree_hash(t.path()), before);
    // a real directory called previous
    let (t, home) = after_commit();
    fs::remove_file(t.path().join("previous")).unwrap();
    fs::create_dir(t.path().join("previous")).unwrap();
    fs::write(t.path().join("previous/keep"), "keep").unwrap();
    let before = tree_hash(t.path());
    assert!(matches!(
        home.drop_previous(),
        Err(LayoutError::BadLink { .. })
    ));
    assert_eq!(tree_hash(t.path()), before);
}

// ---------------------------------------------------------------- mike, packet 7: a rollback is not finished until the old release runs

fn marker(p: &std::path::Path) -> std::path::PathBuf {
    p.join("rollback.unfinished")
}

fn refuses_everything(home: &Home, p: &std::path::Path, when: &str) {
    assert_eq!(
        home.prune(),
        Err(LayoutError::RecoveryPending),
        "prune {when}"
    );
    assert_eq!(
        home.begin_rollback(),
        Err(LayoutError::RecoveryPending),
        "begin_rollback {when}"
    );
    assert_eq!(
        home.flip_back(),
        Err(LayoutError::RecoveryPending),
        "flip_back {when}"
    );
    assert_eq!(
        home.drop_previous(),
        Err(LayoutError::RecoveryPending),
        "drop_previous {when}"
    );
    stage(p, RC3, "expand");
    assert_eq!(
        home.commit(&v(RC3)),
        Err(LayoutError::RecoveryPending),
        "commit {when}"
    );
    fs::remove_dir_all(p.join("releases").join(format!("{RC3}.part"))).unwrap();
}

#[test]
fn lay_24_a_marked_rollback_keeps_a_private_marker_until_it_is_finished_and_everything_refuses_meanwhile()
 {
    let (t, home) = after_commit();
    let p = t.path();
    assert_eq!(home.unfinished_rollback(), Ok(None));
    let flip = home.begin_rollback().unwrap();
    assert_eq!((flip.left.clone(), flip.now.clone()), (v(RC2), v(RC1)));
    // the links are flipped, the intent is gone, the marker stays
    assert_eq!(
        (home.current().unwrap(), home.previous().unwrap()),
        (Some(v(RC1)), Some(v(RC2)))
    );
    assert!(!p.join("rollback.intent").exists());
    assert!(marker(p).is_file(), "the marker stays after the flip");
    assert_eq!(
        fs::metadata(marker(p)).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(files_named_like_temp(p).is_empty());
    assert_eq!(home.unfinished_rollback(), Ok(Some(flip)));
    refuses_everything(&home, p, "while the marker stands");
    // finishing forgets `previous` and the marker, once, and is safe to repeat
    home.finish_rollback().unwrap();
    assert_eq!(
        (home.current().unwrap(), home.previous().unwrap()),
        (Some(v(RC1)), None)
    );
    assert!(!marker(p).exists());
    assert_eq!(home.unfinished_rollback(), Ok(None));
    home.finish_rollback().unwrap();
    // and the home works again; the release that was left is only pruned now
    stage(p, RC3, "expand");
    home.commit(&v(RC3)).unwrap();
    assert_eq!(home.prune().unwrap(), vec![v(RC2)]);
    // the plain flip is as it was: no marker (positive control, passes at once)
    let (t2, home2) = after_commit();
    home2.flip_back().unwrap();
    assert!(!marker(t2.path()).exists());
    assert_eq!(home2.unfinished_rollback(), Ok(None));
}

#[test]
fn lay_25_a_marked_rollback_interrupted_at_every_step_is_finished_by_recover_and_keeps_its_marker()
{
    for stop in [
        FlipStep::IntentWritten,
        FlipStep::PreviousSet,
        FlipStep::CurrentSet,
        FlipStep::MarkerWritten,
    ] {
        let (t, home) = after_commit();
        let p = t.path();
        let r = home.begin_rollback_with(&mut |s| s != stop);
        assert_eq!(r, Err(LayoutError::FlipInterrupted(stop)), "{stop:?}");
        assert!(
            resolves_to_a_release(&p.join("current")) && resolves_to_a_release(&p.join("previous")),
            "no dangling link after {stop:?}"
        );
        refuses_everything(&home, p, &format!("after {stop:?}"));
        assert_eq!(
            home.finish_rollback(),
            Err(LayoutError::RecoveryPending),
            "a flip that is not recovered yet is not finished {stop:?}"
        );
        let flip = home.recover().unwrap().expect("a pending rollback");
        assert_eq!(
            (flip.left.clone(), flip.now.clone()),
            (v(RC2), v(RC1)),
            "{stop:?}"
        );
        assert_eq!(
            (home.current().unwrap(), home.previous().unwrap()),
            (Some(v(RC1)), Some(v(RC2))),
            "{stop:?}"
        );
        assert!(!p.join("rollback.intent").exists(), "{stop:?}");
        assert!(
            marker(p).is_file(),
            "the marker survives recover after {stop:?}"
        );
        assert!(files_named_like_temp(p).is_empty(), "{stop:?}");
        assert_eq!(home.unfinished_rollback(), Ok(Some(flip)), "{stop:?}");
        assert_eq!(
            home.recover(),
            Ok(None),
            "a second recover has nothing to do"
        );
        refuses_everything(&home, p, &format!("after recover, {stop:?}"));
        home.finish_rollback().unwrap();
        assert_eq!(home.previous().unwrap(), None, "{stop:?}");
        assert!(!marker(p).exists(), "{stop:?}");
    }
}

#[test]
fn lay_26_an_unreadable_marker_or_an_odd_previous_stops_finish_and_changes_nothing() {
    // garbage, equal versions, a missing release, an over-long file: BadIntent, and everything keeps refusing
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("garbage", b"garbage".to_vec()),
        (
            "equal versions",
            format!("left={RC2}\nback={RC2}\n").into_bytes(),
        ),
        (
            "a release that does not exist",
            format!("left={RC2}\nback=0.9.9\n").into_bytes(),
        ),
        ("too long", vec![b'x'; 600]),
        (
            "a third line",
            format!("left={RC2}\nback={RC1}\nextra\n").into_bytes(),
        ),
    ];
    for (what, bytes) in cases {
        let (t, home) = after_commit();
        let p = t.path();
        fs::write(marker(p), &bytes).unwrap();
        let before = tree_hash(p);
        assert_eq!(
            home.unfinished_rollback(),
            Err(LayoutError::BadIntent),
            "{what}"
        );
        assert_eq!(
            home.finish_rollback(),
            Err(LayoutError::BadIntent),
            "{what}"
        );
        assert_eq!(
            home.begin_rollback(),
            Err(LayoutError::RecoveryPending),
            "{what}"
        );
        assert_eq!(tree_hash(p), before, "nothing changed: {what}");
    }
    // a `previous` that is not a plain release link is not ours to remove; the marker stays
    let (t, home) = after_commit();
    let p = t.path();
    home.begin_rollback().unwrap();
    fs::remove_file(p.join("previous")).unwrap();
    fs::create_dir(p.join("previous")).unwrap();
    fs::write(p.join("previous/keep"), "keep").unwrap();
    let before = tree_hash(p);
    assert!(matches!(
        home.finish_rollback(),
        Err(LayoutError::BadLink { .. })
    ));
    assert_eq!(tree_hash(p), before, "the marker and the directory stay");
    assert!(marker(p).is_file());
    // a `previous` that names another release than the one that was left is not removed either
    let (t, home) = after_commit();
    let p = t.path();
    home.begin_rollback().unwrap();
    fs::remove_file(p.join("previous")).unwrap();
    symlink(format!("releases/{RC1}"), p.join("previous")).unwrap();
    let before = tree_hash(p);
    assert!(matches!(
        home.finish_rollback(),
        Err(LayoutError::BadLink { .. })
    ));
    assert_eq!(tree_hash(p), before);
}

#[test]
fn lay_27_a_marker_that_disagrees_with_current_is_not_trusted_and_the_tail_line_is_exactly_tail_1()
{
    // the marker says the rollback went to rc.1, but `current` is rc.2: never guessed
    let (t, home) = after_commit();
    let p = t.path();
    fs::write(marker(p), format!("left={RC2}\nback={RC1}\n")).unwrap();
    let before = tree_hash(p);
    assert_eq!(home.unfinished_rollback(), Err(LayoutError::BadIntent));
    assert_eq!(home.finish_rollback(), Err(LayoutError::BadIntent));
    assert_eq!(tree_hash(p), before);
    // the last line of an intent is `tail=1` and nothing else
    let pair = format!("left={RC2}\nback={RC1}\n");
    let bad: Vec<(&str, String)> = vec![
        ("tail=2", format!("{pair}tail=2\n")),
        ("tail=10", format!("{pair}tail=10\n")),
        ("tail= empty", format!("{pair}tail=\n")),
        ("no newline", format!("{pair}tail=1")),
        ("twice", format!("{pair}tail=1\ntail=1\n")),
        ("before the pair", format!("tail=1\n{pair}")),
    ];
    for (what, text) in bad {
        let (t, home) = after_commit();
        fs::write(t.path().join("rollback.intent"), text).unwrap();
        let before = tree_hash(t.path());
        assert_eq!(home.recover(), Err(LayoutError::BadIntent), "{what}");
        assert_eq!(tree_hash(t.path()), before, "{what}: nothing was changed");
    }
    // the marker is the pair alone: a tail line in it is refused
    let (t, home) = after_commit();
    home.begin_rollback().unwrap();
    fs::write(marker(t.path()), format!("{pair}tail=1\n")).unwrap();
    assert_eq!(home.unfinished_rollback(), Err(LayoutError::BadIntent));
    // control: exactly `tail=1` is accepted and the marker is written by recover
    let (t, home) = after_commit();
    fs::write(t.path().join("rollback.intent"), format!("{pair}tail=1\n")).unwrap();
    let flip = home.recover().unwrap().expect("a pending rollback");
    assert_eq!(home.unfinished_rollback(), Ok(Some(flip)));
}

#[test]
fn lay_28_an_io_error_after_the_intent_was_published_is_flip_failed_and_one_before_it_is_not() {
    let pid = std::process::id();
    let block = |p: &std::path::Path, name: &str| {
        fs::create_dir(p.join(format!("{name}.tmp{pid}"))).unwrap()
    };
    // before anything is written: a plain I/O error, nothing changed, nothing to recover
    let (t, home) = after_commit();
    let p = t.path();
    block(p, ".rollback.intent");
    let before = tree_hash(p);
    assert!(matches!(home.begin_rollback(), Err(LayoutError::Io { .. })));
    assert_eq!(tree_hash(p), before, "nothing changed");
    assert!(!p.join("rollback.intent").exists());
    // after the intent was published, at each later effect: FlipFailed, the intent kept, and recover finishes it later
    for (stem, links) in [
        (".previous", (RC2, RC1)),
        (".current", (RC2, RC2)),
        (".rollback.unfinished", (RC1, RC2)),
    ] {
        let (t, home) = after_commit();
        let p = t.path();
        block(p, stem);
        let r = home.begin_rollback();
        assert!(
            matches!(r, Err(LayoutError::FlipFailed { .. })),
            "{stem}: {r:?}"
        );
        assert!(
            p.join("rollback.intent").is_file(),
            "{stem}: the intent is kept"
        );
        assert_eq!(
            (home.current().unwrap(), home.previous().unwrap()),
            (Some(v(links.0)), Some(v(links.1))),
            "{stem}: the links are where the failed effect left them"
        );
        fs::remove_dir(p.join(format!("{stem}.tmp{pid}"))).unwrap();
        let flip = home.recover().unwrap().expect("a pending rollback");
        assert_eq!((flip.left, flip.now), (v(RC2), v(RC1)), "{stem}");
        assert!(marker(p).is_file(), "{stem}: recover writes the marker");
    }
    // recover itself: an I/O error while redoing the links is FlipFailed too, and the intent stays
    let (t, home) = after_commit();
    let p = t.path();
    assert!(
        home.begin_rollback_with(&mut |s| s != FlipStep::IntentWritten)
            .is_err()
    );
    block(p, ".previous");
    let r = home.recover();
    assert!(
        matches!(r, Err(LayoutError::FlipFailed { .. })),
        "recover: {r:?}"
    );
    assert!(p.join("rollback.intent").is_file());
    // and when the MARKER cannot be written while recover redoes a marked flip
    let (t, home) = after_commit();
    let p = t.path();
    assert!(
        home.begin_rollback_with(&mut |s| s != FlipStep::CurrentSet)
            .is_err()
    );
    block(p, ".rollback.unfinished");
    let r = home.recover();
    assert!(
        matches!(r, Err(LayoutError::FlipFailed { .. })),
        "recover, marker: {r:?}"
    );
    assert!(p.join("rollback.intent").is_file());
    // a precondition refusal is not an I/O error at all (positive control, passes at once)
    let (t, home) = home_with_staged();
    assert_eq!(
        home.begin_rollback(),
        Err(LayoutError::RollbackRefused(RollbackRefusal::NoPrevious))
    );
    assert!(!t.path().join("rollback.intent").exists());
}

#[test]
fn lay_29_when_the_intent_cannot_be_unlinked_the_flip_has_failed_after_it_started_and_the_intent_stays()
 {
    // the home directory goes read-only just before the intent is removed: the unlink fails (positive control for the claim that
    // FlipFailed means "the intent is still there"; a failure of the directory sync AFTER a good unlink cannot be forced, see design r12)
    // can this account unlink a file in a directory it cannot write (root can)? Decided by a probe, not by the result under test.
    let probe = Tmp::new("layout-probe");
    fs::write(probe.path().join("f"), "x").unwrap();
    fs::set_permissions(probe.path(), PermissionsExt::from_mode(0o555)).unwrap();
    let privileged = fs::remove_file(probe.path().join("f")).is_ok();
    let _ = fs::set_permissions(probe.path(), PermissionsExt::from_mode(0o755));
    if privileged {
        eprintln!("lay_29 SKIPPED: this account can unlink in a directory it cannot write (root?)");
        return;
    }
    let (t, home) = after_commit();
    let p = t.path().to_path_buf();
    let root = p.clone();
    let r = home.begin_rollback_with(&mut |s| {
        if s == FlipStep::MarkerWritten {
            fs::set_permissions(&root, PermissionsExt::from_mode(0o555)).unwrap();
        }
        true
    });
    fs::set_permissions(&p, PermissionsExt::from_mode(0o755)).unwrap();
    assert!(matches!(r, Err(LayoutError::FlipFailed { .. })), "{r:?}");
    assert!(p.join("rollback.intent").is_file(), "the intent stays");
    assert!(marker(&p).is_file(), "the marker was written");
    assert_eq!(
        (home.current().unwrap(), home.previous().unwrap()),
        (Some(v(RC1)), Some(v(RC2))),
        "the links are flipped"
    );
    let flip = home.recover().unwrap().expect("a pending rollback");
    assert_eq!((flip.left, flip.now), (v(RC2), v(RC1)));
    assert!(!p.join("rollback.intent").exists());
}

#[test]
fn lay_30_restore_previous_puts_the_link_back_as_it_was_and_touches_nothing_else() {
    let (t, home) = after_commit(); // current rc.2, previous rc.1
    let p = t.path();
    // already as it was: nothing to do
    let before = tree_hash(p);
    home.restore_previous(Some(&v(RC1))).unwrap();
    assert_eq!(tree_hash(p), before);
    // overwritten by a commit that failed half way: put back
    fs::remove_file(p.join("previous")).unwrap();
    symlink(format!("releases/{RC2}"), p.join("previous")).unwrap();
    home.restore_previous(Some(&v(RC1))).unwrap();
    assert_eq!(home.previous().unwrap(), Some(v(RC1)));
    assert_eq!(
        home.current().unwrap(),
        Some(v(RC2)),
        "current is untouched"
    );
    assert!(files_named_like_temp(p).is_empty());
    // there was none: remove the link, and again is fine
    home.restore_previous(None).unwrap();
    assert_eq!(home.previous().unwrap(), None);
    assert!(fs::symlink_metadata(p.join("previous")).is_err());
    home.restore_previous(None).unwrap();
    // a release directory that is not there is not a place to point at
    assert!(matches!(
        home.restore_previous(Some(&v("0.9.9"))),
        Err(LayoutError::BadLink { .. })
    ));
    assert_eq!(home.previous().unwrap(), None);
    // refused while a rollback is pending, and nothing changes
    let (t2, home2) = after_commit();
    assert!(
        home2
            .begin_rollback_with(&mut |s| s != FlipStep::IntentWritten)
            .is_err()
    );
    let before = tree_hash(t2.path());
    assert_eq!(
        home2.restore_previous(None),
        Err(LayoutError::RecoveryPending)
    );
    assert_eq!(
        home2.restore_previous(Some(&v(RC1))),
        Err(LayoutError::RecoveryPending),
        "also when the link is already where it should be"
    );
    assert_eq!(tree_hash(t2.path()), before);
    // has_unfinished_rollback: the intent, the marker, and neither
    assert!(home2.has_unfinished_rollback());
    let (t3, home3) = after_commit();
    assert!(!home3.has_unfinished_rollback());
    home3.begin_rollback().unwrap();
    assert!(home3.has_unfinished_rollback(), "the marker alone");
    home3.finish_rollback().unwrap();
    assert!(!home3.has_unfinished_rollback());
    drop((t, t3));
}
