#!/bin/sh
# install-witness.sh: the install witness (G-S5-2, S5 amendment 14 / A74). SOURCE it; it defines functions and runs nothing.
#
#   witness_secrets   CONTAINER NAME...   the container's /run/secrets holds exactly these names, as a set
#   witness_container CONTAINER           the same, with the set this stack allows that container (witness_expected)
#   witness_expected  CONTAINER           prints the allowed names, one per line; fails for a container it does not know
#
# What it shows: after an install, the server container that is ACTUALLY RUNNING holds the app secret and nothing else
# (and the database container its three). It is the running partner of S5-16, which reads the compose file only.
#
# What it does: ONE call, `podman exec CONTAINER ls -1A /run/secrets`, as the container's own user. It lists NAMES, hidden
# dot-entries included (-A; plain `ls -1` would not show them, hale's finding F-1; -A and not -a, which adds . and ..). It never
# reads a secret file: the names are not secret, the values are.
# It fails closed: a failing exec, an empty listing, a listing line that is not a secret name of this stack
# (^cv2-s5-[a-z0-9-]{1,40}$), a name listed twice (a directory cannot do that: an entry whose name holds a newline adds lines),
# or a listing that is not EXACTLY the expected set is a failure. A failure line names the
# container and the unexpected and missing names; any name from the listing is shown cut to 40 characters with every
# character outside [A-Za-z0-9._-] turned into "?". Return 0 pass, 1 fail, 2 bad arguments (usage, nothing is run).
#
# Not covered, on purpose: the one-shot `migrate` container exits, so it cannot be listed (S5-16 checks its mounts
# statically); the host (custody there is by conduct, kai's known-limit sentence).
# Plain POSIX sh. No function here depends on the caller's `set -e` or `set -u`; names are prefixed because sh has no `local`.

witness_nl='
'

# a secret name of this stack: cv2-s5- then 1 to 40 of [a-z0-9-]. Spelled out, not as a range, so a locale cannot widen it.
witness_name_ok() {
  case ${1-} in cv2-s5-*) ;; *) return 1 ;; esac
  wn_rest=${1#cv2-s5-}
  [ -n "$wn_rest" ] && [ "${#wn_rest}" -le 40 ] || return 1
  case $wn_rest in *[!abcdefghijklmnopqrstuvwxyz0123456789-]*) return 1 ;; esac
  return 0
}

# at most 40 bytes, anything outside [A-Za-z0-9._-] shown as "?", "..." when cut, "(empty)" for nothing
witness_cut() {
  [ -n "${1-}" ] || { printf '(empty)'; return 0; }
  printf '%s' "$1" | LC_ALL=C head -c 40 | LC_ALL=C tr -c 'A-Za-z0-9._-' '?'
  if [ "$(printf '%s' "$1" | LC_ALL=C wc -c)" -gt 40 ]; then printf '...'; fi
}

witness_expected() {
  case ${1-} in
    cv2-s5-server) printf '%s\n' cv2-s5-app-password ;;
    cv2-s5-db) printf '%s\n' cv2-s5-app-password cv2-s5-db-superuser cv2-s5-owner-password ;;
    *) return 1 ;;
  esac
}

witness_secrets() {
  if [ $# -lt 2 ]; then
    echo "witness: usage: witness_secrets CONTAINER EXPECTED_NAME..." >&2
    return 2
  fi
  ws_ctr=$1; shift
  if ! witness_name_ok "$ws_ctr"; then
    echo "witness: usage: not a container of this stack: $(witness_cut "$ws_ctr")" >&2
    return 2
  fi
  ws_expected=
  for ws_e in "$@"; do
    if ! witness_name_ok "$ws_e"; then
      echo "witness: usage: not a secret name of this stack: $(witness_cut "$ws_e")" >&2
      return 2
    fi
    ws_expected="$ws_expected$ws_e$witness_nl"
  done

  # The listing, kept byte for byte: "&& echo x" keeps trailing blank lines, and makes the status that of podman.
  # (`if ... else` and not a bare assignment, so a caller's `set -e` cannot end its shell here.)
  if ws_out=$(podman exec "$ws_ctr" ls -1A /run/secrets 2>/dev/null && echo x); then
    ws_rc=0
  else
    ws_rc=$?
  fi
  if [ "$ws_rc" -ne 0 ]; then
    echo "witness FAIL: $ws_ctr: could not list /run/secrets in the running container (exit $ws_rc)" >&2
    return 1
  fi
  ws_out=${ws_out%x}
  ws_out=${ws_out%"$witness_nl"}
  if [ -z "$ws_out" ]; then
    echo "witness FAIL: $ws_ctr: the listing of /run/secrets is empty" >&2
    return 1
  fi

  # Sort each listed line into a good name or a bad entry. Read line by line; nothing here is ever left unquoted.
  ws_listed= ws_bad= ws_repeated=
  while IFS= read -r ws_line; do
    if witness_name_ok "$ws_line"; then
      if printf '%s' "$ws_listed" | LC_ALL=C grep -qxF -- "$ws_line"; then ws_repeated="$ws_repeated $ws_line"; fi
      ws_listed="$ws_listed$ws_line$witness_nl"
    else
      ws_bad="$ws_bad $(witness_cut "$ws_line")"
    fi
  done <<WITNESS_LISTING
$ws_out
WITNESS_LISTING

  ws_unexpected=$ws_bad ws_missing=
  while IFS= read -r ws_name; do
    [ -n "$ws_name" ] || continue
    printf '%s' "$ws_expected" | LC_ALL=C grep -qxF -- "$ws_name" || ws_unexpected="$ws_unexpected $ws_name"
  done <<WITNESS_LISTED
$ws_listed
WITNESS_LISTED
  while IFS= read -r ws_name; do
    [ -n "$ws_name" ] || continue
    printf '%s' "$ws_listed" | LC_ALL=C grep -qxF -- "$ws_name" || ws_missing="$ws_missing $ws_name"
  done <<WITNESS_EXPECTED
$ws_expected
WITNESS_EXPECTED

  if [ -z "$ws_unexpected" ] && [ -z "$ws_missing" ] && [ -z "$ws_repeated" ]; then
    echo "witness: $ws_ctr holds exactly:$(printf '%s' "$ws_expected" | tr '\n' ' ' | sed 's/ $//; s/^/ /')"
    return 0
  fi
  ws_msg="witness FAIL: $ws_ctr:"
  [ -z "$ws_unexpected" ] || ws_msg="$ws_msg unexpected:$ws_unexpected;"
  [ -z "$ws_missing" ] || ws_msg="$ws_msg missing:$ws_missing;"
  [ -z "$ws_repeated" ] || ws_msg="$ws_msg repeated:$ws_repeated;"
  echo "$ws_msg" >&2
  return 1
}

witness_container() {
  wc_ctr=${1-}
  if ! wc_exp=$(witness_expected "$wc_ctr"); then
    echo "witness FAIL: no expected secret set is defined for the container $(witness_cut "$wc_ctr")" >&2
    return 1
  fi
  # The allowed names are constants of this file (letters, digits, dashes): splitting them on newlines is safe and is the
  # only place a value is split. It is done in a subshell, so the caller's IFS (unset, or set to anything) is never touched
  # and nothing here reads IFS (hale's finding F-3).
  # shellcheck disable=SC2086
  ( IFS=$witness_nl; set -- $wc_exp; unset IFS; witness_secrets "$wc_ctr" "$@" )
}
