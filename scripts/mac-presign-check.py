#!/usr/bin/env python3
"""Prove Mac installer cells other than the signed-set cell before signing.

IT-M29 is deferred explicitly; the separate signed-set release check must pass
after signing and before publication. This result is not a full-suite pass.
"""
import os
import re
import subprocess
import sys
from pathlib import Path


def validate(rc, output, required):
    lines = output.splitlines()
    required = [line.strip() for line in required.splitlines()
                if line.strip() and not line.lstrip().startswith("#")]
    if not required or len(required) != len(set(required)) or required.count("IT-M29") != 1:
        raise ValueError("invalid required-cell list")
    observed = [line[5:].split()[0] for line in lines if line.startswith("ok   ") and line[5:].split()]
    skips = [line for line in lines if line.startswith("skip ")]
    summaries = [line for line in lines if line.startswith("install-mac-test: ")]
    summary = re.fullmatch(r"install-mac-test: ([1-9][0-9]*) passed, 0 failed, 1 not run", summaries[0]) if len(summaries) == 1 else None
    if (rc != 77 or any(line.startswith("FAIL ") for line in lines)
            or len(skips) != 1 or not skips[0].startswith("skip IT-M29 NOT RUN: ")
            or not summary or int(summary.group(1)) != len(observed)
            or len(observed) != len(set(observed))
            or set(observed) != set(required) - {"IT-M29"}
            or len(lines) < 2 or lines[-2] != summaries[0]
            or not lines or not re.fullmatch(r"NOT_RUN: 1 mandatory cell\(s\) not run \(IT-M29\): .+", lines[-1])
            or len([line for line in lines if line.startswith("NOT_RUN: ")]) != 1):
        raise ValueError("the suite did not prove all pre-sign cells with only IT-M29 deferred")


def main():
    if len(sys.argv) != 1:
        raise ValueError("no arguments are accepted")
    root = Path(__file__).resolve().parent.parent
    env = {k: v for k, v in os.environ.items() if not k.startswith(("INSTALL_MAC_", "MAC_SET_"))}
    result = subprocess.run(["sh", str(root / "scripts/install-mac-test.sh")], cwd=root,
                            env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                            text=True, timeout=1800)
    sys.stdout.write(result.stdout)
    required = (root / "scripts/install-mac-required-cells.txt").read_text()
    validate(result.returncode, result.stdout, required)
    # Preserve the suite's exact NOT_RUN line as CI's last line and its exit 77.
    raise SystemExit(77)


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, subprocess.TimeoutExpired) as exc:
        print("mac-presign-check: FAIL: " + str(exc), file=sys.stderr)
        raise SystemExit(1)
