#!/usr/bin/env python3
"""Create structurally recognizable synthetic OpenPGP roots for fake tests."""

import base64
import hashlib
import pathlib
import sys

directory = pathlib.Path(sys.argv[1])
directory.mkdir(parents=True, exist_ok=True)
fingerprints = []
for number in (1, 2):
    body = bytes([4, 0, 0, 0, number, 1, 0, 1, number])
    packet = bytes([0xC6, len(body)]) + body
    armor = b"-----BEGIN PGP PUBLIC KEY BLOCK-----\n\n" + base64.b64encode(packet) + b"\n-----END PGP PUBLIC KEY BLOCK-----\n"
    (directory / f"root{number}.asc").write_bytes(armor)
    fingerprints.append(hashlib.sha1(b"\x99" + len(body).to_bytes(2, "big") + body).hexdigest().upper())
(directory / "fingerprints").write_text("\n".join(fingerprints) + "\n", encoding="ascii")
