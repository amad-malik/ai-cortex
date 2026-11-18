//! Repository checks (LAYOUT §6; TESTS F1-01, F1-02).
//!
//! * `fitness`: the crate graph rules (S1 §2.1; strategy P8), from `cargo metadata --no-deps`:
//!   a `mod-*` crate never depends on another `mod-*` crate; `cortex-http` never depends on a
//!   `mod-*` crate; no crate but `cortex-testkit` itself depends on `cortex-testkit` outside
//!   `[dev-dependencies]`. Every dependency kind counts for the first two rules.
//!   Delta 4 adds a source rule: only the chain's one wrapper (`cortex-http/src/idem.rs`) calls
//!   `idempotency::begin` or `idempotency::finish`; `cortex-db` itself is exempt (see
//!   [`idempotency_calls`]).
//! * `licences`: runs `cargo deny check licenses` on a manifest with a given `deny.toml`, and
//!   says whether the licences are accepted (SPEC §2.4, G10, D86).
use std::path::Path;
use std::process::Command;

/// Which crate-graph rule a dependency breaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Rule {
    ModDependsOnMod,
    HttpDependsOnMod,
    ShippedDependsOnTestkit,
    /// Delta 4: a source file outside `cortex-db` and the chain's one wrapper
    /// (`cortex-http/src/idem.rs`) calls `idempotency::begin` or `idempotency::finish`. For this
    /// rule `krate` is the crate directory and `depends_on` names the call and its place.
    IdempotencyOutsideTheChain,
    /// Delta 4b: `Ctx::new` or `with_route` is used in product source outside `cortex-http` (the chain,
    /// which makes the one `Ctx` per request) and `cortex-module` (which defines them). For this rule
    /// `krate` is the crate directory and `depends_on` names the use and its place.
    CtxConstructedOutsideTheChain,
    /// Wave 1 (kai d2a88cf7, D94): a pinned third-party crate is in `Cargo.lock` at another version or
    /// another checksum, or its manifest requirement is not an exact `=` version. `krate` is the crate,
    /// `depends_on` says what is wrong.
    PinnedCrateDrift,
    /// Wave 1 (cox W1-21, after vera's W1S-V1): EVERY serde attribute that makes serde buffer its input
    /// (`flatten`, `untagged`, `tag = ..`, `content = ..`) in product source. `serde_ignored` cannot see an
    /// unknown field inside a buffered payload. `krate` is the crate directory, `depends_on` the key and place.
    BufferedSerdeInProductSource,
    /// Wave 1 ports (d), W1-11: product source outside `cortex-db` (and `cortex-testkit`, which provisions
    /// roles) that names the host role or switches the database role itself (`SET ROLE`, `set_config('role'`).
    /// `krate` is the crate directory, `depends_on` names the text and its place.
    RoleSwitchOutsideCortexDb,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Violation {
    pub rule: Rule,
    pub krate: String,
    pub depends_on: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LicenceVerdict {
    Accepted,
    Refused { detail: String },
}

#[derive(Debug, thiserror::Error)]
pub enum XtaskError {
    #[error("could not run {tool}: {detail}")]
    Tool { tool: String, detail: String },
    #[error("unexpected output from {tool}: {detail}")]
    Output { tool: String, detail: String },
}

fn cargo() -> String {
    std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned())
}

/// The violations of the crate-graph rules in the workspace at `manifest` (empty when it passes),
/// sorted.
pub fn fitness(manifest: &Path) -> Result<Vec<Violation>, XtaskError> {
    let out = Command::new(cargo())
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--offline",
            "--manifest-path",
        ])
        .arg(manifest)
        .output()
        .map_err(|e| XtaskError::Tool {
            tool: "cargo metadata".into(),
            detail: e.to_string(),
        })?;
    if !out.status.success() {
        return Err(XtaskError::Tool {
            tool: "cargo metadata".into(),
            detail: String::from_utf8_lossy(&out.stderr)
                .lines()
                .take(5)
                .collect::<Vec<_>>()
                .join(" | "),
        });
    }
    let doc: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| XtaskError::Output {
            tool: "cargo metadata".into(),
            detail: e.to_string(),
        })?;
    let packages = doc["packages"]
        .as_array()
        .ok_or_else(|| XtaskError::Output {
            tool: "cargo metadata".into(),
            detail: "no packages array".into(),
        })?;

    let mut found = Vec::new();
    for p in packages {
        let name = p["name"].as_str().unwrap_or_default();
        for d in p["dependencies"].as_array().into_iter().flatten() {
            let dep = d["name"].as_str().unwrap_or_default();
            let kind = d["kind"].as_str(); // None for a normal dependency, "dev" or "build"
            let v = |rule| Violation {
                rule,
                krate: name.to_owned(),
                depends_on: dep.to_owned(),
            };
            if name.starts_with("mod-") && dep.starts_with("mod-") {
                found.push(v(Rule::ModDependsOnMod));
            }
            if name == "cortex-http" && dep.starts_with("mod-") {
                found.push(v(Rule::HttpDependsOnMod));
            }
            if dep == "cortex-testkit" && name != "cortex-testkit" && kind != Some("dev") {
                found.push(v(Rule::ShippedDependsOnTestkit));
            }
        }
    }
    if let Some(root) = manifest.parent() {
        found.extend(idempotency_calls(root));
        found.extend(ctx_constructors(root));
        found.extend(pinned_crates(root));
        found.extend(buffered_serde_attributes(root));
        found.extend(role_switches(root));
    }
    found.sort();
    found.dedup();
    Ok(found)
}

/// Delta 4 (and its rework, vera's D4-V2): the PRODUCT SOURCE under `root` that calls, imports or
/// aliases the idempotency store's `begin` or `finish`, outside `cortex-db` and the chain's one
/// wrapper, `cortex-http/src/idem.rs`.
///
/// **The boundary (cox's ruling):** product source is the `.rs` files under a `src/` directory of
/// every crate, and `build.rs`. `tests/`, `benches/` and `examples/` are exempt (cargo never links
/// them into the binary); a `#[cfg(test)]` module inside `src/` stays scanned. Skipped altogether:
/// `target`, `.git`, any `cortex-db` directory, and the repository's own `xtask` (its tests quote the
/// patterns). In a scanned file, comments and string, raw-string and character literals are removed
/// before matching, so a pattern in a message or a doc comment is not a call.
///
/// What counts: `idempotency::begin` or `::finish`; a glob `idempotency::*`; a list
/// `idempotency::{..}` naming `begin`, `finish` or `self` (on one line or several); an alias of the
/// module (`idempotency as x`). **Limits (stated):** a re-export of the module or of `begin` under
/// another name through a crate that is not scanned, a macro that builds the path, and a path split by
/// anything but whitespace are not found. The type system cannot stop a piece calling
/// `cortex_db::idempotency` (it is `pub`); a piece that does still cannot return a stored reply to a
/// client, because only the chain renders one. For this rule a [`Violation`]'s `krate` is the crate
/// directory and `depends_on` names the use and its place (`what (path:line)`).
pub fn idempotency_calls(root: &Path) -> Vec<Violation> {
    let mut out = Vec::new();
    for f in product_sources(root) {
        if f.rel.join("/").ends_with("cortex-http/src/idem.rs") {
            continue;
        }
        for (line, what) in idempotency_uses(&f.code) {
            out.push(f.violation(Rule::IdempotencyOutsideTheChain, what, line));
        }
    }
    out
}

/// Delta 4b (vera's residual): the product source under `root` that makes or replaces a `Ctx`, with
/// `Ctx::new` or `with_route`, outside `cortex-http` (the chain, which makes the one `Ctx` per
/// request) and `cortex-module` (which defines them). Same boundary, skips and literal stripping as
/// [`idempotency_calls`]. Private fields alone do not stop arbitrary trusted Rust from calling the
/// public constructor; this rule does. Limits: a `use ... Ctx as C` alias or a re-export under another
/// name is not followed; `Ctx::default`-like constructors do not exist.
pub fn ctx_constructors(root: &Path) -> Vec<Violation> {
    let mut out = Vec::new();
    for f in product_sources(root) {
        if matches!(f.krate.as_str(), "cortex-http" | "cortex-module") {
            continue;
        }
        for (line, what) in ctx_uses(&f.code) {
            out.push(f.violation(Rule::CtxConstructedOutsideTheChain, what, line));
        }
    }
    out
}

/// One product-source file, already lexed: comments and literals blanked.
struct SourceFile {
    rel: Vec<String>,
    krate: String,
    code: Vec<char>,
    /// Comments blanked but the CONTENT of string literals kept (escapes decoded): for the rules whose
    /// subject is SQL text in a string.
    text: Vec<char>,
}

impl SourceFile {
    fn violation(&self, rule: Rule, what: &str, line: usize) -> Violation {
        Violation {
            rule,
            krate: self.krate.clone(),
            depends_on: format!("{what} ({}:{line})", self.rel.join("/")),
        }
    }
}

/// The product-source `.rs` files under `root` (see [`idempotency_calls`] for the boundary).
fn product_sources(root: &Path) -> Vec<SourceFile> {
    let mut out = Vec::new();
    walk_rs(root, root, &mut out);
    out
}

fn walk_rs(root: &Path, dir: &Path, out: &mut Vec<SourceFile>) {
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = read.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let path = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        let Ok(kind) = e.file_type() else { continue };
        if kind.is_dir() {
            let skip = matches!(name.as_str(), "target" | ".git" | "cortex-db")
                || (dir == root && name == "xtask");
            if !skip {
                walk_rs(root, &path, out);
            }
        } else if kind.is_file() && name.ends_with(".rs") {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            if !is_product_source(&rel) {
                continue;
            }
            let krate = match rel.as_slice() {
                [first, second, ..] if first == "crates" => second.clone(),
                [first, ..] => first.clone(),
                [] => String::new(),
            };
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            out.push(SourceFile {
                rel,
                krate,
                code: code_only(&text),
                text: code_with_strings(&text),
            });
        }
    }
}

/// `src/` of a crate, or its `build.rs`; never anything under `tests/`, `benches/` or `examples/`.
fn is_product_source(rel: &[String]) -> bool {
    let exempt = |c: &String| matches!(c.as_str(), "tests" | "benches" | "examples");
    if let Some(at) = rel.iter().position(|c| c == "src") {
        return !rel[..at].iter().any(exempt);
    }
    rel.last().is_some_and(|f| f == "build.rs") && !rel.iter().any(exempt)
}

fn ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `text` with comments and the CONTENT of string, raw-string and character literals replaced by
/// spaces (newlines kept, so line numbers stay true). Lifetimes are left alone.
fn code_only(text: &str) -> Vec<char> {
    lex(text, false)
}

/// The same lexer with comments and character literals blanked but the CONTENT of string and raw-string
/// literals KEPT, for the rules whose subject is SQL text in a string. The result has the length of the
/// input, so a position still maps to its line: an escape (`\n`, `\x27`, `\u{27}`, `\'`) is decoded
/// into its one character followed by `\0` fillers for the rest of its spelling; a string continuation
/// (a backslash at the end of a line) becomes fillers, its newlines `\u{1}`. Both fillers are ignored by
/// [`squeeze`], and `\u{1}` counts as a newline for line numbers.
fn code_with_strings(text: &str) -> Vec<char> {
    lex(text, true)
}

/// The shared lexer behind [`code_only`] and [`code_with_strings`].
fn lex(text: &str, keep_strings: bool) -> Vec<char> {
    let c: Vec<char> = text.chars().collect();
    let mut out = Vec::with_capacity(c.len());
    let blank = |ch: char| if ch == '\n' { '\n' } else { ' ' };
    let mut i = 0;
    while i < c.len() {
        let next = c.get(i + 1).copied();
        match c[i] {
            '/' if next == Some('/') => {
                while i < c.len() && c[i] != '\n' {
                    out.push(' ');
                    i += 1;
                }
            }
            '/' if next == Some('*') => {
                let mut depth = 0_usize;
                while i < c.len() {
                    if c[i] == '/' && c.get(i + 1) == Some(&'*') {
                        depth += 1;
                        out.extend([' ', ' ']);
                        i += 2;
                    } else if c[i] == '*' && c.get(i + 1) == Some(&'/') {
                        depth -= 1;
                        out.extend([' ', ' ']);
                        i += 2;
                        if depth == 0 {
                            break;
                        }
                    } else {
                        out.push(blank(c[i]));
                        i += 1;
                    }
                }
            }
            '"' if keep_strings => {
                out.push(' ');
                i += 1;
                while i < c.len() {
                    match c[i] {
                        '\\' => i = keep_escape(&c, i, &mut out),
                        '"' => {
                            out.push(' ');
                            i += 1;
                            break;
                        }
                        ch => {
                            out.push(ch);
                            i += 1;
                        }
                    }
                }
            }
            '"' => {
                out.push(' ');
                i += 1;
                while i < c.len() {
                    match c[i] {
                        '\\' => {
                            out.push(' ');
                            i += 1;
                            if i < c.len() {
                                out.push(blank(c[i]));
                                i += 1;
                            }
                        }
                        '"' => {
                            out.push(' ');
                            i += 1;
                            break;
                        }
                        ch => {
                            out.push(blank(ch));
                            i += 1;
                        }
                    }
                }
            }
            'r' if raw_string_hashes(&c, i).is_some() => {
                let hashes = raw_string_hashes(&c, i).unwrap_or(0);
                // r, the hashes and the opening quote
                out.extend(std::iter::repeat_n(' ', hashes + 2));
                i += hashes + 2;
                while i < c.len() {
                    let closes = c[i] == '"' && (0..hashes).all(|k| c.get(i + 1 + k) == Some(&'#'));
                    if closes {
                        out.extend(std::iter::repeat_n(' ', hashes + 1));
                        i += hashes + 1;
                        break;
                    }
                    out.push(if keep_strings { c[i] } else { blank(c[i]) });
                    i += 1;
                }
            }
            '\'' => {
                if next == Some('\\') {
                    // an escaped character literal: '\n', '\'', '\u{..}'
                    out.push(' ');
                    i += 1;
                    while i < c.len() && c[i] != '\'' {
                        out.push(' ');
                        i += 1;
                        if i < c.len() && c[i - 1] == '\\' {
                            out.push(' ');
                            i += 1;
                        }
                    }
                    if i < c.len() {
                        out.push(' ');
                        i += 1;
                    }
                } else if c.get(i + 2) == Some(&'\'') {
                    out.extend([' ', ' ', ' ']);
                    i += 3;
                } else {
                    out.push('\''); // a lifetime or a label
                    i += 1;
                }
            }
            ch => {
                out.push(ch);
                i += 1;
            }
        }
    }
    out
}

/// At `i` a raw string `r"`, `r#"`, `r##"` ... starts (not as the tail of an identifier, but `br`
/// and `cr` are fine): the number of `#`.
fn raw_string_hashes(c: &[char], i: usize) -> Option<usize> {
    if c[i] != 'r' {
        return None;
    }
    if i > 0 && ident_char(c[i - 1]) {
        let prefix = matches!(c[i - 1], 'b' | 'c') && (i < 2 || !ident_char(c[i - 2]));
        if !prefix {
            return None;
        }
    }
    let mut k = i + 1;
    while c.get(k) == Some(&'#') {
        k += 1;
    }
    (c.get(k) == Some(&'"')).then_some(k - i - 1)
}

/// The places in `code` (already comment- and literal-free) that use the store's `begin` or
/// `finish`: `(line, what)`.
fn idempotency_uses(code: &[char]) -> Vec<(usize, &'static str)> {
    const WORD: [char; 11] = ['i', 'd', 'e', 'm', 'p', 'o', 't', 'e', 'n', 'c', 'y'];
    let at = |i: usize, w: &str| {
        w.chars()
            .enumerate()
            .all(|(k, ch)| code.get(i + k) == Some(&ch))
            && !code
                .get(i + w.chars().count())
                .is_some_and(|c| ident_char(*c))
    };
    let skip_ws = |mut i: usize| {
        while code.get(i).is_some_and(|c| c.is_whitespace()) {
            i += 1;
        }
        i
    };
    let mut found = Vec::new();
    let mut i = 0;
    while i + WORD.len() <= code.len() {
        let starts = code[i..i + WORD.len()] == WORD
            && (i == 0 || !ident_char(code[i - 1]))
            && !code.get(i + WORD.len()).is_some_and(|c| ident_char(*c));
        if !starts {
            i += 1;
            continue;
        }
        let line = 1 + code[..i].iter().filter(|c| **c == '\n').count();
        let j = skip_ws(i + WORD.len());
        if code.get(j) == Some(&':') && code.get(j + 1) == Some(&':') {
            let mut k = skip_ws(j + 2);
            // the raw-identifier spelling: idempotency::r#begin
            if code.get(k) == Some(&'r') && code.get(k + 1) == Some(&'#') {
                k += 2;
            }
            if at(k, "begin") {
                found.push((line, "idempotency::begin"));
            } else if at(k, "finish") {
                found.push((line, "idempotency::finish"));
            } else if code.get(k) == Some(&'*') {
                found.push((line, "idempotency::*"));
            } else if code.get(k) == Some(&'{') {
                let end = code[k..]
                    .iter()
                    .position(|c| *c == '}')
                    .map_or(code.len(), |p| k + p);
                let list: String = code[k + 1..end].iter().collect();
                let named = |w: &str| list.split(|c: char| !ident_char(c)).any(|t| t == w);
                if named("begin") || named("finish") || named("self") {
                    found.push((line, "idempotency::{..}"));
                }
            }
        } else if at(j, "as") && code.get(j + 2).is_some_and(|c| c.is_whitespace()) {
            found.push((line, "idempotency (alias)"));
        }
        i += WORD.len();
    }
    found
}

/// The places in `code` (comment- and literal-free) that make or replace a `Ctx`: `Ctx::new` (also
/// `Ctx :: new`, split over lines, or `Ctx::r#new`) and any `with_route` followed by `(`: `(line, what)`.
/// `ConnCtx::new` is not `Ctx::new`: the identifier must start at `Ctx`.
fn ctx_uses(code: &[char]) -> Vec<(usize, &'static str)> {
    let word_at = |i: usize, w: &str| {
        w.chars()
            .enumerate()
            .all(|(k, ch)| code.get(i + k) == Some(&ch))
            && (i == 0 || !ident_char(code[i - 1]))
            && !code
                .get(i + w.chars().count())
                .is_some_and(|c| ident_char(*c))
    };
    let skip_ws = |mut i: usize| {
        while code.get(i).is_some_and(|c| c.is_whitespace()) {
            i += 1;
        }
        i
    };
    let line_of = |i: usize| 1 + code[..i].iter().filter(|c| **c == '\n').count();
    let mut found = Vec::new();
    for i in 0..code.len() {
        if word_at(i, "Ctx") {
            let j = skip_ws(i + 3);
            if code.get(j) == Some(&':') && code.get(j + 1) == Some(&':') {
                let mut k = skip_ws(j + 2);
                if code.get(k) == Some(&'r') && code.get(k + 1) == Some(&'#') {
                    k += 2;
                }
                let new_ok = "new"
                    .chars()
                    .enumerate()
                    .all(|(n, ch)| code.get(k + n) == Some(&ch))
                    && !code.get(k + 3).is_some_and(|c| ident_char(*c));
                if new_ok {
                    found.push((line_of(i), "Ctx::new"));
                }
            }
        } else if word_at(i, "with_route") {
            let j = skip_ws(i + "with_route".len());
            if code.get(j) == Some(&'(') {
                found.push((line_of(i), "with_route"));
            }
        }
    }
    found
}

/// The third-party crates that are PINNED (kai d2a88cf7; D94 register): `(name, version, checksum)`. The
/// version is an exact `=` requirement in the workspace manifest, and the lock must hold exactly this
/// version with exactly this checksum. A different crate, or the same version re-published, fails the
/// check, so the build's gate fails if the crate is not the pinned one (with `--locked`, cargo itself
/// refuses a lock that would change).
pub const PINNED_CRATES: &[(&str, &str, &str)] = &[
    (
        "serde_ignored",
        "0.1.14",
        "115dffd5f3853e06e746965a20dcbae6ee747ae30b543d91b0e089668bb07798",
    ),
    // Wave 1 piece 10 (cox W1-43 Q-4; sophi): the HTTP client and the command-line parser.
    (
        "ureq",
        "3.5.1",
        "e7be76686bdf7198002062423ca0d8cf9b75a27c8e1de804513b97e97b555d9d",
    ),
    (
        "clap",
        "4.6.7",
        "aa8876b300ab35ba921adea3dfd70157a46249b33f95c9084ae5709785478946",
    ),
];

/// The `version` and `checksum` of EVERY package named `name` in the `Cargo.lock` in `dir`, in lock order (empty when
/// the lock is missing, unreadable or does not hold the crate). A name can be listed more than once (two majors of a
/// crate): callers must not pick one of them silently (ellis X-3).
fn locked_entries(dir: &Path, name: &str) -> Vec<(String, Option<String>)> {
    let Ok(lock) = std::fs::read_to_string(dir.join("Cargo.lock")) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let (mut cur_name, mut cur_version, mut cur_sum): (Option<&str>, Option<&str>, Option<&str>) =
        (None, None, None);
    let mut flush = |n: Option<&str>, v: Option<&str>, c: Option<&str>| {
        if n == Some(name) {
            found.push((v.unwrap_or_default().to_owned(), c.map(str::to_owned)));
        }
    };
    for line in lock.lines() {
        let quoted = |key: &str| {
            line.strip_prefix(key)
                .and_then(|r| r.strip_prefix(" = \""))
                .and_then(|r| r.strip_suffix('"'))
        };
        if line.trim() == "[[package]]" {
            flush(cur_name, cur_version, cur_sum);
            (cur_name, cur_version, cur_sum) = (None, None, None);
        } else if let Some(v) = quoted("name") {
            cur_name = Some(v);
        } else if let Some(v) = quoted("version") {
            cur_version = Some(v);
        } else if let Some(v) = quoted("checksum") {
            cur_sum = Some(v);
        }
    }
    flush(cur_name, cur_version, cur_sum);
    found
}

/// The checksum the lock in `dir` holds for `name`, if the crate is in it exactly ONCE. A crate listed more than once has
/// no single checksum, so the answer is `None`, never the last one's.
pub fn locked_checksum(dir: &Path, name: &str) -> Option<String> {
    match locked_entries(dir, name).as_slice() {
        [(_, checksum)] => checksum.clone(),
        _ => None,
    }
}

/// Delta-free pin check (Wave 1; kai d2a88cf7; vera's LOW on the first review): for each [`PINNED_CRATES`] entry
/// that appears in the `Cargo.lock` or in any `Cargo.toml` under `dir`: the lock must hold exactly the pinned
/// version and checksum, and every manifest's requirement must be the exact `"=version"` or inherit the
/// workspace's (`workspace = true`). A crate that appears in neither is not a violation (a fixture or a
/// workspace that does not use it). A pinned crate listed MORE THAN ONCE in the lock (two majors) FAILS CLOSED: one
/// violation with the count and the versions in lock order (the check used to keep the last entry, which hid an
/// older one listed first).
///
/// The manifests are the root `Cargo.toml` first, then every other `Cargo.toml` below `dir` in path order
/// (`target`, `.git` and `xtask/tests/fixtures` skipped). In each, a dependency is read in every form cargo
/// accepts: `name = "req"`, `name = { version = "req", .. }` (one line), a dotted key (`name.version = "req"`,
/// `name.workspace = true`), a table (`[dependencies.name]`, `[workspace.dependencies.name]`,
/// `[target.'cfg'.dependencies.name]`, also `dev-` and `build-`), and a renamed dependency
/// (`other = { package = "name", .. }`), the fully dotted `workspace.dependencies.name = ".."`, and TOML escapes in a
/// quoted key or in `package`. **Limits (stated):** a multi-line inline table (TOML 1.1) and a
/// dependency written by a build script are not read; a requirement given only by `path` or `git` is reported
/// as a missing version. A violation's text starts with the manifest it came from (`Cargo.toml`,
/// `crates/a/Cargo.toml`).
pub fn pinned_crates(dir: &Path) -> Vec<Violation> {
    let manifests = manifest_texts(dir);
    let mut out = Vec::new();
    for (name, version, checksum) in PINNED_CRATES {
        let v = |what: String| Violation {
            rule: Rule::PinnedCrateDrift,
            krate: (*name).to_owned(),
            depends_on: what,
        };
        match locked_entries(dir, name).as_slice() {
            [] => {}
            [(locked_version, locked_sum)] => {
                if locked_version != version {
                    out.push(v(format!(
                        "Cargo.lock holds version {locked_version}, not the pinned {version}"
                    )));
                } else if locked_sum.as_deref() != Some(checksum) {
                    out.push(v(format!(
                        "Cargo.lock holds checksum {}, not the pinned {checksum}",
                        locked_sum.clone().unwrap_or_default()
                    )));
                }
            }
            // Fail closed (ellis X-3): which of several entries the build uses is not for this check to guess.
            many => out.push(v(format!(
                "Cargo.lock holds {} packages named {name} (versions {}), not exactly one",
                many.len(),
                many.iter()
                    .map(|(version, _)| version.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))),
        }
        for (rel, text) in &manifests {
            for dep in dependency_entries(text) {
                if dep.package.as_deref().unwrap_or(&dep.name) != *name || dep.workspace {
                    continue;
                }
                let exact = format!("={version}");
                match dep.version {
                    Some(req) if req != exact => out.push(v(format!(
                        "{rel}: the manifest requires \"{req}\", not the exact \"{exact}\""
                    ))),
                    None => out.push(v(format!(
                        "{rel}: the manifest names no version for it (a path or git source?), not the exact \"{exact}\""
                    ))),
                    _ => {}
                }
            }
        }
    }
    out
}

/// `(path relative to dir, text)` of the root `Cargo.toml` and then every other one below it, in path order.
fn manifest_texts(dir: &Path) -> Vec<(String, String)> {
    fn walk(root: &Path, at: &Path, out: &mut Vec<(String, String)>) {
        let Ok(read) = std::fs::read_dir(at) else {
            return;
        };
        let mut entries: Vec<_> = read.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let path = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            let Ok(kind) = e.file_type() else { continue };
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            if kind.is_dir() {
                if !(matches!(name.as_str(), "target" | ".git") || rel == "xtask/tests/fixtures") {
                    walk(root, &path, out);
                }
            } else if kind.is_file()
                && name == "Cargo.toml"
                && at != root
                && let Ok(text) = std::fs::read_to_string(&path)
            {
                out.push((rel, text));
            }
        }
    }
    let mut out = Vec::new();
    if let Ok(text) = std::fs::read_to_string(dir.join("Cargo.toml")) {
        out.push(("Cargo.toml".to_owned(), text));
    }
    walk(dir, dir, &mut out);
    out
}

/// One dependency as a manifest names it.
#[derive(Debug, Default)]
struct DepEntry {
    name: String,
    package: Option<String>,
    version: Option<String>,
    workspace: bool,
}

/// `line` without a trailing `# comment` (a `#` inside quotes is kept).
fn toml_uncomment(line: &str) -> &str {
    let mut quote: Option<char> = None;
    for (i, ch) in line.char_indices() {
        match (quote, ch) {
            (None, '"' | '\'') => quote = Some(ch),
            (Some(q), c) if c == q => quote = None,
            (None, '#') => return &line[..i],
            _ => {}
        }
    }
    line
}

/// `s` split at `sep` outside quotes and brackets, each part trimmed.
fn toml_split(s: &str, sep: char) -> Vec<String> {
    let (mut parts, mut cur) = (Vec::new(), String::new());
    let (mut quote, mut depth): (Option<char>, usize) = (None, 0);
    for ch in s.chars() {
        match (quote, ch) {
            (None, '"' | '\'') => quote = Some(ch),
            (Some(q), c) if c == q => quote = None,
            (None, '{' | '[') => depth += 1,
            (None, '}' | ']') => depth = depth.saturating_sub(1),
            _ => {}
        }
        if quote.is_none() && depth == 0 && ch == sep {
            parts.push(cur.trim().to_owned());
            cur.clear();
        } else {
            cur.push(ch);
        }
    }
    parts.push(cur.trim().to_owned());
    parts
}

/// The byte index of the first `=` outside quotes.
fn toml_eq(s: &str) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (i, ch) in s.char_indices() {
        match (quote, ch) {
            (None, '"' | '\'') => quote = Some(ch),
            (Some(q), c) if c == q => quote = None,
            (None, '=') => return Some(i),
            _ => {}
        }
    }
    None
}

/// A TOML key or string value without its quotes. A basic (double-quoted) string has its escapes decoded (`\n`,
/// `\"`, `\\`, `\uXXXX`, `\UXXXXXXXX`), so `"serde\u005fignored"` is `serde_ignored`; a literal (single-quoted)
/// string has none.
fn toml_unquote(s: &str) -> String {
    let t = s.trim();
    if let Some(inner) = t.strip_prefix('\'').and_then(|r| r.strip_suffix('\'')) {
        return inner.to_owned();
    }
    let Some(inner) = t.strip_prefix('"').and_then(|r| r.strip_suffix('"')) else {
        return t.to_owned();
    };
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('b') => out.push('\u{8}'),
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('f') => out.push('\u{c}'),
            Some('r') => out.push('\r'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some(u @ ('u' | 'U')) => {
                let width = if u == 'u' { 4 } else { 8 };
                let hex: String = chars.by_ref().take(width).collect();
                if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                    out.push(ch);
                }
            }
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

fn apply_dep_field(e: &mut DepEntry, field: &str, value: &str) {
    match field {
        "version" => e.version = Some(toml_unquote(value)),
        "package" => e.package = Some(toml_unquote(value)),
        "workspace" => e.workspace = value.trim() == "true",
        _ => {}
    }
}

/// The dependencies a manifest's text names, in file order (see [`pinned_crates`] for the forms). Every line is
/// read by its FULL path: the table header's segments followed by the key's dotted segments. The first
/// `dependencies`, `dev-dependencies` or `build-dependencies` segment that has something after it marks a
/// dependency: the next segment is its name, and what follows is nothing (the line gives the requirement, as a
/// string or an inline table) or one field (`version`, `package`, `workspace`). That one rule reads
/// `[dependencies] name = ".."`, `[dependencies.name] version = ".."`, `name.version = ".."` and the fully
/// dotted `workspace.dependencies.name = ".."`. (The FIRST such segment is the table, so a crate that happens to be
/// named like a dependency table is still read as a crate.)
fn dependency_entries(text: &str) -> Vec<DepEntry> {
    const TABLES: [&str; 3] = ["dependencies", "dev-dependencies", "build-dependencies"];
    let mut out: Vec<DepEntry> = Vec::new();
    let mut index: Vec<(Vec<String>, usize)> = Vec::new(); // the path up to the name -> its entry
    let mut header: Vec<String> = Vec::new();
    let segments = |s: &str| -> Vec<String> {
        toml_split(s, '.')
            .iter()
            .map(|part| toml_unquote(part))
            .collect()
    };
    for raw in text.lines() {
        let line = toml_uncomment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(h) = line.strip_prefix('[') {
            header = segments(h.trim_start_matches('[').trim_end_matches(']'));
            continue;
        }
        let Some(eq) = toml_eq(line) else {
            continue;
        };
        let (key, value) = (line[..eq].trim(), line[eq + 1..].trim());
        let mut path = header.clone();
        path.extend(segments(key));
        let Some(at) = path
            .iter()
            .position(|seg| TABLES.contains(&seg.as_str()))
            .filter(|at| at + 1 < path.len())
        else {
            continue;
        };
        let name_path = path[..=at + 1].to_vec();
        let ix = match index.iter().find(|(p, _)| *p == name_path) {
            Some((_, ix)) => *ix,
            None => {
                out.push(DepEntry {
                    name: path[at + 1].clone(),
                    ..DepEntry::default()
                });
                index.push((name_path, out.len() - 1));
                out.len() - 1
            }
        };
        match &path[at + 2..] {
            [] => {
                if let Some(inner) = value.strip_prefix('{').and_then(|v| v.strip_suffix('}')) {
                    for pair in toml_split(inner, ',') {
                        if let Some(eq) = toml_eq(&pair) {
                            apply_dep_field(
                                &mut out[ix],
                                &toml_unquote(pair[..eq].trim()),
                                pair[eq + 1..].trim(),
                            );
                        }
                    }
                } else {
                    out[ix].version = Some(toml_unquote(value));
                }
            }
            [field] => apply_dep_field(&mut out[ix], field, value),
            _ => {}
        }
    }
    out
}

/// Wave 1 (cox W1-16, W1-17, and W1-21 after vera's W1S-V1): the product source under `root` that has a serde
/// attribute which makes serde BUFFER its input, on any type: `flatten`, `untagged`, `tag = ..` (internal and
/// adjacent tagging) and `content = ..`. serde reads such an input into a buffer and deserializes the chosen
/// variant or the flattened struct from it with a deserializer `serde_ignored` never wraps, so an unknown field
/// inside it is accepted by `strict_from_slice` (witness cells `w1_s_4`, `w1_s_9` to `w1_s_11`). A request or
/// query type uses a plain struct or serde's default, externally tagged enum, which IS checked (`w1_s_12`).
/// The rule applies to every type, a `Serialize`-only one included: it cannot tell, and a response type that
/// derives `Deserialize` tomorrow would be unchecked. A piece that needs one asks cox (a reviewed delta:
/// `deny_unknown_fields` on every payload type, `w1_s_13`, a negative cell, vera's pass).
///
/// Same boundary, skips and literal stripping as [`idempotency_calls`]. The attribute is found by its
/// STRUCTURE, not its spelling: an attribute (`#[..]` or `#![..]`, any whitespace between `#`, `!` and `[`,
/// nested brackets balanced) that contains `serde(..)`, `serde [..]` or `serde {..}` (serde and syn accept all three
/// delimiter pairs; also under `cfg_attr`, over several lines), and an item
/// at the TOP LEVEL of that list whose key is one of the four. A word in a nested list (`bound(..)`), a field
/// or variable named `tag`, another attribute namespace (`strum`, `arg`, `serde_with`), `Iterator::flatten` and
/// the other serde attributes are not it. **Limits (stated):** a hand-written `impl Deserialize` that buffers
/// (it names `serde::__private` or `Content`), a `deserialize_with` function that does, and a derive other than
/// serde's are not examined; an attribute written by a macro is not seen.
pub fn buffered_serde_attributes(root: &Path) -> Vec<Violation> {
    let mut out = Vec::new();
    for f in product_sources(root) {
        for (line, key) in buffered_uses(&f.code) {
            let what = match key {
                "flatten" => "serde(flatten)",
                "untagged" => "serde(untagged)",
                "tag" => "serde(tag)",
                _ => "serde(content)",
            };
            out.push(f.violation(Rule::BufferedSerdeInProductSource, what, line));
        }
    }
    out
}

/// Wave 1 ports (d), W1-11 (cox): the PRODUCT SOURCE under `root`, outside `cortex-db` and
/// `cortex-testkit` (which provision the roles), that names the host role `cortex_install_admin` or
/// switches the database role itself: `SET ROLE`, `SET LOCAL ROLE`, `SET SESSION ROLE`, `RESET ROLE`,
/// `SET SESSION AUTHORIZATION` (also `SET LOCAL ...` and `RESET ...`) and `set_config('role'`.
/// `cortex_app` is a member of every role (S1 section 9), so a piece's own SQL could otherwise reach the
/// host role; the transaction's role is `cortex_db`'s alone (`module_role` yields only `cortex_m_<id>`,
/// proved there).
///
/// Same boundary as [`idempotency_calls`] (`src/` of every crate and `build.rs`; `tests/`, `benches/`
/// and `examples/` exempt; `#[cfg(test)]` in `src/` scanned; `target`, `.git`, any `cortex-db` directory
/// and the repository's `xtask` skipped), with one difference: the subject is SQL, so the text of string
/// and raw-string literals IS read. Comments are not. The text is read case-insensitively, with any run
/// of whitespace (a newline or tab included, from the source or from an escape) as one space and no space
/// next to punctuation, and at identifier boundaries (`reset_role_count`, `preset role` and
/// `cortex_install_admin2` are not matches). **Limits (stated):** a role name assembled at run time
/// (`format!`, concatenation across statements, a bound parameter) is not seen; a SQL comment inside the
/// string between the words is not skipped; a quoted identifier (`SET ROLE "cortex_install_admin"`) is
/// found only by the name inside it; and SQL-level backslash escapes inside an `E'..'` string (`E'\x72ole'`) are not
/// decoded; and a dollar delimiter written as Rust escapes (`\x24\x24role\x24\x24`) IS decoded to `$` but is not
/// recognised as a delimiter, because the delimiter reader looks at the neighbouring characters before the escape
/// fillers are dropped (vera's Low on `wave1/ports2`; the fix is to read delimiters after the fillers go). For this rule `krate` is the crate directory and `depends_on` names
/// the text and its place (`what (path:line)`).
pub fn role_switches(root: &Path) -> Vec<Violation> {
    let mut out = Vec::new();
    for f in product_sources(root) {
        if f.krate == "cortex-testkit" {
            continue;
        }
        for (at, what) in role_uses(&f.text) {
            let line = 1 + f.text[..at]
                .iter()
                .filter(|c| matches!(**c, '\n' | '\u{1}'))
                .count();
            out.push(f.violation(Rule::RoleSwitchOutsideCortexDb, what, line));
        }
    }
    out
}

/// The index of the bracket that closes the one at `open` (`(`/`)`, `[`/`]` or `{`/`}`), or the end of `code`.
fn matching(code: &[char], open: usize) -> usize {
    let (o, c) = match code[open] {
        '(' => ('(', ')'),
        '[' => ('[', ']'),
        _ => ('{', '}'),
    };
    let mut depth = 0_usize;
    for (k, ch) in code.iter().enumerate().skip(open) {
        if *ch == o {
            depth += 1;
        } else if *ch == c {
            depth -= 1;
            if depth == 0 {
                return k;
            }
        }
    }
    code.len()
}

/// The `(line, key)` of every buffering key at the top level of a `serde(..)` list inside an attribute.
fn buffered_uses(code: &[char]) -> Vec<(usize, &'static str)> {
    const KEYS: [&str; 4] = ["flatten", "untagged", "tag", "content"];
    let skip_ws = |mut i: usize| {
        while code.get(i).is_some_and(|c| c.is_whitespace()) {
            i += 1;
        }
        i
    };
    let line_of = |i: usize| 1 + code[..i].iter().filter(|c| **c == '\n').count();
    let mut found = Vec::new();
    let mut i = 0;
    while i < code.len() {
        if code[i] != '#' {
            i += 1;
            continue;
        }
        let mut j = skip_ws(i + 1);
        if code.get(j) == Some(&'!') {
            j = skip_ws(j + 1);
        }
        if code.get(j) != Some(&'[') {
            i += 1;
            continue;
        }
        let end = matching(code, j);
        let mut p = j + 1;
        while p < end {
            let is_serde = code[p..end.min(p + 5)] == ['s', 'e', 'r', 'd', 'e']
                && p + 5 <= end
                && (p == 0 || !ident_char(code[p - 1]))
                && !code.get(p + 5).is_some_and(|c| ident_char(*c));
            // serde and syn accept the list in any of the three delimiter pairs (vera's W1S-V1b).
            let paren = skip_ws(p + 5);
            if !(is_serde && matches!(code.get(paren), Some('(' | '[' | '{'))) {
                p += 1;
                continue;
            }
            let close = matching(code, paren).min(end);
            // the top-level items of the list, split at commas outside nested brackets
            let mut depth = 0_usize;
            let mut item_start = true;
            let mut k = paren + 1;
            while k < close {
                let ch = code[k];
                if item_start && !ch.is_whitespace() {
                    item_start = false;
                    let word: String = code[k..close]
                        .iter()
                        .take_while(|c| ident_char(**c))
                        .collect();
                    if let Some(key) = KEYS.iter().find(|w| **w == word) {
                        found.push((line_of(k), *key));
                    }
                }
                match ch {
                    '(' | '[' | '{' => depth += 1,
                    ')' | ']' | '}' => depth = depth.saturating_sub(1),
                    ',' if depth == 0 => item_start = true,
                    _ => {}
                }
                k += 1;
            }
            p = close + 1;
        }
        i = end + 1;
    }
    found
}

/// At `i` in a string literal, a backslash escape (see [`code_with_strings`]): pushes exactly as many
/// characters as the escape has and returns the index after it.
fn keep_escape(c: &[char], i: usize, out: &mut Vec<char>) -> usize {
    let start = i;
    let mut end = i + 1; // past the backslash
    let decoded = match c.get(end) {
        Some('n' | 't' | 'r') => {
            end += 1;
            Some(' ')
        }
        Some(q @ ('\'' | '"' | '\\')) => {
            end += 1;
            Some(*q)
        }
        Some('0') => {
            end += 1;
            Some('\0')
        }
        Some('x') => {
            let hex: String = c[end + 1..].iter().take(2).collect();
            end += 1 + hex.chars().count();
            u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32)
        }
        Some('u') if c.get(end + 1) == Some(&'{') => {
            let close = c[end..].iter().position(|ch| *ch == '}').map(|p| end + p);
            let digits: String = close
                .map(|k| c[end + 2..k].iter().filter(|ch| **ch != '_').collect())
                .unwrap_or_default();
            end = close.map_or(end + 1, |k| k + 1);
            u32::from_str_radix(&digits, 16)
                .ok()
                .and_then(char::from_u32)
        }
        Some('\n' | '\r') => {
            // a string continuation: the newline and the leading whitespace of the next line are dropped
            while c.get(end).is_some_and(|ch| ch.is_whitespace()) {
                end += 1;
            }
            None
        }
        Some(_) => {
            end += 1;
            None
        }
        None => None,
    };
    let end = end.min(c.len());
    if let Some(d) = decoded {
        // A newline written as `\x0a` or `\u{a}` is not a source line: it must not shift the line numbers.
        out.push(if matches!(d, '\n' | '\r') { ' ' } else { d });
        out.extend(std::iter::repeat_n('\0', end - start - 1));
    } else {
        for ch in &c[start..end] {
            out.push(if *ch == '\n' { '\u{1}' } else { '\0' });
        }
    }
    end
}

/// `text` (see [`code_with_strings`]) lowercased, with each run of whitespace one space, fillers dropped and
/// no space next to punctuation: `(char, index in text)`. A dollar-quote delimiter (`$$` or `$tag$`) is read as
/// the quote `'`, and the `E` of an escape string `E'..'` is dropped, so `set_config(E'role'`,
/// `set_config($$role$$` and `set_config($q$role$q$` all read as `set_config('role'` (vera's Low 1 on
/// `wave1/ports`). A placeholder (`$1`) is not a delimiter: a tag does not start with a digit.
fn squeeze(text: &[char]) -> Vec<(char, usize)> {
    let mut v: Vec<(char, usize)> = Vec::with_capacity(text.len());
    let mut pending: Option<usize> = None;
    let mut k = 0;
    while k < text.len() {
        let ch = text[k];
        if ch == '\0' || ch == '\u{1}' {
            k += 1;
            continue;
        }
        if ch.is_whitespace() {
            if !v.is_empty() && pending.is_none() {
                pending = Some(k);
            }
            k += 1;
            continue;
        }
        let (lower, width) = match dollar_delimiter(text, k) {
            Some(w) => ('\'', w),
            None => (ch.to_ascii_lowercase(), 1),
        };
        // `E'..'`: an `e` that starts a word and is followed at once by the quote is the escape-string prefix.
        if lower == '\''
            && pending.is_none()
            && v.last().is_some_and(|(p, _)| *p == 'e')
            && (v.len() == 1 || !ident_char(v[v.len() - 2].0))
        {
            v.pop();
        }
        if let Some(sp) = pending.take() {
            // keep the space only between two identifier characters
            if v.last().is_some_and(|(p, _)| ident_char(*p)) && ident_char(lower) {
                v.push((' ', sp));
            }
        }
        v.push((lower, k));
        k += width;
    }
    v
}

/// At `at` a dollar-quote delimiter: `$$` or `$tag$` (a tag is a letter or `_` and then letters, digits or
/// `_`). Its width in characters, or `None`.
fn dollar_delimiter(text: &[char], at: usize) -> Option<usize> {
    if text[at] != '$' {
        return None;
    }
    let mut j = at + 1;
    if text.get(j) == Some(&'$') {
        return Some(2);
    }
    if !text.get(j).is_some_and(|c| c.is_alphabetic() || *c == '_') {
        return None;
    }
    while text.get(j).is_some_and(|c| ident_char(*c)) {
        j += 1;
    }
    (text.get(j) == Some(&'$')).then_some(j + 1 - at)
}

/// The places in `text` that name the host role or switch the role: `(index in text, what)`.
fn role_uses(text: &[char]) -> Vec<(usize, &'static str)> {
    const NEEDLES: [(&str, &str); 9] = [
        ("set role", "SET ROLE"),
        ("set local role", "SET LOCAL ROLE"),
        ("set session role", "SET SESSION ROLE"),
        ("reset role", "RESET ROLE"),
        ("set session authorization", "SET SESSION AUTHORIZATION"),
        (
            "set local session authorization",
            "SET LOCAL SESSION AUTHORIZATION",
        ),
        ("reset session authorization", "RESET SESSION AUTHORIZATION"),
        ("set_config('role'", "set_config('role'"),
        ("cortex_install_admin", "cortex_install_admin"),
    ];
    let sq = squeeze(text);
    let mut found = Vec::new();
    for start in 0..sq.len() {
        for (needle, what) in NEEDLES {
            let n: Vec<char> = needle.chars().collect();
            let matches = start + n.len() <= sq.len()
                && n.iter().enumerate().all(|(k, ch)| sq[start + k].0 == *ch);
            if !matches {
                continue;
            }
            let before_ok = start == 0 || !ident_char(sq[start - 1].0) || !ident_char(n[0]);
            let after_ok = sq
                .get(start + n.len())
                .is_none_or(|(ch, _)| !ident_char(*ch) || !ident_char(n[n.len() - 1]));
            if before_ok && after_ok {
                found.push((sq[start].1, what));
            }
        }
    }
    found
}

/// Runs `cargo deny check licenses` on `manifest` with the config `deny_toml`. A licence refusal
/// is `Refused`; a tool failure that is not a licence verdict is an error, never `Accepted`.
pub fn licences(manifest: &Path, deny_toml: &Path) -> Result<LicenceVerdict, XtaskError> {
    let out = Command::new(cargo())
        .args(["deny", "--manifest-path"])
        .arg(manifest)
        .arg("--config")
        .arg(deny_toml)
        .args(["check", "licenses"])
        .output()
        .map_err(|e| XtaskError::Tool {
            tool: "cargo deny".into(),
            detail: e.to_string(),
        })?;
    // cargo-deny 0.20 writes the per-check summary ("licenses FAILED") to stdout and the
    // diagnostics ("error[rejected]") to stderr. A refusal needs both; anything else that fails
    // is a tool error, never a verdict.
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if out.status.success() {
        return Ok(LicenceVerdict::Accepted);
    }
    if stdout.contains("licenses FAILED") && stderr.contains("error[rejected]") {
        let detail = stderr
            .lines()
            .filter(|l| l.contains("rejected") || l.contains(" - "))
            .take(6)
            .collect::<Vec<_>>()
            .join(" | ");
        return Ok(LicenceVerdict::Refused { detail });
    }
    Err(XtaskError::Tool {
        tool: "cargo deny".into(),
        detail: stderr.lines().rev().take(5).collect::<Vec<_>>().join(" | "),
    })
}
