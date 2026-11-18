//! TOK-22/TOK-23 on a database, with the port (wave1/ports3, on integration since ca4dc0d): a new token's life is
//! the PROJECT's `token_lifetime_days` (S1 A3: 7 to 365, default 90), not a constant. `expires_at = issued +
//! L · (1 − 0.1 U)` for a stagger `U` in `[0, 1)`: between 90% and 100% of `L` days. Needs the throwaway Postgres
//! (`CV2_DB=1`). vera listed the constant 90 as an open acceptance limit of a3c5da3.
mod common;

use async_trait::async_trait;
use common::chain::{World, start};
use common::db::{DAY, NOW, SqlIdentity};
use cortex_db::{HostContext, ModuleTx};
use cortex_module::{IdentityApi, PortError, ProjectView, WorkerView};
use cortex_types::datetime::parse;
use cortex_types::{ProjectId, RequestId, SystemClock, WorkerId};
use mod_tokens::host::{HostIssueError, issue_as_host};
use mod_tokens::{BootstrapRun, OsEntropy};

async fn set_lifetime(w: &World, project: ProjectId, days: i32) {
    let mut admin = w.db.admin().await;
    sqlx::query("UPDATE identity.projects SET token_lifetime_days = $1 WHERE id = $2::uuid")
        .bind(days)
        .bind(project.to_string())
        .execute(&mut admin)
        .await
        .expect("set the lifetime");
}

/// The life of the token in the 201 body, in microseconds from the (fixed) clock.
fn life_of(a: &common::chain::Answer) -> i64 {
    assert_eq!(a.status.as_u16(), 201, "{}", a.text);
    let expires = a.json["expires_at"].as_str().expect("expires_at");
    parse(expires).expect("RFC 3339").unix_micros() - NOW
}

/// `days` days with the stagger taking off at most a tenth (and the text is rounded down to the second).
fn within_lifetime(life: i64, days: i64) -> bool {
    life >= days * DAY * 9 / 10 - 1_000_000 && life <= days * DAY
}

#[tokio::test]
async fn tok_23c_a_projects_lifetime_setting_sizes_its_new_tokens_and_no_other_projects() {
    let w = start("tl_life").await;
    let s = w.seed;
    // Project A: 7 days. Project B: 365 days. Each lead issues for a worker of its own project.
    set_lifetime(&w, s.a, 7).await;
    set_lifetime(&w, s.b, 365).await;
    let a7 = w
        .post(
            w.bearer(s.lead),
            &format!("/v2/workers/{}/tokens", s.w1),
            serde_json::json!({}),
        )
        .await;
    let b365 = w
        .post(
            w.bearer(s.lb),
            &format!("/v2/workers/{}/tokens", s.wb1),
            serde_json::json!({}),
        )
        .await;
    assert!(within_lifetime(life_of(&a7), 7), "A: {}", life_of(&a7));
    assert!(
        within_lifetime(life_of(&b365), 365),
        "B: {}",
        life_of(&b365)
    );
    // The default is 90; changing A back moves only A.
    set_lifetime(&w, s.a, 90).await;
    let a90 = w
        .post(
            w.bearer(s.lead),
            &format!("/v2/workers/{}/tokens", s.w2),
            serde_json::json!({}),
        )
        .await;
    assert!(within_lifetime(life_of(&a90), 90), "A: {}", life_of(&a90));
    // The lifetime of a 7-day project is never the old constant's: 81 to 90 days.
    set_lifetime(&w, s.a, 7).await;
    let again = w
        .post(
            w.bearer(s.lead),
            &format!("/v2/workers/{}/tokens", s.w3),
            serde_json::json!({}),
        )
        .await;
    assert!(
        life_of(&again) <= 7 * DAY,
        "a 7-day project's token lives at most 7 days"
    );
}

/// The real identity fixture, except that the project's lifetime is whatever the cell says (the database's CHECK
/// keeps a real row in 7..365, so an out-of-range value can only come through the port).
struct FixedLifetime(i32);

#[async_trait]
impl IdentityApi for FixedLifetime {
    async fn project(&self, tx: &mut ModuleTx) -> Result<ProjectView, PortError> {
        let mut p = SqlIdentity.project(tx).await?;
        p.token_lifetime_days = self.0;
        Ok(p)
    }
    async fn worker(
        &self,
        tx: &mut ModuleTx,
        w: &WorkerId,
    ) -> Result<Option<WorkerView>, PortError> {
        SqlIdentity.worker(tx, w).await
    }
    async fn roster(&self, tx: &mut ModuleTx) -> Result<Vec<WorkerView>, PortError> {
        SqlIdentity.roster(tx).await
    }
}

/// A lifetime outside 7..365 from the port is refused (defence in depth): nothing is written.
#[tokio::test]
async fn tok_23d_a_lifetime_outside_7_to_365_from_the_port_writes_nothing() {
    let w = start("tl_life_range").await;
    let s = w.seed;
    let tokens = w.count("SELECT count(*) FROM tokens.tokens").await;
    let audits = w.count("SELECT count(*) FROM plug_audit.events").await;
    for bad in [i32::MIN, -1, 0, 6, 366, 1000, i32::MAX] {
        let r = issue_as_host(
            &w.pool,
            &FixedLifetime(bad),
            &*w.audit_sink,
            &*w.clock,
            &OsEntropy,
            HostContext {
                uid: 1000,
                project: Some(s.a),
                request_id: RequestId::new(&SystemClock),
            },
            s.w1,
            BootstrapRun::First,
        )
        .await;
        assert!(matches!(r, Err(HostIssueError::Unavailable(_))), "{bad}");
    }
    assert_eq!(w.count("SELECT count(*) FROM tokens.tokens").await, tokens);
    assert_eq!(
        w.count("SELECT count(*) FROM plug_audit.events").await,
        audits
    );
    // The edges themselves are served.
    for ok in [7, 365] {
        let r = issue_as_host(
            &w.pool,
            &FixedLifetime(ok),
            &*w.audit_sink,
            &*w.clock,
            &OsEntropy,
            HostContext {
                uid: 1000,
                project: Some(s.a),
                request_id: RequestId::new(&SystemClock),
            },
            s.w1,
            BootstrapRun::First,
        )
        .await;
        assert!(r.is_ok(), "{ok}");
    }
}
