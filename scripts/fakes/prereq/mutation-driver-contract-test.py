#!/usr/bin/env python3
"""Regression tests for prerequisite mutation-driver failure attribution."""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
DRIVER = ROOT / "scripts/install_prereq_mutants.py"


class MutationDriverContract(unittest.TestCase):
    def setUp(self):
        scratch = os.environ.get("TMPDIR")
        if not scratch or Path(scratch).resolve().is_relative_to("/tmp"):
            self.fail("TMPDIR must name private scratch outside /tmp")
        self.temp = tempfile.TemporaryDirectory(prefix="mutation-contract-", dir=scratch)
        self.addCleanup(self.temp.cleanup)
        folder = Path(self.temp.name)
        spec = importlib.util.spec_from_file_location("driver_under_test", DRIVER)
        self.driver = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.driver)
        self.driver.SOURCE = folder / "include.sh"
        self.driver.SPEC = folder / "mutants.json"
        self.driver.SCRATCH = folder
        self.driver.SOURCE.write_text("target=old\n", encoding="utf-8")
        self.output = io.StringIO()

    def run_driver(self, item, suite):
        self.driver.SPEC.write_text(json.dumps([item]), encoding="utf-8")
        with patch.object(sys, "argv", ["driver"]), patch.object(self.driver, "suite", side_effect=suite), contextlib.redirect_stdout(self.output), contextlib.redirect_stderr(self.output):
            return self.driver.main()

    def test_stale_anchor_fails_before_running_any_suite(self):
        calls = []
        def suite(*args):
            calls.append(args)
            return 0, "ok baseline (C)\nok baseline (C.utf8)\n"
        result = self.run_driver({"id": "STALE", "old": "missing", "new": "changed", "what": "anchor", "fails": "FAIL named"}, suite)
        self.assertNotEqual(result, 0)
        self.assertIn("STALE", self.output.getvalue())
        self.assertEqual(calls, [], "invalid anchors must be reported before the slow baselines")

    def test_unrelated_first_failure_cannot_count_as_named_kill(self):
        def suite(source, locale=None):
            if source == self.driver.SOURCE:
                return 0, "ok baseline (C)\nok baseline (C.utf8)\n"
            return 1, "FAIL unrelated: first\nFAIL named: later\n"
        result = self.run_driver({"id": "WRONG", "old": "old", "new": "new", "what": "wrong first", "fails": "FAIL named"}, suite)
        self.assertNotEqual(result, 0)
        self.assertIn("WRONG TEST", self.output.getvalue())

    def test_similar_failure_prefix_is_not_the_named_cell(self):
        def suite(source, locale=None):
            if source == self.driver.SOURCE:
                return 0, "ok baseline (C)\nok baseline (C.utf8)\n"
            return 1, "FAIL named_extra: wrong case\n"
        result = self.run_driver({"id": "PREFIX", "old": "old", "new": "new", "what": "wrong prefix", "fails": "FAIL named"}, suite)
        self.assertNotEqual(result, 0)
        self.assertIn("WRONG TEST", self.output.getvalue())

    def test_fast_mode_keeps_full_baselines_and_selects_each_mutant_cell(self):
        calls = []
        def suite(source, locale=None, named=None):
            calls.append((source == self.driver.SOURCE, locale, named))
            if source == self.driver.SOURCE:
                return 0, "ok baseline (C)\nok baseline (C.utf8)\n"
            return 1, "FAIL named: expected\n"
        item = {"id": "FAST", "old": "old", "new": "new", "what": "screen", "fails": "FAIL named"}
        with patch.dict(os.environ, {"PREREQ_MUTANT_FAST": "1"}):
            result = self.run_driver(item, suite)
        self.assertEqual(result, 0, self.output.getvalue())
        self.assertEqual([named for baseline, _, named in calls if baseline], [None, None])
        self.assertEqual([named for baseline, _, named in calls if not baseline], ["FAST", "FAST"])

    def test_unknown_suite_screen_fails_before_creating_scratch(self):
        env = dict(os.environ, PREREQ_SCREEN_MUTANT="IPM-UNKNOWN", PREREQ_TEST_TMP="/dev/null/not-a-directory")
        result = subprocess.run(["sh", str(ROOT / "scripts/install-prereq-test.sh")],
                                env=env, capture_output=True, text=True, check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("unknown prerequisite mutation screen", result.stdout + result.stderr)

    def test_known_os_screen_reaches_scratch_setup(self):
        env = dict(os.environ, LC_ALL="C", PREREQ_SCREEN_MUTANT="IPM-01", PREREQ_TEST_TMP="/dev/null/not-a-directory")
        result = subprocess.run(["sh", str(ROOT / "scripts/install-prereq-test.sh")],
                                env=env, capture_output=True, text=True, check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("unknown prerequisite mutation screen", result.stdout + result.stderr)
        self.assertIn("Not a directory", result.stdout + result.stderr)

    def test_remaining_os_screens_reach_scratch_setup(self):
        for ident in ("IPM-02", "IPM-03", "IPM-30"):
            with self.subTest(ident=ident):
                env = dict(os.environ, LC_ALL="C", PREREQ_SCREEN_MUTANT=ident, PREREQ_TEST_TMP="/dev/null/not-a-directory")
                result = subprocess.run(["sh", str(ROOT / "scripts/install-prereq-test.sh")],
                                        env=env, capture_output=True, text=True, check=False)
                self.assertNotEqual(result.returncode, 0)
                self.assertNotIn("unknown prerequisite mutation screen", result.stdout + result.stderr)
                self.assertIn("Not a directory", result.stdout + result.stderr)

    def test_remaining_machine_screens_reach_scratch_setup(self):
        ids = ("09", "10", "11", "12", "14", "15", "16", "17", "19", "22", "23", "25",
               "26", "27", "31", "32", "33", "36", "37", "38", "39", "40", "41", "42")
        for number in ids:
            with self.subTest(ident=number):
                env = dict(os.environ, LC_ALL="C", PREREQ_SCREEN_MUTANT=f"IPM-{number}",
                           PREREQ_TEST_TMP="/dev/null/not-a-directory")
                result = subprocess.run(["sh", str(ROOT / "scripts/install-prereq-test.sh")],
                                        env=env, capture_output=True, text=True, check=False)
                self.assertNotIn("unknown prerequisite mutation screen", result.stdout + result.stderr)
                self.assertIn("Not a directory", result.stdout + result.stderr)


if __name__ == "__main__":
    unittest.main()
