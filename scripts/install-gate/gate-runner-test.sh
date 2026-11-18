#!/bin/sh
# The cells GR-nn of the 6f gate runner (nix). Run through cv2-admit:  cv2-admit sh scripts/install-gate/gate-runner-test.sh [GR-01 ...]
# Fakes only (ssh, hostname, ip, getent first on PATH): no network, no real host. Scratch under GR_TMP (default ~/.cache/cortex-install-gate-tests), never /tmp.
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
exec /usr/bin/python3 -I "$here/test_gate_runner.py" "$@"
