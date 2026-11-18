#!/usr/bin/env python3
"""Regression tests from Rocky 10.2 captures; no network or host package tools."""
import ast
import os
import sys
import shlex
from pathlib import Path
import subprocess
import tempfile
import unittest
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parent.parent
SOURCE = Path(os.environ.get('PREREQ_TEST_SOURCE', ROOT / 'deploy/install-prereq.sh'))
FIXTURES = ROOT / 'scripts/fakes/prereq/rocky102'
SCRATCH = Path(os.environ.get('PREREQ_TEST_TMP', '/path/to/workspace/scratch/marv/prereq-real'))


def config(source, repo, dump):
    SCRATCH.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=SCRATCH, prefix='real-repo-') as tmp:
        tmp = Path(tmp)
        (tmp / 'data').write_text(dump)
        fake = tmp / 'dnf'
        fake.write_text('#!/bin/sh\ncat "$PREREQ_REAL_DUMP"\n')
        fake.chmod(0o755)
        env = dict(os.environ, PATH=str(tmp) + ':/usr/bin:/bin', PREREQ_REAL_DUMP=str(tmp / 'data'), LC_ALL='C')
        command = '. "$1"; ' + ('prereq_epel_repo_config_ok 10.2' if repo == 'epel' else 'prereq_rocky_repo_ok 10.2 "$2"')
        return subprocess.run(['sh', '-c', command, 'sh', str(source), repo], env=env, capture_output=True, text=True).returncode


def rocky(repo='baseos'):
    return (FIXTURES / ('dnf-dump-' + repo + '.txt')).read_text()


def epel():
    return (FIXTURES / 'epel-dump.txt').read_text().replace(
        'file:///home/user/.cortex-prereq.HTnf4ChA/mixed/pinned/certificate.key',
        'file:///etc/pki/rpm-gpg/RPM-GPG-KEY-EPEL-10')


class Rocky102(unittest.TestCase):
    def test_A_real_rocky_dump_without_metalink(self):
        # Remove the independent B obstacle, retaining the real missing key.
        for repo in ('baseos', 'appstream', 'crb', 'extras'):
            dump = rocky(repo).replace('[' + repo + ']\n', '')
            with self.subTest(repo=repo):
                self.assertEqual(config(SOURCE, repo, dump), 0, 'A: real Rocky dump rejected without optional metalink')

    def test_B_real_rocky_section(self):
        # Add the formerly required empty key to isolate the real section line.
        dump = rocky().replace('[baseos]\n', '[baseos]\nmetalink = \n', 1)
        self.assertEqual(config(SOURCE, 'baseos', dump), 0, 'B: real Rocky section header rejected')
        dump = epel().replace('[epel]\n', '[epel]\nmirrorlist = \n', 1)
        self.assertEqual(config(SOURCE, 'epel', dump), 0, 'B: real EPEL section header rejected')

    def test_repo_headers_and_optional_fields_remain_strict(self):
        good = rocky()
        for label, dump in (
            ('foreign header', good.replace('[baseos]', '[other]')),
            ('duplicate header', good.replace('[baseos]', '[baseos]\n[baseos]')),
            ('bad line', good.replace('[baseos]', '[baseos]\nnot-a-setting')),
            ('nonempty alternate', good + 'metalink = https://example.invalid/other\n'),
            ('duplicate alternate', good + 'metalink = \nmetalink = \n'),
            ('disabled signature', good.replace('gpgcheck = 1', 'gpgcheck = 0')),
        ):
            with self.subTest(case=label):
                self.assertNotEqual(config(SOURCE, 'baseos', dump), 0, label)

    def test_epel_headers_remain_strict(self):
        good = epel()
        for label, dump in (
            ('foreign header', good.replace('[epel]', '[other]')),
            ('duplicate header', good.replace('[epel]', '[epel]\n[epel]')),
            ('bad line', good.replace('[epel]', '[epel]\nnot-a-setting')),
            ('nonempty alternate', good + 'mirrorlist = https://example.invalid/other\n'),
            ('duplicate alternate', good + 'mirrorlist = \nmirrorlist = \n'),
        ):
            with self.subTest(case=label):
                self.assertNotEqual(config(SOURCE, 'epel', dump), 0, label)

    def test_C_rocky_primary_observed_length(self):
        text = SOURCE.read_text().split("<<'PREREQ_ROCKY_METADATA_PY'\n", 1)[1].split('\nPREREQ_ROCKY_METADATA_PY', 1)[0]
        tree = ast.parse(text)
        values = [n.value.value for n in tree.body if isinstance(n, ast.Assign) and len(n.targets) == 1 and
                  isinstance(n.targets[0], ast.Name) and n.targets[0].id == 'ROCKY_PRIMARY_LIMIT' and isinstance(n.value, ast.Constant)]
        self.assertEqual(values, [536870912], 'C: observed size must fit a bounded Rocky expansion')

    def test_C_limit_and_checksum_stay_enforced(self):
        env = dict(os.environ, PREREQ_TEST_SOURCE=str(SOURCE))
        result = subprocess.run([sys.executable, '-I', '-B', str(ROOT / 'scripts/fakes/prereq/rocky-streaming-test.py')], env=env, capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_E_real_rpm_version_characters(self):
        text = SOURCE.read_text()
        for variable, values in (
            ('pq_private_path', ('/safe/podman-sequoia-0.4.0~pqc.3.rpm', '/safe/passt-0^20251210.rpm')),
            ('pq_locked_path', ('/safe/podman-sequoia-0.4.0~pqc.3.rpm', '/safe/passt-0^20251210.rpm')),
            ('pq_archive_file', ('/safe/podman-sequoia-0.4.0~pqc.3.rpm', '/safe/passt-0^20251210.rpm')),
            ('pq_archive_version:$pq_archive_release', ('0.4.0~pqc.3:1.el10_2', '0^20251210.gd04c480:7.el10_2')),
        ):
            if not values:
                continue
            line = next(line for line in text.splitlines() if line.strip().startswith('case $' + variable + ' in *[!'))
            for value in values:
                if ':' in variable:
                    args = value.split(':', 1)
                    assignments = 'pq_archive_version=$1 pq_archive_release=$2; '
                else:
                    args = [value]
                    assignments = variable + '=$1; '
                result = subprocess.run(['sh', '-c', 'probe() { ' + assignments + line.strip() + '; }; probe "$@"', 'sh', *args], capture_output=True)
                with self.subTest(guard=variable, value=value):
                    self.assertEqual(result.returncode, 0, 'E: real RPM version rejected by ' + variable)
        start = text.index('      NF != 5 || $1 !~', text.index('prereq_rocky_candidates()'))
        end = text.index("\n      END {", start)
        program = text[start:end] + "\nEND { if (NR != 1) exit 1 }"
        for version, release in (('0.4.0~pqc.3', '1.el10_2'), ('0^20251210.gd04c480', '7.el10_2'), ('1.0', '1~rc1'), ('1.0', '1^snapshot')):
            result = subprocess.run(['awk', '-F|', program], input='pkg|0|' + version + '|' + release + '|x86_64\n', text=True, capture_output=True)
            with self.subTest(guard='identity', value=(version, release)):
                self.assertEqual(result.returncode, 0, 'E: RPM identity guard rejected version or release')
        result = subprocess.run(['awk', '-F|', program], input='pkg|0|bad;version|1|x86_64\n', text=True, capture_output=True)
        self.assertNotEqual(result.returncode, 0, 'identity guard must reject shell punctuation')
        condition = next(line.strip()[3:-1] for line in text.splitlines() if 'not all(c in ' in line and 'for c in path.name)' in line)
        source = Path('/synthetic')
        for filename in ('podman-sequoia-0.4.0~pqc.3-1.el10_2.rpm', 'passt-0^20251210.gd04c480-7.el10_2.rpm'):
            path = source / 'packages' / filename
            with self.subTest(guard='combine', value=filename):
                self.assertFalse(eval(condition, dict(path=path, source=source)), 'E: combine filename guard rejected real RPM')

    def test_D_real_epel_dump_without_mirrorlist(self):
        # Remove the independent B obstacle; substitute this invocation's expected pinned key path.
        dump = epel().replace('[epel]\n', '')
        self.assertEqual(config(SOURCE, 'epel', dump), 0, 'D: real EPEL dump rejected without optional mirrorlist')

    def test_F_duplicate_current_digest_rejected(self):
        text = SOURCE.read_text()
        start = text.index('    verification = one([e for e in record', text.index('prereq_epel_fetch()'))
        end = text.index('\n    mirrors = []', start)
        snippet = '\n'.join(line[4:] for line in text[start:end].splitlines())
        metalink = ET.parse(FIXTURES / 'metalink-epel-z-10.2.xml').getroot()
        def local(tag): return tag.rsplit('}', 1)[-1]
        def one(items):
            if len(items) != 1: raise ValueError('expected one')
            return items[0]
        record = one([e for e in metalink.iter() if local(e.tag) == 'file' and e.get('name') == 'repomd.xml'])
        verification = one([e for e in record if local(e.tag) == 'verification'])
        current = one([e for e in verification if local(e.tag) == 'hash' and e.get('type') == 'sha256'])
        ET.SubElement(verification, current.tag, dict(current.attrib)).text = current.text
        with self.assertRaises(ValueError):
            exec(snippet, dict(record=record, local=local, one=one, sha256=lambda value: value))
        verification.remove(list(verification)[-1])
        ET.SubElement(record, verification.tag)
        with self.assertRaises(ValueError):
            exec(snippet, dict(record=record, local=local, one=one, sha256=lambda value: value))

    def test_F_alternate_does_not_rescue_wrong_or_missing_current_digest(self):
        import hashlib
        SCRATCH.mkdir(parents=True, exist_ok=True)
        for mode in ('wrong', 'missing'):
            with self.subTest(mode=mode), tempfile.TemporaryDirectory(dir=SCRATCH, prefix='real-epel-bad-current-') as temp:
                temp = Path(temp); stage = temp/'stage'; stage.mkdir(mode=0o700)
                root = ET.parse(FIXTURES/'metalink-epel-z-10.2.xml').getroot()
                local = lambda tag: tag.rsplit('}', 1)[-1]
                record = next(e for e in root.iter() if local(e.tag) == 'file' and e.get('name') == 'repomd.xml')
                verification = next(e for e in record if local(e.tag) == 'verification')
                current = next(e for e in verification if local(e.tag) == 'hash' and e.get('type') == 'sha256')
                expected = hashlib.sha256((FIXTURES/'epel-repomd.xml').read_bytes()).hexdigest()
                alternatives = [e for e in record.iter() if local(e.tag) == 'hash' and e.get('type') == 'sha256' and e is not current]
                self.assertTrue(alternatives)
                alternatives[0].text = expected
                if mode == 'wrong': current.text = '0' * 64
                else: verification.remove(current)
                (temp/'metalink').write_bytes(ET.tostring(root))
                log = temp/'requests'; curl = temp/'curl'
                curl.write_text('#!/bin/sh\nprintf "%s\\n" "$*" >> ' + shlex.quote(str(log)) + '\ncase "$*" in '
                    '*mirrors.fedoraproject.org/metalink*) cat ' + shlex.quote(str(temp/'metalink')) + ' ;; '
                    '*repodata/repomd.xml*) cat ' + shlex.quote(str(FIXTURES/'epel-repomd.xml')) + ' ;; *) exit 97 ;; esac\n')
                curl.chmod(0o755)
                env = dict(os.environ, PATH=str(temp)+':/usr/bin:/bin', LC_ALL='C')
                command = '. "$1"; prereq_epel_fetch "$2" "$3" "$4" 0 "$5"'
                result = subprocess.run(['sh','-c',command,'sh',str(SOURCE),
                    'https://mirrors.fedoraproject.org/metalink?repo=epel-z-10.2&arch=x86_64',
                    'podman-compose|0|1.5.0|1.el10_1|noarch',str(stage),str(temp/'unused-key')],env=env,capture_output=True,text=True)
                requests = log.read_text().splitlines() if log.exists() else []
                self.assertEqual(result.returncode, 1)
                if mode == 'wrong':
                    self.assertGreaterEqual(len(requests), 2)
                    self.assertLessEqual(len(requests), 9)
                    self.assertTrue(all('/repodata/repomd.xml' in request for request in requests[1:]))
                else:
                    self.assertEqual(len(requests), 1)
                self.assertEqual(list(stage.iterdir()), [])

    def test_F_real_metalink_current_hash(self):
        # Replay the captured metalink and matching repomd through the actual
        # acquisition function. Its known .zst refusal remains out of scope (G).
        SCRATCH.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=SCRATCH, prefix='real-epel-') as tmp:
            tmp = Path(tmp)
            stage = tmp / 'stage'
            stage.mkdir(mode=0o700)
            log = tmp / 'requests'
            curl = tmp / 'curl'
            curl.write_text('#!/bin/sh\nprintf \"%s\\n\" \"$*\" >> ' + shlex.quote(str(log)) + '\n'
                + 'case \"$*\" in *mirrors.fedoraproject.org/metalink*) cat ' + shlex.quote(str(FIXTURES / 'metalink-epel-z-10.2.xml')) + ' ;; '
                + '*repodata/repomd.xml*) cat ' + shlex.quote(str(FIXTURES / 'epel-repomd.xml')) + ' ;; *) exit 97 ;; esac\n')
            curl.chmod(0o755)
            env = dict(os.environ, PATH=str(tmp) + ':/usr/bin:/bin', LC_ALL='C')
            command = '. "$1"; prereq_epel_fetch "$2" "$3" "$4" 0 "$5"'
            result = subprocess.run(['sh', '-c', command, 'sh', str(SOURCE),
                'https://mirrors.fedoraproject.org/metalink?repo=epel-z-10.2&arch=x86_64',
                'podman-compose|0|1.5.0|1.el10_1|noarch', str(stage), str(tmp / 'unused-key')],
                env=env, capture_output=True, text=True)
            requests = log.read_text().splitlines() if log.exists() else []
            self.assertEqual(result.returncode, 1, 'expected G zstd refusal')
            self.assertGreaterEqual(len(requests), 3, 'F: real metalink and repomd must reach the zstd fetch')
            self.assertLessEqual(len(requests), 17)
            self.assertIn('/repodata/repomd.xml', requests[1])
            self.assertIn('-primary.xml.zst', requests[2])


if __name__ == '__main__':
    unittest.main(verbosity=2)
