#!/usr/bin/env python3
"""The Mac manifest links to the Mac release entry."""
import importlib.util
import json
import os
import tempfile
import unittest
from pathlib import Path

spec = importlib.util.spec_from_file_location("mac_fixture", Path(__file__).with_name("test_mac_release.py"))
fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixture)


class MacNotesURL(unittest.TestCase):
    def test_public_default_and_override(self):
        with tempfile.TemporaryDirectory(dir=os.environ["TMPDIR"]) as td:
            fx = fixture.Fixture(td)
            repo = fixture.Repo(td, "default")
            rc, output = fixture.run_recipe(repo, fx)
            self.assertEqual(rc, 0, output)
            entry = json.loads((repo.root / "dist/0.2.2/manifest.json").read_text())["releases"][0]
            self.assertEqual(entry["notes_url"], "https://github.com/amad-malik/ai-cortex/releases/tag/v0.2.2-macos")
            repo = fixture.Repo(td, "override")
            rc, output = fixture.run_recipe(repo, fx, override={"--notes-url": "https://example.invalid/mac-notes"})
            self.assertEqual(rc, 0, output)
            entry = json.loads((repo.root / "dist/0.2.2/manifest.json").read_text())["releases"][0]
            self.assertEqual(entry["notes_url"], "https://example.invalid/mac-notes")

    def test_default_points_to_mac_entry(self):
        with tempfile.TemporaryDirectory(dir=os.environ["TMPDIR"]) as td:
            fx = fixture.Fixture(td)
            repo = fixture.Repo(td, "repo")
            rc, output = fixture.run_recipe(repo, fx)
            self.assertEqual(rc, 0, output)
            entry = json.loads((repo.root / "dist/0.2.2/manifest.json").read_text())["releases"][0]
            self.assertEqual(entry["notes_url"], "https://github.com/amad-malik/ai-cortex/releases/tag/v0.2.2-macos")


if __name__ == "__main__":
    unittest.main()
