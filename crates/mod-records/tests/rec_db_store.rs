//! RED for piece 6, the write path and the reads, on a real database (tests/mod-records.md REC-01, 09 to 11, 13 to 15, 26 to 28):
//! `authorize_log` (404 then 403), `execute_log` (422 then the write), `show`, `section_current`, with a recording audit fake.
//! Needs `supervise.sh`'s throwaway Postgres (CV2_PG_PORT, CV2_PG_PASSWORD).

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cortex_db::migrate::{apply, platform_migrations};
use cortex_db::{DbPool, ModuleTx, OwnerConn, TxOptions};
use cortex_module::{
    AuditDetails, AuditSink, AuditTarget, Piece, PortError, Problem, Role, WorkerKind,
};
use cortex_testkit::{TestDb, ctx, empty_db};
use cortex_types::{ProjectId, RecordId, SystemClock, TeamId, WorkerId};
use mod_records::store::{
    Actor, Created, Env, LineageState, authorize_invalidate, authorize_log, authorize_supersede,
    execute_invalidate, execute_log, execute_supersede, lineage_target, record_exists,
    section_current, show,
};
use mod_records::{BOOTSTRAP_SQL, ID, ModRecords};
use serde_json::{Value, json};

/// Records every audit call: (action, target as text, whether the details were empty).
#[derive(Default)]
struct Recorder(Mutex<Vec<(String, String, bool)>>);

#[async_trait]
impl AuditSink for Recorder {
    async fn record(
        &self,
        _tx: &mut ModuleTx,
        action: &'static str,
        target: AuditTarget,
        details: AuditDetails,
    ) -> Result<(), PortError> {
        let t = match target {
            AuditTarget::Record(r) => r.to_string(),
            other => format!("{other:?}"),
        };
        let empty = format!("{details:?}") == format!("{:?}", AuditDetails::new());
        self.0
            .lock()
            .expect("lock")
            .push((action.to_owned(), t, empty));
        Ok(())
    }
}

struct World {
    db: TestDb,
    pool: Arc<DbPool>,
    audit: Arc<Recorder>,
    a: ProjectId,
    b: ProjectId,
    lead: Actor,
    member: Actor,
    other: Actor,
    team_lead: Actor,
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
        DbPool::connect(&db.db_config(&[ID], Arc::new(SystemClock)))
            .await
            .expect("pool"),
    );
    World {
        db,
        pool,
        audit: Arc::new(Recorder::default()),
        a: ProjectId::new(&SystemClock),
        b: ProjectId::new(&SystemClock),
        lead: actor(Role::ProjectLead),
        member: actor(Role::Member(None)),
        other: actor(Role::Member(None)),
        team_lead: actor(Role::TeamLead(TeamId::new(&SystemClock))),
    }
}

async fn begin(w: &World, project: ProjectId) -> ModuleTx {
    ModuleTx::begin(&w.pool, ctx(ID, Some(project)), TxOptions::default())
        .await
        .expect("begin")
}

/// The route's two closures in order: 404 and 403 (authorize), then 422 and the write (execute). One transaction.
async fn write(
    w: &World,
    who: &Actor,
    project: ProjectId,
    body: Value,
) -> Result<Created, Problem> {
    write_with(&w.pool, &w.audit, who, project, body).await
}

async fn write_with(
    pool: &Arc<DbPool>,
    audit: &Arc<Recorder>,
    who: &Actor,
    project: ProjectId,
    body: Value,
) -> Result<Created, Problem> {
    let req = serde_json::from_value(body).expect("the body decodes");
    let mut tx = ModuleTx::begin(pool, ctx(ID, Some(project)), TxOptions::default())
        .await
        .expect("begin");
    let v = authorize_log(&mut tx, who, &req).await?;
    let env = Env {
        project,
        clock: &SystemClock,
        audit: &**audit,
    };
    let created = execute_log(&mut tx, &env, who, &req, &v).await?;
    tx.commit().await.expect("commit");
    Ok(created)
}

fn code(r: Result<Created, Problem>) -> (&'static str, u16) {
    match r {
        Err(p) => (p.code, p.status),
        Ok(c) => panic!("expected a refusal, got {c:?}"),
    }
}

async fn scalar_i64(w: &World, project: ProjectId, sql: &'static str, bind: Option<String>) -> i64 {
    let mut tx = begin(w, project).await;
    let mut q = sqlx::query_scalar(sql);
    if let Some(b) = bind {
        q = q.bind(b);
    }
    q.fetch_one(tx.executor().expect("executor"))
        .await
        .expect("scalar")
}

const ORIENT_PATHS: [&str; 5] = [
    "/a/one.md",
    "/a/two.md",
    "/b/three.md",
    "/b/four.md",
    "/c/five.md",
];

fn orientation() -> Value {
    json!({"type":"knowledge","summary":"orientation","section":"orientation","body":"read these first",
           "fields":{"reading_path":ORIENT_PATHS,"source_path":"/home/x/orientation.md"}})
}

// ---- REC-01: every writable type, hashes, show ----

#[tokio::test]
async fn st01_every_writable_type_is_stored_and_shown_with_its_hashes() {
    let w = setup("st01").await;
    let bodies = [
        json!({"type":"decision","summary":"d","fields":{"why":"because","k":[1,2,{"z":null}]},"body":"body d"}),
        json!({"type":"lesson","summary":"l"}),
        json!({"type":"note","summary":"n","body":"line one\r\nline two  "}),
        json!({"type":"status","summary":"s"}),
        json!({"type":"question","summary":"q"}),
        json!({"type":"blocker","summary":"b"}),
        json!({"type":"knowledge","summary":"k","source":{"kind":"file","ref":"/x/y"}}),
        json!({"type":"work-product","summary":"wp","fields":{"files":["/a/file.rs","/b/other.rs"]}}),
    ];
    for body in bodies {
        let kind = body["type"].as_str().expect("type").to_owned();
        let created = write(&w, &w.member, w.a, body.clone())
            .await
            .expect("accepted");
        assert_eq!(created.section_revision, None);
        assert!(
            created.created_at.ends_with('Z') && created.created_at.len() == 27,
            "RFC 3339 micros: {}",
            created.created_at
        );
        let mut tx = begin(&w, w.a).await;
        let r = show(&mut tx, created.id)
            .await
            .expect("show")
            .expect("found");
        assert_eq!(r.kind, kind);
        assert_eq!(r.state, "active");
        assert_eq!(r.author, w.member.worker.to_string());
        assert_eq!(r.author_kind, "worker");
        assert_eq!(r.provenance, "worker_written");
        assert_eq!(r.created_at, created.created_at);
        assert_eq!(r.summary, body["summary"].as_str().expect("summary"));
        assert_eq!(r.body.as_deref(), body.get("body").and_then(Value::as_str));
        assert_eq!(
            r.fields,
            body.get("fields").cloned().unwrap_or_else(|| json!({}))
        );
        assert!(r.lineage.is_none() && r.superseded_by.is_none());
    }
    // Three hashes set by the piece: sha256 of the summary, of the body (zero bytes when none), of RFC 8785 canonical fields.
    let ok = scalar_i64(
        &w,
        w.a,
        "SELECT count(*) FROM records.records WHERE summary_sha256 = sha256(convert_to(summary, 'UTF8')) \
         AND body_sha256 = sha256(convert_to(coalesce(body, ''), 'UTF8'))",
        None,
    )
    .await;
    assert_eq!(ok, 8, "summary and body hashes");
    let fields_ok = scalar_i64(
        &w,
        w.a,
        "SELECT count(*) FROM records.records WHERE fields_sha256 = sha256(convert_to(fields::text, 'UTF8')) OR fields = '{}'::jsonb",
        None,
    )
    .await;
    assert!(
        fields_ok >= 5,
        "canonical fields hash differs from jsonb text only when the form differs"
    );
}

// ---- REC-09 / REC-10: references ----

#[tokio::test]
async fn st02_references_must_name_something_in_the_project() {
    let w = setup("st02").await;
    let r1 = write(&w, &w.member, w.a, json!({"type":"note","summary":"r1"}))
        .await
        .expect("r1")
        .id;
    let r2 = write(&w, &w.member, w.a, json!({"type":"note","summary":"r2"}))
        .await
        .expect("r2")
        .id;
    let foreign = write(&w, &w.member, w.b, json!({"type":"note","summary":"in B"}))
        .await
        .expect("B")
        .id;
    let unknown = RecordId::new(&SystemClock);
    let before = scalar_i64(&w, w.a, "SELECT count(*) FROM records.records", None).await;

    for bad in [
        json!({"type":"decision","summary":"d","fields":{"why":"w"},"affects":[unknown.to_string()]}),
        json!({"type":"decision","summary":"d","fields":{"why":"w"},"affects":[r1.to_string(), foreign.to_string()]}),
        json!({"type":"note","summary":"n","reply_to":unknown.to_string()}),
        json!({"type":"note","summary":"n","handoff_id":unknown.to_string()}),
        json!({"type":"knowledge","summary":"k","source":{"kind":"conversation","ref":unknown.to_string()}}),
    ] {
        assert_eq!(
            code(write(&w, &w.member, w.a, bad).await),
            ("not_found", 404)
        );
    }
    // Another project's id and an unknown id look the same.
    let p1 = write(
        &w,
        &w.member,
        w.a,
        json!({"type":"note","summary":"n","reply_to":unknown.to_string()}),
    )
    .await
    .expect_err("404");
    let p2 = write(
        &w,
        &w.member,
        w.a,
        json!({"type":"note","summary":"n","reply_to":foreign.to_string()}),
    )
    .await
    .expect_err("404");
    assert_eq!(
        (p1.code, p1.status, p1.detail.clone()),
        (p2.code, p2.status, p2.detail.clone())
    );
    // STEP 0 comes first (R-16): a bad summary with an unknown reference is 422, not 404 (no database read decides it).
    let both = json!({"type":"note","summary":"","reply_to":unknown.to_string()});
    assert_eq!(
        code(write(&w, &w.member, w.a, both).await),
        ("invalid_request", 422)
    );
    // ...and 422 comes before 403: a bad summary in a reserved section written by a member is 422 too.
    let both = json!({"type":"knowledge","summary":"","section":"rules-project","body":"b"});
    assert_eq!(
        code(write(&w, &w.member, w.a, both).await),
        ("invalid_request", 422)
    );
    assert_eq!(
        scalar_i64(&w, w.a, "SELECT count(*) FROM records.records", None).await,
        before,
        "nothing stored on a refusal"
    );

    // The good case: one platform.refs row per citation, and the record is a platform object.
    let d = write(
        &w,
        &w.member,
        w.a,
        json!({"type":"decision","summary":"d","fields":{"why":"w"},"affects":[r1.to_string(), r2.to_string()],"reply_to":r1.to_string()}),
    )
    .await
    .expect("decision")
    .id;
    let k = write(&w, &w.member, w.a, json!({"type":"knowledge","summary":"k","source":{"kind":"conversation","ref":r2.to_string()}}))
        .await
        .expect("knowledge")
        .id;
    let n = |kind: &'static str, from: RecordId| {
        let sql: &'static str = match kind {
            "affects" => {
                "SELECT count(*) FROM platform.refs WHERE kind = 'affects' AND from_id = $1::uuid"
            }
            "reply_to" => {
                "SELECT count(*) FROM platform.refs WHERE kind = 'reply_to' AND from_id = $1::uuid"
            }
            _ => "SELECT count(*) FROM platform.refs WHERE kind = 'cites' AND from_id = $1::uuid",
        };
        (sql, from.to_string())
    };
    for (kind, from, want) in [("affects", d, 2), ("reply_to", d, 1), ("cites", k, 1)] {
        let (sql, bind) = n(kind, from);
        assert_eq!(scalar_i64(&w, w.a, sql, Some(bind)).await, want, "{kind}");
    }
    let objects = scalar_i64(
        &w,
        w.a,
        "SELECT count(*) FROM platform.objects WHERE kind = 'record' AND id = $1::uuid",
        Some(d.to_string()),
    )
    .await;
    assert_eq!(objects, 1, "every record is a platform object");
    // A handoff id that names a real handoff object is accepted (seeded as the superuser).
    let h = RecordId::new(&SystemClock);
    let mut admin = w.db.admin().await;
    sqlx::query("INSERT INTO platform.objects (id, project_id, kind, module) VALUES ($1::uuid, $2::uuid, 'handoff', 'handoffs')")
        .bind(h.to_string())
        .bind(w.a.to_string())
        .execute(&mut admin)
        .await
        .expect("seed handoff");
    write(
        &w,
        &w.member,
        w.a,
        json!({"type":"note","summary":"n","handoff_id":h.to_string()}),
    )
    .await
    .expect("a real handoff id");
}

// ---- REC-13 / 15 / 26 / 28: reserved sections ----

#[tokio::test]
async fn st03_reserved_sections_permissions_revisions_pointer_and_audit() {
    let w = setup("st03").await;
    let rules = |n: u32| json!({"type":"knowledge","summary":format!("rules {n}"),"section":"rules-project","body":format!("rules body {n}")});

    // Only the project lead writes rules-project and orientation.
    assert_eq!(
        code(write(&w, &w.member, w.a, rules(0)).await),
        ("section_reserved", 403)
    );
    assert_eq!(
        code(write(&w, &w.team_lead, w.a, rules(0)).await),
        ("section_reserved", 403)
    );
    assert_eq!(
        code(write(&w, &w.member, w.a, orientation()).await),
        ("section_reserved", 403)
    );
    let mut ids = vec![];
    for n in 1..=3 {
        let c = write(&w, &w.lead, w.a, rules(n))
            .await
            .expect("lead writes");
        assert_eq!(c.section_revision, Some(n as i32));
        ids.push(c.id);
    }
    let mut tx = begin(&w, w.a).await;
    let ptr = section_current(&mut tx, &sec_name("rules-project"))
        .await
        .expect("pointer")
        .expect("found");
    assert_eq!((ptr.record_id, ptr.revision), (ids[2], 3));
    // A worker writes its own rules section; another member may not; the lead may.
    let sec = format!("rules-worker:{}", w.member.worker);
    let mine = json!({"type":"knowledge","summary":"mine","section":sec,"body":"my rules"});
    assert_eq!(
        code(write(&w, &w.other, w.a, mine.clone()).await),
        ("section_reserved", 403)
    );
    assert_eq!(
        write(&w, &w.member, w.a, mine.clone())
            .await
            .expect("own")
            .section_revision,
        Some(1)
    );
    assert_eq!(
        write(&w, &w.lead, w.a, mine)
            .await
            .expect("lead")
            .section_revision,
        Some(2)
    );

    // Orientation: the lead writes it; reading_path comes back exactly as written (BOOT-13's record-side witness).
    let o = write(&w, &w.lead, w.a, orientation())
        .await
        .expect("orientation");
    assert_eq!(o.section_revision, Some(1));
    let mut tx = begin(&w, w.a).await;
    let rec = show(&mut tx, o.id).await.expect("show").expect("found");
    assert_eq!(rec.fields["reading_path"], json!(ORIENT_PATHS));
    let ptr = section_current(&mut tx, &sec_name("orientation"))
        .await
        .expect("pointer")
        .expect("found");
    assert_eq!(
        (ptr.record_id, ptr.revision, ptr.source_path.as_deref()),
        (o.id, 1, Some("/home/x/orientation.md"))
    );
    assert_eq!(ptr.sha256.len(), 64);
    // Never written: no pointer.
    assert!(
        section_current(
            &mut tx,
            &sec_name("rules-worker:01930000-0000-7000-8000-0000000000bb")
        )
        .await
        .expect("none")
        .is_none()
    );

    // Audit: one row per SUCCESSFUL reserved write (3 + 2 + 1), action section.write_reserved, target the record id, details empty.
    let rows = w.audit.0.lock().expect("lock").clone();
    assert_eq!(rows.len(), 6, "{rows:?}");
    assert!(
        rows.iter()
            .all(|(a, _, empty)| a == "section.write_reserved" && *empty)
    );
    assert!(rows.iter().any(|(_, t, _)| *t == o.id.to_string()));
    // An ordinary record writes no audit row here.
    write(&w, &w.member, w.a, json!({"type":"note","summary":"plain"}))
        .await
        .expect("plain");
    assert_eq!(w.audit.0.lock().expect("lock").len(), 6);
}

fn sec_name(s: &str) -> cortex_module::SectionName {
    cortex_module::SectionName::parse(s).expect("a valid section name")
}

// ---- REC-14: concurrent section writes ----

#[tokio::test]
async fn st04_eight_simultaneous_section_writes_get_revisions_one_to_eight() {
    let w = setup("st04").await;
    let mut set = tokio::task::JoinSet::new();
    for n in 0..8 {
        let (pool, audit, lead, project) = (
            Arc::clone(&w.pool),
            Arc::clone(&w.audit),
            w.lead.clone(),
            w.a,
        );
        set.spawn(async move {
            write_with(
                &pool,
                &audit,
                &lead,
                project,
                json!({"type":"knowledge","summary":format!("c{n}"),"section":"rules-project","body":"x"}),
            )
            .await
        });
    }
    let mut revs = vec![];
    while let Some(r) = set.join_next().await {
        revs.push(
            r.expect("task")
                .expect("write")
                .section_revision
                .expect("revision"),
        );
    }
    revs.sort_unstable();
    assert_eq!(
        revs,
        (1..=8).collect::<Vec<_>>(),
        "no revision lost or repeated"
    );
    let mut tx = begin(&w, w.a).await;
    let ptr = section_current(&mut tx, &sec_name("rules-project"))
        .await
        .expect("pointer")
        .expect("found");
    assert_eq!(ptr.revision, 8, "the pointer ends at the highest revision");
    assert_eq!(w.audit.0.lock().expect("lock").len(), 8);
}

// ---- REC-11: isolation of the reads ----

#[tokio::test]
async fn st05_another_projects_record_and_section_are_invisible() {
    let w = setup("st05").await;
    let id = write(&w, &w.member, w.a, json!({"type":"note","summary":"mine"}))
        .await
        .expect("note")
        .id;
    write(&w, &w.lead, w.a, orientation())
        .await
        .expect("orientation");
    let mut tx = begin(&w, w.b).await;
    assert!(
        show(&mut tx, id).await.expect("show").is_none(),
        "B cannot read A's record"
    );
    assert!(
        section_current(&mut tx, &sec_name("orientation"))
            .await
            .expect("none")
            .is_none(),
        "nor A's section"
    );
    let mut tx = begin(&w, w.a).await;
    assert!(show(&mut tx, id).await.expect("show").is_some());
    let _ = &w.db;
}

// ---- lineage: supersede, invalidate, undo (REC-16 to REC-20, REC-30, REC-31) ----

/// A lineage call as the routes make it (the 403 target rule is the route's, not tested here): step 0 (422), then 404 for the target
/// and for `by`, then the 409 of the state and the write.
async fn sup(
    pool: &Arc<DbPool>,
    audit: &Arc<Recorder>,
    who: &Actor,
    project: ProjectId,
    target: RecordId,
    by: RecordId,
    reason: Option<&str>,
) -> Result<LineageState, Problem> {
    let mut tx = ModuleTx::begin(pool, ctx(ID, Some(project)), TxOptions::default())
        .await
        .expect("begin");
    let (t, reason) = authorize_supersede(&mut tx, target, by, reason).await?;
    let env = Env {
        project,
        clock: &SystemClock,
        audit: &**audit,
    };
    let out = execute_supersede(&mut tx, &env, who, &t, by, &reason).await?;
    tx.commit().await.expect("commit");
    Ok(out)
}

async fn inv(
    pool: &Arc<DbPool>,
    audit: &Arc<Recorder>,
    who: &Actor,
    project: ProjectId,
    target: RecordId,
    reason: Option<&str>,
    undo: bool,
) -> Result<LineageState, Problem> {
    let mut tx = ModuleTx::begin(pool, ctx(ID, Some(project)), TxOptions::default())
        .await
        .expect("begin");
    let (t, reason) = authorize_invalidate(&mut tx, target, reason).await?;
    let env = Env {
        project,
        clock: &SystemClock,
        audit: &**audit,
    };
    let out = execute_invalidate(&mut tx, &env, who, &t, &reason, undo).await?;
    tx.commit().await.expect("commit");
    Ok(out)
}

fn lcode<T: std::fmt::Debug>(r: Result<T, Problem>) -> (&'static str, u16) {
    match r {
        Err(p) => (p.code, p.status),
        Ok(v) => panic!("expected a refusal, got {v:?}"),
    }
}

async fn note(w: &World, project: ProjectId, who: &Actor, summary: &str) -> RecordId {
    write(w, who, project, json!({"type":"note","summary":summary}))
        .await
        .expect("note")
        .id
}

async fn state_of(
    w: &World,
    project: ProjectId,
    id: RecordId,
) -> (String, Option<String>, Option<String>) {
    let mut tx = begin(w, project).await;
    let r = show(&mut tx, id).await.expect("show").expect("found");
    (
        r.state,
        r.superseded_by.map(|s| s.to_string()),
        r.lineage.map(|l| l.reason),
    )
}

fn audit_actions(w: &World) -> Vec<(String, String)> {
    w.audit
        .0
        .lock()
        .expect("lock")
        .iter()
        .map(|(a, t, _)| (a.clone(), t.clone()))
        .collect()
}

#[tokio::test]
async fn ln01_the_state_machine_supersede_invalidate_undo_log_and_audit() {
    let w = setup("ln01").await;
    let (r1, r2, r3) = (
        note(&w, w.a, &w.member, "r1").await,
        note(&w, w.a, &w.member, "r2").await,
        note(&w, w.a, &w.member, "r3").await,
    );
    let (p, au) = (&w.pool, &w.audit);

    // Supersede r1 by r2.
    let out = sup(p, au, &w.member, w.a, r1, r2, Some("replaced by r2"))
        .await
        .expect("supersede");
    assert_eq!((out.id, out.state.as_str()), (r1, "superseded"));
    assert_eq!(
        state_of(&w, w.a, r1).await,
        (
            "superseded".into(),
            Some(r2.to_string()),
            Some("replaced by r2".into())
        )
    );
    // A superseded record: every further lineage call is 409, undo included.
    assert_eq!(
        lcode(sup(p, au, &w.member, w.a, r1, r3, Some("again")).await),
        ("lineage_state_conflict", 409)
    );
    assert_eq!(
        lcode(inv(p, au, &w.member, w.a, r1, Some("x"), false).await),
        ("lineage_state_conflict", 409)
    );
    assert_eq!(
        lcode(inv(p, au, &w.member, w.a, r1, Some("x"), true).await),
        ("lineage_state_conflict", 409)
    );

    // Invalidate r2, twice; supersede an invalidated record; undo it; undo again; invalidate again after the undo.
    assert_eq!(
        inv(p, au, &w.member, w.a, r2, Some("wrong"), false)
            .await
            .expect("invalidate")
            .state,
        "invalidated"
    );
    assert_eq!(
        lcode(inv(p, au, &w.member, w.a, r2, Some("again"), false).await),
        ("lineage_state_conflict", 409)
    );
    assert_eq!(
        lcode(sup(p, au, &w.member, w.a, r2, r3, Some("x")).await),
        ("lineage_state_conflict", 409)
    );
    assert_eq!(
        inv(p, au, &w.member, w.a, r2, Some("it was right"), true)
            .await
            .expect("undo")
            .state,
        "active"
    );
    assert_eq!(
        state_of(&w, w.a, r2).await,
        ("active".into(), None, Some("it was right".into())),
        "the state row stays, active"
    );
    assert_eq!(
        lcode(inv(p, au, &w.member, w.a, r2, Some("x"), true).await),
        ("lineage_state_conflict", 409),
        "undo of an active record"
    );
    assert_eq!(
        inv(p, au, &w.member, w.a, r2, Some("wrong again"), false)
            .await
            .expect("invalidate again")
            .state,
        "invalidated"
    );
    // Undo of a record that was never invalidated.
    assert_eq!(
        lcode(inv(p, au, &w.member, w.a, r3, Some("x"), true).await),
        ("lineage_state_conflict", 409)
    );

    // Audit: one row per SUCCESS, the registry's names, the target the record id; a refusal writes none.
    assert_eq!(
        audit_actions(&w),
        [
            ("record.superseded".to_string(), r1.to_string()),
            ("record.invalidated".to_string(), r2.to_string()),
            ("record.invalidation_undone".to_string(), r2.to_string()),
            ("record.invalidated".to_string(), r2.to_string()),
        ]
    );
    assert!(
        w.audit
            .0
            .lock()
            .expect("lock")
            .iter()
            .all(|(_, _, empty)| *empty),
        "details are empty"
    );
    // The log: gapless sequence, from and to, in order.
    let mut tx = begin(&w, w.a).await;
    let rows: Vec<(i64, String, String)> =
        sqlx::query_as("SELECT seq, from_state, to_state FROM records.state_log ORDER BY seq")
            .fetch_all(tx.executor().expect("executor"))
            .await
            .expect("log");
    let want: Vec<(i64, String, String)> = [
        (1, "active", "superseded"),
        (2, "active", "invalidated"),
        (3, "invalidated", "active"),
        (4, "active", "invalidated"),
    ]
    .into_iter()
    .map(|(n, f, t)| (n, f.to_owned(), t.to_owned()))
    .collect();
    assert_eq!(rows, want);
}

#[tokio::test]
async fn ln02_reasons_and_the_order_step0_404_409() {
    let w = setup("ln02").await;
    let (r1, r2) = (
        note(&w, w.a, &w.member, "r1").await,
        note(&w, w.a, &w.member, "r2").await,
    );
    let (p, au) = (&w.pool, &w.audit);
    // 422: absent, empty, 513 bytes, NUL; self-supersede; 512 bytes pass.
    assert_eq!(
        lcode(sup(p, au, &w.member, w.a, r1, r2, None).await),
        ("reason_required", 422)
    );
    assert_eq!(
        lcode(sup(p, au, &w.member, w.a, r1, r2, Some("")).await),
        ("reason_required", 422)
    );
    assert_eq!(
        lcode(sup(p, au, &w.member, w.a, r1, r2, Some(&"x".repeat(513))).await),
        ("invalid_request", 422)
    );
    assert_eq!(
        lcode(sup(p, au, &w.member, w.a, r1, r2, Some("a\u{0}b")).await),
        ("invalid_request", 422)
    );
    assert_eq!(
        lcode(sup(p, au, &w.member, w.a, r1, r1, Some("me")).await),
        ("self_supersede", 422)
    );
    assert_eq!(
        lcode(inv(p, au, &w.member, w.a, r1, None, false).await),
        ("reason_required", 422)
    );
    assert_eq!(
        lcode(inv(p, au, &w.member, w.a, r1, Some(""), false).await),
        ("reason_required", 422)
    );
    assert!(
        audit_actions(&w).is_empty(),
        "no refusal writes an audit row"
    );
    assert_eq!(state_of(&w, w.a, r1).await.0, "active", "nothing changed");
    // A 512-byte reason (multibyte at the edge: 256 two-byte characters) passes.
    inv(
        p,
        au,
        &w.member,
        w.a,
        r1,
        Some(&"\u{e9}".repeat(256)),
        false,
    )
    .await
    .expect("512 bytes");
    // STEP 0 (R-16) comes before the reads: an invalidated record with an empty reason is reason_required, not 409.
    assert_eq!(
        lcode(inv(p, au, &w.member, w.a, r1, Some(""), false).await),
        ("reason_required", 422)
    );
    assert_eq!(
        lcode(sup(p, au, &w.member, w.a, r1, r1, Some("")).await),
        ("reason_required", 422),
        "per-field before cross-field"
    );
    assert_eq!(
        lcode(sup(p, au, &w.member, w.a, r1, r1, Some("ok")).await),
        ("self_supersede", 422),
        "self-supersede is step 0 too"
    );
    // With a good reason, 404 comes before 409: an unknown target or an unknown `by`; then the 409 of the state.
    let unknown = RecordId::new(&SystemClock);
    assert_eq!(
        lcode(sup(p, au, &w.member, w.a, unknown, r2, Some("")).await),
        ("reason_required", 422),
        "step 0 before 404"
    );
    assert_eq!(
        lcode(sup(p, au, &w.member, w.a, unknown, r2, Some("ok")).await),
        ("not_found", 404)
    );
    assert_eq!(
        lcode(sup(p, au, &w.member, w.a, r2, unknown, Some("ok")).await),
        ("not_found", 404)
    );
    assert_eq!(
        lcode(inv(p, au, &w.member, w.a, r1, Some("ok"), false).await),
        ("lineage_state_conflict", 409),
        "r1 is already invalidated"
    );
    let foreign = note(&w, w.b, &w.member, "in B").await;
    assert_eq!(
        lcode(sup(p, au, &w.member, w.a, r2, foreign, Some("x")).await),
        ("not_found", 404)
    );
    assert_eq!(
        lcode(inv(p, au, &w.member, w.a, foreign, Some("x"), false).await),
        ("not_found", 404)
    );
}

#[tokio::test]
async fn ln03_sections_pointer_undo_and_revisions() {
    let w = setup("ln03").await;
    let (p, au) = (&w.pool, &w.audit);
    let mut revs = vec![];
    for n in 1..=3 {
        let c = write(&w, &w.lead, w.a, json!({"type":"knowledge","summary":format!("rules {n}"),"section":"rules-project","body":"b"}))
            .await
            .expect("write");
        revs.push(c.id);
    }
    let name = sec_name("rules-project");
    let pointer = |w: &World| {
        let (pool, name) = (Arc::clone(&w.pool), name);
        let a = w.a;
        async move {
            let mut tx = ModuleTx::begin(&pool, ctx(ID, Some(a)), TxOptions::default())
                .await
                .expect("begin");
            section_current(&mut tx, &name)
                .await
                .expect("pointer")
                .map(|s| s.revision)
        }
    };
    assert_eq!(pointer(&w).await, Some(3));
    // Invalidating a revision that is not current leaves the pointer; superseding a section revision is refused.
    inv(p, au, &w.lead, w.a, revs[0], Some("old"), false)
        .await
        .expect("invalidate rev1");
    assert_eq!(pointer(&w).await, Some(3));
    let other = note(&w, w.a, &w.member, "o").await;
    assert_eq!(
        lcode(sup(p, au, &w.lead, w.a, revs[2], other, Some("content moved")).await),
        ("lineage_state_conflict", 409),
        "a section revision cannot be superseded (cox W1-36, Q-2)"
    );
    assert_eq!(pointer(&w).await, Some(3), "so the pointer is untouched");
    // Undo the superseded? No: a superseded record cannot be undone; use a fresh section state below.
    // Invalidate the CURRENT revision (3 is superseded, so use revision 2 after making it current by a new write).
    let c4 = write(
        &w,
        &w.lead,
        w.a,
        json!({"type":"knowledge","summary":"rules 4","section":"rules-project","body":"b"}),
    )
    .await
    .expect("rev4");
    assert_eq!(c4.section_revision, Some(4));
    inv(p, au, &w.lead, w.a, c4.id, Some("withdrawn"), false)
        .await
        .expect("invalidate current");
    assert_eq!(pointer(&w).await, None, "no fallback to an older revision");
    // The next write gets last_revision + 1, never a reused number.
    let c5 = write(
        &w,
        &w.lead,
        w.a,
        json!({"type":"knowledge","summary":"rules 5","section":"rules-project","body":"b"}),
    )
    .await
    .expect("rev5");
    assert_eq!(c5.section_revision, Some(5));
    // Undo of revision 4 while revision 5 exists: 409 (a higher revision exists, whatever its state).
    assert_eq!(
        lcode(inv(p, au, &w.lead, w.a, c4.id, Some("back"), true).await),
        ("lineage_state_conflict", 409)
    );
    // Invalidate 5, then undo 5 (no higher revision): the pointer returns to it; undo of 4 is still 409.
    inv(p, au, &w.lead, w.a, c5.id, Some("no"), false)
        .await
        .expect("invalidate 5");
    assert_eq!(pointer(&w).await, None);
    inv(p, au, &w.lead, w.a, c5.id, Some("yes"), true)
        .await
        .expect("undo 5");
    assert_eq!(pointer(&w).await, Some(5));
    assert_eq!(
        lcode(inv(p, au, &w.lead, w.a, c4.id, Some("back"), true).await),
        ("lineage_state_conflict", 409)
    );
}

#[tokio::test]
async fn ln04_eight_simultaneous_supersedes_have_one_winner() {
    let w = setup("ln04").await;
    let target = note(&w, w.a, &w.member, "target").await;
    let mut successors = vec![];
    for n in 0..8 {
        successors.push(note(&w, w.a, &w.member, &format!("s{n}")).await);
    }
    let mut set = tokio::task::JoinSet::new();
    for by in successors.clone() {
        let (pool, audit, who, project) = (
            Arc::clone(&w.pool),
            Arc::clone(&w.audit),
            w.member.clone(),
            w.a,
        );
        set.spawn(async move {
            (
                by,
                sup(&pool, &audit, &who, project, target, by, Some("race")).await,
            )
        });
    }
    let (mut wins, mut conflicts) = (vec![], 0);
    while let Some(r) = set.join_next().await {
        match r.expect("task") {
            (by, Ok(_)) => wins.push(by),
            (_, Err(p)) => {
                assert_eq!((p.code, p.status), ("lineage_state_conflict", 409));
                conflicts += 1;
            }
        }
    }
    assert_eq!((wins.len(), conflicts), (1, 7), "one winner, seven 409");
    assert_eq!(
        state_of(&w, w.a, target).await.1,
        Some(wins[0].to_string()),
        "superseded_by names the winner"
    );
    let mut tx = begin(&w, w.a).await;
    let (rows, logs): (i64, i64) = (
        sqlx::query_scalar("SELECT count(*) FROM records.state WHERE record_id = $1::uuid")
            .bind(target.to_string())
            .fetch_one(tx.executor().expect("executor"))
            .await
            .expect("n"),
        sqlx::query_scalar("SELECT count(*) FROM records.state_log WHERE record_id = $1::uuid")
            .bind(target.to_string())
            .fetch_one(tx.executor().expect("executor"))
            .await
            .expect("n"),
    );
    assert_eq!((rows, logs), (1, 1));
    assert_eq!(audit_actions(&w).len(), 1, "one audit row");

    // A supersede racing an invalidate of another record: one 200, one 409.
    let t2 = note(&w, w.a, &w.member, "t2").await;
    let by = successors[0];
    let ((pa, aa, wa), (pb, ab, wb)) = (
        (Arc::clone(&w.pool), Arc::clone(&w.audit), w.member.clone()),
        (Arc::clone(&w.pool), Arc::clone(&w.audit), w.member.clone()),
    );
    let project = w.a;
    let h1 = tokio::spawn(async move { sup(&pa, &aa, &wa, project, t2, by, Some("s")).await });
    let h2 = tokio::spawn(async move { inv(&pb, &ab, &wb, project, t2, Some("i"), false).await });
    let (r1, r2) = (h1.await.expect("t1"), h2.await.expect("t2"));
    assert_eq!(
        r1.is_ok() as u8 + r2.is_ok() as u8,
        1,
        "exactly one of supersede and invalidate wins: {r1:?} {r2:?}"
    );
}

#[tokio::test]
async fn ln05_lineage_targets_of_another_project_are_invisible() {
    let w = setup("ln05").await;
    let id = note(&w, w.a, &w.member, "mine").await;
    let mut tx = begin(&w, w.b).await;
    assert!(lineage_target(&mut tx, id).await.expect("target").is_none());
    assert!(!record_exists(&mut tx, id).await.expect("exists"));
    let mut tx = begin(&w, w.a).await;
    let t = lineage_target(&mut tx, id)
        .await
        .expect("target")
        .expect("found");
    assert_eq!(
        (
            t.id,
            t.author_kind.as_str(),
            t.author_id.as_str(),
            t.state.as_str()
        ),
        (id, "worker", w.member.worker.to_string().as_str(), "active")
    );
    assert!(record_exists(&mut tx, id).await.expect("exists"));
}

// ---- cox W1-36: Q-1 the replacement must be ACTIVE (no cycle), Q-2 a section revision cannot be superseded ----

#[tokio::test]
async fn ln06_the_replacement_must_be_active_so_a_cycle_is_impossible() {
    let w = setup("ln06").await;
    let (a, b, c) = (
        note(&w, w.a, &w.member, "a").await,
        note(&w, w.a, &w.member, "b").await,
        note(&w, w.a, &w.member, "c").await,
    );
    let (p, au) = (&w.pool, &w.audit);
    sup(p, au, &w.member, w.a, a, b, Some("a by b"))
        .await
        .expect("a by b");
    // The cycle: b is still active, but a (the new replacement) is superseded: 409, nothing written.
    assert_eq!(
        lcode(sup(p, au, &w.member, w.a, b, a, Some("b by a")).await),
        ("lineage_state_conflict", 409)
    );
    assert_eq!(state_of(&w, w.a, b).await.0, "active", "b is unchanged");
    // A replacement that is invalidated or superseded is refused; an active one is fine.
    inv(p, au, &w.member, w.a, c, Some("withdrawn"), false)
        .await
        .expect("invalidate c");
    let d = note(&w, w.a, &w.member, "d").await;
    assert_eq!(
        lcode(sup(p, au, &w.member, w.a, d, c, Some("d by c")).await),
        ("lineage_state_conflict", 409),
        "by is invalidated"
    );
    assert_eq!(
        lcode(sup(p, au, &w.member, w.a, d, a, Some("d by a")).await),
        ("lineage_state_conflict", 409),
        "by is superseded"
    );
    sup(p, au, &w.member, w.a, d, b, Some("d by b"))
        .await
        .expect("b is active");
    // Unchanged rulings: unknown by is 404, self is 422 (step 0), a bad reason is 422 before any read.
    let unknown = RecordId::new(&SystemClock);
    assert_eq!(
        lcode(sup(p, au, &w.member, w.a, b, unknown, Some("x")).await),
        ("not_found", 404)
    );
    assert_eq!(
        lcode(sup(p, au, &w.member, w.a, b, b, Some("x")).await),
        ("self_supersede", 422)
    );
    // Audit and log carry only the three successes (a by b, invalidate c, d by b).
    assert_eq!(audit_actions(&w).len(), 3);
}

#[tokio::test]
async fn ln07_two_records_superseding_each_other_at_once_never_both_win() {
    let w = setup("ln07").await;
    for round in 0..6 {
        let (a, b) = (
            note(&w, w.a, &w.member, &format!("a{round}")).await,
            note(&w, w.a, &w.member, &format!("b{round}")).await,
        );
        let (p1, a1, w1) = (Arc::clone(&w.pool), Arc::clone(&w.audit), w.member.clone());
        let (p2, a2, w2) = (Arc::clone(&w.pool), Arc::clone(&w.audit), w.member.clone());
        let project = w.a;
        let h1 =
            tokio::spawn(async move { sup(&p1, &a1, &w1, project, a, b, Some("a by b")).await });
        let h2 =
            tokio::spawn(async move { sup(&p2, &a2, &w2, project, b, a, Some("b by a")).await });
        let (r1, r2) = (h1.await.expect("t1"), h2.await.expect("t2"));
        assert_eq!(
            u8::from(r1.is_ok()) + u8::from(r2.is_ok()),
            1,
            "round {round}: exactly one wins, never a cycle: {r1:?} {r2:?}"
        );
        let (sa, sb) = (state_of(&w, w.a, a).await.0, state_of(&w, w.a, b).await.0);
        assert!(
            sa == "active" || sb == "active",
            "round {round}: a head stays active ({sa}, {sb})"
        );
    }
}

#[tokio::test]
async fn ln08_a_section_revision_cannot_be_superseded_only_withdrawn() {
    let w = setup("ln08").await;
    let (p, au) = (&w.pool, &w.audit);
    let rev = |n: u32| json!({"type":"knowledge","summary":format!("rules {n}"),"section":"rules-project","body":"b"});
    let (r1, r2) = (
        write(&w, &w.lead, w.a, rev(1)).await.expect("rev1").id,
        write(&w, &w.lead, w.a, rev(2)).await.expect("rev2").id,
    );
    let other = note(&w, w.a, &w.member, "other").await;
    assert_eq!(
        lcode(sup(p, au, &w.lead, w.a, r2, other, Some("x")).await),
        ("lineage_state_conflict", 409),
        "the current revision"
    );
    assert_eq!(
        lcode(sup(p, au, &w.lead, w.a, r1, other, Some("x")).await),
        ("lineage_state_conflict", 409),
        "an older revision too"
    );
    assert_eq!(state_of(&w, w.a, r2).await.0, "active");
    // Withdrawal by invalidation still works, and the 409 is the state check: step 0 and 404 come first.
    assert_eq!(
        lcode(sup(p, au, &w.lead, w.a, r2, other, Some("")).await),
        ("reason_required", 422)
    );
    inv(p, au, &w.lead, w.a, r1, Some("old"), false)
        .await
        .expect("invalidate an older revision");
    // The orientation section is a section too.
    let o = write(&w, &w.lead, w.a, orientation())
        .await
        .expect("orientation")
        .id;
    assert_eq!(
        lcode(sup(p, au, &w.lead, w.a, o, other, Some("x")).await),
        ("lineage_state_conflict", 409)
    );
}

/// hale Q-1: the replacement (or the target) changes between the authorize reads and the write. The write decides: 409, nothing written.
#[tokio::test]
async fn ln09_a_change_between_the_read_and_the_write_is_decided_at_the_write() {
    let w = setup("ln09").await;
    let (p, au) = (&w.pool, &w.audit);
    let (a, b, c) = (
        note(&w, w.a, &w.member, "a").await,
        note(&w, w.a, &w.member, "b").await,
        note(&w, w.a, &w.member, "c").await,
    );
    let env = Env {
        project: w.a,
        clock: &SystemClock,
        audit: &*w.audit,
    };

    // 1. `by` is invalidated after the reads.
    let mut tx = ModuleTx::begin(p, ctx(ID, Some(w.a)), TxOptions::default())
        .await
        .expect("begin");
    let (t, reason) = authorize_supersede(&mut tx, a, b, Some("a by b"))
        .await
        .expect("the reads pass: b is active now");
    inv(p, au, &w.member, w.a, b, Some("withdrawn"), false)
        .await
        .expect("invalidate b meanwhile");
    let r = execute_supersede(&mut tx, &env, &w.member, &t, b, &reason).await;
    assert_eq!(
        lcode(r),
        ("lineage_state_conflict", 409),
        "b is no longer active at the write"
    );
    drop(tx);
    assert_eq!(
        state_of(&w, w.a, a).await.0,
        "active",
        "nothing was written"
    );

    // 2. `by` is superseded after the reads.
    let mut tx = ModuleTx::begin(p, ctx(ID, Some(w.a)), TxOptions::default())
        .await
        .expect("begin");
    let (t, reason) = authorize_supersede(&mut tx, a, c, Some("a by c"))
        .await
        .expect("c is active now");
    let d = note(&w, w.a, &w.member, "d").await;
    sup(p, au, &w.member, w.a, c, d, Some("c by d"))
        .await
        .expect("supersede c meanwhile");
    let r = execute_supersede(&mut tx, &env, &w.member, &t, c, &reason).await;
    assert_eq!(
        lcode(r),
        ("lineage_state_conflict", 409),
        "c is superseded at the write"
    );
    drop(tx);

    // 3. the TARGET is invalidated after the reads.
    let mut tx = ModuleTx::begin(p, ctx(ID, Some(w.a)), TxOptions::default())
        .await
        .expect("begin");
    let (t, reason) = authorize_supersede(&mut tx, a, d, Some("a by d"))
        .await
        .expect("a is active now");
    inv(p, au, &w.member, w.a, a, Some("gone"), false)
        .await
        .expect("invalidate a meanwhile");
    let r = execute_supersede(&mut tx, &env, &w.member, &t, d, &reason).await;
    assert_eq!(
        lcode(r),
        ("lineage_state_conflict", 409),
        "a is no longer active at the write"
    );
}

// ---- REC-36(e) and REC-40: lineage races, RACE_ROUNDS rounds each (cox, 6 Nov 13:36Z) ----
// Every round starts both sides behind a barrier so they reach the database together; the runtime is multi-threaded. The rules the
// cells hold: exactly the outcomes the state machine allows, no deadlock (any error other than the 409 of the state fails the
// cell), `state_log.seq` has no gap and no repeat, and one audit row per success.

/// Rounds per race cell: tests/mod-records.md asks for 50 (REC-36(e), REC-40).
const RACE_ROUNDS: usize = 50;

/// `state_log` holds exactly `successes` rows for the project and `seq` runs 1..=successes without a gap or a repeat.
async fn assert_log_gapless(w: &World, project: ProjectId, successes: usize) {
    let want = i64::try_from(successes).expect("small");
    let mut tx = begin(w, project).await;
    let (n, distinct, min, max): (i64, i64, Option<i64>, Option<i64>) = sqlx::query_as(
        "SELECT count(*), count(DISTINCT seq), min(seq), max(seq) FROM records.state_log",
    )
    .fetch_one(tx.executor().expect("executor"))
    .await
    .expect("log");
    assert_eq!((n, distinct), (want, want), "one log row per success");
    assert_eq!(
        (min, max),
        (Some(1), Some(want)),
        "seq has no gap and no repeat"
    );
}

fn audit_count(w: &World, action: &str) -> usize {
    audit_actions(w).iter().filter(|(a, _)| a == action).count()
}

/// REC-36(e): two records superseding each other at the same instant, round after round. Exactly one side wins, the other is the
/// 409 of the state, and the winner's replacement is a record that stays active, so a cycle is never formed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ln10_fifty_rounds_of_two_records_superseding_each_other_never_form_a_cycle() {
    let w = setup("ln10").await;
    for round in 0..RACE_ROUNDS {
        let (a, b) = (
            note(&w, w.a, &w.member, &format!("a{round}")).await,
            note(&w, w.a, &w.member, &format!("b{round}")).await,
        );
        let gate = Arc::new(tokio::sync::Barrier::new(2));
        let (p1, a1, w1, g1) = (
            Arc::clone(&w.pool),
            Arc::clone(&w.audit),
            w.member.clone(),
            Arc::clone(&gate),
        );
        let (p2, a2, w2, g2) = (
            Arc::clone(&w.pool),
            Arc::clone(&w.audit),
            w.member.clone(),
            gate,
        );
        let project = w.a;
        let h1 = tokio::spawn(async move {
            g1.wait().await;
            sup(&p1, &a1, &w1, project, a, b, Some("a by b")).await
        });
        let h2 = tokio::spawn(async move {
            g2.wait().await;
            sup(&p2, &a2, &w2, project, b, a, Some("b by a")).await
        });
        let (r1, r2) = (h1.await.expect("t1"), h2.await.expect("t2"));
        assert_eq!(
            u8::from(r1.is_ok()) + u8::from(r2.is_ok()),
            1,
            "round {round}: exactly one wins: {r1:?} {r2:?}"
        );
        for loser in [&r1, &r2].into_iter().filter_map(|r| r.as_ref().err()) {
            assert_eq!(
                (loser.code, loser.status),
                ("lineage_state_conflict", 409),
                "round {round}: the loser is the 409 of the state, nothing else"
            );
        }
        let (sa, sb) = (state_of(&w, w.a, a).await, state_of(&w, w.a, b).await);
        match (sa.0.as_str(), sb.0.as_str()) {
            ("superseded", "active") => assert_eq!(sa.1, Some(b.to_string()), "a names b"),
            ("active", "superseded") => assert_eq!(sb.1, Some(a.to_string()), "b names a"),
            other => panic!(
                "round {round}: one is superseded and the other is the active head, got {other:?}"
            ),
        }
    }
    assert_log_gapless(&w, w.a, RACE_ROUNDS).await;
    assert_eq!(audit_count(&w, "record.superseded"), RACE_ROUNDS);
    assert_eq!(
        audit_actions(&w).len(),
        RACE_ROUNDS,
        "nothing else is audited"
    );
}

/// REC-40: invalidating the CURRENT revision of a section while a new revision is written. Both succeed in either order, the new
/// revision gets the next number (none skipped, none repeated) and the pointer ends on the NEW revision, never cleared by the
/// invalidation of the old one.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ln11_fifty_rounds_of_invalidating_the_current_revision_racing_a_new_revision() {
    let w = setup("ln11").await;
    let rev = |n: usize| json!({"type":"knowledge","summary":format!("rules {n}"),"section":"rules-project","body":"b"});
    let name = sec_name("rules-project");
    let mut current = write(&w, &w.lead, w.a, rev(1)).await.expect("rev1").id;
    for round in 0..RACE_ROUNDS {
        let want = i32::try_from(round + 2).expect("small");
        let gate = Arc::new(tokio::sync::Barrier::new(2));
        let (p1, a1, w1, g1) = (
            Arc::clone(&w.pool),
            Arc::clone(&w.audit),
            w.lead.clone(),
            Arc::clone(&gate),
        );
        let (p2, a2, w2, g2) = (
            Arc::clone(&w.pool),
            Arc::clone(&w.audit),
            w.lead.clone(),
            gate,
        );
        let (project, target, body) = (w.a, current, rev(round + 2));
        let h1 = tokio::spawn(async move {
            g1.wait().await;
            inv(&p1, &a1, &w1, project, target, Some("withdraw"), false).await
        });
        let h2 = tokio::spawn(async move {
            g2.wait().await;
            write_with(&p2, &a2, &w2, project, body).await
        });
        let (r1, r2) = (h1.await.expect("t1"), h2.await.expect("t2"));
        r1.unwrap_or_else(|p| {
            panic!(
                "round {round}: invalidating an active revision always succeeds: {}",
                p.code
            )
        });
        let created = r2.unwrap_or_else(|p| {
            panic!("round {round}: a new revision always succeeds: {}", p.code)
        });
        assert_eq!(
            created.section_revision,
            Some(want),
            "round {round}: no number skipped or repeated"
        );
        let mut tx = begin(&w, w.a).await;
        let pointer = section_current(&mut tx, &name)
            .await
            .expect("pointer")
            .expect("a pointer exists");
        assert_eq!(
            (pointer.record_id, pointer.revision),
            (created.id, want),
            "round {round}: the pointer names the new revision whichever came first"
        );
        drop(tx);
        assert_eq!(state_of(&w, w.a, target).await.0, "invalidated");
        current = created.id;
    }
    assert_log_gapless(&w, w.a, RACE_ROUNDS).await;
    assert_eq!(audit_count(&w, "record.invalidated"), RACE_ROUNDS);
    assert_eq!(
        audit_count(&w, "section.write_reserved"),
        RACE_ROUNDS + 1,
        "one per revision written"
    );
}

/// REC-40: a supersede racing the undo of the same invalidated record. The undo always succeeds; the supersede either runs after it
/// (200, the record ends superseded by the replacement) or before it (the 409 of the state, nothing written, the record ends active).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ln12_fifty_rounds_of_a_supersede_racing_an_undo() {
    let w = setup("ln12").await;
    let mut superseded = 0usize;
    for round in 0..RACE_ROUNDS {
        let (a, b) = (
            note(&w, w.a, &w.member, &format!("a{round}")).await,
            note(&w, w.a, &w.member, &format!("b{round}")).await,
        );
        inv(
            &w.pool,
            &w.audit,
            &w.member,
            w.a,
            a,
            Some("withdrawn"),
            false,
        )
        .await
        .expect("invalidate a");
        let gate = Arc::new(tokio::sync::Barrier::new(2));
        let (p1, a1, w1, g1) = (
            Arc::clone(&w.pool),
            Arc::clone(&w.audit),
            w.member.clone(),
            Arc::clone(&gate),
        );
        let (p2, a2, w2, g2) = (
            Arc::clone(&w.pool),
            Arc::clone(&w.audit),
            w.member.clone(),
            gate,
        );
        let project = w.a;
        // One side starts a little after the other, by a different amount each round (0 to 6 ms), the supersede on even rounds and
        // the undo on odd ones, so the rounds cover both orders and the overlap between them instead of one lucky interleaving.
        let lag =
            std::time::Duration::from_micros(u64::try_from(round % 10 / 2).expect("small") * 1500);
        let (lag_supersede, lag_undo) = if round % 2 == 0 {
            (lag, std::time::Duration::ZERO)
        } else {
            (std::time::Duration::ZERO, lag)
        };
        let h1 = tokio::spawn(async move {
            g1.wait().await;
            tokio::time::sleep(lag_supersede).await;
            sup(&p1, &a1, &w1, project, a, b, Some("a by b")).await
        });
        let h2 = tokio::spawn(async move {
            g2.wait().await;
            tokio::time::sleep(lag_undo).await;
            inv(&p2, &a2, &w2, project, a, Some("back"), true).await
        });
        let (r1, r2) = (h1.await.expect("t1"), h2.await.expect("t2"));
        r2.unwrap_or_else(|p| panic!("round {round}: the undo always succeeds: {}", p.code));
        let sa = state_of(&w, w.a, a).await;
        match r1 {
            Ok(_) => {
                superseded += 1;
                assert_eq!(
                    (sa.0.as_str(), sa.1),
                    ("superseded", Some(b.to_string())),
                    "round {round}: the supersede won after the undo"
                );
            }
            Err(p) => {
                assert_eq!(
                    (p.code, p.status),
                    ("lineage_state_conflict", 409),
                    "round {round}: the supersede lost with the 409 of the state"
                );
                assert_eq!(sa.0, "active", "round {round}: nothing was written");
            }
        }
        assert_eq!(state_of(&w, w.a, b).await.0, "active");
    }
    eprintln!("ln12: the supersede ran after the undo in {superseded} of {RACE_ROUNDS} rounds");
    assert_log_gapless(&w, w.a, RACE_ROUNDS * 2 + superseded).await;
    assert_eq!(audit_count(&w, "record.invalidated"), RACE_ROUNDS);
    assert_eq!(audit_count(&w, "record.invalidation_undone"), RACE_ROUNDS);
    assert_eq!(audit_count(&w, "record.superseded"), superseded);
}
