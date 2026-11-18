#!/usr/bin/env python3
"""Run one-fault builder mutations in disposable, private copies; reject vacuous faults."""
import json
import os
import re
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
MUTANTS = ROOT / "crates/cortex-upgrade/mutants/release.json"


def run_tests(root, only=None):
    # a mutant may name the one test it is judged by ("only"); the baseline and the older mutants run the whole suite
    return subprocess.run(["sh", str(root / "scripts/release-test.sh"), *(["-k", only] if only else [])], cwd=root,
                          text=True, capture_output=True, timeout=420)


def cell_for(mutant, cell_only):
    """The one cell a mutant is judged by: its own "only", or with --cell-only (a SCREEN) its named failing cell; None means the whole suite."""
    return mutant.get("only") or (mutant["fails"] if cell_only else None)


def judge(mutant, returncode, out):
    """OK only if the run failed, the named cell is among the failures and nothing raised an error; a run that passed is a survivor."""
    if returncode == 0:
        return "SURVIVED"
    if "ERROR:" in out or "FAIL: " + mutant["fails"] + " " not in out:
        return "BROKEN OR WRONG TEST"
    return "OK"


def parse_args(argv):
    """(cell_only, selector) from the command line: [--cell-only] and at most one of `first last` (a 1-based inclusive slice) or `--ids RM-01,RM-02`."""
    cell_only = "--cell-only" in argv
    rest = [a for a in argv if a != "--cell-only"]
    if not rest:
        return cell_only, None
    if len(rest) == 2 and rest[0] == "--ids":
        return cell_only, ("ids", rest[1].split(","))
    if len(rest) == 2 and all(a.isdigit() for a in rest):
        return cell_only, ("slice", int(rest[0]), int(rest[1]))
    raise SystemExit("usage: run_mutants.py [--cell-only] [first last | --ids RM-01,RM-02]")


def main():
    cell_only, selector = parse_args(sys.argv[1:])
    mutants = json.loads(MUTANTS.read_text())
    if selector and selector[0] == "ids":  # the mutants with these ids, each id must exist
        wanted = selector[1]
        known = {m["id"] for m in mutants}
        if any(w not in known for w in wanted):
            print("UNKNOWN MUTANT ID", [w for w in wanted if w not in known])
            return 1
        mutants = [m for m in mutants if m["id"] in wanted]
    elif selector:  # the slice: first and last position (1-based, inclusive)
        mutants = mutants[selector[1] - 1:selector[2]]
    if cell_only:
        print("SCREEN: the baseline runs every cell; each mutant is judged by its own named cell only (not a whole-suite verdict)", flush=True)
    with tempfile.TemporaryDirectory(prefix="release-mutants-", dir=os.environ["TMPDIR"]) as tmp:
        copy = Path(tmp)
        (copy / "scripts/release-fixtures").mkdir(parents=True)
        for name in ["release.sh", "release-test.sh", "release-squawk.toml", "release_manifest.py", "release-migration-exceptions.json", "release-migration-approval.json", "release-fixtures/vera-gate4-verdicts.json", "release-fixtures/vera-consolidated-rulings.json", "release-fixtures/test_release.py", "release-fixtures/run_mutants.py", "release-fixtures/server-features.fixture",
                     "s5-check.sh", "s5_check.py"]:
            shutil.copy(ROOT / "scripts" / name, copy / "scripts" / name)
        # X-1: the cell that runs the REAL S5 checker on the release layout needs the real deploy/ and the server manifest.
        shutil.copytree(ROOT / "deploy", copy / "deploy")
        (copy / "crates/cortex-server").mkdir(parents=True)
        shutil.copy(ROOT / "crates/cortex-server/Cargo.toml", copy / "crates/cortex-server/Cargo.toml")
        migration = Path("crates/cortex-db/migrations/platform/0001_platform.up.sql")
        (copy / migration).parent.mkdir(parents=True)
        shutil.copy(ROOT / migration, copy / migration)
        # the squawk-approval cells (test_sq_c1, c3, d1) read the five module migrations and the six real squawk reports: the private copy carries them too
        for relative in ("crates/mod-audit/migrations/0001_audit.up.sql", "crates/mod-handoffs/migrations/0001_handoffs.up.sql", "crates/mod-identity/migrations/0001_identity.up.sql",
                         "crates/mod-records/migrations/0001_records.up.sql", "crates/mod-tokens/migrations/0001_tokens.up.sql"):
            (copy / relative).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy(ROOT / relative, copy / relative)
        shutil.copytree(ROOT / "scripts/release-fixtures/squawk-real-reports", copy / "scripts/release-fixtures/squawk-real-reports")
        baseline = run_tests(copy)
        if baseline.returncode or "OK" not in baseline.stderr:
            print("BASELINE FAILED", baseline.stdout, baseline.stderr)
            return 1
        count = re.search(r"Ran (\d+) tests? in", baseline.stderr)
        if not count or int(count.group(1)) < 1:
            print("BASELINE HAS NO TESTS")
            return 1
        print(f"baseline: {count.group(1)} builder tests pass", flush=True)
        bad = 0
        for mutant in mutants:
            target = copy / mutant["file"]
            original = target.read_text()
            old, new = mutant["old"], mutant["new"]
            if original.count(old) != 1 or old == new:
                verdict = "NOT APPLIED OR VACUOUS"
            else:
                try:
                    target.write_text(original.replace(old, new))
                    result = run_tests(copy, cell_for(mutant, cell_only))
                    verdict = judge(mutant, result.returncode, result.stdout + result.stderr)
                except subprocess.TimeoutExpired:
                    verdict = "TIMEOUT"
                finally:
                    target.write_text(original)
            if verdict != "OK":
                bad += 1
            print(f"{mutant['id']} {verdict} [{mutant['what']}]", flush=True)
        if cell_only:
            print(f"screen: {len(mutants)} mutants, each judged by its named cell only; killed {len(mutants) - bad}, NOT killed or broken {bad} (each of those goes to a WHOLE-SUITE run and is not a pass until that run kills it)")
        else:
            print(f"mutants: {len(mutants)}, not caught or broken: {bad}")
        return 1 if bad else 0

if __name__ == "__main__":
    sys.exit(main())
