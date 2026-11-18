//! The `migrate` decision (contract r0.3 row `migrate`): the owner keys are REQUIRED, `platform` first then the enabled pieces in
//! the REGISTRY's order (given by the caller), the first failure stops the run and names the migration, nothing secret is printed. DB-free. RED first.

use async_trait::async_trait;
use cortex_config::{Config, Database, DbHost, DbOwner, ModuleList, Paths, Server};
use cortex_server::bootstrap::Outcome;
use cortex_server::cli::{EXIT_FAILURE, EXIT_OK};
use cortex_server::migrate::{Migrator, StepFailed, run};

fn order(modules: &[&str]) -> Vec<ModuleId> {
    modules
        .iter()
        .map(|m| ModuleId::parse(m).expect("module id"))
        .collect()
}
use cortex_types::ModuleId;
use std::path::PathBuf;

const OWNER_PW_FILE: &str = "/etc/cortex/db/owner.password-ZZ9";
const OWNER_USER: &str = "cortex_owner_zz9";

#[derive(Default)]
struct Fake {
    applied: Vec<String>,
    fail_at: Option<(String, u32)>,
    steps: u32,
}

#[async_trait]
impl Migrator for Fake {
    async fn apply(&mut self, module: &str) -> Result<u32, StepFailed> {
        self.applied.push(module.to_string());
        if let Some((m, v)) = &self.fail_at
            && m == module
        {
            return Err(StepFailed(*v));
        }
        Ok(self.steps)
    }
}

fn cfg(database: Option<Database>, enabled: &[&str]) -> Config {
    Config {
        paths: Paths {
            state_dir: PathBuf::from("/var/lib/cortex"),
            host_socket: PathBuf::from("/run/cortex/admin.sock"),
            worker_socket: PathBuf::from("/run/cortex/api.sock"),
            token_dir: PathBuf::from("/home/x/.config/cortex"),
        },
        modules: ModuleList {
            enabled: enabled
                .iter()
                .map(|m| ModuleId::parse(m).expect("module id"))
                .collect(),
        },
        database,
        server: Server::default(),
    }
}

fn db(owner: bool) -> Database {
    Database {
        host: DbHost::SocketDir(PathBuf::from("/run/postgresql")),
        port: 5432,
        name: "cortex".into(),
        app_user: "cortex_app".into(),
        app_password_file: PathBuf::from("/etc/cortex/db/app.password"),
        owner: owner.then(|| DbOwner {
            user: OWNER_USER.into(),
            password_file: PathBuf::from(OWNER_PW_FILE),
        }),
    }
}

fn clean(o: &Outcome) {
    let all = format!("{o:?}");
    assert!(
        !all.contains(OWNER_PW_FILE) && !all.contains(OWNER_USER),
        "no owner detail may be printed: {all}"
    );
}

#[tokio::test]
async fn mg_01_no_database_table_stops_before_anything_is_applied() {
    let mut f = Fake::default();
    let o = run(
        &cfg(None, &["identity"]),
        &order(&["platform", "identity"]),
        &mut f,
    )
    .await;
    assert_eq!(o.code, EXIT_FAILURE);
    assert!(
        o.stderr.as_deref().is_some_and(|s| s.contains("database")),
        "{o:?}"
    );
    assert!(f.applied.is_empty());
}

#[tokio::test]
async fn mg_02_migrate_requires_the_owner_keys_and_names_the_missing_one() {
    let mut f = Fake::default();
    let o = run(
        &cfg(Some(db(false)), &["identity"]),
        &order(&["platform", "identity"]),
        &mut f,
    )
    .await;
    assert_eq!(o.code, EXIT_FAILURE);
    assert!(
        o.stderr
            .as_deref()
            .is_some_and(|s| s.contains("database.owner_user")),
        "{o:?}"
    );
    assert!(
        f.applied.is_empty(),
        "nothing may run without the owner role"
    );
}

#[tokio::test]
async fn mg_03_the_modules_are_applied_in_the_order_given_platform_first() {
    let mut f = Fake {
        steps: 2,
        ..Default::default()
    };
    let o = run(
        &cfg(Some(db(true)), &["tokens", "identity", "audit"]),
        &order(&["platform", "tokens", "identity", "audit"]),
        &mut f,
    )
    .await;
    assert_eq!((o.code, o.stderr.clone()), (EXIT_OK, None));
    assert_eq!(f.applied, ["platform", "tokens", "identity", "audit"]);
    clean(&o);
}

#[tokio::test]
async fn mg_04_a_current_schema_is_a_no_op_that_still_succeeds() {
    let mut f = Fake {
        steps: 0,
        ..Default::default()
    };
    let o = run(
        &cfg(Some(db(true)), &["identity"]),
        &order(&["platform", "identity"]),
        &mut f,
    )
    .await;
    assert_eq!((o.code, o.stderr.clone()), (EXIT_OK, None));
    assert_eq!(
        f.applied,
        ["platform", "identity"],
        "it still asks each set; the set itself applies nothing"
    );
}

#[tokio::test]
async fn mg_05_the_first_failure_stops_the_run_and_names_module_and_version() {
    let mut f = Fake {
        fail_at: Some(("identity".into(), 7)),
        ..Default::default()
    };
    let o = run(
        &cfg(Some(db(true)), &["tokens", "identity", "audit"]),
        &order(&["platform", "tokens", "identity", "audit"]),
        &mut f,
    )
    .await;
    assert_eq!(o.code, EXIT_FAILURE);
    let s = o.stderr.clone().expect("a message");
    assert!(s.contains("identity") && s.contains('7'), "{s}");
    assert_eq!(
        f.applied,
        ["platform", "tokens", "identity"],
        "audit must not run after the failure"
    );
    assert!(!s.contains('\n'), "one line");
    clean(&o);
}

#[tokio::test]
async fn mg_06_a_platform_failure_stops_everything() {
    let mut f = Fake {
        fail_at: Some(("platform".into(), 1)),
        ..Default::default()
    };
    let o = run(
        &cfg(Some(db(true)), &["identity"]),
        &order(&["platform", "identity"]),
        &mut f,
    )
    .await;
    assert_eq!(o.code, EXIT_FAILURE);
    assert!(o.stderr.as_deref().is_some_and(|s| s.contains("platform")));
    assert_eq!(f.applied, ["platform"]);
}

#[tokio::test]
async fn mg_07_no_enabled_piece_still_applies_platform() {
    let mut f = Fake {
        steps: 1,
        ..Default::default()
    };
    let o = run(&cfg(Some(db(true)), &[]), &order(&["platform"]), &mut f).await;
    assert_eq!(o.code, EXIT_OK);
    assert_eq!(f.applied, ["platform"]);
}

#[tokio::test]
async fn mg_08_the_registry_order_wins_over_the_config_order() {
    // hale N-2: cortex-config accepts any order; S2 needs identity before tokens. The CALLER passes the registry's migration order.
    let mut f = Fake {
        steps: 1,
        ..Default::default()
    };
    let o = run(
        &cfg(Some(db(true)), &["tokens", "identity"]),
        &order(&["platform", "identity", "tokens"]),
        &mut f,
    )
    .await;
    assert_eq!(o.code, EXIT_OK);
    assert_eq!(
        f.applied,
        ["platform", "identity", "tokens"],
        "not the config's order"
    );
}

#[tokio::test]
async fn mg_09_an_order_that_does_not_match_the_enabled_pieces_is_refused_before_anything_runs() {
    let enabled = ["identity", "tokens"];
    let bad: Vec<(&str, Vec<ModuleId>)> = vec![
        ("empty", order(&[])),
        (
            "platform not first",
            order(&["identity", "platform", "tokens"]),
        ),
        ("no platform", order(&["identity", "tokens"])),
        (
            "a duplicate",
            order(&["platform", "identity", "identity", "tokens"]),
        ),
        (
            "a module that is not enabled",
            order(&["platform", "identity", "tokens", "audit"]),
        ),
        (
            "an enabled module missing",
            order(&["platform", "identity"]),
        ),
        (
            "platform twice",
            order(&["platform", "platform", "identity", "tokens"]),
        ),
        (
            "the first is not platform though the rest matches",
            order(&["audit", "identity", "tokens"]),
        ),
    ];
    for (why, o) in bad {
        let mut f = Fake::default();
        let out = run(&cfg(Some(db(true)), &enabled), &o, &mut f).await;
        assert_eq!(out.code, EXIT_FAILURE, "{why}");
        assert!(
            out.stderr.as_deref().is_some_and(|s| s.contains("order")),
            "{why}: {out:?}"
        );
        assert!(
            f.applied.is_empty(),
            "{why}: nothing may run on a bad order"
        );
    }
}

#[tokio::test]
async fn mg_10_platform_cannot_be_an_enabled_piece_and_apply_twice() {
    // A config that lists `platform` as an enabled piece must not make the order [platform, platform, ...] acceptable.
    let mut f = Fake::default();
    let out = run(
        &cfg(Some(db(true)), &["platform", "identity"]),
        &order(&["platform", "platform", "identity"]),
        &mut f,
    )
    .await;
    assert_eq!(out.code, EXIT_FAILURE);
    assert!(f.applied.is_empty(), "nothing may run");
}
