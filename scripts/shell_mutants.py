"""Mutation run for a shell script whose own self-test is its test (A generic form of witness_mutants.py / install_mutants.py).

    cv2-admit python3 -I -B scripts/shell_mutants.py scripts/ci-mutants.json

The spec: {"target": "scripts/ci.sh", "command": ["sh", "{copy}", "--selftest-gates"], "kill_marker": "GATE LOGIC WRONG", "mutants": [{"id", "what", "old", "new"}]}.
Each mutant is the target with ONE fault, placed at the same relative path in a temporary copy of the repository's scripts/
directory, and the command must FAIL (exit non-zero) on it. When the spec has a "kill_marker" the failure must also PRINT that text (the
self-test's own report of wrong logic): a script that merely stopped working, a syntax error for one, is not a kill (hale N-3). The
unmutated target must pass first. A mutant whose `old`
text is not in the target exactly once, that does not change it, or that the command does not fail on, FAILS the run.
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def run(command, copy):
    argv = [a.replace("{copy}", copy) for a in command]
    p = subprocess.run(argv, capture_output=True, text=True, timeout=180, check=False)
    return p.returncode, p.stdout + p.stderr


def main():
    spec = json.load(open(sys.argv[1], encoding="utf-8"))
    unknown = sorted(set(spec) - {"target", "command", "kill_marker", "mutants"})
    if unknown:  # a misspelt kill_marker must not silently turn the rule off (hale N-2)
        print(f"spec error: unknown key {unknown[0]!r} (allowed: target, command, kill_marker, mutants)")
        return 2
    target = os.path.join(ROOT, spec["target"])
    marker = spec.get("kill_marker")
    if "kill_marker" in spec and (not isinstance(marker, str) or not marker.strip()):  # an empty text is "in" every output: the rule would be off (hale N-3)
        print("spec error: kill_marker must be a non-empty text")
        return 2
    original = open(target, encoding="utf-8").read()
    rc, out = run(spec["command"], target)
    if rc != 0:
        print("BASELINE NOT GREEN: the unmutated target fails its own check")
        print("\n".join(out.splitlines()[-6:]))
        return 2
    print(f"baseline: the unmutated {spec['target']} passes: {out.strip().splitlines()[-1][:120]}")
    bad = 0
    for m in spec["mutants"]:
        n = original.count(m["old"])
        if n != 1:
            verdict = f"NOT APPLIED ({n} occurrences of the old text)"
        elif m["new"] == m["old"]:
            verdict = "VACUOUS (the mutant does not change the target)"
        else:
            with tempfile.TemporaryDirectory() as d:
                shutil.copytree(os.path.join(ROOT, "scripts"), os.path.join(d, "scripts"))
                copy = os.path.join(d, spec["target"])
                with open(copy, "w", encoding="utf-8") as f:
                    f.write(original.replace(m["old"], m["new"]))
                rc, out = run(spec["command"], copy)
            if rc == 0:
                verdict = "SURVIVED (the check still passes): vacuous for this fault"
            elif marker and marker not in out:
                verdict = "NOT A KILL (the check failed with exit %d but did not print %r)" % (rc, marker)
            else:
                verdict = "OK (the check failed: exit %d)" % rc
        ok = verdict.startswith("OK")
        bad += 0 if ok else 1
        print(f"{m['id']:7} {verdict}  [{m['what']}]")
    print(f"shell-mutants: {len(spec['mutants'])} mutants, {bad} not caught or broken")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
