#!/usr/bin/env python3
"""Exercise CI gate wiring with isolated failing-suite and surviving-mutant fixtures."""
import io
import json
import os
from pathlib import Path
import re
import shlex
import subprocess
import tarfile
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]


class PrerequisiteCIGates(unittest.TestCase):
    def setUp(self):
        scratch = Path(os.environ["TMPDIR"]).resolve()
        if scratch == Path("/tmp") or Path("/tmp") in scratch.parents:
            self.fail("private scratch outside /tmp required")
        self.temp = tempfile.TemporaryDirectory(prefix="prereq-ci-", dir=scratch)
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.repo = self.base / "repo"
        self.repo.mkdir()
        archive = subprocess.check_output(["git", "archive", "HEAD"], cwd=ROOT)
        with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
            tar.extractall(self.repo, filter="data")
        ci = (ROOT / "scripts/ci.sh").read_text()
        isolated = []
        for line in ci.splitlines(keepends=True):
            match = re.match(r"^(\s*)gate\s+([^\s]+)\s+(?:blocking|report)\s", line)
            if match and match.group(2) not in {"install-prereq-test", "install-prereq-mutants"}:
                line = match.group(1) + ": # fixture: unrelated gate omitted\n"
            isolated.append(line)
        (self.repo / "scripts/ci.sh").write_text("".join(isolated))
        driver = self.repo / "scripts/install_prereq_mutants.py"
        text = driver.read_text()
        actual = '/path/to/workspace/spikes/cortex-v2-s0/admission/cv2-admit'
        assert text.count(actual) == 1
        driver.write_text(text.replace(actual, str(self.repo / "scripts/cv2-admit-shim")))
        (self.repo / "deploy/install-prereq.sh").write_text("target=old\n")
        (self.repo / "scripts/install-prereq-mutants.json").write_text(json.dumps([
            {"id": "SURVIVOR", "what": "fixture", "old": "old", "new": "new", "fails": "FAIL fixture"}
        ]))
        suite = self.repo / "scripts/install-prereq-test.sh"
        suite.write_text('''#!/bin/sh\nif [ "${FIXTURE_FAIL_SUITE:-0}" = 1 ]; then echo "FAIL fixture: prerequisite cell"; exit 1; fi\nprintf '%s\\n' 'ok baseline (C)' 'ok baseline (C.utf8)' 'SKIP IP-64: fixture'\n''')
        shim = self.repo / "scripts/cv2-admit-shim"
        allowed = ["sh scripts/install-prereq-test.sh", f"sh {suite}",
                   "python3 -I -B scripts/install_prereq_mutants.py",
                   "env PREREQ_MUTANT_FAST=1 python3 -I -B scripts/install_prereq_mutants.py"]
        patterns = "|".join(shlex.quote(command) for command in allowed)
        shim.write_text(f'''#!/bin/sh\ncase "$*" in\n  {patterns}) exec "$@";;\n  *) printf '%s\\n' 'REFUSED: command outside CI fixture allow-list' >&2; exit 97;;\nesac\n''')
        shim.chmod(0o755)
        self.logs = self.base / "logs"

    def run_ci(self, fail_suite):
        env = dict(os.environ, TMPDIR=str(self.base), CI_LOG_DIR=str(self.logs),
                   PREREQ_MUTANT_TMP=str(self.base), FIXTURE_FAIL_SUITE="1" if fail_suite else "0")
        return subprocess.run(["sh", "scripts/ci.sh", "--text-only"], cwd=self.repo,
                              env=env, capture_output=True, text=True, check=False)

    def test_failing_prerequisite_cell_fails_ci(self):
        result = self.run_ci(True)
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertRegex(result.stdout, r"(?m)^install-prereq-test\s+FAIL\b")
        self.assertRegex(result.stdout, r"(?m)^prerequisite-IP64\s+NOT_RUN\b")
        self.assertIn("FAIL fixture: prerequisite cell", (self.logs / "install-prereq-test.log").read_text())

    def test_fake_admission_refuses_unlisted_fetch_mutant_driver(self):
        result = subprocess.run([str(self.repo / "scripts/cv2-admit-shim"), "python3", "-I", "-B",
                                 "scripts/install_fetch_mutants.py"], cwd=self.repo,
                                capture_output=True, text=True, check=False)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("REFUSED", result.stderr)

    def test_surviving_mutant_fails_ci(self):
        result = self.run_ci(False)
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertRegex(result.stdout, r"(?m)^install-prereq-mutants\s+FAIL\b")
        self.assertIn("SURVIVED", (self.logs / "install-prereq-mutants.log").read_text())


if __name__ == "__main__":
    unittest.main()
