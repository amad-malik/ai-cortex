#!/bin/sh
# Cells for scripts/ci-fanout.sh (the gate fan-out: kai's row 4fa9e996, cox's calendar finding c52081ff, W1-109 option A).
# The wrapper splits a mutant gate's ids (read from the SAME spec file the driver loads) into N shards, runs the shards as CONCURRENT processes (each
# one the driver command plus its own ids), waits for ALL, and FAILS the gate if any shard exits nonzero, times out or is killed, or if the ids the
# shards report are not exactly the spec's, each once. These cells use a FAKE driver: they prove the wrapper (split, status, timeout, kill, exactly
# once, union), not that the two real suites are safe side by side. No cargo, no container, no database.
#
#   sh scripts/ci-fanout-test.sh          (FANOUT_SH=<path> names another wrapper: the mutants runner uses it)
#
# Cells FO-01 to FO-34. A failure prints 'FANOUT CELLS FAILED' (the mutants runner's kill marker).
set -eu
unset CI_FANOUT_SHARDS FANOUT_TIMEOUT FANOUT_KILL_GRACE CI_LOG_DIR CI_FANOUT_TEST FANOUT_TEST_LAUNCH_PAUSE FANOUT_TEST_SHARD_PAUSE
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
fanout=${FANOUT_SH:-"$here/ci-fanout.sh"}
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
cd "$T"   # whatever a (mutated) wrapper or the fake does with a relative path lands in the cells' own directory, never in the checkout
pass=0 fail=0
ok()  { pass=$((pass + 1)); echo "ok   $1"; }
bad() { fail=$((fail + 1)); echo "FAIL $1"; [ -z "${2:-}" ] || echo "       $2"; }

# ---- the fake driver: its arguments are the ids it is asked to run. It writes ONE ledger line per id it was asked to run, one result line per id it reports
# (the real drivers print '<id>  <verdict>  [<what>]'), and misbehaves when the shard that holds the id named by one of the FAKE_* variables is this one.
cat > "$T/driver" <<'FAKE'
#!/bin/sh
contains() { want=$1; shift; for a in "$@"; do [ "$a" = "$want" ] && return 0; done; return 1; }
: "${FAKE_LEDGER:?the fake driver did not get its environment}"
echo "baseline: the unmutated fake passes (1 checks)"
[ -z "${FAKE_PIDS:-}" ] || echo "$*" > "$FAKE_LEDGER.pid.$$"
echo "args: $*" >> "$FAKE_LEDGER.args"
echo "pass-through: ${FAKE_PASS_THROUGH:-unset}" >> "$FAKE_LEDGER.args"
for a in "$@"; do echo "$a" >> "$FAKE_LEDGER"; done
[ -z "${FAKE_SLEEP:-}" ] || sleep "$FAKE_SLEEP"
if [ -n "${FAKE_IGNORE_TERM_ID:-}" ] && contains "$FAKE_IGNORE_TERM_ID" "$@"; then trap '' TERM; sleep "${FAKE_HANG_SECONDS:-30}" & wait; fi
if [ -n "${FAKE_SLOW_TERM_ID:-}" ] && contains "$FAKE_SLOW_TERM_ID" "$@"; then trap 'sleep 2; exit 143' TERM; sleep "${FAKE_HANG_SECONDS:-30}" & wait; fi
if [ -n "${FAKE_HANG_ID:-}" ] && contains "$FAKE_HANG_ID" "$@"; then sleep "${FAKE_HANG_SECONDS:-30}"; fi
if [ -n "${FAKE_BASELINE_ID:-}" ] && contains "$FAKE_BASELINE_ID" "$@"; then echo "BASELINE NOT GREEN: the fake"; exit 2; fi
for a in "$@"; do
  if [ -n "${FAKE_DROP_ID:-}" ] && [ "$a" = "$FAKE_DROP_ID" ]; then continue; fi
  printf '%-6s OK (fake)  [a mutant %s]\n' "$a" "$a"
  if [ -n "${FAKE_DUP_ID:-}" ] && [ "$a" = "$FAKE_DUP_ID" ]; then printf '%-6s OK (fake)  [a mutant %s]\n' "$a" "$a"; fi
done
[ -z "${FAKE_FOREIGN_ID:-}" ] || { contains "${FAKE_FOREIGN_HOST:-}" "$@" && printf '%-6s OK (fake)  [a mutant of another shard]\n' "$FAKE_FOREIGN_ID"; }
[ -z "${FAKE_EXTRA_ID:-}" ] || { contains "${FAKE_EXTRA_HOST:-}" "$@" && printf '%-6s OK (fake)  [an extra result that is not in the spec]\n' "$FAKE_EXTRA_ID"; }
if [ -n "${FAKE_COMMENTARY:-}" ]; then
  echo "FAIL IT-77 control: the stand-in rebinds the live socket when called with the live runtime directory"
  echo "       inode stayed 1145747667: IT-76 could not fail"
  echo "ok   A02 (a suite line that begins with ok)  [and ends like a record]"
  echo "A01 is a known id at the start of a sentence: commentary, not a result"
  echo "Z77 is an unknown id at the start of a sentence: commentary too"
  echo "install-test: 193 passed, 1 failed"
fi
if [ -n "${FAKE_STATUS_DIR_ID:-}" ] && contains "$FAKE_STATUS_DIR_ID" "$@"; then mkdir "$FAKE_STATUS_DIR_PATH"; fi
if [ -n "${FAKE_KILL_ID:-}" ] && contains "$FAKE_KILL_ID" "$@"; then kill -s TERM $$; fi
if [ -n "${FAKE_FAIL_ID:-}" ] && contains "$FAKE_FAIL_ID" "$@"; then echo "fake: $# mutants, 1 not caught or broken"; exit "${FAKE_FAIL_CODE:-1}"; fi
echo "fake: $# mutants, 0 not caught or broken"
exit 0
FAKE
chmod +x "$T/driver"

# spec <n>: a spec file in the form of the two real ones (a JSON list of objects with an "id"): ids A01 .. A<n>
spec() {
  n=$1 i=1
  printf '[' > "$T/spec.json"
  while [ "$i" -le "$n" ]; do
    [ "$i" -eq 1 ] || printf ',' >> "$T/spec.json"
    printf '{"id": "A%02d", "what": "mutant %d", "old": "x", "new": "y"}' "$i" "$i" >> "$T/spec.json"
    i=$((i + 1))
  done
  printf ']\n' >> "$T/spec.json"
}
unset_fakes() { unset CI_FANOUT_SHARDS FANOUT_TIMEOUT FANOUT_KILL_GRACE CI_LOG_DIR 2>/dev/null || true; unset FAKE_HANG_ID FAKE_HANG_SECONDS FAKE_BASELINE_ID FAKE_DROP_ID FAKE_DUP_ID FAKE_FOREIGN_ID FAKE_FOREIGN_HOST FAKE_KILL_ID FAKE_FAIL_ID FAKE_FAIL_CODE FAKE_SLEEP FAKE_PASS_THROUGH FAKE_PIDS FAKE_IGNORE_TERM_ID FAKE_SLOW_TERM_ID FAKE_EXTRA_ID FAKE_EXTRA_HOST FAKE_COMMENTARY FAKE_STATUS_DIR_ID FAKE_STATUS_DIR_PATH CI_FANOUT_TEST FANOUT_TEST_LAUNCH_PAUSE FANOUT_TEST_SHARD_PAUSE ADMIT_LOG ADMIT_SLOTS_DIR ADMIT_MAX 2>/dev/null || true; }
fresh() { rm -rf "$T/logs" "$T/ledger" "$T/ledger.args" "$T"/ledger.pid.* "$T/ci" "$T/cwd"; : > "$T/ledger"; : > "$T/ledger.args"; FAKE_LEDGER="$T/ledger"; export FAKE_LEDGER; unset_fakes; }
# run the wrapper (bounded by 120 s so that a hang in a MUTATED wrapper is a failed cell, not a stuck run): rc, $T/out, $T/err
fo() {
  start=$(date +%s)
  timeout 120 sh "$fanout" "$@" > "$T/out" 2> "$T/err" && rc=0 || rc=$?
  secs=$(( $(date +%s) - start ))
}
std="--spec $T/spec.json --log-dir $T/logs"
last() { tail -n 1 "$T/out"; }
ran_count() { grep -cx -- "$1" "$T/ledger" || true; }

# FO-01 the plan: round-robin (id i of the spec, counted from 0, goes to shard i mod N), every id once, the union is the spec; --plan runs nothing
fresh; spec 10
fo --shards 3 $std --plan -- "$T/driver"
if [ "$rc" = 0 ] && grep -qx 'ci-fanout: shard 1/3: 4 ids: A01 A04 A07 A10' "$T/out" && grep -qx 'ci-fanout: shard 2/3: 3 ids: A02 A05 A08' "$T/out" \
   && grep -qx 'ci-fanout: shard 3/3: 3 ids: A03 A06 A09' "$T/out" && grep -qx 'ci-fanout: plan: the union of the shards is the spec, every id once (10 ids)' "$T/out" \
   && [ ! -s "$T/ledger" ]; then ok "FO-01 the plan is round-robin in spec order, every id once, the union is the spec, and --plan runs nothing"
else bad "FO-01 the plan" "rc=$rc; $(cat "$T/out" | head -8 | tr '\n' '|')"; fi

# FO-02 all shards ok: exit 0, the last line says OK, EVERY ID IS RUN EXACTLY ONCE (the ledger), each shard has its log and its status
fresh; spec 10
fo --shards 3 $std -- "$T/driver"
dup=0; for i in 01 02 03 04 05 06 07 08 09 10; do [ "$(ran_count "A$i")" = 1 ] || dup=1; done
if [ "$rc" = 0 ] && [ "$dup" = 0 ] && [ "$(wc -l < "$T/ledger" | tr -d ' ')" = 10 ] && [ "$(last)" = 'ci-fanout: OK (3 shards, 10 ids)' ] \
   && [ -f "$T/logs/spec-shard-1.log" ] && [ -f "$T/logs/spec-shard-3.status" ] && grep -q '^exit=0 ' "$T/logs/spec-shard-2.status"; then ok "FO-02 every id is run exactly once, the gate passes, each shard keeps its log and its status"
else bad "FO-02 all ok" "rc=$rc dup=$dup last=$(last)"; fi

# FO-03 a shard FAILS (exit 1) after reporting all its ids: the gate FAILS (a swallowed status would pass it), names the shard and its exit, and the OTHER shards still ran to the end
fresh; spec 9
FAKE_FAIL_ID=A05; export FAKE_FAIL_ID
fo --shards 3 $std -- "$T/driver"
if [ "$rc" = 1 ] && grep -q '^ci-fanout: shard 2 exit=1 ' "$T/out" && grep -q 'FAIL' "$T/out" && [ "$(last | cut -c1-15)" = 'ci-fanout: FAIL' ] && [ "$(wc -l < "$T/ledger" | tr -d ' ')" = 9 ]; then ok "FO-03 a shard that exits 1 fails the gate, is named with its exit, and the other shards still run to the end"
else bad "FO-03 a failing shard" "rc=$rc last=$(last)"; fi

# FO-04 the exit code itself is kept: a shard that exits 3 and one that exits 2 (baseline not green) are both shown with their own code
fresh; spec 6
FAKE_FAIL_ID=A04; FAKE_FAIL_CODE=3; FAKE_BASELINE_ID=A03; export FAKE_FAIL_ID FAKE_FAIL_CODE FAKE_BASELINE_ID
fo --shards 3 $std -- "$T/driver"
if [ "$rc" = 1 ] && grep -q '^ci-fanout: shard 1 exit=3 ' "$T/out" && grep -q '^ci-fanout: shard 3 exit=2 ' "$T/out"; then ok "FO-04 the exit code of every failing shard is kept as it is (3, and 2 for a baseline that is not green)"
else bad "FO-04 exit codes kept" "rc=$rc"; fi

# FO-05 a shard HANGS: the per-shard timeout ends it (TERM, then KILL by timeout(1) after the grace), the gate fails, the table says it timed out, and the run is bounded
fresh; spec 6
FAKE_HANG_ID=A02; FAKE_HANG_SECONDS=8; export FAKE_HANG_ID FAKE_HANG_SECONDS
FANOUT_KILL_GRACE=1 fo --shards 3 $std --timeout 3 -- "$T/driver"
if [ "$rc" = 1 ] && grep -q '^ci-fanout: shard 2 exit=124 ' "$T/out" && grep -q 'timed out' "$T/out" && [ "$secs" -lt 30 ]; then ok "FO-05 a hanging shard is ended by the timeout, the gate fails, the table says timed out (and the run is bounded: ${secs}s)"
else bad "FO-05 a hanging shard" "rc=$rc secs=$secs"; fi

# FO-06 a shard is KILLED by a signal (TERM, 143): the gate fails and the table says it was killed by a signal
fresh; spec 6
FAKE_KILL_ID=A03; export FAKE_KILL_ID
fo --shards 3 $std -- "$T/driver"
if [ "$rc" = 1 ] && grep -q '^ci-fanout: shard 3 exit=143 ' "$T/out" && grep -q 'killed by signal' "$T/out"; then ok "FO-06 a shard killed by a signal fails the gate and is shown as killed by a signal"
else bad "FO-06 a killed shard" "rc=$rc"; fi

# FO-07 a shard exits 0 but DROPS an id (it never reports A04): the gate fails: the id was not run (exactly once)
fresh; spec 6
FAKE_DROP_ID=A04; export FAKE_DROP_ID
fo --shards 3 $std -- "$T/driver"
if [ "$rc" = 1 ] && grep -q 'ci-fanout: id A04 was not reported by any shard' "$T/out"; then ok "FO-07 a shard that exits 0 but drops an id fails the gate: every id must be reported once"
else bad "FO-07 a dropped id" "rc=$rc"; fi

# FO-08 a shard reports an id TWICE: the gate fails
fresh; spec 6
FAKE_DUP_ID=A02; export FAKE_DUP_ID
fo --shards 3 $std -- "$T/driver"
if [ "$rc" = 1 ] && grep -q 'ci-fanout: id A02 was reported 2 times' "$T/out"; then ok "FO-08 an id reported twice fails the gate"
else bad "FO-08 a duplicated report" "rc=$rc"; fi

# FO-09 a shard reports an id that belongs to ANOTHER shard (A05 in shard 1's log): the gate fails
fresh; spec 6
FAKE_FOREIGN_ID=A05; FAKE_FOREIGN_HOST=A01; export FAKE_FOREIGN_ID FAKE_FOREIGN_HOST
fo --shards 3 $std -- "$T/driver"
if [ "$rc" = 1 ] && grep -q 'ci-fanout: id A05 was reported by shard 1 but belongs to shard 2' "$T/out"; then ok "FO-09 an id reported by the wrong shard fails the gate"
else bad "FO-09 a foreign id" "rc=$rc"; fi

# FO-10 a duplicate id IN THE SPEC is refused (exit 2) and nothing runs
fresh; printf '[{"id": "A01"}, {"id": "A02"}, {"id": "A01"}]\n' > "$T/spec.json"
fo --shards 2 $std -- "$T/driver"
if [ "$rc" = 2 ] && grep -q 'ci-fanout: the spec has the id A01 twice' "$T/err" && [ ! -s "$T/ledger" ]; then ok "FO-10 a spec with a duplicate id is refused and nothing runs"
else bad "FO-10 a duplicate id in the spec" "rc=$rc"; fi

# FO-11 a spec that is not a list of objects with an id, an empty one, and one that is not JSON: refused (exit 2), nothing runs
fresh; r=0
for body in '{"mutants": []}' '[]' '[{"what": "no id"}]' 'not json' '[{"id": "A 1"}]' '[{"id": ""}]' '[{"id": "ok"}]' '[{"id": "7A"}]'; do
  printf '%s\n' "$body" > "$T/spec.json"
  fo --shards 2 $std -- "$T/driver"
  [ "$rc" = 2 ] && [ ! -s "$T/ledger" ] || r=1
done
if [ "$r" = 0 ]; then ok "FO-11 a spec that is not a non-empty list of objects with an id that can appear in a result record is refused and nothing runs (eight shapes)"
else bad "FO-11 bad specs" "a shape was accepted or ran something"; fi

# FO-12 more shards than ids: the shard count is the number of ids, NO EMPTY SHARD IS RUN (a driver given no ids would run every mutant); one shard works too
fresh; spec 3
fo --shards 8 $std -- "$T/driver"
dup=0; for i in 01 02 03; do [ "$(ran_count "A$i")" = 1 ] || dup=1; done
if [ "$rc" = 0 ] && [ "$dup" = 0 ] && [ "$(wc -l < "$T/ledger" | tr -d ' ')" = 3 ] && [ "$(last)" = 'ci-fanout: OK (3 shards, 3 ids)' ] && [ "$(grep -c '^args: ' "$T/ledger.args")" = 3 ]; then
  fresh; spec 3; fo --shards 1 $std -- "$T/driver"
  if [ "$rc" = 0 ] && [ "$(wc -l < "$T/ledger" | tr -d ' ')" = 3 ] && [ "$(last)" = 'ci-fanout: OK (1 shards, 3 ids)' ]; then ok "FO-12 more shards than ids runs one shard per id and never an empty shard; one shard works"
  else bad "FO-12 one shard" "rc=$rc last=$(last)"; fi
else bad "FO-12 more shards than ids" "rc=$rc dup=$dup last=$(last)"; fi

# FO-13 the shard count is checked: 0, a negative, a word, and a missing --spec or driver are usage errors (exit 2), nothing runs
fresh; spec 4; r=0
# a count of 0 is refused BY NAME (it must not reach the split: awk's own division by zero would also end the run with exit 2, by accident)
fo --shards 0 $std -- "$T/driver"
{ [ "$rc" = 2 ] && grep -q 'ci-fanout: the shard count must be at least 1' "$T/err"; } || { r=1; echo "       --shards 0 was not refused by name: $(head -c 120 "$T/err")"; }
for args in "--shards 0 $std -- $T/driver" "--shards -1 $std -- $T/driver" "--shards many $std -- $T/driver" "--shards 2 --log-dir $T/logs -- $T/driver" "--shards 2 $std" "--shards 2 $std --timeout soon -- $T/driver"; do
  # shellcheck disable=SC2086
  fo $args
  [ "$rc" = 2 ] && [ ! -s "$T/ledger" ] || { r=1; echo "       accepted: $args"; }
done
if [ "$r" = 0 ]; then ok "FO-13 a bad shard count, a bad timeout, a missing spec or a missing driver command is a usage error and runs nothing"
else bad "FO-13 usage errors"; fi

# FO-14 the driver command is run as given, the ids come AFTER it, and the caller's environment reaches every shard
fresh; spec 4
FAKE_PASS_THROUGH=hello; export FAKE_PASS_THROUGH
fo --shards 2 $std -- "$T/driver"
if [ "$rc" = 0 ] && grep -qx 'args: A01 A03' "$T/ledger.args" && grep -qx 'args: A02 A04' "$T/ledger.args" && [ "$(grep -cx 'pass-through: hello' "$T/ledger.args")" = 2 ]; then ok "FO-14 the shards get the driver command plus their own ids, and the caller's environment"
else bad "FO-14 arguments and environment" "rc=$rc $(tr '\n' '|' < "$T/ledger.args")"; fi

# FO-15 the shards run CONCURRENTLY: four shards that each take 3 s finish in far less than 12 s
fresh; spec 8
FAKE_SLEEP=3; export FAKE_SLEEP
fo --shards 4 $std -- "$T/driver"
if [ "$rc" = 0 ] && [ "$secs" -lt 9 ] && [ "$(wc -l < "$T/ledger" | tr -d ' ')" = 8 ]; then ok "FO-15 four 3-second shards ran side by side and ran all 8 ids (${secs}s, not 12)"
else bad "FO-15 concurrency" "rc=$rc secs=$secs"; fi

# FO-16 the gate WAITS FOR ALL: with one failing shard and one slow shard, the slow shard still finishes and is in the ledger and the table
fresh; spec 6
FAKE_FAIL_ID=A01; FAKE_SLEEP=2; export FAKE_FAIL_ID FAKE_SLEEP
fo --shards 3 $std -- "$T/driver"
if [ "$rc" = 1 ] && [ "$(wc -l < "$T/ledger" | tr -d ' ')" = 6 ] && grep -q '^ci-fanout: shard 3 exit=0 ' "$T/out" && grep -q '^ci-fanout: shard 1 exit=1 ' "$T/out"; then ok "FO-16 the gate waits for every shard even when one has failed, and shows all of them"
else bad "FO-16 waits for all" "rc=$rc"; fi

# FO-17 the receipt before the run: the plan lines come BEFORE any shard output, and the last line is the verdict, nothing after it
fresh; spec 6
fo --shards 2 $std -- "$T/driver"
first_plan=$(grep -n '^ci-fanout: plan: the union' "$T/out" | head -1 | cut -d: -f1)
first_shard=$(grep -n '^ci-fanout: shard 1 exit=' "$T/out" | head -1 | cut -d: -f1)
if [ "$rc" = 0 ] && [ -n "$first_plan" ] && [ -n "$first_shard" ] && [ "$first_plan" -lt "$first_shard" ] && [ "$(last | cut -c1-14)" = 'ci-fanout: OK ' ]; then ok "FO-17 the exactly-once plan is printed before the shards and the verdict is the last line"
else bad "FO-17 order of the output" "plan=$first_plan shard=$first_shard"; fi

# FO-19 CI_FANOUT_SHARDS in the environment overrides --shards (the number on the ci.sh line)
fresh; spec 10
CI_FANOUT_SHARDS=2 fo --shards 5 $std --plan -- "$T/driver"
if [ "$rc" = 0 ] && grep -q '^ci-fanout: shard 1/2: 5 ids: ' "$T/out" && grep -q '^ci-fanout: shard 2/2: 5 ids: ' "$T/out" && ! grep -q '/5: ' "$T/out"; then ok "FO-19 CI_FANOUT_SHARDS overrides the shard count of the command line"
else bad "FO-19 the environment override" "rc=$rc"; fi
fresh; spec 4
CI_FANOUT_SHARDS=many fo --shards 2 $std --plan -- "$T/driver"
if [ "$rc" = 2 ]; then ok "FO-19b a CI_FANOUT_SHARDS that is not a number is a usage error"; else bad "FO-19b a bad CI_FANOUT_SHARDS" "rc=$rc"; fi

# FO-20 FANOUT_TIMEOUT is the default timeout when --timeout is not given
fresh; spec 4
FAKE_HANG_ID=A01; FAKE_HANG_SECONDS=8; export FAKE_HANG_ID FAKE_HANG_SECONDS
FANOUT_TIMEOUT=2 FANOUT_KILL_GRACE=1 fo --shards 2 $std -- "$T/driver"
if [ "$rc" = 1 ] && grep -q '^ci-fanout: shard 1 exit=124 ' "$T/out" && [ "$secs" -lt 30 ]; then ok "FO-20 FANOUT_TIMEOUT is the default shard timeout"
else bad "FO-20 the default timeout" "rc=$rc secs=$secs"; fi

# FO-21 the logs go to <log dir>/<name>-shard-<i>.log: by default under ${CI_LOG_DIR:-target/ci}/fanout, relative to where the wrapper is started
fresh; spec 4
CI_LOG_DIR="$T/ci" fo --shards 2 --spec "$T/spec.json" -- "$T/driver"
a=0; [ -f "$T/ci/fanout/spec-shard-1.log" ] && [ -f "$T/ci/fanout/spec-shard-2.status" ] || a=1
fresh; spec 4; mkdir -p "$T/cwd"
( cd "$T/cwd" && timeout 120 sh "$fanout" --shards 2 --spec "$T/spec.json" -- "$T/driver" > "$T/out" 2> "$T/err" ) && rc=0 || rc=$?
if [ "$a" = 0 ] && [ "$rc" = 0 ] && [ -f "$T/cwd/target/ci/fanout/spec-shard-1.log" ]; then ok "FO-21 the default log directory is \${CI_LOG_DIR:-target/ci}/fanout"
else bad "FO-21 the log directory" "a=$a rc=$rc"; fi

# FO-22 a TERM to the wrapper ends its shards (they are not left behind): the wrapper WAITS for a driver that needs 2 s to end after the TERM, exits 143, and no fake driver is alive the moment it has exited
fresh; spec 4
FAKE_SLOW_TERM_ID=A01; FAKE_HANG_SECONDS=8; FAKE_PIDS=1; export FAKE_SLOW_TERM_ID FAKE_HANG_SECONDS FAKE_PIDS
sh "$fanout" --shards 2 $std -- "$T/driver" > "$T/out" 2> "$T/err" &
wp=$!
n=0; while [ "$(ls "$T"/ledger.pid.* 2>/dev/null | wc -l | tr -d ' ')" -lt 2 ] && [ "$n" -lt 100 ]; do sleep 0.1; n=$((n + 1)); done
kill -TERM "$wp" 2>/dev/null || true
wrc=0; wait "$wp" || wrc=$?
alive=0; for f in "$T"/ledger.pid.*; do [ -f "$f" ] || continue; pid=${f##*.}; kill -0 "$pid" 2>/dev/null && alive=1; done
if [ "$wrc" = 143 ] && [ "$alive" = 0 ] && [ "$n" -lt 100 ]; then ok "FO-22 a TERM to the wrapper ends the shards and the wrapper exits 143"
else bad "FO-22 TERM to the wrapper" "wrc=$wrc alive=$alive waited=$n"; fi

# FO-23 a shard that IGNORES the TERM is KILLed by timeout(1) after the grace: exit 137, shown as timed out and killed, and the run is bounded
fresh; spec 4
FAKE_IGNORE_TERM_ID=A01; FAKE_HANG_SECONDS=8; export FAKE_IGNORE_TERM_ID FAKE_HANG_SECONDS
FANOUT_KILL_GRACE=1 fo --shards 2 $std --timeout 2 -- "$T/driver"
if [ "$rc" = 1 ] && grep -q '^ci-fanout: shard 1 exit=137 ' "$T/out" && grep -q 'timed out and killed' "$T/out" && [ "$secs" -lt 7 ]; then ok "FO-23 a shard that ignores TERM is killed after the grace (137), shown as timed out and killed (${secs}s)"
else bad "FO-23 a shard that ignores TERM" "rc=$rc secs=$secs"; fi

# FO-24 ci.sh AS DESIGNED (no outer admission: CV2_ADMITTED UNSET at the top; cox's ruling of 9 Nov): every shard goes through the REAL scripts/cv2-admit-shim and is its OWN admission
# (the real cv2-admit: CV2_ADMITTED set = it passes at once; unset = it takes a slot and exports CV2_ADMITTED=1 to the command). The wrapper never sets or clears CV2_ADMITTED.
cat > "$T/fakeadmit" <<'FAKE'
#!/bin/sh
if [ -n "${CV2_ADMITTED:-}" ]; then echo nested >> "$ADMIT_LOG"; exec "$@"; fi
echo own-slot >> "$ADMIT_LOG"
CV2_ADMITTED=1 exec "$@"
FAKE
chmod +x "$T/fakeadmit"
shim="$here/cv2-admit-shim"
fresh; spec 6; ADMIT_LOG="$T/admitlog"; : > "$ADMIT_LOG"; export ADMIT_LOG
env -u CV2_ADMITTED CV2_SHIM_TEST=1 CV2_ADMIT="$T/fakeadmit" CV2_SHIM_HOSTNAME=laptop timeout 120 sh "$fanout" --shards 3 $std -- sh "$shim" "$T/driver" > "$T/out" 2> "$T/err" && rc=0 || rc=$?
dup=0; for i in 01 02 03 04 05 06; do [ "$(ran_count "A$i")" = 1 ] || dup=1; done
own=$(grep -c '^own-slot$' "$ADMIT_LOG" || true); nest=$(grep -c '^nested$' "$ADMIT_LOG" || true)
: > "$ADMIT_LOG"; rm -rf "$T/logs" "$T/ledger"; : > "$T/ledger"
CV2_ADMITTED=1 CV2_SHIM_TEST=1 CV2_ADMIT="$T/fakeadmit" CV2_SHIM_HOSTNAME=laptop timeout 120 sh "$fanout" --shards 3 $std -- sh "$shim" "$T/driver" > "$T/out" 2> "$T/err" && rc2=0 || rc2=$?
own2=$(grep -c '^own-slot$' "$ADMIT_LOG" || true); nest2=$(grep -c '^nested$' "$ADMIT_LOG" || true)
if [ "$rc" = 0 ] && [ "$dup" = 0 ] && [ "$own" = 3 ] && [ "$nest" = 0 ] && [ "$rc2" = 0 ] && [ "$own2" = 0 ] && [ "$nest2" = 3 ]; then ok "FO-24 with CV2_ADMITTED unset every shard is its own admission (3 of 3, none nested); with it set they are nested (3 of 3)"
else bad "FO-24 admission per shard" "unset: rc=$rc own=$own nested=$nest dup=$dup; set: rc=$rc2 own=$own2 nested=$nest2"; fi
unset ADMIT_LOG

# FO-25 FEWER SLOTS THAN SHARDS (ci.sh as designed: the slots are VM-wide, 6, shared): 4 shards on 2 slots all complete, every id once, and no more than 2 run at once (and 2 did)
cat > "$T/slotadmit" <<'FAKE'
#!/bin/sh
slots="$ADMIT_SLOTS_DIR"; max="${ADMIT_MAX:-2}"
while :; do
  i=1
  while [ "$i" -le "$max" ]; do
    if mkdir "$slots/slot.$i" 2>/dev/null; then
      ls -d "$slots"/slot.* | wc -l | tr -d ' ' >> "$slots/seen"
      rc=0; CV2_ADMITTED=1 "$@" || rc=$?
      rmdir "$slots/slot.$i"
      exit "$rc"
    fi
    i=$((i + 1))
  done
  sleep 0.1
done
FAKE
chmod +x "$T/slotadmit"
fresh; spec 8; mkdir -p "$T/slots"; rm -f "$T/slots/seen"; ADMIT_SLOTS_DIR="$T/slots"; ADMIT_MAX=2; FAKE_SLEEP=1; export ADMIT_SLOTS_DIR ADMIT_MAX FAKE_SLEEP
env -u CV2_ADMITTED CV2_SHIM_TEST=1 CV2_ADMIT="$T/slotadmit" CV2_SHIM_HOSTNAME=laptop timeout 120 sh "$fanout" --shards 4 $std -- sh "$shim" "$T/driver" > "$T/out" 2> "$T/err" && rc=0 || rc=$?
dup=0; for i in 01 02 03 04 05 06 07 08; do [ "$(ran_count "A$i")" = 1 ] || dup=1; done
peak=$(sort -n "$T/slots/seen" | tail -1)
if [ "$rc" = 0 ] && [ "$dup" = 0 ] && [ "${peak:-0}" = 2 ] && [ "$(wc -l < "$T/slots/seen" | tr -d ' ')" = 4 ] && [ "$(last)" = 'ci-fanout: OK (4 shards, 8 ids)' ]; then ok "FO-25 four shards on two slots all complete, every id once, at most 2 at a time (peak $peak)"
else bad "FO-25 fewer slots than shards" "rc=$rc dup=$dup peak=${peak:-none} admissions=$(wc -l < "$T/slots/seen" 2>/dev/null | tr -d ' ')"; fi
unset ADMIT_SLOTS_DIR ADMIT_MAX

# FO-26 CANCELLATION DURING LAUNCH (sam FO-SAM-01): a TERM to the wrapper that arrives after shard 1 is LAUNCHED but before it is REGISTERED (the test hook pauses the launch loop 3 s there; it is
# honoured only with CI_FANOUT_TEST=1): the wrapper ends shard 1's work and WAITS for it, starts NO other shard, exits 143, and the whole thing is bounded (the driver would hang 20 s)
fresh; spec 6
FAKE_HANG_ID=A01; FAKE_HANG_SECONDS=20; FAKE_PIDS=1; export FAKE_HANG_ID FAKE_HANG_SECONDS FAKE_PIDS
CI_FANOUT_TEST=1 FANOUT_TEST_LAUNCH_PAUSE=3 sh "$fanout" --shards 3 $std -- "$T/driver" > "$T/out" 2> "$T/err" &
wp=$!
n=0; while [ "$(ls "$T"/ledger.pid.* 2>/dev/null | wc -l | tr -d ' ')" -lt 1 ] && [ "$n" -lt 100 ]; do sleep 0.1; n=$((n + 1)); done
t0=$(date +%s)
kill -TERM "$wp" 2>/dev/null || true
wrc=0; wait "$wp" || wrc=$?
secs=$(( $(date +%s) - t0 ))
alive=0; for f in "$T"/ledger.pid.*; do [ -f "$f" ] || continue; pid=${f##*.}; kill -0 "$pid" 2>/dev/null && alive=1; done
started=$(ls "$T"/ledger.pid.* 2>/dev/null | wc -l | tr -d ' ')
if [ "$wrc" = 143 ] && [ "$alive" = 0 ] && [ "$started" = 1 ] && [ "$secs" -lt 12 ] && [ "$n" -lt 100 ]; then ok "FO-26 a TERM during the launch loop ends the shard already launched, starts no other shard, and the wrapper exits 143 after it (${secs}s)"
else bad "FO-26 cancellation during launch" "wrc=$wrc alive=$alive started=$started secs=$secs waited=$n"; fi

# FO-27 CANCELLATION AT THE SHARD (sam FO-SAM-01): a TERM that reaches a shard's subshell after it LAUNCHED the driver but before it REGISTERED the pid (the hook pauses the shard 3 s there): the shard
# still ends the driver it launched (its trap is armed BEFORE the launch and it checks right after it), and the wrapper exits 143 with no fake driver alive, bounded (the driver would hang 20 s)
fresh; spec 4
FAKE_HANG_ID=A01; FAKE_HANG_SECONDS=20; FAKE_PIDS=1; export FAKE_HANG_ID FAKE_HANG_SECONDS FAKE_PIDS
CI_FANOUT_TEST=1 FANOUT_TEST_SHARD_PAUSE=3 sh "$fanout" --shards 2 $std -- "$T/driver" > "$T/out" 2> "$T/err" &
wp=$!
n=0; while [ "$(ls "$T"/ledger.pid.* 2>/dev/null | wc -l | tr -d ' ')" -lt 2 ] && [ "$n" -lt 100 ]; do sleep 0.1; n=$((n + 1)); done
t0=$(date +%s)
kill -TERM "$wp" 2>/dev/null || true
wrc=0; wait "$wp" || wrc=$?
secs=$(( $(date +%s) - t0 ))
alive=0; for f in "$T"/ledger.pid.*; do [ -f "$f" ] || continue; pid=${f##*.}; kill -0 "$pid" 2>/dev/null && alive=1; done
if [ "$wrc" = 143 ] && [ "$alive" = 0 ] && [ "$secs" -lt 12 ] && [ "$n" -lt 100 ]; then ok "FO-27 a TERM at a shard between its launch and its registration still ends the driver it launched (${secs}s, exit 143, none alive)"
else bad "FO-27 cancellation at the shard" "wrc=$wrc alive=$alive secs=$secs waited=$n"; fi

# FO-28 AN EXTRA RESULT (sam FO-SAM-02): every expected id is reported once AND shard 1 prints one more result record for an id that is not in the spec: the gate FAILS and names it
fresh; spec 4
FAKE_EXTRA_ID=Z99; FAKE_EXTRA_HOST=A01; export FAKE_EXTRA_ID FAKE_EXTRA_HOST
fo --shards 2 $std -- "$T/driver"
if [ "$rc" = 1 ] && grep -q '^ci-fanout: id Z99 was reported by shard 1 but is not in the spec' "$T/out" && [ "$(last | cut -c1-15)" = 'ci-fanout: FAIL' ] && grep -q '^ci-fanout: shard 1 exit=0 ' "$T/out"; then ok "FO-28 a result record for an id that is not in the spec fails the gate (expected ids reported once, plus one extra)"
else bad "FO-28 an extra result id" "rc=$rc last=$(last)"; fi

# FO-29 COMMENTARY IS NEVER A RESULT: the driver also prints 'FAIL IT-77 ...', a suite line that begins with ok, a sentence that begins with a KNOWN id and one that begins with an unknown id, and a count line: none
# of them counts, every id is still reported exactly once and the gate passes
fresh; spec 4
FAKE_COMMENTARY=1; export FAKE_COMMENTARY
fo --shards 2 $std -- "$T/driver"
if [ "$rc" = 0 ] && [ "$(last)" = 'ci-fanout: OK (2 shards, 4 ids)' ] && ! grep -q 'was reported' "$T/out" && grep -q '^ci-fanout: shard 1 exit=0 seconds=[0-9]* ids=2 reported=2 OK' "$T/out"; then ok "FO-29 commentary lines (FAIL lines, suite lines, a sentence that starts with a known or an unknown id) are never counted as results"
else bad "FO-29 commentary" "rc=$rc last=$(last) $(grep 'was reported' "$T/out" | head -2 | tr '\n' '|')"; fi

# FO-30 A STATUS THAT CANNOT BE WRITTEN (sam: FM-3 is reached by a status-file write failure, not only by a signal): the shard's status path is made a DIRECTORY by the driver, so the write fails whoever runs this:
# the table says 'no status was recorded for this shard' (not the status-mismatch text) and the gate fails
fresh; spec 2
FAKE_STATUS_DIR_ID=A01; FAKE_STATUS_DIR_PATH="$T/logs/spec-shard-1.status"; export FAKE_STATUS_DIR_ID FAKE_STATUS_DIR_PATH
fo --shards 1 $std -- "$T/driver"
rmdir "$T/logs/spec-shard-1.status" 2>/dev/null || true
if [ "$rc" = 1 ] && grep -q '^ci-fanout: shard 1 exit=none .* FAIL: no status was recorded for this shard' "$T/out" && [ "$(last | cut -c1-15)" = 'ci-fanout: FAIL' ]; then ok "FO-30 a shard whose status cannot be written fails the gate with 'no status was recorded', not a status mismatch"
else bad "FO-30 an unwritable status" "rc=$rc $(grep '^ci-fanout: shard 1' "$T/out" | head -1 | cut -c1-160)"; fi

# FO-31 THE TEST HOOKS ARE INERT UNLESS CI_FANOUT_TEST IS EXACTLY 1 (cox's condition): FANOUT_TEST_LAUNCH_PAUSE and FANOUT_TEST_SHARD_PAUSE are sleeps and nothing else. With CI_FANOUT_TEST unset, empty, 0, true,
# yes or 11, a run with both pauses at 4 s takes no longer than one without (4 shards would take 16 s of launch pauses alone); with CI_FANOUT_TEST=1 the pauses DO apply (the positive controls)
r=0
for v in UNSET EMPTY 0 true yes 11; do
  fresh; spec 4
  s=$(date +%s)
  case $v in
    UNSET) env -u CI_FANOUT_TEST FANOUT_TEST_LAUNCH_PAUSE=4 FANOUT_TEST_SHARD_PAUSE=4 timeout 120 sh "$fanout" --shards 4 $std -- "$T/driver" > "$T/out" 2> "$T/err" && rc=0 || rc=$? ;;
    EMPTY) env CI_FANOUT_TEST= FANOUT_TEST_LAUNCH_PAUSE=4 FANOUT_TEST_SHARD_PAUSE=4 timeout 120 sh "$fanout" --shards 4 $std -- "$T/driver" > "$T/out" 2> "$T/err" && rc=0 || rc=$? ;;
    *) env CI_FANOUT_TEST="$v" FANOUT_TEST_LAUNCH_PAUSE=4 FANOUT_TEST_SHARD_PAUSE=4 timeout 120 sh "$fanout" --shards 4 $std -- "$T/driver" > "$T/out" 2> "$T/err" && rc=0 || rc=$? ;;
  esac
  secs=$(( $(date +%s) - s ))
  { [ "$rc" = 0 ] && [ "$secs" -lt 6 ] && [ "$(last)" = 'ci-fanout: OK (4 shards, 4 ids)' ]; } || { r=1; echo "       CI_FANOUT_TEST=$v: the pause variables took effect or the run failed (rc=$rc ${secs}s)"; }
done
fresh; spec 4; s=$(date +%s)
env CI_FANOUT_TEST=1 FANOUT_TEST_LAUNCH_PAUSE=1 timeout 120 sh "$fanout" --shards 4 $std -- "$T/driver" > "$T/out" 2> "$T/err" && rc=0 || rc=$?
secs=$(( $(date +%s) - s )); { [ "$rc" = 0 ] && [ "$secs" -ge 3 ]; } || { r=1; echo "       control: FANOUT_TEST_LAUNCH_PAUSE=1 with CI_FANOUT_TEST=1 did not pause the launch loop (rc=$rc ${secs}s)"; }
fresh; spec 4; s=$(date +%s)
env CI_FANOUT_TEST=1 FANOUT_TEST_SHARD_PAUSE=3 timeout 120 sh "$fanout" --shards 2 $std -- "$T/driver" > "$T/out" 2> "$T/err" && rc=0 || rc=$?
secs=$(( $(date +%s) - s )); { [ "$rc" = 0 ] && [ "$secs" -ge 2 ]; } || { r=1; echo "       control: FANOUT_TEST_SHARD_PAUSE=3 with CI_FANOUT_TEST=1 did not pause the shard (rc=$rc ${secs}s)"; }
if [ "$r" = 0 ]; then ok "FO-31 the two test pauses have no effect unless CI_FANOUT_TEST is exactly 1 (unset, empty, 0, true, yes, 11), and do take effect with 1"
else bad "FO-31 the test hooks"; fi

# FO-32 THE RESULT-RECORD FORMAT IS THE REAL DRIVERS' OWN (cox's condition): the print lines of install_mutants.py and install_fetch_mutants.py that print a result are READ OUT OF THEIR SOURCE (exactly one in the
# first, two in the second, each of the form <id padded> <verdict>  [<what>]), a record is RENDERED from each with real ids of the real specs, and the wrapper must count those records, once each; a drift in a driver's
# format (a changed print line) fails THIS cell instead of the gate silently counting less. The ids of BOTH real specs must also pass the wrapper's own id check (--plan over the whole of each spec).
fresh
pyrc=0
python3 -I -B "$here/ci-fanout-test-records.py" "$here" "$T" > "$T/records.out" 2>&1 || pyrc=$?
cat > "$T/realdriver" <<'FAKE'
#!/bin/sh
for a in "$@"; do grep -E "^$a +" "$REAL_RECORDS"; done
echo "install-mutants: $# mutants, 0 not caught or broken"
FAKE
chmod +x "$T/realdriver"
r=0; ncell=0
if [ "$pyrc" = 0 ]; then
  for rec in "$T"/real-*.records; do
    base=${rec%.records}; ncell=$((ncell + 1))
    REAL_RECORDS="$rec"; export REAL_RECORDS
    timeout 120 sh "$fanout" --shards 3 --spec "$base.spec" --log-dir "$T/logs-real" -- "$T/realdriver" > "$T/out" 2> "$T/err" && rc=0 || rc=$?
    { [ "$rc" = 0 ] && [ "$(last | cut -c1-15)" = 'ci-fanout: OK (' ] && ! grep -q 'was reported\|was not reported' "$T/out"; } || { r=1; echo "       the records rendered from the format of $(basename "$base") were not counted exactly once (rc=$rc $(last))"; }
  done
  unset REAL_RECORDS
else r=1; echo "       $(cat "$T/records.out")"; fi
for spec in install-mutants.json install-fetch-mutants.json; do
  fo --shards 6 --spec "$here/$spec" --log-dir "$T/logs-plan" --plan -- "$T/realdriver"
  { [ "$rc" = 0 ] && grep -q '^ci-fanout: plan: the union of the shards is the spec, every id once' "$T/out"; } || { r=1; echo "       the ids of $spec do not all pass the wrapper's id check (rc=$rc $(head -c 160 "$T/err"))"; }
done
if [ "$r" = 0 ] && [ "$ncell" -ge 3 ]; then ok "FO-32 the records of the real drivers' own formats (read from their source) are counted exactly once, and every id of both real specs passes the id check"
else bad "FO-32 the real drivers' record formats" "ncell=$ncell pyrc=$pyrc"; fi

# FO-33 ONE CANCELLATION, A REGISTERED LATEST SHARD THAT IS SLOW TO STOP (sam FO-SAM-01-R1): the latest shard's driver needs 2 s to stop after a TERM; ONE TERM reaches the wrapper once both shards are
# registered. The wrapper exits 143 with no driver alive, AND it sends each shard's subshell exactly ONE TERM: a spy kill() (a BASH_ENV file of this cell, nothing in the wrapper) records every kill the
# wrapper and its shards make; no pid may be TERMed twice and both shard pids must be TERMed. (The cell runs the wrapper under bash because BASH_ENV is not read in sh mode; the wrapper is POSIX sh.)
fresh; spec 4
cat > "$T/killspy.sh" <<'SPY'
kill() { echo "$*" >> "$KILL_LOG"; builtin kill "$@"; }
SPY
KILL_LOG="$T/kills"; : > "$KILL_LOG"; export KILL_LOG
FAKE_SLOW_TERM_ID=A02; FAKE_HANG_SECONDS=20; FAKE_PIDS=1; export FAKE_SLOW_TERM_ID FAKE_HANG_SECONDS FAKE_PIDS
BASH_ENV="$T/killspy.sh" bash "$fanout" --shards 2 $std -- "$T/driver" > "$T/out" 2> "$T/err" &
wp=$!
n=0; while [ "$(ls "$T"/ledger.pid.* 2>/dev/null | wc -l | tr -d ' ')" -lt 2 ] && [ "$n" -lt 100 ]; do sleep 0.1; n=$((n + 1)); done
sleep 0.5
# the latest shard's subshell: the grandparent of the driver that holds A02 (driver -> timeout -> the shard's subshell, which is a child of the wrapper)
drv=""; for f in "$T"/ledger.pid.*; do [ -f "$f" ] || continue; case "$(cat "$f")" in A02*) drv=${f##*.} ;; esac; done
tpid=$(ps -o ppid= -p "${drv:-0}" 2>/dev/null | tr -d ' ' || true); spid=$(ps -o ppid= -p "${tpid:-0}" 2>/dev/null | tr -d ' ' || true)
t0=$(date +%s)
kill -TERM "$wp" 2>/dev/null || true
wrc=0; wait "$wp" || wrc=$?
secs=$(( $(date +%s) - t0 ))
alive=0; for f in "$T"/ledger.pid.*; do [ -f "$f" ] || continue; pid=${f##*.}; kill -0 "$pid" 2>/dev/null && alive=1; done
dups=$(grep -- '^-TERM ' "$T/kills" | sort | uniq -d | wc -l | tr -d ' ')
distinct=$(grep -- '^-TERM ' "$T/kills" | sort -u | wc -l | tr -d ' ')
nsp=$(grep -c -- "^-TERM ${spid:-none}\$" "$T/kills" || true)
unset KILL_LOG
if [ "$wrc" = 143 ] && [ "$alive" = 0 ] && [ "$dups" = 0 ] && [ "$nsp" = 1 ] && [ "$distinct" -ge 2 ] && [ "$secs" -lt 12 ] && [ "$n" -lt 100 ]; then ok "FO-33 one cancellation with a registered latest slow shard: exit 143, no driver alive, no pid TERMed twice ($distinct TERM targets, ${secs}s)"
else bad "FO-33 one cancellation, registered latest slow shard" "the latest shard's subshell (pid ${spid:-unknown}) was sent TERM $nsp time(s), once is expected; pids TERMed more than once: [$(grep -- '^-TERM ' "$T/kills" | sort | uniq -cd | tr -s ' ' | tr '\n' '|')]; wrc=$wrc alive=$alive distinct=$distinct secs=$secs waited=$n"; fi

# FO-34 TWO SEPARATE TERMs TO THE LATEST SHARD (sam FO-SAM-01-R1: a retry wait that a second signal ends early): the shard subshell of the slow-to-stop driver gets a TERM and, 0.5 s later, another (its driver
# needs 2 s after a TERM). The wrapper must not go on until the driver is dead: no driver is alive when the wrapper has exited (the shard waits in a loop until its driver is gone, whatever interrupted a wait)
fresh; spec 4
FAKE_SLOW_TERM_ID=A02; FAKE_HANG_SECONDS=20; FAKE_PIDS=1; export FAKE_SLOW_TERM_ID FAKE_HANG_SECONDS FAKE_PIDS
sh "$fanout" --shards 2 $std -- "$T/driver" > "$T/out" 2> "$T/err" &
wp=$!
n=0; while [ "$(ls "$T"/ledger.pid.* 2>/dev/null | wc -l | tr -d ' ')" -lt 2 ] && [ "$n" -lt 100 ]; do sleep 0.1; n=$((n + 1)); done
drv=""; for f in "$T"/ledger.pid.*; do [ -f "$f" ] || continue; case "$(cat "$f")" in A02*) drv=${f##*.} ;; esac; done
tpid=$(ps -o ppid= -p "${drv:-0}" 2>/dev/null | tr -d ' ' || true); spid=$(ps -o ppid= -p "${tpid:-0}" 2>/dev/null | tr -d ' ' || true)
sparent=$(ps -o ppid= -p "${spid:-0}" 2>/dev/null | tr -d ' ' || true)
if [ -n "$drv" ] && [ -n "$spid" ] && [ "$sparent" = "$wp" ]; then
  kill -TERM "$spid" 2>/dev/null || true
  sleep 0.5
  kill -TERM "$spid" 2>/dev/null || true
fi
wrc=0; wait "$wp" || wrc=$?
alive=0; for f in "$T"/ledger.pid.*; do [ -f "$f" ] || continue; pid=${f##*.}; kill -0 "$pid" 2>/dev/null && alive=1; done
if [ "$sparent" = "$wp" ] && [ "$alive" = 0 ] && [ "$n" -lt 100 ]; then ok "FO-34 two separate TERMs to the latest shard do not end its wait while the driver lives: no driver alive when the wrapper has exited (exit $wrc)"
else bad "FO-34 two TERMs to the latest shard" "driver=${drv:-none} shard=${spid:-none} its-parent=${sparent:-none} wrapper=$wp alive=$alive wrc=$wrc waited=$n"; fi

# FO-18 the scripts parse (sh -n), the way ci.sh's shell-syntax gate would check them
if sh -n "$fanout" && sh -n "$here/ci-fanout-test.sh" && python3 -I -B -c 'import ast, sys; ast.parse(open(sys.argv[1]).read())' "$here/ci-fanout-test-records.py"; then ok "FO-18 the wrapper and its cells parse (sh -n), and the records helper parses"; else bad "FO-18 sh -n"; fi

echo "ci-fanout-test: $pass passed, $fail failed"
if [ "$fail" -ne 0 ]; then echo "FANOUT CELLS FAILED"; exit 1; fi
exit 0
