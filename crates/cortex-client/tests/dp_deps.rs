//! DP-01, DP-02 (the local half), DP-03: the dependency allowlist of both crates, the exact pins of `ureq` and `clap` with the
//! checksums the lock holds, and the crates that must not be in the lock. `xtask fitness` will enforce the pins once finn adds the
//! two rows to its PINNED_CRATES (a lane A diff, mvp/pieces/10-cortex-client-cli-DIFF-for-finn-xtask-pins.md); until then this test does.
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// `(name, the rest of the line)` for every dependency in `section` (`dependencies`, `dev-dependencies`, `build-dependencies`):
/// plain `name = ".."` lines, `name.workspace = true`, `[section.NAME]` tables and `[target.<cfg>.section]` sections.
fn deps(manifest: &str, section: &str) -> Vec<(String, String)> {
    let mut in_section = false;
    let mut out = Vec::new();
    for line in manifest.lines() {
        let t = line.trim();
        if t.starts_with('[') {
            let header = t.trim_matches(|c| c == '[' || c == ']');
            let target = header.starts_with("target.");
            in_section = header == section || (target && header.ends_with(&format!(".{section}")));
            // a table form: [dependencies.NAME] or [target.<cfg>.dependencies.NAME]
            let named = header.strip_prefix(&format!("{section}.")).or_else(|| {
                header
                    .split_once(&format!(".{section}."))
                    .map(|(_, n)| n)
                    .filter(|_| target)
            });
            if let Some(name) = named {
                out.push((name.trim_matches('"').to_owned(), "(table)".to_owned()));
            }
            continue;
        }
        if !in_section || t.is_empty() || t.starts_with('#') {
            continue;
        }
        if let Some((name, rest)) = t.split_once('=') {
            let name = name.trim().trim_end_matches(".workspace").to_owned();
            out.push((name, rest.trim().to_owned()));
        }
    }
    out
}

fn names(v: &[(String, String)]) -> BTreeSet<&str> {
    v.iter().map(|(n, _)| n.as_str()).collect()
}

fn manifest(krate: &str) -> String {
    fs::read_to_string(root().join("crates").join(krate).join("Cargo.toml")).unwrap()
}

#[test]
fn dp_01_the_direct_dependencies_are_exactly_the_allowlist() {
    let client = manifest("cortex-client");
    let cli = manifest("cortex-cli");
    let want = |s: &[&'static str]| s.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(
        names(&deps(&client, "dependencies")),
        want(&[
            "serde",
            "serde_json",
            "thiserror",
            "ureq",
            "uuid",
            "zeroize"
        ])
    );
    assert_eq!(
        names(&deps(&client, "dev-dependencies")),
        want(&["trybuild"])
    );
    assert_eq!(
        names(&deps(&cli, "dependencies")),
        // W1-80 (cox 8 Nov 01:04Z): the `cortex upgrade` group (finn) adds cortex-upgrade to the CLI binary; W1-82 (cox 8 Nov 01:20Z):
        // and cortex-types, because finn's engine needs `cortex_types::SystemClock`, which cortex-upgrade does not re-export.
        want(&[
            "clap",
            "cortex-client",
            "cortex-types",
            "cortex-upgrade",
            "serde_json",
            "thiserror"
        ])
    );
    assert!(
        deps(&cli, "dev-dependencies").is_empty(),
        "cortex-cli has no dev-dependency"
    );
    let banned = [
        "reqwest",
        "hyper",
        "native-tls",
        "openssl",
        "log",
        "tracing",
        "self_update",
        "tokio",
        "axum",
        "tempfile",
    ];
    for m in [&client, &cli] {
        for section in ["dependencies", "dev-dependencies", "build-dependencies"] {
            for (n, _) in deps(m, section) {
                assert!(
                    !banned.contains(&n.as_str()),
                    "{n} is not allowed in {section}"
                );
            }
        }
    }
}

/// `(version, checksum)` of `name` in the lock, if it is there once.
fn locked(name: &str) -> Option<(String, String)> {
    let lock = fs::read_to_string(root().join("Cargo.lock")).unwrap();
    let mut found = Vec::new();
    for block in lock.split("[[package]]").skip(1) {
        let field = |k: &str| {
            block
                .lines()
                .find_map(|l| {
                    l.strip_prefix(&format!("{k} = \""))
                        .and_then(|r| r.strip_suffix('"'))
                })
                .map(str::to_owned)
        };
        if field("name").as_deref() == Some(name) {
            found.push((
                field("version").unwrap_or_default(),
                field("checksum").unwrap_or_default(),
            ));
        }
    }
    (found.len() == 1).then(|| found.remove(0))
}

#[test]
fn dp_02_ureq_and_clap_are_pinned_exactly_and_the_lock_holds_the_pinned_bytes() {
    let client = deps(&manifest("cortex-client"), "dependencies");
    let cli = deps(&manifest("cortex-cli"), "dependencies");
    let spec = |v: &[(String, String)], n: &str| {
        v.iter()
            .find(|(k, _)| k == n)
            .map(|(_, s)| s.clone())
            .unwrap()
    };
    let ureq = spec(&client, "ureq");
    assert!(
        ureq.contains("version = \"=3.5.1\"") && ureq.contains("default-features = false"),
        "{ureq}"
    );
    // `default-features = false` holds the word "features"; a `features = [..]` list is what must be absent
    assert!(
        !ureq.contains("features = ["),
        "no tls feature, no other feature: {ureq}"
    );
    let clap = spec(&cli, "clap");
    assert!(
        clap.contains("version = \"=4.6.7\"") && clap.contains("default-features = false"),
        "{clap}"
    );
    assert!(!clap.contains("derive"), "{clap}");
    assert!(
        clap.contains(r#"features = ["std", "help", "usage", "error-context", "suggestions"]"#),
        "clap's feature list is exactly the five the build needs (no color, no derive): {clap}"
    );
    assert_eq!(
        locked("ureq"),
        Some((
            "3.5.1".to_owned(),
            "e7be76686bdf7198002062423ca0d8cf9b75a27c8e1de804513b97e97b555d9d".to_owned()
        ))
    );
    assert_eq!(
        locked("clap"),
        Some((
            "4.6.7".to_owned(),
            "aa8876b300ab35ba921adea3dfd70157a46249b33f95c9084ae5709785478946".to_owned()
        ))
    );
}

#[test]
fn dp_03_the_lock_has_no_derive_no_second_http_client_and_no_self_updater() {
    let lock = fs::read_to_string(root().join("Cargo.lock")).unwrap();
    for banned in [
        "clap_derive",
        "anstream",
        "reqwest",
        "native-tls",
        "openssl",
        "self_update",
    ] {
        assert!(
            !lock.contains(&format!("name = \"{banned}\"")),
            "{banned} is in Cargo.lock"
        );
    }
    assert!(
        lock.contains("name = \"ureq\"") && lock.contains("name = \"clap\""),
        "positive control"
    );
}

#[test]
fn dp_01_the_manifest_reader_sees_tables_and_target_sections() {
    let m = "[dependencies]\nserde.workspace = true\n[dependencies.reqwest]\nversion = \"0.12\"\n[target.'cfg(unix)'.dependencies]\nlibc = \"0.2\"\n[target.'cfg(unix)'.dependencies.openssl]\nversion = \"0.10\"\n[dev-dependencies]\ntrybuild.workspace = true\n[features]\ndefault = []\n";
    assert_eq!(
        names(&deps(m, "dependencies")),
        ["libc", "openssl", "reqwest", "serde"]
            .into_iter()
            .collect()
    );
    assert_eq!(
        names(&deps(m, "dev-dependencies")),
        ["trybuild"].into_iter().collect()
    );
    assert!(deps(m, "build-dependencies").is_empty());
}

/// The names of the crates `cargo tree -p <krate> --edges normal` lists. Selecting the package keeps `ureq` at the features the
/// crate asks for (the workspace-wide resolve would unify piece 14's `rustls` into it once that piece is merged).
fn tree(krate: &str) -> BTreeSet<String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let out = std::process::Command::new(cargo)
        .args([
            "tree",
            "--locked",
            "--offline",
            "-p",
            krate,
            "--edges",
            "normal",
            "--prefix",
            "none",
        ])
        .current_dir(root())
        .output()
        .expect("run cargo tree");
    assert!(
        out.status.success(),
        "cargo tree -p {krate}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.split_whitespace().next().map(str::to_owned))
        .collect()
}

/// W1-80 (cox 8 Nov 01:04Z), W1-43 Q-4 AMENDED: the cortex-client CRATE stays TLS-free. `cargo tree -p cortex-client` is the
/// client's own resolve, so it holds even when the CLI binary carries rustls through cortex-upgrade (cargo unifies `ureq`'s
/// features only in a build that has both). The https refusal of the endpoint parser is `en_05_the_address_table` and the bad
/// list of `en_04_server_precedence_flag_then_variable_then_default` (tests/en_env.rs): a MERGE CONDITION of W1-80.
#[test]
fn dp_04_the_client_crates_own_tree_has_no_tls_derive_colour_or_a_second_http_client() {
    let banned = [
        "rustls",
        "ring",
        "webpki-roots",
        "clap_derive",
        "anstream",
        "reqwest",
        "native-tls",
        "openssl",
        "self_update",
        "hyper",
        "tokio",
        "axum",
    ];
    let client = tree("cortex-client");
    assert!(
        client.contains("ureq") && client.contains("serde"),
        "positive control: {client:?}"
    );
    for b in banned {
        assert!(!client.contains(b), "{b} is in cortex-client's own tree");
    }
}

/// W1-80: the CLI binary's tree may now hold rustls, ring and webpki-roots (through cortex-upgrade, finn's one dependency line)
/// and nothing else of the old list: no second HTTP client, no native TLS, no self updater, no async runtime or server, no
/// derive and no colour.
#[test]
fn dp_04_the_cli_binarys_tree_has_no_second_http_client_derive_colour_or_self_updater() {
    let banned = [
        "reqwest",
        "native-tls",
        "openssl",
        "self_update",
        "hyper",
        "tokio",
        "axum",
        "clap_derive",
        "anstream",
    ];
    let cli = tree("cortex-cli");
    assert!(
        cli.contains("clap") && cli.contains("cortex-client"),
        "positive control: {cli:?}"
    );
    for b in banned {
        assert!(!cli.contains(b), "{b} is in cortex-cli's own tree");
    }
}
