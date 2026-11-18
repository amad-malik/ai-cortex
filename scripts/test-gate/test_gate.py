#!/usr/bin/python3 -I
"""The ci.sh TEST GATE (hue; cox W1-103 (2) and W1-106 A1-A3; plan page mvp/pieces/CI-REQUIRED-CHANGES-hue-r1.md).

    sh scripts/test-gate/test-gate.sh            (ci.sh: gate test blocking sh scripts/test-gate/test-gate.sh)

EVERY workspace test must RUN at the gate; nothing is "declared, not run".

1. FREE DRIVER, ONE admission, bounded (45 min): cargo test --workspace --no-run --message-format=json (package and target of every binary), cargo test --workspace -- --list (the
   INVENTORY), cargo test --workspace --no-fail-fast (the run). Read with the sealed partition tool's own parsing (ws_partition2.py, bob's, byte for byte).
2. CLASSIFICATION BY THE TEST'S OWN PANIC: a FAILED test whose panic message is EXACTLY the testkit's missing-database line is NEEDS_DB (a debt, not a failure of this driver);
   any other FAILED test, a test ignored and not declared in ignored.tsv, a listed test with no result ("neither run nor declared"), a test that ran but was never listed, a
   declared ignore that is not in the inventory, or a trybuild child that is not ok, FAILS the gate. A needs-database list (needs_db.tsv) is kept for the readers (A3: compare it
   with the sealed derived list of the head).
3. DATABASE DRIVERS: one supervised run per package that owes a NEEDS_DB test, ONE AT A TIME, each CV2_DB=1 through the admission program (admit timeout --kill-after=120 780
   supervise.sh cargo test -p PKG --no-fail-fast; the admission wait is outside the 780 s). Every NEEDS_DB test must be ok in its package run; any failure in the run, or a driver
   that exits non-zero, fails. Admission refused (exit 3 or 75): retry, at most 12 tries, then FAIL with the reason. Optional guard (CI_DB_GUARD, default on build-host-1).
4. NOT_RUN (never PASS): the database drivers do not run when nested in an admission (A1: cv2-admit admits a nested call before its database lock, so the VM-wide one-database
   lock would be skipped), or without podman, or without supervise.sh; the LAST line is then exactly  NOT_RUN: <reason>  and the exit code is 77 (A2). A real failure of the free
   driver wins over NOT_RUN.

Exit 0 pass, 1 fail, 77 not run. Seams (TEST_GATE_*) are honoured ONLY with the test hook TEST_GATE_TEST=1, as scripts/cv2-admit-shim does for CV2_ADMIT."""
import importlib.util
import os
import re
import shutil
import socket
import subprocess
import sys
import time
import urllib.error
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
NEEDS_DB = "CV2_PG_PORT is unset: run the tests under crates/cortex-testkit/supervise.sh (L-9): NotPresent"
NOT_RUN_NESTED = "nested in an admission: the database reservation cannot be taken"
TRIES = 12
REFUSED = (3, 75)
PANIC = re.compile(r"^thread '.*' (?:\(\d+\) )?panicked at .*:$")


def guard_on(env):
    if env.get("CI_DB_GUARD") in ("0", "1"):
        return env["CI_DB_GUARD"] == "1"
    return socket.gethostname().split(".")[0] == "build-host-1"


def config(env):
    hook = env.get("TEST_GATE_TEST") == "1"

    def seam(name, default, cast=str):
        return cast(env[name]) if hook and name in env else default

    log_dir = env.get("CI_LOG_DIR") or os.path.join(ROOT, "target", "ci")
    return {
        "cargo": seam("TEST_GATE_CARGO", "cargo"),
        "admit": seam("TEST_GATE_ADMIT", os.path.join(ROOT, "scripts", "cv2-admit-shim")),
        "supervise": seam("TEST_GATE_SUPERVISE", os.path.join(ROOT, "crates", "cortex-testkit", "supervise.sh")),
        "podman": seam("TEST_GATE_PODMAN", shutil.which("podman") or ""),
        "ignored": seam("TEST_GATE_IGNORED", os.path.join(HERE, "ignored.tsv")),
        "loadavg": seam("TEST_GATE_LOADAVG", "/proc/loadavg"),
        "health_url": seam("TEST_GATE_HEALTH_URL", "http://localhost:8501/health"),
        "free_bound": seam("TEST_GATE_FREE_BOUND", 2700, int),
        "db_bound": seam("TEST_GATE_DB_BOUND", 780, int),
        "retry_pause": seam("TEST_GATE_RETRY_PAUSE", 60, int),
        "guard_pause": seam("TEST_GATE_GUARD_PAUSE", 60, int),
        "guard_max": seam("TEST_GATE_GUARD_MAX", 20, int),
        "work": seam("TEST_GATE_WORK", os.path.join(log_dir, "test-gate")),
        "guard": guard_on(env),
    }


def load_partition():
    spec = importlib.util.spec_from_file_location("ws_partition2", os.path.join(HERE, "ws_partition2.py"))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def say(text=""):
    print(text, flush=True)


def name_of(t):
    return " :: ".join(t)


def norm(target):
    return target.replace("-", "_")


# ---- the free driver (runs INSIDE one admission, bounded by the caller) ---------------------------------------------------------------------------------------------
def phase_free(cfg, workdir):
    env = dict(os.environ, CARGO_TERM_COLOR="never")
    cargo = cfg["cargo"]
    with open(os.path.join(workdir, "build.json"), "wb") as out, open(os.path.join(workdir, "build.err"), "wb") as err:
        if subprocess.run([cargo, "test", "--workspace", "--no-run", "--message-format=json"], env=env, stdout=out, stderr=err, check=False).returncode != 0:
            return 10
    with open(os.path.join(workdir, "list.txt"), "wb") as out:
        if subprocess.run([cargo, "test", "--workspace", "--", "--list"], env=env, stdout=out, stderr=subprocess.STDOUT, check=False).returncode != 0:
            return 11
    with open(os.path.join(workdir, "run.txt"), "wb") as out:
        ran = subprocess.run([cargo, "test", "--workspace", "--no-fail-fast"], env=env, stdout=out, stderr=subprocess.STDOUT, check=False)
    with open(os.path.join(workdir, "run.rc"), "w") as f:   # sam CITG-SAM01: the status is checked against the printed rows by the gate, not thrown away (a negative number is a signal)
        f.write(f"{ran.returncode}\n")
    return 0


# ---- reading the outputs -------------------------------------------------------------------------------------------------------------------------------------------
def panic_messages(wp, run_path, m):
    """{(package, target, test): the first line of its panic message} from the `---- name stdout ----` blocks of a run."""
    msgs, block, state = {}, None, None
    for key, line in wp.sections(run_path, m):
        b = wp.BLOCK.match(line)
        if b:
            block, state = (key, b.group(1)), "thread"
            continue
        if line.startswith("failures:"):
            block, state = None, None
            continue
        if block is None:
            continue
        if state == "thread" and PANIC.match(line):
            state = "message"
        elif state == "message" and line.strip():
            msgs[(block[0][0], block[0][2], block[1])] = line.rstrip()
            state = "done"
    return msgs


def parse_db_run(wp, path):
    """{(target, test): ok | FAILED | ignored} from a supervised run of ONE package (the build may carry other flags, so the binary's hash is ignored)."""
    res, cur = {}, None
    for line in open(path, errors="replace"):
        line = wp.strip_ansi(line.rstrip("\n"))
        r = wp.RUNNING.match(line)
        if r:
            cur = norm(r.group(1).rsplit("-", 1)[0])
            continue
        d = wp.DOCTESTS.match(line)
        if d:
            cur = norm(d.group(1))
            continue
        t = wp.RAN.match(line)
        if t and cur is not None and not wp.CHILD.match(t.group(1)):
            res[(cur, t.group(1))] = "ignored" if t.group(2).startswith("ignored") else t.group(2)
    return res


# ---- running things through the admission program -------------------------------------------------------------------------------------------------------------------
class Refused(Exception):
    pass


def admitted(cfg, argv, env, out_path, what):
    """Run argv through the admission program; exit 3 or 75 means refused: pause and try again, at most TRIES in all."""
    for attempt in range(1, TRIES + 1):
        with open(out_path, "wb") as f:
            rc = subprocess.run([cfg["admit"], *argv], env=env, stdout=f, stderr=subprocess.STDOUT, check=False).returncode
        if rc not in REFUSED:
            return rc
        if attempt < TRIES:
            time.sleep(cfg["retry_pause"])
    raise Refused(f"admission refused {TRIES} times for {what}")


def machine_busy(cfg):
    """kai 519ed3d9: pause between packages while the load is >= 12 or the live API takes >= 50 ms or does not answer. Empty text = quiet."""
    try:
        load = float(open(cfg["loadavg"]).read().split()[0])
    except (OSError, ValueError, IndexError):
        return "the load average cannot be read"
    if load >= 12:
        return f"load {load} is at least 12"
    if cfg["health_url"] != "none":
        t0 = time.time()
        try:
            urllib.request.urlopen(cfg["health_url"], timeout=5).read()
        except urllib.error.HTTPError:
            pass   # a 401 in a few ms proves the API answers
        except (urllib.error.URLError, OSError):
            return "the live API does not answer"
        took = time.time() - t0
        if took >= 0.05:
            return f"the live API took {took * 1000:.0f} ms (50 ms or more)"
    return ""


# ---- the gate -------------------------------------------------------------------------------------------------------------------------------------------------------
def fail(problems):
    for p in problems:
        say("problem: " + p)
    say(f"test: FAIL ({len(problems)} problem{'s' if len(problems) != 1 else ''})")
    return 1


def gate(cfg):
    work = cfg["work"]
    os.makedirs(work, exist_ok=True)
    nested = bool(os.environ.get("CV2_ADMITTED"))
    env = dict(os.environ)

    # 1. the free driver: one admission, bounded
    t0 = time.time()
    child = ["timeout", "--kill-after=60", str(cfg["free_bound"]), sys.executable, "-I", "-B", os.path.abspath(__file__), "--phase", "free", "--workdir", work]
    try:
        rc = admitted(cfg, child, env, os.path.join(work, "free.log"), "the free driver")
    except Refused as why:
        return fail([str(why)])
    if rc in (124, 137):
        return fail([f"free driver exceeded its bound of {cfg['free_bound']} s"])
    if rc == 10:
        return fail([f"build failed (see {os.path.join(work, 'build.err')})"])
    if rc == 11:
        return fail(["the test listing failed (cargo test --workspace -- --list)"])
    if rc != 0:
        return fail([f"the free driver exited {rc} (see {os.path.join(work, 'free.log')})"])

    wp = load_partition()
    build, listing, run = (os.path.join(work, n) for n in ("build.json", "list.txt", "run.txt"))
    m = wp.load_map(build)
    inv = wp.tests_listed(listing, m)
    ran = wp.tests_ran(run, m)
    kids = wp.children_ran(run, m)
    if not m or not inv:
        return fail([f"the inventory is empty ({len(m)} test binaries in the build, {len(inv)} tests listed): cargo printed nothing this gate can read"])
    declared = wp.load_ignored(cfg["ignored"])
    msgs = panic_messages(wp, run, m)
    green = {t for t, s in ran.items() if s == "ok"}
    needs_db = {t for t, s in ran.items() if s == "FAILED" and msgs.get(t) == NEEDS_DB}
    real_failed = {t for t, s in ran.items() if s == "FAILED"} - needs_db
    ignored_ran = {t for t, s in ran.items() if s == "ignored"}
    problems = []
    for label, items in (("ran and FAILED", real_failed),
                         ("ignored but not declared", ignored_ran - set(declared)),
                         ("listed but neither run nor declared", inv - green - needs_db - real_failed - ignored_ran - set(declared)),   # a test that ran (ok, failed, ignored) is accounted for
                         ("ran but not in the inventory", set(ran) - inv),
                         ("declared but not in the inventory", set(declared) - inv),
                         ("harness child not ok", {k[:3] for k in kids if k[3] != "ok"})):
        problems += [f"{label}: {name_of(t)}" for t in sorted(items)]
    printed_failed = sum(1 for s in ran.values() if s == "FAILED")
    run_rc = int(open(os.path.join(work, "run.rc")).read().strip())
    if run_rc == 0:
        if printed_failed:
            problems.append(f"cargo exited 0 but {printed_failed} tests are printed FAILED")
    elif run_rc == 101:   # what cargo test --no-fail-fast says when tests failed (the database tests fail here by design)
        if not printed_failed:
            problems.append("cargo exited 101 but no test is printed FAILED")
    else:
        problems.append(f"the free run exited {run_rc} (cargo test exits 0, or 101 when a test failed; anything else, a negative number being a signal, says nothing about the tests)")
    with open(os.path.join(work, "needs_db.tsv"), "w") as f:
        f.writelines("\t".join(t) + "\n" for t in sorted(needs_db))
    free_time = int(time.time() - t0)
    say(f"inventory: {len(inv)} tests in {len({k[:2] for k in inv})} binaries; declared #[ignore]: {len(declared)}")
    say(f"test-free   driver ran {len(ran)}  ok {len(green)}  failed {len(real_failed)}  needs_db {len(needs_db)}  ignored {len(ignored_ran)}   (time {free_time} s)")
    if run_rc == 101 and printed_failed and printed_failed == len(needs_db):
        say(f"test-free   cargo exit status 101: explained by {printed_failed} FAILED tests, all of them needs_db")
    elif run_rc == 0 and not printed_failed:
        say("test-free   cargo exit status 0")
    say(f"needs-database list ({len(needs_db)}): {os.path.join(work, 'needs_db.tsv')}")
    if problems:
        say("test-db     not run: the free driver has problems")
        return fail(problems)
    if not needs_db:
        say(f"test: PASS (free driver only: ran {len(ran)}, ok {len(green)}, needs_db 0)")
        return 0
    unattributed = sorted(t for t in needs_db if t[0] in ("?", "(doc)"))
    if unattributed:
        return fail([f"needs a database but cannot be attributed to a package: {name_of(t)}" for t in unattributed])

    # 2. the database drivers
    reason = None
    if nested:
        reason = NOT_RUN_NESTED
    elif not cfg["podman"] or not os.access(cfg["podman"], os.X_OK):
        reason = "no podman: the database drivers need a container runtime and none is installed here"
    elif not os.access(cfg["supervise"], os.X_OK):
        reason = f"no supervise.sh: {cfg['supervise']} is not executable here"
    if reason:
        say(f"test-db     NOT RUN for {len({t[0] for t in needs_db})} packages ({len(needs_db)} tests): {reason}")
        say(f"NOT_RUN: {reason}")
        return 77

    by_pkg = {}
    for pkg, target, name in sorted(needs_db):
        by_pkg.setdefault(pkg, []).append((target, name))
    db_env = dict(env, CARGO_TERM_COLOR="never", CV2_DB="1", CV2_ADMIT_LOG="1")
    verified = 0
    for pkg in sorted(by_pkg):
        if cfg["guard"]:
            pauses, why = 0, machine_busy(cfg)
            while why:
                if pauses >= cfg["guard_max"]:
                    return fail([f"guard: the machine stayed busy after {pauses} pauses ({why}); tell kai"])
                pauses += 1
                time.sleep(cfg["guard_pause"])
                why = machine_busy(cfg)
        out = os.path.join(work, f"db-{pkg}.txt")
        t1 = time.time()
        try:
            rc = admitted(cfg, ["timeout", "--kill-after=120", str(cfg["db_bound"]), cfg["supervise"], cfg["cargo"], "test", "-p", pkg, "--no-fail-fast"], db_env, out, pkg)
        except Refused as why:
            return fail([str(why)])
        res = parse_db_run(wp, out)
        ok_here = 0
        for target, name in by_pkg[pkg]:
            if res.get((norm(target), name)) == "ok":
                ok_here += 1
            else:
                problems.append(f"needs a database but is not ok in its supervised run: {name_of((pkg, target, name))}")
        bad = sorted(k for k, s in res.items() if s == "FAILED")
        problems += [f"ran and FAILED in its supervised run: {pkg} :: {k[0]} :: {k[1]}" for k in bad if (pkg, k[0], k[1]) not in needs_db and not any(norm(t) == k[0] and n == k[1] for t, n in by_pkg[pkg])]
        if rc != 0:
            problems.append(f"driver for {pkg} exited {rc}" + (f" (its bound is {cfg['db_bound']} s)" if rc in (124, 137) else ""))
        verified += ok_here
        say(f"test-db     package {pkg}  needs_db {len(by_pkg[pkg])}  ok {ok_here}  failed {len(bad)}   (time {int(time.time() - t1)} s)")
        if problems:
            break
    if problems:
        return fail(problems)
    say(f"test: PASS (free driver ok; needs_db {len(needs_db)}, {verified} verified ok in {len(by_pkg)} supervised package runs)")
    return 0


def main(argv):
    cfg = config(os.environ)
    if argv[:2] == ["--phase", "free"] and len(argv) == 4 and argv[2] == "--workdir":
        return phase_free(cfg, argv[3])
    if argv:
        say("usage: test_gate.py   (the internal form is: --phase free --workdir DIR)")
        return 2
    return gate(cfg)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
