#!/bin/sh
# F-7: an output failure in a conditional repair call must stop before consent.
set -eu
source_file=$1
scratch=${TMPDIR:?}
case $scratch in /tmp|/tmp/*) exit 2 ;; esac
work=$(mktemp -d "$scratch/plan-output.XXXXXXXX")
trap 'rm -rf -- "$work"' EXIT HUP INT TERM
printf '#!/bin/sh\nexit 99\n' > "$work/sudo"
chmod 700 "$work/sudo"
sed "s|\[ -x /usr/bin/sudo \]|[ -x $work/sudo ]|g" "$source_file" > "$work/source.sh"
status=0
FAKE_TEST_DIR=$work sh -c '
  . "$1"
  count=0
  say() { count=$((count + 1)); [ "$count" -ne 3 ] || return 1; printf "%s\n" "$*"; }
  warn() { :; }
  id() { [ "$1" = -un ] && printf "%s\n" cortexuser; }
  prereq_requalify_compose() { PREREQ_COMPOSE=/fake/provider; }
  prereq_confirm() { : > "$FAKE_TEST_DIR/consent"; return 2; }
  PREREQ_MISSING="podman-compose (requalify)" PREREQ_FIXES=linger
  EX_MACHINE=2 EX_PREREQ=3
  if prereq_run; then exit 90; else exit "$?"; fi
' sh "$work/source.sh" > "$work/out" 2>&1 || status=$?
if [ "$status" -ne 3 ] || [ -e "$work/consent" ]; then
  printf 'FAIL F-7: repair plan output failure reached consent or returned %s\n' "$status" >&2
  exit 1
fi
printf 'yes\n' > "$work/tty"
status=0
FAKE_TEST_DIR=$work sh -c '
  . "$1"
  say() { return 1; }
  prereq_tty_available() { pq_tty=$FAKE_TEST_DIR/tty; return 0; }
  CORTEX_YES=0 EX_PREREQ=3 EX_MACHINE=2
  if prereq_confirm; then exit 90; else exit "$?"; fi
' sh "$work/source.sh" > "$work/out" 2>&1 || status=$?
if [ "$status" -ne 3 ]; then printf 'FAIL F-7: prompt output failure returned %s\n' "$status" >&2; exit 1; fi
printf '%s\n' 'PASS F-7: repair plan and prompt output failures stop before consent'
