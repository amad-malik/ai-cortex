//! Comparing the `db` service of two `compose.yml` files (design §4b, UP-29). A database change gets its own release
//! and its own path, so an upgrade whose `db` service differs from the one the running stack started from is refused
//! before anything is changed.
//!
//! This is NOT a YAML parser (cox, W1-24 follow-up: no parser crate). It is a strict reader of the few shapes the
//! `services.db` block, and the top-level `volumes`, `networks` and `secrets` entries it names, may have: block
//! mappings, block lists of scalars, one-line flow lists of scalars, plain and quoted scalars, whole-line comments.
//! EVERYTHING else in those blocks is a refusal ("cannot compare"), because a second parser (podman compose) could
//! read it differently. A message names a key path and a class, never a value.

use crate::stack_env::StackEnv;
use std::collections::{BTreeMap, BTreeSet};

/// Which file a refusal is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// The compose file of the running release (`current/compose.yml`).
    Current,
    /// The compose file of the candidate release.
    Candidate,
}

/// Why a block cannot be compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    Tab,
    Anchor,
    Alias,
    MergeKey,
    Tag,
    BlockScalar,
    FlowMapping,
    MultiLine,
    DuplicateKey,
    UnknownInterpolation,
    UnknownVariable,
    InlineComment,
    BadKey,
    BadIndent,
    UnsupportedShape,
    Escape,
    TooLarge,
    Include,
    Extends,
    TopLevelKey,
    MultiDocument,
    Missing,
    EnvFile,
}

impl Refusal {
    pub fn text(self) -> &'static str {
        match self {
            Refusal::Tab => "a tab",
            Refusal::Anchor => "an anchor",
            Refusal::Alias => "an alias",
            Refusal::MergeKey => "a merge key",
            Refusal::Tag => "a tag",
            Refusal::BlockScalar => "a block scalar",
            Refusal::FlowMapping => "a flow mapping",
            Refusal::MultiLine => "a multi-line value",
            Refusal::DuplicateKey => "a repeated key",
            Refusal::UnknownInterpolation => "an interpolation form that is not supported",
            Refusal::UnknownVariable => "a variable that is not in stack.env",
            Refusal::InlineComment => "a comment after a value",
            Refusal::BadKey => "a key that is not plain",
            Refusal::BadIndent => "inconsistent indentation",
            Refusal::UnsupportedShape => "a shape that is not supported",
            Refusal::Escape => "an escape in a quoted value",
            Refusal::TooLarge => "a file or block that is too large",
            Refusal::Include => "include",
            Refusal::Extends => "extends",
            Refusal::TopLevelKey => "a top-level key that is not allowed",
            Refusal::MultiDocument => "more than one document",
            Refusal::Missing => "a missing block",
            Refusal::EnvFile => "env_file",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ComposeError {
    #[error("cannot compare the db service: the {} compose.yml has {} at {path}", side_text(*.side), .class.text())]
    Cannot {
        side: Side,
        class: Refusal,
        path: String,
    },
    #[error("the db service differs at {path}")]
    Differs { path: String },
}

fn side_text(s: Side) -> &'static str {
    match s {
        Side::Current => "running release's",
        Side::Candidate => "candidate release's",
    }
}

const FILE_MAX_BYTES: usize = 256 * 1024;
const LINE_MAX_BYTES: usize = 1000;
const BLOCK_MAX_LINES: usize = 400;
const DEPTH_MAX: usize = 6;
const FILE_PATH: &str = "compose.yml";

/// What the reader builds. A quoted and a plain scalar with the same text are different (YAML may read them as different
/// types, and a toggle is cheaper to refuse than to reason about).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Node {
    Empty,
    Scalar { text: String, quoted: bool },
    Seq(Vec<Node>),
    Map(BTreeMap<String, Node>),
}

/// A refusal inside the reader: the class and the key path.
type Refuse = (Refusal, String);
type R<T> = Result<T, Refuse>;

fn refuse<T>(class: Refusal, path: &str) -> R<T> {
    Err((class, path.to_string()))
}

/// A line of code: not blank, not a whole-line comment. `indent` counts leading spaces and tabs, so that a tab can never
/// make a line leave its block unseen.
struct Line<'a> {
    indent: usize,
    tab: bool,
    raw_len: usize,
    text: &'a str,
}

fn code_lines(text: &str) -> Vec<Line<'_>> {
    text.lines()
        .filter_map(|raw| {
            let body = raw.trim_start_matches([' ', '\t']);
            if body.trim().is_empty() || body.starts_with('#') {
                return None;
            }
            let lead = &raw[..raw.len() - body.len()];
            Some(Line {
                indent: lead.len(),
                tab: raw.contains('\t'),
                raw_len: raw.len(),
                text: body.trim_end(),
            })
        })
        .collect()
}

/// The variables a compose file of a release may use: exactly those of its own `stack.env`.
struct Vars(BTreeMap<&'static str, String>);

impl Vars {
    fn of(env: &StackEnv) -> Self {
        Vars(BTreeMap::from([
            ("CORTEX_VERSION", env.version.to_string()),
            ("CORTEX_SERVER_IMAGE", env.server_image.0.clone()),
            ("CORTEX_DB_IMAGE", env.db_image.0.clone()),
        ]))
    }
}

/// `${NAME}`, `${NAME:-default}` and `${NAME:?message}` with NAME one of the three variables; anything else that holds a
/// `$` is refused (compose reads the process environment too, which this does not model).
fn substitute(text: &str, vars: &Vars, path: &str) -> R<String> {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find('$') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let Some(inner) = after.strip_prefix('{') else {
            return refuse(Refusal::UnknownInterpolation, path);
        };
        let Some(end) = inner.find('}') else {
            return refuse(Refusal::UnknownInterpolation, path);
        };
        let body = &inner[..end];
        let name_len = body
            .bytes()
            .take_while(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || *b == b'_')
            .count();
        let (name, tail) = body.split_at(name_len);
        let form_ok = tail.is_empty()
            || ((tail.starts_with(":-") || tail.starts_with(":?"))
                && !tail[2..].contains(['$', '{']));
        if name.is_empty() || !form_ok {
            return refuse(Refusal::UnknownInterpolation, path);
        }
        match vars.0.get(name) {
            Some(value) => out.push_str(value),
            None => return refuse(Refusal::UnknownVariable, path),
        }
        rest = &inner[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

/// A plain or quoted scalar (no flow, no structure). `quoted` tells which.
fn scalar(raw: &str, vars: &Vars, path: &str) -> R<Node> {
    let t = raw.trim();
    if t.is_empty() {
        return refuse(Refusal::UnsupportedShape, path);
    }
    let mut chars = t.chars();
    match chars.next().unwrap_or(' ') {
        '&' => return refuse(Refusal::Anchor, path),
        '*' => return refuse(Refusal::Alias, path),
        '!' => return refuse(Refusal::Tag, path),
        '|' | '>' => return refuse(Refusal::BlockScalar, path),
        '{' => return refuse(Refusal::FlowMapping, path),
        '[' => return refuse(Refusal::UnsupportedShape, path),
        '%' | '@' | '`' => return refuse(Refusal::UnsupportedShape, path),
        q @ ('"' | '\'') => {
            let body = &t[1..];
            let Some(close) = body.find(q) else {
                return refuse(Refusal::MultiLine, path);
            };
            let inner = &body[..close];
            let after = body[close + 1..].trim();
            if inner.contains('\\') || (q == '\'' && after.starts_with('\'')) {
                return refuse(Refusal::Escape, path);
            }
            if after.starts_with('#') {
                return refuse(Refusal::InlineComment, path);
            }
            if !after.is_empty() {
                return refuse(Refusal::UnsupportedShape, path);
            }
            return Ok(Node::Scalar {
                text: substitute(inner, vars, path)?,
                quoted: true,
            });
        }
        _ => {}
    }
    if t == "-" || t.starts_with("- ") || t.starts_with("? ") {
        return refuse(Refusal::UnsupportedShape, path);
    }
    if t.contains(" #") {
        return refuse(Refusal::InlineComment, path);
    }
    if t.ends_with(':') || t.contains(": ") {
        return refuse(Refusal::UnsupportedShape, path);
    }
    Ok(Node::Scalar {
        text: substitute(t, vars, path)?,
        quoted: false,
    })
}

/// `[a, "b"]` on one line: scalars only, no nesting, nothing after the closing bracket.
fn flow_seq(raw: &str, vars: &Vars, path: &str) -> R<Node> {
    let t = raw.trim();
    let Some(inner) = t.strip_prefix('[') else {
        return refuse(Refusal::UnsupportedShape, path);
    };
    // split on commas outside quotes; the end of the list is the last `]` outside quotes
    let (mut items, mut cur, mut quote, mut closed) =
        (Vec::new(), String::new(), None::<char>, None::<usize>);
    for (i, c) in inner.char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => {
                quote = None;
                cur.push(c);
            }
            (Some(_), c) => cur.push(c),
            (None, '"' | '\'') => {
                quote = Some(c);
                cur.push(c);
            }
            (None, '[') => return refuse(Refusal::UnsupportedShape, path),
            (None, '{') => return refuse(Refusal::FlowMapping, path),
            (None, ',') => items.push(std::mem::take(&mut cur)),
            (None, ']') => {
                closed = Some(i);
                break;
            }
            (None, c) => cur.push(c),
        }
    }
    let Some(end) = closed else {
        return refuse(Refusal::MultiLine, path);
    };
    let after = inner[end + 1..].trim();
    if after.starts_with('#') {
        return refuse(Refusal::InlineComment, path);
    }
    if !after.is_empty() {
        return refuse(Refusal::UnsupportedShape, path);
    }
    if !cur.trim().is_empty() || !items.is_empty() {
        items.push(cur);
    }
    let mut out = Vec::with_capacity(items.len());
    for (n, item) in items.iter().enumerate() {
        out.push(scalar(item, vars, &format!("{path}[{n}]"))?);
    }
    Ok(Node::Seq(out))
}

fn is_item(text: &str) -> bool {
    text == "-" || text.starts_with("- ")
}

/// `key: rest` with a plain key; `rest` is what follows the colon, trimmed.
fn split_key<'a>(text: &'a str, path: &str) -> R<(&'a str, &'a str)> {
    if text.starts_with("<<:") || text == "<<" {
        return refuse(Refusal::MergeKey, path);
    }
    let n = text
        .bytes()
        .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
        .count();
    let after = &text[n..];
    if n == 0 || n > 64 || !(after == ":" || after.starts_with(": ")) {
        return refuse(Refusal::BadKey, path);
    }
    Ok((&text[..n], after[1..].trim()))
}

/// The lines of the block under a key, as a mapping or a list, depending on the first line.
fn parse_child(
    lines: &[Line<'_>],
    i: &mut usize,
    parent: usize,
    path: &str,
    depth: usize,
    vars: &Vars,
) -> R<Node> {
    if depth >= DEPTH_MAX {
        return refuse(Refusal::UnsupportedShape, path);
    }
    let indent = lines[*i].indent;
    if is_item(lines[*i].text) {
        parse_seq(lines, i, indent, parent, path, vars)
    } else {
        parse_map(lines, i, indent, parent, path, depth, vars)
    }
}

fn misaligned(lines: &[Line<'_>], i: usize, indent: usize, parent: usize, path: &str) -> R<()> {
    match lines.get(i) {
        Some(l) if l.indent > parent && l.indent != indent => refuse(Refusal::BadIndent, path),
        _ => Ok(()),
    }
}

fn parse_map(
    lines: &[Line<'_>],
    i: &mut usize,
    indent: usize,
    parent: usize,
    path: &str,
    depth: usize,
    vars: &Vars,
) -> R<Node> {
    let mut map = BTreeMap::new();
    while *i < lines.len() && lines[*i].indent == indent {
        let text = lines[*i].text;
        if is_item(text) {
            return refuse(Refusal::UnsupportedShape, path);
        }
        let (key, rest) = split_key(text, path)?;
        let here = format!("{path}.{key}");
        if map.contains_key(key) {
            return refuse(Refusal::DuplicateKey, &here);
        }
        *i += 1;
        let value = if rest.is_empty() {
            match lines.get(*i) {
                Some(next) if next.indent > indent => {
                    parse_child(lines, i, indent, &here, depth + 1, vars)?
                }
                _ => Node::Empty,
            }
        } else {
            let v = if rest.starts_with('[') {
                flow_seq(rest, vars, &here)?
            } else {
                scalar(rest, vars, &here)?
            };
            if matches!(lines.get(*i), Some(next) if next.indent > indent) {
                return refuse(Refusal::MultiLine, &here);
            }
            v
        };
        map.insert(key.to_string(), value);
    }
    misaligned(lines, *i, indent, parent, path)?;
    Ok(Node::Map(map))
}

fn parse_seq(
    lines: &[Line<'_>],
    i: &mut usize,
    indent: usize,
    parent: usize,
    path: &str,
    vars: &Vars,
) -> R<Node> {
    let mut out = Vec::new();
    while *i < lines.len() && lines[*i].indent == indent {
        let text = lines[*i].text;
        let here = format!("{path}[{}]", out.len());
        if !is_item(text) {
            return refuse(Refusal::UnsupportedShape, path);
        }
        let item = text.strip_prefix('-').unwrap_or("").trim();
        out.push(scalar(item, vars, &here)?);
        *i += 1;
        if matches!(lines.get(*i), Some(next) if next.indent > indent) {
            return refuse(Refusal::MultiLine, &here);
        }
    }
    misaligned(lines, *i, indent, parent, path)?;
    Ok(Node::Seq(out))
}

/// The checks that look at the whole file before any block is read.
fn scan_file(text: &str) -> R<()> {
    if text.len() > FILE_MAX_BYTES {
        return refuse(Refusal::TooLarge, FILE_PATH);
    }
    for raw in text.lines() {
        let body = raw.trim_start_matches([' ', '\t']);
        if body.trim().is_empty() || body.starts_with('#') {
            continue;
        }
        if body.starts_with("extends:") {
            return refuse(Refusal::Extends, FILE_PATH);
        }
        if raw.len() != body.len() {
            continue; // indented: part of a block
        }
        if raw.trim_end() == "---" || raw.trim_end() == "..." || raw.starts_with("--- ") {
            return refuse(Refusal::MultiDocument, FILE_PATH);
        }
        let key = raw.split(':').next().unwrap_or("");
        match key {
            "name" | "services" | "networks" | "volumes" | "secrets"
                if raw[key.len()..].starts_with(':') => {}
            "include" if raw[key.len()..].starts_with(':') => {
                return refuse(Refusal::Include, FILE_PATH);
            }
            _ => return refuse(Refusal::TopLevelKey, FILE_PATH),
        }
    }
    Ok(())
}

/// The checks every block we read goes through: size, line length, tabs.
fn check_block(block: &[Line<'_>], path: &str) -> R<()> {
    if block.len() > BLOCK_MAX_LINES || block.iter().any(|l| l.raw_len > LINE_MAX_BYTES) {
        return refuse(Refusal::TooLarge, path);
    }
    if block.iter().any(|l| l.tab) {
        return refuse(Refusal::Tab, path);
    }
    Ok(())
}

/// The lines of a block: those after position `at` that are indented deeper than `indent`. A line with a tab stays in (it is
/// refused by `check_block`), so a tab line cannot make a block end early.
fn block_after<'a, 'b>(lines: &'b [Line<'a>], at: usize, indent: usize) -> &'b [Line<'a>] {
    let n = lines[at + 1..]
        .iter()
        .take_while(|l| l.indent > indent || l.tab)
        .count();
    &lines[at + 1..at + 1 + n]
}

/// The `services.db` block of a file as a node.
fn db_node(text: &str, vars: &Vars) -> R<Node> {
    let lines = code_lines(text);
    let services = lines
        .iter()
        .position(|l| l.indent == 0 && l.text == "services:");
    let Some(at) = services else {
        return refuse(Refusal::Missing, "services.db");
    };
    let section = block_after(&lines, at, 0);
    let Some(first) = section.first() else {
        return refuse(Refusal::Missing, "services.db");
    };
    let s = first.indent;
    let mut found = section
        .iter()
        .enumerate()
        .filter(|(_, l)| l.indent == s && l.text == "db:")
        .map(|(n, _)| n);
    let Some(db) = found.next() else {
        return refuse(Refusal::Missing, "services.db");
    };
    if found.next().is_some() {
        return refuse(Refusal::DuplicateKey, "services.db");
    }
    let block = block_after(section, db, s);
    check_block(block, "services.db")?;
    if block.is_empty() {
        return Ok(Node::Empty);
    }
    let mut i = 0;
    let node = parse_map(block, &mut i, block[0].indent, s, "services.db", 0, vars)?;
    if i < block.len() {
        return refuse(Refusal::BadIndent, "services.db");
    }
    Ok(node)
}

/// The names of the volumes, networks and secrets a `db` node refers to.
fn names_used(db: &Node) -> BTreeMap<&'static str, BTreeSet<String>> {
    let mut out: BTreeMap<&'static str, BTreeSet<String>> = BTreeMap::new();
    let Node::Map(m) = db else { return out };
    for kind in ["volumes", "networks", "secrets"] {
        let set = out.entry(kind).or_default();
        match m.get(kind) {
            Some(Node::Seq(items)) => {
                for item in items {
                    let Node::Scalar { text, .. } = item else {
                        continue;
                    };
                    if kind == "volumes" && (text.starts_with(['.', '/', '~']) || text.is_empty()) {
                        continue; // a bind mount, not a named volume
                    }
                    set.insert(text.split(':').next().unwrap_or("").to_string());
                }
            }
            Some(Node::Map(entries)) => set.extend(entries.keys().cloned()),
            _ => {}
        }
    }
    // A service that names no network is attached to the implicit `default` network, whose settings are the top-level
    // `networks.default` entry (Compose Specification, section Networks): they are part of what the db service is.
    if let Some(networks) = out.get_mut("networks")
        && networks.is_empty()
    {
        networks.insert("default".to_string());
    }
    out
}

/// `volumes.<name>` and the like, as a node; `None` when the file has no such entry.
fn top_entry(text: &str, kind: &str, name: &str, vars: &Vars) -> R<Option<Node>> {
    let lines = code_lines(text);
    let Some(at) = lines
        .iter()
        .position(|l| l.indent == 0 && l.text == format!("{kind}:"))
    else {
        return Ok(None);
    };
    let section = block_after(&lines, at, 0);
    let Some(first) = section.first() else {
        return Ok(None);
    };
    let e = first.indent;
    let path = format!("{kind}.{name}");
    let Some(pos) = section.iter().position(|l| {
        l.indent == e && (l.text == format!("{name}:") || l.text.starts_with(&format!("{name}: ")))
    }) else {
        return Ok(None);
    };
    let block = block_after(section, pos, e);
    check_block(block, &path)?;
    let rest = section[pos].text[name.len() + 1..].trim();
    if !rest.is_empty() {
        return scalar(rest, vars, &path).map(Some);
    }
    if block.is_empty() {
        return Ok(Some(Node::Empty));
    }
    let mut i = 0;
    let node = parse_map(block, &mut i, block[0].indent, e, &path, 0, vars)?;
    if i < block.len() {
        return refuse(Refusal::BadIndent, &path);
    }
    Ok(Some(node))
}

/// Everything one side contributes: the `db` node and the top-level entries it names.
struct Side1 {
    db: Node,
    entries: BTreeMap<(&'static str, String), Option<Node>>,
}

fn read_side(text: &str, env: &StackEnv, side: Side) -> Result<Side1, ComposeError> {
    let wrap = |(class, path): Refuse| ComposeError::Cannot { side, class, path };
    let vars = Vars::of(env);
    scan_file(text).map_err(wrap)?;
    let db = db_node(text, &vars).map_err(wrap)?;
    // An env_file is a path to a file that is not in this text: equal text would say nothing about the environment the
    // service gets, so it is not accepted (a refusal, not a comparison of paths).
    if let Node::Map(m) = &db
        && m.contains_key("env_file")
    {
        return Err(wrap((Refusal::EnvFile, "services.db.env_file".to_string())));
    }
    let mut entries = BTreeMap::new();
    for (kind, names) in names_used(&db) {
        for name in names {
            let node = top_entry(text, kind, &name, &vars).map_err(wrap)?;
            entries.insert((kind, name), node);
        }
    }
    Ok(Side1 { db, entries })
}

/// The path of the first difference (mappings by key, lists by position), or `None`.
fn diff(a: &Node, b: &Node, path: &str) -> Option<String> {
    match (a, b) {
        (Node::Empty, Node::Empty) => None,
        (Node::Scalar { .. }, Node::Scalar { .. }) => (a != b).then(|| path.to_string()),
        (Node::Map(x), Node::Map(y)) => {
            let keys: BTreeSet<&String> = x.keys().chain(y.keys()).collect();
            for k in keys {
                let here = format!("{path}.{k}");
                match (x.get(k), y.get(k)) {
                    (Some(p), Some(q)) => {
                        if let Some(d) = diff(p, q, &here) {
                            return Some(d);
                        }
                    }
                    _ => return Some(here),
                }
            }
            None
        }
        (Node::Seq(x), Node::Seq(y)) => {
            for (n, (p, q)) in x.iter().zip(y).enumerate() {
                if let Some(d) = diff(p, q, &format!("{path}[{n}]")) {
                    return Some(d);
                }
            }
            (x.len() != y.len()).then(|| format!("{path}[{}]", x.len().min(y.len())))
        }
        _ => Some(path.to_string()),
    }
}

/// `Ok(())` when the `db` service, and the top-level volumes, networks and secrets it names, are equal in both files
/// after `${VAR}` is replaced from each release's own `stack.env`. A shape the reader does not accept, in either file,
/// is `Cannot` (the current file is read first). The first difference is `Differs`, by key path.
pub fn compare_db_service(
    current: (&str, &StackEnv),
    candidate: (&str, &StackEnv),
) -> Result<(), ComposeError> {
    let a = read_side(current.0, current.1, Side::Current)?;
    let b = read_side(candidate.0, candidate.1, Side::Candidate)?;
    if let Some(path) = diff(&a.db, &b.db, "services.db") {
        return Err(ComposeError::Differs { path });
    }
    let keys: BTreeSet<&(&'static str, String)> =
        a.entries.keys().chain(b.entries.keys()).collect();
    for key in keys {
        let path = format!("{}.{}", key.0, key.1);
        let (x, y) = (
            a.entries.get(key).and_then(Option::as_ref),
            b.entries.get(key).and_then(Option::as_ref),
        );
        let found = match (x, y) {
            (None, None) => None,
            (Some(p), Some(q)) => diff(p, q, &path),
            _ => Some(path),
        };
        if let Some(path) = found {
            return Err(ComposeError::Differs { path });
        }
    }
    Ok(())
}
