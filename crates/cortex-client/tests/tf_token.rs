//! TF-01 to TF-10: the token file and the token type (mvp/tests/cortex-client-and-cli.md, section 1). DB-free, no network.
mod support;

use cortex_client::{Token, TokenError, read_token_file};
use std::os::unix::fs::symlink;
use std::sync::mpsc;
use std::time::Duration;

const CANARY: &str = "TOKEN-CANARY-8f31c2a9d4";

fn good(dir: &std::path::Path, content: &str) -> std::path::PathBuf {
    support::write(dir, "tok", content.as_bytes(), 0o600)
}

#[test]
fn tf_01_one_trailing_newline_is_removed_and_nothing_else() {
    let d = support::dir("tf01");
    for (content, want) in [
        ("TOKEN\n", "TOKEN"),
        ("TOKEN\r\n", "TOKEN"),
        ("TOKEN", "TOKEN"),
    ] {
        let p = good(&d, content);
        let t = read_token_file(&p).expect("a good file reads");
        assert_eq!(t.expose(), want, "content {content:?}");
    }
}

#[test]
fn tf_02_the_mode_table() {
    let d = support::dir("tf02");
    for mode in [0o600, 0o400, 0o700] {
        let p = support::write(&d, "ok", format!("{CANARY}\n").as_bytes(), mode);
        read_token_file(&p).unwrap_or_else(|e| panic!("mode {mode:04o} must be accepted: {e}"));
    }
    for mode in [0o640, 0o604, 0o660, 0o606, 0o644, 0o666, 0o610, 0o601] {
        let p = support::write(&d, "bad", format!("{CANARY}\n").as_bytes(), mode);
        match read_token_file(&p) {
            Err(e @ TokenError::FileMode { .. }) => {
                let TokenError::FileMode { path, mode: m } = &e else {
                    unreachable!()
                };
                assert_eq!(path, &p.display().to_string());
                assert_eq!(*m, mode, "the error carries the mode");
                let text = e.to_string();
                assert!(
                    text.contains(&p.display().to_string()),
                    "names the path: {text}"
                );
                assert!(
                    text.contains(&format!("{mode:04o}")),
                    "names the octal mode: {text}"
                );
                assert!(!text.contains(CANARY), "never the contents: {text}");
            }
            other => panic!("mode {mode:04o} must be refused with FileMode, got {other:?}"),
        }
    }
}

#[test]
fn tf_03_the_parent_directory_mode() {
    for (mode, ok) in [
        (0o700, true),
        (0o750, false),
        (0o705, false),
        (0o770, false),
    ] {
        let d = support::dir("tf03");
        let p = good(&d, &format!("{CANARY}\n"));
        support::set_mode(&d, mode);
        let r = read_token_file(&p);
        // restore so the harness can clean up
        support::set_mode(&d, 0o700);
        match (ok, r) {
            (true, Ok(_)) => {}
            (false, Err(TokenError::DirMode { path, mode: m })) => {
                assert_eq!(path, d.display().to_string(), "names the DIRECTORY");
                assert_eq!(m, mode);
            }
            (ok, other) => panic!("dir mode {mode:04o}: expected ok={ok}, got {other:?}"),
        }
    }
}

#[test]
fn tf_04_a_symlink_as_last_component_is_refused_but_an_earlier_one_is_fine() {
    let d = support::dir("tf04");
    let real = good(&d, &format!("{CANARY}\n"));
    let link = d.join("link");
    symlink(&real, &link).unwrap();
    assert!(
        matches!(read_token_file(&link), Err(TokenError::Symlink { .. })),
        "a symlink to a 0600 file is refused"
    );
    // an earlier component: dir2/ -> d (a symlinked directory)
    let outer = support::dir("tf04b");
    let dirlink = outer.join("dirlink");
    symlink(&d, &dirlink).unwrap();
    read_token_file(&dirlink.join("tok")).expect("a symlink in an earlier component is allowed");
}

#[test]
fn tf_05_a_fifo_a_directory_and_a_device_are_refused_without_blocking() {
    let d = support::dir("tf05");
    let fifo = d.join("fifo");
    let st = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("mkfifo");
    assert!(st.success());
    support::set_mode(&fifo, 0o600);
    let (tx, rx) = mpsc::channel();
    let f = fifo.clone();
    std::thread::spawn(move || {
        let _ = tx.send(read_token_file(&f).map(|_| ()));
    });
    match rx.recv_timeout(Duration::from_secs(1)) {
        Ok(Err(TokenError::NotRegular { .. })) => {}
        Ok(other) => panic!("a FIFO must be NotRegular, got {other:?}"),
        Err(_) => panic!("read_token_file blocked on a FIFO"),
    }
    let sub = d.join("sub");
    std::fs::create_dir(&sub).unwrap();
    support::set_mode(&sub, 0o700);
    assert!(matches!(
        read_token_file(&sub),
        Err(TokenError::IsDirectory { .. })
    ));
    assert!(matches!(
        read_token_file(std::path::Path::new("/dev/null")),
        Err(TokenError::NotRegular { .. })
    ));
}

#[test]
fn tf_06_empty_and_blank_files() {
    let d = support::dir("tf06");
    for content in ["", "\n", "   \n\n", " \t \r\n"] {
        let p = good(&d, content);
        assert!(
            matches!(read_token_file(&p), Err(TokenError::Empty { .. })),
            "content {content:?} must be Empty"
        );
    }
}

#[test]
fn tf_07_the_content_grammar() {
    let long_ok = "a".repeat(4096);
    let long_bad = "a".repeat(4097);
    let accepted: [&str; 4] = ["a", "TOKEN-abc_123.xyz~!", "~", &long_ok];
    for s in accepted {
        Token::parse(s, "test").unwrap_or_else(|e| panic!("{:?} must be accepted: {e}", s.len()));
    }
    let refused: [(&str, &str); 9] = [
        ("an interior space", "ab cd"),
        ("a tab", "ab\tcd"),
        ("a second line", "a\nb"),
        ("a CR in the middle", "a\rb"),
        ("NUL", "a\0b"),
        ("DEL", "a\x7fb"),
        ("a high byte", "a\u{e9}b"),
        ("4097 bytes", &long_bad),
        ("a trailing space", "ab "),
    ];
    for (what, s) in refused {
        assert!(Token::parse(s, "test").is_err(), "{what} must be refused");
    }
    // through the file as well: 4096 ok, 4097 refused, a second line refused
    let d = support::dir("tf07");
    assert!(read_token_file(&good(&d, &format!("{long_ok}\n"))).is_ok());
    assert_eq!(
        read_token_file(&good(&d, &long_bad)).unwrap_err(),
        TokenError::TooLong
    );
    assert!(matches!(
        read_token_file(&good(&d, "a\nb\n")),
        Err(TokenError::BadByte { .. })
    ));
}

#[test]
fn tf_08_missing_unreadable_and_directory_name_the_path_and_no_content() {
    let d = support::dir("tf08");
    let missing = d.join("nope");
    let e = read_token_file(&missing).unwrap_err();
    assert!(matches!(e, TokenError::NotFound { .. }));
    assert!(e.to_string().contains(&missing.display().to_string()));

    let p = support::write(&d, "locked", format!("{CANARY}\n").as_bytes(), 0o000);
    let e = read_token_file(&p).unwrap_err();
    assert!(
        matches!(e, TokenError::PermissionDenied { .. }),
        "got {e:?}"
    );
    assert!(e.to_string().contains(&p.display().to_string()));
    assert!(!e.to_string().contains(CANARY));
    support::set_mode(&p, 0o600);

    let sub = d.join("subdir");
    std::fs::create_dir(&sub).unwrap();
    let e = read_token_file(&sub).unwrap_err();
    assert!(matches!(e, TokenError::IsDirectory { .. }));
    assert!(e.to_string().contains(&sub.display().to_string()));
}

#[test]
fn tf_09_the_environment_token_has_the_same_grammar_and_an_empty_one_is_an_error() {
    assert_eq!(
        Token::parse("", "CORTEX_TOKEN").unwrap_err(),
        TokenError::Empty {
            source_name: "CORTEX_TOKEN".into()
        }
    );
    assert!(Token::parse("ab cd", "CORTEX_TOKEN").is_err());
    assert_eq!(Token::parse("abc", "CORTEX_TOKEN").unwrap().expose(), "abc");
}

#[test]
fn tf_10_debug_of_the_token_and_of_every_error_never_shows_the_token() {
    let t = Token::parse(CANARY, "test").unwrap();
    for shown in [format!("{t:?}"), format!("{t:#?}")] {
        assert_eq!(shown.trim(), "Token(<redacted>)");
        assert!(!shown.contains(&CANARY[..8]));
        assert!(!shown.contains(&CANARY.len().to_string()));
    }
    let errors = [
        TokenError::FileMode {
            path: "/p".into(),
            mode: 0o644,
        },
        TokenError::DirMode {
            path: "/d".into(),
            mode: 0o755,
        },
        TokenError::Symlink { path: "/p".into() },
        TokenError::NotRegular { path: "/p".into() },
        TokenError::NotFound { path: "/p".into() },
        TokenError::PermissionDenied { path: "/p".into() },
        TokenError::IsDirectory { path: "/p".into() },
        TokenError::Io {
            path: "/p".into(),
            kind: "other".into(),
        },
        TokenError::Empty {
            source_name: "CORTEX_TOKEN".into(),
        },
        TokenError::TooLong,
        TokenError::BadByte { offset: 3 },
        TokenError::NotUtf8,
        TokenError::NoSource,
    ];
    for e in errors {
        for shown in [format!("{e}"), format!("{e:?}"), format!("{e:#?}")] {
            assert!(!shown.contains(CANARY), "{shown}");
        }
    }
    // a real failure on a file that holds the canary also names no content
    let d = support::dir("tf10");
    let p = support::write(&d, "tok", format!("{CANARY} x\n").as_bytes(), 0o600);
    let e = read_token_file(&p).unwrap_err();
    for shown in [format!("{e}"), format!("{e:?}")] {
        assert!(!shown.contains(CANARY), "{shown}");
    }
}
