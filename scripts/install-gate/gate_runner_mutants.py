#!/usr/bin/python3 -I
"""Mutation check of gate_runner.py (nix; the rule of install_mutants.py): ONE fault per mutant, planted in a COPY; the cells
run against the copy (GR_RUNNER); the named failure line must appear.  The baseline must pass.  A mutant whose `old` text is not found
exactly once, that changes nothing, that does not compile, or that no cell catches FAILS the run.  Run: cv2-admit python3 -I gate_runner_mutants.py"""
import json, os, re, shutil, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
TMP = os.environ.get("GR_TMP", os.path.expanduser("~/.cache/cortex-install-gate-tests"))
if TMP.startswith("/tmp"):
    sys.exit("refusing scratch under /tmp")
os.makedirs(TMP, exist_ok=True)
TIMEOUT = int(os.environ.get("GR_MUTANT_TIMEOUT", "400"))


def cells(runner, only=()):
    env = dict(os.environ, GR_RUNNER=runner)
    p = subprocess.run(["/usr/bin/python3", "-I", os.path.join(HERE, "test_gate_runner.py"), *only], env=env, capture_output=True, timeout=TIMEOUT)
    return p.returncode, p.stdout.decode("utf-8", "replace") + p.stderr.decode("utf-8", "replace")


FILES = ("gate_runner.py", "gate_matrix.py")


def main():
    sources = {n: open(os.path.join(HERE, n)).read() for n in FILES}
    mutants = json.load(open(os.path.join(HERE, "gate_runner_mutants.json")))
    wanted = sys.argv[1:]
    if wanted:
        mutants = [m for m in mutants if m["id"] in wanted]
    broken = [m["id"] for m in mutants if sources[m.get("file", "gate_runner.py")].count(m["old"]) != 1 or m["old"] == m["new"]]
    if broken:
        print("BROKEN before any run (old text not found exactly once): " + ", ".join(broken))
        return 1
    rc, out = cells(os.path.join(HERE, "gate_runner.py"))
    if rc != 0:
        print("BASELINE FAILED\n" + out[-1500:])
        return 1
    print("baseline passed", flush=True)
    bad = 0
    for m in mutants:
        target = m.get("file", "gate_runner.py")
        mutated = sources[target].replace(m["old"], m["new"], 1)
        try:
            compile(mutated, "mutant", "exec")
        except SyntaxError as e:
            bad += 1
            print("BROKEN %s: does not compile: %s" % (m["id"], e))
            continue
        d = tempfile.mkdtemp(prefix="grm.", dir=TMP)
        try:
            for n in FILES:
                open(os.path.join(d, n), "w").write(mutated if n == target else sources[n])
            os.makedirs(os.path.join(d, "matrix"))
            shutil.copy(os.path.join(HERE, "matrix", "inventory.json"), os.path.join(d, "matrix", "inventory.json"))
            named = re.search(r"GR-\d+[a-z]?", m["fails"])
            rc, out = cells(os.path.join(d, "gate_runner.py"), [named.group(0)] if named else [])
        finally:
            shutil.rmtree(d)
        if m["fails"] in out:
            print("caught %s (%s)" % (m["id"], m["what"]), flush=True)
        else:
            bad += 1
            print("NOT CAUGHT %s (%s): wanted %r; cells said: %s" % (m["id"], m["what"], m["fails"], [l for l in out.splitlines() if l.startswith("FAIL")][:3]), flush=True)
    print("gate runner mutants: %d, not caught or broken: %d" % (len(mutants), bad))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
