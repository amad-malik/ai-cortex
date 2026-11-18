#!/usr/bin/python3 -I
"""Cells TG-nn of scripts/test-gate/test_gate.py (the ci.sh test gate, cox W1-103 (2) and W1-106 (A1-A3)). Run as:  cv2-admit sh scripts/test-gate/test-gate-test.sh

Every cell runs the REAL gate against FAKES (scripts/test-gate/fakes: cargo, the admission program, supervise.sh): nothing is built, no database, no podman. The fakes print canned
cargo output from a sandbox directory and record every call, so a cell can say exactly which commands the gate ran, in which order, and how often. TEST_GATE_PY names another
gate file (the mutants runner uses it). Scratch lives under TMPDIR (required, never /tmp). On any failure the last line is 'TEST GATE CELLS FAILED' (the mutants runner needs it)."""
import hashlib
import importlib.util
import json
import os
import re
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

HERE = os.path.dirname(os.path.abspath(__file__))
GATE = os.environ.get("TEST_GATE_PY", os.path.join(HERE, "test_gate.py"))
FAKES = os.path.join(HERE, "fakes")
TMP = os.environ.get("TMPDIR", "")
if not TMP or TMP.startswith("/tmp"):
    sys.exit("test-gate cells: set TMPDIR to private scratch under /home (never /tmp)")
NEEDS_DB = "CV2_PG_PORT is unset: run the tests under crates/cortex-testkit/supervise.sh (L-9): NotPresent"
PARTITION_SHA256 = "614a6d8b5c29526ecb0ec8f496c8363bd8197c71f71f8942ef23139a9a938375"


# ---- a synthetic workspace: what cargo would print for it -----------------------------------------------------------------------------------------------------------
def target(pkg, name, tests, kind="test", digest="1111"):
    """tests: list of (test name, status, panic message or ''), status ok | FAILED | ignored."""
    return {"pkg": pkg, "name": name, "kind": kind, "hash": digest, "tests": tests}


def default_workspace():
    return [
        target("alpha", "alpha", [("a1", "ok", ""), ("a2", "ok", "")], kind="lib", digest="a000"),
        target("alpha", "alpha_it", [("it1", "ok", ""), ("it2", "ok", "")], digest="a001"),
        target("beta", "beta_db", [("b1", "FAILED", NEEDS_DB), ("b2", "FAILED", NEEDS_DB), ("b3", "ok", "")], digest="b001"),
        target("gamma", "gamma_db", [("g1", "FAILED", NEEDS_DB)], digest="c001"),
    ]


def build_json(ws):
    out = ['{"reason":"compiler-message","message":"noise"}']
    for t in ws:
        out.append(json.dumps({"reason": "compiler-artifact", "package_id": f"path+file:///w/crates/{t['pkg']}#0.1.0", "profile": {"test": True},
                               "executable": f"/w/target/debug/deps/{exe(t)}-{t['hash']}", "target": {"kind": [t["kind"]], "name": t["name"]}}))
    out.append('{"reason":"build-finished","success":true}')
    return "\n".join(out) + "\n"


def exe(t):
    return t["name"].replace("-", "_")


def running_line(t):
    where = "unittests src/lib.rs" if t["kind"] == "lib" else "unittests src/main.rs" if t["kind"] == "bin" else f"tests/{t['name']}.rs"
    return f"     Running {where} (target/debug/deps/{exe(t)}-{t['hash']})"


def list_txt(ws):
    out = []
    for t in ws:
        out += [running_line(t)] + [f"{n}: test" for n, _, _ in t["tests"]] + ["", f"{len(t['tests'])} tests, 0 benchmarks", ""]
    return "\n".join(out) + "\n"


def run_txt(ws, ansi=False, children=()):
    ok = (lambda s: f"\x1b[32m{s}\x1b[0m") if ansi else (lambda s: s)
    out = []
    for t in ws:
        out += [running_line(t), "", f"running {len(t['tests'])} tests"]
        for n, st, _ in t["tests"]:
            out.append(f"test {n} ... {ok(st) if st == 'ok' else st}")
        for c in (children if t["name"] == "alpha_it" else ()):
            out.append(c)
        failed = [(n, m) for n, st, m in t["tests"] if st == "FAILED"]
        if failed:
            out += ["", "failures:", ""]
            for n, m in failed:
                out += [f"---- {n} stdout ----", "", f"thread '{n}' (12345) panicked at crates/x/src/lib.rs:1:1:", m, "note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace", ""]
            out += ["", "failures:"] + [f"    {n}" for n, _ in failed]
        n_ok = sum(1 for _, st, _ in t["tests"] if st == "ok")
        out += ["", f"test result: {'FAILED' if failed else 'ok'}. {n_ok} passed; {len(failed)} failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s", ""]
    return "\n".join(out) + "\n"


def db_run_txt(ws, pkg, status="ok", only=None, digest=None):
    """What a supervised run of one package prints: every test of the package, with the given status for the DB tests."""
    out = []
    for t in ws:
        if t["pkg"] != pkg:
            continue
        rows = []
        for n, st, m in t["tests"]:
            if only is not None and n not in only:
                continue
            rows.append((n, status if m == NEEDS_DB else st, m))
        u = dict(t, tests=rows)
        if digest:
            u["hash"] = digest
        out += [running_line(u), "", f"running {len(rows)} tests"] + [f"test {n} ... {st}" for n, st, _ in rows]
        bad = [n for n, st, _ in rows if st == "FAILED"]
        if bad:
            out += ["", "failures:", ""]
            for n in bad:
                out += [f"---- {n} stdout ----", "", f"thread '{n}' panicked at x:1:1:", "assertion failed", ""]
        out += ["", f"test result: {'FAILED' if bad else 'ok'}. x passed; {len(bad)} failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s", ""]
    return "\n".join(out) + "\n"


class Sandbox:
    def __init__(self, ws=None, ansi=False, children=(), ignored="", podman=True):
        self.dir = tempfile.mkdtemp(prefix="test-gate-cell-", dir=TMP)
        self.ws = ws if ws is not None else default_workspace()
        self.put("build.json", build_json(self.ws))
        self.put("list.txt", list_txt(self.ws))
        self.put("run.txt", run_txt(self.ws, ansi=ansi, children=children))
        for pkg in sorted({t["pkg"] for t in self.ws}):
            self.put(f"db-{pkg}.txt", db_run_txt(self.ws, pkg))
        self.put("ignored.tsv", ignored)
        self.work = os.path.join(self.dir, "work")
        self.loadavg = os.path.join(self.dir, "loadavg")
        self.put("loadavg", "0.50 0.50 0.50 1/100 123\n")
        self.podman = os.path.join(self.dir, "podman")
        if podman:
            self.put("podman", "#!/bin/sh\n")
            os.chmod(self.podman, 0o755)
        self.env = {k: v for k, v in os.environ.items() if not k.startswith(("CV2_", "TEST_GATE", "CI_"))}
        self.env.update({"TG_DIR": self.dir, "TEST_GATE_TEST": "1", "TEST_GATE_CARGO": os.path.join(FAKES, "cargo"), "TEST_GATE_ADMIT": os.path.join(FAKES, "admit"),
                         "TEST_GATE_SUPERVISE": os.path.join(FAKES, "supervise"), "TEST_GATE_PODMAN": self.podman, "TEST_GATE_WORK": self.work,
                         "TEST_GATE_IGNORED": os.path.join(self.dir, "ignored.tsv"), "TEST_GATE_RETRY_PAUSE": "0", "TEST_GATE_LOADAVG": self.loadavg,
                         "TEST_GATE_GUARD_PAUSE": "0", "CI_DB_GUARD": "0"})

    def put(self, name, text):
        with open(os.path.join(self.dir, name), "w") as f:
            f.write(text)

    def run(self, args=(), **env):
        e = dict(self.env)
        for k, v in env.items():
            if v is None:
                e.pop(k, None)
            else:
                e[k] = v
        return subprocess.run([sys.executable, "-I", "-B", GATE, *args], cwd=self.dir, env=e, capture_output=True, text=True, timeout=120, check=False)

    def calls(self):
        p = os.path.join(self.dir, "calls.log")
        return open(p).read().splitlines() if os.path.exists(p) else []

    def order(self):
        p = os.path.join(self.dir, "order.log")
        return open(p).read().splitlines() if os.path.exists(p) else []

    def cleanup(self):
        shutil.rmtree(self.dir, ignore_errors=True)


def last(result):
    lines = [l for l in (result.stdout + result.stderr).splitlines() if l.strip()]
    return lines[-1] if lines else ""


class Cell(unittest.TestCase):
    def sandbox(self, **kw):
        sb = Sandbox(**kw)
        self.addCleanup(sb.cleanup)
        return sb

    def assertGate(self, result, rc, final=None, has=(), hasnt=()):
        text = result.stdout + result.stderr
        self.assertEqual(result.returncode, rc, text[-2500:])
        if final is not None:
            self.assertTrue(last(result).startswith(final), (last(result), text[-1500:]))
        for h in has:
            self.assertIn(h, text, text[-2500:])
        for h in hasnt:
            self.assertNotIn(h, text, text[-2500:])


class Gate(Cell):
    def test_tg01_a_workspace_with_no_database_tests_passes_in_one_driver_and_starts_no_database_run(self):
        ws = [target("alpha", "alpha", [("a1", "ok", ""), ("a2", "ok", "")], kind="lib", digest="a000"), target("gamma", "gamma_it", [("g1", "ok", "")], digest="c001")]
        sb = self.sandbox(ws=ws)
        r = sb.run()
        self.assertGate(r, 0, "test: PASS", has=["needs_db 0"])
        calls = sb.calls()
        self.assertEqual([c for c in calls if c.startswith("supervise")], [])
        self.assertEqual(sum(1 for c in calls if c.startswith("cargo [test] [--workspace] [--no-run] [--message-format=json]")), 1)
        self.assertEqual(sum(1 for c in calls if c.startswith("cargo [test] [--workspace] [--] [--list]")), 1)
        self.assertEqual(sum(1 for c in calls if c.startswith("cargo [test] [--workspace] [--no-fail-fast]")), 1)

    def test_tg02_database_tests_are_classed_by_their_panic_and_each_package_runs_once_under_supervise_in_order(self):
        sb = self.sandbox()
        r = sb.run()
        self.assertGate(r, 0, "test: PASS", has=["needs_db 3", "beta", "gamma"])
        sup = [c for c in sb.calls() if c.startswith("supervise")]
        cargo = os.path.join(FAKES, "cargo")
        self.assertEqual(sup, [f"supervise [{cargo}] [test] [-p] [beta] [--no-fail-fast]", f"supervise [{cargo}] [test] [-p] [gamma] [--no-fail-fast]"])
        adm = [c for c in sb.calls() if c.startswith("admit") and "supervise" in c]
        self.assertEqual(len(adm), 2)
        for c, pkg in zip(adm, ("beta", "gamma")):
            self.assertIn(f"[timeout] [--kill-after=120] [780] [{os.path.join(FAKES, 'supervise')}] [{cargo}] [test] [-p] [{pkg}] [--no-fail-fast]", c)
            self.assertTrue(c.endswith("CV2_DB=1"), c)
        # the needs-db list is kept for the readers (A3: compare with the sealed derived list)
        rows = open(os.path.join(sb.work, "needs_db.tsv")).read().splitlines()
        self.assertEqual(rows, ["beta\tbeta_db\tb1", "beta\tbeta_db\tb2", "gamma\tgamma_db\tg1"])

    def test_tg03_a_failure_that_is_not_the_missing_database_panic_fails_the_gate_and_no_database_run_starts(self):
        ws = default_workspace()
        ws[1]["tests"][0] = ("it1", "FAILED", "assertion `left == right` failed")
        sb = self.sandbox(ws=ws)
        r = sb.run()
        self.assertGate(r, 1, "test: FAIL (1 problem)", has=["ran and FAILED", "alpha :: alpha_it :: it1"], hasnt=["explained by"])   # a REAL failure is never 'explained' as the database path
        self.assertEqual(last(r), "test: FAIL (1 problem)")
        self.assertEqual([c for c in sb.calls() if c.startswith("supervise")], [])

    def test_tg04_a_panic_that_only_contains_the_database_text_is_still_a_failure(self):
        ws = default_workspace()
        for message in ("boom: " + NEEDS_DB, NEEDS_DB.replace(": NotPresent", "")):
            ws = default_workspace()
            ws[2]["tests"][0] = ("b1", "FAILED", message)
            sb = self.sandbox(ws=ws)
            # the supervised run is GOOD (b1 ok there): the only thing wrong is that b1's panic is not the missing-database line, so the gate must refuse it on that alone
            sb.put("db-beta.txt", db_run_txt(default_workspace(), "beta"))
            self.assertGate(sb.run(), 1, "test: FAIL (1 problem)", has=["ran and FAILED: beta :: beta_db :: b1"])

    def test_tg05_a_database_test_that_fails_in_its_supervised_run_fails_the_gate(self):
        sb = self.sandbox()
        sb.put("db-beta.txt", db_run_txt(sb.ws, "beta", only=["b1", "b3"]).replace("test b1 ... ok", "test b1 ... FAILED"))
        self.assertGate(sb.run(), 1, "test: FAIL", has=["needs a database but is not ok in its supervised run", "beta :: beta_db :: b1"])

    def test_tg06_a_database_test_missing_from_its_supervised_run_fails_the_gate(self):
        sb = self.sandbox()
        sb.put("db-beta.txt", db_run_txt(sb.ws, "beta", only=["b1", "b3"]))
        self.assertGate(sb.run(), 1, "test: FAIL", has=["needs a database but is not ok in its supervised run", "beta :: beta_db :: b2"])

    def test_tg07_any_failure_in_a_database_run_fails_even_for_a_test_that_is_not_a_database_test(self):
        ws = default_workspace()
        sb = self.sandbox(ws=ws)
        sb.put("db-beta.txt", db_run_txt(ws, "beta").replace("test b3 ... ok", "test b3 ... FAILED"))
        self.assertGate(sb.run(), 1, "test: FAIL", has=["ran and FAILED in its supervised run", "beta :: beta_db :: b3"])

    def test_tg08_a_database_run_that_still_lacks_the_database_is_a_failure_not_a_skip(self):
        ws = default_workspace()
        sb = self.sandbox(ws=ws)
        sb.put("db-gamma.txt", db_run_txt(ws, "gamma", status="FAILED"))
        self.assertGate(sb.run(), 1, "test: FAIL", has=["gamma :: gamma_db :: g1"])

    def test_tg09_a_driver_that_exits_non_zero_fails_even_when_every_line_says_ok(self):
        sb = self.sandbox()
        sb.put("db-beta.rc", "124")
        self.assertGate(sb.run(), 1, "test: FAIL", has=["driver for beta exited 124"])
        self.assertEqual([c for c in sb.calls() if c.startswith("supervise") and "gamma" in c], [], "the gate stops at the first failing package")
        sb = self.sandbox()
        sb.put("db-gamma.rc", "72")
        self.assertGate(sb.run(), 1, "test: FAIL", has=["driver for gamma exited 72"])

    def test_tg10_a_listed_test_with_no_result_anywhere_is_neither_run_nor_listed_and_fails(self):
        sb = self.sandbox()
        extra = default_workspace()
        extra[1]["tests"].append(("it3", "ok", ""))
        sb.put("list.txt", list_txt(extra))
        self.assertGate(sb.run(), 1, "test: FAIL (1 problem)", has=["listed but neither run nor declared", "alpha :: alpha_it :: it3"])

    def test_tg11_a_test_that_ran_but_was_never_listed_fails(self):
        sb = self.sandbox()
        fewer = default_workspace()
        fewer[0]["tests"].pop()
        sb.put("list.txt", list_txt(fewer))
        self.assertGate(sb.run(), 1, "test: FAIL (1 problem)", has=["ran but not in the inventory", "alpha :: alpha :: a2"])

    def test_tg12_an_ignored_test_must_be_declared_with_a_reason_in_the_ignored_list(self):
        ws = default_workspace()
        ws[1]["tests"][1] = ("it2", "ignored", "")
        sb = self.sandbox(ws=ws)
        self.assertGate(sb.run(), 1, "test: FAIL (1 problem)", has=["ignored but not declared", "alpha :: alpha_it :: it2"])
        sb = self.sandbox(ws=ws, ignored="alpha\talpha_it\tit2\trun by its parent test with --ignored\n")
        self.assertGate(sb.run(), 0, "test: PASS", has=["declared #[ignore]: 1"])

    def test_tg13_a_declared_ignored_test_that_no_longer_exists_fails(self):
        sb = self.sandbox(ignored="alpha\talpha_it\tgone\tstale entry\n")
        self.assertGate(sb.run(), 1, "test: FAIL (1 problem)", has=["declared but not in the inventory", "alpha :: alpha_it :: gone"])

    def test_tg14_trybuild_fixture_lines_are_children_not_ghosts_and_a_child_that_is_not_ok_fails(self):
        ok_child = "test tests/ui/fixture.rs [should pass] ... ok"
        sb = self.sandbox(children=[ok_child])
        self.assertGate(sb.run(), 0, "test: PASS")
        sb = self.sandbox(children=["test tests/ui/fixture.rs [should fail to compile] ... FAILED"])
        self.assertGate(sb.run(), 1, "test: FAIL (1 problem)", has=["harness child not ok"])

    def test_tg15_colour_codes_in_the_cargo_output_change_nothing(self):
        sb = self.sandbox(ansi=True)
        self.assertGate(sb.run(), 0, "test: PASS", has=["needs_db 3"])

    def test_tg16_cargo_is_asked_for_plain_text(self):
        sb = self.sandbox()
        sb.run()
        runs = [c for c in sb.calls() if c.startswith("cargo")]
        self.assertTrue(runs and all("TERM_COLOR=never" in c for c in runs), runs)

    def test_tg17_a_failed_build_fails_the_gate_with_the_reason(self):
        sb = self.sandbox()
        sb.put("build.rc", "101")
        self.assertGate(sb.run(), 1, "test: FAIL", has=["build failed"])

    def test_tg18_without_podman_the_database_drivers_are_not_run_and_the_gate_says_so_in_its_last_line(self):
        sb = self.sandbox(podman=False)
        r = sb.run()
        self.assertGate(r, 77, "NOT_RUN: ")
        self.assertRegex(last(r), r"^NOT_RUN: no podman")
        self.assertEqual([c for c in sb.calls() if c.startswith("supervise")], [])
        self.assertIn("needs_db 3", r.stdout)

    def test_tg19_without_podman_a_workspace_with_no_database_tests_still_passes(self):
        ws = [target("alpha", "alpha", [("a1", "ok", "")], kind="lib", digest="a000")]
        sb = self.sandbox(ws=ws, podman=False)
        self.assertGate(sb.run(), 0, "test: PASS")

    def test_tg20_nested_in_an_admission_the_database_drivers_refuse_with_the_exact_reason(self):
        sb = self.sandbox()
        r = sb.run(CV2_ADMITTED="1")
        self.assertGate(r, 77)
        self.assertEqual(last(r), "NOT_RUN: nested in an admission: the database reservation cannot be taken")
        self.assertEqual([c for c in sb.calls() if c.startswith("supervise")], [])

    def test_tg21_a_real_failure_in_the_free_driver_wins_over_not_run(self):
        ws = default_workspace()
        ws[1]["tests"][0] = ("it1", "FAILED", "boom")
        sb = self.sandbox(ws=ws, podman=False)
        self.assertGate(sb.run(), 1, "test: FAIL")
        sb = self.sandbox(ws=ws)
        self.assertGate(sb.run(CV2_ADMITTED="1"), 1, "test: FAIL")

    def test_tg22_a_refused_admission_is_retried_and_then_the_run_goes_ahead(self):
        for rc in ("3", "75"):
            sb = self.sandbox()
            sb.put("admit-refuse-free", "4")
            sb.put("admit-refuse-db", "5")
            sb.env["TG_REFUSE_RC"] = rc
            r = sb.run()
            self.assertGate(r, 0, "test: PASS")
            self.assertEqual(sum(1 for c in sb.calls() if c.startswith("admit") and "--phase" in c), 5, sb.calls())
            self.assertEqual(sum(1 for c in sb.calls() if c.startswith("admit") and "supervise" in c), 5 + 1 + 1, sb.calls())   # the first package takes the five refusals, then both run

    def test_tg23_twelve_refusals_in_a_row_fail_the_gate_with_the_reason_and_never_skip(self):
        sb = self.sandbox()
        sb.put("admit-refuse-db", "99")
        r = sb.run()
        self.assertGate(r, 1, "test: FAIL", has=["admission refused 12 times for beta"])
        self.assertEqual(sum(1 for c in sb.calls() if c.startswith("admit") and "supervise" in c), 12, sb.calls())
        sb = self.sandbox()
        sb.put("admit-refuse-free", "99")
        self.assertGate(sb.run(), 1, "test: FAIL", has=["admission refused 12 times for the free driver"])

    def test_tg24_the_free_driver_runs_inside_a_bound_and_a_hung_one_is_killed_with_the_reason(self):
        sb = self.sandbox()
        sb.run()
        free = [c for c in sb.calls() if c.startswith("admit") and "--phase" in c]
        self.assertEqual(len(free), 1, sb.calls())
        self.assertIn("[timeout] [--kill-after=60] [2700] ", free[0])
        sb = self.sandbox()
        sb.put("hang-free", "")
        t0 = time.time()
        r = sb.run(TEST_GATE_FREE_BOUND="2")
        self.assertGate(r, 1, "test: FAIL", has=["free driver exceeded its bound of 2 s"])
        self.assertLess(time.time() - t0, 25)

    def test_tg25_a_hung_database_driver_is_killed_with_the_reason(self):
        sb = self.sandbox()
        sb.put("hang-db-beta", "")
        r = sb.run(TEST_GATE_DB_BOUND="2")
        self.assertGate(r, 1, "test: FAIL", has=["driver for beta exited 124"])

    def test_tg26_database_packages_run_one_at_a_time_in_sorted_order(self):
        ws = default_workspace() + [target("delta", "delta_db", [("d1", "FAILED", NEEDS_DB)], digest="d001")]
        sb = self.sandbox(ws=ws)
        self.assertGate(sb.run(), 0, "test: PASS")
        self.assertEqual(sb.order(), ["db-start beta", "db-end beta", "db-start delta", "db-end delta", "db-start gamma", "db-end gamma"])

    def test_tg27_a_database_run_built_with_other_flags_is_matched_by_target_and_test_name(self):
        ws = default_workspace()
        sb = self.sandbox(ws=ws)
        sb.put("db-beta.txt", db_run_txt(ws, "beta", digest="ffff"))
        self.assertGate(sb.run(), 0, "test: PASS")

    def test_tg28_the_guard_waits_for_a_quiet_machine_and_gives_up_with_the_reason(self):
        sb = self.sandbox()
        sb.put("loadavg", "12.00 11.00 10.00 1/100 123\n")
        r = sb.run(CI_DB_GUARD="1", TEST_GATE_GUARD_MAX="3", TEST_GATE_HEALTH_URL="none")
        self.assertGate(r, 1, "test: FAIL", has=["guard: the machine stayed busy after 3 pauses"])
        self.assertEqual([c for c in sb.calls() if c.startswith("supervise")], [])
        sb = self.sandbox()
        r = sb.run(CI_DB_GUARD="0")
        self.assertGate(r, 0, "test: PASS")

    def test_tg29_a_slow_health_answer_pauses_the_guard_too(self):
        class Slow(BaseHTTPRequestHandler):
            def do_GET(self):
                time.sleep(0.3)
                self.send_response(401)
                self.end_headers()
            def log_message(self, *a):
                pass
        srv = ThreadingHTTPServer(("127.0.0.1", 0), Slow)
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        self.addCleanup(srv.shutdown)
        sb = self.sandbox()
        r = sb.run(CI_DB_GUARD="1", TEST_GATE_GUARD_MAX="2", TEST_GATE_HEALTH_URL=f"http://127.0.0.1:{srv.server_address[1]}/health")
        self.assertGate(r, 1, "test: FAIL", has=["guard: the machine stayed busy after 2 pauses"])

    def test_tg30_the_result_lines_state_ran_passed_failed_and_the_time_per_driver(self):
        sb = self.sandbox()
        r = sb.run()
        out = r.stdout
        self.assertRegex(out, r"(?m)^test-free +driver ran 8 +ok 5 +failed 0 +needs_db 3 +ignored 0 +\(time \d+ s\)$")
        self.assertRegex(out, r"(?m)^test-db +package beta +needs_db 2 +ok 2 +failed 0 +\(time \d+ s\)$")
        self.assertRegex(out, r"(?m)^test-db +package gamma +needs_db 1 +ok 1 +failed 0 +\(time \d+ s\)$")

    def test_tg31_the_partition_tool_is_the_sealed_copy_byte_for_byte(self):
        self.assertEqual(hashlib.sha256(open(os.path.join(HERE, "ws_partition2.py"), "rb").read()).hexdigest(), PARTITION_SHA256)

    def test_tg32_the_environment_cannot_name_other_programs_without_the_test_hook(self):
        spec = importlib.util.spec_from_file_location("tg_under_test", GATE)
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        seams = {"TEST_GATE_CARGO": "/x/cargo", "TEST_GATE_ADMIT": "/x/admit", "TEST_GATE_SUPERVISE": "/x/sup", "TEST_GATE_PODMAN": "/x/podman", "TEST_GATE_IGNORED": "/x/ign",
                 "TEST_GATE_FREE_BOUND": "5", "TEST_GATE_DB_BOUND": "6", "TEST_GATE_RETRY_PAUSE": "0", "TEST_GATE_LOADAVG": "/x/load", "TEST_GATE_HEALTH_URL": "http://x"}
        plain = mod.config(dict(seams))
        for key in ("cargo", "admit", "supervise", "podman", "ignored", "loadavg", "health_url"):
            self.assertNotIn("/x/", str(plain[key]), key)
        self.assertEqual((plain["free_bound"], plain["db_bound"], plain["retry_pause"]), (2700, 780, 60))
        hooked = mod.config(dict(seams, TEST_GATE_TEST="1"))
        self.assertEqual((hooked["cargo"], hooked["admit"], hooked["supervise"]), ("/x/cargo", "/x/admit", "/x/sup"))
        self.assertEqual((hooked["free_bound"], hooked["db_bound"], hooked["retry_pause"]), (5, 6, 0))

    def test_tg33_the_free_driver_is_one_admission_for_build_list_and_run(self):
        sb = self.sandbox()
        sb.run()
        adm = [c for c in sb.calls() if c.startswith("admit")]
        free = [c for c in adm if "--phase" in c]
        self.assertEqual(len(free), 1)
        self.assertEqual(len(adm), 1 + 2)   # the free phase and the two database packages: nothing else is admitted

    def test_tg34_the_gate_fails_closed_when_cargo_prints_nothing_the_gate_can_read(self):
        sb = self.sandbox()
        sb.put("build.json", "")
        sb.put("list.txt", "")
        sb.put("run.txt", "")
        r = sb.run()
        self.assertGate(r, 1, "test: FAIL", has=["inventory is empty"])

    def test_tg35_without_an_executable_supervise_the_database_drivers_are_not_run(self):
        sb = self.sandbox()
        r = sb.run(TEST_GATE_SUPERVISE=os.path.join(sb.dir, "no-such-supervise.sh"))
        self.assertGate(r, 77)
        self.assertRegex(last(r), r"^NOT_RUN: no supervise\.sh")
        self.assertEqual([c for c in sb.calls() if c.startswith("supervise")], [])

    def test_tg36_a_hyphenated_binary_target_is_matched_by_its_executable_name(self):
        ws = [target("delta", "delta-bin", [("d1", "FAILED", NEEDS_DB), ("d2", "ok", "")], kind="bin", digest="d001")]
        sb = self.sandbox(ws=ws)
        self.assertGate(sb.run(), 0, "test: PASS", has=["needs_db 1"])

    def test_tg37_a_quiet_machine_with_an_api_that_answers_quickly_passes_the_guard(self):
        class Fast(BaseHTTPRequestHandler):
            def do_GET(self):
                self.send_response(401)
                self.end_headers()
            def log_message(self, *a):
                pass
        srv = ThreadingHTTPServer(("127.0.0.1", 0), Fast)
        threading.Thread(target=srv.serve_forever, daemon=True).start()
        self.addCleanup(srv.shutdown)
        sb = self.sandbox()
        r = sb.run(CI_DB_GUARD="1", TEST_GATE_GUARD_MAX="2", TEST_GATE_HEALTH_URL=f"http://127.0.0.1:{srv.server_address[1]}/health")
        self.assertGate(r, 0, "test: PASS")
        sb = self.sandbox()
        s2 = socket.socket()
        s2.bind(("127.0.0.1", 0))
        port = s2.getsockname()[1]
        s2.close()   # nothing listens there: the API does not answer
        r = sb.run(CI_DB_GUARD="1", TEST_GATE_GUARD_MAX="1", TEST_GATE_HEALTH_URL=f"http://127.0.0.1:{port}/health")
        self.assertGate(r, 1, "test: FAIL", has=["guard: the machine stayed busy after 1 pauses", "does not answer"])

    def test_tg38_a_database_test_that_cannot_be_attributed_to_a_package_fails(self):
        ws = default_workspace()
        sb = self.sandbox(ws=ws)
        ghost = target("?", "ghost", [("x1", "FAILED", NEEDS_DB)], digest="9999")   # a binary that is not in the build
        sb.put("list.txt", list_txt(ws + [ghost]))
        sb.put("run.txt", run_txt(ws + [ghost]))
        self.assertGate(sb.run(), 1, "test: FAIL", has=["cannot be attributed to a package"])
        sb = self.sandbox(ws=ws)
        doc = "   Doc-tests alpha\n\nrunning 1 test\ntest src/lib.rs - f (line 3) ... FAILED\n\nfailures:\n\n---- src/lib.rs - f (line 3) stdout ----\n\nthread 'main' panicked at src/lib.rs:3:1:\n" + NEEDS_DB + "\n\nfailures:\n    src/lib.rs - f (line 3)\n\ntest result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s\n"
        sb.put("list.txt", list_txt(ws) + "   Doc-tests alpha\nsrc/lib.rs - f (line 3): test\n\n1 test, 0 benchmarks\n")
        sb.put("run.txt", run_txt(ws) + doc)
        self.assertGate(sb.run(), 1, "test: FAIL", has=["cannot be attributed to a package", "(doc)"])

    def test_tg39_a_build_with_no_binaries_or_a_listing_with_no_tests_is_an_empty_inventory(self):
        sb = self.sandbox()
        sb.put("build.json", "")
        self.assertGate(sb.run(), 1, "test: FAIL", has=["the inventory is empty"])
        sb = self.sandbox()
        sb.put("list.txt", "")
        self.assertGate(sb.run(), 1, "test: FAIL", has=["the inventory is empty"])

    def test_tg40_the_running_lines_cargo_prints_to_stderr_are_kept_in_order_with_the_results(self):
        sb = self.sandbox()
        sb.run()
        run = open(os.path.join(sb.work, "run.txt")).read()
        self.assertRegex(run, r"(?m)^ +Running tests/alpha_it\.rs \(target/debug/deps/alpha_it-a001\)\n\nrunning 2 tests\ntest it1 \.\.\. ok$")
        listing = open(os.path.join(sb.work, "list.txt")).read()
        self.assertRegex(listing, r"(?m)^ +Running tests/alpha_it\.rs \(target/debug/deps/alpha_it-a001\)\nit1: test$")

    def test_tg41_the_fail_line_counts_its_problems(self):
        ws = default_workspace()
        ws[0]["tests"][0] = ("a1", "FAILED", "boom")
        ws[1]["tests"][0] = ("it1", "FAILED", "boom")
        sb = self.sandbox(ws=ws)
        r = sb.run()
        self.assertGate(r, 1)
        self.assertEqual(last(r), "test: FAIL (2 problems)")

    def test_tg42_a_free_run_that_exits_abnormally_fails_even_when_every_printed_row_is_ok(self):
        """sam CITG-SAM01: phase_free discarded the exit status of cargo test --workspace --no-fail-fast; a crashed or killed run whose printed rows are all ok passed."""
        ws = [target("alpha", "alpha", [("a1", "ok", ""), ("a2", "ok", "")], kind="lib", digest="a000")]
        for rc in ("137", "2", "kill"):
            with self.subTest(status=rc):
                sb = self.sandbox(ws=ws)
                sb.put("run.rc", rc)
                r = sb.run()
                self.assertGate(r, 1, "test: FAIL (1 problem)", has=["the free run exited"])
                self.assertEqual([c for c in sb.calls() if c.startswith("supervise")], [])
        sb = self.sandbox(ws=ws)   # the control: the same run with the honest status 0 passes
        self.assertGate(sb.run(), 0, "test: PASS")

    def test_tg43_status_101_needs_a_printed_failure_and_status_0_forbids_one(self):
        ws = [target("alpha", "alpha", [("a1", "ok", "")], kind="lib", digest="a000")]
        sb = self.sandbox(ws=ws)
        sb.put("run.rc", "101")   # cargo says tests failed, no row says so
        self.assertGate(sb.run(), 1, "test: FAIL (1 problem)", has=["cargo exited 101 but no test is printed FAILED"])
        sb = self.sandbox()
        sb.put("run.rc", "0")   # rows print FAILED (the database tests), cargo says all passed
        self.assertGate(sb.run(), 1, "test: FAIL (1 problem)", has=["cargo exited 0 but 3 tests are printed FAILED"])

    def test_tg44_the_intended_101_of_the_database_path_is_explained_in_the_output(self):
        sb = self.sandbox()
        r = sb.run()
        self.assertGate(r, 0, "test: PASS", has=["cargo exit status 101: explained by 3 FAILED tests, all of them needs_db"])
        ws = [target("alpha", "alpha", [("a1", "ok", ""), ("a2", "ok", "")], kind="lib", digest="a000")]
        sb = self.sandbox(ws=ws)
        self.assertGate(sb.run(), 0, "test: PASS", has=["cargo exit status 0"])


def main():
    suite = unittest.defaultTestLoader.loadTestsFromModule(sys.modules[__name__])
    result = unittest.TextTestRunner(verbosity=1).run(suite)
    if not result.wasSuccessful():
        print("TEST GATE CELLS FAILED")
        return 1
    print(f"test-gate cells: {result.testsRun} passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
