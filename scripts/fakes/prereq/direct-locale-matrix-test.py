#!/usr/bin/env python3
"""VR3-01: direct suite refuses skipped or incomplete requested matrices.

Run separately, not from the suite: this calls the suite itself. It checks the
reason as well as status so the known unwired IP-check failure cannot fake a
pass. All suite side effects remain in the fake test scratch tree.
"""
import os
from pathlib import Path
import subprocess
import sys

root = Path(__file__).resolve().parents[3]
suite = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else root / 'scripts/install-prereq-test.sh'
scratch = Path(os.environ['TMPDIR']).resolve()
if scratch == Path('/tmp') or Path('/tmp') in scratch.parents or scratch == Path('/var/tmp') or Path('/var/tmp') in scratch.parents:
    raise SystemExit('refusing temp under /tmp')
for label, requested in (
    ('all-invalid', 'does_not_exist_helix_locale'),
    ('missing-utf8', 'C'),
    ('extra-invalid', 'C C.utf8 does_not_exist_helix_locale'),
):
    env = dict(os.environ, TMPDIR=str(scratch), PREREQ_TEST_TMP=str(scratch), PREREQ_TEST_LOCALES=requested)
    result = subprocess.run(['sh', str(suite)], cwd=root, env=env, capture_output=True, text=True, timeout=1200, check=False)
    output = result.stdout + result.stderr
    if result.returncode == 0 or 'FAIL IP-53: locale matrix' not in output:
        print(f'FAIL VR3-01: {label} returned {result.returncode} without the matrix refusal', file=sys.stderr)
        print(output[-3500:], file=sys.stderr)
        raise SystemExit(1)
    print(f'PASS VR3-01: direct suite rejects {label} (status {result.returncode})', flush=True)
