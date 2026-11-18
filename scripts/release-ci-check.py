#!/usr/bin/env python3
"""Check the CI table and allow only explicitly authorised skipped gates."""
import json
import re
import sys
from pathlib import Path


def check(log_path, allow_path):
    allow = json.loads(Path(allow_path).read_text(encoding="utf-8"))
    if not isinstance(allow, list):
        raise ValueError("allow-list must be a list")
    permitted = {}
    for entry in allow:
        if not isinstance(entry, dict) or set(entry) != {"gate", "reason", "accepted_by", "close_release"}:
            raise ValueError("invalid allow-list entry")
        if any(not isinstance(v, str) or not v.strip() or "\n" in v for v in entry.values()):
            raise ValueError("incomplete allow-list entry")
        if entry["gate"] in permitted:
            raise ValueError("duplicate allow-list gate")
        permitted[entry["gate"]] = entry["reason"]
    lines = Path(log_path).read_text(encoding="utf-8").splitlines()
    if not lines:
        raise ValueError("empty CI log")
    final = lines[-1]
    if final == "ci: PASS":
        expected = 0
    else:
        match = re.fullmatch(r"ci: PASS-WITH-NOT-RUN \(([0-9]+) gates did not run; see NOT_RUN above\)", final)
        if not match:
            raise ValueError("CI did not finish with a passing verdict")
        expected = int(match.group(1))
        if expected == 0:
            raise ValueError("invalid CI skip count")
    seen = set()
    skips = 0
    for line in lines[:-1]:
        match = re.fullmatch(r"([A-Za-z0-9_-]+)\s+(PASS|FAIL|REPORT|NOT_RUN)\s*(.*)", line)
        if not match:
            raise ValueError("invalid CI table line")
        gate, status, reason = match.groups()
        if gate in seen or status == "FAIL":
            raise ValueError("duplicate or failing CI gate: " + gate)
        seen.add(gate)
        if status == "NOT_RUN":
            skips += 1
            if permitted.get(gate) != reason:
                raise ValueError("unapproved CI skip: " + gate)
    if not seen or skips != expected:
        raise ValueError("CI table and verdict do not agree")


if __name__ == "__main__":
    try:
        if len(sys.argv) != 3:
            raise ValueError("usage: release-ci-check.py CI_LOG ALLOW_LIST")
        check(sys.argv[1], sys.argv[2])
    except (OSError, UnicodeError, ValueError) as exc:
        print("release CI check: " + str(exc), file=sys.stderr)
        raise SystemExit(1)
