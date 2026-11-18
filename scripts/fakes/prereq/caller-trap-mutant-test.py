#!/usr/bin/env python3
"""The real caller-trap cell must pass clean code and kill a leaking mutant."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile

root = Path(__file__).resolve().parents[3]
scratch = Path(os.environ['TMPDIR']).resolve()
if scratch == Path('/tmp') or Path('/tmp') in scratch.parents:
    raise SystemExit('private scratch outside /tmp required')
source = (root / 'deploy/install-prereq.sh').read_text()
mutations = (
    ('machine EXIT', 'machine_look() {\n  prereq_secure_call machine_look_body\n}',
     "machine_look() {\n  trap ':' EXIT\n  prereq_secure_call machine_look_body\n}",
     'FAIL W1-109: machine_look'),
    ('prereq HUP', 'prereq_run() {\n',
     "prereq_run() {\n  trap '' HUP\n",
     'FAIL W1-109: prereq_run'),
)
helper = root / 'scripts/fakes/prereq/caller-trap-cell.sh'
if not helper.is_file():
    raise SystemExit('FAIL W1-109: caller-trap cell is not present')
with tempfile.TemporaryDirectory(prefix='caller-trap-mutant-', dir=scratch) as td:
    candidates = [('baseline', root / 'deploy/install-prereq.sh', None)]
    for index, (label, old, new, expected) in enumerate(mutations):
        if source.count(old) != 1:
            raise SystemExit(f'FAIL W1-109: {label} mutant anchor is not unique')
        mutant = Path(td) / f'mutant-{index}.sh'
        mutant.write_text(source.replace(old, new, 1))
        candidates.append((label, mutant, expected))
    for locale in ('C', 'C.utf8'):
        for label, include, expected in candidates:
            env = dict(os.environ, TMPDIR=td, LC_ALL=locale)
            result = subprocess.run(['sh', str(helper), str(include), str(root)], env=env,
                                    stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, check=False)
            if expected is None and result.returncode:
                raise SystemExit(f'FAIL W1-109: baseline ({locale})\n{result.stdout}')
            if expected is not None and (result.returncode == 0 or expected not in result.stdout):
                raise SystemExit(f'FAIL W1-109: {label} mutant survived or wrong test ({locale})\n{result.stdout}')
print('PASS W1-109: caller traps retained; EXIT and HUP replacement mutants caught in both locales')
