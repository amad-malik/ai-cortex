"""Mutation run for mac/install-mac.sh and mac/install-prereq.sh (the Mac installer; the same discipline as scripts/install_mutants.py, which judges the Linux installer and is NOT touched).

A mutant is the Mac installer (or its check-only include) with ONE deliberate fault, in a COPY of mac/. The named IT-M cells must FAIL on it. A mutant whose `old` text is not in its file exactly once, that does
not change it, that is a shell syntax error, or that no cell catches, FAILS the run. The unmutated installer must pass the suite first.

    python3 -I -B scripts/install_mac_mutants.py --once     # ONLY the exactly-once check (every old text occurs once in its file, no mutant is vacuous): run it FIRST, as its own receipt
    python3 -I -B scripts/install_mac_mutants.py [ID ...]   # the baseline, then every mutant (or the named ones)

The limit, stated: these mutants judge install-mac.sh's own logic against FAKE tools (a fake docker, fake GNU and BSD tool directories, a fake launchd tool): they prove nothing about Docker Desktop, Homebrew's tools or Apple's sh.
"""
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
MAC = os.path.join(REPO, "mac")
TEST = os.path.join(HERE, "install-mac-test.sh")
SPEC = os.path.join(HERE, "install-mac-mutants.json")


ALLOWED_SKIP = {"IT-M29"}   # the one cell a mutant run may skip: it needs a signed release set that only the release run has
REQUIRED = os.path.join(HERE, "install-mac-required-cells.txt")
D176_CELLS = set()   # the cells that run in a mutant's run ONLY when the mutant names them (INSTALL_MAC_ONLY): none in this lane; a lane that adds such cells lists them here
OMITTED_IN_MUTANT_MODE = {"IT-M38"}   # the suite's own self-test: a mutant's run does not repeat it (INSTALL_MAC_NO_SELFRUN=1)
FAIL_LABELS = {"IT-M10": "IT-M10a..d"}   # the only cell whose FAIL label differs from its required id (the suite prints bad "IT-M10 ..." for the cell it passes as ok "IT-M10a..d ...")
SUMMARY = re.compile(r"^install-mac-test: ([0-9]+) passed, ([0-9]+) failed, ([0-9]+) not run$")


def required_ids():
    ids = []
    with open(REQUIRED, encoding="utf-8") as f:
        for ln in f:
            ln = ln.strip()
            if ln and not ln.startswith("#"):
                ids.append(ln)
    return ids


def judge_output(out, returncode, mutant_named=None):
    """Fail-closed accounting of ONE run of the suite (mike MAC-MIKE-07, -08, -09). Returns (problem, failed): problem is None only when the output is COMPLETE and CONSISTENT, so a run that printed its expected FAIL and then aborted, a malformed
    skip record, a duplicated or unknown cell, a missing cell or a summary that does not match the records is NOT judged. mutant_named is None for the baseline (every required cell runs) and, for a mutant, the set of the later-lane cells it names."""
    lines = out.splitlines()
    ok_ids = [ln.split()[1] for ln in lines if ln.startswith("ok   ") and len(ln.split()) > 1]
    fail_ids = [ln.split()[1] for ln in lines if ln.startswith("FAIL ") and len(ln.split()) > 1]
    skips = [ln for ln in lines if ln.startswith("skip ")]
    # the skipped-cell field is parsed AS A FIELD: the text between 'skip ' and ' NOT RUN:' must be exactly one permitted cell id (a mixed record, a record with no cell id or a reason that merely mentions IT-M29 is not a skip of IT-M29)
    fields = []
    for ln in skips:
        head, sep, _ = ln[len("skip "):].partition(" NOT RUN:")
        fields.append(head.strip() if sep else None)
    for ln, fld in zip(skips, fields):
        if fld not in ALLOWED_SKIP:
            return "MANDATORY CELL NOT RUN (a skip whose cell field is not exactly " + ", ".join(sorted(ALLOWED_SKIP)) + ", in a run that exited " + str(returncode) + "): " + ln[:100], set()
    sums = [mm for mm in (SUMMARY.match(ln) for ln in lines) if mm]
    if len(sums) != 1:
        return "INCOMPLETE RUN (not exactly one suite summary line: the suite aborted before its end or printed several): exit " + str(returncode) + ", last line: " + (lines[-1][:80] if lines else ""), set()
    passed, failed_n, notrun = (int(x) for x in sums[0].groups())
    if (passed, failed_n, notrun) != (len(ok_ids), len(fail_ids), len(skips)):
        return "INCONSISTENT RUN (the summary says %d passed, %d failed, %d not run; the output holds %d ok, %d FAIL and %d skip records)" % (passed, failed_n, notrun, len(ok_ids), len(fail_ids), len(skips)), set()
    want_rc = 1 if failed_n else (77 if notrun else 0)
    if returncode != want_rc:
        return "INCONSISTENT RUN (the summary wants exit %d, the suite exited %s)" % (want_rc, returncode), set()
    required = set(required_ids())
    # (mike MAC-MIKE-12) a FAIL record names its cell by an EXPLICIT mapping, never by a prefix: the suite's own labels are the required cell's id, except the one grouped label in FAIL_LABELS (the cell 'IT-M10a..d' fails as 'IT-M10'). A label that is
    # neither a required id nor a mapped label (a suite-level 'IT-M00', 'IT-M03bogus') is UNKNOWN; a cell with two FAIL records, or with an ok record AND a FAIL record, has conflicting results. Matching a mutant's expected cells is separate (judge below).
    cells_of = [FAIL_LABELS.get(c, c) for c in fail_ids]
    unknown = sorted({c for c in ok_ids if c not in required} | {c for c, mc in zip(fail_ids, cells_of) if mc not in required})
    if unknown:
        return "INCONSISTENT RUN (a result for a cell that is not in the required list: " + ", ".join(unknown) + ")", set()
    # every result record of every kind (ok, FAIL, and the permitted skip) counts toward the cell's ONE result: two of one kind, or two kinds (ok and skip, FAIL and skip, ok and FAIL), are conflicting results (mike MAC-MIKE-12)
    results = ok_ids + cells_of + [f for f in fields if f]
    dup = sorted({c for c in results if results.count(c) > 1})
    if dup:
        return "INCONSISTENT RUN (a cell has more than one result record, or two kinds of result (ok, FAIL, skip): " + ", ".join(dup) + ")", set()
    accounted = set(results)
    omitted = set(fields)
    if mutant_named is not None:
        omitted |= OMITTED_IN_MUTANT_MODE | (D176_CELLS - mutant_named)
    if mutant_named is not None:
        deliberately = (OMITTED_IN_MUTANT_MODE | (D176_CELLS - mutant_named)) & set(ok_ids + cells_of)
        if deliberately:   # a cell that a mutant run omits on purpose has no business printing a result
            return "INCONSISTENT RUN (a cell that is omitted on purpose in a mutant run printed a result: " + ", ".join(sorted(deliberately)) + ")", set()
    missing = sorted(required - accounted - omitted)
    if missing:
        return "INCOMPLETE RUN (no result record for the cell(s) " + ", ".join(missing[:6]) + ": the run did not account for every cell)", set()
    failed = set(fail_ids) | set(cells_of)
    for c in list(failed):   # a mutant's spec names the cell by its leading id ('IT-M01' for the cell 'IT-M01..M02', 'IT-M10' for 'IT-M10a..d'): matching the EXPECTED cells, separate from the accounting above
        mm = re.match(r"IT-M[0-9]+[a-z]*", c)
        if mm:
            failed.add(mm.group(0))
    return None, failed


def run_suite(installer, fails=None):
    env = dict(os.environ)
    env["INSTALL_MAC_SH"] = installer
    named = None
    if fails is not None:
        env["INSTALL_MAC_NO_SELFRUN"] = "1"   # a mutant's run does not repeat the suite's own self-test (IT-M38: it tests the suite, not the installer); the baseline runs it
        named = {f for f in fails if f in D176_CELLS}
        if D176_CELLS:
            env["INSTALL_MAC_ONLY"] = " ".join(sorted(named)) or "NONE"   # only the later-lane cells the mutant names run (a mutant is judged by the cells it names)
    try:
        p = subprocess.run(["sh", TEST], capture_output=True, text=True, timeout=int(os.environ.get("INSTALL_MAC_MUTANT_TIMEOUT", "300")), env=env, check=False)
    except subprocess.TimeoutExpired:
        return None, set(), "TIMED OUT"
    out = p.stdout + p.stderr
    problem, failed = judge_output(out, p.returncode, named)
    if problem:
        return -1, failed, problem
    if p.returncode == 77 and not failed:
        return 0, failed, out
    failed |= {re.sub(r"[a-z]+$", "", f) for f in failed}   # IT-M03b also counts as IT-M03: a spec may name the base or the exact id
    return p.returncode, failed, out


def main():
    args = sys.argv[1:]
    once = "--once" in args
    only = {a for a in args if a != "--once"}
    spec = json.load(open(SPEC, encoding="utf-8"))
    texts = {}

    def text_of(name):
        if name not in texts:
            texts[name] = open(os.path.join(MAC, name), encoding="utf-8").read()
        return texts[name]

    bad = 0
    for m in spec:
        n = text_of(m["file"]).count(m["old"])
        problem = None if n == 1 else f"{n} occurrences of the old text (want exactly 1)"
        if problem is None and m["new"] == m["old"]:
            problem = "VACUOUS (the mutant does not change the file)"
        if once:
            print(f"{m['id']:8} {'ONCE' if problem is None else 'NOT APPLIED: ' + problem}  [{m['what']}]")
            bad += 0 if problem is None else 1
    if once:
        print(f"install-mac-mutants --once: {len(spec)} mutants, {bad} not applicable")
        return 1 if bad else 0
    with tempfile.TemporaryDirectory() as bd:
        shutil.copytree(MAC, os.path.join(bd, "mac"))
        rc, _, out = run_suite(os.path.join(bd, "mac", "install-mac.sh"))
    if rc != 0:
        print("BASELINE NOT GREEN: the unmutated installer fails its own suite" if rc is not None else "BASELINE NOT JUDGED: " + out)
        print("\n".join(out.splitlines()[-8:]))
        return 2
    print(f"baseline: the unmutated installer passes ({len(re.findall(r'^ok ', out, re.M))} cells)")
    for m in spec:
        if only and m["id"] not in only:
            continue
        n = text_of(m["file"]).count(m["old"])
        if n != 1:
            verdict = f"NOT APPLIED ({n} occurrences of the old text)"
        elif m["new"] == m["old"]:
            verdict = "VACUOUS (the mutant does not change the file)"
        else:
            with tempfile.TemporaryDirectory() as d:
                shutil.copytree(MAC, os.path.join(d, "mac"))
                target = os.path.join(d, "mac", m["file"])
                with open(target, "w", encoding="utf-8") as f:
                    f.write(text_of(m["file"]).replace(m["old"], m["new"]))
                syn = subprocess.run(["sh", "-n", target], capture_output=True, text=True, check=False)
                if syn.returncode != 0:
                    rc, failed, out = -1, set(), "BROKEN (the mutant is a shell syntax error, it proves nothing): " + (syn.stderr.strip().splitlines() or [""])[-1][:80]
                else:
                    rc, failed, out = run_suite(os.path.join(d, "mac", "install-mac.sh"), m["fails"])
            if rc == -1:
                verdict = out
            elif rc is None:
                verdict = "TIMED OUT (not judged: run it again)"
            elif rc == 0:
                verdict = "SURVIVED (the whole suite passed): the cells are vacuous for this fault"
            else:
                missing = [t for t in m["fails"] if t not in failed]
                verdict = "OK (" + ", ".join(sorted(failed)) + ")" if not missing else f"WRONG TEST: {missing} did not fail (failed: {sorted(failed)})"
        ok = verdict.startswith("OK")
        bad += 0 if ok else 1
        print(f"{m['id']:8} {verdict}  [{m['what']}]")
    print(f"install-mac-mutants: {len([m for m in spec if not only or m['id'] in only])} mutants, {bad} not caught or broken")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
