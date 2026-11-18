//! Wave 0 plug, DB-backed, sam's checklist P-03 and P-12 for the provider piece (S1's default
//! migration runner, no guard):
//! - **P-03:** a deliberately bad later step rolls back its changes and its ledger row; a step
//!   whose checksum changed after it was applied stops startup (`ChecksumChanged`).
//! - **P-12:** a piece that is compiled but disabled is not migrated, provides no port and mounts
//!   no route, and its schema and rows are left as they are.
//!
//! Needs `supervise.sh`'s throwaway Postgres. Default features.
use std::sync::OnceLock;

use cortex_db::migrate::{apply, platform_migrations};
use cortex_db::{MigrateError, Migration, MigrationSet, OwnerConn};
use cortex_module::{Assembly, BoxFuture, Health, Manifest, Piece, Port, Ports, Registry, Routes};
use cortex_testkit::empty_db;
use cortex_types::{ModuleId, ProblemCodeMinter};
use plug_identity::{BOOTSTRAP_SQL, ID, PlugIdentity};

fn minter() -> &'static ProblemCodeMinter {
    static M: OnceLock<ProblemCodeMinter> = OnceLock::new();
    M.get_or_init(|| ProblemCodeMinter::take().expect("the first take in this process"))
}

async fn fresh(tag: &str) -> (cortex_testkit::TestDb, OwnerConn, sqlx::PgConnection) {
    let db = empty_db(tag).await;
    let mut admin = db.admin().await;
    sqlx::raw_sql(BOOTSTRAP_SQL)
        .execute(&mut admin)
        .await
        .expect("bootstrap");
    let mut owner = OwnerConn::connect(&db.owner_config()).await.expect("owner");
    apply(&mut owner, &platform_migrations())
        .await
        .expect("platform");
    (db, owner, admin)
}

async fn ledger(admin: &mut sqlx::PgConnection) -> Vec<i32> {
    sqlx::query_scalar("SELECT version FROM platform.migrations WHERE module = $1 ORDER BY version")
        .bind(ID)
        .fetch_all(admin)
        .await
        .expect("ledger")
}

async fn exists(admin: &mut sqlx::PgConnection, rel: &str) -> bool {
    sqlx::query_scalar("SELECT to_regclass($1) IS NOT NULL")
        .bind(rel)
        .fetch_one(admin)
        .await
        .expect("to_regclass")
}

/// The piece's real set, plus extra steps.
fn with_steps(extra: Vec<Migration>) -> MigrationSet {
    let mut set = PlugIdentity::new().migrations();
    set.steps.extend(extra);
    set
}

#[tokio::test]
async fn w0_p03_a_bad_later_step_rolls_back_with_its_ledger_row() {
    let (_db, mut owner, mut admin) = fresh("p03_bad").await;
    let bad = with_steps(vec![Migration {
        version: 2,
        name: "bad",
        up: "CREATE TABLE plug_identity.half_done (a int); SELECT 1 / 0;",
        down: "DROP TABLE plug_identity.half_done;",
    }]);
    let r = apply(&mut owner, &bad).await;
    assert!(matches!(r, Err(MigrateError::Db(_))), "{r:?}");
    assert_eq!(ledger(&mut admin).await, [1], "step 1 recorded, step 2 not");
    assert!(exists(&mut admin, "plug_identity.workers").await);
    assert!(
        !exists(&mut admin, "plug_identity.half_done").await,
        "the bad step's table rolled back"
    );
    // Control: the same step without the error applies and is recorded.
    let good = with_steps(vec![Migration {
        version: 2,
        name: "good",
        up: "CREATE TABLE plug_identity.half_done (a int);",
        down: "DROP TABLE plug_identity.half_done;",
    }]);
    apply(&mut owner, &good)
        .await
        .expect("the good step applies");
    assert_eq!(ledger(&mut admin).await, [1, 2]);
    assert!(exists(&mut admin, "plug_identity.half_done").await);
}

#[tokio::test]
async fn w0_p03_a_changed_applied_step_stops_startup() {
    let (_db, mut owner, mut admin) = fresh("p03_sum").await;
    apply(&mut owner, &PlugIdentity::new().migrations())
        .await
        .expect("applies");
    let mut changed = PlugIdentity::new().migrations();
    changed.steps[0].up = "-- edited after release\nCREATE SCHEMA plug_identity;";
    assert_eq!(
        apply(&mut owner, &changed).await,
        Err(MigrateError::ChecksumChanged { version: 1 })
    );
    assert_eq!(ledger(&mut admin).await, [1], "nothing new recorded");
    // Control: the unchanged set is a no-op.
    apply(&mut owner, &PlugIdentity::new().migrations())
        .await
        .expect("the unchanged set applies nothing");
    assert_eq!(ledger(&mut admin).await, [1]);
}

/// A consumer that uses nothing, so the provider can be left out.
struct Other;
static OTHER: Manifest = Manifest {
    id: "plug_other",
    version: "0.0.1",
    contract: 1,
    core: false,
    schema: "plug_other",
    provides: &[],
    uses: &[],
    scopes: &[],
    problems: &[],
};
impl Piece for Other {
    fn manifest(&self) -> &'static Manifest {
        &OTHER
    }
    fn migrations(&self) -> MigrationSet {
        MigrationSet {
            module: ModuleId::parse("plug_other").expect("id"),
            schema: "plug_other",
            steps: vec![],
        }
    }
    fn routes(&self, _ports: &Ports) -> Routes {
        Routes::new()
    }
    fn health(&self) -> BoxFuture<'_, Health> {
        Box::pin(async { Health::Ready })
    }
}

#[tokio::test]
async fn w0_p12_a_disabled_piece_is_not_migrated_wired_or_mounted_and_its_data_stays() {
    let (_db, mut owner, mut admin) = fresh("p12_disabled").await;
    // A first start with the piece enabled: migrated, and a row written.
    apply(&mut owner, &PlugIdentity::new().migrations())
        .await
        .expect("applies");
    sqlx::query("INSERT INTO plug_identity.projects (id, name, lead) VALUES (gen_random_uuid(), 'kept', gen_random_uuid())")
        .execute(&mut admin)
        .await
        .expect("seed");
    // The next start leaves it out of [modules] enabled.
    let enabled = [ModuleId::parse("plug_other").expect("id")];
    let reg = Registry::assemble(Assembly {
        compiled: vec![
            Box::new(PlugIdentity::new()) as Box<dyn Piece>,
            Box::new(Other) as Box<dyn Piece>,
        ],
        enabled: &enabled,
        minter: minter(),
        chain_codes: &["internal_error", "unavailable", "busy", "conflict"],
    })
    .expect("assembles without the provider");
    assert!(
        !reg.ports().has(Port::Identity),
        "no port from a disabled piece"
    );
    assert_eq!(
        reg.routes().map(|(p, _)| p).collect::<Vec<_>>(),
        ["plug_other"],
        "no route from a disabled piece"
    );
    // The server migrates only the registry's pieces.
    let bumped = with_steps(vec![Migration {
        version: 2,
        name: "would_run_if_enabled",
        up: "CREATE TABLE plug_identity.not_while_disabled (a int);",
        down: "DROP TABLE plug_identity.not_while_disabled;",
    }]);
    for p in reg.pieces() {
        let set = if p.manifest().id == ID {
            bumped.clone()
        } else {
            p.migrations()
        };
        apply(&mut owner, &set)
            .await
            .expect("migrate an enabled piece");
    }
    assert_eq!(
        ledger(&mut admin).await,
        [1],
        "the disabled piece was not migrated"
    );
    assert!(!exists(&mut admin, "plug_identity.not_while_disabled").await);
    let kept: i64 =
        sqlx::query_scalar("SELECT count(*) FROM plug_identity.projects WHERE name = 'kept'")
            .fetch_one(&mut admin)
            .await
            .expect("count");
    assert_eq!(kept, 1, "its data is left as it was");
}
