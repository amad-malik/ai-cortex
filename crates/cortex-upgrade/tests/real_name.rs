//! Piece 14, real ports, R4: the exact name of a service's container. `<project>-<service>`, the project being the
//! `name:` of the release's `compose.yml`, read strictly: exactly ONE line at column 0 `name: <value>`, the value
//! matching `^[a-z][a-z0-9-]{0,31}$`; anything else is refused.
#[path = "real_fakes.rs"]
mod fakes;

use cortex_upgrade::ports::Service;
use cortex_upgrade::real::name::{container_name, project_name};
use std::fs;

fn dir_with(tag: &str, compose: &str) -> fakes::common::Tmp {
    let t = fakes::common::Tmp::new(tag);
    fs::write(t.path().join("compose.yml"), compose).unwrap();
    t
}

#[test]
fn rn_01_the_name_is_the_project_and_the_service() {
    let t = dir_with("rn1", "name: cortexv2\nservices:\n  db:\n    image: x\n");
    assert_eq!(
        container_name(t.path(), Service::Db).unwrap(),
        "cortexv2-db"
    );
    assert_eq!(
        container_name(t.path(), Service::Migrate).unwrap(),
        "cortexv2-migrate"
    );
    assert_eq!(
        container_name(t.path(), Service::Server).unwrap(),
        "cortexv2-server"
    );
    // Hyphens and digits are allowed after the first letter; 32 characters are the longest.
    for ok in [
        "a",
        "a1",
        "cortex-v2",
        "a-b-c",
        &format!("a{}", "b".repeat(31)),
    ] {
        let t = dir_with("rn1b", &format!("name: {ok}\n"));
        assert_eq!(
            container_name(t.path(), Service::Db).unwrap(),
            format!("{ok}-db"),
            "{ok}"
        );
    }
}

#[test]
fn rn_02_a_name_line_that_is_not_exactly_right_is_refused() {
    let bad = |compose: &str, what: &str| {
        let t = dir_with("rn2", compose);
        assert!(
            container_name(t.path(), Service::Db).is_err(),
            "{what}: {compose:?}"
        );
    };
    bad("services:\n  db:\n    image: x\n", "no name");
    bad("name: a\nname: b\n", "two names");
    bad(
        "name: a\nservices: {}\nname: a\n",
        "two names, the same value",
    );
    bad("  name: a\n", "indented, so not at column 0");
    bad(
        "services:\n  name: a\n",
        "a name under a key is not the project's",
    );
    bad("name: Cortex\n", "an upper-case letter");
    bad("name: cortEx\n", "an upper-case letter after the first");
    bad("name: aB\n", "an upper-case letter last");
    bad("name: 1cortex\n", "a digit first");
    bad("name: -cortex\n", "a hyphen first");
    bad("name: cortex_v2\n", "an underscore");
    bad("name: cortex.v2\n", "a dot");
    bad("name: \n", "an empty value");
    bad("name:\n", "no value");
    bad("name:cortex\n", "no space after the colon");
    bad("name:  cortex\n", "two spaces after the colon");
    bad("name: cortex \n", "a trailing space");
    bad("name: cortex\r\n", "a carriage return");
    bad("name: cortex # c\n", "a comment");
    bad("name: \"cortex\"\n", "quotes");
    bad("name: 'cortex'\n", "single quotes");
    bad("NAME: cortex\n", "another case of the key");
    bad("name : cortex\n", "a space before the colon");
    bad(&format!("name: a{}\n", "b".repeat(32)), "33 characters");
    bad("name: caf\u{e9}\n", "a non-ASCII letter");
    bad("name: a\0b\n", "a NUL");
    bad("", "an empty file");
}

#[test]
fn rn_03_a_missing_or_huge_or_unreadable_compose_is_refused() {
    let t = fakes::common::Tmp::new("rn3");
    assert!(
        container_name(t.path(), Service::Db).is_err(),
        "no compose.yml"
    );
    fs::create_dir(t.path().join("compose.yml")).unwrap();
    assert!(
        container_name(t.path(), Service::Db).is_err(),
        "a directory"
    );
    fs::remove_dir(t.path().join("compose.yml")).unwrap();
    // More than 1 MiB is not a compose file of this stack, whatever its first line says.
    let mut big = String::from("name: cortexv2\n");
    big.push_str(&"# padding\n".repeat(120_000));
    fs::write(t.path().join("compose.yml"), &big).unwrap();
    assert!(big.len() > 1 << 20);
    assert!(container_name(t.path(), Service::Db).is_err(), "over 1 MiB");
    // A file just under the limit is read.
    let mut ok = String::from("name: cortexv2\n");
    ok.push_str(&"#\n".repeat(500_000));
    fs::write(t.path().join("compose.yml"), &ok).unwrap();
    assert!(ok.len() < 1 << 20);
    assert!(
        container_name(t.path(), Service::Db).is_ok(),
        "just under 1 MiB"
    );
    // The boundary itself: exactly 1 MiB is read, one byte more is refused.
    let mut edge = String::from("name: cortexv2\n");
    edge.push_str(&"#".repeat((1 << 20) - edge.len()));
    assert_eq!(edge.len(), 1 << 20);
    fs::write(t.path().join("compose.yml"), &edge).unwrap();
    assert!(
        container_name(t.path(), Service::Db).is_ok(),
        "exactly 1 MiB"
    );
    edge.push('#');
    fs::write(t.path().join("compose.yml"), &edge).unwrap();
    assert!(
        container_name(t.path(), Service::Db).is_err(),
        "1 MiB and a byte"
    );
}

#[test]
fn rn_05_other_keys_at_column_zero_that_merely_start_with_name_are_not_the_project() {
    let t = dir_with("rn5", "namespace: x\nnames: y\nname: cortexv2\nname_x: z\n");
    assert_eq!(
        container_name(t.path(), Service::Db).unwrap(),
        "cortexv2-db"
    );
    let t = dir_with("rn5b", "namespace: x\nname_x: z\n");
    assert!(
        container_name(t.path(), Service::Db).is_err(),
        "no name line at all"
    );
}

#[test]
fn rn_04_the_refusal_carries_nothing_of_the_operator() {
    let t = dir_with("rn4", "name: UPPER\n");
    let e = container_name(t.path(), Service::Db).unwrap_err();
    let shown = format!("{e:?} {e:?}");
    assert!(
        !shown.contains(&t.path().display().to_string()) && !shown.contains("UPPER"),
        "{shown}"
    );
}

/// vera's UPREAL-V2: `File::open` of a FIFO that has no writer blocks until one appears, before any check and before any
/// deadline. The call runs on its own thread so that a regression fails this cell instead of hanging the test runner.
#[test]
fn rn_06_a_compose_that_is_a_fifo_with_no_writer_is_refused_at_once() {
    use std::process::Command;
    use std::sync::mpsc;
    use std::time::Duration;
    let t = fakes::common::Tmp::new("rn6");
    let fifo = t.path().join("compose.yml");
    assert!(
        Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    let dir = t.path().to_path_buf();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send((
            project_name(&dir).is_err(),
            container_name(&dir, Service::Db).is_err(),
        ));
    });
    match rx.recv_timeout(Duration::from_secs(5)) {
        Ok(refused) => assert_eq!(refused, (true, true)),
        Err(_) => {
            // let the blocked open go, so that nothing is left waiting, then fail
            let _ = std::fs::OpenOptions::new().write(true).open(&fifo);
            panic!("the project name blocked on a FIFO compose.yml with no writer");
        }
    }
    // A FIFO that HOLDS a valid name is still not a compose file: the answer comes from what the descriptor IS, not from
    // what could be read from it. A reader and a writer are held here, so the name is queued before the call.
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let reader = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(0o4000)
        .open(&fifo)
        .unwrap();
    let mut writer = std::fs::OpenOptions::new().write(true).open(&fifo).unwrap();
    writer.write_all(b"name: cortexv2\n").unwrap();
    // The writer is closed, the reader stays: the bytes stay queued and end-of-file follows, so a read of this FIFO WOULD
    // give a valid name (with the writer still open, a non-blocking read ends in "would block" and the file is refused
    // for the wrong reason).
    drop(writer);
    assert!(
        project_name(t.path()).is_err(),
        "a FIFO is not a compose file"
    );
    drop(reader);
}
