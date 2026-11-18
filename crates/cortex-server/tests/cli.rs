//! The command line (contract r0.3 section 1). DB-free. RED first.

use cortex_server::cli::{
    BootstrapArgs, Command, DEFAULT_CONFIG, EXIT_FAILURE, EXIT_OK, EXIT_REFUSED, EXIT_USAGE,
    UsageError, parse,
};
use std::path::PathBuf;

fn a(s: &[&str]) -> Vec<String> {
    s.iter().map(|x| x.to_string()).collect()
}

fn boot(require: bool) -> Command {
    Command::Bootstrap(BootstrapArgs {
        config: PathBuf::from(DEFAULT_CONFIG),
        project: "p-one".into(),
        lead: "lead-one".into(),
        token_file: PathBuf::from("/run/cortex/bootstrap.token"),
        require_existing: require,
    })
}

#[test]
fn cli_01_the_exit_codes_are_distinct_and_the_refusal_is_not_the_usage_error() {
    assert_eq!(
        (EXIT_OK, EXIT_FAILURE, EXIT_USAGE, EXIT_REFUSED),
        (0, 1, 2, 3)
    );
    assert_ne!(
        EXIT_REFUSED, EXIT_USAGE,
        "an old server that does not know the flag also exits 2: the codes must differ"
    );
}

#[test]
fn cli_02_version() {
    assert_eq!(parse(&a(&["--version"])), Ok(Command::Version));
}

#[test]
fn cli_03_serve_and_migrate_take_an_optional_config_with_the_compose_default() {
    assert_eq!(
        parse(&a(&["serve"])),
        Ok(Command::Serve {
            config: PathBuf::from(DEFAULT_CONFIG)
        })
    );
    assert_eq!(
        parse(&a(&["serve", "--config", "/x/c.toml"])),
        Ok(Command::Serve {
            config: PathBuf::from("/x/c.toml")
        })
    );
    assert_eq!(
        parse(&a(&["migrate"])),
        Ok(Command::Migrate {
            config: PathBuf::from(DEFAULT_CONFIG)
        })
    );
    assert_eq!(
        parse(&a(&["migrate", "--config", "/y.toml"])),
        Ok(Command::Migrate {
            config: PathBuf::from("/y.toml")
        })
    );
}

#[test]
fn cli_04_bootstrap_as_install_sh_calls_it() {
    // deploy/install.sh: cortex-server bootstrap --project P --lead L --token-file /run/cortex/bootstrap.token
    let c = parse(&a(&[
        "bootstrap",
        "--project",
        "p-one",
        "--lead",
        "lead-one",
        "--token-file",
        "/run/cortex/bootstrap.token",
    ]));
    assert_eq!(c, Ok(boot(false)));
}

#[test]
fn cli_05_require_existing_is_a_flag_and_the_order_does_not_matter() {
    let c = parse(&a(&[
        "bootstrap",
        "--require-existing",
        "--token-file",
        "/run/cortex/bootstrap.token",
        "--lead",
        "lead-one",
        "--project",
        "p-one",
    ]));
    assert_eq!(c, Ok(boot(true)));
}

#[test]
fn cli_06_usage_errors() {
    let bad: Vec<Vec<&str>> = vec![
        vec![],
        vec!["unknown"],
        vec!["--version", "serve"],
        vec!["serve", "--bogus"],
        vec!["serve", "extra"],
        vec!["serve", "--config"],
        vec!["serve", "--config", "/a", "--config", "/b"],
        vec!["serve", "--require-existing"],
        vec!["migrate", "--project", "p"],
        vec!["bootstrap", "--lead", "l", "--token-file", "/t"],
        vec!["bootstrap", "--project", "p", "--token-file", "/t"],
        vec!["bootstrap", "--project", "p", "--lead", "l"],
        vec!["bootstrap", "--project", "p", "--lead", "l", "--token-file"],
        vec![
            "bootstrap",
            "--project",
            "p",
            "--project",
            "q",
            "--lead",
            "l",
            "--token-file",
            "/t",
        ],
        vec![
            "bootstrap",
            "--project",
            "p",
            "--lead",
            "l",
            "--token-file",
            "/t",
            "--require-existing",
            "--require-existing",
        ],
        vec![
            "bootstrap",
            "--project",
            "",
            "--lead",
            "l",
            "--token-file",
            "/t",
        ],
    ];
    for args in bad {
        let r = parse(&a(&args));
        assert!(
            matches!(r, Err(UsageError(ref m)) if !m.is_empty()),
            "expected a usage error for {args:?}, got {r:?}"
        );
    }
}

#[test]
fn cli_07_a_usage_message_never_echoes_a_value() {
    // The values (names, paths) of a refused command line are operator input; the message names the FLAG, never the value.
    let r = parse(&a(&[
        "bootstrap",
        "--project",
        "SECRET-NAME-123",
        "--lead",
        "l",
        "--bogus",
        "SECRET-VALUE-456",
    ]));
    let UsageError(m) = r.expect_err("must be a usage error");
    assert!(
        !m.contains("SECRET-NAME-123") && !m.contains("SECRET-VALUE-456"),
        "the message must not echo values: {m}"
    );
}
