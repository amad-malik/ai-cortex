"""S5 checker self-test: the checker must pass the good tree and fail each one-fault mutation of it.

Run it as `scripts/s5-check.sh --self-test`. A mutation whose `old` text is absent from the good tree is
VACUOUS and fails the self-test, so a mutation can never pass by changing nothing. Every rule named in
mutations.json must have at least one mutation.
"""
import hashlib
import json
import os
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
FIX = os.path.join(HERE, "s5-check-fixtures")
GOOD = os.path.join(FIX, "good")
CHECK = os.path.join(HERE, "s5_check.py")
LIMIT_NOTE = "NOTE S5-10: an oracle run in a fixed environment, not a proof of definitions only; F5 is open (vera 5W-REVIEW); the real includes are read by hand at their final hashes and bound by hash (G12)."

passed = 0
failed = 0


def check(name, ok, detail=""):
    global passed, failed
    if ok:
        passed += 1
        print(f"ok   {name}")
    else:
        failed += 1
        print(f"FAIL {name}")
        if detail:
            for line in detail.strip().splitlines()[:6]:
                print(f"       {line}")


def run_checker(tree, mode=None):
    """mode None: check a deploy tree. mode 'never-list': check only the NEVER list over the directory `tree`."""
    args = [tree] if mode is None else ["--never-list", tree]
    p = subprocess.run(
        [sys.executable, "-I", "-B", CHECK] + args,
        capture_output=True, text=True, timeout=120, check=False,
    )
    return p.returncode, p.stdout + p.stderr


def apply(m, tree):
    """Apply one mutation to the copied tree. Returns an error text, or None."""
    if m["op"] == "many":   # several edits, each with its own file (a change that must be made in two places to stay consistent)
        for sub in m["steps"]:
            err = apply(sub, tree)
            if err:
                return err
        return None
    path = os.path.join(tree, m["file"])
    if m["op"] == "delete_file":
        if not os.path.exists(path):
            return f"vacuous: {m['file']} is not in the good tree"
        os.remove(path)
        return None
    if m["op"] == "write":
        with open(path, "w", encoding="utf-8") as f:
            f.write(m["text"])
        return None
    with open(path, encoding="utf-8") as f:
        text = f.read()
    if m["op"] == "replace":
        if m["old"] not in text:
            return f"vacuous: {m['old']!r} is not in {m['file']}"
        text = text.replace(m["old"], m["new"]) if m.get("count") is None else text.replace(m["old"], m["new"], m["count"])
    elif m["op"] == "multi":
        for old, new in m["edits"]:
            if old not in text:
                return f"vacuous: {old!r} is not in {m['file']}"
            text = text.replace(old, new)
    elif m["op"] == "append":
        text += m["text"]
    elif m["op"] == "write":   # create (or overwrite) a whole file: a stack.env beside the release compose (hale 5h N-2)
        text = m["text"]
    else:
        return f"unknown op {m['op']!r}"
    with open(path, "w", encoding="utf-8") as f:
        f.write(text)
    return None


def place_manifest(tree):
    """S5-23 compares deploy/server-features with crates/cortex-server/Cargo.toml, which sits beside deploy/ in the repository: put the fixture's copy beside the tree."""
    dst = os.path.join(os.path.dirname(tree), "crates", "cortex-server")
    os.makedirs(dst, exist_ok=True)
    shutil.copy(os.path.join(FIX, "crates", "cortex-server", "Cargo.toml"), dst)


def release_tree(spec, dest):
    """The release layout (5h): the good tree with the edits release.sh makes, so a mutation can start from it. Returns an error text or None."""
    shutil.copytree(GOOD, dest)
    place_manifest(dest)
    return apply(dict(spec["release_layout"]), dest)


def hostile_tree(d, text):
    tree = os.path.join(d, "tree")
    shutil.copytree(GOOD, tree)
    place_manifest(tree)   # (the merged good tree carries S5-23's server-features: the manifest it is compared with sits beside it)
    with open(os.path.join(tree, "install-prereq.sh"), "a", encoding="utf-8") as f:
        f.write(text)
    return tree


def sandbox_checks():
    """5w: S5-10 lets bash read an included source inside a namespace. KILL RULE (kai, cox W1-70): NO cell or probe here carries a destructive payload. The sandbox's properties are read
    by QUERY (type, $-, $$, ulimit, the shell's own refusal text) and by harmless canaries (an echo, a redirect to a path that must not come to exist), never by calling the dangerous command."""
    import time
    sys.path.insert(0, HERE)
    import s5_check
    with tempfile.TemporaryDirectory() as d:
        target = os.path.join(d, "must-not-come-to-exist")
        probe = os.path.join(d, "probe.sh")
        with open(probe, "w", encoding="utf-8") as f:
            f.write('echo "Q-pid=$$"\necho "Q-flags=$-"\necho "Q-path=[$PATH]"\necho "Q-uid=$UID"\necho "Q-pwd=$PWD"\n'
                    'while read -r q_line; do case $q_line in *:*) echo "Q-net=${q_line%%:*}" ;; esac; done < /proc/net/dev\n'
                    'while read -r q_line; do case $q_line in *" /tmp "*) echo "Q-tmp=$q_line" ;; esac; done < /proc/self/mounts\n'
                    'read -r q_init < /proc/1/comm; echo "Q-init=$q_init"\necho "Q-shellopts=$SHELLOPTS"\n'
                    'if command -v kill; then echo "Q-kill-enabled"; else echo "Q-kill-disabled"; fi\n'
                    'echo Q-ulimit-u; ulimit -u\necho Q-ulimit-t; ulimit -t\necho Q-ulimit-v; ulimit -v\necho Q-ulimit-f; ulimit -f\necho Q-ulimit-c; ulimit -c\n'
                    f'cd /\necho canary > {target}\n')
        out = s5_check.source_trace(probe, whole=True)
        lines = [ln for ln in out.splitlines() if not ln.startswith("+ ")]
        def after(label):
            return lines[lines.index(label) + 1] if label in lines else None
        check("query: the oracle's shell is process 1 of its own pid namespace, and /proc is the namespace's own (process 1 there is a bash, not the host's init)", "Q-pid=1" in lines and "Q-init=bash" in lines, out)
        check("query: the oracle's bash runs in POSIX mode (a file cannot shadow the special builtins of the epilogue)", any(ln.startswith("Q-shellopts=") and "posix" in ln for ln in lines), out)
        check("query: the oracle's shell is restricted (r in $-) and the environment is clean (PATH names a directory that does not exist, uid 0 is root of its own user namespace only)",
              any(ln.startswith("Q-flags=") and "r" in ln[len("Q-flags="):] for ln in lines) and "Q-path=[/oracle/path]" in lines and "Q-uid=0" in lines and not os.path.exists("/oracle/path"), out)
        check("query: the working directory is /tmp, a private tmpfs mounted inside the namespace", "Q-pwd=/tmp" in lines and any(ln.startswith("Q-tmp=") and " tmpfs " in ln for ln in lines), out)
        nets = {ln[len("Q-net="):].strip() for ln in lines if ln.startswith("Q-net=")}
        check("query: the namespace has no network but its own loopback (no interface of the host is there)", nets <= {"lo"}, out)
        check("query: the kill builtin is disabled (command -v kill finds nothing; an echo behind a refused redirect would prove nothing)", "Q-kill-disabled" in lines and "Q-kill-enabled" not in lines and "kill" not in lines, out)
        check("query: ulimit says no new process, 20 s of CPU, 2 GiB, no file bytes, no core",
              [after(x) for x in ("Q-ulimit-u", "Q-ulimit-t", "Q-ulimit-v", "Q-ulimit-f", "Q-ulimit-c")] == ["1", "20", "2097152", "0", "0"], out)
        check("canary: a redirect out of the oracle is refused by the shell and writes nothing", "restricted: cannot redirect output" in out and not os.path.exists(target), out)
        check("canary: cd is refused by the shell", "cd: restricted" in out, out)
        rc, o = run_checker(hostile_tree(d, f"echo canary > {target}\n"))
        check("an included source that runs a command is refused by the checker (S5-10) and the canary file does not exist", rc == 1 and "FAIL S5-10" in o and not os.path.exists(target), o)
    with tempfile.TemporaryDirectory() as d:
        tree = hostile_tree(d, "while :; do :; done\n")
        t0 = time.monotonic()
        rc, out = run_checker(tree)
        check("an included source that loops forever is refused at its first command, in seconds", rc == 1 and "FAIL S5-10" in out and time.monotonic() - t0 < 10, f"{time.monotonic() - t0:.1f} s\n{out}")
    with tempfile.TemporaryDirectory() as d:
        p = subprocess.run([sys.executable, "-I", "-B", CHECK, GOOD], capture_output=True, text=True, timeout=60, env={"PATH": d}, check=False)
        check("with an empty PATH the checker exits 2 (it could not run) and never passes", p.returncode == 2, p.stdout + p.stderr)
        p = subprocess.run([sys.executable, "-I", "-B", "-c",
                            "import sys; sys.path.insert(0, sys.argv[1]); import s5_check\n"
                            "try:\n    s5_check.oracle_tools()\nexcept s5_check.OracleUnavailable as e:\n    print('unavailable:', e)", HERE],
                           capture_output=True, text=True, timeout=60, env={"PATH": d}, check=False)
        check("with no bash on the PATH the oracle is unavailable and says so", "unavailable: bash was not found" in p.stdout, p.stdout + p.stderr)
    # the verdict logic itself, on outputs a real run cannot give: an accept needs BOTH the empty file's output and the sentinel status (CM-155, CM-156)
    base = s5_check.oracle_baseline()
    real_run = s5_check._oracle_run
    try:
        probe_file = os.path.join(tempfile.mkdtemp(prefix="s5-unit-"), "inc.sh")
        with open(probe_file, "w", encoding="utf-8") as f:
            f.write("prereq_unit() {\n  :\n}\n")
        for label, fake, want_accept in (("the output of an empty file and the sentinel status", (base, s5_check.ORACLE_SENTINEL_STATUS, ""), True),
                                         ("the output of an empty file and another status", (base, 5, ""), False),
                                         ("the sentinel status and another output", (b"+ echo x\n" + base, s5_check.ORACLE_SENTINEL_STATUS, ""), False),
                                         ("the sentinel status and a SHORTER output", (base[:20], s5_check.ORACLE_SENTINEL_STATUS, ""), False)):
            s5_check._oracle_run = lambda data, expect, fake=fake: fake
            verdict = s5_check.source_trace(probe_file)
            check(f"unit: {label} is {'accepted' if want_accept else 'refused'}", (verdict == "") == want_accept, repr(verdict))
        calls = []
        def unstable(data, expect):
            calls.append(1)
            return (b"+ case $- in\n+ set +x\n" + (b"A" if len(calls) == 1 else b"B"), s5_check.ORACLE_SENTINEL_STATUS, "")
        s5_check._oracle_run = unstable
        s5_check.ORACLE_BASELINE = None
        try:
            s5_check.oracle_baseline()
            check("unit: a baseline that does not repeat makes the oracle unavailable", False, "no exception")
        except s5_check.OracleUnavailable as e:
            check("unit: a baseline that does not repeat makes the oracle unavailable", "baseline" in str(e), str(e))
    finally:
        s5_check._oracle_run = real_run
        s5_check.ORACLE_BASELINE = None
    base = s5_check.oracle_baseline()
    out, rc, note = s5_check._oracle_run(s5_check.ORACLE_PRELUDE + b"while :; do :; done\n" + s5_check.ORACLE_EPILOGUE, base)
    check("unit: a run that deviates from an empty file's output is ended at once (status None, a short output), not read to the 1 MiB cap", rc is None and 0 < len(out) < (1 << 16), f"rc {rc} bytes {len(out)}")
    with tempfile.TemporaryDirectory() as d:   # a bash that says it is 4.3: refused by the pre-flight (CM-185)
        fake_bash = os.path.join(d, "bash")
        with open(fake_bash, "w", encoding="utf-8") as f:
            f.write('#!/bin/sh\necho "4 3 1"\n')
        os.chmod(fake_bash, 0o755)
        p = subprocess.run([sys.executable, "-I", "-B", "-c",
                            "import sys; sys.path.insert(0, sys.argv[1]); import s5_check\n"
                            "try:\n    s5_check.oracle_tools()\nexcept s5_check.OracleUnavailable as e:\n    print('unavailable:', e)", HERE],
                           capture_output=True, text=True, timeout=60, env={"PATH": d}, check=False)
        check("unit: a bash older than 4.4 makes the oracle unavailable", "unavailable: bash 4.3 is older than 4.4" in p.stdout, p.stdout + p.stderr)
    with tempfile.TemporaryDirectory() as d:
        log = os.path.join(d, "fake-unshare.log")
        fake = os.path.join(d, "unshare")
        with open(fake, "w", encoding="utf-8") as f:
            f.write(f'#!/bin/sh\necho "$@" >> {log}\necho "unshare: unshare failed: Operation not permitted" >&2\nexit 1\n')
        os.chmod(fake, 0o755)
        p = subprocess.run([sys.executable, "-I", "-B", "-c",
                            "import sys; sys.path.insert(0, sys.argv[1]); import s5_check; s5_check.UNSHARE_PATHS = (%r,)\n"
                            "sys.exit(s5_check.run_main(['s5_check.py', sys.argv[2]]))" % fake, HERE, GOOD],
                           capture_output=True, text=True, timeout=60, check=False)
        calls = open(log, encoding="utf-8").read().splitlines() if os.path.exists(log) else []
        check("a unshare that fails: the checker exits 2 (cannot run S5-10) and the only call was the pre-flight, so nothing ran unconfined",
              p.returncode == 2 and "cannot run" in p.stderr and len(calls) == 1 and "--mount-proc" in calls[0], f"rc {p.returncode} calls {calls}\n{p.stdout}{p.stderr}")
        check("a failed unshare pre-flight prints the S5-10 limit once and still exits 2",
              p.returncode == 2 and p.stdout.splitlines().count(LIMIT_NOTE) == 1,
              f"rc {p.returncode}\n{p.stdout}{p.stderr}")
    p = subprocess.run([sys.executable, "-I", "-B", "-c",
                        "import sys; sys.path.insert(0, sys.argv[1]); import s5_check; s5_check.MOUNT_PATHS = ()\n"
                        "try:\n    s5_check.oracle_tools()\nexcept s5_check.OracleUnavailable as e:\n    print('unavailable:', e)", HERE],
                       capture_output=True, text=True, timeout=60, check=False)
    check("with no mount the oracle is unavailable (its private /tmp is a tmpfs mounted inside the namespace)", "unavailable: mount was not found" in p.stdout, p.stdout + p.stderr)
    with tempfile.TemporaryDirectory() as d:
        tree = hostile_tree(d, "")
        with open(os.path.join(tree, "install-prereq.sh"), "ab") as f:
            f.write(b"# a byte that is not UTF-8: \xff\n")
        rc, o = run_checker(tree)
        check("an included source that is not UTF-8 is refused (S5-10)", rc == 1 and "FAIL S5-10" in o and "UTF-8" in o, o)
    p = subprocess.run([sys.executable, "-I", "-B", "-c",
                        "import sys; sys.path.insert(0, sys.argv[1]); import s5_check; s5_check.os.geteuid = lambda: 0\n"
                        "try:\n    s5_check.oracle_tools()\nexcept s5_check.OracleUnavailable as e:\n    print('unavailable:', e)", HERE],
                       capture_output=True, text=True, timeout=60, check=False)
    check("as root the oracle is unavailable (never root: the user namespace would map the caller to the real root)", "unavailable: S5-10 does not run the oracle as root" in p.stdout, p.stdout + p.stderr)
    with tempfile.TemporaryDirectory() as d:
        os.symlink(shutil.which("bash"), os.path.join(d, "bash"))   # a bash, but no unshare anywhere the checker would look
        p = subprocess.run([sys.executable, "-I", "-B", "-c",
                            "import sys; sys.path.insert(0, sys.argv[1]); import s5_check; s5_check.UNSHARE_PATHS = ()\n"
                            "try:\n    s5_check.oracle_tools()\nexcept s5_check.OracleUnavailable as e:\n    print('unavailable:', e)", HERE],
                           capture_output=True, text=True, timeout=60, env={"PATH": d}, check=False)
        check("with no unshare the oracle is unavailable (the checker never runs the file unconfined)", "unavailable: unshare was not found" in p.stdout, p.stdout + p.stderr)


def main():
    with open(os.path.join(FIX, "mutations.json"), encoding="utf-8") as f:
        spec = json.load(f)

    rc, out = run_checker(GOOD)
    check("good tree passes with no finding", rc == 0 and "FAIL" not in out, out)
    check("the good tree prints the W1-78 S5-10 limitation NOTE and names the shells", "NOTE S5-10: an oracle run in a fixed environment, not a proof of definitions only; F5 is open (vera 5W-REVIEW); the real includes are read by hand at their final hashes and bound by hash (G12)." in out.splitlines() and "bash" in out and "sh -n" in out and "dash is not covered" in out, out)
    s5_notes = [line for line in out.splitlines() if line.startswith("NOTE S5-10")]
    check("the good tree prints the exact limit once as its first S5-10 note",
          bool(s5_notes) and s5_notes[0] == LIMIT_NOTE and s5_notes.count(LIMIT_NOTE) == 1, out)

    # The note must count files actually examined, including the zero-file
    # case, and distinguish names absent from those that were read.
    with tempfile.TemporaryDirectory() as d:
        tree = os.path.join(d, "tree")
        shutil.copytree(GOOD, tree)
        place_manifest(tree)
        install = os.path.join(tree, "install.sh")
        with open(install, "a", encoding="utf-8") as f:
            f.write('[ ! -f "$here/install-fetch.sh" ] || . "$here/install-fetch.sh"   # @include\n')
        prereq = os.path.join(tree, "install-prereq.sh")
        os.remove(prereq)
        rc, note_none = run_checker(tree)
        check("S5-10 note reports zero read and names both absent includes", rc == 0 and "NOTE S5-10: 0 included source(s) read; no oracle ran." in note_none and "named but absent: install-fetch.sh, install-prereq.sh" in note_none and "bash None" not in note_none, note_none)
        shutil.copy2(os.path.join(GOOD, "install-prereq.sh"), prereq)
        rc, note_one = run_checker(tree)
        check("S5-10 note counts one read and names the absent include", rc == 0 and "NOTE S5-10: 1 included source(s) read by bash " in note_one and "named but absent: install-fetch.sh" in note_one, note_one)
        with open(os.path.join(tree, "install-fetch.sh"), "w", encoding="utf-8") as f:
            f.write("synthetic_only() {\n  :\n}\n")
        rc, note_both = run_checker(tree)
        check("S5-10 note counts both read and says none absent", rc == 0 and "NOTE S5-10: 2 included source(s) read by bash " in note_both and "named but absent: none" in note_both, note_both)
        with open(install, "a", encoding="utf-8") as f:
            f.write('[ ! -f "$here/install-late.sh" ] || . "$here/install-late.sh"   # @include\n')
        os.mkdir(os.path.join(tree, "install-late.sh"))
        rc, note_unread = run_checker(tree)
        check("S5-10 note names a present include not read as a file", rc == 0 and
              "NOTE S5-10: named but not read: install-late.sh" in note_unread, note_unread)

    with tempfile.TemporaryDirectory() as d:
        tree = os.path.join(d, "tree")
        err = release_tree(spec, tree)
        if err:
            check("the release layout can be built from the good tree", False, err)
        else:
            rc, out = run_checker(tree)
            check("the release layout (server and migrate on ${CORTEX_SERVER_IMAGE:?}, no build) passes with no finding", rc == 0 and "FAIL" not in out, out)
            # hale 5h N-2: a pass on a release tree must not read as "every image is digest-pinned": one NOTE per release image says the pin was NOT checked here
            notes = [ln for ln in out.splitlines() if ln.startswith("NOTE S5-06")]
            check("the release layout prints one NOTE per release image saying its digest pin is stack.env's and was NOT checked here", len(notes) == 2 and all("NOT checked" in n and "stack.env" in n for n in notes), out)
            with open(os.path.join(tree, "stack.env"), "w", encoding="utf-8") as f:
                f.write(spec["good_stack_env"])
            rc, out = run_checker(tree)
            notes = [ln for ln in out.splitlines() if ln.startswith("NOTE S5-06")]
            check("with a stack.env beside it the NOTE says the pin was checked for SHAPE (and the tree still passes)", rc == 0 and "FAIL" not in out and len(notes) == 2 and all("SHAPE" in n and "NOT checked" not in n for n in notes), out)

    rc, out = run_checker(os.path.join(GOOD, "scripts"), "never-list")
    check("good scripts directory passes the NEVER list with no finding", rc == 0 and "FAIL" not in out, out)

    check("an allowed line is accepted and listed as a NOTE (S5-15)", "NOTE S5-15" in out, out)

    # cox W1-70: marv's two real included files, as pinned (fixtures/real-included/INDEX.txt), must be ACCEPTED, alone and in a tree that includes them
    REAL = os.path.join(FIX, "real-included")
    pinned = {}   # cox W1-72: a fixture is a COPY pinned by sha256 (INDEX.txt), never a live path; a fixture that changed is a failure of its own
    with open(os.path.join(REAL, "INDEX.txt"), encoding="utf-8") as f:
        for ln in f:
            if not ln.startswith("#") and ln.strip():
                pinned[ln.split()[1]] = ln.split()[0]
    actual = {fn: hashlib.sha256(open(os.path.join(REAL, fn), "rb").read()).hexdigest() for fn in ("install-prereq.sh", "install-fetch.sh")}
    check("the two real included files are the COPIES pinned by sha256 in INDEX.txt (install-prereq.sh " + actual["install-prereq.sh"][:12] + ", install-fetch.sh " + actual["install-fetch.sh"][:12] + ")", pinned == actual, f"pinned {pinned} actual {actual}")
    with tempfile.TemporaryDirectory() as d:
        tree = os.path.join(d, "tree")
        shutil.copytree(GOOD, tree)
        place_manifest(tree)
        for fn in ("install-prereq.sh", "install-fetch.sh"):
            shutil.copyfile(os.path.join(REAL, fn), os.path.join(tree, fn))
        inst = os.path.join(tree, "install.sh")
        with open(inst, encoding="utf-8") as f:
            text = f.read()
        inc = '[ ! -f "$here/install-prereq.sh" ] || . "$here/install-prereq.sh"   # @include\n'
        assert inc in text
        with open(inst, "w", encoding="utf-8") as f:
            f.write(text.replace(inc, inc + inc.replace("install-prereq.sh", "install-fetch.sh")))
        rc, out = run_checker(tree)
        check("marv's install-prereq.sh and install-fetch.sh (the real included files, as pinned) are ACCEPTED by every rule in a tree that includes them", rc == 0 and "FAIL" not in out, out)

    for ps in spec.get("good_passes", []):   # variants of the GOOD tree that must still pass (hale's positive controls)
        with tempfile.TemporaryDirectory() as d:
            tree = os.path.join(d, "tree")
            shutil.copytree(GOOD, tree)
            place_manifest(tree)
            err = apply(ps, tree)
            if err:
                check(f"{ps['id']} {ps['note']}", False, err)
                continue
            rc, out = run_checker(tree)
            check(f"{ps['id']} {ps['note']}", rc == 0 and "FAIL" not in out, out)

    for ps in spec.get("release_passes", []):   # variants of the release layout that must STILL pass (5h)
        with tempfile.TemporaryDirectory() as d:
            tree = os.path.join(d, "tree")
            err = release_tree(spec, tree) or apply(ps, tree)
            if err:
                check(f"{ps['id']} {ps['note']}", False, err)
                continue
            rc, out = run_checker(tree)
            check(f"{ps['id']} {ps['note']}", rc == 0 and "FAIL" not in out, out)

    # cox W1-55: the relation, written in ONE place (the checker's constants) and mirrored by the engine's en_09: lead = idle + commit latency + slack = 9, the latest commit is E - slack
    sys.path.insert(0, HERE)
    try:
        import s5_check
        rel = tuple(getattr(s5_check, n, None) for n in ("ROLES_IDLE", "ROLES_COMMIT_LATENCY", "ROLES_SLACK", "ROLES_LEAD"))
    except Exception:
        rel = (None,) * 4
    check("the relation: lead 9 = idle 5 + commit latency 2 + slack 2, so the latest commit is E - 2", rel == (5, 2, 2, 9) and rel[3] == sum(rel[:3]), f"constants: {rel}")

    covered = {m["rule"] for m in spec["mutations"]}
    for rule in spec["rules"]:
        check(f"rule {rule} has at least one mutation", rule in covered)

    for m in spec["mutations"]:
        with tempfile.TemporaryDirectory() as d:
            tree = os.path.join(d, "tree")
            if m.get("base") == "release":
                err = release_tree(spec, tree) or apply(m, tree)
            else:
                shutil.copytree(GOOD, tree)
                place_manifest(tree)
                err = apply(m, tree)
            label = f"{m['id']} {m['rule']} {m['note']}"
            if err:
                check(label, False, err)
                continue
            if m.get("mode") == "never-list":
                rc, out = run_checker(os.path.join(tree, m["dir"]), "never-list")
            else:
                rc, out = run_checker(tree)
            ok = rc == 1 and f"FAIL {m['rule']}" in out
            if ok and m.get("expect"):   # the finding must say why (5h): a mutation that fails for another reason proves nothing
                ok = any(f"FAIL {m['rule']}" in line and m["expect"] in line for line in out.splitlines())
            check(label, ok, out)

    sandbox_checks()

    with tempfile.TemporaryDirectory() as d:
        rc, out = run_checker(os.path.join(d, "missing"))
        check("a directory that does not exist fails S5-14", rc == 1 and "FAIL S5-14" in out, out)

    print(f"s5-selftest: {passed} passed, {failed} failed")
    return 0 if failed == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
