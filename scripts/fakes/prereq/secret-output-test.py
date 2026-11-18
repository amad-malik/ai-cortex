#!/usr/bin/env python3
"""V-07: synthetic secret markers stay out of public output and test logs."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile

root = Path(sys.argv[1]); source = Path(sys.argv[2])
suite = (root / 'scripts/install-prereq-test.sh').read_text()
if 'secret-output-test.py' not in suite:
    raise SystemExit('FAIL V-07: prereq_run and say_done secret test is not wired')
markers = ['synthetic-secret-token-V07', 'synthetic-secret-password-V07', 'synthetic-secret-key-V07']
with tempfile.TemporaryDirectory(prefix='prereq-secret-', dir=os.environ['TMPDIR']) as directory:
    work = Path(directory)
    script = r'''. "$1"
say() { printf "%s\n" "$*"; printf "%s\n" "$*" >> "$TEST_LOG"; }
warn() { printf "%s\n" "$*" >&2; }
prereq_compose_path() { printf "%s\n" /synthetic/provider; }
PREREQ_MISSING='' PREREQ_FIXES='' PREREQ_COMPOSE=/synthetic/provider
prereq_run || exit $?
say_done 1.2.3 http://127.0.0.1:8601 /synthetic/token-file /synthetic/log
'''
    env = dict(os.environ, TEST_TOKEN=markers[0], TEST_PASSWORD=markers[1], TEST_KEY=markers[2], TEST_LOG=str(work / 'log'))
    def leaks(path):
        (work / 'log').write_text('')
        result = subprocess.run(['sh', '-c', script, 'sh', str(path)], env=env, capture_output=True, text=True, check=False)
        output = result.stdout + result.stderr + (work / 'log').read_text()
        return result.returncode, any(marker in output for marker in markers)
    rc, found = leaks(source)
    if rc or found:
        raise SystemExit('FAIL V-07: public prerequisite output exposed a synthetic marker or failed')
    original = source.read_text()
    needle = 'say_done() {'
    if original.count(needle) != 1:
        raise SystemExit('FAIL V-07: positive-control insertion point changed')
    mutated = work / 'injected.sh'
    mutated.write_text(original.replace(needle, needle + '\n  printf "%s\\n" "$TEST_PASSWORD"', 1))
    rc, found = leaks(mutated)
    if rc or not found:
        raise SystemExit('FAIL V-07: positive control did not detect the synthetic leak')
print('PASS V-07: prereq_run and say_done output/logs exclude synthetic markers; positive control detected')
