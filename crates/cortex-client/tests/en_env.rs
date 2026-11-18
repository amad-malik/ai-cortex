//! EN-02 to EN-05: token sources, the server address and the loopback rule (mvp/tests/cortex-client-and-cli.md, section 2).
//! The injected lookup panics on any key the module does not declare, so every test also proves EN-02.
mod support;

use cortex_client::endpoint::DEFAULT_SERVER;
use cortex_client::env::{VAR_SERVER, VAR_TOKEN, VAR_TOKEN_FILE, resolve_server, resolve_token};
use cortex_client::{Endpoint, EndpointError, TokenError};
use std::ffi::OsString;
use std::net::{IpAddr, Ipv4Addr};
use std::path::Path;

/// A lookup over a map that PANICS on any key except the three the module declares (EN-02).
fn lookup<'a>(pairs: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<OsString> + 'a {
    move |name: &str| {
        assert!(
            [VAR_TOKEN, VAR_TOKEN_FILE, VAR_SERVER].contains(&name),
            "EN-02: the module looked up {name}, which it does not declare"
        );
        pairs
            .iter()
            .find(|(k, _)| *k == name)
            .map(|(_, v)| OsString::from(v))
    }
}

fn token_file(content: &str) -> std::path::PathBuf {
    let d = support::dir("en");
    support::write(&d, "tok", content.as_bytes(), 0o600)
}

#[test]
fn en_02_the_three_variables_are_exactly_these() {
    assert_eq!(VAR_TOKEN, "CORTEX_TOKEN");
    assert_eq!(VAR_TOKEN_FILE, "CORTEX_TOKEN_FILE");
    assert_eq!(VAR_SERVER, "CORTEX_SERVER");
    // nothing set: both resolvers ask only for declared keys (the lookup panics otherwise)
    let none: [(&str, &str); 0] = [];
    let _ = resolve_token(None, &lookup(&none));
    let _ = resolve_server(None, &lookup(&none));
}

#[test]
fn en_03_token_source_precedence_flag_then_token_then_token_file() {
    let f_flag = token_file("from-flag\n");
    let f_env = token_file("from-token-file\n");
    let flag_s = f_flag.display().to_string();
    let env_s = f_env.display().to_string();
    let all = [(VAR_TOKEN, "from-token"), (VAR_TOKEN_FILE, env_s.as_str())];
    assert_eq!(
        resolve_token(Some(Path::new(&flag_s)), &lookup(&all))
            .unwrap()
            .expose(),
        "from-flag"
    );
    assert_eq!(
        resolve_token(None, &lookup(&all)).unwrap().expose(),
        "from-token"
    );
    let only_file = [(VAR_TOKEN_FILE, env_s.as_str())];
    assert_eq!(
        resolve_token(None, &lookup(&only_file)).unwrap().expose(),
        "from-token-file"
    );
}

#[test]
fn en_03_nothing_set_is_an_error_that_names_the_three_sources() {
    let none: [(&str, &str); 0] = [];
    let e = resolve_token(None, &lookup(&none)).unwrap_err();
    assert_eq!(e, TokenError::NoSource);
    let text = e.to_string();
    for needle in ["--token-file", "CORTEX_TOKEN", "CORTEX_TOKEN_FILE"] {
        assert!(text.contains(needle), "{text}");
    }
}

#[test]
fn en_03_a_broken_higher_source_does_not_fall_through() {
    let good = token_file("good\n");
    let good_s = good.display().to_string();
    // the flag names a missing file while CORTEX_TOKEN is fine: the flag's error wins
    let env = [(VAR_TOKEN, "fine")];
    let d = support::dir("en-missing");
    let e = resolve_token(Some(&d.join("absent")), &lookup(&env)).unwrap_err();
    assert!(matches!(e, TokenError::NotFound { .. }), "{e:?}");
    // set-but-empty CORTEX_TOKEN is an error although CORTEX_TOKEN_FILE is good (W1-43 Q-7)
    let env = [(VAR_TOKEN, ""), (VAR_TOKEN_FILE, good_s.as_str())];
    assert_eq!(
        resolve_token(None, &lookup(&env)).unwrap_err(),
        TokenError::Empty {
            source_name: "CORTEX_TOKEN".into()
        }
    );
    // set-but-empty CORTEX_TOKEN_FILE, and an empty --token-file
    let env = [(VAR_TOKEN_FILE, "")];
    assert_eq!(
        resolve_token(None, &lookup(&env)).unwrap_err(),
        TokenError::Empty {
            source_name: "CORTEX_TOKEN_FILE".into()
        }
    );
    let none: [(&str, &str); 0] = [];
    assert_eq!(
        resolve_token(Some(Path::new("")), &lookup(&none)).unwrap_err(),
        TokenError::Empty {
            source_name: "--token-file".into()
        }
    );
    // a bad byte in CORTEX_TOKEN is refused by the same grammar
    let env = [(VAR_TOKEN, "ab cd")];
    assert!(matches!(
        resolve_token(None, &lookup(&env)).unwrap_err(),
        TokenError::BadByte { .. }
    ));
}

#[test]
fn en_04_server_precedence_flag_then_variable_then_default() {
    let none: [(&str, &str); 0] = [];
    let env = [(VAR_SERVER, "http://127.0.0.1:7001")];
    assert_eq!(DEFAULT_SERVER, "http://127.0.0.1:8601", "W1-43 Q-8");
    assert_eq!(
        resolve_server(None, &lookup(&none)).unwrap().to_string(),
        DEFAULT_SERVER
    );
    assert_eq!(
        resolve_server(None, &lookup(&env)).unwrap().to_string(),
        "http://127.0.0.1:7001"
    );
    assert_eq!(
        resolve_server(Some("http://127.0.0.1:7002"), &lookup(&env))
            .unwrap()
            .to_string(),
        "http://127.0.0.1:7002"
    );
    // a bad variable is an error; it does not fall back to the default
    let bad = [(VAR_SERVER, "https://127.0.0.1:7001")];
    assert!(resolve_server(None, &lookup(&bad)).is_err());
    let bad_flag = resolve_server(Some("http://example.com:80"), &lookup(&env));
    assert_eq!(bad_flag.unwrap_err(), EndpointError::NotLoopback);
}

#[test]
fn en_05_the_address_table() {
    let accepted = [
        ("http://127.0.0.1:1", "http://127.0.0.1:1"),
        ("http://127.0.0.1:8601", "http://127.0.0.1:8601"),
        ("http://127.1.2.3:65535", "http://127.1.2.3:65535"),
        ("http://[::1]:8601", "http://[::1]:8601"),
        ("http://localhost:8601", "http://127.0.0.1:8601"),
        ("http://LOCALHOST:8601", "http://127.0.0.1:8601"),
    ];
    for (input, shown) in accepted {
        let e = Endpoint::parse(input).unwrap_or_else(|e| panic!("{input} must be accepted: {e}"));
        assert_eq!(e.to_string(), shown, "{input}");
    }
    assert_eq!(
        Endpoint::parse("http://127.0.0.1:8601").unwrap().ip(),
        IpAddr::V4(Ipv4Addr::LOCALHOST)
    );
    assert_eq!(
        Endpoint::parse("http://127.0.0.1:8601").unwrap().port(),
        8601
    );

    let refused: [(&str, EndpointError); 20] = [
        ("http://10.0.0.5:80", EndpointError::NotLoopback),
        ("http://example.com:80", EndpointError::NotLoopback),
        ("http://0.0.0.0:80", EndpointError::NotLoopback),
        ("http://192.168.1.1:80", EndpointError::NotLoopback),
        ("http://[::]:80", EndpointError::NotLoopback),
        ("http://[2001:db8::1]:80", EndpointError::NotLoopback),
        (
            "http://localhost.example.com:80",
            EndpointError::NotLoopback,
        ),
        ("https://127.0.0.1:8601", EndpointError::Scheme),
        ("unix:///run/cortex.sock", EndpointError::Scheme),
        ("127.0.0.1:8601", EndpointError::Scheme),
        ("http://u:p@127.0.0.1:8601", EndpointError::Shape),
        ("http://127.0.0.1:8601/", EndpointError::Shape),
        ("http://127.0.0.1:8601/x", EndpointError::Shape),
        ("http://127.0.0.1:8601?a=1", EndpointError::Shape),
        ("http://127.0.0.1:8601#f", EndpointError::Shape),
        ("http://127.0.0.1", EndpointError::Shape),
        ("http://127.0.0.1:0", EndpointError::Port),
        ("http://127.0.0.1:65536", EndpointError::Port),
        ("http://127.0.0.1:08601", EndpointError::Port),
        ("http://127.0.0.1:80 ", EndpointError::Shape),
    ];
    for (input, want) in refused {
        assert_eq!(Endpoint::parse(input).unwrap_err(), want, "{input:?}");
    }
}
