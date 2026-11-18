#!/bin/sh
# A nested machine output failure stops without attempting another diagnostic.
set -eu
source_file=${1:?include required}
scratch=${TMPDIR:?scratch required}
case $scratch in /tmp|/tmp/*|/var/tmp|/var/tmp/*) exit 2 ;; esac
work=$(mktemp -d "$scratch/run-machine-status.XXXXXXXX")
trap 'rm -rf -- "$work"' EXIT HUP INT TERM
printf '%s\n' '#!/bin/sh' 'exit 99' > "$work/sudo"
chmod 700 "$work/sudo"
sed "s|/usr/bin/sudo|$work/sudo|g" "$source_file" > "$work/include.sh"
for machine_status in 3 2; do
  rm -f "$work/diagnostic"
  actual=0
  FAKE_WORK=$work FAKE_MACHINE_STATUS=$machine_status sh -c '
    . "$1"
    EX_PREREQ=3 EX_MACHINE=2 CORTEX_YES=1
    PREREQ_MISSING="podman-compose (requalify)" PREREQ_FIXES=linger
    id() { [ "$1" = -un ] && printf "%s\n" cortexuser; }
    say() { printf "%s\n" "$*"; }
    prereq_requalify_compose() { PREREQ_COMPOSE=/fake/provider; }
    prereq_confirm() { return 0; }
    prereq_fix_linger() { return 0; }
    machine_look() { return "$FAKE_MACHINE_STATUS"; }
    prereq_msg() { : > "$FAKE_WORK/diagnostic"; }
    status=0
    prereq_run || status=$?
    exit "$status"
  ' sh "$work/include.sh" > "$work/out" 2>&1 || actual=$?
  if [ "$actual" -ne 3 ]; then printf 'FAIL F-7 nested machine %s returned %s\n' "$machine_status" "$actual" >&2; cat "$work/out" >&2; exit 1; fi
  if [ "$machine_status" = 3 ] && [ -e "$work/diagnostic" ]; then printf '%s\n' 'FAIL F-7: another diagnostic followed a machine output failure' >&2; exit 1; fi
  if [ "$machine_status" = 2 ] && [ ! -e "$work/diagnostic" ]; then printf '%s\n' 'FAIL F-7: ordinary machine refusal lost its diagnostic' >&2; exit 1; fi
  printf 'PASS F-7 nested machine status %s\n' "$machine_status"
done
