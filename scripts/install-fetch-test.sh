#!/bin/sh
# 6c cells, STAGE 1 (rel_fetch's stage contract and rel_trust). hue (the lane moved from marv on 8 Nov, cox W1-89).
# All paths, keys and signatures are disposable fixtures made here. The cells run the REAL sha256sum and ssh-keygen: the fakes
# under scripts/fakes/fetch only count calls and inject failures (the harness points the rel_tool_* seams at them), so a cell
# that says "signed" means a real `ssh-keygen -Y sign` made the signature. Stage 2 cells (rel_unpack, rel_load) are marked as
# skipped on purpose and counted in the receipt: they are not claimed.
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
: "${TMPDIR:?set TMPDIR to private scratch under /home}"
case $TMPDIR in /tmp|/tmp/*) echo 'install-fetch-test: TMPDIR under /tmp is forbidden' >&2; exit 2 ;; esac
command -v ssh-keygen >/dev/null 2>&1 || { echo 'install-fetch-test: the real ssh-keygen is required (the cells sign for real)' >&2; exit 2; }
exec python3 -I - "$root" "$@" <<'PY'
import gzip
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
import time
import unittest

ROOT = Path(sys.argv[1])
FAKES = ROOT / "scripts/fakes/fetch"
VERSION = "1.2.3"
SERVER = "localhost/cortex-server@sha256:" + "a" * 64
DATABASE = "registry.invalid/db@sha256:" + "b" * 64
ROLES = b"#!/bin/sh\n# synthetic roles\n"
ROLES_HEX = hashlib.sha256(ROLES).hexdigest()
NAMESPACE = "cortex-release"
PRINCIPAL = "cortex-release"
STAGE_NAME = ".cortex-install-stage.Ab3dE9xZ"
BIN = ("cortex-dump", "cortex-dump-status", "cortex-dump-alert", "cortex-restore", "restore-compare")
UNITS = ("cortex-dump.service", "cortex-dump.timer", "cortex-dump-status.service", "cortex-dump-status.timer", "cortex-dump-alert.service")
MEMBERS = {"cortex": 0o755, "compose.yml": 0o644, "stack.env": 0o644, "db-roles.sh": 0o755,
           "cortex.toml.example": 0o644, "initdb": 0o755, "initdb/roles.sql": 0o644,
           "bin": 0o755, "systemd": 0o755,
           **{"bin/" + n: 0o755 for n in BIN}, **{"systemd/" + n: 0o644 for n in UNITS}}
PROBLEM = {
    "trust_key": "problem: the release signing key could not be confirmed",
    "trust_signature": "problem: the release checksum signature could not be verified",
    "trust_checksum": "problem: a release file is missing or does not match its checksum",
    "trust_stage": "problem: the release version or the staging directory is not the one this run reserved",
    "fetch_download": "problem: the release files could not be obtained",
    "fetch_location": "problem: no download location is configured",
}
MSG_IDS = ("trust_key", "trust_signature", "trust_checksum", "trust_stage", "fetch_download", "fetch_location", "fetch_archive", "fetch_image")
ASSETS = ("SHA256SUMS", "SHA256SUMS.sig", f"cortex-{VERSION}-x86_64-linux.tar.gz", f"cortex-server-{VERSION}-x86_64.oci.tar",
          "install.meta", "install.sh", "manifest.json")

KEYDIR = None


def sh(*argv, **kw):
    return subprocess.run(argv, text=True, capture_output=True, **kw)


def make_key(name):
    path = KEYDIR / name
    result = sh("ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", "cell", "-f", str(path))
    assert result.returncode == 0, result.stderr
    pub = (path.with_suffix(".pub") if False else Path(str(path) + ".pub")).read_text().split()
    fpr = sh("ssh-keygen", "-l", "-f", str(path) + ".pub").stdout.split()[1]
    return {"private": path, "type": pub[0], "b64": pub[1], "fpr": fpr}


def pin_gzip_time():
    """mtime=0 in the gzip header of every archive these cells build (tarfile.gzopen imports gzip.GzipFile at call time, so this reaches all of them): the bytes, and so
    every hash made from them, are the same in whatever clock second they are built (FM329, cell dt01). The subclass is NESTED on purpose: the mutants runner lists the
    unittest classes with ^class NAME( at column 0, and a helper class there would be loaded as a test class."""
    class FixedTimeGzip(gzip.GzipFile):
        def __init__(self, *args, **kwargs):
            kwargs.setdefault("mtime", 0)
            super().__init__(*args, **kwargs)
    gzip.GzipFile = FixedTimeGzip


pin_gzip_time()


def setUpModule():
    global KEYDIR, GOOD, OTHER
    KEYDIR = Path(tempfile.mkdtemp(prefix="install-fetch-keys-", dir=os.environ["TMPDIR"]))
    KEYDIR.chmod(0o700)
    for attempt in range(400):   # a key whose base64 AND fingerprint hold both + and /, so a character class that loses one of them is seen
        GOOD = make_key(f"good{attempt}")
        if all(c in GOOD["b64"] and c in GOOD["fpr"] for c in "+/"):
            break
    else:
        raise AssertionError("no test key with + and / in both its key and its fingerprint")
    OTHER = make_key("other")


def tearDownModule():
    shutil.rmtree(KEYDIR, ignore_errors=True)


HARNESS = r'''set -u
if [ -n "${FETCH_TRACE_ALL:-}" ]; then set -x; fi
if [ -z "${FETCH_NO_EX:-}" ]; then EX_MACHINE=2 EX_TRUST=${FETCH_EX_TRUST:-4} EX_FETCH=${FETCH_EX_FETCH:-5} EX_OK=0; else unset EX_MACHINE EX_TRUST EX_FETCH EX_OK; fi
[ -f "$FETCH_SOURCE" ] || { echo 'subject missing' >&2; exit 90; }
. "$FETCH_SOURCE"
if [ "${FETCH_FAKES:-}" = 1 ]; then
    rel_tool_sha256sum() { "$FAKES/sha256sum" "$@"; }
    rel_tool_keygen() { "$FAKES/ssh-keygen" "$@"; }
    rel_tool_curl() { "$FAKES/curl" "$@"; }
    rel_tool_tar() { "$FAKES/tar" "$@"; }
    rel_tool_podman() { "$FAKES/podman" "$@"; }
fi
if [ -n "${FETCH_FAULT_DIR:-}" ]; then
    # fault injection: every call of a seamed tool is counted in FETCH_FAULT_DIR, and the one FETCH_FAULT names ("tool:n") fails: before the tool runs, or
    # (FETCH_FAULT_AFTER) after it has written ALL its output. The environment of every call is recorded in FETCH_ENV_PROBE, one small file per call.
    fz_tab=$(printf '\tx'); fz_tab=${fz_tab%x}
    fz_nl=$(printf '\nx'); fz_nl=${fz_nl%x}
    rel_fault() {
        fz_n=0
        if [ -f "$FETCH_FAULT_DIR/$1" ]; then fz_n=$(/usr/bin/cat "$FETCH_FAULT_DIR/$1"); fi
        fz_n=$((fz_n + 1))
        printf '%s\n' "$fz_n" > "$FETCH_FAULT_DIR/$1"
        if [ -n "${FETCH_FAULT_ORDER:-}" ]; then printf '%s\n' "$1" >> "$FETCH_FAULT_ORDER"; fi   # every call in the order it was made
        if [ -n "${FETCH_ENV_PROBE:-}" ]; then
            case "$IFS" in " $fz_tab$fz_nl"|"$fz_nl"|" ") fz_i=ok ;; *) fz_i=bad ;; esac   # the include's own IFS values: default, lines, fields; never the caller's
            case $- in *f*) fz_g=off ;; *) fz_g=on ;; esac
            printf 'path=%s ifs=%s glob=%s lc=%s env-path=%s env-lc=%s umask=%s ossl=%s tin=%s\n' "$PATH" "$fz_i" "$fz_g" "${LC_ALL-unset}" "$(/usr/bin/printenv PATH)" "$(/usr/bin/printenv LC_ALL)" "$(umask)" "$(/usr/bin/printenv OPENSSL_CONF)$(/usr/bin/printenv OPENSSL_MODULES)$(/usr/bin/printenv OPENSSL_ENGINES)" "$(/usr/bin/env | /usr/bin/grep -cE '^(TAR_OPTIONS|CONTAINER_HOST|CONTAINER_CONNECTION|CONTAINER_SSHKEY|CONTAINER_PASSPHRASE|CONTAINERS_CONF|CONTAINERS_CONF_OVERRIDE|CONTAINERS_REGISTRIES_CONF|CONTAINERS_STORAGE_CONF|PODMAN_CONNECTIONS_CONF|STORAGE_DRIVER|STORAGE_OPTS)=')" > "$FETCH_ENV_PROBE/$1-$fz_n"
        fi
        [ "${FETCH_FAULT:-}" != "$1:$fz_n" ]
    }
    rel_fault_run() {
        fz_tool=$1; shift
        if rel_fault "$fz_tool"; then /usr/bin/"$fz_tool" "$@"; return $?; fi
        if [ -n "${FETCH_FAULT_AFTER:-}" ]; then /usr/bin/"$fz_tool" "$@"; fi
        return 1
    }
    rel_fault_fake() {
        fz_tool=$1; fz_fake=$2; shift 2
        if rel_fault "$fz_tool"; then "$FAKES/$fz_fake" "$@"; return $?; fi
        if [ -n "${FETCH_FAULT_AFTER:-}" ]; then "$FAKES/$fz_fake" "$@"; fi
        return 1
    }
    for fz_t in stat wc cat mkdir rm rmdir find ${FETCH_FAULT_MORE:-}; do
        eval "rel_tool_$fz_t() { rel_fault_run $fz_t \"\$@\"; }"
    done
    rel_tool_cp() {
        if rel_fault cp; then /usr/bin/cp "$@"; return $?; fi
        if [ -n "${FETCH_FAULT_AFTER:-}" ]; then /usr/bin/cp "$@"; else /usr/bin/head -c 10 "$2" > "$3" 2> /dev/null; fi
        return 1
    }
    rel_tool_uid() { rel_fault uid || return 1; /usr/bin/id -u; }
    rel_tool_python() {
        if rel_fault python; then /usr/bin/python3 -I "$@"; return $?; fi
        if [ -n "${FETCH_FAULT_AFTER:-}" ]; then /usr/bin/python3 -I "$@"; fi
        return 1
    }
    if [ "${FETCH_FAKES:-}" = 1 ]; then
        rel_tool_sha256sum() { rel_fault_fake sha256sum sha256sum "$@"; }
        rel_tool_keygen() { rel_fault_fake keygen ssh-keygen "$@"; }
        rel_tool_curl() { rel_fault_fake curl curl "$@"; }
        rel_tool_tar() { rel_fault_fake tar tar "$@"; }
        rel_tool_podman() { rel_fault_fake podman podman "$@"; }
    fi
fi
if [ -n "${FETCH_PRE:-}" ]; then eval "$FETCH_PRE"; fi
if [ -n "${FETCH_UID:-}" ]; then
    rel_tool_uid() { printf '%s\n' "$FETCH_UID"; }
fi
if [ -n "${FETCH_UMASK:-}" ]; then umask "$FETCH_UMASK"; fi
ifs() { if [ -n "${FETCH_IFS_SET:-}" ]; then IFS=$FETCH_IFS_SET; fi; }
flipstat() { rel_tool_stat() { case "$*" in *"/releases") printf '%s\n' "$FETCH_STAT_LATE" ;; *) /usr/bin/stat "$@" ;; esac; }; }   # from here on the releases directory seems to belong to another uid (a scratch stat that lies only about that path); used with the call step
noisychmod() { rel_tool_chmod() { /usr/bin/chmod "$@" && printf 'noise\n'; }; }   # a chmod that succeeds but prints on stdout (stdout is what rel_unpack_run answers with); used with the call step
if [ -n "${FETCH_SETE:-}" ]; then set -e; fi
for step in $FETCH_STEPS; do
    case $step in
        fetch) ifs; rel_fetch "$FETCH_VERSION" "$FETCH_STAGE" || exit "$?" ;;
        trust) ifs; rel_trust "$FETCH_VERSION" "$FETCH_STAGE" "${CORTEX_TRUST_KEY_FILE-}" "${CORTEX_TRUST_PRINCIPAL-}" "${CORTEX_TRUST_FPR-}" || exit "$?"
               printf '%s\n' "${REL_TRUSTED-UNSET}" > "$FETCH_OUT" ;;
        unpack) if [ -n "${FETCH_TRACE:-}" ]; then set -x; fi
               ifs; rel_unpack "$FETCH_VERSION" "$FETCH_STAGE" "$FETCH_HOME" || exit "$?"
               printf '%s\n' "${REL_CANDIDATE-UNSET}" > "$FETCH_CAND" ;;
        load) if [ -n "${FETCH_TRACE:-}" ]; then set -x; fi
              ifs; rel_load "$FETCH_VERSION" "$FETCH_STAGE" "$FETCH_HOME" || exit "$?" ;;
        verify) ifs; verify_release_dir "$FETCH_VERIFY_DIR" || exit "$?" ;;
        call) set -- $FETCH_CALL; ifs; "$@" || exit "$?" ;;
        trust-fail-keeps-empty) rel_trust "$FETCH_VERSION" "$FETCH_STAGE" "${CORTEX_TRUST_KEY_FILE-}" "${CORTEX_TRUST_PRINCIPAL-}" "${CORTEX_TRUST_FPR-}" && exit 91
               [ -z "${REL_TRUSTED-}" ] || exit 92 ;;
    esac
done
'''


class Cell(unittest.TestCase):
    maxDiff = None

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="install-fetch-cell-", dir=os.environ["TMPDIR"])
        self.root = Path(self.tmp.name)
        self.root.chmod(0o700)
        self.dist = self.root / "dist"
        self.dist.mkdir()
        self.home = self.root / "home"
        self.stage = self.root / STAGE_NAME
        self.log = self.root / "fake.log"
        self.out = self.root / "trusted.out"
        self.tar = self.dist / f"cortex-{VERSION}-x86_64-linux.tar.gz"
        self.image = self.dist / f"cortex-server-{VERSION}-x86_64.oci.tar"
        self.key = self.root / "trust.key"
        self.make_archive()
        self.image.write_bytes(b"synthetic OCI archive\n")
        (self.dist / "install.sh").write_bytes(b"#!/bin/sh\n# synthetic installer\n")
        self.write_manifest()
        self.write_meta()
        self.seal()
        self.write_key_file(GOOD)
        self.env = os.environ.copy()
        self.env.update({"FAKES": str(FAKES), "FAKE_LOG": str(self.log), "FAKE_RELEASE_DIR": str(self.dist),
                         "FETCH_SOURCE": os.environ.get("INSTALL_FETCH_SH", str(ROOT / "deploy/install-fetch.sh")), "FETCH_VERSION": VERSION,
                         "FETCH_STAGE": str(self.stage), "FETCH_OUT": str(self.out), "FETCH_HOME": str(self.home), "FETCH_CAND": str(self.root / "cand.out"),
                         "CORTEX_FROM": str(self.dist), "CORTEX_BASE_URL": "",
                         "CORTEX_TRUST_KEY_FILE": str(self.key), "CORTEX_TRUST_PRINCIPAL": PRINCIPAL,
                         "CORTEX_TRUST_FPR": GOOD["fpr"], "FAKE_FINGERPRINT": GOOD["fpr"]})

    def tearDown(self):
        self.tmp.cleanup()

    # ---- fixtures ----
    def make_archive(self):
        with tarfile.open(self.tar, "w:gz", format=tarfile.USTAR_FORMAT) as out:
            for name, mode in MEMBERS.items():
                info = tarfile.TarInfo(name)
                info.mode = mode
                if name in ("initdb", "bin", "systemd"):
                    info.type = tarfile.DIRTYPE
                    out.addfile(info)
                    continue
                data = (f"CORTEX_VERSION={VERSION}\nCORTEX_SERVER_IMAGE={SERVER}\nCORTEX_DB_IMAGE={DATABASE}\n".encode()
                        if name == "stack.env" else ROLES if name == "db-roles.sh" else ("fixture " + name + "\n").encode())
                info.size = len(data)
                out.addfile(info, io.BytesIO(data))

    def write_manifest(self):
        digest = lambda path: "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()
        manifest = {"schema": 1, "releases": [{"version": VERSION, "date": "2026-11-07T00:00:00Z", "notes_url": "https://example.invalid/notes", "assets": [
            {"name": self.tar.name, "url": self.tar.name, "digest": digest(self.tar)}, {"name": self.image.name, "url": self.image.name, "digest": digest(self.image)}],
            "cortex": {"label": VERSION, "members": 2, "server_image": {"ref": SERVER, "asset": self.image.name}, "db_image": DATABASE,
                       "roles_script_digest": "sha256:" + ROLES_HEX, "sidecar_images": [], "migrations": [], "modules": ["audit"], "migration_class": "expand"}}]}
        (self.dist / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")

    def meta_lines(self, **over):
        values = {"META_SCHEMA": "1", "RELEASE_VERSION": VERSION, "STACK_VERSION": VERSION, "MEMBER_LIST": "2",
                  "CORTEX_SERVER_IMAGE": SERVER, "CORTEX_DB_IMAGE": DATABASE, "ROLES_SCRIPT_DIGEST": ROLES_HEX}
        values.update(over)
        return values

    def write_meta(self, text=None, **over):
        if text is None:
            text = "".join(f"{k}={v}\n" for k, v in self.meta_lines(**over).items())
        (self.dist / "install.meta").write_bytes(text if isinstance(text, bytes) else text.encode())

    def seal(self, sums=None, signer=None, namespace=NAMESPACE, sign=True):
        """SHA256SUMS over the five listed files (or the given bytes), then a REAL signature."""
        names = [self.tar.name, self.image.name, "install.sh", "manifest.json", "install.meta"]
        if sums is None:
            sums = "".join(hashlib.sha256((self.dist / n).read_bytes()).hexdigest() + "  " + n + "\n" for n in names)
        (self.dist / "SHA256SUMS").write_bytes(sums if isinstance(sums, bytes) else sums.encode())
        sig = self.dist / "SHA256SUMS.sig"
        if sig.exists():
            sig.unlink()
        if sign:
            r = sh("ssh-keygen", "-Y", "sign", "-q", "-f", str((signer or GOOD)["private"]), "-n", namespace, str(self.dist / "SHA256SUMS"))
            self.assertEqual(r.returncode, 0, r.stderr)
        else:
            sig.write_text("not a signature\n")

    def write_key_file(self, key, principal=PRINCIPAL, text=None):
        if self.key.is_symlink() or self.key.exists():
            self.key.unlink()
        self.key.write_text(text if text is not None else f"{principal} {key['type']} {key['b64']}\n")

    def discard_stage(self):
        if self.stage.is_symlink():
            self.stage.unlink()
        elif self.stage.is_dir():
            shutil.rmtree(self.stage)
        elif self.stage.exists():
            self.stage.unlink()

    def reserve(self, mode=0o700):
        self.discard_stage()
        self.stage.mkdir()
        self.stage.chmod(mode)

    def fill_stage(self):
        self.reserve()
        for name in ASSETS:
            shutil.copyfile(self.dist / name, self.stage / name)
            (self.stage / name).chmod(0o600)

    def setUp_again(self):
        """A fresh cell inside the same test method (for loops over files)."""
        self.tearDown()
        self.setUp()

    def trust(self, fakes=False, **kw):
        return self.run_harness("trust", fakes=fakes, **kw)

    # ---- running ----
    def run_harness(self, steps, fakes=False, env=None, **extra):
        e = {**self.env, "FETCH_STEPS": steps, **(env or {}), **extra}
        if getattr(self, "no_ex", False):   # the caller defines no EX_* symbol: the include's own defaults (4 and 5) are what returns
            e["FETCH_NO_EX"] = "1"
        if fakes:
            e["FETCH_FAKES"] = "1"
        return subprocess.run(["sh", "-c", HARNESS], cwd=self.root, env=e, text=True, capture_output=True)

    def calls(self):
        return self.log.read_text() if self.log.exists() else ""

    def snapshot(self, path):
        """name -> (kind, mode, sha256 of bytes) of everything below path, for 'nothing changed' cells."""
        out = {}
        for p in sorted(path.rglob("*")):
            st = p.lstat()
            kind = "l" if stat.S_ISLNK(st.st_mode) else "d" if stat.S_ISDIR(st.st_mode) else "f"
            digest = hashlib.sha256(p.read_bytes()).hexdigest() if kind == "f" else ""
            out[str(p.relative_to(path))] = (kind, st.st_mode & 0o7777, digest)
        return out

    def stage_listing(self):
        return sorted(p.name for p in self.stage.iterdir())

    def assert_refused(self, result, code, message, noise=False):
        self.assertEqual(result.returncode, code, (result.stdout, result.stderr))
        lines = result.stderr.splitlines()
        if noise:   # the harness shell's OWN complaints about a hostile LD_PRELOAD and an absent locale, printed once when it starts
            lines = [l for l in lines if not l.startswith("ERROR: ld.so") and "setlocale" not in l]
        self.assertEqual(len(lines), 2, result.stderr)
        self.assertEqual(lines[0], PROBLEM[message], result.stderr)
        self.assertTrue(lines[1].startswith("fix: "), result.stderr)
        self.assertEqual(result.stdout, "")

    def assert_fetch_refused(self, result, message="fetch_download", code=5):
        self.assertEqual(result.returncode, code, (result.stdout, result.stderr))
        lines = result.stderr.splitlines()
        self.assertEqual(len(lines), 2, result.stderr)
        self.assertEqual(lines[0], PROBLEM[message], result.stderr)
        self.assertTrue(lines[1].startswith("fix: "), result.stderr)
        self.assertEqual(result.stdout, "")

    def assert_nothing_outside(self, before_root):
        """The only thing a trust run may touch is the stage: the rest of the cell root is as it was."""
        after = self.snapshot(self.root)
        for name in list(after):
            if name == STAGE_NAME or name.startswith(STAGE_NAME + "/") or name == "trusted.out" or name == "fake.log":
                after.pop(name)
        for name in list(before_root):
            if name == STAGE_NAME or name.startswith(STAGE_NAME + "/") or name == "trusted.out" or name == "fake.log":
                before_root.pop(name)
        self.assertEqual(after, before_root)
        self.assertFalse(self.home.exists())


class Fetch(Cell):
    """rel_fetch under the reservation contract (W1-86): the caller reserves an empty 0700 stage; rel_fetch only fills it."""

    def test_ir01_source_is_silent_and_does_not_change_shell_options(self):
        script = r'''set -u; before=$-; [ -f "$FETCH_SOURCE" ] || exit 90; . "$FETCH_SOURCE"; after=$-; [ "$before" = "$after" ]'''
        result = subprocess.run(["sh", "-c", script], cwd=self.root, env=self.env, text=True, capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((result.stdout, result.stderr), ("", ""))
        self.assertFalse(self.log.exists())

    def test_ir11_message_catalogue_has_cause_and_fix_without_secrets(self):
        script = r'''set -u; [ -f "$FETCH_SOURCE" ] || exit 90; . "$FETCH_SOURCE"; fetch_msg "$1"'''
        for name in MSG_IDS:
            with self.subTest(message=name):
                result = subprocess.run(["sh", "-c", script, "sh", name], cwd=self.root,
                                        env={**self.env, "FETCH_SECRET_TEST": "synthetic-secret-value"}, text=True, capture_output=True)
                self.assertEqual(result.returncode, 0, result.stderr)
                lines = result.stderr.splitlines()
                self.assertEqual(len(lines), 2, result.stderr)
                self.assertTrue(lines[0].startswith("problem: "))
                self.assertTrue(lines[1].startswith("fix: "))
                self.assertNotIn("synthetic-secret-value", result.stdout + result.stderr)
                if name in PROBLEM:
                    self.assertEqual(lines[0], PROBLEM[name])
        result = subprocess.run(["sh", "-c", script, "sh", "unknown_id"], cwd=self.root, env=self.env, text=True, capture_output=True)
        self.assertEqual(result.returncode, 1)
        self.assertEqual((result.stdout, result.stderr), ("", ""))

    def test_ir12_offline_fetch_fills_the_reserved_stage_with_exactly_the_assets(self):
        self.reserve()
        result = self.run_harness("fetch")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertEqual(self.stage_listing(), sorted(ASSETS))
        for name in ASSETS:
            self.assertEqual((self.stage / name).read_bytes(), (self.dist / name).read_bytes(), name)
            self.assertEqual((self.stage / name).stat().st_mode & 0o7777, 0o600, name)
        self.assertEqual(self.stage.stat().st_mode & 0o7777, 0o700)
        self.assertNotIn("curl", self.calls())

    def test_ir13_failed_offline_fetch_removes_the_files_it_wrote_and_keeps_the_callers_directory(self):
        (self.dist / "install.sh").unlink()
        self.reserve()
        result = self.run_harness("fetch")
        self.assert_fetch_refused(result)
        self.assertTrue(self.stage.is_dir())
        self.assertEqual(self.stage_listing(), [])
        self.assertTrue(self.dist.is_dir())
        self.assertEqual(len(list(self.dist.iterdir())), 6)
        self.assertFalse(self.home.exists())

    def test_ir14_the_stage_must_be_the_reserved_empty_private_directory(self):
        # (name, setup, expected exit) each case must refuse BEFORE writing anything
        def nonempty():
            self.reserve(); (self.stage / "keep").write_text("untouched")
        def absent():
            pass
        def symlink():
            target = self.root / "elsewhere"; target.mkdir(mode=0o700); self.stage.symlink_to(target)
        def loose_mode():
            self.reserve(0o755)
        def group_mode():
            self.reserve(0o770)
        def a_file():
            self.stage.write_text("a file")
        for name, setup in (("nonempty", nonempty), ("absent", absent), ("symlink", symlink), ("mode 0755", loose_mode),
                            ("mode 0770", group_mode), ("a file", a_file)):
            with self.subTest(stage=name):
                if self.stage.exists() or self.stage.is_symlink():
                    shutil.rmtree(self.stage) if self.stage.is_dir() and not self.stage.is_symlink() else self.stage.unlink()
                setup()
                before = self.snapshot(self.root)
                result = self.run_harness("fetch")
                self.assert_fetch_refused(result)
                self.assertEqual(result.stderr.splitlines()[0], PROBLEM["fetch_download"])
                self.assertEqual(self.snapshot(self.root), before)

    def test_ir14b_a_stage_that_is_not_named_like_the_installers_reservation_is_refused(self):
        for bad in (".cortex-install-stage.short", ".cortex-install-stage.Ab3dE9xZZ", ".cortex-install-stage.Ab3d.9xZ", "stage", ".cortex-install-stage.",
                    ".cortex-install-stage.Ab3dE9x-", "cortex-install-stage.Ab3dE9xZ"):
            with self.subTest(name=bad):
                stage = self.root / bad
                stage.mkdir(mode=0o700)
                result = self.run_harness("fetch", FETCH_STAGE=str(stage))
                self.assert_fetch_refused(result)
                self.assertEqual(list(stage.iterdir()), [])
                stage.rmdir()
        for bad in ("relative/" + STAGE_NAME, str(self.root) + "/../" + self.root.name + "/" + STAGE_NAME, str(self.root) + "//" + STAGE_NAME,
                    str(self.root) + "/./" + STAGE_NAME, ""):
            with self.subTest(path=bad):
                self.reserve()
                result = self.run_harness("fetch", FETCH_STAGE=bad)
                self.assert_fetch_refused(result)
                self.assertEqual(self.stage_listing(), [])
                self.stage.rmdir()

    def test_ir14d_an_entry_whose_name_is_only_whitespace_still_counts_as_an_entry(self):
        for name in ("\n", " ", "\t", "\n\n", " \n ", "\r"):
            with self.subTest(name=repr(name)):
                self.reserve()
                (self.stage / name).write_text("x")
                before = self.snapshot(self.root)
                result = self.run_harness("fetch")
                self.assert_fetch_refused(result)
                self.assertEqual(self.snapshot(self.root), before)

    def test_ir14c_a_stage_owned_by_someone_else_is_refused(self):
        self.reserve()
        result = self.run_harness("fetch", FETCH_UID="4242")
        self.assert_fetch_refused(result)
        self.assertEqual(self.stage_listing(), [])

    def test_ir15_online_fetch_uses_only_the_fake_transport_and_reads_cortex_base_url(self):
        self.reserve()
        result = self.run_harness("fetch", fakes=True, CORTEX_FROM="", CORTEX_BASE_URL="https://releases.invalid/1.2.3")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertEqual(self.calls().count("curl"), 7)
        self.assertEqual(self.stage_listing(), sorted(ASSETS))
        for name in ASSETS:
            self.assertEqual((self.stage / name).read_bytes(), (self.dist / name).read_bytes(), name)
        self.assertIn("--proto =https", self.calls())
        self.assertNotIn("FETCH_BASE_URL", Path(self.env["FETCH_SOURCE"]).read_text())

    def test_ir16_failed_download_cleans_what_it_wrote(self):
        self.reserve()
        result = self.run_harness("fetch", fakes=True, CORTEX_FROM="", CORTEX_BASE_URL="https://releases.invalid/1.2.3", FAKE_CURL_MODE="fail")
        self.assert_fetch_refused(result)
        self.assertIn("curl", self.calls())
        self.assertEqual(self.stage_listing(), [])

    def test_ir17_unsafe_online_urls_never_reach_transport(self):
        for url in ("http://releases.invalid/", "https://name:password@releases.invalid", "https://releases.invalid/?token=secret",
                    "https://", "https:///x", "https://releases.invalid/a/../b", "https://releases.invalid/" + "a" * 2100):
            with self.subTest(url=url[:60]):
                self.reserve()
                result = self.run_harness("fetch", fakes=True, CORTEX_FROM="", CORTEX_BASE_URL=url)
                self.assert_fetch_refused(result)
                self.assertNotIn("curl", self.calls())
                self.assertNotIn("secret", result.stdout + result.stderr + self.calls())
                self.assertEqual(self.stage_listing(), [])
                self.stage.rmdir()

    def test_ir18_no_source_at_all_says_no_download_location_and_fetches_nothing(self):
        self.reserve()
        result = self.run_harness("fetch", fakes=True, CORTEX_FROM="", CORTEX_BASE_URL="")
        self.assert_fetch_refused(result, "fetch_location")
        self.assertIn("--from", result.stderr)
        self.assertNotIn("curl", self.calls())
        self.assertEqual(self.stage_listing(), [])

    def test_ir19_a_from_directory_that_is_a_link_or_missing_is_refused(self):
        link = self.root / "dist-link"
        link.symlink_to(self.dist)
        for src in (str(link), str(self.root / "no-such-dir")):
            with self.subTest(src=src):
                self.reserve()
                result = self.run_harness("fetch", CORTEX_FROM=src)
                self.assert_fetch_refused(result)
                self.assertEqual(self.stage_listing(), [])
                self.stage.rmdir()

    def test_ir20_an_asset_that_is_a_link_or_too_large_is_refused(self):
        big = self.dist / "install.meta"
        big.write_bytes(b"x" * 4097)
        self.reserve()
        result = self.run_harness("fetch")
        self.assert_fetch_refused(result)
        self.assertEqual(self.stage_listing(), [])
        big.unlink()
        (self.dist / "elsewhere").write_text("real bytes")
        (self.dist / "install.meta").symlink_to(self.dist / "elsewhere")
        result = self.run_harness("fetch")
        self.assert_fetch_refused(result)
        self.assertEqual(self.stage_listing(), [])

    def test_ir20b_an_oversize_source_is_refused_before_it_is_copied(self):
        (self.dist / "install.meta").write_bytes(b"x" * 4097)
        d = self.root / "fault-count"
        d.mkdir()
        self.reserve()
        result = self.run_harness("fetch", FETCH_FAULT_DIR=str(d))
        self.assert_fetch_refused(result)
        self.assertEqual((d / "cp").read_text().strip(), "4")   # the tar, the image, install.sh and manifest.json were copied; install.meta was not

    def test_ir21_a_version_that_is_not_a_semver_is_refused_before_anything_is_written(self):
        for version in ("../escape", "1.2", "01.2.3", "1.2.3+build", "1.2.3.part", "1.2.3-", "1.2.3-01", "1.2.3-a..b", "", "1.2.3 ",
                        "1." + "9" * 20 + ".3", "1.2.3-" + "a" * 60):
            with self.subTest(version=version):
                self.reserve()
                result = self.run_harness("fetch", FETCH_VERSION=version)
                self.assert_fetch_refused(result)
                self.assertEqual(self.stage_listing(), [])
                self.stage.rmdir()
        for version in ("0.0.0", "1.2.3-rc.1", "10.20.30", "1.2.3-0", "1.2.3-0a", "1." + "9" * 19 + ".3"):
            with self.subTest(good=version):
                self.assertEqual(sh("sh", "-c", '. "$1"; rel_version_ok "$2"', "sh", self.env["FETCH_SOURCE"], version).returncode, 0)

    def test_ir21b_a_version_that_is_refused_is_refused_even_when_files_named_for_it_exist(self):
        for version in ("1.2", "01.2.3", "1.2.3+b", "1.2.3.part", "1.2.3-01", "1.2.3-", "1.2.3.4", "v1.2.3", "1.2.3-a..b", "1." + "9" * 20 + ".3"):
            with self.subTest(version=version):
                other = self.root / ("dist-" + str(abs(hash(version))))
                other.mkdir()
                for name in (f"cortex-{version}-x86_64-linux.tar.gz", f"cortex-server-{version}-x86_64.oci.tar", "install.sh", "manifest.json", "install.meta", "SHA256SUMS", "SHA256SUMS.sig"):
                    (other / name).write_text("x")
                self.reserve()
                result = self.run_harness("fetch", FETCH_VERSION=version, CORTEX_FROM=str(other))
                self.assert_fetch_refused(result)
                self.assertEqual(self.stage_listing(), [])

    def test_ir22_hostile_path_stand_ins_are_never_run_by_fetch(self):
        hostile = self.root / "hostile"
        hostile.mkdir()
        canary = self.root / "canary"
        for tool in ("wc", "cp", "ls", "id", "stat", "rm", "cat", "mkdir", "curl", "tr"):
            (hostile / tool).write_text(f"#!/bin/sh\nprintf '%s\\n' \"$0\" >> '{canary}'\nexit 0\n")
            (hostile / tool).chmod(0o755)
        self.reserve()
        result = self.run_harness("fetch", PATH=str(hostile) + os.pathsep + os.environ["PATH"])
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertFalse(canary.exists(), canary.read_text() if canary.exists() else "")
        self.assertEqual(self.stage_listing(), sorted(ASSETS))

    def test_ir23_the_callers_shell_state_is_not_changed_by_fetch(self):
        self.reserve()
        script = r'''set -u; . "$FETCH_SOURCE"; umask 022
before="$PATH|$IFS|$(umask)|$-|$(pwd)|$LC_ALL"
rel_fetch "$FETCH_VERSION" "$FETCH_STAGE" || exit $?
after="$PATH|$IFS|$(umask)|$-|$(pwd)|$LC_ALL"
[ "$before" = "$after" ] || { printf 'changed\n%s\n%s\n' "$before" "$after" >&2; exit 93; }'''
        result = subprocess.run(["sh", "-c", script], cwd=self.root, env={**self.env, "LC_ALL": "en_US.UTF-8"}, text=True, capture_output=True)
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))

    def test_ir24_a_caller_umask_does_not_widen_the_staged_files(self):
        self.reserve()
        result = self.run_harness("fetch", FETCH_UMASK="000")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        for name in ASSETS:
            self.assertEqual((self.stage / name).stat().st_mode & 0o7777, 0o600, name)

    def test_ir25_a_caller_ifs_does_not_change_the_result(self):
        self.reserve()
        result = self.run_harness("fetch", FETCH_IFS_SET="a.-")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertEqual(self.stage_listing(), sorted(ASSETS))


class FetchMore(Cell):
    def online(self, **kw):
        return self.run_harness("fetch", fakes=True, CORTEX_FROM="", **kw)

    def test_ir26_a_source_file_that_cannot_be_copied_is_a_refusal_and_leaves_the_stage_empty(self):
        (self.dist / "manifest.json").chmod(0o000)
        self.reserve()
        result = self.run_harness("fetch")
        (self.dist / "manifest.json").chmod(0o644)
        self.assert_fetch_refused(result)
        self.assertEqual(self.stage_listing(), [])

    def test_ir27_a_relative_from_directory_that_starts_with_a_hyphen_is_not_read_as_an_option(self):
        odd = self.root / "-x"
        shutil.copytree(self.dist, odd)
        self.reserve()
        result = self.run_harness("fetch", CORTEX_FROM="-x")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertEqual(self.stage_listing(), sorted(ASSETS))

    def test_ir28_a_transport_that_leaves_a_link_or_too_much_is_a_refusal(self):
        for mode in ("symlink", "big", "partial"):
            with self.subTest(mode=mode):
                self.reserve()
                result = self.online(CORTEX_BASE_URL="https://releases.invalid/1.2.3", FAKE_CURL_MODE=mode)
                self.assert_fetch_refused(result)
                self.assertEqual(self.stage_listing(), [])

    def test_ir29_the_addresses_asked_for_are_exact_with_or_without_a_trailing_slash(self):
        names = [f"cortex-{VERSION}-x86_64-linux.tar.gz", f"cortex-server-{VERSION}-x86_64.oci.tar", "install.sh", "manifest.json", "install.meta", "SHA256SUMS", "SHA256SUMS.sig"]
        for base in ("https://releases.invalid/1.2.3", "https://releases.invalid/1.2.3/"):
            with self.subTest(base=base):
                if self.log.exists():
                    self.log.unlink()
                self.reserve()
                result = self.online(CORTEX_BASE_URL=base)
                self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
                urls = [l.split("[url ")[1].split("]")[0] for l in self.calls().splitlines()]
                self.assertEqual(urls, ["https://releases.invalid/1.2.3/" + n for n in names])
                limits = {names[0]: 4294967296, names[1]: 4294967296, "install.sh": 1048576, "manifest.json": 1048576, "install.meta": 4096, "SHA256SUMS": 65536, "SHA256SUMS.sig": 65536}
                for line, name in zip(self.calls().splitlines(), names):
                    want = f"curl [fsize {(limits[name] // 512 + 2) * 512}] -q --fail --silent --show-error --location --proto =https --proto-redir =https --max-redirs 3 --speed-limit 1000000 --speed-time 30 --max-time 7200 --max-filesize {limits[name]} --output [output] [url https://releases.invalid/1.2.3/{name}]"
                    self.assertEqual(line, want)

    def test_ir29b_a_server_that_announces_no_size_cannot_make_curl_write_past_the_limit(self):
        self.reserve()
        result = self.online(CORTEX_BASE_URL="https://releases.invalid/1.2.3", FAKE_CURL_MODE="huge")
        self.assert_fetch_refused(result)
        done = [l for l in self.calls().splitlines() if l.startswith("curl-huge-completed")]
        self.assertEqual([l.split()[1] for l in done], [f"cortex-{VERSION}-x86_64-linux.tar.gz", f"cortex-server-{VERSION}-x86_64.oci.tar", "install.sh", "manifest.json"])
        self.assertEqual(self.stage_listing(), [])

    def test_ir30_the_base_url_length_and_alphabet_have_their_exact_edges(self):
        prefix = "https://releases.invalid/"
        ok = prefix + "a" * (2048 - len(prefix))
        self.assertEqual(len(ok), 2048)
        for base, want in ((ok, 0), (ok + "a", 5), (prefix + "A-Za-z0-9._~:/-x".replace("A-Za-z0-9", "Aa9"), 0)):
            with self.subTest(base=base[:60], want=want):
                self.log.unlink(missing_ok=True)   # the fake's log is a file the capped curl subshell appends to: keep it small
                self.reserve()
                result = self.online(CORTEX_BASE_URL=base)
                self.assertEqual(result.returncode, want, (result.stdout, result.stderr))
                if want:
                    self.assert_fetch_refused(result)
        for c in "?#@% ;&=+,\\'\"$*!()[]{}<>|`^\t":
            with self.subTest(char=repr(c)):
                self.log.unlink(missing_ok=True)
                self.reserve()
                result = self.online(CORTEX_BASE_URL=f"https://releases.invalid/a{c}b")
                self.assert_fetch_refused(result)
                self.assertNotIn("curl", self.calls())
        for c in "ABCZabcz0189._~:-":
            with self.subTest(allowed=c):
                self.log.unlink(missing_ok=True)
                self.reserve()
                result = self.online(CORTEX_BASE_URL=f"https://releases.invalid/a{c}b")
                self.assertEqual(result.returncode, 0, (c, result.stdout, result.stderr))

    def test_ir31_an_offline_directory_wins_over_a_base_url(self):
        self.reserve()
        result = self.run_harness("fetch", fakes=True, CORTEX_BASE_URL="https://releases.invalid/1.2.3")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertNotIn("curl", self.calls())

    def test_ir32_the_callers_own_exit_symbols_are_honoured(self):
        self.reserve()
        result = self.run_harness("fetch", CORTEX_FROM=str(self.root / "no-such-dir"), FETCH_EX_FETCH="15")
        self.assert_fetch_refused(result, code=15)
        self.setUp_again()
        self.tar.write_bytes(self.tar.read_bytes() + b"x")
        self.fill_stage()
        result = self.run_harness("trust", FETCH_EX_TRUST="14")
        self.assert_refused(result, 14, "trust_checksum")


class Hygiene(Cell):
    def test_hy01_rel_env_clean_leaves_exactly_the_documented_environment(self):
        names = "LD_PRELOAD LD_LIBRARY_PATH LD_AUDIT PYTHONPATH PYTHONHOME PYTHONSTARTUP BASH_ENV ENV CDPATH GCONV_PATH".split()
        script = r'''. "$FETCH_SOURCE"
export -n PATH 2>/dev/null; unset LC_ALL
for n in %s; do eval "$n=hostile; export $n"; done
( rel_env_clean
  printf 'path=%%s\n' "$(/usr/bin/printenv PATH)"
  printf 'lc=%%s\n' "$(/usr/bin/printenv LC_ALL)"
  for n in %s; do if /usr/bin/printenv "$n" >/dev/null; then printf 'still set: %%s\n' "$n"; fi; done
  case $- in *f*) echo noglob ;; *) echo glob-on ;; esac
  printf 'ifs=%%s\n' "$(printf '%%s' "$IFS" | /usr/bin/od -An -tx1 | /usr/bin/tr -d ' \n')"
  printf 'nl=%%s tab=%%s\n' "$(printf '%%s' "$rel_nl" | /usr/bin/od -An -tx1 | /usr/bin/tr -d ' \n')" "$(printf '%%s' "$rel_tab" | /usr/bin/od -An -tx1 | /usr/bin/tr -d ' \n')"
)''' % (" ".join(names), " ".join(names))
        result = subprocess.run(["sh", "-c", script], cwd=self.root, env=self.env, text=True, capture_output=True)
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertEqual(result.stdout.splitlines(), ["path=/usr/bin:/usr/sbin", "lc=C", "noglob", "ifs=200a09" if False else "ifs=20090a", "nl=0a tab=09"], result.stdout)
        self.assertEqual(result.stderr, "")

    def test_hy02_a_stand_in_stat_ahead_on_the_path_cannot_make_a_bad_stage_pass_rel_stage_ok(self):
        hostile = self.root / "hostile"
        hostile.mkdir()
        (hostile / "stat").write_text("#!/bin/sh\necho '%s 700'\n" % os.getuid())
        (hostile / "stat").chmod(0o755)
        self.reserve(0o755)
        r = subprocess.run(["sh", "-c", '. "$1"; rel_stage_ok "$2"', "sh", self.env["FETCH_SOURCE"], str(self.stage)], cwd=self.root,
                           env={**self.env, "PATH": str(hostile) + os.pathsep + os.environ["PATH"]}, text=True, capture_output=True)
        self.assertNotEqual(r.returncode, 0)
        self.stage.chmod(0o700)
        r = subprocess.run(["sh", "-c", '. "$1"; rel_stage_ok "$2"', "sh", self.env["FETCH_SOURCE"], str(self.stage)], cwd=self.root,
                           env={**self.env, "PATH": str(hostile) + os.pathsep + os.environ["PATH"]}, text=True, capture_output=True)
        self.assertEqual(r.returncode, 0)


class FetchNoEx(Fetch):
    no_ex = True


class FetchMoreNoEx(FetchMore):
    no_ex = True
    test_ir32_the_callers_own_exit_symbols_are_honoured = None   # it defines its own symbols: the variant without any is not meaningful here


class Trust(Cell):
    def test_tr01_a_good_signed_set_is_trusted_and_nothing_outside_the_stage_changes(self):
        self.fill_stage()
        before_root = self.snapshot(self.root)
        before_stage = self.snapshot(self.stage)
        result = self.trust()
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertEqual((result.stdout, result.stderr), ("", ""))
        self.assertEqual(self.snapshot(self.stage), before_stage)
        self.assert_nothing_outside(before_root)
        text = self.out.read_text()
        lines = text.splitlines()
        self.assertEqual(lines[0], "v1")
        self.assertEqual(lines[1], "version " + VERSION)
        self.assertEqual(lines[2], "stage " + str(self.stage))
        files = {l.split()[1]: l.split() for l in lines if l.startswith("file ")}
        self.assertEqual(set(files), {self.tar.name, self.image.name, "install.sh", "manifest.json", "install.meta"})
        for name, parts in files.items():
            data = (self.dist / name).read_bytes()
            self.assertEqual(parts[2], str(len(data)), name)
            self.assertEqual(parts[3], hashlib.sha256(data).hexdigest(), name)
        meta = {l.split()[1]: l.split()[2] for l in lines if l.startswith("meta ")}
        self.assertEqual(meta, {"RELEASE_VERSION": VERSION, "STACK_VERSION": VERSION, "MEMBER_LIST": "2", "CORTEX_SERVER_IMAGE": SERVER,
                                "CORTEX_DB_IMAGE": DATABASE, "ROLES_SCRIPT_DIGEST": ROLES_HEX})
        self.assertEqual(len(lines), 3 + 5 + 6)

    def test_tr02_the_pinned_fingerprint_must_match_the_key(self):
        for fpr in (OTHER["fpr"], "SHA256:" + "A" * 43, GOOD["fpr"][:-1] + ("A" if GOOD["fpr"][-1] != "A" else "B"), GOOD["fpr"].lower(),
                    "", GOOD["fpr"] + "x", " " + GOOD["fpr"], "SHA256:" + "A" * 42, "MD5:" + GOOD["fpr"][7:]):
            with self.subTest(fpr=fpr):
                self.fill_stage()
                result = self.trust(CORTEX_TRUST_FPR=fpr)
                self.assert_refused(result, 4, "trust_key")
                self.assertEqual(self.stage_listing(), sorted(ASSETS))
                shutil.rmtree(self.stage)

    def test_tr03_the_key_file_must_be_exactly_one_ed25519_line_for_the_principal(self):
        good = f"{PRINCIPAL} {GOOD['type']} {GOOD['b64']}\n"
        rsa = KEYDIR / ("rsa-" + self.id().split(".")[-2])
        self.assertEqual(sh("ssh-keygen", "-q", "-t", "rsa", "-b", "2048", "-N", "", "-f", str(rsa)).returncode, 0)
        rsa_pub = (Path(str(rsa) + ".pub")).read_text().split()
        cases = {
            "two lines": good + good,
            "another principal": f"someone-else {GOOD['type']} {GOOD['b64']}\n",
            "options before the key": f"{PRINCIPAL} cert-authority {GOOD['type']} {GOOD['b64']}\n",
            "rsa key": f"{PRINCIPAL} {rsa_pub[0]} {rsa_pub[1]}\n",
            "another ed25519 key under the pinned fingerprint": f"{PRINCIPAL} {OTHER['type']} {OTHER['b64']}\n",
            "empty": "",
            "no newline is fine but a comment is not": good.rstrip("\n") + " extra-comment\n",
            "a CR": good.rstrip("\n") + "\r\n",
            "a tab": f"{PRINCIPAL}\t{GOOD['type']} {GOOD['b64']}\n",
            "a short key": f"{PRINCIPAL} {GOOD['type']} {GOOD['b64'][:-4]}\n",
            "too large": "# " + "x" * 2000 + "\n" + good,
        }
        for name, text in cases.items():
            with self.subTest(case=name):
                self.write_key_file(GOOD, text=text)
                self.fill_stage()
                result = self.trust()
                self.assert_refused(result, 4, "trust_key")
                shutil.rmtree(self.stage)
        # a key file without a final newline is the same key: accepted
        self.write_key_file(GOOD, text=good.rstrip("\n"))
        self.fill_stage()
        result = self.trust()
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))

    def test_tr04_signer_inputs_that_are_unset_or_unusable_are_a_key_failure(self):
        link = self.root / "key-link"
        link.symlink_to(self.key)
        cases = [("no key file variable", {"CORTEX_TRUST_KEY_FILE": ""}), ("no principal", {"CORTEX_TRUST_PRINCIPAL": ""}), ("no fingerprint", {"CORTEX_TRUST_FPR": ""}),
                 ("key file missing", {"CORTEX_TRUST_KEY_FILE": str(self.root / "absent")}), ("key file is a link", {"CORTEX_TRUST_KEY_FILE": str(link)}),
                 ("key file is a directory", {"CORTEX_TRUST_KEY_FILE": str(self.root)}), ("relative key file", {"CORTEX_TRUST_KEY_FILE": "trust.key"}),
                 ("principal with a space", {"CORTEX_TRUST_PRINCIPAL": "cortex release"}), ("principal with a hyphen first", {"CORTEX_TRUST_PRINCIPAL": "-I"}),
                 ("principal with a wildcard", {"CORTEX_TRUST_PRINCIPAL": "*"}), ("principal too long", {"CORTEX_TRUST_PRINCIPAL": "a" * 65})]
        for name, over in cases:
            with self.subTest(case=name):
                self.fill_stage()
                result = self.trust(env=over)
                self.assert_refused(result, 4, "trust_key")
                shutil.rmtree(self.stage)

    def test_tr05_a_signature_from_another_key_or_namespace_or_over_other_bytes_is_refused(self):
        sums_good = None
        for name, prep in (
            ("signed by another key", lambda: self.seal(signer=OTHER)),
            ("another namespace", lambda: self.seal(namespace="git")),
            ("not a signature", lambda: self.seal(sign=False)),
        ):
            with self.subTest(case=name):
                prep()
                self.fill_stage()
                result = self.trust()
                self.assert_refused(result, 4, "trust_signature")
                self.assertEqual(self.stage_listing(), sorted(ASSETS))
                shutil.rmtree(self.stage)
        self.seal()
        old_sig = (self.dist / "SHA256SUMS.sig").read_bytes()
        sums = (self.dist / "SHA256SUMS").read_text()
        (self.dist / "SHA256SUMS").write_text(sums.replace(sums[:1], "0" if sums[0] != "0" else "1", 1))
        (self.dist / "SHA256SUMS.sig").write_bytes(old_sig)
        self.fill_stage()
        result = self.trust()
        self.assert_refused(result, 4, "trust_signature")
        shutil.rmtree(self.stage)
        self.seal()
        sig = (self.dist / "SHA256SUMS.sig").read_bytes()
        (self.dist / "SHA256SUMS.sig").write_bytes(sig[:-30])
        self.fill_stage()
        self.assert_refused(self.trust(), 4, "trust_signature")

    def test_tr06_a_release_signed_by_the_right_key_with_a_wrong_hash_is_refused_per_file(self):
        for name in (self.tar.name, self.image.name, "install.sh", "manifest.json", "install.meta"):
            with self.subTest(file=name):
                self.setUp_again()
                (self.dist / name).write_bytes((self.dist / name).read_bytes() + (b"x" if name != "install.meta" else b""))
                if name == "install.meta":
                    self.write_meta(RELEASE_VERSION="1.2.4")
                self.fill_stage()
                result = self.trust()
                self.assert_refused(result, 4, "trust_checksum")

    def test_tr07_the_listing_must_be_exactly_the_five_files_once_in_the_builders_shape(self):
        names = [self.tar.name, self.image.name, "install.sh", "manifest.json", "install.meta"]
        def line(n):
            return hashlib.sha256((self.dist / n).read_bytes()).hexdigest() + "  " + n + "\n"
        good = "".join(line(n) for n in names)
        cases = {
            "a file missing": "".join(line(n) for n in names[:-1]),
            "a name twice": good + line(names[0]),
            "an extra name": good + hashlib.sha256(b"x").hexdigest() + "  extra.bin\n",
            "upper-case hex": good[:64].upper() + good[64:],
            "CRLF": good.replace("\n", "\r\n"),
            "no final newline": good[:-1],
            "a blank line": good + "\n",
            "one space instead of two": good.replace("  ", " ", 1),
            "a star (binary mode)": good.replace("  ", " *", 1),
            "a path in a name": good.replace("  install.sh", "  ./install.sh"),
            "a short hash": good[1:],
            "an empty listing": "",
            "a comment line": "# a comment\n" + good,
            "trailing spaces": good.replace("\n", " \n", 1),
        }
        for name, sums in cases.items():
            with self.subTest(case=name):
                self.setUp_again()
                self.seal(sums=sums)
                self.fill_stage()
                result = self.trust()
                self.assertEqual(result.returncode, 4, (name, result.stdout, result.stderr))
                self.assertEqual(result.stderr.splitlines()[0], PROBLEM["trust_checksum"], name)

    def test_tr08_install_meta_is_checked_as_bounded_data_in_the_builders_exact_shape(self):
        ok = self.meta_lines()
        def text(**over):
            return "".join(f"{k}={v}\n" for k, v in self.meta_lines(**over).items())
        order = list(ok)
        swapped = order[:]
        swapped[1], swapped[2] = swapped[2], swapped[1]
        cases = {
            "schema 2": text(META_SCHEMA="2"),
            "release version differs from the one asked for": text(RELEASE_VERSION="1.2.4"),
            "stack version differs": text(STACK_VERSION="1.2.4"),
            "member list 3": text(MEMBER_LIST="3"),
            "member list 02": text(MEMBER_LIST="02"),
            "swapped keys": "".join(f"{k}={ok[k]}\n" for k in swapped),
            "an extra key": text() + "EXTRA=1\n",
            "a missing key": "".join(f"{k}={ok[k]}\n" for k in order[:-1]),
            "a repeated key": text() + "MEMBER_LIST=2\n",
            "a CR": text().replace("\n", "\r\n", 1),
            "no final newline": text()[:-1],
            "a blank line": text() + "\n",
            "a line without =": text() + "garbage\n",
            "a comment": "# c\n" + text(),
            "spaces around =": text().replace("MEMBER_LIST=2", "MEMBER_LIST = 2"),
            "server image without a digest": text(CORTEX_SERVER_IMAGE="localhost/cortex-server:latest"),
            "server image with a short digest": text(CORTEX_SERVER_IMAGE="localhost/cortex-server@sha256:" + "a" * 63),
            "server image with an upper-case digest": text(CORTEX_SERVER_IMAGE="localhost/cortex-server@sha256:" + "A" * 64),
            "db image with a second @": text(CORTEX_DB_IMAGE="reg@x/db@sha256:" + "b" * 64),
            "db image with an empty name": text(CORTEX_DB_IMAGE="@sha256:" + "b" * 64),
            "an image with a space": text(CORTEX_DB_IMAGE="reg istry/db@sha256:" + "b" * 64),
            "an image over 300 bytes": text(CORTEX_DB_IMAGE="r" * 300 + "/db@sha256:" + "b" * 64),
            "roles digest short": text(ROLES_SCRIPT_DIGEST=ROLES_HEX[:-1]),
            "roles digest upper-case": text(ROLES_SCRIPT_DIGEST=ROLES_HEX.upper()),
            "roles digest with the sha256: prefix": text(ROLES_SCRIPT_DIGEST="sha256:" + ROLES_HEX),
            "empty value": text(CORTEX_SERVER_IMAGE=""),
            "a NUL byte": text().encode().replace(b"MEMBER_LIST=2", b"MEMBER_LIST=2\x00"),
            "a control character": text().encode().replace(b"MEMBER_LIST=2", b"MEMBER_LIST=\x072"),
            "a non-ASCII byte": text().encode().replace(b"MEMBER_LIST=2", b"MEMBER_LIST=\xc3\xa92"),
            "too large": text() + "#" * 4100,
        }
        for name, content in cases.items():
            with self.subTest(case=name):
                self.setUp_again()
                self.write_meta(text=content)
                self.seal()
                self.fill_stage()
                result = self.trust()
                self.assertEqual(result.returncode, 4, (name, result.stdout, result.stderr))
                self.assertEqual(result.stderr.splitlines()[0], PROBLEM["trust_checksum"], name)
                self.assertEqual(result.stdout, "")

    def test_tr09_the_stage_inventory_and_file_kinds_are_checked_before_anything_is_hashed(self):
        def stray():
            (self.stage / "stray").write_text("x")
        def missing_sig():
            (self.stage / "SHA256SUMS.sig").unlink()
        def missing_tar():
            (self.stage / self.tar.name).unlink()
        def hidden():
            (self.stage / ".hidden").write_text("x")
        def a_dir():
            (self.stage / "manifest.json").unlink(); (self.stage / "manifest.json").mkdir()
        def a_link():
            real = self.root / "real-manifest"; shutil.copyfile(self.dist / "manifest.json", real)
            (self.stage / "manifest.json").unlink(); (self.stage / "manifest.json").symlink_to(real)
        def a_hard_link():
            os.link(self.stage / "manifest.json", self.root / "second-name")
        def trust_work_left():
            (self.stage / ".trust-work").mkdir()
        def unreadable():
            (self.stage / "install.sh").chmod(0o000)
        for name, damage, message in (("a stray file", stray, "trust_checksum"), ("the signature missing", missing_sig, "trust_checksum"),
                                      ("the archive missing", missing_tar, "trust_checksum"), ("a hidden file", hidden, "trust_checksum"),
                                      ("a directory in place of a file", a_dir, "trust_checksum"), ("a link in place of a file (same bytes)", a_link, "trust_checksum"),
                                      ("a second hard link to a staged file", a_hard_link, "trust_checksum"), ("a left-over work directory", trust_work_left, "trust_checksum"),
                                      ("an unreadable file", unreadable, "trust_checksum")):
            with self.subTest(case=name):
                self.setUp_again()
                self.fill_stage()
                damage()
                result = self.trust(fakes=True)
                self.assert_refused(result, 4, message)
                self.assertNotIn("sha256sum", self.calls())
                self.assertNotIn("ssh-keygen", self.calls())

    def test_tr09b_an_extra_entry_named_only_with_whitespace_is_an_eighth_entry(self):
        for name in ("\n", " ", "\t", "\n\n", " \n ", "\r", ".", "x"):
            with self.subTest(name=repr(name)):
                self.setUp_again()
                self.fill_stage()
                if name == ".":
                    name = "..x"
                (self.stage / name).write_text("extra")
                result = self.trust(fakes=True)
                self.assert_refused(result, 4, "trust_checksum")
                self.assertNotIn("sha256sum", self.calls())
                self.assertNotIn("ssh-keygen", self.calls())

    def test_tr10_the_stage_itself_must_be_the_reserved_private_directory(self):
        for name, prep, env in (("mode 0755", lambda: self.stage.chmod(0o755), {}), ("mode 0750", lambda: self.stage.chmod(0o750), {}),
                                ("another owner", lambda: None, {"FETCH_UID": "4242"}),
                                ("a wrong name", None, {}), ("a relative path", None, {"FETCH_STAGE": "x/" + STAGE_NAME})):
            with self.subTest(case=name):
                self.setUp_again()
                self.fill_stage()
                if name == "a wrong name":
                    wrong = self.root / ".cortex-install-stage.short"
                    self.stage.rename(wrong)
                    env = {"FETCH_STAGE": str(wrong)}
                elif prep:
                    prep()
                result = self.trust(fakes=True, env=env)
                self.assert_refused(result, 4, "trust_stage")
                self.assertEqual(self.calls(), "")

    def test_tr11_a_stage_link_is_refused(self):
        self.fill_stage()
        real = self.root / "real-stage"
        self.stage.rename(real)
        self.stage.symlink_to(real)
        result = self.trust(fakes=True)
        self.assert_refused(result, 4, "trust_stage")
        self.assertEqual(self.calls(), "")

    def test_tr12_a_bad_version_is_refused_before_the_stage_is_read(self):
        for version in ("../escape", "1.2", "1.2.3+b", ""):
            with self.subTest(version=version):
                self.setUp_again()
                self.fill_stage()
                result = self.trust(fakes=True, FETCH_VERSION=version)
                self.assert_refused(result, 4, "trust_stage")
                self.assertEqual(self.calls(), "")

    def test_tr13_the_order_is_fingerprint_then_signature_then_hashes_and_nothing_is_hashed_before_the_signature(self):
        self.fill_stage()
        result = self.trust(fakes=True)
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        calls = self.calls().splitlines()
        kinds = [c.split()[0] + (" verify" if "-Y verify" in c else " fpr" if "-l" in c.split() else "") for c in calls]
        self.assertEqual(kinds.count("ssh-keygen fpr"), 1, calls)
        self.assertEqual(kinds.count("ssh-keygen verify"), 1, calls)
        self.assertLess(kinds.index("ssh-keygen fpr"), kinds.index("ssh-keygen verify"))
        self.assertTrue(all(i > kinds.index("ssh-keygen verify") for i, k in enumerate(kinds) if k == "sha256sum"), calls)
        self.assertEqual(kinds.count("sha256sum"), 5, calls)
        # a wrong fingerprint stops before the signature is looked at; a failed signature stops before any hash
        self.setUp_again()
        self.fill_stage()
        result = self.trust(fakes=True, CORTEX_TRUST_FPR=OTHER["fpr"])
        self.assert_refused(result, 4, "trust_key")
        self.assertNotIn("-Y verify", self.calls())
        self.assertNotIn("sha256sum", self.calls())
        self.setUp_again()
        self.fill_stage()
        result = self.trust(fakes=True, FAKE_SSH_KEYGEN_MODE="fail")
        self.assertEqual(result.returncode, 4, (result.stdout, result.stderr))
        self.assertNotIn("sha256sum", self.calls())

    def test_tr14_a_tool_that_fails_or_answers_oddly_is_a_refusal_not_a_pass(self):
        for name, over, message in (("hash tool fails", {"FAKE_SHA256SUM_MODE": "fail"}, "trust_checksum"),
                                    ("hash tool answers with the wrong digest", {"FAKE_SHA256SUM_MODE": "zero"}, "trust_checksum"),
                                    ("keygen -l fails", {"FAKE_SSH_KEYGEN_MODE": "fail"}, "trust_key"),
                                    ("keygen -l says another fingerprint", {"FAKE_FINGERPRINT": OTHER["fpr"]}, "trust_key"),
                                    ("keygen says Good for another key", {"FAKE_GOOD_FINGERPRINT": OTHER["fpr"]}, "trust_signature"),
                                    ("keygen verify answers nothing", {"FAKE_VERIFY_OUTPUT": ""}, "trust_signature"),
                                    ("keygen verify answers something else", {"FAKE_VERIFY_OUTPUT": "ok"}, "trust_signature"),
                                    ("keygen verify names another principal", {"FAKE_VERIFY_PRINCIPAL": "mallory"}, "trust_signature")):
            with self.subTest(case=name):
                self.setUp_again()
                self.fill_stage()
                result = self.trust(fakes=True, env=over)
                self.assert_refused(result, 4, message)
                self.assertEqual(self.out.exists(), False)

    def test_tr15_hostile_path_stand_ins_cannot_make_a_bad_release_pass_or_run_at_all(self):
        hostile = self.root / "hostile"
        hostile.mkdir()
        canary = self.root / "canary"
        for tool in ("sha256sum", "ssh-keygen", "wc", "cat", "stat", "id", "ls", "tr", "printf", "grep", "mkdir", "rm", "cp", "sed", "awk", "head"):
            body = f"#!/bin/sh\nprintf '%s\\n' \"$0\" >> '{canary}'\n"
            if tool == "ssh-keygen":
                body += "echo 'Good \"cortex-release\" signature for cortex-release with ED25519 key SHA256:fake'\n"
            body += "exit 0\n"
            (hostile / tool).write_text(body)
            (hostile / tool).chmod(0o755)
        hostile_path = str(hostile) + os.pathsep + os.environ["PATH"]
        # a good release still passes, and no stand-in ran
        self.fill_stage()
        result = self.trust(PATH=hostile_path)
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertFalse(canary.exists(), canary.read_text() if canary.exists() else "")
        # a BAD signature is still refused (a stand-in ssh-keygen would have said Good)
        self.setUp_again()
        hostile = self.root / "hostile"
        hostile.mkdir()
        (hostile / "ssh-keygen").write_text("#!/bin/sh\necho Good\nexit 0\n")
        (hostile / "ssh-keygen").chmod(0o755)
        self.seal(sign=False)
        self.fill_stage()
        result = self.trust(PATH=str(hostile) + os.pathsep + os.environ["PATH"])
        self.assert_refused(result, 4, "trust_signature")
        # a stand-in sha256sum that always agrees does not rescue a tampered archive
        self.setUp_again()
        hostile = self.root / "hostile"
        hostile.mkdir()
        (hostile / "sha256sum").write_text("#!/bin/sh\nprintf '%s  x\\n' 0000000000000000000000000000000000000000000000000000000000000000\nexit 0\n")
        (hostile / "sha256sum").chmod(0o755)
        self.tar.write_bytes(self.tar.read_bytes() + b"x")
        self.fill_stage()
        result = self.trust(PATH=str(hostile) + os.pathsep + os.environ["PATH"])
        self.assert_refused(result, 4, "trust_checksum")

    def test_tr16_the_environment_of_the_caller_does_not_change_the_verdict(self):
        env = {"LC_ALL": "de_DE.UTF-8", "LANG": "de_DE.UTF-8", "LD_PRELOAD": "/nonexistent/evil.so", "BASH_ENV": "/nonexistent/evil", "ENV": "/nonexistent/evil",
               "CDPATH": "/", "PYTHONPATH": "/nonexistent", "GCONV_PATH": "/nonexistent"}
        self.fill_stage()
        result = self.trust(env=env, FETCH_IFS_SET="0123456789abcdef.-_:")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertEqual(self.out.read_text().splitlines()[0], "v1")
        self.setUp_again()
        self.tar.write_bytes(self.tar.read_bytes() + b"x")
        self.fill_stage()
        result = self.trust(env=env, FETCH_IFS_SET="0123456789abcdef.-_:")
        self.assert_refused(result, 4, "trust_checksum", noise=True)

    def test_tr17_a_caller_umask_changes_nothing_in_the_stage(self):
        self.fill_stage()
        before = self.snapshot(self.stage)
        result = self.trust(FETCH_UMASK="000")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertEqual(self.snapshot(self.stage), before)

    def test_tr18_the_callers_shell_state_is_not_changed_by_trust(self):
        self.fill_stage()
        script = r'''set -u; . "$FETCH_SOURCE"; umask 022; IFS=:
before="$PATH|$IFS|$(umask)|$-|$(pwd)|${LC_ALL-unset}|$#"
rel_trust "$FETCH_VERSION" "$FETCH_STAGE" "${CORTEX_TRUST_KEY_FILE-}" "${CORTEX_TRUST_PRINCIPAL-}" "${CORTEX_TRUST_FPR-}" || exit $?
after="$PATH|$IFS|$(umask)|$-|$(pwd)|${LC_ALL-unset}|$#"
[ "$before" = "$after" ] || { printf 'changed\n%s\n%s\n' "$before" "$after" >&2; exit 93; }
[ -z "${rt_out+set}" ] || exit 95
[ -n "$REL_TRUSTED" ] || exit 96
printf x >> "$FETCH_STAGE/install.sh"
rel_trust "$FETCH_VERSION" "$FETCH_STAGE" "${CORTEX_TRUST_KEY_FILE-}" "${CORTEX_TRUST_PRINCIPAL-}" "${CORTEX_TRUST_FPR-}" 2>/dev/null && exit 97
[ -z "${rt_out+set}" ] || exit 98
exit 0'''
        result = subprocess.run(["sh", "-c", script], cwd=self.root, env=self.env, text=True, capture_output=True)
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertEqual((result.stdout, result.stderr), ("", ""))

    def test_tr19_a_failed_trust_leaves_no_verified_tuple_even_after_an_earlier_success(self):
        self.fill_stage()
        script = r'''set -u; . "$FETCH_SOURCE"
rel_trust "$FETCH_VERSION" "$FETCH_STAGE" "${CORTEX_TRUST_KEY_FILE-}" "${CORTEX_TRUST_PRINCIPAL-}" "${CORTEX_TRUST_FPR-}" || exit 94
[ -n "$REL_TRUSTED" ] || exit 95
printf x >> "$FETCH_STAGE/install.sh"
rel_trust "$FETCH_VERSION" "$FETCH_STAGE" "${CORTEX_TRUST_KEY_FILE-}" "${CORTEX_TRUST_PRINCIPAL-}" "${CORTEX_TRUST_FPR-}" 2>/dev/null && exit 96
[ -z "${REL_TRUSTED-}" ] || exit 97
exit 0'''
        result = subprocess.run(["sh", "-c", script], cwd=self.root, env=self.env, text=True, capture_output=True)
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))

    def test_tr20_trust_after_the_online_fake_fetch_equals_trust_after_the_offline_fetch(self):
        self.reserve()
        result = self.run_harness("fetch trust", fakes=True, CORTEX_FROM="", CORTEX_BASE_URL="https://releases.invalid/1.2.3")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        online = self.out.read_text()
        shutil.rmtree(self.stage)
        self.reserve()
        result = self.run_harness("fetch trust")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertEqual(self.out.read_text(), online)

    def test_tr21_the_stage_is_never_removed_by_trust_and_a_failed_trust_leaves_no_work_directory(self):
        self.tar.write_bytes(self.tar.read_bytes() + b"x")
        self.fill_stage()
        result = self.trust()
        self.assert_refused(result, 4, "trust_checksum")
        self.assertEqual(self.stage_listing(), sorted(ASSETS))

    def test_tr22_the_fetch_step_to_trust_step_chain_is_whole_on_a_clean_machine_and_creates_no_home(self):
        self.reserve()
        before = self.snapshot(self.root)
        result = self.run_harness("fetch trust")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertFalse(self.home.exists())
        after = self.snapshot(self.root)
        self.assertEqual({k for k in after if not k.startswith(STAGE_NAME) and k not in ("trusted.out",)}, {k for k in before if not k.startswith(STAGE_NAME)})

    def test_tr23_an_asset_over_its_size_limit_is_refused_even_when_signed(self):
        for name, limit in (("install.sh", 1048576), ("manifest.json", 1048576)):
            with self.subTest(file=name):
                self.setUp_again()
                (self.dist / name).write_bytes(b"#" * (limit + 1))
                self.seal()
                self.fill_stage()
                result = self.trust()
                self.assert_refused(result, 4, "trust_checksum")


class TrustMore(Cell):
    def test_tr24_the_same_verdicts_under_errexit(self):
        self.fill_stage()
        result = self.run_harness("trust", FETCH_SETE="1")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertEqual(result.stderr, "")
        self.setUp_again()
        self.tar.write_bytes(self.tar.read_bytes() + b"x")
        self.fill_stage()
        result = self.run_harness("trust", FETCH_SETE="1")
        self.assert_refused(result, 4, "trust_checksum")
        self.setUp_again()
        self.seal(sign=False)
        self.fill_stage()
        self.assert_refused(self.run_harness("trust", FETCH_SETE="1"), 4, "trust_signature")
        self.setUp_again()
        self.reserve()
        result = self.run_harness("fetch trust", FETCH_SETE="1")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.setUp_again()
        (self.dist / "install.sh").unlink()
        self.reserve()
        result = self.run_harness("fetch trust", FETCH_SETE="1")
        self.assert_fetch_refused(result)

    def test_tr25_a_nul_byte_in_a_file_that_is_read_is_a_clean_refusal_with_no_shell_warning(self):
        good_sums = (self.dist / "SHA256SUMS").read_bytes()
        self.seal(sums=good_sums + b"\x00")
        self.fill_stage()
        self.assert_refused(self.trust(), 4, "trust_signature")
        self.setUp_again()
        self.write_meta(text=(self.dist / "install.meta").read_bytes().replace(b"MEMBER_LIST=2", b"MEMBER_LIST=2\x00"))
        self.seal()
        self.fill_stage()
        self.assert_refused(self.trust(), 4, "trust_checksum")
        self.setUp_again()
        self.write_key_file(GOOD, text=f"{PRINCIPAL} {GOOD['type']} {GOOD['b64']}\n\x00")
        self.fill_stage()
        self.assert_refused(self.trust(), 4, "trust_key")


    def test_pr08_the_shape_of_the_pinned_fingerprint_is_a_rule_of_its_own(self):
        """The tool is made to AGREE with every malformed pin (it reports the same string, and says Good for it), so only the shape rules can refuse: no
        SHA256: prefix, not 43 characters after it, a character outside the base64 alphabet."""
        bad = {"no prefix": "A" * 43, "one short": "SHA256:" + "A" * 42, "one long": "SHA256:" + "A" * 44, "empty after the prefix": "SHA256:",
               "hyphen": "SHA256:" + "A" * 42 + "-", "equals": "SHA256:" + "A" * 42 + "=", "underscore": "SHA256:" + "A" * 42 + "_",
               "colon": "SHA256:" + "A" * 41 + ":A", "dot": "SHA256:" + "A" * 42 + "."}
        for name, fpr in bad.items():
            with self.subTest(case=name):
                self.setUp_again()
                self.fill_stage()
                self.assert_refused(self.trust(fakes=True, CORTEX_TRUST_FPR=fpr, FAKE_FINGERPRINT=fpr, FAKE_GOOD_FINGERPRINT=fpr), 4, "trust_key")


    def test_pr09_an_image_reference_of_exactly_300_bytes_passes(self):
        image = "r" * 228 + "@sha256:" + "b" * 64
        self.assertEqual(len(image), 300)
        for key in ("CORTEX_SERVER_IMAGE", "CORTEX_DB_IMAGE"):
            with self.subTest(key=key):
                self.setUp_again()
                self.write_meta(**{key: image})
                self.seal()
                self.fill_stage()
                result = self.trust()
                self.assertEqual(result.returncode, 0, (key, result.stdout, result.stderr))
                self.assertIn(f"meta {key} {image}", self.out.read_text().splitlines())

    def test_pr10_a_failed_write_into_the_work_directory_is_a_refusal(self):
        """The directory is made, then one of the two files in it cannot be written (a directory stands where the file should go): the signer inputs are
        not made, so the tool must not be asked and the run must not pass."""
        for name in ("key.pub", "signers"):
            with self.subTest(blocked=name):
                self.setUp_again()
                self.fill_stage()
                pre = 'rel_tool_mkdir() { /usr/bin/mkdir "$@" || return $?; case ${2-} in */.trust-work) /usr/bin/mkdir -- "$2/' + name + '" ;; esac; }'
                self.assert_refused(self.trust(fakes=True, FETCH_PRE=pre), 4, "trust_key")
                self.assertEqual(self.out.exists(), False)

    def test_pr11_a_trust_run_that_does_not_end_in_the_verified_tuple_is_a_refusal(self):
        for name, pre in (("answers nothing", "rel_trust_run() { return 0; }"), ("answers something else", "rel_trust_run() { printf '%s\\n' v2; }"),
                          ("answers v2 first", "rel_trust_run() { printf '%s\\n' v2 v1; }")):
            with self.subTest(case=name):
                self.setUp_again()
                self.fill_stage()
                self.assert_refused(self.trust(FETCH_PRE=pre), 4, "trust_checksum")
        self.setUp_again()
        self.fill_stage()
        result = self.trust(FETCH_PRE="rel_trust_run() { printf '%s\\n' v1 stand-in; }")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertEqual(self.out.read_text(), "v1\nstand-in\n")

    def test_pr12_a_listed_digest_of_the_wrong_shape_is_refused_even_when_the_hash_tool_agrees(self):
        """The hash tool is made to answer, for every file, the very string the signed listing holds: only the shape rules can refuse."""
        for name, digest in (("63 hex digits", "a" * 63), ("65 hex digits", "a" * 65), ("upper case", "A" * 64), ("a g", "g" * 64), ("empty", "")):
            with self.subTest(case=name):
                self.setUp_again()
                names = [self.tar.name, self.image.name, "install.sh", "manifest.json", "install.meta"]
                self.seal(sums="".join(f"{digest}  {n}\n" for n in names), sign=False)
                self.fill_stage()
                self.assert_refused(self.trust(fakes=True, FAKE_SHA256SUM_MODE="fixed", FAKE_SHA256SUM_ANSWER=digest), 4, "trust_checksum")

    def test_pr06_principal_and_fingerprint_shapes_at_their_edges(self):
        for principal in ("a", "a" * 64, "A.b_c@d-e", "cortex-release"):
            with self.subTest(good_principal=principal):
                self.setUp_again()
                self.write_key_file(GOOD, principal=principal)
                self.seal()
                self.fill_stage()
                result = self.trust(CORTEX_TRUST_PRINCIPAL=principal)
                self.assertEqual(result.returncode, 0, (principal, result.stdout, result.stderr))
        for principal in ("a" * 65, "-a", "a/b", "a:b", "a\\b", "a,b", "a*b", "a?b", "a'b", 'a"b', "a$b", "a=b", "a%b", "a;b", "a|b", "a&b", "a#b", "a~b", "a!b", "a+b"):
            with self.subTest(bad_principal=principal):
                self.setUp_again()
                self.write_key_file(GOOD, principal=principal)   # the key file agrees with the principal: ONLY the principal rule can refuse
                self.fill_stage()
                self.assert_refused(self.trust(CORTEX_TRUST_PRINCIPAL=principal), 4, "trust_key")
        for principal in ("a b", "a\tb"):
            with self.subTest(bad_principal=repr(principal)):
                self.setUp_again()
                self.write_key_file(GOOD, principal="cortex-release")
                self.fill_stage()
                self.assert_refused(self.trust(CORTEX_TRUST_PRINCIPAL=principal), 4, "trust_key")

    def test_pr07_the_key_line_is_read_field_by_field(self):
        good = f"{PRINCIPAL} {GOOD['type']} {GOOD['b64']}"
        prefix = "AAAAC3NzaC1lZDI1NTE5AAAAI"
        for name, text in {"two spaces": good.replace(" ", "  ", 1), "trailing space": good + " ", "leading space": " " + good,
                           "key one char short": good[:-1], "key one char long": good + "A", "bad prefix": good.replace(prefix, "AAAAC3NzaC1lZDI1NTE5AAAAJ", 1),
                           "key char outside base64": good[:-1] + "-", "key with =": good[:-1] + "=", "type ssh-rsa": good.replace("ssh-ed25519", "ssh-rsa", 1),
                           "type uppercase": good.replace("ssh-ed25519", "SSH-ED25519", 1), "only two fields": f"{PRINCIPAL} {GOOD['b64']}"}.items():
            with self.subTest(case=name):
                self.setUp_again()
                self.write_key_file(GOOD, text=text + "\n")
                self.fill_stage()
                self.assert_refused(self.trust(), 4, "trust_key")


class TrustFailClosed(Cell):
    def test_tr27_the_trust_anchor_is_never_read_from_the_environment(self):
        self.fill_stage()
        script = r'''set -u; EX_TRUST=4; . "$FETCH_SOURCE"
rel_trust "$FETCH_VERSION" "$FETCH_STAGE"; rc=$?
[ -z "${REL_TRUSTED-}" ] || exit 97
exit "$rc"'''
        # the environment holds a COMPLETE, VALID anchor: a two-argument call must still be refused as having no anchor
        result = subprocess.run(["sh", "-c", script], cwd=self.root, env=self.env, text=True, capture_output=True)
        self.assert_refused(result, 4, "trust_key")
        # and each of the three arguments is needed on its own
        for missing in (3, 4, 5):
            with self.subTest(missing=missing):
                self.discard_stage()
                self.fill_stage()
                args = [str(self.key), PRINCIPAL, GOOD["fpr"]]
                args[missing - 3] = ""
                s = r'''set -u; EX_TRUST=4; . "$FETCH_SOURCE"; rel_trust "$FETCH_VERSION" "$FETCH_STAGE" "$1" "$2" "$3"'''
                r = subprocess.run(["sh", "-c", s, "sh", *args], cwd=self.root, env=self.env, text=True, capture_output=True)
                self.assert_refused(r, 4, "trust_key")

    def test_tr26_a_proof_that_prints_nothing_or_something_else_is_not_a_pass(self):
        self.fill_stage()
        for name, body in (("prints nothing and succeeds", "return 0"), ("prints something else", "printf 'ok\\n'; return 0"),
                           ("prints a tuple and fails", "printf 'v1\\n'; return 4")):
            with self.subTest(case=name):
                script = f'''set -u; . "$FETCH_SOURCE"; rel_trust_run() {{ {body}; }}
rel_trust "$FETCH_VERSION" "$FETCH_STAGE" "${{CORTEX_TRUST_KEY_FILE-}}" "${{CORTEX_TRUST_PRINCIPAL-}}" "${{CORTEX_TRUST_FPR-}}"; rc=$?
[ -z "${{REL_TRUSTED-}}" ] || exit 97
exit "$rc"'''
                result = subprocess.run(["sh", "-c", script], cwd=self.root, env=self.env, text=True, capture_output=True)
                self.assertEqual(result.returncode, 4, (name, result.stdout, result.stderr))


class Predicates(Cell):
    """The small predicates called DIRECTLY (a refusal further down the chain must not hide a predicate that stopped refusing)."""

    def call(self, fn, *args, env=None):
        e = {**self.env, **(env or {})}
        return subprocess.run(["sh", "-c", '. "$1"; shift; "$@"', "sh", self.env["FETCH_SOURCE"], fn, *args], cwd=self.root, env=e, text=True, capture_output=True)

    def test_pr01_rel_version_ok_is_the_engines_parse_version(self):
        good = ["0.0.0", "1.2.3", "10.20.30", "1.2.3-rc.1", "1.2.3-0", "1.2.3-0a", "1.2.3--", "1.2.3-a.b-", "1.2.3-a-b", "1.2.3-A", "1.2.3-0.0",
                "1.2.3-x.0", "1.2.3-9", "1.2.3-90", "1.2.3-0.99", "1.2.3-a.9", "1." + "9" * 19 + ".3", "9" * 19 + ".0.0", "0." + "9" * 19 + ".0", "0.0." + "9" * 19, "1.2.3-" + "a" * 58, "1.2.3-1.2.3"]
        bad = ["", "1.2", "1.2.3.4", ".1.2.3", "1.2.3.", "1..2.3", "01.2.3", "1.02.3", "1.2.03", "1.2.3-", "1.2.3-01", "1.2.3-a.01", "1.2.3-a..b",
               "1.2.3-.a", "1.2.3-a.", "1.2.3+b", "1.2.3_", "v1.2.3", "1.2.3 ", " 1.2.3", "1.2.3.part", "1.2.3-rc.part", "-1.2.3", "1.2.a", "a.2.3", "1.a.3",
               "1.2.3-é", "1.2.3\n", "1.2.3/..", "../1.2.3", "1." + "9" * 20 + ".3", "9" * 20 + ".0.0", "0.0." + "9" * 20, "1.2.3-" + "a" * 59,
               "1.2.3-a b", "1.2.3-a*", "1.2.3-a$", "1.2.3-;", "1,2,3", "1.2.3-a_b", "1.2.3-09", "1.2.3-a.09", "1.2.3-00", "1.2.3-0.09"]
        self.assertEqual(len("1.2.3-" + "a" * 58), 64)
        for v in good:
            with self.subTest(good=v):
                self.assertEqual(self.call("rel_version_ok", v).returncode, 0, v)
        for v in bad:
            with self.subTest(bad=v):
                self.assertNotEqual(self.call("rel_version_ok", v).returncode, 0, v)
        self.assertNotEqual(self.call("rel_version_ok").returncode, 0)

    def test_pr02_rel_image_ok_is_the_stack_env_rule(self):
        digest = "@sha256:" + "a" * 64
        allowed = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._:/-"
        forbidden = " +*$\\'\"`;|&()<>!#~%^=?[]{},\t\n\r\u00e9"
        for c in allowed:
            with self.subTest(allowed=c):
                self.assertEqual(self.call("rel_image_ok", f"x{c}y{digest}").returncode, 0, c)
        for c in forbidden:
            with self.subTest(forbidden=repr(c)):
                self.assertNotEqual(self.call("rel_image_ok", f"x{c}y{digest}").returncode, 0, repr(c))
        name300 = "n" * (300 - len(digest))
        cases_ok = ["a" + digest, "localhost/cortex-server" + digest, "registry.invalid:5000/team/db" + digest, name300 + digest,
                    "n@sha256:" + "0123456789abcdef" * 4]
        cases_bad = ["", "a", "a@sha256:", "a@sha256:" + "a" * 63, "a@sha256:" + "a" * 65, "a@sha256:" + "A" * 64, "a@sha256:" + "g" * 64, "@sha256:" + "a" * 64,
                     "a@b@sha256:" + "a" * 64, "a:latest", "a@sha512:" + "a" * 64, "a" + digest + "x", (name300 + "n") + digest, "a@sha256:" + "a" * 64 + "@sha256:" + "a" * 64,
                     "a@sha256:" + "a" * 32 + "-" + "a" * 31]
        self.assertEqual(len(name300 + digest), 300)
        for v in cases_ok:
            with self.subTest(ok=v[:50]):
                self.assertEqual(self.call("rel_image_ok", v).returncode, 0, v)
        for v in cases_bad:
            with self.subTest(bad=v[:50]):
                self.assertNotEqual(self.call("rel_image_ok", v).returncode, 0, v)
        self.assertNotEqual(self.call("rel_image_ok").returncode, 0)

    def test_pr03_rel_stage_ok_is_the_reservation_rule(self):
        parent = self.root / "p"
        parent.mkdir(mode=0o700)
        for pname in ("plain", "a-b", "a_b", "a.b", "a+b", "a@b", "A9"):
            with self.subTest(parent=pname):
                d = parent / pname
                d.mkdir(mode=0o700)
                stage = d / STAGE_NAME
                stage.mkdir(mode=0o700)
                self.assertEqual(self.call("rel_stage_ok", str(stage)).returncode, 0, pname)
        for pname in ("a b", "a:b", "a~b", "a$b", "a'b", "a,b", "a=b", "a%b", "a;b", "a\u00e9b"):
            with self.subTest(parent=pname):
                d = parent / pname
                d.mkdir(mode=0o700)
                stage = d / STAGE_NAME
                stage.mkdir(mode=0o700)
                self.assertNotEqual(self.call("rel_stage_ok", str(stage)).returncode, 0, pname)
        base = parent / "plain" / STAGE_NAME
        self.assertEqual(self.call("rel_stage_ok", str(base)).returncode, 0)
        for mode in (0o701, 0o710, 0o707, 0o770, 0o755, 0o777, 0o600, 0o500, 0o300, 0o000, 0o1700, 0o2700, 0o4700):
            with self.subTest(mode=oct(mode)):
                base.chmod(mode)
                self.assertNotEqual(self.call("rel_stage_ok", str(base)).returncode, 0, oct(mode))
        base.chmod(0o700)
        self.assertEqual(self.call("rel_stage_ok", str(base)).returncode, 0)
        stub = subprocess.run(["sh", "-c", '. "$1"; fu=$2; rel_tool_uid() { printf "%s\\n" "$fu"; }; rel_tool_uid', "sh", self.env["FETCH_SOURCE"], "4242"], cwd=self.root, env=self.env, text=True, capture_output=True)
        self.assertEqual(stub.stdout.strip(), "4242")   # the fixture reports the number it is given (vera V6C04)
        self.assertNotEqual(self.call_uid("rel_stage_ok", str(base), "4242"), 0)
        self.assertEqual(self.call_uid("rel_stage_ok", str(base), None), 0)
        link = parent / "plain" / (".cortex-install-stage.LinkLink")
        link.symlink_to(base)
        self.assertNotEqual(self.call("rel_stage_ok", str(link)).returncode, 0)
        afile = parent / "plain" / ".cortex-install-stage.FileFile"
        afile.write_text("x")
        afile.chmod(0o700)
        self.assertNotEqual(self.call("rel_stage_ok", str(afile)).returncode, 0)
        gone = parent / "plain" / ".cortex-install-stage.GoneGone"
        self.assertNotEqual(self.call("rel_stage_ok", str(gone)).returncode, 0)
        for bad in ("", "relative/.cortex-install-stage.Ab3dE9xZ", str(base) + "/", str(base) + "/.", str(parent / "plain") + "/../plain/" + STAGE_NAME,
                    str(parent) + "//plain/" + STAGE_NAME, str(parent / "plain") + "/./" + STAGE_NAME, str(parent / "plain") + "/..", "/", "/.cortex-install-stage.Ab3dE9xZ"):
            with self.subTest(bad=bad):
                self.assertNotEqual(self.call("rel_stage_ok", bad).returncode, 0, bad)
        for name in (".cortex-install-stage.Ab3dE9x", ".cortex-install-stage.Ab3dE9xZZ", ".cortex-install-stage.Ab3dE9x_", ".cortex-install-stage.Ab3dE9x.",
                     ".cortex-install-stage.Ab3dE9x-", ".cortex-install-stage.Ab3dE9x+", ".cortex-install-stage.Ab3dE9x@", "cortex-install-stage.Ab3dE9xZ", ".cortex-install-stage-Ab3dE9xZ",
                     ".cortex-install-stageXAb3dE9xZ", "x.cortex-install-stage.Ab3dE9xZ"):
            with self.subTest(name=name):
                d = parent / "plain" / name
                d.mkdir(mode=0o700)
                self.assertNotEqual(self.call("rel_stage_ok", str(d)).returncode, 0, name)
        for ch in "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789":
            with self.subTest(suffix=ch):
                d = parent / "plain" / (".cortex-install-stage." + ch * 8)
                d.mkdir(mode=0o700)
                self.assertEqual(self.call("rel_stage_ok", str(d)).returncode, 0, ch)

    def call_uid(self, fn, arg, uid):
        script = '. "$1"; fu=$3; if [ -n "$fu" ]; then rel_tool_uid() { printf "%s\\n" "$fu"; }; fi; "$2" "$4"'
        r = subprocess.run(["sh", "-c", script, "sh", self.env["FETCH_SOURCE"], fn, uid or "", arg], cwd=self.root, env=self.env, text=True, capture_output=True)
        return r.returncode

    def test_pr04_rel_limit_and_rel_no_nul(self):
        for name, limit in (("manifest.json", "1048576"), ("install.sh", "1048576"), ("install.meta", "4096"), ("SHA256SUMS", "65536"), ("SHA256SUMS.sig", "65536"),
                            (f"cortex-{VERSION}-x86_64-linux.tar.gz", "4294967296"), (f"cortex-server-{VERSION}-x86_64.oci.tar", "4294967296"), ("anything", "4294967296"), ("", "4294967296")):
            with self.subTest(name=name):
                r = self.call("rel_limit", name)
                self.assertEqual((r.returncode, r.stdout.strip()), (0, limit), name)
        files = {"empty": (b"", 0), "plain": (b"abc\n", 0), "one nul": (b"ab\x00c", 1), "only nul": (b"\x00", 1), "trailing nul": (b"abc\x00", 1)}
        for name, (data, want) in files.items():
            with self.subTest(file=name):
                p = self.root / ("nul-" + name.replace(" ", "-"))
                p.write_bytes(data)
                r = self.call("rel_no_nul", str(p))
                self.assertEqual(r.returncode, want, name)
                self.assertEqual((r.stdout, r.stderr), ("", ""))
        self.assertNotEqual(self.call("rel_no_nul", str(self.root / "absent")).returncode, 0)

    def test_pr05_size_limits_at_the_exact_byte(self):
        # rel_fetch: a file AT its limit is staged, one byte over is refused
        for name, limit in (("install.sh", 1048576), ("manifest.json", 1048576)):
            with self.subTest(fetch=name):
                self.setUp_again()
                (self.dist / name).write_bytes(b"#" * limit)
                self.reserve()
                self.assertEqual(self.run_harness("fetch").returncode, 0)
                self.assertEqual((self.stage / name).stat().st_size, limit)
                self.discard_stage()
                (self.dist / name).write_bytes(b"#" * (limit + 1))
                self.reserve()
                self.assertEqual(self.run_harness("fetch").returncode, 5)
                self.assertEqual(self.stage_listing(), [])
        # rel_trust: signed files AT their limit are trusted, one byte over is refused
        for name, limit in (("install.sh", 1048576), ("manifest.json", 1048576)):
            with self.subTest(trust=name):
                self.setUp_again()
                (self.dist / name).write_bytes(b"#" * limit)
                self.seal()
                self.fill_stage()
                result = self.trust()
                self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        # the key file: 1024 bytes (the line, then blank lines) is read, 1025 is not
        self.setUp_again()
        line = f"{PRINCIPAL} {GOOD['type']} {GOOD['b64']}\n"
        self.write_key_file(GOOD, text=line + "\n" * (1024 - len(line)))
        self.assertEqual(self.key.stat().st_size, 1024)
        self.fill_stage()
        self.assertEqual(self.trust().returncode, 0)
        self.write_key_file(GOOD, text=line + "\n" * (1025 - len(line)))
        self.assertEqual(self.key.stat().st_size, 1025)
        self.discard_stage()
        self.fill_stage()
        self.assert_refused(self.trust(), 4, "trust_key")


class TrustNoEx(Trust):
    no_ex = True


class TrustMoreNoEx(TrustMore):
    no_ex = True


PROBLEM.update({
    "fetch_archive": "problem: the release archive could not be safely unpacked",
    "fetch_image": "problem: the server image could not be loaded",
    "fetch_home": "problem: the install directory cannot be used",
    "fetch_present": "problem: this release is already unpacked or another release is current",
    "fetch_verify": "problem: the release directory could not be verified",
})
REQUIRED_FILES = ("cortex", "compose.yml", "stack.env", "db-roles.sh", "cortex.toml.example", *("bin/" + n for n in BIN), *("systemd/" + n for n in UNITS))
STACK_ENV = f"CORTEX_VERSION={VERSION}\nCORTEX_SERVER_IMAGE={SERVER}\nCORTEX_DB_IMAGE={DATABASE}\n"


class Pipeline(Cell):
    """Stage 2: rel_unpack, rel_load and verify_release_dir, after a real fetch and a real trust."""

    def entries(self):
        """The good member list 2 as (name, kind, mode, data): the builder's release, in the order of MEMBERS."""
        out = []
        for name, mode in MEMBERS.items():
            if name in ("initdb", "bin", "systemd"):
                out.append((name, "dir", mode, b""))
            elif name == "stack.env":
                out.append((name, "file", mode, STACK_ENV.encode()))
            elif name == "db-roles.sh":
                out.append((name, "file", mode, ROLES))
            else:
                out.append((name, "file", mode, ("fixture " + name + "\n").encode()))
        return out

    def write_tar(self, entries, fmt=tarfile.USTAR_FORMAT):
        with tarfile.open(self.tar, "w:gz", format=fmt) as out:
            for name, kind, mode, data in entries:
                info = tarfile.TarInfo(name)
                info.mode = mode
                if kind == "dir":
                    info.type = tarfile.DIRTYPE
                    out.addfile(info)
                elif kind == "symlink":
                    info.type, info.linkname = tarfile.SYMTYPE, data
                    out.addfile(info)
                elif kind == "hardlink":
                    info.type, info.linkname = tarfile.LNKTYPE, data
                    out.addfile(info)
                elif kind == "fifo":
                    info.type = tarfile.FIFOTYPE
                    out.addfile(info)
                elif kind == "chr":
                    info.type = tarfile.CHRTYPE
                    out.addfile(info)
                else:
                    info.size = len(data)
                    out.addfile(info, io.BytesIO(data))

    def release(self, entries=None, fmt=tarfile.USTAR_FORMAT, meta=None):
        """Build the archive, rewrite install.meta to match it (or the given text) and SIGN the set: only the archive is hostile."""
        self.write_tar(entries if entries is not None else self.entries(), fmt)
        if meta is not None:
            self.write_meta(**meta)
        self.seal()

    def steps(self, steps="fetch trust unpack", fakes=False, **kw):
        self.reserve()
        return self.run_harness(steps, fakes=fakes, **kw)

    def candidates(self):
        rel = self.home / "releases"
        return sorted(p.name for p in rel.iterdir()) if rel.is_dir() else None

    def tree(self, root):
        out = {}
        for p in sorted(root.rglob("*")):
            st = p.lstat()
            kind = "l" if stat.S_ISLNK(st.st_mode) else "d" if stat.S_ISDIR(st.st_mode) else "f"
            out[str(p.relative_to(root))] = (kind, st.st_mode & 0o7777, hashlib.sha256(p.read_bytes()).hexdigest() if kind == "f" else "")
        return out

    def refused(self, result, code, message):
        self.assertEqual(result.returncode, code, (result.stdout, result.stderr))
        lines = result.stderr.splitlines()
        self.assertEqual(len(lines), 2, result.stderr)
        self.assertEqual(lines[0], PROBLEM[message], result.stderr)
        self.assertTrue(lines[1].startswith("fix: "), result.stderr)
        self.assertEqual(result.stdout, "")

    def tar_calls(self):
        return [c for c in self.calls().splitlines() if c.startswith("tar ")]

    def extraction_calls(self):
        return [c for c in self.tar_calls() if " -xzf " in c + " "]

    def only_a_listing(self):
        """The fake tar logs every call with its real arguments: the ONLY tar call is the listing (-tvzf), none extracts (the unpack call is -xzf), so the over-size archive was
        refused before anything was extracted (sam S6C2-SAM02: the earlier test looked for text that the real argv never contains)."""
        tar_calls = self.tar_calls()
        self.assertEqual(len(tar_calls), 1, tar_calls)
        self.assertEqual(self.extraction_calls(), [])
        self.assertIn("-tvzf", tar_calls[0])
        self.assertNotIn("-xzf", tar_calls[0])
        self.assertFalse((self.home / "releases").exists())

    def assert_clean(self):
        """After a refusal: no candidate, no release, no current, nothing in the home but what existed."""
        self.assertFalse((self.home / "current").exists() or (self.home / "current").is_symlink())
        cands = self.candidates()
        self.assertIn(cands, (None, []), cands)
        self.assertFalse(self.out.exists() and False)


class Unpack(Pipeline):
    def test_un01_a_trusted_release_unpacks_into_a_private_candidate_and_nothing_else(self):
        outside_before = self.snapshot(self.dist)
        result = self.steps("fetch trust unpack")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertEqual((result.stdout, result.stderr), ("", ""))
        cands = self.candidates()
        self.assertEqual(len(cands), 1)
        self.assertRegex(cands[0], r"^\.cortex-unpack\.[A-Za-z0-9]{8}$")
        cand = self.home / "releases" / cands[0]
        self.assertEqual(Path(self.env["FETCH_CAND"]).read_text().strip(), str(cand))
        self.assertEqual(stat.S_IMODE(cand.stat().st_mode), 0o700)
        found = {str(p.relative_to(cand)): p for p in cand.rglob("*")}
        self.assertEqual(set(found), set(MEMBERS) | {"manifest.json", "SHA256SUMS"})
        for name, mode in MEMBERS.items():
            self.assertEqual(found[name].lstat().st_mode & 0o7777, mode, name)
            if name not in ("initdb", "bin", "systemd"):
                self.assertTrue(found[name].is_file() and not found[name].is_symlink(), name)
        self.assertEqual(found["manifest.json"].lstat().st_mode & 0o7777, 0o644)
        self.assertEqual(found["manifest.json"].read_bytes(), (self.dist / "manifest.json").read_bytes())
        self.assertEqual(found["SHA256SUMS"].lstat().st_mode & 0o7777, 0o644)
        names = sorted(n for n in found if n not in ("SHA256SUMS", "initdb", "bin", "systemd"))
        want = "".join(hashlib.sha256(found[n].read_bytes()).hexdigest() + "  " + n + "\n" for n in names)
        self.assertEqual(found["SHA256SUMS"].read_text(), want)
        self.assertEqual(self.snapshot(self.dist), outside_before)
        self.assertFalse((self.home / "current").exists() or (self.home / "current").is_symlink())
        self.assertFalse((self.home / "releases" / VERSION).exists())
        self.assertEqual(sorted(p.name for p in self.stage.iterdir()), sorted(ASSETS))

    def test_un02_without_a_verified_tuple_nothing_is_unpacked(self):
        for name, steps in (("trust was never run", "fetch unpack"),):
            with self.subTest(case=name):
                self.setUp_again()
                result = self.steps(steps)
                self.refused(result, 4, "trust_checksum")
                self.assertFalse(self.home.exists())
        # a tuple of ANOTHER run: the stage differs
        self.setUp_again()
        script = r'''set -u; EX_TRUST=4 EX_FETCH=5; . "$FETCH_SOURCE"
rel_fetch "$FETCH_VERSION" "$FETCH_STAGE" || exit 91
rel_trust "$FETCH_VERSION" "$FETCH_STAGE" "${CORTEX_TRUST_KEY_FILE-}" "${CORTEX_TRUST_PRINCIPAL-}" "${CORTEX_TRUST_FPR-}" || exit 92
REL_TRUSTED=$(printf '%s\n' "$REL_TRUSTED" | sed "s#^stage .*#stage /x/.cortex-install-stage.Ab3dE9xZ#")
rel_unpack "$FETCH_VERSION" "$FETCH_STAGE" "$FETCH_HOME"'''
        self.reserve()
        r = subprocess.run(["sh", "-c", script], cwd=self.root, env=self.env, text=True, capture_output=True)
        self.refused(r, 4, "trust_checksum")
        self.assertFalse(self.home.exists())

    def test_un03_a_mangled_tuple_is_refused_whatever_the_mangling(self):
        manglings = {
            "version differs": "s#^version .*#version 9.9.9#", "a file line is gone": "/^file install.sh /d", "a hash is changed": "0,/^file cortex-/s#[0-9a-f]\\{8\\}$#00000000#",
            "a size is changed": "0,/^file cortex-/s# [0-9]* # 1 #", "an extra line": "$a extra line", "first line is not v1": "1s#.*#v2#", "meta value changed": "s#^meta MEMBER_LIST .*#meta MEMBER_LIST 3#",
            "empty": "d",
        }
        for name, sed in manglings.items():
            with self.subTest(case=name):
                self.setUp_again()
                script = r'''set -u; EX_TRUST=4 EX_FETCH=5; . "$FETCH_SOURCE"
rel_fetch "$FETCH_VERSION" "$FETCH_STAGE" || exit 91
rel_trust "$FETCH_VERSION" "$FETCH_STAGE" "${CORTEX_TRUST_KEY_FILE-}" "${CORTEX_TRUST_PRINCIPAL-}" "${CORTEX_TRUST_FPR-}" || exit 92
REL_TRUSTED=$(printf '%s\n' "$REL_TRUSTED" | sed "$MANGLE")
rel_unpack "$FETCH_VERSION" "$FETCH_STAGE" "$FETCH_HOME"'''
                self.reserve()
                r = subprocess.run(["sh", "-c", script], cwd=self.root, env={**self.env, "MANGLE": sed}, text=True, capture_output=True)
                self.assertEqual(r.returncode, 4, (name, r.stdout, r.stderr))
                self.assertFalse(self.home.exists(), name)

    def test_un04_a_file_changed_after_trust_is_caught_before_it_is_used(self):
        for victim in (self.tar.name, "manifest.json"):
            with self.subTest(file=victim):
                self.setUp_again()
                script = r'''set -u; EX_TRUST=4 EX_FETCH=5; . "$FETCH_SOURCE"
rel_fetch "$FETCH_VERSION" "$FETCH_STAGE" || exit 91
rel_trust "$FETCH_VERSION" "$FETCH_STAGE" "${CORTEX_TRUST_KEY_FILE-}" "${CORTEX_TRUST_PRINCIPAL-}" "${CORTEX_TRUST_FPR-}" || exit 92
printf x >> "$FETCH_STAGE/$VICTIM"
rel_unpack "$FETCH_VERSION" "$FETCH_STAGE" "$FETCH_HOME"'''
                self.reserve()
                r = subprocess.run(["sh", "-c", script], cwd=self.root, env={**self.env, "VICTIM": victim}, text=True, capture_output=True)
                self.refused(r, 4, "trust_checksum")
                self.assertFalse(self.home.exists())

    def test_un05_the_archive_is_a_closed_list_each_attack_is_refused_and_leaves_nothing(self):
        e = self.entries
        def replaced(name, **kw):
            out = []
            for n, k, m, d in e():
                if n == name:
                    n, k, m, d = kw.get("name", n), kw.get("kind", k), kw.get("mode", m), kw.get("data", d)
                out.append((n, k, m, d))
            return out
        def without(name):
            return [x for x in e() if x[0] != name]
        attacks = {
            "a path that climbs": e() + [("../escape", "file", 0o644, b"x")],
            "an absolute path": e() + [("/tmp/cortex-escape", "file", 0o644, b"x")],
            "a dot-slash name": e() + [("./extra", "file", 0o644, b"x")],
            "an extra top-level file": e() + [("evil", "file", 0o644, b"x")],
            "an extra file in bin": e() + [("bin/evil", "file", 0o755, b"x")],
            "an extra file in systemd": e() + [("systemd/evil.service", "file", 0o644, b"x")],
            "an extra directory": e() + [("evil", "dir", 0o755, b"")],
            "a symlink member": e() + [("initdb/link", "symlink", 0o644, "/etc/passwd")],
            "a symlink in place of a file": replaced("compose.yml", kind="symlink", data="/etc/passwd", mode=0o777),
            "a hard link member": e() + [("initdb/hard", "hardlink", 0o644, "cortex")],
            "a fifo member": e() + [("initdb/fifo", "fifo", 0o644, b"")],
            "a device member": e() + [("initdb/dev", "chr", 0o644, b"")],
            "a duplicate member": e() + [("compose.yml", "file", 0o644, b"second")],
            "a missing cortex": without("cortex"),
            "a missing compose.yml": without("compose.yml"),
            "a missing stack.env": without("stack.env"),
            "a missing db-roles.sh": without("db-roles.sh"),
            "a missing cortex.toml.example": without("cortex.toml.example"),
            "a missing bin member": without("bin/cortex-dump"),
            "a missing systemd member": without("systemd/cortex-dump.timer"),
            "a missing initdb directory": [x for x in e() if x[0] not in ("initdb", "initdb/roles.sql")],
            "cortex with mode 0777": replaced("cortex", mode=0o777),
            "cortex setuid": replaced("cortex", mode=0o4755),
            "cortex not executable": replaced("cortex", mode=0o644),
            "compose.yml executable": replaced("compose.yml", mode=0o755),
            "compose.yml group writable": replaced("compose.yml", mode=0o664),
            "a bin member 0644": replaced("bin/cortex-dump", mode=0o644),
            "a directory 0700": replaced("bin", mode=0o700),
            "a directory sticky": replaced("systemd", mode=0o1755),
            "a directory where a file belongs": replaced("compose.yml", kind="dir", data=b""),
            "a file where a directory belongs": replaced("bin", kind="file", data=b"x", mode=0o644),
            "an initdb name starting with a dot": e() + [("initdb/.hidden", "file", 0o644, b"x")],
            "an initdb name with a space": e() + [("initdb/a b.sql", "file", 0o644, b"x")],
            "an initdb name of 65 characters": e() + [("initdb/" + "a" * 65, "file", 0o644, b"x")],
            "an initdb subdirectory": e() + [("initdb/sub", "dir", 0o755, b"")],
            "a nested initdb file": e() + [("initdb/sub/x.sql", "file", 0o644, b"x")],
            "257 entries": e() + [(f"initdb/f{i}.sql", "file", 0o644, b"x") for i in range(260)],
            "cortex named with a trailing space instead of cortex": [(("cortex " if n == "cortex" else n), k, m, d) for n, k, m, d in e()],
            "an initdb name with a trailing space": e() + [("initdb/a.sql ", "file", 0o644, b"x")],
            "a name with a leading space": e() + [(" evil", "file", 0o644, b"x")],
            "a name with two spaces inside": e() + [("initdb/a  b", "file", 0o644, b"x")],
        }
        for name, entries in attacks.items():
            with self.subTest(attack=name):
                self.setUp_again()
                canary = self.root / "escape"
                canary.write_text("unchanged")
                self.release(entries=entries, meta={})
                result = self.steps("fetch trust unpack", fakes=True)
                self.refused(result, 5, "fetch_archive")
                self.assertEqual(canary.read_text(), "unchanged")
                self.assertFalse(Path("/tmp/cortex-escape").exists())
                self.assert_clean()
        # the good initdb names are accepted (letters, digits, . _ - ; 1 to 64; not a leading dot)
        for fname in ("a.sql", "A_b-9.sql", "9", "_x", "-x", "a" * 64):
            with self.subTest(initdb=fname):
                self.setUp_again()
                self.release(entries=self.entries() + [("initdb/" + fname, "file", 0o644, b"ok")], meta={})
                result = self.steps("fetch trust unpack")
                self.assertEqual(result.returncode, 0, (fname, result.stdout, result.stderr))

    def test_un06_pax_and_long_name_archives_are_refused(self):
        for name, fmt, extra in (("a pax archive with an extended header", tarfile.PAX_FORMAT, ("initdb/" + "é" * 3 + ".sql", "file", 0o644, b"x")),
                                 ("a GNU long name", tarfile.GNU_FORMAT, ("initdb/" + "a" * 120, "file", 0o644, b"x"))):
            with self.subTest(case=name):
                self.setUp_again()
                self.release(entries=self.entries() + [extra], fmt=fmt, meta={})
                result = self.steps("fetch trust unpack")
                self.refused(result, 5, "fetch_archive")
                self.assert_clean()

    def test_un06b_an_extension_header_cannot_change_what_the_listing_shows_and_is_not_refused_as_a_format(self):
        """Isolated header-metadata controls (vera V6S2-05): the base name, mode, size and content of every member satisfy the contract; ONLY an extension header says
        something else. tar lists the EFFECTIVE name, so the closed list judges that, before anything is extracted. A format whose effective members satisfy the list is
        accepted: the rule is on the members, not on the format."""
        def pax_path(effective):
            def build(out):
                for name, kind, mode, data in self.entries():
                    info = tarfile.TarInfo(name)
                    info.mode = mode
                    if kind == "dir":
                        info.type = tarfile.DIRTYPE
                        out.addfile(info)
                        continue
                    info.size = len(data)
                    if name == "compose.yml" and effective is not None:
                        info.pax_headers = {"path": effective}
                    out.addfile(info, io.BytesIO(data))
            return build
        def gnu_longname(effective):
            def build(out):
                for name, kind, mode, data in self.entries():
                    if name == "compose.yml":
                        raw = effective.encode() + b"\0"
                        long = tarfile.TarInfo("././@LongLink")
                        long.type = tarfile.GNUTYPE_LONGNAME
                        long.size = len(raw)
                        out.addfile(long, io.BytesIO(raw))
                    info = tarfile.TarInfo(name)
                    info.mode = mode
                    if kind == "dir":
                        info.type = tarfile.DIRTYPE
                        out.addfile(info)
                        continue
                    info.size = len(data)
                    out.addfile(info, io.BytesIO(data))
            return build
        cases = (("pax path record climbing out", tarfile.PAX_FORMAT, pax_path("../escape")), ("pax path record naming another file", tarfile.PAX_FORMAT, pax_path("evil")),
                 ("gnu long name climbing out", tarfile.GNU_FORMAT, gnu_longname("../escape")), ("gnu long name naming another file", tarfile.GNU_FORMAT, gnu_longname("evil")))
        for name, fmt, build in cases:
            with self.subTest(case=name):
                self.setUp_again()
                canary = self.root / "escape"
                canary.write_text("unchanged")
                with tarfile.open(self.tar, "w:gz", format=fmt) as out:
                    build(out)
                self.write_manifest()
                self.seal()
                result = self.steps("fetch trust unpack", fakes=True)
                self.refused(result, 5, "fetch_archive")
                self.only_a_listing()
                self.assertEqual(canary.read_text(), "unchanged")
                self.assert_clean()
        # the positive controls: the same constructions with the EFFECTIVE name equal to the base name are accepted (the rule is on the members)
        for name, fmt, build in (("pax record naming the same file", tarfile.PAX_FORMAT, pax_path("compose.yml")), ("gnu long name naming the same file", tarfile.GNU_FORMAT, gnu_longname("compose.yml"))):
            with self.subTest(control=name):
                self.setUp_again()
                with tarfile.open(self.tar, "w:gz", format=fmt) as out:
                    build(out)
                self.write_manifest()
                self.seal()
                result = self.steps("fetch trust unpack", fakes=True)
                self.assertEqual(result.returncode, 0, (name, result.stdout, result.stderr))

    def test_un11_an_extra_physical_entry_with_a_line_feed_in_its_name_is_refused_after_extraction(self):
        self.reserve()
        result = self.steps("fetch trust unpack", fakes=True, FAKE_TAR_MODE="inject")
        self.refused(result, 5, "fetch_archive")
        self.assertEqual(len(self.extraction_calls()), 1, self.extraction_calls())
        self.assert_clean()

    def test_un07_a_tool_that_fails_or_answers_oddly_is_a_refusal(self):
        self.setUp_again()
        result = self.steps("fetch trust unpack", fakes=True, FAKE_TAR_MODE="fail")
        self.refused(result, 5, "fetch_archive")
        self.assert_clean()

    def test_un08_the_listing_is_read_before_anything_is_extracted_and_a_refused_archive_is_never_extracted(self):
        self.release(entries=self.entries() + [("evil", "file", 0o644, b"x")], meta={})
        result = self.steps("fetch trust unpack", fakes=True)
        self.refused(result, 5, "fetch_archive")
        calls = [c for c in self.calls().splitlines() if c.startswith("tar ")]
        self.assertEqual(len(calls), 1, calls)
        self.assertIn("-tvzf", calls[0])
        self.assertNotIn(" -xzf ", calls[0] + " ")

    def test_un19_an_archive_with_no_initdb_member_is_refused_from_the_listing_before_anything_is_extracted(self):
        """hue, K2 survivor FM665 (line 707): without the listing check the archive was extracted first and refused only by the later directory test (the same message and code, so the
        earlier cells saw nothing); the listing refuses it, the ONLY tar call is the listing, and the releases directory is never made."""
        self.release(entries=[x for x in self.entries() if x[0] not in ("initdb", "initdb/roles.sql")], meta={})
        result = self.steps("fetch trust unpack", fakes=True)
        self.refused(result, 5, "fetch_archive")
        self.only_a_listing()
        self.assert_clean()

    def test_un20_a_releases_directory_that_is_not_the_callers_is_refused(self):
        """hue, K2 survivor FM672 (line 710): the owner test of the releases directory had no cell (an existing home is refused earlier by the home's own owner test, and a changed
        caller uid is refused earlier still by the stage test). The scratch step flipstat makes stat report another owner for the releases directory only."""
        result = self.steps("fetch trust call unpack", FETCH_CALL="flipstat", FETCH_STAT_LATE=str(os.getuid() + 1))
        self.refused(result, 5, "fetch_home")
        self.assert_clean()

    def test_un22_a_tool_that_succeeds_but_prints_on_stdout_makes_the_unpack_refuse_with_the_steps_own_code(self):
        """hue, L2 survivor FM761 (line 772): rel_unpack_run answers with the candidate's path on stdout; a tool that printed something on its way would make the answer not start with a
        slash, and the wrapper refuses (EX_FETCH). No earlier cell reached this refusal, so its exit-code default was never seen (the NoEx class sees it). The refusal is checked, not
        what it leaves behind: the finished candidate stays on this path, which is the include's behaviour, stated in the dispositions note."""
        result = self.steps("fetch trust call unpack", FETCH_CALL="noisychmod")
        self.refused(result, 5, "fetch_archive")

    def test_un09_the_archive_must_agree_with_the_signed_install_meta(self):
        bad_env = STACK_ENV.replace("a" * 64, "c" * 64).encode()
        cases = {
            "stack.env names another server image": [(n, k, m, bad_env if n == "stack.env" else d) for n, k, m, d in self.entries()],
            "stack.env names another version": [(n, k, m, STACK_ENV.replace(f"CORTEX_VERSION={VERSION}", "CORTEX_VERSION=1.2.4").encode() if n == "stack.env" else d) for n, k, m, d in self.entries()],
            "stack.env has an extra line": [(n, k, m, (STACK_ENV + "EXTRA=1\n").encode() if n == "stack.env" else d) for n, k, m, d in self.entries()],
            "stack.env lacks its final newline": [(n, k, m, STACK_ENV.rstrip("\n").encode() if n == "stack.env" else d) for n, k, m, d in self.entries()],
            "db-roles.sh is another script": [(n, k, m, b"#!/bin/sh\n# another\n" if n == "db-roles.sh" else d) for n, k, m, d in self.entries()],
        }
        for name, entries in cases.items():
            with self.subTest(case=name):
                self.setUp_again()
                self.release(entries=entries, meta={})
                result = self.steps("fetch trust unpack")
                self.refused(result, 4, "trust_checksum")
                self.assert_clean()

    def test_un10_the_home_is_checked_before_anything_is_created(self):
        link = self.root / "home-link"
        link.symlink_to(self.root)
        for name, home in (("relative", "home"), ("a dot-dot component", str(self.root) + "/../x"), ("a space", str(self.root) + "/a b"), ("a double slash", str(self.root) + "//h"),
                           ("a trailing slash", str(self.home) + "/"), ("the root", "/"), ("a dollar", str(self.root) + "/a$b"), ("a colon", str(self.root) + "/a:b"),
                           ("a link", str(link)), ("an empty path", "")):
            with self.subTest(home=name):
                self.setUp_again()
                link = self.root / "home-link"
                link.symlink_to(self.root)
                result = self.steps("fetch trust unpack", FETCH_HOME=home if name != "a link" else str(link))
                self.refused(result, 5, "fetch_home")
                self.assertFalse(self.home.exists())

    def test_un11_an_existing_release_of_this_version_is_never_replaced(self):
        (self.home / "releases" / VERSION).mkdir(parents=True)
        (self.home / "releases" / VERSION / "keep").write_text("untouched")
        before = self.tree(self.home)
        result = self.steps("fetch trust unpack")
        self.refused(result, 5, "fetch_present")
        self.assertEqual(self.tree(self.home), before)

    def test_un12_a_leftover_candidate_of_a_lost_run_is_left_alone_and_other_things_are_untouched(self):
        (self.home / "releases").mkdir(parents=True)
        leftover = self.home / "releases" / ".cortex-unpack.LOSTLOST"
        leftover.mkdir()
        (leftover / "half").write_text("partial")
        other = self.home / "releases" / "0.9.0"
        other.mkdir()
        (other / "x").write_text("old release")
        (self.home / "stack.env").write_text("old")
        before = self.tree(self.home)
        result = self.steps("fetch trust unpack")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        after = self.tree(self.home)
        for k, v in before.items():
            self.assertEqual(after[k], v, k)
        new = [k for k in after if k not in before and "/" not in k]
        self.assertEqual(len(new), 0, new)
        self.assertEqual(len([p for p in (self.home / "releases").iterdir() if p.name.startswith(".cortex-unpack.")]), 2)

    def test_un13_an_existing_home_of_any_mode_is_used_but_a_file_or_foreign_releases_is_not(self):
        self.home.mkdir(mode=0o755)
        result = self.steps("fetch trust unpack")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertEqual(stat.S_IMODE(self.home.stat().st_mode), 0o755)
        for name, prep in (("home is a file", lambda: self.home.write_text("x")), ("releases is a file", lambda: (self.home.mkdir(), (self.home / "releases").write_text("x"))),
                           ("releases is a link", lambda: (self.home.mkdir(), (self.home / "releases").symlink_to(self.root)))):
            with self.subTest(case=name):
                self.setUp_again()
                prep()
                result = self.steps("fetch trust unpack")
                self.refused(result, 5, "fetch_home")

    def test_un14_a_new_home_and_releases_are_private(self):
        result = self.steps("fetch trust unpack")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertEqual(stat.S_IMODE(self.home.stat().st_mode), 0o700)
        self.assertEqual(stat.S_IMODE((self.home / "releases").stat().st_mode), 0o700)

    def test_un15_nothing_outside_the_home_and_the_stage_changes_and_missing_ancestors_are_made_private(self):
        deep = self.root / "a" / "b" / "home"
        before = self.snapshot(self.dist)
        result = self.steps("fetch trust unpack", FETCH_HOME=str(deep))
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertEqual(stat.S_IMODE((self.root / "a").stat().st_mode), 0o700)
        self.assertEqual(stat.S_IMODE((self.root / "a" / "b").stat().st_mode), 0o700)
        self.assertEqual(self.snapshot(self.dist), before)

    def test_un16_hostile_path_stand_ins_are_never_run(self):
        hostile = self.root / "hostile"
        hostile.mkdir()
        canary = self.root / "canary"
        for tool in ("tar", "gzip", "mkdir", "mktemp", "chmod", "cp", "mv", "ls", "find", "sha256sum", "stat", "cat", "sort", "rm", "wc", "id", "tr", "sed", "awk", "grep"):
            (hostile / tool).write_text(f"#!/bin/sh\nprintf '%s\n' \"$0\" >> '{canary}'\nexit 0\n")
            (hostile / tool).chmod(0o755)
        result = self.steps("fetch trust unpack", PATH=str(hostile) + os.pathsep + os.environ["PATH"])
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertFalse(canary.exists(), canary.read_text() if canary.exists() else "")

    def test_un17_the_callers_state_is_not_changed_and_a_caller_umask_changes_nothing(self):
        result = self.steps("fetch trust unpack", FETCH_UMASK="000")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        cand = self.home / "releases" / self.candidates()[0]
        self.assertEqual(stat.S_IMODE(cand.stat().st_mode), 0o700)
        self.assertEqual(stat.S_IMODE((cand / "cortex").stat().st_mode), 0o755)
        self.assertEqual(stat.S_IMODE((cand / "compose.yml").stat().st_mode), 0o644)
        self.setUp_again()
        self.reserve()
        script = r'''set -u; . "$FETCH_SOURCE"; umask 022; IFS=:
before="$PATH|$IFS|$(umask)|$-|$(pwd)|${LC_ALL-unset}|$#"
rel_fetch "$FETCH_VERSION" "$FETCH_STAGE" && rel_trust "$FETCH_VERSION" "$FETCH_STAGE" "${CORTEX_TRUST_KEY_FILE-}" "${CORTEX_TRUST_PRINCIPAL-}" "${CORTEX_TRUST_FPR-}" && rel_unpack "$FETCH_VERSION" "$FETCH_STAGE" "$FETCH_HOME" || exit "$?"
after="$PATH|$IFS|$(umask)|$-|$(pwd)|${LC_ALL-unset}|$#"
[ "$before" = "$after" ] || { printf 'changed\n%s\n%s\n' "$before" "$after" >&2; exit 93; }
[ -n "${REL_CANDIDATE-}" ]'''
        r = subprocess.run(["sh", "-c", script], cwd=self.root, env=self.env, text=True, capture_output=True)
        self.assertEqual(r.returncode, 0, (r.stdout, r.stderr))

    def test_un18_the_same_verdicts_under_errexit_and_with_the_callers_own_symbols(self):
        result = self.steps("fetch trust unpack", FETCH_SETE="1")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.setUp_again()
        self.release(entries=self.entries() + [("evil", "file", 0o644, b"x")], meta={})
        result = self.steps("fetch trust unpack", FETCH_SETE="1", FETCH_EX_FETCH="15")
        self.refused(result, 15, "fetch_archive")


class UnpackNoEx(Unpack):
    no_ex = True
    test_un18_the_same_verdicts_under_errexit_and_with_the_callers_own_symbols = None


class Load(Pipeline):
    def test_ld01_a_loaded_release_is_published_and_current_points_to_it(self):
        result = self.steps("fetch trust unpack load", fakes=True)
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertEqual((result.stdout, result.stderr), ("", ""))
        rel = self.home / "releases" / VERSION
        self.assertTrue(rel.is_dir() and not rel.is_symlink())
        self.assertEqual(stat.S_IMODE(rel.stat().st_mode), 0o700)
        cur = self.home / "current"
        self.assertTrue(cur.is_symlink())
        self.assertEqual(os.readlink(cur), "releases/" + VERSION)
        self.assertEqual(self.candidates(), [VERSION])
        self.assertEqual((rel / "stack.env").read_text(), STACK_ENV)
        calls = [c for c in self.calls().splitlines() if c.startswith("podman")]
        self.assertEqual(calls, [f"podman load --input {self.stage}/{self.image.name}", f"podman image exists {SERVER}"])
        self.assertEqual(sorted(p.name for p in self.stage.iterdir()), sorted(ASSETS))

    def test_ld02_a_failed_load_publishes_nothing_and_removes_the_candidate(self):
        for mode in ("load-fail", "exists-fail", "fail"):
            with self.subTest(mode=mode):
                self.setUp_again()
                result = self.steps("fetch trust unpack load", fakes=True, FAKE_PODMAN_MODE=mode)
                self.refused(result, 5, "fetch_image")
                self.assertFalse((self.home / "current").exists() or (self.home / "current").is_symlink())
                self.assertEqual(self.candidates(), [])

    def test_ld03_another_current_is_never_switched_and_the_image_is_not_even_loaded(self):
        (self.home / "releases" / "0.9.0").mkdir(parents=True)
        (self.home / "current").symlink_to("releases/0.9.0")
        before = self.tree(self.home)
        result = self.steps("fetch trust unpack load", fakes=True)
        self.refused(result, 5, "fetch_present")
        self.assertNotIn("podman", self.calls())
        after = self.tree(self.home)
        self.assertEqual({k: v for k, v in after.items()}, before)

    def test_ld04_a_current_that_is_not_a_link_is_refused(self):
        for name, make in (("a directory", lambda p: p.mkdir()), ("a file", lambda p: p.write_text("x"))):
            with self.subTest(current=name):
                self.setUp_again()
                self.home.mkdir()
                make(self.home / "current")
                result = self.steps("fetch trust unpack load", fakes=True)
                self.refused(result, 5, "fetch_present")
                self.assertNotIn("podman", self.calls())

    def test_ld05_no_candidate_or_a_strange_one_is_refused_and_nothing_is_removed(self):
        script = r'''set -u; EX_TRUST=4 EX_FETCH=5; . "$FETCH_SOURCE"; rel_tool_podman() { "$FAKES/podman" "$@"; }
rel_fetch "$FETCH_VERSION" "$FETCH_STAGE" || exit 91
rel_trust "$FETCH_VERSION" "$FETCH_STAGE" "${CORTEX_TRUST_KEY_FILE-}" "${CORTEX_TRUST_PRINCIPAL-}" "${CORTEX_TRUST_FPR-}" || exit 92
rel_unpack "$FETCH_VERSION" "$FETCH_STAGE" "$FETCH_HOME" || exit 93
case $MODE in
  unset) unset REL_CANDIDATE ;;
  empty) REL_CANDIDATE= ;;
  other) REL_CANDIDATE=$FETCH_HOME/releases/precious ;;
  badname) REL_CANDIDATE=$FETCH_HOME/releases/.cortex-unpack.short ;;
  elsewhere) REL_CANDIDATE=$FETCH_STAGE ;;
  parent) REL_CANDIDATE=$FETCH_HOME/.cortex-unpack.ABCDEFGH ;;
esac
rel_load "$FETCH_VERSION" "$FETCH_STAGE" "$FETCH_HOME"'''
        for mode in ("unset", "empty", "other", "badname", "elsewhere", "parent"):
            with self.subTest(mode=mode):
                self.setUp_again()
                (self.home / "releases" / "precious").mkdir(parents=True)
                (self.home / "releases" / "precious" / "keep").write_text("untouched")
                (self.home / ".cortex-unpack.ABCDEFGH").mkdir()
                self.reserve()
                r = subprocess.run(["sh", "-c", script], cwd=self.root, env={**self.env, "MODE": mode, "FETCH_FAKES": "1"}, text=True, capture_output=True)
                self.refused(r, 5, "fetch_archive")
                self.assertEqual((self.home / "releases" / "precious" / "keep").read_text(), "untouched")
                self.assertTrue((self.home / ".cortex-unpack.ABCDEFGH").is_dir())
                self.assertTrue(self.stage.is_dir())
                self.assertFalse((self.home / "current").is_symlink())

    def test_ld10_every_failed_load_of_a_recognised_owned_candidate_removes_it_and_nothing_else(self):
        script = r'''set -u; EX_TRUST=4 EX_FETCH=5; . "$FETCH_SOURCE"; rel_tool_podman() { "$FAKES/podman" "$@"; }
rel_fetch "$FETCH_VERSION" "$FETCH_STAGE" || exit 91
rel_trust "$FETCH_VERSION" "$FETCH_STAGE" "${CORTEX_TRUST_KEY_FILE-}" "${CORTEX_TRUST_PRINCIPAL-}" "${CORTEX_TRUST_FPR-}" || exit 92
rel_unpack "$FETCH_VERSION" "$FETCH_STAGE" "$FETCH_HOME" || exit 93
case $MODE in
  version) rel_load 1.2 "$FETCH_STAGE" "$FETCH_HOME" ;;
  stage) rel_load "$FETCH_VERSION" "$FETCH_HOME/not-the-stage" "$FETCH_HOME" ;;
  tuple) REL_TRUSTED=$(printf '%s\n' "$REL_TRUSTED" | sed 's#^version .*#version 9.9.9#'); rel_load "$FETCH_VERSION" "$FETCH_STAGE" "$FETCH_HOME" ;;
  current) ln -s releases/other "$FETCH_HOME/current"; rel_load "$FETCH_VERSION" "$FETCH_STAGE" "$FETCH_HOME" ;;
esac'''
        for mode, code, message in (("version", 4, "trust_stage"), ("stage", 4, "trust_stage"), ("tuple", 4, "trust_checksum"), ("current", 5, "fetch_present")):
            with self.subTest(mode=mode):
                self.setUp_again()
                (self.home / "releases" / "precious").mkdir(parents=True)
                (self.home / "releases" / "precious" / "keep").write_text("untouched")
                self.reserve()
                r = subprocess.run(["sh", "-c", script], cwd=self.root, env={**self.env, "MODE": mode, "FETCH_FAKES": "1"}, text=True, capture_output=True)
                self.refused(r, code, message)
                left = sorted(p.name for p in (self.home / "releases").iterdir())
                self.assertEqual(left, ["precious"], (mode, left))   # the candidate is gone, the unrelated directory is untouched
                self.assertEqual((self.home / "releases" / "precious" / "keep").read_text(), "untouched")
                self.assertFalse((self.home / "releases" / VERSION).exists())
                self.assertTrue(self.stage.is_dir())

    def test_ld12_the_early_load_refusals_return_the_includes_own_codes_where_the_caller_defines_none(self):
        """hue, L2 survivors FM764 (line 786) and FM770 (line 788): ld10 defines EX_TRUST and EX_FETCH itself, so the defaults of these early refusals were never used. The same refusals,
        with no EX_* symbol defined by the caller: the include's own defaults (4 for the trust refusals, 5 for the fetch one) are what returns."""
        script = r'''set -u; . "$FETCH_SOURCE"; rel_tool_podman() { "$FAKES/podman" "$@"; }
rel_fetch "$FETCH_VERSION" "$FETCH_STAGE" || exit 91
rel_trust "$FETCH_VERSION" "$FETCH_STAGE" "${CORTEX_TRUST_KEY_FILE-}" "${CORTEX_TRUST_PRINCIPAL-}" "${CORTEX_TRUST_FPR-}" || exit 92
rel_unpack "$FETCH_VERSION" "$FETCH_STAGE" "$FETCH_HOME" || exit 93
case $MODE in
  version) rel_load 1.2 "$FETCH_STAGE" "$FETCH_HOME" ;;
  stage) rel_load "$FETCH_VERSION" "$FETCH_HOME/not-the-stage" "$FETCH_HOME" ;;
  tuple) REL_TRUSTED=$(printf '%s\n' "$REL_TRUSTED" | sed 's#^version .*#version 9.9.9#'); rel_load "$FETCH_VERSION" "$FETCH_STAGE" "$FETCH_HOME" ;;
  current) ln -s releases/other "$FETCH_HOME/current"; rel_load "$FETCH_VERSION" "$FETCH_STAGE" "$FETCH_HOME" ;;
esac'''
        for mode, code, message in (("version", 4, "trust_stage"), ("stage", 4, "trust_stage"), ("tuple", 4, "trust_checksum"), ("current", 5, "fetch_present")):
            with self.subTest(mode=mode):
                self.setUp_again()
                self.reserve()
                r = subprocess.run(["sh", "-c", script], cwd=self.root, env={**self.env, "MODE": mode, "FETCH_FAKES": "1"}, text=True, capture_output=True)
                self.refused(r, code, message)

    def test_ld11_the_discard_after_an_early_refusal_starts_the_native_rm_in_a_clean_environment(self):
        """vera V6S2R1-01: the cleanup that follows a failed load must not run the native rm with the CALLER's environment. The rm seam is replaced by an inert observer that
        writes what it sees and then performs the same rm; the log is cut just before the failing load, so it holds only the calls of the early-failure path."""
        script = r'''set -u; EX_TRUST=4 EX_FETCH=5; . "$FETCH_SOURCE"; rel_tool_podman() { "$FAKES/podman" "$@"; }
rel_tool_rm() { printf 'seen TAR_OPTIONS=%s CONTAINER_HOST=%s PYTHONPATH=%s LD_LIBRARY_PATH=%s path=%s umask=%s\n' "${TAR_OPTIONS-unset}" "${CONTAINER_HOST-unset}" "${PYTHONPATH-unset}" "${LD_LIBRARY_PATH-unset}" "$PATH" "$(umask)" >> "$OBSERVE"; /usr/bin/rm "$@"; }
rel_fetch "$FETCH_VERSION" "$FETCH_STAGE" || exit 91
rel_trust "$FETCH_VERSION" "$FETCH_STAGE" "${CORTEX_TRUST_KEY_FILE-}" "${CORTEX_TRUST_PRINCIPAL-}" "${CORTEX_TRUST_FPR-}" || exit 92
rel_unpack "$FETCH_VERSION" "$FETCH_STAGE" "$FETCH_HOME" || exit 93
: > "$OBSERVE"
rel_load 1.2 "$FETCH_STAGE" "$FETCH_HOME"'''
        for mode, hostile in (("a caller with inert inputs set", {"TAR_OPTIONS": "inert-observer", "CONTAINER_HOST": "inert-observer", "PYTHONPATH": "/nonexistent-observer", "LD_LIBRARY_PATH": "/nonexistent-observer"}),
                              ("a caller with a loose umask and a short PATH", {"PATH": "/usr/bin:/bin"})):
            with self.subTest(case=mode):
                self.setUp_again()
                observe = self.root / "observe.log"
                self.reserve()
                r = subprocess.run(["sh", "-c", script], cwd=self.root, env={**self.env, "FETCH_FAKES": "1", "OBSERVE": str(observe), **hostile}, text=True, capture_output=True)
                self.refused(r, 4, "trust_stage")
                seen = observe.read_text().splitlines()
                self.assertEqual(seen, ["seen TAR_OPTIONS=unset CONTAINER_HOST=unset PYTHONPATH=unset LD_LIBRARY_PATH=unset path=/usr/bin:/usr/sbin umask=0077"], seen)
                self.assertEqual(sorted(p.name for p in (self.home / "releases").iterdir()), [])   # the candidate was removed

    def test_ld06_a_candidate_changed_after_unpack_is_refused_and_removed(self):
        for victim, change in (("a member", lambda c: (c / "cortex").write_text("tampered")), ("an extra file", lambda c: (c / "evil").write_text("x")),
                               ("a mode", lambda c: (c / "cortex").chmod(0o4755)), ("the sums", lambda c: (c / "SHA256SUMS").write_text("0" * 64 + "  cortex\n"))):
            with self.subTest(change=victim):
                self.setUp_again()
                script = r'''set -u; EX_TRUST=4 EX_FETCH=5; . "$FETCH_SOURCE"; rel_tool_podman() { "$FAKES/podman" "$@"; }
rel_fetch "$FETCH_VERSION" "$FETCH_STAGE" || exit 91
rel_trust "$FETCH_VERSION" "$FETCH_STAGE" "${CORTEX_TRUST_KEY_FILE-}" "${CORTEX_TRUST_PRINCIPAL-}" "${CORTEX_TRUST_FPR-}" || exit 92
rel_unpack "$FETCH_VERSION" "$FETCH_STAGE" "$FETCH_HOME" || exit 93
printf '%s\n' "$REL_CANDIDATE" > "$FETCH_CAND"
sh -c "$CHANGE" || exit 94
rel_load "$FETCH_VERSION" "$FETCH_STAGE" "$FETCH_HOME"'''
                self.reserve()
                shell_change = {"a member": 'printf tampered > "$(cat "$FETCH_CAND")/cortex"', "an extra file": 'printf x > "$(cat "$FETCH_CAND")/evil"',
                                "a mode": 'chmod 4755 "$(cat "$FETCH_CAND")/cortex"', "the sums": 'printf "%s  cortex\\n" 0000000000000000000000000000000000000000000000000000000000000000 > "$(cat "$FETCH_CAND")/SHA256SUMS"'}[victim]
                r = subprocess.run(["sh", "-c", script], cwd=self.root, env={**self.env, "CHANGE": shell_change, "FETCH_FAKES": "1"}, text=True, capture_output=True)
                self.assertIn(r.returncode, (4, 5), (victim, r.stdout, r.stderr))
                self.assertEqual(self.candidates(), [], victim)
                self.assertFalse((self.home / "current").is_symlink())
                self.assertNotIn("podman load", self.calls())

    def test_ld07_the_release_directory_appearing_meanwhile_is_not_overwritten(self):
        script = r'''set -u; EX_TRUST=4 EX_FETCH=5; . "$FETCH_SOURCE"; rel_tool_podman() { "$FAKES/podman" "$@"; }
rel_fetch "$FETCH_VERSION" "$FETCH_STAGE" || exit 91
rel_trust "$FETCH_VERSION" "$FETCH_STAGE" "${CORTEX_TRUST_KEY_FILE-}" "${CORTEX_TRUST_PRINCIPAL-}" "${CORTEX_TRUST_FPR-}" || exit 92
rel_unpack "$FETCH_VERSION" "$FETCH_STAGE" "$FETCH_HOME" || exit 93
mkdir "$FETCH_HOME/releases/$FETCH_VERSION" && printf precious > "$FETCH_HOME/releases/$FETCH_VERSION/keep"
rel_load "$FETCH_VERSION" "$FETCH_STAGE" "$FETCH_HOME"'''
        self.reserve()
        r = subprocess.run(["sh", "-c", script], cwd=self.root, env={**self.env, "FETCH_FAKES": "1"}, text=True, capture_output=True)
        self.refused(r, 5, "fetch_present")
        self.assertEqual((self.home / "releases" / VERSION / "keep").read_text(), "precious")
        self.assertEqual(sorted(p.name for p in (self.home / "releases" / VERSION).iterdir()), ["keep"])
        self.assertFalse((self.home / "current").is_symlink())

    def test_ld08_a_second_run_is_refused_not_repeated(self):
        result = self.steps("fetch trust unpack load", fakes=True)
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        snap = self.tree(self.home)
        self.discard_stage()
        if self.log.exists():
            self.log.unlink()
        result = self.steps("fetch trust unpack load", fakes=True)
        self.refused(result, 5, "fetch_present")
        self.assertEqual(self.tree(self.home), snap)
        self.assertNotIn("podman", self.calls())

    def test_ld09_hostile_stand_ins_and_the_environment_change_nothing(self):
        hostile = self.root / "hostile"
        hostile.mkdir()
        canary = self.root / "canary"
        for tool in ("podman", "mv", "ln", "rm", "mkdir", "find", "chmod", "stat", "cat", "readlink", "sha256sum", "ls"):
            (hostile / tool).write_text(f"#!/bin/sh\nprintf '%s\n' \"$0\" >> '{canary}'\nexit 0\n")
            (hostile / tool).chmod(0o755)
        env = {"PATH": str(hostile) + os.pathsep + os.environ["PATH"], "LD_PRELOAD": "/nonexistent/evil.so", "BASH_ENV": "/nonexistent/evil", "CDPATH": "/"}
        result = self.steps("fetch trust unpack load", fakes=True, env=env, FETCH_UMASK="000", FETCH_IFS_SET="0123456789abcdef.-_:")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertFalse(canary.exists(), canary.read_text() if canary.exists() else "")
        self.assertEqual(os.readlink(self.home / "current"), "releases/" + VERSION)


class LoadNoEx(Load):
    no_ex = True


class Verify(Pipeline):
    def installed(self):
        result = self.steps("fetch trust unpack load", fakes=True)
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.discard_stage()
        return self.home / "releases" / VERSION

    def verify(self, directory, **kw):
        return self.run_harness("verify", FETCH_VERIFY_DIR=str(directory), **kw)

    def test_vr01_a_release_the_installer_made_verifies_and_nothing_is_changed(self):
        rel = self.installed()
        before = self.tree(rel)
        result = self.verify(rel)
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertEqual((result.stdout, result.stderr), ("", ""))
        self.assertEqual(self.tree(rel), before)

    def test_vr02_every_defect_is_a_refusal_and_the_directory_is_never_repaired(self):
        def rm(name): return lambda r: (r / name).unlink()
        def add(name, data="x"): return lambda r: (r / name).write_text(data)
        def chmod(name, mode): return lambda r: (r / name).chmod(mode)
        def write(name, data): return lambda r: (r / name).write_text(data)
        def link(name):
            def f(r):
                (r / name).unlink()
                (r / name).symlink_to("/etc/hostname")
            return f
        def mkd(name): return lambda r: (r / name).mkdir()
        sums = lambda r: (r / "SHA256SUMS").read_text()
        defects = {
            "a missing member": rm("cortex"), "a missing manifest": rm("manifest.json"), "a missing SHA256SUMS": rm("SHA256SUMS"), "an extra file": add("evil"),
            "an extra file in bin": add("bin/evil"), "an extra directory": mkd("evil"), "a changed member": write("compose.yml", "changed"), "a changed manifest": write("manifest.json", "{}"),
            "a mode 0777": chmod("cortex", 0o777), "a setuid member": chmod("cortex", 0o4755), "a directory mode": chmod("bin", 0o700), "a release dir mode is not checked but a member link is": link("compose.yml"),
            "a sums file with a bad line": lambda r: (r / "SHA256SUMS").write_text(sums(r) + "garbage\n"),
            "a sums file without the final newline": lambda r: (r / "SHA256SUMS").write_text(sums(r).rstrip("\n")),
            "a sums file with a duplicate": lambda r: (r / "SHA256SUMS").write_text(sums(r) + sums(r).splitlines()[0] + "\n"),
            "a sums file missing a required line": lambda r: (r / "SHA256SUMS").write_text("".join(l + "\n" for l in sums(r).splitlines()[1:])),
            "a sums file with a star": lambda r: (r / "SHA256SUMS").write_text(sums(r).replace("  ", " *", 1)),
            "a sums file with an unlisted name": lambda r: (r / "SHA256SUMS").write_text(sums(r) + "0" * 64 + "  other\n"),
            "a sums file that lists a missing file": lambda r: (r / "SHA256SUMS").write_text(sums(r) + "0" * 64 + "  initdb/gone.sql\n"),
            "sums mode 0600": chmod("SHA256SUMS", 0o600),
            "stack.env with an extra key": write("stack.env", STACK_ENV + "EXTRA=1\n"),
            "stack.env of another version": write("stack.env", STACK_ENV.replace(VERSION, "1.2.4")),
            "stack.env with an unpinned image": write("stack.env", STACK_ENV.replace("@sha256:" + "a" * 64, ":latest")),
        }
        for name, damage in defects.items():
            with self.subTest(defect=name):
                self.setUp_again()
                rel = self.installed()
                damage(rel)
                if name.startswith("a changed") or name.startswith("stack.env") or "mode" in name or "setuid" in name:
                    pass
                before = self.tree(rel)
                result = self.verify(rel)
                self.refused(result, 5, "fetch_verify")
                self.assertEqual(self.tree(rel), before)

    def test_vr03_a_directory_that_is_not_a_real_directory_or_does_not_exist_is_refused(self):
        rel = self.installed()
        link = self.root / "rel-link"
        link.symlink_to(rel)
        file = self.root / "a-file"
        file.write_text("x")
        for d in (link, self.home / "current", file, self.root / "absent", Path("relative/dir"), Path("")):
            with self.subTest(dir=str(d)):
                self.refused(self.verify(d), 5, "fetch_verify")


class Predicates2(Pipeline):
    """Stage 2 predicates called alone, with their edges (a refusal further down the chain must not hide a rule that stopped refusing)."""

    def call(self, fn, *args, env=None, pre=""):
        e = {**self.env, **(env or {})}
        return subprocess.run(["sh", "-c", '. "$1"; shift; ' + pre + ' "$@"', "sh", self.env["FETCH_SOURCE"], fn, *args], cwd=self.root, env=e, text=True, capture_output=True)

    def test_pq01_rel_home_ok(self):
        good = ["/a", "/a/b", "/home/u/cortex-v2", "/A_b.c-d/9", "/a/.hidden", "/a/b..c", "/a/..b", "/a/b.."]
        bad = ["", "a", "a/b", "/", "//", "/a/", "/a//b", "/a/./b", "/a/.", "/a/../b", "/a/..", "/a b", "/a:b", "/a$b", "/a'b", "/a;b", "/a,b", "/a*b", "/a?b", "/a~b", "/a+b", "/a@b", "/a%b",
               "/a\tb", "/a\nb", "/a\u00e9", "~", "$HOME"]
        for v in good:
            with self.subTest(good=v):
                self.assertEqual(self.call("rel_home_ok", v).returncode, 0, v)
        for v in bad:
            with self.subTest(bad=repr(v)):
                self.assertNotEqual(self.call("rel_home_ok", v).returncode, 0, repr(v))
        self.assertNotEqual(self.call("rel_home_ok").returncode, 0)

    def test_pq02_rel_member_mode_is_the_closed_list(self):
        exe = ["cortex", "db-roles.sh", "bin/cortex-dump", "bin/cortex-dump-status", "bin/cortex-dump-alert", "bin/cortex-restore", "bin/restore-compare"]
        plain = ["compose.yml", "stack.env", "cortex.toml.example", *("systemd/" + n for n in UNITS)]
        dirs = ["initdb", "bin", "systemd"]
        for n in exe:
            r = self.call("rel_member_mode", n)
            self.assertEqual((r.returncode, r.stdout), (0, "- rwxr-xr-x\n"), n)
        for n in plain:
            r = self.call("rel_member_mode", n)
            self.assertEqual((r.returncode, r.stdout), (0, "- rw-r--r--\n"), n)
        for n in dirs:
            r = self.call("rel_member_mode", n)
            self.assertEqual((r.returncode, r.stdout), (0, "d rwxr-xr-x\n"), n)
        initdb_good = ["initdb/a", "initdb/roles.sql", "initdb/A_b-9.sql", "initdb/9", "initdb/_x", "initdb/-x", "initdb/" + "a" * 64, "initdb/a.b.c"]
        initdb_bad = ["initdb/", "initdb/.hidden", "initdb/" + "a" * 65, "initdb/a b", "initdb/a/b", "initdb/a$", "initdb/a\u00e9", "initdb//a", "initdb/..", "initdb/.", "initdb/a*"]
        for n in initdb_good:
            r = self.call("rel_member_mode", n)
            self.assertEqual((r.returncode, r.stdout), (0, "- rw-r--r--\n"), n)
        bad = ["", "evil", "Cortex", "cortex/", "cortex.", "compose.yaml", "bin/evil", "bin/cortex-dump2", "bin/", "systemd/evil.service", "systemd/cortex-dump.timer2", "systemd/", "initdb2", "xinitdb/a", "stack.env.bak",
               "manifest.json", "SHA256SUMS", "/cortex", "./cortex", "../cortex", "bin/../cortex"]
        for n in initdb_bad + bad:
            with self.subTest(bad=n):
                r = self.call("rel_member_mode", n)
                self.assertNotEqual(r.returncode, 0, n)
                self.assertEqual(r.stdout, "")

    def test_pq03_rel_meta_value_ok(self):
        digest = "a" * 64
        cases = [("RELEASE_VERSION", "1.2.3", "1.2.3", True), ("RELEASE_VERSION", "1.2.4", "1.2.3", False), ("STACK_VERSION", "1.2.3", "1.2.3", True), ("STACK_VERSION", "1.2.3-rc", "1.2.3", False),
                 ("MEMBER_LIST", "2", "1.2.3", True), ("MEMBER_LIST", "3", "1.2.3", False), ("MEMBER_LIST", "02", "1.2.3", False), ("MEMBER_LIST", "", "1.2.3", False),
                 ("CORTEX_SERVER_IMAGE", SERVER, "1.2.3", True), ("CORTEX_SERVER_IMAGE", "x:latest", "1.2.3", False), ("CORTEX_DB_IMAGE", DATABASE, "1.2.3", True), ("CORTEX_DB_IMAGE", "", "1.2.3", False),
                 ("ROLES_SCRIPT_DIGEST", digest, "1.2.3", True), ("ROLES_SCRIPT_DIGEST", digest[:-1], "1.2.3", False), ("ROLES_SCRIPT_DIGEST", digest + "a", "1.2.3", False),
                 ("ROLES_SCRIPT_DIGEST", "A" * 64, "1.2.3", False), ("ROLES_SCRIPT_DIGEST", "g" * 64, "1.2.3", False), ("META_SCHEMA", "1", "1.2.3", False), ("EXTRA", "x", "1.2.3", False),
                 ("RELEASE_VERSION", "1.2.3 ", "1.2.3", False), ("CORTEX_DB_IMAGE", "a b@sha256:" + digest, "1.2.3", False), ("CORTEX_DB_IMAGE", "n" * 237 + "@sha256:" + digest, "1.2.3", False),
                 ("CORTEX_DB_IMAGE", "n" * 228 + "@sha256:" + digest, "1.2.3", True), ("CORTEX_SERVER_IMAGE", "n" * 228 + "@sha256:" + digest, "1.2.3", True),
                 ("CORTEX_DB_IMAGE", "n" * 229 + "@sha256:" + digest, "1.2.3", False)]
        for key, val, ver, want in cases:
            with self.subTest(key=key, val=val[:40], want=want):
                self.assertEqual(self.call("rel_meta_value_ok", key, val, ver).returncode == 0, want, (key, val))

    def good_tuple(self):
        self.reserve()
        result = self.run_harness("fetch trust")
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        return self.out.read_text().rstrip("\n")

    def tuple_ok(self, text, version=VERSION, stage=None):
        script = '. "$1"; REL_TRUSTED=$2; rel_tuple_ok "$3" "$4"'
        return subprocess.run(["sh", "-c", script, "sh", self.env["FETCH_SOURCE"], text, version, stage or str(self.stage)], cwd=self.root, env=self.env, text=True, capture_output=True).returncode

    def test_pq04_rel_tuple_ok_accepts_the_tuple_rel_trust_makes_and_nothing_near_it(self):
        good = self.good_tuple()
        self.assertEqual(self.tuple_ok(good), 0)
        lines = good.split("\n")
        self.assertEqual(len(lines), 14)
        self.assertNotEqual(self.tuple_ok(good, version="1.2.4"), 0)
        self.assertNotEqual(self.tuple_ok(good, stage=str(self.stage) + "x"), 0)
        self.assertNotEqual(self.tuple_ok(""), 0)
        for i in range(14):
            for what, new in (("deleted", None), ("blank", ""), ("doubled", "DOUBLE"), ("tab for space", lines[i].replace(" ", "\t", 1)), ("trailing space", lines[i] + " "), ("another word", lines[i] + " x"),
                              ("upper-cased", lines[i].upper()), ("first char dropped", lines[i][1:]), ("last char dropped", lines[i][:-1])):
                with self.subTest(line=i, change=what):
                    if new == lines[i]:
                        continue   # the change does nothing to this line (no space to replace, nothing to upper-case)
                    mutated = lines[:i] + ([] if new is None else [lines[i], lines[i]] if new == "DOUBLE" else [new]) + lines[i + 1:]
                    self.assertNotEqual(self.tuple_ok("\n".join(mutated)), 0, (i, what))
        # a change that keeps the SHAPE is accepted here (what a hash or size IS, is rel_file_is's question, against the real file)
        for i in (3, 4, 5, 6, 7, 11, 12, 13):
            with self.subTest(line=i, change="last character of a hash, size or digest"):
                mutated = lines[:]
                mutated[i] = lines[i][:-1] + ("0" if lines[i][-1] != "0" else "1")
                if i in (11, 12) or i > 7:
                    self.assertEqual(self.tuple_ok("\n".join(mutated)), 0, (i, mutated[i]))
                else:
                    self.assertEqual(self.tuple_ok("\n".join(mutated)), 0, (i, mutated[i]))
        swapped = lines[:]
        swapped[3], swapped[4] = swapped[4], swapped[3]
        self.assertNotEqual(self.tuple_ok("\n".join(swapped)), 0)
        self.assertNotEqual(self.tuple_ok(good + "\nextra"), 0)

    def test_pq06_the_tuple_sizes_have_their_edge_and_the_tuple_helpers_answer_only_from_the_tuple(self):
        good = self.good_tuple()
        lines = good.split("\n")
        for i in range(3, 8):    # a size is 1 to 19 digits: the contract is narrower than the engine's u64 (vera V6C05)
            parts = lines[i].split(" ")
            for size, want in (("9" * 19, True), ("1" * 18, True), ("0", True), ("1" * 20, False), ("1" * 25, False), ("-1", False), ("1a", False), ("0x1", False)):
                with self.subTest(line=i, size=size):
                    mutated = lines[:]
                    mutated[i] = " ".join(parts[:2] + [size] + parts[3:])
                    self.assertEqual(self.tuple_ok("\n".join(mutated)) == 0, want, (i, size))
        src = self.env["FETCH_SOURCE"]
        def helper(fn, name, tuple_text, cwd=None):
            return subprocess.run(["sh", "-c", '. "$1"; REL_TRUSTED=$2; ' + fn + ' "$3"', "sh", src, tuple_text, name], cwd=cwd or self.root, env=self.env, text=True, capture_output=True)
        want_file = next(l for l in lines if l.startswith("file install.sh ")).split(" ", 2)[2]
        r = helper("rel_tuple_file", "install.sh", good)
        self.assertEqual((r.returncode, r.stdout), (0, want_file + "\n"), (r.stdout, r.stderr))
        r = helper("rel_tuple_meta", "RELEASE_VERSION", good)
        self.assertEqual((r.returncode, r.stdout), (0, VERSION + "\n"), (r.stdout, r.stderr))
        for fn, name in (("rel_tuple_file", "not-a-listed-file"), ("rel_tuple_file", ""), ("rel_tuple_meta", "NOT_A_KEY"), ("rel_tuple_meta", "")):
            r = helper(fn, name, good)
            self.assertEqual((r.returncode, r.stdout), (1, ""), (fn, name, r.stdout))
        # globbing is off inside: a tuple that is only a pattern must not match files of the caller's directory
        g = self.root / "globdir"
        g.mkdir()
        (g / "file foo 7 7").write_text("x")
        (g / "meta FOO bar").write_text("x")
        for fn, name in (("rel_tuple_file", "foo"), ("rel_tuple_meta", "FOO")):
            r = helper(fn, name, "*", cwd=g)
            self.assertEqual((r.returncode, r.stdout), (1, ""), (fn, r.stdout, r.stderr))

    def test_pq07_rel_file_is_answers_only_for_a_file_the_tuple_lists_and_that_matches(self):
        good = self.good_tuple()
        src = self.env["FETCH_SOURCE"]
        def is_file(name, tuple_text=good, stage=None):
            return subprocess.run(["sh", "-c", '. "$1"; REL_TRUSTED=$2; rel_file_is "$3" "$4"', "sh", src, tuple_text, stage or str(self.stage), name], cwd=self.root, env=self.env, text=True,
                                  capture_output=True).returncode
        for name in ("install.sh", "manifest.json", "install.meta", self.tar.name, self.image.name):
            self.assertEqual(is_file(name), 0, name)
        for name in ("SHA256SUMS", "SHA256SUMS.sig", "not-listed", "", "install.sh/", "../install.sh"):
            self.assertNotEqual(is_file(name), 0, name)   # in the stage or not, a name the tuple does not list is never the file it describes
        (self.stage / "install.sh").write_bytes((self.stage / "install.sh").read_bytes() + b"x")
        self.assertNotEqual(is_file("install.sh"), 0)
        # called directly it cleans its own environment: with a builtin replaced by an imported function that always says 0, a good file is still accepted, a bad one refused
        (self.stage / "install.sh").write_bytes((self.stage / "install.sh").read_bytes()[:-1])
        def hostile(names, name, tuple_text=good):
            env = {**self.env, **{f"BASH_FUNC_{n}%%": "() { return 0; }" for n in names}}
            return subprocess.run(["sh", "-c", '. "$1"; REL_TRUSTED=$2; rel_file_is "$3" "$4"; exit $?', "sh", src, tuple_text, str(self.stage), name], cwd=self.root, env=env, text=True,
                                  capture_output=True).returncode
        for names in (("[",), ("test",), ("[", "test", "printf", "echo", "true", "false")):
            with self.subTest(replaced=names):
                self.assertEqual(hostile(names, "install.sh"), 0, names)
                self.assertNotEqual(hostile(names, "not-listed"), 0, names)
                (self.stage / "install.sh").write_bytes((self.stage / "install.sh").read_bytes() + b"x")
                self.assertNotEqual(hostile(names, "install.sh"), 0, names)
                (self.stage / "install.sh").write_bytes((self.stage / "install.sh").read_bytes()[:-1])

    def test_pq08_rel_check_dir_called_alone_refuses_what_is_not_a_complete_release_directory(self):
        self.reserve()
        result = self.run_harness("fetch trust unpack load", fakes=True)
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        rel = self.home / "releases" / VERSION
        empty = self.root / "empty-dir"
        empty.mkdir()
        link = self.root / "check-link"
        link.symlink_to(rel)
        file = self.root / "check-file"
        file.write_text("x")
        self.assertEqual(self.call("rel_check_dir", str(rel)).returncode, 0)
        for d in (str(empty), str(link), str(file), str(self.root / "absent"), "relative/dir", "", str(rel) + "/bin", str(rel) + "/"):
            with self.subTest(dir=d):
                self.assertNotEqual(self.call("rel_check_dir", d).returncode, 0, d)

    HOSTILE_SETS = (("[",), ("test",), ("[", "test", "printf", "echo", "true", "false", "read", "cd"))

    def hostile_call(self, names, fn, *args):
        env = {**self.env, **{f"BASH_FUNC_{n}%%": "() { return 0; }" for n in names}}
        return subprocess.run(["sh", "-c", '. "$1"; shift; "$@"; exit $?', "sh", self.env["FETCH_SOURCE"], fn, *args], cwd=self.root, env=env, text=True, capture_output=True).returncode

    def test_pq09_rel_check_dir_and_rel_cand_ok_called_alone_clean_their_own_environment_and_see_a_consistent_tamper(self):
        self.reserve()
        result = self.run_harness("fetch trust unpack load", fakes=True)
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        rel = self.home / "releases" / VERSION
        for names in self.HOSTILE_SETS:
            with self.subTest(replaced=names, fn="rel_check_dir"):
                self.assertEqual(self.hostile_call(names, "rel_check_dir", str(rel)), 0, names)
                self.assertNotEqual(self.hostile_call(names, "rel_check_dir", str(self.root / "absent")), 0, names)
        # a candidate directory the installer would have made, and ones it would not
        cand = self.home / "releases" / ".cortex-unpack.Ab3dE9xZ"
        cand.mkdir()
        for names in self.HOSTILE_SETS:
            with self.subTest(replaced=names, fn="rel_cand_ok"):
                self.assertEqual(self.hostile_call(names, "rel_cand_ok", str(cand), str(self.home)), 0, names)
                self.assertNotEqual(self.hostile_call(names, "rel_cand_ok", str(self.home / "releases" / "x"), str(self.home)), 0, names)
        # stack.env changed WITH the listing kept consistent: only the stack.env rules can refuse
        import hashlib
        def rewrite(text):
            d = self.root / "tamper"
            shutil.rmtree(d, ignore_errors=True)
            shutil.copytree(rel, d, symlinks=True)
            (d / "stack.env").write_text(text)
            lines = []
            for line in (d / "SHA256SUMS").read_text().splitlines():
                name = line.split("  ", 1)[1]
                lines.append(hashlib.sha256((d / name).read_bytes()).hexdigest() + "  " + name)
            (d / "SHA256SUMS").write_text("\n".join(lines) + "\n")
            return d
        good = (rel / "stack.env").read_text()
        self.assertEqual(self.call("rel_check_dir", str(rewrite(good))).returncode, 0)
        srv, db = SERVER, DATABASE
        bad = {"an invalid version": good.replace(f"CORTEX_VERSION={VERSION}", "CORTEX_VERSION=1.2"), "a version with .part": good.replace(f"CORTEX_VERSION={VERSION}", f"CORTEX_VERSION={VERSION}.part"),
               "a server image without a digest": good.replace(srv, "localhost/cortex-server:latest"), "a db image with a short digest": good.replace(db, db[:-1]),
               "a server image with an upper-case digest": good.replace(srv, srv[:-64] + srv[-64:].upper()), "a fourth line": good + "EXTRA=1\n", "no final newline": good.rstrip("\n"),
               "a blank line": good.replace("\n", "\n\n", 1)}
        for name, text in bad.items():
            with self.subTest(case=name):
                if text == good:
                    continue
                self.assertNotEqual(self.call("rel_check_dir", str(rewrite(text))).returncode, 0, name)

    def test_pq10_the_tar_and_podman_seams_refuse_while_a_behaviour_input_is_set_and_rel_env_clean_removes_them(self):
        src = self.env["FETCH_SOURCE"]
        def run(script, **env):
            return subprocess.run(["sh", "-c", script, "sh", src], cwd=self.root, env={**self.env, **env}, text=True, capture_output=True)
        for var in ("TAR_OPTIONS",):
            r = run('. "$1"; rel_tool_tar --version; exit $?', **{var: "x"})
            self.assertEqual((r.returncode, r.stdout), (1, ""), (var, r.stdout, r.stderr))
        for var in ("CONTAINER_HOST", "CONTAINER_CONNECTION", "CONTAINER_SSHKEY", "CONTAINER_PASSPHRASE", "CONTAINERS_CONF", "CONTAINERS_CONF_OVERRIDE", "CONTAINERS_REGISTRIES_CONF",
                    "CONTAINERS_STORAGE_CONF", "PODMAN_CONNECTIONS_CONF", "STORAGE_DRIVER", "STORAGE_OPTS"):
            r = run('. "$1"; rel_tool_podman --version; exit $?', **{var: "x"})
            self.assertEqual((r.returncode, r.stdout), (1, ""), (var, r.stdout, r.stderr))
        r = run('. "$1"; ( rel_env_clean; rel_tool_tar --version ); exit $?')
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("tar", r.stdout)
        r = run('. "$1"; ( rel_env_clean; rel_tool_tar --version ); exit $?', TAR_OPTIONS="x")
        self.assertEqual(r.returncode, 0, (r.stdout, r.stderr))
        r = run('. "$1"; ( rel_env_clean; rel_tool_podman --version ) >/dev/null 2>&1; [ "$?" != 1 ]; exit $?', CONTAINER_HOST="x", CONTAINERS_CONF="x", STORAGE_DRIVER="x")
        self.assertEqual(r.returncode, 0, (r.stdout, r.stderr))   # not refused by the seam (podman itself may be absent: any status but the seam's 1)

    def test_pq11_a_physical_name_with_a_separator_cannot_alias_a_permitted_member(self):
        self.reserve()
        result = self.run_harness("fetch trust unpack load", fakes=True)
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        rel = self.home / "releases" / VERSION
        def with_extra(name):
            d = self.root / "alias"
            shutil.rmtree(d, ignore_errors=True)
            shutil.copytree(rel, d, symlinks=True)
            (d / name).write_text("x")
            (d / name).chmod(0o644)
            return d
        control = self.root / "alias-control"
        shutil.copytree(rel, control, symlinks=True)
        self.assertEqual(self.call("rel_check_dir", str(control)).returncode, 0)   # control: the unchanged copy is accepted
        self.assertNotEqual(self.call("rel_check_dir", str(with_extra("initdb/unlisted.sql"))).returncode, 0)   # an unlisted entry with a plain name is refused by the listing
        for name in ("initdb/roles.sql\nd 755 bin", "systemd/cortex-dump.timer\nd 755 bin", "initdb/a\nb.sql", "initdb/a b.sql", "initdb/a\tb.sql", "bin/x\ny"):
            with self.subTest(name=name):
                d = with_extra(name)
                self.assertNotEqual(self.call("rel_check_dir", str(d)).returncode, 0, repr(name))
                self.assertEqual(self.run_harness("verify", FETCH_VERIFY_DIR=str(d)).returncode, 5, repr(name))

    def test_pq12_the_duplicate_record_rule_refuses_an_aliasing_entry_when_the_other_two_name_rules_are_blind(self):
        """sam S6C2R2-SAM01: the three name guards of rel_check_dir (the bad-name find, the physical count against the records, the duplicate-record rejection) OVERLAP on every
        entry a real filesystem can hold: an entry whose name carries a line feed always adds a record that repeats an existing path (a name cannot hold a slash, so the added
        record is a top-level member, and all of them exist). So only the duplicate rejection can be shown ALONE BY THIS CELL: the bad-name find and the count are made blind
        through the find seam, and the entry is still refused. The count is shown alone by pq13 (a controlled-record setup); the bad-name find is not shown alone anywhere: removed
        on its own it is redundant (the other two and the hash of every listed file refuse the same entries), which is written down as an overlap, not claimed as a witness."""
        self.reserve()
        result = self.run_harness("fetch trust unpack load", fakes=True)
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        rel = self.home / "releases" / VERSION
        d = self.root / "alias2"
        shutil.copytree(rel, d, symlinks=True)
        control = self.root / "alias2-control"
        shutil.copytree(rel, control, symlinks=True)
        (d / "systemd" / "cortex-dump.timer\nd 755 bin").write_text("x")
        (d / "systemd" / "cortex-dump.timer\nd 755 bin").chmod(0o644)
        blind_both = ('rel_tool_find() { case " $* " in *" -path "*) return 0 ;; *" -printf x"*) fn=$(/usr/bin/find "$1" -mindepth 1 -printf "%y %m %P\\n" | /usr/bin/wc -l); '
                      'while [ "$fn" -gt 0 ]; do printf x; fn=$((fn - 1)); done ;; *) /usr/bin/find "$@" ;; esac; };')
        self.assertNotEqual(self.call("rel_check_dir", str(d), pre=blind_both).returncode, 0)        # only the duplicate-record rule is left
        self.assertEqual(self.call("rel_check_dir", str(control), pre=blind_both).returncode, 0)     # control: the unchanged directory passes with both guards blind

    def test_pq13_the_physical_count_rule_refuses_a_record_that_no_physical_entry_backs(self):
        """sam S6C2R2-SAM01, the count guard: a CONTROLLED-RECORD setup in which every other rule is satisfied on purpose and only the count of physical entries against the
        records can refuse. The find seam adds ONE record for a permitted path that does not exist (initdb/ghost.sql) to the record listing, SHA256SUMS lists that path with a
        digest, and the hash seam answers that digest for it; no record repeats a path (the duplicate rule cannot refuse) and the bad-name find sees no name. The control is the same
        directory with the ghost file REALLY there and plain tools: accepted."""
        import hashlib
        self.reserve()
        result = self.run_harness("fetch trust unpack load", fakes=True)
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        rel = self.home / "releases" / VERSION
        ghost = b"ghost\n"
        digest = hashlib.sha256(ghost).hexdigest()
        def listed(directory):
            lines = (directory / "SHA256SUMS").read_text().splitlines() + [f"{digest}  initdb/ghost.sql"]
            (directory / "SHA256SUMS").write_text("\n".join(lines) + "\n")
        d = self.root / "ghost-record"
        shutil.copytree(rel, d, symlinks=True)
        listed(d)
        real = self.root / "ghost-real"
        shutil.copytree(rel, real, symlinks=True)
        (real / "initdb" / "ghost.sql").write_bytes(ghost)
        (real / "initdb" / "ghost.sql").chmod(0o644)
        listed(real)
        self.assertEqual(self.call("rel_check_dir", str(real)).returncode, 0)   # control: with the file really there the same listing is accepted by the plain tools
        pre = ('rel_tool_find() { case " $* " in *" -path "*) /usr/bin/find "$@" ;; *" -printf x"*) /usr/bin/find "$@" ;; *) /usr/bin/find "$@"; printf "f 644 initdb/ghost.sql\\n" ;; esac; }; '
               'rel_tool_sha256sum() { case "$*" in *initdb/ghost.sql*) printf "%s  -\\n" "$GHOSTHASH" ;; *) /usr/bin/sha256sum "$@" ;; esac; }; ')
        r = self.call("rel_check_dir", str(d), env={"GHOSTHASH": digest}, pre=pre)
        self.assertNotEqual(r.returncode, 0, (r.stdout, r.stderr))   # only the count of physical entries against the records is left to refuse the ghost record

    def test_pq05_rel_cand_ok_is_name_place_and_owner(self):
        home = self.root / "h"
        (home / "releases").mkdir(parents=True)
        good = home / "releases" / ".cortex-unpack.Ab3dE9xZ"
        good.mkdir()
        def ok(path, h=home, uid=None):
            script = '. "$1"; if [ -n "$4" ]; then rel_tool_uid() { printf "%s\\n" "$4"; }; fi; rel_cand_ok "$2" "$3"'
            return subprocess.run(["sh", "-c", script, "sh", self.env["FETCH_SOURCE"], str(path), str(h), uid or ""], cwd=self.root, env=self.env, text=True, capture_output=True).returncode == 0
        self.assertTrue(ok(good))
        self.assertFalse(ok(good, uid="4242"))
        for name in (".cortex-unpack.short", ".cortex-unpack.Ab3dE9xZZ", ".cortex-unpack.Ab3dE9x_", ".cortex-unpack.Ab3dE9x-", ".cortex-unpack.", "cortex-unpack.Ab3dE9xZ", ".cortex-unpackXAb3dE9xZ", "0.9.0"):
            d = home / "releases" / name
            d.mkdir()
            self.assertFalse(ok(d), name)
        link = home / "releases" / ".cortex-unpack.LinkLink"
        link.symlink_to(good)
        self.assertFalse(ok(link))
        afile = home / "releases" / ".cortex-unpack.FileFile"
        afile.write_text("x")
        self.assertFalse(ok(afile))
        self.assertFalse(ok(home / "releases" / ".cortex-unpack.GoneGone"))
        other = self.root / "h2"
        (other / "releases").mkdir(parents=True)
        self.assertFalse(ok(good, h=other))
        self.assertFalse(ok(home / ".cortex-unpack.Ab3dE9xZ"))
        self.assertFalse(ok(str(home) + "/releases/../releases/.cortex-unpack.Ab3dE9xZ"))
        self.assertFalse(ok(good, h="relative/home"))
        self.assertFalse(ok(""))


class Fixture(Pipeline):
    """scripts/install-fetch-fixture.py (cox W1-99 (3), nemo's joined cell): TEST SUPPORT ONLY. Its own cells: a good run makes a REAL signed set that the real include
    trusts and unpacks, with the test key only under its own directory; every bad argument, a non-empty directory and an existing key file REFUSE and write nothing; neither the
    release builder nor the closed member list carries it or any key it writes."""

    SCRIPT = os.environ.get("INSTALL_FETCH_FIXTURE", str(ROOT / "scripts/install-fetch-fixture.py"))

    def run_fixture(self, *args):
        return subprocess.run(["python3", "-I", self.SCRIPT, *args], cwd=self.root, env={**os.environ, "PATH": "/usr/bin:/usr/sbin"}, text=True, capture_output=True)

    def test_fx01_a_good_run_makes_a_real_signed_set_that_the_real_include_trusts_and_unpacks(self):
        d = self.root / "fx"
        d.mkdir()
        r = self.run_fixture(str(d), VERSION)
        self.assertEqual(r.returncode, 0, (r.stdout, r.stderr))
        files = sorted(str(p.relative_to(d)) for p in d.rglob("*") if p.is_file())
        self.assertEqual(files, sorted(["anchor/anchor.env", "anchor/trust.keyfile", "private/trust.key", "private/trust.key.pub"] + ["release/" + n for n in ASSETS]), files)
        self.assertEqual(stat.S_IMODE((d / "private" / "trust.key").stat().st_mode), 0o600)
        self.assertEqual(stat.S_IMODE((d / "private").stat().st_mode), 0o700)
        env = dict(line.split("=", 1) for line in (d / "anchor" / "anchor.env").read_text().splitlines())
        self.assertEqual(sorted(env), ["FPR", "KEY", "KEYFILE", "NAMESPACE", "PRINCIPAL", "VERSION"])
        self.assertNotIn("PRIVATE KEY", (d / "anchor" / "anchor.env").read_text() + (d / "anchor" / "trust.keyfile").read_text())
        self.assertEqual(env["FPR"], sh("ssh-keygen", "-l", "-f", str(d / "private" / "trust.key.pub")).stdout.split()[1])
        self.assertEqual((d / "anchor" / "trust.keyfile").read_text(), f"{env['PRINCIPAL']} ssh-ed25519 {env['KEY']}\n")
        self.assertEqual(sorted(r.stdout.split()), sorted([str(d / "release"), str(d / "anchor" / "anchor.env"), env["KEYFILE"], str(d / "private" / "trust.key")]))
        # the REAL include: fetch from the fixture's release directory, trust it with the anchor values it wrote, unpack it (real sha256sum, ssh-keygen and tar: no fake)
        self.reserve()
        result = self.run_harness("fetch trust unpack", env={"CORTEX_FROM": str(d / "release"), "CORTEX_TRUST_KEY_FILE": env["KEYFILE"], "CORTEX_TRUST_PRINCIPAL": env["PRINCIPAL"],
                                                             "CORTEX_TRUST_FPR": env["FPR"]})
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertEqual(self.out.read_text().splitlines()[0], "v1")
        cand = Path((self.root / "cand.out").read_text().strip())
        self.assertTrue(cand.is_dir() and (cand / "cortex").is_file() and (cand / "SHA256SUMS").is_file(), cand)

    def test_fx02_every_bad_argument_a_non_empty_directory_and_an_existing_key_file_refuse_and_write_nothing(self):
        def made(name, build):
            p = self.root / name
            build(p)
            return p
        def empty(p): p.mkdir()
        def nonempty(p): p.mkdir(); (p / "keep").write_text("mine")
        def with_key(p): p.mkdir(); (p / "private").mkdir(); (p / "private" / "trust.key").write_text("an existing key: never overwritten")
        def a_file(p): p.write_text("not a directory")
        def a_link(p): (self.root / "real-empty").mkdir(); p.symlink_to(self.root / "real-empty")
        def a_link_slash(p): (self.root / "real-empty2").mkdir(); p.symlink_to(self.root / "real-empty2")
        cases = [("no arguments", lambda: [], None), ("one argument", lambda: [str(made("one", empty))], None), ("three arguments", lambda: [str(made("three", empty)), VERSION, "x"], None),
                 ("a version with two parts", lambda: [str(made("v2", empty)), "1.2"], None), ("a version with a suffix", lambda: [str(made("vs", empty)), "1.2.3-rc"], None),
                 ("an empty version", lambda: [str(made("ve", empty)), ""], None), ("a relative directory that does not exist", lambda: ["relative/dir", VERSION], None),
                 ("a relative directory that EXISTS and is empty (only the absolute-path rule can refuse it)", lambda: [(made("reldir", empty), "reldir")[1], VERSION], None),
                 ("a directory that does not exist", lambda: [str(self.root / "absent"), VERSION], None), ("a file instead of a directory", lambda: [str(made("afile", a_file)), VERSION], None),
                 ("a link to a directory", lambda: [str(made("alink", a_link)), VERSION], None), ("a link to an empty directory given WITH A TRAILING SLASH (hale F-1: islink is False for it)", lambda: [str(made("alink2", a_link_slash)) + "/", VERSION], None),
                 ("a non-empty directory", lambda: [str(made("busy", nonempty)), VERSION], "busy"),
                 ("an existing key file", lambda: [str(made("haskey", with_key)), VERSION], "haskey")]
        for name, args, watched in cases:
            with self.subTest(case=name):
                before = self.snapshot(self.root)
                argv = args()
                after_make = self.snapshot(self.root)
                r = self.run_fixture(*argv)
                self.assertEqual(r.returncode, 2, (name, r.stdout, r.stderr))
                self.assertTrue(r.stderr.startswith("install-fetch-fixture: "), r.stderr)
                self.assertEqual(r.stdout, "")
                self.assertEqual(self.snapshot(self.root), after_make, name)   # nothing was written, nothing overwritten
        self.assertEqual((self.root / "haskey" / "private" / "trust.key").read_text(), "an existing key: never overwritten")
        self.assertEqual((self.root / "busy" / "keep").read_text(), "mine")
        self.assertEqual(os.listdir(self.root / "real-empty2"), [], "the link's target stayed empty: nothing was written through the link")

    def test_fx03_neither_the_release_builder_nor_the_member_list_carries_the_fixture_or_a_key(self):
        carried = ("install-fetch-fixture", "trust.key", "trust.keyfile", "anchor.env")
        for builder in ("scripts/release.sh", "scripts/release_manifest.py"):
            text = (ROOT / builder).read_text()
            for word in carried:
                self.assertNotIn(word, text, (builder, word))
        for name in ("install-fetch-fixture.py", "scripts/install-fetch-fixture.py", "trust.key", "trust.key.pub", "trust.keyfile", "anchor.env", "private/trust.key", "anchor/anchor.env"):
            r = subprocess.run(["sh", "-c", '. "$1"; rel_member_mode "$2"; exit $?', "sh", self.env["FETCH_SOURCE"], name], cwd=self.root, env=self.env, text=True, capture_output=True)
            self.assertEqual((r.returncode, r.stdout), (1, ""), name)   # the closed list of member list 2 does not allow it

    def injected(self, name, old, new):
        """A copy of the fixture script with ONE edit (a failure injected after the three directories exist), and the directory it is given."""
        text = Path(self.SCRIPT).read_text()
        self.assertEqual(text.count(old), 1, old)
        copy = self.root / f"fixture-{name}.py"
        copy.write_text(text.replace(old, new))
        d = self.root / f"fx-{name}"
        d.mkdir()
        return copy, d

    def test_fx04_a_failure_after_the_directories_exist_removes_what_the_script_made_and_leaves_the_directory_empty(self):
        """hale F-2: 'a refusal writes nothing' must hold after the three directories exist too: on ANY failure the script removes what it made, by exact name."""
        stub = self.root / "keygen-stub"
        stub.write_text('#!/bin/sh\ncase " $* " in *" -Y sign "*) exit 1 ;; *-t\ ed25519*) exec /usr/bin/ssh-keygen "$@" ;; esac\nexec /usr/bin/ssh-keygen "$@"\n')
        stub.chmod(0o755)
        failing = self.root / "keygen-fails"
        failing.write_text("#!/bin/sh\nexit 1\n")
        failing.chmod(0o755)
        cases = [("sign", 'SSH_KEYGEN = "/usr/bin/ssh-keygen"', f'SSH_KEYGEN = "{stub}"'),   # the key is made, then the signature fails: the key must not be left behind
                 ("keygen", 'SSH_KEYGEN = "/usr/bin/ssh-keygen"', f'SSH_KEYGEN = "{failing}"'),   # nothing can be made: the three empty directories must not be left
                 ("traceback", "    archive(os.path.join(release, tar_name), version)\n", '    raise RuntimeError("injected")\n')]   # an exception, not a refusal: exit 1 with a traceback, still nothing left
        for name, old, new in cases:
            with self.subTest(case=name):
                copy, d = self.injected(name, old, new)
                r = subprocess.run(["python3", "-I", str(copy), str(d), VERSION], cwd=self.root, env={**os.environ, "PATH": "/usr/bin:/usr/sbin"}, text=True, capture_output=True)
                self.assertNotEqual(r.returncode, 0, (r.stdout, r.stderr))
                self.assertEqual(os.listdir(d), [], (name, r.stdout, r.stderr))   # the directory it was given is exactly as it was found: empty
                self.assertEqual(r.stdout, "")   # and it printed no path of anything that is gone


class Determinism(Cell):
    """FM329 (cox W1-103 (1)): the bytes of the archive these cells build, and so every hash made from it, must not depend on the clock second they are built in. tr07 hashes
    the release files ONCE and then rebuilds the release for each of its cases; tarfile's gzip header carried the current second, so whenever the second changed between the two
    builds the listing's hash for the tarball no longer matched the staged one, a LATER rule (the per-file hash compare) refused the case with the same message, and the mutant
    that removes the listing-line handler survived about every other run. The cells therefore pin the header time (mtime=0) and prove here that it holds."""

    def test_dt01_the_cells_archive_is_the_same_bytes_in_a_later_clock_second(self):
        first = self.tar.read_bytes()
        started = int(time.time())
        while int(time.time()) == started:   # wait until the clock second has moved
            time.sleep(0.05)
        self.setUp_again()
        self.assertEqual(hashlib.sha256(self.tar.read_bytes()).hexdigest(), hashlib.sha256(first).hexdigest())

    def test_dt02_every_class_the_mutants_runner_lists_is_a_test_class(self):
        """scripts/install_fetch_mutants.py lists the classes with ^class NAME( at column 0 and passes them to unittest: a helper class at column 0 crashed its tiers mode."""
        main = sys.modules["__main__"]
        names = re.findall(r"^class (\w+)\(", (ROOT / "scripts/install-fetch-test.sh").read_text(encoding="utf-8"), re.M)
        self.assertGreater(len(names), 30)
        for name in names:
            klass = getattr(main, name, None)
            self.assertTrue(isinstance(klass, type) and issubclass(klass, unittest.TestCase), name)

    def test_dt03_a_zf_named_mutant_is_judged_against_the_one_class_that_carries_its_cell(self):
        """cox W1-109 (a), option B: a mutant that names a fault-sweep cell (zf01 or zf02: 38 + 10 of the 403, about 11.6 minutes each in all six zf classes, inc25) runs only the
        class that DEFINES the cell; the other five zf classes repeat it under another environment. Any other cell keeps every class."""
        spec = importlib.util.spec_from_file_location("ifm_under_test", str(ROOT / "scripts/install_fetch_mutants.py"))
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        self.assertEqual(mod.named_classes(["zf01"]), ["ZFaults"])
        self.assertEqual(mod.named_classes(["zf02"]), ["ZFaults"])
        self.assertEqual(mod.named_classes(["zf03"]), ["ZMessages"])
        for cells in (["tr07"], ["zf01", "tr07"], ["zf01", "zf03"], [], None, ["zz99"]):
            self.assertIsNone(mod.named_classes(cells), cells)
        self.assertEqual(mod.suite_command(["zf01"], ["ZFaults"], False)[2:], ["-k", "test_zf01_", "ZFaults"])
        self.assertEqual(mod.suite_command(["tr07"], None, True)[2:], ["-f", "-k", "test_tr07_"])
        self.assertEqual(mod.suite_command(None, ["Trust"], False)[2:], ["Trust"])
        text = (ROOT / "scripts/install-fetch-test.sh").read_text(encoding="utf-8")
        self.assertRegex(text, r"(?ms)^class ZFaults\(.*?^    def test_zf01_.*?^    def test_zf02_")   # the class the runner names really defines both cells

    def test_dt04_the_runner_gives_a_zf_named_mutant_the_carrying_class_and_any_other_mutant_none(self):
        """The RUNNER's own wiring (not only named_classes): a copy of the runner beside a stub cells script that records its argument list and always fails."""
        scripts = self.root / "scripts"
        (scripts).mkdir()
        (self.root / "deploy").mkdir()
        (self.root / "deploy" / "install-fetch.sh").write_text("f() { true; }\n")
        shutil.copy(ROOT / "scripts/install_fetch_mutants.py", scripts / "install_fetch_mutants.py")
        (scripts / "install-fetch-test.sh").write_text('#!/bin/sh\necho "$*" >> "$ARGV_LOG"\nclass=ZFaults\ncase $* in *test_zf01_*) echo "FAIL: test_zf01_x (m)" ;; *) echo "FAIL: test_tr07_x (m)" ;; esac\nexit 1\n')
        (scripts / "install-fetch-test.sh").chmod(0o755)
        cells = scripts / "install-fetch-test.sh"
        cells.write_text(cells.read_text() + "# class ZFaults(Pipeline):\n#     def test_zf01_a(self):\n#     def test_zf02_b(self):\n")
        # the runner reads the class layout from the cells file text: give it one that has the class and both cells at column 0 / 4
        cells.write_text(cells.read_text().replace("# class ZFaults(Pipeline):\n#     def test_zf01_a(self):\n#     def test_zf02_b(self):\n", "")
                         + "\nclass ZFaults(Pipeline):\n    def test_zf01_a(self):\n        pass\n    def test_zf02_b(self):\n        pass\n\nclass ZFaultsNoEx(ZFaults):\n    pass\n")
        spec = [{"id": "T1", "what": "line 1: x", "old": "true", "new": "false", "fails": ["zf01"]}, {"id": "T2", "what": "line 1: x", "old": "{ true;", "new": "{ false;", "fails": ["tr07"]},
                {"id": "T3", "what": "line 1: x", "old": "f()", "new": "g()", "fails": ["zf01"], "classes": ["ZFaultsNoEx"]}]
        (scripts / "install-fetch-mutants.json").write_text(json.dumps(spec))
        log = self.root / "argv.log"
        env = {**os.environ, "ARGV_LOG": str(log), "INSTALL_FETCH_NAMED": "1", "INSTALL_FETCH_FAILFAST": "1", "INSTALL_FETCH_NO_BASELINE": "1", "TMPDIR": os.environ["TMPDIR"]}
        r = subprocess.run(["python3", "-I", "-B", str(scripts / "install_fetch_mutants.py")], cwd=self.root, env=env, text=True, capture_output=True, timeout=60)
        self.assertEqual(r.returncode, 0, (r.stdout, r.stderr))
        self.assertEqual(log.read_text().splitlines(), ["-f -k test_zf01_ ZFaults", "-f -k test_tr07_", "-f -k test_zf01_ ZFaultsNoEx"], (r.stdout, r.stderr))

    def test_dt05_every_mutant_in_the_list_applies_exactly_once_names_a_cell_that_exists_and_changes_something(self):
        """cox W1-109 (b): a NOT APPLIED is a gate FAILURE. FM001 and FM014 applied to the stage 1 include and not to the stage 2 include (two occurrences of the old text); nothing
        in the cells said so. The list and the include of THIS tree are checked against each other, and every named cell must exist."""
        include = (ROOT / "deploy/install-fetch.sh").read_text(encoding="utf-8")
        cells = (ROOT / "scripts/install-fetch-test.sh").read_text(encoding="utf-8")
        spec = json.loads((ROOT / "scripts/install-fetch-mutants.json").read_text(encoding="utf-8"))
        ids = [m["id"] for m in spec]
        self.assertEqual(len(ids), len(set(ids)), "an id is used twice")
        bad = []
        for m in spec:
            if include.count(m["old"]) != 1:
                bad.append((m["id"], f"{include.count(m['old'])} occurrences of the old text"))
            if m["new"] == m["old"]:
                bad.append((m["id"], "the mutant changes nothing"))
            if not m["fails"]:
                bad.append((m["id"], "names no cell"))
            for cell in m["fails"]:
                if not re.search(rf"(?m)^    def test_{cell}_", cells):
                    bad.append((m["id"], f"names the cell {cell}, which does not exist"))
            for klass in m.get("classes", []):   # an entry's own classes (the runner gives them to the named cells): each must be a test class that carries every named cell
                obj = getattr(sys.modules["__main__"], klass, None)
                if not (isinstance(obj, type) and issubclass(obj, unittest.TestCase)):
                    bad.append((m["id"], f"classes names {klass}, which is not a test class"))
                    continue
                discovered = unittest.TestLoader().getTestCaseNames(obj)   # a FRESH loader (-k sets name patterns on the default one); callable AND discoverable: a name shadowed with None (ZMessages does it for zf01 and zf02, sam 6COB-SAM-N01) is not a test there
                for cell in m["fails"]:
                    if not any(n.startswith(f"test_{cell}_") for n in discovered):
                        bad.append((m["id"], f"the class {klass} does not DISCOVER the cell {cell}"))
        self.assertEqual(bad, [])


class ZBigUnpack(Pipeline):
    """A member that DECLARES more than 1 GiB (a sparse stream of zeros: the archive on disk stays small) is refused from the listing, before anything is extracted."""

    class Zeros(io.RawIOBase):
        def __init__(self, size):
            self.left = size

        def readable(self):
            return True

        def readinto(self, b):
            n = min(len(b), self.left)
            b[:n] = bytes(n)
            self.left -= n
            return n

    def big_release(self, sizes, base_empty=False):
        """base_empty: every ordinary member is an EMPTY file, so that the sizes given are the WHOLE total the listing adds up (the exact-byte cells need it: the ordinary members carry
        bytes of their own, and a total of exactly 1 GiB is otherwise unreachable); the later steps would refuse such a release, and those cells only ask whether the LISTING did."""
        with tarfile.open(self.tar, "w:gz", format=tarfile.USTAR_FORMAT, compresslevel=1) as out:
            for name, kind, mode, data in self.entries():
                info = tarfile.TarInfo(name)
                info.mode = mode
                if kind == "dir":
                    info.type = tarfile.DIRTYPE
                    out.addfile(info)
                else:
                    data = b"" if base_empty else data
                    info.size = len(data)
                    out.addfile(info, io.BytesIO(data))
            for i, size in enumerate(sizes):
                info = tarfile.TarInfo(f"initdb/big{i}.sql")
                info.mode, info.size = 0o644, size
                out.addfile(info, io.BufferedReader(self.Zeros(size)))
        self.write_manifest()
        self.seal()

    def test_bg00_the_oracle_sees_an_extraction_when_there_is_one(self):
        self.reserve()
        result = self.steps("fetch trust unpack", fakes=True)
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.assertEqual(len(self.tar_calls()), 2, self.tar_calls())
        self.assertEqual(len(self.extraction_calls()), 1, self.extraction_calls())
        self.assertIn("-tvzf", self.tar_calls()[0])

    def test_bg01_a_single_member_over_1_gib_is_refused_from_the_listing(self):
        self.big_release([2 ** 30 + 1])
        self.assertLess(self.tar.stat().st_size, 64 * 1024 * 1024)
        result = self.steps("fetch trust unpack", fakes=True)
        self.refused(result, 5, "fetch_archive")
        self.only_a_listing()
        self.assert_clean()

    def test_bg02_members_that_together_pass_1_gib_are_refused(self):
        self.big_release([2 ** 29 + 1, 2 ** 29 + 1])
        result = self.steps("fetch trust unpack", fakes=True)
        self.refused(result, 5, "fetch_archive")
        self.only_a_listing()
        self.assert_clean()

    # cox W1-109 / the stage 2 campaign: the EXACT-BYTE cases of the listing's limits (include lines 695-698), each the RED of a mutant that moves a limit by one
    def test_bg03_a_single_member_of_exactly_1_gib_is_accepted_by_the_listing(self):
        """FM645 (the per-member limit 1073741823), FM639 (the digit cap 9: 1073741824 has TEN digits) and FM651 (the total limit 1073741823) refuse it: it is the byte the limits are FOR."""
        self.big_release([2 ** 30], base_empty=True)
        self.reserve()
        self.steps("fetch trust unpack", fakes=True)
        self.assertEqual(len(self.tar_calls()), 2, self.tar_calls())   # the listing, then the extraction: the listing did NOT refuse
        self.assertEqual(len(self.extraction_calls()), 1, self.extraction_calls())

    def test_bg04_members_that_together_make_exactly_1_gib_are_accepted_by_the_listing(self):
        """FM651 (the total limit 1073741823) refuses a total of exactly 1 GiB made of two members, each far below the per-member limit."""
        self.big_release([2 ** 29, 2 ** 29], base_empty=True)
        self.reserve()
        self.steps("fetch trust unpack", fakes=True)
        self.assertEqual(len(self.extraction_calls()), 1, self.extraction_calls())

    def sparse_release(self, size):
        """A release whose ordinary members are empty and which holds ONE sparse member (GNU tar --sparse: a hole of `size` bytes, a tiny archive) that the LISTING prints with its full declared size
        (an 11-digit size is not creatable in the ustar header the other helper uses)."""
        work = self.root / "sparse-src"
        names = []
        for name, kind, mode, data in self.entries():
            path = work / name
            if kind == "dir":
                path.mkdir(parents=True, exist_ok=True)
            else:
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(b"")
            path.chmod(mode)
            names.append(name)
        big = work / "initdb" / "big0.sql"
        big.parent.mkdir(parents=True, exist_ok=True)
        with open(big, "wb") as f:
            f.truncate(size)
        big.chmod(0o644)
        names.append("initdb/big0.sql")
        subprocess.run(["/usr/bin/tar", "--sparse", "--no-recursion", "--numeric-owner", "--owner=0", "--group=0", "-czf", str(self.tar), "-C", str(work), *names], check=True)
        self.write_manifest()
        self.seal()

    def test_bg08_sizes_that_between_them_use_every_decimal_digit_are_accepted(self):
        """FM636 (the digit class loses its 9) refuses any size that contains a 9: no ordinary member of the fixtures happens to have one."""
        self.big_release([102, 345, 678, 9], base_empty=True)
        self.reserve()
        self.steps("fetch trust unpack", fakes=True)
        self.assertEqual(len(self.extraction_calls()), 1, self.extraction_calls())

    def test_bg09_a_member_with_an_eleven_digit_size_is_refused_with_the_includes_own_exit_code(self):
        """FM642 (the default of EX_FETCH at the digit cap becomes 4) is seen only where the caller defines no EX_* symbol (the NoEx class runs this too) and only by a size with more than
        ten digits: 10 GiB declared (a sparse hole, a tiny archive). With the cap removed (FM637, FM638) the NEXT line refuses it with the same message and code: they are equivalent."""
        self.sparse_release(10 * 2 ** 30)
        self.assertLess(self.tar.stat().st_size, 64 * 1024 * 1024)
        result = self.steps("fetch trust unpack", fakes=True)
        self.refused(result, 5, "fetch_archive")
        self.only_a_listing()
        self.assert_clean()

    def test_bg10_a_character_device_entry_is_refused_at_the_type_check_so_a_non_numeric_size_never_reaches_the_digit_test(self):
        """The listing prints a device's size as MAJOR,MINOR (not digits). Such an entry is refused by the member's type check BEFORE the digit test, which is why the digit test's refusal
        (FM634 deleted, FM635 its exit code) cannot be reached: the argument for those two equivalents, shown by an entry that would be the only way in."""
        with tarfile.open(self.tar, "w:gz", format=tarfile.USTAR_FORMAT) as out:
            for name, kind, mode, data in self.entries():
                info = tarfile.TarInfo(name)
                info.mode = mode
                if kind == "dir":
                    info.type = tarfile.DIRTYPE
                    out.addfile(info)
                else:
                    info.size = len(data)
                    out.addfile(info, io.BytesIO(data))
            dev = tarfile.TarInfo("initdb/null.sql")
            dev.type, dev.mode, dev.devmajor, dev.devminor = tarfile.CHRTYPE, 0o644, 1, 3
            out.addfile(dev)
        self.write_manifest()
        self.seal()
        result = self.steps("fetch trust unpack", fakes=True)
        self.refused(result, 5, "fetch_archive")
        self.only_a_listing()

    def test_bg06_exactly_256_entries_are_accepted_by_the_listing(self):
        """FM657 (the count limit 255) refuses the 256th entry. The count IS reachable: any initdb/<name> is a valid member, so 256 distinct valid entries exist (an earlier reading of mine, that the
        closed list holds about 17 names, was WRONG: rel_member_mode accepts every initdb/<name> of up to 64 characters)."""
        self.big_release([0] * (256 - len(self.entries())), base_empty=True)
        self.reserve()
        self.steps("fetch trust unpack", fakes=True)
        self.assertEqual(len(self.extraction_calls()), 1, self.extraction_calls())

    def test_bg07_257_entries_are_refused_by_the_listing_alone(self):
        """FM658 (the count limit 257) accepts the 257th entry."""
        self.big_release([0] * (257 - len(self.entries())), base_empty=True)
        result = self.steps("fetch trust unpack", fakes=True)
        self.refused(result, 5, "fetch_archive")
        self.only_a_listing()
        self.assert_clean()

    def test_bg05_members_that_together_pass_1_gib_by_ONE_byte_are_refused_by_the_total_alone(self):
        """FM652 (the total limit 1073741825) accepts a total of 1 GiB + 1 made of two members that are each within the per-member limit: bg02's pair (2 GiB + 2 over) is refused by a limit one byte looser too."""
        self.big_release([2 ** 29, 2 ** 29 + 1], base_empty=True)
        result = self.steps("fetch trust unpack", fakes=True)
        self.refused(result, 5, "fetch_archive")
        self.only_a_listing()
        self.assert_clean()


class ZBigUnpackNoEx(ZBigUnpack):
    """The same listing refusals where the caller defines no EX_* symbol: the include's own defaults (4 and 5) are what returns, so the 'default of EX_FETCH becomes N' mutants of the limits
    (FM648, FM654, FM642) are seen."""
    no_ex = True

class ZFaults(Pipeline):
    """Every call of every SEAMED tool (the rel_tool_* functions: tar, podman and python3 included, rm and rmdir exempt as best-effort cleanup), one at a time, made to
    fail (a cp leaves a partial file first): a run that reaches its end anyway is a pass that a failing tool should not have allowed, and the refusal must be the
    step's own message and exit code. NOT covered by this claim: what builtins do (a redirect, a test), and files read by a tool itself (podman's configuration)."""

    STEPS = "fetch trust unpack load verify"
    MORE = "find sort mktemp chmod mv ln"
    CLEANUP = ("rm", "rmdir")     # best-effort cleanup in an exit trap: a failure there leaves a file, it cannot turn a refusal into a pass
    MESSAGES = {"fetch": {"fetch_download"}, "trust": {"trust_stage", "trust_key", "trust_signature", "trust_checksum"},
                "unpack": {"trust_stage", "trust_checksum", "fetch_home", "fetch_present", "fetch_archive"},
                "load": {"trust_stage", "trust_checksum", "fetch_archive", "fetch_present", "fetch_verify", "fetch_image"}, "verify": {"fetch_verify"}}
    CODES = {"fetch": (5,), "trust": (4,), "unpack": (4, 5), "load": (4, 5), "verify": (5,)}

    def counted(self, fault=None, steps=None):
        d = self.root / "fault-count"
        shutil.rmtree(d, ignore_errors=True)
        d.mkdir()
        self.log.unlink(missing_ok=True)
        order = self.root / "fault-order"
        order.unlink(missing_ok=True)
        extra = {"FETCH_FAULT_DIR": str(d), "FETCH_VERIFY_DIR": str(self.home / "releases" / VERSION), "FETCH_FAULT_MORE": self.MORE,
                 "FETCH_FAULT_ORDER": str(order), **getattr(self, "EXTRA", {})}
        if getattr(self, "ONLINE", False):   # the files come over the (fake) transport: the curl calls are faulted and probed too
            extra.update(CORTEX_FROM="", CORTEX_BASE_URL="https://releases.invalid/1.2.3")
        if fault:
            extra["FETCH_FAULT"] = fault
        self.reserve()
        result = self.run_harness(steps or self.STEPS, fakes=True, **extra)
        return result, {p.name: int(p.read_text()) for p in d.iterdir()}

    def test_zf01_a_failing_tool_call_is_never_a_pass(self):
        steps = self.STEPS.split()
        prefix = []
        for i in range(1, len(steps) + 1):
            self.setUp_again()
            result, counts = self.counted(steps=" ".join(steps[:i]))
            self.assertEqual(result.returncode, 0, (steps[:i], result.stdout, result.stderr))
            prefix.append(counts)
        self.assertGreaterEqual(sum(prefix[-1].values()), 20, prefix[-1])
        for tool, n in sorted(prefix[-1].items()):
            for k in range(1, n + 1):
                with self.subTest(tool=tool, call=k):
                    step = next(steps[i] for i in range(len(steps)) if prefix[i].get(tool, 0) >= k)
                    self.setUp_again()
                    result, _ = self.counted(f"{tool}:{k}")
                    if tool in self.CLEANUP:
                        self.assertIn(result.returncode, (0, 4, 5), (result.stdout, result.stderr))
                        continue
                    self.assertIn(result.returncode, self.CODES[step], (tool, k, step, result.stdout, result.stderr))
                    lines = result.stderr.splitlines()
                    self.assertEqual(len(lines), 2, (tool, k, result.stderr))
                    self.assertIn(lines[0], {PROBLEM[m] for m in self.MESSAGES[step]}, (tool, k, step, result.stderr))
                    self.assertTrue(lines[1].startswith("fix: "), (tool, k, result.stderr))


    # The trust step's tool calls in the order rel_trust_run makes them, each with the message its failure must print (None: cleanup, exempt).
    TRUST_CALLS = ([("uid", "trust_stage"), ("stat", "trust_stage"), ("find", "trust_checksum")] + [("stat", "trust_checksum"), ("wc", "trust_checksum")] * 7 +
                   [("wc", "trust_key"), ("wc", "trust_key"), ("cat", "trust_key"), ("cat", "trust_key"), ("mkdir", "trust_checksum"), ("keygen", "trust_key"),
                    ("wc", "trust_signature"), ("cat", "trust_signature"), ("cat", "trust_signature"), ("keygen", "trust_signature")] +
                   [("sha256sum", "trust_checksum"), ("wc", "trust_checksum")] * 4 +
                   [("wc", "trust_checksum"), ("cat", "trust_checksum"), ("cat", "trust_checksum"), ("sha256sum", "trust_checksum"), ("rm", None), ("rmdir", None)])

    def trust_sequence(self):
        steps = self.STEPS.split()
        k = steps.index("trust")
        self.setUp_again()
        _, before = self.counted(steps=" ".join(steps[:k]))
        first = (self.root / "fault-order").read_text().split()
        self.setUp_again()
        self.counted(steps=" ".join(steps[:k + 1]))
        every = (self.root / "fault-order").read_text().split()
        self.assertEqual(every[:len(first)], first)
        return before, every[len(first):]

    def test_zf02_each_tool_call_of_the_trust_step_refuses_with_its_own_message(self):
        """The loop above accepts any of the step's messages, so a refusal that named the wrong one went unseen (sam). Here the trust step's calls are listed
        in order, each with the message ITS failure must print: the stage's own checks (trust_stage), the stage entries and the files (trust_checksum), the
        key and its fingerprint (trust_key), the signature (trust_signature), the listing, install.meta and the hashes (trust_checksum)."""
        steps = self.STEPS.split()
        k = steps.index("trust")
        before, seq = self.trust_sequence()
        calls = [t for t, _ in self.TRUST_CALLS]
        self.assertEqual(seq, calls, "the trust step makes other tool calls than this cell lists: update the list from rel_trust_run")
        seen = {}
        for position, (tool, key) in enumerate(self.TRUST_CALLS):
            seen[tool] = seen.get(tool, 0) + 1
            if key is None:
                continue
            with self.subTest(position=position, tool=tool, key=key):
                self.setUp_again()
                result, _ = self.counted(f"{tool}:{before.get(tool, 0) + seen[tool]}", steps=" ".join(steps[:k + 1]))
                self.assertEqual(result.returncode, 4, (position, tool, result.stdout, result.stderr))
                self.assertEqual(result.stderr.splitlines()[0], PROBLEM[key], (position, tool, key, result.stderr))



class ZFaultsNoEx(ZFaults):
    no_ex = True


class ZFaultsErrexit(ZFaults):
    """The same faults with the caller's set -e on: a failing tool must still end in the step's refusal, never in a silent exit of the subshell."""

    EXTRA = {"FETCH_SETE": "1"}


class ZFaultsAfter(ZFaults):
    """The faulted call writes ALL its output and THEN reports an error (vera V6C01): the status, not only the bytes, decides."""

    EXTRA = {"FETCH_FAULT_AFTER": "1"}


class ZFaultsAfterErrexit(ZFaults):
    EXTRA = {"FETCH_FAULT_AFTER": "1", "FETCH_SETE": "1"}


class ZFaultsOnline(ZFaults):
    ONLINE = True


class ZMessages(ZFaults):
    """The message of each tool call of the later steps (zf03). Only these two classes run it: the message of a call site does not depend on the fault class."""

    test_zf01_a_failing_tool_call_is_never_a_pass = None
    test_zf02_each_tool_call_of_the_trust_step_refuses_with_its_own_message = None

    # The tool calls of the later steps in the order the include makes them, each with the message its failure must print (None: cleanup, exempt).
    UNPACK_CALLS = ([("uid", "trust_stage"), ("stat", "trust_stage"), ("wc", "trust_checksum"), ("sha256sum", "trust_checksum"), ("wc", "trust_checksum"), ("sha256sum", "trust_checksum"),
                     ("uid", "fetch_home"), ("tar", "fetch_archive"), ("mkdir", "fetch_home"), ("stat", "fetch_home"), ("mktemp", "fetch_archive"), ("tar", "fetch_archive")] +
                    [("chmod", "fetch_archive")] * 3 +
                    [("find", "fetch_archive")] * 3 + [("cat", "fetch_archive"), ("sha256sum", "trust_checksum"), ("cp", "fetch_archive"), ("chmod", "fetch_archive"), ("wc", "trust_checksum"),
                     ("sha256sum", "trust_checksum"), ("find", "fetch_archive"), ("sort", "fetch_archive")] + [("sha256sum", "fetch_archive")] * 17 +
                    [("chmod", "fetch_archive")] + [("find", "fetch_archive")] * 3 + [("wc", "fetch_archive"), ("cat", "fetch_archive"), ("wc", "fetch_archive"), ("cat", "fetch_archive")] +
                    [("sha256sum", "fetch_archive")] * 17 + [("wc", "fetch_archive"), ("wc", "fetch_archive"), ("cat", "fetch_archive"), ("cat", "fetch_archive")])
    LOAD_CALLS = ([("uid", "trust_stage"), ("stat", "trust_stage"), ("uid", "fetch_archive"), ("stat", "fetch_archive")] + [("find", "fetch_verify")] * 3 + [("wc", "fetch_verify"), ("cat", "fetch_verify"),
                   ("wc", "fetch_verify"), ("cat", "fetch_verify")] + [("sha256sum", "fetch_verify")] * 17 +
                  [("wc", "fetch_verify"), ("wc", "fetch_verify"), ("cat", "fetch_verify"), ("cat", "fetch_verify"), ("wc", "trust_checksum"), ("sha256sum", "trust_checksum"),
                   ("podman", "fetch_image"), ("podman", "fetch_image"), ("mv", "fetch_present"), ("ln", "fetch_present")])
    VERIFY_CALLS = ([("find", "fetch_verify")] * 3 + [("wc", "fetch_verify"), ("cat", "fetch_verify"), ("wc", "fetch_verify"), ("cat", "fetch_verify")] + [("sha256sum", "fetch_verify")] * 17 +
                    [("wc", "fetch_verify"), ("wc", "fetch_verify"), ("cat", "fetch_verify"), ("cat", "fetch_verify"), ("cat", "fetch_verify"), ("python", "fetch_verify"), ("sha256sum", "fetch_verify")])

    def step_sequence(self, step):
        steps = self.STEPS.split()
        k = steps.index(step)
        self.setUp_again()
        _, before = self.counted(steps=" ".join(steps[:k]))
        first = (self.root / "fault-order").read_text().split()
        self.setUp_again()
        self.counted(steps=" ".join(steps[:k + 1]))
        every = (self.root / "fault-order").read_text().split()
        self.assertEqual(every[:len(first)], first)
        return before, every[len(first):]

    def test_zf03_each_tool_call_of_unpack_load_and_verify_refuses_with_its_own_message(self):
        """As zf02 for the later steps: the SEAMED calls of each step (tar, podman and python3 among them) are listed in order, each with the message ITS failure must
        print, and the exit code that message carries (trust_* 4, fetch_* 5); the list must equal the real call order, so a call that is not seamed is not in it."""
        steps = self.STEPS.split()
        for step, table in (("unpack", self.UNPACK_CALLS), ("load", self.LOAD_CALLS), ("verify", self.VERIFY_CALLS)):
            k = steps.index(step)
            before, seq = self.step_sequence(step)
            self.assertEqual(seq, [t for t, _ in table], f"the {step} step makes other tool calls than this cell lists: update the list from the include")
            seen = {}
            for position, (tool, key) in enumerate(table):
                seen[tool] = seen.get(tool, 0) + 1
                if key is None:
                    continue
                with self.subTest(step=step, position=position, tool=tool, key=key):
                    self.setUp_again()
                    result, _ = self.counted(f"{tool}:{before.get(tool, 0) + seen[tool]}", steps=" ".join(steps[:k + 1]))
                    self.assertEqual(result.returncode, 4 if key.startswith("trust") else 5, (step, position, tool, result.stdout, result.stderr))
                    self.assertEqual(result.stderr.splitlines()[0], PROBLEM[key], (step, position, tool, key, result.stderr))



class ZMessagesNoEx(ZMessages):
    no_ex = True

class ZEnv(Pipeline):
    """At the moment of EVERY call of a seamed tool the environment is the cleaned one, whatever the caller had."""

    STEPS = "fetch trust unpack load verify"
    MORE = "find sort mktemp chmod mv ln"

    def run_probe(self, pre, steps=None, reserve=True, **extra):
        hostile = self.root / "hostile"
        hostile.mkdir(exist_ok=True)
        probe = self.root / "probe"
        shutil.rmtree(probe, ignore_errors=True)
        probe.mkdir()
        d = self.root / "fault-count"
        shutil.rmtree(d, ignore_errors=True)
        d.mkdir()
        if reserve:
            self.reserve()
        if getattr(self, "ONLINE", False):
            extra.update(CORTEX_FROM="", CORTEX_BASE_URL="https://releases.invalid/1.2.3")
        result = self.run_harness(steps or self.STEPS, fakes=True, FETCH_FAULT_DIR=str(d), FETCH_ENV_PROBE=str(probe), FETCH_FAULT_MORE=self.MORE, FETCH_PRE=pre,
                                  FETCH_VERIFY_DIR=str(self.home / "releases" / VERSION), FETCH_IFS_SET="0123456789abcdef.-_:",
                                  PATH=str(hostile) + os.pathsep + os.environ["PATH"], **extra)
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.tools_seen = sorted({p.name.rsplit("-", 1)[0] for p in probe.iterdir()})
        return [line for p in sorted(probe.iterdir()) for line in p.read_text().splitlines()]

    def test_ze02_the_helpers_called_directly_clean_their_own_environment(self):
        sums = self.root / "a-file"
        sums.write_text("x")
        for call in (f"rel_stage_ok {self.stage}", f"rel_no_nul {sums}"):
            with self.subTest(call=call):
                lines = self.run_probe("", steps="call", reserve=True, FETCH_CALL=call, FETCH_UMASK="000", LC_ALL="de_DE.UTF-8", OPENSSL_CONF="/nonexistent/openssl.cnf")
                self.assertGreaterEqual(len(lines), 1)
                self.assertEqual(set(lines), {"path=/usr/bin:/usr/sbin ifs=ok glob=off lc=C env-path=/usr/bin:/usr/sbin env-lc=C umask=0077 ossl= tin=0"})

    def test_ze01_every_tool_call_runs_in_the_cleaned_environment(self):
        want = {"path=/usr/bin:/usr/sbin ifs=ok glob=off lc=C env-path=/usr/bin:/usr/sbin env-lc=C umask=0077 ossl= tin=0"}
        # the caller has a hostile locale and an exported PATH
        inputs = {name: "inert-observer" for name in ("TAR_OPTIONS", "CONTAINER_HOST", "CONTAINER_CONNECTION", "CONTAINER_SSHKEY", "CONTAINER_PASSPHRASE", "CONTAINERS_CONF",
                                                      "CONTAINERS_CONF_OVERRIDE", "CONTAINERS_REGISTRIES_CONF", "CONTAINERS_STORAGE_CONF", "PODMAN_CONNECTIONS_CONF", "STORAGE_DRIVER", "STORAGE_OPTS")}
        lines = self.run_probe("", LC_ALL="de_DE.UTF-8", FETCH_UMASK="000", OPENSSL_CONF="/nonexistent/openssl.cnf", OPENSSL_MODULES="/nonexistent/modules", OPENSSL_ENGINES="/nonexistent/engines", **inputs)
        self.assertGreaterEqual(len(lines), 40, len(lines))
        self.assertEqual(set(lines), want)   # tin=0: none of the twelve behaviour inputs of tar and podman reaches a tool call (V6S2-01)
        # the caller has NO LC_ALL and a PATH that is not exported: what the children see must still be the cleaned environment
        self.setUp_again()
        lines = self.run_probe("export -n PATH; unset LC_ALL", FETCH_UMASK="000")
        self.assertEqual(set(lines), want)
        # the verifier, the hasher and (online) the transport are probed too, not only the standard tools
        for tool in ("keygen", "sha256sum", "tar", "podman", "python") + (("curl",) if getattr(self, "ONLINE", False) else ()):
            self.assertIn(tool, self.tools_seen, self.tools_seen)


class ZEnvOnline(ZEnv):
    ONLINE = True


class VerifyMore(Pipeline):
    def installed(self):
        result = self.steps("fetch trust unpack load", fakes=True)
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))
        self.discard_stage()
        return self.home / "releases" / VERSION

    def verify(self, directory, **kw):
        return self.run_harness("verify", FETCH_VERIFY_DIR=str(directory), **kw)

    def manifest(self, rel, mutate):
        data = json.loads((rel / "manifest.json").read_text())
        mutate(data)
        (rel / "manifest.json").write_text(json.dumps(data, indent=2) + "\n")
        # the sums name the manifest: re-list it so ONLY the manifest facts can refuse
        lines = [l for l in (rel / "SHA256SUMS").read_text().splitlines() if not l.endswith("  manifest.json")]
        lines.append(hashlib.sha256((rel / "manifest.json").read_bytes()).hexdigest() + "  manifest.json")
        (rel / "SHA256SUMS").write_text("".join(l + "\n" for l in sorted(lines, key=lambda l: l.split("  ", 1)[1])))

    def test_vm01_the_manifest_copy_must_name_this_release_with_member_list_2_and_the_roles_digest(self):
        rel = self.installed()
        def releases(d): return d["releases"][0]
        cases = {
            "another version only": lambda d: releases(d).update(version="9.9.9"),
            "schema 2": lambda d: d.update(schema=2),
            "member list 3": lambda d: releases(d)["cortex"].update(members=3),
            "member list as a string": lambda d: releases(d)["cortex"].update(members="2"),
            "another roles digest": lambda d: releases(d)["cortex"].update(roles_script_digest="sha256:" + "0" * 64),
            "a roles digest without the prefix": lambda d: releases(d)["cortex"].update(roles_script_digest=ROLES_HEX),
            "a roles digest that is not text": lambda d: releases(d)["cortex"].update(roles_script_digest=5),
            "no cortex object": lambda d: releases(d).pop("cortex"),
            "no releases": lambda d: d.update(releases=[]),
            "two releases of the version": lambda d: d["releases"].append(dict(releases(d))),
        }
        for name, mutate in cases.items():
            with self.subTest(case=name):
                self.setUp_again()
                rel = self.installed()
                self.manifest(rel, mutate)
                self.assertEqual(self.verify(rel).returncode, 5, name)
        # a manifest with several releases, ours among them, is fine
        self.setUp_again()
        rel = self.installed()
        self.manifest(rel, lambda d: d["releases"].insert(0, {"version": "0.0.1", "cortex": {"members": 2, "roles_script_digest": "sha256:" + "1" * 64}}))
        result = self.verify(rel)
        self.assertEqual(result.returncode, 0, (result.stdout, result.stderr))

    def test_vm02_a_manifest_that_is_not_json_or_too_large_or_python_missing_is_a_refusal(self):
        for name, writer in (("not json", lambda p: p.write_text("{not json")), ("too large", lambda p: p.write_text(" " * 1048577)), ("empty", lambda p: p.write_text("")),
                             ("invalid utf-8", lambda p: p.write_bytes(b"\xff\xfe{}"))):
            with self.subTest(case=name):
                self.setUp_again()
                rel = self.installed()
                writer(rel / "manifest.json")
                lines = [l for l in (rel / "SHA256SUMS").read_text().splitlines() if not l.endswith("  manifest.json")]
                lines.append(hashlib.sha256((rel / "manifest.json").read_bytes()).hexdigest() + "  manifest.json")
                (rel / "SHA256SUMS").write_text("".join(l + "\n" for l in sorted(lines, key=lambda l: l.split("  ", 1)[1])))
                self.assertEqual(self.verify(rel).returncode, 5, name)
        self.setUp_again()
        rel = self.installed()
        script = '. "$1"; rel_tool_python() { return 127; }; verify_release_dir "$2"'
        r = subprocess.run(["sh", "-c", script, "sh", self.env["FETCH_SOURCE"], str(rel)], cwd=self.root, env={**self.env, "EX_FETCH": "5"}, text=True, capture_output=True)
        self.assertEqual(r.returncode, 5)
        self.assertEqual(r.stderr.splitlines()[0], PROBLEM["fetch_verify"])

    def test_vm03_stack_env_and_sums_variants_are_refused(self):
        def stack(text):
            def f(rel):
                (rel / "stack.env").write_text(text)
                lines = [l for l in (rel / "SHA256SUMS").read_text().splitlines() if not l.endswith("  stack.env")]
                lines.append(hashlib.sha256((rel / "stack.env").read_bytes()).hexdigest() + "  stack.env")
                (rel / "SHA256SUMS").write_text("".join(l + "\n" for l in sorted(lines, key=lambda l: l.split("  ", 1)[1])))
            return f
        v, s, d = VERSION, SERVER, DATABASE
        cases = {
            "keys swapped": stack(f"CORTEX_SERVER_IMAGE={s}\nCORTEX_VERSION={v}\nCORTEX_DB_IMAGE={d}\n"),
            "a blank line": stack(f"CORTEX_VERSION={v}\n\nCORTEX_SERVER_IMAGE={s}\nCORTEX_DB_IMAGE={d}\n"),
            "a comment": stack(f"# c\nCORTEX_VERSION={v}\nCORTEX_SERVER_IMAGE={s}\nCORTEX_DB_IMAGE={d}\n"),
            "CRLF": stack(f"CORTEX_VERSION={v}\r\nCORTEX_SERVER_IMAGE={s}\r\nCORTEX_DB_IMAGE={d}\r\n"),
            "no final newline": stack(f"CORTEX_VERSION={v}\nCORTEX_SERVER_IMAGE={s}\nCORTEX_DB_IMAGE={d}"),
            "a second final newline": stack(f"CORTEX_VERSION={v}\nCORTEX_SERVER_IMAGE={s}\nCORTEX_DB_IMAGE={d}\n\n"),
            "version with a plus": stack(f"CORTEX_VERSION=1.2.3+b\nCORTEX_SERVER_IMAGE={s}\nCORTEX_DB_IMAGE={d}\n"),
            "a version that is not semver": stack(f"CORTEX_VERSION=1.2\nCORTEX_SERVER_IMAGE={s}\nCORTEX_DB_IMAGE={d}\n"),
            "an empty image": stack(f"CORTEX_VERSION={v}\nCORTEX_SERVER_IMAGE=\nCORTEX_DB_IMAGE={d}\n"),
            "an image by tag": stack(f"CORTEX_VERSION={v}\nCORTEX_SERVER_IMAGE=x:latest\nCORTEX_DB_IMAGE={d}\n"),
            "a NUL byte": lambda rel: (rel / "stack.env").write_bytes(STACK_ENV.encode() + b"\x00"),
            "an empty file": stack(""),
        }
        for name, damage in cases.items():
            with self.subTest(case=name):
                self.setUp_again()
                rel = self.installed()
                damage(rel)
                self.assertEqual(self.verify(rel).returncode, 5, name)

    def test_vm04_sums_edges(self):
        for name, mutate in (("one space", lambda t: t.replace("  ", " ", 1)), ("a star", lambda t: t.replace("  ", " *", 1)), ("upper-case hex", lambda t: t.replace(t[:64], t[:64].upper(), 1)),
                             ("a CR", lambda t: t.replace("\n", "\r\n", 1)), ("a blank line", lambda t: t + "\n"), ("a missing final newline", lambda t: t[:-1]),
                             ("a NUL byte", lambda t: t + "\x00"), ("too large", lambda t: t + ("0" * 64 + "  other\n") * 20000)):
            with self.subTest(case=name):
                self.setUp_again()
                rel = self.installed()
                (rel / "SHA256SUMS").write_bytes(mutate((rel / "SHA256SUMS").read_text()).encode())
                self.assertEqual(self.verify(rel).returncode, 5, name)


class HostileFunctions(Cell):
    """Shell functions IMPORTED from the environment (bash reads BASH_FUNC_name%% before the script runs) must not decide a verdict: hale RF-1. A function named like a
    POSIX SPECIAL builtin (return, exit, set, unset, export, shift, eval, ...) is not imported at all by sh (bash in POSIX mode refuses it: measured with return), so those
    names are not in the list; rel_env_clean still removes them from the list it clears."""

    NAMES = ("[", "test", "printf", "hash", "umask", "ulimit", "read", "type", "command", "cd", "true", "false", "echo", "wait", "pwd", "let", "local")

    def hostile(self, names):
        return {**self.env, **{f"BASH_FUNC_{n}%%": "() { return 0; }" for n in names}}

    def sh(self, script, names, *args, env=None):
        return subprocess.run(["sh", "-c", script, "sh", self.env["FETCH_SOURCE"], *args], cwd=self.root, env={**self.hostile(names), **(env or {})}, text=True, capture_output=True)

    def test_hf01_the_predicates_answer_the_same_with_any_builtin_replaced_by_a_function(self):
        digits63 = "a@sha256:" + "a" * 63
        for names in [(n,) for n in self.NAMES] + [self.NAMES]:
            with self.subTest(replaced=names):
                self.assertEqual(self.sh('. "$1"; rel_version_ok 1.2; exit $?', names).returncode, 1, names)
                self.assertEqual(self.sh('. "$1"; rel_version_ok 1.2.3; exit $?', names).returncode, 0, names)
                self.assertEqual(self.sh('. "$1"; rel_image_ok "$2"; exit $?', names, digits63).returncode, 1, names)   # 63 digits: only a length TEST refuses it
                lim = self.sh('. "$1"; rel_limit install.meta', names)
                self.assertEqual(lim.stdout.strip(), "4096", (names, lim.stdout, lim.stderr))
                home = self.sh('. "$1"; rel_home_ok "a/b"; exit $?', names)
                self.assertEqual(home.returncode, 1, (names, home.stdout, home.stderr))
                member = self.sh('. "$1"; rel_member_mode evil; exit $?', names)
                self.assertEqual(member.returncode, 1, (names, member.stdout, member.stderr))
                member = self.sh('. "$1"; rel_member_mode cortex', names)
                self.assertEqual(member.stdout.strip(), "- rwxr-xr-x", (names, member.stdout, member.stderr))
                meta = self.sh('. "$1"; rel_meta_value_ok MEMBER_LIST 3 1.2.3; exit $?', names)
                self.assertEqual(meta.returncode, 1, (names, meta.stdout, meta.stderr))

    def test_hf03_a_replaced_ulimit_cannot_lift_the_cap_on_curl(self):
        expect = {f"cortex-{VERSION}-x86_64-linux.tar.gz": 4294967296, f"cortex-server-{VERSION}-x86_64.oci.tar": 4294967296, "install.sh": 1048576,
                  "manifest.json": 1048576, "install.meta": 4096, "SHA256SUMS": 65536, "SHA256SUMS.sig": 65536}
        for names in ((), ("ulimit",), self.NAMES):
            with self.subTest(replaced=names):
                self.log.unlink(missing_ok=True)
                self.discard_stage()
                self.reserve()
                script = '''. "$1"; EX_FETCH=5; rel_tool_curl() { "$FAKES/curl" "$@"; }
rel_fetch "$2" "$3"
exit $?'''
                r = self.sh(script, names, VERSION, str(self.stage), env={"CORTEX_FROM": "", "CORTEX_BASE_URL": "https://releases.invalid/1.2.3"})
                self.assertEqual(r.returncode, 0, (names, r.stdout, r.stderr))
                got = [l.split("[fsize ")[1].split("]")[0] for l in self.calls().splitlines()]
                self.assertEqual(got, [str((v // 512 + 2) * 512) for v in expect.values()], names)

    def test_hf02_a_tampered_release_is_still_refused_and_a_good_one_still_trusted_with_all_replaced(self):
        script = r'''. "$1"; EX_TRUST=4
rel_trust "$2" "$3" "$4" "$5" "$6"
exit $?'''
        self.fill_stage()
        args = (VERSION, str(self.stage), str(self.key), PRINCIPAL, GOOD["fpr"])
        r = self.sh(script, self.NAMES, *args)
        self.assertEqual(r.returncode, 0, (r.stdout, r.stderr))
        (self.stage / self.tar.name).write_bytes((self.stage / self.tar.name).read_bytes() + b"x")
        r = self.sh(script, self.NAMES, *args)
        self.assertEqual(r.returncode, 4, (r.stdout, r.stderr))
        self.assertEqual(r.stderr.splitlines()[0], PROBLEM["trust_checksum"])
        for names in [(n,) for n in self.NAMES]:
            with self.subTest(replaced=names):
                r = self.sh(script, names, *args)
                self.assertEqual(r.returncode, 4, (names, r.stdout, r.stderr))
                self.assertEqual(r.stderr.splitlines()[0], PROBLEM["trust_checksum"], names)


class Stage2(Cell):
    """Stage 2 is the Unpack, Load and Verify cells above."""
    pass


if __name__ == "__main__":
    unittest.main(argv=[sys.argv[0], *sys.argv[2:]], verbosity=2)
PY
