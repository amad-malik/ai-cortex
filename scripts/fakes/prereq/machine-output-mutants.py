#!/usr/bin/env python3
"""Mutate cause, fix, diagnostic and SELinux checks one at a time."""
import os
import pathlib
import subprocess
import sys

if len(sys.argv) != 3:
    raise SystemExit('usage: machine-output-mutants.py INCLUDE RECEIPT_DIR')
source, receipt = map(lambda x: pathlib.Path(x).resolve(), sys.argv[1:])
if str(receipt) == '/tmp' or str(receipt).startswith('/tmp/'):
    raise SystemExit('refusing /tmp')
receipt.mkdir(parents=True, exist_ok=True)
folder = pathlib.Path(__file__).parent.resolve()
lines = source.read_text().splitlines(keepends=True)
guard = ' || return "${EX_PREREQ:-3}"'
cases = [
    ('problem', "printf 'problem: %s\\n'", 'refusal-output-test.sh'),
    ('fix', "printf 'fix: %s\\n'", 'refusal-output-test.sh'),
    ('machine-refusal', 'prereq_msg compose_missing', 'machine-refusal-output-test.sh'),
    ('selinux', 'prereq_info selinux ', 'output-failure-test.sh'),
]
for name, selector, test_name in cases:
    test = folder / test_name
    base = subprocess.run(['sh', str(test), str(source)], capture_output=True, text=True, env=os.environ)
    (receipt / f'{name}-baseline.log').write_text(base.stdout + base.stderr)
    if base.returncode:
        raise SystemExit(f'FAIL F-7 machine mutants: {name} baseline {base.returncode}')
    matches = [i for i, line in enumerate(lines) if selector in line and guard in line]
    if len(matches) != 1:
        raise SystemExit(f'FAIL F-7 machine mutants: {name} found {len(matches)} sites')
    mutant = list(lines)
    mutant[matches[0]] = mutant[matches[0]].replace(guard, '', 1)
    path = receipt / f'{name}.sh'
    path.write_text(''.join(mutant))
    result = subprocess.run(['sh', str(test), str(path)], capture_output=True, text=True, env=os.environ)
    (receipt / f'{name}.log').write_text(result.stdout + result.stderr)
    if result.returncode == 0 or 'FAIL F-7' not in result.stderr:
        raise SystemExit(f'FAIL F-7 machine mutant survived or fixture failed: {name}')
    print(f'caught machine output mutant: {name}')
print('PASS F-7 machine mutants: 4/4 caught')
