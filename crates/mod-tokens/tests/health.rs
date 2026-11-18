//! cox's row ccfcb9bf (found by mira on the real set): `health()` of mod-tokens was ALWAYS `Ready` although the verifier
//! fails closed (every verification `Unavailable`, 503) until the database pool AND the identity port are bound. On the real
//! server that read `/v2/health/ready` = ready while every authenticated request was 503. The rule: the piece is NOT ready
//! while the store is unbound (`Late::ready()`), and ready once both are bound. DB-free: the pool is lazy (never connects).
use std::sync::Arc;

use async_trait::async_trait;
use cortex_db::{DbConfig, DbEndpoint, DbPool, ModuleRegistry, ModuleTx};
use cortex_module::{Health, IdentityApi, Piece, PortError, ProjectView, WorkerView};
use cortex_types::{Secret, SystemClock, WorkerId};
use mod_tokens::ModTokens;

/// The fixed reason (no data, no secret).
const NOT_BOUND: &str = "the token store is not bound";

struct NoIdentity;

#[async_trait]
impl IdentityApi for NoIdentity {
    async fn project(&self, _tx: &mut ModuleTx) -> Result<ProjectView, PortError> {
        Err(PortError::Unavailable)
    }
    async fn worker(
        &self,
        _tx: &mut ModuleTx,
        _w: &WorkerId,
    ) -> Result<Option<WorkerView>, PortError> {
        Err(PortError::Unavailable)
    }
    async fn roster(&self, _tx: &mut ModuleTx) -> Result<Vec<WorkerView>, PortError> {
        Err(PortError::Unavailable)
    }
}

/// A pool that never connects (port 9). A lazy pool may spawn tasks, so it is built inside a runtime.
fn lazy_pool() -> Arc<DbPool> {
    let cfg = DbConfig {
        endpoint: DbEndpoint::Tcp {
            host: "127.0.0.1".into(),
            port: 9,
        },
        database: "none".into(),
        app_user: "none".into(),
        app_password: Secret::new("none"),
        max_connections: 1,
        statement_cache: false,
        registry: ModuleRegistry::new([]),
        clock: Arc::new(SystemClock),
    };
    Arc::new(DbPool::connect_lazy(&cfg))
}

#[tokio::test]
async fn h_1_a_piece_with_nothing_bound_is_not_ready() {
    let piece = ModTokens::new();
    assert_eq!(piece.health().await, Health::NotReady(NOT_BOUND));
}

#[tokio::test]
async fn h_2_a_pool_without_the_identity_port_is_not_ready() {
    let piece = ModTokens::new();
    assert!(piece.bind_pool(lazy_pool()));
    assert_eq!(piece.health().await, Health::NotReady(NOT_BOUND));
}

#[tokio::test]
async fn h_3_the_identity_port_without_a_pool_is_not_ready() {
    let piece = ModTokens::new();
    assert!(piece.late().bind_identity(Arc::new(NoIdentity)));
    assert_eq!(piece.health().await, Health::NotReady(NOT_BOUND));
}

#[tokio::test]
async fn h_4_a_piece_with_both_bound_is_ready_and_stays_ready() {
    let piece = ModTokens::new();
    assert!(piece.bind_pool(lazy_pool()));
    assert!(piece.late().bind_identity(Arc::new(NoIdentity)));
    assert_eq!(piece.health().await, Health::Ready);
    // a second bind changes nothing (the slots are set once)
    assert!(!piece.bind_pool(lazy_pool()));
    assert_eq!(piece.health().await, Health::Ready);
}

#[tokio::test]
async fn h_5_the_readiness_follows_the_binding_both_ways_on_the_same_piece() {
    let piece = ModTokens::new();
    let seen = |h: Health| matches!(h, Health::Ready);
    assert!(!seen(piece.health().await));
    piece.bind_pool(lazy_pool());
    assert!(!seen(piece.health().await));
    piece.late().bind_identity(Arc::new(NoIdentity));
    assert!(seen(piece.health().await));
    // the same answer as the store's own: `Late::ready()` is what `health()` reports
    assert_eq!(piece.late().ready(), seen(piece.health().await));
}
