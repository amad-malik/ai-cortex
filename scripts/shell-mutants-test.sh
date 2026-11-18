#!/bin/sh
# Logic test for scripts/shell_mutants.py (hale N-3 = N35): with a "kill_marker" in the spec, a mutant counts as killed only when the
# command failed AND printed that marker, so a script that merely stopped working (a syntax error) is not a kill. Without a marker the
# old rule holds. Run it through cv2-admit like any test:  cv2-admit sh scripts/shell-mutants-test.sh
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
fx=$here/shell-mutants-fixtures
T=$(mktemp -d); trap 'rm -rf "$T"' EXIT
pass=0 fail=0
ok()  { pass=$((pass + 1)); echo "ok   $1"; }
bad() { fail=$((fail + 1)); echo "FAIL $1"; [ -z "${2:-}" ] || echo "       $2"; }
runner() { python3 -I -B "$here/shell_mutants.py" "$1" > "$T/out" 2>&1 && rc=0 || rc=$?; }

# SM-1 the unmutated fixture passes, a real fault is a kill, a syntax error is NOT a kill, a fault no check notices survives
runner "$fx/spec.json"
if [ "$rc" = 1 ] && grep -q '^baseline: the unmutated' "$T/out" \
   && grep -E '^FX-1 ' "$T/out" | grep -q ' OK ' \
   && grep -E '^FX-2 ' "$T/out" | grep -q 'NOT A KILL' && ! grep -E '^FX-2 ' "$T/out" | grep -q ' OK ' \
   && grep -E '^FX-3 ' "$T/out" | grep -q 'SURVIVED' \
   && grep -q '3 mutants, 2 not caught or broken' "$T/out"; then ok "SM-1 a failing exit without the marker is not a kill; a real fault is; a surviving one is named"; else bad "SM-1 a failing exit without the marker is not a kill; a real fault is; a surviving one is named" "rc=$rc: $(tail -5 "$T/out")"; fi

# SM-2 only real faults: every mutant is a kill and the run passes
runner "$fx/spec-good.json"
if [ "$rc" = 0 ] && [ "$(grep -cE '^FX-[0-9] +OK ' "$T/out")" = 2 ] && grep -q '2 mutants, 0 not caught or broken' "$T/out"; then ok "SM-2 two faults the self-test reports: both killed, the run passes"; else bad "SM-2 two faults the self-test reports: both killed, the run passes" "rc=$rc: $(tail -4 "$T/out")"; fi

# SM-3 a spec with no marker keeps the old rule: any failing exit is a kill
runner "$fx/spec-nomarker.json"
if [ "$rc" = 0 ] && grep -E '^FX-2 ' "$T/out" | grep -q ' OK ' && grep -q '1 mutants, 0 not caught or broken' "$T/out"; then ok "SM-3 without a kill_marker any failing exit is a kill, as before"; else bad "SM-3 without a kill_marker any failing exit is a kill, as before" "rc=$rc: $(tail -3 "$T/out")"; fi

# SM-4 the real spec asks for the marker
if grep -q '"kill_marker": "GATE LOGIC WRONG"' "$here/ci-mutants.json"; then ok "SM-4 scripts/ci-mutants.json asks for the marker"; else bad "SM-4 scripts/ci-mutants.json asks for the marker" "no kill_marker line"; fi

# SM-5 (hale N-2) a spec key the driver does not know is refused: a misspelt kill_marker must not silently turn the rule off
sed 's/"kill_marker"/"killmarker"/' "$fx/spec.json" > "$T/spec-typo.json"
runner "$T/spec-typo.json"
if [ "$rc" = 2 ] && grep -q 'unknown key' "$T/out" && ! grep -q '^FX-' "$T/out"; then ok "SM-5 an unknown key in the spec is refused before any mutant runs"; else bad "SM-5 an unknown key in the spec is refused before any mutant runs" "rc=$rc: $(head -3 "$T/out")"; fi

# SM-6 (hale N-3) an empty kill_marker is refused: it must not silently turn the N35 rule off (an empty text is "in" every output)
sed 's/"kill_marker": "[^"]*"/"kill_marker": ""/' "$fx/spec.json" > "$T/spec-empty.json"
runner "$T/spec-empty.json"
if [ "$rc" = 2 ] && grep -q 'kill_marker' "$T/out" && ! grep -q '^FX-' "$T/out"; then ok "SM-6 an empty kill_marker is refused before any mutant runs"; else bad "SM-6 an empty kill_marker is refused before any mutant runs" "rc=$rc: $(tail -3 "$T/out")"; fi

echo "shell-mutants-test: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
