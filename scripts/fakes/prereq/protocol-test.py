#!/usr/bin/env python3
"""Check that fake network and RPM verifiers reject unsafe invocations."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile

root = Path(sys.argv[1])
with tempfile.TemporaryDirectory(prefix='prereq-protocol-', dir=os.environ['TMPDIR']) as tmp:
    base = Path(tmp)
    bin_dir, state, fixtures, keys = (base / name for name in ('bin', 'state', 'fixtures', 'keys'))
    for path in (bin_dir, state, fixtures, keys):
        path.mkdir()
    for name in ('curl', 'rpmkeys'):
        (bin_dir / name).symlink_to(root / 'scripts/fakes/prereq/fake-command')
    (fixtures / 'metalink').write_text('fixture')
    (keys / 'certificate.key').write_text('fixture')
    env = dict(os.environ, FAKE_LOG=str(base / 'log'), FAKE_STATE_DIR=str(state),
               FAKE_ALLOW_FETCH='1', FAKE_FETCH_FIXTURES=str(fixtures), FAKE_ALLOW_RPMKEYS='1')
    def run(tool, args):
        return subprocess.run([str(bin_dir / tool), *args], env=env,
                              stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False).returncode
    url = 'https://mirrors.fedoraproject.org/metalink?repo=epel'
    curl = ['--disable', '--fail', '--silent', '--show-error', '--location', '--proto', '=https',
            '--proto-redir', '=https', '--max-time', '45', '--max-filesize', '1024', '--', url]
    rpm = ['--define', '_keyring fs', '--define', f'_keyringpath {keys}',
           '--define', '_pkgverify_level all', '--define', '_pkgverify_flags 0',
           '--define', '__vsflags 0', '--checksig', '--verbose', str(base / 'one.rpm')]
    if run('curl', curl) or run('rpmkeys', rpm):
        raise SystemExit('FAIL fake protocol: valid invocation rejected')
    for label, tool, args in [
        ('curl without HTTPS redirect restriction', 'curl', curl[:7] + curl[9:]),
        ('curl with insecure flag', 'curl', ['--insecure', *curl]),
        ('curl with unbounded size', 'curl', curl[:12] + ['0'] + curl[13:]),
        ('rpmkeys without full verification', 'rpmkeys', rpm[:4] + rpm[6:]),
        ('rpmkeys without filesystem backend', 'rpmkeys', rpm[2:]),
    ]:
        if run(tool, args) == 0:
            raise SystemExit(f'FAIL fake protocol: {label} accepted')
print('PASS fake protocol: required curl and rpmkeys security arguments asserted')
