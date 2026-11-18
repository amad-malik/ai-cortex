//! Piece 14, kai's conditions for the new crates (W1-24): the external crates of `cortex-upgrade` are pinned with `=`,
//! their features are exactly the approved ones, the lock holds exactly those versions with a checksum, and none of the
//! banned TLS stacks is in the lock. This is the check that FAILS THE BUILD if a crate differs from the pinned one.

use std::collections::BTreeMap;

/// (name, exact version, default-features must be false, the only features)
const PINS: &[(&str, &str, bool, &[&str])] = &[
    ("semver", "1.0.28", false, &[]),
    ("ureq", "3.5.1", true, &["rustls"]),
    ("flate2", "1.1.11", true, &["rust_backend"]),
];
const BANNED: &[&str] = &[
    "aws-lc-rs",
    "aws-lc-sys",
    "native-tls",
    "openssl-sys",
    "openssl",
];

fn check_manifest(cargo_toml: &str) -> Result<(), String> {
    let doc: toml::Table = cargo_toml.parse().map_err(|e| format!("Cargo.toml: {e}"))?;
    let deps = doc
        .get("dependencies")
        .and_then(|d| d.as_table())
        .ok_or("no [dependencies]")?;
    for (name, version, no_default, features) in PINS {
        let dep = deps.get(*name).ok_or(format!("{name} is missing"))?;
        let (v, df, fs) = match dep {
            toml::Value::String(v) => (v.clone(), None, vec![]),
            toml::Value::Table(t) => (
                t.get("version")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                t.get("default-features").and_then(|v| v.as_bool()),
                t.get("features")
                    .and_then(|f| f.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(str::to_string))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default(),
            ),
            _ => return Err(format!("{name}: unexpected form")),
        };
        if v != format!("={version}") {
            return Err(format!(
                "{name}: version must be exactly `={version}`, got `{v}`"
            ));
        }
        if *no_default && df != Some(false) {
            return Err(format!("{name}: default-features must be false"));
        }
        let mut fs = fs;
        fs.sort();
        if fs != *features {
            return Err(format!(
                "{name}: features must be exactly {features:?}, got {fs:?}"
            ));
        }
    }
    if deps.contains_key("tar") {
        return Err("tar is not approved for use: the unpack is our own reader".into());
    }
    Ok(())
}

fn check_lock(lock: &str) -> Result<(), String> {
    let doc: toml::Table = lock.parse().map_err(|e| format!("Cargo.lock: {e}"))?;
    let packages = doc
        .get("package")
        .and_then(|p| p.as_array())
        .ok_or("no [[package]]")?;
    let mut by_name: BTreeMap<String, Vec<(String, bool)>> = BTreeMap::new();
    for p in packages {
        let t = p.as_table().ok_or("bad package")?;
        let name = t
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let version = t
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        by_name
            .entry(name)
            .or_default()
            .push((version, t.contains_key("checksum")));
    }
    for (name, version, _, _) in PINS {
        match by_name.get(*name).map(Vec::as_slice) {
            Some([(v, true)]) if v == version => {}
            other => {
                return Err(format!(
                    "{name}: the lock must hold exactly one `{version}` with a checksum, has {other:?}"
                ));
            }
        }
    }
    for b in BANNED {
        if by_name.contains_key(*b) {
            return Err(format!(
                "{b} is in the lock: forbidden (rustls with ring only)"
            ));
        }
    }
    Ok(())
}

#[test]
fn pins_01_the_real_manifest_and_lock_hold_the_approved_pins() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
    let lock = std::fs::read_to_string(root.join("../../Cargo.lock")).unwrap();
    check_manifest(&manifest).unwrap();
    check_lock(&lock).unwrap();
}

#[test]
fn pins_02_the_checks_themselves_fail_on_every_kind_of_drift() {
    let manifest = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"),
    )
    .unwrap();
    let lock = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock"),
    )
    .unwrap();
    assert!(check_manifest(&manifest).is_ok(), "control");
    assert!(check_lock(&lock).is_ok(), "control");
    let m = |from: &str, to: &str| {
        assert!(
            manifest.contains(from),
            "fixture text {from:?} not in the manifest"
        );
        manifest.replacen(from, to, 1)
    };
    for (what, bad) in [
        ("a caret range", m("\"=3.5.1\"", "\"3.5.1\"")),
        ("a different exact version", m("\"=3.5.1\"", "\"=3.5.0\"")),
        (
            "default features on",
            m(
                "default-features = false, features = [\"rustls\"]",
                "features = [\"rustls\"]",
            ),
        ),
        (
            "an extra feature",
            m(
                "features = [\"rustls\"]",
                "features = [\"rustls\", \"gzip\"]",
            ),
        ),
        (
            "a different backend",
            m("features = [\"rust_backend\"]", "features = [\"zlib\"]"),
        ),
        ("semver loosened", m("\"=1.0.28\"", "\"1.0.28\"")),
        (
            "tar added",
            format!("{manifest}\n[target.'cfg(unix)'.dependencies]\n")
                .replace("[dev-dependencies]", "tar = \"0.4\"\n[dev-dependencies]"),
        ),
    ] {
        assert!(check_manifest(&bad).is_err(), "must fail on {what}");
    }
    let l = |from: &str, to: &str| {
        assert!(lock.contains(from), "fixture text {from:?} not in the lock");
        lock.replacen(from, to, 1)
    };
    assert!(
        check_lock(&l(
            "name = \"ureq\"\nversion = \"3.5.1\"",
            "name = \"ureq\"\nversion = \"3.5.2\""
        ))
        .is_err(),
        "another ureq in the lock"
    );
    assert!(
        check_lock(&format!(
            "{lock}\n[[package]]\nname = \"aws-lc-rs\"\nversion = \"1.0.0\"\n"
        ))
        .is_err(),
        "a banned crate in the lock"
    );
    assert!(
        check_lock(&format!(
            "{lock}\n[[package]]\nname = \"openssl-sys\"\nversion = \"0.9.0\"\n"
        ))
        .is_err(),
        "openssl-sys in the lock"
    );
    let no_checksum = {
        let i = lock.find("name = \"flate2\"").unwrap();
        let j = lock[i..].find("checksum = ").unwrap() + i;
        let end = lock[j..].find('\n').unwrap() + j;
        format!("{}{}", &lock[..j], &lock[end + 1..])
    };
    assert!(
        check_lock(&no_checksum).is_err(),
        "a lock entry without a checksum"
    );
}
