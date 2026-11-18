//! What a piece is (section 1.1).

use std::future::Future;
use std::pin::Pin;

use cortex_db::MigrationSet;

use crate::manifest::Manifest;
use crate::ports::{Ports, PortsBuilder};
use crate::route::Routes;

/// A boxed `Send` future, so `Piece` stays usable as `Box<dyn Piece>`.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A piece's own readiness (section 1.3). The reason is a fixed text: no data, no secret.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Ready,
    NotReady(&'static str),
}

/// A piece: one crate, one schema, one manifest. `cortex-server` holds them as
/// `Vec<Box<dyn Piece>>`, one `#[cfg(feature = "mod-x")]` line per compiled piece.
pub trait Piece: Send + Sync + 'static {
    fn manifest(&self) -> &'static Manifest;

    /// Its own schema's migrations only (`manifest().schema`); the server applies them through
    /// S1, after `platform`, before the listener opens.
    fn migrations(&self) -> MigrationSet;

    /// The ports it implements, if any: exactly its manifest's `provides`.
    fn provide(&self, _ports: &mut PortsBuilder) {}

    /// Its routes, after every piece has provided. `ports` holds only the ports its manifest
    /// `uses`, and each of them is present (the registry checked it).
    fn routes(&self, ports: &Ports) -> Routes;

    /// `Ready`, or `NotReady` with a fixed reason. A piece with nothing of its own to check
    /// returns `Ready`. The registry gives it 2 s.
    fn health(&self) -> BoxFuture<'_, Health>;
}
