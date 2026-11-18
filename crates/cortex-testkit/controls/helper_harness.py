"""K27 (r5, vera's SUP-DEADLINE-01): the helper's final report is bounded even when its leader's
exit cannot be confirmed. Standard library only; run as `python3 -I`.

This is a MODEL, not a kernel stall. The helper module is loaded from the path given (so the same
harness runs against an older revision), and its module-level `killpg` is replaced:

- `pending`: a recorder that delivers NO signal. A TERM-resistant driver therefore stays alive
  after the KILL, as one whose termination is still pending would. The helper must still write its
  final report within deadline + grace + verify (+ TOL), must not call it verified, and must not
  wait for the leader (`leader=unreaped` from r5).
- `delivered`: the healthy partner, the real killpg: VERIFIED EMPTY, `group=verified`, within the
  same bound.

r6 (vera's F1B-CONTROL-CUSTODY-01): this harness signals only CERTIFIED identities, through
controls/sigsafe.py (a pidfd opened before the identity is checked). The helper runs in-process, so
the driver's leader is THIS process's child: once the helper has published, the group is certified
while this process still holds the leader unreaped (sigsafe.certify_group, with the leader's parent
= this process, checked before and after the members are listed). The guard (GUARD_S after the start;
it fires only if the helper does not return, and a guard that fired is a FAIL) and the cleanup act on
those identities only, never on a group number read later, and the guard is JOINED before the
cleanup. Then the leader, if the helper left it unreaped, is reaped here, after the last signal.

Usage: python3 -I helper_harness.py GROUP_RUNNER_PY OUT_DIR
Prints one PASS/FAIL line per case; exit 0 only if both pass.
"""

import importlib.util
import os
import signal
import sys
import threading
import time

sys.dont_write_bytecode = True   # loading the helper must leave no __pycache__ in the source tree
HERE = os.path.dirname(os.path.abspath(__file__))
RESIST = 'trap "" TERM INT; (trap "" TERM INT; exec sleep 1000) & sleep 1000'
DEADLINE_S, GRACE_S, VERIFY_S, TOL_S = 2, 3, 2, 2.0
BOUND_S = DEADLINE_S + GRACE_S + VERIFY_S + TOL_S
GUARD_S = BOUND_S + 15


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


ss = load("sigsafe", os.path.join(HERE, "sigsafe.py"))


def published(status):
    try:
        with open(status) as f:
            for line in f:
                if line.startswith("group "):
                    return int(line.split()[1])
    except OSError:
        pass
    return None


def final_line(status):
    with open(status) as f:
        lines = [l.strip() for l in f if l.startswith("final ")]
    return lines[-1] if lines else ""


def leader_last(ids):
    return sorted(ids, key=lambda t: t[0] == "leader")


def run_case(path, case, out_dir):
    gr = load(f"group_runner_{case}", path)
    calls = []
    real_killpg = gr.killpg

    def recorder(group, sig):
        calls.append((round(time.monotonic(), 3), group, int(sig)))
        if case == "delivered":
            real_killpg(group, sig)   # the helper's own killpg, under the helper's own custody

    gr.killpg = recorder
    status = os.path.join(out_dir, f"K27-{case}.status")
    open(status, "w").close()
    r, w = os.pipe()          # stdin: open, never written, never closed: no command, no EOF
    os.dup2(r, 0)
    os.close(r)
    sys.argv = ["group_runner.py", f"K27-{case}", str(DEADLINE_S), str(GRACE_S), str(VERIFY_S),
                status, "--", "bash", "-c", RESIST]
    ids, notes = [], []

    def certifier():
        end = time.monotonic() + 10
        while time.monotonic() < end:
            g = published(status)
            if g:
                time.sleep(0.3)   # the driver starts its descendant at once
                got, why = ss.certify_group(g, os.getpid())
                if got:
                    ids.extend(got)
                else:
                    notes.append(f"not certified: {why}")
                return
            time.sleep(0.02)
        notes.append("not certified: the helper never published")

    fired = []

    def guard():
        c.join()
        fired.append([f"{k}:{p}:{ss.send(p, s, signal.SIGKILL)}" for k, p, s in leader_last(ids)])

    c = threading.Thread(target=certifier)
    c.start()
    t = threading.Timer(GUARD_S, guard)
    t.daemon = True
    t.start()
    t0 = time.monotonic()
    gr.main()
    took = time.monotonic() - t0
    t.cancel()
    t.join()                  # a guard that is firing is finished before anything else happens
    c.join()
    alive = [p for k, p, s in ids if ss.is_alive(p, s)]
    final = final_line(status)
    # Cleanup: the certified identities only, the leader last; then reap the leader if the helper
    # left it unreaped (it is this process's child), after that last signal.
    cleanup = [f"{k}:{p}:{ss.send(p, s, signal.SIGKILL)}" for k, p, s in leader_last(ids)]
    for k, p, s in ids:
        if k == "leader":
            end = time.monotonic() + 10
            while time.monotonic() < end:
                try:
                    if os.waitpid(p, os.WNOHANG)[0]:
                        break
                except ChildProcessError:
                    break
                time.sleep(0.05)
    os.close(w)
    leader = [p for k, p, s in ids if k == "leader"]
    sigs = ",".join(f"{signal.Signals(s).name}@{c_ - t0:.2f}s" for c_, _, s in calls)
    if case == "pending":
        ok = (bool(ids) and not fired and took <= BOUND_S and final and "group=verified" not in final
              and "leader=unreaped" in final and leader and leader[0] in alive)
    else:
        ok = bool(ids) and not fired and took <= BOUND_S and "group=verified" in final and not alive
    word = "PASS" if ok else "FAIL"
    return ok, (f"{word} K27-{case}: report after {took:.2f} s (bound {BOUND_S:.0f} s), guard fired: "
                f"{'yes ' + str(fired[0]) if fired else 'no'}, final '{final}', certified "
                f"{len(ids)} ({'; '.join(notes) or 'leader a child of this harness'}), alive when "
                f"the helper returned: {alive}, cleanup by certified identity: [{' '.join(cleanup)}], "
                f"killpg calls: [{sigs}]")


def main():
    path, out_dir = sys.argv[1], sys.argv[2]
    results = [run_case(path, c, out_dir) for c in ("pending", "delivered")]
    for _, line in results:
        print(line, flush=True)
    return 0 if all(ok for ok, _ in results) else 1


if __name__ == "__main__":
    sys.exit(main())
