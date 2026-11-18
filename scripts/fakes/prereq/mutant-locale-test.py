#!/usr/bin/env python3
"""V-03: exercise mutation baseline selection using stubs only."""
import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

root = Path(sys.argv[1])
spec = importlib.util.spec_from_file_location('prereq_mutants_under_test', root / 'scripts/install_prereq_mutants.py')
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)
with tempfile.TemporaryDirectory(prefix='mutant-locales-', dir=os.environ['TMPDIR']) as td:
    scratch = Path(td)
    mod.SCRATCH = scratch / 'run'
    mod.SOURCE = scratch / 'source.sh'
    mod.SOURCE.write_text('one\n', encoding='utf-8')
    mod.SPEC = scratch / 'mutants.json'
    mod.SPEC.write_text(json.dumps([{'id': 'T1', 'what': 'synthetic', 'old': 'one', 'new': 'two', 'fails': 'FAIL EXPECTED'}]), encoding='utf-8')
    real_run = mod.subprocess.run
    def fake_run(args, **kwargs):
        if args[0] == 'locale':
            return subprocess.CompletedProcess(args, 0, 'UTF-8\n', '')
        if args[:2] == ['sh', '-n']:
            return subprocess.CompletedProcess(args, 0, '', '')
        raise AssertionError(args)
    # Verify the real driver launches complete matrices in the two orders.
    launched = []
    def fake_suite_run(args, **kwargs):
        if args[0] != mod.ADMIT:
            raise AssertionError(args)
        launched.append(kwargs['env']['PREREQ_TEST_LOCALES'])
        return subprocess.CompletedProcess(args, 0, 'ok', '')
    mod.subprocess.run = fake_suite_run
    mod.suite(mod.SOURCE, 'C')
    mod.suite(mod.SOURCE, 'C.utf8')
    if launched != ['C C.utf8', 'C.utf8 C']:
        raise SystemExit(f'FAIL V-03: suite did not request full ordered matrices: {launched}')
    mod.subprocess.run = fake_run
    sys.argv = ['runner', 'T1']
    calls = []
    def passing_suite(source, locale=None, named=None):
        calls.append((source == mod.SOURCE, locale, named))
        return (0, 'ok locale (C)\nok locale (C.utf8)\n') if source == mod.SOURCE else (1, 'FAIL EXPECTED\n')
    mod.suite = passing_suite
    with contextlib.redirect_stdout(io.StringIO()):
        status = mod.main()
    baseline = [(locale, named) for original, locale, named in calls if original]
    mutants = [(locale, named) for original, locale, named in calls if not original]
    expected_name = 'T1' if os.environ.get('PREREQ_MUTANT_FAST') == '1' else None
    if status != 0 or baseline != [('C', None), ('C.utf8', None)]:
        raise SystemExit('FAIL V-03: baseline did not execute once per selected locale')
    if mutants != [('C', expected_name), ('C.utf8', expected_name)]:
        raise SystemExit('FAIL V-03: mutation did not use the requested screen mode in both locales')
    def skipped_suite(source, locale=None, named=None):
        return (0, f'SKIP IP-53: locale {locale} unavailable\n') if source == mod.SOURCE else (1, 'FAIL EXPECTED\n')
    mod.suite = skipped_suite
    with contextlib.redirect_stdout(io.StringIO()):
        status = mod.main()
    if status != 2:
        raise SystemExit('FAIL V-03: a baseline with zero executed locale blocks was accepted')
    def partial_suite(source, locale=None, named=None):
        return (0, 'ok locale (C)\n') if source == mod.SOURCE else (1, 'FAIL EXPECTED\n')
    mod.suite = partial_suite
    with contextlib.redirect_stdout(io.StringIO()):
        status = mod.main()
    if status != 2:
        raise SystemExit('FAIL V-03: a partial baseline was accepted')
    mod.subprocess.run = real_run
print('PASS V-03: each mutation locale has an executed baseline and empty baseline is refused')
