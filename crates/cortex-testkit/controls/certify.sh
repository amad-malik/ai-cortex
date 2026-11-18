# The controls' certification of a supervised run, sourced by controls.sh (r6; r7: vera's
# F1B-CONTROL-CUSTODY-01-R6A). CV2_CERTIFIER names another file for a RED-first run (K35). Uses
# SIGSAFE and group_of from controls.sh. It never signals anything.
#
# certify_sup PID IDFILE: the supervisor, the control's own child just launched, by its parent and
#   its command line (which names $SUP, before and after the exec of env).
# certify_run ERRFILE IDFILE: once the supervisor has published, its helper, the leader and the
#   members, in ONE `sigsafe.py certify-run` call (the helper's pidfd held through the whole
#   interval, the run id on its command line, the leader its child; all-or-nothing). The end of the
#   run (IDFILE.done) is checked FIRST, as a refusal: a custody line left by a run that has ended
#   certifies nothing, and neither does a failed helper check.
certify_sup() { "${SIGSAFE[@]}" certify-pid "$1" "$$" sup "$2" "$SUP" >> "$2.notes" 2>&1; }
certify_run() {
  local line rid hp pg l
  for (( _i = 1; _i <= 1800; _i++ )); do
    [ -e "$2.done" ] && { echo "not certified: the run ended before its custody line was seen" >> "$2.notes"; return 0; }
    grep -q 'custody: helper [0-9]* holds it' "$1" 2>/dev/null && break
    sleep 0.05
  done
  sleep 0.3   # the drivers start their descendants at once
  [ -e "$2.done" ] && { echo "not certified: the run ended before it could be certified" >> "$2.notes"; return 0; }
  # r9 (mike's F1B-R8-CAPTURE-WITNESS): the custody line is read and parsed with builtins, without a
  # capture, so a capture that never ran cannot turn into "nothing to certify" unnoticed
  line="" rid="" hp="" pg=""
  if [ -r "$1" ]; then
    while IFS= read -r l; do case $l in *'custody: helper '*' holds it'*) line=$l; break ;; esac; done < "$1"
  fi
  if [ -n "$line" ]; then
    rid=${line#* cv2-test-supervisor }; rid=${rid%%:*}
    hp=${line#*custody: helper }; hp=${hp%% *}
    case $line in *' leads its own process group '*) pg=${line#* leads its own process group }; pg=${pg%% *} ;; esac
    [[ $hp =~ ^[0-9]+$ ]] || hp=""; [[ $pg =~ ^[0-9]+$ ]] || pg=""
  fi
  if [ -n "$hp" ] && [ -n "$pg" ] && [ -n "$rid" ]; then
    "${SIGSAFE[@]}" certify-run "$hp" "$rid" "$pg" "$2" >> "$2.notes" 2>&1
  else
    echo "not certified: the custody line names no helper, group or run id" >> "$2.notes"
  fi
  : > "$2.ready"
}
