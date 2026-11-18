#!/bin/sh
# Cortex v2 installer for macOS (Docker Desktop and launchd): the Mac counterpart of deploy/install.sh. One command, safe to run twice. Run it with sh (sh install-mac.sh), not bash install-mac.sh.
#   mac/install-mac.sh --check   (looks at this Mac only: Docker Desktop, its memory, Compose and the Homebrew GNU tools; changes nothing)
#   mac/install-mac.sh --project <name> --lead <worker> [--home DIR] [--port N] [--no-start] [--release V (--from DIR | --base-url URL)]
#   mac/install-mac.sh --project <name> --lead <worker> [--home DIR] --reissue-token
# THE TRUST ANCHOR of THIS installer (hue's page TRUST-ANCHOR-LITERALS-hue-r1.md, W1-84): the three statements below are EMPTY in the tree (the development layout: a release run refuses before anything is
# fetched, fail closed) and the join step of the release build fills each with its value in single quotes: the principal, the public ed25519 key (68 characters) and its fingerprint (SHA256: and 43 characters).
# They stay the FIRST three statements, whole lines, each exactly once, in this order. Nothing of the environment is ever the anchor.
CORTEX_ANCHOR_PRINCIPAL=''
CORTEX_ANCHOR_KEY=''
CORTEX_ANCHOR_FPR=''
set -eu
#
# What it does on a Mac: looks at the machine (check-only: it changes nothing), writes three secret FILES (random values, a 0700 directory under the home, never printed and never
# replaced), copies the config example to $HOME_DIR/cortex.toml, LOADS the release's images (nothing is built) and starts the two containers on the cv2-s5-* names with docker compose
# (compose.yml and docker/compose.docker.yml, after a rendering check), waits (bounded) for both to be healthy, runs `cortex-server bootstrap` once to mint the first lead token (written
# inside the container, piped out into a 0600 file, then removed) and installs the two launchd backup agents. It never prints a secret or the token.
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
umask 077   # AFTER the removal (hale 6E-2): an exported umask function would have been what ran here before

project="" lead="" home_dir="${HOME:-/nonexistent}/cortex-v2" port="8601" start=1 reissue=0 without_backups=1   # u2 BY DEFAULT in the 0.2.2 Mac installer (kai 566fae3e, the CTO D172: no working Mac backup tools in this release): the install FINISHES, ONE warning and the closing lines say the backups are NOT scheduled. Only argv can turn the launchd step on (--with-backups): the shell imports every exported variable, so it is set here, before the arguments
SECRET_FILE_MODE=444   # DRAFT (vera or hue rule it): the secret files are made with this mode, inside a 0700 directory
usage() { sed -n '2,5p' "$0" | sed 's/^# \{0,1\}//'; }

# ---- the interface of the install lanes (INSTALL-LANES-INTERFACE-hue-r0.md and r1): exit-code symbols, say, warn, fail, the message catalogue, the joined files ----
EX_OK=0 EX_USAGE=1 EX_MACHINE=2 EX_PREREQ=3 EX_TRUST=4 EX_FETCH=5 EX_START=6 EX_WORKER=7 EX_BACKUP=8
CORTEX_YES=0 CORTEX_LOG=${CORTEX_LOG:-} check_only=0 CORTEX_RELEASE="" CORTEX_BASE_URL="" CORTEX_FROM=""
release_given=0 base_given=0 from_given=0
# The trust anchor is the three CORTEX_ANCHOR_* statements at the very top of this file (W1-84, vera V6E-03, hue r1): anchor_ok checks them, stage_make writes the key line from them.
log_line() { [ -z "$CORTEX_LOG" ] || printf '%s\n' "$1" >> "$CORTEX_LOG" || :; }
say() { say_rc=0; printf '%s\n' "$*" || say_rc=$?; log_line "$*"; return "$say_rc"; }   # the status of the PRINT, not of the log (marv F-7: a plan that could not be shown must not be consented to)
warn() { printf 'install: warning: %s\n' "$*" >&2; log_line "warning: $*"; }
fail() { fail_code=$1; shift; printf 'install: %s\n' "$*" >&2; log_line "install: $*"; exit "$fail_code"; }
die() { [ -z "${die_id:-}" ] || msg "$die_id" "${die_arg:-}" || :; fail "${die_code:-$EX_USAGE}" "$@"; }   # die_code, die_id and die_arg are set by stop (or on the line before a probed class line)
# msg ID [ARG]: the cause and the fix of one problem, from the lane that owns the id (prereq_msg, fetch_msg, then this file's inst_msg); 1 for an id nobody knows
msg() {
  for msg_table in prereq_msg fetch_msg inst_msg; do
    if command -v "$msg_table" >/dev/null 2>&1 && "$msg_table" "$@"; then return 0; fi
  done
  return 1
}
# the Homebrew formula that holds a GNU tool (kai b935583a (B)): the fix line of the seam startup check names it
gnu_formula() { case $1 in gfind) printf 'findutils' ;; gtar) printf 'gnu-tar' ;; *) printf 'coreutils' ;; esac; }
# this file's table: a "problem:" line and a "fix:" line on stderr; no secret, no command that deletes (IT-87 checks every id)
inst_msg() {
  m_again="then run this installer again: it continues where it stopped"
  case $1 in
    bad_name) m_cause="the ${2:-project} name is not allowed: it must start with a small letter, then only small letters, digits and hyphens, 32 characters at most"; m_fix="give --${2:-project} a name such as cortex-1 (a small letter first, then small letters, digits or hyphens, 32 characters at most) and run this installer again" ;;
    not_root) m_cause="this installer runs as your own user, not as root: Docker Desktop runs as you"; m_fix="run it again as a normal user" ;;
    python_missing) m_cause="python3 is not usable here: the release check reads the release's manifest with it${2:+ ($2)}"; m_fix="install the Command Line Tools with xcode-select --install (or a python3 of your own), then run this installer again" ;;
    docker_not_found) m_cause="Docker Desktop is not installed (no docker command was found)"; m_fix="install Docker Desktop for Mac from docker.com (this installer cannot install it for you), start it, then run this installer again" ;;
    stack_env_bad) m_cause="the release's stack.env cannot be used"; m_fix="fetch the release again; do not edit stack.env by hand" ;;
    not_healthy) m_cause="the database or the server did not become healthy in time${2:+ ($2)}"; m_fix="read the logs with docker logs cv2-s5-server and docker logs cv2-s5-db, fix what they show, $m_again" ;;
    token_left) m_cause="the first lead's sign-in file could not be made, and the partial file${2:+ $2} could not be removed: IT MAY STILL HOLD A VALID SIGN-IN TOKEN"; m_fix="remove ${2:-the partial file} by hand; if anyone else could have read it, replace the sign-in token: run this installer again with --reissue-token and the same --project and --lead" ;;
    token_failed) m_cause="the first lead's sign-in file could not be made${2:+ ($2)}; no sign-in file was written"; m_fix="read docker logs cv2-s5-server, $m_again" ;;
    token_exists) m_cause="a new sign-in file with this time stamp already exists"; m_fix="wait one second and run the same command again" ;;
    release_incomplete) m_cause="the release is missing a file the install needs${2:+: $2}"; m_fix="fetch the release again (do not copy files into it by hand), $m_again" ;;
    release_not_installed) m_cause="no release is installed in this home and none was asked for"; m_fix="run this installer with --release VERSION and --from DIR (or --base-url URL): the Mac installer installs a release, never a checkout" ;;
    step_status) m_cause="a step ended with a status the installer does not know${2:+ ($2)}"; m_fix="$m_again; if it repeats, report the status and the install log" ;;
    home_not_real) m_cause="the install home is not a real directory of yours${2:+: $2}"; m_fix="the installer never follows a link or changes a directory that is not yours: choose another --home or make a real directory yourself, $m_again" ;;
    trust_tool_missing) m_cause="a tool the release check needs is missing from this Mac${2:+: $2}"; m_fix="macOS ships it in /usr/bin: reinstall the Command Line Tools (xcode-select --install), then run this installer again: it does not install tools before the release is proven" ;;
    stage_parent) m_cause="there is no directory of yours to stage the release in${2:+ (near $2)}"; m_fix="give --home a path under a directory you own (a link, or a directory of someone else, is not used), then run this installer again" ;;
    port_differs) m_cause="the stack is already installed on another port${2:+ ($2)}"; m_fix="run this installer with the port the stack uses${2:+ (--port $2)}: it never reinstalls over a running stack" ;;
    anchor_unusable) m_cause="this installer carries no usable trust anchor to check a release against (an empty or damaged principal, key or fingerprint)"; m_fix="fetch the complete installer again from the release location (do not edit it)" ;;
    trust_missing) m_cause="this installer has no way to verify a release, so it will not unpack one"; m_fix="fetch the complete installer again from the release location (do not edit it)" ;;
    release_differs) m_cause="a different release is already installed in this home${2:+ ($2)}"; m_fix="use cortex upgrade to move between releases: this installer does not replace an installed release" ;;
    secret_unreadable) m_cause="a secret file of the stack exists but is not a plain file of yours holding 64 hex characters${2:+ ($2)}"; m_fix="the installer never deletes or replaces one: look at the file, and if you are sure that it is wrong remove that one file yourself, $m_again" ;;
    release_unverified) m_cause="the installed release could not be verified${2:+ ($2)}"; m_fix="fetch the release again with this installer (do not edit or copy files into it by hand): current must be the link to the verified releases directory, $m_again" ;;
    secret_create_failed) m_cause="a secret file of the stack could not be created${2:+ ($2)}"; m_fix="check the permissions and the free space under the install home, $m_again" ;;
    bad_path) m_cause="PATH holds an empty or a relative element (a command could then be taken from the current directory)"; m_fix="run the installer with a PATH of absolute directories only, for example PATH=/usr/bin:/bin:/usr/sbin:/sbin, $m_again" ;;
    bad_number) m_cause="${2:-a setting} is not a plain whole number of seconds"; m_fix="unset it or set it to a whole number (the poll from 1, the wait from 0, at most 99999, no sign, no fraction, no leading zero), $m_again" ;;
    no_random) m_cause="the machine did not give the random bytes the install needs for its credentials"; m_fix="check that /dev/urandom can be read and that head and od work, $m_again" ;;
    dir_symlink) m_cause="a directory the install needs is a symbolic link${2:+: $2}"; m_fix="move the link away and make a real directory (the installer never follows or removes it), $m_again" ;;
    cannot_write) m_cause="the install could not write${2:+ $2}"; m_fix="check the permissions and the free space there, $m_again" ;;
    config_modules) m_cause="the [modules] list in the install's cortex.toml is missing, empty, on several lines or holds an invalid id"; m_fix="edit cortex.toml in the install home so that [modules] has one line such as enabled = [\"records\"], $m_again" ;;
    compose_up_failed) m_cause="docker compose could not start the stack"; m_fix="read the compose output above and free what it names (a port or a name in use), $m_again; a volume is never deleted for you" ;;
    render_failed) m_cause="the Docker Desktop configuration failed its own check (the rendering gate)"; m_fix="read the output of the check above, which names what was weakened or is missing; do not edit the compose files by hand, $m_again" ;;
    db_unreadable) m_cause="the database's access rules could not be read"; m_fix="read docker logs cv2-s5-db, $m_again" ;;
    db_trust) m_cause="the database accepts connections without asking for a login (it was made before the auth settings)"; m_fix="this refusal changed nothing in the database and the installer deletes no volume: read S5-PEER-AUTH-MAP.md for the explicit step, then run this installer again (that is safe)" ;;
    roles_failed) m_cause="the database roles for the modules could not be made"; m_fix="read the message above and docker logs cv2-s5-db, $m_again" ;;
    shim_failed) m_cause="the private helper directory for the roles step could not be made${2:+ ($2)}"; m_fix="check the permissions and the free space under the install home, $m_again" ;;
    docker_path_unsafe) m_cause="the path of the docker command holds a character this installer does not pass on${2:+ ($2)}"; m_fix="use a Docker Desktop whose docker command is at a plain path (letters, digits and . _ / + - only), $m_again" ;;
    migrate_failed) m_cause="the database migrations failed${2:+ ($2)}; the server was not started"; m_fix="read docker logs cv2-s5-migrate, $m_again" ;;
    gnu_tool_missing) m_cause="the GNU tool ${2:-it needs} is not installed in Homebrew's directories (the release check needs GNU options that the tools of macOS do not have)"; m_fix="install it with: brew install $(gnu_formula "${2:-}"), then run this installer again: it does not install tools before the release is proven" ;;
    gnu_tool_not_gnu) m_cause="${2:-a tool} in Homebrew's directory does not say GNU in its --version text, and the release check would misread another tool's output"; m_fix="install the GNU tool with: brew install $(gnu_formula "${2:-}"), and make sure that no other program stands at that path, then run this installer again" ;;
    gnu_tool_unsafe) m_cause="a GNU tool that this installer is about to trust could be replaced by other users (${2:-it})"; m_fix="make it writable only by you and root (for example chmod o-w on the path named above, or reinstall it with: brew reinstall $(gnu_formula "${2%%:*}")), then run this installer again" ;;
    launchd_failed) m_cause="the backup agents could not be installed with launchd${2:+ ($2)}: the stack is running but the backups are NOT scheduled"; m_fix="read the message above (it comes from the launchd backup tool), $m_again" ;;
    *) return 1 ;;
  esac
  printf 'problem: %s\n' "$m_cause" >&2
  printf 'fix: %s\n' "$m_fix" >&2
}
# stop CODE ID ARG SENTENCE...: the catalogue's cause and fix for ID (ARG is an optional detail), then "install: SENTENCE" and the exit CODE (6e: every failure after the
# arguments uses it, so the code, the cause and the fix are decided in one place). The lines that scripts/probe-classes.sh takes out of this file VERBATIM keep a plain
# die call (the probe's stub knows only die): the code and id are set on the line before them.
stop() { die_code=$1; die_id=$2; die_arg=$3; shift 3; die "$@"; }
# hale 6E-4: a PATH with an empty or a relative element is refused BEFORE anything runs (an empty element means the current directory). The variables that make a program load or read something the caller
# chose (loader, Python, OpenSSL and its TLS certificate locations SSL_CERT_FILE and SSL_CERT_DIR, shell startup, and the docker and compose variables that name another daemon, context, configuration or project file) are
# unset for the whole run. docker is found ONCE, by absolute path, and every later call (this file's and the joined includes') goes to that path, even if a step changes PATH.
case ${PATH:-} in ""|:*|*:|*::*|[!/]*|*:[!/]*) stop "$EX_USAGE" bad_path "" "PATH holds an empty or a relative element" ;; esac
unset LD_PRELOAD LD_LIBRARY_PATH LD_AUDIT DYLD_INSERT_LIBRARIES DYLD_LIBRARY_PATH DYLD_FRAMEWORK_PATH DYLD_FALLBACK_LIBRARY_PATH DYLD_FALLBACK_FRAMEWORK_PATH PYTHONPATH PYTHONHOME PYTHONSTARTUP BASH_ENV ENV CDPATH OPENSSL_CONF OPENSSL_MODULES OPENSSL_ENGINES SSL_CERT_FILE SSL_CERT_DIR GCONV_PATH CURL_CA_BUNDLE SSLKEYLOGFILE CURL_SSL_BACKEND QLOGDIR DOCKER_HOST DOCKER_CONTEXT DOCKER_CONFIG DOCKER_CERT_PATH DOCKER_TLS_VERIFY DOCKER_API_VERSION COMPOSE_FILE COMPOSE_PROJECT_NAME COMPOSE_PROFILES COMPOSE_ENV_FILES
# hale O-2 (curl is started with -q, which shuts out its configuration file but not its environment): CURL_CA_BUNDLE (curl reads it as the --cacert value: the caller would choose what the release download trusts), SSLKEYLOGFILE (the file curl writes the session's TLS secrets to),
# QLOGDIR (the directory a curl built with HTTP/3 writes its logs to: this machine's curl has none, unset for the rule's sake) and CURL_SSL_BACKEND (the TLS backend of a curl built with more than one: measured no effect on this machine's curl, unset as a defence for another build)
# are unset for the whole run, with SSL_CERT_FILE and SSL_CERT_DIR in the list above. THE PROXY NAMES ARE NOT IN THIS LIST ON PURPOSE (kai, W1-232): only the release download runs without them (rel_fetch_noproxy), so that every later step keeps the caller's: docker pulls the database image of
# compose.yml through the operator's proxy (Docker Desktop's own proxy settings), and nobody decided to change that (the image is pinned by digest: a proxy cannot change its bytes). The release download uses no proxy and only the system trust store; what it brings is checked against the signed list afterwards, but it does not go through a proxy the caller chose.
# rel_fetch_noproxy runs rel_fetch in THIS shell with the eight names taken out and puts the caller's back afterwards, each one exactly as it was (set or not, empty or not): a subshell would hold a TERM or HUP until the fetch ended, and the Linux suite's IT-108 signal cases would fail (the same text, copied byte for byte from deploy/install.sh; the Mac's cell is IT-M24).
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
DOCKER_BIN=""
pin_docker() {   # docker by absolute path, ONCE
  [ -z "$DOCKER_BIN" ] || return 0
  DOCKER_BIN=$(command -v docker 2>/dev/null) || DOCKER_BIN=""
  case $DOCKER_BIN in /*) docker() { "$DOCKER_BIN" "$@"; } ;; *) DOCKER_BIN="" ;; esac
}
pin_docker
# THE OWN DIRECTORY BY THE SHELL ALONE (Q17d, vera's step 1b): no outside program runs before the included parts. here_dir is what dirname gives for $0: no slash is '.', the part before the last slash with the slashes at its end taken off ('x//y' is 'x', '/y' and '//y' are '/').
# cd and pwd are builtins; the supported Bash premise (compgen, cd, pwd) is the one of the exported-function removal above. The line that sets here keeps its start: the gate matrix replaces exactly that line.
case $0 in
  */*) here_dir=${0%/*}
       while case $here_dir in ?*/) true ;; *) false ;; esac; do here_dir=${here_dir%/}; done
       [ -n "$here_dir" ] || here_dir=/ ;;
  *) here_dir=. ;;
esac
here=$(CDPATH= cd -- "$here_dir" && pwd)
# Homebrew's GNU tools at TWO FIXED ABSOLUTE PATHS, never PATH (kai b935583a (B) (c1)): assigned HERE, whole line, before the first include is sourced, so the environment cannot name them (a test lane include reassigns them).
MAC_GNU_DIRS='/opt/homebrew/bin /usr/local/bin'
# The Mac's own stat, the same way (kai 7e00cb44): it looks at the GNU tools before they are trusted, so it is NOT one of the tools it looks at and is not named by the environment.
MAC_STAT='/usr/bin/stat'
# the files of the other install lanes: the Mac check-only prerequisite file and the SHARED fetch file, which is reused UNCHANGED (the release builder replaces each of these lines with the text of the file; the gate s5-deploy reads them in this exact form)
[ ! -f "$here/install-prereq.sh" ] || . "$here/install-prereq.sh"   # @include
[ ! -f "$here/install-fetch.sh" ] || . "$here/install-fetch.sh"   # @include

while [ $# -gt 0 ]; do
  case $1 in
    --project) [ $# -ge 2 ] || die "--project needs a value"; project=$2; shift 2 ;;
    --lead)    [ $# -ge 2 ] || die "--lead needs a value"; lead=$2; shift 2 ;;
    --home)    [ $# -ge 2 ] || die "--home needs a value"; home_dir=$2; shift 2 ;;
    --port)    [ $# -ge 2 ] || die "--port needs a value"; port=$2; shift 2 ;;
    --no-start) start=0; shift ;;
    --without-backups) without_backups=1; shift ;;   # the default since 0.2.2; kept so that an old command line still works
    --with-backups) without_backups=0; shift ;;   # argv only: schedules the two launch agents (scripts/launchd-backup), for a release whose backup tools work on a Mac (0.2.2 has none)
    --yes) CORTEX_YES=1; shift ;;
    --check) check_only=1; shift ;;
    --release) [ $# -ge 2 ] || die "--release needs a value"; CORTEX_RELEASE=$2; release_given=1; shift 2 ;;
    --base-url) [ $# -ge 2 ] || die "--base-url needs a value"; CORTEX_BASE_URL=$2; base_given=1; shift 2 ;;
    --from) [ $# -ge 2 ] || die "--from needs a value"; CORTEX_FROM=$2; from_given=1; shift 2 ;;
    --reissue-token) reissue=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) die "unknown argument: $1 (try --help)" ;;
  esac
done
[ "$check_only" -eq 1 ] || { [ -n "$project" ] && [ -n "$lead" ]; } || die "--project and --lead are required (try --help)"
case $port in *[!0123456789]*|"") die "--port must be a number" ;; esac
case $port in 8501|5499|5500|8765|8766) die "port $port belongs to the live v1 stack" ;; esac
case $home_dir in /*) ;; *) die "--home must be an absolute path" ;; esac
# The home is written into launchd agents (the backup tool's templates) and passed through sed: letters, digits and . _ / - only. A space, | & \ % $ or a quote or a newline would
# change an agent or the substitution. The class is spelled out (a range follows the locale).
case $home_dir in *[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._/-]*) die "--home may contain only letters, digits and . _ / - (it is written into launchd agents)" ;; esac
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
case ${HOME:-} in /*) ;; *) die "HOME must be an absolute path (the agents go to \$HOME/Library/LaunchAgents)" ;; esac
# ---- the compose vectors and the GNU tool seams -------------------------------------------------------------------------------------------------------
# The ONE way this file runs compose: docker compose with BOTH files, the base first (the Docker Desktop override is merged over it). Every call goes through here (IT-M05).
compose_run() { docker compose -f "$rel_dir/compose.yml" -f "$rel_dir/docker/compose.docker.yml" "$@"; }
# THE SEAMS (kai b935583a (B) (c1), cox 3a7fdab8): the shared fetch file calls its tools through rel_tool_* functions at fixed absolute paths of a Linux machine and passes them GNU-only options (find -printf,
# stat -c, ln -s -T, mv -T, sha256sum, the output of tar --numeric-owner -tvzf). They are redefined HERE, after the fetch file is sourced and only after the GNU tools proved themselves: the six GNU tools of
# Homebrew (coreutils, findutils, gnu-tar) at the two fixed directories of MAC_GNU_DIRS, the six plain tools of macOS in /bin (not /usr/bin: macOS keeps cat, cp, mkdir, rm, rmdir and chmod there), and
# docker for podman. A tool that is missing, or that does not say GNU in its --version text, is a REFUSAL naming it and the brew command (exit 3) BEFORE anything is fetched or unpacked: a BSD tool
# behind a GNU-option seam must never run silently. The environment cannot name a directory or a tool (the paths are assigned above, and the variables below are always assigned by the check).
# NOT PROVEN from here (ASK-A-MAC, NOT RUN until M-6): the g-tools at those paths, their behaviour, and the paths of /bin.
MAC_GNU_TOOLS="gsha256sum gfind gstat gtar gmv gln"
mg_sha="" mg_find="" mg_stat="" mg_tar="" mg_mv="" mg_ln=""
gnu_find_tool() {   # gnu_find_tool NAME: gt_path is the FIRST directory of MAC_GNU_DIRS that holds NAME as an executable file (1 when none does)
  gt_path=""
  for gt_d in $MAC_GNU_DIRS; do
    if [ -f "$gt_d/$1" ] && [ -x "$gt_d/$1" ]; then gt_path="$gt_d/$1"; return 0; fi
  done
  return 1
}
# gnu_resolve PATH: the real file behind symbolic links (Homebrew's bin entries link into the Cellar; BSD readlink has no -f): at most 20 links, a relative target is taken from the link's own directory
gnu_resolve() {
  gr_p=$1; gr_n=0
  while [ -L "$gr_p" ]; do
    gr_n=$((gr_n + 1)); [ "$gr_n" -le 20 ] || return 1
    gr_t=$(/usr/bin/readlink "$gr_p") || return 1
    case $gr_t in /*) gr_p=$gr_t ;; *) gr_p="${gr_p%/*}/$gr_t" ;; esac
  done
  printf '%s\n' "$gr_p"
}
# gnu_trust_ok TOOL PATH (kai 7e00cb44, cox 3a7fdab8): the directory named in MAC_GNU_DIRS, the RESOLVED real file and its physical directory must not be replaceable by others: refused when WORLD-WRITABLE or owned by a user who is neither
# root nor the invoker. GROUP-writable (Homebrew's normal state for the admin group) is NOT refused: a weaker trust root than /usr/bin, named in the packet (ASK-A-MAC, NOT RUN). The Mac's own stat does the looking (MAC_STAT, BSD format
# '%u %Lp'), never the GNU seam; a path that cannot be inspected is a refusal. The tool is not run before it has passed.
gnu_trust_ok() {
  gtu_t=$1; gtu_f=$2
  gtu_me=$(id -u) || stop "$EX_PREREQ" gnu_tool_unsafe "$gtu_t: the invoking user could not be read" "the GNU tool $gtu_t is not trusted"
  gtu_real=$(gnu_resolve "$gtu_f") || stop "$EX_PREREQ" gnu_tool_unsafe "$gtu_t: $gtu_f could not be resolved (a link loop, or more than 20 links)" "the GNU tool $gtu_t is not trusted"
  gtu_rd=$(CDPATH= cd -P -- "${gtu_real%/*}/" 2>/dev/null && pwd -P) || stop "$EX_PREREQ" gnu_tool_unsafe "$gtu_t: the directory of $gtu_real could not be read" "the GNU tool $gtu_t is not trusted"
  for gtu_p in "${gtu_f%/*}" "$gtu_real" "$gtu_rd"; do
    gtu_s=$("$MAC_STAT" -f '%u %Lp' -- "$gtu_p" 2>/dev/null) || stop "$EX_PREREQ" gnu_tool_unsafe "$gtu_t: $gtu_p could not be inspected" "the GNU tool $gtu_t is not trusted"
    gtu_u=${gtu_s%% *}; gtu_m=${gtu_s#* }
    case $gtu_u in ""|*[!0123456789]*) stop "$EX_PREREQ" gnu_tool_unsafe "$gtu_t: the owner of $gtu_p could not be read" "the GNU tool $gtu_t is not trusted" ;; esac
    case $gtu_m in ""|*[!01234567]*) stop "$EX_PREREQ" gnu_tool_unsafe "$gtu_t: the mode of $gtu_p could not be read" "the GNU tool $gtu_t is not trusted" ;; esac
    { [ "$gtu_u" = 0 ] || [ "$gtu_u" = "$gtu_me" ]; } || stop "$EX_PREREQ" gnu_tool_unsafe "$gtu_t: $gtu_p is owned by user $gtu_u, who is neither root nor you" "the GNU tool $gtu_t is not trusted"
    gtu_m=000$gtu_m; gtu_m=${gtu_m#"${gtu_m%???}"}
    case ${gtu_m#??} in 2|3|6|7) stop "$EX_PREREQ" gnu_tool_unsafe "$gtu_t: $gtu_p is writable by everyone" "the GNU tool $gtu_t is not trusted" ;; esac
  done
}
gnu_seams_arm() {
  for gs_t in $MAC_GNU_TOOLS; do
    gnu_find_tool "$gs_t" || stop "$EX_PREREQ" gnu_tool_missing "$gs_t" "the GNU tool $gs_t is not in $MAC_GNU_DIRS"
    gnu_trust_ok "$gs_t" "$gt_path"
    gs_v=$("$gt_path" --version 2>/dev/null | sed -n 1p) || gs_v=""
    case $gs_v in *GNU*) ;; *) stop "$EX_PREREQ" gnu_tool_not_gnu "$gs_t" "$gt_path does not say GNU in its --version text" ;; esac
    case $gs_t in gsha256sum) mg_sha=$gt_path ;; gfind) mg_find=$gt_path ;; gstat) mg_stat=$gt_path ;; gtar) mg_tar=$gt_path ;; gmv) mg_mv=$gt_path ;; gln) mg_ln=$gt_path ;; esac
  done
  rel_tool_sha256sum() { "$mg_sha" "$@"; }
  rel_tool_find() { "$mg_find" "$@"; }
  rel_tool_stat() { "$mg_stat" "$@"; }
  rel_tool_tar() { [ -z "${TAR_OPTIONS+x}" ] || return 1; "$mg_tar" "$@"; }
  rel_tool_mv() { "$mg_mv" "$@"; }
  rel_tool_ln() { "$mg_ln" "$@"; }
  rel_tool_cat() { /bin/cat "$@"; }
  rel_tool_cp() { /bin/cp "$@"; }
  rel_tool_mkdir() { /bin/mkdir "$@"; }
  rel_tool_rm() { /bin/rm "$@"; }
  rel_tool_rmdir() { /bin/rmdir "$@"; }
  rel_tool_chmod() { /bin/chmod "$@"; }
  rel_tool_podman() { [ -z "${DOCKER_HOST+x}${DOCKER_CONTEXT+x}${DOCKER_CONFIG+x}${DOCKER_CERT_PATH+x}${DOCKER_TLS_VERIFY+x}${DOCKER_API_VERSION+x}" ] || return 1; [ -n "$DOCKER_BIN" ] || return 1; "$DOCKER_BIN" "$@"; }
}
# py_usable: python3 is used only when it is a real one: on a Mac without the Command Line Tools /usr/bin/python3 is a stub that opens an install dialog
py_usable() {
  pu_p=$(command -v python3 2>/dev/null) || return 1
  [ -n "$pu_p" ] || return 1
  if [ "$pu_p" = /usr/bin/python3 ] && [ "$(uname -s)" = Darwin ] && ! xcode-select -p >/dev/null 2>&1; then return 1; fi
  return 0
}
# mode_of PATH: the permission bits of PATH as three octal digits (such as 700), read from ls -ld (the same on Linux and on macOS: no stat, whose options differ); 1 for a path that cannot be read and for a special bit
mode_of() {
  mo_l=$(LC_ALL=C ls -ld -- "$1" 2>/dev/null) || return 1
  mo_s=${mo_l%% *}; mo_t=${mo_s#?}; mo_n=""
  for mo_i in 1 2 3; do
    mo_x=$(printf '%.3s' "$mo_t"); mo_t=${mo_t#???}
    case $mo_x in ---) mo_d=0 ;; --x) mo_d=1 ;; -w-) mo_d=2 ;; -wx) mo_d=3 ;; r--) mo_d=4 ;; r-x) mo_d=5 ;; rw-) mo_d=6 ;; rwx) mo_d=7 ;; *) return 1 ;; esac
    mo_n=$mo_n$mo_d
  done
  printf '%s\n' "$mo_n"
}
# --check is the machine look (the prerequisite file's machine_look) and the check of the GNU tools: it changes nothing and returns its status
if [ "$check_only" -eq 1 ]; then
  command -v machine_look >/dev/null 2>&1 || die "--check is not available in this build (the machine look is not part of it)"
  machine_look   # bare (hale 6E3-1): errexit stays on inside it; its status ends the shell as it is
  gnu_seams_arm
  exit 0
fi

# Bounded waiting: a fresh start may need to build; a reissue only needs a stack that is already up.
if [ "$reissue" -eq 1 ]; then wait_max=${CORTEX_INSTALL_WAIT:-30}; else wait_max=${CORTEX_INSTALL_WAIT:-180}; fi
poll=${CORTEX_INSTALL_POLL:-5}
# Both reach shell arithmetic, and bash runs a command substitution that sits inside an arithmetic operand (a value such as HOME[$(...)]): each is a plain whole number or the
# install stops here, before anything is looked at or started (kai 5241a6a1, nix's G12 inventory). Digits only, no leading zero (octal), at most 5 digits; a poll of 0 would never end.
valid_secs() { case $1 in ""|*[!0123456789]*|0?*) return 1 ;; esac; [ "${#1}" -le 5 ]; }
valid_secs "$wait_max" || stop "$EX_USAGE" bad_number CORTEX_INSTALL_WAIT "CORTEX_INSTALL_WAIT must be a whole number of seconds from 0 to 99999"
{ valid_secs "$poll" && [ "$poll" != 0 ]; } || stop "$EX_USAGE" bad_number CORTEX_INSTALL_POLL "CORTEX_INSTALL_POLL must be a whole number of seconds from 1 to 99999"

wait_healthy() {   # wait_healthy <container>...
  waited=0
  while :; do
    bad=""
    for c in "$@"; do
      st=$(docker inspect --format '{{.State.Health.Status}}' "$c" 2>/dev/null || echo missing)
      [ "$st" = "healthy" ] || bad="$bad $c=$st"
    done
    [ -z "$bad" ] && return 0
    [ "$waited" -lt "$wait_max" ] || stop "$EX_START" not_healthy "$bad" "not healthy after $wait_max s:$bad (see: docker logs cv2-s5-server)"
    sleep "$poll"; waited=$((waited + poll))
  done
}

# the path of the token INSIDE the Linux container (docker exec runs there, not on the Mac)
token_in_container=/run/cortex/bootstrap.token   # portable: ok the path is inside the Linux container that docker exec enters, not on this Mac
# bootstrap_call: a first run CREATES the project and its lead; a reissue must not, so it passes the server's refuse-if-missing
# flag. Two literal command lines, on purpose: nothing a caller sets can change the flag, and there is no fallback to the
# creating form if the server does not know it. The server's own stderr line is left to reach the terminal.
bootstrap_call() {
  if [ "$reissue" -eq 1 ]; then
    docker exec cv2-s5-server /usr/local/bin/cortex-server bootstrap --project "$project" --lead "$lead" --token-file "$token_in_container" --require-existing
  else
    docker exec cv2-s5-server /usr/local/bin/cortex-server bootstrap --project "$project" --lead "$lead" --token-file "$token_in_container"
  fi
}

# token_copy_drop: remove the temporary copy of the token in the server container (its tmpfs). A removal that FAILS is a WARNING that names it, never silent (cox ccd83501): the copy lasts only as long as the container.
token_copy_drop() {
  docker exec cv2-s5-server rm -f "$token_in_container" >/dev/null 2>&1 || warn "the temporary copy of the token in the server container could not be removed (it lasts only as long as the container)"
}
# mint_token <dest>: run bootstrap in the server, pipe the token from its tmpfs into <dest> (0600, via a .part file made
# under umask 077), remove the tmpfs copy. A failure leaves no file and no secret behind.
mint_token() {
  dest=$1; part="$dest.part"
  rm -f "$part" || stop "$EX_WORKER" cannot_write "$part" "cannot remove the old $part"
  docker exec cv2-s5-server rm -f "$token_in_container" >/dev/null 2>&1 || true   # the PRE-CLEAN of a stale copy from a crashed run, masked ON PURPOSE (cox ccd83501): rm -f of nothing is success, and a container that cannot be reached fails the bootstrap call next with its own cause
  reason=""
  if ! bootstrap_call; then
    reason="bootstrap failed"
  elif ! docker exec cv2-s5-server cat "$token_in_container" > "$part"; then
    reason="the token copy failed"
  elif [ ! -s "$part" ]; then
    reason="the token copy was empty"
  fi
  if [ -n "$reason" ]; then
    token_copy_drop   # cox ccd83501: FIRST, so a partial file that cannot be removed (the token_left stop just below) no longer leaves the container copy behind
    rm -f "$part" || stop "$EX_WORKER" token_left "$part" "$reason; the partial file $part could not be removed (remove it by hand)"   # sam I6E-SAM01: a failing removal ends in exit 7 with its cause and a fix that NAMES the leftover
    stop "$EX_WORKER" token_failed "$reason" "$reason; no token file was written"
  fi
  chmod 600 "$part" || { token_copy_drop; rm -f "$part" || stop "$EX_WORKER" token_left "$part" "the token file could not be made private; the partial file $part could not be removed (remove it by hand)"; stop "$EX_WORKER" token_failed "the file could not be made private" "the token file could not be made private; no token file was written"; }
  mv "$part" "$dest" || { token_copy_drop; rm -f "$part" || stop "$EX_WORKER" token_left "$part" "the token file could not be moved into place; the partial file $part could not be removed (remove it by hand)"; stop "$EX_WORKER" token_failed "the file could not be moved into place" "the token file could not be moved into place; no token file was written"; }
  # the temporary copy in the container's tmpfs: its removal failing after the token is published is a WARNING (it lives only as long as the container), never a lost install
  token_copy_drop
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
  [ ! -d "$home_dir" ] || [ -O "$home_dir" ] || stop "$EX_MACHINE" home_not_real "$home_dir" "$home_dir is not a directory of yours"
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
  for rs_ok in $rs_allowed; do [ "$rs_rc" -ne "$rs_ok" ] || exit "$rs_rc"; done
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
      bin|cortex.toml|current|releases|secrets|dumps|dump-status|"$lead.token"|"$lead".token.[0123456789]*) ;;
      *) rs_list="$rs_list $rs_b" ;;
    esac
  done
  [ -z "$rs_list" ] || say "install: left alone (not part of the install, never removed):$rs_list"
}
# install_finished: 0 only if the MACHINE shows a finished install of this home and lead (W1-81 (a)). Each check VERIFIES (a length, a mode, an owner, a checksum, an answer), and a check
# that cannot run (a tool missing, a file unreadable, a command refused) is NOT a pass: the whole sequence then runs again, and every step of it is idempotent. No state file is read.
# On a Mac the secrets are FILES, the backups are two launch agents (the backup tool's own status says so, and the check is skipped for a --without-backups run), and only a release is installed.
install_finished() {
  inf_uid=$(id -u) || return 1
  { [ ! -L "$home_dir" ] && [ -d "$home_dir" ] && [ -O "$home_dir" ]; } || return 1
  { command -v docker >/dev/null 2>&1 && command -v curl >/dev/null 2>&1; } || return 1
  inf_tok="$home_dir/$lead.token"
  { [ -f "$inf_tok" ] && [ ! -L "$inf_tok" ] && [ -s "$inf_tok" ] && [ -O "$inf_tok" ]; } || return 1
  case $(mode_of "$inf_tok") in 600|400) ;; *) return 1 ;; esac
  { [ -f "$home_dir/cortex.toml" ] && [ -s "$home_dir/cortex.toml" ]; } || return 1
  { [ -d "$home_dir/secrets" ] && [ ! -L "$home_dir/secrets" ] && [ -O "$home_dir/secrets" ] && [ "$(mode_of "$home_dir/secrets")" = 700 ]; } || return 1
  for inf_s in db-superuser owner-password app-password; do
    inf_f="$home_dir/secrets/$inf_s"
    { [ -f "$inf_f" ] && [ ! -L "$inf_f" ] && [ -O "$inf_f" ] && [ "$(mode_of "$inf_f")" = "$SECRET_FILE_MODE" ]; } || return 1
    inf_v=$(cat "$inf_f" 2>/dev/null) || return 1
    case $inf_v in ""|*[!0123456789abcdef]*) return 1 ;; esac
    [ "${#inf_v}" -eq 64 ] || return 1
    inf_v=""
  done
  for inf_c in cv2-s5-db cv2-s5-server; do
    [ "$(docker inspect --format '{{.State.Health.Status}}' "$inf_c" 2>/dev/null)" = healthy ] || return 1
  done
  [ "$(docker inspect --format '{{.State.Status}} {{.State.ExitCode}}' cv2-s5-migrate 2>/dev/null)" = "exited 0" ] || return 1
  [ -f "$home_dir/current/stack.env" ] || return 1
  read_stack_env "$home_dir/current/stack.env" || return 1
  { [ -z "$CORTEX_RELEASE" ] || [ "$sv" = "$CORTEX_RELEASE" ]; } || return 1
  release_verified "$sv" quiet || return 1
  inf_rel=$sv; inf_dir="$home_dir/current"; inf_scripts="$inf_dir/bin"
  inf_doc=$(curl --silent --show-error --fail --noproxy '*' --max-time 5 "http://127.0.0.1:$port/v2/health" 2>/dev/null) || return 1
  inf_flat=$(printf '%s' "$inf_doc" | tr -d ' \n\r\t') || return 1
  case $inf_flat in *'"ready":true'*) ;; *) return 1 ;; esac
  case $inf_flat in *"\"version\":\"$inf_rel\""*) ;; *) return 1 ;; esac
  for inf_d in bin dumps dump-status; do { [ -d "$home_dir/$inf_d" ] && [ ! -L "$home_dir/$inf_d" ]; } || return 1; done
  for inf_f in $backup_scripts; do
    [ "$without_backups" -eq 0 ] || [ -f "$inf_scripts/$inf_f" ] || continue
    { [ -f "$inf_scripts/$inf_f" ] && [ -f "$home_dir/bin/$inf_f" ] && [ ! -L "$home_dir/bin/$inf_f" ]; } || return 1
    [ "$(cksum < "$inf_scripts/$inf_f")" = "$(cksum < "$home_dir/bin/$inf_f")" ] || return 1
    [ "$(mode_of "$home_dir/bin/$inf_f")" = 755 ] || return 1
  done
  if [ "$without_backups" -eq 0 ]; then
    { [ -f "$home_dir/bin/launchd-run" ] && [ ! -L "$home_dir/bin/launchd-run" ] && [ "$(mode_of "$home_dir/bin/launchd-run")" = 755 ]; } || return 1
    "$here/../scripts/launchd-backup" status >/dev/null 2>&1 || return 1
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
stage_made="" key_made="" shim_made=""
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
  if [ -n "$shim_made" ]; then
    case ${shim_made##*/} in .cortex-install-shim.????????) { [ -d "$shim_made" ] && [ ! -L "$shim_made" ]; } && rm -rf -- "$shim_made" || : ;; esac
    shim_made=""
  fi
  anchor_remove
}
# The two 6b wrappers (machine_look, prereq_run) are called BARE (hale 6E-1): inside a tested context (|| && ! if) errexit is OFF in the whole body, so an unchecked failing command would go on. The status they
# end the shell with is mapped HERE, in the EXIT handler, while cur_step names the step: a status the step may give passes (the lane printed its own cause and fix), a signal exit passes, any other becomes the
# step's own code with one cause and fix of ours. A lane may REPLACE the traps (marv's prereq_requalify_compose does): arm_traps puts ours back after each step.
cur_step="" cur_default=0 cur_allowed=""
exit_handler() {
  eh_st=$?
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
  exit "$eh_st"
}
arm_traps() {
  trap exit_handler EXIT
  trap 'exit 129' HUP
  trap 'exit 130' INT
  trap 'exit 143' TERM
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
  { [ -d "$sp" ] && [ ! -L "$sp" ] && [ -O "$sp" ] && [ -w "$sp" ]; } || return 1
  printf '%s\n' "$sp"
}
# stage_leftovers PARENT: a leftover is a real directory of the user's with mode 0700 and NOTHING in it changed for 30 minutes (a live run of another install keeps touching its own): removed by
# its exact path and said. Anything else of that name (a link, a file, another mode, a young one) is KEPT and named.
stage_leftovers() {
  sl_keep=""
  for sl_d in "$1"/.cortex-install-stage.*; do
    { [ -e "$sl_d" ] || [ -L "$sl_d" ]; } || continue
    if [ -d "$sl_d" ] && [ ! -L "$sl_d" ] && [ -O "$sl_d" ] && [ "$(mode_of "$sl_d")" = 700 ] && [ -z "$(find "$sl_d" -mmin -30 2>/dev/null | sed -n 1p)" ]; then
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
# trust_tools_ok: step 3, read-only. The tools of macOS the release check itself needs: gzip and ssh-keygen (sha256sum and tar are the GNU tools that the seam check above proved), and curl for the online channel only. A missing one
# stops here with the exact command (EX_MACHINE): nothing was fetched, nothing changed, and the installer does not install these before the release is proven.
trust_tools_ok() {
  for tt in gzip ssh-keygen; do
    command -v "$tt" >/dev/null 2>&1 || stop "$EX_MACHINE" trust_tool_missing "$tt" "the tool $tt is missing, and the release cannot be checked without it"
  done
  [ -n "$CORTEX_FROM" ] || command -v curl >/dev/null 2>&1 || stop "$EX_MACHINE" trust_tool_missing curl "the tool curl is missing, and an online release cannot be fetched without it"
}

# ---- the sequence ----------------------------------------------------------------------------------------------------------------------------------------------------------------
[ "$(id -u)" -ne 0 ] || stop "$EX_MACHINE" not_root "" "run as a normal user: Docker Desktop runs as you"
# A FINISHED install (every check of install_finished passes) is recognised FIRST, before steps 1 and 2: the machine look would call our own running stack on the port a blocking finding, and
# a second run changes nothing (G5). It says what is installed and exits 0. Anything else runs the whole sequence below, which is idempotent step by step. A reissue is never "finished".
# THE FETCH FILE (cox bfa67e83 (C)): a RELEASE was asked for (--release, --from or --base-url) but the shared fetch file is not part of this build (a checkout has none beside mac/): FAIL CLOSED, BEFORE anything is looked at, fetched or unpacked.
if { [ "$release_given" -eq 1 ] || [ "$from_given" -eq 1 ] || [ "$base_given" -eq 1 ]; } && ! command -v rel_fetch >/dev/null 2>&1; then
  stop "$EX_TRUST" trust_missing "" "a release was asked for (--release, --from or --base-url) but the release fetch and verification code is not part of this build"
fi
# THE SEAMS FIRST (kai b935583a (B)): when the shared fetch file is joined, its tools are checked, and redefined, BEFORE anything of it runs (the finished check below verifies a release with them).
if command -v rel_fetch >/dev/null 2>&1; then gnu_seams_arm; fi
# THE RELEASE CHECK READS THE MANIFEST WITH python3 (verify_release_dir; "unknown is not success"): on a Mac without the Command Line Tools /usr/bin/python3 is a stub that opens an install dialog, so it is judged HERE, before anything of the release is read or fetched
MAC_PYTHON=""
if [ "$reissue" -eq 0 ] && command -v verify_release_dir >/dev/null 2>&1; then
  py_usable || stop "$EX_PREREQ" python_missing "" "python3 is not usable here (on a Mac it comes with the Command Line Tools): the release check cannot run"
  MAC_PYTHON=$(command -v python3)
fi
if [ "$reissue" -eq 0 ] && install_finished; then
  say "install: already installed and healthy in $home_dir; nothing was changed."
  report_strays
  if command -v say_done >/dev/null 2>&1; then
    say_done "${inf_rel:-${CORTEX_VERSION:-unknown}}" "http://127.0.0.1:$port/v2/health/ready" "$home_dir/$lead.token" "${CORTEX_LOG:-none}" || :
  else
    echo "install: done. Server http://127.0.0.1:$port/v2/health/ready"
    echo "install: token file $home_dir/$lead.token (mode 600). Point a worker at it with CORTEX_TOKEN_FILE=$home_dir/$lead.token"
  fi
  exit 0
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
if [ "$reissue" -eq 0 ] && command -v docker >/dev/null 2>&1 && docker container inspect cv2-s5-server >/dev/null 2>&1; then
  ip=$(docker port cv2-s5-server 8601/tcp 2>/dev/null | sed -n 's/^.*://p' | sed -n 1p) || ip=""
  case $ip in ""|*[!0123456789]*) ;; *) [ "$ip" = "$port" ] || stop "$EX_USAGE" port_differs "$ip" "the stack is already installed on port $ip, not $port" ;; esac
fi
# A release to install: --release V, the fetch functions joined, and none installed in this home (an installed one of that version is not fetched again).
need_fetch=0
if [ "$reissue" -eq 0 ] && [ -n "$CORTEX_RELEASE" ] && command -v rel_fetch >/dev/null 2>&1; then
  valid_semver "$CORTEX_RELEASE" || stop "$EX_USAGE" step_status "release" "the release $CORTEX_RELEASE is not a SemVer"
  [ -f "$home_dir/current/stack.env" ] || need_fetch=1
fi
# The order of W1-85 on a Mac: 0 the GNU tools behind the seams (above, when the fetch file is joined), 1 the machine look (check-only: it changes nothing), 2 the trust tools, 3 fetch and trust in the stage, 4 unpack and load.
# A reissue needs none of them. A fetch that cannot be verified is not made (FAIL CLOSED: no rel_trust, rel_unpack or rel_load = no fetch).
if [ "$reissue" -eq 0 ]; then
  if command -v machine_look >/dev/null 2>&1; then
    cur_step=machine_look cur_default=$EX_MACHINE cur_allowed="$EX_MACHINE $EX_PREREQ"   # marv's r4: a failed diagnostic output returns EX_PREREQ, which passes through
    machine_look
    cur_step=""; arm_traps; PATH=$SAFE_PATH; export PATH
  fi
  if [ "$need_fetch" -eq 1 ]; then
    { command -v rel_trust >/dev/null 2>&1 && command -v rel_unpack >/dev/null 2>&1 && command -v rel_load >/dev/null 2>&1; } || stop "$EX_TRUST" trust_missing "" "this build has no rel_trust, rel_unpack or rel_load: it cannot verify or unpack a release"
    anchor_ok || stop "$EX_TRUST" anchor_unusable "" "this installer carries no usable trust anchor, so it will not fetch a release"
    trust_tools_ok
    stage_make
    run_step "$EX_FETCH" "$EX_FETCH" rel_fetch_noproxy "$CORTEX_RELEASE" "$CORTEX_STAGE_DIR"
    run_step "$EX_TRUST" "$EX_TRUST $EX_FETCH" rel_trust "$CORTEX_RELEASE" "$CORTEX_STAGE_DIR" "$key_file" "$CORTEX_ANCHOR_PRINCIPAL" "$CORTEX_ANCHOR_FPR"
    anchor_remove
  fi
fi
[ -n "$DOCKER_BIN" ] || stop "$EX_PREREQ" docker_not_found "" "Docker Desktop is not installed"   # (the machine look says so first; this stands for a build whose look is replaced)
if [ "$need_fetch" -eq 1 ]; then
  home_make
  run_step "$EX_FETCH" "$EX_FETCH $EX_TRUST" rel_unpack "$CORTEX_RELEASE" "$CORTEX_STAGE_DIR" "$home_dir"
  run_step "$EX_FETCH" "$EX_FETCH" rel_load "$CORTEX_RELEASE" "$CORTEX_STAGE_DIR" "$home_dir"
  stage_cleanup
  [ -f "$home_dir/current/stack.env" ] || stop "$EX_FETCH" release_incomplete "$home_dir/current/stack.env" "the release was not laid down: $home_dir/current/stack.env is missing"
fi
[ -f "$home_dir/current/stack.env" ] || stop "$EX_FETCH" release_not_installed "" "no release is installed in $home_dir and none was asked for (--release)"
release_mode=1; rel_dir="$home_dir/current"
read_stack_env "$rel_dir/stack.env" || { msg stack_env_bad || :; fail "$EX_FETCH" "the release at $rel_dir has a stack.env that cannot be used ($STACK_WHY); nothing was started"; }
CORTEX_VERSION=$sv; CORTEX_SERVER_IMAGE=$ss; CORTEX_DB_IMAGE=$sd; export CORTEX_VERSION CORTEX_SERVER_IMAGE CORTEX_DB_IMAGE
[ "$reissue" -eq 1 ] || release_verified "$sv" || stop "$EX_FETCH" release_unverified "$sv" "the installed release $sv could not be verified; nothing was started"

# ---- preflight: the backup scripts (piece 12, bob) must be there before anything is made or started: a release carries them in bin/.
scripts_src="$rel_dir/bin"
if [ "$reissue" -eq 0 ] && [ "$without_backups" -eq 0 ]; then
  for f in $backup_scripts; do [ -f "$scripts_src/$f" ] || stop "$EX_FETCH" release_incomplete "$f in $scripts_src" "backup script $f is missing in $scripts_src"; done
fi

home_make

# ---- reissue: only a new token for an existing, healthy stack -----------------------------------------------------
if [ "$reissue" -eq 1 ]; then
  wait_healthy cv2-s5-db cv2-s5-server
  new="$home_dir/$lead.token.$(date -u +%Y%m%dT%H%M%SZ)"
  [ ! -e "$new" ] || stop "$EX_WORKER" token_exists "" "$new already exists; try again in a second"
  mint_token "$new"
  echo "install: new token file $new (mode 600). The old token is superseded: update CORTEX_TOKEN_FILE to this path."
  exit 0
fi

# ---- secrets: random values into FILES under <home>/secrets (bob's M-1b: Docker Compose reads them as file secrets; the override names db-superuser, owner-password and app-password) ----------------
# A 0700 directory of yours; each file 64 hex characters, made under umask 077 by a temporary name and a rename; the MODE is a DRAFT (vera's or hue's decision): Compose ignores uid, gid and mode of a FILE secret, so the container
# sees the host file's own mode, and the server's and the database's users must be able to read it. An existing file is VERIFIED (a plain file of yours, 64 hex characters), never replaced and never deleted.
secrets_dir="$home_dir/secrets"
[ ! -L "$secrets_dir" ] || stop "$EX_START" dir_symlink "$secrets_dir" "$secrets_dir is a symbolic link; it must be a real directory (a link could point anywhere)"
[ -d "$secrets_dir" ] || { mkdir "$secrets_dir" && chmod 0700 "$secrets_dir"; } || stop "$EX_START" cannot_write "$secrets_dir" "cannot create $secrets_dir"
{ [ -O "$secrets_dir" ] && [ "$(mode_of "$secrets_dir")" = 700 ]; } || stop "$EX_START" secret_unreadable "$secrets_dir" "$secrets_dir is not a 0700 directory of yours"
make_secret() {   # make_secret NAME
  ms_f="$secrets_dir/$1"
  if [ -e "$ms_f" ] || [ -L "$ms_f" ]; then
    { [ -f "$ms_f" ] && [ ! -L "$ms_f" ] && [ -O "$ms_f" ]; } || stop "$EX_START" secret_unreadable "$1" "the secret $1 exists but is not a plain file of yours"
    ms_v=$(cat "$ms_f" 2>/dev/null) || ms_v=""
    case $ms_v in ""|*[!0123456789abcdef]*) ms_v=""; stop "$EX_START" secret_unreadable "$1" "the secret $1 exists but is empty or could not be read" ;; esac
    [ "${#ms_v}" -eq 64 ] || { ms_v=""; stop "$EX_START" secret_unreadable "$1" "the secret $1 exists but is not 64 characters"; }
    ms_v=""
    return 0
  fi
  die_code=$EX_START die_id=no_random die_arg=
  # the pipeline hides a failing head, so the value is checked (64 hex characters) before it becomes a file
  val=$(head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n')
  case $val in *[!0123456789abcdef]*) val=""; die "could not read random bytes for secret $1" ;; esac
  [ "${#val}" -eq 64 ] || { val=""; die "could not read random bytes for secret $1"; }
  rm -f "$ms_f.new" || { val=""; stop "$EX_START" secret_create_failed "$1" "cannot remove the old $ms_f.new"; }
  { printf '%s' "$val" > "$ms_f.new" && chmod "$SECRET_FILE_MODE" "$ms_f.new" && mv "$ms_f.new" "$ms_f"; } || { val=""; rm -f "$ms_f.new" || :; stop "$EX_START" secret_create_failed "$1" "the secret file $1 could not be written"; }
  val=""
}
make_secret db-superuser
make_secret owner-password
make_secret app-password

# ---- config: mounted, never baked --------------------------------------------------------------------------------
[ -f "$home_dir/cortex.toml" ] || { cp "$rel_dir/cortex.toml.example" "$home_dir/cortex.toml" && chmod 644 "$home_dir/cortex.toml"; } || stop "$EX_START" cannot_write "$home_dir/cortex.toml" "cannot write the config $home_dir/cortex.toml"
CORTEX_CONFIG="$home_dir/cortex.toml"; export CORTEX_CONFIG
CORTEX_PORT=$port; export CORTEX_PORT
CORTEX_SECRETS_DIR=$secrets_dir; export CORTEX_SECRETS_DIR   # the Docker Desktop override reads the three file secrets from here

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
# `cortex-server --version` shows the release (a release has no checkout to name): nothing from the environment.
CORTEX_GIT_SHA=unknown; export CORTEX_GIT_SHA

# ---- the backup scripts into <home>/bin, 0755, by a temporary name and a rename (the units run cortex-dump, cortex-dump-status and cortex-dump-alert from here;
# cortex upgrade needs the installed cortex-dump before it takes a dump)
mkdir -p "$home_dir/bin" || stop "$EX_START" cannot_write "$home_dir/bin" "cannot create $home_dir/bin"
for f in $backup_scripts; do
  [ "$without_backups" -eq 0 ] || [ -f "$scripts_src/$f" ] || continue
  cp "$scripts_src/$f" "$home_dir/bin/$f.new" && chmod 0755 "$home_dir/bin/$f.new" && mv -f "$home_dir/bin/$f.new" "$home_dir/bin/$f" || stop "$EX_START" cannot_write "$home_dir/bin" "cannot install $f into $home_dir/bin"
done

# launchd-run (bob's M-1c) stands in for OnFailure= and ExecStartPost= and is run by the agents from <home>/bin: the same temporary name and rename. Only the agents use it.
if [ "$without_backups" -eq 0 ]; then
  lr_src="$here/../scripts/launchd-run"
  [ -f "$lr_src" ] || stop "$EX_FETCH" release_incomplete "launchd-run in $here/../scripts" "the backup helper launchd-run is missing in $here/../scripts"
  cp "$lr_src" "$home_dir/bin/launchd-run.new" && chmod 0755 "$home_dir/bin/launchd-run.new" && mv -f "$home_dir/bin/launchd-run.new" "$home_dir/bin/launchd-run" || stop "$EX_START" cannot_write "$home_dir/bin" "cannot install launchd-run into $home_dir/bin"
fi
[ "$start" -eq 1 ] || { echo "install: secrets and config are ready; --no-start given, nothing started."; exit 0; }

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

# 1. the database alone, 2. the module roles (a superuser step, before any migration), 3. migrate and the server (the release's images are LOADED: nothing is built on a Mac)
# The rendering gate (bob's scripts/docker-compose-render) asks Docker for the MERGED configuration of compose.yml and the Docker Desktop override and fails if anything of the base's hardening was weakened; it starts nothing. ONCE, BEFORE the first up.
# It reads CORTEX_SECRETS_DIR, CORTEX_CONFIG, CORTEX_DUMP_STATUS and CORTEX_SERVER_IMAGE (all four are exported by now).
render_tool="$rel_dir/bin/docker-compose-render"; [ -f "$render_tool" ] || render_tool="$here/../scripts/docker-compose-render"   # the release carries it in bin/ (a downloaded install.sh has no scripts/ beside it); the checkout has it beside
if py_usable; then render_lint=--lint; else render_lint=""; say "install: the hardening lint of the rendered configuration was NOT run: no usable python3 here (on a Mac it needs the Command Line Tools); Docker rendered the files and nothing was started"; fi
sh "$render_tool" --base "$rel_dir/compose.yml" --override "$rel_dir/docker/compose.docker.yml" $render_lint >/dev/null || stop "$EX_START" render_failed "" "the Docker Desktop configuration failed its own check (see the output above); nothing was started"
compose_run up -d db || stop "$EX_START" compose_up_failed "" "compose up failed for the database (see the output above)"
wait_healthy cv2-s5-db
# The initdb auth flags act on a FRESH cluster only, so read the EFFECTIVE rules, not the environment (vera): a database created before the flags may still
# trust. Refuse it: change nothing IN THE DATABASE and never delete or recreate a volume to fix it. This point is reached after the home, the secrets, the configuration, the backup scripts and the database start (W1-223, sam E01-SAM01, as the Linux installer): the three messages below say what the refusal did not touch and that what is made stays in place, never 'nothing was changed'.
die_code=$EX_START die_id=db_unreadable die_arg=
trust_rules=$(docker exec -i --user postgres cv2-s5-db psql -U postgres -d cortex -tAc "select count(*) from pg_hba_file_rules where auth_method = 'trust' or error is not null" 2>/dev/null) || die "could not read the database's access rules: check docker logs cv2-s5-db, then run this installer again (that is safe); this refusal did not touch the database's rules, data or volume, and what the installer has made so far stays in place"
case $trust_rules in ""|*[!0123456789]*) die "could not read a count of trust rules from the database (got: $(printf '%.20s' "$trust_rules")): check docker logs cv2-s5-db, then run this installer again (that is safe); this refusal did not touch the database's rules, data or volume, and what the installer has made so far stays in place" ;; esac
[ "$trust_rules" -eq 0 ] || stop "$EX_START" db_trust "" "the database has $trust_rules trust or broken pg_hba rule(s) (it was created before the auth flags); this refusal did not touch the database's rules, data or volume, and what the installer has made so far stays in place. See S5-PEER-AUTH-MAP.md for the explicit step; do not delete the volume"
# THE ROLES STEP: deploy/db-roles.sh (a Linux file, NOT changed here) runs `podman exec ... psql` and finds podman by PATH. A private directory of ours (0700, inside the home, removed on every exit) holds ONE helper named podman
# that runs the pinned docker with the same arguments; only this one call gets that directory first on its PATH. No Linux byte changes for it (the alternative is a runtime variable in db-roles.sh: a Linux change first).
case $DOCKER_BIN in *[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._/+-]*) stop "$EX_START" docker_path_unsafe "$DOCKER_BIN" "the docker path holds a character that is not passed on" ;; esac
shim_made=$(mktemp -d "$home_dir/.cortex-install-shim.XXXXXXXX") || stop "$EX_START" shim_failed "$home_dir" "cannot make the helper directory in $home_dir"
{ printf '#!/bin/sh\nexec %s "$@"\n' "$DOCKER_BIN" > "$shim_made/podman" && chmod 0700 "$shim_made/podman"; } || stop "$EX_START" shim_failed "$shim_made" "cannot write the helper in $shim_made"
# cox W1-55: the CALLER passes the absolute deadline E (epoch seconds); db-roles.sh reads no clock of its own and refuses to commit past E - 9. There is no engine here, so this script is the caller: now + 60 s.
# The PLATFORM role (the Linux candidate's defect J, kai's first real install 9 Nov): the platform migration grants to cortex_m_platform, which no enabled module is: the roles step makes it too (db-roles.sh makes a role for any valid id).
roles_platform=platform
# shellcheck disable=SC2086
PATH="$shim_made:$SAFE_PATH" sh "$rel_dir/db-roles.sh" --deadline "$(( $(date +%s) + 60 ))" $modules $roles_platform || stop "$EX_START" roles_failed "" "creating the module roles failed (see above); nothing else was started"
case ${shim_made##*/} in .cortex-install-shim.????????) { [ -d "$shim_made" ] && [ ! -L "$shim_made" ]; } && rm -rf -- "$shim_made" || : ;; esac
shim_made=""
up_ok=1
compose_run up -d || up_ok=0
# migrate is a one-shot: it has no health status, so read its exit code. The server waits for it, so a failure stops here.
migrate_state=$(docker inspect --format '{{.State.Status}} {{.State.ExitCode}}' cv2-s5-migrate 2>/dev/null || echo missing)
case $migrate_state in
  "exited 0") ;;
  "exited "*) stop "$EX_START" migrate_failed "$migrate_state" "the migrations failed ($migrate_state); the server was not started (see: docker logs cv2-s5-migrate)" ;;
esac
[ "$up_ok" -eq 1 ] || stop "$EX_START" compose_up_failed "" "compose up failed (see the output above)"
wait_healthy cv2-s5-db cv2-s5-server

# ---- the first lead token: minted once, in memory (tmpfs), piped out into a 0600 file -----------------------------
token="$home_dir/$lead.token"
if [ -s "$token" ]; then
  chmod 0600 "$token" || stop "$EX_WORKER" cannot_write "$token" "cannot set the mode of $token"
  echo "install: $token exists; bootstrap already done (use --reissue-token for a new one)."
else
  [ ! -e "$token" ] || { echo "install: $token is empty (a failed earlier run); minting a new token"; rm -f "$token" || stop "$EX_WORKER" cannot_write "$token" "cannot remove the empty $token"; }
  mint_token "$token"
fi

# ---- the backup agents (piece 12, bob's M-1c scripts/launchd-backup): the two launch agents, loaded into the user's gui domain. THEY RUN ONLY WHILE THIS USER IS LOGGED IN (a missed daily dump is not made up).
# The stack is up by now; if the tool refuses (THE FLOOR: the backup tools are not portable yet) the install fails LOUDLY (a backup that is never scheduled is a data-loss risk: u1, kai's default) and a second run, which repeats all of
# this, finishes the job. --without-backups is a TEST option (argv only): ONE warning, no launchd call, and the closing lines say that the backups are not scheduled.
if [ "$without_backups" -eq 1 ]; then
  warn "the backups are NOT scheduled: no automatic backup exists on the Mac in this release (the backup tools are not available for macOS yet), so the launchd step was skipped; back your data up yourself"
else
  "$here/../scripts/launchd-backup" install --home "$home_dir" --db-container cv2-s5-db || stop "$EX_BACKUP" launchd_failed "" "the launchd backup agents could not be installed (see the message above)"
fi

# ---- step 10: say what happened (the prerequisite file's say_done when the build has it; this file's two lines otherwise)
# say_done takes a FOURTH value, the install log, and refuses an empty one (the Linux candidate's defect N, kai's full pass 9 Nov). This installer keeps NO log of its own (log_line writes only when CORTEX_LOG names a file), so the value is that file
# when CORTEX_LOG is set and the plain word none when it is not.
report_strays
if command -v say_done >/dev/null 2>&1; then
  say_done "${CORTEX_VERSION:-unknown}" "http://127.0.0.1:$port/v2/health/ready" "$token" "${CORTEX_LOG:-none}" || :
else
  echo "install: done. Server http://127.0.0.1:$port/v2/health/ready"
  echo "install: token file $token (mode 600). Point a worker at it with CORTEX_TOKEN_FILE=$token"
fi
[ "$without_backups" -eq 0 ] || echo "install: the backups are NOT scheduled: no automatic backup exists on the Mac in this release; back your data up yourself"
