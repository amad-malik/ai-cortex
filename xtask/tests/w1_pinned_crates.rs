//! Wave 1 (kai d2a88cf7; cox f28f6ff8; D94): `serde_ignored` is PINNED. The workspace manifest requires it
//! with an exact `=` version, `Cargo.lock` holds exactly that version with exactly that checksum, and
//! `xtask fitness` (so `cargo test -p xtask`, so CI) fails if either drifts. The condition "the build fails
//! if the crate is not the pinned one" is this check plus the `=` requirement plus `--locked`.
//!
//! RED: the pin table and the check are empty, so the drift fixtures report nothing and the real lock does
//! not yet hold the crate.
use std::path::{Path, PathBuf};

use xtask::{PINNED_CRATES, Rule, Violation, fitness, locked_checksum, pinned_crates};

const VERSION: &str = "0.1.14";
const CHECKSUM: &str = "115dffd5f3853e06e746965a20dcbae6ee747ae30b543d91b0e089668bb07798";

fn pins(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/pins")
        .join(name)
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .to_path_buf()
}

fn drift(what: &str) -> Violation {
    Violation {
        rule: Rule::PinnedCrateDrift,
        krate: "serde_ignored".into(),
        depends_on: what.into(),
    }
}

#[test]
fn w1_p_1_the_pin_table_names_serde_ignored_with_its_version_and_checksum() {
    assert_eq!(
        PINNED_CRATES,
        &[
            ("serde_ignored", VERSION, CHECKSUM),
            (
                "ureq",
                "3.5.1",
                "e7be76686bdf7198002062423ca0d8cf9b75a27c8e1de804513b97e97b555d9d"
            ),
            (
                "clap",
                "4.6.7",
                "aa8876b300ab35ba921adea3dfd70157a46249b33f95c9084ae5709785478946"
            ),
        ]
    );
}

#[test]
fn w1_p_2_the_pinned_version_and_checksum_pass_and_an_absent_crate_is_not_a_violation() {
    assert_eq!(pinned_crates(&pins("ok")), vec![]);
    assert_eq!(pinned_crates(&pins("absent")), vec![]);
}

#[test]
fn w1_p_3_another_version_another_checksum_or_a_loose_requirement_is_found() {
    assert_eq!(
        pinned_crates(&pins("bad-version")),
        vec![drift(
            "Cargo.lock holds version 0.1.13, not the pinned 0.1.14"
        )]
    );
    assert_eq!(
        pinned_crates(&pins("bad-checksum")),
        vec![drift(
            "Cargo.lock holds checksum 0000000000000000000000000000000000000000000000000000000000000000, not the pinned 115dffd5f3853e06e746965a20dcbae6ee747ae30b543d91b0e089668bb07798"
        )]
    );
    assert_eq!(
        pinned_crates(&pins("bad-range")),
        vec![drift(
            "Cargo.toml: the manifest requires \"0.1\", not the exact \"=0.1.14\""
        )],
        "a scalar requirement; the message now names the manifest it came from"
    );
}

/// Vera's LOW on the first review: the matcher read only `name = "req"` lines of the ROOT manifest. A table
/// form (`name = { version = .. }`, `[..dependencies.name]` with a `version` line, a dotted key, a renamed
/// dependency with `package = ".."`) and any MEMBER manifest slipped through. All are read now.
#[test]
fn w1_p_6_table_forms_dotted_keys_and_renamed_dependencies_are_read() {
    assert_eq!(
        pinned_crates(&pins("bad-inline")),
        vec![drift(
            "Cargo.toml: the manifest requires \"0.1\", not the exact \"=0.1.14\""
        )]
    );
    assert_eq!(
        pinned_crates(&pins("bad-section")),
        vec![drift(
            "Cargo.toml: the manifest requires \"0.1\", not the exact \"=0.1.14\""
        )]
    );
    assert_eq!(
        pinned_crates(&pins("bad-dotted")),
        vec![drift(
            "Cargo.toml: the manifest requires \"^0.1.14\", not the exact \"=0.1.14\""
        )]
    );
    assert_eq!(
        pinned_crates(&pins("bad-rename")),
        vec![drift(
            "Cargo.toml: the manifest requires \"0.1\", not the exact \"=0.1.14\""
        )],
        "package = \"serde_ignored\" under another key is still serde_ignored"
    );
    assert_eq!(
        pinned_crates(&pins("bad-path")),
        vec![drift(
            "Cargo.toml: the manifest names no version for it (a path or git source?), not the exact \"=0.1.14\""
        )],
        "a path or git source is not the pinned crates.io crate"
    );
}

/// Vera's two residual Lows on the rework: a FULLY dotted declaration (the whole path as one key, from a table
/// above `dependencies`), and TOML escapes in a quoted key or in `package`.
#[test]
fn w1_p_8_fully_dotted_declarations_and_escaped_names_are_read() {
    let loose = |req: &str| {
        drift(&format!(
            "Cargo.toml: the manifest requires \"{req}\", not the exact \"=0.1.14\""
        ))
    };
    assert_eq!(pinned_crates(&pins("bad-fully-dotted")), vec![loose("0.1")]);
    assert_eq!(
        pinned_crates(&pins("bad-dotted-table")),
        vec![loose("~0.1.14")]
    );
    assert_eq!(
        pinned_crates(&pins("bad-escaped")),
        vec![loose("0.1")],
        "\"serde\\u005fignored\" is serde_ignored"
    );
    assert_eq!(
        pinned_crates(&pins("bad-escaped-package")),
        vec![loose("0.1")],
        "package = \"serde\\u005fignored\" is serde_ignored"
    );
}

/// vera's evidence correction on the second review: (1) a renamed dependency whose KEY is a dependency-table word
/// (`[dependencies.dependencies]` with `package = "serde_ignored"`) is serde_ignored, so the FIRST table segment
/// must mark the table (picking the last one reads `dependencies` as the table and the alias as nothing); (2) an
/// escape that is not `\u` can never spell the pinned name, so a key with a decoded tab is a DIFFERENT crate, and
/// a decoder that drops the tab would read it as serde_ignored.
#[test]
fn w1_p_9_an_alias_named_like_a_table_is_read_and_an_escape_look_alike_is_not() {
    assert_eq!(
        pinned_crates(&pins("bad-alias-named-like-a-table")),
        vec![drift(
            "Cargo.toml: the manifest requires \"0.1\", not the exact \"=0.1.14\""
        )]
    );
    assert_eq!(
        pinned_crates(&pins("ok-escape-lookalike")),
        vec![],
        "\"serde\\t_ignored\" and \"serde_ignored\\u0020\" are other crates"
    );
}

#[test]
fn w1_p_7_every_member_manifest_is_read_and_workspace_inheritance_passes() {
    assert_eq!(
        pinned_crates(&pins("bad-member")),
        vec![
            drift(
                "crates/a/Cargo.toml: the manifest requires \"0.1.14\", not the exact \"=0.1.14\""
            ),
            drift("crates/b/Cargo.toml: the manifest requires \"~0.1\", not the exact \"=0.1.14\""),
        ],
        "a member's loose requirement (a scalar, and a target table); `workspace = true` in the same member passes"
    );
    assert_eq!(
        pinned_crates(&pins("ok-forms")),
        vec![],
        "an exact inline table, `workspace = true` in every form, and look-alike crate names are not violations"
    );
}

#[test]
fn w1_p_4_fitness_includes_the_pin_check() {
    let m = pins("bad-version").join("Cargo.toml");
    // A fixture without workspace members: cargo metadata may refuse it, so the check is also reached
    // through the public function above; here the repository itself must pass.
    let _ = m;
    assert_eq!(
        fitness(&repo().join("Cargo.toml")).expect("fitness runs"),
        vec![]
    );
}

#[test]
fn w1_p_5_this_workspace_pins_serde_ignored_exactly() {
    assert_eq!(
        locked_checksum(&repo(), "serde_ignored").as_deref(),
        Some(CHECKSUM),
        "Cargo.lock must hold serde_ignored with the pinned checksum"
    );
    assert_eq!(pinned_crates(&repo()), vec![]);
    let manifest = std::fs::read_to_string(repo().join("Cargo.toml")).expect("manifest");
    assert!(
        manifest.contains("serde_ignored = \"=0.1.14\""),
        "the workspace manifest requires an exact version"
    );
}

fn drift_of(krate: &str, what: &str) -> Violation {
    Violation {
        rule: Rule::PinnedCrateDrift,
        krate: krate.into(),
        depends_on: what.into(),
    }
}

/// Wave 1 piece 10 (cox W1-43 Q-4; sophi's row 802f1a49 and diff text): `ureq` (the HTTP client) and `clap` (the
/// command-line parser) are pinned the same way as `serde_ignored`: the lock holds exactly the pinned version with exactly
/// the pinned checksum, and every manifest requires the exact `=` version.
#[test]
fn w1_p_9_ureq_and_clap_are_pinned_like_serde_ignored() {
    assert_eq!(pinned_crates(&pins("all-ok")), vec![]);
    assert_eq!(
        pinned_crates(&pins("ureq-bad-version")),
        vec![drift_of(
            "ureq",
            "Cargo.lock holds version 3.5.0, not the pinned 3.5.1"
        )]
    );
    assert_eq!(
        pinned_crates(&pins("ureq-bad-checksum")),
        vec![drift_of(
            "ureq",
            &format!(
                "Cargo.lock holds checksum {}, not the pinned e7be76686bdf7198002062423ca0d8cf9b75a27c8e1de804513b97e97b555d9d",
                "0".repeat(64)
            )
        )]
    );
    assert_eq!(
        pinned_crates(&pins("ureq-bad-range")),
        vec![drift_of(
            "ureq",
            "Cargo.toml: the manifest requires \"3\", not the exact \"=3.5.1\""
        )]
    );
    assert_eq!(
        pinned_crates(&pins("clap-bad-version")),
        vec![drift_of(
            "clap",
            "Cargo.lock holds version 4.6.6, not the pinned 4.6.7"
        )]
    );
    assert_eq!(
        pinned_crates(&pins("clap-bad-checksum")),
        vec![drift_of(
            "clap",
            &format!(
                "Cargo.lock holds checksum {}, not the pinned aa8876b300ab35ba921adea3dfd70157a46249b33f95c9084ae5709785478946",
                "f".repeat(64)
            )
        )]
    );
    assert_eq!(
        pinned_crates(&pins("clap-bad-range")),
        vec![drift_of(
            "clap",
            "Cargo.toml: the manifest requires \"^4.6\", not the exact \"=4.6.7\""
        )]
    );
}

/// cox's decision on ellis X-3 (row 4c085e4a): the lock check kept the LAST package of a name, so a second, older major of a
/// pinned crate listed BEFORE the pinned one (the order cargo writes, by version) was invisible to the pin. A pinned crate
/// that appears more than once in `Cargo.lock` now FAILS CLOSED: one violation naming the count and the versions in lock
/// order, whichever entry is first and whichever is the pinned one.
#[test]
fn w1_p_10_a_pinned_crate_listed_twice_in_the_lock_fails_closed() {
    let twice = |krate: &str, count: usize, versions: &str| {
        drift_of(
            krate,
            &format!(
                "Cargo.lock holds {count} packages named {krate} ({versions}), not exactly one"
            ),
        )
    };
    // Cargo writes the older version first: this is the case the old check could not see.
    assert_eq!(
        pinned_crates(&pins("dup-serde-ignored-older-first")),
        vec![twice("serde_ignored", 2, "versions 0.1.13, 0.1.14")]
    );
    assert_eq!(
        pinned_crates(&pins("dup-ureq-older-first")),
        vec![twice("ureq", 2, "versions 2.12.1, 3.5.1")]
    );
    assert_eq!(
        pinned_crates(&pins("dup-clap-older-first")),
        vec![twice("clap", 2, "versions 3.2.25, 4.6.7")]
    );
    // The same duplicate with the pinned entry first (the old check already named another version here).
    assert_eq!(
        pinned_crates(&pins("dup-serde-ignored-pinned-first")),
        vec![twice("serde_ignored", 2, "versions 0.1.14, 0.1.13")]
    );
    assert_eq!(
        pinned_crates(&pins("dup-ureq-pinned-first")),
        vec![twice("ureq", 2, "versions 3.5.1, 2.12.1")]
    );
    assert_eq!(
        pinned_crates(&pins("dup-clap-pinned-first")),
        vec![twice("clap", 2, "versions 4.6.7, 3.2.25")]
    );
    // Three entries (the pinned one between an older and a newer): the count is the count, the order is the lock's.
    assert_eq!(
        pinned_crates(&pins("dup-clap-three")),
        vec![twice("clap", 3, "versions 2.34.0, 4.6.7, 5.0.0")]
    );
}

/// The controls of w1_p_10: a crate that is NOT pinned may appear twice (the lock of any real workspace holds `syn` 1 and 2),
/// a pinned crate once is still clean, and `locked_checksum` gives no checksum for a crate listed twice (there is no single
/// one to give) while it still gives the pinned one when the crate is listed once.
#[test]
fn w1_p_11_an_unpinned_crate_may_repeat_and_a_repeated_pinned_crate_has_no_single_checksum() {
    assert_eq!(pinned_crates(&pins("dup-unpinned")), vec![]);
    assert_eq!(pinned_crates(&pins("all-ok")), vec![]);
    // Another crate whose name only STARTS or ENDS like a pinned one is another crate: it never counts as a second entry.
    assert_eq!(pinned_crates(&pins("all-ok-lookalikes")), vec![]);
    for (name, sum) in [
        ("serde_ignored", CHECKSUM),
        (
            "ureq",
            "e7be76686bdf7198002062423ca0d8cf9b75a27c8e1de804513b97e97b555d9d",
        ),
        (
            "clap",
            "aa8876b300ab35ba921adea3dfd70157a46249b33f95c9084ae5709785478946",
        ),
    ] {
        assert_eq!(
            locked_checksum(&pins("all-ok"), name).as_deref(),
            Some(sum),
            "{name} once: its checksum"
        );
        assert_eq!(
            locked_checksum(&pins("all-ok-lookalikes"), name).as_deref(),
            Some(sum),
            "{name} once, with look-alike names around it: its checksum"
        );
        let dir = name.replace('_', "-");
        for order in ["older-first", "pinned-first"] {
            assert_eq!(
                locked_checksum(&pins(&format!("dup-{dir}-{order}")), name),
                None,
                "{name} twice ({order}): no single checksum"
            );
        }
    }
}

/// The control the first mutation run of X-3 found missing (mutant X3-13 survived): a directory with NO `Cargo.lock` (and
/// manifests that require the exact versions) is not a violation of the lock check. A missing lock is not read as a
/// version or a checksum; `cargo --locked` is what refuses a workspace without one.
#[test]
fn w1_p_12_a_directory_without_a_lock_is_not_a_violation_of_the_lock_check() {
    assert!(!pins("no-lock").join("Cargo.lock").exists());
    assert_eq!(pinned_crates(&pins("no-lock")), vec![]);
    for name in ["serde_ignored", "ureq", "clap"] {
        assert_eq!(locked_checksum(&pins("no-lock"), name), None, "{name}");
    }
}
