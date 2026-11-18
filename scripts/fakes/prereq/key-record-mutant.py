#!/usr/bin/env python3
"""Remove the local key-record status check and require the fake cell to fail."""
import os
import pathlib
import subprocess
import sys

if len(sys.argv) != 3:
    raise SystemExit('usage: key-record-mutant.py INCLUDE RECEIPT_DIR')
source, receipt = map(lambda x: pathlib.Path(x).resolve(), sys.argv[1:])
if str(receipt) == '/tmp' or str(receipt).startswith('/tmp/'):
    raise SystemExit('refusing /tmp')
receipt.mkdir(parents=True, exist_ok=True)
test = pathlib.Path(__file__).with_name('key-record-output-test.sh').resolve()
lines = source.read_text().splitlines(keepends=True)
sites = [i for i, line in enumerate(lines) if 'pq_key_records=$(printf ' in line and ' || return 1' in line]
if len(sites) != 1:
    raise SystemExit(f'FAIL F-7 key-record mutant: expected 1 guarded write, found {len(sites)}')
base = subprocess.run(['sh', str(test), str(source)], capture_output=True, text=True, env=os.environ)
(receipt / 'baseline.log').write_text(base.stdout + base.stderr)
if base.returncode:
    raise SystemExit(f'FAIL F-7 key-record mutant: baseline returned {base.returncode}')
lines[sites[0]] = lines[sites[0]].replace(' || return 1', '', 1)
mutant = receipt / 'mutant.sh'
mutant.write_text(''.join(lines))
result = subprocess.run(['sh', str(test), str(mutant)], capture_output=True, text=True, env=os.environ)
(receipt / 'mutant.log').write_text(result.stdout + result.stderr)
if result.returncode == 0 or 'FAIL F-7 key records:' not in result.stderr:
    raise SystemExit('FAIL F-7 key-record mutant survived or fixture failed')
print('PASS F-7 key-record mutant: 1/1 caught')
