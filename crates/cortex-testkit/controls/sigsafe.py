"""cortex-testkit controls: generation-safe signals (r6, vera's F1B-CONTROL-CUSTODY-01).

Standard library only; run as `python3 -I` (or loaded from its path by helper_harness.py). Linux
pidfds (pidfd_open(2), pidfd_send_signal(2)).

EVERY signal a control sends goes through this file, through ONE call: `signal.pidfd_send_signal`
in `send()`. A target is never a bare number. It is an IDENTITY, `pid start` (the start time in
clock ticks since boot, /proc/<pid>/stat field 22), CERTIFIED while the process is known to be ours,
and a signal is sent only through a pidfd opened BEFORE the identity is checked:

  1. fd = pidfd_open(pid)                       no such process: it is gone, nothing is sent
  2. the pidfd's own Pid (/proc/self/fdinfo/fd)  -1: it has exited, nothing is sent; r8: any other
                                                   value than pid: REFUSED (a namespace mismatch)
  3. /proc/<pid>/stat start == the certified one  otherwise the number now names another
                                                   generation: REFUSED, nothing is sent
  4. pidfd_send_signal(fd, sig)

If the number was reused before 1, the pidfd names the new process and 3 refuses it. If it is reused
after 1, the pidfd still names the original: 4 reaches the original or fails with ESRCH, never the
new process. There is no check-then-use on a number.

Certification (who is "ours"):
- a direct child of a known parent: `certify-pid PID PARENT KIND OUT [CMD_SUBSTRING]`: the process's
  parent must be PARENT both before and after its start time is read (a child cannot be reused while
  its parent has not reaped it), and its command line must contain CMD_SUBSTRING if one is given.
  PARENT 0 skips the parent check and then REQUIRES CMD_SUBSTRING, read between two equal start-time
  reads (the controls certify the supervisor's helper this way, by its unique run id);
- the members of a driver's group: `certify-group GROUP HELPER`: the group's leader (pid GROUP) must
  be a child of HELPER (the helper holds it unreaped, which pins the group number), checked before
  AND after the members are listed with their start times. The leader's generation existed at both
  checks, so it existed throughout: the number GROUP could not be reused in between, and every
  process listed in group GROUP then was a member of OUR group. Processes that join later are not
  certified (the controls' drivers start their descendants at once; a later certification adds them).

- r7 (vera's F1B-CONTROL-CUSTODY-01-R6A): a supervised RUN, all-or-nothing, in ONE call:
  `certify-run HELPER RUN_ID GROUP OUT` opens a pidfd for HELPER FIRST and keeps it through the whole
  interval. The helper must be alive (not a zombie), its command line must contain
  `group_runner.py RUN_ID `, and its pidfd must still name it, with the same start time, at the
  end. The leader GROUP must be the helper's child leading its own group, with the same start time,
  before and after the members are listed (r8: its group too, at the end). Only if every check
  holds are the helper, the leader and the members written; otherwise nothing is. A helper or a
  leader whose number now names another generation is therefore refused, and so is a group whose
  helper is not the run's own. "All-or-nothing" (r8 wording) means that EVERY check precedes the
  write; the append itself is not a durable atomic transaction, and the caller's run-end checks
  are not atomic with this call.

A signal command's exit status 0 means "the command completed", not "delivered": its printed
per-target report (delivered, gone or REFUSED) is the evidence, and a caller that needs delivery
checks that report (r8 wording; K35's cleanup does).

Identity file lines: `<kind> <pid> <start>`, kind = sup|helper|leader|member.

Usage:
  sigsafe.py certify-pid   PID PARENT KIND OUT [CMD_SUBSTRING]
  sigsafe.py certify-group GROUP HELPER OUT
  sigsafe.py certify-run   HELPER RUN_ID GROUP OUT
  sigsafe.py signal SIG FILE [KINDS]      (KINDS: comma list; default all; the leader is signalled last)
  sigsafe.py alive FILE [KINDS]           the certified identities still alive (no signal)
Exit: 0 done (see the printed lines); 3 not certified; 2 usage.
"""

import os
import signal
import sys


def stat_fields(pid):
    """(state, ppid, pgrp, start) of pid now, or None."""
    try:
        with open(f"/proc/{pid}/stat") as f:
            s = f.read()
    except OSError:
        return None
    rest = s[s.rindex(")") + 2:].split()
    return rest[0], int(rest[1]), int(rest[2]), int(rest[19])


def cmdline(pid):
    try:
        with open(f"/proc/{pid}/cmdline", "rb") as f:
            return f.read().replace(b"\0", b" ").decode(errors="replace")
    except OSError:
        return ""


def pidfd_pid(fd):
    with open(f"/proc/self/fdinfo/{fd}") as f:
        for line in f:
            if line.startswith("Pid:"):
                return int(line.split()[1])
    return -1


def open_identity(pid, start):
    """A pidfd for the generation (pid, start), or (None, why)."""
    try:
        fd = os.pidfd_open(pid)
    except ProcessLookupError:
        return None, "gone"
    try:
        named = pidfd_pid(fd)
        if named == -1:
            os.close(fd)
            return None, "gone"
        if named != pid:   # r8: the pidfd must name exactly this number
            os.close(fd)
            return None, f"REFUSED (the pidfd names pid {named}, not {pid})"
        st = stat_fields(pid)
        if st is None:
            os.close(fd)
            return None, "gone"
        if st[3] != start:
            os.close(fd)
            return None, f"REFUSED (pid {pid} now has start {st[3]}, not {start})"
        return fd, None
    except Exception:
        os.close(fd)
        raise


def send(pid, start, sig):
    """The ONE signal call of the controls. Returns 'delivered', 'gone' or 'REFUSED …'."""
    fd, why = open_identity(pid, start)
    if fd is None:
        return why
    try:
        signal.pidfd_send_signal(fd, sig)
        return "delivered"
    except ProcessLookupError:
        return "gone"
    finally:
        os.close(fd)


def is_alive(pid, start):
    fd, why = open_identity(pid, start)
    if fd is None:
        return False
    try:
        st = stat_fields(pid)
        return st is not None and st[0] != "Z"
    finally:
        os.close(fd)


def certify_pid(pid, parent, cmd=None):
    if parent == 0 and not cmd:
        return None, "a certification without a parent needs a command-line substring"
    a = stat_fields(pid)
    if a is None or (parent and a[1] != parent):
        return None, f"pid {pid} is not a child of {parent}"
    if cmd and cmd not in cmdline(pid):
        return None, f"pid {pid}'s command line lacks {cmd!r}"
    b = stat_fields(pid)
    if b is None or (parent and b[1] != parent) or b[3] != a[3]:
        return None, f"pid {pid} changed while it was certified"
    return a[3], None


def group_members(group):
    out = []
    for d in os.listdir("/proc"):
        if d.isdigit():
            st = stat_fields(int(d))
            if st and st[2] == group:
                out.append((int(d), st[3]))
    return out


def certify_group(group, helper):
    """[(kind, pid, start)] for the leader and members of group, or (None, why)."""
    a = stat_fields(group)
    if a is None or a[1] != helper or a[2] != group:
        return None, f"the leader {group} is not a child of the helper {helper} leading its own group"
    members = group_members(group)
    b = stat_fields(group)
    if b is None or b[1] != helper or b[3] != a[3]:
        return None, f"the leader {group} changed while its group was listed"
    ids = [("leader", group, a[3])]
    ids += [("member", p, s) for p, s in members if p != group]
    return ids, None


def certify_run(helper, rid, group):
    """[(kind, pid, start)] for the run's helper, leader and members, all-or-nothing, or (None, why)."""
    try:
        hfd = os.pidfd_open(helper)
    except ProcessLookupError:
        return None, f"the helper {helper} is gone"
    try:
        a = stat_fields(helper)
        if a is None or a[0] == "Z" or pidfd_pid(hfd) == -1:
            return None, f"the helper {helper} has exited"
        if f"group_runner.py {rid} " not in cmdline(helper):
            return None, f"pid {helper}'s command line lacks 'group_runner.py {rid} ': not this run's helper"
        la = stat_fields(group)
        if la is None or la[1] != helper or la[2] != group:
            return None, f"the leader {group} is not a child of the helper {helper} leading its own group"
        members = group_members(group)
        lb = stat_fields(group)
        b = stat_fields(helper)
        if b is None or b[3] != a[3] or b[0] == "Z" or pidfd_pid(hfd) == -1:
            return None, f"the helper {helper} changed or exited while its group was listed"
        if lb is None or lb[1] != helper or lb[2] != group or lb[3] != la[3]:   # r8: the group rechecked too
            return None, f"the leader {group} changed while its group was listed"
    finally:
        os.close(hfd)
    ids = [("helper", helper, a[3]), ("leader", group, la[3])]
    ids += [("member", p, s) for p, s in members if p != group]
    return ids, None


def read_ids(path, kinds=None):
    ids = []
    with open(path) as f:
        for line in f:
            k, p, s = line.split()
            if kinds is None or k in kinds:
                ids.append((k, int(p), int(s)))
    ids.sort(key=lambda t: t[0] == "leader")   # the leader last
    return ids


def write_ids(path, ids):
    with open(path, "a") as f:
        for k, p, s in ids:
            f.write(f"{k} {p} {s}\n")


def main(argv):
    if len(argv) < 2:
        print(__doc__)
        return 2
    cmd = argv[1]
    if cmd == "certify-pid" and len(argv) in (6, 7):
        pid, parent, kind, out = int(argv[2]), int(argv[3]), argv[4], argv[5]
        start, why = certify_pid(pid, parent, argv[6] if len(argv) == 7 else None)
        if start is None:
            print(f"not certified: {why}")
            return 3
        write_ids(out, [(kind, pid, start)])
        print(f"certified {kind} {pid} {start}")
        return 0
    if cmd == "certify-group" and len(argv) == 5:
        group, helper, out = int(argv[2]), int(argv[3]), argv[4]
        ids, why = certify_group(group, helper)
        if ids is None:
            print(f"not certified: {why}")
            return 3
        write_ids(out, ids)
        print("certified " + " ".join(f"{k}:{p}:{s}" for k, p, s in ids))
        return 0
    if cmd == "certify-run" and len(argv) == 6:
        helper, rid, group, out = int(argv[2]), argv[3], int(argv[4]), argv[5]
        ids, why = certify_run(helper, rid, group)
        if ids is None:
            print(f"not certified: {why}")
            return 3
        write_ids(out, ids)
        print("certified " + " ".join(f"{k}:{p}:{s}" for k, p, s in ids))
        return 0
    if cmd in ("signal", "alive") and len(argv) >= 3:
        if cmd == "signal":
            sig = signal.Signals["SIG" + argv[2].removeprefix("SIG")]
            path, rest = argv[3], argv[4:]
        else:
            sig, path, rest = None, argv[2], argv[3:]
        kinds = set(rest[0].split(",")) if rest else None
        try:
            ids = read_ids(path, kinds)
        except OSError:
            print("no certified identities")
            return 3
        if cmd == "alive":
            print(" ".join(str(p) for _, p, s in ids if is_alive(p, s)))
            return 0
        if not ids:
            print("no certified identity: nothing signalled")
            return 0
        for k, p, s in ids:
            print(f"{sig.name} {k} {p}: {send(p, s, sig)}")
        return 0
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
