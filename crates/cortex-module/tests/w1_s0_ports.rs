//! Wave 1, `wave1/ports` items (a), (b), (c) (cox W1-9, W1-10; type text: nemo `PORT-DELTA-RecordsApi-nemo-r0.md`
//! b7b99699, bob `tests/mod-handoffs-PORT-TYPES-r0.md` 18f1682b; sophi's name review). DB-free. ADDITIVE in
//! `ports.rs` and `lib.rs`; it touches no `Ctx`, no `Routes` and no proof.
//!
//! - (a) `Target.consult: bool`, default false, set only by `handoffs.create` and read only by `TaskAddressee`.
//! - (b) nemo's real `RecordView` (immutable records: a summary, an optional body, typed fields, a state, a source,
//!   an author, a section, no `revision`, `title` or `updated_at`), `RecordState`, `SourceView`; `RecordCursor`
//!   and `RecordPage` keep their shape.
//! - (c) bob's real `HandoffView`: `kind: HandoffKind`, `to: HandoffTo` (a worker or a job role), `priority: i32`.
//!
//! RED: none of the new names or fields exists, so this file does not compile.
use cortex_module::{
    HandoffKind, HandoffTo, HandoffView, RecordCursor, RecordPage, RecordState, RecordView,
    SourceView, Target, WorkerKind,
};
use cortex_types::{HandoffId, RecordId, SystemClock, TeamId, Timestamp, WorkerId};

#[test]
fn w1_pa_1_target_consult_defaults_to_false_and_is_a_plain_bool() {
    assert!(!Target::default().consult);
    let t = Target {
        consult: true,
        ..Default::default()
    };
    assert!(t.consult);
    assert_eq!(
        t,
        Target {
            consult: true,
            worker: None,
            kind: None,
            team: None,
            lead: None
        }
    );
    assert_ne!(t, Target::default(), "the flag is part of the value");
    // A literal that names the old four fields only needs `consult` or `..Default`.
    let w = WorkerId::new(&SystemClock);
    let task = Target {
        worker: Some(w),
        kind: Some(WorkerKind::Worker),
        team: Some(TeamId::new(&SystemClock)),
        lead: Some(w),
        consult: false,
    };
    assert!(!task.consult);
    assert_eq!(task.clone(), task);
}

fn record(state: RecordState, superseded_by: Option<RecordId>) -> RecordView {
    RecordView {
        id: RecordId::new(&SystemClock),
        kind: "work-product".to_owned(),
        summary: "a summary".to_owned(),
        body: Some("a body".to_owned()),
        fields: serde_json::json!({"k": 1}),
        source: Some(SourceView {
            kind: "file".to_owned(),
            reference: "docs/x.md".to_owned(),
            span: None,
        }),
        state,
        superseded_by,
        author_kind: "worker".to_owned(),
        author: WorkerId::new(&SystemClock).to_string(),
        provenance: "stated".to_owned(),
        section: Some("orientation".to_owned()),
        section_revision: Some(3),
        created_at: Timestamp::from_unix_micros(1_000),
        valid_until: None,
    }
}

#[test]
fn w1_pb_1_a_record_has_the_real_fields_and_a_lineage_state() {
    let newer = RecordId::new(&SystemClock);
    for (state, by) in [
        (RecordState::Active, None),
        (RecordState::Superseded, Some(newer)),
        (RecordState::Invalidated, None),
    ] {
        let r = record(state, by);
        assert_eq!(r.state, state);
        assert_eq!(r.superseded_by, by);
        assert_eq!(r.clone(), r, "Clone and PartialEq");
        assert!(r.fields.is_object());
    }
    // A record with no body and no source, valid for a while.
    let mut r = record(RecordState::Active, None);
    r.body = None;
    r.source = None;
    r.valid_until = Some(Timestamp::from_unix_micros(9_000));
    assert!(r.body.is_none() && r.source.is_none() && r.valid_until.is_some());
}

#[test]
fn w1_pb_2_the_record_state_is_copy_and_the_three_states_differ() {
    let s = RecordState::Active;
    let t = s; // Copy
    assert_eq!(s, t);
    assert_ne!(RecordState::Active, RecordState::Superseded);
    assert_ne!(RecordState::Superseded, RecordState::Invalidated);
}

#[test]
fn w1_pb_3_a_page_keeps_its_shape_and_the_cursor_is_created_at_and_id() {
    let r = record(RecordState::Active, None);
    let cursor = RecordCursor {
        at: r.created_at,
        id: r.id,
    };
    let page = RecordPage {
        records: vec![r.clone()],
        next: Some(cursor),
    };
    assert_eq!(page.records, [r]);
    assert_eq!(page.next, Some(cursor));
    assert_eq!(page.clone(), page);
}

fn handoff(kind: HandoffKind, to: HandoffTo, priority: i32) -> HandoffView {
    HandoffView {
        id: HandoffId::new(&SystemClock),
        kind,
        from: WorkerId::new(&SystemClock),
        to,
        state: "pending".to_owned(),
        priority,
        summary: "do the thing".to_owned(),
        created_at: Timestamp::from_unix_micros(5),
    }
}

#[test]
fn w1_pc_1_a_handoff_has_a_kind_an_addressee_that_can_be_a_job_role_and_an_integer_priority() {
    let w = WorkerId::new(&SystemClock);
    let to_worker = handoff(HandoffKind::Task, HandoffTo::Worker(w), 50);
    let to_role = handoff(
        HandoffKind::Consult,
        HandoffTo::JobRole("qa".to_owned()),
        100,
    );
    assert_eq!(to_worker.to, HandoffTo::Worker(w));
    assert_eq!(to_role.to, HandoffTo::JobRole("qa".to_owned()));
    assert_ne!(
        HandoffTo::Worker(w),
        HandoffTo::JobRole(w.to_string()),
        "a worker id and a role label that spells it are different addressees"
    );
    assert_eq!((to_worker.priority, to_role.priority), (50, 100));
    assert_eq!(to_worker.kind, HandoffKind::Task);
    assert_eq!(to_role.kind, HandoffKind::Consult);
    assert_eq!(to_role.clone(), to_role);
}

#[test]
fn w1_pc_2_the_handoff_kind_is_copy() {
    let k = HandoffKind::Consult;
    let j = k; // Copy
    assert_eq!(k, j);
    assert_ne!(HandoffKind::Task, HandoffKind::Consult);
}

/// The removed fields stay removed: a record never changes (no `revision`, no `updated_at`) and its title is
/// the `summary`. trybuild pins the diagnostics.
#[test]
fn w1_pb_4_the_old_record_fields_are_gone() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/w1_record_old_fields_fail.rs");
}
