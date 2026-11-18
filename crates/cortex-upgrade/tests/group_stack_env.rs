//! Piece 14, design section 2: the rules of `stack.env`. Three keys, no secrets, a pinned digest on each image.

use cortex_upgrade::stack_env::{STACK_ENV_MAX_BYTES, StackEnvError, parse_stack_env};

const D: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn good() -> String {
    format!(
        "CORTEX_VERSION=0.2.1-rc.2\nCORTEX_SERVER_IMAGE=localhost/cortex-server@{D}\nCORTEX_DB_IMAGE=docker.io/pgvector/pgvector:0.8.2-pg18@{D}\n"
    )
}

fn msg_is_safe(e: &StackEnvError) {
    let m = e.to_string();
    assert!(
        !m.is_empty() && m.len() < 200 && !m.chars().any(char::is_control),
        "{m:?}"
    );
}

#[test]
fn se_01_a_good_file_parses_with_comments_blank_lines_any_order_and_no_final_newline() {
    let s = parse_stack_env(&good()).unwrap();
    assert_eq!(s.version.to_string(), "0.2.1-rc.2");
    assert_eq!(s.server_image.0, format!("localhost/cortex-server@{D}"));
    assert_eq!(
        s.db_image.0,
        format!("docker.io/pgvector/pgvector:0.8.2-pg18@{D}")
    );
    let shuffled = format!(
        "# the stack of one release\n\nCORTEX_DB_IMAGE=a/b@{D}\n   \nCORTEX_VERSION=1.0.0\nCORTEX_SERVER_IMAGE=c/d@{D}"
    );
    let s = parse_stack_env(&shuffled).unwrap();
    assert_eq!(
        (s.version.to_string(), s.db_image.0, s.server_image.0),
        ("1.0.0".into(), format!("a/b@{D}"), format!("c/d@{D}"))
    );
}

#[test]
fn se_02_each_key_is_required_exactly_once() {
    for key in ["CORTEX_VERSION", "CORTEX_SERVER_IMAGE", "CORTEX_DB_IMAGE"] {
        let without: String = good()
            .lines()
            .filter(|l| !l.starts_with(key))
            .map(|l| format!("{l}\n"))
            .collect();
        assert_eq!(
            parse_stack_env(&without),
            Err(StackEnvError::Missing {
                key: Box::leak(key.to_string().into_boxed_str())
            }),
            "{key}"
        );
        let twice = format!(
            "{}{}\n",
            good(),
            good().lines().find(|l| l.starts_with(key)).unwrap()
        );
        assert!(
            matches!(
                parse_stack_env(&twice),
                Err(StackEnvError::Duplicate { line: 4 })
            ),
            "{key}"
        );
    }
}

#[test]
fn se_03_no_other_key_may_be_there_least_of_all_a_secret() {
    for extra in [
        "POSTGRES_PASSWORD=hunter2",
        "CORTEX_DB_PASSWORD=hunter2",
        "cortex_version=1.0.0",
        "CORTEX_VERSION =1.0.0",
        "EXTRA=1",
        "=x",
    ] {
        let e = parse_stack_env(&format!("{}{extra}\n", good())).expect_err(extra);
        assert!(
            matches!(
                e,
                StackEnvError::UnknownKey { line: 4 } | StackEnvError::BadLine { line: 4 }
            ),
            "{extra}: {e:?}"
        );
        assert!(
            !e.to_string().contains("hunter2"),
            "the value must never be in the message"
        );
        msg_is_safe(&e);
    }
}

#[test]
fn se_04_values_outside_the_allowed_characters_are_refused_and_never_echoed() {
    for bad in [
        "$(id)",
        "`id`",
        "a b",
        "\"quoted\"",
        "'q'",
        "x;y",
        "x\ry",
        "${HOME}",
        "x\\y",
        "é",
        "a|b",
    ] {
        let text = good().replace(
            "CORTEX_VERSION=0.2.1-rc.2",
            &format!("CORTEX_VERSION={bad}"),
        );
        let e = parse_stack_env(&text).expect_err(bad);
        assert_eq!(e, StackEnvError::BadValue { line: 1 }, "{bad:?}");
        msg_is_safe(&e);
    }
    let long = format!("CORTEX_VERSION={}", "1".repeat(301));
    assert!(matches!(
        parse_stack_env(&good().replace("CORTEX_VERSION=0.2.1-rc.2", &long)),
        Err(StackEnvError::BadValue { line: 1 })
    ));
}

#[test]
fn se_05_the_version_is_a_version_and_each_image_is_pinned_by_digest() {
    for v in ["0.2.001", "1.0", "1.0.0+build", "v1.0.0", ""] {
        let t = good().replace("CORTEX_VERSION=0.2.1-rc.2", &format!("CORTEX_VERSION={v}"));
        assert!(
            matches!(
                parse_stack_env(&t),
                Err(StackEnvError::Invalid {
                    key: "CORTEX_VERSION"
                } | StackEnvError::BadValue { .. })
            ),
            "{v:?}"
        );
    }
    for key in ["CORTEX_SERVER_IMAGE", "CORTEX_DB_IMAGE"] {
        for bad in [
            "localhost/x",
            "localhost/x@sha256:abc",
            "localhost/x:latest",
            &format!("localhost/x@{}", D.to_uppercase()),
            &format!("localhost/x@{D}0"),
            &format!("@{D}"),
            "localhost/x@md5:00",
            &format!("localhost/x@sha256:{}", "A".repeat(64)),
            &format!("a@b@{D}"),
        ] {
            let line = good()
                .lines()
                .find(|l| l.starts_with(key))
                .unwrap()
                .to_string();
            let t = good().replace(&line, &format!("{key}={bad}"));
            let e = parse_stack_env(&t).expect_err(bad);
            assert!(
                matches!(
                    e,
                    StackEnvError::Invalid { .. } | StackEnvError::BadValue { .. }
                ),
                "{key} {bad}: {e:?}"
            );
        }
    }
}

#[test]
fn se_06_a_line_without_equals_is_refused_and_the_size_cap_is_exact() {
    assert_eq!(
        parse_stack_env(&format!("{}JUSTWORDS\n", good())),
        Err(StackEnvError::BadLine { line: 4 })
    );
    // exactly at the cap: pad with comment lines; one byte over: refused
    let mut at = good();
    while at.len() + 100 < STACK_ENV_MAX_BYTES {
        at.push_str(&format!("#{}\n", "x".repeat(98)));
    }
    let pad = STACK_ENV_MAX_BYTES - at.len();
    at.push_str(&format!("#{}", "x".repeat(pad - 1)));
    assert_eq!(at.len(), STACK_ENV_MAX_BYTES);
    assert!(
        parse_stack_env(&at).is_ok(),
        "exactly at the cap is accepted"
    );
    at.push('x');
    assert_eq!(parse_stack_env(&at), Err(StackEnvError::TooLarge));
}
