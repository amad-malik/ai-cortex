#!/bin/sh
# launchd-test.sh: the tests of M-1c, the backup as macOS launchd agents (bob; cox row f60b3c86, kai rows c9dc7843 and e5e9e428).
# Plan: mvp/pieces/12-backup-dump-MAC-M1c-PLAN-r0.md.
#
#   scripts/launchd-test.sh [--shell sh|posix] [ML-nn ...]
#
#   --shell sh      run the tools under `sh` (default; /bin/sh is bash in POSIX mode on this VM)
#   --shell posix   run the tools under `bash --posix`
#   no ids          run every cell; with ids, only those
#
# NOTHING here is real: there is no launchctl on this VM and none is ever called. A FAKE `launchctl` first in PATH records every argument vector
# (one line per call, `launchctl <args>`), keeps a registry of loaded labels, can be told to fail (FAKE_LC_FAIL=<subcommands>, FAKE_LC_FAIL_RC=<n>,
# or FAKE_LC_FAIL_PLIST=<text in one plist path>: only that plist's bootstrap fails, or FAKE_LC_FAIL_AFTER=<subcommand>:<n>: that subcommand fails from its n-th call on)
# and REFUSES the deprecated `load` and `unload`. The fake is the author's reading of launchctl's documented behaviour, NOT a recording of a Mac:
# what only a Mac can answer is listed in the plan (section 8) and stays NOT RUN until M-6. No systemd tool of any kind is called either.
# Run it through cv2-admit: `cv2-admit scripts/launchd-test.sh`. Each cell builds its own directory under target/ml-test; the run removes those
# directories, and only those, by their exact names at the end.
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
lb=$here/launchd-backup
lr=$here/launchd-run
tpl=$here/../deploy/launchd
units=$here/../deploy/systemd
base=${CORTEX_ML_TMP:-"$here/../target/ml-test"}
l_dump=ai.kaidera.cortex-dump
l_stat=ai.kaidera.cortex-dump-status

sut=sh
want=""
while [ $# -gt 0 ]; do
  case $1 in
    --shell)
      shift
      case ${1:-} in
        sh) sut=sh ;;
        posix) sut="bash --posix" ;;
        *) echo "launchd-test: --shell takes sh or posix" >&2; exit 2 ;;
      esac
      ;;
    ML-*) want="$want $1" ;;
    -h | --help) sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "launchd-test: unknown argument $1" >&2; exit 2 ;;
  esac
  shift
done
# the application's own variables never enter a cell from the caller's shell
unset CORTEX_DUMP_STATUS PGPASSWORD
# a fixed locale: the order of directory names, sort and character classes must not depend on the caller's (sam M1C-SAM-04); the cells that compare
# listings also normalise them with lsa (a sorted set), so neither choice alone carries the check
LC_ALL=C
export LC_ALL
mkdir -p "$base"
base=$(CDPATH= cd -P -- "$base" && pwd -P)
uid=$(id -u)
dom=gui/$uid
made=""

# ---- helpers ---------------------------------------------------------------------------------------------

need() { # need <message> <test args...>: a failed check names itself and fails the cell
  _m=$1
  shift
  if "$@"; then return 0; fi
  printf '    CHECK FAILED: %s\n' "$_m" >&2
  return 1
}
run() { # run <name> <command...>: status in $rc, output in $T/<name>.out and .err
  _n=$1
  shift
  rc=0
  "$@" > "$T/$_n.out" 2> "$T/$_n.err" || rc=$?
}
new_test() {
  T=$(mktemp -d "$base/t.XXXXXX")
  made="$made $T"
  mkdir -p "$T/bin" "$T/agentbin" "$T/h" "$T/agents"
  ap="$T/agentbin:/usr/bin:/bin" # the PATH the agents are given (--path-value): the prerequisite floor must hold HERE, not on the caller's PATH
  : > "$T/lc.log"
  : > "$T/lc.reg"
  fake_launchctl
  fake_tools
  mk_home
}
# Stand-ins for the four programs cortex-dump itself requires (the prerequisite check of `launchd-backup install` looks for them by name in the
# PATH the AGENTS will get), so a cell does not depend on what this host has installed; ML-27 hides them one at a time.
req_tools="podman flock timeout sha256sum"
fake_tools() {
  for _t in $req_tools; do
    printf '#!/bin/sh\nexit 0\n' > "$T/agentbin/$_t"
    chmod 0755 "$T/agentbin/$_t"
  done
}
lsa() { ls -A "$1" | LC_ALL=C sort | tr '\n' ' '; } # the names in a directory, sorted in a fixed locale, on one line
# mkpath <name>: a CALLER PATH directory with only the basic utilities (find among them: the tool reads permissions with it) and the fake launchctl (none of the four required programs)
mkpath() {
  _p=$T/path-$1
  mkdir -p "$_p"
  for _t in sh bash env id python3 cat sed grep mv cp mkdir chmod stat cmp tr ls head dirname rm mktemp printf sort wc date awk sleep find; do
    _x=$(command -v "$_t" 2> /dev/null || true)
    [ -n "$_x" ] && ln -sf "$_x" "$_p/$_t"
  done
  ln -sf "$T/bin/launchctl" "$_p/launchctl"
  echo "$_p"
}
cleanup_made() { # remove the directories THIS run made, by their exact names, shape-checked
  for _d in $made; do
    case $_d in "$base"/t.??????) rm -rf -- "$_d" ;; esac # s5-never: ok the exact directories this run made, shape-checked
  done
}
pl() { # pl <plist> <python expression over d>: one value of a plist, parsed with plistlib
  python3 -I -c 'import plistlib,sys; d=plistlib.load(open(sys.argv[1],"rb")); print(eval(sys.argv[2]))' "$1" "$2"
}
calls() { cat "$T/lc.log"; }
ncalls() { _c=$(grep -c "$1" "$T/lc.log" || true); echo "$_c"; }

# The fake launchctl (see the header). Registry lines are `<domain>/<label>`.
fake_launchctl() {
  cat > "$T/bin/launchctl" <<'FAKE'
#!/bin/sh
# (a test double that is itself a shell script: it drops the functions its environment brings, so that ML-44's hostile ones test the tool, not this stand-in)
unset -f unset command env sed 2> /dev/null || :
for _fn in $(command env | command sed -n 's/^BASH_FUNC_\(.*\)%%=.*/\1/p'); do unset -f "$_fn" 2> /dev/null || :; done
set -u
log=${FAKE_LC_LOG:?}
reg=${FAKE_LC_REG:?}
printf '%s\n' "launchctl $*" >> "$log"
sub=${1:-}
failing() { case ",${FAKE_LC_FAIL:-}," in *",$1,"*) return 0 ;; esac; return 1; }
loaded() { grep -qx -- "$1" "$reg" 2> /dev/null; }
label_of() { python3 -I -c 'import plistlib,sys; print(plistlib.load(open(sys.argv[1],"rb"))["Label"])' "$1" 2> /dev/null; }
if failing "$sub"; then
  echo "fake launchctl: told to fail on $sub" >&2
  exit "${FAKE_LC_FAIL_RC:-5}"
fi
case ${FAKE_LC_FAIL_AFTER:-} in
  "$sub":[0-9]*)
    n=$(cat "$reg.count.$sub" 2> /dev/null || echo 0)
    n=$((n + 1))
    echo "$n" > "$reg.count.$sub"
    if [ "$n" -ge "${FAKE_LC_FAIL_AFTER#*:}" ]; then
      echo "fake launchctl: told to fail on $sub from call ${FAKE_LC_FAIL_AFTER#*:} on (this is call $n)" >&2
      exit "${FAKE_LC_FAIL_RC:-5}"
    fi ;;
esac
case $sub in
  load | unload)
    echo "launchctl: $sub is deprecated and not supported by this fake: use bootstrap or bootout" >&2
    exit 64 ;;
  bootstrap)
    d=${2:-}
    p=${3:-}
    case $d in gui/[0-9]* | user/[0-9]*) ;; *) echo "fake launchctl: bad domain target $d" >&2; exit 125 ;; esac
    [ -f "$p" ] || { echo "fake launchctl: no such plist $p" >&2; exit 2; }
    case $p in
      *"${FAKE_LC_FAIL_PLIST:-@@none@@}"*) echo "Bootstrap failed: ${FAKE_LC_FAIL_RC:-5}: told to fail on this plist" >&2; exit "${FAKE_LC_FAIL_RC:-5}" ;;
    esac
    l=$(label_of "$p")
    [ -n "$l" ] || { echo "fake launchctl: the plist has no Label" >&2; exit 78; }
    if loaded "$d/$l"; then echo "Bootstrap failed: 5: Input/output error" >&2; exit 5; fi
    echo "$d/$l" >> "$reg"
    exit 0 ;;
  bootout)
    t=${2:-}
    case $t in gui/[0-9]*/?* | user/[0-9]*/?*) ;; *) echo "fake launchctl: bootout wants <domain>/<label>, got $t" >&2; exit 64 ;; esac
    if ! loaded "$t"; then echo "Boot-out failed: 3: No such process" >&2; exit 3; fi
    grep -vx -- "$t" "$reg" > "$reg.new" || true
    mv -f "$reg.new" "$reg"
    exit 0 ;;
  print)
    loaded "${2:-}" || { echo "Could not find service in domain" >&2; exit 113; }
    printf '%s\n' "${FAKE_LC_PRINT_TEXT:-state = running}"
    exit 0 ;;
  enable | disable)
    exit 0 ;;
  kickstart)
    t=${2:-}
    [ "$t" = "-k" ] && t=${3:-}
    loaded "$t" || exit 113
    exit 0 ;;
  list)
    cat "$reg"
    exit 0 ;;
  *)
    echo "fake launchctl: unexpected subcommand $sub" >&2
    exit 64 ;;
esac
FAKE
  chmod 0755 "$T/bin/launchctl"
}

# A fake installed home: bin/ with the four programs the agents run (the real scripts, copied), the data directories, a logs path.
mk_home() {
  home=$T/home
  mkdir -p "$home/bin" "$home/dumps" "$home/dump-status"
  for f in cortex-dump cortex-dump-status cortex-dump-alert; do
    [ -f "$here/$f" ] && cp -p "$here/$f" "$home/bin/$f"
  done
  if [ -f "$lr" ]; then cp -p "$lr" "$home/bin/launchd-run"; fi
}
# lbrun <name> <launchd-backup args...>: the install tool against the fake launchctl, with HOME inside the test directory
lbrun() {
  _n=$1
  shift
  if [ "${1:-}" = install ]; then
    case " $* " in *" --path-value "*) ;; *) set -- "$@" --path-value "$ap" ;; esac
  fi
  run "$_n" env PATH="$T/bin:$PATH" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" HOME="$T/h" $sut "$lb" "$@"
}
inst() { lbrun "${1:-i}" install --home "$home" --db-container cv2-db-one --agents-dir "$T/agents"; }

# ---- the templates and what they render to ---------------------------------------------------------------

ml_01() { # the templates exist, mode 644, and only the documented placeholders appear in them
  new_test
  for t in "$l_dump" "$l_stat"; do
    f=$tpl/$t.plist.in
    need "$f exists, mode 644" test "$(stat -c %a "$f" 2> /dev/null)" = 644 || return 1
  done
  extra=$(grep -ho '@[A-Z_]*@' "$tpl"/*.plist.in 2> /dev/null | sort -u | grep -v -x -e '@CORTEX_HOME@' -e '@DB_CONTAINER@' -e '@LOG_DIR@' -e '@PATH_VALUE@' || true)
  need "only the four documented placeholders exist in the templates ($extra)" test -z "$extra" || return 1
}
ml_02() { # the rendered plists parse and say exactly what the Linux units say, in launchd's words
  new_test
  inst
  need "install succeeds (got $rc): $(tail -n 2 "$T/i.err" | tr '\n' ' ')" test "$rc" -eq 0 || return 1
  d=$T/agents/$l_dump.plist
  s=$T/agents/$l_stat.plist
  need "both plists exist, mode 644" sh -c "[ \"\$(stat -c %a '$d')\" = 644 ] && [ \"\$(stat -c %a '$s')\" = 644 ]" || return 1
  need "dump Label" test "$(pl "$d" 'd["Label"]')" = "$l_dump" || return 1
  need "the agent PATH is the --path-value given" test "$(pl "$d" 'd["EnvironmentVariables"]["PATH"]')" = "$ap" || return 1
  need "status Label" test "$(pl "$s" 'd["Label"]')" = "$l_stat" || return 1
  need "the dump agent hands the wrapper its unit and home, and no --clear-on-success" test "$(pl "$d" 'str(d["ProgramArguments"][1:5]) + str("--clear-on-success" in d["ProgramArguments"])')" = "['--unit', 'cortex-dump.service', '--home', '$home']False" || return 1
  need "the check agent hands the wrapper its unit and home, and --clear-on-success" test "$(pl "$s" 'd["ProgramArguments"][1:6]')" = "['--unit', 'cortex-dump-status.service', '--home', '$home', '--clear-on-success']" || return 1
  need "dump runs at 02:30 (Hour 2, Minute 30)" test "$(pl "$d" 'd["StartCalendarInterval"]')" = "{'Hour': 2, 'Minute': 30}" || return 1
  need "the check runs at minute 17 of EVERY hour (no Hour key)" test "$(pl "$s" 'd["StartCalendarInterval"]')" = "{'Minute': 17}" || return 1
  need "the dump does not run at load" test "$(pl "$d" 'd["RunAtLoad"]')" = False || return 1
  need "the check runs at load (so it says a stale dump at login)" test "$(pl "$s" 'd["RunAtLoad"]')" = True || return 1
  for p in "$d" "$s"; do
    need "$p: Nice 10, LowPriorityIO, ProcessType Background" sh -c "[ \"\$(python3 -I -c 'import plistlib,sys; d=plistlib.load(open(sys.argv[1],\"rb\")); print(d[\"Nice\"], d[\"LowPriorityIO\"], d[\"ProcessType\"])' '$p')\" = '10 True Background' ]" || return 1
    need "$p: logs under $home/logs" sh -c "[ \"\$(python3 -I -c 'import plistlib,sys; d=plistlib.load(open(sys.argv[1],\"rb\")); print(d[\"StandardOutPath\"].startswith(sys.argv[2]) and d[\"StandardErrorPath\"].startswith(sys.argv[2]))' '$p' '$home/logs/')\" = True ]" || return 1
    need "$p: the environment is exactly PATH" test "$(pl "$p" 'sorted(d["EnvironmentVariables"].keys())')" = "['PATH']" || return 1
    need "$p: and that PATH is the --path-value given" test "$(pl "$p" 'd["EnvironmentVariables"]["PATH"]')" = "$ap" || return 1
    need "$p: every program argument that is a path is absolute (the wrapper first)" test "$(pl "$p" 'd["ProgramArguments"][0]')" = "$home/bin/launchd-run" || return 1
  done
}
ml_03() { # nothing that must not be there: no ~, no leftover placeholder, no secret words, no shell in the arguments
  new_test
  inst
  need "install succeeds (got $rc)" test "$rc" -eq 0 || return 1
  need "no ~ and no leftover @...@ in either plist" test -z "$(grep -l -e '~' -e '@[A-Z_]*@' "$T"/agents/*.plist || true)" || return 1
  need "no password, host or port in either plist" test -z "$(grep -iE 'PGPASSWORD|password|localhost|127\.0\.0\.1|PGHOST|--host|5432' "$T"/agents/*.plist || true)" || return 1
  for p in "$T/agents/$l_dump.plist" "$T/agents/$l_stat.plist"; do
    need "$p: no shell as the program (no /bin/sh, no -c)" test "$(pl "$p" '[a for a in d["ProgramArguments"] if a in ("/bin/sh","/bin/bash","-c","sh","bash")]')" = "[]" || return 1
  done
}
ml_22() { # drift guard: after `--` the agent runs the SAME arguments the Linux unit's ExecStart has
  new_test
  inst
  need "install succeeds (got $rc)" test "$rc" -eq 0 || return 1
  for pair in "$l_dump:cortex-dump.service" "$l_stat:cortex-dump-status.service"; do
    lab=${pair%%:*}
    unit=${pair#*:}
    want_args=$(sed -n 's/^ExecStart=//p' "$units/$unit" | sed -e "s|@CORTEX_HOME@|$home|g" -e 's|@DB_CONTAINER@|cv2-db-one|g' | tr ' ' '\n')
    got_args=$(pl "$T/agents/$lab.plist" '"\n".join(d["ProgramArguments"][d["ProgramArguments"].index("--")+1:])')
    need "$lab runs the arguments of $unit's ExecStart" test "$got_args" = "$want_args" || { printf 'want:\n%s\ngot:\n%s\n' "$want_args" "$got_args" >&2; return 1; }
  done
}

# ---- install, reload, uninstall against the fake launchctl -------------------------------------------------

ml_04() { # a fresh install: print, enable, bootstrap for each label, in that order, in the gui domain; never load
  new_test
  inst
  need "install succeeds (got $rc): $(tail -n 2 "$T/i.err" | tr '\n' ' ')" test "$rc" -eq 0 || return 1
  want_calls="launchctl print $dom/$l_dump
launchctl enable $dom/$l_dump
launchctl bootstrap $dom $T/agents/$l_dump.plist
launchctl print $dom/$l_stat
launchctl enable $dom/$l_stat
launchctl bootstrap $dom $T/agents/$l_stat.plist"
  need "the exact launchctl calls, in order" test "$(calls)" = "$want_calls" || { printf 'got:\n%s\n' "$(calls)" >&2; return 1; }
  need "both labels are loaded in the registry" test "$(sort "$T/lc.reg" | tr '\n' ' ')" = "$dom/$l_dump $dom/$l_stat " || return 1
  need "never load, unload or the legacy flags" test -z "$(grep -E 'launchctl (load|unload)' "$T/lc.log" || true)" || return 1
}
ml_05() { # a second install changes nothing: only print calls, the bytes and the modification times are left alone
  new_test
  inst
  need "first install succeeds (got $rc)" test "$rc" -eq 0 || return 1
  sums=$(cksum "$T"/agents/*.plist)
  times=$(stat -c '%n %Y' "$T"/agents/*.plist)
  : > "$T/lc.log"
  sleep 1
  inst i2
  need "second install succeeds (got $rc)" test "$rc" -eq 0 || return 1
  need "only print calls: $(calls | tr '\n' '|')" test -z "$(grep -v '^launchctl print ' "$T/lc.log" || true)" || return 1
  need "the same bytes" test "$(cksum "$T"/agents/*.plist)" = "$sums" || return 1
  need "the same modification times (not rewritten)" test "$(stat -c '%n %Y' "$T"/agents/*.plist)" = "$times" || return 1
  need "the output says nothing changed" grep -qi 'nothing changed\|unchanged' "$T/i2.out" || return 1
  need "no stray temporary file is left in the agents directory ($(lsa "$T/agents"))" test "$(lsa "$T/agents")" = "$(printf '%s\n' "$l_dump.plist" "$l_stat.plist" | LC_ALL=C sort | tr '\n' ' ')" || return 1
}
ml_06() { # a changed input reloads exactly the agent it changes: print, bootout, (enable), bootstrap for the dump agent; only print for the other
  new_test
  inst
  need "first install succeeds (got $rc)" test "$rc" -eq 0 || return 1
  : > "$T/lc.log"
  lbrun i2 install --home "$home" --db-container cv2-db-two --agents-dir "$T/agents"
  need "reinstall with another container name succeeds (got $rc): $(tail -n 2 "$T/i2.err" | tr '\n' ' ')" test "$rc" -eq 0 || return 1
  need "the dump agent was booted out" grep -qx "launchctl bootout $dom/$l_dump" "$T/lc.log" || return 1
  need "and bootstrapped again from the new file" grep -qx "launchctl bootstrap $dom $T/agents/$l_dump.plist" "$T/lc.log" || return 1
  need "the status agent was only looked at (print), never booted out or bootstrapped" test -z "$(grep "$l_stat" "$T/lc.log" | grep -v '^launchctl print ' || true)" || return 1
  need "the new container name is in the dump plist" grep -q 'cv2-db-two' "$T/agents/$l_dump.plist" || return 1
  need "bootout comes before bootstrap" test "$(grep -n "bootout $dom/$l_dump" "$T/lc.log" | cut -d: -f1)" -lt "$(grep -n "bootstrap $dom $T/agents/$l_dump.plist" "$T/lc.log" | cut -d: -f1)" || return 1
}
ml_07() { # uninstall: bootout for its two labels, removes only its two files; a foreign file survives; a second uninstall is clean
  new_test
  inst
  need "install succeeds (got $rc)" test "$rc" -eq 0 || return 1
  printf 'foreign\n' > "$T/agents/com.example.other.plist"
  : > "$T/lc.log"
  lbrun u uninstall --agents-dir "$T/agents"
  need "uninstall succeeds (got $rc): $(tail -n 2 "$T/u.err" | tr '\n' ' ')" test "$rc" -eq 0 || return 1
  need "bootout for the dump label" grep -qx "launchctl bootout $dom/$l_dump" "$T/lc.log" || return 1
  need "bootout for the status label" grep -qx "launchctl bootout $dom/$l_stat" "$T/lc.log" || return 1
  need "nothing is loaded afterwards" test ! -s "$T/lc.reg" || return 1
  need "its two files are gone" test ! -e "$T/agents/$l_dump.plist" -a ! -e "$T/agents/$l_stat.plist" || return 1
  need "the foreign file is untouched" test "$(cat "$T/agents/com.example.other.plist")" = foreign || return 1
  : > "$T/lc.log"
  lbrun u2 uninstall --agents-dir "$T/agents"
  need "a second uninstall succeeds (got $rc)" test "$rc" -eq 0 || return 1
  need "and makes only print calls" test -z "$(grep -v '^launchctl print ' "$T/lc.log" || true)" || return 1
  printf '<plist><dict><key>Label</key><string>com.example.mine</string></dict></plist>\n' > "$T/agents/$l_dump.plist"
  lbrun u3 uninstall --agents-dir "$T/agents"
  need "a file at our name that carries another label survives uninstall (got $rc)" test "$rc" -eq 0 -a -f "$T/agents/$l_dump.plist" || return 1
}
ml_08() { # the control: the fake refuses the deprecated load and unload, so a tool that used them would fail ML-04
  new_test
  run c1 env PATH="$T/bin:$PATH" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" launchctl load "$T/x.plist"
  need "load is refused (got $rc)" test "$rc" -ne 0 || return 1
  run c2 env PATH="$T/bin:$PATH" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" launchctl unload "$T/x.plist"
  need "unload is refused (got $rc)" test "$rc" -ne 0 || return 1
  run c3 env PATH="$T/bin:$PATH" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" launchctl bootout "$dom/nothing.loaded"
  need "bootout of an unloaded label exits non-zero (got $rc)" test "$rc" -ne 0 || return 1
  run c4 env PATH="$T/bin:$PATH" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" FAKE_LC_FAIL=print FAKE_LC_FAIL_RC=7 launchctl print "$dom/x"
  need "the fake can be told to fail, with the exit code asked for (got $rc)" test "$rc" -eq 7 || return 1
}
ml_09() { # launchctl fails on bootstrap: a plain problem/fix message, a non-zero exit, and nothing half-installed
  new_test
  run f env PATH="$T/bin:$PATH" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" FAKE_LC_FAIL=bootstrap FAKE_LC_FAIL_RC=5 HOME="$T/h" $sut "$lb" install --path-value "$ap" --home "$home" --db-container cortex-other --agents-dir "$T/agents2"
  need "the install fails (got $rc)" test "$rc" -ne 0 || return 1
  need "the message names a problem and a fix" sh -c "grep -q 'problem:' '$T/f.err' && grep -q 'fix:' '$T/f.err'" || return 1
  need "nothing at all is left in the new agents directory (not even a temporary file)" test -z "$(ls -A "$T/agents2" 2> /dev/null || true)" || return 1
  need "nothing is loaded" test ! -s "$T/lc.reg" || return 1
  # a reinstall whose reload fails: the OLD bytes are put back, the old agent stays unloaded (it was booted out), the other agent is untouched
  inst g1
  need "a good install first succeeds (got $rc)" test "$rc" -eq 0 || return 1
  sums=$(cksum "$T"/agents/*.plist)
  run g2 env PATH="$T/bin:$PATH" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" FAKE_LC_FAIL=bootstrap FAKE_LC_FAIL_RC=5 HOME="$T/h" $sut "$lb" install --path-value "$ap" --home "$home" --db-container cv2-db-two --agents-dir "$T/agents"
  need "the reinstall fails (got $rc)" test "$rc" -ne 0 || return 1
  need "the old bytes are back in both files" test "$(cksum "$T"/agents/*.plist)" = "$sums" || return 1
  need "and the dump plist still names the old container" grep -q 'cv2-db-one' "$T/agents/$l_dump.plist" || return 1
  need "the old dump agent is unloaded, the check agent still loaded" test "$(cat "$T/lc.reg")" = "$dom/$l_stat" || return 1
  need "no temporary file is left ($(lsa "$T/agents"))" test "$(lsa "$T/agents")" = "$(printf '%s\n' "$l_dump.plist" "$l_stat.plist" | LC_ALL=C sort | tr '\n' ' ')" || return 1
  # M1C-SAM-01: the same bytes, the label unloaded (a logout, a manual bootout), then a failing bootstrap: files that were already there stay
  lbrun h1 install --home "$home" --db-container cv2-db-one --agents-dir "$T/agents3"
  need "an install into a third directory succeeds (got $rc)" test "$rc" -eq 0 || return 1
  sums3=$(cksum "$T"/agents3/*.plist)
  : > "$T/lc.reg"
  run h2 env PATH="$T/bin:$PATH" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" FAKE_LC_FAIL=bootstrap FAKE_LC_FAIL_RC=5 HOME="$T/h" $sut "$lb" install --path-value "$ap" --home "$home" --db-container cv2-db-one --agents-dir "$T/agents3"
  need "the reinstall of unchanged, unloaded agents fails when bootstrap fails (got $rc)" test "$rc" -ne 0 || return 1
  need "BOTH files that were already there are untouched ($(lsa "$T/agents3"))" test "$(cksum "$T"/agents3/*.plist)" = "$sums3" || return 1
  need "nothing else was left ($(lsa "$T/agents3"))" test "$(lsa "$T/agents3")" = "$(printf '%s\n' "$l_dump.plist" "$l_stat.plist" | LC_ALL=C sort | tr '\n' ' ')" || return 1
  need "the message says the files are as they were before this run" grep -q 'as they were before this run' "$T/h2.err" || return 1
  # (part 2's message, checked last so that parts 2 and 3 are each exercised) the old dump agent could not be reloaded: say so
  need "the message of part 2 says the old dump agent could not be reloaded (the bootstrap fails for every plist there)" grep -q 'could not be reloaded' "$T/g2.err" || return 1
}
ml_23() { # status is read-only (print calls only), says which agent is missing, and exits 0 only when both are loaded
  new_test
  inst
  need "install succeeds (got $rc)" test "$rc" -eq 0 || return 1
  : > "$T/lc.log"
  lbrun s status --agents-dir "$T/agents"
  need "both loaded: exit 0 (got $rc)" test "$rc" -eq 0 || return 1
  need "only print calls" test -z "$(grep -v '^launchctl print ' "$T/lc.log" || true)" || return 1
  grep -vx "$dom/$l_stat" "$T/lc.reg" > "$T/lc.reg.new" || true
  mv -f "$T/lc.reg.new" "$T/lc.reg"
  lbrun s2 status --agents-dir "$T/agents"
  need "one agent not loaded: exit 1 (got $rc)" test "$rc" -eq 1 || return 1
  need "and it names the one, as not loaded" grep -q "^$l_stat: file yes, loaded no\$" "$T/s2.out" || return 1
  inst s3
  rm -f "$T/agents/$l_dump.plist"
  lbrun s4 status --agents-dir "$T/agents"
  need "a loaded agent whose file is gone: exit 1 (got $rc)" test "$rc" -eq 1 || return 1
  need "and it says the file is missing" grep -q "^$l_dump: file no, loaded yes\$" "$T/s4.out" || return 1
}
ml_24() { # the tool reads launchctl's EXIT CODES only, never its text: the same answers whatever print says
  new_test
  inst
  need "install succeeds (got $rc)" test "$rc" -eq 0 || return 1
  : > "$T/lc.log"
  run t env PATH="$T/bin:$PATH" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" FAKE_LC_PRINT_TEXT="state = exited (not running) Could not find service 113" HOME="$T/h" $sut "$lb" install --path-value "$ap" --home "$home" --db-container cv2-db-one --agents-dir "$T/agents"
  need "a second install with odd print text still changes nothing (got $rc)" test "$rc" -eq 0 -a -z "$(grep -v '^launchctl print ' "$T/lc.log" || true)" || return 1
}

# ---- refusals: a plain problem and fix, exit 2, nothing changed -------------------------------------------

refused() { # refused <name> <expected substring of the message> <install args...>
  _n=$1
  _w=$2
  shift 2
  : > "$T/lc.log"
  lbrun "$_n" install "$@"
  need "$_n: refused with exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  need "$_n: the message has problem: and fix:" sh -c "grep -q 'problem:' '$T/$_n.err' && grep -q 'fix:' '$T/$_n.err'" || return 1
  need "$_n: the message mentions $_w" grep -qi -- "$_w" "$T/$_n.err" || return 1
  need "$_n: launchctl was never asked to change anything" test -z "$(grep -vE '^launchctl print ' "$T/lc.log" || true)" || return 1
  need "$_n: no plist was written" test -z "$(ls "$T/agents" 2> /dev/null || true)" || return 1
}
ml_10() { # bad inputs
  new_test
  refused r1 "absolute" --home relative/path --db-container cv2-db-one --agents-dir "$T/agents" || return 1
  refused r2 "~" --home "~/cortex" --db-container cv2-db-one --agents-dir "$T/agents" || return 1
  refused r3 ".." --home "$T/home/../home" --db-container cv2-db-one --agents-dir "$T/agents" || return 1
  refused r4 "container" --home "$home" --db-container 'bad name;x' --agents-dir "$T/agents" || return 1
  refused r5 "empty" --home "$home" --db-container '' --agents-dir "$T/agents" || return 1
  refused r6 "newline" --home "$home
x" --db-container cv2-db-one --agents-dir "$T/agents" || return 1
  refused r7 "agents" --home "$home" --db-container cv2-db-one --agents-dir relative || return 1
  refused r8 "--home" --db-container cv2-db-one --agents-dir "$T/agents" || return 1
  refused r9 "path" --home "$home" --db-container cv2-db-one --agents-dir "$T/agents" --path-value 'relative:/usr/bin' || return 1
}
ml_11() { # a program missing from <home>/bin is named, with the fix
  new_test
  rm -f "$home/bin/cortex-dump-status"
  refused m1 "cortex-dump-status" --home "$home" --db-container cv2-db-one --agents-dir "$T/agents" || return 1
  need "the message says it is missing or not executable (not another check's sentence)" grep -q 'missing or not executable' "$T/m1.err" || return 1
  # a program that is there but NOT executable is refused too, with the same sentence (the ownership and mode checks of H4 do not look at the x bit)
  new_test
  chmod 0644 "$home/bin/cortex-dump"
  refused m2 "cortex-dump" --home "$home" --db-container cv2-db-one --agents-dir "$T/agents" || return 1
  need "a non-executable program: the message says missing or not executable" grep -q 'missing or not executable' "$T/m2.err" || return 1
}
ml_12() { # not as root, and not without launchctl
  new_test
  mkdir -p "$T/rootbin"
  printf '#!/bin/sh\nif [ "${1:-}" = -u ]; then echo 0; else /usr/bin/id "$@"; fi\n' > "$T/rootbin/id"
  chmod 0755 "$T/rootbin/id"
  : > "$T/lc.log"
  run r env PATH="$T/rootbin:$T/bin:$PATH" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" HOME="$T/h" $sut "$lb" install --path-value "$ap" --home "$home" --db-container cv2-db-one --agents-dir "$T/agents"
  need "as uid 0: refused with exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  need "and the message says why and how to fix it" sh -c "grep -q 'problem:' '$T/r.err' && grep -q 'fix:' '$T/r.err' && grep -qi 'root' '$T/r.err'" || return 1
  need "nothing was called" test ! -s "$T/lc.log" || return 1
  mkdir -p "$T/nolc"
  for t in sh bash env id python3 cat sed grep mv cp mkdir chmod stat cmp tr ls head dirname rm mktemp printf; do
    p=$(command -v "$t" 2> /dev/null || true)
    [ -n "$p" ] && ln -sf "$p" "$T/nolc/$t"
  done
  run n env PATH="$T/nolc" HOME="$T/h" $sut "$lb" install --path-value "$ap" --home "$home" --db-container cv2-db-one --agents-dir "$T/agents"
  need "no launchctl on PATH: refused with exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  need "and it says launchctl is missing, with problem: and fix:" sh -c "grep -q 'problem:' '$T/n.err' && grep -q 'fix:' '$T/n.err' && grep -qi 'launchctl' '$T/n.err'" || return 1
}
ml_13() { # a path with a space, &, < and quotes is escaped and round-trips exactly through plistlib
  new_test
  odd="$T/odd & <dir> \"q\" 'x'/home"
  mkdir -p "$odd/bin" "$odd/dumps" "$odd/dump-status"
  cp -p "$home"/bin/* "$odd/bin/"
  lbrun o install --home "$odd" --db-container cv2-db-one --agents-dir "$T/agents"
  need "install succeeds (got $rc): $(tail -n 2 "$T/o.err" | tr '\n' ' ')" test "$rc" -eq 0 || return 1
  got=$(pl "$T/agents/$l_dump.plist" 'd["ProgramArguments"][0]')
  need "the wrapper path round-trips exactly" test "$got" = "$odd/bin/launchd-run" || { printf 'got: %s\n' "$got" >&2; return 1; }
  got2=$(pl "$T/agents/$l_dump.plist" 'd["StandardOutPath"]')
  need "the log path round-trips exactly" test "$got2" = "$odd/logs/cortex-dump.out.log" || { printf 'got: %s\n' "$got2" >&2; return 1; }
}
ml_14() { # a symbolic link in a plist's place is refused and never followed
  new_test
  printf 'precious\n' > "$T/target.txt"
  ln -s "$T/target.txt" "$T/agents/$l_dump.plist"
  : > "$T/lc.log"
  inst l
  need "the install fails (got $rc)" test "$rc" -ne 0 || return 1
  need "the message names a problem and a fix" sh -c "grep -q 'problem:' '$T/l.err' && grep -q 'fix:' '$T/l.err'" || return 1
  need "the link is still a link and its target is untouched" sh -c "[ -L '$T/agents/$l_dump.plist' ] && [ \"\$(cat '$T/target.txt')\" = precious ]" || return 1
  need "nothing was loaded" test ! -s "$T/lc.reg" || return 1
  need "no temporary file was left next to the link ($(ls -A "$T/agents" | tr '\n' ' '))" test "$(ls -A "$T/agents")" = "$l_dump.plist" || return 1
}
ml_15() { # the directories: logs 0700, the default agents directory is ~/Library/LaunchAgents under HOME
  new_test
  run d env PATH="$T/bin:$PATH" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" HOME="$T/h" $sut "$lb" install --path-value "$ap" --home "$home" --db-container cv2-db-one
  need "install with the default agents directory succeeds (got $rc): $(tail -n 2 "$T/d.err" | tr '\n' ' ')" test "$rc" -eq 0 || return 1
  need "the plists are in HOME/Library/LaunchAgents" test -f "$T/h/Library/LaunchAgents/$l_dump.plist" -a -f "$T/h/Library/LaunchAgents/$l_stat.plist" || return 1
  need "the logs directory is mode 700" test "$(stat -c %a "$home/logs")" = 700 || return 1
}

# ---- the wrapper: what OnFailure= and ExecStartPost= did on Linux ---------------------------------------------

alert_stub() { # a recording stand-in for cortex-dump-alert in <home>/bin
  cat > "$home/bin/cortex-dump-alert" <<'STUB'
#!/bin/sh
printf 'ALERT args=%s UNIT=%s STATUS=%s RESULT=%s CODE=%s\n' "$*" "${MONITOR_UNIT-}" "${MONITOR_EXIT_STATUS-}" "${MONITOR_SERVICE_RESULT-}" "${MONITOR_EXIT_CODE-}" >> "${ALERT_LOG:?}"
STUB
  chmod 0755 "$home/bin/cortex-dump-alert"
  : > "$T/alert.log"
}
wrap() { # wrap <name> <extra wrapper args> -- <command...>   (the wrapper under $sut, ALERT_LOG set)
  _n=$1
  shift
  run "$_n" env ALERT_LOG="$T/alert.log" $sut "$home/bin/launchd-run" --unit cortex-dump.service --home "$home" "$@"
}
ml_16() { # exit 0: no alarm; with --clear-on-success the alert tool is asked to clear, once
  new_test
  alert_stub
  wrap w1 -- sh -c 'exit 0'
  need "status 0 (got $rc): $(cat "$T/w1.err")" test "$rc" -eq 0 || return 1
  need "no alert call" test ! -s "$T/alert.log" || return 1
  wrap w2 --clear-on-success -- sh -c 'exit 0'
  need "status 0 with --clear-on-success (got $rc)" test "$rc" -eq 0 || return 1
  need "exactly one call: --clear" test "$(cat "$T/alert.log")" = "ALERT args=--home $home --clear UNIT= STATUS= RESULT= CODE=" || { cat "$T/alert.log" >&2; return 1; }
}
ml_17() { # a failure: the alert tool gets the SAME variables the systemd units give it; the wrapper exits with the command's status
  new_test
  alert_stub
  for n in 1 2 3 4 7; do
    : > "$T/alert.log"
    wrap "f$n" --clear-on-success -- sh -c "exit $n"
    need "exit $n: the wrapper exits $n (got $rc)" test "$rc" -eq "$n" || return 1
    need "exit $n: one alert call with unit, status and result" test "$(cat "$T/alert.log")" = "ALERT args=--home $home UNIT=cortex-dump.service STATUS=$n RESULT=exit-code CODE=exited" || { cat "$T/alert.log" >&2; return 1; }
  done
  : > "$T/alert.log"
  wrap sg -- sh -c 'kill -TERM $$'
  need "killed by a signal: the wrapper exits 143 (got $rc)" test "$rc" -eq 143 || return 1
  need "and the alert result is signal" test "$(cat "$T/alert.log")" = "ALERT args=--home $home UNIT=cortex-dump.service STATUS=143 RESULT=signal CODE=killed" || { cat "$T/alert.log" >&2; return 1; }
}
ml_18() { # end to end with the REAL alert and status tools: failure writes the marker (the alert tool's own reason), success clears it
  new_test
  : > "$T/alert.log"
  rm -f "$home/ALARM"
  mkdir -p "$home/dump-status"
  wrap e1 -- sh -c 'exit 4'
  need "the wrapper exits 4 (got $rc)" test "$rc" -eq 4 || return 1
  need "the marker exists and says it was a timeout (the alert tool's table for status 4)" sh -c "grep -q '\"source\":\"backup\"' '$home/ALARM' && grep -q '\"reason\":\"timed out\"' '$home/ALARM' && grep -q '\"unit\":\"cortex-dump.service\"' '$home/ALARM'" || { cat "$home/ALARM" >&2; return 1; }
  wrap e2 --clear-on-success -- sh -c 'exit 0'
  need "a later success with --clear-on-success exits 0 (got $rc)" test "$rc" -eq 0 || return 1
  need "and the marker is gone" test ! -e "$home/ALARM" || return 1
  printf '{"source":"someone-else"}\n' > "$home/ALARM"
  wrap e3 --clear-on-success -- sh -c 'exit 0'
  need "another source's marker is left exactly as it is" test "$(cat "$home/ALARM")" = '{"source":"someone-else"}' || return 1
}
ml_19() { # nothing from the environment or the command line reaches the marker or the wrapper's own output
  new_test
  : > "$T/alert.log"
  rm -f "$home/ALARM"
  run s env PGPASSWORD=canary-secret-env-7f3 ALERT_LOG="$T/alert.log" $sut "$home/bin/launchd-run" --unit cortex-dump.service --home "$home" -- sh -c 'exit 1' sh canary-secret-arg-9d2
  need "the wrapper exits 1 (got $rc)" test "$rc" -eq 1 || return 1
  need "the marker has no canary" test -z "$(grep -l 'canary-secret' "$home/ALARM" || true)" || return 1
  need "the wrapper's output has no canary" test -z "$(cat "$T/s.out" "$T/s.err" | grep 'canary-secret' || true)" || return 1
}
ml_20() { # bad wrapper usage: exit 2, the command never runs
  new_test
  alert_stub
  rm -f "$T/ran"
  run b1 env ALERT_LOG="$T/alert.log" $sut "$home/bin/launchd-run" --unit cortex-dump.service --home "$home" sh -c ': > "$1"' sh "$T/ran"
  need "no -- : exit 2 (got $rc)" test "$rc" -eq 2 -a ! -e "$T/ran" || return 1
  run b2 env ALERT_LOG="$T/alert.log" $sut "$home/bin/launchd-run" --home "$home" -- sh -c ': > "$1"' sh "$T/ran"
  need "no --unit: exit 2 (got $rc)" test "$rc" -eq 2 -a ! -e "$T/ran" || return 1
  run b3 env ALERT_LOG="$T/alert.log" $sut "$home/bin/launchd-run" --unit 'bad unit;x' --home "$home" -- sh -c ': > "$1"' sh "$T/ran"
  need "a unit name outside the grammar: exit 2 (got $rc)" test "$rc" -eq 2 -a ! -e "$T/ran" || return 1
  run b4 env ALERT_LOG="$T/alert.log" $sut "$home/bin/launchd-run" --unit cortex-dump.service --home relative -- sh -c ': > "$1"' sh "$T/ran"
  need "a relative --home: exit 2 (got $rc)" test "$rc" -eq 2 -a ! -e "$T/ran" || return 1
  run b5 env ALERT_LOG="$T/alert.log" $sut "$home/bin/launchd-run" --unit cortex-dump.service --home "$home" --
  need "no command after --: exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  run b6 env ALERT_LOG="$T/alert.log" $sut "$home/bin/launchd-run" --unit cortex-dump.service --home "$home"
  need "options only, no --: exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  need "and it says the -- is missing, not just that there is no command" grep -q 'no -- before the command' "$T/b6.err" || return 1
  need "each message has problem: and fix:" sh -c "grep -q 'problem:' '$T/b1.err' && grep -q 'fix:' '$T/b1.err' && grep -q 'problem:' '$T/b5.err' && grep -q 'fix:' '$T/b6.err'" || return 1
  need "the alert tool was never called" test ! -s "$T/alert.log" || return 1
}

ml_29() { # a template that renders to an invalid property list: refused with exit 1, launchctl changes nothing, nothing is written or left behind
  new_test
  mkdir -p "$T/badtpl"
  cp "$tpl"/*.plist.in "$T/badtpl/"
  printf '<plist><dict><key>Label</key>\n' > "$T/badtpl/$l_stat.plist.in"
  : > "$T/lc.log"
  lbrun b install --home "$home" --db-container cv2-db-one --agents-dir "$T/agents" --template-dir "$T/badtpl"
  need "the install fails (got $rc)" test "$rc" -eq 1 || return 1
  need "the message names the problem and a fix, and says it is not a valid property list" sh -c "grep -q 'problem:' '$T/b.err' && grep -q 'fix:' '$T/b.err' && grep -qi 'not a valid property list' '$T/b.err'" || return 1
  need "launchctl was never asked to change anything" test -z "$(grep -vE '^launchctl print ' "$T/lc.log" || true)" || return 1
  need "nothing is left in the agents directory ($(ls -A "$T/agents" | tr '\n' ' '))" test -z "$(ls -A "$T/agents")" || return 1
}

ml_30() { # locale independence: the cells that compare directory listings pass at LC_ALL=C.UTF-8 too, which orders the two plist names the other way round (sam M1C-SAM-04)
  new_test
  mkdir -p "$T/nested"
  run n env LC_ALL=C.UTF-8 CORTEX_ML_TMP="$T/nested" $sut "$here/launchd-test.sh" --shell sh ML-05 ML-09
  need "ML-05 and ML-09 pass at LC_ALL=C.UTF-8 (got $rc): $(grep 'CHECK FAILED' "$T/n.err" | head -n 2 | tr '\n' '|')" test "$rc" -eq 0 || return 1
}
ml_31() { # a failure at the SECOND agent puts the FIRST back too: the pair is all or nothing (sam M1C-SAM-02)
  new_test
  # (a) a fresh install: only the check agent's bootstrap fails; the dump agent, already loaded, is unloaded and its new file removed
  : > "$T/lc.log"
  run a env PATH="$T/bin:$PATH" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" FAKE_LC_FAIL_PLIST="$l_stat" FAKE_LC_FAIL_RC=5 HOME="$T/h" $sut "$lb" install --path-value "$ap" --home "$home" --db-container cv2-db-one --agents-dir "$T/agents"
  need "(a) the install fails (got $rc)" test "$rc" -ne 0 || return 1
  need "(a) it did try the dump agent first (it was bootstrapped before the failure)" grep -q "^launchctl bootstrap $dom $T/agents/$l_dump.plist\$" "$T/lc.log" || return 1
  need "(a) nothing is left in the agents directory ($(lsa "$T/agents"))" test -z "$(ls -A "$T/agents")" || return 1
  need "(a) nothing is loaded ($(cat "$T/lc.reg" | tr '\n' ' '))" test ! -s "$T/lc.reg" || return 1
  need "(a) the message says everything is as it was before this run" grep -q 'as they were before this run' "$T/a.err" || return 1
  # (b) both installed and loaded; a changed PATH value changes both files; the check agent's new bootstrap fails: the dump agent goes back to its
  # OLD bytes and is loaded again; the check agent's old bytes are back and it is named as one that could not be reloaded
  lbrun b1 install --home "$home" --db-container cv2-db-one --agents-dir "$T/agentsB"
  need "(b) a good install first succeeds (got $rc)" test "$rc" -eq 0 || return 1
  sumsB=$(cksum "$T"/agentsB/*.plist)
  : > "$T/lc.log"
  run b2 env PATH="$T/bin:$PATH" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" FAKE_LC_FAIL_PLIST="$l_stat" FAKE_LC_FAIL_RC=5 HOME="$T/h" $sut "$lb" install --path-value "$ap:/usr/sbin" --home "$home" --db-container cv2-db-one --agents-dir "$T/agentsB"
  need "(b) the reinstall fails (got $rc)" test "$rc" -ne 0 || return 1
  need "(b) both files have their OLD bytes ($(lsa "$T/agentsB"))" test "$(cksum "$T"/agentsB/*.plist)" = "$sumsB" || return 1
  need "(b) the dump agent is loaded again from its old file" grep -qx "$dom/$l_dump" "$T/lc.reg" || return 1
  need "(b) the message names the check agent as one that could not be reloaded" sh -c "grep -q 'could not be reloaded' '$T/b2.err' && grep -q '$l_stat' '$T/b2.err'" || return 1
  need "(b) no temporary file is left ($(lsa "$T/agentsB"))" test "$(lsa "$T/agentsB")" = "$(printf '%s\n' "$l_dump.plist" "$l_stat.plist" | LC_ALL=C sort | tr '\n' ' ')" || return 1
}

ml_32() { # sam M1C-SAM-05: a backup copy that fails stops the run BEFORE anything is unloaded or replaced (errexit is off inside a function called from if !)
  new_test
  inst a1
  need "a good install first succeeds (got $rc)" test "$rc" -eq 0 || return 1
  sums=$(cksum "$T"/agents/*.plist)
  mkdir -p "$T/failbin"
  printf '#!/bin/sh\nexit 1\n' > "$T/failbin/cp"
  chmod 0755 "$T/failbin/cp"
  : > "$T/lc.log"
  run c env PATH="$T/failbin:$T/bin:$PATH" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" HOME="$T/h" $sut "$lb" install --path-value "$ap" --home "$home" --db-container cv2-db-two --agents-dir "$T/agents"
  need "the reinstall fails (got $rc)" test "$rc" -ne 0 || return 1
  need "the message names the backup, with problem: and fix:" sh -c "grep -q 'problem:' '$T/c.err' && grep -q 'fix:' '$T/c.err' && grep -qi 'backup' '$T/c.err'" || return 1
  need "launchctl was never asked to unload or load anything ($(grep -E 'bootout|bootstrap' "$T/lc.log" | tr '\n' '|'))" test -z "$(grep -E 'bootout|bootstrap' "$T/lc.log" || true)" || return 1
  need "both files have their old bytes" test "$(cksum "$T"/agents/*.plist)" = "$sums" || return 1
  need "both agents are still loaded" test "$(sort "$T/lc.reg" | tr '\n' ' ')" = "$dom/$l_dump $dom/$l_stat " || return 1
  need "no temporary file is left ($(lsa "$T/agents"))" test "$(lsa "$T/agents")" = "$(printf '%s\n' "$l_dump.plist" "$l_stat.plist" | LC_ALL=C sort | tr '\n' ' ')" || return 1
  # part 2: a mode change of the NEW file that fails also stops the run before anything is unloaded or replaced
  rm -f "$T/failbin/cp"
  printf '#!/bin/sh\nfor last; do :; done\ncase $last in *.new.*) exit 1 ;; esac\nexec %s "$@"\n' "$(command -v chmod)" > "$T/failbin/chmod"
  chmod 0755 "$T/failbin/chmod"
  : > "$T/lc.log"
  run c2 env PATH="$T/failbin:$T/bin:$PATH" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" HOME="$T/h" $sut "$lb" install --path-value "$ap" --home "$home" --db-container cv2-db-two --agents-dir "$T/agents"
  need "(part 2) the reinstall fails when the mode of the new file cannot be set (got $rc)" test "$rc" -ne 0 || return 1
  need "(part 2) problem and fix, and it names the mode" sh -c "grep -q 'problem:' '$T/c2.err' && grep -q 'fix:' '$T/c2.err' && grep -qi 'mode' '$T/c2.err'" || return 1
  need "(part 2) launchctl was never asked to unload or load anything" test -z "$(grep -E 'bootout|bootstrap' "$T/lc.log" || true)" || return 1
  need "(part 2) both files have their old bytes" test "$(cksum "$T"/agents/*.plist)" = "$sums" || return 1
}
ml_33() { # sam M1C-SAM-05: a rename that fails after the old agent was unloaded is NOT reported as installed: the old agent is loaded again and nothing changed
  new_test
  inst a1
  need "a good install first succeeds (got $rc)" test "$rc" -eq 0 || return 1
  sums=$(cksum "$T"/agents/*.plist)
  mkdir -p "$T/failbin"
  realmv=$(command -v mv)
  # fails the FIRST rename whose target is a plist (the tool's own; the fake launchctl renames its registry file with the same mv, which must not be hit)
  printf '#!/bin/sh\nfor last; do :; done\ncase $last in *.plist) if [ ! -e "%s" ]; then : > "%s"; exit 1; fi ;; esac\nexec %s "$@"\n' "$T/mvstate" "$T/mvstate" "$realmv" > "$T/failbin/mv"
  chmod 0755 "$T/failbin/mv"
  : > "$T/lc.log"
  run m env PATH="$T/failbin:$T/bin:$PATH" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" HOME="$T/h" $sut "$lb" install --path-value "$ap" --home "$home" --db-container cv2-db-two --agents-dir "$T/agents"
  need "the reinstall fails when the first rename fails (got $rc)" test "$rc" -ne 0 || return 1
  need "it does not say installed" test -z "$(grep -i 'installed' "$T/m.out" || true)" || return 1
  need "the message has problem: and fix:" sh -c "grep -q 'problem:' '$T/m.err' && grep -q 'fix:' '$T/m.err'" || return 1
  need "both files have their old bytes (the old container name is still in the dump plist)" sh -c "[ \"\$(cksum '$T'/agents/*.plist)\" = \"$sums\" ] && grep -q cv2-db-one '$T/agents/$l_dump.plist'" || return 1
  need "the old dump agent is loaded again, the check agent still is" test "$(sort "$T/lc.reg" | tr '\n' ' ')" = "$dom/$l_dump $dom/$l_stat " || return 1
  need "the message says everything is as it was before this run" grep -q 'as they were before this run' "$T/m.err" || return 1
  need "no temporary file is left ($(lsa "$T/agents"))" test "$(lsa "$T/agents")" = "$(printf '%s\n' "$l_dump.plist" "$l_stat.plist" | LC_ALL=C sort | tr '\n' ' ')" || return 1
}
ml_34() { # sam M1C-SAM-06: an agent this run loaded that CANNOT be unloaded in the rollback is reported as still loaded, never as restored
  new_test
  run a env PATH="$T/bin:$PATH" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" FAKE_LC_FAIL=bootout FAKE_LC_FAIL_RC=3 FAKE_LC_FAIL_PLIST="$l_stat" HOME="$T/h" $sut "$lb" install --path-value "$ap" --home "$home" --db-container cv2-db-one --agents-dir "$T/agents"
  need "the install fails (got $rc)" test "$rc" -ne 0 || return 1
  need "the dump agent is still loaded (the fake refused the bootout)" grep -qx "$dom/$l_dump" "$T/lc.reg" || return 1
  need "the message says so: still loaded, and names the dump agent" sh -c "grep -q 'still loaded' '$T/a.err' && grep -q '$l_dump' '$T/a.err'" || return 1
  need "the message does NOT claim that everything is as it was" test -z "$(grep 'as they were before this run' "$T/a.err" | grep -v 'but' || true)" || return 1
  need "the dump plist is left in place, consistent with the loaded agent" test -f "$T/agents/$l_dump.plist" || return 1
  need "the failing check agent's own file was removed" test ! -e "$T/agents/$l_stat.plist" || return 1
  need "no temporary file is left ($(lsa "$T/agents"))" test "$(lsa "$T/agents")" = "$l_dump.plist " || return 1
}

ml_36() { # the unload of the old agent fails during a replacement: nothing changed, the old agent is still loaded, and the message says so
  new_test
  inst a1
  need "a good install first succeeds (got $rc)" test "$rc" -eq 0 || return 1
  sums=$(cksum "$T"/agents/*.plist)
  : > "$T/lc.log"
  run b env PATH="$T/bin:$PATH" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" FAKE_LC_FAIL=bootout FAKE_LC_FAIL_RC=3 HOME="$T/h" $sut "$lb" install --path-value "$ap" --home "$home" --db-container cv2-db-two --agents-dir "$T/agents"
  need "the reinstall fails when the old agent cannot be unloaded (got $rc)" test "$rc" -ne 0 || return 1
  need "the message names the unload, with problem: and fix:" sh -c "grep -q 'problem:' '$T/b.err' && grep -q 'fix:' '$T/b.err' && grep -qi 'unload' '$T/b.err'" || return 1
  need "both files have their old bytes, nothing replaced" test "$(cksum "$T"/agents/*.plist)" = "$sums" || return 1
  need "both agents are still loaded" test "$(sort "$T/lc.reg" | tr '\n' ' ')" = "$dom/$l_dump $dom/$l_stat " || return 1
  need "no bootstrap was attempted" test -z "$(grep '^launchctl bootstrap' "$T/lc.log" || true)" || return 1
  need "no temporary file is left ($(lsa "$T/agents"))" test "$(lsa "$T/agents")" = "$(printf '%s\n' "$l_dump.plist" "$l_stat.plist" | LC_ALL=C sort | tr '\n' ' ')" || return 1
}
ml_37() { # a rollback that cannot remove the new files says which files it could not restore (and nothing is claimed as restored)
  new_test
  mkdir -p "$T/failbin"
  printf '#!/bin/sh\nfor last; do :; done\ncase $last in *.plist) exit 1 ;; esac\nexec %s "$@"\n' "$(command -v rm)" > "$T/failbin/rm"
  chmod 0755 "$T/failbin/rm"
  run a env PATH="$T/failbin:$T/bin:$PATH" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" FAKE_LC_FAIL_PLIST="$l_stat" FAKE_LC_FAIL_RC=5 HOME="$T/h" $sut "$lb" install --path-value "$ap" --home "$home" --db-container cv2-db-one --agents-dir "$T/agents"
  need "the install fails (got $rc)" test "$rc" -ne 0 || return 1
  need "the message names the files it could not restore, and says it could NOT put everything back" sh -c "grep -q 'files not restored' '$T/a.err' && grep -q 'could NOT put everything back' '$T/a.err' && grep -q '$l_stat' '$T/a.err'" || return 1
  need "the message does not claim that everything is as it was" test -z "$(grep 'as they were before this run' "$T/a.err" | grep -v 'but' || true)" || return 1
  need "nothing is loaded (the unloads worked)" test ! -s "$T/lc.reg" || return 1
  need "the files that could not be removed are still there ($(lsa "$T/agents"))" test -f "$T/agents/$l_stat.plist" -o -f "$T/agents/$l_dump.plist" || return 1
}
ml_35() { # found while auditing the same family of defects myself: uninstall must also check its own steps (a file that cannot be removed is a problem with a fix, not a silent exit)
  new_test
  inst a1
  need "a good install first succeeds (got $rc)" test "$rc" -eq 0 || return 1
  mkdir -p "$T/failbin"
  printf '#!/bin/sh\nfor last; do :; done\ncase $last in *.plist) exit 1 ;; esac\nexec %s "$@"\n' "$(command -v rm)" > "$T/failbin/rm"
  chmod 0755 "$T/failbin/rm"
  run u env PATH="$T/failbin:$T/bin:$PATH" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" HOME="$T/h" $sut "$lb" uninstall --agents-dir "$T/agents"
  need "uninstall fails when a plist cannot be removed (got $rc)" test "$rc" -eq 1 || return 1
  need "the message names a problem, a fix and the file" sh -c "grep -q 'problem:' '$T/u.err' && grep -q 'fix:' '$T/u.err' && grep -q 'ai.kaidera.cortex-dump' '$T/u.err'" || return 1
  need "it does not say removed for a file it could not remove" test -z "$(grep 'removed' "$T/u.out" | grep 'cortex-dump.plist' || true)" || return 1
}

ml_38() { # sam M1C-SAM-07 (a): when the restore of an OLD file fails, the saved old copy is KEPT, its path is printed, and the exit cleanup does not delete it
  new_test
  inst a1
  need "a good install first succeeds (got $rc)" test "$rc" -eq 0 || return 1
  old_dump=$(cksum < "$T/agents/$l_dump.plist")
  mkdir -p "$T/failbin"
  realmv=$(command -v mv)
  # fails the restore rename of the DUMP agent's saved copy (its name ends .prev.<pid>); every other rename works
  printf '#!/bin/sh\ncase "$*" in *cortex-dump.plist.prev.*) exit 1 ;; esac\nexec %s "$@"\n' "$realmv" > "$T/failbin/mv"
  chmod 0755 "$T/failbin/mv"
  run a env PATH="$T/failbin:$T/bin:$PATH" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" FAKE_LC_FAIL_PLIST="$l_stat" HOME="$T/h" $sut "$lb" install --path-value "$ap:/usr/sbin" --home "$home" --db-container cv2-db-one --agents-dir "$T/agents"
  need "the reinstall fails (got $rc)" test "$rc" -ne 0 || return 1
  kept=$(ls -A "$T/agents" | grep "^\\.$l_dump\\.plist\\.prev\\." || true)
  need "the OLD copy of the dump plist is still in the agents directory ($(lsa "$T/agents"))" test -n "$kept" || return 1
  need "and it holds the OLD bytes" test "$(cksum < "$T/agents/$kept")" = "$old_dump" || return 1
  need "the message prints the exact path of the kept copy and says KEPT" sh -c "grep -q 'KEPT' '$T/a.err' && grep -qF '$T/agents/$kept' '$T/a.err'" || return 1
  need "the message says the dump file was not restored" grep -q 'files not restored' "$T/a.err" || return 1
}
ml_39() { # sam M1C-SAM-07 (b): when the NEW first replacement cannot be unloaded in the rollback, the old copy is KEPT and named, and the new dump stays loaded
  new_test
  inst a1
  need "a good install first succeeds (got $rc)" test "$rc" -eq 0 || return 1
  old_dump=$(cksum < "$T/agents/$l_dump.plist")
  rm -f "$T/lc.reg.count.bootout"
  run b env PATH="$T/bin:$PATH" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" FAKE_LC_FAIL_PLIST="$l_stat" FAKE_LC_FAIL_AFTER=bootout:3 FAKE_LC_FAIL_RC=3 HOME="$T/h" $sut "$lb" install --path-value "$ap:/usr/sbin" --home "$home" --db-container cv2-db-one --agents-dir "$T/agents"
  need "the reinstall fails (got $rc)" test "$rc" -ne 0 || return 1
  kept=$(ls -A "$T/agents" | grep "^\\.$l_dump\\.plist\\.prev\\." || true)
  need "the OLD copy of the dump plist is still in the agents directory ($(lsa "$T/agents"))" test -n "$kept" || return 1
  need "and it holds the OLD bytes" test "$(cksum < "$T/agents/$kept")" = "$old_dump" || return 1
  need "the new dump agent is still loaded (the rollback unload was refused)" grep -qx "$dom/$l_dump" "$T/lc.reg" || return 1
  need "the message says still loaded, and prints the kept path with KEPT" sh -c "grep -q 'still loaded' '$T/b.err' && grep -q 'KEPT' '$T/b.err' && grep -qF '$T/agents/$kept' '$T/b.err'" || return 1
}
ml_40() { # sam M1C-SAM-07 (c): a retry after such a failure is safe: it works, and it never deletes the copy a PREVIOUS run kept
  new_test
  inst a1
  need "a good install first succeeds (got $rc)" test "$rc" -eq 0 || return 1
  old_dump=$(cksum < "$T/agents/$l_dump.plist")
  rm -f "$T/lc.reg.count.bootout"
  run b env PATH="$T/bin:$PATH" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" FAKE_LC_FAIL_PLIST="$l_stat" FAKE_LC_FAIL_AFTER=bootout:3 FAKE_LC_FAIL_RC=3 HOME="$T/h" $sut "$lb" install --path-value "$ap:/usr/sbin" --home "$home" --db-container cv2-db-one --agents-dir "$T/agents"
  need "the first reinstall fails (got $rc)" test "$rc" -ne 0 || return 1
  kept=$(ls -A "$T/agents" | grep "^\\.$l_dump\\.plist\\.prev\\." || true)
  need "a recovery copy was kept" test -n "$kept" || return 1
  rm -f "$T/lc.reg.count.bootout"
  run c env PATH="$T/bin:$PATH" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" HOME="$T/h" $sut "$lb" install --path-value "$ap:/usr/sbin" --home "$home" --db-container cv2-db-one --agents-dir "$T/agents"
  need "the retry succeeds (got $rc): $(tail -n 2 "$T/c.err" | tr '\n' ' ')" test "$rc" -eq 0 || return 1
  need "both agents are loaded" test "$(sort "$T/lc.reg" | tr '\n' ' ')" = "$dom/$l_dump $dom/$l_stat " || return 1
  need "the retry did not delete the copy the previous run kept, and it is unchanged" sh -c "[ -f '$T/agents/$kept' ] && [ \"\$(cksum < '$T/agents/$kept')\" = '$old_dump' ]" || return 1
  need "the retry left no recovery copy of its OWN ($(lsa "$T/agents"))" test "$(ls -A "$T/agents" | grep -c '[.]prev[.]')" -eq 1 || return 1
}

ml_41() { # sam M1C-SAM-07 (both labels): when the restore of BOTH old files fails, BOTH saved copies are kept, named, and hold the old bytes
  new_test
  inst a1
  need "a good install first succeeds (got $rc)" test "$rc" -eq 0 || return 1
  old_dump=$(cksum < "$T/agents/$l_dump.plist")
  old_stat=$(cksum < "$T/agents/$l_stat.plist")
  mkdir -p "$T/failbin"
  realmv=$(command -v mv)
  # fails EVERY restore rename (the saved copies' names contain .plist.prev.); the forward renames of the new files work
  printf '#!/bin/sh\ncase "$*" in *.plist.prev.*) exit 1 ;; esac\nexec %s "$@"\n' "$realmv" > "$T/failbin/mv"
  chmod 0755 "$T/failbin/mv"
  run a env PATH="$T/failbin:$T/bin:$PATH" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" FAKE_LC_FAIL_PLIST="$l_stat" HOME="$T/h" $sut "$lb" install --path-value "$ap:/usr/sbin" --home "$home" --db-container cv2-db-one --agents-dir "$T/agents"
  need "the reinstall fails (got $rc)" test "$rc" -ne 0 || return 1
  kd=$(ls -A "$T/agents" | grep "^\\.$l_dump\\.plist\\.prev\\." || true)
  ks=$(ls -A "$T/agents" | grep "^\\.$l_stat\\.plist\\.prev\\." || true)
  need "the OLD copy of the dump plist is kept ($(lsa "$T/agents"))" test -n "$kd" || return 1
  need "the OLD copy of the status plist is kept ($(lsa "$T/agents"))" test -n "$ks" || return 1
  need "the dump copy holds the OLD bytes" test "$(cksum < "$T/agents/$kd")" = "$old_dump" || return 1
  need "the status copy holds the OLD bytes" test "$(cksum < "$T/agents/$ks")" = "$old_stat" || return 1
  need "the message prints the exact path of EACH kept copy and says KEPT" sh -c "grep -q 'KEPT' '$T/a.err' && grep -qF '$T/agents/$kd' '$T/a.err' && grep -qF '$T/agents/$ks' '$T/a.err'" || return 1
  need "exactly two recovery copies exist, nothing else of the kind ($(lsa "$T/agents"))" test "$(ls -A "$T/agents" | grep -c '[.]prev[.]')" -eq 2 || return 1
}

ml_42() { # hale M1C-H1: the agents directory is the user's and not writable by group or world (the pid-based temporary and recovery names are only safe then)
  new_test
  for m in 0777 0775 0757; do
    d=$T/ad-$m
    mkdir -p "$d"
    chmod "$m" "$d"
    : > "$T/lc.log"
    run "i$m" env PATH="$T/bin:$PATH" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" HOME="$T/h" $sut "$lb" install --home "$home" --db-container cv2-db-one --agents-dir "$d" --path-value "$ap"
    need "an agents directory of mode $m is refused with exit 2 (got $rc)" test "$rc" -eq 2 || return 1
    need "mode $m: the message names the directory and says it is writable by group or world, with problem and fix" sh -c "grep -q 'problem:' '$T/i$m.err' && grep -q 'fix:' '$T/i$m.err' && grep -qF '$d' '$T/i$m.err' && grep -qi 'writable' '$T/i$m.err'" || return 1
    need "mode $m: nothing was written and launchctl changed nothing ($(lsa "$d"))" sh -c "[ -z \"\$(ls -A '$d')\" ] && [ -z \"\$(grep -vE '^launchctl print ' '$T/lc.log' || true)\" ]" || return 1
    run "u$m" env PATH="$T/bin:$PATH" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" HOME="$T/h" $sut "$lb" uninstall --agents-dir "$d"
    need "mode $m: uninstall refuses it too (got $rc)" test "$rc" -eq 2 || return 1
  done
  # a directory that is not the user's (a system directory, root's): refused before anything is written
  run fo env PATH="$T/bin:$PATH" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" HOME="$T/h" $sut "$lb" install --home "$home" --db-container cv2-db-one --agents-dir /usr/bin --path-value "$ap"
  need "an agents directory owned by someone else (/usr/bin) is refused with exit 2 (got $rc) and the message says it is not yours" sh -c "[ $rc -eq 2 ] && grep -qi 'not owned by you' '$T/fo.err'" || return 1
  # the control: an ordinary 0755 directory of the user's
  mkdir -p "$T/ad-ok"
  chmod 0755 "$T/ad-ok"
  run ok env PATH="$T/bin:$PATH" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" HOME="$T/h" $sut "$lb" install --home "$home" --db-container cv2-db-one --agents-dir "$T/ad-ok" --path-value "$ap"
  need "an ordinary 0755 directory of the user's is accepted (got $rc): $(tail -n 2 "$T/ok.err" | tr '\n' ' ')" test "$rc" -eq 0 || return 1
}
ml_43() { # hale M1C-H2: a PATH element that anyone can write, or that is neither the user's nor root's, is refused; a group-writable one is a WARNING; the resolved tools are printed
  new_test
  mkdir -p "$T/wwbin" "$T/gwbin" "$T/ag1" "$T/ag2" "$T/ag3" "$T/ag4" "$T/ag5"
  chmod 0777 "$T/wwbin"
  chmod 0775 "$T/gwbin"
  pr() { # pr <name> <agents dir> <path value>
    : > "$T/lc.log"
    run "$1" env PATH="$T/bin:$PATH" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" HOME="$T/h" $sut "$lb" install --home "$home" --db-container cv2-db-one --agents-dir "$2" --path-value "$3"
  }
  pr w "$T/ag1" "$ap:$T/wwbin"
  need "a world-writable PATH element is refused with exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  need "the message names the element, says it is writable, with problem and fix" sh -c "grep -q 'problem:' '$T/w.err' && grep -q 'fix:' '$T/w.err' && grep -qF '$T/wwbin' '$T/w.err' && grep -qi 'writable' '$T/w.err'" || return 1
  need "nothing was written for it ($(lsa "$T/ag1"))" test -z "$(ls -A "$T/ag1")" || return 1
  pr g "$T/ag2" "$ap:$T/gwbin"
  need "a group-writable PATH element is accepted (got $rc): $(tail -n 2 "$T/g.err" | tr '\n' ' ')" test "$rc" -eq 0 || return 1
  need "but it is named in a warning" sh -c "grep -qi 'warning' '$T/g.err' '$T/g.out' && grep -qF '$T/gwbin' '$T/g.err' '$T/g.out'" || return 1
  pr n "$T/ag3" "$ap:$T/no-such-directory"
  need "an element that does not exist is skipped (got $rc)" test "$rc" -eq 0 || return 1
  need "the install prints where each of the four tools resolves on the agent PATH" sh -c "for t in $req_tools; do grep -qxF \"launchd-backup: resolved \$t -> $T/agentbin/\$t\" '$T/n.out' || exit 1; done" || return 1
  other=$(find / -xdev -maxdepth 3 -type d ! -user root ! -user "$(id -un)" ! -perm -020 ! -perm -002 2> /dev/null | head -n 1)
  if [ -n "$other" ]; then
    echo "    NOTE: the foreign-owner refusal of a PATH element ran against $other ($(ls -ld "$other"))" >&2
    pr f "$T/ag4" "$ap:$other"
    need "a PATH element owned by someone other than the user or root ($other) is refused (got $rc) and the message says whose" sh -c "[ $rc -eq 2 ] && grep -qF '$other' '$T/f.err' && grep -qi 'not owned' '$T/f.err'" || return 1
  else
    echo "    NOTE: no directory owned by another user was found on this host: the foreign-owner refusal of a PATH element is NOT RUN here" >&2
  fi
  pr r "$T/ag5" "$ap:/usr/bin:/bin"
  need "root's directories on the PATH are accepted (got $rc)" test "$rc" -eq 0 || return 1
}
ml_44() { # hale M1C-H3: exported shell functions in the environment must not replace a command of the tool or of the wrapper
  new_test
  mkdir -p "$T/agh"
  : > "$T/canary44"
  # stand-ins for the commands the tool relies on, hale's list (a [, printf, echo and umask that print a canary, a find that makes every file "open", an id that says root, a grep that
  # fails, a sed that does nothing right): none of them may run, and none may change the result
  run h env PATH="$T/bin:$PATH" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" HOME="$T/h" CANARY="$T/canary44" 'BASH_FUNC_id%%=() { echo 0; }' 'BASH_FUNC_[%%=() { echo "[" >> "$CANARY"; return 1; }' 'BASH_FUNC_grep%%=() { return 1; }' 'BASH_FUNC_sed%%=() { cat; }' 'BASH_FUNC_printf%%=() { echo printf >> "$CANARY"; }' 'BASH_FUNC_echo%%=() { echo echo >> "$CANARY"; }' 'BASH_FUNC_umask%%=() { echo umask >> "$CANARY"; }' 'BASH_FUNC_find%%=() { echo /; }' 'BASH_FUNC_cat%%=() { echo cat >> "$CANARY"; }' $sut "$lb" install --home "$home" --db-container cv2-db-one --agents-dir "$T/agh" --path-value "$ap"
  need "with exported functions id, [, grep, sed, printf, echo, umask, find and cat in the environment the install still succeeds (got $rc): $(tail -n 2 "$T/h.err" | tr '\n' ' ')" test "$rc" -eq 0 || return 1
  need "and none of the stand-ins that print a canary ever ran ($(tr '\n' ' ' < "$T/canary44"))" test ! -s "$T/canary44" || return 1
  need "and both agents are loaded and rendered as they would be" sh -c "[ \"\$(sort '$T/lc.reg' | tr '\n' ' ')\" = \"$dom/$l_dump $dom/$l_stat \" ] && [ -f '$T/agh/$l_dump.plist' ] && [ -f '$T/agh/$l_stat.plist' ]" || return 1
  # the wrapper: an exported [ that always fails must not change what it runs or what it exits with
  run w1 env ALERT_LOG="$T/alert.log" 'BASH_FUNC_[%%=() { return 1; }' $sut "$home/bin/launchd-run" --unit cortex-dump.service --home "$home" -- sh -c 'exit 3'
  need "the wrapper with an exported [ still exits with the command's own status 3 (got $rc)" test "$rc" -eq 3 || return 1
  : > "$T/canary44"
  run w3 env ALERT_LOG="$T/alert.log" CANARY="$T/canary44" 'BASH_FUNC_printf%%=() { echo printf >> "$CANARY"; }' 'BASH_FUNC_echo%%=() { echo echo >> "$CANARY"; }' 'BASH_FUNC_umask%%=() { echo umask >> "$CANARY"; }' $sut "$home/bin/launchd-run" --unit cortex-dump.service --home "$home" -- sh -c 'exit 4'
  need "the wrapper with exported printf, echo and umask stand-ins exits 4 and none of them ran ($(tr '\n' ' ' < "$T/canary44"))" sh -c "[ $rc -eq 4 ] && [ ! -s '$T/canary44' ]" || return 1
  rm -f "$T/ran44"
  run w2 env ALERT_LOG="$T/alert.log" 'BASH_FUNC_[%%=() { return 1; }' $sut "$home/bin/launchd-run" --unit cortex-dump.service --home "$home" -- sh -c ': > "$1"' sh "$T/ran44"
  need "and it still runs the command it was given (got $rc)" sh -c "[ $rc -eq 0 ] && [ -e '$T/ran44' ]" || return 1
}
ml_45() { # hale M1C-H4: what the agents run is the user's and not writable by group or world: the directory, each of the four scripts, and a link to someone else's
  new_test
  : > "$T/lc.log"
  chk() { # chk <name> <agents dir>: an install against this home, refused or not in the caller
    mkdir -p "$2"
    run "$1" env PATH="$T/bin:$PATH" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" HOME="$T/h" $sut "$lb" install --home "$home" --db-container cv2-db-one --agents-dir "$2" --path-value "$ap"
  }
  chmod 0775 "$home/bin"
  chk d "$T/ah1"
  need "a group-writable <home>/bin is refused with exit 2 (got $rc) and the message names it" sh -c "[ $rc -eq 2 ] && grep -qF '$home/bin' '$T/d.err' && grep -qi 'writable' '$T/d.err'" || return 1
  chmod 0755 "$home/bin"
  for s in launchd-run cortex-dump cortex-dump-status cortex-dump-alert; do
    chmod 0775 "$home/bin/$s"
    chk "s$s" "$T/ah-$s"
    need "a group-writable $s is refused with exit 2 (got $rc) and the message names it" sh -c "[ $rc -eq 2 ] && grep -qF '$home/bin/$s' '$T/s$s.err' && grep -qi 'writable' '$T/s$s.err'" || return 1
    chmod 0755 "$home/bin/$s"
    chmod 0757 "$home/bin/$s"
    chk "w$s" "$T/ah-w$s"
    need "a world-writable $s is refused with exit 2 (got $rc)" test "$rc" -eq 2 || return 1
    chmod 0755 "$home/bin/$s"
  done
  # a link to a program of someone else's (the real true, root's): refused as not the user's
  tr_=""
  for _c in /usr/bin/true /bin/true; do
    if [ -x "$_c" ] && [ -z "$tr_" ]; then tr_=$_c; fi
  done
  need "a real program of root's to link to was found" test -n "$tr_" || return 1
  mv "$home/bin/cortex-dump-alert" "$T/alert.keep"
  ln -s "$tr_" "$home/bin/cortex-dump-alert"
  chk l "$T/ah-link"
  need "a script that is a link to a program owned by someone else is refused with exit 2 (got $rc) and the message says it is not yours" sh -c "[ $rc -eq 2 ] && grep -qF '$home/bin/cortex-dump-alert' '$T/l.err' && grep -qi 'not owned by you' '$T/l.err'" || return 1
  rm -f "$home/bin/cortex-dump-alert"
  mv "$T/alert.keep" "$home/bin/cortex-dump-alert"
  # a script that is a link to the user's OWN 0755 program is judged by what it points to, not by the link's own mode (a link is 0777 on Linux)
  mv "$home/bin/cortex-dump" "$T/dump.real"
  ln -s "$T/dump.real" "$home/bin/cortex-dump"
  chk lk "$T/ah-lk"
  need "a script that is a link to the user's own 0755 program is accepted (got $rc): $(tail -n 2 "$T/lk.err" | tr '\n' ' ')" test "$rc" -eq 0 || return 1
  rm -f "$home/bin/cortex-dump"
  mv "$T/dump.real" "$home/bin/cortex-dump"
  chk ok "$T/ah-ok"
  need "with everything back as it was the install succeeds (got $rc): $(tail -n 2 "$T/ok.err" | tr '\n' ' ')" test "$rc" -eq 0 || return 1
}
ml_46() { # sam M1C-SAM08: a permission query that FAILS is not "no match": every write-bit check refuses (exit 2, problem and fix, nothing written, launchctl untouched) when its find fails or prints and then fails
  new_test
  realfind=$(command -v find)
  mkdir -p "$T/fbin" "$T/pe"
  chmod 0755 "$T/pe"
  # a find stand-in: it fails (status FAKE_FIND_RC) when its arguments contain FAKE_FIND_FAIL_ON, after printing its path argument first when FAKE_FIND_PARTIAL is set; otherwise it IS the real find
  cat > "$T/fbin/find" <<'FAKEFIND'
#!/bin/sh
case " $* " in
  *"${FAKE_FIND_FAIL_ON:-@@none@@}"*)
    if [ -n "${FAKE_FIND_PARTIAL:-}" ]; then printf '%s\n' "$2"; fi
    exit "${FAKE_FIND_RC:-1}" ;;
esac
exec "$FAKE_FIND_REAL" "$@"
FAKEFIND
  chmod 0755 "$T/fbin/find"
  pf() { # pf <name> <command> <agents dir> <fail on> <partial: 1 or empty> <rc> [path value]: the tool with the stand-in first in PATH
    if [ ! -d "$3" ]; then
      mkdir -p "$3"
      chmod 0755 "$3"
    fi
    : > "$T/lc.log"
    if [ "$2" = install ]; then
      run "$1" env PATH="$T/fbin:$T/bin:$PATH" FAKE_FIND_REAL="$realfind" FAKE_FIND_FAIL_ON="$4" FAKE_FIND_PARTIAL="$5" FAKE_FIND_RC="$6" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" HOME="$T/h" $sut "$lb" install --home "$home" --db-container cv2-db-one --agents-dir "$3" --path-value "${7:-$ap}"
    else
      run "$1" env PATH="$T/fbin:$T/bin:$PATH" FAKE_FIND_REAL="$realfind" FAKE_FIND_FAIL_ON="$4" FAKE_FIND_PARTIAL="$5" FAKE_FIND_RC="$6" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" HOME="$T/h" $sut "$lb" "$2" --agents-dir "$3"
    fi
  }
  refused() { # refused <name> <path the message names> <agents dir>: exit 2, problem and fix, the permissions could not be read, nothing written, launchctl untouched
    need "$1: refused with exit 2 (got $rc)" test "$rc" -eq 2 || return 1
    need "$1: the message names $2 and says the permissions could not be read, with problem and fix ($(tail -n 2 "$T/$1.err" | tr '\n' ' '))" sh -c "grep -q 'problem:' '$T/$1.err' && grep -q 'fix:' '$T/$1.err' && grep -qF '$2' '$T/$1.err' && grep -qi 'cannot read the permissions' '$T/$1.err'" || return 1
    need "$1: nothing was written ($(lsa "$3")) and launchctl changed nothing" sh -c "[ -z \"\$(ls -A '$3')\" ] && [ -z \"\$(grep -vE '^launchctl print ' '$T/lc.log' || true)\" ]" || return 1
  }
  for m in fail partial; do
    p=""
    if [ "$m" = partial ]; then p=1; fi
    # the agents directory
    pf "da$m" install "$T/ag-da$m" " -H $T/ag-da$m -prune " "$p" 1
    refused "da$m" "$T/ag-da$m" "$T/ag-da$m" || return 1
    # the scripts directory
    pf "db$m" install "$T/ag-db$m" " -H $home/bin -prune " "$p" 1
    refused "db$m" "$home/bin" "$T/ag-db$m" || return 1
    # each of the four scripts
    for s in launchd-run cortex-dump cortex-dump-status cortex-dump-alert; do
      pf "dc$s$m" install "$T/ag-dc$s$m" " -H $home/bin/$s -prune " "$p" 1
      refused "dc$s$m" "$home/bin/$s" "$T/ag-dc$s$m" || return 1
    done
    # a PATH element of the user's: the world-writable query, then the group-writable query
    pf "dd$m" install "$T/ag-dd$m" " -H $T/pe -prune -perm -002 " "$p" 1 "$ap:$T/pe"
    refused "dd$m" "$T/pe" "$T/ag-dd$m" || return 1
    pf "de$m" install "$T/ag-de$m" " -H $T/pe -prune -perm -020 " "$p" 1 "$ap:$T/pe"
    refused "de$m" "$T/pe" "$T/ag-de$m" || return 1
    # a PATH element of root's (/usr/bin): the query whether root owns it
    pf "df$m" install "$T/ag-df$m" " -H /usr/bin -prune -user root " "$p" 1
    refused "df$m" /usr/bin "$T/ag-df$m" || return 1
  done
  # another status than 1 (a find that is not there answers 127)
  pf dg install "$T/ag-dg" " -H $T/ag-dg -prune " "" 127
  refused dg "$T/ag-dg" "$T/ag-dg" || return 1
  # uninstall and status of an existing installation: refused, and what was installed is exactly as it was
  u=$T/ag-u
  mkdir -p "$u"
  chmod 0755 "$u"
  lbrun iu install --home "$home" --db-container cv2-db-one --agents-dir "$u"
  need "a good install into the agents directory first succeeds (got $rc)" test "$rc" -eq 0 || return 1
  before=$(lsa "$u")
  loaded=$(sort "$T/lc.reg" | tr '\n' ' ')
  for c in uninstall status; do
    pf "u$c" "$c" "$u" " -H $u -prune " "" 1
    need "$c: refused with exit 2 (got $rc) and the message says the permissions could not be read" sh -c "[ $rc -eq 2 ] && grep -qi 'cannot read the permissions' '$T/u$c.err' && grep -q 'problem:' '$T/u$c.err'" || return 1
    need "$c: the files in the agents directory are exactly as they were ($(lsa "$u"))" test "$(lsa "$u")" = "$before" || return 1
    need "$c: the loaded agents are exactly as they were" test "$(sort "$T/lc.reg" | tr '\n' ' ')" = "$loaded" || return 1
    need "$c: launchctl was asked for nothing but print" test -z "$(grep -vE '^launchctl print ' "$T/lc.log" || true)" || return 1
  done
  # a find that is not on the caller's PATH at all
  pdir=$(mkpath nof)
  rm -f "$pdir/find"
  mkdir -p "$T/ag-nof"
  chmod 0755 "$T/ag-nof"
  : > "$T/lc.log"
  run nof env PATH="$pdir" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" HOME="$T/h" $sut "$lb" install --home "$home" --db-container cv2-db-one --agents-dir "$T/ag-nof" --path-value "$ap"
  refused nof "$home/bin" "$T/ag-nof" || return 1
  need "nof: the message says find" grep -qw 'find' "$T/nof.err" || return 1
  # the controls: the same stand-in with nothing to fail on IS the real find, and the three outcomes stay three
  pf cok install "$T/ag-cok" "" "" 1
  need "no query fails: the install succeeds (got $rc): $(tail -n 2 "$T/cok.err" | tr '\n' ' ')" test "$rc" -eq 0 || return 1
  mkdir -p "$T/ag-cm"
  chmod 0775 "$T/ag-cm"
  pf cm install "$T/ag-cm" "" "" 1
  need "a match (a group-writable agents directory) is still refused with exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  need "and with ITS OWN message: writable, not the one for a failed query" sh -c "grep -qi 'writable' '$T/cm.err' && ! grep -qi 'cannot read the permissions' '$T/cm.err'" || return 1
}
ml_47() { # sam M1C-SAM08, the same family: the hygiene block of BOTH scripts lists the exported functions with env and sed; a listing that FAILS must refuse, not go on with the functions in place
  new_test
  for t in sed env; do
    mkdir -p "$T/h-$t"
    cat > "$T/h-$t/$t" <<'FAKEHYG'
#!/bin/sh
case ${FAKE_HYG_MODE:-} in
  fail) exit 1 ;;
  partial) "$FAKE_HYG_REAL" "$@"; exit 1 ;;
esac
exec "$FAKE_HYG_REAL" "$@"
FAKEHYG
    chmod 0755 "$T/h-$t/$t"
    real=$(command -v "$t")
    for m in fail partial; do
      mkdir -p "$T/ag-$t$m"
      chmod 0755 "$T/ag-$t$m"
      : > "$T/lc.log"
      run "i$t$m" env PATH="$T/h-$t:$T/bin:$PATH" FAKE_HYG_MODE="$m" FAKE_HYG_REAL="$real" 'BASH_FUNC_zzhost%%=() { :; }' FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" HOME="$T/h" $sut "$lb" install --home "$home" --db-container cv2-db-one --agents-dir "$T/ag-$t$m" --path-value "$ap"
      need "a $t that $m: the install tool refuses with exit 2 (got $rc)" test "$rc" -eq 2 || return 1
      need "a $t that $m: the message has problem and fix and says it cannot list the functions of the environment ($(tail -n 2 "$T/i$t$m.err" | tr '\n' ' '))" sh -c "grep -q 'problem:' '$T/i$t$m.err' && grep -q 'fix:' '$T/i$t$m.err' && grep -qi 'environment' '$T/i$t$m.err'" || return 1
      need "a $t that $m: nothing was written ($(lsa "$T/ag-$t$m")) and launchctl changed nothing" sh -c "[ -z \"\$(ls -A '$T/ag-$t$m')\" ] && [ -z \"\$(grep -vE '^launchctl print ' '$T/lc.log' || true)\" ]" || return 1
      rm -f "$T/ran47"
      run "w$t$m" env PATH="$T/h-$t:$PATH" FAKE_HYG_MODE="$m" FAKE_HYG_REAL="$real" 'BASH_FUNC_zzhost%%=() { :; }' ALERT_LOG="$T/alert.log" $sut "$home/bin/launchd-run" --unit cortex-dump.service --home "$home" -- sh -c ': > "$1"' sh "$T/ran47"
      need "a $t that $m: the wrapper refuses with exit 2 (got $rc) and says so, with problem and fix" sh -c "[ $rc -eq 2 ] && grep -q 'problem:' '$T/w$t$m.err' && grep -q 'fix:' '$T/w$t$m.err' && grep -qi 'environment' '$T/w$t$m.err'" || return 1
      need "a $t that $m: the wrapper did not run the command" test ! -e "$T/ran47" || return 1
    done
    # (an exported function named zzhost is in every environment here, so that the listing has a name to print: a sed that prints it and then fails must refuse too)
    # the control: the stand-in that delegates changes nothing
    mkdir -p "$T/ag-c$t"
    chmod 0755 "$T/ag-c$t"
    run "ci$t" env PATH="$T/h-$t:$T/bin:$PATH" FAKE_HYG_MODE="" FAKE_HYG_REAL="$real" 'BASH_FUNC_zzhost%%=() { :; }' FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" HOME="$T/h" $sut "$lb" install --home "$home" --db-container cv2-db-one --agents-dir "$T/ag-c$t" --path-value "$ap"
    need "a $t that works (the stand-in delegates): the install succeeds (got $rc): $(tail -n 2 "$T/ci$t.err" | tr '\n' ' ')" test "$rc" -eq 0 || return 1
    rm -f "$T/ran47"
    run "cw$t" env PATH="$T/h-$t:$PATH" FAKE_HYG_MODE="" FAKE_HYG_REAL="$real" 'BASH_FUNC_zzhost%%=() { :; }' ALERT_LOG="$T/alert.log" $sut "$home/bin/launchd-run" --unit cortex-dump.service --home "$home" -- sh -c ': > "$1"' sh "$T/ran47"
    need "a $t that works: the wrapper runs the command (got $rc)" sh -c "[ $rc -eq 0 ] && [ -e '$T/ran47' ]" || return 1
  done
}
ml_48() { # sam M1C-SAM09: render runs under || die, where errexit is OFF: a failed or partly printing escaping step (xml_esc, sed_esc, the final sed) must refuse the install; nothing published or loaded, an existing installation untouched
  new_test
  realsed=$(command -v sed)
  mkdir -p "$T/sbin"
  # a sed stand-in: it counts the calls whose arguments contain FAKE_SED_ON, and the FAKE_SED_NTH-th of them FAILS: mode fail prints nothing and exits 1, mode partial prints the real, complete output and then exits 1; every other call IS the real sed
  cat > "$T/sbin/sed" <<'FAKESED'
#!/bin/sh
case " $* " in
  *"${FAKE_SED_ON:-@@none@@}"*)
    n=$(cat "$FAKE_SED_COUNT" 2> /dev/null || echo 0)
    n=$((n + 1))
    echo "$n" > "$FAKE_SED_COUNT"
    if [ "$n" = "${FAKE_SED_NTH:-0}" ]; then
      if [ "${FAKE_SED_MODE:-fail}" = partial ]; then "$FAKE_SED_REAL" "$@"; fi
      exit 1
    fi ;;
esac
exec "$FAKE_SED_REAL" "$@"
FAKESED
  chmod 0755 "$T/sbin/sed"
  sd() { # sd <name> <on> <nth> <mode> <agents dir> [db container]: an install with the stand-in, which fails its <nth> call whose arguments contain <on>
    if [ ! -d "$5" ]; then
      mkdir -p "$5"
      chmod 0755 "$5"
    fi
    : > "$T/lc.log"
    rm -f "$T/cnt-$1"
    run "$1" env PATH="$T/sbin:$T/bin:$PATH" FAKE_SED_REAL="$realsed" FAKE_SED_ON="$2" FAKE_SED_NTH="$3" FAKE_SED_MODE="$4" FAKE_SED_COUNT="$T/cnt-$1" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" HOME="$T/h" $sut "$lb" install --home "$home" --db-container "${6:-cv2-db-one}" --agents-dir "$5" --path-value "$ap"
  }
  rfused() { # rfused <name> <agents dir>: exit 1, problem and fix, "cannot render", nothing published or left behind, launchctl asked for nothing but print
    need "$1: refused with exit 1 (got $rc): $(tail -n 2 "$T/$1.err" | tr '\n' ' ')" test "$rc" -eq 1 || return 1
    need "$1: the message says it cannot render, with problem and fix" sh -c "grep -q 'problem:' '$T/$1.err' && grep -q 'fix:' '$T/$1.err' && grep -qi 'cannot render' '$T/$1.err'" || return 1
    need "$1: nothing was published or left behind ($(lsa "$2"))" test -z "$(ls -A "$2")" || return 1
    need "$1: launchctl was asked for nothing but print" test -z "$(grep -vE '^launchctl print ' "$T/lc.log" || true)" || return 1
  }
  # the four values of each of the two renders go through xml_esc and then sed_esc: the n-th call of each (1 to 4 the dump, 5 to 8 the status), failing, and failing after a complete print
  for m in fail partial; do
    for n in 1 2 3 4 5 6 7 8; do
      sd "x$n$m" '&amp;' "$n" "$m" "$T/ag-x$n$m"
      rfused "x$n$m" "$T/ag-x$n$m" || return 1
      sd "e$n$m" '[\\/&]' "$n" "$m" "$T/ag-e$n$m"
      rfused "e$n$m" "$T/ag-e$n$m" || return 1
    done
    # the final sed of each render (this one already refused: the guard against losing it, and the proof that the partial file does not stay)
    for n in 1 2; do
      sd "f$n$m" '@CORTEX_HOME@' "$n" "$m" "$T/ag-f$n$m"
      rfused "f$n$m" "$T/ag-f$n$m" || return 1
    done
  done
  # an existing installation: a failed render leaves its files byte for byte, its loaded agents and its directory as they were (the new render would differ: another container)
  u=$T/ag-u
  mkdir -p "$u"
  chmod 0755 "$u"
  lbrun iu install --home "$home" --db-container cv2-db-one --agents-dir "$u"
  need "a good install first succeeds (got $rc)" test "$rc" -eq 0 || return 1
  s1=$(cksum < "$u/$l_dump.plist")
  s2=$(cksum < "$u/$l_stat.plist")
  before=$(lsa "$u")
  loaded=$(sort "$T/lc.reg" | tr '\n' ' ')
  rein() { # rein <name> <on> <nth> <mode>
    sd "$1" "$2" "$3" "$4" "$u" cv2-db-two
    need "$1: the reinstall is refused with exit 1 (got $rc)" test "$rc" -eq 1 || return 1
    need "$1: the dump plist is byte for byte as it was" test "$(cksum < "$u/$l_dump.plist")" = "$s1" || return 1
    need "$1: the status plist is byte for byte as it was" test "$(cksum < "$u/$l_stat.plist")" = "$s2" || return 1
    need "$1: no file was added or removed ($(lsa "$u"))" test "$(lsa "$u")" = "$before" || return 1
    need "$1: the same agents are loaded" test "$(sort "$T/lc.reg" | tr '\n' ' ')" = "$loaded" || return 1
    need "$1: launchctl was asked for nothing but print" test -z "$(grep -vE '^launchctl print ' "$T/lc.log" || true)" || return 1
  }
  rein ra '&amp;' 2 fail || return 1
  rein rb '[\\/&]' 3 partial || return 1
  rein rc '&amp;' 6 partial || return 1
  rein rd '[\\/&]' 8 fail || return 1
  rein re '@CORTEX_HOME@' 2 partial || return 1
  # the control: the stand-in with nothing to fail on IS the real sed
  sd cok '@@none@@' 0 fail "$T/ag-cok"
  need "no step fails: the install succeeds (got $rc): $(tail -n 2 "$T/cok.err" | tr '\n' ' ')" test "$rc" -eq 0 || return 1
  need "and the rendered dump plist names the home" grep -qF "$home" "$T/ag-cok/$l_dump.plist" || return 1
}

# ---- the floor: refuse to install agents that could only fail (the Linux-bound backup tools, plan section 9) --------

ml_27() { # the floor holds on the AGENT's PATH (--path-value), whatever the caller's PATH has (sam M1C-SAM-03); refuses, writes nothing, changes nothing
  new_test
  for t in $req_tools; do
    mkdir -p "$T/ap-$t"
    for u in $req_tools; do
      [ "$u" = "$t" ] || cp -p "$T/agentbin/$u" "$T/ap-$t/$u"
    done
    : > "$T/lc.log"
    # the CALLER's PATH has all four stand-ins; the PATH the agents will get lacks $t
    run "p$t" env PATH="$T/bin:$T/agentbin:$PATH" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" HOME="$T/h" $sut "$lb" install --home "$home" --db-container cv2-db-one --agents-dir "$T/agents" --path-value "$T/ap-$t"
    need "without $t on the agent PATH: refused with exit 2 (got $rc)" test "$rc" -eq 2 || return 1
    need "without $t: the message names $t, with problem: and fix:" sh -c "grep -q 'problem:' '$T/p$t.err' && grep -q 'fix:' '$T/p$t.err' && grep -q -w '$t' '$T/p$t.err'" || return 1
    need "without $t: it says automatic backup is not available on macOS yet and gives the by-hand route" sh -c "grep -qi 'not available' '$T/p$t.err' && grep -q 'pg_dump' '$T/p$t.err'" || return 1
    need "without $t: launchctl changed nothing" test -z "$(grep -vE '^launchctl print ' "$T/lc.log" || true)" || return 1
    need "without $t: no plist was written ($(lsa "$T/agents"))" test -z "$(ls -A "$T/agents" 2> /dev/null || true)" || return 1
  done
  mkdir -p "$T/ap-empty"
  : > "$T/lc.log"
  run emp env PATH="$T/bin:$T/agentbin:$PATH" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" HOME="$T/h" $sut "$lb" install --home "$home" --db-container cv2-db-one --agents-dir "$T/agents" --path-value "$T/ap-empty"
  need "an EMPTY agent PATH directory with all four on the caller's PATH: refused with exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  need "and nothing was loaded or written" test ! -s "$T/lc.reg" -a -z "$(ls -A "$T/agents" 2> /dev/null || true)" || return 1
  # the reverse: the caller's PATH has NONE of the four, the agent PATH has all of them: it goes through
  pdir=$(mkpath none)
  run all env PATH="$pdir" FAKE_LC_LOG="$T/lc.log" FAKE_LC_REG="$T/lc.reg" HOME="$T/h" $sut "$lb" install --home "$home" --db-container cv2-db-one --agents-dir "$T/agents" --path-value "$T/agentbin"
  need "all four on the agent PATH, none on the caller's: the install goes through (got $rc): $(tail -n 2 "$T/all.err" | tr '\n' ' ')" test "$rc" -eq 0 || return 1
}
ml_28() { # drift guard: every tool cortex-dump itself requires is named in the install tool's prerequisite list
  new_test
  line=$(grep -E '^required_tools=' "$lb" 2> /dev/null | head -n 1 || true)
  need "launchd-backup has a required_tools= line" test -n "$line" || return 1
  for t in flock podman timeout sha256sum; do
    if grep -vE '^[[:space:]]*#' "$here/cortex-dump" | grep -q -w "$t"; then
      need "cortex-dump uses $t, so required_tools names it ($line)" sh -c "printf '%s' '$line' | grep -q -w '$t'" || return 1
    fi
  done
}

# ---- static and portability checks -------------------------------------------------------------------------

ml_21() { # no systemd tool of any kind in the family (the runtime-directory rule of 7 Nov stays), and no deprecated launchctl verb
  new_test
  bad=""
  for f in "$lr" "$lb" "$tpl"/*.plist.in; do
    [ -f "$f" ] || { bad="$bad missing:$f"; continue; }
    hits=$(grep -vE '^[[:space:]]*(#|<!--)' "$f" | grep -E '(^|[;&|({]|then|do|exec|command)[[:space:]]*(systemctl|systemd-run|systemd-analyze|journalctl|loginctl)([[:space:]]|$)' || true)
    bad="$bad$hits"
    old=$(grep -vE '^[[:space:]]*#' "$f" | grep -E '(launchctl|lc)[[:space:]]+(load|unload)([[:space:]]|$)' || true)
    bad="$bad$old"
  done
  need "none found: $bad" test -z "$bad" || return 1
}
ml_25() { # shell syntax under sh and bash --posix, mode 0755, #!/bin/sh
  new_test
  for f in "$lr" "$lb" "$here/launchd-test.sh"; do
    need "$f exists" test -f "$f" || return 1
    need "sh -n $f" sh -n "$f" || return 1
    need "bash --posix -n $f" bash --posix -n "$f" || return 1
    need "$f starts with #!/bin/sh" test "$(head -n 1 "$f")" = "#!/bin/sh" || return 1
  done
  need "mode 0755 on the two tools" sh -c "[ \"\$(stat -c %a '$lr')\" = 755 ] && [ \"\$(stat -c %a '$lb')\" = 755 ]" || return 1
}
ml_26() { # no bashisms in the two tools (dash and shellcheck are not on this VM; this stands in for them)
  new_test
  bad=""
  for f in "$lr" "$lb"; do
    [ -f "$f" ] || { bad="$bad missing:$f"; continue; }
    shell_text=$(grep -vE '^[[:space:]]*#' "$f" | awk '
      skip { if ($0 == tag) skip = 0; next }
      { i = index($0, "<<")
        if (i > 0) {
          rest = substr($0, i + 2)
          sub(/^-/, "", rest); sub(/^[ \t]*/, "", rest)
          q = substr(rest, 1, 1)
          if (q == "\047" || q == "\"") rest = substr(rest, 2)
          if (match(rest, /^[A-Za-z_][A-Za-z0-9_]*/)) { tag = substr(rest, 1, RLENGTH); skip = 1 }
        }
        print }' | awk '
      { line = $0; res = ""
        for (i = 1; i <= length(line); i++) {
          c = substr(line, i, 1)
          if (c == "\047") { inq = !inq; continue }
          if (!inq) res = res c
        }
        print res }')
    hits=$(printf '%s\n' "$shell_text" | grep -E '\[\[|\]\]|pipefail|(^|[;&|({])[[:space:]]*local[[:space:]]|^[[:space:]]*function[[:space:]]|<\(|>\(|echo -e|&>|(^|[;&|({])[[:space:]]*source[[:space:]]|=\(' || true)
    bad="$bad$hits"
  done
  need "no bashism found: $bad" test -z "$bad" || return 1
}

# ---- run -------------------------------------------------------------------------------------------------

all_ids="ML-01 ML-02 ML-03 ML-22 ML-04 ML-05 ML-06 ML-07 ML-08 ML-09 ML-23 ML-24 ML-10 ML-11 ML-12 ML-13 ML-14 ML-15 ML-16 ML-17 ML-18 ML-19 ML-20 ML-29 ML-27 ML-28 ML-30 ML-31 ML-32 ML-33 ML-34 ML-35 ML-36 ML-37 ML-38 ML-39 ML-40 ML-41 ML-42 ML-43 ML-44 ML-45 ML-46 ML-47 ML-48 ML-21 ML-25 ML-26"
[ -n "$want" ] || want=$all_ids
trap cleanup_made EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
pass=0
fail=0
failed=""
for id in $want; do
  fn=$(printf '%s' "$id" | tr 'A-Z-' 'a-z_')
  if ! command -v "$fn" > /dev/null 2>&1; then
    echo "$id MISSING (no such cell)"
    fail=$((fail + 1))
    failed="$failed $id"
    continue
  fi
  if "$fn"; then
    echo "$id PASS"
    pass=$((pass + 1))
  else
    echo "$id FAIL"
    fail=$((fail + 1))
    failed="$failed $id"
  fi
done
if [ "$fail" -eq 0 ]; then
  echo "SUMMARY under '$sut': $pass passed, 0 failed"
  exit 0
fi
echo "SUMMARY under '$sut': $pass passed, $fail failed ($failed )"
exit 1
