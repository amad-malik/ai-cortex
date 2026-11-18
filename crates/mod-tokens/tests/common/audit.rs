//! An `AuditSink` fixture that is a real piece with a real table, so an audit row commits and rolls
//! back with the caller's transaction (the Wave 0 plug's pattern). It records the actor, the target
//! and the rendered `details`, and can be told to refuse AFTER writing its row (a fault injected in
//! `AuditSink::record`, ID-16): the whole request must then leave nothing behind.
#![allow(dead_code)]

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cortex_db::{Migration, MigrationSet, ModuleTx};
use cortex_module::{
    AuditDetails, AuditSink, AuditTarget, BoxFuture, CONTRACT_VERSION, Health, Manifest, Piece,
    Port, PortError, Ports, PortsBuilder, Routes,
};
use cortex_types::ModuleId;

pub const ID: &str = "plug_audit";

pub const BOOTSTRAP_SQL: &str = "DO $$ BEGIN\n  BEGIN\n    CREATE ROLE cortex_m_plug_audit NOLOGIN;\n  EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL;\n  END;\n  BEGIN\n    GRANT cortex_m_plug_audit TO cortex_app;\n  EXCEPTION WHEN unique_violation THEN NULL;\n  END;\nEND $$;";

const UP: &str = r#"
CREATE SCHEMA plug_audit;
REVOKE ALL ON SCHEMA plug_audit FROM PUBLIC;
CREATE TABLE plug_audit.events (
    id         bigserial PRIMARY KEY,
    project_id uuid NOT NULL,
    action     text NOT NULL,
    actor      text NOT NULL,
    target     text NOT NULL,
    details    text NOT NULL
);
ALTER TABLE plug_audit.events ENABLE ROW LEVEL SECURITY;
ALTER TABLE plug_audit.events FORCE ROW LEVEL SECURITY;
CREATE POLICY by_project ON plug_audit.events
    USING (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid)
    WITH CHECK (project_id = nullif(current_setting('cortex.project_id', true), '')::uuid);
GRANT USAGE ON SCHEMA plug_audit TO cortex_m_plug_audit;
GRANT SELECT, INSERT ON plug_audit.events TO cortex_m_plug_audit;
GRANT USAGE ON SEQUENCE plug_audit.events_id_seq TO cortex_m_plug_audit;
"#;

static MANIFEST: Manifest = Manifest {
    id: ID,
    version: "0.0.0",
    contract: CONTRACT_VERSION,
    core: false,
    schema: ID,
    provides: &[Port::Audit],
    uses: &[],
    scopes: &[],
    problems: &[],
};

/// The token actions of mod-audit's registry, as `validate_details` has them (marv, wave1/mod-audit), with S3
/// r2 section 23 and W1-26: `token.issued` (the issue route and the FIRST bootstrap), `token.reissued` (a bootstrap
/// RE-RUN only), `token.revoked` and `token.self_revoked` (details: the reason code, nothing else).
pub const TOKEN_ACTIONS: [&str; 4] = [
    "token.issued",
    "token.reissued",
    "token.revoked",
    "token.self_revoked",
];

fn validate(action: &str, details: &AuditDetails) -> Result<(), PortError> {
    if !TOKEN_ACTIONS.contains(&action) {
        return Err(PortError::Invalid("audit action"));
    }
    let rendered: serde_json::Value =
        serde_json::from_str(&details.render_json()).map_err(|_| PortError::Invalid("details"))?;
    let keys: Vec<&String> = rendered
        .as_object()
        .map_or_else(Vec::new, |o| o.keys().collect());
    let ok = match action {
        "token.revoked" | "token.self_revoked" => keys.iter().all(|k| *k == "reason"),
        _ => keys.is_empty(),
    };
    if ok {
        Ok(())
    } else {
        Err(PortError::Invalid("audit details"))
    }
}

/// Shared with the test: the action to refuse (after its row is written), if any.
#[derive(Default)]
pub struct Fault {
    refuse: Mutex<Option<&'static str>>,
}

impl Fault {
    pub fn new() -> Arc<Fault> {
        Arc::new(Fault::default())
    }
    pub fn refuse(&self, action: &'static str) {
        *self.refuse.lock().expect("fault lock") = Some(action);
    }
    pub fn clear(&self) {
        *self.refuse.lock().expect("fault lock") = None;
    }
}

pub struct AuditPiece {
    pub fault: Arc<Fault>,
}

struct Provider {
    id: ModuleId,
    fault: Arc<Fault>,
}

fn target_text(t: &AuditTarget) -> String {
    match t {
        AuditTarget::None => "none".into(),
        AuditTarget::Project(i) => format!("project:{i}"),
        AuditTarget::Team(i) => format!("team:{i}"),
        AuditTarget::Worker(i) => format!("worker:{i}"),
        AuditTarget::Token(i) => format!("token:{i}"),
        AuditTarget::Record(i) => format!("record:{i}"),
        AuditTarget::Handoff(i) => format!("handoff:{i}"),
    }
}

#[async_trait]
impl AuditSink for Provider {
    async fn record(
        &self,
        tx: &mut ModuleTx,
        action: &'static str,
        target: AuditTarget,
        details: AuditDetails,
    ) -> Result<(), PortError> {
        // The real audit piece (mod-audit, `validate_details`) refuses an action outside its registry and a detail
        // key outside the action's contract. This fixture is not more permissive than that, so a wrong name or an
        // extra key fails here and not only in the integration (vera's TOKENS-V2). Only the token actions are
        // listed: this piece writes no other.
        validate(action, &details)?;
        let actor = format!("{:?}", tx.context().actor);
        let target = target_text(&target);
        let details = details.render_json();
        tx.delegate(&self.id, async move |t: &mut ModuleTx| {
            sqlx::query(
                "INSERT INTO plug_audit.events (project_id, action, actor, target, details) \
                 VALUES (nullif(current_setting('cortex.project_id', true), '')::uuid, $1, $2, $3, $4)",
            )
            .bind(action)
            .bind(actor)
            .bind(target)
            .bind(details)
            .execute(t.executor()?)
            .await?;
            Ok(())
        })
        .await?;
        if *self.fault.refuse.lock().expect("fault lock") == Some(action) {
            return Err(PortError::Unavailable);
        }
        Ok(())
    }
}

impl Piece for AuditPiece {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }
    fn migrations(&self) -> MigrationSet {
        MigrationSet {
            module: ModuleId::parse(ID).expect("an id"),
            schema: ID,
            steps: vec![Migration {
                version: 1,
                name: "plug_audit",
                up: UP,
                down: "DROP SCHEMA plug_audit CASCADE;",
            }],
        }
    }
    fn provide(&self, ports: &mut PortsBuilder) {
        ports.audit(Arc::new(Provider {
            id: ModuleId::parse(ID).expect("an id"),
            fault: Arc::clone(&self.fault),
        }));
    }
    fn routes(&self, _ports: &Ports) -> Routes {
        Routes::new()
    }
    fn health(&self) -> BoxFuture<'_, Health> {
        Box::pin(async { Health::Ready })
    }
}
