//! The pool, the trusted module registry, the dedicated session and the owner connection
//! (CONTRACTS §3; S1 §9).
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use cortex_types::{Clock, ModuleId, Secret};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{Connection, PgConnection};

use crate::error::DbError;
use crate::tx::AdvisoryClass;

/// How long taking a pool connection may wait before the database counts as unavailable.
const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(10);
/// sqlx's default statement-cache size, when the cache is on.
const STATEMENT_CACHE: usize = 100;
/// The cache "off" (F1-06's control). sqlx 0.9 gives every persistent statement a server-side name
/// whatever the capacity, and with capacity 0 it never closes one (sqlx-postgres 0.9.0,
/// `connection/executor.rs`: `prepare` names it, and `get_or_prepare` caches, so later closes, only
/// `if persistent && cache_statement.is_enabled()`): capacity 0 would leak a prepared statement per
/// query text on the server. Capacity 1 is the smallest bounded cache: the next prepare closes the
/// previous statement, so at most one is ever retained.
const STATEMENT_CACHE_OFF: usize = 1;

fn connect_options(
    endpoint: &DbEndpoint,
    database: &str,
    user: &str,
    password: &Secret,
    cache: bool,
) -> PgConnectOptions {
    let o = PgConnectOptions::new()
        .username(user)
        .password(password.expose())
        .database(database)
        .statement_cache_capacity(if cache {
            STATEMENT_CACHE
        } else {
            STATEMENT_CACHE_OFF
        });
    match endpoint {
        DbEndpoint::Socket { dir, port } => o.socket(dir).port(*port),
        DbEndpoint::Tcp { host, port } => o.host(host).port(*port),
    }
}

/// The role a module runs as: `cortex_m_<module>`, only for a registered module. `ModuleId`'s
/// grammar (`^[a-z][a-z0-9_]{0,31}$`) makes the name match `^cortex_m_[a-z][a-z0-9_]{0,31}$`.
pub(crate) fn module_role(registry: &ModuleRegistry, module: &ModuleId) -> Result<String, DbError> {
    if registry.contains(module) {
        Ok(format!("cortex_m_{}", module.as_str()))
    } else {
        Err(DbError::UnknownModule)
    }
}

/// Under the `testkit` feature, a new session's identity token (F1-06's reuse oracle).
#[cfg(feature = "testkit")]
async fn set_session_token(conn: &mut PgConnection) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT set_config($1, gen_random_uuid()::text, false)")
        .bind(crate::testkit::SESSION_TOKEN_GUC)
        .execute(conn)
        .await
        .map(|_| ())
}

fn pool_options(cfg: &DbConfig) -> PgPoolOptions {
    let o = PgPoolOptions::new()
        .max_connections(cfg.max_connections)
        .acquire_timeout(ACQUIRE_TIMEOUT);
    #[cfg(feature = "testkit")]
    let o = o.after_connect(|conn, _meta| Box::pin(async move { set_session_token(conn).await }));
    o
}

fn app_options(cfg: &DbConfig) -> PgConnectOptions {
    connect_options(
        &cfg.endpoint,
        &cfg.database,
        &cfg.app_user,
        &cfg.app_password,
        cfg.statement_cache,
    )
}

/// Where Postgres listens: the Unix socket in production (S1 §9), loopback TCP in tests until
/// Z-1 reports (LAYOUT §7).
#[derive(Debug, Clone)]
pub enum DbEndpoint {
    Socket { dir: PathBuf, port: u16 },
    Tcp { host: String, port: u16 },
}

/// The modules this server runs, from its manifests (F-1c). The only source of a module role.
#[derive(Debug, Clone, Default)]
pub struct ModuleRegistry(BTreeSet<ModuleId>);

impl ModuleRegistry {
    pub fn new(ids: impl IntoIterator<Item = ModuleId>) -> Self {
        Self(ids.into_iter().collect())
    }
    pub fn contains(&self, id: &ModuleId) -> bool {
        self.0.contains(id)
    }
}

pub struct DbConfig {
    pub endpoint: DbEndpoint,
    pub database: String,
    /// `cortex_app` (S1 §9): `NOINHERIT`, a member of every module role.
    pub app_user: String,
    pub app_password: Secret,
    pub max_connections: u32,
    /// sqlx's prepared-statement cache. On in production; F1-06's cache control turns it off.
    pub statement_cache: bool,
    pub registry: ModuleRegistry,
    /// `platform.now()` reads this clock (TestClock in tests; F1-16).
    pub clock: Arc<dyn Clock>,
}

/// The `cortex_app` pool.
pub struct DbPool {
    pub(crate) pool: sqlx::PgPool,
    pub(crate) registry: ModuleRegistry,
    pub(crate) clock: Arc<dyn Clock>,
    /// The pool's connection options, for a dedicated session outside the pool.
    pub(crate) options: Arc<PgConnectOptions>,
}

impl DbPool {
    /// Connects, and checks one connection. Under the `testkit` feature, every new connection
    /// first sets `testkit::SESSION_TOKEN_GUC` to a fresh random UUID with session scope (F1-06).
    pub async fn connect(cfg: &DbConfig) -> Result<DbPool, DbError> {
        let pool = pool_options(cfg)
            .connect_with(app_options(cfg))
            .await
            .map_err(DbError::unavailable)?;
        sqlx::query("SELECT 1")
            .execute(&pool)
            .await
            .map_err(DbError::unavailable)?;
        Ok(DbPool {
            pool,
            registry: cfg.registry.clone(),
            clock: Arc::clone(&cfg.clock),
            options: Arc::new(app_options(cfg)),
        })
    }

    /// Connects on first use only (F1-06's dead-pool control uses it). The same session token
    /// under the `testkit` feature.
    pub fn connect_lazy(cfg: &DbConfig) -> DbPool {
        DbPool {
            pool: pool_options(cfg).connect_lazy_with(app_options(cfg)),
            registry: cfg.registry.clone(),
            clock: Arc::clone(&cfg.clock),
            options: Arc::new(app_options(cfg)),
        }
    }

    /// nemo's F1-08: a connection outside the pool, under the module's role at session level,
    /// for a long-held advisory lock (the sequencer). Dropping it ends the session.
    pub async fn dedicated_session(&self, module: &ModuleId) -> Result<Session, DbError> {
        let role = module_role(&self.registry, module)?;
        let mut conn = PgConnection::connect_with(&self.options)
            .await
            .map_err(DbError::unavailable)?;
        #[cfg(feature = "testkit")]
        set_session_token(&mut conn)
            .await
            .map_err(DbError::from_sqlx)?;
        sqlx::query("SELECT set_config('role', $1, false)")
            .bind(&role)
            .execute(&mut conn)
            .await
            .map_err(DbError::from_sqlx)?;
        Ok(Session { conn })
    }
}

/// A dedicated session under one module's role.
pub struct Session {
    pub(crate) conn: sqlx::PgConnection,
}

impl Session {
    pub fn conn(&mut self) -> &mut sqlx::PgConnection {
        &mut self.conn
    }
    /// `pg_advisory_lock(class, key)` at session level; held until the session ends.
    pub async fn advisory_lock(&mut self, class: AdvisoryClass, key: i32) -> Result<(), DbError> {
        sqlx::query("SELECT pg_advisory_lock($1, $2)")
            .bind(class as i32)
            .bind(key)
            .execute(&mut self.conn)
            .await
            .map(|_| ())
            .map_err(DbError::from_sqlx)
    }
}

pub struct OwnerConfig {
    pub endpoint: DbEndpoint,
    pub database: String,
    /// `cortex_owner`: owns the schemas; runs migrations; never logs in at run time otherwise.
    pub owner_user: String,
    pub owner_password: Secret,
}

/// A `cortex_owner` connection, for migrations and startup checks only.
pub struct OwnerConn {
    pub(crate) conn: sqlx::PgConnection,
}

/// Why [`OwnerConn::connect_classified`] did not return a connection (DEFECT S, cox 7615a247): the one thing a caller may act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectFailure {
    /// The database, or the way to it, is not ready yet: it is starting up, or nothing answers on the address yet. Worth waiting for.
    NotReady,
    /// Anything else (a refused login, a missing role or database, a TLS or protocol error, a bad address): never worth a retry.
    Refused,
}

/// Is this connect error "the database, or the way to it, is not ready yet"? A POSITIVE allowlist (sam MWD-SAM-03): everything not named here is refused.
///
/// - `Error::Database` with SQLSTATE 57P03 (`cannot_connect_now`: starting up, shutting down, in recovery). NOT the whole class 08 (it holds protocol
///   violations), NOT 28xxx (authentication), 3D000 (no such database), 42501, and NOT 53300 (too many connections: the database is up and answering;
///   waiting would not recover a saturated server and could hide a leak, and it was not in the list of what is worth a wait).
/// - `Error::Io` of the kinds a peer that is not there yet produces: connection refused, reset, aborted, broken pipe, unexpected EOF, a socket file
///   that does not exist yet (NotFound) and a connect that timed out. `Other`, out of memory, a permission, a bad address and every kind not named
///   (the enum is non-exhaustive) are refused: `Other` does not establish a host-name failure. A host name is resolved by the CALLER's own lookup, which
///   identifies that stage by construction, so nothing here has to guess it.
/// - `Tls`, `Protocol`, `Configuration` and every other variant: refused.
pub(crate) fn not_ready(e: &sqlx::Error) -> bool {
    use std::io::ErrorKind as K;
    match e {
        sqlx::Error::Database(d) => d.code().as_deref() == Some("57P03"),
        sqlx::Error::Io(io) => matches!(
            io.kind(),
            K::ConnectionRefused
                | K::ConnectionReset
                | K::ConnectionAborted
                | K::BrokenPipe
                | K::UnexpectedEof
                | K::NotFound
                | K::TimedOut
        ),
        _ => false,
    }
}

impl OwnerConn {
    fn owner_options(cfg: &OwnerConfig) -> PgConnectOptions {
        connect_options(
            &cfg.endpoint,
            &cfg.database,
            &cfg.owner_user,
            &cfg.owner_password,
            true,
        )
    }

    pub async fn connect(cfg: &OwnerConfig) -> Result<OwnerConn, DbError> {
        let options = Self::owner_options(cfg);
        let conn = PgConnection::connect_with(&options)
            .await
            .map_err(DbError::unavailable)?;
        Ok(OwnerConn { conn })
    }

    /// [`OwnerConn::connect`] with the reason kept, for the one caller that waits for a database that is not ready (`cortex-server migrate`).
    /// `connect` itself is unchanged: every other caller still sees `DbError::Unavailable` and nothing else.
    pub async fn connect_classified(cfg: &OwnerConfig) -> Result<OwnerConn, ConnectFailure> {
        let options = Self::owner_options(cfg);
        match PgConnection::connect_with(&options).await {
            Ok(conn) => Ok(OwnerConn { conn }),
            Err(e) if not_ready(&e) => Err(ConnectFailure::NotReady),
            Err(_) => Err(ConnectFailure::Refused),
        }
    }
    pub fn conn(&mut self) -> &mut sqlx::PgConnection {
        &mut self.conn
    }
}

#[cfg(test)]
mod w1_host_role_tests {
    //! Wave 1 ports (d), W1-11: NO module id can reach the host role. DB-free.
    use super::*;
    use crate::tx::HOST_ROLE;

    /// Every id of one or two characters over the id alphabet that `ModuleId` accepts, and some that are
    /// adversarial on purpose (the host role's own words, role prefixes, a platform-looking name).
    fn ids() -> Vec<ModuleId> {
        let alphabet: Vec<char> = ('a'..='z').chain('0'..='9').chain(['_']).collect();
        let mut raw: Vec<String> = alphabet.iter().map(|c| c.to_string()).collect();
        for a in &alphabet {
            for b in &alphabet {
                raw.push(format!("{a}{b}"));
            }
        }
        for extra in [
            "install_admin",
            "cortex_install_admin",
            "m_install_admin",
            "admin",
            "install",
            "platform",
            "owner",
            "cortex_owner",
            "app",
            "host",
            "host_operator",
            "recovery",
            "system",
            "a_very_long_module_id_of_thirty_1",
        ] {
            raw.push(extra.to_owned());
        }
        raw.iter().filter_map(|s| ModuleId::parse(s).ok()).collect()
    }

    #[test]
    fn w1_h_1_a_module_role_is_always_cortex_m_prefixed_and_never_the_host_role() {
        let all = ids();
        assert!(all.len() > 1000, "the battery is real: {} ids", all.len());
        let registry = ModuleRegistry::new(all.clone());
        for id in &all {
            let role = module_role(&registry, id).expect("a registered id has a role");
            assert_eq!(role, format!("cortex_m_{}", id.as_str()));
            assert_ne!(role, HOST_ROLE, "{id:?}");
            assert!(role.starts_with("cortex_m_"), "{role}");
        }
        // The host role has no `cortex_m_` prefix, so no id can spell it.
        assert!(!HOST_ROLE.starts_with("cortex_m_"));
        // And an id that is not registered has no role at all.
        let empty = ModuleRegistry::default();
        assert_eq!(
            module_role(&empty, &ModuleId::parse("install_admin").expect("id")),
            Err(DbError::UnknownModule)
        );
    }

    #[test]
    fn w1_h_2_the_host_role_name_is_written_once_in_this_crates_source() {
        // The literal, quoted, in every source file of cortex-db: exactly once (the const), so the only
        // way to the role is `ModuleTx::begin_host`. A test file is not source.
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut hits = Vec::new();
        for entry in std::fs::read_dir(&src).expect("src") {
            let path = entry.expect("entry").path();
            if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).expect("read");
                // Only the non-test part: stop at the first test module.
                let code = text.split("#[cfg(test)]").next().unwrap_or_default();
                let n = code.matches("\"cortex_install_admin\"").count();
                if n > 0 {
                    hits.push((
                        path.file_name()
                            .expect("name")
                            .to_string_lossy()
                            .into_owned(),
                        n,
                    ));
                }
            }
        }
        assert_eq!(hits, [("tx.rs".to_owned(), 1)], "the one const, in tx.rs");
    }
}

#[cfg(test)]
mod defect_s_connect_class_tests {
    //! DEFECT S (cox 7615a247): which connect errors are "not ready yet" (worth a wait) and which never are. DB-free: the errors are built, not provoked.
    use super::*;
    use sqlx::error::{DatabaseError, ErrorKind};
    use std::io::{Error as IoError, ErrorKind as Io};

    /// A server error with a SQLSTATE, as the Postgres driver hands one up.
    #[derive(Debug)]
    struct Fake(&'static str);

    impl std::fmt::Display for Fake {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "fake {}", self.0)
        }
    }
    impl std::error::Error for Fake {}
    impl DatabaseError for Fake {
        fn message(&self) -> &str {
            "fake"
        }
        fn code(&self) -> Option<std::borrow::Cow<'_, str>> {
            Some(self.0.into())
        }
        fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
            self
        }
        fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
            self
        }
        fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
            self
        }
        fn kind(&self) -> ErrorKind {
            ErrorKind::Other
        }
    }

    fn db(code: &'static str) -> sqlx::Error {
        sqlx::Error::Database(Box::new(Fake(code)))
    }
    fn io(kind: Io) -> sqlx::Error {
        sqlx::Error::Io(IoError::from(kind))
    }

    #[test]
    fn cs_01_a_database_that_is_starting_up_is_not_ready() {
        assert!(not_ready(&db("57P03")));
    }

    #[test]
    fn cs_02_an_address_that_does_not_answer_yet_is_not_ready() {
        for kind in [
            Io::ConnectionRefused,
            Io::ConnectionReset,
            Io::ConnectionAborted,
            Io::BrokenPipe,
            Io::UnexpectedEof,
            Io::NotFound,
            Io::TimedOut,
        ] {
            assert!(not_ready(&io(kind)), "{kind:?}");
        }
    }

    #[test]
    fn cs_03_a_refused_login_a_missing_role_or_database_or_a_permission_is_never_waited_for() {
        // the class 08 (connection exceptions) includes protocol violations: never blanket-retried (sam, MWD answer 1)
        for code in [
            "28P01", "28000", "3D000", "42501", "53300", "42P01", "23505", "08000", "08001",
            "08006", "08P01",
        ] {
            assert!(!not_ready(&db(code)), "{code}");
        }
    }

    #[test]
    fn cs_04_a_tls_protocol_or_configuration_error_is_never_waited_for() {
        assert!(!not_ready(&sqlx::Error::Tls("tls".into())));
        assert!(!not_ready(&sqlx::Error::Protocol("protocol".into())));
        assert!(!not_ready(&sqlx::Error::Configuration("config".into())));
    }

    #[test]
    fn cs_05_an_io_error_that_is_not_on_the_allowlist_is_never_waited_for() {
        // a POSITIVE allowlist (sam MWD-SAM-03): `Other` does not establish a host-name failure, and an unknown or new kind is refused
        for kind in [
            Io::PermissionDenied,
            Io::InvalidInput,
            Io::InvalidData,
            Io::Unsupported,
            Io::Other,
            Io::OutOfMemory,
            Io::AddrNotAvailable,
            Io::AddrInUse,
            Io::WouldBlock,
            Io::Interrupted,
            Io::WriteZero,
        ] {
            assert!(!not_ready(&io(kind)), "{kind:?}");
        }
    }

    fn dead_cfg() -> (tokio::net::TcpSocket, OwnerConfig) {
        // nobody listens: the connection is refused
        let socket = tokio::net::TcpSocket::new_v4().expect("a socket");
        socket
            .bind("127.0.0.1:0".parse().expect("addr"))
            .expect("bind");
        let port = socket.local_addr().expect("addr").port();
        let cfg = OwnerConfig {
            endpoint: DbEndpoint::Tcp {
                host: "127.0.0.1".into(),
                port,
            },
            database: "cortex".into(),
            owner_user: "cortex_owner".into(),
            owner_password: Secret::new("x".to_owned()),
        };
        (socket, cfg)
    }

    /// cox 7615a247: `connect` ITSELF is unchanged: every failure is `DbError::Unavailable` and nothing else, so no other caller can change by accident.
    #[tokio::test]
    async fn cs_06_connect_itself_still_maps_every_error_to_unavailable() {
        let (_held, cfg) = dead_cfg();
        assert_eq!(
            OwnerConn::connect(&cfg).await.err(),
            Some(DbError::Unavailable)
        );
    }

    #[tokio::test]
    async fn cs_07_the_classified_connect_says_not_ready_for_an_address_nobody_listens_on() {
        let (_held, cfg) = dead_cfg();
        assert_eq!(
            OwnerConn::connect_classified(&cfg).await.err(),
            Some(ConnectFailure::NotReady)
        );
    }
}
