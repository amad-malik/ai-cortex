#!/usr/bin/env python3
"""Exercise the real Rocky metadata verifier offline at the observed expanded size."""
import gzip
import hashlib
import os
import resource
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile

source = Path(sys.argv[1]).resolve()
scratch = Path(os.environ.get('PREREQ_TEST_TMP', '/path/to/workspace/scratch/marv/prereq-real')).resolve()
if scratch == Path('/tmp') or Path('/tmp') in scratch.parents:
    raise SystemExit('scratch under /tmp is refused')
scratch.mkdir(parents=True, exist_ok=True)
size = 340525438
with tempfile.TemporaryDirectory(dir=scratch, prefix='large-metadata-') as temp:
    temp = Path(temp)
    stage = temp / 'stage'
    stage.mkdir(mode=0o700)
    package = stage / 'item.rpm'
    package.write_bytes(b'offline synthetic rpm\n')
    pkg_hash = hashlib.sha256(package.read_bytes()).hexdigest()
    manifest = stage / 'transaction.manifest'
    manifest.write_text(f'{package}|{pkg_hash}|gnupg2|0|2.4.5|1.el10|x86_64|baseos\n')
    xml = (f'<metadata xmlns="http://linux.duke.edu/metadata/common"><package type="rpm">'
           f'<name>gnupg2</name><arch>x86_64</arch><version epoch="0" ver="2.4.5" rel="1.el10"/>'
           f'<checksum type="sha256">{pkg_hash}</checksum></package></metadata>').encode()
    digest = hashlib.sha256()
    packed = temp / 'primary.gz'
    with gzip.open(packed, 'wb', compresslevel=1) as stream:
        stream.write(xml)
        digest.update(xml)
        remaining = size - len(xml)
        block = b' ' * 1048576
        while remaining:
            chunk = block[:min(remaining, len(block))]
            stream.write(chunk)
            digest.update(chunk)
            remaining -= len(chunk)
    packed_hash = hashlib.sha256(packed.read_bytes()).hexdigest()
    repomd = temp / 'repomd.xml'
    repomd.write_text(f'<repomd><data type="primary"><checksum type="sha256">{packed_hash}</checksum>'
                      f'<open-checksum type="sha256">{digest.hexdigest()}</open-checksum>'
                      '<location href="repodata/primary.xml.gz"/></data></repomd>')
    fake = temp / 'curl'
    fake.write_text('#!/bin/sh\ncase "$*" in *repodata/repomd.xml*) cat ' + shlex.quote(str(repomd)) +
                    ' ;; *repodata/primary.xml.gz*) cat ' + shlex.quote(str(packed)) + ' ;; *) exit 97 ;; esac\n')
    fake.chmod(0o755)
    env = dict(os.environ, PATH=str(temp) + ':/usr/bin:/bin', LC_ALL='C')
    result = subprocess.run(['sh', '-c', '. "$1"; prereq_rocky_metadata_ok 10.2 "$2" baseos=0 verify_private',
                             'sh', str(source), str(manifest)], env=env, text=True, capture_output=True)
    print('synthetic expanded bytes:', size)
    print('compressed bytes:', packed.stat().st_size)
    print('largest child peak RSS KiB:', resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss)
    print('status:', result.returncode)
    print('stdout:', result.stdout.strip())
    print('stderr:', result.stderr.strip())
    if result.returncode or 'rocky_metadata=baseos' not in result.stdout:
        sys.exit(1)
