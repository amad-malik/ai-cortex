#!/bin/sh
# scripts/verify-units-test.sh: the test of scripts/verify-units.sh, small and standalone so that its mutants (verify-units-mutants.json) run in seconds.
# kai 13:30Z, 7 Nov: `systemd-analyze --user verify` with the live runtime directory unlinks and re-binds the live user manager's control socket. The failure is SIMULATED here,
# never reproduced on the host: a fake runtime directory with a fake systemd/private, and a stand-in systemd-analyze that unlinks and re-creates it (a new inode).
# Prints "VERIFY-UNITS WRONG: <cell>" for every cell that fails and exits 1 (the marker the mutant runner looks for).
set -u
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
vu="$here/verify-units.sh"
T=$(mktemp -d "${TMPDIR:-/tmp}/verify-units-test.XXXXXX") || exit 2
trap 'rm -rf -- "$T"' EXIT
fail=0; pass=0
ok() { pass=$((pass + 1)); echo "ok   $1"; }
bad() { fail=$((fail + 1)); echo "VERIFY-UNITS WRONG: $1${2:+ ($2)}"; }
stub="$T/stub"; mkdir -p "$stub" "$T/rt/systemd" "$T/units" "$T/tmp"
TMPDIR="$T/tmp"; export TMPDIR   # the helper makes its private directory under TMPDIR: keep it in the test's own scratch (a mutant that never cleans up must not leave it in /tmp)
cat > "$stub/systemd-analyze" <<'STUB'
#!/bin/sh
printf '%s\n' "XDG_RUNTIME_DIR=${XDG_RUNTIME_DIR:-unset}" "DBUS=${DBUS_SESSION_BUS_ADDRESS-unset}" "ARGS=$*" "MODE=$(stat -c %a "${XDG_RUNTIME_DIR:-/nonexistent}" 2>/dev/null)" >> "$STUB_LOG"
if [ -n "${XDG_RUNTIME_DIR:-}" ]; then mkdir -p "$XDG_RUNTIME_DIR/systemd"; rm -f "$XDG_RUNTIME_DIR/systemd/private"; : > "$XDG_RUNTIME_DIR/systemd/private"; fi
exit "${STUB_RC:-0}"
STUB
chmod +x "$stub/systemd-analyze"; : > "$T/units/a.service"; : > "$T/units/b.timer"
sock="$T/rt/systemd/private"; : > "$sock"

# VU-1 the control: the stand-in rebinds the socket when it is called straight with the live runtime directory.
# The OLD inode is kept allocated until the stand-in has run: a hard link made OUTSIDE the directory this cell inspects (hue, 9 Nov, cox c3e4da3d). Without it nothing holds the freed number,
# and a filesystem that hands a freed inode number to the next create (XFS here) made "the number differs" fail by itself, now and then, with a stand-in that DID rebind.
keep="$T/keep-old-inode"; ln "$sock" "$keep"
i=$(stat -c %i "$sock"); : > "$T/c.log"
(cd "$T/units" && STUB_LOG="$T/c.log" XDG_RUNTIME_DIR="$T/rt" PATH="$stub:$PATH" systemd-analyze --user --man=no verify a.service) >/dev/null 2>&1
if [ "$i" != "$(stat -c %i "$sock")" ] && [ "$i" = "$(stat -c %i "$keep")" ]; then ok "VU-1 control: the stand-in rebinds the live socket"; else bad "VU-1 the control cannot fail"; fi
rm -f -- "$keep"

# VU-2 the helper: same inode, a private 0700 runtime directory (not the live one), gone afterwards, no bus, the exact arguments
i=$(stat -c %i "$sock"); : > "$T/h.log"
(cd "$T/units" && STUB_LOG="$T/h.log" XDG_RUNTIME_DIR="$T/rt" PATH="$stub:$PATH" sh "$vu" a.service b.timer) >/dev/null 2>&1; rc=$?
priv=$(sed -n 's/^XDG_RUNTIME_DIR=//p' "$T/h.log" | sed -n 1p)
[ "$rc" = 0 ] && ok "VU-2a the helper exits 0 when the tool does" || bad "VU-2a the helper exits 0 when the tool does" "rc=$rc"
[ "$i" = "$(stat -c %i "$sock" 2>/dev/null)" ] && ok "VU-2b the live socket keeps its inode" || bad "VU-2b the live socket keeps its inode"
{ [ -n "$priv" ] && [ "$priv" != unset ] && [ "$priv" != "$T/rt" ]; } && ok "VU-2c the tool got a runtime directory that is not the live one" || bad "VU-2c a private runtime directory" "got '$priv'"
grep -qx 'MODE=700' "$T/h.log" && ok "VU-2d the private directory is 0700" || bad "VU-2d the private directory is 0700"
[ ! -e "$priv" ] && ok "VU-2e the private directory is gone afterwards" || bad "VU-2e the private directory is gone afterwards"
grep -qx 'DBUS=' "$T/h.log" && ok "VU-2f no session bus" || bad "VU-2f no session bus"
grep -qx 'ARGS=--user --man=no verify a.service b.timer' "$T/h.log" && ok "VU-2g the exact arguments" || bad "VU-2g the exact arguments" "$(grep '^ARGS=' "$T/h.log")"

# VU-3 the tool's status passes through and the private directory is removed even then
: > "$T/s.log"; (cd "$T/units" && STUB_LOG="$T/s.log" STUB_RC=3 XDG_RUNTIME_DIR="$T/rt" PATH="$stub:$PATH" sh "$vu" a.service) >/dev/null 2>&1; rc=$?
priv=$(sed -n 's/^XDG_RUNTIME_DIR=//p' "$T/s.log" | sed -n 1p)
[ "$rc" = 3 ] && ok "VU-3a the tool's exit status 3 passes through" || bad "VU-3a the tool's exit status passes through" "rc=$rc"
{ [ -n "$priv" ] && [ ! -e "$priv" ]; } && ok "VU-3b the private directory is removed after a failure" || bad "VU-3b the private directory is removed after a failure" "'$priv'"

# VU-4 a call with no unit is refused
sh "$vu" >/dev/null 2>&1; rc=$?
[ "$rc" = 2 ] && ok "VU-4 no unit given: exit 2" || bad "VU-4 no unit given: exit 2" "rc=$rc"

echo "verify-units-test: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
