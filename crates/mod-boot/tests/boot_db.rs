//! mod-boot's DB cells (stage E): the seam's real side on a real `ModuleTx`, under the role
//! `cortex_m_boot`, with the Wave 0 identity fixture (`plug-identity`) as a REAL `IdentityApi`
//! provider under row-level security. Records and handoffs are fakes until their pieces land (the
//! real bindings are packet 2's).
//!
//! Rows: BOOT-01 (identity from a real provider, the principal's worker only), BOOT-04 (another
//! project's worker is invisible under row-level security), BOOT-50 (boot's transaction writes
//! nothing: no transaction id is ever assigned), BOOT-52 (a `Db` error from a port poisons the
//! transaction, and nothing is returned).
//!
//! Needs `supervise.sh`'s throwaway Postgres (`CV2_PG_PORT`, `CV2_PG_PASSWORD`). Run ONLY as:
//!
//! ```text
//! CV2_DB=1 CV2_ADMIT_LOG=1 /path/to/workspace/spikes/cortex-v2-s0/admission/cv2-admit \
//!     timeout --kill-after=120 780 crates/cortex-testkit/supervise.sh \
//!     cargo test -p mod-boot --test boot_db --no-fail-fast
//! ```
mod support;

use std::sync::Arc;

use async_trait::async_trait;
use cortex_db::migrate::{apply, platform_migrations};
use cortex_db::{Actor, DbError, DbPool, ModuleTx, OwnerConn, TxContext, TxOptions};
use cortex_module::HandoffTo;
use cortex_module::{
    HandoffView, HandoffsApi, Piece, PortError, Principal, Problem, RecordCursor, RecordPage,
    RecordView, RecordsApi, Role, SectionName, SectionView, WorkerKind,
};
use cortex_types::{ModuleId, ProjectId, RecordId, RequestId, SystemClock, TokenId, WorkerId};
use mod_boot::seam::{PortReads, gather};
use mod_boot::{Boot, assemble};
use plug_identity::{PlugIdentity, Provider};
use support::{CANARY, T_HANDOFF, at, orientation, path5, rules, section_text, uuid, view};

/// Records: the sections it is given; `fail` makes `section_current` run a failing statement in
/// a delegated call, as a real provider's SQL error would.
struct Records {
    sections: Vec<(SectionName, SectionView)>,
    fail: bool,
}

#[async_trait]
impl RecordsApi for Records {
    async fn get(
        &self,
        _tx: &mut ModuleTx,
        _ids: &[RecordId],
    ) -> Result<Vec<RecordView>, PortError> {
        Ok(Vec::new())
    }
    async fn changed_since(
        &self,
        _tx: &mut ModuleTx,
        _after: Option<RecordCursor>,
        _limit: u32,
    ) -> Result<RecordPage, PortError> {
        Ok(RecordPage {
            records: Vec::new(),
            next: None,
        })
    }
    async fn section_current(
        &self,
        tx: &mut ModuleTx,
        name: &SectionName,
    ) -> Result<Option<SectionView>, PortError> {
        if self.fail {
            let m = ModuleId::parse(plug_identity::ID).expect("a module id");
            tx.delegate(&m, async |t: &mut ModuleTx| {
                sqlx::query("SELECT 1/0")
                    .execute(t.executor()?)
                    .await
                    .map(|_| ())
                    .map_err(|_| DbError::Other("22012".to_owned()))
            })
            .await?;
        }
        Ok(self
            .sections
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.clone()))
    }
}

/// Handoffs: none claimed, none pending.
struct NoHandoffs;

#[async_trait]
impl HandoffsApi for NoHandoffs {
    async fn claimed_by(
        &self,
        _tx: &mut ModuleTx,
        _w: &WorkerId,
        _limit: u32,
    ) -> Result<Vec<HandoffView>, PortError> {
        Ok(Vec::new())
    }
    async fn pending_for(
        &self,
        _tx: &mut ModuleTx,
        _w: &WorkerId,
        _limit: u32,
    ) -> Result<Vec<HandoffView>, PortError> {
        Ok(Vec::new())
    }
}

/// Handoffs with fixed answers through the REAL `HandoffsApi` trait (W1-33), each list cut at
/// `limit` as the provider cuts it: sam's ask that the flood, the order and the state checks run
/// through the production `PortReads`, not only the seam's fake.
struct Lists {
    claimed: Vec<HandoffView>,
    pending: Vec<HandoffView>,
}

#[async_trait]
impl HandoffsApi for Lists {
    async fn claimed_by(
        &self,
        _tx: &mut ModuleTx,
        _w: &WorkerId,
        limit: u32,
    ) -> Result<Vec<HandoffView>, PortError> {
        Ok(self.claimed.iter().take(limit as usize).cloned().collect())
    }
    async fn pending_for(
        &self,
        _tx: &mut ModuleTx,
        _w: &WorkerId,
        limit: u32,
    ) -> Result<Vec<HandoffView>, PortError> {
        Ok(self.pending.iter().take(limit as usize).cloned().collect())
    }
}

struct Fixture {
    a: ProjectId,
    b: ProjectId,
    lead: WorkerId,
    w1: WorkerId,
    b_worker: WorkerId,
}

/// Both roles, platform and the fixture's migrations, boot's (empty) set, two projects seeded as
/// the superuser, and a `cortex_app` pool that knows `boot` and `plug_identity`.
async fn setup(tag: &str) -> (cortex_testkit::TestDb, Fixture, DbPool) {
    let db = cortex_testkit::empty_db(tag).await;
    let mut admin = db.admin().await;
    for sql in [plug_identity::BOOTSTRAP_SQL, mod_boot::BOOTSTRAP_SQL] {
        sqlx::raw_sql(sql)
            .execute(&mut admin)
            .await
            .expect("bootstrap");
    }
    let mut owner = OwnerConn::connect(&db.owner_config()).await.expect("owner");
    apply(&mut owner, &platform_migrations())
        .await
        .expect("platform");
    apply(&mut owner, &PlugIdentity::new().migrations())
        .await
        .expect("plug_identity migrations");
    apply(&mut owner, &Boot::new().migrations())
        .await
        .expect("boot's empty set applies");
    let c = SystemClock;
    let f = Fixture {
        a: ProjectId::new(&c),
        b: ProjectId::new(&c),
        lead: WorkerId::new(&c),
        w1: WorkerId::new(&c),
        b_worker: WorkerId::new(&c),
    };
    sqlx::query(
        "INSERT INTO plug_identity.projects (id, name, lead) VALUES \
         ($1::uuid, 'A', $2::uuid), ($3::uuid, $4, $5::uuid)",
    )
    .bind(f.a.to_string())
    .bind(f.lead.to_string())
    .bind(f.b.to_string())
    .bind(format!("B-{CANARY}"))
    .bind(f.b_worker.to_string())
    .execute(&mut admin)
    .await
    .expect("seed projects");
    sqlx::query(
        "INSERT INTO plug_identity.workers (id, project_id, name, kind, role, team_id, job_roles) VALUES \
         ($1::uuid, $2::uuid, 'L', 'worker', 'project_lead', NULL, '{}'), \
         ($3::uuid, $2::uuid, 'W1', 'worker', 'member', NULL, '{qa}'), \
         ($4::uuid, $5::uuid, $6, 'worker', 'project_lead', NULL, '{canary}')",
    )
    .bind(f.lead.to_string())
    .bind(f.a.to_string())
    .bind(f.w1.to_string())
    .bind(f.b_worker.to_string())
    .bind(f.b.to_string())
    .bind(format!("W{CANARY}"))
    .execute(&mut admin)
    .await
    .expect("seed workers");
    let pool = DbPool::connect(&db.db_config(&[mod_boot::ID, plug_identity::ID], Arc::new(c)))
        .await
        .expect("pool");
    (db, f, pool)
}

/// Boot's own transaction, as `Ctx::begin` makes it: module `boot`, the token's project, the
/// worker as actor.
async fn begin(pool: &DbPool, project: ProjectId, w: WorkerId) -> ModuleTx {
    let ctx = TxContext {
        module: ModuleId::parse(mod_boot::ID).expect("a module id"),
        project: Some(project),
        actor: Actor::Worker(w),
        request_id: RequestId::new(&SystemClock),
    };
    ModuleTx::begin(pool, ctx, TxOptions::default())
        .await
        .expect("begin as cortex_m_boot")
}

fn principal(w: WorkerId, project: ProjectId) -> Principal {
    Principal {
        worker: w,
        project,
        kind: WorkerKind::Worker,
        // Stale on purpose: the briefing shows the CURRENT role from IdentityApi.
        role: Role::ProjectLead,
        token: TokenId::new(&SystemClock),
        expires_at: at(86_400),
        renew_due: false,
    }
}

fn records() -> Records {
    Records {
        sections: vec![
            (
                SectionName::Orientation,
                orientation(1, 1, "What project A is.\n", &path5()),
            ),
            (
                SectionName::RulesProject,
                rules(2, 3, "/path/to/docs/a/RULES.md"),
            ),
        ],
        fail: false,
    }
}

async fn scalar_text(tx: &mut ModuleTx, sql: &'static str) -> Option<String> {
    sqlx::query_scalar::<_, Option<String>>(sql)
        .fetch_one(tx.executor().expect("executor"))
        .await
        .expect("a scalar")
}

/// BOOT-01 and BOOT-50 on a real transaction: the identity comes from the real provider (the
/// current role, not the token's stale one); the role is `cortex_m_boot` before and after the
/// delegated calls; and after 20 boots the transaction has no transaction id, so it wrote nothing.
#[tokio::test]
async fn boot_db_01_50_port_reads_on_a_real_tx_write_nothing() {
    let (_db, f, pool) = setup("boot_db_01").await;
    let identity = Provider::new();
    let records = records();
    let mut tx = begin(&pool, f.a, f.w1).await;
    assert_eq!(
        scalar_text(&mut tx, "SELECT current_user::text")
            .await
            .as_deref(),
        Some("cortex_m_boot")
    );
    for _ in 0..20 {
        let mut reads = PortReads {
            tx: &mut tx,
            identity: &identity,
            records: &records,
            handoffs: &NoHandoffs,
        };
        let i = gather(&mut reads, &principal(f.w1, f.a))
            .await
            .expect("gathers");
        assert_eq!(
            (
                i.identity.name.as_str(),
                i.identity.role,
                i.identity.job_roles.clone()
            ),
            ("W1", Role::Member(None), vec!["qa".to_owned()])
        );
        let b = assemble(&i, 12_288).expect("fits");
        assert!(b.markdown.contains(&format!("worker: W1 ({})\n", f.w1)));
        assert!(!b.markdown.contains(CANARY));
    }
    assert_eq!(
        scalar_text(&mut tx, "SELECT current_user::text")
            .await
            .as_deref(),
        Some("cortex_m_boot"),
        "the role is back after every delegated call"
    );
    assert_eq!(
        scalar_text(&mut tx, "SELECT pg_current_xact_id_if_assigned()::text").await,
        None,
        "BOOT-50: no transaction id was assigned, so boot's transaction wrote nothing"
    );
    tx.commit().await.expect("a read-only commit");
}

/// BOOT-04 under row-level security: a principal naming project B's worker, in project A's
/// transaction, finds no such worker, and boot shows nothing (404), never B's identity.
#[tokio::test]
async fn boot_db_04_another_projects_worker_is_invisible() {
    let (_db, f, pool) = setup("boot_db_04").await;
    let identity = Provider::new();
    let records = records();
    let mut tx = begin(&pool, f.a, f.b_worker).await;
    let mut reads = PortReads {
        tx: &mut tx,
        identity: &identity,
        records: &records,
        handoffs: &NoHandoffs,
    };
    let e = gather(&mut reads, &principal(f.b_worker, f.a))
        .await
        .expect_err("B's worker is not visible from A");
    assert_eq!(e, PortError::NotFound);
    assert_eq!(Problem::from(e).status, 404);
    tx.rollback().await.expect("rollback");
    // Control: the same worker in its own project is visible.
    let mut tx = begin(&pool, f.b, f.b_worker).await;
    let mut reads = PortReads {
        tx: &mut tx,
        identity: &identity,
        records: &records,
        handoffs: &NoHandoffs,
    };
    let i = gather(&mut reads, &principal(f.b_worker, f.b))
        .await
        .expect("B in B");
    assert_eq!(i.identity.name, format!("W{CANARY}"));
    tx.rollback().await.expect("rollback");
}

/// BOOT-52's `Db` half: a port's SQL error comes back as `PortError::Db`, the transaction is
/// poisoned (its commit is refused), and there is no briefing to render.
#[tokio::test]
async fn boot_db_52_a_db_error_poisons_and_returns_nothing() {
    let (_db, f, pool) = setup("boot_db_52").await;
    let identity = Provider::new();
    let records = Records {
        fail: true,
        ..records()
    };
    let mut tx = begin(&pool, f.a, f.w1).await;
    let mut reads = PortReads {
        tx: &mut tx,
        identity: &identity,
        records: &records,
        handoffs: &NoHandoffs,
    };
    let e = gather(&mut reads, &principal(f.w1, f.a))
        .await
        .expect_err("no partial briefing");
    assert!(matches!(e, PortError::Db(_)), "{e:?}");
    let p = Problem::from(e);
    assert!(p.status >= 500, "{p:?}");
    assert_eq!(
        tx.commit().await,
        Err(DbError::DelegationFailed),
        "the poisoned transaction does not commit"
    );
}

/// BOOT-30 and BOOT12-01 through the production `PortReads` (sam's re-review of b2f752a): the
/// newest CLAIM is the floor even when an older claim has a higher priority, 1,001 pending
/// handoffs at priority 100 never hide it, and a row in a state the lists never return (or in the
/// wrong list) is 500 with no briefing (BOOTRW1-01).
#[tokio::test]
async fn boot_db_30_claimed_order_flood_and_states_through_port_reads() {
    let (_db, f, pool) = setup("boot_db_30").await;
    let identity = Provider::new();
    let records = records();
    let to = || HandoffTo::Worker(f.w1);
    // claimed_by's order: 0x7001 is the newest claim (priority 10); 0x7002 is older (priority 100).
    let handoffs = Lists {
        claimed: vec![
            view(0x7001, to(), "claimed", 10),
            view(0x7002, to(), "claimed", 100),
        ],
        pending: (0..1_001u64)
            .map(|n| view(0x8000 + n, HandoffTo::JobRole("qa".into()), "pending", 100))
            .collect(),
    };
    let mut tx = begin(&pool, f.a, f.w1).await;
    let mut reads = PortReads {
        tx: &mut tx,
        identity: &identity,
        records: &records,
        handoffs: &handoffs,
    };
    let i = gather(&mut reads, &principal(f.w1, f.a))
        .await
        .expect("gathers");
    assert!(i.more_pending && !i.more_claimed);
    for budget in [4_096, 12_288] {
        let b = assemble(&i, budget).expect("fits");
        assert_eq!(
            b.boot.claimed[0].id,
            uuid(T_HANDOFF, 0x7001),
            "budget {budget}"
        );
        assert!(
            section_text(&b.markdown, 4).contains(&format!(
                "- {} claimed task priority 10 ",
                uuid(T_HANDOFF, 0x7001)
            )),
            "budget {budget}: the newest claim is shown"
        );
    }
    tx.rollback().await.expect("rollback");

    for (claimed, pending) in [
        (vec![view(0x7101, to(), "returned", 50)], Vec::new()),
        (vec![view(0x7102, to(), "pending", 50)], Vec::new()),
        (Vec::new(), vec![view(0x7103, to(), "claimed", 50)]),
        (Vec::new(), vec![view(0x7104, to(), "cancelled", 50)]),
    ] {
        let handoffs = Lists { claimed, pending };
        let mut tx = begin(&pool, f.a, f.w1).await;
        let mut reads = PortReads {
            tx: &mut tx,
            identity: &identity,
            records: &records,
            handoffs: &handoffs,
        };
        let e = gather(&mut reads, &principal(f.w1, f.a))
            .await
            .expect_err("a provider defect gives no briefing");
        let p = Problem::from(e);
        assert_eq!((p.status, p.code), (500, "internal_error"));
        tx.rollback().await.expect("rollback");
    }
}
