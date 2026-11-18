#!/usr/bin/env python3
"""Test removal of the two provider-output status mappings."""
import os
import pathlib
import subprocess
import sys

if len(sys.argv) != 3:
    raise SystemExit('usage: requal-provider-mutants.py INCLUDE RECEIPT_DIR')
source, receipt = map(lambda x: pathlib.Path(x).resolve(), sys.argv[1:])
if str(receipt) == '/tmp' or str(receipt).startswith('/tmp/'):
    raise SystemExit('refusing /tmp')
receipt.mkdir(parents=True, exist_ok=True)
test = pathlib.Path(__file__).with_name('requal-provider-output-test.sh').resolve()
original = source.read_text()
base = subprocess.run(['sh', str(test), str(source)], capture_output=True, text=True, env=os.environ)
(receipt / 'baseline.log').write_text(base.stdout + base.stderr)
if base.returncode:
    raise SystemExit(f'FAIL F-7 provider mutants: baseline {base.returncode}')
cases = [
    ('inner', 'prereq_requalify_compose_inner() {', 'prereq_requalify_compose_inner_body() {',
     ' || exit "${EX_PREREQ:-3}"'),
    ('path', 'prereq_requalify_compose_body() {', '# Translate only the exact package names',
     ' || return "${EX_PREREQ:-3}"'),
]
for name, start, stop, guard in cases:
    a = original.index(start)
    b = original.index(stop, a)
    chunk = original[a:b]
    lines = chunk.splitlines(keepends=True)
    sites = [i for i, line in enumerate(lines) if "printf '%s\\n'" in line and guard in line]
    if len(sites) != 1:
        raise SystemExit(f'FAIL F-7 provider mutants: {name} found {len(sites)} sites')
    lines[sites[0]] = lines[sites[0]].replace(guard, '', 1)
    mutant = receipt / f'{name}.sh'
    mutant.write_text(original[:a] + ''.join(lines) + original[b:])
    result = subprocess.run(['sh', str(test), str(mutant)], capture_output=True, text=True, env=os.environ)
    (receipt / f'{name}.log').write_text(result.stdout + result.stderr)
    if result.returncode == 0 or 'FAIL F-7 provider write:' not in result.stderr:
        raise SystemExit(f'FAIL F-7 provider mutant survived or fixture failed: {name}')
    print(f'caught provider output mutant: {name}')
print('PASS F-7 provider mutants: 2/2 caught')
