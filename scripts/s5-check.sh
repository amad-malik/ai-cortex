#!/bin/sh
# S5 static checker (plan S5-PLAN.md section 4, step 1). Text only: it reads files, it starts no container and no build.
#   scripts/s5-check.sh <dir>         check one deploy directory; exit 0 only if every rule passes
#   scripts/s5-check.sh --self-test   run the good fixture and every one-fault mutation of it
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
if [ "${1:-}" = "--self-test" ]; then
  exec python3 -I -B "$here/s5_selftest.py"
fi
exec python3 -I -B "$here/s5_check.py" "$@"
