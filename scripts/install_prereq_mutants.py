#!/usr/bin/env python3
"""Run named prerequisite mutations in a private copy, never in the lane.

Run through cv2-admit. Optional arguments select mutant IDs; every mutant must
apply exactly once, parse, and fail its named test. The baseline must pass.
"""
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / "deploy/install-prereq.sh"
TEST = ROOT / "scripts/install-prereq-test.sh"
SPEC = ROOT / "scripts/install-prereq-mutants.json"
ADMIT = "/path/to/workspace/spikes/cortex-v2-s0/admission/cv2-admit"
SCRATCH = Path(os.environ.get("PREREQ_MUTANT_TMP", "/path/to/workspace/scratch/marv/s5-prereq/mutants")).resolve()


def suite(source, locale=None, named=None):
    env = dict(os.environ, PREREQ_TEST_SOURCE=str(source), PREREQ_TEST_TMP=str(SCRATCH), TMPDIR=str(SCRATCH))
    env.pop("PREREQ_SCREEN_MUTANT", None)
    if named is not None:
        env["PREREQ_SCREEN_MUTANT"] = named
    # Every successful suite invocation must complete both required locales.
    # Reverse their order when exercising the second locale so a mutant which
    # fails early in the first block is still exercised under both locales.
    env["PREREQ_TEST_LOCALES"] = "C.utf8 C" if locale == "C.utf8" else "C C.utf8"
    if locale is not None:
        env["LC_ALL"] = locale
    result = subprocess.run([ADMIT, "sh", str(TEST)], cwd=ROOT, env=env, text=True,
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=False)
    return result.returncode, result.stdout


def main():
    if SCRATCH == Path("/tmp") or Path("/tmp") in SCRATCH.parents:
        print("REFUSED: mutation scratch is under /tmp", file=sys.stderr)
        return 2
    SCRATCH.mkdir(parents=True, exist_ok=True)
    mutations = json.loads(SPEC.read_text(encoding="utf-8"))
    selected = set(sys.argv[1:])
    if selected - {m["id"] for m in mutations}:
        print("unknown mutation ID", file=sys.stderr)
        return 2
    original = SOURCE.read_text(encoding="utf-8")
    invalid = [m["id"] for m in mutations
               if not m["old"] or original.count(m["old"]) != 1 or m["old"] == m["new"]]
    if invalid:
        print("INVALID MUTANT ANCHORS: " + ", ".join(invalid), file=sys.stderr)
        return 2
    locale_env = dict(os.environ, LC_ALL="C")
    check = subprocess.run(["locale", "charmap"], env=locale_env, capture_output=True, text=True, check=False)
    if check.returncode != 0:
        print("BASELINE FAILED: C locale unavailable")
        return 2
    locales = ["C"]
    locale_env = dict(os.environ, LC_ALL="C.utf8")
    check = subprocess.run(["locale", "charmap"], env=locale_env, capture_output=True, text=True, check=False)
    if check.returncode != 0 or check.stderr.strip() or check.stdout.strip().upper().replace("-", "") != "UTF8":
        print("BASELINE FAILED: C.utf8 locale unavailable or not UTF-8")
        return 2
    locales.append("C.utf8")
    print("mutation locales: " + ", ".join(locales), flush=True)
    for locale in locales:
        rc, output = suite(SOURCE, locale)
        executed = all(re.search(r"^ok .+ \(" + re.escape(required) + r"\)$", output, re.MULTILINE)
                       for required in locales)
        if rc or not executed:
            print("BASELINE FAILED (" + locale + "): no passing locale block or suite failed\n" + output)
            return 2
        print("baseline passed (" + locale + ")", flush=True)
    bad = 0
    total = 0
    with tempfile.TemporaryDirectory(prefix="prereq-mutants-", dir=SCRATCH) as directory:
        candidate = Path(directory) / "install-prereq.sh"
        for mutation in mutations:
            if selected and mutation["id"] not in selected:
                continue
            total += 1
            old, new = mutation["old"], mutation["new"]
            if original.count(old) != 1 or old == new:
                verdict = "NOT APPLIED OR VACUOUS"
            else:
                candidate.write_text(original.replace(old, new, 1), encoding="utf-8")
                syntax = subprocess.run(["sh", "-n", str(candidate)], capture_output=True, text=True, check=False)
                if syntax.returncode:
                    verdict = "DOES NOT PARSE"
                else:
                    expected = mutation["fails"]
                    verdict = "OK " + expected
                    for locale in locales:
                        if os.environ.get("PREREQ_MUTANT_FAST") == "1":
                            rc, output = suite(candidate, locale, named=mutation["id"])
                        else:
                            rc, output = suite(candidate, locale)
                        first_failure = next((line for line in output.splitlines()
                                              if line.startswith("FAIL ")), None)
                        matched = first_failure is not None and first_failure.startswith(expected)
                        suffix = first_failure[len(expected):] if matched else ""
                        if rc == 0 or not matched or (suffix and (suffix[0].isalnum() or suffix[0] in "_-")):
                            verdict = "SURVIVED OR WRONG TEST (" + locale + ")"
                            break
            if not verdict.startswith("OK "):
                bad += 1
            print(f"{mutation['id']} {verdict}: {mutation['what']}", flush=True)
    if SOURCE.read_text(encoding="utf-8") != original:
        print("SOURCE CHANGED DURING MUTATION RUN", file=sys.stderr)
        return 2
    print(f"prerequisite mutants: {total}, not caught or broken: {bad}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
