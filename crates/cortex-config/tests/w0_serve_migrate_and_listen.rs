//! Wave 0 S1: the config as piece 11 needs it (cox's pin, row 59cd26c0; piece 11 contract r0
//! sections 1 and 2).
//!
//! - **`serve` needs no owner key.** `owner_user` and `owner_password_file` are optional, both or
//!   neither. Only `cortex-server migrate` asks for them, through `Database::owner_for_migrate`.
//!   The loader never reads a password file, so a file that does not exist in the serve
//!   container cannot stop `serve`.
//! - **`[server] listen`:** an IP address and port; `127.0.0.1:8601` when absent; `0.0.0.0:<port>`
//!   inside the container.
//!
//! RED: `Database::owner_for_migrate` and the `[server]` table are `todo!()`, and the owner keys
//! are still required.
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use cortex_config::{ConfigError, DEFAULT_LISTEN, DbOwner, Env, load};

fn config_file(name: &str, body: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("w0_serve_migrate");
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let p = dir.join(format!("{name}-{}.toml", std::process::id()));
    std::fs::write(&p, body).expect("write config");
    p
}

fn home() -> Env {
    Env::from_pairs(&[("HOME", "/home/w")])
}

const SERVE_KEYS: &str = "[database]\nhost = \"db\"\nport = 5432\nname = \"cortex\"\n\
    app_user = \"cortex_app\"\napp_password_file = \"/run/secrets/app\"\n";

/// Files that exist nowhere: the loader must not care.
const ABSENT_OWNER_FILE: &str = "/nonexistent/w0/owner.password";

#[test]
fn w0_serve_loads_without_the_owner_keys() {
    let f = config_file("serve", SERVE_KEYS);
    let db = load(&f, &home())
        .expect("a serve config without owner keys loads")
        .database
        .expect("the table");
    assert_eq!(db.owner, None);
    match db.owner_for_migrate() {
        Err(ConfigError::Missing(k)) => assert_eq!(k, "database.owner_user"),
        other => panic!("migrate must refuse without the owner keys, got {other:?}"),
    }
}

#[test]
fn w0_the_owner_keys_are_both_or_neither() {
    let only_user = config_file(
        "only-user",
        &format!("{SERVE_KEYS}owner_user = \"cortex_owner\"\n"),
    );
    assert!(
        matches!(load(&only_user, &home()), Err(ConfigError::Missing(k)) if k == "database.owner_password_file")
    );
    let only_file = config_file(
        "only-file",
        &format!("{SERVE_KEYS}owner_password_file = \"{ABSENT_OWNER_FILE}\"\n"),
    );
    assert!(
        matches!(load(&only_file, &home()), Err(ConfigError::Missing(k)) if k == "database.owner_user")
    );
}

#[test]
fn w0_migrate_gets_the_owner_keys_and_no_file_is_read() {
    assert!(!Path::new(ABSENT_OWNER_FILE).exists());
    let f = config_file(
        "migrate",
        &format!(
            "{SERVE_KEYS}owner_user = \"cortex_owner\"\nowner_password_file = \"{ABSENT_OWNER_FILE}\"\n"
        ),
    );
    let db = load(&f, &home())
        .expect("loads: no password file is read")
        .database
        .expect("the table");
    assert_eq!(
        db.owner_for_migrate().expect("migrate has its keys"),
        &DbOwner {
            user: "cortex_owner".into(),
            password_file: PathBuf::from(ABSENT_OWNER_FILE),
        }
    );
}

#[test]
fn w0_owner_keys_that_are_present_are_still_checked() {
    let f = config_file(
        "owner-relative",
        &format!(
            "{SERVE_KEYS}owner_user = \"cortex_owner\"\nowner_password_file = \"run/owner\"\n"
        ),
    );
    assert!(
        matches!(load(&f, &home()), Err(ConfigError::Invalid { key, .. }) if key == "database.owner_password_file")
    );
}

#[test]
fn w0_server_listen_defaults_to_loopback_8601() {
    let f = config_file("no-server", "");
    let cfg = load(&f, &home()).expect("loads");
    let want: SocketAddr = DEFAULT_LISTEN.parse().expect("the default parses");
    assert_eq!(cfg.server.listen, want);
    assert_eq!(DEFAULT_LISTEN, "127.0.0.1:8601");
}

#[test]
fn w0_server_listen_takes_an_ip_and_a_port() {
    for ok in ["0.0.0.0:8601", "127.0.0.1:9000", "[::1]:8601"] {
        let f = config_file("listen-ok", &format!("[server]\nlisten = \"{ok}\"\n"));
        let cfg = load(&f, &home()).unwrap_or_else(|e| panic!("{ok}: {e:?}"));
        assert_eq!(cfg.server.listen, ok.parse::<SocketAddr>().expect("parses"));
    }
    for (i, bad) in [
        "\"localhost:8601\"",
        "\"0.0.0.0\"",
        "\"0.0.0.0:0\"",
        "\"\"",
        "8601",
    ]
    .iter()
    .enumerate()
    {
        let f = config_file(
            &format!("listen-bad-{i}"),
            &format!("[server]\nlisten = {bad}\n"),
        );
        assert!(
            matches!(load(&f, &home()), Err(ConfigError::Invalid { key, .. }) if key == "server.listen"),
            "listen = {bad}"
        );
    }
    let unknown = config_file("server-unknown", "[server]\nport = 1\n");
    assert!(
        matches!(load(&unknown, &home()), Err(ConfigError::UnknownKey(k)) if k == "server.port")
    );
    let not_table = config_file("server-not-table", "server = 1\n");
    assert!(
        matches!(load(&not_table, &home()), Err(ConfigError::Invalid { key, .. }) if key == "server")
    );
}
