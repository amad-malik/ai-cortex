#!/bin/sh
# Logic test for mac/install-mac.sh (the Mac installer, OPTION C of kai 4aca0f04: its OWN file, in mac/ and NOT in deploy/ because scripts/release.sh copies the whole deploy/ tree into the LINUX release (cox 850ef045 (1)); deploy/install.sh and the Linux includes are NOT touched) with a FAKE docker, a FAKE launchd tool, a FAKE
# rendering gate and FAKE GNU and BSD tool directories: no container, no network, no real secret, no Mac. It judges install-mac.sh's OWN logic against fakes: it does NOT prove that Docker Desktop, Homebrew's GNU
# tools, Apple's /bin/sh (bash 3.2.57), ssh-keygen -Y or launchd behave as the fakes say: ASK-A-MAC / NOT RUN until M-6 (a real Mac). THE CELLS ARE THE SPECIFICATION (test first): against the scaffold every BEHAVIOUR cell FAILS; IT-M10e (the Linux bytes) and IT-M12 (the shape of the file) are INVARIANTS that hold from the start and must stay true.
# Run it through the admission shim like any test:  scripts/cv2-admit-shim sh scripts/install-mac-test.sh
# Design: INSTALL-MAC-DESIGN-NOTE-nemo-r0.md. Conditions: kai e2d11f73, 4aca0f04, b935583a; cox c7f4dd60, 452201ed.
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo=$here/..
mac=${INSTALL_MAC_SH:-"$repo/mac/install-mac.sh"}
# the release's own check tests THIS tree's installer: a substituted installer (INSTALL_MAC_SH, which only the mutation run sets) under INSTALL_MAC_REQUIRE_ALL=1 is refused before anything runs
if [ "${INSTALL_MAC_REQUIRE_ALL:-0}" = 1 ] && [ -n "${INSTALL_MAC_SH:-}" ]; then echo "FAIL IT-M00 INSTALL_MAC_REQUIRE_ALL=1 with INSTALL_MAC_SH set: the release check tests the installer of this tree, not another one"; exit 1; fi
if [ "${INSTALL_MAC_REQUIRE_ALL:-0}" = 1 ] && [ "${INSTALL_MAC_NO_SELFRUN:-0}" = 1 ]; then echo "FAIL IT-M00 INSTALL_MAC_REQUIRE_ALL=1 with INSTALL_MAC_NO_SELFRUN=1: the release check runs every cell"; exit 1; fi
if [ "${INSTALL_MAC_REQUIRE_ALL:-0}" = 1 ] && [ "${INSTALL_MAC_INNER:-0}" = 1 ]; then echo "FAIL IT-M00 INSTALL_MAC_REQUIRE_ALL=1 with INSTALL_MAC_INNER=1: the release check runs every cell"; exit 1; fi
lin="$repo/deploy/install.sh"   # the LINUX installer: read by the parity cells and the byte-identity cell only
# the test homes live under a FIXED root that Docker Desktop shares on a Mac (/tmp is under /private: both are in the installer's own list), NOT under $TMPDIR: the release run puts TMPDIR inside the git tree (release.sh: <root>/target/release-work.XXXXXXXX),
# where the installer's own home check (correctly) refuses 'not under a folder that Docker Desktop shares' (cox ee587d70, the 11 Nov IT-55b class W1-166). The check stays real: no seam widens the shared list.
T=$(mktemp -d /tmp/cortex-mac-test.XXXXXXXX)
trap 'rm -rf "$T"' EXIT

# the blob of the Linux fetch include (hue's install-fetch.sh) at integration d350d6b: the Mac installer reuses it UNCHANGED (kai 4aca0f04 (1), b935583a (B)): the cell IT-M10e compares
FETCH_BLOB=27c39d81812771fcf9d8a530f68ad48dbb32813a
# the blob of the Linux installer at the Linux release head 530fbd12 (the second re-sync, kai 7a79e791; it was 71089580 at the candidate 18265612): it must stay byte-identical (kai 4aca0f04: the Linux candidate is untouched by construction). The first pin was the blob at d350d6b, 784f880f059af7e92442ea38fbca6bf92b8f2033; the third batch of Linux
# fixes changed deploy/install.sh by 73 lines added and 7 removed, and the re-sync (cox 93c0f62d, 12 Nov) takes the candidate's blob. The fetch include did not change between the two heads (FETCH_BLOB above).
# T_LINUX repin (cox 4b040336; L3, cox e91d2104): LINUX_BLOB below is the git blob of deploy/install.sh at finn's FINAL Linux pin 7eddcfa7830d0cb9dc44a15291727a1378524612 (read as L1b), which is what T_LINUX ships; it was daf8cfd8e2399e7e46568d7074732e7417343eee at the batch head b9ed7c42 and d99034cf9abf77474d652875d14fe462eb4b76cc at the Linux release head 530fbd12. If that pin changes after the reads, this line is redone.
LINUX_BLOB=8fb6bf42d426d8cde6618b23b0ded2f4d63fd0fd
# the file-secret mode: DRAFT 0444 inside a 0700 directory (Compose ignores uid, gid and mode of a FILE secret); THE DECISION IS vera's OR hue's, NOT MINE: change this one line when it is ruled
SECRET_MODE=444
# the Docker Desktop memory floor (bob's M-1b plan: 3.5 GB of limits) and the Compose floor are DRAFTS: ASK-A-MAC
MEM_FLOOR_NOTE="3.5 GB"

mkdir -p "$T/bin" "$T/state" "$T/home"
: > "$T/log"
FAKE_LOG="$T/log"; FAKE_STATE="$T/state"; export FAKE_LOG FAKE_STATE

# ---- the FAKE docker: records its arguments; simulates only what install-mac.sh calls (the Docker Desktop CLI) -----------------------------------------------------------------------------
cat > "$T/bin/docker" <<'FAKE'
#!/bin/sh
echo "$*" >> "$FAKE_LOG"
case $1 in
  version|--version) [ "${FAKE_DOCKER_DOWN:-0}" != 1 ] || { echo "Cannot connect to the Docker daemon at unix:///var/run/docker.sock. Is the docker daemon running?" >&2; exit 1; }; echo "Docker version 27.3.1" ;;
  info) [ "${FAKE_DOCKER_DOWN:-0}" != 1 ] || { echo "Cannot connect to the Docker daemon" >&2; exit 1; }; echo "Total Memory: ${FAKE_DOCKER_MEM:-8GiB}" ;;
  compose)
    shift
    while [ "${1:-}" = -f ]; do shift 2; done
    case ${1:-} in
      version) [ "${FAKE_COMPOSE_MISSING:-0}" != 1 ] || { echo "docker: 'compose' is not a docker command." >&2; exit 125; }; echo "Docker Compose version ${FAKE_COMPOSE_VERSION:-v2.29.1}" ;;
      *) echo "CORTEX_SERVER_IMAGE=${CORTEX_SERVER_IMAGE-unset} CORTEX_DB_IMAGE=${CORTEX_DB_IMAGE-unset} CORTEX_SECRETS_DIR=${CORTEX_SECRETS_DIR-unset}" >> "$FAKE_STATE/compose.env"
         case " $* " in
           *" up "*) [ "${FAKE_COMPOSE_FAIL:-0}" != 1 ] || exit 1; echo 1 > "$FAKE_STATE/generation"; echo "${CORTEX_PORT:-8601}" > "$FAKE_STATE/port" ;;
           *" build "*) exit 1 ;;   # nothing is built on Docker Desktop: a build is a failure the cells see
         esac ;;
    esac ;;
  load) [ "${FAKE_LOAD_FAIL:-0}" != 1 ] || exit 1 ;;
  image) [ "${2:-}" = inspect ] || { echo "docker: 'image $2' is not a docker command (only inspect exists)" >&2; exit 125; } ;;   # Docker has no 'image exists' (that is Podman's)
  secret) echo "SECRETCALL $*" >> "$FAKE_STATE/secret.calls"; exit 99 ;;   # no runtime secret on the Docker path: any such call is a finding
  port) [ -f "$FAKE_STATE/port" ] || exit 125; echo "127.0.0.1:$(cat "$FAKE_STATE/port")" ;;
  inspect) [ "${FAKE_INSPECT_FAIL:-0}" != 1 ] || exit 1
    case $3 in
      *State.ExitCode*) echo "exited 0" ;;
      *) echo "${FAKE_HEALTH:-healthy}" ;;
    esac ;;
  exec)
    shift
    while [ "${1#-}" != "$1" ]; do if [ "$1" = --user ] || [ "$1" = --env ]; then shift 2; else shift; fi; done   # skip flags such as -i, --user NAME and --env NAME=VALUE
    shift                                        # the container name
    case $1 in
      psql) case " $* " in *" -tAc "*) [ "${FAKE_HBA_FAIL:-0}" != 1 ] || exit 1; echo "${FAKE_HBA_RAW:-${FAKE_HBA_TRUST:-0}}"; exit 0 ;; esac; [ "${FAKE_ROLES_FAIL:-0}" != 1 ] || exit 1; cat >> "$FAKE_STATE/roles.sql" ;;
      /usr/local/bin/cortex-server)
        # `bootstrap --project P --lead L --token-file F`: the token goes to the file INSIDE the container (its /run/cortex/bootstrap.token: the container's /run, not the Mac's)
        [ "${FAKE_BOOTSTRAP_FAIL:-0}" != 1 ] || exit 1
        n=$(cat "$FAKE_STATE/count" 2>/dev/null || echo 0); n=$((n + 1)); echo "$n" > "$FAKE_STATE/count"
        echo "FAKE-TOKEN-$n" > "$FAKE_STATE/bootstrap.token" ;;
      cat) if [ "${FAKE_CAT_FAIL_ONCE:-0}" = 1 ] && [ ! -f "$FAKE_STATE/catfailed" ]; then : > "$FAKE_STATE/catfailed"; exit 1; fi
           [ "${FAKE_CAT_EMPTY:-0}" != 1 ] || exit 0
           cat "$FAKE_STATE/bootstrap.token" ;;
      rm) [ "${FAKE_EXEC_RM_FAIL:-0}" != 1 ] || exit 1
          [ "${FAKE_EXEC_STRICT_PATH:-0}" != 1 ] || [ "$*" = "rm -f /run/cortex/bootstrap.token" ] || exit 1   # with the variable set only the token copy's own path counts as that file (default OFF)
          rm -f "$FAKE_STATE/bootstrap.token" ;;
    esac ;;
esac
FAKE
chmod +x "$T/bin/docker"
# curl: the server's open health document (GET /v2/health)
mkdir -p "$T/bin-curl"
cat > "$T/bin-curl/curl" <<'FAKE'
#!/bin/sh
echo "curl $*" >> "$FAKE_LOG"
[ "${FAKE_CURL_FAIL:-0}" != 1 ] || exit 22
case "$*" in *":$(cat "$FAKE_STATE/port" 2>/dev/null || echo none)/"*) ;; *) exit 7 ;; esac
printf '{"state":"ready","version":"%s","ready":true,"pieces":[]}\n' "${FAKE_SERVER_VERSION:-0.2.1-rc.2}"
FAKE
chmod +x "$T/bin-curl/curl"
# the fault wrapper (the Linux suite's idiom): FAKE_FI is a ';' list of "<command>=<text>[@N]": a call of this command whose arguments contain <text> FAILS; @N only the Nth such call
mkdir -p "$T/bin-fi"
cat > "$T/bin-fi/fi" <<'FIEOF'
#!/bin/sh
me=${0##*/}
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
PATH=/usr/bin:/bin exec "$me" "$@"
FIEOF
chmod +x "$T/bin-fi/fi"; for c in mv cp chmod mkdir rm; do cp "$T/bin-fi/fi" "$T/bin-fi/$c"; done
# the GNU and BSD tool directories of the seam startup check (kai b935583a (B)): the seams point at Homebrew's GNU tools at two FIXED absolute paths; a tool must exist there AND say GNU in its --version text
# gnu_dir <dir> [BSD tool name ...]: the six tools; a tool named in the list says it is a BSD tool; a tool missing from the directory is simply not made
GNU_TOOLS="gsha256sum gfind gstat gtar gmv gln"
gnu_dir() {
  gd=$1; shift; rm -rf "$gd"; mkdir -p "$gd"
  for gt in $GNU_TOOLS; do
    case " $* " in *" -$gt "*) continue ;; esac     # '-gfind' in the list: the tool is MISSING
    if case " $* " in *" $gt "*) true ;; *) false ;; esac; then gtxt="$gt (bsdtar 3.5.3 - libarchive 3.5.3)"; else gtxt="$gt (GNU coreutils) 9.5"; fi
    printf '#!/bin/sh\n[ "${1:-}" = --version ] && { echo "GNUVERSION %s" >> "$FAKE_LOG"; echo "%s"; exit 0; }\necho "GNUTOOL %s $*" >> "$FAKE_LOG"\nexit 0\n' "$gt" "$gtxt" "$gt" > "$gd/$gt"; chmod +x "$gd/$gt"
  done
}
gnu_dir "$T/gnu-ok"
# the Mac's own stat (BSD: stat -f '%u %Lp' -- PATH) on top of this Linux machine's stat, for the tests only; the installer assigns MAC_STAT itself, whole line, before any include, and a test lane include reassigns it.
# FAKE_STAT_UID='PATH=UID' makes ONE path report another owner; FAKE_STAT_FAIL=PATH makes ONE path uninspectable
mkdir -p "$T/bin-stat"
cat > "$T/bin-stat/stat" <<'FAKE'
#!/bin/sh
[ "${1:-}" = -f ] && [ "${2:-}" = '%u %Lp' ] && [ "${3:-}" = -- ] && [ "$#" -eq 4 ] || { echo "fake stat: unexpected arguments: $*" >&2; exit 2; }
p=$4
[ "${FAKE_STAT_FAIL:-}" != "$p" ] || exit 1
if /usr/bin/stat -c %u / >/dev/null 2>&1; then   # portable: ok the GNU branch of the fake; a Mac takes the BSD branch below
  u=$(/usr/bin/stat -c %u -- "$p") || exit 1
  m=$(/usr/bin/stat -c %a -- "$p") || exit 1
else
  u=$(/usr/bin/stat -f %u -- "$p") || exit 1
  m=$(/usr/bin/stat -f %Lp -- "$p") || exit 1
fi
case ${FAKE_STAT_UID:-} in "$p="*) u=${FAKE_STAT_UID#*=} ;; esac
echo "$u $m"
FAKE
chmod +x "$T/bin-stat/stat"
T_STAT="$T/bin-stat/stat"; export T_STAT

PATH="$T/bin:$T/bin-curl:$PATH"; export PATH
HOME=$T/home; export HOME
CORTEX_INSTALL_WAIT=2; CORTEX_INSTALL_POLL=1; export CORTEX_INSTALL_WAIT CORTEX_INSTALL_POLL

pass=0 fail=0 skip=0
# blob_of FILE: the git blob id of a file ('blob <size>' NUL, then the bytes, hashed with SHA-1) WITHOUT git: a stock Mac has no git (its stub opens the Command Line Tools dialog)
blob_of() { { printf 'blob %s\0' "$(wc -c < "$1" | tr -d ' ')"; cat "$1"; } | { sha1sum 2>/dev/null || shasum -a 1; } | cut -d' ' -f1; }   # portable: ok the GNU sha1sum first, shasum -a 1 (a Mac's) is the fallback
skip_ids=""
# INSTALL_MAC_REQUIRE_ALL=1 (the RELEASE's own check, scripts/install-mac-release-check.sh): a cell that cannot run is a FAILURE, never a skip: the release proves ALL of it (mike, re-read of 48ad6e0: an absent mandatory cell must not let the release gate go on)
skipped() {
  if [ "${INSTALL_MAC_REQUIRE_ALL:-0}" = 1 ]; then bad "${1%% NOT RUN*}" "INSTALL_MAC_REQUIRE_ALL=1 and the cell could not run: ${1#* NOT RUN: }"; return 0; fi
  echo "skip $1"; skip=$((skip + 1)); skip_ids="$skip_ids ${1%% NOT RUN*}"
}
# not_run_29 REASON: IT-M29 could not run. With INSTALL_MAC_REQUIRE_SET=1 (the RELEASE run: the cell must run against the SIGNED set with the CTO's public key) that is a FAILURE; without it, a skip (mike MAC-MIKE-03: a skip is never a pass).
not_run_29() { if [ "${INSTALL_MAC_REQUIRE_SET:-0}" = 1 ]; then bad "IT-M29 the set made by the release recipe" "INSTALL_MAC_REQUIRE_SET=1 and the cell could not run: $1"; else skipped "IT-M29 NOT RUN: $1"; fi; }
# finish_suite: the suite's verdict and its exit. A failure is exit 1. A suite in which any cell was NOT RUN (a mandatory end-to-end cell skipped for a missing python3, ssh-keygen, GNU tool or set) is NOT a pass: it prints the last line 'NOT_RUN: <reason>' and exits 77, which scripts/ci.sh gate() reads as NOT_RUN (a failure under --strict). Only a suite with no failure and no skip exits 0.
finish_suite() {
  # THE REQUIRED CELLS (mike MAC-MIKE-04): scripts/install-mac-required-cells.txt lists EVERY cell the suite can pass. Called with 'final' (the end of the suite) the suite checks both ways: a cell that printed ok and is NOT listed is a FAILURE (the list must stay complete), and under INSTALL_MAC_REQUIRE_ALL=1 a LISTED cell that did not print ok is a FAILURE (a cell silently omitted by a switch or a filter cannot pass the release check)
  if [ "${1:-}" = final ]; then
    fs_req="$here/install-mac-required-cells.txt"
    if [ -f "$fs_req" ]; then
      for fs_o in $ok_ids; do grep -qxF -- "$fs_o" "$fs_req" || bad "$fs_o" "the cell is not in scripts/install-mac-required-cells.txt: add it (the release check requires every listed cell to run)"; done
      if [ "${INSTALL_MAC_REQUIRE_ALL:-0}" = 1 ]; then
        while IFS= read -r fs_r; do
          case $fs_r in ""|"#"*) continue ;; esac
          case " $ok_ids " in *" $fs_r "*) ;; *) bad "$fs_r" "INSTALL_MAC_REQUIRE_ALL=1: the required cell $fs_r did not print ok (omitted, filtered or failed)" ;; esac
        done < "$fs_req"
      fi
    else bad "IT-M00" "scripts/install-mac-required-cells.txt is missing"; fi
  fi
  echo "install-mac-test: $pass passed, $fail failed, $skip not run"
  [ "$fail" -eq 0 ] || exit 1
  if [ "$skip" -gt 0 ]; then echo "NOT_RUN: $skip mandatory cell(s) not run (${skip_ids# }): the suite proved less than all of it; the release run must run them (IT-M29 with INSTALL_MAC_REQUIRE_SET=1 and the signed set)"; exit 77; fi
  exit 0
}
py_t=$(command -v python3 2>/dev/null || :)   # the same rule as the installer's py_usable: on a Mac without the Command Line Tools /usr/bin/python3 is a stub that opens an install dialog
if [ "$py_t" = /usr/bin/python3 ] && [ "$(uname -s)" = Darwin ] && ! xcode-select -p >/dev/null 2>&1; then py_t=""; fi   # portable: ok Darwin only: the stub would open a dialog
ok_ids=""
ok() { echo "ok   $1"; pass=$((pass + 1)); ok_ids="$ok_ids ${1%% *}"; }
bad() { echo "FAIL $1"; [ -z "${2:-}" ] || echo "        $2"; fail=$((fail + 1)); }
fresh() { : > "$T/log"; rm -rf "$T/h" "$T/state"; mkdir -p "$T/state"; }

# ---- a COPY of the Mac installer the cells run (mac/ is the repository's directory of the Mac installer, OUTSIDE deploy/: cox 850ef045 (1)); the real Linux includes are NOT in it -------------------------------------
# mkdeploy [lane text]: $T/m/mac (the Mac installer and its own include, side by side as the release artifact will lay them out) and $T/m/scripts (the fake rendering gate and the fake M-1c tool, where '$here/../scripts' finds them);
# with a lane text, install-prereq.sh IS the lane text (the fake machine look, prerequisites and release functions); WITHOUT one the REAL check-only include is used (the fetch include is not copied: it is a Linux file, read by IT-M10e only)
mkdeploy() {
  rm -rf "$T/m"; mkdir -p "$T/m/mac" "$T/m/scripts"
  [ ! -f "$mac" ] || cp "$mac" "$T/m/mac/install-mac.sh"
  [ ! -f "$(dirname "$mac")/install-prereq.sh" ] || cp "$(dirname "$mac")/install-prereq.sh" "$T/m/mac/install-prereq.sh"
  printf '#!/bin/sh\necho "RENDER $*" >> "$FAKE_LOG"\nexit "${FAKE_RENDER_RC:-0}"\n' > "$T/m/scripts/docker-compose-render"
  printf '#!/bin/sh\necho "LAUNCHD $*" >> "$FAKE_LOG"\nexit "${FAKE_LAUNCHD_RC:-0}"\n' > "$T/m/scripts/launchd-backup"
  printf '#!/bin/sh\necho "LAUNCHD-RUN $*" >> "$FAKE_LOG"\n' > "$T/m/scripts/launchd-run"
  chmod +x "$T/m/scripts/docker-compose-render" "$T/m/scripts/launchd-backup" "$T/m/scripts/launchd-run"
  if [ "$#" -gt 0 ]; then printf '%s\n' "$1" > "$T/m/mac/install-prereq.sh"; fi
}
LANE_FLOW='machine_look() { echo ML >> "$FAKE_LOG"; return 0; }
prereq_run() { echo PR >> "$FAKE_LOG"; return 0; }'
# mkrel: an INSTALLED release at $T/h/current (the Mac layout: the compose file, the Docker override, the roles script, the config example, the backup scripts, the launchd templates, stack.env)
SRV_IMG="localhost/cortex-server@sha256:$(printf 'a%.0s' $(seq 64))"
DB_IMG="docker.io/pgvector/pgvector@sha256:$(printf 'b%.0s' $(seq 64))"
mkrel() {
  rel="$T/h/current"; rm -rf "$rel"; mkdir -p "$rel/bin" "$rel/launchd" "$rel/initdb" "$rel/docker"
  cp "$repo/deploy/compose.yml" "$rel/"; echo "# fake override of M-1b" > "$rel/docker/compose.docker.yml"
  cp "$repo/deploy/db-roles.sh" "$rel/"; cp "$repo/deploy/cortex.toml.example" "$rel/"
  for f in cortex-dump cortex-dump-status cortex-dump-alert cortex-restore restore-compare; do printf '#!/bin/sh\n# release script %s\n' "$f" > "$rel/bin/$f"; done
  for u in ai.kaidera.cortex-dump.plist.in ai.kaidera.cortex-dump-status.plist.in; do cp "$repo/deploy/launchd/$u" "$rel/launchd/$u"; done
  printf '%s\n' "CORTEX_VERSION=0.2.1-rc.2
CORTEX_SERVER_IMAGE=$SRV_IMG
CORTEX_DB_IMAGE=$DB_IMG" > "$rel/stack.env"
}
# mrun <env VAR=value ...> -- <installer arguments>: a run of the COPY with the standard arguments; sets rc, $T/out and $T/log
mrun() {
  mr_env=""; while [ "$#" -gt 0 ] && [ "$1" != -- ]; do mr_env="$mr_env $1"; shift; done; [ "$#" -eq 0 ] || shift
  # shellcheck disable=SC2086
  mr_wb=--with-backups; [ "${MR_NO_WB:-0}" != 1 ] || mr_wb=""   # the installer's default is u2 (no launchd step); most cells exercise the launchd path, so mrun asks for it; MR_NO_WB=1 runs the default
  env $mr_env sh "$T/m/mac/install-mac.sh" --project p --lead lead --home "$T/h" $mr_wb "$@" > "$T/out" 2>&1 && rc=0 || rc=$?
}
problems() { grep -c '^problem: ' "$T/out" || true; }
fixes() { grep -c '^fix: ' "$T/out" || true; }
one_refusal() { # one_refusal <wanted rc>: the run refused with that exit code and ONE problem line and ONE fix line
  [ "$rc" = "$1" ] && [ "$(problems)" = 1 ] && [ "$(fixes)" = 1 ]
}

# ============================================================================================================================================================================================
# IT-M01..M03 the CHECK-ONLY Mac prerequisites (kai b935583a (A)): the real include, --check (the machine look: it changes nothing and returns its status), nothing created, nothing started
# ============================================================================================================================================================================================
b01=""
mkdeploy
fresh; mrun FAKE_DOCKER_DOWN=1 -- --check
one_refusal 3 || b01="$b01 [Docker Desktop not running: rc=$rc problems=$(problems) fixes=$(fixes)]"
grep -q '^problem: .*Docker Desktop' "$T/out" || b01="$b01 [the cause does not name Docker Desktop]"
! grep -qiE '^(problem|fix): .*(podman|systemd|systemctl|cgroup|sudo|dnf|apt)' "$T/out" || b01="$b01 [a Linux cause or fix on the Mac: $(grep -iE '^(problem|fix): ' "$T/out" | head -1 | cut -c1-100)]"
{ [ ! -e "$T/h" ] && ! grep -qE '^(compose|load|secret|exec) ' "$T/log"; } || b01="$b01 [the check created a home or started something]"
fresh; mrun PATH=/usr/bin:/bin -- --check
one_refusal 3 || b01="$b01 [no docker command at all: rc=$rc problems=$(problems) fixes=$(fixes)]"
grep -q '^problem: Docker Desktop is not installed' "$T/out" || b01="$b01 [no docker command: the cause does not say that Docker Desktop is not installed: $(grep '^problem: ' "$T/out" | head -1 | cut -c1-90)]"
fresh; mrun FAKE_COMPOSE_MISSING=1 -- --check
one_refusal 3 || b01="$b01 [docker compose missing: rc=$rc problems=$(problems) fixes=$(fixes)]"
grep -q '^problem: docker compose is not available' "$T/out" || b01="$b01 [compose missing: the cause is not 'docker compose is not available' (a GNU-tool refusal also gives exit 3): $(grep '^problem: ' "$T/out" | head -1 | cut -c1-90)]"
fresh; mrun FAKE_COMPOSE_VERSION=v1.29.2 -- --check
one_refusal 3 || b01="$b01 [docker compose v1 (the override's YAML tags need v2: the floor is ASK-A-MAC): rc=$rc]"
grep -q '^problem: docker compose is older than version' "$T/out" || b01="$b01 [compose v1: the cause is not 'docker compose is older': $(grep '^problem: ' "$T/out" | head -1 | cut -c1-90)]"
fresh; mrun FAKE_DOCKER_MEM=2GiB -- --check
one_refusal 3 || b01="$b01 [Docker Desktop's VM memory below the floor ($MEM_FLOOR_NOTE, a DRAFT): rc=$rc]"
grep -q '^problem: Docker Desktop gives its virtual machine too little memory' "$T/out" || b01="$b01 [low memory: the cause is not the memory floor: $(grep '^problem: ' "$T/out" | head -1 | cut -c1-90)]"
fresh; mrun FAKE_DOCKER_MEM=lots -- --check
{ one_refusal 3 && grep -q '^problem: the memory of the Docker Desktop virtual machine could not be read' "$T/out"; } || b01="$b01 [an unreadable memory line: rc=$rc: $(grep '^problem: ' "$T/out" | head -1 | cut -c1-90)]"
# the GOOD case: --check also runs the GNU tool check (IT-M10), and this VM has no Homebrew: the REAL include text plus the test lane's one line that reassigns the directories (the environment cannot) to the fake all-GNU directory
T_GNU="$T/gnu-ok"; export T_GNU
mkdeploy "$(cat "$(dirname "$mac")/install-prereq.sh" 2>/dev/null || :)
MAC_GNU_DIRS=\"\$T_GNU\"
MAC_STAT=\"\$T_STAT\""
fresh; mrun -- --check
{ [ "$rc" = 0 ] && ! grep -q '^problem: ' "$T/out"; } || b01="$b01 [a good Docker Desktop is refused by --check: rc=$rc: $(tail -1 "$T/out" | cut -c1-80)]"
if [ -z "$b01" ]; then ok "IT-M01..M02 the check-only prerequisites: Docker Desktop not running, docker compose missing or v1, the VM memory under the floor: exit 3 with ONE problem and ONE fix (Docker Desktop named, no Linux cause), nothing created, nothing started; a good one passes --check"; else bad "IT-M01..M02 check-only prerequisites" "$b01"; fi

# IT-M01c the Compose floor is 2.24.4, PATCH included (mike MAC-MIKE-01: a major.minor floor let 2.24.0 to 2.24.3 through, and the override needs 2.24.4). The include and the GNU directory are as in the good case above, so an accepted version really passes --check.
b01c=""
mkdeploy "$(cat "$(dirname "$mac")/install-prereq.sh" 2>/dev/null || :)
MAC_GNU_DIRS=\"\$T_GNU\"
MAC_STAT=\"\$T_STAT\""
for v in v2.24.3 v2.24.0 2.24.3 v2.23.9 v2.9.9 v2.24 v2.24. v1.99.99; do
  fresh; mrun FAKE_COMPOSE_VERSION=$v -- --check
  { one_refusal 3 && grep -q '^problem: docker compose is older than version 2\.24\.4' "$T/out"; } || b01c="$b01c [compose $v is below 2.24.4 and must be refused with exit 3 and the cause 'older than version 2.24.4': rc=$rc $(grep '^problem: ' "$T/out" | head -1 | cut -c1-110)]"
done
for v in v2.24.4 2.24.4 v2.24.5-desktop.1 v2.24.4-desktop.2 v2.29.1-desktop.1 v2.25.0 v2.100.0 v3.0.0 v10.0.0; do
  fresh; mrun FAKE_COMPOSE_VERSION=$v -- --check
  { [ "$rc" = 0 ] && ! grep -q '^problem: ' "$T/out"; } || b01c="$b01c [compose $v is at or above 2.24.4 and must pass --check: rc=$rc $(grep '^problem: ' "$T/out" | head -1 | cut -c1-110)]"
done
# a form that is not a plain version or Docker Desktop's -desktop.N is refused (a pre-release of 2.24.4 is OLDER than 2.24.4: mike's re-read of 48ad6e0): ONE problem that says the form is not recognised, never 'older'
for v in v2.24.4-rc1 v2.24.4-rc.1 v2.24.4-beta.1 v2.25.0-rc1 v2.24.4+dev v2.24.4-desktop. v2.24.4-desktop.x v2.24.4-desktop.1-foo v2.24.4-; do
  fresh; mrun FAKE_COMPOSE_VERSION=$v -- --check
  { one_refusal 3 && grep -q '^problem: the docker compose version has a form this installer does not recognise' "$T/out"; } || b01c="$b01c [compose $v has a suffix that is not -desktop.N: it must be refused with exit 3 and the cause 'a form this installer does not recognise': rc=$rc $(grep '^problem: ' "$T/out" | head -1 | cut -c1-110)]"
done
if [ -z "$b01c" ]; then ok "IT-M01c the Compose floor is 2.24.4 with the PATCH compared: 2.24.3, 2.24.0, 2.23.9, 2.9.9, 2.24 (no patch) and 1.x are refused (exit 3, ONE problem naming 2.24.4); 2.24.4, 2.24.5-desktop.1, 2.24.4-desktop.2, 2.25.0, 2.100.0, 3.0.0 and 10.0.0 pass --check; a pre-release or build suffix (-rc1, -beta.1, +dev, -desktop. with no number or a non-number) is refused as a form not recognised"; else bad "IT-M01c the Compose floor is 2.24.4" "$b01c"; fi
b03=""
mkdeploy
fresh; env sh "$T/m/mac/install-mac.sh" --project p --lead lead --home /opt/not-a-shared-folder/h --check > "$T/out" 2>&1 && rc=0 || rc=$?
one_refusal 2 || b03="$b03 [the home outside Docker Desktop's shared folders: rc=$rc problems=$(problems) fixes=$(fixes)]"
[ ! -e /opt/not-a-shared-folder ] || b03="$b03 [the refusal created the directory]"
if [ -z "$b03" ]; then ok "IT-M03 a home outside Docker Desktop's shared folders (/Users, /Volumes, /private, /tmp, /var/folders) is refused with exit 2 (EX_MACHINE), ONE problem and ONE fix, nothing created"; else bad "IT-M03 home outside a shared folder" "$b03"; fi

b03b=""
mkdeploy "$LANE_FLOW"
fresh; mkdir -p "$T/real"; ln -s "$T/real" "$T/h"; env sh "$T/m/mac/install-mac.sh" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?
one_refusal 2 || b03b="$b03b [the home is a symbolic link: rc=$rc problems=$(problems) fixes=$(fixes)]"
fresh; env sh "$T/m/mac/install-mac.sh" --project p --lead lead --home /tmp > "$T/out" 2>&1 && rc=0 || rc=$?
one_refusal 2 || b03b="$b03b [the home is a directory of someone else (/tmp): rc=$rc problems=$(problems) fixes=$(fixes)]"
if [ -z "$b03b" ]; then ok "IT-M03b a home that is a symbolic link, or a directory that is not yours, is refused with exit 2, ONE problem and ONE fix"; else bad "IT-M03b the home must be a real directory of yours" "$b03b"; fi

# ============================================================================================================================================================================================
# IT-M04 SECRETS are FILES (no runtime secret): three files, 64 hex characters, a 0700 directory, the mode as ruled, NEVER overwritten, never printed, no 'secret' call
# ============================================================================================================================================================================================
b04=""
mkdeploy "$LANE_FLOW"
fresh; mkdir -p "$T/h"; mkrel; mrun --
[ "$rc" = 0 ] || b04="$b04 [the full run failed: rc=$rc: $(tail -1 "$T/out" | cut -c1-80)]"
sd="$T/h/secrets"
[ -d "$sd" ] && [ "$(stat -c %a "$sd" 2>/dev/null || stat -f %Lp "$sd")" = 700 ] || b04="$b04 [the secrets directory is not a 0700 directory]"   # portable: ok the GNU form first, BSD stat -f is the fallback on the same line
for s in db-superuser owner-password app-password; do
  f="$sd/$s"
  [ -f "$f" ] || { b04="$b04 [$s is missing]"; continue; }
  v=$(cat "$f"); case $v in *[!0123456789abcdef]*) b04="$b04 [$s is not hex]" ;; esac
  [ "${#v}" = 64 ] || b04="$b04 [$s is ${#v} characters, not 64]"
  [ "$(stat -c %a "$f" 2>/dev/null || stat -f %Lp "$f")" = "$SECRET_MODE" ] || b04="$b04 [$s has another mode than $SECRET_MODE]"   # portable: ok the GNU form first, BSD stat -f is the fallback on the same line
  ! grep -qF "$v" "$T/out" || b04="$b04 [$s was PRINTED]"
  ! grep -qF "$v" "$T/log" || b04="$b04 [$s reached a command line or the call log]"
done
sum1=$(cat "$sd"/db-superuser "$sd"/owner-password "$sd"/app-password | cksum)
[ ! -s "$T/state/secret.calls" ] || b04="$b04 [a runtime 'secret' call was made: $(head -1 "$T/state/secret.calls")]"
mrun --
sum2=$(cat "$sd"/db-superuser "$sd"/owner-password "$sd"/app-password | cksum)
[ "$sum1" = "$sum2" ] || b04="$b04 [a second run REPLACED a secret]"
g64=$(printf 'g%.0s' $(seq 64)); chmod u+w "$sd/app-password" 2>/dev/null || :; printf '%s' "$g64" > "$sd/app-password" 2>/dev/null || :   # (guarded: a cell must never end the suite)
mrun --
{ [ "$rc" = 6 ] && [ "$(problems)" = 1 ] && [ "$(fixes)" = 1 ]; } || b04="$b04 [an existing secret of 64 characters that are not hex: rc=$rc (want 6) problems=$(problems) fixes=$(fixes)]"
[ "$(cat "$sd/app-password")" = "$g64" ] || b04="$b04 [a bad existing secret was REPLACED (the installer never replaces or deletes one)]"
if [ -z "$b04" ]; then ok "IT-M04 the three secrets are FILES (db-superuser, owner-password, app-password): 64 hex characters, a 0700 directory, mode $SECRET_MODE, never replaced by a second run, never printed or on a command line, and no runtime 'secret' call"; else bad "IT-M04 secret files" "$b04"; fi

# ============================================================================================================================================================================================
# IT-M05 THE COMPOSE VECTORS and the rendering gate; IT-M06 the image is NOT built
# ============================================================================================================================================================================================
b05=""
cf="$T/h/current/compose.yml"; of="$T/h/current/docker/compose.docker.yml"
[ "$(grep -c '^compose ' "$T/log" || true)" -gt 0 ] || b05="$b05 [no compose call at all]"
! grep '^compose ' "$T/log" | grep -qvF "compose -f $cf -f $of " || b05="$b05 [a compose call did not carry '-f compose.yml -f docker/compose.docker.yml', base first: $(grep '^compose ' "$T/log" | grep -vF "compose -f $cf -f $of " | head -1 | cut -c1-110)]"
[ "$(grep -c '^RENDER ' "$T/log" || true)" = 1 ] || b05="$b05 [the rendering gate ran $(grep -c '^RENDER ' "$T/log" || true) times, not ONCE]"
first_up=$(grep -n '^compose .* up ' "$T/log" | head -1 | cut -d: -f1); rend=$(grep -n '^RENDER ' "$T/log" | head -1 | cut -d: -f1)
{ [ -n "$first_up" ] && [ -n "$rend" ] && [ "$rend" -lt "$first_up" ]; } || b05="$b05 [the rendering gate did not run BEFORE the first up]"
if [ -n "$py_t" ]; then grep -q -- '--lint' "$T/log" || b05="$b05 [the rendering gate was not run with --lint although a usable python3 exists]"; else ! grep -q -- '--lint' "$T/log" || b05="$b05 [the rendering gate was run with --lint although no usable python3 exists]"; grep -q 'hardening lint of the rendered configuration was NOT run' "$T/out" || b05="$b05 [no plain line says the lint was not run]"; fi
fresh; mkdir -p "$T/h"; mkrel; mrun FAKE_RENDER_RC=1 --
{ [ "$rc" = 6 ] && ! grep -qE '^compose .* up ' "$T/log"; } || b05="$b05 [a FAILING rendering gate: rc=$rc (want 6) and it must start NOTHING (up calls: $(grep -cE '^compose .* up ' "$T/log" || true))]"
[ "$(problems)" = 1 ] && [ "$(fixes)" = 1 ] || b05="$b05 [the failed gate did not give ONE problem and ONE fix]"
if [ -z "$b05" ]; then ok "IT-M05 every compose call carries '-f compose.yml -f docker/compose.docker.yml' (base first); the rendering gate (--lint) runs ONCE before the first up; a failing gate ends in exit 6 and starts nothing"; else bad "IT-M05 compose vectors and the rendering gate" "$b05"; fi
# IT-M05b THE HARDENING LINT RUNS ONLY WHERE python3 IS A REAL ONE: on a machine that looks like a Mac without the Command Line Tools (a fake uname that says Darwin and a fake xcode-select that fails; /usr/bin/python3 stands for the stub) the install still finishes, the rendering gate runs WITHOUT --lint, and ONE plain line says the lint was not run.
if [ -x /usr/bin/python3 ]; then
  b05b=""
  rm -rf "$T/fb05"; mkdir -p "$T/fb05"; printf '#!/bin/sh\necho Darwin\n' > "$T/fb05/uname"; printf '#!/bin/sh\nexit 1\n' > "$T/fb05/xcode-select"; chmod 755 "$T/fb05/uname" "$T/fb05/xcode-select"
  fresh; mkdir -p "$T/h"; mkrel; mrun PATH="$T/fb05:$PATH" --
  [ "$rc" = 0 ] || b05b="$b05b [the install did not finish: rc=$rc: $(tail -1 "$T/out" | cut -c1-80)]"
  [ "$(grep -c '^RENDER ' "$T/log" || true)" = 1 ] || b05b="$b05b [the rendering gate did not run once]"
  ! grep '^RENDER ' "$T/log" | grep -q -- '--lint' || b05b="$b05b [the lint was asked for although python3 is the stub]"
  grep -q 'hardening lint of the rendered configuration was NOT run' "$T/out" || b05b="$b05b [no plain line says the lint was not run]"
  if [ -z "$b05b" ]; then ok "IT-M05b without a usable python3 (the Command Line Tools stub) the install finishes, the rendering gate runs without --lint and ONE plain line says the lint was NOT run"; else bad "IT-M05b the lint without python3" "$b05b"; fi
else skipped "IT-M05b NOT RUN: there is no /usr/bin/python3 on this machine to stand for the stub"; fi
b06=""
fresh; mkdir -p "$T/h"; mkrel; mrun --
[ "$rc" = 0 ] || b06="$b06 [rc=$rc]"
! grep -qE '^compose .* build' "$T/log" || b06="$b06 [the release was BUILT: Docker Desktop installs a LOADED image]"
[ "$(grep -o 'CORTEX_SERVER_IMAGE=[^ ]*' "$T/state/compose.env" | sort -u)" = "CORTEX_SERVER_IMAGE=$SRV_IMG" ] || b06="$b06 [stack.env's server image did not reach compose]"
[ "$(grep -o 'CORTEX_SECRETS_DIR=[^ ]*' "$T/state/compose.env" | sort -u)" = "CORTEX_SECRETS_DIR=$T/h/secrets" ] || b06="$b06 [CORTEX_SECRETS_DIR was not exported to compose]"
if [ -z "$b06" ]; then ok "IT-M06 nothing is built: the images of stack.env reach compose and CORTEX_SECRETS_DIR names the secrets directory"; else bad "IT-M06 the image is loaded, not built" "$b06"; fi

# IT-M06b THE ROLES STEP: deploy/db-roles.sh (a LINUX file, unchanged) runs `podman exec ... psql` and finds podman by PATH: a private helper named podman that runs the pinned docker is the only podman it reaches, for that one call; it is gone afterwards, also on a failure
b06b=""
fresh; mkdir -p "$T/h"; mkrel; mrun --
[ -s "$T/state/roles.sql" ] || b06b="$b06b [the roles script's SQL did not reach 'docker exec' (no roles.sql)]"
# defect J of the Linux candidate (kai's first real install, 9 Nov): the platform migration grants to cortex_m_platform, which no enabled module is: the roles step makes that role too (the id platform follows the module ids)
grep -qF "'cortex_m_platform'" "$T/state/roles.sql" 2>/dev/null || b06b="$b06b [the platform role (cortex_m_platform, the whole name in quotes) was not made by the roles step]"
for m06 in audit identity tokens policy records search handoffs boot; do grep -qF "'cortex_m_$m06'" "$T/state/roles.sql" 2>/dev/null || b06b="$b06b [the module role cortex_m_$m06 was not made]"; done
grep -qE '^exec -i --user postgres --env PGOPTIONS=.* cv2-s5-db psql ' "$T/log" || b06b="$b06b [the roles call did not arrive as docker exec -i --user postgres --env PGOPTIONS=... cv2-s5-db psql: $(grep ' psql ' "$T/log" | tail -1 | cut -c1-100)]"
[ -z "$(ls -d "$T"/h/.cortex-install-shim.* 2>/dev/null)" ] || b06b="$b06b [the helper directory was left in the home after a good run]"
fresh; mkdir -p "$T/h"; mkrel; mrun FAKE_ROLES_FAIL=1 --
{ [ "$rc" = 6 ] && [ "$(problems)" = 1 ] && [ "$(fixes)" = 1 ]; } || b06b="$b06b [a failing roles step: rc=$rc (want 6) problems=$(problems) fixes=$(fixes)]"
[ -z "$(ls -d "$T"/h/.cortex-install-shim.* 2>/dev/null)" ] || b06b="$b06b [the helper directory was left in the home after a FAILED roles step]"
grep -qE '^compose .* up -d $' "$T/log" && b06b="$b06b [the server was started although the roles step failed]" || :
if [ -z "$b06b" ]; then ok "IT-M06b the roles step reaches docker through the private podman helper (no Linux file changed), makes the module roles and the platform role, the helper directory is gone after a good run and after a failed one, and a failed step starts nothing more"; else bad "IT-M06b the roles step" "$b06b"; fi

# ============================================================================================================================================================================================
# IT-M07 mint_token on docker exec: the Linux IT-144/IT-146 matrix mirrored: every failure branch leaves NO token copy in the container, a failing removal is ONE warning that names it
# ============================================================================================================================================================================================
b07=""
fresh; mkdir -p "$T/h"; mkrel; mrun --
pre7=$(grep -n '^exec cv2-s5-server rm -f /run/cortex/bootstrap.token$' "$T/log" | head -1 | cut -d: -f1); boot7=$(grep -n 'cortex-server bootstrap ' "$T/log" | head -1 | cut -d: -f1)
{ [ -n "$pre7" ] && [ -n "$boot7" ] && [ "$pre7" -lt "$boot7" ]; } || b07="$b07 [the PRE-CLEAN of a stale token copy ('rm -f /run/cortex/bootstrap.token' in the container) does not run before the bootstrap call: pre=${pre7:-none} bootstrap=${boot7:-none}]"
mfault() { # mfault <what> <removal in the container fails: 0 or 1> <VAR=value>...: wanted: exit 7, ONE problem line, and (0) no copy left and no warning, (1) exactly ONE warning that names the copy
  f7=$1; r7=$2; shift 2; fresh; mkdir -p "$T/h"; chmod 0700 "$T/h"; mkrel; rm -f "$T/ficount".*
  env PATH="$T/bin-fi:$PATH" FAKE_FI_COUNT="$T/ficount" FAKE_EXEC_RM_FAIL="$r7" FAKE_EXEC_STRICT_PATH=1 "$@" sh "$T/m/mac/install-mac.sh" --project p --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?
  c7=""
  [ "$rc" = 7 ] || c7="$c7 rc=$rc (want 7: $(tail -1 "$T/out" | cut -c1-70))"
  [ "$(problems)" = 1 ] || c7="$c7 problem lines $(problems)"
  n7=$(grep -c '^install: warning: the temporary copy of the token in the server container could not be removed' "$T/out" || true)
  if [ "$r7" = 1 ]; then [ "$n7" = 1 ] || c7="$c7 [the container removal failed and $n7 warning(s) name it (want exactly 1)]"
  else
    [ ! -e "$T/state/bootstrap.token" ] || c7="$c7 [the copy of the token in the container was LEFT]"
    [ "$n7" = 0 ] || c7="$c7 [a warning although the removal worked: $n7]"
  fi
  [ -z "$c7" ] || b07="$b07 [$f7:$c7]"
}
mfault "the token copy fails" 0 FAKE_CAT_FAIL_ONCE=1
mfault "the token copy is empty" 0 FAKE_CAT_EMPTY=1
mfault "the chmod fails" 0 FAKE_FI=chmod=lead.token.part
mfault "the move fails" 0 FAKE_FI=mv=lead.token
mfault "the chmod fails and the partial file cannot be removed" 0 'FAKE_FI=chmod=lead.token.part;rm=lead.token.part@2'
mfault "the token copy fails, the removal in the container fails" 1 FAKE_CAT_FAIL_ONCE=1
mfault "the chmod fails, the removal in the container fails" 1 FAKE_FI=chmod=lead.token.part
if [ -z "$b07" ]; then ok "IT-M07 mint_token on docker exec: every failure branch (the token copy failing or empty, the chmod, the move, each with the partial file not removable) leaves NO token copy in the container; a failing container removal is ONE warning that names it; exit 7, ONE problem line; the path of every removal is judged"; else bad "IT-M07 the token copy in the container" "$b07"; fi

# ============================================================================================================================================================================================
# IT-M08 THE UNITS STEP: the M-1c tool (scripts/launchd-backup) installs the two launch agents; its refusal ends the install (u1, the DEFAULT while kai has not ruled u1 or u2: row 2efd2075)
# ============================================================================================================================================================================================
b08=""
fresh; mkdir -p "$T/h"; mkrel; mrun --
[ "$rc" = 0 ] || b08="$b08 [rc=$rc]"
[ "$(grep -c '^LAUNCHD install ' "$T/log" || true)" = 1 ] || b08="$b08 [the launchd tool was called $(grep -c '^LAUNCHD install ' "$T/log" || true) times, not once]"
{ [ -f "$T/h/bin/launchd-run" ] && [ "$(ls -ld "$T/h/bin/launchd-run" | cut -c1-10)" = "-rwxr-xr-x" ] && cmp -s "$T/m/scripts/launchd-run" "$T/h/bin/launchd-run"; } || b08="$b08 [launchd-run (the agents run it from <home>/bin) is not installed there, mode 755, as the copy of scripts/launchd-run]"
grep -qF "LAUNCHD install --home $T/h --db-container cv2-s5-db" "$T/log" || b08="$b08 [the tool did not get '--home <home> --db-container cv2-s5-db': $(grep '^LAUNCHD ' "$T/log" | head -1 | cut -c1-100)]"
! grep -qiE 'systemctl|enable-linger|loginctl' "$T/log" "$T/out" || b08="$b08 [a systemd or lingering word on the Mac]"
fresh; mkdir -p "$T/h"; mkrel; mrun FAKE_LAUNCHD_RC=2 --
{ [ "$rc" = 8 ] && [ "$(problems)" = 1 ] && [ "$(fixes)" = 1 ]; } || b08="$b08 [the launchd tool REFUSES (exit 2: THE FLOOR, the backup tools are not portable yet): rc=$rc (want 8, u1), problems=$(problems) fixes=$(fixes)]"
if [ -z "$b08" ]; then ok "IT-M08 the units step calls scripts/launchd-backup once with '--home <home> --db-container cv2-s5-db' (no systemd or lingering word); a refusal of the tool ends the install with exit 8, ONE problem and ONE fix (u1)"; else bad "IT-M08 the launchd units step" "$b08"; fi

# IT-M08b THE u2 SHAPE IS THE DEFAULT (kai 566fae3e, the CTO's D172: no working Mac backup tools in the 0.2.2 release): with no flag the install FINISHES (exit 0) although the launchd tool would refuse, the tool is never called, ONE warning says the backups are NOT scheduled and that no automatic backup exists on the Mac in this release, and the LAST line says so;
# --without-backups is still accepted; the launchd step is turned on ONLY by --with-backups on argv (not by the environment, not by a config line) and then a refusing tool ends the install with exit 8 (u1); a release that carries NO backup scripts installs by default and is refused (release_incomplete, exit 5) when --with-backups asks for them.
b08b=""
MR_NO_WB=1
fresh; mkdir -p "$T/h"; mkrel; mrun FAKE_LAUNCHD_RC=2 --
[ "$rc" = 0 ] || b08b="$b08b [the default (u2) with a refusing tool: rc=$rc (want 0)]"
! grep -q '^LAUNCHD ' "$T/log" || b08b="$b08b [the launchd tool was called although u2 is the default]"
[ ! -e "$T/h/bin/launchd-run" ] || b08b="$b08b [launchd-run was installed although the backups are not scheduled]"
n8=$(grep -c '^install: warning: .*backups are NOT scheduled' "$T/out" || true)
[ "$n8" = 1 ] || b08b="$b08b [$n8 warning(s) that the backups are not scheduled (want exactly 1)]"
grep -q 'no automatic backup exists on the Mac in this release' "$T/out" || b08b="$b08b [the output does not say that no automatic backup exists on the Mac in this release]"
grep -q '^install: done' "$T/out" || b08b="$b08b [no closing 'install: done' line]"
tail -1 "$T/out" | grep -qi 'backups are NOT scheduled' || b08b="$b08b [the LAST line does not say that the backups are not scheduled (the warning above it does not count)]"
fresh; mkdir -p "$T/h"; mkrel; mrun FAKE_LAUNCHD_RC=2 -- --without-backups
{ [ "$rc" = 0 ] && ! grep -q '^LAUNCHD ' "$T/log"; } || b08b="$b08b [--without-backups is no longer accepted or calls the tool: rc=$rc]"
# argv only: nothing but --with-backups on argv turns the launchd step on
fresh; mkdir -p "$T/h"; mkrel; cp "$T/h/current/cortex.toml.example" "$T/h/cortex.toml"; echo 'with_backups = true' >> "$T/h/cortex.toml"
mrun FAKE_LAUNCHD_RC=2 without_backups=0 with_backups=1 WITH_BACKUPS=1 CORTEX_WITH_BACKUPS=1 CORTEX_BACKUPS=1 --
{ [ "$rc" = 0 ] && ! grep -q '^LAUNCHD ' "$T/log"; } || b08b="$b08b [the environment or a config file turned the launchd step on: rc=$rc, launchd calls=$(grep -c '^LAUNCHD ' "$T/log" || true)]"
MR_NO_WB=0
fresh; mkdir -p "$T/h"; mkrel; mrun FAKE_LAUNCHD_RC=2 --
[ "$rc" = 8 ] || b08b="$b08b [with --with-backups a refusing tool must end the install with exit 8 (u1): rc=$rc]"
# a release that does not carry the backup scripts
fresh; mkdir -p "$T/h"; mkrel; rm -f "$T/h/current/bin/"cortex-dump* "$T/h/current/bin/cortex-restore" "$T/h/current/bin/restore-compare"
MR_NO_WB=1; mrun --; MR_NO_WB=0
{ [ "$rc" = 0 ] && grep -q '^install: done' "$T/out"; } || b08b="$b08b [a release without backup scripts did not install by default: rc=$rc: $(tail -1 "$T/out" | cut -c1-80)]"
fresh; mkdir -p "$T/h"; mkrel; rm -f "$T/h/current/bin/"cortex-dump* "$T/h/current/bin/cortex-restore" "$T/h/current/bin/restore-compare"
mrun --
{ [ "$rc" = 5 ] && grep -q 'backup script' "$T/out"; } || b08b="$b08b [--with-backups on a release without backup scripts: rc=$rc (want 5, release_incomplete): $(tail -1 "$T/out" | cut -c1-80)]"
if [ -z "$b08b" ]; then ok "IT-M08b u2 is the DEFAULT (the 0.2.2 Mac installer): the install finishes although the launchd tool would refuse, no launchd call, ONE warning and the last line say that no automatic backup exists on the Mac in this release; --without-backups still works; only --with-backups on argv turns the launchd step on (not the environment, not a config line; then a refusing tool gives exit 8); a release without backup scripts installs by default and is refused with --with-backups"; else bad "IT-M08b the u2 default" "$b08b"; fi

# ============================================================================================================================================================================================
# IT-M09 the exit table and the catalogue (the same codes as the Linux installer; the causes name Docker Desktop and launchd, never podman, systemd or cgroup v2)
# ============================================================================================================================================================================================
b09=""
mkdeploy "$LANE_FLOW"
fresh; env sh "$T/m/mac/install-mac.sh" > "$T/out" 2>&1 && rc=0 || rc=$?
[ "$rc" = 1 ] || b09="$b09 [no arguments: rc=$rc (want 1, EX_USAGE)]"
fresh; env sh "$T/m/mac/install-mac.sh" --project p --lead lead --home "$T/h" --frobnicate > "$T/out" 2>&1 && rc=0 || rc=$?
[ "$rc" = 1 ] || b09="$b09 [an unknown option: rc=$rc (want 1)]"
grep -q 'unknown argument' "$T/out" || b09="$b09 [the unknown option is not named]"
if [ -z "$b09" ]; then ok "IT-M09 the exit table: no arguments and an unknown option are EX_USAGE (1)"; else bad "IT-M09 exit table" "$b09"; fi

# ============================================================================================================================================================================================
# IT-M10 THE SEAMS (kai b935583a (B) (c1)): Homebrew's GNU tools at TWO FIXED absolute paths, a fail-closed STARTUP CHECK before anything is fetched, and the fetch include BYTE-IDENTICAL
# ============================================================================================================================================================================================
b10=""
GNU_LANE='MAC_GNU_DIRS="$T_GNU"
MAC_STAT="$T_STAT"'   # a test lane include reassigns the directories the installer assigned first (the environment cannot: the installer assigns them before it sources any include)
SEAM_PATH="" SEAM_ENV=""
seam_run() { # seam_run <what> <directory of the tools> <wanted tool named in the problem line or ''> <wanted words in the fix line or ''>
  mkdeploy "$LANE_FLOW
$GNU_LANE"; fresh; T_GNU=$2; export T_GNU
  env ${SEAM_ENV:-} ${SEAM_PATH:+PATH="$SEAM_PATH:$PATH"} sh "$T/m/mac/install-mac.sh" --project p --lead lead --home "$T/h" --check > "$T/out" 2>&1 && rc=0 || rc=$?
  if [ -z "$3" ]; then
    { [ "$rc" = 0 ] && ! grep -q '^problem: ' "$T/out"; } || b10="$b10 [$1: all six GNU tools were refused: rc=$rc: $(tail -1 "$T/out" | cut -c1-80)]"
  else
    one_refusal 3 || b10="$b10 [$1: rc=$rc (want 3) problems=$(problems) fixes=$(fixes)]"
    grep -q "^problem: .*$3" "$T/out" || b10="$b10 [$1: the problem line does not name $3: $(grep '^problem: ' "$T/out" | head -1 | cut -c1-100)]"
    grep -q "^fix: .*$4" "$T/out" || b10="$b10 [$1: the fix line does not say '$4': $(grep '^fix: ' "$T/out" | head -1 | cut -c1-100)]"
    ! grep -qE '^(curl|FETCH|TRUST) ' "$T/log" || b10="$b10 [$1: something was FETCHED before the check refused]"
  fi
}
seam_run "all six GNU tools present" "$T/gnu-ok" "" ""
gnu_dir "$T/gnu-nofind" -gfind;  seam_run "gfind missing" "$T/gnu-nofind" gfind "brew install findutils"
SEAM_PATH="$T/gnu-ok"; seam_run "gfind on PATH only, not in the two fixed directories (never PATH)" "$T/gnu-nofind" gfind "brew install findutils"; SEAM_PATH=""
gnu_dir "$T/gnu-bsdtar" gtar;    seam_run "a BSD tar behind the GNU-flag seam" "$T/gnu-bsdtar" gtar "brew install gnu-tar"
gnu_dir "$T/gnu-nosha" -gsha256sum; seam_run "gsha256sum missing" "$T/gnu-nosha" gsha256sum "brew install coreutils"
mkdir -p "$T/gnu-empty"; seam_run "the directory holds no tool (a wrong path)" "$T/gnu-empty" "g[a-z0-9]*" "brew install"
# the startup check runs BEFORE the first step of a release run: nothing is fetched when a tool is missing
mkdeploy "$LANE_FLOW
rel_fetch() { echo \"FETCH \$1 \$2\" >> \"\$FAKE_LOG\"; return 0; }
$GNU_LANE"; fresh; T_GNU="$T/gnu-nofind"; export T_GNU
env sh "$T/m/mac/install-mac.sh" --project p --lead lead --home "$T/h" --release 0.2.1-rc.2 --from "$T/from" > "$T/out" 2>&1 && rc=0 || rc=$?
{ [ "$rc" = 3 ] && ! grep -q '^FETCH ' "$T/log"; } || b10="$b10 [a release run with gfind missing: rc=$rc (want 3) and nothing may be fetched (FETCH lines: $(grep -c '^FETCH ' "$T/log" || true))]"
# the installer assigns the directories ITSELF, whole line, before any include, to the two fixed absolute paths; the environment cannot name them
[ "$(grep -c -x "MAC_GNU_DIRS='/opt/homebrew/bin /usr/local/bin'" "$mac" || true)" = 1 ] || b10="$b10 [MAC_GNU_DIRS is not assigned once, whole line, to '/opt/homebrew/bin /usr/local/bin']"
l1=$(grep -n -x "MAC_GNU_DIRS='/opt/homebrew/bin /usr/local/bin'" "$mac" | head -1 | cut -d: -f1); l2=$(grep -n 'install-prereq.sh' "$mac" | head -1 | cut -d: -f1)
{ [ -n "$l1" ] && [ -n "$l2" ] && [ "$l1" -lt "$l2" ]; } || b10="$b10 [MAC_GNU_DIRS is not assigned BEFORE the first include is sourced]"
mkdeploy "$LANE_FLOW"; fresh
env MAC_GNU_DIRS="$T/gnu-nofind" sh "$T/m/mac/install-mac.sh" --project p --lead lead --home "$T/h" --check > "$T/out" 2>&1 && rc=0 || rc=$?
! grep -q "gnu-nofind" "$T/out" || b10="$b10 [the ENVIRONMENT named the tools directory and the installer obeyed]"
if [ -z "$b10" ]; then ok "IT-M10a..d the seam startup check: six GNU tools at the two fixed absolute paths pass; a missing tool, a BSD tool behind a GNU-flag seam and a wrong path are refused with exit 3, ONE problem naming the tool and ONE fix saying how to install it, BEFORE anything is fetched; the environment cannot name the directories"; else bad "IT-M10 the seams and the startup check" "$b10"; fi
b10e=""
[ "$(blob_of "$repo/deploy/install-fetch.sh")" = "$FETCH_BLOB" ] || b10e="$b10e [deploy/install-fetch.sh is not the blob $FETCH_BLOB: the Mac installer reuses it UNCHANGED]"
[ "$(blob_of "$repo/deploy/install.sh")" = "$LINUX_BLOB" ] || b10e="$b10e [deploy/install.sh is not the blob $LINUX_BLOB: a Mac pin touches no Linux byte]"
if [ -z "$b10e" ]; then ok "IT-M10e the Linux installer (deploy/install.sh) and the fetch include (deploy/install-fetch.sh) are the UNCHANGED blobs of the Linux candidate 18265612 (the fetch include is also the one of d350d6b)"; else bad "IT-M10e the Linux bytes" "$b10e"; fi

# IT-M10f the seams, once armed, reach the RIGHT tool: a lane whose machine_look calls every rel_tool_* the fetch file uses for a GNU option (it runs AFTER the startup check armed them in a run that has the fetch functions)
b10f=""
mkdeploy "rel_fetch() { return 0; }
verify_release_dir() { return 0; }
machine_look() { rel_tool_find /x; rel_tool_stat /x; rel_tool_sha256sum /x; rel_tool_tar /x; rel_tool_mv /x; rel_tool_ln /x; TAR_OPTIONS=-x rel_tool_tar /y || echo TAR_REFUSED >> \"\$FAKE_LOG\"; }
$GNU_LANE"
fresh; mkdir -p "$T/h/releases"; mkrel; mv "$T/h/current" "$T/h/releases/0.2.1-rc.2"; ln -s releases/0.2.1-rc.2 "$T/h/current"; T_GNU="$T/gnu-ok"; export T_GNU
mrun -- --no-start
[ "$rc" = 0 ] || b10f="$b10f [the run with the seams armed: rc=$rc: $(tail -1 "$T/out" | cut -c1-90)]"
for s in "gfind /x" "gstat /x" "gsha256sum /x" "gtar /x" "gmv /x" "gln /x"; do grep -qx "GNUTOOL $s" "$T/log" || b10f="$b10f [the seam did not reach 'GNUTOOL $s']"; done
grep -qx 'TAR_REFUSED' "$T/log" || b10f="$b10f [the tar seam ran with TAR_OPTIONS set (GNU tar reads it as initial options: it must refuse)]"
! grep -qx 'GNUTOOL gtar /y' "$T/log" || b10f="$b10f [the refused tar call ran the tool]"
if [ -z "$b10f" ]; then ok "IT-M10f the redefined seams reach the right GNU tool (find, stat, sha256sum, tar, mv, ln) once the startup check armed them, and the tar seam refuses a TAR_OPTIONS in the environment"; else bad "IT-M10f the seams reach the right tool" "$b10f"; fi

# IT-M10g THE TOOLS THE SEAMS RUN MUST NOT BE REPLACEABLE BY OTHERS (kai 7e00cb44, cox 3a7fdab8): the RESOLVED real file and its physical directory (Homebrew's bin entries are symlinks into the Cellar) and the directory named in MAC_GNU_DIRS
# are refused when WORLD-WRITABLE or owned by a user who is neither root nor the invoker; GROUP-writable (Homebrew's normal state for the admin group) is NOT refused; the Mac's own stat (MAC_STAT) does the looking, never the GNU seam;
# a tool is not even RUN (--version) before it has passed; a path that cannot be inspected is a refusal (fail closed)
b10g=""; b10=""   # (seam_run appends its findings to b10, which IT-M10 has already reported: they are collected here)
ran_g() { grep -q "^GNUVERSION $1\$" "$T/log"; }   # the tool was run for its --version text
gnu_dir "$T/gnu-ww"; chmod o+w "$T/gnu-ww/gfind"
seam_run "a world-writable tool file" "$T/gnu-ww" gfind "chmod"
! ran_g gfind || b10g="$b10g [a world-writable gfind was RUN (--version) before it was judged]"
gnu_dir "$T/gnu-wwd"; chmod o+w "$T/gnu-wwd"
seam_run "a world-writable tool directory" "$T/gnu-wwd" "g[a-z0-9]*" "chmod"
rm -rf "$T/gnu-real" "$T/gnu-lnk"; gnu_dir "$T/gnu-real"; mkdir -p "$T/gnu-lnk"; for gt in $GNU_TOOLS; do ln -s "../gnu-real/$gt" "$T/gnu-lnk/$gt"; done
seam_run "tools that are symbolic links to good real files (Homebrew's bin entries) pass" "$T/gnu-lnk" "" ""
chmod o+w "$T/gnu-lnk"
seam_run "a world-writable directory that holds the links (the real files are fine)" "$T/gnu-lnk" "g[a-z0-9]*" "chmod"
chmod o-w "$T/gnu-lnk"
chmod o+w "$T/gnu-real/gtar"
seam_run "a link whose REAL file is world-writable (the link's directory is fine)" "$T/gnu-lnk" gtar "chmod"
chmod o-w "$T/gnu-real/gtar"; chmod o+w "$T/gnu-real"
seam_run "a link whose real file's DIRECTORY is world-writable" "$T/gnu-lnk" "g[a-z0-9]*" "chmod"
chmod o-w "$T/gnu-real"
SEAM_ENV="FAKE_STAT_UID=$T/gnu-ok/gtar=12345"; seam_run "a tool owned by a user who is neither root nor you" "$T/gnu-ok" gtar "chmod"
SEAM_ENV="FAKE_STAT_UID=$T/gnu-ok/gtar=0"; seam_run "a tool owned by root passes" "$T/gnu-ok" "" ""
SEAM_ENV="FAKE_STAT_FAIL=$T/gnu-ok/gfind"; seam_run "a tool that cannot be inspected is refused (fail closed)" "$T/gnu-ok" gfind "chmod"
SEAM_ENV=""
gnu_dir "$T/gnu-gw"; chmod g+w "$T/gnu-gw" "$T/gnu-gw"/*
seam_run "group-writable tools and directory (Homebrew's normal state for the admin group) are NOT refused" "$T/gnu-gw" "" ""
b10g="$b10g$b10"
# the Mac's own stat is assigned by the installer, whole line, before the first include, and the environment cannot name it
[ "$(grep -c -x "MAC_STAT='/usr/bin/stat'" "$mac" || true)" = 1 ] || b10g="$b10g [MAC_STAT is not assigned once, whole line, to '/usr/bin/stat']"
l3=$(grep -n -x "MAC_STAT='/usr/bin/stat'" "$mac" | head -1 | cut -d: -f1); l4=$(grep -n 'install-prereq.sh' "$mac" | head -1 | cut -d: -f1)
{ [ -n "$l3" ] && [ -n "$l4" ] && [ "$l3" -lt "$l4" ]; } || b10g="$b10g [MAC_STAT is not assigned BEFORE the first include is sourced]"
printf '#!/bin/sh\necho used > "%s/envstat.marker"\nexit 1\n' "$T" > "$T/evil-stat"; chmod +x "$T/evil-stat"
mkdeploy "$LANE_FLOW
MAC_GNU_DIRS=\"\$T_GNU\""; fresh; T_GNU="$T/gnu-ok"; export T_GNU
env MAC_STAT="$T/evil-stat" sh "$T/m/mac/install-mac.sh" --project p --lead lead --home "$T/h" --check > "$T/out" 2>&1 && rc=0 || rc=$?
[ ! -e "$T/envstat.marker" ] || b10g="$b10g [the ENVIRONMENT named the stat tool and the installer ran it]"
if [ -z "$b10g" ]; then ok "IT-M10g the GNU tools are refused when world-writable or owned by a user who is neither root nor you (the resolved real file, its directory and the configured directory; a tool is not run before it passed; an uninspectable path is a refusal); group-writable is not refused; the environment cannot name the stat tool"; else bad "IT-M10g tools that others could replace" "$b10g"; fi

# ============================================================================================================================================================================================
# IT-M11 PARITY CELLS (kai 4aca0f04 (2)): the SHARED behaviours run against BOTH installers: the same exit codes for the same refusals (the duplicated logic: argument parsing, the home check)
# ============================================================================================================================================================================================
b11=""
mkdeploy "$LANE_FLOW"
parity() { # parity <what> <arguments...>: the Mac installer and the Linux installer give the SAME exit code
  pw=$1; shift
  fresh; env sh "$T/m/mac/install-mac.sh" "$@" > "$T/out" 2>&1 && rm1=0 || rm1=$?
  fresh; env sh "$lin" "$@" > "$T/out.lin" 2>&1 && rl=0 || rl=$?
  [ "$rm1" = "$rl" ] || b11="$b11 [$pw: the Mac installer gave $rm1, the Linux installer $rl]"
}
parity "no arguments"
parity "no --lead" --project p
parity "a relative --home" --project p --lead lead --home relative/dir
parity "an unknown option" --project p --lead lead --home "$T/h" --frobnicate
if [ -z "$b11" ]; then ok "IT-M11 parity: the same refusals of the argument stage (no arguments, no --lead, a relative --home, an unknown option) give the same exit code in both installers (the Linux installer checks a port only AFTER its machine look: not an argument-stage behaviour: no parity cell for it)"; else bad "IT-M11 parity with deploy/install.sh" "$b11"; fi

# ============================================================================================================================================================================================
# IT-M13 a FINISHED install is recognised first and a second run changes nothing; IT-M14 a database that trusts is refused (parity with Linux); IT-M15 --reissue-token mints a NEW 0600 file with --require-existing and starts nothing
# ============================================================================================================================================================================================
b13=""
mkdeploy "$LANE_FLOW"
fresh; mkdir -p "$T/h"; mkrel; mrun --
[ "$rc" = 0 ] || b13="$b13 [the first run: rc=$rc]"
: > "$T/log"; mrun --
{ [ "$rc" = 0 ] && grep -q '^install: already installed and healthy' "$T/out"; } || b13="$b13 [the second run did not recognise the finished install: rc=$rc: $(tail -1 "$T/out" | cut -c1-80)]"
! grep -qE '^(compose|RENDER|LAUNCHD install|LAUNCHD-RUN|ML|PR|exec) ' "$T/log" || b13="$b13 [the second run changed or started something: $(grep -E '^(compose|RENDER|LAUNCHD install|LAUNCHD-RUN|ML|PR|exec) ' "$T/log" | head -1 | cut -c1-80)]"
grep -q '^LAUNCHD status' "$T/log" || b13="$b13 [the finished check did not ask the launchd tool for its status]"
# a finished install whose backup agents are not loaded (the tool's status refuses) is NOT finished: the whole sequence runs again
: > "$T/log"; mrun FAKE_LAUNCHD_RC=2 --
! grep -q '^install: already installed and healthy' "$T/out" || b13="$b13 [an install whose launchd status refuses was called finished]"
fresh; mkdir -p "$T/h"; mkrel; mrun --; chmod 600 "$T/h/secrets/app-password" 2>/dev/null || :; mrun --   # (guarded: a cell must never end the suite)
! grep -q '^install: already installed and healthy' "$T/out" || b13="$b13 [a secret file with another mode than $SECRET_MODE was called finished]"
if [ -z "$b13" ]; then ok "IT-M13 a finished install is recognised before anything runs and a second run changes nothing; a refusing launchd status makes it not finished"; else bad "IT-M13 the finished check" "$b13"; fi
b14=""
fresh; mkdir -p "$T/h"; mkrel; mrun FAKE_HBA_TRUST=2 --
{ [ "$rc" = 6 ] && [ "$(problems)" = 1 ] && [ "$(fixes)" = 1 ]; } || b14="$b14 [a database with trust rules: rc=$rc (want 6) problems=$(problems) fixes=$(fixes)]"
[ ! -s "$T/state/roles.sql" ] || b14="$b14 [the roles step ran although the database trusts]"
if [ -z "$b14" ]; then ok "IT-M14 a database whose effective rules trust is refused (exit 6, ONE problem and ONE fix) before the roles step"; else bad "IT-M14 the database trust refusal" "$b14"; fi
b15=""
fresh; mkdir -p "$T/h"; mkrel; mrun --
: > "$T/log"; sleep 1; mrun -- --reissue-token
[ "$rc" = 0 ] || b15="$b15 [--reissue-token: rc=$rc: $(tail -1 "$T/out" | cut -c1-80)]"
grep -q -- 'bootstrap --project p --lead lead --token-file /run/cortex/bootstrap.token --require-existing$' "$T/log" || b15="$b15 [the reissue did not pass --require-existing (a reissue must never create)]"
! grep -qE '^(compose|RENDER|LAUNCHD) ' "$T/log" || b15="$b15 [a reissue started or changed something]"
n15=$(ls "$T/h"/lead.token.2* 2>/dev/null | wc -l | tr -d ' '); [ "$n15" = 1 ] || b15="$b15 [$n15 new token file(s), want exactly 1]"
for f in "$T/h"/lead.token.2*; do [ "$(ls -ld "$f" | cut -c1-10)" = "-rw-------" ] || b15="$b15 [the new token file is not mode 600]"; done
if [ -z "$b15" ]; then ok "IT-M15 --reissue-token mints ONE new 0600 file with --require-existing and starts or changes nothing"; else bad "IT-M15 reissue" "$b15"; fi
# IT-M17 THE COMPLETION SUMMARY (defect N of the Linux candidate, kai's full pass 9 Nov): the prerequisite include's say_done takes a FOURTH value, the install log, and REFUSES an empty one, so a good install ended with its usage problem line and no endpoint. Both
# places that call it (the end of an install, and a second run that finds the install finished) pass FOUR values; the fourth is the word none when CORTEX_LOG is not set and the file when it is. The Mac check-only include has no say_done today; a build whose include has one
# (the joined Mac artifact of kai's ruling C) must not end a good install on the usage line: the lane text here defines a say_done that records what it was given.
b17=""
SAYDONE_FLOW='machine_look() { echo ML >> "$FAKE_LOG"; return 0; }
prereq_run() { echo PR >> "$FAKE_LOG"; return 0; }
say_done() { echo "SAYDONE $#|$1|$2|$3|${4-}" >> "$FAKE_LOG"; }'
mkdeploy "$SAYDONE_FLOW"
fresh; mkdir -p "$T/h"; mkrel; mrun --
[ "$rc" = 0 ] || b17="$b17 [the first run: rc=$rc]"
want17="SAYDONE 4|0.2.1-rc.2|http://127.0.0.1:8601/v2/health/ready|$T/h/lead.token|none"
[ "$(grep -c '^SAYDONE ' "$T/log")" = 1 ] && grep -qxF "$want17" "$T/log" || b17="$b17 [the end of an install did not call say_done once with four values ending in none: $(grep '^SAYDONE ' "$T/log" | tail -2 | cut -c1-150 | tr '\n' '|')]"
: > "$T/log"; mrun --
{ [ "$rc" = 0 ] && grep -q '^install: already installed and healthy' "$T/out"; } || b17="$b17 [the second run did not recognise the finished install: rc=$rc]"
[ "$(grep -c '^SAYDONE ' "$T/log")" = 1 ] && grep -qxF "$want17" "$T/log" || b17="$b17 [a finished install did not call say_done once with four values ending in none: $(grep '^SAYDONE ' "$T/log" | tail -2 | cut -c1-150 | tr '\n' '|')]"
fresh; mkdir -p "$T/h"; mkrel; : > "$T/log"; mrun CORTEX_LOG="$T/inst.log" --
grep -qxF "SAYDONE 4|0.2.1-rc.2|http://127.0.0.1:8601/v2/health/ready|$T/h/lead.token|$T/inst.log" "$T/log" || b17="$b17 [with CORTEX_LOG set the fourth value of an install was not that file: $(grep '^SAYDONE ' "$T/log" | tail -1 | cut -c1-150)]"
: > "$T/log"; mrun CORTEX_LOG="$T/inst.log" --
grep -qxF "SAYDONE 4|0.2.1-rc.2|http://127.0.0.1:8601/v2/health/ready|$T/h/lead.token|$T/inst.log" "$T/log" || b17="$b17 [with CORTEX_LOG set the fourth value of a finished install was not that file: $(grep '^SAYDONE ' "$T/log" | tail -1 | cut -c1-150)]"
mkdeploy "$LANE_FLOW"
if [ -z "$b17" ]; then ok "IT-M17 the completion summary is given FOUR values at both call sites (an install and a finished one): the fourth is none without CORTEX_LOG and the file with it"; else bad "IT-M17 the completion summary" "$b17"; fi
# IT-M18 WHY THE MAC DOES NOT PORT DEFECT M (kai's full pass, 9 Nov): the Linux installer removes the server container that podman-compose had started after a FAILED migration (podman-compose does not wait for migrate: its depends_on conditions have no effect). The Mac
# starts the same compose file with Docker Compose, which waits for a dependency's successful completion (compose-spec depends_on condition service_completed_successfully: 'a dependency is expected to run to successful completion before starting a dependent service', the services
# page of docs.docker.com, read on 12 Nov; what Docker Compose does after a NON-ZERO exit is not stated there: ASK-A-MAC). So the server of this stack must wait for migrate with that condition; this cell is the guard that the reason stays true. If the condition is ever dropped from
# deploy/compose.yml the cell fails and the Linux cleanup (own_server_cleanup in deploy/install.sh) must be ported to docker.
b18=""
awk '/^  [a-z0-9-]+:[ ]*$/ { svc = $1 } svc == "server:" && /^      migrate:/ { m = 1; next } svc == "server:" && m && /condition: service_completed_successfully/ { f = 1 } svc == "server:" && m && /^      [a-z]/ { m = 0 } END { exit !f }' "$repo/deploy/compose.yml" || b18="$b18 [in deploy/compose.yml the server does not depend on migrate with condition: service_completed_successfully]"
if [ -z "$b18" ]; then ok "IT-M18 the compose file the Mac starts makes the server wait for the migration's successful completion (the reason the Linux cleanup of a half-started server, defect M, is not ported)"; else bad "IT-M18 the server waits for migrate" "$b18"; fi

# ============================================================================================================================================================================================
# IT-M16 THE SHARED FETCH FILE IS NOT PART OF THIS BUILD (a checkout: no install-fetch.sh beside mac/; cox bfa67e83 (C), checklist item 3): a RELEASE install (--release, --from or --base-url) REFUSES BEFORE anything is looked at, fetched or
# unpacked: exit 4 (EX_TRUST), ONE problem and ONE fix (this installer has no way to verify a release), no machine look, no docker call, no home. It never falls through to an unverified path. (Linux mirror: IT-108 refuses a fetch
# WITHOUT rel_trust, exit 4, nothing unpacked; the Linux installer has no cell for an absent fetch file because a Linux checkout installs from the checkout, which the Mac installer does not do.)
# ============================================================================================================================================================================================
b16=""
mkdeploy "$LANE_FLOW"
for a16 in "--release 0.2.1-rc.2 --from $T/from" "--release 0.2.1-rc.2 --base-url https://example.invalid/r" "--from $T/from" "--base-url https://example.invalid/r"; do
  fresh; env sh "$T/m/mac/install-mac.sh" --project p --lead lead --home "$T/h" $a16 > "$T/out" 2>&1 && rc=0 || rc=$?
  one_refusal 4 || b16="$b16 [no fetch file, '$a16': rc=$rc (want 4) problems=$(problems) fixes=$(fixes)]"
  grep -q '^problem: this installer has no way to verify a release' "$T/out" || b16="$b16 ['$a16': the cause is not 'no way to verify a release': $(grep '^problem: ' "$T/out" | head -1 | cut -c1-80)]"
  { [ ! -s "$T/log" ] && [ ! -e "$T/h" ]; } || b16="$b16 ['$a16': something was looked at, fetched or made before the refusal: log=[$(tr '\n' ' ' < "$T/log" | cut -c1-80)]]"
done
fresh; mkdir -p "$T/h"; mkrel; mrun --
[ "$rc" = 0 ] || b16="$b16 [a plain install of an installed release without --release must still work with no fetch file: rc=$rc]"
if [ -z "$b16" ]; then ok "IT-M16 with no fetch file in the build, a release install (--release, --from or --base-url) is refused BEFORE anything is looked at, fetched or unpacked: exit 4, ONE problem and ONE fix, no docker call, no home; an install without --release is not affected"; else bad "IT-M16 no fetch file: a release install refuses" "$b16"; fi

# ============================================================================================================================================================================================
# ============================================================================================================================================================================================
# THE SECOND RE-SYNC (kai row 7a79e791, 14 Nov): the Linux hunks of 18265612 -> 530fbd12 that apply to a Mac (the table is in the pin note). Cells IT-M19 to IT-M24; the Linux cells they mirror are IT-156, IT-157, IT-164,
# IT-165, IT-158/159, IT-161, IT-162, IT-163 and IT-166/167 of scripts/install-test.sh. NOT mirrored (no end-to-end fetch route in this build: the fetch include is not joined here, kai's ruling C): the full-run cells IT-166/167
# of the download's curl; the Mac covers that text by IT-M24 (the names, the function run on its own, the call site) and the Linux cells cover the same bytes in deploy/install.sh.
# ============================================================================================================================================================================================
mkdeploy "$LANE_FLOW"
# IT-M19 THE TWO NAME CHECKS SAY THE RULE IN WORDS (Linux IT-156): a refused --project or --lead name gets a cause, a fix and a last sentence that each say it; exit 1, nothing touched.
b19=""
for o19 in project lead; do
  for v19 in 'A' 'a_b' '1a' 'abcdefghijklmnopqrstuvwxyz0123456789'; do
    fresh
    if [ "$o19" = project ]; then sh "$T/m/mac/install-mac.sh" --project "$v19" --lead lead --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?; else sh "$T/m/mac/install-mac.sh" --project p --lead "$v19" --home "$T/h" > "$T/out" 2>&1 && rc=0 || rc=$?; fi
    w19="--$o19 '$v19'"
    [ "$rc" = 1 ] || b19="$b19 [$w19: rc=$rc, want 1]"
    { [ "$(grep -c '^problem: ' "$T/out")" = 1 ] && [ "$(grep -c '^fix: ' "$T/out")" = 1 ]; } || b19="$b19 [$w19: not ONE problem: line and ONE fix: line]"
    grep -q "^problem: the $o19 name is not allowed: .*small letter" "$T/out" || b19="$b19 [$w19: the cause does not name the option and say the rule in words]"
    grep -q "^fix: give --$o19 a name .*small letter.*run this installer again" "$T/out" || b19="$b19 [$w19: the fix does not name the option, say the rule in words and say what to do]"
    tail -1 "$T/out" | grep -q "^install: --$o19 .*small letter, then only small letters, digits and hyphens, 32 characters at most" || b19="$b19 [$w19: the last line is not the rule in words: $(tail -1 "$T/out" | cut -c1-90)]"
    { [ ! -e "$T/h" ] && [ ! -s "$T/log" ]; } || b19="$b19 [$w19: something was touched]"
  done
done
if [ -z "$b19" ]; then ok "IT-M19 a refused --project or --lead name says the rule in words (small letter first, then small letters, digits and hyphens, 32 characters at most) in its cause, its fix and its last line, exit 1, nothing touched"; else bad "IT-M19 the name messages" "$b19"; fi
# IT-M20 THE THREE DATABASE-RULES LINES SAY WHAT THEY DID NOT TOUCH, AND NO BLANKET NO-CHANGE CLAIM STANDS AFTER THE FIRST WRITE (Linux IT-157, IT-164, IT-165): the unreadable query (exit 6), a count that is not a number (exit 6) and a
# database that trusts (exit 6) end in the words of the Linux installer with docker for podman; the home, the configuration and the three secrets exist, the roles step did not run, no volume was touched; and every 'nothing was changed',
# 'nothing was written' or 'never changes or deletes' of mac/install-mac.sh is a comment or lies BEFORE the first write (the first stage_make or home_make), none in a catalogue text.
b20=""
for c20 in 'FAKE_HBA_FAIL=1|could not read the database.s access rules: check docker logs cv2-s5-db, then run this installer again \(that is safe\)|cv2-s5-db' 'FAKE_HBA_RAW=abc|could not read a count of trust rules from the database|check docker logs cv2-s5-db, then run this installer again \(that is safe\)' 'FAKE_HBA_TRUST=2|trust or broken pg_hba rule|see S5-PEER-AUTH-MAP.md'; do
  e20=${c20%%|*}; r20=${c20#*|}; w20=${r20%%|*}; x20=${r20#*|}
  fresh; mkdir -p "$T/h"; mkrel; mrun "$e20" --
  [ "$rc" = 6 ] || b20="$b20 [$e20: rc=$rc, want 6]"
  { [ "$(problems)" = 1 ] && [ "$(fixes)" = 1 ]; } || b20="$b20 [$e20: not ONE problem: line and ONE fix: line]"
  grep -qE "$w20" "$T/out" || b20="$b20 [$e20: the line does not say '$w20']"
  grep -qiE "$x20" "$T/out" || b20="$b20 [$e20: no '$x20']"
  grep -q "this refusal did not touch the database's rules, data or volume, and what the installer has made so far stays in place" "$T/out" || b20="$b20 [$e20: the words are not there]"
  ! grep -qiE 'nothing was changed|never changes or deletes' "$T/out" || b20="$b20 [$e20: a blanket no-change claim is still there: $(grep -iE 'nothing was changed|never changes or deletes' "$T/out" | head -1 | cut -c1-90)]"
  { [ -s "$T/h/cortex.toml" ] || [ -d "$T/h/secrets" ]; } && [ "$(ls "$T/h/secrets" 2>/dev/null | wc -l | tr -d ' ')" -ge 3 ] && [ ! -s "$T/state/roles.sql" ] && ! grep -qE 'volume (rm|prune)|rm .*--volumes|compose .* down' "$T/log" || b20="$b20 [$e20: the state after the run is not the one the words describe]"
done
fw20=$(grep -n -E '^( +)?(stage_make|home_make)$' "$mac" | head -1 | cut -d: -f1)
case $fw20 in ""|*[!0123456789]*) b20="$b20 [the first write of the sequence was not found]" ;; esac
n20=0
for ln20 in $(grep -n -E 'nothing was changed|nothing was written|never changes or deletes' "$mac" | grep -v -E '^[0123456789]+:[[:space:]]*#' | cut -d: -f1); do
  n20=$((n20 + 1)); l20=$(sed -n "${ln20}p" "$mac")
  case $l20 in
    *'m_cause='*|*'m_fix='*) b20="$b20 [line $ln20: a catalogue text claims it]" ;;
    *) [ "$ln20" -lt "${fw20:-0}" ] || b20="$b20 [line $ln20: the claim is after the first write (line ${fw20:-?})]" ;;
  esac
done
[ "$n20" -ge 1 ] || b20="$b20 [no claim found at all: the cell is blind (the finished-install line is one)]"
if [ -z "$b20" ]; then ok "IT-M20 the unreadable rules, a count that is not a number and a database that trusts: the last line says what the refusal did not touch and what stays in place (exit 6, one problem, one fix); the home, secrets exist, no roles step, no volume touched; no blanket no-change claim after the first write"; else bad "IT-M20 the database-rules lines" "$b20"; fi
# IT-M21 EVERY MESSAGE ID HAS ONE TEXT IN THE JOINED INSTALLER (Linux IT-158, IT-159): the tables of mac/install-mac.sh (inst_msg), of its prerequisite include (prereq_msg) and of the REAL fetch include (fetch_msg) share no id (msg()
# asks the prerequisite table first, then the fetch table, then inst_msg: an id in two tables is answered by the wrong one: home_unsafe and trust_key were, in the Linux file, before E-01); and with the REAL fetch_msg as lane text a
# symbolic-link home gets the installer's own cause.
b21=""
ids_of() { sed -n "/^$2() {/,/^}/p" "$1" | grep -oE '^ +[a-z_0-9]+\)' | tr -d ' )'; }
ids_of "$mac" inst_msg | sort > "$T/ids.inst"; ids_of "$repo/deploy/install-fetch.sh" fetch_msg | sort > "$T/ids.fetch"; ids_of "$(dirname "$mac")/install-prereq.sh" prereq_msg | sort > "$T/ids.prereq"
[ "$(wc -l < "$T/ids.inst" | tr -d ' ')" -ge 30 ] || b21="$b21 [inst_msg of the Mac installer: only $(wc -l < "$T/ids.inst" | tr -d ' ') ids found: the cell would be blind]"
[ "$(wc -l < "$T/ids.fetch" | tr -d ' ')" -ge 10 ] || b21="$b21 [the real fetch_msg: only $(wc -l < "$T/ids.fetch" | tr -d ' ') ids found: the cell would be blind]"
dup21=$(sort "$T/ids.inst" "$T/ids.fetch" "$T/ids.prereq" | uniq -d | tr '\n' ' ')
[ -z "$dup21" ] || b21="$b21 [ids in more than one table: $dup21]"
for id21 in home_not_real anchor_unusable bad_name; do grep -qx "$id21" "$T/ids.inst" || b21="$b21 [inst_msg has no id $id21]"; done
for id21 in home_unsafe trust_key; do ! grep -qx "$id21" "$T/ids.inst" || b21="$b21 [inst_msg still has the colliding id $id21]"; done
sed -n '/^fetch_msg() {/,/^}/p' "$repo/deploy/install-fetch.sh" > "$T/fetch_msg.txt"
mkdeploy "$LANE_FLOW
$(cat "$T/fetch_msg.txt")"
fresh; rm -rf "$T/real" "$T/lnk"; mkdir -p "$T/real" "$T/lnk"; ln -s "$T/real" "$T/lnk/h"
sh "$T/m/mac/install-mac.sh" --project p --lead lead --home "$T/lnk/h" > "$T/out" 2>&1 && rc=0 || rc=$?
{ [ "$rc" = 2 ] && grep -q '^problem: the install home is not a real directory of yours' "$T/out"; } || b21="$b21 [a symbolic-link home with the real fetch_msg joined: rc=$rc (want 2): $(grep '^problem: ' "$T/out" | head -1 | cut -c1-110)]"
mkdeploy "$LANE_FLOW"
if [ -z "$b21" ]; then ok "IT-M21 the Mac message table, the prerequisite table and the real fetch table share no id (home_not_real, anchor_unusable and bad_name are the Mac's; home_unsafe and trust_key are gone), and with the real fetch_msg joined a symbolic-link home gets the installer's own cause"; else bad "IT-M21 one text per message id" "$b21"; fi
# IT-M22 THE INSTALLER LEARNS ITS OWN DIRECTORY WITH THE SHELL ALONE (Linux IT-161, Q17d step 1b): the statements from 'case $0 in' to 'here=$(' give, for each path form, the same result and the same failure as the dirname, cd and pwd
# they replace (a name without a slash is '.', /script is '/', 14 forms). Apple's /bin/sh is bash 3.2: ${0%/*} and the loop are POSIX; NOT RUN on a Mac.
b22=""
hs22=$(sed -n '/^case \$0 in$/,/^here=\$(/p' "$mac")
[ "$(printf '%s\n' "$hs22" | wc -l)" -ge 4 ] || b22="$b22 [the own-directory statements (from 'case \$0 in' to 'here=\$(') were not found in install-mac.sh]"
rm -rf "$T/own"; mkdir -p "$T/own/x/y" "$T/own/-d" "$T/own/a b"
own_new() { o22=$( ( cd "$1" && sh -c "set -eu
$hs22
printf '%s\\n' \"\$here\"" "$2" ) 2>&1 ) && r22=0 || r22=$?; printf '%s\n' "$o22" | sed 's/line [0-9]*: //'; echo "rc=$r22"; }
own_ref() { o22=$( ( cd "$1" && sh -c 'set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
printf "%s\n" "$here"' "$2" ) 2>&1 ) && r22=0 || r22=$?; printf '%s\n' "$o22" | sed 's/line [0-9]*: //'; echo "rc=$r22"; }
for c22 in "$T/own|install-mac.sh" "$T/own|./install-mac.sh" "$T/own|x/install-mac.sh" "$T/own|x//install-mac.sh" "$T/own|./x/../x/y/install-mac.sh" "$T/own|$T/own/x/install-mac.sh" "$T/own|-d/install-mac.sh" "$T/own|a b/install-mac.sh" \
           "$T/own/x|../x/install-mac.sh" "$T/own|/install-mac.sh" "$T/own|//install-mac.sh" "$T/own|///install-mac.sh" "$T/own|nodir/install-mac.sh" "$T/own|x/nodir/install-mac.sh"; do
  d22=${c22%%|*}; p22=${c22#*|}
  a22=$(own_new "$d22" "$p22"); r22=$(own_ref "$d22" "$p22")
  [ "$a22" = "$r22" ] || b22="$b22 [\$0=$p22: got '$(printf '%s' "$a22" | tr '\n' '|' | cut -c1-70)' want '$(printf '%s' "$r22" | tr '\n' '|' | cut -c1-70)']"
done
[ "$(own_new "$T/own" install-mac.sh | sed -n 1p)" = "$(cd "$T/own" && pwd)" ] || b22="$b22 [a name without a slash is not the current directory]"
[ "$(own_new "$T/own" /install-mac.sh | sed -n 1p)" = / ] || b22="$b22 [/install-mac.sh is not /]"
case $(own_new "$T/own" nodir/install-mac.sh) in *"rc=1") ;; *) b22="$b22 [a missing directory did not fail with status 1]" ;; esac
if [ -z "$b22" ]; then ok "IT-M22 the installer's own directory by the shell alone: a name without a slash is '.', /script is '/', and 14 path forms give the result and the failure of dirname, cd and pwd"; else bad "IT-M22 the own directory" "$b22"; fi
# IT-M23 NO OUTSIDE PROGRAM BEFORE THE INCLUDED PARTS, AND THE CERTIFICATE VARIABLES ARE GONE BEFORE ANY OUTSIDE PROGRAM (Linux IT-162, IT-163): (a) started with a PATH that holds no program at all, the installer reaches its first refusal (no
# arguments) and says 'not found' nowhere; (b) a fake dirname first on PATH is never started; (c) with SSL_CERT_FILE and SSL_CERT_DIR exported, EVERY recorded start of the usual tools sees both unset, in a whole good install.
b23=""
rm -rf "$T/nobin" "$T/bin-dn" "$T/bin-ssl"; mkdir -p "$T/nobin" "$T/bin-dn" "$T/bin-ssl"
env PATH="$T/nobin" /bin/sh "$T/m/mac/install-mac.sh" > "$T/out" 2>&1 && rc=0 || rc=$?
[ "$rc" = 1 ] || b23="$b23 [a PATH with no program: rc=$rc, want the usage refusal 1]"
! grep -q 'not found' "$T/out" || b23="$b23 [a program was started before the included parts: $(grep 'not found' "$T/out" | head -1 | cut -c1-90)]"
printf '#!/bin/sh\necho CANARY >> "%s/dirname.canary"\necho /nonexistent-dir\n' "$T" > "$T/bin-dn/dirname"; chmod +x "$T/bin-dn/dirname"; rm -f "$T/dirname.canary"
env PATH="$T/bin-dn:$PATH" sh "$T/m/mac/install-mac.sh" > "$T/out" 2>&1 && rc=0 || rc=$?
[ ! -e "$T/dirname.canary" ] || b23="$b23 [a fake dirname first on PATH was started before the first refusal]"
[ "$rc" = 1 ] || b23="$b23 [with a fake dirname on PATH the first refusal was not reached: rc=$rc]"
for t23 in id stat date mktemp sed cat tr head od awk grep cp mv rm mkdir chmod wc sort uniq cut find ls ln touch sleep; do
  printf '#!/bin/sh\nprintf "%%s|%%s\\n" "${SSL_CERT_FILE-unset}" "${SSL_CERT_DIR-unset}" >> "$FAKE_STATE/ssl.seen"\nPATH=/usr/bin:/bin exec %s "$@"\n' "$t23" > "$T/bin-ssl/$t23"; chmod +x "$T/bin-ssl/$t23"
done
fresh; mkdir -p "$T/h"; mkrel; mrun SSL_CERT_FILE=/nonexistent/f SSL_CERT_DIR=/nonexistent/d PATH="$T/bin-ssl:$PATH" --
[ "$rc" = 0 ] || b23="$b23 [(c) the install failed: rc=$rc: $(tail -1 "$T/out" | cut -c1-80)]"
[ "$(wc -l < "$T/state/ssl.seen" 2>/dev/null || echo 0)" -ge 5 ] || b23="$b23 [(c) the recording tools were hardly started: the cell would prove nothing]"
! grep -qv '^unset|unset$' "$T/state/ssl.seen" 2>/dev/null || b23="$b23 [(c) a program inherited a certificate variable: $(grep -v '^unset|unset$' "$T/state/ssl.seen" | head -1 | cut -c1-60)]"
if [ -z "$b23" ]; then ok "IT-M23 no outside program runs before the included parts (a PATH with no program reaches the first refusal, a fake dirname is never started) and SSL_CERT_FILE and SSL_CERT_DIR are unset before any recorded program starts in a whole install"; else bad "IT-M23 no outside program before the includes; certificate variables" "$b23"; fi
# IT-M24 WHAT THE DOWNLOAD'S CURL INHERITS (Linux IT-166, IT-167; hale O-2): (a) the one statement that unsets the variables of the whole run names SSL_CERT_FILE, SSL_CERT_DIR, CURL_CA_BUNDLE, SSLKEYLOGFILE, CURL_SSL_BACKEND and QLOGDIR and
# keeps the Docker and DYLD names, and it comes BEFORE the first include; (b) the proxy names are NOT in it (only the release download runs without them); (c) rel_fetch_noproxy, run on its own over a recording rel_fetch, hands rel_fetch all
# eight proxy names unset and gives the caller's names back exactly as they were (unset stays unset, empty stays empty, a value stays); (d) the release download is called through it and nowhere bare.
b24=""
un24=$(grep -n -m1 '^unset LD_PRELOAD' "$mac" | cut -d: -f1); in24=$(grep -n -m1 '@include' "$mac" | cut -d: -f1)
case $un24 in ""|*[!0123456789]*) b24="$b24 [the unset statement was not found]" ;; *)
  us24=$(sed -n "${un24}p" "$mac")
  for n24 in SSL_CERT_FILE SSL_CERT_DIR CURL_CA_BUNDLE SSLKEYLOGFILE CURL_SSL_BACKEND QLOGDIR DYLD_INSERT_LIBRARIES DOCKER_HOST COMPOSE_FILE; do
    case " $us24 " in *" $n24 "*) ;; *) b24="$b24 [the unset statement does not name $n24]" ;; esac
  done
  for n24 in http_proxy https_proxy HTTP_PROXY HTTPS_PROXY all_proxy ALL_PROXY no_proxy NO_PROXY; do
    case " $us24 " in *" $n24 "*) b24="$b24 [the unset statement of the whole run names the proxy name $n24: only the release download may run without it]" ;; esac
  done
  { [ -n "$in24" ] && [ "$un24" -lt "$in24" ]; } || b24="$b24 [the unset statement (line $un24) does not come before the first include (line ${in24:-none})]" ;;
esac
rf24=$(sed -n '/^rel_fetch_noproxy() {/,/^}/p' "$mac")
[ "$(printf '%s\n' "$rf24" | wc -l)" -ge 6 ] || b24="$b24 [rel_fetch_noproxy was not found in install-mac.sh]"
rm -f "$T/rf.out"
env -u http_proxy -u HTTP_PROXY -u all_proxy -u NO_PROXY https_proxy=http://127.0.0.1:9/https_proxy HTTPS_PROXY= ALL_PROXY=http://127.0.0.1:9/ALL_PROXY no_proxy= sh -c "set -eu
$rf24
rel_fetch() { for pn in http_proxy https_proxy HTTP_PROXY HTTPS_PROXY all_proxy ALL_PROXY no_proxy NO_PROXY; do eval \"pv=\\\${\$pn-unset}\"; echo \"IN \$pn=\$pv\"; done; return 7; }
rc=0; rel_fetch_noproxy 0.2.1 /stage || rc=\$?
echo \"RC \$rc\"
for pn in http_proxy https_proxy HTTP_PROXY HTTPS_PROXY all_proxy ALL_PROXY no_proxy NO_PROXY; do eval \"pv=\\\${\$pn-unset}\"; echo \"OUT \$pn=\$pv\"; done" > "$T/rf.out" 2>&1 || :
for pn24 in http_proxy https_proxy HTTP_PROXY HTTPS_PROXY all_proxy ALL_PROXY no_proxy NO_PROXY; do grep -qxF "IN $pn24=unset" "$T/rf.out" || b24="$b24 [rel_fetch still saw $pn24]"; done
grep -qxF "RC 7" "$T/rf.out" || b24="$b24 [the status of rel_fetch was not passed on: $(grep '^RC' "$T/rf.out" | head -1)]"
for pw24 in http_proxy=unset HTTP_PROXY=unset all_proxy=unset NO_PROXY=unset https_proxy=http://127.0.0.1:9/https_proxy HTTPS_PROXY= ALL_PROXY=http://127.0.0.1:9/ALL_PROXY no_proxy=; do
  grep -qxF "OUT $pw24" "$T/rf.out" || b24="$b24 [after the download the name ${pw24%%=*} is '$(grep "^OUT ${pw24%%=*}=" "$T/rf.out" | head -1 | cut -c1-70)', wanted '$pw24']"
done
grep -q 'run_step "\$EX_FETCH" "\$EX_FETCH" rel_fetch_noproxy "\$CORTEX_RELEASE" "\$CORTEX_STAGE_DIR"' "$mac" || b24="$b24 [the release download is not called through rel_fetch_noproxy]"
! grep -nE 'run_step .* rel_fetch "' "$mac" | grep -v '^[0123456789]*:[[:space:]]*#' | grep -q . || b24="$b24 [a bare rel_fetch call through run_step is left]"
if [ -z "$b24" ]; then ok "IT-M24 the variables of the whole run include the certificate, key-log, TLS-backend and QLOG names (and no proxy name) before the first include; rel_fetch_noproxy gives rel_fetch all eight proxy names unset and the caller's names back exactly; the download is called through it"; else bad "IT-M24 what the download's curl inherits" "$b24"; fi
# ============================================================================================================================================================================================
# THE JOIN STEP (kai 7a79e791, cox 3e7a28ef): IT-M25 the JOINED Mac installer, END TO END: the real joiner (scripts/release_manifest.py join-installer), the Mac COPY of the fetch include (mac/install-fetch.sh), a REAL signed Mac
# fixture (scripts/install-mac-fixture.py: a real ssh-keygen -Y sign) and NO fake rel_* function; IT-M26 python3 is judged usable only when it is a real one; IT-M27 the join and its binding record. What is NOT real here: docker (the suite's fake),
# the Homebrew GNU tools (wrappers over this machine's GNU tools, in a directory the test names: MAC_GNU_DIRS), the BSD stat (the suite's fake). The cells need python3 and GNU tools: without them they print 'skip' with the reason (a stock Mac).
# ============================================================================================================================================================================================
fxm="$repo/scripts/install-mac-fixture.py"; incm="$(dirname "$mac")/install-fetch.sh"; jnm="$repo/scripts/release_manifest.py"; prm="$(dirname "$mac")/install-prereq.sh"
gnu_real() { # gnu_real DIR: wrappers named gsha256sum gfind gstat gtar gmv gln that say GNU and run this machine's GNU tool (the plain name, or Homebrew's g-name where there is one)
  gr=$1; rm -rf "$gr"; mkdir -p "$gr"
  for gp in gsha256sum:sha256sum gfind:find gstat:stat gtar:tar gmv:mv gln:ln; do
    gg=${gp%%:*}; gb=${gp#*:}; gx=""
    for gc in "$gb" "$gg"; do gx=$(command -v "$gc" 2>/dev/null || :); [ -n "$gx" ] && case $("$gx" --version 2>/dev/null | sed -n 1p) in *GNU*) break ;; *) gx="" ;; esac; done
    [ -n "$gx" ] || return 1
    printf '#!/bin/sh\n[ "${1:-}" = --version ] && { echo "%s (GNU coreutils) 9.5"; exit 0; }\nexec %s "$@"\n' "$gg" "$gx" > "$gr/$gg"; chmod 755 "$gr/$gg"
  done
}
if [ -z "$py_t" ] || ! [ -f "$fxm" ] || ! [ -f "$incm" ] || ! [ -f "$jnm" ] || ! [ -x /usr/bin/ssh-keygen ]; then
  skipped "IT-M25 / IT-M27 NOT RUN: the joined end-to-end cells need a usable python3, /usr/bin/ssh-keygen, scripts/release_manifest.py, scripts/install-mac-fixture.py and mac/install-fetch.sh"
elif ! gnu_real "$T/gnu-real"; then
  skipped "IT-M25 / IT-M27 NOT RUN: GNU sha256sum, find, stat, tar, mv and ln are not all on PATH here (a stock Mac has the BSD ones; Homebrew's coreutils, findutils and gnu-tar give them)"
else
  b25=""; b27=""
  e25="$T/e25"
  mkfxm() { rm -rf "$e25/$1"; mkdir -p "$e25/$1"; "$py_t" -I "$fxm" "$e25/$1" 0.2.2 > "$e25/$1.out" 2>&1 || return 1; [ -f "$e25/$1/anchor/trust.keyfile" ]; }
  # joinm NAME KEYFILE [record args...]: the layout (the Mac installer as install.sh, its include, the Mac fetch copy) joined by the REAL joiner with that trust key file; then the TWO test-host lines (the GNU tool directory and the BSD stat) are changed
  joinm() {
    jm=$1; jk=$2; shift 2
    rm -rf "$e25/lay-$jm" "$e25/j-$jm"; mkdir -p "$e25/lay-$jm" "$e25/j-$jm"
    cp "$mac" "$e25/lay-$jm/install.sh"; cp "$prm" "$e25/lay-$jm/install-prereq.sh"; cp "$incm" "$e25/lay-$jm/install-fetch.sh"
    "$py_t" -I "$jnm" join-installer --deploy "$e25/lay-$jm" --output "$e25/j-$jm/install.joined.sh" --trust-key-file "$jk" "$@" > "$e25/join-$jm.out" 2>&1 || return 1
    sed -e "s#^MAC_GNU_DIRS=.*#MAC_GNU_DIRS='$T/gnu-real'#" -e "s#^MAC_STAT=.*#MAC_STAT='$T_STAT'#" "$e25/j-$jm/install.joined.sh" > "$e25/j-$jm/install.sh" || return 1
    [ "$(diff "$e25/j-$jm/install.joined.sh" "$e25/j-$jm/install.sh" | grep -c '^<')" = 2 ]   # exactly TWO lines of the joined file differ
  }
  runm() { # runm NAME RELEASE_DIR: a real install of release 0.2.2 from that directory into a fresh home; sets rc, $T/out, $T/log
    fresh; FAKE_SERVER_VERSION=0.2.2 FAKE_LOG="$T/log" sh "$e25/j-$1/install.sh" --project p --lead lead --home "$T/h" --release 0.2.2 --from "$2" --port 8777 > "$T/out" 2>&1 && rc=0 || rc=$?
  }
  if ! { mkfxm fx1 && mkfxm fx2; }; then b25="$b25 [the fixture script failed: $(head -c 200 "$e25/fx1.out" 2>/dev/null | tr '\n' '|')]"; else
    fp1=$(sed -n 's/^FPR=//p' "$e25/fx1/anchor/anchor.env"); pr1=$(sed -n 's/^PRINCIPAL=//p' "$e25/fx1/anchor/anchor.env"); ky1=$(sed -n 's/^KEY=//p' "$e25/fx1/anchor/anchor.env")
    ky2=$(sed -n 's/^KEY=//p' "$e25/fx2/anchor/anchor.env")
    # IT-M27: the join, with its binding record (schema 2: the three anchor values from the key file, the two includes by name and hash, the joined file's own hash)
    cm27=0123456789abcdef0123456789abcdef01234567; tr27=fedcba9876543210fedcba9876543210fedcba98
    if joinm a "$e25/fx1/anchor/trust.keyfile" --record "$e25/rec-a.json" --commit "$cm27" --tree "$tr27" --object-format sha1; then
      [ "$(grep -c '@include' "$e25/j-a/install.joined.sh" || true)" = 0 ] || b27="$b27 [the joined file still holds an @include marker]"
      sh -n "$e25/j-a/install.joined.sh" || b27="$b27 [the joined file is not valid shell]"
      grep -qx "CORTEX_ANCHOR_PRINCIPAL='$pr1'" "$e25/j-a/install.joined.sh" && grep -qx "CORTEX_ANCHOR_KEY='$ky1'" "$e25/j-a/install.joined.sh" && grep -qx "CORTEX_ANCHOR_FPR='$fp1'" "$e25/j-a/install.joined.sh" || b27="$b27 [the three anchor statements do not hold the key file's principal, key and recomputed fingerprint]"
      "$py_t" -I - "$e25/rec-a.json" "$e25/j-a/install.joined.sh" "$mac" "$prm" "$incm" "$cm27" "$tr27" "$fp1" > "$e25/rec27.out" 2>&1 <<'PYR' || b27="$b27 [the binding record is not as it must be: $(head -c 300 "$e25/rec27.out" | tr '\n' '|')]"
import json, hashlib, sys
rec = json.load(open(sys.argv[1])); joined = open(sys.argv[2], "rb").read()
sh = lambda p: hashlib.sha256(open(p, "rb").read()).hexdigest()
assert rec["schema"] == 2, rec["schema"]
assert rec["object_format"] == "sha1" and rec["commit"] == sys.argv[6] and rec["tree"] == sys.argv[7]
assert rec["joined_sha256"] == hashlib.sha256(joined).hexdigest()
inc = {i["name"]: i["sha256"] for i in rec["includes"]}
assert inc == {"install-prereq.sh": sh(sys.argv[4]), "install-fetch.sh": sh(sys.argv[5])}, inc
assert rec["anchor"]["fpr"] == sys.argv[8]
PYR
    else b27="$b27 [the real joiner could not join the Mac layout: $(head -c 200 "$e25/join-a.out" | tr '\n' '|')]"; fi
    if [ -z "$b27" ]; then ok "IT-M27 the REAL joiner joins the Mac layout (install.sh, install-prereq.sh, install-fetch.sh): no marker is left, the three anchor statements hold the key file's values, the file is valid shell, and the binding record (schema 2) names the two includes by hash and the joined file's own hash"; else bad "IT-M27 the join and its binding record" "$b27"; fi
    # IT-M25 (a) the right anchor and a release signed by that key: trusted, unpacked, loaded; the install stops at the module list as designed (the fixture's config example has none); the person adds one; run 2 resumes and finishes; run 3 changes nothing
    if joinm b "$e25/fx1/anchor/trust.keyfile"; then
      runm b "$e25/fx1/release"
      { [ "$rc" = 6 ] && grep -q 'no \[modules\] enabled list' "$T/out" && [ -L "$T/h/current" ] && [ "$(readlink "$T/h/current")" = "releases/0.2.2" ] && [ -f "$T/h/releases/0.2.2/stack.env" ] && [ -f "$T/h/releases/0.2.2/docker/compose.docker.yml" ]; } || b25="$b25 [run 1 (the real fetch, trust, unpack): rc=$rc (want 6 at the module list): $(tail -2 "$T/out" | tr '\n' '|' | cut -c1-200)]"
      grep -q '^load --input .*cortex-server-0.2.2-aarch64.oci.tar' "$T/log" || b25="$b25 [the image file was not loaded with docker load --input: $(grep -c '^load ' "$T/log" || true) load call(s)]"
      grep -q '^image inspect ' "$T/log" || b25="$b25 [the loaded image was not read back with docker image inspect (Docker has no 'image exists')]"
      if [ -f "$T/h/cortex.toml" ]; then printf '[modules]\nenabled = ["records"]\n' >> "$T/h/cortex.toml"; else b25="$b25 [run 1 left no cortex.toml to edit: the rest of the cell cannot go on]"; fi
      : > "$T/log"; FAKE_SERVER_VERSION=0.2.2 FAKE_LOG="$T/log" sh "$e25/j-b/install.sh" --project p --lead lead --home "$T/h" --release 0.2.2 --from "$e25/fx1/release" --port 8777 > "$T/out" 2>&1 && rc=0 || rc=$?
      { [ "$rc" = 0 ] && [ -s "$T/h/lead.token" ] && ! grep -q '^load ' "$T/log"; } || b25="$b25 [run 2 (resume): rc=$rc token=$([ -s "$T/h/lead.token" ] && echo yes || echo no) loads=$(grep -c '^load ' "$T/log" || true): $(tail -2 "$T/out" | tr '\n' '|' | cut -c1-200)]"
      grep -q '^RENDER-OF-THE-RELEASE --base .*/current/compose.yml --override .*/current/docker/compose.docker.yml' "$T/log" || b25="$b25 [the render tool of the RELEASE (bin/docker-compose-render in the unpacked set) did not run with the release's two compose files]"
      grep -q 'no automatic backup exists on the Mac in this release' "$T/out" || b25="$b25 [the closing lines do not say that no automatic backup exists]"
      ! grep -q '^LAUNCHD ' "$T/log" || b25="$b25 [a launchd tool was called in u2]"
      : > "$T/log"; FAKE_SERVER_VERSION=0.2.2 FAKE_LOG="$T/log" sh "$e25/j-b/install.sh" --project p --lead lead --home "$T/h" --release 0.2.2 --from "$e25/fx1/release" --port 8777 > "$T/out" 2>&1 && rc=0 || rc=$?
      { [ "$rc" = 0 ] && ! grep -q '^load ' "$T/log" && grep -q 'already installed' "$T/out"; } || b25="$b25 [run 3 on the finished install: rc=$rc loads=$(grep -c '^load ' "$T/log" || true): $(head -3 "$T/out" | tr '\n' '|' | cut -c1-200)]"
    else b25="$b25 [the joined subject could not be built: $(head -c 200 "$e25/join-b.out" | tr '\n' '|')]"; fi
    refused25() { # refused25 WHAT TEXT: rc 4, ONE problem line with that text, ONE fix line, nothing unpacked or loaded, no home
      r25=""
      [ "$rc" = 4 ] || r25="$r25 rc=$rc (want 4)"
      grep -qxF "problem: $2" "$T/out" || r25="$r25 [the problem line is not '$2': $(grep '^problem: ' "$T/out" | head -1 | cut -c1-90)]"
      { [ "$(grep -c '^problem: ' "$T/out" || true)" = 1 ] && [ "$(grep -c '^fix: ' "$T/out" || true)" = 1 ]; } || r25="$r25 [not exactly one problem: and one fix: line]"
      { [ ! -e "$T/h/releases" ] && ! grep -q '^load ' "$T/log"; } || r25="$r25 [something was unpacked or loaded]"
      [ -z "$r25" ] || b25="$b25 [$1:$r25]"
    }
    # (c) ONE character of the fingerprint literal changed: the key cannot be confirmed although the release is signed by the right key
    if joinm c "$e25/fx1/anchor/trust.keyfile"; then
      sed "s#^CORTEX_ANCHOR_FPR='\\(.*\\).'\$#CORTEX_ANCHOR_FPR='\\1Z'#" "$e25/j-c/install.sh" > "$e25/j-c/install.x" && mv "$e25/j-c/install.x" "$e25/j-c/install.sh"
      [ "$(grep -c "^CORTEX_ANCHOR_FPR='$fp1'" "$e25/j-c/install.sh" || true)" = 0 ] || b25="$b25 [the fingerprint literal was not changed]"
      runm c "$e25/fx1/release"; refused25 "one character of the fingerprint changed" "the release signing key could not be confirmed"
    else b25="$b25 [(c) the joined subject could not be built]"; fi
    # (d) another pair's KEY with the first pair's fingerprint
    if joinm d "$e25/fx1/anchor/trust.keyfile"; then
      sed "s#^CORTEX_ANCHOR_KEY='.*'\$#CORTEX_ANCHOR_KEY='$ky2'#" "$e25/j-d/install.sh" > "$e25/j-d/install.x" && mv "$e25/j-d/install.x" "$e25/j-d/install.sh"
      runm d "$e25/fx1/release"; refused25 "another pair's key" "the release signing key could not be confirmed"
    else b25="$b25 [(d) the joined subject could not be built]"; fi
    # (e) a complete anchor of ANOTHER pair against a release signed by the first: the key is confirmed, the signature is not
    if joinm e "$e25/fx2/anchor/trust.keyfile"; then runm e "$e25/fx1/release"; refused25 "a release signed by ANOTHER key" "the release checksum signature could not be verified"; else b25="$b25 [(e) the joined subject could not be built]"; fi
    # (f) the release's archive is changed after signing (one byte appended): refused before anything is unpacked
    if joinm f "$e25/fx1/anchor/trust.keyfile"; then
      rm -rf "$e25/rel-f"; cp -R "$e25/fx1/release" "$e25/rel-f"; printf 'x' >> "$e25/rel-f/cortex-0.2.2-macos.tar.gz"
      runm f "$e25/rel-f"; { [ "$rc" != 0 ] && [ ! -e "$T/h/releases" ] && ! grep -q '^load ' "$T/log"; } || b25="$b25 [(f) a changed archive was not refused before unpacking: rc=$rc]"
    else b25="$b25 [(f) the joined subject could not be built]"; fi
  fi
    # (g) and (h) a release whose archive LACKS a member of the Mac list (still correctly signed): refused by the closed member list (exit 5), nothing loaded
    for om25 in docker/compose.docker.yml bin/docker-compose-render; do
      rm -rf "$e25/fxo"; mkdir -p "$e25/fxo"
      if FIXTURE_OMIT=$om25 "$py_t" -I "$fxm" "$e25/fxo" 0.2.2 > "$e25/fxo.out" 2>&1 && joinm o "$e25/fxo/anchor/trust.keyfile"; then
        runm o "$e25/fxo/release"; { [ "$rc" = 5 ] && [ ! -e "$T/h/current" ] && ! grep -q '^load ' "$T/log"; } || b25="$b25 [a release without $om25 was not refused by the member list (exit 5): rc=$rc: $(tail -2 "$T/out" | tr '\n' '|' | cut -c1-160)]"
      else b25="$b25 [the fixture without $om25 could not be built]"; fi
    done
  if [ -z "$b25" ]; then ok "IT-M25 (JOINED, END TO END) the REAL joiner, the Mac fetch copy and a REAL signed Mac fixture, no fake rel_*: the right anchor installs (trusted, unpacked, loaded with docker load --input and read back with docker image inspect, the release's own render tool runs, current is the link; the module-list stop, the resume, the finished run; u2 closing lines); one character of the fingerprint, another key, a release signed by another key and a changed archive are refused with exit 4 or before unpacking, nothing unpacked or loaded"; else bad "IT-M25 the joined Mac installer end to end" "$b25"; fi
fi
# IT-M26 python3 IS USED ONLY WHEN IT IS A REAL ONE (the installer's py_usable): with /usr/bin/python3 on a Mac whose Command Line Tools are missing (a stub that opens an install dialog) it says no; with the tools present, or on another system, yes.
b26=""
pu26=$(sed -n '/^py_usable() {/,/^}/p' "$mac")
if [ -z "$pu26" ]; then b26="$b26 [py_usable was not found in install-mac.sh]"
elif [ ! -x /usr/bin/python3 ]; then skipped "IT-M26 NOT RUN: there is no /usr/bin/python3 on this machine to stand for the stub"
else
  rm -rf "$T/fb26"; mkdir -p "$T/fb26"
  printf '#!/bin/sh\necho "%s"\n' Darwin > "$T/fb26/uname-darwin"; printf '#!/bin/sh\necho Linux\n' > "$T/fb26/uname-linux"
  p26() { # p26 UNAME XCODE(0|1): the exit status of py_usable with a fake uname and a fake xcode-select, system directories only
    rm -f "$T/fb26/uname" "$T/fb26/xcode-select"; cp "$T/fb26/uname-$1" "$T/fb26/uname"; printf '#!/bin/sh\nexit %s\n' "$2" > "$T/fb26/xcode-select"; chmod 755 "$T/fb26/uname" "$T/fb26/xcode-select"
    PATH="$T/fb26:/usr/bin:/bin" sh -c "$pu26
py_usable"; }
  p26 darwin 1 && b26="$b26 [the Command Line Tools stub (Darwin, no tools) was judged usable]"
  p26 darwin 0 || b26="$b26 [a Mac WITH the tools was judged unusable]"
  p26 linux 1 || b26="$b26 [a non-Darwin system was judged unusable]"
fi
if [ -z "$b26" ]; then ok "IT-M26 python3 is used only when it is a real one: the /usr/bin/python3 stub of a Mac without the Command Line Tools is refused; with the tools, or off a Mac, it is used"; else bad "IT-M26 py_usable" "$b26"; fi
# IT-M28 THE RELEASE CHECK NEEDS A REAL python3: the joined installer on a machine that looks like a Mac without the Command Line Tools (a fake uname that says Darwin, a fake xcode-select that fails, /usr/bin/python3 as the stand-in for the stub) refuses with exit 3 and ONE problem and ONE fix BEFORE anything is fetched, unpacked or loaded.
if [ -z "$py_t" ] || ! [ -d "${e25:-/nonexistent}" ] || [ ! -x /usr/bin/python3 ] || ! [ -f "${e25:-/nonexistent}/j-b/install.sh" ]; then skipped "IT-M28 NOT RUN: it needs the joined subject of IT-M25 and /usr/bin/python3"
else
  b28=""
  mkdir -p "$T/fb28"; printf '#!/bin/sh\necho Darwin\n' > "$T/fb28/uname"; printf '#!/bin/sh\nexit 1\n' > "$T/fb28/xcode-select"; chmod 755 "$T/fb28/uname" "$T/fb28/xcode-select"
  fresh; env PATH="$T/fb28:$PATH" FAKE_SERVER_VERSION=0.2.2 FAKE_LOG="$T/log" sh "$e25/j-b/install.sh" --project p --lead lead --home "$T/h" --release 0.2.2 --from "$e25/fx1/release" --port 8777 > "$T/out" 2>&1 && rc=0 || rc=$?
  { [ "$rc" = 3 ] && [ "$(grep -c '^problem: ' "$T/out" || true)" = 1 ] && [ "$(grep -c '^fix: ' "$T/out" || true)" = 1 ] && grep -q '^problem: python3 is not usable here' "$T/out"; } || b28="$b28 [rc=$rc (want 3) and ONE problem naming python3: $(grep '^problem: ' "$T/out" | head -1 | cut -c1-100)]"
  { [ ! -e "$T/h/releases" ] && ! grep -q '^load ' "$T/log"; } || b28="$b28 [something was unpacked or loaded before python3 was judged]"
  if [ -z "$b28" ]; then ok "IT-M28 the joined installer on a Mac without the Command Line Tools (a python3 stub) refuses with exit 3, ONE problem and ONE fix, before anything is read, fetched, unpacked or loaded"; else bad "IT-M28 python3 before the release check" "$b28"; fi
fi
# IT-M29 A SET MADE BY THE RELEASE RECIPE (hue's mac-release, cox W1-281 point 5: "its output set passes IT-M25 in place of the fixture"): MAC_SET_DIR (the seven files of the set), MAC_SET_KEYFILE (the public key file it is signed with: one line 'principal ssh-ed25519 key') and MAC_SET_VERSION (default 0.2.2). The JOINED installer (the real joiner, the anchor from that key file) fetches, trusts, unpacks and loads THAT set with the real fetch copy; with a changed archive it refuses before unpacking. The module-list stop or the render check that follows
# (the suite's docker is a fake that cannot render) is accepted as the end of the route; what is proved is the route up to a laid-down release. Without a set the cell is not run.
# MAC-MIKE-05 (mike, re-read of a523b52): THE TESTED INSTALLER MUST BE THE SIGNED ONE. The cell regenerates the installer from this tree with the joiner (the set's own key file), edits the test seams and runs THAT: a differently packaged but validly signed installer in the set would be ignored while the cell passed. So the set's install.sh must be BYTE-IDENTICAL to the generated installer BEFORE any seam edit (cmp); a difference is a FAILURE that names both checksums.
# it29_check SETDIR KEYFILE VERSION: sets b29x (empty = passed) and leaves rc, $T/out and $T/log of the last run
it29_check() {
  i29_set=$1; i29_key=$2; i29_ver=$3; b29x=""; i29_e="$T/e29"
  rm -rf "${i29_e:?}/lay" "${i29_e:?}/j" "${i29_e:?}/chg"; mkdir -p "$i29_e/lay" "$i29_e/j"
  cp "$mac" "$i29_e/lay/install.sh"; cp "$prm" "$i29_e/lay/install-prereq.sh"; cp "$incm" "$i29_e/lay/install-fetch.sh"
  if "$py_t" -I "$jnm" join-installer --deploy "$i29_e/lay" --output "$i29_e/j/install.joined.sh" --trust-key-file "$i29_key" > "$i29_e/join.out" 2>&1 \
     && sed -e "s#^MAC_GNU_DIRS=.*#MAC_GNU_DIRS='$T/gnu-real29'#" -e "s#^MAC_STAT=.*#MAC_STAT='$T_STAT'#" "$i29_e/j/install.joined.sh" > "$i29_e/j/install.sh"; then
    cmp -s "$i29_e/j/install.joined.sh" "$i29_set/install.sh" || b29x="$b29x [the set's install.sh is NOT the installer under test: the joined installer of this tree with the set's key file has cksum $(cksum < "$i29_e/j/install.joined.sh" | cut -d' ' -f1), the set's install.sh $(cksum < "$i29_set/install.sh" 2>/dev/null | cut -d' ' -f1): the signed installer was not the one executed]"
    fresh; FAKE_SERVER_VERSION=$i29_ver FAKE_LOG="$T/log" sh "$i29_e/j/install.sh" --project p --lead lead --home "$T/h" --release "$i29_ver" --from "$i29_set" --port 8777 > "$T/out" 2>&1 && rc=0 || rc=$?
    case $rc in
      0) ;;
      6) grep -qE 'no \[modules\] enabled list|Docker Desktop configuration failed its own check' "$T/out" || b29x="$b29x [rc 6 but not at the module list or the render check: $(grep '^problem: ' "$T/out" | head -1 | cut -c1-100)]" ;;
      *) b29x="$b29x [the route ended before a release was laid down: rc=$rc: $(grep '^problem: ' "$T/out" | head -1 | cut -c1-120)]" ;;
    esac
    { [ -L "$T/h/current" ] && [ "$(readlink "$T/h/current")" = "releases/$i29_ver" ] && [ -f "$T/h/releases/$i29_ver/stack.env" ] && [ -f "$T/h/releases/$i29_ver/docker/compose.docker.yml" ] && [ -f "$T/h/releases/$i29_ver/bin/docker-compose-render" ]; } || b29x="$b29x [no release was laid down at releases/$i29_ver (current link, stack.env, docker/compose.docker.yml, bin/docker-compose-render)]"
    grep -q "^CORTEX_VERSION=$i29_ver\$" "$T/h/releases/$i29_ver/stack.env" 2>/dev/null || b29x="$b29x [stack.env does not say CORTEX_VERSION=$i29_ver]"
    grep -q "^load --input .*cortex-server-$i29_ver-aarch64.oci.tar" "$T/log" || b29x="$b29x [the image file was not loaded with docker load --input]"
    grep -q '^image inspect ' "$T/log" || b29x="$b29x [the loaded image was not read back with docker image inspect]"
    # a changed archive is refused before anything is unpacked
    cp -R "$i29_set" "$i29_e/chg"; chmod -R u+w "$i29_e/chg"; printf 'x' >> "$i29_e/chg/cortex-$i29_ver-macos.tar.gz"   # (the release check hands the suite a READ-ONLY snapshot: the copy is made writable to be changed)
    fresh; FAKE_SERVER_VERSION=$i29_ver FAKE_LOG="$T/log" sh "$i29_e/j/install.sh" --project p --lead lead --home "$T/h" --release "$i29_ver" --from "$i29_e/chg" --port 8777 > "$T/out" 2>&1 && rc=0 || rc=$?
    { [ "$rc" != 0 ] && [ ! -e "$T/h/releases" ] && ! grep -q '^load ' "$T/log"; } || b29x="$b29x [a changed archive was not refused before unpacking: rc=$rc]"
  else b29x="$b29x [the set could not be joined with its key file: $(head -c 200 "$i29_e/join.out" | tr '\n' '|')]"; fi
}
if [ -z "${MAC_SET_DIR:-}" ] || [ -z "${MAC_SET_KEYFILE:-}" ]; then
  not_run_29 "no set was given (MAC_SET_DIR, MAC_SET_KEYFILE, MAC_SET_VERSION): it takes a set made by the release recipe in place of the fixture"
elif [ -z "$py_t" ] || ! [ -f "$jnm" ] || ! [ -f "$incm" ] || ! gnu_real "$T/gnu-real29"; then
  not_run_29 "it needs a usable python3, scripts/release_manifest.py, mac/install-fetch.sh and the GNU tools (see IT-M25)"
else
  mv29=${MAC_SET_VERSION:-0.2.2}
  it29_check "$MAC_SET_DIR" "$MAC_SET_KEYFILE" "$mv29"; b29=$b29x
  if [ -z "$b29" ]; then ok "IT-M29 the set made by the release recipe ($MAC_SET_DIR, version $mv29): its install.sh is BYTE-IDENTICAL to the joined installer of this tree (the signed installer is the one executed); it is fetched, trusted with its own key file, unpacked and loaded by the joined Mac installer (a release is laid down, the image loaded and read back); a changed archive is refused before unpacking"; else bad "IT-M29 a set made by the release recipe" "$b29"; fi
fi
# IT-M39 THE BINDING HAS A NEGATIVE (mike MAC-MIKE-05): a validly SIGNED set whose install.sh is not the joined installer of this tree (the suite's own signed fixture: its install.sh member is a fixture text) is REFUSED by the same check, for that reason and no other.
if [ -z "$py_t" ] || [ -z "${e25:-}" ] || [ ! -f "${e25:-/nonexistent}/fx1/anchor/trust.keyfile" ] || ! gnu_real "$T/gnu-real29"; then
  skipped "IT-M39 NOT RUN: the signed-wrong-installer negative needs the join set-up of IT-M25 (python3, GNU tools, the fixture)"
else
  b39=""
  it29_check "$e25/fx1/release" "$e25/fx1/anchor/trust.keyfile" 0.2.2
  case $b29x in *"is NOT the installer under test"*) ;; *) b39="$b39 [a signed set whose install.sh is not the installer under test was NOT refused for that reason: [$(printf '%s' "$b29x" | cut -c1-160)]]" ;; esac
  [ "$(printf '%s' "$b29x" | grep -o '\[' | wc -l | tr -d ' ')" = 1 ] || b39="$b39 [it must fail for THAT reason only (the rest of the route is the same as a good set's): $(printf '%s' "$b29x" | cut -c1-200)]"
  if [ -z "$b39" ]; then ok "IT-M39 a validly SIGNED set whose install.sh is not the joined installer of this tree is refused by the IT-M29 check, for that reason only"; else bad "IT-M39 a signed wrong installer" "$b39"; fi
fi
# IT-M12 THE MAC PORT: bob's mac-port-scan finds NO unannotated error in mac/install-mac.sh; 'set -eu' within the first 20 lines (the rule of the gate s5-deploy, S5-10, which does NOT see mac/: this cell stands in for it)
# ============================================================================================================================================================================================
b12=""
py12=$py_t
if [ -z "$py12" ]; then
  skipped "IT-M12 mac-port-scan NOT RUN: python3 is not usable here (on a Mac without the Command Line Tools /usr/bin/python3 is a stub that opens an install dialog); the 'set -eu' and first-statements checks of this cell still run"
elif [ -f "$repo/scripts/mac-port-scan" ]; then
  "$py12" -I "$repo/scripts/mac-port-scan" "$mac" > "$T/scan.out" 2>&1 || :
  errs=$(sed -n 's/.*(\([0-9]*\) error.*/\1/p' "$T/scan.out" | tail -1)
  [ "${errs:-x}" = 0 ] || b12="$b12 [mac-port-scan: ${errs:-no summary line} error finding(s) in mac/install-mac.sh (want 0): $(tail -1 "$T/scan.out" | cut -c1-120)]"
else b12="$b12 [scripts/mac-port-scan is not in this tree]"; fi
[ "$(sed -n 1,20p "$mac" | grep -c '^set -eu')" = 1 ] || b12="$b12 [set -eu is not within the first 20 lines of mac/install-mac.sh]"
f3=$(grep -v '^[[:space:]]*$' "$mac" | grep -v '^[[:space:]]*#' | sed -n 1,3p | tr '\n' '|')
[ "$f3" = "CORTEX_ANCHOR_PRINCIPAL=''|CORTEX_ANCHOR_KEY=''|CORTEX_ANCHOR_FPR=''|" ] || b12="$b12 [the three CORTEX_ANCHOR statements are not the first three statements]"
if [ -z "$b12" ]; then if [ -z "$py12" ]; then ok "IT-M12 (mac-port-scan NOT RUN here: no usable python3) set -eu is within its first 20 lines; the three anchor statements are its first three"; else ok "IT-M12 mac-port-scan finds no unannotated error in mac/install-mac.sh; set -eu is within its first 20 lines; the three anchor statements are its first three"; fi; else bad "IT-M12 the Mac port" "$b12"; fi

# IT-M38 THE SUITE COMPLETES WHERE ITS JOIN SET-UP IS ABSENT (the Mac test kit's payload has no scripts/release_manifest.py and no fixture, so IT-M25, IT-M27 and IT-M29 skip): the first kit run of this suite found IT-M28 reading the unset $e25 and ending under set -u with no summary. The suite is run in a tree that lacks those two files: it must END with its summary line and exit 77 (cells skipped), never abort. INSTALL_MAC_INNER marks that inner run (no recursion); INSTALL_MAC_NO_SELFRUN (set only by the mutation run: this cell tests the SUITE, not the installer) skips the cell.
if [ "${INSTALL_MAC_INNER:-0}" != 1 ] && [ "${INSTALL_MAC_NO_SELFRUN:-0}" != 1 ]; then
  b38=""; k38=$T/kitlike; rm -rf "$k38"; mkdir -p "$k38/scripts"
  cp -R "$repo/mac" "$k38/mac"; cp -R "$repo/deploy" "$k38/deploy"; cp "$repo/scripts/install-mac-test.sh" "$repo/scripts/install-mac-required-cells.txt" "$k38/scripts/"
  for f38 in mac-port-scan mac-port-run mac-port-test.sh; do [ ! -e "$repo/scripts/$f38" ] || cp "$repo/scripts/$f38" "$k38/scripts/"; done
  rm -f "$k38/scripts/release_manifest.py" "$k38/scripts/install-mac-fixture.py"
  k38o=$(env -u INSTALL_MAC_SH -u INSTALL_MAC_ONLY -u INSTALL_MAC_REQUIRE_SET -u INSTALL_MAC_REQUIRE_ALL -u MAC_SET_DIR -u MAC_SET_KEYFILE INSTALL_MAC_INNER=1 sh "$k38/scripts/install-mac-test.sh" 2>&1) && k38rc=0 || k38rc=$?
  printf '%s\n' "$k38o" | grep -q '^install-mac-test: [0-9][0-9]* passed, 0 failed, [0-9][0-9]* not run$' || b38="$b38 [the suite in a tree without the join set-up ended with no summary line (aborted?): rc=$k38rc $(printf '%s\n' "$k38o" | tail -1 | cut -c1-120)]"
  ! printf '%s\n' "$k38o" | grep -qi 'unbound variable' || b38="$b38 [an unbound variable: $(printf '%s\n' "$k38o" | grep -i 'unbound variable' | head -1 | cut -c1-120)]"
  [ "$k38rc" = 77 ] || b38="$b38 [exit $k38rc, want 77 (cells skipped: IT-M25, IT-M27, IT-M29)]"
  printf '%s\n' "$k38o" | grep -q '^skip IT-M25 / IT-M27 NOT RUN' || b38="$b38 [IT-M25 / IT-M27 were not skipped in a tree without the join set-up]"
  if [ -z "$b38" ]; then ok "IT-M38 the suite COMPLETES in a tree without the join set-up (the Mac kit's payload): its summary line, exit 77, IT-M25 / IT-M27 / IT-M29 skipped, no unbound variable"; else bad "IT-M38 the suite where the join set-up is absent" "$b38"; fi
fi

# IT-M46 THE MUTATION RUNNER'S SKIP RULE (mike MAC-MIKE-03 and -07): the runner judges a baseline or a mutant only on a run whose skips are IT-M29 and nothing else; a mandatory skip beside the expected FAIL, a skip record without a cell id and an exit 77 without a skip line are NOT judged (fake suites, scripts/install-mac-mutants-runner-test.py)
if [ -z "$py_t" ] || [ ! -f "$repo/scripts/install-mac-mutants-runner-test.py" ] || [ ! -f "$repo/scripts/install_mac_mutants.py" ]; then skipped "IT-M46 NOT RUN: the runner's skip-rule test needs a usable python3, scripts/install_mac_mutants.py and scripts/install-mac-mutants-runner-test.py"
else
  if "$py_t" -I "$repo/scripts/install-mac-mutants-runner-test.py" > "$T/runner-test.out" 2>&1; then ok "IT-M46 the mutation runner's skip rule: only a skip of IT-M29 is accepted, on every result (a mandatory skip beside the expected FAIL, a skip with no cell id and an exit 77 with no skip line are not judged)"; else bad "IT-M46 the mutation runner's skip rule" "$(grep WRONG "$T/runner-test.out" | head -3 | tr '\n' '|' | cut -c1-300)"; fi
fi

# IT-M30 the suite's verdict (mike MAC-MIKE-03): a suite with a mandatory cell NOT RUN is not a pass. finish_suite is run in a subshell with chosen counts; a required IT-M29 without a set is a FAILURE.
b30=""
# kai c968d57a / hue W1-284: the cases are run TWICE, once as they are and once with INSTALL_MAC_REQUIRE_ALL=1 in the environment of the whole run (the release check's own setting): every case sets or needs the variable for ITSELF, so the verdict cell holds under the release check too, and a skipped mandatory cell still fails closed there (the REQUIRE_ALL=1 / IT-M99 case)
it30_cases() {
fo=$( ( pass=5; fail=0; skip=0; skip_ids=""; finish_suite ) 2>&1 ) && frc=0 || frc=$?
{ [ "$frc" = 0 ] && ! printf '%s\n' "$fo" | grep -q '^NOT_RUN: '; } || b30="$b30 [no failure and no skip is not exit 0 without a NOT_RUN line: rc=$frc]"
fo=$( ( pass=5; fail=0; skip=2; skip_ids=" IT-M77 IT-M78"; finish_suite ) 2>&1 ) && frc=0 || frc=$?
{ [ "$frc" = 77 ] && [ "$(printf '%s\n' "$fo" | tail -1 | cut -c1-9)" = "NOT_RUN: " ]; } || b30="$b30 [one skipped cell must exit 77 with a LAST line 'NOT_RUN: <reason>' (ci.sh gate): rc=$frc last=$(printf '%s\n' "$fo" | tail -1 | cut -c1-60)]"
printf '%s\n' "$fo" | tail -1 | grep -qF '(IT-M77 IT-M78)' || b30="$b30 [the NOT_RUN line does not name the skipped cells (it carries their ids in parentheses): $(printf '%s\n' "$fo" | tail -1 | cut -c1-80)]"
fo=$( ( pass=5; fail=1; skip=1; skip_ids=" IT-M29"; finish_suite ) 2>&1 ) && frc=0 || frc=$?
{ [ "$frc" = 1 ] && ! printf '%s\n' "$fo" | grep -q '^NOT_RUN: '; } || b30="$b30 [a failure is exit 1 and never NOT_RUN: rc=$frc]"
fo=$( ( INSTALL_MAC_REQUIRE_SET=1; pass=0; fail=0; skip=0; skip_ids=""; not_run_29 "no set was given"; finish_suite ) 2>&1 ) && frc=0 || frc=$?
{ [ "$frc" = 1 ] && printf '%s\n' "$fo" | grep -q '^FAIL IT-M29' && ! printf '%s\n' "$fo" | grep -q '^NOT_RUN: '; } || b30="$b30 [INSTALL_MAC_REQUIRE_SET=1 without a set must FAIL IT-M29 (exit 1), not skip it: rc=$frc]"
fo=$( ( INSTALL_MAC_REQUIRE_ALL=1; pass=2; fail=0; skip=0; skip_ids=""; skipped "IT-M99 NOT RUN: no python3"; finish_suite ) 2>&1 ) && frc=0 || frc=$?
{ [ "$frc" = 1 ] && printf '%s\n' "$fo" | grep -q '^FAIL IT-M99' && ! printf '%s\n' "$fo" | grep -q '^NOT_RUN: ' && ! printf '%s\n' "$fo" | grep -q '^skip '; } || b30="$b30 [INSTALL_MAC_REQUIRE_ALL=1: ANY skipped cell must be a FAIL (exit 1), never a skip or a NOT_RUN: rc=$frc]"
fo=$( ( INSTALL_MAC_REQUIRE_ALL=0; pass=2; fail=0; skip=0; skip_ids=""; skipped "IT-M99 NOT RUN: no python3"; finish_suite ) 2>&1 ) && frc=0 || frc=$?
{ [ "$frc" = 77 ] && printf '%s\n' "$fo" | grep -q '^skip IT-M99' && [ "$(printf '%s\n' "$fo" | tail -1 | cut -c1-9)" = "NOT_RUN: " ]; } || b30="$b30 [without REQUIRE_ALL a skipped cell stays a skip and the suite exits 77: rc=$frc]"
fo=$( env -u INSTALL_MAC_SH -u INSTALL_MAC_NO_SELFRUN -u INSTALL_MAC_ONLY INSTALL_MAC_REQUIRE_ALL=1 INSTALL_MAC_INNER=1 sh "$0" 2>&1 ) && frc=0 || frc=$?
{ [ "$frc" = 1 ] && printf '%s\n' "$fo" | grep -q '^FAIL IT-M00 INSTALL_MAC_REQUIRE_ALL=1 with INSTALL_MAC_INNER=1'; } || b30="$b30 [INSTALL_MAC_REQUIRE_ALL=1 with INSTALL_MAC_INNER=1 (a run that omits IT-M38) must be refused before anything runs: rc=$frc]"
fo=$( env -u INSTALL_MAC_SH -u INSTALL_MAC_INNER -u INSTALL_MAC_ONLY INSTALL_MAC_REQUIRE_ALL=1 INSTALL_MAC_NO_SELFRUN=1 sh "$0" 2>&1 ) && frc=0 || frc=$?
{ [ "$frc" = 1 ] && printf '%s\n' "$fo" | grep -q '^FAIL IT-M00 INSTALL_MAC_REQUIRE_ALL=1 with INSTALL_MAC_NO_SELFRUN=1'; } || b30="$b30 [INSTALL_MAC_REQUIRE_ALL=1 with INSTALL_MAC_NO_SELFRUN=1 (a run that omits IT-M38) must be refused before anything runs: rc=$frc]"
# THE REQUIRED-CELLS LIST (mike MAC-MIKE-04), in a directory of its own with a list of two cells: under INSTALL_MAC_REQUIRE_ALL=1 a LISTED cell that did not print ok is a FAILURE; a cell that printed ok and is NOT listed is a FAILURE in any mode; a missing list is a FAILURE
rq=$T/rq; mkdir -p "$rq"; printf '# list\nIT-M01\nIT-M29\n' > "$rq/install-mac-required-cells.txt"
fo=$( ( here=$rq; INSTALL_MAC_REQUIRE_ALL=1; pass=1; fail=0; skip=0; skip_ids=""; ok_ids=" IT-M01"; finish_suite final ) 2>&1 ) && frc=0 || frc=$?
{ [ "$frc" = 1 ] && printf '%s\n' "$fo" | grep -q '^FAIL IT-M29' && printf '%s\n' "$fo" | grep -q 'the required cell IT-M29 did not print ok'; } || b30="$b30 [INSTALL_MAC_REQUIRE_ALL=1: a LISTED cell with no ok (IT-M29) must be a FAIL: rc=$frc]"
fo=$( ( here=$rq; INSTALL_MAC_REQUIRE_ALL=0; pass=2; fail=0; skip=0; skip_ids=""; ok_ids=" IT-M01 IT-M99"; finish_suite final ) 2>&1 ) && frc=0 || frc=$?
{ [ "$frc" = 1 ] && printf '%s\n' "$fo" | grep -q '^FAIL IT-M99' && printf '%s\n' "$fo" | grep -q 'is not in scripts/install-mac-required-cells.txt'; } || b30="$b30 [a cell that printed ok and is NOT in the required list (IT-M99) must be a FAIL in any mode: rc=$frc]"
fo=$( ( here=$rq; INSTALL_MAC_REQUIRE_ALL=0; pass=1; fail=0; skip=0; skip_ids=""; ok_ids=" IT-M01"; finish_suite final ) 2>&1 ) && frc=0 || frc=$?
[ "$frc" = 0 ] || b30="$b30 [without INSTALL_MAC_REQUIRE_ALL a listed cell that did not run (IT-M29, no set) is no failure here (it is a skip elsewhere): rc=$frc $(printf '%s\n' "$fo" | head -2 | tr '\n' '|' | cut -c1-100)]"
fo=$( ( here=$T/no-such-dir; INSTALL_MAC_REQUIRE_ALL=0; pass=1; fail=0; skip=0; skip_ids=""; ok_ids=" IT-M01"; finish_suite final ) 2>&1 ) && frc=0 || frc=$?
{ [ "$frc" = 1 ] && printf '%s\n' "$fo" | grep -q '^FAIL IT-M00'; } || b30="$b30 [a missing required-cells list must be a FAIL: rc=$frc]"
fo=$( env -u INSTALL_MAC_NO_SELFRUN -u INSTALL_MAC_INNER -u INSTALL_MAC_ONLY INSTALL_MAC_REQUIRE_ALL=1 INSTALL_MAC_SH=/nonexistent sh "$0" 2>&1 ) && frc=0 || frc=$?
{ [ "$frc" = 1 ] && printf '%s\n' "$fo" | grep -q '^FAIL IT-M00 INSTALL_MAC_REQUIRE_ALL=1 with INSTALL_MAC_SH set'; } || b30="$b30 [INSTALL_MAC_REQUIRE_ALL=1 with INSTALL_MAC_SH set (another installer) must be refused before anything runs (exit 1): rc=$frc $(printf '%s\n' "$fo" | head -1 | cut -c1-80)]"
fo=$( ( INSTALL_MAC_REQUIRE_ALL=0; INSTALL_MAC_REQUIRE_SET=0; pass=0; fail=0; skip=0; skip_ids=""; not_run_29 "no set was given"; finish_suite ) 2>&1 ) && frc=0 || frc=$?
{ [ "$frc" = 77 ] && printf '%s\n' "$fo" | grep -q '^skip IT-M29 NOT RUN' && [ "$(printf '%s\n' "$fo" | tail -1 | cut -c1-9)" = "NOT_RUN: " ]; } || b30="$b30 [without the requirement a missing set is a skip and the suite exits 77: rc=$frc]"
}
it30_saved=${INSTALL_MAC_REQUIRE_ALL-unset}
for it30_o in 0 1; do INSTALL_MAC_REQUIRE_ALL=$it30_o; export INSTALL_MAC_REQUIRE_ALL; it30_cases; done
if [ "$it30_saved" = unset ]; then unset INSTALL_MAC_REQUIRE_ALL; else INSTALL_MAC_REQUIRE_ALL=$it30_saved; export INSTALL_MAC_REQUIRE_ALL; fi
if [ -z "$b30" ]; then ok "IT-M30 the suite's verdict: no failure and no skip exits 0; a skipped mandatory cell exits 77 with the last line 'NOT_RUN: <reason>' naming it (scripts/ci.sh reads that as NOT_RUN, a failure under --strict); a failure exits 1; INSTALL_MAC_REQUIRE_SET=1 turns a missing IT-M29 set into a FAIL"; else bad "IT-M30 the suite's verdict" "$b30"; fi

finish_suite final
