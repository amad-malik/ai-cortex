//! A test piece that provides `IdentityApi` over the real `identity` tables (the pack is applied by the harness; a
//! piece must not depend on mod-identity, so this is a fixture).
#![allow(dead_code)]

use std::sync::Arc;

use cortex_db::{Migration, MigrationSet};
use cortex_module::{
    BoxFuture, CONTRACT_VERSION, Health, Manifest, Piece, Port, Ports, PortsBuilder, Routes,
};
use cortex_types::ModuleId;

use super::db::SqlIdentity;

static MANIFEST: Manifest = Manifest {
    id: "identity",
    version: "0.0.0",
    contract: CONTRACT_VERSION,
    core: true,
    schema: "identity",
    provides: &[Port::Identity],
    uses: &[],
    scopes: &[],
    problems: &[],
};

pub struct IdentityFixture;

impl Piece for IdentityFixture {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }
    fn migrations(&self) -> MigrationSet {
        MigrationSet {
            module: ModuleId::parse("identity").expect("id"),
            schema: "identity",
            steps: Vec::<Migration>::new(),
        }
    }
    fn provide(&self, ports: &mut PortsBuilder) {
        ports.identity(Arc::new(SqlIdentity));
    }
    fn routes(&self, _ports: &Ports) -> Routes {
        Routes::new()
    }
    fn health(&self) -> BoxFuture<'_, Health> {
        Box::pin(async { Health::Ready })
    }
}
