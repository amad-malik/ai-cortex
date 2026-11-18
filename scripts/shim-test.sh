#!/bin/sh
# Test of scripts/cv2-admit-shim (finn's finding, via cox row 2350a624): on build-host-1 a missing or non-executable cv2-admit
# must REFUSE, not run unadmitted; anywhere else a fallback is allowed but never silent; CV2_ADMIT must not be able to name
# any program unless the test hook CV2_SHIM_TEST=1 is also set. No container, no cargo.
# Run it through cv2-admit like any test:  cv2-admit sh scripts/shim-test.sh
set -eu

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
shim="$here/cv2-admit-shim"
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
pass=0 fail=0
ok()  { pass=$((pass + 1)); echo "ok   $1"; }
bad() { fail=$((fail + 1)); echo "FAIL $1"; [ -z "${2:-}" ] || echo "       $2"; }

# a fake admission program: records that it ran and its arguments, then runs the command
cat > "$T/admit" <<'FAKE'
#!/bin/sh
echo "admit-ran" >> "$T_LOG"
for a in "$@"; do printf '[%s]' "$a" >> "$T_LOG"; done; echo >> "$T_LOG"
exec "$@"
FAKE
chmod +x "$T/admit"
# a command that leaves a marker, so a refusal can be proved by the marker's absence
cat > "$T/cmd" <<'CMD'
#!/bin/sh
echo "cmd-ran" >> "$T_MARK"
exit "${CMD_EXIT:-0}"
CMD
chmod +x "$T/cmd"
T_LOG="$T/log"; T_MARK="$T/mark"; export T_LOG T_MARK

fresh() { : > "$T_LOG"; : > "$T_MARK"; }
ran()   { [ -s "$T_MARK" ]; }
viaadmit() { grep -q admit-ran "$T_LOG"; }
n=0
runshim() { n=$((n + 1)); mkdir -p "$T/tmp$n"; TMPDIR="$T/tmp$n" sh "$shim" "$@" > "$T/out" 2> "$T/err"; }

# ST-01 admission present: the command runs through it, arguments intact
fresh
if CV2_SHIM_TEST=1 CV2_ADMIT="$T/admit" CV2_SHIM_HOSTNAME=build-host-1 runshim "$T/cmd" "a b" 'c"d' && ran && viaadmit && grep -q '\[a b\]\[c"d\]' "$T_LOG"; then ok "ST-01 with admission present the command runs through it, arguments intact"; else bad "ST-01 with admission present the command runs through it, arguments intact" "$(cat "$T_LOG")"; fi

# ST-02 admission missing on another host: runs, but SAYS it is unadmitted
fresh
if CV2_SHIM_TEST=1 CV2_ADMIT="$T/none" CV2_SHIM_HOSTNAME=laptop runshim "$T/cmd" && ran && ! viaadmit && grep -q UNADMITTED "$T/err"; then ok "ST-02 on another host a missing admission program runs the command and says UNADMITTED on stderr"; else bad "ST-02 on another host a missing admission program runs the command and says UNADMITTED on stderr" "err=$(cat "$T/err")"; fi

# ST-03 admission missing on build-host-1: refuse (exit 75), the command does not run
fresh
if CV2_SHIM_TEST=1 CV2_ADMIT="$T/none" CV2_SHIM_HOSTNAME=build-host-1 runshim "$T/cmd"; then bad "ST-03 on build-host-1 a missing admission program refuses and does not run the command" "exit 0"; else
  rc=$?
  if [ "$rc" = 75 ] && ! ran && grep -q REFUSED "$T/err"; then ok "ST-03 on build-host-1 a missing admission program refuses and does not run the command"; else bad "ST-03 on build-host-1 a missing admission program refuses and does not run the command" "rc=$rc ran=$(ran && echo yes || echo no) err=$(cat "$T/err")"; fi
fi

# ST-04 admission present but NOT executable on build-host-1: refuse
fresh; cp "$T/admit" "$T/admit-noexec"; chmod -x "$T/admit-noexec"
if CV2_SHIM_TEST=1 CV2_ADMIT="$T/admit-noexec" CV2_SHIM_HOSTNAME=build-host-1 runshim "$T/cmd"; then bad "ST-04 on build-host-1 a non-executable admission program refuses" "exit 0"; else
  rc=$?
  if [ "$rc" = 75 ] && ! ran && grep -q REFUSED "$T/err"; then ok "ST-04 on build-host-1 a non-executable admission program refuses"; else bad "ST-04 on build-host-1 a non-executable admission program refuses" "rc=$rc ran=$(ran && echo yes || echo no)"; fi
fi

# ST-05 non-executable admission elsewhere: runs, says UNADMITTED
fresh
if CV2_SHIM_TEST=1 CV2_ADMIT="$T/admit-noexec" CV2_SHIM_HOSTNAME=laptop runshim "$T/cmd" && ran && grep -q UNADMITTED "$T/err"; then ok "ST-05 elsewhere a non-executable admission program runs the command and says UNADMITTED"; else bad "ST-05 elsewhere a non-executable admission program runs the command and says UNADMITTED" "err=$(cat "$T/err")"; fi

# ST-06 CV2_ADMIT cannot name a program without the test hook
fresh
if CV2_ADMIT="$T/admit" sh "$shim" "$T/cmd" > "$T/out" 2> "$T/err"; then :; fi
if ! viaadmit; then ok "ST-06 CV2_ADMIT is ignored unless CV2_SHIM_TEST=1 is also set"; else bad "ST-06 CV2_ADMIT is ignored unless CV2_SHIM_TEST=1 is also set" "the named program ran"; fi

# ST-07 the command's exit status passes through, with and without admission
fresh
rc1=0; CV2_SHIM_TEST=1 CV2_ADMIT="$T/admit" CV2_SHIM_HOSTNAME=build-host-1 CMD_EXIT=3 runshim "$T/cmd" || rc1=$?
fresh
rc2=0; CV2_SHIM_TEST=1 CV2_ADMIT="$T/none" CV2_SHIM_HOSTNAME=laptop CMD_EXIT=4 runshim "$T/cmd" || rc2=$?
if [ "$rc1" = 3 ] && [ "$rc2" = 4 ]; then ok "ST-07 the command's exit status passes through (3 and 4)"; else bad "ST-07 the command's exit status passes through (3 and 4)" "rc1=$rc1 rc2=$rc2"; fi

# ST-08 (nix N16) the UNADMITTED line is said once per build (per parent process), not on every rustc call
mkdir -p "$T/once"; fresh
env_="CV2_SHIM_TEST=1 CV2_ADMIT=$T/none CV2_SHIM_HOSTNAME=laptop TMPDIR=$T/once"
env CV2_SHIM_TEST=1 CV2_ADMIT="$T/none" CV2_SHIM_HOSTNAME=laptop TMPDIR="$T/once" sh "$shim" "$T/cmd" 2> "$T/e1"
env CV2_SHIM_TEST=1 CV2_ADMIT="$T/none" CV2_SHIM_HOSTNAME=laptop TMPDIR="$T/once" sh "$shim" "$T/cmd" 2> "$T/e2"
env CV2_SHIM_TEST=1 CV2_ADMIT="$T/none" CV2_SHIM_HOSTNAME=laptop TMPDIR="$T/once" sh -c "sh '$shim' '$T/cmd'; true" 2> "$T/e3"
if grep -q UNADMITTED "$T/e1" && ! grep -q UNADMITTED "$T/e2" && grep -q UNADMITTED "$T/e3" && [ "$(wc -l < "$T/mark")" = 3 ]; then ok "ST-08 UNADMITTED is said once per build and again for the next build; the command always runs"; else bad "ST-08 UNADMITTED is said once per build and again for the next build; the command always runs" "e1=$(wc -l < "$T/e1") e2=$(wc -l < "$T/e2") e3=$(wc -l < "$T/e3") ran=$(wc -l < "$T/mark")"; fi

# ST-09 (nix N21) markers of builds that have ended are pruned; the marker of a live build is kept
mkdir -p "$T/prune"; fresh
uid=$(id -u)
: > "$T/prune/cv2-admit-shim.$uid.2147483646"            # no such process
sleep 30 & livepid=$!
: > "$T/prune/cv2-admit-shim.$uid.$livepid"               # a live process
: > "$T/prune/other-file"                                  # not ours: must be left alone
CV2_SHIM_TEST=1 CV2_ADMIT="$T/none" CV2_SHIM_HOSTNAME=laptop TMPDIR="$T/prune" sh "$shim" "$T/cmd" 2> "$T/e9"
if [ ! -e "$T/prune/cv2-admit-shim.$uid.2147483646" ] && [ -e "$T/prune/cv2-admit-shim.$uid.$livepid" ] && [ -e "$T/prune/other-file" ]; then ok "ST-09 a marker of a finished build is pruned; a live build's marker and unrelated files are kept"; else bad "ST-09 a marker of a finished build is pruned; a live build's marker and unrelated files are kept" "$(ls "$T/prune" | tr '\n' ' ')"; fi
kill "$livepid" 2>/dev/null || true

echo "shim-test: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
