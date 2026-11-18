//! F1-31 category 14 (TESTS r7; A7 r4 item 2 step 7). A drop writes a `maintenance_runs` row,
//! and a `DEFERRABLE INITIALLY DEFERRED` constraint trigger refuses the commit
//! (`maintenance_unaudited`) unless the run's audit record, with the same `run_id`, was written in
//! the same transaction. The trigger itself is pinned and verified at startup.
//! RED (b2): the template, the startup check and the b1 core are `todo!()`.
use cortex_db::maintenance::templates;
use cortex_db::maintenance::{DefinerError, MaintError, TriggerFault, TriggerId};
use cortex_testkit::maint::{
    Maint, Mutation, Refusal, SPEC_A, audit, drop, exec, leaf, maint, maint_with, month, refused,
    refused_at_commit, uuid, variant,
};
use cortex_testkit::within;

/// A leaf at `offset`, eligible.
async fn eligible_leaf(w: &Maint, offset: i32) {
    w.make_leaf("test_a", w.a, offset).await;
    w.flag(w.a, offset, true).await;
}

/// r2 (vera's F1B-B2-01, E12): the commit-time refusal is exactly the sanitised code, never a
/// substring of some other text.
fn unaudited(r: &Result<(), cortex_db::DbError>) -> bool {
    *r == Err(refused_at_commit(MaintError::Unaudited))
}

/// vera's F1B-B2-03: an otherwise exact trigger whose only change is `WHEN (false)`.
const FALSE_WHEN: &str = "DROP TRIGGER maintenance_audited ON test_a.maintenance_runs; \
     CREATE CONSTRAINT TRIGGER maintenance_audited AFTER INSERT ON test_a.maintenance_runs \
     DEFERRABLE INITIALLY DEFERRED FOR EACH ROW WHEN (false) EXECUTE FUNCTION test_a.maintenance_audit(); \
     ALTER TABLE test_a.maintenance_runs ENABLE ALWAYS TRIGGER maintenance_audited;";

/// The plant really is otherwise exact: AFTER, INSERT only, ROW, a constraint trigger,
/// DEFERRABLE INITIALLY DEFERRED, ENABLE ALWAYS, the same function; and it has a WHEN.
async fn only_the_when_differs(w: &Maint) -> bool {
    let mut admin = w.db.admin().await;
    sqlx::query_scalar(
        "select (t.tgtype::int & 1) = 1 and (t.tgtype::int & 2) = 0 and (t.tgtype::int & 4) = 4 \
         and (t.tgtype::int & 8) = 0 and (t.tgtype::int & 16) = 0 and (t.tgtype::int & 32) = 0 and (t.tgtype::int & 64) = 0 \
         and t.tgconstraint <> 0 and t.tgdeferrable and t.tginitdeferred and t.tgenabled = 'A' \
         and t.tgfoid = 'test_a.maintenance_audit()'::regprocedure and t.tgqual is not null \
         from pg_trigger t where t.tgrelid = 'test_a.maintenance_runs'::regclass \
         and t.tgname = 'maintenance_audited'",
    )
    .fetch_one(&mut admin)
    .await
    .expect("pg_trigger")
}

fn trigger_id() -> TriggerId {
    TriggerId {
        schema: "test_a".into(),
        table: "maintenance_runs".into(),
        trigger: "maintenance_audited".into(),
    }
}

#[tokio::test]
async fn cat14_drops_and_one_audit_for_their_shared_run_commit_once() {
    within(180, async {
        let w = maint("c14ok").await;
        let a = uuid(w.a);
        eligible_leaf(&w, -3).await;
        eligible_leaf(&w, -4).await;
        let run = uuid::Uuid::now_v7();
        let mut tx = w.begin("test_a", Some(w.a)).await;
        for o in [-3, -4] {
            assert_eq!(
                drop(&mut tx, "test_a", Some(a), Some(&month(o)), run).await,
                Ok("dropped".into())
            );
        }
        audit(&mut tx, a, run, "drop")
            .await
            .expect("one audit for the run");
        tx.commit().await.expect("commit");
        assert_eq!(w.runs("test_a", run).await, 2, "a run row per drop");
        assert_eq!(w.audits(run).await, 1, "one audit per run, not per call");
        for o in [-3, -4] {
            assert_eq!(w.relkind("test_a", &leaf(&a, o)).await, None);
        }
    })
    .await
}

#[tokio::test]
async fn cat14_a_direct_drop_with_no_audit_is_refused_at_commit() {
    within(180, async {
        let w = maint("c14noaudit").await;
        let a = uuid(w.a);
        eligible_leaf(&w, -3).await;
        let run = uuid::Uuid::now_v7();
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            drop(&mut tx, "test_a", Some(a), Some(&month(-3)), run).await,
            Ok("dropped".into())
        );
        let r = tx.commit().await;
        assert!(unaudited(&r), "commit without an audit: {r:?}");
        assert_eq!(
            w.relkind("test_a", &leaf(&a, -3)).await.as_deref(),
            Some("r")
        );
        assert_eq!(w.runs("test_a", run).await, 0);
    })
    .await
}

#[tokio::test]
async fn cat14_a_fault_after_the_ddl_or_in_the_audit_write_rolls_back_both() {
    within(180, async {
        let w = maint("c14fault").await;
        let a = uuid(w.a);
        eligible_leaf(&w, -3).await;
        // After the DDL.
        let run = uuid::Uuid::now_v7();
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            drop(&mut tx, "test_a", Some(a), Some(&month(-3)), run).await,
            Ok("dropped".into())
        );
        assert!(exec(&mut tx, "select 1/0").await.is_err());
        tx.rollback().await.ok();
        // Inside the audit write: test_audit.r refuses v = 'boom'.
        let run2 = uuid::Uuid::now_v7();
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            drop(&mut tx, "test_a", Some(a), Some(&month(-3)), run2).await,
            Ok("dropped".into())
        );
        assert!(audit(&mut tx, a, run2, "boom").await.is_err());
        assert!(
            tx.commit().await.is_err(),
            "commit after a failed audit write"
        );
        assert_eq!(
            w.relkind("test_a", &leaf(&a, -3)).await.as_deref(),
            Some("r")
        );
        assert_eq!(
            w.runs("test_a", run).await + w.runs("test_a", run2).await,
            0
        );
        assert_eq!(w.audits(run2).await, 0);
    })
    .await
}

#[tokio::test]
async fn cat14_a_rerun_of_a_completed_drop_is_absent_and_writes_no_second_run() {
    within(180, async {
        let w = maint("c14rerun").await;
        let a = uuid(w.a);
        eligible_leaf(&w, -3).await;
        let run = uuid::Uuid::now_v7();
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            drop(&mut tx, "test_a", Some(a), Some(&month(-3)), run).await,
            Ok("dropped".into())
        );
        audit(&mut tx, a, run, "drop").await.expect("audit");
        tx.commit().await.expect("commit");
        let run2 = uuid::Uuid::now_v7();
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            drop(&mut tx, "test_a", Some(a), Some(&month(-3)), run2).await,
            Ok("absent".into())
        );
        tx.commit().await.expect("a zero-change run commits");
        assert_eq!(w.runs("test_a", run2).await, 0, "a second run row");
    })
    .await
}

#[tokio::test]
async fn cat14_an_audit_for_another_run_or_a_stale_one_does_not_satisfy_the_trigger() {
    within(180, async {
        let w = maint("c14other").await;
        let a = uuid(w.a);
        eligible_leaf(&w, -3).await;
        // Another run's audit.
        let (run, other) = (uuid::Uuid::now_v7(), uuid::Uuid::now_v7());
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            drop(&mut tx, "test_a", Some(a), Some(&month(-3)), run).await,
            Ok("dropped".into())
        );
        audit(&mut tx, a, other, "drop").await.expect("audit");
        let r = tx.commit().await;
        assert!(unaudited(&r), "another run's audit satisfied it: {r:?}");
        // A stale audit: committed earlier, with the same run_id.
        let stale = uuid::Uuid::now_v7();
        let mut tx = w.begin("test_a", Some(w.a)).await;
        audit(&mut tx, a, stale, "drop").await.expect("audit");
        tx.commit().await.expect("the earlier audit commits");
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            drop(&mut tx, "test_a", Some(a), Some(&month(-3)), stale).await,
            Ok("dropped".into())
        );
        let r = tx.commit().await;
        assert!(unaudited(&r), "a stale audit satisfied it: {r:?}");
        assert_eq!(
            w.relkind("test_a", &leaf(&a, -3)).await.as_deref(),
            Some("r")
        );
    })
    .await
}

#[tokio::test]
async fn cat14_a_rollback_leaves_neither_row_and_the_retry_writes_one_audit() {
    within(180, async {
        let w = maint("c14retry").await;
        let a = uuid(w.a);
        eligible_leaf(&w, -3).await;
        let run = uuid::Uuid::now_v7();
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            drop(&mut tx, "test_a", Some(a), Some(&month(-3)), run).await,
            Ok("dropped".into())
        );
        audit(&mut tx, a, run, "drop").await.expect("audit");
        tx.rollback().await.expect("rollback");
        assert_eq!((w.runs("test_a", run).await, w.audits(run).await), (0, 0));
        assert_eq!(
            w.relkind("test_a", &leaf(&a, -3)).await.as_deref(),
            Some("r")
        );
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            drop(&mut tx, "test_a", Some(a), Some(&month(-3)), run).await,
            Ok("dropped".into())
        );
        audit(&mut tx, a, run, "drop").await.expect("audit");
        tx.commit().await.expect("the retry commits");
        assert_eq!((w.runs("test_a", run).await, w.audits(run).await), (1, 1));
    })
    .await
}

#[tokio::test]
async fn cat14_set_constraints_immediate_refuses_at_once_never_passes() {
    within(180, async {
        let w = maint("c14immediate").await;
        let a = uuid(w.a);
        eligible_leaf(&w, -3).await;
        // After the call, with the obligation pending: it fires there.
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            drop(
                &mut tx,
                "test_a",
                Some(a),
                Some(&month(-3)),
                uuid::Uuid::now_v7()
            )
            .await,
            Ok("dropped".into())
        );
        assert_eq!(
            exec(&mut tx, "set constraints all immediate").await,
            Err(refused(MaintError::Unaudited))
        );
        tx.rollback().await.ok();
        // Before the call: the obligation's own statement refuses at its end.
        let mut tx = w.begin("test_a", Some(w.a)).await;
        exec(&mut tx, "set constraints all immediate")
            .await
            .expect("set constraints");
        assert_eq!(
            drop(
                &mut tx,
                "test_a",
                Some(a),
                Some(&month(-3)),
                uuid::Uuid::now_v7()
            )
            .await,
            Err(refused(MaintError::Unaudited))
        );
        tx.rollback().await.ok();
        assert_eq!(
            w.relkind("test_a", &leaf(&a, -3)).await.as_deref(),
            Some("r")
        );
    })
    .await
}

#[tokio::test]
async fn cat14_a_direct_call_that_never_commits_leaves_the_partition() {
    within(180, async {
        let w = maint("c14nocommit").await;
        let a = uuid(w.a);
        eligible_leaf(&w, -3).await;
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            drop(
                &mut tx,
                "test_a",
                Some(a),
                Some(&month(-3)),
                uuid::Uuid::now_v7()
            )
            .await,
            Ok("dropped".into())
        );
        std::mem::drop(tx);
        assert_eq!(
            w.relkind("test_a", &leaf(&a, -3)).await.as_deref(),
            Some("r")
        );
    })
    .await
}

#[tokio::test]
async fn cat14_startup_refuses_each_planted_trigger_fault() {
    within(600, async {
        let recreate = |body: &str| {
            format!(
                "DROP TRIGGER maintenance_audited ON test_a.maintenance_runs; {body}"
            )
        };
        let cases: Vec<(&str, String, TriggerFault)> = vec![
            (
                "missing",
                "DROP TRIGGER maintenance_audited ON test_a.maintenance_runs".into(),
                TriggerFault::Missing,
            ),
            (
                "disabled before the insert",
                "ALTER TABLE test_a.maintenance_runs DISABLE TRIGGER maintenance_audited".into(),
                TriggerFault::Disabled,
            ),
            (
                "ENABLE REPLICA instead of ALWAYS",
                "ALTER TABLE test_a.maintenance_runs ENABLE REPLICA TRIGGER maintenance_audited".into(),
                TriggerFault::Disabled,
            ),
            (
                "on the wrong table",
                recreate(
                    "CREATE CONSTRAINT TRIGGER maintenance_audited AFTER INSERT ON test_a.eligible_flags \
                     DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION test_a.maintenance_audit();",
                ),
                TriggerFault::WrongTable,
            ),
            (
                "the wrong event",
                recreate(
                    "CREATE CONSTRAINT TRIGGER maintenance_audited AFTER UPDATE ON test_a.maintenance_runs \
                     DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION test_a.maintenance_audit();",
                ),
                TriggerFault::WrongEvent,
            ),
            (
                "the wrong timing (not a deferred constraint trigger)",
                recreate(
                    "CREATE TRIGGER maintenance_audited BEFORE INSERT ON test_a.maintenance_runs \
                     FOR EACH ROW EXECUTE FUNCTION test_a.maintenance_audit();",
                ),
                TriggerFault::WrongTiming,
            ),
            (
                "another, valid function",
                recreate(
                    "CREATE FUNCTION test_a.other_trigger() RETURNS trigger LANGUAGE plpgsql \
                     AS $$ BEGIN RETURN NULL; END $$; \
                     CREATE CONSTRAINT TRIGGER maintenance_audited AFTER INSERT ON test_a.maintenance_runs \
                     DEFERRABLE INITIALLY DEFERRED FOR EACH ROW EXECUTE FUNCTION test_a.other_trigger();",
                ),
                TriggerFault::WrongFunction,
            ),
            (
                "a false WHEN on an otherwise exact trigger",
                FALSE_WHEN.into(),
                TriggerFault::HasWhen,
            ),
        ];
        for (n, (what, plant, fault)) in cases.into_iter().enumerate() {
            let w = maint(&format!("c14t_{n:02}")).await;
            assert_eq!(w.check().await, Ok(()), "{what}: the intact set starts");
            w.as_owner(&plant).await;
            if fault == TriggerFault::HasWhen {
                assert!(
                    only_the_when_differs(&w).await,
                    "the false-WHEN plant changed more than the WHEN"
                );
            }
            let got = w.check().await;
            assert!(
                got.as_ref()
                    .is_err_and(|e| e.iter().any(|x| matches!(x, DefinerError::Trigger(_, f) if *f == fault))),
                "{what}: startup did not refuse with {fault:?}: {got:?}"
            );
            if fault == TriggerFault::Missing {
                assert!(
                    got.as_ref().is_err_and(|e| e.contains(&DefinerError::Trigger(trigger_id(), fault))),
                    "the missing trigger is not named"
                );
            }
        }
    })
    .await
}

#[tokio::test]
async fn cat14_control_with_the_trigger_and_the_check_off_an_unaudited_drop_commits() {
    within(180, async {
        for (n, plant, fault) in [
            (
                0,
                "DROP TRIGGER maintenance_audited ON test_a.maintenance_runs",
                TriggerFault::Missing,
            ),
            (
                1,
                "ALTER TABLE test_a.maintenance_runs DISABLE TRIGGER maintenance_audited",
                TriggerFault::Disabled,
            ),
        ] {
            let w = maint(&format!("c14ctl_{n}")).await;
            let a = uuid(w.a);
            eligible_leaf(&w, -3).await;
            w.as_owner(plant).await;
            // The startup check is off: not run before the drop.
            let mut tx = w.begin("test_a", Some(w.a)).await;
            assert_eq!(
                drop(
                    &mut tx,
                    "test_a",
                    Some(a),
                    Some(&month(-3)),
                    uuid::Uuid::now_v7()
                )
                .await,
                Ok("dropped".into())
            );
            tx.commit()
                .await
                .expect("with the trigger off, the unaudited drop commits");
            assert_eq!(w.relkind("test_a", &leaf(&a, -3)).await, None);
            // So the startup check is what stops it.
            assert!(
                w.check()
                    .await
                    .is_err_and(|e| e.contains(&DefinerError::Trigger(trigger_id(), fault))),
                "{plant}: the startup check does not catch it"
            );
        }
    })
    .await
}

/// vera's F1B-B2-03, the checker-off partner: with the false-WHEN trigger and the startup check
/// off, the deferred check is never queued, so an unaudited drop commits; then the check names
/// `HasWhen`, so the startup check is what stops it.
#[tokio::test]
async fn cat14_control_with_a_false_when_and_the_check_off_an_unaudited_drop_commits() {
    within(180, async {
        let w = maint("c14ctl_when").await;
        let a = uuid(w.a);
        eligible_leaf(&w, -3).await;
        w.as_owner(FALSE_WHEN).await;
        assert!(only_the_when_differs(&w).await);
        let run = uuid::Uuid::now_v7();
        let mut tx = w.begin("test_a", Some(w.a)).await;
        assert_eq!(
            drop(&mut tx, "test_a", Some(a), Some(&month(-3)), run).await,
            Ok("dropped".into())
        );
        tx.commit()
            .await
            .expect("with WHEN (false) and the check off, the unaudited drop commits");
        assert_eq!(w.relkind("test_a", &leaf(&a, -3)).await, None);
        assert_eq!((w.runs("test_a", run).await, w.audits(run).await), (1, 0));
        assert!(
            w.check().await.is_err_and(
                |e| e.contains(&DefinerError::Trigger(trigger_id(), TriggerFault::HasWhen))
            ),
            "the startup check does not name HasWhen"
        );
    })
    .await
}

/// r3 (vera's F1B-B2-01, the deferred side): the commit-time refusal is mapped by SQLSTATE to the
/// sanitised code, never by copying the raw message. With the template's messages changed (a
/// wrong prefix; a message carrying the project id), the SQLSTATE kept:
/// 1. **the raw diagnostic really changed:** `SET CONSTRAINTS ALL IMMEDIATE` after an unaudited
///    drop fires the check at once, and the retained refusal shows the changed message under
///    `P0C05`;
/// 2. **a separate deferred commit is still exactly `DbError::Other("maintenance_unaudited")`**,
///    and neither its Debug nor its Display carries the project id or the wrong prefix.
///
/// The named GREEN mutant, a mapper `DbError::Other(d.message().to_owned())`, fails (2)'s exact
/// assertion in both worlds.
#[tokio::test]
async fn cat14_a_changed_deferred_message_never_reaches_the_commit_refusal() {
    within(240, async {
        let base = templates::partition_maintenance(&SPEC_A);
        for (tag, m) in [
            ("c14msg_prefix", Mutation::MessageWrongPrefix),
            ("c14msg_leak", Mutation::MessageLeaksRow),
        ] {
            let v = variant(&base, &SPEC_A, m).expect("the variant: its message markers exist");
            let w = maint_with(tag, v).await;
            let a = uuid(w.a);
            eligible_leaf(&w, -3).await;
            eligible_leaf(&w, -4).await;
            // 1. The raw diagnostic, at once.
            let mut tx = w.begin("test_a", Some(w.a)).await;
            assert_eq!(
                drop(&mut tx, "test_a", Some(a), Some(&month(-3)), uuid::Uuid::now_v7()).await,
                Ok("dropped".into())
            );
            let raw = exec(&mut tx, "set constraints all immediate").await;
            tx.rollback().await.ok();
            assert!(
                matches!(&raw, Err(Refusal::Maint { code: MaintError::Unaudited, .. })),
                "{m:?}: the SQLSTATE must be kept: {raw:?}"
            );
            assert_ne!(
                raw,
                Err(refused(MaintError::Unaudited)),
                "{m:?}: the raw message did not change, so this world measures nothing"
            );
            if m == Mutation::MessageLeaksRow {
                assert!(
                    matches!(&raw, Err(Refusal::Maint { message, .. }) if message.contains(&a.to_string())),
                    "the leaking variant's raw message does not carry the project id: {raw:?}"
                );
            }
            // 2. A separate deferred commit: exactly the sanitised code.
            let mut tx = w.begin("test_a", Some(w.a)).await;
            assert_eq!(
                drop(&mut tx, "test_a", Some(a), Some(&month(-4)), uuid::Uuid::now_v7()).await,
                Ok("dropped".into())
            );
            let r = tx.commit().await;
            assert_eq!(
                r,
                Err(refused_at_commit(MaintError::Unaudited)),
                "{m:?}: the commit refusal must be the sanitised code, never the raw message"
            );
            let shown = format!(
                "{r:?} {}",
                r.as_ref().err().map(|e| e.to_string()).unwrap_or_default()
            );
            assert!(
                !shown.contains(&a.to_string()) && !shown.contains("x_maintenance"),
                "{m:?}: the commit refusal shows the raw message: {shown}"
            );
            assert_eq!(w.relkind("test_a", &leaf(&a, -4)).await.as_deref(), Some("r"));
        }
    })
    .await
}
