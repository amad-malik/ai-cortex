//! A small JSON Schema checker for BOOT-60, over `tests/fixtures/boot-schemas.r2.6.json` (the
//! `Boot` subtree copied from the pinned S3 r2.6 JSON twin `a31fa244`). No crate is added (D94):
//! it implements exactly the keywords that subtree uses, and it **stops loudly** on any other
//! keyword, `format` or `pattern`, so it can never pass a document by ignoring a rule.

use serde_json::{Map, Value};

const FIXTURE: &str = include_str!("../fixtures/boot-schemas.r2.6.json");

/// The six schemas, by name, from the pinned S3 r2.6 twin (`Orientation.reading_path` is in it:
/// cox W1-20).
pub fn schemas() -> Map<String, Value> {
    let v: Value = serde_json::from_str(FIXTURE).expect("the schema fixture is JSON");
    assert!(
        v["source_sha256"]
            .as_str()
            .is_some_and(|s| s.starts_with("a31fa244")),
        "the fixture names the pinned r2.6 JSON twin"
    );
    v["schemas"].as_object().expect("schemas").clone()
}

/// r2.6's reading-path entry pattern (with `maxLength` and `x-max-bytes` 256 beside it).
const READING_PATH_ENTRY: &str = "^/[^\\x00-\\x1f\\x7f]*$";

/// Every violation of `doc` against schema `name`, as `path: what`. Empty means valid.
pub fn validate(name: &str, doc: &Value) -> Vec<String> {
    let all = schemas();
    let mut errs = Vec::new();
    check(&all, &all[name], doc, "$", &mut errs);
    errs
}

fn type_ok(t: &str, v: &Value) -> bool {
    match t {
        "object" => v.is_object(),
        "array" => v.is_array(),
        "string" => v.is_string(),
        "boolean" => v.is_boolean(),
        "null" => v.is_null(),
        "integer" => v.is_i64() || v.is_u64(),
        other => panic!("schema checker: unknown type {other}"),
    }
}

fn is_hex(s: &str, lower_only: bool) -> bool {
    s.bytes().all(|b| {
        b.is_ascii_digit()
            || (b'a'..=b'f').contains(&b)
            || (!lower_only && (b'A'..=b'F').contains(&b))
    })
}

fn uuid_ok(s: &str) -> bool {
    let parts: Vec<&str> = s.split('-').collect();
    parts.len() == 5
        && parts.iter().map(|p| p.len()).eq([8, 4, 4, 4, 12])
        && parts.iter().all(|p| is_hex(p, false))
}

/// RFC 3339 `date-time`: `YYYY-MM-DDTHH:MM:SS[.frac](Z|±HH:MM)`.
fn date_time_ok(s: &str) -> bool {
    let b = s.as_bytes();
    let digits =
        |r: std::ops::Range<usize>| r.clone().all(|i| b.get(i).is_some_and(u8::is_ascii_digit));
    if b.len() < 20
        || !digits(0..4)
        || b[4] != b'-'
        || !digits(5..7)
        || b[7] != b'-'
        || !digits(8..10)
    {
        return false;
    }
    if !(b[10] == b'T' || b[10] == b't')
        || !digits(11..13)
        || b[13] != b':'
        || !digits(14..16)
        || b[16] != b':'
        || !digits(17..19)
    {
        return false;
    }
    let mut i = 19;
    if b[i] == b'.' {
        i += 1;
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    match &b[i..] {
        [b'Z'] | [b'z'] => true,
        [s, h1, h2, b':', m1, m2] if (*s == b'+' || *s == b'-') => {
            [h1, h2, m1, m2].iter().all(|c| c.is_ascii_digit())
        }
        _ => false,
    }
}

/// The patterns the subtree uses, each as a predicate. Any other pattern stops the check.
fn pattern_ok(p: &str, s: &str) -> bool {
    let label = |first: fn(u8) -> bool, rest: fn(u8) -> bool| {
        let b = s.as_bytes();
        (1..=32).contains(&b.len()) && first(b[0]) && b[1..].iter().all(|c| rest(*c))
    };
    match p {
        "^[A-Za-z][A-Za-z0-9-]{0,31}$" => label(
            |c| c.is_ascii_alphabetic(),
            |c| c.is_ascii_alphanumeric() || c == b'-',
        ),
        "^[a-z][a-z0-9-]{0,31}$" => label(
            |c| c.is_ascii_lowercase(),
            |c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-',
        ),
        "^[0-9a-f]{64}$" => s.len() == 64 && is_hex(s, true),
        READING_PATH_ENTRY => {
            s.starts_with('/') && !s.chars().any(|c| (c as u32) < 0x20 || c == '\u{7f}')
        }
        other => panic!("schema checker: unknown pattern {other}"),
    }
}

fn check(all: &Map<String, Value>, schema: &Value, v: &Value, path: &str, errs: &mut Vec<String>) {
    let obj = schema.as_object().expect("a schema is an object");
    for key in obj.keys() {
        match key.as_str() {
            "type"
            | "additionalProperties"
            | "required"
            | "properties"
            | "$ref"
            | "oneOf"
            | "items"
            | "format"
            | "pattern"
            | "maxLength"
            | "enum"
            | "minItems"
            | "maxItems"
            | "minLength" => {}
            // Annotations, which constrain nothing.
            "description" | "x-label" => {}
            // S3's byte cap beside maxLength: checked as bytes.
            "x-max-bytes" => {}
            other => panic!("schema checker: unknown keyword {other} at {path}"),
        }
    }
    if let Some(r) = obj.get("$ref").and_then(Value::as_str) {
        let name = r
            .strip_prefix("#/components/schemas/")
            .unwrap_or_else(|| panic!("schema checker: unknown $ref {r}"));
        let target = all
            .get(name)
            .unwrap_or_else(|| panic!("schema checker: $ref {name} not in the fixture"));
        check(all, target, v, path, errs);
    }
    if let Some(alts) = obj.get("oneOf").and_then(Value::as_array) {
        let results: Vec<Vec<String>> = alts
            .iter()
            .map(|a| {
                let mut e = Vec::new();
                check(all, a, v, path, &mut e);
                e
            })
            .collect();
        let passing = results.iter().filter(|e| e.is_empty()).count();
        if passing != 1 {
            errs.push(format!("{path}: oneOf matched {passing} alternatives"));
            // Say why each alternative failed, so a refusal names its cause.
            errs.extend(results.into_iter().flatten());
        }
    }
    if let Some(t) = obj.get("type") {
        let ok = match t {
            Value::String(t) => type_ok(t, v),
            Value::Array(ts) => ts
                .iter()
                .any(|t| type_ok(t.as_str().expect("a type name"), v)),
            _ => panic!("schema checker: bad type at {path}"),
        };
        if !ok {
            errs.push(format!("{path}: not of type {t}"));
            return;
        }
    }
    if let Some(e) = obj.get("enum").and_then(Value::as_array)
        && !e.contains(v)
    {
        errs.push(format!("{path}: {v} not in {e:?}"));
    }
    if let Some(s) = v.as_str() {
        if let Some(f) = obj.get("format").and_then(Value::as_str) {
            let ok = match f {
                "uuid" => uuid_ok(s),
                "date-time" => date_time_ok(s),
                other => panic!("schema checker: unknown format {other}"),
            };
            if !ok {
                errs.push(format!("{path}: {s:?} is not a {f}"));
            }
        }
        if let Some(p) = obj.get("pattern").and_then(Value::as_str)
            && !pattern_ok(p, s)
        {
            errs.push(format!("{path}: {s:?} does not match {p}"));
        }
        if let Some(m) = obj.get("minLength").and_then(Value::as_u64)
            && (s.chars().count() as u64) < m
        {
            errs.push(format!("{path}: shorter than {m} characters"));
        }
        if let Some(m) = obj.get("maxLength").and_then(Value::as_u64)
            && s.chars().count() as u64 > m
        {
            errs.push(format!("{path}: longer than {m} characters"));
        }
        if let Some(m) = obj.get("x-max-bytes").and_then(Value::as_u64)
            && s.len() as u64 > m
        {
            errs.push(format!("{path}: longer than {m} bytes"));
        }
    }
    if let Some(arr) = v.as_array() {
        if let Some(m) = obj.get("minItems").and_then(Value::as_u64)
            && (arr.len() as u64) < m
        {
            errs.push(format!("{path}: fewer than {m} items"));
        }
        if let Some(m) = obj.get("maxItems").and_then(Value::as_u64)
            && (arr.len() as u64) > m
        {
            errs.push(format!("{path}: more than {m} items"));
        }
    }
    if let (Some(items), Some(arr)) = (obj.get("items"), v.as_array()) {
        for (i, x) in arr.iter().enumerate() {
            check(all, items, x, &format!("{path}[{i}]"), errs);
        }
    }
    if let Some(o) = v.as_object() {
        let props = obj.get("properties").and_then(Value::as_object);
        if let Some(req) = obj.get("required").and_then(Value::as_array) {
            for r in req {
                let r = r.as_str().expect("a required name");
                if !o.contains_key(r) {
                    errs.push(format!("{path}: missing required {r}"));
                }
            }
        }
        for (k, x) in o {
            match props.and_then(|p| p.get(k)) {
                Some(ps) => check(all, ps, x, &format!("{path}.{k}"), errs),
                None => match obj.get("additionalProperties") {
                    Some(Value::Bool(false)) => errs.push(format!("{path}: unknown property {k}")),
                    None | Some(Value::Bool(true)) => {}
                    Some(_) => panic!("schema checker: additionalProperties schema at {path}"),
                },
            }
        }
    }
}

/// The checker refuses what it must refuse (so a pass means something).
pub fn self_test() {
    assert!(uuid_ok("01930000-0000-7000-8000-000100000001"));
    assert!(!uuid_ok("01930000-0000-7000-8000-00010000000"));
    assert!(date_time_ok("2026-11-06T12:00:00.000000Z"));
    assert!(date_time_ok("2026-11-06T12:00:00+01:00"));
    assert!(!date_time_ok("2026-11-06 12:00:00Z"));
    assert!(pattern_ok("^[a-z][a-z0-9-]{0,31}$", "qa-lead"));
    assert!(!pattern_ok("^[a-z][a-z0-9-]{0,31}$", "QA"));
    assert!(!pattern_ok("^[0-9a-f]{64}$", &"A".repeat(64)));
    assert!(pattern_ok(READING_PATH_ENTRY, "/srv/a.md"));
    assert!(!pattern_ok(READING_PATH_ENTRY, "srv/a.md"));
    assert!(!pattern_ok(READING_PATH_ENTRY, "/a\tb"));
}
