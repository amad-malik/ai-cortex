//! One line per group. An owner adds `mod <name>;` and one `<name>::group()` below, and a file of his own; he edits nothing else.

mod audit;
mod doctor;
pub mod upgrade;

use crate::registry::Group;

pub fn all() -> Vec<Group> {
    vec![audit::group(), doctor::group(), upgrade::group()]
}
