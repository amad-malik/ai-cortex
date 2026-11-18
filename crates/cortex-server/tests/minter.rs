//! The REAL `Minter` (finn's `issue_in_host_tx`) behind the real `DbStore`, against a real database (contract r0.5 section 6; W1-37;
//! hale N-4). DB-ONLY. The audit sink is still a fake (mod-audit is not merged). RED first.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cortex_db::migrate::{apply, platform_migrations};
use cortex_db::{DbPool, ModuleTx, OwnerConn};
use cortex_module::{AuditDetails, AuditSink, AuditTarget, Piece, PortError};
use cortex_server::bootstrap::{BootstrapStore, Found};
use cortex_server::minter::TokenMinter;
use cortex_server::store::DbStore;
use cortex_testkit::{TestDb, empty_db};
use cortex_types::SystemClock;
use mod_identity::{BOOTSTRAP_SQL, ModIdentity, Provider};
use mod_tokens::{Entropy, EntropyError, ModTokens, OsEntropy};

const PROJECT: &str = "zz-mint-project";
const LEAD: &str = "zz-mint-lead";

async fn setup(tag: &str) -> (TestDb, DbPool) {
    let db = empty_db(tag).await;
    let mut admin = db.admin().await;
    sqlx::raw_sql(BOOTSTRAP_SQL)
        .execute(&mut admin)
        .await
        .expect("identity bootstrap");
    sqlx::raw_sql(mod_tokens::BOOTSTRAP_SQL)
        .execute(&mut admin)
        .await
        .expect("tokens bootstrap");
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
    apply(&mut owner, &ModTokens::new().migrations())
        .await
        .expect("tokens");
    let pool = DbPool::connect(&db.db_config(
        &["test_a", mod_identity::ID, mod_tokens::ID],
        Arc::new(SystemClock),
    ))
    .await
    .expect("pool");
    (db, pool)
}

/// Every audit action the real issue and the store record, in order.
#[derive(Default)]
struct Actions(Mutex<Vec<String>>);

struct RecordingAudit(Arc<Actions>);

#[async_trait]
impl AuditSink for RecordingAudit {
    async fn record(
        &self,
        _tx: &mut ModuleTx,
        action: &'static str,
        _target: AuditTarget,
        _details: AuditDetails,
    ) -> Result<(), PortError> {
        self.0.0.lock().unwrap().push(action.to_owned());
        Ok(())
    }
}

struct NoEntropy;

impl Entropy for NoEntropy {
    fn fill(&self, _buf: &mut [u8]) -> Result<(), EntropyError> {
        Err(EntropyError)
    }
}

/// (tokens, active tokens, tokens revoked as 'reissued') in the tokens table.
async fn token_rows(db: &TestDb) -> (i64, i64, i64) {
    let mut admin = db.admin().await;
    let q = |sql: &'static str| sqlx::query_scalar::<_, i64>(sql);
    let all = q("SELECT count(*) FROM tokens.tokens")
        .fetch_one(&mut admin)
        .await
        .expect("count");
    let active = q("SELECT count(*) FROM tokens.tokens WHERE revoked_at IS NULL")
        .fetch_one(&mut admin)
        .await
        .expect("count");
    let reissued = q("SELECT count(*) FROM tokens.tokens WHERE revoke_reason = 'reissued'")
        .fetch_one(&mut admin)
        .await
        .expect("count");
    (all, active, reissued)
}

async fn projects(db: &TestDb) -> i64 {
    let mut admin = db.admin().await;
    sqlx::query_scalar("SELECT count(*) FROM identity.projects")
        .fetch_one(&mut admin)
        .await
        .expect("count")
}

#[tokio::test]
async fn mn_01_the_first_run_issues_the_leads_token_in_the_same_transaction() {
    let (db, pool) = setup("mn01").await;
    let actions = Arc::new(Actions::default());
    let (audit, prov) = (RecordingAudit(actions.clone()), Provider::new());
    let minter = TokenMinter::new(&prov, &audit, &SystemClock, &OsEntropy);
    let mut s = DbStore::new(&pool, &prov, &audit, &minter, &SystemClock, 1000);
    assert_eq!(s.find(PROJECT, LEAD).await.expect("find"), Found::Nothing);
    let token = s.create_or_reissue(PROJECT, LEAD).await.expect("create");
    assert_eq!(
        token_rows(&db).await,
        (0, 0, 0),
        "nothing is visible before the commit"
    );
    s.commit().await.expect("commit");
    assert_eq!(token_rows(&db).await, (1, 1, 0));
    assert_eq!(projects(&db).await, 1);
    // The row holds the DIGEST of the token, never the token.
    let mut admin = db.admin().await;
    let ok: bool = sqlx::query_scalar("SELECT digest = sha256($1::bytea) FROM tokens.tokens")
        .bind(token.expose().as_bytes())
        .fetch_one(&mut admin)
        .await
        .expect("digest");
    assert!(ok, "the stored digest is the SHA-256 of the returned token");
    let seen = actions.0.lock().unwrap().clone();
    assert!(
        seen.iter().any(|a| a == "token.issued")
            && seen.iter().any(|a| a == "project.bootstrapped"),
        "both audit rows were written in the transaction: {seen:?}"
    );
}

#[tokio::test]
async fn mn_02_a_re_run_issues_a_new_token_and_supersedes_the_old_one() {
    let (db, pool) = setup("mn02").await;
    let actions = Arc::new(Actions::default());
    let (audit, prov) = (RecordingAudit(actions.clone()), Provider::new());
    let minter = TokenMinter::new(&prov, &audit, &SystemClock, &OsEntropy);
    let mut s = DbStore::new(&pool, &prov, &audit, &minter, &SystemClock, 1000);
    s.find(PROJECT, LEAD).await.expect("find");
    let first = s.create_or_reissue(PROJECT, LEAD).await.expect("create");
    s.commit().await.expect("commit");
    let mut s = DbStore::new(&pool, &prov, &audit, &minter, &SystemClock, 1000);
    assert_eq!(
        s.find(PROJECT, LEAD).await.expect("find"),
        Found::ProjectAndLead
    );
    let second = s.create_or_reissue(PROJECT, LEAD).await.expect("reissue");
    s.commit().await.expect("commit");
    assert_ne!(first.expose(), second.expose());
    assert_eq!(
        token_rows(&db).await,
        (2, 1, 1),
        "two rows, one active, the earlier one superseded as 'reissued'"
    );
    assert_eq!(projects(&db).await, 1);
    let seen = actions.0.lock().unwrap().clone();
    assert!(seen.iter().any(|a| a == "token.reissued"), "{seen:?}");
}

/// hale N-4, the entropy half: a mint that fails on ENTROPY (drawn before any write) refuses the issue and the old token stays valid. This
/// cell does not need a rollback to pass; the rollback after writes is pm_1 and pm_3.
#[tokio::test]
async fn mn_03_a_failing_mint_on_a_reissue_keeps_the_old_token_valid() {
    let (db, pool) = setup("mn03").await;
    let actions = Arc::new(Actions::default());
    let (audit, prov) = (RecordingAudit(actions.clone()), Provider::new());
    let good = TokenMinter::new(&prov, &audit, &SystemClock, &OsEntropy);
    let mut s = DbStore::new(&pool, &prov, &audit, &good, &SystemClock, 1000);
    s.find(PROJECT, LEAD).await.expect("find");
    s.create_or_reissue(PROJECT, LEAD).await.expect("create");
    s.commit().await.expect("commit");
    let before = token_rows(&db).await;
    assert_eq!(before, (1, 1, 0));
    let bad = TokenMinter::new(&prov, &audit, &SystemClock, &NoEntropy);
    let mut s = DbStore::new(&pool, &prov, &audit, &bad, &SystemClock, 1000);
    s.find(PROJECT, LEAD).await.expect("find");
    let e = s.create_or_reissue(PROJECT, LEAD).await.err();
    assert!(e.is_some(), "no entropy: the issue is refused");
    s.rollback().await;
    assert_eq!(
        token_rows(&db).await,
        before,
        "the old token is still the one active token; nothing was superseded"
    );
}

/// A first run whose mint fails on ENTROPY leaves nothing: no project, no lead, no token (nothing was written; the rollback after writes is pm_2).
#[tokio::test]
async fn mn_04_a_failing_first_mint_leaves_no_project_and_no_token() {
    let (db, pool) = setup("mn04").await;
    let actions = Arc::new(Actions::default());
    let (audit, prov) = (RecordingAudit(actions.clone()), Provider::new());
    let bad = TokenMinter::new(&prov, &audit, &SystemClock, &NoEntropy);
    let mut s = DbStore::new(&pool, &prov, &audit, &bad, &SystemClock, 1000);
    s.find(PROJECT, LEAD).await.expect("find");
    assert!(s.create_or_reissue(PROJECT, LEAD).await.err().is_some());
    s.rollback().await;
    assert_eq!(token_rows(&db).await, (0, 0, 0));
    assert_eq!(projects(&db).await, 0);
}

/// hale F-1 (packet 5): mn_03 and mn_04 fail on ENTROPY, which finn's issue draws BEFORE any write, so they pass with no rollback at all
/// (her mutant X1: DbStore keeps its transaction after a failed mint survived all four). These cells fail the audit sink AFTER the writes:
/// pm_1 (re-run: the real issue's own audit record), pm_2 (first run), pm_3 (the store's own project.bootstrapped record after a good real
/// mint), pm_4 (two re-runs: one active token, each row holds its own digest). The sink here reads the transaction at the moment it fails.
struct FailOn {
    action: &'static str,
    seen: Arc<Actions>,
    inside: Arc<Mutex<Vec<String>>>,
}

#[async_trait]
impl AuditSink for FailOn {
    async fn record(
        &self,
        tx: &mut ModuleTx,
        action: &'static str,
        _target: AuditTarget,
        _details: AuditDetails,
    ) -> Result<(), PortError> {
        self.seen.0.lock().unwrap().push(action.to_owned());
        if action == self.action {
            // what the transaction holds at the moment of failure
            let n: Result<(i64, i64), _> = match tx.executor() {
                Ok(ex) => sqlx::query_as::<_, (i64, i64)>(
                    "SELECT (SELECT count(*) FROM tokens.tokens), (SELECT count(*) FROM tokens.tokens WHERE revoked_at IS NULL)",
                )
                .fetch_one(ex)
                .await
                .map_err(|e| e.to_string()),
                Err(e) => Err(format!("{e:?}")),
            };
            self.inside.lock().unwrap().push(format!(
                "{action}: inside the transaction (tokens, active) = {n:?}"
            ));
            Err(PortError::Invalid("test"))
        } else {
            Ok(())
        }
    }
}

fn fail_on(action: &'static str) -> (Arc<Actions>, Arc<Mutex<Vec<String>>>, FailOn) {
    let seen = Arc::new(Actions::default());
    let inside = Arc::new(Mutex::new(Vec::new()));
    (
        seen.clone(),
        inside.clone(),
        FailOn {
            action,
            seen,
            inside,
        },
    )
}

async fn first_run(pool: &DbPool, prov: &Provider, audit: &RecordingAudit) -> cortex_types::Secret {
    let minter = TokenMinter::new(prov, audit, &SystemClock, &OsEntropy);
    let mut s = DbStore::new(pool, prov, audit, &minter, &SystemClock, 1000);
    s.find(PROJECT, LEAD).await.expect("find");
    let t = s.create_or_reissue(PROJECT, LEAD).await.expect("create");
    s.commit().await.expect("commit");
    t
}

#[tokio::test]
async fn pm_1_an_audit_failure_inside_the_real_issue_after_the_writes_keeps_the_old_token_valid() {
    let (db, pool) = setup("pm1").await;
    let good = RecordingAudit(Arc::new(Actions::default()));
    let prov = Provider::new();
    let old = first_run(&pool, &prov, &good).await;
    let before = token_rows(&db).await;
    assert_eq!(before, (1, 1, 0));
    let (seen, inside, bad) = fail_on("token.reissued");
    let minter = TokenMinter::new(&prov, &bad, &SystemClock, &OsEntropy);
    let mut s = DbStore::new(&pool, &prov, &bad, &minter, &SystemClock, 1000);
    s.find(PROJECT, LEAD).await.expect("find");
    let e = s.create_or_reissue(PROJECT, LEAD).await.err();
    let c = s.commit().await;
    assert!(
        seen.0.lock().unwrap().iter().any(|a| a == "token.reissued"),
        "the failing sink was reached"
    );
    let inside = inside.lock().unwrap().clone();
    assert!(
        inside.iter().any(|l| l.contains("Ok((2, 1))")),
        "at the moment of failure the supersede and the insert were pending in the transaction: {inside:?}"
    );
    assert!(e.is_some(), "the issue is refused");
    assert!(c.is_err(), "nothing is left to commit");
    assert_eq!(
        token_rows(&db).await,
        before,
        "the old token is still the one active token"
    );
    let mut admin = db.admin().await;
    let (active_old, replacements): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM tokens.tokens WHERE digest = sha256($1::bytea) AND revoked_at IS NULL), \
                (SELECT count(*) FROM tokens.tokens WHERE digest <> sha256($1::bytea))",
    )
    .bind(old.expose().as_bytes())
    .fetch_one(&mut admin)
    .await
    .expect("digest check");
    assert_eq!(
        (active_old, replacements),
        (1, 0),
        "vera SERVER-I5-L1: the OLD token's digest is still the one active row and no replacement exists"
    );
}

#[tokio::test]
async fn pm_2_an_audit_failure_inside_the_real_issue_of_a_first_run_leaves_nothing() {
    let (db, pool) = setup("pm2").await;
    let (seen, inside, bad) = fail_on("token.issued");
    let prov = Provider::new();
    let minter = TokenMinter::new(&prov, &bad, &SystemClock, &OsEntropy);
    let mut s = DbStore::new(&pool, &prov, &bad, &minter, &SystemClock, 1000);
    s.find(PROJECT, LEAD).await.expect("find");
    let e = s.create_or_reissue(PROJECT, LEAD).await.err();
    let c = s.commit().await;
    assert!(
        seen.0.lock().unwrap().iter().any(|a| a == "token.issued"),
        "the failing sink was reached"
    );
    let inside = inside.lock().unwrap().clone();
    assert!(
        inside.iter().any(|l| l.contains("Ok((1, 1))")),
        "at the moment of failure the new token row was pending in the transaction: {inside:?}"
    );
    assert!(e.is_some() && c.is_err());
    assert_eq!((token_rows(&db).await, projects(&db).await), ((0, 0, 0), 0));
}

#[tokio::test]
async fn pm_3_the_stores_own_audit_row_failing_after_a_good_real_mint_rolls_the_mint_back() {
    // The atomicity the packet claims ("the lead's token row, its audit row, the project, the lead and the project.bootstrapped row commit or roll back together"):
    // the REAL mint succeeds, then the store's own project.bootstrapped record fails.
    let (db, pool) = setup("pm3").await;
    let good = RecordingAudit(Arc::new(Actions::default()));
    let prov = Provider::new();
    let old = first_run(&pool, &prov, &good).await;
    let before = token_rows(&db).await;
    let (seen, _inside, bad) = fail_on("project.bootstrapped");
    let minter = TokenMinter::new(&prov, &bad, &SystemClock, &OsEntropy);
    let mut s = DbStore::new(&pool, &prov, &bad, &minter, &SystemClock, 1000);
    s.find(PROJECT, LEAD).await.expect("find");
    let e = s.create_or_reissue(PROJECT, LEAD).await.err();
    let c = s.commit().await;
    assert!(
        seen.0.lock().unwrap().iter().any(|a| a == "token.reissued"),
        "the real issue wrote its own audit record first"
    );
    assert!(
        seen.0
            .lock()
            .unwrap()
            .iter()
            .any(|a| a == "project.bootstrapped"),
        "the store's own record was reached and failed"
    );
    assert!(e.is_some() && c.is_err());
    assert_eq!(
        token_rows(&db).await,
        before,
        "the real mint's writes were rolled back with the failed store audit row"
    );
    let mut admin = db.admin().await;
    let (active_old, replacements): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM tokens.tokens WHERE digest = sha256($1::bytea) AND revoked_at IS NULL), \
                (SELECT count(*) FROM tokens.tokens WHERE digest <> sha256($1::bytea))",
    )
    .bind(old.expose().as_bytes())
    .fetch_one(&mut admin)
    .await
    .expect("digest check");
    assert_eq!(
        (active_old, replacements),
        (1, 0),
        "vera SERVER-I5-L1: the OLD token's digest is still the one active row and no replacement exists"
    );
}

#[tokio::test]
async fn pm_4_two_reissues_leave_one_active_token_and_each_row_holds_its_own_digest() {
    let (db, pool) = setup("pm4").await;
    let audit = RecordingAudit(Arc::new(Actions::default()));
    let prov = Provider::new();
    let t1 = first_run(&pool, &prov, &audit).await;
    let mut tokens = vec![t1];
    for _ in 0..2 {
        let minter = TokenMinter::new(&prov, &audit, &SystemClock, &OsEntropy);
        let mut s = DbStore::new(&pool, &prov, &audit, &minter, &SystemClock, 1000);
        s.find(PROJECT, LEAD).await.expect("find");
        tokens.push(s.create_or_reissue(PROJECT, LEAD).await.expect("reissue"));
        s.commit().await.expect("commit");
    }
    let rows = token_rows(&db).await;
    let mut admin = db.admin().await;
    let mut digest_ok = Vec::new();
    for (i, t) in tokens.iter().enumerate() {
        let (n, active): (i64, bool) = sqlx::query_as("SELECT count(*), bool_or(revoked_at IS NULL) FROM tokens.tokens WHERE digest = sha256($1::bytea)")
            .bind(t.expose().as_bytes())
            .fetch_one(&mut admin)
            .await
            .expect("digest lookup");
        digest_ok.push((i, n, active));
    }
    assert_eq!(rows, (3, 1, 2));
    assert_eq!(digest_ok, vec![(0, 1, false), (1, 1, false), (2, 1, true)]);
}
