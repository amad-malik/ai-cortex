#!/usr/bin/env python3
"""VR3-01: remove the required-pair guard and require the direct cell to fail."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile

root = Path(__file__).resolve().parents[3]
source = root / 'scripts/install-prereq-test.sh'
test = root / 'scripts/fakes/prereq/direct-locale-matrix-test.py'
scratch = Path(os.environ['TMPDIR']).resolve()
if scratch == Path('/tmp') or Path('/tmp') in scratch.parents or scratch == Path('/var/tmp') or Path('/var/tmp') in scratch.parents:
    raise SystemExit('refusing temp under /tmp')
base = subprocess.run([sys.executable, '-I', '-B', str(test), str(source)], cwd=root, text=True, capture_output=True, check=False)
if base.returncode:
    print(base.stdout + base.stderr, file=sys.stderr)
    raise SystemExit('FAIL VR3-01 mutant: baseline did not pass')
text = source.read_text()
old = 'if [ "$pq_test_has_c" -ne 1 ] || [ "$pq_test_has_utf8" -ne 1 ]; then'
new = 'if false; then'
root_line = 'root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)'
if text.count(old) != 1 or text.count(root_line) != 1:
    raise SystemExit('FAIL VR3-01 mutant: source anchors changed')
with tempfile.TemporaryDirectory(prefix='locale-matrix-mutant-', dir=scratch) as directory:
    mutant = Path(directory) / 'mutant-suite.sh'
    mutant.write_text(text.replace(old, new, 1).replace(root_line, f"root='{root}'", 1))
    result = subprocess.run([sys.executable, '-I', '-B', str(test), str(mutant)], cwd=root, text=True,
                            capture_output=True, timeout=1800, check=False)
    if result.returncode == 0 or 'FAIL VR3-01: missing-utf8' not in result.stderr:
        print(result.stdout + result.stderr, file=sys.stderr)
        raise SystemExit('FAIL VR3-01 mutant: guard removal survived or failed at wrong cell')
print('PASS VR3-01: missing-required-locale guard mutant caught by direct-suite cell')
