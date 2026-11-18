#!/usr/bin/env python3
"""Exercise confirmation against a private pseudo-terminal, never host tools."""

import os
import pathlib
import pty
import select
import subprocess
import sys
import termios


if len(sys.argv) not in (5, 6) or (len(sys.argv) == 6 and sys.argv[5] not in ("C", "C.utf8")):
    raise SystemExit("FAIL PRQ-SAM01: invalid helper arguments")
include, root, shell, fake_path = sys.argv[1:5]
locale = sys.argv[5] if len(sys.argv) > 5 else "C"
root = pathlib.Path(root)
(root / "dev").mkdir(parents=True, exist_ok=True)
tty = root / "dev" / "tty"

for answer, expected in [("y", 0), ("YES", 0), ("", 2), ("n", 2), ("perhaps", 2)]:
    master, slave = pty.openpty()
    settings = termios.tcgetattr(slave)
    settings[3] &= ~termios.ECHO
    termios.tcsetattr(slave, termios.TCSANOW, settings)
    tty.symlink_to(os.ttyname(slave))
    env = {"PATH": fake_path, "LC_ALL": locale, "PREREQ_ROOT": str(root), "CORTEX_YES": "0"}
    proc = subprocess.Popen(
        [shell, "-c", '. "$1"; say() { printf "%s\\n" "$*"; }; EX_MACHINE=2; prereq_confirm', "sh", include],
        env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    )
    ready, _, _ = select.select([proc.stdout], [], [], 5)
    if not ready:
        proc.kill()
        proc.communicate()
        raise SystemExit("FAIL IP-23/25: prompt did not appear")
    prompt = proc.stdout.readline()
    os.write(master, (answer + "\n").encode())
    out, err = proc.communicate(input=b"y\n", timeout=5)
    tty.unlink()
    os.close(master)
    os.close(slave)
    if proc.returncode != expected or (prompt + out).count(b"Proceed") != 1:
        raise SystemExit(f"FAIL IP-23/25: {answer!r} returned {proc.returncode}; output={prompt + out!r}, error={err!r}")

for yes, expected in [("0", 2), ("1", 0)]:
    env = {"PATH": fake_path, "LC_ALL": locale, "PREREQ_ROOT": str(root), "CORTEX_YES": yes}
    proc = subprocess.run(
        [shell, "-c", '. "$1"; say() { printf "%s\\n" "$*"; }; EX_MACHINE=2; prereq_confirm', "sh", include],
        env=env, input=b"y\n", stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=5,
    )
    if proc.returncode != expected or b"Proceed" in proc.stdout:
        raise SystemExit(f"FAIL IP-24/26: --yes={yes} returned {proc.returncode}")
print("ok IP-23/24/25/26 one confirmation uses the private tty")
