//! Wave 0 S1: the `[database]` table (socket contract r4 §2; cox's ask for hue's S5).
//!
//! Seven keys: `host`, `port`, `name`, `app_user`, `app_password_file`, `owner_user`,
//! `owner_password_file`. The table holds names and file paths, never a secret value. The parser
//! accepts a config without the table (`database` is `None`), so F-1a's accepted tests stay as
//! they are; `cortex-server` refuses to start without it. When the table is present, every key is
//! required, an unknown key stops startup and names it, and no error repeats a value.
//!
//! RED: the `[database]` arm of `load` is `todo!()`.
use std::path::{Path, PathBuf};

use cortex_config::{ConfigError, Database, DbHost, DbOwner, Env, load};

/// Writes `body` to a file of its own under cargo's per-test temporary directory.
fn config_file(name: &str, body: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("w0_database");
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let p = dir.join(format!("{name}-{}.toml", std::process::id()));
    std::fs::write(&p, body).expect("write config");
    p
}

fn home() -> Env {
    Env::from_pairs(&[("HOME", "/home/w")])
}

/// The seven keys, in file order, with valid values.
const KEYS: [(&str, &str); 7] = [
    ("host", "\"/run/postgresql\""),
    ("port", "5432"),
    ("name", "\"cortex\""),
    ("app_user", "\"cortex_app\""),
    ("app_password_file", "\"/etc/cortex/db/app.password\""),
    ("owner_user", "\"cortex_owner\""),
    ("owner_password_file", "\"/etc/cortex/db/owner.password\""),
];

/// A `[database]` table: the seven keys, with `swap` replacing one key's value (`None` leaves the
/// key out), and `extra` lines added at the end.
fn table(swap: Option<(&str, Option<&str>)>, extra: &str) -> String {
    let mut s = String::from("[database]\n");
    for (k, v) in KEYS {
        let v = match swap {
            Some((name, with)) if name == k => match with {
                Some(w) => w,
                None => continue,
            },
            _ => v,
        };
        s.push_str(&format!("{k} = {v}\n"));
    }
    s.push_str(extra);
    s
}

fn invalid_key(r: Result<cortex_config::Config, ConfigError>) -> String {
    match r {
        Err(ConfigError::Invalid { key, .. }) => key,
        other => panic!("want Invalid, got {other:?}"),
    }
}

#[test]
fn w0_db_the_seven_keys_load() {
    let f = config_file("full", &table(None, ""));
    let db = load(&f, &home()).expect("loads").database;
    assert_eq!(
        db,
        Some(Database {
            host: DbHost::SocketDir(PathBuf::from("/run/postgresql")),
            port: 5432,
            name: "cortex".to_owned(),
            app_user: "cortex_app".to_owned(),
            app_password_file: PathBuf::from("/etc/cortex/db/app.password"),
            owner: Some(DbOwner {
                user: "cortex_owner".to_owned(),
                password_file: PathBuf::from("/etc/cortex/db/owner.password"),
            }),
        })
    );
    // A host that is not an absolute path is a TCP host name (libpq's rule).
    let tcp = config_file("tcp", &table(Some(("host", Some("\"127.0.0.1\""))), ""));
    let db = load(&tcp, &home())
        .expect("loads")
        .database
        .expect("a table");
    assert_eq!(db.host, DbHost::Tcp("127.0.0.1".to_owned()));
    // The table sits beside the others and changes nothing in them.
    let both = config_file(
        "both",
        &format!(
            "[paths]\nstate_dir = \"/srv/x\"\n\n[modules]\nenabled = [\"identity\"]\n\n{}",
            table(None, "")
        ),
    );
    let cfg = load(&both, &home()).expect("loads");
    assert_eq!(cfg.paths.state_dir, PathBuf::from("/srv/x"));
    assert_eq!(cfg.modules.enabled.len(), 1);
    assert!(cfg.database.is_some());
}

#[test]
fn w0_db_a_config_without_the_table_loads_with_none() {
    let empty = config_file("empty", "");
    assert_eq!(load(&empty, &home()).expect("loads").database, None);
    let paths = config_file("paths-only", "[paths]\nstate_dir = \"/srv/x\"\n");
    assert_eq!(load(&paths, &home()).expect("loads").database, None);
}

#[test]
fn w0_db_an_unknown_key_stops_startup_naming_the_key() {
    let f = config_file("unknown", &table(None, "colour = \"blue\"\n"));
    match load(&f, &home()) {
        Err(ConfigError::UnknownKey(k)) => assert_eq!(k, "database.colour"),
        other => panic!("want UnknownKey(database.colour), got {other:?}"),
    }
}

#[test]
fn w0_db_a_secret_value_is_refused_and_never_repeated() {
    for key in ["app_password", "owner_password", "password"] {
        let f = config_file(key, &table(None, &format!("{key} = \"hunter2-{key}\"\n")));
        let err = load(&f, &home()).expect_err("a secret value in the file stops startup");
        match &err {
            ConfigError::UnknownKey(k) => assert_eq!(k, &format!("database.{key}")),
            other => panic!("want UnknownKey(database.{key}), got {other:?}"),
        }
        assert!(
            !format!("{err}").contains("hunter2"),
            "Display repeats the value"
        );
        assert!(
            !format!("{err:?}").contains("hunter2"),
            "Debug repeats the value"
        );
    }
}

#[test]
fn w0_db_every_key_is_required_and_a_missing_one_is_named() {
    for (k, _) in KEYS {
        let f = config_file(&format!("missing-{k}"), &table(Some((k, None)), ""));
        match load(&f, &home()) {
            Err(ConfigError::Missing(m)) => assert_eq!(m, format!("database.{k}")),
            other => panic!("want Missing(database.{k}), got {other:?}"),
        }
    }
    // An empty table is missing its first key in file order.
    let f = config_file("missing-all", "[database]\n");
    assert!(matches!(load(&f, &home()), Err(ConfigError::Missing(m)) if m == "database.host"));
}

#[test]
fn w0_db_password_files_must_be_absolute_paths() {
    for k in ["app_password_file", "owner_password_file"] {
        let f = config_file(
            &format!("rel-{k}"),
            &table(Some((k, Some("\"etc/cortex/x.password\""))), ""),
        );
        assert_eq!(invalid_key(load(&f, &home())), format!("database.{k}"));
        let n = config_file(&format!("num-{k}"), &table(Some((k, Some("7"))), ""));
        assert_eq!(invalid_key(load(&n, &home())), format!("database.{k}"));
    }
}

#[test]
fn w0_db_wrong_values_are_invalid_and_named() {
    // Not a table.
    let f = config_file("not-a-table", "database = 3\n");
    assert_eq!(invalid_key(load(&f, &home())), "database");
    // The port: an integer from 1 to 65535.
    for (i, bad) in ["\"5432\"", "0", "65536", "-1", "5432.0"]
        .iter()
        .enumerate()
    {
        let f = config_file(&format!("port-{i}"), &table(Some(("port", Some(bad))), ""));
        assert_eq!(
            invalid_key(load(&f, &home())),
            "database.port",
            "port = {bad}"
        );
    }
    let edge = config_file("port-max", &table(Some(("port", Some("65535"))), ""));
    let db = load(&edge, &home()).expect("65535 loads").database;
    assert_eq!(db.map(|d| d.port), Some(65535));
    // The host: a non-empty string with no whitespace or control character.
    for (i, bad) in ["\"\"", "\"db host\"", "\"db\\u0000\"", "5"]
        .iter()
        .enumerate()
    {
        let f = config_file(&format!("host-{i}"), &table(Some(("host", Some(bad))), ""));
        assert_eq!(
            invalid_key(load(&f, &home())),
            "database.host",
            "host = {bad}"
        );
    }
    // The database and role names: Postgres identifiers, 1 to 63 bytes, no NUL.
    let long = format!("\"{}\"", "a".repeat(64));
    for k in ["name", "app_user", "owner_user"] {
        for (i, bad) in ["\"\"", long.as_str(), "\"a\\u0000b\"", "1"]
            .iter()
            .enumerate()
        {
            let f = config_file(&format!("{k}-{i}"), &table(Some((k, Some(bad))), ""));
            assert_eq!(invalid_key(load(&f, &home())), format!("database.{k}"));
        }
        let max = format!("\"{}\"", "a".repeat(63));
        let ok = config_file(&format!("{k}-max"), &table(Some((k, Some(&max))), ""));
        load(&ok, &home()).expect("63 bytes load");
    }
}

#[test]
fn w0_db_no_error_repeats_a_value() {
    let secretish = "\"s3cr3t value\"";
    let cases = [
        ("host", secretish),
        ("app_password_file", "\"s3cr3t-relative\""),
        ("port", "\"s3cr3t\""),
    ];
    for (k, v) in cases {
        let f = config_file(&format!("echo-{k}"), &table(Some((k, Some(v))), ""));
        let err = load(&f, &home()).expect_err("refused");
        assert!(
            !format!("{err}").contains("s3cr3t"),
            "{k}: Display repeats it"
        );
        assert!(
            !format!("{err:?}").contains("s3cr3t"),
            "{k}: Debug repeats it"
        );
    }
}
