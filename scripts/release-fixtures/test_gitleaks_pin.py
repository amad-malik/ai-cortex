#!/usr/bin/env python3
"""CI refuses absent or altered Gitleaks, and runs the pinned scanner."""
import os
import json
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts/ci-secret-scan.sh"
PINNED = Path(os.environ.get("CORTEX_GITLEAKS_BIN", "/path/to/workspace/scratch/cox/d182/gitleaks/bin/gitleaks"))


class GitleaksPin(unittest.TestCase):
    def test_verified_scanner_and_failures(self):
        with tempfile.TemporaryDirectory() as td:
            repo = Path(td)
            shutil.copy(ROOT / ".gitleaks.toml", repo / ".gitleaks.toml")
            (repo / "benign.txt").write_text("no credential here\n")
            subprocess.run(["git", "init", "-q", str(repo)], check=True)
            subprocess.run(["git", "-C", str(repo), "add", "."], check=True)
            subprocess.run(["git", "-C", str(repo), "-c", "user.name=test", "-c", "user.email=test@example.invalid", "commit", "-qm", "fixture"], check=True)
            def run(binary):
                return subprocess.run(["sh", str(SCRIPT)], cwd=repo, env={**os.environ, "CORTEX_GITLEAKS_BIN": str(binary)}, capture_output=True, text=True)
            self.assertEqual(run(PINNED).returncode, 0)
            self.assertNotEqual(run(repo / "missing").returncode, 0)
            wrong = repo / "wrong"
            wrong.write_text("#!/bin/sh\nexit 0\n")
            wrong.chmod(0o755)
            self.assertNotEqual(run(wrong).returncode, 0)
            link = repo / "link"
            link.symlink_to(PINNED)
            self.assertNotEqual(run(link).returncode, 0)

    def test_only_current_head_history_is_scanned(self):
        with tempfile.TemporaryDirectory() as td:
            repo = Path(td)
            shutil.copy(ROOT / ".gitleaks.toml", repo / ".gitleaks.toml")
            subprocess.run(["git", "init", "-q", str(repo)], check=True)
            def git(*args):
                return subprocess.run(["git", "-C", str(repo), "-c", "user.name=test", "-c", "user.email=test@example.invalid", *args], check=True, capture_output=True, text=True)
            (repo / "base.txt").write_text("benign\n")
            git("add", ".")
            git("commit", "-qm", "base")
            base = git("rev-parse", "HEAD").stdout.strip()
            git("checkout", "-qb", "unrelated")
            # Synthetic test-only token, constructed at runtime and never printed.
            (repo / "other.txt").write_text("ctx" + "2_" + "a" * 32 + "." + "A" * 43 + "\n")
            git("add", "other.txt")
            git("commit", "-qm", "unrelated finding")
            git("checkout", "-q", "--detach", base)
            def scan():
                return subprocess.run(["sh", str(SCRIPT)], cwd=repo, env={**os.environ, "CORTEX_GITLEAKS_BIN": str(PINNED)}, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL).returncode
            self.assertEqual(scan(), 0, "a finding on an unrelated ref must not affect HEAD")
            (repo / "head.txt").write_text("ctx" + "2_" + "b" * 32 + "." + "B" * 43 + "\n")
            git("add", "head.txt")
            git("commit", "-qm", "head finding")
            self.assertNotEqual(scan(), 0, "a finding in HEAD history must fail")
            report = repo / "redacted-report.json"
            result = subprocess.run([str(PINNED), "detect", "--source", ".", "--config", ".gitleaks.toml", "--log-opts=HEAD", "--no-banner", "--redact", "--report-format=json", "--report-path", str(report)], cwd=repo, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            self.assertNotEqual(result.returncode, 0)
            findings = json.loads(report.read_text())
            self.assertEqual(len(findings), 1)
            fingerprint = findings[0]["Fingerprint"]
            self.assertTrue(fingerprint.endswith(":head.txt:cortex-v2-worker-token:1"))
            (repo / ".gitleaksignore").write_text(fingerprint + "\n")
            self.assertEqual(scan(), 0, "an exact cleared fingerprint should suppress that finding")
            (repo / "new.txt").write_text("ctx" + "2_" + "c" * 32 + "." + "C" * 43 + "\n")
            git("add", "new.txt")
            git("commit", "-qm", "another head finding")
            self.assertNotEqual(scan(), 0, "one exact ignore must not hide a new finding")


if __name__ == "__main__":
    unittest.main()
