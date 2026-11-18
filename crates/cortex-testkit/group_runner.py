"""cortex-testkit's driver group runner, used by supervise.sh (r4: mike's F1B-R0-05 items A-D,
vera's SUP-CUSTODY-01 and -02; r5: vera's SUP-CUSTODY-02 rest and SUP-DEADLINE-01, mike's A, B
and G). Standard library only; run as `python3 -I`.

It runs the driver in a process group of its own and ends that group, holding CUSTODY of the
group from the moment it exists until after the last signal:

- **Custody is lifecycle-backed, never a sample.** The driver is this process's own child, the
  leader of its new group (G = its pid). This process learns how the driver ended with
  `waitid(WNOWAIT)`, which does NOT reap it, and reaps it only after the last signal. An unreaped
  child keeps its pid, so the number G cannot be reused by any other group until then. Every
  signal is `killpg(G, …)` sent while that holds. Nothing else ever signals G: supervise.sh talks to
  this process only through the command FIFO on stdin, never by pid.
- **One clock, one budget.** The driver's deadline, the TERM grace and the verification are
  absolute times on the monotonic clock; every scan is charged to the deadline of its phase
  (each `ps` is capped by the time left, at most 3 s). The driver phase is at most
  deadline + grace + verify.
- **Observation keeps its errors.** A scan is the full `ps -e -o pgid=,pid=,stat=` listing. It is
  known only if `ps` exited 0, every row parses as `pgid pid stat`, and the listing contains this
  process's own row. Anything else (a failure, a time-out, an empty or malformed listing) is
  UNKNOWN, never empty; emptiness that cannot be established is reported, and supervise.sh makes
  it exit 73.
- **r5: a listing must agree with what this process knows.** It holds the leader unreaped, so the
  leader EXISTS until it is reaped here. A group scan is UNKNOWN if the listing lacks the leader's
  row, puts the leader in another group, or shows it as a zombie while `waitid` (checked after the
  listing) says it still runs. So a valid but partial listing is never credited as empty, and the
  group can be VERIFIED EMPTY only after the leader has exited.
- **r6: a listing names each pid once.** A listing in which any pid appears twice (compared as
  numbers, so `0201` is `201`) is ambiguous, and UNKNOWN: a second, contradictory row for the held
  leader can no longer hide behind its first one (vera's SUP-CUSTODY-02, duplicate rows).
- **r7: the state token has a grammar, and this process's own row must be possible.** A row's
  STAT is procps's: one state letter of D I R S T t W Z P, then any of the modifiers < N L s l +
  (X, "dead, should never be seen", and anything else is a malformed row: UNKNOWN). This
  process's own row must not be a zombie: it cannot be one while it reads the listing (vera's
  SUP-CUSTODY-02-SELFSTATE). Stopped (T) stays possible: it can be stopped while ps runs.
- **r8: this process's own row must be a state it can REPORT on the supported kernels** (Linux
  5.3 or later, which the controls' pidfds already require): one of R S D T t I P. W (paging
  before 2.6, waking in 2.6.33 to 3.13) and Z are not, so either is UNKNOWN (vera's
  SUP-CUSTODY-02-SELFSTATE-W). The general row grammar is unchanged for other processes' rows.
- **r5: the reap is bounded too.** The leader is reaped only once `waitid` confirms its exit, and
  only after the last signal. If its exit is not confirmed by the end of the verification, it is
  NOT waited for: the final report says `leader=unreaped`, this process ends, custody ends with it,
  and nothing signals the group again. The final report therefore comes within the budget.
- **A command or the end of the channel stops the driver.** A line `stop` on stdin, or EOF (the
  supervisor is gone, even if it was SIGKILLed), starts the same stop sequence as the deadline.

Usage: python3 -I group_runner.py RUN_ID DEADLINE_S GRACE_S VERIFY_S STATUS_FILE -- CMD [ARG...]

STATUS_FILE gets, in order: `group <G> driver <pid> <confirmed|unconfirmed: why>` once the group
exists, then `final rc=<n|unconfirmed> group=<verified|unknown|notempty>
reason=<driver|deadline|stop|eof> leader=<reaped|unreaped>`.
Log lines go to stderr in supervise.sh's format.
"""

import os
import re
import select
import signal
import subprocess
import sys
import time

ROW = re.compile(r"^\s*(\d+)\s+(\d+)\s+([DIRSTtWZP][<NLsl+]*)\s*$")   # r7: the STAT grammar
SELF_STATES = set("RSDTtIP")   # r8: the states this process can report while it runs (Linux 5.3 or later)
SCAN_CAP_S = 3.0


def log(run_id, msg):
    t = time.time()
    stamp = time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime(t)) + ".%03dZ" % int((t % 1) * 1000)
    sys.stderr.write(f"{stamp} cv2-test-supervisor {run_id}: {msg}\n")
    sys.stderr.flush()


def scan_rows(deadline):
    """The whole validated listing as (pgid, pid, stat) rows, or (None, why)."""
    left = deadline - time.monotonic()
    if left <= 0.05:
        return None, "no time left in this phase"
    try:
        r = subprocess.run(
            ["ps", "-e", "-o", "pgid=,pid=,stat="],
            capture_output=True,
            text=True,
            timeout=min(SCAN_CAP_S, left),
        )
    except subprocess.TimeoutExpired:
        return None, "ps did not finish in the time left"
    except OSError as e:
        return None, f"ps could not run: {e}"
    if r.returncode != 0:
        return None, f"ps exit {r.returncode}: {r.stderr.strip()[:160]!r}"
    rows = []
    for line in r.stdout.splitlines():
        if not line.strip():
            continue
        m = ROW.match(line)
        if not m:
            return None, f"malformed ps row {line[:80]!r}"
        rows.append((int(m.group(1)), int(m.group(2)), m.group(3)))
    # r6: each pid once (as a number: a leading-zero alias is the same pid).
    seen = {}
    for _, pid, _ in rows:
        seen[pid] = seen.get(pid, 0) + 1
    dup = [p for p, c in seen.items() if c > 1]
    if dup:
        return None, f"the listing names pid {dup[0]} {seen[dup[0]]} times (ambiguous)"
    me, my_group = os.getpid(), os.getpgid(0)
    mine = [st for pg, pid, st in rows if pid == me and pg == my_group]
    if not mine:
        return None, f"the listing ({len(rows)} rows) lacks this runner's own row"
    if mine[0][0] not in SELF_STATES:   # r7: Z; r8: W too (not reported by Linux 5.3 or later)
        return None, f"the listing shows this runner {me} as {mine[0]}, which it cannot be while it runs"
    return rows, None


def scan(group, deadline):
    """The live (not zombie) pids in `group`, or (None, why) if the listing cannot be trusted.

    r5: the leader (pid == group) is held unreaped by this process, so it exists. A listing that
    lacks its row, puts it in another group, or shows it as a zombie while it still runs
    contradicts that, and is UNKNOWN. `ended` is read AFTER the listing: a leader that runs now
    ran throughout the listing, so a zombie row for it cannot be true."""
    rows, why = scan_rows(deadline)
    if rows is None:
        return None, why
    lead = [(pg, st) for pg, pid, st in rows if pid == group]
    running = ended(group) is None
    if not lead:
        return None, f"the listing ({len(rows)} rows) lacks the driver's leader {group}, which this helper holds unreaped"
    pg, st = lead[0]
    if pg != group:
        return None, f"the listing puts the driver's leader {group} in group {pg}, not its own"
    if running and st.startswith("Z"):
        return None, f"the listing shows the driver's leader {group} as a zombie while it still runs"
    return [pid for pg, pid, st in rows if pg == group and not st.startswith("Z")], None


def ended(pid):
    """How the driver ended, without reaping it, or None while it runs."""
    st = os.waitid(os.P_PID, pid, os.WEXITED | os.WNOHANG | os.WNOWAIT)
    if st is None or st.si_pid == 0:
        return None
    if st.si_code == os.CLD_EXITED:
        return st.si_status
    return 128 + st.si_status


def killpg(group, sig):
    try:
        os.killpg(group, sig)
    except ProcessLookupError:
        pass


def main():
    run_id, deadline_s, grace_s, verify_s, status_file = sys.argv[1:6]
    if sys.argv[6] != "--":
        raise SystemExit("usage: RUN_ID DEADLINE_S GRACE_S VERIFY_S STATUS_FILE -- CMD [ARG...]")
    cmd = sys.argv[7:]
    deadline_s, grace_s, verify_s = int(deadline_s), int(grace_s), int(verify_s)
    for s in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP, signal.SIGPIPE):
        signal.signal(s, signal.SIG_IGN)

    def child_signals():
        for s in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP, signal.SIGQUIT, signal.SIGPIPE):
            signal.signal(s, signal.SIG_DFL)

    status = open(status_file, "a", buffering=1)
    start = time.monotonic()
    driver_deadline = start + deadline_s
    driver = subprocess.Popen(
        cmd, stdin=subprocess.DEVNULL, process_group=0, preexec_fn=child_signals
    )
    group = driver.pid
    log(run_id, f"helper: driver {driver.pid} started in its own process group {group}; publishing it")
    # r5 (mike's B): the publication scan is charged to the driver's own deadline as well.
    rows, why = scan_rows(min(start + 5, driver_deadline))
    if rows is not None and any(pid == group and pg == group for pg, pid, _ in rows):
        state = "confirmed"  # the leader is listed in its own group (a zombie leader counts)
    else:
        state = f"unconfirmed: {why or 'the leader is not listed in its group'}"
    status.write(f"group {group} driver {driver.pid} {state}\n")
    log(run_id, f"helper: driver {driver.pid} leads its own process group {group} ({state}); "
                f"custody: held unreaped by helper {os.getpid()} until after the last signal")

    # Run until the driver ends, its deadline, or a stop command (or EOF) on stdin.
    rc, reason = None, None
    fd = sys.stdin.fileno()
    while True:
        rc = ended(driver.pid)
        if rc is not None:
            reason = "driver"
            break
        if time.monotonic() >= driver_deadline:
            reason, rc = "deadline", 124
            break
        ready, _, _ = select.select([fd], [], [], 0.2)
        if ready:
            data = os.read(fd, 256)
            if not data:
                reason = "eof"
                break
            if b"stop" in data:
                reason = "stop"
                break
    log(run_id, f"helper: driver {driver.pid} {'exited rc=%s' % rc if reason == 'driver' else 'stopping: ' + reason}")

    # The stop sequence, the same on every path, on absolute deadlines.
    term_dl = time.monotonic() + grace_s
    verify_dl = term_dl + verify_s
    members, why = scan(group, term_dl)
    if members is None or members:
        if members:
            log(run_id, f"driver group {group}: TERM to {' '.join(map(str, members))}")
        else:
            log(run_id, f"driver group {group}: membership UNKNOWN ({why}); TERM under custody")
        killpg(group, signal.SIGTERM)
        while True:
            t_scan = time.monotonic()
            members, why = scan(group, term_dl)
            took = time.monotonic() - t_scan
            if members is None:
                log(run_id, f"driver group {group}: membership UNKNOWN during the TERM grace ({why}); KILL under custody")
                break
            if not members:
                break
            # Another scan only if there is room for it (as long as the last one, plus a pause);
            # otherwise the last KNOWN listing stands and the grace is over.
            if time.monotonic() + took + 0.25 >= term_dl:
                log(run_id, f"driver group {group}: KILL to {' '.join(map(str, members))} (TERM-resistant)")
                break
            time.sleep(0.2)
    if members is None or members:
        killpg(group, signal.SIGKILL)  # the last signal, while the leader is still unreaped
    known, members = False, None
    while True:
        t_scan = time.monotonic()
        members, why = scan(group, verify_dl)
        took = time.monotonic() - t_scan
        known = members is not None
        if known and not members:
            break
        if time.monotonic() + took + 0.15 >= verify_dl:
            break  # the last listing stands: known members, or unknown
        time.sleep(0.1)
    if not known:
        verdict = "unknown"
        log(run_id, f"driver group {group}: membership UNKNOWN after the last signal ({why}): emptiness NOT established")
    elif members:
        verdict = "notempty"
        log(run_id, f"driver group {group}: NOT EMPTY after KILL: {' '.join(map(str, members))}")
    else:
        verdict = "verified"
        log(run_id, f"driver group {group}: VERIFIED EMPTY")

    # Only now is the leader reaped, and its group number released: once waitid confirms that it
    # has exited, and no later than the end of the verification (r5, vera's SUP-DEADLINE-01).
    code = ended(driver.pid)
    while code is None and time.monotonic() < verify_dl:
        time.sleep(0.05)
        code = ended(driver.pid)
    if code is not None:
        driver.wait()  # it has exited: this returns at once
        leader = "reaped"
    else:
        leader = "unreaped"
        if verdict == "verified":  # cannot happen (a running leader is a listed member); fail closed
            verdict = "unknown"
        log(run_id, f"driver group {group}: the leader {driver.pid} has not exited by the end of the "
                    f"verification: it is left unreaped, and nothing signals the group again")
    if reason == "driver":
        pass
    elif reason == "deadline":
        rc = 124
    else:
        rc = code if code is not None else "unconfirmed"
    report = f"rc={rc} group={verdict} reason={reason} leader={leader}"
    status.write(f"final {report}\n")
    status.close()
    log(run_id, f"helper: final report: {report}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
