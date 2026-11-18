#!/usr/bin/python3 -I -B
"""S5-23 RUN mount policy: the cells and the mutants of the checker (nix, the pin for sam's S523-SAM01).

    python3 -I -B scripts/s5_mount_mutants.py                       baseline of the mount cells, then every mutant: one fault planted in a COPY of scripts/s5_check.py,
                                                                    and each mutant must make at least one of ITS NAMED cells fail (a survivor fails the run)
    python3 -I -B scripts/s5_mount_mutants.py --cells [--checker F] only the mount cells, against F (default: this tree's checker); every cell prints its own line: this is how
                                                                    the RED against the checker of integration 1548ce8 and the GREEN are shown, one line per id, none abridged

The mount cells are the entries of scripts/s5-check-fixtures/mutations.json that this pin added (negatives m361 to m401, positives p20 to p25) and the positive control p18; they are judged
EXACTLY as scripts/s5_selftest.py judges them (a negative: exit 1, `FAIL <rule>` and the expected words on a finding line; a positive: exit 0 and no FAIL). Run it through cv2-admit.
"""
import argparse, importlib.util, json, os, re, shutil, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
FIX = os.path.join(HERE, "s5-check-fixtures")
MUTANTS = os.path.join(HERE, "s5-mount-mutants.json")
CACHE = os.path.expanduser("~/.cache/s5-mount-tests")
if CACHE.startswith("/tmp"):
    sys.exit("refusing scratch under /tmp")
os.makedirs(CACHE, exist_ok=True)
tempfile.tempdir = CACHE


def mount_cells(spec):
    neg = [m for m in spec["mutations"] if re.fullmatch(r"m\d+", m["id"]) and int(m["id"][1:]) >= 361]
    pos = [p for p in spec["good_passes"] if p["id"] == "p18" or (re.fullmatch(r"p\d+", p["id"]) and int(p["id"][1:]) >= 20)]
    return neg, pos


def stage(checker_path, workdir):
    """A scratch copy of the self-test, its fixtures and the checker under test; the self-test module loaded from there (so its CHECK is that checker)."""
    shutil.copy(os.path.join(HERE, "s5_selftest.py"), workdir)
    shutil.copytree(FIX, os.path.join(workdir, "s5-check-fixtures"))
    shutil.copy(checker_path, os.path.join(workdir, "s5_check.py"))
    sp = importlib.util.spec_from_file_location("s5_selftest_under_test", os.path.join(workdir, "s5_selftest.py"))
    st = importlib.util.module_from_spec(sp)
    sp.loader.exec_module(st)
    return st


def run_cell(st, cell, positive):
    with tempfile.TemporaryDirectory() as d:
        tree = os.path.join(d, "tree")
        shutil.copytree(st.GOOD, tree)
        st.place_manifest(tree)
        err = st.apply(cell, tree)
        if err:
            return False, err
        rc, out = st.run_checker(tree)
        if positive:
            return rc == 0 and "FAIL" not in out, out
        ok = rc == 1 and f"FAIL {cell['rule']}" in out
        if ok and cell.get("expect"):
            ok = any(f"FAIL {cell['rule']}" in line and cell["expect"] in line for line in out.splitlines())
        return ok, out


def run_cells(checker_path, cells, label):
    """cells: list of (cell, positive). Prints one line per cell. Returns (passed, failed, {id: ok})."""
    results = {}
    with tempfile.TemporaryDirectory() as w:
        st = stage(checker_path, w)
        for cell, positive in cells:
            ok, out = run_cell(st, cell, positive)
            results[cell["id"]] = ok
            print(("ok   " if ok else "FAIL ") + f"{cell['id']} {cell['note']}")
            if not ok:
                for line in out.strip().splitlines()[:3]:
                    print("       " + line[:200])
    return sum(results.values()), len(results) - sum(results.values()), results


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cells", action="store_true")
    ap.add_argument("--checker", default=os.path.join(HERE, "s5_check.py"))
    a = ap.parse_args()
    spec = json.load(open(os.path.join(FIX, "mutations.json"), encoding="utf-8"))
    neg, pos = mount_cells(spec)
    cells = [(c, False) for c in neg] + [(c, True) for c in pos]
    by_id = {c["id"]: (c, p) for c, p in cells}
    if not a.cells:
        mutants = json.load(open(MUTANTS, encoding="utf-8"))
        src = open(a.checker, encoding="utf-8").read()
        bad = []
        for m in mutants:
            if src.count(m["old"]) != 1 or m["old"] == m["new"]:
                bad.append(m["id"])
            else:
                try:
                    compile(src.replace(m["old"], m["new"], 1), m["id"], "exec")
                except SyntaxError:
                    bad.append(m["id"])
            missing = [c for c in m["cells"] if c not in by_id]
            if missing:
                bad.append(m["id"] + " names unknown cells " + ",".join(missing))
        if bad:
            print("BROKEN mutants (anchor not exactly once, no change, no compile, or an unknown cell): " + ", ".join(bad))
            return 1
        print(f"anchors: {len(mutants)} mutants, each applies EXACTLY ONCE, changes the text and compiles, and names only known cells (checked before any cell runs)")

    sha = __import__("hashlib").sha256(open(a.checker, "rb").read()).hexdigest()
    print(f"s5 mount cells against {a.checker} (sha256 {sha}): {len(neg)} negatives, {len(pos)} positives (one line each, none abridged)")
    passed, failed, results = run_cells(a.checker, cells, "baseline")
    print(f"s5-mount-cells: {passed} passed, {failed} failed")
    if a.cells:
        return 0 if failed == 0 else 1
    if failed:
        print("the baseline does not pass: no mutant can be judged")
        return 1
    survivors = 0
    for m in mutants:
        with tempfile.TemporaryDirectory() as w:
            mutated = os.path.join(w, "mutated_check.py")
            with open(mutated, "w", encoding="utf-8") as f:
                f.write(src.replace(m["old"], m["new"], 1))
            sub = os.path.join(w, "stage")
            os.makedirs(sub)
            st = stage(mutated, sub)
            failed_cells = []
            for cid in m["cells"]:
                cell, positive = by_id[cid]
                ok, _out = run_cell(st, cell, positive)
                if not ok:
                    failed_cells.append(cid)
        if failed_cells:
            print(f"killed {m['id']} ({m['what']}) by {', '.join(failed_cells)} of {', '.join(m['cells'])}")
        else:
            survivors += 1
            print(f"SURVIVOR {m['id']} ({m['what']}): none of {', '.join(m['cells'])} failed")
    print(f"s5-mount-mutants: {len(mutants)} mutants, survivors: {survivors}")
    return 0 if survivors == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
