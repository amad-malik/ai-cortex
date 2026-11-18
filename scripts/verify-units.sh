#!/bin/sh
# scripts/verify-units.sh <unit-file>...   `systemd-analyze --user verify` on the units in the CURRENT directory, WITHOUT touching the live user manager.
#
# WHY (kai 13:30Z, 7 Nov 2026): `systemd-analyze --user verify` starts a small manager of its own that UNLINKS and RE-BINDS
# $XDG_RUNTIME_DIR/systemd/private, the control socket of the user's LIVE systemd manager. Run with the real runtime directory (/run/user/<uid>) it broke
# `systemctl --user` and `systemd-run --user --scope` for the whole VM, so no new terminal could be created. So the call ALWAYS gets a private, temporary runtime
# directory (mode 0700) that is removed afterwards, even when the tool fails or the run is interrupted, and no session bus. This script is the ONLY place in
# our scripts that calls `systemd-analyze verify`; test IT-76 holds it (against a simulated socket) and IT-66 checks the real socket's inode around the call.
#
# The exit status is systemd-analyze's own. No unit given: exit 2.
set -eu
[ $# -gt 0 ] || { echo "verify-units: no unit files given" >&2; exit 2; }
priv=$(mktemp -d "${TMPDIR:-/tmp}/verify-units.XXXXXX") || { echo "verify-units: cannot make a private runtime directory" >&2; exit 2; }
chmod 0700 "$priv"
cleanup() { case $priv in */verify-units.??????) rm -rf -- "$priv" ;; esac; }
trap cleanup EXIT
trap 'cleanup; exit 130' INT
trap 'cleanup; exit 143' TERM
XDG_RUNTIME_DIR=$priv DBUS_SESSION_BUS_ADDRESS= systemd-analyze --user --man=no verify "$@"
