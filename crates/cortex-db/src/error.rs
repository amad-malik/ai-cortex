//! `DbError` (CONTRACTS §3). No variant carries row data.
use crate::tx::LockStep;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DbError {
    /// 42501.
    #[error("permission denied")]
    PermissionDenied,
    /// 55P03 lock not available, 40P01 deadlock.
    #[error("busy: a lock was not available, or a deadlock was broken")]
    Busy,
    /// 57014.
    #[error("statement timeout")]
    StatementTimeout,
    /// The server ended the session; the connection is closed, never pooled again.
    #[error("the session ended")]
    SessionEnded,
    #[error("constraint {name}")]
    Constraint { name: String },
    /// `commit` after a failed `delegate`.
    #[error("a delegated call failed; the transaction rolls back")]
    DelegationFailed,
    /// r6: refused before any SQL; the transaction is poisoned.
    #[error("lock order: holding {held:?}, asked for {wanted:?}")]
    LockOrder { held: LockStep, wanted: LockStep },
    /// r7: not in the registry, or not a valid role name; refused before any connection.
    #[error("unknown module")]
    UnknownModule,
    /// r6: a transaction whose isolation is not READ COMMITTED.
    #[error("not READ COMMITTED")]
    Isolation,
    #[error("the database is unavailable")]
    Unavailable,
    /// No row data, ever.
    #[error("database error: {0}")]
    Other(String),
}

impl DbError {
    /// A statement's error, by SQLSTATE only: the server's message can carry row data, so it never
    /// passes through (CONTRACTS §3: "no row data, ever"). A maintenance refusal (class P0, A7 r4
    /// item 2) is `Other(<its code>)` exactly (E12).
    pub(crate) fn from_sqlx(e: sqlx::Error) -> DbError {
        match e {
            sqlx::Error::Database(d) => match d.code().as_deref() {
                Some("42501") => DbError::PermissionDenied,
                Some("55P03") | Some("40P01") => DbError::Busy,
                Some("57014") => DbError::StatementTimeout,
                Some(c) if c.starts_with("57P") => DbError::SessionEnded,
                Some(c) if c.starts_with("23") => DbError::Constraint {
                    name: d
                        .constraint()
                        .map(str::to_owned)
                        .unwrap_or_else(|| c.to_owned()),
                },
                Some(c) => match maint_code(c) {
                    Some(code) => DbError::Other(code.to_owned()),
                    None => DbError::Other(format!("sqlstate {c}")),
                },
                None => DbError::Other("database error".into()),
            },
            sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed => DbError::Unavailable,
            sqlx::Error::Io(_) | sqlx::Error::Protocol(_) | sqlx::Error::Tls(_) => {
                DbError::SessionEnded
            }
            _ => DbError::Other("database client error".into()),
        }
    }

    /// An error while taking a connection (a pool acquisition or a new session): the database is
    /// unavailable, whatever the cause.
    pub(crate) fn unavailable(_e: sqlx::Error) -> DbError {
        DbError::Unavailable
    }
}

/// A piece's own statement error, by SQLSTATE only, as `from_sqlx` (Wave 0: pieces outside this
/// crate run their SQL on `ModuleTx::executor`). No row data passes.
impl From<sqlx::Error> for DbError {
    fn from(e: sqlx::Error) -> DbError {
        DbError::from_sqlx(e)
    }
}

/// A maintenance refusal's code (class P0), when the banked maintenance functions are built.
fn maint_code(c: &str) -> Option<&'static str> {
    #[cfg(feature = "maintenance")]
    {
        crate::maintenance::MaintError::from_sqlstate(c).map(|m| m.code())
    }
    #[cfg(not(feature = "maintenance"))]
    {
        let _ = c;
        None
    }
}
