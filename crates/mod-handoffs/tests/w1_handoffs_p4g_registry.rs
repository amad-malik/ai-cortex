//! Wave 1, mod-handoffs P4g (row 2a86598d): the piece inside the registry. `mvp/tests/mod-handoffs.md` HAND-115:
//! health, and a disabled piece. DB-free: nothing here opens a database.
//!
//! **Where the ledger check lives.** My r3.4 row said `health()` is `NotReady` "when the ledger lacks the step".
//! The platform's contract (`Registry::health`: "the pool and the migration ledger are the server's part of
//! readiness"; `Piece::health`: "a piece with nothing of its own to check returns `Ready`") puts that check in the
//! server, not in the piece. This piece has no background task and no external dependency, so it has nothing of its
//! own to check and answers `Ready`; the ledger case belongs to the server's tests. The row is corrected in r3.5.
mod common;

use common::{AUDIT, assemble};
use cortex_db::MigrationSet;
use cortex_module::{
    BoxFuture, CONTRACT_VERSION, Health, Manifest, Piece, Port, Routes, StartupError,
};
use cortex_types::ModuleId;

const NEEDS: &str = "needs_handoffs";

static NEEDS_MANIFEST: Manifest = Manifest {
    id: NEEDS,
    version: "0.0.0",
    contract: CONTRACT_VERSION,
    core: false,
    schema: NEEDS,
    provides: &[],
    uses: &[Port::Handoffs],
    scopes: &[],
    problems: &[],
};

/// A piece that `uses` the handoffs port, as boot does.
struct NeedsHandoffs;

impl Piece for NeedsHandoffs {
    fn manifest(&self) -> &'static Manifest {
        &NEEDS_MANIFEST
    }

    fn migrations(&self) -> MigrationSet {
        MigrationSet {
            module: ModuleId::parse(NEEDS).expect("an id"),
            schema: NEEDS,
            steps: Vec::new(),
        }
    }

    fn routes(&self, _ports: &cortex_module::Ports) -> Routes {
        Routes::new()
    }

    fn health(&self) -> BoxFuture<'_, Health> {
        Box::pin(async { Health::Ready })
    }
}

/// HAND-115: an enabled piece reports `Ready`, with its routes mounted and its port provided.
#[tokio::test]
async fn an_enabled_piece_is_ready_provides_its_port_and_mounts_its_routes() {
    let mut registry = assemble(&[mod_handoffs::ID, plug_identity::ID, AUDIT], Vec::new())
        .expect("the three pieces assemble");
    let health = registry.health().await;
    let mine = health
        .iter()
        .find(|h| h.piece == mod_handoffs::ID)
        .expect("the piece reports");
    assert_eq!(mine.health, Health::Ready);
    assert!(registry.ports().handoffs().is_some(), "the port is there");
    let routes = registry.take_routes();
    assert!(
        routes.iter().any(|(id, _)| *id == mod_handoffs::ID),
        "its routes are mounted"
    );
}

/// HAND-115: with the piece disabled it is not in the health report, its port is absent and none of its routes
/// is mounted (so every path of it falls to the chain's 404 problem, which `cortex-http` tests).
#[tokio::test]
async fn a_disabled_piece_has_no_health_no_port_and_no_routes() {
    let mut registry = assemble(&[plug_identity::ID, AUDIT], Vec::new())
        .expect("identity and audit assemble without it");
    assert!(
        registry
            .health()
            .await
            .iter()
            .all(|h| h.piece != mod_handoffs::ID),
        "a disabled piece is not in the report"
    );
    assert!(
        registry.ports().handoffs().is_none(),
        "HandoffsApi is absent"
    );
    assert!(
        registry
            .take_routes()
            .iter()
            .all(|(id, _)| *id != mod_handoffs::ID),
        "none of its routes is mounted"
    );
}

/// HAND-115: a piece that `uses` the handoffs port fails startup with `MissingPort` when this piece is disabled,
/// and starts when it is enabled (S0 section 1.2).
#[test]
fn a_piece_that_uses_the_port_fails_startup_without_it() {
    let errs = match assemble(
        &[plug_identity::ID, AUDIT, NEEDS],
        vec![Box::new(NeedsHandoffs)],
    ) {
        Ok(_) => panic!("it must not start without the handoffs piece"),
        Err(e) => e,
    };
    assert!(
        errs.iter().any(|e| matches!(
            e,
            StartupError::MissingPort { piece, port }
                if piece == NEEDS && *port == Port::Handoffs
        )),
        "{errs:?}"
    );
    assert!(
        assemble(
            &[mod_handoffs::ID, plug_identity::ID, AUDIT, NEEDS],
            vec![Box::new(NeedsHandoffs)]
        )
        .is_ok(),
        "with the piece enabled it starts"
    );
}
