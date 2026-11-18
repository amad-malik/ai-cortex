//! The assembly shared by `serve`, `migrate` and `bootstrap` (piece 11, mira; cox 15:2xZ). DB-free. RED first.
//! Fixture pieces stand in for the real five, which cannot assemble yet (`Port::Audit` has no provider until mod-audit merges).
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use cortex_config::{Env, load};
use cortex_db::{DbEndpoint, MigrationSet};
use cortex_module::{
    BoxFuture, CONTRACT_VERSION, DefaultRole, Health, Manifest, Piece, Ports, PortsBuilder,
    ProblemSpec, Routes, ScopeClass, ScopeDecl, TargetRule,
};
use cortex_server::assembly::{
    DbConfigError, POOL_SIZE, assemble_with, compiled, db_config, duplicate_enabled,
    ordered_modules, startup_line,
};
use cortex_types::{ModuleId, ProblemCodeMinter, SystemClock};

fn minter() -> &'static ProblemCodeMinter {
    static M: OnceLock<ProblemCodeMinter> = OnceLock::new();
    M.get_or_init(|| ProblemCodeMinter::take().expect("the first take in this process"))
}

// ---- fixture pieces -------------------------------------------------------------------------

static AUTH: Manifest = Manifest {
    id: "fx_auth",
    version: "0.1.0",
    contract: CONTRACT_VERSION,
    core: true,
    schema: "fx_auth",
    provides: &[],
    uses: &[],
    scopes: &[],
    problems: &[],
};

const ROLES: &[DefaultRole] = &[
    DefaultRole::ProjectLead,
    DefaultRole::TeamLead,
    DefaultRole::Member,
];

static THING: Manifest = Manifest {
    id: "fx_thing",
    version: "0.1.0",
    contract: CONTRACT_VERSION,
    core: false,
    schema: "fx_thing",
    provides: &[],
    uses: &[],
    scopes: &[ScopeDecl {
        name: "fx:read",
        class: ScopeClass::Work,
        default_roles: ROLES,
    }],
    problems: &[],
};

static OTHER: Manifest = Manifest {
    id: "fx_other",
    version: "0.1.0",
    contract: CONTRACT_VERSION,
    core: false,
    schema: "fx_other",
    provides: &[],
    uses: &[],
    scopes: &[ScopeDecl {
        name: "fxo:read",
        class: ScopeClass::Work,
        default_roles: ROLES,
    }],
    problems: &[],
};

/// A piece that declares a problem code: `internal_error` is the chain's own, so it must be refused; the control declares its own.
static CLASH: Manifest = Manifest {
    id: "fx_clash",
    version: "0.1.0",
    contract: CONTRACT_VERSION,
    core: false,
    schema: "fx_clash",
    provides: &[],
    uses: &[],
    scopes: &[],
    problems: &[ProblemSpec {
        code: "internal_error",
        status: 500,
        title: "Internal error",
        fix: "Retry later.",
    }],
};

static OWN_CODE: Manifest = Manifest {
    id: "fx_owncode",
    version: "0.1.0",
    contract: CONTRACT_VERSION,
    core: false,
    schema: "fx_owncode",
    provides: &[],
    uses: &[],
    scopes: &[],
    problems: &[ProblemSpec {
        code: "fx_own_code",
        status: 409,
        title: "Own code",
        fix: "Do something else.",
    }],
};

struct Fx {
    manifest: &'static Manifest,
    path: Option<&'static str>,
    scope: &'static str,
}

impl Piece for Fx {
    fn manifest(&self) -> &'static Manifest {
        self.manifest
    }
    fn migrations(&self) -> MigrationSet {
        MigrationSet {
            module: ModuleId::parse(self.manifest.id).expect("a module id"),
            schema: self.manifest.schema,
            steps: vec![],
        }
    }
    fn provide(&self, _ports: &mut PortsBuilder) {}
    fn routes(&self, _ports: &Ports) -> Routes {
        match self.path {
            Some(path) => Routes::new().read(
                axum::http::Method::GET,
                path,
                self.scope,
                TargetRule::None,
                || async { "ok" },
            ),
            None => Routes::new(),
        }
    }
    fn health(&self) -> BoxFuture<'_, Health> {
        Box::pin(async { Health::Ready })
    }
}

fn auth() -> Box<dyn Piece> {
    Box::new(Fx {
        manifest: &AUTH,
        path: None,
        scope: "",
    })
}
fn thing() -> Box<dyn Piece> {
    Box::new(Fx {
        manifest: &THING,
        path: Some("/v2/fx/thing"),
        scope: "fx:read",
    })
}
fn other() -> Box<dyn Piece> {
    Box::new(Fx {
        manifest: &OTHER,
        path: Some("/v2/fx/other"),
        scope: "fxo:read",
    })
}

fn ids(v: &[&str]) -> Vec<ModuleId> {
    v.iter()
        .map(|s| ModuleId::parse(s).expect("a module id"))
        .collect()
}

fn id(s: &str) -> ModuleId {
    ModuleId::parse(s).expect("a module id")
}

// ---- the enabled list -----------------------------------------------------------------------

#[test]
fn as1_a_module_enabled_twice_is_found_and_named() {
    assert_eq!(duplicate_enabled(&ids(&["fx_auth", "fx_thing"])), None);
    assert_eq!(duplicate_enabled(&[]), None);
    let twice = ids(&["fx_thing", "fx_auth", "fx_thing"]);
    assert_eq!(duplicate_enabled(&twice), Some(&twice[0]));
    // The first id seen a second time.
    let many = ids(&["a_a", "b_b", "b_b", "a_a"]);
    assert_eq!(duplicate_enabled(&many), Some(&many[1]));
}

// ---- assembly -------------------------------------------------------------------------------

#[test]
fn as2_startup_errors_come_back_raw_and_join_on_one_line_naming_modules_only() {
    // A core piece not enabled.
    let errs = assemble_with(vec![auth(), thing()], &[], minter()).expect_err("refused");
    assert!(!errs.is_empty());
    let line = startup_line(&errs);
    assert!(line.contains("fx_auth"), "{line}");
    assert!(!line.contains('\n'), "one line: {line:?}");
    // An enabled id that is not compiled in, and a second error: both on the one line.
    let errs = assemble_with(
        vec![auth(), thing()],
        &ids(&["fx_missing", "fx_also_missing"]),
        minter(),
    )
    .expect_err("refused");
    let line = startup_line(&errs);
    assert!(
        line.contains("fx_missing") && line.contains("fx_also_missing"),
        "{line}"
    );
    assert!(line.contains("; "), "errors are joined with '; ': {line}");
    assert!(!line.contains('\n'));
    assert_eq!(startup_line(&[]), "");
}

#[test]
fn as3_only_the_enabled_pieces_are_wired() {
    let reg = assemble_with(
        vec![auth(), thing(), other()],
        &ids(&["fx_auth", "fx_thing"]),
        minter(),
    )
    .expect("assembles");
    let wired: Vec<&str> = reg.pieces().map(|p| p.manifest().id).collect();
    assert_eq!(wired, ["fx_auth", "fx_thing"]);
    let paths: Vec<&str> = reg
        .routes()
        .flat_map(|(_, r)| r.entries().map(|e| e.path).collect::<Vec<_>>())
        .collect();
    assert_eq!(
        paths,
        ["/v2/fx/thing"],
        "the disabled piece's route is not mounted"
    );
}

#[test]
fn as4_the_migration_order_is_platform_then_the_compiled_order_whatever_the_config_says() {
    let want = ids(&["platform", "fx_auth", "fx_thing"]);
    for enabled in [&["fx_auth", "fx_thing"][..], &["fx_thing", "fx_auth"][..]] {
        let reg = assemble_with(vec![auth(), thing(), other()], &ids(enabled), minter())
            .expect("assembles");
        assert_eq!(ordered_modules(&reg), want, "enabled = {enabled:?}");
        assert_eq!(ordered_modules(&reg), reg.db_modules());
    }
}

// ---- the pool's configuration ---------------------------------------------------------------

fn dir(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("asm-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).expect("dir");
    d
}

fn config(name: &str, database: Option<(&str, &PathBuf)>) -> cortex_config::Config {
    let d = dir(name);
    let mut text = String::from("[modules]\nenabled = [\"fx_auth\", \"fx_thing\"]\n");
    if let Some((host, pw)) = database {
        text += &format!(
            "[database]\nhost = \"{host}\"\nport = 5432\nname = \"cortex\"\napp_user = \"cortex_app\"\napp_password_file = \"{}\"\n",
            pw.display()
        );
    }
    let f = d.join("cortex.toml");
    fs::write(&f, text).expect("config");
    load(&f, &Env::from_pairs(&[("HOME", "/home/x")])).expect("a valid config")
}

fn password(name: &str, bytes: &str) -> PathBuf {
    let f = dir(name).join("app.pw");
    fs::write(&f, bytes).expect("password file");
    f
}

fn registry() -> cortex_module::Registry {
    assemble_with(
        vec![auth(), thing(), other()],
        &ids(&["fx_auth", "fx_thing"]),
        minter(),
    )
    .expect("assembles")
}

#[test]
fn as5_the_pool_config_comes_from_the_config_the_password_file_and_the_assembled_registry() {
    let pw = password("pw-tcp", "canary-pw-91\n");
    let cfg = config("tcp", Some(("db", &pw)));
    let reg = registry();
    let c = db_config(&cfg, &reg, Arc::new(SystemClock)).expect("a pool config");
    assert!(
        matches!(&c.endpoint, DbEndpoint::Tcp { host, port } if host == "db" && *port == 5432),
        "{:?}",
        c.endpoint
    );
    assert_eq!(c.database, "cortex");
    assert_eq!(c.app_user, "cortex_app");
    assert_eq!(
        c.app_password.expose(),
        "canary-pw-91",
        "the trailing newline is not part of the password"
    );
    assert_eq!(c.max_connections, POOL_SIZE);
    assert!(c.statement_cache, "the statement cache is on in production");
    // The module registry is the ASSEMBLED registry's: platform, the enabled pieces, not the disabled one.
    for m in ["platform", "fx_auth", "fx_thing"] {
        assert!(c.registry.contains(&id(m)), "{m}");
    }
    assert!(
        !c.registry.contains(&id("fx_other")),
        "a disabled piece has no role"
    );

    // A host that is an absolute path is the Unix socket directory.
    let sock = config("sock", Some(("/run/postgresql", &pw)));
    let c = db_config(&sock, &reg, Arc::new(SystemClock)).expect("a pool config");
    assert!(
        matches!(&c.endpoint, DbEndpoint::Socket { dir, port } if dir.to_str() == Some("/run/postgresql") && *port == 5432),
        "{:?}",
        c.endpoint
    );

    // Only the final newline goes: a password may hold spaces.
    let spaced = password("pw-space", " pw with spaces \r\n");
    let c = db_config(
        &config("space", Some(("db", &spaced))),
        &reg,
        Arc::new(SystemClock),
    )
    .expect("a pool config");
    assert_eq!(c.app_password.expose(), " pw with spaces ");
}

#[test]
fn as6_a_missing_database_or_an_unreadable_password_is_refused_without_naming_a_path_or_a_value() {
    let reg = registry();
    let none = config("nodb", None);
    let e = db_config(&none, &reg, Arc::new(SystemClock))
        .err()
        .expect("refused");
    assert_eq!(e, DbConfigError::NoDatabaseTable);
    assert!(!e.to_string().contains('\n') && !e.to_string().is_empty());

    let missing = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("asm-no-such-dir/canary-secret-path/app.pw");
    let e = db_config(
        &config("nofile", Some(("db", &missing))),
        &reg,
        Arc::new(SystemClock),
    )
    .err()
    .expect("refused");
    assert_eq!(e, DbConfigError::PasswordUnreadable);
    let text = e.to_string();
    assert!(
        !text.is_empty() && !text.contains('\n'),
        "one line of text: {text:?}"
    );
    assert!(
        !text.contains("canary") && !text.contains("asm-no-such-dir"),
        "no path in: {text}"
    );

    // An empty file is not a password.
    let empty = password("pw-empty", "");
    let e = db_config(
        &config("empty", Some(("db", &empty))),
        &reg,
        Arc::new(SystemClock),
    )
    .err()
    .expect("refused");
    assert_eq!(e, DbConfigError::PasswordUnreadable);
    let only_newline = password("pw-nl", "\n");
    assert_eq!(
        db_config(
            &config("nl", Some(("db", &only_newline))),
            &reg,
            Arc::new(SystemClock)
        )
        .err(),
        Some(DbConfigError::PasswordUnreadable)
    );
}

// ---- the compile-time list ------------------------------------------------------------------

#[test]
fn as7_the_compiled_list_is_exactly_the_features_in_migration_order() {
    // audit, identity and tokens are core and always compiled in: AUDIT BEFORE identity (identity writes through the audit sink), identity
    // BEFORE tokens (tokens uses identity). The others follow their features; search comes after records (it reads the records views).
    let mut want: Vec<&str> = vec!["audit", "identity", "tokens"];
    if cfg!(feature = "mod-policy") {
        want.push("policy");
    }
    if cfg!(feature = "mod-records") {
        want.push("records");
    }
    if cfg!(feature = "mod-search") {
        want.push("search");
    }
    if cfg!(feature = "mod-handoffs") {
        want.push("handoffs");
    }
    if cfg!(feature = "mod-boot") {
        want.push("boot");
    }
    let got: Vec<&str> = compiled().iter().map(|p| p.manifest().id).collect();
    assert_eq!(got, want, "one cfg line per piece, in migration order");
}

// ---- the chain's own problem codes ----------------------------------------------------------

fn with_manifest(m: &'static Manifest) -> Box<dyn Piece> {
    Box::new(Fx {
        manifest: m,
        path: None,
        scope: "",
    })
}

#[test]
fn as8_a_piece_may_not_declare_a_code_the_chain_mints_for_itself() {
    let errs = assemble_with(
        vec![auth(), with_manifest(&CLASH)],
        &ids(&["fx_auth", "fx_clash"]),
        minter(),
    )
    .expect_err("refused");
    assert!(
        startup_line(&errs).contains("internal_error"),
        "{}",
        startup_line(&errs)
    );
    // The control: a code of its own is fine (so the refusal above is about the clash, not about declaring codes).
    assemble_with(
        vec![auth(), with_manifest(&OWN_CODE)],
        &ids(&["fx_auth", "fx_owncode"]),
        minter(),
    )
    .expect("a piece with a code of its own assembles");
}

// ---- the app password is read with cox's strict secret-file reader (`secretfile::read_secret_file`) ----------

fn password_bytes(name: &str, bytes: &[u8]) -> PathBuf {
    let f = dir(name).join("app.pw");
    fs::write(&f, bytes).expect("password file");
    f
}

/// What `db_config` says about this password file: the refusal, or the password when it was accepted.
fn app_password_of(tag: &str, pw: &PathBuf) -> Result<String, DbConfigError> {
    db_config(
        &config(tag, Some(("db", pw))),
        &registry(),
        Arc::new(SystemClock),
    )
    .map(|c| c.app_password.expose().to_owned())
}

/// A refusal is one fixed line: no path, no content.
fn assert_fixed_refusal(e: DbConfigError, canary: &str) {
    assert_eq!(e, DbConfigError::PasswordUnreadable);
    let text = e.to_string();
    assert!(
        !text.is_empty() && !text.contains('\n') && !text.contains(canary),
        "{text:?}"
    );
}

#[test]
fn as10_a_symlink_is_not_followed_to_a_password() {
    let real = password("pw-link-real", "canary-linked-pw\n");
    let link = dir("pw-link").join("app.pw");
    std::os::unix::fs::symlink(&real, &link).expect("symlink");
    assert_fixed_refusal(
        app_password_of("link", &link).expect_err("a symlink is refused"),
        "canary-linked-pw",
    );
}

#[test]
fn as10_a_fifo_is_refused_and_does_not_block() {
    let fifo = dir("pw-fifo").join("app.pw");
    let made = std::process::Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("mkfifo runs");
    assert!(made.success(), "mkfifo");
    // Off the test thread: the old reader blocks forever on a FIFO with no writer.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(app_password_of("fifo", &fifo));
    });
    let got = rx
        .recv_timeout(std::time::Duration::from_secs(3))
        .expect("db_config returned (it must not block on a FIFO)");
    assert_fixed_refusal(got.expect_err("a FIFO is refused"), "fifo");
}

#[test]
fn as10_a_directory_is_refused() {
    let d = dir("pw-isdir");
    assert_fixed_refusal(
        app_password_of("isdir", &d).expect_err("a directory"),
        "pw-isdir",
    );
}

#[test]
fn as10_the_size_limit_is_4096_bytes() {
    let at = "a".repeat(4096);
    let over = "a".repeat(4097);
    let f = password_bytes("pw-4096", at.as_bytes());
    assert_eq!(app_password_of("s4096", &f).as_deref(), Ok(at.as_str()));
    let f = password_bytes("pw-4097", over.as_bytes());
    assert_fixed_refusal(
        app_password_of("s4097", &f).expect_err("4097 bytes"),
        "aaaa",
    );
    // The limit counts the file, final newline included.
    let nl = format!("{}\n", "a".repeat(4096));
    let f = password_bytes("pw-4096nl", nl.as_bytes());
    assert_fixed_refusal(
        app_password_of("s4096nl", &f).expect_err("4097 bytes with its newline"),
        "aaaa",
    );
}

#[test]
fn as10_a_nul_or_invalid_utf8_is_refused() {
    let f = password_bytes("pw-nul", b"canary-nul\0tail\n");
    assert_fixed_refusal(app_password_of("nul", &f).expect_err("NUL"), "canary-nul");
    let f = password_bytes("pw-utf8", &[0xff, 0xfe, b'x', b'\n']);
    assert_fixed_refusal(app_password_of("utf8", &f).expect_err("not UTF-8"), "x");
}

#[test]
fn as10_only_one_final_line_ending_is_not_part_of_the_password() {
    for (bytes, want) in [
        (&b"pw"[..], "pw"),
        (&b"pw\n"[..], "pw"),
        (&b"pw\r\n"[..], "pw"),
        (&b"pw\n\n"[..], "pw\n"),
        (&b"pw\r\n\r\n"[..], "pw\r\n"),
        (&b"pw\r"[..], "pw\r"),
        (&b" pw with spaces "[..], " pw with spaces "),
    ] {
        let f = password_bytes("pw-ends", bytes);
        assert_eq!(
            app_password_of("ends", &f).as_deref(),
            Ok(want),
            "{bytes:?}"
        );
    }
    for bytes in [&b""[..], &b"\n"[..], &b"\r\n"[..]] {
        let f = password_bytes("pw-emptyish", bytes);
        assert!(
            app_password_of("emptyish", &f).is_err(),
            "{bytes:?} is not a password"
        );
    }
}

/// Without the `mod-policy` feature the policy is not compiled in, there is no policy check to make, and the production checks pass a
/// registry (the server cannot serve in that build anyway: no piece provides the scope policy). With the feature, `serve_policy.rs` covers it.
#[cfg(not(feature = "mod-policy"))]
#[test]
fn as11_without_the_policy_feature_the_production_checks_have_nothing_to_refuse() {
    let reg = assemble_with(
        vec![auth(), thing(), other()],
        &ids(&["fx_auth", "fx_thing"]),
        minter(),
    )
    .expect("assembles");
    assert!(cortex_server::serve::production_checks(&reg).is_ok());
}
