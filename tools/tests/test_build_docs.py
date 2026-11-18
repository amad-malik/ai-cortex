"""Piece 13's tool cells (tate; cox W1-29). Standard library only. Run from the repo root, through
cv2-admit, with no database:

    python3 -I -m unittest discover -s tools/tests -v

Work directories live under the repo's target/ (git-ignored), never under /tmp.
"""

import copy
import importlib.util
import json
import os
import shutil
import stat
import subprocess
import tempfile
import unittest
import unittest.mock
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TOOLS = ROOT / "tools"
CONTRACT = TOOLS / "contract" / "cortex-v2-skeleton.openapi.r2.7.json"


def _load_module():
    spec = importlib.util.spec_from_file_location("render_reference", TOOLS / "render_reference.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


rr = _load_module()


def workdir():
    base = ROOT / "target" / "tools-tests"
    base.mkdir(parents=True, exist_ok=True)
    return Path(tempfile.mkdtemp(prefix="t.", dir=base))


def mini():
    """A two-operation contract that passes every check."""
    return {
        "openapi": "3.1.0",
        "info": {"title": "mini", "version": "0"},
        "paths": {
            "/v2/a/{id}": {
                "get": {
                    "operationId": "a.show",
                    "summary": "Show one A. The rest of the summary.",
                    "x-scope": "a:read",
                    "x-class": "work",
                    "x-mutates": False,
                    "x-cli": "cortex a show <id>",
                    "x-errors": [],
                    "x-target": "None",
                    "security": [{"workerToken": []}],
                    "parameters": [
                        {"name": "id", "in": "path", "required": True, "schema": {"type": "string", "format": "uuid"},
                         "description": "the A | its id"}
                    ],
                    "responses": {"200": {"description": "the A"}, "404": {"$ref": "#/components/responses/NotFound"}},
                }
            },
            "/v2/b": {
                "post": {
                    "operationId": "b.add",
                    "summary": "Add or remove a B",
                    "x-scope": "b:write",
                    "x-class": "admin",
                    "x-mutates": True,
                    "x-cli": "cortex b add|remove <x> [--flag a|b]   (alias: cortex-b)",
                    "x-errors": ["409 b_taken"],
                    "x-target": "SelfOrProjectLead",
                    "security": [],
                    "responses": {"201": {"description": "added"}, "409": {"description": "taken"}},
                },
                "get": {
                    "operationId": "b.live",
                    "summary": "Liveness",
                    "x-scope": "none",
                    "x-class": "public",
                    "x-mutates": False,
                    "x-cli": "-",
                    "x-errors": [],
                    "x-target": "None",
                    "security": [],
                    "responses": {"200": {"description": "up"}, "503": {"description": "down"}},
                },
            },
        },
        "components": {"responses": {"NotFound": {"description": "not found"}}},
        "x-error-codes": {
            "platform_and_chain": {"not_found": 404, "invalid_request": [400, 422]},
            "pieces": {"b_taken": 409},
            "deferred": {},
            "retired_in_r2": {},
            "rule": "the rule text",
        },
    }


def op(c, path, method):
    return c["paths"][path][method]


class Checks(unittest.TestCase):
    """The contract checks of mvp/docs/S3-DOC-PACK-SPEC.md section 5, items 2 and 3, plus x-cli."""

    def test_c01_the_mini_contract_and_the_pinned_r2_7_pass(self):
        self.assertEqual(rr.check(mini()), [])
        self.assertEqual(rr.check(rr.load(CONTRACT)), [])

    def test_c02_an_operation_with_no_error_entries_fails(self):
        c = mini()
        del op(c, "/v2/a/{id}", "get")["x-errors"]
        self.assertTrue(any("a.show" in p and "x-errors" in p for p in rr.check(c)))
        c = mini()
        op(c, "/v2/a/{id}", "get")["responses"] = {"200": {"description": "ok"}}
        self.assertTrue(any("a.show" in p and "error response" in p for p in rr.check(c)))

    def test_c03_an_operation_with_no_auth_requirement_fails_but_explicit_none_passes(self):
        c = mini()
        del op(c, "/v2/b", "post")["security"]
        self.assertTrue(any("b.add" in p and "security" in p for p in rr.check(c)))
        self.assertEqual(op(mini(), "/v2/b", "post")["security"], [])
        self.assertEqual(rr.check(mini()), [])

    def test_c05_a_verb_or_key_the_tool_does_not_know_fails_closed(self):
        # hale F-4: a HEAD, OPTIONS or TRACE operation is checked like any other, and a path-item
        # key that is neither a verb nor a known OpenAPI field is refused, never skipped.
        c = mini()
        c["paths"]["/v2/a/{id}"]["head"] = {"operationId": "a.head", "responses": {"200": {"description": "ok"}}}
        problems = rr.check(c)
        for want in ["a.head: no x-errors", "a.head: no security", "a.head: no x-cli"]:
            self.assertTrue(any(want in p for p in problems), (want, problems))
        self.assertIn("HEAD /v2/a/{id}", rr.inventory(c))
        c = mini()
        c["paths"]["/v2/b"]["x-future"] = {"responses": {}}
        self.assertTrue(any("/v2/b" in p and "x-future" in p for p in rr.check(c)), rr.check(c))

    def test_c04_an_operation_with_no_cli_entry_fails_and_dash_means_none(self):
        c = mini()
        del op(c, "/v2/a/{id}", "get")["x-cli"]
        self.assertTrue(any("a.show" in p and "x-cli" in p for p in rr.check(c)))
        self.assertNotIn("b live", rr.cli_commands(mini()))


class Cli(unittest.TestCase):
    """Check 4 of section 5: every CLI subcommand appears in the captured reference."""

    def test_l01_parse_the_contracts_cli_forms(self):
        self.assertEqual(rr.parse_cli("cortex boot [--budget N] [--format md|json]"), ["boot"])
        self.assertEqual(rr.parse_cli("cortex worker move|promote|demote"),
                         ["worker move", "worker promote", "worker demote"])
        self.assertEqual(rr.parse_cli("cortex log <type> ... | cortex rules set"), ["log", "rules set"])
        self.assertEqual(rr.parse_cli("cortex handoff accept <id>   (v1 form kept as alias: cortex-x)"),
                         ["handoff accept"])
        self.assertEqual(rr.parse_cli("cortex token issue|reissue --worker <id>"),
                         ["token issue", "token reissue"])
        self.assertEqual(rr.parse_cli("-"), [])
        # hale N-1: a comment ends its own command only, not the commands after it.
        self.assertEqual(rr.parse_cli("cortex a x (a note) | cortex b y"), ["a x", "b y"])

    def test_l02_the_pinned_contract_names_these_subcommands_in_order(self):
        self.assertEqual(rr.cli_commands(rr.load(CONTRACT)), [
            "doctor", "projects", "roster", "team add", "worker add", "worker retire",
            "worker move", "worker promote", "worker demote", "whoami", "token revoke-self",
            "token issue", "token reissue", "token list", "token inspect", "token revoke",
            "audit list", "log", "rules set", "record show", "record supersede",
            "record invalidate", "rules", "search", "handoff create", "handoff mine",
            "handoff list", "handoff show", "handoff claim", "handoff release", "handoff return",
            "handoff accept", "handoff rework", "handoff cancel", "boot",
        ])

    def _fake_cortex(self, d, answers):
        script = d / "cortex"
        lines = ["#!/bin/sh", 'last=""; for a in "$@"; do last="$a"; done',
                 '[ "$last" = "--help" ] || exit 2', 'cmd="$*"', 'case "$cmd" in']
        for a in answers:
            lines.append(f'  "{a} --help") echo "usage: cortex {a} [options]"; echo "  {a}: help text"; exit 0 ;;')
        lines += ['  "--help") echo "usage: cortex <command>"; exit 0 ;;', "esac", "exit 2", ""]
        script.write_text("\n".join(lines))
        script.chmod(script.stat().st_mode | stat.S_IXUSR)
        return script

    def test_l03_a_subcommand_the_binary_does_not_answer_fails(self):
        d = workdir()
        try:
            fake = self._fake_cortex(d, ["a show"])
            c = mini()
            op(c, "/v2/b", "post")["x-cli"] = "cortex b add"
            text, problems = rr.capture(c, str(fake))
            self.assertTrue(any("b add" in p for p in problems), problems)
            fake = self._fake_cortex(d, ["a show", "b add"])
            text, problems = rr.capture(c, str(fake))
            self.assertEqual(problems, [])
            self.assertIn("## cortex a show\n", text)
            self.assertIn("## cortex b add\n", text)
            self.assertIn("usage: cortex b add [options]", text)
        finally:
            shutil.rmtree(d)

    def _odd_cortex(self, d, body):
        script = d / "cortex-odd"
        script.write_text("#!/bin/sh\n" + body + "\n")
        script.chmod(script.stat().st_mode | stat.S_IXUSR)
        return script

    def test_l05_a_help_that_prints_text_but_exits_non_zero_fails(self):
        # hale F-3 (b), M08.
        d = workdir()
        try:
            fake = self._odd_cortex(d, 'echo "usage: cortex"; exit 2')
            text, problems = rr.capture(mini(), str(fake))
            self.assertTrue(any("exit 2" in p for p in problems), problems)
        finally:
            shutil.rmtree(d)

    def test_l06_a_help_that_exits_zero_with_no_text_fails(self):
        # hale F-3 (c), M16: and the message says why.
        d = workdir()
        try:
            fake = self._odd_cortex(d, "exit 0")
            text, problems = rr.capture(mini(), str(fake))
            self.assertTrue(problems)
            self.assertTrue(all("empty answer" in p for p in problems), problems)
        finally:
            shutil.rmtree(d)

    def test_l04_no_binary_is_a_failure_not_a_skip(self):
        text, problems = rr.capture(mini(), "")
        self.assertTrue(problems)
        text, problems = rr.capture(mini(), str(ROOT / "target" / "no-such-cortex"))
        self.assertTrue(problems)


class Inventory(unittest.TestCase):
    """Check 1 of section 5, the hook for sam's SLOT-03: the mounted inventory equals S3."""

    def test_i01_the_inventory_is_method_and_path(self):
        self.assertEqual(rr.inventory(mini()), ["GET /v2/a/{id}", "GET /v2/b", "POST /v2/b"])

    def test_i02_a_difference_in_either_direction_fails(self):
        self.assertEqual(rr.inventory_diff(mini(), ["GET /v2/a/{id}", "GET /v2/b", "POST /v2/b"]), [])
        extra = rr.inventory_diff(mini(), ["GET /v2/a/{id}", "GET /v2/b", "POST /v2/b", "DELETE /v2/b"])
        self.assertTrue(any("DELETE /v2/b" in p and "not in the contract" in p for p in extra), extra)
        missing = rr.inventory_diff(mini(), ["GET /v2/a/{id}", "GET /v2/b"])
        self.assertTrue(any("POST /v2/b" in p and "not mounted" in p for p in missing), missing)

    def test_i03_a_line_listed_twice_fails(self):
        # hale F-3 (a), M07.
        twice = rr.inventory_diff(mini(), ["GET /v2/a/{id}", "GET /v2/b", "POST /v2/b", "GET /v2/b"])
        self.assertTrue(any("GET /v2/b" in p and "twice" in p for p in twice), twice)


class Render(unittest.TestCase):
    """The page structure of section 3, from the contract only, deterministically."""

    def test_r01_the_same_contract_gives_the_same_bytes(self):
        c = rr.load(CONTRACT)
        a = rr.render(c, "x.json", "0" * 64)
        b = rr.render(copy.deepcopy(c), "x.json", "0" * 64)
        self.assertEqual(a.encode(), b.encode())

    def test_r02_every_operation_in_contract_order_with_the_seven_parts(self):
        c = rr.load(CONTRACT)
        text = rr.render(c, "cortex-v2-skeleton.openapi.r2.7.json", "1d2c4b2f" + "0" * 56)
        ids = [o["operationId"] for p in c["paths"].values() for m, o in p.items() if isinstance(o, dict) and "operationId" in o]
        self.assertEqual(len(ids), 33)
        positions = [text.index(f"### `{i}`") for i in ids]
        self.assertEqual(positions, sorted(positions), "contract order")
        boot = text[text.index("### `boot.get`"):]
        boot = boot[: boot.find("\n### ", 1) if "\n### " in boot[1:] else len(boot)]
        for part in ["**Task:**", "**Who may call:**", "**Request:**", "**Response:**", "**Failures:**",
                     "**Idempotency:**", "**CLI:**"]:
            self.assertIn(part, boot)
        self.assertIn("`self:read`", boot)
        self.assertIn("boot_budget_too_small", boot)
        self.assertIn("cortex boot [--budget N] [--format md|json]", boot)
        # A parameter's description may sit in its schema (budget's does), and a lone maximum
        # reads as a bound, not as an open range.
        self.assertIn("bytes; over 262144 is 422 invalid_request", boot)
        self.assertIn("at most 262144", boot)
        self.assertNotIn("..262144", boot)
        self.assertNotIn(" ; ", boot)
        self.assertTrue(text.startswith("# Cortex API reference (generated)\n"))
        self.assertIn("1d2c4b2f", text.splitlines()[2])
        self.assertIn("## Errors\n", text)
        self.assertIn("| `boot_budget_too_small` | 422 |", text)
        # cox W1-34 (S3 r2.7): a code several pieces declare has one status, title and fix; the
        # Errors catalogue lists each with them.
        shared = text[text.index("### Codes shared by several pieces\n"):]
        self.assertIn("| `reason_required` | 422 | Reason required | Send the reason this operation requires, in the form its documentation gives. |", shared)
        self.assertIn("| `target_retired` | 409 | Worker retired | Use an active worker. |", shared)

    def test_r05_shared_codes_are_listed_with_title_and_fix_and_absent_means_none(self):
        c = mini()
        c["x-shared-codes"] = {"rule": "one status, title and fix", "codes": {
            "b_taken": {"status": 409, "title": "B | taken", "fix": "Pick\nanother B."}}}
        text = rr.render(c, "mini.json", "0" * 64)
        shared = text[text.index("### Codes shared by several pieces\n"):]
        self.assertIn("one status, title and fix", shared)
        self.assertIn("| `b_taken` | 409 | B \\| taken | Pick another B. |", shared)
        none = rr.render(mini(), "mini.json", "0" * 64)
        self.assertIn("### Codes shared by several pieces\n\nNone.\n", none)

    def test_r03_table_cells_escape_the_pipe_and_drop_newlines(self):
        text = rr.render(mini(), "mini.json", "0" * 64)
        self.assertIn("the A \\| its id", text)
        self.assertNotIn("the A | its id", text)

    def test_r04_mutating_operations_say_idempotency_key_and_reads_say_safe(self):
        text = rr.render(mini(), "mini.json", "0" * 64)
        b_add = text[text.index("### `b.add`"):]
        self.assertIn("Idempotency-Key", b_add[: b_add.index("**CLI:**")])
        a_show = text[text.index("### `a.show`"):]
        self.assertIn("not a write", a_show[: a_show.index("**CLI:**")])


class Build(unittest.TestCase):
    """tools/build-docs.sh end to end, in a copy of the repo layout under target/."""

    def setUp(self):
        self.d = workdir()
        self.repo = self.d / "repo"
        (self.repo / "tools" / "contract").mkdir(parents=True)
        (self.repo / "docs" / "generated").mkdir(parents=True)
        for f in ["build-docs.sh", "render_reference.py"]:
            shutil.copy2(TOOLS / f, self.repo / "tools" / f)
        (self.repo / "tools" / "contract" / "mini.json").write_text(json.dumps(mini(), indent=1))
        subprocess.run(["sh", "-c", "sha256sum mini.json > SHA256SUMS"], cwd=self.repo / "tools" / "contract", check=True)
        self.fake = Cli()._fake_cortex(self.d, ["a show", "b add", "b remove"])

    def tearDown(self):
        shutil.rmtree(self.d)

    def run_build(self, *args, cortex=None):
        env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "CORTEX_BIN": str(cortex or self.fake)}
        return subprocess.run(["bash", str(self.repo / "tools" / "build-docs.sh"), *args],
                              cwd=self.repo, env=env, capture_output=True, text=True)

    def test_b01_write_then_check_passes_and_names_the_skipped_inventory(self):
        w = self.run_build("--write")
        self.assertEqual(w.returncode, 0, w.stderr)
        for f in ["API-REFERENCE.md", "CLI-REFERENCE.md"]:
            self.assertTrue((self.repo / "docs" / "generated" / f).is_file(), f)
        c = self.run_build()
        self.assertEqual(c.returncode, 0, c.stderr)
        self.assertIn("SKIPPED", c.stdout + c.stderr, "the inventory check says it did not run")

    def test_b02_stale_generated_pages_fail(self):
        self.assertEqual(self.run_build("--write").returncode, 0)
        page = self.repo / "docs" / "generated" / "API-REFERENCE.md"
        page.write_text(page.read_text() + "hand edit\n")
        c = self.run_build()
        self.assertNotEqual(c.returncode, 0)
        self.assertIn("stale", c.stderr)
        self.assertIn("--write", c.stderr)

    def test_b03_a_contract_that_does_not_match_its_pin_fails(self):
        p = self.repo / "tools" / "contract" / "mini.json"
        p.write_text(p.read_text() + " ")
        c = self.run_build("--write")
        self.assertNotEqual(c.returncode, 0)
        self.assertIn("pin", c.stderr)

    def test_b04_no_cortex_binary_fails(self):
        c = self.run_build("--write", cortex=self.d / "missing")
        self.assertNotEqual(c.returncode, 0)
        self.assertIn("CORTEX_BIN", c.stderr)

    def test_b05_strict_fails_without_an_inventory_and_an_inventory_is_diffed(self):
        w = self.run_build("--write")
        self.assertEqual(w.returncode, 0)
        # hale F-1: the final line is one of two fixed sentences, never the file's path.
        self.assertEqual(w.stdout.splitlines()[-1],
                         "build-docs: PASS-WITH-NOT-RUN: the route inventory check (item 1) did not run")
        self.assertNotEqual(self.run_build("--strict").returncode, 0)
        inv = self.d / "inv.txt"
        inv.write_text("GET /v2/a/{id}\nGET /v2/b\nPOST /v2/b\n")
        ok = self.run_build("--strict", "--inventory", str(inv))
        self.assertEqual(ok.returncode, 0)
        self.assertEqual(ok.stdout.splitlines()[-1], "build-docs: PASS")
        inv.write_text("GET /v2/a/{id}\nGET /v2/b\n")
        bad = self.run_build("--inventory", str(inv))
        self.assertNotEqual(bad.returncode, 0)
        self.assertIn("POST /v2/b", bad.stderr)

    def test_b08_an_unreadable_inventory_file_says_so(self):
        # hale F-2: no traceback, and not "differs".
        self.assertEqual(self.run_build("--write").returncode, 0)
        r = self.run_build("--inventory", str(self.d / "no-such-inventory.txt"))
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("cannot read", r.stderr)
        self.assertNotIn("Traceback", r.stderr)
        self.assertNotIn("differs", r.stderr)

    def test_b09_a_sha256sums_naming_two_files_fails(self):
        # hale F-3 (d), M18.
        sums = self.repo / "tools" / "contract" / "SHA256SUMS"
        sums.write_text(sums.read_text() * 2)
        r = self.run_build("--write")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("exactly one contract", r.stderr)

    def test_b06_a_failing_contract_check_fails_the_build(self):
        c = mini()
        del op(c, "/v2/b", "post")["security"]
        p = self.repo / "tools" / "contract" / "mini.json"
        p.write_text(json.dumps(c, indent=1))
        subprocess.run(["sh", "-c", "sha256sum mini.json > SHA256SUMS"], cwd=p.parent, check=True)
        r = self.run_build("--write")
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("b.add", r.stderr)

    def test_b07_the_work_dir_is_under_the_repo_and_is_removed(self):
        self.assertEqual(self.run_build("--write").returncode, 0)
        left = list((self.repo / "target").glob("docs-build.*")) if (self.repo / "target").exists() else []
        self.assertEqual(left, [])

    # ---- quill's gap 4 (row 0c83b139): no v2 cortex binary exists in 0.2.001, so the CLI capture
    # is a SKIPPED check like the inventory, not a red build. A SET variable is still a claim.

    def run_without_bin(self, *args, bin_env=None):
        env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin")}
        if bin_env is not None:
            env["CORTEX_BIN"] = bin_env
        return subprocess.run(["bash", str(self.repo / "tools" / "build-docs.sh"), *args],
                              cwd=self.repo, env=env, capture_output=True, text=True)

    def test_b10_without_cortex_bin_the_cli_capture_is_skipped_and_named(self):
        w = self.run_without_bin("--write")
        self.assertEqual(w.returncode, 0, w.stderr)
        self.assertIn("SKIPPED: the CLI capture (item 4)", w.stdout)
        self.assertEqual(w.stdout.splitlines()[-1],
                         "build-docs: PASS-WITH-NOT-RUN: the route inventory check (item 1) and the CLI capture (item 4) did not run")
        gen = self.repo / "docs" / "generated"
        self.assertTrue((gen / "API-REFERENCE.md").is_file())
        self.assertFalse((gen / "CLI-REFERENCE.md").exists(), "no capture, no CLI page")
        inv = self.d / "inv.txt"
        inv.write_text("GET /v2/a/{id}\nGET /v2/b\nPOST /v2/b\n")
        r = self.run_without_bin("--inventory", str(inv))
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(r.stdout.splitlines()[-1],
                         "build-docs: PASS-WITH-NOT-RUN: the CLI capture (item 4) did not run")

    def test_b11_strict_makes_a_skipped_cli_capture_a_failure(self):
        inv = self.d / "inv.txt"
        inv.write_text("GET /v2/a/{id}\nGET /v2/b\nPOST /v2/b\n")
        r = self.run_without_bin("--strict", "--inventory", str(inv))
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("--strict", r.stderr)
        self.assertIn("CLI capture", r.stderr)

    def test_b12_an_empty_cortex_bin_is_unset_but_a_set_bad_one_still_fails(self):
        e = self.run_without_bin("--write", bin_env="")
        self.assertEqual(e.returncode, 0, e.stderr)
        self.assertIn("SKIPPED: the CLI capture (item 4)", e.stdout)
        notexec = self.d / "not-executable"
        notexec.write_text("#!/bin/sh\necho hi\n")
        notexec.chmod(0o644)
        for bad in (str(notexec), str(self.d / "missing"), str(self.d)):
            r = self.run_without_bin("--write", bin_env=bad)
            self.assertNotEqual(r.returncode, 0, bad)
            self.assertIn("CORTEX_BIN", r.stderr, bad)

    def test_b13_a_skipped_capture_leaves_an_earlier_cli_page_alone_but_a_stale_api_page_still_fails(self):
        self.assertEqual(self.run_build("--write").returncode, 0)
        gen = self.repo / "docs" / "generated"
        cli_before = (gen / "CLI-REFERENCE.md").read_bytes()
        ok = self.run_without_bin()
        self.assertEqual(ok.returncode, 0, ok.stderr)
        self.assertEqual((gen / "CLI-REFERENCE.md").read_bytes(), cli_before)
        (gen / "API-REFERENCE.md").write_text((gen / "API-REFERENCE.md").read_text() + "hand edit\n")
        stale = self.run_without_bin()
        self.assertNotEqual(stale.returncode, 0)
        self.assertIn("stale", stale.stderr)

    def test_b14_a_run_with_the_binary_and_the_inventory_is_still_a_plain_pass(self):
        inv = self.d / "inv.txt"
        inv.write_text("GET /v2/a/{id}\nGET /v2/b\nPOST /v2/b\n")
        self.assertEqual(self.run_build("--write").returncode, 0)
        r = self.run_build("--strict", "--inventory", str(inv))
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(r.stdout.splitlines()[-1], "build-docs: PASS")


def section(text, title):
    """The text of one level-2 section of the page, from its heading to the next level-2 heading."""
    head = f"\n## {title}\n"
    start = text.find(head)
    if start < 0:
        return None
    end = text.find("\n## ", start + len(head))
    return text[start: end if end >= 0 else len(text)]


def norm(s):
    return " ".join(str(s).split())


def rule_no(key):
    return int(key.split()[0].split("-")[1])


class Gap1RouteRules(unittest.TestCase):
    """Quill's gap 1 (row 0c83b139): the 20 route rules, x-rules, are on the page."""

    def test_x01_every_x_rule_of_the_pinned_contract_has_a_heading_and_its_full_text_in_r_order(self):
        c = rr.load(CONTRACT)
        rules = c["x-rules"]
        self.assertEqual(len(rules), 20, "the pinned r2.7 contract has R-1..R-20")
        sec = section(rr.render(c, "x.json", "0" * 64), "Route rules")
        self.assertIsNotNone(sec, "no '## Route rules' section")
        for key, body in rules.items():
            self.assertIn(f"\n### {key}\n", sec, key)
            self.assertIn(norm(body), sec, f"{key}: the full text")
        order = sorted(rules, key=rule_no)
        pos = [sec.index(f"\n### {k}\n") for k in order]
        self.assertEqual(pos, sorted(pos), "R-1, R-2, ... R-10, ... R-20, not the file's order or text order")

    def test_x02_rules_are_in_number_order_and_none_is_said_when_there_are_none(self):
        c = mini()
        c["x-rules"] = {"R-10 ten": "ten  text\nsecond line", "R-9 nine": "nine text", "R-1 one": "one text"}
        sec = section(rr.render(c, "mini.json", "0" * 64), "Route rules")
        self.assertIsNotNone(sec)
        self.assertEqual([sec.index(f"### {k}\n") for k in ("R-1 one", "R-9 nine", "R-10 ten")],
                         sorted(sec.index(f"### {k}\n") for k in ("R-1 one", "R-9 nine", "R-10 ten")))
        self.assertLess(sec.index("### R-9 nine"), sec.index("### R-10 ten"))
        self.assertIn("ten text second line", sec)
        none = section(rr.render(mini(), "mini.json", "0" * 64), "Route rules")
        self.assertEqual(none, "\n## Route rules\n\nNone.\n")

    def test_x14_a_key_that_is_not_r_n_sorts_after_the_numbered_rules_in_file_order(self):
        # hale r0 N-1 (her mutant R02): the docstring promises "after them, in file order".
        c = mini()
        c["x-rules"] = {"note-b": "b text", "R-2 two": "two", "note-a": "a text", "R-1 one": "one"}
        sec = section(rr.render(c, "mini.json", "0" * 64), "Route rules")
        heads = [l for l in sec.splitlines() if l.startswith("### ")]
        self.assertEqual(heads, ["### R-1 one", "### R-2 two", "### note-b", "### note-a"])

    def test_x15_a_rule_heading_is_one_line_with_the_pipe_escaped(self):
        # hale r0 N-1 (her mutant R04).
        c = mini()
        c["x-rules"] = {"R-98 a|b\nc": "text"}
        sec = section(rr.render(c, "mini.json", "0" * 64), "Route rules")
        self.assertIn("\n### R-98 a\\|b c\n", sec)


class Gap2LabelGrammars(unittest.TestCase):
    """Quill's gap 2: all four label grammars, with where each is checked."""

    def rows(self, c):
        sec = section(rr.render(c, "x.json", "0" * 64), "Label grammars")
        self.assertIsNotNone(sec, "no '## Label grammars' section")
        return sec

    def test_x03_all_four_grammars_with_pattern_limit_and_source(self):
        c = rr.load(CONTRACT)
        g = c["x-label-grammar"]
        self.assertEqual(set(g), {"name", "job_role", "revoke_code", "audit_action"})
        sec = self.rows(c)
        for key, v in g.items():
            hits = [l for l in sec.splitlines() if l.startswith(f"| `{key}` |")]
            self.assertEqual(len(hits), 1, key)
            self.assertIn(f"`{rr.cell(v['pattern'])}`", hits[0], key)
            self.assertIn(f"| {v['maxLength']} |", hits[0], key)
            self.assertIn(rr.cell(v["source"]), hits[0], key)

    def test_x04_each_grammar_names_the_fields_it_is_checked_against(self):
        sec = self.rows(rr.load(CONTRACT))
        row = {k: next(l for l in sec.splitlines() if l.startswith(f"| `{k}` |"))
               for k in ("name", "job_role", "revoke_code", "audit_action")}
        for field in ("WorkerCreate.job_roles[]", "TeamAssign.job_roles[]", "Worker.job_roles[]", "Whoami.job_roles[]"):
            self.assertIn(f"`{field}`", row["job_role"], field)
        self.assertIn("`TokenRevoke.reason`", row["revoke_code"])
        self.assertIn("`AuditRecord.action`", row["audit_action"])
        for field in ("Project.name", "Team.name", "TeamCreate.name", "Worker.name", "WorkerCreate.name", "Whoami.name"):
            self.assertIn(f"`{field}`", row["name"], field)
        # the one query parameter the contract checks with the name grammar
        self.assertIn("`identity.workers_list` parameter `name`", row["name"])
        self.assertNotIn("TokenRevoke.reason", row["name"])

    def test_x05_a_small_contract_with_a_grammar_and_no_field_says_so_and_none_says_none(self):
        c = mini()
        c["x-label-grammar"] = {"tag": {"pattern": "^[a-z|]+$", "maxLength": 8, "source": "S9 | tags"},
                                "orphan": {"pattern": "^[0-9]+$", "maxLength": 4, "source": "S9"}}
        c["components"]["schemas"] = {
            "A": {"type": "object", "properties": {"t": {"type": "string", "pattern": "^[a-z|]+$"},
                                                   "ts": {"type": "array", "items": {"type": "string", "pattern": "^[a-z|]+$"}}}}}
        sec = self.rows(c)
        tag = next(l for l in sec.splitlines() if l.startswith("| `tag` |"))
        self.assertIn("`A.t`", tag)
        self.assertIn("`A.ts[]`", tag)
        self.assertIn("^[a-z\\|]+$", tag, "a pipe in a pattern is escaped in the cell")
        self.assertIn("S9 \\| tags", tag)
        orphan = next(l for l in sec.splitlines() if l.startswith("| `orphan` |"))
        self.assertIn("no field in this contract", orphan)
        none = section(rr.render(mini(), "mini.json", "0" * 64), "Label grammars")
        self.assertEqual(none, "\n## Label grammars\n\nNone.\n")

    def test_x16_the_header_and_separator_of_the_table_are_pinned(self):
        # hale r0 F-2 (her mutant R15: "At most (bytes)" survived every cell).
        lines = self.rows(rr.load(CONTRACT)).splitlines()
        i = lines.index("| Grammar | Pattern | At most (characters) | Source | Applies to |")
        self.assertEqual(lines[i + 1], "|---|---|---|---|---|")

    def test_x17_each_row_has_the_five_columns_in_order(self):
        # hale r0 F-2 (her mutant R13: maxLength and source swapped survived every cell).
        c = rr.load(CONTRACT)
        lines = self.rows(c).splitlines()
        for key, v in c["x-label-grammar"].items():
            line = next(l for l in lines if l.startswith(f"| `{key}` |"))
            cols = line[2:-2].split(" | ")
            self.assertEqual(len(cols), 5, key)
            self.assertEqual(cols[:4], [f"`{key}`", f"`{rr.cell(v['pattern'])}`", str(v["maxLength"]),
                                        rr.cell(v["source"])], key)
            self.assertTrue(cols[4].startswith("`"), key)

    def test_x18_a_grammar_without_a_pattern_matches_no_site_and_stays_one_short_row(self):
        # hale r0 F-1: pattern None matched every dict without a pattern (1,310 sites, a 59,911-character row).
        c = rr.load(CONTRACT)
        for bad in (None, "", 5):
            self.assertEqual(rr._pattern_sites(c, bad), [], repr(bad))
        c["x-label-grammar"] = {"nopat": {"maxLength": 8, "source": "test"}}
        row = next(l for l in self.rows(c).splitlines() if l.startswith("| `nopat` |"))
        self.assertLess(len(row), 200)
        self.assertIn("no field in this contract", row)

    def test_x19_one_site_is_listed_once_and_a_component_parameter_is_named(self):
        # hale r0 N-1 (her mutants R11 and R16, equivalent on the real contract).
        c = mini()
        c["x-label-grammar"] = {"tag": {"pattern": "^[a-z]+$", "maxLength": 8, "source": "S9"}}
        op(c, "/v2/b", "post")["parameters"] = [
            {"name": "t", "in": "query", "schema": {"type": "string", "pattern": "^[a-z]+$"}},
            {"name": "t", "in": "header", "schema": {"type": "string", "pattern": "^[a-z]+$"}}]
        c["components"]["parameters"] = {"P": {"name": "p", "in": "query",
                                               "schema": {"type": "string", "pattern": "^[a-z]+$"}}}
        tag = next(l for l in self.rows(c).splitlines() if l.startswith("| `tag` |"))
        self.assertEqual(tag.count("`b.add` parameter `t`"), 1, tag)
        self.assertIn("parameter `P`", tag)


class Gap3Notes(unittest.TestCase):
    """Quill's gap 3: x-notes, the date rule and the body-size note, are on the page."""

    def test_x06_every_public_x_note_of_the_pinned_contract_with_its_full_text_in_contract_order(self):
        c = rr.load(CONTRACT)
        notes = c["x-notes"]
        self.assertEqual(list(notes), ["borrowed-code", "datetime-rule", "max-body"])
        sec = section(rr.render(c, "x.json", "0" * 64), "Notes")
        self.assertIsNotNone(sec, "no '## Notes' section")
        public = {k: v for k, v in notes.items() if k != "borrowed-code"}
        self.assertEqual(list(public), ["datetime-rule", "max-body"])
        for key, body in public.items():
            self.assertIn(f"\n### {key}\n", sec, key)
            self.assertIn(norm(body), sec, f"{key}: the full text")
        pos = [sec.index(f"\n### {k}\n") for k in public]
        self.assertEqual(pos, sorted(pos), "contract order")

    def test_x11_the_internal_borrowed_code_note_is_not_on_the_client_page(self):
        # cox 18:56Z: "borrowed-code" is the internal D94 licensing note; left off.
        c = rr.load(CONTRACT)
        text = rr.render(c, "x.json", "0" * 64)
        self.assertNotIn("borrowed-code", text)
        self.assertNotIn(norm(c["x-notes"]["borrowed-code"]), text)
        self.assertNotIn("INTERIM(ponytail)", text)

    def test_x12_the_internal_keys_are_one_constant_named_in_the_docstring(self):
        self.assertEqual(rr.INTERNAL_NOTES, ("borrowed-code",))
        for key in rr.INTERNAL_NOTES:
            self.assertIn(key, rr.__doc__, "the docstring says which note keys are internal and skipped")
        self.assertIn("INTERNAL_NOTES", rr.__doc__)

    def test_x13_a_key_in_the_constant_is_skipped_and_only_internal_notes_give_none(self):
        c = mini()
        c["x-notes"] = {"public-one": "visible  text", "secret": "do not show", "public-two": "also visible"}
        with unittest.mock.patch.object(rr, "INTERNAL_NOTES", ("secret",)):
            sec = section(rr.render(c, "mini.json", "0" * 64), "Notes")
            self.assertNotIn("secret", sec)
            self.assertNotIn("do not show", sec)
            self.assertLess(sec.index("### public-one"), sec.index("### public-two"))
            self.assertIn("visible text", sec)
            c["x-notes"] = {"secret": "do not show"}
            self.assertEqual(section(rr.render(c, "mini.json", "0" * 64), "Notes"), "\n## Notes\n\nNone.\n")

    def test_x07_the_facts_a_caller_gets_wrong_are_on_the_page(self):
        text = rr.render(rr.load(CONTRACT), "x.json", "0" * 64)
        sec = section(text, "Notes")
        self.assertIsNotNone(sec, "no '## Notes' section")
        for needle in ("year 1970 to 9999", "-12:00 .. +14:00", "14 accept, 48 reject", "default of 1 MiB",
                       "15 operations at 16384"):
            self.assertIn(needle, sec, needle)

    def test_x08_no_notes_says_none(self):
        self.assertEqual(section(rr.render(mini(), "mini.json", "0" * 64), "Notes"), "\n## Notes\n\nNone.\n")

    def test_x09_the_sections_come_before_the_operations_in_this_order(self):
        text = rr.render(rr.load(CONTRACT), "x.json", "0" * 64)
        order = [text.find(f"\n## {t}\n") for t in ("Route rules", "Label grammars", "Notes", "Operations", "Errors")]
        self.assertNotIn(-1, order, f"a section is missing: {order}")
        self.assertEqual(order, sorted(order))


class Gap5Docstring(unittest.TestCase):
    """Quill's small thing: `render` prints to stdout; its docstring must say so."""

    def test_x10_the_render_line_of_the_docstring_says_stdout_and_does_not_claim_a_file_write(self):
        lines = rr.__doc__.splitlines()
        i = next(n for n, l in enumerate(lines) if l.strip().startswith("render CONTRACT"))
        block = " ".join(lines[i: i + 2])
        self.assertIn("stdout", block)
        self.assertNotIn("docs/generated/API-REFERENCE.md", block.split("stdout")[0])


class Guard(unittest.TestCase):
    """A GUARD cell: it passes before the change (so it is not RED) and must keep passing."""

    def test_g01_every_operation_with_a_request_body_shows_its_limit_in_its_own_section(self):
        c = rr.load(CONTRACT)
        text = rr.render(c, "x.json", "0" * 64)
        checked = 0
        for path, method, o in rr.operations(c):
            if not o.get("requestBody"):
                continue
            self.assertIn("x-max-body", o, f"{o['operationId']}: a request body with no declared limit")
            name = o["operationId"]
            start = text.index(f"### `{name}`")
            end = text.find("\n### ", start + 1)
            part = text[start: end if end >= 0 else len(text)]
            self.assertIn(f"Body (at most {o['x-max-body']} bytes)", part, name)
            checked += 1
        self.assertEqual(checked, 15, "the pinned r2.7 contract has 15 operations with a request body")


if __name__ == "__main__":
    unittest.main()
