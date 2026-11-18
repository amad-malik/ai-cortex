//! The `RecordsApi` provider on a real database, called as another module would (boot, search): from a transaction of the CALLER's
//! module, delegated to this piece's role. `get`: any state, each with its `state`; an id of another project or an unknown id is absent.
//! `changed_since`: records CREATED after a cursor, keyset on `(created_at, id)`; lineage changes are not reported.
//! `section_current`: boot's read. Needs `supervise.sh`'s throwaway Postgres.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cortex_db::migrate::{apply, platform_migrations};
use cortex_db::{DbPool, ModuleTx, OwnerConn, TxOptions};
use cortex_module::{
    AuditDetails, AuditSink, AuditTarget, Piece, PortError, RecordCursor, RecordState, RecordsApi,
    Role, SectionName, WorkerKind,
};
use cortex_testkit::{TestDb, ctx, empty_db};
use cortex_types::{ProjectId, RecordId, SystemClock, WorkerId};
use mod_records::provider::Provider;
use mod_records::store::{
    Actor, Env, authorize_invalidate, authorize_log, authorize_supersede, execute_invalidate,
    execute_log, execute_supersede,
};
use mod_records::{BOOTSTRAP_SQL, ID, ModRecords};
use serde_json::{Value, json};

/// The caller's module (the testkit's `cortex_m_test_a`): the provider runs delegated from it, as boot's call would.
const CALLER: &str = "test_a";

#[derive(Default)]
struct Quiet(Mutex<u32>);

#[async_trait]
impl AuditSink for Quiet {
    async fn record(
        &self,
        _tx: &mut ModuleTx,
        _a: &'static str,
        _t: AuditTarget,
        _d: AuditDetails,
    ) -> Result<(), PortError> {
        *self.0.lock().expect("lock") += 1;
        Ok(())
    }
}

struct World {
    _db: TestDb,
    pool: Arc<DbPool>,
    audit: Arc<Quiet>,
    a: ProjectId,
    b: ProjectId,
    lead: Actor,
    member: Actor,
}

fn actor(role: Role) -> Actor {
    Actor {
        worker: WorkerId::new(&SystemClock),
        kind: WorkerKind::Worker,
        role,
    }
}

async fn setup(tag: &str) -> World {
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
    apply(&mut owner, &ModRecords::new().migrations())
        .await
        .expect("records migration");
    let pool = Arc::new(
        DbPool::connect(&db.db_config(&[CALLER, ID], Arc::new(SystemClock)))
            .await
            .expect("pool"),
    );
    World {
        _db: db,
        pool,
        audit: Arc::new(Quiet::default()),
        a: ProjectId::new(&SystemClock),
        b: ProjectId::new(&SystemClock),
        lead: actor(Role::ProjectLead),
        member: actor(Role::Member(None)),
    }
}

/// A transaction of the CALLER's module (not this piece's), as another piece would have.
async fn caller_tx(w: &World, project: ProjectId) -> ModuleTx {
    ModuleTx::begin(&w.pool, ctx(CALLER, Some(project)), TxOptions::default())
        .await
        .expect("begin")
}

/// Writes through the piece's own store, in a transaction of the piece's module.
async fn write(w: &World, who: &Actor, project: ProjectId, body: Value) -> RecordId {
    let req = serde_json::from_value(body).expect("decodes");
    let mut tx = ModuleTx::begin(&w.pool, ctx(ID, Some(project)), TxOptions::default())
        .await
        .expect("begin");
    let v = authorize_log(&mut tx, who, &req).await.expect("authorized");
    let env = Env {
        project,
        clock: &SystemClock,
        audit: &*w.audit,
    };
    let c = execute_log(&mut tx, &env, who, &req, &v)
        .await
        .expect("written");
    tx.commit().await.expect("commit");
    c.id
}

async fn lineage(
    w: &World,
    project: ProjectId,
    target: RecordId,
    by: Option<RecordId>,
    reason: &str,
    undo: bool,
) {
    let mut tx = ModuleTx::begin(&w.pool, ctx(ID, Some(project)), TxOptions::default())
        .await
        .expect("begin");
    let env = Env {
        project,
        clock: &SystemClock,
        audit: &*w.audit,
    };
    match by {
        Some(by) => {
            let (t, r) = authorize_supersede(&mut tx, target, by, Some(reason))
                .await
                .expect("authorized");
            execute_supersede(&mut tx, &env, &w.member, &t, by, &r)
                .await
                .expect("superseded");
        }
        None => {
            let (t, r) = authorize_invalidate(&mut tx, target, Some(reason))
                .await
                .expect("authorized");
            execute_invalidate(&mut tx, &env, &w.member, &t, &r, undo)
                .await
                .expect("invalidated");
        }
    }
    tx.commit().await.expect("commit");
}

#[tokio::test]
async fn pv01_get_returns_records_in_any_state_with_their_state_and_nothing_else() {
    let w = setup("pv01").await;
    let live = write(&w, &w.member, w.a, json!({"type":"knowledge","summary":"live","body":"b","fields":{"k":[1,2]},"source":{"kind":"file","ref":"/x"}})).await;
    let old = write(&w, &w.member, w.a, json!({"type":"note","summary":"old"})).await;
    let new = write(&w, &w.member, w.a, json!({"type":"note","summary":"new"})).await;
    let bad = write(&w, &w.member, w.a, json!({"type":"note","summary":"bad"})).await;
    let foreign = write(&w, &w.member, w.b, json!({"type":"note","summary":"in B"})).await;
    lineage(&w, w.a, old, Some(new), "replaced", false).await;
    lineage(&w, w.a, bad, None, "wrong", false).await;
    let unknown = RecordId::new(&SystemClock);

    let provider = Provider::new();
    let mut tx = caller_tx(&w, w.a).await;
    let got = provider
        .get(&mut tx, &[live, old, new, bad, foreign, unknown])
        .await
        .expect("get");
    let by_id = |id: RecordId| got.iter().find(|r| r.id == id);
    assert_eq!(
        got.len(),
        4,
        "the four records of the project; the foreign and the unknown id are simply absent"
    );
    assert!(by_id(foreign).is_none() && by_id(unknown).is_none());
    let l = by_id(live).expect("live");
    assert_eq!(
        (l.kind.as_str(), l.summary.as_str(), l.body.as_deref()),
        ("knowledge", "live", Some("b"))
    );
    assert_eq!(l.fields, json!({"k":[1,2]}));
    assert_eq!(l.state, RecordState::Active);
    let src = l.source.as_ref().expect("source");
    assert_eq!(
        (
            src.kind.as_str(),
            src.reference.as_str(),
            src.span.as_deref()
        ),
        ("file", "/x", None)
    );
    assert_eq!(
        (l.author_kind.as_str(), l.provenance.as_str()),
        ("worker", "worker_written")
    );
    assert_eq!(l.author, w.member.worker.to_string());
    assert_eq!(
        (l.section.as_deref(), l.section_revision, l.superseded_by),
        (None, None, None)
    );
    let o = by_id(old).expect("old");
    assert_eq!(
        (o.state, o.superseded_by),
        (RecordState::Superseded, Some(new))
    );
    assert_eq!(by_id(bad).expect("bad").state, RecordState::Invalidated);
    assert_eq!(by_id(new).expect("new").state, RecordState::Active);
    // The timestamp is the id's own (UUIDv7).
    assert_eq!(l.created_at, live.timestamp());
    // An empty list is an empty answer.
    assert!(provider.get(&mut tx, &[]).await.expect("empty").is_empty());
}

#[tokio::test]
async fn pv02_changed_since_pages_by_created_at_and_id_and_reports_no_lineage_change() {
    let w = setup("pv02").await;
    let mut ids = vec![];
    for n in 0..5 {
        ids.push(
            write(
                &w,
                &w.member,
                w.a,
                json!({"type":"note","summary":format!("n{n}")}),
            )
            .await,
        );
    }
    write(&w, &w.member, w.b, json!({"type":"note","summary":"in B"})).await;
    let provider = Provider::new();
    let mut tx = caller_tx(&w, w.a).await;

    let p1 = provider
        .changed_since(&mut tx, None, 2)
        .await
        .expect("page 1");
    assert_eq!(
        p1.records.iter().map(|r| r.id).collect::<Vec<_>>(),
        ids[..2],
        "oldest first"
    );
    let c1 = p1.next.expect("more rows: a cursor");
    assert_eq!(
        (c1.id, c1.at),
        (ids[1], p1.records[1].created_at),
        "the cursor is the last record's position"
    );
    let p2 = provider
        .changed_since(&mut tx, Some(c1), 2)
        .await
        .expect("page 2");
    assert_eq!(
        p2.records.iter().map(|r| r.id).collect::<Vec<_>>(),
        ids[2..4]
    );
    let p3 = provider
        .changed_since(&mut tx, p2.next, 2)
        .await
        .expect("page 3");
    assert_eq!(
        p3.records.iter().map(|r| r.id).collect::<Vec<_>>(),
        ids[4..]
    );
    assert!(p3.next.is_none(), "the last page has no cursor");
    // A page that ends exactly at the end has no cursor either.
    let all = provider.changed_since(&mut tx, None, 5).await.expect("all");
    assert_eq!((all.records.len(), all.next), (5, None));
    // Another project's rows never appear.
    let mut txb = caller_tx(&w, w.b).await;
    assert_eq!(
        provider
            .changed_since(&mut txb, None, 50)
            .await
            .expect("B")
            .records
            .len(),
        1
    );
    // Created after, not changed: a supersede or an invalidation of an old record does not bring it back.
    drop(tx);
    lineage(&w, w.a, ids[0], None, "wrong", false).await;
    let mut tx = caller_tx(&w, w.a).await;
    let after = provider
        .changed_since(
            &mut tx,
            Some(RecordCursor {
                at: all.records[4].created_at,
                id: ids[4],
            }),
            50,
        )
        .await
        .expect("after");
    assert!(
        after.records.is_empty() && after.next.is_none(),
        "no lineage feed: only creation"
    );
    // A limit of 0 is refused; a limit above 200 is cut to 200.
    assert!(matches!(
        provider.changed_since(&mut tx, None, 0).await,
        Err(PortError::Invalid(_))
    ));
    assert_eq!(
        provider
            .changed_since(&mut tx, None, 100_000)
            .await
            .expect("big")
            .records
            .len(),
        5
    );
}

#[tokio::test]
async fn pv03_section_current_is_boots_read_with_no_fallback() {
    let w = setup("pv03").await;
    let provider = Provider::new();
    let name = SectionName::parse("orientation").expect("name");
    let mut tx = caller_tx(&w, w.a).await;
    assert!(
        provider
            .section_current(&mut tx, &name)
            .await
            .expect("none")
            .is_none(),
        "never written"
    );
    drop(tx);

    let orient = |n: u32| {
        json!({"type":"knowledge","summary":format!("o{n}"),"section":"orientation","body":format!("body {n}"),
        "fields":{"reading_path":["/a","/b","/c","/d","/e"],"source_path":"/home/x/orientation.md"}})
    };
    let r1 = write(&w, &w.lead, w.a, orient(1)).await;
    let r2 = write(&w, &w.lead, w.a, orient(2)).await;
    let mut tx = caller_tx(&w, w.a).await;
    let s = provider
        .section_current(&mut tx, &name)
        .await
        .expect("current")
        .expect("found");
    assert_eq!(
        (s.record_id, s.revision, s.body.as_str()),
        (r2, 2, "body 2")
    );
    assert_eq!(
        s.fields["reading_path"],
        json!(["/a", "/b", "/c", "/d", "/e"]),
        "reading_path exactly as written"
    );
    assert_eq!(s.fields["source_path"], json!("/home/x/orientation.md"));
    assert_eq!(s.author, w.lead.worker);
    assert_eq!(s.created_at, r2.timestamp());
    let mut hex = String::new();
    for b in s.body_sha256.0 {
        hex.push_str(&format!("{b:02x}"));
    }
    assert_eq!(hex.len(), 64);
    // Another project sees nothing.
    let mut txb = caller_tx(&w, w.b).await;
    assert!(
        provider
            .section_current(&mut txb, &name)
            .await
            .expect("none")
            .is_none()
    );
    drop(tx);
    // Invalidate the current revision: no fallback to revision 1.
    lineage(&w, w.a, r2, None, "withdrawn", false).await;
    let mut tx = caller_tx(&w, w.a).await;
    assert!(
        provider
            .section_current(&mut tx, &name)
            .await
            .expect("none")
            .is_none(),
        "no fallback to revision 1 ({r1})"
    );
}

/// hale F-1: a record with a far-future or offset `valid_until` is stored as the instant it names and the feed steps over it.
#[tokio::test]
async fn pv04_valid_until_round_trips_through_get_and_changed_since() {
    let w = setup("pv04").await;
    let far = write(
        &w,
        &w.member,
        w.a,
        json!({"type":"note","summary":"far","valid_until":"9999-12-31T23:59:59Z"}),
    )
    .await;
    let off = write(
        &w,
        &w.member,
        w.a,
        json!({"type":"note","summary":"offset","valid_until":"2030-01-01T12:34:56+05:30"}),
    )
    .await;
    let frac = write(
        &w,
        &w.member,
        w.a,
        json!({"type":"note","summary":"fraction","valid_until":"2030-01-01T00:00:00.123456Z"}),
    )
    .await;
    let none = write(&w, &w.member, w.a, json!({"type":"note","summary":"none"})).await;
    let provider = Provider::new();
    let mut tx = caller_tx(&w, w.a).await;
    let got = provider
        .get(&mut tx, &[far, off, frac, none])
        .await
        .expect("get");
    let micros = |id: RecordId| {
        got.iter()
            .find(|r| r.id == id)
            .expect("found")
            .valid_until
            .map(|t| t.unix_micros())
    };
    assert_eq!(micros(far), Some(253_402_300_799_000_000));
    assert_eq!(micros(off), Some(1_893_481_496_000_000));
    assert_eq!(micros(frac), Some(1_893_456_000_123_456));
    assert_eq!(micros(none), None);
    // The feed steps over all four, in two pages.
    let p1 = provider
        .changed_since(&mut tx, None, 2)
        .await
        .expect("page 1");
    let p2 = provider
        .changed_since(&mut tx, p1.next, 2)
        .await
        .expect("page 2");
    assert_eq!((p1.records.len(), p2.records.len(), p2.next), (2, 2, None));
}

/// hale F-1 (second line): even a stored `infinity` (written around the API, as the superuser) cannot break the provider.
#[tokio::test]
async fn pv05_an_infinite_valid_until_never_breaks_the_provider() {
    let w = setup("pv05").await;
    let before = write(
        &w,
        &w.member,
        w.a,
        json!({"type":"note","summary":"before"}),
    )
    .await;
    let inf = RecordId::new(&SystemClock);
    let mut admin = w._db.admin().await;
    sqlx::query(
        "INSERT INTO records.records (id, project_id, created_at, type, provenance, author_kind, author_id, written_by, summary, \
         summary_sha256, body_sha256, fields_sha256, valid_until) \
         VALUES ($1::uuid, $2::uuid, uuid_extract_timestamp($1::uuid), 'note', 'worker_written', 'worker', 'w1', $1::uuid, 'inf', \
         sha256('s'::bytea), sha256('b'::bytea), sha256('f'::bytea), 'infinity')",
    )
    .bind(inf.to_string())
    .bind(w.a.to_string())
    .execute(&mut admin)
    .await
    .expect("seed an infinite valid_until");
    let after = write(&w, &w.member, w.a, json!({"type":"note","summary":"after"})).await;
    let provider = Provider::new();
    let mut tx = caller_tx(&w, w.a).await;
    let got = provider
        .get(&mut tx, &[inf])
        .await
        .expect("get over an infinite valid_until");
    assert_eq!(got.len(), 1);
    assert_eq!(
        got[0].valid_until, None,
        "an infinite instant is not a Timestamp: shown as none"
    );
    let page = provider
        .changed_since(&mut tx, None, 10)
        .await
        .expect("the feed is not stuck");
    assert_eq!(
        page.records.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![before, inf, after]
    );
}

/// cox W1-36: every other date-time input is checked the same way. The cursor is a microsecond count; one outside the supported range
/// (year 1 to year 9999) is refused, not a database error.
#[tokio::test]
async fn pv06_a_cursor_outside_the_supported_time_range_is_refused() {
    let w = setup("pv06").await;
    let one = write(&w, &w.member, w.a, json!({"type":"note","summary":"one"})).await;
    let provider = Provider::new();
    let mut tx = caller_tx(&w, w.a).await;
    for micros in [
        i64::MAX,
        i64::MIN,
        253_402_300_800_000_000,
        -62_135_596_801_000_000,
        -1, // 1969-12-31T23:59:59.999999Z: the supported range starts at 1970 (cox W1-39)
        -86_400_000_000,
    ] {
        let cursor = RecordCursor {
            at: cortex_types::Timestamp::from_unix_micros(micros),
            id: one,
        };
        assert!(
            matches!(
                provider.changed_since(&mut tx, Some(cursor), 10).await,
                Err(PortError::Invalid(_))
            ),
            "cursor at {micros}"
        );
    }
    // The extremes of the range are fine and simply return nothing / everything.
    let first = RecordCursor {
        at: cortex_types::Timestamp::from_unix_micros(0),
        id: one,
    };
    assert_eq!(
        provider
            .changed_since(&mut tx, Some(first), 10)
            .await
            .expect("year 1970")
            .records
            .len(),
        1
    );
    let last = RecordCursor {
        at: cortex_types::Timestamp::from_unix_micros(253_402_300_799_999_999),
        id: one,
    };
    assert_eq!(
        provider
            .changed_since(&mut tx, Some(last), 10)
            .await
            .expect("year 9999")
            .records
            .len(),
        0
    );
}
