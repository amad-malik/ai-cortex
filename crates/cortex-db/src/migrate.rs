//! The migration runner (CONTRACTS §3; S1 §2.2). Migrations run as `cortex_owner`, in order,
//! recorded with a checksum in `platform.migrations`.
use cortex_types::ModuleId;
use sqlx::{Connection, PgConnection};

use crate::error::DbError;
use crate::pool::OwnerConn;
use crate::tx::ModuleTx;

/// The cluster-wide key that serialises migration steps (a transaction advisory lock, single
/// bigint form, so it cannot meet the two-integer classes of `AdvisoryClass`).
const MIGRATE_LOCK: i64 = 0x6376_326d_6967_7274; // "cv2migrt"

/// The guard every step runs under when the banked `guard` feature is built
/// (`crate::migrate_guard`): which catalogues a step changed comes from the server's own
/// per-transaction write counts over every catalogue, and each catalogue has exactly one rule. It
/// is a check on trusted release migrations, not confinement.
#[cfg(feature = "guard")]
pub use crate::migrate_guard::{
    CatalogueRule, ReferenceRule, guard_catalogues, guard_changed_endpoints, guard_references,
    owned_initdb_sql,
};

fn db(e: sqlx::Error) -> MigrateError {
    MigrateError::Db(DbError::from_sqlx(e))
}

/// One step's SQL, in the step's transaction: under the guard when it is built (feature `guard`),
/// else directly. Without the guard, release migrations are trusted (C-32); the guard is banked for
/// v0.2.001.
async fn run_step(
    conn: &mut PgConnection,
    schema: &str,
    version: u32,
    sql: &'static str,
) -> Result<(), MigrateError> {
    #[cfg(feature = "guard")]
    {
        crate::migrate_guard::run_guarded(conn, schema, version, sql).await
    }
    #[cfg(not(feature = "guard"))]
    {
        let _ = (schema, version);
        sqlx::raw_sql(sql)
            .execute(&mut *conn)
            .await
            .map(|_| ())
            .map_err(db)
    }
}

/// The step's checksum: sha256 of its `up` and `down`, computed by the server.
async fn checksum(conn: &mut PgConnection, step: &Migration) -> Result<String, MigrateError> {
    sqlx::query_scalar(
        "SELECT encode(sha256(convert_to($1 || E'\n--down--\n' || $2, 'UTF8')), 'hex')",
    )
    .bind(step.up)
    .bind(step.down)
    .fetch_one(conn)
    .await
    .map_err(db)
}

async fn ledger_exists(conn: &mut PgConnection) -> Result<bool, MigrateError> {
    sqlx::query_scalar("SELECT to_regclass('platform.migrations') IS NOT NULL")
        .fetch_one(conn)
        .await
        .map_err(db)
}

/// One step: `up` and its `down`, both fixed SQL from the module's crate.
#[derive(Debug, Clone)]
pub struct Migration {
    pub version: u32,
    pub name: &'static str,
    pub up: &'static str,
    pub down: &'static str,
}

/// A module's migrations, for its own schema (the guard: `crate::migrate_guard`).
#[derive(Debug, Clone)]
pub struct MigrationSet {
    pub module: ModuleId,
    pub schema: &'static str,
    pub steps: Vec<Migration>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MigrateError {
    /// The guard found an object outside the module's schema that the step added, changed or
    /// removed, or a catalogue no migration may change (named `pg_catalog.<name>`); the step was
    /// rolled back.
    #[error("migration {version} touched {object}, outside its schema; rolled back")]
    OutsideSchema { version: u32, object: String },
    /// The step did not run: an inheritance or partition link (`pg_inherits`) already joins two
    /// schemas, and the guard is defined for schema-closed trees only (kai's ruling T-1). `link`
    /// names the first such link.
    #[error(
        "migration {version} not run: the starting topology is not schema-closed ({link}); rolled back"
    )]
    StartTopologyNotSchemaClosed { version: u32, link: String },
    /// The step ran but left such a link (a temporary child of an own table, for one), so it was
    /// rolled back and not recorded (rework 6). `link` names the first one.
    #[error("migration {version} left the topology not schema-closed ({link}); rolled back")]
    StepLeftTopologyNotSchemaClosed { version: u32, link: String },
    /// The step left a holdable cursor open: COMMIT would run the rest of its query, user code
    /// included, after the guard's observations; so it was rolled back and not recorded
    /// (rework 8). `cursor` names the first one.
    #[error("migration {version} left a holdable cursor open ({cursor}); rolled back")]
    StepLeftHoldableCursor { version: u32, cursor: String },
    /// The step left a (non-holdable) cursor open: COMMIT would drop it, and that drop may run
    /// user code after the guard's observations; so it was rolled back and not recorded
    /// (rework 8). `cursor` names the first one.
    #[error("migration {version} left a cursor open ({cursor}); rolled back")]
    StepLeftOpenCursor { version: u32, cursor: String },
    /// An applied step's checksum changed: startup stops.
    #[error("applied migration {version} has changed")]
    ChecksumChanged { version: u32 },
    #[error("migration versions must increase")]
    OutOfOrder,
    #[error(transparent)]
    Db(#[from] DbError),
}

/// The `platform` schema's migrations: `platform.idempotency`, `platform.objects`, `platform.refs`,
/// `platform.migrations` (the checksum ledger) and `platform.now()` (CONTRACTS §3; S2 §5, §6).
pub fn platform_migrations() -> MigrationSet {
    MigrationSet {
        module: ModuleId::parse("platform").expect("a valid module id"),
        schema: "platform",
        steps: vec![Migration {
            version: 1,
            name: "platform",
            up: include_str!("../migrations/platform/0001_platform.up.sql"),
            down: include_str!("../migrations/platform/0001_platform.down.sql"),
        }],
    }
}

/// What the ledger says about ONE module's migration set (W1-41, cox 14:35Z), as [`verify`] reads it. Read-only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LedgerState {
    /// Every step of the set is applied with an equal checksum, and the ledger holds no step the set does not know.
    Current,
    /// These steps (ascending) are not applied yet; nothing else is wrong. The server starts NOT READY.
    Unapplied { steps: Vec<u32> },
    /// An applied step's checksum is not the set's: startup stops (`MigrateError::ChecksumChanged` in `apply`).
    ChecksumChanged { version: u32 },
    /// The ledger holds steps this binary's set does not know (a rollback to an older binary). NOT READY, not a stop.
    DbAhead,
}

/// Compares `set` with the ledger, READ-ONLY, in a transaction under the `platform` module role (the one role that can read
/// `platform.migrations`, W1-41). It uses the SAME [`checksum`] as [`apply`], so the formula exists once. Precedence when several
/// things are true: `ChecksumChanged` (the lowest such version), then `DbAhead`, then `Unapplied`, then `Current`. A set whose
/// versions do not increase is `OutOfOrder`, as in `apply`. With no ledger yet (nothing was ever migrated) every step is unapplied.
/// It takes no advisory lock and writes nothing.
pub async fn verify(tx: &mut ModuleTx, set: &MigrationSet) -> Result<LedgerState, MigrateError> {
    if set.steps.windows(2).any(|w| w[1].version <= w[0].version) {
        return Err(MigrateError::OutOfOrder);
    }
    let conn = tx.executor()?;
    let recorded: std::collections::BTreeMap<u32, String> = if ledger_exists(conn).await? {
        sqlx::query_as::<_, (i32, String)>(
            "SELECT version, checksum FROM platform.migrations WHERE module = $1 ORDER BY version",
        )
        .bind(set.module.as_str())
        .fetch_all(&mut *conn)
        .await
        .map_err(db)?
        .into_iter()
        .map(|(v, sum)| (v as u32, sum))
        .collect()
    } else {
        std::collections::BTreeMap::new()
    };
    for step in &set.steps {
        if let Some(recorded_sum) = recorded.get(&step.version)
            && *recorded_sum != checksum(conn, step).await?
        {
            return Ok(LedgerState::ChecksumChanged {
                version: step.version,
            });
        }
    }
    if recorded
        .keys()
        .any(|v| !set.steps.iter().any(|s| s.version == *v))
    {
        return Ok(LedgerState::DbAhead);
    }
    let steps: Vec<u32> = set
        .steps
        .iter()
        .filter(|s| !recorded.contains_key(&s.version))
        .map(|s| s.version)
        .collect();
    Ok(if steps.is_empty() {
        LedgerState::Current
    } else {
        LedgerState::Unapplied { steps }
    })
}

/// Applies the set's unapplied steps in order, each in its own transaction.
pub async fn apply(owner: &mut OwnerConn, set: &MigrationSet) -> Result<(), MigrateError> {
    if set.steps.windows(2).any(|w| w[1].version <= w[0].version) {
        return Err(MigrateError::OutOfOrder);
    }
    let module = set.module.as_str();
    for step in &set.steps {
        let mut tx = owner.conn.begin().await.map_err(db)?;
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(MIGRATE_LOCK)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        let sum = checksum(&mut tx, step).await?;
        if ledger_exists(&mut tx).await? {
            let recorded: Option<String> = sqlx::query_scalar(
                "SELECT checksum FROM platform.migrations WHERE module = $1 AND version = $2",
            )
            .bind(module)
            .bind(step.version as i32)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db)?;
            if let Some(recorded) = recorded {
                tx.rollback().await.ok();
                if recorded != sum {
                    return Err(MigrateError::ChecksumChanged {
                        version: step.version,
                    });
                }
                continue;
            }
        } else if module != "platform" {
            return Err(MigrateError::Db(DbError::Other(
                "the platform migrations are not applied".into(),
            )));
        }
        // On an error the transaction is dropped, and rolled back.
        run_step(&mut tx, set.schema, step.version, step.up).await?;
        sqlx::query("INSERT INTO platform.migrations (module, version, name, checksum) VALUES ($1, $2, $3, $4)")
            .bind(module)
            .bind(step.version as i32)
            .bind(step.name)
            .bind(&sum)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        tx.commit().await.map_err(db)?;
    }
    Ok(())
}

/// Runs the last applied step's `down`, and removes its record.
pub async fn revert_last(owner: &mut OwnerConn, set: &MigrationSet) -> Result<(), MigrateError> {
    let module = set.module.as_str();
    let mut tx = owner.conn.begin().await.map_err(db)?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(MIGRATE_LOCK)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    let last: Option<i32> =
        sqlx::query_scalar("SELECT max(version) FROM platform.migrations WHERE module = $1")
            .bind(module)
            .fetch_one(&mut *tx)
            .await
            .map_err(db)?;
    let Some(last) = last else {
        return Err(MigrateError::Db(DbError::Other(
            "no applied migration".into(),
        )));
    };
    let version = u32::try_from(last).map_err(|_| MigrateError::OutOfOrder)?;
    let Some(step) = set.steps.iter().find(|s| s.version == version) else {
        return Err(MigrateError::Db(DbError::Other(
            "the last applied migration is not in the set".into(),
        )));
    };
    sqlx::query("DELETE FROM platform.migrations WHERE module = $1 AND version = $2")
        .bind(module)
        .bind(last)
        .execute(&mut *tx)
        .await
        .map_err(db)?;
    run_step(&mut tx, set.schema, version, step.down).await?;
    tx.commit().await.map_err(db)?;
    Ok(())
}
