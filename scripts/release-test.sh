#!/bin/sh
# Release builder tests. TMPDIR must be a private directory under /home, never /tmp.
set -eu
: "${TMPDIR:?set TMPDIR to scratch under /home}"
exec python3 -I -B "$(dirname "$0")/release-fixtures/test_release.py" "$@"
