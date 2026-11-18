#!/bin/sh
# F-2: synthetic sudo records its inherited mask; no root command is executed.
set -eu
: "${TMPDIR:?private scratch required}"
case $TMPDIR in /tmp|/tmp/*) exit 2 ;; esac
include=$1
work=$(mktemp -d "$TMPDIR/prereq-umask.XXXXXXXX")
trap 'rm -rf -- "$work"' EXIT HUP INT TERM
mkdir "$work/root" "$work/private"
printf '%s\n' "$work/root/bootstrap.rpm|boot|1|2|3|4|5|6" "$work/root/compose.rpm|compose|1|2|3|4|5|6" > "$work/root/transaction.manifest"
printf '%s\n' roots > "$work/private/roots.receipt"
# Replace the absolute sudo path only in a private copy, so the shell fake
# below records the call and the real sudo executable is unreachable.
sed 's|/usr/bin/sudo|sudo|g' "$include" > "$work/include.sh"
. "$work/include.sh"
say() { :; }
prereq_rocky_seal_ok() { return 0; }
prereq_root_locked_path() { return 0; }
prereq_rocky_candidates() { awk -F'|' 'NF == 8 { print $1 "|" $2 "|" $3 "|" $4 "|" $5 "|" $6 "|" $7 }' "$1/transaction.manifest"; }
prereq_rocky_metadata_ok() { printf '%s\n' metadata; }
prereq_rocky_archives_ok() { printf '%s\n' archives; }
prereq_recheck_closure() { return 0; }
sudo() { case ${1-} in dnf|usermod|loginctl) ;; *) exit 91 ;; esac; umask >> "$work/masks"; [ "$1" != loginctl ] || : > "$work/linger"; }
prereq_subid_range() { printf '%s\n' 100000-165535; }
prereq_subid_has_range() { return 0; }
loginctl() {
  case ${1-} in
    show-user) if [ -e "$work/linger" ]; then printf '%s\n' yes; else printf '%s\n' no; fi ;;
    enable-linger) : ;;
    *) return 1 ;;
  esac
}
umask 077
prereq_rocky_install "$work/root" "$work/private" 10.2 main synthetic
PREREQ_MIXED=1
PREREQ_RECHECK_BOOT_STAGE=synthetic PREREQ_RECHECK_BOOT_MANIFEST=synthetic PREREQ_RECHECK_BOOT_SIZES=synthetic PREREQ_RECHECK_BOOT_POLICY=synthetic
PREREQ_ROCKY_ROOT_MANIFEST=$(cat "$work/root/transaction.manifest")
PREREQ_BOOT_ROOT_FILES="$work/root/bootstrap.rpm"
prereq_mixed_locked_install "$work/root" "$work/private" 10.2 synthetic
prereq_fix_subids cortexuser 100000-165535 1 1
prereq_fix_linger cortexuser
[ "$(cat "$work/masks")" = "$(printf '0022\n0022\n0022\n0022\n0022')" ] || { printf 'FAIL F-2: fake sudo masks:\n' >&2; cat "$work/masks" >&2; exit 1; }
[ "$(umask)" = 0077 ] || { printf '%s\n' 'FAIL F-2: caller umask changed' >&2; exit 1; }
printf '%s\n' 'PASS F-2: fake sudo dnf, usermod and loginctl calls use 022 and caller remains 077'
