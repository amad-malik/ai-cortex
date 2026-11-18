#!/bin/sh
# Logic test for scripts/install-mac-release-check.sh (the release's fail-closed check of the Mac installer; mike's re-read of 48ad6e0) against a FAKE suite that prints and exits as each cell says. It proves the check's own logic only.
#   sh scripts/install-mac-release-check-test.sh [SCRIPT]     SCRIPT is the script under test (default: the one beside this file); the mutation run (scripts/shell_mutants.py with scripts/install-mac-release-check-mutants.json) passes a mutated copy
# A cell that fails prints 'RELEASE CHECK WRONG: <what>' (the mutation run's kill marker).
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
script=${1:-"$here/install-mac-release-check.sh"}
[ -f "$script" ] || { echo "RELEASE CHECK WRONG: no script to test: $script"; exit 2; }
T=$(mktemp -d /tmp/cortex-mac-rc-test.XXXXXXXX)
trap 'rm -rf -- "$T"' EXIT
pass=0 fail=0
ok() { echo "ok   $1"; pass=$((pass + 1)); }
bad() { echo "RELEASE CHECK WRONG: $1"; [ -z "${2:-}" ] || echo "        $2"; fail=$((fail + 1)); }
# mkcase NAME: a directory with a COPY of the script, a fake suite (the lines in $T/NAME/lines are printed, then exit $T/NAME/rc; it records its call and its environment), a set of seven files and a key file
mkcase() {
  c=$T/$1; rm -rf "$c"; mkdir -p "$c/set"
  cp "$script" "$c/install-mac-release-check.sh"
  printf '# required cells\nIT-M01\nIT-M29\n' > "$c/install-mac-required-cells.txt"
  printf '#!/bin/sh\necho called > "%s/called"\nenv | sort | grep -E "^(INSTALL_MAC_|MAC_SET_)" > "%s/env.rec" || :\n( echo probe >> "$MAC_SET_DIR/install.sh" ) 2>/dev/null || :\necho "ok   IT-M01 a cell that every case passes (the required list holds IT-M01 and IT-M29)"\ncat "%s/lines"\nexit "$(cat "%s/rc")"\n' "$c" "$c" "$c" "$c" > "$c/install-mac-test.sh"
  for f in cortex-0.2.2-macos.tar.gz cortex-server-0.2.2-aarch64.oci.tar install.sh manifest.json install.meta SHA256SUMS SHA256SUMS.sig; do printf 'the content of %s\n' "$f" > "$c/set/$f"; done   # a DIFFERENT content per file, so that a receipt that names one file's hash where another's belongs is seen
  printf 'cto@example ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFAKEKEYFAKEKEYFAKEKEYFAKEKEYFAKEKEYFAKEKEYFAKE\n' > "$c/key.pub"
  : > "$c/lines"; echo 0 > "$c/rc"
}
# run NAME [SET_DIR KEY VERSION]: sets rc, $c/stdout, $c/stderr
run() {
  c=$T/$1; shift
  if [ "$#" -eq 0 ]; then set -- "$c/set" "$c/key.pub" 0.2.2; fi
  rc=0; INSTALL_MAC_ONLY=IT-M33 INSTALL_MAC_SH=/nonexistent INSTALL_MAC_INNER=1 INSTALL_MAC_NO_SELFRUN=1 sh "$c/install-mac-release-check.sh" "$@" > "$c/stdout" 2> "$c/stderr" || rc=$?
}
good_lines='ok   IT-M29 a set made by the release recipe passes the joined end-to-end route
install-mac-test: 2 passed, 0 failed, 0 not run'
# expect_fail NAME WHAT: exit 1, a FAIL line on stderr, no PASS on stdout
expect_fail() {
  if [ "$rc" = 1 ] && grep -q '^install-mac-release-check: FAIL' "$T/$1/stderr" && ! grep -q 'PASS' "$T/$1/stdout"; then ok "$2"; else bad "$2" "rc=$rc stdout=$(head -c 120 "$T/$1/stdout") stderr=$(head -c 160 "$T/$1/stderr")"; fi
}
# RC01 the good case: PASS, exit 0, the receipt names the cells and the set; the suite got the right environment and NOT the caller's test-only switches
mkcase g; printf '%s\n' "$good_lines" > "$T/g/lines"; run g
if [ "$rc" = 0 ] && grep -q '^install-mac-release-check: PASS: 2 cells passed, 0 failed, 0 not run; every cell of install-mac-required-cells.txt ran, IT-M29 against the signed set' "$T/g/stdout"; then ok "RC01 a suite that passes all of it, IT-M29 included, is PASS with exit 0 and a receipt"; else bad "RC01 the good case" "rc=$rc $(head -c 200 "$T/g/stdout") $(head -c 200 "$T/g/stderr")"; fi
hs=$(sha256sum "$T/g/set/SHA256SUMS" | cut -d' ' -f1); hg=$(sha256sum "$T/g/set/SHA256SUMS.sig" | cut -d' ' -f1); hi=$(sha256sum "$T/g/set/install.sh" | cut -d' ' -f1); hk=$(sha256sum "$T/g/key.pub" | cut -d' ' -f1)   # portable: ok the test host's own GNU tool
{ grep -qF "SHA256SUMS sha256 $hs" "$T/g/stdout" && grep -qF "SHA256SUMS.sig sha256 $hg" "$T/g/stdout" && grep -qF "install.sh sha256 $hi" "$T/g/stdout" && grep -qF "public key file sha256 $hk" "$T/g/stdout"; } && ok "RC01b the PASS receipt names the sha256 of SHA256SUMS, its signature, the set's install.sh and the key file (the files EXERCISED)" || bad "RC01b the receipt's hashes" "$(head -c 300 "$T/g/stdout")"
h1=$(sha256sum "$T/g/set/cortex-0.2.2-macos.tar.gz" | cut -d' ' -f1); h2=$(sha256sum "$T/g/set/cortex-server-0.2.2-aarch64.oci.tar" | cut -d' ' -f1); h3=$(sha256sum "$T/g/set/manifest.json" | cut -d' ' -f1); h4=$(sha256sum "$T/g/set/install.meta" | cut -d' ' -f1)   # portable: ok the test host's own GNU tool
{ grep -qF "cortex-0.2.2-macos.tar.gz sha256 $h1" "$T/g/stdout" && grep -qF "cortex-server-0.2.2-aarch64.oci.tar sha256 $h2" "$T/g/stdout" && grep -qF "manifest.json sha256 $h3" "$T/g/stdout" && grep -qF "install.meta sha256 $h4" "$T/g/stdout"; } && ok "RC01c the PASS receipt also names the sha256 of the archive, the image, manifest.json and install.meta (every file of the set that was exercised)" || bad "RC01c the other hashes" "$(head -c 300 "$T/g/stdout")"
e=$(cat "$T/g/env.rec" 2>/dev/null || :)
{ printf '%s\n' "$e" | grep -qx 'INSTALL_MAC_REQUIRE_SET=1' && printf '%s\n' "$e" | grep -qx 'INSTALL_MAC_REQUIRE_ALL=1' && printf '%s\n' "$e" | grep -q '^MAC_SET_DIR=.*/install-mac-release-check\..*/set$' && printf '%s\n' "$e" | grep -q '^MAC_SET_KEYFILE=.*/install-mac-release-check\..*/key.pub$' && printf '%s\n' "$e" | grep -qx 'MAC_SET_VERSION=0.2.2'; } && ok "RC02 the suite runs with INSTALL_MAC_REQUIRE_SET=1, INSTALL_MAC_REQUIRE_ALL=1 and the set, key file and version" || bad "RC02 the suite's environment" "$e"
{ ! printf '%s\n' "$e" | grep -q '^INSTALL_MAC_ONLY=' && ! printf '%s\n' "$e" | grep -q '^INSTALL_MAC_SH=' && ! printf '%s\n' "$e" | grep -q '^INSTALL_MAC_INNER=' && ! printf '%s\n' "$e" | grep -q '^INSTALL_MAC_NO_SELFRUN='; } && ok "RC03 the caller's INSTALL_MAC_ONLY, INSTALL_MAC_SH, INSTALL_MAC_INNER and INSTALL_MAC_NO_SELFRUN (which narrow, replace or shorten the run) never reach the suite" || bad "RC03 test-only switches reached the suite" "$e"
# the failing suites
mkcase n; printf 'skip IT-M29 NOT RUN: no set was given\ninstall-mac-test: 1 passed, 0 failed, 1 not run\nNOT_RUN: 1 mandatory cell(s) not run (IT-M29): x\n' > "$T/n/lines"; echo 77 > "$T/n/rc"; run n
expect_fail n "RC04 a suite that exits 77 (NOT_RUN: a cell did not run) is FAIL"
mkcase l; printf 'ok   IT-M29 a set\ninstall-mac-test: 2 passed, 0 failed, 1 not run\n' > "$T/l/lines"; run l
expect_fail l "RC05 a suite that exits 0 but says '1 not run' is FAIL (the output is judged, not only the exit)"
mkcase m; printf 'install-mac-test: 1 passed, 0 failed, 0 not run\n' > "$T/m/lines"; run m
expect_fail m "RC06 a suite with no 'ok   IT-M29' line is FAIL"
mkcase s; printf 'ok   IT-M29 a set\nskip IT-M05b NOT RUN: no python\ninstall-mac-test: 2 passed, 0 failed, 0 not run\n' > "$T/s/lines"; run s
expect_fail s "RC07 a 'skip' line is FAIL even when the exit is 0 and the summary lies"
mkcase f; printf 'ok   IT-M29 a set\nFAIL IT-M03 a cell\ninstall-mac-test: 2 passed, 1 failed, 0 not run\n' > "$T/f/lines"; echo 1 > "$T/f/rc"; run f
expect_fail f "RC08 a failing cell is FAIL (exit 1)"
mkcase fl; printf 'ok   IT-M29 a set\nFAIL IT-M03 a cell\ninstall-mac-test: 2 passed, 0 failed, 0 not run\n' > "$T/fl/lines"; run fl
expect_fail fl "RC09 a FAIL line is FAIL even when the exit is 0 and the summary lies"
mkcase z; printf 'ok   IT-M29 a set\ninstall-mac-test: 0 passed, 0 failed, 0 not run\n' > "$T/z/lines"; run z
expect_fail z "RC10 a run of zero cells is FAIL"
mkcase nr; printf 'ok   IT-M29 a set\nNOT_RUN: something\ninstall-mac-test: 2 passed, 0 failed, 0 not run\n' > "$T/nr/lines"; run nr
expect_fail nr "RC11 a NOT_RUN line is FAIL even when the exit is 0"
mkcase ns; printf 'ok   IT-M29 a set\n' > "$T/ns/lines"; run ns
expect_fail ns "RC12 no summary line is FAIL"
mkcase x1; printf '%s\n' "$good_lines" > "$T/x1/lines"; echo 3 > "$T/x1/rc"; run x1
expect_fail x1 "RC13 any non-zero exit is FAIL even with a good-looking output"
# the arguments: exit 2, BEFORE the suite runs
argcase() { # argcase NAME WHAT ARGS...: exit 2 and the fake suite was not called
  n=$1; w=$2; shift 2
  mkcase "$n"; printf '%s\n' "$good_lines" > "$T/$n/lines"
  c=$T/$n; rc=0; sh "$c/install-mac-release-check.sh" "$@" > "$c/stdout" 2> "$c/stderr" || rc=$?
  if [ "$rc" = 2 ] && [ ! -e "$c/called" ] && grep -q '^install-mac-release-check: FAIL' "$c/stderr"; then ok "$w"; else bad "$w" "rc=$rc called=$([ -e "$c/called" ] && echo yes || echo no) $(head -c 120 "$c/stderr")"; fi
}
argcase a0 "RC14 no arguments: exit 2, nothing runs"
argcase a2 "RC15 two arguments: exit 2, nothing runs" x y
mkcase a3; rm -f "$T/a3/set/SHA256SUMS.sig"; rc=0; sh "$T/a3/install-mac-release-check.sh" "$T/a3/set" "$T/a3/key.pub" 0.2.2 > "$T/a3/stdout" 2> "$T/a3/stderr" || rc=$?
{ [ "$rc" = 2 ] && [ ! -e "$T/a3/called" ]; } && ok "RC16 a set that lacks one of the seven files (SHA256SUMS.sig): exit 2, nothing runs" || bad "RC16 an incomplete set" "rc=$rc"
mkcase a4; ln -s "$T/a4/set" "$T/a4/setlink"; rc=0; sh "$T/a4/install-mac-release-check.sh" "$T/a4/setlink" "$T/a4/key.pub" 0.2.2 > "$T/a4/stdout" 2> "$T/a4/stderr" || rc=$?
{ [ "$rc" = 2 ] && [ ! -e "$T/a4/called" ]; } && ok "RC17 a SET_DIR that is a symbolic link: exit 2, nothing runs" || bad "RC17 a linked set" "rc=$rc"
mkcase a5; printf 'one ssh-ed25519 AAAA\ntwo ssh-ed25519 BBBB\n' > "$T/a5/key.pub"; rc=0; sh "$T/a5/install-mac-release-check.sh" "$T/a5/set" "$T/a5/key.pub" 0.2.2 > "$T/a5/stdout" 2> "$T/a5/stderr" || rc=$?
{ [ "$rc" = 2 ] && [ ! -e "$T/a5/called" ]; } && ok "RC18 a key file of two lines: exit 2, nothing runs" || bad "RC18 a two-line key file" "rc=$rc"
mkcase a6; printf 'cto@example ssh-rsa AAAA\n' > "$T/a6/key.pub"; rc=0; sh "$T/a6/install-mac-release-check.sh" "$T/a6/set" "$T/a6/key.pub" 0.2.2 > "$T/a6/stdout" 2> "$T/a6/stderr" || rc=$?
{ [ "$rc" = 2 ] && [ ! -e "$T/a6/called" ]; } && ok "RC19 a key file that is not ssh-ed25519: exit 2, nothing runs" || bad "RC19 a wrong key type" "rc=$rc"
mkcase a7; for f in "cortex-0.2.2;x-macos.tar.gz" "cortex-server-0.2.2;x-aarch64.oci.tar"; do printf 'x\n' > "$T/a7/set/$f"; done; rc=0; sh "$T/a7/install-mac-release-check.sh" "$T/a7/set" "$T/a7/key.pub" '0.2.2;x' > "$T/a7/stdout" 2> "$T/a7/stderr" || rc=$?
{ [ "$rc" = 2 ] && [ ! -e "$T/a7/called" ]; } && ok "RC20 a version with a character that is not a letter, digit, dot or hyphen (a semicolon), even when the set's files carry that name: exit 2, nothing runs" || bad "RC20 a bad version" "rc=$rc"
mkcase a8; rm -f "$T/a8/install-mac-test.sh"; rc=0; sh "$T/a8/install-mac-release-check.sh" "$T/a8/set" "$T/a8/key.pub" 0.2.2 > "$T/a8/stdout" 2> "$T/a8/stderr" || rc=$?
[ "$rc" = 2 ] && ok "RC21 no install-mac-test.sh beside the script: exit 2" || bad "RC21 no suite" "rc=$rc"
# RC22 a REQUIRED cell that did not print ok (omitted by a switch or a filter) is FAIL even when the exit is 0 and the summary is clean
mkcase q; printf '# required cells\nIT-M01\nIT-M29\nIT-M38\n' > "$T/q/install-mac-required-cells.txt"; printf '%s\n' "$good_lines" > "$T/q/lines"; run q
expect_fail q "RC22 a cell of the required list that has no ok line (IT-M38 here) is FAIL, whatever the exit and the summary say"
# RC23 no required list: exit 2, nothing runs
mkcase r; rm -f "$T/r/install-mac-required-cells.txt"; printf '%s\n' "$good_lines" > "$T/r/lines"; rc=0; sh "$T/r/install-mac-release-check.sh" "$T/r/set" "$T/r/key.pub" 0.2.2 > "$T/r/stdout" 2> "$T/r/stderr" || rc=$?
{ [ "$rc" = 2 ] && [ ! -e "$T/r/called" ]; } && ok "RC23 no install-mac-required-cells.txt beside the script: exit 2, nothing runs" || bad "RC23 no required list" "rc=$rc"
# RC24 THE SNAPSHOT: the suite is given a COPY of the set (not the live directory), read-only
mkcase p; printf '%s\n' "$good_lines" > "$T/p/lines"; run p
sd=$(sed -n 's/^MAC_SET_DIR=//p' "$T/p/env.rec")
{ [ -n "$sd" ] && [ "$sd" != "$T/p/set" ]; } && ok "RC24 the suite is given a SNAPSHOT of the set, not the live directory" || bad "RC24 the suite got the live set directory" "$sd"
# RC25 the LIVE SHA256SUMS changes during the run: the receipt names the hash of what was EXERCISED (the one at the start)
mkcase l2; printf '%s\n' "$good_lines" > "$T/l2/lines"
sed 's|^cat "|echo changed >> "$FAKE_LIVE_SUMS"\ncat "|' "$T/l2/install-mac-test.sh" > "$T/l2/install-mac-test.sh.new" && mv "$T/l2/install-mac-test.sh.new" "$T/l2/install-mac-test.sh"
h0=$(sha256sum "$T/l2/set/SHA256SUMS" | cut -d' ' -f1)   # portable: ok the test host's own GNU tool
c=$T/l2; rc=0; FAKE_LIVE_SUMS="$c/set/SHA256SUMS" sh "$c/install-mac-release-check.sh" "$c/set" "$c/key.pub" 0.2.2 > "$c/stdout" 2> "$c/stderr" || rc=$?
{ [ "$rc" = 0 ] && grep -qF "SHA256SUMS sha256 $h0" "$c/stdout" && [ "$(sha256sum "$c/set/SHA256SUMS" | cut -d' ' -f1)" != "$h0" ]; } && ok "RC25 the live SHA256SUMS changed during the run: the receipt names the hash that was EXERCISED (the snapshot's), not the live file's" || bad "RC25 the receipt follows the live file" "rc=$rc $(head -c 200 "$c/stdout")"   # portable: ok the test host's own GNU tool
# RC26 the SNAPSHOT changes during the run (a suite that makes it writable and edits it): FAIL
mkcase w; printf '%s\n' "$good_lines" > "$T/w/lines"
sed 's|^cat "|chmod -R u+w "$MAC_SET_DIR"; echo x >> "$MAC_SET_DIR/SHA256SUMS"\ncat "|' "$T/w/install-mac-test.sh" > "$T/w/install-mac-test.sh.new" && mv "$T/w/install-mac-test.sh.new" "$T/w/install-mac-test.sh"
run w
expect_fail w "RC26 a snapshot that changed during the run is FAIL (the receipt would name inputs that were not exercised)"
# RC27 a hash command that fails, and RC28 one that prints no digest: exit 2 BEFORE the suite runs
mkcase h1; mkdir -p "$T/h1/fb"; printf '#!/bin/sh\nexit 1\n' > "$T/h1/fb/sha256sum"; cp "$T/h1/fb/sha256sum" "$T/h1/fb/shasum"; chmod 755 "$T/h1/fb/sha256sum" "$T/h1/fb/shasum"; printf '%s\n' "$good_lines" > "$T/h1/lines"
rc=0; PATH="$T/h1/fb:$PATH" sh "$T/h1/install-mac-release-check.sh" "$T/h1/set" "$T/h1/key.pub" 0.2.2 > "$T/h1/stdout" 2> "$T/h1/stderr" || rc=$?
{ [ "$rc" = 2 ] && [ ! -e "$T/h1/called" ] && ! grep -q PASS "$T/h1/stdout"; } && ok "RC27 a failing hash command is exit 2 before the suite runs, never an empty digest in a PASS" || bad "RC27 a failing hash command" "rc=$rc $(head -c 160 "$T/h1/stderr")"
mkcase h2; mkdir -p "$T/h2/fb"; printf '#!/bin/sh\necho abcdef12\n' > "$T/h2/fb/sha256sum"; cp "$T/h2/fb/sha256sum" "$T/h2/fb/shasum"; chmod 755 "$T/h2/fb/sha256sum" "$T/h2/fb/shasum"; printf '%s\n' "$good_lines" > "$T/h2/lines"
rc=0; PATH="$T/h2/fb:$PATH" sh "$T/h2/install-mac-release-check.sh" "$T/h2/set" "$T/h2/key.pub" 0.2.2 > "$T/h2/stdout" 2> "$T/h2/stderr" || rc=$?
{ [ "$rc" = 2 ] && [ ! -e "$T/h2/called" ] && ! grep -q PASS "$T/h2/stdout"; } && ok "RC28 a hash command that prints no 64-hex digest is exit 2 before the suite runs" || bad "RC28 a bad digest" "rc=$rc $(head -c 160 "$T/h2/stderr")"
mkcase h3; mkdir -p "$T/h3/fb"; printf '#!/bin/sh\necho zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz\n' > "$T/h3/fb/sha256sum"; cp "$T/h3/fb/sha256sum" "$T/h3/fb/shasum"; chmod 755 "$T/h3/fb/sha256sum" "$T/h3/fb/shasum"; printf '%s\n' "$good_lines" > "$T/h3/lines"
rc=0; PATH="$T/h3/fb:$PATH" sh "$T/h3/install-mac-release-check.sh" "$T/h3/set" "$T/h3/key.pub" 0.2.2 > "$T/h3/stdout" 2> "$T/h3/stderr" || rc=$?
{ [ "$rc" = 2 ] && [ ! -e "$T/h3/called" ] && ! grep -q PASS "$T/h3/stdout"; } && ok "RC28b a 64-character digest that is not hex is exit 2 before the suite runs" || bad "RC28b a non-hex digest" "rc=$rc $(head -c 160 "$T/h3/stderr")"
mkcase h4; mkdir -p "$T/h4/fb"; printf '#!/bin/sh\necho 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\nexit 1\n' > "$T/h4/fb/sha256sum"; cp "$T/h4/fb/sha256sum" "$T/h4/fb/shasum"; chmod 755 "$T/h4/fb/sha256sum" "$T/h4/fb/shasum"; printf '%s\n' "$good_lines" > "$T/h4/lines"
rc=0; PATH="$T/h4/fb:$PATH" sh "$T/h4/install-mac-release-check.sh" "$T/h4/set" "$T/h4/key.pub" 0.2.2 > "$T/h4/stdout" 2> "$T/h4/stderr" || rc=$?
{ [ "$rc" = 2 ] && [ ! -e "$T/h4/called" ] && ! grep -q PASS "$T/h4/stdout"; } && ok "RC29 a hash command that prints a plausible digest and FAILS is exit 2 (its status is not masked)" || bad "RC29 a failing hash command with output" "rc=$rc $(head -c 160 "$T/h4/stderr")"
mkcase e0; printf '# required cells\n' > "$T/e0/install-mac-required-cells.txt"; printf '%s\n' "$good_lines" > "$T/e0/lines"; rc=0; sh "$T/e0/install-mac-release-check.sh" "$T/e0/set" "$T/e0/key.pub" 0.2.2 > "$T/e0/stdout" 2> "$T/e0/stderr" || rc=$?
{ [ "$rc" = 2 ] && [ ! -e "$T/e0/called" ]; } && ok "RC30 a required-cells list that lists no cell: exit 2, nothing runs" || bad "RC30 an empty required list" "rc=$rc"
# MAC-MIKE-10: the snapshot is hashed FILE BY FILE before and after: a suite that makes it writable and alters ANY file of it (not only SHA256SUMS) is FAIL
for sf in cortex-0.2.2-macos.tar.gz cortex-server-0.2.2-aarch64.oci.tar manifest.json install.meta SHA256SUMS.sig install.sh; do
  k=sn_$(printf '%s' "$sf" | tr -c 'A-Za-z0-9' '_'); mkcase "$k"; printf '%s\n' "$good_lines" > "$T/$k/lines"
  sed "s|^cat \"|chmod -R u+w \"\$MAC_SET_DIR\"; echo x >> \"\$MAC_SET_DIR/$sf\"\ncat \"|" "$T/$k/install-mac-test.sh" > "$T/$k/install-mac-test.sh.new" && mv "$T/$k/install-mac-test.sh.new" "$T/$k/install-mac-test.sh"
  run "$k"
  expect_fail "$k" "RC31 a snapshot whose $sf changed during the run is FAIL (every file of the snapshot is hashed before and after)"
done
mkcase sk; printf '%s\n' "$good_lines" > "$T/sk/lines"
sed 's|^cat "|chmod u+w "$MAC_SET_KEYFILE"; echo x >> "$MAC_SET_KEYFILE"\ncat "|' "$T/sk/install-mac-test.sh" > "$T/sk/install-mac-test.sh.new" && mv "$T/sk/install-mac-test.sh.new" "$T/sk/install-mac-test.sh"
run sk
expect_fail sk "RC31b a key file that changed during the run is FAIL"
# MAC-MIKE-11: the output is reconciled by ANCHORED unique result records
mkcase em; printf 'a note that merely says ok   IT-M29 inside a longer line\ninstall-mac-test: 2 passed, 0 failed, 0 not run\n' > "$T/em/lines"; run em
expect_fail em "RC32 'ok   IT-M29 ' inside another line is no result record (the summary here counts it, so only the anchoring can refuse it): a required cell with no record of its own is FAIL"
mkcase du; printf 'ok   IT-M29 a set\nok   IT-M29 the same cell again\ninstall-mac-test: 3 passed, 0 failed, 0 not run\n' > "$T/du/lines"; run du
expect_fail du "RC33 a cell with TWO ok records is FAIL (the summary is consistent with the records here)"
mkcase cm; printf 'ok   IT-M29 a set\ninstall-mac-test: 41 passed, 0 failed, 0 not run\n' > "$T/cm/lines"; run cm
expect_fail cm "RC34 a summary that counts more passed cells than there are ok records is FAIL (count mismatch)"
mkcase xt; printf 'ok   IT-M29 a set\nok   IT-M77 a cell that is not in the required list\ninstall-mac-test: 3 passed, 0 failed, 0 not run\n' > "$T/xt/lines"; run xt
expect_fail xt "RC35 an ok record for a cell that is not in the required list is FAIL"
mkcase s2; printf 'ok   IT-M29 a set\ninstall-mac-test: 2 passed, 0 failed, 0 not run\ninstall-mac-test: 2 passed, 0 failed, 0 not run\n' > "$T/s2/lines"; run s2
expect_fail s2 "RC36 two summary lines are FAIL (exactly one summary)"
echo "install-mac-release-check-test: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
