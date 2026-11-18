#!/usr/bin/env python3
"""Strict, standard-library helpers for the local Cortex release builder."""
import argparse
import datetime
import errno
import hashlib
import io
import json
import os
from pathlib import Path
import re
import shutil
import signal
import stat
import subprocess
import sys
import tarfile
import tempfile
import tomllib

DIGEST = re.compile(r"sha256:[0-9a-f]{64}\Z", re.ASCII)
INITDB_NAME = re.compile(r"[A-Za-z0-9_-][A-Za-z0-9._-]{0,63}\Z", re.ASCII)
IMAGE = re.compile(r"[A-Za-z0-9._:/-]+@sha256:[0-9a-f]{64}\Z", re.ASCII)
BIN_MEMBERS = ("cortex-dump", "cortex-dump-status", "cortex-dump-alert", "cortex-restore", "restore-compare")
SYSTEMD_MEMBERS = ("cortex-dump.service", "cortex-dump.timer", "cortex-dump-status.service",
                   "cortex-dump-status.timer", "cortex-dump-alert.service")
# Validation policy only: the Containerfile consumes deploy/server-features.
REQUIRED_SERVER_FEATURES = frozenset(("mod-policy", "mod-records", "mod-search", "mod-handoffs", "mod-boot"))
BASELINE_PATH = "crates/cortex-db/migrations/platform/0001_platform.up.sql"
BASELINE_SHA256 = "d5c79f3b15643af101343eed72888a2e2ab490577fa57047037e0553c4ceb3f4"
BASELINE_DYNAMIC_LINES = [83, 91, 103, 104]
BASELINE_SQUAWK = [("require-lock-timeout", 2), ("require-statement-timeout", 2),
                   ("prefer-bigint-over-int", 7), ("prefer-bigint-over-int", 34)]


def die(message):
    raise ValueError(message)


def regular(path):
    path = Path(path)
    if path.is_symlink() or not path.is_file():
        die("release input is not a regular file: " + path.name)
    return path


def exact_baseline_exception(root, migration, entry):
    """Identify the sole candidate; this function does not authorize an exception."""
    root, migration = Path(root).absolute(), Path(migration).absolute()
    if (not isinstance(entry, dict) or entry.get("path") != BASELINE_PATH or
            entry.get("sha256") != BASELINE_SHA256 or
            entry.get("dynamic_sql_lines") != BASELINE_DYNAMIC_LINES or
            entry.get("squawk") != [{"rule": rule, "line": line} for rule, line in BASELINE_SQUAWK] or
            migration != root / BASELINE_PATH):
        return False
    try:
        path = root
        for part in Path(BASELINE_PATH).parts:
            path = path / part
            if path.is_symlink():
                return False
        raw = regular(migration).read_bytes()
        if hashlib.sha256(raw).hexdigest() != BASELINE_SHA256:
            return False
        text = raw.decode("utf-8")
        lines = [text.count("\n", 0, match.start()) + 1
                 for match in re.finditer(r"\bEXECUTE\b", text, re.IGNORECASE)]
        return lines == BASELINE_DYNAMIC_LINES
    except (OSError, UnicodeError, ValueError):
        return False


def exact_baseline_squawk(migration, findings):
    """Accept only the four reviewed findings for this migration."""
    if not isinstance(findings, list):
        return False
    observed = []
    for finding in findings:
        if (not isinstance(finding, dict) or not isinstance(finding.get("file"), str) or
                not isinstance(finding.get("rule_name"), str) or type(finding.get("line")) is not int or
                Path(finding["file"]).resolve() != Path(migration).resolve()):
            return False
        observed.append((finding["rule_name"], finding["line"]))
    return sorted(observed) == sorted(BASELINE_SQUAWK)


EXCEPTION_KINDS = ("execute_revoke", "privilege_literal_readback")
REVIEW_NAMES = ("REVIEW.md", "RULING.md")
ROW_FORMAT = re.compile(r"[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}\Z", re.ASCII)
ZERO_ROW = "00000000-0000-0000-0000-000000000000"
LEGACY_FIELDS = {"path", "sha256", "approved_by", "dynamic_sql_lines", "squawk"}
V2_FIELDS = {"path", "sha256", "approved_by", "matcher", "raw_execute_words", "dynamic_findings", "squawk"}
FINDING_FIELDS = {"line", "column", "statement_sha256", "kind", "reason"}
PIN_FIELDS = {"migration_path", "migration_sha256", "approved_by", "review_path", "review_sha256"}


def _position(text, offset):
    return text.count("\n", 0, offset) + 1, offset - (text.rfind("\n", 0, offset) + 1) + 1


def scan_execute_words(text):
    """Every raw EXECUTE word of the whole file as (line, column), with multiplicity (two tokens on one line are two)."""
    return [_position(text, match.start()) for match in re.finditer(r"\bEXECUTE\b", text, re.IGNORECASE)]


def _lex(text, name):
    """The one scanner of a migration (cox b36a989b: release.sh and the exception list use this and nothing else). Returns (the text with comments and data
    literals blanked, the offsets of the EXECUTE words that sit INSIDE a procedural body: a dollar-quoted body or a quoted DO or function body)."""
    s = text
    out = []
    dynamic = []
    i = 0
    while i < len(s):
        if s.startswith('--', i):
            end = s.find('\n', i + 2)
            i = len(s) if end < 0 else end
            out.append(' ')
        elif s.startswith('/*', i):
            i += 2
            depth = 1
            while i < len(s) and depth:
                if s.startswith('/*', i):
                    depth += 1
                    i += 2
                elif s.startswith('*/', i):
                    depth -= 1
                    i += 2
                else:
                    i += 1
            if depth:
                die('unterminated SQL comment: ' + name)
            out.append(' ')
        elif s[i] in "'\"":
            quote = s[i]
            # A quoted DO body or function body is executable SQL, not a data literal.
            prefix = ''.join(out).rsplit(';', 1)[-1]
            procedural = quote == "'" and (
                re.match(r'\s*DO\b', prefix, re.IGNORECASE)
                or re.search(r'\bAS\s*(?:E)?$', prefix, re.IGNORECASE)
            )
            start = i
            i += 1
            while i < len(s):
                if quote == "'" and s[i] == chr(92):
                    die('SQL backslash escape needs review: ' + name)
                if s[i] == quote:
                    i += 1
                    if i < len(s) and s[i] == quote:
                        i += 1
                    else:
                        break
                else:
                    i += 1
            else:
                die('unterminated SQL quote: ' + name)
            if procedural:
                dynamic.extend(start + match.start() for match in re.finditer(r'\bEXECUTE\b', s[start:i], re.IGNORECASE))
            out.append(' ' + s[start:i] + ' ' if procedural else ' ')
        elif s[i] == '$' and (m := re.match(r'\$[A-Za-z_][A-Za-z_0-9]*\$|\$\$', s[i:])):
            delimiter = m.group(0)
            end = s.find(delimiter, i + len(delimiter))
            if end < 0:
                die('unterminated SQL dollar quote: ' + name)
            # PL/pgSQL blocks can execute DDL, including SQL in EXECUTE strings.
            # Scan their raw text conservatively rather than treating it as harmless data.
            body = s[i + len(delimiter):end]
            dynamic.extend(i + len(delimiter) + match.start() for match in re.finditer(r'\bEXECUTE\b', body, re.IGNORECASE))
            out.append(' ' + body + ' ')
            i = end + len(delimiter)
        else:
            out.append(s[i])
            i += 1
    return ''.join(out), dynamic


def classify_finding(line_text, column):
    """The kind of one dynamic finding, computed from its own line (1-based character column): a quoted privilege label, or an EXECUTE format('REVOKE ALL ON ...) statement.
    Anything else has no approvable kind (None)."""
    start = column - 1
    before = line_text[start - 1] if start >= 1 else ""
    after = line_text[start + len("EXECUTE"):start + len("EXECUTE") + 1]
    if before == "'" and after == "'":
        return "privilege_literal_readback"
    if line_text.strip().startswith("EXECUTE format('REVOKE ALL ON "):
        return "execute_revoke"
    return None


def scan_dynamic_findings(text):
    """The EXECUTE words inside procedural bodies as (line, column), with multiplicity: exactly what the dynamic-SQL flag of a release is made of."""
    return [_position(text, offset) for offset in _lex(text, "<text>")[1]]


def scan_sql(text, name, allow_dynamic):
    """The whole migration scan of release.sh gate 4 (first releases are expand-only; computed SQL needs review unless the migration is an approved exception)."""
    if 'squawk-ignore' in text.lower():
        die('squawk-ignore is forbidden: ' + name)
    cleaned, dynamic = _lex(text, name)
    tokens = re.findall(r'[A-Za-z_][A-Za-z_0-9$]*|;', cleaned.upper())
    if not any(token != ';' for token in tokens):
        die('empty migration: ' + name)
    for statement in ' '.join(tokens).split(';'):
        words = statement.split()
        operation = None
        for n, word in enumerate(words):
            following = words[n + 1] if n + 1 < len(words) else ''
            if word == 'DROP' and following in ('COLUMN', 'TABLE'):
                operation = 'DROP ' + following
            elif word == 'DROP' and 'ALTER' in words[:n] and 'TABLE' in words[:n]:
                operation = 'DROP COLUMN'
            elif word == 'RENAME':
                operation = 'RENAME'
            elif word == 'SET' and words[n + 1:n + 3] == ['NOT', 'NULL']:
                operation = 'SET NOT NULL'
            elif word == 'TYPE' and 'ALTER' in words[:n] and 'TABLE' in words[:n]:
                operation = 'TYPE CHANGE'
            if operation:
                die('migration ' + name + ': ' + operation + ' requires contract; first release is expand-only')
    if dynamic and not allow_dynamic:
        die('dynamic SQL requires review: ' + name)


def _load_strict(path, what):
    raw = regular(path).read_bytes()
    if len(raw) > 65536:
        die(what + " is too large")

    def no_duplicates(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError(what + " has a duplicate JSON key")
            result[key] = value
        return result
    return raw, json.loads(raw, object_pairs_hook=no_duplicates)


def _is_row(value):
    return isinstance(value, str) and ROW_FORMAT.fullmatch(value) is not None and value != ZERO_ROW


def _check_entry(root, entry):
    """One entry of the list against the tree. Returns "baseline" (the frozen platform entry) or "clean" (a matcher-2 entry: no Squawk waiver)."""
    if not isinstance(entry, dict):
        die("entry is not an object")
    if "matcher" in entry:
        if type(entry["matcher"]) is not int or entry["matcher"] != 2:
            die("entry matcher version is unknown")
        if set(entry) != V2_FIELDS:
            die("entry field set is wrong (an unknown or missing field)")
        v2 = True
    else:
        if set(entry) != LEGACY_FIELDS:
            die("entry field set is wrong (an unknown or missing field)")
        v2 = False
    path = entry["path"]
    if (not isinstance(path, str) or not path or path.startswith("/") or "\\" in path or "\n" in path):
        die("entry path is not a relative repository path")
    parts = path.split("/")
    if any(part in ("", ".", "..") for part in parts):
        die("entry path has an empty, . or .. component")
    walk = root
    for part in parts:
        walk = walk / part
        if walk.is_symlink():
            die("entry path has a symlink component")
    if not _is_row(entry["approved_by"]):
        die("entry approved_by is not a decision row")
    raw = regular(root / path).read_bytes()
    if not isinstance(entry["sha256"], str) or hashlib.sha256(raw).hexdigest() != entry["sha256"]:
        die(f"entry {path}: file sha256 differs")
    if not v2:
        if not exact_baseline_exception(root, root / path, entry):
            die(f"entry {path}: the legacy entry does not match the frozen platform baseline")
        return "baseline"
    text = raw.decode("utf-8")
    words, findings = entry["raw_execute_words"], entry["dynamic_findings"]
    if (not isinstance(words, list) or any(not isinstance(w, dict) or set(w) != {"line", "column"} or type(w["line"]) is not int or type(w["column"]) is not int for w in words)):
        die("entry raw_execute_words has the wrong shape (field)")
    if not isinstance(findings, list):
        die("entry dynamic_findings is not a list")
    for finding in findings:
        if not isinstance(finding, dict) or set(finding) != FINDING_FIELDS:
            die("entry finding field set is wrong (an unknown or missing field)")
        if type(finding["line"]) is not int or type(finding["column"]) is not int or not isinstance(finding["statement_sha256"], str):
            die("entry finding has a wrong type (field)")
        if finding["kind"] not in EXCEPTION_KINDS:
            die("entry finding kind is not in the closed set")
        if not isinstance(finding["reason"], str) or not finding["reason"]:
            die("entry finding reason is empty (field)")
    if entry["squawk"] != []:
        die(f"entry {path}: a matcher-2 entry carries no Squawk waiver (squawk must be an empty list)")
    if words != [{"line": line, "column": column} for line, column in scan_execute_words(text)]:
        die(f"entry {path}: the raw EXECUTE words differ from the file")
    lines = text.split("\n")
    computed = [{"line": line, "column": column, "statement_sha256": hashlib.sha256(lines[line - 1].strip().encode("utf-8")).hexdigest(),
                 "kind": classify_finding(lines[line - 1], column)} for line, column in scan_dynamic_findings(text)]
    if [{k: f[k] for k in ("line", "column", "statement_sha256")} for f in findings] != [{k: c[k] for k in ("line", "column", "statement_sha256")} for c in computed]:
        die(f"entry {path}: the dynamic findings differ from the file")
    for finding, expected in zip(findings, computed):
        if finding["kind"] != expected["kind"]:
            die(f"entry {path}: the kind of the finding at line {finding['line']} column {finding['column']} differs from the computed kind {expected['kind']}")
    return "clean"


def _check_pin(root, pin, entries, seen):
    if not isinstance(pin, dict) or set(pin) != PIN_FIELDS:
        die("pin field set is wrong (an unknown or missing field)")
    path = pin["migration_path"]
    if not isinstance(path, str) or path not in entries:
        die("pin for an unknown path")
    if path in seen:
        die("duplicate pin for path " + path)
    seen.add(path)
    entry = entries[path]
    if pin["migration_sha256"] != entry["sha256"]:
        die(f"pin migration_sha256 differs from the entry {path}")
    if not _is_row(pin["approved_by"]):
        die("pin approved_by is not a decision row")
    if pin["approved_by"] != entry["approved_by"]:
        die("pin approved_by differs from the entry's")
    name = pin["review_path"]
    if (not isinstance(name, str) or not name or "\n" in name or "\r" in name or ".." in Path(name).parts or Path(name).name not in REVIEW_NAMES):
        die("review path or name is not allowed")
    review = Path(name)
    if not review.is_absolute():
        review = root / review
    digest = pin["review_sha256"]
    if not isinstance(digest, str) or re.fullmatch(r"[0-9a-f]{64}", digest) is None:
        die("review hash is malformed")
    try:
        actual = sha(review)
    except (ValueError, OSError):
        die("review file is missing")
    if actual != "sha256:" + digest:
        die("review file differs from its pin")


def verify_exception_list(root, config, record):
    """STRICT ALL-ENTRY validation (cox b36a989b): the list and its approval record, every entry against the tree. Returns {path: mode}; raises ValueError(cause)."""
    root, config, record = Path(root).absolute(), Path(config).absolute(), Path(record).absolute()
    if config != root / "scripts/release-migration-exceptions.json" or record != root / "scripts/release-migration-approval.json":
        die("config or record is not at its fixed path")
    config_bytes, data = _load_strict(config, "exception list")
    _, pins = _load_strict(record, "approval record")
    if not isinstance(data, dict) or data.get("schema") != 2 or type(data.get("schema")) is not int:
        die("list schema must be 2")
    if set(data) != {"schema", "exceptions"} or not isinstance(data["exceptions"], list):
        die("list field set is wrong (an unknown or missing field)")
    if not isinstance(pins, dict) or pins.get("schema") != 2 or type(pins.get("schema")) is not int:
        die("record schema must be 2")
    if set(pins) != {"schema", "config_sha256", "entries"} or not isinstance(pins["entries"], list):
        die("record field set is wrong (an unknown or missing field)")
    if pins["config_sha256"] != hashlib.sha256(config_bytes).hexdigest():
        die("record config hash differs from the list")
    if len(pins["entries"]) != len(data["exceptions"]):
        die("pin count differs from the entry count")
    modes, entries = {}, {}
    for entry in data["exceptions"]:
        mode = _check_entry(root, entry)
        if entry["path"] in entries:
            die("duplicate entry for path " + entry["path"])
        entries[entry["path"]] = entry
        modes[entry["path"]] = mode
    seen = set()
    for pin in pins["entries"]:
        _check_pin(root, pin, entries, seen)
    if seen != set(entries):
        die("a pin is missing for an entry")
    return modes


def verified_approval_record(root, config, record):
    """Check local integrity pins only; a separate review establishes authenticity."""
    try:
        verify_exception_list(root, config, record)
    except (OSError, UnicodeError, ValueError, TypeError, KeyError):
        return False
    return True


# The verifier (crates/cortex-upgrade/src/verify.rs, finn's R-4a: STAGED_FILE_MAX_BYTES) refuses a staged db-roles.sh or compose.yml LARGER than this;
# the builder refuses the same files at build time, so it never builds what the verifier would refuse on a user's machine. The two values MUST change together.
STAGED_FILE_MAX_BYTES = 1024 * 1024
# compose.yml has its OWN, tighter bound (hale SC-1, cox's ruling): the upgrade engine's compose reader (FILE_MAX_BYTES in crates/cortex-upgrade/src/compose.rs, used by
# engine.rs compare_db_service on the candidate's compose.yml) refuses a file LARGER than 256 KiB, so a bigger one would build and then fail on a user's machine.
# The two values MUST change together. The engine's other bounds (a db-block line of 1000 bytes, the block at 400 lines, a member at 256 KiB) are a stated limit.
COMPOSE_FILE_MAX_BYTES = 256 * 1024


def check_staged_sizes(args):
    bounds = (("db-roles.sh", STAGED_FILE_MAX_BYTES, "the verifier accepts (STAGED_FILE_MAX_BYTES in crates/cortex-upgrade/src/verify.rs)"),
              ("compose.yml", COMPOSE_FILE_MAX_BYTES, "the upgrade engine reads (FILE_MAX_BYTES in crates/cortex-upgrade/src/compose.rs)"))
    for name, cap, reason in bounds:
        size = regular(Path(args.layout) / name).stat().st_size
        if size > cap:
            die(f"{name} is {size} bytes, over the {cap} bytes {reason}: make the file smaller, then build again")


SPLICE = "each '# @include' marker line, newline included, is replaced by the exact bytes of its include; nothing is added"


# The anchor join (cox c7a09dae; contract G12-BINDING-RECORD-SCHEMA-nix-r4.md, accepted by vera): the approved public key file is read ONCE, validated, hashed and
# parsed from that one buffer; principal and key come from it and the fingerprint is recomputed with ssh-keygen. SSH_KEYGEN and KEYGEN_TIMEOUT are module
# constants (a test patches them; the builder has no seam for them).
SSH_KEYGEN = "/usr/bin/ssh-keygen"
KEYGEN_TIMEOUT = 10
TRUST_KEY_MAX_BYTES = 1024
ANCHOR_NAMES = ("CORTEX_ANCHOR_PRINCIPAL", "CORTEX_ANCHOR_KEY", "CORTEX_ANCHOR_FPR")
ANCHOR_PRINCIPAL = re.compile(r"[A-Za-z0-9._@][A-Za-z0-9._@-]{0,63}\Z", re.ASCII)
ANCHOR_KEY = re.compile(r"AAAAC3NzaC1lZDI1NTE5AAAAI[A-Za-z0-9+/]{43}\Z", re.ASCII)
KEYGEN_LINE = re.compile(r"([0-9]+) (SHA256:[A-Za-z0-9+/]{43}) (\S.*) \(([A-Za-z0-9-]+)\)\Z", re.ASCII)
ANCHOR_SPLICE = ("each '# @include' marker line, newline included, is replaced by the exact bytes of its include; then each of the three empty anchor "
                 "statements CORTEX_ANCHOR_PRINCIPAL='', CORTEX_ANCHOR_KEY='', CORTEX_ANCHOR_FPR='' is replaced by the same statement holding its "
                 "recorded value in single quotes; nothing else is added or changed")


def read_trust_key(path):
    """ONE bounded read of the approved key file: a regular file (never followed through a symlink), at most 1024 bytes, ASCII without NUL or CR, at most one
    trailing LF, exactly one line '<principal> ssh-ed25519 <key>'. Returns (the buffer, principal, key); the hash is taken from that same buffer."""
    try:
        descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK | os.O_CLOEXEC)
    except OSError as error:
        die("trust key file: the file is not a regular file (a symbolic link)" if error.errno == errno.ELOOP else "trust key file: the file cannot be read")
    try:
        if not stat.S_ISREG(os.fstat(descriptor).st_mode):
            die("trust key file: the file is not a regular file")
        buffer = b""
        while len(buffer) <= TRUST_KEY_MAX_BYTES:
            chunk = os.read(descriptor, TRUST_KEY_MAX_BYTES + 1 - len(buffer))
            if not chunk:
                break
            buffer += chunk
    except OSError:
        die("trust key file: the file cannot be read")
    finally:
        os.close(descriptor)
    if len(buffer) > TRUST_KEY_MAX_BYTES:
        die("trust key file: the file is larger than 1024 bytes")
    if b"\0" in buffer:
        die("trust key file: the file contains a NUL")
    if b"\r" in buffer:
        die("trust key file: the file contains a CR")
    if not buffer.isascii():
        die("trust key file: the file is not ASCII")
    text = buffer.decode("ascii")
    body = text[:-1] if text.endswith("\n") else text
    if body == "":
        die("trust key file: the file is empty")
    if "\n" in body:
        die("trust key file: the file has more than one line")
    if body.startswith("-----BEGIN"):
        die("trust key file: the file holds a private key header, not a public key line")
    parts = body.split(" ")
    if len(parts) != 3 or not all(parts):
        die("trust key file: the line must be '<principal> ssh-ed25519 <key>'")
    principal, kind, key = parts
    if kind != "ssh-ed25519":
        die("trust key file: the key type must be ssh-ed25519")
    if not ANCHOR_PRINCIPAL.fullmatch(principal):
        die("trust key file: the principal is not 1 to 64 characters of [A-Za-z0-9._@-] without a leading hyphen")
    if not ANCHOR_KEY.fullmatch(key):
        die("trust key file: the key is not 68 characters of [A-Za-z0-9+/] starting AAAAC3NzaC1lZDI1NTE5AAAAI")
    return buffer, principal, key


def fingerprint(key):
    """The fingerprint, recomputed by ssh-keygen -l on a 0600 file in a fresh 0700 directory (the modes are OUR rule, not the tool's). The scratch is removed on
    every exit, a trappable signal included; the tool is bounded by KEYGEN_TIMEOUT and ended when it overruns."""
    scratch = tempfile.mkdtemp(prefix="anchor-")
    handlers = {}

    def terminate(number, frame):
        raise SystemExit(128 + number)
    for number in (signal.SIGTERM, signal.SIGHUP, signal.SIGINT):
        handlers[number] = signal.signal(number, terminate)
    try:
        os.chmod(scratch, 0o700)
        path = os.path.join(scratch, "key.pub")
        descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
        try:
            os.fchmod(descriptor, 0o600)
            os.write(descriptor, ("ssh-ed25519 " + key + "\n").encode("ascii"))
        finally:
            os.close(descriptor)
        try:
            done = subprocess.run([SSH_KEYGEN, "-l", "-f", path], stdin=subprocess.DEVNULL, capture_output=True, timeout=KEYGEN_TIMEOUT,
                                  env={"PATH": "/usr/bin:/bin", "LC_ALL": "C"}, check=False)
        except subprocess.TimeoutExpired:
            die(f"trust key file: fingerprint tool timed out after {KEYGEN_TIMEOUT} seconds")
        except OSError:
            die("trust key file: fingerprint tool could not be started")
    finally:
        shutil.rmtree(scratch, ignore_errors=True)
        for number, handler in handlers.items():
            signal.signal(number, handler)
    if done.returncode != 0:
        die(f"trust key file: fingerprint tool failed (status {done.returncode})")
    try:
        text = done.stdout.decode("ascii")
    except UnicodeDecodeError:
        die("trust key file: fingerprint tool printed malformed output")
    line = text[:-1] if text.endswith("\n") else text
    match = KEYGEN_LINE.fullmatch(line) if line and "\n" not in line else None
    if not match:
        die("trust key file: fingerprint tool printed malformed output")
    bits, digest, _comment, kind = match.groups()
    if kind != "ED25519":
        die("trust key file: fingerprint tool reported a key that is not ED25519")
    if bits != "256":
        die("trust key file: fingerprint tool printed malformed output")
    return digest


def derive_anchor(path):
    buffer, principal, key = read_trust_key(path)
    return {"principal": principal, "key": key, "fpr": fingerprint(key), "key_input_sha256": hashlib.sha256(buffer).hexdigest()}


def apply_anchor(template, anchor):
    """T2: in the joined template each of the three empty statements, a whole line occurring exactly once, directly after the header (the leading blank or
    comment lines) and in this order, is replaced by the same statement holding its value in single quotes; every other byte stays."""
    lines = template.split(b"\n")
    for name in ANCHOR_NAMES:
        empty = name.encode("ascii") + b"=''"
        if any(line.startswith(name.encode("ascii") + b"=") and line != empty for line in lines):
            die("installer anchor placeholder already filled: " + name)
        count = lines.count(empty)
        if count > 1:
            die("installer anchor placeholder duplicated: " + name)
        if count == 0:
            die("installer anchor placeholder missing: " + name)
    top = 0
    while top < len(lines) and (lines[top] == b"" or lines[top].startswith(b"#")):
        top += 1
    if lines[top:top + 3] != [name.encode("ascii") + b"=''" for name in ANCHOR_NAMES]:
        die("installer anchor placeholders are out of order or not at the top")
    for offset, (name, value) in enumerate(zip(ANCHOR_NAMES, (anchor["principal"], anchor["key"], anchor["fpr"]))):
        lines[top + offset] = (name + "='" + value + "'").encode("ascii")
    return b"\n".join(lines)


def build_installer(root, anchor=None):
    """T1 (each marker line replaced by the raw bytes of its include), then, with an anchor, T2. Returns (the final bytes, the include records)."""
    root = Path(root)
    template = regular(root / "install.sh").read_bytes()
    original = template
    includes = []
    names = ("install-prereq.sh", "install-fetch.sh")
    function_pattern = rb'(?m)^([a-zA-Z_][a-zA-Z0-9_]*)[ \t]*\([ \t]*\)[ \t\n]*[\{(]'
    required = []
    for name in names:
        name_bytes = name.encode("ascii")
        plain = b'. "$here/' + name_bytes + b'"   # @include\n'
        guarded = b'[ ! -f "$here/' + name_bytes + b'" ] || ' + plain
        markers = [line for line in template.splitlines(keepends=True) if line in (plain, guarded)]
        if len(markers) != 1:
            die("installer include marker missing or duplicated: " + name)
        source = regular(root / name).read_bytes()
        if not source.endswith(b'\n') or b'\r' in source or source.startswith(b'#!') or b'# @include' in source:
            die("invalid installer include: " + name)
        functions = re.findall(function_pattern, source)
        if not functions:
            die("installer include has no functions: " + name)
        required.extend(functions)
        includes.append((original.index(markers[0]), name, hashlib.sha256(source).hexdigest()))
        template = template.replace(markers[0], source, 1)
    if b'# @include' in template:
        die("installer has leftover include marker")
    present = set(re.findall(function_pattern, template))
    if any(name not in present for name in required):
        die("installer is missing an included function")
    if anchor is not None:
        template = apply_anchor(template, anchor)
    return template, includes


def join_installer(args):
    # G12: with --record the join also writes a binding record from the SAME bytes it joined; the three ids come from the caller (release.sh holds the checked
    # commit); all four options or none, validated before anything is written. With --trust-key-file the join also anchors the installer (T2) and the record is
    # schema 2 (eight keys, anchor with four); without it the record is schema 1 (nix's r3) and the statements stay empty.
    given = [args.record, args.commit, args.tree, args.object_format]
    if any(given) and not all(given):
        die("a binding record needs --record, --commit, --tree and --object-format together")
    if args.record:
        width = {"sha1": 40, "sha256": 64}.get(args.object_format)
        if width is None:
            die("object format must be sha1 or sha256")
        for value in (args.commit, args.tree):
            if not re.fullmatch("[0-9a-f]{%d}" % width, value, re.ASCII):
                die("commit and tree must be lowercase hex of the object format's width")
    anchor = derive_anchor(args.trust_key_file) if args.trust_key_file else None
    template, includes = build_installer(args.deploy, anchor)
    Path(args.output).write_bytes(template)
    if args.record:
        record = {"schema": 2 if anchor else 1, "object_format": args.object_format, "commit": args.commit, "tree": args.tree,
                  "includes": [{"name": name, "sha256": digest} for _, name, digest in sorted(includes)],
                  "splice": ANCHOR_SPLICE if anchor else SPLICE, "joined_sha256": hashlib.sha256(template).hexdigest()}
        if anchor:
            record["anchor"] = {"principal": anchor["principal"], "key": anchor["key"], "fpr": anchor["fpr"], "key_input_sha256": anchor["key_input_sha256"]}
        Path(args.record).write_bytes((json.dumps(record, indent=2) + "\n").encode("ascii"))
    if anchor:
        print(anchor["key_input_sha256"])


def check_anchor(args):
    """The START check of a release: the key file, the fingerprint tool and the template are validated (nothing written) so a bad key or template fails
    before the first gate, not hours later at the join. Prints the hash of the one buffer read."""
    anchor = derive_anchor(args.trust_key_file)
    build_installer(args.deploy, anchor)
    print(anchor["key_input_sha256"])


def db_image(path):
    lines = regular(path).read_text(encoding="utf-8").splitlines()
    in_db = False
    found = []
    for line in lines:
        section = re.fullmatch(r"  ([A-Za-z0-9_-]+):\s*(?:#.*)?", line)
        if section:
            in_db = section.group(1) == "db"
        match = re.fullmatch(r"    image:\s*([^\s#]+)\s*(?:#.*)?", line)
        if in_db and match:
            found.append(match.group(1).strip("'\""))
    if len(found) != 1 or not IMAGE.fullmatch(found[0]):
        die("db image must be one digest-pinned literal")
    return found[0]


def server_features(path):
    """Read the image recipe's one feature list without interpreting shell text."""
    raw = regular(path).read_bytes()
    if len(raw) > 512 or not raw.endswith(b"\n") or raw.count(b"\n") != 1:
        die("server feature file must be one bounded line")
    line = raw[:-1].decode("ascii")
    features = line.split(",")
    if (len(set(features)) != len(features) or set(features) != REQUIRED_SERVER_FEATURES):
        die("server feature file does not match the required release feature set")
    return line


def image_digest(path):
    with tarfile.open(regular(path), "r:") as archive:
        matches = [m for m in archive.getmembers() if m.name == "index.json"]
        if len(matches) != 1 or not matches[0].isfile() or matches[0].size > 1024 * 1024:
            die("OCI archive must have one bounded index.json")
        index = json.load(archive.extractfile(matches[0]))
        manifests = index.get("manifests")
        if not isinstance(manifests, list) or len(manifests) != 1 or not isinstance(manifests[0], dict):
            die("OCI archive must have one manifest")
        digest = manifests[0].get("digest")
        if not isinstance(digest, str) or not DIGEST.fullmatch(digest):
            die("OCI manifest digest is invalid")
        return digest


def archive(args):
    layout = Path(args.layout)
    cli = regular(args.cli)
    compose = regular(layout / "compose.yml")
    roles = regular(layout / "db-roles.sh")
    initdb = layout / "initdb"
    if initdb.is_symlink() or not initdb.is_dir():
        die("initdb must be a real directory")
    entries = {"cortex": (cli, 0o755), "compose.yml": (compose, 0o644),
               "db-roles.sh": (roles, 0o755),
               "cortex.toml.example": (regular(layout / "cortex.toml.example"), 0o644)}
    systemd = layout / "systemd"
    if systemd.is_symlink() or not systemd.is_dir():
        die("systemd must be a real directory")
    for name in BIN_MEMBERS:
        entries["bin/" + name] = (regular(Path(args.scripts) / name), 0o755)
    for name in SYSTEMD_MEMBERS:
        entries["systemd/" + name] = (regular(systemd / name), 0o644)
    for path in initdb.iterdir():
        if not INITDB_NAME.fullmatch(path.name):
            die("invalid initdb filename")
        entries["initdb/" + path.name] = (regular(path), 0o644)
    if not IMAGE.fullmatch(args.server_image) or not IMAGE.fullmatch(args.db_image):
        die("image reference is not pinned")
    env = (f"CORTEX_VERSION={args.version}\nCORTEX_SERVER_IMAGE={args.server_image}\n"
           f"CORTEX_DB_IMAGE={args.db_image}\n").encode()
    with tarfile.open(args.output, "w", format=tarfile.USTAR_FORMAT) as out:
        for name in sorted([*entries, "initdb/", "bin/", "systemd/", "stack.env"]):
            info = tarfile.TarInfo(name)
            info.uid = info.gid = 0
            info.uname = info.gname = ""
            info.mtime = args.epoch
            if name in ("initdb/", "bin/", "systemd/"):
                info.type = tarfile.DIRTYPE
                info.mode = 0o755
                out.addfile(info)
            elif name == "stack.env":
                info.mode, info.size = 0o644, len(env)
                out.addfile(info, io.BytesIO(env))
            else:
                source, info.mode = entries[name]
                info.size = source.stat().st_size
                with source.open("rb") as f:
                    out.addfile(info, f)


def sha(path):
    h = hashlib.sha256()
    with regular(path).open("rb") as f:
        for chunk in iter(lambda: f.read(128 * 1024), b""):
            h.update(chunk)
    return "sha256:" + h.hexdigest()


def verify_dist(args):
    version = args.version
    match = re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?", version, re.ASCII)
    if (len(version.encode()) > 64 or version.endswith(".part") or not match or
            (match.group(4) and any(p.isdigit() and len(p) > 1 and p[0] == "0" for p in match.group(4).split(".")))):
        die("invalid distribution version")
    root = Path(args.output_dir)
    if root.is_symlink() or not root.is_dir():
        die("distribution must be a real directory")
    expected = {f"cortex-{version}-x86_64-linux.tar.gz", f"cortex-server-{version}-x86_64.oci.tar",
                "install.sh", "manifest.json", "install.meta"}
    if {p.name for p in root.iterdir()} != expected | {"SHA256SUMS"}:
        die("distribution inventory differs")
    with regular(root / "SHA256SUMS").open("rb") as stream:
        sums = stream.read(65537)
    if len(sums) > 65536 or not sums.endswith(b"\n"):
        die("invalid distribution checksums")
    listed = {}
    for line in sums.splitlines():
        match = re.fullmatch(rb"([0-9a-f]{64})  ([A-Za-z0-9._-]+)", line)
        if not match:
            die("invalid distribution checksum line")
        digest, name = match.group(1).decode(), match.group(2).decode()
        if name in listed:
            die("duplicate distribution checksum")
        listed[name] = digest
    if set(listed) != expected:
        die("distribution checksum inventory differs")
    for name in expected:
        if sha(root / name) != "sha256:" + listed[name]:
            die("distribution checksum mismatch: " + name)


def migration_paths(crates):
    root = Path(crates)
    if root.is_symlink() or not root.is_dir():
        die("migration root is not a real directory")
    platform = sorted((root / "cortex-db/migrations/platform").glob("*.up.sql"))
    modules = sorted(root.glob("mod-*/migrations/*.up.sql"))
    if not platform:
        die("platform migrations missing")
    paths = [*platform, *modules]
    for path in paths:
        regular(path)
        if "\n" in str(path) or "\r" in str(path):
            die("migration path contains a line break")
    return paths


def migrations(crates):
    result = []
    for path in migration_paths(crates):
        module = "platform" if path.parent.name == "platform" else path.parent.parent.name.removeprefix("mod-")
        result.append(module + "/" + path.name.removesuffix(".up.sql"))
    return result


def manifest(args):
    # RB-3: a release built with test seams says so. The list is empty (no line is written) for every real release.
    if args.seams and not re.fullmatch(r"CORTEX_RELEASE_[A-Z0-9_]+(?:,CORTEX_RELEASE_[A-Z0-9_]+)*", args.seams, re.ASCII):
        die("seam list is invalid")
    out = Path(args.output_dir)
    files = [f"cortex-{args.version}-x86_64-linux.tar.gz", f"cortex-server-{args.version}-x86_64.oci.tar"]
    assets = [{"name": name, "url": name, "digest": sha(out / name)} for name in files]
    modules = tomllib.loads(regular(args.toml).read_text(encoding="utf-8"))["modules"]["enabled"]
    if not isinstance(modules, list) or not modules or any(not isinstance(m, str) or not re.fullmatch(r"[a-z][a-z0-9_]{0,31}", m, re.ASCII) for m in modules):
        die("enabled modules are invalid")
    cortex = {"label": args.label, "members": 2, "min_upgrade_from": args.minimum or None,
              "server_image": {"ref": args.server_image, "asset": files[1]},
              "db_image": args.db_image, "roles_script_digest": sha(Path(args.layout) / "db-roles.sh"),
              "sidecar_images": [], "migrations": migrations(args.crates), "modules": modules,
              "migration_class": "expand"}
    entry = {"version": args.version, "date": datetime.datetime.fromtimestamp(args.epoch, datetime.timezone.utc).isoformat().replace("+00:00", "Z"),
             "notes_url": args.notes_url, "assets": assets, "cortex": cortex}
    (out / "manifest.json").write_text(json.dumps({"schema": 1, "releases": [entry]}, indent=2) + "\n", encoding="utf-8")
    metadata = ("META_SCHEMA=1\n"
                f"RELEASE_VERSION={entry['version']}\n"
                f"STACK_VERSION={entry['version']}\n"
                f"MEMBER_LIST={cortex['members']}\n"
                f"CORTEX_SERVER_IMAGE={cortex['server_image']['ref']}\n"
                f"CORTEX_DB_IMAGE={cortex['db_image']}\n"
                f"ROLES_SCRIPT_DIGEST={cortex['roles_script_digest'].removeprefix('sha256:')}\n"
                + (f"RELEASE_SEAMS={args.seams}\n" if args.seams else ""))
    if len(metadata.encode("ascii")) > 4096:
        die("install metadata exceeds 4 KiB")
    (out / "install.meta").write_text(metadata, encoding="ascii")


def main():
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)
    for name in ("db-image", "image-digest", "migration-files", "server-features"):
        p = sub.add_parser(name)
        p.add_argument("path")
    p = sub.add_parser("archive")
    for name in ("layout", "scripts", "cli", "output", "version", "server-image", "db-image"):
        p.add_argument("--" + name, required=True)
    p.add_argument("--epoch", type=int, required=True)
    p = sub.add_parser("manifest")
    for name in ("output-dir", "layout", "version", "server-image", "db-image", "label", "notes-url", "toml", "crates"):
        p.add_argument("--" + name, required=True)
    p.add_argument("--minimum", default="")
    p.add_argument("--seams", default="")
    p.add_argument("--epoch", type=int, required=True)
    p = sub.add_parser("verify-dist")
    p.add_argument("--output-dir", required=True)
    p.add_argument("--version", required=True)
    p = sub.add_parser("join-installer")
    p.add_argument("--deploy", required=True)
    p.add_argument("--output", required=True)
    for name in ("record", "commit", "tree", "object-format", "trust-key-file"):
        p.add_argument("--" + name, default=None)
    p = sub.add_parser("trust-key-hash")
    p.add_argument("--trust-key-file", required=True)
    p = sub.add_parser("check-anchor")
    p.add_argument("--deploy", required=True)
    p.add_argument("--trust-key-file", required=True)
    p = sub.add_parser("check-staged-sizes")
    p.add_argument("--layout", required=True)
    p = sub.add_parser("verify-approval")
    p.add_argument("--root", required=True)
    p = sub.add_parser("exception-candidate")
    p.add_argument("--root", required=True)
    p.add_argument("--migration", required=True)
    p = sub.add_parser("exception-squawk")
    p.add_argument("--migration", required=True)
    p.add_argument("--input", required=True)
    p = sub.add_parser("squawk-clean")
    p.add_argument("--input", required=True)
    p = sub.add_parser("scan-migration")
    p.add_argument("--migration", required=True)
    p.add_argument("--allow-dynamic", choices=("0", "1"), required=True)
    args = parser.parse_args()
    if args.command == "db-image":
        print(db_image(args.path))
    elif args.command == "server-features":
        print(server_features(args.path))
    elif args.command == "image-digest":
        print(image_digest(args.path))
    elif args.command == "migration-files":
        for path in migration_paths(args.path):
            print(path)
    elif args.command == "archive":
        archive(args)
    elif args.command == "verify-dist":
        verify_dist(args)
    elif args.command == "join-installer":
        join_installer(args)
    elif args.command == "trust-key-hash":
        # the end re-read of the release: the SAME checked, bounded, no-follow read as the start and the join (read_trust_key), nothing else reads the key path
        print(hashlib.sha256(read_trust_key(args.trust_key_file)[0]).hexdigest())
    elif args.command == "check-anchor":
        check_anchor(args)
    elif args.command == "check-staged-sizes":
        check_staged_sizes(args)
    elif args.command in ("verify-approval", "exception-candidate"):
        root = Path(args.root)
        config = root / "scripts/release-migration-exceptions.json"
        record = root / "scripts/release-migration-approval.json"
        try:
            modes = verify_exception_list(root, config, record)
        except (OSError, UnicodeError, ValueError, TypeError, KeyError) as error:
            print("release: exception check: " + str(error), file=sys.stderr)
            die("migration exception approval pins are invalid")
        if args.command == "exception-candidate":
            try:
                relative = Path(args.migration).absolute().relative_to(root.absolute()).as_posix()
            except ValueError:
                die("migration is outside the root")
            if relative not in modes:
                die("migration is not an approved exception candidate")
            print(modes[relative])
    elif args.command == "squawk-clean":
        raw = regular(args.input).read_bytes()
        if len(raw) > 1024 * 1024 or json.loads(raw) != []:
            die("the Squawk report is not an empty list")
    elif args.command == "scan-migration":
        scan_sql(Path(args.migration).read_text(encoding="utf-8"), Path(args.migration).name, args.allow_dynamic == "1")
    elif args.command == "exception-squawk":
        raw = regular(args.input).read_bytes()
        if len(raw) > 1024 * 1024 or not exact_baseline_squawk(args.migration, json.loads(raw)):
            die("baseline Squawk findings differ from the approved four")
    else:
        manifest(args)

if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, KeyError, TypeError, tarfile.TarError) as exc:
        print("release: " + str(exc), file=sys.stderr)
        sys.exit(1)
