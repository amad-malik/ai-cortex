//! W1-37: assembly must bind the audit provider before it reports readiness.
use std::sync::Arc;

use async_trait::async_trait;
use cortex_db::{MigrationSet, ModuleTx};
use cortex_module::{
    Assembly, BoxFuture, Health, IdentityApi, Manifest, Piece, Port, PortError, Ports,
    PortsBuilder, ProjectView, Registry, Routes, WorkerView,
};
use cortex_types::{ModuleId, ProblemCodeMinter, WorkerId};
use mod_audit::AuditPiece;

struct Identity;

#[async_trait]
impl IdentityApi for Identity {
    async fn project(&self, _: &mut ModuleTx) -> Result<ProjectView, PortError> {
        Err(PortError::Unavailable)
    }
    async fn worker(
        &self,
        _: &mut ModuleTx,
        _: &WorkerId,
    ) -> Result<Option<WorkerView>, PortError> {
        Err(PortError::Unavailable)
    }
    async fn roster(&self, _: &mut ModuleTx) -> Result<Vec<WorkerView>, PortError> {
        Err(PortError::Unavailable)
    }
}

static IDENTITY: Manifest = Manifest {
    id: "identity",
    version: "1",
    contract: 1,
    core: false,
    schema: "identity",
    provides: &[Port::Identity],
    uses: &[Port::Audit],
    scopes: &[],
    problems: &[],
};

impl Piece for Identity {
    fn manifest(&self) -> &'static Manifest {
        &IDENTITY
    }
    fn migrations(&self) -> MigrationSet {
        MigrationSet {
            module: ModuleId::parse("identity").unwrap(),
            schema: "identity",
            steps: vec![],
        }
    }
    fn provide(&self, ports: &mut PortsBuilder) {
        ports.identity(Arc::new(Identity));
    }
    fn routes(&self, _: &Ports) -> Routes {
        Routes::new()
    }
    fn health(&self) -> BoxFuture<'_, Health> {
        Box::pin(async { Health::Ready })
    }
}

#[tokio::test]
async fn registry_binds_cyclic_identity_dependency_before_readiness() {
    assert!(matches!(
        AuditPiece::new().health().await,
        Health::NotReady(_)
    ));
    let minter = ProblemCodeMinter::take().unwrap();
    let enabled = [
        ModuleId::parse("audit").unwrap(),
        ModuleId::parse("identity").unwrap(),
    ];
    let registry = Registry::assemble(Assembly {
        compiled: vec![Box::new(AuditPiece::new()), Box::new(Identity)],
        enabled: &enabled,
        minter: &minter,
        chain_codes: &[],
    })
    .unwrap();
    assert!(registry.ports().audit().is_some());
    assert_eq!(
        registry
            .health()
            .await
            .into_iter()
            .find(|h| h.piece == "audit")
            .unwrap()
            .health,
        Health::Ready
    );
}
