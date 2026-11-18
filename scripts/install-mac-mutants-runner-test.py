"""Logic test for the skip rule of scripts/install_mac_mutants.py (mike MAC-MIKE-03 and MAC-MIKE-07): the runner judges a baseline or a mutant only on a run whose skips are the optional set cell IT-M29 and nothing else. It runs the runner's own run_suite against FAKE suites that print and exit as each case says.
    python3 -I scripts/install-mac-mutants-runner-test.py      (exit 0 = every case as it must be)
"""
import importlib.util
import os
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
spec = importlib.util.spec_from_file_location("runner", os.path.join(HERE, "install_mac_mutants.py"))
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)

# each case: (what, the fake suite's lines, its exit status, the mutant's expected-FAIL cells (None = the baseline), the later-lane cells (D176_CELLS) for the case, the required list, the runner's want)
REQ = ["IT-M01", "IT-M03", "IT-M29"]
S = lambda p, f, n: "install-mac-test: %d passed, %d failed, %d not run" % (p, f, n)
CASES = [
    ("baseline, exit 77, only IT-M29 skipped, complete: accepted", ["ok   IT-M01 a", "ok   IT-M03 b", "skip IT-M29 NOT RUN: no set", S(2, 0, 1)], 77, None, set(), REQ, 0),
    ("exit 77, IT-M25 / IT-M27 skipped: NOT judged", ["ok   IT-M01 a", "ok   IT-M03 b", "skip IT-M25 / IT-M27 NOT RUN: no python", S(2, 0, 1)], 77, None, set(), REQ, -1),
    ("exit 77, IT-M29 AND IT-M05b skipped: NOT judged", ["ok   IT-M01 a", "ok   IT-M03 b", "skip IT-M05b NOT RUN: x", "skip IT-M29 NOT RUN: y", S(2, 0, 2)], 77, None, set(), REQ, -1),
    ("exit 77 with no skip line at all: NOT judged", ["ok   IT-M01 a", "ok   IT-M03 b", "ok   IT-M29 c", S(3, 0, 0)], 77, None, set(), REQ, -1),
    ("exit 1, the expected FAIL AND a mandatory skip: NOT judged (MAC-MIKE-07)", ["ok   IT-M01 a", "FAIL IT-M03 x", "skip IT-M25 / IT-M27 NOT RUN: no python", S(1, 1, 1)], 1, ["IT-M03"], set(), REQ, -1),
    ("exit 1, the expected FAIL AND only IT-M29 skipped: judged", ["ok   IT-M01 a", "FAIL IT-M03 x", "skip IT-M29 NOT RUN: y", S(1, 1, 1)], 1, ["IT-M03"], set(), REQ, 1),
    ("a skip record with NO cell id: NOT judged (fail closed)", ["ok   IT-M01 a", "ok   IT-M03 b", "skip something odd NOT RUN: z", S(2, 0, 1)], 77, None, set(), REQ, -1),
    ("exit 1, a FAIL and a skip record with no NOT RUN field: NOT judged", ["ok   IT-M01 a", "FAIL IT-M03 x", "skip odd", S(1, 1, 1)], 1, ["IT-M03"], set(), REQ, -1),
    ("exit 0, complete: judged", ["ok   IT-M01 a", "ok   IT-M03 b", "ok   IT-M29 c", S(3, 0, 0)], 0, None, set(), REQ, 0),
    ("exit 1 with a FAIL, complete: judged", ["ok   IT-M01 a", "FAIL IT-M03 x", "ok   IT-M29 c", S(2, 1, 0)], 1, ["IT-M03"], set(), REQ, 1),
    ("a FAIL of the cell 'IT-M01..M02' counts as a failure of IT-M01 (a mutant names the leading id): judged", ["FAIL IT-M01..M02 x", "ok   IT-M03 b", "ok   IT-M29 c", S(2, 1, 0)], 1, ["IT-M01"], set(), ["IT-M01..M02", "IT-M03", "IT-M29"], 1),
    # MAC-MIKE-08: a run that prints its expected FAIL and then aborts, and a complete failing run
    ("the expected FAIL, then the suite ABORTS before its summary: NOT judged (MAC-MIKE-08)", ["ok   IT-M01 a", "FAIL IT-M03 x"], 1, ["IT-M03"], set(), REQ, -1),
    ("the same failing run, COMPLETE (every cell accounted, the summary consistent): judged (MAC-MIKE-08)", ["ok   IT-M01 a", "FAIL IT-M03 x", "ok   IT-M29 c", S(2, 1, 0)], 1, ["IT-M03"], set(), REQ, 1),
    ("the summary says 5 passed but the output holds 3 ok records: NOT judged", ["ok   IT-M01 a", "ok   IT-M03 b", "ok   IT-M29 c", S(5, 0, 0)], 0, None, set(), REQ, -1),
    ("a cell with two result records: NOT judged", ["ok   IT-M01 a", "ok   IT-M01 again", "ok   IT-M03 b", "ok   IT-M29 c", S(4, 0, 0)], 0, None, set(), REQ, -1),
    ("a required cell with no result record at all (consistent summary): NOT judged", ["ok   IT-M01 a", "ok   IT-M29 c", S(2, 0, 0)], 0, None, set(), REQ, -1),
    ("a result for a cell that is not in the required list: NOT judged", ["ok   IT-M01 a", "ok   IT-M03 b", "ok   IT-M29 c", "ok   IT-M77 d", S(4, 0, 0)], 0, None, set(), REQ, -1),
    ("two summary lines: NOT judged", ["ok   IT-M01 a", "ok   IT-M03 b", "ok   IT-M29 c", S(3, 0, 0), S(3, 0, 0)], 0, None, set(), REQ, -1),
    ("the summary says all passed but the suite exited 1: NOT judged", ["ok   IT-M01 a", "ok   IT-M03 b", "ok   IT-M29 c", S(3, 0, 0)], 1, None, set(), REQ, -1),
    ("a FAIL record and exit 0: NOT judged", ["ok   IT-M01 a", "FAIL IT-M03 x", "ok   IT-M29 c", S(2, 1, 0)], 0, ["IT-M03"], set(), REQ, -1),
    # MAC-MIKE-12: a FAIL accounts for its OWN cell by an explicit mapping, not by a prefix
    ("FAIL IT-M10 (the cell IT-M10a..d) and NO records for IT-M10e, IT-M10f, IT-M10g: NOT judged (MAC-MIKE-12)", ["ok   IT-M01 a", "FAIL IT-M10 x", "ok   IT-M29 c", S(2, 1, 0)], 1, ["IT-M10"], set(), ["IT-M01", "IT-M10a..d", "IT-M10e", "IT-M10f", "IT-M10g", "IT-M29"], -1),
    ("FAIL IT-M10 and the three other IT-M10 cells accounted for: judged (the label maps to IT-M10a..d)", ["ok   IT-M01 a", "FAIL IT-M10 x", "ok   IT-M10e e", "ok   IT-M10f f", "ok   IT-M10g g", "ok   IT-M29 c", S(5, 1, 0)], 1, ["IT-M10"], set(), ["IT-M01", "IT-M10a..d", "IT-M10e", "IT-M10f", "IT-M10g", "IT-M29"], 1),
    ("FAIL IT-M03 and NO record for IT-M03b: NOT judged (MAC-MIKE-12)", ["ok   IT-M01 a", "FAIL IT-M03 x", "ok   IT-M29 c", S(2, 1, 0)], 1, ["IT-M03"], set(), ["IT-M01", "IT-M03", "IT-M03b", "IT-M29"], -1),
    ("an UNKNOWN FAIL label 'IT-M03bogus' beside a required IT-M03: NOT judged (no reverse-prefix match)", ["ok   IT-M01 a", "FAIL IT-M03bogus x", "ok   IT-M29 c", S(2, 1, 0)], 1, ["IT-M03"], set(), ["IT-M01", "IT-M03", "IT-M29"], -1),
    ("a suite-level FAIL IT-M00 (not a cell): NOT judged", ["ok   IT-M01 a", "ok   IT-M03 b", "FAIL IT-M00 x", "ok   IT-M29 c", S(3, 1, 0)], 1, ["IT-M03"], set(), REQ, -1),
    ("two FAIL records for the same cell: NOT judged (duplicate)", ["ok   IT-M01 a", "FAIL IT-M03 x", "FAIL IT-M03 again", "ok   IT-M29 c", S(2, 2, 0)], 1, ["IT-M03"], set(), REQ, -1),
    ("an ok AND a FAIL record for the same cell: NOT judged (conflicting)", ["ok   IT-M01 a", "ok   IT-M03 b", "FAIL IT-M03 x", "ok   IT-M29 c", S(3, 1, 0)], 1, ["IT-M03"], set(), REQ, -1),
    ("an ok AND a skip record for IT-M29 (exit 77, the summary and the exit agree): NOT judged (MAC-MIKE-12)", ["ok   IT-M01 a", "ok   IT-M03 b", "ok   IT-M29 c", "skip IT-M29 NOT RUN: no set", S(3, 0, 1)], 77, None, set(), REQ, -1),
    ("a FAIL AND a skip record for IT-M29 (exit 1): NOT judged (MAC-MIKE-12)", ["ok   IT-M01 a", "ok   IT-M03 b", "FAIL IT-M29 x", "skip IT-M29 NOT RUN: no set", S(2, 1, 1)], 1, ["IT-M29"], set(), REQ, -1),
    ("two skip records for IT-M29 (exit 77): NOT judged (MAC-MIKE-12)", ["ok   IT-M01 a", "ok   IT-M03 b", "skip IT-M29 NOT RUN: no set", "skip IT-M29 NOT RUN: again", S(2, 0, 2)], 77, None, set(), REQ, -1),
    ("ONE legitimate skip of IT-M29 beside a FAIL of another cell: judged", ["ok   IT-M01 a", "FAIL IT-M03 x", "skip IT-M29 NOT RUN: no set", S(1, 1, 1)], 1, ["IT-M03"], set(), REQ, 1),
    ("a mutant run in which the deliberately omitted IT-M38 printed a result: NOT judged", ["ok   IT-M01 a", "FAIL IT-M03 x", "ok   IT-M38 z", "ok   IT-M29 c", S(3, 1, 0)], 1, ["IT-M03"], set(), REQ + ["IT-M38"], -1),
    ("a mutant run in which a later-lane cell it does not name printed a result: NOT judged", ["ok   IT-M01 a", "FAIL IT-M03 x", "ok   IT-M31 z", "ok   IT-M29 c", S(3, 1, 0)], 1, ["IT-M03"], {"IT-M31"}, REQ + ["IT-M31"], -1),
    # MAC-MIKE-09: the skipped-cell field is parsed as a field
    ("a MIXED known and unknown skip field 'IT-M29 / UNKNOWN': NOT judged (MAC-MIKE-09)", ["ok   IT-M01 a", "ok   IT-M03 b", "skip IT-M29 / UNKNOWN NOT RUN: x", S(2, 0, 1)], 77, None, set(), REQ, -1),
    ("an id-less skipped cell whose REASON names IT-M29: NOT judged (MAC-MIKE-09)", ["ok   IT-M01 a", "ok   IT-M03 b", "skip odd cell NOT RUN: it needs IT-M29", S(2, 0, 1)], 77, None, set(), REQ, -1),
    ("a skip field with the id and a trailing word 'IT-M29 extra': NOT judged", ["ok   IT-M01 a", "ok   IT-M03 b", "skip IT-M29 extra NOT RUN: x", S(2, 0, 1)], 77, None, set(), REQ, -1),
    # the deliberate omissions of a mutant run: the suite's own self-test IT-M38, and the later-lane cells a mutant does not name
    ("a mutant run without IT-M38 (its self-test is not repeated): judged", ["ok   IT-M01 a", "FAIL IT-M03 x", "ok   IT-M29 c", S(2, 1, 0)], 1, ["IT-M03"], set(), REQ + ["IT-M38"], 1),
    ("the BASELINE without IT-M38: NOT judged", ["ok   IT-M01 a", "ok   IT-M03 b", "ok   IT-M29 c", S(3, 0, 0)], 0, None, set(), REQ + ["IT-M38"], -1),
    ("a mutant run without a later-lane cell it does not name: judged", ["ok   IT-M01 a", "FAIL IT-M03 x", "ok   IT-M29 c", S(2, 1, 0)], 1, ["IT-M03"], {"IT-M31"}, REQ + ["IT-M31"], 1),
    ("a mutant run without the later-lane cell it NAMES: NOT judged", ["ok   IT-M01 a", "ok   IT-M03 b", "ok   IT-M29 c", S(3, 0, 0)], 0, ["IT-M31"], {"IT-M31"}, REQ + ["IT-M31"], -1),
    ("the BASELINE without a later-lane cell: NOT judged", ["ok   IT-M01 a", "ok   IT-M03 b", "ok   IT-M29 c", S(3, 0, 0)], 0, None, {"IT-M31"}, REQ + ["IT-M31"], -1),
]


def main():
    bad = 0
    with tempfile.TemporaryDirectory() as d:
        for what, lines, rc, fails, later, req, want in CASES:
            p = os.path.join(d, "fake.sh")
            with open(p, "w") as f:
                f.write("#!/bin/sh\n" + "".join("echo '%s'\n" % ln.replace("'", "'\\''") for ln in lines) + "exit %d\n" % rc)
            m.TEST = p
            m.D176_CELLS = set(later)
            m.required_ids = (lambda r: (lambda: list(r)))(req)
            got, failed, out = m.run_suite("/x", fails)
            good = got == want
            bad += 0 if good else 1
            print(("ok   " if good else "WRONG ") + what + " (got %s)" % got)
            if good and want == 1 and fails and not set(fails) <= failed:
                bad += 1
                print("WRONG the failed set lost " + ", ".join(sorted(set(fails) - failed)))
    print("install-mac-mutants-runner-test: %d cases, %d wrong" % (len(CASES), bad))
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
