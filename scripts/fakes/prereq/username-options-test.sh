#!/bin/sh
# F-4: stub all effects, verify user names cannot become command options.
set -eu
: "${TMPDIR:?private scratch required}"
case $TMPDIR in /tmp|/tmp/*) exit 2 ;; esac
work=$(mktemp -d "$TMPDIR/prereq-user.XXXXXXXX")
trap 'rm -rf -- "$work"' EXIT HUP INT TERM
sed 's|/usr/bin/sudo|sudo|g' "$1" > "$work/include.sh"
. "$work/include.sh"
prereq_subid_range() { printf '%s\n' 100000-165535; }
prereq_subid_has_range() { return 0; }
loginctl() { case $1 in show-user) [ ! -f "$work/linger" ] || { printf '%s\n' yes; return; }; printf '%s\n' no ;; enable-linger) : ;; *) return 1 ;; esac; }
sudo() { printf '%s\n' "$*" >> "$work/calls"; [ "$1" != loginctl ] || : > "$work/linger"; }
status=0; prereq_fix_subids -bad 100000-165535 1 1 > "$work/out" 2>&1 || status=$?
[ "$status" = 3 ] && [ ! -f "$work/calls" ] || { printf '%s\n' 'FAIL F-4: leading-hyphen subid user accepted' >&2; exit 1; }
status=0; prereq_fix_linger -bad > "$work/out" 2>&1 || status=$?
[ "$status" = 3 ] && [ ! -f "$work/calls" ] || { printf '%s\n' 'FAIL F-4: leading-hyphen linger user accepted' >&2; exit 1; }
prereq_fix_subids cortexuser 100000-165535 1 1
prereq_fix_linger cortexuser
grep -Fxq 'usermod --add-subuids 100000-165535 --add-subgids 100000-165535 -- cortexuser' "$work/calls" || { printf '%s\n' 'FAIL F-4: usermod argument lacks delimiter' >&2; exit 1; }
grep -Fxq 'loginctl enable-linger -- cortexuser' "$work/calls" || { printf '%s\n' 'FAIL F-4: loginctl argument lacks delimiter' >&2; exit 1; }
printf '%s\n' 'PASS F-4: leading-hyphen names refused and privileged user arguments delimited'
