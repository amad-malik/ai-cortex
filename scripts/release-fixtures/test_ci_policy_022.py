#!/usr/bin/env python3
"""W1-284: exact approved 0.2.2 CI skips, including IT-M29."""
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
ALLOW = ROOT / "scripts/release-ci-allowlist.json"
CHECK = ROOT / "scripts/release-ci-check.py"
REASONS = {
    "prerequisite-IP64": "joined prerequisite suite is not implemented (IP-64)",
    "witness-live": "needs a running stack (Wave 2, G-S5-2): install.sh on a throwaway stack, then witness_container on the server and the database. No live result recorded.",
    "roles-proof": "needs a database: run by hand, CV2_DB=1 cv2-admit sh scripts/roles-proof.sh. last live run matches the current scripts (roles-proof: 98 passed, 0 failed)",
    "install-mac-test": "1 mandatory cell(s) not run (IT-M29): the suite proved less than all of it; the release run must run them (IT-M29 with INSTALL_MAC_REQUIRE_SET=1 and the signed set)",
}


class Policy022(unittest.TestCase):
    def test_exact_policy_and_refusals(self):
        entries = json.loads(ALLOW.read_text())
        self.assertEqual({e["gate"]: e["reason"] for e in entries}, REASONS)
        for entry in entries:
            self.assertTrue(entry["accepted_by"] and entry["close_release"])
        with tempfile.TemporaryDirectory() as td:
            log = Path(td) / "ci.log"
            def check(rows):
                log.write_text("".join(f"{gate} NOT_RUN {reason}\n" for gate, reason in rows) + f"ci: PASS-WITH-NOT-RUN ({len(rows)} gates did not run; see NOT_RUN above)\n")
                return subprocess.run([sys.executable, "-I", str(CHECK), str(log), str(ALLOW)], capture_output=True).returncode
            self.assertEqual(check(list(REASONS.items())), 0)
            for rows in ([ ("unknown", "reason") ], [("witness-live", REASONS["witness-live"] + " changed")],
                         [("install-mac-test", REASONS["install-mac-test"].replace("(IT-M29)", "(IT-M29 IT-M28)", 1))],
                         [("secret-scan", "no approved scanner installed")]):
                with self.subTest(rows=rows):
                    self.assertNotEqual(check(rows), 0)


if __name__ == "__main__":
    unittest.main()
