#!/usr/bin/env python3
"""Pre-sign CI proves the non-signed-set cells and defers only IT-M29."""
import importlib.util
import contextlib
import io
import os
import subprocess
import sys
import shutil
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("mac_presign", Path(__file__).resolve().parents[1] / "mac-presign-check.py")
checker = importlib.util.module_from_spec(spec)
spec.loader.exec_module(checker)

GOOD = ("ok   IT-M01 check\nskip IT-M29 NOT RUN: no signed set\n"
        "install-mac-test: 1 passed, 0 failed, 1 not run\n"
        "NOT_RUN: 1 mandatory cell(s) not run (IT-M29): deferred\n")
REQUIRED = "# cells\nIT-M01\nIT-M29\n"


class Presign(unittest.TestCase):
    def test_accepts_only_one_explicit_deferred_signed_set_cell(self):
        checker.validate(77, GOOD, REQUIRED)
        for rc, output in ((0, GOOD), (1, GOOD),
                           (77, GOOD.replace("IT-M29", "IT-M28")),
                           (77, GOOD.replace("1 not run", "2 not run")),
                           (77, GOOD.replace("0 failed", "1 failed")),
                           (77, GOOD.replace("ok   IT-M01", "FAIL IT-M01")),
                           (77, GOOD.replace("skip IT-M29", "skip IT-M28\nskip IT-M29")),
                           (77, GOOD.replace("1 passed", "0 passed")),
                           (77, GOOD.replace("1 passed", "2 passed")),
                           (77, GOOD + "extra output\n"),
                           (77, GOOD.replace("skip IT-M29 NOT RUN: no signed set\n", ""))):
            with self.subTest(rc=rc, output=output):
                with self.assertRaises(ValueError):
                    checker.validate(rc, output, REQUIRED)

    def test_missing_or_duplicate_cell_is_rejected(self):
        for required, output in ((REQUIRED + "IT-M38\n", GOOD),
                                 (REQUIRED, GOOD.replace("ok   IT-M01 check\n", "ok   IT-M01 check\nok   IT-M01 duplicate\n").replace("1 passed", "2 passed")),
                                 ("IT-M01\nIT-M01\nIT-M29\n", GOOD)):
            with self.assertRaises(ValueError):
                checker.validate(77, output, required)

    def test_removes_suite_overrides(self):
        result = subprocess.CompletedProcess([], 77, GOOD)
        with patch.dict(os.environ, {"INSTALL_MAC_ONLY": "IT-M01", "INSTALL_MAC_SH": "/tmp/other", "INSTALL_MAC_INNER": "1", "INSTALL_MAC_NO_SELFRUN": "1", "MAC_SET_DIR": "/tmp/set"}), \
             patch.object(sys, "argv", ["mac-presign-check.py"]), \
             patch.object(checker.subprocess, "run", return_value=result) as run, \
             patch.object(Path, "read_text", return_value=REQUIRED), \
             contextlib.redirect_stdout(io.StringIO()):
            with self.assertRaises(SystemExit) as exit_code:
                checker.main()
        self.assertEqual(exit_code.exception.code, 77)
        env = run.call_args.kwargs["env"]
        self.assertNotIn("INSTALL_MAC_ONLY", env)
        self.assertNotIn("INSTALL_MAC_SH", env)
        self.assertNotIn("INSTALL_MAC_INNER", env)
        self.assertNotIn("INSTALL_MAC_NO_SELFRUN", env)
        self.assertNotIn("MAC_SET_DIR", env)

    def test_ci_gate_runs_checker_and_preserves_only_valid_not_run(self):
        root = Path(__file__).resolve().parents[2]
        ci = (root / "scripts/ci.sh").read_text()
        gate_functions = ci[ci.index("gate() {"):ci.index("table() {")]
        gate_line = next(line for line in ci.splitlines() if line.startswith("gate install-mac-test "))
        with tempfile.TemporaryDirectory() as tmp:
            work = Path(tmp)
            scripts = work / "scripts"
            scripts.mkdir()
            shutil.copy(root / "scripts/mac-presign-check.py", scripts)
            (scripts / "install-mac-required-cells.txt").write_text(REQUIRED)
            (scripts / "install-mac-test.sh").write_text(
                'for name in INSTALL_MAC_SH INSTALL_MAC_INNER INSTALL_MAC_NO_SELFRUN; do\n'
                '  eval "value=\\${$name-}"; [ -z "$value" ] || exit 1\n'
                'done\ncat scripts/fake-output\nexit "$(cat scripts/fake-exit)"\n')
            (scripts / "admit").write_text('#!/bin/sh\nexec "$@"\n')
            (scripts / "admit").chmod(0o755)
            driver = ('results="" failed=0 notrun=0\n'
                      'record() { results="$results$1|$2|$3\\n"; }\n'
                      + gate_functions + '\n' + gate_line + '\nprintf "%s" "$results"\n')
            for output, rc, expected in ((GOOD, 77, "NOT_RUN"),
                                         (GOOD, 0, "FAIL"),
                                         (GOOD.replace("skip IT-M29", "skip IT-M38 extra\nskip IT-M29"), 77, "FAIL"),
                                         (GOOD.replace("ok   IT-M01 check\n", ""), 77, "FAIL")):
                (scripts / "fake-output").write_text(output)
                (scripts / "fake-exit").write_text(str(rc))
                env = dict(os.environ, admit=str(scripts / "admit"), log_dir=str(work),
                           INSTALL_MAC_SH="bad", INSTALL_MAC_INNER="1", INSTALL_MAC_NO_SELFRUN="1")
                result = subprocess.run(["sh", "-c", driver], cwd=work, env=env,
                                        text=True, capture_output=True, check=True)
                self.assertIn("install-mac-test|" + expected + "|", result.stdout)


if __name__ == "__main__":
    unittest.main()
