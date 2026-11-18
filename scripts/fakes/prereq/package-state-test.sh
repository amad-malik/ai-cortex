#!/bin/sh
# SAM-03: exercise only the fake DNF/RPM protocol in an isolated directory.
set -eu
: "${TMPDIR:?private scratch required}"
case $TMPDIR in /tmp|/tmp/*) exit 2 ;; esac
root=$1
work=$(mktemp -d "$TMPDIR/prereq-package-state.XXXXXXXX")
trap 'rm -rf -- "$work"' EXIT HUP INT TERM
mkdir -p "$work/bin" "$work/state" "$work/root/packages"
ln -s "$root/scripts/fakes/prereq/fake-command" "$work/bin/dnf"
ln -s "$root/scripts/fakes/prereq/fake-command" "$work/bin/rpm"
: > "$work/root/packages/dep1.rpm"
: > "$work/root/packages/dep2.rpm"
FAKE_LOG=$work/log FAKE_STATE_DIR=$work/state FAKE_ROOT_STAGE=$work/root FAKE_ALLOW_ROCKY_INSTALL=1
export FAKE_LOG FAKE_STATE_DIR FAKE_ROOT_STAGE FAKE_ALLOW_ROCKY_INSTALL
"$work/bin/dnf" --releasever=10.2 --disablerepo='*' install -y "$work/root/packages/dep1.rpm"
"$work/bin/rpm" -q --queryformat identity dependency-one > "$work/one"
[ "$(cat "$work/one")" = 'dependency-one|0|1.0|1.el10|noarch' ] || { printf '%s\n' 'FAIL SAM-03: installed identity wrong' >&2; exit 1; }
if "$work/bin/rpm" -q --queryformat identity dependency-two > "$work/two"; then printf '%s\n' 'FAIL SAM-03: uninstalled dependency reported installed' >&2; exit 1; fi
"$work/bin/dnf" --releasever=10.2 --disablerepo='*' install -y "$work/root/packages/dep2.rpm"
"$work/bin/rpm" -q --queryformat identity dependency-two > "$work/two"
[ "$(cat "$work/two")" = 'dependency-two|0|1.0|1.el10|noarch' ] || { printf '%s\n' 'FAIL SAM-03: resumed dependency identity wrong' >&2; exit 1; }
printf '%s\n' 'PASS SAM-03: fake installed identities follow individual archives and resume'
