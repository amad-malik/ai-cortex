#!/bin/sh
# ci-fanout.sh: the GATE FAN-OUT (kai's row 4fa9e996, cox's calendar finding c52081ff, W1-109 option A). A mutant gate runs its mutants one after the other; this splits a
# gate's ids into N shards, runs the shards as CONCURRENT processes, waits for ALL of them, and FAILS the gate if any shard exits nonzero, times out or is killed, or if the
# ids the shards report are not exactly the spec's, each once.
#
#   scripts/ci-fanout.sh [--shards N] --spec <spec.json> [--timeout SECONDS] [--log-dir DIR] [--name NAME] [--plan] -- <driver command...>
#
#   --spec     the spec file the driver itself loads: a JSON list of objects with a plain "id" (scripts/install-mutants.json, scripts/install-fetch-mutants.json). The ids are read
#              HERE, at run time (nothing is hard-coded), and given to the driver on its command line, AFTER the driver command: <driver command...> <id> <id> ...
#   --shards   how many shards (default 6). CI_FANOUT_SHARDS in the environment overrides it. More shards than ids: one shard per id; NEVER an empty shard (a driver given no ids
#              runs EVERY mutant). The ids are dealt ROUND-ROBIN in spec order: id number i (from 0) goes to shard i mod N.
#   --timeout  seconds per shard (default 14400 = 4 h; FANOUT_TIMEOUT overrides the default). timeout(1) sends TERM, and KILL after FANOUT_KILL_GRACE seconds (default 60).
#   --log-dir  where each shard's full output (<name>-shard-<i>.log) and status (<name>-shard-<i>.status) go; default ${CI_LOG_DIR:-target/ci}/fanout.
#   --plan     print the plan and the exactly-once check (the union of the shards is the spec, every id once) and run NOTHING.
#
# Each shard is the driver command given after '--' plus its ids, started as ONE background process; through scripts/cv2-admit-shim that is an admission per shard: nested
# under the one outer admission of gate 1 (CV2_ADMITTED=1) the call passes at once, by hand each shard takes its own slot. The caller's environment reaches every shard.
# Every shard's exit status is written to its status file and reconciled with wait(1)'s; nothing is piped; a shard that cannot be accounted for is a failure.
#
# RESULT RECORDS (sam's FO-SAM-02): a driver reports one result per id as a line of the form  <id> <verdict>  [<what>]  (the id padded, then the verdict text, then two spaces and the
# bracketed description to the end of the line: exactly what scripts/install_mutants.py and scripts/install_fetch_mutants.py print for a result). The id is letters, digits, dot, dash or
# underscore, starts with a letter and ends with a digit (IM-12, FM045); a spec id that cannot be one is refused (exit 2). Every OTHER line (baseline text, the tail of a failing suite,
# 'FAIL IT-77 ...', a sentence that merely starts with an id) is commentary and is never counted. The accounting is exact in both directions: an expected id with no record, a record
# twice, a record from another shard's id, and a record for an id that is not in the spec are each a problem and fail the gate.
#
# CANCELLATION (sam's FO-SAM-01): the wrapper's TERM/INT/HUP handler is armed BEFORE the first shard is launched, and each shard's own trap is armed BEFORE it launches its driver. A signal ends every
# shard already launched by exact pid (and the one being launched, whose pid is not registered yet, through $!), waits for all of them and exits 143; nothing starts after it. No group signal.
#
# TEST HOOKS (never set in a real run; they are sleeps and nothing else: they touch no verdict, no status, no signal and no counter): FANOUT_TEST_LAUNCH_PAUSE=<seconds> pauses the wrapper after each
# shard is launched and before its pid is registered; FANOUT_TEST_SHARD_PAUSE=<seconds> pauses a shard after it launched its driver and before it registered the driver's pid. Both are honoured
# ONLY when CI_FANOUT_TEST is exactly 1 (cells FO-26, FO-27 use them; FO-31 shows they do nothing otherwise).
#
# PRECONDITIONS (stated, not enforced): the wrapper reads the spec once and the drivers read their spec again, so the checkout must not change during a run; the log directory and the --name
# are this run's own (the wrapper deletes earlier <name>-shard-* files and does not isolate two simultaneous runs with the same name). The shard bound (--timeout) includes the time a
# shard waits for an admission slot, not only the time it runs.
# stdout: the plan (before any shard), one line per shard, the log tail of each failing shard, and as the LAST line 'ci-fanout: OK (N shards, M ids)' or 'ci-fanout: FAIL (...)'.
# Exit: 0 the gate passes; 1 it fails; 2 a usage or spec error (nothing was run).
set -eu

die() { echo "ci-fanout: $*" >&2; exit 2; }

shards_cli=6 spec="" tmo=${FANOUT_TIMEOUT:-14400} grace=${FANOUT_KILL_GRACE:-60} log_dir="" name="" plan_only=0
while [ $# -gt 0 ]; do
  case $1 in
    --shards) [ $# -ge 2 ] || die "--shards needs a number"; shards_cli=$2; shift 2 ;;
    --spec) [ $# -ge 2 ] || die "--spec needs a file"; spec=$2; shift 2 ;;
    --timeout) [ $# -ge 2 ] || die "--timeout needs a number of seconds"; tmo=$2; shift 2 ;;
    --log-dir) [ $# -ge 2 ] || die "--log-dir needs a directory"; log_dir=$2; shift 2 ;;
    --name) [ $# -ge 2 ] || die "--name needs a name"; name=$2; shift 2 ;;
    --plan) plan_only=1; shift ;;
    --) shift; break ;;
    *) die "unknown argument $1" ;;
  esac
done
shards=${CI_FANOUT_SHARDS:-$shards_cli}
posint() { case $2 in ''|*[!0-9]*) die "$1 must be a whole number (got '$2')" ;; esac; [ "$2" -ge 1 ] || die "$1 must be at least 1 (got '$2')"; }
posint "the shard count" "$shards"
posint "the timeout" "$tmo"
posint "the kill grace" "$grace"
[ -n "$spec" ] || die "--spec is required"
[ -f "$spec" ] || die "the spec $spec does not exist"
[ $# -ge 1 ] || die "the driver command is required after --"
[ -n "$name" ] || { name=${spec##*/}; name=${name%.json}; }
case $name in ''|*[!A-Za-z0-9._-]*) die "the name '$name' is not plain" ;; esac
[ -n "$log_dir" ] || log_dir=${CI_LOG_DIR:-target/ci}/fanout

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# ---- the ids, read from the spec (a list of objects with a plain id; a duplicate is a refusal)
python3 -I -B -c '
import json, re, sys
path = sys.argv[1]
try:
    spec = json.load(open(path, encoding="utf-8"))
except Exception as e:
    print("ci-fanout: the spec %s is not JSON: %s" % (path, e), file=sys.stderr)
    sys.exit(2)
if not isinstance(spec, list) or not spec:
    print("ci-fanout: the spec %s must be a non-empty JSON list of objects with an id" % path, file=sys.stderr)
    sys.exit(2)
seen = set()
for n, m in enumerate(spec):
    i = m.get("id") if isinstance(m, dict) else None
    if not isinstance(i, str) or not re.fullmatch(r"[A-Za-z][A-Za-z0-9._-]*[0-9]", i):
        print("ci-fanout: entry %d of the spec has no id that can appear in a result record (letters, digits, dot, dash, underscore; starts with a letter, ends with a digit)" % n, file=sys.stderr)
        sys.exit(2)
    if i in seen:
        print("ci-fanout: the spec has the id %s twice" % i, file=sys.stderr)
        sys.exit(2)
    seen.add(i)
    print(i)
' "$spec" > "$work/ids" || exit 2
total=$(wc -l < "$work/ids" | tr -d ' ')
[ "$shards" -le "$total" ] || shards=$total      # no empty shard, ever

# ---- the plan: round-robin, and its own check: the union of the shards is the spec, every id once
awk -v n="$shards" '{ s = (NR - 1) % n + 1; print $1, s; print $1 >> (w "/shard-" s ".ids") }' w="$work" "$work/ids" > "$work/plan"
i=1
while [ "$i" -le "$shards" ]; do : >> "$work/shard-$i.ids"; i=$((i + 1)); done
echo "ci-fanout: plan: $name: $total ids, $shards shards, timeout ${tmo}s"
i=1
while [ "$i" -le "$shards" ]; do
  k=$(wc -l < "$work/shard-$i.ids" | tr -d ' ')
  echo "ci-fanout: shard $i/$shards: $k ids: $(tr '\n' ' ' < "$work/shard-$i.ids" | sed 's/ $//')"
  i=$((i + 1))
done
sort "$work/ids" > "$work/ids.sorted"
i=1; : > "$work/union"
while [ "$i" -le "$shards" ]; do cat "$work/shard-$i.ids" >> "$work/union"; i=$((i + 1)); done
sort "$work/union" > "$work/union.sorted"
cmp -s "$work/ids.sorted" "$work/union.sorted" || die "internal error: the union of the shards is not the spec"
echo "ci-fanout: plan: the union of the shards is the spec, every id once ($total ids)"
[ "$plan_only" -eq 0 ] || exit 0

# ---- the run: one background process per shard, ALL are waited for
mkdir -p "$log_dir"
rm -f "$log_dir/$name"-shard-*.log "$log_dir/$name"-shard-*.status
set -f
pids=""
# the test pause (see TEST HOOKS in the header): a sleep, honoured only when CI_FANOUT_TEST is exactly 1
test_pause() { [ "${CI_FANOUT_TEST:-}" = 1 ] || return 0; [ -z "${1:-}" ] || sleep "$1"; }
# CANCELLATION IS ARMED HERE, BEFORE THE FIRST SHARD IS LAUNCHED: a TERM, INT or HUP ends every shard already launched, by exact pid, and the one being launched ($! is the pid of the most recent
# background command, set the moment it is forked, before it is registered in $pids), waits for all of them and exits 143; because it exits, no later shard is ever started. Each shard gets
# exactly ONE TERM from one cancellation: $! is signalled only when it is NOT registered yet (once registered it is already in $pids, and a second TERM could end the shard's wait early, FO-SAM-01-R1)
stop_shards() { for p in $pids; do kill -TERM "$p" 2>/dev/null || true; done; }
cancel() {
  stop_shards
  set +u; late=$!; set -u
  case " $pids " in
    *" $late "*) ;;
    *) [ -z "$late" ] || kill -TERM "$late" 2>/dev/null || true ;;
  esac
  wait || true
  exit 143
}
trap cancel TERM INT HUP
i=1
while [ "$i" -le "$shards" ]; do
  ids=$(tr '\n' ' ' < "$work/shard-$i.ids")
  k=$(wc -l < "$work/shard-$i.ids" | tr -d ' ')
  log="$log_dir/$name-shard-$i.log" stat="$log_dir/$name-shard-$i.status"
  (
    s=$(date +%s)
    rc=0 cpid="" cancelled=0
    # the shard's own cancellation is armed BEFORE it launches the driver: a TERM before this line finds no child to leak; after it, the handler ends the child (or, if the child's pid is not
    # recorded yet, remembers the cancellation and the check right after the registration ends it)
    trap 'cancelled=1; [ -z "$cpid" ] || kill -TERM "$cpid" 2>/dev/null || true' TERM INT HUP
    # shellcheck disable=SC2086
    timeout -k "$grace" "$tmo" "$@" $ids > "$log" 2>&1 &
    test_pause "${FANOUT_TEST_SHARD_PAUSE:-}"
    cpid=$!
    [ "$cancelled" = 0 ] || kill -TERM "$cpid" 2>/dev/null || true
    wait "$cpid" || rc=$?
    # a trapped signal (a TERM that came to this subshell) interrupts wait(1) while the child is still alive, and that result says nothing about the child: wait AGAIN, as often as it takes, until
    # the child is really gone, and take ITS status (a single retry could itself be interrupted and report the shard finished with its driver alive, FO-SAM-01-R1)
    while [ "$rc" -gt 128 ] && kill -0 "$cpid" 2>/dev/null; do rc=0; wait "$cpid" || rc=$?; done
    e=$(date +%s)
    echo "exit=$rc seconds=$((e - s)) ids=$k" > "$stat"
    exit "$rc"
  ) &
  test_pause "${FANOUT_TEST_LAUNCH_PAUSE:-}"
  pids="$pids $!"
  i=$((i + 1))
done
i=1
for p in $pids; do
  wrc=0
  wait "$p" || wrc=$?
  echo "$wrc" > "$work/wait-$i"
  i=$((i + 1))
done
trap - TERM INT HUP

# ---- the accounting: every id reported once, by its own shard
python3 -I -B -c '
import re, sys
plan, n, prefix = sys.argv[1], int(sys.argv[2]), sys.argv[3]
# a RESULT RECORD: <id> <verdict>  [<what>] (the header of this script says where the format comes from); every other line is commentary
record = re.compile(r"([A-Za-z][A-Za-z0-9._-]*[0-9]) +\S.*  \[.*\]")
owner, order = {}, []
for line in open(plan, encoding="utf-8"):
    i, s = line.split()
    owner[i] = int(s)
    order.append(i)
seen, reported, extra = {}, {}, {}
for s in range(1, n + 1):
    reported[s] = 0
    try:
        text = open("%s%d.log" % (prefix, s), encoding="utf-8", errors="replace").read()
    except OSError:
        continue
    for line in text.splitlines():
        m = record.fullmatch(line)
        if not m:
            continue
        rid = m.group(1)
        if rid in owner:
            seen.setdefault(rid, []).append(s)
            reported[s] += 1
        else:
            extra.setdefault(rid, []).append(s)
for s in range(1, n + 1):
    print("reported %d %d" % (s, reported[s]))
for i in order:
    got = seen.get(i, [])
    if not got:
        print("problem: id %s was not reported by any shard" % i)
    elif len(got) > 1:
        print("problem: id %s was reported %d times" % (i, len(got)))
    for s in sorted(set(got)):
        if s != owner[i]:
            print("problem: id %s was reported by shard %d but belongs to shard %d" % (i, s, owner[i]))
for rid in sorted(extra):
    for s in sorted(set(extra[rid])):
        print("problem: id %s was reported by shard %d but is not in the spec" % (rid, s))
' "$work/plan" "$shards" "$log_dir/$name-shard-" > "$work/account"

bad=0 nprob=0
i=1
while [ "$i" -le "$shards" ]; do
  stat="$log_dir/$name-shard-$i.status"
  k=$(wc -l < "$work/shard-$i.ids" | tr -d ' ')
  rep=$(sed -n "s/^reported $i //p" "$work/account")
  wrc=$(cat "$work/wait-$i")
  if [ -f "$stat" ]; then
    rc=$(sed -n 's/^exit=\([0-9]*\) .*/\1/p' "$stat")
    secs=$(sed -n 's/^exit=[0-9]* seconds=\([0-9]*\) .*/\1/p' "$stat")
  else
    rc="" secs=""
  fi
  why=""
  if [ -z "$rc" ]; then why="no status was recorded for this shard"
  elif [ "$rc" != "$wrc" ]; then why="the status file says $rc but wait says $wrc"
  elif [ "$rc" -eq 124 ]; then why="timed out after ${tmo}s"
  elif [ "$rc" -eq 137 ]; then why="timed out and killed (killed by signal 9 after the ${grace}s grace)"
  elif [ "$rc" -gt 128 ]; then why="killed by signal $((rc - 128))"
  elif [ "$rc" -ne 0 ]; then why="exit code $rc"
  fi
  if [ -n "$why" ]; then
    bad=$((bad + 1))
    echo "ci-fanout: shard $i exit=${rc:-none} seconds=${secs:-?} ids=$k reported=${rep:-0} FAIL: $why log=$log_dir/$name-shard-$i.log"
  else
    echo "ci-fanout: shard $i exit=$rc seconds=$secs ids=$k reported=${rep:-0} OK log=$log_dir/$name-shard-$i.log"
  fi
  i=$((i + 1))
done
while IFS= read -r line; do
  case $line in problem:*) nprob=$((nprob + 1)); echo "ci-fanout: ${line#problem: }" ;; esac
done < "$work/account"
i=1
while [ "$i" -le "$shards" ]; do
  stat="$log_dir/$name-shard-$i.status"
  rc=$(sed -n 's/^exit=\([0-9]*\) .*/\1/p' "$stat" 2>/dev/null || true)
  if [ "${rc:-1}" != 0 ]; then
    echo "ci-fanout: ---- tail of shard $i ($log_dir/$name-shard-$i.log) ----"
    tail -n 12 "$log_dir/$name-shard-$i.log" 2>/dev/null || true
  fi
  i=$((i + 1))
done
if [ "$bad" -eq 0 ] && [ "$nprob" -eq 0 ]; then
  echo "ci-fanout: OK ($shards shards, $total ids)"
  exit 0
fi
echo "ci-fanout: FAIL ($bad of $shards shards failed, $nprob id problems)"
exit 1
