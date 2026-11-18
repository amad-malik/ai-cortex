#!/usr/bin/env python3
"""Bounded zstd decoder contract; isolates native calls from network and package tools."""
import ast
import ctypes
import hashlib
import os
from pathlib import Path
import stat
import shlex
import subprocess
import tempfile
import unittest
from unittest.mock import patch

SOURCE = Path(os.environ.get('PREREQ_TEST_SOURCE', str(Path(__file__).resolve().parents[3] / 'deploy/install-prereq.sh')))
CAP = 268435456
RAW = b'<metadata><package/></metadata>'
FRAME = (Path(__file__).parent / 'rocky102/tiny-primary.xml.zst').read_bytes()
UNKNOWN_FRAME = (Path(__file__).parent / 'rocky102/tiny-unknown.xml.zst').read_bytes()



def load_decoder():
    text = SOURCE.read_text().split("<<'PREREQ_EPEL_PY'\n", 1)[1].split('\nPREREQ_EPEL_PY', 1)[0]
    tree = ast.parse(text)
    names = {'trusted_zstd_library', 'decode_zstd'}
    functions = [node for node in tree.body if isinstance(node, ast.FunctionDef) and node.name in names]
    if {node.name for node in functions} != names:
        raise AssertionError('G5 decoder and library checks are not implemented')
    def require(condition):
        if not condition: raise ValueError('unverified EPEL metadata')
    ns = dict(ctypes=ctypes, os=os, stat=stat, require=require)
    exec(compile(ast.fix_missing_locations(ast.Module(body=functions, type_ignores=[])), str(SOURCE), 'exec'), ns)
    return ns['decode_zstd']


def fake_library(declared, frame_size, decoded):
    class Function:
        def __init__(self, value): self.value = value
        def __call__(self, *args): return self.value
    class Library: pass
    lib = Library()
    lib.ZSTD_getFrameContentSize = Function(declared)
    lib.ZSTD_findFrameCompressedSize = Function(frame_size)
    lib.ZSTD_decompress = Function(decoded)
    lib.ZSTD_isError = Function(0)
    return lib


class G5(unittest.TestCase):
    def setUp(self): self.decode = load_decoder()
    def test_G_positive_tiny_frame(self):
        try: decoded = self.decode(FRAME, len(RAW))
        except (ValueError, OSError) as exc: self.fail('valid frame refused: ' + str(exc))
        self.assertEqual(decoded, RAW)
    def test_G_unknown_declared_size_bounded(self):
        try: decoded = self.decode(UNKNOWN_FRAME, len(RAW))
        except (ValueError, OSError) as exc: self.fail('valid unknown-size frame refused: ' + str(exc))
        self.assertEqual(decoded, RAW)
        with self.assertRaises((ValueError, OSError)): self.decode(UNKNOWN_FRAME, len(RAW)-1)
    def test_G_oversized_and_invalid_frame_declarations_refused(self):
        real = ctypes.CDLL('/usr/lib64/libzstd.so.1')
        class Function:
            def __init__(self, value): self.value = value
            def __call__(self, *args): return self.value
        class Library:
            def __init__(self, value): self.ZSTD_getFrameContentSize = Function(value)
            def __getattr__(self, key): return getattr(real, key)
        for value in (CAP + 1, (1 << 64) - 2):
            with self.subTest(value=value), patch('ctypes.CDLL', return_value=Library(value)), self.assertRaises((ValueError, OSError)):
                self.decode(FRAME, len(RAW))
    def test_G_untrusted_ancestor_refused(self):
        old = os.lstat
        def changed(path):
            info = old(path)
            if str(path) == '/usr/lib64':
                values = list(info); values[0] |= 0o020
                return os.stat_result(values)
            return info
        with patch('os.lstat', side_effect=changed), self.assertRaises((ValueError, OSError)):
            self.decode(FRAME, len(RAW))
    def test_G_end_to_end_checksum_and_size_controls(self):
        lib = ctypes.CDLL('/usr/lib64/libzstd.so.1')
        lib.ZSTD_compressBound.argtypes = [ctypes.c_size_t]
        lib.ZSTD_compressBound.restype = ctypes.c_size_t
        lib.ZSTD_compress.argtypes = [ctypes.c_void_p, ctypes.c_size_t, ctypes.c_void_p, ctypes.c_size_t, ctypes.c_int]
        lib.ZSTD_compress.restype = ctypes.c_size_t
        package = b'synthetic rpm; never executed\n'
        package_hash = hashlib.sha256(package).hexdigest()
        raw = (f'<metadata><package type="rpm"><name>podman-compose</name><arch>noarch</arch>'
               f'<version epoch="0" ver="1.5.0" rel="1.el10_1"/>'
               f'<checksum type="sha256">{package_hash}</checksum>'
               '<location href="Packages/p/podman-compose.rpm"/></package></metadata>').encode()
        buf = ctypes.create_string_buffer(lib.ZSTD_compressBound(len(raw)))
        n = lib.ZSTD_compress(buf, len(buf), ctypes.create_string_buffer(raw), len(raw), 1)
        compressed = buf.raw[:n]
        scratch = Path(os.environ.get('PREREQ_TEST_TMP', '/path/to/workspace/scratch/marv/prereq-real')).resolve()
        if scratch == Path('/tmp') or Path('/tmp') in scratch.parents: raise ValueError('unsafe scratch')
        scratch.mkdir(parents=True, exist_ok=True)
        for case in ('valid', 'bad_open_checksum', 'wrong_open_size', 'oversized_open_size', 'bad_compressed_checksum', 'plus_size', 'long_size', 'duplicate_open_size'):
            with self.subTest(case=case), tempfile.TemporaryDirectory(dir=scratch, prefix='zstd-chain-') as directory:
                directory = Path(directory); stage = directory / 'stage'; stage.mkdir(mode=0o700)
                open_hash = hashlib.sha256(raw).hexdigest() if case != 'bad_open_checksum' else '0' * 64
                packed_hash = hashlib.sha256(compressed).hexdigest() if case != 'bad_compressed_checksum' else '0' * 64
                length = CAP+1 if case == 'oversized_open_size' else len(raw)+(case == 'wrong_open_size')
                if case == 'plus_size': length = '+' + str(len(raw))
                if case == 'long_size': length = '0000000000' + str(len(raw))
                extra_size = f'<open-size>{length}</open-size>' if case == 'duplicate_open_size' else ''
                repomd = (f'<repomd><data type="primary"><checksum type="sha256">{packed_hash}</checksum>'
                          f'<open-checksum type="sha256">{open_hash}</open-checksum><open-size>{length}</open-size>{extra_size}'
                          '<location href="repodata/primary.xml.zst"/></data></repomd>').encode()
                repomd_hash = hashlib.sha256(repomd).hexdigest()
                metalink = (f'<metalink><file name="repomd.xml"><verification><hash type="sha256">{repomd_hash}</hash>'
                            '</verification><resources><url>https://mirror.example/epel/repodata/repomd.xml</url></resources>'
                            '</file></metalink>').encode()
                for name, data in (('metalink', metalink), ('repomd', repomd), ('primary', compressed), ('package', package)):
                    (directory / name).write_bytes(data)
                log = directory / 'requests'; curl = directory / 'curl'; q = lambda name: shlex.quote(str(directory / name))
                curl.write_text('#!/bin/sh\nprintf "%s\\n" "$*" >> ' + q('requests') + '\ncase "$*" in '
                    '*mirrors.fedoraproject.org/metalink*) cat ' + q('metalink') + ' ;; '
                    '*repodata/repomd.xml*) cat ' + q('repomd') + ' ;; '
                    '*repodata/primary.xml.zst*) cat ' + q('primary') + ' ;; '
                    '*Packages/p/podman-compose.rpm*) cat ' + q('package') + ' ;; *) exit 97 ;; esac\n')
                curl.chmod(0o755)
                env = dict(os.environ, PATH=str(directory)+':/usr/bin:/bin', LC_ALL='C')
                command = '. "$1"; prereq_epel_fetch "$2" "$3" "$4" 0 "$5"'
                result = subprocess.run(['sh', '-c', command, 'sh', str(SOURCE),
                    'https://mirrors.fedoraproject.org/metalink?repo=epel-z-10.2&arch=x86_64',
                    'podman-compose|0|1.5.0|1.el10_1|noarch', str(stage), str(directory / 'unused-key')],
                    env=env, capture_output=True, text=True)
                requests = log.read_text().splitlines() if log.exists() else []
                self.assertEqual(result.returncode, 0 if case == 'valid' else 1, result.stderr)
                self.assertEqual(len(requests), 4 if case == 'valid' else 3, case)
                self.assertEqual((stage / 'compose.rpm').exists(), case == 'valid')

    def test_G_untrusted_root_refused(self):
        old = os.lstat
        def changed(path):
            info = old(path)
            if str(path) == '/':
                values = list(info); values[0] |= 0o002
                return os.stat_result(values)
            return info
        with patch('os.lstat', side_effect=changed), self.assertRaises((ValueError, OSError)):
            self.decode(FRAME, len(RAW))

    def test_G_non_directory_ancestor_refused(self):
        old = os.lstat
        def changed(path):
            info = old(path)
            if str(path) == '/usr/lib64':
                values = list(info); values[0] = stat.S_IFREG | 0o755
                return os.stat_result(values)
            return info
        with patch('os.lstat', side_effect=changed), self.assertRaises((ValueError, OSError)):
            self.decode(FRAME, len(RAW))

    def test_G_non_regular_target_refused(self):
        old = os.lstat
        resolved = os.path.realpath('/usr/lib64/libzstd.so.1')
        def changed(path):
            info = old(path)
            if str(path) == resolved:
                values = list(info); values[0] = stat.S_IFDIR | 0o755
                return os.stat_result(values)
            return info
        with patch('os.lstat', side_effect=changed), self.assertRaises((ValueError, OSError)):
            self.decode(FRAME, len(RAW))

    def test_G_outside_library_roots_refused(self):
        original_readlink = os.readlink
        def redirected(path):
            if str(path) == '/usr/lib64/libzstd.so.1': return '/usr/bin/true'
            return original_readlink(path)
        real = ctypes.CDLL('/usr/lib64/libzstd.so.1')
        with patch('os.readlink', side_effect=redirected), patch('ctypes.CDLL', return_value=real), self.assertRaises((ValueError, OSError)):
            self.decode(FRAME, len(RAW))

    def test_G_symlink_limit_enforced(self):
        original_readlink = os.readlink
        target = os.path.realpath('/usr/lib64/libzstd.so.1')
        count = 0
        def chain(path):
            nonlocal count
            if str(path) == '/usr/lib64/libzstd.so.1':
                count += 1
                return 'libzstd.so.1' if count <= 40 else target
            return original_readlink(path)
        with patch('os.readlink', side_effect=chain), self.assertRaises((ValueError, OSError)):
            self.decode(FRAME, len(RAW))

    def test_G_symlink_loop_refused(self):
        original_readlink = os.readlink
        def loop(path):
            if str(path) == '/usr/lib64/libzstd.so.1': return 'libzstd.so.1'
            return original_readlink(path)
        with patch('os.readlink', side_effect=loop), self.assertRaises((ValueError, OSError)):
            self.decode(FRAME, len(RAW))

    def test_G_bad_frame_magic_refused_before_decode(self):
        bad = b'not-a-zstd-frame'
        lib = fake_library(len(RAW), len(bad), len(RAW))
        with patch('ctypes.CDLL', return_value=lib), self.assertRaises((ValueError, OSError)):
            self.decode(bad, len(RAW))

    def test_G_complete_single_frame_enforced_before_decode(self):
        both = FRAME + FRAME
        lib = fake_library(len(RAW), len(FRAME), len(RAW))
        with patch('ctypes.CDLL', return_value=lib), self.assertRaises((ValueError, OSError)):
            self.decode(both, len(RAW))

    def test_G_exact_decoded_length_enforced(self):
        lib = fake_library(len(RAW), len(FRAME), len(RAW)-1)
        with patch('ctypes.CDLL', return_value=lib), self.assertRaises((ValueError, OSError)):
            self.decode(FRAME, len(RAW))

    def test_G_missing_library_refused(self):
        old = os.lstat
        def absent(path):
            if str(path) in ('/usr/lib64/libzstd.so.1', '/lib64/libzstd.so.1'): raise FileNotFoundError(path)
            return old(path)
        with patch('os.lstat', side_effect=absent), self.assertRaises((ValueError, OSError)):
            self.decode(FRAME, len(RAW))
    def _unsafe(self, uid=None, writable=0):
        old = os.lstat
        resolved = os.path.realpath('/usr/lib64/libzstd.so.1')
        def changed(path):
            info = old(path)
            if str(path) in ('/usr/lib64/libzstd.so.1', '/lib64/libzstd.so.1', resolved):
                values = list(info); values[4] = info.st_uid if uid is None else uid; values[0] |= writable
                return os.stat_result(values)
            return info
        with patch('os.lstat', side_effect=changed), self.assertRaises((ValueError, OSError)):
            self.decode(FRAME, len(RAW))
    def test_G_unowned_library_refused(self): self._unsafe(uid=12345)
    def test_G_group_writable_library_refused(self): self._unsafe(writable=0o020)
    def test_G_world_writable_library_refused(self): self._unsafe(writable=0o002)
    def test_G_truncated_stream_refused(self):
        with self.assertRaises((ValueError, OSError)): self.decode(FRAME[:-1], len(RAW))
    def test_G_corrupt_stream_refused(self):
        with self.assertRaises((ValueError, OSError)): self.decode(b'not a zstd frame', len(RAW))
    def test_G_wrong_open_size_refused(self):
        with self.assertRaises((ValueError, OSError)): self.decode(FRAME, len(RAW)+1)
    def test_G_native_error_codes_refused(self):
        real = ctypes.CDLL('/usr/lib64/libzstd.so.1')
        class Function:
            def __init__(self, which): self.which = which
            def __call__(self, value): return int(value == self.which)
        class Library:
            def __init__(self, which): self.ZSTD_isError = Function(which)
            def __getattr__(self, key): return getattr(real, key)
        for code in (len(FRAME), len(RAW)):
            with self.subTest(code=code), patch('ctypes.CDLL', return_value=Library(code)), self.assertRaises((ValueError, OSError)):
                self.decode(FRAME, len(RAW))

    def test_G_open_size_bounds_before_allocation(self):
        original_buffer = ctypes.create_string_buffer
        class Output:
            raw = b''
        def buffer(value, *args):
            if isinstance(value, int): return Output()
            return original_buffer(value, *args)
        for size in (0, CAP + 1):
            lib = fake_library((1 << 64) - 1, len(FRAME), size)
            with self.subTest(size=size), patch('ctypes.CDLL', return_value=lib), patch('ctypes.create_string_buffer', side_effect=buffer), self.assertRaises((ValueError, OSError)):
                self.decode(FRAME, size)

    def test_G_oversized_open_size_refused(self):
        with self.assertRaises((ValueError, OSError)): self.decode(FRAME, CAP+1)
    def test_G_zero_open_size_refused(self):
        with self.assertRaises((ValueError, OSError)): self.decode(FRAME, 0)
    def test_G_trailing_bytes_refused(self):
        with self.assertRaises((ValueError, OSError)): self.decode(FRAME+b'extra', len(RAW))
    def test_G_second_frame_refused(self):
        with self.assertRaises((ValueError, OSError)): self.decode(FRAME+FRAME, len(RAW))
    def test_G_skippable_frame_refused(self):
        with self.assertRaises((ValueError, OSError)): self.decode(bytes.fromhex('502a4d1800000000')+FRAME, len(RAW))

if __name__ == '__main__': unittest.main(verbosity=2)
