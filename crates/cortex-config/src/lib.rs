//! `cortex-config`: the config file plus the environment, and every path from one place
//! (CONTRACTS §2; S1 §2.1 and §4.5; P1 §2 for the defaults; TESTS F1-05).
//!
//! The file is TOML. Known keys (r1 of this contract):
//!
//! ```toml
//! [paths]
//! state_dir = "/var/lib/cortex"            # P1 §2
//! host_socket = "/run/cortex/admin.sock"   # S1 §7.1
//! worker_socket = "/run/cortex/api.sock"   # P1 §2, S1 §4.5
//!
//! [modules]
//! enabled = ["audit", "identity", "tokens"]
//!
//! [database]                               # Wave 0 S1 (socket contract r4 §2)
//! host = "/run/postgresql"                 # an absolute path is the socket directory
//! port = 5432
//! name = "cortex"
//! app_user = "cortex_app"
//! app_password_file = "/etc/cortex/db/app.password"
//! owner_user = "cortex_owner"                    # migrate only; both owner keys or neither
//! owner_password_file = "/etc/cortex/db/owner.password"
//!
//! [server]                                 # Wave 0 S1 (piece 11 contract r0 section 2)
//! listen = "127.0.0.1:8601"                # an IP and a port; 0.0.0.0:<port> in the container
//! ```
//!
//! `[database]` holds names and file paths, never a secret value, and the loader reads no file.
//! It is optional here, so a config without it loads with `database: None`; `cortex-server`
//! refuses to start without it. `cortex-server serve` needs no owner key and ignores them; only
//! `migrate` asks for them, through [`Database::owner_for_migrate`] (piece 11 contract r0).
//!
//! The token directory is not a file key: it is `$XDG_CONFIG_HOME/cortex` when that variable is
//! set and not empty, else `$HOME/.config/cortex` (S1 §4.5). An unknown key, in any table, stops
//! startup and names the key (F1-05; my proposal in CONTRACTS §2). Every path must be absolute.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cortex_types::ModuleId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub paths: Paths,
    pub modules: ModuleList,
    /// `[database]`, when the file has the table (Wave 0 S1).
    pub database: Option<Database>,
    /// `[server]`; its defaults when the file has no such table.
    pub server: Server,
}

/// The server's listener (piece 11 contract r0 section 2): TCP. A recorded deviation from S1
/// section 4.5's worker socket, which returns with the packaging release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Server {
    pub listen: std::net::SocketAddr,
}

/// `server.listen` when the file does not set it: loopback only.
pub const DEFAULT_LISTEN: &str = "127.0.0.1:8601";

impl Default for Server {
    fn default() -> Self {
        Server {
            listen: DEFAULT_LISTEN
                .parse()
                .expect("the default listen address parses"),
        }
    }
}

/// Where the database is and who connects (`[database]`): names and file paths only. The server
/// reads each password file into a `Secret` for `cortex-db`'s `DbConfig` and `OwnerConfig`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Database {
    pub host: DbHost,
    pub port: u16,
    /// The database name.
    pub name: String,
    /// `cortex_app` (S1 §9).
    pub app_user: String,
    pub app_password_file: PathBuf,
    /// `owner_user` and `owner_password_file`: both or neither. Only `cortex-server migrate` uses
    /// them ([`Database::owner_for_migrate`]); `serve` ignores them.
    pub owner: Option<DbOwner>,
}

/// `cortex_owner` (S1 §9): migrations and startup checks only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbOwner {
    pub user: String,
    pub password_file: PathBuf,
}

impl Database {
    /// For `cortex-server migrate`: the owner keys, or `Missing` naming the first absent one.
    pub fn owner_for_migrate(&self) -> Result<&DbOwner, ConfigError> {
        self.owner
            .as_ref()
            .ok_or_else(|| ConfigError::Missing("database.owner_user".to_owned()))
    }
}

/// `host`, by libpq's rule: an absolute path is the Unix socket directory, anything else a TCP
/// host name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DbHost {
    SocketDir(PathBuf),
    Tcp(String),
}

/// Every path Cortex uses, resolved in one place (S1 §4.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub state_dir: PathBuf,
    pub host_socket: PathBuf,
    pub worker_socket: PathBuf,
    pub token_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModuleList {
    pub enabled: Vec<ModuleId>,
}

/// The defaults, in one place: P1 §2 and S1 §7.1.
pub const DEFAULT_STATE_DIR: &str = "/var/lib/cortex";
pub const DEFAULT_HOST_SOCKET: &str = "/run/cortex/admin.sock";
pub const DEFAULT_WORKER_SOCKET: &str = "/run/cortex/api.sock";

/// The environment variables Cortex reads: the only keys `Env` captures (r2; mike's F1A-R0-01,
/// kai's ruling in row 82054725). A later slice that needs another key adds it here, by a
/// reviewed contract change.
pub const ENV_KEYS: &[&str] = &["HOME", "XDG_CONFIG_HOME"];

/// The environment, passed in rather than read from the process, so that tests are hermetic.
///
/// r2 contract (F1A-R0-01): `Env` holds only the `ENV_KEYS`; `get` is the one way to read a value;
/// its `Debug` names the keys it holds and prints every value as `<redacted>`; it has no iterator
/// over values.
#[derive(Clone, Default)]
pub struct Env(BTreeMap<&'static str, String>);

impl Env {
    /// Keeps only the pairs whose key is in `ENV_KEYS`.
    pub fn from_pairs(pairs: &[(&str, &str)]) -> Self {
        let mut m = BTreeMap::new();
        for (k, v) in pairs {
            if let Some(key) = ENV_KEYS.iter().find(|n| **n == *k) {
                m.insert(*key, (*v).to_owned());
            }
        }
        Self(m)
    }

    /// Reads each of `ENV_KEYS` from the process by name, and nothing else. A value that is not
    /// valid UTF-8 is left out, so a later `load` fails closed with `Missing`.
    pub fn from_process() -> Self {
        let mut m = BTreeMap::new();
        for key in ENV_KEYS {
            if let Some(v) = std::env::var_os(key).and_then(|v| v.into_string().ok()) {
                m.insert(*key, v);
            }
        }
        Self(m)
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).map(String::as_str)
    }
}

/// Names the keys; every value is `<redacted>`.
impl std::fmt::Debug for Env {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        struct Redacted;
        impl std::fmt::Debug for Redacted {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("<redacted>")
            }
        }
        let mut m = f.debug_map();
        for key in self.0.keys() {
            m.entry(key, &Redacted);
        }
        m.finish()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("unknown config key `{0}`")]
    UnknownKey(String),
    #[error("missing: {0}")]
    Missing(String),
    #[error("invalid value for `{key}`: {reason}")]
    Invalid { key: String, reason: String },
    #[error("cannot read the config file: {0}")]
    Io(#[from] std::io::Error),
}

fn invalid(key: &str, reason: impl Into<String>) -> ConfigError {
    ConfigError::Invalid {
        key: key.to_owned(),
        reason: reason.into(),
    }
}

/// An absolute path from a file value.
fn abs_path(key: &str, v: &toml::Value) -> Result<PathBuf, ConfigError> {
    let s = v.as_str().ok_or_else(|| invalid(key, "must be a string"))?;
    let p = PathBuf::from(s);
    if p.is_absolute() {
        Ok(p)
    } else {
        Err(invalid(key, "must be an absolute path"))
    }
}

/// The `[database]` keys, in the order a missing one is reported.
const DATABASE_KEYS: [&str; 7] = [
    "host",
    "port",
    "name",
    "app_user",
    "app_password_file",
    "owner_user",
    "owner_password_file",
];

/// `[database]`: every key required, an unknown key named, no value repeated in an error.
fn database_table(value: &toml::Value) -> Result<Database, ConfigError> {
    let table = value
        .as_table()
        .ok_or_else(|| invalid("database", "must be a table"))?;
    // Unknown keys first: a secret value under any other name (`app_password`) stops startup
    // naming only the key.
    if let Some(k) = table.keys().find(|k| !DATABASE_KEYS.contains(&k.as_str())) {
        return Err(ConfigError::UnknownKey(format!("database.{k}")));
    }
    let get = |k: &str| {
        let key = format!("database.{k}");
        match table.get(k) {
            Some(v) => Ok((key, v)),
            None => Err(ConfigError::Missing(key)),
        }
    };
    let (key, v) = get("host")?;
    let host = db_host(&key, v)?;
    let (key, v) = get("port")?;
    let port = db_port(&key, v)?;
    let (key, v) = get("name")?;
    let name = pg_name(&key, v)?;
    let (key, v) = get("app_user")?;
    let app_user = pg_name(&key, v)?;
    let (key, v) = get("app_password_file")?;
    let app_password_file = abs_path(&key, v)?;
    // The owner keys: both or neither (serve needs neither). When present they are checked like
    // the others; no file is read.
    let owner = match (
        table.contains_key("owner_user"),
        table.contains_key("owner_password_file"),
    ) {
        (false, false) => None,
        _ => {
            let (key, v) = get("owner_user")?;
            let user = pg_name(&key, v)?;
            let (key, v) = get("owner_password_file")?;
            let password_file = abs_path(&key, v)?;
            Some(DbOwner {
                user,
                password_file,
            })
        }
    };
    Ok(Database {
        host,
        port,
        name,
        app_user,
        app_password_file,
        owner,
    })
}

/// `[server]`.
fn server_table(value: &toml::Value) -> Result<Server, ConfigError> {
    let table = value
        .as_table()
        .ok_or_else(|| invalid("server", "must be a table"))?;
    let mut server = Server::default();
    for (k, v) in table {
        let key = format!("server.{k}");
        match k.as_str() {
            "listen" => server.listen = listen_addr(&key, v)?,
            _ => return Err(ConfigError::UnknownKey(key)),
        }
    }
    Ok(server)
}

/// `server.listen`: an IP address and a port from 1 to 65535; no host name.
fn listen_addr(key: &str, v: &toml::Value) -> Result<std::net::SocketAddr, ConfigError> {
    let s = v.as_str().ok_or_else(|| invalid(key, "must be a string"))?;
    match s.parse::<std::net::SocketAddr>() {
        Ok(a) if a.port() != 0 => Ok(a),
        _ => Err(invalid(
            key,
            "must be an IP address and a port from 1 to 65535, such as 127.0.0.1:8601",
        )),
    }
}

/// `database.host`: an absolute path is the socket directory, anything else a TCP host name;
/// never empty, no whitespace or control character.
fn db_host(key: &str, v: &toml::Value) -> Result<DbHost, ConfigError> {
    let s = v.as_str().ok_or_else(|| invalid(key, "must be a string"))?;
    if s.is_empty() || s.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(invalid(
            key,
            "must be a socket directory or a host name, with no whitespace or control character",
        ));
    }
    let p = Path::new(s);
    Ok(if p.is_absolute() {
        DbHost::SocketDir(p.to_path_buf())
    } else {
        DbHost::Tcp(s.to_owned())
    })
}

/// `database.port`: an integer from 1 to 65535.
fn db_port(key: &str, v: &toml::Value) -> Result<u16, ConfigError> {
    let n = v
        .as_integer()
        .ok_or_else(|| invalid(key, "must be an integer"))?;
    u16::try_from(n)
        .ok()
        .filter(|p| *p != 0)
        .ok_or_else(|| invalid(key, "must be from 1 to 65535"))
}

/// A database or role name: a Postgres identifier, 1 to 63 bytes, no NUL.
fn pg_name(key: &str, v: &toml::Value) -> Result<String, ConfigError> {
    let s = v.as_str().ok_or_else(|| invalid(key, "must be a string"))?;
    if s.is_empty() || s.len() > 63 || s.contains('\0') {
        Err(invalid(key, "must be 1 to 63 bytes, with no NUL"))
    } else {
        Ok(s.to_owned())
    }
}

/// Reads the config file and the environment into a `Config`. Fails closed on any unknown key.
pub fn load(file: &Path, env: &Env) -> Result<Config, ConfigError> {
    let text = std::fs::read_to_string(file)?;
    let doc: toml::Table = text
        .parse()
        .map_err(|e: toml::de::Error| invalid("(file)", e.message().to_owned()))?;

    let mut paths = Paths {
        state_dir: PathBuf::from(DEFAULT_STATE_DIR),
        host_socket: PathBuf::from(DEFAULT_HOST_SOCKET),
        worker_socket: PathBuf::from(DEFAULT_WORKER_SOCKET),
        token_dir: token_dir(env)?,
    };
    let mut modules = ModuleList::default();
    let mut database = None;
    let mut server = Server::default();

    for (top, value) in &doc {
        match top.as_str() {
            "paths" => {
                let table = value
                    .as_table()
                    .ok_or_else(|| invalid("paths", "must be a table"))?;
                for (k, v) in table {
                    let key = format!("paths.{k}");
                    match k.as_str() {
                        "state_dir" => paths.state_dir = abs_path(&key, v)?,
                        "host_socket" => paths.host_socket = abs_path(&key, v)?,
                        "worker_socket" => paths.worker_socket = abs_path(&key, v)?,
                        _ => return Err(ConfigError::UnknownKey(key)),
                    }
                }
            }
            "modules" => {
                let table = value
                    .as_table()
                    .ok_or_else(|| invalid("modules", "must be a table"))?;
                for (k, v) in table {
                    let key = format!("modules.{k}");
                    match k.as_str() {
                        "enabled" => {
                            let list = v
                                .as_array()
                                .ok_or_else(|| invalid(&key, "must be an array"))?;
                            modules.enabled = list
                                .iter()
                                .map(|m| {
                                    let s = m
                                        .as_str()
                                        .ok_or_else(|| invalid(&key, "must hold strings"))?;
                                    ModuleId::parse(s).map_err(|e| invalid(&key, e.to_string()))
                                })
                                .collect::<Result<_, _>>()?;
                        }
                        _ => return Err(ConfigError::UnknownKey(key)),
                    }
                }
            }
            "database" => database = Some(database_table(value)?),
            "server" => server = server_table(value)?,
            _ => return Err(ConfigError::UnknownKey(top.clone())),
        }
    }
    Ok(Config {
        paths,
        modules,
        database,
        server,
    })
}

/// The token directory (S1 §4.5): `$XDG_CONFIG_HOME/cortex` if that is set, not empty and
/// absolute, else `$HOME/.config/cortex`. The one function every crate uses for it
/// (cortex-client included). r3 (mike's F1A-R2-02): the result is always absolute. A relative
/// `XDG_CONFIG_HOME` is invalid and ignored, as the XDG base directory rule says; a relative `HOME`
/// is an error, never a path relative to the working directory.
pub fn token_dir(env: &Env) -> Result<PathBuf, ConfigError> {
    if let Some(x) = env
        .get("XDG_CONFIG_HOME")
        .filter(|x| !x.is_empty() && Path::new(x).is_absolute())
    {
        return Ok(PathBuf::from(x).join("cortex"));
    }
    match env.get("HOME") {
        Some(h) if !h.is_empty() => {
            let home = PathBuf::from(h);
            if home.is_absolute() {
                Ok(home.join(".config").join("cortex"))
            } else {
                Err(invalid("HOME", "must be an absolute path"))
            }
        }
        _ => Err(ConfigError::Missing("HOME".to_owned())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(name: &str, body: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cortex-config-unit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("tmp dir");
        let p = dir.join(name);
        std::fs::write(&p, body).expect("write");
        p
    }

    fn home() -> Env {
        Env::from_pairs(&[("HOME", "/home/w")])
    }

    #[test]
    fn a_relative_path_is_refused() {
        let p = write("rel.toml", "[paths]\nstate_dir = \"var/lib\"\n");
        assert!(
            matches!(load(&p, &home()), Err(ConfigError::Invalid { key, .. }) if key == "paths.state_dir")
        );
    }

    #[test]
    fn modules_are_validated_ids() {
        let ok = write(
            "mods.toml",
            "[modules]\nenabled = [\"audit\", \"identity\"]\n",
        );
        let cfg = load(&ok, &home()).expect("loads");
        assert_eq!(cfg.modules.enabled.len(), 2);
        let bad = write("badmod.toml", "[modules]\nenabled = [\"Audit\"]\n");
        assert!(
            matches!(load(&bad, &home()), Err(ConfigError::Invalid { key, .. }) if key == "modules.enabled")
        );
        let unknown = write("unkmod.toml", "[modules]\ndisabled = []\n");
        assert!(
            matches!(load(&unknown, &home()), Err(ConfigError::UnknownKey(k)) if k == "modules.disabled")
        );
    }

    #[test]
    fn a_missing_file_is_an_io_error() {
        let p = std::env::temp_dir().join("cortex-config-unit-does-not-exist.toml");
        assert!(matches!(load(&p, &home()), Err(ConfigError::Io(_))));
    }

    #[test]
    fn a_wrong_type_is_invalid() {
        let p = write("type.toml", "paths = 3\n");
        assert!(
            matches!(load(&p, &home()), Err(ConfigError::Invalid { key, .. }) if key == "paths")
        );
    }
}
