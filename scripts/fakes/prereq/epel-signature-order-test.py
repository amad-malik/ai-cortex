#!/usr/bin/env python3
"""Exercise a bad RPM signature after the real offline EPEL hash chain."""
import importlib.util
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

HERE=Path(__file__).resolve().parent
spec=importlib.util.spec_from_file_location('mirror_test',HERE/'epel-mirror-test.py')
mirror=importlib.util.module_from_spec(spec); spec.loader.exec_module(mirror)

class SignatureOrder(unittest.TestCase):
    def test_bad_signature_fails_without_trying_second_mirror(self):
        with tempfile.TemporaryDirectory(prefix='signature-order-',dir=mirror.SCRATCH) as tmp:
            tmp=Path(tmp); fixture=tmp/'fixture'; fixture.mkdir(); home=tmp/'home'; home.mkdir(mode=0o700); bin_dir=tmp/'bin'; bin_dir.mkdir(); root=tmp/'root'; key=root/'etc/pki/rpm-gpg/RPM-GPG-KEY-EPEL-10'; key.parent.mkdir(parents=True); key.write_text('synthetic key')
            subprocess.run(['python3',str(mirror.ROOT/'scripts/fakes/prereq/repo-fixtures.py'),str(fixture),'valid'],check=True)
            metalink=fixture/'metalink'; text=metalink.read_text(); old='<url>https://mirror.example/epel/repodata/repomd.xml</url>'; self.assertIn(old,text); metalink.write_text(text.replace(old,''.join(f'<url>https://m{i}.example/epel/repodata/repomd.xml</url>' for i in (1,2))))
            log=tmp/'requests'; fake=mirror.CURL.replace("os.environ['MIRROR_LOG']",repr(str(log))).replace("os.environ['MIRROR_FIXTURE']",repr(str(fixture))).replace("os.environ['MIRROR_MODE']",repr('valid'))
            curl=bin_dir/'curl'; curl.write_text(fake); curl.chmod(0o755)
            env=dict(os.environ,PATH=str(bin_dir)+':/usr/bin:/bin',HOME=str(home),FAKE_ROOT=str(root))
            command='''. "$1"
prereq_harden_environment() { :; }
prereq_home_safe() { :; }
prereq_system_root() { printf %s "$FAKE_ROOT"; }
prereq_os_ok() { pq_os_version=10.2; }
uname() { printf '%s\\n' x86_64; }
prereq_epel_key_ok() { :; }
prereq_epel_repo_config_ok() { PREREQ_EPEL_METALINK='https://mirrors.fedoraproject.org/metalink?repo=epel-z-10.2&arch=x86_64'; PREREQ_EPEL_REPO_GPGCHECK=0; }
rpm() { printf '%s\\n' 'podman-compose|0|1.5.0|1.el10_1|noarch'; }
rpmkeys() { printf '%s\\n' 'Header V4 RSA/SHA256 Signature, key ID deadbeef: NOKEY'; return 1; }
prereq_rpm_payload_ok() { printf '%s\\n' 'ERROR payload reached'; return 1; }
say() { printf '%s\\n' "$*"; }
EX_PREREQ=3; PREREQ_COMPOSE=untrusted
prereq_requalify_compose
'''
            result=subprocess.run(['sh','-c',command,'sh',str(mirror.SOURCE)],env=env,capture_output=True,text=True)
            requests=log.read_text().splitlines() if log.exists() else []
            self.assertNotEqual(result.returncode,0,result.stdout+result.stderr)
            self.assertTrue(any('m1.example/epel/Packages/' in x for x in requests),requests)
            self.assertFalse(any('m2.example' in x for x in requests),requests)
            self.assertIn('problem: the installed Compose provider could not be verified',result.stderr)
            self.assertNotIn('mirrors failed:',result.stderr)
            self.assertNotIn('ERROR payload reached',result.stdout+result.stderr)

if __name__=='__main__': unittest.main(verbosity=2)
