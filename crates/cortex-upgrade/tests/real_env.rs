//! Piece 14, real ports: R1 (no shell, nothing looked up on PATH but `podman`), R2 (a fixed environment for every
//! child) and R5 (no token anywhere). The crate forbids `unsafe`, so a test cannot set variables in its own process: each
//! cell re-runs itself as a CHILD test process with the environment it needs (`common::rerun_self`) and the child runs
//! the wrappers against fakes whose logs the parent then reads.
#[path = "real_fakes.rs"]
mod fakes;

use cortex_types::ModuleId;
use cortex_upgrade::ports::{Containers, DumpLabel, Dumper, Psql, Service};
use cortex_upgrade::real::{ComposeEnv, Deadlines, RealContainers, RealDumper, RealPsql};
use cortex_upgrade::version::parse_version;
use fakes::{Harness, Paths, common};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

const SENTINEL: &str = "SYNTHETIC-TOKEN-REAL-PORTS";

fn quick() -> Deadlines {
    Deadlines {
        load: Duration::from_secs(20),
        up: Duration::from_secs(20),
        inspect: Duration::from_secs(20),
    }
}

/// Every method of the three wrappers, once, against the fakes at `p`; what they returned, as one Debug text.
fn exercise(c: &RealContainers, p: &Paths) -> String {
    let mut shown = String::new();
    let release = &p.release;
    shown += &format!("{:?}\n", c.load_image(&p.base.join("a.tar")));
    shown += &format!(
        "{:?}\n",
        c.compose_up(release, &[Service::Db, Service::Server])
    );
    shown += &format!("{:?}\n", c.state(release, Service::Server));
    shown += &format!(
        "{:?}\n",
        c.run_roles(
            release,
            &[ModuleId::parse("audit").unwrap()],
            Duration::from_secs(20)
        )
    );
    let psql = RealPsql::with(
        p.podman.as_os_str(),
        Duration::from_secs(20),
        release.clone(),
    );
    shown += &format!("{:?}\n", psql.trust_rule_count());
    let dumper = RealDumper::new(p.home.clone(), release.clone());
    let label = DumpLabel::new(
        &parse_version("0.2.0").unwrap(),
        &parse_version("0.3.0").unwrap(),
    );
    shown += &format!("{:?}\n", dumper.dump(&label, Duration::from_secs(20)));
    shown += &format!("{c:?} {psql:?} {dumper:?}\n");
    shown
}

/// The harness every cell here uses: a fake podman that answers plausibly for each call, a dump tool, a roles script.
fn harness(tag: &str) -> Harness {
    let h = Harness::new(
        tag,
        &format!(
            "case \"$1\" in\n load) echo 'Loaded image: localhost/a:1' ;;\n image) echo '{}' ;;\n inspect) echo 'cortexv2-server|running|0|healthy' ;;\n exec) echo 0 ;;\nesac\n",
            fakes::DIGEST
        ),
    );
    h.dump_tool("echo /s/p\nexit 0\n");
    h.roles_script("exit 0\n");
    h
}

const ENV_MARK: &str = "CORTEX_REAL_ENV_CHILD";

/// Like `common::rerun_self`, and the child also LACKS the variables named in `unset` (the test process may have them).
fn rerun_without(
    test: &str,
    marker: (&str, &str),
    env: &[(&str, &str)],
    unset: &[&str],
) -> std::process::Output {
    let mut cmd = std::process::Command::new(std::env::current_exe().unwrap());
    cmd.args(["--exact", test, "--nocapture", "--test-threads=1"])
        .env(marker.0, marker.1);
    for k in unset {
        cmd.env_remove(k);
    }
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().unwrap()
}

#[test]
fn re_01_every_child_gets_only_the_four_fixed_names_and_compose_three_more_from_its_constructor() {
    if let Ok(base) = std::env::var(ENV_MARK) {
        let p = Paths::at(&base);
        let c = RealContainers::with(
            p.podman.as_os_str(),
            quick(),
            ComposeEnv::new(p.home.clone(), fakes::PORT),
        );
        println!("{}", exercise(&c, &p));
        return;
    }
    let poison: [(&str, &str); 19] = [
        // the parent's own values of the three compose variables must NOT reach the compose child
        ("CORTEX_CONFIG", "/evil/cortex.toml"),
        ("CORTEX_DUMP_STATUS", "/evil/dump-status"),
        ("CORTEX_PORT", "1"),
        ("CORTEX_DB_IMAGE", "example.invalid/db:1"),
        ("CORTEX_RELEASE_TOKEN", SENTINEL),
        ("CORTEX_ANYTHING", "x"),
        ("HTTPS_PROXY", "http://127.0.0.1:9"),
        ("https_proxy", "http://127.0.0.1:9"),
        ("HTTP_PROXY", "http://127.0.0.1:9"),
        ("NO_PROXY", "example.invalid"),
        ("COMPOSE_PROJECT_NAME", "evil"),
        ("COMPOSE_FILE", "/evil.yml"),
        ("PODMAN_HOST", "tcp://evil"),
        ("CONTAINER_HOST", "tcp://evil"),
        ("DOCKER_HOST", "tcp://evil"),
        ("LD_PRELOAD", "/evil.so"),
        ("BASH_ENV", "/evil.sh"),
        ("IFS", "x"),
        ("TZ", "Mars/Olympus"),
    ];
    for xdg in [Some("/run/user/4242"), None] {
        let h = harness("re1");
        let mut env: Vec<(&str, &str)> = poison.to_vec();
        env.push(("PATH", "/usr/bin:/bin"));
        env.push(("HOME", "/home/someone"));
        env.push(("LANG", "de_DE.UTF-8"));
        if let Some(x) = xdg {
            env.push(("XDG_RUNTIME_DIR", x));
        }
        let out = rerun_without(
            "re_01_every_child_gets_only_the_four_fixed_names_and_compose_three_more_from_its_constructor",
            (ENV_MARK, &h.tmp.path().display().to_string()),
            &env,
            &["XDG_RUNTIME_DIR"],
        );
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stdout)
        );
        let calls = h.calls();
        assert!(
            calls.len() >= 6,
            "every wrapper started its program: {}",
            calls.len()
        );
        let mut want = vec!["HOME", "LANG", "PATH"];
        if xdg.is_some() {
            want.push("XDG_RUNTIME_DIR");
        }
        for c in &calls {
            let compose = c.argv.first().map(String::as_str) == Some("compose");
            let mut names: Vec<&str> = want.clone();
            if compose {
                names.extend(["CORTEX_CONFIG", "CORTEX_DUMP_STATUS", "CORTEX_PORT"]);
                names.sort();
            }
            assert_eq!(
                c.env_names(),
                names,
                "{} {:?}: {:?}",
                c.program,
                c.argv,
                c.env
            );
            let get = |k: &str| c.env.iter().find_map(|l| l.strip_prefix(&format!("{k}=")));
            assert_eq!(get("LANG"), Some("C"));
            assert_eq!(get("HOME"), Some("/home/someone"));
            assert_eq!(get("PATH"), Some("/usr/bin:/bin"));
            assert_eq!(get("XDG_RUNTIME_DIR"), xdg);
            if compose {
                let home = h.home.display().to_string();
                assert_eq!(
                    get("CORTEX_CONFIG"),
                    Some(format!("{home}/cortex.toml").as_str())
                );
                assert_eq!(
                    get("CORTEX_DUMP_STATUS"),
                    Some(format!("{home}/dump-status").as_str())
                );
                assert_eq!(get("CORTEX_PORT"), Some(fakes::PORT.to_string().as_str()));
            }
        }
    }
}

#[test]
fn re_02_no_wrapper_leaks_a_token_into_a_vector_an_environment_or_a_return_value() {
    if let Ok(base) = std::env::var(ENV_MARK) {
        let p = Paths::at(&base);
        let c = RealContainers::with(
            p.podman.as_os_str(),
            quick(),
            ComposeEnv::new(p.home.clone(), fakes::PORT),
        );
        let shown = exercise(&c, &p);
        assert!(
            !shown.contains(SENTINEL),
            "a returned value or a Debug text holds the sentinel: {shown}"
        );
        println!("CHILD-OK");
        return;
    }
    let h = harness("re2");
    // The sentinel is also in files around: the operator's token file, the release's stack.env, next to the tool.
    fs::write(h.home.join("operator.token"), format!("{SENTINEL}\n")).unwrap();
    fs::write(
        h.home.join("cortex.toml"),
        format!("token = \"{SENTINEL}\"\n"),
    )
    .unwrap();
    fs::write(
        h.release.join("stack.env"),
        format!("CORTEX_RELEASE_TOKEN={SENTINEL}\n"),
    )
    .unwrap();
    let out = common::rerun_self(
        "re_02_no_wrapper_leaks_a_token_into_a_vector_an_environment_or_a_return_value",
        (ENV_MARK, &h.tmp.path().display().to_string()),
        &[
            ("CORTEX_RELEASE_TOKEN", SENTINEL),
            ("CORTEX_OPERATOR_TOKEN", SENTINEL),
            ("PATH", "/usr/bin:/bin"),
        ],
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("CHILD-OK"), "{text}");
    assert!(!text.contains(SENTINEL), "the child printed the sentinel");
    let log = fs::read_to_string(&h.log).unwrap();
    assert!(
        !log.is_empty() && !log.contains(SENTINEL),
        "the fakes' log holds the sentinel"
    );
    assert!(
        !String::from_utf8_lossy(&out.stderr).contains(SENTINEL),
        "the child wrote the sentinel to its error stream"
    );
}

const PATH_MARK: &str = "CORTEX_REAL_PATH_CHILD";

#[test]
fn re_03_no_shell_is_started_and_only_podman_is_looked_up_on_path() {
    if let Ok(base) = std::env::var(PATH_MARK) {
        // `RealContainers::new()` and `RealPsql::new()` look `podman` up on PATH, which starts with the fakes directory.
        let p = Paths::at(&base);
        let c = RealContainers::new(
            p.home.clone(),
            fakes::PORT,
            std::path::PathBuf::from("/usr/bin/podman-compose"),
        );
        let shown = {
            let mut s = String::new();
            s += &format!("{:?}\n", c.compose_up(&p.release, &[Service::Db]));
            s += &format!("{:?}\n", c.state(&p.release, Service::Server));
            s += &format!(
                "{:?}\n",
                c.load_image(&p.base.join("a b;touch HACKED $(id) `id`.tar"))
            );
            s += &format!(
                "{:?}\n",
                c.run_roles(
                    &p.release,
                    &[ModuleId::parse("audit").unwrap()],
                    Duration::from_secs(20)
                )
            );
            s += &format!(
                "{:?}\n",
                RealPsql::new(p.release.clone()).trust_rule_count()
            );
            let label = DumpLabel::new(
                &parse_version("0.2.0").unwrap(),
                &parse_version("0.3.0").unwrap(),
            );
            s += &format!(
                "{:?}\n",
                RealDumper::new(p.home.clone(), p.release.clone())
                    .dump(&label, Duration::from_secs(20))
            );
            s
        };
        println!("CHILD-OK {}", shown.len());
        return;
    }
    let h = harness("re3");
    // this cell passes an archive path full of shell characters on purpose (see the child above)
    h.hostile_archive_path.set(true);
    let bin = h.tmp.path().join("pathbin");
    let marker = h.tmp.path().join("SHELL-INVOKED");
    let wrong = h.tmp.path().join("WRONG-PROGRAM");
    // Fake shells and fake look-alikes of the other two programs, FIRST on PATH.
    fs::create_dir_all(&bin).unwrap();
    for name in ["sh", "bash", "dash", "zsh", "env"] {
        let p = bin.join(name);
        fs::write(
            &p,
            format!("#!/bin/sh\necho {name} >> '{}'\nexit 0\n", marker.display()),
        )
        .unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    }
    for name in ["cortex-dump", "db-roles.sh"] {
        let p = bin.join(name);
        fs::write(
            &p,
            format!("#!/bin/sh\necho {name} >> '{}'\nexit 0\n", wrong.display()),
        )
        .unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
    }
    // The fake podman on the same PATH.
    fs::copy(&h.podman, bin.join("podman")).unwrap();
    fs::set_permissions(bin.join("podman"), fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!("{}:/usr/bin:/bin", bin.display());
    let out = common::rerun_self(
        "re_03_no_shell_is_started_and_only_podman_is_looked_up_on_path",
        (PATH_MARK, &h.tmp.path().display().to_string()),
        &[("PATH", &path)],
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("CHILD-OK"),
        "{text} {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !marker.exists(),
        "SHELL-INVOKED: {}",
        fs::read_to_string(&marker).unwrap_or_default()
    );
    assert!(
        !wrong.exists(),
        "a look-alike on PATH was started instead of the exact path"
    );
    assert!(
        !h.tmp.path().join("HACKED").exists() && !h.tmp.path().join("home/HACKED").exists(),
        "an archive name was run as a command"
    );
    let calls = h.calls();
    // The fake podman copy logs to the same file; the dump tool and the roles script are the ones under home/release.
    assert!(
        calls.iter().any(|c| c.program.ends_with("/pathbin/podman")),
        "podman was found on PATH"
    );
    assert!(
        calls
            .iter()
            .any(|c| c.program == h.home.join("bin/cortex-dump").display().to_string())
    );
    assert!(
        calls
            .iter()
            .any(|c| c.program == h.release.join("db-roles.sh").display().to_string())
    );
    // The archive name reached podman as ONE argument, whole.
    let load = calls
        .iter()
        .find(|c| c.argv.first().map(String::as_str) == Some("load"))
        .unwrap();
    assert_eq!(
        load.argv[2],
        h.tmp
            .path()
            .join("a b;touch HACKED $(id) `id`.tar")
            .display()
            .to_string()
    );
}

const STDIN_MARK: &str = "CORTEX_REAL_STDIN_CHILD";

/// The child of a wrapper has NO input: its descriptor 0 is `/dev/null`, whatever the test process was given.
#[test]
fn re_04_a_child_gets_dev_null_as_its_input_not_the_parents() {
    use cortex_upgrade::real::run::run;
    use std::ffi::OsString;
    use std::process::Stdio;
    if let Ok(dir) = std::env::var(STDIN_MARK) {
        let fake = std::path::Path::new(&dir).join("fake");
        let none: Vec<OsString> = vec![];
        run(fake.as_os_str(), &none, Duration::from_secs(20)).unwrap();
        return;
    }
    let (t, _fake, _log) = fakes::standalone(
        "re4",
        "readlink /proc/$$/fd/0 > \"$(dirname \"$0\")/stdin\"\nexit 0\n",
    );
    // The test binary runs again with a PIPE as its own standard input (still open while the fake looks).
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "re_04_a_child_gets_dev_null_as_its_input_not_the_parents",
            "--nocapture",
            "--test-threads=1",
        ])
        .env(STDIN_MARK, t.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let held = child.stdin.take();
    let status = child.wait().unwrap();
    drop(held);
    assert!(status.success());
    assert_eq!(
        fs::read_to_string(t.path().join("stdin")).unwrap().trim(),
        "/dev/null"
    );
}

/// An extra variable (the compose child's) can never replace one of the four fixed names.
#[test]
fn re_05_an_extra_variable_never_replaces_a_fixed_name() {
    use cortex_upgrade::real::env::child_env;
    use std::ffi::OsString;
    let extra: Vec<(OsString, OsString)> = vec![
        (OsString::from("PATH"), OsString::from("/evil")),
        (OsString::from("HOME"), OsString::from("/evil")),
        (OsString::from("LANG"), OsString::from("evil")),
        (OsString::from("XDG_RUNTIME_DIR"), OsString::from("/evil")),
        (OsString::from("CORTEX_PORT"), OsString::from("8601")),
    ];
    let env = child_env(&extra);
    let get = |k: &str| env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
    assert_eq!(get("PATH"), std::env::var_os("PATH"));
    assert_eq!(get("HOME"), std::env::var_os("HOME"));
    assert_eq!(get("XDG_RUNTIME_DIR"), std::env::var_os("XDG_RUNTIME_DIR"));
    assert_eq!(get("LANG"), Some(OsString::from("C")));
    assert_eq!(get("CORTEX_PORT"), Some(OsString::from("8601")));
    // each name once
    let mut names: Vec<_> = env.iter().map(|(n, _)| n.clone()).collect();
    let before = names.len();
    names.sort();
    names.dedup();
    assert_eq!(names.len(), before);
}

const RESERVED_MARK: &str = "CORTEX_REAL_RESERVED_CHILD";

/// vera's UPREAL-L1: the four fixed names are reserved UNCONDITIONALLY. When the parent lacks `HOME`, `PATH` or
/// `XDG_RUNTIME_DIR`, an extra variable of that name must still not supply it.
#[test]
fn re_06_the_fixed_names_are_reserved_even_when_the_parent_lacks_them() {
    use cortex_upgrade::real::env::child_env;
    use std::ffi::OsString;
    if std::env::var_os(RESERVED_MARK).is_some() {
        let extra: Vec<(OsString, OsString)> = [
            ("PATH", "/evil"),
            ("HOME", "/evil"),
            ("LANG", "evil"),
            ("XDG_RUNTIME_DIR", "/evil"),
            ("CORTEX_PORT", "8601"),
        ]
        .iter()
        .map(|(k, v)| (OsString::from(k), OsString::from(v)))
        .collect();
        for (k, v) in child_env(&extra) {
            println!("ENV {}={}", k.to_string_lossy(), v.to_string_lossy());
        }
        return;
    }
    let out = rerun_without(
        "re_06_the_fixed_names_are_reserved_even_when_the_parent_lacks_them",
        (RESERVED_MARK, "1"),
        &[],
        &["PATH", "HOME", "XDG_RUNTIME_DIR"],
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{text}");
    assert!(
        !text.contains("evil"),
        "an extra supplied a fixed name: {text}"
    );
    assert!(text.contains("ENV LANG=C\n"), "{text}");
    assert!(text.contains("ENV CORTEX_PORT=8601\n"), "{text}");
    for k in ["PATH", "HOME", "XDG_RUNTIME_DIR"] {
        assert!(
            !text.contains(&format!("ENV {k}=")),
            "{k} appeared although the parent lacks it: {text}"
        );
    }
}
