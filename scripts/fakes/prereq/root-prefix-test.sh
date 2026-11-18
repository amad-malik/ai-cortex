#!/bin/sh
# F-6: call public entry wrappers with pure stub bodies, no real host operation.
set -eu
: "${TMPDIR:?private scratch required}"
case $TMPDIR in /tmp|/tmp/*) exit 2 ;; esac
. "$1"
probe() { [ "${PREREQ_ROOT+x}" != x ] || { printf '%s\n' 'FAIL F-6: caller-controlled root prefix reached production entry' >&2; return 1; }; }
machine_look_body() { probe; }
prereq_run_body() { probe; }
prereq_requalify_compose_body() { probe; }
PREREQ_ROOT=/attacker-chosen; export PREREQ_ROOT
[ -z "$(prereq_system_root)" ] || { printf '%s\n' 'FAIL F-6: system root helper accepted injected prefix' >&2; exit 1; }
machine_look
prereq_run
prereq_requalify_compose
[ "$PREREQ_ROOT" = /attacker-chosen ] || { printf '%s\n' 'FAIL F-6: caller environment not restored' >&2; exit 1; }
printf '%s\n' 'PASS F-6: production entry points ignore injected root prefix'
