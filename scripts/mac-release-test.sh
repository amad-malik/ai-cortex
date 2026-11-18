#!/bin/sh
# Mac release recipe tests. TMPDIR must be a private directory under /home, never /tmp.
set -eu
: "${TMPDIR:?set TMPDIR to scratch under /home}"
exec python3 -I -B "$(dirname "$0")/mac-release-fixtures/test_mac_release.py" "$@"
