#!/bin/sh
# Failure to construct the locally derived key record must not verify a key.
set -eu
source_file=${1:?include required}
scratch=${TMPDIR:?scratch required}
case $scratch in /tmp|/tmp/*|/var/tmp|/var/tmp/*) exit 2 ;; esac
work=$(mktemp -d "$scratch/key-record-output.XXXXXXXX")
trap 'rm -rf -- "$work"' EXIT HUP INT TERM
# Isolated fixture forces the no-gpg path without changing the host PATH.
sed 's/if ! command -v gpg >\/dev\/null 2>\&1; then/if true; then/' "$source_file" > "$work/include.sh"
actual=0
FAKE_WORK=$work sh -c '
  . "$1"
  prereq_pgp_fingerprints() { command printf "%s\n" 7D8D15CBFC4E62688591FB2633D98517E37ED158; }
  printf() {
    case ${2-} in
      pub::::::::::) : > "$FAKE_WORK/hit"; command printf "$@" > /dev/full ;;
      *) command printf "$@" ;;
    esac
  }
  status=0
  prereq_epel_key_ok /fake/gnupg /fake/certificate || status=$?
  exit "$status"
' sh "$work/include.sh" > "$work/out" 2>&1 || actual=$?
if [ "$actual" -ne 1 ] || [ ! -f "$work/hit" ]; then
  printf 'FAIL F-7 key records: failed record construction returned %s or was not reached\n' "$actual" >&2
  cat "$work/out" >&2
  exit 1
fi
printf '%s\n' 'PASS F-7 key records: failed construction rejects verification'
