//! The per-binary test database on the supervisor's Postgres (L-9), and small test helpers.
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use cortex_db::{Actor, DbConfig, DbEndpoint, ModuleRegistry, OwnerConfig, TxContext};
use cortex_types::{Clock, ModuleId, ProjectId, RequestId, Secret, SystemClock};
use sqlx::postgres::PgConnectOptions;
use sqlx::{Connection, PgConnection};
use tokio::sync::OnceCell;

/// The roles of S1 §9 in miniature, cluster-wide, created idempotently. `cortex_app` is
/// `NOINHERIT` and a member of every module role; `cortex_app_rr` is the same with REPEATABLE READ
/// as its default (F1-32's isolation case); `cortex_base` holds both test grants with no switching
/// (Z2-4's positive control). `cortex_install_admin` (NOLOGIN) is the host-path role of
/// `ModuleTx::begin_host` (W1-11), granted to the same two. Nothing has SUPERUSER or BYPASSRLS.
const ROLES: &str = r#"
DO $$ BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cortex_owner')        THEN CREATE ROLE cortex_owner LOGIN; END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cortex_app')          THEN CREATE ROLE cortex_app LOGIN NOINHERIT; END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cortex_app_rr')       THEN CREATE ROLE cortex_app_rr LOGIN NOINHERIT; END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cortex_base')         THEN CREATE ROLE cortex_base LOGIN; END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cortex_m_test_a')     THEN CREATE ROLE cortex_m_test_a NOLOGIN; END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cortex_m_test_b')     THEN CREATE ROLE cortex_m_test_b NOLOGIN; END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cortex_m_test_audit') THEN CREATE ROLE cortex_m_test_audit NOLOGIN; END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cortex_m_test_m')     THEN CREATE ROLE cortex_m_test_m NOLOGIN; END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cortex_m_platform')   THEN CREATE ROLE cortex_m_platform NOLOGIN; END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cortex_m_test_ret')   THEN CREATE ROLE cortex_m_test_ret NOLOGIN; END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cortex_install_admin') THEN CREATE ROLE cortex_install_admin NOLOGIN; END IF;
END $$;
GRANT cortex_m_test_a, cortex_m_test_b, cortex_m_test_audit, cortex_m_test_m, cortex_m_platform, cortex_m_test_ret TO cortex_app, cortex_app_rr;
GRANT cortex_install_admin TO cortex_app, cortex_app_rr;
ALTER ROLE cortex_app_rr SET default_transaction_isolation = 'repeatable read';
"#;

/// The `cortex_m_<piece>` roles that the fixtures of this workspace create themselves: their pieces' `BOOTSTRAP_SQL` (and the audit, identity and
/// probe fixtures), and `cortex-server`'s `serve_real`, which creates one per compiled piece at run time (`cortex_m_{id}`). Cox 692bf752: the
/// platform migration grants to every `cortex_m_*` role of the CLUSTER that exists at its loop and re-reads `pg_roles` in its read-back; a
/// role that ANOTHER test creates (and commits) in between makes the read-back raise (P0001). Tests of one binary run in parallel threads
/// against one cluster, so only the FIRST creation of a role in a cluster can land in a sibling's window. `create_database_with` creates
/// all of these, under the cluster-wide role-setup lock, BEFORE any test has a database: afterwards every `BOOTSTRAP_SQL` is a no-op.
/// NOT here, on purpose: `cortex_m_audit_race` (mod-audit's `audit_bootstrap_race` asserts that its control role does not exist).
/// `tests/w1_piece_roles.rs` (pr_02, pr_03) refuses a role of the workspace that is missing from this list.
pub const PIECE_ROLES: &[&str] = &[
    "cortex_m_audit",
    "cortex_m_boot",
    "cortex_m_fx_audit",
    "cortex_m_handoffs",
    "cortex_m_identity",
    "cortex_m_plug_audit",
    "cortex_m_plug_identity",
    "cortex_m_policy",
    "cortex_m_probe",
    "cortex_m_records",
    "cortex_m_search",
    "cortex_m_tokens",
];

/// One `DO` block that creates every [`PIECE_ROLES`] role, in the form of the pieces' own bootstraps: an existing role, or one created at
/// the same moment by another session (a duplicate or a unique violation), is fine. The names are the compiled-in constants above.
fn piece_roles_sql() -> String {
    let mut sql = String::from("DO $$ BEGIN\n");
    for role in PIECE_ROLES {
        sql.push_str(&format!(
            "  BEGIN CREATE ROLE {role} NOLOGIN; EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL; END;\n"
        ));
    }
    sql.push_str("END $$;");
    sql
}

/// The F-1b fixture schema (CONTRACTS §3: "`cortex-testkit`'s two-module schema"), created as
/// `cortex_owner` in the binary's own database. `test_a.t` and `test_b.t` are project-scoped
/// under FORCE ROW LEVEL SECURITY with the ruled `nullif` policy (L-7). F1-10's two controls are
/// the bare-cast and the strict-setting variants. `test_a.locks` gives F1-32 rows to lock.
/// `test_audit.r` refuses `v = 'boom'` (the injected delegated failure).
const SCHEMA: &str = r#"
SET ROLE cortex_owner;
CREATE SCHEMA test_a; CREATE SCHEMA test_b; CREATE SCHEMA test_audit;
CREATE TABLE test_a.t (id bigserial PRIMARY KEY, project_id uuid NOT NULL, v text NOT NULL);
CREATE TABLE test_b.t (id bigserial PRIMARY KEY, project_id uuid NOT NULL, v text NOT NULL);
CREATE TABLE test_a.t_bare_cast (id bigserial PRIMARY KEY, project_id uuid NOT NULL, v text NOT NULL);
CREATE TABLE test_a.t_strict (id bigserial PRIMARY KEY, project_id uuid NOT NULL, v text NOT NULL);
ALTER TABLE test_a.t ENABLE ROW LEVEL SECURITY;           ALTER TABLE test_a.t FORCE ROW LEVEL SECURITY;
ALTER TABLE test_b.t ENABLE ROW LEVEL SECURITY;           ALTER TABLE test_b.t FORCE ROW LEVEL SECURITY;
ALTER TABLE test_a.t_bare_cast ENABLE ROW LEVEL SECURITY; ALTER TABLE test_a.t_bare_cast FORCE ROW LEVEL SECURITY;
ALTER TABLE test_a.t_strict ENABLE ROW LEVEL SECURITY;    ALTER TABLE test_a.t_strict FORCE ROW LEVEL SECURITY;
CREATE POLICY by_project ON test_a.t
  USING (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid)
  WITH CHECK (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid);
CREATE POLICY by_project ON test_b.t
  USING (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid)
  WITH CHECK (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid);
CREATE POLICY bare_cast ON test_a.t_bare_cast USING (project_id = current_setting('cortex.project_id', true)::uuid);
CREATE POLICY strict ON test_a.t_strict USING (project_id = nullif(current_setting('cortex.project_id'), '')::uuid);
CREATE TABLE test_a.locks (id int PRIMARY KEY, v int NOT NULL);
INSERT INTO test_a.locks VALUES (1, 0), (2, 0);
CREATE TABLE test_audit.r (id bigserial PRIMARY KEY, project_id uuid, v text NOT NULL CHECK (v <> 'boom'));
GRANT USAGE ON SCHEMA test_a TO cortex_m_test_a, cortex_base;
GRANT USAGE ON SCHEMA test_b TO cortex_m_test_b;
GRANT USAGE ON SCHEMA test_audit TO cortex_m_test_audit, cortex_base;
GRANT SELECT, INSERT ON test_a.t, test_a.t_bare_cast, test_a.t_strict TO cortex_m_test_a, cortex_base;
GRANT SELECT, UPDATE ON test_a.locks TO cortex_m_test_a;
GRANT SELECT, INSERT ON test_b.t TO cortex_m_test_b;
GRANT SELECT, INSERT ON test_audit.r TO cortex_m_test_audit, cortex_base;
GRANT USAGE ON ALL SEQUENCES IN SCHEMA test_a TO cortex_m_test_a, cortex_base;
GRANT USAGE ON ALL SEQUENCES IN SCHEMA test_b TO cortex_m_test_b;
GRANT USAGE ON ALL SEQUENCES IN SCHEMA test_audit TO cortex_m_test_audit, cortex_base;
RESET ROLE;
"#;

/// The cluster-wide key that serialises role setup (a session advisory lock in `postgres`).
pub const ROLE_SETUP_LOCK: i64 = 0x6376_3274_726f_6c65; // "cv2trole"

/// The modules the fixture registers.
/// r2: `test_ret` is `test_a`'s retention role (F1-31 category 10, E3).
pub const MODULES: &[&str] = &[
    "test_a",
    "test_b",
    "test_audit",
    "test_m",
    "platform",
    "test_ret",
];

/// This binary's database on the supervisor's Postgres. No `Debug`: it holds the passwords.
pub struct TestDb {
    port: u16,
    password: String,
    pub database: String,
}

static DB: OnceCell<TestDb> = OnceCell::const_new();

/// The fixture, set up once per test binary.
pub async fn test_db() -> &'static TestDb {
    DB.get_or_init(setup).await
}

fn opts(port: u16, user: &str, pw: &str, db: &str) -> PgConnectOptions {
    PgConnectOptions::new()
        .host("127.0.0.1")
        .port(port)
        .username(user)
        .password(pw)
        .database(db)
}

fn database_name() -> String {
    let stem = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "bin".into());
    let clean: String = stem
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    let mut name = format!("t_{clean}_{}", std::process::id());
    name.truncate(63);
    name
}

async fn setup() -> TestDb {
    create_database(database_name()).await
}

/// A database of its own for one test (F1-31: each case changes the catalogue, so cases never
/// share one). Named `<binary's name>_<tag>`, cut to 63 bytes; `tag` must be unique in the binary.
pub async fn isolated_db(tag: &str) -> TestDb {
    assert!(
        tag.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
        "a tag is lowercase ASCII, digits and _"
    );
    let mut name = format!("{}_{tag}", database_name());
    name.truncate(63);
    create_database(name).await
}

/// A database of its own with the roles and NO schema: no F-1b fixture, so a piece's migrations
/// run on a truly empty database (the Wave 0 freeze bar, P-03). Named like `isolated_db`.
pub async fn empty_db(tag: &str) -> TestDb {
    create_database_with(database_name_for(tag), false).await
}

/// `<binary's name>_<tag>`, cut to 63 bytes; `tag` must be unique in the binary.
fn database_name_for(tag: &str) -> String {
    assert!(
        tag.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
        "a tag is lowercase ASCII, digits and _"
    );
    let mut name = format!("{}_{tag}", database_name());
    name.truncate(63);
    name
}

async fn create_database(database: String) -> TestDb {
    create_database_with(database, true).await
}

/// `fixture`: whether F-1b's fixture schema is created in the new database.
async fn create_database_with(database: String, fixture: bool) -> TestDb {
    let port: u16 = std::env::var("CV2_PG_PORT")
        .expect(
            "CV2_PG_PORT is unset: run the tests under crates/cortex-testkit/supervise.sh (L-9)",
        )
        .parse()
        .expect("CV2_PG_PORT is not a port");
    let password =
        std::env::var("CV2_PG_PASSWORD").expect("CV2_PG_PASSWORD is unset: run under supervise.sh");
    assert!(
        password.chars().all(|c| c.is_ascii_alphanumeric()),
        "the supervisor's password is alphanumeric"
    );
    let mut admin = PgConnection::connect_with(&opts(port, "postgres", &password, "postgres"))
        .await
        .expect("connect to the supervisor's Postgres as its superuser");
    // r2 (mike's F1B-R0-06): the roles are cluster-wide, and binaries and isolated cases set up
    // concurrently. The check-then-create runs under one cluster-wide session advisory lock, taken
    // in the `postgres` database every caller connects to, and released after the passwords.
    sqlx::query("select pg_advisory_lock($1)")
        .bind(ROLE_SETUP_LOCK)
        .execute(&mut admin)
        .await
        .expect("the role-setup lock");
    sqlx::raw_sql(ROLES)
        .execute(&mut admin)
        .await
        .expect("roles");
    // The module roles the fixtures create (cox 692bf752): created here, under the same lock and before any test has a database, so that
    // no fixture creates a `cortex_m_*` role inside another test's platform migration (see PIECE_ROLES).
    sqlx::raw_sql(sqlx::AssertSqlSafe(piece_roles_sql()))
        .execute(&mut admin)
        .await
        .expect("piece roles");
    // The password is alphanumeric (checked above) and never reaches a log line.
    // A multi-statement query runs as one implicit transaction, and CREATE DATABASE refuses one:
    // it is sent alone.
    let pw_sql = format!(
        "ALTER ROLE cortex_owner PASSWORD '{password}'; ALTER ROLE cortex_app PASSWORD '{password}'; \
         ALTER ROLE cortex_app_rr PASSWORD '{password}'; ALTER ROLE cortex_base PASSWORD '{password}';"
    );
    sqlx::raw_sql(sqlx::AssertSqlSafe(pw_sql))
        .execute(&mut admin)
        .await
        .expect("role passwords");
    sqlx::query("select pg_advisory_unlock($1)")
        .bind(ROLE_SETUP_LOCK)
        .execute(&mut admin)
        .await
        .expect("release the role-setup lock");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "CREATE DATABASE {database} OWNER cortex_owner"
    )))
    .execute(&mut admin)
    .await
    .expect("the binary's database");
    sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "REVOKE ALL ON DATABASE {database} FROM PUBLIC; \
         GRANT CONNECT ON DATABASE {database} TO cortex_app, cortex_app_rr, cortex_base;"
    )))
    .execute(&mut admin)
    .await
    .expect("database grants");
    admin.close().await.ok();
    if fixture {
        let mut db = PgConnection::connect_with(&opts(port, "postgres", &password, &database))
            .await
            .expect("the new database");
        sqlx::raw_sql(SCHEMA)
            .execute(&mut db)
            .await
            .expect("the fixture schema");
        db.close().await.ok();
    }
    TestDb {
        port,
        password,
        database,
    }
}

impl TestDb {
    fn endpoint(&self) -> DbEndpoint {
        DbEndpoint::Tcp {
            host: "127.0.0.1".into(),
            port: self.port,
        }
    }

    /// A `cortex_app` configuration registering `modules`.
    pub fn db_config(&self, modules: &[&str], clock: Arc<dyn Clock>) -> DbConfig {
        DbConfig {
            endpoint: self.endpoint(),
            database: self.database.clone(),
            app_user: "cortex_app".into(),
            app_password: Secret::new(self.password.clone()),
            max_connections: 4,
            statement_cache: true,
            registry: ModuleRegistry::new(modules.iter().map(|m| module(m))),
            clock,
        }
    }

    /// The same, against a port where nothing listens: every connection attempt fails.
    pub fn dead_db_config(&self, modules: &[&str], clock: Arc<dyn Clock>) -> DbConfig {
        let mut c = self.db_config(modules, clock);
        c.endpoint = DbEndpoint::Tcp {
            host: "127.0.0.1".into(),
            port: 1,
        };
        c
    }

    pub fn owner_config(&self) -> OwnerConfig {
        OwnerConfig {
            endpoint: self.endpoint(),
            database: self.database.clone(),
            owner_user: "cortex_owner".into(),
            owner_password: Secret::new(self.password.clone()),
        }
    }

    /// The superuser, in this binary's database: for setup and for reading past RLS.
    pub async fn admin(&self) -> PgConnection {
        PgConnection::connect_with(&opts(self.port, "postgres", &self.password, &self.database))
            .await
            .expect("admin")
    }

    /// `cortex_owner`, raw: for fixtures that a migration would install (F1-31). Not
    /// `OwnerConn`, so a fixture does not depend on the code under test.
    pub async fn owner(&self) -> PgConnection {
        PgConnection::connect_with(&opts(
            self.port,
            "cortex_owner",
            &self.password,
            &self.database,
        ))
        .await
        .expect("owner")
    }

    /// Any login role of the fixture, raw.
    pub async fn login(&self, role: &str) -> PgConnection {
        PgConnection::connect_with(&opts(self.port, role, &self.password, &self.database))
            .await
            .expect("login")
    }

    /// `cortex_base`: one role holding both test grants, with no switching.
    pub async fn base(&self) -> PgConnection {
        PgConnection::connect_with(&opts(
            self.port,
            "cortex_base",
            &self.password,
            &self.database,
        ))
        .await
        .expect("base")
    }
}

/// A module id from a literal.
pub fn module(id: &str) -> ModuleId {
    ModuleId::parse(id).expect("a valid module id")
}

/// A context for `module`, acting as that module's system task.
pub fn ctx(m: &str, project: Option<ProjectId>) -> TxContext {
    TxContext {
        module: module(m),
        project,
        actor: Actor::System(module(m)),
        request_id: RequestId::new(&SystemClock),
    }
}

/// The identity of the session behind a connection (TESTS r7 F1-06; Z-2's Z2-2): its backend
/// pid and the random token `DbPool` sets on every new connection under the `testkit` feature
/// (`cortex_db::testkit::SESSION_TOKEN_GUC`). A replacement session has a new token, even if the
/// system reuses the pid. Readable under any role, so it works inside a `ModuleTx`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionId {
    pub pid: i32,
    pub token: String,
}

/// Panics if the session has no token: two missing tokens must never compare equal.
pub async fn session_identity(conn: &mut PgConnection) -> SessionId {
    use sqlx::Row;
    let r = sqlx::query("select pg_backend_pid(), current_setting($1, true)")
        .bind(cortex_db::testkit::SESSION_TOKEN_GUC)
        .fetch_one(conn)
        .await
        .expect("session identity");
    let token: Option<String> = r.get(1);
    let token = token.filter(|t| !t.is_empty()).unwrap_or_else(|| {
        panic!(
            "no {} on this session: DbPool sets one on every new connection under the testkit feature",
            cortex_db::testkit::SESSION_TOKEN_GUC
        )
    });
    SessionId {
        pid: r.get(0),
        token,
    }
}

/// The reuse oracle: Ok only if `after` is the very session `before` was.
pub fn same_session(before: &SessionId, after: &SessionId) -> Result<(), String> {
    if before == after {
        Ok(())
    } else {
        Err(format!(
            "replaced, not reused: pid {} -> {}, session token changed: {}",
            before.pid,
            after.pid,
            before.token != after.token
        ))
    }
}

/// The SQLSTATE of a database error.
pub fn sqlstate(e: &sqlx::Error) -> Option<String> {
    e.as_database_error()
        .and_then(|d| d.code())
        .map(|c| c.into_owned())
}

/// Every case runs inside a deadline, so a hang is a failure, not a stuck run.
pub async fn within<T>(secs: u64, f: impl Future<Output = T>) -> T {
    tokio::time::timeout(Duration::from_secs(secs), f)
        .await
        .unwrap_or_else(|_| panic!("case exceeded its {secs} s deadline"))
}
