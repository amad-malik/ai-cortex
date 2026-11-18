use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use axum::http::Method;
use cortex_db::{Actor, DbError, Migration, MigrationSet, ModuleTx};
use cortex_module::{
    AuditDetails, AuditSink, AuditTarget, BoxFuture, CONTRACT_VERSION, DefaultRole, Health,
    IdentityApi, Manifest, Piece, Port, PortError, Ports, PortsBuilder, Routes, ScopeClass,
    ScopeDecl, TargetRule,
};
use cortex_types::ModuleId;

use crate::validate_details;

#[cfg(test)]
#[path = "../tests/support/piece_tx.rs"]
mod tx_tests;

static MANIFEST: Manifest = Manifest {
    id: "audit",
    version: env!("CARGO_PKG_VERSION"),
    contract: CONTRACT_VERSION,
    core: true,
    schema: "audit",
    provides: &[Port::Audit],
    uses: &[Port::Identity],
    scopes: &[ScopeDecl {
        name: "audit:read",
        class: ScopeClass::Admin,
        default_roles: &[DefaultRole::ProjectLead],
    }],
    problems: &[],
};

/// The sink is published before the registry binds its identity dependency.
#[derive(Default)]
pub struct AuditPiece {
    sink: Arc<AuditWriter>,
}

impl AuditPiece {
    pub fn new() -> Self {
        Self::default()
    }
}

impl Piece for AuditPiece {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn migrations(&self) -> MigrationSet {
        MigrationSet {
            module: ModuleId::parse("audit").expect("valid module id"),
            schema: "audit",
            steps: vec![Migration {
                version: 1,
                name: "audit",
                up: include_str!("../migrations/0001_audit.up.sql"),
                down: include_str!("../migrations/0001_audit.down.sql"),
            }],
        }
    }

    fn provide(&self, ports: &mut PortsBuilder) {
        ports.audit(self.sink.clone());
    }

    fn routes(&self, ports: &Ports) -> Routes {
        if let Some(identity) = ports.identity() {
            let _ = self.sink.identity.set(identity);
        }
        Routes::new()
            .read(
                Method::GET,
                "/v2/audit",
                "audit:read",
                TargetRule::None,
                crate::read::list,
            )
            .query()
    }

    fn health(&self) -> BoxFuture<'_, Health> {
        Box::pin(async move {
            if self.sink.identity.get().is_some() {
                Health::Ready
            } else {
                Health::NotReady("audit identity not bound")
            }
        })
    }
}

#[derive(Default)]
struct AuditWriter {
    identity: OnceLock<Arc<dyn IdentityApi>>,
}

#[async_trait]
impl AuditSink for AuditWriter {
    async fn record(
        &self,
        tx: &mut ModuleTx,
        action: &'static str,
        target: AuditTarget,
        details: AuditDetails,
    ) -> Result<(), PortError> {
        let identity = self.identity.get().cloned();
        let validation = validate_details(action, &details);
        let audit = ModuleId::parse("audit").expect("valid module id");
        let result = tx.delegate(&audit, async move |tx| {
            let identity = identity.ok_or_else(|| DbError::Other("audit identity not bound".into()))?;
            validation.map_err(|_| DbError::Other("audit details invalid".into()))?;
            let ctx = tx.context().clone();
            let project = ctx.project.ok_or_else(|| DbError::Other("audit project missing".into()))?;
            let view = identity.project(tx).await.map_err(|_| DbError::Other("audit identity lookup failed".into()))?;
            if view.id != project {
                return Err(DbError::Other("audit identity project mismatch".into()));
            }
            let (actor_kind, actor_id, actor_uid) = match ctx.actor {
                Actor::Worker(id) => ("worker", Some(id.to_string()), None),
                Actor::Service(id) => ("service", Some(id.to_string()), None),
                Actor::HostOperator { uid } => ("host", None, Some(i64::from(uid))),
                Actor::System(id) => ("system", Some(id.as_str().to_owned()), None),
                Actor::Recovery => ("system", Some("recovery".to_owned()), None),
            };
            let target = match target {
                AuditTarget::None => serde_json::json!({}),
                AuditTarget::Project(id) => serde_json::json!({"project": id.to_string()}),
                AuditTarget::Team(id) => serde_json::json!({"team": id.to_string()}),
                AuditTarget::Worker(id) => serde_json::json!({"worker": id.to_string()}),
                AuditTarget::Token(id) => serde_json::json!({"token": id.to_string()}),
                AuditTarget::Record(id) => serde_json::json!({"record": id.to_string()}),
                AuditTarget::Handoff(id) => serde_json::json!({"handoff": id.to_string()}),
            };
            sqlx::query("INSERT INTO audit.records (id, installation_id, project_id, actor_kind, actor_id, actor_uid, action, target, outcome, request_id, details) VALUES ($1, $2::text::uuid, $3::text::uuid, $4, $5, $6, $7, $8::text::jsonb, 'allowed', $9::text::uuid, $10::text::jsonb)")
                .bind(uuid::Uuid::now_v7())
                .bind(view.installation.to_string())
                .bind(project.to_string())
                .bind(actor_kind)
                .bind(actor_id)
                .bind(actor_uid)
                .bind(action)
                .bind(target.to_string())
                .bind(ctx.request_id.to_string())
                .bind(details.render_json())
                .execute(tx.executor()?)
                .await
                .map_err(DbError::from)?;
            Ok(())
        }).await;
        result.map_err(PortError::from)
    }
}
