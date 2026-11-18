//! The database's second line, as raw SQL under the real roles (S2 revision 6).
//! Rows: ID-12c, ID-17c, ID-19, ID-20, ID-21, ID-22, ID-34, ID-35. No route is involved.
//! Needs `supervise.sh`'s throwaway Postgres. These test the pack's own constraints, triggers, row-level
//! security and column grants, so they need the migration only.
mod common;

use common::seed::{F, migrate_and_seed};
use cortex_testkit::{TestDb, sqlstate};
use cortex_types::{ProjectId, SystemClock, WorkerId};
use sqlx::{Connection, PgConnection};
use std::collections::BTreeSet;

const MOD: &str = "cortex_m_identity";
const HOST: &str = "cortex_install_admin";

/// A connection that is the superuser until `SET ROLE`; project set as the transaction setting would.
async fn as_role(db: &TestDb, role: &str, project: Option<ProjectId>) -> PgConnection {
    let mut c = db.admin().await;
    let set = match role {
        MOD => "SET ROLE cortex_m_identity",
        HOST => "SET ROLE cortex_install_admin",
        _ => panic!("unknown role"),
    };
    sqlx::raw_sql(set).execute(&mut c).await.expect("set role");
    if let Some(p) = project {
        sqlx::query("SELECT set_config('cortex.project_id', $1, false)")
            .bind(p.to_string())
            .execute(&mut c)
            .await
            .expect("project");
    }
    c
}

fn code<T>(r: &Result<T, sqlx::Error>) -> Option<String> {
    r.as_ref().err().and_then(sqlstate)
}

async fn new_worker(
    c: &mut PgConnection,
    f: &F,
    roles: Vec<Option<String>>,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO identity.workers (id, project_id, kind, job_roles) VALUES ($1::uuid, $2::uuid, 'worker', $3::text[])")
        .bind(WorkerId::new(&SystemClock).to_string())
        .bind(f.a.to_string())
        .bind(roles)
        .execute(c)
        .await
        .map(|_| ())
}

fn some(xs: &[&str]) -> Vec<Option<String>> {
    xs.iter().map(|s| Some((*s).to_owned())).collect()
}

#[tokio::test]
async fn id_12c_the_job_roles_check_is_a_second_line() {
    let (db, f) = migrate_and_seed("mi_s_roles").await;
    let mut c = as_role(&db, MOD, Some(f.a)).await;
    let nine: Vec<String> = (0..9).map(|i| format!("r{i}")).collect();
    let nine: Vec<&str> = nine.iter().map(String::as_str).collect();
    for (label, roles) in [
        ("a NULL element", vec![None]),
        ("upper case", some(&["QA"])),
        ("nine roles", some(&nine)),
        ("one element with a comma", some(&["qa,dev"])),
        ("a comma role next to a valid one", some(&["qa", "dev,ops"])),
        ("a trailing newline", some(&["qa\n"])),
    ] {
        let r = new_worker(&mut c, &f, roles).await;
        assert_eq!(code(&r).as_deref(), Some("23514"), "{label}: {r:?}");
    }
    // The boundary passes: eight roles, a 32-character role.
    let eight: Vec<&str> = nine[..8].to_vec();
    assert!(new_worker(&mut c, &f, some(&eight)).await.is_ok());
    assert!(
        new_worker(&mut c, &f, some(&[&"a".repeat(32)]))
            .await
            .is_ok()
    );
    assert_eq!(
        code(&new_worker(&mut c, &f, some(&[&"a".repeat(33)])).await).as_deref(),
        Some("23514")
    );
}

#[tokio::test]
async fn id_17c_the_retire_reason_check_and_the_one_way_trigger() {
    let (db, f) = migrate_and_seed("mi_s_reason").await;
    let mut c = as_role(&db, MOD, Some(f.a)).await;
    let id = f.a_m2.to_string();
    // A reason on an active worker, an empty reason, a 513-byte reason: 23514.
    let r = sqlx::query("UPDATE identity.workers SET retired_reason = 'x' WHERE id = $1::uuid")
        .bind(&id)
        .execute(&mut c)
        .await;
    assert_eq!(code(&r).as_deref(), Some("23514"), "{r:?}");
    for reason in [String::new(), "x".repeat(513)] {
        let r = sqlx::query(
            "UPDATE identity.workers SET state = 'retired', retired_at = now(), retired_reason = $2 WHERE id = $1::uuid",
        )
        .bind(&id)
        .bind(reason)
        .execute(&mut c)
        .await;
        assert_eq!(code(&r).as_deref(), Some("23514"), "{r:?}");
    }
    // A retire by raw SQL with NO reason passes: the column is nullable; the reason is the handler's rule.
    let r = sqlx::query(
        "UPDATE identity.workers SET state = 'retired', retired_at = now() WHERE id = $1::uuid",
    )
    .bind(&id)
    .execute(&mut c)
    .await;
    assert!(r.is_ok(), "{r:?}");
    // ID-21 and the freeze: once retired, nothing about the retirement can change, and it is one way.
    for sql in [
        "UPDATE identity.workers SET retired_reason = 'late' WHERE id = $1::uuid",
        "UPDATE identity.workers SET retired_at = now() + interval '1 day' WHERE id = $1::uuid",
        "UPDATE identity.workers SET state = 'active', retired_at = NULL WHERE id = $1::uuid",
        "UPDATE identity.workers SET state = 'active' WHERE id = $1::uuid",
    ] {
        let r = sqlx::query(sql).bind(&id).execute(&mut c).await;
        assert_eq!(code(&r).as_deref(), Some("23514"), "{sql}: {r:?}");
    }
}

macro_rules! refused_delete_and_truncate {
    ($c:expr, $role:expr, $($table:literal),+) => {$(
        let r = sqlx::query(concat!("DELETE FROM identity.", $table)).execute(&mut $c).await;
        assert_eq!(code(&r).as_deref(), Some("42501"), "{} DELETE {}: {:?}", $role, $table, r);
        let r = sqlx::query(concat!("TRUNCATE identity.", $table)).execute(&mut $c).await;
        assert_eq!(code(&r).as_deref(), Some("42501"), "{} TRUNCATE {}: {:?}", $role, $table, r);
    )+};
}

#[tokio::test]
async fn id_19_20_nothing_is_deleted_and_the_lead_is_permanent() {
    let (db, f) = migrate_and_seed("mi_s_lead").await;
    for role in [MOD, HOST] {
        let mut c = as_role(&db, role, Some(f.a)).await;
        refused_delete_and_truncate!(
            c,
            role,
            "installations",
            "projects",
            "teams",
            "workers",
            "team_members",
            "names"
        );
        // The lead column is not granted: 42501 for both roles (the trigger is the second line).
        let r = sqlx::query("UPDATE identity.projects SET lead_worker_id = $1::uuid")
            .bind(f.a_m1.to_string())
            .execute(&mut c)
            .await;
        assert_eq!(code(&r).as_deref(), Some("42501"), "{role}: {r:?}");
        // Retiring the lead by raw SQL: 23514 (the trigger), for both roles.
        let r = sqlx::query(
            "UPDATE identity.workers SET state = 'retired', retired_at = now() WHERE id = $1::uuid",
        )
        .bind(f.a_lead.to_string())
        .execute(&mut c)
        .await;
        assert_eq!(code(&r).as_deref(), Some("23514"), "{role}: {r:?}");
    }
    // A project whose lead row is missing fails AT COMMIT (the deferred key): 23503.
    let mut c = as_role(&db, HOST, None).await;
    let project = ProjectId::new(&SystemClock);
    let mut tx = c.begin().await.expect("tx");
    sqlx::query("INSERT INTO identity.projects (id, installation_id, lead_worker_id) VALUES ($1::uuid, $2::uuid, $3::uuid)")
        .bind(project.to_string())
        .bind(f.inst.to_string())
        .bind(WorkerId::new(&SystemClock).to_string())
        .execute(&mut *tx)
        .await
        .expect("the insert itself passes: the key is deferred");
    let r = tx.commit().await;
    assert_eq!(code(&r).as_deref(), Some("23503"), "{r:?}");
}

#[tokio::test]
async fn id_22_only_the_granted_columns_can_be_updated() {
    let (db, f) = migrate_and_seed("mi_s_cols").await;
    let expected: BTreeSet<(String, String)> = [
        ("installations", "label"),
        ("projects", "state"),
        ("projects", "token_lifetime_days"),
        ("teams", "state"),
        ("workers", "job_roles"),
        ("workers", "mode"),
        ("workers", "perm_gen"),
        ("workers", "retired_at"),
        ("workers", "retired_reason"),
        ("workers", "state"),
        ("workers", "token_path_hint"),
        ("team_members", "role"),
        ("team_members", "team_id"),
        ("names", "current"),
        ("names", "reserved"),
        ("names", "valid_to"),
    ]
    .iter()
    .map(|(t, c)| ((*t).to_owned(), (*c).to_owned()))
    .collect();
    let mut admin = db.admin().await;
    for role in [MOD, HOST] {
        let rows: Vec<(String, String)> = sqlx::query_as(
            "SELECT table_name::text, column_name::text FROM information_schema.column_privileges \
             WHERE table_schema = 'identity' AND grantee = $1 AND privilege_type = 'UPDATE'",
        )
        .bind(role)
        .fetch_all(&mut admin)
        .await
        .expect("column privileges");
        let got: BTreeSet<(String, String)> = rows.into_iter().collect();
        assert_eq!(got, expected, "{role}: the UPDATE column grants");
    }
    // And a few immutable columns are refused as raw UPDATEs (42501) for both roles.
    for role in [MOD, HOST] {
        let mut c = as_role(&db, role, Some(f.a)).await;
        for sql in [
            "UPDATE identity.workers SET id = gen_random_uuid() WHERE id = $1::uuid",
            "UPDATE identity.workers SET project_id = project_id WHERE id = $1::uuid",
            "UPDATE identity.workers SET kind = 'service' WHERE id = $1::uuid",
            "UPDATE identity.workers SET created_at = now() WHERE id = $1::uuid",
            "UPDATE identity.names SET name_key = 'zzz' WHERE entity_id = $1::uuid",
            "UPDATE identity.names SET label = 'Zzz' WHERE entity_id = $1::uuid",
            "UPDATE identity.team_members SET worker_id = worker_id WHERE worker_id = $1::uuid",
            "UPDATE identity.team_members SET since = now() WHERE worker_id = $1::uuid",
        ] {
            let r = sqlx::query(sql)
                .bind(f.a_m1.to_string())
                .execute(&mut c)
                .await;
            assert_eq!(code(&r).as_deref(), Some("42501"), "{role}: {sql}: {r:?}");
        }
    }
}

#[tokio::test]
async fn id_34_35_the_lead_stays_outside_teams_and_rows_stay_in_their_project() {
    let (db, f) = migrate_and_seed("mi_s_consistency").await;
    let mut c = as_role(&db, MOD, Some(f.a)).await;
    // The project lead cannot become a team member: 23514 (trigger).
    let r = sqlx::query("INSERT INTO identity.team_members (worker_id, team_id, project_id, role) VALUES ($1::uuid, $2::uuid, $3::uuid, 'member')")
        .bind(f.a_lead.to_string())
        .bind(f.t1.to_string())
        .bind(f.a.to_string())
        .execute(&mut c)
        .await;
    assert_eq!(code(&r).as_deref(), Some("23514"), "{r:?}");
    // Cross-project rows, written by the host role (which RLS lets write anywhere): 23503.
    let mut h = as_role(&db, HOST, None).await;
    let r = sqlx::query("INSERT INTO identity.team_members (worker_id, team_id, project_id, role) VALUES ($1::uuid, $2::uuid, $3::uuid, 'member')")
        .bind(f.b_worker.to_string())
        .bind(f.t1.to_string())
        .bind(f.a.to_string())
        .execute(&mut h)
        .await;
    assert_eq!(
        code(&r).as_deref(),
        Some("23503"),
        "worker of B, team of A: {r:?}"
    );
    let r = sqlx::query("INSERT INTO identity.team_members (worker_id, team_id, project_id, role) VALUES ($1::uuid, $2::uuid, $3::uuid, 'member')")
        .bind(f.a_m2.to_string())
        .bind(f.tb.to_string())
        .bind(f.a.to_string())
        .execute(&mut h)
        .await;
    assert_eq!(
        code(&r).as_deref(),
        Some("23503"),
        "worker of A, team of B: {r:?}"
    );
    // A names row whose project_id is not its entity's project: 23503, for a project, a team, a worker.
    for (kind, entity, scope) in [
        ("project", f.a.to_string(), f.inst.to_string()),
        ("team", f.t1.to_string(), f.a.to_string()),
        ("worker", f.a_m1.to_string(), f.a.to_string()),
    ] {
        let r = sqlx::query(
            "INSERT INTO identity.names (entity_kind, entity_id, scope_id, project_id, name_key, label, current) \
             VALUES ($1, $2::uuid, $3::uuid, $4::uuid, 'wrongproject', 'Wrongproject', true)",
        )
        .bind(kind)
        .bind(entity)
        .bind(scope)
        .bind(f.b.to_string())
        .execute(&mut h)
        .await;
        assert_eq!(
            code(&r).as_deref(),
            Some("23503"),
            "{kind} name with B's project id: {r:?}"
        );
    }
}
