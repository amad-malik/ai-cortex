//! S3 r2.7 audit list through the real auth chain and a disposable PostgreSQL.
use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::http::Request;
use cortex_db::migrate::{apply, platform_migrations};
use cortex_db::{DbPool, OwnerConn};
use cortex_http::chain::{Chain, Limits, LogSink, router};
use cortex_http::request_id::RequestIds;
use cortex_module::route::AppState;
use cortex_module::{
    AuthError, Piece, Ports, Principal, Role, ScopePolicy, Target, TargetRule, TokenVerifier,
    WorkerKind,
};
use cortex_testkit::isolated_db;
use cortex_types::{ProjectId, ScopeName, Secret, SystemClock, Timestamp, TokenId, WorkerId};
use mod_audit::AuditPiece;
use serde_json::Value;
use tower::ServiceExt;

const PROJECT_A: &str = "01930000-0000-7000-8000-0000000000a1";
const PROJECT_B: &str = "01930000-0000-7000-8000-0000000000b1";
const INSTALL: &str = "01930000-0000-7000-8000-000000000001";

struct Verifier;
#[async_trait]
impl TokenVerifier for Verifier {
    async fn verify(&self, bearer: &Secret) -> Result<Principal, AuthError> {
        let (project, role) = match bearer.expose() {
            "lead-a" => (PROJECT_A, Role::ProjectLead),
            "lead-b" => (PROJECT_B, Role::ProjectLead),
            "member-a" => (PROJECT_A, Role::Member(None)),
            _ => return Err(AuthError::Unknown),
        };
        Ok(Principal {
            worker: WorkerId::parse("01930000-0000-7000-8000-000000000002").unwrap(),
            project: ProjectId::parse(project).unwrap(),
            kind: WorkerKind::Worker,
            role,
            token: TokenId::new(&SystemClock),
            expires_at: Timestamp::from_unix_micros(4_102_444_799_000_000),
            renew_due: false,
        })
    }
}

struct Policy;
impl ScopePolicy for Policy {
    fn allows(&self, p: &Principal, scope: &ScopeName) -> bool {
        scope.as_str() == "audit:read" && p.role == Role::ProjectLead
    }
    fn target(&self, _: &Principal, _: TargetRule, _: &Target) -> bool {
        true
    }
}

async fn get(app: &axum::Router, path: &str, token: Option<&str>) -> (u16, Value, String) {
    let mut request = Request::builder().uri(path);
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status().as_u16();
    let text = String::from_utf8(
        to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    let json = serde_json::from_str(&text).unwrap_or(Value::Null);
    (status, json, text)
}

#[tokio::test]
async fn audit_list_is_scoped_strict_and_keyset_paged() {
    let db = isolated_db("audit_http").await;
    let mut admin = db.admin().await;
    sqlx::raw_sql("DO $$ BEGIN BEGIN CREATE ROLE cortex_m_audit NOLOGIN; EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL; END; END $$; DO $$ BEGIN GRANT cortex_m_audit TO cortex_app; EXCEPTION WHEN unique_violation THEN NULL; END $$;")
        .execute(&mut admin).await.unwrap();
    let mut owner = OwnerConn::connect(&db.owner_config()).await.unwrap();
    apply(&mut owner, &platform_migrations()).await.unwrap();
    apply(&mut owner, &AuditPiece::new().migrations())
        .await
        .unwrap();
    for (suffix, project, action, at) in [
        (
            "0001",
            Some(PROJECT_A),
            "token.issued",
            "2026-11-06T12:00:00.000001Z",
        ),
        (
            "0002",
            Some(PROJECT_A),
            "token.revoked",
            "2026-11-06T12:00:00.000001Z",
        ),
        (
            "0003",
            Some(PROJECT_A),
            "token.issued",
            "2026-11-06T12:00:00.000002Z",
        ),
        (
            "0004",
            Some(PROJECT_B),
            "foreign.canary",
            "2026-11-06T12:00:00.000001Z",
        ),
        ("0005", None, "host.canary", "2026-11-06T12:00:00.000001Z"),
    ] {
        let id = format!("01930000-0000-7000-8000-00000000{suffix}");
        sqlx::query("INSERT INTO audit.records (id, installation_id, project_id, actor_kind, actor_id, action, target, outcome, request_id, details, at) VALUES ($1::text::uuid, $2::text::uuid, $3::text::uuid, 'system', 'fixture', $4, '{\"worker\":\"01930000-0000-7000-8000-000000000002\"}'::jsonb, 'allowed', NULL, '{}'::jsonb, $5::text::timestamptz)")
            .bind(id).bind(INSTALL).bind(project).bind(action).bind(at).execute(&mut admin).await.unwrap();
    }
    let pool = DbPool::connect(&db.db_config(&["audit"], Arc::new(SystemClock)))
        .await
        .unwrap();
    let chain = Chain {
        verifier: Arc::new(Verifier),
        policy: Arc::new(Policy),
        log: LogSink::new(|_| {}),
        limits: Limits::default(),
    };
    let app = router(
        chain,
        AppState {
            pool: Arc::new(pool),
        },
        RequestIds {
            clock: Arc::new(SystemClock),
        },
        vec![("audit", AuditPiece::new().routes(&Ports::default()))],
    )
    .unwrap();

    assert_eq!(get(&app, "/v2/audit", None).await.0, 401);
    let denied = get(&app, "/v2/audit", Some("member-a")).await;
    assert_eq!(
        (denied.0, denied.1["type"].as_str()),
        (403, Some("missing_scope"))
    );
    for query in [
        "?chain=project",
        "?limit=0",
        "?limit=201",
        "?limit=bad",
        "?after=bogus",
    ] {
        let bad = get(&app, &format!("/v2/audit{query}"), Some("lead-a")).await;
        assert_eq!(
            (bad.0, bad.1["type"].as_str()),
            (422, Some("invalid_request")),
            "{query}: {}",
            bad.2
        );
    }
    let mut path = "/v2/audit?limit=1".to_owned();
    let mut ids = Vec::new();
    loop {
        let (status, body, text) = get(&app, &path, Some("lead-a")).await;
        assert_eq!(status, 200, "{text}");
        assert!(!text.contains("foreign.canary") && !text.contains("host.canary"));
        let items = body["items"].as_array().unwrap();
        for item in items {
            let keys: BTreeSet<_> = item
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            assert_eq!(
                keys,
                BTreeSet::from([
                    "id",
                    "at",
                    "action",
                    "actor_kind",
                    "actor_id",
                    "target",
                    "outcome",
                    "request_id",
                    "details"
                ])
            );
            ids.push(item["id"].as_str().unwrap().to_owned());
        }
        match body["next"].as_str() {
            Some(cursor) => path = format!("/v2/audit?limit=1&after={cursor}"),
            None => break,
        }
        assert!(ids.len() < 5, "pagination did not terminate");
    }
    assert_eq!(
        ids,
        [
            "01930000-0000-7000-8000-000000000001",
            "01930000-0000-7000-8000-000000000002",
            "01930000-0000-7000-8000-000000000003"
        ]
    );
    let b = get(&app, "/v2/audit", Some("lead-b")).await;
    assert_eq!(b.0, 200);
    assert_eq!(b.1["items"].as_array().unwrap().len(), 1);
    assert_eq!(b.1["items"][0]["action"], "foreign.canary");
    assert!(!b.2.contains("host.canary"));
}

#[tokio::test]
async fn audit_default_and_maximum_page_sizes_keep_every_tied_row() {
    let db = isolated_db("audit_limits").await;
    let mut admin = db.admin().await;
    sqlx::raw_sql("DO $$ BEGIN BEGIN CREATE ROLE cortex_m_audit NOLOGIN; EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL; END; END $$; DO $$ BEGIN GRANT cortex_m_audit TO cortex_app; EXCEPTION WHEN unique_violation THEN NULL; END $$;")
        .execute(&mut admin).await.unwrap();
    let mut owner = OwnerConn::connect(&db.owner_config()).await.unwrap();
    apply(&mut owner, &platform_migrations()).await.unwrap();
    apply(&mut owner, &AuditPiece::new().migrations())
        .await
        .unwrap();
    sqlx::query("INSERT INTO audit.records (id, installation_id, project_id, actor_kind, action, outcome, at) SELECT ('01930000-0000-7000-8000-00000000' || lpad(n::text, 4, '0'))::uuid, $1::text::uuid, $2::text::uuid, 'host', 'token.issued', 'allowed', '2026-11-06T12:00:00.000001Z'::timestamptz FROM generate_series(1, 201) AS n")
        .bind(INSTALL).bind(PROJECT_A).execute(&mut admin).await.unwrap();
    sqlx::query("INSERT INTO audit.records (id, installation_id, project_id, actor_kind, action, outcome, at) VALUES ('01930000-0000-7000-8000-000000009999'::uuid, $1::text::uuid, $2::text::uuid, 'host', 'foreign.canary', 'allowed', '2026-11-06T12:00:00.000001Z'::timestamptz)")
        .bind(INSTALL).bind(PROJECT_B).execute(&mut admin).await.unwrap();
    let pool = DbPool::connect(&db.db_config(&["audit"], Arc::new(SystemClock)))
        .await
        .unwrap();
    let chain = Chain {
        verifier: Arc::new(Verifier),
        policy: Arc::new(Policy),
        log: LogSink::new(|_| {}),
        limits: Limits::default(),
    };
    let app = router(
        chain,
        AppState {
            pool: Arc::new(pool),
        },
        RequestIds {
            clock: Arc::new(SystemClock),
        },
        vec![("audit", AuditPiece::new().routes(&Ports::default()))],
    )
    .unwrap();

    let (status, default, text) = get(&app, "/v2/audit", Some("lead-a")).await;
    assert_eq!(status, 200, "{text}");
    assert_eq!(default["items"].as_array().unwrap().len(), 50);
    assert!(default["next"].is_string());
    let a_cursor = default["next"].as_str().unwrap();
    let (status, foreign, text) =
        get(&app, &format!("/v2/audit?after={a_cursor}"), Some("lead-b")).await;
    assert_eq!(status, 200, "{text}");
    assert_eq!(foreign["items"].as_array().unwrap().len(), 1);
    assert_eq!(foreign["items"][0]["action"], "foreign.canary");
    assert!(
        !text.contains("token.issued"),
        "cursor must not widen the token's project"
    );
    let (status, first, text) = get(&app, "/v2/audit?limit=200", Some("lead-a")).await;
    assert_eq!(status, 200, "{text}");
    let first_items = first["items"].as_array().unwrap();
    assert_eq!(first_items.len(), 200);
    let cursor = first["next"].as_str().unwrap();
    let (status, last, text) = get(
        &app,
        &format!("/v2/audit?limit=200&after={cursor}"),
        Some("lead-a"),
    )
    .await;
    assert_eq!(status, 200, "{text}");
    assert!(last["next"].is_null());
    let last_items = last["items"].as_array().unwrap();
    assert_eq!(last_items.len(), 1);
    let ids: BTreeSet<_> = first_items
        .iter()
        .chain(last_items)
        .map(|item| item["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), 201);
}
