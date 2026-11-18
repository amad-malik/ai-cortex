#!/bin/sh
# F-7 r4: receipt writes from the locked Rocky helper stop before fake sudo.
set -eu
source_file=${1:?include required}
scratch=${TMPDIR:?scratch required}
case $scratch in /tmp|/tmp/*|/var/tmp|/var/tmp/*) exit 2 ;; esac
work=$(mktemp -d "$scratch/rocky-output.XXXXXXXX")
trap 'rm -rf -- "$work"' EXIT HUP INT TERM
mkdir -p "$work/stage" "$work/private"
printf '%s\n' '/fixture/pkg.rpm|hash|pkg|0|1|1|x86_64|baseos' > "$work/stage/transaction.manifest"
printf '%s\n' 'fixture root receipt' > "$work/private/roots.receipt"
printf '%s\n' '#!/bin/sh' ': > "$FAKE_WORK/sudo-called"' > "$work/sudo"
chmod 700 "$work/sudo"
sed "s|/usr/bin/sudo|$work/sudo|g" "$source_file" > "$work/include.sh"
run_case() {
  target=$1 expected=$2
  rm -f "$work/sudo-called"
  printf '%s\n' 'fixture root receipt' > "$work/private/roots.receipt"
  [ "$target" != roots_missing ] || rm "$work/private/roots.receipt"
  actual=0
  FAKE_WORK=$work FAKE_TARGET=$target sh -c '
    . "$1"
    EX_PREREQ=3
    prereq_rocky_seal_ok() { return 0; }
    prereq_rocky_candidates() { printf "%s\n" "/fixture/pkg.rpm|hash|pkg|0|1|1|x86_64"; }
    prereq_rocky_metadata_ok() { printf "%s\n" "fixture metadata"; }
    prereq_rocky_archives_ok() { printf "%s\n" "fixture archives"; }
    say() {
      case $FAKE_TARGET:$* in
        roots:"fixture root receipt"|metadata:"fixture metadata"|archives:"fixture archives"|verified:"Verified Rocky install:"*) printf "%s\n" "$*" > /dev/full ;;
        *) printf "%s\n" "$*" ;;
      esac
    }
    status=0
    prereq_rocky_install "$FAKE_WORK/stage" "$FAKE_WORK/private" 10.2 fake root || status=$?
    exit "$status"
  ' sh "$work/include.sh" > "$work/out" 2>&1 || actual=$?
  if [ "$actual" -ne "$expected" ]; then
    printf 'FAIL F-7 r4: Rocky %s returned %s, expected %s\n' "$target" "$actual" "$expected" >&2
    cat "$work/out" >&2
    return 1
  fi
  if [ "$target" = roots_missing ] && { ! grep -q '^problem:' "$work/out" || ! grep -q '^fix:' "$work/out"; }; then
    printf '%s\n' 'FAIL F-7 r4: missing Rocky receipt lacked cause and fix' >&2
    return 1
  fi
  if [ "$target" = none ]; then
    [ -f "$work/sudo-called" ] || { printf '%s\n' 'FAIL F-7 r4: fake baseline did not reach fake sudo' >&2; return 1; }
  elif [ -e "$work/sudo-called" ]; then
    printf 'FAIL F-7 r4: Rocky %s reached fake sudo after output failure\n' "$target" >&2
    return 1
  fi
  printf 'PASS F-7 r4: Rocky %s returned %s\n' "$target" "$actual"
}
failures=0
run_case none 0 || failures=$((failures + 1))
for target in roots roots_missing metadata archives verified; do run_case "$target" 3 || failures=$((failures + 1)); done
[ "$failures" -eq 0 ]
