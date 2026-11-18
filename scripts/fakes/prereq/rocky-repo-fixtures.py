#!/usr/bin/env python3
"""Construct fake Rocky repository metadata for two local fake archives."""

import gzip
import hashlib
import pathlib
import sys

directory, first, second, case = sys.argv[1:]
directory = pathlib.Path(directory)
directory.mkdir(parents=True, exist_ok=True)
sha = lambda value: hashlib.sha256(value).hexdigest()
main_name, main_version = ("curl", "8.0.0") if case == "deps_curl" else ("gnupg2", "2.4.5")
for repo, archive, name, version, release, arch in (
    ("baseos", first, main_name, main_version, "1.el10", "x86_64"),
    ("appstream", first, "", "", "", ""),
    ("crb", first, "", "", "", ""),
    ("extras", second, "epel-release", "10", "8.el10", "noarch"),
):
    checksum = sha(pathlib.Path(archive).read_bytes())
    if case == "wrong_hash" and repo == "baseos":
        checksum = "0" * 64
    entry = (f'<package type="rpm"><name>{name}</name><arch>{arch}</arch>'
             f'<version epoch="0" ver="{version}" rel="{release}"/>'
             f'<checksum type="sha256">{checksum}</checksum><location href="Packages/a/{name}.rpm"/></package>')
    if not name:
        entry = ""
    if repo == "baseos" and case.startswith("deps"):
        for suffix in ("one", "two"):
            dependency = f"synthetic dependency {suffix}\n".encode()
            entry += (f'<package type="rpm"><name>dependency-{suffix}</name><arch>noarch</arch>'
                      f'<version epoch="0" ver="1.0" rel="1.el10"/>'
                      f'<checksum type="sha256">{sha(dependency)}</checksum><location href="Packages/d/dependency-{suffix}.rpm"/></package>')
        if case == "deps_bad":
            entry = entry.replace(sha(pathlib.Path(first).read_bytes()), "0" * 64, 1)
    if case == "missing" and repo == "extras":
        entry = ""
    primary = f'<metadata xmlns="http://linux.duke.edu/metadata/common">{entry}</metadata>'.encode()
    packed = gzip.compress(primary, mtime=0)
    packed_hash = sha(packed)
    if case == "primary_bad" and repo == "baseos":
        packed += b"changed"
    repomd = (f'<repomd xmlns="http://linux.duke.edu/metadata/repo"><data type="primary">'
              f'<checksum type="sha256">{packed_hash}</checksum><open-checksum type="sha256">{sha(primary)}</open-checksum>'
              f'<location href="repodata/primary.xml.gz"/></data></repomd>').encode()
    (directory / f"{repo}-repomd").write_bytes(repomd)
    (directory / f"{repo}-signature").write_bytes(b"synthetic metadata signature\n")
    (directory / f"{repo}-primary").write_bytes(packed)
