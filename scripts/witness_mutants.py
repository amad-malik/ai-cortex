"""Mutation run for scripts/install-witness.sh (G-S5-2; the same discipline as s5_selftest.py).

A mutant is the witness script with ONE deliberate fault. The named IW tests must FAIL on it. A mutant whose `old`
text is not in the script exactly once, that does not change the script, or that no test catches, FAILS the run
(vacuity guard). The unmutated script must pass the suite first.

    scripts/witness_mutants.py            (run through cv2-admit like any test: cv2-admit python3 -I -B scripts/witness_mutants.py)
"""
import json
import os
import re
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
WITNESS = os.path.join(HERE, "install-witness.sh")
TEST = os.path.join(HERE, "witness-test.sh")
SPEC = os.path.join(HERE, "witness-mutants.json")


def run_suite(witness_path):
    env = dict(os.environ, WITNESS_SH=witness_path)
    p = subprocess.run(["sh", TEST], capture_output=True, text=True, timeout=120, env=env, check=False)
    out = p.stdout + p.stderr
    failed = set(re.findall(r"^FAIL (IW-[0-9]+)", out, re.M))
    return p.returncode, failed, out


def main():
    spec = json.load(open(SPEC, encoding="utf-8"))
    original = open(WITNESS, encoding="utf-8").read()
    rc, failed, out = run_suite(WITNESS)
    if rc != 0:
        print("BASELINE NOT GREEN: the unmutated witness fails its own suite")
        print("\n".join(out.splitlines()[-8:]))
        return 2
    print(f"baseline: the unmutated witness passes ({len(re.findall(r'^ok ', out, re.M))} checks)")
    bad = 0
    for m in spec:
        n = original.count(m["old"])
        if n != 1:
            verdict = f"NOT APPLIED ({n} occurrences of the old text)"
        elif m["new"] == m["old"]:
            verdict = "VACUOUS (the mutant does not change the script)"
        else:
            with tempfile.TemporaryDirectory() as d:
                path = os.path.join(d, "install-witness.sh")
                with open(path, "w", encoding="utf-8") as f:
                    f.write(original.replace(m["old"], m["new"]))
                rc, failed, out = run_suite(path)
            if rc == 0:
                verdict = "SURVIVED (the whole suite passed): the tests are vacuous for this fault"
            else:
                missing = [t for t in m["fails"] if t not in failed]
                verdict = "OK (" + ", ".join(sorted(failed)) + ")" if not missing else f"WRONG TEST: {missing} did not fail (failed: {sorted(failed)})"
        ok = verdict.startswith("OK")
        bad += 0 if ok else 1
        print(f"{m['id']:7} {verdict}  [{m['what']}]")
    print(f"witness-mutants: {len(spec)} mutants, {bad} not caught or broken")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
