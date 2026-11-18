//! The builder output must be accepted by the actual release consumers.
mod common;

use common::Tmp;
use cortex_upgrade::manifest::{Entry, parse};
use cortex_upgrade::unpack::{UnpackError, UnpackLimits, unpack_release, unpack_release_gz};
use cortex_upgrade::verify::verify_release_dir;
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::Cursor;
use std::path::Path;
use std::process::Command;

fn collect_files(root: &Path, dir: &Path, names: &mut Vec<String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let kind = entry.file_type().unwrap();
        let path = entry.path();
        if kind.is_dir() {
            collect_files(root, &path, names);
        } else {
            assert!(kind.is_file(), "unexpected non-file in unpacked release");
            let name = path.strip_prefix(root).unwrap().to_str().unwrap();
            if name != "SHA256SUMS" {
                names.push(name.to_owned());
            }
        }
    }
}

#[test]
fn installer_metadata_is_forbidden_inside_the_release_archive() {
    let t = Tmp::new("release-sidecar-forbidden");
    let target = t.path().join("release");
    let archive =
        common::tar::finish(&[common::tar::file("install.meta", b"META_SCHEMA=1\n", 0o644)]);
    assert!(matches!(
        unpack_release(Cursor::new(archive), &target, &UnpackLimits::default()),
        Err(UnpackError::Forbidden { index: 0 })
    ));
    assert!(!target.exists());
}

#[test]
fn generated_release_passes_manifest_unpack_and_resume_verification() {
    let t = Tmp::new("release-integration");
    let dist = t.path().join("dist");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("python3")
        .arg("-I")
        .arg(root.join("scripts/release-fixtures/test_release.py"))
        .arg("--build-to")
        .arg(&dist)
        // the fixture builder reads TMPDIR and fails with a KeyError when it is unset (a test must not depend on the caller's environment): the cell's own directory
        .env("TMPDIR", t.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "fixture builder failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let verified_dist = Command::new("python3")
        .arg("-I")
        .arg(root.join("scripts/release_manifest.py"))
        .args(["verify-dist", "--output-dir"])
        .arg(&dist)
        .args(["--version", "1.2.3"])
        .output()
        .unwrap();
    assert!(
        verified_dist.status.success(),
        "distribution: {}",
        String::from_utf8_lossy(&verified_dist.stderr)
    );
    let manifest = fs::read(dist.join("manifest.json")).unwrap();
    let parsed = parse(&manifest).unwrap();
    assert!(matches!(parsed.entries.as_slice(), [Entry::Usable(_)]));
    let release = t.path().join("release");
    unpack_release_gz(
        File::open(dist.join("cortex-1.2.3-x86_64-linux.tar.gz")).unwrap(),
        &release,
        &UnpackLimits::default(),
    )
    .unwrap();
    let version = Command::new(release.join("cortex"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(version.status.success());
    let line = String::from_utf8(version.stdout).unwrap();
    assert_eq!(line.split_whitespace().nth(1), Some("1.2.3"), "{line}");
    fs::write(release.join("manifest.json"), manifest).unwrap();
    let mut names = Vec::new();
    collect_files(&release, &release, &mut names);
    names.sort_unstable();
    let sums: String = names
        .iter()
        .map(|name| {
            format!(
                "{:x}  {name}\n",
                Sha256::digest(fs::read(release.join(name)).unwrap())
            )
        })
        .collect();
    fs::write(release.join("SHA256SUMS"), sums).unwrap();
    verify_release_dir(&release).unwrap();
}
