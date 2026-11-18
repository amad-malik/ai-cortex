//! Delta 4, mira's half of the fitness rule (finn's xtask covers the rest of the workspace):
//! `idempotency::begin(` and `idempotency::finish(` are called from `src/idem.rs` and nowhere else
//! in this crate. A stored reply therefore leaves the database only through the one wrapper that
//! takes the erased `&Authorization`, so no code path can replay without a proof (R-19).
//!
//! DB-free: it reads the crate's source text.
use std::fs;
use std::path::Path;

const CALLS: [&str; 2] = ["idempotency::begin(", "idempotency::finish("];

/// The code part of a line: everything before a `//` (a comment may name the calls).
fn code(line: &str) -> &str {
    line.split("//").next().unwrap_or("")
}

#[test]
fn only_idem_rs_calls_idempotency_begin_and_finish() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut scanned = 0;
    let mut offenders = Vec::new();
    for entry in fs::read_dir(&src).expect("the src folder") {
        let path = entry.expect("an entry").path();
        if path.extension().is_none_or(|e| e != "rs") {
            continue;
        }
        scanned += 1;
        let name = path
            .file_name()
            .expect("a name")
            .to_string_lossy()
            .into_owned();
        let text = fs::read_to_string(&path).expect("a source file");
        for (n, line) in text.lines().enumerate() {
            if name != "idem.rs" && CALLS.iter().any(|c| code(line).contains(c)) {
                offenders.push(format!("{name}:{}: {}", n + 1, line.trim()));
            }
        }
    }
    assert!(scanned >= 5, "the scan found only {scanned} source files");
    assert!(
        offenders.is_empty(),
        "only src/idem.rs may call idempotency::begin( and finish(; found: {offenders:#?}"
    );
}

/// The control: the rule is not vacuous, because `idem.rs` does hold both calls.
#[test]
fn the_wrapper_in_idem_rs_holds_both_calls() {
    let text = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/idem.rs"))
        .expect("src/idem.rs");
    for call in CALLS {
        assert!(
            text.lines().any(|l| code(l).contains(call)),
            "src/idem.rs does not call {call}"
        );
    }
}
