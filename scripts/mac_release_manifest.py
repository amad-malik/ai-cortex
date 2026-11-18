#!/usr/bin/env python3
"""Strict, standard-library helpers for the MAC release recipe (scripts/mac-release.sh): hue, 15 Nov 2026, cox's ruling W1-281, kai D173.

It IMPORTS scripts/release_manifest.py (the Linux helper, byte-identical, not reopened) and re-uses its checks (regular, sha, image_digest, migrations, db_image and the regexes);
it adds only what the Mac set does differently:

  archive       the Mac archive: the closed member list 3 (compose.yml, stack.env, db-roles.sh, cortex.toml.example, docker/compose.docker.yml, bin/docker-compose-render,
                bin/docker-compose-lint, initdb/*), NO native program, NO systemd units, NO backup scripts; the modes are the Mac fetch copy's (mac/install-fetch.sh rel_member_mode)
  manifest      manifest.json (members 3) and install.meta (MEMBER_LIST=3, RELEASE_VERSION=STACK_VERSION) for the names cortex-V-macos.tar.gz and cortex-server-V-aarch64.oci.tar
  verify-dist   the distribution inventory (the five files and SHA256SUMS) and every checksum, for the Mac names
  digest        the sha256 (bare hex) of ONE regular file, read in chunks with no size cap (the Linux file-digest refuses over 64 MiB; the image archive is 88 MiB)
  image-check   the IMPORTED server image: its sha256 is the one named, the archive holds ONE manifest, its config says os linux and architecture arm64 (nothing is run)
  image-binding the image's SOURCE commit and the head: the commit is an ancestor of the head (or the head), every path the image build READS is identical in both (the same
                git object id), and the workspace version at the image's commit is the release version; prints the record the recipe writes beside the set

Every refusal is one line on stderr and exit status 1 (a ValueError, as in the Linux helper). Standard library only; run with python3 -I."""
import argparse
import datetime
import importlib.util
import io
import json
import os
import re
import subprocess
import sys
import tarfile
import tomllib
from pathlib import Path

# a release writes nothing into the tree it is built from: importing the Linux helper must not leave scripts/__pycache__ behind (the recipe's own clean check would refuse it)
sys.dont_write_bytecode = True
_HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("release_manifest", _HERE / "release_manifest.py")
rm = importlib.util.module_from_spec(_spec)
sys.modules["release_manifest"] = rm
_spec.loader.exec_module(rm)

die, regular, sha, DIGEST, IMAGE, INITDB_NAME = rm.die, rm.regular, rm.sha, rm.DIGEST, rm.IMAGE, rm.INITDB_NAME

MAC_BIN = ("docker-compose-render", "docker-compose-lint")
ARCHIVE_NAME = "cortex-{v}-macos.tar.gz"
IMAGE_NAME = "cortex-server-{v}-aarch64.oci.tar"
FIXED_FILES = ("install.sh", "manifest.json", "install.meta")
# every path the arm64 server image build READS (deploy/Containerfile.arm64: its COPY lines, the file itself and its dockerignore): the image binding compares exactly these
BUILD_INPUTS = ("rust-toolchain.toml", "Cargo.toml", "Cargo.lock", "crates", "xtask", "deploy/server-features", "deploy/Containerfile.arm64", "deploy/Containerfile.arm64.dockerignore")
HEX40 = re.compile(r"[0-9a-f]{40}\Z", re.ASCII)
HEX64 = re.compile(r"[0-9a-f]{64}\Z", re.ASCII)


def archive(args):
    layout = Path(args.layout)
    entries = {"compose.yml": (regular(layout / "compose.yml"), 0o644),
               "db-roles.sh": (regular(layout / "db-roles.sh"), 0o755),
               "cortex.toml.example": (regular(layout / "cortex.toml.example"), 0o644)}
    for name in ("initdb", "docker"):
        d = layout / name
        if d.is_symlink() or not d.is_dir():
            die(name + " must be a real directory")
    docker = layout / "docker"
    if sorted(p.name for p in docker.iterdir()) != ["compose.docker.yml"]:
        die("docker/ must hold exactly compose.docker.yml")
    entries["docker/compose.docker.yml"] = (regular(docker / "compose.docker.yml"), 0o644)
    scripts = Path(args.scripts)
    for name in MAC_BIN:
        entries["bin/" + name] = (regular(scripts / name), 0o755)
    for path in (layout / "initdb").iterdir():
        if not INITDB_NAME.fullmatch(path.name):
            die("invalid initdb filename")
        entries["initdb/" + path.name] = (regular(path), 0o644)
    if not IMAGE.fullmatch(args.server_image) or not IMAGE.fullmatch(args.db_image):
        die("image reference is not pinned")
    env = (f"CORTEX_VERSION={args.version}\nCORTEX_SERVER_IMAGE={args.server_image}\n"
           f"CORTEX_DB_IMAGE={args.db_image}\n").encode()
    with tarfile.open(args.output, "w", format=tarfile.USTAR_FORMAT) as out:
        for name in sorted([*entries, "initdb/", "bin/", "docker/", "stack.env"]):
            info = tarfile.TarInfo(name)
            info.uid = info.gid = 0
            info.uname = info.gname = ""
            info.mtime = args.epoch
            if name in ("initdb/", "bin/", "docker/"):
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


def _check_version(version):
    match = re.fullmatch(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?", version, re.ASCII)
    if (len(version.encode()) > 64 or version.endswith(".part") or not match or
            (match.group(4) and any(p.isdigit() and len(p) > 1 and p[0] == "0" for p in match.group(4).split(".")))):
        die("invalid distribution version")


def verify_dist(args):
    version = args.version
    _check_version(version)
    root = Path(args.output_dir)
    if root.is_symlink() or not root.is_dir():
        die("distribution must be a real directory")
    expected = {ARCHIVE_NAME.format(v=version), IMAGE_NAME.format(v=version), *FIXED_FILES}
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


def manifest(args):
    if args.seams and not re.fullmatch(r"CORTEX_RELEASE_[A-Z0-9_]+(?:,CORTEX_RELEASE_[A-Z0-9_]+)*", args.seams, re.ASCII):
        die("seam list is invalid")
    _check_version(args.version)
    out = Path(args.output_dir)
    files = [ARCHIVE_NAME.format(v=args.version), IMAGE_NAME.format(v=args.version)]
    assets = [{"name": name, "url": name, "digest": sha(out / name)} for name in files]
    modules = tomllib.loads(regular(args.toml).read_text(encoding="utf-8"))["modules"]["enabled"]
    if not isinstance(modules, list) or not modules or any(not isinstance(m, str) or not re.fullmatch(r"[a-z][a-z0-9_]{0,31}", m, re.ASCII) for m in modules):
        die("enabled modules are invalid")
    cortex = {"label": args.label, "members": 3, "min_upgrade_from": args.minimum or None,
              "server_image": {"ref": args.server_image, "asset": files[1]},
              "db_image": args.db_image, "roles_script_digest": sha(Path(args.layout) / "db-roles.sh"),
              "sidecar_images": [], "migrations": rm.migrations(args.crates), "modules": modules,
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


def _blob(archive, digest, limit):
    """The bytes of blobs/sha256/<hex> of an OCI layout archive, bounded; the member must be a regular file whose name is exactly that path."""
    name = "blobs/sha256/" + digest.removeprefix("sha256:")
    matches = [m for m in archive.getmembers() if m.name == name]
    if len(matches) != 1 or not matches[0].isfile() or matches[0].size > limit:
        die("OCI archive blob missing or too large: " + name[:24])
    return archive.extractfile(matches[0]).read()


def image_check(args):
    if not HEX64.fullmatch(args.sha256):
        die("the image sha256 must be 64 lowercase hex")
    path = regular(args.archive)
    if sha(path) != "sha256:" + args.sha256:
        die("the image archive sha256 is not the one named")
    digest = rm.image_digest(path)   # ONE manifest in index.json, its digest well formed
    with tarfile.open(path, "r:") as archive:
        indexes = [m for m in archive.getmembers() if m.name == "index.json"]
        if len(indexes) != 1 or not indexes[0].isfile() or indexes[0].size > 1024 * 1024:
            die("OCI archive must have one bounded index.json")
        index = json.load(archive.extractfile(indexes[0]))
        if (not isinstance(index, dict) or not isinstance(index.get("manifests"), list)
                or len(index["manifests"]) != 1 or not isinstance(index["manifests"][0], dict)
                or index["manifests"][0].get("digest") != digest):
            die("OCI manifest changed during the image check")
        manifest_doc = json.loads(rm.oci_blob(archive, index["manifests"][0]))
        config = manifest_doc.get("config")
        if not isinstance(config, dict) or not isinstance(config.get("digest"), str) or not DIGEST.fullmatch(config["digest"]):
            die("the image manifest has no config digest")
        conf = json.loads(rm.oci_blob(archive, config))
    if conf.get("os") != "linux" or conf.get("architecture") != "arm64":
        die("the image is not linux/arm64 (os %r, architecture %r)" % (conf.get("os"), conf.get("architecture")))
    print(json.dumps({"digest": digest, "os": conf["os"], "architecture": conf["architecture"], "config": config["digest"]}))


def digest(args):
    print(sha(regular(args.path)).removeprefix("sha256:"))


def verify_receipt(args):
    root = Path(args.receipt)
    if not root.is_absolute() or root.is_symlink() or not root.is_dir():
        die("receipt must be an absolute real directory")
    if not HEX64.fullmatch(args.image_sha256) or not re.fullmatch(r"[0-9a-f]{40}", args.image_commit):
        die("invalid image binding")
    sums_path = regular(root / "SHA256SUMS-part-b-run-receipt.txt")
    sums = sums_path.read_bytes()
    if len(sums) > 1024 * 1024:
        die("receipt checksum list is too large")
    seen = set()
    verdict = None
    for line in sums.decode("ascii").splitlines():
        match = re.fullmatch(r"([0-9a-f]{64})  \./([A-Za-z0-9_.-]+(?:/[A-Za-z0-9_.-]+)*)", line)
        if not match or any(part in (".", "..") for part in match.group(2).split("/")):
            die("invalid receipt checksum entry")
        name = match.group(2)
        if name in seen:
            die("duplicate receipt checksum entry")
        seen.add(name)
        entry = root
        for part in name.split("/"):
            entry = entry / part
            if entry.is_symlink():
                die("receipt checksum entry is a link")
        if name == "VERDICT.txt":
            verdict_bytes = regular(entry).read_bytes()
            actual = rm.hashlib.sha256(verdict_bytes).hexdigest()
            verdict = verdict_bytes.decode("utf-8")
        else:
            actual = sha(regular(entry)).removeprefix("sha256:")
        if actual != match.group(1):
            die("receipt checksum mismatch: " + name)
    if verdict is None:
        die("receipt verdict is not checked")
    image_line = "ok    the archive's sha256 here (" + args.image_sha256 + ") equals the machine's own (" + args.image_sha256 + ")"
    version_line = ("ok    cortex-server --version ran once on the machine (exit 0) and prints " + args.version
                    + " and the source's git sha " + args.image_commit + " (" + args.version + " " + args.image_commit + ")")
    if verdict.splitlines().count(image_line) != 1 or verdict.splitlines().count(version_line) != 1:
        die("receipt verdict does not bind this image, version and source")
    print(rm.hashlib.sha256(sums).hexdigest())


def _git(root, *a):
    env = {"PATH": "/usr/bin:/bin", "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": "/dev/null"}
    return subprocess.run(["git", "-C", str(root), *a], env=env, capture_output=True, text=True)


def image_binding(args):
    root, commit, version = args.root, args.image_commit, args.version
    if not HEX40.fullmatch(commit):
        die("the image source commit must be 40 lowercase hex")
    _check_version(version)
    head = _git(root, "rev-parse", "HEAD")
    if head.returncode != 0:
        die("cannot read the git HEAD")
    head = head.stdout.strip()
    if _git(root, "cat-file", "-e", commit + "^{commit}").returncode != 0:
        die("the image source commit is not in this repository")
    if commit != head and _git(root, "merge-base", "--is-ancestor", commit, head).returncode != 0:
        die("the image source commit is not an ancestor of the head")
    compared = {}
    for p in BUILD_INPUTS:
        a, b = _git(root, "rev-parse", commit + ":" + p), _git(root, "rev-parse", head + ":" + p)
        if a.returncode != 0 or b.returncode != 0:
            die("a build input of the image is missing: " + p)
        if a.stdout.strip() != b.stdout.strip():
            die("a build input of the image differs between the image's commit and the head: " + p)
        if _git(root, "diff", "--quiet", commit, head, "--", p).returncode != 0:
            die("a build input of the image differs between the image's commit and the head: " + p)
        compared[p] = a.stdout.strip()
    shown = _git(root, "show", commit + ":Cargo.toml")
    try:
        at = tomllib.loads(shown.stdout)["workspace"]["package"]["version"]
    except (KeyError, tomllib.TOMLDecodeError):
        die("cannot read the workspace version at the image's commit")
    if at != version:
        die("the workspace version at the image's commit is %s, not %s" % (at, version))
    tree = lambda c: _git(root, "rev-parse", c + "^{tree}").stdout.strip()
    print(json.dumps({"head": head, "head_tree": tree(head), "image_source_commit": commit, "image_source_tree": tree(commit),
                      "version": version, "build_inputs_compared": compared}, indent=2, sort_keys=True))


def main():
    parser = argparse.ArgumentParser()
    sub = parser.add_subparsers(dest="command", required=True)
    p = sub.add_parser("archive")
    for name in ("layout", "scripts", "output", "version", "server-image", "db-image"):
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
    p = sub.add_parser("digest")
    p.add_argument("--path", required=True)
    p = sub.add_parser("image-check")
    p.add_argument("--archive", required=True)
    p.add_argument("--sha256", required=True)
    p = sub.add_parser("image-binding")
    p.add_argument("--root", required=True)
    p.add_argument("--image-commit", required=True)
    p.add_argument("--version", required=True)
    p = sub.add_parser("verify-receipt")
    for name in ("receipt", "image-sha256", "image-commit", "version"):
        p.add_argument("--" + name, required=True)
    args = parser.parse_args()
    try:
        if args.command == "archive":
            archive(args)
        elif args.command == "manifest":
            manifest(args)
        elif args.command == "verify-dist":
            verify_dist(args)
        elif args.command == "digest":
            digest(args)
        elif args.command == "image-check":
            image_check(args)
        elif args.command == "image-binding":
            image_binding(args)
        elif args.command == "verify-receipt":
            verify_receipt(args)
    except (ValueError, OSError, tarfile.TarError, json.JSONDecodeError) as error:
        print("mac-release: " + str(error), file=sys.stderr)
        raise SystemExit(1)


if __name__ == "__main__":
    main()
