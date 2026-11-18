#!/bin/sh
# THE RELEASE'S OWN FAIL-CLOSED CHECK OF THE MAC INSTALLER (mike's re-reads of 48ad6e0 and a523b52, MAC-MIKE-03 to -06: ci.sh without --strict lets a NOT_RUN gate pass, the Mac suite skips IT-M29 without a set, and a receipt must name what was TESTED).
#   scripts/install-mac-release-check.sh SET_DIR KEYFILE VERSION
#     SET_DIR  the SIGNED Mac release set (the seven files: cortex-VERSION-macos.tar.gz, cortex-server-VERSION-aarch64.oci.tar, install.sh, manifest.json, install.meta, SHA256SUMS, SHA256SUMS.sig), made AND SIGNED by the release recipe
#     KEYFILE  the CTO's PUBLIC key file: one line 'principal ssh-ed25519 key'
#     VERSION  the release version of the set
# It takes a READ-ONLY SNAPSHOT of the seven files and the key file (a private 0700 directory, copied, hashed BEFORE the suite runs, hashed again AFTER: a change is a FAILURE), runs scripts/install-mac-test.sh BESIDE IT against the SNAPSHOT with INSTALL_MAC_REQUIRE_SET=1 and INSTALL_MAC_REQUIRE_ALL=1
# (IT-M29, the joined installer of this tree against the signed set, must RUN and pass, and so must EVERY cell of scripts/install-mac-required-cells.txt: a cell that cannot run, or that a switch or a filter left out, is a failure) and takes the suite's test-only switches (INSTALL_MAC_ONLY, INSTALL_MAC_SH, INSTALL_MAC_INNER,
# INSTALL_MAC_NO_SELFRUN) out of the environment. It judges the OUTPUT too: exit 0, no FAIL, skip or NOT_RUN line, an 'ok' line for EVERY required cell, and a summary 'N passed, 0 failed, 0 not run' with N above 0. The PASS receipt names the hashes of the files that were EXERCISED (SHA256SUMS, its signature, the set's install.sh, the key file), every digest checked to be 64 hex characters.
# Anything else is exit 1 and says why; a wrong argument, a missing file or a digest that cannot be made is exit 2 BEFORE the suite runs. The release run calls THIS script after signing; it is the Mac suite's gate for the release, not ci.sh (which treats NOT_RUN as a pass unless --strict).
# Built and tested against fakes (scripts/install-mac-release-check-test.sh): it proves the check's own logic, not a Mac.
set -eu
umask 077
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
fail() { printf 'install-mac-release-check: FAIL: %s\n' "$1" >&2; exit "${2:-1}"; }
# sha_of FILE: the sha256 as 64 lower-case hex characters on stdout, or a non-zero status (a failing hash command is never masked by a pipe: the digest comes from a command substitution of its own)
sha_of() {
  if command -v sha256sum >/dev/null 2>&1; then sho=$(sha256sum "$1") || return 1; else sho=$(shasum -a 256 "$1") || return 1; fi   # portable: ok sha256sum first, shasum -a 256 (a Mac's) is the fallback
  sho=${sho%% *}
  [ "${#sho}" -eq 64 ] || return 1
  case $sho in *[!0123456789abcdef]*) return 1 ;; esac
  printf '%s' "$sho"
}
[ "$#" -eq 3 ] || fail "usage: install-mac-release-check.sh SET_DIR KEYFILE VERSION" 2
set_dir=$1; key=$2; ver=$3
case $ver in ""|*[!0123456789.ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz-]*) fail "VERSION is not a plain version: $ver" 2 ;; esac
{ [ -d "$set_dir" ] && [ ! -L "$set_dir" ]; } || fail "SET_DIR is not a real directory: $set_dir" 2
set_files="cortex-$ver-macos.tar.gz cortex-server-$ver-aarch64.oci.tar install.sh manifest.json install.meta SHA256SUMS SHA256SUMS.sig"
for f in $set_files; do
  { [ -f "$set_dir/$f" ] && [ ! -L "$set_dir/$f" ]; } || fail "the set lacks a plain file $f" 2
done
{ [ -f "$key" ] && [ ! -L "$key" ]; } || fail "KEYFILE is not a plain file: $key" 2
{ [ "$(wc -l < "$key" | tr -d ' ')" = 1 ] && awk 'NF == 3 && $2 == "ssh-ed25519" { ok = 1 } END { exit ok ? 0 : 1 }' "$key"; } || fail "KEYFILE is not ONE line 'principal ssh-ed25519 key': $key" 2
suite=$here/install-mac-test.sh
[ -f "$suite" ] || fail "no install-mac-test.sh beside this script ($here)" 2
required=$here/install-mac-required-cells.txt
[ -f "$required" ] || fail "no install-mac-required-cells.txt beside this script ($here)" 2
grep -qE '^IT-M[0-9]' "$required" || fail "install-mac-required-cells.txt lists no cell" 2
snap=$(mktemp -d "${TMPDIR:-/tmp}/install-mac-release-check.XXXXXXXX") || fail "no temporary directory" 2
out=$snap/suite.out
cleanup() { chmod -R u+w "${snap:?}" 2>/dev/null || :; rm -rf -- "${snap:?}"; }
trap cleanup EXIT
# THE SNAPSHOT: what is hashed and exercised is this copy, which nothing else holds open
mkdir "$snap/set" || fail "no snapshot directory" 2
for f in $set_files; do cp "$set_dir/$f" "$snap/set/$f" || fail "cannot copy $f into the snapshot" 2; done
cp "$key" "$snap/key.pub" || fail "cannot copy the key file into the snapshot" 2
# digest_all OUTFILE (mike MAC-MIKE-10): the sha256 of EACH of the seven snapshot files and of the key file, one line 'sha256 name' each, in a fixed order; a digest that cannot be made (not 64 hex characters, or a failing command) is a non-zero status
digest_all() {
  : > "$1" || return 1
  for da_f in $set_files; do da_d=$(sha_of "$snap/set/$da_f") || return 1; printf '%s %s\n' "$da_d" "$da_f" >> "$1" || return 1; done
  da_d=$(sha_of "$snap/key.pub") || return 1
  printf '%s key.pub\n' "$da_d" >> "$1"
}
digest_all "$snap/digests.before" || fail "cannot make the sha256 of every file of the snapshot (a digest of 64 hex characters for each of the seven files and the key file)" 2
d_of() { awk -v n="$1" '$2 == n { print $1 }' "$snap/digests.before"; }
d_sums=$(d_of SHA256SUMS); d_sig=$(d_of SHA256SUMS.sig); d_inst=$(d_of install.sh); d_key=$(d_of key.pub)
d_tgz=$(d_of "cortex-$ver-macos.tar.gz"); d_oci=$(d_of "cortex-server-$ver-aarch64.oci.tar"); d_man=$(d_of manifest.json); d_meta=$(d_of install.meta)
chmod -R a-w "$snap/set" "$snap/key.pub"
rc=0
env -u INSTALL_MAC_ONLY -u INSTALL_MAC_SH -u INSTALL_MAC_INNER -u INSTALL_MAC_NO_SELFRUN INSTALL_MAC_REQUIRE_SET=1 INSTALL_MAC_REQUIRE_ALL=1 MAC_SET_DIR="$snap/set" MAC_SET_KEYFILE="$snap/key.pub" MAC_SET_VERSION="$ver" sh "$suite" > "$out" 2>&1 || rc=$?
why=""
[ "$rc" -eq 0 ] || why="$why [the suite exited $rc, not 0]"
if grep -q '^FAIL ' "$out"; then why="$why [a cell FAILED: $(grep '^FAIL ' "$out" | head -1 | cut -c1-80)]"; fi
if grep -q '^skip ' "$out"; then why="$why [a cell was skipped: $(grep '^skip ' "$out" | head -1 | cut -c1-80)]"; fi
if grep -q '^NOT_RUN: ' "$out"; then why="$why [the suite says NOT_RUN]"; fi
# THE OUTPUT IS RECONCILED (mike MAC-MIKE-11): a result record is a line that BEGINS with 'ok   IT-M' (a mention inside another line is no record); every required cell has an ok record, no cell has two (so each exactly one), no cell outside the list has one,
# and the summary's count of passed cells is the number of ok records (a partial or malformed transcript cannot make an inaccurate receipt)
ok_ids=$snap/ok.ids; req_ids=$snap/req.ids
awk '/^ok   IT-M/ { print $2 }' "$out" > "$ok_ids"
grep -E '^IT-M[0-9]' "$required" > "$req_ids" || :
dups=$(sort "$ok_ids" | uniq -d | tr '\n' ' ')
[ -z "$dups" ] || why="$why [a cell has more than one ok record: $dups]"
miss=""
while IFS= read -r rid; do
  grep -qxF -- "$rid" "$ok_ids" || miss="$miss $rid"
done < "$req_ids"
[ -z "$miss" ] || why="$why [required cell(s) with no ok record of their own:$miss]"
extra=""
while IFS= read -r oid; do
  grep -qxF -- "$oid" "$req_ids" || extra="$extra $oid"
done < "$ok_ids"
[ -z "$extra" ] || why="$why [ok record(s) for cell(s) that are not in the required list:$extra]"
nsl=$(grep -c '^install-mac-test: ' "$out" || true)
n=$(sed -n 's/^install-mac-test: \([0-9][0-9]*\) passed, 0 failed, 0 not run$/\1/p' "$out" | tail -1)
{ [ "$nsl" = 1 ] && [ -n "$n" ] && [ "$n" -gt 0 ]; } || why="$why [the output has not exactly one summary 'N passed, 0 failed, 0 not run' with N above 0: $(grep '^install-mac-test: ' "$out" | tail -1 | cut -c1-80)]"
nok=$(wc -l < "$ok_ids" | tr -d ' ')
{ [ -z "$n" ] || [ "$n" = "$nok" ]; } || why="$why [the summary says $n passed but the output holds $nok ok records]"
# the inputs are the ones that were hashed (mike MAC-MIKE-10): EVERY file of the snapshot, the key file too, must hash as it did before the run
if digest_all "$snap/digests.after"; then
  cmp -s "$snap/digests.before" "$snap/digests.after" || why="$why [the snapshot CHANGED during the run, so the receipt would name inputs that were not exercised: $(diff "$snap/digests.before" "$snap/digests.after" | sed -n 's/^> [0-9a-f]* //p' | tr '\n' ' ')]"
else
  why="$why [the snapshot could not be hashed again after the run]"
fi
if [ -n "$why" ]; then
  fail "the Mac installer's release check did not pass:$why; last lines of the suite: $(tail -4 "$out" | tr '\n' '|' | cut -c1-300)" 1
fi
printf 'install-mac-release-check: PASS: %s cells passed, 0 failed, 0 not run; every cell of install-mac-required-cells.txt ran, IT-M29 against the signed set %s (version %s; EXERCISED: SHA256SUMS sha256 %s, SHA256SUMS.sig sha256 %s, install.sh sha256 %s, public key file sha256 %s); the other files of the set EXERCISED: cortex-%s-macos.tar.gz sha256 %s, cortex-server-%s-aarch64.oci.tar sha256 %s, manifest.json sha256 %s, install.meta sha256 %s\n' "$n" "$set_dir" "$ver" "$d_sums" "$d_sig" "$d_inst" "$d_key" "$ver" "$d_tgz" "$ver" "$d_oci" "$d_man" "$d_meta"
