#!/usr/bin/env python3
"""Release gate and atomicity contracts using a synthetic repository and command stubs."""
import builtins
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import tarfile
import tempfile
import time
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[2]
BIN_MEMBERS = ("cortex-dump", "cortex-dump-status", "cortex-dump-alert", "cortex-restore", "restore-compare")
SYSTEMD_MEMBERS = ("cortex-dump.service", "cortex-dump.timer", "cortex-dump-status.service",
                   "cortex-dump-status.timer", "cortex-dump-alert.service")

class ReleaseTest(unittest.TestCase):
    # The eight test seams of release.sh, in the order the script names them (RB-3).
    SEAMS = ("CORTEX_RELEASE_CI", "CORTEX_RELEASE_S5", "CORTEX_RELEASE_PROOF", "CORTEX_RELEASE_SQUAWK",
             "CORTEX_RELEASE_CLI", "CORTEX_RELEASE_GIT_SHA", "CORTEX_RELEASE_CRATES_DIR", "CORTEX_RELEASE_TOML")

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="release-test-", dir=os.environ["TMPDIR"])
        self.root = Path(self.temp.name)
        for d in ["scripts", "deploy/initdb", "deploy/systemd", "crates/cortex-db/migrations/platform", "crates/cortex-server", "bin"]:
            (self.root / d).mkdir(parents=True, exist_ok=True)
        shutil.copy(ROOT / "scripts/release.sh", self.root / "scripts/release.sh")
        shutil.copy(ROOT / "scripts/release_manifest.py", self.root / "scripts/release_manifest.py")
        shutil.copy(ROOT / "scripts/release-ci-check.py", self.root / "scripts/release-ci-check.py")
        shutil.copy(ROOT / "scripts/release-ci-allowlist.json", self.root / "scripts/release-ci-allowlist.json")
        shutil.copy(ROOT / "scripts/release-squawk.toml", self.root / "scripts/release-squawk.toml")
        (self.root / "deploy/compose.yml").write_text(
            "services:\n  db:\n    image: registry.invalid/db@sha256:" + "a" * 64 +
            "\n  migrate:\n    image: localhost/cv2-s5-server:dev\n    pull_policy: never\n  server:\n    build: .\n    image: localhost/cv2-s5-server:dev\n    pull_policy: never\n")
        (self.root / "deploy/Containerfile").write_text("FROM scratch\n")
        shutil.copy(ROOT / "scripts/release-fixtures/server-features.fixture", self.root / "deploy/server-features")
        (self.root / "deploy/db-roles.sh").write_text("#!/bin/sh\nexit 0\n")
        (self.root / "deploy/install.sh").write_text('#!/bin/sh\n. "$here/install-prereq.sh"   # @include\n. "$here/install-fetch.sh"   # @include\n')
        (self.root / "deploy/install-prereq.sh").write_text('machine_look() { return 0; }\n')
        (self.root / "deploy/install-fetch.sh").write_text('rel_fetch() { return 0; }\n')
        (self.root / "deploy/initdb/roles.sql").write_text("SELECT 1;\n")
        (self.root / "deploy/cortex.toml.example").write_text('[modules]\nenabled = ["audit"]\n')
        for name in BIN_MEMBERS:
            (self.root / "scripts" / name).write_text("#!/bin/sh\n# " + name + "\n")
        for name in SYSTEMD_MEMBERS:
            (self.root / "deploy/systemd" / name).write_text("# @CORTEX_HOME@ @DB_CONTAINER@ " + name + "\n")
        (self.root / "crates/cortex-db/migrations/platform/0001_platform.up.sql").write_text("SELECT 1;\n")
        # X-1: gate 2 (the S5 checker) reads crates/cortex-server/Cargo.toml beside the layout; a release copies it there.
        (self.root / "crates/cortex-server/Cargo.toml").write_text(
            '[package]\nname = "cortex-server"\n\n[features]\ntest-faults = []\nmod-policy = []\nmod-records = []\n'
            'mod-search = []\nmod-handoffs = []\nmod-boot = []\n')
        self.log = self.root / "calls"
        self.env = os.environ.copy()
        self.env.update({"SOURCE_DATE_EPOCH": "1700000000", "RELEASE_TEST_LOG": str(self.log),
                         "CORTEX_RELEASE_CI": str(self.root / "bin/ci"),
                         "CORTEX_RELEASE_S5": str(self.root / "bin/s5"),
                         "CORTEX_RELEASE_PROOF": str(self.root / "bin/proof"),
                         "CORTEX_RELEASE_SQUAWK": str(self.root / "bin/squawk"),
                         "CORTEX_RELEASE_CLI": str(self.root / "bin/cortex"),
                         "CORTEX_RELEASE_GIT_SHA": "a" * 40,
                         # RB-3: the test seams above are honoured only when this names the very root being built.
                         "CORTEX_RELEASE_FIXTURE_ROOT": str(self.root),
                         "PATH": str(self.root / "bin") + os.pathsep + os.environ["PATH"]})
        for name in ["ci", "s5", "proof", "squawk", "cortex", "podman"]:
            script = self.root / "bin" / name
            script.write_text('#!/bin/sh\nif [ "${1:-}" = "--version" ]; then echo "cortex 1.2.3 (fixture)"; exit 0; fi\n' if name == "cortex" else
                              '#!/bin/sh\nprintf "%s\\n" "' + name + '" >> "$RELEASE_TEST_LOG"\n'
                              '[ "${RELEASE_TEST_FAIL:-}" != "' + name + '" ] || exit 1\n'
                              + ('printf "%s\\n" "${RELEASE_TEST_PROOF:-last live run matches the current scripts}"\n' if name == "proof" else '')
                              + ('printf "fixture PASS\\nci: PASS\\n"\n' if name == "ci" else '')
                              + 'exit 0\n')
            script.chmod(0o755)
        self.metadata_fixture("1.2.3")
        self.metadata_custom = False
        self.exc_write([], [])   # the explicit schema-2 EMPTY list and its exact zero-entry record: "no waivers" (a missing list is refused, W1-107)
        cargo = self.root / "bin/cargo"
        cargo.write_text('''#!/bin/sh
set -eu
[ "$1" = metadata ] || exit 1
[ ! -f "$(dirname "$RELEASE_TEST_LOG")/metadata-fail" ] || exit 1
cat "$(dirname "$RELEASE_TEST_LOG")/metadata.json"
''')
        cargo.chmod(0o755)

    def tearDown(self):
        self.temp.cleanup()

    def run_release(self, *args, no_key=False, **env):
        if not self.metadata_custom and args and re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?", args[0]):
            self.metadata_fixture(args[0])
            self.metadata_custom = False
        # a real (git-checked) release needs --trust-key-file: a fixture made by git_root() carries the key and the option is added here
        extra = ["--trust-key-file", str(self.anchor_key)] if getattr(self, "anchor_key", None) and not no_key and "--trust-key-file" not in args else []
        return subprocess.run(["sh", str(self.root / "scripts/release.sh"), *args, *extra], cwd=self.root,
                              env={**self.env, **env}, text=True, capture_output=True, timeout=180)

    def calls(self):
        return self.log.read_text().splitlines() if self.log.exists() else []

    def test_unapproved_ci_skip_stops_release_before_build(self):
        self.valid_podman()
        ci = self.root / "bin/ci"
        ci.write_text("#!/bin/sh\nprintf 'unapproved NOT_RUN reason\\nci: PASS-WITH-NOT-RUN (1 gates did not run; see NOT_RUN above)\\n'\n")
        result = self.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("CI results are not approved", result.stderr)
        self.assertNotIn("podman build", self.calls())
        self.assertFalse((self.root / "dist/1.2.3").exists())

    def metadata_fixture(self, version, *, inherited=True, fail=False):
        self.metadata_custom = True
        manifest = self.root / "crates/cortex-cli/Cargo.toml"
        manifest.parent.mkdir(parents=True, exist_ok=True)
        manifest.write_text('[package]\nname = "cortex-cli"\n' +
                            ('version.workspace = true\n' if inherited else f'version = "{version}"\n'))
        (self.root / "Cargo.toml").write_text(f'[workspace.package]\nversion = "{version}"\n')
        metadata = {"workspace_root": str(self.root), "workspace_members": ["cli"],
                    "packages": [{"id": "cli", "name": "cortex-cli", "version": version,
                                  "manifest_path": str(manifest)}]}
        (self.root / "metadata.json").write_text(json.dumps(metadata))
        failure = self.root / "metadata-fail"
        failure.unlink(missing_ok=True)
        if fail:
            failure.touch()

    def test_release_version_must_match_workspace_before_gates(self):
        self.metadata_fixture("1.2.4")
        result = self.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.calls(), [])
        self.assertFalse((self.root / "dist").exists())

    def test_development_version_is_not_a_release(self):
        self.metadata_fixture("0.1.0")
        result = self.run_release("0.1.0")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.calls(), [])
        self.assertFalse((self.root / "dist").exists())

    def test_workspace_crate_must_inherit_version(self):
        self.metadata_fixture("1.2.3", inherited=False)
        result = self.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.calls(), [])
        self.assertFalse((self.root / "dist").exists())

    def test_metadata_failure_stops_before_gates(self):
        self.metadata_fixture("1.2.3", fail=True)
        result = self.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.calls(), [])
        self.assertFalse((self.root / "dist").exists())

    def test_expanded_archive_has_each_required_member_with_exact_mode_and_bytes(self):
        self.valid_podman()
        result = self.run_release("1.2.3")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        expected = {"cortex.toml.example": ("deploy/cortex.toml.example", 0o644)}
        expected.update({"bin/" + name: ("scripts/" + name, 0o755) for name in BIN_MEMBERS})
        expected.update({"systemd/" + name: ("deploy/systemd/" + name, 0o644) for name in SYSTEMD_MEMBERS})
        with tarfile.open(self.root / "dist/1.2.3/cortex-1.2.3-x86_64-linux.tar.gz", "r:gz") as tar:
            entries = {m.name.rstrip('/'): m for m in tar.getmembers()}
            for directory in ("bin", "systemd"):
                with self.subTest(member=directory):
                    self.assertIn(directory, entries)
                    self.assertTrue(entries[directory].isdir())
                    self.assertEqual(entries[directory].mode, 0o755)
            for name, (source, mode) in expected.items():
                with self.subTest(member=name):
                    self.assertIn(name, entries)
                    self.assertTrue(entries[name].isfile())
                    self.assertEqual(entries[name].mode, mode)
                    self.assertEqual(tar.extractfile(name).read(), (self.root / source).read_bytes())
            self.assertEqual(set(entries), {"cortex", "compose.yml", "stack.env", "db-roles.sh", "initdb", "initdb/roles.sql", "bin", "systemd", *expected})

    def test_each_missing_expanded_source_refuses_release(self):
        sources = ["deploy/cortex.toml.example", *("scripts/" + n for n in BIN_MEMBERS),
                   *("deploy/systemd/" + n for n in SYSTEMD_MEMBERS)]
        self.valid_podman()
        for source in sources:
            with self.subTest(source=source):
                path = self.root / source
                data = path.read_bytes()
                path.unlink()
                result = self.run_release("1.2.3")
                published = (self.root / "dist/1.2.3").exists()
                shutil.rmtree(self.root / "dist/1.2.3", ignore_errors=True)
                path.write_bytes(data)
                self.assertNotEqual(result.returncode, 0, source)
                self.assertFalse(published)

    def test_installer_is_separate_checksummed_asset(self):
        self.valid_podman()
        result = self.run_release("1.2.3")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        dist = self.root / "dist/1.2.3"
        installer = dist / "install.sh"
        self.assertTrue(installer.is_file())
        self.assertEqual(installer.read_bytes(), b'#!/bin/sh\nmachine_look() { return 0; }\nrel_fetch() { return 0; }\n')
        self.assertEqual(installer.stat().st_mode & 0o777, 0o755)
        sums = (dist / "SHA256SUMS").read_text()
        self.assertIn(hashlib.sha256(installer.read_bytes()).hexdigest() + "  install.sh\n", sums)
        with tarfile.open(dist / "cortex-1.2.3-x86_64-linux.tar.gz", "r:gz") as archive:
            self.assertNotIn("install.sh", archive.getnames())

    def test_installer_joins_both_sources_into_one_checked_asset(self):
        template = ('#!/bin/sh\n. "$here/install-prereq.sh"   # @include\n'
                    '. "$here/install-fetch.sh"   # @include\n')
        prerequisite = 'machine_look() { return 0; }\nprereq_run() { return 0; }\n'
        fetch = 'rel_fetch() ( return 0; )\nrel_trust()\n{ return 0; }\n'
        (self.root / "deploy/install.sh").write_text(template)
        (self.root / "deploy/install-prereq.sh").write_text(prerequisite)
        (self.root / "deploy/install-fetch.sh").write_text(fetch)
        self.valid_podman()
        result = self.run_release("1.2.3")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        installer = (self.root / "dist/1.2.3/install.sh").read_text()
        self.assertEqual(installer, '#!/bin/sh\n' + prerequisite + fetch)
        self.assertNotIn('# @include', installer)
        self.assertEqual(subprocess.run(['sh', '-n'], input=installer, text=True).returncode, 0)

    def test_installer_joins_guarded_development_includes(self):
        template = ('#!/bin/sh\n[ ! -f "$here/install-prereq.sh" ] || . "$here/install-prereq.sh"   # @include\n'
                    '[ ! -f "$here/install-fetch.sh" ] || . "$here/install-fetch.sh"   # @include\n')
        (self.root / 'deploy/install.sh').write_text(template)
        self.valid_podman()
        result = self.run_release('1.2.3')
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        expected = ('#!/bin/sh\n' + (self.root / 'deploy/install-prereq.sh').read_text()
                    + (self.root / 'deploy/install-fetch.sh').read_text())
        self.assertEqual((self.root / 'dist/1.2.3/install.sh').read_text(), expected)

    def test_installer_join_refuses_missing_or_invalid_sources_and_markers(self):
        self.valid_podman()
        template = self.root / 'deploy/install.sh'
        prereq = self.root / 'deploy/install-prereq.sh'
        fetch = self.root / 'deploy/install-fetch.sh'
        original = {p: p.read_bytes() for p in (template, prereq, fetch)}
        for case in ('missing-prereq', 'missing-fetch', 'symlink', 'missing-marker',
                     'duplicate-marker', 'leftover-marker', 'invalid-shell', 'no-function', 'shebang'):
            with self.subTest(case=case):
                for path, data in original.items():
                    path.unlink(missing_ok=True)
                    path.write_bytes(data)
                if case == 'missing-prereq':
                    prereq.unlink()
                elif case == 'missing-fetch':
                    fetch.unlink()
                elif case == 'symlink':
                    fetch.unlink()
                    fetch.symlink_to('install-prereq.sh')
                elif case == 'missing-marker':
                    template.write_bytes(original[template].replace(b'   # @include', b'', 1))
                elif case == 'duplicate-marker':
                    template.write_bytes(original[template] + b'[ ! -f "$here/install-fetch.sh" ] || . "$here/install-fetch.sh"   # @include\n')
                elif case == 'leftover-marker':
                    template.write_bytes(original[template] + b'# @include\n')
                elif case == 'invalid-shell':
                    fetch.write_text('rel_fetch() { if; }\n')
                elif case == 'no-function':
                    fetch.write_text('# no functions\n')
                else:
                    fetch.write_text('#!/bin/sh\nrel_fetch() { return 0; }\n')
                result = self.run_release('1.2.3')
                self.assertNotEqual(result.returncode, 0, case)
                self.assertFalse((self.root / 'dist/1.2.3').exists(), case)

    def test_installer_join_accepts_subshell_and_next_line_function_bodies(self):
        self.valid_podman()
        fetch = self.root / 'deploy/install-fetch.sh'
        for source in ('rel_fetch() ( return 0; )\n', 'rel_fetch()\n{ return 0; }\n'):
            with self.subTest(source=source):
                fetch.write_text(source)
                result = self.run_release('1.2.3')
                self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
                self.assertIn(source, (self.root / 'dist/1.2.3/install.sh').read_text())
                shutil.rmtree(self.root / 'dist/1.2.3')

    def test_install_metadata_matches_manifest_and_release_archive(self):
        self.valid_podman()
        result = self.run_release("1.2.3")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        dist = self.root / "dist/1.2.3"
        metadata = dist / "install.meta"
        self.assertTrue(metadata.is_file(), "install.meta must be a release sibling")
        release = json.loads((dist / "manifest.json").read_text())["releases"][0]
        cortex = release["cortex"]
        expected = ["META_SCHEMA=1", "RELEASE_VERSION=" + release["version"],
                    "STACK_VERSION=" + release["version"], "MEMBER_LIST=" + str(cortex["members"]),
                    "CORTEX_SERVER_IMAGE=" + cortex["server_image"]["ref"],
                    "CORTEX_DB_IMAGE=" + cortex["db_image"],
                    "ROLES_SCRIPT_DIGEST=" + cortex["roles_script_digest"].removeprefix("sha256:"),
                    # RB-3: this run is a seam run (the fixture sets six seams), so the artefact says so.
                    "RELEASE_SEAMS=" + ",".join(name for name in self.SEAMS if self.env.get(name))]
        self.assertEqual(metadata.read_bytes(), ("\n".join(expected) + "\n").encode("ascii"))
        self.assertLessEqual(metadata.stat().st_size, 4096)
        self.assertIn(hashlib.sha256(metadata.read_bytes()).hexdigest() + "  install.meta\n",
                      (dist / "SHA256SUMS").read_text())
        with tarfile.open(dist / "cortex-1.2.3-x86_64-linux.tar.gz", "r:gz") as archive:
            self.assertNotIn("install.meta", archive.getnames())
            stack = dict(line.split("=", 1) for line in archive.extractfile("stack.env").read().decode().splitlines())
            self.assertEqual(stack["CORTEX_VERSION"], release["version"])
            self.assertEqual(stack["CORTEX_SERVER_IMAGE"], cortex["server_image"]["ref"])
            self.assertEqual(stack["CORTEX_DB_IMAGE"], cortex["db_image"])
            self.assertEqual(hashlib.sha256(archive.extractfile("db-roles.sh").read()).hexdigest(),
                             cortex["roles_script_digest"].removeprefix("sha256:"))

    def test_missing_installer_refuses_release(self):
        (self.root / "deploy/install.sh").unlink()
        self.valid_podman()
        result = self.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.root / "dist/1.2.3").exists())

    def test_installer_source_symlink_is_refused(self):
        installer = self.root / "deploy/install.sh"
        copy = self.root / "installer-copy"
        copy.write_bytes(installer.read_bytes())
        installer.unlink()
        installer.symlink_to(copy)
        self.valid_podman()
        result = self.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.root / "dist/1.2.3").exists())

    def test_dist_verification_requires_every_sibling_and_exact_hash(self):
        self.valid_podman()
        result = self.run_release("1.2.3")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        source = self.root / "dist/1.2.3"

        def check(path):
            return subprocess.run(["python3", "-I", str(self.root / "scripts/release_manifest.py"),
                                   "verify-dist", "--output-dir", str(path), "--version", "1.2.3"],
                                  text=True, capture_output=True)

        self.assertEqual(check(source).returncode, 0, "valid distribution must pass")
        assets = ["cortex-1.2.3-x86_64-linux.tar.gz", "cortex-server-1.2.3-x86_64.oci.tar", "install.sh", "manifest.json", "install.meta"]
        cases = [(kind, name) for kind in ("missing", "changed", "unlisted", "symlink") for name in assets]
        cases += [("extra", "extra"), ("listed-extra", "extra"), ("duplicate", "install.sh")]
        for index, (kind, name) in enumerate(cases):
            with self.subTest(kind=kind, asset=name):
                directory = self.root / f"dist-variant-{index}"
                shutil.copytree(source, directory)
                path = directory / name
                sums = directory / "SHA256SUMS"
                if kind == "missing":
                    path.unlink()
                elif kind == "changed":
                    path.write_bytes(b"tampered")
                elif kind == "unlisted":
                    sums.write_text(''.join(line for line in sums.read_text().splitlines(keepends=True) if not line.endswith('  ' + name + '\n')))
                elif kind == "symlink":
                    external = self.root / f"same-bytes-{index}"
                    external.write_bytes(path.read_bytes())
                    path.unlink()
                    path.symlink_to(external)
                elif kind in ("extra", "listed-extra"):
                    path.write_bytes(b"extra")
                    if kind == "listed-extra":
                        with sums.open("a") as stream:
                            stream.write(hashlib.sha256(b"extra").hexdigest() + "  extra\n")
                else:
                    with sums.open("a") as stream:
                        stream.write(hashlib.sha256(path.read_bytes()).hexdigest() + "  " + name + "\n")
                self.assertNotEqual(check(directory).returncode, 0, (kind, name))

    def test_builder_refuses_if_dist_verification_fails(self):
        self.valid_podman()
        helper = self.root / "scripts/release_manifest.py"
        helper.rename(helper.with_name("release_manifest_original.py"))
        helper.write_text('''import pathlib, runpy, sys
if sys.argv[1] == "verify-dist":
    raise SystemExit(42)
runpy.run_path(str(pathlib.Path(__file__).with_name("release_manifest_original.py")), run_name="__main__")
''')
        result = self.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.root / "dist/1.2.3").exists())

    def test_squawk_uses_explicit_config_json_and_no_credential_environment(self):
        config = self.root / "scripts/release-squawk.toml"
        config.write_text('excluded_paths = []\nexcluded_rules = []\nincluded_rules = []\n')
        squawk = self.root / "bin/squawk"
        squawk.write_text(f'''#!/bin/sh
printf '%s\\n' "$@" > "{self.root}/squawk-args"
if [ "${{RELEASE_TEST_CREDENTIAL+x}}" ]; then exit 1; fi
printf '%s\\n' squawk >> "{self.log}"
''')
        squawk.chmod(0o755)
        self.run_release("1.2.3", RELEASE_TEST_CREDENTIAL="do-not-pass")
        self.assertTrue((self.root / "squawk-args").exists(), "the squawk gate must have run")
        args = (self.root / "squawk-args").read_text().splitlines()
        self.assertIn("--config", args)
        self.assertEqual(args[args.index("--config") + 1], str(config))
        self.assertIn("--reporter", args)
        self.assertEqual(args[args.index("--reporter") + 1], "json")
        self.assertIn("squawk", self.calls())

    def test_squawk_inline_ignore_directives_are_refused(self):
        migration = self.root / "crates/cortex-db/migrations/platform/0001_platform.up.sql"
        for directive in ("-- squawk-ignore ban-drop-column", "-- squawk-ignore-file"):
            with self.subTest(directive=directive):
                migration.write_text(directive + "\nSELECT 1;\n")
                result = self.run_release("1.2.3")
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("squawk-ignore", result.stderr)
                self.assertFalse((self.root / "dist/1.2.3").exists())

    def test_unapproved_migration_exception_refuses_release(self):
        self.valid_podman()
        exception = {
            "schema": 1,
            "exceptions": [{
                "path": "crates/cortex-db/migrations/platform/0001_platform.up.sql",
                "sha256": "d5c79f3b15643af101343eed72888a2e2ab490577fa57047037e0553c4ceb3f4",
                "approved_by": "",
                "dynamic_sql_lines": [83, 91, 103, 104],
                "squawk": [{"rule": "require-lock-timeout", "line": 2},
                           {"rule": "require-statement-timeout", "line": 2},
                           {"rule": "prefer-bigint-over-int", "line": 7},
                           {"rule": "prefer-bigint-over-int", "line": 34}],
            }],
        }
        config = self.root / "scripts/release-migration-exceptions.json"
        for approval in ("", "PENDING", "00000000-0000-0000-0000-000000000000"):
            with self.subTest(approval=approval):
                exception["exceptions"][0]["approved_by"] = approval
                config.write_text(json.dumps(exception) + "\n")
                result = self.run_release("1.2.3")
                published = (self.root / "dist/1.2.3").exists()
                shutil.rmtree(self.root / "dist/1.2.3", ignore_errors=True)
                self.assertNotEqual(result.returncode, 0, approval)
                self.assertFalse(published)

    def test_exception_candidate_is_exact_path_hash_and_dynamic_lines(self):
        spec = importlib.util.spec_from_file_location("release_manifest", ROOT / "scripts/release_manifest.py")
        helper = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(helper)
        self.assertTrue(hasattr(helper, "exact_baseline_exception"), "candidate validator is missing")
        data = json.loads((ROOT / "scripts/release-migration-exceptions.json").read_text())["exceptions"][0]
        relative = Path(data["path"])
        migration = self.root / relative
        migration.write_bytes((ROOT / relative).read_bytes())
        self.assertTrue(helper.exact_baseline_exception(self.root, migration, data))
        original = migration.read_bytes()
        migration.write_bytes(original + b"\n")
        self.assertFalse(helper.exact_baseline_exception(self.root, migration, data))
        migration.write_bytes(original)
        other = migration.with_name("0002_platform.up.sql")
        other.write_bytes(original)
        self.assertFalse(helper.exact_baseline_exception(self.root, other, data))
        changed = dict(data, dynamic_sql_lines=[83, 91, 103, 105])
        self.assertFalse(helper.exact_baseline_exception(self.root, migration, changed))

    def test_exception_squawk_findings_require_exact_rule_line_multiset(self):
        spec = importlib.util.spec_from_file_location("release_manifest", ROOT / "scripts/release_manifest.py")
        helper = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(helper)
        self.assertTrue(hasattr(helper, "exact_baseline_squawk"), "finding validator is missing")
        migration = self.root / "crates/cortex-db/migrations/platform/0001_platform.up.sql"
        findings = [{"file": str(migration), "rule_name": rule, "line": line} for rule, line in
                    (("require-lock-timeout", 2), ("require-statement-timeout", 2),
                     ("prefer-bigint-over-int", 7), ("prefer-bigint-over-int", 34))]
        self.assertTrue(helper.exact_baseline_squawk(migration, findings))
        self.assertFalse(helper.exact_baseline_squawk(migration, findings + [findings[0]]))
        self.assertFalse(helper.exact_baseline_squawk(migration, findings[:-1]))
        self.assertFalse(helper.exact_baseline_squawk(migration, findings + [{"file": str(migration), "rule_name": "other", "line": 2}]))
        self.assertFalse(helper.exact_baseline_squawk(migration, [dict(findings[0], file=str(migration.with_name("0002_platform.up.sql"))), *findings[1:]]))

    def test_approval_record_pins_all_local_inputs(self):
        # the platform-only list in the schema-2 form (cox b36a989b): one entry, one pin; every input the record pins is attacked in turn
        helper = self.exc_helper()
        self.assertTrue(hasattr(helper, "verified_approval_record"), "approval record validator is missing")
        entries, pins = self.exc_fixture(v2=())
        config = self.root / "scripts/release-migration-exceptions.json"
        record = self.root / "scripts/release-migration-approval.json"
        migration = self.root / self.PLATFORM_PATH
        review = self.root / "ReviewService/example/REVIEW.md"
        self.assertTrue(helper.verified_approval_record(self.root, config, record))
        original_config, original_migration, original_review, original_record = config.read_bytes(), migration.read_bytes(), review.read_bytes(), record.read_bytes()
        for attack in ("missing_record", "empty_row", "placeholder_row", "other_migration", "exception_byte", "migration_byte", "review_byte", "missing_review"):
            with self.subTest(attack=attack):
                config.write_bytes(original_config)
                migration.write_bytes(original_migration)
                review.write_bytes(original_review)
                record.write_bytes(original_record)
                pinned = json.loads(original_record)
                if attack == "missing_record":
                    record.unlink()
                elif attack in ("empty_row", "placeholder_row", "other_migration"):
                    if attack == "empty_row": pinned["entries"][0]["approved_by"] = ""
                    elif attack == "placeholder_row": pinned["entries"][0]["approved_by"] = "00000000-0000-0000-0000-000000000000"
                    else: pinned["entries"][0]["migration_path"] = "crates/cortex-db/migrations/platform/0002_platform.up.sql"
                    record.write_text(json.dumps(pinned) + "\n")
                elif attack == "exception_byte": config.write_bytes(original_config + b" ")
                elif attack == "migration_byte": migration.write_bytes(original_migration + b" ")
                elif attack == "review_byte": review.write_bytes(original_review + b" ")
                else: review.unlink()
                self.assertFalse(helper.verified_approval_record(self.root, config, record), attack)

    def prepare_synthetic_approved_baseline(self, extra_finding=False):
        self.exc_fixture(v2=())
        platform = self.root / self.PLATFORM_PATH
        report = [{"file": str(platform), "rule_name": rule, "line": line} for rule, line in
                  (("require-lock-timeout", 2), ("require-statement-timeout", 2), ("prefer-bigint-over-int", 7), ("prefer-bigint-over-int", 34))]
        if extra_finding:
            report.append({"file": str(platform), "rule_name": "unexpected-rule", "line": 2})
        self.exc_squawk_stub(platform_report=json.dumps(report))
        return platform

    def test_approved_exact_baseline_passes_both_migration_gates(self):
        self.prepare_synthetic_approved_baseline()
        result = self.run_release("1.2.3")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertIn("squawk", self.calls())
        self.assertTrue((self.root / "dist/1.2.3").is_dir())

    def test_approved_baseline_rejects_changed_byte(self):
        migration = self.prepare_synthetic_approved_baseline()
        migration.write_bytes(migration.read_bytes() + b" ")
        result = self.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("approval", result.stderr)
        self.assertFalse((self.root / "dist/1.2.3").exists())

    def test_approved_baseline_does_not_except_another_dynamic_migration(self):
        migration = self.prepare_synthetic_approved_baseline()
        migration.with_name("0002_platform.up.sql").write_bytes(migration.read_bytes())
        result = self.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("dynamic SQL", result.stderr)
        self.assertFalse((self.root / "dist/1.2.3").exists())

    def test_approved_baseline_rejects_fifth_squawk_finding(self):
        self.prepare_synthetic_approved_baseline(extra_finding=True)
        result = self.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("approved baseline findings differ", result.stderr)
        self.assertIn("squawk", self.calls())
        self.assertFalse((self.root / "dist/1.2.3").exists())

    def test_empty_migration_is_refused(self):
        migration = self.root / "crates/cortex-db/migrations/platform/0001_platform.up.sql"
        migration.write_text(" \n-- no SQL\n")
        result = self.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("empty migration", result.stderr)
        self.assertNotIn("podman", " ".join(self.calls()))

    def test_invalid_versions_cannot_run_gates_or_create_files(self):
        for version in ["1.0.0;touch x", "$(id)", "1.0.0\n", "x" * 200, "1.0", "1.0.0+metadata", "1.0.0.part"]:
            result = self.run_release(version)
            self.assertNotEqual(result.returncode, 0, version)
            self.assertEqual(self.calls(), [])
            self.assertFalse((self.root / "dist").exists())
        # The mocked workspace agrees with an invalid version, so metadata
        # mismatch cannot hide a missing SemVer check.
        self.metadata_fixture("1.0")
        self.assertNotEqual(self.run_release("1.0").returncode, 0)
        self.assertEqual(self.calls(), [])

    def test_existing_destination_is_untouched_and_runs_no_gate(self):
        dest = self.root / "dist/1.2.3"
        dest.mkdir(parents=True)
        (dest / "keep").write_text("preserve")
        self.assertNotEqual(self.run_release("1.2.3").returncode, 0)
        self.assertEqual((dest / "keep").read_text(), "preserve")
        self.assertEqual(self.calls(), [])

    def test_gates_run_in_order_and_fail_before_build(self):
        for failed, expected in [("ci", ["ci"]), ("s5", ["ci", "s5"]),
                                 ("proof", ["ci", "s5", "proof"]),
                                 ("squawk", ["ci", "s5", "proof", "squawk"])]:
            self.log.unlink(missing_ok=True)
            result = self.run_release("1.2.3", RELEASE_TEST_FAIL=failed)
            self.assertNotEqual(result.returncode, 0, (failed, result.stdout, result.stderr))
            self.assertEqual(self.calls(), expected)
            self.assertFalse((self.root / "dist/1.2.3").exists())
            self.assertFalse((self.root / "dist/1.2.3.part").exists())

    def test_notes_and_label_validation_happens_before_gates(self):
        for flag, value in [("--notes-url", "http://x"), ("--notes-url", "https://x/with space"),
                            ("--notes-url", 'https://x/"'), ("--notes-url", "https://x/\\"),
                            ("--notes-url", "https://x/\n"), ("--notes-url", "https://x/" + "a" * 2001),
                            ("--label", ""), ("--label", "x" * 65), ("--label", ".bad"),
                            ("--label", "bad value"), ("--label", "bad/value")]:
            self.assertNotEqual(self.run_release("1.2.3", flag, value).returncode, 0)
            self.assertEqual(self.calls(), [])
            self.assertFalse((self.root / "dist").exists())
        self.log.unlink(missing_ok=True)
        self.run_release("1.2.3", "--notes-url", "https://example.invalid/notes", "--label", "0.2.001")
        self.assertEqual(self.calls()[:4], ["ci", "s5", "proof", "squawk"])

    def test_child_commands_use_private_work_directory_for_temporary_files(self):
        ci = self.root / "bin/ci"
        ci.write_text("""#!/bin/sh
printf '%s\\n' ci >> "$RELEASE_TEST_LOG"
printf '%s\\n' "$TMPDIR" > "$(dirname "$RELEASE_TEST_LOG")/child-tmp"
exit 1
""")
        ci.chmod(0o755)
        result = self.run_release("1.2.3", TMPDIR="/tmp")
        self.assertNotEqual(result.returncode, 0)
        child_tmp_file = self.root / "child-tmp"
        self.assertTrue(child_tmp_file.is_file(), "CI did not record its temporary directory")
        child_tmp = child_tmp_file.read_text().strip()
        self.assertTrue(child_tmp.startswith(str(self.root / "target") + "/"), child_tmp)
        self.assertFalse(Path(child_tmp).exists())

    def test_minimum_version_is_validated_before_gates(self):
        for minimum in ["", "1.0", "1.0.0;touch x", "01.0.0", "2.0.0+metadata"]:
            result = self.run_release("1.2.3", "--min-upgrade-from", minimum)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(self.calls(), [])
        self.valid_podman()
        result = self.run_release("1.2.3", "--min-upgrade-from", "1.0.0")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        release = json.loads((self.root / "dist/1.2.3/manifest.json").read_text())["releases"][0]
        self.assertEqual(release["cortex"]["min_upgrade_from"], "1.0.0")

    def test_both_server_services_use_release_image_before_s5(self):
        s5 = self.root / "bin/s5"
        s5.write_text("""#!/bin/sh
set -eu
printf '%s\\n' s5 >> "$RELEASE_TEST_LOG"
[ "$(grep -F -c 'image: ${CORTEX_SERVER_IMAGE:?}' "$1/compose.yml")" -eq 2 ]
! grep -q '    build:' "$1/compose.yml"
""")
        s5.chmod(0o755)
        self.run_release("1.2.3")
        self.assertEqual(self.calls()[:4], ["ci", "s5", "proof", "squawk"])

    def test_missing_migration_root_stops_before_build(self):
        missing = self.root / "absent-crates"
        result = self.run_release("1.2.3", CORTEX_RELEASE_CRATES_DIR=str(missing))
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("podman", " ".join(self.calls()))
        self.assertFalse((self.root / "dist/1.2.3.part").exists())

    def test_missing_platform_migration_stops_before_build(self):
        (self.root / "crates/cortex-db/migrations/platform/0001_platform.up.sql").unlink()
        result = self.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("podman", " ".join(self.calls()))
        self.assertFalse((self.root / "dist/1.2.3.part").exists())

    def test_plug_migrations_are_not_in_release_migration_set(self):
        plug = self.root / "crates/plug-identity/migrations"
        plug.mkdir(parents=True)
        (plug / "0001_plug.up.sql").write_text("DROP TABLE ignored_plug;\n")
        self.valid_podman()
        result = self.run_release("1.2.3")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        manifest = json.loads((self.root / "dist/1.2.3/manifest.json").read_text())
        migrations = manifest["releases"][0]["cortex"]["migrations"]
        self.assertEqual(migrations, ["platform/0001_platform"])

    def test_contract_words_in_data_literals_are_not_sql_operations(self):
        migration = self.root / "crates/cortex-db/migrations/platform/0001_platform.up.sql"
        migration.write_text("INSERT INTO notes (body) VALUES ('DROP TABLE is just text');\n")
        self.valid_podman()
        result = self.run_release("1.2.3")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))

    def test_contract_sql_inside_quoted_do_stops_before_build(self):
        migration = self.root / "crates/cortex-db/migrations/platform/0001_platform.up.sql"
        migration.write_text("DO 'BEGIN EXECUTE ''DROP TABLE objects''; END';\n")
        self.valid_podman()
        result = self.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertIn("0001_platform.up.sql", result.stderr)
        self.assertIn("DROP TABLE", result.stderr)
        self.assertNotIn("podman", " ".join(self.calls()))
        self.assertFalse((self.root / "dist/1.2.3.part").exists())

    def test_computed_contract_sql_inside_do_or_function_is_refused(self):
        migration = self.root / "crates/cortex-db/migrations/platform/0001_platform.up.sql"
        self.valid_podman()
        for body in [
            "DO $$ BEGIN EXECUTE 'DR' || 'OP TABLE t'; END $$;",
            "CREATE FUNCTION f() RETURNS void AS $$ BEGIN EXECUTE 'DR' || 'OP TABLE t'; END $$ LANGUAGE plpgsql;",
            "DO 'BEGIN EXECUTE ''DR'' || ''OP TABLE t''; END';",
        ]:
            with self.subTest(body=body):
                migration.write_text(body)
                result = self.run_release("1.2.3")
                published = (self.root / "dist/1.2.3").exists()
                shutil.rmtree(self.root / "dist/1.2.3", ignore_errors=True)
                self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))
                self.assertFalse(published)

    def test_do_language_before_quoted_body_cannot_hide_contract_sql(self):
        migration = self.root / "crates/cortex-db/migrations/platform/0001_platform.up.sql"
        self.valid_podman()
        for body in [
            "DO LANGUAGE plpgsql 'BEGIN DROP TABLE objects; END';",
            "DO /* comment */ LANGUAGE \"plpgsql\" 'BEGIN DROP TABLE objects; END';",
            "DO LANGUAGE plpgsql E'BEGIN DROP TABLE objects; END';",
        ]:
            with self.subTest(body=body):
                migration.write_text(body)
                result = self.run_release("1.2.3")
                published = (self.root / "dist/1.2.3").exists()
                shutil.rmtree(self.root / "dist/1.2.3", ignore_errors=True)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("DROP TABLE", result.stderr)
                self.assertFalse(published)

    def test_static_trigger_execute_and_data_text_are_allowed(self):
        migration = self.root / "crates/cortex-db/migrations/platform/0001_platform.up.sql"
        migration.write_text("CREATE TRIGGER t AFTER INSERT ON records FOR EACH ROW EXECUTE FUNCTION f();\nINSERT INTO notes VALUES ('EXECUTE');\n")
        self.valid_podman()
        result = self.run_release("1.2.3")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))

    def test_first_release_rejects_contract_migrations_even_with_marker(self):
        migration = self.root / "crates/cortex-db/migrations/platform/0001_platform.up.sql"
        for sql, operation in [
            ("ALTER TABLE objects DROP COLUMN obsolete;", "DROP COLUMN"),
            ("-- cortex:contract\nALTER TABLE objects DROP COLUMN obsolete;", "DROP COLUMN"),
            ("DROP TABLE objects;", "DROP TABLE"),
            ("ALTER TABLE objects RENAME TO archived;", "RENAME"),
            ("ALTER TABLE objects ALTER COLUMN name SET NOT NULL;", "SET NOT NULL"),
            ("ALTER TABLE objects ALTER COLUMN count TYPE bigint;", "TYPE CHANGE"),
            ("DO $$ BEGIN EXECUTE 'DROP TABLE objects'; END $$;", "DROP TABLE"),
        ]:
            self.log.unlink(missing_ok=True)
            migration.write_text(sql + "\n")
            result = self.run_release("1.2.3")
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("0001_platform.up.sql", result.stderr)
            self.assertIn(operation, result.stderr)
            self.assertNotIn("podman", " ".join(self.calls()))
            self.assertFalse((self.root / "dist/1.2.3.part").exists())

    def valid_podman(self):
        # The podman stub emits an OCI archive whose index names one manifest digest.
        podman = self.root / "bin/podman"
        podman.write_text("""#!/usr/bin/env python3
import hashlib, io, json, os, sys, tarfile
with open(os.environ['RELEASE_TEST_LOG'], 'a') as log:
    log.write('podman ' + sys.argv[1] + '\\n')
if sys.argv[1] == 'build':
    if os.environ.get('RELEASE_TEST_MKDIR_ON_BUILD'):
        os.mkdir(os.environ['RELEASE_TEST_MKDIR_ON_BUILD'])
    with open(os.path.join(os.path.dirname(os.environ['RELEASE_TEST_LOG']), 'podman-build-args.json'), 'w') as args:
        json.dump(sys.argv[2:], args)
    context_feature_file = os.path.join(sys.argv[-1], 'deploy', 'server-features')
    with open(context_feature_file, 'rb') as context_file:
        context_digest = hashlib.sha256(context_file.read()).hexdigest()
    with open(os.path.join(os.path.dirname(os.environ['RELEASE_TEST_LOG']), 'podman-context-feature-sha256'), 'w') as digest_file:
        digest_file.write(context_digest)
if os.environ.get('RELEASE_TEST_FAIL') == 'podman-' + sys.argv[1]:
    sys.exit(42)
if sys.argv[1] == 'save':
    dest = sys.argv[sys.argv.index('--output') + 1]
    config = b'{\"architecture\":\"amd64\",\"os\":\"linux\"}'
    config_digest = 'sha256:' + hashlib.sha256(config).hexdigest()
    manifest = json.dumps({'schemaVersion': 2, 'config': {'digest': config_digest, 'size': len(config)}}).encode()
    digest = 'sha256:' + hashlib.sha256(manifest).hexdigest()
    mode = os.environ.get('RELEASE_TEST_OCI', 'good')
    descriptor = {'digest': 'sha512:' + 'a' * 64 if mode == 'algorithm' else digest, 'size': len(manifest)}
    manifests = [descriptor, descriptor] if mode == 'multiple' else [descriptor]
    index = json.dumps({'schemaVersion': 2, 'manifests': manifests}).encode()
    entries = [('index.json', index), ('blobs/sha256/' + digest[7:], manifest), ('blobs/sha256/' + config_digest[7:], config)]
    if mode == 'missing':
        entries = entries[1:]
    if mode == 'duplicate':
        entries.append(('index.json', index))
    with tarfile.open(dest, 'w') as tar:
        for name, data in entries:
            info = tarfile.TarInfo(name)
            info.size = len(data)
            tar.addfile(info, io.BytesIO(data))
""")
        podman.chmod(0o755)

    def test_release_image_uses_exactly_declared_features(self):
        expected = {"mod-policy", "mod-records", "mod-search", "mod-handoffs", "mod-boot"}
        source = self.root / "deploy/server-features"
        original_sha = hashlib.sha256(source.read_bytes()).hexdigest()
        declared = source.read_text().strip().split(",")
        self.assertEqual(set(declared), expected)
        self.assertEqual(len(declared), len(expected))
        self.valid_podman()
        result = self.run_release("1.2.3")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        args = json.loads((self.root / "podman-build-args.json").read_text())
        self.assertFalse(any("feature" in arg.lower() for arg in args), args)
        self.assertEqual(Path(args[-1]), self.root)
        self.assertEqual((self.root / "podman-context-feature-sha256").read_text(), original_sha)

    def test_release_refuses_missing_or_invalid_feature_file(self):
        features = self.root / "deploy/server-features"
        original = features.read_text()
        names = original.strip().split(",")
        variants = {"missing-file": None, "no-final-lf": original.rstrip("\n"),
                    "blank": "\n", "two-lines": original + "mod-policy\n", "duplicate": original.strip() + ",mod-policy\n",
                    "extra-module": original.strip() + ",mod-extra\n", "test-faults": original.strip() + ",test-faults\n"}
        variants.update({"missing-" + name: ",".join(n for n in names if n != name) + "\n" for name in names})
        self.valid_podman()
        for label, value in variants.items():
            with self.subTest(case=label):
                features.unlink(missing_ok=True)
                if value is not None: features.write_text(value)
                self.log.unlink(missing_ok=True)
                result = self.run_release("1.2.3")
                published = (self.root / "dist/1.2.3").exists()
                shutil.rmtree(self.root / "dist/1.2.3", ignore_errors=True)
                self.assertNotEqual(result.returncode, 0, (label, result.stdout, result.stderr))
                self.assertNotIn("podman build", self.calls())
                self.assertFalse(published)

    def test_release_rejects_test_faults_feature(self):
        features = self.root / "deploy/server-features"
        features.write_text(features.read_text().strip() + ",test-faults\n")
        self.valid_podman()
        result = self.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("podman build", self.calls())
        self.assertFalse((self.root / "dist/1.2.3").exists())

    def test_release_refuses_feature_file_changed_after_validation(self):
        self.valid_podman()
        ci = self.root / "bin/ci"
        ci.write_text('''#!/bin/sh
printf '%s\\n' ci >> "$RELEASE_TEST_LOG"
printf '%s\\n' 'mod-boot,mod-handoffs,mod-search,mod-records,mod-policy' > "$(dirname "$RELEASE_TEST_LOG")/deploy/server-features"
printf 'fixture PASS\\nci: PASS\\n'
''')
        ci.chmod(0o755)
        result = self.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("server feature file changed before image build", result.stderr)
        self.assertNotIn("podman build", self.calls())

    def test_real_cli_build_receives_the_release_git_sha(self):
        self.valid_podman()
        self.env.pop("CORTEX_RELEASE_CLI")
        self.env.pop("CORTEX_GIT_SHA", None)
        cargo = self.root / "bin/cargo"
        cargo.write_text("""#!/bin/sh
set -eu
if [ "$1" = metadata ]; then cat "$(dirname "$RELEASE_TEST_LOG")/metadata.json"; exit 0; fi
printf '%s\\n' "${CORTEX_GIT_SHA-}" > "$(dirname "$RELEASE_TEST_LOG")/cli-build-sha"
mkdir -p target/release
printf '#!/bin/sh\\nexit 0\\n' > target/release/cortex
chmod 755 target/release/cortex
""")
        cargo.chmod(0o755)
        result = self.run_release("1.2.3")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertEqual((self.root / "cli-build-sha").read_text().strip(), "a" * 40)

    def test_complete_release_has_reproducible_verified_assets(self):
        self.valid_podman()
        result = self.run_release("1.2.3")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        dist = self.root / "dist/1.2.3"
        self.assertFalse((self.root / "dist/1.2.3.part").exists())
        archive_name = "cortex-1.2.3-x86_64-linux.tar.gz"
        image_name = "cortex-server-1.2.3-x86_64.oci.tar"
        self.assertEqual({p.name for p in dist.iterdir()}, {archive_name, image_name, "install.sh", "manifest.json", "install.meta", "SHA256SUMS"})
        with tarfile.open(dist / archive_name, "r:gz") as tar:
            entries = {m.name.rstrip('/'): m for m in tar.getmembers()}
            self.assertEqual(set(entries), {"cortex", "compose.yml", "stack.env", "db-roles.sh", "initdb", "initdb/roles.sql", "cortex.toml.example", "bin", "systemd", *("bin/" + n for n in BIN_MEMBERS), *("systemd/" + n for n in SYSTEMD_MEMBERS)})
            for name, member in entries.items():
                self.assertEqual(member.mode, 0o755 if name in ("cortex", "db-roles.sh", "initdb", "bin", "systemd") or name.startswith("bin/") else 0o644)
                self.assertEqual((member.uid, member.gid, member.mtime), (0, 0, 1700000000))
            self.assertEqual(tar.extractfile("db-roles.sh").read(), (self.root / "deploy/db-roles.sh").read_bytes())
            env = tar.extractfile("stack.env").read().decode().splitlines()
            self.assertEqual({line.split('=', 1)[0] for line in env}, {"CORTEX_VERSION", "CORTEX_SERVER_IMAGE", "CORTEX_DB_IMAGE"})
            self.assertEqual(len(env), 3)
        manifest = json.loads((dist / "manifest.json").read_text())
        release = manifest["releases"][0]
        self.assertEqual(release["version"], "1.2.3")
        self.assertEqual(release["cortex"].get("members"), 2)
        for asset in release["assets"]:
            self.assertEqual(asset["digest"], "sha256:" + hashlib.sha256((dist / asset["name"]).read_bytes()).hexdigest())
        self.assertEqual(release["cortex"]["roles_script_digest"], "sha256:" + hashlib.sha256((self.root / "deploy/db-roles.sh").read_bytes()).hexdigest())
        with tarfile.open(dist / image_name) as tar:
            digest = json.load(tar.extractfile("index.json"))["manifests"][0]["digest"]
        self.assertEqual(release["cortex"]["server_image"]["ref"], "localhost/cortex-server@" + digest)
        verified = subprocess.run(["sha256sum", "-c", "SHA256SUMS"], cwd=dist, capture_output=True, text=True)
        self.assertEqual(verified.returncode, 0, verified.stderr)

    def test_malformed_oci_index_refuses_and_cleans_staging(self):
        self.valid_podman()
        for mode in ["missing", "multiple", "algorithm", "duplicate"]:
            result = self.run_release("1.2.3", RELEASE_TEST_OCI=mode)
            self.assertNotEqual(result.returncode, 0, mode)
            self.assertFalse((self.root / "dist/1.2.3").exists())
            self.assertFalse((self.root / "dist/1.2.3.part").exists())

    def test_initdb_names_allow_underscore_and_hyphen_prefixes(self):
        self.valid_podman()
        for name in ["_first.sql", "-second.sql"]:
            (self.root / "deploy/initdb" / name).write_text("SELECT 1;\n")
        result = self.run_release("1.2.3")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        with tarfile.open(self.root / "dist/1.2.3/cortex-1.2.3-x86_64-linux.tar.gz", "r:gz") as tar:
            self.assertIn("initdb/_first.sql", tar.getnames())
            self.assertIn("initdb/-second.sql", tar.getnames())

    def test_parameterised_database_service_survives_release_build(self):
        self.valid_podman()
        compose = self.root / "deploy/compose.yml"
        original = compose.read_text()
        cases = [
            "    environment:\n      EXAMPLE: changed\n",
            "    healthcheck:\n      interval: 30s\n",
            "    volumes:\n      - db-data:/var/lib/db\n",
            "    secrets:\n      - db-password\n",
            "    image: registry.invalid/other@sha256:" + "b" * 64 + "\n",
        ]
        old_image = "    image: registry.invalid/db@sha256:" + "a" * 64 + "\n"
        for index, change in enumerate(cases):
            text = original.replace(old_image, change if index == 4 else old_image + change)
            compose.write_text(text)
            version = f"1.2.{index + 10}"
            result = self.run_release(version)
            self.assertEqual(result.returncode, 0, (index, result.stdout, result.stderr))
            with tarfile.open(self.root / f"dist/{version}/cortex-{version}-x86_64-linux.tar.gz", "r:gz") as tar:
                built = tar.extractfile("compose.yml").read().decode()
            source_db = text.split("  db:\n", 1)[1].split("  migrate:\n", 1)[0]
            built_db = built.split("  db:\n", 1)[1].split("  migrate:\n", 1)[0]
            self.assertEqual(built_db, source_db, index)

    def test_build_failures_remove_staging(self):
        self.valid_podman()
        for overrides in [{"RELEASE_TEST_FAIL": "podman-build"}, {"RELEASE_TEST_FAIL": "podman-save"},
                          {"CORTEX_RELEASE_CLI": str(self.root / "missing-cli")},
                          {"CORTEX_RELEASE_GIT_SHA": "not-a-sha"}, {"SOURCE_DATE_EPOCH": "bad"}]:
            result = self.run_release("1.2.3", **overrides)
            self.assertNotEqual(result.returncode, 0, (overrides, result.stdout, result.stderr))
            self.assertFalse((self.root / "dist/1.2.3").exists())
            self.assertFalse((self.root / "dist/1.2.3.part").exists())
        (self.root / "deploy/initdb/.hidden").write_text("bad")
        result = self.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse((self.root / "dist/1.2.3").exists())
        self.assertFalse((self.root / "dist/1.2.3.part").exists())

    def test_notes_default_points_to_public_release(self):
        self.valid_podman()
        self.assertEqual(self.run_release("1.2.3").returncode, 0)
        entry = json.loads((self.root / "dist/1.2.3/manifest.json").read_text())["releases"][0]
        self.assertEqual(entry["notes_url"], "https://github.com/amad-malik/ai-cortex/releases/tag/v1.2.3")

    def test_notes_and_label_are_written_verbatim(self):
        self.valid_podman()
        self.assertEqual(self.run_release("1.2.3").returncode, 0)
        default = json.loads((self.root / "dist/1.2.3/manifest.json").read_text())["releases"][0]
        self.assertEqual(default["notes_url"], "https://github.com/amad-malik/ai-cortex/releases/tag/v1.2.3")
        self.assertEqual(default["cortex"]["label"], "1.2.3")
        result = self.run_release("1.2.4", "--label", "0.2.001", "--notes-url", "https://example.invalid/r/notes")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        custom = json.loads((self.root / "dist/1.2.4/manifest.json").read_text())["releases"][0]
        self.assertEqual(custom["notes_url"], "https://example.invalid/r/notes")
        self.assertEqual(custom["cortex"]["label"], "0.2.001")

    def test_default_proof_gate_reads_ci_status_and_rejects_zero_or_stale(self):
        self.env.pop("CORTEX_RELEASE_PROOF")
        ci = self.root / "bin/ci"
        prefix = "roles-proof      NOT_RUN  needs a database: run by hand. "
        for verdict, reaches_squawk in [
            ("last live run matches the current scripts (roles-proof: 90 passed, 0 failed)", True),
            ("last live run matches the current scripts (roles-proof: 0 passed, 0 failed)", False),
            ("STALE: scripts changed", False), ("last live run FAILED", False),
        ]:
            (self.root / "scripts/release-ci-allowlist.json").write_text(json.dumps([{"gate": "roles-proof", "reason": "needs a database: run by hand. " + verdict, "accepted_by": "fixture", "close_release": "9.9.9"}]))
            ci.write_text('#!/bin/sh\nprintf "%s\\n" ci >> "$RELEASE_TEST_LOG"\nprintf "%s\\n" ' +
                          repr(prefix + verdict) + '\nprintf "%s\\n" "ci: PASS-WITH-NOT-RUN (1 gates did not run; see NOT_RUN above)"\n')
            ci.chmod(0o755)
            self.log.unlink(missing_ok=True)
            self.run_release("1.2.3")
            self.assertEqual("squawk" in self.calls(), reaches_squawk, verdict)
            if not reaches_squawk:
                self.assertEqual(self.calls(), ["ci", "s5"])

    def test_stale_failed_and_missing_proof_stop_before_build(self):
        for status in ["STALE", "FAILED", "no live result recorded", "STALE last live run matches the current scripts"]:
            self.log.unlink(missing_ok=True)
            result = self.run_release("1.2.3", RELEASE_TEST_PROOF=status)
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(self.calls(), ["ci", "s5", "proof"])
            self.assertFalse((self.root / "dist/1.2.3.part").exists())

    # ---- the release-fixes series: hale's RB-1 .. RB-4 and X-1 (scripts/release.sh) ----

    def git(self, *args, cwd=None):
        env = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
        env.update({"GIT_CONFIG_GLOBAL": "/dev/null", "GIT_CONFIG_NOSYSTEM": "1"})
        return subprocess.run(["git", "-c", "user.name=fixture", "-c", "user.email=fixture@invalid", "-c", "commit.gpgsign=false", *args],
                              cwd=cwd or self.root, env=env, text=True, capture_output=True, check=True).stdout.strip()

    def git_root(self):
        """Make the synthetic root a real git work tree with everything committed and the SHA seam off; return HEAD."""
        (self.root / ".gitignore").write_text("calls\npodman-build-args.json\npodman-context-feature-sha256\ncli-build-sha\ncanary-log\nlate-edit-backup\n*.pyc\nkeys/\nscratch-parent/\nsignal-parent/\nkey-copy\nci-started\n")
        # the anchor join: the installer template carries the three empty statements and the release is given a key (test keys only)
        (self.root / "deploy/install.sh").write_text(self.ANCHOR_HEADER + self.ANCHOR_LINES + self.ANCHOR_BODY)
        self.anchor_key = self.make_key("release")[0]
        self.git("init", "-q")
        self.git("add", "-A")
        self.git("commit", "-q", "-m", "fixture")
        self.env.pop("CORTEX_RELEASE_GIT_SHA")
        return self.git("rev-parse", "HEAD")

    def test_rb1_an_untracked_file_in_deploy_refuses_a_release_under_a_clean_sha(self):
        self.valid_podman()
        self.git_root()
        (self.root / "deploy/initdb/zz_canary.sql").write_text("SELECT 'canary';\n")
        result = self.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertIn("work tree is not clean", result.stderr)
        self.assertEqual(self.calls(), [])
        self.assertFalse((self.root / "dist/1.2.3").exists())
        self.assertFalse((self.root / "dist/1.2.3.part").exists())

    def test_rb1_an_uncommitted_edit_of_a_tracked_file_refuses_a_release(self):
        self.valid_podman()
        self.git_root()
        (self.root / "deploy/db-roles.sh").write_text("#!/bin/sh\nexit 1\n")
        result = self.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertIn("work tree is not clean", result.stderr)
        self.assertEqual(self.calls(), [])
        self.assertFalse((self.root / "dist/1.2.3").exists())

    def test_rb1_a_root_inside_another_work_tree_is_refused(self):
        self.valid_podman()
        outer = Path(tempfile.mkdtemp(prefix="release-outer-", dir=os.environ["TMPDIR"]))
        self.addCleanup(shutil.rmtree, outer, True)
        self.git("init", "-q", cwd=outer)
        (outer / "README").write_text("outer\n")
        self.git("add", "-A", cwd=outer)
        self.git("commit", "-q", "-m", "outer", cwd=outer)
        old, new = str(self.root), outer / "extracted"
        shutil.move(old, new)
        self.root, self.log = new, new / "calls"
        self.env = {k: v.replace(old, str(new)) for k, v in self.env.items()}
        self.metadata_fixture("1.2.3")
        self.env.pop("CORTEX_RELEASE_GIT_SHA")
        # a real release needs the key option; this one reaches the work-tree check with a key given (the key file lives outside the moved root)
        key_file = Path(tempfile.mkdtemp(prefix="release-key-", dir=os.environ["TMPDIR"])) / "key.txt"
        self.addCleanup(shutil.rmtree, key_file.parent, True)
        key_file.write_text("release-test@fixture ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI" + "A" * 43 + "\n")
        result = self.run_release("1.2.3", "--trust-key-file", str(key_file))
        self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertIn("not the top of its own git work tree", result.stderr)
        self.assertEqual(self.calls(), [])
        self.assertFalse((new / "dist/1.2.3").exists())

    def test_rb1_a_clean_git_tree_builds_and_labels_the_release_with_head(self):
        self.valid_podman()
        head = self.git_root()
        result = self.run_release("1.2.3")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        args = json.loads((self.root / "podman-build-args.json").read_text())
        self.assertIn("CORTEX_GIT_SHA=" + head, args)

    def test_rb1_target_dist_and_the_one_squawk_binary_do_not_dirty_the_tree(self):
        self.valid_podman()
        self.git_root()
        (self.root / "target").mkdir(exist_ok=True)
        (self.root / "target/scratch.txt").write_text("x\n")
        (self.root / "dist/0.9.9").mkdir(parents=True)
        (self.root / "dist/0.9.9/install.sh").write_text("old\n")
        (self.root / "tools/squawk/2.71.0").mkdir(parents=True)
        (self.root / "tools/squawk/2.71.0/squawk-linux-musl-x64").write_text("binary\n")
        result = self.run_release("1.2.3")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))

    def late_edit(self, relpath, edited, restore):
        """RB-2: the squawk gate (after S5 read its copy) rewrites a LIVE file; with restore, podman's save step puts the original back."""
        live = self.root / relpath
        backup = self.root / "late-edit-backup"
        shutil.copy2(live, backup)
        squawk = self.root / "bin/squawk"
        squawk.write_text(f"#!/bin/sh\nprintf '%s\\n' squawk >> \"{self.log}\"\ncat > \"{live}\" <<'LATE_EDIT_END'\n{edited}LATE_EDIT_END\n")
        squawk.chmod(0o755)
        if restore:
            podman = self.root / "bin/podman"
            podman.rename(self.root / "bin/podman-real")
            podman.write_text(f'#!/bin/sh\nif [ "$1" = save ]; then cp -p "{backup}" "{live}"; fi\nexec "{self.root}/bin/podman-real" "$@"\n')
            podman.chmod(0o755)

    def test_rb2_the_shipped_installer_is_the_copy_s5_read_not_the_live_file(self):
        self.valid_podman()
        live = self.root / "deploy/install-prereq.sh"
        self.late_edit("deploy/install-prereq.sh", live.read_text() + "canary_late_edit() { :; }\n", restore=True)
        result = self.run_release("1.2.3")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertNotIn("canary_late_edit", (self.root / "dist/1.2.3/install.sh").read_text())

    def test_rb2_every_input_of_the_join_db_image_archive_and_manifest_steps_is_a_snapshot_in_the_work_directory(self):
        self.valid_podman()
        helper = self.root / "scripts/release_manifest.py"
        real = self.root / "scripts/release_manifest_real.py"
        helper.rename(real)
        helper.write_text("import json, os, runpy, sys\n"
                          "with open(os.environ['RELEASE_TEST_LOG'] + '.helper', 'a') as f:\n"
                          "    f.write(json.dumps(sys.argv[1:]) + '\\n')\n"
                          f"sys.argv[0] = {str(real)!r}\n"
                          f"runpy.run_path({str(real)!r}, run_name='__main__')\n")
        result = self.run_release("1.2.3")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        by = {c[0]: c for c in (json.loads(l) for l in Path(str(self.log) + ".helper").read_text().splitlines())}
        def flag(command, name):
            return by[command][by[command].index(name) + 1]
        inputs = {"join-installer --deploy": flag("join-installer", "--deploy"), "db-image path": by["db-image"][1],
                  "archive --layout": flag("archive", "--layout"), "archive --scripts": flag("archive", "--scripts"),
                  "manifest --layout": flag("manifest", "--layout"), "manifest --toml": flag("manifest", "--toml")}
        for what, value in inputs.items():
            with self.subTest(input=what):
                self.assertIn("release-work.", value)
                self.assertFalse(value.startswith(str(self.root / "deploy")) or value.startswith(str(self.root / "scripts")), value)

    def test_rb2_a_deploy_file_changed_during_the_release_refuses_it(self):
        self.valid_podman()
        live = self.root / "deploy/install-prereq.sh"
        self.late_edit("deploy/install-prereq.sh", live.read_text() + "canary_late_edit() { :; }\n", restore=False)
        result = self.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertIn("changed during the release", result.stderr)
        self.assertFalse((self.root / "dist/1.2.3").exists())
        self.assertFalse((self.root / "dist/1.2.3.part").exists())

    def test_rb2_a_bin_script_changed_during_the_release_refuses_it(self):
        self.valid_podman()
        live = self.root / "scripts/cortex-restore"
        self.late_edit("scripts/cortex-restore", live.read_text() + "# late edit\n", restore=False)
        result = self.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertIn("changed during the release", result.stderr)
        self.assertFalse((self.root / "dist/1.2.3").exists())
        self.assertFalse((self.root / "dist/1.2.3.part").exists())

    def test_rb3_each_seam_without_the_fixture_marker_refuses_before_any_gate(self):
        for name in self.SEAMS:
            with self.subTest(seam=name):
                self.log.unlink(missing_ok=True)
                for other in self.SEAMS:
                    self.env.pop(other, None)
                self.env.pop("CORTEX_RELEASE_FIXTURE_ROOT", None)
                result = self.run_release("1.2.3", **{name: str(self.root / "bin/ci")})
                self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))
                self.assertIn("outside a fixture", result.stderr)
                self.assertIn(name, result.stderr)
                self.assertEqual(self.calls(), [])
                self.assertFalse((self.root / "dist/1.2.3").exists())
                self.assertFalse((self.root / "dist/1.2.3.part").exists())

    def test_rb3_a_marker_naming_another_root_does_not_unlock_the_seams(self):
        self.valid_podman()
        self.env["CORTEX_RELEASE_FIXTURE_ROOT"] = str(self.root.parent)
        result = self.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertIn("outside a fixture", result.stderr)
        self.assertIn("CORTEX_RELEASE_CI", result.stderr)
        self.assertEqual(self.calls(), [])
        self.assertFalse((self.root / "dist/1.2.3").exists())

    def test_rb3_a_seam_release_names_its_seams_on_stderr_and_in_install_meta(self):
        self.valid_podman()
        result = self.run_release("1.2.3")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        active = [name for name in self.SEAMS if self.env.get(name)]
        self.assertEqual(len(active), 6)
        self.assertIn("TEST SEAMS ACTIVE", result.stderr)
        for name in active:
            self.assertIn(name, result.stderr)
        meta = (self.root / "dist/1.2.3/install.meta").read_text()
        self.assertIn("RELEASE_SEAMS=" + ",".join(active) + "\n", meta)

    def test_rb3_the_manifest_helper_writes_the_seams_line_only_when_given(self):
        out = self.root / "meta-out"
        out.mkdir()
        (out / "cortex-1.2.3-x86_64-linux.tar.gz").write_bytes(b"archive")
        (out / "cortex-server-1.2.3-x86_64.oci.tar").write_bytes(b"image")
        layout = self.root / "deploy"
        args = [sys.executable, "-I", str(self.root / "scripts/release_manifest.py"), "manifest", "--output-dir", str(out),
                "--layout", str(layout), "--version", "1.2.3", "--server-image", "localhost/cortex-server@sha256:" + "b" * 64,
                "--db-image", "registry.invalid/db@sha256:" + "a" * 64, "--label", "1.2.3",
                "--notes-url", "https://releases.cortex.invalid/", "--toml", str(layout / "cortex.toml.example"),
                "--crates", str(self.root / "crates"), "--epoch", "1700000000"]
        plain = subprocess.run(args, text=True, capture_output=True)
        self.assertEqual(plain.returncode, 0, plain.stderr)
        self.assertNotIn("RELEASE_SEAMS", (out / "install.meta").read_text())
        given = subprocess.run([*args, "--seams", "CORTEX_RELEASE_CI,CORTEX_RELEASE_S5"], text=True, capture_output=True)
        self.assertEqual(given.returncode, 0, given.stderr)
        self.assertTrue((out / "install.meta").read_text().endswith("RELEASE_SEAMS=CORTEX_RELEASE_CI,CORTEX_RELEASE_S5\n"))
        bad = subprocess.run([*args, "--seams", "not a seam name"], text=True, capture_output=True)
        self.assertNotEqual(bad.returncode, 0)

    def canary(self, name, directory=None):
        """An executable named NAME that records that it ran and then runs the real tool."""
        path = (directory or self.root / "bin") / name
        path.write_text(f'#!/bin/sh\nprintf "%s\\n" {name} >> "{self.root}/canary-log"\nexec /usr/bin/{name} "$@"\n')
        path.chmod(0o755)

    def test_rb4_an_empty_path_element_is_refused_before_any_tree_file_runs(self):
        self.valid_podman()
        self.canary("sha256sum", self.root)
        self.canary("dirname", self.root)
        result = self.run_release("1.2.3", PATH=str(self.root / "bin") + "::" + os.environ["PATH"])
        self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertIn("PATH", result.stderr)
        self.assertFalse((self.root / "canary-log").exists())
        self.assertEqual(self.calls(), [])

    def test_rb4_a_relative_path_element_is_refused_before_any_tree_file_runs(self):
        self.valid_podman()
        self.canary("sha256sum", self.root)
        result = self.run_release("1.2.3", PATH=str(self.root / "bin") + ":." + os.pathsep + os.environ["PATH"])
        self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertIn("PATH", result.stderr)
        self.assertFalse((self.root / "canary-log").exists())
        self.assertEqual(self.calls(), [])

    def test_rb4_helper_tools_are_not_looked_up_on_the_callers_path(self):
        self.valid_podman()
        helpers = ["dirname", "mkdir", "cp", "mv", "rm", "mktemp", "date", "sed", "sha256sum", "gzip", "install", "python3", "find", "sort", "cut"]
        for name in helpers:
            self.canary(name)
        result = self.run_release("1.2.3")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        ran = (self.root / "canary-log").read_text().split() if (self.root / "canary-log").exists() else []
        self.assertEqual(sorted(set(ran)), [], "helper tools found on the caller's PATH instead of /usr/bin:/bin")

    def test_rb4_the_three_gates_keep_the_callers_path(self):
        # ci.sh needs cargo and its subcommands from the caller's PATH: only the helper tools are pinned to /usr/bin:/bin.
        self.valid_podman()
        tool = self.root / "bin/gate-only-tool"
        tool.write_text("#!/bin/sh\nexit 0\n")
        tool.chmod(0o755)
        for name in ("ci", "s5", "proof"):
            script = self.root / "bin" / name
            script.write_text("#!/bin/sh\ncommand -v gate-only-tool >/dev/null || exit 1\n" + script.read_text().split("\n", 1)[1])
        result = self.run_release("1.2.3")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertEqual(self.calls()[:3], ["ci", "s5", "proof"])

    # ---- hale's RB-12 (the windows of the clean-tree check) and RB-11 (exported shell functions) ----

    def test_rb12_an_edit_during_the_ci_gate_refuses_the_release(self):
        self.valid_podman()
        ci = self.root / "bin/ci"
        ci.write_text(f'#!/bin/sh\nprintf "%s\\n" ci >> "$RELEASE_TEST_LOG"\nprintf "# edited during ci\\n" >> "{self.root}/deploy/db-roles.sh"\nprintf "fixture PASS\\nci: PASS\\n"\n')
        ci.chmod(0o755)
        self.git_root()
        result = self.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertIn("work tree is not clean", result.stderr)
        self.assertEqual(self.calls(), ["ci"])
        self.assertFalse((self.root / "dist/1.2.3").exists())
        self.assertFalse((self.root / "dist/1.2.3.part").exists())

    def test_rb12_an_edit_during_the_image_build_refuses_the_release(self):
        self.valid_podman()
        podman = self.root / "bin/podman"
        podman.rename(self.root / "bin/podman-real")
        podman.write_text(f'#!/bin/sh\nif [ "$1" = build ]; then\n'
                          f'  printf "SELECT 2;\\n" >> "{self.root}/crates/cortex-db/migrations/platform/0001_platform.up.sql"\n'
                          f'  printf "fn x() {{}}\\n" > "{self.root}/crates/cortex-server/canary.rs"\nfi\n'
                          f'exec "{self.root}/bin/podman-real" "$@"\n')
        podman.chmod(0o755)
        self.git_root()
        result = self.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertIn("work tree is not clean", result.stderr)
        self.assertFalse((self.root / "dist/1.2.3").exists())
        self.assertFalse((self.root / "dist/1.2.3.part").exists())

    def test_rb12_an_ignored_file_under_deploy_refuses_the_release(self):
        self.valid_podman()
        self.git_root()
        (self.root / "deploy/initdb/stale.pyc").write_bytes(b"x")
        result = self.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertIn("ignored file under deploy/", result.stderr)
        self.assertEqual(self.calls(), [])
        self.assertFalse((self.root / "dist/1.2.3").exists())

    def test_rb12_a_head_that_moves_during_the_release_refuses_it(self):
        self.valid_podman()
        ci = self.root / "bin/ci"
        ci.write_text(f'#!/bin/sh\nprintf "%s\\n" ci >> "$RELEASE_TEST_LOG"\ncd "{self.root}" && git -c core.hooksPath=/dev/null -c user.name=x -c user.email=x@invalid '
                      f'-c commit.gpgsign=false commit -q --allow-empty -m moved\nprintf "fixture PASS\\nci: PASS\\n"\n')
        ci.chmod(0o755)
        self.git_root()
        result = self.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertIn("HEAD moved", result.stderr)
        self.assertFalse((self.root / "dist/1.2.3").exists())
        self.assertFalse((self.root / "dist/1.2.3.part").exists())

    def test_rb11_exported_shell_functions_cannot_replace_the_guards(self):
        # bash (sh here) imports exported functions from the environment; one named [ or compgen or echo must not defeat the seam guard
        self.env.pop("CORTEX_RELEASE_FIXTURE_ROOT")
        for functions in ({"[": "() { return 0; }"}, {"[": "() { return 0; }", "compgen": "() { return 0; }"}, {"echo": "() { return 0; }"}):
            with self.subTest(functions=sorted(functions)):
                self.log.unlink(missing_ok=True)
                result = self.run_release("1.2.3", **{f"BASH_FUNC_{name}%%": body for name, body in functions.items()})
                self.assertEqual(result.returncode, 2, (result.stdout, result.stderr))
                self.assertIn("outside a fixture", result.stderr)
                self.assertEqual(self.calls(), [])
                self.assertFalse((self.root / "dist/1.2.3").exists())

    # ---- the G12 binding-record emitter (cox cfe3df64; nix's schema r3 G12-BINDING-RECORD-SCHEMA-nix-r3.md) ----

    SPLICE = "each '# @include' marker line, newline included, is replaced by the exact bytes of its include; nothing is added"
    RECORD_KEYS = ["schema", "object_format", "commit", "tree", "includes", "splice", "joined_sha256"]
    MARKER = re.compile(r'"\$here/([A-Za-z0-9_.-]+)"   # @include$')

    def join_helper(self, deploy, out, record=None, commit=None, tree=None, fmt=None):
        args = [sys.executable, "-I", str(self.root / "scripts/release_manifest.py"), "join-installer", "--deploy", str(deploy), "--output", str(out)]
        if record is not None:
            args += ["--record", str(record)]
        if commit is not None:
            args += ["--commit", commit]
        if tree is not None:
            args += ["--tree", tree]
        if fmt is not None:
            args += ["--object-format", fmt]
        return subprocess.run(args, text=True, capture_output=True)

    def strict_record(self, record, deploy, shipped, *, fmt, commit, tree):
        """The record as nix's runner will read it (a validator of my own: the runner's strictness is nix's): the exact key set and order, duplicate keys
        rejected, schema an int and not a bool, hex of the declared width, the include order of the marker lines, every hash, the splice sentence, and the
        rebuilt text equal to the shipped bytes by the second method (each marker line replaced by the include's raw bytes)."""
        def no_duplicates(pairs):
            seen = {}
            for key, value in pairs:
                self.assertNotIn(key, seen, "duplicate key " + key)
                seen[key] = value
            return seen
        raw = Path(record).read_bytes()
        self.assertTrue(raw.endswith(b"\n"))
        rec = json.loads(raw.decode("ascii"), object_pairs_hook=no_duplicates)
        self.assertEqual(list(rec), self.RECORD_KEYS)
        self.assertIs(type(rec["schema"]), int)
        self.assertEqual(rec["schema"], 1)
        self.assertEqual(rec["object_format"], fmt)
        width = {"sha1": 40, "sha256": 64}[fmt]
        self.assertEqual((rec["commit"], rec["tree"]), (commit, tree))
        for key in ("commit", "tree"):
            self.assertIsNotNone(re.fullmatch("[0-9a-f]{%d}" % width, rec[key]), key)
        template = (Path(deploy) / "install.sh").read_bytes()
        order = [m.group(1) for line in template.decode().splitlines() for m in [self.MARKER.search(line)] if m]
        self.assertEqual([i["name"] for i in rec["includes"]], order)
        self.assertEqual(len(set(order)), len(order))
        for entry in rec["includes"]:
            self.assertEqual(list(entry), ["name", "sha256"])
            self.assertIsNotNone(re.fullmatch("[0-9a-f]{64}", entry["sha256"]))
            self.assertEqual(entry["sha256"], hashlib.sha256((Path(deploy) / entry["name"]).read_bytes()).hexdigest())
        self.assertEqual(rec["splice"], self.SPLICE)
        shipped_bytes = Path(shipped).read_bytes()
        self.assertEqual(rec["joined_sha256"], hashlib.sha256(shipped_bytes).hexdigest())
        rebuilt = b"".join((Path(deploy) / self.MARKER.search(line.decode().rstrip("\n")).group(1)).read_bytes() if self.MARKER.search(line.decode().rstrip("\n")) else line
                           for line in template.splitlines(keepends=True))
        self.assertEqual(rebuilt, shipped_bytes)
        return rec

    def test_g12_the_join_helper_writes_a_record_a_strict_validator_accepts(self):
        out, record = self.root / "joined.sh", self.root / "joined.sh.binding.json"
        result = self.join_helper(self.root / "deploy", out, record, "a" * 40, "b" * 40, "sha1")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.strict_record(record, self.root / "deploy", out, fmt="sha1", commit="a" * 40, tree="b" * 40)

    def test_g12_the_record_is_the_same_bytes_on_a_second_run(self):
        first = self.join_helper(self.root / "deploy", self.root / "j1.sh", self.root / "j1.json", "a" * 40, "b" * 40, "sha1")
        second = self.join_helper(self.root / "deploy", self.root / "j2.sh", self.root / "j2.json", "a" * 40, "b" * 40, "sha1")
        self.assertEqual((first.returncode, second.returncode), (0, 0), (first.stderr, second.stderr))
        self.assertEqual((self.root / "j1.json").read_bytes(), (self.root / "j2.json").read_bytes())

    def test_g12_the_object_format_decides_the_width_of_the_ids(self):
        ok = self.join_helper(self.root / "deploy", self.root / "ok.sh", self.root / "ok.json", "a" * 64, "b" * 64, "sha256")
        self.assertEqual(ok.returncode, 0, ok.stderr)
        self.strict_record(self.root / "ok.json", self.root / "deploy", self.root / "ok.sh", fmt="sha256", commit="a" * 64, tree="b" * 64)
        for fmt, commit, tree in [("sha1", "a" * 64, "b" * 64), ("sha256", "a" * 40, "b" * 40), ("sha1", "A" * 40, "b" * 40), ("sha1", "a" * 40, "b" * 39),
                                  ("sha1", "g" * 40, "b" * 40), ("sha1", " " + "a" * 39, "b" * 40), ("md5", "a" * 32, "b" * 32)]:
            with self.subTest(fmt=fmt, commit=commit[:6], tree=tree[:6]):
                out, record = self.root / "bad.sh", self.root / "bad.json"
                out.unlink(missing_ok=True)
                record.unlink(missing_ok=True)
                result = self.join_helper(self.root / "deploy", out, record, commit, tree, fmt)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(out.exists() or record.exists())

    def test_g12_a_record_needs_all_three_arguments_and_writes_nothing_without_them(self):
        control = self.join_helper(self.root / "deploy", self.root / "c.sh", self.root / "c.json", "a" * 40, "b" * 40, "sha1")
        self.assertEqual(control.returncode, 0, control.stderr)
        for given in [("a" * 40, "b" * 40, None), ("a" * 40, None, "sha1"), (None, "b" * 40, "sha1"), (None, None, None)]:
            with self.subTest(given=[g is not None for g in given]):
                out, record = self.root / "n.sh", self.root / "n.json"
                out.unlink(missing_ok=True)
                record.unlink(missing_ok=True)
                result = self.join_helper(self.root / "deploy", out, record, *given)
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(out.exists() or record.exists())
        with self.subTest(given="the three ids without --record"):
            out = self.root / "o.sh"
            out.unlink(missing_ok=True)
            result = self.join_helper(self.root / "deploy", out, None, "a" * 40, "b" * 40, "sha1")
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(out.exists())

    def test_g12_the_include_order_is_the_order_of_the_marker_lines(self):
        swapped = self.root / "deploy-swapped"
        shutil.copytree(self.root / "deploy", swapped)
        (swapped / "install.sh").write_text('#!/bin/sh\n. "$here/install-fetch.sh"   # @include\n. "$here/install-prereq.sh"   # @include\n')
        out, record = self.root / "s.sh", self.root / "s.json"
        result = self.join_helper(swapped, out, record, "a" * 40, "b" * 40, "sha1")
        self.assertEqual(result.returncode, 0, result.stderr)
        rec = self.strict_record(record, swapped, out, fmt="sha1", commit="a" * 40, tree="b" * 40)
        self.assertEqual([i["name"] for i in rec["includes"]], ["install-fetch.sh", "install-prereq.sh"])

    def test_g12_the_guarded_marker_form_is_recorded_like_the_plain_one(self):
        guarded = self.root / "deploy-guarded"
        shutil.copytree(self.root / "deploy", guarded)
        (guarded / "install.sh").write_text('#!/bin/sh\n[ ! -f "$here/install-prereq.sh" ] || . "$here/install-prereq.sh"   # @include\n'
                                            '[ ! -f "$here/install-fetch.sh" ] || . "$here/install-fetch.sh"   # @include\n')
        out, record = self.root / "g.sh", self.root / "g.json"
        result = self.join_helper(guarded, out, record, "a" * 40, "b" * 40, "sha1")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.strict_record(record, guarded, out, fmt="sha1", commit="a" * 40, tree="b" * 40)

    def test_g12_a_refused_join_writes_no_record(self):
        control = self.join_helper(self.root / "deploy", self.root / "c.sh", self.root / "c.json", "a" * 40, "b" * 40, "sha1")
        self.assertEqual(control.returncode, 0, control.stderr)
        def broken(name, edit):
            copy = self.root / ("deploy-" + name)
            shutil.copytree(self.root / "deploy", copy)
            edit(copy)
            return copy
        def drop_include(copy):
            (copy / "install-fetch.sh").unlink()
        def symlink_include(copy):
            target = copy / "real-fetch.sh"
            target.write_bytes((copy / "install-fetch.sh").read_bytes())
            (copy / "install-fetch.sh").unlink()
            (copy / "install-fetch.sh").symlink_to(target)
        def shebang_include(copy):
            (copy / "install-fetch.sh").write_text("#!/bin/sh\nrel_fetch() { return 0; }\n")
        def no_function(copy):
            (copy / "install-fetch.sh").write_text("true\n")
        for name, edit in [("missing", drop_include), ("symlink", symlink_include), ("shebang", shebang_include), ("nofunction", no_function)]:
            with self.subTest(case=name):
                out, record = self.root / f"{name}.sh", self.root / f"{name}.json"
                result = self.join_helper(broken(name, edit), out, record, "a" * 40, "b" * 40, "sha1")
                self.assertNotEqual(result.returncode, 0)
                self.assertFalse(out.exists() or record.exists())

    def test_g12_a_real_release_publishes_the_record_beside_the_release_directory(self):
        self.valid_podman()
        head = self.git_root()
        tree = self.git("rev-parse", "HEAD^{tree}")
        fmt = self.git("rev-parse", "--show-object-format")
        result = self.run_release("1.2.3")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        record = self.root / "dist/1.2.3.install.sh.binding.json"
        self.assertTrue(record.is_file() and not record.is_symlink())
        self.assertEqual(os.stat(record).st_mode & 0o077, 0)
        self.assertEqual({p.name for p in (self.root / "dist/1.2.3").iterdir()},
                         {"cortex-1.2.3-x86_64-linux.tar.gz", "cortex-server-1.2.3-x86_64.oci.tar", "install.sh", "manifest.json", "install.meta", "SHA256SUMS"})
        self.assertNotIn("binding", (self.root / "dist/1.2.3/SHA256SUMS").read_text())
        key_bytes = self.anchor_key.read_bytes()
        principal, _, key = key_bytes.decode().split()
        self.strict_anchor_record(record, self.root / "deploy", self.root / "dist/1.2.3/install.sh", principal=principal, key=key.strip(),
                                  fpr=self.independent_fpr(key.strip()), key_bytes=key_bytes, commit=head, tree=tree, fmt=fmt)

    def test_g12_a_seam_release_writes_no_record_and_says_so(self):
        self.valid_podman()
        result = self.run_release("1.2.3")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertFalse((self.root / "dist/1.2.3.install.sh.binding.json").exists())
        self.assertIn("no binding record", result.stderr)

    def test_g12_a_stale_record_destination_refuses_before_any_gate(self):
        self.valid_podman()
        self.git_root()
        (self.root / "dist").mkdir()
        (self.root / "dist/1.2.3.install.sh.binding.json").write_text("{}\n")
        result = self.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertIn("binding record destination exists", result.stderr)
        self.assertEqual(self.calls(), [])
        self.assertFalse((self.root / "dist/1.2.3").exists())

    def test_g12e1_a_directory_created_at_the_record_name_during_the_build_does_not_absorb_the_record(self):
        self.valid_podman()
        self.git_root()
        trap = self.root / "dist/1.2.3.install.sh.binding.json"
        result = self.run_release("1.2.3", RELEASE_TEST_MKDIR_ON_BUILD=str(trap))
        self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertNotIn("built dist/1.2.3", result.stdout + result.stderr)
        self.assertTrue(trap.is_dir())
        self.assertEqual(list(trap.iterdir()), [], "the record must not be moved into the directory that took its name")

    def test_g12e2_a_failed_record_move_says_the_release_directory_is_already_published(self):
        self.valid_podman()
        self.git_root()
        trap = self.root / "dist/1.2.3.install.sh.binding.json"
        result = self.run_release("1.2.3", RELEASE_TEST_MKDIR_ON_BUILD=str(trap))
        self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertIn("release: binding record destination appeared during build", result.stderr)
        self.assertIn("dist/1.2.3 is already published: remove it before running again", result.stderr)
        self.assertTrue((self.root / "dist/1.2.3").is_dir())

    def test_g12_a_release_that_fails_after_the_join_publishes_no_record(self):
        self.valid_podman()
        self.git_root()
        first = self.run_release("1.2.3")
        self.assertEqual(first.returncode, 0, (first.stdout, first.stderr))
        record = self.root / "dist/1.2.3.install.sh.binding.json"
        self.assertTrue(record.is_file())
        shutil.rmtree(self.root / "dist/1.2.3")
        record.unlink()
        second = self.run_release("1.2.3", RELEASE_TEST_FAIL="podman-build")
        self.assertNotEqual(second.returncode, 0, (second.stdout, second.stderr))
        self.assertFalse(record.exists())
        self.assertFalse((self.root / "dist/1.2.3").exists())

    # ---- the builder must not build what the verifier refuses (cox a4c59607): db-roles.sh and compose.yml over STAGED_FILE_MAX_BYTES ----

    # = STAGED_FILE_MAX_BYTES (1024 * 1024) in crates/cortex-upgrade/src/verify.rs of finn's R-4a (a02e8f3, branch wave1/r4a-r6; it refuses a file only when
    # it is LARGER than that): the two values MUST change together (a comment, not a build-time link).
    STAGED_CAP = 1024 * 1024
    # = FILE_MAX_BYTES (256 * 1024) in crates/cortex-upgrade/src/compose.rs: the upgrade engine's compose reader refuses a compose.yml LARGER than that (engine.rs
    # compare_db_service reads the candidate's compose.yml through it), so the builder refuses it too (hale SC-1, cox's ruling): the two MUST change together.
    COMPOSE_CAP = 256 * 1024

    def pad_to(self, path, size):
        base = Path(path).read_bytes()
        filler = size - len(base) - 2
        self.assertGreaterEqual(filler, 0)
        Path(path).write_bytes(base + b"#" + b"x" * filler + b"\n")
        self.assertEqual(Path(path).stat().st_size, size)

    def sizes_helper(self, layout):
        return subprocess.run([sys.executable, "-I", str(self.root / "scripts/release_manifest.py"), "check-staged-sizes", "--layout", str(layout)],
                              text=True, capture_output=True)

    def test_cap_each_staged_file_passes_at_its_own_cap_and_refuses_one_byte_over(self):
        for name, cap, constant in (("db-roles.sh", self.STAGED_CAP, "STAGED_FILE_MAX_BYTES"), ("compose.yml", self.COMPOSE_CAP, "FILE_MAX_BYTES in crates/cortex-upgrade/src/compose.rs")):
            with self.subTest(file=name):
                layout = self.root / ("layout-" + name)
                shutil.copytree(self.root / "deploy", layout)
                self.pad_to(layout / name, cap)
                at_cap = self.sizes_helper(layout)
                self.assertEqual(at_cap.returncode, 0, at_cap.stderr)
                (layout / name).write_bytes((layout / name).read_bytes() + b"x")
                over = self.sizes_helper(layout)
                self.assertNotEqual(over.returncode, 0)
                self.assertIn(name, over.stderr)
                self.assertIn(str(cap), over.stderr)
                self.assertIn(constant, over.stderr)

    def test_cap_compose_yml_and_db_roles_sh_have_different_bounds(self):
        # 300 KiB: over the engine's compose bound, inside the verifier's 1 MiB for db-roles.sh
        compose_layout, roles_layout = self.root / "layout-compose-300k", self.root / "layout-roles-300k"
        for layout in (compose_layout, roles_layout):
            shutil.copytree(self.root / "deploy", layout)
        self.pad_to(compose_layout / "compose.yml", 300 * 1024)
        self.pad_to(roles_layout / "db-roles.sh", 300 * 1024)
        compose, roles = self.sizes_helper(compose_layout), self.sizes_helper(roles_layout)
        self.assertNotEqual(compose.returncode, 0, "a compose.yml over 256 KiB must be refused (the engine's reader refuses it)")
        self.assertIn("compose.yml", compose.stderr)
        self.assertNotIn("STAGED_FILE_MAX_BYTES", compose.stderr)
        self.assertEqual(roles.returncode, 0, "db-roles.sh keeps the verifier's 1 MiB bound")

    def test_cap_a_release_with_a_roles_script_at_the_cap_builds_and_one_byte_over_refuses_before_s5(self):
        self.valid_podman()
        self.pad_to(self.root / "deploy/db-roles.sh", self.STAGED_CAP)
        built = self.run_release("1.2.3")
        self.assertEqual(built.returncode, 0, (built.stdout, built.stderr))
        shutil.rmtree(self.root / "dist/1.2.3")
        roles = self.root / "deploy/db-roles.sh"
        roles.write_bytes(roles.read_bytes() + b"x")
        self.log.unlink(missing_ok=True)
        over = self.run_release("1.2.3")
        self.assertNotEqual(over.returncode, 0, (over.stdout, over.stderr))
        self.assertIn("db-roles.sh", over.stderr)
        self.assertIn("STAGED_FILE_MAX_BYTES", over.stderr)
        self.assertEqual(self.calls(), ["ci"])
        self.assertFalse((self.root / "dist/1.2.3").exists())
        self.assertFalse((self.root / "dist/1.2.3.part").exists())

    def test_cap_a_release_with_a_compose_file_over_the_cap_refuses_before_s5(self):
        self.valid_podman()
        self.pad_to(self.root / "deploy/compose.yml", self.COMPOSE_CAP - 2000)
        built = self.run_release("1.2.3")
        self.assertEqual(built.returncode, 0, (built.stdout, built.stderr))
        shutil.rmtree(self.root / "dist/1.2.3")
        self.pad_to(self.root / "deploy/compose.yml", self.COMPOSE_CAP + 1000)
        self.log.unlink(missing_ok=True)
        over = self.run_release("1.2.3")
        self.assertNotEqual(over.returncode, 0, (over.stdout, over.stderr))
        self.assertIn("compose.yml", over.stderr)
        self.assertIn("FILE_MAX_BYTES in crates/cortex-upgrade/src/compose.rs", over.stderr)
        self.assertEqual(self.calls(), ["ci"])
        self.assertFalse((self.root / "dist/1.2.3").exists())

    def test_x1_the_real_s5_checker_passes_the_release_layout(self):
        shutil.rmtree(self.root / "deploy")
        shutil.copytree(ROOT / "deploy", self.root / "deploy")
        for name in ("s5-check.sh", "s5_check.py"):
            shutil.copy(ROOT / "scripts" / name, self.root / "scripts" / name)
        shutil.copy(ROOT / "crates/cortex-server/Cargo.toml", self.root / "crates/cortex-server/Cargo.toml")
        self.env.pop("CORTEX_RELEASE_S5")
        result = self.run_release("1.2.3")
        output = result.stdout + result.stderr
        self.assertNotIn("FAIL S5-23", output)
        self.assertIn("s5-check: OK", output)
        self.assertIn("squawk", self.calls())

    def test_x1_a_missing_server_cargo_toml_refuses_before_the_s5_gate(self):
        (self.root / "crates/cortex-server/Cargo.toml").unlink()
        result = self.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertIn("Cargo.toml is missing", result.stderr)
        self.assertEqual(self.calls(), ["ci"])
        self.assertFalse((self.root / "dist/1.2.3").exists())

    def test_x1_a_symlinked_server_cargo_toml_is_refused_before_the_s5_gate(self):
        manifest = self.root / "crates/cortex-server/Cargo.toml"
        target = self.root / "server-manifest-copy"
        target.write_bytes(manifest.read_bytes())
        manifest.unlink()
        manifest.symlink_to(target)
        result = self.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertIn("Cargo.toml is missing", result.stderr)
        self.assertEqual(self.calls(), ["ci"])

    # ---- the anchor join, the PRODUCER (cox c7a09dae; contract G12-BINDING-RECORD-SCHEMA-nix-r4.md, accepted for implementation by vera) ----
    # Test keys only: every key below is made by the test with ssh-keygen in a private directory. A test installer is NOT a candidate.
    # The refusal texts below are the contract of the GREEN code (a cell pins the cause, not only the exit status).

    ANCHOR_SPLICE = ("each '# @include' marker line, newline included, is replaced by the exact bytes of its include; then each of the three empty anchor "
                     "statements CORTEX_ANCHOR_PRINCIPAL='', CORTEX_ANCHOR_KEY='', CORTEX_ANCHOR_FPR='' is replaced by the same statement holding its "
                     "recorded value in single quotes; nothing else is added or changed")
    ANCHOR_RECORD_KEYS = ["schema", "object_format", "commit", "tree", "includes", "splice", "joined_sha256", "anchor"]
    ANCHOR_FIELDS = ["principal", "key", "fpr", "key_input_sha256"]
    ANCHOR_NAMES = ("CORTEX_ANCHOR_PRINCIPAL", "CORTEX_ANCHOR_KEY", "CORTEX_ANCHOR_FPR")
    ANCHOR_HEADER = "#!/bin/sh\n# Cortex installer (fixture)\n\n# a second header line after a blank line\n"
    ANCHOR_LINES = "CORTEX_ANCHOR_PRINCIPAL=''\nCORTEX_ANCHOR_KEY=''\nCORTEX_ANCHOR_FPR=''\n"
    ANCHOR_BODY = '. "$here/install-prereq.sh"   # @include\n. "$here/install-fetch.sh"   # @include\n'
    FAKE_FPR = "SHA256:" + "A" * 43

    def anchor_keydir(self):
        directory = self.root / "keys"
        if not directory.exists():
            directory.mkdir(mode=0o700)
            os.chmod(directory, 0o700)
        return directory

    def make_key(self, name="good", principal="release-test@fixture"):
        """A throwaway ed25519 key made by the real tool; the file is the contract's format: '<principal> ssh-ed25519 <key>' and one LF."""
        directory = self.anchor_keydir()
        subprocess.run(["/usr/bin/ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", "fixture", "-f", str(directory / ("gen-" + name))],
                       check=True, capture_output=True)
        key = (directory / ("gen-" + name + ".pub")).read_text().split()[1]
        path = directory / (name + ".txt")
        path.write_text(f"{principal} ssh-ed25519 {key}\n")
        return path, principal, key

    def independent_fpr(self, key):
        """The expectation the test derives for itself, by the real tool, in its own 0700 directory and 0600 file."""
        scratch = Path(tempfile.mkdtemp(prefix="indep-", dir=str(self.anchor_keydir())))
        os.chmod(scratch, 0o700)
        descriptor = os.open(scratch / "k.pub", os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        os.write(descriptor, f"ssh-ed25519 {key}\n".encode())
        os.close(descriptor)
        out = subprocess.run(["/usr/bin/ssh-keygen", "-l", "-f", str(scratch / "k.pub")], capture_output=True, text=True, check=True).stdout
        shutil.rmtree(scratch)
        match = re.fullmatch(r"256 (SHA256:[A-Za-z0-9+/]{43}) .* \(ED25519\)\n", out)
        self.assertIsNotNone(match, out)
        return match.group(1)

    def anchor_deploy(self, name="deploy-anchor", lines=None, header=None, body=None, fetch_extra=""):
        """A copy of the fixture deploy/ whose install.sh carries the three empty anchor statements right after the header."""
        directory = self.root / name
        shutil.copytree(self.root / "deploy", directory)
        text = (self.ANCHOR_HEADER if header is None else header) + (self.ANCHOR_LINES if lines is None else lines) + (self.ANCHOR_BODY if body is None else body)
        (directory / "install.sh").write_text(text)
        if fetch_extra:
            (directory / "install-fetch.sh").write_text((directory / "install-fetch.sh").read_text() + fetch_extra)
        return directory

    def join_anchor(self, deploy, out, record, key_file, commit="a" * 40, tree="b" * 40, fmt="sha1", **env):
        args = [sys.executable, "-I", str(self.root / "scripts/release_manifest.py"), "join-installer", "--deploy", str(deploy), "--output", str(out)]
        if record is not None:
            args += ["--record", str(record), "--commit", commit, "--tree", tree, "--object-format", fmt]
        if key_file is not None:
            args += ["--trust-key-file", str(key_file)]
        return subprocess.run(args, text=True, capture_output=True, timeout=60, env={**os.environ, **env})

    def expected_anchor_installer(self, deploy, principal, key, fpr):
        """T1 then T2 as the test computes them for itself: markers replaced by the raw include bytes, then each empty statement by its quoted value."""
        template = (Path(deploy) / "install.sh").read_bytes()
        t1 = b"".join((Path(deploy) / self.MARKER.search(line.decode().rstrip("\n")).group(1)).read_bytes() if self.MARKER.search(line.decode().rstrip("\n")) else line
                      for line in template.splitlines(keepends=True))
        final = t1
        for name, value in zip(self.ANCHOR_NAMES, (principal, key, fpr)):
            empty = f"{name}=''\n".encode()
            self.assertEqual(final.count(empty), 1, name)
            final = final.replace(empty, f"{name}='{value}'\n".encode())
        return final

    def strict_anchor_record(self, record, deploy, shipped, *, principal, key, fpr, key_bytes, commit="a" * 40, tree="b" * 40, fmt="sha1"):
        """The schema-2 record as nix's runner will read it (contract sections 2 and 5): eight keys in order, an int schema 2, the four-key anchor equal to the
        values the TEST derived for itself, the exact splice sentence, joined_sha256 over the FINAL bytes, which equal the test's own T1 then T2."""
        def no_duplicates(pairs):
            seen = {}
            for name, value in pairs:
                self.assertNotIn(name, seen, "duplicate key " + name)
                seen[name] = value
            return seen
        raw = Path(record).read_bytes()
        self.assertTrue(raw.endswith(b"\n"))
        rec = json.loads(raw.decode("ascii"), object_pairs_hook=no_duplicates)
        self.assertEqual(list(rec), self.ANCHOR_RECORD_KEYS)
        self.assertIs(type(rec["schema"]), int)
        self.assertEqual(rec["schema"], 2)
        self.assertEqual(rec["object_format"], fmt)
        self.assertEqual((rec["commit"], rec["tree"]), (commit, tree))
        self.assertEqual(rec["splice"], self.ANCHOR_SPLICE)
        self.assertEqual(list(rec["anchor"]), self.ANCHOR_FIELDS)
        self.assertEqual(rec["anchor"], {"principal": principal, "key": key, "fpr": fpr, "key_input_sha256": hashlib.sha256(key_bytes).hexdigest()})
        order = [m.group(1) for line in (Path(deploy) / "install.sh").read_text().splitlines() for m in [self.MARKER.search(line)] if m]
        self.assertEqual([i["name"] for i in rec["includes"]], order)
        for entry in rec["includes"]:
            self.assertEqual(entry["sha256"], hashlib.sha256((Path(deploy) / entry["name"]).read_bytes()).hexdigest())
        shipped_bytes = Path(shipped).read_bytes()
        self.assertEqual(rec["joined_sha256"], hashlib.sha256(shipped_bytes).hexdigest())
        self.assertEqual(shipped_bytes, self.expected_anchor_installer(deploy, principal, key, fpr))
        return rec

    # ---- A: the key file (contract section 4.1: one bounded read, one line, three fields) ----

    def test_anchor_a1_a_consistent_key_file_joins_and_every_derived_value_equals_an_independent_ssh_keygen(self):
        path, principal, key = self.make_key()
        deploy = self.anchor_deploy()
        out, record = self.root / "a1.sh", self.root / "a1.json"
        result = self.join_anchor(deploy, out, record, path)
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.strict_anchor_record(record, deploy, out, principal=principal, key=key, fpr=self.independent_fpr(key), key_bytes=path.read_bytes())

    def test_anchor_a2_the_key_file_may_lack_its_trailing_line_feed_and_may_use_the_whole_principal_alphabet(self):
        for label, principal, trailing in [("no trailing LF", "release-test@fixture", ""), ("one trailing LF", "release-test@fixture", "\n"),
                                           ("64 characters", "p" * 64, "\n"), ("every allowed character", "aZ09._@-x", "\n")]:
            with self.subTest(case=label):
                path, _, key = self.make_key("alpha-" + str(abs(hash(label))), principal)
                path.write_text(f"{principal} ssh-ed25519 {key}{trailing}")
                deploy = self.anchor_deploy("deploy-a2-" + str(abs(hash(label))))
                out, record = self.root / "a2.sh", self.root / "a2.json"
                out.unlink(missing_ok=True)
                result = self.join_anchor(deploy, out, record, path)
                self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
                self.strict_anchor_record(record, deploy, out, principal=principal, key=key, fpr=self.independent_fpr(key), key_bytes=path.read_bytes())

    def test_anchor_a3_each_key_file_rule_refuses_with_its_own_cause_and_writes_nothing(self):
        _, principal, key = self.make_key("rules")
        good = f"{principal} ssh-ed25519 {key}"
        wrong_prefix = "AAAAC3NzaC1lZDI1NTE5AAAAJ" + key[25:]
        padded = lambda size: good.encode() + b" " * (size - len(good))
        cases = [
            ("an empty file", b"", "is empty", None),
            ("a single line feed", b"\n", "is empty", None),
            ("a second line", (good + "\n" + good + "\n").encode(), "more than one line", None),
            ("two trailing line feeds", (good + "\n\n").encode(), "more than one line", None),
            ("a ssh-rsa key", f"{principal} ssh-rsa {key}\n".encode(), "key type must be ssh-ed25519", None),
            ("an ecdsa key", f"{principal} ecdsa-sha2-nistp256 {key}\n".encode(), "key type must be ssh-ed25519", None),
            ("a security-key ed25519", f"{principal} sk-ssh-ed25519@openssh.com {key}\n".encode(), "key type must be ssh-ed25519", None),
            ("a private key header", b"-----BEGIN OPENSSH PRIVATE KEY-----\n", "private key", None),
            ("an empty principal (a leading space)", f" ssh-ed25519 {key}\n".encode(), "must be '<principal> ssh-ed25519 <key>'", None),
            ("a principal of 65 characters", f"{'p' * 65} ssh-ed25519 {key}\n".encode(), "principal is not 1 to 64 characters", None),
            ("a hyphen-led principal", f"-lead ssh-ed25519 {key}\n".encode(), "principal is not 1 to 64 characters", None),
            ("a principal with a colon", f"a:b ssh-ed25519 {key}\n".encode(), "principal is not 1 to 64 characters", None),
            ("a principal with a single quote", f"a'b ssh-ed25519 {key}\n".encode(), "principal is not 1 to 64 characters", None),
            ("a key one character short", (good[:-1] + "\n").encode(), "key is not 68 characters", None),
            ("a key one character long", (good + "A\n").encode(), "key is not 68 characters", None),
            ("a key with the wrong prefix", f"{principal} ssh-ed25519 {wrong_prefix}\n".encode(), "starting AAAAC3NzaC1lZDI1NTE5AAAAI", None),
            ("a key with a character outside the alphabet", f"{principal} ssh-ed25519 {key[:10]}!{key[11:]}\n".encode(), "key is not 68 characters", None),
            ("a carriage return", (good + "\r\n").encode(), "contains a CR", None),
            ("a NUL byte", good.encode()[:5] + b"\x00" + good.encode()[6:] + b"\n", "contains a NUL", None),
            ("a non-ASCII byte", good.encode()[:5] + "é".encode() + good.encode()[6:] + b"\n", "is not ASCII", None),
            ("two fields", f"{principal} {key}\n".encode(), "must be '<principal> ssh-ed25519 <key>'", None),
            ("four fields", (good + " extra\n").encode(), "must be '<principal> ssh-ed25519 <key>'", None),
            ("a double space", f"{principal}  ssh-ed25519 {key}\n".encode(), "must be '<principal> ssh-ed25519 <key>'", None),
            ("a leading space", (" " + good + "\n").encode(), "must be '<principal> ssh-ed25519 <key>'", None),
            ("a tab as the separator", good.replace(" ", "\t", 1).encode() + b"\n", "must be '<principal> ssh-ed25519 <key>'", None),
            ("1025 bytes (over the bound)", padded(1025), "is larger than 1024 bytes", None),
            ("1024 bytes (inside the bound: refused by the grammar, not the size)", padded(1024), "must be '<principal> ssh-ed25519 <key>'", "larger than"),
        ]
        deploy = self.anchor_deploy("deploy-a3")
        for number, (label, data, cause, forbidden) in enumerate(cases):
            with self.subTest(case=label):
                path = self.anchor_keydir() / f"rule-{number}.txt"
                path.write_bytes(data)
                out, record = self.root / f"a3-{number}.sh", self.root / f"a3-{number}.json"
                result = self.join_anchor(deploy, out, record, path)
                self.assertEqual(result.returncode, 1, (result.stdout, result.stderr))
                self.assertTrue(result.stderr.startswith("release: trust key file"), result.stderr)
                self.assertIn(cause, result.stderr)
                if forbidden:
                    self.assertNotIn(forbidden, result.stderr)
                self.assertNotIn("Traceback", result.stderr)
                self.assertFalse(out.exists() or record.exists())

    def test_anchor_a5_a_key_file_is_read_to_one_byte_past_the_bound_even_when_the_reads_are_short(self):
        # a descriptor may answer in short chunks: the loop must keep asking until it holds one byte MORE than the bound, or an oversized file whose chunks
        # happen to end exactly at the bound would look like a file of exactly the bound
        helper = self.helper_module()
        real_read = os.read
        def chunked(descriptor, count):
            return real_read(descriptor, min(count, 128))
        for size, oversized in ((1024, False), (1025, True), (1300, True)):
            with self.subTest(size=size):
                key = self.root / ("short-read-%d.pub" % size)
                key.write_bytes(b"A" * size)
                with mock.patch.object(helper.os, "read", chunked):
                    with self.assertRaises(ValueError) as caught:
                        helper.read_trust_key(str(key))
                self.assertEqual("larger than 1024 bytes" in str(caught.exception), oversized, str(caught.exception))

    def test_anchor_a4_a_key_file_that_is_not_a_plain_readable_file_is_refused(self):
        good, _, _ = self.make_key("kinds")
        deploy = self.anchor_deploy("deploy-a4")
        keydir = self.anchor_keydir()
        link = keydir / "link.txt"
        link.symlink_to(good)
        directory = keydir / "adir"
        directory.mkdir()
        fifo = keydir / "afifo"
        os.mkfifo(fifo)
        unreadable = keydir / "unreadable.txt"
        unreadable.write_bytes(good.read_bytes())
        os.chmod(unreadable, 0o000)
        cases = [("a symlink to a good file", link, "is not a regular file"), ("a directory", directory, "is not a regular file"),
                 ("a FIFO (must not hang)", fifo, "is not a regular file"), ("a missing file", keydir / "absent.txt", "cannot be read"),
                 ("an unreadable file", unreadable, "cannot be read")]
        try:
            for number, (label, path, cause) in enumerate(cases):
                with self.subTest(case=label):
                    out, record = self.root / f"a4-{number}.sh", self.root / f"a4-{number}.json"
                    result = self.join_anchor(deploy, out, record, path)
                    self.assertEqual(result.returncode, 1, (result.stdout, result.stderr))
                    self.assertTrue(result.stderr.startswith("release: trust key file"), result.stderr)
                    self.assertIn(cause, result.stderr)
                    self.assertFalse(out.exists() or record.exists())
        finally:
            os.chmod(unreadable, 0o600)

    # ---- B: the fingerprint tool (contract section 4.2). Unit cells: the tool path and the bound are module constants a cell patches, so
    # production carries no seam. vera's named rules: TIMEOUT, MALFORMED output, TOOL KIND, the modes, the cleanup. ----

    def helper_module(self):
        spec = importlib.util.spec_from_file_location("release_manifest_anchor", ROOT / "scripts/release_manifest.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def fake_keygen(self, body, name="fake-keygen"):
        path = self.root / name
        path.write_text("#!/bin/sh\n" + body)
        path.chmod(0o755)
        return path

    def scratch_parent(self):
        """A private directory the helper's scratch lands in (tempfile.tempdir), so a leftover is visible."""
        parent = self.root / "scratch-parent"
        parent.mkdir(exist_ok=True)
        previous = tempfile.tempdir
        tempfile.tempdir = str(parent)
        self.addCleanup(setattr, tempfile, "tempdir", previous)
        return parent

    def test_anchor_b1_the_tool_and_its_bound_are_fixed_by_the_helper(self):
        module = self.helper_module()
        self.assertEqual(module.SSH_KEYGEN, "/usr/bin/ssh-keygen")
        self.assertEqual(module.KEYGEN_TIMEOUT, 10)

    def test_anchor_b2_a_good_tool_answer_is_taken_and_the_tool_gets_a_0600_file_in_a_0700_directory_that_is_removed_after(self):
        path, principal, key = self.make_key("b2")
        parent = self.scratch_parent()
        record = self.root / "keygen-record"
        fake = self.fake_keygen(f'printf "%s\\n" "$@" > "{record}.args"\nstat -c %a "$3" > "{record}.filemode"\nstat -c %a "$(dirname "$3")" > "{record}.dirmode"\n'
                                f'dirname "$3" > "{record}.dir"\ncat "$3" > "{record}.content"\necho "256 {self.FAKE_FPR} fixture (ED25519)"\n')
        module = self.helper_module()
        module.SSH_KEYGEN = str(fake)
        derived = module.derive_anchor(str(path))
        self.assertEqual(derived, {"principal": principal, "key": key, "fpr": self.FAKE_FPR, "key_input_sha256": hashlib.sha256(path.read_bytes()).hexdigest()})
        args = Path(f"{record}.args").read_text().splitlines()
        self.assertEqual(args[:2], ["-l", "-f"])
        self.assertEqual(len(args), 3)
        self.assertEqual(Path(f"{record}.filemode").read_text().strip(), "600")
        self.assertEqual(Path(f"{record}.dirmode").read_text().strip(), "700")
        self.assertEqual(Path(f"{record}.content").read_text(), f"ssh-ed25519 {key}\n")
        self.assertFalse(Path(Path(f"{record}.dir").read_text().strip()).exists(), "the scratch directory must be removed after a success")
        self.assertEqual(list(parent.iterdir()), [])

    def test_anchor_b3_a_tool_failure_is_refused_and_leaves_no_scratch(self):
        path, _, _ = self.make_key("b3")
        parent = self.scratch_parent()
        module = self.helper_module()
        module.SSH_KEYGEN = str(self.fake_keygen("exit 3\n"))
        with self.assertRaises(ValueError) as caught:
            module.derive_anchor(str(path))
        self.assertTrue(str(caught.exception).startswith("trust key file: fingerprint tool failed (status 3)"), str(caught.exception))
        self.assertEqual(list(parent.iterdir()), [])

    def test_anchor_b4_a_tool_that_does_not_finish_in_the_bound_is_ended_and_refused(self):
        path, _, _ = self.make_key("b4")
        parent = self.scratch_parent()
        pidfile = self.root / "sleeper.pid"
        module = self.helper_module()
        module.SSH_KEYGEN = str(self.fake_keygen(f'echo $$ > "{pidfile}"\nexec sleep 30\n'))
        module.KEYGEN_TIMEOUT = 1
        started = time.monotonic()
        with self.assertRaises(ValueError) as caught:
            module.derive_anchor(str(path))
        self.assertLess(time.monotonic() - started, 10)
        self.assertTrue(str(caught.exception).startswith("trust key file: fingerprint tool timed out after 1 seconds"), str(caught.exception))
        pid = int(pidfile.read_text())
        for _ in range(50):
            try:
                os.kill(pid, 0)
            except ProcessLookupError:
                break
            time.sleep(0.1)
        else:
            self.fail("the tool is still running after the timeout")
        self.assertEqual(list(parent.iterdir()), [])

    def test_anchor_b5_malformed_tool_output_is_refused_whatever_its_shape(self):
        path, _, _ = self.make_key("b5")
        parent = self.scratch_parent()
        module = self.helper_module()
        fpr = self.FAKE_FPR
        outputs = [("empty", ""), ("a word", "hello\n"), ("a second line", f"256 {fpr} c (ED25519)\\nextra\\n"),
                   ("a short digest", "256 SHA256:short c (ED25519)\\n"), ("text after the kind", f"256 {fpr} c (ED25519) trailing\\n"),
                   ("no bit count", f"{fpr} c (ED25519)\\n"), ("a wrong bit count", f"255 {fpr} c (ED25519)\\n"),
                   ("a digest that is not SHA256", f"256 MD5:{'A' * 43} c (ED25519)\\n"), ("a comment-less line", f"256 {fpr} (ED25519)\\n")]
        for label, text in outputs:
            with self.subTest(output=label):
                body = 'printf "" \n' if text == "" else f'printf "{text}"\n'
                module.SSH_KEYGEN = str(self.fake_keygen(body))
                with self.assertRaises(ValueError) as caught:
                    module.derive_anchor(str(path))
                self.assertTrue(str(caught.exception).startswith("trust key file: fingerprint tool printed malformed output"), str(caught.exception))
                self.assertEqual(list(parent.iterdir()), [])

    def test_anchor_b6_a_tool_that_reports_another_key_kind_is_refused(self):
        path, _, _ = self.make_key("b6")
        parent = self.scratch_parent()
        module = self.helper_module()
        digest = "SHA256:" + "A" * 43
        for label, line in [("RSA", f"3072 {digest} c (RSA)"), ("ECDSA", f"256 {digest} c (ECDSA)"), ("a security-key ED25519", f"256 {digest} c (ED25519-SK)"),
                            ("DSA", f"1024 {digest} c (DSA)")]:
            with self.subTest(kind=label):
                module.SSH_KEYGEN = str(self.fake_keygen(f'echo "{line}"\n'))
                with self.assertRaises(ValueError) as caught:
                    module.derive_anchor(str(path))
                self.assertTrue(str(caught.exception).startswith("trust key file: fingerprint tool reported a key that is not ED25519"), str(caught.exception))
                self.assertEqual(list(parent.iterdir()), [])

    def test_anchor_b7_the_key_file_is_opened_exactly_once_without_following_a_symlink(self):
        path, _, _ = self.make_key("b7")
        module = self.helper_module()
        module.SSH_KEYGEN = str(self.fake_keygen(f'echo "256 {self.FAKE_FPR} fixture (ED25519)"\n'))
        real_open, real_io_open, real_builtin_open = os.open, io.open, builtins.open
        opened, other = [], []
        def counting_open(target, flags, *rest, **keywords):
            if str(target) == str(path):
                opened.append(flags)
            return real_open(target, flags, *rest, **keywords)
        def counting_io_open(target, *rest, **keywords):
            if str(target) == str(path):
                other.append("io.open")
            return real_io_open(target, *rest, **keywords)
        def counting_builtin_open(target, *rest, **keywords):
            if str(target) == str(path):
                other.append("builtins.open")
            return real_builtin_open(target, *rest, **keywords)
        with mock.patch("os.open", side_effect=counting_open), mock.patch("io.open", side_effect=counting_io_open), mock.patch("builtins.open", side_effect=counting_builtin_open):
            module.derive_anchor(str(path))
        self.assertEqual(len(opened), 1, "the key file must be read once, from one buffer")
        self.assertEqual(other, [], "no second route to the key file (a second read would not be the buffer that was hashed and parsed)")
        self.assertTrue(opened[0] & os.O_NOFOLLOW, "the key file must be opened without following a symlink")

    def test_anchor_b8_a_trappable_signal_during_the_tool_leaves_no_scratch_and_no_process(self):
        path, _, _ = self.make_key("b8")
        parent = self.root / "signal-parent"
        parent.mkdir()
        pidfile = self.root / "signal-sleeper.pid"
        fake = self.fake_keygen(f'echo $$ > "{pidfile}"\nexec sleep 30\n', "signal-keygen")
        harness = ("import importlib.util, sys, tempfile\n"
                   "spec = importlib.util.spec_from_file_location('h', sys.argv[1]); m = importlib.util.module_from_spec(spec); spec.loader.exec_module(m)\n"
                   "m.SSH_KEYGEN = sys.argv[2]; m.KEYGEN_TIMEOUT = 25; tempfile.tempdir = sys.argv[4]\n"
                   "m.derive_anchor(sys.argv[3])\n")
        child = subprocess.Popen([sys.executable, "-I", "-c", harness, str(ROOT / "scripts/release_manifest.py"), str(fake), str(path), str(parent)],
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            for _ in range(100):
                if pidfile.exists() and pidfile.read_text().strip():
                    break
                time.sleep(0.1)
            else:
                self.fail("the tool never started")
            child.send_signal(signal.SIGTERM)
            try:
                child.communicate(timeout=20)
            except subprocess.TimeoutExpired:
                self.fail("the helper did not stop within 20 s of SIGTERM")
        finally:
            if child.poll() is None:
                child.kill()
                child.communicate()
        self.assertNotEqual(child.returncode, 0)
        self.assertEqual(list(parent.iterdir()), [], "the scratch directory must be removed on SIGTERM")
        sleeper = int(pidfile.read_text())
        for _ in range(50):
            try:
                os.kill(sleeper, 0)
            except ProcessLookupError:
                break
            time.sleep(0.1)
        else:
            self.fail("the tool is still running after the signal")

    # ---- C: the transforms (contract section 3): T1 then T2, the three empty statements exactly once, at the top, in order ----

    def test_anchor_c1_the_final_bytes_differ_from_the_joined_template_in_exactly_the_three_statements(self):
        path, principal, key = self.make_key("c1")
        deploy = self.anchor_deploy("deploy-c1")
        t1, final, record = self.root / "c1-t1.sh", self.root / "c1-final.sh", self.root / "c1.json"
        keyless = self.join_anchor(deploy, t1, None, None)
        anchored = self.join_anchor(deploy, final, record, path)
        self.assertEqual((keyless.returncode, anchored.returncode), (0, 0), (keyless.stderr, anchored.stderr))
        self.assertIn(self.ANCHOR_LINES.encode(), t1.read_bytes())
        fpr = self.independent_fpr(key)
        differing = [(a, b) for a, b in zip(t1.read_bytes().splitlines(), final.read_bytes().splitlines()) if a != b]
        self.assertEqual(len(t1.read_bytes().splitlines()), len(final.read_bytes().splitlines()))
        self.assertEqual(differing, [(b"CORTEX_ANCHOR_PRINCIPAL=''", f"CORTEX_ANCHOR_PRINCIPAL='{principal}'".encode()),
                                     (b"CORTEX_ANCHOR_KEY=''", f"CORTEX_ANCHOR_KEY='{key}'".encode()),
                                     (b"CORTEX_ANCHOR_FPR=''", f"CORTEX_ANCHOR_FPR='{fpr}'".encode())])

    def test_anchor_c2_the_final_statements_are_valid_shell_that_hands_back_exactly_the_values(self):
        path, principal, key = self.make_key("c2")
        deploy = self.anchor_deploy("deploy-c2")
        out, record = self.root / "c2.sh", self.root / "c2.json"
        self.assertEqual(self.join_anchor(deploy, out, record, path).returncode, 0)
        self.assertEqual(subprocess.run(["sh", "-n", str(out)], capture_output=True).returncode, 0)
        statements = [line for line in out.read_text().splitlines() if line.startswith("CORTEX_ANCHOR_")]
        self.assertEqual(len(statements), 3)
        printed = subprocess.run(["sh", "-c", "\n".join(statements) + '\nprintf "%s|%s|%s" "$CORTEX_ANCHOR_PRINCIPAL" "$CORTEX_ANCHOR_KEY" "$CORTEX_ANCHOR_FPR"'],
                                 capture_output=True, text=True, check=True).stdout
        self.assertEqual(printed, f"{principal}|{key}|{self.independent_fpr(key)}")

    def test_anchor_c3_each_placeholder_defect_is_refused_with_its_own_cause_and_writes_nothing(self):
        path, _, _ = self.make_key("c3")
        names = self.ANCHOR_NAMES
        def lines(*parts):
            return "".join(parts)
        empty = {name: f"{name}=''\n" for name in names}
        cases = []
        for name in names:
            cases.append((f"{name} missing", dict(lines=lines(*(empty[n] for n in names if n != name))), f"release: installer anchor placeholder missing: {name}"))
            cases.append((f"{name} duplicated in the template", dict(body=self.ANCHOR_BODY + empty[name]), f"release: installer anchor placeholder duplicated: {name}"))
            cases.append((f"{name} already filled", dict(lines=lines(*((f"{n}='x'\n" if n == name else empty[n]) for n in names))), f"release: installer anchor placeholder already filled: {name}"))
        cases.append(("one carried by an include (a count of two after T1)", dict(fetch_extra=empty["CORTEX_ANCHOR_KEY"]), "release: installer anchor placeholder duplicated: CORTEX_ANCHOR_KEY"))
        cases.append(("the first two swapped", dict(lines=lines(empty[names[1]], empty[names[0]], empty[names[2]])), "release: installer anchor placeholders are out of order or not at the top"))
        cases.append(("the last two swapped", dict(lines=lines(empty[names[0]], empty[names[2]], empty[names[1]])), "release: installer anchor placeholders are out of order or not at the top"))
        cases.append(("a command between the header and the statements", dict(header=self.ANCHOR_HEADER + "set -eu\n"), "release: installer anchor placeholders are out of order or not at the top"))
        cases.append(("a command between two statements", dict(lines=empty[names[0]] + ":\n" + empty[names[1]] + empty[names[2]]), "release: installer anchor placeholders are out of order or not at the top"))
        cases.append(("the statements after the include markers", dict(lines="", body=self.ANCHOR_BODY + self.ANCHOR_LINES), "release: installer anchor placeholders are out of order or not at the top"))
        for number, (label, edit, cause) in enumerate(cases):
            with self.subTest(case=label):
                deploy = self.anchor_deploy(f"deploy-c3-{number}", **edit)
                out, record = self.root / f"c3-{number}.sh", self.root / f"c3-{number}.json"
                result = self.join_anchor(deploy, out, record, path)
                self.assertEqual(result.returncode, 1, (result.stdout, result.stderr))
                self.assertTrue(result.stderr.startswith(cause), result.stderr)
                self.assertNotIn("Traceback", result.stderr)
                self.assertFalse(out.exists() or record.exists())

    def test_anchor_c4_a_blank_or_comment_line_before_the_statements_is_still_the_header(self):
        path, principal, key = self.make_key("c4")
        deploy = self.anchor_deploy("deploy-c4", header="#!/bin/sh\n\n# one\n\n\n# two\n")
        out, record = self.root / "c4.sh", self.root / "c4.json"
        result = self.join_anchor(deploy, out, record, path)
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.strict_anchor_record(record, deploy, out, principal=principal, key=key, fpr=self.independent_fpr(key), key_bytes=path.read_bytes())

    # ---- D: the record and the two other forms of the join (contract sections 2 and 7) ----

    def test_anchor_d1_a_record_without_a_key_is_schema_1_and_the_statements_stay_empty(self):
        deploy = self.anchor_deploy("deploy-d1")
        out, record = self.root / "d1.sh", self.root / "d1.json"
        result = self.join_helper(deploy, out, record, "a" * 40, "b" * 40, "sha1")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertIn(self.ANCHOR_LINES.encode(), out.read_bytes())
        self.assertEqual(json.loads(record.read_text())["schema"], 1)
        self.assertEqual(list(json.loads(record.read_text())), self.RECORD_KEYS)

    def test_anchor_d2_a_key_without_a_record_anchors_the_installer_and_writes_no_record(self):
        path, principal, key = self.make_key("d2")
        deploy = self.anchor_deploy("deploy-d2")
        out = self.root / "d2.sh"
        result = self.join_anchor(deploy, out, None, path)
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertEqual(out.read_bytes(), self.expected_anchor_installer(deploy, principal, key, self.independent_fpr(key)))
        self.assertEqual([p.name for p in self.root.glob("d2*.json")], [])

    def test_anchor_d3_the_record_is_the_same_bytes_on_a_second_run_and_changes_with_the_key(self):
        first_key, _, _ = self.make_key("d3a")
        second_key, _, _ = self.make_key("d3b")
        deploy = self.anchor_deploy("deploy-d3")
        outputs = {}
        for label, key_file in (("one", first_key), ("again", first_key), ("other", second_key)):
            out, record = self.root / f"d3-{label}.sh", self.root / f"d3-{label}.json"
            result = self.join_anchor(deploy, out, record, key_file)
            self.assertEqual(result.returncode, 0, result.stderr)
            outputs[label] = (out.read_bytes(), record.read_bytes())
        self.assertEqual(outputs["one"], outputs["again"])
        self.assertNotEqual(outputs["one"][0], outputs["other"][0])
        self.assertNotEqual(outputs["one"][1], outputs["other"][1])
        self.assertNotEqual(json.loads(outputs["one"][1])["joined_sha256"], json.loads(outputs["other"][1])["joined_sha256"])

    # ---- E: release.sh (a real git-checked release needs the key; the key and the template are validated at the START) ----

    def anchored_release_root(self):
        """The fixture root as a real git work tree whose installer template carries the three statements, plus a key file outside the tree's tracked files."""
        self.valid_podman()
        return self.git_root()

    def test_anchor_e1_a_real_release_without_a_key_refuses_at_the_start_before_any_gate(self):
        self.anchored_release_root()
        result = self.run_release("1.2.3", no_key=True)
        self.assertEqual(result.returncode, 2, (result.stdout, result.stderr))
        self.assertIn("release: --trust-key-file is required for a real release", result.stderr)
        self.assertEqual(self.calls(), [])
        self.assertFalse((self.root / "dist/1.2.3").exists() or (self.root / "dist/1.2.3.part").exists())
        self.assertFalse((self.root / "dist/1.2.3.install.sh.binding.json").exists())

    def test_anchor_e2_a_bad_key_or_a_bad_template_refuses_at_the_start_before_any_gate(self):
        self.anchored_release_root()
        key_file = self.anchor_key
        good = key_file.read_bytes()
        key_file.write_bytes(b"")
        result = self.run_release("1.2.3")
        self.assertEqual(result.returncode, 1, (result.stdout, result.stderr))
        self.assertIn("release: trust key file: the file is empty", result.stderr)
        self.assertEqual(self.calls(), [])
        key_file.write_bytes(good)
        template = (self.root / "deploy/install.sh").read_text()
        (self.root / "deploy/install.sh").write_text(template.replace("CORTEX_ANCHOR_KEY=''\n", ""))
        self.git("commit", "-q", "-am", "template without the key statement")
        result = self.run_release("1.2.3")
        self.assertEqual(result.returncode, 1, (result.stdout, result.stderr))
        self.assertIn("release: installer anchor placeholder missing: CORTEX_ANCHOR_KEY", result.stderr)
        self.assertEqual(self.calls(), [])
        self.assertFalse((self.root / "dist/1.2.3").exists() or (self.root / "dist/1.2.3.part").exists())

    def test_anchor_e3_a_real_release_publishes_the_anchored_installer_and_the_schema_2_record_and_nothing_else(self):
        head = self.anchored_release_root()
        tree = self.git("rev-parse", "HEAD^{tree}")
        fmt = self.git("rev-parse", "--show-object-format")
        result = self.run_release("1.2.3")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        key_bytes = self.anchor_key.read_bytes()
        principal, _, key = key_bytes.decode().split()
        record = self.root / "dist/1.2.3.install.sh.binding.json"
        self.assertTrue(record.is_file() and not record.is_symlink())
        self.assertEqual(os.stat(record).st_mode & 0o077, 0)
        self.strict_anchor_record(record, self.root / "deploy", self.root / "dist/1.2.3/install.sh", principal=principal, key=key.strip(),
                                  fpr=self.independent_fpr(key.strip()), key_bytes=key_bytes, commit=head, tree=tree, fmt=fmt)
        self.assertEqual({p.name for p in (self.root / "dist/1.2.3").iterdir()},
                         {"cortex-1.2.3-x86_64-linux.tar.gz", "cortex-server-1.2.3-x86_64.oci.tar", "install.sh", "manifest.json", "install.meta", "SHA256SUMS"})
        self.assertEqual({p.name for p in (self.root / "dist").iterdir()}, {"1.2.3", "1.2.3.install.sh.binding.json"})
        sums = (self.root / "dist/1.2.3/SHA256SUMS").read_text()
        self.assertNotIn("binding", sums)
        self.assertNotIn("anchor", sums)
        self.assertEqual(sorted(p.name for p in (self.root / "dist").rglob("*") if "anchor" in p.name.lower()), [])

    def test_anchor_e4_a_seam_release_needs_no_key_and_writes_no_record_with_or_without_one(self):
        self.valid_podman()
        keyless = self.run_release("1.2.3")
        self.assertEqual(keyless.returncode, 0, (keyless.stdout, keyless.stderr))
        self.assertFalse((self.root / "dist/1.2.3.install.sh.binding.json").exists())
        self.assertIn("no binding record", keyless.stderr)
        shutil.rmtree(self.root / "dist")
        path, principal, key = self.make_key("e4")
        (self.root / "deploy/install.sh").write_text(self.ANCHOR_HEADER + self.ANCHOR_LINES + self.ANCHOR_BODY)
        self.log.unlink(missing_ok=True)
        keyed = self.run_release("1.2.3", "--trust-key-file", str(path))
        self.assertEqual(keyed.returncode, 0, (keyed.stdout, keyed.stderr))
        self.assertFalse((self.root / "dist/1.2.3.install.sh.binding.json").exists())
        self.assertIn("no binding record", keyed.stderr)
        installed = (self.root / "dist/1.2.3/install.sh").read_text()
        self.assertIn(f"CORTEX_ANCHOR_KEY='{key}'\n", installed)
        self.assertIn(f"CORTEX_ANCHOR_PRINCIPAL='{principal}'\n", installed)

    def test_anchor_e5_the_trust_key_file_option_is_checked_and_no_environment_variable_stands_in_for_it(self):
        self.anchored_release_root()
        path = self.anchor_key
        other, _, _ = self.make_key("e5-other", "someone-else@fixture")
        cases = [("no value", ["--trust-key-file"], "release: --trust-key-file needs a value"),
                 ("a relative path", ["--trust-key-file", "keys/release.txt"], "release: --trust-key-file must be an absolute path"),
                 ("a path with a newline", ["--trust-key-file", str(path) + "\n"], "release: --trust-key-file must be an absolute path"),
                 ("given twice", ["--trust-key-file", str(path), "--trust-key-file", str(path)], "release: --trust-key-file given twice")]
        for label, args, cause in cases:
            with self.subTest(case=label):
                self.log.unlink(missing_ok=True)
                result = self.run_release("1.2.3", *args, no_key=True)
                self.assertEqual(result.returncode, 2, (result.stdout, result.stderr))
                self.assertIn(cause, result.stderr)
                self.assertEqual(self.calls(), [])
        with self.subTest(case="an environment variable does not stand in for the option"):
            env = {"CORTEX_TRUST_KEY_FILE": str(path), "CORTEX_ANCHOR_KEY_FILE": str(path), "CORTEX_APPROVED_KEY_FILE": str(path)}
            result = self.run_release("1.2.3", no_key=True, **env)
            self.assertEqual(result.returncode, 2, (result.stdout, result.stderr))
            self.assertIn("release: --trust-key-file is required for a real release", result.stderr)
            self.assertEqual(self.calls(), [])
        with self.subTest(case="the option wins over an environment variable naming another key"):
            result = self.run_release("1.2.3", CORTEX_TRUST_KEY_FILE=str(other))
            self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
            record = json.loads((self.root / "dist/1.2.3.install.sh.binding.json").read_text())
            self.assertEqual(record["anchor"]["principal"], path.read_text().split()[0])

    # ---- F: nothing is published on any failure (contract section 9) ----

    def test_anchor_f1_every_failure_of_the_join_path_leaves_nothing_published_and_no_work_directory(self):
        self.anchored_release_root()
        key_file = self.anchor_key
        good = key_file.read_bytes()
        failures = [("an empty key file", lambda: key_file.write_bytes(b""), {}),
                    ("a key of another type", lambda: key_file.write_text("a@b ssh-rsa " + "A" * 68 + "\n"), {}),
                    ("a failing image build", lambda: None, {"RELEASE_TEST_FAIL": "podman-build"})]
        for label, edit, env in failures:
            with self.subTest(case=label):
                self.log.unlink(missing_ok=True)
                key_file.write_bytes(good)
                edit()
                result = self.run_release("1.2.3", **env)
                self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))
                dist = self.root / "dist"
                leftovers = sorted(p.name for p in dist.rglob("*")) if dist.exists() else []
                self.assertEqual([n for n in leftovers if n.startswith("1.2.3")], [], leftovers)
                self.assertEqual([n for n in leftovers if "binding" in n or "anchor" in n], [])
                self.assertEqual(sorted(p.name for p in (self.root / "target").glob("release-work.*")) if (self.root / "target").exists() else [], [])

    # ---- G: the key file is an input that must not move under a long build (contract section 5): hashed at the start, in the join and at the end ----

    def hook_stub(self, name, body):
        """Wrap a fixture stub: run `body` (shell) when the stub is called, then the original stub."""
        original = self.root / "bin" / name
        real = self.root / "bin" / (name + "-real")
        original.rename(real)
        original.write_text(f'#!/bin/sh\n{body}\nexec "{real}" "$@"\n')
        original.chmod(0o755)
        self.git("add", "-A")
        self.git("commit", "-q", "-m", "hook stub " + name)

    def test_anchor_g1_a_key_file_replaced_during_the_gates_refuses_and_publishes_nothing(self):
        self.anchored_release_root()
        replacement, _, _ = self.make_key("g1-other", "someone-else@fixture")
        self.hook_stub("ci", f'cp "{replacement}" "{self.anchor_key}"')
        result = self.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertIn("release: the trust key file changed during the release", result.stderr)
        self.assertNotIn("podman build", self.calls(), "the change must be caught at the join, before the image build, not only at the end")
        self.assertFalse((self.root / "dist/1.2.3").exists() or (self.root / "dist/1.2.3.part").exists())
        self.assertFalse((self.root / "dist/1.2.3.install.sh.binding.json").exists())

    def test_anchor_g2_a_key_file_replaced_after_the_join_refuses_at_the_end_and_publishes_nothing(self):
        self.anchored_release_root()
        replacement, _, _ = self.make_key("g2-other", "someone-else@fixture")
        self.hook_stub("podman", f'[ "$1" != build ] || cp "{replacement}" "{self.anchor_key}"')
        result = self.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertIn("release: the trust key file changed during the release", result.stderr)
        self.assertIn("podman build", self.calls())
        self.assertFalse((self.root / "dist/1.2.3").exists() or (self.root / "dist/1.2.3.part").exists())
        self.assertFalse((self.root / "dist/1.2.3.install.sh.binding.json").exists())

    def test_anchor_g3_rewriting_the_key_file_with_the_same_bytes_is_not_a_change(self):
        self.anchored_release_root()
        self.hook_stub("ci", f'cp "{self.anchor_key}" "{self.root}/key-copy" && cat "{self.root}/key-copy" > "{self.anchor_key}"')
        result = self.run_release("1.2.3")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertTrue((self.root / "dist/1.2.3.install.sh.binding.json").is_file())

    def after_join_replacement(self, shell, timeout=30):
        """Run a REAL release whose podman build step (after the join, before the end re-read) runs `shell` on the key file; bounded: a release that blocks is
        killed (its whole process group) and the cell fails. Returns (status, stdout, stderr)."""
        self.anchored_release_root()
        shell = shell.replace("__KEY__", str(self.anchor_key))
        self.hook_stub("podman", f'[ "$1" != build ] || {{ {shell}; }}')
        process = subprocess.Popen(["sh", str(self.root / "scripts/release.sh"), "1.2.3", "--trust-key-file", str(self.anchor_key)], cwd=self.root,
                                   env=self.env, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
        try:
            stdout, stderr = process.communicate(timeout=timeout)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGTERM)
            process.communicate()
            self.fail("the final re-read of the key file BLOCKED: no refusal within %d seconds" % timeout)
        return process.returncode, stdout, stderr

    def assert_key_change_refused_and_nothing_published(self, outcome, label):
        status, stdout, stderr = outcome
        self.assertNotEqual(status, 0, (label, stdout, stderr))
        self.assertIn("release: the trust key file changed during the release", stderr, label)
        self.assertFalse((self.root / "dist/1.2.3").exists() or (self.root / "dist/1.2.3.part").exists(), label)
        self.assertFalse((self.root / "dist/1.2.3.install.sh.binding.json").exists(), label)

    def test_anchor_g4_a_key_path_replaced_after_the_join_by_a_fifo_refuses_and_never_blocks(self):
        # sam TAJ-SAM01: the end re-read was a plain sha256sum of the path: a FIFO there blocks the release forever
        outcome = self.after_join_replacement('rm -f "__KEY__"; mkfifo "__KEY__"')
        self.assert_key_change_refused_and_nothing_published(outcome, "a FIFO")

    def test_anchor_g5_a_key_path_replaced_after_the_join_by_an_oversized_file_refuses(self):
        outcome = self.after_join_replacement('head -c 5000 /dev/zero | tr "\\0" A > "__KEY__"')
        self.assert_key_change_refused_and_nothing_published(outcome, "an oversized file")

    def test_anchor_g6_a_key_path_replaced_after_the_join_by_a_link_to_the_same_bytes_refuses(self):
        # sam TAJ-SAM01: the plain re-read followed the link, so a link to a file with the very bytes of the key looked unchanged
        outcome = self.after_join_replacement('cp "__KEY__" "__KEY__.copy"; rm -f "__KEY__"; ln -s "__KEY__.copy" "__KEY__"')
        self.assert_key_change_refused_and_nothing_published(outcome, "a link to the same bytes")

    def test_anchor_g7_a_key_path_replaced_after_the_join_by_a_directory_refuses(self):
        outcome = self.after_join_replacement('rm -f "__KEY__"; mkdir "__KEY__"')
        self.assert_key_change_refused_and_nothing_published(outcome, "a directory")

    def test_anchor_a6_the_final_hash_command_uses_the_one_checked_bounded_read(self):
        """sam TAJ-SAM01: the end re-read must be the SAME checked, bounded, no-follow read as the start and the join; `trust-key-hash` prints the hash of the buffer
        that read returns (the very value check-anchor and the join print) and refuses, fast and with no output, a path that is not a plain small key file."""
        helper = str(ROOT / "scripts/release_manifest.py")
        def run(path, timeout=15):
            process = subprocess.Popen([sys.executable, "-I", helper, "trust-key-hash", "--trust-key-file", str(path)], text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
            try:
                out, err = process.communicate(timeout=timeout)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGTERM)
                process.communicate()
                self.fail("trust-key-hash blocked on " + str(path))
            return process.returncode, out, err
        key, _, _ = self.make_key("a6")
        with self.subTest(case="a plain key file: its hash, exit 0"):
            status, out, err = run(key)
            self.assertEqual((status, err), (0, ""), (out, err))
            self.assertEqual(out.strip(), hashlib.sha256(Path(key).read_bytes()).hexdigest())
        broken = {"a FIFO": lambda p: os.mkfifo(p), "a symbolic link to the key": lambda p: p.symlink_to(key), "an oversized file": lambda p: p.write_bytes(b"A" * 5000),
                  "a directory": lambda p: p.mkdir(), "a missing path": lambda p: None}
        causes = {"a FIFO": "is not a regular file", "a symbolic link to the key": "(a symbolic link)", "an oversized file": "is larger than 1024 bytes",
                  "a directory": "is not a regular file", "a missing path": "cannot be read"}
        for label, make in broken.items():
            with self.subTest(case=label):
                target = self.root / ("a6-" + label.replace(" ", "-"))
                make(target)
                status, out, err = run(target)
                self.assertNotEqual(status, 0, (label, out, err))
                self.assertEqual(out, "", (label, err))
                self.assertIn("trust key file", err, label)
                self.assertIn(causes[label], err, label)   # each refusal names ITS cause (the cap, the link, the file kind), never a later format error
                self.assertNotIn("Traceback", err, label)

    # ---- I: the other end digests of release.sh are bounded, no-follow reads too (sam TAJ-SAM01's class; cox's ruling on O-1: tree_digest, the bin members and the feature file) ----

    def bounded_release(self, stub, shell, timeout=30):
        """Run a REAL release whose `stub` (a fixture gate: ci, s5, squawk or podman's build) runs `shell` first; bounded: a release that blocks is killed with its whole
        process group and the cell fails. __ROOT__ in the shell is the fixture root. Returns (status, stdout, stderr)."""
        self.anchored_release_root()
        shell = shell.replace("__ROOT__", str(self.root))
        condition = '[ "$1" != build ] || ' if stub == "podman" else ""
        self.hook_stub(stub, f'{condition}{{ {shell}; }}')
        process = subprocess.Popen(["sh", str(self.root / "scripts/release.sh"), "1.2.3", "--trust-key-file", str(self.anchor_key)], cwd=self.root,
                                   env=self.env, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
        try:
            stdout, stderr = process.communicate(timeout=timeout)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGTERM)
            process.communicate()
            self.fail("the release BLOCKED: no refusal within %d seconds (a plain read of a path that was replaced)" % timeout)
        return process.returncode, stdout, stderr

    def assert_refused_and_nothing_published(self, outcome, message, label):
        status, stdout, stderr = outcome
        self.assertNotEqual(status, 0, (label, stdout, stderr))
        self.assertIn(message, stderr, label)
        self.assertFalse((self.root / "dist/1.2.3").exists() or (self.root / "dist/1.2.3.part").exists(), label)

    def test_anchor_i1_a_link_to_a_fifo_planted_in_deploy_after_the_last_clean_check_refuses_at_the_end_and_never_blocks(self):
        # cox O-1: tree_digest listed -type l entries and a plain sha256sum FOLLOWED the link: a link to a FIFO blocked the end digest forever
        outcome = self.bounded_release("podman", 'mkfifo "__ROOT__/fifo-target" && ln -s "__ROOT__/fifo-target" "__ROOT__/deploy/zz-planted-link"')
        self.assert_refused_and_nothing_published(outcome, "release: a deploy file changed during the release", "a link to a FIFO in deploy")

    def test_anchor_i2_a_link_to_a_regular_file_planted_in_deploy_after_the_last_clean_check_refuses_at_the_end(self):
        outcome = self.bounded_release("podman", 'echo x > "__ROOT__/plain-target" && ln -s "__ROOT__/plain-target" "__ROOT__/deploy/zz-planted-link"')
        self.assert_refused_and_nothing_published(outcome, "release: a deploy file changed during the release", "a link to a regular file in deploy")

    def test_anchor_i3_a_bin_member_replaced_by_a_fifo_after_the_start_refuses_at_the_end_and_never_blocks(self):
        outcome = self.bounded_release("podman", 'rm -f "__ROOT__/scripts/cortex-dump" && mkfifo "__ROOT__/scripts/cortex-dump"')
        self.assert_refused_and_nothing_published(outcome, "release: a bin script changed during the release", "a bin member replaced by a FIFO")

    def test_anchor_i4_the_server_feature_file_replaced_by_a_fifo_before_the_image_build_refuses_and_never_blocks(self):
        outcome = self.bounded_release("s5", 'rm -f "__ROOT__/deploy/server-features" && mkfifo "__ROOT__/deploy/server-features"')
        self.assert_refused_and_nothing_published(outcome, "release: server feature file changed before image build", "the feature file replaced by a FIFO")

    def test_anchor_i5_a_deploy_file_over_the_bound_is_refused_not_silently_hashed_whole(self):
        self.anchored_release_root()
        big = self.root / "deploy/zz-too-big.bin"
        with open(big, "wb") as handle:
            handle.truncate(70 * 1024 * 1024)
        self.git("add", "-A")
        self.git("commit", "-q", "-m", "an oversized deploy file")
        result = self.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertIn("larger than", result.stderr)
        self.assertFalse((self.root / "dist/1.2.3").exists() or (self.root / "dist/1.2.3.part").exists())

    def squawk_release(self, plant, timeout=30):
        """A REAL release with the squawk seam OFF, so gate 4 digests the pinned path tools/squawk/2.71.0/squawk-linux-musl-x64 (that path is excluded from the clean checks);
        `plant(path)` makes what is at that path (made executable by the caller where it should pass `[ -x ]`). Bounded: a release that blocks is killed with its group."""
        self.anchored_release_root()
        path = self.root / "tools/squawk/2.71.0/squawk-linux-musl-x64"
        path.parent.mkdir(parents=True)
        plant(path)
        env = {key: value for key, value in self.env.items() if key != "CORTEX_RELEASE_SQUAWK"}
        process = subprocess.Popen(["sh", str(self.root / "scripts/release.sh"), "1.2.3", "--trust-key-file", str(self.anchor_key)], cwd=self.root,
                                   env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
        try:
            stdout, stderr = process.communicate(timeout=timeout)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGTERM)
            process.communicate()
            self.fail("the release BLOCKED at the squawk digest: no refusal within %d seconds (a plain sha256sum of a FIFO)" % timeout)
        return process.returncode, stdout, stderr

    def test_anchor_j1_a_fifo_at_the_pinned_squawk_path_is_refused_and_never_blocks(self):
        # the same class as O-1: gate 4 ran a plain sha256sum of this path; a FIFO with the execute bit passes [ -x ] and blocked the release for ever
        def plant(path):
            os.mkfifo(path, 0o755)
        outcome = self.squawk_release(plant)
        self.assert_refused_and_nothing_published(outcome, "release: squawk digest mismatch", "a FIFO at the squawk path")
        self.assertIn("is not a regular file", outcome[2])

    def test_anchor_j2_a_regular_file_with_the_wrong_digest_at_the_pinned_squawk_path_is_refused(self):
        # the control: the pinned digest is compared, and a regular executable with other bytes is refused (there was no cell for this line at all)
        def plant(path):
            path.write_text("not squawk\n")
            path.chmod(0o755)
        outcome = self.squawk_release(plant)
        self.assert_refused_and_nothing_published(outcome, "release: squawk digest mismatch", "a regular file with other bytes")
        self.assertNotIn("digest:", outcome[2])

    def test_anchor_j3_a_link_at_the_pinned_squawk_path_is_refused_not_followed(self):
        def plant(path):
            real = self.root / "target/squawk-elsewhere"   # under target/, which the clean checks exclude: the cell must fail for the link, not for an untracked file
            real.parent.mkdir(exist_ok=True)
            real.write_text("not squawk\n")
            real.chmod(0o755)
            path.symlink_to(real)
        outcome = self.squawk_release(plant)
        self.assert_refused_and_nothing_published(outcome, "release: squawk digest mismatch", "a link at the squawk path")
        self.assertIn("is a symbolic link", outcome[2])

    def test_anchor_j4_an_oversized_file_at_the_pinned_squawk_path_is_refused_not_hashed_whole(self):
        def plant(path):
            with open(path, "wb") as handle:
                handle.truncate(70 * 1024 * 1024)
            path.chmod(0o755)
        outcome = self.squawk_release(plant)
        self.assert_refused_and_nothing_published(outcome, "release: squawk digest mismatch", "an oversized file at the squawk path")
        self.assertIn("larger than", outcome[2])

    def test_anchor_a7_the_digest_commands_are_bounded_no_follow_reads(self):
        """The helper commands release.sh uses for its end digests: file-digest (the feature file), members-digest (the bin members) and tree-digest (deploy): a
        regular file is hashed through O_NOFOLLOW and O_NONBLOCK and a size bound; a FIFO, a link or an oversized file is refused fast and with no output; in a tree a
        link is RECORDED by its target text and never followed (so a link to a FIFO cannot block), and a FIFO entry is not listed (as find -type f -o -type l did not)."""
        helper = str(ROOT / "scripts/release_manifest.py")
        def run(*args, timeout=15):
            process = subprocess.Popen([sys.executable, "-I", helper, *args], text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
            try:
                out, err = process.communicate(timeout=timeout)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGTERM)
                process.communicate()
                self.fail("a digest command blocked: " + " ".join(args))
            return process.returncode, out, err
        plain = self.root / "a7-plain"
        plain.write_bytes(b"hello\n")
        with self.subTest(case="file-digest of a plain file"):
            status, out, err = run("file-digest", "--path", str(plain))
            self.assertEqual((status, err), (0, ""))
            self.assertEqual(out.strip(), hashlib.sha256(b"hello\n").hexdigest())
        broken = {"a FIFO": lambda path: os.mkfifo(path), "a link to the plain file": lambda path: path.symlink_to(plain), "an oversized file": lambda path: open(path, "wb").truncate(70 * 1024 * 1024),
                  "a directory": lambda path: path.mkdir(), "a missing path": lambda path: None}
        for label, make in broken.items():
            with self.subTest(case="file-digest of " + label):
                target = self.root / ("a7-" + label.replace(" ", "-"))
                make(target)
                status, out, err = run("file-digest", "--path", str(target))
                self.assertNotEqual(status, 0, (label, out, err))
                self.assertEqual(out, "", (label, err))
                self.assertNotIn("Traceback", err, label)
        tree = self.root / "a7-tree"
        (tree / "sub").mkdir(parents=True)
        (tree / "a.txt").write_text("a\n")
        (tree / "sub/b.txt").write_text("b\n")
        with self.subTest(case="tree-digest is stable and changes with a file"):
            first = run("tree-digest", "--dir", str(tree))
            self.assertEqual((first[0], first[2]), (0, ""))
            self.assertEqual(run("tree-digest", "--dir", str(tree))[1], first[1])
            (tree / "a.txt").write_text("changed\n")
            self.assertNotEqual(run("tree-digest", "--dir", str(tree))[1], first[1])
        with self.subTest(case="a link to a FIFO is recorded, never followed, and its target text is part of the digest"):
            os.mkfifo(self.root / "a7-fifo")
            (tree / "zz-link").symlink_to(self.root / "a7-fifo")
            with_fifo_link = run("tree-digest", "--dir", str(tree))
            self.assertEqual((with_fifo_link[0], with_fifo_link[2]), (0, ""))
            (tree / "zz-link").unlink()
            (tree / "zz-link").symlink_to(self.root / "a7-other")
            self.assertNotEqual(run("tree-digest", "--dir", str(tree))[1], with_fifo_link[1])
        with self.subTest(case="a FIFO entry in the tree is not listed"):
            (tree / "zz-link").unlink()
            before = run("tree-digest", "--dir", str(tree))
            os.mkfifo(tree / "zz-fifo")
            self.assertEqual(run("tree-digest", "--dir", str(tree))[1], before[1])
        with self.subTest(case="members-digest hashes the named members and refuses a FIFO member"):
            members = self.root / "a7-members"
            members.mkdir()
            (members / "m1").write_text("1\n")
            (members / "m2").write_text("2\n")
            ok = run("members-digest", "--dir", str(members), "m1", "m2")
            self.assertEqual((ok[0], ok[2]), (0, ""))
            os.unlink(members / "m2")
            os.mkfifo(members / "m2")
            bad = run("members-digest", "--dir", str(members), "m1", "m2")
            self.assertNotEqual(bad[0], 0)
            self.assertEqual(bad[1], "")

    # ---- H: a trappable signal during a gate leaves nothing (contract section 9) ----

    def test_anchor_h1_sigterm_during_a_gate_leaves_no_staging_no_record_and_no_work_directory(self):
        self.anchored_release_root()
        ready = self.root / "ci-started"
        self.hook_stub("ci", f'touch "{ready}"; sleep 30')
        process = subprocess.Popen(["sh", str(self.root / "scripts/release.sh"), "1.2.3", "--trust-key-file", str(self.anchor_key)], cwd=self.root,
                                   env=self.env, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
        try:
            for _ in range(200):
                if ready.exists():
                    break
                time.sleep(0.1)
            else:
                self.fail("the CI gate never started")
            os.killpg(process.pid, signal.SIGTERM)
            try:
                process.communicate(timeout=30)
            except subprocess.TimeoutExpired:
                self.fail("release.sh did not stop within 30 s of SIGTERM")
        finally:
            if process.poll() is None:
                os.killpg(process.pid, signal.SIGKILL)
                process.communicate()
        self.assertNotEqual(process.returncode, 0)
        dist = self.root / "dist"
        self.assertEqual([p.name for p in dist.rglob("*") if p.name.startswith("1.2.3")] if dist.exists() else [], [])
        self.assertEqual(sorted(p.name for p in (self.root / "target").glob("release-work.*")) if (self.root / "target").exists() else [], [])

    # ---- the exception LIST (cox b36a989b; plan exception-mechanism-PLAN-r0.md, accepted; vera's RULING.md option A: exact hash and finding approvals) ----
    # The platform entry stays byte for byte under the frozen matcher 1 (no "matcher" key); the new entries carry "matcher": 2 with complete identities.
    # The refusal causes below are the contract of the GREEN code. Fixture keys, rows and reviews are synthetic: nothing here approves anything.

    PLATFORM_PATH = "crates/cortex-db/migrations/platform/0001_platform.up.sql"
    # sha256 of the platform entry of the production list as canonical JSON (sort_keys, compact separators): it must never change (cox and vera: byte for byte)
    PLATFORM_ENTRY_SHA256 = "20ea9069a1ac4f5e23b921a075aafd930ce1587535525dae63ae62f2dab6cede"
    ROW_A, ROW_B, ROW_C = ("abcdef01-1234-4321-9876-abcdefabcdef", "abcdef02-1234-4321-9876-abcdefabcdef", "abcdef03-1234-4321-9876-abcdefabcdef")
    REVOKE_FIXTURE = ("-- fixture migration: an audit-like file\n"
                      "CREATE SCHEMA fx;\n"
                      "CREATE FUNCTION fx.f() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RETURN NEW; END $$;\n"
                      "CREATE TRIGGER fx_t BEFORE UPDATE ON fx.t FOR EACH ROW EXECUTE FUNCTION fx.f();\n"
                      "DO $$\n"
                      "DECLARE r record;\n"
                      "BEGIN\n"
                      "    FOR r IN SELECT 'PUBLIC' AS who LOOP\n"
                      "        EXECUTE format('REVOKE ALL ON SCHEMA fx FROM %s', r.who);\n"
                      "    END LOOP;\n"
                      "END $$;\n"
                      "-- a comment that says EXECUTE\n"
                      "REVOKE EXECUTE ON FUNCTION fx.f() FROM PUBLIC;\n")
    LITERAL_FIXTURE = ("-- fixture migration: a records-like file with two privilege labels on one line\n"
                       "CREATE SCHEMA gx;\n"
                       "DO $$\n"
                       "BEGIN\n"
                       "    PERFORM 1 FROM (VALUES ('f()', 'role', 'EXECUTE'), ('g()', 'role', 'EXECUTE')) AS v(a, b, c);\n"
                       "END $$;\n")

    def exc_helper(self):
        spec = importlib.util.spec_from_file_location("release_manifest_exc", ROOT / "scripts/release_manifest.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def own_scan(self, text):
        """The test's own scanner (independent of the helper): every raw EXECUTE word as (line, column), and those inside $$ ... $$ bodies or a quoted DO '...' body. Fixtures use one tag only."""
        words = [(text.count("\n", 0, m.start()) + 1, m.start() - (text.rfind("\n", 0, m.start()) + 1) + 1) for m in re.finditer(r"\bEXECUTE\b", text, re.IGNORECASE)]
        spans = [(m.start(1), m.end(1)) for m in re.finditer(r"\$\$(.*?)\$\$", text, re.DOTALL)]
        spans += [(m.start(1), m.end(1)) for m in re.finditer(r"\bDO\s+'((?:[^']|'')*)'", text)]
        inside = [(line, col) for (line, col), m in zip(words, re.finditer(r"\bEXECUTE\b", text, re.IGNORECASE)) if any(a <= m.start() < b for a, b in spans)]
        return words, inside

    def v2_entry(self, relative, row, kinds=None, squawk=None):
        """A matcher-2 entry computed from the file in the fixture root, by the test's own scanner; `squawk` is the list of Squawk approvals (default: none, a clean report is then required)."""
        text = (self.root / relative).read_text()
        words, inside = self.own_scan(text)
        lines = text.split("\n")
        findings = []
        for number, (line, col) in enumerate(inside):
            kind = (kinds or {}).get((line, col)) or ("execute_revoke" if "REVOKE" in lines[line - 1] else "privilege_literal_readback")
            findings.append({"line": line, "column": col, "statement_sha256": hashlib.sha256(lines[line - 1].strip().encode()).hexdigest(), "kind": kind,
                             "reason": "fixture reason " + str(number)})
        return {"path": relative, "sha256": hashlib.sha256((self.root / relative).read_bytes()).hexdigest(), "approved_by": row, "matcher": 2,
                "raw_execute_words": [{"line": l, "column": c} for l, c in words], "dynamic_findings": findings, "squawk": list(squawk or [])}

    def exc_fixture(self, with_platform=True, v2=(("crates/mod-fx/migrations/0001_fx.up.sql", REVOKE_FIXTURE),), order=None, squawk=None):
        """A root with the platform migration (the real bytes), the v2 fixture migrations, the schema-2 list, the schema-2 record and one synthetic review.
        `squawk` maps a migration path to its list of Squawk approvals."""
        self.valid_podman()
        entries, pins = [], []
        review = self.root / "ReviewService/example/REVIEW.md"
        review.parent.mkdir(parents=True, exist_ok=True)
        review.write_text("synthetic test review\n")
        review_pin = {"review_path": str(review.relative_to(self.root)), "review_sha256": hashlib.sha256(review.read_bytes()).hexdigest()}
        if with_platform:
            entry = json.loads((ROOT / "scripts/release-migration-exceptions.json").read_text())["exceptions"][0]
            entry["approved_by"] = self.ROW_A
            (self.root / self.PLATFORM_PATH).write_bytes((ROOT / self.PLATFORM_PATH).read_bytes())
            entries.append(entry)
            pins.append({"migration_path": entry["path"], "migration_sha256": entry["sha256"], "approved_by": self.ROW_A, **review_pin})
        for number, (relative, text) in enumerate(v2):
            (self.root / relative).parent.mkdir(parents=True, exist_ok=True)
            (self.root / relative).write_text(text)
            entry = self.v2_entry(relative, (self.ROW_B, self.ROW_C)[number % 2], squawk=(squawk or {}).get(relative))
            entries.append(entry)
            pins.append({"migration_path": relative, "migration_sha256": entry["sha256"], "approved_by": entry["approved_by"], **review_pin})
        if order:
            entries, pins = [entries[i] for i in order], [pins[i] for i in order]
        self.exc_write(entries, pins)
        return entries, pins

    def exc_write(self, entries, pins, schema=2, record_schema=2, config_sha=None):
        config = self.root / "scripts/release-migration-exceptions.json"
        config.write_text(json.dumps({"schema": schema, "exceptions": entries}, indent=1) + "\n")
        record = {"schema": record_schema, "config_sha256": config_sha or hashlib.sha256(config.read_bytes()).hexdigest(), "entries": pins}
        (self.root / "scripts/release-migration-approval.json").write_text(json.dumps(record, indent=1) + "\n")

    def exc_cli(self, *args):
        return subprocess.run([sys.executable, "-I", str(self.root / "scripts/release_manifest.py"), *args], text=True, capture_output=True, timeout=60)

    def exc_verify(self):
        return self.exc_cli("verify-approval", "--root", str(self.root))

    def exc_candidate(self, relative):
        return self.exc_cli("exception-candidate", "--root", str(self.root), "--migration", str(self.root / relative))

    # ---- A: the list verifies, each entry is found by its path, the platform entry is frozen ----

    def test_exc_a1_the_platform_entry_and_two_new_entries_verify_and_each_is_found_by_its_path(self):
        self.exc_fixture(v2=(("crates/mod-fx/migrations/0001_fx.up.sql", self.REVOKE_FIXTURE), ("crates/mod-gx/migrations/0001_gx.up.sql", self.LITERAL_FIXTURE)))
        verified = self.exc_verify()
        self.assertEqual(verified.returncode, 0, (verified.stdout, verified.stderr))
        expected = {self.PLATFORM_PATH: "baseline", "crates/mod-fx/migrations/0001_fx.up.sql": "clean", "crates/mod-gx/migrations/0001_gx.up.sql": "clean"}
        for relative, mode in expected.items():
            with self.subTest(path=relative):
                found = self.exc_candidate(relative)
                self.assertEqual(found.returncode, 0, (found.stdout, found.stderr))
                self.assertEqual(found.stdout.strip(), mode)
        other = self.root / "crates/mod-hx/migrations/0001_hx.up.sql"
        other.parent.mkdir(parents=True)
        other.write_text(self.REVOKE_FIXTURE)
        unlisted = self.exc_candidate("crates/mod-hx/migrations/0001_hx.up.sql")
        self.assertNotEqual(unlisted.returncode, 0, "an unlisted migration must not be a candidate")

    # VFR-PRE-01 (vera, accepted by cox; plan r1 ADDENDUM 1): the platform object is frozen by its ORIGINAL RAW BYTES. The canonical-JSON hash above proves parsed values only
    # (a re-serialization of the same object, 569 bytes, has the same canonical hash), so it is a supplement, never the check.
    PLATFORM_OBJECT_RAW_LEN = 521
    PLATFORM_OBJECT_RAW_SHA256 = "b10ac089a343dbc0d3b45c9fdcb7667294e637f56f4c569f8994171addfb7361"

    @staticmethod
    def exception_object_spans(raw):
        """The test's own finder (independent of the helper): the byte span (start, end) of every object that is a direct element of the top-level array of a list file,
        from its opening brace to its MATCHING closing brace, whitespace of the enclosing array excluded. String-aware: a brace or an escaped quote inside a string does not count."""
        spans, stack, start, in_string, escaped = [], [], None, False, False
        for i, byte in enumerate(raw):
            char = bytes([byte])
            if in_string:
                if escaped:
                    escaped = False
                elif char == b"\\":
                    escaped = True
                elif char == b'"':
                    in_string = False
            elif char == b'"':
                in_string = True
            elif char in (b"{", b"["):
                if char == b"{" and stack == [b"{", b"["]:
                    start = i
                stack.append(char)
            elif char in (b"}", b"]"):
                if stack:
                    stack.pop()
                if char == b"}" and stack == [b"{", b"["]:
                    spans.append((start, i + 1))
        return spans

    def platform_object_raw(self, raw):
        """The raw bytes of the ONE element whose parsed path is the platform path (None when there is not exactly one)."""
        found = []
        for start, end in self.exception_object_spans(raw):
            try:
                parsed = json.loads(raw[start:end])
            except ValueError:
                continue
            if isinstance(parsed, dict) and parsed.get("path") == self.PLATFORM_PATH:
                found.append((start, end))
        return found[0] if len(found) == 1 else None

    def platform_object_raw_ok(self, raw):
        """True only when the platform element is exactly the original 521 raw bytes (length and sha256)."""
        span = self.platform_object_raw(raw)
        if span is None:
            return False
        obj = raw[span[0]:span[1]]
        return len(obj) == self.PLATFORM_OBJECT_RAW_LEN and hashlib.sha256(obj).hexdigest() == self.PLATFORM_OBJECT_RAW_SHA256

    def test_exc_a2_the_platform_entry_of_the_production_list_is_byte_identical_to_the_frozen_one(self):
        raw = (ROOT / "scripts/release-migration-exceptions.json").read_bytes()
        span = self.platform_object_raw(raw)
        self.assertIsNotNone(span, "the production list must hold exactly one element for the platform path")
        obj = raw[span[0]:span[1]]
        self.assertEqual((len(obj), hashlib.sha256(obj).hexdigest()), (self.PLATFORM_OBJECT_RAW_LEN, self.PLATFORM_OBJECT_RAW_SHA256), "the platform object is not the original raw bytes")
        self.assertTrue(self.platform_object_raw_ok(raw))
        # supplement: the parsed values and the absence of a matcher key
        entry = json.loads(obj)
        self.assertEqual(hashlib.sha256(json.dumps(entry, sort_keys=True, separators=(",", ":")).encode()).hexdigest(), self.PLATFORM_ENTRY_SHA256)
        self.assertNotIn("matcher", entry)

    def test_exc_a2b_whitespace_only_changes_inside_the_platform_object_are_refused_and_outside_it_are_not(self):
        raw = (ROOT / "scripts/release-migration-exceptions.json").read_bytes()
        self.assertTrue(self.platform_object_raw_ok(raw))
        start, end = self.platform_object_raw(raw)
        obj = raw[start:end]
        canonical = lambda value: hashlib.sha256(json.dumps(json.loads(value), sort_keys=True, separators=(",", ":")).encode()).hexdigest()
        inside = {"one extra space after a colon": obj.replace(b'"path": ', b'"path":  ', 1), "the dynamic lines list re-spaced": obj.replace(b"[83, 91, 103, 104]", b"[83,  91, 103, 104]"),
                  "re-indented as json.dumps(indent=2)": json.dumps(json.loads(obj), indent=2).encode(), "the whole object on one line": json.dumps(json.loads(obj)).encode(),
                  "tabs for the indentation": obj.replace(b"\n      ", b"\n\t\t"), "CRLF line ends inside the object": obj.replace(b"\n", b"\r\n"),
                  "a trailing space before the closing brace": obj[:-1] + b" }"}
        for label, variant in inside.items():
            with self.subTest(case=label):
                self.assertNotEqual(variant, obj)
                self.assertEqual(canonical(variant), self.PLATFORM_ENTRY_SHA256, "the variant must keep the canonical hash (the r1 check would have passed it)")
                self.assertFalse(self.platform_object_raw_ok(raw[:start] + variant + raw[end:]), label)
        outside = {"the array indentation widened": raw[:start - 4] + b"        " + raw[start:], "a trailing newline added": raw + b"\n",
                   "the header re-spaced": raw.replace(b'"schema": ', b'"schema":  ', 1), "CRLF outside the object": raw[:start].replace(b"\n", b"\r\n") + obj + raw[end:].replace(b"\n", b"\r\n"),
                   "a second element after the platform object": raw[:end] + b',\n    {"path": "x"}' + raw[end:]}
        for label, changed in outside.items():
            with self.subTest(case=label):
                self.assertNotEqual(changed, raw)
                self.assertTrue(self.platform_object_raw_ok(changed), label)
        with self.subTest(case="two elements for the platform path are refused"):
            self.assertFalse(self.platform_object_raw_ok(raw[:end] + b",\n    " + obj + raw[end:]))
        with self.subTest(case="no element for the platform path is refused"):
            self.assertFalse(self.platform_object_raw_ok(b'{"schema": 2, "exceptions": []}'))

    def test_exc_a2c_the_span_finder_respects_strings_nesting_and_line_ends(self):
        def spans(raw):
            return [raw[a:b] for a, b in self.exception_object_spans(raw)]
        self.assertEqual(spans(b'{"exceptions": [{"a": "}{", "b": "\\"}"}, {"c": [1, {"d": 2}]}]}'), [b'{"a": "}{", "b": "\\"}"}', b'{"c": [1, {"d": 2}]}'])
        self.assertEqual(spans(b'{\r\n "exceptions": [\r\n  {\r\n "x": 1\r\n  }\r\n ]\r\n}'), [b'{\r\n "x": 1\r\n  }'])
        self.assertEqual(spans(b'{"exceptions": []}'), [])
        self.assertEqual(spans(b'{"exceptions": [{"a": 1},\n{"b": {"c": "x{y"}}]}'), [b'{"a": 1}', b'{"b": {"c": "x{y"}}'])

    def test_exc_a3_the_two_matcher_constants_are_in_one_place(self):
        helper = self.exc_helper()
        self.assertEqual(set(helper.EXCEPTION_KINDS), {"execute_revoke", "privilege_literal_readback"})
        self.assertEqual(set(helper.REVIEW_NAMES), {"REVIEW.md", "RULING.md"})

    def test_exc_a4_one_scanner_gives_the_raw_words_and_the_dynamic_findings_with_multiplicity(self):
        helper = self.exc_helper()
        for text in (self.REVOKE_FIXTURE, self.LITERAL_FIXTURE):
            words, inside = self.own_scan(text)
            self.assertEqual(helper.scan_execute_words(text), words)
            self.assertEqual(helper.scan_dynamic_findings(text), inside)
        literal_words, literal_inside = self.own_scan(self.LITERAL_FIXTURE)
        self.assertEqual(len(literal_inside), 2)
        self.assertEqual(literal_inside[0][0], literal_inside[1][0], "two tokens on one line stay two")

    # ---- B: STRICT ALL-ENTRY validation: every entry is checked against the tree before any release step ----

    def test_exc_b1_a_stale_entry_refuses_the_whole_release_before_any_gate_even_when_it_is_not_the_one_scanned(self):
        entries, _ = self.exc_fixture(v2=(("crates/mod-fx/migrations/0001_fx.up.sql", self.REVOKE_FIXTURE), ("crates/mod-gx/migrations/0001_gx.up.sql", self.LITERAL_FIXTURE)))
        stale = self.root / "crates/mod-gx/migrations/0001_gx.up.sql"
        stale.write_text(stale.read_text() + "-- a later edit\n")
        verified = self.exc_verify()
        self.assertNotEqual(verified.returncode, 0)
        self.assertIn("release: migration exception approval pins are invalid", verified.stderr)
        self.assertIn("sha256 differs", verified.stderr)
        result = self.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertEqual(self.calls(), [], "no gate may run when an approval is stale")
        self.assertFalse((self.root / "dist/1.2.3").exists())

    def test_exc_b2_each_defect_of_an_entry_refuses_with_its_own_cause(self):
        base = "crates/mod-gx/migrations/0001_gx.up.sql"
        def entry_edit(edit):
            def apply(entries):
                edit(entries[1])
            return apply
        def drop_finding(e): e["dynamic_findings"].pop()
        def add_finding(e): e["dynamic_findings"].append(dict(e["dynamic_findings"][0], line=1, column=1))
        def duplicate_finding(e): e["dynamic_findings"].append(dict(e["dynamic_findings"][0]))
        def second_on_line(e): e["dynamic_findings"].append(dict(e["dynamic_findings"][0], column=e["dynamic_findings"][0]["column"] + 1))
        def move_column(e): e["dynamic_findings"][0]["column"] += 1
        def change_statement(e): e["dynamic_findings"][0]["statement_sha256"] = "0" * 64
        def reduce_multiplicity(e): e["raw_execute_words"].pop()
        def add_word(e): e["raw_execute_words"].append({"line": 1, "column": 1})
        def drop_word(e): e["raw_execute_words"].pop(0)
        def duplicate_word(e): e["raw_execute_words"].append(dict(e["raw_execute_words"][0]))
        def change_hash(e): e["sha256"] = "0" * 64
        def bad_kind(e): e["dynamic_findings"][0]["kind"] = "execute_anything"
        def no_reason(e): del e["dynamic_findings"][0]["reason"]
        def empty_reason(e): e["dynamic_findings"][0]["reason"] = ""
        def float_matcher(e): e["matcher"] = 2.0
        def swap_kind(e): e["dynamic_findings"][0]["kind"] = "execute_revoke" if e["dynamic_findings"][0]["kind"] == "privilege_literal_readback" else "privilege_literal_readback"
        def unknown_field(e): e["extra"] = 1
        def unknown_matcher(e): e["matcher"] = 3
        def missing_squawk(e): del e["squawk"]
        def squawk_waiver(e): e["squawk"] = [{"rule": "require-lock-timeout", "line": 2}]
        def zero_row(e): e["approved_by"] = "00000000-0000-0000-0000-000000000000"
        def malformed_row(e): e["approved_by"] = "PENDING"
        cases = [("a finding dropped", drop_finding, "dynamic findings differ"), ("a finding added", add_finding, "dynamic findings differ"),
                 ("a finding duplicated", duplicate_finding, "dynamic findings differ"), ("a second finding on the same line", second_on_line, "dynamic findings differ"),
                 ("a column moved", move_column, "dynamic findings differ"), ("a statement hash changed", change_statement, "dynamic findings differ"),
                 ("the multiplicity of two tokens on one line reduced to one", reduce_multiplicity, "raw EXECUTE words differ"),
                 ("an inventory word added", add_word, "raw EXECUTE words differ"), ("an inventory word dropped", drop_word, "raw EXECUTE words differ"),
                 ("an inventory word duplicated (the same line and column twice)", duplicate_word, "raw EXECUTE words differ"),
                 ("the file hash changed", change_hash, "sha256 differs"), ("a kind outside the closed set", bad_kind, "kind"),
                 ("a reason missing", no_reason, "field"), ("an empty reason", empty_reason, "reason"), ("the matcher version as a float", float_matcher, "matcher"),
                 ("the kind swapped for the other member of the closed set", swap_kind, "kind"), ("an unknown field", unknown_field, "field"), ("an unknown matcher version", unknown_matcher, "matcher"),
                 ("the squawk list missing", missing_squawk, "field"), ("a squawk waiver of the old shape (no digests) on a matcher-2 entry", squawk_waiver, "Squawk approval"),
                 ("a zero approved_by", zero_row, "approved_by"), ("a malformed approved_by", malformed_row, "approved_by")]
        for label, edit, cause in cases:
            with self.subTest(case=label):
                shutil.rmtree(self.root / "crates/mod-fx", ignore_errors=True)
                shutil.rmtree(self.root / "crates/mod-gx", ignore_errors=True)
                entries, pins = self.exc_fixture(v2=(("crates/mod-fx/migrations/0001_fx.up.sql", self.REVOKE_FIXTURE), (base, self.LITERAL_FIXTURE)))
                edit(entries[2])
                pins[2]["approved_by"] = entries[2]["approved_by"] if isinstance(entries[2].get("approved_by"), str) else pins[2]["approved_by"]
                self.exc_write(entries, pins)
                verified = self.exc_verify()
                self.assertNotEqual(verified.returncode, 0, label)
                self.assertIn("release: migration exception approval pins are invalid", verified.stderr)
                self.assertIn(cause, verified.stderr, label)
                self.assertNotIn("Traceback", verified.stderr)

    QUOTED_FIXTURE = ("-- fixture migration: a quoted DO body (procedural, not a dollar quote)\n"
                      "CREATE SCHEMA qx;\n"
                      "DO '\n"
                      "BEGIN\n"
                      "    PERFORM 1 FROM (VALUES (''f()'', ''EXECUTE'')) AS v(a, b);\n"
                      "END';\n")
    NONASCII_FIXTURE = ("-- fixture migration: a non-ASCII character before the token\n"
                        "CREATE SCHEMA nx;\n"
                        "DO $$\n"
                        "BEGIN\n"
                        "    PERFORM 1 FROM (VALUES ('\u00e9\u00e9', 'EXECUTE')) AS v(a, b);\n"
                        "END $$;\n")

    def test_exc_b7_the_platform_entry_must_match_the_frozen_baseline(self):
        def lines_changed(e): e["dynamic_sql_lines"] = [83, 91, 103, 105]
        def squawk_extra(e): e["squawk"].append({"rule": "extra-rule", "line": 2})
        def squawk_dropped(e): e["squawk"].pop()
        def hash_changed(e): e["sha256"] = "1" * 64
        def other_path(e): e["path"] = "crates/cortex-db/migrations/platform/0002_platform.up.sql"
        def unknown_field(e): e["extra"] = 1
        def explicit_matcher(e): e["matcher"] = 1
        for label, edit in (("the dynamic lines changed", lines_changed), ("a fifth Squawk diagnostic", squawk_extra), ("a Squawk diagnostic dropped", squawk_dropped),
                            ("the hash changed", hash_changed), ("another path", other_path), ("an unknown field", unknown_field), ("an explicit matcher 1", explicit_matcher)):
            with self.subTest(case=label):
                shutil.rmtree(self.root / "crates/mod-fx", ignore_errors=True)
                entries, pins = self.exc_fixture()
                edit(entries[0])
                pins[0]["migration_path"], pins[0]["migration_sha256"] = entries[0]["path"], entries[0]["sha256"]
                self.exc_write(entries, pins)
                verified = self.exc_verify()
                self.assertNotEqual(verified.returncode, 0, label)
                self.assertIn("release: migration exception approval pins are invalid", verified.stderr)

    def test_exc_b8_quoted_procedural_bodies_are_findings_as_well_as_dollar_bodies(self):
        helper = self.exc_helper()
        words, inside = self.own_scan(self.QUOTED_FIXTURE)
        found = helper.scan_dynamic_findings(self.QUOTED_FIXTURE)
        self.assertEqual(len(found), 1, "the EXECUTE word inside a quoted DO body must be a dynamic finding")
        self.assertEqual(found, inside)
        self.assertEqual(found[0][0], 5)
        relative = "crates/mod-qx/migrations/0001_qx.up.sql"
        entries, pins = self.exc_fixture(v2=((relative, self.QUOTED_FIXTURE),))
        self.assertEqual(self.exc_verify().returncode, 0)
        with self.subTest(case="an entry that omits the quoted-body finding refuses"):
            entries[1]["dynamic_findings"] = []
            self.exc_write(entries, pins)
            verified = self.exc_verify()
            self.assertNotEqual(verified.returncode, 0)
            self.assertIn("dynamic findings differ", verified.stderr)
        with self.subTest(case="a quoted-body file with no entry is refused by the release"):
            shutil.rmtree(self.root / "crates/mod-qx", ignore_errors=True)
            self.exc_fixture(v2=())
            stranger = self.root / relative
            stranger.parent.mkdir(parents=True, exist_ok=True)
            stranger.write_text(self.QUOTED_FIXTURE)
            self.exc_squawk_stub()
            result = self.run_release("1.2.3")
            self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))
            self.assertIn("dynamic SQL requires review: 0001_qx.up.sql", result.stderr)

    def test_exc_b9_columns_are_characters_not_bytes(self):
        helper = self.exc_helper()
        words, inside = self.own_scan(self.NONASCII_FIXTURE)
        self.assertEqual(helper.scan_execute_words(self.NONASCII_FIXTURE), words)
        self.assertEqual(helper.scan_dynamic_findings(self.NONASCII_FIXTURE), inside)
        entries, pins = self.exc_fixture(v2=(("crates/mod-nx/migrations/0001_nx.up.sql", self.NONASCII_FIXTURE),))
        verified = self.exc_verify()
        self.assertEqual(verified.returncode, 0, (verified.stdout, verified.stderr))

    NOKIND_FIXTURES = (("an EXECUTE with no quote around it", "DO $$\nBEGIN\n    EXECUTE 'SELECT 1';\nEND $$;\n"),
                       ("a quote before the token only", "DO $$\nBEGIN\n    PERFORM 1 WHERE 'EXECUTE y' <> '';\nEND $$;\n"),
                       ("a quote after the token only", "DO $$\nBEGIN\n    PERFORM 1 WHERE 'y EXECUTE' <> '';\nEND $$;\n"),
                       ("a REVOKE string that is not the exact format form", "DO $$\nBEGIN\n    EXECUTE 'REVOKE ALL ON SCHEMA fx FROM PUBLIC';\nEND $$;\n"))

    def test_exc_b10_a_finding_with_no_approvable_kind_is_refused_whatever_kind_the_entry_names(self):
        helper = self.exc_helper()
        for label, text in self.NOKIND_FIXTURES:
            with self.subTest(case=label + ": computed kind"):
                found = helper.scan_dynamic_findings(text)
                self.assertEqual(len(found), 1)
                self.assertIsNone(helper.classify_finding(text.split("\n")[found[0][0] - 1], found[0][1]))
            for kind in (None, "execute_revoke", "privilege_literal_readback"):
                with self.subTest(case=label, entry_kind=kind):
                    shutil.rmtree(self.root / "crates/mod-kx", ignore_errors=True)
                    entries, pins = self.exc_fixture(v2=(("crates/mod-kx/migrations/0001_kx.up.sql", text),))
                    entries[1]["dynamic_findings"][0]["kind"] = kind
                    self.exc_write(entries, pins)
                    verified = self.exc_verify()
                    self.assertNotEqual(verified.returncode, 0, (label, kind))
                    self.assertIn("kind", verified.stderr)
                    self.assertNotIn("Traceback", verified.stderr)

    def test_exc_b11_the_frozen_matcher_guards_its_own_inputs(self):
        # the frozen platform matcher is also called directly: it must refuse on its own, not rely on the validator that calls it (RM-44, RM-45)
        helper = self.exc_helper()
        entries, _ = self.exc_fixture()
        entry = entries[0]
        platform = self.root / self.PLATFORM_PATH
        self.assertTrue(helper.exact_baseline_exception(self.root, platform, entry))
        with self.subTest(case="the bytes it reads differ from the approved bytes (a change between two reads) but keep the same EXECUTE lines"):
            class Changed:
                def read_bytes(self):
                    return platform.read_bytes() + b"\n-- changed after the validator read it\n"
            with mock.patch.object(helper, "regular", lambda path: Changed()):
                self.assertFalse(helper.exact_baseline_exception(self.root, platform, entry))
        with self.subTest(case="a byte-identical copy at another path is not the platform migration"):
            copy = self.root / "crates/mod-copy/migrations/0001_copy.up.sql"
            copy.parent.mkdir(parents=True, exist_ok=True)
            copy.write_bytes(platform.read_bytes())
            self.assertFalse(helper.exact_baseline_exception(self.root, copy, entry))

    def test_exc_b3_a_dynamic_migration_with_no_entry_is_refused_even_when_other_entries_are_valid(self):
        self.exc_fixture()
        self.exc_squawk_stub()
        stranger = self.root / "crates/mod-hx/migrations/0001_hx.up.sql"
        stranger.parent.mkdir(parents=True)
        stranger.write_text(self.REVOKE_FIXTURE)
        result = self.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertIn("dynamic SQL requires review: 0001_hx.up.sql", result.stderr)
        self.assertFalse((self.root / "dist/1.2.3").exists())

    def test_exc_b4_the_list_and_record_structure_refusals(self):
        def two_entries_one_path(entries, pins):
            entries.append(json.loads(json.dumps(entries[1])))
            pins.append(dict(pins[1]))
        def entry_not_object(entries, pins): entries[1] = "not an object"
        def path_dotdot(entries, pins):
            entries[1]["path"] = "crates/../crates/mod-fx/migrations/0001_fx.up.sql"
            pins[1]["migration_path"] = entries[1]["path"]
        def path_absolute(entries, pins):
            entries[1]["path"] = str(self.root / "crates/mod-fx/migrations/0001_fx.up.sql")
            pins[1]["migration_path"] = entries[1]["path"]
        cases = [("two entries for one path", two_entries_one_path, "duplicate entry for path"), ("an entry that is not an object", entry_not_object, "entry"),
                 ("a path with ..", path_dotdot, "path"), ("an absolute path", path_absolute, "path")]
        for label, edit, cause in cases:
            with self.subTest(case=label):
                shutil.rmtree(self.root / "crates/mod-fx", ignore_errors=True)
                entries, pins = self.exc_fixture()
                edit(entries, pins)
                self.exc_write(entries, pins)
                verified = self.exc_verify()
                self.assertNotEqual(verified.returncode, 0, label)
                self.assertIn(cause, verified.stderr, label)
        with self.subTest(case="a symlink component in an entry's path"):
            shutil.rmtree(self.root / "crates/mod-fx", ignore_errors=True)
            entries, pins = self.exc_fixture()
            real = self.root / "crates/mod-fx"
            moved = self.root / "elsewhere-fx"
            real.rename(moved)
            real.symlink_to(moved, target_is_directory=True)
            verified = self.exc_verify()
            self.assertNotEqual(verified.returncode, 0)
            self.assertIn("path", verified.stderr)
        for label, text in (("schema 1 with a matcher-2 entry", 1), ("an unknown top-level schema", 3)):
            with self.subTest(case=label):
                shutil.rmtree(self.root / "crates/mod-fx", ignore_errors=True)
                shutil.rmtree(self.root / "elsewhere-fx", ignore_errors=True)
                (self.root / "crates/mod-fx").unlink(missing_ok=True)
                entries, pins = self.exc_fixture()
                self.exc_write(entries, pins, schema=text)
                verified = self.exc_verify()
                self.assertNotEqual(verified.returncode, 0, label)
                self.assertIn("schema", verified.stderr, label)
        def reseal(config_text=None, record_edit=None):
            config = self.root / "scripts/release-migration-exceptions.json"
            if config_text is not None:
                config.write_text(config_text)
            record_path = self.root / "scripts/release-migration-approval.json"
            record = json.loads(record_path.read_text())
            record["config_sha256"] = hashlib.sha256(config.read_bytes()).hexdigest()
            if record_edit:
                record_edit(record)
            record_path.write_text(json.dumps(record, indent=1) + "\n")
            return config
        def fresh():
            shutil.rmtree(self.root / "crates/mod-fx", ignore_errors=True)
            shutil.rmtree(self.root / "elsewhere-fx", ignore_errors=True)
            (self.root / "crates/mod-fx").unlink(missing_ok=True)
            return self.exc_fixture()
        with self.subTest(case="a duplicate JSON key at the top of the list (the rest valid, the record sealed)"):
            fresh()
            config = self.root / "scripts/release-migration-exceptions.json"
            reseal(config.read_text().replace('"schema": 2,', '"schema": 2,\n "schema": 2,', 1))
            verified = self.exc_verify()
            self.assertNotEqual(verified.returncode, 0)
            self.assertIn("duplicate JSON key", verified.stderr)
        with self.subTest(case="a duplicate JSON key inside an entry"):
            fresh()
            config = self.root / "scripts/release-migration-exceptions.json"
            text = config.read_text()
            self.assertEqual(text.count('"matcher": 2,'), 1)
            reseal(text.replace('"matcher": 2,', '"matcher": 2,\n "matcher": 2,', 1))
            verified = self.exc_verify()
            self.assertNotEqual(verified.returncode, 0)
            self.assertIn("duplicate JSON key", verified.stderr)
        with self.subTest(case="a duplicate JSON key in the record"):
            fresh()
            record_path = self.root / "scripts/release-migration-approval.json"
            record_path.write_text(record_path.read_text().replace('"schema": 2,', '"schema": 2,\n "schema": 2,', 1))
            verified = self.exc_verify()
            self.assertNotEqual(verified.returncode, 0)
            self.assertIn("duplicate JSON key", verified.stderr)
        for label, value in (("a string", '""'), ("an object", "{}"), ("null", "null")):
            with self.subTest(case="the entries of the list are " + label + " (the record sealed with no pins)"):
                fresh()
                reseal('{"schema": 2, "exceptions": ' + value + '}\n', lambda record: record.update(entries=[]))
                verified = self.exc_verify()
                self.assertNotEqual(verified.returncode, 0)
                self.assertIn("release: migration exception approval pins are invalid", verified.stderr)

    def test_exc_b5_lookup_does_not_depend_on_order_and_a_bad_first_entry_unlocks_nothing(self):
        v2 = (("crates/mod-fx/migrations/0001_fx.up.sql", self.REVOKE_FIXTURE), ("crates/mod-gx/migrations/0001_gx.up.sql", self.LITERAL_FIXTURE))
        expected = {self.PLATFORM_PATH: "baseline", v2[0][0]: "clean", v2[1][0]: "clean"}
        for order in ((0, 1, 2), (2, 1, 0), (1, 2, 0), (1, 0, 2)):
            with self.subTest(order=order):
                for d in ("mod-fx", "mod-gx"):
                    shutil.rmtree(self.root / "crates" / d, ignore_errors=True)
                self.exc_fixture(v2=v2, order=order)
                self.assertEqual(self.exc_verify().returncode, 0)
                for relative, mode in expected.items():
                    found = self.exc_candidate(relative)
                    self.assertEqual((found.returncode, found.stdout.strip()), (0, mode), relative)
        with self.subTest(case="a bad first entry refuses the whole list"):
            for d in ("mod-fx", "mod-gx"):
                shutil.rmtree(self.root / "crates" / d, ignore_errors=True)
            entries, pins = self.exc_fixture(v2=v2)
            entries[0]["sha256"] = "0" * 64
            self.exc_write(entries, pins)
            self.assertNotEqual(self.exc_verify().returncode, 0)
            self.assertNotEqual(self.exc_candidate(v2[0][0]).returncode, 0)
        with self.subTest(case="a good entry at index 2 is found"):
            for d in ("mod-fx", "mod-gx"):
                shutil.rmtree(self.root / "crates" / d, ignore_errors=True)
            self.exc_fixture(v2=v2)
            found = self.exc_candidate(v2[1][0])
            self.assertEqual((found.returncode, found.stdout.strip()), (0, "clean"))

    def test_exc_b6_an_empty_list_is_valid_and_waives_nothing(self):
        self.exc_fixture(with_platform=False, v2=())
        self.assertEqual(self.exc_verify().returncode, 0)
        stranger = self.root / "crates/mod-hx/migrations/0001_hx.up.sql"
        stranger.parent.mkdir(parents=True)
        stranger.write_text(self.REVOKE_FIXTURE)
        self.assertNotEqual(self.exc_candidate("crates/mod-hx/migrations/0001_hx.up.sql").returncode, 0)
        result = self.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertIn("dynamic SQL requires review", result.stderr)
        with self.subTest(case="a record for a non-empty list cannot validate an empty one"):
            entries, pins = self.exc_fixture()
            self.exc_write([], pins)
            self.assertNotEqual(self.exc_verify().returncode, 0)
        with self.subTest(case="a record with entries cannot validate an empty config"):
            shutil.rmtree(self.root / "crates/mod-fx", ignore_errors=True)
            entries, pins = self.exc_fixture()
            self.exc_write([], [pins[0]])
            self.assertNotEqual(self.exc_verify().returncode, 0)

    # ---- C: release.sh against the list: the approved new entries pass both gates, squawk is NOT waived for them, the platform baseline is unchanged ----

    def exc_squawk_stub(self, platform_exit=1, platform_report=None, other_exit=0, other_report="[]"):
        """A squawk stand-in that answers per migration (the migration is the last argument): the platform file gets the approved four unless told otherwise."""
        platform = self.root / self.PLATFORM_PATH
        if platform_report is None:
            platform_report = json.dumps([{"file": str(platform), "rule_name": rule, "line": line} for rule, line in
                                          (("require-lock-timeout", 2), ("require-statement-timeout", 2), ("prefer-bigint-over-int", 7), ("prefer-bigint-over-int", 34))])
        stub = self.root / "bin/squawk"
        stub.write_text('#!/bin/sh\nprintf "%s\\n" squawk >> "$RELEASE_TEST_LOG"\nfor a in "$@"; do last=$a; done\ncase $last in\n'
                        f"  *0001_platform.up.sql) cat <<'JSON'\n{platform_report}\nJSON\n    exit {platform_exit} ;;\n"
                        f"  *) cat <<'JSON'\n{other_report}\nJSON\n    exit {other_exit} ;;\nesac\n")
        stub.chmod(0o755)

    def test_exc_c1_the_approved_new_entries_pass_both_migration_gates_with_a_clean_squawk(self):
        self.exc_fixture(v2=(("crates/mod-fx/migrations/0001_fx.up.sql", self.REVOKE_FIXTURE), ("crates/mod-gx/migrations/0001_gx.up.sql", self.LITERAL_FIXTURE)))
        self.exc_squawk_stub()
        result = self.run_release("1.2.3")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertTrue((self.root / "dist/1.2.3").is_dir())
        self.assertEqual(self.calls().count("squawk"), 3)

    def test_exc_c2_a_new_entry_gets_no_squawk_waiver_whatever_the_tool_says(self):
        fx = self.root / "crates/mod-fx/migrations/0001_fx.up.sql"
        finding = json.dumps([{"file": str(fx), "rule_name": "require-lock-timeout", "line": 2}])
        platform_four = lambda: json.dumps([{"file": str(fx), "rule_name": rule, "line": line} for rule, line in
                                            (("require-lock-timeout", 2), ("require-statement-timeout", 2), ("prefer-bigint-over-int", 7), ("prefer-bigint-over-int", 34))])
        cases = [("status 1 with an empty report", 1, "[]"), ("status 2 (a tool error)", 2, "[]"), ("a finding with status 1", 1, finding),
                 ("a finding with status 0", 0, finding), ("a malformed report with status 0", 0, "{not json"), ("a truncated report with status 0", 0, '[{"file": '),
                 ("a report that is not a list", 0, '{"findings": []}'), ("the platform's four findings reported for the new file", 1, platform_four())]
        for label, status, report in cases:
            with self.subTest(case=label):
                shutil.rmtree(self.root / "dist", ignore_errors=True)
                shutil.rmtree(self.root / "crates/mod-fx", ignore_errors=True)
                self.log.unlink(missing_ok=True)
                self.exc_fixture()
                self.exc_squawk_stub(other_exit=status, other_report=report)
                result = self.run_release("1.2.3")
                self.assertNotEqual(result.returncode, 0, (label, result.stdout, result.stderr))
                self.assertIn("release: migration gate failed", result.stderr, label)
                self.assertFalse((self.root / "dist/1.2.3").exists())

    def test_exc_c3_the_platform_baseline_still_needs_exactly_its_four_findings(self):
        cases = [("an empty report with status 0 cannot replace the four", 0, "[]"), ("an empty report with status 1", 1, "[]"), ("status 2", 2, "[]"),
                 ("a malformed report", 1, "{not json")]
        for label, status, report in cases:
            with self.subTest(case=label):
                shutil.rmtree(self.root / "dist", ignore_errors=True)
                shutil.rmtree(self.root / "crates/mod-fx", ignore_errors=True)
                self.log.unlink(missing_ok=True)
                self.exc_fixture()
                self.exc_squawk_stub(platform_exit=status, platform_report=report)
                result = self.run_release("1.2.3")
                self.assertNotEqual(result.returncode, 0, (label, result.stdout, result.stderr))
                self.assertIn("approved baseline findings differ", result.stderr, label)
                self.assertFalse((self.root / "dist/1.2.3").exists())

    def test_exc_c4_the_expand_only_refusals_still_apply_to_an_approved_file(self):
        relative = "crates/mod-fx/migrations/0001_fx.up.sql"
        self.exc_fixture(v2=((relative, self.REVOKE_FIXTURE + "ALTER TABLE fx.t DROP COLUMN c;\n"),))
        self.exc_squawk_stub()
        result = self.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertIn("requires contract", result.stderr)
        self.assertFalse((self.root / "dist/1.2.3").exists())

    def test_exc_c5_a_list_that_is_stale_refuses_before_any_gate(self):
        self.exc_fixture()
        (self.root / "crates/mod-fx/migrations/0001_fx.up.sql").write_text(self.REVOKE_FIXTURE + "-- edited\n")
        result = self.run_release("1.2.3")
        self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertEqual(self.calls(), [])

    def test_exc_c6_a_zero_status_candidate_answer_other_than_baseline_or_clean_waives_nothing(self):
        # hale X-1: release.sh must set the dynamic-SQL waiver only for the two words the helper may print; the helper is replaced by a shim that answers
        # exception-candidate for ONE unlisted migration with status 0 and another word (or nothing) and delegates everything else to the real helper
        real_bytes = (ROOT / "scripts/release_manifest.py").read_bytes()
        for label, answer in (("an unknown word", "other\n"), ("an empty answer", ""), ("a word that only starts like a mode", "clean2\n"), ("a word that only starts like findings", "findings2\n")):
            with self.subTest(case=label):
                shutil.rmtree(self.root / "dist", ignore_errors=True)
                shutil.rmtree(self.root / "crates/mod-hx", ignore_errors=True)
                self.log.unlink(missing_ok=True)
                self.exc_fixture()
                self.exc_squawk_stub()
                stranger = self.root / "crates/mod-hx/migrations/0001_hx.up.sql"
                stranger.parent.mkdir(parents=True, exist_ok=True)
                stranger.write_text(self.REVOKE_FIXTURE)
                (self.root / "scripts/release_manifest_real.py").write_bytes(real_bytes)
                (self.root / "scripts/release_manifest.py").write_text(
                    "import os, sys\n"
                    "real = os.path.join(os.path.dirname(os.path.abspath(__file__)), 'release_manifest_real.py')\n"
                    "if sys.argv[1] == 'exception-candidate' and sys.argv[-1].endswith('0001_hx.up.sql'):\n"
                    "    sys.stdout.write(%r)\n"
                    "    sys.exit(0)\n"
                    "os.execv(sys.executable, [sys.executable, '-I', real] + sys.argv[1:])\n" % answer)
                result = self.run_release("1.2.3")
                self.assertNotEqual(result.returncode, 0, (label, result.stdout, result.stderr))
                self.assertIn("dynamic SQL requires review: 0001_hx.up.sql", result.stderr, label)
                self.assertFalse((self.root / "dist/1.2.3").exists(), label)

    MISSING_LIST = "release: the migration exception list is missing: a release needs the reviewed list (an explicit empty list means no waivers)"

    def test_exc_c7_a_missing_exception_list_refuses_before_any_gate_whatever_record_is_left(self):
        # sam EXC-SAM01 (cox W1-107, vera's answer (d): missing or invalid data is failure, not an implicit empty configuration): only an EXPLICIT schema-2 empty list
        # with its exact zero-entry record means no waivers; an absent list is refused with a fixed wording before the CI gate, an orphan record changes nothing
        config = self.root / "scripts/release-migration-exceptions.json"
        record = self.root / "scripts/release-migration-approval.json"
        self.valid_podman()
        def reset():
            shutil.rmtree(self.root / "dist", ignore_errors=True)
            self.log.unlink(missing_ok=True)
            for path in (config, record):
                if path.is_symlink() or path.is_file():
                    path.unlink()
                elif path.is_dir():
                    shutil.rmtree(path)
            (self.root / self.PLATFORM_PATH).write_text("SELECT 1;\n")
            for name in ("mod-fx", "mod-gx"):
                shutil.rmtree(self.root / "crates" / name, ignore_errors=True)
            self.exc_write([], [])
        def refused(result, wording, label):
            self.assertNotEqual(result.returncode, 0, (label, result.stdout, result.stderr))
            self.assertIn(wording, result.stderr, label)
            self.assertEqual(self.calls(), [], label + ": no gate may run")
            self.assertFalse((self.root / "dist/1.2.3").exists(), label)
        with self.subTest(case="control: the explicit empty list with its exact zero-entry record means no waivers and the release builds"):
            reset()
            result = self.run_release("1.2.3")
            self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
            self.assertTrue((self.root / "dist/1.2.3").is_dir())
            self.assertNotIn("is missing", result.stderr)
        with self.subTest(case="control: a normal present list with its record passes"):
            reset()
            self.exc_fixture()
            self.exc_squawk_stub()
            result = self.run_release("1.2.3")
            self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        with self.subTest(case="control: the list present and the record missing is refused as unverified"):
            reset()
            record.unlink()
            refused(self.run_release("1.2.3"), "release: migration exception approval is not verified", "list without record")
        with self.subTest(case="a missing list with an ORPHAN record is refused with the fixed wording"):
            reset()
            config.unlink()
            self.assertTrue(record.exists())
            refused(self.run_release("1.2.3"), self.MISSING_LIST, "orphan record")
        with self.subTest(case="a missing list and a missing record is refused with the fixed wording"):
            reset()
            config.unlink()
            record.unlink()
            refused(self.run_release("1.2.3"), self.MISSING_LIST, "nothing at all")
        with self.subTest(case="a directory at the list path is refused as unverified, not as missing"):
            reset()
            config.unlink()
            config.mkdir()
            result = self.run_release("1.2.3")
            refused(result, "release: migration exception approval is not verified", "a directory")
            self.assertNotIn(self.MISSING_LIST, result.stderr)
        with self.subTest(case="a dangling symbolic link at the list path is refused as unverified, not as missing"):
            reset()
            config.unlink()
            config.symlink_to(self.root / "nowhere.json")
            result = self.run_release("1.2.3")
            refused(result, "release: migration exception approval is not verified", "a dangling link")
            self.assertNotIn(self.MISSING_LIST, result.stderr)
        with self.subTest(case="a symbolic link to a valid list is refused as unverified"):
            reset()
            elsewhere = self.root / "elsewhere-list.json"
            elsewhere.write_bytes(config.read_bytes())
            config.unlink()
            config.symlink_to(elsewhere)
            refused(self.run_release("1.2.3"), "release: migration exception approval is not verified", "a link to a valid list")

    # ---- D: the approval record (schema 2): the config hash and one pin per entry ----

    def test_exc_d1_each_defect_of_the_record_refuses(self):
        def edit_pin(index, **changes):
            def apply(entries, pins, record):
                pins[index].update(changes)
            return apply
        def edit_record(**changes):
            def apply(entries, pins, record):
                record.update(changes)
            return apply
        def drop_pin(entries, pins, record): pins.pop()
        def extra_pin(entries, pins, record): pins.append(dict(pins[0], migration_path="crates/mod-zz/migrations/0001_zz.up.sql"))
        def duplicate_pin(entries, pins, record): pins[2] = dict(pins[1])
        def unknown_pin_key(entries, pins, record): pins[1]["extra"] = 1
        def missing_pin_key(entries, pins, record): del pins[1]["review_sha256"]
        def review_other_name(entries, pins, record):
            other = self.root / "ReviewService/example/OTHER.md"
            other.write_bytes((self.root / "ReviewService/example/REVIEW.md").read_bytes())
            pins[1]["review_path"] = str(other.relative_to(self.root))
        def review_removed(entries, pins, record): (self.root / "ReviewService/example/REVIEW.md").unlink()
        def review_changed(entries, pins, record): (self.root / "ReviewService/example/REVIEW.md").write_text("changed after the record\n")
        cases = [("a wrong config hash", edit_record(config_sha256="0" * 64), "config"), ("a wrong per-entry migration hash", edit_pin(1, migration_sha256="0" * 64), "pin"),
                 ("a zero approved_by in a pin", edit_pin(1, approved_by="00000000-0000-0000-0000-000000000000"), "approved_by"),
                 ("a malformed approved_by in a pin", edit_pin(1, approved_by="PENDING"), "approved_by"),
                 ("a pin's approved_by that differs from the entry's", edit_pin(1, approved_by="abcdef09-1234-4321-9876-abcdefabcdef"), "approved_by"),
                 ("a review path with ..", edit_pin(1, review_path="../outside/REVIEW.md"), "review"), ("a review file removed", review_removed, "review"),
                 ("a review file changed after the record", review_changed, "review"), ("a review file with a name outside the allowed set", review_other_name, "review"),
                 ("one pin fewer than entries", drop_pin, "pin"), ("a pin for an unknown path", extra_pin, "pin"), ("a duplicate pin", duplicate_pin, "duplicate pin"),
                 ("an unknown key in a pin", unknown_pin_key, "field"), ("a missing key in a pin", missing_pin_key, "field"),
                 ("record schema 1", edit_record(schema=1), "schema"), ("an unknown record key", edit_record(extra=1), "field")]
        for label, edit, cause in cases:
            with self.subTest(case=label):
                for d in ("mod-fx", "mod-gx"):
                    shutil.rmtree(self.root / "crates" / d, ignore_errors=True)
                entries, pins = self.exc_fixture(v2=(("crates/mod-fx/migrations/0001_fx.up.sql", self.REVOKE_FIXTURE), ("crates/mod-gx/migrations/0001_gx.up.sql", self.LITERAL_FIXTURE)))
                record_path = self.root / "scripts/release-migration-approval.json"
                record = json.loads(record_path.read_text())
                edit(entries, record["entries"], record)
                record_path.write_text(json.dumps(record, indent=1) + "\n")
                verified = self.exc_verify()
                self.assertNotEqual(verified.returncode, 0, label)
                self.assertIn("release: migration exception approval pins are invalid", verified.stderr)
                self.assertIn(cause, verified.stderr, label)
                self.assertNotIn("Traceback", verified.stderr)

    def test_exc_d2_the_old_aggregate_record_cannot_cover_a_changed_list(self):
        # the old schema-1 record (one migration, the old config hash) is the form vera's ruling retires for a changed list
        entries, pins = self.exc_fixture()
        config = self.root / "scripts/release-migration-exceptions.json"
        old_record = {"schema": 1, "approved_by": self.ROW_A, "migration_path": self.PLATFORM_PATH, "migration_sha256": entries[0]["sha256"],
                      "exception_sha256": hashlib.sha256(config.read_bytes()).hexdigest(), "review_path": pins[0]["review_path"], "review_sha256": pins[0]["review_sha256"]}
        (self.root / "scripts/release-migration-approval.json").write_text(json.dumps(old_record) + "\n")
        verified = self.exc_verify()
        self.assertNotEqual(verified.returncode, 0)
        self.assertIn("schema", verified.stderr)

    # ---- S: Squawk approvals on a matcher-2 entry (cox c8e39569, plan amendment accepted 06:25Z; vera's RULING.md line 28: any actual finding needs its own full migration hash ... before exemption) ----
    # An approval is {"rule", "line", "tool_sha256", "config_sha256"}: squawk's REPORTED line (one LESS than the file's line: the statement is at file line `line + 1`), the pinned tool
    # digest and the digest of scripts/release-squawk.toml, bound to the entry's own FULL migration sha256 (which pins every statement). `squawk: []` keeps "no waiver, a clean report".
    # The platform entry (matcher 1, the constants BASELINE_*) is frozen and untouched. The refusal causes below are the contract of the GREEN code. Fixture data: nothing here approves anything.

    SQUAWK_TOOL_SHA256 = "cf542e159fb3a94e5fd31d8b5282a31334df8205c6d7993afe286e9133fdff3e"
    SQ_A, SQ_B, SQ_C = "require-lock-timeout", "prefer-bigint-over-int", "require-statement-timeout"
    SQ_FX, SQ_NX, SQ_GX = "crates/mod-fx/migrations/0001_fx.up.sql", "crates/mod-nx/migrations/0001_nx.up.sql", "crates/mod-gx/migrations/0001_gx.up.sql"
    NODYN_FIXTURE = ("-- fixture migration: no EXECUTE word at all (a handoffs-like file without privilege words)\n"
                     "CREATE SCHEMA nx;\n"
                     "CREATE TABLE nx.t (id integer);\n")
    GRANT_FIXTURE = ("-- fixture migration: a privilege word that is NOT dynamic SQL (a handoffs-like file)\n"
                     "CREATE SCHEMA hx;\n"
                     "GRANT EXECUTE ON FUNCTION hx.f() TO PUBLIC;\n")
    REAL_MIGRATIONS = {"0001_platform": "crates/cortex-db/migrations/platform/0001_platform.up.sql", "0001_audit": "crates/mod-audit/migrations/0001_audit.up.sql",
                       "0001_handoffs": "crates/mod-handoffs/migrations/0001_handoffs.up.sql", "0001_identity": "crates/mod-identity/migrations/0001_identity.up.sql",
                       "0001_records": "crates/mod-records/migrations/0001_records.up.sql", "0001_tokens": "crates/mod-tokens/migrations/0001_tokens.up.sql"}
    # what the one scanner says about the five module migrations at d350d6b: (raw EXECUTE words, dynamic findings); handoffs, identity and tokens have raw words (privilege syntax) and NO dynamic SQL
    REAL_SCAN = {"0001_audit": (6, 3), "0001_handoffs": (5, 0), "0001_identity": (5, 0), "0001_records": (13, 5), "0001_tokens": (2, 0)}
    REAL_REPORT_SHA256 = {"0001_platform": "339dfcbf00d5145d4b929a6ddc6aed12bb362f043e2fc323818ecac8a809dab4", "0001_audit": "25514209c7024cbca2ace0362353d2c4c88c48b135f8cc7f748d6678dfd899bd",
                          "0001_handoffs": "04649b3a107ac3d57cd1e9a4d1f4fec2b42066af7a4849cd198d23ce9b7bfd48", "0001_identity": "ca709a5bdb9b40c86367f68ae518963af138a5dfcb4259001f105d254ca606a2",
                          "0001_records": "0a77d33112944dfb30e36b745cf46c5056c2b570a405c5599145217f534309d5", "0001_tokens": "dd30d634555a7ff6ea4cfda7a3de96dec12491119bfee6eb9c02d8574c203258"}
    # the statement each of the 26 real findings flags: file line `reported line + 1`, code part only (a trailing comment cut, spaces collapsed)
    REAL_FLAGGED = (
        ('0001_platform', 'require-lock-timeout', 2, 'CREATE SCHEMA platform;'),
        ('0001_platform', 'require-statement-timeout', 2, 'CREATE SCHEMA platform;'),
        ('0001_platform', 'prefer-bigint-over-int', 7, 'version integer NOT NULL,'),
        ('0001_platform', 'prefer-bigint-over-int', 34, 'status integer,'),
        ('0001_audit', 'require-lock-timeout', 8, 'CREATE SCHEMA audit;'),
        ('0001_audit', 'require-statement-timeout', 8, 'CREATE SCHEMA audit;'),
        ('0001_handoffs', 'require-lock-timeout', 12, 'CREATE SCHEMA handoffs;'),
        ('0001_handoffs', 'require-statement-timeout', 12, 'CREATE SCHEMA handoffs;'),
        ('0001_handoffs', 'prefer-bigint-over-int', 30, 'lease_ttl_s integer NOT NULL DEFAULT 7200 CHECK (lease_ttl_s BETWEEN 300 AND 86400),'),
        ('0001_handoffs', 'prefer-bigint-over-int', 32, 'priority integer NOT NULL DEFAULT 50 CHECK (priority BETWEEN 1 AND 100),'),
        ('0001_handoffs', 'prefer-bigint-over-int', 45, 'round integer NOT NULL DEFAULT 1 CHECK (round >= 1),'),
        ('0001_handoffs', 'prefer-bigint-over-int', 46, 'transition_seq integer NOT NULL DEFAULT 0 CHECK (transition_seq >= 0),'),
        ('0001_handoffs', 'prefer-bigint-over-int', 91, 'round integer NOT NULL CHECK (round >= 1),'),
        ('0001_handoffs', 'prefer-bigint-over-int', 109, 'size integer NOT NULL CHECK (size >= 0),'),
        ('0001_handoffs', 'prefer-bigint-over-int', 122, 'round integer NOT NULL CHECK (round >= 1),'),
        ('0001_handoffs', 'prefer-bigint-over-int', 138, 'seq integer NOT NULL CHECK (seq >= 1),'),
        ('0001_identity', 'require-lock-timeout', 7, 'CREATE SCHEMA identity;'),
        ('0001_identity', 'require-statement-timeout', 7, 'CREATE SCHEMA identity;'),
        ('0001_identity', 'prefer-bigint-over-int', 25, 'token_lifetime_days integer NOT NULL DEFAULT 90 CHECK (token_lifetime_days BETWEEN 7 AND 365),'),
        ('0001_records', 'require-lock-timeout', 4, 'CREATE SCHEMA records;'),
        ('0001_records', 'require-statement-timeout', 4, 'CREATE SCHEMA records;'),
        ('0001_records', 'prefer-bigint-over-int', 55, 'section_revision integer CHECK (section_revision IS NULL OR section_revision >= 1),'),
        ('0001_records', 'prefer-bigint-over-int', 148, 'last_revision integer NOT NULL DEFAULT 0 CHECK (last_revision >= 0),'),
        ('0001_tokens', 'require-lock-timeout', 6, 'CREATE SCHEMA tokens;'),
        ('0001_tokens', 'require-statement-timeout', 6, 'CREATE SCHEMA tokens;'),
    )

    def sq(self, rule, line, tool=None, config=None):
        """One Squawk approval as the entry carries it, with the real digests of the fixture root unless told otherwise."""
        return {"rule": rule, "line": line, "tool_sha256": tool or self.SQUAWK_TOOL_SHA256,
                "config_sha256": config or hashlib.sha256((self.root / "scripts/release-squawk.toml").read_bytes()).hexdigest()}

    def sq_report(self, relative, pairs, file=None):
        return json.dumps([{"file": file or str(self.root / relative), "rule_name": rule, "line": line} for rule, line in pairs])

    def sq_stub(self, answers):
        """A squawk stand-in answering per migration (the migration is the last argument): `answers` maps a migration path to (status, report text); the platform file gets its
        approved four; any other file gets an empty report with status 0."""
        platform = str(self.root / self.PLATFORM_PATH)
        four = json.dumps([{"file": platform, "rule_name": rule, "line": line} for rule, line in
                           (("require-lock-timeout", 2), ("require-statement-timeout", 2), ("prefer-bigint-over-int", 7), ("prefer-bigint-over-int", 34))])
        text = '#!/bin/sh\nprintf "%s\\n" squawk >> "$RELEASE_TEST_LOG"\nfor a in "$@"; do last=$a; done\ncase $last in\n'
        for relative, (status, report) in answers.items():
            text += f"  *{relative}) cat <<'JSON'\n{report}\nJSON\n    exit {status} ;;\n"
        text += f"  *0001_platform.up.sql) cat <<'JSON'\n{four}\nJSON\n    exit 1 ;;\n  *) echo '[]'; exit 0 ;;\nesac\n"
        stub = self.root / "bin/squawk"
        stub.write_text(text)
        stub.chmod(0o755)

    def sq_clean_root(self):
        shutil.rmtree(self.root / "dist", ignore_errors=True)
        for name in ("mod-fx", "mod-gx", "mod-nx", "mod-hx", "mod-ux"):
            shutil.rmtree(self.root / "crates" / name, ignore_errors=True)
        self.log.unlink(missing_ok=True)

    def sq_release(self, files, approvals, answers, mutate=None):
        """Build the fixture list (platform + `files`, the approvals per path), optionally edit entries and pins, install the stub, run a release."""
        self.sq_clean_root()
        entries, pins = self.exc_fixture(v2=files, squawk=approvals)
        if mutate:
            mutate(entries, pins)
            self.exc_write(entries, pins)
        self.sq_stub(answers)
        return self.run_release("1.2.3")

    FX_FILES = ((SQ_FX, REVOKE_FIXTURE), (SQ_NX, NODYN_FIXTURE), (SQ_GX, LITERAL_FIXTURE))

    def test_sq_a1_a_findings_entry_verifies_and_is_a_findings_candidate_beside_the_others(self):
        self.exc_fixture(v2=self.FX_FILES, squawk={self.SQ_FX: [self.sq(self.SQ_A, 1), self.sq(self.SQ_B, 3)], self.SQ_NX: [self.sq(self.SQ_A, 1), self.sq(self.SQ_C, 1)]})
        verified = self.exc_verify()
        self.assertEqual(verified.returncode, 0, (verified.stdout, verified.stderr))
        for relative, mode in ((self.PLATFORM_PATH, "baseline"), (self.SQ_FX, "findings"), (self.SQ_NX, "findings"), (self.SQ_GX, "clean")):
            with self.subTest(path=relative):
                found = self.exc_candidate(relative)
                self.assertEqual(found.returncode, 0, (found.stdout, found.stderr))
                self.assertEqual(found.stdout.strip(), mode)

    def test_sq_a2_every_malformed_approval_is_refused_with_its_own_cause(self):
        M_SHAPE, M_LINE, M_RULE = "a Squawk approval has the wrong shape", "a Squawk approval line is not an integer", "a Squawk approval rule is not an ASCII name"
        M_TOOL, M_CONFIG, M_LIST = "the Squawk approval tool digest is not the pinned one", "the Squawk approval config digest differs from scripts/release-squawk.toml", "Squawk approvals are not a list"
        def item(**change):
            def apply(entry):
                entry["squawk"][0].update(change)
            return apply
        def drop(key):
            def apply(entry):
                del entry["squawk"][0][key]
            return apply
        def replace(value):
            def apply(entry):
                entry["squawk"] = value
            return apply
        def old_shape(entry): entry["squawk"] = [{"rule": self.SQ_A, "line": 1}]
        cases = [("an unknown field", item(extra=1), M_SHAPE), ("the config digest missing", drop("config_sha256"), M_SHAPE), ("the rule missing", drop("rule"), M_SHAPE),
                 ("the old shape (rule and line only)", old_shape, M_SHAPE), ("an approval that is not an object", replace(["x"]), M_SHAPE),
                 ("a line as a string", item(line="1"), M_LINE), ("a line as a boolean", item(line=True), M_LINE), ("a line as a float", item(line=1.0), M_LINE), ("a negative line", item(line=-1), M_LINE),
                 ("an empty rule", item(rule=""), M_RULE), ("a rule that is not a string", item(rule=5), M_RULE), ("a non-ASCII rule", item(rule="régle"), M_RULE),
                 ("a rule with a space", item(rule="two words"), M_RULE), ("an upper-case rule", item(rule="Require-Lock"), M_RULE), ("a rule of 65 characters", item(rule="a" * 65), M_RULE),
                 ("the digest of another tool", item(tool_sha256="0" * 64), M_TOOL), ("a tool digest that is not 64 hex characters", item(tool_sha256="xyz"), M_TOOL),
                 ("the digest of another config", item(config_sha256="0" * 64), M_CONFIG), ("a config digest that is not 64 hex characters", item(config_sha256="xyz"), M_CONFIG),
                 ("the approvals as an object", replace({"rule": self.SQ_A}), M_LIST), ("the approvals as null", replace(None), M_LIST)]
        with self.subTest(case="control: the valid approval verifies"):
            self.exc_fixture(v2=self.FX_FILES, squawk={self.SQ_FX: [self.sq(self.SQ_A, 1)]})
            verified = self.exc_verify()
            self.assertEqual(verified.returncode, 0, (verified.stdout, verified.stderr))
        for label, edit, cause in cases:
            with self.subTest(case=label):
                self.sq_clean_root()
                entries, pins = self.exc_fixture(v2=self.FX_FILES, squawk={self.SQ_FX: [self.sq(self.SQ_A, 1)]})
                edit(entries[1])
                self.exc_write(entries, pins)
                verified = self.exc_verify()
                self.assertNotEqual(verified.returncode, 0, label)
                self.assertIn("release: migration exception approval pins are invalid", verified.stderr)
                self.assertIn(cause, verified.stderr, label)
                self.assertNotIn("Traceback", verified.stderr)

    def test_sq_a3_the_approvals_are_bound_to_the_pinned_tool_and_to_the_config_in_the_tree(self):
        helper = self.exc_helper()
        release_text = (ROOT / "scripts/release.sh").read_text()
        literal = re.search(r'\[ "\$actual" = ([0-9a-f]{64}) \]', release_text)
        self.assertIsNotNone(literal, "release.sh no longer compares the squawk digest to a literal")
        self.assertEqual(getattr(helper, "SQUAWK_TOOL_SHA256", None), literal.group(1), "the helper's pinned tool digest must equal the one release.sh compares")
        self.assertEqual(literal.group(1), self.SQUAWK_TOOL_SHA256)
        with self.subTest(case="control: approvals bound to the config that is in the tree verify"):
            self.exc_fixture(v2=self.FX_FILES, squawk={self.SQ_FX: [self.sq(self.SQ_A, 1)]})
            self.assertEqual(self.exc_verify().returncode, 0)
        with self.subTest(case="the config edited after the approval"):
            (self.root / "scripts/release-squawk.toml").write_text((self.root / "scripts/release-squawk.toml").read_text() + "# edited after the approval\n")
            verified = self.exc_verify()
            self.assertNotEqual(verified.returncode, 0)
            self.assertIn("the Squawk approval config digest differs from scripts/release-squawk.toml", verified.stderr)

    def test_sq_a4_a_duplicate_approval_is_refused_with_its_own_message_and_near_misses_are_not_duplicates(self):
        for label, approvals, ok in (("the same rule on two lines", [self.sq(self.SQ_A, 1), self.sq(self.SQ_A, 2)], True), ("two rules on one line", [self.sq(self.SQ_A, 1), self.sq(self.SQ_B, 1)], True),
                                     ("the same rule and the same line twice", [self.sq(self.SQ_A, 1), self.sq(self.SQ_A, 1)], False)):
            with self.subTest(case=label):
                self.sq_clean_root()
                self.exc_fixture(v2=self.FX_FILES, squawk={self.SQ_FX: approvals})
                verified = self.exc_verify()
                if ok:
                    self.assertEqual(verified.returncode, 0, (verified.stdout, verified.stderr))
                else:
                    self.assertNotEqual(verified.returncode, 0)
                    self.assertIn("duplicate Squawk approval", verified.stderr)

    def test_sq_b1_gate_4_passes_when_the_report_equals_the_approvals_and_clean_mode_is_unchanged(self):
        fx_pairs, nx_pairs = [(self.SQ_A, 1), (self.SQ_B, 3)], [(self.SQ_A, 1), (self.SQ_C, 1), (self.SQ_B, 2)]
        result = self.sq_release(self.FX_FILES, {self.SQ_FX: [self.sq(r, l) for r, l in fx_pairs], self.SQ_NX: [self.sq(r, l) for r, l in nx_pairs]},
                                 {self.SQ_FX: (1, self.sq_report(self.SQ_FX, fx_pairs)), self.SQ_NX: (1, self.sq_report(self.SQ_NX, list(reversed(nx_pairs)))), self.SQ_GX: (0, "[]")})
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertTrue((self.root / "dist/1.2.3").is_dir())
        self.assertEqual(self.calls().count("squawk"), 4)

    def test_sq_b2_gate_4_refuses_every_difference_between_the_report_and_the_approvals(self):
        approved = [(self.SQ_A, 1), (self.SQ_B, 3)]
        M_EXTRA, M_STALE = "Squawk finding is not approved: ", "approved Squawk finding is not in the report (stale): "
        big = self.sq_report(self.SQ_FX, approved) + " " * 1100000   # valid JSON EQUAL to the approvals, over 1 MiB: only the size bound refuses it
        elsewhere = self.sq_report(self.SQ_FX, approved, file="/elsewhere/0001_fx.up.sql")
        cases = [("an extra finding", 1, lambda: self.sq_report(self.SQ_FX, approved + [(self.SQ_C, 1)]), M_EXTRA + self.SQ_C + " line 1"),
                 ("a missing finding (a stale approval)", 1, lambda: self.sq_report(self.SQ_FX, approved[:1]), M_STALE + self.SQ_B + " line 3"),
                 ("one finding on another line", 1, lambda: self.sq_report(self.SQ_FX, [approved[0], (self.SQ_B, 4)]), None),
                 ("one finding of another rule on the same line", 1, lambda: self.sq_report(self.SQ_FX, [approved[0], (self.SQ_C, 3)]), None),
                 ("a finding reported twice", 1, lambda: self.sq_report(self.SQ_FX, approved + [approved[0]]), M_EXTRA + self.SQ_A + " line 1"),
                 ("status 0 with the approved report (an approval is stale at status 0)", 0, lambda: self.sq_report(self.SQ_FX, approved), None),
                 ("status 0 with an empty report", 0, lambda: "[]", None), ("status 1 with an empty report", 1, lambda: "[]", None),
                 ("status 2 with the approved report", 2, lambda: self.sq_report(self.SQ_FX, approved), None),
                 ("a malformed report", 1, lambda: "{not json", None), ("a truncated report", 1, lambda: '[{"file": ', None), ("a report that is not a list", 1, lambda: '{"findings": []}', None),
                 ("a report over 1 MiB", 1, lambda: big, None), ("the approved findings reported for another file", 1, lambda: elsewhere, None)]
        with self.subTest(case="control: the equal report passes"):
            result = self.sq_release(self.FX_FILES, {self.SQ_FX: [self.sq(r, l) for r, l in approved]}, {self.SQ_FX: (1, self.sq_report(self.SQ_FX, approved)), self.SQ_GX: (0, "[]")})
            self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        for label, status, report, message in cases:
            with self.subTest(case=label):
                result = self.sq_release(self.FX_FILES, {self.SQ_FX: [self.sq(r, l) for r, l in approved]}, {self.SQ_FX: (status, report()), self.SQ_GX: (0, "[]")})
                self.assertNotEqual(result.returncode, 0, (label, result.stdout, result.stderr))
                self.assertIn("release: approved Squawk findings differ: ", result.stderr, label)
                self.assertIn("0001_fx.up.sql", result.stderr, label)
                if message:
                    self.assertIn(message, result.stderr, label)
                self.assertFalse((self.root / "dist/1.2.3").exists(), label)

    def test_sq_b3_the_approvals_of_one_migration_cannot_satisfy_another(self):
        fx_pairs, nx_pairs = [(self.SQ_A, 1), (self.SQ_B, 3)], [(self.SQ_C, 1), (self.SQ_B, 2)]
        answers = {self.SQ_FX: (1, self.sq_report(self.SQ_FX, fx_pairs)), self.SQ_NX: (1, self.sq_report(self.SQ_NX, nx_pairs)), self.SQ_GX: (0, "[]")}
        approvals = {self.SQ_FX: [self.sq(r, l) for r, l in fx_pairs], self.SQ_NX: [self.sq(r, l) for r, l in nx_pairs]}
        with self.subTest(case="control: each entry with its own approvals passes"):
            result = self.sq_release(self.FX_FILES, approvals, answers)
            self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        with self.subTest(case="the two entries carry each other's approvals"):
            def swap(entries, pins):
                entries[1]["squawk"], entries[2]["squawk"] = entries[2]["squawk"], entries[1]["squawk"]
            result = self.sq_release(self.FX_FILES, approvals, answers, mutate=swap)
            self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))
            self.assertIn("release: approved Squawk findings differ: ", result.stderr)
            self.assertFalse((self.root / "dist/1.2.3").exists())
        with self.subTest(case="an entry names one migration's path and the other's full hash"):
            def swap_identity(entries, pins):
                entries[1]["sha256"], entries[2]["sha256"] = entries[2]["sha256"], entries[1]["sha256"]
                pins[1]["migration_sha256"], pins[2]["migration_sha256"] = pins[2]["migration_sha256"], pins[1]["migration_sha256"]
            result = self.sq_release(self.FX_FILES, approvals, answers, mutate=swap_identity)
            self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))
            self.assertIn("sha256 differs", result.stderr)
            self.assertEqual(self.calls(), [])
        with self.subTest(case="an unlisted migration reporting the same findings as an approved one gets nothing"):
            self.sq_clean_root()
            self.exc_fixture(v2=self.FX_FILES, squawk=approvals)
            unlisted = self.root / "crates/mod-ux/migrations/0001_ux.up.sql"
            unlisted.parent.mkdir(parents=True)
            unlisted.write_text("-- an unlisted file whose squawk report equals the approved one of another\nCREATE SCHEMA ux;\nCREATE TABLE ux.t (id integer);\n")
            self.sq_stub(dict(answers, **{"crates/mod-ux/migrations/0001_ux.up.sql": (1, self.sq_report("crates/mod-ux/migrations/0001_ux.up.sql", nx_pairs))}))
            result = self.run_release("1.2.3")
            self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))
            self.assertIn("release: migration gate failed", result.stderr)
            self.assertIn("0001_ux.up.sql", result.stderr)
            self.assertFalse((self.root / "dist/1.2.3").exists())

    def test_sq_b4_a_file_changed_after_its_approval_refuses_before_any_gate(self):
        pairs = [(self.SQ_A, 1)]
        with self.subTest(case="control: the unchanged file passes"):
            result = self.sq_release(self.FX_FILES, {self.SQ_FX: [self.sq(r, l) for r, l in pairs]}, {self.SQ_FX: (1, self.sq_report(self.SQ_FX, pairs)), self.SQ_GX: (0, "[]")})
            self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        with self.subTest(case="one byte appended after the approval"):
            self.sq_clean_root()
            self.exc_fixture(v2=self.FX_FILES, squawk={self.SQ_FX: [self.sq(r, l) for r, l in pairs]})
            (self.root / self.SQ_FX).write_text(self.REVOKE_FIXTURE + "-\n")
            self.sq_stub({self.SQ_FX: (1, self.sq_report(self.SQ_FX, pairs)), self.SQ_GX: (0, "[]")})
            result = self.run_release("1.2.3")
            self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))
            self.assertIn("sha256 differs", result.stderr)
            self.assertEqual(self.calls(), [])

    def test_sq_c1_the_three_real_migrations_without_dynamic_sql_get_entries_that_equal_what_the_scan_says(self):
        # cox A(2): adding an entry for handoffs, identity or tokens must not hide dynamic SQL: each entry's raw words and dynamic findings are the scan's, per migration
        helper = self.exc_helper()
        for key, (words, dynamic) in self.REAL_SCAN.items():
            relative = self.REAL_MIGRATIONS[key]
            text = (ROOT / relative).read_text()
            with self.subTest(migration=key, what="the scan says exactly this"):
                self.assertEqual((len(helper.scan_execute_words(text)), len(helper.scan_dynamic_findings(text))), (words, dynamic))
        for key in ("0001_handoffs", "0001_identity", "0001_tokens"):
            relative = self.REAL_MIGRATIONS[key]
            report = json.loads((ROOT / "scripts/release-fixtures/squawk-real-reports" / (key + ".json")).read_text())
            pairs = [(f["rule_name"], f["line"]) for f in report]
            with self.subTest(migration=key, what="an entry with the real approvals and the scan's raw words verifies"):
                self.sq_clean_root()
                shutil.rmtree(self.root / "crates/mod-fx", ignore_errors=True)
                entries, pins = self.exc_fixture(v2=())
                (self.root / relative).parent.mkdir(parents=True, exist_ok=True)
                (self.root / relative).write_bytes((ROOT / relative).read_bytes())
                entry = self.v2_entry(relative, self.ROW_B, squawk=[self.sq(r, l) for r, l in pairs])
                self.assertEqual(entry["dynamic_findings"], [], "the test's own scanner finds no dynamic SQL in " + key)
                self.assertEqual(len(entry["raw_execute_words"]), self.REAL_SCAN[key][0])
                entries.append(entry)
                pins.append({"migration_path": relative, "migration_sha256": entry["sha256"], "approved_by": entry["approved_by"],
                             "review_path": pins[0]["review_path"], "review_sha256": pins[0]["review_sha256"]})
                self.exc_write(entries, pins)
                verified = self.exc_verify()
                self.assertEqual(verified.returncode, 0, (key, verified.stdout, verified.stderr))
                with self.subTest(migration=key, what="a raw word dropped refuses"):
                    entries[-1]["raw_execute_words"].pop()
                    self.exc_write(entries, pins)
                    refused = self.exc_verify()
                    self.assertNotEqual(refused.returncode, 0)
                    self.assertIn("raw EXECUTE words differ", refused.stderr)
        with self.subTest(case="an entry that lists no dynamic finding cannot hide dynamic SQL in a file that has one"):
            self.sq_clean_root()
            entries, pins = self.exc_fixture(v2=((self.SQ_FX, self.REVOKE_FIXTURE),), squawk={self.SQ_FX: [self.sq(self.SQ_A, 1)]})
            entries[1]["dynamic_findings"] = []
            self.exc_write(entries, pins)
            refused = self.exc_verify()
            self.assertNotEqual(refused.returncode, 0)
            self.assertIn("dynamic findings differ", refused.stderr)

    def test_sq_c2_the_frozen_platform_constants_and_the_clean_mode_are_untouched(self):
        helper = self.exc_helper()
        self.assertEqual(helper.BASELINE_PATH, self.PLATFORM_PATH)
        self.assertEqual(helper.BASELINE_SHA256, "d5c79f3b15643af101343eed72888a2e2ab490577fa57047037e0553c4ceb3f4")
        self.assertEqual(helper.BASELINE_DYNAMIC_LINES, [83, 91, 103, 104])
        self.assertEqual(helper.BASELINE_SQUAWK, [("require-lock-timeout", 2), ("require-statement-timeout", 2), ("prefer-bigint-over-int", 7), ("prefer-bigint-over-int", 34)])
        with self.subTest(case="a matcher-2 entry with squawk [] still needs a clean report with status 0 (clean mode)"):
            result = self.sq_release(self.FX_FILES, {}, {self.SQ_FX: (1, self.sq_report(self.SQ_FX, [(self.SQ_A, 1)])), self.SQ_GX: (0, "[]")})
            self.assertNotEqual(result.returncode, 0, (result.stdout, result.stderr))
            self.assertIn("release: migration gate failed", result.stderr)

    def test_sq_c3_the_flagged_statement_of_each_real_finding_is_the_file_line_after_the_reported_line(self):
        # squawk reports a line ONE LESS than the file's line (measured on all 26): the statement a reported line identifies is the file line reported + 1
        lines_of = {key: (ROOT / relative).read_text().split("\n") for key, relative in self.REAL_MIGRATIONS.items()}
        self.assertEqual(len(self.REAL_FLAGGED), 25)   # 26 at d350d6b; the audit bigint finding is gone with actor_uid bigint (finn b781fe3, vera 29aba9ba)
        for key, rule, line, statement in self.REAL_FLAGGED:
            with self.subTest(migration=key, rule=rule, reported_line=line):
                self.assertEqual(" ".join(lines_of[key][line].split("--")[0].split()), statement)
        for key, relative in self.REAL_MIGRATIONS.items():
            report = (ROOT / "scripts/release-fixtures/squawk-real-reports" / (key + ".json")).read_bytes()
            with self.subTest(migration=key, what="the stored real report is the one pinned here"):
                self.assertEqual(hashlib.sha256(report).hexdigest(), self.REAL_REPORT_SHA256[key])
                self.assertEqual(sorted((f["rule_name"], f["line"]) for f in json.loads(report)), sorted((r, l) for k, r, l, _ in self.REAL_FLAGGED if k == key))

    def test_sq_d1_the_six_real_reports_replayed_against_the_six_real_migrations_pass_gate_4(self):
        # the mechanism against the REAL data: entries are built from the real reports exactly as the data step would (rule and reported line of every finding), the dynamic
        # findings by the helper's own scanner (these are real files), and a stand-in replays each real report with its `file` rewritten to the fixture path
        helper = self.exc_helper()
        self.sq_clean_root()
        entries, pins = self.exc_fixture(v2=())
        answers = {}
        for key, relative in self.REAL_MIGRATIONS.items():
            target = self.root / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes((ROOT / relative).read_bytes())
            report = json.loads((ROOT / "scripts/release-fixtures/squawk-real-reports" / (key + ".json")).read_text())
            for finding in report:
                finding["file"] = str(target)
            answers[relative] = (1, json.dumps(report))
            if key == "0001_platform":
                continue
            text = target.read_text()
            lines = text.split("\n")
            findings = [{"line": line, "column": column, "statement_sha256": hashlib.sha256(lines[line - 1].strip().encode()).hexdigest(),
                         "kind": helper.classify_finding(lines[line - 1], column), "reason": "replay reason"} for line, column in helper.scan_dynamic_findings(text)]
            entry = {"path": relative, "sha256": hashlib.sha256(target.read_bytes()).hexdigest(), "approved_by": self.ROW_B, "matcher": 2,
                     "raw_execute_words": [{"line": l, "column": c} for l, c in helper.scan_execute_words(text)], "dynamic_findings": findings,
                     "squawk": [self.sq(f["rule_name"], f["line"]) for f in report]}
            entries.append(entry)
            pins.append({"migration_path": relative, "migration_sha256": entry["sha256"], "approved_by": entry["approved_by"], "review_path": pins[0]["review_path"],
                         "review_sha256": pins[0]["review_sha256"]})
        self.exc_write(entries, pins)
        self.sq_stub(answers)
        verified = self.exc_verify()
        self.assertEqual(verified.returncode, 0, (verified.stdout, verified.stderr))
        result = self.run_release("1.2.3")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertTrue((self.root / "dist/1.2.3").is_dir())
        self.assertEqual(self.calls().count("squawk"), 6)

    # ---- P: the PRODUCTION data of the Squawk approvals (cox 65357864; vera's gate-4 ruling, local text relayed by kai, sha256 4b73344788b0...) ----
    # The data step, handoffs, identity and tokens first (records waits for the custody decision, audit for finn's new migration hash). The cells read the repository's OWN
    # scripts/release-migration-exceptions.json and scripts/release-migration-approval.json and the real files of this tree; the 22 verdicts are a hash-pinned fixture transcribed
    # by script from the table of her ruling; the six real reports are the fixtures of the replay cell d1. Where a cell needs vera's review files (absolute paths under /srv)
    # it is skipped on a host that does not have them, and says so.

    PD_ROW = "d4b7aeb6-93a3-4346-8f03-0b608b5914d6"
    PD_RULING = "/path/to/docs/Program/ReviewService/2026-11-09-squawk-gate4-d4b7aeb-vera/RULING.md"
    PD_RULING_SHA256 = "4b73344788b02711206f7226d26b6b62a96cd936fb20709250359cd3c25a789b"
    PD_TOOL_SHA256 = "cf542e159fb3a94e5fd31d8b5282a31334df8205c6d7993afe286e9133fdff3e"
    PD_CONFIG_SHA256 = "7816611e12c601652dcb67848876893642dd2f42647483df3893ae02fe4f9427"
    PD_NEW = ("0001_handoffs", "0001_identity", "0001_tokens")   # the migrations that had no entry and whose hashes do not change

    def pd_verdicts(self):
        return json.loads((ROOT / "scripts/release-fixtures/vera-gate4-verdicts.json").read_text())

    def pd_report(self, key, historical=False):
        """The real report of `key`; for audit the HISTORICAL one is the report of the OLD migration (d350d6b, 3 findings) that vera's gate-4 ruling judged."""
        return json.loads((ROOT / "scripts/release-fixtures/squawk-real-reports" / (key + (".d350d6b" if historical and key == "0001_audit" else "") + ".json")).read_text())

    def pd_list(self):
        return json.loads((ROOT / "scripts/release-migration-exceptions.json").read_text())["exceptions"]

    def pd_entry(self, key):
        wanted = self.REAL_MIGRATIONS[key]
        found = [e for e in self.pd_list() if e["path"] == wanted]
        return found[0] if len(found) == 1 else None

    def pd_expected_entry(self, key):
        """The entry the production list must carry for `key`: computed from the file in this tree, the helper's scan and vera's BASELINE verdicts (never from the list itself)."""
        helper = self.exc_helper()
        relative = self.REAL_MIGRATIONS[key]
        raw = (ROOT / relative).read_bytes()
        text = raw.decode("utf-8")
        approvals = sorted(({"rule": v["rule"], "line": v["line"], "tool_sha256": self.PD_TOOL_SHA256, "config_sha256": self.PD_CONFIG_SHA256}
                            for v in self.pd_verdicts()["verdicts"] if v["migration"] == key and v["verdict"] == "BASELINE"), key=lambda a: (a["line"], a["rule"]))
        return {"path": relative, "sha256": hashlib.sha256(raw).hexdigest(), "approved_by": self.PD_ROW, "matcher": 2,
                "raw_execute_words": [{"line": l, "column": c} for l, c in helper.scan_execute_words(text)], "dynamic_findings": [], "squawk": approvals}

    def pd_review_files_present(self):
        return all(Path(p).is_file() for p in (self.PD_RULING, "/path/to/docs/Program/ReviewService/2026-11-07-piece14-baseline-exception/REVIEW.md",
                                               "/path/to/docs/Program/ReviewService/2026-11-08-rehearsal-sql-7250e58/RULING.md"))

    def test_pd_a1_the_verdict_fixture_is_veras_22_verdicts_and_equals_the_real_reports(self):
        data = self.pd_verdicts()
        self.assertEqual(data["ruling"]["sha256"], self.PD_RULING_SHA256)
        self.assertEqual(data["ruling"]["row"], self.PD_ROW)
        self.assertEqual((data["tool_sha256"], data["config_sha256"]), (self.PD_TOOL_SHA256, self.PD_CONFIG_SHA256))
        self.assertEqual(hashlib.sha256((ROOT / "scripts/release-squawk.toml").read_bytes()).hexdigest(), self.PD_CONFIG_SHA256)
        verdicts = data["verdicts"]
        self.assertEqual((len(verdicts), sum(v["verdict"] == "BASELINE" for v in verdicts), [v for v in verdicts if v["verdict"] == "FIX"]),
                         (22, 21, [{"migration": "0001_audit", "sha256": "10266dc510b78dd7bed352ac8f5ce4050c24c81a6c0150b5b404e509f0599aa8", "rule": "prefer-bigint-over-int", "line": 17,
                                    "verdict": "FIX"}]))
        for key in ("0001_audit", "0001_handoffs", "0001_identity", "0001_records", "0001_tokens"):
            with self.subTest(migration=key):
                reported = sorted((f["rule_name"], f["line"]) for f in self.pd_report(key, historical=True))
                judged = sorted((v["rule"], v["line"]) for v in verdicts if v["migration"] == key)
                self.assertEqual(judged, reported, "vera judged exactly the findings the real binary reported")
                self.assertEqual(len({v["sha256"] for v in verdicts if v["migration"] == key}), 1)
        for key in ("0001_handoffs", "0001_identity", "0001_records", "0001_tokens"):   # their bytes did not change; audit's hash is the OLD one in the ruling
            with self.subTest(migration=key, what="the ruling's hash is the file's hash in this tree"):
                self.assertEqual({v["sha256"] for v in verdicts if v["migration"] == key}, {hashlib.sha256((ROOT / self.REAL_MIGRATIONS[key]).read_bytes()).hexdigest()})

    def test_pd_b1_the_production_list_carries_the_exact_findings_entries_for_handoffs_identity_and_tokens(self):
        for key in self.PD_NEW:
            with self.subTest(migration=key):
                entry = self.pd_entry(key)
                self.assertIsNotNone(entry, "the production list has no entry for " + key)
                self.assertEqual(entry, self.pd_expected_entry(key))
        with self.subTest(case="the three new entries carry vera's counts: 10, 3 and 2 approvals, 5, 5 and 2 raw words, no dynamic finding"):
            self.assertEqual([(len(self.pd_expected_entry(k)["squawk"]), len(self.pd_expected_entry(k)["raw_execute_words"])) for k in self.PD_NEW], [(10, 5), (3, 5), (2, 2)])

    def test_pd_b2_the_production_record_pins_the_three_entries_to_veras_ruling_and_the_list_it_hashes(self):
        record = json.loads((ROOT / "scripts/release-migration-approval.json").read_text())
        self.assertEqual(record["config_sha256"], hashlib.sha256((ROOT / "scripts/release-migration-exceptions.json").read_bytes()).hexdigest())
        pins = {p["migration_path"]: p for p in record["entries"]}
        for key in self.PD_NEW:
            with self.subTest(migration=key):
                relative = self.REAL_MIGRATIONS[key]
                self.assertIn(relative, pins)
                self.assertEqual(pins[relative], {"migration_path": relative, "migration_sha256": hashlib.sha256((ROOT / relative).read_bytes()).hexdigest(), "approved_by": self.PD_ROW,
                                                  "review_path": self.PD_RULING, "review_sha256": self.PD_RULING_SHA256})

    @unittest.skipUnless(Path("/path/to/docs/Program/ReviewService/2026-11-09-squawk-gate4-d4b7aeb-vera/RULING.md").is_file(), "vera's review files are under /srv on the VM only")
    def test_pd_c1_the_production_list_and_record_verify_on_this_tree_and_the_three_entries_are_findings_candidates(self):
        self.assertTrue(self.pd_review_files_present())
        verified = subprocess.run([sys.executable, "-I", str(ROOT / "scripts/release_manifest.py"), "verify-approval", "--root", str(ROOT)], text=True, capture_output=True, timeout=60)
        self.assertEqual(verified.returncode, 0, (verified.stdout, verified.stderr))
        for key in self.PD_NEW:
            with self.subTest(migration=key):
                found = subprocess.run([sys.executable, "-I", str(ROOT / "scripts/release_manifest.py"), "exception-candidate", "--root", str(ROOT), "--migration", str(ROOT / self.REAL_MIGRATIONS[key])],
                                       text=True, capture_output=True, timeout=60)
                self.assertEqual((found.returncode, found.stdout.strip()), (0, "findings"), (found.stdout, found.stderr))

    @unittest.skipUnless(Path("/path/to/docs/Program/ReviewService/2026-11-09-squawk-gate4-d4b7aeb-vera/RULING.md").is_file(), "vera's review files are under /srv on the VM only")
    def test_pd_d1_the_real_reports_replayed_against_the_production_list_pass_for_the_three_and_cannot_cross(self):
        def compare(key, against):
            report = self.pd_report(key)
            for finding in report:
                finding["file"] = str(ROOT / self.REAL_MIGRATIONS[against])
            target = self.root / ("report-" + key + "-" + against + ".json")
            target.write_text(json.dumps(report))
            return subprocess.run([sys.executable, "-I", str(ROOT / "scripts/release_manifest.py"), "exception-squawk", "--root", str(ROOT), "--migration", str(ROOT / self.REAL_MIGRATIONS[against]),
                                   "--input", str(target)], text=True, capture_output=True, timeout=60)
        for key in self.PD_NEW:
            with self.subTest(migration=key, what="its own real report equals its approvals"):
                result = compare(key, key)
                self.assertEqual(result.returncode, 0, (key, result.stdout, result.stderr))
        for key, other in (("0001_handoffs", "0001_identity"), ("0001_identity", "0001_tokens"), ("0001_tokens", "0001_handoffs")):
            with self.subTest(migration=key, what="another migration's real report cannot satisfy it: " + other):
                result = compare(other, key)
                self.assertNotEqual(result.returncode, 0, (key, other, result.stdout))
                self.assertTrue("Squawk finding is not approved" in result.stderr or "approved Squawk finding is not in the report (stale)" in result.stderr, result.stderr)

    # ---- E: the production data, step D2: AUDIT (the new hash) and RECORDS (cox answer B: vera's two CONSOLIDATED rulings of 9 Nov, one review file per entry, section A = the
    # dynamic-SQL findings, section B = the real Squawk findings, kept as separate kinds in the entry) ----
    # The fixture vera-consolidated-rulings.json is transcribed BY SCRIPT from the tables of the two rulings; the real reports are the fixtures of cell d1 (audit's is the report of
    # the NEW migration: two findings, the bigint one is gone with actor_uid bigint).

    PE_AUDIT = {"row": "29aba9ba-33f1-44b2-8aa5-d9f9a5bb61e3", "path": "/path/to/docs/Program/ReviewService/2026-11-09-audit-consolidated-29aba9ba-vera/RULING.md",
                "sha256": "35ed7b581e129864b6b05e16141bcf821897ddcfa3494dd288b0a9c15feb602f", "migration": "8f3b85baf619f9eec5847d1904470fa99a91177e1a045b3570893cf671825775"}
    PE_RECORDS = {"row": "893b4137-d853-4c85-bc89-5c19cad4e81e", "path": "/path/to/docs/Program/ReviewService/2026-11-09-records-consolidated-893b4137-vera/RULING.md",
                  "sha256": "adf16c4ec6854026603a9b6330b6ebb6d42da5afcb270e3adb67c8a072a6dd86", "migration": "69e0393a3ab48c4825dee5b133c03c6838468c417f57a40a6e99c6ba6c99f7b5"}
    PE_KEYS = {"0001_audit": PE_AUDIT, "0001_records": PE_RECORDS}
    PE_STATEMENTS = {"0001_audit": ["b2b7d2de29b99294c8273dcd511336fc86a52c3cd26432710306ce4ae4e84fac", "73ec3966ad46fc0c0348e691cfeaf10ab417749b2158ebd4a23dce454cd83259",
                                    "c5bae6f106b495cdb3c2b0a4f3b817714c8577fbd7968c17add94c0ad83d2c72"],
                     "0001_records": ["45fdeb5ccdaf354f6cae47a4686fa7847145f842e4793f775c6a4bf1052ed106", "77fb7698b656839ce5d826270992d2e9a6b5842b61293dce8fcfe669b04eaa40",
                                      "49f40ddbe3918e228fbe0e9e4444e1f992ca27b70cd5b5f48740e663f10f749e", "b1e6fa99a8a20914324816c50087541780eeb83902a76dcc2128c22187d285e8",
                                      "b1e6fa99a8a20914324816c50087541780eeb83902a76dcc2128c22187d285e8"]}

    def pe_fixture(self, key):
        return json.loads((ROOT / "scripts/release-fixtures/vera-consolidated-rulings.json").read_text())[key.replace("0001_", "")]

    def pe_expected_entry(self, key):
        """The entry the production list must carry for audit or records: computed from the file in this tree, the helper's scan, vera's section A (the statements and the
        reasons she re-approved) and section B (the real Squawk findings), never from the list itself."""
        helper = self.exc_helper()
        fx = self.pe_fixture(key)
        raw = (ROOT / self.REAL_MIGRATIONS[key]).read_bytes()
        text = raw.decode("utf-8")
        lines = text.split("\n")
        dynamic = [{"line": d["line"], "column": d["column"], "statement_sha256": d["statement_sha256"], "kind": helper.classify_finding(lines[d["line"] - 1], d["column"]), "reason": d["reason"]}
                   for d in fx["dynamic_findings"]]
        approvals = sorted(({"rule": s["rule"], "line": s["line"], "tool_sha256": self.PD_TOOL_SHA256, "config_sha256": self.PD_CONFIG_SHA256} for s in fx["squawk"]),
                           key=lambda a: (a["line"], a["rule"]))
        return {"path": self.REAL_MIGRATIONS[key], "sha256": hashlib.sha256(raw).hexdigest(), "approved_by": fx["ruling"]["row"], "matcher": 2,
                "raw_execute_words": [{"line": l, "column": c} for l, c in helper.scan_execute_words(text)], "dynamic_findings": dynamic, "squawk": approvals}

    def test_pe_a1_the_consolidated_fixture_equals_the_files_the_scan_and_the_real_reports(self):
        helper = self.exc_helper()
        for key, pinned in self.PE_KEYS.items():
            fx = self.pe_fixture(key)
            with self.subTest(migration=key):
                self.assertEqual((fx["ruling"]["row"], fx["ruling"]["path"], fx["ruling"]["sha256"], fx["migration_sha256"]), (pinned["row"], pinned["path"], pinned["sha256"], pinned["migration"]))
                raw = (ROOT / self.REAL_MIGRATIONS[key]).read_bytes()
                self.assertEqual(hashlib.sha256(raw).hexdigest(), pinned["migration"], "the file in this tree is the file vera ruled on")
                text = raw.decode("utf-8")
                lines = text.split("\n")
                self.assertEqual([(d["line"], d["column"]) for d in fx["dynamic_findings"]], helper.scan_dynamic_findings(text), "section A is exactly the scan's dynamic findings")
                self.assertEqual([d["statement_sha256"] for d in fx["dynamic_findings"]], self.PE_STATEMENTS[key])
                self.assertEqual([hashlib.sha256(lines[d["line"] - 1].strip().encode("utf-8")).hexdigest() for d in fx["dynamic_findings"]], self.PE_STATEMENTS[key],
                                 "the statements of the file hash to the ones she approved")
                self.assertEqual(sorted((s["rule"], s["line"]) for s in fx["squawk"]), sorted((f["rule_name"], f["line"]) for f in self.pd_report(key)), "section B is exactly the real report")
        self.assertEqual((len(self.pe_fixture("0001_audit")["dynamic_findings"]), len(self.pe_fixture("0001_audit")["squawk"]),
                          len(self.pe_fixture("0001_records")["dynamic_findings"]), len(self.pe_fixture("0001_records")["squawk"])), (3, 2, 5, 4))

    def test_pe_b1_the_production_list_carries_audit_and_records_with_the_two_kinds_kept_apart(self):
        for key in self.PE_KEYS:
            with self.subTest(migration=key):
                entry = self.pd_entry(key)
                self.assertIsNotNone(entry)
                self.assertEqual(entry, self.pe_expected_entry(key))
        with self.subTest(case="the platform entry and the three D1 entries are unchanged: the list holds six entries, each path once"):
            paths = [e["path"] for e in self.pd_list()]
            self.assertEqual(sorted(paths), sorted(self.REAL_MIGRATIONS.values()))

    def test_pe_b2_the_production_record_pins_audit_and_records_to_their_own_consolidated_rulings(self):
        record = json.loads((ROOT / "scripts/release-migration-approval.json").read_text())
        self.assertEqual(record["config_sha256"], hashlib.sha256((ROOT / "scripts/release-migration-exceptions.json").read_bytes()).hexdigest())
        pins = {p["migration_path"]: p for p in record["entries"]}
        self.assertEqual(len(record["entries"]), 6)
        for key, pinned in self.PE_KEYS.items():
            with self.subTest(migration=key):
                relative = self.REAL_MIGRATIONS[key]
                self.assertEqual(pins[relative], {"migration_path": relative, "migration_sha256": pinned["migration"], "approved_by": pinned["row"], "review_path": pinned["path"],
                                                  "review_sha256": pinned["sha256"]})
        with self.subTest(case="the platform pin is the frozen one"):
            self.assertEqual(pins[self.PLATFORM_PATH]["approved_by"], "729cbe80-cfed-4814-9ca7-9e7e7f99e191")
            self.assertEqual(pins[self.PLATFORM_PATH]["review_sha256"][:12], "14788a7ab60f")

    @unittest.skipUnless(Path("/path/to/docs/Program/ReviewService/2026-11-09-audit-consolidated-29aba9ba-vera/RULING.md").is_file(), "vera's review files are under /srv on the VM only")
    def test_pe_c1_the_final_pair_verifies_and_every_non_platform_migration_is_a_findings_candidate(self):
        verified = subprocess.run([sys.executable, "-I", str(ROOT / "scripts/release_manifest.py"), "verify-approval", "--root", str(ROOT)], text=True, capture_output=True, timeout=60)
        self.assertEqual(verified.returncode, 0, (verified.stdout, verified.stderr))
        for key, relative in self.REAL_MIGRATIONS.items():
            with self.subTest(migration=key):
                found = subprocess.run([sys.executable, "-I", str(ROOT / "scripts/release_manifest.py"), "exception-candidate", "--root", str(ROOT), "--migration", str(ROOT / relative)],
                                       text=True, capture_output=True, timeout=60)
                self.assertEqual((found.returncode, found.stdout.strip()), (0, "baseline" if key == "0001_platform" else "findings"), (found.stdout, found.stderr))

    @unittest.skipUnless(Path("/path/to/docs/Program/ReviewService/2026-11-09-audit-consolidated-29aba9ba-vera/RULING.md").is_file(), "vera's review files are under /srv on the VM only")
    def test_pe_d1_the_six_real_reports_replayed_against_the_production_list_pass_and_audit_and_records_cannot_cross(self):
        def compare(key, against):
            report = self.pd_report(key)
            for finding in report:
                finding["file"] = str(ROOT / self.REAL_MIGRATIONS[against])
            target = self.root / ("report-" + key + "-" + against + ".json")
            target.write_text(json.dumps(report))
            arguments = ["exception-squawk", "--migration", str(ROOT / self.REAL_MIGRATIONS[against]), "--input", str(target)]
            if against != "0001_platform":
                arguments += ["--root", str(ROOT)]
            return subprocess.run([sys.executable, "-I", str(ROOT / "scripts/release_manifest.py"), *arguments], text=True, capture_output=True, timeout=60)
        for key in self.REAL_MIGRATIONS:
            with self.subTest(migration=key, what="its own real report equals its approvals"):
                result = compare(key, key)
                self.assertEqual(result.returncode, 0, (key, result.stdout, result.stderr))
        for key, other in (("0001_audit", "0001_records"), ("0001_records", "0001_audit")):
            with self.subTest(migration=key, what="the other's real report cannot satisfy it: " + other):
                result = compare(other, key)
                self.assertNotEqual(result.returncode, 0, (key, other, result.stdout))

    def test_pe_e1_the_two_kinds_of_approval_are_separate_fields_with_their_own_exact_contents(self):
        for key in self.PE_KEYS:
            entry = self.pd_entry(key)
            with self.subTest(migration=key):
                self.assertIsNotNone(entry)
                self.assertTrue(entry["dynamic_findings"] and entry["squawk"], "both kinds are present")
                self.assertEqual([f["statement_sha256"] for f in entry["dynamic_findings"]], self.PE_STATEMENTS[key])
                self.assertTrue(all(set(f) == {"line", "column", "statement_sha256", "kind", "reason"} for f in entry["dynamic_findings"]))
                self.assertTrue(all(set(a) == {"rule", "line", "tool_sha256", "config_sha256"} for a in entry["squawk"]))
                self.assertEqual(entry["approved_by"], self.PE_KEYS[key]["row"], "one approved_by row per entry: the consolidated ruling's")

    @unittest.skipUnless(Path("/path/to/docs/Program/ReviewService/2026-11-09-audit-consolidated-29aba9ba-vera/RULING.md").is_file(), "vera's review files are under /srv on the VM only")
    def test_pe_f1_gate_4_passes_on_all_six_migrations_with_the_production_list_and_the_real_reports_replayed(self):
        # a whole release in a fixture root: the six REAL migrations, the PRODUCTION list and record, a stand-in that replays the six REAL reports (NOT the real binary)
        self.sq_clean_root()
        self.exc_fixture(v2=())
        shutil.copy(ROOT / "scripts/release-migration-exceptions.json", self.root / "scripts/release-migration-exceptions.json")
        shutil.copy(ROOT / "scripts/release-migration-approval.json", self.root / "scripts/release-migration-approval.json")
        answers = {}
        for key, relative in self.REAL_MIGRATIONS.items():
            (self.root / relative).parent.mkdir(parents=True, exist_ok=True)
            (self.root / relative).write_bytes((ROOT / relative).read_bytes())
            report = self.pd_report(key)
            for finding in report:
                finding["file"] = str(self.root / relative)
            answers[relative] = (1, json.dumps(report))
        self.sq_stub(answers)
        verified = self.exc_verify()
        self.assertEqual(verified.returncode, 0, (verified.stdout, verified.stderr))
        result = self.run_release("1.2.3")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertTrue((self.root / "dist/1.2.3").is_dir())
        self.assertEqual(self.calls().count("squawk"), 6)

    # ---- the mutant driver's SCREEN (--cell-only; cox W1-100): each mutant judged by its own named cell only, the baseline still runs every cell ----

    def driver_module(self):
        spec = importlib.util.spec_from_file_location("run_mutants_driver", ROOT / "scripts/release-fixtures/run_mutants.py")
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        return module

    def test_driver_a1_the_screen_reports_a_survivor_when_the_named_cell_passes_and_a_kill_only_when_it_fails(self):
        driver = self.driver_module()
        mini = self.root / "mini"
        (mini / "scripts/release-fixtures").mkdir(parents=True)
        shutil.copy(ROOT / "scripts/release-test.sh", mini / "scripts/release-test.sh")
        (mini / "scripts/release-fixtures/test_release.py").write_text(
            "import unittest\n"
            "class T(unittest.TestCase):\n"
            "    def test_passes(self):\n        self.assertTrue(True)\n"
            "    def test_fails(self):\n        self.assertTrue(False)\n"
            "    def test_errors(self):\n        raise RuntimeError('boom')\n"
            "if __name__ == '__main__':\n    unittest.main(verbosity=2)\n")
        def screened(name):
            mutant = {"id": "RM-X", "fails": name}
            cell = driver.cell_for(mutant, True)
            self.assertEqual(cell, name)
            result = driver.run_tests(mini, cell)
            return driver.judge(mutant, result.returncode, result.stdout + result.stderr)
        self.assertEqual(screened("test_fails"), "OK", "a named cell that fails is a kill")
        self.assertEqual(screened("test_passes"), "SURVIVED", "a named cell that PASSES is reported as a survivor, never as a kill")
        self.assertEqual(screened("test_errors"), "BROKEN OR WRONG TEST", "a named cell that errors is not a kill")
        self.assertIn(screened("test_not_there"), ("BROKEN OR WRONG TEST", "SURVIVED"), "a name that matches no cell is never a kill (an interpreter that exits 0 when no test ran reads SURVIVED)")
        with self.subTest(case="an error anywhere in the run is not a kill even when the named cell failed"):
            result = driver.run_tests(mini, None)
            out = result.stdout + result.stderr
            self.assertIn("FAIL: test_fails ", out)
            self.assertIn("ERROR: test_errors ", out)
            self.assertEqual(driver.judge({"id": "RM-X", "fails": "test_fails"}, result.returncode, out), "BROKEN OR WRONG TEST")
        with self.subTest(case="a cell of another name failing is not a kill of the named one"):
            result = driver.run_tests(mini, "test_fails")
            self.assertEqual(driver.judge({"id": "RM-X", "fails": "test_passes"}, result.returncode, result.stdout + result.stderr), "BROKEN OR WRONG TEST")

    def test_driver_a2_the_cell_selection_and_the_command_line(self):
        driver = self.driver_module()
        self.assertIsNone(driver.cell_for({"fails": "test_a"}, False), "without the screen an old mutant runs the whole suite")
        self.assertEqual(driver.cell_for({"fails": "test_a"}, True), "test_a")
        self.assertEqual(driver.cell_for({"fails": "test_a", "only": "test_b"}, False), "test_b", "a mutant's own only wins, screen or not")
        self.assertEqual(driver.cell_for({"fails": "test_a", "only": "test_b"}, True), "test_b")
        self.assertEqual(driver.parse_args([]), (False, None))
        self.assertEqual(driver.parse_args(["--cell-only"]), (True, None))
        self.assertEqual(driver.parse_args(["1", "21"]), (False, ("slice", 1, 21)))
        self.assertEqual(driver.parse_args(["--cell-only", "--ids", "RM-01,RM-02"]), (True, ("ids", ["RM-01", "RM-02"])))
        self.assertEqual(driver.parse_args(["--ids", "RM-01", "--cell-only"]), (True, ("ids", ["RM-01"])))
        for bad in (["--ids"], ["1"], ["1", "x"], ["--ids", "RM-01", "extra"], ["--unknown"]):
            with self.subTest(argv=bad):
                with self.assertRaises(SystemExit):
                    driver.parse_args(bad)

if __name__ == "__main__":
    if len(sys.argv) == 3 and sys.argv[1] == "--build-to":
        case = ReleaseTest("test_complete_release_has_reproducible_verified_assets")
        case.setUp()
        try:
            case.valid_podman()
            if os.environ.get("CORTEX_TEST_REAL_CLI"):
                case.env["CORTEX_RELEASE_CLI"] = os.environ["CORTEX_TEST_REAL_CLI"]
            result = case.run_release("1.2.3")
            if result.returncode:
                sys.stderr.write(result.stdout + result.stderr)
                raise SystemExit(result.returncode)
            shutil.copytree(case.root / "dist/1.2.3", sys.argv[2])
        finally:
            case.tearDown()
    else:
        unittest.main(verbosity=2)
