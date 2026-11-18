#!/usr/bin/env bash
# Harness controls for supervise.sh (Z-2's r3 controls, mike's Z2-R0-02, Z2-D01 and Z2-D02, with
# the names changed for cortex-testkit; kai's L-9). Each control runs the
# supervisor with a small driver and checks its exit code. It then checks INDEPENDENTLY, with the
# real podman and the exact container name the supervisor recorded, whether the container is
# gone. Faults are injected from outside by a fake `podman` first on the supervisor's PATH
# (controls/fake-podman/podman), and phase times are measured from that fake's call log, on the
# same monotonic clock (/proc/uptime), not from the supervisor's own log.
#
#   CV2_ADMIT_LOG=1 /path/to/workspace/spikes/cortex-v2-s0/admission/cv2-admit \
#       timeout --kill-after=60 840 ./controls.sh <evidence_dir> [<supervisor>] [<case> ...]
#
# <supervisor> defaults to ./supervise.sh. Naming an older revision (for example 36a426b's run.sh,
# extracted with git show) runs the same controls against it: that is how K8-K11 are shown to
# discriminate. With no cases named, all of K1-K41 run.
#
#   K1  a passing driver                         exit 0,      absent
#   K2  a failing driver                         exit 1,      absent (the driver's failure is kept)
#   K3  removal does nothing (stop, rm no-ops)   exit 70,     present; cleanup within 120 s + TOL;
#                                                             the control removes it by exact name
#   K4  observation fails (exists, ps fail)      exit 71,     never 0; absent (stop and rm ran)
#   K5  TERM to the supervisor during the driver exit 143,    absent, within 130 s of the TERM
#   K6  INT to the supervisor during the driver  exit 130,    absent
#   K7  the driver's deadline (5 s)              exit 124,    absent
#   K8  readiness probe hangs (Z2-D01)           exit 72,     absent; readiness within 30 s + TOL
#   K9  slow cleanup, mike's path (Z2-D01):      exit 70|71,  never 0; cleanup within 120 s + TOL;
#       stop 30 s and rm 30 s that do not remove,             the control removes it by exact name
#       queries 19 s each
#   K10 TERM during cleanup (Z2-D02)             exit 143,    ABSENT AT THE MOMENT the supervisor exits,
#                                                             and the supervisor logged VERIFIED ABSENT
#   K11 INT during cleanup (Z2-D02)              exit 130,    as K10
#   K12 (r2, F1B-R0-05) INT while a TERM- and INT-resistant driver and grandchild run:
#                                                exit 130,    absent; the driver's process group is
#                                                             empty at the supervisor's exit, and it
#                                                             logged TERM-resistant and VERIFIED EMPTY
#   K13 (r2) a passing driver leaves a TERM-resistant descendant behind:
#                                                exit 0,      absent; the descendant was reaped
#   K14 (r2) CV2_SUP_DRIVER_TIMEOUT 0, 541, -1, abc, 1.5, 9999999999:
#                                                exit 64 each, and podman run was never called
#   K15 (r2, port ruling (b)) one forced port collision, then a free port:
#                                                exit 0, absent; exactly 2 run calls; the collision and
#                                                "after 2 attempt(s)" are logged
#   K16 (r2) every attempt collides:            exit 72, absent, no container ever started; exactly
#                                                3 run calls; "attempt 3 of 3" is logged
#   K17 (r3, F1B-R0-05 / SUP-CUSTODY-02) the supervisor's `ps` fails (fake ps):
#       (a) with a lingering TERM-resistant descendant: exit 73, never "VERIFIED EMPTY", "UNKNOWN"
#           logged, and the descendant is gone (killed under custody; checked with the real ps);
#       (b) with no descendant: exit 73 too (unknown is never empty)
#   K18 (r3) the healthy partner: a passing driver, a working ps: exit 0, "VERIFIED EMPTY", no UNKNOWN
#   K19 (r4, SUP-CUSTODY-01) custody lost: the control SIGKILLs the group helper while a TERM-resistant
#                                                driver runs: exit 73, "custody LOST" and "NOT signalled"
#                                                logged, and the driver's group is still alive (this script
#                                                never signals a group); the control then removes it by pid
#   K20 (r4, mike's item A) the supervisor's ps succeeds with a malformed listing, with a lingering
#                                                descendant: exit 73, UNKNOWN and "malformed ps row" logged,
#                                                never VERIFIED EMPTY; the group empty by the real ps
#   K21 (r4, item B) slow successful scans (2.5 s each) near a 5 s driver deadline, a TERM-resistant
#                                                driver: the driver phase within 5 + 15 + 5 s (+ TOL) on the
#                                                supervisor's own log clock, r5: from the helper's launch
#                                                (before its publication scan) to cleanup's start (after the
#                                                final report); exit 124 or 73; the group empty by the real ps
#   K22 (r4, item D) INT inside the launch window (publication slowed by a 2.5 s first scan; later
#                                                scans run the real ps at once), a TERM-resistant
#                                                driver: exit 130, the INT logged as deferred until
#                                                ownership was published, VERIFIED EMPTY, the group empty;
#                                                an INT that misses the window is INVALID
#   K23 (r5, vera's SUP-CUSTODY-02, mike's A) a VALID listing that omits the driver's group, its
#       unreaped leader included (fake ps omitafter:1: publication sees the real ps), with an
#       already-ended leader and a TERM-resistant descendant (K13's driver):
#       (a) omitted: exit 73, "lacks the driver's leader" logged, never VERIFIED EMPTY, the
#           descendant gone by the real ps; (b) the healthy partner, a complete listing: exit 0,
#           VERIFIED EMPTY
#   K24 (r5, vera's SUP-DEADLINE-01, mike's B) the final report is bounded: a TERM-resistant
#       driver, a 5 s deadline; (a) omitted listings: exit 73; (b) complete listings: exit 124 and
#       VERIFIED EMPTY; each within 5 + 15 + 5 s (+ TOL) from the helper's launch to cleanup's
#       start. A guard ends the group by exact pid 55 s after the launch; a guard that fired is a FAIL
#   K25 (r5, vera's F1B-CHANNEL-01, mike's G) the supervisor alone dies: a TERM-resistant driver
#       (60 s deadline); (a) SIGKILL to the supervisor only: the helper logs "stopping: eof" within
#       2 s, verifies the group empty, and is gone within 2 + 15 + 5 s (+ TOL); the control then
#       removes the container by exact name and the run directory by exact path; (b) the healthy
#       channel partner, TERM to the supervisor: exit 143, "stopping: stop", VERIFIED EMPTY. The
#       FIFO's holders are recorded before the signal
#   K26 (r5, vera's SUP-DEADLINE-01, the supervisor's own wait) the helper never reports (SIGSTOP
#       after publication), a TERM-resistant driver, a 5 s deadline: exit 73 and "did not report"
#       logged within 5 + 15 + 5 + 10 s (+ TOL) from the launch to cleanup's start, the group NOT
#       signalled by the supervisor; then SIGCONT: the helper stops the driver itself and the group
#       is empty. A guard SIGCONTs the helper 80 s after the launch; a guard that fired is a FAIL
#   K27 (r5, vera's SUP-DEADLINE-01, the helper's own reap) controls/helper_harness.py runs the
#       helper in-process with KILL modelled as pending (its killpg records, delivers nothing):
#       the final report within deadline + grace + verify (+ TOL), not verified, the leader left
#       unreaped; and the healthy partner (real killpg): VERIFIED EMPTY within the same bound
#   K28 (r5, mike's A, the observer) the controls' own observer accepts the real listing and
#       refuses a malformed one, a valid listing without its own row, and a failing ps (fault
#       injected through OBS_PS)
#   K29 (r6, vera's SUP-CUSTODY-02, duplicate rows) a VALID listing that names the held leader twice,
#       its true row first and a contradictory live row in another group second (fake ps
#       dupleader:1; dupalias:1 writes the second pid with a leading zero), with K13's driver:
#       exit 73, "names pid G 2 times" logged, never VERIFIED EMPTY, the descendant gone; and the
#       healthy partner, a complete listing: exit 0, VERIFIED EMPTY
#   K30 (r6, vera's F1B-CONTROL-CUSTODY-01, the deterministic interleaving) the controls' signal
#       primitive (controls/sigsafe.py) on real processes: (a) a stale identity whose number is held
#       by a live bystander (the emulated reuse) is REFUSED and the bystander untouched; (b) the
#       healthy identity is delivered; (c) a certified child that has exited and been reaped is
#       "gone" (the pidfd does not follow the number); (d) the r5 method, a bare number, reaches
#       the bystander: the hazard, shown
#   K31 (r6) a static count of every signal call site in cortex-testkit (controls/signal_sites.py);
#       r7 (mike's recogniser finding): (a) THIS revision's recogniser fixtures (a quoted trap
#       `kill` among them) run on the copy of signal_sites.py under test; (b) the sweep of EVERY
#       file of the crate under test, listed, and the pin of the helper probe's command
#       substitutions (one, checked; r6's second, unchecked `$(echo $r)` fails it)
#   K32 (r6, mike's B/G remainder, vera's SUP-WAIT-OBS-01) the helper's state cannot be observed
#       (the inert hook CV2_SUP_TEST_FAULT=helper-observe-fails): (a) a stopped helper: exit 73 within
#       5 + 15 + 5 + 10 s (+ TOL) from the launch to cleanup's start, never waited for; then SIGCONT:
#       the helper stops the driver itself; (b) the healthy ended-helper partner: exit 0
#   K33 (r7, mike's B/G remainder: the probe's normalisation) the same two cases under
#       CV2_SUP_TEST_FAULT=probe-normalize-fails. r7 has no normalisation step (K31 pins it), so the
#       value reaches nothing: the stopped helper is "run" and the result is exit 73 within the bound.
#       RED first: r6 plus that hook on its `$(echo $r)` (it yields nothing once the group is
#       published), which then reads "ended" and waits without a deadline
#   K34 (r7, vera's SUP-CUSTODY-02-SELFSTATE) a valid, unique-pid listing of two rows, the helper's
#       OWN row as a zombie and the leader's true row (fake ps selfzombie:1), with an already-ended
#       leader and a lingering descendant: exit 73, the self-row refusal logged, never VERIFIED EMPTY;
#       and the healthy partner, a complete listing: exit 0, VERIFIED EMPTY
#   K35 (r7, vera's F1B-CONTROL-CUSTODY-01-R6A, the deterministic schedule) the controls' run
#       certifier (controls/certify.sh; CV2_CERTIFIER names another, for a RED-first run) on crafted
#       custody lines whose numbers name the control's own bystanders: (a) a parent without the run
#       id and its group-leading child (the effect of reuse on a stale log): nothing certified; (b)
#       an authentic-looking parent, with the run's end set first: nothing certified; (c) the
#       healthy partner, a parent carrying the run id: its helper and leader certified
#       r8 (mike's Low): the bystander parents are the control's direct children; the verdict also
#       requires every cleanup send delivered and every parent reaped (status 137)
#   K36 (r8, vera's SUP-CUSTODY-02-SELFSTATE-W) vera's two rows with the helper's OWN row as W
#       (fake ps selfstate:W:1; W is not reported by the supported kernels, Linux 5.3 or later):
#       exit 73, the self-state refusal logged, never VERIFIED EMPTY; the possible-state partner, a
#       FULL listing with the helper's own row as T (selfas:T:1): exit 0, VERIFIED EMPTY; and the
#       complete listing: exit 0, VERIFIED EMPTY
#   K37 (r8, mike's positive query witness) a stopped helper under
#       CV2_SUP_TEST_FAULT=probe-capture-empty (the jobs capture comes back EMPTY WITH STATUS 0, as
#       when its pipe could not be made): no witness, so unknown: exit 73, "could not be observed",
#       within 35 s (+ TOL), never waited for; the healthy ended child with no fault: exit 0.
#       RED first: r7 plus that hook, which reads the empty capture as "ended" and waits unbounded
#   K38 (r9, mike's F1B-R8-CAPTURE-WITNESS, production) the container list query's capture forced
#       EMPTY WITH STATUS 0 (CV2_SUP_TEST_FAULT=list-capture-empty, a third value of the one inert
#       hook): no witness, so the query is unknown and cleanup's observation fails: exit 71, never
#       VERIFIED ABSENT; the healthy partner: exit 0, VERIFIED ABSENT
#   K39 (r9, the same family, the controls' observer) on a live bystander group: (a) the member seen;
#       (b) the observer's OUTER capture emptied (CV2_CTRL_FAULT=observer-capture-empty): never a
#       valid empty listing; (c) wcap's capture emptied (CV2_CTRL_FAULT=wcap-capture-empty): unknown;
#       (d) the genuinely empty partner, an ended and reaped group: valid and empty
#   K40 (r9) a static inventory of EVERY Bash command substitution in the crate, each classified
#       (controls/capture_inventory.py); r10 (mike's timing finding, vera's
#       F1B-R9-OWNED-RESCUE-CAPTURE), BY CONSTRUCTION: the shell files are discovered, not listed; the
#       supervisor's captures are pinned; a control's capture may stand only as a witnessed body, the
#       observer's anchored listing, or side-effect-free TEXT in an output string (nested captures
#       and captures inside arithmetic judged too); any capture in a decision position, or with an
#       effect, is a FAIL; and no control may hold a pipeline, a process substitution, an unpinned
#       here-string or a capture continued across lines (the same failure without a capture).
#       (a) the judge's own fixtures (controls/inventory_fixtures.py: 18 bad refused, 9 good passed,
#       the baseline passing) on the copy under test; (b) the inventory of the crate under test
#   K41 (r10) the capture-free consumers (controls/consumers.sh; CV2_CONSUMERS names another, for a
#       RED-first run), each with its fault and healthy partner: (a) an unread clock
#       (CV2_CTRL_FAULT=clock-empty) and every span from it are EMPTY, never a number a bound accepts;
#       (b) log stamps exact, malformed ones empty; (c) unreadable files empty, never 0; (d) a state
#       query that cannot run is unknown, never absent; (e) a signal not run (CV2_CTRL_FAULT=sig-not-run),
#       or to unreadable or empty identities, is never "sent"; (f) an undelivered injection makes the
#       verdict INVALID; (g) an owned rescue that never signalled a live group is a FAIL of its own,
#       and its partner empties the group (witnessed); (h) wait_gone's builtin process test
#   K42 (W1-305) the owned-volume lifecycle and its mutants against an isolated fake Podman store;
#       no real container or volume is created by this cell.
# r6: every signal a control sends goes through controls/sigsafe.py to a certified identity (K31).
# r7: a run is certified all-or-nothing in one sigsafe call (certify-run), the run's end first.
# r9: every capture whose empty result would mean none, absent or valid is witnessed (wcap; K40).
# r10: no control decision reads a capture or a pipeline, and no effect runs inside one (K40, K41).
# The controls' own observer (controls/observer.sh; CV2_OBSERVER names another one, for a RED-first
# run) validates every row and its own anchor row, and treats a failed listing as INVALID, never empty.
# TOL is 2 s: shell overhead between capped commands, process exit, and the clock's 10 ms steps.
# Exit 0 only if every control met its expectation.
set -u
case $0 in */*) cd "${0%/*}/.." ;; *) cd .. ;; esac   # the testkit crate: ./supervise.sh and controls/fake-podman
OUT=$1; mkdir -p "$OUT"
SUP=${2:-./supervise.sh}
shift; [ $# -gt 0 ] && shift
CASES=${*:-K1 K2 K3 K4 K5 K6 K7 K8 K9 K10 K11 K12 K13 K14 K15 K16 K17 K18 K19 K20 K21 K22 K23 K24 K25 K26 K27 K28 K29 K30 K31 K32 K33 K34 K35 K36 K37 K38 K39 K40 K41 K42}
REAL=/usr/bin/podman
FAKE_DIR="$PWD/controls/fake-podman"
TOL_MS=2000
FAILS=0 INVALID=0
# A supervisor that cannot be executed would make every control "fail" without measuring anything
# (r3: found when 36a426b's run.sh was first run from /path/to/docs, a share without exec bits).
[ -f "$SUP" ] && [ -x "$SUP" ] || { echo "INVALID: supervisor $SUP is not an executable file; no control was run"; exit 2; }
ts() { date -u +%FT%T.%3NZ; }   # TEXT only: a timestamp in notes and reports
# r6 (vera's F1B-CONTROL-CUSTODY-01): EVERY signal a control sends goes through controls/sigsafe.py, to
# a CERTIFIED identity (pid and start time), through a pidfd opened before the identity is checked.
# No control signals a bare number (K31 counts the call sites; K30's (d) is the one marked
# exception, and it signals only the control's own bystander).
SIGSAFE=(python3 -I "$PWD/controls/sigsafe.py")
# r10 (mike's timing finding, vera's F1B-R9-OWNED-RESCUE-CAPTURE): no control DECISION reads a captured
# value, and no effect runs inside a capture. The clocks, log timestamps, states, names, groups,
# counts and signal reports come from the capture-free helpers of controls/consumers.sh (CV2_CONSUMERS
# names another file, for a RED-first run); signals and removals run as direct commands. K40 checks
# this by construction.
. "${CV2_CONSUMERS:-$PWD/controls/consumers.sh}"
sig() { "${SIGSAFE[@]}" signal "$@" 2>&1; }   # SIG IDFILE [KINDS]: a DIRECT sigsafe call (r10: no capture), its report on stdout
INJ=""
inject() {   # SIG IDFILE KINDS NOTEFILE TEXT: a control's injected signal, by a DIRECT sigsafe call (r10). It must be
             # delivered to every target, or the case's next verdict is INVALID; the report goes to NOTEFILE
  local __r; sig_into __r "$1" "$2" "$3"
  echo "$(ts) $5: $__r" >> "$4"
  [ "$SIGALL" = 1 ] || INJ="${INJ:+$INJ; }the injected $1 was not delivered ($__r)"
}
state() { local __s; state_of __s "$1"; echo "$__s"; }   # TEXT only: a container's state in a report
self_pgid() { local __g; self_pgid_into __g; echo "$__g"; }   # TEXT only (and for an older observer under test)
verdict() {   # name ok(0|1) text [why-invalid]. A supervisor that never started (no name recorded), or a
             # measured phase that never began (why-invalid), is INVALID, never a FAIL: nothing was measured.
             # r10: an injected signal that was not delivered (INJ, set by inject) makes it INVALID too
  set -- "$1" "$2" "$3" "${4:-}${INJ:+${4:+; }$INJ}"; INJ=""
  if [ -z "${C:-}" ]; then echo "INVALID  $1: the supervisor did not start (exit $RC, no container name recorded); nothing was measured"; FAILS=$((FAILS + 1)); INVALID=$((INVALID + 1))
  elif [ -n "${4:-}" ]; then echo "INVALID  $1: $4 (exit $RC); nothing was measured"; FAILS=$((FAILS + 1)); INVALID=$((INVALID + 1))
  elif [ "$2" = 0 ]; then echo "PASS  $1: $3"; else echo "FAIL  $1: $3"; FAILS=$((FAILS + 1)); fi
}
remove_exact() {   # the K3/K9 follow-up: remove the recorded container and its exact owned volume
  [ -n "$1" ] || return 0   # no name recorded: the case is already INVALID, and there is nothing of ours to remove
  local __s __v="$1-pgdata" __pid __co="" __vo="" __rc __exists
  if ! [[ "$1" =~ ^cv2-a-f1-([0-9]+)-[0-9]+$ ]]; then
    echo "      $2 follow-up: refused unexpected container name $1"; FAILS=$((FAILS + 1)); return
  fi
  __pid=${BASH_REMATCH[1]}
  if ! "$REAL" container inspect --format '{{index .Config.Labels "cv2.owner"}}' "$1" > "$OUT/$2.container-owner" 2>/dev/null; then
    echo "      $2 follow-up: could not read container owner for $1"; FAILS=$((FAILS + 1)); return
  fi
  IFS= read -r __co < "$OUT/$2.container-owner" || true
  if ! [[ "$__co" =~ ^f1-[0-9]{8}T[0-9]{6}Z-${__pid}$ ]]; then
    echo "      $2 follow-up: refused unexpected container owner for $1"; FAILS=$((FAILS + 1)); return
  fi
  "$REAL" rm --force "$1" >/dev/null 2>&1
  state_of __s "$1"   # r10: a direct query; only "absent" counts as removed
  echo "      $2 follow-up: removed $1 by exact name; now $__s"
  if [ "$__s" != absent ]; then FAILS=$((FAILS + 1)); return; fi
  "$REAL" volume exists "$__v" >/dev/null 2>&1; __exists=$?
  case $__exists in
    1) echo "      $2 follow-up: volume $__v already absent"; return ;;
    0) : ;;
    *) echo "      $2 follow-up: volume $__v existence unknown"; FAILS=$((FAILS + 1)); return ;;
  esac
  if ! "$REAL" volume inspect --format '{{index .Labels "cv2.owner"}}' "$__v" > "$OUT/$2.volume-owner" 2>/dev/null; then
    echo "      $2 follow-up: could not read owner of volume $__v"; FAILS=$((FAILS + 1)); return
  fi
  IFS= read -r __vo < "$OUT/$2.volume-owner" || true
  if [ "$__vo" != "$__co" ]; then
    echo "      $2 follow-up: refused owner mismatch for volume $__v"; FAILS=$((FAILS + 1)); return
  fi
  "$REAL" volume rm "$__v" >/dev/null 2>&1; __rc=$?
  "$REAL" volume exists "$__v" >/dev/null 2>&1; __exists=$?
  if [ "$__rc" = 0 ] && [ "$__exists" = 1 ]; then
    echo "      $2 follow-up: removed exact owned volume $__v; verified absent"
  else
    echo "      $2 follow-up: volume $__v removal not verified (rm $__rc, exists $__exists)"; FAILS=$((FAILS + 1))
  fi
}

general_volume_followup() {   # CASE: account for each new recorded run, even when the container is already absent
  local __case=$1 __nf __base __name __pid __v __exists __state __line __owner __actual __hits
  for __nf in "$OUT/$__case"-*.name; do
    [ -e "$__nf" ] || continue
    __base=${__nf%.name}; __name=""; IFS= read -r __name < "$__nf" || true
    if ! [[ "$__name" =~ ^cv2-a-f1-([0-9]+)-[0-9]+$ ]]; then
      echo "      $__case follow-up: invalid recorded container name"; FAILS=$((FAILS + 1)); FOLLOWUP_FAILED=1; continue
    fi
    __pid=${BASH_REMATCH[1]}; __v="$__name-pgdata"
    "$REAL" volume exists "$__v" >/dev/null 2>&1; __exists=$?
    case $__exists in
      1) echo "      $__case follow-up: volume $__v verified absent"; continue ;;
      0) : ;;
      *) echo "      $__case follow-up: volume $__v existence unknown"; FAILS=$((FAILS + 1)); FOLLOWUP_FAILED=1; continue ;;
    esac
    __owner=""; __hits=0
    if [ -r "$__base.stderr" ]; then
      while IFS= read -r __line; do
        case $__line in
          *"start: container $__name (cv2.owner="*"), image "*)
            __owner=${__line#*"start: container $__name (cv2.owner="}; __owner=${__owner%%)*}; __hits=$((__hits + 1)) ;;
        esac
      done < "$__base.stderr"
    fi
    if [ "$__hits" != 1 ] || ! [[ "$__owner" =~ ^f1-[0-9]{8}T[0-9]{6}Z-${__pid}$ ]]; then
      echo "      $__case follow-up: no unique recorded owner for $__v"; FAILS=$((FAILS + 1)); FOLLOWUP_FAILED=1; continue
    fi
    state_of __state "$__name"
    if [ "$__state" != absent ]; then
      echo "      $__case follow-up: container $__name is $__state; volume retained"; FAILS=$((FAILS + 1)); FOLLOWUP_FAILED=1; continue
    fi
    if ! "$REAL" ps -a --filter "volume=$__v" --format '{{.Names}}' > "$__base.volume-refs" 2>/dev/null || [ -s "$__base.volume-refs" ]; then
      echo "      $__case follow-up: references to $__v present or unknown"; FAILS=$((FAILS + 1)); FOLLOWUP_FAILED=1; continue
    fi
    if ! "$REAL" volume inspect --format '{{index .Labels "cv2.owner"}}' "$__v" > "$__base.final-volume-owner" 2>/dev/null; then
      echo "      $__case follow-up: volume $__v owner unknown"; FAILS=$((FAILS + 1)); FOLLOWUP_FAILED=1; continue
    fi
    __actual=""; IFS= read -r __actual < "$__base.final-volume-owner" || true
    if [ "$__actual" != "$__owner" ]; then
      echo "      $__case follow-up: volume $__v owner mismatch"; FAILS=$((FAILS + 1)); FOLLOWUP_FAILED=1; continue
    fi
    "$REAL" volume rm "$__v" >/dev/null 2>&1; __exists=$?
    if [ "$__exists" != 0 ]; then
      echo "      $__case follow-up: volume $__v removal failed"; FAILS=$((FAILS + 1)); FOLLOWUP_FAILED=1; continue
    fi
    "$REAL" volume exists "$__v" >/dev/null 2>&1; __exists=$?
    if [ "$__exists" != 1 ]; then
      echo "      $__case follow-up: volume $__v absence not verified"; FAILS=$((FAILS + 1)); FOLLOWUP_FAILED=1; continue
    fi
    echo "      $__case follow-up: removed exact owned volume $__v; verified absent"
  done
}

# run NAME FAKE_SPEC DRIVER...: the supervisor in the foreground. Sets RC, C (the exact name),
# FLOG (the fake's call log) and END_MS (when the supervisor exited; "" if the clock could not be read).
run() {
  local n=$1 spec=$2; shift 2
  FLOG="$OUT/$n.fakelog"; : > "$FLOG"
  local nf="$OUT/$n.name" cp
  ID="$OUT/$n.id"; : > "$ID"; rm -f "$ID.done"
  certify_run "$OUT/$n.stderr" "$ID" & cp=$!   # r6: the helper and the driver's group, certified while held
  env PATH="$FAKE_DIR:$PATH" CV2_FAKE="$spec" CV2_FAKE_LOG="$FLOG" CV2_SUP_NAME_FILE="$nf" ${DRV_TIMEOUT:+CV2_SUP_DRIVER_TIMEOUT=$DRV_TIMEOUT} ${CV2_FAKE_PS:+CV2_FAKE_PS=$CV2_FAKE_PS} ${CV2_FAKE_PS_MARK:+CV2_FAKE_PS_MARK=$CV2_FAKE_PS_MARK} \
    ${CV2_SUP_TEST_FAULT:+CV2_SUP_TEST_FAULT=$CV2_SUP_TEST_FAULT} "$SUP" "$@" > "$OUT/$n.stdout" 2> "$OUT/$n.stderr"
  RC=$?; now_ms END_MS; read_into C "$nf"
  : > "$ID.done"; wait "$cp" 2>/dev/null
}

simple() {   # NAME WANT_RC WANT_STATE FAKE_SPEC DRIVER...
  local n=$1 want=$2 ws=$3; shift 3
  run "$n" "$@"
  local st; state_of st "$C"
  # r2: a start failure (for example the random port already in use) measures nothing for a case
  # that does not test the start: INVALID, never FAIL or PASS.
  local nostart=""
  [ "$want" != 72 ] && grep -q 'podman run failed' "$OUT/$n.stderr" 2>/dev/null && nostart="the container never started (see its stderr, for example 'Address already in use')"
  [ "$RC" = "$want" ] && [ "$st" = "$ws" ]; verdict "$n" $? "exit $RC (want $want), container $C $st (want $ws)" "$nostart"
}

bounded_cleanup() {   # NAME FAKE_SPEC WANT_RCS: the cleanup must end within 120 s + TOL, never with 0
  local n=$1 spec=$2 wants=$3
  run "$n" "$spec" true
  local s0 took st; first_call_into s0 "$FLOG" stop; span_ms took "$s0" "$END_MS"
  state_of st "$C"
  local ok=1; case " $wants " in *" $RC "*) [ -n "$took" ] && [ "$took" -le $((120000 + TOL_MS)) ] && ok=0 ;; esac
  ir=""; [ -z "$s0" ] && ir="cleanup never called stop"
  [ -n "$s0" ] && [ -z "$took" ] && ir="the supervisor's exit time was not measured (a missing or reversed clock sample)"
  verdict "$n" $ok "exit $RC (want one of: $wants), cleanup took ${took:-?} ms from its first stop to the supervisor's exit (bound 120000 + $TOL_MS), container $C $st" \
    "$ir"
  [ "$st" != absent ] && remove_exact "$C" "$n"
}

signal_in_driver() {   # NAME SIGNAL WANT_RC
  local n=$1 sig=$2 want=$3 nf="$OUT/$1.name" cn cs t0 t1 took r inj="" c st
  FLOG="$OUT/$n.fakelog"; : > "$FLOG"
  set -m   # job control: a background job of a non-interactive shell would otherwise ignore SIGINT
  env PATH="$FAKE_DIR:$PATH" CV2_FAKE_LOG="$FLOG" CV2_SUP_NAME_FILE="$nf" "$SUP" sleep 300 > "$OUT/$n.stdout" 2> "$OUT/$n.stderr" &
  local sup=$!
  set +m
  ID="$OUT/$n.id"; : > "$ID"; certify_sup "$sup" "$ID"
  for (( _i = 1; _i <= 60; _i++ )); do read_into cn "$nf"; state_of cs "$cn"; [ "$cs" = present ] && grep -q "ready after\|ready:" "$OUT/$n.stderr" && break; sleep 1; done
  sleep 3   # let the driver start
  now_ms t0
  sig_into r "$sig" "$ID" sup   # r10: the injection is a direct command, and it must be delivered
  echo "$(ts) $n: sending $sig to supervisor $sup: $r" >> "$OUT/$n.notes"
  [ "$SIGALL" = 1 ] || inj="the $sig was not delivered ($r)"
  wait "$sup"; RC=$?
  now_ms t1; span_ms took "$t0" "$t1"; read_into c "$nf"; C=$c
  echo "$(ts) $n: supervisor exited rc=$RC after ${took:-?} ms" >> "$OUT/$n.notes"
  state_of st "$c"
  [ -n "$took" ] || inj="${inj:+$inj; }the time to the exit was not measured (a missing or reversed clock sample)"
  [ "$RC" = "$want" ] && [ "$st" = absent ] && [ -n "$took" ] && [ "$took" -le 130000 ]
  verdict "$n" $? "exit $RC (want $want), container $c $st (want absent), ${took:-?} ms after the $sig (bound 130000)" "$inj"
}

signal_in_cleanup() {   # NAME SIGNAL WANT_RC: the signal lands while cleanup's (slowed) stop runs
  local n=$1 sig=$2 want=$3 nf="$OUT/$1.name" t0 t1 took r inj="" at_exit c s15
  FLOG="$OUT/$n.fakelog"; : > "$FLOG"
  set -m
  env PATH="$FAKE_DIR:$PATH" CV2_FAKE="stop=10" CV2_FAKE_LOG="$FLOG" CV2_SUP_NAME_FILE="$nf" "$SUP" true > "$OUT/$n.stdout" 2> "$OUT/$n.stderr" &
  local sup=$!
  set +m
  ID="$OUT/$n.id"; : > "$ID"; certify_sup "$sup" "$ID"
  for (( _i = 1; _i <= 600; _i++ )); do grep -q ' stop begin' "$FLOG" 2>/dev/null && break; sleep 0.1; done   # cleanup has begun
  sleep 1   # inside the stop, which the fake holds for 10 s
  now_ms t0
  sig_into r "$sig" "$ID" sup
  echo "$(ts) $n: cleanup's stop began; sending $sig to supervisor $sup: $r" >> "$OUT/$n.notes"
  [ "$SIGALL" = 1 ] || inj="the $sig was not delivered ($r)"
  wait "$sup"; RC=$?
  read_into c "$nf"; C=$c; state_of at_exit "$c"   # checked at once, before anything else can remove it
  now_ms t1; span_ms took "$t0" "$t1"
  echo "$(ts) $n: supervisor exited rc=$RC after ${took:-?} ms; container $c $at_exit at exit" >> "$OUT/$n.notes"
  local verified=no; grep -q 'VERIFIED ABSENT' "$OUT/$n.stderr" && verified=yes
  [ "$RC" = "$want" ] && [ "$at_exit" = absent ] && [ "$verified" = yes ]
  verdict "$n" $? "exit $RC (want $want), container $c $at_exit at the supervisor's exit (want absent), supervisor logged VERIFIED ABSENT: $verified, ${took:-?} ms after the $sig" "$inj"
  if [ "$at_exit" != absent ]; then   # the old supervisor leaves its orphaned stop running: wait for it, then make sure
    sleep 15; state_of s15 "$c"; [ "$s15" != absent ] && remove_exact "$c" "$n"   # r10: a direct query; only "absent" skips the rescue
    echo "      $n: container $c $s15 15 s later" >> "$OUT/$n.notes"
  fi
}

# r3 (SUP-CUSTODY-02), r5 (mike's A): the observer keeps its own errors and validates its own
# listing (controls/observer.sh). A caller sets OBS_OK=0 when live_in_group returns 1; a verdict
# then is INVALID, never "empty".
OBS_OK=1
. "${CV2_OBSERVER:-$PWD/controls/observer.sh}"
# r9 (mike's F1B-R8-CAPTURE-WITNESS): a capture whose EMPTY result would mean "none", "absent" or
# "valid" goes through wcap: the command runs in ONE capture that ends with a witness printed after
# it and checked outside. WRC is the command's own status, or "unknown" when the witness is missing
# (the capture never ran or was cut short), so a capture that never ran is never a valid empty
# result. CV2_CTRL_FAULT=wcap-capture-empty (the controls' own fault hook, K39) empties every such capture.
wcap() {   # VAR CMD...
  local __v=$1 __r; shift
  __r=$("$@"; printf '\nwcap-rc=%s' "$?")
  [ "${CV2_CTRL_FAULT:-}" = wcap-capture-empty ] && __r=""
  case $__r in
    *$'\n'wcap-rc=*) WRC=${__r##*$'\n'wcap-rc=}; __r=${__r%$'\n'wcap-rc=*} ;;
    *) WRC=unknown; __r="" ;;
  esac
  [[ $WRC =~ ^[0-9]+$ ]] || WRC=unknown
  while [[ $__r == *$'\n' ]]; do __r=${__r%$'\n'}; done
  printf -v "$__v" '%s' "$__r"
}
group_empty() { local __m; wcap __m live_in_group "$1"; [ "$WRC" = 0 ] && [ -z "$__m" ]; }   # a witnessed, valid, empty listing only
wait_gone() {   # PID MAX_MS: 0 once the helper PID has exited (gone, or a zombie); 1 at MAX_MS (or while its
                # state cannot be read); r10: 2 at once when the clock cannot be read, which the caller makes
                # INVALID (an unread clock is never "still inside the deadline", and never "gone")
  local __t0 __t __dl
  now_ms __t0; [ -n "$__t0" ] || return 2
  __dl=$(( __t0 + $2 ))
  while :; do   # r10: no pipeline decides "gone" (a pipe that cannot be made would read as exited)
    runs_prog "$1" group_runner; case $? in 1) return 0 ;; 2) return 1 ;; esac
    now_ms __t; [ -n "$__t" ] || return 2
    [ "$__t" -lt "$__dl" ] || return 1
    sleep 0.1
  done
}
gone_into() {   # VAR PID MAX_MS: VAR = yes (gone), no (still running at MAX_MS), or "" (no pid, or the clock unread)
  local __g=""
  if [ -n "$2" ]; then wait_gone "$2" "$3"; case $? in 0) __g=yes ;; 1) __g=no ;; esac; fi
  printf -v "$1" '%s' "$__g"
}
. "${CV2_CERTIFIER:-$PWD/controls/certify.sh}"   # r7: certify_sup and certify_run
bg_certify() {   # NAME: for the supervisor just launched in the background as $sup
  ID="$OUT/$1.id"; : > "$ID"; rm -f "$ID.done" "$ID.ready"
  certify_sup "$sup" "$ID"
  certify_run "$OUT/$1.stderr" "$ID" & CP=$!
}
bg_done() { : > "$ID.done"; wait "$CP" 2>/dev/null; }
await_ready() { for (( _i = 1; _i <= 200; _i++ )); do [ -e "$1.ready" ] && return 0; sleep 0.05; done; return 1; }
guard() {   # SECONDS FILE CMD...: after SECONDS, unless FILE.cancel exists, run CMD DIRECTLY (r10: never in a
            # capture; its output goes to FILE.out) and record that the guard fired
  local secs=$1 f=$2; shift 2
  GUARD_FILE=$f; rm -f "$f.cancel"
  ( for (( _i = 0; _i < secs * 10; _i++ )); do [ -e "$f.cancel" ] && exit 0; sleep 0.1; done
    "$@" >> "$f.out" 2>&1; echo "$(ts) guard fired (its command's report: ${f##*/}.out)" >> "$f" ) &
  GUARD_PID=$!
}
end_guard() { : > "$GUARD_FILE.cancel"; wait "$GUARD_PID" 2>/dev/null; }   # joins a guard that is firing
group_verdict() {   # NAME WANT_RC: exit code, absence, an empty group, and the supervisor's own log lines
  local n=$1 want=$2 err="$OUT/$1.stderr" pg left st
  group_into pg "$err"; OBS_OK=1; wcap left live_in_group "$pg"; [ "$WRC" = 0 ] || OBS_OK=0; state_of st "$C"
  local resisted=no emptied=no
  grep -q 'TERM-resistant' "$err" && resisted=yes
  grep -q "driver group $pg: VERIFIED EMPTY" "$err" && emptied=yes
  ir=""; [ -z "$pg" ] && ir="${ir:+$ir; }""the driver's process group was never recorded"; [ "$OBS_OK" = 0 ] && ir="${ir:+$ir; }""the control's own ps listing failed"
  [ "$RC" = "$want" ] && [ "$st" = absent ] && [ -z "$left" ] && [ "$resisted" = yes ] && [ "$emptied" = yes ]
  verdict "$n" $? "exit $RC (want $want), container $C $st (want absent), driver group ${pg:-?} live members now: '${left}' (want none), logged TERM-resistant: $resisted, VERIFIED EMPTY: $emptied" \
    "$ir"
  [ -n "$left" ] && rescue_group "$n" "$pg" "$OUT/$n.id"
}
observe_fault_case() {   # NAME FAULT WANT [ENDED_FAULT]: r6 K32, r7 K33, r8 K37: a stopped helper under
                         # CV2_SUP_TEST_FAULT=FAULT (exit 73, the message WANT, an ERE), and the healthy ended-helper
                         # partner under ENDED_FAULT (default: the same; "none": no fault at all)
  local name=$1 fault=$2 want=$3 efault=${4:-$2} fv bad="" inv="" fu="" sub n nf err drv dt hp pg t0 t1 ms fired unobs verified st gone obs left
         for sub in stopped ended; do
           n=$name-$sub; nf="$OUT/$n.name"; FLOG="$OUT/$n.fakelog"; : > "$FLOG"; err="$OUT/$n.stderr"
           if [ "$sub" = stopped ]; then drv=(bash -c "$RESIST"); dt=5; fv=$fault; else drv=(true); dt=540; fv=$efault; fi
           [ "$fv" = none ] && fv=""
           env PATH="$FAKE_DIR:$PATH" CV2_FAKE_LOG="$FLOG" CV2_SUP_NAME_FILE="$nf" CV2_SUP_DRIVER_TIMEOUT=$dt ${fv:+CV2_SUP_TEST_FAULT=$fv} \
             "$SUP" "${drv[@]}" > "$OUT/$n.stdout" 2> "$err" &
           sup=$!
           bg_certify "$n"
           if [ "$sub" = stopped ]; then
             for (( _i = 1; _i <= 900; _i++ )); do grep -q 'custody: helper [0-9]* holds it' "$err" 2>/dev/null && break; sleep 0.1; done
             helper_into hp "$err"; group_into pg "$err"
             await_ready "$ID"; sleep 0.5
             [ -n "$hp" ] && inject STOP "$ID" helper "$OUT/$n.notes" "$n: SIGSTOP to the helper $hp"
             [ -n "$hp" ] && guard 80 "$OUT/$n.guard" sig CONT "$ID" helper
           fi
           wait "$sup"; RC=$?; bg_done; read_into C "$nf"
           [ "$sub" = stopped ] && [ -n "$hp" ] && end_guard
           log_ms_into t0 "$err" 'start: helper [0-9]* launched'; log_ms_into t1 "$err" 'cleanup: start'
           span_ms ms "$t0" "$t1"
           fired=no; [ -s "$OUT/$n.guard" ] && fired=yes
           unobs=no; grep -qE "$want" "$err" && unobs=yes
           verified=no; grep -q 'VERIFIED EMPTY' "$err" && verified=yes
           state_of st "$C"
           if [ "$sub" = stopped ]; then
             [ -n "$hp" ] && inject CONT "$ID" helper "$OUT/$n.notes" "$n: SIGCONT to the helper $hp"
             gone_into gone "$hp" 40000
             [ "$gone" != yes ] && [ -n "$pg" ] && { echo "      $n: the helper was not seen to end within 40 s of SIGCONT"; rescue_group "$n" "$pg" "$ID"; }
             obs=1; wcap left live_in_group "$pg"; [ "$WRC" = 0 ] || obs=0
             [ "$obs" = 1 ] || inv="the control's own listing failed"
             { [ -z "$hp" ] || [ -z "$pg" ]; } && inv="the helper or its group was never found"
             [ -n "$ms" ] || inv="${inv:-the phase was not measured (a log line is missing)}"
             [ -n "$hp" ] && [ -z "$gone" ] && inv="the clock could not be read, so the helper's end was not timed"   # r10
             [ "$RC" = 73 ] && [ "$unobs" = yes ] && [ "$fired" = no ] && [ -n "$ms" ] && [ "$ms" -le $(( 35000 + TOL_MS )) ] && [ "$gone" = yes ] && [ -z "$left" ] && [ "$st" = absent ] \
               || bad="$bad stopped(exit $RC want 73, "$want" logged $unobs, guard fired $fired, ${ms:-?} ms from the launch to cleanup's start want <= $(( 35000 + TOL_MS )), helper gone after SIGCONT ${gone:-unknown}, left '$left', container $st)"
             [ -n "$left" ] && rescue_group "$n" "$pg" "$ID"
           else
             [ "$RC" = 0 ] && [ "$verified" = yes ] && [ "$unobs" = no ] && [ "$st" = absent ] \
               || bad="$bad ended(exit $RC want 0, VERIFIED EMPTY $verified, "$want" logged $unobs want no, container $st)"
           fi
         done
  local fdesc="CV2_SUP_TEST_FAULT=$efault"; [ "$efault" = none ] && fdesc="no fault"
         [ -z "$bad" ]; verdict "$name" $? "with CV2_SUP_TEST_FAULT=$fault: a stopped helper gave exit 73 within 35000 + $TOL_MS ms, logged '$(grep -oE "$want" "$OUT/$name-stopped.stderr" 2>/dev/null | head -1)', and was never waited for; the healthy ended-helper partner ($fdesc) gave exit 0; failures:${bad:- none}" "$inv"
  [ -n "$fu" ] && echo "      $name follow-up:$fu"
}
RESIST='trap "" TERM INT; (trap "" TERM INT; exec sleep 1000) & sleep 1000'
LINGER='(trap "" TERM INT; exec sleep 1000) & exit 0'
LINGER60='(trap "" TERM INT; exec sleep 60) & exit 0'   # r6, K29: a leftover that an old helper released
                                                        # before it could be certified ends by itself
echo "# cortex-testkit supervisor controls (Z-2 r3), $(ts), supervisor $SUP sha256 $(sha256sum "$SUP" | cut -c1-16), fake podman sha256 $(sha256sum "$FAKE_DIR/podman" | cut -c1-16), cases: $CASES"
for k in $CASES; do
  stale=0
  for nf in "$OUT/$k"-*.name; do [ -e "$nf" ] && stale=1; done
  if [ "$stale" = 1 ]; then echo "FAIL  $k: refusing pre-existing run name files"; FAILS=$((FAILS + 1)); break; fi
  case $k in
    K1)  simple K1-pass         0  absent ""                       true ;;
    K2)  simple K2-driver-fail  1  absent ""                       false ;;
    K3)  bounded_cleanup K3-rm-does-nothing "stop=0:noop rm=0:noop" "70" ;;
    K4)  simple K4-observe-fails 71 absent "exists=fail ps=fail"   true ;;
    K5)  signal_in_driver K5-term TERM 143 ;;
    K6)  signal_in_driver K6-int  INT  130 ;;
    K7)  DRV_TIMEOUT=5 simple K7-deadline 124 absent ""            sleep 60 ;;
    K8)  run K8-ready-hangs "exec=hang" true
         first_call_into r0 "$FLOG" exec; first_call_into c0 "$FLOG" stop; span_ms took "$r0" "$c0"; state_of st "$C"   # r10: no raw subtraction
         ir=""; [ -z "$r0" ] && ir="${ir:+$ir; }""readiness never began (no probe was logged; see the case's stderr, for example a failed podman run)"
         [ -n "$r0" ] && [ -n "$c0" ] && [ -z "$took" ] && ir="${ir:+$ir; }""the phase was not measured (cleanup's first stop is logged before the first probe: a reversed sample)"
         [ "$RC" = 72 ] && [ "$st" = absent ] && [ -n "$took" ] && [ "$took" -le $((30000 + TOL_MS)) ]
         verdict K8-ready-hangs $? "exit $RC (want 72), readiness took ${took:-?} ms from its first probe to cleanup's first stop (bound 30000 + $TOL_MS; a stop not logged, or before the probe, is not measured), container $C $st (want absent)" \
           "$ir"
         [ "$st" != absent ] && remove_exact "$C" K8 ;;
    K9)  bounded_cleanup K9-slow-cleanup "stop=30:noop rm=30:noop exists=19 ps=19" "70 71" ;;
    K10) signal_in_cleanup K10-term-in-cleanup TERM 143 ;;
    K11) signal_in_cleanup K11-int-in-cleanup  INT  130 ;;
    K12) n=K12-resistant-group-int; nf="$OUT/$n.name"; FLOG="$OUT/$n.fakelog"; : > "$FLOG"
         set -m
         env PATH="$FAKE_DIR:$PATH" CV2_FAKE_LOG="$FLOG" CV2_SUP_NAME_FILE="$nf" "$SUP" bash -c "$RESIST" > "$OUT/$n.stdout" 2> "$OUT/$n.stderr" &
         sup=$!
         set +m
         bg_certify "$n"
         for (( _i = 1; _i <= 60; _i++ )); do grep -q 'leads its own process group' "$OUT/$n.stderr" 2>/dev/null && break; sleep 1; done
         sleep 2
         inject INT "$ID" sup "$OUT/$n.notes" "$n: sending INT to supervisor $sup"
         wait "$sup"; RC=$?; bg_done; read_into C "$nf"
         group_verdict "$n" 130 ;;
    K13) run K13-lingering-descendant "" bash -c "$LINGER"
         group_verdict K13-lingering-descendant 0 ;;
    K14) bad=""
         for v in 0 541 -1 abc 1.5 9999999999; do
           DRV_TIMEOUT=$v run "K14-timeout-$v" "" true
           count_calls runs "$FLOG" run
           [ "$RC" = 64 ] && [ "$runs" = 0 ] || bad="$bad $v(exit $RC, runs $runs)"
         done
         C=refused   # nothing is started by design, so no container name is a valid outcome here
         [ -z "$bad" ]; verdict K14-driver-timeout-refused $? "each of 0 541 -1 abc 1.5 9999999999 gave exit 64 with no podman run; failures:${bad:- none}" ;;
    K15) run K15-port-collision-retried "run=collide:1" true
         count_calls runs "$FLOG" run; state_of st "$C"; err="$OUT/K15-port-collision-retried.stderr"
         coll=no; grep -q 'port collision on attempt 1' "$err" && coll=yes
         two=no; grep -q 'after 2 attempt(s)' "$err" && two=yes
         [ "$RC" = 0 ] && [ "$st" = absent ] && [ "$runs" = 2 ] && [ "$coll" = yes ] && [ "$two" = yes ]
         verdict K15-port-collision-retried $? "exit $RC (want 0), container $C $st (want absent), run calls $runs (want 2), collision logged: $coll, 'after 2 attempt(s)' logged: $two" ;;
    K16) run K16-all-collide "run=collide:9" true
         count_calls runs "$FLOG" run; state_of st "$C"; err="$OUT/K16-all-collide.stderr"
         third=no; grep -q 'attempt 3 of 3' "$err" && third=yes
         failed=no; grep -q 'podman run failed or ran out of its 60 s (attempt 3 of 3' "$err" && failed=yes
         started=no; grep -q 'start: started on' "$err" && started=yes
         [ "$RC" = 72 ] && [ "$st" = absent ] && [ "$runs" = 3 ] && [ "$third" = yes ] && [ "$failed" = yes ] && [ "$started" = no ]
         verdict K16-all-collide $? "exit $RC (want 72), container $C $st (want absent), run calls $runs (want 3), 'attempt 3 of 3' logged: $third, final start failure logged: $failed, a container started: $started (want no)" ;;
    K17) bad=""
         for sub in linger none; do
           n=K17-ps-fails-$sub
           if [ "$sub" = linger ]; then drv=("bash" "-c" "$LINGER"); else drv=("true"); fi
           CV2_FAKE_PS=fail run "$n" "" "${drv[@]}"
           err="$OUT/$n.stderr"; group_into pg "$err"; OBS_OK=1; wcap left live_in_group "$pg"; [ "$WRC" = 0 ] || OBS_OK=0
           verified=no; grep -q 'VERIFIED EMPTY' "$err" && verified=yes
           unknown=no; grep -q 'UNKNOWN' "$err" && unknown=yes
           state_of st "$C"
           [ "$OBS_OK" = 1 ] || bad="$bad $sub(INVALID: the control's own listing failed)"
           [ "$RC" = 73 ] && [ "$verified" = no ] && [ "$unknown" = yes ] && [ "$st" = absent ] && [ -z "$left" ] && [ -n "$pg" ] \
             || bad="$bad $sub(exit $RC, verified $verified, unknown $unknown, container $st, left '$left', group '${pg:-?}')"
           [ -n "$left" ] && rescue_group "$n" "$pg" "$ID"
         done
         case "$bad" in *INVALID*) inv="the control's own observer failed";; *) inv="";; esac
         [ -z "$bad" ]; verdict K17-ps-fails-is-unknown $? "with the supervisor's ps failing: a lingering resistant descendant and no descendant each gave exit 73, UNKNOWN logged, never VERIFIED EMPTY, container absent, the group empty by the real ps; failures:${bad:- none}" "$inv" ;;
    K18) run K18-healthy-empty-group "" true
         err="$OUT/K18-healthy-empty-group.stderr"
         verified=no; grep -q 'VERIFIED EMPTY' "$err" && verified=yes
         unknown=no; grep -q 'UNKNOWN' "$err" && unknown=yes
         state_of st "$C"
         [ "$RC" = 0 ] && [ "$verified" = yes ] && [ "$unknown" = no ] && [ "$st" = absent ]
         verdict K18-healthy-empty-group $? "exit $RC (want 0), VERIFIED EMPTY: $verified (want yes), UNKNOWN: $unknown (want no), container $C $st" ;;
    K19) n=K19-custody-lost; nf="$OUT/$n.name"; FLOG="$OUT/$n.fakelog"; : > "$FLOG"
         set -m
         env PATH="$FAKE_DIR:$PATH" CV2_FAKE_LOG="$FLOG" CV2_SUP_NAME_FILE="$nf" "$SUP" bash -c "$RESIST" > "$OUT/$n.stdout" 2> "$OUT/$n.stderr" &
         sup=$!
         set +m
         bg_certify "$n"
         for (( _i = 1; _i <= 60; _i++ )); do grep -q 'custody: helper [0-9]* holds it' "$OUT/$n.stderr" 2>/dev/null && break; sleep 1; done
         custody_helper_into hp "$OUT/$n.stderr"
         await_ready "$ID"; sleep 0.5
         [ -n "$hp" ] && inject KILL "$ID" helper "$OUT/$n.notes" "$n: SIGKILL to the helper $hp"
         wait "$sup"; RC=$?; bg_done; read_into C "$nf"
         err="$OUT/$n.stderr"; group_into pg "$err"; OBS_OK=1; wcap left live_in_group "$pg"; [ "$WRC" = 0 ] || OBS_OK=0
         lost=no; grep -q 'custody LOST' "$err" && lost=yes
         notsig=no; grep -q 'NOT signalled' "$err" && notsig=yes
         state_of st "$C"
         ir=""; [ -z "$hp" ] && ir="${ir:+$ir; }""the helper's pid was never logged"; [ -z "$pg" ] && ir="${ir:+$ir; }""the driver's process group was never recorded"; [ "$OBS_OK" = 0 ] && ir="${ir:+$ir; }""the control's own ps listing failed"
         [ "$RC" = 73 ] && [ "$lost" = yes ] && [ "$notsig" = yes ] && [ -n "$left" ] && [ "$st" = absent ]
         verdict K19-custody-lost $? "exit $RC (want 73), custody LOST logged: $lost, NOT signalled logged: $notsig, the driver's group still alive (unsignalled): '${left}', container $C $st" \
           "$ir"
         [ -n "$left" ] && rescue_group K19-custody-lost "$pg" "$ID"
         ;;
    K20) n=K20-ps-malformed
         CV2_FAKE_PS=malformed run "$n" "" bash -c "$LINGER"
         err="$OUT/$n.stderr"; group_into pg "$err"; OBS_OK=1; wcap left live_in_group "$pg"; [ "$WRC" = 0 ] || OBS_OK=0
         verified=no; grep -q 'VERIFIED EMPTY' "$err" && verified=yes
         unknown=no; grep -q 'UNKNOWN' "$err" && unknown=yes
         malformed=no; grep -q 'malformed ps row' "$err" && malformed=yes
         state_of st "$C"
         ir=""; [ -z "$pg" ] && ir="${ir:+$ir; }""the driver's process group was never recorded"; [ "$OBS_OK" = 0 ] && ir="${ir:+$ir; }""the control's own ps listing failed"
         [ "$RC" = 73 ] && [ "$verified" = no ] && [ "$unknown" = yes ] && [ "$malformed" = yes ] && [ -z "$left" ] && [ "$st" = absent ]
         verdict K20-ps-malformed $? "exit $RC (want 73), VERIFIED EMPTY: $verified (want no), UNKNOWN: $unknown, malformed logged: $malformed, group ${pg:-?} live now: '$left' (want none), container $C $st" \
           "$ir"
         [ -n "$left" ] && rescue_group "$n" "$pg" "$ID" ;;
    K21) n=K21-slow-scans-near-deadline
         DRV_TIMEOUT=5 CV2_FAKE_PS=slow:2.5 run "$n" "" bash -c "$RESIST"
         err="$OUT/$n.stderr"; group_into pg "$err"; OBS_OK=1; wcap left live_in_group "$pg"; [ "$WRC" = 0 ] || OBS_OK=0
         log_ms_into t0 "$err" 'start: helper [0-9]* launched'   # r5: before the publication scan
         log_ms_into t1 "$err" 'cleanup: start'                  # r5: after the final report
         span_ms ms "$t0" "$t1"                                  # r10: both stamps read, and the end not before the start
         state_of st "$C"
         ok=1; case "$RC" in 124|73) [ -n "$ms" ] && [ "$ms" -le $(( 25000 + TOL_MS )) ] && [ -z "$left" ] && [ "$st" = absent ] && ok=0 ;; esac
         ir=""; [ -z "$ms" ] && ir="${ir:+$ir; }""the phase was not measured (a log line is missing, or the end precedes the start)"; [ "$OBS_OK" = 0 ] && ir="${ir:+$ir; }""the control's own ps listing failed"
         verdict K21-slow-scans-near-deadline $ok "exit $RC (want 124 or 73), driver phase ${ms:-?} ms from the helper's launch to cleanup's start (bound 25000 + $TOL_MS), group ${pg:-?} live now: '$left', container $C $st" \
           "$ir"
         [ -n "$left" ] && rescue_group "$n" "$pg" "$ID" ;;
    K22) n=K22-int-during-launch; nf="$OUT/$n.name"; FLOG="$OUT/$n.fakelog"; : > "$FLOG"; rmdir "$OUT/$n.ps-first" 2>/dev/null
         set -m
         env PATH="$FAKE_DIR:$PATH" CV2_FAKE_LOG="$FLOG" CV2_SUP_NAME_FILE="$nf" CV2_FAKE_PS=slowfirst:2.5 CV2_FAKE_PS_MARK="$OUT/$n.ps-first" "$SUP" bash -c "$RESIST" > "$OUT/$n.stdout" 2> "$OUT/$n.stderr" &
         sup=$!
         set +m
         bg_certify "$n"
         for (( _i = 1; _i <= 6000; _i++ )); do grep -q 'start: helper [0-9]* launched' "$OUT/$n.stderr" 2>/dev/null && break; sleep 0.01; done
         inject INT "$ID" sup "$OUT/$n.notes" "$n: sending INT to supervisor $sup inside the launch window"
         wait "$sup"; RC=$?; bg_done; read_into C "$nf"
         err="$OUT/$n.stderr"; group_into pg "$err"; OBS_OK=1; wcap left live_in_group "$pg"; [ "$WRC" = 0 ] || OBS_OK=0
         deferred=no; grep -q 'received INT while the helper started: deferred until ownership was published' "$err" && deferred=yes
         verified=no; grep -q 'VERIFIED EMPTY' "$err" && verified=yes
         state_of st "$C"
         ir=""; [ "$deferred" = no ] && grep -q 'received INT' "$err" && ir="${ir:+$ir; }""the INT missed the launch window (arrived after publication)"; [ -z "$pg" ] && ir="${ir:+$ir; }""the driver's process group was never recorded"; [ "$OBS_OK" = 0 ] && ir="${ir:+$ir; }""the control's own ps listing failed"
         [ "$RC" = 130 ] && [ "$deferred" = yes ] && [ "$verified" = yes ] && [ -z "$left" ] && [ "$st" = absent ]
         verdict K22-int-during-launch $? "exit $RC (want 130), deferred until published: $deferred, VERIFIED EMPTY: $verified, group ${pg:-?} live now: '$left', container $C $st" \
           "$ir"
         [ -n "$left" ] && rescue_group "$n" "$pg" "$ID" ;;
    K23) bad="" inv="" fu=""
         for sub in omitted complete; do
           n=K23-leader-row-$sub; : > "$OUT/$n.ps-calls"
           fps=""; [ "$sub" = omitted ] && fps="omitafter:1"
           CV2_FAKE_PS=$fps CV2_FAKE_PS_MARK="$OUT/$n.ps-calls" run "$n" "" bash -c "$LINGER"
           err="$OUT/$n.stderr"; group_into pg "$err"; obs=1; wcap left live_in_group "$pg"; [ "$WRC" = 0 ] || obs=0
           verified=no; grep -q 'VERIFIED EMPTY' "$err" && verified=yes
           lacks=no; grep -q "lacks the driver's leader" "$err" && lacks=yes
           state_of st "$C"
           [ "$obs" = 1 ] || inv="the control's own listing failed ($sub)"
           [ -n "$pg" ] || inv="the driver's process group was never recorded ($sub)"
           if [ "$sub" = omitted ]; then
             [ "$RC" = 73 ] && [ "$verified" = no ] && [ "$lacks" = yes ] && [ -z "$left" ] && [ "$st" = absent ] \
               || bad="$bad omitted(exit $RC want 73, VERIFIED EMPTY $verified want no, leader-row refusal logged $lacks want yes, left '$left', container $st)"
           else
             [ "$RC" = 0 ] && [ "$verified" = yes ] && [ "$lacks" = no ] && [ -z "$left" ] && [ "$st" = absent ] \
               || bad="$bad complete(exit $RC want 0, VERIFIED EMPTY $verified want yes, refusal logged $lacks want no, left '$left', container $st)"
           fi
           [ -n "$left" ] && rescue_group "$n" "$pg" "$ID"
         done
         [ -z "$bad" ]; verdict K23-leader-row-omitted $? "a valid listing without the driver's group (omitafter:1) gave exit 73, the leader-row refusal and never VERIFIED EMPTY, with the descendant killed under custody; the complete-listing partner gave exit 0 and VERIFIED EMPTY; failures:${bad:- none}" "$inv"
         [ -n "$fu" ] && echo "      K23 follow-up:$fu" ;;
    K24) bad="" inv="" fu=""
         for sub in omitted complete; do
           n=K24-report-bounded-$sub; nf="$OUT/$n.name"; FLOG="$OUT/$n.fakelog"; : > "$FLOG"; : > "$OUT/$n.ps-calls"; err="$OUT/$n.stderr"
           fps=""; [ "$sub" = omitted ] && fps="omitafter:1"
           env PATH="$FAKE_DIR:$PATH" CV2_FAKE_LOG="$FLOG" CV2_SUP_NAME_FILE="$nf" CV2_SUP_DRIVER_TIMEOUT=5 ${fps:+CV2_FAKE_PS=$fps} CV2_FAKE_PS_MARK="$OUT/$n.ps-calls" \
             "$SUP" bash -c "$RESIST" > "$OUT/$n.stdout" 2> "$err" &
           sup=$!
           bg_certify "$n"
           for (( _i = 1; _i <= 900; _i++ )); do grep -q 'start: helper [0-9]* launched' "$err" 2>/dev/null && break; sleep 0.1; done
           guard 55 "$OUT/$n.guard" sig KILL "$ID" member,leader
           wait "$sup"; RC=$?; read_into C "$nf"
           end_guard; bg_done
           log_ms_into t0 "$err" 'start: helper [0-9]* launched'; log_ms_into t1 "$err" 'cleanup: start'
           span_ms ms "$t0" "$t1"
           fired=no; [ -s "$OUT/$n.guard" ] && fired=yes
           group_into pg "$err"; obs=1; wcap left live_in_group "$pg"; [ "$WRC" = 0 ] || obs=0
           verified=no; grep -q 'VERIFIED EMPTY' "$err" && verified=yes
           state_of st "$C"
           [ "$obs" = 1 ] || inv="the control's own listing failed ($sub)"
           [ -n "$ms" ] || inv="the phase was not measured ($sub: a log line is missing)"
           want=124; [ "$sub" = omitted ] && want=73
           ok=1; [ "$RC" = "$want" ] && [ "$fired" = no ] && [ -n "$ms" ] && [ "$ms" -le $(( 25000 + TOL_MS )) ] && [ -z "$left" ] && [ "$st" = absent ] && ok=0
           [ "$sub" = omitted ] && [ "$verified" = yes ] && ok=1
           [ "$sub" = complete ] && [ "$verified" = no ] && ok=1
           [ "$ok" = 0 ] || bad="$bad $sub(exit $RC want $want, guard fired $fired, phase ${ms:-?} ms, VERIFIED EMPTY $verified, left '$left', container $st)"
           [ -n "$left" ] && rescue_group "$n" "$pg" "$ID"
           [ "$fired" = yes ] && fu="$fu $n: $(tr '\n' ' ' < "$OUT/$n.guard");"
           [ "$st" != absent ] && remove_exact "$C" "$n"
         done
         [ -z "$bad" ]; verdict K24-final-report-bounded $? "a TERM-resistant driver with a 5 s deadline: omitted listings gave exit 73 and complete ones exit 124 with VERIFIED EMPTY, each from the helper's launch to cleanup's start within 25000 + $TOL_MS ms, no guard; failures:${bad:- none}" "$inv"
         [ -n "$fu" ] && echo "      K24 follow-up:$fu" ;;
    K25) bad="" inv="" fu="" hold=""
         for sub in sigkill term; do
           n=K25-supervisor-$sub; nf="$OUT/$n.name"; FLOG="$OUT/$n.fakelog"; : > "$FLOG"; err="$OUT/$n.stderr"
           set -m
           env PATH="$FAKE_DIR:$PATH" CV2_FAKE_LOG="$FLOG" CV2_SUP_NAME_FILE="$nf" CV2_SUP_DRIVER_TIMEOUT=60 "$SUP" bash -c "$RESIST" > "$OUT/$n.stdout" 2> "$err" &
           sup=$!
           set +m
           bg_certify "$n"
           for (( _i = 1; _i <= 900; _i++ )); do grep -q 'custody: helper [0-9]* holds it' "$err" 2>/dev/null && break; sleep 0.1; done
           helper_into hp "$err"; group_into pg "$err"; sp=""; [ -n "$hp" ] && status_path_into sp "$hp"; rd=${sp%/status}
           await_ready "$ID"; sleep 0.5
           h=""; [ -n "$rd" ] && fifo_holders_into h "$rd/cmd"; hold="$hold $sub: [$h]"
           echo "$(ts) $n: the FIFO $rd/cmd is held by: $h" >> "$OUT/$n.notes"
           wall_ms t0; now_ms u0
           if [ "$sub" = sigkill ]; then w=KILL; else w=TERM; fi
           { sig_into r $w "$ID" sup; wait "$sup"; RC=$?; } 2>/dev/null   # r10: one group, so that bash's "Killed" notice for its own child goes to /dev/null
           [ "$SIGALL" = 1 ] || INJ="${INJ:+$INJ; }the $w to the supervisor was not delivered ($r)"
           echo "$(ts) $n: sent $sub to the supervisor $sup only: $r" >> "$OUT/$n.notes"
           bg_done; read_into C "$nf"
           gone_into gone "$hp" 40000   # r10: "" when the clock cannot be read (INVALID below)
           now_ms u1; span_ms took "$u0" "$u1"
           if [ "$gone" != yes ] && [ -n "$pg" ]; then
             echo "      $n: the helper was not seen to end within 40 s of the signal"; rescue_group "$n" "$pg" "$ID"
             wait_gone "$hp" 10000
           fi
           obs=1; wcap left live_in_group "$pg"; [ "$WRC" = 0 ] || obs=0
           log_ms_into e "$err" 'stopping: eof'; span_ms eof_ms "$t0" "$e"
           stop=no; grep -q 'stopping: stop' "$err" && stop=yes
           verified=no; grep -q 'VERIFIED EMPTY' "$err" && verified=yes
           state_of st "$C"
           [ "$obs" = 1 ] || inv="the control's own listing failed ($sub)"
           { [ -z "$hp" ] || [ -z "$pg" ] || [ -z "$rd" ]; } && inv="the helper, its group or its run directory was never found ($sub)"
           { [ -n "$t0" ] && [ -n "$u0" ] && [ -n "$u1" ] && { [ -z "$hp" ] || [ -n "$gone" ]; }; } || inv="the clock could not be read, so nothing was timed ($sub)"   # r10 (mike): INVALID, never a FAIL or a PASS
           [ "$sub" = sigkill ] && [ -n "$t0" ] && [ -n "$e" ] && [ -z "$eof_ms" ] && inv="the eof stamp precedes the signal (a reversed sample) ($sub)"
           if [ "$sub" = sigkill ]; then
             [ -n "$eof_ms" ] && [ "$eof_ms" -le 2000 ] && [ "$gone" = yes ] && [ -n "$took" ] && [ "$took" -le $(( 22000 + TOL_MS )) ] && [ "$verified" = yes ] && [ -z "$left" ] \
               || bad="$bad sigkill(eof after ${eof_ms:-never} ms want <= 2000, helper gone ${gone:-unknown} after ${took:-?} ms want <= $(( 22000 + TOL_MS )), VERIFIED EMPTY $verified, left '$left')"
             [ "$st" != absent ] && remove_exact "$C" "$n"
             case "$rd" in "${TMPDIR:-/tmp}"/cv2-sup.*) [ -d "$rd" ] && rm -rf "$rd" && fu="$fu $n: removed the dead supervisor's run directory $rd by exact path;" ;; esac
           else
             [ "$RC" = 143 ] && [ "$stop" = yes ] && [ "$verified" = yes ] && [ -z "$left" ] && [ "$st" = absent ] \
               || bad="$bad term(exit $RC want 143, stopping: stop $stop, VERIFIED EMPTY $verified, left '$left', container $st)"
           fi
           [ -n "$left" ] && rescue_group "$n" "$pg" "$ID"
         done
         [ -z "$bad" ]; verdict K25-supervisor-only-death $? "SIGKILL to the supervisor alone: the helper saw EOF within 2 s, verified the group empty and ended in time; the TERM partner: exit 143 through the channel; FIFO holders before the signal:$hold; failures:${bad:- none}" "$inv"
         [ -n "$fu" ] && echo "      K25 follow-up:$fu" ;;
    K26) n=K26-helper-never-reports; nf="$OUT/$n.name"; FLOG="$OUT/$n.fakelog"; : > "$FLOG"; err="$OUT/$n.stderr"; inv="" fu=""
         env PATH="$FAKE_DIR:$PATH" CV2_FAKE_LOG="$FLOG" CV2_SUP_NAME_FILE="$nf" CV2_SUP_DRIVER_TIMEOUT=5 "$SUP" bash -c "$RESIST" > "$OUT/$n.stdout" 2> "$err" &
         sup=$!
         bg_certify "$n"
         for (( _i = 1; _i <= 900; _i++ )); do grep -q 'custody: helper [0-9]* holds it' "$err" 2>/dev/null && break; sleep 0.1; done
         helper_into hp "$err"; group_into pg "$err"
         await_ready "$ID"; sleep 0.5
         [ -n "$hp" ] && inject STOP "$ID" helper "$OUT/$n.notes" "$n: SIGSTOP to the helper $hp"
         [ -n "$hp" ] && guard 80 "$OUT/$n.guard" sig CONT "$ID" helper
         wait "$sup"; RC=$?; bg_done; read_into C "$nf"
         [ -n "$hp" ] && end_guard
         log_ms_into t0 "$err" 'start: helper [0-9]* launched'; log_ms_into t1 "$err" 'cleanup: start'
         span_ms ms "$t0" "$t1"
         fired=no; [ -s "$OUT/$n.guard" ] && fired=yes
         late=no; grep -q 'did not report within' "$err" && late=yes
         [ -n "$hp" ] && inject CONT "$ID" helper "$OUT/$n.notes" "$n: SIGCONT to the helper $hp"
         gone_into gone "$hp" 40000
         if [ "$gone" != yes ] && [ -n "$pg" ]; then echo "      $n: the helper was not seen to end within 40 s of SIGCONT"; rescue_group "$n" "$pg" "$ID"; fi
         obs=1; wcap left live_in_group "$pg"; [ "$WRC" = 0 ] || obs=0
         state_of st "$C"
         [ "$obs" = 1 ] || inv="the control's own listing failed"
         { [ -z "$hp" ] || [ -z "$pg" ]; } && inv="the helper or its group was never found"
         [ -n "$ms" ] || inv="${inv:-the phase was not measured (a log line is missing)}"
         [ -n "$hp" ] && [ -z "$gone" ] && inv="the clock could not be read, so the helper's end was not timed"   # r10
         [ "$RC" = 73 ] && [ "$late" = yes ] && [ "$fired" = no ] && [ -n "$ms" ] && [ "$ms" -le $(( 35000 + TOL_MS )) ] && [ "$gone" = yes ] && [ -z "$left" ] && [ "$st" = absent ]
         verdict K26-helper-never-reports $? "exit $RC (want 73), 'did not report' logged: $late, guard fired: $fired, ${ms:-?} ms from the launch to cleanup's start (bound 35000 + $TOL_MS), helper gone after SIGCONT: ${gone:-unknown}, group ${pg:-?} live now: '$left', container $C $st" "$inv"
         [ -n "$left" ] && rescue_group "$n" "$pg" "$ID"
         [ -n "$fu" ] && echo "      K26 follow-up:$fu" ;;
    K27) n=K27-helper-reap-bounded; C=harness
         abs_dir_into root "$SUP"
         python3 -I controls/helper_harness.py "$root/group_runner.py" "$OUT" > "$OUT/$n.stdout" 2> "$OUT/$n.stderr"; hrc=$?
         [ "$hrc" = 0 ]; verdict "$n" $? "$(tr '\n' ' ' < "$OUT/$n.stdout")" ;;
    K28) C=observer; bad=""; self_pgid_into me
         wcap x live_in_group "$me"; r=$WRC
         { [ "$r" = 0 ] && [ -n "$x" ]; } || bad="$bad real(rc $r, members '$(echo $x)')"
         for f in malformed "omitpid:$$" fail; do
           CV2_FAKE_PS=$f OBS_PS="$FAKE_DIR/ps" wcap x live_in_group "$me"; r=$WRC
           [ "$r" != 0 ] || bad="$bad $f(accepted: rc 0, members '$(echo $x)')"
         done
         [ -z "$bad" ]; verdict K28-observer-validates $? "the observer accepted the real listing and refused malformed output, a valid listing without its own row (omitpid:\$\$) and a failing ps; failures:${bad:- none}" ;;
    K29) bad="" inv="" fu=""
         for sub in duplicate alias complete; do
           n=K29-leader-row-$sub; : > "$OUT/$n.ps-calls"
           case $sub in duplicate) fps="dupleader:1" ;; alias) fps="dupalias:1" ;; *) fps="" ;; esac
           CV2_FAKE_PS=$fps CV2_FAKE_PS_MARK="$OUT/$n.ps-calls" run "$n" "" bash -c "$LINGER60"
           err="$OUT/$n.stderr"; group_into pg "$err"; obs=1; wcap left live_in_group "$pg"; [ "$WRC" = 0 ] || obs=0
           verified=no; grep -q 'VERIFIED EMPTY' "$err" && verified=yes
           dup=no; grep -q "names pid [0-9]* 2 times" "$err" && dup=yes
           state_of st "$C"
           [ "$obs" = 1 ] || inv="the control's own listing failed ($sub)"
           [ -n "$pg" ] || inv="the driver's process group was never recorded ($sub)"
           if [ "$sub" = complete ]; then
             [ "$RC" = 0 ] && [ "$verified" = yes ] && [ "$dup" = no ] && [ -z "$left" ] && [ "$st" = absent ] \
               || bad="$bad complete(exit $RC want 0, VERIFIED EMPTY $verified want yes, duplicate refusal logged $dup want no, left '$left', container $st)"
           else
             [ "$RC" = 73 ] && [ "$verified" = no ] && [ "$dup" = yes ] && [ -z "$left" ] && [ "$st" = absent ] \
               || bad="$bad $sub(exit $RC want 73, VERIFIED EMPTY $verified want no, duplicate refusal logged $dup want yes, left '$left', container $st)"
           fi
           [ -n "$left" ] && rescue_group "$n" "$pg" "$ID"   # r10: certified KILL, then empty or an observed-only wait
         done
         [ -z "$bad" ]; verdict K29-leader-row-duplicated $? "a valid listing that names the held leader twice (dupleader:1) or by a leading-zero alias (dupalias:1) gave exit 73, the duplicate refusal and never VERIFIED EMPTY, with the descendant killed under custody; the complete-listing partner gave exit 0 and VERIFIED EMPTY; failures:${bad:- none}" "$inv"
         [ -n "$fu" ] && echo "      K29 follow-up:$fu" ;;
    K30) C=identity; n=K30-stale-identity; f="$OUT/$n"; bad=""; : > "$f.hits"; : > "$f.ok.id"
         # the bystander: the control's own child, which records each SIGUSR1 it receives
         python3 -I -c 'import signal, sys, time
signal.signal(signal.SIGUSR1, lambda *a: open(sys.argv[1], "a").write("usr1\n"))
time.sleep(60)' "$f.hits" &
         b=$!; sleep 0.3
         "${SIGSAFE[@]}" certify-pid "$b" "$$" member "$f.ok.id" > "$f.notes" 2>&1
         # the emulated reuse: an identity whose NUMBER is now held by the live bystander, a different
         # generation (the effect that reuse has on a stored identity)
         awk '{print $1, $2, $3 - 1}' "$f.ok.id" > "$f.stale.id"
         sig_into a USR1 "$f.stale.id"; sleep 0.3; lines_into ha "$f.hits"
         case "$a" in *REFUSED*) [ "$ha" = 0 ] || bad="$bad a(refused, but the bystander was hit)" ;; *) bad="$bad a(not refused: $a)" ;; esac
         sig_into h USR1 "$f.ok.id"; sleep 0.3; lines_into hb "$f.hits"
         case "$h" in *delivered*) [ "$hb" = 1 ] || bad="$bad b(delivered, hits $hb want 1)" ;; *) bad="$bad b(the healthy identity was not delivered: $h)" ;; esac
         # (c) the pidfd does not follow the number: a certified child that has exited and been reaped is gone
         sleep 30 & x=$!; : > "$f.x.id"; "${SIGSAFE[@]}" certify-pid "$x" "$$" member "$f.x.id" >> "$f.notes" 2>&1
         { sig_into xr KILL "$f.x.id"; wait "$x"; sig_into g KILL "$f.x.id"; } 2>/dev/null   # r10: one group, so that bash's "Killed" notice for its own child goes to /dev/null
         case "$xr $g" in *delivered*gone*) : ;; *) bad="$bad c(first '$xr', after the reap '$g', want delivered then gone)" ;; esac
         # (d) the r5 method, a bare number: it reaches whoever holds the number now (the bystander)
         sn=""; [ -r "$f.stale.id" ] && read -r _k sn _s < "$f.stale.id"
         kill -USR1 "$sn"   # K30-numeric: the one bare-number signal in the controls, to the control's own bystander
         sleep 0.3; lines_into hd "$f.hits"
         [ "$hd" = 2 ] || bad="$bad d(the bare number did not reach the bystander: hits $hd want 2)"
         { sig_into k KILL "$f.ok.id"; wait "$b"; wb=$?; } 2>/dev/null; echo "bystander: $k; reaped with status $wb" >> "$f.notes"
         case "$k" in *"member $b: delivered"*) [ "$wb" = 137 ] || bad="$bad cleanup(the bystander reaped with status $wb, want 137)" ;; *) bad="$bad cleanup(the bystander's KILL was not delivered: '$k')" ;; esac   # r10 (vera's r9 line 739): proved, not assumed
         [ -z "$bad" ]; verdict "$n" $? "(a) a stale identity, its number held by a live bystander: '$a', bystander hits $ha (want 0); (b) the healthy identity: '$h', hits $hb (want 1); (c) a reaped child: '$xr' then '$g'; (d) the r5 method, the bare number: hits $hd (want 2: it reached the bystander); the bystander removed by certified identity: $k; failures:${bad:- none}" ;;
    K31) C=static; n=K31-signal-sites; abs_dir_into root "$SUP"
         # (a) THIS revision's fixtures on the recogniser under test (the copy next to the supervisor);
         # (b) THIS revision's sweep of the whole crate under test, with the probe pin
         python3 -I controls/recogniser_fixtures.py "$root/controls/signal_sites.py" > "$OUT/$n.fixtures" 2>&1; fr=$?
         python3 -I controls/signal_sites.py "$root" > "$OUT/$n.stdout" 2> "$OUT/$n.stderr"; src=$?
         [ "$fr" = 0 ] && [ "$src" = 0 ]; verdict "$n" $? "(a) $(tr '\n' ' ' < "$OUT/$n.fixtures") (b) $(tr '\n' ' ' < "$OUT/$n.stdout")" ;;
    K32) observe_fault_case K32-helper-observation-fails helper-observe-fails 'could not be observed' ;;
    K33) observe_fault_case K33-probe-normalisation-fails probe-normalize-fails 'did not report within|could not be observed' ;;
    K34) bad="" inv="" fu=""
         for sub in selfzombie complete; do
           n=K34-self-row-$sub; : > "$OUT/$n.ps-calls"
           fps=""; [ "$sub" = selfzombie ] && fps="selfzombie:1"
           CV2_FAKE_PS=$fps CV2_FAKE_PS_MARK="$OUT/$n.ps-calls" run "$n" "" bash -c "$LINGER60"
           err="$OUT/$n.stderr"; group_into pg "$err"; obs=1; wcap left live_in_group "$pg"; [ "$WRC" = 0 ] || obs=0
           verified=no; grep -q 'VERIFIED EMPTY' "$err" && verified=yes
           selfz=no; grep -q "cannot be while it runs" "$err" && selfz=yes
           state_of st "$C"
           [ "$obs" = 1 ] || inv="the control's own listing failed ($sub)"
           [ -n "$pg" ] || inv="the driver's process group was never recorded ($sub)"
           if [ "$sub" = complete ]; then
             [ "$RC" = 0 ] && [ "$verified" = yes ] && [ "$selfz" = no ] && [ -z "$left" ] && [ "$st" = absent ] \
               || bad="$bad complete(exit $RC want 0, VERIFIED EMPTY $verified want yes, self-row refusal logged $selfz want no, left '$left', container $st)"
           else
             [ "$RC" = 73 ] && [ "$verified" = no ] && [ "$selfz" = yes ] && [ -z "$left" ] && [ "$st" = absent ] \
               || bad="$bad $sub(exit $RC want 73, VERIFIED EMPTY $verified want no, self-row refusal logged $selfz want yes, left '$left', container $st)"
           fi
           [ -n "$left" ] && rescue_group "$n" "$pg" "$ID"   # r10: certified KILL, then empty or an observed-only wait
         done
         [ -z "$bad" ]; verdict K34-self-row-zombie $? "a valid, unique-pid listing that shows the helper itself as a zombie (selfzombie:1, vera's two rows) gave exit 73, the self-row refusal and never VERIFIED EMPTY, with the descendant killed under custody; the complete-listing partner gave exit 0 and VERIFIED EMPTY; failures:${bad:- none}" "$inv"
         [ -n "$fu" ] && echo "      K34 follow-up:$fu" ;;
    K35) C=certifier; n=K35-certification-schedule; f="$OUT/$n"; bad=""; : > "$f.notes"
         # vera's schedule, deterministic: a custody line whose NUMBERS now name an unrelated parent and
         # its group-leading child (the effect of reuse on a stale log), and the run's end set first.
         # The bystanders are the control's own children: P (a parent) and its child Q, which leads its
         # own process group; P's command line does or does not carry the run's id.
         byst() {   # NAME ARG...: r8: a bystander parent P that is the control's DIRECT child (started here,
                    # not in a subshell), whose child Q leads its own group; sets BP and BQ
           python3 -I -c 'import subprocess, sys, time
c = subprocess.Popen(["sleep", "40"], process_group=0)
open(sys.argv[-1], "w").write(str(c.pid))
time.sleep(40)' "$@" "$f.$1.child" &
           BP=$!; for (( _i = 1; _i <= 100; _i++ )); do [ -s "$f.$1.child" ] && break; sleep 0.05; done
           read_into BQ "$f.$1.child"
         }
         line() { echo "$(ts) cv2-test-supervisor $1: driver $3 leads its own process group $3 (confirmed); custody: helper $2 holds it unreaped until after the last signal"; }
         ids_into() {   # VAR FILE: r9: the identities in FILE as "kind:pid ...", read without a capture;
                        # IDS_OK=0 if FILE cannot be read (an unread file is never "nothing certified")
           local __v=$1 __s="" k p s; IDS_OK=1
           if [ -r "$2" ]; then while read -r k p s; do __s="$__s$k:$p "; done < "$2"; else IDS_OK=0; fi
           printf -v "$__v" '%s' "$__s"
         }
         # (a) foreign: the numbers name a parent whose command line lacks the run id
         byst foreign; pa=$BP qa=$BQ; line RIDA "$pa" "$qa" > "$f.a.err"; : > "$f.a.id"
         certify_run "$f.a.err" "$f.a.id"; ids_into ia "$f.a.id"; oka=$IDS_OK
         # r9: a negative needs the predicate's own refusal, positively: the run id check refused
         ra=no; grep -qF "lacks 'group_runner.py RIDA '" "$f.a.id.notes" 2>/dev/null && ra=yes
         { [ "$oka" = 1 ] && [ -z "$ia" ] && [ "$ra" = yes ]; } || bad="$bad a(ids read $oka, certified '$ia', run-id refusal logged $ra)"
         # (b) the run ended first: an authentic-looking parent, but IDFILE.done is already set
         byst ended group_runner.py RIDB x; pb=$BP qb=$BQ; line RIDB "$pb" "$qb" > "$f.b.err"; : > "$f.b.id"; : > "$f.b.id.done"
         certify_run "$f.b.err" "$f.b.id"; ids_into ib "$f.b.id"; okb=$IDS_OK
         rb=no; grep -qF "the run ended before" "$f.b.id.notes" 2>/dev/null && rb=yes
         { [ "$okb" = 1 ] && [ -z "$ib" ] && [ "$rb" = yes ]; } || bad="$bad b(ids read $okb, certified '$ib', run-end refusal logged $rb)"
         # (c) the healthy partner: the parent carries the run id, the run has not ended
         byst healthy group_runner.py RIDC x; pc=$BP qc=$BQ; line RIDC "$pc" "$qc" > "$f.c.err"; : > "$f.c.id"
         certify_run "$f.c.err" "$f.c.id"; ids_into ic "$f.c.id"
         case " $ic " in *" helper:$pc "*"leader:$qc "*) : ;; *) bad="$bad c(the authentic run was not certified: '$ic')" ;; esac
         # cleanup: the bystanders, by identities certified here (Q its parent's child, P the control's own
         # child), leader last; r8 (mike's Low): every send must be delivered and every P reaped (137)
         cl=""
         for w in a:$pa:$qa b:$pb:$qb c:$pc:$qc; do
           t=${w%%:*}; p=${w#*:}; q=${p#*:}; p=${p%%:*}; : > "$f.$t.own"   # r10: parameter expansion, no here-string
           "${SIGSAFE[@]}" certify-pid "$q" "$p" member "$f.$t.own" >> "$f.notes" 2>&1
           "${SIGSAFE[@]}" certify-pid "$p" "$$" leader "$f.$t.own" >> "$f.notes" 2>&1
           { sig_into r KILL "$f.$t.own"; wait "$p"; ws=$?; } 2>/dev/null   # r10: one group, so that bash's "Killed" notice for its own child goes to /dev/null
           echo "$t: $r; P $p reaped with status $ws" >> "$f.notes"
           case "$r" in
             *"member $q: delivered"*"leader $p: delivered"*) [ "$ws" = 137 ] || bad="$bad cleanup-$t(P $p reaped with status $ws, want 137)" ;;
             *) bad="$bad cleanup-$t('$r')" ;;
           esac
           cl="$cl $t: Q and P delivered, P reaped $ws;"
         done
         [ -z "$bad" ]; verdict "$n" $? "(a) a stale custody line whose numbers name a foreign parent and its group: certified '${ia:-nothing}' (want nothing), the run-id refusal logged: $ra; (b) the run's end set first: certified '${ib:-nothing}' (want nothing), the run-end refusal logged: $rb; (c) the authentic partner: certified '${ic:-nothing}' (want helper and leader); cleanup by certified identity:${cl:- none}; failures:${bad:- none}" ;;
    K36) bad="" inv="" fu=""
         for sub in W T complete; do
           n=K36-self-state-$sub; : > "$OUT/$n.ps-calls"
           case $sub in W) fps="selfstate:W:1" ;; T) fps="selfas:T:1" ;; *) fps="" ;; esac
           CV2_FAKE_PS=$fps CV2_FAKE_PS_MARK="$OUT/$n.ps-calls" run "$n" "" bash -c "$LINGER60"
           err="$OUT/$n.stderr"; group_into pg "$err"; obs=1; wcap left live_in_group "$pg"; [ "$WRC" = 0 ] || obs=0
           verified=no; grep -q 'VERIFIED EMPTY' "$err" && verified=yes
           selfr=no; grep -q "cannot be while it runs" "$err" && selfr=yes
           state_of st "$C"
           [ "$obs" = 1 ] || inv="the control's own listing failed ($sub)"
           [ -n "$pg" ] || inv="the driver's process group was never recorded ($sub)"
           if [ "$sub" = W ]; then
             [ "$RC" = 73 ] && [ "$verified" = no ] && [ "$selfr" = yes ] && [ -z "$left" ] && [ "$st" = absent ] \
               || bad="$bad W(exit $RC want 73, VERIFIED EMPTY $verified want no, self-state refusal logged $selfr want yes, left '$left', container $st)"
           else
             [ "$RC" = 0 ] && [ "$verified" = yes ] && [ "$selfr" = no ] && [ -z "$left" ] && [ "$st" = absent ] \
               || bad="$bad $sub(exit $RC want 0, VERIFIED EMPTY $verified want yes, self-state refusal logged $selfr want no, left '$left', container $st)"
           fi
           [ -n "$left" ] && rescue_group "$n" "$pg" "$ID"   # r10: certified KILL, then empty or an observed-only wait
         done
         [ -z "$bad" ]; verdict K36-self-state-W $? "vera's two rows with the helper's own row as W (selfstate:W:1) gave exit 73, the self-state refusal and never VERIFIED EMPTY, the descendant killed under custody; a FULL listing with the helper's own row as T (selfas:T:1, a possible state) and the complete listing each gave exit 0 and VERIFIED EMPTY; failures:${bad:- none}" "$inv"
         [ -n "$fu" ] && echo "      K36 follow-up:$fu" ;;
    K37) observe_fault_case K37-probe-capture-empty probe-capture-empty 'could not be observed' none ;;
    K38) bad="" inv="" fu=""
         for sub in fault healthy; do
           n=K38-list-capture-$sub
           if [ "$sub" = fault ]; then CV2_SUP_TEST_FAULT=list-capture-empty run "$n" "" true; else run "$n" "" true; fi
           err="$OUT/$n.stderr"; state_of st "$C"
           va=no; grep -q 'VERIFIED ABSENT' "$err" && va=yes
           lu=no; grep -q 'list rc=unknown' "$err" && lu=yes
           if [ "$sub" = fault ]; then
             [ "$RC" = 71 ] && [ "$va" = no ] && [ "$lu" = yes ] && [ "$st" = absent ] \
               || bad="$bad fault(exit $RC want 71, VERIFIED ABSENT $va want no, 'list rc=unknown' logged $lu want yes, container $st)"
           else
             [ "$RC" = 0 ] && [ "$va" = yes ] && [ "$lu" = no ] && [ "$st" = absent ] \
               || bad="$bad healthy(exit $RC want 0, VERIFIED ABSENT $va want yes, 'list rc=unknown' logged $lu want no, container $st)"
           fi
           [ "$st" != absent ] && remove_exact "$C" "$n"
         done
         [ -z "$bad" ]; verdict K38-list-capture-empty $? "the container list query's capture forced EMPTY WITH STATUS 0 (CV2_SUP_TEST_FAULT=list-capture-empty): no witness, the query unknown, cleanup OBSERVATION FAILED, exit 71, never VERIFIED ABSENT (the container itself absent by the control's own query); the healthy partner: exit 0, VERIFIED ABSENT; failures:${bad:- none}" ;;
    K39) C=observer-capture; n=K39-observer-capture; f="$OUT/$n"; bad=""; : > "$f.notes"
         # a live bystander group: the control's own child, leading its own process group
         python3 -I -c 'import os, time
os.setpgid(0, 0)
time.sleep(30)' &
         b=$!; sleep 0.3
         wcap m live_in_group "$b"; ra=$WRC; ma=$m
         case " $ma " in *" $b "*) [ "$ra" = 0 ] || bad="$bad a(rc $ra)" ;; *) bad="$bad a(the live member $b was not seen: '$ma', rc $ra)" ;; esac
         CV2_CTRL_FAULT=observer-capture-empty wcap m live_in_group "$b"; rb=$WRC; mb=$m
         [ "$rb" != 0 ] || bad="$bad b(a VALID result '$mb' with the observer's own capture emptied)"
         CV2_CTRL_FAULT=wcap-capture-empty wcap m live_in_group "$b"; rcw=$WRC
         [ "$rcw" = unknown ] || bad="$bad c(wcap reported '$rcw' with its own capture emptied)"
         # the genuinely empty partner: the group of a child that led it, has ended and was reaped
         python3 -I -c 'import os
os.setpgid(0, 0)' &
         e=$!; wait "$e"
         wcap m live_in_group "$e"; rd=$WRC; md=$m
         { [ "$rd" = 0 ] && [ -z "$md" ] && group_empty "$e"; } || bad="$bad d(the genuinely empty group $e: rc $rd, members '$md')"
         : > "$f.b.id"; "${SIGSAFE[@]}" certify-pid "$b" "$$" member "$f.b.id" >> "$f.notes" 2>&1
         { sig_into r KILL "$f.b.id"; wait "$b"; wb=$?; } 2>/dev/null; echo "bystander: $r; reaped with status $wb" >> "$f.notes"
         case "$r" in *"member $b: delivered"*) [ "$wb" = 137 ] || bad="$bad cleanup(the bystander reaped with status $wb, want 137)" ;; *) bad="$bad cleanup(the bystander's KILL was not delivered: '$r')" ;; esac   # r10: proved, not assumed
         [ -z "$bad" ]; verdict "$n" $? "(a) a live group: rc $ra, its member seen; (b) the observer's outer capture emptied: rc $rb (want not 0: never a valid empty listing); (c) wcap's capture emptied: rc $rcw (want unknown); (d) a genuinely empty group: rc $rd, members '${md}' (valid and empty); the bystander removed by certified identity: $r; failures:${bad:- none}" ;;
    K40) C=static; n=K40-capture-inventory; abs_dir_into root "$SUP"
         # r10: (a) THIS revision's judge fixtures on the inventory under test (the copy next to the
         # supervisor), each on a copy of that crate; (b) the inventory of the crate under test
         python3 -I controls/inventory_fixtures.py "$root/controls/capture_inventory.py" "$root" > "$OUT/$n.fixtures" 2>&1; fr=$?
         python3 -I controls/capture_inventory.py "$root" --list > "$OUT/$n.stdout" 2> "$OUT/$n.stderr"; cr=$?
         [ "$fr" = 0 ] && [ "$cr" = 0 ]; verdict "$n" $? "(a) $(tr '\n' ' ' < "$OUT/$n.fixtures") (b) $(head -1 "$OUT/$n.stdout") $(grep '^mismatches' "$OUT/$n.stdout" | cut -c1-3000)" ;;
    K41) C=consumers; n=K41-consumers; f="$OUT/$n"; bad=""; : > "$f.notes"
         # r10: the capture-free consumers (controls/consumers.sh, or CV2_CONSUMERS), each with its fault and
         # its healthy partner. Every fault must give an EMPTY or unknown value, an INVALID verdict or a FAIL
         # line, never a number, "absent", "sent" or "empty"
         # (a) the clocks: an unread clock (CV2_CTRL_FAULT=clock-empty) and every span from it are EMPTY,
         # never a number that a bound check could accept (mike: r9's `END_MS=$(up_ms)`); and a reversed span
         CV2_CTRL_FAULT=clock-empty now_ms x; CV2_CTRL_FAULT=clock-empty wall_ms y
         span_ms d1 1000 "$x"; span_ms d2 "$y" 1000; span_ms d3 2000 1000
         ac=yes; [ -n "$d1" ] && [ "$d1" -le 5000 ] && ac=no   # what a bound check would do with it
         { [ -z "$x$y$d1$d2$d3" ] && [ "$ac" = yes ]; } || bad="$bad a-fault(clocks '$x' '$y', spans '$d1' '$d2' '$d3', want all empty; a 5 s bound refused it: $ac)"
         now_ms u0; wall_ms w0; sleep 0.3; now_ms u1; wall_ms w1; span_ms du "$u0" "$u1"; span_ms dw "$w0" "$w1"
         { [ -n "$du" ] && [ "$du" -ge 250 ] && [ "$du" -le 3000 ] && [ -n "$dw" ] && [ "$dw" -ge 250 ] && [ "$dw" -le 3000 ]; } \
           || bad="$bad a-healthy(0.3 s measured as '$du' ms monotonic and '$dw' ms realtime, want 250 to 3000)"
         # (b) the log stamps: four known stamps (one a leap day), malformed ones EMPTY; a log's first match
         iso_ms i1 2026-11-04T13:22:36.123Z; iso_ms i2 1970-01-01T00:00:00.000Z; iso_ms i3 2024-02-29T23:59:59.999Z; iso_ms i4 2000-03-01T00:00:00.001Z
         [ "$i1:$i2:$i3:$i4" = 1793798556123:0:1709251199999:951868800001 ] || bad="$bad b-healthy(stamps read '$i1' '$i2' '$i3' '$i4')"
         iso_ms j1 "2026-11-04 13:22:36"; iso_ms j2 ""; iso_ms j3 2026-11-04T13:22:36Z
         printf '%s\n' "2026-11-04T13:22:36.123Z cv2-test-supervisor RIDK: start: helper 42 launched" \
           "2026-11-04T13:22:37.000Z cv2-test-supervisor RIDK: cleanup: start" "2026-11-04T13:22:38.000Z cv2-test-supervisor RIDK: cleanup: start" > "$f.log"
         log_ms_into l1 "$f.log" 'cleanup: start'; log_ms_into l2 "$f.log" 'stopping: eof'; log_ms_into l3 "$f.missing" 'cleanup: start'
         [ "$l1" = 1793798557000 ] || bad="$bad b-healthy(the first 'cleanup: start' read '$l1', want 1793798557000)"
         [ -z "$j1$j2$j3$l2$l3" ] || bad="$bad b-fault(malformed stamps read '$j1' '$j2' '$j3', an absent line '$l2', an unreadable log '$l3'; want all empty)"
         # (c) the readers: an unreadable file is EMPTY, never 0; a known file reads exactly
         printf '%s\n' "100.25 run begin pid=1" "101.50 exists begin pid=2" "102.00 run begin pid=3" > "$f.fake"
         count_calls c1 "$f.fake" run; first_call_into fc1 "$f.fake" exists; lines_into n1 "$f.fake"; read_into r1 "$f.fake"; helper_into h1 "$f.log"
         count_calls c2 "$f.missing" run; first_call_into fc2 "$f.missing" run; lines_into n2 "$f.missing"; read_into r2 "$f.missing"; group_into g1 "$f.log"
         [ "$c1|$fc1|$n1|$r1|$h1" = "2|101500|3|100.25 run begin pid=1|42" ] || bad="$bad c-healthy(calls '$c1', first '$fc1', lines '$n1', line '$r1', helper '$h1')"
         [ -z "$c2$fc2$n2$r2$g1" ] || bad="$bad c-fault(an unreadable file read as '$c2' '$fc2' '$n2' '$r2', a log without a group line as '$g1'; want all empty, never 0)"
         # (d) the state query, DIRECT: one that cannot run, and no name, are unknown (never absent); a
         # name never created is absent
         REAL=/nonexistent/podman state_of s1 cv2-k41-never; state_of s2 ""; state_of s3 "cv2-k41-never-$$"
         [ "$s1:$s2" = unknown:unknown ] || bad="$bad d-fault(a query that could not run read '$s1', no name '$s2'; want unknown)"
         [ "$s3" = absent ] || bad="$bad d-healthy(a name never created read '$s3', want absent)"
         # (e) the signal report, DIRECT: not run (CV2_CTRL_FAULT=sig-not-run), an unreadable identity file and
         # an empty one are never "sent"; a certified child: delivered, then (reaped) gone
         sleep 60 & x=$!; : > "$f.x.id"; "${SIGSAFE[@]}" certify-pid "$x" "$$" member "$f.x.id" >> "$f.notes" 2>&1
         CV2_CTRL_FAULT=sig-not-run sig_into e1 KILL "$f.x.id"; o1="$SIGRC:$SIGOK:$SIGALL"; runs_prog "$x" sleep; a1=$?
         sig_into e2 KILL "$f.missing.id"; o2="$SIGOK:$SIGALL"; : > "$f.empty.id"; sig_into e3 KILL "$f.empty.id"; o3="$SIGOK:$SIGALL"
         { [ "${o1#*:}" = 0:0 ] && [ "${o1%%:*}" != 0 ] && [ -z "$e1" ] && [ "$a1" = 0 ] && [ "$o2" = 0:0 ] && [ "$o3" = 0:0 ]; } \
           || bad="$bad e-fault(not run: rc:ok:all '$o1', report '$e1', the child still running $a1 (0 = yes); unreadable ids '$o2' ($e2); empty ids '$o3' ($e3); want ok 0, all 0)"
         { sig_into e4 KILL "$f.x.id"; o4="$SIGOK:$SIGALL"; wait "$x"; wx=$?; sig_into e5 KILL "$f.x.id"; o5="$SIGOK:$SIGALL"; } 2>/dev/null
         case "$e4|$e5" in *"member $x: delivered|"*"member $x: gone") [ "$o4:$o5:$wx" = 1:1:1:0:137 ] || bad="$bad e-healthy(ok:all '$o4' then '$o5', reaped $wx)" ;;
           *) bad="$bad e-healthy(reports '$e4' then '$e5', want delivered then gone)" ;; esac
         # (f) an injected signal that was not delivered makes the case's verdict INVALID (in a subshell, so
         # that the probe verdicts count nothing here); the delivered partner's verdict is a PASS
         sleep 60 & y=$!; : > "$f.y.id"; "${SIGSAFE[@]}" certify-pid "$y" "$$" member "$f.y.id" >> "$f.notes" 2>&1
         ( RC=0 INJ=""; CV2_CTRL_FAULT=sig-not-run inject CONT "$f.y.id" member "$f.notes" "K41 fault"; verdict K41-probe 0 measured ) > "$f.vf" 2>&1; read_into vf "$f.vf"
         ( RC=0 INJ=""; inject CONT "$f.y.id" member "$f.notes" "K41 healthy"; verdict K41-probe 0 measured ) > "$f.vh" 2>&1; read_into vh "$f.vh"
         case "$vf" in "INVALID  K41-probe: the injected CONT was not delivered"*) : ;; *) bad="$bad f-fault(an undelivered injection gave '$vf', want INVALID)" ;; esac
         [ "$vh" = "PASS  K41-probe: measured" ] || bad="$bad f-healthy(a delivered injection gave '$vh', want PASS)"
         { sig_into k KILL "$f.y.id"; wait "$y"; wy=$?; } 2>/dev/null
         case "$k" in *"member $y: delivered"*) [ "$wy" = 137 ] || bad="$bad f-cleanup(reaped $wy, want 137)" ;; *) bad="$bad f-cleanup('$k')" ;; esac
         # (g) an owned rescue whose KILL never ran, of a live group: a FAIL line of its own and one more in
         # FAILS (in a subshell), after its observed-only wait (about 70 s); the healthy partner, the same
         # group: delivered, then the group EMPTY by the witnessed observer
         python3 -I -c 'import os, time
os.setpgid(0, 0)
time.sleep(150)' &
         g=$!; sleep 0.3; : > "$f.g.id"; "${SIGSAFE[@]}" certify-pid "$g" "$$" leader "$f.g.id" >> "$f.notes" 2>&1
         ( FAILS=0; CV2_CTRL_FAULT=sig-not-run rescue_group K41-rescue-fault "$g" "$f.g.id"; echo "fails=$FAILS" ) > "$f.rf" 2>&1
         { ( FAILS=0; rescue_group K41-rescue-healthy "$g" "$f.g.id"; echo "fails=$FAILS" ) > "$f.rh" 2>&1; wait "$g"; wg=$?; } 2>/dev/null
         { grep -qF "FAIL  K41-rescue-fault-cleanup: group $g is still live" "$f.rf" && grep -qx 'fails=1' "$f.rf"; } \
           || bad="$bad g-fault(a rescue whose KILL never ran, of a live group, was not a FAIL: see ${f##*/}.rf)"
         { grep -qF "K41-rescue-healthy follow-up: the left-over members of group $g, by certified identity: SIGKILL leader $g: delivered; the group is now empty (witnessed)" "$f.rh" \
           && grep -qx 'fails=0' "$f.rh" && [ "$wg" = 137 ]; } || bad="$bad g-healthy(see ${f##*/}.rh; reaped $wg, want 137)"
         # (h) runs_prog, wait_gone's builtin test: running, another program, no pid (unknown), an unread clock
         # (never "gone"), and a reaped child (gone)
         sleep 60 & z=$!; : > "$f.z.id"; "${SIGSAFE[@]}" certify-pid "$z" "$$" member "$f.z.id" >> "$f.notes" 2>&1
         runs_prog "$z" sleep; p1=$?; runs_prog "$z" group_runner; p2=$?; runs_prog "" sleep; p3=$?
         CV2_CTRL_FAULT=clock-empty wait_gone "$z" 1000; p4=$?
         { sig_into k KILL "$f.z.id"; wait "$z"; wz=$?; } 2>/dev/null; runs_prog "$z" sleep; p5=$?
         [ "$p1:$p2:$p3:$p4:$p5:$wz" = 0:1:2:2:1:137 ] || bad="$bad h(running $p1 want 0, another program $p2 want 1, no pid $p3 want 2, wait_gone with an unread clock $p4 want 2, reaped $p5 want 1, status $wz want 137)"
         [ -z "$bad" ]; verdict "$n" $? "(a) an unread clock and its spans empty, a reversed span empty, 0.3 s measured as $du/$dw ms; (b) known stamps exact, malformed ones empty; (c) unreadable files empty, never 0; (d) a failed state query unknown, a never-created name absent; (e) a signal not run, unreadable or empty identities never sent, a certified child delivered then gone; (f) an undelivered injection INVALID, a delivered one PASS; (g) a rescue that never signalled a live group a FAIL, its partner delivered and the group empty; (h) runs_prog and wait_gone; failures:${bad:- none}" ;;
    K42) C=static; n=K42-volume-lifecycle
         TMPDIR="$OUT" VOLUME_SUPERVISOR="$SUP" python3 -I -B controls/test_volume_lifecycle.py > "$OUT/$n.cells" 2>&1; cells_rc=$?
         TMPDIR="$OUT" VOLUME_SUPERVISOR="$SUP" python3 -I -B controls/volume_mutants.py > "$OUT/$n.mutants" 2>&1; mutants_rc=$?
         [ "$cells_rc" = 0 ] && [ "$mutants_rc" = 0 ]; verdict "$n" $? "fake-Podman volume cells exit $cells_rc; mutants exit $mutants_rc (logs $OUT/$n.cells and $OUT/$n.mutants)" ;;
    *)   echo "unknown case $k"; FAILS=$((FAILS + 1)) ;;
  esac
  FOLLOWUP_FAILED=0
  general_volume_followup "$k"
  [ "$FOLLOWUP_FAILED" = 0 ] || break
done
echo "# controls: $FAILS failed, of which $INVALID INVALID (nothing measured)"
exit $((FAILS > 0))
