#!/bin/sh
# The ci.sh test gate (hue; cox W1-103 (2), W1-106): every workspace test RUNS; the database tests under their own supervised runs. See test_gate.py.
#   ci.sh:  gate test blocking sh scripts/test-gate/test-gate.sh      (NOT under an outer cv2-admit: the database drivers take their own admission)
# Exit 0 pass, 1 fail, 77 not run (the last output line is then exactly  NOT_RUN: <reason>).
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
exec /usr/bin/python3 -I -B "$here/test_gate.py" "$@"
