#!/usr/bin/env python3
"""Mac publication core runs only after its signed-set receipt gate."""
import hashlib
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
VERSION = "0.2.2"
TARGET = "a" * 40
REPO_ID = "12345"


class PublishGate(unittest.TestCase):
    def test_receipt_gate_precedes_core(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td) / "repo"
            root.mkdir()
            artifacts = Path(td) / "artifacts"
            artifacts.mkdir()
            scripts = root / "scripts"
            scripts.mkdir()
            for name in ("mac-publish-core.sh", "mac-postsign-check.py"):
                shutil.copy(ROOT / "scripts" / name, scripts / name)
            release = artifacts / "set"
            release.mkdir()
            names = [f"cortex-{VERSION}-macos.tar.gz", f"cortex-server-{VERSION}-aarch64.oci.tar", "install.sh", "manifest.json", "install.meta", "SHA256SUMS", "SHA256SUMS.sig"]
            for name in names:
                (release / name).write_text("fixture " + name + "\n")
            key = artifacts / "key"
            key.write_text("test public key\n")
            digest = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
            receipt = artifacts / "receipt"
            receipt.write_text(f"install-mac-release-check: PASS: 1 cells passed, 0 failed, 0 not run; IT-M29 ran against the signed set {release} (SHA256SUMS sha256 {digest(release / 'SHA256SUMS')}, public key file sha256 {digest(key)}), version {VERSION}\n")
            (scripts / "install-mac-release-check.sh").write_text(f"#!/bin/sh\ncat '{receipt}'\n")
            (scripts / "install-mac-test.sh").write_text("#!/bin/sh\nexit 0\n")
            extra = [artifacts / "THIRD-PARTY-NOTICES.txt", artifacts / "LICENSE"]
            for path in extra:
                path.write_text("fixture\n")
            asset_list = artifacts / "assets.txt"
            asset_list.write_text("".join(f"{digest(p)}  {p}\n" for p in [*(release / n for n in names), *extra]))
            inner = artifacts / "inner.sh"
            inner.write_text(f"#!/bin/sh\nprintf done > '{artifacts / 'published'}'\n")
            subprocess.run(["git", "init", "-q", str(root)], check=True)
            subprocess.run(["git", "-C", str(root), "add", "-A"], check=True)
            subprocess.run(["git", "-C", str(root), "-c", "user.name=fixture", "-c", "user.email=fixture@example.invalid", "-c", "commit.gpgsign=false", "commit", "-qm", "fixture"], check=True)
            source = subprocess.check_output(["git", "-C", str(root), "rev-parse", "HEAD"], text=True).strip()
            line = f"PUSH PRERELEASE REPO-ID {REPO_ID} v{VERSION}-macos {TARGET} SOURCE {source}"
            inner_args = ["apply-core", "amad-malik/ai-cortex", f"v{VERSION}-macos", TARGET, str(asset_list), "notes", "0" * 64, "title", "reach", REPO_ID, "oldcheck"]
            args = ["bash", str(scripts / "mac-publish-core.sh"), REPO_ID, VERSION, TARGET, str(release), str(key), str(receipt), digest(receipt), str(inner), digest(inner), source, *inner_args]
            def run(exact=line):
                return subprocess.run(args, env={**os.environ, "GATE_EXACT_LINE": exact}, capture_output=True, text=True)
            self.assertEqual(run().returncode, 0)
            (artifacts / "published").unlink()
            sums = release / "SHA256SUMS"
            original_sums = sums.read_bytes()
            sums.write_bytes(b"different signed set\n")
            self.assertNotEqual(run().returncode, 0)
            self.assertFalse((artifacts / "published").exists())
            sums.write_bytes(original_sums)
            checker = scripts / "install-mac-release-check.sh"
            original_checker = checker.read_bytes()
            checker.write_bytes(original_checker + b"# dirty\n")
            self.assertNotEqual(run().returncode, 0)
            self.assertFalse((artifacts / "published").exists())
            checker.write_bytes(original_checker)
            for action in (lambda: run("wrong go line"), lambda: receipt.unlink() or run()):
                self.assertNotEqual(action().returncode, 0)
                self.assertFalse((artifacts / "published").exists())
            receipt.write_text("wrong receipt\n")
            self.assertNotEqual(run().returncode, 0)
            self.assertFalse((artifacts / "published").exists())


if __name__ == "__main__":
    unittest.main()
