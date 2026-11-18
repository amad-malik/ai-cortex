#!/bin/sh
# The two provider-path writes map a full output device to EX_PREREQ.
set -eu
source_file=${1:?include required}
scratch=${TMPDIR:?scratch required}
case $scratch in /tmp|/tmp/*|/var/tmp|/var/tmp/*) exit 2 ;; esac
work=$(mktemp -d "$scratch/requal-provider.XXXXXXXX")
trap 'rm -rf -- "$work"' EXIT HUP INT TERM
printf '%s\n' '#!/bin/sh' 'exit 99' > "$work/provider"
chmod 700 "$work/provider"
for kind in inner path; do
  actual=0
  FAKE_WORK=$work FAKE_KIND=$kind sh -c '
    . "$1"
    EX_PREREQ=3
    prereq_requalify_compose_inner_body() { pq_requalify_provider=$FAKE_WORK/provider; }
    status=0
    if [ "$FAKE_KIND" = inner ]; then
      prereq_requalify_compose_inner > /dev/full || status=$?
    else
      prereq_requalify_compose_inner() { printf "%s\n" "$FAKE_WORK/provider"; }
      prereq_requalify_compose || exit "$?"
      prereq_compose_path > /dev/full || status=$?
    fi
    exit "$status"
  ' sh "$source_file" > "$work/out" 2>&1 || actual=$?
  if [ "$actual" -ne 3 ]; then
    printf 'FAIL F-7 provider write: %s returned %s, expected 3\n' "$kind" "$actual" >&2
    cat "$work/out" >&2
    exit 1
  fi
  printf 'PASS F-7 provider write: %s returned 3\n' "$kind"
done
