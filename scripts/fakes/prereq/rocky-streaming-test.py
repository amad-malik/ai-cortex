#!/usr/bin/env python3
"""Rocky streaming verification and memory preflight, using offline data."""
import ast
import gzip
import hashlib
import io
import os
import shlex
import subprocess
import tempfile
from pathlib import Path
import unittest
from unittest.mock import mock_open, patch
import xml.etree.ElementTree as ET

ROOT=Path(__file__).resolve().parents[3]
SOURCE=Path(os.environ.get('PREREQ_TEST_SOURCE', str(ROOT/'deploy/install-prereq.sh')))

def load():
    text=SOURCE.read_text().split("<<'PREREQ_ROCKY_METADATA_PY'\n",1)[1].split('\nPREREQ_ROCKY_METADATA_PY',1)[0]
    tree=ast.parse(text); names={'require_rocky_memory','verify_rocky_primary','iter_rocky_packages'}
    funcs=[n for n in tree.body if isinstance(n,ast.FunctionDef) and n.name in names]
    if {n.name for n in funcs}!=names: raise AssertionError('bounded Rocky streaming verifier not implemented')
    def require(value):
        if not value: raise ValueError('unverified metadata')
    ns=dict(gzip=gzip,hashlib=hashlib,io=io,ET=ET,require=require,ROCKY_PRIMARY_LIMIT=256,local=lambda tag:tag.rsplit('}',1)[-1])
    exec(compile(ast.fix_missing_locations(ast.Module(body=funcs,type_ignores=[])),str(SOURCE),'exec'),ns)
    return ns

class Streaming(unittest.TestCase):
    def setUp(self): self.ns=load()
    def test_C_stream_accepts_checked_primary(self):
        raw=b'<metadata><package type="rpm"><name>ok</name></package></metadata>'
        data=gzip.compress(raw,mtime=0)
        self.ns['verify_rocky_primary'](data,hashlib.sha256(raw).hexdigest())
        packages=list(self.ns['iter_rocky_packages'](data))
        self.assertEqual(len(packages),1)
    def test_C_stream_rejects_bad_digest(self):
        with self.assertRaises(ValueError): self.ns['verify_rocky_primary'](gzip.compress(b'<metadata/>',mtime=0),'0'*64)
    def test_C_stream_rejects_over_cap(self):
        raw=b' '*257
        with self.assertRaises(ValueError): self.ns['verify_rocky_primary'](gzip.compress(raw,mtime=0),hashlib.sha256(raw).hexdigest())
    def test_C_stream_rejects_truncated_gzip(self):
        raw=b'<metadata/>'; data=gzip.compress(raw,mtime=0)[:-1]
        with self.assertRaises((ValueError,OSError,EOFError)): self.ns['verify_rocky_primary'](data,hashlib.sha256(raw).hexdigest())
    def test_C_stream_rejects_doctype_and_entity(self):
        for raw in (b'<!DOCTYPE metadata><metadata/>', b'<!ENTITY x "y"><metadata/>'):
            with self.subTest(raw=raw), self.assertRaises(ValueError): self.ns['verify_rocky_primary'](gzip.compress(raw,mtime=0),hashlib.sha256(raw).hexdigest())
    def test_C_stream_rejects_doctype_across_chunk_boundary(self):
        raw = b' ' * 65532 + b'<!DOCTYPE metadata><metadata/>'
        self.ns['ROCKY_PRIMARY_LIMIT'] = len(raw)
        with self.assertRaises(ValueError): self.ns['verify_rocky_primary'](gzip.compress(raw,mtime=0),hashlib.sha256(raw).hexdigest())

    def test_C_stream_read_chunks_are_bounded(self):
        self.ns['ROCKY_PRIMARY_LIMIT'] = 1000000
        raw = b'<metadata/>'; limits=[]
        class FakeSource:
            def __enter__(self): return self
            def __exit__(self,*args): pass
            def read(self, limit):
                limits.append(limit)
                return raw if len(limits)==1 else b''
        with patch.object(gzip,'GzipFile',return_value=FakeSource()):
            self.ns['verify_rocky_primary'](b'ignored',hashlib.sha256(raw).hexdigest())
        self.assertTrue(limits); self.assertLessEqual(max(limits),65536)

    def test_C_stream_preserves_package_children_until_consumed(self):
        raw=b'<metadata><package type="rpm"><name>first</name></package><package type="rpm"><name>second</name></package></metadata>'
        names=[]
        for package in self.ns['iter_rocky_packages'](gzip.compress(raw,mtime=0)):
            names.append(package.find('name').text)
        self.assertEqual(names,['first','second'])

    def test_C_stream_releases_completed_elements(self):
        raw=b'<metadata><package type="rpm"><name>first</name></package><package type="rpm"><name>second</name></package></metadata>'
        original = ET.iterparse
        seen = []
        def observe(*args, **kwargs):
            for event, element in original(*args, **kwargs):
                if not seen and event == 'start': seen.append(element)
                yield event, element
        retained=[]
        with patch.object(ET,'iterparse',side_effect=observe):
            for element in self.ns['iter_rocky_packages'](gzip.compress(raw,mtime=0)):
                retained.append(element)
        self.assertEqual(len(seen[0]),0, 'completed packages must be removed from root')
        self.assertTrue(retained)
        self.assertTrue(all(len(element)==0 for element in retained), 'completed package contents must be cleared')

    def test_C_stream_ignores_nested_packages(self):
        raw=b'<metadata><container><package type="rpm"/></container><package type="rpm"/></metadata>'
        self.assertEqual(len(list(self.ns['iter_rocky_packages'](gzip.compress(raw,mtime=0)))),1)
    def test_C_memory_preflight_stops_before_primary_fetch(self):
        scratch = Path(os.environ.get('PREREQ_TEST_TMP', '/path/to/workspace/scratch/marv/prereq-real')).resolve()
        if scratch == Path('/tmp') or Path('/tmp') in scratch.parents: raise ValueError('unsafe scratch')
        scratch.mkdir(parents=True,exist_ok=True)
        for available, mismatch in ((1, False), (1048576, False), (1048576, True)):
            with self.subTest(available=available, mismatch=mismatch), tempfile.TemporaryDirectory(dir=scratch,prefix='memory-preflight-') as temp:
                temp=Path(temp); stage=temp/'stage';stage.mkdir(mode=0o700)
                package_hash=hashlib.sha256(b'fixture').hexdigest()
                (stage/'transaction.manifest').write_text(str(stage/'package.rpm')+'|'+package_hash+'|gnupg2|0|2.4.5|1.el10|x86_64|baseos\n')
                raw=(f'<metadata><package type="rpm"><name>gnupg2</name><arch>x86_64</arch>'
                     f'<version epoch="0" ver="2.4.5" rel="1.el10"/><checksum type="sha256">{package_hash}</checksum>'
                     '</package></metadata>').encode()
                packed=gzip.compress(raw,mtime=0);(temp/'primary').write_bytes(packed)
                repomd=(f'<repomd><data type="primary"><checksum type="sha256">{hashlib.sha256(packed).hexdigest()}</checksum>'
                        f'<open-checksum type="sha256">{"0"*64 if mismatch else hashlib.sha256(raw).hexdigest()}</open-checksum>'
                        '<location href="repodata/primary.xml.gz"/></data></repomd>')
                (temp/'repomd').write_text(repomd);(temp/'meminfo').write_text(f'MemAvailable: {available} kB\n')
                original=SOURCE.read_text(); needle='open("/proc/meminfo", encoding="ascii")'
                self.assertEqual(original.count(needle),1)
                modified=original.replace(needle, 'open('+repr(str(temp/'meminfo'))+', encoding="ascii")',1)
                (temp/'include.sh').write_text(modified)
                log=temp/'requests'; curl=temp/'curl';q=lambda name:shlex.quote(str(temp/name))
                curl.write_text('#!/bin/sh\nprintf "%s\\n" "$*" >> '+q('requests')+'\ncase "$*" in '
                    '*repodata/repomd.xml*) cat '+q('repomd')+' ;; *repodata/primary.xml.gz*) cat '+q('primary')+' ;; *) exit 97 ;; esac\n')
                curl.chmod(0o755)
                result=subprocess.run(['sh','-c','. "$1"; prereq_rocky_metadata_ok 10.2 "$2" baseos=0 verify_private',
                    'sh',str(temp/'include.sh'),str(stage/'transaction.manifest')],env=dict(os.environ,PATH=str(temp)+':/usr/bin:/bin'),capture_output=True,text=True)
                requests=log.read_text().splitlines() if log.exists() else []
                self.assertEqual(result.returncode,0 if available == 1048576 and not mismatch else 1)
                self.assertEqual(len(requests),2 if available == 1048576 else 1)
                if available == 1:
                    self.assertIn('at least 1 GiB currently available',result.stderr)
                    self.assertIn('at least 4 GiB RAM',result.stderr)

    def test_C_memory_accepts_exact_threshold(self):
        with patch('builtins.open',mock_open(read_data='MemTotal: 9000 kB\nMemAvailable: 2000 kB\n')):
            self.ns['require_rocky_memory'](2000)
    def test_C_memory_refuses_below_threshold(self):
        with patch('builtins.open',mock_open(read_data='MemAvailable: 1999 kB\n')), self.assertRaises(MemoryError):
            self.ns['require_rocky_memory'](2000)
    def test_C_memory_refuses_missing_malformed_and_duplicate(self):
        for text in ('MemTotal: 9000 kB\n','MemAvailable: nope kB\n','MemAvailable: 2000 MB\n','MemAvailable: 2000 kB\nMemAvailable: 2000 kB\n'):
            with self.subTest(text=text), patch('builtins.open',mock_open(read_data=text)), self.assertRaises(MemoryError):
                self.ns['require_rocky_memory'](2000)
    def test_C_memory_refuses_unreadable(self):
        with patch('builtins.open',side_effect=OSError('unavailable')), self.assertRaises(MemoryError):
            self.ns['require_rocky_memory'](2000)

if __name__=='__main__': unittest.main(verbosity=2)
