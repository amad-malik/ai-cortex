#!/usr/bin/env python3
"""TEST SUPPORT ONLY (the MAC copy of install-fetch-fixture.py: the Mac archive cortex-V-macos.tar.gz and image cortex-server-V-aarch64.oci.tar, the Mac member list 3). Not part of any release, not carried by the release builder's member list, and it writes no key anywhere but under the directory it is given.

    python3 -I scripts/install-fetch-fixture.py <ABSOLUTE EMPTY DIRECTORY> <version>

Makes a REAL signed release set for the 6c include's joined cells (cox W1-99 (3), nemo's IT-140): the same set scripts/install-fetch-test.sh builds, so that rel_fetch, rel_trust,
rel_unpack, rel_load and verify_release_dir can run against it with NO fake in the lane. It makes TEST keys only (ssh-keygen -t ed25519, no passphrase), under the given
directory, and refuses everything else:

    <dir>/release/   the seven files as a release server would serve them: cortex-<v>-x86_64-linux.tar.gz, cortex-server-<v>-x86_64.oci.tar (a stand-in, not an image),
                     install.sh, manifest.json, install.meta, SHA256SUMS (five lines in the builder's order) and SHA256SUMS.sig (a REAL ssh-keygen -Y sign, namespace cortex-release)
    <dir>/private/   trust.key (mode 0600) and trust.key.pub: the TEST signing key; nothing else holds the private half
    <dir>/anchor/    trust.keyfile (ONE line '<principal> ssh-ed25519 <key>', the KEYFILE argument of rel_trust) and anchor.env (PRINCIPAL, KEY, FPR, KEYFILE, NAMESPACE, VERSION:
                     public values only, no secret)

It never overwrites: a missing, relative, linked (or reached THROUGH a link, with or without a trailing slash) or NON-EMPTY directory, a bad version, a wrong number of
arguments, or an existing key file is a refusal (exit 2) that writes nothing. If anything fails AFTER the three directories exist (a refusal, an exception, an interrupt), it
removes what it made, by exact name (release, private, anchor), so the directory it was given is left as it was found: empty. NOTE: the children it starts (ssh-keygen) inherit the
caller's whole environment; the interpreter itself runs isolated (-I). Standard library plus /usr/bin/ssh-keygen."""
import hashlib
import io
import json
import os
import re
import shutil
import subprocess
import sys
import tarfile

SERVER = "localhost/cortex-server@sha256:" + "a" * 64
DATABASE = "registry.invalid/db@sha256:" + "b" * 64
ROLES = b"#!/bin/sh\n# synthetic roles\n"
RENDER = b'#!/bin/sh\n# synthetic render tool: it records its call and succeeds\necho "RENDER-OF-THE-RELEASE $*" >> "${FAKE_LOG:-/dev/null}"\nexit 0\n'
NAMESPACE = "cortex-release"
PRINCIPAL = "cortex-release"
BIN = ("docker-compose-render", "docker-compose-lint")
MEMBERS = {"compose.yml": 0o644, "stack.env": 0o644, "db-roles.sh": 0o755, "cortex.toml.example": 0o644, "initdb": 0o755, "initdb/roles.sql": 0o644,
           "docker": 0o755, "docker/compose.docker.yml": 0o644, "bin": 0o755, **{"bin/" + n: 0o755 for n in BIN}}
SSH_KEYGEN = "/usr/bin/ssh-keygen"


def refuse(text):
    print("install-fetch-fixture: " + text, file=sys.stderr)
    sys.exit(2)


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def archive(path, version):
    stack_env = f"CORTEX_VERSION={version}\nCORTEX_SERVER_IMAGE={SERVER}\nCORTEX_DB_IMAGE={DATABASE}\n".encode()
    with tarfile.open(path, "w:gz", format=tarfile.USTAR_FORMAT) as out:
        for name, mode in MEMBERS.items():
            if name == os.environ.get("FIXTURE_OMIT"):   # TEST ONLY: one member is left out of the archive (still correctly signed)
                continue
            info = tarfile.TarInfo(name)
            info.mode = mode
            if name in ("initdb", "bin", "docker"):
                info.type = tarfile.DIRTYPE
                out.addfile(info)
                continue
            data = (stack_env if name == "stack.env" else ROLES if name == "db-roles.sh"
                    else RENDER if name == "bin/docker-compose-render" else ("fixture " + name + "\n").encode())
            info.size = len(data)
            out.addfile(info, io.BytesIO(data))


def write_set(release, private, anchor, version):
    key = os.path.join(private, "trust.key")
    made = subprocess.run([SSH_KEYGEN, "-q", "-t", "ed25519", "-N", "", "-C", "fixture", "-f", key], capture_output=True, text=True, check=False)
    if made.returncode != 0:
        refuse("ssh-keygen could not make the test key: " + made.stderr.strip()[:200])
    os.chmod(key, 0o600)
    pub = open(key + ".pub").read().split()
    key_type, key_b64 = pub[0], pub[1]
    fpr = subprocess.run([SSH_KEYGEN, "-l", "-f", key + ".pub"], capture_output=True, text=True, check=True).stdout.split()[1]
    tar_name = f"cortex-{version}-macos.tar.gz"
    image_name = f"cortex-server-{version}-aarch64.oci.tar"
    archive(os.path.join(release, tar_name), version)
    with open(os.path.join(release, image_name), "wb") as f:
        f.write(b"synthetic OCI archive\n")
    with open(os.path.join(release, "install.sh"), "wb") as f:
        f.write(b"#!/bin/sh\n# synthetic installer\n")
    digest = lambda name: "sha256:" + sha256(open(os.path.join(release, name), "rb").read())
    manifest = {"schema": 1, "releases": [{"version": version, "date": "2026-11-07T00:00:00Z", "notes_url": "https://example.invalid/notes", "assets": [
        {"name": tar_name, "url": tar_name, "digest": digest(tar_name)}, {"name": image_name, "url": image_name, "digest": digest(image_name)}],
        "cortex": {"label": version, "members": 3, "server_image": {"ref": SERVER, "asset": image_name}, "db_image": DATABASE,
                   "roles_script_digest": "sha256:" + sha256(ROLES), "sidecar_images": [], "migrations": [], "modules": ["audit"], "migration_class": "expand"}}]}
    with open(os.path.join(release, "manifest.json"), "w") as f:
        f.write(json.dumps(manifest, indent=2) + "\n")
    meta = {"META_SCHEMA": "1", "RELEASE_VERSION": version, "STACK_VERSION": version, "MEMBER_LIST": "3", "CORTEX_SERVER_IMAGE": SERVER, "CORTEX_DB_IMAGE": DATABASE,
            "ROLES_SCRIPT_DIGEST": sha256(ROLES)}
    with open(os.path.join(release, "install.meta"), "w") as f:
        f.write("".join(f"{k}={v}\n" for k, v in meta.items()))
    listed = [tar_name, image_name, "install.sh", "manifest.json", "install.meta"]
    sums = "".join(sha256(open(os.path.join(release, n), "rb").read()) + "  " + n + "\n" for n in listed)
    with open(os.path.join(release, "SHA256SUMS"), "w") as f:
        f.write(sums)
    signed = subprocess.run([SSH_KEYGEN, "-Y", "sign", "-q", "-f", key, "-n", NAMESPACE, os.path.join(release, "SHA256SUMS")], capture_output=True, text=True, check=False)
    if signed.returncode != 0:
        refuse("ssh-keygen could not sign SHA256SUMS: " + signed.stderr.strip()[:200])
    keyfile = os.path.join(anchor, "trust.keyfile")
    with open(keyfile, "w") as f:
        f.write(f"{PRINCIPAL} {key_type} {key_b64}\n")
    with open(os.path.join(anchor, "anchor.env"), "w") as f:
        f.write(f"PRINCIPAL={PRINCIPAL}\nKEY={key_b64}\nFPR={fpr}\nKEYFILE={keyfile}\nNAMESPACE={NAMESPACE}\nVERSION={version}\n")
    for line in (release, os.path.join(anchor, "anchor.env"), keyfile, key):
        print(line)


def main(argv):
    if len(argv) != 3:
        refuse("usage: install-fetch-fixture.py <absolute empty directory> <version>")
    where, version = argv[1], argv[2]
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", version):
        refuse("the version must be MAJOR.MINOR.PATCH (digits only)")
    if not os.path.isabs(where):
        refuse("the directory must be an absolute path")
    if not os.path.isdir(where) or os.path.realpath(where) != os.path.normpath(os.path.abspath(where)):   # a link (with a trailing slash too), or a path through one, is refused; the absolute rule above stays its own rule
        refuse("the directory must exist and be a real directory (not a link, and not reached through one)")
    if os.listdir(where):
        refuse("the directory is not empty: nothing is ever overwritten (not even an existing key file)")
    if not os.path.exists(SSH_KEYGEN):
        refuse(SSH_KEYGEN + " is required")
    release, private, anchor = (os.path.join(where, n) for n in ("release", "private", "anchor"))
    made = []
    try:
        for d, mode in ((release, 0o755), (private, 0o700), (anchor, 0o755)):
            os.mkdir(d, mode)
            made.append(d)
        write_set(release, private, anchor, version)
    except BaseException:   # a refusal (SystemExit), an exception, an interrupt: remove what THIS run made, by exact name; the directory it was given stays empty
        for d in made:
            shutil.rmtree(d, ignore_errors=True)
        raise
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
