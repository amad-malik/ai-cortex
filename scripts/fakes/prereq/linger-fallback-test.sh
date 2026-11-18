#!/bin/sh
# F-3: fake loginctl and sudo, including a denied fallback, never host services.
set -eu
: "${TMPDIR:?private scratch required}"
case $TMPDIR in /tmp|/tmp/*) exit 2 ;; esac
work=$(mktemp -d "$TMPDIR/prereq-linger.XXXXXXXX")
trap 'rm -rf -- "$work"' EXIT HUP INT TERM
sed 's|/usr/bin/sudo|sudo|g' "$1" > "$work/include.sh"
. "$work/include.sh"
loginctl() {
  case ${1-} in
    show-user) if [ -f "$work/linger" ]; then printf '%s\n' yes; else printf '%s\n' no; fi ;;
    enable-linger) printf '%s\n' user >> "$work/calls"; [ "$mode" != user ] || : > "$work/linger" ;;
    *) return 1 ;;
  esac
}
sudo() {
  printf '%s\n' sudo >> "$work/calls"
  [ "$mode" = fallback ] || return 1
  : > "$work/linger"
}
for mode in user fallback denied; do
  rm -f "$work/linger" "$work/calls"
  status=0; prereq_fix_linger cortexuser > "$work/out" 2>&1 || status=$?
  case $mode in
    user) [ "$status" = 0 ] && [ "$(cat "$work/calls")" = user ] || { printf '%s\n' 'FAIL F-3: successful user attempt still invoked sudo or failed' >&2; exit 1; } ;;
    fallback) [ "$status" = 0 ] && [ "$(cat "$work/calls")" = "$(printf 'user\nsudo')" ] && [ -f "$work/linger" ] || { printf '%s\n' 'FAIL F-3: sudo fallback did not enable linger' >&2; exit 1; } ;;
    denied) [ "$status" = 3 ] && [ "$(cat "$work/calls")" = "$(printf 'user\nsudo')" ] || { printf '%s\n' 'FAIL F-3: denied fallback not reported' >&2; exit 1; } ;;
  esac
done
printf '%s\n' 'PASS F-3: user success skips sudo; fallback and denial are verified'
