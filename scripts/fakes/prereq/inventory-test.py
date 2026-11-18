#!/usr/bin/env python3
"""V-05: synthetic filesystem inventory detects type, mode, link and HOME changes."""
import importlib.util
import os
from pathlib import Path
import sys
import tempfile

sys.dont_write_bytecode = True
root = Path(sys.argv[1])
helper = root / 'scripts/fakes/prereq/filesystem-inventory.py'
if not helper.is_file():
    raise SystemExit('FAIL V-05: filesystem inventory helper missing')
spec = importlib.util.spec_from_file_location('prereq_inventory', helper)
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)
with tempfile.TemporaryDirectory(prefix='prereq-inventory-', dir=os.environ['TMPDIR']) as tmp:
    base = Path(tmp)
    roots = [base / name for name in ('tree', 'state', 'home')]
    for path in roots:
        path.mkdir()
    file = roots[0] / 'file'; file.write_text('old'); file.chmod(0o644)
    link = roots[1] / 'link'; link.symlink_to('first')
    original = mod.inventory(roots)
    # Substitute only one lstat result; no host identity or other path changes.
    real_lstat = Path.lstat
    class OwnerView:
        def __init__(self, info, field):
            self.info = info
            self.field = field
        def __getattr__(self, name):
            value = getattr(self.info, name)
            return value + 1 if name == self.field else value
    for field in ('st_uid', 'st_gid'):
        def changed_lstat(path, *args, **kwargs):
            info = real_lstat(path, *args, **kwargs)
            return OwnerView(info, field) if path == file else info
        try:
            Path.lstat = changed_lstat
            assert mod.inventory(roots) != original, f'{field} owner change not detected'
        finally:
            Path.lstat = real_lstat
    # Exercise a real change on the disposable fake file when a second group
    # is available. The owner stays unchanged and the group is always restored.
    original_gid = real_lstat(file).st_gid
    alternate_groups = sorted(set(os.getgroups() + [os.getgid()]) - {original_gid})
    if alternate_groups:
        try:
            os.chown(file, -1, alternate_groups[0])
        except PermissionError:
            print('SKIP V-05: filesystem refused disposable group change')
        else:
            try:
                assert mod.inventory(roots) != original, 'real fake-file group change not detected'
            finally:
                os.chown(file, -1, original_gid)
    else:
        print('SKIP V-05: no alternate group for disposable file')
    file.write_text('new')
    assert mod.inventory(roots) != original, 'content not detected'
    file.write_text('old')
    file.chmod(0o600)
    assert mod.inventory(roots) != original, 'mode not detected'
    file.chmod(0o644)
    link.unlink(); link.symlink_to('second')
    assert mod.inventory(roots) != original, 'link target not detected'
    link.unlink(); link.symlink_to('first')
    (roots[2] / 'created').mkdir()
    assert mod.inventory(roots) != original, 'HOME directory not detected'
    (roots[2] / 'created').rmdir()
    file.unlink(); file.mkdir()
    assert mod.inventory(roots) != original, 'file type not detected'
print('PASS V-05: uid, gid, content, modes, links, types and HOME changes detected')
