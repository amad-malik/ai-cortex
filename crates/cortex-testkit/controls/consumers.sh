# The controls' capture-free CONSUMERS, sourced by controls.sh (r10, mike's timing finding and vera's
# F1B-R9-OWNED-RESCUE-CAPTURE). CV2_CONSUMERS names another file, for a RED-first run.
#
# The failure model (accepted at r7 to r9): a command substitution whose pipe cannot be made gives
# EMPTY text with assignment status 0, although its command never ran. So no control DECISION reads
# a captured value: every value a decision uses (a clock sample, a log timestamp, a container's
# state, a recorded name, a group, a count, a signal's report) is set by one of these helpers into
# a caller variable with builtins only (read, mapfile, [[ =~ ]], parameter expansion, printf -v),
# and every effect (a signal, a removal) runs as a DIRECT command, never inside a capture. A missing,
# malformed or reversed sample is an EMPTY result, which the caller treats as INVALID; it is never 0.
# (K40 checks the controls by construction: no capture in a decision position. K41 exercises these
# helpers with their faults and healthy partners.)
#
# CV2_CTRL_FAULT (the controls' own fault hook, never set by a normal run) also takes, for K41 only:
#   clock-empty   now_ms and wall_ms read nothing (as if the clock read failed)
#   sig-not-run   sig_into's sigsafe command does not run (as if it could not be started)
# Uses REAL, SIGSAFE and OUT from controls.sh.

now_ms() {   # VAR: the monotonic clock in ms (CLOCK_BOOTTIME, /proc/uptime, 10 ms steps); "" if unreadable
  local __u __r
  if [ "${CV2_CTRL_FAULT:-}" != clock-empty ] && read -r __u __r < /proc/uptime && [[ $__u =~ ^([0-9]+)\.([0-9]{2})$ ]]; then
    printf -v "$1" '%s' $(( BASH_REMATCH[1] * 1000 + 10#${BASH_REMATCH[2]} * 10 ))
  else
    printf -v "$1" '%s' ""
  fi
}
wall_ms() {   # VAR: the realtime clock in ms since the epoch (Bash's EPOCHREALTIME); "" if unavailable
  local __t=${EPOCHREALTIME:-}
  if [ "${CV2_CTRL_FAULT:-}" != clock-empty ] && [[ $__t =~ ^([0-9]+)[.,]([0-9]{3}) ]]; then
    printf -v "$1" '%s' $(( BASH_REMATCH[1] * 1000 + 10#${BASH_REMATCH[2]} ))
  else
    printf -v "$1" '%s' ""
  fi
}
span_ms() {   # VAR START END: END - START, only if both are whole numbers and END >= START; else "" (INVALID)
  if [[ $2 =~ ^[0-9]+$ && $3 =~ ^[0-9]+$ ]] && (( $3 >= $2 )); then printf -v "$1" '%s' $(( $3 - $2 )); else printf -v "$1" '%s' ""; fi
}
iso_ms() {   # VAR STAMP: ms since the epoch of a YYYY-MM-DDTHH:MM:SS.mmmZ stamp (days from civil, in Bash arithmetic); "" if malformed
  if [[ $2 =~ ^([0-9]{4})-([0-9]{2})-([0-9]{2})T([0-9]{2}):([0-9]{2}):([0-9]{2})\.([0-9]{3})Z$ ]]; then
    local __y=$((10#${BASH_REMATCH[1]})) __mo=$((10#${BASH_REMATCH[2]})) __d=$((10#${BASH_REMATCH[3]}))
    local __H=$((10#${BASH_REMATCH[4]})) __M=$((10#${BASH_REMATCH[5]})) __S=$((10#${BASH_REMATCH[6]})) __ms=$((10#${BASH_REMATCH[7]}))
    local __era __yoe __doy __doe __days
    (( __mo <= 2 )) && __y=$(( __y - 1 ))
    __era=$(( __y / 400 )); __yoe=$(( __y - __era * 400 ))
    __doy=$(( (153 * (__mo > 2 ? __mo - 3 : __mo + 9) + 2) / 5 + __d - 1 ))
    __doe=$(( __yoe * 365 + __yoe / 4 - __yoe / 100 + __doy ))
    __days=$(( __era * 146097 + __doe - 719468 ))
    printf -v "$1" '%s' $(( ((__days * 24 + __H) * 60 + __M) * 60000 + __S * 1000 + __ms ))
  else
    printf -v "$1" '%s' ""
  fi
}
log_ms_into() {   # VAR FILE ERE: the first matching log line's own timestamp, in ms since the epoch; "" if none
  local __l __t=""
  if [ -r "$2" ]; then
    while IFS= read -r __l; do [[ $__l =~ $3 ]] && { __t=${__l%% *}; break; }; done < "$2"
  fi
  iso_ms "$1" "$__t"
}
first_call_into() {   # VAR LOG SUB: the first call of SUB in the fake's log, in ms (its /proc/uptime stamp); "" if none
  local __a __b __rest __v=""
  if [ -r "$2" ]; then
    while read -r __a __b __rest; do
      if [ "$__b" = "$3" ] && [[ $__a =~ ^([0-9]+)\.([0-9]{2})$ ]]; then __v=$(( BASH_REMATCH[1] * 1000 + 10#${BASH_REMATCH[2]} * 10 )); break; fi
    done < "$2"
  fi
  printf -v "$1" '%s' "$__v"
}
count_calls() {   # VAR LOG SUB: how many calls of SUB the fake's log records; "" if the log cannot be read
  local __a __b __rest __n=0
  if [ -r "$2" ]; then while read -r __a __b __rest; do [ "$__b" = "$3" ] && __n=$(( __n + 1 )); done < "$2"; else __n=""; fi
  printf -v "$1" '%s' "$__n"
}
lines_into() {   # VAR FILE: its number of lines; "" if it cannot be read
  local __l __n=0
  if [ -r "$2" ]; then while IFS= read -r __l; do __n=$(( __n + 1 )); done < "$2"; else __n=""; fi
  printf -v "$1" '%s' "$__n"
}
read_into() {   # VAR FILE: its first line; "" if it cannot be read
  local __x=""
  [ -r "$2" ] && IFS= read -r __x < "$2"
  printf -v "$1" '%s' "$__x"
}
state_of() {   # VAR NAME: absent (exists rc 1), present (rc 0) or unknown (any other status, or no name), DIRECT
  local __rc
  if [ -z "$2" ]; then printf -v "$1" '%s' unknown; return; fi
  "$REAL" container exists "$2" >/dev/null 2>&1; __rc=$?
  case $__rc in 1) printf -v "$1" '%s' absent ;; 0) printf -v "$1" '%s' present ;; *) printf -v "$1" '%s' unknown ;; esac
}
group_into() {   # VAR ERRFILE: the driver's process group, from the first "driver N leads its own process group" (or
                 # "driver N: NO CUSTODY") line of the supervisor's log; "" if none
  local __l __g="" __re='driver ([0-9]+)( leads its own process group|: NO CUSTODY)'
  if [ -r "$2" ]; then while IFS= read -r __l; do [[ $__l =~ $__re ]] && { __g=${BASH_REMATCH[1]}; break; }; done < "$2"; fi
  printf -v "$1" '%s' "$__g"
}
helper_into() {   # VAR ERRFILE: the helper's pid, from "start: helper N launched"; "" if none
  local __l __h="" __re='start: helper ([0-9]+) launched'
  if [ -r "$2" ]; then while IFS= read -r __l; do [[ $__l =~ $__re ]] && { __h=${BASH_REMATCH[1]}; break; }; done < "$2"; fi
  printf -v "$1" '%s' "$__h"
}
custody_helper_into() {   # VAR ERRFILE: the helper's pid, from "custody: helper N holds it"; "" if none
  local __l __h="" __re='custody: helper ([0-9]+) holds it'
  if [ -r "$2" ]; then while IFS= read -r __l; do [[ $__l =~ $__re ]] && { __h=${BASH_REMATCH[1]}; break; }; done < "$2"; fi
  printf -v "$1" '%s' "$__h"
}
status_path_into() {   # VAR PID: the helper's STATUS_FILE argument (its 8th argv word); "" if unreadable
  local -a __a=()
  [ -n "$2" ] && [ -r "/proc/$2/cmdline" ] && mapfile -d '' -t __a < "/proc/$2/cmdline"
  printf -v "$1" '%s' "${__a[7]:-}"
}
runs_prog() {   # PID WORD: 0 while PID runs a command line containing WORD; 1 once it has exited (no /proc entry,
               # a zombie's empty command line, or another program now); 2 if it exists but cannot be read.
               # Builtins only (r10: wait_gone's `tr | grep -q` read a pipe that could not be made as "exited")
  local -a __a=()
  if [ -n "$1" ] && mapfile -d '' -t __a 2>/dev/null < "/proc/$1/cmdline"; then
    case " ${__a[*]} " in *"$2"*) return 0 ;; *) return 1 ;; esac
  fi
  [ -n "$1" ] && [ ! -e "/proc/$1" ] && return 1
  return 2
}
self_pgid_into() {   # VAR: this control shell's own process group
  local __s __f
  read -r __s < "/proc/$$/stat"; __s=${__s##*) }; read -ra __f <<<"$__s"
  printf -v "$1" '%s' "${__f[2]:-}"
}
abs_dir_into() {   # VAR PATH: the absolute directory of PATH, by parameter expansion
  local __d=${2%/*}
  [ "$__d" = "$2" ] && __d=.
  case $__d in /*) ;; *) __d="$PWD/$__d" ;; esac
  printf -v "$1" '%s' "$__d"
}
fifo_holders_into() {   # VAR FIFO: every process of ours that holds FIFO open, as pid(comm):fd<n>:<r|w|rw> (a diagnostic)
  local __p __fd __fl __m __c __l __out=""
  for __p in /proc/[0-9]*; do
    for __fd in "$__p"/fd/*; do
      [[ $__fd -ef $2 ]] || continue
      __fl=0; __c=""
      if [ -r "$__p/fdinfo/${__fd##*/}" ]; then while read -r __l; do [[ $__l =~ ^flags:[[:space:]]+([0-7]+) ]] && __fl=${BASH_REMATCH[1]}; done < "$__p/fdinfo/${__fd##*/}"; fi
      [ -r "$__p/comm" ] && read -r __c < "$__p/comm"
      case $(( 8#$__fl & 3 )) in 0) __m=r ;; 1) __m=w ;; *) __m=rw ;; esac
      __out="$__out ${__p#/proc/}($__c):fd${__fd##*/}:$__m"
    done
  done
  printf -v "$1" '%s' "${__out# }"
}
sig_into() {   # VAR SIG IDFILE [KINDS]: the signals sent by a DIRECT sigsafe call (never inside a capture). VAR = its
               # per-target report, read back from a file with builtins. SIGRC = sigsafe's status. SIGOK=1 only if
               # it ran, reported at least one target, and every target was "delivered" or "gone"; SIGALL=1 only
               # if every target was "delivered" (never REFUSED, never "no certified identity", never unread)
  local __v=$1 __f="$OUT/.sig.$BASHPID.$RANDOM" __s="" __l __n=0; shift
  SIGOK=1 SIGALL=1
  if [ "${CV2_CTRL_FAULT:-}" = sig-not-run ]; then : > "$__f"; SIGRC=127; else "${SIGSAFE[@]}" signal "$@" > "$__f" 2>&1; SIGRC=$?; fi
  if [ -r "$__f" ]; then
    while IFS= read -r __l; do
      __s="$__s$__l "; __n=$(( __n + 1 ))
      case $__l in *": delivered") : ;; *": gone") SIGALL=0 ;; *) SIGOK=0; SIGALL=0 ;; esac
    done < "$__f"
  fi
  rm -f "$__f"
  { [ "$SIGRC" = 0 ] && [ "$__n" -gt 0 ]; } || { SIGOK=0; SIGALL=0; }
  printf -v "$__v" '%s' "${__s% }"
}
rescue_group() {   # NAME PG IDFILE: the owned group's certified members and leader, KILLed by a DIRECT sigsafe call;
                   # then the group must be EMPTY by the witnessed observer (5 s; uncertified leftovers are only
                   # observed, up to 65 s more, never signalled). Prints the outcome. A group still live, or one
                   # that cannot be observed, is a FAIL line of its own and counts in FAILS.
  local __r __k
  sig_into __r KILL "$3" member,leader
  for (( __k = 0; __k < 50; __k++ )); do group_empty "$2" && break; sleep 0.1; done
  if ! group_empty "$2"; then for (( __k = 0; __k < 130; __k++ )); do group_empty "$2" && break; sleep 0.5; done; fi
  if group_empty "$2"; then
    echo "      $1 follow-up: the left-over members of group $2, by certified identity: ${__r:-no report}; the group is now empty (witnessed)"
  else
    echo "FAIL  $1-cleanup: group $2 is still live, or cannot be observed, after its rescue: ${__r:-no report}"
    FAILS=$((FAILS + 1))
  fi
}
