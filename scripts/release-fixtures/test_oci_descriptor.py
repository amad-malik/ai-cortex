#!/usr/bin/env python3
"""OCI descriptors must authenticate the manifest and config bytes."""
import hashlib
import importlib.util
import io
import json
import subprocess
import sys
import tarfile
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parents[1] / "release_manifest.py"
spec = importlib.util.spec_from_file_location("release_manifest", SCRIPT)
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


def make_archive(path, corrupt=None):
    config = b'{"architecture":"arm64","os":"linux"}'
    config_hash = hashlib.sha256(config).hexdigest()
    config_size = len(config) + (1 if corrupt == "config-size" else 0)
    manifest = json.dumps({"schemaVersion": 2, "config": {"digest": "sha256:" + config_hash, "size": config_size}, "layers": []}).encode()
    manifest_hash = hashlib.sha256(manifest).hexdigest()
    index = json.dumps({"schemaVersion": 2, "manifests": [{"digest": "sha256:" + manifest_hash, "size": len(manifest) + (1 if corrupt == "manifest-size" else 0)}]}).encode()
    members = {"index.json": index, "blobs/sha256/" + manifest_hash: manifest, "blobs/sha256/" + config_hash: config}
    if corrupt == "manifest-bytes":
        members["blobs/sha256/" + manifest_hash] += b" "
    if corrupt == "config-bytes":
        members["blobs/sha256/" + config_hash] += b" "
    with tarfile.open(path, "w") as out:
        for name, data in members.items():
            entry = tarfile.TarInfo(name)
            entry.size = len(data)
            out.addfile(entry, io.BytesIO(data))
    return "sha256:" + manifest_hash


class OCIIntegrity(unittest.TestCase):
    def test_descriptor_integrity(self):
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "image.tar"
            expected = make_archive(path)
            self.assertEqual(release.image_digest(path), expected)
            for corruption in ("manifest-size", "manifest-bytes", "config-size", "config-bytes"):
                with self.subTest(corruption=corruption):
                    make_archive(path, corruption)
                    with self.assertRaises((SystemExit, ValueError)):
                        release.image_digest(path)

    def test_same_length_blob_replacement_fails_hash_check(self):
        with tempfile.TemporaryDirectory() as td:
            path = Path(td) / "image.tar"
            make_archive(path)
            with tarfile.open(path, "r") as original:
                files = [(m.name, original.extractfile(m).read()) for m in original.getmembers()]
            for blob in ("manifest", "config"):
                with self.subTest(blob=blob):
                    with tarfile.open(path, "w") as out:
                        for name, data in files:
                            if name.startswith("blobs/") and ((b'architecture' in data) == (blob == "config")):
                                data = bytes([data[0] ^ 1]) + data[1:]
                            info = tarfile.TarInfo(name)
                            info.size = len(data)
                            out.addfile(info, io.BytesIO(data))
                    with self.assertRaises(ValueError):
                        release.image_digest(path)
                    result = subprocess.run([sys.executable, "-I", str(SCRIPT.parent / "mac_release_manifest.py"), "image-check", "--archive", str(path), "--sha256", hashlib.sha256(path.read_bytes()).hexdigest()], capture_output=True)
                    self.assertNotEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
