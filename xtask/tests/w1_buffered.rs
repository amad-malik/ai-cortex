//! Wave 1 (cox W1-16, W1-17, and W1-21 after vera's W1S-V1): EVERY serde attribute that makes serde BUFFER its input
//! is FORBIDDEN in product source: `flatten`, `untagged`, `tag = ..` (internal and adjacent tagging) and
//! `content = ..`. Found by tests `w1_s_4` (flatten) and `w1_s_9` to `w1_s_11` (the enum forms): `serde_ignored`
//! cannot see an unknown field inside a buffered payload, so a request or query type that used one would accept
//! what the strict decoders are there to refuse. The externally tagged enum (serde's default) is checked
//! (`w1_s_12`), so request and query types use it, or a plain struct. A piece that truly needs a buffered form
//! asks cox (W1-21): a reviewed delta with `deny_unknown_fields` on every payload type (`w1_s_13`), a negative
//! cell in the owning piece, and vera's pass.
//!
//! Boundary (the same as the idempotency and `Ctx` rules): `src/` of every crate and `build.rs`; `tests/`,
//! `benches/`, `examples/` exempt; a `#[cfg(test)]` module in `src/` stays scanned; comments and string
//! literals stripped. The attribute is found by its structure, not its spelling: an attribute (`#[..]` or
//! `#![..]`, any whitespace between `#` and `[`, nested brackets balanced) that contains `serde(..)` (also
//! through `cfg_attr`, spaced, over several lines), and a TOP-LEVEL item of that list whose key is one of the four.
//!
//! Controls: flagged in a piece's `src/` (every form above); NOT under `tests/`, `benches/`, `examples/`; NOT in a
//! comment or a string literal; NOT `Iterator::flatten()`, a field, variable or module named `tag`, `content`,
//! `untagged` or `flatten`, a word inside a nested list of another key, another attribute namespace (`strum`,
//! `arg`, `command`, `serde_with`), or another serde attribute.
//!
//! RED: the rule is still named and shaped for `flatten` alone (`FlattenInProductSource`), so these cells, which
//! name `BufferedSerdeInProductSource`, find nothing.
use std::path::{Path, PathBuf};

use xtask::{Rule, Violation, fitness};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/fitness")
        .join(name)
        .join("Cargo.toml")
}

fn v(krate: &str, place: &str) -> Violation {
    w(krate, "flatten", place)
}

fn w(krate: &str, key: &str, place: &str) -> Violation {
    Violation {
        rule: Rule::BufferedSerdeInProductSource,
        krate: krate.into(),
        depends_on: format!("serde({key}) ({place})"),
    }
}

#[test]
fn w1_fl_1_tests_comments_literals_iterator_flatten_and_other_attributes_are_not_flagged() {
    assert_eq!(
        fitness(&fixture("flatten-ok")).expect("fitness runs"),
        vec![]
    );
}

#[test]
fn w1_fl_2_a_flatten_attribute_in_product_source_is_found_in_every_form() {
    assert_eq!(
        fitness(&fixture("flatten-bad")).expect("fitness runs"),
        vec![
            v("mod-identity", "mod-identity/src/lib.rs:12"),
            v("mod-identity", "mod-identity/src/lib.rs:5"),
            v("mod-policy", "mod-policy/build.rs:2"),
            v("mod-tokens", "mod-tokens/src/lib.rs:5"),
            v("mod-tokens", "mod-tokens/src/more.rs:3"),
            v("mod-tokens", "mod-tokens/src/more.rs:5"),
        ],
        "a plain flatten, one with other options, one inside cfg_attr, a spaced attribute over several lines, \
         one in a #[cfg(test)] module in src/ and one in build.rs are found"
    );
}

#[test]
fn w1_fl_3_this_workspace_uses_no_flatten_in_product_source() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo root")
        .join("Cargo.toml");
    assert_eq!(fitness(&root).expect("fitness runs"), vec![]);
}

#[test]
fn w1_bu_1_allowed_forms_look_alikes_and_exempt_places_are_not_flagged() {
    assert_eq!(
        fitness(&fixture("buffered-ok")).expect("fitness runs"),
        vec![],
        "the default (externally tagged) enum, fields and variables named tag/content/untagged, other attribute \
         namespaces, a nested list that mentions the words, comments, literals, tests/, benches/, examples/"
    );
}

#[test]
fn w1_bu_2_untagged_tag_and_content_are_found_in_every_form() {
    let mut expected = vec![
        w("mod-tokens", "untagged", "mod-tokens/src/lib.rs:2"),
        w("mod-tokens", "tag", "mod-tokens/src/lib.rs:5"),
        w("mod-tokens", "tag", "mod-tokens/src/lib.rs:8"),
        w("mod-tokens", "content", "mod-tokens/src/lib.rs:8"),
        w("mod-tokens", "untagged", "mod-tokens/src/more.rs:3"),
        w("mod-tokens", "flatten", "mod-tokens/src/more.rs:7"),
        w("mod-tokens", "untagged", "mod-tokens/src/more.rs:11"),
        w("mod-tokens", "tag", "mod-tokens/src/more.rs:17"),
        w("mod-tokens", "untagged", "mod-tokens/src/more.rs:22"),
        w("mod-identity", "tag", "mod-identity/src/inner.rs:1"),
        w("mod-identity", "content", "mod-identity/src/lib.rs:5"),
        w("mod-identity", "tag", "mod-identity/src/lib.rs:5"),
        w("mod-identity", "untagged", "mod-identity/build.rs:2"),
        // vera's W1S-V1b: serde accepts `#[serde { .. }]` and `#[serde [..]]` as well as `#[serde(..)]`.
        w("mod-tokens", "untagged", "mod-tokens/src/delims.rs:2"),
        w("mod-tokens", "tag", "mod-tokens/src/delims.rs:5"),
        w("mod-tokens", "content", "mod-tokens/src/delims.rs:9"),
        w("mod-tokens", "tag", "mod-tokens/src/delims.rs:10"),
        w("mod-tokens", "flatten", "mod-tokens/src/delims.rs:13"),
        w("mod-tokens", "untagged", "mod-tokens/src/delims.rs:15"),
    ];
    expected.sort();
    assert_eq!(
        fitness(&fixture("buffered-bad")).expect("fitness runs"),
        expected,
        "container untagged, internal tag with other options, adjacent tag and content in one attribute, a variant \
         attribute, `# [ serde ( flatten ) ]` spaced out, inside cfg_attr with nested parentheses (one line and \
         several), brace and bracket lists (`serde {{ .. }}`, `serde [ .. ]`, also under cfg_attr and over several lines), a second serde(..) after the first in one cfg_attr, an inner attribute `#![serde(..)]`, in a \
         #[cfg(test)] module in src/ and in build.rs"
    );
}
