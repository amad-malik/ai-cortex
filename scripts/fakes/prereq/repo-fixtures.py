#!/usr/bin/env python3
"""Create synthetic repository responses entirely inside the test scratch."""

import gzip
import hashlib
import pathlib
import sys

directory = pathlib.Path(sys.argv[1])
case = sys.argv[2]
package_name = sys.argv[3] if len(sys.argv) > 3 else "podman-compose"
directory.mkdir(parents=True, exist_ok=True)
sha = lambda value: hashlib.sha256(value).hexdigest()
package = b"synthetic rpm: never executed\n"
metadata_digest = sha(package)
if case == "package_mismatch":
    package += b"changed after metadata\n"
epoch = "1" if case == "wrong_epoch" else "0"
href = "../untrusted.rpm" if case == "path_escape" else "Packages/p/podman-compose.rpm"
entry = (f'<package type="rpm"><name>{package_name}</name><arch>noarch</arch>'
         f'<version epoch="{epoch}" ver="1.5.0" rel="1.el10_1"/>'
         f'<checksum type="sha256">{metadata_digest}</checksum><location href="{href}"/></package>')
primary = (f'<metadata xmlns="http://linux.duke.edu/metadata/common">{entry}'
           f'{entry if case == "duplicate" else ""}</metadata>').encode()
primary_gzip = gzip.compress(primary, mtime=0)
primary_digest = sha(primary_gzip)
if case == "primary_mismatch":
    primary_gzip += b"corrupt"
repomd = (f'<repomd xmlns="http://linux.duke.edu/metadata/repo"><data type="primary">'
          f'<checksum type="sha256">{primary_digest}</checksum>'
          f'<open-checksum type="sha256">{sha(primary)}</open-checksum>'
          f'<location href="repodata/primary.xml.gz"/></data></repomd>').encode()
repomd_digest = sha(repomd)
if case == "repomd_mismatch":
    repomd += b" \n"
mirror = "http://mirror.example" if case == "http_mirror" else "https://mirror.example"
metalink = (f'<metalink xmlns="urn:ietf:params:xml:ns:metalink"><files><file name="repomd.xml">'
            f'<verification><hash type="sha256">{repomd_digest}</hash></verification>'
            f'<resources><url>{mirror}/epel/repodata/repomd.xml</url></resources>'
            f'</file></files></metalink>').encode()
for name, contents in (("metalink", metalink), ("repomd", repomd), ("signature", b"synthetic signature\n"), ("primary", primary_gzip), ("package", package)):
    (directory / name).write_bytes(contents)
