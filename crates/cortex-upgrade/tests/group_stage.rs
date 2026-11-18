//! R-4a, stage A (cox W1-65, W1-66, W1-67, W1-91): the VERIFIED bytes of `db-roles.sh` and `compose.yml` (marv's `verify.rs`, one public function:
//! each file read ONCE, that buffer hashed against `SHA256SUMS` and, for the script, against the manifest's roles digest, that buffer returned),
//! and the PRIVATE STAGE built from THOSE bytes (`src/stage.rs`): `<home>/.roles-stage-<release>`, 0700, the script 0500, `compose.yml` 0400,
//! a stale stage replaced only when it is a real directory of the user's own with mode 0700, every ancestor to `/` on the canonical path owned
//! by root or the user without a group or other write bit. DB-free; no engine yet (stage B).
mod common;

use common::Tmp;
use cortex_types::ModuleId;
use cortex_upgrade::ports::{Containers, PortError};
use cortex_upgrade::real::{ComposeEnv, Deadlines, RealContainers};
use cortex_upgrade::stage::{build_stage, check_ancestors};
use cortex_upgrade::verify::{
    ReleaseBytes, STAGED_FILE_MAX_BYTES, verify_release_dir, verify_release_dir_bytes,
};
use sha2::{Digest, Sha256};
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::time::Duration;

const SCRIPT: &[u8] = b"#!/bin/sh\necho roles\n";
const COMPOSE: &[u8] = b"name: cv2-s5\nservices: {}\n";

const NAMES: &[&str] = &[
    "cortex",
    "compose.yml",
    "stack.env",
    "db-roles.sh",
    "cortex.toml.example",
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
    "initdb/roles.sql",
    "manifest.json",
];

fn put(p: &Path, bytes: &[u8], mode: u32) {
    fs::write(p, bytes).unwrap();
    fs::set_permissions(p, fs::Permissions::from_mode(mode)).unwrap();
}

fn mode_of(name: &str) -> u32 {
    if name == "cortex" || name == "db-roles.sh" || name.starts_with("bin/") {
        0o755
    } else {
        0o644
    }
}

/// A valid release directory (the layout `verify_release_dir` accepts) holding `script` and `compose`; every mode set explicitly.
fn release(parent: &Path, script: &[u8], compose: &[u8]) -> PathBuf {
    let dir = parent.join("release");
    for d in ["", "initdb", "bin", "systemd"] {
        let p = dir.join(d);
        fs::create_dir_all(&p).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    }
    for name in NAMES {
        let bytes: Vec<u8> = match *name {
            "db-roles.sh" => script.to_vec(),
            "compose.yml" => compose.to_vec(),
            "stack.env" => format!(
                "CORTEX_VERSION=1.2.3\nCORTEX_SERVER_IMAGE=localhost/cortex-server@{}\nCORTEX_DB_IMAGE=example.invalid/db@{}\n",
                common::DIGEST_A,
                common::DIGEST_A
            )
            .into_bytes(),
            "manifest.json" => Vec::new(),
            other => other.as_bytes().to_vec(),
        };
        put(&dir.join(name), &bytes, mode_of(name));
    }
    write_manifest(&dir, script);
    seal(&dir);
    dir
}

fn write_manifest(dir: &Path, script: &[u8]) {
    let image = format!("localhost/cortex-server@{}", common::DIGEST_A);
    let database = format!("example.invalid/db@{}", common::DIGEST_A);
    let manifest = serde_json::json!({
        "schema": 1,
        "releases": [{
            "version": "1.2.3", "date": "2026-11-07T00:00:00Z", "notes_url": "https://example.invalid/",
            "assets": [{"name": "image.oci.tar", "url": "image.oci.tar", "digest": common::DIGEST_A}],
            "cortex": {"label": "1.2.3", "members": 2, "server_image": {"ref": image, "asset": "image.oci.tar"},
                "db_image": database, "roles_script_digest": format!("sha256:{:x}", Sha256::digest(script)),
                "sidecar_images": [], "migrations": [], "modules": ["audit"], "migration_class": "expand"}
        }]
    });
    put(
        &dir.join("manifest.json"),
        &serde_json::to_vec(&manifest).unwrap(),
        0o644,
    );
}

fn seal(dir: &Path) {
    let mut names = NAMES.to_vec();
    names.sort_unstable();
    let lines: String = names
        .iter()
        .map(|n| {
            format!(
                "{:x}  {n}\n",
                Sha256::digest(fs::read(dir.join(n)).unwrap())
            )
        })
        .collect();
    put(&dir.join("SHA256SUMS"), lines.as_bytes(), 0o644);
}

fn valid() -> (Tmp, PathBuf) {
    let t = Tmp::new("stage-release");
    let dir = release(t.path(), SCRIPT, COMPOSE);
    (t, dir)
}

fn refusal(r: Result<impl std::fmt::Debug, impl std::fmt::Display>) -> String {
    match r {
        Ok(v) => panic!("expected a refusal, got {v:?}"),
        Err(e) => e.to_string(),
    }
}

fn me() -> u32 {
    fs::metadata("/proc/self").unwrap().uid()
}

fn bytes() -> ReleaseBytes {
    ReleaseBytes {
        roles_script: SCRIPT.to_vec(),
        compose_yml: COMPOSE.to_vec(),
    }
}

/// A private home under the test's own temporary directory.
fn home_of(t: &Tmp) -> PathBuf {
    let home = t.path().join("home");
    fs::create_dir(&home).unwrap();
    fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).unwrap();
    home
}

fn listing(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

// ---- the verified bytes (marv's verify.rs)

#[test]
fn vb_01_the_verified_bytes_are_the_files_bytes_and_the_plain_verification_still_passes() {
    let (_t, dir) = valid();
    verify_release_dir(&dir).expect("the plain verification passes");
    let got = verify_release_dir_bytes(&dir).expect("the bytes function passes");
    assert_eq!(got.roles_script, SCRIPT);
    assert_eq!(got.compose_yml, COMPOSE);
}

#[test]
fn vb_02_every_refusal_of_the_plain_verification_is_the_same_refusal_of_the_bytes_function() {
    type Break = fn(&Path);
    let cases: Vec<(&str, Break)> = vec![
        ("a changed script", |d| {
            put(&d.join("db-roles.sh"), b"#!/bin/sh\nevil\n", 0o755)
        }),
        ("a changed compose.yml", |d| {
            put(&d.join("compose.yml"), b"name: other\n", 0o644)
        }),
        ("a script resealed but not in the manifest", |d| {
            put(&d.join("db-roles.sh"), b"#!/bin/sh\nevil\n", 0o755);
            seal(d);
        }),
        ("a script that is a link", |d| {
            fs::remove_file(d.join("db-roles.sh")).unwrap();
            symlink("stack.env", d.join("db-roles.sh")).unwrap();
        }),
        ("a compose.yml that is a link", |d| {
            fs::remove_file(d.join("compose.yml")).unwrap();
            symlink("stack.env", d.join("compose.yml")).unwrap();
        }),
        ("a compose.yml that is a FIFO", |d| {
            fs::remove_file(d.join("compose.yml")).unwrap();
            assert!(
                std::process::Command::new("mkfifo")
                    .arg(d.join("compose.yml"))
                    .status()
                    .unwrap()
                    .success()
            );
        }),
        ("a missing script", |d| {
            fs::remove_file(d.join("db-roles.sh")).unwrap()
        }),
        ("a script with another mode", |d| {
            fs::set_permissions(d.join("db-roles.sh"), fs::Permissions::from_mode(0o700)).unwrap();
        }),
    ];
    for (what, brk) in cases {
        let (_t, dir) = valid();
        brk(&dir);
        let plain = refusal(verify_release_dir(&dir));
        let with_bytes = refusal(verify_release_dir_bytes(&dir));
        assert_eq!(
            with_bytes, plain,
            "{what}: the bytes function refuses in the same words"
        );
    }
}

#[test]
fn vb_03_a_staged_file_over_the_limit_is_refused_and_exactly_the_limit_is_accepted() {
    assert_eq!(
        STAGED_FILE_MAX_BYTES,
        1024 * 1024,
        "the documented limit is 1 MiB, not whatever the constant says"
    );
    let big = |n: usize| {
        let mut v = b"#!/bin/sh\n".to_vec();
        v.resize(n, b'#');
        v
    };
    // exactly the limit: accepted, and the bytes come back whole
    let t = Tmp::new("stage-limit-ok");
    let dir = release(
        t.path(),
        &big(STAGED_FILE_MAX_BYTES),
        &big(STAGED_FILE_MAX_BYTES),
    );
    let got = verify_release_dir_bytes(&dir).expect("exactly the limit is accepted");
    assert_eq!(got.roles_script.len(), STAGED_FILE_MAX_BYTES);
    assert_eq!(got.compose_yml.len(), STAGED_FILE_MAX_BYTES);
    // one byte over, the script and the compose.yml each: refused (the checksums and the manifest are made to match, so only the limit refuses)
    let t = Tmp::new("stage-limit-script");
    let dir = release(t.path(), &big(STAGED_FILE_MAX_BYTES + 1), COMPOSE);
    let m = refusal(verify_release_dir_bytes(&dir));
    assert!(m.contains("size limit") && m.contains("db-roles.sh"), "{m}");
    let t = Tmp::new("stage-limit-compose");
    let dir = release(t.path(), SCRIPT, &big(STAGED_FILE_MAX_BYTES + 1));
    let m = refusal(verify_release_dir_bytes(&dir));
    assert!(m.contains("size limit") && m.contains("compose.yml"), "{m}");
}

// ---- the stage (src/stage.rs)

#[test]
fn ss_01_the_stage_holds_the_two_verified_files_with_their_modes() {
    let t = Tmp::new("stage-01");
    let home = home_of(&t);
    let stage = build_stage(&home, "1.2.3", &bytes()).expect("the stage is built");
    assert_eq!(stage.path(), home.join(".roles-stage-1.2.3"));
    let md = fs::symlink_metadata(stage.path()).unwrap();
    assert!(md.file_type().is_dir());
    assert_eq!(md.mode() & 0o7777, 0o700, "the stage directory");
    assert_eq!(md.uid(), me());
    assert_eq!(listing(stage.path()), ["compose.yml", "db-roles.sh"]);
    let script = fs::symlink_metadata(stage.path().join("db-roles.sh")).unwrap();
    assert!(script.file_type().is_file());
    assert_eq!(script.mode() & 0o7777, 0o500, "the script");
    assert_eq!(script.uid(), me());
    let compose = fs::symlink_metadata(stage.path().join("compose.yml")).unwrap();
    assert!(compose.file_type().is_file());
    assert_eq!(compose.mode() & 0o7777, 0o400, "compose.yml");
    assert_eq!(compose.uid(), me());
    assert_eq!(fs::read(stage.path().join("db-roles.sh")).unwrap(), SCRIPT);
    assert_eq!(fs::read(stage.path().join("compose.yml")).unwrap(), COMPOSE);
}

#[test]
fn ss_02_dropping_the_stage_removes_it_and_nothing_else() {
    let t = Tmp::new("stage-02");
    let home = home_of(&t);
    fs::write(home.join("keep-me"), b"1").unwrap();
    let stage = build_stage(&home, "1.2.3", &bytes()).unwrap();
    let path = stage.path().to_path_buf();
    assert!(path.exists());
    drop(stage);
    assert!(!path.exists(), "the stage is gone");
    assert_eq!(listing(&home), ["keep-me"], "and only it");
    // a drop is one best-effort attempt and never panics: the directory already gone ...
    let stage = build_stage(&home, "1.2.3", &bytes()).unwrap();
    fs::remove_dir_all(stage.path()).unwrap();
    drop(stage);
    // ... and a removal the file system refuses (a home that cannot be written): the stage stays, nothing panics
    if me() != 0 {
        let stage = build_stage(&home, "1.2.4", &bytes()).unwrap();
        let path = stage.path().to_path_buf();
        fs::set_permissions(&home, fs::Permissions::from_mode(0o500)).unwrap();
        drop(stage);
        fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).unwrap();
        assert!(path.exists(), "the refused removal left the stage");
        fs::remove_dir_all(&path).unwrap();
    }
}

#[test]
fn ss_03_a_stale_stage_is_replaced_only_under_the_installs_conditions() {
    // (a) a real directory of mine with mode 0700, with a file and a nested directory in it: replaced whole
    let t = Tmp::new("stage-03a");
    let home = home_of(&t);
    let stale = home.join(".roles-stage-1.2.3");
    fs::create_dir(&stale).unwrap();
    fs::set_permissions(&stale, fs::Permissions::from_mode(0o700)).unwrap();
    fs::write(stale.join("old-script"), b"old").unwrap();
    fs::create_dir(stale.join("nested")).unwrap();
    fs::write(stale.join("nested").join("x"), b"x").unwrap();
    let stage = build_stage(&home, "1.2.3", &bytes()).expect("a stale stage of mine is replaced");
    assert_eq!(
        listing(stage.path()),
        ["compose.yml", "db-roles.sh"],
        "nothing of the stale one is left"
    );
    drop(stage);
    // (b) a link at the name: refused, the link and its target untouched
    let t = Tmp::new("stage-03b");
    let home = home_of(&t);
    let target = t.path().join("elsewhere");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("precious"), b"P").unwrap();
    symlink(&target, home.join(".roles-stage-1.2.3")).unwrap();
    let m = refusal(build_stage(&home, "1.2.3", &bytes()));
    assert!(
        m.contains(".roles-stage-1.2.3"),
        "the line names the path: {m}"
    );
    assert!(
        m.contains("is not a real directory of yours with mode 0700"),
        "a link is refused for what it is: {m}"
    );
    assert!(
        fs::symlink_metadata(home.join(".roles-stage-1.2.3"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(fs::read(target.join("precious")).unwrap(), b"P");
    // (c) a regular file at the name: refused, untouched
    let t = Tmp::new("stage-03c");
    let home = home_of(&t);
    fs::write(home.join(".roles-stage-1.2.3"), b"not a directory").unwrap();
    // with the mode a stage directory would have, so that only "it is not a directory" can refuse it
    fs::set_permissions(
        home.join(".roles-stage-1.2.3"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    let m = refusal(build_stage(&home, "1.2.3", &bytes()));
    assert!(
        m.contains("is not a real directory of yours with mode 0700"),
        "a regular file is refused for what it is, not for a failed removal: {m}"
    );
    assert_eq!(
        fs::read(home.join(".roles-stage-1.2.3")).unwrap(),
        b"not a directory"
    );
    // (d) a directory with another mode: refused, untouched
    let t = Tmp::new("stage-03d");
    let home = home_of(&t);
    let other = home.join(".roles-stage-1.2.3");
    fs::create_dir(&other).unwrap();
    fs::set_permissions(&other, fs::Permissions::from_mode(0o755)).unwrap();
    fs::write(other.join("mine"), b"m").unwrap();
    let m = refusal(build_stage(&home, "1.2.3", &bytes()));
    assert!(
        m.contains("is not a real directory of yours with mode 0700"),
        "{m}"
    );
    assert_eq!(
        fs::read(other.join("mine")).unwrap(),
        b"m",
        "a directory that is not 0700 is not removed"
    );
}

#[test]
fn ss_04_a_home_that_cannot_be_written_or_does_not_exist_gives_a_refusal_and_no_stage() {
    let t = Tmp::new("stage-04");
    refusal(build_stage(
        &t.path().join("no-such-home"),
        "1.2.3",
        &bytes(),
    ));
    if me() == 0 {
        return; // root can write into a read-only directory
    }
    let home = home_of(&t);
    fs::set_permissions(&home, fs::Permissions::from_mode(0o500)).unwrap();
    refusal(build_stage(&home, "1.2.3", &bytes()));
    // a home that cannot be searched: the path of the stage cannot be looked at, and the line says so
    fs::set_permissions(&home, fs::Permissions::from_mode(0o600)).unwrap();
    let m = refusal(build_stage(&home, "1.2.3", &bytes()));
    fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(m.contains("cannot be looked at"), "{m}");
    assert!(listing(&home).is_empty(), "nothing was left behind");
}

#[test]
fn ss_05_an_ancestor_writable_by_group_or_other_is_refused_naming_it_and_the_fix() {
    for (mode, what) in [
        (0o775, "group"),
        (0o757, "other"),
        (0o777, "group and other"),
    ] {
        let t = Tmp::new("stage-05");
        let parent = t.path().join("parent");
        fs::create_dir(&parent).unwrap();
        let home = parent.join("home");
        fs::create_dir(&home).unwrap();
        fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&parent, fs::Permissions::from_mode(mode)).unwrap();
        let canonical = fs::canonicalize(&parent).unwrap();
        let m = refusal(check_ancestors(&home));
        assert!(
            m.contains(canonical.to_str().unwrap()),
            "{what}: the line names the directory: {m}"
        );
        assert!(m.contains("chmod"), "{what}: and says the fix: {m}");
        let m2 = refusal(build_stage(&home, "1.2.3", &bytes()));
        assert_eq!(m2, m, "{what}: build_stage refuses with the same line");
        fs::set_permissions(&parent, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(listing(&home).is_empty(), "{what}: nothing was built");
    }
}

#[test]
fn ss_06_a_release_name_that_is_not_a_version_is_refused() {
    let t = Tmp::new("stage-06");
    let home = home_of(&t);
    for bad in [
        "",
        ".",
        "..",
        "../x",
        "a/b",
        "1.2.3 ",
        "1.2.3\n",
        "a\0b",
        ".hidden",
        "x y",
        "é",
        &"1".repeat(65),
    ] {
        let m = refusal(build_stage(&home, bad, &bytes()));
        assert!(
            m.contains("the release name is not a version"),
            "{bad:?}: refused for the name itself, before anything is made: {m}"
        );
    }
    assert!(
        listing(&home).is_empty(),
        "nothing was built for any of them"
    );
    for good in ["1.2.3", "0.0.1", "1.2.3-rc.1+build5", "10.20.30"] {
        let s = build_stage(&home, good, &bytes()).unwrap_or_else(|e| panic!("{good}: {e}"));
        assert!(s.path().ends_with(format!(".roles-stage-{good}")));
    }
}

#[test]
fn ss_07_two_releases_have_two_stages_and_a_stage_can_be_built_again_after_its_drop() {
    let t = Tmp::new("stage-07");
    let home = home_of(&t);
    let a = build_stage(&home, "1.2.3", &bytes()).unwrap();
    let b = build_stage(&home, "1.2.4", &bytes()).unwrap();
    assert!(a.path().exists() && b.path().exists());
    let a_path = a.path().to_path_buf();
    drop(a);
    assert!(
        !a_path.exists() && b.path().exists(),
        "dropping one leaves the other"
    );
    let again = build_stage(&home, "1.2.3", &bytes()).expect("built again");
    assert_eq!(fs::read(again.path().join("db-roles.sh")).unwrap(), SCRIPT);
}

#[test]
fn ss_08_the_ancestor_check_and_the_stage_work_on_the_canonical_path() {
    let t = Tmp::new("stage-08");
    let real = home_of(&t);
    let link = t.path().join("link-to-home");
    symlink(&real, &link).unwrap();
    let canonical = fs::canonicalize(&real).unwrap();
    assert_eq!(
        check_ancestors(&link),
        Ok(canonical.clone()),
        "a link is resolved first"
    );
    let stage = build_stage(&link, "1.2.3", &bytes()).expect("a home reached through a link");
    assert_eq!(
        stage.path(),
        canonical.join(".roles-stage-1.2.3"),
        "the stage is under the canonical home"
    );
    // a path that does not exist is refused (nothing to canonicalise)
    refusal(check_ancestors(&t.path().join("nope")));
}

// ---- through the REAL adapter (W1-66 (4): a fake port proves nothing here): the staged script is what runs; the adapter's own project check
// still applies through the stage. These cells exercise code that already exists, so they were not RED first; their kills are the mutants.

/// A roles script that records WHO it is in `marker` (an absolute path chosen by the cell) and succeeds.
fn marking_script(marker: &Path, who: &str) -> Vec<u8> {
    format!("#!/bin/sh\necho {who} >> '{}'\nexit 0\n", marker.display()).into_bytes()
}

fn adapter(home: &Path) -> RealContainers {
    // `run_roles` runs the script by its path: the program name of podman is never started by these cells
    RealContainers::with(
        "/nonexistent/podman",
        Deadlines::default(),
        ComposeEnv::new(home.to_path_buf(), 8601),
    )
}

fn modules() -> Vec<ModuleId> {
    vec![ModuleId::parse("audit").unwrap()]
}

#[test]
fn rr_01_the_adapter_runs_the_staged_script_and_a_replacement_of_the_original_after_the_check_changes_nothing()
 {
    let t = Tmp::new("stage-rr01");
    let marker = t.path().join("marker.txt");
    let verified_script = marking_script(&marker, "VERIFIED");
    let dir = release(t.path(), &verified_script, COMPOSE);
    let verified = verify_release_dir_bytes(&dir).expect("the release verifies");
    let home = home_of(&t);
    let stage = build_stage(&home, "1.2.3", &verified).unwrap();
    // AFTER the check and the staging, the original is replaced by a script that records another name
    put(
        &dir.join("db-roles.sh"),
        &marking_script(&marker, "EVIL"),
        0o755,
    );
    let r = adapter(&home).run_roles(stage.path(), &modules(), Duration::from_secs(30));
    assert_eq!(r, Ok(()), "the staged script ran and succeeded");
    assert_eq!(
        fs::read_to_string(&marker).unwrap(),
        "VERIFIED\n",
        "only the verified bytes ran: the replaced original never did"
    );
    // a second attempt (the confirming re-run) runs the same staged copy
    let r = adapter(&home).run_roles(stage.path(), &modules(), Duration::from_secs(30));
    assert_eq!(r, Ok(()));
    assert_eq!(fs::read_to_string(&marker).unwrap(), "VERIFIED\nVERIFIED\n");
}

#[test]
fn rr_02_the_adapters_project_check_applies_through_the_stage_and_the_script_never_starts() {
    let t = Tmp::new("stage-rr02");
    let marker = t.path().join("marker.txt");
    let script = marking_script(&marker, "RAN");
    let dir = release(
        t.path(),
        &script,
        b"name: some-other-project\nservices: {}\n",
    );
    let verified = verify_release_dir_bytes(&dir)
        .expect("the release itself verifies (it is the adapter that refuses the project)");
    let home = home_of(&t);
    let stage = build_stage(&home, "1.2.3", &verified).unwrap();
    let r = adapter(&home).run_roles(stage.path(), &modules(), Duration::from_secs(30));
    assert!(matches!(r, Err(PortError::Failed(_))), "{r:?}");
    assert!(!marker.exists(), "the script never started");
}
