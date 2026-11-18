#!/bin/sh
# A tiny gate with a self-test, the target of scripts/shell-mutants-test.sh (nothing else uses it).
[ "${1:-}" = --selftest ] || exit 64
size() { if [ "$1" -gt 5 ]; then echo big; else echo small; fi; }
[ "$(size 9)" = big ] || { echo "GATE LOGIC WRONG: 9 should be big"; exit 1; }
[ "$(size 1)" = small ] || { echo "GATE LOGIC WRONG: 1 should be small"; exit 1; }
echo "fixture self-test ok"
