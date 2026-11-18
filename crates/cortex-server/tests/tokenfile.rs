//! The fresh token file (contract r0.3, row `bootstrap`). DB-free. RED first.

use cortex_server::tokenfile::{write_fresh, write_fresh_with};
use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::PathBuf;

fn dir(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("tokenfile-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).expect("make the test dir");
    d
}

#[test]
fn tf_01_a_fresh_file_has_the_secret_and_mode_0600() {
    let d = dir("fresh");
    let p = d.join("lead.token");
    write_fresh(&p, b"secret-bytes-1").expect("write");
    assert_eq!(fs::read(&p).expect("read"), b"secret-bytes-1");
    assert_eq!(
        fs::metadata(&p).expect("meta").permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn tf_02_an_existing_file_is_never_overwritten() {
    let d = dir("exists");
    let p = d.join("lead.token");
    fs::write(&p, b"OLD").expect("seed");
    let e = write_fresh(&p, b"NEW").expect_err("must refuse");
    assert_eq!(e.kind(), ErrorKind::AlreadyExists);
    assert_eq!(
        fs::read(&p).expect("read"),
        b"OLD",
        "the old file is untouched"
    );
}

#[test]
fn tf_03_a_symlink_is_never_followed_existing_or_dangling() {
    let d = dir("symlink");
    let target = d.join("victim");
    fs::write(&target, b"VICTIM").expect("seed");
    let link = d.join("link-to-existing");
    symlink(&target, &link).expect("symlink");
    assert!(write_fresh(&link, b"NEW").is_err());
    assert_eq!(
        fs::read(&target).expect("read"),
        b"VICTIM",
        "the symlink target is untouched"
    );

    let dangling = d.join("link-dangling");
    let nowhere = d.join("created-through-link");
    symlink(&nowhere, &dangling).expect("symlink");
    assert!(write_fresh(&dangling, b"NEW").is_err());
    assert!(
        !nowhere.exists(),
        "a dangling symlink must not make the write create its target"
    );
}

#[test]
fn tf_04_a_missing_parent_or_a_directory_fails_and_leaves_nothing() {
    let d = dir("fail");
    let p = d.join("no-such-dir").join("lead.token");
    assert!(write_fresh(&p, b"x").is_err());
    assert!(!p.exists());
    assert!(
        !d.join("no-such-dir").exists(),
        "the parent is not created for the caller"
    );
    let sub = d.join("a-directory");
    fs::create_dir(&sub).expect("dir");
    assert!(write_fresh(&sub, b"x").is_err());
    assert!(sub.is_dir());
}

#[test]
fn tf_05_an_error_never_contains_the_secret() {
    let d = dir("leak");
    let p = d.join("lead.token");
    fs::write(&p, b"seed").expect("seed");
    let e = write_fresh(&p, b"TOPSECRET-TOKEN-VALUE").expect_err("must refuse");
    assert!(!format!("{e} {e:?}").contains("TOPSECRET-TOKEN-VALUE"));
}

#[test]
fn tf_06_a_write_or_sync_failure_after_the_create_leaves_no_file() {
    // vera's Low: the post-create failure path was not exercised. The seam stands in for the write and the sync.
    let d = dir("postcreate");
    let p = d.join("lead.token");
    let e = write_fresh_with(&p, b"TOPSECRET-TOKEN-VALUE", |f, _secret| {
        use std::io::Write;
        f.write_all(b"partial")?; // some bytes reach the file ...
        Err(std::io::Error::other("disk full")) // ... then the write (or the sync) fails
    })
    .expect_err("must fail");
    assert_eq!(e.to_string(), "disk full");
    assert!(!p.exists(), "no partial file may remain");
    assert!(!format!("{e} {e:?}").contains("TOPSECRET-TOKEN-VALUE"));
}

#[test]
fn tf_07_the_seam_hands_the_secret_to_the_writer_and_the_real_path_still_works() {
    let d = dir("seam");
    let p = d.join("lead.token");
    write_fresh_with(&p, b"abc", |f, secret| {
        use std::io::Write;
        f.write_all(secret)?;
        f.sync_all()
    })
    .expect("write");
    assert_eq!(fs::read(&p).expect("read"), b"abc");
    assert_eq!(
        fs::metadata(&p).expect("meta").permissions().mode() & 0o777,
        0o600
    );
}
