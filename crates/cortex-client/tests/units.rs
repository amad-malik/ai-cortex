//! IK-01, IK-04, RT-02, EX-01, PR-09: the pure parts (no socket). PR-09 and EX-01 read the pinned S3 twin at test time.
mod support;

use cortex_client::exit::{exit_code, exit_for_status};
use cortex_client::{
    ClientError, IdempotencyKey, KeyError, Problem, ProblemKind, RetryPolicy, TokenError,
    TransportKind,
};
use std::time::Duration;

#[test]
fn ik_01_generated_keys_are_uuid_v7_distinct_and_time_ordered() {
    let keys: Vec<String> = (0..1000)
        .map(|_| IdempotencyKey::generate().as_str().to_owned())
        .collect();
    for k in &keys {
        assert_eq!(k.len(), 36, "{k}");
        let b = k.as_bytes();
        assert_eq!(b[14], b'7', "version nibble: {k}");
        assert!(matches!(b[19], b'8' | b'9' | b'a' | b'b'), "variant: {k}");
        assert!([8, 13, 18, 23].iter().all(|&i| b[i] == b'-'), "{k}");
        assert!(
            IdempotencyKey::parse(k).is_ok(),
            "a generated key passes the chain's rule"
        );
    }
    let mut sorted = keys.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), 1000, "1000 distinct keys");
    for w in keys.windows(2) {
        assert!(
            w[0][..13] <= w[1][..13],
            "the 48-bit time prefix never decreases: {} then {}",
            w[0],
            w[1]
        );
    }
}

#[test]
fn ik_04_a_caller_supplied_key_is_validated() {
    let ok = |s: &str| IdempotencyKey::parse(s).map(|k| k.as_str().to_owned());
    assert_eq!(ok("abcdefg"), Err(KeyError::Length), "7 refused");
    assert!(ok("abcdefgh").is_ok(), "8 accepted");
    assert!(ok(&"k".repeat(128)).is_ok(), "128 accepted");
    assert_eq!(ok(&"k".repeat(129)), Err(KeyError::Length), "129 refused");
    assert_eq!(ok("abcdefg\x1f"), Err(KeyError::Byte), "0x1F refused");
    assert!(ok("abc defg").is_ok(), "0x20 accepted");
    assert!(ok("abcdefg~").is_ok(), "0x7E accepted");
    assert_eq!(ok("abcdefg\x7f"), Err(KeyError::Byte), "0x7F refused");
    assert_eq!(
        ok("abcdef\u{e9}h"),
        Err(KeyError::Byte),
        "non-ASCII refused"
    );
    assert_eq!(ok(""), Err(KeyError::Length));
}

#[test]
fn rt_02_the_retry_after_table() {
    let p = RetryPolicy::default();
    let s = Duration::from_secs;
    let table: [(Option<&str>, Duration); 17] = [
        (None, s(1)),
        (Some("1"), s(1)),
        (Some("0"), s(1)),
        (Some("3"), s(3)),
        (Some("5"), s(5)),
        (Some("6"), s(5)),
        (Some("120"), s(5)),
        (Some("Wed, 21 Oct 2026 07:28:00 GMT"), s(1)),
        (Some("-1"), s(1)),
        (Some("+3"), s(1)),
        (Some("abc"), s(1)),
        (Some("1.5"), s(1)),
        (Some(""), s(1)),
        (Some("18446744073709551616"), s(1)),
        (Some("00003"), s(3)),
        (Some("3 "), s(1)),
        (Some("\u{661}"), s(1)),
    ];
    for (h, want) in table {
        assert_eq!(p.wait_for(h), want, "Retry-After {h:?}");
    }
    let tight = RetryPolicy {
        max_wait_each: s(2),
        ..RetryPolicy::default()
    };
    assert_eq!(tight.wait_for(Some("3")), s(2));
    assert_eq!(RetryPolicy::default().max_attempts, 4);
    assert_eq!(RetryPolicy::default().max_total_wait, s(10));
}

#[test]
fn ex_01_every_status_the_twin_gives_has_its_exit_code() {
    let expected: [(u16, u8); 15] = [
        (200, 0),
        (201, 0),
        (400, 6),
        (401, 3),
        (403, 3),
        (404, 4),
        (405, 6),
        (409, 5),
        (413, 6),
        (415, 6),
        (422, 6),
        (428, 6),
        (429, 7),
        (500, 1),
        (503, 7),
    ];
    for (status, code) in expected {
        assert_eq!(exit_for_status(status), code, "HTTP {status}");
    }
    assert_eq!(exit_for_status(418), 1, "a status the twin does not list");
    assert_eq!(exit_for_status(204), 0);

    // drift: every status the pinned twin gives must be in the table above
    let twin = support::twin();
    let mut seen = std::collections::BTreeSet::new();
    for item in twin["paths"].as_object().unwrap().values() {
        for op in item.as_object().unwrap().values() {
            if let Some(r) = op.get("responses").and_then(|r| r.as_object()) {
                seen.extend(r.keys().filter_map(|k| k.parse::<u16>().ok()));
            }
        }
    }
    for group in ["platform_and_chain"] {
        for (_, v) in twin["x-error-codes"][group].as_object().unwrap() {
            match v {
                serde_json::Value::Number(n) => {
                    seen.insert(n.as_u64().unwrap() as u16);
                }
                serde_json::Value::Array(a) => {
                    seen.extend(a.iter().map(|n| n.as_u64().unwrap() as u16));
                }
                other => panic!("unexpected status shape {other}"),
            }
        }
    }
    let known: std::collections::BTreeSet<u16> = expected.iter().map(|(s, _)| *s).collect();
    let missing: Vec<_> = seen.difference(&known).collect();
    assert!(
        missing.is_empty(),
        "the twin gives statuses this table does not assert: {missing:?}"
    );
}

#[test]
fn ex_01_exit_code_of_each_failure() {
    let problem = |status| {
        ClientError::Problem(Box::new(Problem {
            status,
            body_status: None,
            code: "x".into(),
            title: "t".into(),
            detail: None,
            fix: "f".into(),
            request_id: "r".into(),
            scope: None,
            token_expires: None,
            token_renew_due: false,
        }))
    };
    for (status, code) in [
        (401, 3),
        (403, 3),
        (404, 4),
        (409, 5),
        (422, 6),
        (428, 6),
        (503, 7),
        (500, 1),
    ] {
        assert_eq!(exit_code(&problem(status)), code, "Problem {status}");
    }
    assert_eq!(
        exit_code(&ClientError::Http {
            status: 404,
            snippet: String::new(),
            request_id: None
        }),
        4
    );
    assert_eq!(
        exit_code(&ClientError::Http {
            status: 502,
            snippet: String::new(),
            request_id: None
        }),
        1
    );
    assert_eq!(
        exit_code(&ClientError::Busy {
            attempts: 4,
            retry_after_secs: 1,
            last: None
        }),
        7
    );
    assert_eq!(
        exit_code(&ClientError::Transport {
            kind: TransportKind::Refused,
            addr: "a".into(),
            key: None
        }),
        1
    );
    assert_eq!(exit_code(&ClientError::Token(TokenError::NoSource)), 1);
    assert_eq!(
        exit_code(&ClientError::UnexpectedRedirect { status: 302 }),
        1
    );
    assert_eq!(
        exit_code(&ClientError::Decode {
            what: "x".into(),
            request_id: None
        }),
        1
    );
    assert_eq!(
        exit_code(&ClientError::RequestTooLarge { size: 2, cap: 1 }),
        1
    );
    assert_eq!(exit_code(&ClientError::ResponseTooLarge { cap: 1 }), 1);
}

#[test]
fn pr_09_the_platform_codes_of_the_pinned_twin_are_all_known() {
    let twin = support::twin();
    let codes = twin["x-error-codes"]["platform_and_chain"]
        .as_object()
        .unwrap();
    assert_eq!(
        codes.len(),
        20,
        "the twin gives 20 platform and chain codes (r2.7)"
    );
    for (code, status) in codes {
        let kind = ProblemKind::from_code(code);
        assert!(
            !matches!(kind, ProblemKind::Other(_)),
            "{code} is not a known ProblemKind"
        );
        let mut want: Vec<u16> = match status {
            serde_json::Value::Number(n) => vec![n.as_u64().unwrap() as u16],
            serde_json::Value::Array(a) => a.iter().map(|n| n.as_u64().unwrap() as u16).collect(),
            other => panic!("{other}"),
        };
        want.sort_unstable();
        let mut got = kind.statuses().to_vec();
        got.sort_unstable();
        assert_eq!(got, want, "statuses of {code}");
    }
    assert_eq!(
        ProblemKind::from_code("some_future_code"),
        ProblemKind::Other("some_future_code".into())
    );
    assert!(
        ProblemKind::from_code("some_future_code")
            .statuses()
            .is_empty()
    );
}

/// W1-90 (cox 8 Nov 02:45Z): the ONE exit table gains `8 = incomplete` (a capped `cortex audit list --all`: the records are
/// printed, the list is not complete). The constant is pinned in the source text (a RED stub would change source), and no HTTP
/// status the table maps and no other constant uses 8.
#[test]
fn ex_06_the_incomplete_status_is_eight_and_nothing_else_uses_it() {
    let src = include_str!("../src/exit.rs");
    assert!(
        src.contains("pub const INCOMPLETE: u8 = 8;"),
        "exit.rs names INCOMPLETE as 8"
    );
    assert_eq!(
        src.matches(": u8 = 8;").count(),
        1,
        "no other constant of the table is 8"
    );
    assert!(
        src.contains("8 incomplete"),
        "the table's doc line says 8 is incomplete"
    );
    for status in 100u16..=599 {
        assert_ne!(
            exit_for_status(status),
            8,
            "status {status} must not map to 8"
        );
    }
}
