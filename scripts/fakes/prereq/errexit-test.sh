#!/bin/sh
# F-7: bare calls preserve the caller's errexit semantics; conditional calls
# retain the shell's normal conditional semantics. No host tools are called.
set -eu
source_file=$1
scratch=${TMPDIR:?}
case $scratch in /tmp|/tmp/*) exit 2 ;; esac
for pair in 'machine_look machine_look_body' 'prereq_run prereq_run_body' 'prereq_requalify_compose prereq_requalify_compose_body'; do
  set -- $pair
  wrapper=$1 body=$2
  out=$scratch/errexit-$$-$wrapper.out
  status=0
  sh -c 'set -e; . "$1"; eval "$2() { false; printf CONTINUED; }"; "$3"; printf RETURNED' sh "$source_file" "$body" "$wrapper" > "$out" 2>&1 || status=$?
  if [ "$status" -eq 0 ] || grep -Eq 'CONTINUED|RETURNED' "$out"; then
    rm -f "$out"
    printf 'FAIL F-7: bare %s ignored errexit in its body\n' "$wrapper" >&2
    exit 1
  fi
  status=0
  sh -c 'set -e; . "$1"; eval "$2() { false; printf CONTINUED; }"; if "$3"; then printf RETURNED; fi' sh "$source_file" "$body" "$wrapper" > "$out" 2>&1 || status=$?
  if [ "$status" -ne 0 ] || [ "$(cat "$out")" != CONTINUEDRETURNED ]; then
    rm -f "$out"
    printf 'FAIL F-7: conditional %s did not retain normal shell semantics\n' "$wrapper" >&2
    exit 1
  fi
  rm -f "$out"
done
printf '%s\n' 'PASS F-7: three bare wrappers preserve errexit; conditional calls retain shell semantics'
