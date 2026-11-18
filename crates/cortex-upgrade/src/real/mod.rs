//! The real implementations of the ports (`Containers`, `Psql`, `Dumper`): thin wrappers that start ONE program with
//! an argument vector. Brief: `pieces/14-cortex-upgrade-REAL-PORTS-BRIEF-r1.md` (the eight rules R1 to R8 are test
//! cells in `tests/real_*.rs`). Lane UPREAL, builder finn.
//!
//! - [`run`]: the one place a child is started (R1, R2, R6, R7, R8).
//! - [`env`]: the one function that builds the fixed environment of every child (R2).
//! - [`name`]: the exact container name of a service of a release (R4).
//! - [`containers`], [`psql`], [`dumper`]: the three wrappers.

pub mod containers;
pub mod dumper;
pub mod env;
pub mod name;
pub mod psql;
pub mod run;

pub(crate) use containers::ROLES_PROJECT;
pub use containers::{Deadlines, RealContainers};
pub use dumper::RealDumper;
pub use env::ComposeEnv;
pub use psql::RealPsql;
