//! Shared test helpers: the frozen S3 twin and the S2 pin, read at test time (read-only).
#![allow(dead_code)]

use std::path::PathBuf;

pub mod audit;
pub mod chain;
pub mod seed;

/// The S3 r2.5 JSON twin (`bb8775e8…`). `CORTEX_S3_TWIN` overrides the pinned path.
pub fn twin() -> serde_json::Value {
    let p = std::env::var("CORTEX_S3_TWIN").unwrap_or_else(|_| {
        "/path/to/docs/mvp/wave0-partB-pins-20261106-S3r25/S3/cortex-v2-skeleton.openapi.r2.5-bb8775e8.json".to_owned()
    });
    let text = std::fs::read_to_string(PathBuf::from(&p)).expect("the S3 twin is readable");
    serde_json::from_str(&text).expect("the S3 twin is JSON")
}

/// `mvp/api/shared-problem-codes.json`: the ONE source of the title and fix of every problem code that
/// more than one piece declares (cox W1-34). `CORTEX_SHARED_PROBLEM_CODES` overrides the path.
pub fn shared_problem_codes() -> serde_json::Value {
    let p = std::env::var("CORTEX_SHARED_PROBLEM_CODES")
        .unwrap_or_else(|_| "/path/to/docs/mvp/api/shared-problem-codes.json".to_owned());
    let text = std::fs::read_to_string(PathBuf::from(&p))
        .expect("the shared problem codes file is readable");
    serde_json::from_str(&text).expect("the shared problem codes file is JSON")
}

/// The S2 revision 6 `identity.sql` (`c08fab0dc0d0…`). `CORTEX_S2_IDENTITY_SQL` overrides the path.
pub fn s2_identity_sql() -> String {
    let p = std::env::var("CORTEX_S2_IDENTITY_SQL").unwrap_or_else(|_| {
        "/path/to/docs/mvp/wave0-partB-pins-20261106-S2r6/S2/identity.sql".to_owned()
    });
    std::fs::read_to_string(PathBuf::from(&p)).expect("the S2 pin is readable")
}

/// A tiny matcher for the S3 label patterns, which have exactly one shape:
/// `^[CLASS][CLASS]{0,N}$` (ASCII ranges and a trailing literal hyphen). It is written from the
/// pattern TEXT in the twin, so it is an oracle that does not share code with the crate (ID-57).
pub struct Shape {
    first: Vec<(u8, u8)>,
    rest: Vec<(u8, u8)>,
    max_rest: usize,
}

fn parse_class(c: &str) -> Vec<(u8, u8)> {
    let b = c.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if i + 2 < b.len() && b[i + 1] == b'-' {
            out.push((b[i], b[i + 2]));
            i += 3;
        } else {
            out.push((b[i], b[i]));
            i += 1;
        }
    }
    out
}

impl Shape {
    pub fn from_pattern(p: &str) -> Shape {
        let inner = p
            .strip_prefix('^')
            .and_then(|s| s.strip_suffix('$'))
            .expect("anchored");
        let (c1, tail) = inner
            .strip_prefix('[')
            .and_then(|s| s.split_once(']'))
            .expect("first class");
        let (c2, q) = tail
            .strip_prefix('[')
            .and_then(|s| s.split_once(']'))
            .expect("second class");
        let n: usize = q
            .strip_prefix("{0,")
            .and_then(|s| s.strip_suffix('}'))
            .expect("quantifier")
            .parse()
            .expect("number");
        Shape {
            first: parse_class(c1),
            rest: parse_class(c2),
            max_rest: n,
        }
    }

    /// Matches to the END of the string only (no `$` before a final newline).
    pub fn matches(&self, s: &str) -> bool {
        let b = s.as_bytes();
        let inr = |set: &[(u8, u8)], c: u8| set.iter().any(|(lo, hi)| (*lo..=*hi).contains(&c));
        !b.is_empty()
            && inr(&self.first, b[0])
            && b.len() - 1 <= self.max_rest
            && b[1..].iter().all(|c| inr(&self.rest, *c))
    }
}
