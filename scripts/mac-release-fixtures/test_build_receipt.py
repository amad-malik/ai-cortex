#!/usr/bin/env python3
"""The image receipt must bind the imported archive and its source."""
import hashlib
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

HELPER = Path(__file__).resolve().parents[1] / "mac_release_manifest.py"
COMMIT = "a" * 40
IMAGE = "b" * 64


class BuildReceipt(unittest.TestCase):
    def test_receipt_binding_and_integrity(self):
        with tempfile.TemporaryDirectory() as td:
            receipt = Path(td)
            verdict = ("ok    the archive's sha256 here (" + IMAGE + ") equals the machine's own (" + IMAGE + ")\n"
                       + "ok    cortex-server --version ran once on the machine (exit 0) and prints 0.2.2 and the source's git sha " + COMMIT + " (0.2.2 " + COMMIT + ")\n")
            (receipt / "VERDICT.txt").write_text(verdict)
            sums = hashlib.sha256(verdict.encode()).hexdigest() + "  ./VERDICT.txt\n"
            (receipt / "SHA256SUMS-part-b-run-receipt.txt").write_text(sums)
            def run(commit=COMMIT, image=IMAGE):
                return subprocess.run([sys.executable, "-I", str(HELPER), "verify-receipt", "--receipt", str(receipt), "--image-sha256", image, "--image-commit", commit, "--version", "0.2.2"], capture_output=True, text=True)
            self.assertEqual(run().returncode, 0)
            self.assertNotEqual(run(commit="c" * 40).returncode, 0)
            self.assertNotEqual(run(image="d" * 64).returncode, 0)
            (receipt / "VERDICT.txt").write_text(verdict + "tampered\n")
            self.assertNotEqual(run().returncode, 0)
            (receipt / "VERDICT.txt").write_text(verdict)
            (receipt / "SHA256SUMS-part-b-run-receipt.txt").write_text(sums + hashlib.sha256(b"x").hexdigest() + "  ./../outside\n")
            self.assertNotEqual(run().returncode, 0)


if __name__ == "__main__":
    unittest.main()
