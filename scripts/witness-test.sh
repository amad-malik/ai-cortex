#!/bin/sh
# Logic test for scripts/install-witness.sh (G-S5-2, S5 amendment 14 / A74) with a FAKE podman: no container, no
# network, no real secret. The witness lists the NAMES under /run/secrets of a running container and compares them,
# as a set, with what that container may hold. It never reads a secret file.
#
#   cv2-admit sh scripts/witness-test.sh                       the suite against scripts/install-witness.sh
#   WITNESS_SH=/path/to/copy cv2-admit sh scripts/witness-test.sh   the suite against another copy (the mutation run uses this)
#
# Every test that expects a refusal also expects the refusal's own words in the output, so a witness that is missing
# or broken cannot pass them by failing. IW-01 and IW-07 are the positive controls.
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
WITNESS_SH=${WITNESS_SH:-"$here/install-witness.sh"}
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
mkdir -p "$T/bin" "$T/state" "$T/cwd"
: > "$T/log"

# The fake podman: records every call; answers only `exec [flags] CONTAINER ls -1A /run/secrets` (from a knob file; the
# plain `ls -1` form hides dot-entries like the real ls does) and,
# so that a leak is visible, `exec CONTAINER cat /run/secrets/NAME`. Anything else is an error.
cat > "$T/bin/podman" <<'FAKE'
#!/bin/sh
echo "$*" >> "$FAKE_LOG"
[ "${1:-}" = exec ] || { echo "fake podman: only exec is modelled, got: $*" >&2; exit 99; }
shift
while [ "${1#-}" != "$1" ]; do if [ "$1" = --user ]; then shift 2; else shift; fi; done
ctr=$1; shift
if [ -f "$FAKE_STATE/down.$ctr" ]; then
  echo "Error: fake: container $ctr is not available" >&2
  exit "$(cat "$FAKE_STATE/down.$ctr")"
fi
case "$*" in
  "ls -1 /run/secrets") grep -v '^\.' "$FAKE_STATE/list.$ctr" || true ;;   # real ls: dot-entries are hidden without -a or -A
  "ls -1A /run/secrets") cat "$FAKE_STATE/list.$ctr" ;;                      # -A shows them, but not . and ..
  "cat /run/secrets/"*) cat "$FAKE_STATE/secret.${*##*/}" ;;
  *) echo "fake podman: exec not modelled: $*" >&2; exit 98 ;;
esac
FAKE
chmod +x "$T/bin/podman"

PATH="$T/bin:$PATH"; export PATH
FAKE_LOG="$T/log"; FAKE_STATE="$T/state"; export FAKE_LOG FAKE_STATE

APP=cv2-s5-app-password OWN=cv2-s5-owner-password SUP=cv2-s5-db-superuser
SRV=cv2-s5-server DB=cv2-s5-db
# files in the working directory whose names are the expected secret names: an unquoted `*` in the listing would
# expand to exactly these (the glob trap of IW-08)
: > "$T/cwd/$APP"; : > "$T/cwd/$OWN"; : > "$T/cwd/$SUP"
for sname in $APP $OWN $SUP; do printf 'SYNTHETIC-SECRET-1\n' > "$T/state/secret.$sname"; done

pass=0 fail=0
ok()  { pass=$((pass + 1)); echo "ok   $1"; }
bad() { fail=$((fail + 1)); echo "FAIL $1"; [ -z "${2:-}" ] || echo "       $2"; }

# w CMD...: run a witness function in a subshell, from the glob-trap directory; sets rc and leaves the output in $T/out
w() { ( cd "$T/cwd" && . "$WITNESS_SH" && "$@" ) > "$T/out" 2>&1 && rc=0 || rc=$?; }
list() { lctr=$1; shift; : > "$T/state/list.$lctr"; for litem in "$@"; do printf '%s\n' "$litem" >> "$T/state/list.$lctr"; done; }
has() { grep -qF -- "$1" "$T/out"; }
reset() { rm -f "$T/state"/list.* "$T/state"/down.*; : > "$T/log"; }
esc=$(printf '\033')

# IW-01 the server lists exactly its one secret: pass (positive control)
reset; list $SRV $APP
w witness_container $SRV
if [ "$rc" = 0 ] && has "$SRV" && has "$APP" && ! has FAIL; then ok "IW-01 the server holds exactly the app secret: pass"; else bad "IW-01 the server holds exactly the app secret: pass" "rc=$rc: $(head -3 "$T/out")"; fi

# IW-02 the owner secret is also there: fail, named as unexpected
reset; list $SRV $APP $OWN
w witness_container $SRV
if [ "$rc" != 0 ] && has "$OWN" && has unexpected && ! has missing; then ok "IW-02 an extra owner secret fails and is named"; else bad "IW-02 an extra owner secret fails and is named" "rc=$rc: $(head -3 "$T/out")"; fi

# IW-03 the superuser secret is also there
reset; list $SRV $APP $SUP
w witness_container $SRV
if [ "$rc" != 0 ] && has "$SUP" && has unexpected; then ok "IW-03 an extra superuser secret fails and is named"; else bad "IW-03 an extra superuser secret fails and is named" "rc=$rc: $(head -3 "$T/out")"; fi

# IW-04 nothing listed: never a pass
reset; list $SRV
w witness_container $SRV
if [ "$rc" != 0 ] && has "the listing of /run/secrets is empty" && ! has unexpected; then ok "IW-04 an empty listing fails, with its own message (not as a bad name)"; else bad "IW-04 an empty listing fails, with its own message (not as a bad name)" "rc=$rc: $(head -3 "$T/out")"; fi

# IW-05 only the owner secret: the count is still one, so a count is not enough
reset; list $SRV $OWN
w witness_container $SRV
if [ "$rc" != 0 ] && has "$OWN" && has "$APP" && has unexpected && has missing; then ok "IW-05 the wrong single secret fails: unexpected and missing, both named"; else bad "IW-05 the wrong single secret fails: unexpected and missing, both named" "rc=$rc: $(head -3 "$T/out")"; fi

# IW-06 the exec fails: container not running (126), no such container (125); never a pass
for code in 126 125; do
  reset; list $SRV $APP; echo "$code" > "$T/state/down.$SRV"
  w witness_container $SRV
  if [ "$rc" != 0 ] && has "could not list" && has "(exit $code)" && ! has "holds exactly"; then ok "IW-06 a failing exec ($code) fails with 'could not list' and its exit status"; else bad "IW-06 a failing exec ($code) fails with 'could not list' and its exit status" "rc=$rc: $(head -3 "$T/out")"; fi
done

# IW-07 the database holds the three; then only two
reset; list $DB $APP $SUP $OWN
w witness_container $DB
if [ "$rc" = 0 ] && has "$DB" && has "$SUP" && ! has FAIL; then ok "IW-07 the database holds exactly its three: pass"; else bad "IW-07 the database holds exactly its three: pass" "rc=$rc: $(head -3 "$T/out")"; fi
reset; list $DB $APP $SUP
w witness_container $DB
if [ "$rc" != 0 ] && has "$OWN" && has missing && ! has unexpected; then ok "IW-07 the database with two of its three fails naming the missing one"; else bad "IW-07 the database with two of its three fails naming the missing one" "rc=$rc: $(head -3 "$T/out")"; fi

# IW-08 hostile listing lines: fail; nothing raw in the output; the names are never glob-expanded or word-split
long=$(printf 'cv2-s5-'; i=0; while [ $i -lt 193 ]; do printf a; i=$((i + 1)); done)
n=0
for hostile in "cv2-s5-bad name" "*" "cv2-s5-*" "${esc}[31mcv2-s5-evil" "$long" "" "-n" "cv2-s5-app-password " 'cv2-s5-$(id)' "cv2-s5-x;y"; do
  n=$((n + 1)); reset; list $SRV $APP "$hostile"
  w witness_container $SRV
  clean=1
  grep -q "$esc" "$T/out" && clean=0                      # no escape byte reaches the output
  grep -q 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' "$T/out" && clean=0   # no 50 'a's in a row: the long name is cut
  [ "$(awk '{ if (length($0) > m) m = length($0) } END { print m + 0 }' "$T/out")" -le 400 ] || clean=0
  if [ "$rc" != 0 ] && [ "$clean" = 1 ] && has unexpected && ! has "holds exactly"; then ok "IW-08.$n a hostile listing line fails and is not echoed raw"; else bad "IW-08.$n a hostile listing line fails and is not echoed raw" "rc=$rc clean=$clean: $(head -3 "$T/out" | cut -c1-120)"; fi
done
# the glob trap alone: the listing is just `*`; the app secret is "missing", and it must not have been found by expansion
reset; list $SRV "*"
w witness_container $SRV
if [ "$rc" != 0 ] && has missing && has "$APP"; then ok "IW-08.g a lone * is not expanded into the files of the working directory"; else bad "IW-08.g a lone * is not expanded into the files of the working directory" "rc=$rc: $(head -3 "$T/out" | cut -c1-120)"; fi

# IW-09 the only exec is `ls -1 /run/secrets`; the synthetic secret value reaches no output and no log
reset; list $SRV $APP; list $DB $APP $SUP $OWN
w witness_container $SRV; r1=$rc; cp "$T/out" "$T/out1"
w witness_container $DB;  r2=$rc; cat "$T/out1" "$T/out" > "$T/out.all"
execs=$(grep -c '^exec ' "$T/log" || true)
lsonly=$(grep -c '^exec cv2-s5-[a-z]* ls -1A /run/secrets$' "$T/log" || true)
if [ "$r1" = 0 ] && [ "$r2" = 0 ] && [ "$execs" = 2 ] && [ "$lsonly" = 2 ] && ! grep -q 'SYNTHETIC-SECRET' "$T/out.all" "$T/log"; then ok "IW-09 one exec per container, only 'ls -1A /run/secrets'; no secret value anywhere"; else bad "IW-09 one exec per container, only 'ls -1A /run/secrets'; no secret value anywhere" "r1=$r1 r2=$r2 execs=$execs lsonly=$lsonly"; fi

# IW-10 the wrong container with the other container's list: the check follows the name it is given
reset; list $DB $APP $SUP $OWN
w witness_secrets $DB $APP
if [ "$rc" != 0 ] && has "$DB" && has "$SUP" && has "$OWN" && has unexpected; then ok "IW-10 the database checked against the server's list fails"; else bad "IW-10 the database checked against the server's list fails" "rc=$rc: $(head -3 "$T/out")"; fi
reset; list cv2-s5-other $APP
w witness_container cv2-s5-other
if [ "$rc" != 0 ] && has "no expected" && [ ! -s "$T/log" ]; then ok "IW-10 a container with no expected set fails without calling podman"; else bad "IW-10 a container with no expected set fails without calling podman" "rc=$rc log=$(wc -c < "$T/log"): $(head -3 "$T/out")"; fi

# IW-11 (hue's addition) bad arguments fail closed and never reach podman
for args in "witness_secrets" "witness_secrets $SRV" "witness_secrets $SRV ../x" "witness_secrets $SRV cv2-s5-" "witness_secrets cv2-s5-ser;ver $APP" "witness_secrets -x $APP" "witness_secrets $SRV $APP bad name"; do
  reset; list $SRV $APP
  # shellcheck disable=SC2086
  w $args
  if [ "$rc" != 0 ] && has usage && [ ! -s "$T/log" ]; then ok "IW-11 bad arguments fail with usage and no podman call: $args"; else bad "IW-11 bad arguments fail with usage and no podman call: $args" "rc=$rc log=$(wc -c < "$T/log"): $(head -2 "$T/out")"; fi
done

# IW-12 (hue's addition, from the first mutation run) a listed name that is only PART of an expected name is not that name
reset; list $SRV $APP cv2-s5-app
w witness_container $SRV
if [ "$rc" != 0 ] && has unexpected && has "cv2-s5-app;" ; then ok "IW-12 a listed name that is a prefix of an expected one is unexpected"; else bad "IW-12 a listed name that is a prefix of an expected one is unexpected" "rc=$rc: $(head -3 "$T/out")"; fi
reset; list $SRV cv2-s5-app-password-2
w witness_container $SRV
if [ "$rc" != 0 ] && has unexpected && has missing; then ok "IW-12 a listed name that contains an expected one is unexpected, and the expected one is missing"; else bad "IW-12 a listed name that contains an expected one is unexpected, and the expected one is missing" "rc=$rc: $(head -3 "$T/out")"; fi

# IW-13 (hale F-1) a HIDDEN extra: a dot-entry is what plain `ls -1` never shows
reset; list $SRV $APP .cv2-s5-owner-password
w witness_container $SRV
if [ "$rc" != 0 ] && has ".cv2-s5-owner-password" && has unexpected && ! has "holds exactly"; then ok "IW-13 a hidden dot-entry is seen and fails as unexpected"; else bad "IW-13 a hidden dot-entry is seen and fails as unexpected" "rc=$rc: $(head -3 "$T/out")"; fi
reset; list $SRV $APP
w witness_container $SRV
if [ "$rc" = 0 ] && has "holds exactly"; then ok "IW-13 control: without the hidden entry the same call passes"; else bad "IW-13 control: without the hidden entry the same call passes" "rc=$rc: $(head -3 "$T/out")"; fi

# IW-14 (hale F-2) a repeated listing line is never a legitimate listing (a name with a newline in it adds lines)
reset; list $SRV $APP $APP
w witness_container $SRV
if [ "$rc" != 0 ] && has repeated && ! has "holds exactly"; then ok "IW-14 a repeated listing line fails with its own words"; else bad "IW-14 a repeated listing line fails with its own words" "rc=$rc: $(head -3 "$T/out")"; fi

# IW-15 (hale F-3) witness_container leaves the caller's IFS as it found it, and does not abort under set -u
reset; list $SRV $APP
unset_out=$( cd "$T/cwd" && . "$WITNESS_SH" && unset IFS && set -u && { witness_container $SRV > /dev/null 2>&1 || echo "FAILED"; case ${IFS+set} in set) echo "IFS-NOW-SET" ;; *) echo "IFS-STILL-UNSET" ;; esac; } 2>&1 ) || true
custom_out=$( cd "$T/cwd" && . "$WITNESS_SH" && IFS=: && witness_container $SRV > /dev/null 2>&1; printf '%s' "$IFS" ) || true
if [ "$unset_out" = "IFS-STILL-UNSET" ] && [ "$custom_out" = ":" ]; then ok "IW-15 the caller's IFS is left as it was (unset, and a custom one)"; else bad "IW-15 the caller's IFS is left as it was (unset, and a custom one)" "unset case: [$unset_out]; custom case: [$custom_out]"; fi

echo "witness-test: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
