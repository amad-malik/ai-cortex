#!/bin/sh
# Logic test with FAKE podman: no container, external network or real secret.
# W1-308 peer cells use an owned synthetic loopback HTTP listener.
# It proves the token handling (S5 amendment 2; kai's ruling, row 2a7de258):
#   the first run bootstraps once; a second run does not; --reissue-token mints into a NEW 0600 file and leaves the
#   old one alone; a failed bootstrap leaves no partial file; no token ever reaches the output or the call log.
# Run it through cv2-admit like any test:  cv2-admit scripts/install-test.sh
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
install=${INSTALL_SH:-"$here/../deploy/install.sh"}   # INSTALL_SH lets a test run another copy, for example the commit nix reviewed
T=$(mktemp -d)
# THE LIMIT OF THIS SUITE, stated: it judges install.sh's OWN logic against FAKE lanes. It does NOT run against the real includes (deploy/install-prereq.sh and deploy/install-fetch.sh): each real include has its own
# suite and mutants, and the JOINED real-bytes run is the L2 / G12 gate (nix and marv). Integration puts the REAL includes into deploy/, where install.sh sources them (the fetch include AFTER install-prereq.sh) and their
# functions REPLACE the fakes of the cells (and a half-joined build fails closed); the real prerequisite file would also make every direct run look at the real machine. So, unless INSTALL_SH names the file to test (the
# mutant harness does, with its own copy), the suite runs a private copy of deploy/ WITHOUT those two files, and every copy of deploy/ that a cell makes (ifd and the like) drops them too.
# copy_deploy SRC DEST is THE ONE PLACE where this suite copies a deploy directory (cox W1-99): the copy has NEITHER real include. It is used for the tree itself (below) and for every copy a cell makes (ifd and the
# two cells that build their own lane); the copy lives only under the test's temp directory, never in deploy/ of the checkout.
copy_deploy() {
  mkdir -p "$2" && cp -R "$1/." "$2/" && rm -f "$2/install-fetch.sh" "$2/install-prereq.sh" || return 1
  python3 -I - "$2/install.sh" "$T/account-runtime" <<'RUNTIME_COPY'
import pathlib,re,shlex,sys
p=pathlib.Path(sys.argv[1]);s=p.read_text()
pattern=r'^mint_runtime_dir\(\) \{\n.*?^}$'
if re.search(pattern,s,re.M|re.S):
    replacement="mint_runtime_dir() {\n  printf '%s\\n' "+shlex.quote(sys.argv[2])+"\n}"
    s=re.sub(pattern,lambda _:replacement,s,count=1,flags=re.M|re.S)
p.write_text(s)
RUNTIME_COPY
}
unisolated_install=$install
copy_deploy "$(dirname "$install")" "$T/deploy-noinc" && install="$T/deploy-noinc/install.sh"
# the compose provider must sit where no one else can replace it (no directory on its way writable by group or other: /tmp is): the fake one lives beside this script
P=$(mktemp -d "$here/.provider.XXXXXX")
# test-only hooks for nix's L2 launcher (cox fc450500; the installer is not touched; cells IT-121 to IT-124): L2_KEEP=dir keeps each reported test's output, call log and compose.env under its test id, and the last
# log and compose.env as final.*, BEFORE the temporary directory goes (a caller's directory is created 0700 and only copied into, never emptied or removed); L2_POLL, L2_WAIT, L2_SCRIPTS_DIR, L2_HOME and L2_PATH_PREFIX
# let a caller's value REACH the installer (UNREACHED BY DESIGN: FAKE_LOG and FAKE_STATE, the channels of the fakes); IT_ONLY="IT-55 IT-06" selects test ids (see want below).
keep_one() { # keep_one <title>
  [ -n "${L2_KEEP:-}" ] || return 0
  ko_id=${1%% *}
  ( umask 077; mkdir -p "$L2_KEEP" ) || return 0
  cp "$T/out" "$L2_KEEP/$ko_id.out" 2>/dev/null || :
  cp "$T/log" "$L2_KEEP/$ko_id.log" 2>/dev/null || :
  cp "$T/state/compose.env" "$L2_KEEP/$ko_id.compose.env" 2>/dev/null || :
}
keep_final() {
  [ -n "${L2_KEEP:-}" ] || return 0
  ( umask 077; mkdir -p "$L2_KEEP" ) || return 0
  cp "$T/log" "$L2_KEEP/final.log" 2>/dev/null || :
  cp "$T/state/compose.env" "$L2_KEEP/final.compose.env" 2>/dev/null || :
  for keep_probe in "$T"/abort-output-* "$T"/peer-*; do
    [ -d "$keep_probe" ] || continue
    cp -R "$keep_probe" "$L2_KEEP/" 2>/dev/null || :
  done
}
trap 'keep_final; rm -rf "$T" "$P"' EXIT
mkdir -p "$T/bin" "$T/state" "$T/home" "$T/account-runtime"
chmod 700 "$T/account-runtime"
SUM_ACCOUNT_RUNTIME=$T/account-runtime; export SUM_ACCOUNT_RUNTIME
: > "$T/log"

# No suite layout, including a copy-isolation mutant, may use a real account
# runtime. Private copies use the owned sandbox; the original factory is tested
# read-only. Other metadata calls keep their real meaning.
cat > "$T/bin/stat" <<'SAFE_STAT'
#!/bin/sh
for arg in "$@"; do
  case $arg in
    /run/user/*/cortex-mint.reservation*) exit 1 ;;
    /run/user/*) runtime_uid=${arg#/run/user/}; case $runtime_uid in ''|*[!0123456789]*) ;; *) exit 1 ;; esac ;;
  esac
done
exec /usr/bin/stat "$@"
SAFE_STAT
chmod +x "$T/bin/stat"
cat > "$T/bin/podman" <<'FAKE'
#!/bin/sh
# records its arguments; simulates only what install.sh calls
if [ "${FAKE_INSPECT_SECRET_ENV:-0}" = 1 ] && [ "${val+x}" = x ] && [ "${#val}" = 64 ]; then
  case ${val:-} in *[!0123456789abcdef]*) ;; *) : > "$FAKE_STATE/secret-env-leak" ;; esac
fi
case $1 in container)   # `container exists NAME` (not logged): the stack exists after a compose up; the server container (defect M, kai's full pass) is also gone after `rm -f` of it, never made with FAKE_NO_SERVER=1, and unanswerable (125) with FAKE_CONTAINER_EXISTS_ERR=1
  if [ "${2:-}" = inspect ]; then   # M v2 (sam NMR-SAM-02): ONE call by name that answers the full id and the labels of the SAME object, one value per line: id, cortexv2, compose project, compose working directory (podman-compose 1.5.0 records the RESOLVED directory of the -f file; measured, probe-m2.out)
    echo "$*" >> "$FAKE_LOG"
    [ "${FAKE_CONTAINER_INSPECT_FAIL:-0}" != 1 ] || exit 125
    { [ -f "$FAKE_STATE/generation" ] && [ ! -f "$FAKE_STATE/server-removed" ]; } || exit 125
    case " $* " in *Config.Labels*) printf '%s\n%s\n%s\n%s\n' "${FAKE_SERVER_ID-0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef}" "${FAKE_SERVER_LABEL-dev}" "${FAKE_SERVER_PROJECT-cv2-s5}" "${FAKE_SERVER_WORKDIR-$(cat "$FAKE_STATE/workdir" 2>/dev/null)}" ;; esac
    exit 0
  fi
  [ "${2:-}" = exists ] || exit 1
  [ "${3:-}" != cv2-s5-server ] || [ "${FAKE_CONTAINER_EXISTS_ERR:-0}" != 1 ] || exit 125
  [ -f "$FAKE_STATE/generation" ] || exit 1
  if [ "${3:-}" = cv2-s5-server ]; then [ "${FAKE_NO_SERVER:-0}" != 1 ] || exit 1; [ ! -f "$FAKE_STATE/server-removed" ] || exit 1; fi
  exit 0 ;;
esac
[ -z "${FAKE_ENVCHECK:-}" ] || env | grep -E '^(LD_PRELOAD|LD_LIBRARY_PATH|LD_AUDIT|PYTHONPATH|PYTHONHOME|PYTHONSTARTUP|BASH_ENV|ENV|CDPATH|OPENSSL_CONF|OPENSSL_MODULES|OPENSSL_ENGINES|GCONV_PATH|CONTAINER_HOST|CONTAINER_CONNECTION|CONTAINERS_CONF|CONTAINERS_STORAGE_CONF|DOCKER_HOST|REGISTRY_AUTH_FILE|CONTAINERS_REGISTRIES_CONF)=' >> "$FAKE_PROVLOG.env" || :   # a read-only existence question: answered, not logged with the calls
echo "$*" >> "$FAKE_LOG"
case $1 in
  --version) echo "podman version ${FAKE_PODMAN_VERSION:-5.8.2}" ;;
  compose) echo "$* | ${PODMAN_COMPOSE_PROVIDER-unset}" >> "$FAKE_PROVLOG"   # W1-68: every compose call is handed the verified provider through this variable
    [ "${PODMAN_COMPOSE_WARNING_LOGS:-true}" = false ] || echo '>>>> Executing external compose provider "/usr/bin/podman-compose". Please see podman-compose(1) for how to disable this message. <<<<' >&2
    shift
    case ${1:-} in
      version)
        [ "${FAKE_COMPOSE_VERSION_FAIL:-0}" != 1 ] || { echo "Error: look up compose: executable file not found" >&2; exit 125; }
        [ -z "${FAKE_COMPOSE_DIAGNOSTIC:-}" ] || printf '%s\n' "$FAKE_COMPOSE_DIAGNOSTIC" >&2
        # what the REAL podman 5.8.2 with podman-compose 1.5.0 prints on stdout (kai's first install on a fresh Rocky 10.2 VM, 9 Nov, defect H): TWO lines, podman's OWN line FIRST ('podman version 5.8.2'), then the
        # provider's ('podman-compose version 1.5.0'); podman's banner about the external provider goes to STDERR, first; a test sets FAKE_COMPOSE_VERSION_OUT (printf %b text)
        printf '%b' "${FAKE_COMPOSE_VERSION_OUT-podman version 5.8.2\npodman-compose version 1.5.0\n}" ;;
      *) echo "CORTEX_DUMP_STATUS=${CORTEX_DUMP_STATUS-unset} CORTEX_DUMPS=${CORTEX_DUMPS-unset} CORTEX_GIT_SHA=${CORTEX_GIT_SHA-unset}" >> "$FAKE_STATE/compose.env"
         [ -z "${CORTEX_SERVER_IMAGE:-}${CORTEX_DB_IMAGE:-}" ] || echo "CORTEX_VERSION=${CORTEX_VERSION-unset} CORTEX_SERVER_IMAGE=${CORTEX_SERVER_IMAGE-unset} CORTEX_DB_IMAGE=${CORTEX_DB_IMAGE-unset}" >> "$FAKE_STATE/compose.images"
         case " $* " in
           *" up "*) [ "${FAKE_COMPOSE_FAIL:-0}" != 1 ] || exit 1
                     echo $(( $(cat "$FAKE_STATE/generation" 2>/dev/null || echo 0) + 1 )) > "$FAKE_STATE/generation"   # the containers are made again: a new identity
                     echo "${CORTEX_PORT:-8601}" > "$FAKE_STATE/port"
                     rm -f "$FAKE_STATE/server-removed"; : > "$FAKE_STATE/server-up"
                     fw=""; fprev=""; for fa in "$@"; do [ "$fprev" = -f ] && fw=$fa; fprev=$fa; done   # what podman-compose records as com.docker.compose.project.working_dir: the RESOLVED directory of the -f file
                     [ -z "$fw" ] || ( cd -P -- "$(dirname -- "$fw")" 2>/dev/null && pwd -P ) > "$FAKE_STATE/workdir" ;;   # podman-compose starts the server even when migrate failed (kai's t4): it is up and its rootlessport holds the port
           *" build "*) [ "${FAKE_BUILD_FAIL:-0}" != 1 ] || exit 1 ;;
         esac; exit 0 ;;
    esac ;;
  port)
    [ "${FAKE_PORT_FAIL:-0}" != 1 ] || exit 125
    [ -f "$FAKE_STATE/port" ] || exit 125
    if [ "${FAKE_PORT_WRONG:-0}" = 1 ] || [ -e "$FAKE_STATE/peer-port-wrong" ]; then
      echo "127.0.0.1:$(( $(cat "$FAKE_STATE/port") + 1 ))"
    else echo "127.0.0.1:$(cat "$FAKE_STATE/port")"; fi ;;
  secret)
    case $2 in
      inspect) for last in "$@"; do :; done   # the secret's name is the LAST argument (flags such as --showsecret and --format come before it)
               [ -f "$FAKE_STATE/secret-$last" ] || exit 1
               case " $* " in *"{{len .SecretData}}"*) wc -c < "$FAKE_STATE/secret-$last" | tr -d ' ' ;; esac ;;
      create) [ "${FAKE_SECRET_CREATE_FAIL:-0}" != 1 ] || exit 125
              cat > "$FAKE_STATE/secret-$3" ;;
    esac ;;
  inspect) [ "${FAKE_INSPECT_FAIL:-0}" != 1 ] || exit 1
    case $3 in
      *State.ExitCode*) if [ -n "${FAKE_MIGRATE_STATE:-}" ]; then echo "$FAKE_MIGRATE_STATE"; elif [ "${FAKE_MIGRATE_FAIL:-0}" = 1 ]; then echo "exited 1"; else echo "exited 0"; fi ;;   # FAKE_MIGRATE_STATE='running 0' is finn's retry window (defect S, IT-155): the status and the exit code, as podman prints them
      *Config.Labels*) echo "${FAKE_SERVER_LABEL-dev}" ;;   # the label cortexv2 of the server container (compose.yml sets dev); FAKE_SERVER_LABEL= (empty) or another value is a container that is not ours
      *State.Status*)
        for inspect_target in "$@"; do :; done
        if [ "$inspect_target" = cv2-s5-server ] && { [ "${FAKE_SERVER_EXITED:-0}" = 1 ] || [ -e "$FAKE_STATE/peer-server-down" ]; }; then
          echo 'exited unhealthy'
        elif [ "$inspect_target" = cv2-s5-db ] && [ "${FAKE_DB_EXITED:-0}" = 1 ]; then
          echo 'exited unhealthy'
        elif [ -n "${FAKE_SUM_CONTAINER_LATE:-}" ] && [ -e "$FAKE_STATE/summary-container-late" ]; then
          echo "$FAKE_SUM_CONTAINER_LATE"
        elif [ "$inspect_target" = cv2-s5-db ]; then
          echo "${FAKE_SUM_CONTAINER:-running ${FAKE_DB_HEALTH:-${FAKE_HEALTH:-healthy}}}"
        else echo "${FAKE_SUM_CONTAINER:-running ${FAKE_HEALTH:-healthy}}"; fi ;;
      *) for inspect_target in "$@"; do :; done
         if [ "$inspect_target" = cv2-s5-db ]; then echo "${FAKE_DB_HEALTH:-${FAKE_HEALTH:-healthy}}"; else echo "${FAKE_HEALTH:-healthy}"; fi ;;
    esac ;;
  rm) [ "${FAKE_RM_FAIL:-0}" != 1 ] || exit 1   # podman rm (defect M): only the server container is modelled, by name or by its id
      [ "${FAKE_RM_ID_GONE:-0}" != 1 ] || { echo "Error: no container with ID or name found: the object was replaced" >&2; exit 1; }   # M v2: the object the id named is gone (replaced between the inspect and the removal)
      case " $* " in *" cv2-s5-server "*|*" ${FAKE_SERVER_ID-0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef} "*) : > "$FAKE_STATE/server-removed"; rm -f "$FAKE_STATE/server-up" ;; esac ;;
  exec)
    shift
    while [ "${1#-}" != "$1" ]; do if [ "$1" = --user ] || [ "$1" = --env ]; then shift 2; else shift; fi; done   # skip flags such as -i, --user NAME and --env NAME=VALUE
    shift                                        # the container name
    case $1 in
      psql)
        case " $* " in *"SELECT p.id::text FROM identity.projects"*)
          if [ -n "${FAKE_PROJECT_LOOKUP_SIGNAL:-}" ]; then
            exec python3 -I -c '
import os, signal, time
sig = getattr(signal, "SIG" + os.environ["FAKE_PROJECT_LOOKUP_SIGNAL"])
signal.signal(sig, signal.SIG_DFL)
os.kill(os.getpid(), sig)
time.sleep(1)
raise RuntimeError("fake lookup survived its own signal")
'
          fi
          [ -z "${FAKE_PROJECT_LOOKUP_EXIT:-}" ] || exit "$FAKE_PROJECT_LOOKUP_EXIT"
          if [ "${FAKE_PROJECT_LOOKUP_STALL:-0}" = 1 ]; then
            exec python3 -I -c '
import os, pathlib, signal, time
start = pathlib.Path("/proc/self/stat").read_text().rsplit(") ", 1)[1].split()[19]
pathlib.Path(os.environ["FAKE_STATE"], "lookup.pid").write_text(str(os.getpid()) + " " + start)
signal.signal(signal.SIGTERM, signal.SIG_IGN)
while True: time.sleep(1)
'
          fi
          [ "${FAKE_PEER_RACE:-}" != server ] || : > "$FAKE_STATE/peer-server-down"
          [ "${FAKE_PEER_RACE:-}" != port ] || : > "$FAKE_STATE/peer-port-wrong"
          [ "${FAKE_PROJECT_LOOKUP_FAIL:-0}" != 1 ] || exit 1
          for lookup_sql in "$@"; do :; done
          lookup_name=${lookup_sql##*"n.name_key = '"}; lookup_name=${lookup_name%%"'"*}
          grep -q "^$lookup_name:" "$FAKE_STATE/projects" 2>/dev/null || exit 1
          printf '%s\n' "${FAKE_PROJECT_ID:-11111111-1111-4111-8111-111111111111}"; exit 0 ;;
        esac
        case " $* " in *" -tAc "*) [ "${FAKE_HBA_FAIL:-0}" != 1 ] || exit 1
                                   if [ "${FAKE_HBA_RAW+x}" = x ]; then printf '%s\n' "$FAKE_HBA_RAW"; else echo "${FAKE_HBA_TRUST:-0}"; fi; exit 0 ;; esac   # the effective-rules query: how many trust rules
        [ "${FAKE_PSQL_FAIL:-0}" != 1 ] || exit 1          # the roles step
        cat >> "$FAKE_STATE/roles.sql" ;;
      /usr/local/bin/cortex-server)
        # piece 11 contract r0.3 (sha256 prefix 3edf19653df7), the part install.sh depends on: `bootstrap --project P --lead L
        # --token-file F [--require-existing]`. Without the flag the first run CREATES the project and its lead and every run mints;
        # with it, nothing is created: unless P:L is in the table it prints the ONE fixed line ($FAKE_REFUSAL), writes nothing and
        # exits 3. An older server (FAKE_OLD_SERVER=1) does not know the flag: a usage error, exit 2, another line. The exit code
        # of every call is recorded in $FAKE_STATE/bootstrap.exit.
        shift
        [ "${1:-}" = bootstrap ] || exit 64
        shift
        fp="" fl="" guard=0
        while [ $# -gt 0 ]; do
          case $1 in
            --project) fp=$2; shift 2 ;;
            --lead) fl=$2; shift 2 ;;
            --token-file) shift 2 ;;
            --require-existing)
              if [ "${FAKE_OLD_SERVER:-0}" = 1 ]; then echo "error: unexpected argument '--require-existing'" >&2; echo 2 > "$FAKE_STATE/bootstrap.exit"; exit 2; fi
              guard=1; shift ;;
            *) echo "error: unexpected argument" >&2; echo 2 > "$FAKE_STATE/bootstrap.exit"; exit 2 ;;
          esac
        done
        [ "${FAKE_BOOTSTRAP_FAIL:-0}" != 1 ] || { echo 1 > "$FAKE_STATE/bootstrap.exit"; exit 1; }
        tbl="$FAKE_STATE/projects"; touch "$tbl"
        if [ "$guard" = 1 ]; then
          grep -qxF "$fp:$fl" "$tbl" || { echo "$FAKE_REFUSAL" >&2; echo 3 > "$FAKE_STATE/bootstrap.exit"; exit 3; }
        elif ! grep -qxF "$fp:$fl" "$tbl"; then
          if grep -q "^$fp:" "$tbl"; then echo "bootstrap failed: the project has another lead" >&2; echo 1 > "$FAKE_STATE/bootstrap.exit"; exit 1; fi
          echo "$fp:$fl" >> "$tbl"
        fi
        n=$(cat "$FAKE_STATE/count" 2>/dev/null || echo 0); n=$((n + 1)); echo "$n" > "$FAKE_STATE/count"
        if [ "${FAKE_BOOT_NO_NEWLINE:-0}" = 1 ]; then
          printf '%s' "${FAKE_BOOT_TOKEN:-FAKE-TOKEN-$n}" > "$FAKE_STATE/bootstrap.token"
        else echo "${FAKE_BOOT_TOKEN:-FAKE-TOKEN-$n}" > "$FAKE_STATE/bootstrap.token"; fi
        echo 0 > "$FAKE_STATE/bootstrap.exit"
        cp "$FAKE_STATE/bootstrap.token" "$FAKE_STATE/active.token"
        printf '%s\n' "$fl" > "$FAKE_STATE/last-lead" ;;
      cat)
        if [ "${FAKE_CAT_FAIL_ONCE:-0}" = 1 ] && [ ! -f "$FAKE_STATE/catfailed" ]; then : > "$FAKE_STATE/catfailed"; exit 1; fi
        [ "${FAKE_CAT_EMPTY:-0}" != 1 ] || exit 0
        cat "$FAKE_STATE/bootstrap.token" ;;
      rm) [ "${FAKE_EXEC_RM_FAIL:-0}" != 1 ] || exit 1
          [ "${FAKE_EXEC_STRICT_PATH:-0}" != 1 ] || [ "$*" = "rm -f /run/cortex/bootstrap.token" ] || exit 1   # cox 9fb1d9a1: with the variable set only the token copy's own path counts as that file (OFF by default: IT-146 turns it on, so a wrong path in the helper is a removal that fails)
          if [ "${FAKE_BOUNDARY_PHASE:-}" = reissue-cleanup ] && [ -f "$FAKE_STATE/published-reissue-path" ] && [ ! -e "$FAKE_STATE/boundary-delivered" ]; then
            "$SUM_BOUNDARY_HELPER" reissue-cleanup
          fi
          if [ "${FAKE_BOUNDARY_PHASE:-}" = handler-copy ] && [ -e "$FAKE_STATE/first-signal-sent" ] && [ -f "$FAKE_STATE/bootstrap.token" ] && [ ! -e "$FAKE_STATE/boundary-delivered" ]; then
            "$SUM_BOUNDARY_HELPER" handler-copy
          fi
          if [ "${FAKE_BOUNDARY_PHASE:-}" = prepublication-cleanup ] && [ -f "$FAKE_STATE/bootstrap.token" ] && [ ! -e "$FAKE_STATE/boundary-delivered" ]; then
            "$SUM_BOUNDARY_HELPER" prepublication-cleanup
          fi
          rm -f "$FAKE_STATE/bootstrap.token" ;;
    esac ;;
esac
FAKE
chmod +x "$T/bin/podman"
# systemctl: records its arguments; FAKE_SYSTEMCTL_FAIL=1 makes it fail like a user manager that is not reachable; FAKE_TIMERS_ASK_FAIL=1 makes ONLY the is-enabled question about the cortex timers fail (the restart unit still answers), so that the timers line of install_finished is the only guard that can say "not finished" (IM-296)
cat > "$T/bin/systemctl" <<'FAKE'
#!/bin/sh
case " $* " in
  *" is-enabled "*) echo "$*" >> "$FAKE_PROVLOG.ro"
    [ "${FAKE_SYSTEMCTL_FAIL:-0}" != 1 ] || { echo "Failed to connect to bus: No medium found" >&2; exit 1; }
    case " $* " in *" podman-restart.service "*)   # the distro's unit: the state word systemd prints, with its real exit status (NMR-SAM-01): 0 for enabled, enabled-runtime, linked, linked-runtime, alias, static, indirect, generated, transient and an empty answer; 1 for disabled, masked and not-found
      if [ -f "$FAKE_STATE/restart-state" ]; then rw=$(cat "$FAKE_STATE/restart-state"); elif grep -qx podman-restart.service "$FAKE_STATE/enabled" 2>/dev/null; then rw=enabled; else rw=disabled; fi
      case $rw in
        empty) exit 0 ;;
        not-found) echo "Failed to get unit file state for podman-restart.service: No such file or directory" >&2; exit 1 ;;
        disabled|masked|masked-runtime) echo "$rw"; exit 1 ;;
        *) echo "$rw"; exit 0 ;;
      esac ;;
    esac
    [ "${FAKE_TIMERS_ASK_FAIL:-0}" != 1 ] || { echo "Failed to connect to bus: No medium found" >&2; exit 1; }   # only the timer question (the restart unit answered above)
    # Real systemctl returns 0 if AT LEAST ONE requested unit is enabled.
    any_enabled=1
    for u in "$@"; do case $u in cortex-*.timer)
      state=disabled
      grep -qx "$u" "$FAKE_STATE/enabled" 2>/dev/null && state=enabled
      [ "${FAKE_TIMER_DISABLED:-}" != "$u" ] || state=disabled
      if [ "${FAKE_TIMER_SELECTED:-}" = "$u" ]; then state=${FAKE_TIMER_STATE-}; fi
      if [ "${FAKE_TIMER_LATE:-}" = "$u" ]; then
        asks=$(cat "$FAKE_STATE/asks-$u" 2>/dev/null || echo 0)
        asks=$((asks + 1)); echo "$asks" > "$FAKE_STATE/asks-$u"
        [ "$asks" -lt 2 ] || state=${FAKE_TIMER_LATE_STATE-disabled}
      fi
      echo "$state"
      case $state in disabled|masked|masked-runtime|not-found) ;; *) any_enabled=0 ;; esac ;;
    esac; done
    exit "$any_enabled" ;;
esac
echo "$*" >> "$FAKE_STATE/systemctl.log"
case " $* " in *" enable "*podman-restart.service*) echo "systemctl $*" >> "$FAKE_LOG" ;; esac   # defect R: the call order against the stack and the summary is read from the one log
[ "${FAKE_SYSTEMCTL_FAIL:-0}" != 1 ] || { echo "Failed to connect to bus: No medium found" >&2; exit 1; }
[ "${FAKE_RESTART_ENABLE_FAIL:-0}" != 1 ] || case " $* " in *" enable "*podman-restart.service*) echo "Failed to enable unit: Unit file podman-restart.service does not exist." >&2; exit 1 ;; esac
case " $* " in *" enable "*) for u in "$@"; do case $u in cortex-*.timer|podman-restart.service) grep -qx "$u" "$FAKE_STATE/enabled" 2>/dev/null || echo "$u" >> "$FAKE_STATE/enabled" ;; esac; done ;; esac
# Diagnostic-only: enable persisted, but --now failed to start the timer job.
case " $* " in *" enable --now cortex-dump.timer cortex-dump-status.timer "*)
  [ "${FAKE_TIMER_ENABLE_FAIL_AFTER_ENABLE:-0}" != 1 ] || { echo "Failed to start timer job after enable" >&2; exit 1; } ;;
esac
# after an enable the unit is persistent, unless FAKE_RESTART_AFTER_ENABLE names the state it stays in (static, masked ...: an enable that does not take)
case " $* " in *" enable "*podman-restart.service*) if [ -n "${FAKE_RESTART_AFTER_ENABLE:-}" ]; then echo "$FAKE_RESTART_AFTER_ENABLE" > "$FAKE_STATE/restart-state"; else rm -f "$FAKE_STATE/restart-state"; fi ;; esac
exit 0
FAKE
# dnf and sudo: they only log (a cell proves that a failed trust step called neither)
for t in dnf sudo; do printf '#!/bin/sh\necho "%s $*" >> "$FAKE_LOG"\nexit 0\n' "$t" > "$T/bin/$t"; chmod +x "$T/bin/$t"; done
# curl: the server's open health document (GET /v2/health); FAKE_CURL_FAIL=1 makes it fail, FAKE_SERVER_VERSION names the version it reports
mkdir -p "$T/bin-curl"
cat > "$T/bin-curl/curl" <<'FAKE'
#!/bin/sh
echo "curl $*" >> "$FAKE_LOG"
[ "${FAKE_CURL_FAIL:-0}" != 1 ] || exit 22
case "$*" in */v2/health) [ "${FAKE_HEALTH_DOC_FAIL:-0}" != 1 ] || exit 22 ;; esac
case "$*" in *":$(cat "$FAKE_STATE/port" 2>/dev/null || echo none)/"*) ;; *) exit 7 ;; esac   # nothing listens on another port
case "$*" in
  */v2/whoami*)
    # Only this explicitly selected synthetic loopback listener uses real curl.
    # All other calls and every existing cell retain their original fake lane.
    if [ "${FAKE_REAL_WHOAMI_LISTENER:-0}" = 1 ]; then exec /usr/bin/curl "$@"; fi
    IFS= read -r config || exit 1
    case $config in 'header = "Authorization: Bearer '*'"') ;; *) exit 1 ;; esac
    bearer=${config#'header = "Authorization: Bearer '}; bearer=${bearer%'"'}
    http=${FAKE_SUM_WHO_HTTP:-200}
    project_id=${FAKE_WHO_PROJECT:-11111111-1111-4111-8111-111111111111}
    [ "$bearer" = "$(cat "$FAKE_STATE/active.token" 2>/dev/null)" ] || http=401
    if [ -f "$FAKE_STATE/other.token" ] && [ "$bearer" = "$(cat "$FAKE_STATE/other.token")" ]; then
      http=200; project_id=22222222-2222-4222-8222-222222222222
    fi
    # Check the config header, but never record it or its token in the call log.
    [ "${FAKE_SUM_TRANSPORT:-ok}" != fail ] || exit 7
    if [ "${FAKE_SUM_ENV_CHECK:-0}" = 1 ]; then
      env | grep '^summary_\(secret\|reply\|tail\)=' >/dev/null && exit 99
    fi
    printf '{"installation":"33333333-3333-4333-8333-333333333333","project":"%s","worker":"44444444-4444-4444-8444-444444444444","name":"%s","kind":"%s","role":"%s","team":null,"job_roles":[],"token_id":"0123456789abcdef0123456789abcdef","expires_at":"2027-01-01T00:00:00Z","renew_due":false}\n%s' \
      "$project_id" "${FAKE_WHO_NAME-$(cat "$FAKE_STATE/last-lead" 2>/dev/null || echo lead)}" "${FAKE_WHO_KIND:-worker}" "${FAKE_WHO_ROLE:-project_lead}" "$http"
    exit 0 ;;
  *--write-out*health/ready*) [ -z "${FAKE_SUM_CONTAINER_LATE:-}" ] || : > "$FAKE_STATE/summary-container-late"
    printf '%s' "${FAKE_SUM_HEALTH_HTTP:-200}"; exit 0 ;;
esac
printf '{"state":"ready","version":"%s","ready":%s,"pieces":[]}\n' "${FAKE_SERVER_VERSION:-0.2.1-rc.2}" "${FAKE_SERVER_READY:-true}"
FAKE
chmod +x "$T/bin-curl/curl"
chmod +x "$T/bin/systemctl"
# the five backup scripts (bob's, piece 12) are not in this lane: STUBS in a directory the installer is told about; their mode is 0644
# on purpose, the installer must set 0755 itself
mkdir -p "$T/scripts"
for f in cortex-dump cortex-dump-status cortex-dump-alert cortex-restore restore-compare; do printf '#!/bin/sh\n# stub of %s\nexit 0\n' "$f" > "$T/scripts/$f"; chmod 0644 "$T/scripts/$f"; done
CORTEX_SCRIPTS_DIR=${L2_SCRIPTS_DIR-$T/scripts}; export CORTEX_SCRIPTS_DIR
mkdir -p "$T/bin-badrand"; printf '#!/bin/sh\nexit 1\n' > "$T/bin-badrand/head"; chmod +x "$T/bin-badrand/head"

# a stand-in for podman-compose (the install resolves it from PATH once, in a checkout without the prerequisite file); it is never run by the fake podman
printf '#!/bin/sh\n# stand-in for podman-compose\nexit 0\n' > "$P/podman-compose"; chmod 0755 "$P/podman-compose"; chmod 0755 "$P"
PROV="$P/podman-compose"
PATH="${L2_PATH_PREFIX:+$L2_PATH_PREFIX:}$T/bin:$T/bin-curl:$P:$PATH"; export PATH
FAKE_LOG="$T/log"; FAKE_STATE="$T/state"; FAKE_PROVLOG="$T/provider.log"; export FAKE_LOG FAKE_STATE FAKE_PROVLOG
: > "$FAKE_PROVLOG"
HOME=${L2_HOME-$T/home}; export HOME
CORTEX_INSTALL_WAIT=${L2_WAIT-2}; CORTEX_INSTALL_POLL=${L2_POLL-1}; export CORTEX_INSTALL_WAIT CORTEX_INSTALL_POLL

pass=0 fail=0
# N3 (IT_ONLY="IT-06 IT-01"): only the selected test ids are REPORTED and counted (a base id such as IT-54 also selects IT-54b); the cells before them still RUN, because they share state (first=, the helpers, the
# release layout), and the run stops as soon as the first cell that is not selected reports after all selected ones have: so a selection costs the run up to its last selected cell. An id that no cell carries is
# refused up front (exit 2), and one that no cell reports before the end is refused at the end.
sel_left=0 sel_hit=" "
for it_i in ${IT_ONLY:-}; do
  grep -q "$it_i[a-z]*[ \"]" "$here/install-test.sh" || { echo "install-test: unknown test id in IT_ONLY: $it_i"; exit 2; }
  sel_left=$((sel_left + 1))
done
want() { # want <title>: 0 when there is no selection or the title's id (its first word) is selected
  [ -n "${IT_ONLY:-}" ] || return 0
  w_id=${1%% *}
  for w_i in $IT_ONLY; do case $w_id in "$w_i"|"$w_i"[a-z]*) return 0 ;; esac; done
  return 1
}
note_sel() { # note_sel <title>: a reported selected id satisfies its list entry
  [ -n "${IT_ONLY:-}" ] || return 0
  n_id=${1%% *}
  for n_i in $IT_ONLY; do
    case $n_id in "$n_i"|"$n_i"[a-z]*) case $sel_hit in *" $n_i "*) ;; *) sel_hit="$sel_hit$n_i "; sel_left=$((sel_left - 1)) ;; esac ;; esac
  done
}
sel_gate() { # a cell that is NOT selected has reported: when every selected id has been reported already, the run ends here
  [ -n "${IT_ONLY:-}" ] && [ "$sel_left" -le 0 ] || return 0
  echo "install-test: $pass passed, $fail failed"
  [ "$fail" -eq 0 ]; exit
}
ok()  { if want "$1"; then pass=$((pass + 1)); echo "ok   $1"; keep_one "$1"; note_sel "$1"; else sel_gate; fi; }
bad() { if want "$1"; then fail=$((fail + 1)); echo "FAIL $1"; [ -z "${2:-}" ] || echo "       $2"; keep_one "$1"; note_sel "$1"; else sel_gate; fi; }
run() { sh "$install" --project p --lead lead --home "$T/h" "$@" > "$T/out" 2>&1; }
boots() { grep -c 'exec cv2-s5-server /usr/local/bin/cortex-server bootstrap' "$T/log" || true; }
mode() { stat -c %a "$1"; }
leak() { grep -rl 'FAKE-TOKEN' "$T/out" "$T/log" 2>/dev/null || true; }

sum_path_once() { # file, short colon-terminated label, exact standalone path
  python3 -I - "$1" "$2" "$3" <<'PYCODE'
from pathlib import Path
import sys
lines = Path(sys.argv[1]).read_bytes().splitlines(keepends=True)
expected = [sys.argv[2].encode('ascii') + b'\n', sys.argv[3].encode('ascii') + b'\n']
count = sum(lines[i:i + 2] == expected for i in range(len(lines)))
sys.exit(0 if count == 1 else 1)
PYCODE
}
sum_private_files() { # secret file, public channel files
  python3 -I - "$@" <<'PY'
from pathlib import Path
import sys
secret = Path(sys.argv[1]).read_bytes().strip()
parts = [secret[i:i + 16] for i in range(max(1, len(secret) - 15))]
for name in sys.argv[2:]:
    data = Path(name).read_bytes()
    # Refuse whole values and every 16-byte fragment, including formatter folds.
    # Normalization is only for detecting a leak, never for accepting a message.
    for view in (data, data.replace(b'\r', b'').replace(b'\n', b'')):
        if secret in view or any(part and part in view for part in parts):
            sys.exit(1)
PY
}
sum_both_private() { # old value file, new value file, public channels from THIS run
  sb_old=$1; sb_new=$2; shift 2
  sum_private_files "$sb_old" "$@" && sum_private_files "$sb_new" "$@"
}
sum_no_stages() { # private install home; all per-run owned stage names
  python3 -I -c 'import pathlib,sys; sys.exit(any(pathlib.Path(sys.argv[1]).glob("*.token*.part*")))' "$1"
}
sum_no_file_notice() { # public stdout, whether bootstrap ran: 0 or 1
  python3 -I - "$1" "$2" <<'NO_FILE_NOTICE'
import pathlib,sys
text=' '.join(pathlib.Path(sys.argv[1]).read_text().split())
expected=('No new token was made; your existing token files are unchanged.' if sys.argv[2]=='0' else 'No new token file was published; the server may already have replaced the old token: run --reissue-token again.')
sys.exit(0 if text.count(expected)==1 else 1)
NO_FILE_NOTICE
}
sum_path_absent() { # public path, public channels; detect unfolded full paths too
  python3 -I -c 'import pathlib,sys; p=sys.argv[1].encode(); sys.exit(any(p in pathlib.Path(f).read_bytes().replace(b"\r",b"").replace(b"\n",b"") for f in sys.argv[2:]))' "$@"
}
sum_path_total_once() { # file, token path, optional exact approved label/path pairs
  python3 -I - "$@" <<'PYCODE'
from pathlib import Path
import sys
data = Path(sys.argv[1]).read_bytes()
path = sys.argv[2].encode('ascii')
lines = data.splitlines()
if lines.count(path) != 1 or data.replace(b'\r', b'').replace(b'\n', b'').count(path) != 1:
    sys.exit(1)
ignored = {i for i, line in enumerate(lines) if line == path}
if len(sys.argv[3:]) % 2:
    sys.exit(1)
for label, other in zip(sys.argv[3::2], sys.argv[4::2]):
    indexes = [i for i, line in enumerate(lines) if line == other.encode('ascii')]
    if len(indexes) > 1 or any(i == 0 or lines[i - 1] != label.encode('ascii') for i in indexes):
        sys.exit(1)
    ignored.update(indexes)
remaining = b'\n'.join(line for i, line in enumerate(lines) if i not in ignored)
parts = [path[i:i + 16] for i in range(max(1, len(path) - 15))]
for view in (remaining, remaining.replace(b'\r', b'').replace(b'\n', b'')):
    if any(part and part in view for part in parts):
        sys.exit(1)
PYCODE
}

sum_fake_token='ctx2_0123456789abcdef0123456789abcdef.ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopq'
sum_reissue_value='ctx2_fedcba9876543210fedcba9876543210.qrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZabcdef'
# IT-01 first run
if run && [ -f "$T/h/lead.token" ] && [ "$(mode "$T/h/lead.token")" = 600 ] && [ "$(boots)" = 1 ]; then ok "IT-01 first run bootstraps once into a 0600 file"; else bad "IT-01 first run bootstraps once into a 0600 file" "$(tail -3 "$T/out")"; fi
first=$(cat "$T/h/lead.token" 2>/dev/null || echo none)

# IT-02 second run
if run && [ "$(boots)" = 1 ] && [ "$(cat "$T/h/lead.token")" = "$first" ]; then ok "IT-02 a second run does not bootstrap again and leaves the token alone"; else bad "IT-02 a second run does not bootstrap again and leaves the token alone" "$(tail -3 "$T/out")"; fi

# IT-03 reissue
: > "$T/log"
if run --reissue-token; then
  new=$(ls "$T/h"/lead.token.* 2>/dev/null | head -1 || true)
  if [ -n "$new" ] && [ "$(mode "$new")" = 600 ] && [ "$(cat "$new")" != "$first" ] && [ "$(cat "$T/h/lead.token")" = "$first" ] && [ "$(boots)" = 1 ] \
     && grep -q "$new" "$T/out"; then ok "IT-03 --reissue-token mints into a NEW 0600 file, names its path, leaves the old file alone"; else bad "IT-03 --reissue-token mints into a NEW 0600 file, names its path, leaves the old file alone" "new=$new boots=$(boots)"; fi
else bad "IT-03 --reissue-token mints into a NEW 0600 file, names its path, leaves the old file alone" "$(tail -3 "$T/out")"; fi

# IT-04 failed first bootstrap leaves nothing behind
rm -rf "$T/h"; : > "$T/log"
if FAKE_BOOTSTRAP_FAIL=1 run; then bad "IT-04 a failed first bootstrap fails and leaves no token file" "exit 0"; else
  if [ "$(boots)" = 1 ] && [ -z "$(ls "$T/h"/lead.token* 2>/dev/null || true)" ] && grep -q 'bootstrap failed' "$T/out"; then ok "IT-04 a failed first bootstrap fails and leaves no token file"; else bad "IT-04 a failed first bootstrap fails and leaves no token file" "boots=$(boots) files=$(ls "$T/h" 2>/dev/null)"; fi
fi

# IT-05 failed reissue leaves nothing behind and the old token alone
rm -rf "$T/h"; run >/dev/null 2>&1 || true; first=$(cat "$T/h/lead.token" 2>/dev/null || echo none); : > "$T/log"
if FAKE_BOOTSTRAP_FAIL=1 run --reissue-token; then bad "IT-05 a failed reissue fails, leaves no new file and the old token alone" "exit 0"; else
  if [ "$(boots)" = 1 ] && [ -z "$(ls "$T/h"/lead.token.* 2>/dev/null || true)" ] && [ "$(cat "$T/h/lead.token")" = "$first" ]; then ok "IT-05 a failed reissue fails, leaves no new file and the old token alone"; else bad "IT-05 a failed reissue fails, leaves no new file and the old token alone" "boots=$(boots) files=$(ls "$T/h" 2>/dev/null)"; fi
fi

# IT-06 reissue with an unhealthy stack: bounded wait, clear reason, no bootstrap
: > "$T/log"
if FAKE_HEALTH=missing run --reissue-token; then bad "IT-06 reissue on an unhealthy stack stops with a reason and does not bootstrap" "exit 0"; else
  if [ "$(boots)" = 0 ] && grep -q 'not healthy' "$T/out"; then ok "IT-06 reissue on an unhealthy stack stops with a reason and does not bootstrap"; else bad "IT-06 reissue on an unhealthy stack stops with a reason and does not bootstrap" "$(tail -2 "$T/out")"; fi
fi

# IT-07 reissue starts nothing and creates no secret
rm -rf "$T/h"; run >/dev/null 2>&1 || true; : > "$T/log"
if run --reissue-token && [ "$(boots)" = 1 ] && ! grep -q 'compose -f' "$T/log" && ! grep -q 'secret create' "$T/log"; then ok "IT-07 reissue does not start the stack or create secrets"; else bad "IT-07 reissue does not start the stack or create secrets" "boots=$(boots) $(grep -E 'compose -f|secret create' "$T/log" | head -2)"; fi

# IT-08 nothing secret in the output or in the call log
if [ -z "$(leak)" ]; then ok "IT-08 no token in any output or call log"; else bad "IT-08 no token in any output or call log" "$(leak)"; fi
if ! grep -Eq '[0-9a-f]{40,}' "$T/log"; then ok "IT-09 no long random value in any podman argument"; else bad "IT-09 no long random value in any podman argument" "$(grep -E '[0-9a-f]{40,}' "$T/log" | head -1)"; fi

# IT-10 (nix B1) the token copy fails once: nothing is left behind, and the next run recovers
rm -rf "$T/h"; rm -f "$T/state/catfailed"; : > "$T/log"
if FAKE_CAT_FAIL_ONCE=1 run; then bad "IT-10 a failed token copy fails and leaves no token file, and the next run recovers" "first run exited 0"; else
  left=$(ls "$T/h"/lead.token* 2>/dev/null || true); msg=$(tail -1 "$T/out")
  if [ "$(boots)" = 1 ] && [ -z "$left" ] && case $msg in *"token copy failed"*) true ;; *) false ;; esac && FAKE_CAT_FAIL_ONCE=1 run && [ -s "$T/h/lead.token" ] && [ "$(mode "$T/h/lead.token")" = 600 ]; then ok "IT-10 a failed token copy fails and leaves no token file, and the next run recovers"; else bad "IT-10 a failed token copy fails and leaves no token file, and the next run recovers" "boots=$(boots) left=$left msg=$msg"; fi
fi

# IT-11 the copy succeeds but is EMPTY: refused, nothing left behind
rm -rf "$T/h"; : > "$T/log"
if FAKE_CAT_EMPTY=1 run; then bad "IT-11 an empty token copy is refused and leaves no token file" "exit 0"; else
  if [ "$(boots)" = 1 ] && [ -z "$(ls "$T/h"/lead.token* 2>/dev/null || true)" ] && grep -q 'empty' "$T/out"; then ok "IT-11 an empty token copy is refused and leaves no token file"; else bad "IT-11 an empty token copy is refused and leaves no token file" "boots=$(boots) $(ls "$T/h" 2>/dev/null) $(tail -1 "$T/out")"; fi
fi

# IT-12 an EMPTY token file left by an older script is treated as missing, not as "already done"
if rm -rf "$T/h" && mkdir -p "$T/h" && : > "$T/h/lead.token" && : > "$T/log"; then
if run && [ -s "$T/h/lead.token" ] && [ "$(mode "$T/h/lead.token")" = 600 ] && [ "$(boots)" = 1 ]; then ok "IT-12 an empty token file is treated as missing and replaced"; else bad "IT-12 an empty token file is treated as missing and replaced" "boots=$(boots) size=$(wc -c < "$T/h/lead.token" 2>/dev/null) $(tail -1 "$T/out")"; fi
else bad "IT-12 fixture setup" "could not prepare the dependent fixture"; fi

# IT-13 (nix N1) the random source fails: no secret is created
rm -rf "$T/h"; rm -f "$T/state"/secret-*; : > "$T/log"
if PATH="$T/bin-badrand:$PATH" run; then bad "IT-13 a failing random source stops the install before any secret is created" "exit 0"; else
  if ! grep -q 'secret create' "$T/log" && grep -q 'random' "$T/out"; then ok "IT-13 a failing random source stops the install before any secret is created"; else bad "IT-13 a failing random source stops the install before any secret is created" "$(grep 'secret create' "$T/log" | head -1) $(tail -1 "$T/out")"; fi
fi

# IT-14 (nix N4) names are validated before anything is touched
badnames=0
for v in '../x' 'A' 'a b' 'x;y' '' 'a_b' 'abcdefghijklmnopqrstuvwxyz0123456789'; do
  rm -rf "$T/h"; : > "$T/log"
  if sh "$install" --project p --lead "$v" --home "$T/h" >/dev/null 2>&1; then badnames=$((badnames + 1)); echo "       lead accepted: [$v]"; fi
  if [ -e "$T/h" ] || grep -qE 'exec|secret|compose' "$T/log"; then badnames=$((badnames + 1)); echo "       lead [$v] touched something"; fi
done
for v in 'p;x' 'P' '../p' 'p_q' ''; do
  rm -rf "$T/h"; : > "$T/log"
  if sh "$install" --project "$v" --lead lead --home "$T/h" >/dev/null 2>&1; then badnames=$((badnames + 1)); echo "       project accepted: [$v]"; fi
done
if [ "$badnames" = 0 ]; then ok "IT-14 bad --lead and --project names are refused before anything is touched"; else bad "IT-14 bad --lead and --project names are refused before anything is touched" "$badnames problem(s)"; fi

# IT-15 (cox N7) a failed migration stops the install before any token is minted
rm -rf "$T/h"; : > "$T/log"
if FAKE_MIGRATE_FAIL=1 run; then bad "IT-15 a failed migration stops the install and mints no token" "exit 0"; else
  if [ "$(boots)" = 0 ] && grep -q 'migrations failed' "$T/out" && [ -z "$(ls "$T/h"/lead.token* 2>/dev/null || true)" ]; then ok "IT-15 a failed migration stops the install and mints no token"; else bad "IT-15 a failed migration stops the install and mints no token" "boots=$(boots) $(tail -1 "$T/out")"; fi
fi

# IT-16 a failed 'compose up' is named, and nothing is minted
rm -rf "$T/h"; : > "$T/log"
if FAKE_COMPOSE_FAIL=1 run; then bad "IT-16 a failed compose up is named and nothing is minted" "exit 0"; else
  if [ "$(boots)" = 0 ] && grep -q 'compose up failed' "$T/out"; then ok "IT-16 a failed compose up is named and nothing is minted"; else bad "IT-16 a failed compose up is named and nothing is minted" "boots=$(boots) $(tail -1 "$T/out")"; fi
fi

# IT-17 the install reads the migrate container's state (it is a one-shot, so it has no health status)
rm -rf "$T/h"; : > "$T/log"; run >/dev/null 2>&1 || true
if grep -q 'inspect --format {{.State.Status}} {{.State.ExitCode}} cv2-s5-migrate' "$T/log"; then ok "IT-17 the install checks the exit code of the migrate container"; else bad "IT-17 the install checks the exit code of the migrate container" "$(grep inspect "$T/log" | head -2)"; fi

# IT-18 (cox, finn's role design) the order: db up and healthy, then the module roles, then migrate and the server
rm -rf "$T/h"; rm -f "$T/state/roles.sql"; : > "$T/log"; run >/dev/null 2>&1 || true
l_db=$(grep -n 'compose -f .* up -d db$' "$T/log" | head -1 | cut -d: -f1)
l_roles=$(grep -n 'exec -i --user postgres cv2-s5-db psql' "$T/log" | head -1 | cut -d: -f1)
l_build=$(grep -n 'compose -f .* build server$' "$T/log" | head -1 | cut -d: -f1)
l_full=$(grep -n 'compose -f .* up -d$' "$T/log" | head -1 | cut -d: -f1)
if [ -n "$l_db" ] && [ -n "$l_roles" ] && [ -n "$l_build" ] && [ -n "$l_full" ] && [ "$l_db" -lt "$l_roles" ] && [ "$l_roles" -lt "$l_build" ] && [ "$l_build" -lt "$l_full" ]; then ok "IT-18 the order is db, module roles, IMAGE BUILD, then migrate and the server"; else bad "IT-18 the order is db, module roles, IMAGE BUILD, then migrate and the server" "db=$l_db roles=$l_roles build=$l_build full=$l_full"; fi

# IT-19 the SQL: one idempotent role per enabled module, granted to cortex_app, no password, nothing through argv
missing=""
for id in audit identity tokens policy records search handoffs boot; do
  grep -q "cortex_m_$id NOLOGIN" "$T/state/roles.sql" 2>/dev/null || missing="$missing $id"
  grep -q "GRANT cortex_m_$id TO cortex_app" "$T/state/roles.sql" 2>/dev/null || missing="$missing grant:$id"
done
if [ -z "$missing" ] && grep -q 'IF NOT EXISTS' "$T/state/roles.sql" && ! grep -qi 'password' "$T/state/roles.sql" && ! grep -qE '(^|[^A-Z])(CREATEROLE|SUPERUSER)' "$T/state/roles.sql" && ! grep -q 'cortex_m_' "$T/log"; then ok "IT-19 each enabled module gets an idempotent NOLOGIN role granted to cortex_app, with no password and nothing in argv"; else bad "IT-19 each enabled module gets an idempotent NOLOGIN role granted to cortex_app, with no password and nothing in argv" "missing:$missing"; fi

# IT-20 a run that is not the finished one repeats the step and still succeeds (the health route cannot be asked, so the install is not "finished": IT-110 covers the finished one)
: > "$T/log"; rm -f "$T/state/roles.sql"
if FAKE_HEALTH_DOC_FAIL=1 run && grep -q 'exec -i --user postgres cv2-s5-db psql' "$T/log"; then ok "IT-20 a second run that is not the finished one repeats the role step and succeeds"; else bad "IT-20 a second run repeats the role step and succeeds" "$(tail -1 "$T/out")"; fi

# IT-21 a hostile module id in the config is refused before the database is touched
if rm -rf "$T/h" && mkdir -p "$T/h" && : > "$T/log" &&
printf '[modules]\nenabled = ["audit", "x'"'"'; DROP ROLE postgres; --"]\n' > "$T/h/cortex.toml"; then
if run; then bad "IT-21 a hostile module id is refused before the database is touched" "exit 0"; else
  if ! grep -q 'exec -i --user postgres cv2-s5-db psql' "$T/log" && grep -q 'module id' "$T/out"; then ok "IT-21 a hostile module id is refused before the database is touched"; else bad "IT-21 a hostile module id is refused before the database is touched" "$(tail -1 "$T/out")"; fi
fi
else bad "IT-21 fixture setup" "could not prepare the dependent fixture"; fi

# IT-22 a failing role step stops the install before migrate and the server
rm -rf "$T/h"; : > "$T/log"
if FAKE_PSQL_FAIL=1 run; then bad "IT-22 a failing role step stops the install before migrate, the server and any token" "exit 0"; else
  if ! grep -qE 'build server$|up -d$' "$T/log" && [ "$(boots)" = 0 ] && grep -q 'roles' "$T/out"; then ok "IT-22 a failing role step stops the install before migrate, the server and any token"; else bad "IT-22 a failing role step stops the install before migrate, the server and any token" "boots=$(boots) $(tail -1 "$T/out")"; fi
fi

# IT-23 cortex_owner has no standing CREATEROLE
if ! grep -v '^[[:space:]]*#' "$here/../deploy/initdb/00-cortex-roles.sh" | grep -qi 'createrole'; then ok "IT-23 cortex_owner is created without CREATEROLE"; else bad "IT-23 cortex_owner is created without CREATEROLE" "deploy/initdb/00-cortex-roles.sh still grants it"; fi

# IT-24 (nix r3, item a) a failed image build is named and stops the install before migrate, the server and any token
rm -rf "$T/h"; : > "$T/log"
if FAKE_BUILD_FAIL=1 run; then bad "IT-24 a failed image build stops the install before migrate, the server and any token" "exit 0"; else
  if ! grep -q 'up -d$' "$T/log" && [ "$(boots)" = 0 ] && grep -q 'image build failed' "$T/out"; then ok "IT-24 a failed image build stops the install before migrate, the server and any token"; else bad "IT-24 a failed image build stops the install before migrate, the server and any token" "boots=$(boots) $(tail -1 "$T/out")"; fi
fi

# IT-25 (nix N17) the module ids come from the [modules] table, not from the first "enabled" line anywhere
if rm -rf "$T/h" && mkdir -p "$T/h" && rm -f "$T/state/roles.sql" && : > "$T/log" &&
printf '[providers]\nenabled = ["evil"]\n\n[modules]\nenabled = ["audit", "identity"]\n' > "$T/h/cortex.toml"; then
if run && grep -q 'cortex_m_audit NOLOGIN' "$T/state/roles.sql" && grep -q 'cortex_m_identity NOLOGIN' "$T/state/roles.sql" && ! grep -q 'cortex_m_evil' "$T/state/roles.sql"; then ok "IT-25 the module ids come from the [modules] table, not from an earlier table"; else bad "IT-25 the module ids come from the [modules] table, not from an earlier table" "$(grep -o 'cortex_m_[a-z]*' "$T/state/roles.sql" 2>/dev/null | sort -u | tr '\n' ' ')"; fi
else bad "IT-25 fixture setup" "could not prepare the dependent fixture"; fi

# IT-26 an "enabled" list outside [modules] is not a module list
if rm -rf "$T/h" && mkdir -p "$T/h" && : > "$T/log" &&
printf '[providers]\nenabled = ["x"]\n' > "$T/h/cortex.toml"; then
if run; then bad "IT-26 an enabled list outside [modules] is refused as no module list" "exit 0"; else
  if ! grep -q 'psql' "$T/log" && grep -q 'no \[modules\] enabled list' "$T/out"; then ok "IT-26 an enabled list outside [modules] is refused as no module list"; else bad "IT-26 an enabled list outside [modules] is refused as no module list" "$(tail -1 "$T/out")"; fi
fi
else bad "IT-26 fixture setup" "could not prepare the dependent fixture"; fi

# IT-27 (nix N18) an empty list says it is empty
if rm -rf "$T/h" && mkdir -p "$T/h" && : > "$T/log" &&
printf '[modules]\nenabled = []\n' > "$T/h/cortex.toml"; then
if run; then bad "IT-27 an empty module list says it is empty" "exit 0"; else
  if grep -q 'is empty' "$T/out"; then ok "IT-27 an empty module list says it is empty"; else bad "IT-27 an empty module list says it is empty" "$(tail -1 "$T/out")"; fi
fi
else bad "IT-27 fixture setup" "could not prepare the dependent fixture"; fi

# IT-28 (vera's boundaries) the role SQL fixes the attributes, refuses unsafe existing roles, and runs as an explicit OS user
rm -rf "$T/h"; rm -f "$T/state/roles.sql"; : > "$T/log"; run >/dev/null 2>&1 || true
miss=""
for w in NOLOGIN NOSUPERUSER NOCREATEROLE NOCREATEDB NOBYPASSRLS NOREPLICATION 'RAISE EXCEPTION' pg_auth_members pg_has_role rolinherit; do
  grep -q "$w" "$T/state/roles.sql" 2>/dev/null || miss="$miss [$w]"
done
grep -q "exec -i --user postgres cv2-s5-db psql" "$T/log" || miss="$miss [--user postgres]"
if [ -z "$miss" ]; then ok "IT-28 the role SQL fixes safe attributes, refuses unsafe existing roles and runs as an explicit OS user"; else bad "IT-28 the role SQL fixes safe attributes, refuses unsafe existing roles and runs as an explicit OS user" "missing:$miss"; fi

# IT-29 db-roles.sh may target another container only if it is one of ours (cv2-s5-*): never v1's, never a name with odd characters
dbr="$(dirname -- "$install")/db-roles.sh"   # beside the install.sh under test, so that a mutant copy of deploy/ is the one exercised
DL=2000000000   # cox W1-55: the CALLER passes its absolute deadline E; the script reads no clock of its own, so a fixed E makes every expected value exact (the fake psql does not evaluate the guard)
GB=$((DL - 9))  # the guard's limit: E - lead, lead = idle 5 + commit latency 2 + slack 2
: > "$T/log"; refused=0
for c in cortex-pg cortex-api harness-appdb 'cv2-s5-x;y' 'cv2-s5-' 'CV2-S5-db' '../cv2-s5-db'; do   # s5-never: ok these are the names db-roles.sh must REFUSE; they are test inputs to the fake podman, never run
  : > "$T/log"
  if CORTEX_DB_CONTAINER="$c" sh "$dbr" --deadline "$DL" audit > "$T/out" 2>&1; then refused=$((refused + 1)); echo "       accepted: [$c]"; fi
  if [ -s "$T/log" ]; then refused=$((refused + 1)); echo "       [$c] reached podman"; fi
done
: > "$T/log"; rm -f "$T/state/roles.sql"
CORTEX_DB_CONTAINER=cv2-s5-test-roles sh "$dbr" --deadline "$DL" audit > "$T/out" 2>&1 || true
if [ "$refused" = 0 ] && grep -q "exec -i --user postgres --env PGOPTIONS=-c transaction_timeout=30s -c idle_in_transaction_session_timeout=5s -c db_roles.guard_by=$GB cv2-s5-test-roles psql" "$T/log"; then ok "IT-29 CORTEX_DB_CONTAINER may name only a cv2-s5-* container; v1's names and odd names never reach podman"; else bad "IT-29 CORTEX_DB_CONTAINER may name only a cv2-s5-* container; v1's names and odd names never reach podman" "problems=$refused log=$(head -1 "$T/log")"; fi

# IT-67 (cox 19:41Z, mike R6; A106) the role script's transaction cannot outlive a budget SHORTER than the engine's run_roles deadline: the psql session gets
# transaction_timeout=30s from the SERVER side through PGOPTIONS on the podman exec (the engine's deadline is at least 45 s: design r13), and not through a SET inside the SQL
: > "$T/log"; CORTEX_DB_CONTAINER=cv2-s5-test-roles sh "$dbr" --deadline "$DL" audit > "$T/out" 2>&1 || true
n67=$(grep -c -- "--env PGOPTIONS=-c transaction_timeout=30s -c idle_in_transaction_session_timeout=5s -c db_roles.guard_by=$GB cv2-s5-test-roles psql" "$T/log" || true)
if [ "$n67" = 1 ] && ! grep -q 'SET LOCAL' "$T/state/roles.sql" 2>/dev/null; then ok "IT-67 db-roles.sh gives its psql session transaction_timeout=30s through PGOPTIONS (once), and no SET LOCAL"; else bad "IT-67 db-roles.sh gives its psql session transaction_timeout=30s through PGOPTIONS" "n=$n67: $(tail -2 "$T/log" | cut -c1-200)"; fi
# IT-68 (A97) no shell case pattern of db-roles.sh uses a character range (the locale decides what a range holds). The pattern lives in a function so that IT-68b holds it.
case_range_hits() { grep -nE '^[[:space:]]*(for .*; do )?case .*\[[!^]?([0-9A-Za-z]-[0-9A-Za-z]|[^] ][^]]*[0-9A-Za-z]-[0-9A-Za-z])'; }
if ! case_range_hits < "$dbr" > "$T/ranges68"; then ok "IT-68 no case pattern in db-roles.sh uses a character range"; else bad "IT-68 no case pattern in db-roles.sh uses a character range" "$(head -3 "$T/ranges68" | cut -c1-150)"; fi
# IT-69 (A97) the module-id and container-name classes of db-roles.sh accept exactly their alphabets (C and two UTF-8 locales when the host has them)
b69=""
through() { # through <locale> <container> <id>: 0 when db-roles.sh reached podman
  : > "$T/log"; ( LC_ALL=$1; export LC_ALL; CORTEX_DB_CONTAINER=$2 sh "$dbr" --deadline "$DL" "$3" > "$T/out" 2>&1 ) || true; [ -s "$T/log" ]; }
for loc in C C.utf8 en_AU.utf8; do
  for id in z a9 a_ a_9 zz; do through "$loc" cv2-s5-test-roles "$id" || b69="$b69 [$loc: id $id refused]"; done
  for id in A 9a _a a-b 'a b' é a.b Ａ ４a a４ a٥; do through "$loc" cv2-s5-test-roles "$id" && b69="$b69 [$loc: id $id accepted]"; done
  for c in cv2-s5-z9 cv2-s5-9 cv2-s5-a-b; do through "$loc" "$c" audit || b69="$b69 [$loc: container $c refused]"; done
  for c in cv2-s5-x_ cv2-s5-A cv2-s5-é cv2-s5-Ａ cv2-s5-x.y cv2-s5-４; do through "$loc" "$c" audit && b69="$b69 [$loc: container $c accepted]"; done
done
if [ -z "$b69" ]; then ok "IT-69 the module-id and container-name classes of db-roles.sh accept exactly their alphabets"; else bad "IT-69 the module-id and container-name classes of db-roles.sh accept exactly their alphabets" "$b69"; fi

# IT-70 (cox W1-55; mike's M-4) the session gets the THREE options, once each, with the guard's limit E - lead (lead = 9) computed from the CALLER's deadline E; the script reads no clock
: > "$T/log"; CORTEX_DB_CONTAINER=cv2-s5-test-roles sh "$dbr" --deadline "$DL" audit > "$T/out" 2>&1 || true
n70=$(grep -c -- "--env PGOPTIONS=-c transaction_timeout=30s -c idle_in_transaction_session_timeout=5s -c db_roles.guard_by=$GB cv2-s5-test-roles psql" "$T/log" || true)
n70b=$(grep -c -- '--env PGOPTIONS=' "$T/log" || true)
if [ "$n70" = 1 ] && [ "$n70b" = 1 ]; then ok "IT-70 the session gets transaction_timeout=30s, idle_in_transaction_session_timeout=5s and guard_by = E - 9, once each"; else bad "IT-70 the session gets transaction_timeout=30s, idle_in_transaction_session_timeout=5s and guard_by = E - 9, once each" "matching=$n70 pgoptions lines=$n70b log=$(tail -1 "$T/log" | cut -c1-230)"; fi
# IT-71 the LAST statement of the roles SQL is the deadline guard, exactly, after every role and grant; it reads the DATABASE clock (clock_timestamp, not now()) and a missing setting fails closed
cat > "$T/guard71" <<'GUARD'
DO $$ BEGIN
  IF extract(epoch FROM clock_timestamp()) > current_setting('db_roles.guard_by')::float8 THEN
    RAISE EXCEPTION 'db-roles: past its deadline; nothing was committed';
  END IF;
END $$;
GUARD
: > "$T/log"; rm -f "$T/state/roles.sql"   # the fake psql APPENDS to roles.sql: start from nothing
CORTEX_DB_CONTAINER=cv2-s5-test-roles sh "$dbr" --deadline "$DL" audit identity > "$T/out" 2>&1 || true
last_grant71=$(grep -n '^GRANT cortex_m_' "$T/state/roles.sql" 2>/dev/null | tail -n 1 | cut -d: -f1)
guard_at71=$(grep -n 'clock_timestamp' "$T/state/roles.sql" 2>/dev/null | cut -d: -f1)
if tail -n 5 "$T/state/roles.sql" 2>/dev/null | cmp -s - "$T/guard71" && [ "$(grep -c 'clock_timestamp' "$T/state/roles.sql")" = 1 ] && [ -n "$last_grant71" ] && [ "$last_grant71" -lt "$guard_at71" ]; then ok "IT-71 the roles SQL ends with the deadline guard, exactly, after the last grant"; else bad "IT-71 the roles SQL ends with the deadline guard, exactly, after the last grant" "last grant at line '${last_grant71:-none}', guard at '${guard_at71:-none}'; tail: $(tail -n 2 "$T/state/roles.sql" 2>/dev/null | tr '\n' '~' | cut -c1-160)"; fi
# IT-72 EACH TERM of the relation moves the guard (a cell on each term, cox W1-55): idle 6, latency 3 or slack 3 gives lead 10, so guard_by = E - 10, and the idle option follows the idle term
b72=""
for pair in 'idle=5:idle=6:6s:10' 'latency=2:latency=3:5s:10' 'slack=2:slack=3:5s:10'; do
  old72=${pair%%:*}; rest72=${pair#*:}; new72=${rest72%%:*}; rest72=${rest72#*:}; idle72=${rest72%%:*}; lead72=${rest72#*:}
  sed "s/^$old72\$/$new72/" "$dbr" > "$T/dbr72.sh"
  : > "$T/log"; CORTEX_DB_CONTAINER=cv2-s5-test-roles sh "$T/dbr72.sh" --deadline "$DL" audit > "$T/out" 2>&1 || true
  grep -q -- "-c idle_in_transaction_session_timeout=$idle72 -c db_roles.guard_by=$((DL - lead72)) cv2-s5-test-roles psql" "$T/log" || b72="$b72 [$new72: $(tail -1 "$T/log" | cut -c1-200)]"
done
if [ -z "$b72" ]; then ok "IT-72 each term of the relation (idle, latency, slack) moves the guard: guard_by = E - (idle + latency + slack), and the idle option is the idle term"; else bad "IT-72 each term of the relation moves the guard" "$b72"; fi
# IT-73 the deadline is a required, validated argument: nothing reaches podman without a clean one (digits only, no leading zero, at most 11), in three locales
b73=""
refused73() { : > "$T/log"; ( LC_ALL=$1; export LC_ALL; shift; CORTEX_DB_CONTAINER=cv2-s5-test-roles sh "$dbr" "$@" > "$T/out" 2>&1 ) && return 1; [ ! -s "$T/log" ]; }
for loc in C C.utf8 en_AU.utf8; do
  refused73 "$loc" || b73="$b73 [$loc: no arguments reached podman]"
  refused73 "$loc" audit || b73="$b73 [$loc: no --deadline]"
  refused73 "$loc" --deadline || b73="$b73 [$loc: --deadline alone]"
  refused73 "$loc" --deadline 2000000000 || b73="$b73 [$loc: no module ids]"
  refused73 "$loc" audit --deadline 2000000000 || b73="$b73 [$loc: --deadline not first]"
  refused73 "$loc" audit 2000000000 audit || b73="$b73 [$loc: a number in the second place without the --deadline flag]"
  refused73 "$loc" --dline 2000000000 audit || b73="$b73 [$loc: a misspelt flag]"
  refused73 "$loc" -- 2000000000 audit || b73="$b73 [$loc: -- instead of --deadline]"
  for bad_e in abc "" 0123456789 0200000000 123456789012 -5 1e9 2000000000x '2000000000 ' ５１２３４５６７８９０; do
    refused73 "$loc" --deadline "$bad_e" audit || b73="$b73 [$loc: deadline '$bad_e' reached podman]"
  done
done
: > "$T/log"; ( CORTEX_DB_CONTAINER=cv2-s5-test-roles sh "$dbr" --deadline 2000000000 audit > "$T/out" 2>&1 ) || b73="$b73 [a clean deadline was refused]"
if [ -z "$b73" ]; then ok "IT-73 the deadline is a required, validated argument; nothing reaches podman without a clean one"; else bad "IT-73 the deadline is a required, validated argument; nothing reaches podman without a clean one" "$b73"; fi
# IT-74 install.sh is the caller of its own run: it passes E = its own now + 60, once, so guard_by is now + 51
rm -rf "$T/h"; : > "$T/log"; t1=$(date +%s); run >/dev/null 2>&1 || true; t2=$(date +%s)
gb74=$(grep -o -- 'db_roles.guard_by=[0123456789-]*' "$T/log" | sed -n 1p); gb74=${gb74#db_roles.guard_by=}
n74=$(grep -c -- 'db_roles.guard_by=' "$T/log" || true)
case $gb74 in ""|*[!0123456789-]*) gb74="" ;; esac
if [ "$n74" = 1 ] && [ -n "$gb74" ] && [ "$gb74" -ge $((t1 + 60 - 9)) ] && [ "$gb74" -le $((t2 + 60 - 9)) ]; then ok "IT-74 install.sh passes its own now plus 60 as the deadline, once"; else bad "IT-74 install.sh passes its own now plus 60 as the deadline, once" "calls=$n74 guard_by='$gb74' window=$((t1 + 51))..$((t2 + 51))"; fi
# IT-75 the script reads NO clock of its own (no `date` in its code) and writes the relation once: one idle=, latency=, slack= and lead= line, the guard's limit once
nd75=$(grep -v '^[[:space:]]*#' "$dbr" | grep -cw 'date' || true)
ni75=$(grep -c '^idle=5$' "$dbr" || true); nl75=$(grep -c '^latency=2$' "$dbr" || true); ns75=$(grep -c '^slack=2$' "$dbr" || true)
nf75=$(grep -c '^lead=\$((idle + latency + slack))$' "$dbr" || true); ng75=$(grep -c '^guard_by=\$((deadline - lead))$' "$dbr" || true)
if [ "$nd75" = 0 ] && [ "$ni75" = 1 ] && [ "$nl75" = 1 ] && [ "$ns75" = 1 ] && [ "$nf75" = 1 ] && [ "$ng75" = 1 ]; then ok "IT-75 db-roles.sh reads no clock of its own and writes idle, latency, slack, lead and guard_by once each"; else bad "IT-75 db-roles.sh reads no clock of its own and writes idle, latency, slack, lead and guard_by once each" "date lines=$nd75 idle=$ni75 latency=$nl75 slack=$ns75 lead=$nf75 guard_by=$ng75"; fi

# IT-80 (hale 5i F-1, F-2; 5m F-3) the roles SQL is ONE transaction and psql is called exactly one way: no statement ends or splits the transaction (COMMIT, ROLLBACK, ABORT, SAVEPOINT, END;, START TRANSACTION), no psql
# meta-command (a backslash line could turn ON_ERROR_STOP off), no statement that sets an option (set_config, however its name is split by printf, or a line-leading SET: comment lines are skipped, the SQL mentions
# "SET ROLE" in one), and the exec's psql arguments are exactly `-v ON_ERROR_STOP=1 -1 -U postgres -d cortex -q` (the guard is only the last statement of ONE transaction). Text rules cannot see a split name; the generated SQL can.
: > "$T/log"; rm -f "$T/state/roles.sql"
CORTEX_DB_CONTAINER=cv2-s5-test-roles sh "$dbr" --deadline "$DL" audit identity tokens > "$T/out" 2>&1 || true
bad80=$(grep -nEi '^[[:space:]]*(COMMIT|ROLLBACK|ABORT|SAVEPOINT|START[[:space:]]+TRANSACTION|END[[:space:]]*;)|^[[:space:]]*\\' "$T/state/roles.sql" 2>/dev/null | head -3 | cut -c1-80 || true)
# the lint of the generated SQL for a statement that sets an option or ends the transaction, as a function (IT-82 feeds it fixed lines). It reads the SQL RAW (hale's
# 5n and 5q F-1; vera's F4): no lexer to get out of step with PostgreSQL's quoting. The real SQL (84 lines) holds the phrase SET ROLE in two `--` comment lines and nothing
# else of the list, so a line that STARTS with `--` may hold that phrase (it alone is removed); anything else that holds the word SET, set_config, a backslash ANYWHERE (psql
# takes a meta-command after SQL on the same line), a carriage return (PostgreSQL ends a `--` comment at a CR as well as a LF: vera's F4), U& and UESCAPE (an escaped identifier
# can spell set_config without its letters: vera's F4), COMMIT, ROLLBACK, ABORT, SAVEPOINT, RELEASE, PREPARE, START, CHAIN, EXECUTE, or END that is not END IF, END LOOP, END CASE
# or END $$, is a hit, in code, in a string and in a comment: too wide on purpose, the script as it is has none. The numbered output is the offending lines.
sql_set_hits_stdin() {
  awk '{ l = $0; u = toupper(l)
         if (l ~ /^[ \t]*--/) gsub(/SET ROLE/, "X", u)
         gsub(/END[ \t]+(IF|LOOP|CASE)([^A-Z0-9_]|$)/, "X ", u); gsub(/END[ \t]+\$\$/, "X ", u)
         if (u ~ /SET_CONFIG/ || u ~ /U&/ || u ~ /UESCAPE/ || u ~ /(^|[^A-Z0-9_])(SET|COMMIT|ROLLBACK|ABORT|SAVEPOINT|RELEASE|PREPARE|START|CHAIN|EXECUTE|END)([^A-Z0-9_]|$)/ || index(l, "\\") || index(l, "\r")) print NR ": " substr(l, 1, 70) }'
}
bad80b=$(sql_set_hits_stdin < "$T/state/roles.sql" 2>/dev/null | head -3 | cut -c1-80 || true)
# IT-80b (cox's closed allow-list, 5u; vera's F4 twice): the roles SQL of THIS release is exactly this text (84 lines, its sha256 pinned below) for the module list of this run. A lint that
# parses a language it does not own meets a new shape each time; the SQL does not change unless the script changes, so ANY change of it fails here and needs a ruling (vera and cox), the
# pin being moved with it. The raw-scan lint above stays as a second layer for the day the pin moves.
want_sql_sha=9f39db0ec34a949f8087d47878efc51fff0372b55f63761e18fa994c2306b24c
got_sql_sha=$(sha256sum "$T/state/roles.sql" 2>/dev/null | cut -d' ' -f1)
n80b=$(wc -l < "$T/state/roles.sql" 2>/dev/null || echo 0)
n80=$(grep -c -- ' psql -v ON_ERROR_STOP=1 -1 -U postgres -d cortex -q$' "$T/log" || true)
if [ -z "$bad80" ] && [ -z "$bad80b" ] && [ "$got_sql_sha" = "$want_sql_sha" ] && [ "$n80b" = 84 ] && [ "$n80" = 1 ] && [ -s "$T/state/roles.sql" ]; then ok "IT-80 the roles SQL is the pinned text (84 lines, sha256), has no statement that ends or splits the transaction, no psql meta-command and no statement that sets an option, and psql is called exactly as one -1 transaction on the cortex database"; else bad "IT-80 the roles SQL is one transaction and psql is called exactly one way" "offending: '$bad80' '$bad80b' psql calls=$n80 sql bytes=$(wc -c < "$T/state/roles.sql" 2>/dev/null)"; fi

# IT-82 (vera F4 on 5o part 2, hale's F-1 on 5n and 5q) the generated-SQL lint of IT-80 reads the SQL RAW: it holds the word SET, set_config, a backslash, COMMIT, END TRANSACTION and
# the other words that end a transaction, wherever they stand on a line (code, string, comment or after a quote that a lexer would misread), except the phrase SET ROLE in a comment
# line and END IF, END LOOP, END CASE, END $$. Hale's 21 lines that must hit (the shapes the 5s lexer missed, and the transaction words) and 7 that must stay clean. Defined below
# IT-80's lint; fed fixed lines, as IT-51b feeds the range check.
b82=""
while IFS= read -r want_hit; do
  [ -n "$(printf '%s\n' "$want_hit" | sql_set_hits_stdin)" ] || b82="$b82 [MISSED: $want_hit]"
done <<'HITS'
/* diagnostic */ SET db_roles.guard_by = 99999999999;
SELECT set_config('db_roles.guard_by', '1', true);
SELECT '/*'; SET x = 1; SELECT '*/';
SELECT '--'; SET statement_timeout = 0;
  set statement_timeout = 0;
/* a */ /* b */ SET x = 1;
SELECT 1; /* note */ SET x = 1;
SELECT 1 AS "--"; SET statement_timeout = 0;
SELECT 1 AS "/*"; SET statement_timeout = 0;
SELECT "it's"; SET statement_timeout = 0; SELECT 'x';
SELECT E'\''; SET statement_timeout = 0; --'
SELECT E'a\'b'; SET statement_timeout = 0; SELECT 'c';
SELECT $q$it's$q$; SET statement_timeout = 0; SELECT 'x';
SELECT 1; \unset ON_ERROR_STOP
SELECT 1; \! true
END TRANSACTION;
END WORK;
SELECT 1; COMMIT;
PREPARE TRANSACTION 'x';
ALTER ROLE postgres SET statement_timeout = 0;
SELECT U&"s!0065t_config" UESCAPE '!'('db_roles.guard_by','99999999999',true);
EXECUTE 'SE' || 'T LOCAL statement_timeout = 0';
HITS
# (a hit that holds a carriage return cannot sit in a here-document line: PostgreSQL ends a `--` comment at a CR as well as a LF, so the second SET ROLE below is a STATEMENT)
[ -n "$(printf -- '-- SET ROLE\rSET ROLE postgres;\n' | sql_set_hits_stdin)" ] || b82="$b82 [MISSED: a CR that ends a comment]"
while IFS= read -r want_clean; do
  [ -z "$(printf '%s\n' "$want_clean" | sql_set_hits_stdin)" ] || b82="$b82 [FALSE HIT: $want_clean]"
done <<'CLEAN'
-- a comment about one SET ROLE away
  END IF;
    END LOOP;
END $$;
RAISE EXCEPTION 'unsafe existing role %', r;
SELECT current_setting('db_roles.guard_by');
SELECT offset_val, reset_x FROM t;
CLEAN
if [ -z "$b82" ]; then ok "IT-82 the generated-SQL lint reads the SQL raw: every shape of a SET, a transaction end or a meta-command that a lexer could be talked out of is a hit, and the real SQL shapes stay clean"; else bad "IT-82 the generated-SQL lint reads the SQL raw" "$b82"; fi
# IT-68b (hale rework-2 F-1) the range check of IT-68 is itself held, in the style of IT-51b: a range that STARTS a bracket expression, after the first character, or after a !, is flagged; spelled-out classes,
# a character that merely follows a dash, the ? glob and the PostgreSQL regexes of the SQL text are left alone
f68=""
while IFS= read -r l; do printf '%s\n' "$l" | case_range_hits > /dev/null || f68="$f68 [not flagged: $l]"; done <<'EOF68'
case $id in [a-z]*) ;; esac
case $x in *[0-9]*) ;; esac
case $x in [A-Z]*) ;; esac
for id in "$@"; do case $id in [a-z]*) echo bad;; esac
case $x in [!a-z]*) ;; esac
case $x in *[a-z0-9_]*) ;; esac
EOF68
while IFS= read -r l; do printf '%s\n' "$l" | case_range_hits > /dev/null && f68="$f68 [flagged: $l]"; done <<'EOF68'
case $x in *[!abcdefghijklmnopqrstuvwxyz]*) ;; esac
case $db in cv2-s5-?*) ;; esac
case $c in [a-]*) ;; esac
IF r.granted ~ '^cortex_m_[a-z][a-z0-9_]{0,31}$' THEN
echo "case [a-z] is a range"
EOF68
if [ -z "$f68" ]; then ok "IT-68b the case-range check flags a range that starts a bracket expression and leaves spelled-out classes, a?, and SQL regexes alone"; else bad "IT-68b the case-range check flags a range that starts a bracket expression and leaves spelled-out classes alone" "$f68"; fi
# IT-30 (cox 08dd8528) roles are cluster-wide: the SQL is ONE transaction and takes an advisory lock, so a concurrent run waits instead of racing
rm -rf "$T/h"; rm -f "$T/state/roles.sql"; : > "$T/log"; run >/dev/null 2>&1 || true
if grep -q 'pg_advisory_xact_lock' "$T/state/roles.sql" 2>/dev/null && grep -qE 'psql .* -1 ' "$T/log"; then ok "IT-30 the role SQL runs in one transaction under an advisory lock"; else bad "IT-30 the role SQL runs in one transaction under an advisory lock" "lock=$(grep -c pg_advisory_xact_lock "$T/state/roles.sql" 2>/dev/null) args=$(grep psql "$T/log" | head -1)"; fi

# IT-31 (cox ce1b9137, nix N20) the fixed roles belong to no other role; cortex_install_admin is NOLOGIN
miss=""
for w in 'cortex_install_admin must be NOLOGIN' 'must not be a member of' '^cortex_m_[a-z][a-z0-9_]{0,31}$'; do
  grep -qF -- "$w" "$T/state/roles.sql" 2>/dev/null || miss="$miss [$w]"
done
if [ -z "$miss" ]; then ok "IT-31 the fixed roles may belong only to cortex_install_admin and cortex_m_* roles (owner to none), and cortex_install_admin is NOLOGIN"; else bad "IT-31 the fixed roles may belong only to cortex_install_admin and cortex_m_* roles (owner to none), and cortex_install_admin is NOLOGIN" "missing:$miss"; fi

# IT-32 (nix N19) a multi-line module list is not "empty": the message says to put it on one line
if rm -rf "$T/h" && mkdir -p "$T/h" && : > "$T/log" &&
printf '[modules]\nenabled = [\n  "audit",\n  "identity",\n]\n' > "$T/h/cortex.toml"; then
if run; then bad "IT-32 a multi-line module list is refused with a message that says one line" "exit 0"; else
  if grep -q 'one line' "$T/out" && ! grep -q 'is empty' "$T/out"; then ok "IT-32 a multi-line module list is refused with a message that says one line"; else bad "IT-32 a multi-line module list is refused with a message that says one line" "$(tail -1 "$T/out")"; fi
fi

else bad "IT-32 fixture setup" "could not prepare the dependent fixture"; fi

# did the role step run? Its SQL arrived (roles.sql) or its psql call is in the log, in ANY spelling of the exec line (--env PGOPTIONS since A106; vera's Low S5-ROLES-L1: the old oracle looked for the old line only)
roles_ran() { [ -s "$T/state/roles.sql" ] || grep -qE 'exec -i --user postgres .*cv2-s5-[a-z0-9-]+ psql -v' "$T/log"; }
# IT-33 (vera, cox e0b0b76b) the effective pg_hba rules are checked before the roles step; a database that still has trust rules is refused and NOTHING is recreated or deleted
rm -rf "$T/h"; : > "$T/log"; rm -f "$T/state/roles.sql"
if FAKE_HBA_TRUST=2 run; then bad "IT-33 a database with trust rules is refused before the roles step, and nothing is deleted" "exit 0"; else
  if grep -q 'pg_hba_file_rules' "$T/log" && ! roles_ran && ! grep -qE 'volume (rm|prune)|rm .*--volumes|compose .* down' "$T/log" && grep -q 'trust' "$T/out"; then ok "IT-33 a database with trust rules is refused before the roles step, and nothing is deleted"; else bad "IT-33 a database with trust rules is refused before the roles step, and nothing is deleted" "$(tail -1 "$T/out") | $(grep -c pg_hba_file_rules "$T/log")"; fi
fi

# IT-33b the oracle itself is held: a log with the role step's call (new or old spelling) or a roles.sql counts as "ran"; the HBA query alone and an empty log do not
g33=""
rm -f "$T/state/roles.sql"
: > "$T/log"; roles_ran && g33="$g33 [empty log counted as ran]"
printf '%s\n' 'exec -i --user postgres cv2-s5-db psql -U postgres -d cortex -tAc select count(*) from pg_hba_file_rules' > "$T/log"; roles_ran && g33="$g33 [the HBA query counted as ran]"
printf '%s\n' 'exec -i --user postgres --env PGOPTIONS=-c transaction_timeout=30s cv2-s5-db psql -v ON_ERROR_STOP=1 -1 -U postgres -d cortex -q' > "$T/log"; roles_ran || g33="$g33 [the new spelling not seen]"
printf '%s\n' 'exec -i --user postgres cv2-s5-db psql -v ON_ERROR_STOP=1 -1 -U postgres -d cortex -q' > "$T/log"; roles_ran || g33="$g33 [the old spelling not seen]"
: > "$T/log"; printf 'x\n' > "$T/state/roles.sql"; roles_ran || g33="$g33 [a roles.sql not seen]"; rm -f "$T/state/roles.sql"
if [ -z "$g33" ]; then ok "IT-33b the \"did the role step run\" oracle sees both spellings of the exec line and a roles.sql, and not the HBA query"; else bad "IT-33b the \"did the role step run\" oracle" "$g33"; fi
: > "$T/log"

# IT-34 with no trust rule the check runs (against pg_hba_file_rules, as an explicit OS user) and the install goes on
rm -rf "$T/h"; : > "$T/log"; rm -f "$T/state/roles.sql"
if run && grep -q 'exec -i --user postgres cv2-s5-db psql -U postgres -d cortex -tAc select count' "$T/log" || grep -q 'pg_hba_file_rules' "$T/log"; then
  l_hba=$(grep -n 'pg_hba_file_rules' "$T/log" | head -1 | cut -d: -f1); l_rl=$(grep -n 'psql -v ON_ERROR_STOP=1' "$T/log" | head -1 | cut -d: -f1)
  if [ -n "$l_hba" ] && [ -n "$l_rl" ] && [ "$l_hba" -lt "$l_rl" ]; then ok "IT-34 the effective-rules check runs before the roles step and the install continues"; else bad "IT-34 the effective-rules check runs before the roles step and the install continues" "hba=$l_hba roles=$l_rl"; fi
else bad "IT-34 the effective-rules check runs before the roles step and the install continues" "$(tail -1 "$T/out")"; fi

# IT-35 an unreadable effective-rules query stops the install: it never assumes the database is safe
rm -rf "$T/h"; : > "$T/log"; rm -f "$T/state/roles.sql"
if FAKE_HBA_FAIL=1 run; then bad "IT-35 an unreadable effective-rules query stops the install before the roles step" "exit 0"; else
  if ! grep -q 'psql -v ON_ERROR_STOP=1' "$T/log" && grep -q "could not read the database's access rules" "$T/out"; then ok "IT-35 an unreadable effective-rules query stops the install before the roles step"; else bad "IT-35 an unreadable effective-rules query stops the install before the roles step" "$(tail -1 "$T/out")"; fi
fi

# IT-36 (nix N23) every cortex_m_* role that cortex_app belongs to is attribute-checked, not only the ids in this run's list
rm -rf "$T/h"; rm -f "$T/state/roles.sql"; : > "$T/log"; run >/dev/null 2>&1 || true
if grep -qF 'cortex_app is a member of the unsafe role' "$T/state/roles.sql" 2>/dev/null; then ok "IT-36 every cortex_m_* role cortex_app belongs to is attribute-checked, whether or not its id is in this run's list"; else bad "IT-36 every cortex_m_* role cortex_app belongs to is attribute-checked, whether or not its id is in this run's list" "check missing from the role SQL"; fi

# IT-37 (nix N26) the count of trust rules must be a NUMBER: anything else says it could not read a count (and does not claim "the database has abc trust rules")
badc=0
for raw in abc '' '-1' '1x' ' '; do
  rm -rf "$T/h"; : > "$T/log"
  if FAKE_HBA_RAW="$raw" run; then badc=$((badc + 1)); echo "       accepted: [$raw]"; fi
  if ! grep -q 'could not read a count' "$T/out"; then badc=$((badc + 1)); echo "       [$raw] wrong message: $(tail -1 "$T/out")"; fi
  if grep -q 'psql -v ON_ERROR_STOP=1' "$T/log"; then badc=$((badc + 1)); echo "       [$raw] reached the roles step"; fi
done
rm -rf "$T/h"; : > "$T/log"
if FAKE_HBA_RAW="00" run; then :; else badc=$((badc + 1)); echo "       [00] is the number zero and must pass: $(tail -1 "$T/out")"; fi
if [ "$badc" = 0 ]; then ok "IT-37 a trust-rule count that is not a number is refused as unreadable, and 00 counts as zero"; else bad "IT-37 a trust-rule count that is not a number is refused as unreadable, and 00 counts as zero" "$badc problem(s)"; fi

# IT-38 (nix N28) nobody but cortex_app may belong to a cortex_m_* role (a login role granted one could SET ROLE into it)
rm -rf "$T/h"; rm -f "$T/state/roles.sql"; : > "$T/log"; run >/dev/null 2>&1 || true
if grep -qF 'must not be a member of the module role' "$T/state/roles.sql" 2>/dev/null; then ok "IT-38 no role other than cortex_app may belong to a cortex_m_* role"; else bad "IT-38 no role other than cortex_app may belong to a cortex_m_* role" "check missing from the role SQL"; fi

# ---- A78 (S5 amendments 15 to 17; cox W1-REISSUE): --reissue-token passes the server's refuse-if-missing flag ------------------
# The fake server above follows piece 11 contract r0.3. THE fixed refusal line is held here, once, copied from the contract
# (pieces/11-cortex-server-CONTRACT-r0.md, sha256 prefix 3edf19653df7); the fake prints it from $FAKE_REFUSAL.
REFUSED='bootstrap refused: --require-existing, and the project, the lead, or their pairing does not exist'
FAKE_REFUSAL=$REFUSED; export FAKE_REFUSAL
seed() { rm -rf "$T/h"; : > "$T/log"; rm -f "$T/state/count" "$T/state/bootstrap.exit" "$T/state/bootstrap.token"; printf 'a:lead-a\nb:lead-b\n' > "$T/state/projects"; cp "$T/state/projects" "$T/seed"; }
runpl() { rp=$1; rl=$2; shift 2; sh "$install" --project "$rp" --lead "$rl" --home "$T/h" "$@" > "$T/out" 2>&1; }
flags() { grep -o -- '--require-existing' "$T/log" | wc -l | tr -d ' '; }
code() { cat "$T/state/bootstrap.exit" 2>/dev/null || echo none; }
nofiles() { [ -z "$(ls -A "$T/h" 2>/dev/null | grep -E 'token' || true)" ]; }

# IT-39 reissue for an existing project: one new 0600 file, the flag exactly once and literally, the table untouched.
# (a hostile environment must not be able to change the flag: CORTEX_REQUIRE_FLAG is set to something else)
seed
if CORTEX_REQUIRE_FLAG='--evil' runpl a lead-a --reissue-token \
   && [ "$(ls "$T/h"/lead-a.token.* 2>/dev/null | wc -l | tr -d ' ')" = 1 ] && [ "$(mode "$(ls "$T/h"/lead-a.token.*)")" = 600 ] \
   && [ "$(flags)" = 1 ] && ! grep -q -- '--evil' "$T/log" && cmp -s "$T/seed" "$T/state/projects"; then ok "IT-39 --reissue-token for an existing project passes --require-existing once, literally, and creates nothing"; else bad "IT-39 --reissue-token for an existing project passes --require-existing once, literally, and creates nothing" "flags=$(flags) code=$(code): $(tail -2 "$T/out")"; fi

# IT-40..42 the server refuses (exit 3, the fixed line): install.sh exits 1 with its own reason, the line is shown once, nothing is written
for tc in "nosuch lead-a|IT-40 a project that does not exist" "a lead-b|IT-41 a lead that is not the project's lead" "a nosuch-lead|IT-42 a lead that does not exist"; do
  args=${tc%%|*}; label=${tc#*|}; seed; set -- $args
  runpl "$1" "$2" --reissue-token && rc=0 || rc=$?
  if [ "$rc" = 7 ] && [ "$(code)" = 3 ] && [ "$(grep -c -F "$REFUSED" "$T/out")" = 1 ] && grep -q 'install: bootstrap failed; no token file was written' "$T/out" \
     && nofiles && [ ! -e "$T/h/$2.token.part" ] && cmp -s "$T/seed" "$T/state/projects" && [ "$(flags)" = 1 ]; then ok "$label is refused: server exit 3, install.sh exit 1, the fixed line once, no file, no project created"; else bad "$label is refused: server exit 3, install.sh exit 1, the fixed line once, no file, no project created" "rc=$rc code=$(code) flags=$(flags): $(tail -3 "$T/out")"; fi
done

# IT-43 two runs, one per project: two different files, the first untouched by the second, the table unchanged
seed
if runpl a lead-a --reissue-token; then fa=$(ls "$T/h"/lead-a.token.*); ca=$(cat "$fa") || fa=""; else fa=""; fi
if [ -n "$fa" ] && runpl b lead-b --reissue-token && fb=$(ls "$T/h"/lead-b.token.*) && [ "$fa" != "$fb" ] && [ "$(cat "$fa")" = "$ca" ] && [ "$(cat "$fb")" != "$ca" ] && cmp -s "$T/seed" "$T/state/projects"; then ok "IT-43 one run per project: two files, the first left alone, no project created"; else bad "IT-43 one run per project: two files, the first left alone, no project created" "$(tail -2 "$T/out")"; fi

# IT-44 a FIRST run creates (the flag is not passed) and the table gains the project
seed
if runpl c lead-c && [ "$(flags)" = 0 ] && grep -qxF 'c:lead-c' "$T/state/projects" && grep -qxF 'a:lead-a' "$T/state/projects" && grep -qxF 'b:lead-b' "$T/state/projects"; then ok "IT-44 a first run does not pass --require-existing and creates the project"; else bad "IT-44 a first run does not pass --require-existing and creates the project" "flags=$(flags): $(tail -2 "$T/out")"; fi

# IT-45 an older server does not know the flag: exit 2 and ITS OWN line. install.sh fails the same way, creates nothing and does NOT
# retry without the flag; and the test tells the two cases apart by code (2 against 3) and by line.
seed
FAKE_OLD_SERVER=1 runpl a lead-a --reissue-token && rc=0 || rc=$?
if [ "$rc" = 7 ] && [ "$(code)" = 2 ] && grep -q "unexpected argument '--require-existing'" "$T/out" && ! grep -qF "$REFUSED" "$T/out" && grep -q 'install: bootstrap failed; no token file was written' "$T/out" \
   && [ "$(boots)" = 1 ] && nofiles && cmp -s "$T/seed" "$T/state/projects"; then ok "IT-45 an older server (exit 2, another line) fails the reissue the same way and is not retried without the flag"; else bad "IT-45 an older server (exit 2, another line) fails the reissue the same way and is not retried without the flag" "rc=$rc code=$(code) boots=$(boots): $(tail -3 "$T/out")"; fi

# ---- A90 (CONFLICTS-FOUND M-15a, ACCEPTED): the compose provider is podman-compose 1.5.0 or newer; docker-compose is not part of the contract.
# `podman compose` hands over to whichever provider it finds, so the script reads what that provider says it is: the FIRST line of
# `podman compose version` on stdout must be exactly "podman-compose version N.N" or "N.N.N" (digits only, at most four per part).
# Nothing from the provider's output is ever printed. A refusal starts nothing: the log holds only the two preflight calls.
MSG_A='install: no compose provider: podman-compose 1.5.0 or newer is required (it is not bundled)'
MSG_B='install: could not confirm podman-compose 1.5.0 or newer from "podman compose version" (other providers are not tested)'
fresh() { : > "$T/log"; : > "$T/provider.log"; rm -rf "$T/h" "$T/state" "$T/account-runtime"; mkdir -p "$T/state"; mkdir -m 700 "$T/account-runtime"; }
untouched() { ! grep -qvxE -e '--version|compose version' "$T/log" && [ -z "$(ls -A "$T/state")" ] && [ ! -e "$T/h" ]; }
prov() {   # prov <ok|a|b|c:VERSION> <text for printf %b>
  fresh
  case $1 in
    a) FAKE_COMPOSE_VERSION_FAIL=1 run && rc=0 || rc=$? ;;
    *) FAKE_COMPOSE_VERSION_OUT=$2 run && rc=0 || rc=$? ;;
  esac
  case $1 in
    ok) [ "$rc" = 0 ] ;;
    a) [ "$rc" = 3 ] && [ "$(tail -1 "$T/out")" = "$MSG_A" ] && untouched ;;
    b) [ "$rc" = 3 ] && [ "$(tail -1 "$T/out")" = "$MSG_B" ] && untouched ;;
    c:*) [ "$rc" = 3 ] && [ "$(tail -1 "$T/out")" = "install: podman-compose ${1#c:} is too old; 1.5.0 or newer is required" ] && untouched ;;
  esac
}
group() {   # group <id> <title> <expect> then the texts, one argument each
  gid=$1; gtitle=$2; gexp=$3; shift 3; gbad=""
  for text in "$@"; do prov "$gexp" "$text" || gbad="$gbad [$text rc=$rc: $(tail -1 "$T/out" | cut -c1-90)]"; done
  if [ -z "$gbad" ]; then ok "$gid $gtitle"; else bad "$gid $gtitle" "$gbad"; fi
}

# IT-46 no provider answers `podman compose version` (exit 125, as podman does when it finds none)
if prov a ''; then ok "IT-46 no compose provider: the fixed line (a), exit 1, nothing started"; else bad "IT-46 no compose provider: the fixed line (a), exit 1, nothing started" "rc=$rc: $(tail -2 "$T/out")"; fi

# IT-47 a provider answers, but it is not podman-compose or does not say a version we can read: the fixed line (b)
group IT-47 "a provider that is not podman-compose, or an unreadable version, gets the fixed line (b)" b \
  'Docker Compose version v2.29.1\n' \
  '' \
  '\n' \
  'podman version 5.8.2\n' \
  'podman version 5.8.2\npodman version 5.8.2\npodman-compose version 1.5.0\n' \
  'podman version x\npodman-compose version 1.5.0\n' \
  'podman version \npodman-compose version 1.5.0\n' \
  'podman version 5.8.2 \npodman-compose version 1.5.0\n' \
  'podman version 5.8.2\n\npodman-compose version 1.5.0\n' \
  'podman version 5.8.2\nDocker Compose version v2.29.1\n' \
  'podman version 5..8\npodman-compose version 1.5.0\n' \
  'podman version 1111111111111111111111111111111111\npodman-compose version 1.5.0\n' \
  'podman-compose version v1.5.0\n' \
  'podman-compose version 1.5.0-rc1\n' \
  'podman-compose version 1.5.0.1\n' \
  'podman-compose version 1.5.\n' \
  'podman-compose version 1\n' \
  'podman-compose version \n' \
  'podman-compose version  1.5.0\n' \
  'podman-compose version 1.5.0 \n' \
  'podman-compose version 99999.5.0\n' \
  'podman-compose version 1.99999.0\n' \
  'podman-compose version 1.5.99999\n' \
  'podman-compose version: 0.1.7dev\n' \
  'PODMAN-COMPOSE version 1.5.0\n' \
  'podman-composer version 1.5.0\n' \
  'xpodman-compose version 1.5.0\n' \
  'fake compose 1.0\n' \
  '1.5.0\n' \
  'podman-compose version .5.0\n' \
  'podman-compose version 1..0\n' \
  'podman-compose version 12345\n' \
  '\npodman-compose version 1.5.0\n'

# IT-48 podman-compose, but older than 1.5.0: the fixed line (c) with the version it reported
group IT-48 "podman-compose older than 1.5.0 gets the fixed line (c) with its version" c:1.4.9 'podman-compose version 1.4.9\npodman version 5.8.2\n'
group IT-48g "the real two-line output with an old provider is named too old, with its version" c:1.4.9 'podman version 5.8.2\npodman-compose version 1.4.9\n'
group IT-48b "a two-part version is named as the provider reported it" c:1.4 'podman-compose version 1.4\n'
group IT-48c "version 0.9.9 is too old" c:0.9.9 'podman-compose version 0.9.9\n'
group IT-48d "version 1.0.0 is too old" c:1.0.0 'podman-compose version 1.0.0\n'
group IT-48e "version 0.0.0 is too old" c:0.0.0 'podman-compose version 0.0.0\n'
group IT-48f "version 0.10.0 is too old (numeric, not by string)" c:0.10.0 'podman-compose version 0.10.0\n'

# IT-49 podman-compose 1.5.0 or newer is accepted, compared by number and not by text
FAKE_COMPOSE_DIAGNOSTIC='provider diagnostic on stderr (not the disabled banner)' group IT-49 "1.5.0, 1.5.1, 1.5, 1.10.0, 2.0.0 and 10.0.0 are accepted; stderr diagnostics are ignored" ok \
  'podman version 5.8.2\npodman-compose version 1.5.0\n' \
  'podman version 5.8.2\npodman-compose version 1.5.1\n' \
  'podman version 5.8.2\npodman-compose version 1.10.0\n' \
  'podman version 10.0.0\npodman-compose version 1.5.0\n' \
  'podman-compose version 1.5.0\npodman version 5.8.2\n' \
  'podman-compose version 1.5.0\n' \
  'podman-compose version 1.5.1\n' \
  'podman-compose version 1.5\n' \
  'podman-compose version 1.10.0\n' \
  'podman-compose version 2.0.0\n' \
  'podman-compose version 10.0.0\n' \
  'podman-compose version 1.5.0'

# IT-50 nothing the provider prints is ever repeated in a message
fresh
FAKE_COMPOSE_VERSION_OUT='podman-compose version 1.5.0$(id) SYNTHETIC-PROVIDER-TEXT\n' run && rc=0 || rc=$?
if [ "$rc" = 3 ] && ! grep -q 'SYNTHETIC-PROVIDER-TEXT' "$T/out" && [ "$(tail -1 "$T/out")" = "$MSG_B" ] && untouched; then ok "IT-50 the provider's own text is never printed"; else bad "IT-50 the provider's own text is never printed" "rc=$rc: $(tail -2 "$T/out")"; fi
fresh
FAKE_COMPOSE_VERSION_OUT='podman-compose version 1.4.9\nSYNTHETIC-SECOND-LINE\n' run && rc=0 || rc=$?
if [ "$rc" = 3 ] && ! grep -q 'SYNTHETIC-SECOND-LINE' "$T/out" && untouched; then ok "IT-50b only the first line of the output is looked at and none of the rest is printed"; else bad "IT-50b only the first line of the output is looked at and none of the rest is printed" "rc=$rc: $(tail -2 "$T/out")"; fi

# ---- hale F-1 (S5-INC4-REVIEW-hale-r0): a bracket RANGE such as [0-9] follows the locale's collation, so in a UTF-8 locale such as en_US.utf8 bash lets
# full-width, superscript and Arabic-Indic digits through every "digits only" check. install.sh spells its classes out ([!0123456789.]).
# IT-51 is a text check (always runs); IT-52 runs the script under a UTF-8 locale when the host has one (otherwise it prints "skip" and is NOT a pass).
# (hale N-2) any bracket expression in the CODE, not only on a case line: comments and the text of die messages are not code; sed, tr and a pattern on a line of its own count.
# A test bracket `[ -d "$x-y" ]` has a space after the [ and is not a character class (the first A100 line tripped on exactly that).
range_hits() { grep -v '^[[:space:]]*#' | sed -E 's/die "[^"]*"//g' | grep -nE '\[[!^]?[0-9A-Za-z]-[0-9A-Za-z][^]]*\]|\[[!^]?[^] ][^]]*[0-9A-Za-z]-[0-9A-Za-z][^]]*\]'; }
if ! range_hits < "$install" > "$T/ranges"; then ok "IT-51 no bracket expression in install.sh uses a character range"; else bad "IT-51 no bracket expression in install.sh uses a character range" "$(head -3 "$T/ranges" | cut -c1-150)"; fi

loc=$(locale -a 2>/dev/null | grep -iE '^en_[A-Za-z]+\.utf-?8$' | head -1 || true)
if [ -z "$loc" ]; then
  echo "skip IT-52 no en_*.utf8 locale on this host: the locale cell did not run (IT-51 still covers the text)"
else
  l52=""
  for text in 'podman-compose version １.５.０\n' 'podman-compose version 1.⁵.0\n' 'podman-compose version 1.٥.0\n'; do
    LC_ALL=$loc prov b "$text" || l52="$l52 [version $text rc=$rc]"
  done
  fresh; LC_ALL=$loc run --port １２３４ && l52="$l52 [port accepted]" || { grep -q -- '--port must be a number' "$T/out" || l52="$l52 [port: $(tail -1 "$T/out" | cut -c1-80)]"; }
  fresh; LC_ALL=$loc sh "$install" --project 'a１' --lead lead --home "$T/h" > "$T/out" 2>&1 && l52="$l52 [project accepted]" || { grep -q -- '--project must start with a small letter' "$T/out" || l52="$l52 [project: $(tail -1 "$T/out" | cut -c1-80)]"; }
  fresh; LC_ALL=$loc FAKE_HBA_RAW='１' run && l52="$l52 [count accepted]" || { grep -q 'could not read a count of trust rules' "$T/out" || l52="$l52 [count: $(tail -1 "$T/out" | cut -c1-80)]"; }
  fresh
  if mkdir -p "$T/h" && printf '[modules]\nenabled = ["aud１t"]\n' > "$T/h/cortex.toml"; then
  LC_ALL=$loc run && l52="$l52 [module id accepted]" || { grep -q 'invalid module id' "$T/out" || l52="$l52 [module: $(tail -1 "$T/out" | cut -c1-80)]"; }
  else l52="$l52 [module fixture setup]"; fi
  if [ -z "$l52" ]; then ok "IT-52 under $loc: non-ASCII digits are refused by the version, port, name, trust-count and module-id checks"; else bad "IT-52 under $loc: non-ASCII digits are refused by the version, port, name, trust-count and module-id checks" "$l52"; fi
fi

# IT-51b (hale F-3) the range check itself is held: lines it must flag and lines it must not (a range that STARTS a bracket expression is the commonest fault)
f51=""
while IFS= read -r l; do printf '%s\n' "$l" | range_hits > /dev/null || f51="$f51 [not flagged: $l]"; done <<'EOF51'
case $port in *[0-9]*) ;; esac
case $port in [a-z]*) ;; esac
case $port in *[A-Z]*) ;; esac
x=$(printf %s "$y" | tr -d "[0-9]")
x=$(sed 's/[^0-9.]//g')
case $x in *[!0-9]*) ;; esac
case $x in *[a-z0-9_]*) ;; esac
EOF51
while IFS= read -r l; do printf '%s\n' "$l" | range_hits > /dev/null && f51="$f51 [flagged: $l]"; done <<'EOF51'
if [ -d "$home_dir/dump-status" ]; then
case $x in *[!0123456789]*) ;; esac
die "bad [a-z] class"
echo "[ok] 0-9 are digits"
EOF51
if [ -z "$f51" ]; then ok "IT-51b the range check flags a range that starts a bracket expression and leaves test brackets and messages alone"; else bad "IT-51b the range check flags a range that starts a bracket expression and leaves test brackets and messages alone" "$f51"; fi
# IT-64 (hale N-1) each spelled-out class accepts EXACTLY its alphabet: scripts/probe-classes.sh takes the six lines out of the script under test and feeds each every printable ASCII character and five non-ASCII ones under three locales
if command -v bash >/dev/null 2>&1; then
  bash "$here/probe-classes.sh" "$install" > "$T/probe" 2>&1 || true
  if grep -q '^result: every class accepts exactly its alphabet' "$T/probe" && ! grep -qE 'MISMATCH|NOT FOUND' "$T/probe"; then ok "IT-64 every spelled-out character class accepts exactly its alphabet (C, C.utf8, en_AU.utf8)"; else bad "IT-64 every spelled-out character class accepts exactly its alphabet" "$(grep -E 'MISMATCH|NOT FOUND|result:' "$T/probe" | head -3 | cut -c1-160)"; fi
else
  echo "skip IT-64 no bash on this host: the class probe did not run (IT-51 still covers the text)"
fi

# ---- hale N-3: the provider's own banner (podman prints one on stderr) must not reach the operator's terminal
fresh; FAKE_COMPOSE_DIAGNOSTIC='UNTRUSTED-PROVIDER-STDERR' run && rc=0 || rc=$?
if [ "$rc" = 0 ] && ! grep -q 'Executing external compose provider\|UNTRUSTED-PROVIDER-STDERR' "$T/out"; then ok "IT-53 the provider's stderr banner and diagnostics are not shown"; else bad "IT-53 the provider's stderr banner and diagnostics are not shown" "rc=$rc: provider stderr reached output"; fi

# ---- A94 / A95 (cox 14:55Z and 15:2xZ): the dump directory mounted read-only into the server, and the git sha of the build
# IT-54 (A100, vera option C) install.sh makes <home>/dump-status (0755: any uid in the container reads it, only the owner writes it) and gives it to compose as
# CORTEX_DUMP_STATUS; <home>/dumps (the private backup tree) is made 0700 and goes to NO compose call
fresh; run && rc=0 || rc=$?
if [ "$rc" = 0 ] && [ -d "$T/h/dump-status" ] && [ "$(mode "$T/h/dump-status")" = 755 ] && [ -d "$T/h/dumps" ] && [ "$(mode "$T/h/dumps")" = 700 ] && [ -s "$T/state/compose.env" ] && ! grep -qv "^CORTEX_DUMP_STATUS=$T/h/dump-status CORTEX_DUMPS=unset CORTEX_GIT_SHA=" "$T/state/compose.env"; then ok "IT-54 <home>/dump-status is 0755 and in every compose call; <home>/dumps is 0700 and in none"; else bad "IT-54 <home>/dump-status is 0755 and in every compose call; <home>/dumps is 0700 and in none" "rc=$rc status=$(mode "$T/h/dump-status" 2>&1) dumps=$(mode "$T/h/dumps" 2>&1) env=$(head -2 "$T/state/compose.env" 2>&1)"; fi
# existing directories keep their content and are not made wider
if fresh && mkdir -p "$T/h/dump-status" "$T/h/dumps" && chmod 0750 "$T/h/dump-status" "$T/h/dumps" && echo keep > "$T/h/dump-status/LAST_GOOD" && echo keep > "$T/h/dumps/set-1"; then
run && rc=0 || rc=$?
if [ "$rc" = 0 ] && [ "$(cat "$T/h/dump-status/LAST_GOOD")" = keep ] && [ "$(mode "$T/h/dump-status")" = 750 ] && [ "$(cat "$T/h/dumps/set-1")" = keep ] && [ "$(mode "$T/h/dumps")" = 750 ]; then ok "IT-54b existing dump-status and dumps directories keep their files and their modes"; else bad "IT-54b existing dump-status and dumps directories keep their files and their modes" "rc=$rc status=$(mode "$T/h/dump-status") dumps=$(mode "$T/h/dumps")"; fi
else bad "IT-54b fixture setup" "could not prepare dependent fixture"; fi
# a CORTEX_DUMPS in the operator's environment is never passed on: compose has no use for the private tree
fresh; CORTEX_DUMPS=/etc run && rc=0 || rc=$?
if [ "$rc" = 0 ] && [ -s "$T/state/compose.env" ] && ! grep -qv ' CORTEX_DUMPS=unset ' "$T/state/compose.env"; then ok "IT-54c CORTEX_DUMPS from the environment is not passed to compose"; else bad "IT-54c CORTEX_DUMPS from the environment is not passed to compose" "rc=$rc env=$(sort -u "$T/state/compose.env" | head -2)"; fi

# IT-54d (hale F-4) a dump directory that cannot be made stops the installer before anything is started: a regular file at <home>/dump-status, then at <home>/dumps
b54=""
for d in dump-status dumps; do
  fresh
  if ! { mkdir -p "$T/h" && : > "$T/h/$d"; }; then b54="$b54 [$d: fixture setup]"; continue; fi
  run && rc=0 || rc=$?
  if [ "$rc" != 6 ] || ! grep -q "cannot create $T/h/$d" "$T/out" || grep -q ' up ' "$T/log"; then b54="$b54 [$d: rc=$rc: $(tail -1 "$T/out" | cut -c1-80)]"; fi
done
if [ -z "$b54" ]; then ok "IT-54d a file where <home>/dump-status or <home>/dumps should be stops the installer with the cannot-create sentence, before anything is started"; else bad "IT-54d a dump directory that cannot be made stops the installer" "$b54"; fi

# IT-79 (hale N-2) a SYMLINK at <home>/dump-status or <home>/dumps stops the installer before anything is started: dump-status is the one directory the server sees, and a link to the
# private dumps would show it the whole tree (a -d test follows the link)
b79=""
for d in dump-status dumps; do
  fresh
  if ! { mkdir -p "$T/h/elsewhere" && ln -s "$T/h/elsewhere" "$T/h/$d"; }; then b79="$b79 [$d: fixture setup]"; continue; fi
  run && rc=0 || rc=$?
  if [ "$rc" != 6 ] || ! grep -q "$T/h/$d is a symbolic link" "$T/out" || grep -q ' up ' "$T/log"; then b79="$b79 [$d: rc=$rc: $(tail -1 "$T/out" | cut -c1-90)]"; fi
done
if [ -z "$b79" ]; then ok "IT-79 a symbolic link at <home>/dump-status or <home>/dumps stops the installer with the symbolic-link sentence, before anything is started"; else bad "IT-79 a symbolic link at <home>/dump-status or <home>/dumps stops the installer" "$b79"; fi

# IT-55 the build gets the git sha of the checkout the installer runs from; outside a checkout it is `unknown`; nothing from the environment is used.
# THE EXPECTED VALUE COMES FROM A THROWAWAY GIT REPOSITORY (hale 308f0142, cox ff2476b9): the installer derives the sha from ITS OWN location (git -C "$here/..", install.sh:761) and the suite runs it from a copy under $T that is not
# a work tree, so a cell that took its expected value from the same place compared unknown with unknown and no longer tested a real checkout's HEAD (IM-486: an installer that always says unknown). The cell makes $T/repo (git init,
# one commit), copies deploy/ into $T/repo/deploy (copy_deploy) and runs the installer FROM THERE; the expected value is git -C $T/repo rev-parse HEAD (40 hex). So the real-git path runs in EVERY run, plain or through the harness.
rm -rf "$T/repo"; mkdir -p "$T/repo" && ( cd "$T/repo" && command git init -q && command git -c user.email=t@example.invalid -c user.name=t commit -q --allow-empty -m one ) && copy_deploy "$(dirname "$install")" "$T/repo/deploy"
want=$(command git -C "$T/repo" rev-parse HEAD 2>/dev/null || echo unknown)
fresh; CORTEX_GIT_SHA="x; touch $T/pwned" sh "$T/repo/deploy/install.sh" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?
if [ "${#want}" = 40 ] && [ "$rc" = 0 ] && ! grep -qv "CORTEX_GIT_SHA=$want\$" "$T/state/compose.env" && [ ! -e "$T/pwned" ]; then ok "IT-55 CORTEX_GIT_SHA is the HEAD ($want) of the git repository the installer runs from (a throwaway one made here) and not what the environment says"; else bad "IT-55 CORTEX_GIT_SHA is the checkout's HEAD" "rc=$rc want=$want got=$(sort -u "$T/state/compose.env" | head -2 | tr '\n' ' ' | cut -c1-120)"; fi
# IT-55b / IT-55d (cox W1-166, 11 Nov): outside a git checkout the sha is `unknown`, WHEREVER the suite's temporary folder lies. The installer asks git for the folder above its own copy (git -C "$here/..", install.sh:787) and git walks UP from
# there: a copy made under a temporary folder that is itself inside a git worktree (the real release flow: RUN-TREE/target/release-work.*) found the run tree and said its HEAD, so IT-55b was green only where the suite ran outside every repository.
# nogit_sha_check ROOT ID WHERE NEEDGIT: copies deploy/ into ROOT/nogit/ and runs the installer from there; ID names the cell and WHERE ends its title; NEEDGIT=1 requires that plain git DOES find a repository above the copy (else the cell would prove nothing).
nogit_sha_check() {
  ng_root=$1; ng_id=$2; ng_where=$3; ng_need=${4:-0}
  fresh; rm -rf "$ng_root/nogit"; mkdir -p "$ng_root/nogit"; cp -R "$(dirname "$install")" "$ng_root/nogit/deploy"
  if [ "$ng_need" = 1 ]; then
    ng_pre=$(command git -C "$ng_root/nogit" rev-parse HEAD 2>/dev/null || echo none)
    [ "${#ng_pre}" = 40 ] || { bad "$ng_id outside a git checkout the sha is unknown$ng_where" "precondition: git finds no repository above $ng_root/nogit, so this cell would prove nothing (got '$ng_pre')"; return 0; }
  fi
  GIT_CEILING_DIRECTORIES="$ng_root" sh "$ng_root/nogit/deploy/install.sh" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?   # the ceiling is ROOT, the folder ABOVE the one git starts in ($ROOT/nogit): git never climbs into it (a ceiling equal to the starting folder itself would have no effect)
  if [ "$rc" = 0 ] && ! grep -qv 'CORTEX_GIT_SHA=unknown$' "$T/state/compose.env"; then ok "$ng_id outside a git checkout the sha is unknown$ng_where"; else bad "$ng_id outside a git checkout the sha is unknown$ng_where" "rc=$rc got=$(sort -u "$T/state/compose.env" | head -2)"; fi
}
nogit_sha_check "$T" IT-55b "" 0
# IT-55d the same check with the temporary folder INSIDE a git worktree that has a commit (made here, as IT-55 makes its own): the failure of the real release flow, reproduced in every run, in whatever place the suite runs
rm -rf "$T/outer"; mkdir -p "$T/outer/work" && ( cd "$T/outer" && command git init -q && command git -c user.email=t@example.invalid -c user.name=t commit -q --allow-empty -m one )
nogit_sha_check "$T/outer/work" IT-55d " even when the temporary folder is inside a git worktree (the real release flow)" 1
# IT-55c only a full lowercase sha is used, whatever git prints; anything else is `unknown` (a fake git on the PATH)
mkdir -p "$T/bin-git"; printf '#!/bin/sh\nprintf "%%s\\n" "${FAKE_GIT_OUT-}"\n' > "$T/bin-git/git"; chmod +x "$T/bin-git/git"
h40=0123456789abcdef0123456789abcdef01234567
g55=""
for out in "" "unknown" "${h40%?}" "${h40}0" "$(printf '%s' "$h40" | tr a-f A-F)" "ghijklmnopghijklmnopghijklmnopghijklmnop" "$h40 extra" "$h40;id"; do
  fresh; PATH="$T/bin-git:$PATH" FAKE_GIT_OUT="$out" run && rc=0 || rc=$?
  if [ "$rc" != 0 ] || grep -qv 'CORTEX_GIT_SHA=unknown$' "$T/state/compose.env"; then g55="$g55 [git said '$out' -> $(sort -u "$T/state/compose.env" | head -1 | cut -c1-90)]"; fi
done
fresh; PATH="$T/bin-git:$PATH" FAKE_GIT_OUT="$h40" run && rc=0 || rc=$?
grep -qv "CORTEX_GIT_SHA=$h40\$" "$T/state/compose.env" && g55="$g55 [a good sha was not passed on]"
if [ -z "$g55" ]; then ok "IT-55c a sha that is not 40 lowercase hex digits is replaced by unknown"; else bad "IT-55c a sha that is not 40 lowercase hex digits is replaced by unknown" "$g55"; fi

# ---- piece 12 (bob; cox W1-42): the five backup scripts go to <home>/bin and the five user units are filled in and the two timers enabled
UNITS="cortex-dump.service cortex-dump.timer cortex-dump-status.service cortex-dump-status.timer cortex-dump-alert.service"
SCRIPTS="cortex-dump cortex-dump-status cortex-dump-alert cortex-restore restore-compare"
UD="$T/home/.config/systemd/user"
# IT-56 the scripts: copied, mode 0755 (the source is 0644), same bytes
fresh; rm -rf "$T/home/.config"; run && rc=0 || rc=$?
b56=""; for f in $SCRIPTS; do [ "$(mode "$T/h/bin/$f" 2>/dev/null)" = 755 ] && cmp -s "$T/scripts/$f" "$T/h/bin/$f" || b56="$b56 $f"; done
if [ "$rc" = 0 ] && [ -z "$b56" ]; then ok "IT-56 the five backup scripts are in <home>/bin as 0755 with the source's bytes"; else bad "IT-56 the five backup scripts are in <home>/bin as 0755 with the source's bytes" "rc=$rc bad:$b56"; fi
# IT-57 the units: five files, both placeholders filled, nothing else changed, 0644
b57=""; for u in $UNITS; do
  [ -f "$UD/$u" ] && [ "$(mode "$UD/$u")" = 644 ] && ! grep -q '@[A-Z_]*@' "$UD/$u" || b57="$b57 $u"
done
grep -q "$T/h/bin/cortex-dump " "$UD/cortex-dump.service" 2>/dev/null && grep -q -- '--target cv2-s5-db' "$UD/cortex-dump.service" 2>/dev/null || b57="$b57 [cortex-dump.service lacks the home or the container]"
if [ -z "$b57" ]; then ok "IT-57 the five units are filled in with the home and the database container"; else bad "IT-57 the five units are filled in with the home and the database container" "$b57"; fi
# IT-58 systemctl: reload, then enable ONLY the two timers (never a service or the alert), nothing else
if [ "$(cat "$T/state/systemctl.log" 2>/dev/null)" = "--user daemon-reload
--user enable --now cortex-dump.timer cortex-dump-status.timer
--user enable podman-restart.service" ]; then ok "IT-58 daemon-reload, then enable --now of the two timers only, then the distro's own podman-restart.service (defect R) and nothing else"; else bad "IT-58 daemon-reload, then enable --now of the two timers only, then the distro's own podman-restart.service" "$(cat "$T/state/systemctl.log" 2>&1)"; fi
# IT-59 a second run changes nothing in the units and does it all again
for u in $UNITS; do cp "$UD/$u" "$T/before-$u" 2>/dev/null || : > "$T/before-$u"; done
fresh; run && rc=0 || rc=$?
b59=""; for u in $UNITS; do cmp -s "$UD/$u" "$T/before-$u" || b59="$b59 $u"; done
if [ "$rc" = 0 ] && [ -z "$b59" ] && [ "$(wc -l < "$T/state/systemctl.log")" = 3 ]; then ok "IT-59 a second run leaves the units as they were and enables the timers again"; else bad "IT-59 a second run leaves the units as they were and enables the timers again" "rc=$rc changed:$b59"; fi
# IT-60 a user manager that cannot be reached is a LOUD failure after the stack is up, and the next run (manager back) succeeds
fresh; FAKE_SYSTEMCTL_FAIL=1 run && rc=0 || rc=$?
if [ "$rc" = 8 ] && grep -q 'backup timers could not be enabled' "$T/out" && grep -q ' up ' "$T/log"; then ok "IT-60 systemctl failing is exit 8 (EX_BACKUP) with its own sentence, after the stack is up"; else bad "IT-60 systemctl failing is exit 8 (EX_BACKUP) with its own sentence, after the stack is up" "rc=$rc: $(tail -2 "$T/out")"; fi
fresh; run && rc=0 || rc=$?
if [ "$rc" = 0 ]; then ok "IT-60b the next run, with the manager back, succeeds"; else bad "IT-60b the next run, with the manager back, succeeds" "rc=$rc: $(tail -2 "$T/out")"; fi
# IT-60c diagnostic candidate for IM-58: command failure cannot be masked by enabled readback.
fresh; FAKE_TIMER_ENABLE_FAIL_AFTER_ENABLE=1 run && rc=0 || rc=$?
if [ "$rc" = 8 ] && [ "$(grep -c '^problem:' "$T/out")" = 1 ] &&
  [ "$(grep -c '^fix:' "$T/out")" = 1 ] &&
  grep -q 'Backups ... FAILED' "$T/out" &&
  ! grep -q 'is installed and running' "$T/out"; then
  ok "IT-60c timer enable command failure is terminal even when enablement persisted"
else bad "IT-60c timer enable command failure masked by enabled readback" "rc=$rc problems=$(grep -c '^problem:' "$T/out" || :) fixes=$(grep -c '^fix:' "$T/out" || :)"; fi

# IT-61 --no-start installs the scripts and neither the units nor the timers; --reissue-token touches none of it
fresh; rm -rf "$T/home/.config"; run --no-start && rc=0 || rc=$?
if [ "$rc" = 0 ] && [ -x "$T/h/bin/cortex-dump" ] && [ ! -e "$UD" ] && [ ! -s "$T/state/systemctl.log" ]; then ok "IT-61 --no-start installs the scripts and neither units nor timers"; else bad "IT-61 --no-start installs the scripts and neither units nor timers" "rc=$rc: $(tail -2 "$T/out")"; fi
fresh; run >/dev/null 2>&1; rm -rf "$T/home/.config" "$T/h/bin"; : > "$T/state/systemctl.log"
mv "$T/scripts/cortex-restore" "$T/cortex-restore.away"   # a reissue needs none of the backup scripts
run --reissue-token && rc=0 || rc=$?
mv "$T/cortex-restore.away" "$T/scripts/cortex-restore"
if [ "$rc" = 0 ] && [ ! -e "$T/h/bin" ] && [ ! -e "$UD" ] && [ ! -s "$T/state/systemctl.log" ]; then ok "IT-61b --reissue-token touches neither scripts nor units, and needs none of them"; else bad "IT-61b --reissue-token touches neither scripts nor units" "rc=$rc"; fi
# IT-62 a missing script is refused BEFORE anything is started (the preflight), naming it
fresh; mv "$T/scripts/cortex-restore" "$T/cortex-restore.away"; run && rc=0 || rc=$?
mv "$T/cortex-restore.away" "$T/scripts/cortex-restore"
if [ "$rc" = 5 ] && grep -q 'backup script cortex-restore is missing' "$T/out" && untouched; then ok "IT-62 a missing backup script is refused before anything is started"; else bad "IT-62 a missing backup script is refused before anything is started" "rc=$rc: $(tail -2 "$T/out")"; fi
# IT-63 --home goes into systemd units, so only letters, digits and . _ / - are allowed (a space, |, &, backslash, %, $, a quote or a newline would change a unit or the sed)
b63=""
for bad_home in "$T/h x" "$T/h|x" "$T/h&x" "$T/h\\x" "$T/h%x" "$T/h\$x" "$T/h'x" "$T/h\"x" "$T/h
x" "$T/hé"; do
  fresh; sh "$install" --project p --lead lead --home "$bad_home" > "$T/out" 2>&1 && rc=0 || rc=$?
  if [ "$rc" != 1 ] || ! tail -1 "$T/out" | grep -q -- '--home may contain only' || ! untouched; then b63="$b63 [$(printf '%s' "$bad_home" | tr '\n' '~' | cut -c1-30) rc=$rc]"; fi
done
fresh; run --home "$T/h-a_b.c" && rc=0 || rc=$?
[ "$rc" = 0 ] || b63="$b63 [a plain path with - _ . was refused]"
rm -rf "$T/h-a_b.c"
# the units go to $HOME/.config/systemd/user: a relative HOME is refused BEFORE anything is looked at or started (6e: G7, no half-install), exit 1 (EX_USAGE)
fresh; (cd "$T" && HOME=relative/home run) && rc=0 || rc=$?
[ "$rc" = 1 ] && grep -q 'HOME must be an absolute path' "$T/out" && [ ! -s "$T/log" ] || b63="$b63 [a relative HOME was not refused before anything ran: rc=$rc, podman calls $(wc -l < "$T/log" | tr -d ' ')]"
rm -rf "$T/relative"
if [ -z "$b63" ]; then ok "IT-63 --home with a space, |, &, backslash, %, \$, a quote, a newline or a non-ASCII letter is refused; - _ . are fine"; else bad "IT-63 --home with a space, |, &, backslash, %, \$, a quote, a newline or a non-ASCII letter is refused; - _ . are fine" "$b63"; fi

# IT-65 (hale 5b F-1) --home must be an ABSOLUTE path: it ends up in systemd units, and a relative one makes services systemd refuses to start while the
# timers would still be enabled. Run in a scratch directory so that a script without the rule cannot write into the repository.
b65=""
mkdir -p "$T/w65"
for rel_home in rel/dir ./x ../x x; do
  fresh; (cd "$T/w65" && sh "$install" --project p --lead lead --home "$rel_home" > "$T/out" 2>&1) && rc=0 || rc=$?
  if [ "$rc" != 1 ] || ! tail -1 "$T/out" | grep -q -- '--home must be an absolute path' || ! untouched; then b65="$b65 [$rel_home rc=$rc]"; fi
done
rm -rf "$T/w65"
if [ -z "$b65" ]; then ok "IT-65 a relative --home (rel/dir, ./x, ../x, x) is refused with the absolute-path sentence before anything is made"; else bad "IT-65 a relative --home is refused before anything is made" "$b65"; fi

# IT-81 (cox 5q, R-5) --home holds no empty component and no .. component: a double slash, a trailing slash, the root alone, a .. in any place; the engine's render uses the same rule. A name that only CONTAINS dots is a plain component.
b81=""
for bad_home in "$T/a/../b" "$T/a//b" "$T/a/" "/" "/.." "$T/a/.." "$T/.."; do
  fresh; sh "$install" --project p --lead lead --home "$bad_home" > "$T/out" 2>&1 && rc=0 || rc=$?
  if [ "$rc" != 1 ] || ! tail -1 "$T/out" | grep -q -- '--home may not hold an empty or a .. component' || ! untouched; then b81="$b81 [$bad_home rc=$rc]"; fi
done
fresh; run --home "$T/h..x/a...b" && rc=0 || rc=$?
[ "$rc" = 0 ] || b81="$b81 [a name that only contains dots was refused: rc=$rc]"
rm -rf "$T/h..x"
if [ -z "$b81" ]; then ok "IT-81 --home holds no empty and no .. component, and a name that only contains dots is accepted"; else bad "IT-81 --home holds no empty and no .. component" "$b81"; fi
# IT-66 (hale 5b) the rendered units pass systemd's own OFFLINE check; printed as skip (not a pass) when the tool is absent. The call goes through
# scripts/verify-units.sh, the ONLY place that calls `systemd-analyze verify`: with the real runtime directory it unlinks and re-binds the live user manager's
# control socket ($XDG_RUNTIME_DIR/systemd/private) and breaks `systemctl --user` and `systemd-run --user` for the whole VM (kai 13:30Z, 7 Nov). This cell also
# records the inode of that socket before and after, and fails if it changed (skipped where the path does not exist).
if command -v systemd-analyze >/dev/null 2>&1; then
  fresh; run >/dev/null 2>&1 || true
  live66="/run/user/$(id -u)/systemd/private"; i66a=""; [ ! -e "$live66" ] || i66a=$(stat -c %i "$live66")
  o66=$(cd "$UD" 2>/dev/null && sh "$here/verify-units.sh" cortex-dump.service cortex-dump.timer cortex-dump-status.service cortex-dump-status.timer cortex-dump-alert.service 2>&1) && rc=0 || rc=$?
  i66b=""; [ ! -e "$live66" ] || i66b=$(stat -c %i "$live66")
  if [ "$rc" = 0 ] && [ -z "$o66" ] && [ "$i66a" = "$i66b" ]; then ok "IT-66 the five rendered units pass systemd-analyze --user verify with no warning, and the live user manager's socket is untouched (the tool: $(command -v systemd-analyze); a stand-in that always says yes would hide IM-85, so a receipt names it)"; else bad "IT-66 the five rendered units pass systemd-analyze --user verify, and the live socket is untouched" "rc=$rc inode before='$i66a' after='$i66b': $(printf '%s' "$o66" | head -3 | cut -c1-160)"; fi
else
  echo "skip IT-66 no systemd-analyze on this host: the rendered units were not verified (IT-57 covers the text)"
fi

# IT-76/77/78 (kai 13:30Z, 7 Nov) `systemd-analyze verify` must never bind the LIVE user manager's socket. The failure is SIMULATED, never reproduced on the host: a fake runtime directory
# with a fake systemd/private, and a stand-in systemd-analyze that unlinks and re-creates that file the way the real one does (a new inode).
vu="$here/verify-units.sh"; stub76="$T/stub76"; mkdir -p "$stub76" "$T/rt76/systemd" "$T/units76"
cat > "$stub76/systemd-analyze" <<'STUB'
#!/bin/sh
printf '%s\n' "XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-unset}" "DBUS=${DBUS_SESSION_BUS_ADDRESS-unset}" "ARGS=$*" "MODE=$(stat -c %a "${XDG_RUNTIME_DIR:-/nonexistent}" 2>/dev/null)" >> "$STUB_LOG"
if [ -n "${XDG_RUNTIME_DIR:-}" ]; then mkdir -p "$XDG_RUNTIME_DIR/systemd"; printf 'LINKS=%s\n' "$(stat -c %h "$XDG_RUNTIME_DIR/systemd/private" 2>/dev/null)" >> "$STUB_LOG"; rm -f "$XDG_RUNTIME_DIR/systemd/private"; : > "$XDG_RUNTIME_DIR/systemd/private"; fi
exit "${STUB_RC:-0}"
STUB
chmod +x "$stub76/systemd-analyze"; : > "$T/units76/a.service"; : > "$T/units76/b.timer"
: > "$T/rt76/systemd/private"; ino77a=$(stat -c %i "$T/rt76/systemd/private")
ln "$T/rt76/systemd/private" "$T/keep77" 2>/dev/null || :   # THE HOLD: a hard link keeps the OLD inode allocated while the stand-in re-creates the file, so no create can be handed its number (the link count then shows 2: IT-77 checks it)
# IT-77 the control: the stand-in DOES rebind the socket when it is called straight with the live runtime directory (so IT-76 can fail)
: > "$T/stub77.log"; (cd "$T/units76" && STUB_LOG="$T/stub77.log" XDG_RUNTIME_DIR="$T/rt76" PATH="$stub76:$PATH" systemd-analyze --user --man=no verify a.service) >/dev/null 2>&1 || true
ino77b=$(stat -c %i "$T/rt76/systemd/private")
rm -f "$T/keep77"
# the control compares inode NUMBERS, and a number that is FREE can be handed to the very next create (finn's measurement under concurrent file churn on one filesystem: a baseline failed with 'inode stayed'): the control is only sound if the OLD inode is still
# ALLOCATED while the stand-in re-creates the file, so the stand-in logs the link count of the file just before it removes it (LINKS=) and the cell wants at least 2 (the name and the hold)
l77=$(sed -n 's/^LINKS=//p' "$T/stub77.log" | sed -n 1p)
if [ "$ino77a" != "$ino77b" ] && [ "${l77:-0}" -ge 2 ]; then ok "IT-77 control: the stand-in rebinds the live socket (new inode) when called with the live runtime directory, with the old inode held allocated"; else bad "IT-77 control: the stand-in rebinds the live socket when called with the live runtime directory" "inode before $ino77a after $ino77b, link count of the old file when the stand-in ran: '${l77:-none}' (want at least 2: a free inode number can be reused, and IT-76 could not fail)"; fi
# IT-76 the helper leaves the live socket alone: same inode before and after, a PRIVATE runtime directory (not the live one, mode 0700, gone afterwards), the exact arguments, no bus
ino76a=$(stat -c %i "$T/rt76/systemd/private"); : > "$T/stub76.log"
(cd "$T/units76" && TMPDIR="$T" STUB_LOG="$T/stub76.log" XDG_RUNTIME_DIR="$T/rt76" PATH="$stub76:$PATH" sh "$vu" a.service b.timer) >/dev/null 2>&1 && rc76=0 || rc76=$?
ino76b=$(stat -c %i "$T/rt76/systemd/private" 2>/dev/null || echo gone)
priv76=$(sed -n 's/^XDG_RUNTIME_DIR=//p' "$T/stub76.log" | sed -n 1p)
if [ "$rc76" = 0 ] && [ "$ino76a" = "$ino76b" ] && [ -n "$priv76" ] && [ "$priv76" != unset ] && [ "$priv76" != "$T/rt76" ] && [ ! -e "$priv76" ] \
  && grep -qx 'MODE=700' "$T/stub76.log" && grep -qx 'DBUS=' "$T/stub76.log" && grep -qx 'ARGS=--user --man=no verify a.service b.timer' "$T/stub76.log"; then ok "IT-76 verify-units.sh leaves the live user manager's socket alone: a private 0700 runtime directory, removed afterwards, no bus, the exact arguments"; else bad "IT-76 verify-units.sh leaves the live user manager's socket alone" "rc=$rc76 inode $ino76a -> $ino76b private='$priv76' (exists: $([ -e "$priv76" ] && echo yes || echo no)) log: $(tr '\n' '|' < "$T/stub76.log" | cut -c1-200)"; fi
# IT-78 the helper passes the tool's status through, removes its private directory even then, and refuses an empty call
: > "$T/stub78.log"; (cd "$T/units76" && TMPDIR="$T" STUB_LOG="$T/stub78.log" STUB_RC=3 XDG_RUNTIME_DIR="$T/rt76" PATH="$stub76:$PATH" sh "$vu" a.service) >/dev/null 2>&1 && rc78=0 || rc78=$?
priv78=$(sed -n 's/^XDG_RUNTIME_DIR=//p' "$T/stub78.log" | sed -n 1p)
sh "$vu" >/dev/null 2>&1 && rc78b=0 || rc78b=$?
if [ "$rc78" = 3 ] && [ -n "$priv78" ] && [ ! -e "$priv78" ] && [ "$rc78b" = 2 ]; then ok "IT-78 verify-units.sh passes the tool's exit status through, removes its private directory even then, and refuses a call with no unit"; else bad "IT-78 verify-units.sh passes the tool's exit status through, removes its private directory even then, and refuses a call with no unit" "rc=$rc78 (want 3) private='$priv78' gone=$([ ! -e "$priv78" ] && echo yes || echo no) empty-call rc=$rc78b (want 2)"; fi

# ---- the interface for the install lanes (INSTALL-LANES-INTERFACE-hue-r0.md and r1; kai's order, increment 6d-0) ----
# a COPY of deploy/ with a fake sibling `install-prereq.sh` (nix's file, as the dev include line finds it): `ifc <fake file text> <install.sh arguments>`
ifc() {
  rm -rf "$T/d83"; mkdir -p "$T/d83"; cp "$install" "$T/d83/install.sh"
  [ -z "$1" ] || printf '%s\n' "$1" > "$T/d83/install-prereq.sh"
  shift; fresh; sh "$T/d83/install.sh" "$@" > "$T/out" 2>&1 && rc=0 || rc=$?
}
# IT-83 the new arguments: --yes, --check, --release, --base-url, --from are accepted and validated BEFORE anything is looked at or started
b83=""
for bad in "--release 1.2" "--release v1.2.3" "--release 1.2.3-" "--release ''" "--release 01.2.3" "--release 1.2.3.4" "--release 1.2.3-01" "--release 0.0.0.part" "--release 1.2.3-rc.1.part" "--release 1.2.3+build" "--base-url http://h.example/p" "--base-url 'https://h.example/a b'" "--base-url ''" "--from rel/dir" "--from /a/../b" "--from /a//b"; do
  eval "ifc '' --project p --lead lead --home \"\$T/h\" --no-start $bad"; flag=${bad%% *}
  if [ "$rc" != 1 ] || ! untouched || ! tail -1 "$T/out" | grep -q -- "^install: $flag "; then b83="$b83 [$bad rc=$rc: $(tail -1 "$T/out" | cut -c1-60)]"; fi
done
# the good releases include two valid SemVer that a looser rule of install.sh's own used to refuse (an identifier may end in a hyphen): the SAME rule as the engine's, valid_semver
for rel in 0.2.1-rc.2 0.2.1 1.2.3-- 1.2.3-a.b- 1.2.3-0a; do
  fresh; sh "$install" --project p --lead lead --home "$T/h" --no-start --yes --release "$rel" --base-url https://releases.example.invalid/cortex --from "$T/from" > "$T/out" 2>&1 && rc=0 || rc=$?
  [ "$rc" = 0 ] || b83="$b83 [good release $rel refused: rc=$rc $(tail -1 "$T/out" | cut -c1-60)]"
done
if [ -z "$b83" ]; then ok "IT-83 --yes, --release, --base-url and --from are accepted, and a bad value is refused before anything starts"; else bad "IT-83 the new arguments" "$b83"; fi
# IT-84 --check runs the prerequisite file's machine_look, returns ITS status, needs no --project and no --lead, and starts nothing; without machine_look it says so
ifc 'machine_look() { say "looked"; return 0; }' --check
[ "$rc" = 0 ] && grep -qx 'looked' "$T/out" && untouched && c84=ok || c84="clean rc=$rc: $(tail -1 "$T/out" | cut -c1-60)"
ifc 'machine_look() { fail "$EX_MACHINE" "boom"; }' --check
[ "$rc" = 2 ] && [ "$(tail -1 "$T/out")" = "install: boom" ] && untouched && d84=ok || d84="blocking rc=$rc: $(tail -1 "$T/out" | cut -c1-60)"
ifc "" --check
[ "$rc" = 1 ] && tail -1 "$T/out" | grep -q -- '--check is not available in this build' && untouched && e84=ok || e84="no file rc=$rc: $(tail -1 "$T/out" | cut -c1-60)"
ifc 'machine_look() { return 2; }' --check
[ "$rc" = 2 ] && untouched && f84=ok || f84="returned status rc=$rc: $(tail -1 "$T/out" | cut -c1-60)"
ifc 'machine_look() { return 0; }' --check
[ "$rc" = 0 ] && untouched && g84=ok || g84="status 0 rc=$rc"
if [ "$c84" = ok ] && [ "$d84" = ok ] && [ "$e84" = ok ] && [ "$f84" = ok ] && [ "$g84" = ok ]; then ok "IT-84 --check runs machine_look and returns its status (say, fail and EX_MACHINE work), and says so when the build has none"; else bad "IT-84 --check" "$c84 | $d84 | $e84 | $f84 | $g84"; fi
# IT-85 the exit-code symbols are the table of the interface file (0 to 7); a joined file reaches them
ifc 'machine_look() { echo "$EX_OK $EX_USAGE $EX_MACHINE $EX_PREREQ $EX_TRUST $EX_FETCH $EX_START $EX_WORKER"; }' --check
if [ "$rc" = 0 ] && [ "$(sed -n 1p "$T/out")" = "0 1 2 3 4 5 6 7" ]; then ok "IT-85 the exit-code symbols EX_OK to EX_WORKER are 0 to 7"; else bad "IT-85 the exit-code symbols" "rc=$rc: $(sed -n 1p "$T/out")"; fi
# IT-86 podman_ok and compose_provider_ok: a failed preflight prints the cause and fix through the catalogue (the lane that owns the id answers first: prereq_msg, then inst_msg),
# and the LAST line is still today's fixed sentence with exit 1
fresh; FAKE_COMPOSE_VERSION_FAIL=1 sh "$install" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?
if [ "$rc" = 3 ] && grep -q '^problem: ' "$T/out" && grep -q '^fix: ' "$T/out" && [ "$(tail -1 "$T/out")" = "$MSG_A" ]; then ok "IT-86a a failed compose preflight prints problem: and fix: (inst_msg) and ends with the fixed sentence, exit 1"; else bad "IT-86a a failed preflight prints problem and fix" "rc=$rc: $(cat "$T/out" | cut -c1-120)"; fi
rm -rf "$T/d83"; mkdir -p "$T/d83"; cp "$install" "$T/d83/install.sh"
printf '%s\n' 'prereq_msg() { case $1 in compose_not_found) echo "problem: FROM-PREREQ" >&2; echo "fix: FROM-PREREQ-FIX" >&2; return 0 ;; esac; return 1; }' > "$T/d83/install-prereq.sh"
fresh; FAKE_COMPOSE_VERSION_FAIL=1 sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?
if [ "$rc" = 3 ] && grep -qx 'problem: FROM-PREREQ' "$T/out" && ! grep -q 'no compose provider found' "$T/out" && [ "$(tail -1 "$T/out")" = "$MSG_A" ]; then ok "IT-86b the lane that owns a catalogue id answers first (prereq_msg before inst_msg)"; else bad "IT-86b the catalogue dispatcher" "rc=$rc: $(cat "$T/out" | cut -c1-160)"; fi
# IT-87 every id of inst_msg prints a problem: line and a fix: line and no secret-looking text (G7: each lane tests its own table)
ids=$(sed -n '/^inst_msg() {/,/^}/p' "$install" | grep -oE '^    [a-z_0-9]+\)' | tr -d ' )')
b87=""
[ -n "$ids" ] || b87="no ids found in inst_msg"
for id in $ids; do
  ifc "machine_look() { msg $id; }" --check
  if [ "$(grep -c '^problem: ' "$T/out")" != 1 ] || [ "$(grep -c '^fix: ' "$T/out")" != 1 ] || grep -qiE "$(case $id in token_left|credential_marker_failed|token_stage_left|mint_busy|token_not_private) echo 'password|secret' ;; *) echo 'token|password|secret' ;; esac)" "$T/out"; then b87="$b87 [$id]"; fi   # W1-300e/W1-303a: W308c adds the mandated non-private refusal: only these five public message IDs may name token; password/secret and actual credential/fragment guards stay.
done
ifc 'machine_look() { msg no_such_id_at_all || echo "rc=$?"; }' --check   # (--check is bare since 6E3-1: errexit is on in the body, so the status is read in the || branch)
grep -qx 'rc=1' "$T/out" || b87="$b87 [an unknown id did not return 1]"
if [ -z "$b87" ]; then ok "IT-87 every inst_msg id prints exactly one problem: and one fix: line, no secret-looking text; an unknown id returns 1"; else bad "IT-87 the catalogue table" "$b87"; fi
# IT-89 podman_ok: podman older than 5 is refused with its version (numeric, not by string), 5.0.0 and 10.1.0 are accepted, a version that is not a number is refused
b89=""
for v in 4.9.4 3.0.1 banana; do
  fresh; FAKE_PODMAN_VERSION=$v run && rc=0 || rc=$?
  want="install: podman 5 or newer is required (found $(printf '%s' "$v" | sed 's/[^0123456789.]//g'))"
  if [ "$rc" != 3 ] || [ "$(tail -1 "$T/out")" != "$want" ] || ! grep -q '^fix: ' "$T/out" || ! untouched; then b89="$b89 [$v rc=$rc: $(tail -1 "$T/out" | cut -c1-70)]"; fi
done
for v in 5.0.0 10.1.0; do fresh; FAKE_PODMAN_VERSION=$v run && rc=0 || rc=$?; [ "$rc" = 0 ] || b89="$b89 [$v refused rc=$rc]"; done
if [ -z "$b89" ]; then ok "IT-89 podman 4 and 3 are refused with their version and a fix line, a version that is not a number is refused, 5.0.0 and 10.1.0 are accepted"; else bad "IT-89 podman_ok" "$b89"; fi
# ---- 6d-1: the install reads a RELEASE laid out by the fetch step (<home>/current with stack.env), not the source checkout ----
SRV_IMG="localhost/cortex-server@sha256:$(printf 'a%.0s' $(seq 64))"
DB_IMG="docker.io/pgvector/pgvector@sha256:$(printf 'b%.0s' $(seq 64))"
# mkrel [stack.env text]: a release directory at $T/h/current from the repository's own files, each member carrying a mark of its own
mkrel() {
  rel="$T/h/current"
  rm -rf "$rel" && mkdir -p "$rel/bin" "$rel/systemd" "$rel/initdb" || return 1
  cp "$here/../deploy/compose.yml" "$rel/" || return 1
  # the release's roles script says it ran (the real one is byte-identical to the checkout's, so the copy is told apart by this line)
  { sed -n 1p "$here/../deploy/db-roles.sh" && echo 'echo release-roles >> "${FAKE_STATE:-/dev/null}/roles.ran"' && sed -n '2,$p' "$here/../deploy/db-roles.sh"; } > "$rel/db-roles.sh" || return 1
  { echo "# from the release" && cat "$here/../deploy/cortex.toml.example"; } > "$rel/cortex.toml.example" || return 1
  for f in cortex-dump cortex-dump-status cortex-dump-alert cortex-restore restore-compare; do printf '#!/bin/sh\n# release script %s\n' "$f" > "$rel/bin/$f" || return 1; done
  for u in cortex-dump.service cortex-dump.timer cortex-dump-status.service cortex-dump-status.timer cortex-dump-alert.service; do { echo "# release unit" && cat "$here/../deploy/systemd/$u"; } > "$rel/systemd/$u" || return 1; done
  printf '%s\n' "${1-CORTEX_VERSION=0.2.1-rc.2
CORTEX_SERVER_IMAGE=$SRV_IMG
CORTEX_DB_IMAGE=$DB_IMG}" > "$rel/stack.env"
}
# IT-90 release mode: every file comes from <home>/current (compose, the roles script, the config example, the scripts, the units), the images of stack.env reach compose,
# nothing is built, and a source checkout's files are not used
b90=""
if fresh && mkdir -p "$T/h" && mkrel; then
PATH="$T/bin-git:$PATH" FAKE_GIT_OUT="$(printf '1%.0s' $(seq 40))" run && rc=0 || rc=$?   # a git that names a sha: a release must not take it
[ "$rc" = 0 ] || b90="$b90 [rc=$rc: $(tail -1 "$T/out" | cut -c1-80)]"
grep -qF -- "compose -f $T/h/current/compose.yml up -d db" "$T/log" || b90="$b90 [compose was not run from the release]"
! grep -q 'compose .* build' "$T/log" || b90="$b90 [the release was built]"
! grep '^compose -f ' "$T/log" | grep -qvF "compose -f $T/h/current/compose.yml " || b90="$b90 [a compose call did not use the release's compose.yml]"
[ "$(grep -o 'CORTEX_GIT_SHA=[^ ]*' "$T/state/compose.env" 2>/dev/null | sort -u)" = "CORTEX_GIT_SHA=unknown" ] || b90="$b90 [the release took a git sha from the checkout it is not part of]"
[ "$(sed -n 2p "$T/h/bin/cortex-dump")" = "# release script cortex-dump" ] || b90="$b90 [the script is not the release's]"
grep -q '^# from the release' "$T/h/cortex.toml" || b90="$b90 [the config example is not the release's]"
grep -q '^# release unit' "$HOME/.config/systemd/user/cortex-dump.service" 2>/dev/null || b90="$b90 [the unit is not the release's]"
grep -qxF "CORTEX_VERSION=0.2.1-rc.2 CORTEX_SERVER_IMAGE=$SRV_IMG CORTEX_DB_IMAGE=$DB_IMG" "$T/state/compose.images" 2>/dev/null || b90="$b90 [compose did not get the version and images of stack.env]"
[ "$(cat "$T/state/roles.ran" 2>/dev/null)" = "release-roles" ] || b90="$b90 [the roles script is not the release's]"
if [ -z "$b90" ]; then ok "IT-90 a release at <home>/current is what the install reads: compose, roles script, config, scripts, units, the images of stack.env; nothing is built"; else bad "IT-90 release layout" "$b90"; fi
else bad "IT-90 fixture setup" "could not prepare dependent fixture"; fi
# IT-90c (hale N-5) the release image refuses an empty, malformed or missing feature set at BUILD time: an empty deploy/server-features would build the core pieces only, without an error.
# The guard in front of the cargo build is read out of the Containerfile and run on sample files (the build itself is never run here: no image is built by this suite).
b90c=""; cf90c="$(dirname "$install")/Containerfile"
guard90c=$(sed -n 's/^RUN \(.*\) && cargo build --release --locked -p cortex-server --features "\$(cat server-features)"$/\1/p' "$cf90c")
if [ -z "$guard90c" ]; then
  b90c="$b90c [the cargo build RUN line of the Containerfile has no guard in front of it]"
else
  rm -rf "$T/g90c"; mkdir -p "$T/g90c"
  for pair in "accept:mod-policy,mod-records,mod-search,mod-handoffs,mod-boot" "accept:mod-policy" "refuse:" "refuse:@NL" "refuse:mod-policy mod-records" "refuse:Mod-Policy" "refuse:mod-policy," "refuse:a,,b" "refuse:@TWO" "refuse:@MISSING" "refuse:@UNTERMINATED2" "refuse:@NOLF"; do
    want=${pair%%:*}; body=${pair#*:}; rm -f "$T/g90c/server-features"
    case $body in
      @MISSING) ;;
      @NL) printf '\n' > "$T/g90c/server-features" ;;
      @TWO) printf 'mod-policy\nmod-records\n' > "$T/g90c/server-features" ;;
      @UNTERMINATED2) printf 'mod-policy\nmod-records' > "$T/g90c/server-features" ;;   # V6D-01 (vera): one LF, two records, the second not terminated: wc -l says 1 and grep -Eq matches the first line
      @NOLF) printf 'mod-policy,mod-records' > "$T/g90c/server-features" ;;
      "") : > "$T/g90c/server-features" ;;
      *) printf '%s\n' "$body" > "$T/g90c/server-features" ;;
    esac
    if (cd "$T/g90c" && sh -c "$guard90c") >/dev/null 2>&1; then got=accept; else got=refuse; fi
    [ "$got" = "$want" ] || b90c="$b90c [$body: the guard says $got, expected $want]"
  done
fi
grep -q 'features "\$(cat server-features)"' "$cf90c" || b90c="$b90c [the build no longer reads the set with --features \"\$(cat server-features)\"]"
if [ -z "$b90c" ]; then ok "IT-90c the Containerfile refuses an empty, malformed or missing feature set before the cargo build (twelve sample files)"; else bad "IT-90c the feature-set guard" "$b90c"; fi
# IT-91 stack.env is read strictly (the engine's rules: three keys, exact spelling, no duplicate, no CR, name@sha256:<64 lower hex>); anything else stops the install before it touches podman
b91=""
for v in "CORTEX_VERSION=1.0.0
CORTEX_SERVER_IMAGE=$SRV_IMG" "CORTEX_VERSION=1.0.0
CORTEX_SERVER_IMAGE=$SRV_IMG
CORTEX_DB_IMAGE=$DB_IMG
EXTRA=1" "CORTEX_VERSION=1.0.0
CORTEX_SERVER_IMAGE=$SRV_IMG
CORTEX_SERVER_IMAGE=$SRV_IMG
CORTEX_DB_IMAGE=$DB_IMG" "CORTEX_VERSION=1.0.0
CORTEX_SERVER_IMAGE=localhost/cortex-server@sha256:$(printf 'A%.0s' $(seq 64))
CORTEX_DB_IMAGE=$DB_IMG" "CORTEX_VERSION=1.0.0
CORTEX_SERVER_IMAGE=localhost/cortex-server@sha256:abc
CORTEX_DB_IMAGE=$DB_IMG" "CORTEX_VERSION=1.0.0
CORTEX_SERVER_IMAGE=localhost/cortex-server:latest
CORTEX_DB_IMAGE=$DB_IMG" "CORTEX_VERSION=
CORTEX_SERVER_IMAGE=$SRV_IMG
CORTEX_DB_IMAGE=$DB_IMG" "cortex_server_image=$SRV_IMG" "" \
  "CORTEX_VERSION=1.0.0$(printf '\r')
CORTEX_SERVER_IMAGE=$SRV_IMG
CORTEX_DB_IMAGE=$DB_IMG" "CORTEX_VERSION=1.0.0
this is not a pair
CORTEX_SERVER_IMAGE=$SRV_IMG
CORTEX_DB_IMAGE=$DB_IMG" "CORTEX_VERSION=1.0 0
CORTEX_SERVER_IMAGE=$SRV_IMG
CORTEX_DB_IMAGE=$DB_IMG" "CORTEX_VERSION=1.0.0/../x
CORTEX_SERVER_IMAGE=$SRV_IMG
CORTEX_DB_IMAGE=$DB_IMG" "CORTEX_VERSION=1.0.0
CORTEX_SERVER_IMAGE=localhost/cortex server@sha256:$(printf 'a%.0s' $(seq 64))
CORTEX_DB_IMAGE=$DB_IMG" "CORTEX_VERSION=1.0.0
CORTEX_SERVER_IMAGE=$SRV_IMG
CORTEX_DB_IMAGE=docker.io/pgvector/pgvector@sha256:$(printf 'b%.0s' $(seq 65))" "CORTEX_VERSION=1.0.0
CORTEX_SERVER_IMAGE=@sha256:$(printf 'a%.0s' $(seq 64))
CORTEX_DB_IMAGE=$DB_IMG" "CORTEX_VERSION=1.0.0
CORTEX_SERVER_IMAGE=$(printf 'n%.0s' $(seq 301))@sha256:$(printf 'a%.0s' $(seq 64))
CORTEX_DB_IMAGE=$DB_IMG" "CORTEX_VERSION=1.0.0
CORTEX_SERVER_IMAGE=$SRV_IMG
CORTEX_DB_IMAGE=docker.io/pgvector/pgvector@sha256:$(printf 'b%.0s' $(seq 63))g" "CORTEX_VERSION=1.0.0
CORTEX_SERVER_IMAGE=$SRV_IMG
CORTEX_DB_IMAGE=$DB_IMG
 CORTEX_VERSION=2.0.0" "CORTEX_VERSION=1.0.0
CORTEX_VERSION=1.0.0
CORTEX_SERVER_IMAGE=$SRV_IMG
CORTEX_DB_IMAGE=$DB_IMG" "CORTEX_VERSION=1.0.0
CORTEX_SERVER_IMAGE=$SRV_IMG
CORTEX_DB_IMAGE=$DB_IMG
CORTEX_DB_IMAGE=$DB_IMG"; do
  fresh
  if ! { mkdir -p "$T/h" && mkrel "$v"; }; then b91="$b91 [fixture setup]"; continue; fi
  run && rc=0 || rc=$?
  if [ "$rc" != 5 ] || ! tail -1 "$T/out" | grep -q 'has a stack.env that cannot be used' || ! grep -q '^fix: ' "$T/out" || grep -qvxE -e '--version|compose version' "$T/log" || [ -n "$(ls -A "$T/state")" ]; then b91="$b91 [$(printf '%s' "$v" | tr '\n' '~' | cut -c1-40) rc=$rc]"; fi
done
if fresh && mkdir -p "$T/h" && mkrel "# a comment

CORTEX_DB_IMAGE=$DB_IMG
CORTEX_VERSION=2.0.0-rc.1
CORTEX_SERVER_IMAGE=$SRV_IMG"; then
run && rc=0 || rc=$?
[ "$rc" = 0 ] || b91="$b91 [a good stack.env (comments, blank lines, any order) was refused: rc=$rc $(tail -1 "$T/out" | cut -c1-60)]"
else b91="${b91} [fixture setup]"; fi
if fresh && mkdir -p "$T/h" && mkrel && printf 'CORTEX_VERSION=1.2.3\nCORTEX_SERVER_IMAGE=reg.example:5000/team/cortex-server@sha256:%s\nCORTEX_DB_IMAGE=%s' "$(printf 'c%.0s' $(seq 64))" "$DB_IMG" > "$T/h/current/stack.env"; then
run && rc=0 || rc=$?
[ "$rc" = 0 ] || b91="$b91 [a good stack.env (no final newline, a registry port) was refused: rc=$rc $(tail -1 "$T/out" | cut -c1-60)]"
else b91="${b91} [fixture setup]"; fi
if [ -z "$b91" ]; then ok "IT-91 stack.env: three keys, exact spelling, no duplicate, name@sha256:<64 lower hex>, anything else stops the install before podman is touched"; else bad "IT-91 stack.env" "$b91"; fi
# IT-92 a release without one of the backup scripts is refused with the release directory named, before anything is made
if fresh && mkdir -p "$T/h" && mkrel && rm -f "$T/h/current/bin/cortex-restore"; then
run && rc=0 || rc=$?
if [ "$rc" = 5 ] && tail -1 "$T/out" | grep -q "backup script cortex-restore is missing in $T/h/current/bin" && [ ! -e "$T/h/dumps" ]; then ok "IT-92 a release without a backup script is refused with its directory named, before anything is made"; else bad "IT-92 a release without a backup script" "rc=$rc: $(tail -1 "$T/out" | cut -c1-120)"; fi
else bad "IT-92 fixture setup" "could not prepare dependent fixture"; fi
# IT-93 read_stack_env says what the ENGINE says: scripts/stack-env-corpus holds the files (the live count is in the message below; it was 84 at the first run of this cell, 96 at b27a969, 99 with the NUL and u64 cells) and the engine's own verdict on each (engine-verdicts.txt, from the engine's parse_stack_env);
# the shell reader must agree on every one (the first reader differed on 31: no SemVer rule, no length or size limits, blank and indented-comment lines)
b93=""
vf="$here/stack-env-corpus/engine-verdicts.txt"; nfiles=0; nacc=0; nref=0
{ sed -n '/^valid_semver() {/,/^}/p' "$install"; sed -n '/^read_stack_env() {/,/^}/p' "$install"; } > "$T/se-funcs.sh"
se_loaded=1; sh -n "$T/se-funcs.sh" 2>/dev/null && . "$T/se-funcs.sh" || { se_loaded=0; b93="$b93 [valid_semver and read_stack_env do not parse]"; }
[ "$se_loaded" = 0 ] || for f in "$here"/stack-env-corpus/*.env; do
  nfiles=$((nfiles + 1)); n=$(basename "$f"); want=$(grep -v '^#' "$vf" | awk -v n="$n" '$1 == n { print $2 }')
  case $want in ACCEPT) nacc=$((nacc + 1)) ;; REFUSE) nref=$((nref + 1)) ;; *) b93="$b93 [no engine verdict for $n]"; continue ;; esac
  if (read_stack_env "$f" >/dev/null 2>&1); then got=ACCEPT; else got=REFUSE; fi
  [ "$got" = "$want" ] || b93="$b93 [$n: engine $want, install.sh $got]"
done
[ "$(grep -vc '^#' "$vf")" -eq "$nfiles" ] || b93="$b93 [the verdict file and the corpus differ in size]"
{ [ "$nacc" -ge 15 ] && [ "$nref" -ge 50 ]; } || b93="$b93 [the corpus is too small a mix: $nacc accepted, $nref refused]"
if [ -z "$b93" ]; then ok "IT-93 install.sh reads stack.env as the engine does: $nfiles corpus files, $nacc accepted and $nref refused, the same verdict on every one"; else bad "IT-93 stack.env against the engine" "$b93"; fi
# IT-93b the six known differences are written down and held: the engine's verdict on each is recorded (known-differences/engine-verdicts.txt), install.sh says the opposite, as documented
# in read_stack_env's comment (Unicode white space is blank to the engine; a file that is not UTF-8 is refused by the engine and a comment is not read here; a version core of 10^19 or u64::MAX is valid to
# the engine and refused here: install.sh stops at 19 digits, fail closed; a NUL byte inside a comment is skipped by the engine and refused here: the NUL test is on the whole file, fail closed)
b93b=""
kv="$here/stack-env-corpus/known-differences/engine-verdicts.txt"
[ "$se_loaded" = 1 ] || b93b="$b93b [the functions did not load]"
for pair in nbsp-only-line:ACCEPT:REFUSE nel-only-line:ACCEPT:REFUSE invalid-utf8-comment:REFUSE:ACCEPT core-1e19-valid-in-engine:ACCEPT:REFUSE core-u64-max-valid-in-engine:ACCEPT:REFUSE nul-in-comment-valid-in-engine:ACCEPT:REFUSE; do
  [ "$se_loaded" = 1 ] || break
  n=${pair%%:*}; rest=${pair#*:}; eng=${rest%%:*}; mine=${rest#*:}
  [ "$(grep -v '^#' "$kv" | awk -v n="$n.env" '$1 == n { print $2 }')" = "$eng" ] || b93b="$b93b [$n: the recorded engine verdict is not $eng]"
  if (read_stack_env "$here/stack-env-corpus/known-differences/$n.env" >/dev/null 2>&1); then got=ACCEPT; else got=REFUSE; fi
  [ "$got" = "$mine" ] || b93b="$b93b [$n: install.sh now says $got, the documented difference is $mine]"
done
[ "$(grep -vc '^#' "$kv")" -eq 6 ] || b93b="$b93b [the known-differences verdict file does not hold exactly six files]"
if [ -z "$b93b" ]; then ok "IT-93b the six known differences from the engine's stack.env reading are held as written (three of text, two of the u64 boundary of a version core, one NUL byte in a comment)"; else bad "IT-93b known differences" "$b93b"; fi
# IT-93c the reason each refusal gives (STACK_WHY, shown to the operator: a fixed sentence and a line number, never the text of the line)
b93c=""
if [ "$se_loaded" = 1 ]; then
  for pair in bad-missing-version:"CORTEX_VERSION is missing" bad-missing-server:"CORTEX_SERVER_IMAGE is missing" bad-missing-db:"CORTEX_DB_IMAGE is missing" \
              bad-extra-key:"line 4 names a key this file may not hold" bad-dup-version:"line 4 repeats a key" bad-line-words:"line 1 is not KEY=VALUE" \
              bad-value-301-bytes:"line 2 has an empty or too long value" bad-value-quoted:"line 1 has a value with a character outside the allowed ones" \
              bad-size-8193:"larger than 8192 bytes" bad-version-00:"CORTEX_VERSION is not a SemVer" bad-image-tag:"is not name@sha256" \
              bad-image-name-empty:"an image has an empty name" bad-image-digest-63:"a digest that is not 64 digits" \
              bad-image-digest-upper:"an image digest is not 64 lower-case hex digits" bad-image-name-second-at:"an image name holds a second @"; do
    n=${pair%%:*}; want=${pair#*:}
    got=$( (read_stack_env "$here/stack-env-corpus/$n.env" >/dev/null 2>&1; printf '%s' "$STACK_WHY") || true)
    case $got in *"$want"*) ;; *) b93c="$b93c [$n: said '$got', wanted '$want']" ;; esac
  done
  secret=$( (read_stack_env "$here/stack-env-corpus/bad-secret-key.env" >/dev/null 2>&1; printf '%s' "$STACK_WHY") || true)
  case $secret in *hunter2*|*PASSWORD*) b93c="$b93c [a refusal repeated the text of a line: $secret]" ;; esac
else b93c="$b93c [the functions did not load]"; fi
if [ -z "$b93c" ]; then ok "IT-93c every kind of refusal gives its own reason and a line number, and never repeats a line"; else bad "IT-93c stack.env reasons" "$b93c"; fi
# IT-88 install.sh holds exactly the two include lines of the interface, each with the exact trailing marker, in the tolerant dev form
n88=$(grep -c '# @include$' "$install" || true)
l88=$(grep -E '^\[ ! -f "\$here/install-(prereq|fetch)\.sh" \] \|\| \. "\$here/install-(prereq|fetch)\.sh"   # @include$' "$install" | sort | tr '\n' '~')
if [ "$n88" = 2 ] && [ "$(printf '%s' "$l88" | tr '~' '\n' | grep -c .)" = 2 ]; then ok "IT-88 install.sh has exactly two include lines (install-prereq.sh, install-fetch.sh) in the dev form"; else bad "IT-88 the include lines" "marker lines=$n88 well-formed=$(printf '%s' "$l88" | tr '~' '\n' | grep -c .)"; fi

# ===================================================================================================================================================================
# 6e: THE EXIT-CODE TABLE (INSTALL-LANES-INTERFACE-hue-r0 section 5, owned here since kai's row cfc1bccc): every failure ends with ITS code, ONE problem: line, ONE fix: line
# (cause and fix, G7) and the usual last line "install: ..."; the symbols are the interface's plus EX_BACKUP=8 (the stack is up, the backups could not be scheduled: r0 had no code for step 9).
# ===================================================================================================================================================================
chk94() { # chk94 <cell> <wanted exit code> <what>   (after a run that set rc and $T/out)
  c94p=$(grep -c '^problem: ' "$T/out" || true); c94f=$(grep -c '^fix: ' "$T/out" || true)
  if [ "$rc" = "$2" ] && [ "$c94p" = 1 ] && [ "$c94f" = 1 ] && tail -1 "$T/out" | grep -q '^install: '; then ok "$1 $3: exit $2, one problem: and one fix: line"
  else bad "$1 $3" "rc=$rc wanted $2; problem lines $c94p, fix lines $c94f; last: $(tail -1 "$T/out" | cut -c1-90)"; fi
}
runc() { fresh; sh "$install" --project p --lead lead --home "$T/h" "$@" > "$T/out" 2>&1 && rc=0 || rc=$?; }   # runc [args]: a fresh run, rc and $T/out set
# IT-94 the symbols: the interface's eight and EX_BACKUP=8, written once, in one line
if grep -qx 'EX_OK=0 EX_USAGE=1 EX_MACHINE=2 EX_PREREQ=3 EX_TRUST=4 EX_FETCH=5 EX_START=6 EX_WORKER=7 EX_BACKUP=8' "$install"; then ok "IT-94 the exit-code symbols are the interface's eight plus EX_BACKUP=8, in one line"; else bad "IT-94 the exit-code symbols" "$(grep -n '^EX_OK=' "$install" | cut -c1-120)"; fi
# IT-95 step 1 (the machine) is EX_MACHINE 2, step 2 (the prerequisites) is EX_PREREQ 3
mkdir -p "$T/bin-root" "$T/bin-nocg"
printf '#!/bin/sh\n[ "$1" = -u ] && { echo 0; exit 0; }\nexec /usr/bin/id "$@"\n' > "$T/bin-root/id"; chmod +x "$T/bin-root/id"
printf '#!/bin/sh\n[ "$1" = -fc ] && [ "$2" = %%T ] && { echo tmpfs; exit 0; }\nexec /usr/bin/stat "$@"\n' > "$T/bin-nocg/stat"; chmod +x "$T/bin-nocg/stat"
fresh; PATH="$T/bin-root:$PATH" sh "$install" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?; chk94 IT-95a 2 "run as root"
fresh; PATH="$T/bin-nocg:$PATH" sh "$install" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?; chk94 IT-95b 2 "no cgroup v2"
fresh; FAKE_PODMAN_VERSION=4.9.0 sh "$install" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?; chk94 IT-95c 3 "podman older than 5"
fresh; FAKE_COMPOSE_VERSION_FAIL=1 sh "$install" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?; chk94 IT-95d 3 "no compose provider"
fresh; FAKE_COMPOSE_VERSION_OUT='podman-compose version 1.4.9\n' sh "$install" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?; chk94 IT-95e 3 "compose provider too old"
fresh; FAKE_COMPOSE_VERSION_OUT='something else\n' sh "$install" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?; chk94 IT-95f 3 "compose provider not confirmed"
# IT-96 the release tree (steps 3 and 4 leave it, step 5 onward read it) is EX_FETCH 5 when it is not usable
fresh
if mkdir -p "$T/h" && mkrel 'CORTEX_VERSION=0.2.1-rc.2'; then
  run && rc=0 || rc=$?; chk94 IT-96a 5 "stack.env that cannot be used"
else bad "IT-96a fixture setup"; fi
fresh
if mkdir -p "$T/h" && mkrel && rm -f "$T/h/current/bin/cortex-restore"; then
  run && rc=0 || rc=$?; chk94 IT-96b 5 "release without a backup script"
else bad "IT-96b fixture setup"; fi
fresh; mv "$T/scripts/cortex-restore" "$T/cortex-restore.away"; run && rc=0 || rc=$?; mv "$T/cortex-restore.away" "$T/scripts/cortex-restore"; chk94 IT-96c 5 "checkout without a backup script"
# IT-97 preparing and starting the stack (secrets, config, directories, database, roles, build, migrate, health) is EX_START 6
fresh
if mkdir -p "$T/h" && ln -s "$T" "$T/h/dumps"; then
  run && rc=0 || rc=$?; chk94 IT-97a 6 "dumps is a symbolic link"
else bad "IT-97a fixture setup"; fi
fresh
if mkdir -p "$T/h" && ln -s "$T" "$T/h/dump-status"; then
  run && rc=0 || rc=$?; chk94 IT-97b 6 "dump-status is a symbolic link"
else bad "IT-97b fixture setup"; fi
fresh; PATH="$T/bin-badrand:$PATH" sh "$install" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?; chk94 IT-97c 6 "no random bytes for a secret"
fresh
if mkdir -p "$T/h" && printf '[server]\nport = 1\n' > "$T/h/cortex.toml"; then
  run && rc=0 || rc=$?; chk94 IT-97d 6 "config without a [modules] list"
else bad "IT-97d fixture setup"; fi
fresh
if mkdir -p "$T/h" && printf '[modules]\nenabled = [\n  "a",\n]\n' > "$T/h/cortex.toml"; then
  run && rc=0 || rc=$?; chk94 IT-97e 6 "[modules] list on several lines"
else bad "IT-97e fixture setup"; fi
fresh
if mkdir -p "$T/h" && printf '[modules]\nenabled = []\n' > "$T/h/cortex.toml"; then
  run && rc=0 || rc=$?; chk94 IT-97f 6 "empty [modules] list"
else bad "IT-97f fixture setup"; fi
fresh
if mkdir -p "$T/h" && printf '[modules]\nenabled = ["Bad-Id"]\n' > "$T/h/cortex.toml"; then
  run && rc=0 || rc=$?; chk94 IT-97g 6 "invalid module id"
else bad "IT-97g fixture setup"; fi
fresh; FAKE_COMPOSE_FAIL=1 sh "$install" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?; chk94 IT-97h 6 "compose cannot start the database"
fresh; FAKE_HBA_TRUST=2 sh "$install" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?; chk94 IT-97i 6 "database with trust rules"
fresh; FAKE_HBA_FAIL=1 sh "$install" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?; chk94 IT-97j 6 "trust rules cannot be read"
fresh; FAKE_HBA_RAW='abc' sh "$install" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?; chk94 IT-97k 6 "trust rule count is not a number"
fresh; FAKE_PSQL_FAIL=1 sh "$install" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?; chk94 IT-97l 6 "module roles fail"
fresh; FAKE_BUILD_FAIL=1 sh "$install" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?; chk94 IT-97m 6 "image build fails"
fresh; FAKE_MIGRATE_FAIL=1 sh "$install" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?; chk94 IT-97n 6 "migrations fail"
fresh; FAKE_HEALTH=starting sh "$install" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?; chk94 IT-97o 6 "containers never become healthy"
# IT-98 the first project and lead token (step 8) is EX_WORKER 7; a reissue on a stack that is not healthy is still EX_START 6
fresh; FAKE_BOOTSTRAP_FAIL=1 sh "$install" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?; chk94 IT-98a 7 "bootstrap fails"
fresh; FAKE_CAT_EMPTY=1 sh "$install" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?; chk94 IT-98b 7 "the token copy is empty"
fresh; run >/dev/null 2>&1 || :; FAKE_BOOTSTRAP_FAIL=1 sh "$install" --project p --lead lead --home "$T/h" --reissue-token > "$T/out" 2>&1 && rc=0 || rc=$?; chk94 IT-98c 7 "reissue: bootstrap fails"
fresh; run >/dev/null 2>&1 || :; FAKE_HEALTH=starting sh "$install" --project p --lead lead --home "$T/h" --reissue-token > "$T/out" 2>&1 && rc=0 || rc=$?; chk94 IT-98d 6 "reissue: the stack is not healthy"
# IT-99 the backup units and timers (step 9) are EX_BACKUP 8: the stack is up
fresh; FAKE_SYSTEMCTL_FAIL=1 sh "$install" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?; chk94 IT-99a 8 "the user manager cannot be reached"
# IT-100 a bad argument stays EX_USAGE 1 and starts nothing
runc --port 12x; if [ "$rc" = 1 ] && [ ! -s "$T/log" ]; then ok "IT-100a a bad --port is exit 1 and no podman call"; else bad "IT-100a bad --port" "rc=$rc calls=$(wc -l < "$T/log" | tr -d ' ')"; fi
fresh; sh "$install" --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?; if [ "$rc" = 1 ] && [ ! -s "$T/log" ]; then ok "IT-100b a missing --project is exit 1 and no podman call"; else bad "IT-100b missing --project" "rc=$rc"; fi

# the mutation harness may stop the suite at two points once the cells a mutant names have run (INSTALL_TEST_STOP_AFTER=100 or 110; unset = the whole suite, which is what every receipt uses)
stop_after() { [ "${INSTALL_TEST_STOP_AFTER:-}" = "$1" ] || return 0; echo "install-test: stopped after IT-$1 for mutation judging: $pass passed, $fail failed"; [ "$fail" -eq 0 ]; exit; }
stop_after 100
# ===================================================================================================================================================================
# kai's row 5241a6a1 (the two defects of nix's G12 input inventory, routed to the one writer of install.sh), before the rest of 6e:
#  IT-101 CORTEX_INSTALL_POLL and CORTEX_INSTALL_WAIT reach shell arithmetic, and bash evaluates a command substitution inside an arithmetic expression: both are checked as plain
#         whole numbers BEFORE anything is looked at or started, and a bad value is refused with its cause and fix (EX_USAGE 1).
#  IT-102 the online fetch (marv's rel_fetch) reads FETCH_BASE_URL while install.sh holds CORTEX_BASE_URL: install.sh sets the one from the other, in one place, always (an
#         inherited FETCH_BASE_URL must not turn the online channel on without --base-url's checks).
# NO destructive payload (W1-70): the canary appends one line to a file under $T, and every run that could execute it is inside an unprivileged user+mount namespace that keeps our uid.
# ===================================================================================================================================================================
ns101="unshare --user --map-user=$(id -u) --map-group=$(id -g) --mount"
can101="$T/canary101"
mkdir -p "$T/bin-sleep"; printf '#!/bin/sh\nexit 0\n' > "$T/bin-sleep/sleep"; chmod +x "$T/bin-sleep/sleep"
canval101='HOME[$(echo c >> '"$can101"')]'   # a SET variable name: under set -u an unset one stops before the subscript is evaluated, a set one does not
cv101() { # cv101 <POLL|WAIT> <value>: a fresh install that has to wait (the containers never become healthy), with that value in that variable
  rm -f "$can101"; fresh
  # shellcheck disable=SC2086
  # timeout: an unvalidated 100000 s wait or a poll of 0 would otherwise hang the suite instead of failing the cell (rc 124)
  # a no-op sleep (the real one refuses a value such as x[...] before the arithmetic is reached, and the cells would not wait)
  timeout 20 env PATH="$T/bin-sleep:$PATH" "CORTEX_INSTALL_$1=$2" FAKE_HEALTH=starting $ns101 sh "$install" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?
}
# IT-101a the control: this shell DOES run a command that sits in an arithmetic value (so IT-101b can fail); on a shell that does not, the refusal cells still hold
rm -f "$can101"; $ns101 sh -c 'p=$1; w=0; w=$((w + p))' sh "$canval101" >/dev/null 2>&1 || :
if [ -f "$can101" ]; then ok "IT-101a control: this shell runs a command substitution that is the value of an arithmetic operand (the canary file appeared)"; else ok "IT-101a control: this shell does not evaluate such a value (the canary file did not appear); the refusal cells below still hold"; fi
b101=""
for c101 in "POLL:$canval101" "POLL:0" "POLL:-1" "POLL:1.5" "POLL:abc" "POLL:1+1" "POLL:01" "POLL: 1" "POLL:100000" "WAIT:$canval101" "WAIT:-1" "WAIT:abc" "WAIT:1.5" "WAIT:01" "WAIT:100000"; do
  v101=${c101%%:*}; x101=${c101#*:}; cv101 "$v101" "$x101"
  p101=$(grep -c '^problem: ' "$T/out" || true); f101=$(grep -c '^fix: ' "$T/out" || true)
  { [ "$rc" = 1 ] && [ ! -e "$can101" ] && untouched && [ "$p101" = 1 ] && [ "$f101" = 1 ] && tail -1 "$T/out" | grep -q "^install: CORTEX_INSTALL_$v101 "; } \
    || b101="$b101 [$v101=$(printf '%.20s' "$x101") rc=$rc canary=$([ -e "$can101" ] && echo RAN || echo no) problem=$p101 fix=$f101 untouched=$(untouched && echo y || echo n): $(tail -1 "$T/out" | cut -c1-50)]"
done
if [ -z "$b101" ]; then ok "IT-101b a CORTEX_INSTALL_POLL or CORTEX_INSTALL_WAIT that is not a plain whole number (a command substitution, 0 for the poll, a sign, a fraction, a leading zero, a space, six digits) is refused: exit 1, one problem: and one fix: line, nothing started, the canary never ran"; else bad "IT-101b bad POLL or WAIT values" "$b101"; fi
rm -f "$can101"; fresh; env CORTEX_INSTALL_POLL=1 CORTEX_INSTALL_WAIT=0 sh "$install" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?
r101=$rc; fresh; env CORTEX_INSTALL_POLL=1 CORTEX_INSTALL_WAIT=2 sh "$install" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?
if [ "$r101" = 0 ] && [ "$rc" = 0 ]; then ok "IT-101c plain values are accepted: POLL=1 with WAIT=0 and POLL=1 with WAIT=2 both install"; else bad "IT-101c good POLL and WAIT values" "WAIT=0 rc=$r101, WAIT=2 rc=$rc: $(tail -1 "$T/out" | cut -c1-70)"; fi
# IT-102 the fetch base: --base-url reaches FETCH_BASE_URL unchanged; without it an inherited FETCH_BASE_URL is overwritten with the (empty) configured base
fb102='machine_look() { printf "BASE=%s FETCH=%s\n" "$CORTEX_BASE_URL" "${FETCH_BASE_URL-unset}"; }'
ifc "$fb102" --check --base-url https://releases.example.invalid/cortex
a102=$(sed -n 's/^BASE=//p' "$T/out")
ifc "$fb102" --check
b102=$(sed -n 's/^BASE=//p' "$T/out")
FETCH_BASE_URL=https://inherited.example.invalid/x ifc "$fb102" --check
c102=$(sed -n 's/^BASE=//p' "$T/out")
if [ "$a102" = "https://releases.example.invalid/cortex FETCH=https://releases.example.invalid/cortex" ] && [ "$b102" = " FETCH=" ] && [ "$c102" = " FETCH=" ]; then ok "IT-102 the fetch lane sees the configured base: FETCH_BASE_URL is set from CORTEX_BASE_URL, empty when none is given, and an inherited FETCH_BASE_URL never gets through"; else bad "IT-102 FETCH_BASE_URL and CORTEX_BASE_URL" "with --base-url: '$a102' | none: '$b102' | inherited: '$c102'"; fi

# ===================================================================================================================================================================
# 6e increment 2: W1-68 and W1-77 (the ONE writer of install.sh wires the verified compose provider and the call sites of steps 1 and 2)
#  IT-103 every compose call goes through ONE helper that hands podman the verified provider (PODMAN_COMPOSE_PROVIDER, documented in podman 5's `podman compose --help`); no other
#         line of install.sh names `podman compose` as a command; every call the fake podman sees carries that provider.
#  IT-104 the provider is ONE absolute path to a regular executable that no one else can replace (no link, no directory on its way and not the file itself writable by group or
#         other): a checkout without the prerequisite file resolves podman-compose from PATH once; with it the provider is prereq_compose_path's single line and a failing or odd
#         answer stops the install (EX_PREREQ 3) before anything is made.
#  IT-105 <home>/compose-provider: ONE line, that path, 0644, written only when prereq_compose_path gave it, re-verified and kept as it is when right, a link or a non-file there stops it.
#  IT-106 the call sites: step 1 (machine_look), step 2 (prereq_run) in that order, before the preflight, each with the real exit table; an unknown status is mapped, not passed.
# ===================================================================================================================================================================
# r4: the trust anchor is three empty statements at the top of install.sh (CORTEX_ANCHOR_PRINCIPAL/KEY/FPR; the tree keeps them EMPTY until the CTO's key, C-57; the release build's join step fills them).
# A RELEASE run with an empty anchor refuses before anything is fetched, so a cell that installs a release runs a COPY with TEST values filled in (never a real key). fill_anchor SRC DST PRINCIPAL KEY FPR
# replaces each empty statement by its single-quoted value ("\n" in a value is a newline: awk -v does that); a statement the file does not have (the tree before r4) is simply not found.
A_P='cortex-release'
A_K="AAAAC3NzaC1lZDI1NTE5AAAAI$(printf 'B%.0s' $(seq 43))"
A_F="SHA256:$(printf 'C%.0s' $(seq 43))"
fill_anchor() {
  awk -v q="'" -v p="$3" -v k="$4" -v f="$5" '
    $0 == "CORTEX_ANCHOR_PRINCIPAL=" q q { print "CORTEX_ANCHOR_PRINCIPAL=" q p q; next }
    $0 == "CORTEX_ANCHOR_KEY=" q q { print "CORTEX_ANCHOR_KEY=" q k q; next }
    $0 == "CORTEX_ANCHOR_FPR=" q q { print "CORTEX_ANCHOR_FPR=" q f q; next }
    { print }' "$1" > "$2"
}
ifd() { # ifd <fake prerequisite file text> <install.sh arguments>: like ifc, but a COPY of the whole deploy directory (a full install needs the compose file and the config example beside the script); install.sh in the copy has the TEST anchor filled in, install.empty.sh is the copy as it is
  rm -rf "$T/d83"; copy_deploy "$(dirname "$install")" "$T/d83"   # (see THE LIMIT at the top)
  cp "$T/d83/install.sh" "$T/d83/install.empty.sh"; fill_anchor "$T/d83/install.empty.sh" "$T/d83/install.sh" "$A_P" "$A_K" "$A_F"
  [ -z "$1" ] || printf '%s\n' "$1" > "$T/d83/install-prereq.sh"
  shift; fresh; sh "$T/d83/install.sh" "$@" > "$T/out" 2>&1 && rc=0 || rc=$?
}
chk104() { # chk104 <what> [text of the problem: line]: after a run that set rc and $T/out: exit 3, nothing started or made, exactly one problem: and one fix: line
  c104p=$(grep -c '^problem: ' "$T/out" || true); c104f=$(grep -c '^fix: ' "$T/out" || true)
  { [ "$rc" = 3 ] && [ "$c104p" = 1 ] && [ "$c104f" = 1 ] && untouched && { [ -z "${2:-}" ] || grep -q "^problem: .*$2" "$T/out"; }; } || b104="$b104 [$1 rc=$rc problem=$c104p fix=$c104f untouched=$(untouched && echo y || echo n): $(tail -1 "$T/out" | cut -c1-70)]"
}
provs() { awk -F' [|] ' '{ print $NF }' "$T/provider.log" | sort -u; }   # the providers the fake podman was handed, one per distinct value
# IT-103 the one helper
n103=$(grep -c '^[^#"'"'"']*podman compose' "$install" || true)
fresh; run && rc=0 || rc=$?
c103=$(wc -l < "$T/provider.log" | tr -d ' '); p103=$(provs)
if fresh && mkdir -p "$T/h" && mkrel; then
run && rc2=0 || rc2=$?
c103b=$(wc -l < "$T/provider.log" | tr -d ' '); p103b=$(provs)
if [ "$n103" = 1 ] && [ "$rc" = 0 ] && [ "$c103" = 4 ] && [ "$p103" = "$PROV" ] && [ "$rc2" = 0 ] && [ "$c103b" = 3 ] && [ "$p103b" = "$PROV" ]; then ok "IT-103 one helper names compose; the 4 calls of a checkout and the 3 of a release all carry the verified provider path"; else bad "IT-103 one compose helper" "lines naming podman compose=$n103 (want 1); checkout rc=$rc calls=$c103 (want 4) providers='$p103'; release rc=$rc2 calls=$c103b (want 3) providers='$p103b'"; fi
else bad "IT-103 fixture setup" "could not prepare dependent fixture"; fi
# IT-104 the path of the provider (a checkout: found on PATH)
b104=""
nop=$(printf '%s' "$PATH" | sed "s|$P:||")
# a PATH that holds the fake podman and the basic tools only (a real podman-compose may be installed on the machine that runs this suite, further along PATH)
mkdir -p "$P/minbin"; for t in sh sed dirname id stat cat grep awk tr head od date sleep mkdir chmod cp mv rm wc ls; do tp=$(command -v "$t") && ln -sf "$tp" "$P/minbin/$t" || :; done
fresh; PATH="$T/bin:$P/minbin" run && rc=0 || rc=$?; chk104 "no podman-compose on PATH"
[ "$(tail -1 "$T/out")" = "$MSG_A" ] || b104="$b104 [no podman-compose on PATH: the last line is not the fixed sentence: $(tail -1 "$T/out" | cut -c1-70)]"
mkdir -p "$P/b" "$P/bt"; cp "$PROV" "$P/bt/podman-compose"; chmod 0775 "$P/bt"; ln -s "$P/bt/podman-compose" "$P/b/podman-compose"   # a safe directory holding a link to a file in an unsafe one
fresh; PATH="$P/b:$nop" run && rc=0 || rc=$?; chk104 "the provider is a symbolic link (its own directory is safe, its target's is not)"
mkdir -p "$P/d"; cp "$PROV" "$P/d/podman-compose"; chmod 0775 "$P/d"
fresh; PATH="$P/d:$nop" run && rc=0 || rc=$?; chk104 "its directory is writable by the group"
chmod 0757 "$P/d"
fresh; PATH="$P/d:$nop" run && rc=0 || rc=$?; chk104 "its directory is writable by others"
mkdir -p "$P/e/f"; cp "$PROV" "$P/e/f/podman-compose"; chmod 0775 "$P/e"
fresh; PATH="$P/e/f:$nop" run && rc=0 || rc=$?; chk104 "a directory further up is writable by the group"
mkdir -p "$P/g"; cp "$PROV" "$P/g/podman-compose"; chmod 0775 "$P/g/podman-compose"; chmod 0755 "$P/g"
fresh; PATH="$P/g:$nop" run && rc=0 || rc=$?; chk104 "the file itself is writable by the group"
rm -rf "$P/b" "$P/bt" "$P/d" "$P/e" "$P/g"
# with the prerequisite file: prereq_compose_path is the answer, and it fails closed
mkdir -p "$P/prereq"; cp "$PROV" "$P/prereq/podman-compose"; chmod 0755 "$P/prereq" "$P/prereq/podman-compose"
PROV2="$P/prereq/podman-compose"; export PROV2
pl='machine_look() { return 0; }
prereq_run() { return 0; }'
ifd "$pl
prereq_compose_path() { return 1; }" --project p --lead lead --home "$T/h"; chk104 "prereq_compose_path fails (fail closed)" "did not verify a compose provider"
# a relative path that EXISTS in the directory the installer runs from (it must still be refused: only an absolute path is accepted; a bare name would fail the later directory step for another reason, mutant IM-243)
cd "$P/prereq"
ifd "$pl
prereq_compose_path() { printf '%s\n' ./podman-compose; }" --project p --lead lead --home "$T/h"; chk104 "prereq_compose_path gives a relative path (with a directory part) that exists here"
cd "$OLDPWD"
ifd "$pl
prereq_compose_path() { :; }" --project p --lead lead --home "$T/h"; chk104 "prereq_compose_path gives nothing" "did not verify a compose provider"
ifd "$pl
prereq_compose_path() { printf '%s\n%s\n' \"\$PROV2\" /bin/sh; }" --project p --lead lead --home "$T/h"; chk104 "prereq_compose_path gives two lines"
ifd "$pl
prereq_compose_path() { printf '%s\n' \"\$PROV2/../podman-compose\"; }" --project p --lead lead --home "$T/h"; chk104 "prereq_compose_path gives a path with .."
mkdir -p "$P/sp ace"; cp "$PROV" "$P/sp ace/podman-compose"; chmod 0755 "$P/sp ace"; PROV_SP="$P/sp ace/podman-compose"; export PROV_SP
ifd "$pl
prereq_compose_path() { printf '%s\n' \"\$PROV_SP\"; }" --project p --lead lead --home "$T/h"; chk104 "prereq_compose_path gives a path with a space"
rm -rf "$P/sp ace"
mkdir -p "$P/h2"; cp "$PROV" "$P/h2/podman-compose"; chmod 0644 "$P/h2/podman-compose"; chmod 0755 "$P/h2"; PROV_H2="$P/h2/podman-compose"; export PROV_H2
ifd "$pl
prereq_compose_path() { printf '%s\n' \"\$PROV_H2\"; }" --project p --lead lead --home "$T/h"; chk104 "the file is not executable"
rm -rf "$P/h2"
if [ -z "$b104" ]; then ok "IT-104 a provider that is missing, a link, not executable, replaceable by group or others (itself or any directory above) or not an absolute single line from prereq_compose_path stops the install: exit 3, one problem: and one fix: line, nothing made"; else bad "IT-104 the provider path" "$b104"; fi
# the answer of prereq_compose_path is used, not PATH's podman-compose: a HOSTILE stand-in sits FIRST in PATH (cox 89ae82e7, hale's 6b read) and is never named by any compose call
mkdir -p "$P/hostile"; printf '#!/bin/sh\n# hostile stand-in\nexit 0\n' > "$P/hostile/podman-compose"; chmod 0755 "$P/hostile" "$P/hostile/podman-compose"
PATH="$P/hostile:$PATH"; export PATH
ifd "$pl
prereq_compose_path() { printf '%s\n' \"\$PROV2\"; }" --project p --lead lead --home "$T/h"
hostile_named=$(grep -c "$P/hostile" "$T/provider.log" || true)
if [ "$rc" = 0 ] && [ "$(provs)" = "$PROV2" ] && [ "$(wc -l < "$T/provider.log" | tr -d ' ')" = 4 ] && [ "$hostile_named" = 0 ]; then ok "IT-104b prereq_compose_path's path is the provider of every compose call: a hostile podman-compose first in PATH is never named"; else bad "IT-104b the verified path reaches compose" "rc=$rc providers='$(provs)' calls=$(wc -l < "$T/provider.log" | tr -d ' ') hostile named=$hostile_named"; fi
# and with the prerequisite file joined, a failing prereq_compose_path never falls back to PATH, hostile stand-in or not
ifd "$pl
prereq_compose_path() { return 1; }" --project p --lead lead --home "$T/h"
{ [ "$rc" = 3 ] && [ ! -s "$T/provider.log" ] && untouched; } && ok "IT-104c a release install whose prerequisite step verified no provider stops (EX_PREREQ) and never looks on PATH: no compose call at all, a hostile podman-compose first in PATH or not" || bad "IT-104c no PATH fallback" "rc=$rc calls=$(wc -l < "$T/provider.log" | tr -d ' ')"
rm -rf "$P/hostile"
PATH=$(printf '%s' "$PATH" | sed "s|$P/hostile:||"); export PATH
# IT-105 <home>/compose-provider
cpf="$T/h/compose-provider"
pgood="$pl
prereq_compose_path() { printf '%s\n' \"\$PROV2\"; }"
ifd "$pgood" --project p --lead lead --home "$T/h"
b105=""
{ [ "$rc" = 0 ] && [ -f "$cpf" ] && [ ! -L "$cpf" ] && [ "$(stat -c %a "$cpf")" = 644 ] && [ "$(stat -c %u "$cpf")" = "$(id -u)" ] && [ "$(cat "$cpf")" = "$PROV2" ] && [ "$(wc -l < "$cpf" | tr -d ' ')" = 1 ] && [ "$(wc -c < "$cpf" | tr -d ' ')" = "$(( ${#PROV2} + 1 ))" ]; } || b105="$b105 [first run rc=$rc: mode=$(stat -c %a "$cpf" 2>/dev/null || :) content='$(cat "$cpf" 2>/dev/null || :)']"
[ -z "$(ls -A "$T/h" | grep -F 'compose-provider.' || true)" ] || b105="$b105 [a temporary file was left: $(ls -A "$T/h" | grep -F 'compose-provider.')]"
if touch -d '2020-01-01 00:00:00' "$cpf" && m105a=$(stat -c %Y "$cpf"); then
# a second run that is NOT the finished one (the health route cannot be asked: curl fails), so the whole sequence runs and reaches the record (a finished run exits before it: mutant IM-255)
FAKE_HEALTH_DOC_FAIL=1 sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?
{ [ "$rc" = 0 ] && [ "$(stat -c %Y "$cpf")" = "$m105a" ] && [ "$(cat "$cpf")" = "$PROV2" ]; } || b105="$b105 [a second run rewrote a right file: rc=$rc]"
else b105="$b105 [fixture setup: compose-provider]"; fi
if printf '%s\n' /old/podman-compose > "$cpf" && chmod 0644 "$cpf"; then
sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?
{ [ "$rc" = 0 ] && [ "$(cat "$cpf")" = "$PROV2" ] && [ "$(stat -c %a "$cpf")" = 644 ]; } || b105="$b105 [a stale record (mode 0644) was not corrected: rc=$rc content='$(cat "$cpf" || :)' mode=$(stat -c %a "$cpf" || :)]"
else b105="$b105 [fixture setup: compose-provider]"; fi
if chmod 0600 "$cpf"; then
sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?
{ [ "$rc" = 0 ] && [ "$(cat "$cpf")" = "$PROV2" ] && [ "$(stat -c %a "$cpf")" = 644 ]; } || b105="$b105 [a right record with the wrong mode was not corrected: rc=$rc mode=$(stat -c %a "$cpf" || :)]"
else b105="$b105 [fixture setup: compose-provider]"; fi
if mkdir -p "$T/h" && rm -f "$cpf" && ln -s "$T/linktarget" "$cpf" && : > "$T/linktarget"; then
sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?
{ [ "$rc" = 6 ] && [ -L "$cpf" ] && [ "$(readlink "$cpf")" = "$T/linktarget" ] && [ ! -s "$T/linktarget" ] && [ "$(grep -c '^problem: ' "$T/out")" = 1 ] && [ "$(grep -c '^fix: ' "$T/out")" = 1 ]; } || b105="$b105 [a link at the record: rc=$rc link=$(readlink "$cpf" 2>/dev/null || :) target-bytes=$(wc -c < "$T/linktarget")]"
else b105="$b105 [fixture setup: compose-provider]"; fi
if rm -f "$cpf" && mkdir "$cpf"; then
sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?
{ [ "$rc" = 6 ] && [ -d "$cpf" ] && [ "$(grep -c '^problem: ' "$T/out")" = 1 ]; } || b105="$b105 [a directory at the record: rc=$rc]"
else b105="$b105 [fixture setup: compose-provider]"; fi
ifd "$pgood" --project p --lead lead --home "$T/h" --no-start
{ [ "$rc" = 0 ] && [ "$(cat "$cpf" 2>/dev/null || :)" = "$PROV2" ]; } || b105="$b105 [--no-start did not record the provider: rc=$rc]"
fresh; run && rc=0 || rc=$?
{ [ "$rc" = 0 ] && [ ! -e "$T/h/compose-provider" ]; } || b105="$b105 [a checkout without the prerequisite file recorded a provider the 6b step never verified: rc=$rc]"
if [ -z "$b105" ]; then ok "IT-105 <home>/compose-provider is one line, the verified path, 0644, written by rename, kept when right, corrected when stale, never written through a link or over a directory, and only when prereq_compose_path gave the path"; else bad "IT-105 compose-provider record" "$b105"; fi
# IT-106 the call sites of steps 1 and 2: order, the real table, unknown statuses mapped, messages of the lane reach the person, reissue skips both
b106=""
chk106() { # chk106 <what> <exit> <log lines wanted, space-separated> [problem lines wanted]
  g106=$(tr '\n' ' ' < "$T/log" | sed 's/ $//')
  { [ "$rc" = "$2" ] && [ "$g106" = "$3" ] && [ ! -e "$T/h" ] && [ "$(grep -c '^problem: ' "$T/out" || true)" = "${4:-0}" ]; } || b106="$b106 [$1 rc=$rc (want $2) log='$g106' (want '$3') home=$([ -e "$T/h" ] && echo made || echo none) problems=$(grep -c '^problem: ' "$T/out" || true)]"
}
lane() { printf 'machine_look() { echo ML >> "$FAKE_LOG"; %s }\nprereq_run() { echo PR >> "$FAKE_LOG"; %s }\nprereq_compose_path() { printf "%%s\\n" "$PROV2"; }' "$1" "$2"; }
ifd "$(lane 'return 0;' 'return 0;')" --project p --lead lead --home "$T/h" --no-start
{ [ "$rc" = 0 ] && [ "$(sed -n 1,3p "$T/log" | tr '\n' ' ')" = "ML PR --version " ]; } || b106="$b106 [order: rc=$rc first lines '$(sed -n 1,3p "$T/log" | tr '\n' ' ')' (want 'ML PR --version ')]"
ifd "$(lane 'echo "problem: ML-OWN-LINE" >&2; echo "fix: ML-OWN-FIX" >&2; return 2;' 'return 0;')" --project p --lead lead --home "$T/h"
chk106 "machine_look blocks (2)" 2 "ML" 1
grep -qx 'problem: ML-OWN-LINE' "$T/out" || b106="$b106 [the lane's own problem line did not reach the person]"
ifd "$(lane 'return 0;' 'return 3;')" --project p --lead lead --home "$T/h"; chk106 "prereq_run fails (3)" 3 "ML PR"
ifd "$(lane 'return 0;' 'return 2;')" --project p --lead lead --home "$T/h"; chk106 "prereq_run declined (2)" 2 "ML PR"
ifd "$(lane 'return 3;' 'return 0;')" --project p --lead lead --home "$T/h"; chk106 "machine_look gives EX_PREREQ (3), marv's r4 for a failed diagnostic output: passes through, no cause of ours" 3 "ML" 0
ifd "$(lane 'return 9;' 'return 0;')" --project p --lead lead --home "$T/h"; chk106 "machine_look gives an unknown status (9): mapped to 2, one problem and fix" 2 "ML" 1
ifd "$(lane 'return 0;' 'return 9;')" --project p --lead lead --home "$T/h"; chk106 "prereq_run gives an unknown status (9): mapped to 3, one problem and fix" 3 "ML PR" 1
ifd "$(lane 'return 0;' 'return 0;')" --project p --lead lead --home "$T/h"
[ "$rc" = 0 ] || b106="$b106 [a full install through both steps failed: rc=$rc: $(tail -1 "$T/out" | cut -c1-70)]"
printf '%s\n' "$(lane 'return 0;' 'return 3;')" > "$T/d83/install-prereq.sh"; : > "$T/log"
sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" --reissue-token > "$T/out" 2>&1 && rc=0 || rc=$?
{ [ "$rc" = 0 ] && ! grep -qxE 'ML|PR' "$T/log"; } || b106="$b106 [--reissue-token called a prerequisite step or failed: rc=$rc log='$(tr '\n' ' ' < "$T/log")']"
if [ -z "$b106" ]; then ok "IT-106 step 1 (machine_look) then step 2 (prereq_run) run before the preflight; 2 and 3 pass through, an unknown status becomes the step's own code with one problem: and one fix: line, the lane's own lines reach the person, nothing is made; --reissue-token calls neither"; else bad "IT-106 the call sites of steps 1 and 2" "$b106"; fi

# ===================================================================================================================================================================
# 6e increment 3a: the home, the stage directory (INSTALL-LANES-INTERFACE r1 section 2, W1-81 (c)), the call chain of steps 3 and 4, the closing lines (step 10)
#  IT-107 the home is a real directory of the user's: a link is refused (nothing changed), a file where it should be is refused.
#  IT-108 steps 3 and 4: with a release to install (--release V) and not yet installed, <home>/.stage-V is cleared by the stage rule (a real 0700 directory only; a link, a directory of
#         another mode or a file is KEPT and stops the install, EX_FETCH 5), then rel_fetch V STAGE, rel_trust STAGE, rel_unpack STAGE HOME V, rel_load HOME V, each with its own code. The
#         argument lists are PROVISIONAL (marv's lane has only rel_fetch); trust is fail closed: a fetch without rel_trust stops (EX_TRUST 4) and nothing is unpacked.
#  IT-109 step 10: say_done VERSION ENDPOINT TOKENFILE when the prerequisite file has it; the old closing lines otherwise.
# ===================================================================================================================================================================
b107=""
if fresh && rm -rf "$T/realhome" "$T/linkhome" && mkdir -p "$T/realhome" && chmod 0755 "$T/realhome" && ln -s "$T/realhome" "$T/linkhome"; then
sh "$install" --project p --lead lead --home "$T/linkhome" > "$T/out" 2>&1 && rc=0 || rc=$?
{ [ "$rc" = 2 ] && [ "$(stat -c %a "$T/realhome")" = 755 ] && [ -z "$(ls -A "$T/realhome")" ] && [ "$(grep -c '^problem: ' "$T/out" || true)" = 1 ] && [ "$(grep -c '^fix: ' "$T/out" || true)" = 1 ] && [ "$(grep -vcxE -e '--version|compose version' "$T/log" || true)" = 0 ]; } || b107="$b107 [a symbolic link as the home: rc=$rc mode=$(stat -c %a "$T/realhome") files='$(ls -A "$T/realhome" | tr '\n' ' ')' log-lines=$(grep -vcxE -e '--version|compose version' "$T/log" || true)]"
else b107="$b107 [home-link fixture setup]"; fi
rm -rf "$T/realhome" "$T/linkhome"
fresh; : > "$T/filehome"
sh "$install" --project p --lead lead --home "$T/filehome" > "$T/out" 2>&1 && rc=0 || rc=$?
{ [ "$rc" = 6 ] && [ -f "$T/filehome" ] && [ "$(grep -c '^problem: ' "$T/out" || true)" = 1 ] && [ "$(grep -c '^fix: ' "$T/out" || true)" = 1 ]; } || b107="$b107 [a file as the home: rc=$rc problems=$(grep -c '^problem: ' "$T/out" || true)]"
rm -f "$T/filehome"
if [ -z "$b107" ]; then ok "IT-107 a home that is a symbolic link is refused with nothing changed (exit 2), a file where the home should be is refused (exit 6), each with one problem: and one fix: line"; else bad "IT-107 the home" "$b107"; fi
fp() { # fp <full|files>. full: one line per file, link and directory under the home, the unit directory and the fake's state: path, mode, owner, size, modification time, checksum
       # (the same machine before and after a run). files: only what an install makes and without times, secret and token bytes (two different runs are compared).
  ( cd "$T" && for d in h home/.config state; do [ -e "$d" ] || continue; find "$d" \( -type f -o -type l -o -type d \) | LC_ALL=C sort | while IFS= read -r f; do
      case $1:$f in files:state/*) case $f in state|state/secret-*|state/enabled|state/projects) ;; *) continue ;; esac ;; esac
      if [ -f "$f" ] && [ ! -L "$f" ]; then
        if [ "$1" = full ]; then printf '%s %s %s\n' "$f" "$(stat -c '%a %u %s %Y' "$f")" "$(cksum < "$f")"
        else case $f in state/secret-*|h/lead.token) printf '%s %s\n' "$f" "$(stat -c '%a %u %s' "$f")" ;; *) printf '%s %s %s\n' "$f" "$(stat -c '%a %u %s' "$f")" "$(cksum < "$f")" ;; esac; fi
      else
        if [ "$1" = full ]; then printf '%s %s\n' "$f" "$(stat -c '%a %u %Y' "$f")"; else printf '%s %s\n' "$f" "$(stat -c '%a %u' "$f")"; fi
      fi
    done; done )
}
# IT-108 THE ORDER OF W1-85 AND THE STAGE OF W1-86: (1) look, (2) the trust tools, (3) fetch and trust in a PRIVATE stage made by mktemp -d (.cortex-install-stage.XXXXXXXX, 0700) in the
# nearest EXISTING ancestor of the home, BEFORE any prerequisite and BEFORE the home exists, (4) prerequisites, (5) unpack and load from the verified stage into the home. A failed fetch or
# trust changes nothing: no package manager, no sudo, no prerequisite, no home, no unpack, no stage left. The stage goes on every exit (success, failure, INT, TERM); a leftover of a lost run is
# NEVER trusted (a stale real 0700 one is removed by name, the rest is kept and named). Marv's contract page (INSTALL-6C-SHELL-CONTRACTS-marv-r0.md, 0d48d909): rel_fetch REL STAGE, rel_trust REL STAGE,
# rel_unpack REL STAGE HOME, rel_load REL STAGE HOME. A second release is refused (hale N-2), so is another port.
fresh
if mkdir -p "$T/h" && mkrel && rm -rf "$T/relsrc" && mv "$T/h/current" "$T/relsrc" && rm -rf "$T/h"; then :
else bad "IT-108 shared release fixture setup" "could not prepare release source"; fi
REL_SRC="$T/relsrc"; export REL_SRC
T_HOME="$T/h"; export T_HOME
lane3='machine_look() { echo ML >> "$FAKE_LOG"; return 0; }
prereq_run() { echo "PR home=$([ -e "$T_HOME" ] && echo yes || echo no)" >> "$FAKE_LOG"; return "${FAKE_PR_RC:-0}"; }
prereq_compose_path() { printf "%s\n" "$PROV2"; }
rel_fetch() { echo "FETCH $1 $2 home=$([ -e "$T_HOME" ] && echo yes || echo no)" >> "$FAKE_LOG"; echo "STAGE $(ls -A "$2" | wc -l | tr -d " ")|$(stat -c %a "$2")|${2%/*}|${2##*/}" >> "$FAKE_LOG"; echo staged > "$2/f"; if [ -n "${FAKE_FETCH_WAIT:-}" ]; then fw=0; while [ ! -e "$FAKE_FETCH_WAIT" ] && [ "$fw" -lt 300 ]; do sleep 0.1; fw=$((fw + 1)); done; fi; return "${FAKE_FETCH_RC:-0}"; }
rel_trust() { echo "TRUST $1 $2 home=$([ -e "$T_HOME" ] && echo yes || echo no)" >> "$FAKE_LOG"; echo "TRUSTARGS $#|${3-}|${4-}|${5-}" >> "$FAKE_LOG"; echo "STAGEAT $(ls -A "$2" | wc -l | tr -d " ")" >> "$FAKE_LOG"; if [ -n "${3-}" ]; then if [ -f "$3" ]; then echo "KEYFILE $(wc -l < "$3" | tr -d " ")|$(stat -c %a "$3")|$(stat -c %a "${3%/*}")|$(cat "$3")" >> "$FAKE_LOG"; else echo "KEYFILE missing" >> "$FAKE_LOG"; fi; fi; return "${FAKE_TRUST_RC:-0}"; }
rel_unpack() { echo "UNPACK $1 $2 $3" >> "$FAKE_LOG"; [ "${FAKE_UNPACK_RC:-0}" = 0 ] || return "$FAKE_UNPACK_RC"; { [ -d "$3" ] && [ "$(stat -c %a "$3")" = 700 ]; } || return "$EX_FETCH"; mkdir -p "$3/releases" && cp -R "$REL_SRC" "$3/releases/$1"; }   # the HOME must exist, 0700, when the unpack runs (install.sh makes it at step 5; mutant IM-334)
rel_load() { echo "LOAD $1 $2 $3" >> "$FAKE_LOG"; [ "${FAKE_LOAD_RC:-0}" = 0 ] || return "$FAKE_LOAD_RC"; [ -z "${FAKE_LOAD_NOLINK:-}" ] || return 0; ln -sfn "$3/releases/$1" "$3/current"; }
verify_release_dir() { echo "VERIFY $1" >> "$FAKE_LOG"; return "${FAKE_VERIFY_RC:-0}"; }'
b108=""
callsof() { grep -E '^(ML|FETCH|TRUST|PR|UNPACK|LOAD)( |$)' "$T/log" | cut -d' ' -f1 | tr '\n' ' '; }
stages108() { find "$T" -maxdepth 1 -name '.cortex-install-stage.*' 2>/dev/null | head -3 | tr '\n' ' '; }   # stage directories beside the home ($T is the nearest existing ancestor of $T/h)
inst3() { sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" --release 0.2.1-rc.2 --from "$T/from" "$@" > "$T/out" 2>&1 && rc=0 || rc=$?; }   # a run of the copy that ifd made
untouched108() { [ ! -e "$T/h" ] && [ -z "$(stages108)" ] && ! grep -qE '^(dnf|sudo) ' "$T/log"; }   # no home, no stage, no package manager, no sudo
ifd "$lane3" --project p --lead lead --home "$T/h" --release 0.2.1-rc.2 --from "$T/from"
STG=$(sed -n 's/^FETCH [^ ]* \([^ ]*\) home=.*/\1/p' "$T/log")
{ [ "$rc" = 0 ] && [ "$(callsof)" = "ML FETCH TRUST PR UNPACK LOAD " ] && grep -qxF "FETCH 0.2.1-rc.2 $STG home=no" "$T/log" && grep -qxF "TRUST 0.2.1-rc.2 $STG home=no" "$T/log" && grep -qxF 'PR home=no' "$T/log" && grep -qxF "UNPACK 0.2.1-rc.2 $STG $T/h" "$T/log" && grep -qxF "LOAD 0.2.1-rc.2 $STG $T/h" "$T/log" && grep -qE "^STAGE 0\|700\|$T\|\.cortex-install-stage\.[A-Za-z0-9]{8}\$" "$T/log" && [ -f "$T/h/current/stack.env" ] && [ -z "$(stages108)" ]; } || b108="$b108 [the order and the arguments: rc=$rc calls='$(callsof)' stage='$STG' $(grep -E '^(STAGE|PR) ' "$T/log" | tr '\n' ' ' | cut -c1-120) $(tail -1 "$T/out" | cut -c1-60)]"
# a second run: the install is finished, nothing is fetched again and no step runs
: > "$T/log"; inst3
{ [ "$rc" = 0 ] && [ -z "$(callsof)" ] && [ "$(grep -c 'already installed' "$T/out" || true)" = 1 ]; } || b108="$b108 [a second run did something: rc=$rc calls='$(callsof)']"
# the release is installed but the install is not finished (the token is gone): step 3 is not repeated, the prerequisites are looked at again, the token is minted
rm -f "$T/h/lead.token"; : > "$T/log"; inst3
{ [ "$rc" = 0 ] && [ "$(callsof)" = "ML PR " ] && [ -s "$T/h/lead.token" ]; } || b108="$b108 [an installed release was fetched again or the token not minted: rc=$rc calls='$(callsof)']"
# a failed fetch or trust: nothing changed (no home, no stage, no package manager, no sudo, no prerequisite step, nothing unpacked)
chain108() { # chain108 <what> <env assignment> <want rc> <want calls>
  fresh; env "$2" sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" --release 0.2.1-rc.2 --from "$T/from" > "$T/out" 2>&1 && rc=0 || rc=$?
  { [ "$rc" = "$3" ] && [ "$(callsof)" = "$4" ]; } || b108="$b108 [$1: rc=$rc (want $3) calls='$(callsof)' (want '$4')]"
}
chain108 "a wrong checksum or signature (trust fails)" FAKE_TRUST_RC=4 4 "ML FETCH TRUST "
untouched108 || b108="$b108 [a failed trust left a home, a stage, a package manager call or a sudo call: home=$([ -e "$T/h" ] && echo yes || echo no) stages='$(stages108)']"
chain108 "the download fails" FAKE_FETCH_RC=5 5 "ML FETCH "
untouched108 || b108="$b108 [a failed fetch left a home, a stage, a package manager call or a sudo call]"
chain108 "trust gives a status outside the table" FAKE_TRUST_RC=9 4 "ML FETCH TRUST "
chain108 "fetch gives a status outside the table" FAKE_FETCH_RC=9 5 "ML FETCH "
chain108 "trust gives a fetch status (5)" FAKE_TRUST_RC=5 5 "ML FETCH TRUST "
chain108 "the prerequisites are declined (2) after trust" FAKE_PR_RC=2 2 "ML FETCH TRUST PR "
[ ! -e "$T/h" ] && [ -z "$(stages108)" ] || b108="$b108 [declined prerequisites: a home or a stage was left]"
chain108 "the prerequisites fail (3)" FAKE_PR_RC=3 3 "ML FETCH TRUST PR "
chain108 "the unpack fails" FAKE_UNPACK_RC=5 5 "ML FETCH TRUST PR UNPACK "
[ -z "$(stages108)" ] || b108="$b108 [a failed unpack left the stage]"
chain108 "the unpack gives a status outside the table" FAKE_UNPACK_RC=9 5 "ML FETCH TRUST PR UNPACK "
chain108 "the load fails" FAKE_LOAD_RC=5 5 "ML FETCH TRUST PR UNPACK LOAD "
[ ! -e "$T/h/current" ] && [ -z "$(stages108)" ] || b108="$b108 [a failed load made current or left the stage]"
chain108 "the load gives a status outside the table" FAKE_LOAD_RC=9 5 "ML FETCH TRUST PR UNPACK LOAD "
# every hook said yes but NOTHING was laid down (no <home>/current/stack.env): stop (EX_FETCH, release_incomplete), never fall back to the checkout and start it (mutant IM-273)
chain108 "the load says yes and lays nothing down" FAKE_LOAD_NOLINK=1 5 "ML FETCH TRUST PR UNPACK LOAD "
{ grep -q '^problem: .*release is missing a file\|^problem: .*missing a file the install needs' "$T/out" && ! grep -q '^compose -f ' "$T/log" && [ ! -e "$T/h/dumps" ]; } || b108="$b108 [nothing laid down: no stop with a cause, or something was started: $(grep '^problem: ' "$T/out" | cut -c1-70)]"
# fail closed: no way to verify (no rel_trust) stops BEFORE anything is fetched, EX_TRUST
ifd "machine_look() { echo ML >> \"\$FAKE_LOG\"; return 0; }
prereq_run() { echo PR >> \"\$FAKE_LOG\"; return 0; }
prereq_compose_path() { printf '%s\n' \"\$PROV2\"; }
rel_fetch() { echo \"FETCH \$1 \$2\" >> \"\$FAKE_LOG\"; return 0; }
rel_unpack() { echo \"UNPACK \$1\" >> \"\$FAKE_LOG\"; return 0; }
rel_load() { echo \"LOAD \$1\" >> \"\$FAKE_LOG\"; return 0; }" --project p --lead lead --home "$T/h" --release 0.2.1-rc.2 --from "$T/from"
{ [ "$rc" = 4 ] && [ "$(callsof)" = "ML " ] && untouched108 && [ "$(grep -c '^problem: ' "$T/out" || true)" = 1 ] && grep -q '^problem: .*no way to verify' "$T/out"; } || b108="$b108 [no rel_trust: rc=$rc calls='$(callsof)' $(grep '^problem: ' "$T/out" | cut -c1-60)]"
# the trust tools, read-only, at step 2: a missing one stops with EX_MACHINE before anything is fetched
mkdir -p "$P/minbin"; for t in sh sed dirname id stat cat grep awk tr head od date sleep mkdir chmod cp mv rm wc ls mktemp find sort uniq ln cut touch kill; do tp=$(command -v "$t") && ln -sf "$tp" "$P/minbin/$t" || :; done
for t in sha256sum tar gzip ssh-keygen curl; do mkdir -p "$P/tt-$t"; for u in sha256sum tar gzip ssh-keygen curl; do [ "$u" = "$t" ] || { printf '#!/bin/sh\nexit 0\n' > "$P/tt-$t/$u"; chmod +x "$P/tt-$t/$u"; }; done; done
ifd "$lane3" --check
for t in sha256sum tar gzip ssh-keygen curl; do
  fresh; extra="--from $T/from"; [ "$t" != curl ] || extra="--base-url https://releases.example.invalid/cortex"
  # shellcheck disable=SC2086
  PATH="$T/bin:$P/tt-$t:$P/minbin" sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" --release 0.2.1-rc.2 $extra > "$T/out" 2>&1 && rc=0 || rc=$?
  { [ "$rc" = 2 ] && [ "$(callsof)" = "ML " ] && untouched108 && [ "$(grep -c '^problem: ' "$T/out" || true)" = 1 ] && grep -q "^problem: .*$t" "$T/out" && [ "$(grep -c '^fix: ' "$T/out" || true)" = 1 ]; } || b108="$b108 [missing $t: rc=$rc calls='$(callsof)' $(grep '^problem: ' "$T/out" | cut -c1-70)]"
done
fresh; PATH="$T/bin:$P/tt-curl:$P/minbin" sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" --release 0.2.1-rc.2 --from "$T/from" > "$T/out" 2>&1 && rc=0 || rc=$?
{ [ "$rc" = 0 ] || [ "$(callsof)" != "" ]; } && [ "$(callsof)" = "ML FETCH TRUST PR UNPACK LOAD " ] || b108="$b108 [curl is not needed for an offline install (--from): rc=$rc calls='$(callsof)']"
# the stage's parent: the nearest existing ancestor must be a real directory of the user's that the user can write; nothing is created before trust
fresh; sh "$T/d83/install.sh" --project p --lead lead --home /nonexistent-cv2-test-parent/h --release 0.2.1-rc.2 --from "$T/from" > "$T/out" 2>&1 && rc=0 || rc=$?
{ [ "$rc" = 2 ] && [ "$(callsof)" = "ML " ] && [ ! -e /nonexistent-cv2-test-parent ] && [ "$(grep -c '^problem: ' "$T/out" || true)" = 1 ] && [ "$(grep -c '^fix: ' "$T/out" || true)" = 1 ]; } || b108="$b108 [no qualifying ancestor: rc=$rc calls='$(callsof)']"
fresh; mkdir -p "$T/realparent"; ln -s "$T/realparent" "$T/linkparent"
sh "$T/d83/install.sh" --project p --lead lead --home "$T/linkparent/h" --release 0.2.1-rc.2 --from "$T/from" > "$T/out" 2>&1 && rc=0 || rc=$?
{ [ "$rc" = 2 ] && [ "$(callsof)" = "ML " ] && [ -z "$(ls -A "$T/realparent")" ]; } || b108="$b108 [a symbolic link as the nearest ancestor: rc=$rc calls='$(callsof)' left='$(ls -A "$T/realparent" | tr '\n' ' ')']"
rm -rf "$T/realparent" "$T/linkparent"
# the stage goes on TERM and HUP too (the install is stopped while rel_fetch waits; the signal goes to that one process only). INT is the same trap, but a background job of a
# non-interactive shell ignores INT from its start (POSIX), so it cannot be sent this way: the INT line is read from the script instead.
# W1-303d: require the context-aware INT registration and its ordinary
# signal-to-exit branch. Actual TERM/HUP stage cleanup and PTY INT tests stay.
int_cleanup_trap_present() {
  grep -q "trap 'install_signal 130' INT" "$install" &&
  sed -n '/^install_signal() {/,/^}/p' "$install" | grep -qF 'exit "$1"'
}
int_cleanup_trap_present || b108="$b108 [no INT trap that leads to the cleanup]"
ignored108=""
for sig in TERM HUP; do
  fresh; rm -f "$T/go"
  FAKE_FETCH_WAIT="$T/go" sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" --release 0.2.1-rc.2 --from "$T/from" > "$T/out" 2>&1 &
  ipid=$!; w=0; while ! grep -q '^STAGE ' "$T/log" 2>/dev/null && [ "$w" -lt 100 ]; do sleep 0.1; w=$((w + 1)); done
  have108=$(stages108)
  kill -s "$sig" "$ipid" 2>/dev/null || :
  w=0; while kill -0 "$ipid" 2>/dev/null && [ "$w" -lt 50 ]; do sleep 0.1; w=$((w + 1)); done
  if kill -0 "$ipid" 2>/dev/null; then
    # still running: a signal that the environment ignores from the start (nohup, a background job: SigIgn) cannot be tested here; anything else is a failure. TERM is never ignored: it ends the
    # install either way (its trap removes the stage), so the cleanup is still checked below.
    ign=$(awk '/^SigIgn:/ { print $2 }' "/proc/$ipid/status" 2>/dev/null || :)
    if [ "$sig" = HUP ] && [ -n "$ign" ] && [ $(( 0x$ign & 1 )) -eq 1 ]; then ignored108="$ignored108 $sig"; else b108="$b108 [$sig: the install did not stop]"; fi
    kill -s TERM "$ipid" 2>/dev/null || :
    w=0; while kill -0 "$ipid" 2>/dev/null && [ "$w" -lt 50 ]; do sleep 0.1; w=$((w + 1)); done
    kill -0 "$ipid" 2>/dev/null && { kill -s KILL "$ipid" 2>/dev/null || :; }   # s5-never: ok the test kills ITS OWN installer process, stuck in the cell's fake fetch, by its exact pid: the hard kill is the fallback of a signal cell
  fi
  wait "$ipid" 2>/dev/null && rc=0 || rc=$?
  { [ -n "$have108" ] && [ -z "$(stages108)" ] && [ ! -e "$T/h" ] && [ "$(callsof)" = "ML FETCH " ]; } || b108="$b108 [$sig: stage before='$have108' after='$(stages108)' home=$([ -e "$T/h" ] && echo yes || echo no) calls='$(callsof)' rc=$rc]"
done
# a LEFTOVER stage of a lost run is never trusted: a stale real 0700 one is removed by its name and said; a fresh one (a live run may own it), another mode, a link are kept and named; none is the stage of this run
leftover108() { # leftover108 <what> <setup text run in $T> <expect removed 1|0>
  fresh
  if ! ( cd "$T" && eval "$2" ); then b108="$b108 [$1: fixture setup]"; return 0; fi
  inst3
  now108=$(stages108); stg108=$(sed -n 's/^FETCH [^ ]* \([^ ]*\) home=.*/\1/p' "$T/log")
  c108=""
  [ "$rc" = 0 ] || c108="$c108 rc=$rc: $(tail -1 "$T/out" | cut -c1-60)"
  grep -qxF 'STAGE 0|700|'"$T"'|'"${stg108##*/}" "$T/log" || c108="$c108 the stage of this run was not fresh, empty and 0700"
  case " $now108" in *" $T/.cortex-install-stage.leftover "*) [ "$3" = 0 ] || c108="$c108 the leftover was kept" ;; *) [ "$3" = 1 ] || c108="$c108 the leftover was removed" ;; esac
  [ "$stg108" != "$T/.cortex-install-stage.leftover" ] || c108="$c108 the leftover was USED as the stage"
  [ "$3" = 1 ] || [ "$(grep -c 'left alone' "$T/out" || true)" -ge 1 ] || c108="$c108 a kept leftover was not named"
  [ "$3" = 1 ] && { [ "$(grep -c 'removed the leftover' "$T/out" || true)" = 1 ] || c108="$c108 the removal was not said once"; }
  [ -z "$c108" ] || b108="$b108 [$1:$c108]"
  rm -rf "$T/.cortex-install-stage.leftover"
}
leftover108 "a stale real 0700 leftover" 'mkdir -m 0700 .cortex-install-stage.leftover && echo old > .cortex-install-stage.leftover/old && touch -d "3 hours ago" .cortex-install-stage.leftover/old && touch -d "3 hours ago" .cortex-install-stage.leftover' 1
leftover108 "a fresh leftover (a live run may own it)" 'mkdir -m 0700 .cortex-install-stage.leftover && echo old > .cortex-install-stage.leftover/old' 0
leftover108 "a leftover with another mode" 'mkdir -m 0755 .cortex-install-stage.leftover && echo old > .cortex-install-stage.leftover/old && touch -d "3 hours ago" .cortex-install-stage.leftover/old .cortex-install-stage.leftover' 0
leftover108 "a leftover that is a symbolic link" 'ln -s "$T/state" .cortex-install-stage.leftover' 0
leftover108 "a leftover that is a file" 'echo old > .cortex-install-stage.leftover && touch -d "3 hours ago" .cortex-install-stage.leftover' 0
# a different release than the installed one is refused (hale N-2), before anything is looked at; another port is refused when the stack is up; neither changes the machine
ifd "$lane3" --project p --lead lead --home "$T/h" --release 0.2.1-rc.2 --from "$T/from" --port 8777; fpa=$(fp full)
: > "$T/log"; sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" --release 0.3.0 --from "$T/from" --port 8777 > "$T/out" 2>&1 && rc=0 || rc=$?
{ [ "$rc" = 1 ] && [ -z "$(callsof)" ] && grep -q '^problem: .*different release' "$T/out" && [ "$(grep -c '^fix: ' "$T/out" || true)" = 1 ] && [ "$(fp full)" = "$fpa" ]; } || b108="$b108 [another release asked for: rc=$rc calls='$(callsof)' $(grep '^problem: ' "$T/out" | cut -c1-70)]"
: > "$T/log"; sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" --from "$T/from" --port 8778 > "$T/out" 2>&1 && rc=0 || rc=$?
{ [ "$rc" = 1 ] && grep -q '^problem: .*another port\|^problem: .*port' "$T/out" && [ "$(grep -c '^fix: ' "$T/out" || true)" = 1 ] && [ "$(fp full)" = "$fpa" ] && ! grep -q '^compose -f ' "$T/log"; } || b108="$b108 [another port asked for: rc=$rc $(grep '^problem: ' "$T/out" | cut -c1-70)]"
if [ -z "$b108" ]; then ok "IT-108 (W1-85, W1-86) look, trust tools, fetch and trust in a private stage beside the home BEFORE any prerequisite and before the home exists, prerequisites, unpack and load; a failed trust or fetch changes nothing; the stage goes on every exit; a leftover is never trusted; another release or port is refused${ignored108:+ [signals ignored on entry in this environment and so not tested:$ignored108]}"; else bad "IT-108 the order of W1-85 and the stage of W1-86" "$b108"; fi
# IT-167, IT-167b, IT-167c (cox ad9da414 and W1-232, kai d7f6c666) THE PROXY NAMES ARE TAKEN OUT OF THE RELEASE DOWNLOAD ONLY: rel_fetch runs without the eight proxy names (http_proxy, https_proxy, HTTP_PROXY, HTTPS_PROXY, all_proxy, ALL_PROXY, no_proxy, NO_PROXY) and EVERY LATER STEP still has them. The caller
# sets all eight (each with its own value); the lane's fakes (lane3 of IT-108, each one told to record) write the eight names they inherit at the start of rel_fetch, rel_trust, prereq_run, rel_unpack and rel_load. IT-167: rel_fetch sees all eight unset (the download uses no proxy: the real curl call is IT-166c). IT-167b:
# rel_trust, prereq_run, rel_unpack and rel_load see all eight, each with the caller's value (the scoping is real: the database image pull of compose.yml, which podman makes through the operator's proxy, is not touched; a mutant that moves the names into the whole-run list fails this cell). IT-167c: the --from route is
# unchanged: with the proxy names set the install finishes (exit 0, the same six calls in the same order, the release linked as current).
penv167='penv() { for pn in http_proxy https_proxy HTTP_PROXY HTTPS_PROXY all_proxy ALL_PROXY no_proxy NO_PROXY; do eval "pv=\${$pn-unset}"; echo "PENV $1 $pn=$pv" >> "$FAKE_LOG"; done; }'
lane167=$(printf '%s\n' "$lane3" | sed 's/^rel_fetch() { /rel_fetch() { penv fetch; /; s/^rel_trust() { /rel_trust() { penv trust; /; s/^prereq_run() { /prereq_run() { penv pr; /; s/^rel_unpack() { /rel_unpack() { penv unpack; /; s/^rel_load() { /rel_load() { penv load; /')
lane167="$penv167
$lane167"
a167=""; b167b=""; c167c=""
[ "$(printf '%s\n' "$lane167" | grep -c 'penv \(fetch\|trust\|pr\|unpack\|load\);')" = 5 ] || a167="$a167 [the five fakes were not all changed to record: the cell would prove nothing]"
ifd "$lane167" --check
fresh
env http_proxy=http://127.0.0.1:9/http_proxy https_proxy=http://127.0.0.1:9/https_proxy HTTP_PROXY=http://127.0.0.1:9/HTTP_PROXY HTTPS_PROXY=http://127.0.0.1:9/HTTPS_PROXY all_proxy=http://127.0.0.1:9/all_proxy ALL_PROXY=http://127.0.0.1:9/ALL_PROXY no_proxy=localhost,no_proxy NO_PROXY=localhost,NO_PROXY \
  sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" --release 0.2.1-rc.2 --from "$T/from" > "$T/out" 2>&1 && rc=0 || rc=$?
for pn167 in http_proxy https_proxy HTTP_PROXY HTTPS_PROXY all_proxy ALL_PROXY no_proxy NO_PROXY; do
  case $pn167 in no_proxy|NO_PROXY) want167="localhost,$pn167" ;; *) want167="http://127.0.0.1:9/$pn167" ;; esac
  grep -qxF "PENV fetch $pn167=unset" "$T/log" || a167="$a167 [rel_fetch still saw $pn167: $(grep "^PENV fetch $pn167=" "$T/log" | head -1 | cut -c1-70)]"
  for st167 in trust pr unpack load; do
    grep -qxF "PENV $st167 $pn167=$want167" "$T/log" || b167b="$b167b [$st167 did not see $pn167 with the caller's value: $(grep "^PENV $st167 $pn167=" "$T/log" | head -1 | cut -c1-70)]"
  done
done
{ [ "$rc" = 0 ] && [ "$(callsof)" = "ML FETCH TRUST PR UNPACK LOAD " ] && [ -f "$T/h/current/stack.env" ] && [ -z "$(stages108)" ]; } || c167c="$c167c [the --from route with the proxy names set: rc=$rc calls='$(callsof)' $(tail -1 "$T/out" | cut -c1-80)]"
if [ -z "$a167" ]; then ok "IT-167 (kai, W1-232) rel_fetch runs without the eight proxy names (the caller had set all eight): the release download uses no proxy"; else bad "IT-167 the proxy names in rel_fetch" "$a167"; fi
if [ -z "$b167b" ]; then ok "IT-167b (W1-232) the steps after the download (rel_trust, prereq_run, rel_unpack, rel_load) still see all eight proxy names with the caller's values: the scoping is to the download only"; else bad "IT-167b the proxy names after the download" "$b167b"; fi
if [ -z "$c167c" ]; then ok "IT-167c (W1-232) the --from route is unchanged with the proxy names set: exit 0, ML FETCH TRUST PR UNPACK LOAD in that order, the release linked as current, no stage left"; else bad "IT-167c the --from route" "$c167c"; fi
# IT-167d (W1-232) THE CALLER'S NAMES COME BACK EXACTLY AS THEY WERE: rel_fetch_noproxy runs in THIS shell (a subshell would hold a TERM or HUP until the fetch ended: IT-108's signal cases) and puts the names back after the fetch. A name the caller did NOT set is still unset after it, a name set to the EMPTY string is
# still set and empty, the others keep their values. The caller leaves http_proxy, HTTP_PROXY, all_proxy and NO_PROXY unset, sets https_proxy and ALL_PROXY to values and HTTPS_PROXY and no_proxy to the empty string; rel_trust (the step after the download) records what it sees. (A guard: it passes on the original bytes too, where nothing is touched; its mutants are in the note.)
d167=""
fresh
env -u http_proxy -u HTTP_PROXY -u all_proxy -u NO_PROXY https_proxy=http://127.0.0.1:9/https_proxy HTTPS_PROXY= ALL_PROXY=http://127.0.0.1:9/ALL_PROXY no_proxy= \
  sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" --release 0.2.1-rc.2 --from "$T/from" > "$T/out" 2>&1 && rc=0 || rc=$?
for pw167 in http_proxy=unset HTTP_PROXY=unset all_proxy=unset NO_PROXY=unset https_proxy=http://127.0.0.1:9/https_proxy HTTPS_PROXY= ALL_PROXY=http://127.0.0.1:9/ALL_PROXY no_proxy=; do
  grep -qxF "PENV trust $pw167" "$T/log" || d167="$d167 [after the download the step saw '$(grep "^PENV trust ${pw167%%=*}=" "$T/log" | head -1 | cut -c1-80)' for ${pw167%%=*}, wanted '$pw167']"
done
[ "$rc" = 0 ] || d167="$d167 [the install did not finish: rc=$rc]"
if [ -z "$d167" ]; then ok "IT-167d (W1-232) the caller's names come back exactly: a name the caller left unset is still unset after the download, an empty one is still set and empty, the others keep their values"; else bad "IT-167d the names after the download" "$d167"; fi
# IT-109 step 10
sd109='say_done() { echo "SAYDONE $#|$1|$2|$3|$4" >> "$FAKE_LOG"; }'
ifd "machine_look() { return 0; }
prereq_run() { return 0; }
prereq_compose_path() { printf '%s\n' \"\$PROV2\"; }
$sd109" --project p --lead lead --home "$T/h" --port 8777
n109=$(grep -c '^SAYDONE ' "$T/log" || true)
r109="$rc"; l109=$(grep '^SAYDONE' "$T/log" | head -1); o109=$(grep -c '^install: done' "$T/out" || true)
if fresh && mkdir -p "$T/h" && mkrel; then
sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" --port 8777 > "$T/out" 2>&1 && rc=0 || rc=$?
v109=$(grep '^SAYDONE' "$T/log" | head -1)
if [ "$r109" = 0 ] && [ "$n109" = 1 ] && [ "$l109" = "SAYDONE 4|unknown|http://127.0.0.1:8777/v2/health/ready|$T/h/lead.token|none" ] && [ "$o109" = 0 ] && [ "$rc" = 0 ] && [ "$v109" = "SAYDONE 4|0.2.1-rc.2|http://127.0.0.1:8777/v2/health/ready|$T/h/lead.token|none" ]; then ok "IT-109 say_done VERSION ENDPOINT TOKENFILE LOG (the log is none: the installer keeps none; defect N) ends the install (the version is unknown for a checkout, the release's for a release); the old closing lines are not printed then"; else bad "IT-109 say_done" "rc=$r109 calls=$n109 '$l109' out-done=$o109; release run rc=$rc '$v109'"; fi
else bad "IT-109 fixture setup" "could not prepare dependent fixture"; fi

# ===================================================================================================================================================================
# 6e increment 3b: RESUME (W1-81). No state file: a step is "done" only if the MACHINE says so, and a check that cannot run means "not done".
#  IT-110 G5: a second run on a finished install changes nothing (file checksums, modes, owners, sizes, modification times, the fake's container generation) and exits 0; it
#         calls neither step 1 nor step 2 nor anything that writes or starts, and it still says what is installed (say_done).
#  IT-111 every broken or unknown condition makes the install NOT finished: the whole sequence runs again (step 1 is called) and repairs what it can; what it cannot repair stops.
#  IT-112 what a lost run leaves: named leftovers its own writers consume are gone, those nobody consumes and everything unknown are KEPT and reported, never removed.
#  IT-113 after a failure at any step the next run finishes the job: the end state equals a clean single run, and a third run changes nothing.
# ===================================================================================================================================================================
sd110='say_done() { echo "SAYDONE $#|$1|$2|$3|$4" >> "$FAKE_LOG"; }'
lane110="$(lane 'return 0;' 'return 0;')
$sd110"
ifd "$lane110" --check                                   # a copy of deploy/ with the lane's files: $T/d83
run3() { sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" "$@" > "$T/out" 2>&1 && rc=0 || rc=$?; }
rel_install() { fresh && mkdir -p "$T/h" && mkrel || return 1; run3; }   # a first install in release mode
allowed110='^(secret inspect |inspect |curl |--version$|compose version$|SAYDONE )'
lookup110="exec --user postgres --env PGCONNECT_TIMEOUT=5 --env PGOPTIONS=-c statement_timeout=5000 -c default_transaction_read_only=on cv2-s5-db psql -X -v ON_ERROR_STOP=1 -U postgres -d cortex -tAc SELECT p.id::text FROM identity.projects p JOIN identity.names n ON n.entity_kind = 'project' AND n.entity_id = p.id AND n.current WHERE n.name_key = 'p'"
if rel_install; then
r110=$rc; fpa=$(fp full)
: > "$T/log"; run3; r110b=$rc; fpb=$(fp full)
w110=$(grep -vE "$allowed110" "$T/log" | grep -vxF "$lookup110" | grep -vxF 'port cv2-s5-server 8601/tcp' || :)
b110=""
[ "$r110" = 0 ] || b110="$b110 [the first install failed: rc=$r110 $(tail -1 "$T/out" | cut -c1-70)]"
[ "$r110b" = 0 ] || b110="$b110 [the second run failed: rc=$r110b $(tail -1 "$T/out" | cut -c1-70)]"
[ "$fpa" = "$fpb" ] || b110="$b110 [the second run changed the machine: $(printf '%s\n%s\n' "$fpa" "$fpb" | LC_ALL=C sort | uniq -u | head -3 | tr '\n' '|')]"
[ -z "$w110" ] || b110="$b110 [the second run called: $(printf '%s' "$w110" | head -2 | tr '\n' '|')]"
[ "$(grep -c 'already installed' "$T/out" || true)" = 1 ] || b110="$b110 [the second run did not say it is already installed once]"
# SUM-7 also covers an installed release: all eight stages were actually done.
for stage110 in 'Checked this machine' Prerequisites 'Release fetched and verified' 'Images loaded' 'Containers started' 'Project and lead created' 'Lead credential written' Backups; do
  grep -qF "$stage110 ... ok (already done;" "$T/out" || b110="$b110 [repeat receipt missing: $stage110]"
done
! grep -q 'left alone' "$T/out" || b110="$b110 [a finished install reported entries of its own as strays: $(grep 'left alone' "$T/out" | cut -c1-100)]"
[ "$(grep -c '^SAYDONE ' "$T/log" || true)" = 1 ] || b110="$b110 [the second run did not say what is installed (say_done)]"
case $(grep '^SAYDONE ' "$T/log" | sed -n 1p) in "SAYDONE 4|"*"|none") ;; *) b110="$b110 [the second run did not pass say_done its fourth value, the install log (none): '$(grep '^SAYDONE ' "$T/log" | sed -n 1p | cut -c1-90)']" ;; esac
if [ -z "$b110" ]; then ok "IT-110 (G5) a second run on a finished install exits 0 and changes nothing: no file, mode, owner, size or modification time, no container made again; it calls neither step 1 nor step 2 nor anything that writes, and says what is installed"; else bad "IT-110 a second run changes nothing" "$b110"; fi
else bad "IT-110 fixture setup" "could not prepare dependent fixture"; fi
stop_after 110
# IT-111 each broken condition: the install is NOT finished (step 1 is called), what can be repaired is, the rest stops
# state/enabled is a SET (the fake appends in the order the units are enabled, and a repair that enables only the timers appends them AFTER the restart unit): it is left out of the fingerprint and compared sorted below
b111=""; ref111=""; en111=""
if rel_install; then
  ref111=$(fp files | grep -v -e ' h/lead.token ' -e 'state/enabled ' || :); en111=$(sort "$T/state/enabled")
else b111="$b111 [reference fixture setup]"; fi
case111() { # case111 <name> <break command, run in the test shell> <wanted rc> [VAR=value for the run] [extra install.sh arguments]
  if ! { rel_install && eval "$2" && : > "$T/log"; }; then
    b111="$b111 [$1: fixture setup]"; return 0
  fi
  p111=""; [ -z "${pre111:-}" ] || eval "$pre111" || p111=" the fixture is not what this cell says (precondition: $pre111)"   # a cell that isolates ONE guard proves the others cannot answer (finn, IM-296)
  # shellcheck disable=SC2086
  if [ -n "${4:-}" ]; then env "$4" sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" ${5:-} > "$T/out" 2>&1 && rc=0 || rc=$?; else run3 ${5:-}; fi
  c111="$p111"
  [ "$rc" = "$3" ] || c111="$c111 rc=$rc (want $3: $(tail -1 "$T/out" | cut -c1-60))"
  grep -qx ML "$T/log" || c111="$c111 step 1 was not called: the install took itself for finished"
  if [ "$3" = 0 ]; then
    now111=$(fp files | grep -v -e ' h/lead.token ' -e 'state/enabled ' || :)
    [ "$now111" = "$ref111" ] || c111="$c111 not repaired to the clean state: $(printf '%s\n%s\n' "$ref111" "$now111" | LC_ALL=C sort | uniq -u | head -2 | tr '\n' '|')"
    [ "$(sort "$T/state/enabled")" = "$en111" ] || c111="$c111 the enabled units are not what a clean install leaves: $(sort "$T/state/enabled" | tr '\n' ' ')"
    [ -s "$T/h/lead.token" ] && [ "$(stat -c %a "$T/h/lead.token")" = 600 ] || c111="$c111 the token file is missing, empty or not 0600"
  fi
  [ -z "$c111" ] || b111="$b111 [$1:$c111]"
}
case111 "the token file is gone" 'rm -f "$T/h/lead.token"' 0
case111 "the token file is open to others" 'chmod 0644 "$T/h/lead.token"' 7
case111 "the token file is empty" ': > "$T/h/lead.token"' 0
case111 "the config is gone" 'rm -f "$T/h/cortex.toml"' 0
case111 "a secret is gone" 'rm -f "$T/state/secret-cv2-s5-app-password"' 0
case111 "a secret is empty (never deleted for the person: the install stops)" ': > "$T/state/secret-cv2-s5-app-password"' 6
case111 "the containers are not healthy" ':' 6 FAKE_HEALTH=starting
case111 "only the database is unhealthy while the server remains healthy" ':' 6 FAKE_DB_HEALTH=starting
case111 "podman cannot be asked about its containers" ':' 6 FAKE_INSPECT_FAIL=1
# D181 now requires a successful final health/sign-in check. A repaired stack
# whose HTTP checks still fail must exit 6; this previously returned success.
case111 "the health route cannot be asked (curl fails)" ':' 6 FAKE_CURL_FAIL=1
case111 "the server reports another version than the release" ':' 0 FAKE_SERVER_VERSION=9.9.9
case111 "the server says it is not ready" ':' 0 FAKE_SERVER_READY=false
case111 "a unit file is gone" 'rm -f "$T/home/.config/systemd/user/cortex-dump.service"' 0
case111 "a unit file was edited" 'echo "# edited" >> "$T/home/.config/systemd/user/cortex-dump.timer"' 0
case111 "nothing at all is enabled (the backup timers AND the restart unit)" ': > "$T/state/enabled"' 0
# IM-296 (finn, 11 Nov, the second real run): the case above empties the WHOLE enabled list, so restart_unit_persistent said "not finished" too and the timers line of install_finished could be removed unseen; the same
# for FAKE_SYSTEMCTL_FAIL=1 below, which fails EVERY is-enabled question. The cells that follow isolate the timers line: the restart unit stays DURABLY enabled (each cell proves it) and only the timers are off or
# cannot be asked about, so that line is the only guard that can answer. (The other way round, ONLY the restart unit off, is IT-153 and IT-154 with the timers enabled.)
off111='grep -vx $(for t in $t111; do printf " -e %s" "$t"; done) "$T/state/enabled" > "$T/enabled.x" || :; cp "$T/enabled.x" "$T/state/enabled" && rm -f "$T/enabled.x"'
rst111='grep -qx podman-restart.service "$T/state/enabled"'
t111="cortex-dump.timer cortex-dump-status.timer"
pre111="$rst111 && ! grep -qx cortex-dump.timer \"\$T/state/enabled\" && ! grep -qx cortex-dump-status.timer \"\$T/state/enabled\""
case111 "both backup timers are off while the restart unit is durably enabled" "$off111" 0
t111="cortex-dump.timer"
pre111="$rst111 && ! grep -qx cortex-dump.timer \"\$T/state/enabled\" && grep -qx cortex-dump-status.timer \"\$T/state/enabled\""
case111 "only the dump timer is off (the restart unit and the status timer are enabled)" "$off111" 0
t111="cortex-dump-status.timer"
pre111="$rst111 && grep -qx cortex-dump.timer \"\$T/state/enabled\" && ! grep -qx cortex-dump-status.timer \"\$T/state/enabled\""
case111 "only the status timer is off (the restart unit and the dump timer are enabled)" "$off111" 0
pre111=""   # no precondition for the next cell: it fails EVERY question on purpose (it overlaps the restart-unit guard, which the cell after it does not)
case111 "the user manager cannot be asked at all (a timer check that cannot run is not done)" ':' 8 FAKE_SYSTEMCTL_FAIL=1
pre111="$rst111 && grep -qx cortex-dump.timer \"\$T/state/enabled\" && grep -qx cortex-dump-status.timer \"\$T/state/enabled\""
case111 "only the question about the timers fails (the timers and the restart unit are enabled and the restart unit answers)" ':' 8 FAKE_TIMERS_ASK_FAIL=1
unset pre111 t111 off111 rst111
case111 "an installed backup script was edited" 'echo "# edited" >> "$T/h/bin/cortex-dump"' 0
case111 "the backup scripts are gone" 'rm -rf "$T/h/bin"' 0
case111 "the compose provider record is stale" 'printf "%s\n" /old/podman-compose > "$T/h/compose-provider"' 0
case111 "the compose provider record is gone" 'rm -f "$T/h/compose-provider"' 0
case111 "the compose provider record is open to others' write" 'chmod 0666 "$T/h/compose-provider"' 0
case111 "the recorded provider says it is too old" ':' 3 'FAKE_COMPOSE_VERSION_OUT=podman-compose version 1.0.0\n'
case111 "the migration did not end well" ':' 6 FAKE_MIGRATE_FAIL=1
case111 "the dump directory is gone" 'rm -rf "$T/h/dumps"' 0
case111 "an installed backup script lost its mode" 'chmod 0700 "$T/h/bin/cortex-dump"' 0
case111 "an installed unit lost its mode" 'chmod 0600 "$T/home/.config/systemd/user/cortex-dump.service"' 0
# (another release asked for than the one installed is REFUSED before step 1: IT-108)
if [ -z "$b111" ]; then ok "IT-111 a missing or open token file, a missing config, a missing or empty secret, unhealthy or unaskable containers, a failing health route, another server version, a unit or script that is gone or edited, timers that are off or cannot be asked: each makes the install NOT finished and the sequence repairs what it can"; else bad "IT-111 not finished" "$b111"; fi
# IT-112 what a lost run leaves
if rel_install && : > "$T/log" &&
echo half > "$T/h/lead.token.part" && echo half > "$T/h/compose-provider.new" && echo half > "$T/h/bin/cortex-dump.new" && echo half > "$T/home/.config/systemd/user/cortex-dump.timer.new" &&
echo mine > "$T/h/notes.txt" && mkdir "$T/h/.mine" && echo mine > "$T/h/.mine/x" && cksum < "$T/h/notes.txt" > "$T/notes.sum"; then
rm -f "$T/h/cortex.toml"          # something that is not right, so that the run is not the finished one
run3
b112=""
[ "$rc" = 0 ] || b112="$b112 [rc=$rc: $(tail -1 "$T/out" | cut -c1-70)]"
[ "$(cksum < "$T/h/notes.txt")" = "$(cat "$T/notes.sum")" ] && [ "$(cat "$T/h/.mine/x")" = mine ] || b112="$b112 [an unknown file or directory was changed or removed]"
[ ! -e "$T/h/bin/cortex-dump.new" ] && [ ! -e "$T/home/.config/systemd/user/cortex-dump.timer.new" ] || b112="$b112 [a temporary file its own writer consumes is still there]"
[ -e "$T/h/lead.token.part" ] && [ -e "$T/h/compose-provider.new" ] || b112="$b112 [a leftover nobody consumes was removed]"
n112=$(grep -c 'left alone' "$T/out" || true)
{ [ "$n112" = 1 ] && grep 'left alone' "$T/out" | grep -q 'notes.txt' && grep 'left alone' "$T/out" | grep -q '.mine' && grep 'left alone' "$T/out" | grep -q 'lead.token.part' && grep 'left alone' "$T/out" | grep -q 'compose-provider.new'; } || b112="$b112 [the entries left alone were not reported in one line: $(grep 'left alone' "$T/out" | cut -c1-140)]"
if [ -z "$b112" ]; then ok "IT-112 a lost run's leftovers: what its own writers consume is gone, what nobody consumes and everything unknown is kept untouched and reported in one line"; else bad "IT-112 leftovers" "$b112"; fi
else bad "IT-112 fixture setup" "could not prepare dependent fixture"; fi
# IT-113 a failure at any step, then the next run finishes the job: the end state equals a clean single run; a third run changes nothing
b113=""; refc=""
if rel_install; then refc=$(fp files | grep -v ' h/lead.token ' || :); else b113="$b113 [reference fixture setup]"; fi
resume113() { # resume113 <VAR=value> <rc of the failing run>
  fresh
  if ! { mkdir -p "$T/h" && mkrel; }; then b113="$b113 [$1: fixture setup]"; return 0; fi
  env "$1" sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?; r1=$rc
  run3; r2=$rc; fpm=$(fp files | grep -v ' h/lead.token ' || :)
  fp2=$(fp full); : > "$T/log"; run3; r3=$rc; fp3=$(fp full)
  c113=""
  [ "$r1" = "$2" ] || c113="$c113 failing run rc=$r1 (want $2)"
  [ "$r2" = 0 ] || c113="$c113 next run rc=$r2: $(tail -1 "$T/out" | cut -c1-60)"
  [ "$fpm" = "$refc" ] || c113="$c113 end state differs from a clean run: $(printf '%s\n%s\n' "$refc" "$fpm" | LC_ALL=C sort | uniq -u | head -2 | tr '\n' '|')"
  [ "$r3" = 0 ] && [ "$fp2" = "$fp3" ] && ! grep -qx ML "$T/log" || c113="$c113 a third run was not a no-change run (rc=$r3)"
  [ -s "$T/h/lead.token" ] && [ "$(stat -c %a "$T/h/lead.token")" = 600 ] || c113="$c113 no 0600 token file"
  [ -z "$c113" ] || b113="$b113 [$1:$c113]"
}
resume113 FAKE_COMPOSE_FAIL=1 6
resume113 FAKE_HBA_FAIL=1 6
resume113 FAKE_PSQL_FAIL=1 6
resume113 FAKE_MIGRATE_FAIL=1 6
resume113 FAKE_HEALTH=starting 6
resume113 FAKE_BOOTSTRAP_FAIL=1 7
resume113 FAKE_CAT_EMPTY=1 7
resume113 FAKE_SYSTEMCTL_FAIL=1 8
if [ -z "$b113" ]; then ok "IT-113 after a failure at the database start, the trust check, the roles, the migration, the health wait, the first token or the backup timers, the next run finishes the job: the end state equals a clean install and a third run changes nothing"; else bad "IT-113 resume after a failure" "$b113"; fi

# IT-114 check_home, pinned to the ENGINE'S rule (cox 820170f5 Q2; model: finn's eu_ cells for unit_dir): scripts/check-home-corpus.txt holds the lists of the engine's own test mb_02 (good and bad homes). install.sh's
# --home check (the same rule in shell: absolute, only letters, digits and . _ / -, no empty and no .. component) must give the engine's verdict on every line: a good home is accepted (--check, so nothing is made),
# a bad one is refused with exit 1 naming --home. The real directory, owner and mode of an existing home are IT-107 and IT-111.
b114=""; n114=0; ng114=0; nb114=0
while IFS="$(printf '\t')" read -r kind114 path114; do
  case $kind114 in "#"*|"") continue ;; esac
  n114=$((n114 + 1)); p114=$(printf '%b' "$path114")
  ifc 'machine_look() { return 0; }' --check --home "$p114"
  if [ "$kind114" = good ]; then ng114=$((ng114 + 1)); [ "$rc" = 0 ] || b114="$b114 [good $path114 refused: rc=$rc $(tail -1 "$T/out" | cut -c1-50)]"
  else nb114=$((nb114 + 1)); { [ "$rc" = 1 ] && tail -1 "$T/out" | grep -q '^install: --home '; } || b114="$b114 [bad $path114 accepted or refused another way: rc=$rc $(tail -1 "$T/out" | cut -c1-50)]"; fi
done < "$here/check-home-corpus.txt"
[ "$ng114" -ge 4 ] && [ "$nb114" -ge 24 ] || b114="$b114 [the corpus is smaller than the engine's: $ng114 good, $nb114 bad]"
if [ -z "$b114" ]; then ok "IT-114 install.sh's --home check gives the engine's verdict on all $n114 homes of its own corpus ($ng114 good, $nb114 bad)"; else bad "IT-114 check_home against the engine's corpus" "$b114"; fi

# IT-115 the installed release is the VERIFIED one (W1-81 (1)(a); marv's page: verify_release_dir RELEASE_DIR, read-only, 0 only for a complete verified directory): in a build that installs releases (rel_fetch joined)
# <home>/current must be the link to <home>/releases/<version> and the shell verifier must say yes, both in the finished check and before anything is started; a verifier that is missing is "cannot run" = not done
# = refused (EX_FETCH, release_unverified), never skipped.
laneV="$lane3"
b115=""
calls115() { grep -E '^(ML|FETCH|TRUST|PR|UNPACK|LOAD|VERIFY)( |$)' "$T/log" | cut -d' ' -f1 | tr '\n' ' '; }
ifd "$laneV" --project p --lead lead --home "$T/h" --release 0.2.1-rc.2 --from "$T/from"
{ [ "$rc" = 0 ] && grep -qxF "VERIFY $T/h/releases/0.2.1-rc.2" "$T/log" && [ "$(calls115 | sed 's/.*LOAD //')" != "$(calls115)" ]; } || b115="$b115 [a first install: rc=$rc calls='$(calls115)']"
: > "$T/log"; inst3
{ [ "$rc" = 0 ] && grep -qxF "VERIFY $T/h/releases/0.2.1-rc.2" "$T/log" && [ "$(callsof)" = "" ] && grep -q 'already installed' "$T/out"; } || b115="$b115 [the finished check did not ask the verifier: rc=$rc calls='$(calls115)']"
: > "$T/log"; env FAKE_VERIFY_RC=1 sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" --release 0.2.1-rc.2 --from "$T/from" > "$T/out" 2>&1 && rc=0 || rc=$?
{ [ "$rc" = 5 ] && grep -q '^problem: .*could not be verified' "$T/out" && ! grep -q '^compose -f ' "$T/log" && ! grep -q 'already installed' "$T/out"; } || b115="$b115 [the verifier says no: rc=$rc compose-calls=$(grep -c '^compose -f ' "$T/log" || true) $(grep '^problem: ' "$T/out" | cut -c1-70)]"
if ln -sfn "$T/relsrc" "$T/h/current" && : > "$T/log"; then
inst3
{ [ "$rc" = 5 ] && grep -q '^problem: .*could not be verified' "$T/out" && ! grep -q '^compose -f ' "$T/log"; } || b115="$b115 [current points somewhere else than releases/V: rc=$rc compose-calls=$(grep -c '^compose -f ' "$T/log" || true)]"
else b115="$b115 [current fixture setup]"; fi
# <home>/current as a REAL DIRECTORY (a copy of the release) is refused too: it is not the link to releases/V (its physical path differs, so this holds even without the explicit link test: IM-343 is equivalent)
if rm -f "$T/h/current" && cp -R "$T/h/releases/0.2.1-rc.2" "$T/h/current" && : > "$T/log"; then
inst3
{ [ "$rc" = 5 ] && grep -q '^problem: .*could not be verified' "$T/out" && ! grep -q '^compose -f ' "$T/log"; } || b115="$b115 [current is a real directory: rc=$rc compose-calls=$(grep -c '^compose -f ' "$T/log" || true)]"
else b115="$b115 [current fixture setup]"; fi
ifd "$(printf '%s\n' "$lane3" | grep -v '^verify_release_dir')" --project p --lead lead --home "$T/h" --release 0.2.1-rc.2 --from "$T/from"
{ [ "$rc" = 5 ] && grep -q '^problem: .*could not be verified' "$T/out" && ! grep -q '^compose -f ' "$T/log"; } || b115="$b115 [no verify_release_dir in a build that installs releases: rc=$rc compose-calls=$(grep -c '^compose -f ' "$T/log" || true)]"
if [ -z "$b115" ]; then ok "IT-115 in a build that installs releases the finished check and the start both need <home>/current to be the link to releases/V and verify_release_dir to say yes; a missing verifier is refused, never skipped"; else bad "IT-115 verify_release_dir" "$b115"; fi

# IT-116 (hale RF-1, cox c3e157a3): bash IMPORTS EXPORTED FUNCTIONS before the script runs, also under sh and --posix, and an exported function named [ or printf shadows the builtin every check relies on: the very first
# lines of install.sh remove every function of the environment before anything is defined or sourced. The cells export functions named [, printf, command and compgen (the first says every test is true, the second prints nothing,
# the third says every command exists, the fourth lists no function: the removal must unset it first) and run the installer through that environment: a refusal must still refuse WITH its message, and a good install must still work exactly as in a clean environment.
cat > "$T/fnenv.sh" <<'FNENV'
#!/bin/bash
[() { return 0; }
printf() { :; }
command() { return 0; }
compgen() { :; }
umask() { echo RAN >> "${UMASK_CANARY:-/dev/null}"; }
export -f '[' printf command compgen umask
exec sh "$@"
FNENV
b116=""
rm -f "$T/umask.canary"
fresh; env UMASK_CANARY="$T/umask.canary" bash "$T/fnenv.sh" "$install" --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?
{ [ "$rc" = 1 ] && grep -q '^install: --project and --lead are required' "$T/out"; } || b116="$b116 [a refusal under exported [ and printf: rc=$rc out='$(cut -c1-60 "$T/out" | head -2 | tr '\n' '|')']"
fresh; env UMASK_CANARY="$T/umask.canary" bash "$T/fnenv.sh" "$install" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?
[ ! -e "$T/umask.canary" ] || b116="$b116 [an exported umask function RAN (hale 6E-2: umask was called before the functions were removed)]"
{ [ "$rc" = 0 ] && [ -s "$T/h/lead.token" ] && [ "$(mode "$T/h/lead.token")" = 600 ] && grep -qxF 'Endpoint: http://127.0.0.1:8601/v2/health/ready' "$T/out" && grep -q 'is installed and running' "$T/out"; } || b116="$b116 [a good install under exported [ printf command: rc=$rc token=$([ -s "$T/h/lead.token" ] && echo yes || echo no) out='$(head -c 80 "$T/out" | tr '\n' '|')']"
rm -f "$T/fnenv.sh"
if [ -z "$b116" ]; then ok "IT-116 functions exported by the environment ([, printf, command) are removed before anything is defined: a refusal still refuses with its message, a good install still works"; else bad "IT-116 exported functions" "$b116"; fi
# IT-117 (marv F-7, cox c3e157a3): say() must return the status of its PRINT: a failed stdout with a successful log must not read as success (a plan that could not be shown must not be consented to: G9)
b117=""
sayfns=$(grep -E '^(log_line|say)\(\) ' "$install")
r117=$(sh -c "$sayfns"'
CORTEX_LOG=$1; if say "plan line"; then echo SAY-OK >&2; else echo "SAY-FAILED rc=$?" >&2; fi' sh "$T/say.log" 2>&1 >&-)
{ printf '%s\n' "$r117" | grep -q '^SAY-FAILED' && ! printf '%s\n' "$r117" | grep -q '^SAY-OK' && grep -qx 'plan line' "$T/say.log"; } || b117="$b117 [stdout closed, log fine: '$(printf '%s' "$r117" | tr '\n' '|')' log='$(cat "$T/say.log" 2>/dev/null || :)']"
r117b=$(sh -c "$sayfns"'
CORTEX_LOG=$1; if say "plan line"; then echo SAY-OK >&2; else echo "SAY-FAILED rc=$?" >&2; fi' sh "$T/say.log" 2>&1 >/dev/null)
[ "$r117b" = SAY-OK ] || b117="$b117 [a say that prints fine did not return 0: '$r117b']"
rm -f "$T/say.log"
if [ -z "$b117" ]; then ok "IT-117 say() returns the status of its print: a closed stdout with a good log file is a failure (and the line is still logged), an open stdout is 0"; else bad "IT-117 say() propagates a failed print" "$b117"; fi

# ===================================================================================================================================================================
# vera's READ of 243fcf5 (REWORK_SCOPED, three MEDIUM): V6E-01 an existing release must be PROVEN before the prerequisites change anything; V6E-02 raw operational failures must end in the exit table with a cause and fix;
# V6E-03 rel_trust takes FIVE arguments (release, stage, key file, principal, fingerprint) bound to anchor data written into install.sh, never to the environment (W1-84).
# ===================================================================================================================================================================
# IT-118 (V6E-01): a release that is installed but whose install is NOT finished (the token is gone) must be proven BEFORE prereq_run (consent, packages, accounts, linger): an unverifiable release, a missing verifier, a
# current that points elsewhere, a damaged stack.env: exit 5 with a cause, and prereq_run, dnf, sudo and compose are NEVER reached. The positive control (a good release) still runs steps 1 and 4.
b118=""
prbad() { # prbad <what> <setup in the home before the run> [VAR=value] [regex the problem: line must match]: a finished install, then the damage, the token removed, one more run
  ifd "$lane3" --project p --lead lead --home "$T/h" --release 0.2.1-rc.2 --from "$T/from" --port 8777
  [ "$rc" = 0 ] || { b118="$b118 [$1: the first install failed rc=$rc]"; return 0; }
  if ! { eval "$2" && rm -f "$T/h/lead.token" && : > "$T/log"; }; then
    b118="$b118 [$1: fixture setup]"; return 0
  fi
  env ${3:-FAKE_NOTHING=1} sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" --release 0.2.1-rc.2 --from "$T/from" --port 8777 > "$T/out" 2>&1 && rc=0 || rc=$?
  c118=""
  [ "$rc" = 5 ] || c118="$c118 rc=$rc (want 5)"
  grep -qxE 'PR( .*)?' "$T/log" && c118="$c118 prereq_run WAS called"
  grep -qE '^(dnf|sudo|compose -f) ' "$T/log" && c118="$c118 a package manager, sudo or compose was called"
  [ "$(grep -c '^problem: ' "$T/out" || true)" = 1 ] && [ "$(grep -c '^fix: ' "$T/out" || true)" = 1 ] || c118="$c118 not exactly one problem: and one fix: line"
  # the CAUSE must be the true one (IM-349: a damaged stack.env read as version 0.0.0 is still refused, but as "release could not be verified")
  [ -z "${4:-}" ] || grep -qE "^problem: .*$4" "$T/out" || c118="$c118 the problem: line does not name the cause ($4): $(grep '^problem: ' "$T/out" | cut -c1-90)"
  [ -z "$c118" ] || b118="$b118 [$1:$c118]"
}
prbad "the verifier says no" ':' FAKE_VERIFY_RC=1
prbad "current points elsewhere" 'ln -sfn "$T/relsrc" "$T/h/current"'
prbad "stack.env is damaged" 'printf "%s\n" "GARBAGE" >> "$T/h/current/stack.env"' FAKE_NOTHING=1 'stack[.]env'
# a missing verifier: the lane's include without verify_release_dir
ifd "$lane3" --project p --lead lead --home "$T/h" --release 0.2.1-rc.2 --from "$T/from" --port 8777
printf '%s\n' "$(printf '%s\n' "$lane3" | grep -v '^verify_release_dir')" > "$T/d83/install-prereq.sh"; rm -f "$T/h/lead.token"; : > "$T/log"
sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" --release 0.2.1-rc.2 --from "$T/from" --port 8777 > "$T/out" 2>&1 && rc=0 || rc=$?
{ [ "$rc" = 5 ] && ! grep -qxE 'PR( .*)?' "$T/log" && ! grep -qE '^(dnf|sudo|compose -f) ' "$T/log"; } || b118="$b118 [no verifier: rc=$rc prereq_run called=$(grep -cxE 'PR( .*)?' "$T/log" || true)]"
# the control: a good, unfinished install goes on to the machine look and the prerequisites
ifd "$lane3" --project p --lead lead --home "$T/h" --release 0.2.1-rc.2 --from "$T/from" --port 8777
rm -f "$T/h/lead.token"; : > "$T/log"; sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" --release 0.2.1-rc.2 --from "$T/from" --port 8777 > "$T/out" 2>&1 && rc=0 || rc=$?
{ [ "$rc" = 0 ] && [ "$(callsof)" = "ML PR " ]; } || b118="$b118 [control: a good unfinished release did not reach steps 1 and 4: rc=$rc calls='$(callsof)']"
if [ -z "$b118" ]; then ok "IT-118 (V6E-01) an installed release is PROVEN before prereq_run: an unverifiable release, a missing verifier, a current that points elsewhere and a damaged stack.env stop with exit 5 and a cause before any prerequisite, package manager, sudo or compose call; a good one goes on"; else bad "IT-118 a reused release is proven before the prerequisites" "$b118"; fi
# IT-119 (V6E-02): an operational failure after the preflight ends in the exit table with ONE cause and ONE fix, never as a raw status of the tool: a fault-injection wrapper makes mv, cp, chmod, mkdir or rm fail
# for ONE named file; the podman fake makes secret create and the final temporary-copy removal fail.
mkdir -p "$T/bin-fi"
cat > "$T/bin-fi/fi" <<'FIEOF'
#!/bin/sh
# FAKE_FI is a list of specs "<command>=<text>[@N]" separated by ';': a call of this command whose arguments contain <text> FAILS (exit 1); with @N only the Nth such call fails (the calls are counted in
# "$FAKE_FI_COUNT.<command>", so a LATER removal can be faulted after an earlier one succeeded, sam I6E-SAM01); every other call runs the real command
me=${0##*/}; fi_real=$me
# W1-299: the legacy move fault is the publication fault. Apply it to the
# no-replace hard link too; marker links keep their independent controls.
if [ "$me" = ln ]; then
  for fi_target in "$@"; do :; done
  case $fi_target in *.token|*.token.[0123456789]*) me=mv ;; esac
fi
fi_old=$IFS; IFS=';'
for fi_spec in ${FAKE_FI:-}; do
  IFS=$fi_old
  case $fi_spec in "$me="*)
    pat=${fi_spec#*=}; nth=""; case $pat in *@[0-9]*) nth=${pat##*@}; pat=${pat%@*} ;; esac
    case " $* " in *"$pat"*)
      if [ -n "$nth" ]; then
        n=0; [ -z "${FAKE_FI_COUNT:-}" ] || n=$(cat "$FAKE_FI_COUNT.$me" 2>/dev/null || echo 0); n=$((n + 1)); [ -z "${FAKE_FI_COUNT:-}" ] || echo "$n" > "$FAKE_FI_COUNT.$me"
        [ "$n" = "$nth" ] || { IFS=';'; continue; }
      fi
      echo "fake $me: refused ($pat)" >&2; exit 1 ;;
    esac ;;
  esac
  IFS=';'
done
IFS=$fi_old
PATH=/usr/bin:/bin exec "$fi_real" "$@"
FIEOF
chmod +x "$T/bin-fi/fi"; for c in mv ln cp chmod mkdir rm; do cp "$T/bin-fi/fi" "$T/bin-fi/$c"; done
b119=""
fault119() { # fault119 <what> <wanted rc> <VAR=value> [prefill command run in $T/h before]
  fresh
  if ! { mkdir -p "$T/h" && chmod 0700 "$T/h" && { [ -z "${4:-}" ] || ( cd "$T/h" && eval "$4" ); }; }; then
    b119="$b119 [$1: fixture setup]"; return 0
  fi
  env PATH="$T/bin-fi:$PATH" "$3" sh "$install" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?
  c119=""
  [ "$rc" = "$2" ] || c119="$c119 rc=$rc (want $2: $(tail -1 "$T/out" | cut -c1-70))"
  [ "$(grep -c '^problem: ' "$T/out" || true)" = 1 ] && [ "$(grep -c '^fix: ' "$T/out" || true)" = 1 ] || c119="$c119 problem lines $(grep -c '^problem: ' "$T/out" || true), fix lines $(grep -c '^fix: ' "$T/out" || true)"
  [ -z "$c119" ] || b119="$b119 [$1:$c119]"
}
fault119 "podman secret create fails" 6 FAKE_SECRET_CREATE_FAIL=1
fault119 "the token cannot be made 0600" 7 FAKE_FI=chmod=lead.token.part
fault119 "an old partial token cannot be removed" 7 FAKE_FI=rm=lead.token.part ': > lead.token.part'
fault119 "the token cannot be moved into place" 7 FAKE_FI=mv=lead.token
fault119 "an empty earlier token cannot be removed" 7 FAKE_FI=rm=lead.token ': > lead.token'
fault119 "the config cannot be copied" 6 FAKE_FI=cp=cortex.toml
fault119 "the config cannot be made readable" 6 FAKE_FI=chmod=cortex.toml
fault119 "the bin directory cannot be made" 6 FAKE_FI=mkdir=/h/bin
# the removal of the temporary token copy in the container failing AFTER the token is published is a WARNING, not a lost install: exit 0, one warning, the token is in place
fresh; env FAKE_EXEC_RM_FAIL=1 sh "$install" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?
{ [ "$rc" = 0 ] && [ -s "$T/h/lead.token" ] && grep -q '^install: warning: ' "$T/out"; } || b119="$b119 [the temporary copy cannot be removed: rc=$rc warning=$(grep -c '^install: warning: ' "$T/out" || true)]"
if [ -z "$b119" ]; then ok "IT-119 (V6E-02) a failed secret create, token chmod or move, empty-token removal, config copy or chmod and bin mkdir end in EX_START 6 or EX_WORKER 7 with one cause and one fix (never a raw tool status); a failed removal of the temporary token copy is a warning"; else bad "IT-119 operational failures end in the exit table" "$b119"; fi
# IT-120 (V6E-03, r4): rel_trust is called with FIVE arguments (release, stage, key file, principal, fingerprint). They come from the THREE LITERALS at the top of install.sh (CORTEX_ANCHOR_PRINCIPAL, CORTEX_ANCHOR_KEY,
# CORTEX_ANCHOR_FPR: empty in the tree until D56/C-57, filled by the release build's join step; a cell fills a COPY with TEST values); the environment can never be the anchor (W1-84, hostile values are set in every run
# below). The key line goes into a private file in its own directory beside the stage (never in it: the stage holds exactly the seven files), removed on every exit.
b120=""
anchor_args() { grep '^TRUSTARGS ' "$T/log" | head -1; }
HOSTILE120="CORTEX_TRUST_KEY_FILE=/etc/evil CORTEX_TRUST_PRINCIPAL=evil CORTEX_TRUST_FPR=SHA256:evil CORTEX_ANCHOR_PRINCIPAL=evil CORTEX_ANCHOR_KEY=evil CORTEX_ANCHOR_FPR=SHA256:evil"
ifd "$lane3" --check
fresh; env $HOSTILE120 sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" --release 0.2.1-rc.2 --from "$T/from" > "$T/out" 2>&1 && rc=0 || rc=$?
a120=$(anchor_args); kf120=$(printf '%s' "$a120" | cut -d'|' -f2)
{ [ "$rc" = 0 ] && [ "$(printf '%s' "$a120" | cut -d'|' -f1)" = 'TRUSTARGS 5' ] && case $kf120 in /*) true ;; *) false ;; esac && [ "$(printf '%s' "$a120" | cut -d'|' -f3)" = "$A_P" ] && [ "$(printf '%s' "$a120" | cut -d'|' -f4)" = "$A_F" ]; } || b120="$b120 [the anchor: rc=$rc '$a120' (want 5 arguments, the principal and fingerprint of the literals, a hostile environment ignored)]"
{ grep -qxF "KEYFILE 1|600|700|$A_P ssh-ed25519 $A_K" "$T/log" && case $kf120 in "$T"/.cortex-install-stage.*) false ;; *) true ;; esac; } || b120="$b120 [the key file: $(grep '^KEYFILE' "$T/log" | head -1 | cut -c1-90) path=$kf120]"
[ -z "$(find "$T" -maxdepth 1 -name '.cortex-install-*' 2>/dev/null | head -1)" ] || b120="$b120 [a stage or anchor directory was left: $(find "$T" -maxdepth 1 -name '.cortex-install-*' | head -2 | tr '\n' ' ')]"
[ ! -e "$kf120" ] || b120="$b120 [the key file outlived the install]"
if [ -z "$b120" ]; then ok "IT-120 (V6E-03) rel_trust gets FIVE arguments: release, stage and the anchor written into install.sh (key file beside the stage, 0600 in a 0700 directory, removed on every exit); a hostile environment never changes them"; else bad "IT-120 rel_trust's five arguments" "$b120"; fi

# ===================================================================================================================================================================
# hale's PARTIAL READ of 243fcf5 (cox ab74e6b9, her order): 6E-1 the two 6b wrappers are called BARE (errexit is OFF inside a tested context, bodies included), an unknown status is mapped by an EXIT handler, and the traps
# survive a lane that replaces them; 6E-2 umask is no longer ahead of the removal of imported functions (IT-116); 6E-3/6E-5 the usage says "run it with sh" and that CORTEX_SCRIPTS_DIR is a CHECKOUT run's;
# 6E-4 PATH has absolute elements only, the loader/interpreter/OpenSSL and podman remote variables are unset for the run, podman is found ONCE by absolute path.
# ===================================================================================================================================================================
# IT-125 (6E-1)
b125=""
n125=$(grep -c '^[[:space:]]*machine_look$\|^[[:space:]]*prereq_run$' "$install" || true)
m125=$(grep -v '^[[:space:]]*#' "$install" | grep -c 'run_step .* \(machine_look\|prereq_run\)$' || true)
{ [ "$n125" = 2 ] && [ "$m125" = 0 ]; } || b125="$b125 [the 6b wrappers are not called bare: $n125 bare line(s), $m125 run_step call(s)]"
ifd "machine_look() { echo ML >> \"\$FAKE_LOG\"; false; echo AFTER-ML >> \"\$FAKE_LOG\"; return 0; }
prereq_compose_path() { printf '%s\n' \"\$PROV2\"; }" --project p --lead lead --home "$T/h"
{ [ "$rc" = 2 ] && ! grep -q '^AFTER-ML' "$T/log" && [ "$(grep -c '^problem: ' "$T/out" || true)" = 1 ] && [ "$(grep -c '^fix: ' "$T/out" || true)" = 1 ]; } || b125="$b125 [an unchecked failing command in machine_look: rc=$rc (want 2) after-line=$(grep -c '^AFTER-ML' "$T/log" || true) problems=$(grep -c '^problem: ' "$T/out" || true)]"
ifd "machine_look() { return 0; }
prereq_run() { echo PR >> \"\$FAKE_LOG\"; false; echo AFTER-PR >> \"\$FAKE_LOG\"; return 0; }
prereq_compose_path() { printf '%s\n' \"\$PROV2\"; }" --project p --lead lead --home "$T/h"
{ [ "$rc" = 3 ] && ! grep -q '^AFTER-PR' "$T/log" && [ "$(grep -c '^problem: ' "$T/out" || true)" = 1 ]; } || b125="$b125 [an unchecked failing command in prereq_run: rc=$rc (want 3) after-line=$(grep -c '^AFTER-PR' "$T/log" || true)]"
# a lane that REPLACES the traps (marv's prereq_requalify_compose does) must not make the stage outlive a later failure
lane125=$(printf '%s\n' "$lane3" | grep -v '^prereq_run()\|^machine_look()'; printf '%s\n' 'machine_look() { echo ML >> "$FAKE_LOG"; trap '"'"'echo THEIRS-ML >> "$FAKE_LOG"'"'"' EXIT; trap - HUP INT TERM; return 0; }' 'prereq_run() { echo "PR home=no" >> "$FAKE_LOG"; trap '"'"'echo THEIRS >> "$FAKE_LOG"'"'"' EXIT; trap - HUP INT TERM; return 0; }')
fresh; env FAKE_UNPACK_RC=5 sh -c 'exit 0'; ifd "$lane125" --check
fresh; env FAKE_UNPACK_RC=5 sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" --release 0.2.1-rc.2 --from "$T/from" > "$T/out" 2>&1 && rc=0 || rc=$?
{ [ "$rc" = 5 ] && [ -z "$(stages108)" ]; } || b125="$b125 [a lane replaced the traps: rc=$rc (want 5) stage left='$(stages108)']"
# the traps are put back right after machine_look too (IM-390): here the failure comes from the FETCH, which runs BEFORE prereq_run puts them back a second time
fresh; env FAKE_FETCH_RC=5 sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" --release 0.2.1-rc.2 --from "$T/from" > "$T/out" 2>&1 && rc=0 || rc=$?
{ [ "$rc" = 5 ] && [ -z "$(stages108)" ] && ! grep -q '^THEIRS-ML' "$T/log"; } || b125="$b125 [a lane's machine_look replaced the traps and they were not put back before the fetch: rc=$rc (want 5) stage left='$(stages108)' lane trap ran=$(grep -c '^THEIRS-ML' "$T/log" || true)]"
# IM-390 and IM-391 on the bytes AFTER run_step restores the traps (the whole run on 6d9807e: IM-390 SURVIVED; the fetch case above cannot tell the mutant from the original since 6f89dea, because the FETCH step puts the traps back itself before the exit). A stop that is NOT inside run_step comes right after the re-arm line, so it exits through whichever EXIT trap is installed THEN.
# after machine_look: an empty anchor literal refuses anchor_unusable (exit 4) outside run_step; the lane's machine_look replaced the EXIT trap, so if install.sh had not put its own back the LANE's trap (THEIRS-ML) would run
fresh; sh "$T/d83/install.empty.sh" --project p --lead lead --home "$T/h" --release 0.2.1-rc.2 --from "$T/from" > "$T/out" 2>&1 && rc=0 || rc=$?   # (ranc is defined further down, in the r4 section: not yet here)
{ [ "$rc" = 4 ] && grep -q '^ML' "$T/log" && ! grep -q '^THEIRS-ML' "$T/log" && [ "$(grep -c '^problem: ' "$T/out" || true)" = 1 ]; } || b125="$b125 [after machine_look replaced the traps, a stop outside run_step (an empty anchor, anchor_unusable): rc=$rc (want 4) machine_look ran=$(grep -c '^ML' "$T/log" || true) (want 1) lane trap ran=$(grep -c '^THEIRS-ML' "$T/log" || true) (want 0) problems=$(grep -c '^problem: ' "$T/out" || true)]"
# after prereq_run: the preflight (podman older than 5) stops with exit 3 right after the re-arm line; prereq_run replaced the EXIT trap (THEIRS), which must not be the one that runs
fresh; env FAKE_PODMAN_VERSION=4.9.0 sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?
{ [ "$rc" = 3 ] && grep -q '^PR home=no' "$T/log" && ! grep -qx 'THEIRS' "$T/log" && [ "$(grep -c '^problem: ' "$T/out" || true)" = 1 ]; } || b125="$b125 [after prereq_run replaced the traps, a stop outside run_step (the preflight: podman 4.9.0): rc=$rc (want 3) prereq_run ran=$(grep -c '^PR home=no' "$T/log" || true) (want 1) lane trap ran=$(grep -cx 'THEIRS' "$T/log" || true) (want 0) problems=$(grep -c '^problem: ' "$T/out" || true)]"
if [ -z "$b125" ]; then ok "IT-125 (6E-1) machine_look and prereq_run are called BARE (errexit stays on inside them: an unchecked failing command aborts the body and the EXIT handler maps it to the step's code with one cause and fix), and a lane that replaces the traps does not make the stage outlive a failure"; else bad "IT-125 the 6b wrappers are called bare" "$b125"; fi
# IT-127 (6E-3, 6E-5): the usage says how to run the file and what CORTEX_SCRIPTS_DIR is
b127=""
sh "$install" --help > "$T/out" 2>&1 || :
grep -q 'Run it with sh' "$T/out" || b127="$b127 [the usage does not say 'Run it with sh']"
grep -q 'CORTEX_SCRIPTS_DIR' "$T/out" && grep 'CORTEX_SCRIPTS_DIR' "$T/out" | grep -q 'CHECKOUT' || b127="$b127 [the usage does not say that CORTEX_SCRIPTS_DIR is a CHECKOUT run's]"
if [ -z "$b127" ]; then ok "IT-127 (6E-3, 6E-5) the usage says to run it with sh and that CORTEX_SCRIPTS_DIR names the backup scripts of a source CHECKOUT run only"; else bad "IT-127 the usage" "$b127"; fi
# IT-128 (6E-4)
b128=""
SHBIN=$(command -v sh)
for badpath in "relative/bin:$PATH" ":$PATH" "$PATH:" "/usr/bin::/bin:$PATH" "./x:$PATH" "/usr/bin:rel/bin:/bin:$PATH"; do
  fresh; env PATH="$badpath" "$SHBIN" "$install" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?
  { [ "$rc" = 1 ] && grep -q '^problem: .*PATH' "$T/out" && [ "$(grep -c '^fix: ' "$T/out" || true)" = 1 ] && untouched; } || b128="$b128 [PATH=$(printf '%.20s' "$badpath"): rc=$rc untouched=$(untouched && echo y || echo n) $(tail -1 "$T/out" | cut -c1-60)]"
done
HOST128="LD_PRELOAD= LD_AUDIT= LD_LIBRARY_PATH=/nonexistent-cv2 PYTHONPATH=/nonexistent-cv2 PYTHONHOME=/nonexistent-cv2 PYTHONSTARTUP=/nonexistent-cv2 BASH_ENV=/nonexistent-cv2 ENV=/nonexistent-cv2 CDPATH=/nonexistent-cv2 OPENSSL_CONF=/nonexistent-cv2 OPENSSL_MODULES=/nonexistent-cv2 OPENSSL_ENGINES=/nonexistent-cv2 GCONV_PATH=/nonexistent-cv2 CONTAINER_HOST=tcp://127.0.0.1:1 CONTAINER_CONNECTION=nonexistent CONTAINERS_CONF=/nonexistent-cv2 CONTAINERS_STORAGE_CONF=/nonexistent-cv2 DOCKER_HOST=tcp://127.0.0.1:1 REGISTRY_AUTH_FILE=/nonexistent-cv2 CONTAINERS_REGISTRIES_CONF=/nonexistent-cv2"
: > "$FAKE_PROVLOG.env"; fresh
# shellcheck disable=SC2086
env $HOST128 FAKE_ENVCHECK=1 sh "$install" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?
{ [ "$rc" = 0 ] && [ ! -s "$FAKE_PROVLOG.env" ]; } || b128="$b128 [a dangerous variable reached podman: rc=$rc seen=$(sed 's/=.*//' "$FAKE_PROVLOG.env" | sort -u | tr '\n' ' ' | cut -c1-120)]"
# podman is found ONCE, by absolute path: a lane that puts a hostile podman first in PATH afterwards is not obeyed
mkdir -p "$T/hostile-bin"; printf '#!/bin/sh\necho "HOSTILE-PODMAN $*" >> "%s"\nexit 1\n' "$T/hostile.canary" > "$T/hostile-bin/podman"; chmod +x "$T/hostile-bin/podman"; rm -f "$T/hostile.canary"
ifd "machine_look() { PATH=\"$T/hostile-bin:\$PATH\"; export PATH; return 0; }
prereq_compose_path() { printf '%s\n' \"\$PROV2\"; }" --project p --lead lead --home "$T/h"
{ [ "$rc" = 0 ] && [ ! -e "$T/hostile.canary" ]; } || b128="$b128 [a hostile podman put first in PATH by a lane WAS called: rc=$rc canary=$(head -c 160 "$T/hostile.canary" 2>/dev/null | tr '\n' '|')]"
# a machine WITHOUT podman at the start: the prerequisite step brings it, and from then on it is pinned too (the pin is made again after prereq_run)
mkdir -p "$T/late" "$T/binx"; for f in systemctl dnf sudo; do ln -sf "$T/bin/$f" "$T/binx/$f"; done; ln -sf "$T/bin-curl/curl" "$T/binx/curl"
# D181's summary uses the prerequisite Python JSON parser and coreutils formatter.
# W1-303b: uname/rmdir supply reservation metadata/release; none provides podman. The late-podman proof stays intact.
for f in python3 fold uname rmdir; do ln -sf "$(command -v "$f")" "$T/binx/$f"; done
rm -f "$T/late/podman"
lanel="machine_look() { return 0; }
prereq_run() { cp \"$T/bin/podman\" \"$T/late/podman\"; return 0; }
prereq_compose_path() { printf '%s\n' \"\$PROV2\"; }"
rm -rf "$T/d83"; copy_deploy "$(dirname "$install")" "$T/d83"; printf '%s\n' "$lanel" > "$T/d83/install-prereq.sh"; fresh
env PATH="$T/late:$T/binx:$P/minbin:$P" "$SHBIN" "$T/d83/install.sh" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?
{ [ "$rc" = 0 ] && [ -s "$T/h/lead.token" ]; } || b128="$b128 [podman only after the prerequisites: rc=$rc $(tail -1 "$T/out" | cut -c1-80)]"
# (IM-414) a lane's prereq_run that puts a hostile podman first in PATH: the PATH is put back after it, so no child process finds it either
rm -f "$T/hostile.canary"
ifd "prereq_run() { PATH=\"$T/hostile-bin:\$PATH\"; export PATH; return 0; }
prereq_compose_path() { printf '%s\n' \"\$PROV2\"; }" --project p --lead lead --home "$T/h"
{ [ "$rc" = 0 ] && [ ! -e "$T/hostile.canary" ]; } || b128="$b128 [a hostile podman put first in PATH by prereq_run WAS called: rc=$rc canary=$(head -c 160 "$T/hostile.canary" 2>/dev/null | tr '\n' '|')]"
# THE PIN ITSELF (IM-410, IM-413, IM-415), proven where no PATH restore can help: rel_load puts a hostile podman first in PATH and CALLS podman itself inside the same step (the PATH is put back only after the step). Child processes (db-roles.sh,
# the compose provider) look podman up in PATH and are not covered by the pin, so the hostile podman passes every call on to the real fake and names its caller: only a call from the installer's OWN shell
# (its parent's command line holds install.sh) counts, and the pin must have kept those on the absolute path. (Linux: /proc.)
mkdir -p "$T/hostile2"
printf '#!/bin/sh\nc=$(tr "\\000" " " < /proc/$PPID/cmdline 2>/dev/null)\ncase $c in *install.sh*) echo "HOSTILE-FROM-INSTALLER $*" >> "%s" ;; esac\nexec "%s" "$@"\n' "$T/hostile2.canary" "$T/bin/podman" > "$T/hostile2/podman"; chmod +x "$T/hostile2/podman"
hl128() { printf '%s\n' "$1" | sed "s#^rel_load() {#rel_load() { PATH=\"$T/hostile2:\$PATH\"; export PATH; podman version >/dev/null || :;#"; }
# (a) podman on the machine from the start, no prerequisite file: only the pin made at the start can protect the installer's own calls
laneA=$(hl128 "$(printf '%s\n' "$lane3" | grep -v '^prereq_run()\|^machine_look()')")
rm -f "$T/hostile2.canary"
ifd "$laneA" --project p --lead lead --home "$T/h" --release 0.2.1-rc.2 --from "$T/from" --port 8777
{ [ "$rc" = 0 ] && [ -s "$T/h/lead.token" ] && [ ! -s "$T/hostile2.canary" ]; } || b128="$b128 [pin at the start: rc=$rc the installer's own shell called the hostile podman: $(head -c 120 "$T/hostile2.canary" 2>/dev/null | tr '\n' '|')]"
# (b) podman only after the prerequisites: it is pinned again once prereq_run has brought it
laneB=$(hl128 "$(printf '%s\n' "$lane3" | grep -v '^prereq_run()'; printf '%s\n' 'prereq_run() { echo "PR" >> "$FAKE_LOG"; cp "'"$T"'/bin/podman" "'"$T"'/late/podman"; return 0; }')")
rm -f "$T/late/podman" "$T/hostile2.canary"
# a release run needs the trust tools (step 2) and a few more than a checkout run: link them into the minimal PATH (no podman among them)
for tl in sha256sum tar gzip ssh-keygen readlink basename realpath tee uname; do tlp=$(command -v "$tl" 2>/dev/null) && ln -sf "$tlp" "$T/binx/$tl" || :; done
rm -rf "$T/d83"; copy_deploy "$(dirname "$install")" "$T/d83"; cp "$T/d83/install.sh" "$T/d83/install.empty.sh"; fill_anchor "$T/d83/install.empty.sh" "$T/d83/install.sh" "$A_P" "$A_K" "$A_F"; printf '%s\n' "$laneB" > "$T/d83/install-prereq.sh"; fresh
env PATH="$T/late:$T/binx:$P/minbin:$P" "$SHBIN" "$T/d83/install.sh" --project p --lead lead --home "$T/h" --release 0.2.1-rc.2 --from "$T/from" --port 8777 > "$T/out" 2>&1 && rc=0 || rc=$?
{ [ "$rc" = 0 ] && [ -s "$T/h/lead.token" ] && [ ! -s "$T/hostile2.canary" ]; } || b128="$b128 [pin after the prerequisites: rc=$rc the installer's own shell called the hostile podman: $(head -c 120 "$T/hostile2.canary" 2>/dev/null | tr '\n' '|') $(tail -1 "$T/out" | cut -c1-80)]"
if [ -z "$b128" ]; then ok "IT-128 (6E-4) a PATH with an empty or relative element is refused before anything; the loader, interpreter, OpenSSL, shell-startup and podman remote/config variables are unset for the run; podman is found ONCE by absolute path (a hostile one put first in PATH later is not obeyed)"; else bad "IT-128 PATH, environment and podman" "$b128"; fi

# IT-129: the proof of an installed release (V6E-01) is skipped for a REISSUE (it starts nothing and creates nothing), so the release read further down is the refusal of a damaged stack.env on that path: exit 5, the
# catalogue's cause and fix, nothing started or minted (mutants IM-171 and its catalogue twin survived while this path had no cell)
b129=""
if fresh && mkdir -p "$T/h" && mkrel && printf 'GARBAGE\n' >> "$T/h/current/stack.env"; then
run --reissue-token && rc=0 || rc=$?
{ [ "$rc" = 5 ] && [ "$(grep -c '^problem: ' "$T/out" || true)" = 1 ] && [ "$(grep -c '^fix: ' "$T/out" || true)" = 1 ] && ! grep -q '^compose -f ' "$T/log" && ! grep -q '^exec ' "$T/log" && ! ls "$T/h"/lead.token.* >/dev/null 2>&1; } || b129="$b129 [a damaged stack.env with --reissue-token: rc=$rc problems=$(grep -c '^problem: ' "$T/out" || true) fix=$(grep -c '^fix: ' "$T/out" || true) token files=$(ls "$T/h"/lead.token.* 2>/dev/null | wc -l)]"
else b129="${b129} [fixture setup]"; fi
if [ -z "$b129" ]; then ok "IT-129 a damaged stack.env stops a --reissue-token too: exit 5 with the catalogue's cause and fix, nothing started, no token minted"; else bad "IT-129 stack.env and a reissue" "$b129"; fi

# ===================================================================================================================================================================
# r4: THE TRUST ANCHOR AS THREE LITERALS (hue's page TRUST-ANCHOR-LITERALS-hue-r1.md, vera's ruling f8644b5, cox 29cfe86f and d7dd3993, plan INSTALL-6E-R4-ANCHOR-CELL-PLAN-nemo-r0.md): IT-130 to IT-139.
# The tree holds CORTEX_ANCHOR_PRINCIPAL, CORTEX_ANCHOR_KEY and CORTEX_ANCHOR_FPR EMPTY (the development layout, until the CTO's key, C-57); the release build's join step fills them. A cell fills a COPY with TEST values.
# NOT RUN here: the joined cell with hue's REAL rel_trust and a real throwaway key (right = trusted, one FPR character or another KEY = trust_key, another signer = trust_signature): his lane is not merged with the
# real rel_unpack and rel_load yet; the fake rel_trust of lane3 models only the argument and file contract.
# ===================================================================================================================================================================
ifd "$lane3" --check      # $T/d83: the deploy files, the fake lane (lane3), install.sh with the TEST anchor filled in, install.empty.sh as it is
ancleft() { find "$T" -maxdepth 1 -name '.cortex-install-anchor.*' 2>/dev/null | head -3 | tr '\n' ' '; }   # anchor directories beside the home
ranc() { # ranc <installer file in $T/d83> [VAR=value ...]: a release run of that copy (lane3 is in $T/d83); sets rc, $T/out and $T/log
  ra_f=$1; shift; fresh; env "$@" sh "$T/d83/$ra_f" --project p --lead lead --home "$T/h" --release 0.2.1-rc.2 --from "$T/from" > "$T/out" 2>&1 && rc=0 || rc=$?
}
refused131() { # after a run: sets r131 to the reasons it was NOT refused the way an empty or malformed literal must be (empty = refused correctly)
  r131=""
  [ "$rc" = 4 ] || r131="$r131 rc=$rc (want 4)"
  { [ "$(grep -c '^problem: ' "$T/out" || true)" = 1 ] && [ "$(grep -c '^fix: ' "$T/out" || true)" = 1 ]; } || r131="$r131 [not exactly one problem: and one fix: line]"
  ! grep -qE '^(FETCH|TRUST|PR|UNPACK|LOAD)( |$)' "$T/log" || r131="$r131 [a step ran: $(callsof)]"
  { [ -z "$(stages108)" ] && [ -z "$(ancleft)" ] && [ ! -e "$T/h" ]; } || r131="$r131 [stage='$(stages108)' anchor='$(ancleft)' home=$([ -e "$T/h" ] && echo made || echo none)]"
}
# IT-130 the three statements: whole lines, empty in the tree, each exactly once, in this order, the first three statements after the header (blank and # lines), before set -eu and before any function
b130=""
w130=$(printf '%s\n' "CORTEX_ANCHOR_PRINCIPAL=''" "CORTEX_ANCHOR_KEY=''" "CORTEX_ANCHOR_FPR=''")
f130=$(grep -v '^[[:space:]]*$' "$install" | grep -v '^[[:space:]]*#' | sed -n 1,3p)
[ "$f130" = "$w130" ] || b130="$b130 [the first three statements are not the placeholders: '$(printf '%s' "$f130" | tr '\n' '|' | cut -c1-120)']"
for n130 in CORTEX_ANCHOR_PRINCIPAL CORTEX_ANCHOR_KEY CORTEX_ANCHOR_FPR; do
  c130=$(grep -c -x "$n130=''" "$install" || true); [ "$c130" = 1 ] || b130="$b130 [$n130='' occurs $c130 times as a whole line (want 1)]"
  c130=$(grep -c "^[[:space:]]*$n130=" "$install" || true); [ "$c130" = 1 ] || b130="$b130 [$n130 is assigned $c130 times (want 1: the placeholder only)]"
done
l130=$(grep -n -x "CORTEX_ANCHOR_PRINCIPAL=''" "$install" | cut -d: -f1 | head -1)
{ [ -n "$l130" ] && [ "$(sed -n "1,$((l130 - 1))p" "$install" | grep -c -v -e '^[[:space:]]*$' -e '^[[:space:]]*#' || true)" = 0 ]; } || b130="$b130 [a statement comes before the placeholders]"
[ "$(grep -c 'TRUST_KEY_LINE\|TRUST_PRINCIPAL\|TRUST_FPR' "$install" || true)" = 0 ] || b130="$b130 [the old builder-written anchor names are still in the file]"
if [ -z "$b130" ]; then ok "IT-130 (r4) CORTEX_ANCHOR_PRINCIPAL='', CORTEX_ANCHOR_KEY='' and CORTEX_ANCHOR_FPR='' are whole lines, each exactly once, empty in the tree, in this order, the first three statements after the header, before any other command or function; the old TRUST_* names are gone"; else bad "IT-130 the anchor placeholders" "$b130"; fi
# IT-131 an EMPTY literal (each alone, and all three: the tree as it is) refuses anchor_unusable (exit 4, one problem and one fix) before anything is fetched, trusted, prepared, unpacked or loaded: no stage, no anchor directory, no home
b131=""
try131() { # try131 <what> <principal> <key> <fpr>
  fill_anchor "$T/d83/install.empty.sh" "$T/d83/install.try.sh" "$2" "$3" "$4"; ranc install.try.sh; refused131; [ -z "$r131" ] || b131="$b131 [$1:$r131]"
}
ranc install.empty.sh; refused131; [ -z "$r131" ] || b131="$b131 [all three empty, the tree as it is:$r131]"
try131 "the principal empty" "" "$A_K" "$A_F"
try131 "the key empty" "$A_P" "" "$A_F"
try131 "the fingerprint empty" "$A_P" "$A_K" ""
# the refusal comes BEFORE the stage is reserved: with a parent the user cannot write to, the stage could not be made (exit 2, EX_MACHINE), so an empty anchor must still say exit 4
mkdir -p "$T/ro"; chmod 0555 "$T/ro"; fresh
sh "$T/d83/install.empty.sh" --project p --lead lead --home "$T/ro/h" --release 0.2.1-rc.2 --from "$T/from" > "$T/out" 2>&1 && rc=0 || rc=$?
{ [ "$rc" = 4 ] && [ "$(grep -c '^problem: ' "$T/out" || true)" = 1 ] && ! grep -qE '^(FETCH|TRUST|PR|UNPACK|LOAD)( |$)' "$T/log"; } || b131="$b131 [an empty anchor with a parent that cannot hold a stage: rc=$rc (want 4: the refusal is before the stage is reserved)]"
chmod 0755 "$T/ro"; rmdir "$T/ro"
if [ -z "$b131" ]; then ok "IT-131 (r4) an empty literal, each alone and all three (the tree as it is), refuses anchor_unusable with exit 4, one problem: and one fix: line, before any fetch, trust, prerequisite, unpack or load; no stage, no anchor directory, no home"; else bad "IT-131 an empty anchor literal" "$b131"; fi
# IT-132 the environment is never the anchor (W1-84, hue's cell tr27): a COMPLETE valid anchor in the environment does not fill empty literals, and other valid values do not replace filled ones
b132=""
ranc install.empty.sh CORTEX_ANCHOR_PRINCIPAL=evil-principal "CORTEX_ANCHOR_KEY=AAAAC3NzaC1lZDI1NTE5AAAAI$(printf 'D%.0s' $(seq 43))" "CORTEX_ANCHOR_FPR=SHA256:$(printf 'E%.0s' $(seq 43))"
refused131; [ -z "$r131" ] || b132="$b132 [empty literals with a complete anchor in the environment:$r131]"
# (a2) each literal ALONE empty, the other two filled, and a valid value for it in the environment: still refused (a fallback to the environment for any one of them is a fault)
for x132 in PRINCIPAL KEY FPR; do
  case $x132 in PRINCIPAL) f1="" f2="$A_K" f3="$A_F" ;; KEY) f1="$A_P" f2="" f3="$A_F" ;; FPR) f1="$A_P" f2="$A_K" f3="" ;; esac
  fill_anchor "$T/d83/install.empty.sh" "$T/d83/install.try.sh" "$f1" "$f2" "$f3"
  ranc install.try.sh "CORTEX_ANCHOR_PRINCIPAL=$A_P" "CORTEX_ANCHOR_KEY=$A_K" "CORTEX_ANCHOR_FPR=$A_F"
  refused131; [ -z "$r131" ] || b132="$b132 [$x132 empty with a valid one in the environment:$r131]"
done
ranc install.sh CORTEX_ANCHOR_PRINCIPAL=evil-principal "CORTEX_ANCHOR_KEY=AAAAC3NzaC1lZDI1NTE5AAAAI$(printf 'D%.0s' $(seq 43))" "CORTEX_ANCHOR_FPR=SHA256:$(printf 'E%.0s' $(seq 43))"
a132=$(anchor_args)
{ [ "$rc" = 0 ] && [ "$(printf '%s' "$a132" | cut -d'|' -f3)" = "$A_P" ] && [ "$(printf '%s' "$a132" | cut -d'|' -f4)" = "$A_F" ] && grep -qxF "KEYFILE 1|600|700|$A_P ssh-ed25519 $A_K" "$T/log"; } || b132="$b132 [filled literals with other valid values in the environment: rc=$rc '$a132' $(grep '^KEYFILE' "$T/log" | head -1 | cut -c1-60)]"
if [ -z "$b132" ]; then ok "IT-132 (r4) a complete valid anchor in the environment does not fill empty literals (refused), and other valid values in the environment never replace filled ones (rel_trust gets the literals)"; else bad "IT-132 the environment is never the anchor" "$b132"; fi
# IT-133 filled literals: rel_trust runs with the release, the stage, the key file, the principal and the fingerprint IN THAT ORDER; the key file holds exactly "PRINCIPAL ssh-ed25519 KEY" and LF (one line), 0600, in a 0700 directory, and rel_trust SEES that content at the call
b133=""
ranc install.sh
a133=$(anchor_args); stg133=$(sed -n 's/^FETCH [^ ]* \([^ ]*\) home=.*/\1/p' "$T/log")
{ [ "$rc" = 0 ] && grep -qxF "TRUST 0.2.1-rc.2 $stg133 home=no" "$T/log" && [ "$(printf '%s' "$a133" | cut -d'|' -f1)" = 'TRUSTARGS 5' ] && [ "$(printf '%s' "$a133" | cut -d'|' -f3)" = "$A_P" ] && [ "$(printf '%s' "$a133" | cut -d'|' -f4)" = "$A_F" ]; } || b133="$b133 [the five arguments in order: rc=$rc '$a133']"
grep -qxF "KEYFILE 1|600|700|$A_P ssh-ed25519 $A_K" "$T/log" || b133="$b133 [what rel_trust saw in the key file: $(grep '^KEYFILE' "$T/log" | head -1 | cut -c1-110) (want 1 line, 600, 700, the line of the literals)]"
if [ -z "$b133" ]; then ok "IT-133 (r4) with filled literals rel_trust gets the release, the stage, the key file, the principal and the fingerprint in this order, and sees one line PRINCIPAL ssh-ed25519 KEY in a 0600 file in a 0700 directory"; else bad "IT-133 the filled literals reach rel_trust" "$b133"; fi
# IT-134 placement: the anchor directory is NOT inside the stage (the stage keeps exactly what rel_fetch put there: the fake puts one file), it sits beside the stage (the same parent), has a fresh mktemp name every run
b134=""
kf134=$(printf '%s' "$a133" | cut -d'|' -f2); kd134=${kf134%/*}
case $kd134 in "$stg133"|"$stg133"/*) b134="$b134 [the anchor directory is INSIDE the stage: $kd134]" ;; esac
[ "${kd134%/*}" = "${stg133%/*}" ] || b134="$b134 [the anchor directory's parent '${kd134%/*}' is not the stage's parent '${stg133%/*}']"
case ${kd134##*/} in .cortex-install-anchor.????????) ;; *) b134="$b134 [the anchor directory's name: ${kd134##*/}]" ;; esac
grep -qx 'STAGEAT 1' "$T/log" || b134="$b134 [the stage held $(grep '^STAGEAT' "$T/log" | head -1) when rel_trust ran (want 1: the one file rel_fetch made; the anchor is not an entry of it)]"
ranc install.sh; k2134=$(anchor_args | cut -d'|' -f2)
{ [ -n "$kd134" ] && [ "${k2134%/*}" != "$kd134" ] && [ "${kd134##*/}" != '.cortex-install-anchor.XXXXXXXX' ]; } || b134="$b134 [the anchor directory is not fresh every run: '$kd134' then '${k2134%/*}']"
if [ -z "$b134" ]; then ok "IT-134 (r4) the anchor directory is outside the stage (the stage keeps what rel_fetch put there), beside it (same parent), named by mktemp afresh every run"; else bad "IT-134 the anchor directory's place" "$b134"; fi
# IT-135 MALFORMED INPUT: a literal outside rel_trust's grammar refuses anchor_unusable before anything runs and before a key file is written (principal 1 to 64 of [A-Za-z0-9._@-], not hyphen-led; key 68 of [A-Za-z0-9+/];
# fingerprint SHA256: and 43 of [A-Za-z0-9+/]); the limits themselves are ACCEPTED (positive controls)
b135=""
try135() { # try135 <what> <principal> <key> <fpr>
  fill_anchor "$T/d83/install.empty.sh" "$T/d83/install.try.sh" "$2" "$3" "$4"; ranc install.try.sh; refused131; [ -z "$r131" ] || b135="$b135 [$1:$r131]"
}
for p135 in "$(printf 'a%.0s' $(seq 65))|65 characters" "-lead|hyphen-led" "a b|a space" 'a\nb|a newline' "a/b|a slash" "a:b|a colon" 'a;b|a semicolon' 'a$(id)|a command substitution' " x|leading white space" ; do
  try135 "principal: ${p135#*|}" "${p135%%|*}" "$A_K" "$A_F"
done
try135 "key: 67 characters" "$A_P" "$(printf '%s' "$A_K" | cut -c1-67)" "$A_F"
try135 "key: 69 characters" "$A_P" "${A_K}B" "$A_F"
try135 "key: a character outside the set in the middle" "$A_P" "$(printf '%s' "$A_K" | sed 's/B/!/')" "$A_F"
try135 "key: base64 padding" "$A_P" "$(printf '%s' "$A_K" | sed 's/B$/=/')" "$A_F"
try135 "key: a space" "$A_P" "$(printf '%s' "$A_K" | sed 's/B/ /')" "$A_F"
try135 "fingerprint: 42 characters after SHA256:" "$A_P" "$A_K" "SHA256:$(printf 'C%.0s' $(seq 42))"
try135 "fingerprint: 44 characters after SHA256:" "$A_P" "$A_K" "SHA256:$(printf 'C%.0s' $(seq 44))"
try135 "fingerprint: MD5 form" "$A_P" "$A_K" "MD5:$(printf 'C%.0s' $(seq 43))"
try135 "fingerprint: lower-case prefix" "$A_P" "$A_K" "sha256:$(printf 'C%.0s' $(seq 43))"
try135 "fingerprint: no prefix" "$A_P" "$A_K" "$(printf 'C%.0s' $(seq 43))"
try135 "fingerprint: base64 padding" "$A_P" "$A_K" "SHA256:$(printf 'C%.0s' $(seq 42))="
# the limits are accepted: a 64-character principal and every allowed punctuation mark
for pc135 in "$(printf 'a%.0s' $(seq 64))" "a._@-b"; do
  fill_anchor "$T/d83/install.empty.sh" "$T/d83/install.try.sh" "$pc135" "$A_K" "$A_F"; ranc install.try.sh
  { [ "$rc" = 0 ] && [ "$(anchor_args | cut -d'|' -f3)" = "$pc135" ]; } || b135="$b135 [a good principal of ${#pc135} characters was refused or not passed on: rc=$rc]"
done
if [ -z "$b135" ]; then ok "IT-135 (r4) MALFORMED INPUT: a literal outside rel_trust's grammar refuses anchor_unusable before anything runs (principal: 65 characters, hyphen, space, newline, slash, colon, semicolon, command substitution, white space; key: 67, 69, a character outside the set, padding, a space; fingerprint: 42, 44, MD5 form, lower-case prefix, no prefix, padding); the 64-character principal and every allowed mark are accepted"; else bad "IT-135 a malformed anchor literal" "$b135"; fi
# IT-136 TOOL KIND: only an ed25519 key is an anchor: another type's key (an RSA, ECDSA or security-key blob, or a private-key header) refuses, and the key line always carries the type ssh-ed25519 (never a field of a literal)
b136=""
pad136() { printf '%s' "$1" | cut -c1-68 | awk '{ while (length($0) < 68) $0 = $0 "B"; print }'; }
for k136 in "AAAAB3NzaC1yc2EAAAADAQABAAABgQ|an RSA key" "AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABB|an ECDSA key" "AAAAGnNrLXNzaC1lZDI1NTE5QG9wZW5zc2guY29tAAAAI|a security-key blob" "-----BEGIN OPENSSH PRIVATE KEY-----|a private-key header"; do
  fill_anchor "$T/d83/install.empty.sh" "$T/d83/install.try.sh" "$A_P" "$(pad136 "${k136%%|*}")" "$A_F"; ranc install.try.sh; refused131; [ -z "$r131" ] || b136="$b136 [${k136#*|}:$r131]"
done
fill_anchor "$T/d83/install.empty.sh" "$T/d83/install.try.sh" "ssh-rsa" "$A_K" "$A_F"; ranc install.try.sh
l136=$(grep '^KEYFILE' "$T/log" | head -1 | cut -d'|' -f4)
{ [ "$rc" = 0 ] && [ "$(printf '%s' "$l136" | awk '{ print NF " " $1 " " $2 }')" = "3 ssh-rsa ssh-ed25519" ]; } || b136="$b136 [a principal named ssh-rsa: the key line is '$(printf '%s' "$l136" | cut -c1-60)' (want three fields, the second ssh-ed25519)]"
if [ -z "$b136" ]; then ok "IT-136 (r4) TOOL KIND: a key of another type (RSA, ECDSA, security key, private-key header) refuses anchor_unusable, and the key line always has three fields with ssh-ed25519 second"; else bad "IT-136 only an ed25519 key is an anchor" "$b136"; fi
# IT-137 a STALE anchor directory of a lost run is IGNORED: not used, not adopted, not removed (its mode and age do not matter), a link of that name is not followed
b137=""
st137="$T/.cortex-install-anchor.StaleDr1"
if mkdir -m 0700 "$st137" && printf 'decoy\n' > "$st137/key" && touch -d "3 hours ago" "$st137/key" "$st137" &&
ln -s "$T/nowhere" "$T/.cortex-install-anchor.LinkDir1" && mkdir -m 0755 "$T/.cortex-install-anchor.Mode0755" && printf 'decoy2\n' > "$T/.cortex-install-anchor.Mode0755/key"; then
ranc install.sh
kf137=$(anchor_args | cut -d'|' -f2)
{ [ "$rc" = 0 ] && case $kf137 in "$T"/.cortex-install-anchor.StaleDr1/*|"$T"/.cortex-install-anchor.LinkDir1/*|"$T"/.cortex-install-anchor.Mode0755/*) false ;; /*) true ;; *) false ;; esac && grep -qxF "KEYFILE 1|600|700|$A_P ssh-ed25519 $A_K" "$T/log"; } || b137="$b137 [the run used or adopted an old directory: rc=$rc key file '$kf137']"
{ [ "$(cat "$st137/key" 2>/dev/null)" = decoy ] && [ -L "$T/.cortex-install-anchor.LinkDir1" ] && [ "$(cat "$T/.cortex-install-anchor.Mode0755/key" 2>/dev/null)" = decoy2 ]; } || b137="$b137 [an old anchor directory was changed or removed]"
! grep -q 'anchor' "$T/out" || b137="$b137 [the output speaks of an anchor directory: $(grep 'anchor' "$T/out" | head -1 | cut -c1-80)]"
rm -rf -- "$st137" "$T/.cortex-install-anchor.Mode0755"; rm -f "$T/.cortex-install-anchor.LinkDir1"
else b137="$b137 [anchor fixture setup]"; fi
if [ -z "$b137" ]; then ok "IT-137 (r4) a stale anchor directory of a lost run (any age, any mode, or a link) is ignored: never used, never adopted, never removed, and the run uses its own"; else bad "IT-137 stale anchor directories" "$b137"; fi
# IT-138 cleanup: the anchor directory is attempted to be removed on a normal exit, on failure exits and on HUP, INT and TERM with the status preserved; NO claim for KILL (the leftover is then ignored by the next run, IT-137)
b138=""
for f138 in "success|" "trust refused (exit 4)|FAKE_TRUST_RC=4" "fetch failed (exit 5)|FAKE_FETCH_RC=5" "unpack failed (exit 5)|FAKE_UNPACK_RC=5" "load failed (exit 5)|FAKE_LOAD_RC=5"; do
  if [ -n "${f138#*|}" ]; then ranc install.sh "${f138#*|}"; else ranc install.sh; fi
  { [ -z "$(ancleft)" ] && [ -z "$(stages108)" ]; } || b138="$b138 [${f138%%|*}: anchor='$(ancleft)' stage='$(stages108)' left]"
done
int_cleanup_trap_present || b138="$b138 [no INT trap that leads to the cleanup]"
for sig in TERM HUP; do
  fresh; rm -f "$T/go"
  FAKE_FETCH_WAIT="$T/go" sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" --release 0.2.1-rc.2 --from "$T/from" > "$T/out" 2>&1 &
  ipid=$!; w=0; while ! grep -q '^STAGE ' "$T/log" 2>/dev/null && [ "$w" -lt 100 ]; do sleep 0.1; w=$((w + 1)); done
  have138=$(ancleft)
  kill -s "$sig" "$ipid" 2>/dev/null || :
  w=0; while kill -0 "$ipid" 2>/dev/null && [ "$w" -lt 50 ]; do sleep 0.1; w=$((w + 1)); done
  ign138=""
  if kill -0 "$ipid" 2>/dev/null; then
    ign=$(awk '/^SigIgn:/ { print $2 }' "/proc/$ipid/status" 2>/dev/null || :)
    if [ "$sig" = HUP ] && [ -n "$ign" ] && [ $(( 0x$ign & 1 )) -eq 1 ]; then ign138=1; else b138="$b138 [$sig: the install did not stop]"; fi
    kill -s TERM "$ipid" 2>/dev/null || :
    w=0; while kill -0 "$ipid" 2>/dev/null && [ "$w" -lt 50 ]; do sleep 0.1; w=$((w + 1)); done
    kill -0 "$ipid" 2>/dev/null && { kill -s KILL "$ipid" 2>/dev/null || :; }   # s5-never: ok the test kills ITS OWN installer process, stuck in the cell's fake fetch, by its exact pid: the hard kill is the fallback of a signal cell
  fi
  wait "$ipid" 2>/dev/null && rc=0 || rc=$?
  want138=143; [ "$sig" = HUP ] && [ -z "$ign138" ] && want138=129
  { [ -n "$have138" ] && [ -z "$(ancleft)" ] && [ -z "$(stages108)" ] && [ "$rc" = "$want138" ]; } || b138="$b138 [$sig: anchor before='$have138' after='$(ancleft)' stage='$(stages108)' status=$rc (want $want138, preserved)]"
done
# KILL: no claim (nothing can run), the leftover is what IT-137 says the next run ignores; the cell only records it and then removes it BY ITS EXACT PATH
fresh; rm -f "$T/go"
FAKE_FETCH_WAIT="$T/go" sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" --release 0.2.1-rc.2 --from "$T/from" > "$T/out" 2>&1 &
ipid=$!; w=0; while ! grep -q '^STAGE ' "$T/log" 2>/dev/null && [ "$w" -lt 100 ]; do sleep 0.1; w=$((w + 1)); done
kill -s KILL "$ipid" 2>/dev/null || :; wait "$ipid" 2>/dev/null || :   # s5-never: ok the test kills ITS OWN installer process by its exact pid: the hard kill is the point of the cell (a killed run leaves its stage, never trusted)
k138=$(ancleft); s138=$(stages108)
ranc install.sh
{ [ "$rc" = 0 ] && [ "$(anchor_args | cut -d'|' -f2 | sed 's|/key$||')" != "${k138% }" ]; } || b138="$b138 [after a KILLed run the next run failed or reused the leftover: rc=$rc]"
for d138 in $k138 $s138; do case ${d138##*/} in .cortex-install-anchor.????????|.cortex-install-stage.????????) rm -rf -- "$d138" ;; esac; done
if [ -z "$b138" ]; then ok "IT-138 (r4) the anchor directory is removed after a success, a refused trust, a failed fetch, unpack or load, and after TERM and HUP with the status preserved (143, 129); INT has the same trap; no claim for KILL (its leftover is ignored by the next run)"; else bad "IT-138 the anchor directory's cleanup" "$b138"; fi
# IT-139 TIMEOUT: install.sh itself runs NO ssh-keygen for the anchor (the fingerprint check and its time bound are rel_trust's, hue's): a logging ssh-keygen first in PATH is never called
b139=""
mkdir -p "$T/fakessh"; printf '#!/bin/sh\necho "SSH-KEYGEN $*" >> "%s"\nexit 0\n' "$T/ssh.calls" > "$T/fakessh/ssh-keygen"; chmod +x "$T/fakessh/ssh-keygen"; : > "$T/ssh.calls"
ranc install.sh PATH="$T/fakessh:$PATH"
{ [ "$rc" = 0 ] && [ ! -s "$T/ssh.calls" ]; } || b139="$b139 [rc=$rc; install.sh ran ssh-keygen: $(head -c 100 "$T/ssh.calls" | tr '\n' '|')]"
if [ -z "$b139" ]; then ok "IT-139 (r4) TIMEOUT: install.sh runs no ssh-keygen for the anchor (the fingerprint check and its time bound belong to rel_trust)"; else bad "IT-139 no tool is run for the anchor" "$b139"; fi
# IT-141 (hale 6E3-2): the exit handler maps a status the step may not give AND cleans up even when stderr cannot be written (a closed stderr made its printf fail under set -e: the handler stopped, the stage stayed, the exit status was 1)
b141=""
fresh; env FAKE_PR_RC=7 sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" --release 0.2.1-rc.2 --from "$T/from" > "$T/out" 2>&- && rc=0 || rc=$?
{ [ "$rc" = 3 ] && [ -z "$(stages108)" ] && [ -z "$(ancleft)" ]; } || b141="$b141 [prereq_run ends with 7 and stderr is closed: rc=$rc (want 3, the step's code) stage='$(stages108)' anchor='$(ancleft)' (both must be gone)]"
fresh; env FAKE_PR_RC=3 sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" --release 0.2.1-rc.2 --from "$T/from" > "$T/out" 2>&- && rc=0 || rc=$?
{ [ "$rc" = 3 ] && [ -z "$(stages108)" ] && [ -z "$(ancleft)" ]; } || b141="$b141 [prereq_run ends with 3 and stderr is closed: rc=$rc (want 3) stage='$(stages108)' anchor='$(ancleft)']"
if [ -z "$b141" ]; then ok "IT-141 (6E3-2) a step status outside its table is mapped to the step's code and the stage and the anchor directory are removed even when stderr is closed (status 7 and a passing status 3)"; else bad "IT-141 the exit handler with a closed stderr" "$b141"; fi
# IT-142 (hale 6E3-1): --check calls machine_look BARE too: an unchecked failing command in it ends the body (errexit stays on), and the status of the step ends the shell as it is
b142=""
ifd "machine_look() { echo ML >> \"\$FAKE_LOG\"; false; echo AFTER-ML >> \"\$FAKE_LOG\"; return 0; }" --check
{ [ "$rc" != 0 ] && grep -qx 'ML' "$T/log" && ! grep -q '^AFTER-ML' "$T/log"; } || b142="$b142 [--check with an unchecked failing command in machine_look: rc=$rc after-line=$(grep -c '^AFTER-ML' "$T/log" || true) (want a failure and no line after it)]"
ifd "machine_look() { return 3; }" --check
[ "$rc" = 3 ] || b142="$b142 [--check: machine_look gives 3 and the shell ends with $rc]"
ifd "machine_look() { return 0; }" --check
[ "$rc" = 0 ] || b142="$b142 [--check: a good machine look ends with $rc]"
if [ -z "$b142" ]; then ok "IT-142 (6E3-1) --check calls machine_look bare: an unchecked failing command ends it, its status ends the shell as it is (3 passes, 0 is 0)"; else bad "IT-142 --check is bare" "$b142"; fi
# IT-143 (hale note 2): run_step puts the PATH and the traps back after EVERY step (fetch, trust, unpack, load), like the two 6b steps: a step that replaces the PATH and the traps changes nothing for the rest of the run
b143=""
lane143=$(printf '%s\n' "$lane3" | grep -v '^rel_fetch()'; printf '%s\n' 'rel_fetch() { echo "FETCH $1 $2 home=no" >> "$FAKE_LOG"; echo "STAGE 0|700|${2%/*}|${2##*/}" >> "$FAKE_LOG"; echo staged > "$2/f"; PATH="/nonexistent-cv2-bin:$PATH"; export PATH; trap '"'"'echo THEIRS-FETCH >> "$FAKE_LOG"'"'"' EXIT; trap - HUP INT TERM; return 0; }' 'rel_trust() { echo "PATHAT $PATH" >> "$FAKE_LOG"; echo "TRUST $1 $2 home=no" >> "$FAKE_LOG"; echo "TRUSTARGS $#|${3-}|${4-}|${5-}" >> "$FAKE_LOG"; return "${FAKE_TRUST_RC:-0}"; }')
ifd "$lane143" --check
fresh; env FAKE_UNPACK_RC=5 sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" --release 0.2.1-rc.2 --from "$T/from" > "$T/out" 2>&1 && rc=0 || rc=$?
p143=$(sed -n 's/^PATHAT //p' "$T/log" | head -1)
case $p143 in /nonexistent-cv2-bin:*) b143="$b143 [the PATH a fetch step set was still in force at the trust step]" ;; "") b143="$b143 [the trust step did not run]" ;; esac
{ [ "$rc" = 5 ] && [ -z "$(stages108)" ] && [ -z "$(ancleft)" ] && ! grep -q '^THEIRS-FETCH' "$T/log"; } || b143="$b143 [a fetch step replaced the traps and a later unpack failed: rc=$rc (want 5) stage='$(stages108)' anchor='$(ancleft)' lane trap ran=$(grep -c '^THEIRS-FETCH' "$T/log" || true)]"
# the traps must be back BEFORE the next step can fail: a failing trust step comes right after the fetch step (prereq_run, which puts them back itself, comes later: a failure at the unpack alone hides a missing restore, IM-462)
fresh; env FAKE_TRUST_RC=4 sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" --release 0.2.1-rc.2 --from "$T/from" > "$T/out" 2>&1 && rc=0 || rc=$?
{ [ "$rc" = 4 ] && [ -z "$(stages108)" ] && [ -z "$(ancleft)" ] && ! grep -q '^THEIRS-FETCH' "$T/log"; } || b143="$b143 [a fetch step replaced the traps and the trust step failed right after: rc=$rc (want 4) stage='$(stages108)' anchor='$(ancleft)' lane trap ran=$(grep -c '^THEIRS-FETCH' "$T/log" || true)]"
if [ -z "$b143" ]; then ok "IT-143 (hale) run_step puts the PATH and the traps back after every step: a fetch step that replaces both changes nothing for the trust step, and a later failure still removes the stage and the anchor directory"; else bad "IT-143 run_step restores" "$b143"; fi
# IT-144 (sam I6E-SAM01): EVERY partial-token removal on the failure path is guarded: a LATER removal that fails (the first one, before bootstrap, succeeded) must not end the shell with a raw status; each branch ends in exit 7 with
# ONE problem: and ONE fix: line (the fault wrapper fails only the 2nd removal of the partial file, "rm=lead.token.part@2"; IT-119 faulted only the first)
b144=""
fault144() { # fault144 <what> <VAR=value>...: a run in a fresh home with the fault wrapper in PATH; wanted: exit 7, one problem line, one fix line
  f144=$1; shift; fresh
  if ! { mkdir -p "$T/h" && chmod 0700 "$T/h" && rm -f "$T/ficount".*; }; then
    b144="$b144 [$f144: fixture setup]"; return 0
  fi
  env PATH="$T/bin-fi:$PATH" FAKE_FI_COUNT="$T/ficount" "$@" sh "$install" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?
  c144=""
  [ "$rc" = 7 ] || c144="$c144 rc=$rc (want 7: $(tail -1 "$T/out" | cut -c1-70))"
  { [ "$(grep -c '^problem: ' "$T/out" || true)" = 1 ] && [ "$(grep -c '^fix: ' "$T/out" || true)" = 1 ]; } || c144="$c144 problem lines $(grep -c '^problem: ' "$T/out" || true), fix lines $(grep -c '^fix: ' "$T/out" || true)"
  # a removal that FAILED IS NOT SILENT (cox W1-109 (1)): the fix line names the leftover path and says to remove it by hand
  grep -q "^fix: .*$T/h/lead.token.part.* by hand" "$T/out" || c144="$c144 [the fix line does not name the leftover $T/h/lead.token.part and say 'by hand': $(grep '^fix: ' "$T/out" | head -1 | cut -c1-110)]"
  # sam (cox 9a94d5ca): the leftover MAY STILL HOLD A VALID SIGN-IN TOKEN (the chmod and the move can fail AFTER the bootstrap committed and cat copied the whole token, and nothing revokes it on those exits): the cause says so and
  # claims no 'no usable', 'nothing' or 'harmless'; the fix line names the path, says by hand AND says to REPLACE the sign-in with the installer's own documented way, --reissue-token (the path IT-129 tests)
  grep -q '^problem: .*MAY STILL HOLD A VALID SIGN-IN TOKEN' "$T/out" || c144="$c144 [the cause does not say the file MAY STILL HOLD A VALID SIGN-IN TOKEN: $(grep '^problem: ' "$T/out" | head -1 | cut -c1-140)]"
  ! grep -iE '^(problem|fix): .*(no usable|nothing was|harmless)' "$T/out" > /dev/null || c144="$c144 [the cause or the fix claims the leftover is harmless: $(grep -iE '^(problem|fix): .*(no usable|nothing was|harmless)' "$T/out" | head -1 | cut -c1-140)]"
  grep -q '^fix: .*by hand.*--reissue-token' "$T/out" || c144="$c144 [the fix line does not say to replace the token with --reissue-token: $(grep '^fix: ' "$T/out" | head -1 | cut -c1-160)]"
  [ -z "$c144" ] || b144="$b144 [$f144:$c144]"
}
fault144 "the token copy fails, then the removal of the partial file fails" FAKE_CAT_FAIL_ONCE=1 'FAKE_FI=rm=lead.token.part@2'
fault144 "the chmod fails, then the removal of the partial file fails" 'FAKE_FI=chmod=lead.token.part;rm=lead.token.part@2'
fault144 "the move fails, then the removal of the partial file fails" 'FAKE_FI=mv=lead.token;rm=lead.token.part@2'
if [ -z "$b144" ]; then ok "IT-144 (sam I6E-SAM01) a LATER removal of the partial token that fails, in the copy-failure, the chmod and the move branches, still ends in exit 7 with ONE cause and ONE fix (the wrapper faults the 2nd removal; list of specs and @N)"; else bad "IT-144 later partial-token removals" "$b144"; fi
# W1-300b: atomic publication succeeded but removing its private hard link
# failed. Keep the published file and stage, clean the server, release the
# reservation, and name the published file once with suffix-only stage prose.
if fresh && mkdir -p "$T/h" && chmod 700 "$T/h" && rm -f "$T/ficount".*; then
: > "$T/log214"
env PATH="$T/bin-fi:$PATH" FAKE_FI_COUNT="$T/ficount" 'FAKE_FI=rm=lead.token.part@2' CORTEX_LOG="$T/log214" sh "$install" --project p --lead lead --home "$T/h" > "$T/pub214.stdout" 2> "$T/pub214.stderr" &
pub214_pid=$!
wait "$pub214_pid" && rc=0 || rc=$?
pub214_stage=$T/h/lead.token.part.$pub214_pid
b214=''
{ [ "$rc" = 7 ] && [ -s "$T/h/lead.token" ] && [ -s "$pub214_stage" ] &&
  [ "$(stat -c '%d:%i' "$T/h/lead.token")" = "$(stat -c '%d:%i' "$pub214_stage")" ] &&
  [ ! -e "$T/state/bootstrap.token" ] && [ ! -e "$T/account-runtime/cortex-mint.reservation" ] &&
  [ "$(grep -c '^problem:' "$T/pub214.stderr")" = 1 ] && [ "$(grep -c '^fix:' "$T/pub214.stderr")" = 1 ] &&
  grep -q '^problem: .*published' "$T/pub214.stderr" && grep -q '^fix: .*[.]part.*by hand' "$T/pub214.stderr" &&
  sum_path_once "$T/pub214.stdout" '   Token file (0600):' "$T/h/lead.token" &&
  sum_path_total_once "$T/pub214.stdout" "$T/h/lead.token" &&
  sum_path_absent "$T/h/lead.token" "$T/pub214.stderr" "$T/log214" "$T/log" &&
  sum_private_files "$T/h/lead.token" "$T/pub214.stdout" "$T/pub214.stderr" "$T/log214" "$T/log"; } || b214="rc=$rc"
if [ -z "$b214" ]; then ok 'IT-214 published-stage removal failure retains both private links, one recovery pair and explicit suffix instructions'; else bad 'IT-214 published stage remains usable and explicit' "$b214"; fi
else bad "IT-214 fixture setup" "could not prepare dependent fixture"; fi

# IT-145 (sam I6E-SAM02, settled by cox W1-109 (2)): (i) the anchor directory is removed as soon as rel_trust returns (its key file is not needed after the trust step), so it is GONE when the prerequisites run and STAYS gone
# if the next step kills the shell outright (SIGKILL: no trap can run); (ii) THE STATED LIMIT, documented by a cell: a joined step that REPLACES the traps and then ends the shell from inside (an unchecked failing command) gets ITS
# trap run, not the installer's: the status is the RAW one and the STAGE is left (the anchor directory is not: it went after the trust step); the next run IGNORES the stale stage (a young one is kept and named, never trusted) and
# installs. The REAL include does not do this by its own contract (prereq_secure_call; its package step in a subshell, line 2527) and marv's cell holds it to that; the cell below keeps the limit honest for a HYPOTHETICAL callee:
# if the behaviour ever changes (a caller-side hook, say) this cell fails and the stated limit must be changed with it. No ERR hook (bash-only, a false assurance: cox W1-109).
b145=""
q145="'"
lane145a=$(printf '%s\n' "$lane3" | grep -v '^prereq_run()'; printf '%s\n' 'prereq_run() { echo "ANCHORS $(find "${T_HOME%/*}" -maxdepth 1 -name '"$q145"'.cortex-install-anchor.*'"$q145"' | wc -l | tr -d " ")" >> "$FAKE_LOG"; return 0; }')
ifd "$lane145a" --project p --lead lead --home "$T/h" --release 0.2.1-rc.2 --from "$T/from"
{ [ "$rc" = 0 ] && grep -qx 'ANCHORS 0' "$T/log"; } || b145="$b145 [the anchor directory when the prerequisites run: '$(grep '^ANCHORS' "$T/log" | head -1)' (want ANCHORS 0: removed right after rel_trust) rc=$rc]"
lane145k=$(printf '%s\n' "$lane3" | grep -v '^prereq_run()'; printf '%s\n' 'prereq_run() { echo PR >> "$FAKE_LOG"; kill -s KILL $$; }')   # s5-never: ok the fake prerequisite lane kills the INSTALLER'S OWN shell ($$) inside the test's private temp tree: the hard kill is the point of the cell (IT-145)
ifd "$lane145k" --project p --lead lead --home "$T/h" --release 0.2.1-rc.2 --from "$T/from"
{ [ "$rc" = 137 ] && [ -z "$(ancleft)" ]; } || b145="$b145 [the next step kills the shell (SIGKILL): rc=$rc (want 137) anchor directory left='$(ancleft)' (want none: it went after the trust step)]"
for s145 in $(stages108); do case ${s145##*/} in .cortex-install-stage.????????) rm -rf -- "$s145" ;; esac; done   # the stage of a KILLed run is left (no claim); removed here by its exact path
lane145p=$(printf '%s\n' "$lane3" | grep -v '^prereq_run()'; printf '%s\n' 'prereq_run() { echo PR >> "$FAKE_LOG"; trap '"$q145"'echo THEIRS-PR >> "$FAKE_LOG"'"$q145"' EXIT; trap - HUP INT TERM; false; echo AFTER-PR >> "$FAKE_LOG"; return 0; }')
ifd "$lane145p" --project p --lead lead --home "$T/h" --release 0.2.1-rc.2 --from "$T/from"
stale145=$(stages108)
{ [ "$rc" = 1 ] && grep -qx 'THEIRS-PR' "$T/log" && ! grep -q '^AFTER-PR' "$T/log" && [ -n "$stale145" ] && [ -z "$(ancleft)" ]; } || b145="$b145 [THE STATED LIMIT, a step that replaces the traps and fails before returning: rc=$rc (documented: the RAW status 1) its own trap ran=$(grep -c '^THEIRS-PR' "$T/log" || true) stage left='$stale145' (documented: left) anchor='$(ancleft)' (gone)]"
# the NEXT run, with the plain fake lane, ignores the stale stage (kept and named, never trusted) and installs
ifd "$lane3" --project p --lead lead --home "$T/h" --release 0.2.1-rc.2 --from "$T/from"
{ [ "$rc" = 0 ] && [ -d "${stale145% }" ] && grep -q 'left alone (not removed, never trusted)' "$T/out"; } || b145="$b145 [the next run with a stale stage beside it: rc=$rc stale stage still there=$([ -d "${stale145% }" ] && echo yes || echo no) named in the output=$(grep -c 'left alone (not removed, never trusted)' "$T/out" || true)]"
for s145 in $(find "$T" -maxdepth 1 -name '.cortex-install-stage.*' 2>/dev/null); do case ${s145##*/} in .cortex-install-stage.????????) rm -rf -- "$s145" ;; esac; done
if [ -z "$b145" ]; then ok "IT-145 (sam I6E-SAM02, cox W1-109) the anchor directory is gone when the prerequisites run and stays gone if the next step is KILLed; THE STATED LIMIT is documented: a step that replaces the traps and then fails from inside gets its own trap, the status is the raw one and the stage is left, and the next run ignores the stale stage (kept, named, never trusted) and installs"; else bad "IT-145 the anchor directory and the stated limit" "$b145"; fi
# IT-146 (cox ccd83501, the shape of I6E-SAM01 on the other side of the token): EVERY failure branch of mint_token removes the temporary copy of the token in the server container (the fake's $T/state/bootstrap.token) with its status
# RECONCILED: the copy is gone, and a removal that FAILS is a WARNING that names it, exactly one (the pre-clean BEFORE the bootstrap stays silent on purpose: it removes a stale copy of a crashed run, rm -f of nothing is success, and a
# container that cannot be reached fails the bootstrap call next with its own cause), never a silent leftover and never another exit code. The chmod and the move branches, and the token_left stop (the partial file cannot be removed either), did not remove the copy at all.
b146=""
fault146() { # fault146 <what> <removal of the container copy fails: 0 or 1> <VAR=value>...: a run in a fresh home with the fault wrapper in PATH; wanted: exit 7, ONE problem line, and (0) no copy left and no warning, (1) exactly ONE warning that names the copy
  f146=$1; r146=$2; shift 2; fresh
  if ! { mkdir -p "$T/h" && chmod 0700 "$T/h" && rm -f "$T/ficount".*; }; then
    b146="$b146 [$f146: fixture setup]"; return 0
  fi
  env PATH="$T/bin-fi:$PATH" FAKE_FI_COUNT="$T/ficount" FAKE_EXEC_RM_FAIL="$r146" "$@" sh "$install" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?
  c146=""
  [ "$rc" = 7 ] || c146="$c146 rc=$rc (want 7: $(tail -1 "$T/out" | cut -c1-70))"
  [ "$(grep -c '^problem: ' "$T/out" || true)" = 1 ] || c146="$c146 problem lines $(grep -c '^problem: ' "$T/out" || true)"
  n146=$(grep -c '^install: warning: the temporary copy of the token in the server container could not be removed' "$T/out" || true)
  if [ "$r146" = 1 ]; then
    [ "$n146" = 1 ] || c146="$c146 [the removal of the container copy failed and $n146 warning(s) name it (want exactly 1)]"
  else
    [ ! -e "$T/state/bootstrap.token" ] || c146="$c146 [the copy of the token in the container was LEFT]"
    [ "$n146" = 0 ] || c146="$c146 [a warning about the container copy although its removal worked: $n146]"
  fi
  [ -z "$c146" ] || b146="$b146 [$f146:$c146]"
}
FAKE_EXEC_STRICT_PATH=1; export FAKE_EXEC_STRICT_PATH   # the fake podman judges the PATH of the removal in these runs (IM-498: the helper removing another file than the token copy)
fault146 "the token copy fails" 0 FAKE_CAT_FAIL_ONCE=1
fault146 "the token copy is empty" 0 FAKE_CAT_EMPTY=1
fault146 "the chmod fails" 0 FAKE_FI=chmod=lead.token.part
fault146 "the move fails" 0 FAKE_FI=mv=lead.token
fault146 "the token copy fails and the partial file cannot be removed" 0 FAKE_CAT_FAIL_ONCE=1 'FAKE_FI=rm=lead.token.part@2'
fault146 "the chmod fails and the partial file cannot be removed" 0 'FAKE_FI=chmod=lead.token.part;rm=lead.token.part@2'
fault146 "the move fails and the partial file cannot be removed" 0 'FAKE_FI=mv=lead.token;rm=lead.token.part@2'
fault146 "the token copy fails, the removal in the container fails" 1 FAKE_CAT_FAIL_ONCE=1
fault146 "the token copy is empty, the removal in the container fails" 1 FAKE_CAT_EMPTY=1
fault146 "the chmod fails, the removal in the container fails" 1 FAKE_FI=chmod=lead.token.part
fault146 "the move fails, the removal in the container fails" 1 FAKE_FI=mv=lead.token
unset FAKE_EXEC_STRICT_PATH   # the gate is OFF again: the cases below set it per call

# W1-311: final absence cannot prove cleanup before a blocking stage deletion.
# This opt-in wrapper is private to this cell; the shared fault fake is unchanged.
mkdir -p "$T/bin-order146"
cat > "$T/bin-order146/rm" <<'ORDER146_RM'
#!/bin/sh
if [ "${FAKE_ORDER146:-0}" = 1 ]; then
  for order146_target in "$@"; do :; done
  case $order146_target in "$FAKE_ORDER146_HOME"/lead.token.part.*)
    if [ -e "$order146_target" ]; then
      : > "$FAKE_ORDER146_READY"
      while [ ! -e "$FAKE_ORDER146_GO" ]; do sleep 0.02; done
      [ "${FAKE_ORDER146_FAIL:-0}" != 1 ] || exit 1
    fi ;;
  esac
fi
exec "$FAKE_ORDER146_RM" "$@"
ORDER146_RM
chmod +x "$T/bin-order146/rm"
cat > "$T/order146.py" <<'ORDER146_PY'
from pathlib import Path
import json,os,subprocess,sys,time
install,root,name,want = sys.argv[1:]
t=Path(root); ready=Path(os.environ['FAKE_ORDER146_READY']); go=Path(os.environ['FAKE_ORDER146_GO'])
receipt={'case':name,'expected_order_assertion_exit':int(want),'boundary_seen':False}
proc=None
try:
    with (t/'out').open('wb') as out:
        proc=subprocess.Popen(['sh',install,'--project','p','--lead','lead','--home',str(t/'h')],stdout=out,stderr=subprocess.STDOUT)
        receipt['installer_pid']=proc.pid
        end=time.monotonic()+15
        while time.monotonic()<end and not ready.exists() and proc.poll() is None: time.sleep(.02)
        receipt['boundary_seen']=ready.exists()
        # Inspect the real fake-container file while the delete is held, not a
        # declaration from the observing wrapper and not the final file state.
        receipt['container_absent_before_delete']=not(t/'state/bootstrap.token').exists()
        receipt['order_assertion_exit']=0 if receipt['boundary_seen'] and receipt['container_absent_before_delete'] else 1
        go.touch()
        try: receipt['installer_exit']=proc.wait(timeout=20)
        except subprocess.TimeoutExpired:
            proc.terminate()
            try: proc.wait(timeout=2)
            except subprocess.TimeoutExpired: proc.kill();proc.wait()
            receipt['installer_timeout']=True
            receipt['installer_exit']=proc.returncode
    output=(t/'out').read_text(errors='replace')
    receipt['container_warnings']=output.count('install: warning: the temporary copy of the token in the server container could not be removed')
    receipt['problem_lines']=sum(x.startswith('problem: ') for x in output.splitlines())
    receipt['published_token']=(t/'h/lead.token').exists()
    receipt['container_absent_after_exit']=not(t/'state/bootstrap.token').exists()
    expected_fail=os.environ.get('FAKE_EXEC_RM_FAIL')=='1'
    receipt['ok']=(receipt['boundary_seen'] and receipt['installer_exit']==7 and
        receipt['order_assertion_exit']==int(want) and receipt['problem_lines']==1 and
        not receipt['published_token'] and receipt['container_warnings']==int(expected_fail) and
        receipt['container_absent_after_exit']==(not expected_fail) and not receipt.get('installer_timeout'))
finally:
    go.touch()
    if proc is not None and proc.poll() is None:
        proc.terminate()
        try:proc.wait(timeout=2)
        except subprocess.TimeoutExpired:proc.kill();proc.wait()
    (t/('order146-'+name+'.json')).write_text(json.dumps(receipt,sort_keys=True)+'\n')
    if os.environ.get('L2_KEEP'):
        dest=Path(os.environ['L2_KEEP'])/'IT-146-order';dest.mkdir(parents=True,exist_ok=True)
        (dest/(name+'.json')).write_text(json.dumps(receipt,sort_keys=True)+'\n')
print(json.dumps(receipt,sort_keys=True))
sys.exit(0 if receipt.get('ok') else 1)
ORDER146_PY
order146() {
  o146_name=$1; o146_stage=$2; o146_want=$3; shift 3
  fresh
  mkdir -p "$T/h"; chmod 0700 "$T/h"
  rm -f "$T/order146.ready" "$T/order146.go"
  if env PATH="$T/bin-order146:$T/bin-fi:$PATH" FAKE_ORDER146=1 FAKE_ORDER146_HOME="$T/h" \
    FAKE_ORDER146_RM="$T/bin-fi/rm" FAKE_ORDER146_READY="$T/order146.ready" FAKE_ORDER146_GO="$T/order146.go" \
    FAKE_ORDER146_FAIL="$o146_stage" FAKE_EXEC_STRICT_PATH=1 "$@" \
    python3 -I "$T/order146.py" "$install" "$T" "$o146_name" "$o146_want" > "$T/order146.out" 2>&1; then :
  else b146="$b146 [order $o146_name: $(tail -1 "$T/order146.out" | cut -c1-500)]"; fi
}
# The hook is inactive by default and ignores non-stage paths when enabled.
fresh; mkdir -p "$T/h"
for o146_gate in default other-path; do
  rm -f "$T/order146.ready" "$T/order146.go"
  o146_hook=0; o146_file="$T/h/lead.token.part.probe"
  if [ "$o146_gate" = other-path ]; then o146_hook=1; o146_file="$T/h/not-a-stage"; fi
  : > "$o146_file"
  if env FAKE_ORDER146="$o146_hook" FAKE_ORDER146_HOME="$T/h" FAKE_ORDER146_RM="$T/bin-fi/rm" \
    FAKE_ORDER146_READY="$T/order146.ready" FAKE_ORDER146_GO="$T/order146.go" \
    timeout --kill-after=1s 3s "$T/bin-order146/rm" -f "$o146_file" &&
    [ ! -e "$o146_file" ] && [ ! -e "$T/order146.ready" ]; then :
  else b146="$b146 [order observer gate $o146_gate]"; fi
done
order146 failed-block-delete 0 0 FAKE_CAT_FAIL_ONCE=1
order146 empty-block-delete 0 0 FAKE_CAT_EMPTY=1
order146 failed-block-refuse 1 0 FAKE_CAT_FAIL_ONCE=1
order146 empty-block-refuse 1 0 FAKE_CAT_EMPTY=1
# Negative controls: deliberately failed fake container cleanup must be rejected
# by the same held-boundary absence assertion, with the documented one warning.
order146 negative-failed-copy 1 1 FAKE_CAT_FAIL_ONCE=1 FAKE_EXEC_RM_FAIL=1
order146 negative-empty-copy 1 1 FAKE_CAT_EMPTY=1 FAKE_EXEC_RM_FAIL=1

# THE GATE ITSELF (cox 9fb1d9a1 (c), class 7: a switch nobody tested is no cell): with FAKE_EXEC_STRICT_PATH=1 the fake podman REJECTS an exec rm of another path than the token copy's (exit 1, the copy stays) and ACCEPTS the token copy's own path; WITHOUT the variable the same wrong path is accepted as before (exit 0, the copy is removed): default behaviour byte for byte as it was
gate146=""
g146() { # g146 <path> <strict: 1, or empty for the variable not set> <wanted rc> <the copy is removed afterwards: yes or no>
  fresh; echo STALE > "$T/state/bootstrap.token"
  if [ "$2" = 1 ]; then FAKE_EXEC_STRICT_PATH=1 "$T/bin/podman" exec cv2-s5-server rm -f "$1" > /dev/null 2>&1 && gr146=0 || gr146=$?
  else "$T/bin/podman" exec cv2-s5-server rm -f "$1" > /dev/null 2>&1 && gr146=0 || gr146=$?; fi
  gone146=no; [ -e "$T/state/bootstrap.token" ] || gone146=yes
  { [ "$gr146" = "$3" ] && [ "$gone146" = "$4" ]; } || gate146="$gate146 [rm $1 with the variable ${2:-not set}: rc=$gr146 (want $3), the copy removed=$gone146 (want $4)]"
}
g146 /run/cortex/bootstrap.token.x 1 1 no
g146 /run/cortex/bootstrap.token 1 0 yes
g146 /run/cortex/bootstrap.token.x "" 0 yes
b146="$b146$gate146"
if [ -z "$b146" ]; then ok "IT-146 (cox ccd83501) every failure branch of mint_token (the token copy failing or empty, the chmod, the move, and each with the partial file not removable) removes the temporary token copy in the server container, and a removal that fails is ONE warning that names it, with the same exit 7 and ONE problem line"; else bad "IT-146 the temporary token copy in the container on a failure path" "$b146"; fi

# IT-147 (kai's first real install, 9 Nov, defect J) THE ROLES STEP MAKES EVERY cortex_m_ ROLE THAT THE SHIPPED MIGRATIONS NAME. The platform migration grants to cortex_m_platform, and the roles step made only the roles of the enabled modules: every fresh install
# died in the migrate step ('role cortex_m_platform does not exist'). The consistency check: the migrations of cortex-db's platform schema and of every mod-<id> of the template's enabled list name a set of cortex_m_ roles; the SQL of the roles step must create each (NOLOGIN)
# and grant it to cortex_app. (plug-identity's role is NOT in the set: that crate is not an enabled module and not in the server features.) The scan must not be blind: it has to find the platform role and the records role.
b147=""
tpl147="$(dirname -- "$install")/cortex.toml.example"
ids147=$(sed -n 's/^enabled[[:space:]]*=[[:space:]]*\[\(.*\)\].*$/\1/p' "$tpl147" | sed -n 1p | tr -d '" ' | tr ',' ' ')
files147="$here/../crates/cortex-db/migrations/platform/0001_platform.up.sql"
for id147 in $ids147; do files147="$files147 $here/../crates/mod-$id147/migrations/0001_$id147.up.sql"; done
need147=$(for f147 in $files147; do [ ! -f "$f147" ] || cat "$f147"; done | grep -oE 'cortex_m_[a-z][a-z0-9_]*' | sort -u)
for must147 in cortex_m_platform cortex_m_records; do
  case " $(echo $need147) " in *" $must147 "*) ;; *) b147="$b147 [the scan is blind: it did not find $must147 in the migrations: $(echo $need147)]" ;; esac
done
rm -rf "$T/h"; rm -f "$T/state/roles.sql"; : > "$T/log"; run >/dev/null 2>&1 || true
for r147 in $need147; do
  grep -q "$r147 NOLOGIN" "$T/state/roles.sql" 2>/dev/null || b147="$b147 [$r147 is named by a migration and NOT created by the roles step]"
  grep -q "GRANT $r147 TO cortex_app" "$T/state/roles.sql" 2>/dev/null || b147="$b147 [$r147 is not granted to cortex_app]"
done
if [ -z "$b147" ]; then ok "IT-147 (defect J) every cortex_m_ role named by the migrations of the platform schema and of the enabled modules ($(echo $need147)) is created and granted by the roles step"; else bad "IT-147 the roles step makes every role the migrations name" "$b147"; fi
# IT-148 (kai's first real install, 9 Nov, defect I; cox's ruling, row 91327c6a, hue's measurement on test-vm's own versions) EVERY tmpfs OPTION OF compose.yml IS ONE THE REAL ROOTLESS PODMAN ACCEPTS, AND /run/cortex IS WRITABLE BY uid 10001 AND BY NO ONE ELSE.
# The line `/run/cortex:rw,noexec,nosuid,size=16m,uid=10001,gid=10001,mode=0700` made podman stop with 'unknown mount option "uid=10001": invalid mount option' (podman has no uid or gid for a tmpfs), so the server container was never created. A mode alone leaves
# the directory owned by root (10001 cannot write); mode 1777 lets anyone write; the option U makes the tmpfs belong to the container's own user (the compose `user:`), measured by hue: 10001:10001 mode 700, exec as 10002 and as the container's root with
# cap_drop ALL are both refused. podman-compose 1.5.0 passes a short-form `tmpfs:` entry to `podman run --tmpfs` as written, and its long form (`type: tmpfs`) can say only size and mode (no U): the long form is refused. The set below is what was MEASURED
# (rw, noexec, nosuid, size, mode, U) plus the plain Linux flags; anything else, uid= and gid= included, is refused. The check is STATIC (no podman runs). It reads deploy/compose.yml (the copy beside the installer under test, so a mutant of it is judged) AND the
# upgrade engine's mirrored copy, crates/cortex-upgrade/tests/fixtures/s5-compose.yml, whose /run/cortex entry must equal the compose.yml one.
chk148() { # chk148 <compose file>: one problem per line, nothing when the file is fine
  c148=0; r148=0
  while IFS= read -r l148; do
    case $l148 in
      "INLINE "*) echo "an inline tmpfs value is not read by this check (write the block list): ${l148#INLINE }" ;;
      "LONG "*) echo "the long form 'type: tmpfs' is refused (podman-compose 1.5.0 cannot say U, so the directory stays root-owned): ${l148#LONG }" ;;
      "ITEM "*)
        i148=${l148#ITEM }; p148=${i148%%:*}; o148=""; u148=0; m148=""
        case $i148 in *:*) o148=${i148#*:} ;; esac
        c148=$((c148 + 1))
        for x148 in $(printf '%s' "$o148" | tr ',' ' '); do
          case $x148 in
            U) u148=1 ;;
            rw|ro|exec|noexec|suid|nosuid|dev|nodev) ;;
            size=*) s148=${x148#size=}; case $s148 in *[kKmMgG]) s148=${s148%?} ;; esac
                    case $s148 in ""|*[!0123456789]*) echo "tmpfs size '$x148' of $p148 is not digits and one unit letter" ;; esac ;;
            mode=*) m148=${x148#mode=}
                    case $m148 in ""|*[!01234567]*) echo "tmpfs mode '$x148' of $p148 is not an octal number" ;; esac
                    case ${#m148} in 3|4) ;; *) echo "tmpfs mode '$x148' of $p148 is not 3 or 4 octal digits" ;; esac ;;
            *) echo "tmpfs option '$x148' of $p148 is not in the set rootless podman 5.8.2 accepts (it refuses uid= and gid=)" ;;
          esac
        done
        if [ "$p148" = /run/cortex ]; then
          r148=1
          [ "$u148" = 1 ] || echo "the /run/cortex tmpfs must carry U (without it the directory is owned by root and the server user 10001 cannot write)"
          case $m148 in 700|0700) ;; *) echo "the /run/cortex tmpfs must have mode 0700 (writable by 10001 and no one else), not '${m148:-none}'" ;; esac
        fi ;;
    esac
  done <<EOT
$(awk '
/^[[:space:]]*tmpfs:[[:space:]]*(#.*)?$/ { in_t = 1; next }
/^[[:space:]]*tmpfs:/ { print "INLINE " $0; in_t = 0; next }
in_t && /^[[:space:]]*-[[:space:]]/ { s = $0; sub(/^[[:space:]]*-[[:space:]]+/, "", s); sub(/[[:space:]]+#.*$/, "", s); sub(/[[:space:]]+$/, "", s); gsub(/"/, "", s); gsub(/\047/, "", s); print "ITEM " s; next }
in_t && /^[[:space:]]*(#.*)?$/ { next }
{ in_t = 0 }
/^[[:space:]]*(-[[:space:]]+)?type:[[:space:]]*["\047]?tmpfs/ { print "LONG " $0 }
' "$1")
EOT
  [ "$c148" -ge 2 ] || echo "the check is blind: it found $c148 tmpfs entries, and compose.yml has at least two"
  [ "$r148" = 1 ] || echo "the check found no /run/cortex tmpfs entry"
}
b148=""
cf148="$(dirname -- "$install")/compose.yml"
fx148="$here/../crates/cortex-upgrade/tests/fixtures/s5-compose.yml"
for f148 in "$cf148" "$fx148"; do
  if [ ! -f "$f148" ]; then b148="$b148 [$f148 is not there]"; continue; fi
  p148=$(chk148 "$f148"); [ -z "$p148" ] || b148="$b148 [$(basename -- "$f148"): $(printf '%s' "$p148" | tr '\n' ';')]"
done
if [ -f "$cf148" ] && [ -f "$fx148" ]; then
  [ "$(grep -F '/run/cortex:' "$cf148" | sed 's/^[[:space:]]*//')" = "$(grep -F '/run/cortex:' "$fx148" | sed 's/^[[:space:]]*//')" ] || b148="$b148 [the /run/cortex entry of the upgrade fixture is not the one of compose.yml]"
  # the checker itself is proved on faulty copies of the real file: each must be refused with its own reason (a checker that reads nothing, or passes everything, fails here)
  neg148() { # neg148 <name> <text the problem list must contain>: judge $T/neg148.yml, which must differ from the real file (an edit that changed nothing proves nothing)
    if cmp -s "$cf148" "$T/neg148.yml"; then b148="$b148 [faulty copy $1: the edit changed nothing]"; return 0; fi
    n148=$(chk148 "$T/neg148.yml" | tr '\n' ';'); case $n148 in *"$2"*) ;; *) b148="$b148 [faulty copy $1 was not refused with '$2': ${n148:-nothing reported}]" ;; esac
  }
  sed 's#size=16m,mode=0700,U#size=16m,uid=10001,gid=10001,mode=0700#' "$cf148" > "$T/neg148.yml"; neg148 uid-gid 'uid=10001'
  sed 's#mode=0700,U#mode=0700#' "$cf148" > "$T/neg148.yml"; neg148 no-U 'must carry U'
  sed 's#mode=0700,U#mode=0770,U#' "$cf148" > "$T/neg148.yml"; neg148 mode-0770 'must have mode 0700'
  sed 's#mode=0700,U#mode=1777,U#' "$cf148" > "$T/neg148.yml"; neg148 mode-1777 'must have mode 0700'
  sed 's#mode=0700,U#mode=0700,U,noatime#' "$cf148" > "$T/neg148.yml"; neg148 unknown-option "option 'noatime'"
  sed 's#size=16m#size=16x#' "$cf148" > "$T/neg148.yml"; neg148 bad-size "size 'size=16x'"
  sed '/\/run\/cortex:/d' "$cf148" > "$T/neg148.yml"; neg148 no-run-cortex 'no /run/cortex tmpfs entry'
  { cat "$cf148"; printf '      - type: tmpfs\n        target: /run/cortex\n'; } > "$T/neg148.yml"; neg148 long-form "long form 'type: tmpfs'"
  sed 's#^\([[:space:]]*\)tmpfs:$#\1tmpfs: [/run/cortex:rw]#' "$cf148" > "$T/neg148.yml"; neg148 inline 'inline tmpfs value'
  printf 'services: {}\n' > "$T/neg148.yml"; neg148 none 'the check is blind'
  rm -f "$T/neg148.yml"
fi
if [ -z "$b148" ]; then ok "IT-148 (defect I) every tmpfs option of deploy/compose.yml and of the upgrade fixture is one rootless podman accepts, /run/cortex carries U and mode 0700 (10001 only), the two files agree, and ten faulty copies are each refused"; else bad "IT-148 every tmpfs option is one rootless podman accepts and /run/cortex is 10001 only" "$b148"; fi
# IT-149 (kai's full pass on a fresh Rocky 10.2, 9 Nov, defect N; cox row c2ca1f6c) A GOOD INSTALL PRINTS ITS SUMMARY, NOT A 'problem:' LINE. install.sh called say_done with THREE arguments (version, endpoint, token file); the REAL say_done (deploy/install-prereq.sh) wants a
# FOURTH, the log path (from the argument or CORTEX_LOG), and refuses an empty one: 'problem: completion summary usage requires a version, endpoint, token path and log path / fix: provide all four values and print the summary again'. Every good install, first and second run, ended
# with that line (exit 0) and gave the user NO endpoint and NO token path (kai's t7 and t8). IT-109 and IT-110 stayed green because their lane has a FAKE say_done that counts three arguments. This cell puts the REAL say_done, prereq_info and prereq_msg (cut out of the real include by
# function name, nothing else of it: no machine is looked at or changed) into the lane of a first install, a first install with CORTEX_LOG set, a release install and a second run on a finished install (with and without CORTEX_LOG): each must print the version, the endpoint, the token file and the log
# line (the installer keeps no log of its own: the word none, or the file CORTEX_LOG names), and NO problem: or fix: line. The guard of say_done is not loosened (it is marv's file and stays as it is).
b149=""
fn149() { awk -v n="$1" '$0 ~ "^" n "\\(\\) \\{" { p = 1 } p { print } p && /^}/ { exit }' "$here/../deploy/install-prereq.sh"; }
sd149=$(for f149 in prereq_msg prereq_info say_done; do fn149 $f149; done)
for f149 in prereq_msg prereq_info say_done; do printf '%s\n' "$sd149" | grep -q "^$f149() {" || b149="$b149 [the scan is blind: $f149 was not found in the real include]"; done
printf '%s\n' "$sd149" | grep -q 'completion summary usage' || printf '%s\n' "$sd149" | grep -q 'done_usage' || b149="$b149 [the scan is blind: the usage refusal is not in the text cut out]"
lane149="$(lane 'return 0;' 'return 0;')
$sd149"
chk149() { # chk149 <what> <the text after 'Install log: '>: judge one finished run ($rc and $T/out)
  e149=""
  [ "$rc" = 0 ] || e149="$e149 [rc=$rc]"
  grep -qE '^Endpoint: http://127\.0\.0\.1:[0123456789]+/v2/health/ready$' "$T/out" || e149="$e149 [no Endpoint line]"
  sum_path_once "$T/out" '   Token file (0600):' "$T/h/lead.token" || e149="$e149 [no standalone Token file pair]"
  sum_path_total_once "$T/out" "$T/h/lead.token" 'Install log:' "$2" || e149="$e149 [token path is missing repeated folded or inline]"
  grep -q '^Installed Cortex version: ' "$T/out" || e149="$e149 [no version line]"
  if [ "$2" = none ]; then
    grep -qxF 'Install log: none' "$T/out" || e149="$e149 [no no-log fact]"
    sum_private_files "$T/h/lead.token" "$T/out" "$T/log" || e149="$e149 [credential fragment in public output/log]"
  else
    sum_path_once "$T/out" 'Install log:' "$2" && sum_path_total_once "$T/out" "$2" '   Token file (0600):' "$T/h/lead.token" || e149="$e149 [log path is not standalone exactly once]"
    sum_private_files "$T/h/lead.token" "$T/out" "$T/log" "$2" || e149="$e149 [credential fragment in public output/log]"
  fi
  ! grep -q '^problem: ' "$T/out" || e149="$e149 [a problem line after the install: $(grep '^problem: ' "$T/out" | head -1 | cut -c1-90)]"
  ! grep -q '^fix: ' "$T/out" || e149="$e149 [a fix line after the install]"
  [ -z "$e149" ] || b149="$b149 [$1:$e149]"
}
unset CORTEX_LOG
ifd "$lane149" --project p --lead lead --home "$T/h" --port 8777; chk149 "first install, no log kept" none
rm -f "$T/log149"; export CORTEX_LOG="$T/log149"; ifd "$lane149" --project p --lead lead --home "$T/h" --port 8777; unset CORTEX_LOG; chk149 "first install, CORTEX_LOG set" "$T/log149"
[ -s "$T/log149" ] || b149="$b149 [CORTEX_LOG was set and the installer wrote nothing to it]"
ifd "$lane149" --check
if rel_install; then chk149 "release install" none; else b149="$b149 [release fixture setup]"; fi
: > "$T/log"; run3; chk149 "second run on a finished install" none
grep -q 'already installed' "$T/out" || b149="$b149 [the second run did not say it is already installed]"
rm -f "$T/log149b"; export CORTEX_LOG="$T/log149b"; : > "$T/log"; run3; unset CORTEX_LOG; chk149 "second run, CORTEX_LOG set" "$T/log149b"
if [ -z "$b149" ]; then ok "IT-149 (defect N) with the REAL say_done a first install, a first install with CORTEX_LOG set, a release install and a second run (with and without CORTEX_LOG) each print the version, the endpoint, the token file and the install log line (none, or the file CORTEX_LOG names) and no problem: or fix: line"; else bad "IT-149 a good install prints its summary and no problem line" "$b149"; fi
# W1-298 SUM-7: join the WHOLE real prerequisite include after a completed
# install. Its unperformed prereq_compose_path correctly refuses in a new shell;
# only the recorded provider checked by install_finished can justify the receipt.
b203=''
ifd "$lane149" --project p --lead lead --home "$T/h" --no-show-credentials
[ "$rc" = 0 ] || b203="$b203 [seed rc=$rc]"
cp "$here/../deploy/install-prereq.sh" "$T/d83/install-prereq.sh"
fp203=$(fp full)
: > "$T/log"; run3 --no-show-credentials
[ "$rc" = 0 ] && [ "$fp203" = "$(fp full)" ] &&
grep -q 'is installed and running' "$T/out" && ! grep -q 'FAILED\|^problem:' "$T/out" &&
grep -q 'Prerequisites ... ok (already done;' "$T/out" &&
! grep -q '^ML\|^PR\|^compose up\|^exec .* bootstrap ' "$T/log" || b203="$b203 [real-include repeat rc=$rc]"
sum_path_once "$T/out" '   Token file (0600):' "$T/h/lead.token" &&
sum_path_total_once "$T/out" "$T/h/lead.token" &&
sum_private_files "$T/h/lead.token" "$T/out" "$T/log" || b203="$b203 [path/value guard]"
if [ -z "$b203" ]; then ok 'IT-203 SUM-7 real joined already-done prerequisites reuse the checked recorded provider with no writes'; else bad 'IT-203 real prerequisite repeat' "$b203"; fi

# SUM-21: run the actual formatter/receipt functions cut from the candidate.
# The values are public fixtures. A separate Python oracle checks word integrity,
# column bounds, exact continuation indent and exact compact hash/fingerprint.
python3 -I - "$install" "$T/format-functions" <<'PY203'
import pathlib,re,sys
s=pathlib.Path(sys.argv[1]).read_text(); wanted=['summary_line','summary_record']
chunks=[]
for n in wanted:
 m=re.search(r'^'+n+r'\(\) \{\n.*?^}$',s,re.M|re.S)
 if not m: raise SystemExit('missing real formatter '+n)
 chunks.append(m[0])
pathlib.Path(sys.argv[2]).write_text('\n'.join(chunks)+'\n')
PY203
(
 . "$T/format-functions"
 summary_steps=''
 summary_record 'Release fetched and verified' ok 'already done; verified signer SHA256:ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopq'
 summary_record 'Images loaded' ok 'server sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef database sha256:fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210'
 summary_record Prerequisites ok "changes requested: podman
podman-compose
coreutils; epel linger"
 summary_record 'Checked this machine' ok 'Linux Rocky Enterprise Operating System with a deliberately long descriptive name 10.2 x86_64'
 summary_line "$summary_steps"
) > "$T/format-output"
r204=0
python3 -I - "$T/format-output" <<'PY204' || r204=$?
import pathlib,re,sys
s=pathlib.Path(sys.argv[1]).read_text(); lines=s.splitlines()
expected=[
('Release fetched and verified','already done; verified signer SHA256:ABCD...pq'),
('Images loaded','server sha256:0123456789ab... database sha256:fedcba987654...'),
('Prerequisites','changes requested: podman, podman-compose, coreutils; epel linger'),
('Checked this machine','Linux Rocky Enterprise Operating System with a deliberately long descriptive name 10.2 x86_64')]
assert lines and all(0x20<=ord(c)<=0x7e or c=='\n' for c in s)
assert all(len(line)<=78 for line in lines), 'column bound'
i=0
for label,value in expected:
 prefix='   '+label+' ... ok ('
 assert lines[i].startswith(prefix), (label,lines[i])
 pieces=[lines[i][len(prefix):]];i+=1
 while i<len(lines) and not re.match(r'^   \S',lines[i]):
  assert lines[i].startswith(' '*len(prefix)), 'continuation indent'
  pieces.append(lines[i][len(prefix):]); i+=1
 assert ' '.join(pieces)==value+')', (label,pieces)
assert i==len(lines)
PY204
if [ "$r204" = 0 ]; then ok 'IT-204 SUM-21 wraps whole words at value indent, compacts hashes and fingerprints and joins list values'; else bad 'IT-204 readable summary values' "oracle rc=$r204"; fi

# D8-MIKE-09: realistic image references must retain the complete specified
# compact digest even when a registry/repository prefix needs shortening.
(
 . "$T/format-functions"
 summary_line '   Images loaded ... ok (server localhost/cortex-server@sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef; database registry.example.invalid/department/very-long-database-repository/postgres@sha256:fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210)'
) > "$T/image-format-output"
r208=0
python3 -I - "$T/image-format-output" <<'PY208' || r208=$?
import pathlib,re,sys
lines=pathlib.Path(sys.argv[1]).read_text().splitlines()
assert all(len(line)<=78 for line in lines)
prefix='   Images loaded ... ok ('
assert lines[0].startswith(prefix)
assert all(line.startswith(' '*len(prefix)) for line in lines[1:])
text=' '.join(line.strip() for line in lines)
m=re.search(r'server (\S+); database (\S+)\)',text)
assert m and m[1].endswith('@sha256:0123456789ab...') and m[2].endswith('@sha256:fedcba987654...'), 'complete compact digest suffixes'
assert m[1].split('@sha256:')[0].endswith('cortex-server') and m[2].split('@sha256:')[0].endswith('postgres'), 'repository tail kept'
assert '0123456789abcdef' not in text and 'fedcba9876543210' not in text
PY208
if [ "$r208" = 0 ]; then ok 'IT-208 real image references preserve all 12 compact digest digits while long repository prefixes shorten'; else bad 'IT-208 compact named image references' "oracle rc=$r208"; fi

# SUM-22 and one-summary integration: substitute only /etc/os-release in a private
# deployment copy. No product environment hook can choose another machine root.
ifd "$lane149" --check
printf 'NAME="Rocky Linux"\nVERSION_ID="10.2"\n' > "$T/os-release205"
python3 -I - "$T/d83/install.sh" "$T/os-release205" <<'PY205'
import pathlib,sys
p=pathlib.Path(sys.argv[1]);s=p.read_text()
p.write_text(s.replace('/etc/os-release',sys.argv[2]))
PY205
fresh; run3 --no-show-credentials
b205=''
grep -qxF " Linux, Rocky Linux 10.2, $(uname -m)" "$T/out" || b205="$b205 [OS header]"
r205=0
python3 -I - "$T/out" <<'PY205' || r205=$?
import pathlib,sys
s=pathlib.Path(sys.argv[1]).read_text();lines=s.splitlines()
headers=[i for i,line in enumerate(lines) if line.startswith(' Cortex ') and ' is installed' in line]
assert len(headers)==1
h=headers[0]
for fact in ['Installed Cortex version:','Endpoint:','Install log:']:
 hits=[i for i,line in enumerate(lines) if line.startswith(fact)]
 assert len(hits)==1 and hits[0]>h, (fact,hits)
assert not any('install: done.' in line for line in lines)
PY205
[ "$r205" = 0 ] && [ "$rc" = 0 ] || b205="$b205 [sole-summary oracle, rc=$rc]"
if [ -z "$b205" ]; then ok 'IT-205 SUM-22 names OS-release system and architecture and renders completion facts only inside one summary'; else bad 'IT-205 OS header and sole summary' "$b205"; fi

# W1-300c: the maximum valid version and an overlong OS word in actual headers.
version226=$(python3 -I -c 'print("0.2.2-"+"a"*58)')
python3 -I - "$T/os-release205" <<'HEADER226'
import pathlib,sys
pathlib.Path(sys.argv[1]).write_text('NAME="'+('W'*96)+'"\nVERSION_ID="10.2"\n')
HEADER226
fresh; : > "$T/header226.log"
CORTEX_VERSION=$version226 CORTEX_LOG=$T/header226.log run3 --no-show-credentials
r226=0
python3 -I - "$T/out" "$T/header226.log" "$version226" <<'HEADER226' || r226=$?
import pathlib,sys
s=pathlib.Path(sys.argv[1]).read_text();lines=s.splitlines();v=sys.argv[3]
assert ' Cortex '+v[:16]+'... is installed and running' in lines
assert any(line.startswith(' Linux, '+('W'*16)+'... 10.2, ') for line in lines)
assert all(len(line)<=78 for line in lines if line.startswith((' Cortex ',' Linux, ')))
assert v in pathlib.Path(sys.argv[2]).read_text(), 'full version in receipt log'
HEADER226
if [ "$r226" = 0 ] && [ "$rc" = 0 ]; then ok 'IT-226 maximum version and long OS word shorten to first16 plus ellipsis in actual headers, with full receipt retained'; else bad 'IT-226 readable bounded header words' "oracle rc=$r226 install=$rc"; fi

# SWEEP-MIKE-03: the fake prerequisite deliberately enables tracing after
# boot. All database and client values are virtual test-stack credentials.
trace_lane="$(lane 'return 0;' 'set -x; return 0;')
$sd149"
ifd "$trace_lane" --check
trace_bad=0; : > "$T/trace.log"
for trace_mode in fresh repeat reissue; do
  if [ "$trace_mode" = reissue ]; then
    FAKE_BOOT_TOKEN=$sum_reissue_value CORTEX_LOG="$T/trace.log" sh -x "$T/d83/install.sh" --project p --lead lead --home "$T/h" --reissue-token --no-show-credentials > "$T/trace.stdout" 2> "$T/trace.stderr" && trace_rc=0 || trace_rc=$?
  else
    FAKE_BOOT_TOKEN=$sum_fake_token CORTEX_LOG="$T/trace.log" sh -x "$T/d83/install.sh" --project p --lead lead --home "$T/h" --no-show-credentials > "$T/trace.stdout" 2> "$T/trace.stderr" && trace_rc=0 || trace_rc=$?
  fi
  [ "$trace_rc" = 0 ] || trace_bad=$((trace_bad+1))
  for trace_secret in "$T/state/secret-cv2-s5-db-superuser" "$T/state/secret-cv2-s5-owner-password" "$T/state/secret-cv2-s5-app-password" "$T/h/lead.token" "$T/state/active.token"; do
    sum_private_files "$trace_secret" "$T/trace.stdout" "$T/trace.stderr" "$T/trace.log" "$T/log" || trace_bad=$((trace_bad+1))
  done
  # Each run is scanned now, before either channel is overwritten.
done
if [ "$trace_bad" = 0 ]; then ok 'IT-215 shell tracing before and after boot exposes no database or client credential on fresh repeat reissue channels'; else bad 'IT-215 tracing of synthetic credentials' "$trace_bad guards"; fi

# SWEEP-MIKE-04: observe only whether a generated 64-hex synthetic database
# value reached a child through inherited exported val; never dump an environment.
ifd "$lane149" --check
: > "$T/env-secret.log"
val=PUBLIC_CANARY FAKE_INSPECT_SECRET_ENV=1 CORTEX_LOG="$T/env-secret.log" sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" --no-show-credentials > "$T/env-secret.stdout" 2> "$T/env-secret.stderr" && env_secret_rc=0 || env_secret_rc=$?
env_secret_bad=0
[ "$env_secret_rc" = 0 ] && [ ! -e "$T/state/secret-env-leak" ] || env_secret_bad=1
for env_secret_file in "$T/state/secret-cv2-s5-db-superuser" "$T/state/secret-cv2-s5-owner-password" "$T/state/secret-cv2-s5-app-password" "$T/h/lead.token"; do
  sum_private_files "$env_secret_file" "$T/env-secret.stdout" "$T/env-secret.stderr" "$T/env-secret.log" "$T/log" || env_secret_bad=1
done
if [ "$env_secret_bad" = 0 ]; then ok 'IT-216 inherited exported val never carries a generated database credential into a child environment'; else bad 'IT-216 exported secret variable' "rc=$env_secret_rc leak-marker=$([ -e "$T/state/secret-env-leak" ] && echo present || echo absent)"; fi

r224=0
python3 -I - "$unisolated_install" "$T" <<'RUNTIME_FACTORY' || r224=$?
import os,pathlib,re,subprocess,sys
s=pathlib.Path(sys.argv[1]).read_text()
m=re.search(r'^mint_runtime_dir\(\) \{\n.*?^}$',s,re.M|re.S)
if not m: raise SystemExit(1)
e=dict(os.environ);e.update(HOME=sys.argv[2]+'/different-home',XDG_RUNTIME_DIR=sys.argv[2]+'/different-runtime',CORTEX_HOME=sys.argv[2]+'/different-install',SUM_ACCOUNT_RUNTIME=sys.argv[2]+'/untrusted-factory-value')
r=subprocess.run(['sh'],input=m[0]+'\nmint_runtime_dir\n',text=True,capture_output=True,env=e)
uid=subprocess.check_output(['id','-u'],text=True).strip()
assert r.returncode==0 and r.stdout=='/run/user/'+uid+'\n' and not r.stderr
RUNTIME_FACTORY
if [ "$r224" = 0 ]; then ok 'IT-224 production account-runtime factory is fixed by uid and ignores HOME XDG_RUNTIME_DIR and install home'; else bad 'IT-224 immutable runtime factory' "oracle rc=$r224"; fi

ifd "$lane149" --project p --lead lead --home "$T/h" --no-show-credentials
repeat219_bad=0
[ "$rc" = 0 ] || repeat219_bad=1
repeat219_old=$(cksum "$T/h/lead.token")
cp "$here/../deploy/install-prereq.sh" "$T/d83/install-prereq.sh"
mkdir -p "$T/os219/etc"; printf 'ID=unsupported-fixture\nVERSION_ID="10.2"\n' > "$T/os219/etc/os-release"
printf '\nprereq_system_root() { printf '\''%%s\\n'\'' '\''%s'\''; }\n' "$T/os219" >> "$T/d83/install-prereq.sh"
# Keep this focused on one REAL prerequisite refusal. The full machine checker
# legitimately reports several unrelated issues on the test host; its own suite
# owns those checks. This private entry uses the real OS predicate/message.
cat >> "$T/d83/install-prereq.sh" <<'REFUSAL219'
machine_look() {
  if ! prereq_os_ok "$(prereq_system_root)/etc/os-release"; then
    prereq_msg os_unsupported
    return "$EX_MACHINE"
  fi
  return 0
}
REFUSAL219
repeat219_rc=setup
if chmod 644 "$T/h/bin/cortex-dump"; then
: > "$T/repeat219.log"
CORTEX_LOG=$T/repeat219.log sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" --no-show-credentials > "$T/repeat219.stdout" 2> "$T/repeat219.stderr" && repeat219_rc=0 || repeat219_rc=$?
{ [ "$repeat219_rc" = 2 ] && [ "$repeat219_old" = "$(cksum "$T/h/lead.token")" ] && [ "$(cat "$T/state/count")" = 1 ] &&
  [ "$(grep -c '^problem:' "$T/repeat219.stderr")" = 1 ] && [ "$(grep -c '^fix:' "$T/repeat219.stderr")" = 1 ] &&
  sum_no_file_notice "$T/repeat219.stdout" 0 &&
  sum_path_absent "$T/h/lead.token" "$T/repeat219.stdout" "$T/repeat219.stderr" "$T/repeat219.log" "$T/log" &&
  sum_private_files "$T/h/lead.token" "$T/repeat219.stdout" "$T/repeat219.stderr" "$T/repeat219.log" "$T/log"; } || repeat219_bad=1
else repeat219_bad=1; fi
if [ "$repeat219_bad" = 0 ]; then ok 'IT-219 real-include refusal after a failed finished check claims no old credential and says no server mint ran'; else bad 'IT-219 rerun refusal after selecting old token' "rc=$repeat219_rc"; fi

# IT-150 to IT-152 (kai's full pass on a fresh Rocky 10.2, 9 Nov, defect M; cox's ruling c2ca1f6c, option 1) A FAILED MIGRATION REMOVES THE SERVER CONTAINER THAT COMPOSE HAD STARTED, SO THE MESSAGE IS TRUE AND THE RETRY IT RECOMMENDS PASSES THE PORT GUARD.
# podman-compose 1.5.0 does not wait for migrate (depends_on has no effect there): after a failed migration it HAD started cv2-s5-server (Up, health 503, rootlessport on 127.0.0.1:8601) while the installer said 'the server was not started', and the
# retry died on 'the selected port 8601 is already listening' (kai's t4). The ruling: in the migrate_failed branch ONLY, remove that container BY EXACT NAME with `podman rm -f cv2-s5-server` (NOT compose down: the message points at podman logs
# cv2-s5-migrate, which down would delete), but only when it is this installer's (the label cortexv2=dev that compose.yml sets), say what was done in the message, and never touch another container. The database container stays; marv's port guard is unchanged.
# The fake podman models the server container (exists after a compose up, its label, `rm`, a listener on the port while it is up); the port guard is the REAL prereq_port_free (and prereq_msg) cut out of the real include, against a fake ss that sees that listener.
mrun() { ( [ "$#" = 0 ] || export "$@"; run ); }   # a run with the given VAR=value settings, in a subshell: nothing leaks into the next cell
mkdir -p "$T/ssbin"
cat > "$T/ssbin/ss" <<'SS'
#!/bin/sh
# the fake of `ss -H -ltn`: one listening line for the port of the fake stack while its server container is up
if [ -f "$FAKE_STATE/server-up" ] && [ -f "$FAKE_STATE/port" ]; then echo "LISTEN 0 4096 127.0.0.1:$(cat "$FAKE_STATE/port") 0.0.0.0:*"; fi
exit 0
SS
chmod +x "$T/ssbin/ss"
pf150=$(fn149 prereq_msg; fn149 prereq_port_free)
guard150() { # the status of the REAL prereq_port_free for the port of the fake stack: 0 free, 1 busy (listening)
  ( PATH="$T/ssbin:$PATH"; prereq_system_root() { :; }; say() { :; }; warn() { :; }; eval "$pf150"; prereq_port_free "$(cat "$FAKE_STATE/port")" >/dev/null 2>&1; echo $? )
}
pm150() { # pm150 <what>: PRINTS what is wrong with the run just made (nothing when it is right): exit 6 (EX_START), one problem: and one fix: line, the installer's own closing line, no token made, nothing stopped or taken down
  [ "$rc" = 6 ] || printf ' [%s: rc=%s, want 6]' "$1" "$rc"
  { [ "$(grep -c '^problem: ' "$T/out" || true)" = 1 ] && [ "$(grep -c '^fix: ' "$T/out" || true)" = 1 ]; } || printf ' [%s: not exactly one problem: and one fix: line]' "$1"
  tail -1 "$T/out" | grep -q '^install: the migrations failed' || printf ' [%s: the last line is not the installers own install: the migrations failed ...: %s]' "$1" "$(tail -1 "$T/out" | cut -c1-80)"
  { [ "$(boots)" = 0 ] && [ -z "$(ls "$T/h"/lead.token* 2>/dev/null || true)" ]; } || printf ' [%s: a token was made]' "$1"
  ! grep -E '^(stop|kill|container (stop|kill|rm)|pod |compose .* down)' "$T/log" | grep -q . || printf ' [%s: something was stopped or taken down: %s]' "$1" "$(grep -E '^(stop|kill|container (stop|kill|rm)|pod |compose .* down)' "$T/log" | head -1)"
  return 0
}
# IT-150..IT-152 (M v2: cox a0c75b58, sam NMR-SAM-02): the inspected object is BOUND to the removed object (one inspect call by name answers the full id AND the labels; the removal is by that FULL ID, never by name), and what makes the container ours is PER-INSTALL: the
# label cortexv2=dev AND the compose project label equal to this installer's project (cv2-s5) AND the compose working directory label equal to the physical directory of THIS install's compose file (podman-compose 1.5.0 records them: probe-m2.out). A container with only
# a matching cortexv2 label (a user's own cv2-s5-server) is never removed.
id150=0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef   # the fake's server container id: 64 hex
# IT-150: the server container compose started is OURS: ONE inspect (id and labels), then ONE removal BY THAT ID, once; nothing else is removed; the message says so; the retry passes the REAL port guard
b150=""; b151=""; b152=""
fresh; mrun FAKE_MIGRATE_FAIL=1 && rc=0 || rc=$?
b150="$b150$(pm150 ours)"
[ "$(grep -c '^container inspect ' "$T/log" || true)" = 1 ] && grep -qE '^container inspect --format .*\.Id.*"cortexv2".*com\.docker\.compose\.project".*working_dir".* cv2-s5-server$' "$T/log" || b150="$b150 [ours: not exactly ONE inspect by name asking for the id and the labels: $(grep '^container inspect ' "$T/log" | cut -c1-120 | tr '
' '|')]"
[ "$(grep -c '^rm ' "$T/log" || true)" = 1 ] && grep -qx "rm -f $id150" "$T/log" || b150="$b150 [ours: the log does not have exactly one 'rm -f <the full id>': $(grep '^rm ' "$T/log" | tr '
' '|')]"
! grep '^rm ' "$T/log" | grep -q 'cv2-s5-server' || b150="$b150 [ours: the removal named the container by NAME: $(grep '^rm ' "$T/log" | head -1)]"
l_lab=$(grep -n '^container inspect ' "$T/log" | head -1 | cut -d: -f1); l_rm=$(grep -n '^rm -f ' "$T/log" | head -1 | cut -d: -f1)
{ [ -n "$l_lab" ] && [ -n "$l_rm" ] && [ "$l_lab" -lt "$l_rm" ]; } || b150="$b150 [ours: the container was not inspected before it was removed (inspect at '$l_lab', rm at '$l_rm')]"
grep -q '^problem: .*migrations failed.*removed' "$T/out" || b150="$b150 [ours: the problem line does not say the server container was removed: $(grep '^problem: ' "$T/out" | cut -c1-120)]"
! grep -q 'was not started' "$T/out" || b150="$b150 [ours: the message still says the server was not started]"
[ "$(guard150)" = 0 ] || b150="$b150 [ours: after the failure the REAL port guard still sees the port listening: a retry would die on it]"
: > "$T/log"; mrun && rc=0 || rc=$?    # the retry the message recommends (the migration passes now)
{ [ "$rc" = 0 ] && [ -s "$T/h/lead.token" ]; } || b150="$b150 [ours: the retry did not finish: rc=$rc $(tail -1 "$T/out" | cut -c1-80)]"
! grep -q '^rm ' "$T/log" || b150="$b150 [ours: a run whose migration passed removed a container: $(grep '^rm ' "$T/log" | head -1)]"
# the control: when the removal fails the listener stays, and the REAL guard must see it (so that this cell is not blind to the port)
fresh; mrun FAKE_MIGRATE_FAIL=1 FAKE_RM_FAIL=1 && rc=0 || rc=$?
[ "$(guard150)" = 1 ] || b150="$b150 [control: with the container still up the REAL port guard did not see the listener: the cell is blind]"
if [ -z "$b150" ]; then ok "IT-150 (defect M v2) after a failed migration the server container compose had started is inspected ONCE by name (full id and labels together) and removed BY THAT ID, once, never by name; nothing else is stopped or removed, the problem line says so, no token is made, and the retry passes the REAL port guard and finishes"; else bad "IT-150 a failed migration removes the server container compose started, by its id" "$b150"; fi
# IT-151: a container named cv2-s5-server that is NOT this install's is never removed: another cortexv2 value or none, no compose project label or another project, no working directory label or another directory
for c151 in "FAKE_SERVER_LABEL=" "FAKE_SERVER_LABEL=other" "FAKE_SERVER_PROJECT=" "FAKE_SERVER_PROJECT=other-project" "FAKE_SERVER_WORKDIR=" "FAKE_SERVER_WORKDIR=/somewhere/else"; do
  fresh; mrun FAKE_MIGRATE_FAIL=1 "$c151" && rc=0 || rc=$?
  b151="$b151$(pm150 "not ours ($c151)")"
  [ "$(grep -c '^rm ' "$T/log" || true)" = 0 ] || b151="$b151 [$c151: a container that is not this install's was removed: $(grep '^rm ' "$T/log" | head -1)]"
  grep -q '^problem: .*did not make.*left alone' "$T/out" || b151="$b151 [$c151: the problem line does not say it was left alone: $(grep '^problem: ' "$T/out" | cut -c1-120)]"
done
if [ -z "$b151" ]; then ok "IT-151 (defect M v2) a container named cv2-s5-server is never removed unless it carries cortexv2=dev AND this installer's compose project AND the working directory of THIS install's compose file: another or no cortexv2 value, another or no project, another or no directory each leave it alone and the message says so"; else bad "IT-151 a container that is not this install's is never removed" "$b151"; fi
# IT-152: the inspect fails, podman cannot say whether the container exists, the id is not an id, the removal fails or the object was replaced (its id is gone): the message says it could not be checked or removed and may still be running, nothing is removed by name;
# when there is no server container nothing is removed and the message says the server was not started
fresh; mrun FAKE_MIGRATE_FAIL=1 FAKE_RM_FAIL=1 && rc=0 || rc=$?
b152="$b152$(pm150 "removal fails")"
[ "$(grep -c "^rm -f $id150\$" "$T/log" || true)" = 1 ] || b152="$b152 [removal fails: one attempt by id expected]"
grep -q '^problem: .*could not be checked or removed.*may still be running' "$T/out" || b152="$b152 [removal fails: the problem line does not say it may still be running: $(grep '^problem: ' "$T/out" | cut -c1-120)]"
fresh; mrun FAKE_MIGRATE_FAIL=1 FAKE_RM_ID_GONE=1 && rc=0 || rc=$?
b152="$b152$(pm150 "the object was replaced")"
{ [ "$(grep -c '^rm ' "$T/log" || true)" = 1 ] && grep -qx "rm -f $id150" "$T/log" && ! grep '^rm ' "$T/log" | grep -q 'cv2-s5-server'; } || b152="$b152 [the object was replaced: expected exactly one removal, by the old id and never by name: $(grep '^rm ' "$T/log" | tr '
' '|')]"
grep -q '^problem: .*could not be checked or removed.*may still be running' "$T/out" || b152="$b152 [the object was replaced: the problem line does not report a removal failure: $(grep '^problem: ' "$T/out" | cut -c1-120)]"
for c152 in FAKE_CONTAINER_EXISTS_ERR=1 FAKE_CONTAINER_INSPECT_FAIL=1 FAKE_SERVER_ID=not-an-id FAKE_SERVER_ID= FAKE_SERVER_ID=0123abcd FAKE_SERVER_ID=zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz ; do
  fresh; mrun FAKE_MIGRATE_FAIL=1 "$c152" && rc=0 || rc=$?
  b152="$b152$(pm150 "$c152")"
  [ "$(grep -c '^rm ' "$T/log" || true)" = 0 ] || b152="$b152 [$c152: something was removed: $(grep '^rm ' "$T/log" | head -1)]"
  grep -q '^problem: .*could not be checked or removed.*may still be running' "$T/out" || b152="$b152 [$c152: the problem line does not say it could not be checked: $(grep '^problem: ' "$T/out" | cut -c1-120)]"
done
fresh; mrun FAKE_MIGRATE_FAIL=1 FAKE_NO_SERVER=1 && rc=0 || rc=$?
b152="$b152$(pm150 "no server container")"
[ "$(grep -c '^rm ' "$T/log" || true)" = 0 ] || b152="$b152 [no server container: a removal was tried]"
grep -q '^problem: .*migrations failed.*the server was not started' "$T/out" || b152="$b152 [no server container: the old, true sentence is missing: $(grep '^problem: ' "$T/out" | cut -c1-120)]"
if [ -z "$b152" ]; then ok "IT-152 (defect M v2) when the inspect fails, podman cannot say, the id is not an id, the removal fails or the object was replaced (its id is gone), the message says it could not be checked or removed and may still be running and nothing is removed by name; when there is no server container nothing is removed and the message says the server was not started"; else bad "IT-152 what the message says when the server container cannot be checked or removed, or does not exist" "$b152"; fi
# IT-153 (kai's reboot test on a fresh Rocky 10.2, 9 Nov, defect R; cox row f7ef0231) THE STACK COMES BACK AFTER A REBOOT. compose.yml gives db and server restart: unless-stopped, but ROOTLESS podman restarts containers at boot only through the USER unit
# podman-restart.service (the distro's: `podman start --all --filter should-start-on-boot=true`; its ExecStop `podman stop --service --all` keeps unless-stopped containers eligible: measured on podman 5.8.2), and that unit is disabled on a fresh install: after
# any reboot Cortex was silently down until the installer was run again (no data lost). The installer enables the DISTRO'S unit (no unit of ours), after the stack is up and before the summary; an enabled unit is left alone (nothing is run, nothing alarming
# is said); an unreachable or failing user manager FAILS CLOSED (EX_START, one problem and one fix: a silent down-after-reboot is the defect); a finished install whose unit is not enabled is NOT finished, so a second run repairs it.
b153=""
sd153='say_done() { echo "SAYDONE $#|$1|$2|$3|$4" >> "$FAKE_LOG"; }'
lane153="$(lane 'return 0;' 'return 0;')
$sd153"
ifd "$lane153" --project p --lead lead --home "$T/h" --port 8777
[ "$rc" = 0 ] || b153="$b153 [the first install failed: rc=$rc $(tail -1 "$T/out" | cut -c1-80)]"
[ "$(grep -c -x -e '--user enable podman-restart.service' "$T/state/systemctl.log" || true)" = 1 ] || b153="$b153 [the first install did not enable podman-restart.service exactly once: $(tr '\n' '|' < "$T/state/systemctl.log")]"
grep -qx 'podman-restart.service' "$T/state/enabled" 2>/dev/null || b153="$b153 [the unit is not enabled after the first install]"
l_up=$(grep -n ' up -d$' "$T/log" | head -1 | cut -d: -f1); l_en=$(grep -n '^systemctl --user enable podman-restart.service$' "$T/log" | head -1 | cut -d: -f1); l_sd=$(grep -n '^SAYDONE ' "$T/log" | head -1 | cut -d: -f1)
{ [ -n "$l_up" ] && [ -n "$l_en" ] && [ -n "$l_sd" ] && [ "$l_up" -lt "$l_en" ] && [ "$l_en" -lt "$l_sd" ]; } || b153="$b153 [the order is not: stack up ($l_up), enable ($l_en), summary ($l_sd)]"
[ ! -e "$T/home/.config/systemd/user/podman-restart.service" ] || b153="$b153 [the installer wrote a unit of its own named podman-restart.service]"
# a second run on a FINISHED install changes nothing: the unit is enabled, so nothing is enabled again and nothing alarming is said
ifd "$lane153" --check
if rel_install; then
[ "$rc" = 0 ] || b153="$b153 [the release install failed: rc=$rc $(tail -1 "$T/out" | cut -c1-80)]"
: > "$T/log"; : > "$T/state/systemctl.log"; run3
{ [ "$rc" = 0 ] && grep -q 'already installed' "$T/out" && [ ! -s "$T/state/systemctl.log" ] && ! grep -q '^problem: ' "$T/out"; } || b153="$b153 [the second run on a finished install was not a quiet no-op: rc=$rc systemctl calls='$(tr '\n' '|' < "$T/state/systemctl.log")']"
# a finished install whose restart unit is NOT enabled is not finished: the run repairs it (the unit is enabled once, the stack stays)
grep -vx 'podman-restart.service' "$T/state/enabled" > "$T/enabled.x" || :; cp "$T/enabled.x" "$T/state/enabled"; rm -f "$T/enabled.x"
: > "$T/log"; : > "$T/state/systemctl.log"; run3
{ [ "$rc" = 0 ] && ! grep -q 'already installed' "$T/out" && [ "$(grep -c -x -e '--user enable podman-restart.service' "$T/state/systemctl.log" || true)" = 1 ] && grep -qx 'podman-restart.service' "$T/state/enabled"; } || b153="$b153 [an install whose restart unit was disabled was not repaired by a run: rc=$rc $(tail -1 "$T/out" | cut -c1-70) systemctl calls='$(tr '\n' '|' < "$T/state/systemctl.log")']"
else b153="$b153 [release fixture setup]"; fi
# the unit was ALREADY enabled before the first run: it is not enabled again, and nothing alarming is said
ifd "$lane153" --check
fresh; echo 'podman-restart.service' > "$T/state/enabled"; run3 --port 8777
{ [ "$rc" = 0 ] && ! grep -q 'enable podman-restart.service' "$T/state/systemctl.log" && ! grep -q '^problem: ' "$T/out"; } || b153="$b153 [an already enabled unit was enabled again or an alarm was raised: rc=$rc calls='$(tr '\n' '|' < "$T/state/systemctl.log")']"
# the enable FAILS (no user bus for this shell, a distro without the unit): the install FAILS CLOSED after the stack is up, with ONE problem and ONE fix, and no summary
export FAKE_RESTART_ENABLE_FAIL=1; ifd "$lane153" --project p --lead lead --home "$T/h" --port 8777; unset FAKE_RESTART_ENABLE_FAIL
{ [ "$rc" = 6 ] && [ "$(grep -c '^problem: ' "$T/out" || true)" = 1 ] && [ "$(grep -c '^fix: ' "$T/out" || true)" = 1 ] && grep -q '^problem: .*a restart after a reboot could not be confirmed' "$T/out" && grep -q '^fix: .*XDG_RUNTIME_DIR' "$T/out" && tail -1 "$T/out" | grep -q 'could not be enabled' && [ "$(grep -c '^SAYDONE ' "$T/log" || true)" = 0 ] && grep -q ' up -d$' "$T/log"; } || b153="$b153 [a failing enable did not fail closed (rc 6, one problem naming the reboot, one fix naming XDG_RUNTIME_DIR, no summary, after the stack was up): rc=$rc problems=$(grep -c '^problem: ' "$T/out" || true) $(grep '^problem: ' "$T/out" | cut -c1-100)]"
if [ -z "$b153" ]; then ok "IT-153 (defect R) the installer enables the distro's podman-restart.service after the stack is up and before the summary; an enabled unit is left alone and nothing alarming is said; a finished install without it is repaired by a run; a failing enable fails closed with one problem and one fix and no summary"; else bad "IT-153 the stack comes back after a reboot: the installer enables podman-restart.service" "$b153"; fi
# IT-154 (sam's NMR-SAM-01, 9 Nov; cox's ruling a0c75b58) ONLY A DURABLE ENABLEMENT COUNTS. systemd's is-enabled exits 0 also for enabled-runtime (its links vanish at reboot), linked, linked-runtime, alias, static, indirect, generated and transient, so a successful STATUS
# is not proof that the stack comes back after a reboot. ONE predicate, used at BOTH sites (the step and install_finished): persistent means is-enabled exits 0 AND prints EXACTLY the word enabled. When it is false the installer runs `systemctl --user enable
# podman-restart.service` (no --now) and READS BACK with the same predicate; a readback that is not the exact word enabled FAILS CLOSED (exit 6, restart_unit_failed, the problem names the state systemd reported). The fake systemctl prints each state word with its
# real exit status (an empty answer and a missing unit included); the number of is-enabled questions about the unit shows the readback (two) or its absence (one).
b154=""
ifd "$lane153" --check
q154() { grep -c 'is-enabled .*podman-restart.service' "$FAKE_PROVLOG.ro" 2>/dev/null || true; }   # the is-enabled questions about the unit since the file was emptied
e154() { grep -c -x -e '--user enable podman-restart.service' "$T/state/systemctl.log" || true; }
# the persistent state: nothing is enabled, one question, no alarm
fresh; : > "$FAKE_PROVLOG.ro"; echo 'podman-restart.service' > "$T/state/enabled"; run3 --port 8777
{ [ "$rc" = 0 ] && [ "$(e154)" = 0 ] && [ "$(q154)" = 1 ]; } || b154="$b154 [enabled: rc=$rc enable calls=$(e154) questions=$(q154), want 0 and 1]"
# every state that is NOT persistent: the enable is run once and READ BACK (two questions) and the install finishes when the enable takes
for st154 in enabled-runtime linked linked-runtime alias static indirect generated transient disabled empty; do
  fresh; : > "$FAKE_PROVLOG.ro"; echo "$st154" > "$T/state/restart-state"; run3 --port 8777
  { [ "$rc" = 0 ] && [ "$(e154)" = 1 ] && [ "$(q154)" = 2 ] && grep -qx 'podman-restart.service' "$T/state/enabled"; } || b154="$b154 [$st154: the enable was not run once and read back, or the install failed: rc=$rc enable calls=$(e154) questions=$(q154), want 0, 1 and 2]"
done
# an enable that does NOT take (the unit stays in a state that does not survive a reboot, is masked or missing): FAIL CLOSED, naming the state, after one enable and one readback, with no summary
for st154 in enabled-runtime linked static indirect alias generated transient masked not-found empty; do
  fresh; : > "$FAKE_PROVLOG.ro"; echo "$st154" > "$T/state/restart-state"; export FAKE_RESTART_AFTER_ENABLE=$st154; run3 --port 8777; unset FAKE_RESTART_AFTER_ENABLE
  case $st154 in empty|not-found) w154="no usable answer" ;; *) w154=$st154 ;; esac
  { [ "$rc" = 6 ] && [ "$(grep -c '^problem: ' "$T/out" || true)" = 1 ] && [ "$(grep -c '^fix: ' "$T/out" || true)" = 1 ] && grep -qF "systemd reports: $w154" "$T/out" && tail -1 "$T/out" | grep -q 'not durably enabled after the enable' && [ "$(e154)" = 1 ] && [ "$(q154)" = 2 ] && [ "$(grep -c '^SAYDONE ' "$T/log" || true)" = 0 ]; } || b154="$b154 [$st154: an enable that does not take did not fail closed naming '$w154' after one enable and one readback with no summary: rc=$rc enable calls=$(e154) questions=$(q154) $(grep '^problem: ' "$T/out" | cut -c1-110)]"
done
# the SAME predicate at the other site: a FINISHED install whose unit is only runtime-enabled is not finished, and a run repairs it
if rel_install; then
: > "$T/log"; : > "$T/state/systemctl.log"; echo enabled-runtime > "$T/state/restart-state"; run3
{ [ "$rc" = 0 ] && ! grep -q 'already installed' "$T/out" && [ "$(e154)" = 1 ] && grep -qx 'podman-restart.service' "$T/state/enabled"; } || b154="$b154 [a finished install with a runtime-only unit was accepted or not repaired: rc=$rc enable calls=$(e154) $(tail -1 "$T/out" | cut -c1-70)]"
else b154="$b154 [release fixture setup]"; fi
if [ -z "$b154" ]; then ok "IT-154 (NMR-SAM-01) only the exact word enabled is persistent: every other state (enabled-runtime, linked, alias, static, indirect, generated, transient, disabled, empty ...) triggers the enable and a readback, an enable that does not take fails closed naming the state, and a finished install with a runtime-only unit is repaired"; else bad "IT-154 only a durable enablement of podman-restart.service counts" "$b154"; fi
# IT-155 (kai's defect S, the boot race; cox's ruling 2b9235e9; the note BOOT-RACE-S-NOTE-nemo-r0.md) THE WINDOW THAT A RETRY IN `cortex-server migrate` CREATES IS PINNED. After a reboot podman starts cv2-s5-migrate as the server's --requires 0.06 s after the db
# (probe-s.out); today it fails at once and stays exited 1 (defect S); finn's bounded retry makes it WAIT for the db, so for up to its cap migrate is 'running' (exit code 0) while the server is already up (probe-s2.out), and then ends exited 0. This cell adds NO installer
# behaviour: it pins what install.sh must keep doing in that window. With migrate 'running 0': install_finished is FALSE (it wants 'exited 0'), a rerun does not misjudge the stack: it removes nothing, stops nothing, does not read the server container's labels (the
# M path, `container inspect`), and prints no 'migrations failed' message; the control, migrate 'exited 1', still reaches the M path; and 'exited 0' is a finished install (a quiet no-op). (The false 'port already listening' that the prerequisite file's machine look prints
# in the window is a lane this suite fakes: it is stated in the note, accepted by cox, and not pinned here.)
b155=""
sd155='say_done() { echo "SAYDONE $#|$1|$2|$3|$4" >> "$FAKE_LOG"; }'
lane155="$(lane 'return 0;' 'return 0;')
$sd155"
# the lines of the call log that mean "something was removed, stopped, taken down, or a container's labels were read": TOKEN and END-OF-LINE boundaries, never a trailing space (sam's BRC-SAM-01: a pattern that ends in a space needs TWO spaces after 'pod' and
# something after 'compose ... down', and so misses 'pod stop' and a bare 'compose down'). The pattern has its own DENIED and ALLOWED controls, judged BEFORE it is trusted below.
bl155='^(rm|stop|kill)( |$)|^container (stop|kill|rm|inspect)( |$)|^pod (stop|kill|rm)( |$)|^compose( .*)? down( |$)'
for d155 in 'pod stop' 'pod stop cv2-s5-server' 'pod rm -f x' 'compose down' 'compose -f f.yml down' 'compose -f f.yml down --remove-orphans' 'rm' 'rm -f abc' 'stop cv2-s5-server' 'kill 9' 'container inspect --format x cv2-s5-server' 'container rm x' 'container stop'; do
  printf '%s\n' "$d155" | grep -qE "$bl155" || b155="$b155 [the blacklist MISSES a denied line: '$d155']"
done
for a155 in 'compose -f f.yml up -d' 'compose version' 'inspect --format {{.State.Status}} cv2-s5-migrate' 'secret inspect x' 'exec cv2-s5-server cat /run/cortex/bootstrap.token' 'stopwatch x' 'podman x' 'pod' 'compose -f f.yml up -d down-stream' 'rmdir x' 'killswitch x' 'port cv2-s5-server' '--version' 'SAYDONE 4|a|b|c|none' 'systemctl --user enable podman-restart.service'; do
  printf '%s\n' "$a155" | grep -qE "$bl155" && b155="$b155 [the blacklist CATCHES an allowed line: '$a155']"
done
ifd "$lane155" --check
if rel_install; then
[ "$rc" = 0 ] || b155="$b155 [the release install failed: rc=$rc $(tail -1 "$T/out" | cut -c1-80)]"
# migrate 'exited 0' (the state after a boot with the retry, once it has finished): a finished install, a quiet no-op
: > "$T/log"; export FAKE_MIGRATE_STATE='exited 0'; run3; unset FAKE_MIGRATE_STATE
{ [ "$rc" = 0 ] && [ "$(grep -c 'already installed' "$T/out" || true)" = 1 ] && ! grep -qE "$bl155" "$T/log"; } || b155="$b155 [migrate exited 0 was not a finished install, or something was removed or stopped: rc=$rc]"
# the RETRY WINDOW: migrate 'running 0' while the server is up
: > "$T/log"; export FAKE_MIGRATE_STATE='running 0'; run3; unset FAKE_MIGRATE_STATE
{ [ "$rc" = 0 ] && ! grep -q 'already installed' "$T/out"; } || b155="$b155 [migrate running: the install was judged finished, or the rerun failed: rc=$rc $(tail -1 "$T/out" | cut -c1-80)]"
! grep -qE "$bl155" "$T/log" || b155="$b155 [migrate running: something was removed, stopped or inspected for its labels: $(grep -E "$bl155" "$T/log" | head -1)]"
! grep -q 'Config.Labels' "$T/log" || b155="$b155 [migrate running: the labels of a container were read (the M path)]"
! grep -qE 'migrations failed|^problem: ' "$T/out" || b155="$b155 [migrate running: a failure was printed: $(grep -E 'migrations failed|^problem: ' "$T/out" | head -1 | cut -c1-100)]"
else b155="$b155 [release fixture setup]"; fi
# the CONTROL: migrate 'exited 1' still reaches the M path (the server container is inspected, ONE inspect) and the message is the failed-migration one
fresh; mrun FAKE_MIGRATE_STATE='exited 1' && rc=0 || rc=$?
{ [ "$rc" = 6 ] && [ "$(grep -c '^container inspect ' "$T/log" || true)" = 1 ] && grep -q '^problem: .*migrations failed' "$T/out"; } || b155="$b155 [the control, migrate exited 1, did not reach the M path: rc=$rc inspects=$(grep -c '^container inspect ' "$T/log" || true) $(grep '^problem: ' "$T/out" | head -1 | cut -c1-90)]"
if [ -z "$b155" ]; then ok "IT-155 (defect S) while migrate is 'running 0' (finn's retry window) install_finished is false and a rerun removes nothing, stops nothing, reads no container's labels and prints no failure; migrate 'exited 0' is a finished install; the control, 'exited 1', still reaches the M path"; else bad "IT-155 the retry window: a running migrate is not misjudged" "$b155"; fi
# ======== E-01 (cox W1-188) and Q17c / Q17d (kai e0d60d61; the CTO's D140 to D142; vera's W1-185; Q17d-PREINCLUDE-COVERAGE-LOSS-cox-r5.md section 5) ====================================================================
# IT-156 (Q17c, messages 1 and 2) THE TWO NAME CHECKS SAY THE RULE IN WORDS: a refused --project or --lead name gets a cause (problem:), a fix (fix:) and a last sentence that each say it: starts with a small letter, then only small letters, digits and hyphens,
# 32 characters at most; exit 1, nothing touched. (The pattern '[a-z][a-z0-9-]{0,31}' alone was the only remedy: vera's W1-185 listed both lines as findings on the installer's messages.)
b156=""
for o156 in project lead; do
  for v156 in 'A' 'a_b' '1a' 'abcdefghijklmnopqrstuvwxyz0123456789'; do
    fresh
    if [ "$o156" = project ]; then sh "$install" --project "$v156" --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?; else sh "$install" --project p --lead "$v156" --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?; fi
    w156="--$o156 '$v156'"
    [ "$rc" = 1 ] || b156="$b156 [$w156: rc=$rc, want 1]"
    { [ "$(grep -c '^problem: ' "$T/out")" = 1 ] && [ "$(grep -c '^fix: ' "$T/out")" = 1 ]; } || b156="$b156 [$w156: not ONE problem: line and ONE fix: line]"
    grep -q "^problem: the $o156 name is not allowed: .*small letter" "$T/out" || b156="$b156 [$w156: the cause does not name the option and say the rule in words]"
    grep -q "^fix: give --$o156 a name .*small letter.*run this installer again" "$T/out" || b156="$b156 [$w156: the fix does not name the option, say the rule in words and say what to do]"
    tail -1 "$T/out" | grep -q "^install: --$o156 .*small letter, then only small letters, digits and hyphens, 32 characters at most" || b156="$b156 [$w156: the last line is not the rule in words: $(tail -1 "$T/out" | cut -c1-90)]"
    { [ ! -e "$T/h" ] && [ ! -s "$T/log" ]; } || b156="$b156 [$w156: something was touched]"
  done
done
if [ -z "$b156" ]; then ok "IT-156 (Q17c) a refused --project or --lead name says the rule in words (small letter first, then small letters, digits and hyphens, 32 characters at most) in its cause, its fix and its last line, exit 1, nothing touched"; else bad "IT-156 the name messages" "$b156"; fi
# IT-157 (Q17c, message 3; sam E01-SAM01, cox W1-223) THE UNREADABLE-RULES MESSAGE ENDS IN A FIX AND DENIES NOTHING THAT EXISTS: the catalogue id db_unreadable already prints a problem: and a fix: line before it; the LAST line says what could not be read, what to do, what this refusal did NOT touch (the
# database's rules, data and volume) and that what the installer has made so far stays in place. It is reached AFTER the home, the secrets, the configuration, the backup scripts and the database start, so a blanket 'nothing was changed' was false: the cell checks the words AND that state.
b157=""
fresh; env FAKE_HBA_FAIL=1 sh "$install" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?
[ "$rc" = 6 ] || b157="$b157 [rc=$rc, want 6]"
{ [ "$(grep -c '^problem: ' "$T/out")" = 1 ] && [ "$(grep -c '^fix: ' "$T/out")" = 1 ]; } || b157="$b157 [not ONE problem: line and ONE fix: line]"
tail -1 "$T/out" | grep -q "^install: could not read the database's access rules: check podman logs cv2-s5-db, then run this installer again (that is safe); this refusal did not touch the database's rules, data or volume, and what the installer has made so far stays in place\$" || b157="$b157 [the last line is not the words: $(tail -1 "$T/out" | cut -c1-200)]"
! grep -qi 'nothing was changed' "$T/out" || b157="$b157 [the output still says nothing was changed]"
{ [ -d "$T/h" ] && [ -f "$T/h/cortex.toml" ] && [ -d "$T/h/bin" ] && [ "$(grep -c 'secret create' "$T/log")" -ge 3 ] && grep -q 'up -d db$' "$T/log"; } || b157="$b157 [the state sam names is missing: the refused run did not leave the home, the configuration, the three secrets and the started database that the message must not deny]"
{ ! grep -qE 'volume (rm|prune)|rm .*--volumes|compose .* down' "$T/log" && [ ! -e "$T/h/lead.token" ]; } || b157="$b157 [a volume was touched or a token was made on the refused run]"
if [ -z "$b157" ]; then ok "IT-157 (Q17c) an unreadable access-rules query ends in a last line with its fix, what the refusal did not touch and what stays in place (exit 6, one problem, one fix), and the home, configuration, secrets and database start it follows exist"; else bad "IT-157 the unreadable-rules message" "$b157"; fi
# IT-164 (sam E01-SAM01, cox W1-223) THE TWO NEXT LINES, SAME PATH, SAME WORDS: a count of trust rules that is not a number (exit 6, id db_unreadable) and a database that trusts (exit 6, id db_trust) say what the refusal did not touch (the database's rules, data and volume) and that what is made stays in place;
# neither says nothing was changed (nor 'never changes or deletes a database'); the state after the run: the home, the configuration and the secrets exist, the roles step did not run, no volume was touched.
b164=""
for c164 in 'FAKE_HBA_RAW=abc|could not read a count of trust rules from the database|check podman logs cv2-s5-db, then run this installer again \(that is safe\)' 'FAKE_HBA_TRUST=2|trust or broken pg_hba rule|see S5-PEER-AUTH-MAP.md'; do
  e164=${c164%%|*}; r164=${c164#*|}; w164=${r164%%|*}; x164=${r164#*|}
  fresh; env "$e164" sh "$install" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?
  [ "$rc" = 6 ] || b164="$b164 [$e164: rc=$rc, want 6]"
  grep -q "$w164" "$T/out" || b164="$b164 [$e164: the line does not say '$w164']"
  grep -qiE "$x164" "$T/out" || b164="$b164 [$e164: no '$x164']"
  grep -q "this refusal did not touch the database's rules, data or volume, and what the installer has made so far stays in place" "$T/out" || b164="$b164 [$e164: the words are not there]"
  ! grep -qiE 'nothing was changed|never changes or deletes' "$T/out" || b164="$b164 [$e164: a blanket no-change claim is still there: $(grep -iE 'nothing was changed|never changes or deletes' "$T/out" | head -1 | cut -c1-90)]"
  { [ -d "$T/h" ] && [ -f "$T/h/cortex.toml" ] && [ "$(grep -c 'secret create' "$T/log")" -ge 3 ] && ! roles_ran && ! grep -qE 'volume (rm|prune)|rm .*--volumes|compose .* down' "$T/log"; } || b164="$b164 [$e164: the state after the run is not the one the words describe]"
done
if [ -z "$b164" ]; then ok "IT-164 (cox W1-223) a count of trust rules that is not a number and a database that trusts: the last line says what the refusal did not touch and what stays in place, no blanket no-change claim, and the home, configuration and secrets exist, no roles step, no volume touched"; else bad "IT-164 the next two lines of the same path" "$b164"; fi
# IT-165 (cox W1-223, THE CLASS) NO BLANKET NO-CHANGE CLAIM AFTER THE FIRST WRITE: (a) no catalogue text (inst_msg) says 'nothing was changed', 'nothing was written' or 'never changes or deletes' (the catalogue is read on any path); (b) every such sentence of the sequence lies BEFORE the first write
# (the first stage_make or home_make call of the sequence). 'nothing was started' and 'nothing else was started' (no container, or none beyond the database) are not claims of this class.
b165=""
fw165=$(grep -n -E '^( +)?(stage_make|home_make)$' "$install" | head -1 | cut -d: -f1)
case $fw165 in ""|*[!0123456789]*) b165="$b165 [the first write of the sequence was not found]" ;; esac
n165=0
for ln165 in $(grep -n -E 'nothing was changed|nothing was written|never changes or deletes' "$install" | grep -v -E '^[0123456789]+:[[:space:]]*#' | cut -d: -f1); do
  n165=$((n165 + 1)); l165=$(sed -n "${ln165}p" "$install")
  case $l165 in
    *'m_cause='*|*'m_fix='*) b165="$b165 [line $ln165: a catalogue text claims it]" ;;
    *) [ "$ln165" -lt "${fw165:-0}" ] || b165="$b165 [line $ln165: the claim is after the first write (line ${fw165:-?})]" ;;
  esac
done
[ "$n165" -ge 1 ] || b165="$b165 [no claim found at all: the cell is blind (line 648's sentence is one)]"
if [ -z "$b165" ]; then ok "IT-165 (cox W1-223) every 'nothing was changed' of install.sh lies before the first write, and no catalogue text makes a blanket claim"; else bad "IT-165 no blanket claim after the first write" "$b165"; fi
# IT-158 (E-01, cox W1-188) EVERY MESSAGE ID HAS ONE TEXT IN THE JOINED INSTALLER: the three tables (install.sh's inst_msg, install-prereq.sh's prereq_msg, install-fetch.sh's fetch_msg) share no id (msg() asks the prerequisite table first, then the fetch table, then inst_msg:
# an id in two tables was answered by the wrong one). The two lane files are the repository's (deploy/), install.sh is the one under test.
ids_of() { sed -n "/^$2() {/,/^}/p" "$1" | grep -oE '^ +[a-z_0-9]+\)' | tr -d ' )'; }
b158=""
for f158 in "$install:inst_msg" "$here/../deploy/install-prereq.sh:prereq_msg" "$here/../deploy/install-fetch.sh:fetch_msg"; do
  n158=$(ids_of "${f158%%:*}" "${f158##*:}" | wc -l); [ "$n158" -ge 10 ] || b158="$b158 [${f158##*:}: only $n158 ids found: the table moved or the reader is blind]"
done
dup158=$( { ids_of "$install" inst_msg; ids_of "$here/../deploy/install-prereq.sh" prereq_msg; ids_of "$here/../deploy/install-fetch.sh" fetch_msg; } | sort | uniq -d | tr '\n' ' ')
[ -z "$dup158" ] || b158="$b158 [ids with TWO texts in the joined installer: $dup158]"
if [ -z "$b158" ]; then ok "IT-158 (E-01) the message tables of install.sh, install-prereq.sh and install-fetch.sh share no id: every id has ONE text in the joined installer"; else bad "IT-158 one text per id" "$b158"; fi
# IT-159 (E-01) THE JOINED INSTALLER, PER ID, AGAINST THE TABLE THAT MUST ANSWER: install.sh with the REAL prereq_msg and fetch_msg tables as its lane text (no machine_look and no prereq_run: the steps that need them are skipped) prints, for its own four stops, ITS OWN
# cause: a symbolic-link home, an empty trust anchor, podman older than 5 after the prerequisite step, no compose provider after it. (Before E-01 the other lane's text answered: 'the private staging parent can be replaced', 'the release signing key could not be confirmed', ...)
real_tables() { sed -n '/^prereq_msg() {/,/^}/p' "$here/../deploy/install-prereq.sh"; sed -n '/^fetch_msg() {/,/^}/p' "$here/../deploy/install-fetch.sh"; }
mk159() { rm -rf "$T/d83"; mkdir -p "$T/d83"; cp "$install" "$T/d83/install.sh"; { real_tables; [ -z "${1:-}" ] || printf '%s\n' "$1"; } > "$T/d83/install-prereq.sh"; }
b159=""
n159=$(real_tables | wc -l); [ "$n159" -ge 150 ] || b159="$b159 [the real tables were not found: $n159 lines]"
if rm -rf "$T/hl" "$T/hl-target" && mkdir "$T/hl-target" && ln -s "$T/hl-target" "$T/hl"; then
mk159 ""; fresh; sh "$T/d83/install.sh" --project p --lead lead --home "$T/hl" > "$T/out" 2>&1 && rc=0 || rc=$?
[ "$rc" = 2 ] || b159="$b159 [a symbolic-link home: rc=$rc, want 2]"
grep -q '^problem: the install home is not a real directory of yours' "$T/out" || b159="$b159 [a symbolic-link home: the cause is not install.sh's own: $(grep '^problem: ' "$T/out" | cut -c1-100)]"
else b159="$b159 [home-link fixture setup]"; fi
rm -rf "$T/hl" "$T/hl-target"
mk159 'rel_fetch() { :; }
rel_trust() { :; }
rel_unpack() { :; }
rel_load() { :; }'
fresh; sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" --release 0.2.1 > "$T/out" 2>&1 && rc=0 || rc=$?
[ "$rc" = 4 ] || b159="$b159 [an empty trust anchor: rc=$rc, want 4]"
grep -q '^problem: this installer carries no usable trust anchor' "$T/out" || b159="$b159 [an empty trust anchor: the cause is not install.sh's own: $(grep '^problem: ' "$T/out" | cut -c1-100)]"
grep -q '^fix: fetch the complete installer again' "$T/out" || b159="$b159 [an empty trust anchor: the fix is not install.sh's own: $(grep '^fix: ' "$T/out" | cut -c1-100)]"
mk159 ""; fresh; env FAKE_PODMAN_VERSION=4.9.0 sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?
[ "$rc" = 3 ] || b159="$b159 [podman 4: rc=$rc, want 3]"
grep -qx 'problem: the installed podman is older than 5' "$T/out" || b159="$b159 [podman 4: the cause is not install.sh's own: $(grep '^problem: ' "$T/out" | cut -c1-100)]"
mk159 ""; fresh; env FAKE_COMPOSE_VERSION_FAIL=1 sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?
[ "$rc" = 3 ] || b159="$b159 [no compose provider: rc=$rc, want 3]"
grep -q '^problem: no compose provider found (podman-compose 1.5.0 or newer is required, and it is not bundled)' "$T/out" || b159="$b159 [no compose provider: the cause is not install.sh's own: $(grep '^problem: ' "$T/out" | cut -c1-100)]"
if [ -z "$b159" ]; then ok "IT-159 (E-01) with the REAL prerequisite and fetch tables joined, install.sh's own four stops (a symbolic-link home, an empty trust anchor, podman older than 5, no compose provider) print install.sh's own cause and fix"; else bad "IT-159 the joined installer per id" "$b159"; fi
# IT-160 (Q17c) G7's STATIC RULE, THE GATE RUNNER'S OWN FUNCTION (scripts/install-gate/gate_runner.py g7_static), over the three installer files as one text lists only the NINE messages vera ruled as bounded runner cues (W1-185: the seven 'needs a value' messages, the SemVer message
# with its examples, the 'no compose provider: podman-compose 1.5.0 or newer is required' message) and none of the three that were findings (the two name checks and the unreadable-rules message). Any other bare die or fail message is listed here too.
b160=""
cat "$install" "$here/../deploy/install-prereq.sh" "$here/../deploy/install-fetch.sh" > "$T/joined160.sh"
python3 -I -B - "$here/install-gate" "$T/joined160.sh" > "$T/g7.out" 2>&1 <<'PY' || b160="$b160 [the static rule did not run: $(tail -1 "$T/g7.out" | cut -c1-100)]"
import sys
sys.path.insert(0, sys.argv[1])
import gate_runner
ok, listed = gate_runner.g7_static(sys.argv[2])
allowed = ("--project needs a value", "--lead needs a value", "--home needs a value", "--port needs a value", "--release needs a value", "--base-url needs a value", "--from needs a value",
           "--release must be a SemVer such as", "no compose provider: podman-compose 1.5.0 or newer is requir")
outside = [l for l in listed if not l.split(": ", 1)[1].startswith(allowed)]
absent = [a for a in allowed if not any(l.split(": ", 1)[1].startswith(a) for l in listed)]
for l in outside:
    print("LISTED " + l)
for a in absent:
    print("MISSING " + a)
print("static rule ran: %d listed, %d outside the nine ruled cues" % (len(listed), len(outside)))
PY
grep -q '^static rule ran: ' "$T/g7.out" || b160="$b160 [no result line]"
grep -q '^LISTED ' "$T/g7.out" && b160="$b160 [$(grep '^LISTED ' "$T/g7.out" | head -3 | cut -c1-90 | tr '\n' '|')]"
grep -q '^MISSING ' "$T/g7.out" && b160="$b160 [a ruled cue is no longer listed (the cell asserts all nine, sam): $(grep '^MISSING ' "$T/g7.out" | head -3 | cut -c1-70 | tr '\n' '|')]"
if [ -z "$b160" ]; then ok "IT-160 (Q17c) G7's static rule lists EXACTLY the nine messages vera ruled as bounded cues (all nine present, none other): the two name checks and the unreadable-rules message are no longer listed"; else bad "IT-160 G7's static rule" "$b160"; fi
# IT-161 (Q17d step 1b, vera's conditions) THE INSTALLER LEARNS ITS OWN DIRECTORY WITH THE SHELL ALONE: the statements from 'case $0 in' to 'here=$(' (the include context of the gate matrix keeps the here= line) give, for each path form, the same result and the same error behaviour as
# the dirname, cd and pwd they replace: a name without a slash maps to '.', a root-level /script to '/', a path with a slash to its directory (extra slashes, dot and dot-dot parts, a leading hyphen, a space), a missing directory fails with the same status and message.
b161=""
hs161=$(sed -n '/^case \$0 in$/,/^here=\$(/p' "$install")
[ "$(printf '%s\n' "$hs161" | wc -l)" -ge 4 ] || b161="$b161 [the own-directory statements (from 'case \$0 in' to 'here=\$(') were not found in install.sh]"
rm -rf "$T/own"; mkdir -p "$T/own/x/y" "$T/own/-d" "$T/own/a b"
own_new() { o161=$( ( cd "$1" && sh -c "set -eu
$hs161
printf '%s\\n' \"\$here\"" "$2" ) 2>&1 ) && r161=0 || r161=$?; printf '%s\n' "$o161" | sed 's/line [0-9]*: //'; echo "rc=$r161"; }
own_ref() { o161=$( ( cd "$1" && sh -c 'set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
printf "%s\n" "$here"' "$2" ) 2>&1 ) && r161=0 || r161=$?; printf '%s\n' "$o161" | sed 's/line [0-9]*: //'; echo "rc=$r161"; }
for c161 in "$T/own|install.sh" "$T/own|./install.sh" "$T/own|x/install.sh" "$T/own|x//install.sh" "$T/own|./x/../x/y/install.sh" "$T/own|$T/own/x/install.sh" "$T/own|-d/install.sh" "$T/own|a b/install.sh" \
            "$T/own/x|../x/install.sh" "$T/own|/install.sh" "$T/own|//install.sh" "$T/own|///install.sh" "$T/own|nodir/install.sh" "$T/own|x/nodir/install.sh"; do
  d161=${c161%%|*}; p161=${c161#*|}
  a161=$(own_new "$d161" "$p161"); r161=$(own_ref "$d161" "$p161")
  [ "$a161" = "$r161" ] || b161="$b161 [\$0=$p161: got '$(printf '%s' "$a161" | tr '\n' '|' | cut -c1-70)' want '$(printf '%s' "$r161" | tr '\n' '|' | cut -c1-70)']"
done
[ "$(own_new "$T/own" install.sh | sed -n 1p)" = "$(cd "$T/own" && pwd)" ] || b161="$b161 [a name without a slash is not the current directory]"
[ "$(own_new "$T/own" /install.sh | sed -n 1p)" = / ] || b161="$b161 [/install.sh is not /]"
case $(own_new "$T/own" nodir/install.sh) in *"rc=1") ;; *) b161="$b161 [a missing directory did not fail with status 1]" ;; esac
if [ -z "$b161" ]; then ok "IT-161 (Q17d) the installer's own directory by the shell alone: a name without a slash is '.', /script is '/', and 14 path forms give the result and the failure of dirname, cd and pwd"; else bad "IT-161 the own directory" "$b161"; fi
# IT-162 (Q17d step 1b) NO OUTSIDE PROGRAM BEFORE THE INCLUDED PARTS: (a) started with a PATH that holds no program at all, the installer reaches its first refusal (--check without a machine look in this build) and says 'not found' nowhere; (b) a fake dirname first on PATH is never started
# (necessary, not sufficient: vera) before the first refusal. The shell's own start-up files are outside the proof (vera's premise: the supported Bash, with compgen, cd and pwd as builtins).
b162=""
rm -rf "$T/d83" "$T/nobin" "$T/bin-dn"; mkdir -p "$T/d83" "$T/nobin" "$T/bin-dn"; cp "$install" "$T/d83/install.sh"
env PATH="$T/nobin" /bin/sh "$T/d83/install.sh" --check > "$T/out" 2>&1 && rc=0 || rc=$?
[ "$rc" = 1 ] || b162="$b162 [a PATH with no program: rc=$rc, want the refusal 1]"
grep -q -- '--check is not available in this build' "$T/out" || b162="$b162 [a PATH with no program: the first refusal was not reached: $(tail -1 "$T/out" | cut -c1-90)]"
! grep -q 'not found' "$T/out" || b162="$b162 [a program was started before the included parts: $(grep 'not found' "$T/out" | head -1 | cut -c1-90)]"
printf '#!/bin/sh\necho CANARY >> "%s/dirname.canary"\necho /nonexistent-dir\n' "$T" > "$T/bin-dn/dirname"; chmod +x "$T/bin-dn/dirname"; rm -f "$T/dirname.canary"
env PATH="$T/bin-dn:$PATH" sh "$T/d83/install.sh" --check > "$T/out" 2>&1 && rc=0 || rc=$?   # as far as the first refusal: the later steps run scripts of their own (db-roles.sh and others), which may use dirname
[ ! -e "$T/dirname.canary" ] || b162="$b162 [a fake dirname first on PATH was started before the first refusal]"
{ [ "$rc" = 1 ] && grep -q -- '--check is not available in this build' "$T/out"; } || b162="$b162 [with a fake dirname on PATH the first refusal was not reached: rc=$rc: $(tail -1 "$T/out" | cut -c1-90)]"
if [ -z "$b162" ]; then ok "IT-162 (Q17d) no outside program runs before the included parts: a PATH with no program reaches the first refusal without 'not found', and a fake dirname on PATH is never started"; else bad "IT-162 no outside program before the includes" "$b162"; fi
# IT-163 (Q17d step 1) SSL_CERT_FILE AND SSL_CERT_DIR ARE REMOVED BEFORE ANY OUTSIDE PROGRAM RUNS: tools that record what they inherit (id, stat, date, mktemp, sed, cat, tr, head, od, awk, grep, cp, mv, rm, mkdir, chmod, wc, sort, uniq, cut, find, ls, ln, touch, sleep) are first on PATH; with both
# variables exported, EVERY recorded start sees both unset, in a plain run (a) and in a run whose lane text starts a program while the included parts are read (b: the first program of the build), so the unset comes before the include.
b163=""
rm -rf "$T/bin-ssl"; mkdir -p "$T/bin-ssl"
for t163 in id stat date mktemp sed cat tr head od awk grep cp mv rm mkdir chmod wc sort uniq cut find ls ln touch sleep; do
  printf '#!/bin/sh\nprintf "%%s|%%s\\n" "${SSL_CERT_FILE-unset}" "${SSL_CERT_DIR-unset}" >> "$FAKE_STATE/ssl.seen"\nPATH=/usr/bin:/bin exec %s "$@"\n' "$t163" > "$T/bin-ssl/$t163"; chmod +x "$T/bin-ssl/$t163"
done
fresh; env SSL_CERT_FILE=/nonexistent/f SSL_CERT_DIR=/nonexistent/d PATH="$T/bin-ssl:$PATH" sh "$install" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?
[ "$rc" = 0 ] || b163="$b163 [(a) the install failed: rc=$rc: $(tail -1 "$T/out" | cut -c1-80)]"
[ "$(wc -l < "$T/state/ssl.seen" 2>/dev/null || echo 0)" -ge 5 ] || b163="$b163 [(a) the recording tools were hardly started: the cell would prove nothing]"
! grep -qv '^unset|unset$' "$T/state/ssl.seen" 2>/dev/null || b163="$b163 [(a) a program inherited a certificate variable: $(grep -v '^unset|unset$' "$T/state/ssl.seen" | head -1 | cut -c1-60)]"
rm -rf "$T/d83"; copy_deploy "$(dirname "$install")" "$T/d83"; printf '%s\n' 'id -u >/dev/null' > "$T/d83/install-prereq.sh"   # a whole copy of deploy/ (a full install needs the compose file and the config example beside the script)
fresh; env SSL_CERT_FILE=/nonexistent/f SSL_CERT_DIR=/nonexistent/d PATH="$T/bin-ssl:$PATH" sh "$T/d83/install.sh" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?
[ "$rc" = 0 ] || b163="$b163 [(b) the install failed: rc=$rc: $(tail -1 "$T/out" | cut -c1-80)]"
[ "$(sed -n 1p "$T/state/ssl.seen" 2>/dev/null)" = "unset|unset" ] || b163="$b163 [(b) the first program of the build, started while the included parts are read, inherited a certificate variable: '$(sed -n 1p "$T/state/ssl.seen" 2>/dev/null)']"
if [ -z "$b163" ]; then ok "IT-163 (Q17d) SSL_CERT_FILE and SSL_CERT_DIR are unset before any outside program runs: every recorded program start sees both unset, also the first program of the build (started while the included parts are read)"; else bad "IT-163 the certificate variables" "$b163"; fi
# IT-140 THE JOINED CELL (cox W1-99 (3)): the REAL fetch include (rel_fetch, rel_trust, rel_unpack, rel_load, verify_release_dir) with hue's REAL signed fixture (scripts/install-fetch-fixture.py: a real ssh-keygen -Y sign over the
# builder's SHA256SUMS), NO fake rel_* in the lane. What is NOT real here and why: the machine tools of the machine (podman: the include's podman seam is the ONE line of the real include this cell changes, from /usr/bin/podman to
# a PATH lookup, so that the real podman of the machine is never touched; the fake records `load --input` and `image exists`; the fake curl answers the health route with the release's own version, FAKE_SERVER_VERSION=0.2.1), and the 6b prerequisite lane (marv's real install-prereq.sh looks at and changes the real machine:
# its joined run is L2 / IP-64): a no-op machine_look and prereq_run stand in. The cell needs the stage 2 include and the fixture script IN THE TREE; a tree without them (before hue's stage 2 pin merges) prints NOT RUN and is
# neither a pass nor a fail: r4 does not merge without this cell GREEN on the joined tree (cox W1-99 (3)).
fx140="$here/install-fetch-fixture.py"; inc140="$here/../deploy/install-fetch.sh"
if [ -f "$fx140" ] && [ -f "$inc140" ] && grep -q '^verify_release_dir()' "$inc140" && grep -q '^rel_unpack()' "$inc140" && grep -q '^rel_load()' "$inc140"; then
  b140=""
  mkfx() { # mkfx <name>: a fresh signed release set and its anchor in $T/<name> (a TEST key; the private key stays in $T/<name>/private)
    rm -rf "$T/$1"; mkdir "$T/$1"; python3 -I "$fx140" "$T/$1" 0.2.1 > "$T/$1.out" 2>&1 || return 1
    [ -f "$T/$1/anchor/anchor.env" ] || return 1
  }
  anc140() { sed -n "s/^$2=//p" "$T/$1/anchor/anchor.env" | head -1; }   # anc140 <name> <PRINCIPAL|KEY|FPR>
  joined() { # joined <principal> <key> <fpr>: $T/joined = the tree's deploy/ (copy_deploy: neither real include) + the REAL fetch include with its podman seam on PATH + a no-op prerequisite lane + the anchor literals given
    rm -rf "$T/joined"; copy_deploy "$here/../deploy" "$T/joined" || return 1
    sed 's#/usr/bin/podman "\$@"#podman "$@"#' "$inc140" > "$T/joined/install-fetch.sh"
    [ "$(diff "$inc140" "$T/joined/install-fetch.sh" | grep -c '^<')" = 1 ] || return 1   # exactly ONE line of the real include differs
    printf '%s\n' 'machine_look() { return 0; }' 'prereq_run() { return 0; }' 'prereq_compose_path() { printf "%s\n" "$PROV2"; }' > "$T/joined/install-prereq.sh"
    cp "$T/joined/install.sh" "$T/joined/install.empty.sh"; fill_anchor "$T/joined/install.empty.sh" "$T/joined/install.sh" "$1" "$2" "$3"
  }
  runj() { # runj: a real install of the signed release 0.2.1 from $T/fx/release into a fresh home; sets rc, $T/out, $T/log
    fresh; FAKE_SERVER_VERSION=0.2.1 sh "$T/joined/install.sh" --project p --lead lead --home "$T/h" --release 0.2.1 --from "$T/fx/release" --port 8777 > "$T/out" 2>&1 && rc=0 || rc=$?
  }
  refused140() { # refused140 <what> <catalogue problem text>: rc 4, ONE problem: line with that text, ONE fix: line, nothing unpacked or loaded, no home, no stage, no anchor directory
    r140=""
    [ "$rc" = 4 ] || r140="$r140 rc=$rc (want 4)"
    grep -qxF "problem: $2" "$T/out" || r140="$r140 [the problem line is not '$2': $(grep '^problem: ' "$T/out" | head -1 | cut -c1-90)]"
    { [ "$(grep -c '^problem: ' "$T/out" || true)" = 1 ] && [ "$(grep -c '^fix: ' "$T/out" || true)" = 1 ]; } || r140="$r140 [not exactly one problem: and one fix: line]"
    { [ ! -e "$T/h" ] && [ -z "$(stages108)" ] && [ -z "$(ancleft)" ] && ! grep -q '^load ' "$T/log"; } || r140="$r140 [home=$([ -e "$T/h" ] && echo made || echo none) stage='$(stages108)' anchor='$(ancleft)' loaded=$(grep -c '^load ' "$T/log" || true)]"
    [ -z "$r140" ] || b140="$b140 [$1:$r140]"
  }
  if ! { mkfx fx && mkfx fx2; }; then b140="$b140 [the fixture script failed: $(head -c 200 "$T/fx.out" "$T/fx2.out" 2>/dev/null | tr '\n' '|')]"; else
    P1=$(anc140 fx PRINCIPAL); K1=$(anc140 fx KEY); F1=$(anc140 fx FPR); P2=$(anc140 fx2 PRINCIPAL); K2=$(anc140 fx2 KEY); F2=$(anc140 fx2 FPR)
    # (a) the right anchor and a release signed by that key. RUN 1: the REAL include trusts, unpacks, loads and (by the installer's read of the release) the REAL verifier says yes; install.sh then stops AT THE MODULE LIST,
    # as designed (the fixture's config example has no [modules] list): exit 6 and its message. The person does what the message says; RUN 2 CONTINUES (resume, W1-81): nothing is fetched or loaded again, the REAL verifier is
    # asked again, the install finishes; RUN 3 changes nothing.
    joined "$P1" "$K1" "$F1" || b140="$b140 [the joined subject could not be built]"
    runj
    { [ "$rc" = 6 ] && grep -q 'no \[modules\] enabled list' "$T/out" && [ -L "$T/h/current" ] && [ "$(readlink "$T/h/current")" = "releases/0.2.1" ] && [ -f "$T/h/releases/0.2.1/stack.env" ] && [ "$(grep -c '^load --input ' "$T/log" || true)" = 1 ]; } || b140="$b140 [run 1 on the real include: rc=$rc (want 6 at the module list) current='$(readlink "$T/h/current" 2>/dev/null)' loads=$(grep -c '^load --input ' "$T/log" || true) $(tail -1 "$T/out" | cut -c1-120)]"
    { [ -z "$(stages108)" ] && [ -z "$(ancleft)" ]; } || b140="$b140 [a stage or anchor directory was left after run 1]"
    if [ -f "$T/h/cortex.toml" ] && printf '[modules]\nenabled = ["records"]\n' >> "$T/h/cortex.toml"; then
    : > "$T/log"; FAKE_SERVER_VERSION=0.2.1 sh "$T/joined/install.sh" --project p --lead lead --home "$T/h" --release 0.2.1 --from "$T/fx/release" --port 8777 > "$T/out" 2>&1 && rc=0 || rc=$?
    { [ "$rc" = 0 ] && [ -s "$T/h/lead.token" ] && ! grep -q '^load ' "$T/log" && [ -L "$T/h/current" ]; } || b140="$b140 [run 2 (resume after the module list): rc=$rc token=$([ -s "$T/h/lead.token" ] && echo yes || echo no) loads=$(grep -c '^load ' "$T/log" || true) $(tail -2 "$T/out" | tr '\n' '|' | cut -c1-160)]"
    : > "$T/log"; FAKE_SERVER_VERSION=0.2.1 sh "$T/joined/install.sh" --project p --lead lead --home "$T/h" --release 0.2.1 --from "$T/fx/release" --port 8777 > "$T/out" 2>&1 && rc=0 || rc=$?
    { [ "$rc" = 0 ] && ! grep -q '^load ' "$T/log" && grep -q 'already installed' "$T/out"; } || b140="$b140 [run 3 on the finished real install: rc=$rc loads=$(grep -c '^load ' "$T/log" || true) $(head -4 "$T/out" | tr '\n' '|' | cut -c1-320)]"
    else b140="$b140 [config fixture setup: the dependent resume runs cannot go on]"; fi
    # (c) ONE character of the fingerprint literal changed: the real rel_trust refuses (the key could not be confirmed), although the release is signed by the right key
    FX=$(printf '%s' "$F1" | sed 's/.$/Z/'); [ "$FX" != "$F1" ] || FX=$(printf '%s' "$F1" | sed 's/.$/Y/')
    joined "$P1" "$K1" "$FX"; runj; refused140 "one character of the fingerprint changed" "the release signing key could not be confirmed"
    # (d) another pair's KEY with the first pair's fingerprint: the key file's fingerprint is not the literal: refused the same way
    joined "$P1" "$K2" "$F1"; runj; refused140 "another pair's key" "the release signing key could not be confirmed"
    # (e) a complete anchor of ANOTHER pair against a release signed by the first: the key is confirmed, the signature is not
    joined "$P2" "$K2" "$F2"; runj; refused140 "right literals, a release signed by ANOTHER key" "the release checksum signature could not be verified"
  fi
  if [ -z "$b140" ]; then ok "IT-140 (cox W1-99 (3), JOINED) the REAL fetch include and hue's REAL signed fixture, no fake rel_* in the lane: the right anchor and a release signed by that key install (trusted, unpacked, loaded, verified by the real verifier, current is the link; the module-list stop and the resume after it; a third run changes nothing); one character of the fingerprint changed, another pair's key, and a release signed by another key are each refused with exit 4, one problem and one fix, before anything is unpacked or loaded (the machine tools and the 6b prerequisite lane are stand-ins, said above)"; else bad "IT-140 the joined cell" "$b140"; fi
else
  echo "NOT RUN IT-140 (the joined cell): this tree has no stage 2 include (verify_release_dir, rel_unpack, rel_load in deploy/install-fetch.sh) and/or no scripts/install-fetch-fixture.py yet; r4 does not merge without it GREEN on the joined tree (cox W1-99 (3))"
fi

# IT-166, IT-166b, IT-166c, IT-166d (cox ad9da414 and W1-232, kai d7f6c666, hale O-2 on 16be99e) WHAT THE DOWNLOAD'S CURL INHERITS: the release download uses NO PROXY, only the system trust store, and no caller value can make curl write TLS secrets. The subject is IT-140's kind of joined tree (the REAL fetch include,
# the anchor of a test key) built from the installer UNDER TEST ($install: a mutant of install.sh in a mutant run, which IT-140's joined() would not see) with TWO lines of the real include changed: podman's seam (as IT-140) and the curl seam (rel_tool_curl: /usr/bin/curl becomes the path of a fake curl; a PATH lookup would not do, rel_env_clean sets PATH inside rel_fetch). An ONLINE install is asked for (--base-url); the fake
# curl records its first argument and the names it inherits, one NAME=value line each, and fails like a download that could not be made (exit 22). The caller sets every name: the three trust and key-log names and the other curl names (CURL_CA_BUNDLE, SSLKEYLOGFILE, CURL_SSL_BACKEND, QLOGDIR), SSL_CERT_FILE and SSL_CERT_DIR (IT-163), and the eight proxy
# names. IT-166: a caller's CURL_CA_BUNDLE (and SSL_CERT_FILE, SSL_CERT_DIR) does not reach the download's curl. IT-166b: SSLKEYLOGFILE, QLOGDIR and CURL_SSL_BACKEND do not. IT-166c: none of the eight proxy names does (http_proxy, https_proxy, HTTP_PROXY, HTTPS_PROXY, all_proxy, ALL_PROXY, no_proxy,
# NO_PROXY). IT-166d: the first argument of every curl start is -q (hale: it shuts out .curlrc, which HOME, CURL_HOME and XDG_CONFIG_HOME can point at; without it a .curlrc with a cacert line was accepted; HOME cannot be unset, so -q is load-bearing).
# WHAT THESE CELLS PROVE: for every curl start of that run the installer process handed the seam's curl that environment, whatever the caller had set. WHAT THEY DO NOT PROVE: what the machine's own /usr/bin/curl honours (hale measured that: the note's table), nor that a name is unset on a path that starts no curl.
a166=""; b166b=""; c166=""; d166=""
j166() { # j166 <principal> <key> <fpr>: $T/joined166 = a copy of the deploy directory of the installer UNDER TEST ($install: in a mutant run, the mutant) + the REAL fetch include with its podman seam and its curl seam changed (exactly two lines differ) + a no-op prerequisite lane + the anchor literals given
  rm -rf "$T/joined166"; copy_deploy "$(dirname "$install")" "$T/joined166" || return 1
  sed -e 's#/usr/bin/podman "\$@"#podman "$@"#' -e 's#/usr/bin/curl "\$@"#'"$T"'/bin-cenv/curl "$@"#' "$inc140" > "$T/joined166/install-fetch.sh"
  [ "$(diff "$inc140" "$T/joined166/install-fetch.sh" | grep -c '^<')" = 2 ] || return 1
  printf '%s\n' 'machine_look() { return 0; }' 'prereq_run() { return 0; }' 'prereq_compose_path() { printf "%s\n" "$PROV2"; }' > "$T/joined166/install-prereq.sh"
  cp "$T/joined166/install.sh" "$T/joined166/install.empty.sh"; fill_anchor "$T/joined166/install.empty.sh" "$T/joined166/install.sh" "$1" "$2" "$3"
}
if type mkfx >/dev/null 2>&1 && type anc140 >/dev/null 2>&1 && type mkfx >/dev/null 2>&1 && [ -f "$fx140" ] && [ -f "$inc140" ]; then
  rm -rf "$T/bin-cenv"; mkdir -p "$T/bin-cenv"
  cat > "$T/bin-cenv/curl" <<'FAKE'
#!/bin/sh
# the fake curl of IT-166: it records its first argument and the variables it inherits (NAME=value, 'unset' when the name is not in its environment), a line of dashes after each start, and fails like a download that could not be made
echo "ARG1=${1-none}" >> "$FAKE_STATE/curl.seen"
for n in CURL_CA_BUNDLE SSLKEYLOGFILE CURL_SSL_BACKEND QLOGDIR SSL_CERT_FILE SSL_CERT_DIR http_proxy https_proxy HTTP_PROXY HTTPS_PROXY all_proxy ALL_PROXY no_proxy NO_PROXY; do eval "v=\${$n-unset}"; printf '%s=%s\n' "$n" "$v"; done >> "$FAKE_STATE/curl.seen"
echo '---' >> "$FAKE_STATE/curl.seen"
exit 22
FAKE
  chmod +x "$T/bin-cenv/curl"
  if mkfx fx166; then
    j166 "$(anc140 fx166 PRINCIPAL)" "$(anc140 fx166 KEY)" "$(anc140 fx166 FPR)" || a166="$a166 [the joined subject could not be built (or not exactly two lines of the real include differ from the tree: the podman seam and the curl seam)]"
    fresh
    env CURL_CA_BUNDLE=/nonexistent/ca.pem SSLKEYLOGFILE=/nonexistent/keys.log CURL_SSL_BACKEND=openssl QLOGDIR=/nonexistent/qlog SSL_CERT_FILE=/nonexistent/f SSL_CERT_DIR=/nonexistent/d \
      http_proxy=http://127.0.0.1:9/http_proxy https_proxy=http://127.0.0.1:9/https_proxy HTTP_PROXY=http://127.0.0.1:9/HTTP_PROXY HTTPS_PROXY=http://127.0.0.1:9/HTTPS_PROXY all_proxy=http://127.0.0.1:9/all_proxy ALL_PROXY=http://127.0.0.1:9/ALL_PROXY no_proxy=localhost,no_proxy NO_PROXY=localhost,NO_PROXY \
      sh "$T/joined166/install.sh" --project p --lead lead --home "$T/h" --release 0.2.1 --base-url https://releases.example.invalid/cortex --port 8777 > "$T/out" 2>&1 && rc=0 || rc=$?
    seen166="$T/state/curl.seen"; starts166=$(grep -c '^---$' "$seen166" 2>/dev/null || true); starts166=${starts166:-0}
    if [ "$starts166" -lt 1 ]; then a166="$a166 [the fake curl was never started (rc=$rc): $(tail -2 "$T/out" | tr '\n' '|' | cut -c1-200)]"; b166b="$a166"; c166="$a166"; d166="$a166"; else
      unset166() { # unset166 NAME...: the names that reached some curl start (printed), empty when none did
        for n166 in "$@"; do [ "$(grep -c "^$n166=unset$" "$seen166" || true)" = "$starts166" ] || printf ' %s' "$n166"; done; }
      r166=$(unset166 CURL_CA_BUNDLE SSL_CERT_FILE SSL_CERT_DIR); [ -z "$r166" ] || a166="$a166 [reached the download's curl:$r166]"
      r166=$(unset166 SSLKEYLOGFILE QLOGDIR CURL_SSL_BACKEND); [ -z "$r166" ] || b166b="$b166b [reached the download's curl:$r166]"
      r166=$(unset166 http_proxy https_proxy HTTP_PROXY HTTPS_PROXY all_proxy ALL_PROXY no_proxy NO_PROXY); [ -z "$r166" ] || c166="$c166 [reached the download's curl:$r166]"
      [ "$(grep -c '^ARG1=-q$' "$seen166" || true)" = "$starts166" ] || d166="$d166 [a curl start whose first argument is not -q: $(grep '^ARG1=' "$seen166" | grep -v '^ARG1=-q$' | head -1 | cut -c1-40)]"
    fi
  else a166="$a166 [the fixture script failed: $(head -c 200 "$T/fx166.out" 2>/dev/null | tr '\n' '|')]"; b166b="$a166"; c166="$a166"; d166="$a166"; fi
  if [ -z "$a166" ]; then ok "IT-166 (hale O-2) a caller's CURL_CA_BUNDLE (and SSL_CERT_FILE, SSL_CERT_DIR) does not reach the download's curl: every curl start of an online install on the joined tree sees all three unset"; else bad "IT-166 the trust store of the download" "$a166"; fi
  if [ -z "$b166b" ]; then ok "IT-166b (hale O-2) SSLKEYLOGFILE, QLOGDIR and CURL_SSL_BACKEND do not reach the download's curl: no caller value makes curl write TLS secrets or log, or choose a TLS backend"; else bad "IT-166b the key log, qlog and backend of the download" "$b166b"; fi
  if [ -z "$c166" ]; then ok "IT-166c (kai, W1-232) none of the eight proxy names (http_proxy, https_proxy, HTTP_PROXY, HTTPS_PROXY, all_proxy, ALL_PROXY, no_proxy, NO_PROXY) reaches the download's curl: the release download uses no proxy"; else bad "IT-166c the proxy names of the download" "$c166"; fi
  if [ -z "$d166" ]; then ok "IT-166d (hale O-2) the first argument of every curl start is -q (it shuts out .curlrc, which HOME, CURL_HOME and XDG_CONFIG_HOME can point at)"; else bad "IT-166d -q is the first argument" "$d166"; fi
else
  echo "NOT RUN IT-166 (the joined cell of IT-140 is not available in this tree)"
fi

# ===================================================================================================================================================================
# the four TEST-ONLY HOOKS cox asked for from nix's L2 spike (row fc450500; the installer's behaviour is not touched): IT-121 (N6) no cell writes outside its private temp, IT-122 (N1) the harness values a caller sets REACH the
# installer, IT-123 (N3) IT_ONLY selects test ids, IT-124 (N2) L2_KEEP keeps each test's output, the call log and compose.env. The cells run THIS suite on itself with IT_SUBRUN=1 (a sub-run never runs these cells).
# ===================================================================================================================================================================
if [ -z "${IT_SUBRUN:-}" ]; then
# THE BOUND of a sub-run (sub_bound, seconds; 30 unless a call sets another): the cells BEFORE the selected id still run, so a selection costs the time of the suite up to its last id. MEASURED (scratch copies, the shell's own clock, through the admission shim,
# 16 vCPUs; the 1-minute load average at the start and at the end of each run is in the sealed receipts): the IT-54 selection (the 53 cells before it) took 16.173 s with the machine's own load of about 5 to 7, 20.058 s with 8 whole suites side by side (load 7.5 to 13.7),
# 28.622 s with 16 sub-runs at once (11.8 to 18.9) and 40.827 s with 24 at once (18.9 to 29.8): it was past 30 s under load (a baseline failed with rc=124), so its call sets 180 s (4.4 times the heaviest measure, 6.3 times the 16-at-once one).
# The selections of id 01 or 06 took at most 4.726 s with 24 at once: 30 s is 6.3 times that. A bound stays a bound: a selection that HANGS fails the cell within its bound (the hang case of IT-123).
sub() { # sub <env assignments...> -- : this suite again, selected by IT_ONLY, output in $T/sub.out, status in src
  timeout "${sub_bound:-30}" env IT_SUBRUN=1 INSTALL_SH="$install" "$@" sh "$here/install-test.sh" > "$T/sub.out" 2>&1 && src=0 || src=$?   # bounded: a selection that does not stop must fail the cell, not run the whole suite
}
# IT-121 (N6): no cell plants or writes a file outside its private temp directory ($T): the old IT-55 wrote /tmp/pwned
b121=""
n121=$(grep -c 'touch /tm''p/\|> /tm''p/\|>> /tm''p/\|tee /tm''p/' "$here/install-test.sh" || true)
[ "$n121" = 0 ] || b121="$b121 [$n121 line(s) of the suite write under /tm""p directly: $(grep -n 'touch /tm''p/\|> /tm''p/\|>> /tm''p/' "$here/install-test.sh" | head -2 | cut -c1-80 | tr '\n' '|')]"
if [ -z "$b121" ]; then ok "IT-121 (N6) no cell of the suite writes outside its private temp directory (IT-55 plants its canary under \$T)"; else bad "IT-121 a cell writes outside its temp" "$b121"; fi
# IT-122 (N1): the harness's test-only values are set from a caller variable when one is given, so a HOSTILE value reaches the installer and is judged there (the first install of IT-01 then fails with the installer's own
# refusal). UNREACHED BY DESIGN: FAKE_LOG and FAKE_STATE (the channels of the fakes, not inputs of the installer).
b122=""
sub L2_POLL=hostile IT_ONLY=IT-01
{ [ "$src" != 0 ] && grep -q '^FAIL IT-01' "$T/sub.out" && grep -q 'CORTEX_INSTALL_POLL must be a whole number' "$T/sub.out"; } || b122="$b122 [L2_POLL=hostile did not reach the installer: rc=$src $(grep -c '^ok   IT-01' "$T/sub.out" || true) ok line(s)]"
sub L2_WAIT=hostile IT_ONLY=IT-01
{ [ "$src" != 0 ] && grep -q 'CORTEX_INSTALL_WAIT must be a whole number' "$T/sub.out"; } || b122="$b122 [L2_WAIT=hostile did not reach the installer]"
sub L2_SCRIPTS_DIR=/nonexistent-cv2-scripts IT_ONLY=IT-01
{ [ "$src" != 0 ] && grep -q 'backup script cortex-dump is missing in /nonexistent-cv2-scripts' "$T/sub.out"; } || b122="$b122 [L2_SCRIPTS_DIR did not reach the installer]"
sub L2_HOME=relative/home IT_ONLY=IT-01
{ [ "$src" != 0 ] && grep -q 'HOME must be an absolute path' "$T/sub.out"; } || b122="$b122 [L2_HOME did not reach the installer]"
mkdir -p "$T/pfx"; printf '#!/bin/sh\necho 0\n' > "$T/pfx/id"; chmod +x "$T/pfx/id"
sub L2_PATH_PREFIX="$T/pfx" IT_ONLY=IT-01
{ [ "$src" != 0 ] && grep -q 'run as a normal user' "$T/sub.out"; } || b122="$b122 [L2_PATH_PREFIX did not reach the installer]"
sub IT_ONLY=IT-01
{ [ "$src" = 0 ] && grep -q '^ok   IT-01' "$T/sub.out"; } || b122="$b122 [the control (no override) did not pass IT-01: rc=$src]"
if [ -z "$b122" ]; then ok "IT-122 (N1) L2_POLL, L2_WAIT, L2_SCRIPTS_DIR, L2_HOME and L2_PATH_PREFIX reach the installer (a hostile value is judged by it); FAKE_LOG and FAKE_STATE are unreached by design"; else bad "IT-122 harness values reach the installer" "$b122"; fi
# IT-123 (N3): IT_ONLY="IT-06 IT-01" selects test ids (IT-54 also selects IT-54b): only those are REPORTED and counted, the run stops once they have all been reported (the cells before them still run: they share state);
# an unknown id is exit 2 with a message, never a silent empty run
b123=""
sub IT_ONLY="IT-06 IT-01"
{ [ "$src" = 0 ] && [ "$(grep -c '^ok ' "$T/sub.out" || true)" = 2 ] && grep -q '^ok   IT-01 ' "$T/sub.out" && grep -q '^ok   IT-06 ' "$T/sub.out" && ! grep -q '^ok   IT-02 ' "$T/sub.out" && grep -qx 'install-test: 2 passed, 0 failed' "$T/sub.out"; } || b123="$b123 [two selected ids: rc=$src ok-lines=$(grep -c '^ok ' "$T/sub.out" || true) summary='$(tail -1 "$T/sub.out" | cut -c1-60)']"
sub_bound=180; sub IT_ONLY=IT-54; sub_bound=30
{ [ "$src" = 0 ] && grep -q '^ok   IT-54' "$T/sub.out" && [ "$(grep -c '^ok ' "$T/sub.out" || true)" -ge 2 ] && ! grep -q '^ok   IT-55' "$T/sub.out"; } || b123="$b123 [a base id does not select its sub-ids: rc=$src ok-lines=$(grep -c '^ok ' "$T/sub.out" || true)]"
nox123="IT-9$((9 * 111))"   # IT-9999, built here so that the suite's own text does not carry it
sub IT_ONLY="$nox123"
{ [ "$src" = 2 ] && grep -q 'unknown test id' "$T/sub.out" && grep -q "$nox123" "$T/sub.out"; } || b123="$b123 [an unknown id: rc=$src '$(tail -1 "$T/sub.out" | cut -c1-70)']"
sub IT_ONLY="IT-06 $nox123"
{ [ "$src" = 2 ] && grep -q 'unknown test id' "$T/sub.out"; } || b123="$b123 [a known and an unknown id: rc=$src]"
# the bound is a BOUND (kai and cox, 9 Nov): a selection that HANGS must FAIL within the bound its call sets (rc 124), not run on. A stand-in for sh that does `exec sleep 25` is reached through the env assignments that sub() passes (env sets PATH, then looks sh up);
# exec, so that the timeout's signal reaches the sleep itself and nothing is left behind. The call sets a bound of 2 s: it must end with rc 124 well before the 25 s of the sleep.
mkdir -p "$T/hang123"; printf '#!/bin/sh\nexec sleep 25\n' > "$T/hang123/sh"; chmod +x "$T/hang123/sh"
t123a=$(date +%s); sub_bound=2; sub PATH="$T/hang123:$PATH" IT_ONLY=IT-01; sub_bound=30; t123b=$(date +%s)
{ [ "$src" = 124 ] && [ $((t123b - t123a)) -lt 10 ]; } || b123="$b123 [a selection that hangs was not stopped by its bound: rc=$src after $((t123b - t123a)) s (want rc=124 within 10 s)]"
if [ -z "$b123" ]; then ok "IT-123 (N3) IT_ONLY selects test ids (a base id selects its sub-ids); only they are reported and counted, the run stops after them; an unknown id is exit 2; a selection that hangs is stopped by its bound (rc 124)"; else bad "IT-123 IT_ONLY selects test ids" "$b123"; fi
# IT-124 (N2): L2_KEEP=dir keeps, BEFORE the temp directory is removed, each reported test's output ($T/out), the call log and compose.env under its test id, and the last log and compose.env as final.*
b124=""
rm -rf "$T/keep"
sub L2_KEEP="$T/keep" IT_ONLY="IT-01 IT-06"
{ [ "$src" = 0 ] && [ -s "$T/keep/IT-01.out" ] && grep -q 'install: ' "$T/keep/IT-01.out" && [ -s "$T/keep/IT-01.log" ] && grep -q 'secret create' "$T/keep/IT-01.log" && [ -s "$T/keep/IT-01.compose.env" ] && [ -e "$T/keep/IT-06.out" ] && [ -s "$T/keep/final.log" ]; } || b124="$b124 [kept files: rc=$src $(ls "$T/keep" 2>/dev/null | tr '\n' ' ')]"
[ "$(stat -c %a "$T/keep" 2>/dev/null || echo none)" = 700 ] || b124="$b124 [the keep directory is not 0700: $(stat -c %a "$T/keep" 2>/dev/null || echo none)]"
rm -rf "$T/keep"; sub IT_ONLY=IT-01
[ ! -e "$T/keep" ] || b124="$b124 [files were kept without L2_KEEP: $(ls "$T/keep" | tr '\n' ' ')]"
if [ -z "$b124" ]; then ok "IT-124 (N2) L2_KEEP=dir keeps each reported test's output, the call log and compose.env under its test id (and final.log), in a 0700 directory, before the temp directory goes"; else bad "IT-124 L2_KEEP keeps the evidence" "$b124"; fi
fi

# D181 summary cells. Each has its own stack, so these are independent of IT-01.
# Selecting older cells avoids this block; the full suite always runs it.
case " ${IT_ONLY:-} " in
  '  '|*' IT-168 '*|*' IT-169 '*|*' IT-170 '*|*' IT-171 '*|*' IT-172 '*|*' IT-173 '*|*' IT-174 '*|*' IT-175 '*|*' IT-176 '*|*' IT-177 '*|*' IT-178 '*|*' IT-179 '*|*' IT-180 '*|*' IT-181 '*|*' IT-182 '*|*' IT-183 '*|*' IT-184 '*|*' IT-185 '*|*' IT-186 '*|*' IT-187 '*|*' IT-188 '*|*' IT-189 '*|*' IT-190 '*|*' IT-191 '*|*' IT-192 '*|*' IT-193 '*|*' IT-196 '*|*' IT-197 '*|*' IT-198 '*|*' IT-199 '*|*' IT-200 '*|*' IT-201 '*|*' IT-202 '*|*' IT-206 '*|*' IT-207 '*|*' IT-209 '*|*' IT-210 '*|*' IT-211 '*|*' IT-212 '*|*' IT-213 '*|*' IT-217 '*|*' IT-218 '*|*' IT-225 '*|*' IT-227 '*|*' IT-228 '*|*' IT-229 '*|*' IT-230 '*|*' IT-231 '*|*' IT-232 '*|*' IT-233 '*|*' IT-234 '*|*' IT-235 '*|*' IT-236 '*|*' IT-237 '*|*' IT-238 '*|*' IT-239 '*|*' IT-240 '*|*' IT-241 '*|*' IT-242 '*)
sum_home=$T/h
sum_fresh() { rm -rf "$T/h" "$sum_home" "$T/state" "$T/home/.config" "$T/account-runtime"; mkdir -m 700 "$T/account-runtime"; mkdir -p "$T/state"; : > "$T/log"; }
sum_run() { setsid sh "$install" --project p --lead lead --home "$sum_home" "$@" > "$T/out" 2> "$T/sum.err"; }
sum_pty() {
  SUM_INSTALL=$install SUM_HOME=$sum_home SUM_OUT=$T/${SUM_TAG:-sum}.stdout SUM_ERR=$T/${SUM_TAG:-sum}.stderr SUM_TTY=$T/${SUM_TAG:-sum}.tty \
  python3 -I -c '
import errno, os, pty, select, signal, subprocess, sys, time
pid, fd = pty.fork()
if pid == 0:
    if os.environ.get("FAKE_MARKER_TTY_FAIL") == "1":
        signal.signal(signal.SIGHUP, signal.SIG_IGN)
    out = os.open(os.environ["SUM_OUT"], os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    err = os.open(os.environ["SUM_ERR"], os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
    os.dup2(out, 1); os.dup2(err, 2)
    os.execvp("sh", ["sh", os.environ["SUM_INSTALL"], "--project", "p", "--lead", "lead", "--home", os.environ["SUM_HOME"], *sys.argv[1:]])
end = time.monotonic() + 30
tty_closed = False
boundary = os.environ.get("FAKE_BOUNDARY_PHASE")
boundary_sent = False
with open(os.environ["SUM_TTY"], "wb") as transcript:
    while True:
        if time.monotonic() >= end:
            os.killpg(pid, signal.SIGKILL)
            os.waitpid(pid, 0)
            sys.exit(124)
        boundary_ready = os.path.join(os.environ["FAKE_STATE"], "boundary-ready")
        if boundary and not boundary_sent and os.path.exists(boundary_ready):
            with open(boundary_ready) as ready_file:
                observed = ready_file.read().strip()
            if observed == boundary:
                # Signal only the private pty child this helper created, once.
                if os.environ.get("FAKE_BOUNDARY_SIGNAL") == "NONE":
                    contender_env = dict(os.environ)
                    contender_env.pop("FAKE_BOUNDARY_PHASE", None)
                    contender_env.pop("FAKE_BOUNDARY_SIGNAL", None)
                    contender_env["FAKE_BOOT_TOKEN"] = os.environ["FAKE_CONTENDER_TOKEN"]
                    contender_args = ["sh", os.environ["SUM_INSTALL"], "--project", "p", "--lead", os.environ.get("FAKE_CONTENDER_LEAD", "lead"), "--home", os.environ.get("FAKE_CONTENDER_HOME", os.environ["SUM_HOME"]), "--reissue-token"]
                    with open(os.path.join(os.environ["FAKE_STATE"], "contender.stdout"), "wb") as co, open(os.path.join(os.environ["FAKE_STATE"], "contender.stderr"), "wb") as ce:
                        contender = subprocess.run(contender_args, env=contender_env, stdout=co, stderr=ce, timeout=20)
                    with open(os.path.join(os.environ["FAKE_STATE"], "contender.rc"), "w") as report:
                        report.write(str(contender.returncode))
                elif os.environ.get("FAKE_BOUNDARY_SIGNAL") == "SKIP":
                    pass  # control run reaches the same boundary without its second signal
                else:
                    chosen = getattr(signal, "SIG" + os.environ.get("FAKE_BOUNDARY_SIGNAL", "TERM"))
                    os.kill(pid, chosen)
                boundary_sent = True
                open(os.path.join(os.environ["FAKE_STATE"], "boundary-delivered"), "w").close()
                open(os.path.join(os.environ["FAKE_STATE"], "boundary-release"), "w").close()
        ready = os.path.join(os.environ["FAKE_STATE"], "marker-ready")
        if os.environ.get("FAKE_MARKER_TTY_FAIL") == "1" and os.path.exists(ready):
            os.close(fd)
            tty_closed = True
            open(os.path.join(os.environ["FAKE_STATE"], "tty-closed"), "w").close()
            break
        if select.select([fd], [], [], .1)[0]:
            try: data = os.read(fd, 65536)
            except OSError as e:
                if e.errno == errno.EIO: break
                raise
            if not data: break
            transcript.write(data)
if not tty_closed: os.close(fd)
_, status = os.waitpid(pid, 0)
if boundary and boundary != "reissue-collision" and not boundary_sent: sys.exit(125)
sys.exit(os.waitstatus_to_exitcode(status))
' "$@"
}
# IT-199: exact known other path pairs must not hide partial token-path references.
mkdir -p "$T/path-fixtures"
python3 -I - "$T/path-fixtures" <<'PYCODE'
from pathlib import Path
import sys
root = Path(sys.argv[1])
path = b'/shared-root/long-home-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx/h/lead.token'
log = b'/shared-root/long-home-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx/install.log'
base = b'Token file:\n' + path + b'\nInstall log:\n' + log + b'\n'
cases = [('clean', base, 0), ('duplicate', base + path + b'\n', 1),
         ('inline', base + b'old path: ' + path + b'\n', 1),
         ('prefix', base + path[:16] + b'\n', 1),
         ('middle', base + path[20:36] + b'\n', 1),
         ('suffix', base + path[-16:] + b'\n', 1),
         ('unlabelled-log', base.replace(b'Install log:\n', b'wrong label\n'), 1)]
(root / 'token-path').write_bytes(path)
(root / 'log-path').write_bytes(log)
for name, data, expected in cases:
    (root / name).write_bytes(data)
(root / 'cases').write_text(''.join(f'{name} {expected}\n' for name, data, expected in cases))
PYCODE
sum_path_fixture_bad=0
while read -r sum_case sum_want; do
  sum_path_total_once "$T/path-fixtures/$sum_case" "$(cat "$T/path-fixtures/token-path")" 'Install log:' "$(cat "$T/path-fixtures/log-path")" && sum_actual=0 || sum_actual=$?
  if [ "$sum_actual" != "$sum_want" ]; then
    echo "path fixture $sum_case returned $sum_actual, expected $sum_want"
    sum_path_fixture_bad=$((sum_path_fixture_bad + 1))
  fi
done < "$T/path-fixtures/cases"
if [ "$sum_path_fixture_bad" = 0 ]; then ok 'IT-199 token-path fragments outside exact known labelled pairs fail; shared-root log pair passes'; else bad 'IT-199 path-fragment guard' "$sum_path_fixture_bad fixtures"; fi
# IT-196: a full-token-only grep misses formatter-folded tokens and fragments.
mkdir -p "$T/private-fixtures"
python3 -I - "$T/private-fixtures" <<'PY'
from pathlib import Path
import subprocess
import sys
root = Path(sys.argv[1])
tokens = [b'ctx2_0123456789abcdef0123456789abcdef.ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopq',
          b'FOREIGN-ACTIVE-TOKEN']
cases = []
for ident, token in enumerate(tokens):
    for channel in ('stdout', 'stderr', 'app-log', 'fake-log'):
        for label in ('public', 'raw', 'folded', 'fragment'):
            name = f'{ident}-{channel}-{label}'
            case = root / name
            case.mkdir()
            (case / 'secret').write_bytes(token + b'\n')
            for public in ('stdout', 'stderr', 'app-log', 'fake-log'):
                (case / public).write_bytes(b'public recovery instructions\n')
            if label == 'raw':
                data = token + b'\n'
            elif label == 'folded':
                data = subprocess.run(['fold', '-w78'], input=b'x' * 70 + token + b'\n',
                                      stdout=subprocess.PIPE, check=True).stdout
                assert token not in data
            elif label == 'fragment':
                data = token[2:18] + b'\n'
            else:
                data = b'public recovery instructions\n'
            (case / channel).write_bytes(data)
            cases.append(f'{name} {0 if label == "public" else 1}\n')
(root / 'cases').write_text(''.join(cases))
PY
sum_private_bad=0
while read -r sum_case sum_want; do
  sum_fixture=$T/private-fixtures/$sum_case
  sum_private_files "$sum_fixture/secret" "$sum_fixture/stdout" "$sum_fixture/stderr" "$sum_fixture/app-log" "$sum_fixture/fake-log" && sum_actual=0 || sum_actual=$?
  if [ "$sum_actual" != "$sum_want" ]; then
    echo "private channel fixture $sum_case returned $sum_actual, expected $sum_want"
    sum_private_bad=$((sum_private_bad + 1))
  fi
done < "$T/private-fixtures/cases"
if [ "$sum_private_bad" = 0 ]; then ok 'IT-196 both token values and formatted or 16-byte fragment leaks fail in every public channel; clean controls pass'; else bad 'IT-196 credential fragment assertions' "$sum_private_bad fixtures"; fi
sum_fresh
if sum_run && grep -q 'is installed and running' "$T/out" &&
  grep -q 'Checked this machine ... ok' "$T/out" && grep -q 'Verified prerequisites ... ok' "$T/out" &&
  grep -q 'Project and lead created ... ok' "$T/out" && grep -q 'Lead credential written ... ok' "$T/out" &&
  grep -q 'Backups ... ok' "$T/out" && grep -q 'Health (/v2/health/ready) ... 200 ok' "$T/out" &&
  grep -q '2 of 2 running, healthy' "$T/out" && grep -q '200 ok: lead, project_lead of p' "$T/out" &&
  grep -q '/v2/whoami' "$T/log" && ! grep -q 'Executing external compose provider' "$T/out" "$T/sum.err"; then
  ok 'IT-168 SUM-1 measured steps and three real check calls; no compose banner'
else bad 'IT-168 SUM-1 good install summary'; fi
sum_fresh
if sum_run --no-start && grep -q 'Containers started ... skipped (--no-start)' "$T/out" &&
  ! grep -q 'is installed and running\|SHOWN ONCE\|Containers started ... ok' "$T/out"; then
  ok 'IT-169 SUM-2 no-start reports skipped stages'
else bad 'IT-169 SUM-2 no-start reports skipped stages'; fi
sum_bad=0
for sum_case in status name kind role transport health container; do
  sum_fresh
  case $sum_case in
    status) FAKE_SUM_WHO_HTTP=401 sum_run && sum_rc=0 || sum_rc=$? ;;
    name) FAKE_WHO_NAME=another sum_run && sum_rc=0 || sum_rc=$? ;;
    kind) FAKE_WHO_KIND=installation sum_run && sum_rc=0 || sum_rc=$? ;;
    role) FAKE_WHO_ROLE=reader sum_run && sum_rc=0 || sum_rc=$? ;;
    transport) FAKE_SUM_TRANSPORT=fail sum_run && sum_rc=0 || sum_rc=$? ;;
    health) FAKE_SUM_HEALTH_HTTP=503 sum_run && sum_rc=0 || sum_rc=$? ;;
    container) FAKE_SUM_CONTAINER='exited unhealthy' sum_run && sum_rc=0 || sum_rc=$? ;;
  esac
  { [ "$sum_rc" = 6 ] && grep -q 'installed but NOT working' "$T/out" &&
    grep -q FAILED "$T/out" && grep -q '^problem:' "$T/sum.err" && grep -q '^fix:' "$T/sum.err" &&
    ! grep -q 'SHOWN ONCE' "$T/out" "$T/sum.err"; } || sum_bad=$((sum_bad + 1))
done
if [ "$sum_bad" = 0 ]; then ok 'IT-170 SUM-3 wrong sign-in, transport, health and container results fail with exit 6'; else bad 'IT-170 SUM-3 end checks fail closed' "$sum_bad incorrect cases"; fi
sum_fresh
# Synthetic 81-byte token proves the real token shape fits by wrapping on screen.
: > "$T/sum.log"
if FAKE_BOOT_TOKEN=$sum_fake_token FAKE_SUM_ENV_CHECK=1 CORTEX_LOG=$T/sum.log sum_pty &&
  [ "$(tr -d '\r' < "$T/sum.tty" | grep -c '^ Credential: SHOWN ONCE')" = 1 ] &&
  [ "$(tr -d '\r' < "$T/sum.tty" | grep -c -x -F "$sum_fake_token")" = 1 ] &&
  ! grep -qF "$sum_fake_token" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log" &&
  LC_ALL=C awk '/^ctx2_/ {next} length($0)>79 {exit 1}' "$T/sum.tty"; then
  ok 'IT-171 SUM-4 PTY credential once; stdout stderr log argv and child env exclude it'
else bad 'IT-171 SUM-4 terminal-only new credential'; fi
sum_fresh
: > "$T/sum.log"
if FAKE_BOOT_TOKEN=$sum_fake_token CORTEX_LOG=$T/sum.log sum_run && grep -q 'Credential not shown: no terminal' "$T/out" &&
  sum_path_once "$T/out" '   Token file (0600):' "$T/h/lead.token" &&
  sum_private_files "$T/h/lead.token" "$T/out" "$T/sum.err" "$T/sum.log" "$T/log"; then
  ok 'IT-172 SUM-5 no terminal reports file and displays no credential'
else bad 'IT-172 SUM-5 no terminal'; fi
sum_fresh
: > "$T/sum.log"
if FAKE_BOOT_TOKEN=$sum_fake_token CORTEX_LOG=$T/sum.log sum_pty --no-show-credentials && [ ! -s "$T/sum.tty" ] &&
  sum_path_once "$T/sum.stdout" '   Token file (0600):' "$T/h/lead.token" &&
  sum_private_files "$T/h/lead.token" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log"; then
  ok 'IT-173 SUM-6 suppression flag also works with a controlling terminal'
else bad 'IT-173 SUM-6 suppression flag'; fi
sum_fresh
if sum_pty && [ -f "$T/h/lead.token.shown" ] && sum_pty && [ ! -s "$T/sum.tty" ] &&
  grep -q 'Checked this machine ... ok (already done' "$T/sum.stdout" &&
  grep -q 'Prerequisites ... ok (already done' "$T/sum.stdout" &&
  grep -q 'Release fetched and verified ... skipped (source checkout)' "$T/sum.stdout" &&
  grep -q 'Images loaded ... skipped (source checkout builds the server image)' "$T/sum.stdout" &&
  grep -q 'Containers started ... ok (already done' "$T/sum.stdout" &&
  grep -q 'Project and lead created ... ok (already done' "$T/sum.stdout" &&
  grep -q 'Lead credential written ... ok (already done' "$T/sum.stdout" &&
  grep -q 'Backups ... ok (already done' "$T/sum.stdout" &&
  ! grep -q 'Credential:' "$T/sum.stdout"; then
  ok 'IT-174 SUM-7 repeat checks stack and never redisplays credential'
else bad 'IT-174 SUM-7 repeat' "$(tail -15 "$T/sum.stdout")"; fi
w311_width_ok=0
if python3 -I - "$install" <<'W311_WIDTH'
from pathlib import Path
import os,re,subprocess,sys
s=Path(sys.argv[1]).read_text()
m=re.search(r'^summary_line\(\) \{\n.*?^}',s,re.M|re.S);assert m
def within(data):
    lines=data.splitlines()
    return bool(lines) and all(len(x)<=78 and all(32<=c<=126 for c in x) for x in lines)
assert within(b'control\n') and not within(b'x'*79+b'\n'),'width oracle controls'
for label,value in [('ordinary','   A reason ... '+'word '*100),('long-word','   '+'x'*400),('leading79',' '*79+'word'),('leading160',' '*160+'word')]:
    p=subprocess.run(['sh','-c',m[0]+'\nsummary_line "$1"\n','sh',value],capture_output=True,timeout=5,
        env={**os.environ,'PATH':'/usr/bin:/bin','LC_ALL':'C'})
    maximum=max(map(len,p.stdout.splitlines()),default=0)
    print('formatter',label,'native='+str(p.returncode),'max='+str(maximum))
    assert p.returncode==0 and within(p.stdout),(label,p.returncode,maximum)
W311_WIDTH
then w311_width_ok=1; fi

sum_fresh
if [ "$w311_width_ok" = 1 ] && sum_run && LC_ALL=C sed -n '/^==============================================================================$/,$p' "$T/out" |
  LC_ALL=C awk -v token_path="$T/h/lead.token" '$0 == token_path {next} length($0)>78 || /[^ -~]/ {exit 1} END {if (NR < 15) exit 1}'; then
  ok 'IT-175 SUM-8 summary is ASCII with at most 78 columns'
else bad 'IT-175 SUM-8 layout'; fi
sum_fresh
if FAKE_MIGRATE_FAIL=1 sum_run; then sum_rc=0; else sum_rc=$?; fi
if [ "$sum_rc" = 6 ] && grep -q 'What was done' "$T/out" && grep -q 'Containers started ... FAILED' "$T/out" &&
  grep -q '^problem:' "$T/sum.err" && grep -q '^fix:' "$T/sum.err" &&
  ! grep -q 'SHOWN ONCE\|Project and lead created ... ok' "$T/out" "$T/sum.err"; then
  ok 'IT-176 SUM-9 stop shows reached stages and no credential'
else bad 'IT-176 SUM-9 stop summary'; fi
sum_fresh
sum_private_channels() {
  sum_private_files "$T/h/lead.token" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log"
}
w311_marker_ok=0
if python3 -I - "$install" "$T" <<'W311_MARKER'
from pathlib import Path
import hashlib,os,re,subprocess,sys,tempfile
s=Path(sys.argv[1]).read_text();bodies=[]
for name in ['token_file_owned','token_private','summary_credential_check']:
    m=re.search(r'^'+name+r'\(\) \{\n.*?^}',s,re.M|re.S);assert m,name
    bodies.append(m[0])
def digest(p):return hashlib.sha256(p.read_bytes()).hexdigest() if p.exists() else None
with tempfile.TemporaryDirectory(prefix='w311-marker.',dir=sys.argv[2]) as work:
    for case in ['absent','valid','malformed']:
        d=Path(work)/case;d.mkdir(mode=0o700)
        t=d/'lead.token';t.write_bytes(b'ctx2_'+b'0'*32+b'.'+b'A'*43+b'\n');t.chmod(0o600)
        marker=Path(str(t)+'.shown')
        if case!='absent':
            marker.write_bytes(b'id='+b'0'*32+b'\nshown_at=20261001T000000Z\n' if case=='valid' else b'invalid marker\n')
            marker.chmod(0o600)
        before=digest(marker)
        code='token=$1\n'+'\n'.join(bodies)+'\nsummary_credential_check\n'
        p=subprocess.run(['sh','-c',code,'sh',str(t)],capture_output=True,timeout=5,
            env={**os.environ,'PATH':'/usr/bin:/bin','LC_ALL':'C'})
        after=digest(marker);expected=1 if case=='malformed' else 0
        print('marker',case,'native='+str(p.returncode),'unchanged='+str(before==after))
        assert p.returncode==expected and before==after,(case,p.returncode,before==after)
W311_MARKER
then w311_marker_ok=1; fi

: > "$T/sum.log"
if [ "$w311_marker_ok" = 1 ] && FAKE_BOOT_TOKEN=$sum_fake_token CORTEX_LOG=$T/sum.log sum_pty &&
  [ "$(tr -d '\r' < "$T/sum.tty" | grep -cxF "$sum_fake_token")" = 1 ] &&
  grep -q 'Copy the WHOLE next line' "$T/sum.tty" && sum_private_channels; then
  ok 'IT-177 SUM-10 one whole copyable token line, once; no 16-byte fragments elsewhere'
else bad 'IT-177 SUM-10 one whole token line and no fragments'; fi
sum_fresh
FAKE_BOOT_TOKEN=$sum_fake_token FAKE_RESTART_ENABLE_FAIL=1 sum_pty && sum_rc=0 || sum_rc=$?
sum_recovery=0
if [ "$sum_rc" = 6 ] && [ -s "$T/h/lead.token" ] && [ ! -e "$T/h/lead.token.shown" ] &&
  [ ! -s "$T/sum.tty" ] && sum_path_once "$T/sum.stdout" '   Token file (0600):' "$T/h/lead.token"; then
  if FAKE_BOOT_TOKEN=$sum_fake_token sum_pty &&
    [ "$(tr -d '\r' < "$T/sum.tty" | grep -cxF "$sum_fake_token")" = 1 ] &&
    [ "$(stat -c %a "$T/h/lead.token.shown")" = 600 ] &&
    grep -q '^id=' "$T/h/lead.token.shown" && grep -q '^shown_at=' "$T/h/lead.token.shown" &&
    ! grep -qF "$sum_fake_token" "$T/h/lead.token.shown" &&
    FAKE_BOOT_TOKEN=$sum_fake_token sum_pty && [ ! -s "$T/sum.tty" ]; then sum_recovery=1; fi
fi
if [ "$sum_recovery" = 1 ]; then ok 'IT-178 SUM-11 failed-after-mint recovers display once with a private non-secret marker'; else bad 'IT-178 SUM-11 display recovery marker'; fi
sum_fresh
if sum_run && sum_pty --reissue-token && sum_pty &&
  grep -q '200 ok: lead, project_lead of p' "$T/sum.stdout" && [ ! -s "$T/sum.tty" ] &&
  grep -q 'lead.token.' "$T/sum.stdout" &&
  [ "$(find "$T/h" -name 'lead.token.*.shown' | wc -l)" = 1 ]; then
  ok 'IT-179 LSUM-01 repeat selects the active reissued credential; whoami revokes old tokens'
else bad 'IT-179 LSUM-01 active credential after reissue'; fi
sum_fresh
if sum_run && chmod 0400 "$T/h/lead.token" &&
  sum_pty && [ "$(stat -c %a "$T/h/lead.token")" = 400 ] &&
  grep -q '200 ok: lead, project_lead of p' "$T/sum.stdout"; then
  ok 'IT-180 LSUM-02 existing owner-private 0400 token remains valid and unchanged'
else bad 'IT-180 LSUM-02 owner-private 0400 token'; fi
sum_fresh
FAKE_SUM_WHO_HTTP=401 sum_run && sum_rc=0 || sum_rc=$?
if [ "$sum_rc" = 6 ] && ! grep -Eq 'is installed and running|already installed and healthy|install: done[.]|Installed Cortex version:|^SAYDONE ' "$T/out"; then
  ok 'IT-181 LSUM-03 no success or done message precedes failed final checks'
else bad 'IT-181 LSUM-03 success ordering'; fi
sum_fresh
if sum_run && FAKE_SUM_CONTAINER_LATE='exited unhealthy' sum_run; then sum_rc=0; else sum_rc=$?; fi
sum_measured=0
if [ "$sum_rc" = 6 ] && grep -q 'Containers started ... FAILED' "$T/out" &&
  ! grep -q 'Containers started ... ok' "$T/out" &&
  grep -q 'Release fetched and verified ... skipped (source checkout)' "$T/out"; then
  sum_measured=1
fi
for timer in cortex-dump.timer cortex-dump-status.timer; do
  sum_fresh; sum_run || sum_measured=0
  FAKE_TIMER_DISABLED=$timer sum_run && sum_rc=0 || sum_rc=$?
  { [ "$sum_rc" = 8 ] && ! grep -q 'already installed and healthy' "$T/out"; } || sum_measured=0
  sum_fresh; sum_run || sum_measured=0
  FAKE_TIMER_LATE=$timer sum_run && sum_rc=0 || sum_rc=$?
  { [ "$sum_rc" = 6 ] && grep -q 'Backups ... FAILED' "$T/out" &&
    ! grep -q 'Backups ... ok' "$T/out"; } || sum_measured=0
done
if [ "$sum_measured" = 1 ]; then ok 'IT-182 LSUM-06 every timer measured separately in completion, enable readback and repeat'; else bad 'IT-182 LSUM-06 repeat receipts are measured'; fi

# Faults at the durable-display boundary, with no product-only test hooks.
mkdir -p "$T/bin-summary-fault"
real_mv=$(command -v mv); real_stat=$(command -v stat); real_ln=$(command -v ln); real_date=$(command -v date); real_fold=$(command -v fold); real_mkdir=$(command -v mkdir); real_rm=$(command -v rm)
export SUM_REAL_MV=$real_mv SUM_REAL_STAT=$real_stat SUM_REAL_LN=$real_ln SUM_REAL_DATE=$real_date SUM_REAL_FOLD=$real_fold SUM_REAL_MKDIR=$real_mkdir SUM_REAL_RM=$real_rm
cat > "$T/bin-summary-fault/publish" <<'FAKE'
#!/bin/sh
case ${0##*/} in ln) real_publish=$SUM_REAL_LN ;; *) real_publish=$SUM_REAL_MV ;; esac
for target in "$@"; do :; done
case $target in *.token*.shown)
  [ "${FAKE_MARKER_PUBLISH_FAIL:-0}" != 1 ] || exit 1
  "$real_publish" "$@" || exit $?
  [ "${FAKE_MARKER_INTERRUPT:-0}" != 1 ] || { kill -TERM "$PPID"; exit 0; }
  if [ "${FAKE_MARKER_TTY_FAIL:-0}" = 1 ]; then
    : > "$FAKE_STATE/marker-ready"
    while [ ! -e "$FAKE_STATE/tty-closed" ]; do sleep .05; done
  fi
  exit 0 ;;
esac
if [ "${FAKE_BOUNDARY_PHASE:-}" = dead-stdout ]; then
  : > "$FAKE_STATE/dead-stdout-ready"
  dead_wait=0
  while [ ! -e "$FAKE_STATE/dead-stdout-go" ] && [ "$dead_wait" -lt 1000 ]; do
    /usr/bin/sleep 0.02; dead_wait=$((dead_wait + 1))
  done
  exit 1
fi
case ${0##*/}:$target in mv:*.token|ln:*.token)
  if [ "${FAKE_HEAD_FIRST_SIGNAL:-0}" = 1 ]; then
    : > "$FAKE_STATE/first-signal-sent"
    kill -TERM "$PPID"
    exit 0
  fi
  if [ "${FAKE_BOUNDARY_PHASE:-}" = fresh-decoy ]; then
    printf '%s\n' "$target" > "$FAKE_STATE/candidate-reissue-path"
    printf 'unpublished fresh decoy\n' > "$target"; chmod 0600 "$target"
    "$SUM_BOUNDARY_HELPER" fresh-decoy
    exit 143
  fi ;;
esac
case ${0##*/}:$target in mv:*.token.*|ln:*.token.*)
  case ${FAKE_BOUNDARY_PHASE:-} in handler-release|handler-copy|handler-path)
    printf '%s\n' "$target" > "$FAKE_STATE/candidate-reissue-path"
    if [ "$FAKE_BOUNDARY_PHASE" != handler-copy ]; then
      "$real_publish" "$@" || exit $?
      printf '%s\n' "$target" > "$FAKE_STATE/published-reissue-path"
    fi
    : > "$FAKE_STATE/first-signal-sent"
    kill -TERM "$PPID"
    exit 0 ;;
  esac
  if [ "${FAKE_BOUNDARY_PHASE:-}" = reissue-collision ]; then
    printf '%s\n' "$target" > "$FAKE_STATE/candidate-reissue-path"
    printf 'unpublished collision decoy\n' > "$target"; chmod 0600 "$target"
    "$SUM_REAL_STAT" -c '%d:%i' "$target" > "$FAKE_STATE/decoy-inode"
    exec "$real_publish" "$@"
  fi
  if [ "${FAKE_BOUNDARY_PHASE:-}" = reservation-signal ]; then
    if [ -d "$SUM_ACCOUNT_RUNTIME/cortex-mint.reservation" ] && [ ! -L "$SUM_ACCOUNT_RUNTIME/cortex-mint.reservation" ] && [ "$("$SUM_REAL_STAT" -c %a "$SUM_ACCOUNT_RUNTIME/cortex-mint.reservation")" = 700 ] && [ "$("$SUM_REAL_STAT" -c %u "$SUM_ACCOUNT_RUNTIME/cortex-mint.reservation")" = "$(id -u)" ] && [ "$("$SUM_REAL_STAT" -c %a "$SUM_ACCOUNT_RUNTIME/cortex-mint.reservation/owner")" = 600 ]; then
      : > "$FAKE_STATE/reservation-observed"
    fi
    "$SUM_BOUNDARY_HELPER" reservation-signal
    exit 143
  fi
  if [ "${FAKE_BOUNDARY_PHASE:-}" = reissue-reservation ]; then
    printf '%s\n' "$target" > "$FAKE_STATE/candidate-reissue-path"
    "$SUM_BOUNDARY_HELPER" reissue-reservation
    exec "$real_publish" "$@"
  fi
  case ${FAKE_BOUNDARY_PHASE:-} in reissue-before|reissue-decoy|reissue-published|reissue-cleanup)
    printf '%s\n' "$target" > "$FAKE_STATE/candidate-reissue-path"
    case $FAKE_BOUNDARY_PHASE in reissue-before|reissue-decoy)
      if [ "$FAKE_BOUNDARY_PHASE" = reissue-decoy ]; then printf 'unpublished decoy\n' > "$target"; chmod 0600 "$target"; fi
      "$SUM_BOUNDARY_HELPER" "$FAKE_BOUNDARY_PHASE"
      exit 143 ;;
    esac
    "$real_publish" "$@" || exit $?
    printf '%s\n' "$target" > "$FAKE_STATE/published-reissue-path"
    if [ "$FAKE_BOUNDARY_PHASE" = reissue-published ]; then "$SUM_BOUNDARY_HELPER" reissue-published; fi
    exit 0 ;;
  esac ;;
esac
exec "$real_publish" "$@"
FAKE
cp "$T/bin-summary-fault/publish" "$T/bin-summary-fault/mv"
cp "$T/bin-summary-fault/publish" "$T/bin-summary-fault/ln"
cat > "$T/bin-summary-fault/mkdir" <<'FAKE'
#!/bin/sh
for target in "$@"; do :; done
if [ "${FAKE_BOUNDARY_PHASE:-}" = reservation-mkdir ] && [ "$target" = "$SUM_ACCOUNT_RUNTIME/cortex-mint.reservation" ]; then
  "$SUM_REAL_MKDIR" "$@" || exit $?
  : > "$FAKE_STATE/reservation-created"
  "$SUM_BOUNDARY_HELPER" reservation-mkdir
  exit 0
fi
exec "$SUM_REAL_MKDIR" "$@"
FAKE
chmod +x "$T/bin-summary-fault/mkdir"
cat > "$T/bin-summary-fault/rm" <<'RELEASE_RM'
#!/bin/sh
for target in "$@"; do :; done
if [ "${FAKE_RELEASE_STAGE_FAIL:-0}" = 1 ]; then
  case $target in *.token.part.*)
    n=$(cat "$FAKE_STATE/release-stage-removals" 2>/dev/null || echo 0)
    n=$((n+1)); printf '%s\n' "$n" > "$FAKE_STATE/release-stage-removals"
    [ "$n" != 2 ] || exit 73 ;;
  esac
fi
case ${FAKE_BOUNDARY_PHASE:-}:$target in reservation-release:"$SUM_ACCOUNT_RUNTIME/cortex-mint.reservation/owner"|handler-release:"$SUM_ACCOUNT_RUNTIME/cortex-mint.reservation/owner")
  "$SUM_REAL_RM" "$@" || exit $?
  : > "$FAKE_STATE/reservation-owner-removed"
  "$SUM_BOUNDARY_HELPER" "$FAKE_BOUNDARY_PHASE"
  exit 0 ;;
esac
exec "$SUM_REAL_RM" "$@"
RELEASE_RM
chmod +x "$T/bin-summary-fault/rm"

cat > "$T/bin-summary-fault/date" <<'FAKE'
#!/bin/sh
if [ "$*" = '-u +%Y%m%dT%H%M%SZ' ]; then
  [ "${FAKE_FIXED_STAMP:-0}" != 1 ] || { echo 20991115T030405Z; exit 0; }
  [ "${FAKE_MINT_ID_FAIL:-0}" = 0 ] || { echo 20991115T030405Z; exit 0; }
  if [ "${FAKE_MARKER_BARRIER:-0}" = 1 ]; then
    : > "$FAKE_STATE/date-$SUM_TAG"
    tries=0
    while [ ! -e "$FAKE_STATE/date-left" ] || [ ! -e "$FAKE_STATE/date-right" ]; do
      tries=$((tries + 1)); [ "$tries" -le 200 ] || exit 1; sleep .05
    done
    case $SUM_TAG in left) echo 20261115T010101Z ;; right) echo 20261115T020202Z ;; *) exit 1 ;; esac
    exit 0
  fi
  if [ -n "${FAKE_MARKER_APPEARS:-}" ]; then
    marker="$SUM_HOME/lead.token.shown"
    case $FAKE_MARKER_APPEARS in
      regular) printf 'id=0123456789abcdef0123456789abcdef\nshown_at=20000101T000000Z\n' > "$marker"; chmod 0600 "$marker" ;;
      malformed) printf 'appeared after validation\n' > "$marker"; chmod 0644 "$marker" ;;
      directory) mkdir "$marker" ;;
      symlink) printf 'unchanged link target\n' > "$FAKE_STATE/appeared-target"; "$SUM_REAL_LN" -s "$FAKE_STATE/appeared-target" "$marker" ;;
      fifo) mkfifo "$marker" ;;
      *) exit 1 ;;
    esac
    "$SUM_REAL_STAT" -c '%F:%a:%u:%i' "$marker" > "$FAKE_STATE/appeared-stat"
    if [ -f "$marker" ] && [ ! -L "$marker" ]; then cksum < "$marker" > "$FAKE_STATE/appeared-sum"; fi
  fi
fi
exec "$SUM_REAL_DATE" "$@"
FAKE
cat > "$T/bin-summary-fault/stat" <<'FAKE'
#!/bin/sh
if [ "${FAKE_RUNTIME_OWNER_FAIL:-0}" = 1 ] && [ "$*" = "-c %u $SUM_ACCOUNT_RUNTIME" ]; then echo 999999; exit 0; fi
case "$*" in '-c %d:%i '*.part*) case ${FAKE_MINT_ID_FAIL:-0} in 1) exit 1 ;; empty) exit 0 ;; esac ;; esac
case "$*" in '-c %u '*.shown)
  [ "${FAKE_MARKER_OWNER:-0}" != 1 ] || { echo 999999; exit 0; } ;;
esac
if [ "${FAKE_BOUNDARY_PHASE:-}" = after-write ] && [ -e "$FAKE_STATE/header-done" ] && [ ! -e "$FAKE_STATE/boundary-delivered" ]; then
  case "$*" in '-c %a '*)
    "$SUM_REAL_STAT" "$@" || exit $?
    "$SUM_BOUNDARY_HELPER" after-write
    exit 0 ;;
  esac
fi
exec "$SUM_REAL_STAT" "$@"
FAKE
cat > "$T/bin-summary-fault/boundary" <<'FAKE'
#!/bin/sh
printf '%s\n' "$1" > "$FAKE_STATE/boundary-ready"
while [ ! -e "$FAKE_STATE/boundary-release" ]; do /usr/bin/sleep .01; done
FAKE
SUM_BOUNDARY_HELPER=$T/bin-summary-fault/boundary
export SUM_BOUNDARY_HELPER
cat > "$T/bin-summary-fault/fold" <<'FAKE'
#!/bin/sh
if [ "${FAKE_ABORT_PRINT_BARRIER:-0}" = 1 ] && [ ! -e "$FAKE_STATE/abort-print-ready" ]; then
  : > "$FAKE_STATE/abort-print-ready"
  abort_print_wait=0
  while [ ! -e "$FAKE_STATE/abort-print-go" ] && [ "$abort_print_wait" -lt 1000 ]; do
    /usr/bin/sleep 0.02; abort_print_wait=$((abort_print_wait+1))
  done
fi
if [ "${FAKE_BOUNDARY_PHASE:-}" = handler-path ]; then
  text=$(cat)
  printf '%s\n' "$text" | "$SUM_REAL_FOLD" "$@" || exit $?
  case $text in '   Token file ('*)
    if [ -e "$FAKE_STATE/first-signal-sent" ] && [ ! -e "$FAKE_STATE/boundary-delivered" ]; then "$SUM_BOUNDARY_HELPER" handler-path; fi ;;
  esac
  exit 0
fi
case ${FAKE_BOUNDARY_PHASE:-} in after-header|after-write) ;; *) exec "$SUM_REAL_FOLD" "$@" ;; esac
text=$(cat)
printf '%s\n' "$text" | "$SUM_REAL_FOLD" "$@" || exit $?
if [ "$text" = '==============================================================================' ]; then
  n=$(cat "$FAKE_STATE/header-borders" 2>/dev/null || echo 0)
  n=$((n + 1)); printf '%s\n' "$n" > "$FAKE_STATE/header-borders"
  if [ "$n" = 2 ]; then
    : > "$FAKE_STATE/header-done"
    if [ "$FAKE_BOUNDARY_PHASE" = after-header ]; then "$SUM_BOUNDARY_HELPER" after-header; fi
  fi
fi
FAKE
cat > "$T/bin-summary-fault/timeout" <<'FAKE'
#!/bin/sh
[ "${FAKE_PROJECT_LOOKUP_STALL:-0}" != 1 ] || : > "$FAKE_STATE/timeout-from-PATH"
exec /usr/bin/timeout "$@"
FAKE
chmod +x "$T/bin-summary-fault/boundary" "$T/bin-summary-fault/fold" "$T/bin-summary-fault/mv" "$T/bin-summary-fault/ln" "$T/bin-summary-fault/stat" "$T/bin-summary-fault/date" "$T/bin-summary-fault/timeout"
PATH="$T/bin-summary-fault:$PATH"
sum_no_success() { ! grep -Eq 'is installed and running|already installed and healthy|install: done[.]|Installed Cortex version:|^SAYDONE ' "$T/sum.stdout" "$T/sum.stderr"; }
sum_fresh
FAKE_BOOT_TOKEN=$sum_fake_token FAKE_MARKER_PUBLISH_FAIL=1 sum_pty && sum_rc=0 || sum_rc=$?
if [ "$sum_rc" = 7 ] && sum_no_success && [ ! -s "$T/sum.tty" ] &&
  grep -q 'lead.token' "$T/sum.stdout" && grep -q '^problem:' "$T/sum.stderr" &&
  grep -q '^fix:' "$T/sum.stderr" && [ ! -e "$T/h/lead.token.shown" ]; then
  ok 'IT-183 SUM-12 failed marker publication prevents all success and credential output'
else bad 'IT-183 SUM-12 marker write must precede success and display' "rc=$sum_rc"; fi
sum_marker_bad=0
for marker_case in malformed mode owner id type link time trailing; do
  sum_fresh
  if ! FAKE_BOOT_TOKEN=$sum_fake_token sum_run; then
    sum_marker_bad=$((sum_marker_bad + 1)); continue
  fi
  marker="$T/h/lead.token.shown"
  if ! { printf 'id=0123456789abcdef0123456789abcdef\nshown_at=20261115T000000Z\n' > "$marker" && chmod 0600 "$marker"; }; then
    sum_marker_bad=$((sum_marker_bad + 1)); continue
  fi
  if ! case $marker_case in
    malformed) printf 'bad marker\n' > "$marker" ;;
    mode) chmod 0644 "$marker" ;;
    id) sed -i 's/0123456789abcdef0123456789abcdef/ffffffffffffffffffffffffffffffff/' "$marker" ;;
    type) rm "$marker" && mkdir "$marker" ;;
    link) mv "$marker" "$T/marker-link-target" && ln -s "$T/marker-link-target" "$marker" ;;
    time) sed -i 's/20261115T000000Z/xxxxxxxxTxxxxxxZ/' "$marker" ;;
    trailing) printf 'extra without newline' >> "$marker" ;;
  esac; then sum_marker_bad=$((sum_marker_bad + 1)); continue; fi
  if ! before=$(stat -c '%F:%a:%u' "$marker"); then sum_marker_bad=$((sum_marker_bad + 1)); continue; fi
  sum_before=$(cksum "$marker" 2>/dev/null || :)
  if [ "$marker_case" = owner ]; then FAKE_MARKER_OWNER=1 sum_pty --no-show-credentials && sum_rc=0 || sum_rc=$?
  else sum_pty && sum_rc=0 || sum_rc=$?; fi
  { [ "$sum_rc" = 7 ] && sum_no_success && [ ! -s "$T/sum.tty" ] &&
    [ "$before" = "$(stat -c '%F:%a:%u' "$marker")" ] &&
    [ "$sum_before" = "$(cksum "$marker" 2>/dev/null || :)" ]; } || sum_marker_bad=$((sum_marker_bad + 1))
done
if [ "$sum_marker_bad" = 0 ]; then ok 'IT-184 SUM-13 invalid marker fails untouched, including suppressed display'; else bad 'IT-184 SUM-13 invalid marker' "$sum_marker_bad cases"; fi
sum_fresh
FAKE_BOOT_TOKEN=$sum_fake_token FAKE_MARKER_INTERRUPT=1 sum_pty && sum_rc=0 || sum_rc=$?
if [ "$sum_rc" != 0 ] && [ -f "$T/h/lead.token.shown" ] && [ ! -s "$T/sum.tty" ] &&
  sum_pty && [ ! -s "$T/sum.tty" ] && grep -q -- '--reissue-token' "$T/sum.stdout" &&
  grep -q 'lead.token' "$T/sum.stdout"; then
  ok 'IT-185 SUM-14 interruption after reservation never redisplays on recovery'
else bad 'IT-185 SUM-14 interruption boundary' "rc=$sum_rc"; fi
sum_fresh
: > "$T/sum.log"
FAKE_BOOT_TOKEN=$sum_fake_token FAKE_MARKER_TTY_FAIL=1 CORTEX_LOG=$T/sum.log sum_pty && sum_rc=0 || sum_rc=$?
if [ "$sum_rc" = 0 ] && [ ! -s "$T/sum.tty" ] &&
  sum_path_once "$T/sum.stdout" " Credential could not be shown. It is in this file (0600):" "$T/h/lead.token" &&
  sum_path_total_once "$T/sum.stdout" "$T/h/lead.token" 'Install log:' "$T/sum.log" &&
  sum_private_files "$T/h/lead.token" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log" &&
  [ -f "$T/h/lead.token.shown" ] && CORTEX_LOG=$T/sum.log sum_pty && [ ! -s "$T/sum.tty" ] &&
  sum_private_files "$T/h/lead.token" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log"; then
  ok 'IT-186 SUM-15 opened tty write fails after marker; install succeeds without redisplay'
else bad 'IT-186 SUM-15 tty write failure' "rc=$sum_rc"; fi
sum_fresh
: > "$T/sum.log"
FAKE_BOOT_TOKEN=$sum_fake_token FAKE_WHO_PROJECT=22222222-2222-4222-8222-222222222222 CORTEX_LOG=$T/sum.log sum_run && sum_rc=0 || sum_rc=$?
sum_project_ok=1
{ [ "$sum_rc" = 6 ] && grep -q 'Sign-in (/v2/whoami) ... FAILED' "$T/out" &&
  grep -q 'installed but NOT working' "$T/out" && ! grep -q 'project_lead of\|project lead of' "$T/out" &&
  sum_private_files "$T/h/lead.token" "$T/out" "$T/sum.err" "$T/sum.log" "$T/log"; } || sum_project_ok=0
sum_fresh
# Both tokens authenticate as the same-named lead, but only the base belongs to p.
if FAKE_BOOT_TOKEN=$sum_fake_token CORTEX_LOG=$T/sum.log sum_run &&
  sum_private_files "$T/h/lead.token" "$T/out" "$T/sum.err" "$T/sum.log" "$T/log" &&
  printf '%s\n' 'FOREIGN-ACTIVE-TOKEN' > "$T/h/lead.token.20991115T000000Z" &&
  chmod 0600 "$T/h/lead.token.20991115T000000Z" &&
  cp "$T/h/lead.token.20991115T000000Z" "$T/state/other.token"; then
  { CORTEX_LOG=$T/sum.log sum_run && sum_path_once "$T/out" "   Token file (0600):" "$T/h/lead.token" &&
    grep -q '200 ok: lead, project_lead of p' "$T/out" &&
    sum_private_files "$T/h/lead.token" "$T/out" "$T/sum.err" "$T/sum.log" "$T/log" &&
    sum_private_files "$T/state/other.token" "$T/out" "$T/sum.err" "$T/sum.log" "$T/log"; } || sum_project_ok=0
FAKE_PROJECT_LOOKUP_FAIL=1 CORTEX_LOG=$T/sum.log sum_run && sum_rc=0 || sum_rc=$?
{ [ "$sum_rc" = 6 ] && ! grep -q 'project_lead of\|project lead of' "$T/out" &&
  sum_private_files "$T/h/lead.token" "$T/out" "$T/sum.err" "$T/sum.log" "$T/log" &&
  sum_private_files "$T/state/other.token" "$T/out" "$T/sum.err" "$T/sum.log" "$T/log"; } || sum_project_ok=0
else
  sum_project_ok=0
fi
if [ "$sum_project_ok" = 1 ]; then ok 'IT-187 SUM-16 own lookup ID binds whoami and token selection; missing lookup fails'; else bad 'IT-187 SUM-16 project binding'; fi
sum_fresh
sum_race_ok=0
if FAKE_BOOT_TOKEN=$sum_fake_token sum_run; then
  FAKE_MARKER_BARRIER=1 SUM_TAG=left sum_pty & sum_left_pid=$!
  FAKE_MARKER_BARRIER=1 SUM_TAG=right sum_pty & sum_right_pid=$!
  wait "$sum_left_pid" && sum_left_rc=0 || sum_left_rc=$?
  wait "$sum_right_pid" && sum_right_rc=0 || sum_right_rc=$?
  sum_left_count=$(tr -d '\r' < "$T/left.tty" | grep -cxF "$sum_fake_token" || :)
  sum_right_count=$(tr -d '\r' < "$T/right.tty" | grep -cxF "$sum_fake_token" || :)
  if [ "$sum_left_rc" = 0 ] && [ "$sum_right_rc" = 0 ] && [ "$((sum_left_count + sum_right_count))" = 1 ]; then
    if [ "$sum_left_count" = 1 ]; then sum_winner_time=20261115T010101Z; sum_loser=right; else sum_winner_time=20261115T020202Z; sum_loser=left; fi
    if grep -qx "shown_at=$sum_winner_time" "$T/h/lead.token.shown" &&
      grep -q -- '--reissue-token' "$T/$sum_loser.stdout" &&
      ! grep -qF "$sum_fake_token" "$T/left.stdout" "$T/left.stderr" "$T/right.stdout" "$T/right.stderr" "$T/log"; then sum_race_ok=1; fi
  fi
fi
if [ "$sum_race_ok" = 1 ]; then ok 'IT-188 SUM-17 exactly one concurrent terminal run publishes and shows; winning record retained'; else bad 'IT-188 SUM-17 exclusive publication under two contenders'; fi
sum_appeared_bad=0
for appeared_case in regular malformed directory symlink fifo; do
  sum_fresh
  if ! FAKE_BOOT_TOKEN=$sum_fake_token sum_run; then sum_appeared_bad=$((sum_appeared_bad + 1)); continue; fi
  FAKE_MARKER_APPEARS=$appeared_case sum_pty && sum_rc=0 || sum_rc=$?
  marker="$T/h/lead.token.shown"
  if [ "$sum_rc" != 0 ] || [ -s "$T/sum.tty" ] ||
    ! grep -q -- '--reissue-token' "$T/sum.stdout" ||
    [ "$(cat "$T/state/appeared-stat" 2>/dev/null || :)" != "$(stat -c '%F:%a:%u:%i' "$marker" 2>/dev/null || :)" ]; then sum_appeared_bad=$((sum_appeared_bad + 1)); continue; fi
  if [ -f "$T/state/appeared-sum" ]; then
    [ "$(cat "$T/state/appeared-sum")" = "$(cksum < "$marker")" ] || sum_appeared_bad=$((sum_appeared_bad + 1))
  fi
  [ "$appeared_case" != directory ] || [ -z "$(ls -A "$marker")" ] || sum_appeared_bad=$((sum_appeared_bad + 1))
done
if [ "$sum_appeared_bad" = 0 ]; then ok 'IT-189 SUM-18 appeared regular malformed directory symlink and FIFO marker names stay untouched'; else bad 'IT-189 SUM-18 marker appears after validation' "$sum_appeared_bad cases"; fi
sum_timer_bad=0
for timer in cortex-dump.timer cortex-dump-status.timer; do
  for timer_state in enabled-runtime static indirect alias; do
    sum_fresh
    if ! sum_run; then sum_timer_bad=$((sum_timer_bad + 1)); continue; fi
    FAKE_TIMER_SELECTED=$timer FAKE_TIMER_STATE=$timer_state sum_run && sum_rc=0 || sum_rc=$?
    { [ "$sum_rc" = 8 ] && ! grep -q 'already installed and healthy\|is installed and running' "$T/out" &&
      [ "$(grep -c '^problem:' "$T/sum.err")" = 1 ] && [ "$(grep -c '^fix:' "$T/sum.err")" = 1 ]; } || sum_timer_bad=$((sum_timer_bad + 1))
  done
  sum_fresh
  if sum_run; then
    FAKE_TIMER_LATE=$timer FAKE_TIMER_LATE_STATE=enabled-runtime sum_run && sum_rc=0 || sum_rc=$?
    { [ "$sum_rc" = 6 ] && grep -q 'Backups ... FAILED' "$T/out" &&
      ! grep -q 'Backups ... ok' "$T/out"; } || sum_timer_bad=$((sum_timer_bad + 1))
  else sum_timer_bad=$((sum_timer_bad + 1)); fi
done
if [ "$sum_timer_bad" = 0 ]; then ok 'IT-190 LSUM-09 both timers require persistent enabled state in completion/readback/repeat'; else bad 'IT-190 LSUM-09 timer state words' "$sum_timer_bad cases"; fi
sum_fresh
sum_stall_ok=0
if sum_run; then
  FAKE_PROJECT_LOOKUP_STALL=1 SUM_STALL_INSTALL=$install SUM_STALL_HOME=$T/h SUM_STALL_OUT=$T/out SUM_STALL_ERR=$T/sum.err \
  python3 -I -c '
import os, pathlib, signal, subprocess, time
state = pathlib.Path(os.environ["FAKE_STATE"])
start = time.monotonic()
with open(os.environ["SUM_STALL_OUT"], "wb") as out, open(os.environ["SUM_STALL_ERR"], "wb") as err:
    result = subprocess.run(["/usr/bin/timeout", "--kill-after=1s", "12s", "sh", os.environ["SUM_STALL_INSTALL"], "--project", "p", "--lead", "lead", "--home", os.environ["SUM_STALL_HOME"]], stdout=out, stderr=err)
elapsed = time.monotonic() - start
pidfile = state / "lookup.pid"
started = pidfile.is_file()
alive = False
if started:
    pid_text, expected_start = pidfile.read_text().split()
    pid = int(pid_text)
    proc = pathlib.Path("/proc", str(pid))
    for attempt in range(20):
        try:
            fields = proc.joinpath("stat").read_text().rsplit(") ", 1)[1].split()
            if fields[19] != expected_start or proc.stat().st_uid != os.getuid():
                raise RuntimeError("refusing cleanup of an unowned process generation")
            alive = fields[0] != "Z"
        except FileNotFoundError:
            alive = False
        if not alive: break
        time.sleep(.01)
    if alive:
        try:
            fd = os.pidfd_open(pid)
            try:
                fields = proc.joinpath("stat").read_text().rsplit(") ", 1)[1].split()
                if fields[19] != expected_start: raise RuntimeError("process generation changed")
                signal.pidfd_send_signal(fd, signal.SIGKILL)
            finally: os.close(fd)
        except ProcessLookupError: pass
print("lookup stall: rc=%d elapsed=%.2fs started=%s alive_after_run=%s" % (result.returncode, elapsed, started, alive))
raise SystemExit(0 if result.returncode == 6 and elapsed < 10 and started and not alive and not (state / "timeout-from-PATH").exists() else 1)
' && sum_stall_ok=1 || sum_stall_ok=0
fi
if [ "$sum_stall_ok" = 1 ] && [ "$(grep -c '^problem:' "$T/sum.err")" = 1 ] &&
  [ "$(grep -c '^fix:' "$T/sum.err")" = 1 ] &&
  ! grep -Eq 'is installed and running|already installed and healthy|install: done[.]' "$T/out"; then
  ok 'IT-191 LSUM-10 a TERM-ignoring stalled lookup is bounded and only its own process ends'
else bad 'IT-191 LSUM-10 host lookup deadline'; fi
sum_lookup_bad=0
for lookup_failure in TERM INT HUP USR1 4 123 125 126 127 128 255; do
  sum_fresh
  if ! sum_run; then sum_lookup_bad=$((sum_lookup_bad + 1)); continue; fi
  : > "$T/log"; : > "$T/state/systemctl.log"
  case $lookup_failure in
    TERM|INT|HUP|USR1) FAKE_PROJECT_LOOKUP_SIGNAL=$lookup_failure sum_run && sum_rc=0 || sum_rc=$? ;;
    *) FAKE_PROJECT_LOOKUP_EXIT=$lookup_failure sum_run && sum_rc=0 || sum_rc=$? ;;
  esac
  if [ "$sum_rc" != 6 ] || [ "$(grep -c '^problem:' "$T/sum.err")" != 1 ] ||
    [ "$(grep -c '^fix:' "$T/sum.err")" != 1 ] ||
    ! grep -q 'Project identity lookup ... FAILED' "$T/out" ||
    grep -Eq '^   (Checked this machine|Verified prerequisites|Prerequisites|Images loaded|Containers started)|is installed and running|already installed and healthy' "$T/out" ||
    grep -Eq '^compose |^secret create |^rm |exec .*bootstrap|exec .*psql -U' "$T/log" ||
    [ -s "$T/state/systemctl.log" ]; then
    echo "lookup terminal case $lookup_failure: rc=$sum_rc repair_calls=$(grep -Ec '^compose |^secret create |^rm |exec .*bootstrap|exec .*psql -U' "$T/log" || :)"
    sum_lookup_bad=$((sum_lookup_bad + 1))
  fi
done
if [ "$sum_lookup_bad" = 0 ]; then ok 'IT-192 LSUM-11 signalled and unknown lookup failures are terminal before any repair'; else bad 'IT-192 LSUM-11 terminal lookup failure' "$sum_lookup_bad cases"; fi
sum_db_bad=0
for lookup_db_error in 1 2 3; do
  sum_fresh
  if ! sum_run; then sum_db_bad=$((sum_db_bad + 1)); continue; fi
  : > "$T/log"
  FAKE_PROJECT_LOOKUP_EXIT=$lookup_db_error sum_run && sum_rc=0 || sum_rc=$?
  { [ "$sum_rc" = 6 ] && grep -q '^compose .* up -d db$' "$T/log" &&
    grep -q 'Sign-in (/v2/whoami) ... FAILED' "$T/out" &&
    ! grep -q 'Project identity lookup ... FAILED' "$T/out" &&
    [ "$(grep -c '^problem:' "$T/sum.err")" = 1 ] &&
    [ "$(grep -c '^fix:' "$T/sum.err")" = 1 ]; } || sum_db_bad=$((sum_db_bad + 1))
done
if [ "$sum_db_bad" = 0 ]; then ok 'IT-193 LSUM-11 psql own errors retain handled unfinished-install and final refusal behavior'; else bad 'IT-193 LSUM-11 handled database errors' "$sum_db_bad cases"; fi
# IT-197 SUM-19: the actual long token path must be copyable in every display mode.
sum_home=$T/long-summary-home-xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx/h
sum_path_bad=0
for sum_path_mode in no-terminal suppressed repeat valid-marker reissue tty-success tty-failure stop-after-mint final-failure; do
  sum_fresh
  : > "$T/sum.log"
  sum_path=$sum_home/lead.token
  sum_path_rc=0
  sum_path_expect=0
  case $sum_path_mode in
    no-terminal) FAKE_BOOT_TOKEN=$sum_fake_token CORTEX_LOG=$T/sum.log sum_run || sum_path_rc=$? ;;
    suppressed) FAKE_BOOT_TOKEN=$sum_fake_token CORTEX_LOG=$T/sum.log sum_run --no-show-credentials || sum_path_rc=$? ;;
    repeat)
      FAKE_BOOT_TOKEN=$sum_fake_token CORTEX_LOG=$T/sum.log sum_pty || sum_path_rc=$?
      CORTEX_LOG=$T/sum.log sum_run || sum_path_rc=$? ;;
    valid-marker)
      FAKE_BOOT_TOKEN=$sum_fake_token CORTEX_LOG=$T/sum.log sum_pty || sum_path_rc=$?
      CORTEX_LOG=$T/sum.log sum_pty || sum_path_rc=$?
      cp "$T/sum.stdout" "$T/out"; cp "$T/sum.stderr" "$T/sum.err"
      [ ! -s "$T/sum.tty" ] || sum_path_rc=1 ;;
    stop-after-mint)
      sum_path_expect=6
      FAKE_BOOT_TOKEN=$sum_fake_token FAKE_RESTART_ENABLE_FAIL=1 CORTEX_LOG=$T/sum.log sum_pty && sum_path_rc=0 || sum_path_rc=$?
      cp "$T/sum.stdout" "$T/out"; cp "$T/sum.stderr" "$T/sum.err"
      [ ! -s "$T/sum.tty" ] || sum_path_rc=1 ;;
    final-failure)
      sum_path_expect=6
      FAKE_BOOT_TOKEN=$sum_fake_token FAKE_SUM_HEALTH_HTTP=503 CORTEX_LOG=$T/sum.log sum_run && sum_path_rc=0 || sum_path_rc=$? ;;
    reissue)
      FAKE_BOOT_TOKEN=$sum_fake_token CORTEX_LOG=$T/sum.log sum_run || sum_path_rc=$?
      FAKE_BOOT_TOKEN=$sum_fake_token CORTEX_LOG=$T/sum.log sum_run --reissue-token || sum_path_rc=$?
      sum_path=$(find "$sum_home" -maxdepth 1 -type f -name 'lead.token.*' ! -name '*.shown' | head -1) ;;
    tty-success)
      FAKE_BOOT_TOKEN=$sum_fake_token CORTEX_LOG=$T/sum.log sum_pty || sum_path_rc=$?
      cp "$T/sum.stdout" "$T/out"; cp "$T/sum.stderr" "$T/sum.err"
      [ "$(tr -d '\r' < "$T/sum.tty" | grep -cxF "$sum_fake_token")" = 1 ] || sum_path_rc=1 ;;
    tty-failure)
      FAKE_BOOT_TOKEN=$sum_fake_token FAKE_MARKER_TTY_FAIL=1 CORTEX_LOG=$T/sum.log sum_pty || sum_path_rc=$?
      cp "$T/sum.stdout" "$T/out"; cp "$T/sum.stderr" "$T/sum.err"
      { [ ! -s "$T/sum.tty" ] && [ -f "$sum_path.shown" ] &&
        sum_path_once "$T/out" ' Credential could not be shown. It is in this file (0600):' "$sum_path"; } || sum_path_rc=1 ;;
  esac
  if [ "$sum_path_rc" != "$sum_path_expect" ] || [ "${#sum_path}" -le 78 ] ||
    ! sum_path_total_once "$T/out" "$sum_path" 'Install log:' "$T/sum.log" ||
    { [ "$sum_path_mode" != tty-failure ] && ! sum_path_once "$T/out" '   Token file (0600):' "$sum_path"; } ||
    ! sum_private_files "$sum_path" "$T/out" "$T/sum.err" "$T/sum.log" "$T/log" ||
    { [ "$sum_path_expect" = 0 ] && ! LC_ALL=C sed -n '/^==============================================================================$/,$p' "$T/out" |
      LC_ALL=C awk -v token_path="$sum_path" -v log_path="$T/sum.log" '$0 == token_path || $0 == log_path {next} length($0)>78 || /[^ -~]/ {exit 1} END {if (NR < 15) exit 1}'; }; then
    echo "long-path mode $sum_path_mode failed (rc=$sum_path_rc)"
    sum_path_bad=$((sum_path_bad + 1))
  fi
done
sum_fresh
sum_home=$T/h
if [ "$sum_path_bad" = 0 ]; then ok 'IT-197 SUM-19 exact unwrapped column-one token paths across no-terminal suppression repeat reissue tty-success and tty-write-failure; ordinary lines stay within 78'; else bad 'IT-197 SUM-19 copyable long paths' "$sum_path_bad modes"; fi
if [ "$sum_path_bad" = 0 ]; then ok 'IT-198 SUM-20 exactly one standalone token path across all stdout and no credential value fragments in any public channel for all nine modes'; else bad 'IT-198 SUM-20 whole-output path/value guards' "$sum_path_bad modes"; fi
sum_interrupt_bad=0
sum_signal_original_install=$install
for sum_boundary in after-header after-write after-path-write; do
  install=$sum_signal_original_install
  if [ "$sum_boundary" = after-path-write ]; then
    copy_deploy "$(dirname -- "$install")" "$T/signal-deploy"
    if python3 -I - "$T/signal-deploy/install.sh" <<'PYBOUND'
from pathlib import Path
import sys
p = Path(sys.argv[1])
s = p.read_text()
needle = '  printf \'%s\\n\' "$2"'
# A labelled public-path write only; the secret-to-tty write is never modified.
if s.count(needle) != 1:
    raise SystemExit(1)
extra = '\n  sum_injected_status=$?\n  if [ "${FAKE_BOUNDARY_PHASE:-}" = after-path-write ] && [ "$2" = "${token:-}" ] && [ ! -e "$FAKE_STATE/boundary-delivered" ]; then "$SUM_BOUNDARY_HELPER" after-path-write; fi\n  (exit "$sum_injected_status")'
p.write_text(s.replace(needle, needle + extra))
PYBOUND
    then install=$T/signal-deploy/install.sh
    else echo 'public-path boundary instrumentation could not apply once'; sum_interrupt_bad=$((sum_interrupt_bad + 1)); continue; fi
  fi
  for sum_signal in HUP INT TERM; do
    sum_fresh; : > "$T/sum.log"
    case $sum_signal in HUP) sum_expect=129 ;; INT) sum_expect=130 ;; TERM) sum_expect=143 ;; esac
    FAKE_BOOT_TOKEN=$sum_fake_token FAKE_BOUNDARY_PHASE=$sum_boundary FAKE_BOUNDARY_SIGNAL=$sum_signal CORTEX_LOG=$T/sum.log sum_pty && sum_rc=0 || sum_rc=$?
    sum_first=$(tr -d '\r' < "$T/sum.tty" | grep -cxF "$sum_fake_token" || :)
    sum_marker_before=$(cksum "$T/h/lead.token.shown" 2>/dev/null || :)
    sum_boundary_ok=1
    { [ "$sum_rc" = "$sum_expect" ] && [ -e "$T/state/boundary-delivered" ] &&
      [ -f "$T/h/lead.token.shown" ] &&
      sum_path_once "$T/sum.stdout" '   Token file (0600):' "$T/h/lead.token" &&
      sum_path_total_once "$T/sum.stdout" "$T/h/lead.token" 'Install log:' "$T/sum.log" &&
      sum_private_files "$T/h/lead.token" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log"; } || sum_boundary_ok=0
    case $sum_boundary in after-header) [ "$sum_first" = 0 ] || sum_boundary_ok=0 ;; after-write|after-path-write) [ "$sum_first" = 1 ] || sum_boundary_ok=0 ;; esac
    { CORTEX_LOG=$T/sum.log sum_pty && [ ! -s "$T/sum.tty" ] &&
      [ "$sum_marker_before" = "$(cksum "$T/h/lead.token.shown")" ] &&
      sum_path_total_once "$T/sum.stdout" "$T/h/lead.token" 'Install log:' "$T/sum.log" &&
      sum_private_files "$T/h/lead.token" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log"; } || sum_boundary_ok=0
    if [ "$sum_boundary_ok" != 1 ]; then
      echo "signal boundary $sum_boundary/$sum_signal failed (rc=$sum_rc first_display=$sum_first)"
      sum_interrupt_bad=$((sum_interrupt_bad + 1))
    fi
  done
done
install=$sum_signal_original_install
if [ "$sum_interrupt_bad" = 0 ]; then ok 'IT-200 trapped HUP INT TERM after header or tty write retain exactly one recovery pair and never redisplay'; else bad 'IT-200 deferred path remains armed across signals' "$sum_interrupt_bad boundaries"; fi
sum_reissue_signal_bad=0
for sum_boundary in reissue-before reissue-decoy reissue-published reissue-cleanup; do
  sum_fresh; : > "$T/sum.log"
  sum_boundary_ok=1
  FAKE_BOOT_TOKEN=$sum_fake_token CORTEX_LOG=$T/sum.log sum_run || sum_boundary_ok=0
  sum_old_before=$(cksum "$T/h/lead.token")
  FAKE_BOOT_TOKEN=$sum_reissue_value FAKE_BOUNDARY_PHASE=$sum_boundary FAKE_BOUNDARY_SIGNAL=TERM CORTEX_LOG=$T/sum.log sum_pty --reissue-token && sum_rc=0 || sum_rc=$?
  sum_candidate=$(cat "$T/state/candidate-reissue-path" 2>/dev/null || :)
  [ -n "$sum_candidate" ] && [ "$sum_rc" = 143 ] && [ -e "$T/state/boundary-delivered" ] && [ ! -s "$T/sum.tty" ] || sum_boundary_ok=0
  sum_both_private "$T/h/lead.token" "$T/state/active.token" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log" || sum_boundary_ok=0
  if [ "$sum_boundary" = reissue-before ] || [ "$sum_boundary" = reissue-decoy ]; then
    if [ "$sum_boundary" = reissue-before ]; then [ ! -e "$sum_candidate" ] || sum_boundary_ok=0
    else grep -qxF 'unpublished decoy' "$sum_candidate" || sum_boundary_ok=0; fi
    {
      ! grep -qF "$T/h/lead.token" "$T/sum.stdout" &&
      ! grep -q 'Token file\|Credential could not be shown' "$T/sum.stdout" &&
      sum_no_file_notice "$T/sum.stdout" 1 &&
      [ "$sum_old_before" = "$(cksum "$T/h/lead.token")" ] &&
      sum_no_stages "$T/h" && [ ! -e "$T/state/bootstrap.token" ] &&
      ! grep -qF "$sum_candidate" "$T/sum.stdout" &&
      sum_private_files "$T/state/active.token" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log"; } || sum_boundary_ok=0
  else
    { [ -f "$sum_candidate" ] && [ ! -e "$sum_candidate.shown" ] && [ ! -e "$T/state/bootstrap.token" ] &&
      sum_path_once "$T/sum.stdout" '   Token file (0600):' "$sum_candidate" &&
      sum_path_total_once "$T/sum.stdout" "$sum_candidate" 'Install log:' "$T/sum.log" &&
      sum_private_files "$sum_candidate" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log"; } || sum_boundary_ok=0
    { CORTEX_LOG=$T/sum.log sum_pty &&
      [ "$(tr -d '\r' < "$T/sum.tty" | grep -cxF "$sum_reissue_value")" = 1 ] &&
      sum_path_total_once "$T/sum.stdout" "$sum_candidate" 'Install log:' "$T/sum.log" &&
      sum_both_private "$T/h/lead.token" "$sum_candidate" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log" &&
      CORTEX_LOG=$T/sum.log sum_pty && [ ! -s "$T/sum.tty" ] &&
      sum_both_private "$T/h/lead.token" "$sum_candidate" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log"; } || sum_boundary_ok=0
  fi
  sum_private_files "$T/h/lead.token" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log" || sum_boundary_ok=0
  if [ "$sum_boundary_ok" != 1 ]; then
    echo "reissue signal boundary $sum_boundary failed (rc=$sum_rc)"
    sum_reissue_signal_bad=$((sum_reissue_signal_bad + 1))
  fi
done
if [ "$sum_reissue_signal_bad" = 0 ]; then ok 'IT-201 reissue publication and cleanup signals name only the published destination; prepublication control never names an unpublished file; recovery displays once'; else bad 'IT-201 published reissue recovery path' "$sum_reissue_signal_bad boundaries"; fi
sum_id_ok=1
for sum_id_failure in 1 empty; do
  sum_fresh; : > "$T/sum.log"
FAKE_BOOT_TOKEN=$sum_fake_token CORTEX_LOG=$T/sum.log sum_run || sum_id_ok=0
sum_old_before=$(cksum "$T/h/lead.token")
FAKE_BOOT_TOKEN=$sum_reissue_value FAKE_MINT_ID_FAIL=$sum_id_failure CORTEX_LOG=$T/sum.log sum_pty --reissue-token && sum_rc=0 || sum_rc=$?
sum_unpublished=$T/h/lead.token.20991115T030405Z
{ [ "$sum_rc" = 7 ] && [ ! -s "$T/sum.tty" ] &&
  [ ! -e "$sum_unpublished" ] && sum_no_stages "$T/h" &&
  [ ! -e "$T/state/bootstrap.token" ] && [ "$sum_old_before" = "$(cksum "$T/h/lead.token")" ] &&
  [ "$(grep -c '^problem:' "$T/sum.stderr")" = 1 ] && [ "$(grep -c '^fix:' "$T/sum.stderr")" = 1 ] &&
  ! grep -qF "$T/h/lead.token" "$T/sum.stdout" &&
  ! grep -q 'Token file\|Credential could not be shown' "$T/sum.stdout" &&
  sum_no_file_notice "$T/sum.stdout" 1 &&
  ! grep -qF "$sum_unpublished" "$T/sum.stdout" &&
  sum_both_private "$T/h/lead.token" "$T/state/active.token" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log"; } || sum_id_ok=0
done
if [ "$sum_id_ok" = 1 ]; then ok 'IT-202 missing staged-file identity refuses before publication and removes both private copies without claiming a new path'; else bad 'IT-202 mint identity failure cleanup' "rc=$sum_rc"; fi
# D8-MIKE-05: a real fresh destination decoy and an inherited lowercase token.
sum_env_bad=0
sum_fresh; : > "$T/sum.log"
FAKE_BOOT_TOKEN=$sum_reissue_value FAKE_BOUNDARY_PHASE=fresh-decoy CORTEX_LOG=$T/sum.log sum_pty && sum_rc=0 || sum_rc=$?
sum_decoy=$T/h/lead.token
{ [ "$sum_rc" = 143 ] && grep -qxF 'unpublished fresh decoy' "$sum_decoy" &&
  sum_no_stages "$T/h" && [ ! -e "$T/state/bootstrap.token" ] && [ ! -s "$T/sum.tty" ] &&
  sum_no_file_notice "$T/sum.stdout" 1 &&
  sum_path_absent "$sum_decoy" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log" &&
  sum_private_files "$T/state/active.token" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log"; } || sum_env_bad=$((sum_env_bad+1))
sum_fresh; : > "$T/sum.log"
FAKE_BOOT_TOKEN=$sum_fake_token CORTEX_LOG=$T/sum.log sum_run || sum_env_bad=$((sum_env_bad+1))
sum_env_file=$T/inherited-token-path; printf 'unrelated environment file\n' > "$sum_env_file"; chmod 600 "$sum_env_file"
sum_old_before=$(cksum "$T/h/lead.token"); sum_env_before=$(cksum "$sum_env_file")
token=$sum_env_file FAKE_BOOT_TOKEN=$sum_reissue_value FAKE_MINT_ID_FAIL=1 CORTEX_LOG=$T/sum.log sum_pty --reissue-token && sum_rc=0 || sum_rc=$?
{ [ "$sum_rc" = 7 ] && [ "$sum_old_before" = "$(cksum "$T/h/lead.token")" ] && [ "$sum_env_before" = "$(cksum "$sum_env_file")" ] &&
  [ ! -e "$T/h/lead.token.20991115T030405Z" ] && sum_no_stages "$T/h" && [ ! -e "$T/state/bootstrap.token" ] &&
  sum_no_file_notice "$T/sum.stdout" 1 && [ ! -s "$T/sum.tty" ] &&
  sum_path_absent "$sum_env_file" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log" &&
  sum_path_absent "$T/h/lead.token" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log" &&
  sum_both_private "$T/h/lead.token" "$T/state/active.token" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log"; } || sum_env_bad=$((sum_env_bad+1))
if [ "$sum_env_bad" = 0 ]; then ok 'IT-206 fresh unpublished decoy and inherited token never become recovery claims'; else bad 'IT-206 active mint requires positive publication' "$sum_env_bad controls"; fi

# D8-MIKE-07: TERM while a failed mint is cleaning its container copy must
# independently finish staged-file cleanup, preserving the signal status.
sum_fresh; : > "$T/sum.log"; sum_cleanup_bad=0
FAKE_BOOT_TOKEN=$sum_fake_token CORTEX_LOG=$T/sum.log sum_run || sum_cleanup_bad=1
sum_old_before=$(cksum "$T/h/lead.token")
FAKE_BOOT_TOKEN=$sum_reissue_value FAKE_MINT_ID_FAIL=1 FAKE_BOUNDARY_PHASE=prepublication-cleanup CORTEX_LOG=$T/sum.log sum_pty --reissue-token && sum_rc=0 || sum_rc=$?
{ [ "$sum_rc" = 143 ] && [ -e "$T/state/boundary-delivered" ] && [ ! -s "$T/sum.tty" ] &&
  [ ! -e "$T/h/lead.token.20991115T030405Z" ] && sum_no_stages "$T/h" && [ ! -e "$T/state/bootstrap.token" ] &&
  [ "$sum_old_before" = "$(cksum "$T/h/lead.token")" ] && sum_no_file_notice "$T/sum.stdout" 1 &&
  sum_path_absent "$T/h/lead.token" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log" &&
  sum_both_private "$T/h/lead.token" "$T/state/active.token" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log"; } || sum_cleanup_bad=1
if [ "$sum_cleanup_bad" = 0 ]; then ok 'IT-207 interrupted container cleanup still removes the unpublished private stage independently'; else bad 'IT-207 independent cleanup progress' "rc=$sum_rc"; fi

# D8-MIKE-10: both values on every per-run stream, including a leak that an
# overwrite would erase. These are synthetic guard controls, never real tokens.
mkdir -p "$T/both-guard"; sum_both_bad=0
printf 'OLD_ONLY_0123456789abcdef0123456789abcdef\n' > "$T/both-guard/old"
printf 'NEW_ONLY_fedcba9876543210fedcba9876543210\n' > "$T/both-guard/new"
for sum_guard_value in old new; do
  for sum_guard_channel in stdout stderr log calls; do
    for sum_guard_shape in full fragment; do
      for sum_guard_file in stdout stderr log calls; do printf 'public output\n' > "$T/both-guard/$sum_guard_file"; done
      if [ "$sum_guard_shape" = full ]; then cat "$T/both-guard/$sum_guard_value" > "$T/both-guard/$sum_guard_channel"
      else head -c 16 "$T/both-guard/$sum_guard_value" > "$T/both-guard/$sum_guard_channel"; fi
      if sum_both_private "$T/both-guard/old" "$T/both-guard/new" "$T/both-guard/stdout" "$T/both-guard/stderr" "$T/both-guard/log" "$T/both-guard/calls"; then sum_both_bad=$((sum_both_bad+1)); fi
      printf 'later public output\n' > "$T/both-guard/$sum_guard_channel"
      sum_both_private "$T/both-guard/old" "$T/both-guard/new" "$T/both-guard/stdout" "$T/both-guard/stderr" "$T/both-guard/log" "$T/both-guard/calls" || sum_both_bad=$((sum_both_bad+1))
    done
  done
done
if [ "$sum_both_bad" = 0 ]; then ok 'IT-209 old and new whole values and 16-byte fragments are checked on every stream before an overwrite'; else bad 'IT-209 both-value per-run guard controls' "$sum_both_bad controls"; fi

# D8-MIKE-08/11: ordinary pre-existing and TOCTOU collision refusals preserve
# the unrelated inode and claim no token path on any public stream.
sum_collision_bad=0
for sum_collision in preexisting racing; do
  sum_fresh; : > "$T/sum.log"
  if ! { FAKE_BOOT_TOKEN=$sum_fake_token CORTEX_LOG=$T/sum.log sum_run &&
    sum_old_before=$(cksum "$T/h/lead.token"); }; then
    sum_collision_bad=$((sum_collision_bad+1)); continue
  fi
  sum_candidate=$T/h/lead.token.20991115T030405Z
  if [ "$sum_collision" = preexisting ]; then
    if ! { printf 'unpublished collision decoy\n' > "$sum_candidate" && chmod 600 "$sum_candidate" &&
      "$SUM_REAL_STAT" -c '%d:%i' "$sum_candidate" > "$T/state/decoy-inode"; }; then
      sum_collision_bad=$((sum_collision_bad+1)); continue
    fi
    FAKE_BOOT_TOKEN=$sum_reissue_value FAKE_FIXED_STAMP=1 CORTEX_LOG=$T/sum.log sum_pty --reissue-token && sum_rc=0 || sum_rc=$?
  else
    FAKE_BOOT_TOKEN=$sum_reissue_value FAKE_FIXED_STAMP=1 FAKE_BOUNDARY_PHASE=reissue-collision CORTEX_LOG=$T/sum.log sum_pty --reissue-token && sum_rc=0 || sum_rc=$?
  fi
  if [ "$sum_collision" = preexisting ]; then sum_collision_minted=0; else sum_collision_minted=1; fi
  { [ "$sum_rc" = 7 ] && grep -qxF 'unpublished collision decoy' "$sum_candidate" &&
    [ "$(cat "$T/state/decoy-inode")" = "$("$SUM_REAL_STAT" -c '%d:%i' "$sum_candidate")" ] &&
    [ "$sum_old_before" = "$(cksum "$T/h/lead.token")" ] && sum_no_stages "$T/h" && [ ! -e "$T/state/bootstrap.token" ] && [ ! -s "$T/sum.tty" ] &&
    sum_no_file_notice "$T/sum.stdout" "$sum_collision_minted" &&
    sum_path_absent "$T/h/lead.token" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log" &&
    sum_both_private "$T/h/lead.token" "$T/state/active.token" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log"; } || sum_collision_bad=$((sum_collision_bad+1))
done
if [ "$sum_collision_bad" = 0 ]; then ok 'IT-210 existing and non-signal raced destinations are never replaced or named on refusal'; else bad 'IT-210 no-replace publication and path-free collision' "$sum_collision_bad controls"; fi

# A second same-name reissue is launched by the private PTY owner while the
# first is held at publication. No signal is sent; the contender must refuse
# before bootstrap so it cannot revoke the first run and lose the active token.
sum_fresh; : > "$T/sum.log"; sum_concurrent_bad=0
FAKE_BOOT_TOKEN=$sum_fake_token CORTEX_LOG=$T/sum.log sum_run || sum_concurrent_bad=1
sum_old_before=$(cksum "$T/h/lead.token")
sum_third_value='ctx2_abcdef0123456789abcdef0123456789.ThirdSyntheticValueForConcurrentBootstrapOnly'
FAKE_BOOT_TOKEN=$sum_reissue_value FAKE_CONTENDER_TOKEN=$sum_third_value FAKE_FIXED_STAMP=1 FAKE_BOUNDARY_PHASE=reissue-reservation FAKE_BOUNDARY_SIGNAL=NONE CORTEX_LOG=$T/sum.log sum_pty --reissue-token && sum_rc=0 || sum_rc=$?
sum_candidate=$T/h/lead.token.20991115T030405Z
{ [ "$sum_rc" = 0 ] && [ "$(cat "$T/state/contender.rc")" = 7 ] && [ "$(cat "$T/state/count")" = 2 ] &&
  grep -qF 'another install or token reissue' "$T/state/contender.stderr" &&
  [ "$(grep -c '^problem:' "$T/state/contender.stderr")" = 1 ] && [ "$(grep -c '^fix:' "$T/state/contender.stderr")" = 1 ] &&
  [ ! -e "$T/account-runtime/cortex-mint.reservation" ] &&
  cmp -s "$sum_candidate" "$T/state/active.token" && [ "$sum_old_before" = "$(cksum "$T/h/lead.token")" ] &&
  sum_path_once "$T/sum.stdout" '   Token file (0600):' "$sum_candidate" &&
  sum_path_total_once "$T/sum.stdout" "$sum_candidate" 'Install log:' "$T/sum.log" &&
  sum_path_absent "$T/h/lead.token" "$T/state/contender.stdout" "$T/state/contender.stderr" &&
  sum_both_private "$T/h/lead.token" "$T/state/active.token" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log" "$T/state/contender.stdout" "$T/state/contender.stderr" &&
  sum_no_stages "$T/h" && [ ! -e "$T/state/bootstrap.token" ]; } || sum_concurrent_bad=1
if [ "$sum_concurrent_bad" = 0 ]; then ok 'IT-211 concurrent same-name reissue refuses before bootstrap and preserves the saved active credential'; else bad 'IT-211 exclusive mint before bootstrap' "first rc=$sum_rc"; fi

# W1-300a: a DIFFERENT home and lead shares the fixed account reservation.
# A second reissue is launched by the private PTY owner while the
# first is held at publication. No signal is sent; the contender must refuse
# before bootstrap so it cannot revoke the first run and lose the active token.
sum_fresh; : > "$T/sum.log"; sum_concurrent_bad=0
FAKE_BOOT_TOKEN=$sum_fake_token CORTEX_LOG=$T/sum.log sum_run || sum_concurrent_bad=1
sum_old_before=$(cksum "$T/h/lead.token")
printf 'p:other\n' >> "$T/state/projects"
sum_third_value='ctx2_abcdef0123456789abcdef0123456789.ThirdSyntheticValueForConcurrentBootstrapOnly'
FAKE_BOOT_TOKEN=$sum_reissue_value FAKE_CONTENDER_HOME="$T/second-home" FAKE_CONTENDER_LEAD=other FAKE_CONTENDER_TOKEN=$sum_third_value FAKE_FIXED_STAMP=1 FAKE_BOUNDARY_PHASE=reissue-reservation FAKE_BOUNDARY_SIGNAL=NONE CORTEX_LOG=$T/sum.log sum_pty --reissue-token && sum_rc=0 || sum_rc=$?
sum_candidate=$T/h/lead.token.20991115T030405Z
{ [ "$sum_rc" = 0 ] && [ "$(cat "$T/state/contender.rc")" = 7 ] && [ "$(cat "$T/state/count")" = 2 ] &&
  grep -qF 'another install or token reissue' "$T/state/contender.stderr" &&
  [ "$(grep -c '^problem:' "$T/state/contender.stderr")" = 1 ] && [ "$(grep -c '^fix:' "$T/state/contender.stderr")" = 1 ] &&
  [ ! -e "$T/account-runtime/cortex-mint.reservation" ] &&
  cmp -s "$sum_candidate" "$T/state/active.token" && [ "$sum_old_before" = "$(cksum "$T/h/lead.token")" ] &&
  sum_path_once "$T/sum.stdout" '   Token file (0600):' "$sum_candidate" &&
  sum_path_total_once "$T/sum.stdout" "$sum_candidate" 'Install log:' "$T/sum.log" &&
  sum_path_absent "$T/h/lead.token" "$T/state/contender.stdout" "$T/state/contender.stderr" &&
  sum_both_private "$T/h/lead.token" "$T/state/active.token" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log" "$T/state/contender.stdout" "$T/state/contender.stderr" &&
  sum_no_stages "$T/h" && [ ! -e "$T/state/bootstrap.token" ]; } || sum_concurrent_bad=1
if [ "$sum_concurrent_bad" = 0 ]; then ok 'IT-217 different homes and leads share one account reservation and never interleave the container token'; else bad 'IT-217 account-wide mint ownership' "first rc=$sum_rc"; fi

# W1-299a: a stale owned reservation is kept with its public owner record;
# no bootstrap happens and no token path is named on any stream.
sum_fresh; : > "$T/sum.log"; sum_stale_bad=0
sum_rc=setup
if FAKE_BOOT_TOKEN=$sum_fake_token CORTEX_LOG=$T/sum.log sum_run &&
  sum_old_before=$(cksum "$T/h/lead.token") &&
  mkdir "$T/account-runtime/cortex-mint.reservation" && chmod 700 "$T/account-runtime/cortex-mint.reservation" &&
  printf 'pid=99999999\nstarted_at=20000101T000000Z\nhost=%s\nhome=%s\n' "$(uname -n)" "$T/h" > "$T/account-runtime/cortex-mint.reservation/owner" &&
  chmod 600 "$T/account-runtime/cortex-mint.reservation/owner" &&
  sum_record_before=$(cksum "$T/account-runtime/cortex-mint.reservation/owner") &&
  sum_dir_id=$("$SUM_REAL_STAT" -c '%d:%i' "$T/account-runtime/cortex-mint.reservation"); then
FAKE_BOOT_TOKEN=$sum_reissue_value FAKE_FIXED_STAMP=1 CORTEX_LOG=$T/sum.log sum_pty --reissue-token && sum_rc=0 || sum_rc=$?
{ [ "$sum_rc" = 7 ] && [ "$(cat "$T/state/count")" = 1 ] && [ "$sum_old_before" = "$(cksum "$T/h/lead.token")" ] &&
  [ "$sum_record_before" = "$(cksum "$T/account-runtime/cortex-mint.reservation/owner")" ] && [ "$sum_dir_id" = "$("$SUM_REAL_STAT" -c '%d:%i' "$T/account-runtime/cortex-mint.reservation")" ] &&
  [ "$(grep -c '^problem:' "$T/sum.stderr")" = 1 ] && [ "$(grep -c '^fix:' "$T/sum.stderr")" = 1 ] &&
  grep -qF 'pid=99999999' "$T/sum.stderr" && grep -qF 'started_at=20000101T000000Z' "$T/sum.stderr" && grep -qF "host=$(uname -n)" "$T/sum.stderr" &&
  grep -q 'fix: .*no installer is running' "$T/sum.stderr" && [ ! -s "$T/sum.tty" ] &&
  sum_path_absent "$T/h/lead.token" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log" &&
  sum_private_files "$T/h/lead.token" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log"; } || sum_stale_bad=1
else sum_stale_bad=1; fi
if [ "$sum_stale_bad" = 0 ]; then ok 'IT-212 stale mint reservation is never stolen or deleted, its owner record is shown, and no bootstrap occurs'; else bad 'IT-212 stale reservation fail closed' "rc=$sum_rc"; fi

sum_reservation_bad=0
for sum_signal in HUP INT TERM; do
  sum_fresh; : > "$T/sum.log"
  FAKE_BOOT_TOKEN=$sum_fake_token CORTEX_LOG=$T/sum.log sum_run || sum_reservation_bad=$((sum_reservation_bad+1))
  FAKE_BOOT_TOKEN=$sum_reissue_value FAKE_FIXED_STAMP=1 FAKE_BOUNDARY_PHASE=reservation-signal FAKE_BOUNDARY_SIGNAL=$sum_signal CORTEX_LOG=$T/sum.log sum_pty --reissue-token && sum_rc=0 || sum_rc=$?
  case $sum_signal in HUP) sum_want=129 ;; INT) sum_want=130 ;; TERM) sum_want=143 ;; esac
  { [ "$sum_rc" = "$sum_want" ] && [ -e "$T/state/reservation-observed" ] && [ ! -e "$T/account-runtime/cortex-mint.reservation" ] &&
    sum_no_stages "$T/h" && [ ! -e "$T/state/bootstrap.token" ] && [ ! -s "$T/sum.tty" ] &&
    sum_path_absent "$T/h/lead.token" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log" &&
    sum_both_private "$T/h/lead.token" "$T/state/active.token" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log"; } || sum_reservation_bad=$((sum_reservation_bad+1))
done
if [ "$sum_reservation_bad" = 0 ]; then ok 'IT-213 owned private mint reservation is released on HUP INT TERM while unpublished copies are cleaned'; else bad 'IT-213 trapped reservation release' "$sum_reservation_bad signals"; fi

# W1-300: invalid public log paths fail before a file write or mint.
log_path_bad=0
for log_case in relative space newline control; do
  sum_fresh
  case $log_case in
    relative) log_candidate=relative.log ;;
    space) log_candidate="$T/log with space" ;;
    newline) log_candidate=$(printf '%s/log\nnewline' "$T") ;;
    control) log_candidate=$(printf '%s/log\033control' "$T") ;;
  esac
  ( cd "$T"; CORTEX_LOG=$log_candidate sh "$install" --project p --lead lead --home "$T/h" --no-show-credentials ) > "$T/log-path.stdout" 2> "$T/log-path.stderr" && log_rc=0 || log_rc=$?
  case $log_candidate in /*) log_created=$log_candidate ;; *) log_created=$T/$log_candidate ;; esac
  { [ "$log_rc" = 1 ] && [ ! -e "$log_created" ] && [ ! -e "$T/h/lead.token" ] && [ ! -e "$T/state/count" ] &&
    [ "$(grep -c '^problem:' "$T/log-path.stderr")" = 1 ] && [ "$(grep -c '^fix:' "$T/log-path.stderr")" = 1 ]; } || log_path_bad=$((log_path_bad+1))
done
sum_fresh; : > "$T/valid-log-path"
CORTEX_LOG=$T/valid-log-path sum_run || log_path_bad=$((log_path_bad+1))
sum_path_once "$T/out" 'Install log:' "$T/valid-log-path" || log_path_bad=$((log_path_bad+1))
if [ "$log_path_bad" = 0 ]; then ok 'IT-218 public log path is absolute ASCII without spaces controls or newlines, and invalid names are never written'; else bad 'IT-218 log-path validation before writes' "$log_path_bad controls"; fi

runtime_bad=0
for runtime_case in missing open symlink owner; do
  sum_fresh
  if ! case $runtime_case in
    missing) rmdir "$T/account-runtime" ;;
    open) chmod 755 "$T/account-runtime" ;;
    symlink) rmdir "$T/account-runtime" && mkdir -p "$T/runtime-canary" && printf 'unchanged\n' > "$T/runtime-canary/kept" && ln -s "$T/runtime-canary" "$T/account-runtime" ;;
  esac; then runtime_bad=$((runtime_bad+1)); continue; fi
  if [ "$runtime_case" = owner ]; then FAKE_RUNTIME_OWNER_FAIL=1 sum_run && runtime_rc=0 || runtime_rc=$?
  else sum_run && runtime_rc=0 || runtime_rc=$?; fi
  { [ "$runtime_rc" = 7 ] && [ ! -e "$T/state/count" ] && [ ! -e "$T/h/lead.token" ] &&
    [ "$(grep -c '^problem:' "$T/sum.err")" = 1 ] && [ "$(grep -c '^fix:' "$T/sum.err")" = 1 ] &&
    sum_no_file_notice "$T/out" 0; } || runtime_bad=$((runtime_bad+1))
  if [ "$runtime_case" = symlink ]; then grep -qxF unchanged "$T/runtime-canary/kept" || runtime_bad=$((runtime_bad+1)); fi
done
if [ "$runtime_bad" = 0 ]; then ok 'IT-225 missing symlink open or foreign account runtime refuses with one cause and fix before any bootstrap'; else bad 'IT-225 account-runtime ownership' "$runtime_bad controls"; fi

# W1-300c: an otherwise healthy contender must still refuse while the
# first run is paused after measuring, before its one terminal write/path pair.
sum_fresh; : > "$T/sum.log"; section_bad=0
printf '%s\n' "$sum_reissue_value" > "$T/contender-value"
FAKE_BOOT_TOKEN=$sum_fake_token FAKE_CONTENDER_TOKEN=$sum_reissue_value FAKE_BOUNDARY_PHASE=after-header FAKE_BOUNDARY_SIGNAL=NONE CORTEX_LOG=$T/sum.log sum_pty && section_rc=0 || section_rc=$?
{ [ "$section_rc" = 0 ] && [ "$(cat "$T/state/contender.rc")" = 7 ] && [ "$(cat "$T/state/count")" = 1 ] &&
  cmp -s "$T/h/lead.token" "$T/state/active.token" &&
  [ "$(tr -d '\r' < "$T/sum.tty" | grep -cxF "$sum_fake_token")" = 1 ] &&
  sum_path_once "$T/sum.stdout" '   Token file (0600):' "$T/h/lead.token" &&
  sum_path_total_once "$T/sum.stdout" "$T/h/lead.token" 'Install log:' "$T/sum.log" &&
  sum_both_private "$T/h/lead.token" "$T/contender-value" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log" "$T/state/contender.stdout" "$T/state/contender.stderr" &&
  [ ! -e "$T/account-runtime/cortex-mint.reservation" ]; } || section_bad=1
if [ "$section_bad" = 0 ]; then ok 'IT-227 account reservation remains held through measurement marker terminal write and the sole path pair'; else bad 'IT-227 reservation lifetime through credential section' "rc=$section_rc"; fi

# W1-300c: a signal after mkdir has created the directory but before the
# creating shell assigns ownership must not strand an unrecorded reservation.
mkdir_bad=0
for mkdir_signal in HUP INT TERM; do
  sum_fresh; : > "$T/sum.log"
  FAKE_BOOT_TOKEN=$sum_fake_token CORTEX_LOG=$T/sum.log sum_run || mkdir_bad=$((mkdir_bad+1))
  FAKE_BOOT_TOKEN=$sum_reissue_value FAKE_FIXED_STAMP=1 FAKE_BOUNDARY_PHASE=reservation-mkdir FAKE_BOUNDARY_SIGNAL=$mkdir_signal CORTEX_LOG=$T/sum.log sum_pty --reissue-token && mkdir_rc=0 || mkdir_rc=$?
  case $mkdir_signal in HUP) mkdir_want=129 ;; INT) mkdir_want=130 ;; TERM) mkdir_want=143 ;; esac
  { [ "$mkdir_rc" = "$mkdir_want" ] && [ -e "$T/state/reservation-created" ] && [ "$(cat "$T/state/count")" = 1 ] &&
    [ ! -e "$T/account-runtime/cortex-mint.reservation" ] && [ ! -e "$T/state/bootstrap.token" ] && [ ! -s "$T/sum.tty" ] &&
    sum_no_stages "$T/h" && sum_no_file_notice "$T/sum.stdout" 0 &&
    sum_path_absent "$T/h/lead.token" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log" &&
    sum_private_files "$T/h/lead.token" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log"; } || mkdir_bad=$((mkdir_bad+1))
done
if [ "$mkdir_bad" = 0 ]; then ok 'IT-228 mkdir-to-owner-record signal boundary releases only the created reservation before any bootstrap'; else bad 'IT-228 reservation acquisition signal boundary' "$mkdir_bad signals"; fi

# The final reservation release also defers trapped signals until the owner
# record AND the same owned empty directory have been removed.
release_bad=0
for release_signal in HUP INT TERM; do
  sum_fresh; : > "$T/sum.log"
  FAKE_BOOT_TOKEN=$sum_fake_token FAKE_BOUNDARY_PHASE=reservation-release FAKE_BOUNDARY_SIGNAL=SKIP CORTEX_LOG=$T/sum.log sum_pty && release_base_rc=0 || release_base_rc=$?
  { [ "$release_base_rc" = 0 ] && sum_private_files "$T/h/lead.token" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log"; } || release_bad=$((release_bad+1))
  cp "$T/sum.stdout" "$T/release-base.stdout"; cp "$T/sum.stderr" "$T/release-base.stderr"
  sum_fresh; : > "$T/sum.log"
  FAKE_BOOT_TOKEN=$sum_fake_token FAKE_BOUNDARY_PHASE=reservation-release FAKE_BOUNDARY_SIGNAL=$release_signal CORTEX_LOG=$T/sum.log sum_pty && release_rc=0 || release_rc=$?
  case $release_signal in HUP) release_want=129 ;; INT) release_want=130 ;; TERM) release_want=143 ;; esac
  { cmp -s "$T/release-base.stdout" "$T/sum.stdout" && cmp -s "$T/release-base.stderr" "$T/sum.stderr" &&
    [ "$release_rc" = "$release_want" ] && [ "$(grep -c '^problem:' "$T/sum.stderr" || true)" = 0 ] &&
    [ "$(grep -c '^fix:' "$T/sum.stderr" || true)" = 0 ] && [ -e "$T/state/reservation-owner-removed" ] &&
    [ ! -e "$T/account-runtime/cortex-mint.reservation" ] && [ ! -e "$T/state/bootstrap.token" ] &&
    sum_no_stages "$T/h" && cmp -s "$T/h/lead.token" "$T/state/active.token" &&
    [ "$(tr -d '\r' < "$T/sum.tty" | grep -cxF "$sum_fake_token")" = 1 ] &&
    sum_path_once "$T/sum.stdout" '   Token file (0600):' "$T/h/lead.token" &&
    sum_path_total_once "$T/sum.stdout" "$T/h/lead.token" 'Install log:' "$T/sum.log" &&
    sum_private_files "$T/h/lead.token" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log"; } || release_bad=$((release_bad+1))
done
if [ "$release_bad" = 0 ]; then ok 'IT-229 final owner-unlink to directory-release boundary honors signals without stranding the owned reservation'; else bad 'IT-229 reservation release signal boundary' "$release_bad signals"; fi

# W1-303: the first TERM enters EXIT; a second signal lands inside each
# nested cleanup/output/release boundary. The first status143 must stand.
for handler_case in handler-release handler-copy handler-path; do
  handler_bad=0
  for handler_signal in HUP INT TERM; do
    sum_fresh; : > "$T/sum.log"
    FAKE_BOOT_TOKEN=$sum_fake_token CORTEX_LOG=$T/sum.log sum_run || handler_bad=$((handler_bad+1))
    FAKE_BOOT_TOKEN=$sum_reissue_value FAKE_FIXED_STAMP=1 FAKE_BOUNDARY_PHASE=$handler_case FAKE_BOUNDARY_SIGNAL=SKIP CORTEX_LOG=$T/sum.log sum_pty --reissue-token && handler_base_rc=0 || handler_base_rc=$?
    { [ "$handler_base_rc" = 143 ] && sum_both_private "$T/h/lead.token" "$T/state/active.token" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log"; } || handler_bad=$((handler_bad+1))
    cp "$T/sum.stdout" "$T/handler-base.stdout"; cp "$T/sum.stderr" "$T/handler-base.stderr"
    sum_fresh; : > "$T/sum.log"
    FAKE_BOOT_TOKEN=$sum_fake_token CORTEX_LOG=$T/sum.log sum_run || handler_bad=$((handler_bad+1))
    handler_old=$(cksum "$T/h/lead.token")
    FAKE_BOOT_TOKEN=$sum_reissue_value FAKE_FIXED_STAMP=1 FAKE_BOUNDARY_PHASE=$handler_case FAKE_BOUNDARY_SIGNAL=$handler_signal CORTEX_LOG=$T/sum.log sum_pty --reissue-token && handler_rc=0 || handler_rc=$?
    handler_dest=$(cat "$T/state/candidate-reissue-path")
    { cmp -s "$T/handler-base.stdout" "$T/sum.stdout" && cmp -s "$T/handler-base.stderr" "$T/sum.stderr" &&
      [ "$handler_rc" = 143 ] && [ "$(grep -c '^problem:' "$T/sum.stderr" || true)" = 0 ] &&
      [ "$(grep -c '^fix:' "$T/sum.stderr" || true)" = 0 ] && [ -e "$T/state/first-signal-sent" ] && [ -e "$T/state/boundary-delivered" ] &&
      [ ! -e "$T/account-runtime/cortex-mint.reservation" ] && [ ! -e "$T/state/bootstrap.token" ] &&
      sum_no_stages "$T/h" && [ ! -s "$T/sum.tty" ] && [ "$handler_old" = "$(cksum "$T/h/lead.token")" ] &&
      sum_both_private "$T/h/lead.token" "$T/state/active.token" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log"; } || handler_bad=$((handler_bad+1))
    if [ "$handler_case" = handler-copy ]; then
      { [ ! -e "$handler_dest" ] && sum_no_file_notice "$T/sum.stdout" 1 &&
        sum_path_absent "$handler_dest" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log" &&
        sum_path_absent "$T/h/lead.token" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log"; } || handler_bad=$((handler_bad+1))
    else
      { cmp -s "$handler_dest" "$T/state/active.token" &&
        sum_path_once "$T/sum.stdout" '   Token file (0600):' "$handler_dest" &&
        sum_path_total_once "$T/sum.stdout" "$handler_dest" 'Install log:' "$T/sum.log"; } || handler_bad=$((handler_bad+1))
    fi
  done
  case $handler_case in
    handler-release) if [ "$handler_bad" = 0 ]; then ok 'IT-230 second signals during failed-run reservation release preserve original status and finish cleanup'; else bad 'IT-230 in-handler reservation release' "$handler_bad controls"; fi ;;
    handler-copy) if [ "$handler_bad" = 0 ]; then ok 'IT-231 second signals during in-handler container cleanup finish stage no-file words and reservation release'; else bad 'IT-231 nested cleanup deferral' "$handler_bad controls"; fi ;;
    handler-path) if [ "$handler_bad" = 0 ]; then ok 'IT-232 second signals during in-handler path output finish exactly one pair and release the reservation'; else bad 'IT-232 nested path deferral' "$handler_bad controls"; fi ;;
  esac
done

# W1-303c: a diagnosed postpublication stage failure already owns one
# cause/fix. A signal during its EXIT release adds no lines and cannot replace7.
diagnosed_bad=0
for diagnosed_signal in HUP INT TERM; do
  sum_fresh; : > "$T/sum.log"
  FAKE_BOOT_TOKEN=$sum_fake_token FAKE_RELEASE_STAGE_FAIL=1 FAKE_BOUNDARY_PHASE=reservation-release FAKE_BOUNDARY_SIGNAL=SKIP CORTEX_LOG=$T/sum.log sum_pty && diagnosed_base_rc=0 || diagnosed_base_rc=$?
  { [ "$diagnosed_base_rc" = 7 ] && sum_private_files "$T/h/lead.token" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log"; } || diagnosed_bad=$((diagnosed_bad+1))
  cp "$T/sum.stdout" "$T/diagnosed-base.stdout"
  sed -E 's/[.]part[.][0123456789]+/.part.PID/g' "$T/sum.stderr" > "$T/diagnosed-base.stderr"
  sum_fresh; : > "$T/sum.log"
  FAKE_BOOT_TOKEN=$sum_fake_token FAKE_RELEASE_STAGE_FAIL=1 FAKE_BOUNDARY_PHASE=reservation-release FAKE_BOUNDARY_SIGNAL=$diagnosed_signal CORTEX_LOG=$T/sum.log sum_pty && diagnosed_rc=0 || diagnosed_rc=$?
  diagnosed_stage=$(printf '%s\n' "$T/h/lead.token.part."*)
  diagnosed_pid=${diagnosed_stage##*.}
  sed -E 's/[.]part[.][0123456789]+/.part.PID/g' "$T/sum.stderr" > "$T/diagnosed-current.stderr"
  { cmp -s "$T/diagnosed-base.stdout" "$T/sum.stdout" && cmp -s "$T/diagnosed-base.stderr" "$T/diagnosed-current.stderr" &&
    [ "$diagnosed_rc" = 7 ] && [ -f "$diagnosed_stage" ] && [ ! -e "$T/account-runtime/cortex-mint.reservation" ] &&
    [ ! -e "$T/state/bootstrap.token" ] && [ ! -s "$T/sum.tty" ] &&
    [ "$(stat -c '%d:%i' "$T/h/lead.token")" = "$(stat -c '%d:%i' "$diagnosed_stage")" ] &&
    [ "$(grep -c '^problem:' "$T/sum.stderr" || true)" = 1 ] && [ "$(grep -c '^fix:' "$T/sum.stderr" || true)" = 1 ] &&
    grep -qF ".part.$diagnosed_pid added to its name. Remove ONLY that file, by hand" "$T/sum.stderr" &&
    sum_path_once "$T/sum.stdout" '   Token file (0600):' "$T/h/lead.token" &&
    sum_path_total_once "$T/sum.stdout" "$T/h/lead.token" 'Install log:' "$T/sum.log" &&
    sum_path_absent "$T/h/lead.token" "$T/sum.stderr" "$T/sum.log" "$T/log" &&
    sum_private_files "$T/h/lead.token" "$T/sum.stdout" "$T/sum.stderr" "$T/sum.log" "$T/log"; } || diagnosed_bad=$((diagnosed_bad+1))
done
if [ "$diagnosed_bad" = 0 ]; then ok 'IT-233 a second signal preserves diagnosed status7 one cause and fix and the PID-qualified stage recovery'; else bad 'IT-233 diagnosed handler preserves its own status and words' "$diagnosed_bad controls"; fi

trace_fake_token=$(python3 -I -c 'import base64,hashlib;print("ctx2_"+hashlib.sha256(b"W1-303e-id").hexdigest()[:32]+"."+base64.urlsafe_b64encode(hashlib.sha256(b"W1-303e-value").digest()).decode().rstrip("="))')
TRACE_HELPER=$here/fakes/install/exit-signal-test.py
for trace_group in tail head; do
trace_bad=0
case $trace_group in tail) trace_modes='tail tail-error tail-rearm' ;; head) trace_modes='head-normal head-signal head-error head-raw' ;; esac
for trace_mode in $trace_modes; do
 for trace_signal in HUP INT TERM; do
  for trace_run in base signal; do
   sum_fresh; : > "$T/trace-app.log"
   trace_dir=$T/trace-$trace_mode-$trace_signal-$trace_run
   if [ "$trace_run" = base ]; then trace_send=SKIP; else trace_send=$trace_signal; fi
   CORTEX_LOG=$T/trace-app.log FAKE_BOOT_TOKEN=$trace_fake_token python3 -I -B "$TRACE_HELPER" "$install" "$T/h" "$trace_dir" "$trace_mode" "$trace_send" > "$T/trace-report" && trace_native=0 || trace_native=$?
   python3 -I - "$trace_dir" "$trace_mode" "$trace_send" "$T" <<'TRACE_ASSERT' || trace_bad=$((trace_bad+1))
import json,pathlib,re,sys
p=pathlib.Path(sys.argv[1]);mode,sig=sys.argv[2:4];t=pathlib.Path(sys.argv[4]);d=json.loads((p/'report.json').read_text())
active=t/'state/active.token'
if active.exists():
 value=active.read_bytes().strip()
 for channel in [p/'stdout',p/'stderr',p/'trace',t/'trace-app.log',t/'log']:
  data=channel.read_bytes()
  for view in (data,data.replace(b'\r',b'').replace(b'\n',b'')):
   if value in view or any(value[i:i+16] in view for i in range(len(value)-15)):raise AssertionError('credential guard')
base={'tail':0,'tail-error':8,'tail-rearm':0,'head-normal':0,'head-signal':143,'head-error':8,'head-raw':1}[mode]
want=base if base or sig=='SKIP' else {'HUP':129,'INT':130,'TERM':143}[sig]
checks={'status':d['status']==want,'boundary':d['boundary_observed'],'value':d['raw_value_guard'],'reservation':not (t/'account-runtime/cortex-mint.reservation').exists(),'stage':not list((t/'h').glob('*.token*.part*')),'server_copy':not(t/'state/bootstrap.token').exists(),'cause':d['problem_count']==(1 if mode.endswith('error') else 0),'fix':d['fix_count']==(1 if mode.endswith('error') else 0),'tty':d['tty_value_count']==(1 if mode in ('tail','tail-rearm','head-normal') else 0)}
stdout=(p/'stdout').read_bytes()
path=str(t/'h/lead.token').encode()
if mode=='head-signal':
 checks['unpublished']=not(t/'h/lead.token').exists()
 checks['no_file_words']=b'No new token file was published; the server may already have replaced the old token: run --reissue-token again.' in b' '.join(stdout.split())
 checks['no_path']=path not in stdout
else:
 checks['published']=(t/'h/lead.token').is_file()
 checks['one_pair']=stdout.splitlines().count(path)==1 and b'Token file (0600):\n'+path in stdout
if sig=='SKIP' and mode in ('tail','tail-error'):checks['final_exit_untraced']=len(d['markers_seen'])==1
if sig!='SKIP':
 b=p.with_name(p.name.rsplit('-',1)[0]+'-base');checks['same_stdout']=(p/'stdout').read_bytes()==(b/'stdout').read_bytes();checks['same_stderr']=(p/'public-stderr').read_bytes()==(b/'public-stderr').read_bytes()
print(mode,sig,'failed='+','.join(k for k,v in checks.items() if not v))
if not checks['status']:print('TRACE-DIAG',json.dumps({k:d[k] for k in ['mode','signal','status','boundary_observed','markers_seen','target_line']}))
if not all(checks.values()):sys.exit(1)
TRACE_ASSERT
   [ "$trace_native" = 0 ] || trace_bad=$((trace_bad+1))
  done
 done
done
case $trace_group in
 tail) if [ "$trace_bad" = 0 ]; then ok 'IT-234 final trace checkpoint and untraced status read honor late HUP INT TERM without added public lines'; else bad 'IT-234 final exit signal window' "$trace_bad controls"; fi ;;
 head) if [ "$trace_bad" = 0 ]; then ok 'IT-235 normal signal diagnosed and bare-error heads preserve cleanup status and already-recorded signals'; else bad 'IT-235 handler entry windows' "$trace_bad controls"; fi ;;
esac
done

# W1-308: real output failure must not preempt credential cleanup. The private
# publication wrapper exposes a stage; the whole installer owns every cleanup.
abort_static_bad=0
python3 -I - "$install" <<'ABORT_CONTRACT' || abort_static_bad=1
from pathlib import Path
import re,sys
s=Path(sys.argv[1]).read_text()
def body(name):
 m=re.search(r'^'+name+r'\(\) \{\n.*?^}',s,re.M|re.S)
 assert m,name
 return m[0]
h=body('exit_handler');a=body('summary_abort')
commands=[x.strip() for x in h.splitlines()[1:] if x.strip() and not x.lstrip().startswith('#')]
assert commands[0]=='set +e','handler must disable errexit before its cleanup'
cleanup_match=re.search(r'^  mint_abort_cleanup(?: \|\| :)?$',a,re.M); assert cleanup_match
cleanup=cleanup_match.start()
prints=[(a.index(line),line) for line in a.splitlines() if not line.lstrip().startswith('#') and re.search(r'\bsummary_(?:line|token_path)\s',line)]
assert prints and all(cleanup<pos and line.rstrip().endswith('|| :') for pos,line in prints),'cleanup precedes nonfatal abort prints'
stderr_prints=[line.split('#',1)[0].rstrip() for line in h.splitlines() if not line.lstrip().startswith('#') and re.search(r'\bprintf\b',line) and '>&2' in line]
assert stderr_prints and all(re.search(r'>&2\s*\|\| :$',line) for line in stderr_prints),'STATIC handler stderr diagnostic keeps its nonfatal guard'
ABORT_CONTRACT
if [ "$abort_static_bad" = 0 ]; then ok 'IT-242 STATIC abort policy and handler diagnostic disable errexit, clean before output and keep nonfatal print guards'; else bad 'IT-242 STATIC W1-308e abort policy contract' 'source policy predicate failed; no behavioral result inferred'; fi
abort_old_value='ctx2_bae7418e3077e8cf82a3a8f8ea2e7cb5.J4rIF9BbR2nvU-dPHtADrHjp3KsCteAy0o1d5OeVr2A'
abort_new_value='ctx2_2422e1ecd29c71ff08cebfc537f05684.Pj1bx6ZyQoVN6uxrGB_XAHjCYSZjnQMzY2mC-N1O6xA'
for abort_kind in pipe pty; do
  abort_bad=0
  for abort_cause in HUP STOP; do
    [ "$abort_kind:$abort_cause" != pty:STOP ] || continue
    sum_fresh
    if ! FAKE_BOOT_TOKEN=$abort_old_value sum_run; then abort_bad=$((abort_bad+1)); continue; fi
    abort_out=$T/abort-output-$abort_kind-$abort_cause
    FAKE_BOOT_TOKEN=$abort_new_value python3 -I -B "$here/fakes/install/abort-output-test.py" "$install" "$T/h" "$abort_out" "$abort_kind" "$abort_cause" || abort_bad=$((abort_bad+1))
  done
  case $abort_kind in
    pipe) if [ "$abort_bad" = 0 ]; then ok 'IT-236 W1-308 a dead stdout pipe cannot skip credential cleanup or replace HUP and diagnosed statuses'; else bad 'IT-236 W1-308 dead stdout pipe cleanup' "$abort_bad controls"; fi ;;
    pty) if [ "$abort_bad" = 0 ]; then ok 'IT-237 W1-308 a hung-up controlling terminal releases all unpublished credential resources'; else bad 'IT-237 W1-308 hung-up terminal cleanup' "$abort_bad controls"; fi ;;
  esac
done

# A real, owned loopback HTTP listener observes only whether a synthetic bearer
# arrived. Unhealthy, unmapped and raced peers must receive no Authorization.
peer_bad=0
for peer_mode in repeat-exited repeat-db repeat-port repeat-unavailable repeat-race-server repeat-race-port positive; do
  sum_fresh
  FAKE_BOOT_TOKEN=$sum_fake_token python3 -I -B "$here/fakes/install/peer-guard-test.py" "$install" "$T/h" "$T/peer-$peer_mode" "$peer_mode" || peer_bad=$((peer_bad+1))
done
if [ "$peer_bad" = 0 ]; then ok 'IT-238 W1-308 every repeat bearer verifies the running healthy server and exact port immediately before use'; else bad 'IT-238 W1-308 repeat bearer peer verification' "$peer_bad controls"; fi
peer_summary_bad=0
for peer_mode in summary-exited summary-db-exited; do
  sum_fresh
  FAKE_BOOT_TOKEN=$sum_fake_token python3 -I -B "$here/fakes/install/peer-guard-test.py" "$install" "$T/h" "$T/peer-$peer_mode" "$peer_mode" || peer_summary_bad=$((peer_summary_bad+1))
done
if [ "$peer_summary_bad" = 0 ]; then ok 'IT-239 W1-308 summary sends no bearer unless both containers are running and healthy'; else bad 'IT-239 W1-308 summary bearer peer verification' "$peer_summary_bad controls"; fi

mode_bad=0
for unsafe_mode in 640 644 666 601; do
  sum_fresh
  if ! { FAKE_BOOT_TOKEN=$sum_fake_token sum_run && chmod "$unsafe_mode" "$T/h/lead.token" && mode_before=$(cksum "$T/h/lead.token"); }; then mode_bad=$((mode_bad+1)); continue; fi
  FAKE_BOOT_TOKEN=$sum_fake_token sum_pty && sum_rc=0 || sum_rc=$?
  { [ "$sum_rc" = 7 ] && [ "$(stat -c %a "$T/h/lead.token")" = "$unsafe_mode" ] && [ "$mode_before" = "$(cksum "$T/h/lead.token")" ] &&
    [ ! -s "$T/sum.tty" ] && [ ! -e "$T/h/lead.token.shown" ] &&
    [ "$(grep -c '^problem:' "$T/sum.stderr")" = 1 ] && [ "$(grep -c '^fix:' "$T/sum.stderr")" = 1 ] &&
    grep -q -- '--reissue-token' "$T/sum.stderr" && ! grep -q 'private token file exists\|Token file (\|Credential:' "$T/sum.stdout" &&
    sum_private_files "$T/h/lead.token" "$T/sum.stdout" "$T/sum.stderr" "$T/log"; } || mode_bad=$((mode_bad+1))
  FAKE_BOOT_TOKEN=$sum_reissue_value FAKE_FIXED_STAMP=1 sum_pty --reissue-token && sum_rc=0 || sum_rc=$?
  mode_new=$T/h/lead.token.20991115T030405Z
  { [ "$sum_rc" = 0 ] && [ "$(stat -c %a "$mode_new")" = 600 ] && [ "$mode_before" = "$(cksum "$T/h/lead.token")" ] &&
    [ "$(tr -d '\r' < "$T/sum.tty" | grep -cxF "$sum_reissue_value")" = 1 ] &&
    sum_both_private "$T/h/lead.token" "$mode_new" "$T/sum.stdout" "$T/sum.stderr" "$T/log"; } || mode_bad=$((mode_bad+1))
done
for private_mode in 400 700; do
  sum_fresh
  if ! { FAKE_BOOT_TOKEN=$sum_fake_token sum_run && chmod "$private_mode" "$T/h/lead.token" && mode_before=$(cksum "$T/h/lead.token"); }; then mode_bad=$((mode_bad+1)); continue; fi
  FAKE_BOOT_TOKEN=$sum_fake_token sum_pty && sum_rc=0 || sum_rc=$?
  { [ "$sum_rc" = 0 ] && [ "$(stat -c %a "$T/h/lead.token")" = "$private_mode" ] && [ "$mode_before" = "$(cksum "$T/h/lead.token")" ] &&
    [ "$(tr -d '\r' < "$T/sum.tty" | grep -cxF "$sum_fake_token")" = 1 ] &&
    sum_private_files "$T/h/lead.token" "$T/sum.stdout" "$T/sum.stderr" "$T/log"; } || mode_bad=$((mode_bad+1))
done
if [ "$mode_bad" = 0 ]; then ok 'IT-240 W1-308c refuses non-private tokens unchanged, reissue recovers, and private owner permissions stay unchanged'; else bad 'IT-240 W1-308c token privacy policy' "$mode_bad controls"; fi

# Rust write_all(secret) supplies no final newline. Both nonempty-read fallbacks
# must remain useful through fresh display, repeat, and reissue.
sum_fresh; newline_bad=0
FAKE_BOOT_TOKEN=$sum_fake_token FAKE_BOOT_NO_NEWLINE=1 sum_pty && sum_rc=0 || sum_rc=$?
{ [ "$sum_rc" = 0 ] && [ "$(wc -c < "$T/h/lead.token")" = "${#sum_fake_token}" ] &&
  [ "$(tr -d '\r' < "$T/sum.tty" | grep -cxF "$sum_fake_token")" = 1 ] && [ -f "$T/h/lead.token.shown" ] &&
  grep -q '200 ok: lead, project_lead of p' "$T/sum.stdout" &&
  sum_private_files "$T/h/lead.token" "$T/sum.stdout" "$T/sum.stderr" "$T/log"; } || newline_bad=$((newline_bad+1))
FAKE_BOOT_TOKEN=$sum_fake_token FAKE_BOOT_NO_NEWLINE=1 sum_pty && sum_rc=0 || sum_rc=$?
{ [ "$sum_rc" = 0 ] && [ ! -s "$T/sum.tty" ] && grep -q '200 ok: lead, project_lead of p' "$T/sum.stdout" &&
  sum_private_files "$T/h/lead.token" "$T/sum.stdout" "$T/sum.stderr" "$T/log"; } || newline_bad=$((newline_bad+1))
FAKE_BOOT_TOKEN=$sum_reissue_value FAKE_BOOT_NO_NEWLINE=1 FAKE_FIXED_STAMP=1 sum_pty --reissue-token && sum_rc=0 || sum_rc=$?
newline_new=$T/h/lead.token.20991115T030405Z
{ [ "$sum_rc" = 0 ] && [ "$(wc -c < "$newline_new")" = "${#sum_reissue_value}" ] && [ -f "$newline_new.shown" ] &&
  [ "$(tr -d '\r' < "$T/sum.tty" | grep -cxF "$sum_reissue_value")" = 1 ] && grep -q '200 ok: lead, project_lead of p' "$T/sum.stdout" &&
  sum_both_private "$T/h/lead.token" "$newline_new" "$T/sum.stdout" "$T/sum.stderr" "$T/log"; } || newline_bad=$((newline_bad+1))
if [ "$newline_bad" = 0 ]; then ok 'IT-241 W1-308a real-length newline-free tokens authenticate show once and reserve markers through fresh repeat and reissue'; else bad 'IT-241 W1-308a newline-free credential lifecycle' "$newline_bad controls"; fi

sum_fresh
;;
esac

if [ -n "${IT_ONLY:-}" ] && [ "$sel_left" -gt 0 ]; then echo "install-test: unknown test id in IT_ONLY (no cell reported it):$(for u_i in $IT_ONLY; do case $sel_hit in *" $u_i "*) ;; *) printf ' %s' "$u_i" ;; esac; done)"; exit 2; fi
echo "install-test: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
