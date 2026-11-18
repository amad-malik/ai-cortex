#!/usr/bin/env python3
"""D95 controls through the builder using the separately verified, pinned Squawk."""
import importlib.util
from pathlib import Path
import shutil
import unittest

spec = importlib.util.spec_from_file_location("release_fixture", Path(__file__).with_name("test_release.py"))
fixture = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fixture)


class LiveSquawk(unittest.TestCase):
    def setUp(self):
        self.case = fixture.ReleaseTest("test_complete_release_has_reproducible_verified_assets")
        self.case.setUp()
        self.case.valid_podman()
        self.case.env.pop("CORTEX_RELEASE_SQUAWK")
        dest = self.case.root / "tools/squawk/2.71.0/squawk-linux-musl-x64"
        dest.parent.mkdir(parents=True)
        # a COPY, not a link: since O-1 gate 4 refuses a symbolic link at the pinned path (bounded no-follow digest); the digest is still checked against the pin
        shutil.copy2(fixture.ROOT / "tools/squawk/2.71.0/squawk-linux-musl-x64", dest)
        self.migration = self.case.root / "crates/cortex-db/migrations/platform/0001_platform.up.sql"

    def tearDown(self):
        self.case.tearDown()

    def test_safe_migration_builds(self):
        self.migration.write_text("SELECT 1;\n")
        result = self.case.run_release("1.2.3")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))

    def test_squawk_default_rule_blocks_nonconcurrent_index(self):
        self.migration.write_text("CREATE INDEX example ON records (id);\n")
        result = self.case.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("migration gate failed", result.stderr)
        self.assertFalse((self.case.root / "dist/1.2.3").exists())

    def test_contract_sql_and_ignored_rules_cannot_pass(self):
        for sql, error in [
            ("ALTER TABLE t DROP COLUMN c;", "DROP COLUMN"),
            ("-- squawk-ignore-file\nSELECT 1;", "squawk-ignore"),
            ("DO 'BEGIN EXECUTE ''DROP TABLE t''; END';", "DROP TABLE"),
            ("DO LANGUAGE plpgsql 'BEGIN DROP TABLE objects; END';", "DROP TABLE"),
            ("DO $$ BEGIN EXECUTE 'DR' || 'OP TABLE t'; END $$;", "dynamic SQL"),
            ("-- no statements\n", "empty migration"),
        ]:
            with self.subTest(sql=sql):
                self.migration.write_text(sql)
                result = self.case.run_release("1.2.3")
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(error, result.stderr)
                self.assertFalse((self.case.root / "dist/1.2.3").exists())

    def test_missing_migration_is_refused(self):
        self.migration.unlink()
        result = self.case.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.case.root / "dist/1.2.3").exists())

    def test_language_prefixed_do_is_refused_with_real_squawk(self):
        self.migration.write_text("DO LANGUAGE plpgsql 'BEGIN DROP TABLE objects; END';\n")
        result = self.case.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertIn("DROP TABLE", result.stderr)
        self.assertFalse((self.case.root / "dist/1.2.3").exists())


if __name__ == "__main__":
    unittest.main(verbosity=2)
