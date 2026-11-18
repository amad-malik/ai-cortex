#!/bin/sh
# W1-73: a real repair plan states the executable environment before consent.
set -eu
source_file=$1
scratch=${TMPDIR:?}
case $scratch in /tmp|/tmp/*) exit 2 ;; esac
work=$(mktemp -d "$scratch/plan-header.XXXXXXXX")
trap 'rm -rf -- "$work"' EXIT HUP INT TERM
printf '#!/bin/sh\nexit 99\n' > "$work/sudo"
chmod 700 "$work/sudo"
sed "s|\[ -x /usr/bin/sudo \]|[ -x $work/sudo ]|g" "$source_file" > "$work/source.sh"
status=0
sh -c '
  . "$1"
  say() { printf "%s\n" "$*"; }
  warn() { :; }
  id() { [ "$1" = -un ] && printf "%s\n" cortexuser; }
  prereq_requalify_compose() { PREREQ_COMPOSE=/fake/provider; }
  prereq_confirm() { printf "%s\n" CONSENT_BOUNDARY; return 2; }
  PREREQ_MISSING="podman-compose (requalify)" PREREQ_FIXES=linger
  EX_MACHINE=2 EX_PREREQ=3
  prereq_run_body
' sh "$work/source.sh" > "$work/out" 2>&1 || status=$?
if [ "$status" -ne 2 ] || ! grep -Fxq 'Privileged commands use /usr/bin/sudo and PATH=/usr/bin:/usr/sbin; package and account changes use umask 022.' "$work/out" ||
   ! awk '/^Privileged commands use / { header=NR } /^CONSENT_BOUNDARY$/ { consent=NR } END { exit !(header && consent && header < consent) }' "$work/out"; then
  printf '%s\n' 'FAIL W1-73: privilege execution context missing before consent' >&2
  exit 1
fi
printf '%s\n' 'PASS W1-73: privilege execution context disclosed before consent'
