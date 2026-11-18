#!/usr/bin/env python3
"""VR3-02: remove UID and GID separately and require owner cells to fail."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile

root = Path(__file__).resolve().parents[3]
# When committed, this script is scripts/fakes/prereq/<name>.
source = root / 'scripts/fakes/prereq/filesystem-inventory.py'
test = root / 'scripts/fakes/prereq/inventory-test.py'
scratch = Path(os.environ['TMPDIR']).resolve()
if scratch == Path('/tmp') or Path('/tmp') in scratch.parents or scratch == Path('/var/tmp') or Path('/var/tmp') in scratch.parents:
    raise SystemExit('refusing /tmp')
base = subprocess.run([sys.executable, '-I', '-B', str(test), str(root)], text=True, capture_output=True, check=False)
if base.returncode:
    print(base.stdout + base.stderr, file=sys.stderr)
    raise SystemExit('FAIL VR3-02: owner-cell baseline failed')
text = source.read_text()
with tempfile.TemporaryDirectory(prefix='inventory-owner-mutants-', dir=scratch) as directory:
    fixture = Path(directory)
    target = fixture / 'scripts/fakes/prereq/filesystem-inventory.py'
    target.parent.mkdir(parents=True)
    for field in ('uid', 'gid'):
        token = f"'{field}': info.st_{field}, "
        if text.count(token) != 1:
            raise SystemExit(f'FAIL VR3-02: {field} mutation anchor absent or duplicated')
        target.write_text(text.replace(token, '', 1))
        result = subprocess.run([sys.executable, '-I', '-B', str(test), str(fixture)], text=True,
                                capture_output=True, check=False)
        if result.returncode == 0 or f'st_{field} owner change not detected' not in result.stderr:
            print(result.stdout + result.stderr, file=sys.stderr)
            raise SystemExit(f'FAIL VR3-02: {field} mutant survived or failed for another reason')
        print(f'PASS VR3-02: {field} omission mutant caught', flush=True)
print('PASS VR3-02: 2/2 owner mutants caught')
