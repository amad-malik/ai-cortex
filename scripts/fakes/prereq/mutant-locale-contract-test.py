#!/usr/bin/env python3
"""The locale self-check must work in both whole-run and fast-screen modes."""
import os
from pathlib import Path
import subprocess
import sys
import unittest

ROOT = Path(__file__).resolve().parents[3]


class LocaleSelfCheck(unittest.TestCase):
    def check_mode(self, fast):
        env = dict(os.environ)
        env.pop("PREREQ_MUTANT_FAST", None)
        if fast:
            env["PREREQ_MUTANT_FAST"] = "1"
        run = subprocess.run(
            [sys.executable, "-I", "-B", str(ROOT / "scripts/fakes/prereq/mutant-locale-test.py"), str(ROOT)],
            env=env, capture_output=True, text=True, check=False,
        )
        self.assertEqual(run.returncode, 0, run.stdout + run.stderr)

    def test_whole_run(self):
        self.check_mode(False)

    def test_fast_screen(self):
        self.check_mode(True)


if __name__ == "__main__":
    unittest.main()
