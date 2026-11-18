#!/bin/sh
# Cortex v2 skeleton installer (S5). Rootless Podman, one command, safe to run twice. Run it with sh (sh install.sh), not bash install.sh.
# CORTEX_SCRIPTS_DIR names the backup scripts of a source CHECKOUT run only (a release uses its own bin/).
#   deploy/install.sh --project <name> --lead <worker> [--home DIR] [--port N] [--no-start] [--no-show-credentials]
#   deploy/install.sh --project <name> --lead <worker> [--home DIR] --reissue-token
# THE TRUST ANCHOR of THIS installer (hue's page TRUST-ANCHOR-LITERALS-hue-r1.md, W1-84): the three statements below are EMPTY in the tree (the development layout: a release run refuses before anything is
# fetched, fail closed) and the join step of the release build fills each with its value in single quotes: the principal, the public ed25519 key (68 characters) and its fingerprint (SHA256: and 43 characters).
# They stay the FIRST three statements, whole lines, each exactly once, in this order. Nothing of the environment is ever the anchor.
CORTEX_ANCHOR_PRINCIPAL=''
CORTEX_ANCHOR_KEY=''
CORTEX_ANCHOR_FPR=''
set -eu
#
# What it does: checks the machine, creates three Podman secrets with random values (never in a file, argv or
# the terminal), copies the config example to $HOME_DIR/cortex.toml, builds and starts the two containers on the
# cv2-s5-* names, waits (bounded) for both to be healthy, and runs `cortex-server bootstrap` once to mint the
# first lead token (written to the container's tmpfs, piped out into a 0600 file, then removed). It never prints a
# random secret; a newly minted token is shown once on /dev/tty unless suppressed.
#
# --reissue-token (kai 2a7de258): re-mints the lead's token into a NEW 0600 file, $HOME_DIR/<lead>.token.<UTC stamp>; the old
# token is superseded, the old file is left alone, only the new path is printed. Starts nothing, creates no secret: the stack
# must be healthy. ANY existing project (one run per project, exact names); it NEVER CREATES: --require-existing makes the
# server refuse (exit 3, no file) for a missing project or lead, or a lead that is not that project's (contract r0.3).
# INTERIM(ponytail): a script around compose is the 0.2.001 bridge; the package install (P-6) replaces it in 0.2.003.
# hale RF-1 (cox c3e157a3): bash IMPORTS EXPORTED FUNCTIONS (also under sh and --posix): an exported [ or printf or command would shadow the builtin every check relies on. So the first statements below
# remove every function the environment brought, before anything is defined or sourced (globbing off: a name is not a pattern; compgen itself first, an exported one could hide the rest; a shell
# without compgen has no imported functions to remove).
set -f
unset -f compgen 2>/dev/null || :
for f in $(compgen -A function 2>/dev/null); do unset -f "$f"; done
set +f
case $- in *x*) install_trace_requested=1 ;; *) install_trace_requested=0 ;; esac
set +x
umask 077   # AFTER the removal (hale 6E-2): an exported umask function would have been what ran here before

project="" lead="" home_dir="${HOME:-/nonexistent}/cortex-v2" port="8601" start=1 reissue=0
show_credentials=1 summary_new_token=0 summary_complete=0 summary_active=0 summary_defer_path=0
summary_steps='' summary_phase='Checked this machine' summary_repeat=0
summary_no_token_reported=0
token='' summary_mint_attempt=0 summary_mint_part_cleanup_attempted=1
summary_mint_part='' summary_server_mint_ran=0
mint_reservation_dir='' mint_reservation_inode='' mint_reservation_owned=0
exit_handler_active=0 exit_handler_signal=''
summary_token_path_emitted=0 summary_mint_destination='' summary_mint_file_id=''
usage() { sed -n '2,5p' "$0" | sed 's/^# \{0,1\}//'; }

# ---- the interface of the install lanes (INSTALL-LANES-INTERFACE-hue-r0.md and r1): exit-code symbols, say, warn, fail, the message catalogue, the joined files ----
EX_OK=0 EX_USAGE=1 EX_MACHINE=2 EX_PREREQ=3 EX_TRUST=4 EX_FETCH=5 EX_START=6 EX_WORKER=7 EX_BACKUP=8
CORTEX_LOG_REQUESTED=${CORTEX_LOG:-}
CORTEX_YES=0 CORTEX_LOG='' check_only=0 CORTEX_RELEASE="" CORTEX_BASE_URL="" CORTEX_FROM=""
release_given=0 base_given=0 from_given=0
# The trust anchor is the three CORTEX_ANCHOR_* statements at the very top of this file (W1-84, vera V6E-03, hue r1): anchor_ok checks them, stage_make writes the key line from them.
log_line() { [ -z "$CORTEX_LOG" ] || printf '%s\n' "$1" >> "$CORTEX_LOG" || :; }
say() { say_rc=0; printf '%s\n' "$*" || say_rc=$?; log_line "$*"; return "$say_rc"; }   # the status of the PRINT, not of the log (marv F-7: a plan that could not be shown must not be consented to)
warn() { printf 'install: warning: %s\n' "$*" >&2; log_line "warning: $*"; }
fail() { fail_code=$1; shift; summary_abort; printf 'install: %s\n' "$*" >&2; log_line "install: $*"; { exit_handler_active=1; } 2>/dev/null; exit "$fail_code"; }
die() { summary_abort; [ -z "${die_id:-}" ] || msg "$die_id" "${die_arg:-}" || :; fail "${die_code:-$EX_USAGE}" "$@"; }   # die_code, die_id and die_arg are set by stop (or on the line before a probed class line)
# msg ID [ARG]: the cause and the fix of one problem, from the lane that owns the id (prereq_msg, fetch_msg, then this file's inst_msg); 1 for an id nobody knows
msg() {
  for msg_table in prereq_msg fetch_msg inst_msg; do
    if command -v "$msg_table" >/dev/null 2>&1 && "$msg_table" "$@"; then return 0; fi
  done
  return 1
}
# this file's table: a "problem:" line and a "fix:" line on stderr; no secret, no command that deletes (IT-87 checks every id)
inst_msg() {
  m_again="then run this installer again: it continues where it stopped"
  case $1 in
    bad_name) m_cause="the ${2:-project} name is not allowed: it must start with a small letter, then only small letters, digits and hyphens, 32 characters at most"; m_fix="give --${2:-project} a name such as cortex-1 (a small letter first, then small letters, digits or hyphens, 32 characters at most) and run this installer again" ;;
    not_root) m_cause="this installer runs as your own user, not as root: the stack is rootless Podman"; m_fix="run it again as a normal user" ;;
    podman_missing) m_cause="podman is not installed"; m_fix="install podman 5 or newer from your distribution, then run this installer again" ;;
    podman_too_old) m_cause="the installed podman is older than 5"; m_fix="update podman to version 5 or newer, then run this installer again" ;;
    no_cgroup2) m_cause="this machine does not use cgroup v2"; m_fix="boot with cgroup v2 (the default on current distributions), then run this installer again" ;;
    compose_not_found) m_cause="no compose provider found (podman-compose 1.5.0 or newer is required, and it is not bundled)"; m_fix="install podman-compose 1.5.0 or newer from your distribution, then run this installer again" ;;
    compose_unconfirmed) m_cause="the compose provider could not be confirmed as podman-compose 1.5.0 or newer"; m_fix="check what podman compose version prints, and install podman-compose 1.5.0 or newer from your distribution" ;;
    stack_env_bad) m_cause="the release's stack.env cannot be used"; m_fix="fetch the release again; do not edit stack.env by hand" ;;
    compose_old) m_cause="the installed podman-compose is older than 1.5.0"; m_fix="update podman-compose to 1.5.0 or newer from your distribution, then run this installer again" ;;
    not_healthy) m_cause="the database or the server did not become healthy in time${2:+ ($2)}"; m_fix="read the logs with podman logs cv2-s5-server and podman logs cv2-s5-db, fix what they show, $m_again" ;;
    token_left) m_cause="the first lead's sign-in file could not be made, and the partial file${2:+ $2} could not be removed: IT MAY STILL HOLD A VALID SIGN-IN TOKEN"; m_fix="remove ${2:-the partial file} by hand; if anyone else could have read it, replace the sign-in token: run this installer again with --reissue-token and the same --project and --lead" ;;
    token_failed) m_cause="the first lead's sign-in file could not be made${2:+ ($2)}; no sign-in file was written"; m_fix="read podman logs cv2-s5-server, $m_again" ;;
    token_stage_left) m_cause="the sign-in file was published, but its private stage could not be removed"; m_fix="A leftover copy of the token is still on disk: it is the file named below with .part.$$ added to its name. Remove ONLY that file, by hand, after checking that no installer is running." ;;
    mint_runtime_bad) m_cause="the account runtime directory is absent or is not a private directory of this user"; m_fix="start a login session as this user and ensure the systemd user runtime directory is owned by this user with mode 0700, then run this installer again" ;;
    bad_log_path) m_cause="CORTEX_LOG must be one absolute ASCII path using only letters, digits, . _ / and -"; m_fix="unset CORTEX_LOG or give an absolute path without spaces, controls, empty components or .. components, then run this installer again" ;;
    mint_busy) m_cause="another install or token reissue is running for this user account"; m_fix="wait for it to finish; if its reservation remains, check that no installer is running before removing that reservation directory" ;;
    mint_stale) m_cause="a mint reservation remains${2:+: $2}"; m_fix="remove that reservation directory only after checking that no installer is running; its public owner record is shown above" ;;
    mint_reservation_failed) m_cause="the private mint reservation could not be prepared"; m_fix="check the install home's ownership and permissions, then run this installer again" ;;
    token_not_private) m_cause="the existing token file is not private to its owner"; m_fix="run this installer with --reissue-token to create a new credential; the existing file was left unchanged" ;;
    token_exists) m_cause="a new sign-in file with this time stamp already exists"; m_fix="wait one second and run the same command again" ;;
    credential_marker_failed) m_cause="the credential display marker could not be validated or reserved for ${2:-the token file}; nothing was shown"; m_fix="read the private token file ${2:-named above}; use --reissue-token for a new token and a separate display marker" ;;
    project_lookup_failed) m_cause="the project identity lookup could not run or was interrupted before it completed"; m_fix="check podman info and podman logs cv2-s5-db, then run this installer again" ;;
    release_incomplete) m_cause="the release is missing a file the install needs${2:+: $2}"; m_fix="fetch the release again (do not copy files into it by hand), $m_again" ;;
    compose_unverified) m_cause="the prerequisite step did not verify a compose provider"; m_fix="read the lines above (the prerequisite step says what it could not confirm), then run this installer again" ;;
    compose_path_unsafe) m_cause="the compose provider's path is not safe to run${2:+ ($2)}"; m_fix="it must be one absolute path to a regular executable file that no one else can replace (no link, nothing writable by group or others on the way): reinstall podman-compose from your distribution, $m_again" ;;
    step_status) m_cause="a step ended with a status the installer does not know${2:+ ($2)}"; m_fix="$m_again; if it repeats, report the status and the install log" ;;
    provider_file) m_cause="the record of the compose provider is not a plain file${2:+: $2}"; m_fix="move it away yourself (the installer never follows or removes it), $m_again" ;;
    home_not_real) m_cause="the install home is not a real directory of yours${2:+: $2}"; m_fix="the installer never follows a link or changes a directory that is not yours: choose another --home or make a real directory yourself, $m_again" ;;
    trust_tool_missing) m_cause="a tool the release check needs is not installed${2:+: $2}"; case ${2:-} in ssh-keygen) tt_pkg=openssh ;; sha256sum) tt_pkg=coreutils ;; "") tt_pkg=the-package ;; *) tt_pkg=$2 ;; esac; m_fix="install it first (for example: sudo dnf install $tt_pkg), then run this installer again: it does not install these tools before the release is proven" ;;
    stage_parent) m_cause="there is no directory of yours to stage the release in${2:+ (near $2)}"; m_fix="give --home a path under a directory you own (a link, or a directory of someone else, is not used), then run this installer again" ;;
    port_differs) m_cause="the stack is already installed on another port${2:+ ($2)}"; m_fix="run this installer with the port the stack uses${2:+ (--port $2)}: it never reinstalls over a running stack" ;;
    anchor_unusable) m_cause="this installer carries no usable trust anchor to check a release against (an empty or damaged principal, key or fingerprint)"; m_fix="fetch the complete installer again from the release location (do not edit it)" ;;
    trust_missing) m_cause="this installer has no way to verify a release, so it will not unpack one"; m_fix="fetch the complete installer again from the release location (do not edit it)" ;;
    release_differs) m_cause="a different release is already installed in this home${2:+ ($2)}"; m_fix="use cortex upgrade to move between releases: this installer does not replace an installed release" ;;
    secret_unreadable) m_cause="a stored credential of the stack exists but could not be confirmed as non-empty${2:+ ($2)}"; m_fix="the installer never deletes or replaces one: look at what podman keeps for the stack, and if you are sure that one is wrong remove that one entry yourself, $m_again" ;;
    release_unverified) m_cause="the installed release could not be verified${2:+ ($2)}"; m_fix="fetch the release again with this installer (do not edit or copy files into it by hand): current must be the link to the verified releases directory, $m_again" ;;
    secret_create_failed) m_cause="a stored credential of the stack could not be created${2:+ ($2)}"; m_fix="read the message above and check that podman works for this user (podman info), $m_again" ;;
    bad_path) m_cause="PATH holds an empty or a relative element (a command could then be taken from the current directory)"; m_fix="run the installer with a PATH of absolute directories only, for example PATH=/usr/bin:/usr/sbin:/bin, $m_again" ;;
    bad_number) m_cause="${2:-a setting} is not a plain whole number of seconds"; m_fix="unset it or set it to a whole number (the poll from 1, the wait from 0, at most 99999, no sign, no fraction, no leading zero), $m_again" ;;
    no_random) m_cause="the machine did not give the random bytes the install needs for its credentials"; m_fix="check that /dev/urandom can be read and that head and od work, $m_again" ;;
    dir_symlink) m_cause="a directory the install needs is a symbolic link${2:+: $2}"; m_fix="move the link away and make a real directory (the installer never follows or removes it), $m_again" ;;
    cannot_write) m_cause="the install could not write${2:+ $2}"; m_fix="check the permissions and the free space there, $m_again" ;;
    config_modules) m_cause="the [modules] list in the install's cortex.toml is missing, empty, on several lines or holds an invalid id"; m_fix="edit cortex.toml in the install home so that [modules] has one line such as enabled = [\"records\"], $m_again" ;;
    compose_up_failed) m_cause="the container tool could not start the stack"; m_fix="read the compose output above and free what it names (a port or a name in use), $m_again; a volume is never deleted for you" ;;
    db_unreadable) m_cause="the database's access rules could not be read"; m_fix="read podman logs cv2-s5-db, $m_again" ;;
    db_trust) m_cause="the database accepts connections without asking for a login (it was made before the auth settings)"; m_fix="this refusal changed nothing in the database and the installer deletes no volume: read S5-PEER-AUTH-MAP.md for the explicit step, then run this installer again (that is safe)" ;;
    roles_failed) m_cause="the database roles for the modules could not be made"; m_fix="read the message above and podman logs cv2-s5-db, $m_again" ;;
    build_failed) m_cause="the server image could not be built"; m_fix="read the build output above and fix what it shows, $m_again" ;;
    migrate_failed) m_cause="the database migrations failed${2:+ ($2)}; the server was not started"; m_fix="read podman logs cv2-s5-migrate, $m_again" ;;
    migrate_failed_removed) m_cause="the database migrations failed${2:+ ($2)}; the server container that compose had started (cv2-s5-server, found by the compose project and directory this install made it with) was removed, so its port is free again"; m_fix="read podman logs cv2-s5-migrate and fix what it shows, $m_again; the database was left as it is" ;;
    migrate_failed_foreign) m_cause="the database migrations failed${2:+ ($2)}; a container named cv2-s5-server exists that this installer did not make (it does not carry the label cortexv2=dev together with this install's compose project and directory), so it was left alone"; m_fix="look at it with podman ps -a and, if it is not yours to keep, remove or rename it yourself (this installer never removes a container it did not make), read podman logs cv2-s5-migrate, $m_again" ;;
    migrate_failed_stuck) m_cause="the database migrations failed${2:+ ($2)}; the server container compose may have started (cv2-s5-server) could not be checked or removed (inspected by name, removed by its id), so it may still be running"; m_fix="look at it with podman ps -a and stop and remove cv2-s5-server yourself, read podman logs cv2-s5-migrate, $m_again" ;;
    units_failed) m_cause="the backup unit files could not be written${2:+ ($2)}"; m_fix="check the permissions of ~/.config/systemd/user, $m_again" ;;
    restart_unit_failed) m_cause="the stack is running but a restart after a reboot could not be confirmed: the distro's user unit podman-restart.service could not be established as enabled${2:+ (systemd reports: $2)}"; m_fix="run this installer from a login session of your own user (ssh, or sudo -iu USER) so that systemctl --user can reach your user manager (XDG_RUNTIME_DIR must be set), turn lingering on (loginctl enable-linger \$USER), check that podman-restart.service exists (it comes with podman), $m_again" ;;
    timers_failed) m_cause="the stack is running but the backups are NOT scheduled"; m_fix="turn lingering on for this user (loginctl enable-linger \$USER) and check that systemctl --user works, $m_again" ;;
    summary_failed) m_cause="Cortex is installed but NOT working: an end-of-install check failed"; m_fix="check the FAILED result above, read podman logs cv2-s5-server and podman logs cv2-s5-db, $m_again" ;;
    *) return 1 ;;
  esac
  printf 'problem: %s\n' "$m_cause" >&2
  printf 'fix: %s\n' "$m_fix" >&2
}
# stop CODE ID ARG SENTENCE...: the catalogue's cause and fix for ID (ARG is an optional detail), then "install: SENTENCE" and the exit CODE (6e: every failure after the
# arguments uses it, so the code, the cause and the fix are decided in one place). The lines that scripts/probe-classes.sh takes out of this file VERBATIM keep a plain
# die call (the probe's stub knows only die): the code and id are set on the line before them.
stop() { die_code=$1; die_id=$2; die_arg=$3; shift 3; die "$@"; }

# D181: only measured receipts enter the summary. All ordinary lines are ASCII,
# wrapped to 78 columns. Credential bytes never pass through this formatter or say.
summary_line() {
  if command -v tr >/dev/null 2>&1 && command -v awk >/dev/null 2>&1 && command -v fold >/dev/null 2>&1; then
    printf '%s\n' "$*" | LC_ALL=C tr -c '\12\40-\176' '?' | LC_ALL=C awk '
      function compact(t, p,n,b) {
        if (match(t,/sha256:[0123456789abcdef]+/)) {
          p=RSTART; n=RLENGTH; b=substr(t,p+7,n-7)
          if (length(b)==64) t=substr(t,1,p-1) "sha256:" substr(b,1,12) "..." substr(t,p+n)
        }
        if (match(t,/SHA256:[ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+\/=]+/)) {
          p=RSTART; n=RLENGTH; b=substr(t,p+7,n-7)
          if (length(b)==43) t=substr(t,1,p-1) "SHA256:" substr(b,1,4) "..." substr(b,length(b)-1) substr(t,p+n)
        }
        return t
      }
      {
        first=match($0,/[^ ]/); lead=substr($0,1,first-1); continuation=lead
        if (match($0,/ \.\.\. (ok|FAILED|skipped) \(/)) continuation=sprintf("%*s",RSTART+RLENGTH-1,"")
        else if (match($0,/ \.\.\. /)) continuation=sprintf("%*s",RSTART+RLENGTH-1,"")
        count=split(substr($0,first),words,/ +/); line=lead
        for (i=1;i<=count;i++) {
          word=compact(words[i]); room=78-length(continuation)
          if (length(word)>room) {
            if (match(word,/@sha256:/)) {
              suffix=substr(word,RSTART); repository=substr(word,1,RSTART-1)
              keep=room-length(suffix)-3
              if (keep>0) word="..." substr(repository,length(repository)-keep+1) suffix
            } else word=substr(word,1,room-3) "..."
          }
          separator=(line==lead || line==continuation ? "" : " ")
          if (length(line)+length(separator)+length(word)>78) { print line; line=continuation; separator="" }
          line=line separator word
        }
        print line
      }
    ' | LC_ALL=C fold -w 78
  else
    # Preserve the original refusal when prerequisites are not available yet.
    printf '%.78s\n' "$*"
  fi
}
summary_header_line() {
  summary_header_value=$(printf '%s\n' "$*" | LC_ALL=C awk '
    length($0)<=78 { print; next }
    {
      first=match($0,/[^ ]/); out=substr($0,1,first-1)
      count=split(substr($0,first),words,/ +/)
      for (i=1;i<=count;i++) {
        word=words[i]
        if (length(word)>16) word=substr(word,1,16) "..."
        out=out (i==1 ? "" : " ") word
      }
      print out
    }
  ')
  summary_line "$summary_header_value"
}
summary_system() {
  # os-release is data, never shell code. Only the two literal display values
  # are read; no environment variable can select another machine root.
  summary_os=$(LC_ALL=C awk -F= '
    /^NAME=/ || /^VERSION_ID=/ {
      key=$1; value=substr($0,index($0,"=")+1)
      quote=substr(value,1,1)
      if ((quote=="\042" || quote=="\047") && substr(value,length(value))==quote) value=substr(value,2,length(value)-2)
      if (key=="NAME") name=value; else version=value
    }
    END { if (name!="" && version!="") print name " " version; else exit 1 }
  ' /etc/os-release 2>/dev/null) || summary_os=unknown
  printf 'Linux, %s, %s\n' "$summary_os" "$(uname -m 2>/dev/null || printf unknown)"
}
# W1-296: validated public paths are standalone; credential values never enter here.
summary_path() {
  # Finish this small public pair before honoring a trapped exit. The emitted
  # flag and stdout must agree even when a signal arrives just after printf.
  summary_path_rc=0 summary_path_signal=''
  trap 'summary_path_signal=${summary_path_signal:-129}' HUP
  trap 'summary_path_signal=${summary_path_signal:-130}' INT
  trap 'summary_path_signal=${summary_path_signal:-143}' TERM
  if summary_line "$1"; then
  printf '%s\n' "$2"
    summary_path_rc=$?
  else
    summary_path_rc=$?
  fi
  if [ "$summary_path_rc" = 0 ] && [ "${3:-}" = token ]; then summary_token_path_emitted=1; fi
  summary_saved_signal=$summary_path_signal
  restore_deferred_signal "$summary_saved_signal"
  return "$summary_path_rc"
}
summary_token_mode() {
  summary_path_mode=$(stat -c %a "$token" 2>/dev/null || printf unavailable)
  case $summary_path_mode in [01234567][01234567][01234567]) summary_path_mode=0$summary_path_mode ;; esac
}
summary_token_path() {
  summary_token_mode
  summary_path "$1 ($summary_path_mode):" "$token" token
}
summary_record() {
  summary_value=$(printf '%s' "${3:-}" | LC_ALL=C awk '{ if (NR>1) printf ", "; printf "%s",$0 }')
  summary_steps="${summary_steps}${summary_steps:+
}   $1 ... $2${summary_value:+ ($summary_value)}"
}
summary_recover_published_token() {
  # A pending destination alone proves nothing. Match the inode of the private
  # staged file against the actual destination, including inside a moving tool.
  [ -n "$summary_mint_destination" ] && [ -n "$summary_mint_file_id" ] || return 1
  token_file_owned "$summary_mint_destination" || return 1
  [ "$(stat -c '%d:%i' "$summary_mint_destination" 2>/dev/null)" = "$summary_mint_file_id" ] || return 1
  token=$summary_mint_destination
}
summary_abort() {
  [ "$summary_active" = 1 ] || return 0
  [ "$summary_token_path_emitted" = 1 ] && return 0
  [ "$summary_no_token_reported" = 1 ] && return 0
  summary_mint_published=0
  summary_recover_published_token && summary_mint_published=1
  mint_abort_cleanup || :
  if [ "$summary_complete" != 1 ]; then
    summary_complete=1
    summary_line ' What was done' || :
    [ -z "$summary_steps" ] || summary_line "$summary_steps" || :
    summary_line "   $summary_phase ... FAILED" || :
  fi
  if [ "$summary_mint_published" = 1 ]; then
    summary_token_path '   Token file' || :
    return 0
  fi
  summary_no_token_reported=1
  if [ "$summary_server_mint_ran" = 1 ]; then
    summary_line ' No new token file was published; the server may already have replaced the old token: run --reissue-token again.' || :
  else
    summary_line ' No new token was made; your existing token files are unchanged.' || :
  fi
}

# File discovery never reads a credential. New reissues sort before the base file;
# whoami, not a filename or mtime, decides which credential is still active.
token_file_owned() {
  [ -f "$1" ] && [ ! -L "$1" ] && [ "$(stat -c %u "$1" 2>/dev/null)" = "$(id -u)" ]
}
token_private() {
  token_file_owned "$1" && [ -s "$1" ] || return 1
  tp_mode=$(stat -c %a "$1" 2>/dev/null) || return 1
  case $tp_mode in ''|*[!01234567]*) return 1 ;; esac
  [ $((0$tp_mode & 077)) -eq 0 ]
}
token_candidates() {
  for tc_file in "$home_dir/$lead.token" "$home_dir/$lead.token."*; do
    token_file_owned "$tc_file" || continue
    if [ "$tc_file" != "$home_dir/$lead.token" ]; then
      tc_suffix=${tc_file#"$home_dir/$lead.token."}
      case $tc_suffix in ????????T??????Z) ;; *) continue ;; esac
      tc_date=${tc_suffix%T*}; tc_clock=${tc_suffix#*T}; tc_clock=${tc_clock%Z}
      case $tc_date$tc_clock in *[!0123456789]*) continue ;; esac
    fi
    printf '%s\n' "$tc_file"
  done | LC_ALL=C sort -r
}
select_token() {
  case $- in *x*) install_trace_requested=1 ;; esac
  set +x
  token="$home_dir/$lead.token"; st_first=''
  st_candidates=$(token_candidates)
  [ -n "$st_candidates" ] || return 1
  st_project_ok=0
  summary_project_lookup && st_project_ok=1
  while IFS= read -r st_candidate; do
    [ -n "$st_candidate" ] || continue
    [ -n "$st_first" ] || st_first=$st_candidate
    token=$st_candidate
    if [ "$st_project_ok" = 1 ] && summary_whoami; then return 0; fi
  done <<TOKEN_CANDIDATES
$st_candidates
TOKEN_CANDIDATES
  token=${st_first:-"$home_dir/$lead.token"}
  return 1
}

# Bootstrap has no success body. Resolve the requested name ourselves, using the
# same identity join as find_project, never whoami's untrusted project field.
# Names were validated before any call; the local lookup is read-only and bounded.
summary_project_lookup() {
  summary_project_id=''
  sp_rc=0
  sp_id=$("/usr/bin/timeout" --kill-after=1s 5s "$PODMAN_BIN" exec --user postgres --env PGCONNECT_TIMEOUT=5 \
    --env 'PGOPTIONS=-c statement_timeout=5000 -c default_transaction_read_only=on' \
    cv2-s5-db psql -X -v ON_ERROR_STOP=1 -U postgres -d cortex -tAc \
    "SELECT p.id::text FROM identity.projects p JOIN identity.names n ON n.entity_kind = 'project' AND n.entity_id = p.id AND n.current WHERE n.name_key = '$project'" 2>/dev/null) || sp_rc=$?
  case $sp_rc in
    0) ;;
    # psql's own documented error statuses retain the unfinished-install path.
    # Timeout/exec failures and every signal status are unknown, never repairable.
    1|2|3) return 1 ;;
    *) summary_phase='Project identity lookup'; stop "$EX_START" project_lookup_failed '' 'project identity lookup did not complete; the stack was left as it is' ;;
  esac
  printf '%s' "$sp_id" | python3 -I -c '
import re, sys
sys.exit(0 if re.fullmatch(r"[0123456789abcdef]{8}-[0123456789abcdef]{4}-[0123456789abcdef]{4}-[0123456789abcdef]{4}-[0123456789abcdef]{12}", sys.stdin.read(80)) else 1)
' || return 1
  summary_project_id=$sp_id
}

# No response body is printed or saved. curl's config (including Authorization)
# arrives on stdin, not argv; curl reads no user config, follows no redirects,
# uses no proxy, and each call has a deadline and a response-size bound.
summary_peer_verified() {
  summary_peer_state=$("$PODMAN_BIN" inspect --format '{{.State.Status}} {{.State.Health.Status}}' cv2-s5-server 2>/dev/null) || return 1
  [ "$summary_peer_state" = 'running healthy' ] || return 1
  summary_peer_port=$("$PODMAN_BIN" port cv2-s5-server 8601/tcp 2>/dev/null) || return 1
  [ "$summary_peer_port" = "127.0.0.1:$port" ]
}
summary_whoami() (
  set +x
  unset summary_secret summary_reply
  [ -n "${summary_project_id:-}" ] || exit 1
  token_private "$token" || exit 1
  [ "$(wc -c < "$token")" -le 1024 ] || exit 1
  summary_secret=''
  IFS= read -r summary_secret < "$token" || [ -n "$summary_secret" ] || exit 1
  case $summary_secret in ''|*[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._-]*) exit 1 ;; esac
  summary_reply=$(
    summary_peer_verified || exit 1
    printf 'header = "Authorization: Bearer %s"\n' "$summary_secret" |
      curl -q --silent --noproxy '*' --max-time 5 --max-filesize 65536 \
        --config - --write-out '\n%{http_code}' "http://127.0.0.1:$port/v2/whoami" 2>/dev/null
  ) || exit 1
  unset summary_secret
  printf '%s' "$summary_reply" | python3 -I -c '
import json, sys
try:
    body, status = sys.stdin.read(65540).rsplit("\n", 1)
    def unique(pairs):
        d = {}
        for k, v in pairs:
            if k in d: raise ValueError()
            d[k] = v
        return d
    d = json.loads(body, object_pairs_hook=unique)
    ok = status == "200" and isinstance(d, dict) and d.get("name") == sys.argv[1] and d.get("kind") == "worker" and d.get("role") == "project_lead" and d.get("project") == sys.argv[2]
except (ValueError, TypeError, AttributeError):
    ok = False
sys.exit(0 if ok else 1)
' "$lead" "$summary_project_id" 2>/dev/null
)
# Validate even when display is suppressed. Reservation and display are separate:
# the durable marker is committed BEFORE success output and BEFORE any tty write.
summary_credential_check() {
  set +x
  token_private "$token" || return 1
  [ "$(wc -c < "$token")" -le 1024 ] || return 1
  unset summary_secret summary_id
  summary_secret=''
  IFS= read -r summary_secret < "$token" || [ -n "$summary_secret" ] || return 1
  case $summary_secret in ''|*[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._-]*) return 1 ;; esac
  summary_id=${summary_secret#ctx2_}; summary_id=${summary_id%%.*}
  case $summary_id in *[!0123456789abcdef]*) summary_id=unknown ;; esac
  [ "${#summary_id}" = 32 ] || summary_id=unknown
  summary_marker="$token.shown"
  summary_seen=0
  summary_published=0
  if [ -e "$summary_marker" ] || [ -L "$summary_marker" ]; then
    token_file_owned "$summary_marker" || return 1
    [ "$(stat -c %a "$summary_marker" 2>/dev/null)" = 600 ] || return 1
    python3 -I -c '
import datetime, pathlib, re, sys
try:
    data = pathlib.Path(sys.argv[1]).read_bytes() if pathlib.Path(sys.argv[1]).stat().st_size <= 128 else b""
    match = re.fullmatch(rb"id=" + sys.argv[2].encode() + rb"\nshown_at=([0123456789]{8}T[0123456789]{6}Z)\n", data)
    if not match: raise ValueError()
    datetime.datetime.strptime(match[1].decode(), "%Y%m%dT%H%M%SZ")
except (OSError, ValueError):
    sys.exit(1)
' "$summary_marker" "$summary_id" || return 1
    summary_seen=1
  fi
}
summary_credential_reserve() {
  sm_now=$(date -u +%Y%m%dT%H%M%SZ) || return 1
  sm_tmp=$(mktemp "$summary_marker.XXXXXX") || return 1
  if ! { printf 'id=%s\nshown_at=%s\n' "$summary_id" "$sm_now" > "$sm_tmp" &&
    chmod 0600 "$sm_tmp"; }; then
    rm -f -- "$sm_tmp" || :
    return 1
  fi
  # -T treats the marker as one name even if a directory appeared meanwhile.
  # ln never replaces that name. Only the process that linked it may display.
  if ln -T -- "$sm_tmp" "$summary_marker"; then
    summary_published=1
  elif [ -e "$summary_marker" ] || [ -L "$summary_marker" ]; then
    summary_seen=1
  else
    rm -f -- "$sm_tmp" || :
    return 1
  fi
  rm -f -- "$sm_tmp" || return 1
}
summary_with_tty() {
  # fd 3 was opened on /dev/tty by the caller, before reserving the display.
  summary_credential_reserve || return 7
  if [ "$summary_published" != 1 ]; then
    unset summary_secret
    summary_display_reason=" Read the private file above; --reissue-token shows a new token once."
    summary_success
    return 0
  fi
  summary_token_mode
  summary_defer_path=1
  summary_success || return 7
  if ! {
    printf '%s\n' ' Credential: SHOWN ONCE' \
      " This is the lead's sign-in token. It is a secret. Store it now." \
      " It will not be shown again. It is also in the file named below ($summary_path_mode)." \
      ' Use --no-show-credentials for recorded sessions.' \
      ' Copy the WHOLE next line (one line, no spaces):' >&3 &&
    printf '%s\n' "$summary_secret" >&3;  # s5-never: ok D181 r3 and Cox 4490c825: fd 3 opened only on /dev/tty; private file, suppression, exclusive marker and SUM-4/6/10/12-18 guard this one write
  }; then
    summary_write_ok=0
  else
    summary_write_ok=1
  fi
  unset summary_secret
  summary_defer_path=0
  if [ "$summary_write_ok" = 1 ]; then
    summary_token_path '   Token file'
  else
    summary_token_path ' Credential could not be shown. It is in this file'
  fi
  return 0
}
backup_timers_enabled() {
  for bt_timer in cortex-dump.timer cortex-dump-status.timer; do
    bt_state=$(systemctl --user is-enabled "$bt_timer" 2>/dev/null) || return 1
    [ "$bt_state" = enabled ] || return 1
  done
}
summary_repeat_receipts() {
  summary_record 'Checked this machine' ok "already done; $(summary_system)"
  if { if command -v prereq_compose_path >/dev/null 2>&1; then
    [ -n "${COMPOSE_PROVIDER:-}" ] && compose_path_ok "$COMPOSE_PROVIDER" && compose_version_ok
  else compose_provider_ok; fi; }; then
    summary_record Prerequisites ok "already done; $(podman --version); podman-compose $COMPOSE_VERSION"
  else
    summary_record Prerequisites FAILED 'compose provider could not be confirmed'; summary_ok=0
  fi
  if [ -n "${inf_rel:-}" ]; then
    summary_record 'Release fetched and verified' ok "already done; $inf_rel; signer $CORTEX_ANCHOR_FPR"
    summary_record 'Images loaded' ok "already done; server $ss; database $sd"
  else
    summary_record 'Release fetched and verified' skipped 'source checkout'
    summary_record 'Images loaded' skipped 'source checkout builds the server image'
  fi
  if [ "$summary_running" = 2 ]; then sr_containers=ok; else sr_containers=FAILED; fi
  summary_record 'Containers started' "$sr_containers" "already done; $summary_running of 2 running, healthy"
  if [ "$summary_signin" != FAILED ]; then sr_identity=ok; else sr_identity=FAILED; fi
  summary_record 'Project and lead created' "$sr_identity" "already done; $summary_signin"
  if token_private "$token"; then
    summary_record 'Lead credential written' ok "already done; 0$(stat -c %a "$token"), path below"
  else
    summary_record 'Lead credential written' FAILED 'private token file below'; summary_ok=0
  fi
  if backup_timers_enabled; then
    summary_record Backups ok 'already done; both backup timers enabled'
  else
    summary_record Backups FAILED 'backup timers not confirmed enabled'; summary_ok=0
  fi
}
summary_legacy_done() {
  if [ "$summary_repeat" = 1 ]; then
    say 'install: already installed and healthy; nothing was changed.'
  fi
  summary_done_output=''
  if command -v say_done >/dev/null 2>&1; then
    # Keep the real completion API/receipts and its stderr usage errors. Its old
    # stdout embeds paths; the summary now owns their sole copyable rendering.
    summary_done_output=$(say_done "${inf_rel:-${CORTEX_VERSION:-unknown}}" "http://127.0.0.1:$port/v2/health/ready" "$token" "${CORTEX_LOG:-none}") || :
  fi
  summary_line "Installed Cortex version: ${inf_rel:-${CORTEX_VERSION:-unknown}}"
  summary_line "Endpoint: http://127.0.0.1:$port/v2/health/ready"
  # Use the real helper's returned log fact when it supplies one. This preserves
  # the joined API check: a wrong fourth argument cannot be hidden by our renderer.
  summary_done_log=$(printf '%s\n' "$summary_done_output" | LC_ALL=C sed -n 's/^Install log: //p')
  [ -n "$summary_done_log" ] || summary_done_log=${CORTEX_LOG:-none}
  if [ "$summary_done_log" != none ]; then
    summary_path 'Install log:' "$summary_done_log"
  else
    summary_line 'Install log: none'
  fi
}
summary_print() {
  summary_line '=============================================================================='
  if [ "$summary_ok" = 1 ]; then
    summary_header_line " Cortex ${CORTEX_VERSION:-${CORTEX_RELEASE:-checkout}} is installed and running"
  else
    summary_header_line " Cortex ${CORTEX_VERSION:-${CORTEX_RELEASE:-checkout}} is installed but NOT working"
  fi
  summary_header_system=$(summary_system)
  log_line "System: $summary_header_system"
  log_line "Installed Cortex version: ${CORTEX_VERSION:-${CORTEX_RELEASE:-checkout}}"
  summary_header_line " $summary_header_system"
  summary_line ' What was done'
  summary_line "$summary_steps"
  summary_line ' Test result'
  summary_line "   Health (/v2/health/ready) ... $summary_health"
  summary_line "   Containers ... $summary_containers"
  summary_line "   Sign-in (/v2/whoami) ... $summary_signin"
  if [ "$summary_defer_path" != 1 ] && { [ "$summary_ok" = 1 ] || summary_recover_published_token; }; then
    summary_token_path '   Token file'
  fi
  summary_complete=1
}
summary_success() {
  summary_print
  summary_legacy_done
  [ -z "$summary_display_reason" ] || summary_line "$summary_display_reason"
  summary_line ' Next'
  summary_line '   Sign in and make the first call: see the guide, section "First call".'
  summary_line '=============================================================================='
}
summary_finish() {
  summary_health=FAILED summary_containers=FAILED summary_signin=FAILED summary_ok=1
  summary_phase='Test result'
  summary_http=$(curl -q --silent --noproxy '*' --max-time 5 --max-filesize 65536 \
    --output /dev/null --write-out '%{http_code}' "http://127.0.0.1:$port/v2/health/ready" 2>/dev/null) || summary_http=''
  if [ "$summary_http" = 200 ]; then summary_health='200 ok'; else summary_ok=0; fi
  summary_running=0
  for summary_container in cv2-s5-db cv2-s5-server; do
    if [ "$(podman inspect --format '{{.State.Status}} {{.State.Health.Status}}' "$summary_container" 2>/dev/null)" = 'running healthy' ]; then
      summary_running=$((summary_running + 1))
    fi
  done
  if [ "$summary_running" = 2 ]; then summary_containers='2 of 2 running, healthy'; else summary_ok=0; fi
  if [ "$summary_running" = 2 ] && summary_project_lookup && summary_whoami; then summary_signin="200 ok: $lead, project_lead of $project"; else summary_ok=0; fi
  [ "$summary_repeat" != 1 ] || summary_repeat_receipts
  if [ "$summary_ok" != 1 ]; then
    summary_print
    stop "$EX_START" summary_failed '' 'installed but NOT working'
  fi
  summary_phase='Credential display'
  if ! summary_credential_check; then
    stop "$EX_WORKER" credential_marker_failed '' 'credential display marker is invalid; read the private token file'
  fi
  summary_display_reason=''
  if [ "$summary_seen" = 1 ]; then
    unset summary_secret
    summary_display_reason=" Read the private file above; --reissue-token shows a new token once."
  elif [ "$show_credentials" != 1 ]; then
    unset summary_secret
    summary_display_reason=" Credential not shown: --no-show-credentials. Read the private file above."
  else
    if summary_with_tty 2>/dev/null 3>/dev/tty; then return 0; else sm_rc=$?; fi
    if [ "$sm_rc" = 7 ]; then
      unset summary_secret
      stop "$EX_WORKER" credential_marker_failed '' 'credential display could not be reserved; read the private token file'
    fi
    unset summary_secret
    summary_display_reason=" Credential not shown: no terminal. Read the private token file above."
  fi
  summary_success
}
# hale 6E-4: a PATH with an empty or a relative element is refused BEFORE anything runs (an empty element means the current directory). The variables that make a program load or read something the caller
# chose (loader, Python, OpenSSL and its TLS certificate locations SSL_CERT_FILE and SSL_CERT_DIR, shell startup, and podman's remote and config variables: measured on podman 5.8.2, CONTAINER_HOST, CONTAINER_CONNECTION and CONTAINERS_CONF are honoured WITHOUT --remote) are
# unset for the whole run. podman is found ONCE, by absolute path, and every later call (this file's and the joined includes') goes to that path, even if a step changes PATH.
case ${PATH:-} in ""|:*|*:|*::*|[!/]*|*:[!/]*) stop "$EX_USAGE" bad_path "" "PATH holds an empty or a relative element" ;; esac
unset LD_PRELOAD LD_LIBRARY_PATH LD_AUDIT PYTHONPATH PYTHONHOME PYTHONSTARTUP BASH_ENV ENV CDPATH OPENSSL_CONF OPENSSL_MODULES OPENSSL_ENGINES SSL_CERT_FILE SSL_CERT_DIR GCONV_PATH CURL_CA_BUNDLE SSLKEYLOGFILE CURL_SSL_BACKEND QLOGDIR CONTAINER_HOST CONTAINER_CONNECTION CONTAINERS_CONF CONTAINERS_STORAGE_CONF DOCKER_HOST REGISTRY_AUTH_FILE CONTAINERS_REGISTRIES_CONF
# hale O-2 (curl is started with -q, which shuts out its configuration file but not its environment): CURL_CA_BUNDLE (curl reads it as the --cacert value: the caller would choose what the release download trusts), SSLKEYLOGFILE (the file curl writes the session's TLS secrets to),
# QLOGDIR (the directory a curl built with HTTP/3 writes its logs to: this machine's curl has none, unset for the rule's sake) and CURL_SSL_BACKEND (the TLS backend of a curl built with more than one: measured no effect on this machine's curl, unset as a defence for another build)
# are unset for the whole run, with SSL_CERT_FILE and SSL_CERT_DIR in the list above. THE PROXY NAMES ARE NOT IN THIS LIST ON PURPOSE (kai, W1-232): only the release download runs without them (rel_fetch_noproxy), so that every later step keeps the caller's: podman pulls the database image of
# compose.yml through the operator's proxy, and nobody decided to change that (the image is pinned by digest: a proxy cannot change its bytes). The release download uses no proxy and only the system trust store; what it brings is checked against the signed list afterwards, but it does not go through a proxy the caller chose.
# rel_fetch_noproxy runs rel_fetch in THIS shell with the eight names taken out and puts the caller's back afterwards, each one exactly as it was (set or not, empty or not): a subshell would hold a TERM or HUP until the fetch ended, and IT-108's signal cases would fail.
rel_fetch_noproxy() {
  rfn_set=""
  for rfn_n in http_proxy https_proxy HTTP_PROXY HTTPS_PROXY all_proxy ALL_PROXY no_proxy NO_PROXY; do
    eval "if [ \"\${$rfn_n+x}\" = x ]; then rfn_v_$rfn_n=\$$rfn_n; rfn_set=\"\$rfn_set $rfn_n\"; fi"   # the names are the literals of the list above
  done
  unset http_proxy https_proxy HTTP_PROXY HTTPS_PROXY all_proxy ALL_PROXY no_proxy NO_PROXY
  rfn_rc=0; rel_fetch "$@" || rfn_rc=$?
  for rfn_n in $rfn_set; do eval "$rfn_n=\$rfn_v_$rfn_n; export $rfn_n"; done
  return "$rfn_rc"
}
SAFE_PATH=$PATH   # the PATH that was checked above: put back after every step of a joined lane (a lane may change it, and child processes such as db-roles.sh look podman up in PATH, not in this shell's functions)
PODMAN_BIN=""
pin_podman() {   # podman by absolute path, ONCE (again after the prerequisites on a machine that had none)
  [ -z "$PODMAN_BIN" ] || return 0
  PODMAN_BIN=$(command -v podman 2>/dev/null) || PODMAN_BIN=""
  case $PODMAN_BIN in /*) podman() { "$PODMAN_BIN" "$@"; } ;; *) PODMAN_BIN="" ;; esac
}
pin_podman
# THE OWN DIRECTORY BY THE SHELL ALONE (Q17d, vera's step 1b): no outside program runs before the included parts. here_dir is what dirname gives for $0: no slash is '.', the part before the last slash with the slashes at its end taken off ('x//y' is 'x', '/y' and '//y' are '/').
# cd and pwd are builtins; the supported Bash premise (compgen, cd, pwd) is the one of the exported-function removal above. The line that sets here keeps its start: the gate matrix replaces exactly that line.
case $0 in
  */*) here_dir=${0%/*}
       while case $here_dir in ?*/) true ;; *) false ;; esac; do here_dir=${here_dir%/}; done
       [ -n "$here_dir" ] || here_dir=/ ;;
  *) here_dir=. ;;
esac
here=$(CDPATH= cd -- "$here_dir" && pwd)
# the files of the other install lanes: the release builder replaces each of these two lines with the text of the file (curl | sh has no sibling to read)
[ ! -f "$here/install-prereq.sh" ] || . "$here/install-prereq.sh"   # @include
[ ! -f "$here/install-fetch.sh" ] || . "$here/install-fetch.sh"   # @include

log_path_bad=0
case $CORTEX_LOG_REQUESTED in
  '') ;;
  /*) case $CORTEX_LOG_REQUESTED in
    *[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._/-]*|*/|*//*|*/..|*/../*) log_path_bad=1 ;;
  esac ;;
  *) log_path_bad=1 ;;
esac
[ "$log_path_bad" = 0 ] || stop "$EX_USAGE" bad_log_path '' 'the install log path is invalid'
CORTEX_LOG=$CORTEX_LOG_REQUESTED
unset CORTEX_LOG_REQUESTED

while [ $# -gt 0 ]; do
  case $1 in
    --project) [ $# -ge 2 ] || die "--project needs a value"; project=$2; shift 2 ;;
    --lead)    [ $# -ge 2 ] || die "--lead needs a value"; lead=$2; shift 2 ;;
    --home)    [ $# -ge 2 ] || die "--home needs a value"; home_dir=$2; shift 2 ;;
    --port)    [ $# -ge 2 ] || die "--port needs a value"; port=$2; shift 2 ;;
    --no-start) start=0; shift ;;
    --no-show-credentials) show_credentials=0; shift ;;
    --yes) CORTEX_YES=1; shift ;;
    --check) check_only=1; shift ;;
    --release) [ $# -ge 2 ] || die "--release needs a value"; CORTEX_RELEASE=$2; release_given=1; shift 2 ;;
    --base-url) [ $# -ge 2 ] || die "--base-url needs a value"; CORTEX_BASE_URL=$2; base_given=1; shift 2 ;;
    --from) [ $# -ge 2 ] || die "--from needs a value"; CORTEX_FROM=$2; from_given=1; shift 2 ;;
    --reissue-token) reissue=1; shift ;;
    -h|--help) usage; { exit_handler_active=1; } 2>/dev/null; exit 0 ;;
    *) die "unknown argument: $1 (try --help)" ;;
  esac
done
[ "$check_only" -eq 1 ] || { [ -n "$project" ] && [ -n "$lead" ]; } || die "--project and --lead are required (try --help)"
case $port in *[!0123456789]*|"") die "--port must be a number" ;; esac
case $port in 8501|5499|5500|8765|8766) die "port $port belongs to the live v1 stack" ;; esac
case $home_dir in /*) ;; *) die "--home must be an absolute path" ;; esac
# The home is written into systemd units and passed through sed (piece 12): letters, digits and . _ / - only. A space, | & \ % $ or a quote or a newline would
# change a unit or the substitution. The class is spelled out (a range follows the locale).
case $home_dir in *[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._/-]*) die "--home may contain only letters, digits and . _ / - (it is written into systemd units)" ;; esac
# No empty component (a double slash, a trailing slash, the root alone) and no .. component (cox 5q; the engine's render of the same units uses the same rule).
case $home_dir in */|*//*|*/..|*/../*) die "--home may not hold an empty or a .. component (a double slash, a trailing slash or a ..)" ;; esac
# A worker or project name: the data pack's rule ^[a-z][a-z0-9-]{0,31}$ (hyphen, no underscore). $lead is used in a path.
valid_name() { case $1 in ""|[!abcdefghijklmnopqrstuvwxyz]*|*[!abcdefghijklmnopqrstuvwxyz0123456789-]*) return 1 ;; esac; [ "${#1}" -le 32 ]; }
[ "$check_only" -eq 1 ] || valid_name "$project" || stop "$EX_USAGE" bad_name project "--project must start with a small letter, then only small letters, digits and hyphens, 32 characters at most; use such a name"
[ "$check_only" -eq 1 ] || valid_name "$lead" || stop "$EX_USAGE" bad_name lead "--lead must start with a small letter, then only small letters, digits and hyphens, 32 characters at most; use such a name"
# valid_semver is the engine's parse_version: strict SemVer 2.0 WITHOUT build metadata, at most 64 bytes, not ending in .part; MAJOR.MINOR.PATCH with no leading zero
# (up to 19 digits: the engine allows u64), then an optional -PRERELEASE of dot-separated identifiers (letters, digits, hyphen; none empty; a numeric one has no leading
# zero). The alphabets are spelled out (a range follows the locale). 0 for a version, else 1.
valid_semver() {
  vs=$1
  [ "${#vs}" -le 64 ] || return 1
  case $vs in ""|*.part|*[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789.-]*) return 1 ;; esac
  vs_core=${vs%%-*}; vs_pre=""
  if [ "$vs_core" != "$vs" ]; then vs_pre=${vs#*-}; [ -n "$vs_pre" ] || return 1; fi
  case $vs_core in .*|*.|*..*) return 1 ;; esac
  vs_ifs=$IFS; IFS=.; set -- $vs_core; IFS=$vs_ifs
  [ "$#" -eq 3 ] || return 1
  for vs_n in "$1" "$2" "$3"; do
    case $vs_n in *[!0123456789]*|0?*) return 1 ;; esac   # (no empty field gets here: the dot checks above refuse it)
    [ "${#vs_n}" -le 19 ] || return 1
  done
  if [ -n "$vs_pre" ]; then
    case $vs_pre in .*|*.|*..*) return 1 ;; esac
    IFS=.; set -- $vs_pre; IFS=$vs_ifs
    for vs_n in "$@"; do
      case $vs_n in *[!0123456789]*|0) ;; 0*) return 1 ;; esac
    done
  fi
  return 0
}
# the arguments of the install lanes: validated here, before anything is looked at or started (the class lines spell the alphabets out: a range follows the locale)
if [ "$release_given" -eq 1 ]; then
  # the engine's rule and no looser one (hale P-2): the value goes into file names, argv and labels, and `<v>.part` is the engine's staging name
  valid_semver "$CORTEX_RELEASE" || die "--release must be a SemVer such as 0.2.1 or 0.2.1-rc.2"
fi
if [ "$base_given" -eq 1 ]; then
  case $CORTEX_BASE_URL in https://?*) ;; *) die "--base-url must start with https:// (plain http is refused)" ;; esac
  case $CORTEX_BASE_URL in *[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._~:/%@+-]*) die "--base-url may contain only letters, digits and . _ ~ : / % @ + - (no space, query or quote)" ;; esac
fi
# marv's rel_fetch reads FETCH_BASE_URL, this file holds CORTEX_BASE_URL (nix's G12 inventory, kai 5241a6a1): ONE place sets the one from the other, ALWAYS (empty when no
# --base-url was given), so an inherited FETCH_BASE_URL can never turn the online channel on without the checks above.
FETCH_BASE_URL=$CORTEX_BASE_URL
if [ "$from_given" -eq 1 ]; then
  case $CORTEX_FROM in /*) ;; *) die "--from must be an absolute path" ;; esac
  case $CORTEX_FROM in *[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._/-]*) die "--from may contain only letters, digits and . _ / -" ;; esac
  case $CORTEX_FROM in */|*//*|*/..|*/../*) die "--from may not hold an empty or a .. component" ;; esac
fi
# HOME is read by the units and by the machine look: absolute, checked HERE so that a bad one stops everything before anything is looked at or started (6e, G7: no half-install)
case ${HOME:-} in /*) ;; *) die "HOME must be an absolute path (the units go to \$HOME/.config/systemd/user)" ;; esac
# --check is the machine look of step 1 (the prerequisite file's machine_look): it changes nothing and returns its status
if [ "$check_only" -eq 1 ]; then
  command -v machine_look >/dev/null 2>&1 || die "--check is not available in this build (the machine look is not part of it)"
  machine_look   # bare (hale 6E3-1): errexit stays on inside it; its status ends the shell as it is
  { exit_handler_active=1; } 2>/dev/null; exit 0
fi

# Bounded waiting: a fresh start may need to build; a reissue only needs a stack that is already up.
if [ "$reissue" -eq 1 ]; then wait_max=${CORTEX_INSTALL_WAIT:-30}; else wait_max=${CORTEX_INSTALL_WAIT:-180}; fi
poll=${CORTEX_INSTALL_POLL:-5}
# Both reach shell arithmetic, and bash runs a command substitution that sits inside an arithmetic operand (a value such as HOME[$(...)]): each is a plain whole number or the
# install stops here, before anything is looked at or started (kai 5241a6a1, nix's G12 inventory). Digits only, no leading zero (octal), at most 5 digits; a poll of 0 would never end.
valid_secs() { case $1 in ""|*[!0123456789]*|0?*) return 1 ;; esac; [ "${#1}" -le 5 ]; }
valid_secs "$wait_max" || stop "$EX_USAGE" bad_number CORTEX_INSTALL_WAIT "CORTEX_INSTALL_WAIT must be a whole number of seconds from 0 to 99999"
{ valid_secs "$poll" && [ "$poll" != 0 ]; } || stop "$EX_USAGE" bad_number CORTEX_INSTALL_POLL "CORTEX_INSTALL_POLL must be a whole number of seconds from 1 to 99999"

# ---- preflight --------------------------------------------------------------------------------------------------
# podman_ok and compose_provider_ok are FUNCTIONS (the install lanes call them as the post-condition of the prerequisites): 0 when the check passes; else 1 with
# PREFLIGHT_ID (the catalogue id) and PREFLIGHT_WHY (today's fixed sentence) set. They never exit and never print.
PREFLIGHT_ID="" PREFLIGHT_WHY=""
podman_ok() {
  command -v podman >/dev/null 2>&1 || { PREFLIGHT_ID=podman_missing; PREFLIGHT_WHY="podman is not installed"; return 1; }
  pv=$(podman --version | sed 's/[^0123456789.]//g'); [ "${pv%%.*}" -ge 5 ] 2>/dev/null || { PREFLIGHT_ID=podman_too_old; PREFLIGHT_WHY="podman 5 or newer is required (found $pv)"; return 1; }
  return 0
}
# W1-68: the ONE way this file runs compose. podman is TOLD its provider by the environment variable PODMAN_COMPOSE_PROVIDER (podman 5: `podman compose --help`), the verified
# absolute path of COMPOSE_PROVIDER, so no call lets podman search for one by name. No other line may name `podman compose` as a command (IT-103).
COMPOSE_PROVIDER="" COMPOSE_FROM_PREREQ=0 COMPOSE_VERSION=""
compose_run() { PODMAN_COMPOSE_WARNING_LOGS=false PODMAN_COMPOSE_PROVIDER=$COMPOSE_PROVIDER podman compose "$@"; }
# compose_path_ok PATH: ONE absolute path to a regular executable file that no one else can replace: no link, the file and every directory above it (physical) not writable by group or
# others (the rule the engine applies to <home>/compose-provider, W1-68 item 4). 0 or 1; it never prints. The alphabets are spelled out (a range follows the locale).
compose_path_ok() {
  cp_path=$1
  case $cp_path in /*) ;; *) return 1 ;; esac
  case $cp_path in *[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._/+-]*|*//*|*/.|*/./*|*/..|*/../*) return 1 ;; esac
  [ -f "$cp_path" ] && [ -x "$cp_path" ] && [ ! -L "$cp_path" ] || return 1
  cp_cur=$(cd -P -- "${cp_path%/*}/" 2>/dev/null && pwd -P) || return 1
  cp_item=$cp_path
  while :; do
    cp_mode=$(stat -c %a -- "$cp_item" 2>/dev/null) || return 1
    case $cp_mode in ""|*[!01234567]*) return 1 ;; esac
    cp_mode=000$cp_mode; cp_mode=${cp_mode#"${cp_mode%???}"}
    cp_g=${cp_mode#?}; cp_g=${cp_g%?}; cp_o=${cp_mode#??}
    case $cp_g in 2|3|6|7) return 1 ;; esac
    case $cp_o in 2|3|6|7) return 1 ;; esac
    [ "$cp_cur" != / ] || break
    cp_item=$cp_cur; cp_cur=${cp_cur%/*}; [ -n "$cp_cur" ] || cp_cur=/
  done
  return 0
}
# compose_provider_find: sets COMPOSE_PROVIDER to the path compose is run with, or returns 1 with PREFLIGHT_ID and PREFLIGHT_WHY. With the prerequisite file the path is prereq_compose_path's ONE
# line (its step verified the provider; a failing or odd answer stops, fail closed) and COMPOSE_FROM_PREREQ is 1; a checkout without it finds podman-compose on PATH ONCE and verifies that path.
compose_provider_find() {
  COMPOSE_PROVIDER="" COMPOSE_FROM_PREREQ=0
  cf_nl='
'
  if command -v prereq_compose_path >/dev/null 2>&1; then
    cf_out=$(prereq_compose_path 2>/dev/null) || cf_out=""
    [ -n "$cf_out" ] || { PREFLIGHT_ID=compose_unverified; PREFLIGHT_WHY="the prerequisite step did not verify a compose provider"; return 1; }
    COMPOSE_FROM_PREREQ=1
  else
    cf_out=$(command -v podman-compose 2>/dev/null) || cf_out=""
    [ -n "$cf_out" ] || { PREFLIGHT_ID=compose_not_found; PREFLIGHT_WHY="no compose provider: podman-compose 1.5.0 or newer is required (it is not bundled)"; return 1; }
  fi
  case $cf_out in *"$cf_nl"*) cf_out="" ;; esac
  if [ -z "$cf_out" ] || ! compose_path_ok "$cf_out"; then
    PREFLIGHT_ID=compose_path_unsafe; PREFLIGHT_WHY="the compose provider's path is not safe to run (one absolute path to a regular executable that no one else can replace is required)"; return 1
  fi
  COMPOSE_PROVIDER=$cf_out
  return 0
}
# The compose provider (CONFLICTS-FOUND M-15a): podman-compose 1.5.0 or newer, the only one tested; it is not bundled. `podman compose`
# hands over to whichever provider it finds, so read what that provider says it is: the FIRST line of its stdout (after podman's own 'podman version N.N.N' line, when
# podman prints one: defect H, below) must be exactly "podman-compose version N.N" or "N.N.N" (digits only, at most four to a part). Nothing the provider prints is ever repeated.
# The character classes in the checks below are spelled out on purpose: a range such as [0-9] follows the locale's collation and, in a
# UTF-8 locale such as en_US.utf8, lets full-width and other-script digits through (hale F-1; tests IT-51, IT-52).
# The body runs in a subshell whose own `die` hands the sentence back instead of exiting the installer (the class lines stay verbatim: IT-64 probes them).
compose_version_ok() {
  PREFLIGHT_ID=compose_not_found
  PREFLIGHT_WHY=$(
    die() { printf '%s' "$*"; exit 1; }
    cv=$(compose_run version 2>/dev/null) || die "no compose provider: podman-compose 1.5.0 or newer is required (it is not bundled)"
    cant='could not confirm podman-compose 1.5.0 or newer from "podman compose version" (other providers are not tested)'
    # podman 5.8.2 prints ITS OWN version line first and the provider's second: 'podman version 5.8.2' then 'podman-compose version 1.5.0' (kai's first install on a fresh Rocky 10.2 VM, 9 Nov, defect H). At most ONE leading line that is exactly
    # 'podman version' and digits and dots (classes spelled out) is skipped; the rule below then reads the provider's line as before. A banner with other text, a second banner or a banner alone is not skipped, and the install refuses.
    cv_first=$(printf '%s\n' "$cv" | sed -n 1p)
    case $cv_first in
      "podman version "*)
        cv_pv=${cv_first#podman version }
        case $cv_pv in ""|*[!0123456789.]*|.*|*.|*..*) die "$cant" ;; esac
        [ "${#cv_pv}" -le 32 ] || die "$cant"
        cv=$(printf '%s\n' "$cv" | sed -n '2,$p') ;;
    esac
    cv=$(printf '%s\n' "$cv" | sed -n 1p)
    cvn=${cv#podman-compose version }
    [ "$cvn" != "$cv" ] || die "$cant"
    case $cvn in ""|*[!0123456789.]*|.*|*.|*..*) die "$cant" ;; esac
    case $cvn in *.*) ;; *) die "$cant" ;; esac
    cv_major=${cvn%%.*}; cv_rest=${cvn#*.}; cv_minor=${cv_rest%%.*}; cv_patch=0
    case $cv_rest in *.*) cv_patch=${cv_rest#*.} ;; esac
    case $cv_patch in *.*) die "$cant" ;; esac
    for cv_part in "$cv_major" "$cv_minor" "$cv_patch"; do [ "${#cv_part}" -le 4 ] || die "$cant"; done
    if [ "$cv_major" -lt 1 ] || { [ "$cv_major" -eq 1 ] && [ "$cv_minor" -lt 5 ]; }; then die "podman-compose $cvn is too old; 1.5.0 or newer is required"; fi
    printf '%s' "$cvn"
  ) && { COMPOSE_VERSION=$PREFLIGHT_WHY; PREFLIGHT_WHY=''; return 0; }
  case $PREFLIGHT_WHY in "no compose provider"*) ;; *"is too old"*) PREFLIGHT_ID=compose_old ;; *) PREFLIGHT_ID=compose_unconfirmed ;; esac
  return 1
}
compose_provider_ok() {
  compose_provider_find || return 1
  compose_version_ok
}
wait_healthy() {   # wait_healthy <container>...
  waited=0
  while :; do
    bad=""
    for c in "$@"; do
      st=$(podman inspect --format '{{.State.Health.Status}}' "$c" 2>/dev/null || echo missing)
      [ "$st" = "healthy" ] || bad="$bad $c=$st"
    done
    [ -z "$bad" ] && return 0
    [ "$waited" -lt "$wait_max" ] || stop "$EX_START" not_healthy "$bad" "not healthy after $wait_max s:$bad (see: podman logs cv2-s5-server)"
    sleep "$poll"; waited=$((waited + poll))
  done
}

# bootstrap_call: a first run CREATES the project and its lead; a reissue must not, so it passes the server's refuse-if-missing
# flag. Two literal command lines, on purpose: nothing a caller sets can change the flag, and there is no fallback to the
# creating form if the server does not know it. The server's own stderr line is left to reach the terminal.
bootstrap_call() {
  if [ "$reissue" -eq 1 ]; then
    podman exec cv2-s5-server /usr/local/bin/cortex-server bootstrap --project "$project" --lead "$lead" --token-file /run/cortex/bootstrap.token --require-existing
  else
    podman exec cv2-s5-server /usr/local/bin/cortex-server bootstrap --project "$project" --lead "$lead" --token-file /run/cortex/bootstrap.token
  fi
}

# Small metadata/cleanup operations finish their state before honoring a
# trapped signal. No credential bytes pass through this signal state.
mint_defer_signals() {
  mint_pending_signal=''
  trap 'mint_pending_signal=${mint_pending_signal:-129}' HUP
  trap 'mint_pending_signal=${mint_pending_signal:-130}' INT
  trap 'mint_pending_signal=${mint_pending_signal:-143}' TERM
}
mint_restore_signals() {
  mint_exit_signal=$mint_pending_signal
  restore_deferred_signal "$mint_exit_signal"
}
mint_process_started() {
  # Public process metadata binds a PID to its start, so PID reuse does not
  # turn a stale reservation into a claim that its installer is still running.
  python3 -I - "$1" <<'MINT_STARTED'
import datetime,os,pathlib,sys
try:
    fields=pathlib.Path('/proc',sys.argv[1],'stat').read_text().rsplit(')',1)[1].split()
    ticks=int(fields[19]); rate=os.sysconf('SC_CLK_TCK')
    boot=next(int(line.split()[1]) for line in pathlib.Path('/proc/stat').read_text().splitlines() if line.startswith('btime '))
    print(datetime.datetime.fromtimestamp(boot+ticks/rate,datetime.timezone.utc).strftime('%Y%m%dT%H%M%SZ'))
except (OSError,ValueError,IndexError,StopIteration):
    sys.exit(1)
MINT_STARTED
}
mint_reservation_record() {
  # A refused reservation is untrusted: never follow its owner record, read a
  # credential through it, print arbitrary bytes, or execute record contents.
  python3 -I - "$mint_reservation_dir/owner" <<'MINT_RECORD'
import os,pathlib,re,stat,sys
try:
    p=pathlib.Path(sys.argv[1]); st=p.lstat()
    if not stat.S_ISREG(st.st_mode) or st.st_uid!=os.getuid() or stat.S_IMODE(st.st_mode)!=0o600 or st.st_size>512: raise ValueError()
    lines=p.read_text(encoding="ascii").splitlines()
    if len(lines)!=4: raise ValueError()
    if not re.fullmatch(r"pid=[123456789][0123456789]{0,9}",lines[0]): raise ValueError()
    if not re.fullmatch(r"started_at=[0123456789]{8}T[0123456789]{6}Z",lines[1]): raise ValueError()
    if not re.fullmatch(r"host=[ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789.-]{1,253}",lines[2]): raise ValueError()
    if not re.fullmatch(r"home=/[ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._/-]+",lines[3]): raise ValueError()
    print("\n".join(lines))
except (OSError,UnicodeError,ValueError):
    sys.exit(1)
MINT_RECORD
}
mint_runtime_dir() {
  printf '/run/user/%s\n' "$(id -u)"
}
mint_reservation_prepare() {
  mkdir -m 0700 -- "$mint_reservation_dir" 2>/dev/null || return 2
  mint_reservation_owned=1
  mint_reservation_inode=$(stat -c '%d:%i' "$mint_reservation_dir" 2>/dev/null) || return 3
  [ "$(stat -c %u "$mint_reservation_dir" 2>/dev/null)" = "$(id -u)" ] &&
    [ "$(stat -c %a "$mint_reservation_dir" 2>/dev/null)" = 700 ] || return 3
  mint_started=$(mint_process_started "$$") || return 3
  mint_host=$(uname -n 2>/dev/null) || return 3
  case $mint_host in ''|*[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789.-]*) return 3 ;; esac
  printf 'pid=%s\nstarted_at=%s\nhost=%s\nhome=%s\n' "$$" "$mint_started" "$mint_host" "$home_dir" > "$mint_reservation_dir/owner" || return 3
  [ "$(stat -c %a "$mint_reservation_dir/owner" 2>/dev/null)" = 600 ] || return 3
}
mint_reservation_acquire() {
  mint_runtime=$(mint_runtime_dir) || stop "$EX_WORKER" mint_runtime_bad '' 'the account runtime directory could not be resolved'
  [ -d "$mint_runtime" ] && [ ! -L "$mint_runtime" ] &&
    [ "$(stat -c %u "$mint_runtime" 2>/dev/null)" = "$(id -u)" ] &&
    [ "$(stat -c %a "$mint_runtime" 2>/dev/null)" = 700 ] || stop "$EX_WORKER" mint_runtime_bad '' 'the account runtime directory is not private and owned by this user'
  mint_reservation_dir=$mint_runtime/cortex-mint.reservation
  mint_defer_signals
  mint_prepare_rc=0
  mint_reservation_prepare || mint_prepare_rc=$?
  mint_restore_signals
  case $mint_prepare_rc in
    0) return 0 ;;
    2)
      mint_record=''
      if [ -d "$mint_reservation_dir" ] && [ ! -L "$mint_reservation_dir" ] &&
        [ "$(stat -c %u "$mint_reservation_dir" 2>/dev/null)" = "$(id -u)" ] &&
        [ "$(stat -c %a "$mint_reservation_dir" 2>/dev/null)" = 700 ]; then
        mint_record=$(mint_reservation_record 2>/dev/null) || mint_record=''
      fi
      printf 'Mint reservation: %s\n' "$mint_reservation_dir" >&2
      if [ -n "$mint_record" ]; then
        printf '%s\n' "$mint_record" >&2
        mint_owner_pid=$(printf '%s\n' "$mint_record" | sed -n 's/^pid=//p')
        mint_owner_host=$(printf '%s\n' "$mint_record" | sed -n 's/^host=//p')
        mint_owner_started=$(printf '%s\n' "$mint_record" | sed -n 's/^started_at=//p')
        if [ "$mint_owner_host" = "$(uname -n 2>/dev/null)" ] && kill -0 "$mint_owner_pid" 2>/dev/null &&
          [ "$mint_owner_started" = "$(mint_process_started "$mint_owner_pid" 2>/dev/null)" ]; then
          stop "$EX_WORKER" mint_busy '' 'another install or token reissue is running for this user account'
        fi
      fi
      stop "$EX_WORKER" mint_stale "$mint_reservation_dir" 'the mint reservation needs checking; nothing was removed' ;;
    *) stop "$EX_WORKER" mint_reservation_failed '' 'the private mint reservation could not be prepared' ;;
  esac
}
mint_reservation_release() {
  [ "$mint_reservation_owned" = 1 ] || return 0
  if [ -d "$mint_reservation_dir" ] && [ ! -L "$mint_reservation_dir" ] &&
    [ "$(stat -c %u "$mint_reservation_dir" 2>/dev/null)" = "$(id -u)" ] &&
    [ "$(stat -c %a "$mint_reservation_dir" 2>/dev/null)" = 700 ] &&
    [ "$(stat -c '%d:%i' "$mint_reservation_dir" 2>/dev/null)" = "$mint_reservation_inode" ]; then
    if [ -e "$mint_reservation_dir/owner" ] || [ -L "$mint_reservation_dir/owner" ]; then
      [ -f "$mint_reservation_dir/owner" ] && [ ! -L "$mint_reservation_dir/owner" ] &&
        [ "$(stat -c %u "$mint_reservation_dir/owner" 2>/dev/null)" = "$(id -u)" ] &&
        [ "$(stat -c %a "$mint_reservation_dir/owner" 2>/dev/null)" = 600 ] &&
        rm -f -- "$mint_reservation_dir/owner" || { warn 'the owned mint reservation record could not be removed'; return 0; }
    fi
    rmdir -- "$mint_reservation_dir" || warn 'the owned mint reservation could not be removed; check it before another install'
  else
    warn 'the mint reservation no longer matches the owned directory; it was left alone'
  fi
  mint_reservation_owned=0
}
token_stage_drop() {
  mint_defer_signals
  mint_stage_rc=0
  rm -f "$part" || mint_stage_rc=$?
  summary_mint_part_cleanup_attempted=1
  if [ "$mint_stage_rc" != 0 ] && [ -n "$mint_pending_signal" ]; then
    warn "the private token stage $part could not be removed and may still hold a valid token; remove it by hand"
  fi
  mint_restore_signals
  return "$mint_stage_rc"
}
mint_abort_cleanup() {
  # A contender owns nothing and must not remove the winning mint's copies.
  [ "$mint_reservation_owned" = 1 ] && [ -n "$summary_mint_destination" ] || return 0
  [ "$summary_mint_cleanup_attempted" = 1 ] || token_copy_drop
  if [ "$summary_mint_part_cleanup_attempted" != 1 ]; then
    part=$summary_mint_part
    if [ ! -e "$part" ] && [ ! -L "$part" ]; then
      summary_mint_part_cleanup_attempted=1
    elif token_file_owned "$part" && { [ -z "$summary_mint_file_id" ] || [ "$(stat -c '%d:%i' "$part" 2>/dev/null)" = "$summary_mint_file_id" ]; }; then
      token_stage_drop || warn "the private token stage $part could not be removed and may still hold a valid token; remove it by hand"
    else
      warn 'the private token stage no longer matches this mint and was left alone'
    fi
  fi
}

# token_copy_drop: remove the temporary copy of the token in the server container (its tmpfs). A removal that FAILS is a WARNING that names it, never silent (cox ccd83501): the copy lasts only as long as the container.
token_copy_drop() {
  mint_defer_signals
  ( trap - EXIT HUP INT TERM; podman exec cv2-s5-server rm -f /run/cortex/bootstrap.token >/dev/null 2>&1 ) || warn "the temporary copy of the token in the server container could not be removed (it lasts only as long as the container)"
  summary_mint_cleanup_attempted=1
  mint_restore_signals
}
# mint_token <dest>: run bootstrap in the server, pipe the token from its tmpfs into <dest> (0600, via a .part file made
# under umask 077), remove the tmpfs copy. A failure leaves no file and no secret behind.
mint_token() {
  case $- in *x*) install_trace_requested=1 ;; esac
  set +x
  dest=$1; part="$dest.part.$$"
  summary_mint_attempt=1; token=''
  summary_mint_cleanup_attempted=1; summary_mint_part_cleanup_attempted=1
  mint_reservation_acquire
  [ ! -e "$dest" ] && [ ! -L "$dest" ] || stop "$EX_WORKER" token_exists '' 'a new sign-in file with this time stamp already exists; try again in a second'
  summary_mint_destination=$dest; summary_mint_file_id=''; summary_mint_part=$part
  rm -f "$part" || stop "$EX_WORKER" cannot_write "$part" "cannot remove the old $part"
  ( trap - EXIT HUP INT TERM; podman exec cv2-s5-server rm -f /run/cortex/bootstrap.token >/dev/null 2>&1 ) || true   # the PRE-CLEAN of a stale copy from a crashed run, masked ON PURPOSE (cox ccd83501): rm -f of nothing is success, and a container that cannot be reached fails the bootstrap call next with its own cause
  summary_mint_cleanup_attempted=0; summary_mint_part_cleanup_attempted=0
  reason=""
  summary_server_mint_ran=1
  if ! bootstrap_call; then
    reason="bootstrap failed"
  elif ! podman exec cv2-s5-server cat /run/cortex/bootstrap.token > "$part"; then
    reason="the token copy failed"
  elif [ ! -s "$part" ]; then
    reason="the token copy was empty"
  elif ! summary_mint_file_id=$(stat -c '%d:%i' "$part" 2>/dev/null) || [ -z "$summary_mint_file_id" ]; then
    reason="the private token file could not be confirmed"
  fi
  if [ -n "$reason" ]; then
    token_copy_drop   # cox ccd83501: FIRST, so a partial file that cannot be removed (the token_left stop just below) no longer leaves the container copy behind
    token_stage_drop || stop "$EX_WORKER" token_left "$part" "$reason; the partial file $part could not be removed (remove it by hand)"   # sam I6E-SAM01: a failing removal ends in exit 7 with its cause and a fix that NAMES the leftover
    stop "$EX_WORKER" token_failed "$reason" "$reason; no token file was written"
  fi
  chmod 600 "$part" || { token_copy_drop; token_stage_drop || stop "$EX_WORKER" token_left "$part" "the token file could not be made private; the partial file $part could not be removed (remove it by hand)"; stop "$EX_WORKER" token_failed "the file could not be made private" "the token file could not be made private; no token file was written"; }
  ln -T -- "$part" "$dest" 2>/dev/null || { token_copy_drop; token_stage_drop || stop "$EX_WORKER" token_left "$part" "the token file could not be moved into place; the partial file $part could not be removed (remove it by hand)"; stop "$EX_WORKER" token_failed "the file could not be moved into place" "the token file could not be moved into place; no token file was written"; }
  token_stage_drop || stop "$EX_WORKER" token_stage_left '' 'the published token is safe, but its private stage with .part appended could not be removed (remove it by hand)'
  # the temporary copy in the container's tmpfs: its removal failing after the token is published is a WARNING (it lives only as long as the container), never a lost install
  token_copy_drop
  token=$dest
}

# ---- the tree the install reads (6d): a RELEASE laid out by the fetch step (<home>/current holding stack.env: compose.yml, db-roles.sh, cortex.toml.example,
# bin/, systemd/ and the images named by stack.env), or this directory (a source checkout: the files beside this script, the image built here).
# read_stack_env follows the engine's parse_stack_env (stack_env.rs), rule for rule: at most 8192 bytes; blank lines (only spaces, tabs, CR, VT, FF) and #-comments (after
# leading ones of those) are skipped; any other line is KEY=VALUE with KEY one of CORTEX_VERSION, CORTEX_SERVER_IMAGE, CORTEX_DB_IMAGE, none twice, any order, and VALUE
# non-empty, at most 300 bytes, of letters, digits and . _ : / @ - (so a CR, a space or a + is refused); the version is valid_semver; the two images are name@sha256:<64
# lower hex> (the name is what is before the LAST @sha256:, not empty, no @). A file with a NUL byte ANYWHERE is refused (the engine refuses one in a line or a value; bash's read would drop the byte; one inside a comment is stricter than the engine: a known difference, fail closed). A last line without a newline is fine. 0 with CORTEX_VERSION, CORTEX_SERVER_IMAGE and
# CORTEX_DB_IMAGE's values in sv, ss and sd, or 1 with STACK_WHY (a fixed sentence and a line number, never the text of the line). Known limit: the engine's blank test is
# Unicode white space and it refuses a file that is not UTF-8; this accepts only the ASCII blanks above and does not look at the encoding of a comment.
read_stack_env() {
  STACK_WHY=""; sv=""; ss=""; sd=""; se_seen=""; se_n=0
  se_tab=$(printf '\t'); se_cr=$(printf '\r'); se_vt=$(printf '\v'); se_ff=$(printf '\f')
  [ "$(wc -c < "$1")" -le 8192 ] || { STACK_WHY="the file is larger than 8192 bytes"; return 1; }
  [ "$(tr -d '\000' < "$1" | wc -c)" -eq "$(wc -c < "$1")" ] || { STACK_WHY="the file holds a NUL byte"; return 1; }   # bash read drops a NUL byte; the engine refuses it (hale P-1)
  while IFS= read -r se_line || [ -n "$se_line" ]; do
    se_n=$((se_n + 1)); se_t=$se_line
    while :; do case $se_t in " "*|"$se_tab"*|"$se_cr"*|"$se_vt"*|"$se_ff"*) se_t=${se_t#?} ;; *) break ;; esac; done
    case $se_t in "") continue ;; "#"*) continue ;; esac
    se_key=${se_line%%=*}
    [ "$se_key" != "$se_line" ] || { STACK_WHY="line $se_n is not KEY=VALUE, a comment or blank"; return 1; }
    case $se_key in CORTEX_VERSION|CORTEX_SERVER_IMAGE|CORTEX_DB_IMAGE) ;; *) STACK_WHY="line $se_n names a key this file may not hold"; return 1 ;; esac
    case " $se_seen " in *" $se_key "*) STACK_WHY="line $se_n repeats a key"; return 1 ;; esac
    se_seen="$se_seen $se_key"; se_val=${se_line#*=}
    { [ -n "$se_val" ] && [ "${#se_val}" -le 300 ]; } || { STACK_WHY="line $se_n has an empty or too long value"; return 1; }
    case $se_val in *[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._:/@-]*) STACK_WHY="line $se_n has a value with a character outside the allowed ones"; return 1 ;; esac
    case $se_key in CORTEX_VERSION) sv=$se_val ;; CORTEX_SERVER_IMAGE) ss=$se_val ;; CORTEX_DB_IMAGE) sd=$se_val ;; esac
  done < "$1"
  [ -n "$sv" ] || { STACK_WHY="CORTEX_VERSION is missing"; return 1; }
  [ -n "$ss" ] || { STACK_WHY="CORTEX_SERVER_IMAGE is missing"; return 1; }
  [ -n "$sd" ] || { STACK_WHY="CORTEX_DB_IMAGE is missing"; return 1; }
  valid_semver "$sv" || { STACK_WHY="CORTEX_VERSION is not a SemVer without build metadata"; return 1; }
  for se_img in "$ss" "$sd"; do
    case $se_img in *@sha256:*) ;; *) STACK_WHY="an image is not name@sha256:<64 lower-case hex digits>"; return 1 ;; esac
    se_name=${se_img%@sha256:*}; se_dig=${se_img##*@sha256:}
    { [ -n "$se_name" ] && [ "${#se_dig}" -eq 64 ]; } || { STACK_WHY="an image has an empty name or a digest that is not 64 digits"; return 1; }
    case $se_name in *@*) STACK_WHY="an image name holds a second @"; return 1 ;; esac
    case $se_dig in *[!0123456789abcdef]*) STACK_WHY="an image digest is not 64 lower-case hex digits"; return 1 ;; esac
  done
  return 0
}
# ---- the home (W1-81 (c): check_home in shell): a real directory of the user's. A link is never followed and nothing is changed through one. home_check looks only (it runs before
# anything else reads the home); home_make creates the home, 0700 (it runs before a fetch and where the install first writes: a refusal before that makes nothing, IT-62).
home_check() {
  [ ! -L "$home_dir" ] || stop "$EX_MACHINE" home_not_real "$home_dir" "$home_dir is a symbolic link; it must be a real directory"
  [ ! -d "$home_dir" ] || [ "$(stat -c %u "$home_dir" 2>/dev/null)" = "$(id -u)" ] || stop "$EX_MACHINE" home_not_real "$home_dir" "$home_dir is not a directory of yours"
}
home_make() {
  home_check
  mkdir -p "$home_dir" || stop "$EX_START" cannot_write "$home_dir" "cannot create $home_dir"
  chmod 700 "$home_dir" || stop "$EX_START" cannot_write "$home_dir" "cannot set the mode of $home_dir"
  home_check
}

# Steps 1 and 2 (the prerequisite file's machine_look and prereq_run): a build without the file skips them, and a reissue needs neither. 0 passes; the codes each step may give pass through
# (the lane has printed its own cause and fix); any other status becomes the step's own code with one cause and fix of ours (W1-77: the real table).
run_step() {   # run_step CODE-FOR-AN-UNKNOWN-STATUS "ALLOWED CODES" COMMAND [ARGUMENT...]
  rs_dflt=$1; rs_allowed=$2; shift 2
  rs_rc=0; "$@" || rs_rc=$?
  arm_traps; PATH=$SAFE_PATH; export PATH   # a lane step may replace both (hale, line 109's own promise: every step)
  [ "$rs_rc" -ne 0 ] || return 0
  for rs_ok in $rs_allowed; do [ "$rs_rc" -ne "$rs_ok" ] || { { exit_handler_active=1; } 2>/dev/null; exit "$rs_rc"; }; done
  stop "$rs_dflt" step_status "$1 $rs_rc" "$1 ended with status $rs_rc, which the installer does not know"
}
backup_scripts="cortex-dump cortex-dump-status cortex-dump-alert cortex-restore restore-compare"
# report_strays: what the home holds that is not part of the install is KEPT and named in ONE line (W1-81 (c): what is not recognised is left alone and reported). Only the top level is read.
report_strays() {
  rs_list=""
  for rs_e in "$home_dir"/* "$home_dir"/.[!.]*; do
    { [ -e "$rs_e" ] || [ -L "$rs_e" ]; } || continue
    rs_b=${rs_e##*/}
    case $rs_b in
      bin|cortex.toml|compose-provider|current|releases|dumps|dump-status|"$lead.token"|"$lead.token.shown"|"$lead".token.[0123456789]*) ;;
      *) rs_list="$rs_list $rs_b" ;;
    esac
  done
  [ -z "$rs_list" ] || say "install: left alone (not part of the install, never removed):$rs_list"
}
# install_finished: 0 only if the MACHINE shows a finished install of this home and lead (W1-81 (a)). Each check VERIFIES (a length, a mode, an owner, a checksum, an answer), and a check
# that cannot run (a tool missing, a file unreadable, a command refused) is NOT a pass: the whole sequence then runs again, and every step of it is idempotent. No state file is read.
# The compose provider is the RECORDED one (<home>/compose-provider), re-verified here (path rules and version); a build without the prerequisite file has none and skips that one check.
# restart_unit_persistent (sam's NMR-SAM-01, cox a0c75b58): ONE predicate for "the distro's podman-restart.service is DURABLY enabled", used at BOTH sites (install_finished and the enable step) so that they cannot diverge. systemd's is-enabled
# exits 0 also for enabled-runtime (its links vanish at reboot) and for linked, linked-runtime, alias, static, indirect, generated and transient, so a successful status proves nothing: persistent means the status is 0 AND the answer is EXACTLY the word
# enabled (a comparison of the whole word, never a prefix). Anything else (those states, masked, disabled, an empty answer, an error) is NOT persistent. It leaves what systemd answered in restart_unit_state.
restart_unit_persistent() {
  ru_rc=0; restart_unit_state=$(systemctl --user is-enabled podman-restart.service 2>/dev/null) || ru_rc=$?
  [ "$ru_rc" -eq 0 ] && [ "$restart_unit_state" = enabled ]
}
install_finished() {
  inf_uid=$(id -u) || return 1
  { [ ! -L "$home_dir" ] && [ -d "$home_dir" ] && [ "$(stat -c %u "$home_dir" 2>/dev/null)" = "$inf_uid" ]; } || return 1
  { command -v podman >/dev/null 2>&1 && command -v curl >/dev/null 2>&1 && command -v systemctl >/dev/null 2>&1; } || return 1
  { [ -f "$home_dir/cortex.toml" ] && [ -s "$home_dir/cortex.toml" ]; } || return 1
  for inf_s in cv2-s5-db-superuser cv2-s5-owner-password cv2-s5-app-password; do
    inf_l=$(podman secret inspect --showsecret --format '{{len .SecretData}}' "$inf_s" 2>/dev/null) || return 1
    case $inf_l in ""|*[!0123456789]*|0) return 1 ;; esac
  done
  for inf_c in cv2-s5-db cv2-s5-server; do
    [ "$(podman inspect --format '{{.State.Status}} {{.State.Health.Status}}' "$inf_c" 2>/dev/null)" = 'running healthy' ] || return 1
  done
  select_token || return 1
  inf_tok=$token
  [ "$(podman inspect --format '{{.State.Status}} {{.State.ExitCode}}' cv2-s5-migrate 2>/dev/null)" = "exited 0" ] || return 1
  inf_rel=""
  if [ -f "$home_dir/current/stack.env" ]; then
    read_stack_env "$home_dir/current/stack.env" || return 1
    { [ -z "$CORTEX_RELEASE" ] || [ "$sv" = "$CORTEX_RELEASE" ]; } || return 1
    release_verified "$sv" quiet || return 1
    inf_rel=$sv; inf_dir="$home_dir/current"; inf_scripts="$inf_dir/bin"
  else
    [ -z "$CORTEX_RELEASE" ] || return 1
    inf_dir=$here; inf_scripts=${CORTEX_SCRIPTS_DIR:-"$here/../scripts"}
  fi
  inf_doc=$(curl --silent --show-error --fail --noproxy '*' --max-time 5 "http://127.0.0.1:$port/v2/health" 2>/dev/null) || return 1
  inf_flat=$(printf '%s' "$inf_doc" | tr -d ' \n\r\t') || return 1
  case $inf_flat in *'"ready":true'*) ;; *) return 1 ;; esac
  [ -z "$inf_rel" ] || case $inf_flat in *"\"version\":\"$inf_rel\""*) ;; *) return 1 ;; esac
  for inf_d in bin dumps dump-status; do { [ -d "$home_dir/$inf_d" ] && [ ! -L "$home_dir/$inf_d" ]; } || return 1; done
  for inf_f in $backup_scripts; do
    { [ -f "$inf_scripts/$inf_f" ] && [ -f "$home_dir/bin/$inf_f" ] && [ ! -L "$home_dir/bin/$inf_f" ]; } || return 1
    [ "$(cksum < "$inf_scripts/$inf_f")" = "$(cksum < "$home_dir/bin/$inf_f")" ] || return 1
    [ "$(stat -c %a "$home_dir/bin/$inf_f" 2>/dev/null)" = 755 ] || return 1
  done
  for inf_u in cortex-dump.service cortex-dump.timer cortex-dump-status.service cortex-dump-status.timer cortex-dump-alert.service; do
    { [ -f "$inf_dir/systemd/$inf_u" ] && [ -f "$HOME/.config/systemd/user/$inf_u" ] && [ ! -L "$HOME/.config/systemd/user/$inf_u" ]; } || return 1
    inf_want=$(sed -e "s|@CORTEX_HOME@|$home_dir|g" -e "s|@DB_CONTAINER@|cv2-s5-db|g" "$inf_dir/systemd/$inf_u" && printf x) || return 1
    inf_have=$(cat "$HOME/.config/systemd/user/$inf_u" && printf x) || return 1
    [ "$inf_want" = "$inf_have" ] || return 1
    [ "$(stat -c %a "$HOME/.config/systemd/user/$inf_u" 2>/dev/null)" = 644 ] || return 1
  done
  backup_timers_enabled || return 1
  restart_unit_persistent || return 1   # defect R: a finished install comes back after a reboot: the unit is DURABLY enabled (not just a successful status)
  if command -v prereq_compose_path >/dev/null 2>&1; then
    inf_rec="$home_dir/compose-provider"
    { [ -f "$inf_rec" ] && [ ! -L "$inf_rec" ] && [ "$(stat -c %a "$inf_rec" 2>/dev/null)" = 644 ] && [ "$(stat -c %u "$inf_rec" 2>/dev/null)" = "$inf_uid" ]; } || return 1
    inf_nl='
'
    inf_p=$(cat "$inf_rec" && printf x) || return 1
    inf_p=${inf_p%x}; inf_p=${inf_p%"$inf_nl"}
    case $inf_p in *"$inf_nl"*|"") return 1 ;; esac
    compose_path_ok "$inf_p" || return 1
    COMPOSE_PROVIDER=$inf_p; compose_version_ok || return 1
  fi
  return 0
}

# release_verified VERSION [quiet]: in a build that installs releases (rel_fetch joined) the installed release is the VERIFIED one: <home>/current is the link to <home>/releases/VERSION (compared by
# physical path) and the shell verifier (marv's verify_release_dir RELEASE_DIR, read-only, 0 only for a complete verified directory) says yes. A verifier that is missing is "cannot run" = no. A build
# that installs no release (a checkout) has nothing to verify here: 0.
release_verified() {
  command -v rel_fetch >/dev/null 2>&1 || return 0
  command -v verify_release_dir >/dev/null 2>&1 || return 1
  rv_dir="$home_dir/releases/$1"
  { [ -L "$home_dir/current" ] && [ -d "$rv_dir" ] && [ ! -L "$rv_dir" ]; } || return 1
  rv_a=$(cd -P -- "$home_dir/current" 2>/dev/null && pwd -P) || return 1
  rv_b=$(cd -P -- "$rv_dir" 2>/dev/null && pwd -P) || return 1
  [ "$rv_a" = "$rv_b" ] || return 1
  if [ "${2:-}" = quiet ]; then verify_release_dir "$rv_dir" >/dev/null 2>&1; else verify_release_dir "$rv_dir"; fi
}

# ---- the stage of the release (W1-85, W1-86): a PRIVATE directory made by mktemp -d (.cortex-install-stage.XXXXXXXX, 0700) inside the nearest EXISTING ancestor of the target home. The
# installer creates neither the home nor any directory above it before the release is proven. The stage is removed on EVERY exit (success, failure, HUP, INT, TERM); a leftover of a lost run
# (kill -9) is NEVER trusted: a stale real 0700 one of the user's is removed by its name, anything else is kept and named. A leftover ANCHOR directory (.cortex-install-anchor.*) is IGNORED: never used,
# never adopted, never removed (it holds public data only; a lost run cannot clean it, and an installer must not delete what it did not just make).
stage_made="" key_made=""
# anchor_remove: the key file is not needed after the trust step, so the anchor directory goes as soon as rel_trust is done (sam I6E-SAM02: its removal then does not depend on any trap), and again in the cleanup of every exit
anchor_remove() {
  if [ -n "$key_made" ]; then
    case ${key_made##*/} in .cortex-install-anchor.????????) { [ -d "$key_made" ] && [ ! -L "$key_made" ]; } && rm -rf -- "$key_made" || : ;; esac
    key_made=""
  fi
}
stage_cleanup() {
  if [ -n "$stage_made" ]; then
    case ${stage_made##*/} in .cortex-install-stage.????????) { [ -d "$stage_made" ] && [ ! -L "$stage_made" ]; } && rm -rf -- "$stage_made" || : ;; esac
    stage_made=""
  fi
  anchor_remove
}
# The two 6b wrappers (machine_look, prereq_run) are called BARE (hale 6E-1): inside a tested context (|| && ! if) errexit is OFF in the whole body, so an unchecked failing command would go on. The status they
# end the shell with is mapped HERE, in the EXIT handler, while cur_step names the step: a status the step may give passes (the lane printed its own cause and fix), a signal exit passes, any other becomes the
# step's own code with one cause and fix of ours. A lane may REPLACE the traps (marv's prereq_requalify_compose does): arm_traps puts ours back after each step.
cur_step="" cur_default=0 cur_allowed=""
exit_handler() {
  set +e
  eh_st=$exit_handler_status
  arm_signal_traps
  case $eh_st in 0|129|130|143) ;; *)
    if [ -n "$cur_step" ]; then
      eh_ok=0; for eh_c in $cur_allowed; do [ "$eh_st" -ne "$eh_c" ] || eh_ok=1; done
      if [ "$eh_ok" -eq 0 ]; then
        msg step_status "$cur_step $eh_st" || :
        printf 'install: %s ended with status %s, which the installer does not know\n' "$cur_step" "$eh_st" >&2 || :   # hale 6E3-2: a closed stderr must not skip the cleanup below
        eh_st=$cur_default
      fi
    fi ;;
  esac
  stage_cleanup
  [ "$eh_st" = 0 ] || summary_abort
  mint_reservation_release
  unset val summary_secret summary_reply
  arm_final_signal_traps
  if [ "$eh_st" = 0 ] && [ -n "$exit_handler_signal" ]; then eh_st=$exit_handler_signal; fi
  [ "$install_trace_requested" != 1 ] || set -x
  : # tracing is restored before the final untraced status read
  { exit "$eh_st"; } 2>/dev/null
}
# Ordinary traps also consult the handler flag, so deferral starts with its
# first status/flag assignment and survives every nested trap restoration.
install_signal() {
  if [ "$exit_handler_active" = 1 ]; then
    exit_handler_signal=${exit_handler_signal:-$1}
  else
    { exit_handler_active=1; } 2>/dev/null; exit "$1"
  fi
}
arm_signal_traps() {
  trap 'install_signal 129' HUP
  trap 'install_signal 130' INT
  trap 'install_signal 143' TERM
}
restore_deferred_signal() {
  if [ "$exit_handler_active" = 1 ]; then
    [ -z "$1" ] || exit_handler_signal=${exit_handler_signal:-$1}
    arm_signal_traps
    return 0
  fi
  arm_signal_traps
  [ -z "$1" ] || { { exit_handler_active=1; } 2>/dev/null; exit "$1"; }
}
# Once cleanup is complete, signals terminate with the same status contract
# even if tracing blocks after the last adoption of recorded state.
arm_final_signal_traps() {
  if [ "$eh_st" = 0 ]; then
    trap 'exit "${exit_handler_signal:-129}"' HUP
    trap 'exit "${exit_handler_signal:-130}"' INT
    trap 'exit "${exit_handler_signal:-143}"' TERM
  else
    trap 'exit "$eh_st"' HUP INT TERM
  fi
}
# Capture status and enter deferral without a trace write before the handler.
# The grouped redirection covers this assignment only; cleanup remains public.
arm_traps() {
  trap '{ exit_handler_status=$? exit_handler_active=1; } 2>/dev/null; exit_handler' EXIT
  arm_signal_traps
}
arm_traps
# stage_parent: the nearest existing ancestor of the home; a real directory (not a link), owned by the user and writable by the user, or 1 (it never goes further up to find a better one)
stage_parent() {
  sp=${home_dir%/*}
  while :; do
    [ -n "$sp" ] || sp=/
    { [ -e "$sp" ] || [ -L "$sp" ]; } && break
    sp=${sp%/*}
  done
  { [ -d "$sp" ] && [ ! -L "$sp" ] && [ "$(stat -c %u "$sp" 2>/dev/null)" = "$(id -u)" ] && [ -w "$sp" ]; } || return 1
  printf '%s\n' "$sp"
}
# stage_leftovers PARENT: a leftover is a real directory of the user's with mode 0700 and NOTHING in it changed for 30 minutes (a live run of another install keeps touching its own): removed by
# its exact path and said. Anything else of that name (a link, a file, another mode, a young one) is KEPT and named.
stage_leftovers() {
  sl_keep=""
  for sl_d in "$1"/.cortex-install-stage.*; do
    { [ -e "$sl_d" ] || [ -L "$sl_d" ]; } || continue
    if [ -d "$sl_d" ] && [ ! -L "$sl_d" ] && [ "$(stat -c %u "$sl_d" 2>/dev/null)" = "$(id -u)" ] && [ "$(stat -c %a "$sl_d" 2>/dev/null)" = 700 ] && [ -z "$(find "$sl_d" -mmin -30 2>/dev/null | sed -n 1p)" ]; then
      rm -rf -- "$sl_d" && say "install: removed the leftover stage directory $sl_d of a lost run" || sl_keep="$sl_keep ${sl_d##*/}"
    else
      sl_keep="$sl_keep ${sl_d##*/}"
    fi
  done
  [ -z "$sl_keep" ] || say "install: left alone (not removed, never trusted):$sl_keep"
}
# anchor_ok: the three literals are all filled and each is in rel_trust's own grammar (hue r1 section 2, confirmed by hue 12:33Z): principal 1 to 64 of [A-Za-z0-9._@-] not starting with a hyphen; key 68 of
# [A-Za-z0-9+/] starting AAAAC3NzaC1lZDI1NTE5AAAAI (an ed25519 key and nothing else: another type's blob has another prefix); fingerprint SHA256: and exactly 43 of [A-Za-z0-9+/]. The letters are spelled out
# (a range depends on the locale). Checked here so that a bad literal refuses BEFORE anything is fetched and before a key file is written.
anchor_ok() {
  [ -n "$CORTEX_ANCHOR_PRINCIPAL" ] && [ -n "$CORTEX_ANCHOR_KEY" ] && [ -n "$CORTEX_ANCHOR_FPR" ] || return 1
  [ "${#CORTEX_ANCHOR_PRINCIPAL}" -le 64 ] || return 1
  case $CORTEX_ANCHOR_PRINCIPAL in -*|*[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._@-]*) return 1 ;; esac
  [ "${#CORTEX_ANCHOR_KEY}" -eq 68 ] || return 1
  case $CORTEX_ANCHOR_KEY in AAAAC3NzaC1lZDI1NTE5AAAAI*) ;; *) return 1 ;; esac
  case $CORTEX_ANCHOR_KEY in *[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/]*) return 1 ;; esac
  case $CORTEX_ANCHOR_FPR in SHA256:*) ;; *) return 1 ;; esac
  af_b=${CORTEX_ANCHOR_FPR#SHA256:}
  [ "${#af_b}" -eq 43 ] || return 1
  case $af_b in *[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/]*) return 1 ;; esac
  return 0
}
stage_make() {
  sm_parent=$(stage_parent) || stop "$EX_MACHINE" stage_parent "$home_dir" "no directory of yours to stage the release in near $home_dir"
  stage_leftovers "$sm_parent"
  stage_made=$(mktemp -d "$sm_parent/.cortex-install-stage.XXXXXXXX") || stop "$EX_MACHINE" cannot_write "$sm_parent" "cannot make a stage directory in $sm_parent"
  CORTEX_STAGE_DIR=$stage_made; export CORTEX_STAGE_DIR
  # the anchor's key line "PRINCIPAL ssh-ed25519 KEY" goes into a private 0600 file in a fresh 0700 directory BESIDE the stage (never inside it: the stage holds exactly the seven files), a new mktemp name every
  # run, removed on every exit that can run a trap; an old anchor directory of a lost run is ignored. anchor_ok has passed before this function is called.
  key_made=$(mktemp -d "$sm_parent/.cortex-install-anchor.XXXXXXXX") || stop "$EX_MACHINE" cannot_write "$sm_parent" "cannot make an anchor directory in $sm_parent"
  { printf '%s ssh-ed25519 %s\n' "$CORTEX_ANCHOR_PRINCIPAL" "$CORTEX_ANCHOR_KEY" > "$key_made/key" && chmod 0600 "$key_made/key"; } || stop "$EX_MACHINE" cannot_write "$key_made" "cannot write the key file in $key_made"
  key_file="$key_made/key"
}
# trust_tools_ok: step 2, read-only. The tools the release check itself needs (marv's inventory): sha256sum, tar with gzip, ssh-keygen, and curl for the online channel only. A missing one
# stops here with the exact command (EX_MACHINE): nothing was fetched, nothing changed, and the installer does not install these before the release is proven.
trust_tools_ok() {
  for tt in sha256sum tar gzip ssh-keygen; do
    command -v "$tt" >/dev/null 2>&1 || stop "$EX_MACHINE" trust_tool_missing "$tt" "the tool $tt is missing, and the release cannot be checked without it"
  done
  [ -n "$CORTEX_FROM" ] || command -v curl >/dev/null 2>&1 || stop "$EX_MACHINE" trust_tool_missing curl "the tool curl is missing, and an online release cannot be fetched without it"
}

# ---- the sequence ----------------------------------------------------------------------------------------------------------------------------------------------------------------
summary_active=1
[ "$(id -u)" -ne 0 ] || stop "$EX_MACHINE" not_root "" "run as a normal user: the stack is rootless Podman"
# A FINISHED install (every check of install_finished passes) is recognised FIRST, before steps 1 and 2: the machine look would call our own running stack on the port a blocking finding, and
# a second run changes nothing (G5). It says what is installed and exits 0. Anything else runs the whole sequence below, which is idempotent step by step. A reissue is never "finished".
if [ "$reissue" -eq 0 ] && install_finished; then
  report_strays
  summary_repeat=1
  summary_finish
  { exit_handler_active=1; } 2>/dev/null; exit 0
fi
home_check
# A --release that differs from the release installed in this home is REFUSED, read-only, before anything runs (hale N-2): moving between releases is cortex upgrade, never a silent reinstall
# of the old one. No --release means "whatever is installed".
if [ "$reissue" -eq 0 ] && [ -n "$CORTEX_RELEASE" ] && [ -f "$home_dir/current/stack.env" ] && read_stack_env "$home_dir/current/stack.env" && [ "$sv" != "$CORTEX_RELEASE" ]; then
  stop "$EX_USAGE" release_differs "$sv" "release $sv is installed in $home_dir, not $CORTEX_RELEASE"
fi
# An INSTALLED release is PROVEN before anything changes the machine (vera V6E-01, W1-85 on this path too): its stack.env must read and, in a build that installs releases, current must be the link to
# releases/V and the shell verifier must say yes. Read-only; a failure or an unavailable proof stops here with a cause and fix, before the machine look, the prerequisites and their consent.
if [ "$reissue" -eq 0 ] && [ -f "$home_dir/current/stack.env" ]; then
  read_stack_env "$home_dir/current/stack.env" || { msg stack_env_bad || :; fail "$EX_FETCH" "the release at $home_dir/current has a stack.env that cannot be used ($STACK_WHY); nothing was changed"; }
  release_verified "$sv" || stop "$EX_FETCH" release_unverified "$sv" "the installed release $sv could not be verified; nothing was changed"
fi
# Another --port while the stack is up is refused too (cox 820170f5): read-only, and before anything runs. What it cannot read is not a refusal.
if [ "$reissue" -eq 0 ] && command -v podman >/dev/null 2>&1 && podman container exists cv2-s5-server >/dev/null 2>&1; then
  ip=$(podman port cv2-s5-server 8601/tcp 2>/dev/null | sed -n 's/^.*://p' | sed -n 1p) || ip=""
  case $ip in ""|*[!0123456789]*) ;; *) [ "$ip" = "$port" ] || stop "$EX_USAGE" port_differs "$ip" "the stack is already installed on port $ip, not $port" ;; esac
fi
# A release to install: --release V, the fetch functions joined, and none installed in this home (an installed one of that version is not fetched again).
need_fetch=0
if [ "$reissue" -eq 0 ] && [ -n "$CORTEX_RELEASE" ] && command -v rel_fetch >/dev/null 2>&1; then
  valid_semver "$CORTEX_RELEASE" || stop "$EX_USAGE" step_status "release" "the release $CORTEX_RELEASE is not a SemVer"
  [ -f "$home_dir/current/stack.env" ] || need_fetch=1
fi
# The order of W1-85: 1 look, 2 the trust tools, 3 fetch and trust in the stage, 4 the prerequisites, 5 unpack and load. A build without the prerequisite file skips 1 and 4, a reissue needs
# none of them. A fetch that cannot be verified is not made (FAIL CLOSED: no rel_trust, rel_unpack or rel_load = no fetch).
if [ "$reissue" -eq 0 ]; then
  if command -v machine_look >/dev/null 2>&1; then
    cur_step=machine_look cur_default=$EX_MACHINE cur_allowed="$EX_MACHINE $EX_PREREQ"   # marv's r4: a failed diagnostic output returns EX_PREREQ, which passes through
    machine_look
    summary_record 'Checked this machine' ok "$(summary_system)"
    cur_step=""; arm_traps; PATH=$SAFE_PATH; export PATH
  fi
  if [ "$need_fetch" -eq 1 ]; then
    { command -v rel_trust >/dev/null 2>&1 && command -v rel_unpack >/dev/null 2>&1 && command -v rel_load >/dev/null 2>&1; } || stop "$EX_TRUST" trust_missing "" "this build has no rel_trust, rel_unpack or rel_load: it cannot verify or unpack a release"
    anchor_ok || stop "$EX_TRUST" anchor_unusable "" "this installer carries no usable trust anchor, so it will not fetch a release"
    trust_tools_ok
    stage_make
    run_step "$EX_FETCH" "$EX_FETCH" rel_fetch_noproxy "$CORTEX_RELEASE" "$CORTEX_STAGE_DIR"
    run_step "$EX_TRUST" "$EX_TRUST $EX_FETCH" rel_trust "$CORTEX_RELEASE" "$CORTEX_STAGE_DIR" "$key_file" "$CORTEX_ANCHOR_PRINCIPAL" "$CORTEX_ANCHOR_FPR"
    summary_record 'Release fetched and verified' ok "$CORTEX_RELEASE; signer $CORTEX_ANCHOR_FPR"
    anchor_remove
  fi
  if command -v prereq_run >/dev/null 2>&1; then
    summary_phase=Prerequisites
    summary_prereq_changes="${PREREQ_MISSING:-none}; ${PREREQ_FIXES:-none}"
    cur_step=prereq_run cur_default=$EX_PREREQ cur_allowed="$EX_PREREQ $EX_MACHINE"
    prereq_run
    summary_record Prerequisites ok "changes requested: $summary_prereq_changes"
    cur_step=""; arm_traps; PATH=$SAFE_PATH; export PATH; pin_podman
  fi
fi
podman_ok || stop "$EX_PREREQ" "$PREFLIGHT_ID" "" "$PREFLIGHT_WHY"
[ "$(stat -fc %T /sys/fs/cgroup 2>/dev/null)" = "cgroup2fs" ] || stop "$EX_MACHINE" no_cgroup2 "" "cgroup v2 is required"
compose_provider_ok || stop "$EX_PREREQ" "$PREFLIGHT_ID" "" "$PREFLIGHT_WHY"
if ! command -v machine_look >/dev/null 2>&1; then
  summary_record 'Checked this machine' ok "$(summary_system); cgroup v2"
fi
summary_record 'Verified prerequisites' ok "$(podman --version); podman-compose $COMPOSE_VERSION"
summary_phase='Images loaded'
if [ "$need_fetch" -eq 1 ]; then
  home_make
  run_step "$EX_FETCH" "$EX_FETCH $EX_TRUST" rel_unpack "$CORTEX_RELEASE" "$CORTEX_STAGE_DIR" "$home_dir"
  run_step "$EX_FETCH" "$EX_FETCH" rel_load "$CORTEX_RELEASE" "$CORTEX_STAGE_DIR" "$home_dir"
  summary_record 'Images loaded' ok "$CORTEX_RELEASE"
  stage_cleanup
  [ -f "$home_dir/current/stack.env" ] || stop "$EX_FETCH" release_incomplete "$home_dir/current/stack.env" "the release was not laid down: $home_dir/current/stack.env is missing"
fi
if [ -f "$home_dir/current/stack.env" ]; then
  release_mode=1; rel_dir="$home_dir/current"
  read_stack_env "$rel_dir/stack.env" || { msg stack_env_bad || :; fail "$EX_FETCH" "the release at $rel_dir has a stack.env that cannot be used ($STACK_WHY); nothing was started"; }
  CORTEX_VERSION=$sv; CORTEX_SERVER_IMAGE=$ss; CORTEX_DB_IMAGE=$sd; export CORTEX_VERSION CORTEX_SERVER_IMAGE CORTEX_DB_IMAGE
  [ "$reissue" -eq 1 ] || release_verified "$sv" || stop "$EX_FETCH" release_unverified "$sv" "the installed release $sv could not be verified; nothing was started"
  summary_record 'Release and image references' ok "$sv; $ss; $sd"
else
  release_mode=0; rel_dir="$here"
  summary_record 'Release fetched and verified' 'skipped' 'source checkout'
  summary_record 'Images loaded' 'skipped' 'source checkout builds the server image'
fi

# ---- preflight: the backup scripts (piece 12, bob) must be there before anything is made or started. A release carries them in bin/; a checkout has them in scripts/
# (CORTEX_SCRIPTS_DIR names another directory for a checkout: the tests and the packaging use it).
if [ "$release_mode" -eq 1 ]; then scripts_src="$rel_dir/bin"; else scripts_src=${CORTEX_SCRIPTS_DIR:-"$here/../scripts"}; fi
if [ "$reissue" -eq 0 ]; then
  for f in $backup_scripts; do [ -f "$scripts_src/$f" ] || stop "$EX_FETCH" release_incomplete "$f in $scripts_src" "backup script $f is missing in $scripts_src"; done
fi

home_make

# ---- reissue: only a new token for an existing, healthy stack -----------------------------------------------------
if [ "$reissue" -eq 1 ]; then
  wait_healthy cv2-s5-db cv2-s5-server
  new="$home_dir/$lead.token.$(date -u +%Y%m%dT%H%M%SZ)"
  mint_token "$new"
  token=$new; summary_new_token=1
  summary_record 'Project and lead created' skipped 'reissue requires an existing lead'
  summary_record 'Lead credential written' ok "0$(stat -c %a "$token"), path below"
  echo 'install: new token file created (0600); the old token is superseded. Use the file named below.'
  summary_finish
  { exit_handler_active=1; } 2>/dev/null; exit 0
fi

# ---- the verified compose provider, recorded where the engine reads it (W1-68 item 3): <home>/compose-provider, ONE line, the absolute path, 0644, owned by the user. Only a path that the
# prerequisite step verified is recorded (the engine trusts 6b, not a guess). Written by a temporary name and a rename; left alone when it is already right; a link or anything that
# is not a plain file there is never followed or removed.
if [ "$COMPOSE_FROM_PREREQ" -eq 1 ]; then
  cprec="$home_dir/compose-provider"
  if [ -L "$cprec" ] || { [ -e "$cprec" ] && [ ! -f "$cprec" ]; }; then
    stop "$EX_START" provider_file "$cprec" "$cprec is not a plain file"
  fi
  cp_nl='
'
  cp_now=$(cat "$cprec" 2>/dev/null && printf x) || cp_now=""
  if [ "${cp_now%x}" = "$COMPOSE_PROVIDER$cp_nl" ] && [ "$(stat -c %a "$cprec" 2>/dev/null)" = 644 ] && [ "$(stat -c %u "$cprec" 2>/dev/null)" = "$(id -u)" ]; then :; else
    printf '%s\n' "$COMPOSE_PROVIDER" > "$cprec.new" && chmod 0644 "$cprec.new" && mv -f "$cprec.new" "$cprec" || stop "$EX_START" cannot_write "$cprec" "cannot write $cprec"
  fi
fi

# ---- secrets: random values straight into Podman secrets ---------------------------------------------------------
make_secret() {
  case $- in *x*) make_secret_trace=1 ;; *) make_secret_trace=0 ;; esac
  set +x
  unset val
  if podman secret inspect "$1" >/dev/null 2>&1; then
    sl=$(podman secret inspect --showsecret --format '{{len .SecretData}}' "$1" 2>/dev/null) || sl=""
    case $sl in ""|*[!0123456789]*|0) stop "$EX_START" secret_unreadable "$1" "the secret $1 exists but is empty or could not be read" ;; esac
    [ "$make_secret_trace" != 1 ] || set -x
    return 0
  fi
  die_code=$EX_START die_id=no_random die_arg=
  val=$(head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n')
  case $val in *[!0123456789abcdef]*) unset val; die "could not read random bytes for secret $1" ;; esac
  [ "${#val}" -eq 64 ] || { unset val; die "could not read random bytes for secret $1"; }
  printf '%s' "$val" | podman secret create "$1" - >/dev/null || { unset val; stop "$EX_START" secret_create_failed "$1" "podman could not create the secret $1"; }
  unset val
  [ "$make_secret_trace" != 1 ] || set -x
}
make_secret cv2-s5-db-superuser
make_secret cv2-s5-owner-password
make_secret cv2-s5-app-password

# ---- config: mounted, never baked --------------------------------------------------------------------------------
[ -f "$home_dir/cortex.toml" ] || { cp "$rel_dir/cortex.toml.example" "$home_dir/cortex.toml" && chmod 644 "$home_dir/cortex.toml"; } || stop "$EX_START" cannot_write "$home_dir/cortex.toml" "cannot write the config $home_dir/cortex.toml"
CORTEX_CONFIG="$home_dir/cortex.toml"; export CORTEX_CONFIG
CORTEX_PORT=$port; export CORTEX_PORT

# ---- the dump directories (piece 12) and the git sha of the build ------------------------------------------------------
# <home>/dumps: cortex-dump writes its sets here. PRIVATE (vera, option C): 0700 when WE make it, and it is mounted into NO container. An existing
# directory keeps its files and its mode.
[ ! -L "$home_dir/dumps" ] || stop "$EX_START" dir_symlink "$home_dir/dumps" "$home_dir/dumps is a symbolic link; it must be a real directory (a link could point anywhere)"
[ -d "$home_dir/dumps" ] || { mkdir "$home_dir/dumps" && chmod 0700 "$home_dir/dumps"; } || stop "$EX_START" cannot_write "$home_dir/dumps" "cannot create $home_dir/dumps"
# <home>/dump-status: the ONLY dump thing the server sees, read-only at /var/lib/cortex-dumps. It holds LAST_GOOD alone (cortex-dump writes a temporary
# file inside it with the final mode 0644 and renames it). 0755 so any uid in the container can read it and only the owner can write it. An existing
# directory keeps its files and its mode.
[ ! -L "$home_dir/dump-status" ] || stop "$EX_START" dir_symlink "$home_dir/dump-status" "$home_dir/dump-status is a symbolic link; it must be a real directory (the server reads it, and a link to the private dumps would show it everything)"
[ -d "$home_dir/dump-status" ] || { mkdir "$home_dir/dump-status" && chmod 0755 "$home_dir/dump-status"; } || stop "$EX_START" cannot_write "$home_dir/dump-status" "cannot create $home_dir/dump-status"
CORTEX_DUMP_STATUS="$home_dir/dump-status"; export CORTEX_DUMP_STATUS
unset CORTEX_DUMPS   # compose has no use for the private tree; a stale value from the environment is not passed on
# The sha of the checkout this script runs from, for `cortex-server --version`: a full lowercase sha or `unknown`; nothing from the environment.
if [ "$release_mode" -eq 1 ]; then sha=unknown; else sha=$(git -C "$here/.." rev-parse HEAD 2>/dev/null || echo unknown); fi
case $sha in ""|*[!0123456789abcdef]*) sha=unknown ;; esac
[ "${#sha}" -eq 40 ] || sha=unknown
CORTEX_GIT_SHA=$sha; export CORTEX_GIT_SHA

# ---- the backup scripts into <home>/bin, 0755, by a temporary name and a rename (the units run cortex-dump, cortex-dump-status and cortex-dump-alert from here;
# cortex upgrade needs the installed cortex-dump before it takes a dump)
mkdir -p "$home_dir/bin" || stop "$EX_START" cannot_write "$home_dir/bin" "cannot create $home_dir/bin"
for f in $backup_scripts; do
  cp "$scripts_src/$f" "$home_dir/bin/$f.new" && chmod 0755 "$home_dir/bin/$f.new" && mv -f "$home_dir/bin/$f.new" "$home_dir/bin/$f" || stop "$EX_START" cannot_write "$home_dir/bin" "cannot install $f into $home_dir/bin"
done

[ "$start" -eq 1 ] || {
  summary_record 'Containers started' skipped '--no-start'
  summary_record 'Project and lead created' skipped '--no-start'
  summary_record 'Lead credential written' skipped '--no-start'
  summary_record Backups skipped '--no-start'
  summary_line ' What was done'; summary_line "$summary_steps"; summary_complete=1
  echo "install: secrets and config are ready; --no-start given, nothing started."; { exit_handler_active=1; } 2>/dev/null; exit 0;
}
summary_phase='Containers started'

# ---- start, then wait (bounded) for both containers --------------------------------------------------------------
# The module ids come from the mounted config's single-line [modules] enabled list, so the roles match the build.
# Only the [modules] table counts: an "enabled" list in another table is not a module list (nix N17). One line, no multi-line arrays.
die_code=$EX_START die_id=config_modules die_arg=
raw=$(awk '
  /^[[:space:]]*\[/ { t = $0; gsub(/[[:space:]]/, "", t); in_mod = (t == "[modules]"); next }
  in_mod && /^[[:space:]]*enabled[[:space:]]*=/ { if (index($0, "]") == 0) { print "M:"; exit } line = $0; sub(/^[^[]*\[/, "", line); sub(/\].*$/, "", line); gsub(/["[:space:]]/, "", line); gsub(/,/, " ", line); print "F:" line; exit }
' "$CORTEX_CONFIG")
[ -n "$raw" ] || die "no [modules] enabled list (one line) found in $CORTEX_CONFIG"
case $raw in "M:") die "the [modules] enabled list in $CORTEX_CONFIG must be on one line" ;; esac
modules=${raw#F:}
[ -n "$modules" ] || die "the [modules] enabled list in $CORTEX_CONFIG is empty"
for id in $modules; do
  case $id in ""|[!abcdefghijklmnopqrstuvwxyz]*|*[!abcdefghijklmnopqrstuvwxyz0123456789_]*) die "invalid module id in $CORTEX_CONFIG: $(printf '%.40s' "$id")" ;; esac
  [ "${#id}" -le 32 ] || die "invalid module id in $CORTEX_CONFIG (longer than 32 characters)"
done

# 1. the database alone, 2. the module roles (a superuser step, before any migration), 3. build the image, 4. migrate and the server
compose_run -f "$rel_dir/compose.yml" up -d db || stop "$EX_START" compose_up_failed "" "compose up failed for the database (see the output above)"
wait_healthy cv2-s5-db
# The initdb auth flags act on a FRESH cluster only, so read the EFFECTIVE rules, not the environment (vera): a database created before the flags may still
# trust. Refuse it: change nothing IN THE DATABASE and never delete or recreate a volume to fix it. This point is reached after the home, the secrets, the configuration, the backup scripts and the database start (W1-223, sam E01-SAM01): the three messages below say what the refusal did not touch and that what is made stays in place, never 'nothing was changed'.
die_code=$EX_START die_id=db_unreadable die_arg=
trust_rules=$(podman exec -i --user postgres cv2-s5-db psql -U postgres -d cortex -tAc "select count(*) from pg_hba_file_rules where auth_method = 'trust' or error is not null" 2>/dev/null) || die "could not read the database's access rules: check podman logs cv2-s5-db, then run this installer again (that is safe); this refusal did not touch the database's rules, data or volume, and what the installer has made so far stays in place"
case $trust_rules in ""|*[!0123456789]*) die "could not read a count of trust rules from the database (got: $(printf '%.20s' "$trust_rules")): check podman logs cv2-s5-db, then run this installer again (that is safe); this refusal did not touch the database's rules, data or volume, and what the installer has made so far stays in place" ;; esac
[ "$trust_rules" -eq 0 ] || stop "$EX_START" db_trust "" "the database has $trust_rules trust or broken pg_hba rule(s) (it was created before the auth flags); this refusal did not touch the database's rules, data or volume, and what the installer has made so far stays in place. See S5-PEER-AUTH-MAP.md for the explicit step; do not delete the volume"
# The PLATFORM role (kai's first real install, 9 Nov, defect J): the platform migration grants to cortex_m_platform, which no enabled module is: the roles step makes it too (db-roles.sh makes a role for any valid id; IT-147 checks every cortex_m_ role that the shipped migrations name).
roles_platform=platform
# cox W1-55: the CALLER passes the absolute deadline E (epoch seconds); db-roles.sh reads no clock of its own and refuses to commit past E - 9. There is no engine here, so this script is the caller: now + 60 s.
# shellcheck disable=SC2086
sh "$rel_dir/db-roles.sh" --deadline "$(( $(date +%s) + 60 ))" $modules $roles_platform || stop "$EX_START" roles_failed "" "creating the module roles failed (see above); nothing else was started"
# The image is built here and never pulled (pull_policy: never), and migrate runs from the same image as the server, so build it
# explicitly BEFORE either starts rather than relying on the order compose picks.
# (a release carries the image: it was loaded by the fetch step and nothing is built)
if [ "$release_mode" -eq 0 ]; then
  compose_run -f "$rel_dir/compose.yml" build server || stop "$EX_START" build_failed "" "image build failed (see the output above); nothing else was started"
fi
# DEFECT M (kai's full pass, 9 Nov; cox c2ca1f6c option 1, then M v2 after sam's NMR-SAM-02, cox a0c75b58): podman-compose does not wait for migrate (depends_on has no effect there), so after a failed migration it HAD started cv2-s5-server
# (Up, health 503, its rootlessport on the port): the message said the server was not started, and the retry it recommends died on 'the selected port is already listening'. The container compose started is removed, but only when it is THIS install's:
#  - ONE `podman container inspect` by name answers the container's FULL ID and its labels together, so they describe the SAME object, and the removal is by that ID (`podman rm -f <64 hex>`), never by name: an object replaced in between has another
#    id, the removal of the old one fails, and that is reported as a removal failure (nothing else is touched);
#  - ours means cortexv2=dev (compose.yml's own label, which every compose service of ours shares and a user can copy) AND the compose project label equal to the project this installer gives compose (cv2-s5: compose.yml's name, held by S5-03)
#    AND the compose working-directory label equal to the physical directory of THIS install's compose file (podman-compose 1.5.0 records the RESOLVED directory of the -f file: measured on the real podman 5.8.2, probe-m2.out). WHAT THIS DOES NOT
#    PROVE: labels are text, not a signature; a container made by another install of the same project from the same directory, or one that copies all three labels, looks the same. It ties the container to this project and this directory, no more.
# `compose down` is NOT used (it would delete cv2-s5-migrate, whose logs the message points to), the database container stays (the retry is idempotent) and the port guard of the prerequisite file is unchanged. It prints ONE word: none (no such
# container), removed, foreign (not this install's: left alone), stuck (podman could not say, the answer was not an id, the directory could not be resolved, or the removal failed).
own_project=cv2-s5
own_server_cleanup() {
  oc_rc=0; podman container exists cv2-s5-server >/dev/null 2>&1 || oc_rc=$?
  case $oc_rc in 0) ;; 1) echo none; return 0 ;; *) echo stuck; return 0 ;; esac
  oc_info=$(podman container inspect --format '{{.Id}}{{"\n"}}{{index .Config.Labels "cortexv2"}}{{"\n"}}{{index .Config.Labels "com.docker.compose.project"}}{{"\n"}}{{index .Config.Labels "com.docker.compose.project.working_dir"}}' cv2-s5-server 2>/dev/null) || { echo stuck; return 0; }
  oc_id=$(printf '%s\n' "$oc_info" | sed -n 1p); oc_lab=$(printf '%s\n' "$oc_info" | sed -n 2p); oc_proj=$(printf '%s\n' "$oc_info" | sed -n 3p); oc_wd=$(printf '%s\n' "$oc_info" | sed -n 4p)
  case $oc_id in ""|*[!0123456789abcdef]*) echo stuck; return 0 ;; esac
  [ "${#oc_id}" -eq 64 ] || { echo stuck; return 0; }
  oc_here=$(cd -P -- "$rel_dir" 2>/dev/null && pwd -P) || { echo stuck; return 0; }
  { [ "$oc_lab" = dev ] && [ "$oc_proj" = "$own_project" ] && [ "$oc_wd" = "$oc_here" ]; } || { echo foreign; return 0; }
  if podman rm -f "$oc_id" >/dev/null 2>&1; then echo removed; else echo stuck; fi
}
up_ok=1
compose_run -f "$rel_dir/compose.yml" up -d || up_ok=0
# migrate is a one-shot: it has no health status, so read its exit code. The server waits for it, so a failure stops here.
migrate_state=$(podman inspect --format '{{.State.Status}} {{.State.ExitCode}}' cv2-s5-migrate 2>/dev/null || echo missing)
case $migrate_state in
  "exited 0") ;;
  "exited "*)
    case $(own_server_cleanup) in
      removed) stop "$EX_START" migrate_failed_removed "$migrate_state" "the migrations failed ($migrate_state); the server container compose had started was removed (see: podman logs cv2-s5-migrate)" ;;
      foreign) stop "$EX_START" migrate_failed_foreign "$migrate_state" "the migrations failed ($migrate_state); a container named cv2-s5-server that is not this installer's was left alone (see: podman logs cv2-s5-migrate)" ;;
      stuck) stop "$EX_START" migrate_failed_stuck "$migrate_state" "the migrations failed ($migrate_state); the server container compose may have started could not be checked or removed (see: podman logs cv2-s5-migrate)" ;;
      *) stop "$EX_START" migrate_failed "$migrate_state" "the migrations failed ($migrate_state); the server was not started (see: podman logs cv2-s5-migrate)" ;;
    esac ;;
esac
[ "$up_ok" -eq 1 ] || stop "$EX_START" compose_up_failed "" "compose up failed (see the output above)"
wait_healthy cv2-s5-db cv2-s5-server
summary_record 'Containers started' ok 'cv2-s5-db and cv2-s5-server healthy; migrate exited 0'

# ---- the first lead token: minted once, in memory (tmpfs), piped out into a 0600 file -----------------------------
select_token || :
summary_phase='Project and lead created'
if [ -s "$token" ]; then
  token_file_owned "$token" || stop "$EX_WORKER" token_failed '' 'the token is not a real file owned by this user'
  token_private "$token" || stop "$EX_WORKER" token_not_private '' 'the existing token file grants access to group or others; use --reissue-token for a new credential'
  echo 'install: private token file exists; bootstrap already done (use --reissue-token for a new one).'
else
  [ ! -e "$token" ] || { echo 'install: private token file is empty (a failed earlier run); minting a new token'; rm -f "$token" || stop "$EX_WORKER" cannot_write '' 'cannot remove the empty private token file'; }
  mint_token "$token"
  summary_new_token=1
fi
if [ "$summary_new_token" = 1 ]; then
  summary_record 'Project and lead created' ok "project $project; lead $lead"
else
  summary_record 'Project and lead created' skipped 'existing credential; sign-in checked below'
fi
summary_record 'Lead credential written' ok "0$(stat -c %a "$token"), path below"
summary_phase=Backups

# ---- the backup units (piece 12): user units with two placeholders filled in, and the two TIMERS enabled (never a service or the alert) ---------------------
# The stack is up by now; if the user manager cannot be reached the install fails LOUDLY (a backup that is never scheduled is a data-loss risk) and a second run,
# which repeats all of this, finishes the job.
unit_dir="$HOME/.config/systemd/user"
mkdir -p "$unit_dir" || stop "$EX_BACKUP" units_failed "$unit_dir" "cannot make $unit_dir"
for u in cortex-dump.service cortex-dump.timer cortex-dump-status.service cortex-dump-status.timer cortex-dump-alert.service; do
  sed -e "s|@CORTEX_HOME@|$home_dir|g" -e "s|@DB_CONTAINER@|cv2-s5-db|g" "$rel_dir/systemd/$u" > "$unit_dir/$u.new" && chmod 0644 "$unit_dir/$u.new" && mv -f "$unit_dir/$u.new" "$unit_dir/$u" || stop "$EX_BACKUP" units_failed "$u" "cannot install the unit $u"
done
{ systemctl --user daemon-reload && systemctl --user enable --now cortex-dump.timer cortex-dump-status.timer; } || stop "$EX_BACKUP" timers_failed "" "backup timers could not be enabled (systemctl --user failed; is lingering on for this user?). The stack is up; fix that and run install.sh again"
backup_timers_enabled || stop "$EX_BACKUP" timers_failed '' 'backup timers were not confirmed enabled'
summary_record Backups ok 'cortex-dump.timer and cortex-dump-status.timer enabled'

# ---- R (kai's reboot test, 9 Nov; cox f7ef0231): the stack must come back after a reboot. compose.yml gives db and server restart: unless-stopped, but ROOTLESS podman restarts containers at boot only through the USER unit
# podman-restart.service (the distro's own: podman start --all --filter should-start-on-boot=true; its ExecStop, podman stop --service --all, keeps unless-stopped containers eligible: measured on podman 5.8.2), and that unit is DISABLED after an
# install: after any reboot Cortex was silently down until the installer was run again (no data lost). So the installer enables that unit (no unit of ours), after the stack is up and before the summary. Idempotent: an enabled unit is left alone and
# nothing is run. It FAILS CLOSED (EX_START): a silent down-after-reboot is the defect. systemctl --user needs the user bus, as the backup timers above do: in an ssh login XDG_RUNTIME_DIR is /run/user/<uid> and the user manager runs (lingering on);
# after a plain sudo -u it is unset and the call fails, which the message says. install_finished asks the same question, so a finished install without the unit is not finished and a run repairs it.
if ! restart_unit_persistent; then
  ru_seen() { case $restart_unit_state in ""|*[!abcdefghijklmnopqrstuvwxyz-]*) echo "no usable answer" ;; *) [ "${#restart_unit_state}" -le 32 ] && echo "$restart_unit_state" || echo "no usable answer" ;; esac; }   # the state word systemd gave, only when it is one plain word
  systemctl --user enable podman-restart.service || stop "$EX_START" restart_unit_failed "$(ru_seen)" "podman-restart.service could not be enabled (systemctl --user failed or the unit is missing); a restart after a reboot could not be confirmed"
  # the READBACK, with the same predicate (sam NMR-SAM-01): an enable that exits 0 can leave a runtime-only, static, indirect or masked unit, which does not survive a reboot
  restart_unit_persistent || stop "$EX_START" restart_unit_failed "$(ru_seen)" "podman-restart.service is not durably enabled after the enable; a restart after a reboot could not be confirmed"
  echo "install: enabled podman-restart.service: the stack comes back after a reboot."
fi

# ---- step 10: say what happened (the prerequisite file's say_done when the build has it; this file's two lines otherwise)
# say_done takes a FOURTH value, the install log, and refuses an empty one (defect N, kai's full pass 9 Nov: a good install ended with its usage problem line and no endpoint). This installer keeps NO log of its own (log_line
# writes only when CORTEX_LOG names a file, and a second run must change nothing), so the value is that file when CORTEX_LOG is set and the plain word none when it is not; say_done's own guard is not loosened.
report_strays
summary_finish
{ exit_handler_active=1; } 2>/dev/null; exit 0
