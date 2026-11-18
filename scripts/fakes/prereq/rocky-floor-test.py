#!/usr/bin/env python3
"""Rocky 10.MINOR floor and fail-before-package entry behavior."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[3]
SOURCE = Path(os.environ.get('PREREQ_TEST_SOURCE', str(ROOT / 'deploy/install-prereq.sh')))
SCRATCH = Path(os.environ.get('PREREQ_TEST_TMP', '/path/to/workspace/scratch/marv/prereq-real')).resolve()
if SCRATCH == Path('/tmp') or Path('/tmp') in SCRATCH.parents:
    raise SystemExit('refusing scratch under /tmp')
SCRATCH.mkdir(parents=True, exist_ok=True)


def inspect(identity, version):
    with tempfile.TemporaryDirectory(dir=SCRATCH, prefix='floor-') as directory:
        file = Path(directory) / 'os-release'
        file.write_text('ID=' + identity + '\nVERSION_ID=' + version + '\n')
        command = '''. "$1"
say() { printf '%s\\n' "$*"; }
warn() { printf '%s\\n' "$*"; }
if prereq_os_ok "$2"; then
  if [ "${PREREQ_OS_CONFIDENCE-}" = UNPROVEN ]; then prereq_info os_unproven "$pq_os_version"; fi
  printf 'ACCEPT|%s|%s\\n' "$pq_os_version" "${PREREQ_OS_CONFIDENCE-unset}"
else
  prereq_msg os_unsupported
  printf 'REFUSE|%s\\n' "${PREREQ_OS_REASON-unset}"
fi'''
        return subprocess.run(['sh', '-c', command, 'sh', str(SOURCE), str(file)], text=True, capture_output=True)


def entry(identity, version, architecture):
    with tempfile.TemporaryDirectory(dir=SCRATCH, prefix='floor-entry-') as directory:
        directory = Path(directory); etc = directory / 'etc'; etc.mkdir()
        (etc / 'os-release').write_text('ID=' + identity + '\nVERSION_ID=' + version + '\n')
        marker = directory / 'package-called'
        command = '''. "$1"
TEST_ROOT=$2 TEST_ARCH=$3 TEST_MARK=$4
say() { printf '%s\\n' "$*"; }
warn() { printf '%s\\n' "$*" >&2; }
prereq_harden_environment() { :; }
prereq_system_root() { printf '%s' "$TEST_ROOT"; }
prereq_parse_missing() { :; }
uname() { printf '%s\\n' "$TEST_ARCH"; }
dnf() { : > "$TEST_MARK"; }
rpm() { : > "$TEST_MARK"; }
EX_PREREQ=3 EX_MACHINE=2
prereq_run_packages_inner_body'''
        result = subprocess.run(['sh', '-c', command, 'sh', str(SOURCE), str(directory), architecture, str(marker)], text=True, capture_output=True, env=dict(os.environ, HOME=str(directory)))
        return result, marker.exists()


class RockyFloor(unittest.TestCase):
    def accepted(self, version, confidence):
        result = inspect('rocky', version)
        self.assertEqual(result.returncode, 0)
        self.assertIn('ACCEPT|' + version + '|' + confidence, result.stdout)
        if confidence == 'UNPROVEN':
            self.assertIn('UNPROVEN', result.stdout)
            self.assertIn('10.2', result.stdout)
        else:
            self.assertNotIn('UNPROVEN', result.stdout)
    def test_L2_repo_policy_carries_validated_minor(self):
        fixtures = ROOT / 'scripts/fakes/prereq/rocky102'
        for version in ('10.0', '10.1', '10.2', '10.3'):
            for repo in ('baseos', 'epel'):
                with self.subTest(version=version, repo=repo), tempfile.TemporaryDirectory(dir=SCRATCH, prefix='floor-repo-') as directory:
                    directory = Path(directory)
                    if repo == 'baseos':
                        dump = (fixtures / 'dnf-dump-baseos.txt').read_text().replace('BaseOS-10.2', 'BaseOS-' + version)
                        function = 'prereq_rocky_repo_ok "$2" baseos'
                    else:
                        dump = (fixtures / 'epel-dump.txt').read_text().replace('repo=epel-z-10.2', 'repo=epel-z-' + version)
                        dump = dump.replace('file:///home/user/.cortex-prereq.HTnf4ChA/mixed/pinned/certificate.key', 'file:///etc/pki/rpm-gpg/RPM-GPG-KEY-EPEL-10')
                        function = 'prereq_epel_repo_config_ok "$2"'
                    (directory / 'dump').write_text(dump)
                    fake = directory / 'dnf'; fake.write_text('#!/bin/sh\ncat "$FLOOR_DUMP"\n'); fake.chmod(0o755)
                    env = dict(os.environ, PATH=str(directory)+':/usr/bin:/bin', FLOOR_DUMP=str(directory/'dump'))
                    result = subprocess.run(['sh', '-c', '. "$1"; ' + function, 'sh', str(SOURCE), version], env=env, capture_output=True, text=True)
                    self.assertEqual(result.returncode, 0, result.stderr)

    def test_L2_repo_policy_refuses_major_only(self):
        fixtures = ROOT / 'scripts/fakes/prereq/rocky102'
        for repo in ('baseos', 'epel'):
            with self.subTest(repo=repo), tempfile.TemporaryDirectory(dir=SCRATCH, prefix='floor-repo-invalid-') as directory:
                directory = Path(directory)
                if repo == 'baseos':
                    dump = (fixtures / 'dnf-dump-baseos.txt').read_text().replace('BaseOS-10.2', 'BaseOS-10')
                    function = 'prereq_rocky_repo_ok "$2" baseos'
                else:
                    dump = (fixtures / 'epel-dump.txt').read_text().replace('repo=epel-z-10.2', 'repo=epel-10')
                    dump = dump.replace('file:///home/user/.cortex-prereq.HTnf4ChA/mixed/pinned/certificate.key', 'file:///etc/pki/rpm-gpg/RPM-GPG-KEY-EPEL-10')
                    function = 'prereq_epel_repo_config_ok "$2"'
                (directory / 'dump').write_text(dump)
                fake = directory / 'dnf'; fake.write_text('#!/bin/sh\ncat "$FLOOR_DUMP"\n'); fake.chmod(0o755)
                env = dict(os.environ, PATH=str(directory)+':/usr/bin:/bin', FLOOR_DUMP=str(directory/'dump'))
                result = subprocess.run(['sh', '-c', '. "$1"; ' + function, 'sh', str(SOURCE), '10'], env=env, capture_output=True, text=True)
                self.assertNotEqual(result.returncode, 0, 'bare major must be refused')

    def test_L2_release_guard_requires_one_numeric_minor(self):
        command = '. "$1"; prereq_release_ok "$2"'
        for release, allowed in (('10.0', True), ('10.1', True), ('10.2', True), ('10.3', True),
                                 ('10', False), ('10.', False), ('10.2.1', False), ('11.0', False), ('10.a', False)):
            result = subprocess.run(['sh', '-c', command, 'sh', str(SOURCE), release], capture_output=True)
            with self.subTest(release=release): self.assertEqual(result.returncode == 0, allowed)

    def test_L2_epel_url_rejects_major_only_before_fetch(self):
        for repo, should_fetch in (('epel-10', False), ('epel-z-10.3', True), ('epel-z-11.0', False), ('epel-z-10.３', False)):
            with self.subTest(repo=repo), tempfile.TemporaryDirectory(dir=SCRATCH, prefix='floor-epel-') as directory:
                directory = Path(directory); stage = directory/'stage'; stage.mkdir(mode=0o700)
                marker = directory/'called'; curl = directory/'curl'
                curl.write_text('#!/bin/sh\n: > ' + str(marker) + '\nexit 97\n'); curl.chmod(0o755)
                env = dict(os.environ, PATH=str(directory)+':/usr/bin:/bin')
                command = '. "$1"; prereq_epel_fetch "$2" "$3" "$4" 0 "$5"'
                subprocess.run(['sh','-c',command,'sh',str(SOURCE),
                    'https://mirrors.fedoraproject.org/metalink?repo='+repo+'&arch=x86_64',
                    'podman-compose|0|1.5.0|1.el10_1|noarch',str(stage),str(directory/'key')],env=env,capture_output=True)
                self.assertEqual(marker.exists(), should_fetch)

    def test_L2_machine_reports_unproven_before_packages(self):
        with tempfile.TemporaryDirectory(dir=SCRATCH, prefix='floor-machine-') as directory:
            directory=Path(directory);(directory/'etc').mkdir()
            (directory/'etc/os-release').write_text('ID=rocky\nVERSION_ID=10.3\n')
            command = ''' . "$1"
TEST_ROOT=$2
prereq_system_root() { printf '%s' "$TEST_ROOT"; }
prereq_info() { if [ "$1" = os_unproven ]; then printf 'UNPROVEN before checks\\n'; return 99; fi; :; }
prereq_msg() { exit 88; }
prereq_subid_ok() { return 1; }
uname() { printf 'x86_64\\n'; }
EX_PREREQ=3
machine_look_body
'''
            result=subprocess.run(['sh','-c',command,'sh',str(SOURCE),str(directory)],capture_output=True,text=True)
            self.assertIn('UNPROVEN before checks',result.stdout)
            self.assertEqual(result.returncode,3)

    def test_L2_unproven_notice_from_package_entry(self):
        result, called = entry('rocky','10.3','x86_64')
        self.assertNotEqual(result.returncode, 0); self.assertFalse(called)
        self.assertIn('UNPROVEN', result.stderr)
        self.assertIn('10.2', result.stderr)

    def test_L2_unproven_notice_rejects_untrusted_text(self):
        command = '. "$1"; warn() { printf "%s\\n" "$*"; }; prereq_info os_unproven "$2"'
        result = subprocess.run(['sh','-c',command,'sh',str(SOURCE),'10.3;unsafe'],capture_output=True,text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn('unsafe', result.stdout + result.stderr)

    def test_L2_requalification_refuses_wrong_arch_before_package(self):
        with tempfile.TemporaryDirectory(dir=SCRATCH, prefix='floor-requal-') as directory:
            directory = Path(directory); etc=directory/'etc';etc.mkdir()
            (etc/'os-release').write_text('ID=rocky\nVERSION_ID=10.2\n')
            marker=directory/'package-called'
            command = ''' . "$1"
TEST_ROOT=$2 TEST_MARK=$3
prereq_harden_environment() { :; }
prereq_home_safe() { :; }
prereq_system_root() { printf '%s' "$TEST_ROOT"; }
uname() { printf 'aarch64\\n'; }
dnf() { : > "$TEST_MARK"; }
rpm() { : > "$TEST_MARK"; }
EX_PREREQ=3
(prereq_requalify_compose_inner_body)
'''
            result=subprocess.run(['sh','-c',command,'sh',str(SOURCE),str(directory),str(marker)],env=dict(os.environ,HOME=str(directory)),capture_output=True,text=True)
            self.assertNotEqual(result.returncode,0);self.assertFalse(marker.exists())
            self.assertIn('x86_64',result.stderr)

    def test_L2_accept_10_0(self): self.accepted('10.0', 'SUPPORTED')
    def test_L2_accept_10_1(self): self.accepted('10.1', 'SUPPORTED')
    def test_L2_accept_10_2(self): self.accepted('10.2', 'SUPPORTED')
    def test_L2_unproven_10_3(self): self.accepted('10.3', 'UNPROVEN')
    def test_L2_unproven_leading_zero_minor(self): self.accepted('10.0003', 'UNPROVEN')
    def test_L2_supported_leading_zero_minor(self): self.accepted('10.0002', 'SUPPORTED')
    def test_L2_unproven_large_minor(self): self.accepted('10.12345678901234567890', 'UNPROVEN')
    def test_L2_refuse_major_only(self): self.assertIn('REFUSE|invalid_version', inspect('rocky','10').stdout)
    def test_L2_refuse_multiple_minor_parts(self): self.assertIn('REFUSE|invalid_version', inspect('rocky','10.2.1').stdout)
    def test_L2_refuse_empty_minor(self): self.assertIn('REFUSE|invalid_version', inspect('rocky','10.').stdout)
    def test_L2_refuse_nonrocky(self):
        result = inspect('almalinux','10.2')
        self.assertIn('REFUSE|distribution', result.stdout)
        self.assertIn('distribution', result.stderr.lower())
    def test_L2_refuse_el11_missing_chain(self):
        result = inspect('rocky','11.0')
        self.assertIn('REFUSE|untrusted_major', result.stdout)
        self.assertIn('trusted', result.stderr.lower())
    def test_L2_nonrocky_refused_before_package_actions(self):
        result, called = entry('almalinux','10.2','x86_64')
        self.assertNotEqual(result.returncode, 0); self.assertFalse(called)
        self.assertIn('distribution', result.stderr.lower())
    def test_L2_major11_refused_before_package_actions(self):
        result, called = entry('rocky','11.0','x86_64')
        self.assertNotEqual(result.returncode, 0); self.assertFalse(called)
        self.assertIn('trusted', result.stderr.lower())
    def test_L2_wrong_arch_refused_before_package_actions(self):
        result, called = entry('rocky','10.2','aarch64')
        self.assertNotEqual(result.returncode, 0); self.assertFalse(called)
        self.assertIn('x86_64', result.stderr)

if __name__ == '__main__': unittest.main(verbosity=2)
