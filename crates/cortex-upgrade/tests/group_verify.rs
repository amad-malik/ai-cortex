//! Resumed release verification: real unpacker positive control and hostile local trees.
mod common;

use common::{Tmp, tar};
use cortex_upgrade::unpack::{UnpackLimits, unpack_release_gz};
use cortex_upgrade::verify::verify_release_dir;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Cursor;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::{Path, PathBuf};

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

fn fixture() -> (Tmp, PathBuf) {
    let t = Tmp::new("verify");
    let dir = t.path().join("release");
    let mut entries = vec![
        tar::file("cortex", b"cli", 0o755),
        tar::file("compose.yml", b"compose", 0o644),
        tar::file("stack.env", b"env", 0o644),
        tar::file("db-roles.sh", b"roles", 0o755),
        tar::dir("initdb/"),
        tar::file("initdb/roles.sql", b"sql", 0o644),
        tar::dir("bin/"),
        tar::dir("systemd/"),
    ];
    for name in NAMES.iter().filter(|name| {
        name.starts_with("bin/") || name.starts_with("systemd/") || **name == "cortex.toml.example"
    }) {
        entries.push(tar::file(
            name,
            name.as_bytes(),
            if name.starts_with("bin/") {
                0o755
            } else {
                0o644
            },
        ));
    }
    let archive = tar::gz(&tar::finish(&entries));
    unpack_release_gz(Cursor::new(archive), &dir, &UnpackLimits::default()).unwrap();
    let image = format!("localhost/cortex-server@{}", common::DIGEST_A);
    let database = format!("example.invalid/db@{}", common::DIGEST_A);
    fs::write(
        dir.join("stack.env"),
        format!("CORTEX_VERSION=1.2.3\nCORTEX_SERVER_IMAGE={image}\nCORTEX_DB_IMAGE={database}\n"),
    )
    .unwrap();
    let roles_digest = format!("sha256:{:x}", Sha256::digest(b"roles"));
    let manifest = serde_json::json!({
        "schema": 1,
        "releases": [{
            "version": "1.2.3", "date": "2026-11-07T00:00:00Z", "notes_url": "https://example.invalid/",
            "assets": [{"name": "image.oci.tar", "url": "image.oci.tar", "digest": common::DIGEST_A}],
            "cortex": {"label": "1.2.3", "members": 2, "server_image": {"ref": image, "asset": "image.oci.tar"},
                "db_image": database, "roles_script_digest": roles_digest, "sidecar_images": [],
                "migrations": [], "modules": ["audit"], "migration_class": "expand"}
        }]
    });
    fs::write(
        dir.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    seal(&dir);
    (t, dir)
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
    fs::write(dir.join("SHA256SUMS"), lines).unwrap();
}

#[test]
fn member_list_version_is_read_and_unknown_versions_are_refused() {
    let (_t, dir) = fixture();
    let path = dir.join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["releases"][0]["cortex"]["members"] = serde_json::json!(2);
    fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let parsed = cortex_upgrade::manifest::parse(&fs::read(&path).unwrap()).unwrap();
    let cortex_upgrade::manifest::Entry::Usable(release) = &parsed.entries[0] else {
        panic!("release must be usable");
    };
    assert_eq!(release.cortex.members, 2);
    seal(&dir);
    verify_release_dir(&dir).unwrap();
    for value in [
        serde_json::json!(3),
        serde_json::json!(0),
        serde_json::json!("2"),
        serde_json::Value::Null,
    ] {
        manifest["releases"][0]["cortex"]["members"] = value;
        fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        seal(&dir);
        refuses_unchanged(&dir);
    }
    manifest["releases"][0]["cortex"]
        .as_object_mut()
        .unwrap()
        .remove("members");
    fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    seal(&dir);
    refuses_unchanged(&dir);
}

fn refuses_unchanged(dir: &Path) {
    let before = common::tree_hash(dir);
    let err = verify_release_dir(dir).expect_err("hostile release must fail");
    assert!(
        !err.to_string().contains(&dir.display().to_string()),
        "error disclosed an absolute path"
    );
    assert_eq!(
        before,
        common::tree_hash(dir),
        "verification must not repair the tree"
    );
}

#[test]
fn accepts_real_unpacked_release_and_binary_checksum_format() {
    let (_t, dir) = fixture();
    verify_release_dir(&dir).unwrap();
    let sums = fs::read_to_string(dir.join("SHA256SUMS"))
        .unwrap()
        .replacen("  ", " *", 1);
    fs::write(dir.join("SHA256SUMS"), sums).unwrap();
    verify_release_dir(&dir).unwrap();
}

#[test]
fn rejects_changed_or_missing_files() {
    for name in NAMES {
        let (_t, dir) = fixture();
        fs::write(dir.join(name), b"tampered").unwrap();
        refuses_unchanged(&dir);
        let (_t, dir) = fixture();
        fs::remove_file(dir.join(name)).unwrap();
        refuses_unchanged(&dir);
    }
}

#[test]
fn rejects_extra_inventory_and_directories() {
    for name in [
        "extra",
        "initdb/extra",
        "other/extra",
        "bin/extra",
        "systemd/extra",
    ] {
        let (_t, dir) = fixture();
        if name.starts_with("other/") {
            fs::create_dir(dir.join("other")).unwrap();
        }
        fs::write(dir.join(name), b"extra").unwrap();
        refuses_unchanged(&dir);
    }
}

#[test]
fn rejects_extra_bin_and_systemd_names_even_when_checksummed() {
    for name in ["bin/extra", "systemd/extra"] {
        let (_t, dir) = fixture();
        fs::write(dir.join(name), b"extra").unwrap();
        if name.starts_with("bin/") {
            fs::set_permissions(dir.join(name), fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path = dir.join("SHA256SUMS");
        let mut sums = fs::read_to_string(&path).unwrap();
        sums.push_str(&format!("{:x}  {name}\n", Sha256::digest(b"extra")));
        fs::write(path, sums).unwrap();
        refuses_unchanged(&dir);
    }
}

#[test]
fn rejects_links_and_special_files() {
    for target in ["compose.yml", "initdb", "missing"] {
        let (_t, dir) = fixture();
        symlink(target, dir.join("link")).unwrap();
        refuses_unchanged(&dir);
    }
    let (_t, dir) = fixture();
    fs::remove_file(dir.join("compose.yml")).unwrap();
    symlink("stack.env", dir.join("compose.yml")).unwrap();
    refuses_unchanged(&dir);
    let (_t, dir) = fixture();
    assert!(
        std::process::Command::new("mkfifo")
            .arg(dir.join("fifo"))
            .status()
            .unwrap()
            .success()
    );
    assert!(verify_release_dir(&dir).is_err());
    assert!(fs::symlink_metadata(dir.join("fifo")).is_ok());
}

#[test]
fn rejects_bad_checksum_grammar_and_bounds() {
    let hash = "a".repeat(64);
    let long = "x".repeat(65);
    for line in [
        format!("{}  manifest.json\n", "a".repeat(63)),
        format!("{}  manifest.json\n", "A".repeat(64)),
        format!("{hash} manifest.json\n"),
        format!("{hash}\t manifest.json\n"),
        format!("{hash}  ../manifest.json\n"),
        format!("{hash}  /manifest.json\n"),
        format!("{hash}  .hidden\n"),
        format!("{hash}  {long}\n"),
        format!("{hash}  initdb//x\n"),
        format!("{hash}  initdb/a/b\n"),
        format!("{hash}  manifest.json"),
        format!("{hash}  manifest.json\r\n"),
        format!("{hash}  manifest.json\n{hash}  manifest.json\n"),
        format!("{hash}  SHA256SUMS\n"),
        (0..1001).map(|i| format!("{hash}  file{i}\n")).collect(),
        "x".repeat(2 * 1024 * 1024),
    ] {
        let (_t, dir) = fixture();
        fs::write(dir.join("SHA256SUMS"), line).unwrap();
        refuses_unchanged(&dir);
    }
    let (_t, dir) = fixture();
    let path = dir.join("SHA256SUMS");
    let valid = fs::read_to_string(&path).unwrap();
    fs::write(&path, valid.replace('\n', "\r\n")).unwrap();
    refuses_unchanged(&dir);
    let (_t, dir) = fixture();
    let path = dir.join("SHA256SUMS");
    fs::remove_file(&path).unwrap();
    symlink("manifest.json", &path).unwrap();
    refuses_unchanged(&dir);
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    refuses_unchanged(&dir);
}

#[test]
fn rejects_invalid_names_and_duplicate_in_otherwise_valid_inventory() {
    for bad in [
        ".hidden".to_owned(),
        "x".repeat(65),
        "initdb/.hidden".to_owned(),
        format!("initdb/{}", "x".repeat(65)),
    ] {
        let (_t, dir) = fixture();
        fs::write(dir.join(&bad), b"present").unwrap();
        let path = dir.join("SHA256SUMS");
        let mut sums = fs::read_to_string(&path).unwrap();
        sums.push_str(&format!("{:x}  {bad}\n", Sha256::digest(b"present")));
        fs::write(path, sums).unwrap();
        refuses_unchanged(&dir);
    }
    let (_t, dir) = fixture();
    let path = dir.join("SHA256SUMS");
    let sums = fs::read_to_string(&path).unwrap();
    fs::write(&path, format!("{sums}{}\n", sums.lines().next().unwrap())).unwrap();
    refuses_unchanged(&dir);
}

#[test]
fn rejects_unlisted_manifest_even_if_it_is_missing_from_disk() {
    let (_t, dir) = fixture();
    fs::remove_file(dir.join("manifest.json")).unwrap();
    let path = dir.join("SHA256SUMS");
    let sums = fs::read_to_string(&path).unwrap();
    let sums: String = sums
        .lines()
        .filter(|line| !line.ends_with("  manifest.json"))
        .map(|line| format!("{line}\n"))
        .collect();
    fs::write(path, sums).unwrap();
    refuses_unchanged(&dir);
}

#[test]
fn rejects_each_wrong_mode() {
    for name in NAMES
        .iter()
        .copied()
        .chain(["SHA256SUMS", "initdb", "bin", "systemd"])
    {
        let (_t, dir) = fixture();
        let mode = if ["initdb", "bin", "systemd"].contains(&name) {
            0o700
        } else if name == "cortex" || name == "db-roles.sh" || name.starts_with("bin/") {
            0o644
        } else {
            0o755
        };
        fs::set_permissions(dir.join(name), fs::Permissions::from_mode(mode)).unwrap();
        refuses_unchanged(&dir);
    }
}

#[test]
fn rejects_each_missing_expanded_member_even_when_unlisted() {
    for name in NAMES.iter().copied().filter(|n| {
        *n == "cortex.toml.example" || n.starts_with("bin/") || n.starts_with("systemd/")
    }) {
        let (_t, dir) = fixture();
        fs::remove_file(dir.join(name)).unwrap();
        let sums = fs::read_to_string(dir.join("SHA256SUMS")).unwrap();
        let kept: String = sums
            .lines()
            .filter(|line| !line.ends_with(&format!("  {name}")))
            .map(|line| format!("{line}\n"))
            .collect();
        fs::write(dir.join("SHA256SUMS"), kept).unwrap();
        refuses_unchanged(&dir);
    }
    for name in ["bin", "systemd"] {
        let (_t, dir) = fixture();
        fs::remove_dir_all(dir.join(name)).unwrap();
        let sums = fs::read_to_string(dir.join("SHA256SUMS")).unwrap();
        let kept: String = sums
            .lines()
            .filter(|line| !line.contains(&format!("  {name}/")))
            .map(|line| format!("{line}\n"))
            .collect();
        fs::write(dir.join("SHA256SUMS"), kept).unwrap();
        refuses_unchanged(&dir);
    }
}

#[test]
fn rejects_symlink_as_release_directory() {
    let (t, dir) = fixture();
    let link = t.path().join("linked-release");
    symlink(&dir, &link).unwrap();
    let err = verify_release_dir(&link).unwrap_err().to_string();
    assert!(!err.contains(&link.display().to_string()));
}

#[test]
fn rejects_noncanonical_lines_in_an_otherwise_valid_checksum_file() {
    for transform in ["uppercase", "single_space", "tab", "no_newline", "crlf"] {
        let (_t, dir) = fixture();
        let path = dir.join("SHA256SUMS");
        let good = fs::read_to_string(&path).unwrap();
        let bad = match transform {
            "uppercase" => format!("{}{}", good[..64].to_ascii_uppercase(), &good[64..]),
            "single_space" => good.replacen("  ", " ", 1),
            "tab" => good.replacen("  ", "\t ", 1),
            "no_newline" => good.trim_end_matches('\n').to_owned(),
            "crlf" => good.replace('\n', "\r\n"),
            _ => unreachable!(),
        };
        fs::write(path, bad).unwrap();
        refuses_unchanged(&dir);
    }
}

#[test]
fn checksum_line_limit_accepts_1000_and_rejects_1001() {
    let (_t, dir) = fixture();
    let path = dir.join("SHA256SUMS");
    let mut sums = fs::read_to_string(&path).unwrap();
    let digest = format!("{:x}", Sha256::digest(b""));
    for i in 0..(1000 - NAMES.len()) {
        let name = format!("initdb/extra{i:04}");
        fs::write(dir.join(&name), b"").unwrap();
        sums.push_str(&format!("{digest}  {name}\n"));
    }
    fs::write(&path, &sums).unwrap();
    verify_release_dir(&dir).unwrap();
    fs::write(dir.join("initdb/overflow"), b"").unwrap();
    sums.push_str(&format!("{digest}  initdb/overflow\n"));
    fs::write(path, sums).unwrap();
    refuses_unchanged(&dir);
}

#[test]
fn rejects_release_without_the_initdb_directory() {
    let (_t, dir) = fixture();
    fs::remove_file(dir.join("initdb/roles.sql")).unwrap();
    fs::remove_dir(dir.join("initdb")).unwrap();
    let path = dir.join("SHA256SUMS");
    let good = fs::read_to_string(&path).unwrap();
    let sums: String = good
        .lines()
        .filter(|line| !line.ends_with("  initdb/roles.sql"))
        .map(|line| format!("{line}\n"))
        .collect();
    fs::write(path, sums).unwrap();
    refuses_unchanged(&dir);
}

#[test]
fn roles_script_must_match_manifest_even_when_checksums_are_rewritten() {
    let (_t, dir) = fixture();
    verify_release_dir(&dir).unwrap();
    fs::write(dir.join("db-roles.sh"), b"different privileged script").unwrap();
    seal(&dir);
    refuses_unchanged(&dir);
}

#[test]
fn rejects_resealed_malformed_manifest() {
    let (_t, dir) = fixture();
    fs::write(dir.join("manifest.json"), b"{}").unwrap();
    seal(&dir);
    refuses_unchanged(&dir);
}

#[test]
fn rejects_resealed_unmatched_version() {
    let (_t, dir) = fixture();
    let env_path = dir.join("stack.env");
    let env = fs::read_to_string(&env_path)
        .unwrap()
        .replace("CORTEX_VERSION=1.2.3", "CORTEX_VERSION=9.9.9");
    fs::write(env_path, env).unwrap();
    seal(&dir);
    refuses_unchanged(&dir);
}

#[test]
fn manifest_binding_selects_the_stack_version_not_the_first_entry() {
    let (_t, dir) = fixture();
    let path = dir.join("manifest.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let mut other = manifest["releases"][0].clone();
    other["version"] = "0.1.0".into();
    other["cortex"]["roles_script_digest"] = common::DIGEST_A.into();
    manifest["releases"]
        .as_array_mut()
        .unwrap()
        .insert(0, other);
    fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    seal(&dir);
    verify_release_dir(&dir).unwrap();
}

#[test]
fn rejects_roles_script_removed_from_both_release_and_checksums() {
    let (_t, dir) = fixture();
    fs::remove_file(dir.join("db-roles.sh")).unwrap();
    let path = dir.join("SHA256SUMS");
    let sums = fs::read_to_string(&path).unwrap();
    let sums: String = sums
        .lines()
        .filter(|line| !line.ends_with("  db-roles.sh"))
        .map(|line| format!("{line}\n"))
        .collect();
    fs::write(path, sums).unwrap();
    refuses_unchanged(&dir);
}

#[test]
fn rejects_manifest_present_but_omitted_from_checksums() {
    let (_t, dir) = fixture();
    let path = dir.join("SHA256SUMS");
    let sums = fs::read_to_string(&path).unwrap();
    let sums: String = sums
        .lines()
        .filter(|line| !line.ends_with("  manifest.json"))
        .map(|line| format!("{line}\n"))
        .collect();
    fs::write(path, sums).unwrap();
    refuses_unchanged(&dir);
}
