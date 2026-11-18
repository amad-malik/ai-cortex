#!/usr/bin/env python3
"""Check that the F-7 lint rejects an unguarded write in every named scope."""
import importlib.util
import pathlib
import sys
import tempfile

lint_path = pathlib.Path(__file__).with_name('output-lint.py')
spec = importlib.util.spec_from_file_location('output_lint', lint_path)
lint = importlib.util.module_from_spec(spec)
spec.loader.exec_module(lint)
scratch = pathlib.Path(__import__('os').environ['TMPDIR'])
if str(scratch) == '/tmp' or str(scratch).startswith('/tmp/'):
    raise SystemExit('refusing /tmp')
with tempfile.TemporaryDirectory(dir=scratch, prefix='f7-lint-') as directory:
    source = pathlib.Path(directory) / 'fixture.sh'
    clean = ''.join(f'{name}() {{\n  say "value; with semicolon" || return 3\n}}\n' for name in sorted(lint.SCOPES))
    source.write_text(clean)
    if lint.audit(source, set()):
        raise SystemExit('FAIL F-7 lint: rejected checked writes')
    source.write_text(clean.replace('machine_look_body() {\n', 'machine_look_body() {\n  for package in awk:gawk sed:sed grep:grep; do :; done\n', 1))
    if lint.audit(source, set()):
        raise SystemExit('FAIL F-7 lint: package-name list treated as output')
    for name in sorted(lint.SCOPES):
        source.write_text(clean.replace(f'{name}() {{\n', f'{name}() {{\n  say "injected"\n', 1))
        errors = lint.audit(source, set())
        if len(errors) != 1 or f'{name}: unchecked say' not in errors[0]:
            raise SystemExit(f'FAIL F-7 lint: missed injected write in {name}: {errors}')
    for command in ('cat "$some_file"', 'printf "%s\\n" data | cat || return 3'):
        name = 'machine_look_body'
        source.write_text(clean.replace(f'{name}() {{\n', f'{name}() {{\n  {command}\n', 1))
        if not lint.audit(source, set()):
            raise SystemExit(f'FAIL F-7 lint: missed direct writer or masked pipeline: {command}')
    for expansion in ('${#value}', '${value#prefix}'):
        name = 'machine_look_body'
        source.write_text(clean.replace(f'{name}() {{\n', f'{name}() {{\n  x={expansion}; say "injected"\n', 1))
        if not lint.audit(source, set()):
            raise SystemExit(f'FAIL F-7 lint: missed write after {expansion}')
    source.write_text(clean.replace('  say "value; with semicolon" || return 3', '  say "approved best effort" || : # best-effort', 1))
    if not lint.audit(source, set()):
        raise SystemExit('FAIL F-7 lint: unlisted best effort accepted')
    name = sorted(lint.SCOPES)[0]
    if lint.audit(source, {f'{name}:say "approved best effort" || : # best-effort'}):
        raise SystemExit('FAIL F-7 lint: listed best effort rejected')
print('PASS F-7 lint: checked baseline, one injected write per scope, best-effort list')
