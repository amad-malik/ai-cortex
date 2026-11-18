//! CL-01, CL-02, CL-06 (registry part), CL-07: the registry refuses duplicates, owners edit only their own file, the help is
//! sorted, and no flag takes a credential value. DB-free, no process, no network.
use clap::{ArgMatches, Command};
use cortex_cli::groups;
use cortex_cli::registry::{Ctx, Group, Output, Registry, RegistryError};
use cortex_client::ClientError;
use std::fs;
use std::path::Path;

fn run_nothing(_c: &Ctx<'_>, _n: &str, _m: &ArgMatches) -> Result<Output, ClientError> {
    Ok(Output::default())
}

fn group(name: &'static str, commands: fn() -> Vec<Command>) -> Group {
    Group {
        name,
        owner: "test",
        implements: &[],
        commands,
        run: run_nothing,
    }
}

fn alpha() -> Vec<Command> {
    vec![Command::new("alpha")]
}
fn zeta() -> Vec<Command> {
    vec![Command::new("zeta")]
}
fn dup() -> Vec<Command> {
    vec![Command::new("dup")]
}

#[test]
fn cl_01_a_duplicate_group_or_command_is_refused_at_startup() {
    let same_name = Registry::new(vec![group("g", alpha), group("g", zeta)]);
    assert_eq!(
        same_name.err(),
        Some(RegistryError::DuplicateGroup("g".into()))
    );
    let same_command = Registry::new(vec![group("one", dup), group("two", dup)]);
    assert_eq!(
        same_command.err(),
        Some(RegistryError::DuplicateCommand("dup".into()))
    );
    // a group that contributes the same command twice
    fn twice() -> Vec<Command> {
        vec![Command::new("x"), Command::new("x")]
    }
    assert_eq!(
        Registry::new(vec![group("g", twice)]).err(),
        Some(RegistryError::DuplicateCommand("x".into()))
    );
    assert!(Registry::new(vec![group("one", alpha), group("two", zeta)]).is_ok());
    assert!(
        Registry::new(groups::all()).is_ok(),
        "the real groups are consistent"
    );
}

fn groups_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/groups")
}

#[test]
fn cl_02_each_group_is_one_file_and_one_line_and_names_no_other_group() {
    let mut stems = Vec::new();
    for e in fs::read_dir(groups_dir()).unwrap() {
        let p = e.unwrap().path();
        let stem = p.file_stem().unwrap().to_string_lossy().into_owned();
        if stem == "mod" {
            continue;
        }
        let text = fs::read_to_string(&p).unwrap();
        // a `Group { .. }` literal; the signature `-> Group {` of the constructor is not one
        let literals = text.matches("Group {").count() - text.matches("-> Group {").count();
        assert_eq!(literals, 1, "{stem}.rs defines exactly ONE Group");
        for banned in ["super::", "crate::groups::", "groups::"] {
            assert!(
                !text.contains(banned),
                "{stem}.rs names another group's module ({banned})"
            );
        }
        stems.push(stem);
    }
    assert!(!stems.is_empty(), "the scan saw no group file");
    // comments do not count: the doc comment of mod.rs says how to add a group
    let modrs: String = fs::read_to_string(groups_dir().join("mod.rs"))
        .unwrap()
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    for stem in &stems {
        assert_eq!(
            modrs.matches(&format!("mod {stem};")).count(),
            1,
            "mod.rs declares {stem} once"
        );
        assert_eq!(
            modrs.matches(&format!("{stem}::group()")).count(),
            1,
            "mod.rs registers {stem} once"
        );
    }
    assert_eq!(
        modrs.matches("::group()").count(),
        stems.len(),
        "one register line per group file"
    );
}

#[test]
fn cl_06_groups_and_commands_are_sorted_and_the_help_is_stable() {
    let r = Registry::new(vec![group("zeta", zeta), group("alpha", alpha)]).unwrap();
    let names: Vec<String> = r
        .command()
        .get_subcommands()
        .map(|c| c.get_name().to_owned())
        .filter(|n| n != "help")
        .collect();
    assert_eq!(
        names,
        ["alpha", "zeta"],
        "sorted by name whatever the registration order"
    );
    let a = r.command().render_help().to_string();
    let b = r.command().render_help().to_string();
    assert_eq!(a, b);
    assert!(a.find("alpha").unwrap() < a.find("zeta").unwrap());
    assert!(!a.contains('\u{1b}'), "no ESC byte in the help");
}

fn walk(cmd: &Command, found: &mut Vec<(String, Option<String>)>) {
    for a in cmd.get_arguments() {
        let long = a.get_long().unwrap_or("").to_owned();
        let value_names: Vec<String> = a
            .get_value_names()
            .map(|v| v.iter().map(|s| s.to_string()).collect())
            .unwrap_or_default();
        found.push((long, value_names.first().cloned()));
    }
    for sub in cmd.get_subcommands() {
        walk(sub, found);
    }
}

#[test]
fn cl_07_no_flag_takes_a_credential() {
    let r = Registry::new(groups::all()).unwrap();
    let mut flags = Vec::new();
    walk(&r.command(), &mut flags);
    assert!(
        flags.iter().any(|(l, _)| l == "token-file"),
        "positive control: --token-file exists: {flags:?}"
    );
    assert!(flags.iter().any(|(l, _)| l == "server") && flags.iter().any(|(l, _)| l == "format"));
    for (long, value_name) in &flags {
        if long.contains("token") {
            assert_eq!(
                long, "token-file",
                "the only flag that names a token is --token-file"
            );
        }
        for banned in ["password", "secret", "key"] {
            assert!(
                !long.contains(banned),
                "--{long} looks like a credential flag"
            );
        }
        if let Some(v) = value_name {
            assert!(
                !v.eq_ignore_ascii_case("token"),
                "--{long} has the value name TOKEN"
            );
        }
    }
}

#[test]
fn n3_no_group_file_names_problem_or_reads_a_servers_text_fields() {
    // ellis N-3 (cox's batch of 7 Nov): a group prints a failure through the error's Display or `safe_text`, never through a raw
    // `Problem` field. Without the type's name a group cannot bind its fields, so the guard is the name and the three field reads.
    let mut seen = 0;
    for e in fs::read_dir(groups_dir()).unwrap() {
        let p = e.unwrap().path();
        let stem = p.file_stem().unwrap().to_string_lossy().into_owned();
        if stem == "mod" {
            continue;
        }
        seen += 1;
        let text = fs::read_to_string(&p).unwrap();
        for (i, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            for banned in ["Problem", ".detail", ".fix", ".title"] {
                assert!(
                    !code.contains(banned),
                    "N-3: {}:{} uses {banned}; print the error with its Display or cortex_client::display::safe_text: {line}",
                    p.display(),
                    i + 1
                );
            }
        }
    }
    assert!(seen >= 1, "the scan saw no group file");
}

/// The files under `dir` (relative to `root`, `/`-separated, sorted) whose text contains `needle`.
fn rs_files_naming(root: &Path, dir: &Path, needle: &str, out: &mut Vec<String>) {
    for e in fs::read_dir(dir).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            rs_files_naming(root, &p, needle, out);
        } else if p.extension().is_some_and(|x| x == "rs")
            && fs::read_to_string(&p).unwrap().contains(needle)
        {
            out.push(
                p.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
    out.sort();
}

/// W1-80 (cox 8 Nov 01:04Z): the CLI binary carries `cortex-upgrade`, and a TLS stack with it, through ONE group file (finn's
/// `groups/upgrade.rs`). No other source file of the CLI may name the crate: the operator-token path must not reach it.
#[test]
fn w1_80_only_the_upgrade_group_file_names_cortex_upgrade() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found = Vec::new();
    rs_files_naming(&src, &src, "cortex_upgrade", &mut found);
    let others: Vec<&String> = found.iter().filter(|f| *f != "groups/upgrade.rs").collect();
    assert!(
        others.is_empty(),
        "W1-80: only src/groups/upgrade.rs may name cortex_upgrade, these also do: {others:?}"
    );
}

/// The scanner of the cell above finds a planted mention in a nested file and ignores a file that does not name the crate and a
/// non-Rust file that does (a control: an empty result from a broken scanner would pass the cell above).
#[test]
fn w1_80_the_scanner_finds_a_planted_mention_and_nothing_else() {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("w180-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("a/b")).unwrap();
    fs::write(root.join("x.rs"), "use cortex_upgrade::Engine;\n").unwrap();
    fs::write(
        root.join("a/b/y.rs"),
        "// cortex_upgrade in a comment still counts\n",
    )
    .unwrap();
    fs::write(root.join("a/z.rs"), "fn nothing() {}\n").unwrap();
    fs::write(root.join("a/notes.txt"), "cortex_upgrade\n").unwrap();
    let mut found = Vec::new();
    rs_files_naming(&root, &root, "cortex_upgrade", &mut found);
    assert_eq!(found, vec!["a/b/y.rs".to_owned(), "x.rs".to_owned()]);
    fs::remove_dir_all(&root).unwrap();
}
