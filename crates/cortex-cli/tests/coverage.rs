//! CV-01 to CV-04: every `x-cli` string of the pinned S3 twin is either REGISTERED by a group or listed NOT YET with its owner.
//! The twin is read at test time (env CORTEX_S3_TWIN, else the r2.7 pin), so a new pin shows the drift.
mod common;

use common::support;
use cortex_cli::coverage::NOT_YET;
use cortex_cli::{groups, registry::Registry};
use std::collections::{BTreeMap, BTreeSet};

/// The twin's `(x-cli string, x-piece)` per operation, `-` and the health piece's unnamed routes left out.
fn twin_cli() -> Vec<(String, String)> {
    let twin = support::twin();
    let mut v = Vec::new();
    for item in twin["paths"].as_object().unwrap().values() {
        for (m, op) in item.as_object().unwrap() {
            if !["get", "post", "put", "patch", "delete"].contains(&m.as_str()) {
                continue;
            }
            let cli = op["x-cli"].as_str().unwrap_or("-").to_owned();
            if cli != "-" {
                v.push((cli, op["x-piece"].as_str().unwrap().to_owned()));
            }
        }
    }
    v
}

#[test]
fn cv_01_registered_and_not_yet_partition_the_twins_x_cli_strings() {
    let twin: BTreeSet<String> = twin_cli().into_iter().map(|(c, _)| c).collect();
    let registered: BTreeSet<String> = Registry::new(groups::all())
        .unwrap()
        .registered_cli()
        .into_iter()
        .map(str::to_owned)
        .collect();
    let not_yet_vec: Vec<&str> = NOT_YET.iter().map(|(c, _)| *c).collect();
    let not_yet: BTreeSet<String> = not_yet_vec.iter().map(|s| s.to_string()).collect();
    assert_eq!(
        not_yet_vec.len(),
        not_yet.len(),
        "a string is listed NOT YET twice"
    );
    assert!(
        registered.is_disjoint(&not_yet),
        "CV-03: both registered and NOT YET: {:?}",
        registered.intersection(&not_yet)
    );
    let union: BTreeSet<String> = registered.union(&not_yet).cloned().collect();
    assert_eq!(
        union, twin,
        "CV-01/CV-02: the two sets together are exactly the twin's strings"
    );
    assert_eq!(
        twin.len(),
        30,
        "the r2.7 twin has 30 distinct x-cli strings"
    );
    assert_eq!(
        (registered.len(), not_yet.len()),
        (2, 28),
        "today: doctor and audit list"
    );
    assert!(registered.contains("cortex doctor"));
    assert!(registered.contains("cortex audit list"));
}

#[test]
fn cv_02_a_stale_row_in_either_set_fails() {
    let twin: BTreeSet<String> = twin_cli().into_iter().map(|(c, _)| c).collect();
    for (cli, _) in NOT_YET {
        assert!(twin.contains(*cli), "NOT YET row not in the twin: {cli:?}");
    }
    for cli in Registry::new(groups::all()).unwrap().registered_cli() {
        assert!(
            twin.contains(cli),
            "registered string not in the twin: {cli:?}"
        );
    }
}

#[test]
fn cv_04_every_not_yet_row_has_an_owner_and_the_operation_counts_are_the_staffing() {
    let allowed = [
        "bob@helix",
        "nemo@helix",
        "finn@helix",
        "marv@helix",
        "sophi@helix",
    ];
    let owner_of: BTreeMap<&str, &str> = NOT_YET.iter().copied().collect();
    for (cli, owner) in NOT_YET {
        assert!(allowed.contains(owner), "{cli:?} has owner {owner:?}");
    }
    let mut per_owner: BTreeMap<String, usize> = BTreeMap::new();
    let mut registered_ops = 0;
    for (cli, _) in twin_cli() {
        match owner_of.get(cli.as_str()) {
            Some(o) => *per_owner.entry((*o).to_owned()).or_default() += 1,
            None => registered_ops += 1,
        }
    }
    let want: BTreeMap<String, usize> = [
        ("bob@helix", 9),
        ("nemo@helix", 6),
        ("finn@helix", 6),
        ("sophi@helix", 8),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect();
    assert_eq!(per_owner, want, "operations per owner");
    assert_eq!(
        registered_ops, 2,
        "plus doctor and audit list: 31 operations in all"
    );
}
