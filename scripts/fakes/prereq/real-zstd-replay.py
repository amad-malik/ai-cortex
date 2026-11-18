#!/usr/bin/env python3
"""Offline actual EPEL metadata replay; stop before accepting synthetic RPM bytes."""
import hashlib, os, shlex, subprocess, sys, tempfile
from pathlib import Path
source=Path(sys.argv[1]).resolve()
fixtures=Path('/path/to/docs/mvp/pieces/test-vm-fresh-install-defects-20261109')
zst=fixtures/'77a07a83214bc8792c768ee81d6cca228864241a029965b166639a31bc2abaa9-primary.xml.zst'
assert hashlib.sha256(zst.read_bytes()).hexdigest()=='77a07a83214bc8792c768ee81d6cca228864241a029965b166639a31bc2abaa9'
scratch=Path('/path/to/workspace/scratch/marv/prereq-real')
with tempfile.TemporaryDirectory(dir=scratch,prefix='real-zstd-') as tmp:
    tmp=Path(tmp); stage=tmp/'stage';stage.mkdir(mode=0o700);log=tmp/'requests';curl=tmp/'curl'
    curl.write_text('#!/bin/sh\nprintf "%s\\n" "$*" >> '+shlex.quote(str(log))+'\ncase "$*" in '
      '*mirrors.fedoraproject.org/metalink*) cat '+shlex.quote(str(fixtures/'metalink-epel-z-10.2.xml'))+' ;; '
      '*repodata/repomd.xml*) cat '+shlex.quote(str(fixtures/'epel-repomd.xml'))+' ;; '
      '*primary.xml.zst*) cat '+shlex.quote(str(zst))+' ;; *) exit 97 ;; esac\n')
    curl.chmod(0o755)
    env=dict(os.environ,PATH=str(tmp)+':/usr/bin:/bin',LC_ALL='C')
    r=subprocess.run(['sh','-c','. "$1"; prereq_epel_fetch "$2" "$3" "$4" 0 "$5"','sh',str(source),
       'https://mirrors.fedoraproject.org/metalink?repo=epel-z-10.2&arch=x86_64',
       'podman-compose|0|1.5.0|1.el10_1|noarch',str(stage),str(tmp/'unused-key')],env=env,capture_output=True,text=True)
    requests=log.read_text().splitlines() if log.exists() else []
    print('exit',r.returncode,'requests',len(requests))
    for i,request in enumerate(requests,1): print(i,request.split(' -- ')[-1])
    if r.returncode!=1 or len(requests)!=4 or 'podman-compose-1.5.0-1.el10_1.noarch.rpm' not in requests[-1]: sys.exit(1)
