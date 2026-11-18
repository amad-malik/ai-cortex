//! F1-31 category 5 (TESTS r7; A7 r4 item 2, `proconfig` `search_path=pg_catalog, <schema>,
//! pg_temp`). Same-name tables, types, functions and operators planted in `public` (by the
//! superuser) and in the calling session's `pg_temp` (by the module role, which this database
//! lets create temporary objects) change nothing the function does.
//! RED (b2): the template and the b1 core are `todo!()`.
use cortex_db::maintenance::MaintError;
use cortex_db::maintenance::templates;
use cortex_testkit::maint::{
    Maint, Mutation, SPEC_A, create, exec, leaf, maint, maint_with, month, refused, uuid, variant,
};
use cortex_testkit::within;
use cortex_types::ProjectId;

/// A project with list partitions under both roots but no `identity_pub` row: it does not exist.
async fn ghost(w: &Maint) -> ProjectId {
    let g = ProjectId::new(&*w.clock);
    let u = uuid(g);
    let part = templates::project_partition_name("p", &u);
    w.as_owner(&format!(
        "CREATE TABLE test_a.{part} PARTITION OF test_a.p FOR VALUES IN ('{u}') PARTITION BY RANGE (month);"
    ))
    .await;
    w.as_admin(&format!(
        "GRANT TEMPORARY ON DATABASE {} TO cortex_m_test_a",
        w.db.database
    ))
    .await;
    g
}

/// The calling session plants, in its own `pg_temp`, a `projects` table that calls the ghost open,
/// and a `p` table. GREEN (finn): both are readable by PUBLIC, so the definer could read them: the
/// qualified reference must ignore a readable plant, and the temp-first control can read it (a
/// module role's temp table is otherwise unreadable to `cortex_owner`, and the control saw 42501).
async fn plant_temp(tx: &mut cortex_db::ModuleTx, g: uuid::Uuid) {
    exec(
        tx,
        &format!(
            "CREATE TEMP TABLE projects (id uuid, state text); \
             INSERT INTO pg_temp.projects VALUES ('{g}', 'open'); \
             CREATE TEMP TABLE p (project_id uuid, month date, id bigint, v text); \
             GRANT SELECT ON pg_temp.projects, pg_temp.p TO PUBLIC;"
        ),
    )
    .await
    .expect("plant in pg_temp");
}

#[tokio::test]
async fn cat05_objects_planted_in_public_and_pg_temp_change_nothing() {
    within(180, async {
        let w = maint("c05").await;
        let g = ghost(&w).await;
        let gu = uuid(g);
        w.as_admin(&format!(
            "CREATE TABLE public.projects (id uuid, state text); \
             INSERT INTO public.projects VALUES ('{gu}', 'open'); \
             CREATE TABLE public.p (project_id uuid, month date, id bigint, v text); \
             CREATE TABLE public.maintenance_runs (run_id uuid); \
             CREATE DOMAIN public.uuid AS text; \
             CREATE FUNCTION public.now() RETURNS timestamptz LANGUAGE sql AS $$ SELECT '1900-01-01'::timestamptz $$; \
             CREATE FUNCTION public.date_trunc(text, date) RETURNS date LANGUAGE sql AS $$ SELECT '1900-01-01'::date $$; \
             CREATE FUNCTION public.always_true(uuid, uuid) RETURNS boolean LANGUAGE sql AS $$ SELECT true $$; \
             CREATE OPERATOR public.= (LEFTARG = uuid, RIGHTARG = uuid, FUNCTION = public.always_true); \
             GRANT USAGE ON SCHEMA public TO cortex_m_test_a;"
        ))
        .await;
        // The ghost as the transaction's own project: the planted rows must not authorise it.
        let mut tx = w.begin("test_a", Some(g)).await;
        plant_temp(&mut tx, gu).await;
        assert_eq!(
            create(&mut tx, "test_a", Some(gu), Some(&month(1))).await,
            Err(refused(MaintError::NotAuthorised)),
            "a planted projects table authorised a nonexistent project"
        );
        tx.rollback().await.ok();
        // A's own call acts on its own schema's objects, with the plants in place.
        let a = uuid(w.a);
        let mut tx = w.begin("test_a", Some(w.a)).await;
        plant_temp(&mut tx, gu).await;
        assert_eq!(
            create(&mut tx, "test_a", Some(a), Some(&month(1))).await,
            Ok("created".into())
        );
        tx.commit().await.expect("commit");
        assert_eq!(w.relkind("test_a", &leaf(&a, 1)).await.as_deref(), Some("r"));
        assert_eq!(w.relkind("public", &leaf(&a, 1)).await, None);
        let mut admin = w.db.admin().await;
        let planted: (i64, i64) = sqlx::query_as(
            "select (select count(*) from public.p), (select count(*) from public.maintenance_runs)",
        )
        .fetch_one(&mut admin)
        .await
        .expect("public plants");
        assert_eq!(planted, (0, 0), "the function wrote to a planted table");
    })
    .await
}

#[tokio::test]
async fn cat05_control_temp_first_and_unqualified_uses_the_pg_temp_table() {
    within(180, async {
        let base = templates::partition_maintenance(&SPEC_A);
        let v = variant(&base, &SPEC_A, Mutation::SearchPathTempFirst)
            .expect("the variant: its markers exist");
        let w = maint_with("c05ctl", v).await;
        let g = ghost(&w).await;
        let gu = uuid(g);
        let mut tx = w.begin("test_a", Some(g)).await;
        plant_temp(&mut tx, gu).await;
        assert_eq!(
            create(&mut tx, "test_a", Some(gu), Some(&month(1))).await,
            Ok("created".into()),
            "with pg_temp first and an unqualified reference, the planted table authorises the ghost"
        );
        tx.rollback().await.ok();
    })
    .await
}
