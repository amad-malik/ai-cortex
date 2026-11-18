//! The real `BootstrapStore` against a real database (contract r0.4 section 5; W1-35 A; W1-18). DB-ONLY: needs `supervise.sh`'s throwaway
//! Postgres. The token mint and the audit sink are fakes (mod-tokens and mod-audit are not merged yet); identity, the host path and the
//! transactions are REAL. RED first.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cortex_db::migrate::{apply, platform_migrations};
use cortex_db::{DbPool, ModuleTx, OwnerConn};
use cortex_module::{AuditDetails, AuditSink, AuditTarget, Piece, PortError};
use cortex_server::bootstrap::{BootstrapStore, Found};
use cortex_server::store::{DbStore, MAX_ATTEMPTS, MintError, Minter};
use cortex_testkit::{TestDb, empty_db};
use cortex_types::{Secret, SystemClock, WorkerId};
use mod_identity::{BOOTSTRAP_SQL, ModIdentity, Provider};

const PROJECT: &str = "zz-store-project";
const LEAD: &str = "zz-store-lead";

async fn setup(tag: &str) -> (TestDb, DbPool) {
    let db = empty_db(tag).await;
    let mut admin = db.admin().await;
    sqlx::raw_sql(BOOTSTRAP_SQL)
        .execute(&mut admin)
        .await
        .expect("bootstrap");
    sqlx::raw_sql("DO $$ BEGIN CREATE ROLE cortex_install_admin NOLOGIN; EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL; END $$;")
        .execute(&mut admin)
        .await
        .expect("install admin role");
    let mut owner = OwnerConn::connect(&db.owner_config()).await.expect("owner");
    apply(&mut owner, &platform_migrations())
        .await
        .expect("platform");
    apply(&mut owner, &ModIdentity::new().migrations())
        .await
        .expect("identity");
    let pool = DbPool::connect(&db.db_config(&["test_a", mod_identity::ID], Arc::new(SystemClock)))
        .await
        .expect("pool");
    (db, pool)
}

#[derive(Default)]
struct Calls {
    mints: Vec<(WorkerId, bool)>,
    audits: Vec<(String, bool, bool)>, // action, created, reissued
    /// hale N-2: the exact detail KEYS (sorted) and whether the target is the project, per audit call.
    shapes: Vec<(Vec<String>, bool)>,
}

struct FakeMinter {
    calls: Arc<Mutex<Calls>>,
    fail: bool,
}

#[async_trait]
impl Minter for FakeMinter {
    async fn mint(
        &self,
        _tx: &mut ModuleTx,
        lead: WorkerId,
        reissue: bool,
    ) -> Result<Secret, MintError> {
        self.calls.lock().unwrap().mints.push((lead, reissue));
        if self.fail {
            Err(MintError)
        } else {
            Ok(Secret::new("ctx2_FAKE-TOKEN-FOR-THE-TEST"))
        }
    }
}

struct FakeAudit {
    calls: Arc<Mutex<Calls>>,
    fail: bool,
}

#[async_trait]
impl AuditSink for FakeAudit {
    async fn record(
        &self,
        _tx: &mut ModuleTx,
        action: &'static str,
        target: AuditTarget,
        details: AuditDetails,
    ) -> Result<(), PortError> {
        use cortex_module::DetailValue;
        {
            let mut keys: Vec<String> = details
                .entries()
                .iter()
                .map(|(k, _)| (*k).to_owned())
                .collect();
            keys.sort();
            self.calls
                .lock()
                .unwrap()
                .shapes
                .push((keys, matches!(target, AuditTarget::Project(_))));
        }
        let flag = |k: &str| {
            details
                .entries()
                .iter()
                .any(|(key, v)| *key == k && matches!(v, DetailValue::Flag(true)))
        };
        self.calls.lock().unwrap().audits.push((
            action.to_owned(),
            flag("created"),
            flag("reissued"),
        ));
        if self.fail {
            Err(PortError::Invalid("test"))
        } else {
            Ok(())
        }
    }
}

fn store<'a>(
    pool: &'a DbPool,
    p: &'a Provider,
    a: &'a FakeAudit,
    m: &'a FakeMinter,
) -> DbStore<'a> {
    DbStore::new(pool, p, a, m, &SystemClock, 1000)
}

async fn counts(db: &TestDb) -> (i64, i64) {
    let mut admin = db.admin().await;
    let p: i64 = sqlx::query_scalar("SELECT count(*) FROM identity.projects")
        .fetch_one(&mut admin)
        .await
        .expect("count");
    let w: i64 = sqlx::query_scalar("SELECT count(*) FROM identity.workers")
        .fetch_one(&mut admin)
        .await
        .expect("count");
    (p, w)
}

#[tokio::test]
async fn st_01_the_first_run_creates_the_project_and_lead_mints_and_audits_in_one_transaction() {
    let (db, pool) = setup("st01").await;
    let calls = Arc::new(Mutex::new(Calls::default()));
    let (m, a, prov) = (
        FakeMinter {
            calls: calls.clone(),
            fail: false,
        },
        FakeAudit {
            calls: calls.clone(),
            fail: false,
        },
        Provider::new(),
    );
    let mut s = store(&pool, &prov, &a, &m);
    assert_eq!(s.find(PROJECT, LEAD).await.expect("find"), Found::Nothing);
    assert_eq!(counts(&db).await, (0, 0), "find writes nothing");
    let token = s.create_or_reissue(PROJECT, LEAD).await.expect("create");
    assert_eq!(token.expose(), "ctx2_FAKE-TOKEN-FOR-THE-TEST");
    assert_eq!(
        counts(&db).await,
        (0, 0),
        "nothing is visible before the commit"
    );
    s.commit().await.expect("commit");
    let (p, w) = counts(&db).await;
    assert_eq!((p, w), (1, 1), "one project, its lead");
    let c = calls.lock().unwrap();
    assert_eq!(c.mints.len(), 1);
    assert!(!c.mints[0].1, "a first run is not a reissue");
    assert_eq!(
        c.shapes,
        vec![(
            vec![
                "created".to_owned(),
                "lead".to_owned(),
                "project".to_owned(),
                "reissued".to_owned()
            ],
            true
        )],
        "the detail keys are exactly project, lead, created, reissued (mod-audit's allow-list) and the target is the project"
    );
    assert_eq!(
        c.audits,
        vec![("project.bootstrapped".to_owned(), true, false)],
        "exactly one audit row, created and not reissued"
    );
}

#[tokio::test]
async fn st_02_a_re_run_finds_the_pair_and_reissues_without_creating() {
    let (db, pool) = setup("st02").await;
    let calls = Arc::new(Mutex::new(Calls::default()));
    let (m, a, prov) = (
        FakeMinter {
            calls: calls.clone(),
            fail: false,
        },
        FakeAudit {
            calls: calls.clone(),
            fail: false,
        },
        Provider::new(),
    );
    {
        let mut s = store(&pool, &prov, &a, &m);
        s.find(PROJECT, LEAD).await.expect("find");
        s.create_or_reissue(PROJECT, LEAD).await.expect("create");
        s.commit().await.expect("commit");
    }
    calls.lock().unwrap().audits.clear();
    let mut s = store(&pool, &prov, &a, &m);
    assert_eq!(
        s.find(PROJECT, LEAD).await.expect("find"),
        Found::ProjectAndLead
    );
    s.create_or_reissue(PROJECT, LEAD).await.expect("reissue");
    s.commit().await.expect("commit");
    assert_eq!(counts(&db).await, (1, 1), "nothing new was created");
    let c = calls.lock().unwrap();
    assert!(c.mints.last().expect("a mint").1, "a re-run is a reissue");
    assert_eq!(
        c.audits,
        vec![("project.bootstrapped".to_owned(), false, true)]
    );
}

#[tokio::test]
async fn st_03_find_tells_the_three_existing_cases_apart() {
    let (_db, pool) = setup("st03").await;
    let calls = Arc::new(Mutex::new(Calls::default()));
    let (m, a, prov) = (
        FakeMinter {
            calls: calls.clone(),
            fail: false,
        },
        FakeAudit {
            calls: calls.clone(),
            fail: false,
        },
        Provider::new(),
    );
    let mut s = store(&pool, &prov, &a, &m);
    s.find(PROJECT, LEAD).await.expect("find");
    s.create_or_reissue(PROJECT, LEAD).await.expect("create");
    s.commit().await.expect("commit");
    let mut s = store(&pool, &prov, &a, &m);
    assert_eq!(
        s.find("zz-other-project", LEAD).await.expect("find"),
        Found::Nothing,
        "another project: nothing"
    );
    assert_eq!(
        s.find(PROJECT, "zz-unknown-lead").await.expect("find"),
        Found::ProjectOnly,
        "the project exists, the lead does not"
    );
}

/// hale N-1 and vera (packet 3): `find`'s third answer was unobserved (st_03 asserted two of the three). A worker who is on the
/// project's roster under the lead's name but is NOT the project's lead (here: added by admin SQL, as hale did in his probe) is
/// `LeadNotProjectLead`, not `ProjectOnly` and not `ProjectAndLead`. Without the roster scan this cell fails.
#[tokio::test]
async fn st_03b_find_tells_a_roster_member_who_is_not_the_lead_apart() {
    let (db, pool) = setup("st03b").await;
    let calls = Arc::new(Mutex::new(Calls::default()));
    let (m, a, prov) = (
        FakeMinter {
            calls: calls.clone(),
            fail: false,
        },
        FakeAudit {
            calls: calls.clone(),
            fail: false,
        },
        Provider::new(),
    );
    let mut s = store(&pool, &prov, &a, &m);
    s.find(PROJECT, LEAD).await.expect("find");
    s.create_or_reissue(PROJECT, LEAD).await.expect("create");
    s.commit().await.expect("commit");
    let mut admin = db.admin().await;
    let project: String = sqlx::query_scalar("SELECT id::text FROM identity.projects")
        .fetch_one(&mut admin)
        .await
        .expect("the project");
    let member = WorkerId::new(&SystemClock).to_string();
    sqlx::query(
        "INSERT INTO identity.workers (id, project_id, kind) VALUES ($1::uuid, $2::uuid, 'worker')",
    )
    .bind(&member)
    .bind(&project)
    .execute(&mut admin)
    .await
    .expect("a member who is not the lead");
    sqlx::query(
        "INSERT INTO identity.names (entity_kind, entity_id, scope_id, project_id, name_key, label, current) \
         VALUES ('worker', $1::uuid, $2::uuid, $2::uuid, 'zz-member', 'zz-member', true)",
    )
    .bind(&member)
    .bind(&project)
    .execute(&mut admin)
    .await
    .expect("its name");
    let mut s = store(&pool, &prov, &a, &m);
    assert_eq!(
        s.find(PROJECT, "zz-member").await.expect("find"),
        Found::LeadNotProjectLead,
        "on the roster, not the project's lead"
    );
    assert_eq!(
        s.find(PROJECT, LEAD).await.expect("find"),
        Found::ProjectAndLead,
        "the real lead is still the lead"
    );
}

#[tokio::test]
async fn st_04_a_different_lead_for_an_existing_project_is_a_conflict_and_changes_nothing() {
    let (db, pool) = setup("st04").await;
    let calls = Arc::new(Mutex::new(Calls::default()));
    let (m, a, prov) = (
        FakeMinter {
            calls: calls.clone(),
            fail: false,
        },
        FakeAudit {
            calls: calls.clone(),
            fail: false,
        },
        Provider::new(),
    );
    let mut s = store(&pool, &prov, &a, &m);
    s.find(PROJECT, LEAD).await.expect("find");
    s.create_or_reissue(PROJECT, LEAD).await.expect("create");
    s.commit().await.expect("commit");
    let mut s = store(&pool, &prov, &a, &m);
    s.find(PROJECT, "zz-someone-else").await.expect("find");
    let e = s
        .create_or_reissue(PROJECT, "zz-someone-else")
        .await
        .err()
        .expect("a conflict");
    s.rollback().await;
    assert!(
        !format!("{e:?}").contains(PROJECT) && !format!("{e:?}").contains("zz-someone-else"),
        "no name in the error: {e:?}"
    );
    assert_eq!(counts(&db).await, (1, 1));
}

#[tokio::test]
async fn st_05_a_failing_mint_rolls_the_whole_transaction_back() {
    let (db, pool) = setup("st05").await;
    let calls = Arc::new(Mutex::new(Calls::default()));
    let (m, a, prov) = (
        FakeMinter {
            calls: calls.clone(),
            fail: true,
        },
        FakeAudit {
            calls: calls.clone(),
            fail: false,
        },
        Provider::new(),
    );
    let mut s = store(&pool, &prov, &a, &m);
    s.find(PROJECT, LEAD).await.expect("find");
    s.create_or_reissue(PROJECT, LEAD)
        .await
        .err()
        .expect("the mint fails");
    s.rollback().await;
    assert_eq!(
        counts(&db).await,
        (0, 0),
        "the project and lead are NOT left behind"
    );
    assert!(
        calls.lock().unwrap().audits.is_empty(),
        "no audit row after a failed mint"
    );
}

#[tokio::test]
async fn st_06_a_failing_audit_rolls_the_whole_transaction_back() {
    let (db, pool) = setup("st06").await;
    let calls = Arc::new(Mutex::new(Calls::default()));
    let (m, a, prov) = (
        FakeMinter {
            calls: calls.clone(),
            fail: false,
        },
        FakeAudit {
            calls: calls.clone(),
            fail: true,
        },
        Provider::new(),
    );
    let mut s = store(&pool, &prov, &a, &m);
    s.find(PROJECT, LEAD).await.expect("find");
    s.create_or_reissue(PROJECT, LEAD)
        .await
        .err()
        .expect("the audit fails");
    s.rollback().await;
    assert_eq!(
        counts(&db).await,
        (0, 0),
        "the project, the lead and the token are NOT left behind"
    );
}

#[tokio::test]
async fn st_07_a_rollback_after_success_leaves_nothing_the_file_write_failed_case() {
    let (db, pool) = setup("st07").await;
    let calls = Arc::new(Mutex::new(Calls::default()));
    let (m, a, prov) = (
        FakeMinter {
            calls: calls.clone(),
            fail: false,
        },
        FakeAudit {
            calls: calls.clone(),
            fail: false,
        },
        Provider::new(),
    );
    let mut s = store(&pool, &prov, &a, &m);
    s.find(PROJECT, LEAD).await.expect("find");
    s.create_or_reissue(PROJECT, LEAD).await.expect("create");
    s.rollback().await;
    assert_eq!(counts(&db).await, (0, 0));
}

#[tokio::test]
async fn st_08_two_concurrent_first_runs_make_one_project_and_both_succeed() {
    let (db, pool) = setup("st08").await;
    let calls = Arc::new(Mutex::new(Calls::default()));
    let (m, a, prov) = (
        FakeMinter {
            calls: calls.clone(),
            fail: false,
        },
        FakeAudit {
            calls: calls.clone(),
            fail: false,
        },
        Provider::new(),
    );
    let run = || async {
        let mut s = store(&pool, &prov, &a, &m);
        s.find(PROJECT, LEAD).await.expect("find");
        s.create_or_reissue(PROJECT, LEAD)
            .await
            .expect("create (maybe after OtherProject)");
        s.commit().await.expect("commit");
    };
    tokio::join!(run(), run());
    assert_eq!(counts(&db).await, (1, 1), "ONE project and ONE lead");
    let c = calls.lock().unwrap();
    assert_eq!(
        c.audits.iter().filter(|(_, created, _)| *created).count(),
        1,
        "exactly one run created it"
    );
    assert_eq!(
        c.audits.iter().filter(|(_, _, reissued)| *reissued).count(),
        1,
        "the other reissued"
    );
}

/// vera (packet 3): st_08 races two runs, so whether the OtherProject retry actually ran depends on timing. This cell forces it with no
/// timing at all: run B's `find` sees NOTHING (and so plans a fresh project id); run A then creates and commits the project; only then
/// does B create. B's first attempt must meet the project under A's id (`OtherProject`) and succeed by retrying with that id, as a
/// reissue. Without the retry B fails; with a second project it would be (2, 2).
#[tokio::test]
async fn st_08b_a_stale_find_is_retried_with_the_found_project_and_ends_as_a_reissue() {
    let (db, pool) = setup("st08b").await;
    let calls = Arc::new(Mutex::new(Calls::default()));
    let (m, a, prov) = (
        FakeMinter {
            calls: calls.clone(),
            fail: false,
        },
        FakeAudit {
            calls: calls.clone(),
            fail: false,
        },
        Provider::new(),
    );
    let mut b = store(&pool, &prov, &a, &m);
    assert_eq!(
        b.find(PROJECT, LEAD).await.expect("find"),
        Found::Nothing,
        "B looks first and sees nothing"
    );
    let mut first = store(&pool, &prov, &a, &m);
    first.find(PROJECT, LEAD).await.expect("find");
    first
        .create_or_reissue(PROJECT, LEAD)
        .await
        .expect("create");
    first.commit().await.expect("commit");
    b.create_or_reissue(PROJECT, LEAD)
        .await
        .expect("B retries with the project that now exists");
    b.commit().await.expect("commit");
    assert_eq!(counts(&db).await, (1, 1), "ONE project and ONE lead");
    let c = calls.lock().unwrap();
    assert_eq!(
        c.audits.iter().filter(|(_, created, _)| *created).count(),
        1
    );
    assert_eq!(
        c.audits.iter().filter(|(_, _, reissued)| *reissued).count(),
        1,
        "B ended as a reissue"
    );
}

#[test]
fn st_09_the_attempt_cap_is_three() {
    assert_eq!(MAX_ATTEMPTS, 3);
}
