#!/usr/bin/env python3
"""Check a real built CLI extracted from a synthetic-gated release archive."""
import importlib.util
import os
from pathlib import Path
import subprocess
import tarfile
import unittest

spec = importlib.util.spec_from_file_location("release_fixture", Path(__file__).with_name("test_release.py"))
fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixture)


class RealCliRelease(unittest.TestCase):
    def test_packaged_real_binary_reports_release_argument(self):
        binary = Path(os.environ["CORTEX_TEST_REAL_CLI"])
        self.assertEqual(binary.read_bytes()[:4], b"\x7fELF", "must be a real built executable")
        case = fixture.ReleaseTest("test_complete_release_has_reproducible_verified_assets")
        case.setUp()
        try:
            case.valid_podman()
            result = case.run_release("1.2.3", CORTEX_RELEASE_CLI=str(binary))
            self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
            with tarfile.open(case.root / "dist/1.2.3/cortex-1.2.3-x86_64-linux.tar.gz") as archive:
                member = archive.getmember("cortex")
                executable = case.root / "extracted-cortex"
                executable.write_bytes(archive.extractfile(member).read())
                executable.chmod(member.mode)
            output = subprocess.run([str(executable), "--version"], text=True, capture_output=True, check=True)
            self.assertEqual(output.stdout.split()[1], "1.2.3", output.stdout)
            print("extracted real CLI:", output.stdout.strip(), flush=True)
        finally:
            case.tearDown()


if __name__ == "__main__":
    unittest.main(verbosity=2)
