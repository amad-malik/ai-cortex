"""Mutation run for deploy/install.sh (A78; the same discipline as witness_mutants.py and s5_selftest.py).

A mutant is install.sh with ONE deliberate fault, in a COPY of deploy/ (install.sh finds its siblings by its own path).
The named IT tests must FAIL on it. A mutant whose `old` text is not in the script exactly once, that does not change
it, or that no test catches, FAILS the run. The unmutated script must pass the suite first.

    cv2-admit python3 -I -B scripts/install_mutants.py
"""
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
DEPLOY = os.path.join(os.path.dirname(HERE), "deploy")
TEST = os.path.join(HERE, "install-test.sh")
SPEC = os.path.join(HERE, "install-mutants.json")


# A mutant needs only the cells it names: when every cell in its `fails` is at or before IT-100 (or IT-110) the suite is told to STOP there (INSTALL_TEST_STOP_AFTER, honoured by
# install-test.sh at those two points and by nothing else), which halves the time of most mutants. The unmutated baseline and every other run use the WHOLE suite.
def stop_after(fails):
    nums = [int(m.group(1)) for f in fails for m in [re.match(r"IT-([0-9]+)", f)] if m]
    if not nums or len(nums) != len(fails):
        return None
    return "100" if max(nums) <= 100 else "110" if max(nums) <= 110 else None


def run_suite(install_path, stop=None, test=None):
    env = dict(os.environ)
    env.pop("INSTALL_SH", None)
    if install_path:   # None: the suite takes the deploy/ beside its own scripts/ (a SUITE mutant: the suite's own isolation is what is judged)
        env["INSTALL_SH"] = install_path
    env.pop("INSTALL_TEST_STOP_AFTER", None)
    if stop:
        env["INSTALL_TEST_STOP_AFTER"] = stop
    timeout = int(os.environ.get("INSTALL_MUTANT_TIMEOUT", "900"))   # a loaded machine may need several minutes; a timeout is a verdict, not a crash
    try:
        p = subprocess.run(["sh", test or TEST], capture_output=True, text=True, timeout=timeout, env=env, check=False)
    except subprocess.TimeoutExpired:
        return None, set(), "TIMED OUT after %d s" % timeout
    out = p.stdout + p.stderr
    failed = set(re.findall(r"^FAIL (IT-[0-9]+[a-z]*)", out, re.M))
    failed |= {re.sub(r"[a-z]+$", "", f) for f in failed}  # IT-54b also counts as IT-54: a spec may name the base or the exact id (hale N-2)
    return p.returncode, failed, out


def print_failed_cells(out):
    """A baseline that is not green NAMES its failing cell(s): every line that starts with the suite's own failure marker (install-test.sh's bad(): 'FAIL <name>', then ONE indented detail line) and its detail, in
    addition to the last lines above (the failing cell is usually EARLIER than the last 8 lines: in finn's smoke run one of eight concurrent baselines failed with '193 passed, 1 failed' and no receipt named the cell)."""
    lines = out.splitlines()
    cells = []
    for i, line in enumerate(lines):
        if line.startswith("FAIL "):
            cells.append(line)
            j = i + 1
            while j < len(lines) and lines[j].startswith("       "):
                cells.append(lines[j])
                j += 1
    if not cells:
        print("the baseline output holds no failed cell (no line starts with 'FAIL '): the suite stopped without naming one")
        return
    print(f"the baseline failed {sum(1 for c in cells if c.startswith('FAIL '))} cell(s):")
    print("\n".join(cells))


def main():
    spec = json.load(open(SPEC, encoding="utf-8"))
    originals = {}   # a mutant may name another file of deploy/ (default install.sh), for example db-roles.sh

    def original_of(name):
        if name not in originals:
            base = os.path.dirname(HERE) if name.startswith("scripts/") else DEPLOY
            originals[name] = open(os.path.join(base, name), encoding="utf-8").read()
        return originals[name]

    original = original_of("install.sh")
    # THE LIMIT, stated: these mutants judge install.sh's own logic against FAKE lanes; the real includes (deploy/install-fetch.sh and deploy/install-prereq.sh, merged from integration) are not part of it (their own
    # suites and mutants judge them, the joined run is L2), and a half-joined build fails closed, so the baseline and every mutant copy of deploy/ go without them
    with tempfile.TemporaryDirectory() as bd:
        shutil.copytree(DEPLOY, os.path.join(bd, "deploy"))
        for inc in ("install-fetch.sh", "install-prereq.sh"):
            if os.path.exists(os.path.join(bd, "deploy", inc)):
                os.remove(os.path.join(bd, "deploy", inc))
        rc, _, out = run_suite(os.path.join(bd, "deploy", "install.sh"))
    if rc is None:
        print("BASELINE NOT JUDGED: " + out)
        return 2
    if rc != 0:
        print("BASELINE NOT GREEN: the unmutated install.sh fails its own suite")
        print("\n".join(out.splitlines()[-8:]))
        print_failed_cells(out)
        return 2
    print(f"baseline: the unmutated install.sh passes ({len(re.findall(r'^ok ', out, re.M))} checks)")
    bad = 0
    layout_green = False   # set once the unmutated suite has passed in the layout used for suite mutants
    only = set(sys.argv[1:])   # optional: ids to run (development); the ci gate runs them all
    for m in spec:
        if only and m["id"] not in only:
            continue
        target = m.get("file", "install.sh")
        n = original_of(target).count(m["old"])
        if n != 1:
            verdict = f"NOT APPLIED ({n} occurrences of the old text)"
        elif m["new"] == m["old"]:
            verdict = "VACUOUS (the mutant does not change the script)"
        else:
            # A mutant of the SUITE is run from a copy of scripts/, and the suite refuses a compose provider whose path has a group- or other-writable directory (/tmp is one): that copy must NOT sit
            # under /tmp. It goes beside the lane's own scripts/ (the suite already makes its fake provider there), so every cell sees the layout of a normal run. (IM-370..387 judged "wrong test" for
            # nothing while the copy was in /tmp: every cell that reaches compose failed.)
            with tempfile.TemporaryDirectory(prefix=".install-mutant-", dir=os.path.dirname(HERE) if target.startswith("scripts/") else None) as d:
                copy = os.path.join(d, "deploy")
                shutil.copytree(DEPLOY, copy)
                suite_copy = None
                if not target.startswith("scripts/"):
                    # a mutant of install.sh: the copy goes without the real includes (see the baseline above). A mutant of the SUITE keeps them (the real includes are in its deploy/ copy and the suite's OWN copy helper
                    # must drop them: that isolation is what such a mutant judges, cox W1-99 (2a))
                    for inc in ("install-fetch.sh", "install-prereq.sh"):
                        if os.path.exists(os.path.join(copy, inc)):
                            os.remove(os.path.join(copy, inc))
                if target.startswith("scripts/"):
                    # a mutant of the SUITE (its test-only hooks): a copy of scripts/ with ONE fault runs against the unmutated installer, the whole suite (the hook cells are at its end)
                    # the copy IGNORES the transient directories of OTHER runs that sit beside the suite: ci-fanout deals the 29 suite ids to EVERY shard, so while this copy is made the other shards' install-test.sh create and remove their
                    # own scripts/.provider.XXXXXX (a fake provider with a symlink under it: a copy that meets a half-built or vanishing one failed with 'Too many levels of symbolic links', hue's smoke finding 4), and a sibling driver
                    # may have its own .install-mutant-* layout. Those are not part of the suite; the suite's own copy helper makes its own provider in its own copy. (cox 34176243)
                    shutil.copytree(HERE, os.path.join(d, "scripts"), ignore=shutil.ignore_patterns(".provider.*", ".install-mutant-*"))
                    # the cells of the suite also READ the repository's crates/ through $here/.. (IT-147: the migrations; IT-148: the upgrade engine's fixture): the layout gives them the lane's OWN crates/ as a LINK (read-only use;
                    # the temporary directory is removed WITHOUT following the link, so the lane's crates/ is never touched). A lane with no crates/ beside its scripts/ gets nothing, and those cells fail closed and NAMED: the
                    # layout never invents the files (cox a17af225: every suite mutant stopped at 'BASELINE NOT GREEN' since a136d0d added IT-147).
                    crates_src = os.path.join(os.path.dirname(HERE), "crates")
                    if os.path.isdir(crates_src):
                        os.symlink(crates_src, os.path.join(d, "crates"))
                    suite_copy = os.path.join(d, target)
                    if not layout_green:
                        # once per run: the UNMUTATED suite must pass in this very layout, or every suite mutant is judged against a broken layout
                        with open(suite_copy, "w", encoding="utf-8") as f:
                            f.write(original_of(target))
                        lrc, _, lout = run_suite(None, None, suite_copy)   # no INSTALL_SH: the suite takes the deploy/ copy beside its scripts/ (with the real includes) and isolates itself
                        if lrc != 0:
                            print("BASELINE NOT GREEN: the unmutated suite fails in the layout used for suite mutants" + (" (timed out)" if lrc is None else ""))
                            print("\n".join(lout.splitlines()[-8:]))
                            print_failed_cells(lout)
                            return 2
                        layout_green = True
                    with open(suite_copy, "w", encoding="utf-8") as f:
                        f.write(original_of(target).replace(m["old"], m["new"]))
                else:
                    with open(os.path.join(copy, target), "w", encoding="utf-8") as f:
                        f.write(original_of(target).replace(m["old"], m["new"]))
                # a mutant that is a SHELL SYNTAX ERROR breaks every cell and would count as "caught" for nothing (IM-286, IM-335, IM-337 were such): it proves nothing and is refused
                syn = subprocess.run(["sh", "-n", suite_copy or os.path.join(copy, target)], capture_output=True, text=True, check=False) if target.endswith(".sh") else None
                if syn is not None and syn.returncode != 0:
                    rc, failed, out = -1, set(), "BROKEN (the mutant is a shell syntax error, it proves nothing): " + (syn.stderr.strip().splitlines() or [""])[-1][:80]
                else:
                    rc, failed, out = run_suite(None if suite_copy else os.path.join(copy, "install.sh"), None if suite_copy else stop_after(m["fails"]), suite_copy)
            if rc == -1:
                verdict = out
            elif rc is None:
                verdict = "TIMED OUT (not judged: run it again): " + out
            elif rc == 0:
                verdict = "SURVIVED (the whole suite passed): the tests are vacuous for this fault"
            else:
                missing = [t for t in m["fails"] if t not in failed]
                verdict = "OK (" + ", ".join(sorted(failed)) + ")" if not missing else f"WRONG TEST: {missing} did not fail (failed: {sorted(failed)})"
        ok = verdict.startswith("OK")
        bad += 0 if ok else 1
        print(f"{m['id']:6} {verdict}  [{m['what']}]")
    print(f"install-mutants: {len([m for m in spec if not only or m['id'] in only])} mutants, {bad} not caught or broken")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
