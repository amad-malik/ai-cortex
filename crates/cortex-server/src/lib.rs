//! `cortex-server` (piece 11, cox). Contract: `mvp/pieces/11-cortex-server-CONTRACT-r0.md` r0.3.
//!
//! This increment holds three small, pure parts the rest of the server will stand on:
//! * [`cli`]: the command line (`--version`, `serve`, `migrate`, `bootstrap`), parsed by hand (clap is not in the lock);
//! * [`bootstrap`]: the host bootstrap decision, including `--require-existing` (W1-REISSUE: exit 3, one fixed stderr line,
//!   nothing created);
//! * [`tokenfile`]: the fresh 0600 token file, never through a symlink or an existing file, no partial file on failure.
//!
//! STUB: every function is `todo!()` so the tests are RED first.

pub mod assembly;
pub mod bootstrap;
pub mod bootstrap_cmd;
pub mod cli;
pub mod dbowner;
pub mod fault;
pub mod health;
pub mod health_routes;
pub mod ledger;
pub mod migrate;
pub mod migrate_cmd;
pub mod migrator;
pub mod minter;
pub mod probe;
pub mod secretfile;
pub mod serve;
pub mod store;
pub mod tokenfile;
