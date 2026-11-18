#!/usr/bin/env python3
"""Remove each direct-output guard individually; the lint must reject it.

These are static lint mutants, not substitutes for the behavioral /dev/full
mutants run separately for requalification, provider, machine and Rocky writes.
"""
import importlib.util
import os
import pathlib
import re
import sys
import tempfile

if len(sys.argv) != 2:
    raise SystemExit('usage: output-lint-mutants.py INCLUDE')
source = pathlib.Path(sys.argv[1]).resolve()
scratch = pathlib.Path(os.environ['TMPDIR']).resolve()
if str(scratch) == '/tmp' or str(scratch).startswith('/tmp/'):
    raise SystemExit('refusing /tmp')
path = pathlib.Path(__file__).with_name('output-lint.py')
spec = importlib.util.spec_from_file_location('output_lint', path)
lint = importlib.util.module_from_spec(spec)
spec.loader.exec_module(lint)
if lint.audit(source, set()):
    raise SystemExit('FAIL output lint mutants: baseline fails')
lines = source.read_text().splitlines(keepends=True)
current = None
sites = []
remove = re.compile(r'\|\|\s*(?:return|exit)\s+(?:"[^"\n]*"|\d+)')
for i, line in enumerate(lines):
    start = lint.START.match(line)
    if start:
        current = start.group(1) if start.group(1) in lint.SCOPES else None
    if current:
        masked = lint.mask(line)
        for writer in lint.WRITER.finditer(masked):
            before = masked[:writer.start()]
            if '$(' in before and ')' not in before.split('$(')[-1]:
                continue
            stop = masked.find(';', writer.end())
            stop = len(masked) if stop < 0 else stop
            guard = remove.search(line, writer.end(), stop)
            if guard:
                sites.append((i, guard.start(), guard.end(), current, writer.group().strip()))
        if line == '}\n':
            current = None
if not sites:
    raise SystemExit('FAIL output lint mutants: no sites found')
with tempfile.TemporaryDirectory(dir=scratch, prefix='output-lint-mutants-') as directory:
    target = pathlib.Path(directory) / 'mutant.sh'
    for index, (line_no, start, end, scope, writer) in enumerate(sites, 1):
        mutant = list(lines)
        mutant[line_no] = mutant[line_no][:start] + mutant[line_no][end:]
        target.write_text(''.join(mutant))
        if not lint.audit(target, set()):
            raise SystemExit(f'FAIL output lint mutant survived: {scope} line {line_no+1} {writer}')
        print(f'caught static lint mutant {index:02}: {scope} line {line_no+1} {writer}')
print(f'PASS output lint mutants: {len(sites)}/{len(sites)} caught')
