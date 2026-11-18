#!/usr/bin/env python3
"""Prove fast screens pass the requested locale to their real children."""
import importlib.util
import os
from pathlib import Path
import shlex
import shutil
import tempfile

root = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location('locale_probe_driver', root / 'scripts/install_prereq_mutants.py')
driver = importlib.util.module_from_spec(spec)
spec.loader.exec_module(driver)
tmp = Path(os.environ['TMPDIR']).resolve()
if tmp == Path('/tmp') or Path('/tmp') in tmp.parents:
    raise SystemExit('private scratch outside /tmp required')

with tempfile.TemporaryDirectory(prefix='child-locale-', dir=tmp) as td:
    home = Path(td)
    clone = home / 'clone'
    (clone / 'scripts').mkdir(parents=True)
    (clone / 'deploy').mkdir()
    shutil.copy2(root / 'deploy/install.sh', clone / 'deploy/install.sh')
    shutil.copy2(root / 'scripts/install-prereq-test.sh', clone / 'scripts/install-prereq-test.sh')
    shutil.copytree(root / 'scripts/fakes/prereq', clone / 'scripts/fakes/prereq')
    driver.TEST = clone / 'scripts/install-prereq-test.sh'
    driver.SCRATCH = home / 'run'
    driver.SCRATCH.mkdir()
    probe = home / 'observed.txt'
    include = home / 'include.sh'
    destination = shlex.quote(str(probe))
    prefix = f'''\
case ${{PREREQ_SCREEN_MUTANT-}} in
  IPM-28) printf 'source:%s\\n' "${{LC_ALL-unset}}" >> {destination} ;;
  IPM-72) printf 'linger:%s\\n' "${{LC_ALL-unset}}" >> {destination} ;;
esac
if [ -n "${{SIGNAL_READY-}}" ]; then printf 'signal:%s\\n' "${{LC_ALL-unset}}" >> {destination}; fi
case ${{CORTEX_YES-unset}} in
  0|1) if [ -n "${{PREREQ_ROOT-}}" ]; then printf 'consent:%s\\n' "${{LC_ALL-unset}}" >> {destination}; fi ;;
esac
'''
    include.write_text(prefix + (root / 'deploy/install-prereq.sh').read_text())

    def check(ident, label, requested, expected, source=include):
        probe.unlink(missing_ok=True)
        rc, output = driver.suite(source, requested, named=ident)
        observed = []
        if probe.exists():
            observed = [line.split(':', 1)[1] for line in probe.read_text().splitlines()
                        if line.startswith(label + ':')]
        valid = rc == 0 and f'SCREEN {ident}: survived' in output.splitlines()
        valid = valid and bool(observed) and all(value == expected for value in observed)
        print(f'{ident} requested={requested} expected={expected} observed={observed} rc={rc} valid={valid}', flush=True)
        return valid

    # The outer shell can exit before the inner shell's synchronous EXIT cleanup.
    # Delay that real cleanup, without replacing its rm or retrying a failed screen.
    delayed = home / 'delayed-cleanup.sh'
    completed = home / 'cleanup-completed'
    delayed.write_text(include.read_text() + '\nrm() {\n'
                       + '  /usr/bin/sleep 0.2\n  /usr/bin/rm "$@"\n  rc=$?\n'
                       + '  printf "%s\\n" "$rc" > ' + shlex.quote(str(completed)) + '\n'
                       + '  return "$rc"\n}\n')
    delayed_results = []
    for locale in ('C', 'C.utf8'):
        completed.unlink(missing_ok=True)
        passed = check('IPM-65', 'signal', locale, locale, delayed)
        finished = completed.exists() and completed.read_text().strip() == '0'
        delayed_results.append(passed and finished)
        print(f'cleanup completion requested={locale} screen_ok={passed} cleanup_done={finished}', flush=True)
    if not all(delayed_results):
        raise SystemExit('FAIL PRQ-SAM01: signal screen must await real inner cleanup in both locales')

    cases = [('IPM-28', 'source'), ('IPM-72', 'linger'), ('IPM-45', 'consent'), ('IPM-65', 'signal')]
    results = [check(ident, label, locale, locale)
               for ident, label in cases for locale in ('C', 'C.utf8')]
    if not all(results):
        raise SystemExit('FAIL PRQ-SAM01: child locale does not match the selected driver order')

    for ident, label, name, count in [('IPM-45', 'consent', 'consent-test.py', 2),
                                      ('IPM-65', 'signal', 'signal-test.py', 1)]:
        helper = clone / 'scripts/fakes/prereq' / name
        original = helper.read_text()
        marker = '"LC_ALL": locale'
        if original.count(marker) != count:
            raise SystemExit(f'FAIL PRQ-SAM01: cannot apply hardcoded-C mutant to {name}')
        helper.write_text(original.replace(marker, '"LC_ALL": "C"'))
        try:
            if not check(ident, label, 'C.utf8', 'C'):
                raise SystemExit(f'FAIL PRQ-SAM01: hardcoded-C mutant did not reach the child in {name}')
        finally:
            helper.write_text(original)
        print(f'PASS PRQ-SAM01: hardcoded-C mutant killed in {name}', flush=True)

        suite = driver.TEST
        original_suite = suite.read_text()
        lines = original_suite.splitlines(keepends=True)
        matches = [line for line in lines if f'/prereq/{name}' in line and '"$prereq_screen_locale"' in line]
        if len(matches) != 1:
            raise SystemExit(f'FAIL PRQ-SAM01: cannot exercise {name} standalone default')
        suite.write_text(original_suite.replace(matches[0], matches[0].replace(' "$prereq_screen_locale"', ''), 1))
        try:
            if not check(ident, label, 'C.utf8', 'C'):
                raise SystemExit(f'FAIL PRQ-SAM01: standalone {name} does not default to C')
        finally:
            suite.write_text(original_suite)
        print(f'PASS PRQ-SAM01: standalone {name} defaults to C', flush=True)

        suite.write_text(original_suite.replace(matches[0], matches[0].rstrip('\n') + ' unexpected-extra-argument\n', 1))
        try:
            rc, output = driver.suite(include, 'C', named=ident)
            if rc == 0 or 'FAIL PRQ-SAM01: invalid helper arguments' not in output:
                raise SystemExit(f'FAIL PRQ-SAM01: {name} accepted an unexpected extra argument')
        finally:
            suite.write_text(original_suite)
        print(f'PASS PRQ-SAM01: {name} rejects an extra argument', flush=True)

        suite.write_text(original_suite.replace(matches[0], matches[0].replace('"$prereq_screen_locale"', "'--unknown-locale'"), 1))
        try:
            rc, output = driver.suite(include, 'C', named=ident)
            if rc == 0 or 'FAIL PRQ-SAM01: invalid helper arguments' not in output:
                raise SystemExit(f'FAIL PRQ-SAM01: {name} accepted an invalid locale option')
        finally:
            suite.write_text(original_suite)
        print(f'PASS PRQ-SAM01: {name} rejects an invalid locale option', flush=True)

print('PASS PRQ-SAM01: source, linger, consent and signal children receive both selected locales', flush=True)
