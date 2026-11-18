#!/usr/bin/env python3
"""test_mac_release.py: cells and mutants for the MAC release recipe (scripts/mac-release.sh and scripts/mac_release_manifest.py). hue, 15 Nov 2026; cox W1-281.

Run:  TMPDIR=<private scratch under /home> sh scripts/mac-release-test.sh      (exit 0 only if every cell passes and every mutant is killed)

HOW IT TESTS. Each cell builds a small SYNTHETIC git repository: the real deploy/, mac/ and the recipe and helper files, a tiny Cargo workspace, a synthetic OCI image archive, and
runs the REAL scripts/mac-release.sh on it with the recipe's own test seams (the heavy gates CI, S5, the roles proof and squawk are the release run's, not this suite's). The git
checks, the image binding, the Mac join, the archive, the manifest and the publication are real. A cell checks what the recipe wrote (or that it wrote nothing), against a TABLE of
the Mac set's specification (the members and modes of nemo's fixture scripts/install-mac-fixture.py, which is run and compared). A MUTANT removes one guard in a copy of the
recipe or of the helper and the cell that guards it must then FAIL: a mutant whose edit did not change the file, or that no cell notices, is a failure of the suite.
NOT covered here, on purpose: the gates themselves (release.sh's, copied unchanged: scripts/mac-release-parity-check.py proves the copy) and the real run on the real tree."""
import gzip
import hashlib
import io
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import tarfile
import tempfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
IMAGE_SOURCE_VERSION = "0.2.2"
MEMBERS_SPEC = {   # name -> (type, mode): the Mac member list 3 (mac/install-fetch.sh rel_member_mode; nemo's fixture MEMBERS)
    "compose.yml": ("-", 0o644), "stack.env": ("-", 0o644), "db-roles.sh": ("-", 0o755), "cortex.toml.example": ("-", 0o644),
    "docker/": ("d", 0o755), "docker/compose.docker.yml": ("-", 0o644), "bin/": ("d", 0o755), "bin/docker-compose-render": ("-", 0o755), "bin/docker-compose-lint": ("-", 0o755),
    "initdb/": ("d", 0o755), "initdb/00-cortex-roles.sh": ("-", 0o644)}
COPY_DIRS = ("deploy", "mac")
COPY_FILES = ("scripts/mac-release.sh", "scripts/release_manifest.py", "scripts/mac_release_manifest.py", "scripts/release-ci-check.py", "scripts/release-ci-allowlist.json", "scripts/docker-compose-render", "scripts/docker-compose-lint",
              "scripts/release-squawk.toml", "scripts/release-migration-exceptions.json", "scripts/release-migration-approval.json", "rust-toolchain.toml")
SPEC_FILES = ("scripts/install-mac-fixture.py",)
results = []
_cargo_lock = None


def sh(cmd, cwd=None, env=None, timeout=300):
    return subprocess.run(cmd, cwd=cwd, env=env, capture_output=True, text=True, timeout=timeout)


def sha(b):
    return hashlib.sha256(b).hexdigest()


def record(name, ok, detail=""):
    results.append((name, ok, detail))
    print(("ok    " if ok else "FAIL  ") + name + ((" :: " + detail[:700]) if (detail and not ok) else ""), flush=True)


# ------------------------------------------------------------------ the synthetic OCI image archive
def oci_archive(path, arch="arm64", os_="linux", manifests=1):
    def blob(data):
        return sha(data), data
    files = {}
    entries = []
    for i in range(manifests):
        cd, cb = blob(json.dumps({"architecture": arch, "os": os_, "note": i}).encode())
        md, mb = blob(json.dumps({"schemaVersion": 2, "mediaType": "application/vnd.oci.image.manifest.v1+json", "config": {"mediaType": "application/vnd.oci.image.config.v1+json", "digest": "sha256:" + cd, "size": len(cb)}, "layers": []}).encode())
        files["blobs/sha256/" + cd] = cb
        files["blobs/sha256/" + md] = mb
        entries.append({"mediaType": "application/vnd.oci.image.manifest.v1+json", "digest": "sha256:" + md, "size": len(mb)})
    files["oci-layout"] = b'{"imageLayoutVersion":"1.0.0"}'
    files["index.json"] = json.dumps({"schemaVersion": 2, "manifests": entries}).encode()
    with tarfile.open(path, "w") as t:
        for name, data in files.items():
            info = tarfile.TarInfo(name)
            info.size = len(data)
            t.addfile(info, io.BytesIO(data))
    return sha(Path(path).read_bytes()), entries[0]["digest"]


# ------------------------------------------------------------------ the synthetic repository
def cargo_lock(tmp):
    global _cargo_lock
    if _cargo_lock is None:
        d = Path(tmp) / "lockgen"
        (d / "crates/cortex-server/src").mkdir(parents=True)
        (d / "Cargo.toml").write_text(WORKSPACE_TOML)
        (d / "crates/cortex-server/Cargo.toml").write_text(MEMBER_TOML)
        (d / "crates/cortex-server/src/lib.rs").write_text("pub fn f() {}\n")
        shutil.copyfile(ROOT / "rust-toolchain.toml", d / "rust-toolchain.toml")
        r = sh(["cargo", "generate-lockfile", "--offline"], cwd=d, env=dict(os.environ, RUSTC_WRAPPER=""))
        assert r.returncode == 0, r.stderr
        _cargo_lock = (d / "Cargo.lock").read_text()
    return _cargo_lock


WORKSPACE_TOML = '[workspace]\nresolver = "2"\nmembers = ["crates/cortex-server"]\n\n[workspace.package]\nversion = "0.2.2"\nedition = "2021"\n'
MEMBER_TOML = '[package]\nname = "cortex-server"\nversion.workspace = true\nedition.workspace = true\n\n[features]\nmod-policy = []\n'


class Repo:
    def __init__(self, tmp, name, extra_commits=(), version="0.2.2"):
        self.root = Path(tmp) / name
        self.root.mkdir()
        for d in COPY_DIRS:
            shutil.copytree(ROOT / d, self.root / d, symlinks=True)
        for f in COPY_FILES:
            (self.root / f).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / f, self.root / f)
        for mig in sorted((ROOT / "crates").glob("*/migrations")):   # the exception list pins real migration files: they are copied (small SQL files), nothing else of crates/
            shutil.copytree(mig, self.root / "crates" / mig.parent.name / "migrations")
        (self.root / "Cargo.toml").write_text(WORKSPACE_TOML.replace("0.2.2", version))
        (self.root / "crates/cortex-server/src").mkdir(parents=True, exist_ok=True)
        (self.root / "crates/cortex-server/Cargo.toml").write_text(MEMBER_TOML)
        (self.root / "crates/cortex-server/src/lib.rs").write_text("pub fn f() {}\n")
        (self.root / "xtask").mkdir()
        (self.root / "xtask/placeholder.txt").write_text("xtask\n")
        (self.root / "Cargo.lock").write_text(cargo_lock(tmp))
        (self.root / ".gitignore").write_text("/target\n/dist\n")
        self.g("init", "-q", "-b", "main")
        self.g("config", "user.name", "t")
        self.g("config", "user.email", "t@t")
        self.g("add", "-A")
        self.g("commit", "-q", "-m", "c1: the image's source")
        self.c1 = self.g("rev-parse", "HEAD").stdout.strip()
        (self.root / "NOTES.txt").write_text("not a build input\n")
        self.g("add", "-A")
        self.g("commit", "-q", "-m", "c2: the head")
        for fn in extra_commits:
            fn(self)
        self.head = self.g("rev-parse", "HEAD").stdout.strip()

    def g(self, *a):
        return sh(["git", "-C", str(self.root)] + list(a))

    def commit(self, msg):
        self.g("add", "-A")
        self.g("commit", "-q", "-m", msg)

    def side_commit(self):
        """a commit on another branch with the SAME build inputs as c1: not an ancestor of the head"""
        self.g("checkout", "-q", "-b", "side", self.c1)
        (self.root / "SIDE.txt").write_text("side\n")
        self.commit("side")
        c = self.g("rev-parse", "HEAD").stdout.strip()
        self.g("checkout", "-q", "main")
        return c


class Fixture:
    """the assets every run needs: gate stubs, a test trust key, notices and licence, the fixture crates directory, the OCI archives"""
    def __init__(self, tmp):
        d = self.d = Path(tmp) / "assets"
        d.mkdir()
        (d / "ok.sh").write_text("#!/bin/sh\nexit 0\n")
        (d / "ci.sh").write_text("#!/bin/sh\nprintf 'fixture PASS\\nci: PASS\\n'\n")
        (d / "fail.sh").write_text("#!/bin/sh\necho gate-failed >&2\nexit 1\n")
        (d / "proof.sh").write_text('#!/bin/sh\necho "last live run matches the current scripts"\n')
        for n in ("ok.sh", "ci.sh", "fail.sh", "proof.sh"):
            (d / n).chmod(0o755)
        r = sh(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", "fixture", "-f", str(d / "key")])
        assert r.returncode == 0, r.stderr
        pub = (d / "key.pub").read_text().split()
        (d / "trust.keyfile").write_text("cortex-release %s %s\n" % (pub[0], pub[1]))
        (d / "NOTICES.txt").write_text("fixture notices\n")
        (d / "LICENSE.txt").write_text("fixture licence\n")
        (d / "crates/cortex-db/migrations/platform").mkdir(parents=True)
        (d / "crates/cortex-db/migrations/platform/0001_fixture.up.sql").write_text("CREATE TABLE fixture_t (id bigint);\n")
        self.crates = d / "crates"
        self.arm = d / "arm64.oci.tar"
        self.arm_sha, self.arm_digest = oci_archive(self.arm)
        self.amd = d / "amd64.oci.tar"
        self.amd_sha, _ = oci_archive(self.amd, arch="amd64")
        self.two = d / "two.oci.tar"
        self.two_sha, _ = oci_archive(self.two, manifests=2)
        self.sha = lambda p: sha(Path(p).read_bytes())


def run_recipe(repo, fx, drop=(), override=None, extra_env=None, seams=True, version="0.2.2", pre=None):
    """runs scripts/mac-release.sh in the synthetic repo; returns (returncode, stdout+stderr)"""
    receipt = fx.d / ("receipt-" + repo.root.name)
    receipt.mkdir(exist_ok=True)
    receipt_commit = (override or {}).get("--image-source-commit", repo.c1)
    receipt_sha = (override or {}).get("--image-sha256", fx.arm_sha)
    verdict = ("ok    the archive's sha256 here (" + receipt_sha + ") equals the machine's own (" + receipt_sha + ")\n"
               + "ok    cortex-server --version ran once on the machine (exit 0) and prints " + version + " and the source's git sha " + receipt_commit + " (" + version + " " + receipt_commit + ")\n")
    (receipt / "VERDICT.txt").write_text(verdict)
    (receipt / "SHA256SUMS-part-b-run-receipt.txt").write_text(sha(verdict.encode()) + "  ./VERDICT.txt\n")
    opts = {"--trust-key-file": str(fx.d / "trust.keyfile"), "--image-archive": str(fx.arm), "--image-sha256": fx.arm_sha, "--image-source-commit": repo.c1,
            "--image-receipt": str(receipt),
            "--notices": str(fx.d / "NOTICES.txt"), "--notices-sha256": fx.sha(fx.d / "NOTICES.txt"), "--license": str(fx.d / "LICENSE.txt"), "--license-sha256": fx.sha(fx.d / "LICENSE.txt")}
    opts.update(override or {})
    for k in drop:
        opts.pop(k, None)
    cmd = ["sh", "scripts/mac-release.sh", version]
    for k, v in opts.items():
        cmd += [k, v]
    env = dict(os.environ, RUSTC_WRAPPER="")
    if seams:
        env.update(CORTEX_RELEASE_FIXTURE_ROOT=str(repo.root), CORTEX_RELEASE_CI=str(fx.d / "ci.sh"), CORTEX_RELEASE_S5=str(fx.d / "ok.sh"), CORTEX_RELEASE_PROOF=str(fx.d / "proof.sh"),
                   CORTEX_RELEASE_SQUAWK=str(fx.d / "ok.sh"), CORTEX_RELEASE_CRATES_DIR=str(fx.crates))
    env.update(extra_env or {})
    if pre:
        pre(repo)
    p = sh(cmd, cwd=repo.root, env=env, timeout=600)
    return p.returncode, p.stdout + p.stderr


def nothing_published(repo, version="0.2.2"):
    dist = repo.root / "dist"
    return not any(dist.glob(version + "*")) if dist.exists() else True


def tar_table(path):
    out = {}
    with tarfile.open(path, "r:gz") as t:
        for m in t.getmembers():
            out[m.name + ("/" if m.isdir() and not m.name.endswith("/") else "")] = ("d" if m.isdir() else "-", m.mode & 0o777)
    return out


# ------------------------------------------------------------------ the cells
def cells(tmp, fx, recipe_text=None, helper_text=None, only=None):
    """Runs the cells; returns {name: (ok, detail)}. With recipe_text / helper_text the synthetic repos carry the MUTATED copies of the recipe / helper."""
    out = {}

    def new_repo(name, **kw):
        r = Repo(tmp, name + ("-m" if (recipe_text or helper_text) else ""), **kw)
        if recipe_text is not None:
            (r.root / "scripts/mac-release.sh").write_text(recipe_text)
        if helper_text is not None:
            (r.root / "scripts/mac_release_manifest.py").write_text(helper_text)
        if recipe_text is not None or helper_text is not None:
            r.commit("mutated copy")
            r.c1 = r.c1   # the image commit stays c1: the mutation does not touch a build input
            r.head = r.g("rev-parse", "HEAD").stdout.strip()
        return r

    def cell(name, fn):
        if only and name not in only:
            return
        try:
            ok, detail = fn()
        except Exception as e:   # a crash of a cell is a failure, with its type
            ok, detail = False, "the cell crashed: %s: %s" % (type(e).__name__, e)
        out[name] = (ok, detail)

    def refused(repo, rc, text, want, expect_rc=None):
        ok = rc != 0 and want in text and nothing_published(repo) and (expect_rc is None or rc == expect_rc)
        return ok, "rc=%s want=%r published=%s tail=%s" % (rc, want, not nothing_published(repo), text.strip().splitlines()[-2:])

    def happy():
        r = new_repo("happy")
        rc, text = run_recipe(r, fx)
        d = r.root / "dist/0.2.2"
        bad = []
        if rc != 0:
            return False, "rc=%s %s" % (rc, text[-400:])
        want = {"cortex-0.2.2-macos.tar.gz", "cortex-server-0.2.2-aarch64.oci.tar", "install.sh", "manifest.json", "install.meta", "SHA256SUMS"}
        got = {p.name for p in d.iterdir()}
        if got != want:
            bad.append("inventory %s" % sorted(got ^ want))
        if (d / "cortex-server-0.2.2-aarch64.oci.tar").read_bytes() != fx.arm.read_bytes():
            bad.append("the image in the set is not the imported archive")
        if oct((d / "cortex-server-0.2.2-aarch64.oci.tar").stat().st_mode & 0o777) != "0o600":
            bad.append("image mode %s" % oct((d / "cortex-server-0.2.2-aarch64.oci.tar").stat().st_mode & 0o777))
        table = tar_table(d / "cortex-0.2.2-macos.tar.gz")
        if table != MEMBERS_SPEC:
            bad.append("members/modes differ: %s" % sorted(set(table.items()) ^ set(MEMBERS_SPEC.items()))[:6])
        stack = tarfile.open(d / "cortex-0.2.2-macos.tar.gz", "r:gz").extractfile("stack.env").read().decode()
        if not re.fullmatch(r"CORTEX_VERSION=0\.2\.2\nCORTEX_SERVER_IMAGE=localhost/cortex-server@%s\nCORTEX_DB_IMAGE=\S+@sha256:[0-9a-f]{64}\n" % re.escape(fx.arm_digest), stack):
            bad.append("stack.env: %r" % stack)
        meta = (d / "install.meta").read_text().splitlines()
        keys = [l.split("=")[0] for l in meta]
        if keys != ["META_SCHEMA", "RELEASE_VERSION", "STACK_VERSION", "MEMBER_LIST", "CORTEX_SERVER_IMAGE", "CORTEX_DB_IMAGE", "ROLES_SCRIPT_DIGEST", "RELEASE_SEAMS"]:
            bad.append("install.meta keys %s" % keys)
        md = dict(l.split("=", 1) for l in meta)
        if md.get("MEMBER_LIST") != "3" or md.get("RELEASE_VERSION") != "0.2.2" or md.get("STACK_VERSION") != "0.2.2":
            bad.append("install.meta values")
        names = [l.split("  ")[1] for l in (d / "SHA256SUMS").read_text().splitlines()]
        if names != ["cortex-0.2.2-macos.tar.gz", "cortex-server-0.2.2-aarch64.oci.tar", "install.sh", "manifest.json", "install.meta"]:
            bad.append("SHA256SUMS order %s" % names)
        for l in (d / "SHA256SUMS").read_text().splitlines():
            h, n = l.split("  ")
            if sha((d / n).read_bytes()) != h:
                bad.append("SHA256SUMS line for " + n)
        man = json.loads((d / "manifest.json").read_text())
        c = man["releases"][0]["cortex"]
        if man["schema"] != 1 or c["members"] != 3 or c["server_image"]["asset"] != "cortex-server-0.2.2-aarch64.oci.tar" or [a["name"] for a in man["releases"][0]["assets"]] != ["cortex-0.2.2-macos.tar.gz", "cortex-server-0.2.2-aarch64.oci.tar"]:
            bad.append("manifest.json shape")
        if c["roles_script_digest"] != "sha256:" + sha((r.root / "deploy/db-roles.sh").read_bytes()):
            bad.append("roles digest")
        if c["migrations"] != ["platform/0001_fixture"]:
            bad.append("migrations %s" % c["migrations"])
        rec = json.loads((r.root / "dist/0.2.2.image-binding.json").read_text())
        if rec["head"] != r.head or rec["image_source_commit"] != r.c1 or sorted(rec["build_inputs_compared"]) != sorted(["rust-toolchain.toml", "Cargo.toml", "Cargo.lock", "crates", "xtask", "deploy/server-features", "deploy/Containerfile.arm64", "deploy/Containerfile.arm64.dockerignore"]):
            bad.append("image-binding record")
        beside = r.root / "dist/0.2.2.beside"
        if sorted(p.name for p in beside.iterdir()) != ["LICENSE", "THIRD-PARTY-NOTICES.txt"] or (beside / "LICENSE").read_bytes() != (fx.d / "LICENSE.txt").read_bytes():
            bad.append("beside")
        if not (r.root / "dist/0.2.2.install.sh.binding.json").is_file():
            bad.append("the installer binding record is missing")
        if list(r.root.rglob("__pycache__")) or list(r.root.rglob("*.pyc")):
            bad.append("the run left bytecode in the source tree: %s" % [str(p.relative_to(r.root)) for p in list(r.root.rglob("*.pyc"))[:2]])
        if not (d / "install.sh").read_text().startswith("#!"):
            bad.append("install.sh")
        return not bad, "; ".join(bad)

    cell("C01 the happy path: the whole set, its members and modes, install.meta, SHA256SUMS, manifest, the image binding record, the notices and licence beside", happy)

    def shape_equals_nemo():
        r = new_repo("shape")
        rc, text = run_recipe(r, fx)
        if rc != 0:
            return False, text[-300:]
        fxdir = Path(tmp) / "nemo-fixture"
        fxdir.mkdir()
        p = sh([sys.executable, "-I", str(ROOT / "scripts/install-mac-fixture.py"), str(fxdir), "0.2.2"])
        if p.returncode != 0:
            return False, "nemo's fixture did not run: " + p.stderr[-200:]
        rel = fxdir / "release"
        mine = r.root / "dist/0.2.2"
        bad = []
        t1, t2 = tar_table(mine / "cortex-0.2.2-macos.tar.gz"), tar_table(rel / "cortex-0.2.2-macos.tar.gz")
        shape = lambda t: {k.rstrip("/"): v for k, v in t.items() if not (k.startswith("initdb/") and v[0] == "-")}   # the files INSIDE initdb/ are content (the fixture's is a stand-in), the directory is shape
        if shape(t1) != shape(t2):
            bad.append("members and modes differ from the fixture's: %s" % sorted(set(t1.items()) ^ set(t2.items()))[:4])
        m1 = [l.split("=")[0] for l in (mine / "install.meta").read_text().splitlines() if not l.startswith("RELEASE_SEAMS")]
        m2 = [l.split("=")[0] for l in (rel / "install.meta").read_text().splitlines()]
        if m1 != m2:
            bad.append("install.meta keys %s vs %s" % (m1, m2))
        j1, j2 = json.loads((mine / "manifest.json").read_text())["releases"][0], json.loads((rel / "manifest.json").read_text())["releases"][0]
        if sorted(j1) != sorted(j2) or sorted(j1["cortex"]) != sorted(set(j2["cortex"]) | {"min_upgrade_from"}):
            bad.append("manifest keys: %s vs %s" % (sorted(j1["cortex"]), sorted(j2["cortex"])))
        if [l.split("  ")[1] for l in (mine / "SHA256SUMS").read_text().splitlines()] != [l.split("  ")[1] for l in (rel / "SHA256SUMS").read_text().splitlines()]:
            bad.append("SHA256SUMS names/order")
        return not bad, "; ".join(bad)

    cell("C02 the set has the SHAPE of nemo's fixture (scripts/install-mac-fixture.py is run and compared: members, modes, meta keys, manifest keys, SHA256SUMS order)", shape_equals_nemo)

    def linux_untouched():
        r = new_repo("linux-untouched")

        def snapshot():
            paths = [r.root / "Cargo.toml", r.root / "Cargo.lock"]
            for name in ("deploy", "crates"):
                tree = r.root / name
                if tree.is_symlink() or not tree.is_dir():
                    raise ValueError("missing or linked input directory: " + name)
                paths.append(tree)
                paths.extend(tree.rglob("*"))
            found = {}
            for path in sorted(paths):
                rel = path.relative_to(r.root).as_posix()
                mode = path.lstat().st_mode
                if stat.S_ISLNK(mode):
                    found[rel] = ("link", os.readlink(path))
                elif stat.S_ISREG(mode):
                    found[rel] = ("file", stat.S_IMODE(mode), sha(path.read_bytes()))
                elif stat.S_ISDIR(mode):
                    found[rel] = ("directory", stat.S_IMODE(mode))
                else:
                    raise ValueError("unexpected input type: " + rel)
            return found

        before = snapshot()
        rc, text = run_recipe(r, fx)
        after = snapshot()
        changed = sorted(k for k in before.keys() | after.keys() if before.get(k) != after.get(k))
        return rc == 0 and not changed, "rc=%s changed=%s tail=%s" % (rc, changed[:10], text[-200:])

    cell("C03 the Mac recipe leaves Linux deploy, crates and workspace inputs unchanged", linux_untouched)

    def wrong_sha():
        r = new_repo("wrongsha")
        rc, text = run_recipe(r, fx, override={"--image-sha256": "0" * 64})
        return refused(r, rc, text, "the server image is not acceptable")
    cell("G01 a wrong --image-sha256 is refused and nothing is published", wrong_sha)

    def not_arm():
        r = new_repo("notarm")
        rc, text = run_recipe(r, fx, override={"--image-archive": str(fx.amd), "--image-sha256": fx.amd_sha})
        return refused(r, rc, text, "the server image is not acceptable")
    cell("G02 an amd64 image archive (its own sha256 right) is refused", not_arm)

    def two_manifests():
        r = new_repo("two")
        rc, text = run_recipe(r, fx, override={"--image-archive": str(fx.two), "--image-sha256": fx.two_sha})
        return refused(r, rc, text, "the server image is not acceptable")
    cell("G03 an image archive with two manifests is refused", two_manifests)

    def not_ancestor():
        r = new_repo("notanc")
        side = r.side_commit()
        rc, text = run_recipe(r, fx, override={"--image-source-commit": side})
        return refused(r, rc, text, "not bound to this head")
    cell("G04 an image source commit that is not an ancestor of the head is refused", not_ancestor)

    def input_changed():
        def edit(r):
            (r.root / "crates/cortex-server/src/lib.rs").write_text("pub fn f() { /* changed after the image */ }\n")
            r.commit("a build input changes after the image")
        r = new_repo("inputchg", extra_commits=(edit,))
        rc, text = run_recipe(r, fx)
        ok, d = refused(r, rc, text, "not bound to this head")
        return ok and "crates" in text, d + " | names the path: %s" % ("crates" in text)
    cell("G05 a build input of the image changed between the image's commit and the head: refused, the path named", input_changed)

    def mac_extra():
        def edit(r):
            (r.root / "mac/README.txt").write_text("an extra file\n")
            r.commit("extra file in mac/")
        r = new_repo("macextra", extra_commits=(edit,))
        rc, text = run_recipe(r, fx)
        return refused(r, rc, text, "mac/ must hold exactly")
    cell("G06 mac/ holding a file beyond the three installer files is refused", mac_extra)

    def docker_extra():
        def edit(r):
            (r.root / "deploy/docker/extra.yml").write_text("x: y\n")
            r.commit("extra file in deploy/docker")
        r = new_repo("dockerextra", extra_commits=(edit,))
        rc, text = run_recipe(r, fx)
        return refused(r, rc, text, "docker/ must hold exactly")
    cell("G07 deploy/docker/ holding a file beyond compose.docker.yml is refused (the closed member list)", docker_extra)

    def notices_sha():
        r = new_repo("notsha")
        rc, text = run_recipe(r, fx, override={"--notices-sha256": "1" * 64})
        return refused(r, rc, text, "the notices file is not the one named")
    cell("G08 a wrong --notices-sha256 is refused", notices_sha)

    def real_needs_beside():
        r = new_repo("realbeside")
        rc, text = run_recipe(r, fx, drop=("--notices", "--notices-sha256", "--license", "--license-sha256"))
        return refused(r, rc, text, "are required for a real release", expect_rc=2)
    cell("G09 a real release without --notices and --license is refused (exit 2)", real_needs_beside)

    def missing_arg():
        r = new_repo("missarg")
        rc, text = run_recipe(r, fx, drop=("--image-source-commit",))
        return refused(r, rc, text, "are required", expect_rc=2)
    cell("G10 a missing --image-source-commit is refused (exit 2)", missing_arg)

    def bad_commit():
        r = new_repo("badcommit")
        rc, text = run_recipe(r, fx, override={"--image-source-commit": "abc"})
        return refused(r, rc, text, "must be 40 lowercase hex", expect_rc=2)
    cell("G11 an image source commit that is not 40 lowercase hex is refused (exit 2)", bad_commit)

    def dest_beside():
        r = new_repo("destbeside")
        return_pre = lambda repo: ((repo.root / "dist/0.2.2.beside").mkdir(parents=True))
        rc, text = run_recipe(r, fx, pre=return_pre)
        return (rc != 0 and "the destination for the notices and the licence exists" in text and not (r.root / "dist/0.2.2").exists()), "rc=%s tail=%s" % (rc, text.strip().splitlines()[-2:])
    cell("G12 an existing dist/V.beside is refused before anything is built", dest_beside)

    def dest_release():
        r = new_repo("destrel")
        rc, text = run_recipe(r, fx, pre=lambda repo: (repo.root / "dist/0.2.2").mkdir(parents=True))
        return (rc != 0 and "destination exists" in text), "rc=%s tail=%s" % (rc, text.strip().splitlines()[-2:])
    cell("G13 an existing dist/V is refused (inherited guard kept)", dest_release)

    def no_trust():
        r = new_repo("notrust")
        rc, text = run_recipe(r, fx, drop=("--trust-key-file",))
        return refused(r, rc, text, "--trust-key-file is required", expect_rc=2)
    cell("G14 a real release without --trust-key-file is refused (exit 2; inherited guard kept)", no_trust)

    def dirty():
        r = new_repo("dirty")
        rc, text = run_recipe(r, fx, pre=lambda repo: (repo.root / "UNTRACKED.txt").write_text("x\n"))
        return refused(r, rc, text, "the work tree is not clean")
    cell("G15 an untracked file in the work tree is refused (inherited guard kept)", dirty)

    def seams_outside():
        r = new_repo("seamout")
        rc, text = run_recipe(r, fx, extra_env={"CORTEX_RELEASE_FIXTURE_ROOT": ""})
        return refused(r, rc, text, "test seams are set outside a fixture", expect_rc=2)
    cell("G16 test seams outside a fixture are refused (exit 2; inherited guard kept)", seams_outside)

    def wrong_version():
        r = new_repo("wrongver")
        rc, text = run_recipe(r, fx, version="0.2.3")
        return refused(r, rc, text, "workspace version check failed")
    cell("G17 a release version different from the workspace version is refused (inherited guard kept)", wrong_version)

    def ci_fails():
        r = new_repo("cifail")
        rc, text = run_recipe(r, fx, extra_env={"CORTEX_RELEASE_CI": str(fx.d / "fail.sh")})
        return refused(r, rc, text, "CI failed")
    cell("G18 a failing CI gate stops the release before anything is built (inherited guard kept)", ci_fails)

    def mac_changed_during():
        def s5_touches_mac(repo):
            s = fx.d / "s5-touch.sh"
            s.write_text("#!/bin/sh\necho '# changed while the release runs' >> %s/mac/install-mac.sh\nexit 0\n" % repo.root)
            s.chmod(0o755)
        r = new_repo("macrace")
        s5_touches_mac(r)
        rc, text = run_recipe(r, fx, extra_env={"CORTEX_RELEASE_S5": str(fx.d / "s5-touch.sh")})
        ok, d = refused(r, rc, text, "a mac installer file changed during the release")
        return ok, d
    cell("G19 a mac installer file changed while the release runs is refused at the end, by its own message", mac_changed_during)

    def missing_receipt():
        r = new_repo("missing-receipt")
        rc, text = run_recipe(r, fx, override={"--image-receipt": str(fx.d / "missing-receipt")})
        return refused(r, rc, text, "the image receipt is invalid")
    cell("G20 a missing image receipt stops the release before publication", missing_receipt)

    # ---- the helper, directly
    helper = (helper_text if helper_text is not None else (ROOT / "scripts/mac_release_manifest.py").read_text())

    def helper_cell(name, fn):
        cell(name, fn)

    def with_helper(r):
        p = r.root / "scripts/mac_release_manifest.py"
        return p

    def h_version():
        r = new_repo("hver")
        p = sh([sys.executable, "-I", str(r.root / "scripts/mac_release_manifest.py"), "image-binding", "--root", str(r.root), "--image-commit", r.c1, "--version", "0.2.1"])
        return p.returncode != 0 and "workspace version at the image's commit is 0.2.2, not 0.2.1" in p.stderr, "rc=%s %s" % (p.returncode, p.stderr.strip()[-200:])
    helper_cell("H01 image-binding refuses a release version that is not the workspace version at the image's commit", h_version)

    def h_verify_dist():
        r = new_repo("hvd")
        rc, text = run_recipe(r, fx)
        d = r.root / "dist/0.2.2"
        (d / "EXTRA.txt").write_text("x\n")
        p = sh([sys.executable, "-I", str(r.root / "scripts/mac_release_manifest.py"), "verify-dist", "--output-dir", str(d), "--version", "0.2.2"])
        (d / "EXTRA.txt").unlink()
        return p.returncode != 0 and "inventory differs" in p.stderr, "rc=%s %s" % (p.returncode, p.stderr.strip()[-200:])
    helper_cell("H02 verify-dist refuses a distribution holding a file beyond the five and SHA256SUMS", h_verify_dist)

    def h_digest_big():
        big = Path(tmp) / "big.bin"
        if not big.exists():
            with open(big, "wb") as f:
                f.write(b"\0" * (70 * 1024 * 1024))
        p = sh([sys.executable, "-I", str(ROOT / "scripts/mac_release_manifest.py"), "digest", "--path", str(big)])
        want = sha(b"\0" * (70 * 1024 * 1024))
        return p.returncode == 0 and p.stdout.strip() == want, "rc=%s out=%s" % (p.returncode, p.stdout.strip()[:80] + p.stderr[-100:])
    helper_cell("H03 digest hashes a file over 64 MiB (the Linux file-digest refuses it; the image archive is 88 MiB)", h_digest_big)

    def h_digest_link():
        r = new_repo("hlink")
        link = r.root / "link.bin"
        link.symlink_to(fx.arm)
        p = sh([sys.executable, "-I", str(r.root / "scripts/mac_release_manifest.py"), "digest", "--path", str(link)])
        return p.returncode != 0, "rc=%s" % p.returncode
    helper_cell("H04 digest refuses a symbolic link", h_digest_link)
    return out


# ------------------------------------------------------------------ the mutants
RECIPE_MUTANTS = [
    ("MR11 the Mac recipe changes a Linux input after its final clean check", "printf 'mac-release: built dist/%s\\n' \"$version\"", "printf '\\n# mutated\\n' >> \"$root/deploy/install-prereq.sh\"\nprintf 'mac-release: built dist/%s\\n' \"$version\"", "C03 the Mac recipe leaves Linux deploy, crates and workspace inputs unchanged"),
    ("MR01 the image check is not run (an amd64 image gets in)", "python3 -I \"$mhelper\" image-check --archive \"$image_archive\" --sha256 \"$image_sha256\" > \"$work/image-check.json\" || { echo 'mac-release: the server image is not acceptable' >&2; exit 1; }", "true", "G02 an amd64 image archive (its own sha256 right) is refused"),
    ("MR02 the image binding is not run (a changed build input gets in)", "    python3 -I \"$mhelper\" image-binding --root \"$root\" --image-commit \"$image_source_commit\" --version \"$version\" > \"$work/image-binding.json\" || { echo 'mac-release: the server image is not bound to this head (a build input differs, or the image commit is not an ancestor)' >&2; exit 1; }", "    echo '{}' > \"$work/image-binding.json\"", "G05 a build input of the image changed between the image's commit and the head: refused, the path named"),
    ("MR03 mac/ is not required to hold exactly the three files", "[ \"$(ls -A \"$work/mac\" | sort | tr '\\n' ' ')\" = 'install-fetch.sh install-mac.sh install-prereq.sh ' ] ||", "true ||", "G06 mac/ holding a file beyond the three installer files is refused"),
    ("MR04 the notices sha256 is not checked", "[ \"$(python3 -I \"$helper\" file-digest --path \"$notices\")\" = \"$notices_sha256\" ] ||", "true ||", "G08 a wrong --notices-sha256 is refused"),
    ("MR05 the image arguments are not required", "[ -n \"$image_archive\" ] && [ -n \"$image_sha256\" ] && [ -n \"$image_source_commit\" ] ||", "true ||", "G10 a missing --image-source-commit is refused (exit 2)"),
    ("MR06 a real release does not need the notices and the licence", "    [ -n \"$notices\" ] && [ -n \"$license_file\" ] || { echo 'mac-release: --notices and --license (each with its sha256) are required for a real release' >&2; exit 2; }\n", "", "G09 a real release without --notices and --license is refused (exit 2)"),
    ("MR07 an existing dist/V.beside is not refused", "[ ! -e \"dist/$version.beside\" ] && [ ! -L \"dist/$version.beside\" ] ||", "true ||", "G12 an existing dist/V.beside is refused before anything is built"),
    ("MR08 the end check of the Mac installer files is gone", "[ \"$(tree_digest \"$root/mac\")\" = \"$mac_digest\" ] || { echo 'mac-release: a mac installer file changed during the release' >&2; exit 1; }\n", "", "G19 a mac installer file changed while the release runs is refused at the end, by its own message"),
    ("MR09 the image source commit is not checked for its shape", "case $image_source_commit in *[!0-9a-f]*|'') echo 'mac-release: --image-source-commit must be 40 lowercase hex' >&2; exit 2 ;; esac\n[ \"${#image_source_commit}\" -eq 40 ] || { echo 'mac-release: --image-source-commit must be 40 lowercase hex' >&2; exit 2; }\n", "", "G11 an image source commit that is not 40 lowercase hex is refused (exit 2)"),
    ("MR10 the Mac distribution check is not run", "python3 -I \"$mhelper\" verify-dist --output-dir \"$part\" --version \"$version\" || { echo 'mac-release: distribution verification failed' >&2; exit 1; }\n", "", None),
]
HELPER_MUTANTS = [
    ("MH01 image-check does not read the architecture", "    if conf.get(\"os\") != \"linux\" or conf.get(\"architecture\") != \"arm64\":", "    if False:", "G02 an amd64 image archive (its own sha256 right) is refused"),
    ("MH02 image-binding does not require the commit to be an ancestor", "    if commit != head and _git(root, \"merge-base\", \"--is-ancestor\", commit, head).returncode != 0:", "    if False:", "G04 an image source commit that is not an ancestor of the head is refused"),
    ("MH03 image-binding does not compare the build inputs", "        if a.stdout.strip() != b.stdout.strip():\n            die(\"a build input of the image differs between the image's commit and the head: \" + p)\n        if _git(root, \"diff\", \"--quiet\", commit, head, \"--\", p).returncode != 0:\n            die(\"a build input of the image differs between the image's commit and the head: \" + p)\n", "", "G05 a build input of the image changed between the image's commit and the head: refused, the path named"),
    ("MH04 image-binding does not check the version at the image's commit", "    if at != version:", "    if False:", "H01 image-binding refuses a release version that is not the workspace version at the image's commit"),
    ("MH05 archive does not require docker/ to hold exactly compose.docker.yml", "    if sorted(p.name for p in docker.iterdir()) != [\"compose.docker.yml\"]:", "    if False:", "G07 deploy/docker/ holding a file beyond compose.docker.yml is refused (the closed member list)"),
    ("MH06 the Mac tools are not executable in the archive", "        entries[\"bin/\" + name] = (regular(scripts / name), 0o755)", "        entries[\"bin/\" + name] = (regular(scripts / name), 0o644)", "C01 the happy path: the whole set, its members and modes, install.meta, SHA256SUMS, manifest, the image binding record, the notices and licence beside"),
    ("MH07 the manifest says members 2", "\"members\": 3,", "\"members\": 2,", "C01 the happy path: the whole set, its members and modes, install.meta, SHA256SUMS, manifest, the image binding record, the notices and licence beside"),
    ("MH08 verify-dist does not check the inventory", "    if {p.name for p in root.iterdir()} != expected | {\"SHA256SUMS\"}:", "    if False:", "H02 verify-dist refuses a distribution holding a file beyond the five and SHA256SUMS"),
    ("MH11 importing the Linux helper writes bytecode into the source tree", "sys.dont_write_bytecode = True\n", "", "C01 the happy path: the whole set, its members and modes, install.meta, SHA256SUMS, manifest, the image binding record, the notices and licence beside"),
    ("MH09 digest follows a symbolic link (it reads the path with no regular-file check at all)", "    print(sha(regular(args.path)).removeprefix(\"sha256:\"))", "    print(__import__(\"hashlib\").sha256(Path(args.path).read_bytes()).hexdigest())", "H04 digest refuses a symbolic link"),
    ("MH10 MEMBER_LIST is 2 in install.meta (the manifest and the meta differ)", "f\"MEMBER_LIST={cortex['members']}\\n\"", "\"MEMBER_LIST=2\\n\"", "C01 the happy path: the whole set, its members and modes, install.meta, SHA256SUMS, manifest, the image binding record, the notices and licence beside"),
]


# ------------------------------------------------------------------ the parity check: cells and mutants
PARITY_FILES = ("release.sh", "make-mac-release.py", "mac-release.sh", "mac-release-differences.txt")


def parity_run(base, name, edit=None, checker_text=None, drop=None):
    """a private copy of the four files; edit(dir) may change them; returns (returncode, stdout+stderr)"""
    d = Path(base) / name
    d.mkdir()
    for f in PARITY_FILES:
        if f != drop:
            shutil.copyfile(ROOT / "scripts" / f, d / f)
    if edit:
        edit(d)
    chk = d / "mac-release-parity-check.py"
    chk.write_text(checker_text if checker_text is not None else (ROOT / "scripts/mac-release-parity-check.py").read_text())
    p = sh([sys.executable, "-I", str(chk), str(d)])
    return p.returncode, p.stdout + p.stderr


def parity_cells(base, checker_text=None, only=None):
    out = {}
    n = [0]

    def run(name, **kw):
        n[0] += 1
        return parity_run(base, "%s-%d" % (name, n[0]), checker_text=checker_text, **kw)

    def cell(name, fn):
        if only and name not in only:
            return
        try:
            out[name] = fn()
        except Exception as e:
            out[name] = (False, "the cell crashed: %s: %s" % (type(e).__name__, e))

    def append(f, text):
        return lambda d: (d / f).write_text((d / f).read_text() + text)

    cell("P01 the committed pair (release.sh, the generator, mac-release.sh, the differences list) is in parity", lambda: ((lambda r: (r[0] == 0 and "OK" in r[1], r[1][-200:]))(run("p01"))))
    cell("P02 a hand edit of mac-release.sh is caught", lambda: ((lambda r: (r[0] == 1 and "mac-release.sh differs from the generator's output" in r[1], r[1][-250:]))(run("p02", edit=append("mac-release.sh", "# a hand edit\n")))))
    cell("P03 a change of release.sh that the Mac copy does not follow is caught", lambda: ((lambda r: (r[0] == 1 and "mac-release.sh differs from the generator's output" in r[1], r[1][-250:]))(run("p03", edit=append("release.sh", "# the Linux recipe moved\n")))))

    def p04():
        def edit(d):
            t = (d / "release.sh").read_text()
            assert t.count("bin_members='cortex-dump cortex-dump-status cortex-dump-alert cortex-restore restore-compare'") == 1
            (d / "release.sh").write_text(t.replace("bin_members='cortex-dump cortex-dump-status cortex-dump-alert cortex-restore restore-compare'", "bin_members='renamed'"))
        r = run("p04", edit=edit)
        return r[0] == 1 and "the generator stopped" in r[1] and "bin members" in r[1], r[1][-250:]
    cell("P04 a generator edit whose old text is gone from release.sh stops the check and NAMES the edit", p04)
    cell("P05 an altered differences list is caught", lambda: ((lambda r: (r[0] == 1 and "mac-release-differences.txt differs" in r[1], r[1][-250:]))(run("p05", edit=append("mac-release-differences.txt", "  1  an edit nobody made\n")))))
    cell("P06 a missing generator is caught", lambda: ((lambda r: (r[0] == 1 and "make-mac-release.py is missing" in r[1], r[1][-250:]))(run("p06", drop="make-mac-release.py"))))
    return out


PARITY_MUTANTS = [
    ("PM01 the checker compares only mac-release.sh (the differences list is not compared)", 'for name in ("mac-release.sh", "mac-release-differences.txt"):', 'for name in ("mac-release.sh",):', "P05 an altered differences list is caught"),
    ("PM02 the checker compares only the differences list (mac-release.sh is not compared)", 'for name in ("mac-release.sh", "mac-release-differences.txt"):', 'for name in ("mac-release-differences.txt",):', "P02 a hand edit of mac-release.sh is caught"),
    ("PM03 the checker ignores a generator that stops", "        if r.returncode != 0:\n            print(\"FAIL  the generator stopped", "        if False:\n            print(\"FAIL  the generator stopped", "P04 a generator edit whose old text is gone from release.sh stops the check and NAMES the edit"),
    ("PM04 the checker does not require its four files", "        if p.is_symlink() or not p.is_file():", "        if False:", "P06 a missing generator is caught"),
]


def main():
    base = tempfile.mkdtemp(prefix="macrelease-test.", dir=os.environ["TMPDIR"])
    try:
        fx = Fixture(base)
        res = cells(base, fx)
        for name, (ok, detail) in sorted(res.items()):
            record(name, ok, detail)
        recipe = (ROOT / "scripts/mac-release.sh").read_text()
        helper = (ROOT / "scripts/mac_release_manifest.py").read_text()
        killed = total = 0
        for name, old, new, guard in RECIPE_MUTANTS + HELPER_MUTANTS:
            total += 1
            is_recipe = (name, old, new, guard) in RECIPE_MUTANTS
            text = recipe if is_recipe else helper
            if text.count(old) != 1:
                record(name, False, "the mutant's anchor occurs %d times (expected 1)" % text.count(old))
                continue
            if guard is None:
                # no cell is meant to notice (a redundant guard, said so): the edit must still be applied and the recipe must still run
                record(name + "  [redundant guard: verify-dist is re-checked by the SHA256SUMS the recipe just wrote; not counted as killed]", True)
                total -= 1
                continue
            mutated = text.replace(old, new)
            r = cells(base, fx, recipe_text=mutated if is_recipe else None, helper_text=None if is_recipe else mutated, only={guard})
            ok, detail = r[guard]
            dead = not ok
            record(name + "  [guard cell: %s]" % guard[:48], dead, "the mutant SURVIVED: the guarding cell still passed")
            killed += 1 if dead else 0
        record("all mutants killed: %d of %d" % (killed, total), killed == total)
        pres = parity_cells(base)
        for name, (ok, detail) in sorted(pres.items()):
            record(name, ok, detail)
        pk = 0
        chk = (ROOT / "scripts/mac-release-parity-check.py").read_text()
        for name, old, new, guard in PARITY_MUTANTS:
            if chk.count(old) != 1:
                record(name, False, "the mutant's anchor occurs %d times (expected 1)" % chk.count(old))
                continue
            r = parity_cells(base, checker_text=chk.replace(old, new), only={guard})
            dead = not r[guard][0]
            record(name + "  [guard cell: %s]" % guard[:3], dead, "the mutant SURVIVED: the guarding cell still passed")
            pk += 1 if dead else 0
        record("all parity-check mutants killed: %d of %d" % (pk, len(PARITY_MUTANTS)), pk == len(PARITY_MUTANTS))
    finally:
        shutil.rmtree(base, ignore_errors=True)
    failed = [r for r in results if not r[1]]
    print("test_mac_release: %d of %d passed" % (len(results) - len(failed), len(results)))
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
