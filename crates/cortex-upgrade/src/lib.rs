//! `cortex upgrade` (piece 14): the manifest rules, the choice of a release, and (later) the release
//! layout and the upgrade state machine. Design `pieces/14-cortex-upgrade-DESIGN-r7.md`, test
//! definitions `pieces/14-cortex-upgrade-TEST-DEFINITIONS-r4.md`.
//!
//! This crate depends on `cortex-types` only (no other piece); marv's `cortex-cli` calls it.

pub mod choose;
pub mod compose;
pub mod engine;
pub mod fetch;
pub mod http;
pub mod layout;
pub mod manifest;
pub mod members;
pub mod ports;
pub mod provider;
pub mod real;
pub mod stack_env;
pub mod stage;
mod text;
pub mod unpack;
pub mod verify;
pub mod version;
