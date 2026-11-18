#!/bin/sh
# The cells TG-nn of the ci.sh test gate (hue; cox W1-103 (2), W1-106). Run through cv2-admit:  cv2-admit sh scripts/test-gate/test-gate-test.sh [-k pattern]
# Fakes only (cargo, the admission program, supervise.sh): nothing is built, no database, no podman. Scratch under TMPDIR (required, never /tmp).
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
: "${TMPDIR:?set TMPDIR to private scratch under /home}"
case $TMPDIR in /tmp|/tmp/*) echo 'test-gate-test: TMPDIR under /tmp is forbidden' >&2; exit 2 ;; esac
exec /usr/bin/python3 -I -B "$here/test_test_gate.py" "$@"
