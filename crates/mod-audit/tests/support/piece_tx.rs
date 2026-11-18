use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use cortex_db::migrate::{apply, platform_migrations};
use cortex_db::{DbPool, HostContext, OwnerConn, TxOptions};
use cortex_module::{Assembly, ProjectView, Registry, WorkerView};
use cortex_testkit::{ctx, isolated_db, test_db};
use cortex_types::{
    InstallationId, ProblemCodeMinter, ProjectId, RequestId, SystemClock, WorkerId,
};
use sqlx::Row;

use super::*;

fn minter() -> &'static ProblemCodeMinter {
    static MINTER: OnceLock<ProblemCodeMinter> = OnceLock::new();
    MINTER.get_or_init(|| ProblemCodeMinter::take().unwrap())
}

struct FakeIdentity {
    transaction: AtomicUsize,
    fail: AtomicBool,
}

#[async_trait]
impl IdentityApi for FakeIdentity {
    async fn project(&self, tx: &mut ModuleTx) -> Result<ProjectView, PortError> {
        self.transaction
            .store(tx as *mut ModuleTx as usize, Ordering::SeqCst);
        if self.fail.load(Ordering::SeqCst) {
            return Err(PortError::NotFound);
        }
        Ok(ProjectView {
            id: tx.context().project.ok_or(PortError::NotFound)?,
            name: "fixture".into(),
            state: "active".into(),
            lead: WorkerId::parse("01930000-0000-7000-8000-000000000002").unwrap(),
            installation: InstallationId::parse("01930000-0000-7000-8000-000000000001").unwrap(),
            token_lifetime_days: 90,
        })
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

struct IdentityPiece(Arc<FakeIdentity>);
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
impl Piece for IdentityPiece {
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
        ports.identity(self.0.clone());
    }
    fn routes(&self, _: &Ports) -> Routes {
        Routes::new()
    }
    fn health(&self) -> BoxFuture<'_, Health> {
        Box::pin(async { Health::Ready })
    }
}

#[tokio::test]
async fn audit_uses_caller_transaction_and_failures_poison_it() {
    let db = test_db().await;
    let mut admin = db.admin().await;
    sqlx::raw_sql("DO $$ BEGIN BEGIN CREATE ROLE cortex_m_audit NOLOGIN; EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL; END; BEGIN CREATE ROLE cortex_install_admin NOLOGIN; EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL; END; END $$; DO $$ BEGIN GRANT cortex_m_audit TO cortex_app; EXCEPTION WHEN unique_violation THEN NULL; END $$;")
        .execute(&mut admin).await.unwrap();
    let mut owner = OwnerConn::connect(&db.owner_config()).await.unwrap();
    apply(&mut owner, &platform_migrations()).await.unwrap();
    let piece = AuditPiece::new();
    apply(&mut owner, &piece.migrations()).await.unwrap();
    let pool =
        DbPool::connect(&db.db_config(&["test_a", "audit", "identity"], Arc::new(SystemClock)))
            .await
            .unwrap();
    let project = ProjectId::new(&SystemClock);

    let mut unbound = ModuleTx::begin(&pool, ctx("test_a", Some(project)), TxOptions::default())
        .await
        .unwrap();
    sqlx::query("INSERT INTO test_a.t (project_id, v) VALUES ($1::text::uuid, 'unbound')")
        .bind(project.to_string())
        .execute(unbound.executor().unwrap())
        .await
        .unwrap();
    assert!(
        piece
            .sink
            .record(
                &mut unbound,
                "token.issued",
                AuditTarget::None,
                AuditDetails::new()
            )
            .await
            .is_err()
    );
    assert!(unbound.commit().await.is_err());

    let identity = Arc::new(FakeIdentity {
        transaction: AtomicUsize::new(0),
        fail: AtomicBool::new(false),
    });
    let minter = minter();
    let enabled = [
        ModuleId::parse("audit").unwrap(),
        ModuleId::parse("identity").unwrap(),
    ];
    let registry = Registry::assemble(Assembly {
        compiled: vec![Box::new(piece), Box::new(IdentityPiece(identity.clone()))],
        enabled: &enabled,
        minter,
        chain_codes: &[],
    })
    .unwrap();
    let sink = registry.ports().audit().unwrap();
    let replacement = Arc::new(FakeIdentity {
        transaction: AtomicUsize::new(0),
        fail: AtomicBool::new(true),
    });
    let another = Registry::assemble(Assembly {
        compiled: vec![
            Box::new(AuditPiece::new()),
            Box::new(IdentityPiece(replacement.clone())),
        ],
        enabled: &enabled,
        minter,
        chain_codes: &[],
    })
    .unwrap();
    registry
        .pieces()
        .find(|piece| piece.manifest().id == "audit")
        .unwrap()
        .routes(another.ports());
    let mut tx = ModuleTx::begin(&pool, ctx("test_a", Some(project)), TxOptions::default())
        .await
        .unwrap();
    sqlx::query("INSERT INTO test_a.t (project_id, v) VALUES ($1::text::uuid, 'committed')")
        .bind(project.to_string())
        .execute(tx.executor().unwrap())
        .await
        .unwrap();
    let pointer = &mut tx as *mut ModuleTx as usize;
    sink.record(
        &mut tx,
        "token.issued",
        AuditTarget::None,
        AuditDetails::new(),
    )
    .await
    .unwrap();
    assert_eq!(identity.transaction.load(Ordering::SeqCst), pointer);
    assert_eq!(replacement.transaction.load(Ordering::SeqCst), 0);
    tx.commit().await.unwrap();

    let row = sqlx::query("SELECT installation_id::text, project_id::text, actor_kind, actor_id, action, target::text, outcome, request_id IS NOT NULL AS has_request, details::text FROM audit.records")
        .fetch_one(&mut admin).await.unwrap();
    assert_eq!(
        row.get::<String, _>(0),
        "01930000-0000-7000-8000-000000000001"
    );
    assert_eq!(row.get::<String, _>(1), project.to_string());
    assert_eq!(row.get::<String, _>(2), "system");
    assert_eq!(row.get::<String, _>(3), "test_a");
    assert_eq!(row.get::<String, _>(4), "token.issued");
    assert_eq!(row.get::<String, _>(5), "{}");
    assert_eq!(row.get::<String, _>(6), "allowed");
    assert!(row.get::<bool, _>(7));
    assert_eq!(row.get::<String, _>(8), "{}");

    identity.fail.store(true, Ordering::SeqCst);
    let mut tx = ModuleTx::begin(&pool, ctx("test_a", Some(project)), TxOptions::default())
        .await
        .unwrap();
    sqlx::query("INSERT INTO test_a.t (project_id, v) VALUES ($1::text::uuid, 'failed')")
        .bind(project.to_string())
        .execute(tx.executor().unwrap())
        .await
        .unwrap();
    assert!(
        sink.record(
            &mut tx,
            "token.issued",
            AuditTarget::None,
            AuditDetails::new()
        )
        .await
        .is_err()
    );
    assert!(tx.commit().await.is_err());

    let writes: i64 = sqlx::query_scalar("SELECT count(*) FROM test_a.t")
        .fetch_one(&mut admin)
        .await
        .unwrap();
    let audits: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.records")
        .fetch_one(&mut admin)
        .await
        .unwrap();
    assert_eq!((writes, audits), (1, 1));
}

#[tokio::test]
async fn bootstrap_audit_stays_in_host_transaction_and_restores_host_role() {
    let db = isolated_db("audit_host").await;
    let mut admin = db.admin().await;
    sqlx::raw_sql("DO $$ BEGIN BEGIN CREATE ROLE cortex_m_audit NOLOGIN; EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL; END; END $$; DO $$ BEGIN GRANT cortex_m_audit TO cortex_app; EXCEPTION WHEN unique_violation THEN NULL; END $$;")
        .execute(&mut admin).await.unwrap();
    let mut owner = OwnerConn::connect(&db.owner_config()).await.unwrap();
    apply(&mut owner, &platform_migrations()).await.unwrap();
    let piece = AuditPiece::new();
    apply(&mut owner, &piece.migrations()).await.unwrap();
    let pool = DbPool::connect(&db.db_config(&["audit", "identity"], Arc::new(SystemClock)))
        .await
        .unwrap();
    let identity = Arc::new(FakeIdentity {
        transaction: AtomicUsize::new(0),
        fail: AtomicBool::new(false),
    });
    let minter = minter();
    let enabled = [
        ModuleId::parse("audit").unwrap(),
        ModuleId::parse("identity").unwrap(),
    ];
    let registry = Registry::assemble(Assembly {
        compiled: vec![Box::new(piece), Box::new(IdentityPiece(identity.clone()))],
        enabled: &enabled,
        minter,
        chain_codes: &[],
    })
    .unwrap();
    let sink = registry.ports().audit().unwrap();
    let project = ProjectId::new(&SystemClock);
    let lead = WorkerId::parse("01930000-0000-7000-8000-000000000002").unwrap();
    let details = AuditDetails::new()
        .id("project", project)
        .unwrap()
        .id("lead", lead)
        .unwrap()
        .flag("created", true)
        .unwrap()
        .flag("reissued", false)
        .unwrap();
    let host = HostContext {
        uid: 1000,
        project: Some(project),
        request_id: RequestId::new(&SystemClock),
    };
    let mut tx = ModuleTx::begin_host(&pool, host, TxOptions::default())
        .await
        .unwrap();
    let pointer = &mut tx as *mut ModuleTx as usize;
    sink.record(
        &mut tx,
        "project.bootstrapped",
        AuditTarget::Project(project),
        details,
    )
    .await
    .unwrap();
    assert_eq!(identity.transaction.load(Ordering::SeqCst), pointer);
    let current: String = sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(tx.executor().unwrap())
        .await
        .unwrap();
    assert_eq!(current, "cortex_install_admin");
    tx.commit().await.unwrap();
    let row = sqlx::query("SELECT actor_kind, actor_uid, actor_id, project_id::text, action, details->>'created', details->>'reissued' FROM audit.records").fetch_one(&mut admin).await.unwrap();
    assert_eq!(row.get::<String, _>(0), "host");
    assert_eq!(row.get::<i64, _>(1), 1000);
    assert_eq!(row.get::<Option<String>, _>(2), None);
    assert_eq!(row.get::<String, _>(3), project.to_string());
    assert_eq!(row.get::<String, _>(4), "project.bootstrapped");
    assert_eq!(row.get::<String, _>(5), "true");
    assert_eq!(row.get::<String, _>(6), "false");
}

/// vera's gate-4 ruling, row 25 (9 Nov): `audit.records.actor_uid` must hold the WHOLE accepted host-UID domain, a
/// `u32`, and not only the `i32` range (a domain user from SSSD or AD, or a user namespace, can have a uid above
/// `i32::MAX`, and the writer used to refuse it, failing the audit write and the operation). The boundary uids 0, 1000,
/// `i32::MAX`, `i32::MAX + 1` and `u32::MAX` are written through the real sink and read back EXACTLY, and the column is a bigint.
#[tokio::test]
async fn host_uid_covers_the_whole_u32_domain() {
    let db = isolated_db("audit_uid").await;
    let mut admin = db.admin().await;
    sqlx::raw_sql("DO $$ BEGIN BEGIN CREATE ROLE cortex_m_audit NOLOGIN; EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL; END; END $$; DO $$ BEGIN GRANT cortex_m_audit TO cortex_app; EXCEPTION WHEN unique_violation THEN NULL; END $$;")
        .execute(&mut admin).await.unwrap();
    let mut owner = OwnerConn::connect(&db.owner_config()).await.unwrap();
    apply(&mut owner, &platform_migrations()).await.unwrap();
    let piece = AuditPiece::new();
    apply(&mut owner, &piece.migrations()).await.unwrap();
    let pool = DbPool::connect(&db.db_config(&["audit", "identity"], Arc::new(SystemClock)))
        .await
        .unwrap();
    let identity = Arc::new(FakeIdentity {
        transaction: AtomicUsize::new(0),
        fail: AtomicBool::new(false),
    });
    let minter = minter();
    let enabled = [
        ModuleId::parse("audit").unwrap(),
        ModuleId::parse("identity").unwrap(),
    ];
    let registry = Registry::assemble(Assembly {
        compiled: vec![Box::new(piece), Box::new(IdentityPiece(identity.clone()))],
        enabled: &enabled,
        minter,
        chain_codes: &[],
    })
    .unwrap();
    let sink = registry.ports().audit().unwrap();
    let project = ProjectId::new(&SystemClock);
    let lead = WorkerId::parse("01930000-0000-7000-8000-000000000002").unwrap();
    let uids: [u32; 5] = [0, 1000, i32::MAX as u32, i32::MAX as u32 + 1, u32::MAX];
    let mut refused = Vec::new();
    for uid in uids {
        let details = AuditDetails::new()
            .id("project", project)
            .unwrap()
            .id("lead", lead)
            .unwrap()
            .flag("created", true)
            .unwrap()
            .flag("reissued", false)
            .unwrap();
        let host = HostContext {
            uid,
            project: Some(project),
            request_id: RequestId::new(&SystemClock),
        };
        let mut tx = ModuleTx::begin_host(&pool, host, TxOptions::default())
            .await
            .unwrap();
        match sink
            .record(
                &mut tx,
                "project.bootstrapped",
                AuditTarget::Project(project),
                details,
            )
            .await
        {
            Ok(()) => tx.commit().await.unwrap(),
            Err(e) => refused.push((uid, format!("{e:?}"))),
        }
    }
    assert!(
        refused.is_empty(),
        "host uids refused by the audit writer: {refused:?}"
    );
    let rows: Vec<i64> = sqlx::query_scalar(
        "SELECT actor_uid FROM audit.records WHERE actor_kind = 'host' ORDER BY id",
    )
    .fetch_all(&mut admin)
    .await
    .unwrap();
    assert_eq!(
        rows,
        vec![
            0,
            1000,
            i64::from(i32::MAX),
            i64::from(i32::MAX) + 1,
            i64::from(u32::MAX)
        ]
    );
    let data_type: String = sqlx::query_scalar("SELECT data_type::text FROM information_schema.columns WHERE table_schema = 'audit' AND table_name = 'records' AND column_name = 'actor_uid'")
        .fetch_one(&mut admin)
        .await
        .unwrap();
    assert_eq!(data_type, "bigint");
}
