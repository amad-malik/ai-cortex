//! Piece 14, real ports: R6 (a deadline for every call, and only the child's own pid is ended) and R7 (bounded output,
//! no deadlock on a chatty child), plus the runner itself (`real::run::run`).
#[path = "real_fakes.rs"]
mod fakes;

use cortex_types::ModuleId;
use cortex_upgrade::ports::{Containers, DumpLabel, Dumper, OneLine, PortError, Psql, Service};
use cortex_upgrade::real::run::{RunError, run};
use cortex_upgrade::real::{Deadlines, RealContainers, RealDumper, RealPsql};
use cortex_upgrade::version::parse_version;
use fakes::Harness;
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn label() -> DumpLabel {
    DumpLabel::new(
        &parse_version("0.2.0").unwrap(),
        &parse_version("0.3.0").unwrap(),
    )
}

fn alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

/// A sleeping fake: records its own pid and the pid of a child it started, then waits for it.
const SLEEPER: &str = "echo $$ > \"$(dirname \"$0\")/pid\"\nsleep 60 &\necho $! > \"$(dirname \"$0\")/grandchild\"\nwait\n";

fn read_pid(path: &Path) -> u32 {
    fs::read_to_string(path).unwrap().trim().parse().unwrap()
}

/// Runs `call` (which must start the fake at `dir`/`pid` file) and checks R6.
fn check_deadline(dir: &Path, bound: Duration, call: impl FnOnce() -> bool) {
    // a process started by the TEST itself, which must survive
    let mut bystander = Command::new("sleep")
        .arg("60")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let t = Instant::now();
    let timed_out = call();
    let took = t.elapsed();
    let pid = read_pid(&dir.join("pid"));
    let child_alive = alive(pid);
    let bystander_alive = bystander.try_wait().unwrap().is_none();
    let grand = read_pid(&dir.join("grandchild"));
    let grand_alive = alive(grand);
    // Clean up the exact processes this test started before assertions can panic on a mutant.
    if child_alive {
        let _ = Command::new("kill").arg(pid.to_string()).status();
    }
    if grand_alive {
        let _ = Command::new("kill").arg(grand.to_string()).status();
    }
    if bystander_alive {
        let _ = bystander.kill();
    }
    let _ = bystander.wait();
    assert!(timed_out, "the call did not end as a timeout");
    assert!(
        took < bound + Duration::from_secs(1),
        "took {took:?} for a bound of {bound:?}"
    );
    assert!(took >= bound, "ended before its deadline: {took:?}");
    assert!(!child_alive, "the child {pid} was not ended");
    assert!(bystander_alive, "a process of the test was ended");
    // Only the child's own pid: what the child itself started is not ours to end (no group, no session, no pkill).
    assert!(
        grand_alive,
        "the grandchild {grand} was ended: a process group or a name was signalled"
    );
}

#[test]
fn rl_01_every_call_ends_at_its_deadline_and_only_the_childs_own_pid_is_ended() {
    let d = Deadlines {
        load: Duration::from_millis(700),
        up: Duration::from_millis(700),
        inspect: Duration::from_millis(700),
    };
    // Containers: load, up, state (the fake podman sleeps whatever it is asked)
    for which in 0..3 {
        let h = Harness::new("rl1", SLEEPER);
        let c = RealContainers::with(h.podman.as_os_str(), d, fakes::compose_env(&h.home));
        let dir = h.podman.parent().unwrap().to_path_buf();
        check_deadline(&dir, Duration::from_millis(700), || match which {
            0 => c.load_image(&h.tmp.path().join("a.tar")) == Err(PortError::TimedOut),
            1 => c.compose_up(&h.release, &[Service::Db]) == Err(PortError::TimedOut),
            _ => c.state(&h.release, Service::Db) == Err(PortError::TimedOut),
        });
    }
    // run_roles: the deadline is the caller's
    let h = Harness::new("rl1r", "exit 0\n");
    let script = h.roles_script(SLEEPER);
    let c = RealContainers::with(h.podman.as_os_str(), d, fakes::compose_env(&h.home));
    check_deadline(script.parent().unwrap(), Duration::from_millis(900), || {
        c.run_roles(
            &h.release,
            &[ModuleId::parse("audit").unwrap()],
            Duration::from_millis(900),
        ) == Err(PortError::TimedOut)
    });
    // Psql: no answer
    let h = Harness::new("rl1p", SLEEPER);
    let dir = h.podman.parent().unwrap().to_path_buf();
    let p = RealPsql::with(
        h.podman.as_os_str(),
        Duration::from_millis(700),
        h.release.clone(),
    );
    check_deadline(&dir, Duration::from_millis(700), || {
        p.trust_rule_count().is_err()
    });
    // Dumper: timeout plus the grace
    let h = Harness::new("rl1d", "exit 0\n");
    let tool = h.dump_tool(SLEEPER);
    let dumper = RealDumper::with_grace(
        h.home.clone(),
        h.release.clone(),
        Duration::from_millis(400),
    );
    check_deadline(tool.parent().unwrap(), Duration::from_millis(1400), || {
        dumper.dump(&label(), Duration::from_secs(1)) == Err(PortError::TimedOut)
    });
}

#[test]
fn rl_02_output_is_read_at_most_1_mib_per_stream_and_a_chatty_child_does_not_block() {
    // 5 MiB to stderr first, then 5 MiB to stdout: a reader that takes the streams one after the other deadlocks.
    let body = "head -c 5242880 /dev/zero | tr '\\0' x >&2\nhead -c 5242880 /dev/zero | tr '\\0' y\nexit 3\n";
    let (_t, fake, _log) = fakes::standalone("rl2s", body);
    let h = Harness::new("rl2", body);
    let t = Instant::now();
    let args: Vec<OsString> = vec![];
    let o = run(fake.as_os_str(), &args, Duration::from_secs(20)).unwrap();
    assert!(t.elapsed() < Duration::from_secs(15), "{:?}", t.elapsed());
    assert_eq!(o.code, Some(3));
    assert_eq!(
        o.stdout.len(),
        1 << 20,
        "exactly the first MiB of stdout is kept"
    );
    assert_eq!(
        o.stderr.len(),
        1 << 20,
        "exactly the first MiB of stderr is kept"
    );
    assert!(o.stdout.iter().all(|b| *b == b'y') && o.stderr.iter().all(|b| *b == b'x'));
    // Through a wrapper: the last line is bounded to 200 characters and three dots.
    let c = RealContainers::with(
        h.podman.as_os_str(),
        Deadlines {
            up: Duration::from_secs(60),
            ..Deadlines::default()
        },
        fakes::compose_env(&h.home),
    );
    let r = c.compose_up(&h.release, &[Service::Db]);
    let Err(PortError::Failed(l)) = r else {
        panic!("{r:?}")
    };
    assert_eq!(l.as_str(), OneLine::new(&"x".repeat(1 << 20)).as_str());
    assert_eq!(l.as_str().len(), 203);
    // Psql: 5 MiB of one line is no answer (never shortened); Dumper: a stdout line over 4096 bytes is None.
    let h = Harness::new("rl2b", "head -c 5242880 /dev/zero | tr '\\0' z\nexit 0\n");
    let p = RealPsql::with(
        h.podman.as_os_str(),
        Duration::from_secs(60),
        h.release.clone(),
    );
    assert_eq!(p.trust_rule_count(), Err(cortex_upgrade::ports::NoAnswer));
    h.dump_tool("head -c 5242880 /dev/zero | tr '\\0' z\nhead -c 5242880 /dev/zero | tr '\\0' q >&2\nexit 0\n");
    let r = RealDumper::new(h.home.clone(), h.release.clone())
        .dump(&label(), Duration::from_secs(60))
        .unwrap();
    assert_eq!(r.last_stdout_line, None);
    assert_eq!(r.last_stderr_line.as_str().len(), 203);
}

#[test]
fn rl_03_the_last_line_comes_from_the_kept_first_mib_and_the_rest_is_discarded() {
    // 1.5 MiB of numbered lines on stdout: the last kept line is somewhere in the first MiB, not the real last line,
    // and the child still ends normally (the rest was read and dropped).
    let (_t, fake, _log) = fakes::standalone(
        "rl3",
        "i=0; while [ $i -lt 120000 ]; do echo \"line $i\"; i=$((i+1)); done\n",
    );
    let args: Vec<OsString> = vec![];
    let o = run(fake.as_os_str(), &args, Duration::from_secs(60)).unwrap();
    assert_eq!(o.code, Some(0));
    assert_eq!(o.stdout.len(), 1 << 20);
    assert!(!o.stdout.ends_with(b"line 119999\n"));
}

#[test]
fn rl_06_an_outcome_says_when_the_kept_bytes_are_not_all_the_child_wrote() {
    let none: Vec<OsString> = vec![];
    // a normal child: nothing is cut
    let (_t, fake, _l) = fakes::standalone("rl6a", "echo out\necho err >&2\n");
    let o = run(fake.as_os_str(), &none, Duration::from_secs(20)).unwrap();
    assert_eq!((o.stdout_cut, o.stderr_cut), (false, false), "{o:?}");
    // exactly 1 MiB is all of it; one byte more is cut
    let (_t, fake, _l) = fakes::standalone("rl6b", "head -c 1048576 /dev/zero | tr '\\0' x\n");
    let o = run(fake.as_os_str(), &none, Duration::from_secs(20)).unwrap();
    assert_eq!((o.stdout.len(), o.stdout_cut), (1 << 20, false));
    let (_t, fake, _l) = fakes::standalone("rl6c", "head -c 1048577 /dev/zero | tr '\\0' x\n");
    let o = run(fake.as_os_str(), &none, Duration::from_secs(20)).unwrap();
    assert_eq!((o.stdout.len(), o.stdout_cut), (1 << 20, true));
    // a child that ends while something it started still holds its output open: after the 1 s the readers get, the
    // bytes kept are not known to be all of it, and the outcome says so (stdout and stderr share the holder here)
    let (_t, fake, _l) =
        fakes::standalone("rl6d", "echo first\n( sleep 4; echo late ) &\nexit 0\n");
    let t = Instant::now();
    let o = run(fake.as_os_str(), &none, Duration::from_secs(20)).unwrap();
    assert!(
        t.elapsed() < Duration::from_millis(1800),
        "the call waited longer than the one second the readers get: {:?}",
        t.elapsed()
    );
    assert_eq!(o.code, Some(0));
    assert_eq!(o.stdout, b"first\n");
    assert!(
        o.stdout_cut && o.stderr_cut,
        "an open stream at the grace must be reported: {o:?}"
    );
    // control: the same holder with its output closed is not an open stream
    let (_t, fake, _l) = fakes::standalone(
        "rl6e",
        "echo first\n( sleep 4 ) >/dev/null 2>&1 &\nexit 0\n",
    );
    let t = Instant::now();
    let o = run(fake.as_os_str(), &none, Duration::from_secs(20)).unwrap();
    assert!(
        t.elapsed() < Duration::from_millis(1500),
        "{:?}",
        t.elapsed()
    );
    assert_eq!((o.stdout_cut, o.stderr_cut), (false, false), "{o:?}");
}

#[test]
fn rl_04_the_runner_reports_codes_signals_and_programs_that_cannot_start() {
    let none: Vec<OsString> = vec![];
    let (_t, fake, _l) = fakes::standalone("rl4", "exit 5\n");
    assert_eq!(
        run(fake.as_os_str(), &none, Duration::from_secs(20))
            .unwrap()
            .code,
        Some(5)
    );
    let (_t2, fake, _l) = fakes::standalone("rl4b", "kill -9 $$\n");
    assert_eq!(
        run(fake.as_os_str(), &none, Duration::from_secs(20))
            .unwrap()
            .code,
        None
    );
    assert_eq!(
        run(
            Path::new("/nonexistent-dir-for-the-cell/x").as_os_str(),
            &none,
            Duration::from_secs(5)
        ),
        Err(RunError::NotStarted)
    );
    // a file that is not executable
    let (t3, _fake, _l) = fakes::standalone("rl4c", "exit 0\n");
    let f = t3.path().join("plain");
    fs::write(&f, "x").unwrap();
    assert_eq!(
        run(f.as_os_str(), &none, Duration::from_secs(5)),
        Err(RunError::NotStarted)
    );
    // the arguments reach the child whole, one element each
    let args: Vec<OsString> = ["a b", "c;d", "$(x)", "*", "--all"]
        .iter()
        .map(OsString::from)
        .collect();
    let (_t4, fake, log) = fakes::standalone("rl4d", "exit 0\n");
    run(fake.as_os_str(), &args, Duration::from_secs(20)).unwrap();
    assert_eq!(
        fakes::calls(&log)[0].argv,
        ["a b", "c;d", "$(x)", "*", "--all"]
    );
    // the child has no input: it reads end-of-file at once
    let (_t5, fake, _l) = fakes::standalone("rl4e", "cat > /dev/null; exit 0\n");
    assert_eq!(
        run(fake.as_os_str(), &none, Duration::from_secs(5))
            .unwrap()
            .code,
        Some(0)
    );
}

#[test]
fn rl_05_the_child_starts_in_the_root_directory_with_no_input() {
    let (t, fake, _log) = fakes::standalone("rl5", "pwd > \"$(dirname \"$0\")/cwd\"\nexit 0\n");
    let none: Vec<OsString> = vec![];
    run(fake.as_os_str(), &none, Duration::from_secs(20)).unwrap();
    assert_eq!(
        fs::read_to_string(t.path().join("cwd")).unwrap().trim(),
        "/"
    );
}

/// vera's UPREAL-V2 through the wrappers: a `compose.yml` that is a FIFO with no writer must be a prompt refusal, and
/// no program may be started for it.
#[test]
fn rl_07_a_compose_that_is_a_fifo_with_no_writer_is_a_prompt_refusal_and_nothing_is_started() {
    use std::sync::mpsc;
    let h = Harness::new("rl7", "exit 0\n");
    h.roles_script("exit 0\n");
    h.dump_tool("exit 0\n");
    let fifo = h.release.join("compose.yml");
    fs::remove_file(&fifo).unwrap();
    assert!(
        Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    fn bounded<T: Send + 'static>(
        fifo: &Path,
        what: &str,
        f: impl FnOnce() -> T + Send + 'static,
    ) -> T {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(v) => v,
            Err(_) => {
                // let the blocked open go, then fail
                let _ = fs::OpenOptions::new().write(true).open(fifo);
                panic!("{what} blocked on a FIFO compose.yml with no writer");
            }
        }
    }
    let (podman, release, home) = (h.podman.clone(), h.release.clone(), h.home.clone());
    let p = (podman.clone(), release.clone());
    let r = bounded(&fifo, "Psql", move || {
        RealPsql::with(p.0.as_os_str(), Duration::from_secs(20), p.1).trust_rule_count()
    });
    assert_eq!(r, Err(cortex_upgrade::ports::NoAnswer));
    let (rel, hm) = (release.clone(), home.clone());
    let r = bounded(&fifo, "Dumper", move || {
        RealDumper::new(hm, rel).dump(&label(), Duration::from_secs(5))
    });
    assert!(matches!(r, Err(PortError::Failed(_))), "{r:?}");
    let (pd, rel, hm) = (podman.clone(), release.clone(), home.clone());
    let r = bounded(&fifo, "state", move || {
        RealContainers::with(
            pd.as_os_str(),
            Deadlines::default(),
            fakes::compose_env(&hm),
        )
        .state(&rel, Service::Db)
    });
    assert!(matches!(r, Err(PortError::Failed(_))), "{r:?}");
    let (pd, rel, hm) = (podman.clone(), release.clone(), home.clone());
    let r = bounded(&fifo, "compose_up", move || {
        RealContainers::with(
            pd.as_os_str(),
            Deadlines::default(),
            fakes::compose_env(&hm),
        )
        .compose_up(&rel, &[Service::Db])
    });
    assert!(matches!(r, Err(PortError::Failed(_))), "{r:?}");
    let (pd, rel, hm) = (podman, release, home);
    let r = bounded(&fifo, "run_roles", move || {
        RealContainers::with(
            pd.as_os_str(),
            Deadlines::default(),
            fakes::compose_env(&hm),
        )
        .run_roles(
            &rel,
            &[ModuleId::parse("audit").unwrap()],
            Duration::from_secs(5),
        )
    });
    assert!(matches!(r, Err(PortError::Failed(_))), "{r:?}");
    assert!(
        h.calls().is_empty(),
        "a program was started for a FIFO compose.yml"
    );
}
