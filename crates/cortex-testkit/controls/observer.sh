# The controls' own process observer, sourced by controls.sh (r5, mike's F1B-R0-05 item A: the
# observer validates its listing as the helper does). It never signals anything.
#
# A listing is trusted only if `ps` exits 0, EVERY row parses as `pgid pid stat`, and it contains
# the row of the control shell itself ($$, in its own process group, read from /proc). Anything
# else is a failed observation: the case is INVALID, never "empty". OBS_PS (default /usr/bin/ps)
# exists only so that K28 can inject a fault into this observer; the controls never set it.
# self_pgid is defined by controls.sh.
OBS_PS=${OBS_PS:-/usr/bin/ps}
obs_listing() {   # prints the validated listing; returns 1 if it cannot be trusted
  local out mg
  self_pgid_into mg   # r10: the anchor's own group, read with a builtin (no capture)
  out=$("$OBS_PS" -e -o pgid=,pid=,stat= 2>/dev/null) || return 1
  awk -v me="$$" -v mg="$mg" '
    /^[ \t]*$/ { next }
    !/^[ \t]*[0-9]+[ \t]+[0-9]+[ \t]+[^ \t]+[ \t]*$/ { bad = 1; exit }
    $2 == me && $1 == mg { anchor = 1 }
    { print }
    END { exit (bad || !anchor) ? 1 : 0 }' <<<"$out"
}
# r9 (mike's F1B-R8-CAPTURE-WITNESS): the outer capture of the validated listing ends with
# `obs-rc=<obs_listing's status>`, printed INSIDE it and checked OUTSIDE it: a capture that never
# ran (empty, status 0) cannot pass for a valid empty listing. obs_listing's own inner capture is
# guarded by its anchor row (an empty listing has no anchor). No group given is not an empty group.
# CV2_CTRL_FAULT=observer-capture-empty (the controls' own fault hook, K39) empties this capture.
live_in_group() {   # GROUP: its live (not zombie) members now; 1 if the listing cannot be trusted, 2 if no group
  local out
  [ -n "$1" ] || return 2
  out=$(obs_listing; printf 'obs-rc=%s\n' "$?")
  [ "${CV2_CTRL_FAULT:-}" = observer-capture-empty ] && out=""
  case $out in
    obs-rc=0) out="" ;;
    *$'\n'obs-rc=0) out=${out%$'\n'obs-rc=0} ;;
    *) return 1 ;;
  esac
  awk -v g="$1" '$1 == g && $3 !~ /^Z/ { print $2 }' <<<"$out"
}
