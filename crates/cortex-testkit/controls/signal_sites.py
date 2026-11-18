"""K31 (r6, kai's row 09ccf268; r7, mike's recogniser finding): every signal call site in
cortex-testkit, counted statically, and the helper probe's command substitutions pinned.

Standard library only; run as `python3 -I`. Nothing is imported or executed.

THE SWEEP (r7; r8 wording): every regular file under CRATE_DIR (except target/, .git/ and
__pycache__/) is LISTED, by kind, and every CODE file is read (data files are listed only); a
directory that cannot be read is a FAIL (r8), not a silent gap:
- `.py`: parsed with ast. A site is a call of os.kill, os.killpg, signal.pidfd_send_signal, a
  `.send_signal`, `.terminate` or `.kill` method, or a bare `kill`, `killpg` or `real_killpg`.
- `.rs`: line comments removed; a site is `kill(`, `killpg(`, `libc::kill`, `signal::kill`,
  `.kill()` or `pidfd_send_signal`.
- `.toml`, `.md`: data, listed and not searched.
- anything else is read as shell: comments removed (quotes respected), and a site is a `kill`,
  `pkill` or `killall` command word. r7: a quote may come before it, as in a quoted trap body
  (`trap 'kill "$s"' TERM`, which the r6 recogniser missed in r5's controls).
The table must equal EXPECTED exactly, and the helper's two group signals must come before its one
reap (by line order).

THE PIN (r7, mike's B/G remainder; r8, his positive query witness): in supervise.sh,
`helper_probe` contains exactly ONE command substitution, `$(jobs -rp 2>/dev/null; printf
'jobs-rc=%s\n' "$?")`, its status checked (`||`), and the witness checked (`jobs-rc=0)`);
`await_helper` contains exactly one, `$(helper_probe)`. A second, unchecked substitution in the
probe (r6's `$(echo $r)`) or a capture without the witness (r7's) is a FAIL.

NOT COUNTED (stated, not claimed): aliases or renamed imports (`from os import kill as k`),
dynamic calls (getattr, eval, generated code), commands built in strings and run by `eval` or
`bash -c`, signals sent inside external programs (timeout(1), podman), and absolute or escaped
command paths (`/bin/kill`, `\\kill`). K31 is a regression alarm on the source's shape, not a proof
that no other path can send a signal.

Usage: python3 -I signal_sites.py CRATE_DIR
       python3 -I signal_sites.py --fixtures      (the recogniser's own fixtures; also used by K31
                                                   on an older revision's copy of this file)
"""

import ast
import os
import re
import sys

EXPECTED = {
    # the helper: its wrapper's one primitive, and its two group signals (TERM, KILL) under custody
    ("group_runner.py", "os.killpg"): 1,
    ("group_runner.py", "killpg"): 2,
    # the controls' ONE signal call: through a pidfd, to a certified identity
    ("controls/sigsafe.py", "signal.pidfd_send_signal"): 1,
    # K27's healthy model partner hands the helper's own killpg back to it
    ("controls/helper_harness.py", "real_killpg"): 1,
    # Private fake-Podman controls signal only their own supervisor subprocess.
    ("controls/test_volume_lifecycle.py", "p.terminate"): 1,
    ("controls/test_volume_lifecycle.py", "p.send_signal"): 1,
    # K30 (d): the r5 method shown on purpose, a bare number, to the control's own bystander
    ("controls/controls.sh", "kill [K30-numeric]"): 1,
}
SKIP_DIRS = {"target", ".git", "__pycache__"}
CMD = re.compile(r"(?:^|[\s;&|({`'\"])(kill|pkill|killall)(?=[\s;&|)'\"]|$)")
RUST = re.compile(r"\bkill\s*\(|\bkillpg\s*\(|libc::kill\b|signal::kill\b|\.kill\(\)|pidfd_send_signal")
SUBST = re.compile(r"\$\((?!\()")

# (line, expected sites): the recogniser must count these exactly (r7 adds the quoted-trap case).
SHELL_FIXTURES = [
    ("trap 'kill \"$s\" 2>/dev/null; exit 0' TERM", 1),
    ('kill -KILL "$p"', 1),
    ('  [ -n "$x" ] && kill -0 "$p"', 1),
    ('x=$(kill -TERM 5)', 1),
    ('pkill -f thing', 1),
    ('{ killall thing; }', 1),
    ('echo "the left-over members were killed"', 0),
    ('kill_members "$err"', 0),
    ('# kill "$p"', 0),
    ('skill thing', 0),
    ('log "KILL to 5"', 0),
    ('echo "--kill-after=120"', 0),
]


def strip_comment(line):
    out, q = [], None
    for i, ch in enumerate(line):
        if q:
            if ch == q:
                q = None
        elif ch in "'\"":
            q = ch
        elif ch == "#" and (i == 0 or line[i - 1] in " \t;"):
            break
        out.append(ch)
    return "".join(out)


def shell_sites(path):
    sites = []
    with open(path, errors="replace") as f:
        for n, line in enumerate(f, 1):
            code = strip_comment(line)
            for m in CMD.finditer(code):
                tag = " [K30-numeric]" if "K30-numeric" in line else ""
                sites.append((n, m.group(1) + tag))
    return sites


def rust_sites(path):
    sites = []
    with open(path, errors="replace") as f:
        for n, line in enumerate(f, 1):
            code = line.split("//", 1)[0]
            for m in RUST.finditer(code):
                sites.append((n, "rust:" + m.group(0).strip("( ")))
    return sites


def py_sites(path):
    tree = ast.parse(open(path).read(), path)
    sites, reaps = [], []
    for node in ast.walk(tree):
        if not isinstance(node, ast.Call):
            continue
        f = node.func
        if isinstance(f, ast.Attribute):
            base = f.value.id if isinstance(f.value, ast.Name) else None
            if base == "os" and f.attr in ("kill", "killpg"):
                sites.append((node.lineno, f"os.{f.attr}"))
            elif base == "signal" and f.attr == "pidfd_send_signal":
                sites.append((node.lineno, "signal.pidfd_send_signal"))
            elif f.attr in ("send_signal", "terminate", "kill"):
                sites.append((node.lineno, f"{base or '?'}.{f.attr}"))
            elif base == "driver" and f.attr == "wait":
                reaps.append(node.lineno)
        elif isinstance(f, ast.Name) and f.id in ("killpg", "real_killpg", "kill"):
            sites.append((node.lineno, f.id))
    return sites, reaps


def function_body(text, name):
    m = re.search(r"^" + re.escape(name) + r"\(\) \{\n(.*?)^\}", text, re.S | re.M)
    return m.group(1) if m else None


def probe_pin(path):
    """Problems with the probe's command substitutions, or []."""
    text = open(path).read()
    problems = []
    for name, want, forms in (("helper_probe", 1, ("r=$(jobs -rp 2>/dev/null; printf 'jobs-rc=%s\\n' \"$?\") ||",
                                                    "jobs-rc=0)")),
                              ("await_helper", 1, ("st=$(helper_probe)",))):
        body = function_body(text, name)
        if body is None:
            problems.append(f"supervise.sh: no {name}() found")
            continue
        code = "\n".join(strip_comment(l) for l in body.splitlines())
        n = len(SUBST.findall(code))
        missing = [f for f in forms if f not in code]
        if n != want or missing:
            problems.append(f"supervise.sh {name}: {n} command substitution(s) (want {want}), missing {missing}")
    return problems


def fixtures(mod_shell_sites):
    """Problems of a shell recogniser on SHELL_FIXTURES, or []."""
    import tempfile
    with tempfile.NamedTemporaryFile("w", suffix=".sh", delete=False) as t:
        for line, _ in SHELL_FIXTURES:
            t.write(line + "\n")
        path = t.name
    try:
        counts = {}
        for n, _ in mod_shell_sites(path):
            counts[n] = counts.get(n, 0) + 1
    finally:
        os.unlink(path)
    return [f"fixture {i}: {line!r} counted {counts.get(i, 0)} (want {want})"
            for i, (line, want) in enumerate(SHELL_FIXTURES, 1) if counts.get(i, 0) != want]


SWEEP_ERRORS = []


def sweep(root):
    for d, dirs, files in os.walk(root, onerror=SWEEP_ERRORS.append):
        dirs[:] = sorted(x for x in dirs if x not in SKIP_DIRS)
        for f in sorted(files):
            p = os.path.join(d, f)
            if os.path.isfile(p) and not os.path.islink(p):
                yield os.path.relpath(p, root)


def main():
    if sys.argv[1:] == ["--fixtures"]:
        problems = fixtures(shell_sites)
        print(("PASS" if not problems else "FAIL") + f": {len(SHELL_FIXTURES)} recogniser fixtures"
              + ("; " + "; ".join(problems) if problems else ""))
        return 0 if not problems else 1
    root = sys.argv[1]
    table, lines, problems, swept = {}, [], [], []
    for rel in sweep(root):
        p = os.path.join(root, rel)
        reaps = []
        if rel.endswith(".py"):
            kind = "python"
            sites, reaps = py_sites(p)
        elif rel.endswith(".rs"):
            kind, sites = "rust", rust_sites(p)
        elif rel.endswith((".toml", ".md")):
            kind, sites = "data", []
        else:
            kind, sites = "shell", shell_sites(p)
        swept.append(f"{rel} ({kind})")
        for n, k in sites:
            table[(rel, k)] = table.get((rel, k), 0) + 1
            lines.append(f"{rel}:{n} {k}")
        if rel == "group_runner.py":
            late = [n for n, k in sites if k == "killpg" and reaps and n > min(reaps)]
            if not reaps:
                problems.append("group_runner.py: no reap (driver.wait) found")
            elif late:
                problems.append(f"group_runner.py: a group signal after the reap at line {min(reaps)}: {late}")
    for key in sorted(set(table) | set(EXPECTED)):
        if table.get(key, 0) != EXPECTED.get(key, 0):
            problems.append(f"{key[0]} {key[1]}: {table.get(key, 0)} (want {EXPECTED.get(key, 0)})")
    problems += [f"the sweep could not read a directory: {e}" for e in SWEEP_ERRORS]
    if os.path.exists(os.path.join(root, "supervise.sh")):
        problems += probe_pin(os.path.join(root, "supervise.sh"))
    else:
        problems.append("supervise.sh missing")
    word = "PASS" if not problems else "FAIL"
    print(f"{word}: {sum(table.values())} signal call sites: " + "; ".join(lines))
    print(f"swept {len(swept)} files: " + ", ".join(swept))
    print("probe pin: " + ("helper_probe 1 checked substitution with its witness, await_helper 1" if not probe_pin(os.path.join(root, "supervise.sh")) else "see mismatches") if os.path.exists(os.path.join(root, "supervise.sh")) else "probe pin: no supervise.sh")
    if problems:
        print("mismatches: " + "; ".join(problems))
    return 0 if not problems else 1


if __name__ == "__main__":
    sys.exit(main())
