#!/usr/bin/env python3 -I
"""Mutation run for cortex-upgrade (test definitions r4 section 4).

A mutant is the source with ONE deliberate fault. The named tests must FAIL on it; a mutant that
changes nothing, does not compile, or that no test catches fails this run (the same vacuity guard
as the S5 self-test). The files are patched in place and always restored, then checked by sha256.

    python3 -I mutants/run.py <group.json> [--only ID]     (from the crate directory; each mutant names its test targets, default group_a)
"""
import hashlib, json, os, re, subprocess, sys, time

ADMIT = "/path/to/workspace/spikes/cortex-v2-s0/admission/cv2-admit"
CRATE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
def test_cmd(targets):
    cmd = ["cargo", "test", "--locked", "-p", "cortex-upgrade", "--no-fail-fast"]
    for t in targets:
        cmd += ["--lib"] if t == "__lib__" else ["--test", t]
    return cmd


def sha(b):
    return hashlib.sha256(b).hexdigest()


def run_tests(targets):
    env = dict(os.environ, CV2_ADMIT_LOG="1")
    try:
        p = subprocess.run([ADMIT] + test_cmd(targets), cwd=CRATE, env=env, capture_output=True, text=True, timeout=240)
    except subprocess.TimeoutExpired:
        return {"<the run timed out: a test hung>"}, set(), False, -1
    out = p.stdout + p.stderr
    failed = set(re.findall(r"^test (\S+) \.\.\. FAILED$", out, re.M))
    passed = set(re.findall(r"^test (\S+) \.\.\. ok$", out, re.M))
    compile_error = "could not compile" in out or re.search(r"^error(\[E\d+\])?:", out, re.M) is not None and not (failed or passed)
    return failed, passed, bool(compile_error), p.returncode


def main():
    args = sys.argv[1:]
    group = args[0]
    only = args[args.index("--only") + 1] if "--only" in args else None
    mutants = json.load(open(group))
    print(f"mutation run {group} start {time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())}")
    bad = 0
    baselined = set()
    for m in mutants:
        if only and m["id"] != only:
            continue
        targets = tuple(m.get("tests", ["group_a"]))
        if targets not in baselined:
            failed, passed, ce, rc = run_tests(targets)
            if ce or failed or not passed:
                print(f"BASELINE NOT GREEN for {targets}: failed={sorted(failed)} compile_error={ce} rc={rc}")
                return 2
            print(f"baseline {targets}: {len(passed)} tests pass")
            baselined.add(targets)
        path = os.path.join(CRATE, m["file"])
        original = open(path, "rb").read()
        text = original.decode()
        verdict = None
        try:
            edits = m.get("edits") or [[m["old"], m["new"]]]
            counts = [text.count(o) for o, _ in edits]
            patched = text
            for o, nw in edits:
                patched = patched.replace(o, nw)
            if any(c != 1 for c in counts):
                verdict = f"NOT APPLIED (occurrences of the old text: {counts})"
            elif patched == text:
                verdict = "VACUOUS (the mutant does not change the source)"
            else:
                open(path, "w").write(patched)
                failed, passed, ce, rc = run_tests(targets)
                if ce:
                    verdict = "DOES NOT COMPILE"
                elif not failed:
                    verdict = "SURVIVED (no test failed): the tests are vacuous for this fault"
                else:
                    missing = [t for t in m["fails"] if t not in failed]
                    verdict = "OK" if not missing else f"WRONG TEST: {missing} did not fail (failed: {sorted(failed)})"
                if verdict == "OK":
                    verdict = f"OK ({len(failed)} failed, incl. {', '.join(m['fails'])})"
        finally:
            open(path, "wb").write(original)
            assert sha(open(path, "rb").read()) == sha(original), "source not restored"
        ok = verdict.startswith("OK")
        bad += 0 if ok else 1
        print(f"{m['id']:6} {('design ' + m['design']) if m.get('design') else 'extra':10} {verdict}  [{m['what']}]")
    print(f"mutants: {len([m for m in mutants if not only or m['id'] == only])}, not caught or broken: {bad}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
