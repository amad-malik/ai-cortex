#!/usr/bin/env python3
"""A release can only accept CI skips whose exact reasons were approved."""
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

CHECKER = Path(__file__).resolve().parents[1] / "release-ci-check.py"


class CIGate(unittest.TestCase):
    def test_rejects_unapproved_skips_and_incomplete_ci(self):
        with tempfile.TemporaryDirectory() as td:
            base = Path(td)
            allow = base / "allow.json"
            log = base / "ci.log"
            allow.write_text(json.dumps([{"gate": "roles-proof", "reason": "needs a database", "accepted_by": "W1-282", "close_release": "0.3.0"}]))
            def check(text):
                log.write_text(text)
                return subprocess.run([sys.executable, "-I", str(CHECKER), str(log), str(allow)], capture_output=True).returncode
            self.assertEqual(check("roles-proof      NOT_RUN  needs a database\nci: PASS-WITH-NOT-RUN (1 gates did not run; see NOT_RUN above)\n"), 0)
            for text in ("other NOT_RUN  needs a database\nci: PASS-WITH-NOT-RUN (1 gates did not run; see NOT_RUN above)\n",
                         "roles-proof NOT_RUN wrong reason\nci: PASS-WITH-NOT-RUN (1 gates did not run; see NOT_RUN above)\n",
                         "roles-proof NOT_RUN needs a database\nci: PASS-WITH-NOT-RUN (2 gates did not run; see NOT_RUN above)\n",
                         "roles-proof NOT_RUN needs a database\n",
                         "roles-proof NOT_RUN needs a database\nroles-proof NOT_RUN needs a database\nci: PASS-WITH-NOT-RUN (2 gates did not run; see NOT_RUN above)\n",
                         "test FAIL broken\nci: PASS\n"):
                with self.subTest(text=text):
                    self.assertNotEqual(check(text), 0)


if __name__ == "__main__":
    unittest.main()
