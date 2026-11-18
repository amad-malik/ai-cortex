#!/usr/bin/env python3
"""Inventory fake roots without following symlinks or touching their contents."""
import hashlib
import json
import os
from pathlib import Path
import stat
import sys


def inventory(roots):
    entries = []

    def visit(root_index, path, relative):
        info = path.lstat()
        entry = {'root': root_index, 'path': relative, 'uid': info.st_uid, 'gid': info.st_gid, 'mode': stat.S_IMODE(info.st_mode)}
        if stat.S_ISDIR(info.st_mode):
            entry['type'] = 'directory'
        elif stat.S_ISREG(info.st_mode):
            entry['type'] = 'file'
            digest = hashlib.sha256()
            with path.open('rb') as stream:
                for chunk in iter(lambda: stream.read(65536), b''):
                    digest.update(chunk)
            entry['sha256'] = digest.hexdigest()
        elif stat.S_ISLNK(info.st_mode):
            entry['type'] = 'symlink'
            entry['target'] = os.readlink(path)
        else:
            entry['type'] = f'other:{stat.S_IFMT(info.st_mode)}'
            entry['device'] = info.st_rdev
        entries.append(entry)
        if stat.S_ISDIR(info.st_mode):
            with os.scandir(path) as children:
                names = sorted(child.name for child in children)
            for name in names:
                visit(root_index, path / name, f'{relative}/{name}' if relative else name)

    for index, root in enumerate(roots):
        visit(index, Path(root), '')
    return entries


if __name__ == '__main__':
    print(json.dumps(inventory(sys.argv[1:]), ensure_ascii=True, sort_keys=True))
