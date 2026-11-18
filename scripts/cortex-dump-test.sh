#!/bin/sh
# cortex-dump-test.sh: the tests of piece 12, backup by dump (bob; cox b04b22c9, W1-42).
# Test definitions: mvp/pieces/backup-dump-TEST-DEFINITIONS-r0.md (nix, r0.2); decisions: backup-dump-BUILD-PLAN-bob-r0.md.
#
#   scripts/cortex-dump-test.sh [--shell sh|posix] [--db] [BK-nn | BK-nnD | PORT-nn ...]
#
#   --shell sh      run the scripts under `sh` (default; /bin/sh is bash in POSIX mode on this VM)
#   --shell posix   run the scripts under `bash --posix`
#   --db            the group that needs a REAL throwaway database (ids ending in D): it starts one container,
#                   a source database (named for this run only, label cortexv2=dev, hue's image), applies the S2 pack, and removes the
#                   container by that exact name at the end. It refuses to run unless the database reservation is held:
#                   CV2_DB=1 cv2-admit scripts/cortex-dump-test.sh --db
#   --systemd       BK-04S: the OnFailure seam on a REAL systemd user manager (units named cv2bk-*, in $XDG_RUNTIME_DIR/systemd/user, removed by
#                   their exact paths at the end); it needs a running user manager and CV2_SYSTEMD=1
#   no ids          run every test of the chosen group; with ids, only those
#
# The default group needs NO database and no container: a fake `podman` stands in for `podman exec ... pg_dump` and its
# companions, and logs every call. Run it through cv2-admit: `cv2-admit scripts/cortex-dump-test.sh`.
# Each test builds its own directory under target/bk-test and removes it by its exact name at the end.
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
dump_bin=$here/cortex-dump
status_bin=$here/cortex-dump-status
restore_bin=$here/cortex-restore
compare_bin=$here/restore-compare
alert_bin=$here/cortex-dump-alert
units=$here/../deploy/systemd
base=${CORTEX_BK_TMP:-"$here/../target/bk-test"}
nl='
'

sut=sh
want=""
db=0
sd=0
selftest=""
while [ $# -gt 0 ]; do
  case $1 in
    --shell)
      shift
      case ${1:-} in
        sh) sut=sh ;;
        posix) sut="bash --posix" ;;
        *) echo "cortex-dump-test: --shell takes sh or posix" >&2; exit 2 ;;
      esac
      ;;
    --db) db=1 ;;
    --systemd) sd=1 ;;
    --selftest) shift; selftest=${1:-} ;;
    BK-* | PORT-*) want="$want $1" ;;
    -h | --help) sed -n '2,14p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "cortex-dump-test: unknown argument $1" >&2; exit 2 ;;
  esac
  shift
done
# The APPLICATION'S status variable never enters a test from the caller's environment (BK12-ST-01): with an installed status directory exported
# in the shell, the fake-podman dumps below would otherwise write, or the status reader read, the installed LAST_GOOD. A test that needs the
# variable sets it itself, for that one child (BK-52, BK-53); BK-56 proves the boundary with a nested run.
unset CORTEX_DUMP_STATUS
mkdir -p "$base"
base=$(CDPATH= cd -P -- "$base" && pwd -P) # an absolute path with no . or .. component: cortex-dump refuses those in --out

# ---- helpers ----------------------------------------------------------------------------------------------

need() { # need <message> <test args...>: a failed check names itself and fails the test
  _m=$1
  shift
  if "$@"; then return 0; fi
  printf '    CHECK FAILED: %s\n' "$_m" >&2
  return 1
}

# The fake podman. It logs every call (one line per call) to $FAKE_LOG and behaves as the real commands the tool uses:
#   podman container exists C                        -> exit 0 unless FAKE_NO_CONTAINER=1
#   podman exec --user postgres C psql ...           -> prints FAKE_DB_SIZE (default 1000000)
#   podman exec --user postgres C [timeout S] pg_dump ...   -> a dump, by FAKE_DUMP_MODE: good | fail_half | truncated | slow
#   podman exec -i --user postgres C pg_restore --list      -> reads stdin; exit 0 only for a complete dump
#   podman exec --user postgres C pg_dumpall ...     -> roles, WITH a password hash (a secret the tool must not echo)
fake_podman() {
  cat > "$T/bin/podman" <<'FAKE'
#!/bin/sh
set -u
printf '%s\n' "podman $*" >> "${FAKE_LOG:?}"
[ -n "${PGPASSWORD:-}" ] && printf 'PGPASSWORD-WAS-SET\n' >> "$FAKE_LOG"
case ${1:-} in
  container) [ "${FAKE_NO_CONTAINER:-0}" = 1 ] && exit 1; exit 0 ;;
  exec) shift ;;
  *) echo "fake podman: unexpected call: $*" >&2; exit 97 ;;
esac
while :; do
  case ${1:-} in
    -i) shift ;;
    --user) shift 2 ;;
    *) break ;;
  esac
done
shift # the container name
to=""
if [ "${1:-}" = timeout ]; then to=$2; shift 2; fi
cmd=${1:-}
shift
good_dump() { printf 'PGDMP-FAKE\nTOC-END\nTABLE platform objects\nTABLE records records\nEND-OF-DUMP\n'; }
gen_get() { if [ -f "$FAKE_STATE/gen" ]; then cat "$FAKE_STATE/gen"; else printf '%s' "${FAKE_GEN-}"; fi; }
case $cmd in
  psql)
    [ "${FAKE_PSQL_FAIL:-0}" = 1 ] && { echo "psql: error: fake failure" >&2; exit 2; }
    sql=""
    for a in "$@"; do sql=$a; done
    case $sql in
      *" "*) ;;
      *)
        # no SQL string as the last argument: a script on stdin (the role step of cortex-restore)
        body=$(cat)
        if [ -n "${FAKE_STATE:-}" ]; then mkdir -p "$FAKE_STATE"; printf '%s\n' "$body" > "$FAKE_STATE/stdin.sql"; fi
        if [ "${FAKE_ROLES_HANG:-0}" = 1 ]; then echo $$ > "$FAKE_STATE/hang.pid"; : > "$FAKE_STATE/roles-ready"; sleep "${FAKE_ROLES_HANG_SECS:-3}"; fi
        exit 0 ;;
    esac
    case $sql in
      *pg_database_size*) printf '%s\n' "${FAKE_DB_SIZE:-1000000}" ;;
      *pg_hba_file_rules*) printf '%s\n' "${FAKE_HBA_COUNT-0}" ;;
      *"datallowconn from pg_database"*) printf '%s\n' "${FAKE_DBS:-postgres}" | tr ',' '\n' | awk -F: '{ print $1 "|" ($2 == "f" ? "f" : "t") }' ;;
      *pg_database*) printf '%s\n' "${FAKE_DBS:-postgres}" | tr ',' '\n' | awk -F: '$2 != "f" { print $1 }' ;;
      *pg_class*) printf '%s\n' "${FAKE_USER_TABLES:-0}" ;;
      *pg_roles*) printf 'postgres\n' ;;
      "create database"*) ;;
      *"DROP DATABASE"*) ;;
      *"from identity.installations"*) if [ -n "${FAKE_INST-}" ]; then printf '%s\n' "$FAKE_INST"; fi ;;
      *"count(*) from tokens.tokens"*) printf '%s\n' "${FAKE_NTOK:-0}" ;;
      "begin; set local role cortex_install_admin"*)
        cur=$(gen_get)
        mkdir -p "$FAKE_STATE"
        if [ -z "$cur" ]; then echo 1 > "$FAKE_STATE/gen"; else echo $((cur + 1)) > "$FAKE_STATE/gen"; fi ;;
      *"gen from tokens.generations"*) g=$(gen_get); if [ -n "$g" ]; then printf '%s\n' "$g"; fi ;;
      *) echo "fake psql: unexpected sql" >&2; exit 96 ;;
    esac ;;
  pg_dumpall)
    printf 'CREATE ROLE cortex_app LOGIN PASSWORD '"'"'SCRAM-SHA-256$4096:FAKEHASH-CANARY-7731'"'"';\n'
    if [ "${FAKE_DUMPALL_FAIL:-0}" = 1 ]; then echo "pg_dumpall: error: fake failure" >&2; exit 1; fi ;;
  pg_restore)
    body=$(cat)
    case " $* " in
      *" --list "*)
        # like the real one: only the table of contents is read
        case $body in *TOC-END*) printf '; Archive created\nTABLE platform objects\n'; exit 0 ;; esac
        echo "pg_restore: error: unexpected end of file" >&2; exit 1 ;;
      *" --file"*)
        # a full read of every data block
        if [ "${FAKE_FULLREAD_HANG:-0}" = 1 ]; then echo $$ > "$FAKE_STATE/fullread.pid"; : > "$FAKE_STATE/fullread-ready"; sleep "${FAKE_FULLREAD_SECS:-3}"; fi
        case $body in *END-OF-DUMP*) exit 0 ;; esac
        echo "pg_restore: error: could not read data block (fake)" >&2; exit 1 ;;
      *" -d "*) exit 0 ;;
    esac
    echo "fake pg_restore: unexpected arguments" >&2; exit 98 ;;
  pg_dump)
    case ${FAKE_DUMP_MODE:-good} in
      good) good_dump ;;
      fail_half) printf 'PGDMP-FAKE\npartial'; exit 1 ;;
      truncated) printf 'PGDMP-FAKE\npartial'; exit 0 ;;
      truncated_after_toc) printf 'PGDMP-FAKE\nTOC-END\nTABLE platform objects\npartial'; exit 0 ;;
      slow)
        # the worker is the one process that sleeps; timeout, if given, signals it by its own pid
        if [ -n "$to" ]; then
          timeout "$to" sh -c 'echo $$ > "$1"; exec sleep "$2"' sh "${FAKE_WORKER_PID:?}" "${FAKE_SLOW_SECS:-60}" || exit $?
        else
          sh -c 'echo $$ > "$1"; exec sleep "$2"' sh "${FAKE_WORKER_PID:?}" "${FAKE_SLOW_SECS:-60}" || exit $?
        fi
        good_dump ;;
    esac ;;
  *) echo "fake podman: unexpected command $cmd" >&2; exit 98 ;;
esac
FAKE
  chmod 0755 "$T/bin/podman"
}

sdflag=1 # 1: the dump helpers add --status-dir "$T/status"; a test that must run WITHOUT one sets 0
new_test() {
  T=$(mktemp -d "$base/t.XXXXXX")
  mkdir "$T/bin" "$T/status"
  sdflag=1
  : > "$T/podman.log"
  fake_podman
  DENV=""
}

end_test() {
  case $T in "$base"/t.*) rm -rf -- "$T" ;; esac
}

# run <name> <command...>: capture stdout and stderr in $T/<name>.out and .err, the exit status in rc
run() {
  _n=$1
  shift
  rc=0
  "$@" > "$T/$_n.out" 2> "$T/$_n.err" || rc=$?
}

# dump <name> [args]: cortex-dump against the fake, into $T/out; extra environment goes in $DENV
dump() {
  _n=$1
  shift
  if [ "$sdflag" = 1 ]; then set -- --status-dir "$T/status" "$@"; fi # a later --status-dir of the test wins
  # shellcheck disable=SC2086
  run "$_n" env PATH="$T/bin:$PATH" FAKE_LOG="$T/podman.log" $DENV $sut "$dump_bin" --target fakedb --out "$T/out" "$@"
}

# status <name> [args]
status() {
  _n=$1
  shift
  # shellcheck disable=SC2086
  run "$_n" env PATH="$T/bin:$PATH" $DENV $sut "$status_bin" --status-dir "$T/status" --out "$T/out" "$@"
}

# restore <name> [args]: cortex-restore against the fake
restore() {
  _n=$1
  shift
  # shellcheck disable=SC2086
  run "$_n" env PATH="$T/bin:$PATH" FAKE_LOG="$T/podman.log" TMPDIR="$T" $DENV $sut "$restore_bin" "$@" < /dev/null
}

make_set() { # a set that verifies, in $T/set
  mkdir -p "$T/set"
  printf 'PGDMP-FAKE\nTABLE platform objects\nEND-OF-DUMP\n' > "$T/set/platform.dump"
  printf 'CREATE ROLE cortex_app;\n' > "$T/set/globals.sql"
  (cd "$T/set" && sha256sum platform.dump globals.sql > SHA256SUMS)
}

no_writes_logged() { # nothing that writes was asked of any container
  test -z "$(grep -E 'pg_restore|create database|CREATE ROLE|ALTER ROLE|DROP DATABASE|globals' "$T/podman.log" || true)"
}

good_sets() { ls -1 "$T/out" 2> /dev/null | grep -E '^[0-9]{8}T[0-9]{6}Z(-[0-9]+)?$' || true; }
count_good_sets() { good_sets | grep -c . || true; }
labelled_sets() { ls -1 "$T/out/pre-upgrade" 2> /dev/null | grep -E '^[0-9]{8}T[0-9]{6}Z' || true; }
count_labelled_sets() { labelled_sets | grep -c . || true; }
last_good_set() { sed -n 's/.*"set":"\([^"]*\)".*/\1/p' "$T/status/LAST_GOOD" 2> /dev/null || true; }
last_line() { tail -n 1 "$1"; }
set_ok() { (cd "$1" && sha256sum -c SHA256SUMS > /dev/null 2>&1 && [ -s platform.dump ] && [ -s globals.sql ]); }
temp_left() { ls -A "$T/out" 2> /dev/null | grep -E '^\.tmp-' || true; }
sum_of() { sha256sum "$1" | cut -d' ' -f1; }

# ---- the tests --------------------------------------------------------------------------------------------

bk_02() { # a dump that fails half way leaves no set that looks good
  new_test
  dump a
  need "the first run is good (rc=$rc)" test "$rc" -eq 0 || return 1
  first=$(last_good_set)
  DENV="FAKE_DUMP_MODE=fail_half" dump b
  need "the failing run exits non-zero (rc=$rc)" test "$rc" -ne 0 || return 1
  need "still exactly one set" test "$(count_good_sets)" -eq 1 || return 1
  need "no temp directory left behind" test -z "$(temp_left)" || return 1
  need "LAST_GOOD still names the first set" test "$(last_good_set)" = "$first" || return 1
  need "the first set is intact" set_ok "$T/out/$first" || return 1
  # pg_dumpall fails after it printed (a partial globals.sql is not a set)
  DENV="FAKE_DUMPALL_FAIL=1" dump c
  need "pg_dumpall failing: exit 1 (got $rc)" test "$rc" -eq 1 || return 1
  need "pg_dumpall failing: still exactly one set" test "$(count_good_sets)" -eq 1 || return 1
  need "pg_dumpall failing: no temp directory left behind" test -z "$(temp_left)" || return 1
  need "pg_dumpall failing: LAST_GOOD still names the first set" test "$(last_good_set)" = "$first" || return 1
}

bk_03() { # a dump that exits 0 but is truncated is caught by the pg_restore --list check
  new_test
  dump a
  need "the first run is good (rc=$rc)" test "$rc" -eq 0 || return 1
  first=$(last_good_set)
  DENV="FAKE_DUMP_MODE=truncated" dump b
  need "the truncated dump fails the run (rc=$rc)" test "$rc" -eq 1 || return 1
  need "still exactly one set" test "$(count_good_sets)" -eq 1 || return 1
  need "LAST_GOOD still names the first set" test "$(last_good_set)" = "$first" || return 1
  need "the check ran (pg_restore --list was called)" grep -q 'pg_restore --list' "$T/podman.log" || return 1
}

write_last_good() { # write_last_good <utc>
  printf '{"set":"20261106T000000Z","utc":"%s","bytes":123,"sha256":"%s"}\n' "$1" \
    "0000000000000000000000000000000000000000000000000000000000000000" > "$T/status/LAST_GOOD"
}

bk_06() { # cortex-dump-status: the age, with an injected clock, and every way LAST_GOOD can be wrong
  new_test
  mkdir -p "$T/out"
  DENV="CORTEX_DUMP_NOW=2026-11-06T12:00:00Z"
  # (utc, expected exit)
  for case_ in "2026-11-06T11:00:00Z 0" "2026-11-05T10:06:00Z 0" "2026-11-05T09:54:00Z 1"; do
    set -- $case_
    write_last_good "$1"
    status s
    need "LAST_GOOD $1: exit $2 (got $rc)" test "$rc" -eq "$2" || return 1
    need "LAST_GOOD $1: one line" test "$(wc -l < "$T/s.out")" -eq 1 || return 1
  done
  rm -f "$T/status/LAST_GOOD"
  status s
  need "missing: exit 1 (got $rc)" test "$rc" -eq 1 || return 1
  : > "$T/status/LAST_GOOD"
  status s
  need "empty: exit 1 (got $rc)" test "$rc" -eq 1 || return 1
  printf 'CANARY-INVALID-9f3a {not json' > "$T/status/LAST_GOOD"
  status s
  need "invalid: exit 1 (got $rc)" test "$rc" -eq 1 || return 1
  need "the file's own words are never echoed" test -z "$(grep -h 'CANARY-INVALID' "$T/s.out" "$T/s.err" || true)" || return 1
  printf '{"set":"x","utc":"2026-11-06T11:00:00","bytes":1,"sha256":"y"}\n' > "$T/status/LAST_GOOD"
  status s
  need "a time without the Z: exit 1 (got $rc)" test "$rc" -eq 1 || return 1
  write_last_good "2026-11-06T11:00:00Z"
  chmod 000 "$T/status/LAST_GOOD"
  status s
  chmod 600 "$T/status/LAST_GOOD"
  need "unreadable: exit 1 (got $rc)" test "$rc" -eq 1 || return 1
  rm -f "$T/status/LAST_GOOD"
  write_last_good "2026-11-06T11:00:00Z"
  mv "$T/status/LAST_GOOD" "$T/real"
  ln -s "$T/real" "$T/status/LAST_GOOD"
  status s
  need "a symlink is never followed: exit 1 (got $rc)" test "$rc" -eq 1 || return 1
  rm -f "$T/status/LAST_GOOD"
  { printf '{"set":"x","utc":"2026-11-06T11:00:00Z","bytes":1,"sha256":"'; head -c 5000 /dev/zero | tr '\0' 'a'; printf '"}\n'; } > "$T/status/LAST_GOOD"
  status s
  need "a file over 4096 bytes: exit 1 (got $rc)" test "$rc" -eq 1 || return 1
  need "and the reason is the size, not the format: $(cat "$T/s.out")" grep -q 'too large' "$T/s.out" || return 1
  rm -f "$T/status/LAST_GOOD"
  write_last_good "2026-11-06T14:00:00Z"
  status s
  need "a time two hours in the future: exit 1 (got $rc)" test "$rc" -eq 1 || return 1
  need "and the reason says so: $(cat "$T/s.out")" grep -q 'in the future' "$T/s.out" || return 1
  write_last_good "2026-11-06T12:03:00Z"
  status s
  need "a time three minutes ahead (clock skew) is tolerated: exit 0 (got $rc)" test "$rc" -eq 0 || return 1
}

bk_07() { # corruption after a good run is seen by --verify
  new_test
  dump a
  need "a good run (rc=$rc)" test "$rc" -eq 0 || return 1
  set_=$(last_good_set)
  status v --verify
  need "an intact set verifies (rc=$rc)" test "$rc" -eq 0 || return 1
  printf 'X' | dd of="$T/out/$set_/platform.dump" bs=1 seek=3 conv=notrunc 2> /dev/null
  status v --verify
  need "one flipped byte: exit 1 (got $rc)" test "$rc" -eq 1 || return 1
  need "the reason says checksum" grep -q 'checksum' "$T/v.out" || return 1
  # the files and their SHA256SUMS rewritten TOGETHER pass sha256sum -c, but it is no longer the set LAST_GOOD recorded
  (cd "$T/out/$set_" && sha256sum platform.dump globals.sql > SHA256SUMS)
  status v2 --verify
  need "a consistently rewritten set: exit 1 (got $rc)" test "$rc" -eq 1 || return 1
  need "the reason says it is not the one LAST_GOOD names: $(cat "$T/v2.out")" grep -q 'LAST_GOOD names' "$T/v2.out" || return 1
}

bk_09() { # retention: failing runs delete nothing; the three newest good sets stay
  new_test
  for i in 1 2 3 4 5; do
    dump r$i
    need "good run $i (rc=$rc)" test "$rc" -eq 0 || return 1
  done
  need "three sets after five good runs" test "$(count_good_sets)" -eq 3 || return 1
  keep_before=$(good_sets)
  DENV="FAKE_DUMP_MODE=fail_half"
  for i in 1 2 3; do
    dump f$i
    need "failing run $i exits non-zero" test "$rc" -ne 0 || return 1
  done
  need "failing runs delete nothing" test "$(good_sets)" = "$keep_before" || return 1
  DENV=""
  dump g
  need "the last good run (rc=$rc)" test "$rc" -eq 0 || return 1
  need "exactly three sets after it" test "$(count_good_sets)" -eq 3 || return 1
  newest=$(good_sets | tail -n 1)
  need "the newest set is the new one and verifies" set_ok "$T/out/$newest" || return 1
  need "LAST_GOOD names the newest" test "$(last_good_set)" = "$newest" || return 1
  # a clock that went backwards: three sets dated 2099 sort after the new one; retention must not remove the set just written
  for n in 20990101T000000Z 20990102T000000Z 20990103T000000Z; do cp -rp "$T/out/$newest" "$T/out/$n"; done
  dump h
  need "a run after the clock went backwards (rc=$rc)" test "$rc" -eq 0 || return 1
  hset=$(last_good_set)
  need "the set just written still exists and verifies ($hset)" set_ok "$T/out/$hset" || return 1
  need "three sets remain" test "$(count_good_sets)" -eq 3 || return 1
}

bk_10() { # the output directory guard
  new_test
  mkdir "$T/keep"
  : > "$T/keep/file"
  for o in "" "/" "$HOME"; do
    run g env PATH="$T/bin:$PATH" FAKE_LOG="$T/podman.log" $sut "$dump_bin" --target fakedb --out "$o" --status-dir "$T/status"
    need "--out '$o' refused with exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  done
  run g env PATH="$T/bin:$PATH" FAKE_LOG="$T/podman.log" $sut "$dump_bin" --target fakedb
  need "no --out at all: exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  need "nothing was started" test -z "$(grep 'pg_dump' "$T/podman.log" || true)" || return 1
  need "the sentinel file is untouched" test -f "$T/keep/file" || return 1
}

bk_11() { # permissions, whatever the caller's umask; no secret on stdout or stderr
  new_test
  old_umask=$(umask)
  umask 022
  dump a
  umask "$old_umask"
  need "a good run (rc=$rc)" test "$rc" -eq 0 || return 1
  s=$(last_good_set)
  need "set directory is 0700" test "$(stat -c %a "$T/out/$s")" = 700 || return 1
  for f in platform.dump globals.sql SHA256SUMS; do
    need "$f is 0600" test "$(stat -c %a "$T/out/$s/$f")" = 600 || return 1
  done
  need "LAST_GOOD is 0644 (the server reads the status directory through a read-only mount)" test "$(stat -c %a "$T/status/LAST_GOOD")" = 644 || return 1
  need "the output directory is 0700 (the dump sets are private; no mount sees them)" test "$(stat -c %a "$T/out")" = 700 || return 1
  need "no password hash on stdout or stderr" test -z "$(grep -h 'FAKEHASH-CANARY\|SCRAM-SHA-256' "$T/a.out" "$T/a.err" || true)" || return 1
}

bk_12() { # (superseded by W1-42) no secret in argv, environment or logs; no PGPASSWORD reaches a child
  new_test
  DENV="PGPASSWORD=canary-pw-1"
  dump a
  need "a good run (rc=$rc)" test "$rc" -eq 0 || return 1
  need "PGPASSWORD never reached podman" test -z "$(grep 'PGPASSWORD' "$T/podman.log" || true)" || return 1
  need "no password in any podman argument" test -z "$(grep -i 'password' "$T/podman.log" || true)" || return 1
}

bk_20() { # a labelled dump: the set path is the last stdout line and lives under pre-upgrade/
  new_test
  dump d
  need "a daily run first (rc=$rc)" test "$rc" -eq 0 || return 1
  lg_before=$(sum_of "$T/status/LAST_GOOD")
  dump l --label pre-upgrade-0.1.0-to-0.2.0
  need "the labelled run is good (rc=$rc)" test "$rc" -eq 0 || return 1
  p=$(last_line "$T/l.out")
  need "the last stdout line is an absolute path ($p)" test "${p#/}" != "$p" || return 1
  need "that path is a set under pre-upgrade/" test "$(dirname "$p")" = "$T/out/pre-upgrade" || return 1
  need "the set verifies" set_ok "$p" || return 1
  need "the set is not in the daily area" test "$(count_good_sets)" -eq 1 || return 1
  need "a labelled run leaves LAST_GOOD alone (plan decision 2)" test "$(sum_of "$T/status/LAST_GOOD")" = "$lg_before" || return 1
}

bk_21() { # every failure, with a label: no set, nothing else touched, never exit 0
  new_test
  dump d
  need "a daily run first (rc=$rc)" test "$rc" -eq 0 || return 1
  lg_before=$(sum_of "$T/status/LAST_GOOD")
  for mode in fail_half truncated; do
    DENV="FAKE_DUMP_MODE=$mode" dump l --label pre-upgrade-a-to-b
    need "$mode: exit 1 (got $rc)" test "$rc" -eq 1 || return 1
    need "$mode: no labelled set" test "$(count_labelled_sets)" -eq 0 || return 1
  done
  need "the daily set and LAST_GOOD are untouched" test "$(sum_of "$T/status/LAST_GOOD")" = "$lg_before" || return 1
  need "no temp directory left" test -z "$(temp_left)" || return 1
}

bk_22() { # disk preflight: not enough free space is exit 2 before pg_dump starts
  new_test
  DENV="FAKE_DB_SIZE=1000000000000000"
  dump p
  need "exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  need "pg_dump never started" test -z "$(grep 'pg_dump' "$T/podman.log" || true)" || return 1
  need "nothing written" test "$(count_good_sets)" -eq 0 || return 1
  need "no temp directory" test -z "$(temp_left)" || return 1
  need "one line of reason on stderr" test "$(wc -l < "$T/p.err")" -ge 1 || return 1
}

wait_for() { # wait_for <path>: up to 10 s
  _i=0
  while [ ! -e "$1" ] && [ "$_i" -lt 100 ]; do
    sleep 0.1
    _i=$((_i + 1))
  done
  [ -e "$1" ]
}

bk_23() { # two dumps at once: one runs, the other exits 3 at once
  new_test
  DENV="FAKE_DUMP_MODE=slow FAKE_SLOW_SECS=3 FAKE_WORKER_PID=$T/worker.pid"
  (dump first; exit "$rc") &
  bg=$!
  need "the first run took the worker" wait_for "$T/worker.pid" || return 1
  DENV=""
  t0=$(date +%s)
  dump second
  t1=$(date +%s)
  need "the second run exits 3 (got $rc)" test "$rc" -eq 3 || return 1
  need "and at once (under 3 s)" test $((t1 - t0)) -lt 3 || return 1
  wb=0
  wait "$bg" || wb=$?
  need "the first run is good (rc=$wb)" test "$wb" -eq 0 || return 1
  need "exactly one set, intact" test "$(count_good_sets)" -eq 1 || return 1
  need "it verifies" set_ok "$T/out/$(last_good_set)" || return 1
}

bk_24() { # --timeout: exit 4, the worker ends by its own pid, partial files and the lock are gone
  new_test
  DENV="FAKE_DUMP_MODE=slow FAKE_SLOW_SECS=60 FAKE_WORKER_PID=$T/worker.pid"
  t0=$(date +%s)
  dump t --timeout 5
  t1=$(date +%s)
  need "exit 4 (got $rc)" test "$rc" -eq 4 || return 1
  need "within 15 s ($((t1 - t0)) s)" test $((t1 - t0)) -lt 15 || return 1
  need "the worker pid was recorded" test -s "$T/worker.pid" || return 1
  wp=$(cat "$T/worker.pid")
  need "the worker ($wp) is gone" sh -c "! kill -0 $wp 2> /dev/null" || return 1
  need "no temp directory" test -z "$(temp_left)" || return 1
  need "the lock is released" test ! -e "$T/out/.lock" || return 1
  need "and nobody holds it" lock_free "$T/out" || return 1
  need "no set" test "$(count_good_sets)" -eq 0 || return 1
}

bk_25() { # bad labels: exit 2, nothing outside the output directory, no shell interpretation
  new_test
  mkdir -p "$T/out"
  long=$(head -c 200 /dev/zero | tr '\0' 'a')
  for lab in "" "$long" "../x" "a b" ";ls" "a${nl}b" '$(touch /tmp/never)' '`id`'; do
    dump b --label "$lab"
    need "label '$lab': exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  done
  need "nothing escaped the output directory" test ! -e "$T/x" || return 1
  need "nothing was created in it either" test -z "$(ls -A "$T/out")" || return 1
  need "no pg_dump" test -z "$(grep 'pg_dump' "$T/podman.log" || true)" || return 1
}

bk_26() { # five labelled runs keep three; the daily sets are not touched
  new_test
  dump d1
  dump d2
  daily=$(good_sets)
  for i in 1 2 3 4 5; do
    dump l$i --label pre-upgrade-0.$i-to-0.9
    need "labelled run $i (rc=$rc)" test "$rc" -eq 0 || return 1
  done
  need "three labelled sets remain" test "$(count_labelled_sets)" -eq 3 || return 1
  need "the daily sets are untouched" test "$(good_sets)" = "$daily" || return 1
  dump d3
  need "a daily run does not prune the labelled sets" test "$(count_labelled_sets)" -eq 3 || return 1
}

bk_29() { # peer authentication only: the dump and status scripts have no TCP and no password
  new_test
  dump a
  need "a good run (rc=$rc)" test "$rc" -eq 0 || return 1
  need "every exec runs as the postgres OS user" test -z "$(grep ' exec ' "$T/podman.log" | grep -v -- '--user postgres' || true)" || return 1
  code=$(grep -vE '^[[:space:]]*#' "$dump_bin" "$status_bin" "$restore_bin" "$compare_bin" "$alert_bin")
  need "no localhost, 127.0.0.1 or password variable in the scripts" test -z "$(printf '%s\n' "$code" | grep -E 'localhost|127\.0\.0\.1|PGPASSWORD' || true)" || return 1
  need "no -h <host> or --host on any psql or pg_* command line" test -z "$(printf '%s\n' "$code" | grep -E 'psql|pg_dump|pg_restore|pg_dumpall' | grep -E -- ' -h |--host' || true)" || return 1
}

bk_37() { # the fixed warning is the last line of stderr; the set path is the last line of stdout
  new_test
  dump d
  need "a good run (rc=$rc)" test "$rc" -eq 0 || return 1
  w='restore ONLY with cortex-restore; a plain pg_restore can make revoked tokens valid again'
  need "daily: the last stderr line is the warning, byte for byte" test "$(last_line "$T/d.err")" = "$w" || return 1
  need "daily: the last stdout line is the set path" test "$(last_line "$T/d.out")" = "$T/out/$(last_good_set)" || return 1
  dump l --label pre-upgrade-a-to-b
  need "labelled: the last stderr line is the warning" test "$(last_line "$T/l.err")" = "$w" || return 1
}

bk_18() { # static check of the scripts: nothing that kills by name, wipes, or cleans
  new_test
  forbidden='\b(pkill|killall)\b|kill -9|kill -KILL|kill -- -|rm -f -a|podman rm -a|podman rm --all|podman system prune|--clean|--no-acl|--no-owner'   # s5-never: ok the words this static check searches FOR; they are data for grep, never run
  bad=$(grep -nE "$forbidden" "$dump_bin" "$status_bin" 2> /dev/null | grep -vE ':[0-9]+:[[:space:]]*#' || true)
  need "no forbidden command in the scripts: $bad" test -z "$bad" || return 1
  livenames='cortex-pg|spike-s0-|harness-appdb|8501|5499'   # s5-never: ok the live-service names this static check searches FOR; data for grep, never run
  names=$(grep -nE "$livenames" "$dump_bin" "$status_bin" 2> /dev/null | grep -vE ':[0-9]+:[[:space:]]*#' || true)
  need "no live-service name in the dump or status scripts: $names" test -z "$names" || return 1
  need "cortex-dump exists and is not a stub" sh -c "! grep -q 'not built yet' '$dump_bin'" || return 1
}

port_01() { # shell syntax under sh and under bash --posix
  new_test
  for f in "$dump_bin" "$status_bin" "$restore_bin" "$compare_bin" "$alert_bin" "$here/cortex-dump-test-analyze.sh" "$here/cortex-dump-test.sh"; do
    need "sh -n $f" sh -n "$f" || return 1
    need "bash --posix -n $f" bash --posix -n "$f" || return 1
  done
  need "mode 0755 on the five scripts" sh -c "[ \"\$(stat -c %a '$dump_bin')\" = 755 ] && [ \"\$(stat -c %a '$status_bin')\" = 755 ] && [ \"\$(stat -c %a '$restore_bin')\" = 755 ] && [ \"\$(stat -c %a '$compare_bin')\" = 755 ] && [ \"\$(stat -c %a '$alert_bin')\" = 755 ]" || return 1
  for f in "$dump_bin" "$status_bin" "$restore_bin" "$compare_bin" "$alert_bin"; do
    need "$f starts with #!/bin/sh" test "$(head -n 1 "$f")" = "#!/bin/sh" || return 1
  done
}

port_02() { # no bashisms (dash and shellcheck are not on this VM; this is the check that stands in for them)
  new_test
  bad=""
  for f in "$dump_bin" "$status_bin" "$restore_bin" "$compare_bin" "$alert_bin" "$here/cortex-dump-test-analyze.sh"; do
    # drop comment lines, then everything inside single quotes (awk programs, regexes, printf formats can hold any word)
    shell_text=$(grep -vE '^[[:space:]]*#' "$f" | awk '
      # a here-document body is data (an awk program, SQL), not shell: skip it up to its terminator
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
    hits=$(printf '%s\n' "$shell_text" | grep -E '\[\[|\]\]|pipefail|(^|[;&|({])[[:space:]]*local[[:space:]]|^[[:space:]]*function[[:space:]]|<\(|>\(|echo -e|&>|(^|[;&|({])[[:space:]]*source[[:space:]]|=\(|\$\{[A-Za-z_]+(//|,,|\^\^|:[0-9])|^[[:space:]]*select [A-Za-z_]+ in|==' || true)
    bad="$bad$hits"
  done
  need "no bashism found: $bad" test -z "$bad" || return 1
}

bk_18r() { # static check of cortex-restore and restore-compare: no forbidden command; ONE removal of a database, by the exact created name
  new_test
  forbidden='\b(pkill|killall)\b|kill -9|kill -KILL|kill -- -|rm -f -a|podman rm -a|podman rm --all|podman system prune|--clean|--no-acl|--no-owner'   # s5-never: ok the words this static check searches FOR; they are data for grep, never run
  bad=$(grep -nE "$forbidden" "$restore_bin" "$compare_bin" "$alert_bin" 2> /dev/null | grep -vE ':[0-9]+:[[:space:]]*#' || true)
  need "no forbidden command in the restore scripts: $bad" test -z "$bad" || return 1
  livenames='cortex-pg|spike-s0-|harness-appdb|platform_agent_memory|8501|5499'   # s5-never: ok the live-service names this static check searches FOR; data for grep, never run
  names=$(grep -nE "$livenames" "$restore_bin" "$compare_bin" 2> /dev/null |
    grep -vE ':[0-9]+:[[:space:]]*#' | grep -v ') die 2 "the target is a live or reserved database' || true)
  need "no live-service name outside the one refusal list: $names" test -z "$names" || return 1
  need "the removal of a database is in exactly one place (got $(grep -ci 'drop database' "$restore_bin" || true))" test "$(grep -ci 'drop database' "$restore_bin" || true)" -eq 1 || return 1
  need "and it names the database this run created, not a variable from outside" grep -q 'DROP DATABASE \\"\$db\\" WITH (FORCE)' "$restore_bin" || return 1
  need "the archive is loaded in one transaction and stops at the first error" sh -c "grep -q -- '--single-transaction --exit-on-error' '$restore_bin'" || return 1
  need "the compare tool has no write: no insert, update, delete, drop, alter, grant, revoke, truncate in its SQL" test -z "$(grep -niE '^[[:space:]]*(insert|update|delete|drop|alter|grant|revoke|truncate|create) ' "$compare_bin" | grep -v 'create or' || true)" || return 1
  need "cortex-restore and restore-compare are not stubs" sh -c "! grep -q 'not built yet' '$restore_bin' && grep -q 'RESULT' '$compare_bin'" || return 1
}

bk_13() { # restore guards: every one refused, exit 2, nothing written
  new_test
  make_set
  for name in cortex-pg platform_agent_memory harness-appdb; do   # s5-never: ok these are the names cortex-restore must REFUSE; they are test inputs to the fake podman, never run
    restore r --set "$T/set" --target "$name" --confirm-new
    need "$name is refused with exit 2 (got $rc)" test "$rc" -eq 2 || return 1
    need "nothing was run in $name" test -z "$(grep ' exec ' "$T/podman.log" || true)" || return 1
  done
  restore r --set "$T/set" --target fakedb
  need "no --confirm-new: exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  DENV="FAKE_NO_CONTAINER=1" restore r --set "$T/set" --target fakedb --confirm-new
  need "a missing container: exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  DENV="FAKE_USER_TABLES=3" restore r --set "$T/set" --target fakedb --confirm-new
  need "a target with user tables: exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  DENV="FAKE_DBS=postgres,cortex" restore r --set "$T/set" --target fakedb --confirm-new
  need "a database that already exists: exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  need "nothing that writes was asked of any container" no_writes_logged || return 1
  rm -f "$T/set/SHA256SUMS"
  restore r --set "$T/set" --target fakedb --confirm-new
  need "a set without SHA256SUMS: exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  make_set
  printf 'X' >> "$T/set/platform.dump"
  restore r --set "$T/set" --target fakedb --confirm-new
  need "a set that does not verify: exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  make_set
  for opt in --clean --drop --no-acl --no-owner --create --force; do
    restore r --set "$T/set" --target fakedb --confirm-new "$opt"
    need "$opt is unknown: exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  done
  need "nothing that writes was asked of any container" no_writes_logged || return 1
}

bk_28() { # the target's EFFECTIVE rules are read first; a trusting, empty, odd or unreadable answer refuses
  new_test
  make_set
  for bad in 2 "" abc; do
    DENV="FAKE_HBA_COUNT=$bad" restore r --set "$T/set" --target fakedb --confirm-new
    need "hba count '$bad': exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  done
  DENV="FAKE_PSQL_FAIL=1" restore r --set "$T/set" --target fakedb --confirm-new
  need "the query cannot run: exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  need "nothing that writes was asked of any container" no_writes_logged || return 1
  need "the query is the one install.sh uses" grep -q "pg_hba_file_rules where auth_method = 'trust' or error is not null" "$T/podman.log" || return 1
}

bk_33a() { # the raise cannot be skipped: no option, no variable, no word in the script
  new_test
  make_set
  for opt in --skip-generation --no-generation --no-bump; do
    restore r --set "$T/set" --target fakedb --confirm-new "$opt"
    need "$opt is unknown: exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  done
  need "nothing that writes was asked of any container" no_writes_logged || return 1
  code=$(grep -vE '^[[:space:]]*#' "$restore_bin")
  need "no skip path in the script" test -z "$(printf '%s\n' "$code" | grep -iE 'skip|no-generation|no-bump|SKIP_GENERATION' || true)" || return 1
  need "cortex-restore exists and is not a stub" sh -c "! grep -q 'not built yet' '$restore_bin'" || return 1
}

bk_37r() { # the warning is in --help, where a human reads it
  new_test
  restore h --help
  need "--help exits 0 (got $rc)" test "$rc" -eq 0 || return 1
  need "--help carries the fixed warning" grep -qF 'restore ONLY with cortex-restore; a plain pg_restore can make revoked tokens valid again' "$T/h.out" || return 1
}

# ---- the alarm and the units (BK-04, BK-05, BK-19) ----------------------------------------------------------

# alert <name> [args]: cortex-dump-alert as systemd starts it (a clean environment plus what $DENV adds)
alert() {
  _n=$1
  shift
  # shellcheck disable=SC2086
  run "$_n" env -i PATH="$PATH" $DENV $sut "$alert_bin" "$@"
}
alarm_re='^\{"source":"backup","utc":"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z","unit":"[A-Za-z0-9@:_.-]+","status":"[0-9?]+","reason":"[A-Za-z0-9 :.,()-]+","state":"[A-Za-z0-9 :.,()+-]+"\}$'

bk_04() { # the OnFailure seam: a failed unit leaves a journal line and the ALARM marker, never a secret, whatever systemd hands over
  new_test
  need "cortex-dump-alert exists and is not a stub" sh -c "! grep -q 'not built yet' '$alert_bin'" || return 1
  home=$T/home
  mkdir -p "$home/dumps"
  DENV="MONITOR_UNIT=cortex-dump.service MONITOR_SERVICE_RESULT=exit-code MONITOR_EXIT_CODE=exited MONITOR_EXIT_STATUS=4 PGPASSWORD=bk-secret-canary-4471 CORTEX_TOKEN=bk-token-canary-9902"
  alert a --home "$home"
  need "exit 0 (got $rc): $(tail -n 2 "$T/a.err")" test "$rc" -eq 0 || return 1
  need "the marker is a regular file, mode 644" test "$(stat -c '%F %a' "$home/ALARM" 2> /dev/null)" = "regular file 644" || return 1
  need "the marker is ONE line of the recorded shape: $(cat "$home/ALARM" 2> /dev/null)" sh -c "[ \$(wc -l < '$home/ALARM') -eq 1 ] && grep -Eq '$alarm_re' '$home/ALARM'" || return 1
  need "it names the unit, the exit status and the reason" sh -c "grep -q '\"unit\":\"cortex-dump.service\"' '$home/ALARM' && grep -q '\"status\":\"4\"' '$home/ALARM' && grep -q '\"reason\":\"timed out\"' '$home/ALARM'" || return 1
  need "the journal line (stdout or stderr) says BACKUP ALARM with the unit and the reason" sh -c "cat '$T/a.out' '$T/a.err' | grep -q 'BACKUP ALARM.*cortex-dump.service.*timed out'" || return 1
  need "no secret reached the marker, stdout or stderr" test -z "$(grep -h -e canary "$home/ALARM" "$T/a.out" "$T/a.err" || true)" || return 1
  # every exit status the dump can leave has its own words
  for pair in "1:dump or check failed" "2:refused" "3:another dump was running" "4:timed out"; do
    DENV="MONITOR_UNIT=cortex-dump.service MONITOR_EXIT_STATUS=${pair%%:*}"
    alert m --home "$home"
    need "status ${pair%%:*} is reported as '${pair#*:}' (rc=$rc)" grep -q "\"reason\":\"${pair#*:}\"" "$home/ALARM" || return 1
  done
  DENV="MONITOR_UNIT=cortex-dump-status.service MONITOR_EXIT_STATUS=1"
  alert s --home "$home"
  need "the freshness check failing is reported as stale or untrustworthy" grep -q '"reason":"last good dump is stale or untrustworthy"' "$home/ALARM" || return 1
  # what systemd hands over is validated, never trusted
  run inj env -i PATH="$PATH" 'MONITOR_UNIT=a"b},{"x":"1' 'MONITOR_EXIT_STATUS=4;touch /tmp/bk-pwned' $sut "$alert_bin" --home "$home"
  need "hostile values: exit 0 (got $rc)" test "$rc" -eq 0 || return 1
  need "hostile values: the marker is still one line of the recorded shape: $(cat "$home/ALARM")" sh -c "[ \$(wc -l < '$home/ALARM') -eq 1 ] && grep -Eq '$alarm_re' '$home/ALARM'" || return 1
  need "hostile values: nothing was executed" test ! -e /tmp/bk-pwned || return 1
  # a symbolic link in the marker's place is replaced, not followed
  printf 'victim\n' > "$T/victim"
  rm -f "$home/ALARM"
  ln -s "$T/victim" "$home/ALARM"
  DENV="MONITOR_UNIT=cortex-dump.service MONITOR_EXIT_STATUS=1"
  alert l --home "$home"
  need "the symbolic link's target is untouched" test "$(cat "$T/victim")" = victim || return 1
  need "the marker is a regular file again" test -f "$home/ALARM" -a ! -L "$home/ALARM" || return 1
  # usage
  alert u1 --home "$T/no-such-dir"
  need "a missing home is exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  alert u2
  need "no --home is exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  alert u3 --home "$home" --bogus
  need "an unknown argument is exit 2 (got $rc)" test "$rc" -eq 2 || return 1
}

bk_05() { # the second timer: a dump that never ran (LAST_GOOD 28 hours old) is seen, and the alarm clears when a good dump is back
  new_test
  need "cortex-dump-alert exists and is not a stub" sh -c "! grep -q 'not built yet' '$alert_bin'" || return 1
  home=$T/home
  mkdir -p "$home/dumps" "$home/dump-status"
  old=$(date -u -d '28 hours ago' +%Y-%m-%dT%H:%M:%SZ)
  printf '{"set":"20261105T000000Z","utc":"%s","bytes":123,"sha256":"%s"}\n' "$old" "0000000000000000000000000000000000000000000000000000000000000000" > "$home/dump-status/LAST_GOOD"
  run st env -i PATH="$PATH" $sut "$status_bin" --status-dir "$home/dump-status" --max-age-hours 26
  need "the check exits 1 on a 28 hour old LAST_GOOD (got $rc): $(cat "$T/st.out")" test "$rc" -eq 1 || return 1
  DENV="MONITOR_UNIT=cortex-dump-status.service MONITOR_SERVICE_RESULT=exit-code MONITOR_EXIT_CODE=exited MONITOR_EXIT_STATUS=1"
  alert a --home "$home"
  need "the same alarm path fires (rc=$rc)" test "$rc" -eq 0 -a -f "$home/ALARM" || return 1
  need "the marker says the backup is stale" grep -q '"state":"backup stale: ' "$home/ALARM" || return 1
  need "and the state line was read from the status directory (the age of THAT file, not 'no last good dump')" grep -q '"state":"backup stale: last good dump 28' "$home/ALARM" || return 1
  need "the marker names the freshness check" grep -q '"unit":"cortex-dump-status.service"' "$home/ALARM" || return 1
  # a good dump is back: the check passes, and --clear removes OUR marker
  now=$(date -u +%Y-%m-%dT%H:%M:%SZ)
  printf '{"set":"20261106T000000Z","utc":"%s","bytes":123,"sha256":"%s"}\n' "$now" "0000000000000000000000000000000000000000000000000000000000000000" > "$home/dump-status/LAST_GOOD"
  run st2 env -i PATH="$PATH" $sut "$status_bin" --status-dir "$home/dump-status" --max-age-hours 26
  need "the check passes on a fresh LAST_GOOD (got $rc)" test "$rc" -eq 0 || return 1
  alert c --home "$home" --clear
  need "--clear: exit 0 (got $rc)" test "$rc" -eq 0 || return 1
  need "--clear removed the backup marker" test ! -e "$home/ALARM" || return 1
  # a marker that is not ours stays
  printf '{"source":"other","utc":"2026-11-06T00:00:00Z"}\n' > "$home/ALARM"
  alert c2 --home "$home" --clear
  need "--clear leaves another source's marker alone" grep -q '"source":"other"' "$home/ALARM" || return 1
  # --clear on nothing, and on a symbolic link, is harmless
  rm -f "$home/ALARM"
  alert c3 --home "$home" --clear
  need "--clear with no marker is exit 0 (got $rc)" test "$rc" -eq 0 || return 1
  printf '{"source":"backup","utc":"2026-11-06T00:00:00Z"}\n' > "$T/keep"
  ln -s "$T/keep" "$home/ALARM"
  alert c4 --home "$home" --clear
  need "--clear never follows or removes a link (rc=$rc)" test "$rc" -eq 0 -a -L "$home/ALARM" -a -f "$T/keep" || return 1
}

unit_text() { # unit_text <file>: the unit with its two placeholders filled in
  sed -e "s|@CORTEX_HOME@|$home|g" -e "s|@DB_CONTAINER@|cv2-s5-db|g" "$units/$1"
}

bk_19() { # the unit files: systemd-analyze verify clean, OnFailure= and Persistent=true present, the planned slots
  new_test
  home=$T/home
  mkdir -p "$home/bin" "$home/dumps" "$T/units"
  for f in cortex-dump cortex-dump-status cortex-dump-alert; do cp -p "$here/$f" "$home/bin/$f"; done
  all="cortex-dump.service cortex-dump.timer cortex-dump-status.service cortex-dump-status.timer cortex-dump-alert.service"
  for u in $all; do
    need "$u exists, mode 644" test "$(stat -c %a "$units/$u" 2> /dev/null)" = 644 || return 1
    unit_text "$u" > "$T/units/$u"
  done
  need "only the two documented placeholders exist in the templates" test -z "$(grep -ho '@[A-Z_]*@' "$units"/* | sort -u | grep -v -x -e '@CORTEX_HOME@' -e '@DB_CONTAINER@' || true)" || return 1
  need "none is left after filling them in" test -z "$(grep -l '@[A-Z_]*@' "$T/units"/* || true)" || return 1
  # every systemd-analyze call goes through the helper: a PRIVATE runtime directory, never the live one (kai 13:30Z, 7 Nov: the tool re-binds the live user
  # manager's control socket). The inode of that socket is recorded before and after, here, as well.
  an=$here/cortex-dump-test-analyze.sh
  socks_a=$(live_sockets)
  (cd "$T/units" && sh "$an" verify --user ./cortex-dump.service ./cortex-dump.timer ./cortex-dump-status.service ./cortex-dump-status.timer ./cortex-dump-alert.service) > "$T/verify.out" 2>&1 || {
    echo "    systemd-analyze verify said: $(head -c 600 "$T/verify.out")" >&2
    need "systemd-analyze verify is clean (exit 0)" false || return 1
  }
  need "and it printed nothing" test ! -s "$T/verify.out" || { cat "$T/verify.out" >&2; return 1; }
  need "the live user manager's control socket was not touched by the verify ($socks_a before, $(live_sockets) after)" test "$(live_sockets)" = "$socks_a" || return 1
  for u in cortex-dump.service cortex-dump-status.service; do
    need "$u has OnFailure=cortex-dump-alert.service" grep -qx 'OnFailure=cortex-dump-alert.service' "$T/units/$u" || return 1
  done
  need "the alert unit has no OnFailure= of its own" test -z "$(grep '^OnFailure=' "$T/units/cortex-dump-alert.service" || true)" || return 1
  for u in cortex-dump.timer cortex-dump-status.timer; do
    need "$u has Persistent=true" grep -qx 'Persistent=true' "$T/units/$u" || return 1
  done
  need "cortex-dump.timer starts cortex-dump.service" grep -qx 'Unit=cortex-dump.service' "$T/units/cortex-dump.timer" || return 1
  need "cortex-dump-status.timer starts cortex-dump-status.service" grep -qx 'Unit=cortex-dump-status.service' "$T/units/cortex-dump-status.timer" || return 1
  cal=$(sed -n 's/^OnCalendar=//p' "$T/units/cortex-dump.timer")
  sh "$an" calendar "$cal" > "$T/cal.out" 2>&1 || true
  need "the daily slot is 02:30 UTC (next elapse: $(grep -i 'Next elapse' "$T/cal.out" | head -n 1))" grep -q 'Next elapse:.* 02:30:00 UTC' "$T/cal.out" || return 1
  cal2=$(sed -n 's/^OnCalendar=//p' "$T/units/cortex-dump-status.timer")
  sh "$an" calendar "$cal2" > "$T/cal2.out" 2>&1 || true
  need "the live user manager's control socket was not touched by the calendar calls ($socks_a before, $(live_sockets) after)" test "$(live_sockets)" = "$socks_a" || return 1
  need "the freshness check is hourly, at minute 17 (next elapse: $(grep -i 'Next elapse' "$T/cal2.out" | head -n 1))" grep -q 'Next elapse:.* [0-9][0-9]:17:00 UTC' "$T/cal2.out" || return 1
  need "the check runs more often than the dump (hourly against daily)" test "$(grep -c 'Next elapse' "$T/cal2.out" || true)" -ge 1 || return 1
  tmo=$(sed -n 's/^TimeoutStartSec=//p' "$T/units/cortex-dump.service")
  need "systemd's limit ($tmo s) is above the dump's own 1800 s plus its 30 s grace" test "${tmo:-0}" -ge 1830 || return 1
  need "the dump unit passes the same --timeout the default has (1800)" grep -q -- '--timeout 1800' "$T/units/cortex-dump.service" || return 1
  need "no password, host or port in any unit" test -z "$(grep -iE 'PGPASSWORD|password|localhost|127\.0\.0\.1|PGHOST| -h |--host|5432' "$units"/* | grep -v '^[^:]*:[[:space:]]*#' || true)" || return 1
  need "no User= and no Environment= (user units, nothing secret to set)" test -z "$(grep -E '^(User|Group|Environment|EnvironmentFile)=' "$units"/* || true)" || return 1
  need "the dump unit publishes into the status directory, not into the dump directory" grep -q -- "--out $home/dumps --status-dir $home/dump-status --timeout 1800" "$T/units/cortex-dump.service" || return 1
  need "the check unit reads LAST_GOOD from the status directory and verifies the sets in the dump directory" grep -q -- "cortex-dump-status --status-dir $home/dump-status --out $home/dumps --max-age-hours 26 --verify" "$T/units/cortex-dump-status.service" || return 1
  need "no unit names the dump directory as a status directory" test -z "$(grep -h -- "--status-dir $home/dumps" "$T"/units/* || true)" || return 1
  need "the status check clears the alarm only after it passed (ExecStartPost with the leading minus)" grep -qx "ExecStartPost=-$home/bin/cortex-dump-alert --home $home --clear" "$T/units/cortex-dump-status.service" || return 1
}

# ---- systemd-analyze safety (kai 13:30Z, 7 Nov 2026): the real tool, run with the LIVE runtime directory, unlinks and re-binds the live user
# manager's control socket ($XDG_RUNTIME_DIR/systemd/private) and breaks `systemctl --user` and `systemd-run --user` for the whole VM. Every cell
# below uses a STAND-IN systemd-analyze that is first in PATH; none of them can reach the real tool, and none touches the real socket.

sd_stub() { # sd_stub: put a stand-in systemd-analyze into $T/bin. It logs what it was given and, like the real tool, unlinks and re-creates
  # $XDG_RUNTIME_DIR/systemd/private (a NEW inode). `calendar` answers with the lines bk_19 reads. STUB_RC is its exit status; while STUB_GO is
  # set and does not exist it waits for that file (at most 20 s), so a test can signal it.
  cat > "$T/bin/systemd-analyze" <<'SHIM'
#!/bin/sh
{
  printf 'CALL %s\n' "$*"
  printf 'RT=%s\n' "${XDG_RUNTIME_DIR:-unset}"
  printf 'DBUS=%s\n' "${DBUS_SESSION_BUS_ADDRESS-unset}"
  printf 'MODE=%s\n' "$(stat -c %a "${XDG_RUNTIME_DIR:-/nonexistent}" 2> /dev/null)"
  printf 'PID=%s\n' "$$"
} >> "$STUB_LOG"
if [ -n "${XDG_RUNTIME_DIR:-}" ]; then
  mkdir -p "$XDG_RUNTIME_DIR/systemd"
  # the new file is made BEFORE the old one is replaced, so it cannot get the old inode number back (the live manager keeps the real socket open, so the real
  # tool cannot get it back either; a plain unlink and create could)
  : > "$XDG_RUNTIME_DIR/systemd/private.new"
  mv -f "$XDG_RUNTIME_DIR/systemd/private.new" "$XDG_RUNTIME_DIR/systemd/private"
fi
if [ -n "${STUB_TOUCH:-}" ] && { [ -z "${STUB_TOUCH_ON:-}" ] || [ "${1:-}" = "$STUB_TOUCH_ON" ]; }; then : > "$STUB_TOUCH.new"; mv -f "$STUB_TOUCH.new" "$STUB_TOUCH"; fi
case "$*" in
  *02:30:00*) printf '    Next elapse: Sun 2026-11-08 02:30:00 UTC\n' ;;
  *calendar*) printf '    Next elapse: Sat 2026-11-07 20:17:00 UTC\n' ;;
esac
n=0
while [ -n "${STUB_GO:-}" ] && [ ! -e "$STUB_GO" ] && [ "$n" -lt 200 ]; do sleep 0.1; n=$((n + 1)); done
exit "${STUB_RC:-0}"
SHIM
  chmod 0755 "$T/bin/systemd-analyze"
}

live_socket() { # the inode of a control socket path, or `absent`
  if [ -e "$1" ]; then stat -c %i "$1"; else echo absent; fi
}
live_sockets() { # the live user manager's control socket in every place it can be: /run/user/<uid> and the caller's $XDG_RUNTIME_DIR; one line each
  for _s in "/run/user/$(id -u)/systemd/private" "${XDG_RUNTIME_DIR:-/nonexistent}/systemd/private"; do
    printf '%s:%s\n' "$_s" "$(live_socket "$_s")"
  done
}

bk_57() { # kai 461a5d25 / cox e82567ad: bk_19 run NESTED against a stand-in: every systemd-analyze call of the runner gets a private runtime directory
  new_test
  sd_stub
  rt=$T/rt
  mkdir -p "$rt/systemd"
  : > "$rt/systemd/private"
  chmod 0700 "$rt"
  real=/run/user/$(id -u)/systemd/private
  ri_a=$(live_socket "$real")
  # the safety precondition: with this PATH the stand-in IS what systemd-analyze resolves to; otherwise nothing below runs
  got=$(env PATH="$T/bin:$PATH" sh -c 'command -v systemd-analyze')
  need "the stand-in is first in PATH (got $got)" test "$got" = "$T/bin/systemd-analyze" || return 1
  # (0) control: the stand-in DOES rebind a socket when it is called straight with a runtime directory, so the cells below can fail
  fa=$(live_socket "$rt/systemd/private")
  : > "$T/ctl.log"
  run k env PATH="$T/bin:$PATH" XDG_RUNTIME_DIR="$rt" STUB_LOG="$T/ctl.log" systemd-analyze --user verify a.service
  need "control: the stand-in rebinds the socket of the directory it is given (inode $fa -> $(live_socket "$rt/systemd/private"))" test "$fa" != "$(live_socket "$rt/systemd/private")" || return 1
  # (a) bk_19 nested, with the fake "live" runtime directory exported, as it is in a user's shell
  ino=$(live_socket "$rt/systemd/private")
  ln -f "$rt/systemd/private" "$rt/systemd/private.keep" # keeps that inode allocated, as the live manager keeps the real socket open: a rebind can never hand the number back
  : > "$T/s.log"
  run n env PATH="$T/bin:$PATH" XDG_RUNTIME_DIR="$rt" DBUS_SESSION_BUS_ADDRESS="unix:path=$rt/bus" STUB_LOG="$T/s.log" CORTEX_BK_TMP="$T/nested" $sut "$here/cortex-dump-test.sh" --shell sh BK-19 < /dev/null
  n=$(grep -c '^CALL ' "$T/s.log" || true)
  need "bk_19 called systemd-analyze three times: verify, calendar, calendar (got $n)" test "$n" -eq 3 || return 1
  need "the first call is the verify of the five units" grep -qx 'CALL verify --user ./cortex-dump.service ./cortex-dump.timer ./cortex-dump-status.service ./cortex-dump-status.timer ./cortex-dump-alert.service' "$T/s.log" || return 1
  for rtl in $(sed -n 's/^RT=//p' "$T/s.log"); do
    need "no call ran with the runtime directory of the caller (this one had $rtl)" test "$rtl" != "$rt" -a "$rtl" != unset || return 1
    need "each call had a PRIVATE runtime directory under /tmp/cortex-bk-rt. (this one had $rtl)" test "${rtl#/tmp/cortex-bk-rt.}" != "$rtl" || return 1
    need "and that directory is gone afterwards ($rtl)" test ! -e "$rtl" || return 1
  done
  need "each private directory was mode 0700 (MODE lines: $(grep -c '^MODE=700$' "$T/s.log"))" test "$(grep -c '^MODE=700$' "$T/s.log" || true)" -eq 3 || return 1
  need "each call had an EMPTY session bus address (lines: $(grep -c '^DBUS=$' "$T/s.log"))" test "$(grep -c '^DBUS=$' "$T/s.log" || true)" -eq 3 || return 1
  need "the caller's socket kept its inode ($ino before, $(live_socket "$rt/systemd/private") after)" test "$(live_socket "$rt/systemd/private")" = "$ino" || return 1
  need "the real live socket kept its inode ($ri_a before, $(live_socket "$real") after)" test "$(live_socket "$real")" = "$ri_a" || return 1
  need "the nested bk_19 itself passed (rc $rc): $(tail -n 3 "$T/n.err" | tr '\n' ' ')" test "$rc" -eq 0 || return 1
  # (b) bk_19's OWN inode check: the stand-in is told to rebind the caller's socket as well (a leak nobody foresaw), on ONE subcommand at a time; the nested
  # bk_19 must FAIL and name the call after which it noticed: a rebind by `verify` is caught right after the verify, one by `calendar` only after the calendar calls
  for on in verify calendar; do
    case $on in verify) said="touched by the verify" ;; *) said="touched by the calendar calls" ;; esac
    rm -f "$rt/systemd/private"
    : > "$rt/systemd/private"
    ino=$(live_socket "$rt/systemd/private")
    ln -f "$rt/systemd/private" "$rt/systemd/private.keep" # as above: the old inode stays allocated, so two rebinds cannot cycle back to it
    : > "$T/t.log"
    run t env PATH="$T/bin:$PATH" XDG_RUNTIME_DIR="$rt" STUB_LOG="$T/t.log" STUB_TOUCH="$rt/systemd/private" STUB_TOUCH_ON="$on" CORTEX_BK_TMP="$T/nested-$on" $sut "$here/cortex-dump-test.sh" --shell sh BK-19 < /dev/null
    need "$on: control: the stand-in ran (calls: $(grep -c '^CALL ' "$T/t.log" || true)) and rebound the caller's socket" test "$(grep -c '^CALL ' "$T/t.log" || true)" -ge 1 -a "$(live_socket "$rt/systemd/private")" != "$ino" || return 1
    need "$on: bk_19 FAILS when the live control socket changes (rc $rc)" test "$rc" -ne 0 || return 1
    need "$on: and says which call did it ('$said'): $(tail -n 2 "$T/t.err" | tr '\n' ' ')" grep -q "$said" "$T/t.err" || return 1
  done
}

bk_58() { # kai 461a5d25 / cox 6ca7b35b: no command in the runner or in a piece 12 script calls systemd-analyze directly; the helper's one call sets the private directory on its own line
  new_test
  an=$here/cortex-dump-test-analyze.sh
  bad=""
  for f in "$here/cortex-dump-test.sh" "$dump_bin" "$status_bin" "$restore_bin" "$compare_bin" "$alert_bin"; do
    hits=$(grep -nE '(^|[;&|(]|[$][(])[[:space:]]*systemd-analyze[[:space:]]' "$f" | grep -vE '^[0-9]+:[[:space:]]*#' || true)
    if [ -n "$hits" ]; then bad="$bad ${f##*/} line $(printf '%s\n' "$hits" | cut -d: -f1 | tr '\n' ',')"; fi
  done
  need "no direct systemd-analyze command (found:$bad)" test -z "$bad" || return 1
  need "the helper exists" test -f "$an" || return 1
  need "the helper has exactly ONE systemd-analyze command" test "$(grep -cE '(^|[;&|(]|[$][(])[[:space:]]*systemd-analyze[[:space:]]|^[A-Za-z_]+=[^ ]* .*systemd-analyze ' "$an" || true)" -eq 1 || return 1
  need "and it sets the private runtime directory and an empty bus on that same line" grep -qx 'XDG_RUNTIME_DIR=\$priv DBUS_SESSION_BUS_ADDRESS= systemd-analyze "\$@"' "$an" || return 1
}

bk_59() { # the helper: private 0700 runtime directory, empty bus, exact arguments, the tool's exit status, removal on every exit path
  new_test
  sd_stub
  an=$here/cortex-dump-test-analyze.sh
  need "the helper exists, mode 0755, starts with #!/bin/sh" test -f "$an" -a "$(stat -c %a "$an" 2> /dev/null)" = 755 -a "$(head -n 1 "$an" 2> /dev/null)" = "#!/bin/sh" || return 1
  rt=$T/rt
  mkdir -p "$rt/systemd"
  : > "$rt/systemd/private"
  chmod 0700 "$rt"
  ino=$(live_socket "$rt/systemd/private")
  # (a) a good call
  : > "$T/a.log"
  run a env PATH="$T/bin:$PATH" XDG_RUNTIME_DIR="$rt" DBUS_SESSION_BUS_ADDRESS="unix:path=$rt/bus" STUB_LOG="$T/a.log" $sut "$an" verify --user x.service y.timer
  need "exit 0 (got $rc): $(tail -n 2 "$T/a.err" | tr '\n' ' ')" test "$rc" -eq 0 || return 1
  need "the exact arguments reach the tool" grep -qx 'CALL verify --user x.service y.timer' "$T/a.log" || return 1
  priv=$(sed -n 's/^RT=//p' "$T/a.log" | sed -n 1p)
  need "a private directory under /tmp/cortex-bk-rt. that is not the caller's ($priv)" test "${priv#/tmp/cortex-bk-rt.}" != "$priv" -a "$priv" != "$rt" || return 1
  need "mode 0700, an empty session bus" sh -c "grep -qx 'MODE=700' '$T/a.log' && grep -qx 'DBUS=' '$T/a.log'" || return 1
  need "the private directory is gone afterwards" test ! -e "$priv" || return 1
  need "the caller's socket kept its inode" test "$(live_socket "$rt/systemd/private")" = "$ino" || return 1
  # (b) the tool fails: its status passes through and the directory is still removed
  : > "$T/b.log"
  run b env PATH="$T/bin:$PATH" STUB_LOG="$T/b.log" STUB_RC=3 $sut "$an" verify x.service
  need "the tool's exit status 3 passes through (got $rc)" test "$rc" -eq 3 || return 1
  privb=$(sed -n 's/^RT=//p' "$T/b.log" | sed -n 1p)
  need "and its private directory is gone" test -n "$privb" -a ! -e "$privb" || return 1
  # (c) no arguments: refused with exit 2 before anything is made or run
  : > "$T/c.log"
  run c env PATH="$T/bin:$PATH" STUB_LOG="$T/c.log" $sut "$an"
  need "no arguments: exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  need "and the tool was never called" test ! -s "$T/c.log" || return 1
  # (d) SIGTERM while the tool runs: the directory is removed and the status is 143. The signals go to exact pids (the helper and the stand-in),
  # as a Ctrl-C reaches both; a signal is never sent to a group. (SIGINT cannot be tested from a background job: the shell starts it ignoring SIGINT.)
  : > "$T/d.log"
  rm -f "$T/go"
  env PATH="$T/bin:$PATH" STUB_LOG="$T/d.log" STUB_GO="$T/go" $sut "$an" verify x.service > "$T/d.out" 2> "$T/d.err" &
  hp=$!
  i=0
  until grep -q '^PID=' "$T/d.log" 2> /dev/null; do
    i=$((i + 1))
    [ "$i" -lt 100 ] || break
    sleep 0.1
  done
  sp=$(sed -n 's/^PID=//p' "$T/d.log" | sed -n 1p)
  privd=$(sed -n 's/^RT=//p' "$T/d.log" | sed -n 1p)
  need "the stand-in is running inside a private directory ($privd, pid $sp)" test -n "$sp" -a -d "$privd" || { kill -TERM "$hp" 2> /dev/null || true; return 1; }
  need "pid $sp is the stand-in (its command line names systemd-analyze)" grep -q 'systemd-analyze' "/proc/$sp/cmdline" || { kill -TERM "$hp" 2> /dev/null || true; return 1; }
  kill -TERM "$sp"
  kill -TERM "$hp"
  rc=0
  wait "$hp" || rc=$?
  need "SIGTERM: exit status 143 (got $rc)" test "$rc" -eq 143 || return 1
  need "SIGTERM: the private directory is gone" test ! -e "$privd" || return 1
  # (e) sam BK12-SD-N01 (P3): a setup step that fails AFTER the directory was made must not leave it behind. A stand-in chmod (first in PATH) logs its
  # last argument, the directory, and fails; the helper must exit non-zero, never call the tool, and the directory it made must be gone.
  mkdir -p "$T/binc"
  cat > "$T/binc/chmod" << 'CHMODSTUB'
#!/bin/sh
for _last; do :; done
printf '%s\n' "$_last" >> "${CHMOD_LOG:?}"
exit 1
CHMODSTUB
  chmod 0755 "$T/binc/chmod"
  : > "$T/e.log"
  : > "$T/e.chmod"
  run e env PATH="$T/binc:$T/bin:$PATH" STUB_LOG="$T/e.log" CHMOD_LOG="$T/e.chmod" $sut "$an" verify x.service
  prive=$(sed -n 1p "$T/e.chmod")
  need "(e) the stand-in chmod was reached, with a private directory under /tmp/cortex-bk-rt. (${prive:-none})" test "${prive#/tmp/cortex-bk-rt.}" != "$prive" -a -n "$prive" || return 1
  if [ -e "$prive" ]; then # the helper leaked it: remove that one directory by its exact, shape-checked name so a failing run leaves nothing
    case $prive in /tmp/cortex-bk-rt.??????) rm -rf -- "$prive" ;; esac # s5-never: ok the exact directory the helper made, shape-checked
    need "(e) the helper left its private directory behind after a failing chmod (removed here by name)" false || return 1
  fi
  need "(e) a failing chmod fails the helper (got $rc)" test "$rc" -ne 0 || return 1
  need "(e) and the tool was never called" test ! -s "$T/e.log" || return 1
}

bk_04s() { # the OnFailure seam on a real systemd user manager: a failing unit starts the alert unit, which gets MONITOR_* and leaves the marker
  new_test
  [ "${CV2_SYSTEMD:-0}" = 1 ] || { echo "    CHECK FAILED: --systemd needs CV2_SYSTEMD=1 (it loads units named cv2bk-* into the user manager)" >&2; return 1; }
  need "a systemd user manager is running" systemctl --user is-active default.target || return 1
  need "cortex-dump-alert is not a stub" sh -c "! grep -q 'not built yet' '$alert_bin'" || return 1
  home=$T/home
  mkdir -p "$home/dumps"
  tag=cv2bk-$(od -An -N4 -tx1 /dev/urandom | tr -d ' \n')
  udir=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/systemd/user
  mkdir -p "$udir"
  # the real alert unit text with its placeholders filled in, renamed; the dump unit is a shim that exits 4 as a timed-out dump would
  sed -e "s|@CORTEX_HOME@|$home|g" -e "s|cortex-dump-alert|$tag-alert|g" "$units/cortex-dump-alert.service" > "$udir/$tag-alert.service"
  sed -i -e "s|^ExecStart=$home/bin/$tag-alert|ExecStart=$here/cortex-dump-alert|" "$udir/$tag-alert.service"
  printf '[Unit]\nDescription=BK-04S shim: a dump that times out\nOnFailure=%s-alert.service\n[Service]\nType=oneshot\nExecStart=/bin/sh -c "exit 4"\n' "$tag" > "$udir/$tag-dump.service"
  systemctl --user daemon-reload
  systemctl --user start "$tag-dump.service" > "$T/start.out" 2>&1 || true
  i=0
  until [ -f "$home/ALARM" ] || [ "$i" -ge 40 ]; do i=$((i + 1)); sleep 0.25; done
  state=$(systemctl --user show "$tag-dump.service" -p Result -p ExecMainStatus | tr '\n' ' ')
  cp "$home/ALARM" "$T/alarm.copy" 2> /dev/null || true
  # clean up by exact names and paths, whatever happened
  systemctl --user stop "$tag-dump.service" "$tag-alert.service" > /dev/null 2>&1 || true
  systemctl --user reset-failed "$tag-dump.service" "$tag-alert.service" > /dev/null 2>&1 || true
  rm -f -- "$udir/$tag-dump.service" "$udir/$tag-alert.service"
  systemctl --user daemon-reload
  need "the shim failed with status 4 as intended ($state)" sh -c "echo '$state' | grep -q 'Result=exit-code' && echo '$state' | grep -q 'ExecMainStatus=4'" || return 1
  need "systemd started the alert unit and the marker exists" test -f "$T/alarm.copy" || return 1
  echo "    the marker systemd's OnFailure produced: $(cat "$T/alarm.copy")" >&2
  need "it names the failed unit ($tag-dump.service)" grep -q "\"unit\":\"$tag-dump.service\"" "$T/alarm.copy" || return 1
  need "it carries the exit status systemd handed over (4) and its reason" sh -c "grep -q '\"status\":\"4\"' '$T/alarm.copy' && grep -q '\"reason\":\"timed out\"' '$T/alarm.copy'" || return 1
  need "the units are gone from the user manager" test -z "$(systemctl --user list-units --all --no-legend "$tag*" 2> /dev/null | tr -d ' ')" || return 1
}

# ---- rework 1 (sam's review of af94bf1, findings BK12-01 to BK12-14): one test per finding, written before the fix -------------

make_set_g() { # make_set_g <globals text; '@@' stands for a newline>: a set whose globals.sql is that text
  mkdir -p "$T/set"
  printf 'PGDMP-FAKE\nTOC-END\nTABLE platform objects\nEND-OF-DUMP\n' > "$T/set/platform.dump"
  printf '%s\n' "$1" | sed 's/@@/\n/g' > "$T/set/globals.sql"
  (cd "$T/set" && sha256sum platform.dump globals.sql > SHA256SUMS)
}

pid_of() { # pid_of <exact command line words, space separated>: the pid whose /proc cmdline is exactly that (to signal ONE process by its pid)
  for _p in /proc/[0-9]*; do
    _c=$(tr '\0' ' ' < "$_p/cmdline" 2> /dev/null || true)
    case $_c in
      "$1 "* | "$1") echo "${_p#/proc/}"; return 0 ;;
    esac
  done
  return 1
}

lock_free() { # lock_free <out dir>: no dump holds the lock (a lock FILE may stay behind; the kernel frees it with its owner)
  [ ! -e "$1/.dump.lock" ] || flock -n "$1/.dump.lock" true
}

bk_38() { # BK12-01: globals.sql is accepted statement by statement, whole lines only; a stacked statement or a psql command is refused (exit 2) before anything is written
  new_test
  mkdir -p "$T/fs"
  n=0
  while IFS= read -r g; do
    [ -n "$g" ] || continue
    n=$((n + 1))
    rm -f -- "$T/fs/stdin.sql" /tmp/bk-pwned2
    make_set_g "$g"
    DENV="FAKE_STATE=$T/fs" restore "g$n" --set "$T/set" --target fakedb --confirm-new
    need "hostile globals #$n is refused with exit 2 (got $rc): $g" test "$rc" -eq 2 || return 1
    need "#$n: nothing was forwarded to a role script" test ! -e "$T/fs/stdin.sql" || return 1
    need "#$n: nothing that writes was asked of any container" no_writes_logged || return 1
    need "#$n: nothing was executed" test ! -e /tmp/bk-pwned2 || return 1
  done <<'EOF'
CREATE ROLE fresh;@@ALTER ROLE fresh WITH LOGIN; ALTER ROLE postgres NOLOGIN;
CREATE ROLE fresh;@@ALTER ROLE fresh WITH LOGIN \! touch /tmp/bk-pwned2
SET client_encoding = 'UTF8' \! touch /tmp/bk-pwned2;
GRANT a TO b; ALTER ROLE postgres NOLOGIN;
\restrict abc \! touch /tmp/bk-pwned2
-- a comment \! touch /tmp/bk-pwned2
CREATE ROLE fresh;@@ALTER ROLE fresh SET search_path TO x;
CREATE ROLE fresh;@@ALTER ROLE fresh WITH NOSUPERUSER; DROP ROLE postgres;
CREATE ROLE fresh;@@ALTER ROLE fresh WITH LOGIN PASSWORD 'x'; ALTER ROLE postgres PASSWORD 'y';
EOF
  # control: the shape pg_dumpall writes passes; statements about a role the target has are left out
  new_test
  mkdir -p "$T/fs"
  make_set
  cat > "$T/set/globals.sql" <<'EOF'
--
-- PostgreSQL database cluster dump
--

SET default_transaction_read_only = off;

SET client_encoding = 'UTF8';
SET standard_conforming_strings = on;

CREATE ROLE postgres;
ALTER ROLE postgres WITH SUPERUSER INHERIT CREATEROLE CREATEDB LOGIN REPLICATION BYPASSRLS PASSWORD 'SCRAM-SHA-256$4096:AAAA+bbb/CCC=$dddd=:eeee=';
CREATE ROLE fresh;
ALTER ROLE fresh WITH NOSUPERUSER INHERIT NOCREATEROLE NOCREATEDB LOGIN NOREPLICATION NOBYPASSRLS CONNECTION LIMIT 5 PASSWORD 'SCRAM-SHA-256$4096:AAAA+bbb/CCC=$dddd=:eeee=';
GRANT pg_read_all_data TO fresh WITH INHERIT TRUE, SET FALSE GRANTED BY postgres;
GRANT fresh TO postgres WITH ADMIN OPTION, INHERIT FALSE, SET FALSE GRANTED BY postgres;
EOF
  (cd "$T/set" && sha256sum platform.dump globals.sql > SHA256SUMS)
  DENV="FAKE_STATE=$T/fs" restore ok --set "$T/set" --target fakedb --confirm-new
  need "a real-shaped globals.sql restores (rc=$rc): $(tail -n 3 "$T/ok.err")" test "$rc" -eq 0 || return 1
  need "the missing role is created" grep -q '^CREATE ROLE fresh;' "$T/fs/stdin.sql" || return 1
  need "its attributes and verifier are forwarded" grep -q "^ALTER ROLE fresh WITH .* CONNECTION LIMIT 5 PASSWORD " "$T/fs/stdin.sql" || return 1
  need "its membership in a predefined role is forwarded" grep -q '^GRANT pg_read_all_data TO fresh' "$T/fs/stdin.sql" || return 1
  need "nothing about the role the target has (postgres) is forwarded" test -z "$(grep -E '^(CREATE|ALTER) ROLE postgres|^GRANT fresh TO postgres' "$T/fs/stdin.sql" || true)" || return 1
  need "no verifier on stdout or stderr" test -z "$(grep -h 'SCRAM-SHA-256' "$T/ok.out" "$T/ok.err" || true)" || return 1
}

write_cat_barrier() { # a `cat` that holds back the reads of an old-style lock owner file so that BOTH runs hold the same stale value: A waits for B's read, B waits until A is inside pg_dump (forces the interleaving of the stale-lock race)
  cat > "$T/bin/cat" <<'SHIM'
#!/bin/sh
case "${1:-}" in
  */.lock/pid)
    side=${PROBE_SIDE:-X}
    v=$(/usr/bin/cat "$1")
    : > "$SYNC/$side-read"
    i=0
    if [ "$side" = A ]; then
      while [ ! -e "$SYNC/B-read" ] && [ "$i" -lt 600 ]; do i=$((i + 1)); sleep 0.01; done
    else
      while [ ! -s "$WAIT_FOR" ] && [ "$i" -lt 600 ]; do i=$((i + 1)); sleep 0.01; done
    fi
    printf '%s\n' "$v" ;;
  *) exec /usr/bin/cat "$@" ;;
esac
SHIM
  chmod 0755 "$T/bin/cat"
}

bk_39() { # BK12-02: one writer, whatever the order; stale-lock recovery cannot let two dumps run; a hard-killed owner needs no clean-up by hand
  new_test
  mkdir -p "$T/out/.lock" "$T/sync"
  printf '2147483647\n' > "$T/out/.lock/pid" # a stale old-style lock (its owner is dead); the barrier shim makes both runs read it at once
  write_cat_barrier
  DENV="FAKE_DUMP_MODE=slow FAKE_SLOW_SECS=4 FAKE_WORKER_PID=$T/worker-A.pid SYNC=$T/sync PROBE_SIDE=A"
  (dump A; exit "$rc") &
  pa=$!
  DENV="FAKE_DUMP_MODE=slow FAKE_SLOW_SECS=4 FAKE_WORKER_PID=$T/worker-B.pid SYNC=$T/sync PROBE_SIDE=B WAIT_FOR=$T/worker-A.pid"
  (dump B; exit "$rc") &
  pb=$!
  ra=0 rb=0
  wait "$pa" || ra=$?
  wait "$pb" || rb=$?
  started_n=$(grep -c ' pg_dump -U' "$T/podman.log" || true)
  need "exactly ONE of the two runs started pg_dump (got $started_n)" test "$started_n" -eq 1 || return 1
  need "one run exits 0 and the other 3 (got $ra and $rb)" test "$ra$rb" = 03 -o "$ra$rb" = 30 || return 1
  need "exactly one set" test "$(count_good_sets)" -eq 1 || return 1
  need "the lock is free afterwards" lock_free "$T/out" || return 1
  # the owner dies hard (SIGKILL by its exact pid): the next run just works
  rm -f "$T/worker.pid"
  DENV="FAKE_DUMP_MODE=slow FAKE_SLOW_SECS=20 FAKE_WORKER_PID=$T/worker.pid"
  (dump K; exit "$rc") &
  pk=$!
  need "the doomed run took the worker" wait_for "$T/worker.pid" || return 1
  sp=$(pid_of "$sut $dump_bin --target fakedb --out $T/out") || sp=""
  need "found the dump process's pid" test -n "$sp" || return 1
  kill -9 "$sp" 2> /dev/null || true # s5-never: ok the test kills ITS OWN doomed dump process by its exact pid; the hard kill is the point of the test
  wait "$pk" 2> /dev/null || true
  wp=$(cat "$T/worker.pid")
  need "the dead owner's worker ($wp) is still running (so its children could hold a lock they inherited)" kill -0 "$wp" || return 1
  DENV=""
  dump after
  after_rc=$rc
  kill "$wp" 2> /dev/null || true
  need "a run right after the kill exits 0 although the dead owner's children still run (got $after_rc): $(tail -n 2 "$T/after.err")" test "$after_rc" -eq 0 || return 1
}

bk_40() { # BK12-03: an archive cut after its table of contents (pg_restore --list still passes) is not published
  new_test
  DENV="FAKE_DUMP_MODE=truncated_after_toc" dump a
  need "exit 1 (got $rc)" test "$rc" -eq 1 || return 1
  need "no set" test "$(count_good_sets)" -eq 0 || return 1
  need "no temp directory" test -z "$(temp_left)" || return 1
  need "no LAST_GOOD" test ! -e "$T/status/LAST_GOOD" || return 1
  dump b
  need "a whole archive still works (rc=$rc)" test "$rc" -eq 0 || return 1
}

bk_42() { # BK12-07: a symbolic link to a DIRECTORY at ALARM is replaced, never followed
  new_test
  home=$T/home
  mkdir -p "$home" "$T/dest"
  ln -s "$T/dest" "$home/ALARM"
  DENV="MONITOR_UNIT=cortex-dump.service MONITOR_EXIT_STATUS=1"
  alert a --home "$home"
  need "exit 0 (got $rc)" test "$rc" -eq 0 || return 1
  need "ALARM is a regular file now, not a link" test -f "$home/ALARM" -a ! -L "$home/ALARM" || return 1
  need "nothing was written into the link's target" test -z "$(ls -A "$T/dest")" || return 1
  need "the marker has the recorded shape" grep -Eq "$alarm_re" "$home/ALARM" || return 1
}

bk_43() { # BK12-08: a signal during the role step leaves no file with password hashes behind, and stops at once
  new_test
  make_set
  mkdir -p "$T/fs"
  DENV="FAKE_STATE=$T/fs FAKE_ROLES_HANG=1 FAKE_ROLES_HANG_SECS=3"
  (restore s --set "$T/set" --target fakedb --confirm-new; exit "$rc") &
  bg=$!
  need "the role step was reached" wait_for "$T/fs/roles-ready" || return 1
  rp=$(pid_of "$sut $restore_bin --set $T/set --target fakedb --confirm-new") || rp=""
  need "found the restore process's pid" test -n "$rp" || return 1
  t0=$(date +%s%3N)
  kill -TERM "$rp"
  rcs=0
  wait "$bg" || rcs=$?
  t1=$(date +%s%3N)
  need "it exits 1 after the signal (got $rcs)" test "$rcs" -eq 1 || return 1
  need "and stops promptly ($((t1 - t0)) ms, not after the role command)" test $((t1 - t0)) -lt 2000 || return 1
  hp=$(cat "$T/fs/hang.pid" 2> /dev/null || true)
  need "the running role job was stopped by its pid, not left to finish (pid $hp)" sh -c "! kill -0 '$hp' 2> /dev/null" || return 1
  need "no filtered role file (password hashes) is left: $(find "$T" -name 'globals.filtered.sql' 2> /dev/null)" test -z "$(find "$T" -name 'globals.filtered.sql' 2> /dev/null)" || return 1
  need "the details directory is kept for the operator" test "$(ls -d "$T"/cortex-restore.* 2> /dev/null | wc -l | tr -d ' ')" -eq 1 || return 1
}

bk_44() { # BK12-09: a database that does not accept connections is part of the inventory: refuse it (template0 is the one exception)
  new_test
  make_set
  mkdir -p "$T/fs"
  DENV="FAKE_STATE=$T/fs FAKE_DBS=postgres,hidden:f" restore a --set "$T/set" --target fakedb --confirm-new
  need "a user database with ALLOW_CONNECTIONS false: exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  need "the reason says it cannot be inspected: $(cat "$T/a.err")" grep -q 'does not accept connections' "$T/a.err" || return 1
  need "nothing that writes was asked of any container" no_writes_logged || return 1
  DENV="FAKE_STATE=$T/fs FAKE_DBS=postgres,cortex:f" restore b --set "$T/set" --target fakedb --confirm-new
  need "the requested name exists but does not accept connections: exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  need "nothing that writes was asked of any container" no_writes_logged || return 1
  DENV="FAKE_STATE=$T/fs FAKE_DBS=postgres,template0:f" restore c --set "$T/set" --target fakedb --confirm-new
  need "template0 is the one exception: the restore goes on (rc=$rc): $(tail -n 2 "$T/c.err")" test "$rc" -eq 0 || return 1
}

bk_45() { # BK12-12: --out is resolved before the denied-directory checks: HOME by any spelling, a link to it, a dot-dot path are refused
  new_test
  hd=$T/fakehome
  mkdir -p "$hd/sub"
  chmod 0750 "$hd"
  ln -s "$hd" "$T/lnk"
  for o in "$hd/." "$hd/" "$hd/sub/.." "$T/lnk" "$T/lnk/." "$T/x/../fakehome"; do
    run g env HOME="$hd" PATH="$T/bin:$PATH" FAKE_LOG="$T/podman.log" $sut "$dump_bin" --target fakedb --out "$o" --status-dir "$T/status"
    need "--out '$o' is refused with exit 2 (got $rc)" test "$rc" -eq 2 || return 1
    need "HOME's mode is untouched (got $(stat -c %a "$hd"))" test "$(stat -c %a "$hd")" = 750 || return 1
    need "nothing was created in HOME: $(ls -A "$hd")" test "$(ls -A "$hd")" = sub || return 1
  done
  run g env HOME="$hd" PATH="$T/bin:$PATH" FAKE_LOG="$T/podman.log" $sut "$dump_bin" --target fakedb --out "$hd/dumps" --status-dir "$T/status"
  need "control: a real subdirectory of HOME works (rc=$rc): $(tail -n 2 "$T/g.err")" test "$rc" -eq 0 || return 1
}

bk_46() { # BK12-13: LAST_GOOD with a time that does not exist is untrustworthy, not fresh
  new_test
  mkdir -p "$T/out"
  DENV="CORTEX_DUMP_NOW=2026-11-06T12:00:00Z"
  for u in 2026-10-37T11:00:00Z 2026-02-29T11:00:00Z 2026-13-01T11:00:00Z 2026-00-10T11:00:00Z 2026-11-00T11:00:00Z 2026-11-06T24:00:00Z 2026-11-06T11:60:00Z 2026-11-06T11:00:61Z; do
    write_last_good "$u"
    status s
    need "$u: exit 1 (got $rc): $(cat "$T/s.out")" test "$rc" -eq 1 || return 1
    need "$u: the reason says the time is not real: $(cat "$T/s.out")" grep -q 'not a real' "$T/s.out" || return 1
  done
  write_last_good "2024-02-29T11:00:00Z"
  status s
  need "a real leap day is a real date, so it is just stale: exit 1 (got $rc)" test "$rc" -eq 1 || return 1
  need "and it is reported as stale, not as an invalid time: $(cat "$T/s.out")" test -z "$(grep 'not a real' "$T/s.out" || true)" || return 1
  write_last_good "2026-11-06T11:00:00Z"
  status s
  need "a real recent time is fresh: exit 0 (got $rc)" test "$rc" -eq 0 || return 1
  DENV="CORTEX_DUMP_NOW=2026-02-30T00:00:00Z"
  status s
  need "an impossible CORTEX_DUMP_NOW is a usage error: exit 2 (got $rc)" test "$rc" -eq 2 || return 1
}

bk_47() { # BK12-14: the real-database fixtures are named per run and only what the run started is removed; no reclaim of another run's container by a shared label
  new_test
  src=$here/cortex-dump-test.sh
  lab="Config""\\.Labels"
  need "no container is reclaimed because it carries the shared label" test -z "$(grep -n "$lab" "$src" | grep -v 'lab=' || true)" || return 1
  fixed=$(grep -n 'cv2-s5-bk' "$src" | grep -v '^[0-9]*:[[:space:]]*#' | grep -v 'bkp=' | grep -v 'fixed=' || true)
  need "no fixed container name outside the one prefix definition: $(printf '%s' "$fixed" | head -n 3)" test -z "$fixed" || return 1
  need "the prefix carries a per-run id" grep -q '^bkp=cv2-s5-bk-\$rid' "$src" || return 1
  need "cleanup removes only containers this run started and verifies they are gone" sh -c "grep -q 'for _p in \$started' '$src' && grep -q 'still exists after removal' '$src'" || return 1
}

# ---- rework 2 (sam's re-read of c233170): the residuals and the new findings ------------------------------------------------

slow_shims() { # stand-ins for sha256sum and rm -rf that block, when asked, BEFORE doing their real work (a long checksum, a long removal)
  cat > "$T/bin/sha256sum" <<'SHIM'
#!/bin/sh
if [ "${SLOW_SHA:-0}" = 1 ]; then echo $$ > "$SLOW_PID_FILE"; sleep "${SLOW_SECS:-20}"; fi
exec /usr/bin/sha256sum "$@"
SHIM
  cat > "$T/bin/rm" <<'SHIM'
#!/bin/sh
case " $* " in
  *" -rf "*) if [ "${SLOW_RM:-0}" = 1 ]; then echo $$ > "$SLOW_PID_FILE"; sleep "${SLOW_SECS:-20}"; fi ;;
esac
exec /usr/bin/rm "$@"
SHIM
  chmod 0755 "$T/bin/sha256sum" "$T/bin/rm"
}

bk_48() { # BK12-02 residual: the lock dies with its owner even while the owner's CHILDREN (a long checksum, a long removal) are still running
  new_test
  slow_shims
  mkdir -p "$T/out"
  # the helper itself: it sees a held lock (and the old .lock directory check could not)
  flock -x "$T/out/.dump.lock" sleep 2 &
  hp=$!
  sleep 0.5
  if lock_free "$T/out"; then echo "    CHECK FAILED: lock_free does not see a held lock" >&2; wait "$hp" 2> /dev/null; return 1; fi
  wait "$hp" 2> /dev/null || true # the holder ends by itself: nothing is killed, so nothing is left holding the lock
  need "after the holder ended the lock is free" lock_free "$T/out" || return 1
  # (a) the owner is killed while sha256sum runs
  DENV="SLOW_SHA=1 SLOW_PID_FILE=$T/sha.pid SLOW_SECS=20"
  (dump K1; exit "$rc") &
  pk=$!
  need "the checksum child started" wait_for "$T/sha.pid" || return 1
  sp=$(pid_of "$sut $dump_bin --target fakedb --out $T/out") || sp=""
  need "found the dump process's pid" test -n "$sp" || return 1
  kill -9 "$sp" 2> /dev/null || true # s5-never: ok the test kills ITS OWN doomed dump process by its exact pid; the hard kill is the point of the test
  wait "$pk" 2> /dev/null || true
  shp=$(cat "$T/sha.pid")
  need "the dead owner's checksum child ($shp) still runs" kill -0 "$shp" || return 1
  DENV=""
  dump after1
  ar=$rc
  kill "$shp" 2> /dev/null || true
  need "(a) the next run exits 0 although the checksum child still runs (got $ar): $(tail -n 1 "$T/after1.err")" test "$ar" -eq 0 || return 1
  # (b) the owner is killed while the retention removal runs (three good sets exist, so the fourth makes one go)
  dump r2
  dump r3
  need "three good sets" test "$(count_good_sets)" -ge 3 || return 1
  DENV="SLOW_RM=1 SLOW_PID_FILE=$T/rm.pid SLOW_SECS=20"
  (dump K2; exit "$rc") &
  pk=$!
  need "the removal child started" wait_for "$T/rm.pid" || return 1
  sp=$(pid_of "$sut $dump_bin --target fakedb --out $T/out") || sp=""
  need "found the dump process's pid (b)" test -n "$sp" || return 1
  kill -9 "$sp" 2> /dev/null || true # s5-never: ok the test kills ITS OWN doomed dump process by its exact pid; the hard kill is the point of the test
  wait "$pk" 2> /dev/null || true
  rmp=$(cat "$T/rm.pid")
  need "the dead owner's removal child ($rmp) still runs" kill -0 "$rmp" || return 1
  DENV=""
  dump after2
  ar=$rc
  kill "$rmp" 2> /dev/null || true
  need "(b) the next run exits 0 although the removal child still runs (got $ar): $(tail -n 1 "$T/after2.err")" test "$ar" -eq 0 || return 1
}

bk_49() { # BK12-R1-01: TERM during the archive check (a foreground child defers the trap) stops the dump at once: child gone, temp gone, lock free
  new_test
  mkdir -p "$T/fs"
  DENV="FAKE_STATE=$T/fs FAKE_FULLREAD_HANG=1 FAKE_FULLREAD_SECS=3"
  (dump H; exit "$rc") &
  bg=$!
  need "the full read was reached" wait_for "$T/fs/fullread-ready" || return 1
  dp=$(pid_of "$sut $dump_bin --target fakedb --out $T/out") || dp=""
  need "found the dump process's pid" test -n "$dp" || return 1
  t0=$(date +%s%3N)
  kill -TERM "$dp"
  rcs=0
  wait "$bg" || rcs=$?
  t1=$(date +%s%3N)
  need "it exits 143 after TERM (got $rcs)" test "$rcs" -eq 143 || return 1
  need "and promptly ($((t1 - t0)) ms; the blocked read would hold it for 3000 ms)" test $((t1 - t0)) -lt 1500 || return 1
  need "the blocked read was stopped (pid $(cat "$T/fs/fullread.pid"))" sh -c "! kill -0 '$(cat "$T/fs/fullread.pid")' 2> /dev/null" || return 1
  need "no temp directory is left" test -z "$(temp_left)" || return 1
  need "the lock is free" lock_free "$T/out" || return 1
  need "no set" test "$(count_good_sets)" -eq 0 || return 1
}

tripwire_podman() { # a podman that only RECORDS the call, first in PATH: for a test whose subject must NEVER reach a real podman
  cat > "$T/bin/podman" <<'SHIM'
#!/bin/sh
echo "TRIPWIRE podman $*" >> "$FAKE_LOG"
exit 0
SHIM
  chmod 0755 "$T/bin/podman"
  : > "$T/trip.log"
}

bk_54() { # BK12-14 (lifecycle) and BK12-R3-01 (no real podman): a name used twice in one run: every ID is removed once, by its own ID, and none is left for the final cleanup to miss; the mode records its OWN calls on stdout and the podman in PATH is never called
  new_test
  tripwire_podman
  run c env PATH="$T/bin:$PATH" FAKE_LOG="$T/trip.log" $sut "$here/cortex-dump-test.sh" --shell sh --selftest custody < /dev/null
  need "the self-test exits 0 (cleanup_failed stayed 0) (got $rc): $(tail -n 2 "$T/c.err")" test "$rc" -eq 0 || return 1
  for cid in id-one id-two id-three; do # not `id`: that is the runner's own loop variable
    need "$cid was removed exactly once, by its own ID (got $(grep -c "^podman rm --force --volumes $cid\$" "$T/c.out" || true))" test "$(grep -c "^podman rm --force --volumes $cid\$" "$T/c.out" || true)" -eq 1 || return 1
  done
  need "nothing else was removed" test "$(grep -c '^podman rm ' "$T/c.out" || true)" -eq 3 || return 1
  need "the podman in PATH was never called (tripwire log: $(cat "$T/trip.log"))" test ! -s "$T/trip.log" || return 1
}

bk_55() { # BK12-R3-01 (direct entry): the self-test mode cannot reach a real podman whatever the caller's PATH and environment, and takes no other mode
  new_test
  tripwire_podman
  run k env PATH="$T/bin:$PATH" FAKE_LOG="$T/trip.log" sh -c 'podman rm --force --volumes control-id'
  need "control: the tripwire is what podman resolves to with this PATH (log: $(cat "$T/trip.log"))" test "$(grep -c '^TRIPWIRE podman rm --force --volumes control-id$' "$T/trip.log" || true)" -eq 1 || return 1
  : > "$T/trip.log"
  # (a) the environment of a database run changes nothing
  run a env PATH="$T/bin:$PATH" FAKE_LOG="$T/trip.log" CV2_DB=1 CV2_ADMIT_LOG=1 $sut "$here/cortex-dump-test.sh" --shell sh --selftest custody < /dev/null
  need "(a) exits 0 (got $rc): $(tail -n 2 "$T/a.err")" test "$rc" -eq 0 || return 1
  need "(a) the recorded calls are the mode's own (3 removals on stdout, got $(grep -c '^podman rm ' "$T/a.out" || true))" test "$(grep -c '^podman rm ' "$T/a.out" || true)" -eq 3 || return 1
  need "(a) the podman in PATH was never called (tripwire log: $(cat "$T/trip.log"))" test ! -s "$T/trip.log" || return 1
  # (b) the mode takes no other mode: with --db, --systemd or a test id it is refused (exit 2) before anything is registered
  for extra in --db --systemd BK-54; do
    run b env PATH="$T/bin:$PATH" FAKE_LOG="$T/trip.log" $sut "$here/cortex-dump-test.sh" --shell sh $extra --selftest custody < /dev/null
    need "(b) $extra with --selftest is refused, exit 2 (got $rc)" test "$rc" -eq 2 || return 1
    need "(b) $extra: the refusal says why ($(tail -n 1 "$T/b.err"))" grep -q 'takes no other mode' "$T/b.err" || return 1
    need "(b) $extra: nothing was recorded or called (stdout lines: $(wc -l < "$T/b.out"))" test ! -s "$T/b.out" || return 1
    need "(b) $extra: the podman in PATH was never called (tripwire log: $(cat "$T/trip.log"))" test ! -s "$T/trip.log" || return 1
  done
  # (c) an unknown mode is refused
  run u env PATH="$T/bin:$PATH" FAKE_LOG="$T/trip.log" $sut "$here/cortex-dump-test.sh" --shell sh --selftest nonsense < /dev/null
  need "(c) an unknown self-test is exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  need "(c) it says so ($(tail -n 1 "$T/u.err"))" grep -q 'unknown --selftest nonsense' "$T/u.err" || return 1
  need "(c) the podman in PATH was never called (tripwire log: $(cat "$T/trip.log"))" test ! -s "$T/trip.log" || return 1
  # (d) the mode touches no file: a fixture that ANOTHER run left in the base directory is kept, and so is everything else there
  mkdir -p "$T/selfbase/fixture" "$T/selfbase/sibling"
  printf 'dump material of another run\n' > "$T/selfbase/fixture/sentinel"
  printf 'keep\n' > "$T/selfbase/sibling/keep"
  (cd "$T/selfbase" && find . | LC_ALL=C sort && sha256sum fixture/sentinel sibling/keep) > "$T/base.before" 2>&1
  run d env PATH="$T/bin:$PATH" FAKE_LOG="$T/trip.log" CORTEX_BK_TMP="$T/selfbase" $sut "$here/cortex-dump-test.sh" --shell sh --selftest custody < /dev/null
  need "(d) exits 0 (got $rc): $(tail -n 2 "$T/d.err")" test "$rc" -eq 0 || return 1
  need "(d) the three registry removals are still recorded (got $(grep -c '^podman rm ' "$T/d.out" || true))" test "$(grep -c '^podman rm ' "$T/d.out" || true)" -eq 3 || return 1
  (cd "$T/selfbase" && find . | LC_ALL=C sort && sha256sum fixture/sentinel sibling/keep) > "$T/base.after" 2>&1 || true
  need "(d) the pre-existing fixture and the whole base directory are unchanged (diff: $(diff "$T/base.before" "$T/base.after" | head -n 3 | tr '\n' ' '))" cmp -s "$T/base.before" "$T/base.after" || return 1
  need "(d) the podman in PATH was never called (tripwire log: $(cat "$T/trip.log"))" test ! -s "$T/trip.log" || return 1
}

bk_50() { # BK12-14 (custody, runtime): a name that cannot be proved absent is refused; ownership starts only when podman run succeeded and is held as the returned ID; an inspection error in cleanup fails the cleanup
  new_test
  mkdir -p "$T/fs"
  cat > "$T/bin/podman" <<'SHIM'
#!/bin/sh
echo "podman $*" >> "$FAKE_LOG"
case "${1:-} ${2:-}" in
  "container exists")
    if [ -f "$FAKE_STATE/removed" ]; then exit "${EXISTS_AFTER_RM_RC:-1}"; fi
    exit "${EXISTS_RC:-1}" ;;
  "rm --force") : > "$FAKE_STATE/removed"; exit 0 ;;
  "run -d")
    if [ "${RUN_RC:-0}" -ne 0 ]; then echo "Error: the container name is already in use" >&2; exit "$RUN_RC"; fi
    if [ "${RUN_EMPTY:-0}" = 1 ]; then exit 0; fi
    echo 0123456789abcdef0123456789abcdef
    exit 0 ;;
  "exec "*) cat > /dev/null 2>&1; exit 0 ;;
esac
exit 0
SHIM
  chmod 0755 "$T/bin/podman"
  nested() { # nested <name> <env assignments...>: the real-database setup of THIS runner against the recording podman; it never reaches a test
    _nn=$1
    shift
    : > "$T/podman.log"
    rm -f "$T/fs/removed"
    run "$_nn" env PATH="$T/bin:$PATH" FAKE_LOG="$T/podman.log" FAKE_STATE="$T/fs" CV2_DB=1 CORTEX_BK_TMP="$T/nested" "$@" $sut "$here/cortex-dump-test.sh" --shell sh --db BK-01D < /dev/null
  }
  runs() { grep -c 'podman run -d' "$T/podman.log" || true; }
  rms() { grep -c 'podman rm ' "$T/podman.log" || true; }
  nested a EXISTS_RC=125
  need "(a) 'container exists' fails (125): refused with exit 3 (got $rc)" test "$rc" -eq 3 || return 1
  need "(a) nothing was started (runs: $(runs)) and nothing removed (rm: $(rms))" test "$(runs)$(rms)" = 00 || return 1
  nested b EXISTS_RC=0
  need "(b) a name that exists: refused with exit 3 (got $rc)" test "$rc" -eq 3 || return 1
  need "(b) nothing was started (runs: $(runs)) and the existing container was not removed (rm: $(rms))" test "$(runs)$(rms)" = 00 || return 1
  nested c RUN_RC=125
  need "(c) podman run fails: exit 3 (got $rc)" test "$rc" -eq 3 || return 1
  need "(c) nothing was removed: a failed creation confers no ownership (rm: $(rms))" test "$(rms)" = 0 || return 1
  nested f RUN_EMPTY=1
  need "(f) podman run succeeds but returns no ID: exit 3 (got $rc)" test "$rc" -eq 3 || return 1
  need "(f) no ID means no ownership: nothing was removed (rm: $(rms))" test "$(rms)" = 0 || return 1
  nested d EXISTS_AFTER_RM_RC=125
  need "(d) an inspection error after the removal fails the cleanup: exit 4 (got $rc)" test "$rc" -eq 4 || return 1
  need "(d) the removal was by the returned ID" grep -q 'podman rm --force --volumes 0123456789abcdef0123456789abcdef' "$T/podman.log" || return 1
  need "(d) and the message says the absence could not be verified: $(tail -n 1 "$T/d.err")" grep -q 'cannot verify' "$T/d.err" || return 1
  nested e EXISTS_AFTER_RM_RC=1
  need "(e) control: removal confirmed absent: the setup failure (smoke data) stays exit 3 (got $rc)" test "$rc" -eq 3 || return 1
  need "(e) and the container was removed by ID" grep -q 'podman rm --force --volumes 0123456789abcdef0123456789abcdef' "$T/podman.log" || return 1
}

# ---- the status-only directory (vera's ruling C, cox 17:59Z): LAST_GOOD is published there and nowhere else ------------------------------

mv_spy() { # a `mv` that records the rename onto LAST_GOOD (where the temp file is, its mode and its completeness AT that moment) and can fail it
  cat > "$T/bin/mv" <<'SHIM'
#!/bin/sh
src="" last=""
for a in "$@"; do src=$last; last=$a; done
case $last in
  */LAST_GOOD)
    {
      echo "src=$src"
      echo "dst=$last"
      echo "srcdir=$(dirname "$src")"
      echo "dstdir=$(dirname "$last")"
      echo "mode=$(stat -c %a "$src")"
      echo "lines=$(wc -l < "$src")"
      echo "type=$(stat -c %F "$src")"
    } > "$MV_RECORD"
    if [ "${MV_FAIL:-0}" = 1 ]; then exit 1; fi ;;
esac
exec /usr/bin/mv "$@"
SHIM
  chmod 0755 "$T/bin/mv"
}

lg_shape='^\{"set":"[0-9]{8}T[0-9]{6}Z(-[0-9]{2})?","utc":"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z","bytes":[0-9]+,"sha256":"[0-9a-f]{64}"\}$'

bk_51() { # LAST_GOOD is published ONLY into --status-dir: a temp file inside it with its final mode, an atomic rename, nothing else ever written there
  new_test
  st=$T/st
  mkdir -p "$st"
  # (1) a good run, under two umasks: a regular file, mode 644 whatever the umask, one line of the recorded shape, and nothing else in the directory
  for um in 077 000; do
    ( umask "$um"; dump "u$um" --status-dir "$st"; echo "$rc" > "$T/rc.$um" )
    need "umask $um: a good run (rc=$(cat "$T/rc.$um")): $(tail -n 1 "$T/u$um.err")" test "$(cat "$T/rc.$um")" -eq 0 || return 1
    need "umask $um: the status directory holds LAST_GOOD and nothing else (got: $(ls -A "$st" | tr '\n' ' '))" test "$(ls -A "$st")" = LAST_GOOD || return 1
    need "umask $um: a regular file, mode 644 (got $(stat -c '%F %a' "$st/LAST_GOOD"))" test "$(stat -c '%F %a' "$st/LAST_GOOD")" = "regular file 644" || return 1
    need "umask $um: one line of the recorded shape" sh -c "[ \$(wc -l < '$st/LAST_GOOD') -eq 1 ] && grep -Eq '$lg_shape' '$st/LAST_GOOD'" || return 1
    sleep 1
  done
  need "the dump directory has no LAST_GOOD and no temp of one: $(ls -A "$T/out" | tr '\n' ' ')" test -z "$(ls -A "$T/out" | grep -i 'LAST_GOOD' || true)" || return 1
  need "the dump directory is private (0700), no mount ever sees it (got $(stat -c %a "$T/out"))" test "$(stat -c %a "$T/out")" = 700 || return 1
  need "LAST_GOOD names the newest set" test "$(sed -n 's/.*"set":"\([^"]*\)".*/\1/p' "$st/LAST_GOOD")" = "$(good_sets | tail -n 1)" || return 1
  # (2) a failed run never refreshes (or touches) the stale LAST_GOOD
  h=$(sum_of "$st/LAST_GOOD")
  m=$(stat -c %Y "$st/LAST_GOOD")
  for mode in fail_half truncated truncated_after_toc; do
    DENV="FAKE_DUMP_MODE=$mode" dump "f$mode" --status-dir "$st"
    need "$mode: exits non-zero (got $rc)" test "$rc" -ne 0 || return 1
    need "$mode: LAST_GOOD is byte for byte the old one" test "$(sum_of "$st/LAST_GOOD")" = "$h" -a "$(stat -c %Y "$st/LAST_GOOD")" = "$m" || return 1
    need "$mode: nothing else appeared in the status directory (got: $(ls -A "$st" | tr '\n' ' '))" test "$(ls -A "$st")" = LAST_GOOD || return 1
  done
  DENV="FAKE_DUMPALL_FAIL=1" dump fdumpall --status-dir "$st"
  need "pg_dumpall failing: non-zero and LAST_GOOD untouched" test "$rc" -ne 0 -a "$(sum_of "$st/LAST_GOOD")" = "$h" -a "$(ls -A "$st")" = LAST_GOOD || return 1
  # (3) a failing rename: the temp was INSIDE the status directory, complete and mode 644 at the moment of the rename; it does not survive; the old file stays
  mv_spy
  DENV="MV_RECORD=$T/mv.rec MV_FAIL=1" dump fmv --status-dir "$st"
  need "a failing rename: exit 1 (got $rc)" test "$rc" -eq 1 || return 1
  need "the status directory is as before: only LAST_GOOD, the same bytes (got: $(ls -A "$st" | tr '\n' ' '))" test "$(ls -A "$st")" = LAST_GOOD -a "$(sum_of "$st/LAST_GOOD")" = "$h" || return 1
  need "the rename was onto <status dir>/LAST_GOOD from a file in the SAME directory (atomic): $(tr '\n' ' ' < "$T/mv.rec")" sh -c "grep -qx 'dstdir=$st' '$T/mv.rec' && grep -qx 'srcdir=$st' '$T/mv.rec'" || return 1
  need "the temp already had its final mode (644), was a regular file and held the complete line at that moment" sh -c "grep -qx 'mode=644' '$T/mv.rec' && grep -qx 'lines=1' '$T/mv.rec' && grep -qx 'type=regular file' '$T/mv.rec'" || return 1
  # (4) a status directory that cannot be written is refused BEFORE anything is dumped
  chmod 0555 "$st"
  : > "$T/podman.log"
  DENV="" dump ro --status-dir "$st"
  rcro=$rc
  chmod 0755 "$st"
  need "a read-only status directory: exit 2 (got $rcro)" test "$rcro" -eq 2 || return 1
  need "and pg_dump never started" test -z "$(grep ' pg_dump -U' "$T/podman.log" || true)" || return 1
}

bk_52() { # the status directory is validated before anything is made, and is never derived from --out
  new_test
  st=$T/st
  mkdir -p "$st" "$T/out"
  : > "$T/podman.log"
  check_refused() { # check_refused <label> <fragment of the reason> <args...>: each guard is tested by ITS OWN reason, not only by the exit code (a later guard also exits 2)
    _l=$1
    _frag=$2
    shift 2
    : > "$T/podman.log"
    dump "x$_l" "$@"
    need "$_l: refused with exit 2 (got $rc): $(tail -n 1 "$T/x$_l.err")" test "$rc" -eq 2 || return 1
    need "$_l: and the reason names that guard ('$_frag'): $(tail -n 1 "$T/x$_l.err")" grep -q -- "$_frag" "$T/x$_l.err" || return 1
    need "$_l: pg_dump never started and no set was made" test -z "$(grep ' pg_dump -U' "$T/podman.log" || true)" -a "$(count_good_sets)" -eq 0 || return 1
    need "$_l: nothing was written into the status directory (got: $(ls -A "$st" | tr '\n' ' '))" test -z "$(ls -A "$st")" || return 1
  }
  sdflag=0 # no --status-dir from the helper: neither flag nor variable
  check_refused none "is required for a daily dump" || { sdflag=1; return 1; }
  sdflag=1
  check_refused relative "must be an absolute path" --status-dir st || return 1
  check_refused dot "must not contain . or .. components" --status-dir "$st/." || return 1
  check_refused dotdot "must not contain . or .. components" --status-dir "$T/x/../st" || return 1
  check_refused missing "not an existing directory" --status-dir "$T/no-such-dir" || return 1
  : > "$T/afile"
  check_refused notdir "not an existing directory" --status-dir "$T/afile" || return 1
  check_refused same "different directory from --out" --status-dir "$T/out" || return 1
  mkdir -p "$T/out/inner"
  check_refused inside "must not be inside --out" --status-dir "$T/out/inner" || return 1
  ln -s "$T/out" "$T/lnk-out"
  check_refused alias "different directory from --out" --status-dir "$T/lnk-out" || return 1
  # the root directory in every spelling is refused BY THAT REASON (BK12-ST-03): the early guard strips ONE trailing slash, so // and a link
  # to / pass it; after physical resolution the status directory is / and the descendant pattern // misses every ordinary --out
  ln -s / "$T/lnk-root"
  check_refused root "must not be the root directory" --status-dir / || return 1
  check_refused rootslashes "must not be the root directory" --status-dir // || return 1
  check_refused rootlink "must not be the root directory" --status-dir "$T/lnk-root" || return 1
  # an --out inside the status directory is refused too (the sets must stay out of every mount)
  : > "$T/podman.log"
  run xo env PATH="$T/bin:$PATH" FAKE_LOG="$T/podman.log" $sut "$dump_bin" --target fakedb --out "$st/dumps" --status-dir "$st"
  need "--out inside the status directory: exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  need "and the reason names that guard: $(tail -n 1 "$T/xo.err")" grep -q 'must not be inside --status-dir' "$T/xo.err" || return 1
  need "and nothing was created there (got: $(ls -A "$st" | tr '\n' ' '))" test -z "$(ls -A "$st")" || return 1
  # the variable works, the flag wins over it, a labelled run needs neither
  sdflag=0
  DENV="CORTEX_DUMP_STATUS=$st" dump envonly
  sdflag=1
  need "CORTEX_DUMP_STATUS alone is enough (rc=$rc)" test "$rc" -eq 0 -a -f "$st/LAST_GOOD" || return 1
  st2=$T/st2
  mkdir -p "$st2"
  DENV="CORTEX_DUMP_STATUS=$T/no-such-dir" dump flagwins --status-dir "$st2"
  need "the flag wins over a bad variable (rc=$rc)" test "$rc" -eq 0 -a -f "$st2/LAST_GOOD" || return 1
  h=$(sum_of "$st/LAST_GOOD")
  sdflag=0
  DENV="" dump lab --label pre-upgrade-a-to-b
  need "a labelled run needs no status directory (rc=$rc)" test "$rc" -eq 0 || return 1
  DENV="CORTEX_DUMP_STATUS=$st" dump lab2 --label pre-upgrade-b-to-c
  sdflag=1
  need "a labelled run writes nothing into the status directory" test "$(sum_of "$st/LAST_GOOD")" = "$h" -a "$(ls -A "$st")" = LAST_GOOD || return 1
  # a link (to a directory) at LAST_GOOD is replaced, never followed
  mkdir -p "$T/linkdest"
  rm -f "$st/LAST_GOOD"
  ln -s "$T/linkdest" "$st/LAST_GOOD"
  DENV="" dump lnk --status-dir "$st"
  need "a link at LAST_GOOD: a good run (rc=$rc)" test "$rc" -eq 0 || return 1
  need "LAST_GOOD is a regular file now and the link's target stayed empty (got: $(ls -A "$T/linkdest" | tr '\n' ' '))" test -f "$st/LAST_GOOD" -a ! -L "$st/LAST_GOOD" -a -z "$(ls -A "$T/linkdest")" || return 1
}

bk_53() { # cortex-dump-status reads LAST_GOOD from --status-dir (or CORTEX_DUMP_STATUS); --out is only for --verify and never supplies LAST_GOOD
  new_test
  st=$T/st
  mkdir -p "$st" "$T/out"
  dump good --status-dir "$st"
  need "a good dump" test "$rc" -eq 0 || return 1
  run s1 env PATH="$T/bin:$PATH" $sut "$status_bin" --status-dir "$st"
  need "--status-dir alone: fresh, exit 0 (got $rc): $(cat "$T/s1.out")" test "$rc" -eq 0 || return 1
  run s2 env PATH="$T/bin:$PATH" CORTEX_DUMP_STATUS="$st" $sut "$status_bin"
  need "CORTEX_DUMP_STATUS alone: exit 0 (got $rc)" test "$rc" -eq 0 || return 1
  run s3 env PATH="$T/bin:$PATH" $sut "$status_bin"
  need "neither: usage error, exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  run s4 env PATH="$T/bin:$PATH" $sut "$status_bin" --status-dir "$st" --verify
  need "--verify without --out: exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  run s5 env PATH="$T/bin:$PATH" $sut "$status_bin" --status-dir "$st" --out "$T/out" --verify
  need "--verify with --out finds the set (rc=$rc): $(cat "$T/s5.out")" test "$rc" -eq 0 || return 1
  # a LAST_GOOD in the dump directory is never read: seed it in --out, the place a fallback would look, and qualify it as readable and fresh
  cp -p "$st/LAST_GOOD" "$T/out/LAST_GOOD"
  run s6q env PATH="$T/bin:$PATH" $sut "$status_bin" --status-dir "$T/out"
  need "control: the marker seeded in --out is itself readable and fresh (exit 0, got $rc): $(cat "$T/s6q.out")" test "$rc" -eq 0 || return 1
  mkdir -p "$T/st-empty"
  run s6 env PATH="$T/bin:$PATH" $sut "$status_bin" --status-dir "$T/st-empty" --out "$T/out"
  need "a LAST_GOOD in --out is ignored: stale, exit 1 (got $rc): $(cat "$T/s6.out")" test "$rc" -eq 1 || return 1
  need "and the reason is that none is recorded" grep -q 'no last good dump is recorded' "$T/s6.out" || return 1
}

bk_56() { # BK12-ST-01: a CORTEX_DUMP_STATUS inherited from the caller's environment never reaches a test: an installed status directory exported in the shell is not written, read or listed
  new_test
  ext=$T/installed-status
  mkdir -p "$ext" "$T/nested"
  printf '{"set":"20260101T000000Z","utc":"2026-01-01T00:00:00Z","bytes":1,"sha256":"%s"}\n' "$(printf 'installed' | sha256sum | cut -d' ' -f1)" > "$ext/LAST_GOOD"
  chmod 0644 "$ext/LAST_GOOD"
  printf 'sibling\n' > "$ext/sibling"
  (cd "$ext" && sha256sum LAST_GOOD sibling && ls -A) > "$T/ext.before" 2>&1
  run n env PATH="$T/bin:$PATH" CORTEX_DUMP_STATUS="$ext" CORTEX_BK_TMP="$T/nested" $sut "$here/cortex-dump-test.sh" --shell sh BK-52 BK-53 < /dev/null
  need "the nested BK-52 and BK-53 pass although the caller exported CORTEX_DUMP_STATUS (got $rc): $(tail -n 3 "$T/n.err" "$T/n.out" | tr '\n' ' ')" test "$rc" -eq 0 || return 1
  (cd "$ext" && sha256sum LAST_GOOD sibling && ls -A) > "$T/ext.after" 2>&1 || true
  need "the installed marker and its directory are unchanged (diff: $(diff "$T/ext.before" "$T/ext.after" | head -n 3 | tr '\n' ' '))" cmp -s "$T/ext.before" "$T/ext.after" || return 1
}

# ---- the DB group: a real throwaway Postgres (--db) ---------------------------------------------------------

rid=$(od -An -N4 -tx1 /dev/urandom | tr -d ' \n') # this run's id: every container below carries it, so no other run's container can be ours
bkp=cv2-s5-bk-$rid
db_name=$bkp-src
src2_name=$bkp-src2
# the SAME image and initdb flags as hue's deploy/compose.yml service `db` (cv2-s5-db): the tool must be proved against the database it will run on
db_image=docker.io/pgvector/pgvector:0.8.2-pg18
db_initdb_args='--auth-local=peer --auth-host=scram-sha-256 -E UTF8 --locale-provider=builtin --locale=C.UTF-8 --builtin-locale=C.UTF-8 --data-checksums'
# The S2 inputs are the SEALED revision-8 pin (cox, wave1-pins-20261106-S2r8; read-only, with its own SHA256SUMS), not the living
# data-contract folder: when S2 revision 9 superseded revision 8 the living evidence path was renamed and the DB group could no longer
# set up (BK-50 d failed at the unchanged base 39b0bbd). The candidate.SHA256SUMS check below still stops a pack that is not rev8's.
s2_dir=${CORTEX_S2_PACK_DIR:-/path/to/docs/mvp/wave1-pins-20261106-S2r8/S2}
s2_evidence=${CORTEX_S2_EVIDENCE_DIR:-/path/to/docs/mvp/wave1-pins-20261106-S2r8/receipts/rev8}
db_started=0
holder=""
started=""                                      # name=ID of every container THIS run started, got an ID for and has not yet seen removed (nothing else is ever removed)
reg_add() { started="$started $1=$2"; }
id_of() { # the LATEST live entry of that name: a name may be used again after its first container was removed
  _found=""
  for _q in $started; do
    if [ "${_q%%=*}" = "$1" ]; then _found=${_q#*=}; fi
  done
  [ -n "$_found" ] || return 1
  printf '%s\n' "$_found"
}
reg_retire() { # reg_retire <id>: forget the entry whose removal is confirmed (and only that one)
  _kept=""
  for _q in $started; do
    if [ "${_q#*=}" != "$1" ]; then _kept="$_kept $_q"; fi
  done
  started=$_kept
}
must_be_absent() { # must_be_absent <name>: only `podman container exists` exit 1 is a proof of absence; 0 means taken, anything else means unknown
  _e=0
  podman container exists "$1" > /dev/null 2>&1 || _e=$?
  case $_e in
    1) return 0 ;;
    0) echo "cortex-dump-test: a container named $1 already exists and this run did not start it; not touching it (setup failed, nothing proven)" >&2 ;;
    *) echo "cortex-dump-test: $1 cannot be checked (podman container exists exit $_e); refusing, nothing was started" >&2 ;;
  esac
  return 1
}
fx=$base/fixture                                # the main drill's shared state (BK-14D builds it, later tests read it)
fx_state=new                                    # new | built | broken

cleanup_failed=0
rm_id() { # rm_id <name> <id>: remove that container BY ITS ID, check it is gone; a confirmed removal retires the entry, anything else keeps it and fails the cleanup
  podman rm --force --volumes "$2" > /dev/null 2>&1 || true
  _e=0
  podman container exists "$2" > /dev/null 2>&1 || _e=$? # not a bare command: under set -e a status of 125 would end the shell with 125 instead of reaching the case
  case $_e in
    1) reg_retire "$2" ;;
    0) echo "cortex-dump-test: $1 ($2) still exists after removal; remove it by hand: podman rm --force --volumes $2" >&2; cleanup_failed=1 ;;
    *) echo "cortex-dump-test: cannot verify that $1 ($2) is gone (podman container exists exit $_e); check by hand: podman ps -a" >&2; cleanup_failed=1 ;;
  esac
}
rm_checked() { # rm_checked <name>: remove the LATEST live container THIS run started under that name
  _id=$(id_of "$1") || return 0 # this run holds no live entry for that name: it started nothing under it that is still ours to remove
  rm_id "$1" "$_id"
}

reg_down() { # remove EVERY remaining registry entry, each by its own ID (the list is expanded once, retiring inside the loop is safe)
  for _p in $started; do rm_id "${_p%%=*}" "${_p#*=}"; done
}
db_down() { # the cleanup of the --db group, which owns its database reservation, its containers and its fixture
  if [ -n "$holder" ]; then kill "$holder" 2> /dev/null || true; holder=""; fi
  reg_down
  db_started=0
  case $fx in "$base"/fixture) rm -rf -- "$fx" ;; esac
  [ "$cleanup_failed" -eq 0 ] || exit 4
}
selftest_down() { # the cleanup of --selftest: the REGISTRY only. That mode creates and owns no fixture, no holder and no database, and the base
  reg_down      # directory may hold another run's material, so nothing but the recorded (synthetic) removals may happen here
  [ "$cleanup_failed" -eq 0 ] || exit 4
}

dbx() { podman exec --user postgres "$db_name" "$@"; }                       # a command in the container as the postgres OS user
dbsql() { podman exec -i --user postgres "$db_name" psql -U postgres -d cortex -X -q -v ON_ERROR_STOP=1 "$@"; }
qx() { podman exec --user postgres "$1" psql -U postgres -d "$2" -X -A -t -q -v ON_ERROR_STOP=1 -c "$3"; }   # qx <container> <db> <sql>

# mk_pack_db <name> <smoke 1|0>: a container from hue's image and flags, the roles, the six pack files as cortex_owner, and (1) the pack's smoke data
mk_pack_db() {
  _c=$1
  _smoke=$2
  _sql() { podman exec -i --user postgres "$_c" psql -U postgres -d cortex -X -q -v ON_ERROR_STOP=1 "$@"; }
  pw=$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')
  must_be_absent "$_c" || exit 3
  _cid=$(podman run -d --rm --name "$_c" --label cortexv2=dev --pull=never --memory 2g --cpus 1 \
    -e POSTGRES_DB=cortex -e POSTGRES_INITDB_ARGS="$db_initdb_args" -e POSTGRES_PASSWORD="$pw" "$db_image") ||
    { echo "cortex-dump-test: the database container $_c did not start" >&2; exit 3; }
  [ -n "$_cid" ] || { echo "cortex-dump-test: podman returned no ID for $_c" >&2; exit 3; }
  reg_add "$_c" "$_cid"
  # readiness over TCP, as hue's healthcheck does: the image first runs a temporary server on the socket only; the real one listens on TCP.
  # (This probe is the test harness; cortex-dump, cortex-restore and cortex-dump-status never use TCP.)
  i=0
  until podman exec "$_c" pg_isready -q -h 127.0.0.1 -U postgres -d cortex > /dev/null 2>&1; do
    i=$((i + 1))
    [ "$i" -lt 240 ] || { echo "cortex-dump-test: $_c was not ready in 120 s" >&2; exit 3; }
    sleep 0.5
  done
  # roles (the socket's provisioning), then the six files as cortex_owner, then the pack's own smoke data
  sed 's/DATABASE postgres/DATABASE cortex/' "$s2_evidence/00-roles.sql" | _sql > /dev/null || { echo "cortex-dump-test: roles failed" >&2; exit 3; }
  for f in platform identity tokens audit records handoffs; do
    { printf 'SET ROLE cortex_owner;\n'; cat "$s2_dir/$f.sql"; } > "$base/apply-$f.sql"
    _sql < "$base/apply-$f.sql" > /dev/null 2> "$base/apply-$f.err" || { echo "cortex-dump-test: applying $f.sql failed: $(head -c 300 "$base/apply-$f.err")" >&2; exit 3; }
  done
  if [ "$_smoke" = 1 ]; then
    _sql < "$s2_evidence/90-smoke.sql" > "$base/smoke.out" 2>&1 || { echo "cortex-dump-test: the smoke data failed: $(tail -n 3 "$base/smoke.out")" >&2; exit 3; }
    grep -q 'SMOKE PASSED' "$base/smoke.out" || { echo "cortex-dump-test: the smoke data did not report SMOKE PASSED" >&2; exit 3; }
  fi
  rm -f "$base"/apply-*.sql "$base"/apply-*.err "$base/smoke.out"
}

db_up() {
  [ "${CV2_DB:-0}" = 1 ] || { echo "cortex-dump-test: --db needs the database reservation: CV2_DB=1 cv2-admit $0 --db" >&2; exit 2; }
  command -v podman > /dev/null 2>&1 || { echo "cortex-dump-test: podman is not installed" >&2; exit 3; }
  # the fixture is the S2 pack exactly as rev8's smoke test ran it: refuse if it moved
  (cd "$s2_dir" && sha256sum -c "$s2_evidence/candidate.SHA256SUMS" > /dev/null 2>&1) || {
    echo "cortex-dump-test: the S2 pack in $s2_dir is not the one rev8's smoke test ran; update the pin (setup failed, nothing proven)" >&2
    exit 3
  }
  # the names carry this run's id: if one exists anyway it is not ours, and it is left alone
  for c in "$db_name" "$src2_name" $tgt_names; do must_be_absent "$c" || exit 3; done
  db_started=1
  mk_pack_db "$db_name" 1
  # what a production cluster has and the pack does not: the application login role with its memberships (db-roles.sh makes it), so the
  # disaster-recovery restore has to bring them from globals.sql alone; and one canary row of project B in records, for the isolation read
  app_pw=$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')
  printf "create role cortex_app login password '%s';\ngrant cortex_m_records, cortex_m_handoffs to cortex_app;\n" "$app_pw" | dbsql > /dev/null ||
    { echo "cortex-dump-test: creating cortex_app failed" >&2; exit 3; }
  dbsql > /dev/null 2> "$base/canary.err" <<'SQL' || { echo "cortex-dump-test: the project B canary failed: $(head -c 300 "$base/canary.err")" >&2; exit 3; }
set role cortex_m_records;
select set_config('cortex.project_id', 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb', false);
with x as (select uuidv7() as i)
insert into records.records (id, project_id, created_at, type, provenance, author_kind, author_id, written_by, summary, body,
                             summary_sha256, body_sha256, fields_sha256, section, section_revision, source_kind, source_ref)
select i, 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb'::uuid, uuid_extract_timestamp(i), 'note', 'worker_written', 'worker', 'w1', i,
       'canary of project b', 'canary body of project b', sha256('s'::bytea), sha256('b'::bytea), sha256('f'::bytea), NULL, NULL, NULL, NULL
  from x;
SQL
  # the smoke data leaves five tables empty (platform.migrations, idempotency, objects, refs, handoffs.evidence); an empty table is equal to an
  # empty table whatever the restore did, so give each of them rows (as the superuser, who bypasses row-level security): BK-15 names the
  # migration ledger and the idempotency rows
  dbsql > /dev/null 2> "$base/seed.err" <<'SQL' || { echo "cortex-dump-test: seeding the empty tables failed: $(head -c 300 "$base/seed.err")" >&2; exit 3; }
insert into platform.migrations (module, version, name, checksum, applied_by)
values ('platform', 1, 'bk-seed-platform', 'sha256:bk-seed-1', 'cortex_owner'), ('handoffs', 1, 'bk-seed-handoffs', 'sha256:bk-seed-2', 'cortex_owner');
insert into platform.idempotency (principal_id, key, project_id, route, request_sha256, status, response)
values ('cccccccc-cccc-4ccc-8ccc-cccccccccccc', 'bk-key-1', 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', 'POST /v2/handoffs', sha256('r1'::bytea), 201, '{"ok": true}'::jsonb),
       ('cccccccc-cccc-4ccc-8ccc-cccccccccccc', 'bk-key-2', 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb', 'POST /v2/records', sha256('r2'::bytea), NULL, NULL);
insert into platform.objects (id, project_id, kind, module)
values ('90000000-0000-4000-8000-000000000001', 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', 'record', 'records'),
       ('90000000-0000-4000-8000-000000000002', 'bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb', 'handoff', 'handoffs');
insert into platform.refs (project_id, from_id, to_id, kind)
values ('aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', '90000000-0000-4000-8000-000000000001', '90000000-0000-4000-8000-000000000002', 'bk-ref');
insert into handoffs.evidence (id, return_id, project_id, kind, ref, inline, sha256, size)
select uuidv7(), r.id, r.project_id, 'test', 'bk-evidence', null, sha256('e'::bytea), 0 from handoffs.returns r limit 1;
SQL
  rm -f "$base/canary.err" "$base/seed.err"
}

new_test_db() { # a test directory with NO fake podman: the real one is used
  T=$(mktemp -d "$base/t.XXXXXX")
  mkdir "$T/bin" "$T/status"
  sdflag=1
  : > "$T/podman.log"
  DENV=""
}

real_dump() { # real_dump <name> [args]
  _n=$1
  shift
  # shellcheck disable=SC2086
  run "$_n" env $DENV $sut "$dump_bin" --target "$db_name" --out "$T/out" --status-dir "$T/status" "$@"
}

toc_of() { podman exec -i --user postgres "$db_name" pg_restore --list < "$1"; }

bk_01d() { # a populated database: the set has all three files, verifies, and holds the schema, the policies, the grants and the roles
  new_test_db
  old_umask=$(umask)
  umask 022
  real_dump a
  umask "$old_umask"
  need "exit 0 (got $rc): $(tail -n 3 "$T/a.err")" test "$rc" -eq 0 || return 1
  s=$(last_good_set)
  need "LAST_GOOD names a set" test -n "$s" || return 1
  need "the set verifies (platform.dump, globals.sql, SHA256SUMS)" set_ok "$T/out/$s" || return 1
  toc_of "$T/out/$s/platform.dump" > "$T/toc.txt" 2> "$T/toc.err" || { echo "    pg_restore --list failed: $(head -c 200 "$T/toc.err")" >&2; return 1; }
  # a TOC line is `<id>; <oid> <oid> <TYPE> <schema> <name> <owner>`: anchor on the type field
  toc_n() { grep -cE "^[0-9]+; [0-9]+ [0-9]+ $1 " "$T/toc.txt" || true; }
  tables=$(toc_n 'TABLE (audit|handoffs|identity|platform|records|tokens)')
  # S2 revision 7 (cox W1-14) put the two search views INSIDE schema records: records.pub_sources and records.pub_state
  # (the definitions r0.2 still say schema records_pub, from revision 6)
  views=$(toc_n 'VIEW records pub_(sources|state)')
  if [ "$tables" -ne 23 ]; then
    echo "    the TABLE entries of the archive:" >&2
    grep -E '^[0-9]+; [0-9]+ [0-9]+ TABLE (audit|handoffs|identity|platform|records|tokens) ' "$T/toc.txt" | sed 's/^.* TABLE /      /' >&2
  fi
  need "23 base tables in the archive (got $tables)" test "$tables" -eq 23 || return 1
  need "the 2 search views are in the archive (got $views)" test "$views" -eq 2 || return 1
  rls=$(toc_n 'ROW SECURITY')
  pol=$(toc_n 'POLICY')
  acl=$(toc_n 'ACL')
  echo "    archive: $tables tables, $views views, $rls row-security entries, $pol policies, $acl grants" >&2
  need "row-level security is in the archive (got $rls)" test "$rls" -ge 15 || return 1
  need "policies are in the archive (got $pol)" test "$pol" -ge 15 || return 1
  need "grants are in the archive, so no --no-acl (got $acl)" test "$acl" -ge 5 || return 1
  for r in cortex_owner cortex_m_handoffs cortex_m_records cortex_install_admin; do
    need "globals.sql creates role $r" grep -q "CREATE ROLE $r" "$T/out/$s/globals.sql" || return 1
  done
  need "set directory 0700, files 0600 under a permissive umask" test "$(stat -c %a "$T/out/$s")$(stat -c %a "$T/out/$s/platform.dump")$(stat -c %a "$T/out/$s/globals.sql")" = 700600600 || return 1
  need "no role hash on stdout or stderr" test -z "$(grep -h 'SCRAM-SHA-256' "$T/a.out" "$T/a.err" || true)" || return 1
  status st --verify
  need "cortex-dump-status --verify agrees (rc=$rc)" test "$rc" -eq 0 || return 1
}

bk_20d() { # a labelled dump on the real database
  new_test_db
  real_dump l --label pre-upgrade-0.1.0-to-0.2.0
  need "exit 0 (got $rc): $(tail -n 3 "$T/l.err")" test "$rc" -eq 0 || return 1
  p=$(last_line "$T/l.out")
  need "the set is under pre-upgrade/ and verifies" set_ok "$p" || return 1
  need "LAST_GOOD was not written by a labelled run" test ! -e "$T/status/LAST_GOOD" || return 1
}

bk_24d() { # --timeout against a REAL pg_dump blocked on a lock: exit 4, no pg_dump backend left, no temp, no lock
  new_test_db
  PGAPPNAME=bk-lock-holder podman exec -e PGAPPNAME --user postgres "$db_name" psql -U postgres -d cortex -Atc \
    "begin; lock table platform.objects in access exclusive mode; select pg_sleep(40); commit;" > /dev/null 2>&1 &
  holder=$!
  i=0
  until [ "$(dbx psql -U postgres -d cortex -Atc "select count(*) from pg_locks l join pg_stat_activity a using (pid) where a.application_name = 'bk-lock-holder' and l.mode = 'AccessExclusiveLock'")" -ge 1 ] 2> /dev/null; do
    i=$((i + 1))
    [ "$i" -lt 60 ] || { echo "    CHECK FAILED: the lock holder never took the lock" >&2; return 1; }
    sleep 0.5
  done
  t0=$(date +%s)
  real_dump t --timeout 5
  t1=$(date +%s)
  need "exit 4 (got $rc): $(tail -n 2 "$T/t.err")" test "$rc" -eq 4 || return 1
  need "within 25 s ($((t1 - t0)) s)" test $((t1 - t0)) -lt 25 || return 1
  sleep 1
  left=$(dbx psql -U postgres -d cortex -Atc "select count(*) from pg_stat_activity where application_name = 'pg_dump'")
  need "no pg_dump backend is left in the database (got $left)" test "$left" -eq 0 || return 1
  need "no temp directory" test -z "$(temp_left)" || return 1
  need "the lock is released (flock, not the old .lock directory)" lock_free "$T/out" || return 1
  need "no set" test "$(count_good_sets)" -eq 0 || return 1
  # let go of the table: end the holder's backend by the marker it set, then its client by its exact pid
  dbx psql -U postgres -d cortex -Atc "select pg_terminate_backend(pid) from pg_stat_activity where application_name = 'bk-lock-holder'" > /dev/null 2>&1 || true
  kill "$holder" 2> /dev/null || true
  holder=""
}

# ---- the restore tests (BK-14 to BK-17, 27, 28, 30, 31, 33 to 36): each target is a fresh EMPTY cluster ------------------------------

A_ID=aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa      # the S2 smoke values
B_ID=bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb
INST_ID=0a0a0a0a-0a0a-4a0a-8a0a-0a0a0a0a0a0a
W2_ID=dddddddd-dddd-4ddd-8ddd-dddddddddddd
TID_ID=11111111-1111-4111-8111-111111111111
T1_ID=71111111-1111-4111-8111-111111111111
T2_ID=72222222-2222-4222-8222-222222222222
T3_ID=73333333-3333-4333-8333-333333333333
tgt_main=$bkp-t-main
tgt_names="$bkp-t-main $bkp-t-roles $bkp-t-label $bkp-t-trust $bkp-t-b31 $bkp-t-33b $bkp-t-35a $bkp-t-35b $bkp-t-ch2 $bkp-t-ch3"
gen_line='install generation 3 -> 4: every token issued before this restore is refused (401 token_revoked); re-issue the lead token with install.sh --reissue-token'

tgt_up() { # tgt_up <name> [plain]: a fresh EMPTY cluster on hue's flags ("plain" = the image's defaults, which leave a trust rule)
  _tn=$1
  _args=$db_initdb_args
  [ "${2:-}" = plain ] && _args=""
  pw=$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')
  must_be_absent "$_tn" || return 1
  _cid=$(podman run -d --rm --name "$_tn" --label cortexv2=dev --pull=never --memory 1g --cpus 1 \
    -e POSTGRES_INITDB_ARGS="$_args" -e POSTGRES_PASSWORD="$pw" "$db_image") || { echo "    CHECK FAILED: $_tn did not start" >&2; return 1; }
  [ -n "$_cid" ] || { echo "    CHECK FAILED: podman returned no ID for $_tn" >&2; return 1; }
  reg_add "$_tn" "$_cid"
  i=0
  until podman exec "$_tn" pg_isready -q -h 127.0.0.1 -U postgres > /dev/null 2>&1; do
    i=$((i + 1))
    [ "$i" -lt 240 ] || { echo "    CHECK FAILED: $_tn was not ready in 120 s" >&2; return 1; }
    sleep 0.5
  done
}
tgt_rm() { rm_checked "$1"; }

srcq() { qx "$db_name" cortex "$1"; }

# cr <name> <set> <target> [args]: the real cortex-restore, confirmed
cr() {
  _n=$1
  _set=$2
  _tg=$3
  shift 3
  run "$_n" env TMPDIR="$T" $sut "$restore_bin" --set "$_set" --target "$_tg" --confirm-new "$@"
}
rcmp() { _n=$1; shift; run "$_n" env TMPDIR="$T" $sut "$compare_bin" "$@"; }

rls_read() { # rls_read <container> <db>: "<rows of A> <rows of B>" as cortex_m_records sees them with the project set to A
  podman exec -i --user postgres "$1" psql -U postgres -d "$2" -X -A -t -q -v ON_ERROR_STOP=1 <<SQL | tail -n 1
set role cortex_m_records;
select set_config('cortex.project_id', '$A_ID', false);
select (select count(*) from records.records where project_id = '$A_ID') || ' ' || (select count(*) from records.records where project_id = '$B_ID');
SQL
}

vl() { # vl <container> <db> <token id>: "install_gen|current_install_gen|revoked_at is null", read as the verifier's role
  podman exec -i --user postgres "$1" psql -U postgres -d "$2" -X -A -t -q -v ON_ERROR_STOP=1 <<SQL | tail -n 1
set role cortex_m_tokens;
select coalesce(install_gen::text, 'null') || '|' || coalesce(current_install_gen::text, 'null') || '|' || (revoked_at is null)::text from tokens.verify_lookup('$3');
SQL
}

# the main drill, built once: dump the source, restore into a fresh cluster, compare, read across the isolation, time every step.
# CORTEX_RESTORE_SKIP_GENERATION=1 is set for the restore on purpose: the variable must have no effect (BK-33 a).
fx_up() {
  [ "$fx_state" = built ] && return 0
  [ "$fx_state" = broken ] && return 1
  fx_state=broken
  _saveT=$T
  case $fx in "$base"/fixture) rm -rf -- "$fx" ;; esac
  mkdir -p "$fx/out" "$fx/status"
  T=$fx
  if ! tgt_up "$tgt_main"; then T=$_saveT; return 1; fi
  fx_pg_before=$(qx "$tgt_main" postgres "select md5(coalesce(rolpassword, '')) from pg_authid where rolname = 'postgres'")
  t0=$(date +%s)
  run dump env $sut "$dump_bin" --target "$db_name" --out "$fx/out" --status-dir "$fx/status"
  t1=$(date +%s)
  fx_dump_rc=$rc
  fx_set=$(tail -n 1 "$fx/dump.out")
  fx_lg_before=$(sha256sum "$fx/status/LAST_GOOD" 2> /dev/null | cut -d' ' -f1 || true)
  run restore env TMPDIR="$fx" CORTEX_RESTORE_SKIP_GENERATION=1 $sut "$restore_bin" --set "$fx_set" --target "$tgt_main" --confirm-new
  t2=$(date +%s)
  fx_restore_rc=$rc
  fx_lg_after=$(sha256sum "$fx/status/LAST_GOOD" 2> /dev/null | cut -d' ' -f1 || true)
  run cmp env TMPDIR="$fx" $sut "$compare_bin" --source "$db_name" --target "$tgt_main" --after-restore
  t3=$(date +%s)
  fx_cmp_rc=$rc
  run iso env TMPDIR="$fx" $sut "$compare_bin" --source "$db_name" --target "$tgt_main" --isolation
  fx_iso_rc=$rc
  rls_read "$tgt_main" cortex > "$fx/rls.out" 2> /dev/null || true
  rls_read "$db_name" cortex > "$fx/rls.src.out" 2> /dev/null || true
  t4=$(date +%s)
  printf 'drill seconds: dump %d, restore %d, compare %d, isolation %d, total %d\n' $((t1 - t0)) $((t2 - t1)) $((t3 - t2)) $((t4 - t3)) $((t4 - t0)) > "$fx/drill.line"
  T=$_saveT
  fx_state=built
}

fx_or_fail() {
  fx_up || { echo "    CHECK FAILED: the drill fixture could not be built (a container did not start)" >&2; return 1; }
}

bk_14d() { # a fresh cluster (disaster recovery): every role, attribute, membership and password verifier comes from globals.sql alone
  new_test_db
  fx_or_fail || return 1
  need "cortex-dump of the source: exit 0 (got $fx_dump_rc)" test "$fx_dump_rc" -eq 0 || return 1
  need "cortex-restore into a fresh empty cluster: exit 0 (got $fx_restore_rc): $(tail -n 3 "$fx/restore.err")" test "$fx_restore_rc" -eq 0 || return 1
  qa="select rolname, rolsuper, rolinherit, rolcreaterole, rolcreatedb, rolcanlogin, rolreplication, rolbypassrls, rolconnlimit from pg_roles where rolname like 'cortex\\_%' order by 1"
  s_roles=$(qx "$db_name" cortex "$qa")
  t_roles=$(qx "$tgt_main" cortex "$qa")
  need "the cortex_* roles and their attributes are equal" test "$s_roles" = "$t_roles" || return 1
  need "at least 10 cortex_* roles exist in the target (got $(printf '%s\n' "$t_roles" | grep -c '^cortex_' || true))" test "$(printf '%s\n' "$t_roles" | grep -c '^cortex_' || true)" -ge 10 || return 1
  need "cortex_app is among them" test -n "$(printf '%s\n' "$t_roles" | grep '^cortex_app|' || true)" || return 1
  qm="select m.rolname || '>' || r.rolname || ':' || a.admin_option::text || a.inherit_option::text || a.set_option::text from pg_auth_members a join pg_roles m on m.oid = a.member join pg_roles r on r.oid = a.roleid where m.rolname like 'cortex\\_%' order by 1"
  s_mem=$(qx "$db_name" cortex "$qm")
  t_mem=$(qx "$tgt_main" cortex "$qm")
  need "the memberships are equal" test "$s_mem" = "$t_mem" || return 1
  need "cortex_app is a member of cortex_m_records" test -n "$(printf '%s\n' "$t_mem" | grep '^cortex_app>cortex_m_records:' || true)" || return 1
  # the REAL verifier lives in pg_authid; pg_roles shows ******** for everyone, even to the superuser, so a check on it sees nothing
  qp="select md5(rolpassword) from pg_authid where rolname = 'cortex_app'"
  need "cortex_app's password verifier is the source's (md5 compared; nothing printed)" test "$(qx "$tgt_main" cortex "$qp")" = "$(qx "$db_name" cortex "$qp")" || return 1
  need "cortex_app's verifier is a real SCRAM verifier, not empty and not a mask" test "$(qx "$tgt_main" cortex "select (rolpassword like 'SCRAM-SHA-256\$%')::text from pg_authid where rolname = 'cortex_app'")" = true || return 1
  need "the target's own postgres role was not altered" test "$(qx "$tgt_main" postgres "select md5(coalesce(rolpassword, '')) from pg_authid where rolname = 'postgres'")" = "$fx_pg_before" || return 1
  need "the new database is owned by cortex_owner" test "$(qx "$tgt_main" postgres "select pg_get_userbyid(datdba) from pg_database where datname = 'cortex'")" = cortex_owner || return 1
  need "no role hash in the output of the restore" test -z "$(grep -h 'SCRAM-SHA-256' "$fx/restore.out" "$fx/restore.err" || true)" || return 1
  need "a successful restore leaves no working directory behind (it held the role hashes)" test -z "$(ls -A "$fx" | grep '^cortex-restore\.' || true)" || return 1
  # control: this check sees a changed verifier (through pg_roles it would not). The fixture's cortex_app is changed for good; no later test reads it.
  qx "$tgt_main" cortex "alter role cortex_app password 'bk-changed-for-the-control'" > /dev/null || return 1
  need "control: a changed verifier is seen" test "$(qx "$tgt_main" cortex "$qp")" != "$(qx "$db_name" cortex "$qp")" || return 1
}

bk_14e() { # roles that already exist are left unchanged: no ALTER ROLE, no password overwrite; the missing ones are created
  new_test_db
  fx_or_fail || return 1
  tn=$bkp-t-roles
  tgt_up "$tn" || return 1
  podman exec -i --user postgres "$tn" psql -U postgres -d postgres -X -q -v ON_ERROR_STOP=1 < "$s2_evidence/00-roles.sql" > /dev/null || { echo "    CHECK FAILED: seeding the roles" >&2; tgt_rm "$tn"; return 1; }
  printf "alter role cortex_owner password 'bk-target-own-password';\nalter role cortex_install_admin connection limit 7;\nalter role cortex_owner createrole;\nalter role cortex_m_records login;\n" |
    podman exec -i --user postgres "$tn" psql -U postgres -d postgres -X -q -v ON_ERROR_STOP=1 > /dev/null || { echo "    CHECK FAILED: marking the roles" >&2; tgt_rm "$tn"; return 1; }
  snap="select rolname, md5(coalesce(rolpassword, '')), rolconnlimit, rolcanlogin, rolsuper, rolinherit, rolcreaterole from pg_authid where rolname in ('postgres', 'cortex_owner', 'cortex_m_platform', 'cortex_m_identity', 'cortex_m_tokens', 'cortex_m_audit', 'cortex_m_records', 'cortex_m_handoffs', 'cortex_m_search', 'cortex_install_admin') order by 1"
  before=$(qx "$tn" postgres "$snap")
  cr r "$fx_set" "$tn"
  need "exit 0 (got $rc): $(tail -n 3 "$T/r.err")" test "$rc" -eq 0 || { tgt_rm "$tn"; return 1; }
  after=$(qx "$tn" postgres "$snap")
  need "the roles that existed are exactly as they were (password verifier, connection limit, attributes)" test "$before" = "$after" || { tgt_rm "$tn"; return 1; }
  need "cortex_app, which was missing, was created" test "$(qx "$tn" postgres "select count(*) from pg_roles where rolname = 'cortex_app'")" -eq 1 || { tgt_rm "$tn"; return 1; }
  need "and it has its memberships" test "$(qx "$tn" postgres "select count(*) from pg_auth_members a join pg_roles m on m.oid = a.member join pg_roles r on r.oid = a.roleid where m.rolname = 'cortex_app' and r.rolname = 'cortex_m_records'")" -eq 1 || { tgt_rm "$tn"; return 1; }
  need "the data is there and the generation raised: $(grep -c 'install generation 3 -> 4' "$T/r.out" || true) line" grep -q 'install generation 3 -> 4' "$T/r.out" || { tgt_rm "$tn"; return 1; }
  tgt_rm "$tn"
}

tweak_check() { # tweak_check <name> <data|iso> <tweak sql> <revert sql> <pattern the report must show>
  _n=$1
  qx "$tgt_main" cortex "$3" > /dev/null || { echo "    CHECK FAILED: the change '$_n' did not apply" >&2; return 1; }
  if [ "$2" = iso ]; then
    rcmp "$_n" --source "$db_name" --target "$tgt_main" --isolation
  else
    rcmp "$_n" --source "$db_name" --target "$tgt_main" --after-restore
  fi
  _rc=$rc
  qx "$tgt_main" cortex "$4" > /dev/null || { echo "    CHECK FAILED: undoing '$_n' did not apply (the fixture is damaged)" >&2; fx_state=broken; return 1; }
  need "$_n: the report fails (rc=1, got $_rc)" test "$_rc" -eq 1 || return 1
  need "$_n: and it names the difference ($5)" grep -Eq "$5" "$T/$_n.out" || return 1
}

COL1_ID=00000000-0000-0000-0000-00000000d791 # two distinct token ids whose md5(id) share the first 8 hex digits (f5eeba4d)
COL2_ID=00000000-0000-0000-0000-000000011ef0
src_clean_gen() { srcq "update tokens.generations set gen = 3 where scope = 'install'" > /dev/null 2>&1 || true; }
src_clean_collide() {
  srcq "delete from tokens.tokens where id in ('$COL1_ID', '$COL2_ID')" > /dev/null 2>&1 || true
  qx "$tgt_main" cortex "delete from tokens.tokens where id in ('$COL1_ID', '$COL2_ID')" > /dev/null 2>&1 || true
}
col_sql() { # col_sql <issued_by json of the first token>: two tokens whose short fingerprints collide
  echo "insert into tokens.tokens (id, worker_id, project_id, digest, install_gen, project_gen, issued_at, expires_at, renew_after, issued_by) values ('$COL1_ID', '$W2_ID', '$A_ID', sha256('c1'::bytea), 3, 7, '2026-11-06 00:00:00+00', '2026-12-06 00:00:00+00', '2026-11-20 00:00:00+00', '$1'::jsonb), ('$COL2_ID', '$W2_ID', '$A_ID', sha256('c2'::bytea), 3, 7, '2026-11-06 00:00:00+00', '2026-12-06 00:00:00+00', '2026-11-20 00:00:00+00', '{}'::jsonb)"
}

collide_control() {
  srcq "$(col_sql '{}')" > /dev/null || return 1
  qx "$tgt_main" cortex "$(col_sql '{}')" > /dev/null || return 1
  rcmp col0 --source "$db_name" --target "$tgt_main" --after-restore
  need "control: the colliding pair, identical on both sides, compares clean (rc=$rc)" test "$rc" -eq 0 || { cat "$T/col0.out" >&2; return 1; }
  qx "$tgt_main" cortex "delete from tokens.tokens where id in ('$COL1_ID', '$COL2_ID')" > /dev/null || return 1
  qx "$tgt_main" cortex "$(col_sql '{"c": 1}')" > /dev/null || return 1
  rcmp col1 --source "$db_name" --target "$tgt_main" --after-restore
  need "a changed token row behind a colliding short fingerprint is found (rc=1, got $rc)" test "$rc" -eq 1 || return 1
  need "and tokens.tokens is named: $(grep '^tokens.tokens' "$T/col1.out")" grep -Eq '^tokens\.tokens +FAIL .*different 1' "$T/col1.out" || return 1
}

bk_15d() { _bk_15d; _r=$?; src_clean_gen; src_clean_collide; src_clean_install_row; return "$_r"; }
_bk_15d() { # the equality report: every table, no aggregate-only pass, one expected +1; and it can fail
  new_test_db
  fx_or_fail || return 1
  echo "    the equality report (restore-compare --after-restore), as the drill printed it:" >&2
  sed 's/^/    | /' "$fx/cmp.out" >&2
  need "the comparison after the drill: exit 0 (got $fx_cmp_rc)" test "$fx_cmp_rc" -eq 0 || return 1
  need "no table of the source is empty (an empty table compares equal whatever the restore did): $(grep -E 'count 0 = 0' "$fx/cmp.out" | cut -d' ' -f1 | tr '\n' ' ')" test -z "$(grep -E 'count 0 = 0' "$fx/cmp.out" || true)" || return 1
  need "24 comparisons (23 tables and the tsv hash), none failed" grep -q '^RESULT: 24 comparisons, 0 failed$' "$fx/cmp.out" || return 1
  need "every table is named with its own PASS line (24)" test "$(grep -cE '^[a-z_]+\.[a-z_.]+ +PASS ' "$fx/cmp.out" || true)" -eq 24 || return 1
  need "the one expected difference is printed, with both values" grep -Eq '^tokens\.generations +PASS .*install row expected: \+1 \(source 3, target 4\)$' "$fx/cmp.out" || return 1
  need "and it is the only one" test "$(grep -c 'expected: +1' "$fx/cmp.out" || true)" -eq 1 || return 1
  need "records.records.tsv has its own line" grep -Eq '^records\.records\.tsv +PASS .*tsv hash equal' "$fx/cmp.out" || return 1
  need "the migration ledger and the idempotency rows are in it" sh -c "grep -q '^platform.migrations ' '$fx/cmp.out' && grep -q '^platform.idempotency ' '$fx/cmp.out'" || return 1
  # it can fail: equal counts with a different row; a moved project generation; an edited token row
  tweak_check c-row data "update identity.installations set label = label || 'x'" "update identity.installations set label = left(label, length(label) - 1)" '^identity\.installations +FAIL .*different 1' || return 1
  tweak_check c-gen data "update tokens.generations set gen = gen + 1 where scope = 'project'" "update tokens.generations set gen = gen - 1 where scope = 'project'" '^tokens\.generations +FAIL .*different 1' || return 1
  tweak_check c-token data "update tokens.tokens set issued_by = issued_by || jsonb_build_object('x', 1)" "update tokens.tokens set issued_by = issued_by - 'x'" '^tokens\.tokens +FAIL .*different 1' || return 1
  # a missing row, an extra row, an install generation that was not raised, one that was raised twice
  ref_at=$(srcq "select at from platform.refs where kind = 'bk-ref'")
  tweak_check c-missing data "delete from platform.refs where kind = 'bk-ref'" "insert into platform.refs (project_id, from_id, to_id, kind, at) values ('$A_ID', '90000000-0000-4000-8000-000000000001', '90000000-0000-4000-8000-000000000002', 'bk-ref', '$ref_at')" '^platform\.refs +FAIL .*missing 1' || return 1
  tweak_check c-extra data "insert into platform.objects (id, project_id, kind, module) values ('90000000-0000-4000-8000-000000000003', '$A_ID', 'record', 'records')" "delete from platform.objects where id = '90000000-0000-4000-8000-000000000003'" '^platform\.objects +FAIL .*extra 1' || return 1
  tweak_check c-notraised data "update tokens.generations set gen = gen - 1 where scope = 'install'" "update tokens.generations set gen = gen + 1 where scope = 'install'" '^tokens\.generations +FAIL .*install row NOT as expected: source 3, target 3' || return 1
  first_rec="select id from records.records order by id limit 1"
  tweak_check c-tsv data "alter table records.records disable trigger records_immutable; update records.records set summary = summary || ' bk' where id = ($first_rec); alter table records.records enable trigger records_immutable" "alter table records.records disable trigger records_immutable; update records.records set summary = left(summary, length(summary) - 3) where id = ($first_rec); alter table records.records enable trigger records_immutable" '^records\.records\.tsv +FAIL .*tsv hash differs' || return 1
  tweak_check c-twice data "update tokens.generations set gen = gen + 1 where scope = 'install'" "update tokens.generations set gen = gen - 1 where scope = 'install'" '^tokens\.generations +FAIL .*install row NOT as expected: source 3, target 5' || return 1
  fails=""
  # BK12-05: the +1 is waived for the install row's VALUE only: a changed scope_id with gen = source + 1 is a different row
  tweak_check c-scope data "update tokens.generations set scope_id = '0b0b0b0b-0b0b-4b0b-8b0b-0b0b0b0b0b0b' where scope = 'install'" "update tokens.generations set scope_id = '$INST_ID' where scope = 'install'" '^tokens\.generations +FAIL' || fails="$fails c-scope"
  # BK12-06: exact bigint arithmetic: a generation above 2^53 that was NOT raised must not pass as +1
  srcq "update tokens.generations set gen = 9007199254740992 where scope = 'install'" > /dev/null || return 1
  tweak_check c-bigint data "update tokens.generations set gen = 9007199254740992 where scope = 'install'" "update tokens.generations set gen = 4 where scope = 'install'" '^tokens\.generations +FAIL .*install row NOT as expected' || fails="$fails c-bigint"
  src_clean_gen
  # BK12-R1-02: a populated installation whose install generation is missing on BOTH sides is a restore that skipped the raise, not "nothing to protect"
  srcq "delete from tokens.generations where scope = 'install'" > /dev/null || return 1
  tweak_check c-bothmissing data "delete from tokens.generations where scope = 'install'" "insert into tokens.generations (scope, scope_id, gen) values ('install', '$INST_ID', 4)" '^tokens\.generations +FAIL .*install row NOT as expected' || fails="$fails c-bothmissing"
  src_clean_install_row
  # BK12-04: two tokens whose short fingerprints collide, identical on both sides, then the FIRST differs in the target
  collide_control || fails="$fails c-collide"
  src_clean_collide
  need "the controls of sam's findings 04, 05 and 06 each fail the report as they must (not:$fails)" test -z "$fails" || return 1
  rcmp again --source "$db_name" --target "$tgt_main" --after-restore
  need "after undoing the changes the report is clean again (rc=$rc)" test "$rc" -eq 0 || return 1
}

bk_16d() { # isolation survives: flags, policies, grants identical; project A cannot read B's canary; and it can fail
  new_test_db
  fx_or_fail || return 1
  echo "    the isolation report (restore-compare --isolation):" >&2
  sed 's/^/    | /' "$fx/iso.out" >&2
  need "the isolation comparison: exit 0 (got $fx_iso_rc)" test "$fx_iso_rc" -eq 0 || return 1
  for k in rls policy grant-table grant-column grant-schema grant-function view; do
    need "$k: PASS with entries" grep -Eq "^$k +PASS +[1-9][0-9]* entries, identical" "$fx/iso.out" || return 1
  done
  need "23 tables carry their row-security flags" grep -Eq '^rls +PASS +23 entries' "$fx/iso.out" || return 1
  need "the two search views are there" grep -Eq '^view +PASS +[2-9][0-9]* entries' "$fx/iso.out" || return 1
  need "the result line" grep -q '^RESULT: isolation equal' "$fx/iso.out" || return 1
  src_read=$(cat "$fx/rls.src.out")
  tgt_read=$(cat "$fx/rls.out")
  echo "    cortex_m_records with project A set, rows of A and of B: source '$src_read', restored '$tgt_read'" >&2
  need "positive control: A sees its own rows on the restored database (got '$tgt_read')" test "${tgt_read%% *}" -ge 1 || return 1
  need "B's canary rows are invisible on the restored database (got '$tgt_read')" test "${tgt_read#* }" -eq 0 || return 1
  need "the canary exists (as the superuser the target has B's rows)" test "$(qx "$tgt_main" cortex "select count(*) from records.records where project_id = '$B_ID'")" -ge 1 || return 1
  tweak_check c-rls iso "alter table tokens.tokens no force row level security" "alter table tokens.tokens force row level security" '^rls +FAIL' || return 1
  tweak_check c-grant iso "revoke select on tokens.generations from cortex_m_tokens" "grant select on tokens.generations to cortex_m_tokens" '^grant-table +FAIL' || return 1
  tweak_check c-viewopt iso "alter view records.pub_sources set (security_barrier = false)" "alter view records.pub_sources set (security_barrier = true)" '^view +FAIL' || return 1
  rcmp again --source "$db_name" --target "$tgt_main" --isolation
  need "after undoing the changes the report is clean again (rc=$rc)" test "$rc" -eq 0 || return 1
}

bk_17d() { # the timed drill: one line with seconds for each step and the total
  new_test_db
  fx_or_fail || return 1
  line=$(cat "$fx/drill.line")
  echo "    $line" >&2
  need "every step of the drill succeeded (dump $fx_dump_rc, restore $fx_restore_rc, compare $fx_cmp_rc, isolation $fx_iso_rc); a time printed for a failed step is no record" test "$fx_dump_rc$fx_restore_rc$fx_cmp_rc$fx_iso_rc" = 0000 || return 1
  need "one line with five whole numbers: $line" sh -c "printf '%s\n' '$line' | grep -Eq '^drill seconds: dump [0-9]+, restore [0-9]+, compare [0-9]+, isolation [0-9]+, total [0-9]+\$'" || return 1
}

bk_30d() { # the install generation goes 3 -> 4, in one line; the set and LAST_GOOD are untouched; no secret printed
  new_test_db
  fx_or_fail || return 1
  echo "    stdout of cortex-restore:" >&2
  sed 's/^/    | /' "$fx/restore.out" >&2
  need "exit 0 (got $fx_restore_rc)" test "$fx_restore_rc" -eq 0 || return 1
  need "exactly one line with both values and the consequence" test "$(grep -cxF "$gen_line" "$fx/restore.out" || true)" -eq 1 || return 1
  need "the target's install generation is 4" test "$(qx "$tgt_main" cortex "select gen from tokens.generations where scope = 'install'")" = 4 || return 1
  need "the source's is still 3" test "$(srcq "select gen from tokens.generations where scope = 'install'")" = 3 || return 1
  need "sha256sum -c on the set passes after the run" sh -c "cd '$fx_set' && sha256sum -c SHA256SUMS > /dev/null 2>&1" || return 1
  need "LAST_GOOD is unchanged (before $fx_lg_before)" test -n "$fx_lg_before" -a "$fx_lg_before" = "$fx_lg_after" || return 1
  ids=$(srcq "select id::text from tokens.tokens union all select encode(digest, 'hex') from tokens.tokens")
  need "the source has token ids to look for" test -n "$ids" || return 1
  for v in $ids; do
    need "no token id or digest in the output" test -z "$(grep -F "$v" "$fx/restore.out" "$fx/restore.err" || true)" || return 1
  done
  need "no role hash in the output" test -z "$(grep -h 'SCRAM-SHA-256' "$fx/restore.out" "$fx/restore.err" || true)" || return 1
}

bk_34d() { # nothing else changes: only the install row of tokens.generations differs; tokens are equal row for row
  new_test_db
  fx_or_fail || return 1
  need "the comparison is clean except the expected row (rc=$fx_cmp_rc)" test "$fx_cmp_rc" -eq 0 || return 1
  need "tokens.tokens: no missing, extra or different row" grep -Eq '^tokens\.tokens +PASS .*rows: missing 0, extra 0, different 0' "$fx/cmp.out" || return 1
  need "the project row of tokens.generations is equal (7 = 7)" test "$(srcq "select gen from tokens.generations where scope = 'project'")" = "$(qx "$tgt_main" cortex "select gen from tokens.generations where scope = 'project'")" || return 1
  q="select md5(string_agg(coalesce(revoked_at::text, '-') || coalesce(revoke_reason, '-'), ',' order by id)) from tokens.tokens"
  need "revoked_at and revoke_reason are equal for every token (the tool re-revoked nothing)" test "$(srcq "$q")" = "$(qx "$tgt_main" cortex "$q")" || return 1
  need "the source has a revoked token to make that count" test "$(srcq "select count(*) from tokens.tokens where revoked_at is not null")" -ge 1 || return 1
}

bk_27d() { # a pre-upgrade (labelled) set is restorable with cortex-restore, and the BK-14 to BK-16 and BK-30 checks pass on it
  new_test_db
  fx_or_fail || return 1
  run lab env $sut "$dump_bin" --target "$db_name" --out "$T/out" --label pre-upgrade-0.1.0-to-0.2.0
  need "labelled dump: exit 0 (got $rc)" test "$rc" -eq 0 || return 1
  lset=$(tail -n 1 "$T/lab.out")
  tn=$bkp-t-label
  tgt_up "$tn" || return 1
  cr r "$lset" "$tn"
  need "cortex-restore of the labelled set: exit 0 (got $rc): $(tail -n 3 "$T/r.err")" test "$rc" -eq 0 || { tgt_rm "$tn"; return 1; }
  need "the generation line" grep -qxF "$gen_line" "$T/r.out" || { tgt_rm "$tn"; return 1; }
  rcmp c --source "$db_name" --target "$tn" --after-restore
  need "data equal except the expected +1 (rc=$rc)" test "$rc" -eq 0 || { cat "$T/c.out" >&2; tgt_rm "$tn"; return 1; }
  rcmp i --source "$db_name" --target "$tn" --isolation
  need "isolation equal (rc=$rc)" test "$rc" -eq 0 || { cat "$T/i.out" >&2; tgt_rm "$tn"; return 1; }
  rd=$(rls_read "$tn" cortex)
  need "A reads its rows, not B's ('$rd')" test "${rd%% *}" -ge 1 -a "${rd#* }" -eq 0 || { tgt_rm "$tn"; return 1; }
  need "target generation 4" test "$(qx "$tn" cortex "select gen from tokens.generations where scope = 'install'")" = 4 || { tgt_rm "$tn"; return 1; }
  tgt_rm "$tn"
}

bk_28d() { # a target with the image's default (trust) rules is refused, and nothing is written to it
  new_test_db
  fx_or_fail || return 1
  tn=$bkp-t-trust
  tgt_up "$tn" plain || return 1
  need "the target really has a trust rule (the test would prove nothing otherwise)" test "$(qx "$tn" postgres "select count(*) from pg_hba_file_rules where auth_method = 'trust'")" -ge 1 || { tgt_rm "$tn"; return 1; }
  need "and the S5-flag target has none" test "$(qx "$tgt_main" postgres "select count(*) from pg_hba_file_rules where auth_method = 'trust' or error is not null")" -eq 0 || { tgt_rm "$tn"; return 1; }
  cr r "$fx_set" "$tn"
  need "refused: exit 2 (got $rc)" test "$rc" -eq 2 || { tgt_rm "$tn"; return 1; }
  need "one-line reason that says why" grep -q 'trust' "$T/r.err" || { tgt_rm "$tn"; return 1; }
  need "no database was created" test "$(qx "$tn" postgres "select count(*) from pg_database where datname = 'cortex'")" -eq 0 || { tgt_rm "$tn"; return 1; }
  need "no role was created" test "$(qx "$tn" postgres "select count(*) from pg_roles where rolname like 'cortex\\_%'")" -eq 0 || { tgt_rm "$tn"; return 1; }
  tgt_rm "$tn"
}

# the tests that change the SOURCE put it back, on every path: the wrapper runs the inner test and then the cleanup
src_clean_tokens() { srcq "delete from tokens.tokens where id in ('$T1_ID', '$T2_ID', '$T3_ID')" > /dev/null 2>&1 || true; }
src_clean_grant() { printf 'set role cortex_owner;\ngrant update on tokens.generations to cortex_install_admin;\n' | dbsql > /dev/null 2>&1 || true; }
src_clean_install_row() {
  if [ "$(srcq "select count(*) from tokens.generations where scope = 'install'" 2> /dev/null || echo 9)" = 0 ]; then
    srcq "insert into tokens.generations (scope, scope_id, gen) values ('install', '$INST_ID', 3)" > /dev/null 2>&1 || true
  fi
}

bk_31d() { _bk_31d; _r=$?; src_clean_tokens; return "$_r"; }
_bk_31d() { # the restore witness, database layer: after the tool's restore the old tokens are refused by the generation; the control shows the hole
  new_test_db
  fx_or_fail || return 1
  tn=$bkp-t-b31
  tgt_up "$tn" || return 1
  srcq "insert into tokens.tokens (id, worker_id, project_id, digest, install_gen, project_gen, issued_at, expires_at, renew_after) values ('$T1_ID', '$W2_ID', '$A_ID', sha256('t1'::bytea), 3, 7, now(), now() + interval '90 days', now() + interval '60 days'), ('$T2_ID', '$W2_ID', '$A_ID', sha256('t2'::bytea), 3, 7, now(), now() + interval '90 days', now() + interval '60 days')" > /dev/null ||
    { echo "    CHECK FAILED: creating T1 and T2 in the source" >&2; tgt_rm "$tn"; return 1; }
  run d env $sut "$dump_bin" --target "$db_name" --out "$T/out" --status-dir "$T/status"
  need "dump with T1 and T2: exit 0 (got $rc)" test "$rc" -eq 0 || { tgt_rm "$tn"; return 1; }
  s=$(tail -n 1 "$T/d.out")
  srcq "update tokens.tokens set revoked_at = now(), revoke_reason = 'revoked' where id = '$T1_ID'" > /dev/null || { tgt_rm "$tn"; return 1; }
  need "T1 is revoked in the source" test "$(srcq "select count(*) from tokens.tokens where id = '$T1_ID' and revoked_at is not null")" -eq 1 || { tgt_rm "$tn"; return 1; }
  cr r "$s" "$tn"
  need "cortex-restore: exit 0 (got $rc): $(tail -n 3 "$T/r.err")" test "$rc" -eq 0 || { tgt_rm "$tn"; return 1; }
  tool1=$(vl "$tn" cortex "$T1_ID")
  tool2=$(vl "$tn" cortex "$T2_ID")
  need "tool target: T1 has revoked_at NULL (the un-revoke is real) and install_gen 3 < current 4 (got '$tool1')" test "$tool1" = "3|4|true" || { tgt_rm "$tn"; return 1; }
  need "tool target: T2 has install_gen 3 < current 4 (got '$tool2')" test "$tool2" = "3|4|true" || { tgt_rm "$tn"; return 1; }
  podman exec --user postgres "$tn" psql -U postgres -d postgres -X -q -v ON_ERROR_STOP=1 -c "create database control template template0 owner cortex_owner" > /dev/null || { tgt_rm "$tn"; return 1; }
  podman exec -i --user postgres "$tn" pg_restore -U postgres -d control --no-password < "$s/platform.dump" > /dev/null 2> "$T/control.err" ||
    { echo "    CHECK FAILED: the control restore (plain pg_restore) failed: $(head -c 200 "$T/control.err")" >&2; tgt_rm "$tn"; return 1; }
  ctl1=$(vl "$tn" control "$T1_ID")
  ctl2=$(vl "$tn" control "$T2_ID")
  echo "    tool target:     T1 $tool1   T2 $tool2   (install_gen | current_install_gen | revoked_at is null)" >&2
  echo "    CONTROL (plain pg_restore, the forbidden way): T1 $ctl1   T2 $ctl2" >&2
  need "control: install_gen 3 equals current 3 and revoked_at is NULL, so the token would verify (got T1 '$ctl1', T2 '$ctl2')" test "$ctl1" = "3|3|true" -a "$ctl2" = "3|3|true" || { tgt_rm "$tn"; return 1; }
  tgt_rm "$tn"
}

bk_33d() { _bk_33d; _r=$?; src_clean_grant; return "$_r"; }
_bk_33d() { # (a) no skip: the env var had no effect in the drill; (b) a missing grant fails, the created database is removed; (c) control: the drill; (d) an existing name is refused
  new_test_db
  fx_or_fail || return 1
  need "(a) CORTEX_RESTORE_SKIP_GENERATION=1 was set in the drill and the generation was raised anyway" grep -qxF "$gen_line" "$fx/restore.out" || return 1
  need "(c) control: the same kind of set with the grant present restored and raised (rc=$fx_restore_rc)" test "$fx_restore_rc" -eq 0 || return 1
  # (b) a synthetic source with the grant revoked, so the dump carries it
  srcq "revoke update on tokens.generations from cortex_install_admin" > /dev/null || { echo "    CHECK FAILED: revoking the grant in the source" >&2; return 1; }
  run d env $sut "$dump_bin" --target "$db_name" --out "$T/out" --status-dir "$T/status"
  src_clean_grant
  need "dump of the source without the grant: exit 0 (got $rc)" test "$rc" -eq 0 || return 1
  s=$(tail -n 1 "$T/d.out")
  tn=$bkp-t-33b
  tgt_up "$tn" || return 1
  cr r "$s" "$tn"
  need "(b) exit 1 (got $rc)" test "$rc" -eq 1 || { tgt_rm "$tn"; return 1; }
  need "(b) the reason says the generation could not be raised and that the database was removed" sh -c "grep -q 'generation could not be raised' '$T/r.err' && grep -q 'removed' '$T/r.err'" || { cat "$T/r.err" >&2; tgt_rm "$tn"; return 1; }
  need "(b) the created database is gone (checked by name)" test "$(qx "$tn" postgres "select count(*) from pg_database where datname = 'cortex'")" -eq 0 || { tgt_rm "$tn"; return 1; }
  need "(b) no success line" test -z "$(grep -E 'restored|install generation [0-9n]' "$T/r.out" || true)" || { tgt_rm "$tn"; return 1; }
  need "(b) the set is unchanged" sh -c "cd '$s' && sha256sum -c SHA256SUMS > /dev/null 2>&1" || { tgt_rm "$tn"; return 1; }
  need "(b) the kept details hold no role-hash file (globals.filtered.sql is deleted on every path)" test -z "$(find "$T" -name 'globals.filtered.sql' 2> /dev/null)" || { tgt_rm "$tn"; return 1; }
  need "(b) and the details directory is kept for the operator (one cortex-restore.* directory)" test "$(ls -d "$T"/cortex-restore.* 2> /dev/null | wc -l | tr -d ' ')" -eq 1 || { tgt_rm "$tn"; return 1; }
  tgt_rm "$tn"
  # (d) a name that exists: the fixture target holds database cortex with its data
  before=$(qx "$tgt_main" cortex "select (select count(*) from identity.projects) || ' ' || (select count(*) from platform.objects) || ' ' || (select count(*) from records.records)")
  cr e "$fx_set" "$tgt_main"
  need "(d) exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  need "(d) the reason says the database already exists" grep -q 'already exists' "$T/e.err" || return 1
  after=$(qx "$tgt_main" cortex "select (select count(*) from identity.projects) || ' ' || (select count(*) from platform.objects) || ' ' || (select count(*) from records.records)")
  need "(d) the existing database is still there with its data ('$before' = '$after')" test -n "$before" -a "$before" = "$after" || return 1
  rcmp again --source "$db_name" --target "$tgt_main" --after-restore
  need "(d) and it still equals the source apart from the expected +1 (rc=$rc)" test "$rc" -eq 0 || return 1
}

bk_35d() { _bk_35d; _r=$?; src_clean_install_row; return "$_r"; }
_bk_35d() { # unusual sets: (a) no install row, (b) nothing bootstrapped, (c) tokens without an installation (impossible: the foreign keys)
  new_test_db
  fx_or_fail || return 1
  # (a)
  srcq "delete from tokens.generations where scope = 'install'" > /dev/null || { echo "    CHECK FAILED: deleting the install row in the source" >&2; return 1; }
  run a env $sut "$dump_bin" --target "$db_name" --out "$T/outa" --status-dir "$T/status"
  src_clean_install_row
  need "(a) dump of a source without an install row: exit 0 (got $rc)" test "$rc" -eq 0 || return 1
  sa=$(tail -n 1 "$T/a.out")
  tn=$bkp-t-35a
  tgt_up "$tn" || return 1
  cr ra "$sa" "$tn"
  need "(a) exit 0 (got $rc): $(tail -n 3 "$T/ra.err")" test "$rc" -eq 0 || { tgt_rm "$tn"; return 1; }
  need "(a) the line says none -> 1" grep -q '^install generation none -> 1: ' "$T/ra.out" || { tgt_rm "$tn"; return 1; }
  need "(a) the row was created with gen 1" test "$(qx "$tn" cortex "select gen from tokens.generations where scope = 'install'")" = 1 || { tgt_rm "$tn"; return 1; }
  va=$(vl "$tn" cortex "$TID_ID")
  need "(a) the stored token (install_gen 3) is not equal to the current 1, so it is refused (got '$va')" test "$va" = "3|1|false" -o "$va" = "3|1|true" || { tgt_rm "$tn"; return 1; }
  tgt_rm "$tn"
  # (b) a migrated but never bootstrapped install: a second source, the pack without the smoke data
  mk_pack_db $src2_name 0 || return 1
  run b env $sut "$dump_bin" --target $src2_name --out "$T/outb" --status-dir "$T/status"
  need "(b) dump of the empty install: exit 0 (got $rc)" test "$rc" -eq 0 || return 1
  sb=$(tail -n 1 "$T/b.out")
  tn=$bkp-t-35b
  tgt_up "$tn" || return 1
  cr rb "$sb" "$tn"
  need "(b) exit 0 (got $rc): $(tail -n 3 "$T/rb.err")" test "$rc" -eq 0 || { tgt_rm "$tn"; return 1; }
  need "(b) the line says there is nothing to protect" grep -q 'nothing to protect' "$T/rb.out" || { tgt_rm "$tn"; return 1; }
  need "(b) no generation row was created" test "$(qx "$tn" cortex "select count(*) from tokens.generations")" -eq 0 || { tgt_rm "$tn"; return 1; }
  rcmp cb --source "$src2_name" --target "$tn" --after-restore
  need "(b) control: the unbootstrapped install compares clean (rc=$rc) and says there is nothing to protect" sh -c "[ $rc -eq 0 ] && grep -q 'nothing to protect' '$T/cb.out'" || { cat "$T/cb.out" >&2; tgt_rm "$tn"; return 1; }
  tgt_rm "$tn"
  tgt_rm $src2_name
  # (c) cannot be built: the installation row cannot go while a project references it
  if srcq "delete from identity.installations" > /dev/null 2> "$T/c.err"; then
    echo "    CHECK FAILED: (c) the installation row could be deleted although projects and tokens reference it" >&2
    return 1
  fi
  need "(c) the foreign keys make it impossible (case dropped, as the definition allows): $(head -c 120 "$T/c.err")" grep -q 'foreign key' "$T/c.err" || return 1
}

bk_36d() { _bk_36d; _r=$?; qx "$tgt_main" cortex "delete from tokens.tokens where id = '$T3_ID'" > /dev/null 2>&1 || true; return "$_r"; }
_bk_36d() { # a chain: S (3 -> 4), dump of the restored database S2, restore S2 (-> 5); S twice gives 4 both times
  new_test_db
  fx_or_fail || return 1
  need "S into the first target gave 4" test "$(qx "$tgt_main" cortex "select gen from tokens.generations where scope = 'install'")" = 4 || return 1
  qx "$tgt_main" cortex "insert into tokens.tokens (id, worker_id, project_id, digest, install_gen, project_gen, issued_at, expires_at, renew_after) values ('$T3_ID', '$W2_ID', '$A_ID', sha256('t3'::bytea), 4, 7, now(), now() + interval '90 days', now() + interval '60 days')" > /dev/null ||
    { echo "    CHECK FAILED: creating T3 (issued after the first restore) in the restored database" >&2; return 1; }
  run d2 env $sut "$dump_bin" --target "$tgt_main" --out "$T/out2" --status-dir "$T/status"
  need "dump of the restored database: exit 0 (got $rc): $(tail -n 3 "$T/d2.err")" test "$rc" -eq 0 || return 1
  s2=$(tail -n 1 "$T/d2.out")
  tn2=$bkp-t-ch2
  tn3=$bkp-t-ch3
  tgt_up "$tn2" || return 1
  cr r2 "$s2" "$tn2"
  need "S2 into a fresh database: exit 0 (got $rc): $(tail -n 3 "$T/r2.err")" test "$rc" -eq 0 || { tgt_rm "$tn2"; return 1; }
  need "S2's restore says 4 -> 5" grep -q '^install generation 4 -> 5: ' "$T/r2.out" || { tgt_rm "$tn2"; return 1; }
  need "S2's target generation is 5" test "$(qx "$tn2" cortex "select gen from tokens.generations where scope = 'install'")" = 5 || { tgt_rm "$tn2"; return 1; }
  v3=$(vl "$tn2" cortex "$T3_ID")
  need "T3 (stored 4, present in S2) is refused after S2's restore: 4 < 5 (got '$v3')" test "$v3" = "4|5|true" || { tgt_rm "$tn2"; return 1; }
  tgt_rm "$tn2"
  tgt_up "$tn3" || return 1
  cr r3 "$fx_set" "$tn3"
  need "S a second time into another fresh database: exit 0 (got $rc)" test "$rc" -eq 0 || { tgt_rm "$tn3"; return 1; }
  need "S twice gives 4 both times" test "$(qx "$tn3" cortex "select gen from tokens.generations where scope = 'install'")" = 4 || { tgt_rm "$tn3"; return 1; }
  tgt_rm "$tn3"
}

bk_40d() { _bk_40d; _r=$?; srcq "drop table if exists public.bk_big" > /dev/null 2>&1 || true; return "$_r"; }
_bk_40d() { # BK12-03 on the real thing: pg_dump's stream cut to three quarters (the table of contents is whole, the data is not) is never published
  new_test_db
  srcq "create table public.bk_big as select i, md5(i::text) || repeat('synthetic', 20) as body from generate_series(1, 30000) i" > /dev/null || return 1
  cat > "$T/bin/podman" <<'SHIM'
#!/bin/sh
# the real podman, except that the stream of pg_dump is cut to three quarters
case " $* " in
  *" pg_dump "*)
    "$REAL_PODMAN" "$@" > "$TRUNC_DIR/full.dump" || exit $?
    n=$(wc -c < "$TRUNC_DIR/full.dump")
    head -c $((n * 3 / 4)) "$TRUNC_DIR/full.dump"
    exit 0 ;;
esac
exec "$REAL_PODMAN" "$@"
SHIM
  chmod 0755 "$T/bin/podman"
  real=$(command -v podman)
  run t env PATH="$T/bin:$PATH" REAL_PODMAN="$real" TRUNC_DIR="$T" $sut "$dump_bin" --target "$db_name" --out "$T/out" --status-dir "$T/status"
  need "a cut archive: exit 1 (got $rc): $(tail -n 2 "$T/t.err")" test "$rc" -eq 1 || return 1
  need "no set was published" test "$(count_good_sets)" -eq 0 || return 1
  need "no temp directory" test -z "$(temp_left)" || return 1
  need "no LAST_GOOD" test ! -e "$T/status/LAST_GOOD" || return 1
  cut_size=$(( $(wc -c < "$T/full.dump") * 3 / 4 ))
  head -c "$cut_size" "$T/full.dump" > "$T/cut.dump"
  need "the cut archive really passes pg_restore --list (so only a full read can refuse it)" sh -c "podman exec -i --user postgres '$db_name' pg_restore --list < '$T/cut.dump' > /dev/null 2>&1" || return 1
  need "and really fails a full read" sh -c "! podman exec -i --user postgres '$db_name' pg_restore --file=/dev/null < '$T/cut.dump' > /dev/null 2>&1" || return 1
  real_dump ok
  need "control: the same database dumped whole is published (rc=$rc)" test "$rc" -eq 0 || return 1
}

# ---- run --------------------------------------------------------------------------------------------------

all_ids="BK-13 BK-28 BK-33A BK-37R BK-18R BK-51 BK-52 BK-53 BK-56 BK-57 BK-58 BK-59 BK-48 BK-49 BK-54 BK-55 BK-50 BK-38 BK-39 BK-40 BK-42 BK-43 BK-44 BK-45 BK-46 BK-47 BK-04 BK-05 BK-19 BK-02 BK-03 BK-06 BK-07 BK-09 BK-10 BK-11 BK-12 BK-18 BK-20 BK-21 BK-22 BK-23 BK-24 BK-25 BK-26 BK-29 BK-37 PORT-01 PORT-02"
db_ids="BK-01D BK-20D BK-24D BK-40D BK-14D BK-15D BK-16D BK-17D BK-30D BK-34D BK-14E BK-27D BK-28D BK-31D BK-33D BK-35D BK-36D"
if [ -n "$selftest" ]; then
  # the registry lifecycle only (BK-54, BK-55): reg_add, rm_checked and selftest_down against a recording podman that THIS MODE defines itself.
  # The synthetic IDs below name no container; podman takes a name as well as an ID, so they must never reach a real podman. A shell
  # function is found before any program in PATH (and PATH is the caller's), so no `podman` call of this mode can leave the shell. The
  # recorded calls go to stdout (fd 3 keeps them past the `> /dev/null` of the callers). The mode takes no other mode: it is refused
  # (exit 2) before anything is registered.
  [ "$selftest" = custody ] || { echo "cortex-dump-test: unknown --selftest $selftest" >&2; exit 2; }
  if [ "$db" -ne 0 ] || [ "$sd" -ne 0 ] || [ -n "$want" ]; then
    echo "cortex-dump-test: --selftest takes no other mode (--db, --systemd or a test id)" >&2
    exit 2
  fi
  exec 3>&1
  podman() {
    echo "podman $*" >&3
    case "${1:-} ${2:-}" in
      "container exists") return 1 ;; # nothing exists: every removal is confirmed
    esac
    return 0
  }
  trap selftest_down EXIT # registry-only: the real db_down would delete a fixture this mode never created
  reg_add t1 id-one
  rm_checked t1 # the per-test removal of the first container under that name
  reg_add t1 id-two # the SAME name again: a new container
  reg_add t2 id-three
  rm_checked t2 # a different name in between
  exit 0 # selftest_down must now remove id-two, and only that
fi
if [ "$sd" -eq 1 ]; then
  [ -n "$want" ] || want=BK-04S
elif [ "$db" -eq 1 ]; then
  [ -n "$want" ] || want=$db_ids
  trap db_down EXIT
  trap 'exit 130' INT
  trap 'exit 143' TERM
  db_up
else
  [ -n "$want" ] || want=$all_ids
fi
pass=0
fail=0
failed=""
for id in $want; do
  fn=$(printf '%s' "$id" | tr 'A-Z-' 'a-z_')
  if ! command -v "$fn" > /dev/null 2>&1; then
    echo "$id MISSING (no such test in this group)"
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
  end_test
done
echo "SUMMARY under '$sut': $pass passed, $fail failed${failed:+ ($failed )}"
[ "$fail" -eq 0 ]
