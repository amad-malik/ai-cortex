//! Piece 14, test definitions r4, group A: pure parsing and decisions (no fakes, no files, no
//! network). IDs are the definitions' (`pieces/14-cortex-upgrade-TEST-DEFINITIONS-r4.md`).
//!
//! UP-01 here is its decision half (newer / same / lower); the exit codes and "writes nothing"
//! half need the engine and come with it. All data is synthetic; every digest is computed from
//! bytes, never typed.

use cortex_upgrade::choose::{ChooseError, Decision, Installed, Wanted, choose};
use cortex_upgrade::manifest::{
    CortexInfo, Digest, Entry, MANIFEST_MAX_BYTES, Manifest, ManifestError, MigrationClass,
    Release, Rule, ServerImage, parse,
};
use cortex_upgrade::version::{Version, parse_version};
use serde_json::{Value, json};
use sha2::Digest as _;

// ---------------------------------------------------------------- fixtures

fn sha256_bytes(bytes: &[u8]) -> [u8; 32] {
    sha2::Sha256::digest(bytes).into()
}

fn digest_of(bytes: &[u8]) -> String {
    format!(
        "sha256:{}",
        cortex_types::Sha256(sha256_bytes(bytes)).to_hex()
    )
}

fn db_image() -> String {
    format!("example.invalid/db:1@{}", digest_of(b"db image"))
}

fn v(s: &str) -> Version {
    s.parse().expect("test version is SemVer")
}

/// One complete, valid release in the manifest's JSON form.
fn release(version: &str) -> Value {
    let tar = format!("cortex-{version}-x86_64-linux.tar.gz");
    let oci = format!("cortex-server-{version}-x86_64.oci.tar");
    json!({
        "version": version,
        "date": "2026-11-12T00:00:00Z",
        "notes_url": "https://example.invalid/notes",
        "assets": [
            { "name": tar, "url": tar, "digest": digest_of(format!("tar {version}").as_bytes()) },
            { "name": oci, "url": oci, "digest": digest_of(format!("oci {version}").as_bytes()) }
        ],
        "cortex": {
            "label": "0.2.001-rc2",
            "members": 2,
            "server_image": {
                "ref": format!("localhost/cortex-server@{}", digest_of(format!("img {version}").as_bytes())),
                "asset": oci
            },
            "db_image": db_image(),
            "roles_script_digest": digest_of(format!("roles {version}").as_bytes()),
            "sidecar_images": [],
            "migrations": ["platform/0001_platform"],
            "modules": ["audit", "identity", "tokens"],
            "migration_class": "expand"
        }
    })
}

fn body(releases: Vec<Value>) -> Vec<u8> {
    serde_json::to_vec(&json!({ "schema": 1, "releases": releases })).unwrap()
}

fn mutated(mut r: Value, f: impl FnOnce(&mut Value)) -> Value {
    f(&mut r);
    r
}

fn refused(bytes: &[u8]) -> ManifestError {
    parse(bytes).expect_err("this manifest must be refused")
}

/// A refusal message is one line, bounded, with no control character in it (a hostile value must
/// never be able to move the cursor or end the line).
fn assert_safe_line(msg: &str) {
    assert!(!msg.is_empty());
    assert!(msg.len() <= 240, "message too long ({}): {msg}", msg.len());
    assert!(
        !msg.chars().any(char::is_control),
        "control character in: {msg:?}"
    );
}

// ---- direct (no parsing) fixtures for the choose tests

fn rel(version: &str, min: Option<&str>, db: &str) -> Release {
    Release {
        version: v(version),
        date: "2026-11-12T00:00:00Z".into(),
        notes_url: "https://example.invalid/notes".into(),
        assets: vec![],
        cortex: CortexInfo {
            label: "x".into(),
            members: 2,
            min_upgrade_from: min.map(v),
            server_image: ServerImage {
                image_ref: "r".into(),
                asset: "a".into(),
            },
            db_image: db.into(),
            roles_script_digest: Digest([0; 32]),
            sidecar_images: vec![],
            migrations: vec![],
            modules: vec![],
            migration_class: MigrationClass::Expand,
        },
    }
}

fn manifest_of(entries: Vec<Entry>) -> Manifest {
    Manifest { entries }
}

fn usable(version: &str) -> Entry {
    Entry::Usable(Box::new(rel(version, None, &db_image())))
}

fn installed(version: &str) -> Installed {
    Installed {
        version: v(version),
        db_image: db_image(),
    }
}

fn picked(d: Decision<'_>) -> Version {
    match d {
        Decision::Upgrade(r) => r.version.clone(),
        Decision::UpToDate => panic!("expected an upgrade, got UpToDate"),
    }
}

// ---------------------------------------------------------------- UP-02: the manifest rules

#[test]
fn up_02_a_good_manifest_parses_and_keeps_every_field() {
    let m = parse(&body(vec![release("0.2.1-rc.2")])).expect("a good manifest parses");
    assert_eq!(m.entries.len(), 1);
    let Entry::Usable(r) = &m.entries[0] else {
        panic!("expected a usable release")
    };
    assert_eq!(r.version, v("0.2.1-rc.2"));
    assert_eq!(r.assets.len(), 2);
    assert_eq!(r.assets[0].digest, Digest(sha256_bytes(b"tar 0.2.1-rc.2")));
    assert_eq!(r.assets[1].digest, Digest(sha256_bytes(b"oci 0.2.1-rc.2")));
    let names: Vec<&str> = r.cortex.modules.iter().map(|m| m.as_str()).collect();
    assert_eq!(names, ["audit", "identity", "tokens"]);
    assert_eq!(r.cortex.migration_class, MigrationClass::Expand);
    assert_eq!(r.cortex.db_image, db_image());
    assert_eq!(r.cortex.min_upgrade_from, None);
}

#[test]
fn up_02_min_upgrade_from_is_read_as_a_version() {
    let r = mutated(release("0.2.1-rc.3"), |r| {
        r["cortex"]["min_upgrade_from"] = json!("0.2.1-rc.1")
    });
    let m = parse(&body(vec![r])).unwrap();
    let Entry::Usable(r) = &m.entries[0] else {
        panic!()
    };
    assert_eq!(r.cortex.min_upgrade_from, Some(v("0.2.1-rc.1")));
}

#[test]
fn up_02_a_body_that_is_not_json_is_refused() {
    for b in [
        &b"not json"[..],
        b"",
        b"{",
        b"{\"schema\":1,\"releases\":[}",
    ] {
        let e = refused(b);
        assert_eq!(e.rule(), Rule::Json, "{b:?}");
        assert_safe_line(&e.to_string());
    }
}

#[test]
fn up_02_schema_must_be_the_number_1() {
    for schema in [
        json!(2),
        json!(0),
        json!("1"),
        json!(null),
        json!(true),
        json!(1.5),
        json!([1]),
    ] {
        let b =
            serde_json::to_vec(&json!({ "schema": schema, "releases": [release("0.2.1-rc.2")] }))
                .unwrap();
        let e = refused(&b);
        assert_eq!(e.rule(), Rule::Schema, "{schema}");
        assert_safe_line(&e.to_string());
    }
    // An object with no `schema` at all is a schema failure too.
    assert_eq!(refused(b"{}").rule(), Rule::Schema);
}

#[test]
fn up_02_valid_json_of_the_wrong_shape_is_refused_as_shape() {
    assert!(
        parse(&body(vec![release("0.2.1-rc.2")])).is_ok(),
        "control: the right shape parses"
    );
    for b in [
        &b"[]"[..],
        b"3",
        b"\"x\"",
        b"null",
        b"{\"schema\":1}",
        b"{\"schema\":1,\"releases\":{}}",
    ] {
        let e = refused(b);
        assert_eq!(e.rule(), Rule::Shape, "{}", String::from_utf8_lossy(b));
        assert_safe_line(&e.to_string());
    }
}

#[test]
fn up_02_size_cap_is_checked_first_and_at_exactly_one_mib() {
    // Exactly at the limit: a good manifest padded with trailing white space still parses.
    let mut at_limit = body(vec![release("0.2.1-rc.2")]);
    assert!(at_limit.len() < MANIFEST_MAX_BYTES);
    at_limit.resize(MANIFEST_MAX_BYTES, b' ');
    assert_eq!(at_limit.len(), MANIFEST_MAX_BYTES);
    assert!(
        parse(&at_limit).is_ok(),
        "a manifest of exactly the limit is accepted"
    );

    // One byte over: refused as SIZE (the body is valid JSON, so only the size rule can refuse it).
    let mut over = at_limit.clone();
    over.push(b' ');
    let e = refused(&over);
    assert_eq!(
        e,
        ManifestError::TooLarge {
            len: MANIFEST_MAX_BYTES + 1
        }
    );
    assert_eq!(e.rule(), Rule::Size);

    // The size rule comes before the JSON rule: a huge body that is not JSON is a size refusal.
    let junk = vec![b'x'; 2 * MANIFEST_MAX_BYTES];
    assert_eq!(refused(&junk).rule(), Rule::Size);
}

#[test]
fn up_02_every_asset_needs_a_valid_sha256_digest() {
    let h64 = "a".repeat(64);
    let bad: Vec<Value> = vec![
        json!(null), // placeholder replaced below: "no digest at all"
        json!("md5:d41d8cd98f00b204e9800998ecf8427e"),
        json!(format!("sha256:{}", "a".repeat(63))),
        json!(format!("sha256:{}", "a".repeat(65))),
        json!(format!("sha256:{}", "A".repeat(64))),
        json!(format!("sha256:{}", "g".repeat(64))),
        json!(format!("sha512:{h64}")),
        json!(format!("SHA256:{h64}")),
        json!(format!("sha256:{h64} ")),
        json!(format!(" sha256:{h64}")),
        json!(h64.clone()),
        json!("sha256:"),
        json!(""),
        json!(5),
    ];
    for (i, d) in bad.into_iter().enumerate() {
        let r = mutated(release("0.2.1-rc.2"), |r| {
            if i == 0 {
                r["assets"][1].as_object_mut().unwrap().remove("digest");
            } else {
                r["assets"][1]["digest"] = d.clone();
            }
        });
        let e = refused(&body(vec![r]));
        assert_eq!(
            e,
            ManifestError::Digest {
                release: 0,
                asset: 1
            },
            "case {i}: {d}"
        );
        assert_safe_line(&e.to_string());
        assert!(e.to_string().contains("digest"));
    }
}

#[test]
fn up_02_the_roles_script_digest_is_read_per_release_and_is_exactly_what_the_release_names() {
    // cox 14:30Z, Q-7 condition 2: the manifest carries the sha256 of db-roles.sh on its own key
    let m = parse(&body(vec![release("0.2.1-rc.2"), release("0.2.1-rc.3")])).unwrap();
    let digests: Vec<Digest> = m
        .entries
        .iter()
        .map(|e| match e {
            Entry::Usable(r) => r.cortex.roles_script_digest,
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(
        digests,
        [
            Digest(sha256_bytes(b"roles 0.2.1-rc.2")),
            Digest(sha256_bytes(b"roles 0.2.1-rc.3"))
        ],
        "each release has its own"
    );
    assert_ne!(digests[0], digests[1]);
}

#[test]
fn up_02_the_roles_script_digest_is_required_and_strict() {
    let h64 = "a".repeat(64);
    let bad: Vec<Value> = vec![
        json!(null), // placeholder replaced below: "no key at all"
        json!(null),
        json!("md5:d41d8cd98f00b204e9800998ecf8427e"),
        json!(format!("sha256:{}", "a".repeat(63))),
        json!(format!("sha256:{}", "a".repeat(65))),
        json!(format!("sha256:{}", "A".repeat(64))),
        json!(format!("sha256:{}", "g".repeat(64))),
        json!(format!("sha512:{h64}")),
        json!(format!("SHA256:{h64}")),
        json!(format!("sha256:{h64} ")),
        json!(format!(" sha256:{h64}")),
        json!(h64.clone()),
        json!("sha256:"),
        json!(""),
        json!(5),
        json!([format!("sha256:{h64}")]),
        json!({"sha256": h64}),
    ];
    for (i, d) in bad.into_iter().enumerate() {
        let r = mutated(release("0.2.1-rc.2"), |r| {
            if i == 0 {
                r["cortex"]
                    .as_object_mut()
                    .unwrap()
                    .remove("roles_script_digest");
            } else {
                r["cortex"]["roles_script_digest"] = d.clone();
            }
        });
        let e = refused(&body(vec![r]));
        assert_eq!(
            e,
            ManifestError::RolesScriptDigest { release: 0 },
            "case {i}: {d}"
        );
        assert_eq!(e.rule(), Rule::RolesScriptDigest);
        assert_eq!(e.rule().name(), "roles_script_digest");
        assert_safe_line(&e.to_string());
        assert!(e.to_string().contains("roles_script_digest"));
        assert!(!e.to_string().contains(&h64), "no value in the message");
    }
    // the whole manifest is refused, and the message names the release that is bad
    let bad = mutated(release("0.2.1-rc.3"), |r| {
        r["cortex"]["roles_script_digest"] = json!("sha256:abc")
    });
    let e = refused(&body(vec![release("0.2.1-rc.2"), bad]));
    assert_eq!(e, ManifestError::RolesScriptDigest { release: 1 });
    // a release that is skipped (hostile version) is not looked at, as before
    let skipped = mutated(release("1.0.0;touch x"), |r| {
        r["cortex"]
            .as_object_mut()
            .unwrap()
            .remove("roles_script_digest");
    });
    assert!(parse(&body(vec![release("0.2.1-rc.2"), skipped])).is_ok());
}

#[test]
fn up_02_a_bad_manifest_is_never_partly_used() {
    // The first release is good, the second has a bad digest: the whole manifest is refused,
    // not "use the good one" (M-05).
    let bad = mutated(release("0.2.1-rc.3"), |r| {
        r["assets"][0]["digest"] = json!("md5:00")
    });
    let e = refused(&body(vec![release("0.2.1-rc.2"), bad]));
    assert_eq!(
        e,
        ManifestError::Digest {
            release: 1,
            asset: 0
        }
    );
}

#[test]
fn up_02_every_refusal_names_its_rule_in_one_safe_line() {
    let cases: Vec<(Vec<u8>, Rule)> = vec![
        (b"not json".to_vec(), Rule::Json),
        (
            serde_json::to_vec(&json!({"schema": 2, "releases": []})).unwrap(),
            Rule::Schema,
        ),
        (vec![b' '; MANIFEST_MAX_BYTES + 1], Rule::Size),
        (
            body(vec![mutated(release("0.2.1-rc.2"), |r| {
                r["assets"][0].as_object_mut().unwrap().remove("digest");
            })]),
            Rule::Digest,
        ),
        (
            body(vec![mutated(release("0.2.1-rc.2"), |r| {
                r["assets"][0]["digest"] = json!("md5:ab")
            })]),
            Rule::Digest,
        ),
    ];
    for (b, rule) in cases {
        let e = refused(&b);
        assert_eq!(e.rule(), rule);
        let msg = e.to_string();
        assert_safe_line(&msg);
        assert!(
            msg.contains(rule.name()),
            "{msg} should name {}",
            rule.name()
        );
    }
}

#[test]
fn up_02_migration_class_is_expand_or_contract_only() {
    for ok in ["expand", "contract"] {
        let r = mutated(release("0.2.1-rc.2"), |r| {
            r["cortex"]["migration_class"] = json!(ok)
        });
        assert!(parse(&body(vec![r])).is_ok(), "{ok}");
    }
    for bad in [
        json!("Expand"),
        json!("other"),
        json!(""),
        json!(null),
        json!(1),
        json!("expand "),
    ] {
        let r = mutated(release("0.2.1-rc.2"), |r| {
            r["cortex"]["migration_class"] = bad.clone()
        });
        let e = refused(&body(vec![r]));
        assert_eq!(e, ManifestError::MigrationClass { release: 0 }, "{bad}");
    }
}

#[test]
fn up_02_the_cortex_object_is_strict_and_versions_are_unique() {
    assert!(
        parse(&body(vec![release("0.2.1-rc.2")])).is_ok(),
        "control: the base release parses"
    );
    assert!(
        parse(&body(vec![release("0.2.1-rc.2"), release("0.2.1-rc.3")])).is_ok(),
        "control: two versions parse"
    );
    // An unknown key in `cortex` is refused: a typo or a rule this client does not know must not
    // be ignored silently. (Added by hue; for the reviewer.)
    let r = mutated(release("0.2.1-rc.2"), |r| {
        r["cortex"]["modulez"] = json!(["x"])
    });
    assert_eq!(refused(&body(vec![r])).rule(), Rule::Shape);

    // Required parts missing or of the wrong type are shape refusals.
    for path in ["assets", "cortex", "date", "notes_url"] {
        let r = mutated(release("0.2.1-rc.2"), |r| {
            r.as_object_mut().unwrap().remove(path);
        });
        assert_eq!(
            refused(&body(vec![r])).rule(),
            Rule::Shape,
            "missing {path}"
        );
    }
    let r = mutated(release("0.2.1-rc.2"), |r| r["assets"] = json!("none"));
    assert_eq!(refused(&body(vec![r])).rule(), Rule::Shape);

    // A bad min_upgrade_from is not guessed into a version.
    let r = mutated(release("0.2.1-rc.2"), |r| {
        r["cortex"]["min_upgrade_from"] = json!("0.2.001")
    });
    assert_eq!(refused(&body(vec![r])).rule(), Rule::Shape);

    // The same version twice is ambiguous.
    let e = refused(&body(vec![release("0.2.1-rc.2"), release("0.2.1-rc.2")]));
    assert_eq!(e.rule(), Rule::Shape);
}

// ---------------------------------------------------------------- UP-14: labels are not versions

#[test]
fn up_14_a_label_or_a_short_version_is_skipped_with_a_reason() {
    let m = parse(&body(vec![
        release("0.2.001"),
        release("1.0"),
        release("0.2.1-rc.2"),
    ]))
    .unwrap();
    assert_eq!(m.entries.len(), 3);
    for (i, entry) in m.entries[..2].iter().enumerate() {
        let Entry::Skipped(s) = entry else {
            panic!("entry {i} must be skipped")
        };
        assert_eq!(s.index, i);
        assert_safe_line(&s.reason);
    }
    assert!(matches!(m.entries[2], Entry::Usable(_)));
}

#[test]
fn up_14_a_skipped_release_is_never_compared_or_picked() {
    // `0.2.001` would read as 0.2.1 with its leading zeros stripped: installed 0.2.0 must NOT be
    // offered it (M-07).
    let m = parse(&body(vec![release("0.2.001")])).unwrap();
    let d = choose(&installed("0.2.0"), &m, &Wanted::Newest).unwrap();
    assert_eq!(d, Decision::UpToDate);
    // A skipped release does not stop the usable ones next to it from being picked.
    let m = parse(&body(vec![
        release("0.2.001"),
        release("1.0"),
        release("0.2.1-rc.2"),
    ]))
    .unwrap();
    let d = choose(&installed("0.2.0"), &m, &Wanted::Newest).unwrap();
    assert_eq!(picked(d), v("0.2.1-rc.2"));
}

#[test]
fn up_14_asking_for_a_label_with_to_is_refused() {
    assert!(parse_version("0.2.001").is_err());
    assert!(parse_version("1.0").is_err());
    assert!(parse_version("0.2.1-rc.2").is_ok());
}

// ---------------------------------------------------------------- UP-20: hostile versions

fn hostile_versions() -> Vec<String> {
    vec![
        "1.0.0;touch x".into(),
        "$(id)".into(),
        "`id`".into(),
        "1.0.0\n".into(),
        "1.0.0\ntouch x".into(),
        "1.0.0\r".into(),
        "1.0.0\0".into(),
        "\x1b[31m1.0.0".into(),
        " 1.0.0".into(),
        "1.0.0 ".into(),
        "v1.0.0".into(),
        "1".into(),
        "".into(),
        "01.0.0".into(),
        "1.0.0-01".into(),
        "1.0.0-".into(),
        "1.0.0+build".into(),
        "１.０.０".into(),
        "1.0.0-é".into(),
        "9".repeat(200),
        format!("1.0.0-{}", "a".repeat(194)), // 200 characters, valid SemVer shape but over the cap
    ]
}

#[test]
fn up_20_a_hostile_version_is_refused_and_never_echoed_raw() {
    assert_eq!(
        parse_version("1.0.0").unwrap(),
        v("1.0.0"),
        "control: a plain version parses"
    );
    assert_eq!(
        parse_version("0.2.1-rc.2").unwrap(),
        v("0.2.1-rc.2"),
        "control: a pre-release parses"
    );
    for h in hostile_versions() {
        let e = parse_version(&h).expect_err(&format!("must be refused: {h:?}"));
        let msg = e.to_string();
        assert_safe_line(&msg);
        // No more than 40 characters of the value can be in the message.
        assert!(
            !msg.contains(&h) || h.chars().count() <= 40,
            "whole value echoed for {h:?}"
        );
    }
}

#[test]
fn up_20_a_version_that_ends_in_dot_part_is_refused() {
    // `releases/<v>.part` is a staging directory; a release whose own name ends in `.part` would be
    // taken for one (and removed as one).
    assert!(
        parse_version("1.0.0-rc").is_ok(),
        "control: the staged release's own name is a version"
    );
    for h in [
        "1.0.0-rc.part",
        "0.2.1-rc.9.part",
        "1.0.0-part",
        "1.0.0-x.part",
    ] {
        let e = parse_version(h);
        if h.ends_with(".part") {
            assert!(e.is_err(), "{h}");
        } else {
            assert!(e.is_ok(), "control: {h} does not end in .part");
        }
    }
}

#[test]
fn up_20_the_length_cap_is_exactly_64() {
    let ok = format!("1.0.0-{}", "a".repeat(58));
    assert_eq!(ok.len(), 64);
    assert!(parse_version(&ok).is_ok());
    let over = format!("1.0.0-{}", "a".repeat(59));
    assert_eq!(over.len(), 65);
    assert!(parse_version(&over).is_err());
}

#[test]
fn up_20_a_hostile_manifest_version_is_skipped_not_used() {
    for h in hostile_versions() {
        let m = parse(&body(vec![release(&h)])).unwrap();
        let Entry::Skipped(s) = &m.entries[0] else {
            panic!("{h:?} must be skipped")
        };
        assert_safe_line(&s.reason);
        assert!(!s.reason.contains(&h) || h.chars().count() <= 40);
        // And it can never be chosen.
        assert_eq!(
            choose(&installed("0.0.1"), &m, &Wanted::Newest).unwrap(),
            Decision::UpToDate
        );
    }
}

// ---------------------------------------------------------------- UP-24: module ids end up in SQL

#[test]
fn up_24_a_bad_module_id_refuses_the_manifest_and_names_its_position() {
    let bad = [
        "x'; DROP ROLE postgres; --".to_string(),
        "Upper".to_string(),
        "a".repeat(33),
        "a".repeat(200),
        "has space".into(),
        "9starts_with_digit".into(),
        "with-dash".into(),
        "line\nbreak".into(),
        "".into(),
        "é".into(),
    ];
    for b in bad {
        let r = mutated(release("0.2.1-rc.2"), |r| {
            r["cortex"]["modules"] = json!(["audit", b.clone(), "tokens"])
        });
        let e = refused(&body(vec![r]));
        assert_eq!(
            e,
            ManifestError::Modules {
                release: 0,
                position: 1
            },
            "{b:?}"
        );
        let msg = e.to_string();
        assert_safe_line(&msg);
        assert!(!msg.contains(&b) || b.chars().count() <= 40);
    }
}

#[test]
fn up_24_an_empty_list_and_a_non_string_entry_are_refused() {
    let r = mutated(release("0.2.1-rc.2"), |r| {
        r["cortex"]["modules"] = json!([])
    });
    assert_eq!(refused(&body(vec![r])).rule(), Rule::Modules);
    for entry in [
        json!(5),
        json!(null),
        json!(["audit"]),
        json!({"id": "audit"}),
    ] {
        let r = mutated(release("0.2.1-rc.2"), |r| {
            r["cortex"]["modules"] = json!(["audit", entry.clone()])
        });
        assert_eq!(
            refused(&body(vec![r])),
            ManifestError::Modules {
                release: 0,
                position: 1
            },
            "{entry}"
        );
    }
}

#[test]
fn up_24_a_module_id_of_exactly_32_characters_is_accepted() {
    let ok = format!("a{}", "b".repeat(31));
    assert_eq!(ok.len(), 32);
    let r = mutated(release("0.2.1-rc.2"), |r| {
        r["cortex"]["modules"] = json!([ok.clone(), "snake_case_9"])
    });
    let m = parse(&body(vec![r])).unwrap();
    let Entry::Usable(r) = &m.entries[0] else {
        panic!()
    };
    assert_eq!(r.cortex.modules[0].as_str(), ok);
}

// ---------------------------------------------------------------- UP-01 (decision half) and UP-13

#[test]
fn up_01_newer_same_and_lower_releases() {
    let me = installed("0.2.1-rc.1");
    // Newer: offered.
    let m = manifest_of(vec![usable("0.2.1-rc.2")]);
    assert_eq!(
        picked(choose(&me, &m, &Wanted::Newest).unwrap()),
        v("0.2.1-rc.2")
    );
    // Same: nothing to do.
    let m = manifest_of(vec![usable("0.2.1-rc.1")]);
    assert_eq!(
        choose(&me, &m, &Wanted::Newest).unwrap(),
        Decision::UpToDate
    );
    // Lower: nothing to do, never offered.
    let m = manifest_of(vec![usable("0.2.0")]);
    assert_eq!(
        choose(&me, &m, &Wanted::Newest).unwrap(),
        Decision::UpToDate
    );
    // Empty manifest: nothing to do.
    assert_eq!(
        choose(&me, &manifest_of(vec![]), &Wanted::Newest).unwrap(),
        Decision::UpToDate
    );
}

#[test]
fn up_01_the_newest_is_picked_by_semver_precedence() {
    let me = installed("0.2.0");
    let m = manifest_of(vec![
        usable("0.2.1-rc.2"),
        usable("0.2.1"),
        usable("0.2.1-rc.10"),
        usable("0.1.9"),
        usable("0.2.1-rc.3"),
    ]);
    // rc.10 > rc.3 (numeric, not text) and the release 0.2.1 is above every rc of it.
    assert_eq!(
        picked(choose(&me, &m, &Wanted::Newest).unwrap()),
        v("0.2.1")
    );
    let m = manifest_of(vec![
        usable("0.2.1-rc.3"),
        usable("0.2.1-rc.10"),
        usable("0.2.1-rc.2"),
    ]);
    assert_eq!(
        picked(choose(&me, &m, &Wanted::Newest).unwrap()),
        v("0.2.1-rc.10")
    );
}

#[test]
fn up_01_a_pre_release_is_below_its_release() {
    let m = manifest_of(vec![usable("0.2.1-rc.3")]);
    assert_eq!(
        choose(&installed("0.2.1"), &m, &Wanted::Newest).unwrap(),
        Decision::UpToDate
    );
}

#[test]
fn up_13_a_downgrade_is_refused_without_rollback() {
    let me = installed("0.2.1-rc.2");
    let m = manifest_of(vec![usable("0.2.0"), usable("0.2.1-rc.1")]);
    for old in ["0.2.0", "0.2.1-rc.1"] {
        assert_eq!(
            choose(&me, &m, &Wanted::Exactly(v(old))),
            Err(ChooseError::Downgrade),
            "{old}"
        );
    }
    // A downgrade is a downgrade even when the manifest does not list it.
    assert_eq!(
        choose(&me, &m, &Wanted::Exactly(v("0.1.0"))),
        Err(ChooseError::Downgrade)
    );
    // Asking for the installed version is "nothing to do", not an error.
    let m = manifest_of(vec![usable("0.2.1-rc.2")]);
    assert_eq!(
        choose(&me, &m, &Wanted::Exactly(v("0.2.1-rc.2"))).unwrap(),
        Decision::UpToDate
    );
}

#[test]
fn up_13_to_a_version_the_manifest_does_not_have_is_refused() {
    let m = manifest_of(vec![usable("0.2.1-rc.2")]);
    assert_eq!(
        picked(
            choose(
                &installed("0.2.1-rc.1"),
                &m,
                &Wanted::Exactly(v("0.2.1-rc.2"))
            )
            .unwrap()
        ),
        v("0.2.1-rc.2"),
        "control: a version the manifest has is chosen"
    );
    assert_eq!(
        choose(&installed("0.2.1-rc.1"), &m, &Wanted::Exactly(v("0.9.9"))),
        Err(ChooseError::NotInManifest)
    );
    // A skipped entry is not a candidate.
    let m = manifest_of(vec![Entry::Skipped(cortex_upgrade::manifest::Skipped {
        index: 0,
        reason: "r".into(),
    })]);
    assert_eq!(
        choose(
            &installed("0.2.1-rc.1"),
            &m,
            &Wanted::Exactly(v("0.2.1-rc.2"))
        ),
        Err(ChooseError::NotInManifest)
    );
}

#[test]
fn up_13_below_min_upgrade_from_is_refused_and_the_boundary_is_allowed() {
    let m = manifest_of(vec![Entry::Usable(Box::new(rel(
        "0.3.0",
        Some("0.2.1-rc.2"),
        &db_image(),
    )))]);
    // Below: refused, for the newest and for --to.
    assert_eq!(
        choose(&installed("0.2.1-rc.1"), &m, &Wanted::Newest),
        Err(ChooseError::BelowMinUpgradeFrom)
    );
    assert_eq!(
        choose(&installed("0.2.1-rc.1"), &m, &Wanted::Exactly(v("0.3.0"))),
        Err(ChooseError::BelowMinUpgradeFrom)
    );
    // Exactly at the minimum, and above it: allowed.
    assert_eq!(
        picked(choose(&installed("0.2.1-rc.2"), &m, &Wanted::Newest).unwrap()),
        v("0.3.0")
    );
    assert_eq!(
        picked(choose(&installed("0.2.5"), &m, &Wanted::Newest).unwrap()),
        v("0.3.0")
    );
}

#[test]
fn up_13_newest_does_not_fall_back_to_an_older_compatible_release() {
    // 0.3.0 needs rc.2 or later; 0.2.1-rc.2 is fine for rc.1. The newest is refused, not skipped:
    // the operator must be told that a step is needed.
    let m = manifest_of(vec![
        usable("0.2.1-rc.2"),
        Entry::Usable(Box::new(rel("0.3.0", Some("0.2.1-rc.2"), &db_image()))),
    ]);
    assert_eq!(
        choose(&installed("0.2.1-rc.1"), &m, &Wanted::Newest),
        Err(ChooseError::BelowMinUpgradeFrom)
    );
}

#[test]
fn up_13_a_different_db_image_is_refused() {
    let other = format!("example.invalid/db:2@{}", digest_of(b"another db image"));
    let m = manifest_of(vec![Entry::Usable(Box::new(rel(
        "0.2.1-rc.2",
        None,
        &other,
    )))]);
    assert_eq!(
        choose(&installed("0.2.1-rc.1"), &m, &Wanted::Newest),
        Err(ChooseError::DbImageDiffers)
    );
    assert_eq!(
        choose(
            &installed("0.2.1-rc.1"),
            &m,
            &Wanted::Exactly(v("0.2.1-rc.2"))
        ),
        Err(ChooseError::DbImageDiffers)
    );
}

#[test]
fn up_13_every_refusal_is_one_safe_line() {
    for e in [
        ChooseError::Downgrade,
        ChooseError::BelowMinUpgradeFrom,
        ChooseError::DbImageDiffers,
        ChooseError::NotInManifest,
    ] {
        assert_safe_line(&e.to_string());
    }
}
