#!/bin/bash
# probe-fail-compat.sh: a fixture of mac-port-test.sh. It fails (exit 7) only when BASH_COMPAT is 32, so a runner that stops at the first
# shape, or that ignores a failing shape, is caught. It changes nothing.
if [ "${BASH_COMPAT:-}" = 32 ]; then
  echo "probe-fail-compat: failing under BASH_COMPAT=32" >&2
  exit 7
fi
echo "probe-fail-compat: ok"
