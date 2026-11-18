#!/usr/bin/env python3
"""Build small, non-executable newc archives for the private bootstrap tests."""
import stat
import sys
from pathlib import Path

out, case = Path(sys.argv[1]), sys.argv[2]
entries = [
    ("./etc/pki/rpm-gpg/RPM-GPG-KEY-EPEL-10", b"synthetic pinned key\n", stat.S_IFREG | 0o644),
    ("./etc/yum.repos.d/epel.repo", b"[epel]\nmetalink=stable\n", stat.S_IFREG | 0o644),
]
if case == "duplicate":
    entries.append(entries[0])
elif case == "symlink":
    entries[0] = (entries[0][0], b"/etc/shadow", stat.S_IFLNK | 0o777)
elif case == "missing":
    entries.pop()
elif case == "traversal":
    entries[1] = ("./etc/yum.repos.d/../epel.repo", entries[1][1], entries[1][2])

def member(name, content, mode):
    name = name.encode() + b"\0"
    fields = (1, mode, 0, 0, 1, 0, len(content), 0, 0, 0, 0, len(name), 0)
    header = b"070701" + b"".join(f"{field:08x}".encode() for field in fields)
    result = header + name
    result += b"\0" * (-len(result) % 4)
    result += content
    result += b"\0" * (-len(result) % 4)
    return result

out.write_bytes(b"".join(member(*entry) for entry in entries) + member("TRAILER!!!", b"", 0))
