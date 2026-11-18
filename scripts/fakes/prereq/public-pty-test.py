#!/usr/bin/env python3
"""V-09: drive the public prerequisite flow through a private PTY and fakes."""
import os
from pathlib import Path
import pty
import select
import subprocess
import sys
import termios
import time

if len(sys.argv) == 3 and sys.argv[1] == '--self-check':
    suite = (Path(sys.argv[2]) / 'scripts/install-prereq-test.sh').read_text()
    if not all(token in suite for token in ('public-pty-test.py', 'pty_yes', 'pty_no', 'pty_blank', 'pty_eof')):
        raise SystemExit('FAIL V-09: public PTY cases are not wired into the suite')
    print('PASS V-09: public PTY cases are wired into the suite')
    raise SystemExit(0)

include, fake_root, shell, answer = sys.argv[1:]
root = Path(fake_root)
tty = root / 'dev' / 'tty'
tty.parent.mkdir(parents=True, exist_ok=True)
if tty.exists() or tty.is_symlink():
    raise SystemExit('FAIL V-09: PTY path already exists')
master, slave = pty.openpty()
settings = termios.tcgetattr(slave)
settings[3] &= ~termios.ECHO
termios.tcsetattr(slave, termios.TCSANOW, settings)
tty.symlink_to(os.ttyname(slave))
script = r'''. "$1"
say() { printf "%s\n" "$*"; }
warn() { :; }
EX_PREREQ=3 EX_MACHINE=2 home_dir=$HOME port=8601
FAKE_ROOT_STAGE="$PREREQ_ROOT/var/cache/cortex-prereq-1000-$$"; export FAKE_ROOT_STAGE
PREREQ_MISSING=$RUN_MISSING PREREQ_FIXES=$RUN_FIXES
prereq_run'''
env = dict(os.environ, CORTEX_YES='0')
proc = None
try:
    proc = subprocess.Popen([shell, '-c', script, 'sh', include], env=env, stdin=subprocess.PIPE,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    proc.stdin.write(b'y\n')
    proc.stdin.close()
    proc.stdin = None
    output = bytearray()
    streams = [proc.stdout, proc.stderr]
    deadline = time.monotonic() + 180
    while b'Proceed with these changes? [y/N]' not in output:
        remaining = deadline - time.monotonic()
        if remaining <= 0 or not streams:
            raise RuntimeError('public prompt did not appear')
        ready, _, _ = select.select(streams, [], [], min(remaining, 2))
        if not ready and proc.poll() is not None:
            raise RuntimeError('public flow exited before prompt')
        for stream in ready:
            chunk = os.read(stream.fileno(), 65536)
            if chunk:
                output.extend(chunk)
            else:
                streams.remove(stream)
    before = Path(env['FAKE_LOG']).read_text() if Path(env['FAKE_LOG']).exists() else ''
    if any(line.startswith('sudo ') for line in before.splitlines()):
        raise RuntimeError('sudo ran before the question')
    text = output.decode('utf-8', 'replace').lower()
    if 'prerequisite plan:' not in text or 'community' not in text or 'epel' not in text or 'persist' not in text:
        raise RuntimeError('complete EPEL disclosure was not before the question')
    os.write(master, {'pty_yes': b'y\n', 'pty_no': b'n\n', 'pty_blank': b'\n', 'pty_eof': b'\x04'}[answer])
    stdout, stderr = proc.communicate(timeout=180)
    output.extend(stdout)
    output.extend(stderr)
    expected = 0 if answer == 'pty_yes' else 2
    calls = Path(env['FAKE_LOG']).read_text() if Path(env['FAKE_LOG']).exists() else ''
    sudo_count = sum(line.startswith('sudo ') for line in calls.splitlines())
    if proc.returncode != expected or output.count(b'Proceed with these changes? [y/N]') != 1:
        raise RuntimeError('wrong status or question count: ' + str(proc.returncode))
    if (answer == 'pty_yes' and not sudo_count) or (answer != 'pty_yes' and sudo_count):
        raise RuntimeError('unexpected privileged calls after response')
    sys.stdout.buffer.write(output)
    print('\nPASS V-09: ' + answer + ' public plan, one question and sudo boundary')
except (OSError, RuntimeError, subprocess.TimeoutExpired) as exc:
    if proc is not None and proc.poll() is None:
        proc.kill()
        proc.communicate()
    raise SystemExit('FAIL V-09: ' + str(exc))
finally:
    tty.unlink(missing_ok=True)
    os.close(master)
    os.close(slave)
