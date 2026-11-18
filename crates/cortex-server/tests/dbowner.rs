//! The owner role's connection settings from the config. DB-free. RED first.

use cortex_config::{Database, DbHost, DbOwner};
use cortex_db::DbEndpoint;
use cortex_server::dbowner::{OwnerConfigError, owner_config};
use std::fs;
use std::path::PathBuf;

fn dir(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("dbowner-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).expect("dir");
    d
}

fn db(host: DbHost, owner: Option<(&str, PathBuf)>) -> Database {
    Database {
        host,
        port: 5432,
        name: "cortex".into(),
        app_user: "cortex_app".into(),
        app_password_file: PathBuf::from("/nowhere/app.password"),
        owner: owner.map(|(u, p)| DbOwner {
            user: u.into(),
            password_file: p,
        }),
    }
}

#[test]
fn ow_01_the_owner_settings_come_from_the_owner_keys_and_the_owner_file_only() {
    let d = dir("ok");
    let pw = d.join("owner.password");
    fs::write(&pw, "ownerpw\n").expect("seed");
    let c = owner_config(&db(
        DbHost::SocketDir(PathBuf::from("/run/postgresql")),
        Some(("cortex_owner", pw.clone())),
    ))
    .expect("config");
    assert_eq!(
        (
            c.database.as_str(),
            c.owner_user.as_str(),
            c.owner_password.expose()
        ),
        ("cortex", "cortex_owner", "ownerpw")
    );
    assert!(
        matches!(&c.endpoint, DbEndpoint::Socket { dir, port: 5432 } if dir == &PathBuf::from("/run/postgresql"))
    );
    let c2 = owner_config(&db(
        DbHost::Tcp("db.internal".into()),
        Some(("cortex_owner", pw)),
    ))
    .expect("config");
    assert!(matches!(&c2.endpoint, DbEndpoint::Tcp { host, port: 5432 } if host == "db.internal"));
}

#[test]
fn ow_02_missing_owner_keys_are_named_by_key_and_the_app_file_is_never_read() {
    let e = owner_config(&db(
        DbHost::SocketDir(PathBuf::from("/run/postgresql")),
        None,
    ))
    .err()
    .expect("no owner keys");
    assert_eq!(e, OwnerConfigError::Missing("database.owner_user".into()));
    assert_eq!(e.to_string(), "missing: database.owner_user");
}

#[test]
fn ow_03_an_unreadable_owner_file_is_a_fixed_line_without_the_path() {
    let d = dir("unreadable");
    let secret_path = d.join("owner-secret-path-ZZ9");
    let e = owner_config(&db(
        DbHost::SocketDir(PathBuf::from("/run/postgresql")),
        Some(("cortex_owner", secret_path)),
    ))
    .err()
    .expect("missing file");
    assert_eq!(e, OwnerConfigError::PasswordUnreadable);
    assert!(!format!("{e} {e:?}").contains("ZZ9"));
}
