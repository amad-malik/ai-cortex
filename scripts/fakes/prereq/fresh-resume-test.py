#!/usr/bin/env python3
"""Require the fresh-process resume scenario to derive findings itself."""
from pathlib import Path
import sys

root = Path(sys.argv[1])
suite = (root / 'scripts/install-prereq-test.sh').read_text()
start = suite.index('        if [ "$RUN_FLOW_CASE" = partial_resume ]; then')
end = suite.index('        prereq_run || exit $?', start)
resume = suite[start:end]
if 'PREREQ_MISSING=$RUN_MISSING' in resume or 'RUN_RESUME_HELPER' not in resume:
    raise SystemExit('FAIL SAM-03: fresh resume must rediscover prerequisites with machine_look')
if 'FRESH_RESUME' not in resume or 'installed.gnupg2' not in resume:
    raise SystemExit('FAIL SAM-03: fresh resume lacks a retained package-state check')
helper = root / 'scripts/fakes/prereq/fresh-resume-child.sh'
if not helper.is_file() or 'machine_look || exit' not in helper.read_text():
    raise SystemExit('FAIL SAM-03: fresh-process machine look is missing')
print('PASS SAM-03: fresh resume derives findings from machine state')
