#!/usr/bin/env python3
"""Offline mirror failover regression against the production fetch function."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
SOURCE = Path(os.environ.get('PREREQ_TEST_SOURCE', ROOT / 'deploy/install-prereq.sh'))
SCRATCH = Path(os.environ.get('PREREQ_TEST_TMP', '/path/to/workspace/scratch/marv/epel-mirrors'))
if SCRATCH == Path('/tmp') or Path('/tmp') in SCRATCH.parents:
    raise SystemExit('refusing /tmp scratch')
SCRATCH.mkdir(parents=True, exist_ok=True)

CURL = '''#!/usr/bin/python3
import os, pathlib, sys, time
from urllib.parse import urlsplit
url=sys.argv[-1]
log=pathlib.Path(os.environ['MIRROR_LOG'])
prior=log.read_text().splitlines() if log.exists() else []
with log.open('a') as f: f.write(url+'\\n')
p=urlsplit(url); fixture=pathlib.Path(os.environ['MIRROR_FIXTURE'])
if p.hostname == 'mirrors.fedoraproject.org': name='metalink'
elif p.path.endswith('/repodata/repomd.xml.asc'): name='signature'
elif p.path.endswith('/repodata/repomd.xml'): name='repomd'
elif p.path.endswith('/repodata/primary.xml.gz'): name='primary'
elif p.path.endswith('/Packages/p/podman-compose.rpm'): name='package'
else: sys.exit(90)
mode=os.environ['MIRROR_MODE']
if p.hostname == 'mirrors.fedoraproject.org' and mode == 'metalink_retry' and not prior: sys.exit(7)
if mode == 'slow': time.sleep(4)
first=p.hostname == 'm1.example'
if p.hostname and p.hostname.startswith('m') and p.hostname.endswith('.example'):
    if first and mode == 'bad_metadata_signature' and name == 'signature':
        sys.stdout.buffer.write(b'bad synthetic signature'); sys.exit(0)
    if mode in ('all_bad','bound') or (first and mode == 'unreachable'): sys.exit(7)
    if first and mode == 'wrong_index' and name == 'repomd':
        sys.stdout.buffer.write((fixture/name).read_bytes()+b' '); sys.exit(0)
    if first and mode == 'wrong_package' and name == 'package':
        sys.stdout.buffer.write(b'corrupted synthetic content'); sys.exit(0)
sys.stdout.buffer.write((fixture/name).read_bytes())
'''

class MirrorFallback(unittest.TestCase):
    def run_case(self, mode, count=2, source=SOURCE):
        with tempfile.TemporaryDirectory(prefix='mirror-', dir=SCRATCH) as tmp:
            tmp=Path(tmp); fixture=tmp/'fixture'; fixture.mkdir(); stage=tmp/'stage'; stage.mkdir(mode=0o700); bin_dir=tmp/'bin'; bin_dir.mkdir()
            subprocess.run(['python3',str(ROOT/'scripts/fakes/prereq/repo-fixtures.py'),str(fixture),'valid'],check=True)
            p=fixture/'metalink'; data=p.read_text(); original='<url>https://mirror.example/epel/repodata/repomd.xml</url>'
            urls=''.join(f'<url>https://m{i}.example/epel/repodata/repomd.xml</url>' for i in range(1,count+1))
            if original not in data: raise AssertionError('fixture metalink changed')
            p.write_text(data.replace(original,urls))
            log=tmp/'requests'
            fake=CURL.replace("os.environ['MIRROR_LOG']",repr(str(log))).replace("os.environ['MIRROR_FIXTURE']",repr(str(fixture))).replace("os.environ['MIRROR_MODE']",repr(mode))
            curl=bin_dir/'curl'; curl.write_text(fake); curl.chmod(0o755)
            gpg=bin_dir/'gpg'; gpg.write_text("#!/bin/sh\nwhile [ \"$1\" != --output ]; do shift; done\nshift\nprintf key > \"$1\"\n"); gpg.chmod(0o755)
            gpgv=bin_dir/'gpgv'; gpgv.write_text("#!/bin/sh\nfor arg do previous=$last; last=$arg; done\nif grep -q bad \"$previous\"; then exit 1; fi\nprintf '%s\\n' '[GNUPG:] VALIDSIG 7D8D15CBFC4E62688591FB2633D98517E37ED158 0 0 0 0 0 0 0 0 7D8D15CBFC4E62688591FB2633D98517E37ED158'\n"); gpgv.chmod(0o755)
            env=dict(os.environ,PATH=str(bin_dir)+':/usr/bin:/bin')
            cmd='''. "$1"
prereq_msg() { printf 'problem: %s\\n' "$*" >&2; }
prereq_epel_key_ok() { :; }
prereq_epel_fetch 'https://mirrors.fedoraproject.org/metalink?repo=epel-z-10.2&arch=x86_64' 'podman-compose|0|1.5.0|1.el10_1|noarch' "$2" "$3" "$2/key"
'''
            result=subprocess.run(['sh','-c',cmd,'sh',str(source),str(stage),'1' if mode == 'bad_metadata_signature' else '0'],env=env,text=True,capture_output=True)
            urls=log.read_text().splitlines() if log.exists() else []
            return result.returncode,result.stdout+result.stderr,urls,(stage/'compose.rpm').exists()

    def test_first_mirror_failures_fall_back_in_order(self):
        for mode in ('unreachable','wrong_index','wrong_package'):
            with self.subTest(mode=mode):
                rc,out,urls,archive=self.run_case(mode)
                self.assertEqual(rc,0,f'{mode}: expected second mirror to pass: {out}; {urls}')
                self.assertTrue(archive)
                hosts=[u.split('/')[2] for u in urls if 'm1.example' in u or 'm2.example' in u]
                self.assertIn('m1.example',hosts); self.assertIn('m2.example',hosts)
                self.assertLess(hosts.index('m1.example'),hosts.index('m2.example'))

    def test_all_bad_fails_with_actionable_count_and_no_archive(self):
        rc,out,urls,archive=self.run_case('all_bad')
        self.assertNotEqual(rc,0); self.assertFalse(archive)
        self.assertIn('2 tried',out); self.assertIn('index fetch=2',out)

    def test_bound_never_fetches_ninth_mirror(self):
        rc,out,urls,archive=self.run_case('bound',12)
        self.assertNotEqual(rc,0); self.assertFalse(archive)
        self.assertTrue(any('m8.example' in u for u in urls),urls)
        self.assertFalse(any('m9.example' in u for u in urls),urls)

    def test_overall_deadline_aborts_a_stalled_fetch(self):
        with tempfile.TemporaryDirectory(prefix='deadline-',dir=SCRATCH) as tmp:
            source=Path(tmp)/'include.sh'
            original=SOURCE.read_text(); anchor='deadline = time.monotonic() + 180'
            self.assertEqual(original.count(anchor),1)
            source.write_text(original.replace(anchor,'deadline = time.monotonic() + 1',1))
            import time
            started=time.monotonic(); rc,out,urls,archive=self.run_case('slow',source=source)
            self.assertNotEqual(rc,0); self.assertFalse(archive)
            self.assertLess(time.monotonic()-started,3.5)

    def test_bad_metadata_signature_tries_next_mirror(self):
        rc,out,urls,archive=self.run_case('bad_metadata_signature')
        self.assertEqual(rc,0,out); self.assertTrue(archive)
        self.assertTrue(any('m1.example' in u and u.endswith('.asc') for u in urls),urls)
        self.assertTrue(any('m2.example' in u and u.endswith('.asc') for u in urls),urls)

    def test_transient_metalink_failure_is_retried(self):
        rc,out,urls,archive=self.run_case('metalink_retry')
        self.assertEqual(rc,0,out); self.assertTrue(archive)
        self.assertEqual(sum('mirrors.fedoraproject.org' in u for u in urls),2)

    def test_first_good_does_not_issue_extra_requests(self):
        rc,out,urls,archive=self.run_case('valid')
        self.assertEqual(rc,0,out); self.assertTrue(archive)
        self.assertEqual(len(urls),4,urls)
        self.assertFalse(any('m2.example' in u for u in urls),urls)

if __name__ == '__main__': unittest.main(verbosity=2)
