#!/usr/bin/env bash
# cortex-v2's admitted test supervisor (kai's L-9, LAYOUT r7 §7): one throwaway Postgres per admitted
# run, owned here. It is Z-2's r3 run.sh (mike's R0 ACCEPT, review 575ee0e3, at d3de733; byte-identical
# to 5c81356's 8935c865), with only names changed: CV2_PG_PORT/CV2_PG_PASSWORD for the tests, the
# cv2-a-f1- container prefix, and the CV2_SUP_*/CV2_FAKE* control knobs. (Z2-D01 and Z2-D02 under
# Z2-R0-02; START PLAN s8.1 A2, T9 v2.)
#
# r2 (mike's F-1b R0, F1B-R0-05), r4: the driver runs in a process group of its own, under
# group_runner.py, and on EVERY path (it ends, it fails, its deadline, INT or TERM) the whole group is
# sent TERM, then KILL, and verified empty before cleanup is released. A TERM-resistant
# descendant can no longer outlive the run. A group that cannot be emptied is exit 73. And
# CV2_SUP_DRIVER_TIMEOUT must be a whole number from 1 to 540 (a shortening only): anything else
# is exit 64, before anything starts.
#
#   CV2_ADMIT_LOG=1 /path/to/workspace/spikes/cortex-v2-s0/admission/cv2-admit \
#       timeout --kill-after=120 780 crates/cortex-testkit/supervise.sh <driver command ...>
#
# It starts one throwaway Postgres 18 container that it owns, runs the driver (for example
# `cargo test --no-fail-fast`), and before it exits, on every path (normal end, driver failure,
# driver deadline, INT, TERM, and INT or TERM arriving during cleanup), it stops and removes that
# exact container and VERIFIES it is gone, then removes and verifies the run's named PostgreSQL
# volume. The admission lease (fd 9, inherited from cv2-admit) is held until both are accounted for.
#
# Bounds (T9 v2). Each phase has an ABSOLUTE deadline on the monotonic clock (/proc/uptime), and
# every command, probe and pause inside it is capped by the time left (r3; Z2-D01):
#   start      60 s   `podman run -d`; r2: up to 3 attempts, each on a new port, only on a port collision
#   readiness  30 s   pg_isready probes, the pauses between them, and the version read
#   driver    570 s   r4: under group_runner.py: the driver's 540 s deadline, then the TERM grace
#                     (15 s) and the verification (5 s) on the same clock: at most 560 s to the
#                     helper's final report (r5: the reap is inside it); this script waits for the
#                     helper at most 570 s (r5)
#   cleanup   120 s   stop, remove and verify the container and owned volume, including retries
# The four sum to 780 s. The outer timeout TERMs this script at 780 s and KILLs it at 900 s.
# A command is run as `timeout -k 1 <cap>` with cap = min(its own limit, time left - 1 s), so it
# has ended, killed if need be, by the phase deadline. With under 1.5 s left, it is not started.
# The container's own --timeout 600 s bounds the residue if this script is itself SIGKILLed.
#
# Signals (r3; Z2-D02). The first INT or TERM starts the shutdown: it stops the driver's whole
# process group through the helper (TERM, 15 s, then KILL, verified empty; r2, r4) and runs cleanup.
# r4: an INT or TERM that arrives while the helper starts is acted on once it has published
# the group. From then on, INT and TERM are DEFERRED, not reset: they are
# recorded and cleanup runs to its end. A run that saw any signal never exits 0.
#
# Exit codes, in this order of precedence:
#   64   CV2_SUP_DRIVER_TIMEOUT is not a whole number from 1 to 540 (r2); nothing was started;
#   70   cleanup failed: the container or its owned volume remains;
#   71   observation failed: container or volume absence could not be established (a query errored or ran out of
#        time, or the two independent queries disagree);
#   73   the driver's process group could not be verified empty, or custody of it was lost and
#        something was left in it (r2, r3);
#   the driver's own code if it is not 0 (72 if the database never became ready; 124 the driver's
#        deadline; 128 + the signal if a signal ended the driver);
#   128 + the signal, if the driver passed but INT or TERM arrived during cleanup;
#   0 only if the driver passed, no signal arrived, and absence was verified.
#
# Faults are injected from outside, with a fake `podman` (and `ps`) first on PATH
# (controls/fake-podman/), except ONE inert, off-by-default test hook (r6, the P-1 R4 pattern), read
# only by helper_probe and observe, and only for these three values:
#   CV2_SUP_TEST_FAULT=helper-observe-fails  turns the observation of a RUNNING helper into "unknown"
#                                            (an ended helper is still seen as ended) (K32);
#   CV2_SUP_TEST_FAULT=probe-capture-empty   r8: once the group is published, the jobs capture comes back
#                                            EMPTY WITH STATUS 0, as when its pipe could not be made (K37);
#   CV2_SUP_TEST_FAULT=list-capture-empty    r9: the container list query's capture (observe) comes back
#                                            EMPTY WITH STATUS 0 (K38).
# Nothing else reads it, and it changes no other path. Parameters: CV2_SUP_DRIVER_TIMEOUT=<s> shortens the driver's
# deadline (K7); CV2_SUP_NAME_FILE=<path> records the container's exact name for a control.
set -uo pipefail
set +m   # r7 (vera's qualification): job control off, whatever the environment; the helper probe assumes it

IMAGE_ID=5a9c2dbe6ab521f35e87c81124aa5137678992ddabb9c11ef46e04e5172af73c   # docker.io/pgvector/pgvector:0.8.2-pg18
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)-$$"
NAME="cv2-a-f1-$$-$RANDOM"
OWNER_LABEL="cv2.owner=f1-$RUN_ID"
VOLUME="$NAME-pgdata"
VOLUME_ELIGIBLE=0   # only after absence was established, before create can take effect
PORT=$((57000 + RANDOM % 1000))
PW=$(head -c 64 /dev/urandom | base64 | tr -dc 'A-Za-z0-9' | head -c 24)
DRIVER_TIMEOUT=${CV2_SUP_DRIVER_TIMEOUT:-540}
START_S=60 READY_S=30 CLEANUP_S=120
DRIVER_RC=1     # the driver's code, from the helper's final report
[ -n "${CV2_SUP_NAME_FILE:-}" ] && echo "$NAME" > "$CV2_SUP_NAME_FILE"   # lets a control check by exact name
DRIVER_PID=""
DRIVER_PGID=""  # set only once the driver is verified to lead its own process group
GROUP_RC=0      # 73 if the driver's process group could not be verified empty
CLEANED=0
CLEAN_RC=71     # until cleanup establishes absence, absence is unknown
FINISHING=0
PENDING=""      # 128 + the first signal that arrived during the shutdown
OBS=unknown
NOW=0

exec 3>&2   # the supervisor's own stderr, kept for log(): a trap can run while a command's stderr is redirected
log() { printf '%s cv2-test-supervisor %s: %s\n' "$(date -u +%FT%T.%3NZ)" "$RUN_ID" "$*" >&3; }

# r2: a shortening only, as a whole number of seconds from 1 to 540. 0 would disable timeout(1)
# and a larger value would extend the declared phase. Checked before anything starts.
if ! [[ "$DRIVER_TIMEOUT" =~ ^[1-9][0-9]{0,2}$ ]] || [ "$DRIVER_TIMEOUT" -gt 540 ]; then
  log "refused: CV2_SUP_DRIVER_TIMEOUT='$DRIVER_TIMEOUT' is not a whole number from 1 to 540; nothing was started"
  exit 64
fi

now() {   # NOW := the monotonic clock in ms (CLOCK_BOOTTIME via /proc/uptime, 10 ms steps); no subshell
  local u _
  read -r u _ < /proc/uptime
  NOW=$(( ${u%.*} * 1000 + 10#${u#*.} * 10 ))
}

run_capped() {   # DEADLINE_MS MAX_S cmd...: cmd under `timeout -k 1`, ended by the deadline; 124 if not started
  local dl=$1 max=$2 rem cap cs; shift 2
  now; rem=$(( dl - NOW - 1000 ))          # the time left, less the 1 s between TERM and KILL
  [ "$rem" -lt 500 ] && return 124
  cap=$(( max * 1000 < rem ? max * 1000 : rem ))
  printf -v cs '%d.%03d' $(( cap / 1000 )) $(( cap % 1000 ))
  timeout -k 1 "$cs" "$@"
}

nap() {   # DEADLINE_MS MAX_MS: sleep, but never past the deadline
  local dl=$1 max=$2 rem cap cs
  now; rem=$(( dl - NOW ))
  [ "$rem" -le 0 ] && return
  cap=$(( max < rem ? max : rem ))
  printf -v cs '%d.%03d' $(( cap / 1000 )) $(( cap % 1000 ))
  sleep "$cs"
}

# Absence of OUR container, by two independent queries, each capped by the deadline. Sets OBS to
# absent|present|unknown. A query that errors, times out or is not started for lack of time
# gives unknown.
# r9 (mike's F1B-R8-CAPTURE-WITNESS): the list query's capture ends with `list-rc=<its status>`,
# printed INSIDE the capture after the query has run, and checked OUTSIDE it. A capture that never
# ran (its pipe could not be made: empty text with status 0) or was cut short is an UNKNOWN query,
# never an empty list, so it can never complete the two-query absence. (The exists query runs
# directly, not in a capture.)
observe() {
  local dl=$1 e l lr
  run_capped "$dl" 20 podman container exists "$NAME" >/dev/null 2>&1; e=$?
  l=$(run_capped "$dl" 20 podman ps -a --filter "label=$OWNER_LABEL" --format '{{.Names}}' 2>/dev/null; printf 'list-rc=%s\n' "$?")
  if [ "${CV2_SUP_TEST_FAULT:-}" = list-capture-empty ]; then l=""; fi
  case $l in
    list-rc=*) lr=${l#list-rc=}; l="" ;;
    *$'\n'list-rc=*) lr=${l##*$'\n'list-rc=}; l=${l%$'\n'list-rc=*} ;;
    *) lr=unknown; l="" ;;   # no witness: the query did not run, or was cut short
  esac
  [[ $lr =~ ^[0-9]+$ ]] || lr=unknown
  if [ "$e" = 1 ] && [ "$lr" = 0 ] && [ -z "$l" ]; then OBS=absent
  elif [ "$e" = 0 ] && [ "$lr" = 0 ] && [ -n "$l" ]; then OBS=present
  else log "observe: exists rc=$e, list rc=$lr, list='$l'"; OBS=unknown
  fi
}

## Independently establish the exact volume's presence and ownership. Failed, truncated or
## disagreeing queries are unknown; an unrelated volume is never eligible for removal.
observe_volume() {
  local dl=$1 e l lr
  run_capped "$dl" 20 podman volume exists "$VOLUME" >/dev/null 2>&1; e=$?
  l=$(run_capped "$dl" 20 podman volume ls --filter "label=$OWNER_LABEL" --format '{{.Name}}' 2>/dev/null; printf 'list-rc=%s\n' "$?")
  case $l in
    list-rc=*) lr=${l#list-rc=}; l="" ;;
    *$'\n'list-rc=*) lr=${l##*$'\n'list-rc=}; l=${l%$'\n'list-rc=*} ;;
    *) lr=unknown; l="" ;;
  esac
  [[ $lr =~ ^[0-9]+$ ]] || lr=unknown
  if [ "$e" = 1 ] && [ "$lr" = 0 ] && [ -z "$l" ]; then VOLUME_OBS=absent
  elif [ "$e" = 0 ] && [ "$lr" = 0 ] && [ "$l" = "$VOLUME" ]; then VOLUME_OBS=present
  else log "volume observe $VOLUME: exists rc=$e, list rc=$lr, list='$l'"; VOLUME_OBS=unknown
  fi
}

cleanup() {   # at most CLEANUP_S s on the monotonic clock; sets CLEAN_RC (0 verified, 70 present, 71 unknown)
  [ "$CLEANED" = 1 ] && return
  CLEANED=1
  local t0 dl
  now; t0=$NOW; dl=$(( NOW + CLEANUP_S * 1000 ))
  log "cleanup: start (container $NAME), deadline ${CLEANUP_S} s"
  run_capped "$dl" 40 podman stop --time 5 "$NAME" >/dev/null 2>&1; log "cleanup: stop rc=$?"
  run_capped "$dl" 40 podman rm --force --volumes "$NAME" >/dev/null 2>&1;  log "cleanup: rm rc=$? (an absent container is fine; absence is verified next)"
  observe "$dl"
  while [ "$OBS" = present ]; do
    now; [ $(( dl - NOW )) -lt 4000 ] && break   # too little left for another removal and two queries
    nap "$dl" 1000
    run_capped "$dl" 20 podman rm --force --volumes "$NAME" >/dev/null 2>&1
    observe "$dl"
  done
  now
  case $OBS in
    absent)  log "cleanup: container confirmed absent after $(( NOW - t0 )) ms" ;;
    present) CLEAN_RC=70; log "cleanup: FAILED, $NAME is still present after $(( NOW - t0 )) ms (deadline)" ;;
    *)       CLEAN_RC=71; log "cleanup: OBSERVATION FAILED after $(( NOW - t0 )) ms" ;;
  esac
  [ "$OBS" = absent ] || return
  observe_volume "$dl"
  while [ "$VOLUME_OBS" = present ] && [ "$VOLUME_ELIGIBLE" = 1 ]; do
    now; [ $(( dl - NOW )) -lt 4000 ] && break
    run_capped "$dl" 20 podman volume rm "$VOLUME" >/dev/null 2>&1
    log "cleanup: remove volume $VOLUME rc=$?"
    observe_volume "$dl"
    [ "$VOLUME_OBS" = present ] && nap "$dl" 1000
  done
  now
  case $VOLUME_OBS in
    absent)  CLEAN_RC=0; log "cleanup: volume $VOLUME VERIFIED ABSENT after $(( NOW - t0 )) ms" ;;
    present) CLEAN_RC=70; log "cleanup: FAILED, volume $VOLUME is still present after $(( NOW - t0 )) ms" ;;
    *)       CLEAN_RC=71; log "cleanup: volume $VOLUME OBSERVATION FAILED after $(( NOW - t0 )) ms" ;;
  esac
}

defer() {   # a signal during the shutdown: record it, and let cleanup run to its end
  [ -z "$PENDING" ] && PENDING=$2
  log "received $1 during shutdown (handled when the running command ended): deferred until cleanup is verified"
}

finish() {   # $1: the driver's or the signal's code
  local rc=$1 final why
  [ "$FINISHING" = 1 ] && return
  FINISHING=1
  trap - EXIT                    # no second finish may overwrite the code below (K5/K6, r2)
  trap 'defer INT 130' INT       # r3 (Z2-D02): deferred, not reset, while cleanup runs
  trap 'defer TERM 143' TERM
  cleanup
  rm -rf "${RUN_DIR:-/nonexistent-cv2-run-dir}"
  if [ "$CLEAN_RC" != 0 ]; then final=$CLEAN_RC; why="cleanup outranks driver rc=$rc"
  elif [ "$GROUP_RC" != 0 ]; then final=$GROUP_RC; why="the driver's process group was not verified empty (driver rc=$rc)"
  elif [ "$rc" != 0 ]; then final=$rc; why="driver, cleanup verified"
  elif [ -n "$PENDING" ]; then final=$PENDING; why="a signal arrived during cleanup, which was verified; never 0"
  else final=0; why="driver passed, cleanup verified"
  fi
  log "exit $final ($why)"
  exit "$final"
}

# r4 (mike's F1B-R0-05 items A-D; vera's SUP-CUSTODY-01 and -02). The driver runs under
# group_runner.py (next to this script; standard library only, `python3 -I`), which holds custody
# of the driver's process group from its creation until after the last signal: the driver is
# its own unreaped child, so the group number cannot be reused, and only it signals the group.
# This script never signals a process group, and never signals the helper by pid: it talks to the
# helper only through a private FIFO (`stop`; or EOF, if this script dies). The helper's scans
# keep their errors (a failed, timed-out, empty or malformed `ps` listing is UNKNOWN) and are
# charged to one absolute budget. Its final report decides GROUP_RC: anything but a verified
# empty group, or no report at all (custody lost), is exit 73.
#
# r5 (vera's F1B-CHANNEL-01, mike's G): this script is the ONLY writer of the FIFO. It is created
# and opened (fd 5) only just before the helper starts, after `podman run` and the readiness
# probes, so no podman process (and no long-lived conmon) inherits a writer; and the helper is
# started with fd 5 closed, so it holds only its read end. If this script dies, even by SIGKILL,
# the helper's stdin therefore reaches EOF and it stops the driver itself. (A short-lived child of
# this script, such as a 20 ms pause in the publication poll, can hold the writer for its own few
# milliseconds.)
# r5 (vera's SUP-DEADLINE-01): this script's wait for the helper is bounded as well. The helper's
# own budget is the driver's deadline + the grace + the verification (it no longer waits for a
# leader whose exit is not confirmed); AWAIT_DL adds 10 s for its start and its report: 570 s at
# the default 540 s deadline, the declared driver phase. A helper that has not ended by then is
# exit 73: this script does not take custody over and does not signal the group; cleanup goes on.
# r6 (mike's B/G remainder, vera's SUP-WAIT-OBS-01): the helper's state comes from Bash's own job
# table (the helper is this script's ONLY background job), three-valued: run (a running job; a
# stopped helper counts as running, since a non-interactive shell does not learn of SIGSTOP),
# ended (no longer a running job: Bash has reaped it, and `wait` returns its saved status at once)
# or unknown (the observation failed). ONLY ended leads to `wait`; run and unknown stay inside
# AWAIT_DL. No /proc read, so no reused pid can be mistaken for the helper. Once the helper is done
# or given up, fd 5 is closed: this script's later commands (cleanup's podman calls) hold no writer.
HELPER="$(cd "$(dirname "$0")" && pwd)/group_runner.py"
RUN_DIR=$(mktemp -d "${TMPDIR:-/tmp}/cv2-sup.XXXXXX")
CMD_FIFO="$RUN_DIR/cmd" STATUS_FILE="$RUN_DIR/status"
HELPER_PID="" FINAL_SEEN=0 AWAIT_DL=0 HELPER_T0=0
DRIVER_GRACE_S=15 DRIVER_VERIFY_S=5

# The helper's state, from Bash's job table: run, ended or unknown (r6). r7 (mike's B/G remainder):
# ONE command substitution, its status checked; the captured list is validated (empty, or pids one
# per line, by Bash's own [[ =~ ]]) and matched as it is, with no second conversion. r8 (mike's
# positive query witness): the capture ends with `jobs-rc=<jobs's status>`, printed INSIDE it after
# jobs has run, and checked OUTSIDE it. A capture that never ran (its pipe could not be made: empty
# text with status 0) has no witness, and neither does one cut short: unknown. Only `jobs-rc=0`
# with a validated list without the helper is "ended". (K31 pins the substitution and the check.)
helper_probe() {
  local r list re=$'^[0-9]+(\n[0-9]+)*$'
  r=$(jobs -rp 2>/dev/null; printf 'jobs-rc=%s\n' "$?") || { echo unknown; return; }
  if [ "${CV2_SUP_TEST_FAULT:-}" = probe-capture-empty ] && [ -n "${DRIVER_PGID:-}" ]; then r=""; fi
  case $r in
    jobs-rc=0) list="" ;;
    *$'\n'jobs-rc=0) list=${r%$'\n'jobs-rc=0} ;;
    *) echo unknown; return ;;   # no witness, or jobs failed: never "ended"
  esac
  [ -z "$list" ] || [[ $list =~ $re ]] || { echo unknown; return; }
  case $'\n'"$list"$'\n' in
    *$'\n'"$HELPER_PID"$'\n'*)
      if [ "${CV2_SUP_TEST_FAULT:-}" = helper-observe-fails ]; then echo unknown; else echo run; fi ;;
    *) echo ended ;;
  esac
}

# Waits for the helper, but not past AWAIT_DL. 0 once it has ended (and was waited for); 1 if not.
await_helper() {
  local st
  while :; do
    st=$(helper_probe)
    [ "$st" = ended ] && break
    now
    if [ "$NOW" -ge "$AWAIT_DL" ]; then
      GROUP_RC=73
      if [ "$st" = unknown ]; then
        log "driver group ${DRIVER_PGID:-?}: the helper $HELPER_PID's state could not be observed within the driver phase ($(( (AWAIT_DL - HELPER_T0) / 1000 )) s); it is NOT waited for, custody stays with it, and the group is NOT signalled by this script"
      else
        log "driver group ${DRIVER_PGID:-?}: the helper $HELPER_PID did not report within the driver phase ($(( (AWAIT_DL - HELPER_T0) / 1000 )) s); custody stays with it, and the group is NOT signalled by this script"
      fi
      exec 5>&-   # r6: the channel is closed; once this script is gone the helper sees EOF
      return 1
    fi
    nap "$AWAIT_DL" 50
  done
  wait "$HELPER_PID" 2>/dev/null
  exec 5>&-
  return 0
}

# Reads the helper's final report (once it has exited) into rc and GROUP_RC.
read_final() {
  local line
  line=$(grep '^final ' "$STATUS_FILE" 2>/dev/null | tail -1)
  if [ -z "$line" ]; then
    GROUP_RC=73; log "driver group ${DRIVER_PGID:-?}: custody LOST (the helper ${HELPER_PID:-?} ended without a final report); the group is NOT signalled by this script"
    return
  fi
  FINAL_SEEN=1
  DRIVER_RC=$(sed -n 's/.* rc=\([0-9][0-9]*\) .*/\1/p' <<<"$line")
  [ -n "$DRIVER_RC" ] || DRIVER_RC=1   # r5: rc=unconfirmed (the leader's exit was not confirmed)
  case "$line" in
    *group=verified*) : ;;
    *) GROUP_RC=73; log "driver group ${DRIVER_PGID:-?}: not verified empty (${line#final })" ;;
  esac
  case "$line" in
    *leader=unreaped*) log "driver group ${DRIVER_PGID:-?}: the leader's exit was not confirmed within the budget; it is left unreaped and unsignalled" ;;
  esac
}

# Stops the driver through the helper and waits for it, within the helper's own budget.
stop_driver() {
  if [ -n "$HELPER_PID" ]; then
    printf 'stop\n' >&5 2>/dev/null
    await_helper && read_final
  fi
}

on_signal() {
  local sig=$1 code=$2
  trap 'defer INT 130' INT; trap 'defer TERM 143' TERM   # one signal starts the shutdown; later ones wait
  log "received $sig"
  stop_driver
  finish "$code"
}
trap 'on_signal INT 130' INT
trap 'on_signal TERM 143' TERM
trap 'finish 1' EXIT   # an unexpected exit path still cleans up and verifies

log "start: container $NAME ($OWNER_LABEL), image $IMAGE_ID, driver timeout ${DRIVER_TIMEOUT}s"
now; START_DL=$(( NOW + START_S * 1000 ))
observe_volume "$START_DL"
if [ "$VOLUME_OBS" != absent ]; then
  log "refused: volume $VOLUME is present or its absence cannot be established"
  trap - EXIT; finish 72
fi
VOLUME_ELIGIBLE=1
if ! run_capped "$START_DL" 20 podman volume create --label cortexv2=dev --label "$OWNER_LABEL" "$VOLUME" >/dev/null; then
  log "failed to create volume $VOLUME"; trap - EXIT; finish 72
fi
observe_volume "$START_DL"
if [ "$VOLUME_OBS" != present ]; then
  log "failed to verify ownership of volume $VOLUME"; trap - EXIT; finish 72
fi
log "start: created owned volume $VOLUME"
# r2 (kai's port ruling (b), row 9c3f5b35): the random port can be in use. Only on podman's
# "Address already in use", the start is retried on a NEW random port in 57000-57999, at most 3
# attempts in all, all inside the one absolute start deadline. Every attempt is logged and counted.
# Any other start failure, or a third collision, is exit 72 as before.
ATTEMPTS=0 TRIED=""
while :; do
  ATTEMPTS=$(( ATTEMPTS + 1 )); TRIED="$TRIED $PORT"
  log "start: attempt $ATTEMPTS of 3 on 127.0.0.1:$PORT"
  RUN_ERR=$(run_capped "$START_DL" "$START_S" podman run -d --rm --name "$NAME" --label cortexv2=dev --label "$OWNER_LABEL" --volume "$VOLUME:/var/lib/postgresql" \
      --pull=never --timeout 600 --memory 2g --cpus 1 -p "127.0.0.1:$PORT:5432" -e POSTGRES_PASSWORD="$PW" "$IMAGE_ID" 2>&1 >/dev/null)
  RUN_RC=$?
  [ -n "$RUN_ERR" ] && printf '%s\n' "$RUN_ERR" >&3
  [ "$RUN_RC" = 0 ] && break
  if [ "$ATTEMPTS" -lt 3 ] && [[ "$RUN_ERR" == *"Address already in use"* ]]; then
    run_capped "$START_DL" 10 podman rm --force --volumes "$NAME" >/dev/null 2>&1   # no remnant may hold the name
    while [[ " $TRIED " == *" $PORT "* ]]; do PORT=$((57000 + RANDOM % 1000)); done
    log "start: port collision on attempt $ATTEMPTS; retrying on a new port"
    continue
  fi
  log "podman run failed or ran out of its ${START_S} s (attempt $ATTEMPTS of 3, rc $RUN_RC)"; trap - EXIT; finish 72
done
log "start: started on 127.0.0.1:$PORT after $ATTEMPTS attempt(s)"

now; R0=$NOW; READY_DL=$(( NOW + READY_S * 1000 ))
ready=0
while :; do
  if run_capped "$READY_DL" 5 podman exec "$NAME" pg_isready -q -h 127.0.0.1 -U postgres 2>/dev/null; then ready=1; break; fi
  now; [ $(( READY_DL - NOW )) -lt 1500 ] && break
  nap "$READY_DL" 1000
done
now
if [ "$ready" != 1 ]; then log "not ready within ${READY_S} s ($(( NOW - R0 )) ms)"; trap - EXIT; finish 72; fi
PGV=$(run_capped "$READY_DL" 5 podman exec "$NAME" postgres --version 2>/dev/null) || PGV="(version not read within the readiness deadline)"
now; log "ready after $(( NOW - R0 )) ms: $PGV"

export CV2_PG_PORT="$PORT" CV2_PG_PASSWORD="$PW"
# r4 (mike's item D): INT and TERM that arrive while the helper starts are recorded, and acted on
# only once the helper has published the driver's group, so a shutdown always has the whole
# owned state to work with.
PUB_PENDING=""
trap 'PUB_PENDING=${PUB_PENDING:-INT}' INT
trap 'PUB_PENDING=${PUB_PENDING:-TERM}' TERM
mkfifo -m 600 "$CMD_FIFO"
exec 5<>"$CMD_FIFO"   # r5: opened only now; this script is its only writer from here on
now; HELPER_T0=$NOW; AWAIT_DL=$(( NOW + (DRIVER_TIMEOUT + DRIVER_GRACE_S + DRIVER_VERIFY_S + 10) * 1000 ))
python3 -I "$HELPER" "$RUN_ID" "$DRIVER_TIMEOUT" "$DRIVER_GRACE_S" "$DRIVER_VERIFY_S" "$STATUS_FILE" -- "$@" < "$CMD_FIFO" 5>&- &
HELPER_PID=$!
log "start: helper $HELPER_PID launched; INT and TERM are deferred until it publishes the driver's group"
now; dl=$(( NOW + 6000 ))
while ! grep -q '^group ' "$STATUS_FILE" 2>/dev/null; do
  now; [ "$NOW" -ge "$dl" ] && break
  [ "$(helper_probe)" = ended ] && break
  nap "$dl" 20
done
if read -r _ DRIVER_PGID _ DRIVER_PID GROUP_STATE < <(grep '^group ' "$STATUS_FILE" 2>/dev/null | head -1) && [ -n "$DRIVER_PID" ]; then
  log "driver $DRIVER_PID leads its own process group $DRIVER_PGID ($GROUP_STATE); custody: helper $HELPER_PID holds it unreaped until after the last signal"
else
  GROUP_RC=73; log "driver: NO CUSTODY (the helper did not publish the driver's group); it is told to stop"
fi
trap 'on_signal INT 130' INT
trap 'on_signal TERM 143' TERM
case "$PUB_PENDING" in
  INT)  log "received INT while the helper started: deferred until ownership was published"; on_signal INT 130 ;;
  TERM) log "received TERM while the helper started: deferred until ownership was published"; on_signal TERM 143 ;;
esac
[ "$GROUP_RC" = 73 ] && printf 'stop\n' >&5 2>/dev/null
await_helper && read_final
log "driver exited rc=$DRIVER_RC"
trap - EXIT
finish "$DRIVER_RC"
