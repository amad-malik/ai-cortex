//! cox's row 6dfc0225 and kai's 7ba01ce1: the test gate failed on a clean archive of 3c35f0f with
//! `mod-identity :: h_assign :: id_31_…: platform: Db(Other("sqlstate P0001"))` at
//! `tests/common/seed.rs:162`, the `.expect` of `apply(&mut owner, &platform_migrations())`.
//!
//! The only default-errcode RAISE of the platform migration is its grants read-back (the last DO block of
//! `crates/cortex-db/migrations/platform/0001_platform.up.sql`, 'platform grants are not exactly the allowed
//! set'). It compares the grants of the platform schema with a set built from EVERY role of the CLUSTER named
//! `cortex_m_*`; the GRANT loop before it read the same roles earlier in the same transaction. A `cortex_m_*`
//! role created, and committed, by ANOTHER test between the two reads is a role the loop never granted to: the
//! read-back raises. h_assign runs seven tests in parallel threads against one cluster, and the chain fixture
//! (`mod-identity/tests/common/chain.rs::start`) creates `cortex_m_plug_audit` AFTER its own platform
//! migration, so the FIRST test to get there lands in a sibling's window. Narrow, intermittent, and only at the
//! start of a package's run (afterwards the role exists, `CREATE ROLE` is a duplicate and nothing changes).
//!
//! The migration's read-back is a security check and stays exactly as it is. The repair is in the kit: every
//! `cortex_m_*` role a fixture of this workspace creates is created by `empty_db`, under the existing
//! cluster-wide role-setup lock, BEFORE any test of the cluster migrates, so no fixture creates a module role
//! while another test's platform migration runs.
//!
//! Needs `supervise.sh`'s throwaway Postgres. The two cells that migrate the platform are ONE test on purpose:
//! two of them in parallel would be the very race (a role created by one inside the other's window).
use cortex_db::OwnerConn;
use cortex_db::migrate::{apply, platform_migrations};
use cortex_testkit::empty_db;
use std::path::{Path, PathBuf};

/// A superuser-owned event trigger, installed in the test's own database. After the GRANT that is the LAST of
/// the platform migration's grants (`cortex_m_platform` may SELECT `platform.migrations`), and BEFORE the
/// read-back, it creates `role`: the exact effect of another test's bootstrap committing in that window.
/// (An idempotent CREATE ROLE, the bootstrap form: an existing role is fine.)
fn window_trigger(role: &str) -> String {
    format!(
        r#"
CREATE FUNCTION public.pr_window() RETURNS event_trigger LANGUAGE plpgsql SECURITY DEFINER AS $fn$
BEGIN
    IF to_regclass('platform.migrations') IS NOT NULL
       AND has_table_privilege('cortex_m_platform', 'platform.migrations', 'SELECT') THEN
        BEGIN
            CREATE ROLE {role} NOLOGIN;
        EXCEPTION WHEN duplicate_object OR unique_violation THEN NULL;
        END;
    END IF;
END
$fn$;
CREATE EVENT TRIGGER pr_window ON ddl_command_end WHEN TAG IN ('GRANT') EXECUTE FUNCTION public.pr_window();
"#
    )
}

/// The platform migration on an empty database of its own, with `role` created inside its window.
async fn migrate_with_a_role_created_in_the_window(tag: &str, role: &str) -> Result<(), String> {
    let db = empty_db(tag).await;
    let mut admin = db.admin().await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(window_trigger(role)))
        .execute(&mut admin)
        .await
        .expect("the window trigger");
    let mut owner = OwnerConn::connect(&db.owner_config()).await.expect("owner");
    apply(&mut owner, &platform_migrations())
        .await
        .map(|_| ())
        .map_err(|e| format!("{e:?}"))
}

/// One test: see the header. First the CONTROL (the read-back is exactly as sensitive as it was: a role that is
/// no piece role, created in the window, makes it raise), then the cell: the role a fixture creates after its
/// own platform migration (`cortex_m_plug_audit`, mod-identity's chain fixture and mod-tokens') does not.
#[tokio::test]
async fn pr_01_a_piece_role_created_inside_a_platform_migration_does_not_break_its_read_back() {
    let control =
        migrate_with_a_role_created_in_the_window("pr_control", "cortex_m_zz_window").await;
    let why = control.expect_err(
        "the control: a role the migration's loop never saw, created before the read-back, must make the read-back raise",
    );
    assert!(
        why.contains("P0001"),
        "the control raises the read-back's P0001, not something else: {why}"
    );
    // the cell: the role the chain fixture creates right after its own migration
    let cell = migrate_with_a_role_created_in_the_window("pr_window", "cortex_m_plug_audit").await;
    assert_eq!(
        cell,
        Ok(()),
        "a piece role that another test's bootstrap creates inside the window must already exist"
    );
}

/// Every `.rs` file under `dir`, recursively (no symlink is followed; the workspace has none under src and tests).
fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        let Ok(t) = e.file_type() else { continue };
        if t.is_dir() {
            rust_files(&p, out);
        } else if t.is_file() && p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
}

/// The `cortex_m_<name>` roles the sources of this workspace create: after `CREATE ROLE ` and as the literal
/// argument of the cortex-http fixture macro `bootstrap_sql!("…")`. A name built at run time
/// (`cortex_m_{id}`, cortex-server's serve_real) has no literal name and is not seen here.
fn roles_the_workspace_creates() -> Vec<String> {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the crates directory");
    let me = Path::new(file!())
        .file_name()
        .expect("this file's name")
        .to_owned();
    let mut files = Vec::new();
    for c in std::fs::read_dir(crates).expect("crates").flatten() {
        for sub in ["src", "tests"] {
            rust_files(&c.path().join(sub), &mut files);
        }
    }
    let mut names = std::collections::BTreeSet::new();
    for f in files {
        // this file names roles of its own in its fixtures
        if f.file_name() == Some(&me) {
            continue;
        }
        let text = std::fs::read_to_string(&f).expect("a source file");
        names.extend(role_names_in(&text));
    }
    names.into_iter().collect()
}

/// The `cortex_m_<name>` roles one source text creates (see [`roles_the_workspace_creates`]).
fn role_names_in(text: &str) -> std::collections::BTreeSet<String> {
    let mut names = std::collections::BTreeSet::new();
    for lead in ["CREATE ROLE cortex_m_", "bootstrap_sql!(\"cortex_m_"] {
        for (at, _) in text.match_indices(lead) {
            let name: String = text[at + lead.len() - "cortex_m_".len()..]
                .chars()
                .take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_')
                .collect();
            if name.len() > "cortex_m_".len() {
                names.insert(name);
            }
        }
    }
    names
}

/// The roles that a test CREATES ON PURPOSE, to prove how a bootstrap behaves when it meets a role that
/// is being created at the same moment (mod-audit's `audit_bootstrap_race.rs` asserts that its control role "must not already exist").
const CREATED_ON_PURPOSE: &[&str] = &["cortex_m_audit_race"];

/// The names are the workspace's own, found in its sources, so a new piece or fixture whose role is not in the
/// kit's list fails HERE, by name, and not as a flake of somebody else's test.
#[tokio::test]
async fn pr_02_every_module_role_a_fixture_of_the_workspace_creates_exists_once_empty_db_returned()
{
    let wanted: Vec<String> = roles_the_workspace_creates()
        .into_iter()
        .filter(|n| !CREATED_ON_PURPOSE.contains(&n.as_str()))
        .collect();
    // not vacuous: the sources of the pieces name at least these (read from the sources above)
    for known in [
        "cortex_m_identity",
        "cortex_m_plug_audit",
        "cortex_m_tokens",
        "cortex_m_records",
        "cortex_m_search",
    ] {
        assert!(
            wanted.iter().any(|n| n == known),
            "the scan of the sources finds {known}: {wanted:?}"
        );
    }
    let db = empty_db("pr_roles").await;
    let mut admin = db.admin().await;
    let have: Vec<String> =
        sqlx::query_scalar("SELECT rolname::text FROM pg_roles WHERE rolname = ANY($1)")
            .bind(&wanted)
            .fetch_all(&mut admin)
            .await
            .expect("pg_roles");
    let missing: Vec<&String> = wanted.iter().filter(|n| !have.contains(n)).collect();
    assert!(
        missing.is_empty(),
        "empty_db must create these module roles (add them to PIECE_ROLES in crates/cortex-testkit/src/pg.rs): {missing:?}"
    );
}

/// The `cortex_m_<id>` role of every piece crate: a `mod-*` or `plug-*` crate's `pub const ID: &str = "<id>";`. cortex-server's
/// `serve_real` creates one role per COMPILED piece at run time (`cortex_m_{id}`): a name no scan for literals sees, and a feature
/// that the test build may not enable, so the pieces' own constants are read here, without a database and without a feature.
fn piece_ids_of_the_workspace() -> Vec<String> {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the crates directory");
    let mut ids = std::collections::BTreeSet::new();
    for c in std::fs::read_dir(crates).expect("crates").flatten() {
        let name = c.file_name().to_string_lossy().into_owned();
        if !(name.starts_with("mod-") || name.starts_with("plug-")) {
            continue;
        }
        let mut files = Vec::new();
        rust_files(&c.path().join("src"), &mut files);
        for f in files {
            let text = std::fs::read_to_string(&f).expect("a source file");
            for line in text.lines() {
                if let Some(rest) = line.trim().strip_prefix("pub const ID: &str = \"")
                    && let Some((id, _)) = rest.split_once('"')
                {
                    ids.insert(id.to_owned());
                }
            }
        }
    }
    ids.into_iter().collect()
}

/// The kit's list is checked against the workspace without a database: every piece id, every scanned fixture role, the migration's own
/// name pattern, no duplicate, and nothing a test creates on purpose. A new piece (or fixture) whose role is not in the list fails HERE, by
/// name, and not as a flake of somebody else's platform migration.
#[test]
fn pr_03_the_kits_list_names_every_piece_role_and_nothing_made_on_purpose() {
    let listed: Vec<&str> = cortex_testkit::PIECE_ROLES.to_vec();
    let ids = piece_ids_of_the_workspace();
    // not vacuous: the pieces of this workspace (read from their sources above)
    for known in [
        "identity",
        "records",
        "search",
        "tokens",
        "handoffs",
        "boot",
        "policy",
        "plug_identity",
    ] {
        assert!(
            ids.iter().any(|i| i == known),
            "the scan of the pieces' ID constants finds {known}: {ids:?}"
        );
    }
    let mut wanted: Vec<String> = ids.iter().map(|i| format!("cortex_m_{i}")).collect();
    wanted.extend(
        roles_the_workspace_creates()
            .into_iter()
            .filter(|n| !CREATED_ON_PURPOSE.contains(&n.as_str())),
    );
    // the roles the kit's own ROLES block already creates (cortex_m_platform, cortex_m_test_*) need no entry in the list
    let kit_roles = role_names_in(
        &std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("src/pg.rs"))
            .expect("the kit's pg.rs"),
    );
    assert!(
        kit_roles.contains("cortex_m_platform") && kit_roles.contains("cortex_m_test_a"),
        "the scan of the kit's own ROLES block finds its roles: {kit_roles:?}"
    );
    wanted.retain(|n| !kit_roles.contains(n));
    wanted.sort();
    wanted.dedup();
    let missing: Vec<&String> = wanted
        .iter()
        .filter(|n| !listed.contains(&n.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "PIECE_ROLES (crates/cortex-testkit/src/pg.rs) lacks these module roles: {missing:?}"
    );
    for name in &listed {
        let body = name.strip_prefix("cortex_m_");
        assert!(
            body.is_some_and(|b| !b.is_empty()
                && b.len() <= 32
                && b.starts_with(|c: char| c.is_ascii_lowercase())
                && b.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')),
            "{name} matches the pattern of the platform migration's modules CTE (^cortex_m_[a-z][a-z0-9_]{{0,31}}$)"
        );
        assert!(
            !CREATED_ON_PURPOSE.contains(name),
            "{name} is created on purpose by a test and must NOT be precreated"
        );
    }
    let mut sorted = listed.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), listed.len(), "PIECE_ROLES has no duplicate");
}

/// What the kit creates must be what the pieces' own bootstraps create: plain `NOLOGIN` roles with no privilege (a role of this list that could
/// log in, or had `SUPERUSER`, `BYPASSRLS`, `CREATEROLE`, `CREATEDB` or `REPLICATION`, would be a hole in every test cluster, and the list is
/// the one place that makes the roles).
#[tokio::test]
async fn pr_04_the_precreated_piece_roles_are_plain_nologin_roles() {
    assert!(
        !cortex_testkit::PIECE_ROLES.is_empty(),
        "the kit's list of piece roles is not empty"
    );
    let names: Vec<String> = cortex_testkit::PIECE_ROLES
        .iter()
        .map(|n| (*n).to_owned())
        .collect();
    let db = empty_db("pr_plain").await;
    let mut admin = db.admin().await;
    let rows: Vec<(String, bool, bool, bool, bool, bool, bool)> = sqlx::query_as(
        "SELECT rolname::text, rolcanlogin, rolsuper, rolbypassrls, rolcreaterole, rolcreatedb, rolreplication \
         FROM pg_roles WHERE rolname = ANY($1) ORDER BY rolname",
    )
    .bind(&names)
    .fetch_all(&mut admin)
    .await
    .expect("pg_roles");
    assert_eq!(
        rows.len(),
        names.len(),
        "every listed role exists, once: {rows:?}"
    );
    let odd: Vec<&String> = rows
        .iter()
        .filter(|r| r.1 || r.2 || r.3 || r.4 || r.5 || r.6)
        .map(|r| &r.0)
        .collect();
    assert!(
        odd.is_empty(),
        "these roles can log in or carry a privilege; a piece role is NOLOGIN and plain: {odd:?}"
    );
}
