#!/usr/bin/env python3
"""A Mac publication requires the current signed-set check and its receipt."""
import hashlib
import importlib.util
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("postsign", Path(__file__).resolve().parents[1] / "mac-postsign-check.py")
postsign = importlib.util.module_from_spec(spec)
spec.loader.exec_module(postsign)


class Postsign(unittest.TestCase):
    def test_receipt_and_set_are_bound(self):
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            release = root / "set"
            release.mkdir()
            (release / "SHA256SUMS").write_text("signed sums\n")
            (release / "SHA256SUMS.sig").write_text("signature\n")
            key = root / "key"
            key.write_text("public key\n")
            h = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
            receipt = root / "receipt"
            text = (f"install-mac-release-check: PASS: 37 cells passed, 0 failed, 0 not run; IT-M29 ran against the signed set {release} "
                    f"(SHA256SUMS sha256 {h(release / 'SHA256SUMS')}, public key file sha256 {h(key)}), version 0.2.2\n")
            receipt.write_text(text)
            receipt_hash = h(receipt)
            def run():
                return postsign.check(release, key, "0.2.2", receipt, receipt_hash)
            with patch.object(postsign.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, text, "")):
                run()
                with self.assertRaises(ValueError):
                    postsign.check(release, key, "0.2.2", receipt, "0" * 64)
                receipt.unlink()
                with self.assertRaises(ValueError):
                    run()
                receipt.write_text(text)
                (release / "SHA256SUMS").write_text("other set\n")
                with self.assertRaises(ValueError):
                    run()
                (release / "SHA256SUMS").write_text("signed sums\n")
                key.write_text("other public key\n")
                with self.assertRaises(ValueError):
                    run()
                key.write_text("public key\n")
                (release / "SHA256SUMS.sig").unlink()
                with self.assertRaises(ValueError):
                    run()
                (release / "SHA256SUMS.sig").write_text("signature\n")
            for result in (subprocess.CompletedProcess([], 1, text, "failed"),
                           subprocess.CompletedProcess([], 0, "stale receipt\n", "")):
                with patch.object(postsign.subprocess, "run", return_value=result):
                    with self.assertRaises(ValueError):
                        run()
            def change_signature(*args, **kwargs):
                (release / "SHA256SUMS.sig").write_text("changed signature\n")
                return subprocess.CompletedProcess([], 0, text, "")
            with patch.object(postsign.subprocess, "run", side_effect=change_signature):
                with self.assertRaises(ValueError):
                    run()


if __name__ == "__main__":
    unittest.main()
