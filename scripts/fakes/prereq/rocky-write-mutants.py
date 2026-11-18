#!/usr/bin/env python3
"""Remove each Rocky receipt say guard, one at a time, and run failure cells."""
import os
import pathlib
import subprocess
import sys

if len(sys.argv) != 3:
    raise SystemExit('usage: rocky-write-mutants.py INCLUDE RECEIPT_DIR')
source = pathlib.Path(sys.argv[1]).resolve()
receipt = pathlib.Path(sys.argv[2]).resolve()
if str(receipt) == '/tmp' or str(receipt).startswith('/tmp/'):
    raise SystemExit('refusing /tmp')
receipt.mkdir(parents=True, exist_ok=True)
test = pathlib.Path(__file__).with_name('rocky-output-failure-test.sh').resolve()
lines = source.read_text().splitlines(keepends=True)
start = next(i for i, line in enumerate(lines) if line == 'prereq_rocky_install() {\n')
end = next(i for i in range(start + 1, len(lines)) if lines[i] == '}\n')
guard = ' || return "${EX_PREREQ:-3}"'
sites = [i for i in range(start, end) if 'say ' in lines[i] and guard in lines[i]]
if len(sites) != 4:
    raise SystemExit(f'FAIL F-7 Rocky mutants: expected 4 guarded say sites, found {len(sites)}')
base = subprocess.run(['sh', str(test), str(source)], text=True, capture_output=True, env=os.environ)
(receipt / 'baseline.log').write_text(base.stdout + base.stderr)
if base.returncode:
    raise SystemExit(f'FAIL F-7 Rocky mutants: baseline returned {base.returncode}')
for number, site in enumerate(sites, 1):
    mutant = list(lines)
    mutant[site] = mutant[site].replace(guard, '', 1)
    path = receipt / f'mutant-{number:02}.sh'
    path.write_text(''.join(mutant))
    result = subprocess.run(['sh', str(test), str(path)], text=True, capture_output=True, env=os.environ)
    (receipt / f'mutant-{number:02}.log').write_text(result.stdout + result.stderr)
    if result.returncode == 0 or 'FAIL F-7 r4:' not in result.stderr:
        raise SystemExit(f'FAIL F-7 Rocky mutants: site {number} line {site+1} survived or test failed unexpectedly')
    print(f'caught Rocky output mutant {number:02} at original line {site+1}')
print(f'PASS F-7 Rocky mutants: {len(sites)}/{len(sites)} caught')
