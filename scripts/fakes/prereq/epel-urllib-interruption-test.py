#!/usr/bin/env python3
"""Replay interrupted urllib responses without a network or curl."""
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

ROOT=Path(__file__).resolve().parents[3]
SOURCE=Path(os.environ.get('PREREQ_TEST_SOURCE',ROOT/'deploy/install-prereq.sh'))
SCRATCH=Path(os.environ.get('PREREQ_TEST_TMP','/path/to/workspace/scratch/marv/epel-mirrors'))
if SCRATCH == Path('/tmp') or Path('/tmp') in SCRATCH.parents: raise SystemExit('refusing /tmp')
SCRATCH.mkdir(parents=True,exist_ok=True)

class InterruptedUrl(unittest.TestCase):
    def run_case(self,mode):
        with tempfile.TemporaryDirectory(prefix='interrupted-',dir=SCRATCH) as name:
            tmp=Path(name); fixture=tmp/'fixtures'; fixture.mkdir(); stage=tmp/'stage'; stage.mkdir(mode=0o700)
            subprocess.run(['python3',str(ROOT/'scripts/fakes/prereq/repo-fixtures.py'),str(fixture),'valid'],check=True)
            p=fixture/'metalink'; data=p.read_text(); old='<url>https://mirror.example/epel/repodata/repomd.xml</url>'
            self.assertIn(old,data); p.write_text(data.replace(old,''.join(f'<url>https://m{i}.example/epel/repodata/repomd.xml</url>' for i in (1,2))))
            text=SOURCE.read_text(); program=text.split("<<'PREREQ_EPEL_PY'\n",1)[1].split('\nPREREQ_EPEL_PY',1)[0]
            log=tmp/'requests'
            shim='''\nimport http.client\nfixture = Path(%r)\nlog = Path(%r)\nmode = %r\nshutil.which = lambda name: None if name == "curl" else None\nclass Response:\n    def __init__(self, url):\n        self.url = url\n        self.offset = 0\n        with log.open("a") as f: f.write(url + "\\n")\n        if "mirrors.fedoraproject.org" in url: item = "metalink"\n        elif url.endswith("/repodata/repomd.xml"): item = "repomd"\n        elif url.endswith("/repodata/primary.xml.gz"): item = "primary"\n        elif url.endswith("/Packages/p/podman-compose.rpm"): item = "package"\n        else: raise ValueError("unexpected offline URL")\n        self.data = (fixture / item).read_bytes()\n        self.interrupt = (item == "repomd" and (mode == "all" or "m1.example" in url)) or (item == "metalink" and mode == "metalink_once" and len(log.read_text().splitlines()) == 1)\n    def __enter__(self): return self\n    def __exit__(self,*args): return False\n    def read(self, n):\n        if self.interrupt: raise http.client.IncompleteRead(b"partial", 20)\n        result = self.data[self.offset:self.offset+n]\n        self.offset += len(result)\n        return result\nclass Opener:\n    def open(self,url,timeout): return Response(url)\nurllib.request.build_opener = lambda *args: Opener()\n''' % (str(fixture),str(log),mode)
            marker='metalink_url, requested, directory, metadata_check, keyfile, selection = sys.argv[1:]'
            self.assertEqual(program.count(marker),1); program=program.replace(marker,shim+'\n'+marker,1)
            result=subprocess.run(['python3','-I','-c',program,'https://mirrors.fedoraproject.org/metalink?repo=epel-z-10.2&arch=x86_64','podman-compose|0|1.5.0|1.el10_1|noarch',str(stage),'0',str(tmp/'key'),'compose'],capture_output=True,text=True)
            requests=log.read_text().splitlines() if log.exists() else []
            return result.returncode,result.stderr,requests,sorted(p.name for p in stage.iterdir())
    def test_interrupted_first_mirror_falls_back(self):
        rc,err,requests,files=self.run_case('first')
        self.assertEqual(rc,0,err); self.assertIn('compose.rpm',files)
        self.assertTrue(any('m2.example' in u for u in requests),requests)
    def test_interrupted_metalink_is_retried(self):
        rc,err,requests,files=self.run_case('metalink_once')
        self.assertEqual(rc,0,err); self.assertIn('compose.rpm',files)
        self.assertEqual(sum('mirrors.fedoraproject.org' in u for u in requests),2)
    def test_all_interrupted_fail_closed_without_files(self):
        rc,err,requests,files=self.run_case('all')
        self.assertNotEqual(rc,0); self.assertEqual(files,[])
        self.assertIn('2 tried',err); self.assertTrue(any('m2.example' in u for u in requests),requests)
if __name__=='__main__': unittest.main(verbosity=2)
