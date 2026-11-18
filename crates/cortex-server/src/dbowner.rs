//! The owner role's connection settings from the config (contract r0.3: only `migrate` mounts the owner secret; `serve` ignores every
//! `owner_*` key). The owner password comes from its FILE through the strict reader; the app password is never read here.

use cortex_config::{ConfigError, Database, DbHost};
use cortex_db::{DbEndpoint, OwnerConfig};

use crate::secretfile::read_secret_file;

/// Why the owner configuration could not be made. Fixed text, no path, no value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnerConfigError {
    /// A key is missing (the name of the KEY only).
    Missing(String),
    /// The owner password file could not be read.
    PasswordUnreadable,
}

impl std::fmt::Display for OwnerConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OwnerConfigError::Missing(k) => write!(f, "missing: {k}"),
            OwnerConfigError::PasswordUnreadable => {
                f.write_str("the owner password file could not be read")
            }
        }
    }
}

pub fn owner_config(db: &Database) -> Result<OwnerConfig, OwnerConfigError> {
    let owner = db.owner_for_migrate().map_err(|e| match e {
        ConfigError::Missing(key) => OwnerConfigError::Missing(key),
        _ => OwnerConfigError::Missing("database.owner_user".to_owned()),
    })?;
    let password =
        read_secret_file(&owner.password_file).map_err(|_| OwnerConfigError::PasswordUnreadable)?;
    let endpoint = match &db.host {
        DbHost::SocketDir(dir) => DbEndpoint::Socket {
            dir: dir.clone(),
            port: db.port,
        },
        DbHost::Tcp(host) => DbEndpoint::Tcp {
            host: host.clone(),
            port: db.port,
        },
    };
    Ok(OwnerConfig {
        endpoint,
        database: db.name.clone(),
        owner_user: owner.user.clone(),
        owner_password: password,
    })
}
