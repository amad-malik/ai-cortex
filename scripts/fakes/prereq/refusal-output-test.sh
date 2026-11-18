#!/bin/sh
# Each half of a cause-and-fix refusal must detect a failed write.
set -eu
source_file=${1:?include required}
scratch=${TMPDIR:?scratch required}
case $scratch in /tmp|/tmp/*|/var/tmp|/var/tmp/*) exit 2 ;; esac
work=$(mktemp -d "$scratch/refusal-output.XXXXXXXX")
trap 'rm -rf -- "$work"' EXIT HUP INT TERM
for target in problem fix; do
  actual=0
  FAKE_WORK=$work FAKE_TARGET=$target sh -c '
    . "$1"
    EX_PREREQ=3
    printf() {
      case $1 in
        "problem: %s\n") label=problem ;;
        "fix: %s\n") label=fix ;;
        *) command printf "$@"; return "$?" ;;
      esac
      : > "$FAKE_WORK/seen-$label"
      if [ "$label" = "$FAKE_TARGET" ]; then command printf "$@" > /dev/full; else command printf "$@"; fi
    }
    status=0
    prereq_msg compose_missing || status=$?
    exit "$status"
  ' sh "$source_file" > "$work/out" 2>&1 || actual=$?
  if [ "$actual" -ne 3 ] || [ ! -f "$work/seen-$target" ]; then
    printf 'FAIL F-7 refusal: %s returned %s or was not reached\n' "$target" "$actual" >&2
    cat "$work/out" >&2
    exit 1
  fi
  if [ "$target" = problem ] && [ -f "$work/seen-fix" ]; then
    printf '%s\n' 'FAIL F-7 refusal: fix written after failed problem write' >&2
    exit 1
  fi
  printf 'PASS F-7 refusal: %s write failure returned 3\n' "$target"
  rm -f "$work/seen-problem" "$work/seen-fix"
done
