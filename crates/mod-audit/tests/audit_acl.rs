//! AUDIT-V1: real migrations must deny TRUNCATE under hostile creator defaults.
use cortex_db::OwnerConn;
use cortex_db::migrate::{apply, platform_migrations};
use cortex_module::Piece;
use cortex_testkit::{TestDb, empty_db};
use mod_audit::AuditPiece;
use sqlx::Connection;

async fn fixture(tag: &str, hostile: bool) -> TestDb {
    let db = empty_db(tag).await;
    let mut admin = db.admin().await;
    sqlx::raw_sql("DO $$ BEGIN BEGIN CREATE ROLE cortex_m_audit NOLOGIN; EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL; END; END $$;")
        .execute(&mut admin).await.unwrap();
    if hostile {
        sqlx::raw_sql("ALTER DEFAULT PRIVILEGES FOR ROLE cortex_owner GRANT ALL ON TABLES TO PUBLIC; \
            ALTER DEFAULT PRIVILEGES FOR ROLE cortex_owner GRANT ALL ON TABLES TO cortex_m_audit WITH GRANT OPTION; \
            ALTER DEFAULT PRIVILEGES FOR ROLE cortex_owner GRANT ALL ON FUNCTIONS TO cortex_m_test_a;")
            .execute(&mut admin).await.unwrap();
    }
    let mut owner = OwnerConn::connect(&db.owner_config()).await.unwrap();
    apply(&mut owner, &platform_migrations()).await.unwrap();
    apply(&mut owner, &AuditPiece::new().migrations())
        .await
        .unwrap();
    db
}

async fn check(db: &TestDb) {
    let mut admin = db.admin().await;
    sqlx::raw_sql("INSERT INTO audit.records (id, installation_id, project_id, actor_kind, action, outcome) VALUES \
      ('01930000-0000-7000-8000-000000000001','01930000-0000-7000-8000-000000000010','01930000-0000-7000-8000-000000000020','system','token.issued','allowed'), \
      ('01930000-0000-7000-8000-000000000002','01930000-0000-7000-8000-000000000010','01930000-0000-7000-8000-000000000021','system','token.issued','allowed')")
        .execute(&mut admin).await.unwrap();
    let mut tx = admin.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE cortex_m_audit")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(
        "SELECT set_config('cortex.project_id', '01930000-0000-7000-8000-000000000020', true)",
    )
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query("INSERT INTO audit.records (id, installation_id, project_id, actor_kind, action, outcome) VALUES ('01930000-0000-7000-8000-000000000003','01930000-0000-7000-8000-000000000010','01930000-0000-7000-8000-000000000020','system','token.issued','allowed')")
        .execute(&mut *tx).await.expect("audit role can insert");
    let visible: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.records")
        .fetch_one(&mut *tx)
        .await
        .expect("audit role can select");
    assert_eq!(visible, 2, "only the selected project's rows are visible");
    tx.commit().await.unwrap();

    let mut tx = admin.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE cortex_m_audit")
        .execute(&mut *tx)
        .await
        .unwrap();
    let truncate = sqlx::query("TRUNCATE audit.records")
        .execute(&mut *tx)
        .await;
    tx.rollback().await.unwrap(); // Also protects the fixture when the RED control succeeds.
    let code = truncate
        .as_ref()
        .err()
        .and_then(|e| e.as_database_error())
        .and_then(|e| e.code());
    assert_eq!(
        code.as_deref(),
        Some("42501"),
        "audit TRUNCATE must be denied"
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.records")
        .fetch_one(&mut admin)
        .await
        .unwrap();
    assert_eq!(
        count, 3,
        "both projects' rows remain after refused TRUNCATE"
    );
    let rows: Vec<(String, String, bool)> = sqlx::query_as("SELECT CASE WHEN a.grantee=0 THEN 'PUBLIC' ELSE pg_get_userbyid(a.grantee)::text END, a.privilege_type::text, a.is_grantable FROM pg_class c CROSS JOIN LATERAL aclexplode(coalesce(c.relacl, acldefault('r', c.relowner))) a WHERE c.oid='audit.records'::regclass AND a.grantee<>c.relowner ORDER BY 1,2,3")
        .fetch_all(&mut admin).await.unwrap();
    assert_eq!(
        rows,
        vec![
            ("cortex_install_admin".into(), "INSERT".into(), false),
            ("cortex_install_admin".into(), "SELECT".into(), false),
            ("cortex_m_audit".into(), "INSERT".into(), false),
            ("cortex_m_audit".into(), "SELECT".into(), false),
        ],
        "non-owner table grants must match exactly, without grant option"
    );
}

#[tokio::test]
async fn audit_acl_clean_defaults_keep_append_only() {
    check(&fixture("audit_acl_clean", false).await).await;
}

#[tokio::test]
async fn audit_acl_hostile_defaults_keep_append_only() {
    check(&fixture("audit_acl_hostile", true).await).await;
}

#[tokio::test]
async fn audit_acl_readback_detects_excess_missing_and_grant_options() {
    let db = fixture("audit_acl_readback", false).await;
    let mut admin = db.admin().await;
    let migration = include_str!("../migrations/0001_audit.up.sql");
    let (_, guard) = migration
        .split_once("-- Fail closed if any grant is extra, missing, or carries grant option.\n")
        .expect("exact ACL readback is present");
    sqlx::raw_sql(sqlx::AssertSqlSafe(guard.to_owned()))
        .execute(&mut admin)
        .await
        .expect("unaltered ACL passes the same migration readback");
    for mutation in [
        "GRANT TRUNCATE ON audit.records TO cortex_m_test_a",
        "REVOKE SELECT ON audit.records FROM cortex_m_audit",
        "GRANT SELECT ON audit.records TO cortex_m_audit WITH GRANT OPTION",
        "GRANT EXECUTE ON FUNCTION audit.refuse_change() TO cortex_m_test_a",
        "GRANT CREATE ON SCHEMA audit TO cortex_m_audit",
    ] {
        let mut tx = admin.begin().await.unwrap();
        sqlx::raw_sql(sqlx::AssertSqlSafe(mutation.to_owned()))
            .execute(&mut *tx)
            .await
            .unwrap();
        let result = sqlx::raw_sql(sqlx::AssertSqlSafe(guard.to_owned()))
            .execute(&mut *tx)
            .await;
        let code = result
            .as_ref()
            .err()
            .and_then(|e| e.as_database_error())
            .and_then(|e| e.code());
        assert_eq!(
            code.as_deref(),
            Some("42501"),
            "readback missed: {mutation}"
        );
        tx.rollback().await.unwrap();
    }
}
