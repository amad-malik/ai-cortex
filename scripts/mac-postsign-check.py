#!/usr/bin/env python3
"""Re-run the Mac signed-set gate and bind its receipt to the publication inputs.

The publisher must invoke this check immediately before publishing the Mac set.
The expected receipt hash must come from the reviewed release authorization.
"""
import hashlib
import os
import re
import stat
import subprocess
import sys
from pathlib import Path


def read_file(path, limit=65536):
    try:
        fd = os.open(path, os.O_RDONLY | os.O_NONBLOCK | os.O_NOFOLLOW)
        with os.fdopen(fd, "rb") as src:
            if not stat.S_ISREG(os.fstat(src.fileno()).st_mode):
                raise ValueError("input is not a regular file")
            data = src.read(limit + 1)
    except OSError as exc:
        raise ValueError("cannot read a required input") from exc
    if not data or len(data) > limit:
        raise ValueError("required input is empty or too large")
    return data


def check(set_dir, key, version, receipt, expected_hash):
    set_dir, key, receipt = Path(set_dir), Path(key), Path(receipt)
    if not set_dir.is_absolute() or set_dir.is_symlink() or not set_dir.is_dir() or not key.is_absolute():
        raise ValueError("set and key paths must be absolute and the set must be a real directory")
    if not re.fullmatch(r"[0-9A-Za-z][0-9A-Za-z.-]{0,63}", version) or not re.fullmatch(r"[0-9a-f]{64}", expected_hash):
        raise ValueError("invalid version or receipt hash")
    digest = lambda data: hashlib.sha256(data).hexdigest()
    receipt_bytes = read_file(receipt)
    if digest(receipt_bytes) != expected_hash:
        raise ValueError("receipt hash does not match the authorized hash")
    sums = read_file(set_dir / "SHA256SUMS")
    signature = read_file(set_dir / "SHA256SUMS.sig")
    key_bytes = read_file(key)
    text = receipt_bytes.decode("utf-8")
    pattern = (r"install-mac-release-check: PASS: [1-9][0-9]* cells passed, 0 failed, 0 not run; IT-M29 ran against the signed set "
               + re.escape(str(set_dir)) + r" \(SHA256SUMS sha256 " + digest(sums)
               + r", public key file sha256 " + digest(key_bytes) + r"\), version " + re.escape(version) + r"\n")
    if not re.fullmatch(pattern, text):
        raise ValueError("receipt does not bind this signed set, key and version")
    checker = Path(__file__).resolve().parent / "install-mac-release-check.sh"
    env = {k: v for k, v in os.environ.items() if not k.startswith(("INSTALL_MAC_", "MAC_SET_"))}
    result = subprocess.run(["sh", str(checker), str(set_dir), str(key), version], env=env,
                            stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, timeout=1800)
    if result.returncode != 0 or result.stdout != text:
        raise ValueError("fresh signed-set check did not reproduce the receipt")
    if (read_file(set_dir / "SHA256SUMS") != sums or read_file(set_dir / "SHA256SUMS.sig") != signature
            or read_file(key) != key_bytes or read_file(receipt) != receipt_bytes):
        raise ValueError("signed-set gate input changed while checking")


if __name__ == "__main__":
    try:
        if len(sys.argv) != 6:
            raise ValueError("usage: mac-postsign-check.py SET_DIR KEYFILE VERSION RECEIPT RECEIPT_SHA256")
        check(*sys.argv[1:])
        print("mac-postsign-check: PASS: fresh signed-set check and authorized receipt match")
    except (OSError, UnicodeError, ValueError, subprocess.TimeoutExpired) as exc:
        print("mac-postsign-check: FAIL: " + str(exc), file=sys.stderr)
        raise SystemExit(1)
