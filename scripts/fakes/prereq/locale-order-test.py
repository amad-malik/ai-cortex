#!/usr/bin/env python3
"""Reproduce the real RPM pair's locale-dependent shell glob ordering."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT=Path(__file__).resolve().parents[3]
SOURCE=Path(os.environ.get('PREREQ_TEST_SOURCE',str(ROOT/'deploy/install-prereq.sh'))).resolve()
SCRATCH=Path(os.environ.get('PREREQ_TEST_TMP','/path/to/workspace/scratch/marv/prereq-real-lo')).resolve()
if SCRATCH==Path('/tmp') or Path('/tmp') in SCRATCH.parents: raise SystemExit('scratch under /tmp is refused')
SCRATCH.mkdir(parents=True,exist_ok=True)
LOCALE='en_US.utf8'
NAMES=('python3-dotenv-1.0.1-3.el10_0.noarch.rpm','python3-dotenv+cli-1.0.1-3.el10_0.noarch.rpm')

class LocaleOrder(unittest.TestCase):
    def test_L_real_pair_candidate_order_is_byte_order(self):
        available=subprocess.run(['locale','-a'],capture_output=True,text=True,check=True).stdout.splitlines()
        self.assertIn(LOCALE,available,'real en_US.utf8 locale required for L evidence')
        with tempfile.TemporaryDirectory(dir=SCRATCH,prefix='real-collation-') as tmp:
            tmp=Path(tmp); stage=tmp/'stage'; stage.mkdir(mode=0o700);packages=stage/'packages';packages.mkdir(mode=0o700)
            for name in NAMES: (packages/name).write_bytes(b'synthetic RPM, never executed\n')
            rpm=tmp/'rpm';rpm.write_text('''#!/bin/sh
case "$*" in *python3-dotenv+cli*) printf 'python3-dotenv+cli|0|1.0.1|3.el10_0|noarch\\n' ;; *) printf 'python3-dotenv|0|1.0.1|3.el10_0|noarch\\n' ;; esac
''');rpm.chmod(0o755)
            env=dict(os.environ,LANG=LOCALE,LC_ALL=LOCALE,PATH=str(tmp)+':/usr/bin:/bin')
            glob=subprocess.run(['sh','-c','for p in "$1"/*; do printf "%s\\n" "${p##*/}"; done','sh',str(packages)],env=env,text=True,capture_output=True,check=True).stdout.splitlines()
            self.assertEqual(glob,list(NAMES),'en_US.utf8 must reproduce the captured collation before interpreting the result')
            command='''. "$1"
prereq_stage_path_ok() { :; }
prereq_rocky_candidates "$2" private || exit $?
printf 'CALLER_LOCALE=%s\\n' "$LC_ALL" >&2'''
            result=subprocess.run(['sh','-c',command,'sh',str(SOURCE),str(stage)],env=env,text=True,capture_output=True)
            self.assertEqual(result.returncode,0,result.stderr)
            records=result.stdout.splitlines()
            self.assertEqual(len(records),2)
            paths=[line.split('|',1)[0] for line in records]
            self.assertEqual(paths,sorted(paths,key=os.fsencode),'L: candidate list differs from the Python byte-sorted manifest')
            self.assertIn('CALLER_LOCALE='+LOCALE,result.stderr,'L: scoped ordering must preserve caller locale')

    def test_L_hidden_and_prefix_paths_are_globally_byte_sorted(self):
        with tempfile.TemporaryDirectory(dir=SCRATCH,prefix='path-order-') as tmp:
            tmp=Path(tmp); stage=tmp/'stage'; stage.mkdir(mode=0o700); packages=stage/'packages'; packages.mkdir(mode=0o700)
            for name in ('.a.rpm','..a.rpm','a.rpm','a.rpmX.rpm'): (packages/name).write_bytes(b'synthetic')
            rpm=tmp/'rpm'; rpm.write_text("#!/bin/sh\nfor last do :; done\nname=${last##*/}; name=${name%.rpm}\nprintf '%s|0|1.0|1|noarch\\n' \"$name\"\n"); rpm.chmod(0o755)
            env=dict(os.environ,LANG=LOCALE,LC_ALL=LOCALE,PATH=str(tmp)+':/usr/bin:/bin')
            command='. \"$1\"; prereq_stage_path_ok() { :; }; prereq_rocky_candidates \"$2\" private'
            result=subprocess.run(['sh','-c',command,'sh',str(SOURCE),str(stage)],env=env,text=True,capture_output=True)
            self.assertEqual(result.returncode,0,result.stderr)
            paths=[line.split('|',1)[0] for line in result.stdout.splitlines()]
            self.assertEqual(len(paths),4)
            self.assertEqual(paths,sorted(paths,key=os.fsencode),'L: hidden or prefix paths are not globally byte-sorted')

    def test_L_secure_call_uses_c_locale_and_restores_caller(self):
        env=dict(os.environ,LANG=LOCALE,LC_ALL=LOCALE)
        command='''. "$1"
probe() { printf 'INNER=%s\\n' "${LC_ALL-unset}"; }
prereq_secure_call probe || exit $?
printf 'AFTER=%s\\n' "${LC_ALL-unset}"'''
        result=subprocess.run(['sh','-c',command,'sh',str(SOURCE)],env=env,text=True,capture_output=True)
        self.assertEqual(result.returncode,0,result.stderr)
        self.assertIn('INNER=C',result.stdout)
        self.assertIn('AFTER='+LOCALE,result.stdout)

    def test_L_secure_call_restores_unset_and_empty_locale(self):
        for original in (None,''):
            with self.subTest(original=original):
                env=dict(os.environ,LANG=LOCALE)
                if original is None: env.pop('LC_ALL',None)
                else: env['LC_ALL']=original
                command='''. "$1"
probe() { [ "$LC_ALL" = C ] && env | grep -qx LC_ALL=C; }
prereq_secure_call probe || exit $?
printf 'AFTER=%s:%s\\n' "${LC_ALL+x}" "${LC_ALL-}"'''
                result=subprocess.run(['sh','-c',command,'sh',str(SOURCE)],env=env,text=True,capture_output=True)
                self.assertEqual(result.returncode,0,result.stderr)
                self.assertEqual(result.stdout.strip(),'AFTER='+(':' if original is None else 'x:'))

    def test_L_every_public_entry_restores_locale_on_success_and_error(self):
        command='''. "$1"
test_status=$3
probe() { [ "$LC_ALL" = C ] && env | grep -qx LC_ALL=C || return 91; return "$test_status"; }
machine_look_body() { probe "$@"; }
prereq_run_body() { probe "$@"; }
prereq_requalify_compose_body() { probe "$@"; }
rc=0; "$2" || rc=$?
printf 'STATUS=%s LC_ALL=%s LC_COLLATE=%s LANG=%s\\n' "$rc" "${LC_ALL-unset}" "${LC_COLLATE-unset}" "${LANG-unset}"
'''
        env=dict(os.environ,LANG=LOCALE,LC_ALL=LOCALE,LC_COLLATE='C.utf8')
        for entry in ('machine_look','prereq_run','prereq_requalify_compose'):
            for status in (0,42):
                with self.subTest(entry=entry,status=status):
                    result=subprocess.run(['sh','-c',command,'sh',str(SOURCE),entry,str(status)],env=env,text=True,capture_output=True)
                    self.assertEqual(result.returncode,0,result.stderr)
                    self.assertEqual(result.stdout.strip(),f'STATUS={status} LC_ALL={LOCALE} LC_COLLATE=C.utf8 LANG={LOCALE}')

if __name__=='__main__':unittest.main(verbosity=2)
