"""Mutation run for deploy/install-fetch.sh (6c; the same discipline as install_mutants.py).

A mutant is install-fetch.sh with ONE deliberate fault, in a COPY. The named cells (for example tr02, ir14b) must FAIL on it.
A mutant whose `old` text is not in the file exactly once, that does not change it, or that no cell catches, FAILS the run.
The unmutated file must pass the suite first. `fails: ["__collect__"]` is the candidate mode: every cell that fails is printed.

    scripts/cv2-admit-shim python3 -I -B scripts/install_fetch_mutants.py [ids...] [spec.json]
"""
import json
import os
import re
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
SUBJECT = os.path.join(os.path.dirname(HERE), "deploy", "install-fetch.sh")
TEST = os.path.join(HERE, "install-fetch-test.sh")
SPEC = os.path.join(HERE, "install-fetch-mutants.json")


def suite_command(named=None, classes=None, failfast=False):
    """The command that runs the cells: the whole suite, or only the named cells (a cell id such as tr02 selects test_tr02_* in every class), optionally only in the given classes."""
    cmd = ["sh", TEST] + (["-f"] if failfast else [])   # failfast only while COLLECTING candidates
    for cell in named or []:
        cmd += ["-k", f"test_{cell}_"]
    return cmd + (classes or [])


def named_classes(cells):
    """cox W1-109 (a), option B. A mutant that names a fault-sweep cell (zf*) is judged against the ONE class that DEFINES the cell (ZFaults for zf01 and zf02), not against all six
    zf classes (about 11.6 minutes each in all six, inc25): the other five repeat the cell under another environment. Any other cell, or a mix, keeps every class (None).
    A list entry may carry its own "classes" (a mutant that only another environment can see, e.g. the default of an EX_* code, which only the NoEx variants leave unset); it wins."""
    if not cells or not all(re.fullmatch(r"zf[0-9]+[a-z]?", c) for c in cells):
        return None
    text = open(TEST, encoding="utf-8").read()
    bodies = re.split(r"(?m)^class (\w+)\(", text)   # [before, name, body, name, body, ...]
    carriers = set()
    for cell in cells:
        owners = [bodies[i] for i in range(1, len(bodies), 2) if re.search(rf"(?m)^    def test_{cell}_", bodies[i + 1])]
        if len(owners) != 1:
            return None
        carriers.add(owners[0])
    return sorted(carriers) if len(carriers) == 1 else None


def run_suite(subject_path, named=None, classes=None):
    """The whole suite, or (the final run) only the cells the mutant names: a cell id such as tr02 selects test_tr02_* in every class, with and without the
    caller's symbols. The unmutated file always runs the whole suite first."""
    env = dict(os.environ, INSTALL_FETCH_SH=subject_path)
    cmd = suite_command(named, classes, bool(os.environ.get("INSTALL_FETCH_FAILFAST")))
    p = subprocess.run(cmd, capture_output=True, text=True, timeout=int(os.environ.get("INSTALL_FETCH_TIMEOUT", "300")), env=env, check=False)   # the stage 2 suite needs about 13 minutes
    out = p.stdout + p.stderr
    failed = {m.group(1) for m in re.finditer(r"^(?:FAIL|ERROR): test_([a-z]{2}[0-9]+[a-z]?)_", out, re.M)}
    return p.returncode, failed, out


def run_tiers(subject_path):
    """Candidate collection only (INSTALL_FETCH_TIERS): the cheap classes first, then the one-pass message cells (zf02), then the full fault sweep of the two base fault
    classes. A candidate that passes all three is a SURVIVOR OF THE FAST TIERS; the rest of the suite (the four other fault variants) is run on those by the
    delta, with INSTALL_FETCH_TIERS unset. Each tier stops at its first failure."""
    names = re.findall(r"^class (\w+)\(", open(TEST, encoding="utf-8").read(), re.M)
    light = [c for c in names if not c.startswith("ZFaults")]
    for classes, named in ((light, None), (None, ["zf02"]), (["ZFaults", "ZFaultsNoEx"], None)):
        rc, failed, out = run_suite(subject_path, named, classes)
        if rc != 0:
            return rc, failed, out
    return 0, set(), ""


def main():
    args = sys.argv[1:]
    spec_path = SPEC
    for a in list(args):
        if a.endswith(".json"):
            spec_path = a
            args.remove(a)
    spec = json.load(open(spec_path, encoding="utf-8"))
    original = open(SUBJECT, encoding="utf-8").read()
    if os.environ.get("INSTALL_FETCH_NO_BASELINE") and os.environ.get("INSTALL_FETCH_FAILFAST"):   # candidate collection only: the baseline run is a receipt of its own
        print("baseline: NOT RUN here (INSTALL_FETCH_NO_BASELINE, collect mode); the same tree has its own whole-suite receipt")
    else:
        rc, _, out = run_suite(SUBJECT)
        if rc != 0:
            print("BASELINE NOT GREEN: the unmutated install-fetch.sh fails its own suite")
            print("\n".join(out.splitlines()[-8:]))
            return 2
        ran = re.search(r"^Ran (\d+) tests", out, re.M)
        print(f"baseline: the unmutated install-fetch.sh passes ({ran.group(1) if ran else '?'} cells)")
    bad = 0
    only = set(args)
    for m in spec:
        if only and m["id"] not in only:
            continue
        n = original.count(m["old"])
        if n != 1:
            verdict = f"NOT APPLIED ({n} occurrences of the old text)"
        elif m["new"] == m["old"]:
            verdict = "VACUOUS (the mutant does not change the file)"
        else:
            with tempfile.TemporaryDirectory(dir=os.environ.get("TMPDIR")) as d:
                copy = os.path.join(d, "install-fetch.sh")
                with open(copy, "w", encoding="utf-8") as f:
                    f.write(original.replace(m["old"], m["new"]))
                sx = subprocess.run(["sh", "-n", copy], capture_output=True, text=True, check=False)
                if sx.returncode != 0:
                    rc, failed, out = 0, set(), ""
                    verdict = "DOES NOT PARSE (sh -n): " + sx.stderr.strip().splitlines()[0][:100]
                    print(f"{m['id']:7} {verdict}  [{m['what']}]")
                    bad += 1
                    continue
                if os.environ.get("INSTALL_FETCH_TIERS") and os.environ.get("INSTALL_FETCH_FAILFAST"):
                    rc, failed, out = run_tiers(copy)
                else:
                    named = m["fails"] if os.environ.get("INSTALL_FETCH_NAMED") and m["fails"] != ["__collect__"] else None
                    rc, failed, out = run_suite(copy, named, (m.get("classes") or named_classes(named)) if named else None)   # an entry's own classes win (cox W1-109 (a): the NoEx variants carry the EX_* defaults)
            if rc == 0:
                verdict = "SURVIVED (the whole suite passed" + (", fast tiers only" if os.environ.get("INSTALL_FETCH_TIERS") else "") + "): the cells are vacuous for this fault"
            else:
                missing = [t for t in m["fails"] if t not in failed]
                verdict = "OK (" + ", ".join(sorted(failed)) + ")" if not missing else f"WRONG TEST: {missing} did not fail (failed: {sorted(failed)})"
        ok = verdict.startswith("OK")
        bad += 0 if ok else 1
        print(f"{m['id']:7} {verdict}  [{m['what']}]")
    print(f"install-fetch-mutants: {len([m for m in spec if not only or m['id'] in only])} mutants, {bad} not caught or broken")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
