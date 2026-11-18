#!/bin/sh
# Synthetic logs only: prove ignored mutating requests cannot pass the check.
set -eu
: "${TMPDIR:?private scratch required}"
case $TMPDIR in /tmp|/tmp/*) exit 2 ;; esac
root=$1
work=$(mktemp -d "$TMPDIR/prereq-readonly.XXXXXXXX")
trap 'rm -rf -- "$work"' EXIT HUP INT TERM
printf '%s\n' 'loginctl <show-user> <cortexuser> <-p> <Linger> <--value>' 'podman <--version>' 'podman <info> <--format> <{{.Store.GraphRoot}}>' > "$work/safe"
printf '%s\n' 'loginctl <enable-linger> <cortexuser>' > "$work/loginctl"
printf '%s\n' 'podman <rm> <--force> <victim>' > "$work/podman"
# The former deny-list misses both forbidden lines.
if grep -Eq '^(dnf|sudo|usermod|systemctl|systemd-run|systemd-analyze|gpg|curl|wget|pip|pip3|podman-compose|setenforce|semanage|chcon|restorecon)( |$)' "$work/loginctl" "$work/podman"; then
  printf '%s\n' 'FAIL SAM-02: legacy regression premise changed' >&2; exit 1
fi
checker=$root/scripts/fakes/prereq/check-readonly-log.sh
if [ ! -f "$checker" ]; then printf '%s\n' 'FAIL SAM-02: read-only log checker missing' >&2; exit 1; fi
sh "$checker" "$work/safe" || { printf '%s\n' 'FAIL SAM-02: safe requests refused' >&2; exit 1; }
for bad in "$work/loginctl" "$work/podman"; do
  if sh "$checker" "$bad"; then printf 'FAIL SAM-02: mutating request accepted: %s\n' "${bad##*/}" >&2; exit 1; fi
done
printf '%s\n' 'PASS SAM-02: mutating loginctl and podman requests are refused even if ignored'
