#!/usr/bin/env python3
"""mac-release-parity-check.py: proves that scripts/mac-release.sh IS scripts/release.sh plus exactly the counted edits of scripts/make-mac-release.py, and that
scripts/mac-release-differences.txt lists exactly those edits (hue, 15 Nov 2026; cox W1-281: the Mac recipe is a copy-adapt, the Linux recipe is not changed).

usage:  python3 -I scripts/mac-release-parity-check.py [SCRIPTS_DIR]        (exit 0 = the pair is in parity, 1 = it is not, 64 = usage)

It copies SCRIPTS_DIR/release.sh and SCRIPTS_DIR/make-mac-release.py into a private temporary directory, runs the generator there, and compares the generated mac-release.sh and
mac-release-differences.txt with the committed ones, byte for byte. Three kinds of drift fail it: an edit of the committed mac-release.sh made by hand, a change of release.sh that the
Mac copy does not follow, and a generator edit whose old text is no longer in release.sh (the generator names the edit)."""
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path


def main(argv):
    if len(argv) > 2:
        print("usage: mac-release-parity-check.py [SCRIPTS_DIR]", file=sys.stderr)
        return 64
    d = Path(argv[1]) if len(argv) == 2 else Path(__file__).resolve().parent
    need = [d / "release.sh", d / "make-mac-release.py", d / "mac-release.sh", d / "mac-release-differences.txt"]
    for p in need:
        if p.is_symlink() or not p.is_file():
            print("FAIL  %s is missing or not a regular file" % p.name)
            return 1
    with tempfile.TemporaryDirectory(prefix="mac-parity.") as t:
        shutil.copyfile(d / "release.sh", Path(t) / "release.sh")
        r = subprocess.run([sys.executable, "-I", str(d / "make-mac-release.py"), t], capture_output=True, text=True)
        if r.returncode != 0:
            print("FAIL  the generator stopped: %s" % (r.stderr.strip() or r.stdout.strip())[:300])
            return 1
        bad = 0
        for name in ("mac-release.sh", "mac-release-differences.txt"):
            if (Path(t) / name).read_bytes() == (d / name).read_bytes():
                print("ok    %s is exactly what the generator makes from release.sh" % name)
            else:
                bad += 1
                a, b = (d / name).read_text().splitlines(), (Path(t) / name).read_text().splitlines()
                first = next((i for i, (x, y) in enumerate(zip(a, b)) if x != y), min(len(a), len(b)))
                print("FAIL  %s differs from the generator's output at line %d: committed %r, generated %r" % (name, first + 1, (a[first] if first < len(a) else "<end>")[:110], (b[first] if first < len(b) else "<end>")[:110]))
        print("mac-release-parity-check: %s" % ("OK" if not bad else "FAILED"))
        return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
