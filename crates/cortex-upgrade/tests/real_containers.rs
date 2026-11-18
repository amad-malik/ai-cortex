//! Piece 14, real ports: `RealContainers` (brief r1 section 3). A fake `podman`, a fake roles script, no real program.
//! Rules covered here: R1 (the whole argument vector), R3 (every logged vector checked against the closed list by the
//! `Harness`' drop), R4 (exact names), R8 (a program that cannot start).
#[path = "real_fakes.rs"]
mod fakes;

use cortex_types::ModuleId;
use cortex_upgrade::manifest::Digest;
use cortex_upgrade::ports::{Containers, OneLine, PortError, Service, ServiceState};
use cortex_upgrade::real::{Deadlines, RealContainers};
use fakes::{DIGEST, Harness, INSPECT_FORMAT};
use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;
use std::time::{Duration, Instant};

fn quick() -> Deadlines {
    Deadlines {
        load: Duration::from_secs(20),
        up: Duration::from_secs(20),
        inspect: Duration::from_secs(20),
    }
}

fn containers(h: &Harness) -> RealContainers {
    RealContainers::with(h.podman.as_os_str(), quick(), fakes::compose_env(&h.home))
}

fn failed(msg: &str) -> PortError {
    PortError::Failed(OneLine::new(msg))
}

fn digest(text: &str) -> Digest {
    let hex = text.strip_prefix("sha256:").unwrap();
    let mut out = [0u8; 32];
    for i in 0..32 {
        out[i] = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap();
    }
    Digest(out)
}

/// A fake `podman` answering `load` with `load_out` (printf %b) and `image inspect` with `digest_out`.
fn podman_load(load_out: &str, digest_out: &str) -> String {
    format!(
        "case \"$1\" in\n load) printf '%b' '{load_out}' ;;\n image) printf '%b' '{digest_out}' ;;\nesac\n"
    )
}

// ---------------------------------------------------------------- load_image

#[test]
fn rc_01_load_image_runs_load_then_image_inspect_and_parses_the_digest() {
    let h = Harness::new(
        "rc1",
        &podman_load(
            "Loaded image: localhost/cortex-server:0.2.0\\n",
            &format!("{DIGEST}\\n"),
        ),
    );
    let archive = h.tmp.path().join("images.tar");
    let d = containers(&h).load_image(&archive).unwrap();
    assert_eq!(d, digest(DIGEST));
    let calls = h.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(
        calls[0].argv,
        ["load", "--input", archive.to_str().unwrap()]
    );
    assert_eq!(
        calls[1].argv,
        [
            "image",
            "inspect",
            "--format",
            "{{.Digest}}",
            "localhost/cortex-server:0.2.0"
        ]
    );
}

#[test]
fn rc_02_the_reference_comes_from_exactly_one_loaded_image_line_that_is_the_last_line() {
    let ok = |load: &str, want_ref: &str| {
        let h = Harness::new("rc2ok", &podman_load(load, &format!("{DIGEST}\\n")));
        let r = containers(&h).load_image(&h.tmp.path().join("a.tar"));
        assert!(r.is_ok(), "{load:?}: {r:?}");
        assert_eq!(h.calls()[1].argv.last().unwrap(), want_ref, "{load:?}");
    };
    ok("Loaded image: localhost/a:1\\n", "localhost/a:1");
    ok("Loaded image(s): localhost/a:1\\n", "localhost/a:1");
    ok("Loaded image: localhost/a:1", "localhost/a:1");
    ok(
        "Getting image source signatures\\nCopying blob 1\\nLoaded image: localhost/a:1\\n",
        "localhost/a:1",
    );
    ok(
        &format!("Loaded image: a{}\\n", "b".repeat(255)),
        &format!("a{}", "b".repeat(255)),
    );
    ok(
        "Loaded image: example.invalid/db:1@sha256:abc\\n",
        "example.invalid/db:1@sha256:abc",
    );

    let bad = |load: &str| {
        let h = Harness::new("rc2bad", &podman_load(load, &format!("{DIGEST}\\n")));
        let r = containers(&h).load_image(&h.tmp.path().join("a.tar"));
        assert!(matches!(r, Err(PortError::Failed(_))), "{load:?}: {r:?}");
        assert_eq!(
            h.calls().len(),
            1,
            "image inspect must not run for {load:?}"
        );
    };
    bad("");
    bad("nothing was loaded\\n");
    bad("Loaded image: localhost/a:1\\nLoaded image: localhost/b:2\\n");
    // the same line twice: still two reports, so no single reference (a comparison of the last line with the
    // first would pass it)
    bad("Loaded image: localhost/a:1\\nLoaded image: localhost/a:1\\n");
    bad("Loaded image(s): localhost/a:1,localhost/b:2\\n");
    bad("Loaded image: localhost/a:1\\nsome trailing text\\n");
    bad("Loaded image: \\n");
    bad("Loaded image: a b\\n");
    bad("Loaded image: a;rm -rf x\\n");
    bad("Loaded image: $(id)\\n");
    bad("Loaded image: a`id`\\n");
    bad("Loaded image: -x\\n");
    bad("Loaded image: .hidden\\n");
    bad("Loaded image: a*\\n");
    bad("Loaded image: a;b\\n");
    bad("Loaded image: a$b\\n");
    bad("Loaded image: a`b\\n");
    bad("Loaded image: a|b\\n");
    bad("Loaded image: a&b\\n");
    bad("Loaded image: a\\\\b\\n");
    bad(&format!("Loaded image: a{}\\n", "b".repeat(256)));
    bad("Loaded image: caf\\0303\\0251\\n");
}

#[test]
fn rc_03_a_digest_is_sha256_and_64_lowercase_hex_digits_or_the_image_has_none() {
    let none = failed("the loaded image has no digest");
    let case = |answer: &str, want: Result<Digest, PortError>| {
        let h = Harness::new(
            "rc3",
            &podman_load("Loaded image: localhost/a:1\\n", answer),
        );
        let r = containers(&h).load_image(&h.tmp.path().join("a.tar"));
        assert_eq!(r, want, "{answer:?}");
    };
    case(&format!("{DIGEST}\\n"), Ok(digest(DIGEST)));
    case(DIGEST, Ok(digest(DIGEST)));
    case(&format!("{DIGEST}\\r\\n"), Ok(digest(DIGEST)));
    case(&format!("noise\\n{DIGEST}\\n"), Ok(digest(DIGEST)));
    case("", Err(none.clone()));
    case("\\n", Err(none.clone()));
    case("<no value>\\n", Err(none.clone()));
    case(
        &DIGEST.to_uppercase().replace("SHA256", "sha256"),
        Err(none.clone()),
    );
    case(&DIGEST[..DIGEST.len() - 1], Err(none.clone()));
    case(&format!("{DIGEST}0"), Err(none.clone()));
    case(&DIGEST.replacen("sha256", "sha512", 1), Err(none.clone()));
    case(&format!("{DIGEST} extra"), Err(none.clone()));
    case(&format!("{DIGEST}\\nmore\\n"), Err(none.clone()));
    case(&DIGEST.replacen('0', "g", 1), Err(none.clone()));
    case(&DIGEST.replacen("sha256:", "", 1), Err(none));
}

#[test]
fn rc_04_a_failing_load_or_inspect_is_failed_with_the_last_stderr_line() {
    let h = Harness::new(
        "rc4a",
        "case \"$1\" in\n load) echo 'noise' >&2; echo 'Error: the archive is bad' >&2; exit 125 ;;\nesac\n",
    );
    let r = containers(&h).load_image(&h.tmp.path().join("a.tar"));
    assert_eq!(r, Err(failed("Error: the archive is bad")));
    assert_eq!(h.calls().len(), 1);

    let h = Harness::new(
        "rc4b",
        "case \"$1\" in\n load) echo 'Loaded image: localhost/a:1' ;;\n image) echo 'Error: no such image' >&2; exit 125 ;;\nesac\n",
    );
    let r = containers(&h).load_image(&h.tmp.path().join("a.tar"));
    assert_eq!(r, Err(failed("Error: no such image")));
}

// ---------------------------------------------------------------- compose_up

#[test]
fn rc_05_compose_up_runs_podman_compose_with_the_release_files_and_the_services_in_order() {
    let h = Harness::new("rc5", "exit 0\n");
    let c = containers(&h);
    c.compose_up(&h.release, &[Service::Migrate, Service::Server])
        .unwrap();
    c.compose_up(&h.release, &[Service::Db]).unwrap();
    let calls = h.calls();
    let d = h.release.display().to_string();
    assert_eq!(
        calls[0].argv,
        [
            "compose",
            "-f",
            &format!("{d}/compose.yml"),
            "--env-file",
            &format!("{d}/stack.env"),
            "up",
            "-d",
            "migrate",
            "server"
        ]
    );
    assert_eq!(
        calls[1].argv,
        [
            "compose",
            "-f",
            &format!("{d}/compose.yml"),
            "--env-file",
            &format!("{d}/stack.env"),
            "up",
            "-d",
            "db"
        ]
    );
}

#[test]
fn rc_06_compose_up_with_no_service_starts_nothing() {
    let h = Harness::new("rc6", "exit 0\n");
    let r = containers(&h).compose_up(&h.release, &[]);
    assert!(matches!(r, Err(PortError::Failed(_))), "{r:?}");
    assert!(h.calls().is_empty(), "nothing is started for an empty list");
}

#[test]
fn rc_07_compose_up_that_exits_non_zero_is_failed_with_the_last_stderr_line_escaped() {
    let h = Harness::new(
        "rc7",
        "echo first >&2; printf 'last\\tline \\033[31m\\n' >&2; exit 1\n",
    );
    let r = containers(&h).compose_up(&h.release, &[Service::Server]);
    assert_eq!(r, Err(failed("last\tline \u{1b}[31m")));
    let Err(PortError::Failed(l)) = r else {
        panic!()
    };
    assert!(
        !l.as_str().contains('\t') && !l.as_str().contains('\u{1b}'),
        "escaped"
    );
}

// ---------------------------------------------------------------- state

fn state_of(answer: &str, service: Service) -> Result<ServiceState, PortError> {
    let h = Harness::new("rc8", &format!("printf '%s\\n' '{answer}'\n"));
    let r = containers(&h).state(&h.release, service);
    let calls = h.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0].argv,
        [
            "inspect",
            "--format",
            INSPECT_FORMAT,
            &h.ctr(service.name())
        ]
    );
    r
}

#[test]
fn rc_08_a_container_reading_becomes_a_service_state() {
    let s = |name: &str, rest: &str| format!("cortexv2-{name}|{rest}");
    let cases: Vec<(String, Service, ServiceState)> = vec![
        (
            s("server", "running|0|healthy"),
            Service::Server,
            ServiceState::Healthy,
        ),
        (
            s("server", "running|0|unhealthy"),
            Service::Server,
            ServiceState::Unhealthy,
        ),
        (
            s("server", "running|0|starting"),
            Service::Server,
            ServiceState::Starting,
        ),
        (s("db", "running|0|"), Service::Db, ServiceState::Running),
        (
            s("migrate", "exited|0|"),
            Service::Migrate,
            ServiceState::Exited(0),
        ),
        (
            s("migrate", "exited|3|"),
            Service::Migrate,
            ServiceState::Exited(3),
        ),
        (
            s("migrate", "exited|137|"),
            Service::Migrate,
            ServiceState::Exited(137),
        ),
        (
            s("migrate", "exited|0|healthy"),
            Service::Migrate,
            ServiceState::Exited(0),
        ),
        (
            s("server", "created|0|"),
            Service::Server,
            ServiceState::Other,
        ),
        (
            s("server", "paused|0|"),
            Service::Server,
            ServiceState::Other,
        ),
        (
            s("server", "stopping|0|"),
            Service::Server,
            ServiceState::Other,
        ),
        (
            s("server", "removing|0|"),
            Service::Server,
            ServiceState::Other,
        ),
        (
            s("server", "running|0|something-else"),
            Service::Server,
            ServiceState::Other,
        ),
        (s("server", "dead|0|"), Service::Server, ServiceState::Other),
    ];
    for (answer, svc, want) in cases {
        assert_eq!(state_of(&answer, svc), Ok(want), "{answer}");
    }
}

#[test]
fn rc_09_a_reading_that_is_not_four_fields_with_the_exact_name_is_failed() {
    for answer in [
        "",
        "cortexv2-server",
        "cortexv2-server|running|0",
        "cortexv2-server|running|0|healthy|extra",
        "cortexv2-server|running|zero|healthy",
        "cortexv2-server|running||healthy",
        // R4: an answer for another name than the one asked for
        "cortexv2-db|running|0|healthy",
        "other-server|running|0|healthy",
        "/cortexv2-server|running|0|healthy",
        "cortexv2-server2|running|0|healthy",
        "xcortexv2-server|running|0|healthy",
        "CORTEXV2-SERVER|running|0|healthy",
        "cortexv2-server |running|0|healthy",
    ] {
        let r = state_of(answer, Service::Server);
        assert!(matches!(r, Err(PortError::Failed(_))), "{answer:?}: {r:?}");
    }
}

#[test]
fn rc_10_a_container_that_does_not_exist_is_missing_only_for_exit_125_and_that_exact_message() {
    let body = |code: i32, stderr: &str| format!("echo '{stderr}' >&2; exit {code}\n");
    let run = |code: i32, stderr: &str| {
        let h = Harness::new("rc10", &body(code, stderr));
        containers(&h).state(&h.release, Service::Server)
    };
    assert_eq!(
        run(125, "Error: no such object: \"cortexv2-server\""),
        Ok(ServiceState::Missing)
    );
    // another name in the message, another code, another text: all Failed
    for (code, msg) in [
        (125, "Error: no such object: \"cortexv2-db\""),
        (125, "Error: no such object: \"cortexv2-server\" and more"),
        (125, "Error: something else"),
        (125, ""),
        (1, "Error: no such object: \"cortexv2-server\""),
        (126, "Error: no such object: \"cortexv2-server\""),
        (2, ""),
    ] {
        let r = run(code, msg);
        assert!(
            matches!(r, Err(PortError::Failed(_))),
            "{code} {msg:?}: {r:?}"
        );
    }
    // the same message, but exit 0 with it on stdout, is a reading to be parsed (and refused)
    let h = Harness::new(
        "rc10b",
        "echo 'Error: no such object: \"cortexv2-server\"'\n",
    );
    assert!(matches!(
        containers(&h).state(&h.release, Service::Server),
        Err(PortError::Failed(_))
    ));
}

// ---------------------------------------------------------------- R4 through the wrappers

#[test]
fn rc_11_a_release_without_a_valid_project_name_fails_before_anything_is_started() {
    for compose in [
        "services: {}\n",
        "name: a\nname: b\n",
        "name: Upper\n",
        "name: 9x\n",
        &format!("name: a{}\n", "b".repeat(32)),
    ] {
        let h = Harness::new("rc11", "exit 0\n");
        h.set_compose(compose);
        let c = containers(&h);
        assert!(
            matches!(
                c.state(&h.release, Service::Server),
                Err(PortError::Failed(_))
            ),
            "{compose:?}"
        );
        assert!(
            matches!(
                c.compose_up(&h.release, &[Service::Db]),
                Err(PortError::Failed(_))
            ),
            "{compose:?}"
        );
        assert!(h.calls().is_empty(), "the fake was started for {compose:?}");
    }
}

// ---------------------------------------------------------------- run_roles

const NOT_A_SCRIPT: &str = "the roles script is not an executable regular file";

fn ids(v: &[&str]) -> Vec<ModuleId> {
    v.iter().map(|m| ModuleId::parse(m).unwrap()).collect()
}

/// The wall clock in epoch seconds, as the cells read it.
fn epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// The `--deadline E` pair the roles script was started with, and the module ids after it.
fn roles_args(argv: &[String]) -> (u64, Vec<String>) {
    assert_eq!(argv[0], "--deadline", "{argv:?}");
    assert!(
        !argv[1].is_empty() && argv[1].len() <= 11 && argv[1].bytes().all(|b| b.is_ascii_digit()),
        "{argv:?}"
    );
    (argv[1].parse().unwrap(), argv[2..].to_vec())
}

#[test]
fn rc_12_run_roles_runs_the_script_by_its_absolute_path_with_one_argument_per_module() {
    let h = Harness::new("rc12", "exit 0\n");
    let script = h.roles_script("exit 0\n");
    let before = epoch();
    containers(&h)
        .run_roles(
            &h.release,
            &ids(&["audit", "tokens", "mod_x1"]),
            Duration::from_secs(20),
        )
        .unwrap();
    let after = epoch();
    let calls = h.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].program, script.display().to_string());
    assert!(Path::new(&calls[0].program).is_absolute());
    // `db-roles.sh --deadline <E> <module id>...` (hue's design r1, cox W1-49)
    let (e, mods) = roles_args(&calls[0].argv);
    assert!(
        (before + 20..=after + 20).contains(&e),
        "{e} not in {before}+20..={after}+20"
    );
    assert_eq!(mods, ["audit", "tokens", "mod_x1"]);
}

#[test]
fn rc_13_even_the_usage_refusal_is_unknown() {
    let h = Harness::new("rc13", "exit 0\n");
    h.roles_script("echo 'db-roles: usage: db-roles.sh --deadline <epoch-seconds> <module-id>...' >&2; exit 2\n");
    let timeout = Duration::from_secs(1);
    let begin = Instant::now();
    let result = containers(&h).run_roles(&h.release, &ids(&["audit"]), timeout);
    assert!(matches!(result, Err(PortError::TimedOut)) && begin.elapsed() >= timeout);
}

#[test]
fn rc_37_settled_refusal_table_is_exactly_empty() {
    assert_eq!(
        cortex_upgrade::real::containers::ROLES_SETTLED_REFUSALS,
        &[]
    );
}

#[test]
fn rc_33_signalled_roles_parent_with_live_descendant_is_unknown_until_deadline() {
    let h = Harness::new("rc33", "exit 0\n");
    let started = h.tmp.path().join("descendant-started");
    let finished = h.tmp.path().join("descendant-finished");
    h.roles_script(&format!(
        "( printf started > '{}'; sleep 1; printf finished > '{}' ) </dev/null >/dev/null 2>&1 &\nwhile [ ! -f '{}' ]; do sleep 0.01; done\nkill -KILL $$\n",
        started.display(), finished.display(), started.display()
    ));
    let timeout = Duration::from_secs(3);
    let begin = Instant::now();
    let result = containers(&h).run_roles(&h.release, &ids(&["audit"]), timeout);
    let elapsed = begin.elapsed();
    // Keep the admission lease until the real descendant has finished, even on RED.
    let cleanup_deadline = Instant::now() + Duration::from_secs(3);
    while !finished.exists() && Instant::now() < cleanup_deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        started.exists() && finished.exists(),
        "descendant did not run to completion"
    );
    assert!(
        matches!(result, Err(PortError::TimedOut)) && elapsed >= timeout,
        "signalled parent must be unknown after its full window: {result:?} at {elapsed:?}"
    );
}

#[test]
fn rc_34_unlisted_roles_exit_codes_are_unknown_until_the_deadline() {
    for code in [1, 2, 3, 125, 126, 127, 137] {
        let h = Harness::new("rc34", "exit 0\n");
        h.roles_script(&format!("echo 'unknown outcome' >&2; exit {code}\n"));
        let timeout = Duration::from_secs(1);
        let begin = Instant::now();
        let result = containers(&h).run_roles(&h.release, &ids(&["audit"]), timeout);
        let elapsed = begin.elapsed();
        assert!(
            matches!(result, Err(PortError::TimedOut)) && elapsed >= timeout,
            "exit {code}: expected fenced unknown, got {result:?} at {elapsed:?}"
        );
    }
}

#[test]
fn rc_35_no_error_line_proves_a_started_script_settled() {
    let usage = "db-roles: usage: db-roles.sh --deadline <epoch-seconds> <module-id>...";
    let h = Harness::new("rc35", "exit 0\n");
    h.roles_script(&format!("echo '{usage}' >&2; exit 2\n"));
    let timeout = Duration::from_secs(1);
    let begin = Instant::now();
    let result = containers(&h).run_roles(&h.release, &ids(&["audit"]), timeout);
    assert!(matches!(result, Err(PortError::TimedOut)) && begin.elapsed() >= timeout);
    let bad = [
        "exit 2\n".to_string(),
        format!("printf 'extra\\n%s\\n' '{usage}' >&2; exit 2\n"),
        format!("echo '{usage}' >&2; exit 3\n"),
    ];
    for body in bad {
        let h = Harness::new("rc35bad", "exit 0\n");
        h.roles_script(&body);
        let timeout = Duration::from_secs(1);
        let begin = Instant::now();
        let result = containers(&h).run_roles(&h.release, &ids(&["audit"]), timeout);
        assert!(
            matches!(result, Err(PortError::TimedOut)) && begin.elapsed() >= timeout,
            "incomplete or multiline refusal: {result:?}"
        );
    }
}

#[test]
fn rc_36_cut_roles_output_is_unknown_until_the_deadline() {
    let h = Harness::new("rc36", "exit 0\n");
    h.roles_script("head -c 1048577 /dev/zero; echo 'db-roles: usage: db-roles.sh --deadline <epoch-seconds> <module-id>...' >&2; exit 2\n");
    let timeout = Duration::from_secs(1);
    let begin = Instant::now();
    let result = containers(&h).run_roles(&h.release, &ids(&["audit"]), timeout);
    assert!(
        matches!(result, Err(PortError::TimedOut)) && begin.elapsed() >= timeout,
        "cut stdout must be fenced: {result:?}"
    );
}

#[test]
fn rc_38_success_with_cut_output_is_unknown_until_the_deadline() {
    for redirect in ["", " >&2"] {
        let h = Harness::new("rc38", "exit 0\n");
        h.roles_script(&format!("head -c 1048577 /dev/zero{redirect}; exit 0\n"));
        let timeout = Duration::from_secs(1);
        let begin = Instant::now();
        let result = containers(&h).run_roles(&h.release, &ids(&["audit"]), timeout);
        assert!(
            matches!(result, Err(PortError::TimedOut)) && begin.elapsed() >= timeout,
            "cut output on exit zero must be fenced: {result:?}"
        );
    }
}

#[test]
fn rc_39_forged_usage_line_with_live_descendant_is_unknown() {
    const PID_ENV: &str = "CORTEX_RC39_DESCENDANT_PID_FILE";
    if std::env::var_os(PID_ENV).is_none() {
        // Isolate Linux subreaping to this test's subprocess; the supervisor
        // owns and waits for the orphan without affecting parallel tests.
        let tmp = fakes::common::Tmp::new("rc39-reaper");
        let pid_file = tmp.path().join("descendant-pid");
        let supervisor = r#"
import ctypes, os, subprocess, sys
libc = ctypes.CDLL(None, use_errno=True)
if libc.prctl(36, 1, 0, 0, 0) != 0:
    raise OSError(ctypes.get_errno(), 'PR_SET_CHILD_SUBREAPER')
child = subprocess.Popen([sys.argv[1], '--exact', sys.argv[2], '--nocapture'])
result = child.wait()
with open(os.environ['CORTEX_RC39_DESCENDANT_PID_FILE'], encoding='ascii') as f:
    pid = int(f.read())
reaped, status = os.waitpid(pid, 0)
if reaped != pid or not os.WIFEXITED(status) or os.WEXITSTATUS(status) != 0:
    raise RuntimeError('descendant was not cleanly reaped')
sys.exit(result)
"#;
        let output = std::process::Command::new("/usr/bin/python3")
            .args(["-c", supervisor])
            .arg(std::env::current_exe().unwrap())
            .arg("rc_39_forged_usage_line_with_live_descendant_is_unknown")
            .env(PID_ENV, pid_file)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "isolated forged-refusal cell: {}\n{}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let h = Harness::new("rc39", "exit 0\n");
    let started = h.tmp.path().join("descendant-started");
    let finished = h.tmp.path().join("descendant-finished");
    let pid_file = std::env::var_os(PID_ENV).unwrap();
    h.roles_script(&format!(
        "( printf started > '{}'; sleep 1; printf finished > '{}' ) </dev/null >/dev/null 2>&1 &\nprintf '%s\\n' \"$!\" > '{}'\nwhile [ ! -f '{}' ]; do sleep 0.01; done\nprintf '%s\\n' 'db-roles: usage: db-roles.sh --deadline <epoch-seconds> <module-id>...' >&2\nexit 2\n",
        started.display(), finished.display(), Path::new(&pid_file).display(), started.display()
    ));
    let timeout = Duration::from_secs(3);
    let begin = Instant::now();
    let result = containers(&h).run_roles(&h.release, &ids(&["audit"]), timeout);
    let elapsed = begin.elapsed();
    // Wait for the test's descendant even if this assertion is RED.
    let cleanup_deadline = Instant::now() + Duration::from_secs(3);
    while !finished.exists() && Instant::now() < cleanup_deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(started.exists() && finished.exists());
    assert!(
        matches!(result, Err(PortError::TimedOut)) && elapsed >= timeout,
        "a forged refusal cannot settle a live descendant: {result:?} at {elapsed:?}"
    );
}

#[test]
fn rc_14_the_script_must_be_a_regular_executable_file_and_nothing_is_started_otherwise() {
    let run = |h: &Harness| {
        containers(h).run_roles(&h.release, &ids(&["audit"]), Duration::from_secs(20))
    };
    // missing
    let h = Harness::new("rc14a", "exit 0\n");
    assert_eq!(run(&h), Err(failed(NOT_A_SCRIPT)));
    // not executable: the fixed line, not "could not start"
    let h = Harness::new("rc14b", "exit 0\n");
    let p = h.roles_script("exit 0\n");
    fs::set_permissions(&p, fs::Permissions::from_mode(0o644)).unwrap();
    assert_eq!(
        run(&h),
        Err(failed("the roles script is not an executable regular file"))
    );
    // a symlink, even to an executable file
    let h = Harness::new("rc14c", "exit 0\n");
    let real = h.tmp.path().join("elsewhere.sh");
    fakes::script(&real, &h.log, "exit 0\n");
    symlink(&real, h.release.join("db-roles.sh")).unwrap();
    assert_eq!(run(&h), Err(failed(NOT_A_SCRIPT)));
    // a directory
    let h = Harness::new("rc14d", "exit 0\n");
    fs::create_dir(h.release.join("db-roles.sh")).unwrap();
    assert_eq!(run(&h), Err(failed(NOT_A_SCRIPT)));
}

#[test]
fn rc_15_nothing_is_started_for_a_missing_or_unsafe_script() {
    // The cells above return Failed; this one states that the script was never started (the log stays empty).
    for (tag, make) in [("rc15a", 0u8), ("rc15b", 1), ("rc15c", 2)] {
        let h = Harness::new(tag, "exit 0\n");
        match make {
            0 => {}
            1 => {
                let p = h.roles_script("exit 0\n");
                fs::set_permissions(&p, fs::Permissions::from_mode(0o644)).unwrap();
            }
            _ => {
                let real = h.tmp.path().join("e.sh");
                fakes::script(&real, &h.log, "exit 0\n");
                symlink(&real, h.release.join("db-roles.sh")).unwrap();
            }
        }
        let before = h.calls().len();
        let _ = containers(&h).run_roles(&h.release, &ids(&["audit"]), Duration::from_secs(20));
        assert_eq!(
            h.calls().len(),
            before,
            "{tag}: a script that must not run was started"
        );
    }
}

#[test]
fn rc_16_a_relative_release_directory_is_refused() {
    let h = Harness::new("rc16", "exit 0\n");
    h.roles_script("exit 0\n");
    let r = containers(&h).run_roles(
        Path::new("releases/0.2.0"),
        &ids(&["audit"]),
        Duration::from_secs(20),
    );
    assert_eq!(
        r,
        Err(failed("the release directory is not an absolute path"))
    );
    assert!(h.calls().is_empty());
}

// ---------------------------------------------------------------- R8

#[test]
fn rc_17_a_program_that_cannot_start_is_failed_with_a_fixed_line_that_holds_no_path() {
    let c = RealContainers::with(
        "/nonexistent-dir-for-the-cell/podman",
        quick(),
        fakes::compose_env(Path::new("/h")),
    );
    let t = fakes::common::Tmp::new("rc17");
    fs::write(t.path().join("compose.yml"), "name: cortexv2\n").unwrap();
    let results: Vec<Result<(), PortError>> = vec![
        c.load_image(&t.path().join("a.tar")).map(|_| ()),
        c.compose_up(t.path(), &[Service::Db]),
        c.state(t.path(), Service::Db).map(|_| ()),
    ];
    for r in results {
        assert_eq!(r, Err(failed("could not start podman")));
    }
}

// ---------------------------------------------------------------- cwd, the compose variables, the empty list (hue 18:5xZ)

#[test]
fn rc_18_only_compose_runs_in_the_release_directory_every_other_child_runs_in_the_root() {
    let h = Harness::new(
        "rc18",
        &format!(
            "case \"$1\" in\n load) echo 'Loaded image: localhost/a:1' ;;\n image) echo '{DIGEST}' ;;\n inspect) echo 'cv2-s5-server|running|0|healthy' ;;\nesac\n"
        ),
    );
    h.roles_script("exit 0\n");
    let c = containers(&h);
    c.load_image(&h.tmp.path().join("a.tar")).unwrap();
    c.compose_up(&h.release, &[Service::Db]).unwrap();
    c.state(&h.release, Service::Server).unwrap();
    c.run_roles(&h.release, &ids(&["audit"]), Duration::from_secs(20))
        .unwrap();
    let calls = h.calls();
    assert_eq!(calls.len(), 5);
    let release = h.release.display().to_string();
    for call in &calls {
        let want = if call.argv.first().map(String::as_str) == Some("compose") {
            release.as_str()
        } else {
            "/"
        };
        assert_eq!(call.cwd, want, "{} {:?}", call.program, call.argv);
    }
}

#[test]
fn rc_19_the_compose_child_gets_the_three_variables_from_the_constructor_and_no_other_child_does() {
    let h = Harness::new("rc19", "exit 0\n");
    let c = containers(&h);
    c.compose_up(&h.release, &[Service::Server]).unwrap();
    let call = &h.calls()[0];
    let get = |k: &str| {
        call.env
            .iter()
            .find_map(|l| l.strip_prefix(&format!("{k}=")))
    };
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
    // another home and port give other values
    let other = RealContainers::with(
        h.podman.as_os_str(),
        quick(),
        cortex_upgrade::real::ComposeEnv::new("/srv/other".into(), 9100),
    );
    other.compose_up(&h.release, &[Service::Db]).unwrap();
    let second = &h.calls()[1];
    let get2 = |k: &str| {
        second
            .env
            .iter()
            .find_map(|l| l.strip_prefix(&format!("{k}=")))
    };
    assert_eq!(get2("CORTEX_CONFIG"), Some("/srv/other/cortex.toml"));
    assert_eq!(get2("CORTEX_DUMP_STATUS"), Some("/srv/other/dump-status"));
    assert_eq!(get2("CORTEX_PORT"), Some("9100"));
}

#[test]
fn rc_20_run_roles_with_no_module_ids_runs_nothing() {
    let h = Harness::new("rc20", "exit 0\n");
    h.roles_script("exit 0\n");
    let r = containers(&h).run_roles(&h.release, &[], Duration::from_secs(20));
    assert_eq!(r, Err(failed("no module ids")));
    assert!(
        h.calls().is_empty(),
        "the script was started without an argument"
    );
}

#[test]
fn rc_21_a_relative_release_directory_is_refused_by_compose_up_too() {
    let h = Harness::new("rc21", "exit 0\n");
    let r = containers(&h).compose_up(Path::new("releases/0.2.0"), &[Service::Db]);
    assert_eq!(
        r,
        Err(failed("the release directory is not an absolute path"))
    );
    assert!(h.calls().is_empty());
}

#[test]
fn rc_22_a_bad_home_or_port_is_failed_before_any_child_starts() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;
    let h = Harness::new("rc22", "exit 0\n");
    let bad = |home: &OsStr, port: u16| {
        let c = RealContainers::with(
            h.podman.as_os_str(),
            quick(),
            cortex_upgrade::real::ComposeEnv::new(home.into(), port),
        );
        let r = c.compose_up(&h.release, &[Service::Db]);
        assert_eq!(
            r,
            Err(failed("the compose variables are not valid")),
            "{home:?} {port}"
        );
    };
    bad(OsStr::new("relative/home"), 8601);
    bad(OsStr::new(""), 8601);
    bad(OsStr::new("/h/with\nnewline"), 8601);
    bad(OsStr::new("/h/with\rreturn"), 8601);
    bad(OsStr::from_bytes(b"/h/with\0nul"), 8601);
    bad(OsStr::new("/h/ok"), 0);
    assert!(
        h.calls().is_empty(),
        "a child was started with a bad variable"
    );
    // the edges that are valid
    for (home, port) in [
        ("/h", 1u16),
        ("/srv/cortex home", 65535),
        ("/h/with space", 8601),
    ] {
        let c = RealContainers::with(
            h.podman.as_os_str(),
            quick(),
            cortex_upgrade::real::ComposeEnv::new(home.into(), port),
        );
        c.compose_up(&h.release, &[Service::Db]).unwrap();
    }
    assert_eq!(h.calls().len(), 3);
}

// ---------------------------------------------------------------- output that is not all there is (mike, 6 Nov)
//
// The runner keeps the first MiB of each stream and says when that is not all the child wrote. A wrapper that reads a
// stream as a whole answer must refuse the part. The cells build output whose FIRST MiB is a valid answer and whose
// rest is something else, so only the flag tells the two apart.

/// A shell snippet printing `valid` (one line), then blank lines up to exactly 1 MiB in all, then `junk`.
fn mib_then(valid: &str, junk: &str) -> String {
    let n = (1usize << 20) - valid.len();
    format!(
        "printf '%b' '{}'; head -c {n} /dev/zero | tr '\\0' '\\n'; printf '%b' '{}'",
        valid.replace('\n', "\\n"),
        junk.replace('\n', "\\n"),
    )
}

#[test]
fn rc_23_a_load_whose_output_is_cut_or_open_is_refused_not_read_as_one_image() {
    let first = "Loaded image: localhost/a:1\n";
    let digest_line = format!("{DIGEST}\\n");
    let podman = |load: &str| {
        format!("case \"$1\" in\n load) {load} ;;\n image) printf '%b' '{digest_line}' ;;\nesac\n")
    };
    // mike's case: one valid line in the first MiB, a SECOND image after the cap
    let h = Harness::new(
        "rc23a",
        &podman(&mib_then(first, "Loaded image: localhost/b:2\n")),
    );
    let r = containers(&h).load_image(&h.tmp.path().join("a.tar"));
    assert!(matches!(r, Err(PortError::Failed(_))), "{r:?}");
    assert_eq!(h.calls().len(), 1, "image inspect must not run");
    // even blank lines after the cap: not all of the output is known
    let h = Harness::new("rc23b", &podman(&mib_then(first, "\n\n\n")));
    let r = containers(&h).load_image(&h.tmp.path().join("a.tar"));
    assert!(matches!(r, Err(PortError::Failed(_))), "{r:?}");
    assert_eq!(h.calls().len(), 1);
    // control: the same bytes with NOTHING after the cap are all there is
    let h = Harness::new("rc23c", &podman(&mib_then(first, "")));
    assert_eq!(
        containers(&h).load_image(&h.tmp.path().join("a.tar")),
        Ok(digest(DIGEST))
    );
    // an output stream still open one second after podman ended (something it started holds it): refused
    let h = Harness::new(
        "rc23d",
        &podman(
            "echo 'Loaded image: localhost/a:1'; ( sleep 4; echo 'Loaded image: localhost/b:2' ) &",
        ),
    );
    let t = std::time::Instant::now();
    let r = containers(&h).load_image(&h.tmp.path().join("a.tar"));
    assert!(matches!(r, Err(PortError::Failed(_))), "{r:?}");
    assert!(
        t.elapsed() < Duration::from_millis(3500),
        "{:?}",
        t.elapsed()
    );
    assert_eq!(h.calls().len(), 1);
}

#[test]
fn rc_24_a_digest_whose_output_is_cut_or_open_is_no_digest() {
    let ok_load = "printf 'Loaded image: localhost/a:1\\n'";
    let podman = |inspect: &str| {
        format!("case \"$1\" in\n load) {ok_load} ;;\n image) {inspect} ;;\nesac\n")
    };
    let line = format!("{DIGEST}\n");
    let h = Harness::new("rc24a", &podman(&mib_then(&line, "junk\n")));
    assert_eq!(
        containers(&h).load_image(&h.tmp.path().join("a.tar")),
        Err(failed("the loaded image has no digest"))
    );
    // control: exactly 1 MiB and nothing after it
    let h = Harness::new("rc24b", &podman(&mib_then(&line, "")));
    assert_eq!(
        containers(&h).load_image(&h.tmp.path().join("a.tar")),
        Ok(digest(DIGEST))
    );
    // an open stream at the grace
    let h = Harness::new(
        "rc24c",
        &podman(&format!(
            "echo '{DIGEST}'; ( sleep 4; echo 'sha256:{}' ) &",
            "0".repeat(64)
        )),
    );
    assert_eq!(
        containers(&h).load_image(&h.tmp.path().join("a.tar")),
        Err(failed("the loaded image has no digest"))
    );
}

#[test]
fn rc_25_a_reading_whose_output_is_cut_or_open_is_failed() {
    let name = "cortexv2-server|running|0|healthy\n";
    let state = |body: &str| {
        let h = Harness::new("rc25", body);
        let r = containers(&h).state(&h.release, Service::Server);
        (r, h.calls().len())
    };
    let (r, _) = state(&mib_then(name, "extra\n"));
    assert!(matches!(r, Err(PortError::Failed(_))), "{r:?}");
    let (r, _) = state(&mib_then(name, "\n"));
    assert!(matches!(r, Err(PortError::Failed(_))), "{r:?}");
    // control: exactly 1 MiB and nothing after it
    let (r, _) = state(&mib_then(name, ""));
    assert_eq!(r, Ok(ServiceState::Healthy));
    // an open stream at the grace
    let (r, _) = state(
        "echo 'cortexv2-server|running|0|healthy'; ( sleep 4; echo 'cortexv2-server|exited|1|' ) &\n",
    );
    assert!(matches!(r, Err(PortError::Failed(_))), "{r:?}");
}

#[test]
fn rc_26_the_roles_script_runs_only_for_the_project_it_targets() {
    // db-roles.sh reaches `cv2-s5-db` unless CORTEX_DB_CONTAINER names another, and the wrapper never sets it (R2: four
    // names): a release of another project would have its roles applied to the wrong database.
    let ids =
        |v: &[&str]| -> Vec<ModuleId> { v.iter().map(|s| ModuleId::parse(s).unwrap()).collect() };
    for compose in [
        "name: cortexv2\n",
        "name: cv2-s5x\n",
        "name: xcv2-s5\n",
        "name: cv2-s\n",
        "name: cv2-s6\n",
        "name: cv2-s5-db\n",
        "name: cv2\n",
        "name: cv2-s5-\n",
        "services: {}\n",
        "name: Cv2-s5\n",
    ] {
        let h = Harness::new("rc26", "exit 0\n");
        h.roles_script("exit 0\n");
        h.set_compose(compose);
        let r = containers(&h).run_roles(&h.release, &ids(&["audit"]), Duration::from_secs(20));
        assert!(matches!(r, Err(PortError::Failed(_))), "{compose:?}: {r:?}");
        assert!(
            h.calls().is_empty(),
            "the script was started for {compose:?}"
        );
    }
    // the refusal for another project is its own fixed line, and it holds no name
    let h = Harness::new("rc26b", "exit 0\n");
    h.roles_script("exit 0\n");
    h.set_compose("name: cortexv2\n");
    assert_eq!(
        containers(&h).run_roles(&h.release, &ids(&["audit"]), Duration::from_secs(20)),
        Err(failed(
            "the release is not of the project the roles script targets"
        ))
    );
    // the one project it targets runs
    let h = Harness::new("rc26c", "exit 0\n");
    h.roles_script("exit 0\n");
    assert_eq!(
        containers(&h).run_roles(&h.release, &ids(&["audit"]), Duration::from_secs(20)),
        Ok(())
    );
    assert_eq!(h.calls().len(), 1);
}

// ---------------------------------------------------------------- R3: command positions, not data (hue, brief r3)
//
// R3 constrains the COMMAND and OPERATOR positions of a vector (the subcommands and flags). A value position is data
// and is held to its OWN shape: a ModuleId may spell `kill` or `secret`, and a release whose roles step is refused for
// that would be refused for a valid manifest. The allow-list helper is the one place that holds the rule.

#[test]
fn rc_27_module_ids_that_spell_a_forbidden_word_are_data_and_are_passed_as_they_are() {
    let words = [
        "prune", "rm", "kill", "stop", "down", "volume", "system", "network", "secret", "pod",
        "image",
    ];
    let mods: Vec<ModuleId> = words
        .iter()
        .map(|w| ModuleId::parse(w).unwrap_or_else(|e| panic!("{w}: {e:?}")))
        .collect();
    let h = Harness::new("rc27", "exit 0\n");
    let script = h.roles_script("exit 0\n");
    assert_eq!(
        containers(&h).run_roles(&h.release, &mods, Duration::from_secs(20)),
        Ok(())
    );
    let calls = h.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].program, script.display().to_string());
    let (_, mods) = roles_args(&calls[0].argv);
    assert_eq!(mods, words);
    // (the Harness checks the vector against the allow-list when it is dropped: it must pass)
}

#[test]
fn rc_28_the_allow_list_helper_refuses_commands_and_globs_and_accepts_data() {
    let h = Harness::new(
        "rc28",
        &podman_load("Loaded image: localhost/a:1\\n", &format!("{DIGEST}\\n")),
    );
    containers(&h)
        .load_image(&h.tmp.path().join("a.tar"))
        .unwrap();
    let base = h.calls()[0].clone(); // a genuine call: its environment is the right one
    let roles = h.release.join("db-roles.sh").display().to_string();
    let (rel, home) = (h.release.clone(), h.home.clone());
    let accepts = |program: &str, argv: &[&str]| -> bool {
        let mut c = base.clone();
        c.program = program.to_string();
        c.argv = argv.iter().map(|s| s.to_string()).collect();
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            fakes::check_closed(&c, &home, &rel)
        }))
        .is_ok()
    };
    let podman = "/x/podman";
    let compose = h.release.join("compose.yml").display().to_string();
    let stack = h.release.join("stack.env").display().to_string();
    // data is accepted
    for argv in [
        vec!["load", "--input", "/home/u/workspace/x/a.tar"],
        vec!["load", "--input", "/tmp/x_y.1-2+3/a.tar"],
    ] {
        assert!(accepts(podman, &argv), "{argv:?}");
    }
    let dl = ["--deadline", "1793000000"];
    let with = |ids: &[&str]| -> Vec<String> {
        dl.iter().chain(ids.iter()).map(|s| s.to_string()).collect()
    };
    let acc = |v: Vec<String>| accepts(&roles, &v.iter().map(String::as_str).collect::<Vec<_>>());
    assert!(acc(with(&["kill", "secret", "down", "image"])));
    assert!(acc(with(&["audit"])));
    // a glob character, a space, a shell character or a relative path is not an archive path our code can produce
    for path in [
        "/tmp/a*b.tar",
        "/tmp/a?b.tar",
        "/tmp/a[1].tar",
        "a.tar",
        "/tmp/a b.tar",
        "/tmp/a;b.tar",
        "/tmp/$x.tar",
        "/tmp/a\nb.tar",
    ] {
        assert!(!accepts(podman, &["load", "--input", path]), "{path:?}");
    }
    // a command or operator position is never a forbidden word
    for argv in [
        vec!["rm", "-f", "cortexv2-db"],
        vec!["system", "prune", "--all"],
        vec!["image", "prune"],
        vec!["kill", "cortexv2-db"],
        vec!["stop", "cortexv2-db"],
        vec!["volume", "rm", "x"],
        vec!["compose", &compose, "--env-file", &stack, "down", "--all"],
        vec!["inspect", "--format", "{{.Name}}", "cortexv2-*"],
    ] {
        assert!(!accepts(podman, &argv), "{argv:?}");
    }
    // the shapes of the values
    assert!(!accepts(&roles, &[]));
    assert!(!acc(with(&[])));
    assert!(!acc(with(&["Audit"])));
    assert!(!acc(with(&["a;b"])));
    assert!(!acc(with(&["*"])));
    assert!(!acc(with(&[&"a".repeat(33)])));
    assert!(acc(with(&[&"a".repeat(32)])));
    // the deadline: required, first, digits only, at most 11
    assert!(!accepts(&roles, &["audit"]));
    assert!(!accepts(&roles, &["kill", "secret"]));
    for e in [
        "",
        "abc",
        "-1",
        "1e9",
        "12345678901x",
        "123456789012",
        " 1",
        "1793000000 ",
    ] {
        assert!(!accepts(&roles, &["--deadline", e, "audit"]), "{e:?}");
    }
    assert!(accepts(&roles, &["--deadline", "99999999999", "audit"]));
    // no leading zero (hue's script: digits only, no leading zero, at most 11): `0` is a leading zero of nothing, and E = 0 is
    // a deadline in 1970
    for e in ["0", "00", "007", "01793000000"] {
        assert!(!accepts(&roles, &["--deadline", e, "audit"]), "{e:?}");
    }
    assert!(accepts(&roles, &["--deadline", "1", "audit"]));
    assert!(!accepts(&roles, &["audit", "--deadline", "1793000000"]));
    assert!(!accepts(&roles, &["--dead-line", "1793000000", "audit"]));
}

// ---------------------------------------------------------------- the roles script's deadline (hue's design r1, cox W1-49)
//
// The script refuses to start its transaction late: it is given the engine's ABSOLUTE deadline E in epoch seconds
// (`db-roles.sh --deadline <E> <module id>...`). The adapter computes it, once, at the moment it launches the script:
// E = the wall clock in whole seconds + the port's `timeout` in whole seconds (both rounded down, so E is never later
// than the real deadline). The frozen port is unchanged.

#[test]
fn rc_29_the_deadline_is_the_wall_clock_at_the_launch_plus_the_timeout() {
    for secs in [1u64, 20, 45, 60, 900, 86_400] {
        let h = Harness::new("rc29", "exit 0\n");
        h.roles_script("exit 0\n");
        let before = epoch();
        containers(&h)
            .run_roles(&h.release, &ids(&["audit"]), Duration::from_secs(secs))
            .unwrap();
        let after = epoch();
        let (e, mods) = roles_args(&h.calls()[0].argv);
        assert!(
            (before + secs..=after + secs).contains(&e),
            "timeout {secs}: {e} is not in {before}+{secs}..={after}+{secs}"
        );
        assert_eq!(mods, ["audit"]);
    }
    // a fraction of a second is not counted (E is never later than the real deadline)
    let h = Harness::new("rc29b", "exit 0\n");
    h.roles_script("exit 0\n");
    let before = epoch();
    containers(&h)
        .run_roles(&h.release, &ids(&["audit"]), Duration::from_millis(45_999))
        .unwrap();
    let after = epoch();
    let (e, _) = roles_args(&h.calls()[0].argv);
    assert!((before + 45..=after + 45).contains(&e), "{e}");
    // E is computed when the script is LAUNCHED, not before the checks: a call that is refused starts nothing
    let h = Harness::new("rc29c", "exit 0\n");
    h.roles_script("exit 0\n");
    h.set_compose("name: cortexv2\n");
    let r = containers(&h).run_roles(&h.release, &ids(&["audit"]), Duration::from_secs(45));
    assert!(matches!(r, Err(PortError::Failed(_))), "{r:?}");
    assert!(h.calls().is_empty());
}

#[test]
fn rc_30_a_deadline_the_script_cannot_take_is_failed_before_anything_starts() {
    // the script takes digits only, at most 11 (up to 99_999_999_999): a timeout that goes past it is refused, not wrapped
    // or cut. 100 years is far over the engine's own bounds and still fits; the cell uses values that cannot.
    for t in [
        Duration::from_secs(u64::MAX),
        Duration::from_secs(u64::MAX / 2),
        Duration::MAX,
        Duration::from_secs(99_999_999_999),
    ] {
        let h = Harness::new("rc30", "exit 0\n");
        h.roles_script("exit 0\n");
        let r = containers(&h).run_roles(&h.release, &ids(&["audit"]), t);
        assert_eq!(
            r,
            Err(failed("the roles deadline is out of range")),
            "{t:?}"
        );
        assert!(h.calls().is_empty(), "the script was started for {t:?}");
    }
    // the largest that fits is accepted: E has 11 digits
    let h = Harness::new("rc30b", "exit 0\n");
    h.roles_script("exit 0\n");
    let r = containers(&h).run_roles(
        &h.release,
        &ids(&["audit"]),
        Duration::from_secs(99_999_999_999 - epoch() - 5),
    );
    assert_eq!(r, Ok(()));
    let (e, _) = roles_args(&h.calls()[0].argv);
    assert!(e <= 99_999_999_999 && e.to_string().len() <= 11, "{e}");
}

#[test]
fn rc_31_the_roles_deadline_is_whole_seconds_now_plus_timeout_and_at_most_11_digits() {
    use cortex_upgrade::real::env::{MAX_ROLES_DEADLINE, roles_deadline};
    const CAP: u64 = 99_999_999_999;
    assert_eq!(MAX_ROLES_DEADLINE, CAP);
    assert_eq!(CAP.to_string().len(), 11);
    let now = 1_793_000_000;
    assert_eq!(roles_deadline(now, Duration::from_secs(45)), Some(now + 45));
    assert_eq!(roles_deadline(now, Duration::ZERO), Some(now));
    // E = 0 is refused (a deadline in 1970; the script takes no leading zero and `0` is one), E = 1 is the smallest
    assert_eq!(roles_deadline(0, Duration::ZERO), None);
    assert_eq!(roles_deadline(0, Duration::from_millis(999)), None);
    assert_eq!(roles_deadline(0, Duration::from_secs(1)), Some(1));
    assert_eq!(roles_deadline(1, Duration::ZERO), Some(1));
    // a fraction of a second is dropped, never rounded up (E is never later than the real deadline)
    assert_eq!(
        roles_deadline(now, Duration::from_millis(45_999)),
        Some(now + 45)
    );
    assert_eq!(roles_deadline(now, Duration::from_millis(999)), Some(now));
    // the exact boundary: E = CAP is taken, E = CAP + 1 is not
    assert_eq!(roles_deadline(CAP - 45, Duration::from_secs(45)), Some(CAP));
    assert_eq!(roles_deadline(CAP - 45, Duration::from_secs(46)), None);
    assert_eq!(roles_deadline(CAP, Duration::ZERO), Some(CAP));
    assert_eq!(roles_deadline(CAP + 1, Duration::ZERO), None);
    assert_eq!(roles_deadline(CAP, Duration::from_millis(999)), Some(CAP));
    // an overflow is not wrapped
    assert_eq!(roles_deadline(1, Duration::from_secs(u64::MAX)), None);
    assert_eq!(roles_deadline(u64::MAX, Duration::from_secs(1)), None);
    assert_eq!(roles_deadline(u64::MAX, Duration::ZERO), None);
    assert_eq!(roles_deadline(now, Duration::MAX), None);
}

// ---------------------------------------------------------------- vera's UPREAL-L3: Missing only from a stderr that is all there
//
// `state()` reads `Missing` from exit 125 and the exact `Error: no such object: "<name>"` as the LAST line of stderr. A stderr that
// was cut at the cap, or is still open at the grace, may hide a later line; the last line of the kept part is then not the last line.

#[test]
fn rc_32_missing_is_read_only_from_a_stderr_that_is_all_there() {
    let msg = "Error: no such object: \"cortexv2-server\"";
    let state = |body: &str| {
        let h = Harness::new("rc32", body);
        containers(&h).state(&h.release, Service::Server)
    };
    // control: the exact message, nothing else, exit 125: Missing
    assert_eq!(
        state(&format!("echo '{msg}' >&2; exit 125\n")),
        Ok(ServiceState::Missing)
    );
    // the message is the last line of the FIRST MiB and the child wrote more after the cap: not Missing
    let n = (1usize << 20) - (msg.len() + 1);
    let r = state(&format!(
        "echo '{msg}' >&2; head -c {n} /dev/zero | tr '\\0' '\\n' >&2; echo 'another error' >&2; exit 125\n"
    ));
    assert!(matches!(r, Err(PortError::Failed(_))), "{r:?}");
    // exactly 1 MiB and nothing after it is all there is: Missing
    assert_eq!(
        state(&format!(
            "echo '{msg}' >&2; head -c {n} /dev/zero | tr '\\0' '\\n' >&2; exit 125\n"
        )),
        Ok(ServiceState::Missing)
    );
    // a stream still open one second after podman ended (something it started holds it): not Missing
    let r = state(&format!(
        "echo '{msg}' >&2; ( sleep 4; echo 'another error' >&2 ) &\nexit 125\n"
    ));
    assert!(matches!(r, Err(PortError::Failed(_))), "{r:?}");
}
