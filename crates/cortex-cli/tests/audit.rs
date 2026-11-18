//! `cortex audit list` (piece 10, the audit group), the built binary against the std-only fake server with a cleared
//! environment: AU-01 to AU-20 of `mvp/tests/cortex-cli-audit.md` (AU-21 is the existing `n3_...` and `en_01_...` scans).
//! DB-free. RED: the group does not exist, so every row that runs the command fails with a usage error.
mod common;

use std::collections::BTreeMap;

use common::support::fake::{Fake, Recorded, Reply};
use common::{Run, cortex};
use serde_json::{Value, json};

const T: &str = "AUDIT-TOKEN-5c2e8a71b4d90f36";
const CANARY: &str = "CANARY-9b3e51d70a4c8f26";
const RID: &str = "0198d2c0-7000-7000-8000-000000000001";
/// W1-90 (cox 8 Nov 02:45Z): the capped listing is INCOMPLETE and says so; the exit status is 8.
const ALL_MORE: &str =
    "incomplete: 20 pages were printed and more records exist; continue with --after ";
const INCOMPLETE: i32 = 8;

fn env() -> [(&'static str, &'static str); 1] {
    [("CORTEX_TOKEN", T)]
}

fn run(url: &str, extra: &[&str]) -> Run {
    let mut a = vec!["audit", "list", "--server", url];
    a.extend_from_slice(extra);
    cortex(&a, &env())
}

/// Record `n`: every member present.
fn rec(n: u32) -> Value {
    json!({
        "id": format!("0198d2c0-7000-7000-8000-{n:012}"),
        "at": format!("2026-11-07T10:00:{n:02}Z"),
        "action": "worker.retired",
        "actor_kind": "worker",
        "actor_id": format!("0198d2c0-7000-7000-8000-{:012}", 1000 + n),
        "target": {"worker": format!("W{n}")},
        "outcome": "allowed",
        "request_id": RID,
        "details": {"role": "member"}
    })
}

/// What the md line of `rec(n)` must be.
fn line(n: u32) -> String {
    format!(
        "2026-11-07T10:00:{n:02}Z worker.retired worker:0198d2c0-7000-7000-8000-{:012} allowed worker=W{n} {RID} role=member",
        1000 + n
    )
}

fn page(items: &[Value], next: Option<&str>) -> String {
    json!({"items": items, "next": next}).to_string()
}

fn fake_pages(pages: &[(Vec<Value>, Option<&str>)]) -> Fake {
    let f = Fake::start();
    for (items, next) in pages {
        f.push(Reply::json(200, &page(items, *next)));
    }
    f
}

fn one_page_fake(items: &[Value], next: Option<&str>) -> Fake {
    fake_pages(&[(items.to_vec(), next)])
}

fn targets(f: &Fake) -> Vec<String> {
    f.requests().iter().map(|r| r.target.clone()).collect()
}

fn pct_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = std::str::from_utf8(&b[i + 1..i + 3]).unwrap();
            out.push(u8::from_str_radix(hex, 16).unwrap());
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap()
}

fn unsafe_oracle(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{2028}' | '\u{2029}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
                | '\u{61c}' | '\u{e0000}'..='\u{e007f}'
        )
}

/// What `safe_text(text, budget)` must give (written again here, not called).
fn shown(text: &str, budget: usize) -> String {
    let n = text.chars().count();
    let map = |c: char| if unsafe_oracle(c) { '?' } else { c };
    if n <= budget {
        text.chars().map(map).collect()
    } else {
        let mut s: String = text.chars().take(budget - 3).map(map).collect();
        s.push_str("...");
        s
    }
}

fn hostile(tag: &str) -> String {
    format!(
        "{tag}:A\x1b[31mB\rC\x07D\0E\x7fF\u{9b}G\u{2028}H\u{200e}I\u{202e}J\u{2066}K\u{61c}L\u{e0041}M\nN\tO"
    )
}

/// A one-pair JSON object whose key is not a literal.
fn kv(key: String, value: String) -> Value {
    let mut m = serde_json::Map::new();
    m.insert(key, Value::String(value));
    Value::Object(m)
}

fn clean(r: &Run) {
    assert!(
        !r.stdout.chars().any(|c| unsafe_oracle(c) && c != '\n'),
        "unsafe character in stdout: {:?}",
        r.stdout
    );
    assert!(
        !r.stderr.chars().any(|c| unsafe_oracle(c) && c != '\n'),
        "unsafe character in stderr: {:?}",
        r.stderr
    );
}

// ---- 2. request and arguments ------------------------------------------------------------------------------------------

#[test]
fn au_01_one_get_with_one_bearer_header_and_no_query() {
    let fake = one_page_fake(&[rec(1)], None);
    let r = run(&fake.url(), &[]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(fake.count(), 1);
    let q: Recorded = fake.requests().remove(0);
    assert_eq!(q.method, "GET");
    assert_eq!(
        q.target, "/v2/audit",
        "no query: the server's default 50 stands"
    );
    assert_eq!(
        q.header("authorization"),
        Some(format!("Bearer {T}").as_str())
    );
    assert_eq!(q.headers_named("authorization").len(), 1);
    assert_eq!(
        q.head.matches(T).count(),
        1,
        "the token is only in the Authorization header: {}",
        q.head
    );
    assert!(
        q.header("idempotency-key").is_none(),
        "a read carries no key"
    );
    assert!(q.body.is_empty());
    assert!(
        q.header("accept").is_some_and(|a| a.contains("json")),
        "{:?}",
        q.header("accept")
    );
}

#[test]
fn au_02_limit_is_one_to_two_hundred_and_a_usage_error_otherwise() {
    for good in ["1", "200"] {
        let fake = one_page_fake(&[], None);
        let r = run(&fake.url(), &["--limit", good]);
        assert_eq!(r.code, 0, "{good}: {}", r.stderr);
        assert_eq!(targets(&fake), vec![format!("/v2/audit?limit={good}")]);
    }
    for bad in [
        "0",
        "201",
        "-1",
        "abc",
        "",
        "1.5",
        "+5",
        "99999999999999999999",
        "\u{ff15}",
    ] {
        let fake = one_page_fake(&[], None);
        let r = run(&fake.url(), &["--limit", bad]);
        assert_eq!(r.code, 2, "{bad:?}: {}", r.stderr);
        assert!(r.stdout.is_empty(), "{bad:?}");
        assert_eq!(
            fake.connections(),
            0,
            "{bad:?}: a usage error sends nothing"
        );
    }
    let fake = one_page_fake(&[], None);
    run(&fake.url(), &[]);
    assert!(
        !targets(&fake)[0].contains("limit"),
        "no --limit, no limit sent"
    );
}

#[test]
fn au_03_after_is_printable_ascii_up_to_256_and_is_sent_encoded() {
    let good_cursors: Vec<String> = vec![
        "a+b/c=d e~".to_owned(),
        "%41".to_owned(),
        "x".to_owned(),
        "k".repeat(256),
        format!("{} {}", "a".repeat(127), "b".repeat(128)),
    ];
    for cursor in &good_cursors {
        let fake = one_page_fake(&[], None);
        let r = run(&fake.url(), &["--after", cursor]);
        assert_eq!(r.code, 0, "{cursor:?}: {}", r.stderr);
        let t = targets(&fake).remove(0);
        let value = t
            .strip_prefix("/v2/audit?after=")
            .unwrap_or_else(|| panic!("{t}"));
        assert_eq!(pct_decode(value), *cursor, "round trip of {cursor:?}: {t}");
        assert!(
            !value.contains(['+', ' ', '=', '/']),
            "an encoded value holds none of + space = /: {t}"
        );
    }
    for bad in [
        String::new(),
        "a".repeat(257),
        "a\tb".to_owned(),
        "a\nb".to_owned(),
        "a\u{7f}b".to_owned(),
        "\u{e9}".to_owned(),
        "\u{1b}".to_owned(),
    ] {
        let fake = one_page_fake(&[], None);
        let r = run(&fake.url(), &["--after", &bad]);
        assert_eq!(r.code, 2, "{bad:?}: {}", r.stderr);
        assert!(r.stdout.is_empty(), "{bad:?}");
        assert_eq!(fake.connections(), 0, "{bad:?}");
    }
    // both together: one query, `limit` then `after`, whatever the order given
    for args in [
        ["--limit", "5", "--after", "x"],
        ["--after", "x", "--limit", "5"],
    ] {
        let fake = one_page_fake(&[], None);
        let r = run(&fake.url(), &args);
        assert_eq!(r.code, 0, "{}", r.stderr);
        assert_eq!(targets(&fake), vec!["/v2/audit?limit=5&after=x"]);
    }
}

#[test]
fn au_04_global_flags_either_side_and_unknown_things_are_usage_errors() {
    let page_body = page(&[rec(1), rec(2)], Some("c1"));
    let make = || {
        let f = Fake::start();
        f.set_default(Reply::json(200, &page_body));
        f
    };
    let (a, b, c) = (make(), make(), make());
    let ra = cortex(
        &["audit", "list", "--format", "json", "--server", &a.url()],
        &env(),
    );
    let rb = cortex(
        &["--format", "json", "--server", &b.url(), "audit", "list"],
        &env(),
    );
    let rc = cortex(
        &["--server", &c.url(), "audit", "list", "--format", "json"],
        &env(),
    );
    assert_eq!(
        (ra.code, rb.code, rc.code),
        (0, 0, 0),
        "{} {} {}",
        ra.stderr,
        rb.stderr,
        rc.stderr
    );
    assert_eq!(ra.stdout, rb.stdout);
    assert_eq!(ra.stdout, rc.stdout);
    assert!(ra.stdout.trim_start().starts_with('{'), "{}", ra.stdout);
    for args in [
        vec!["audit"],
        vec!["audit", "verify"],
        vec!["audit", "list", "--action", "x"],
        vec!["audit", "list", "--since", "x"],
        vec!["audit", "list", "--format", "xml"],
        vec!["audit", "list", "extra"],
    ] {
        let fake = one_page_fake(&[], None);
        let url = fake.url();
        let mut full: Vec<&str> = args.clone();
        full.extend_from_slice(&["--server", &url]);
        let r = cortex(&full, &env());
        assert_eq!(r.code, 2, "{args:?}: {}", r.stderr);
        assert!(r.stdout.is_empty(), "{args:?}");
        assert_eq!(fake.connections(), 0, "{args:?}");
    }
}

// ---- 3. output, one page -----------------------------------------------------------------------------------------------

fn three_records() -> Vec<Value> {
    vec![
        json!({
            "id": "0198d2c0-7000-7000-8000-000000000011", "at": "2026-11-07T10:00:01Z", "action": "worker.retired",
            "actor_kind": "worker", "actor_id": "0198d2c0-7000-7000-8000-0000000000a1",
            "target": {"worker": "W1", "team": "T1"}, "outcome": "allowed", "request_id": "RID1",
            "details": {"role": "member", "from": "x"}
        }),
        json!({
            "id": "0198d2c0-7000-7000-8000-000000000012", "at": "2026-11-07T10:00:02Z", "action": "team.created",
            "actor_kind": "system", "outcome": "allowed"
        }),
        json!({
            "id": "0198d2c0-7000-7000-8000-000000000013", "at": "2026-11-07T10:00:03Z", "action": "token.issued",
            "actor_kind": "host", "actor_id": null, "target": {}, "outcome": "denied", "request_id": null,
            "details": {"n": 3, "ok": true, "l": [1, 2], "z": null, "o": {"a": 1}}
        }),
    ]
}

#[test]
fn au_05_md_is_one_line_per_record_with_seven_fixed_fields() {
    let fake = one_page_fake(&three_records(), Some("CURSOR-2"));
    let r = run(&fake.url(), &[]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(
        r.stdout,
        "2026-11-07T10:00:01Z worker.retired worker:0198d2c0-7000-7000-8000-0000000000a1 allowed team=T1,worker=W1 RID1 from=x,role=member\n\
         2026-11-07T10:00:02Z team.created system:- allowed - - -\n\
         2026-11-07T10:00:03Z token.issued host:- denied - - l=[1,2],n=3,o={\"a\":1},ok=true,z=null\n"
    );
    assert_eq!(
        r.stderr, "next: CURSOR-2\n",
        "the cursor is on stderr, stdout stays the records"
    );
    let last = one_page_fake(&three_records(), None);
    let r = run(&last.url(), &[]);
    assert_eq!(r.code, 0);
    assert!(
        r.stderr.is_empty(),
        "next is null: nothing on stderr: {:?}",
        r.stderr
    );
}

#[test]
fn au_06_an_empty_page_prints_nothing_in_md_and_the_object_in_json() {
    let fake = one_page_fake(&[], None);
    let md = run(&fake.url(), &[]);
    assert_eq!(
        (md.code, md.stdout.as_str(), md.stderr.as_str()),
        (0, "", "")
    );
    let fake = one_page_fake(&[], None);
    let js = run(&fake.url(), &["--format", "json"]);
    assert_eq!(js.code, 0, "{}", js.stderr);
    assert_eq!(js.stdout, "{\"items\":[],\"next\":null}\n");
    assert!(js.stderr.is_empty());
}

#[test]
fn au_07_records_print_in_the_order_received_ties_and_duplicates_kept() {
    let mut a = rec(5);
    let mut b = rec(4);
    a["at"] = json!("2026-11-07T10:00:09Z");
    b["at"] = json!("2026-11-07T10:00:09Z");
    // the tie is in the server's order (b's id sorts BEFORE a's here, the server sent a first); a duplicate follows
    let fake = one_page_fake(&[a.clone(), b.clone(), a.clone()], None);
    let r = run(&fake.url(), &[]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let got: Vec<&str> = r.stdout.lines().collect();
    assert_eq!(got.len(), 3, "{:?}", r.stdout);
    assert!(
        got[0].contains("worker=W5")
            && got[1].contains("worker=W4")
            && got[2].contains("worker=W5"),
        "{got:?}"
    );
    assert_eq!(got[0], got[2]);
}

#[test]
fn au_08_json_is_one_line_that_parses_to_the_served_page() {
    let mut items = three_records();
    items[0]["extra_member"] = json!({"k": [1, "two"]});
    let mut body = json!({"items": items, "next": "CURSOR-9", "page_extra": 7});
    let fake = Fake::start();
    fake.set_default(Reply::json(200, &body.to_string()));
    let r = run(&fake.url(), &["--format", "json"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert!(
        r.stdout.ends_with('\n') && r.stdout.matches('\n').count() == 1,
        "one line: {:?}",
        r.stdout
    );
    let parsed: Value = serde_json::from_str(r.stdout.trim_end()).expect("the line is JSON");
    body["page_extra"] = json!(7);
    assert_eq!(
        parsed, body,
        "the same members and values, unknown ones kept"
    );
    assert!(
        r.stderr.is_empty(),
        "the cursor is in the object: {:?}",
        r.stderr
    );
}

#[test]
fn au_09_a_hostile_page_is_safe_bounded_and_one_line_per_record() {
    // (a) every printed string is hostile and short enough: each unsafe character is `?`, one line
    let h = |t: &str| hostile(t);
    let rec_h = json!({
        "id": RID, "at": h("at"), "action": h("ac"), "actor_kind": h("ak"), "actor_id": h("ai"),
        "target": kv(h("tk"), h("tv")), "outcome": h("oc"), "request_id": h("ri"), "details": kv(h("dk"), h("dv"))
    });
    let next = h("nx");
    let fake = one_page_fake(std::slice::from_ref(&rec_h), Some(next.as_str()));
    let r = run(&fake.url(), &[]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let want = format!(
        "{} {} {}:{} {} {}={} {} {}={}\n",
        shown(&h("at"), 64),
        shown(&h("ac"), 64),
        shown(&h("ak"), 64),
        shown(&h("ai"), 64),
        shown(&h("oc"), 64),
        shown(&h("tk"), 64),
        shown(&h("tv"), 128),
        shown(&h("ri"), 64),
        shown(&h("dk"), 64),
        shown(&h("dv"), 128),
    );
    assert_eq!(r.stdout, want);
    assert_eq!(
        r.stdout.lines().count(),
        1,
        "an LF inside a value makes no second line"
    );
    assert_eq!(r.stderr, format!("next: {}\n", shown(&next, 256)));
    clean(&r);
    // json: no unsafe character in the line, and it parses to the SAME strings
    let fake = one_page_fake(std::slice::from_ref(&rec_h), Some(next.as_str()));
    let j = run(&fake.url(), &["--format", "json"]);
    assert_eq!(j.code, 0, "{}", j.stderr);
    assert_eq!(j.stdout.matches('\n').count(), 1);
    assert!(
        !j.stdout.chars().any(|c| unsafe_oracle(c) && c != '\n'),
        "{:?}",
        j.stdout
    );
    let parsed: Value = serde_json::from_str(j.stdout.trim_end()).expect("JSON");
    assert_eq!(parsed, json!({"items": [rec_h], "next": next}));
    clean(&j);
}

#[test]
fn au_09_values_over_their_budgets_are_cut_with_the_marker() {
    let long = |c: char| c.to_string().repeat(300);
    let rec_l = json!({
        "id": RID, "at": long('a'), "action": long('b'), "actor_kind": long('c'), "actor_id": long('d'),
        "target": kv(long('e'), long('f')), "outcome": long('g'), "request_id": long('h'),
        "details": kv(long('i'), long('j'))
    });
    let fake = one_page_fake(&[rec_l], None);
    let r = run(&fake.url(), &[]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    let want = format!(
        "{} {} {}:{} {} {}={} {} {}={}\n",
        shown(&long('a'), 64),
        shown(&long('b'), 64),
        shown(&long('c'), 64),
        shown(&long('d'), 64),
        shown(&long('g'), 64),
        shown(&long('e'), 64),
        shown(&long('f'), 128),
        shown(&long('h'), 64),
        shown(&long('i'), 64),
        shown(&long('j'), 128),
    );
    assert_eq!(r.stdout, want);
    assert!(r.stdout.contains("..."));
}

// ---- 4. --all ----------------------------------------------------------------------------------------------------------

#[test]
fn au_10_all_follows_next_until_null_in_md() {
    let fake = fake_pages(&[
        (vec![rec(1), rec(2)], Some("c1")),
        (vec![rec(3)], Some("c2")),
        (vec![rec(4)], None),
    ]);
    let r = run(&fake.url(), &["--all", "--limit", "3"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(
        targets(&fake),
        vec![
            "/v2/audit?limit=3",
            "/v2/audit?limit=3&after=c1",
            "/v2/audit?limit=3&after=c2"
        ]
    );
    assert_eq!(
        r.stdout,
        format!("{}\n{}\n{}\n{}\n", line(1), line(2), line(3), line(4))
    );
    assert!(r.stderr.is_empty(), "{:?}", r.stderr);
    // --all --after X starts at X, without a limit sends none
    let fake = fake_pages(&[(vec![rec(1)], Some("c9")), (vec![rec(2)], None)]);
    let r = run(&fake.url(), &["--all", "--after", "X"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(
        targets(&fake),
        vec!["/v2/audit?after=X", "/v2/audit?after=c9"]
    );
}

#[test]
fn au_11_all_in_json_is_one_object_with_every_record_and_next_null() {
    let fake = fake_pages(&[(vec![rec(1), rec(2)], Some("c1")), (vec![rec(3)], None)]);
    let r = run(&fake.url(), &["--all", "--format", "json"]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stdout.matches('\n').count(), 1);
    let parsed: Value = serde_json::from_str(r.stdout.trim_end()).expect("JSON");
    assert_eq!(
        parsed,
        json!({"items": [rec(1), rec(2), rec(3)], "next": null})
    );
    assert!(r.stderr.is_empty());
}

fn pages_with_cursors(n: u32, last_next_null: bool) -> Vec<(Vec<Value>, Option<String>)> {
    (1..=n)
        .map(|i| {
            let next = if last_next_null && i == n {
                None
            } else {
                Some(format!("p{i}"))
            };
            (vec![rec(i)], next)
        })
        .collect()
}

fn fake_owned(pages: &[(Vec<Value>, Option<String>)]) -> Fake {
    let f = Fake::start();
    for (items, next) in pages {
        f.push(Reply::json(200, &page(items, next.as_deref())));
    }
    f
}

#[test]
fn au_12_the_cap_is_twenty_pages_then_one_stderr_line_and_exit_eight_incomplete() {
    let fake = fake_owned(&pages_with_cursors(25, false));
    let r = run(&fake.url(), &["--all"]);
    assert_eq!(fake.count(), 20, "exactly 20 requests: not 21, not 19");
    assert_eq!(r.code, INCOMPLETE, "{}", r.stderr);
    assert_eq!(r.stdout.lines().count(), 20);
    assert_eq!(
        r.stderr,
        format!("{ALL_MORE}p20\n"),
        "ONE line: more exist, with the last cursor"
    );
    // json: next is the last cursor, not null (null would claim completeness)
    let fake = fake_owned(&pages_with_cursors(25, false));
    let j = run(&fake.url(), &["--all", "--format", "json"]);
    assert_eq!(fake.count(), 20);
    assert_eq!(j.code, INCOMPLETE, "{}", j.stderr);
    let parsed: Value = serde_json::from_str(j.stdout.trim_end()).expect("JSON");
    assert_eq!(parsed["next"], json!("p20"));
    assert_eq!(parsed["items"].as_array().map(Vec::len), Some(20));
    assert_eq!(j.stderr, format!("{ALL_MORE}p20\n"));
    // exactly 20 pages, the last next null: no such line
    let fake = fake_owned(&pages_with_cursors(20, true));
    let r = run(&fake.url(), &["--all"]);
    assert_eq!(fake.count(), 20);
    assert_eq!((r.code, r.stderr.as_str()), (0, ""), "{}", r.stderr);
    assert_eq!(r.stdout.lines().count(), 20);
}

#[test]
fn au_13_a_hostile_cursor_in_all_stops_the_traversal() {
    let too_long = "n".repeat(257);
    for bad in [too_long.as_str(), "a\u{7}b", "\u{e9}", "\t"] {
        let fake = fake_pages(&[(vec![rec(1)], Some(bad)), (vec![rec(2)], None)]);
        let r = run(&fake.url(), &["--all"]);
        assert_eq!(r.code, 1, "{bad:?}: {}", r.stderr);
        assert!(
            r.stdout.is_empty(),
            "{bad:?}: nothing half-printed: {:?}",
            r.stdout
        );
        assert_eq!(r.stderr.lines().count(), 1, "{bad:?}: {:?}", r.stderr);
        assert_eq!(
            fake.count(),
            1,
            "{bad:?}: no request after the page that gave the bad cursor"
        );
        clean(&r);
    }
    // a cursor equal to the one just sent
    let fake = fake_pages(&[
        (vec![rec(1)], Some("x")),
        (vec![rec(2)], Some("x")),
        (vec![rec(3)], None),
    ]);
    let r = run(&fake.url(), &["--all"]);
    assert_eq!(r.code, 1, "{}", r.stderr);
    assert!(r.stdout.is_empty());
    assert_eq!(
        fake.count(),
        2,
        "stopped at the page whose next did not advance"
    );
    // the same odd next on a single page is only displayed
    let fake = one_page_fake(&[rec(1)], Some("a\u{7}b"));
    let r = run(&fake.url(), &[]);
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(r.stderr, "next: a?b\n");
}

#[test]
fn au_14_a_failure_on_a_later_page_prints_nothing_at_all() {
    let scripts: Vec<(&str, Reply, i32)> = vec![
        ("503", Reply::problem(503, "unavailable"), 7),
        ("401", Reply::problem(401, "token_revoked"), 3),
        (
            "not the contract",
            Reply::json(200, "{\"items\":5,\"next\":null}"),
            1,
        ),
        ("closed", Reply::CloseAfterRequest, 1),
    ];
    for (what, second, exit) in scripts {
        for format in ["md", "json"] {
            let fake = Fake::start();
            fake.push(Reply::json(200, &page(&[rec(1), rec(2)], Some("c1"))));
            fake.push(second.clone());
            let r = run(&fake.url(), &["--all", "--format", format]);
            assert_eq!(r.code, exit, "{what} {format}: {}", r.stderr);
            assert!(
                r.stdout.is_empty(),
                "{what} {format}: earlier pages are not printed: {:?}",
                r.stdout
            );
            assert_eq!(
                r.stderr.lines().count(),
                1,
                "{what} {format}: {:?}",
                r.stderr
            );
            assert_eq!(fake.count(), 2, "{what} {format}");
        }
    }
}

// ---- 5. decode, errors, token ------------------------------------------------------------------------------------------

#[test]
fn au_15_a_page_that_is_not_the_contract_is_a_decode_error_naming_the_member() {
    let ok = rec(1);
    let without = |k: &str| {
        let mut r = ok.clone();
        r.as_object_mut().unwrap().remove(k);
        r
    };
    let with = |k: &str, v: Value| {
        let mut r = ok.clone();
        r[k] = v;
        r
    };
    let mut cases: Vec<(String, String, &str)> = vec![
        ("[]".into(), "page".into(), "array"),
        ("5".into(), "page".into(), "number"),
        ("{\"next\":null}".into(), "items".into(), "items missing"),
        (
            "{\"items\":{},\"next\":null}".into(),
            "items".into(),
            "items an object",
        ),
        ("{\"items\":[]}".into(), "next".into(), "next missing"),
        (
            "{\"items\":[],\"next\":5}".into(),
            "next".into(),
            "next a number",
        ),
        (
            json!({"items": [5], "next": null}).to_string(),
            "items[0]".into(),
            "item a number",
        ),
    ];
    for k in ["id", "at", "action", "actor_kind", "outcome"] {
        cases.push((
            page(&[without(k)], None),
            format!("items[0].{k}"),
            "required missing",
        ));
        cases.push((
            page(&[with(k, json!(7))], None),
            format!("items[0].{k}"),
            "required a number",
        ));
    }
    for (k, v) in [
        ("actor_id", json!(5)),
        ("request_id", json!(5)),
        ("target", json!("x")),
        ("details", json!([])),
    ] {
        cases.push((
            page(&[with(k, v)], None),
            format!("items[0].{k}"),
            "optional of the wrong type",
        ));
    }
    for (body, member, what) in cases {
        let fake = Fake::start();
        fake.set_default(Reply::json(200, &body));
        let r = run(&fake.url(), &[]);
        assert_eq!(r.code, 1, "{what}: {body}: {}", r.stderr);
        assert!(r.stdout.is_empty(), "{what}");
        assert_eq!(r.stderr.lines().count(), 1, "{what}: {:?}", r.stderr);
        assert!(
            r.stderr.contains(&member),
            "{what}: the text names {member}: {:?}",
            r.stderr
        );
        assert!(!r.stderr.contains("panicked"), "{what}");
    }
}

#[test]
fn au_16_every_failure_has_its_exit_code_one_diagnostic_and_an_empty_stdout() {
    for (status, code, exit) in [
        (401, "token_revoked", 3),
        (403, "missing_scope", 3),
        (422, "invalid_request", 6),
        (503, "unavailable", 7),
    ] {
        let fake = Fake::start();
        fake.set_default(Reply::problem(status, code));
        let md = run(&fake.url(), &[]);
        assert_eq!(md.code, exit, "{status}: {}", md.stderr);
        assert!(md.stdout.is_empty(), "{status}");
        assert_eq!(md.stderr.lines().count(), 1, "{status}: {:?}", md.stderr);
        let js = run(&fake.url(), &["--format", "json"]);
        assert_eq!(js.code, exit, "{status}: {}", js.stderr);
        assert!(js.stdout.is_empty(), "{status}");
        assert_eq!(js.stderr.lines().count(), 1);
        let e: Value = serde_json::from_str(js.stderr.trim_end()).expect("the error line is JSON");
        assert_eq!(
            (e["error"]["code"].as_str(), e["error"]["exit"].as_u64()),
            (Some(code), Some(u64::from(exit as u32)))
        );
    }
    // a hostile 403 goes through the existing display boundary
    let fake = Fake::start();
    fake.set_default(Reply::problem_with(
        403,
        "missing_scope",
        &hostile("T"),
        Some(&hostile("D")),
        &hostile("F"),
    ));
    let r = run(&fake.url(), &[]);
    assert_eq!(r.code, 3, "{}", r.stderr);
    assert_eq!(r.stderr.lines().count(), 1);
    clean(&r);
    // 429 on every attempt: the client's bounded retry (4 attempts, 3 waits of 1 s), then 7
    let busy = Fake::start();
    busy.set_default(Reply::problem(429, "busy").header("Retry-After", "1"));
    let r = run(&busy.url(), &[]);
    assert_eq!(r.code, 7, "{}", r.stderr);
    assert!(r.stdout.is_empty());
    assert_eq!(busy.count(), 4);
    // a refused connection: one line naming the address, exit 1
    let down = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        format!("http://127.0.0.1:{}", l.local_addr().unwrap().port())
    };
    let r = run(&down, &[]);
    assert_eq!(r.code, 1, "{}", r.stderr);
    assert_eq!(r.stderr.lines().count(), 1);
    assert!(
        r.stderr.contains(down.trim_start_matches("http://")),
        "{:?}",
        r.stderr
    );
}

#[test]
fn au_17_the_token_sources_are_the_ones_of_w1_43() {
    // nothing set: exit 1, the text names the three sources, no connection
    let fake = one_page_fake(&[], None);
    let url = fake.url();
    let r = cortex(&["audit", "list", "--server", &url], &[]);
    assert_eq!(r.code, 1, "{}", r.stderr);
    assert!(r.stdout.is_empty());
    for source in ["--token-file", "CORTEX_TOKEN", "CORTEX_TOKEN_FILE"] {
        assert!(r.stderr.contains(source), "names {source}: {:?}", r.stderr);
    }
    assert_eq!(fake.connections(), 0);
    // --token-file is used
    let dir = common::support::dir("au17");
    let good = common::support::write(&dir, "good", b"FILE-TOKEN-77aa\n", 0o600);
    let good_s = good.display().to_string();
    let fake = one_page_fake(&[], None);
    let url = fake.url();
    let r = cortex(
        &["audit", "list", "--server", &url, "--token-file", &good_s],
        &[],
    );
    assert_eq!(r.code, 0, "{}", r.stderr);
    assert_eq!(
        fake.requests()[0].header("authorization"),
        Some("Bearer FILE-TOKEN-77aa")
    );
    // a 0644 file: refused before any request
    let bad = common::support::write(&dir, "bad", b"FILE-TOKEN-77aa\n", 0o644);
    let bad_s = bad.display().to_string();
    let fake = one_page_fake(&[], None);
    let url = fake.url();
    let r = cortex(
        &["audit", "list", "--server", &url, "--token-file", &bad_s],
        &[],
    );
    assert_eq!(r.code, 1, "{}", r.stderr);
    assert!(r.stderr.contains("644"), "{:?}", r.stderr);
    assert_eq!(fake.connections(), 0);
    // CORTEX_TOKEN set but empty: no fall-through to CORTEX_TOKEN_FILE
    let fake = one_page_fake(&[], None);
    let url = fake.url();
    let r = cortex(
        &["audit", "list", "--server", &url],
        &[("CORTEX_TOKEN", ""), ("CORTEX_TOKEN_FILE", good_s.as_str())],
    );
    assert_eq!(r.code, 1, "{}", r.stderr);
    assert_eq!(fake.connections(), 0);
}

fn canary_scenarios() -> Vec<(&'static str, Fake, Vec<&'static str>)> {
    vec![
        (
            "one page md",
            one_page_fake(&three_records(), Some("c")),
            vec![],
        ),
        (
            "one page json",
            one_page_fake(&three_records(), Some("c")),
            vec!["--format", "json"],
        ),
        (
            "--all",
            fake_pages(&[(vec![rec(1)], Some("c1")), (vec![rec(2)], None)]),
            vec!["--all"],
        ),
        (
            "--all, 20 pages",
            fake_owned(&pages_with_cursors(25, false)),
            vec!["--all"],
        ),
        (
            "401",
            {
                let f = Fake::start();
                f.set_default(Reply::problem_with(
                    401,
                    "token_revoked",
                    "t",
                    Some(CANARY),
                    "f",
                ));
                f
            },
            vec![],
        ),
        (
            "403 json",
            {
                let f = Fake::start();
                f.set_default(Reply::problem(403, "missing_scope"));
                f
            },
            vec!["--format", "json"],
        ),
        (
            "422",
            {
                let f = Fake::start();
                f.set_default(Reply::problem(422, "invalid_request"));
                f
            },
            vec!["--after", "x"],
        ),
        (
            "503",
            {
                let f = Fake::start();
                f.set_default(Reply::problem(503, "unavailable"));
                f
            },
            vec![],
        ),
        (
            "usage error",
            one_page_fake(&[], None),
            vec!["--limit", "0"],
        ),
    ]
}

#[test]
fn au_18_a_canary_in_every_token_source_is_never_printed_or_sent_by_audit_list() {
    let dir = common::support::dir("au18");
    let file = common::support::write(&dir, "tok", format!("{CANARY}\n").as_bytes(), 0o600);
    let file_s = file.display().to_string();
    for source in ["env", "file"] {
        for (name, fake, extra) in &canary_scenarios() {
            let url = fake.url();
            let before = fake.count();
            let mut args: Vec<String> = vec![
                "audit".into(),
                "list".into(),
                "--server".into(),
                url.clone(),
            ];
            args.extend(extra.iter().map(|s| s.to_string()));
            let env: Vec<(&str, &str)> = if source == "env" {
                vec![("CORTEX_TOKEN", CANARY)]
            } else {
                args.push("--token-file".into());
                args.push(file_s.clone());
                vec![]
            };
            let a: Vec<&str> = args.iter().map(String::as_str).collect();
            let r = cortex(&a, &env);
            assert!(
                !r.stdout.contains(CANARY) && !r.stderr.contains(CANARY),
                "{source} {name}: stdout {:?} stderr {:?}",
                r.stdout,
                r.stderr
            );
            assert!(
                !a.iter().any(|x| x.contains(CANARY)),
                "{source} {name}: argv"
            );
            // not vacuous: every scenario but the usage error really reached the fake (a command that does
            // not exist sends nothing, and an empty log would pass every check below)
            let sent = fake.count() - before;
            if *name == "usage error" {
                assert_eq!(sent, 0, "{source} {name}: a usage error sends nothing");
            } else {
                assert!(sent >= 1, "{source} {name}: the command reached the server");
            }
            for q in fake.requests().iter().skip(before) {
                assert!(
                    !q.target.contains(CANARY),
                    "{source} {name}: path or query: {}",
                    q.target
                );
                assert_eq!(
                    q.head.matches(CANARY).count(),
                    1,
                    "{source} {name}: the canary is only in the Authorization header: {}",
                    q.head
                );
                assert_eq!(
                    q.header("authorization"),
                    Some(format!("Bearer {CANARY}").as_str())
                );
            }
        }
    }
}

// ---- 6. registry, coverage, help ---------------------------------------------------------------------------------------

#[test]
fn au_19_the_group_is_audit_owned_by_sophi_and_implements_exactly_the_twins_string() {
    use cortex_cli::coverage::NOT_YET;
    use cortex_cli::{groups, registry::Registry};
    let registry = Registry::new(groups::all()).expect("the registry builds with the audit group");
    let group = registry
        .groups()
        .iter()
        .find(|g| g.name == "audit")
        .expect("a group named audit");
    assert_eq!(group.owner, "sophi@helix");
    assert_eq!(group.implements, &["cortex audit list"]);
    assert!(
        !NOT_YET.iter().any(|(c, _)| *c == "cortex audit list"),
        "CV-03: the NOT_YET row is gone in the same commit"
    );
    let by_owner: BTreeMap<&str, usize> = NOT_YET.iter().fold(BTreeMap::new(), |mut m, (_, o)| {
        *m.entry(*o).or_default() += 1;
        m
    });
    assert_eq!(NOT_YET.len(), 28, "the other 28 rows are unchanged");
    assert_eq!(
        by_owner.get("marv@helix"),
        None,
        "marv's only row was this one"
    );
}

#[test]
fn au_20_the_help_names_the_command_the_bounds_and_that_the_log_is_plain() {
    let top = cortex(&["--help"], &[]);
    assert_eq!(top.code, 0, "{}", top.stderr);
    assert!(top.stdout.contains("audit"), "{}", top.stdout);
    let mid = cortex(&["audit", "--help"], &[]);
    assert_eq!(mid.code, 0, "{}", mid.stderr);
    assert!(mid.stdout.contains("list"), "{}", mid.stdout);
    let help = cortex(&["audit", "list", "--help"], &[]);
    assert_eq!(help.code, 0, "{}", help.stderr);
    for want in [
        "--limit",
        "--after",
        "--all",
        "200",
        "256",
        "20 pages",
        "incomplete",
        "exits 8",
        "no tamper evidence",
    ] {
        assert!(
            help.stdout.contains(want),
            "the help names {want:?}: {}",
            help.stdout
        );
    }
    assert!(help.stdout.contains("--token-file"));
    for r in [&top, &mid, &help] {
        assert!(!r.stdout.contains('\u{1b}') && !r.stderr.contains('\u{1b}'));
        assert!(
            !r.stdout.contains("--token ") && !r.stdout.contains("--token="),
            "never a --token flag"
        );
    }
}
