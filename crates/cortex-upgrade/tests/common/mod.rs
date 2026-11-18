//! Shared fixtures for the layout tests: a temporary home under the target directory (never /tmp),
//! installed and staged releases, and a hash of a whole tree so that "nothing changed" is a diff.
#![allow(dead_code)]

pub mod http;
pub mod tar;

use cortex_types::{Clock, Timestamp};
use cortex_upgrade::version::Version;
use sha2::Digest as _;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A fresh directory under cargo's target tmp dir; removed by its exact path when dropped.
pub struct Tmp(pub PathBuf);

impl Tmp {
    pub fn new(tag: &str) -> Tmp {
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::SeqCst);
        let p = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join(format!("{tag}-{}-{n}", std::process::id()));
        fs::create_dir_all(&p).unwrap();
        Tmp(p)
    }
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Tmp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub struct FixedClock(pub i64);

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        Timestamp::from_unix_micros(self.0 * 1_000_000)
    }
}

pub fn v(s: &str) -> Version {
    s.parse().unwrap()
}

pub const DIGEST_A: &str =
    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

/// The files of one release directory (design §2), with a manifest copy that lists `version` with
/// the given migration class.
pub fn write_release_files(dir: &Path, version: &str, class: &str) {
    fs::create_dir_all(dir.join("initdb")).unwrap();
    fs::write(dir.join("cortex"), format!("#!/bin/sh\necho {version}\n")).unwrap();
    fs::write(
        dir.join("compose.yml"),
        format!("# compose of {version}\nservices: {{}}\n"),
    )
    .unwrap();
    fs::write(
        dir.join("stack.env"),
        format!(
            "CORTEX_VERSION={version}\nCORTEX_SERVER_IMAGE=localhost/cortex-server@{}\nCORTEX_DB_IMAGE=example.invalid/db:1@{}\n",
            DIGEST_A, DIGEST_A
        ),
    )
    .unwrap();
    let manifest = serde_json::json!({
        "schema": 1,
        "releases": [{
            "version": version, "date": "2026-11-12T00:00:00Z", "notes_url": "https://example.invalid/n",
            "assets": [{ "name": "a.tar.gz", "url": "a.tar.gz", "digest": DIGEST_A }],
            "cortex": {
                "label": "x",
                "members": 2,
                "server_image": { "ref": format!("localhost/cortex-server@{DIGEST_A}"), "asset": "a.tar.gz" },
                "db_image": format!("example.invalid/db:1@{DIGEST_A}"),
                "roles_script_digest": DIGEST_A,
                "sidecar_images": [], "migrations": ["platform/0001_platform"],
                "modules": ["audit"], "migration_class": class
            }
        }]
    });
    fs::write(
        dir.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    fs::write(dir.join("SHA256SUMS"), "synthetic\n").unwrap();
}

/// A home the way `install.sh` lays it out, with `version` installed and current.
pub fn install(home: &Path, version: &str) {
    fs::create_dir_all(home.join("releases")).unwrap();
    fs::create_dir_all(home.join("bin")).unwrap();
    fs::create_dir_all(home.join("dumps/set-1")).unwrap();
    fs::create_dir_all(home.join("upgrades")).unwrap();
    write_release_files(&home.join("releases").join(version), version, "expand");
    symlink(format!("releases/{version}"), home.join("current")).unwrap();
    symlink("../current/cortex", home.join("bin/cortex")).unwrap();
    fs::write(
        home.join("cortex.toml"),
        "# operator config, synthetic\n[server]\n",
    )
    .unwrap();
    fs::write(home.join("operator.token"), "SYNTHETIC-TOKEN-1\n").unwrap();
    // the dump tool install.sh puts in <home>/bin (cox 14:30Z, Q-8)
    fs::write(home.join("bin/cortex-dump"), "#!/bin/sh\nexit 0\n").unwrap();
    std::os::unix::fs::PermissionsExt::set_mode(
        &mut fs::metadata(home.join("bin/cortex-dump"))
            .unwrap()
            .permissions(),
        0o755,
    );
    fs::set_permissions(
        home.join("bin/cortex-dump"),
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .unwrap();
    fs::write(home.join("dumps/set-1/dump.sql"), "-- synthetic dump\n").unwrap();
    fs::write(home.join("dumps/set-1/SHA256SUMS"), "synthetic\n").unwrap();
}

/// A staged release `releases/<version>.part/`, ready for `commit`.
pub fn stage(home: &Path, version: &str, class: &str) {
    write_release_files(
        &home.join("releases").join(format!("{version}.part")),
        version,
        class,
    );
}

/// One hash over a tree: every path, kind, mode, file content and link target, in name order.
pub fn tree_hash(path: &Path) -> String {
    fn walk(p: &Path, rel: &str, h: &mut sha2::Sha256) {
        let md = fs::symlink_metadata(p).unwrap();
        use std::os::unix::fs::PermissionsExt;
        h.update(format!("{rel}|{:o}|", md.permissions().mode()).as_bytes());
        if md.file_type().is_symlink() {
            h.update(format!("L:{}\n", fs::read_link(p).unwrap().display()).as_bytes());
        } else if md.is_dir() {
            h.update(b"D\n");
            let mut names: Vec<_> = fs::read_dir(p)
                .unwrap()
                .map(|e| e.unwrap().file_name())
                .collect();
            names.sort();
            for n in names {
                walk(&p.join(&n), &format!("{rel}/{}", n.to_string_lossy()), h);
            }
        } else {
            h.update(b"F:");
            h.update(fs::read(p).unwrap());
            h.update(b"\n");
        }
    }
    let mut h = sha2::Sha256::new();
    walk(path, "", &mut h);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// A link that resolves to a directory holding a `compose.yml` (the invariant of UP-16).
pub fn resolves_to_a_release(link: &Path) -> bool {
    fs::metadata(link).map(|m| m.is_dir()).unwrap_or(false) && link.join("compose.yml").is_file()
}

/// Like `rerun_self`, with the child under the given umask (three octal digits), set by `sh` so that no `unsafe` is
/// needed and the umask of the test process itself is never touched.
pub fn rerun_self_umask(test: &str, marker: (&str, &str), umask: &str) -> std::process::Output {
    assert!(umask.len() == 3 && umask.bytes().all(|b| (b'0'..=b'7').contains(&b)));
    std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("umask {umask} && exec \"$0\" \"$@\""))
        .arg(std::env::current_exe().unwrap())
        .args(["--exact", test, "--nocapture", "--test-threads=1"])
        .env(marker.0, marker.1)
        .output()
        .unwrap()
}

/// Run ONE test of the calling test binary again as a child process with extra environment variables (the crate forbids
/// `unsafe`, so a test cannot set variables in its own process). The child test must do nothing unless `marker` is set.
pub fn rerun_self(test: &str, marker: (&str, &str), env: &[(&str, &str)]) -> std::process::Output {
    let mut cmd = std::process::Command::new(std::env::current_exe().unwrap());
    cmd.args(["--exact", test, "--nocapture", "--test-threads=1"])
        .env(marker.0, marker.1)
        .env_remove("NO_PROXY")
        .env_remove("no_proxy");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().unwrap()
}
