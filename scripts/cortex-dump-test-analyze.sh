#!/bin/sh
# cortex-dump-test-analyze.sh <systemd-analyze arguments...>   run `systemd-analyze` WITHOUT touching the live user manager (piece 12 test helper; bob)
#
# WHY (kai, 13:30Z, 7 Nov 2026): `systemd-analyze --user verify` starts a small manager of its own that UNLINKS and RE-BINDS
# $XDG_RUNTIME_DIR/systemd/private, the control socket of the user's LIVE systemd manager. Run with the real runtime directory (/run/user/<uid>) it
# broke `systemctl --user` and `systemd-run --user` for the whole VM, so no new terminal could be created. So every call here gets a PRIVATE, temporary
# runtime directory (mode 0700) that is removed afterwards, even when the tool fails or the run is interrupted, and an EMPTY session bus address.
# This script is the ONLY place in the piece 12 scripts that calls systemd-analyze (BK-58 holds that); BK-57 and BK-59 hold what it does, against a
# stand-in tool, never the real socket.
#
# The directory is made under /tmp on purpose: a unix socket path is limited to 107 bytes, and the test directories of this lane are longer.
# It is a few bytes and is removed by its exact name.
#
# The exit status is systemd-analyze's own. No arguments: exit 2 before anything is made. SIGTERM: 143; SIGINT: 130.
set -eu
[ $# -gt 0 ] || { echo "cortex-dump-test-analyze: no arguments given" >&2; exit 2; }
priv=$(mktemp -d /tmp/cortex-bk-rt.XXXXXX) || { echo "cortex-dump-test-analyze: cannot make a private runtime directory" >&2; exit 2; }
cleanup() { case $priv in /tmp/cortex-bk-rt.??????) rm -rf -- "$priv" ;; esac; } # s5-never: ok the exact name this run made, shape-checked
# the cleanup is registered RIGHT AFTER the directory exists and BEFORE any other setup step, so a step that fails (chmod) cannot leak it (sam BK12-SD-N01)
trap cleanup EXIT
trap 'cleanup; exit 130' INT
trap 'cleanup; exit 143' TERM
chmod 0700 "$priv"
XDG_RUNTIME_DIR=$priv DBUS_SESSION_BUS_ADDRESS= systemd-analyze "$@"
