//! The one place a child program is started (R1, R2, R6, R7, R8).
//!
//! - **R1** `Command::new(program).args(args)`: an argument vector, never a shell and never a string split later; the
//!   program is whatever the caller passes (the wrappers pass only their three fixed programs).
//! - **R2** `env_clear()`, then [`super::env::child_env`]; no input (`/dev/null`); the working directory is `/`
//!   (the compose child alone gets the release directory, through [`run_spec`]).
//! - **R6** a deadline: `Child::kill()` on the pid started here, then `wait()`; never a name, a group or a session.
//! - **R7** at most 1 MiB is kept from each stream; the rest is read and dropped so the child never blocks; each stream
//!   has its own reader, so a child that fills one pipe while the other is unread cannot deadlock the call.
//! - **R8** a program that cannot start is [`RunError::NotStarted`].

use super::env::child_env;
use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The most bytes kept from one stream.
const CAP: usize = 1 << 20;
/// How long the readers get, after the child has ended, to reach end-of-file (something the child started may hold a
/// stream open).
const READERS_GRACE: Duration = Duration::from_secs(1);
/// How often the child is looked at.
const POLL: Duration = Duration::from_millis(5);

/// What a child that ended on its own left.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// The exit code; `None` when a signal ended the child.
    pub code: Option<i32>,
    /// At most the first 1 MiB.
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    /// The bytes kept are NOT known to be all the child wrote: more than 1 MiB was written (the rest was dropped), or
    /// the stream had not reached end-of-file one second after the child ended (something the child started still
    /// holds it, and may write more). A caller that reads the stream as a whole answer must refuse such a stream.
    pub stdout_cut: bool,
    pub stderr_cut: bool,
}

/// Why a child gave no outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunError {
    /// The program could not be started.
    NotStarted,
    /// It outlived the deadline and was ended.
    TimedOut,
    /// It was started but the system could not tell how it ended.
    Lost,
}

/// One stream's reader: keeps the first [`CAP`] bytes, drops the rest, and says when it has seen end-of-file.
struct Capture {
    buf: Arc<Mutex<(Vec<u8>, bool)>>,
    done: Arc<AtomicBool>,
}

impl Capture {
    fn start<R: Read + Send + 'static>(stream: Option<R>) -> Capture {
        let buf = Arc::new(Mutex::new((Vec::new(), false)));
        let done = Arc::new(AtomicBool::new(false));
        match stream {
            Some(mut r) => {
                let (b, d) = (Arc::clone(&buf), Arc::clone(&done));
                let spawned = std::thread::Builder::new().spawn(move || {
                    let mut chunk = [0u8; 8192];
                    loop {
                        match r.read(&mut chunk) {
                            Ok(0) => break,
                            Err(_) => {
                                if let Ok(mut g) = b.lock() {
                                    g.1 = true;
                                }
                                break;
                            }
                            Ok(n) => {
                                if let Ok(mut g) = b.lock() {
                                    let room = CAP.saturating_sub(g.0.len());
                                    let take = n.min(room);
                                    g.0.extend_from_slice(&chunk[..take]);
                                    if take < n {
                                        g.1 = true;
                                    }
                                }
                            }
                        }
                    }
                    d.store(true, Ordering::SeqCst);
                });
                if spawned.is_err() {
                    if let Ok(mut g) = buf.lock() {
                        g.1 = true;
                    }
                    done.store(true, Ordering::SeqCst);
                }
            }
            None => done.store(true, Ordering::SeqCst),
        }
        Capture { buf, done }
    }

    /// What was kept, once the reader is done or `end` has passed, and whether it is NOT known to be all of the stream:
    /// cut at the cap, or the reader had not seen end-of-file by `end`.
    fn finish(self, end: Instant) -> (Vec<u8>, bool) {
        while !self.done.load(Ordering::SeqCst) && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(2));
        }
        let open = !self.done.load(Ordering::SeqCst);
        match self.buf.lock() {
            Ok(g) => (g.0.clone(), g.1 || open),
            Err(_) => (Vec::new(), true),
        }
    }
}

/// Starts `program` with `args` and waits at most `deadline`.
pub fn run(program: &OsStr, args: &[OsString], deadline: Duration) -> Result<Outcome, RunError> {
    run_spec(
        &Spec {
            program,
            args,
            cwd: Path::new("/"),
            extra_env: &[],
        },
        deadline,
    )
}

/// What to start: the program, its argument vector, its working directory and the few variables ADDED to the fixed
/// environment (never read from the parent).
pub struct Spec<'a> {
    pub program: &'a OsStr,
    pub args: &'a [OsString],
    pub cwd: &'a Path,
    pub extra_env: &'a [(OsString, OsString)],
}

/// [`run`] with a working directory and extra variables (the compose child's, see [`super::env::ComposeEnv`]).
pub fn run_spec(spec: &Spec, deadline: Duration) -> Result<Outcome, RunError> {
    run_spec_with_poll(spec, deadline, Child::try_wait)
}

// Signal only the direct child. A bounded reap check prevents an unconfirmed
// kill from blocking the deadline; a reaper retains ownership if needed.
fn stop_direct_child(child: Child) {
    let _ = stop_direct_child_with(child, Child::kill, spawn_reaper);
}

/// How long the fallback waits for a child itself when no reaper thread could be started.
const FALLBACK_REAP: Duration = Duration::from_secs(1);

/// How a stopped direct child was left (observable by the cells; the caller of the runner does not need it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reaped {
    /// The child was seen to have exited within the bounded check.
    Confirmed,
    /// A detached reaper thread owns the wait.
    Handed,
    /// No reaper could be started and the child had not exited within the fallback wait either: it is left to the operating system.
    Unreaped,
}

/// The detached reaper: one thread that runs `job` (the blocking `wait`).
fn spawn_reaper(job: Box<dyn FnOnce() + Send>) -> std::io::Result<()> {
    std::thread::Builder::new().spawn(job).map(|_| ())
}

fn stop_direct_child_with<F, S>(mut child: Child, kill: F, spawn: S) -> Reaped
where
    F: FnOnce(&mut Child) -> std::io::Result<()>,
    S: FnOnce(Box<dyn FnOnce() + Send>) -> std::io::Result<()>,
{
    let _ = kill(&mut child);
    let reap_until = Instant::now() + Duration::from_millis(100);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return Reaped::Confirmed,
            Ok(None) if Instant::now() < reap_until => std::thread::sleep(Duration::from_millis(1)),
            _ => break,
        }
    }
    // Hand the child to a detached reaper through a SLOT, so that a reaper that cannot be started gives it back (the thread's closure is dropped with its
    // captures when `spawn` fails: a child moved into it would be lost, never waited for).
    let slot = Arc::new(Mutex::new(Some(child)));
    let theirs = Arc::clone(&slot);
    let job: Box<dyn FnOnce() + Send> = Box::new(move || {
        let taken = theirs.lock().ok().and_then(|mut g| g.take());
        if let Some(mut child) = taken {
            let _ = child.wait();
        }
    });
    if spawn(job).is_ok() {
        return Reaped::Handed;
    }
    // The reaper could not be started (mike's LOW on W1-55's runner): wait for the child HERE, for a bounded time; a child that has still not exited is
    // reported as `Unreaped` (it is left to the operating system when this process ends) and the deadline is not held for longer.
    let Some(mut child) = slot.lock().ok().and_then(|mut g| g.take()) else {
        return Reaped::Handed;
    };
    let until = Instant::now() + FALLBACK_REAP;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return Reaped::Confirmed,
            Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(5)),
            _ => return Reaped::Unreaped,
        }
    }
}

fn fence_until(end: Instant) {
    while let Some(remaining) = end.checked_duration_since(Instant::now()) {
        if remaining.is_zero() {
            break;
        }
        std::thread::sleep(remaining);
    }
}

// Keep the actual process operations in the runner; the poll seam permits a deterministic wait-error cell.
fn run_spec_with_poll<F>(spec: &Spec, deadline: Duration, mut poll: F) -> Result<Outcome, RunError>
where
    F: FnMut(&mut Child) -> std::io::Result<Option<ExitStatus>>,
{
    run_spec_with_poll_and_stop(spec, deadline, &mut poll, stop_direct_child)
}

fn run_spec_with_poll_and_stop<F, G>(
    spec: &Spec,
    deadline: Duration,
    mut poll: F,
    mut stop: G,
) -> Result<Outcome, RunError>
where
    F: FnMut(&mut Child) -> std::io::Result<Option<ExitStatus>>,
    G: FnMut(Child),
{
    let (program, args) = (spec.program, spec.args);
    let mut child = Command::new(program)
        .args(args)
        .env_clear()
        .envs(child_env(spec.extra_env))
        .current_dir(spec.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| RunError::NotStarted)?;
    let out = Capture::start(child.stdout.take());
    let err = Capture::start(child.stderr.take());
    let end = Instant::now() + deadline;
    let status = loop {
        match poll(&mut child) {
            Ok(Some(s)) => break s,
            Ok(None) => {}
            Err(_) => {
                stop(child);
                fence_until(end);
                return Err(RunError::Lost);
            }
        }
        if Instant::now() >= end {
            // The pid started here, and nothing else.
            stop(child);
            return Err(RunError::TimedOut);
        }
        std::thread::sleep(POLL);
    };
    // ONE second for both streams together, counted from the end of the child.
    let end = Instant::now() + READERS_GRACE;
    let (stdout, stdout_cut) = out.finish(end);
    let (stderr, stderr_cut) = err.finish(end);
    Ok(Outcome {
        code: status.code(),
        stdout,
        stderr,
        stdout_cut,
        stderr_cut,
    })
}

use crate::ports::{OneLine, PortError};

/// A fixed one-line failure.
pub fn failed(msg: &str) -> PortError {
    PortError::Failed(OneLine::new(msg))
}

/// A run that gave no outcome, as a port error (R6, R8): a fixed line that names the program by its fixed name and
/// holds no path of the operator's and no text from outside.
pub fn run_failure(e: RunError, program: &str) -> PortError {
    match e {
        RunError::NotStarted => failed(&format!("could not start {program}")),
        RunError::TimedOut => PortError::TimedOut,
        RunError::Lost => PortError::TimedOut,
    }
}

/// `Failed` with the last non-empty stderr line of a child that did not succeed.
pub fn stderr_failure(o: &Outcome) -> PortError {
    PortError::Failed(OneLine::last_line_of(&o.stderr))
}

/// The last non-empty line of a stream, trimmed, as text.
pub fn last_line(bytes: &[u8]) -> Option<String> {
    String::from_utf8_lossy(bytes)
        .split('\n')
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_read_error_marks_output_incomplete() {
        struct FailingReader(bool);
        impl Read for FailingReader {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                if self.0 {
                    Err(std::io::Error::other("synthetic stream read error"))
                } else {
                    self.0 = true;
                    buf[..7].copy_from_slice(b"partial");
                    Ok(7)
                }
            }
        }
        let capture = Capture::start(Some(FailingReader(false)));
        let (bytes, incomplete) = capture.finish(Instant::now() + Duration::from_secs(1));
        assert_eq!(bytes, b"partial");
        assert!(incomplete, "a read fault cannot prove the stream complete");
    }

    #[test]
    fn lost_wait_after_one_second_is_fenced_and_reported_as_timeout() {
        let args = [OsString::from("5")];
        let spec = Spec {
            program: OsStr::new("/bin/sleep"),
            args: &args,
            cwd: Path::new("/"),
            extra_env: &[],
        };
        let timeout = Duration::from_secs(2);
        let begin = Instant::now();
        let mut launched = None;
        let mut injected = false;
        let result = run_spec_with_poll(&spec, timeout, |child| {
            let at = launched.get_or_insert_with(Instant::now);
            if at.elapsed() >= Duration::from_secs(1) {
                injected = true;
                Err(std::io::Error::other("synthetic wait fault"))
            } else {
                child.try_wait()
            }
        })
        .map_err(|error| run_failure(error, "sleep"));
        let elapsed = begin.elapsed();
        assert!(injected, "fault did not reach a live child");
        assert!(
            matches!(result, Err(PortError::TimedOut)) && elapsed >= timeout,
            "lost child status must be fenced: {result:?} at {elapsed:?}"
        );
    }

    #[test]
    fn unconfirmed_kill_is_fenced_and_the_direct_child_is_reaped() {
        let args = [OsString::from("1")];
        let spec = Spec {
            program: OsStr::new("/bin/sleep"),
            args: &args,
            cwd: Path::new("/"),
            extra_env: &[],
        };
        let timeout = Duration::from_secs(2);
        let begin = Instant::now();
        let mut launched = None;
        let mut pid = None;
        let mut kill_fault = false;
        let result = run_spec_with_poll_and_stop(
            &spec,
            timeout,
            |child| {
                pid = Some(child.id());
                let at = launched.get_or_insert_with(Instant::now);
                if at.elapsed() >= Duration::from_millis(100) {
                    Err(std::io::Error::other("synthetic wait fault"))
                } else {
                    child.try_wait()
                }
            },
            |child| {
                stop_direct_child_with(
                    child,
                    |_| {
                        kill_fault = true;
                        Err(std::io::Error::other("synthetic kill fault"))
                    },
                    spawn_reaper,
                );
            },
        )
        .map_err(|error| run_failure(error, "sleep"));
        let elapsed = begin.elapsed();
        let proc_path = format!("/proc/{}", pid.expect("child was polled"));
        let cleanup = Instant::now() + Duration::from_secs(2);
        while Path::new(&proc_path).exists() && Instant::now() < cleanup {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(kill_fault, "kill failure was not injected");
        assert!(
            !Path::new(&proc_path).exists(),
            "direct child was not reaped"
        );
        assert!(
            matches!(result, Err(PortError::TimedOut)) && elapsed >= timeout,
            "unconfirmed kill must be fenced: {result:?} at {elapsed:?}"
        );
    }

    fn sleeper(seconds: &str) -> Child {
        Command::new("/bin/sleep")
            .arg(seconds)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start sleep")
    }

    #[test]
    fn a_reaper_that_cannot_start_falls_back_to_a_bounded_wait_here_and_the_child_is_reaped() {
        // the kill fails (so the child outlives the first bounded check) AND the reaper thread cannot be started (mike's LOW on W1-55's runner)
        let child = sleeper("0.4");
        let pid = child.id();
        let begin = Instant::now();
        let how = stop_direct_child_with(
            child,
            |_| Err(std::io::Error::other("synthetic kill fault")),
            |_job| Err(std::io::Error::other("synthetic spawn fault")),
        );
        assert_eq!(how, Reaped::Confirmed, "the fallback waited for it here");
        assert!(
            begin.elapsed() < Duration::from_secs(3),
            "and within its bound: {:?}",
            begin.elapsed()
        );
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "reaped: no zombie is left"
        );
    }

    #[test]
    fn a_reaper_that_cannot_start_and_a_child_that_will_not_die_in_time_is_reported_as_unreaped_and_does_not_block_for_long()
     {
        let child = sleeper("3");
        let pid = child.id();
        let begin = Instant::now();
        let how = stop_direct_child_with(
            child,
            |_| Err(std::io::Error::other("synthetic kill fault")),
            |_job| Err(std::io::Error::other("synthetic spawn fault")),
        );
        let took = begin.elapsed();
        assert_eq!(how, Reaped::Unreaped);
        assert!(
            took < Duration::from_millis(2500),
            "the fallback is bounded: {took:?}"
        );
        // clean up OUR child by its exact pid
        let _ = Command::new("kill").arg("-9").arg(pid.to_string()).status();
    }

    #[test]
    fn a_reaper_that_starts_owns_the_wait() {
        let child = sleeper("0.4");
        let pid = child.id();
        let how = stop_direct_child_with(
            child,
            |_| Err(std::io::Error::other("synthetic kill fault")),
            spawn_reaper,
        );
        assert_eq!(how, Reaped::Handed);
        let until = Instant::now() + Duration::from_secs(3);
        while Path::new(&format!("/proc/{pid}")).exists() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "the detached reaper reaped it"
        );
    }
}
