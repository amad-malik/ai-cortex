//! The `test-faults` feature is declared, is empty, and is NOT a default feature (contract r0.3 section 4). DB-free. RED first.
//! (The release-build refusal is `xtask fitness`'s, lane A; this proves the manifest never turns the feature on by itself.)

#[test]
fn ft_06_test_faults_is_declared_empty_and_not_default() {
    let toml = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .expect("manifest");
    let features: Vec<&str> = toml
        .split("\n[")
        .find(|t| t.starts_with("features]"))
        .map(|t| {
            t.lines()
                .skip(1)
                .take_while(|l| !l.starts_with('['))
                .collect()
        })
        .expect("a [features] table");
    assert!(
        features.iter().any(|l| l.trim() == "test-faults = []"),
        "test-faults must be declared and empty: {features:?}"
    );
    assert!(
        !features
            .iter()
            .any(|l| l.trim_start().starts_with("default")),
        "no default feature may exist (it could turn test-faults on): {features:?}"
    );
}

/// The value list of one feature in the `[features]` table (`name = ["a", "b"]`).
fn feature_list(name: &str) -> Vec<String> {
    let toml = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .expect("manifest");
    let line = toml
        .lines()
        .find(|l| l.trim_start().starts_with(&format!("{name} = [")))
        .unwrap_or_else(|| panic!("feature {name} is declared"));
    let inner = line
        .split_once('[')
        .and_then(|(_, rest)| rest.rsplit_once(']'))
        .map(|(inside, _)| inside)
        .expect("a list");
    inner
        .split(',')
        .map(|s| s.trim().trim_matches('"').to_owned())
        .filter(|s| !s.is_empty())
        .collect()
}

/// vera SERVE-NEXT-L1: Cargo lets `mod-search` be selected without `mod-records` (search reads the records scope) and `mod-boot` without
/// `mod-records` and `mod-handoffs` (boot uses both ports); the registry then refuses the assembly. A feature that needs another piece
/// must IMPLY it, so every selectable profile is a dependency-complete one. DB-free.
#[test]
fn ft_07_a_feature_implies_the_pieces_its_piece_needs() {
    let search = feature_list("mod-search");
    assert!(
        search.iter().any(|f| f == "mod-records"),
        "mod-search must imply mod-records: {search:?}"
    );
    let boot = feature_list("mod-boot");
    for needed in ["mod-records", "mod-handoffs"] {
        assert!(
            boot.iter().any(|f| f == needed),
            "mod-boot must imply {needed}: {boot:?}"
        );
    }
    // The control: a feature with no needs stays a plain optional dependency, so this test cannot pass by listing everything everywhere.
    assert_eq!(feature_list("mod-records"), ["dep:mod-records"]);
    assert_eq!(feature_list("mod-policy"), ["dep:mod-policy"]);
}
