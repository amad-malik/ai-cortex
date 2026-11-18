#!/bin/sh
# IP-53: each prerequisite message has one plain cause and one actionable fix.
# This suite sources only the functions-only include; it never runs host package tools.
set -eu

prereq_screen_case='' prereq_screen_kind=''
case ${PREREQ_SCREEN_MUTANT:-} in
  '') ;;
  IPM-01) prereq_screen_kind=os; prereq_screen_case=alma10 ;;
  IPM-02) prereq_screen_kind=os; prereq_screen_case=rocky9 ;;
  IPM-03) prereq_screen_kind=os; prereq_screen_case=injected ;;
  IPM-30) prereq_screen_kind=os; prereq_screen_case=duplicate ;;
  IPM-04) prereq_screen_kind=machine; prereq_screen_case=healthy ;;
  IPM-05) prereq_screen_kind=machine; prereq_screen_case=root ;;
  IPM-13) prereq_screen_kind=machine; prereq_screen_case=disk_graph_low ;;
  IPM-34) prereq_screen_kind=machine; prereq_screen_case=healthy ;;
  IPM-35) prereq_screen_kind=machine; prereq_screen_case=healthy ;;
  IPM-09) prereq_screen_kind=machine; prereq_screen_case=time_no ;;
  IPM-10) prereq_screen_kind=machine; prereq_screen_case=healthy ;;
  IPM-11) prereq_screen_kind=machine; prereq_screen_case=healthy ;;
  IPM-12) prereq_screen_kind=machine; prereq_screen_case=fs_home_nfs ;;
  IPM-14) prereq_screen_kind=machine; prereq_screen_case=port_substring ;;
  IPM-15) prereq_screen_kind=machine; prereq_screen_case=proc_free ;;
  IPM-16) prereq_screen_kind=machine; prereq_screen_case=sudo_missing ;;
  IPM-17) prereq_screen_kind=machine; prereq_screen_case=no_tty ;;
  IPM-19) prereq_screen_kind=machine; prereq_screen_case=healthy ;;
  IPM-22) prereq_screen_kind=machine; prereq_screen_case=podman_missing ;;
  IPM-23) prereq_screen_kind=machine; prereq_screen_case=gzip_missing ;;
  IPM-25) prereq_screen_kind=machine; prereq_screen_case=injected_provider ;;
  IPM-26) prereq_screen_kind=machine; prereq_screen_case=healthy ;;
  IPM-27) prereq_screen_kind=machine; prereq_screen_case=proc_bad ;;
  IPM-31) prereq_screen_kind=machine; prereq_screen_case=fs_graph_tmpfs ;;
  IPM-32) prereq_screen_kind=machine; prereq_screen_case=healthy ;;
  IPM-33) prereq_screen_kind=machine; prereq_screen_case=check_provider ;;
  IPM-36) prereq_screen_kind=machine; prereq_screen_case=check_missing ;;
  IPM-37) prereq_screen_kind=machine; prereq_screen_case=check_no_tty ;;
  IPM-38) prereq_screen_kind=machine; prereq_screen_case=epel_missing ;;
  IPM-39) prereq_screen_kind=machine; prereq_screen_case=os ;;
  IPM-40) prereq_screen_kind=machine; prereq_screen_case=dnf_missing ;;
  IPM-41) prereq_screen_kind=machine; prereq_screen_case=gpg_missing ;;
  IPM-42) prereq_screen_kind=machine; prereq_screen_case=coreutils_once ;;
  IPM-07) prereq_screen_kind=subid; prereq_screen_case=prefix ;;
  IPM-08) prereq_screen_kind=subid; prereq_screen_case=short ;;
  IPM-21) prereq_screen_kind=version; prereq_screen_case=1.5.0:0 ;;
  IPM-29) prereq_screen_kind=version; prereq_screen_case=1.4.9:1 ;;
  IPM-43) prereq_screen_kind=range; prereq_screen_case=occupied ;;
  IPM-44) prereq_screen_kind=repair; prereq_screen_case=stale ;;
  IPM-18) prereq_screen_kind=message; prereq_screen_case=disk_low ;;
  IPM-20) prereq_screen_kind=completion; prereq_screen_case=selected ;;
  IPM-24) prereq_screen_kind=provider; prereq_screen_case=selected ;;
  IPM-28) prereq_screen_kind=source; prereq_screen_case=selected ;;
  IPM-45) prereq_screen_kind=consent; prereq_screen_case=selected ;;
  IPM-65) prereq_screen_kind=signal; prereq_screen_case=selected ;;
  IPM-72) prereq_screen_kind=linger; prereq_screen_case=selected ;;
  IPM-46) prereq_screen_kind=key; prereq_screen_case=short ;;
  IPM-47) prereq_screen_kind=signature; prereq_screen_case=header_only ;;
  IPM-48) prereq_screen_kind=payload; prereq_screen_case=owner ;;
  IPM-49) prereq_screen_kind=payload; prereq_screen_case=ghost ;;
  IPM-50) prereq_screen_kind=payload; prereq_screen_case=skip_digest ;;
  IPM-51) prereq_screen_kind=payload; prereq_screen_case=missing_module ;;
  IPM-52) prereq_screen_kind=fetch; prereq_screen_case=repomd_mismatch ;;
  IPM-53) prereq_screen_kind=fetch; prereq_screen_case=package_mismatch ;;
  IPM-54) prereq_screen_kind=fetch; prereq_screen_case=metadata_foreign_sig ;;
  IPM-55) prereq_screen_kind=qualify; prereq_screen_case=valid ;;
  IPM-56) prereq_screen_kind=qualify; prereq_screen_case=bad_payload ;;
  IPM-73) prereq_screen_kind=fetch; prereq_screen_case=metadata_no_tool ;;
  IPM-58) prereq_screen_kind=candidate; prereq_screen_case=foreign_signature ;;
  IPM-59) prereq_screen_kind=extract; prereq_screen_case=changed ;;
  IPM-60) prereq_screen_kind=combine; prereq_screen_case=changed ;;
  IPM-67) prereq_screen_kind=extract; prereq_screen_case=foreign_origin ;;
  IPM-61) prereq_screen_kind=metadata; prereq_screen_case=classify_missing ;;
  IPM-68) prereq_screen_kind=nocurl; prereq_screen_case=rocky_redirect ;;
  IPM-69) prereq_screen_kind=nocurl; prereq_screen_case=redirect ;;
  IPM-70) prereq_screen_kind=pre; prereq_screen_case=signed_without_tool ;;
  IPM-06) prereq_screen_kind=flow; prereq_screen_case=approved ;;
  IPM-57) prereq_screen_kind=repairrun; prereq_screen_case=subids ;;
  IPM-64) prereq_screen_kind=flow; prereq_screen_case=missing_dependency ;;
  IPM-74) prereq_screen_kind=run; prereq_screen_case=unsafe_home ;;
  Q17E-BASE) prereq_screen_kind=run; prereq_screen_case=approved ;;
  Q17E-INHERITED) prereq_screen_kind=run; prereq_screen_case=inherited_boot ;;
  Q17E-EXTRACT) prereq_screen_kind=flow; prereq_screen_case=bootstrap_existing ;;
  IPM-158) prereq_screen_kind=run; prereq_screen_case=inherited_boot ;;
  IPM-75) prereq_screen_kind=flow; prereq_screen_case=unsafe_home_install ;;
  IPM-62) prereq_screen_kind=bootapply; prereq_screen_case=first_failure ;;
  IPM-63) prereq_screen_kind=bootapply; prereq_screen_case=changed_after_first ;;
  IPM-66) prereq_screen_kind=approved; prereq_screen_case=valid ;;
  IPM-71) prereq_screen_kind=mixedapply; prereq_screen_case=valid ;;
  IPM-76) prereq_screen_kind=real; prereq_screen_case=A ;;
  IPM-77|IPM-78|IPM-79|IPM-115) prereq_screen_kind=real; prereq_screen_case=B ;;
  IPM-80|IPM-81|IPM-82|IPM-133|IPM-134|IPM-135|IPM-136|IPM-137|IPM-138|IPM-139|IPM-140|IPM-141|IPM-142) prereq_screen_kind=real; prereq_screen_case=C ;;
  IPM-83) prereq_screen_kind=real; prereq_screen_case=D ;;
  IPM-84|IPM-85|IPM-86|IPM-87|IPM-88|IPM-89|IPM-90) prereq_screen_kind=real; prereq_screen_case=E ;;
  IPM-91|IPM-92|IPM-93|IPM-116) prereq_screen_kind=real; prereq_screen_case=F ;;
  IPM-94|IPM-95|IPM-96|IPM-97|IPM-98|IPM-99|IPM-100|IPM-101|IPM-102|IPM-103|IPM-104|IPM-105|IPM-106|IPM-107|IPM-108|IPM-109|IPM-110|IPM-111|IPM-112|IPM-113|IPM-114|IPM-144) prereq_screen_kind=real; prereq_screen_case=G ;;
  IPM-117|IPM-118|IPM-119|IPM-120|IPM-121|IPM-122|IPM-123|IPM-124|IPM-125|IPM-126|IPM-127|IPM-128|IPM-129|IPM-130|IPM-131|IPM-132|IPM-143) prereq_screen_kind=real; prereq_screen_case=L ;;
  IPM-145) prereq_screen_kind=qualify; prereq_screen_case=valid ;;
  IPM-149|IPM-150|IPM-151|IPM-152|IPM-153|IPM-155) prereq_screen_kind=mirror; prereq_screen_case=selected ;;
  IPM-154) prereq_screen_kind=signature_order; prereq_screen_case=selected ;;
  IPM-156|IPM-157) prereq_screen_kind=urllib_interrupt; prereq_screen_case=selected ;;
  IPM-146|IPM-147|IPM-148) prereq_screen_kind=real; prereq_screen_case=ORDER ;;
  *) printf 'unknown prerequisite mutation screen: %s\n' "$PREREQ_SCREEN_MUTANT" >&2; exit 2 ;;
esac

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
include=${PREREQ_TEST_SOURCE:-$root/deploy/install-prereq.sh}
original_include=$include
scratch=${PREREQ_TEST_TMP:-/path/to/workspace/scratch/marv/s5-prereq}
case $scratch in /tmp|/tmp/*) printf '%s\n' 'refusing test scratch under /tmp' >&2; exit 2 ;; esac
mkdir -p "$scratch"
test_dir=$(mktemp -d "$scratch/ip-53.XXXXXXXX")
trap 'rm -rf -- "$test_dir"' EXIT HUP INT TERM

# A successful direct run is evidence only if the required matrix ran.
prereq_test_locale_matrix() {
  pq_test_requested=${PREREQ_TEST_LOCALES:-C C.utf8}
  pq_test_has_c=0 pq_test_has_utf8=0
  if ! LC_ALL=C locale -a > "$test_dir/locale.available" 2> "$test_dir/locale.error" || [ -s "$test_dir/locale.error" ]; then
    printf '%s\n' 'FAIL IP-53: locale matrix cannot enumerate installed locales' >&2
    return 1
  fi
  for pq_test_locale in $pq_test_requested; do
    case $pq_test_locale in
      C) pq_test_has_c=1 ;;
      C.utf8) pq_test_has_utf8=1 ;;
    esac
    if ! LC_ALL=C grep -Fx -- "$pq_test_locale" "$test_dir/locale.available" >/dev/null ||
       ! LC_ALL="$pq_test_locale" locale charmap > "$test_dir/locale.charmap" 2> "$test_dir/locale.error" ||
       [ -s "$test_dir/locale.error" ]; then
      printf 'FAIL IP-53: locale matrix requested unavailable locale %s\n' "$pq_test_locale" >&2
      return 1
    fi
    if [ "$pq_test_locale" = C.utf8 ] && ! LC_ALL=C grep -Eix 'UTF-?8' "$test_dir/locale.charmap" >/dev/null; then
      printf '%s\n' 'FAIL IP-53: locale matrix C.utf8 is not UTF-8' >&2
      return 1
    fi
  done
  if [ "$pq_test_has_c" -ne 1 ] || [ "$pq_test_has_utf8" -ne 1 ]; then
    printf '%s\n' 'FAIL IP-53: locale matrix requires C and C.utf8' >&2
    return 1
  fi
}
prereq_test_locale_matrix || exit 1
prereq_screen_locale=''
for prereq_requested_locale in ${PREREQ_TEST_LOCALES:-C C.utf8}; do
  prereq_screen_locale=$prereq_requested_locale
  break
done
if [ -n "$prereq_screen_case" ]; then
  LC_ALL=$prereq_screen_locale
  export LC_ALL
fi
pq_test_completed_c=0 pq_test_completed_utf8=0

if [ ! -r "$include" ]; then
  printf '%s\n' 'FAIL IP-53: prerequisite include is missing' >&2
  exit 1
fi
if [ -z "$prereq_screen_case" ] || [ "$prereq_screen_kind" = mirror ]; then
  if ! PREREQ_TEST_SOURCE="$original_include" PREREQ_TEST_TMP="$scratch" python3 -I -B "$root/scripts/fakes/prereq/epel-mirror-test.py" > "$test_dir/mirror-result" 2>&1; then
    printf '%s\n' 'FAIL IP-mirror: offline mirror fallback regression' >&2
    cat "$test_dir/mirror-result" >&2
    exit 1
  fi
  if [ "$prereq_screen_kind" = mirror ]; then printf 'SCREEN %s: survived\n' "$PREREQ_SCREEN_MUTANT"; exit 0; fi
fi
if [ -z "$prereq_screen_case" ] || [ "$prereq_screen_kind" = signature_order ]; then
  if ! PREREQ_TEST_SOURCE="$original_include" PREREQ_TEST_TMP="$scratch" python3 -I -B "$root/scripts/fakes/prereq/epel-signature-order-test.py" > "$test_dir/signature-order-result" 2>&1; then
    printf '%s\n' 'FAIL IP-signature-order: RPM signature must fail without another mirror' >&2
    cat "$test_dir/signature-order-result" >&2
    exit 1
  fi
  if [ "$prereq_screen_kind" = signature_order ]; then printf 'SCREEN %s: survived\n' "$PREREQ_SCREEN_MUTANT"; exit 0; fi
fi
if [ -z "$prereq_screen_case" ] || [ "$prereq_screen_kind" = urllib_interrupt ]; then
  if ! PREREQ_TEST_SOURCE="$original_include" PREREQ_TEST_TMP="$scratch" python3 -I -B "$root/scripts/fakes/prereq/epel-urllib-interruption-test.py" > "$test_dir/urllib-interruption-result" 2>&1; then
    printf '%s\n' 'FAIL IP-urllib-interrupt: interrupted response must retry and clean up' >&2
    cat "$test_dir/urllib-interruption-result" >&2
    exit 1
  fi
  if [ "$prereq_screen_kind" = urllib_interrupt ]; then printf 'SCREEN %s: survived\n' "$PREREQ_SCREEN_MUTANT"; exit 0; fi
fi
if [ "$prereq_screen_kind" = linger ]; then
  TMPDIR=$scratch sh "$root/scripts/fakes/prereq/linger-fallback-test.sh" "$original_include"
  printf 'SCREEN %s: survived\n' "$PREREQ_SCREEN_MUTANT"
  exit 0
fi
if [ -z "$prereq_screen_case" ]; then
TMPDIR=$scratch sh "$root/scripts/fakes/prereq/hale-path-test.sh" "$original_include"
TMPDIR=$scratch sh "$root/scripts/fakes/prereq/hale-umask-test.sh" "$original_include"
TMPDIR=$scratch sh "$root/scripts/fakes/prereq/sam-readonly-test.sh" "$root"
TMPDIR=$scratch sh "$root/scripts/fakes/prereq/linger-fallback-test.sh" "$original_include"
TMPDIR=$scratch sh "$root/scripts/fakes/prereq/username-options-test.sh" "$original_include"
TMPDIR=$scratch sh "$root/scripts/fakes/prereq/root-prefix-test.sh" "$original_include"
TMPDIR=$scratch sh "$root/scripts/fakes/prereq/errexit-test.sh" "$original_include"
TMPDIR=$scratch sh "$root/scripts/fakes/prereq/openssl-env-test.sh" "$original_include"
TMPDIR=$scratch sh "$root/scripts/fakes/prereq/plan-header-test.sh" "$original_include"
TMPDIR=$scratch sh "$root/scripts/fakes/prereq/plan-output-test.sh" "$original_include"
TMPDIR=$scratch sh "$root/scripts/fakes/prereq/output-failure-test.sh" "$original_include"
TMPDIR=$scratch sh "$root/scripts/fakes/prereq/refusal-output-test.sh" "$original_include"
TMPDIR=$scratch sh "$root/scripts/fakes/prereq/machine-refusal-output-test.sh" "$original_include"
TMPDIR=$scratch sh "$root/scripts/fakes/prereq/run-machine-status-test.sh" "$original_include"
TMPDIR=$scratch sh "$root/scripts/fakes/prereq/requal-output-cells.sh" "$original_include"
TMPDIR=$scratch sh "$root/scripts/fakes/prereq/requal-extract-output-test.sh" "$original_include"
TMPDIR=$scratch sh "$root/scripts/fakes/prereq/key-record-output-test.sh" "$original_include"
TMPDIR=$scratch sh "$root/scripts/fakes/prereq/requal-provider-output-test.sh" "$original_include"
TMPDIR=$scratch sh "$root/scripts/fakes/prereq/rocky-output-failure-test.sh" "$original_include"
TMPDIR=$scratch python3 -I -B "$root/scripts/fakes/prereq/output-lint-test.py"
TMPDIR=$scratch python3 -I -B "$root/scripts/fakes/prereq/output-lint.py" "$original_include" "$root/scripts/fakes/prereq/output-best-effort.txt"
TMPDIR=$scratch python3 -I "$root/scripts/fakes/prereq/mutant-locale-test.py" "$root"
TMPDIR=$scratch python3 -I -B "$root/scripts/fakes/prereq/child-locale-test.py"
TMPDIR=$scratch python3 -I -B "$root/scripts/fakes/prereq/inventory-test.py" "$root"
TMPDIR=$scratch python3 -I -B "$root/scripts/fakes/prereq/protocol-test.py" "$root"
TMPDIR=$scratch python3 -I -B "$root/scripts/fakes/prereq/fresh-resume-test.py" "$root"
TMPDIR=$scratch python3 -I -B "$root/scripts/fakes/prereq/secret-output-test.py" "$root" "$original_include"
TMPDIR=$scratch sh "$root/scripts/fakes/prereq/package-state-test.sh" "$root"
TMPDIR=$scratch python3 -I "$root/scripts/fakes/prereq/public-pty-test.py" --self-check "$root"

fi

# No host package, container, service-manager or network program is reachable.
mkdir -p "$test_dir/bin" "$test_dir/state" "$test_dir/install-deploy" "$test_dir/tree/etc" \
  "$test_dir/tree/sys/fs/cgroup" "$test_dir/tree/proc/net" "$test_dir/home/graph" "$test_dir/token-bin"
for safe_cmd in sh awk cat chmod cp cut dirname find grep gzip head ln mkdir mktemp mv python3 readlink rm sed sha256sum sort ssh-keygen stat tar tee tr wc; do
  safe_path=$(command -v "$safe_cmd")
  ln -s "$safe_path" "$test_dir/bin/$safe_cmd"
done
# Substitute the fixed system tool paths only in this private test copy. The
# production include has no override, and no real sudo or package tool runs.
sed -e "s|/usr/bin:/usr/sbin|$test_dir/bin|g" -e "s|/usr/bin/sudo|$test_dir/bin/sudo|g" \
  -e 's|^prereq_system_root() { :; }$|prereq_system_root() { printf "%s" "${PREREQ_ROOT:-}"; }|' \
  -e 's#GCONV_PATH PREREQ_ROOT || return 1#GCONV_PATH || return 1#' "$include" > "$test_dir/fake-include.sh"
if ! grep -Fxq "  PATH=$test_dir/bin" "$test_dir/fake-include.sh" ||
   ! grep -Fxq 'prereq_system_root() { printf "%s" "${PREREQ_ROOT:-}"; }' "$test_dir/fake-include.sh" ||
   grep -Fq '/usr/bin/sudo' "$test_dir/fake-include.sh"; then
  printf '%s\n' 'FAIL F-6: isolated fake-only substitutions did not apply' >&2; exit 1
fi
include=$test_dir/fake-include.sh
cp "$root/deploy/install.sh" "$test_dir/install-deploy/install.sh"
cp "$include" "$test_dir/install-deploy/install-prereq.sh"
ln -s "$(command -v sh)" "$test_dir/token-bin/sh"
ln -s "$root/scripts/fakes/prereq/fake-command" "$test_dir/token-bin/cat"
printf '%s\n' 'ip50-token-contents-must-not-appear' > "$test_dir/token"
chmod 000 "$test_dir/token"
for fake_cmd in id uname df ss timedatectl loginctl getenforce podman podman-compose dnf sudo usermod systemctl systemd-run systemd-analyze rpm rpmkeys rpm2cpio gpg gpgv curl wget pip pip3 setenforce semanage chcon restorecon; do
  ln -s "$root/scripts/fakes/prereq/fake-command" "$test_dir/bin/$fake_cmd"
done
printf 'cortexuser:100000:65536\n' > "$test_dir/tree/etc/subuid"
printf 'cortexuser:100000:65536\n' > "$test_dir/tree/etc/subgid"
: > "$test_dir/tree/sys/fs/cgroup/cgroup.controllers"
: > "$test_dir/log"
if ! PATH="$test_dir/bin" FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" \
  "$test_dir/bin/sh" -c 'before=$-; CORTEX_YES=sentinel; . "$1"; [ "$before" = "$-" ] && [ "$CORTEX_YES" = sentinel ]' \
  sh "$include" > "$test_dir/out" 2>&1 || [ -s "$test_dir/out" ] || [ -s "$test_dir/log" ]; then
  printf '%s\n' 'FAIL IP-60: sourcing changed options, printed output or ran a command' >&2
  exit 1
fi
printf '%s\n' 'ok IP-60 sourcing is quiet and leaves shell options alone'
if [ "$prereq_screen_kind" = source ]; then printf 'SCREEN %s: survived\n' "$PREREQ_SCREEN_MUTANT"; exit 0; fi
if ! sh -n "$original_include" || { command -v dash >/dev/null 2>&1 && ! dash -n "$original_include"; } ||
   { command -v bash >/dev/null 2>&1 && ! bash --posix -n "$original_include"; }; then
  printf '%s\n' 'FAIL IP-62: prerequisite include is not valid POSIX shell' >&2
  exit 1
fi
if LC_ALL=C grep -Eq '\[[^]]*[[:alnum:]]-[[:alnum:]][^]]*\]' "$original_include"; then
  printf '%s\n' 'FAIL IP-61: prerequisite include contains a locale-sensitive character range' >&2
  exit 1
fi
printf '%s\n' 'ok IP-61/62 include syntax and character classes'
mkdir -m 700 "$test_dir/home-safe" "$test_dir/home-writable"
chmod 777 "$test_dir/home-writable"
ln -s "$test_dir/home-safe" "$test_dir/home-link"
for home_case in safe writable link; do
  home_status=1; [ "$home_case" != safe ] || home_status=0
  actual=0
  env -i PATH="$test_dir/bin" HOME="$test_dir/home-$home_case" FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" \
    "$test_dir/bin/sh" -c '. "$1"; prereq_home_safe' sh "$include" > "$test_dir/out" 2>&1 || actual=$?
  if [ "$actual" != "$home_status" ]; then printf 'FAIL W1-73: %s staging parent returned %s, expected %s\n' "$home_case" "$actual" "$home_status" >&2; exit 1; fi
done
rm "$test_dir/home-link"
chmod 700 "$test_dir/home-writable"
rm -rf "$test_dir/home-safe" "$test_dir/home-writable"
printf '%s\n' 'ok W1-73 staging parent is owned and not replaceable'
if [ -z "$prereq_screen_case" ] || [ "$prereq_screen_kind" = signal ]; then
  python3 "$root/scripts/fakes/prereq/signal-test.py" "$include" "$test_dir/bin/sh" "$test_dir/bin" "$test_dir" "$prereq_screen_locale"
fi
if [ "$prereq_screen_kind" = signal ]; then printf 'SCREEN %s: survived\n' "$PREREQ_SCREEN_MUTANT"; exit 0; fi
if [ -z "$prereq_screen_case" ] || [ "$prereq_screen_kind" = consent ]; then
  python3 "$root/scripts/fakes/prereq/consent-test.py" "$include" "$test_dir/tree" "$test_dir/bin/sh" "$test_dir/bin" "$prereq_screen_locale"
fi
if [ "$prereq_screen_kind" = consent ]; then printf 'SCREEN %s: survived\n' "$PREREQ_SCREEN_MUTANT"; exit 0; fi
LC_ALL=C awk '
  /^prereq_msg\(\) \{/ { table = 1; next }
  table && /^}/ { table = 0 }
  table && /^    [abcdefghijklmnopqrstuvwxyz0123456789_]+\)/ { id = $1; sub(/\).*/, "", id); print id }
' "$include" | LC_ALL=C sort -u > "$test_dir/messages"
LC_ALL=C awk '
  {
    line = $0
    while (match(line, /prereq_msg [abcdefghijklmnopqrstuvwxyz0123456789_]+/)) {
      id = substr(line, RSTART, RLENGTH); sub(/^prereq_msg /, "", id); print id
      line = substr(line, RSTART + RLENGTH)
    }
  }
' "$include" | LC_ALL=C sort -u > "$test_dir/message-uses"
prereq_test_os_cases() {
  # IP-03 and IP-04: os-release is parsed as data, never sourced or evaluated.
  for test_case in rocky10 rocky10quoted rocky10single rocky9 rocky100 alma10 rhel10 ubuntu empty missing truncated malformed injected backticks duplicate; do
    if [ -n "$prereq_screen_case" ] && [ "$test_case" != "$prereq_screen_case" ]; then continue; fi
    case $test_case in
      rocky10) printf 'ID=rocky\nVERSION_ID=10.2\n' > "$test_dir/os-release"; expect=0 ;;
      rocky10quoted) printf 'ID="rocky"\nVERSION_ID="10.0"\n' > "$test_dir/os-release"; expect=0 ;;
      rocky10single) printf "ID='rocky'\nVERSION_ID='10.2'\n" > "$test_dir/os-release"; expect=0 ;;
      rocky9) printf 'ID=rocky\nVERSION_ID=9.5\n' > "$test_dir/os-release"; expect=1 ;;
      rocky100) printf 'ID=rocky\nVERSION_ID=100\n' > "$test_dir/os-release"; expect=1 ;;
      alma10) printf 'ID=almalinux\nVERSION_ID=10.0\n' > "$test_dir/os-release"; expect=1 ;;
      rhel10) printf 'ID=rhel\nVERSION_ID=10\n' > "$test_dir/os-release"; expect=1 ;;
      ubuntu) printf 'ID=ubuntu\nVERSION_ID=24.04\n' > "$test_dir/os-release"; expect=1 ;;
      empty) : > "$test_dir/os-release"; expect=1 ;;
      missing) rm -f "$test_dir/os-release"; expect=1 ;;
      truncated) printf 'ID=rocky\nVERSION_ID=' > "$test_dir/os-release"; expect=1 ;;
      malformed) printf 'ID=rocky\nVERSION_ID=10evil\n' > "$test_dir/os-release"; expect=1 ;;
      injected) printf 'ID=$(touch %s)\nVERSION_ID=10\n' "$test_dir/injected" > "$test_dir/os-release"; expect=1 ;;
      backticks) printf 'ID=`touch %s`\nVERSION_ID=10\n' "$test_dir/injected" > "$test_dir/os-release"; expect=1 ;;
      duplicate) printf 'ID=almalinux\nID=rocky\nVERSION_ID=10.2\n' > "$test_dir/os-release"; expect=1 ;;
    esac
    actual=0
    LC_ALL="$test_locale" sh -c '. "$1"; prereq_os_ok "$2"' sh "$include" "$test_dir/os-release" > "$test_dir/out" 2>&1 || actual=$?
    if [ "$actual" != "$expect" ] || [ -e "$test_dir/injected" ]; then
      printf 'FAIL IP-03/04: %s in %s returned %s (expected %s) or executed input\n' "$test_case" "$test_locale" "$actual" "$expect" >&2
      exit 1
    fi
  done
  printf 'ok IP-03/04 Rocky 10 only; os-release is data (%s)\n' "$test_locale"

}

prereq_test_machine_cases() {
  # IP-01, IP-03, IP-05, IP-06, IP-07 and IP-14 use a fake machine and PATH.
  for test_case in healthy check_provider check_missing check_no_tty os arch root cgroup multiple subuid_missing subgid_short subuid_prefix time_no time_unknown linger_off linger_no_sudo disk_home_low disk_graph_low disk_invalid disk_long disk_malformed disk_failure fs_home_nfs fs_home_cifs fs_home_fuse fs_home_ext4 fs_graph_nfs4 fs_graph_tmpfs port_busy port_substring proc_free proc_busy proc_busy6 proc_other proc_bad podman_missing podman_old gzip_missing ssh_missing curl_missing gpg_missing dnf_missing coreutils_once sudo_missing no_tty epel_missing rpm_missing selinux_permissive selinux_disabled selinux_unknown injected_provider; do
    if [ -n "$prereq_screen_case" ] && [ "$test_case" != "$prereq_screen_case" ]; then continue; fi
    printf 'ID=rocky\nVERSION_ID=10.2\n' > "$test_dir/tree/etc/os-release"
    printf 'cortexuser:100000:65536\n' > "$test_dir/tree/etc/subuid"
    printf 'cortexuser:100000:65536\n' > "$test_dir/tree/etc/subgid"
    : > "$test_dir/tree/sys/fs/cgroup/cgroup.controllers"
    printf 'sl local_address rem_address st\n' > "$test_dir/tree/proc/net/tcp"
    printf 'sl local_address rem_address st\n' > "$test_dir/tree/proc/net/tcp6"
    [ -L "$test_dir/bin/ss" ] || ln -s "$root/scripts/fakes/prereq/fake-command" "$test_dir/bin/ss"
    [ -L "$test_dir/bin/podman" ] || ln -s "$root/scripts/fakes/prereq/fake-command" "$test_dir/bin/podman"
    [ -L "$test_dir/bin/curl" ] || ln -s "$root/scripts/fakes/prereq/fake-command" "$test_dir/bin/curl"
    [ -L "$test_dir/bin/sudo" ] || ln -s "$root/scripts/fakes/prereq/fake-command" "$test_dir/bin/sudo"
    [ -L "$test_dir/bin/rpm" ] || ln -s "$root/scripts/fakes/prereq/fake-command" "$test_dir/bin/rpm"
    [ -L "$test_dir/bin/gpg" ] || ln -s "$root/scripts/fakes/prereq/fake-command" "$test_dir/bin/gpg"
    [ -L "$test_dir/bin/dnf" ] || ln -s "$root/scripts/fakes/prereq/fake-command" "$test_dir/bin/dnf"
    for safe_cmd in gzip ssh-keygen sha256sum wc; do
      [ -L "$test_dir/bin/$safe_cmd" ] || ln -s "$(command -v "$safe_cmd")" "$test_dir/bin/$safe_cmd"
    done
    test_arch=x86_64 test_uid=1000 test_time=yes test_linger=yes test_podman_version=5.8.2 test_selinux=Enforcing test_yes=1 test_check=0 test_compose=yes test_provider='' test_epel=present expect=0 expect_problem='' expect_fix='' expect_warning='' expect_missing=''
    test_home_free=21474836480 test_graph_free=21474836480 test_home_fs=xfs test_graph_fs=ext4 test_min=20 test_busy=0 test_busy_port=8601 test_df_mode=ok
    case $test_case in
      check_provider) test_check=1; expect=2; expect_problem='signature'; expect_missing='podman-compose (requalify)' ;;
      check_missing) test_check=1; test_compose=no; expect=2; expect_problem='compose'; expect_missing='podman-compose' ;;
      check_no_tty) test_check=1; test_yes=0; expect=2; expect_problem='signature' ;;
      os) printf 'ID=almalinux\nVERSION_ID=10\n' > "$test_dir/tree/etc/os-release"; expect=2; expect_problem='Rocky' ;;
      arch) test_arch=aarch64; expect=2; expect_problem='x86_64' ;;
      root) test_uid=0; expect=2; expect_problem='root' ;;
      cgroup) rm "$test_dir/tree/sys/fs/cgroup/cgroup.controllers"; expect=2; expect_problem='cgroup' ;;
      multiple) printf 'ID=ubuntu\nVERSION_ID=24\n' > "$test_dir/tree/etc/os-release"; test_arch=aarch64; test_uid=0; rm "$test_dir/tree/sys/fs/cgroup/cgroup.controllers"; expect=2 ;;
      subuid_missing) : > "$test_dir/tree/etc/subuid"; expect_fix=subuid ;;
      subgid_short) printf 'cortexuser:100000:1000\n' > "$test_dir/tree/etc/subgid"; expect_fix=subgid ;;
      subuid_prefix) printf 'cortexuser-extra:100000:65536\n' > "$test_dir/tree/etc/subuid"; expect_fix=subuid ;;
      time_no) test_time=no; expect=2; expect_problem='clock' ;;
      time_unknown) test_time=unknown; expect_warning='clock' ;;
      linger_off) test_linger=no; expect_fix=linger ;;
      disk_home_low) test_home_free=21474836479; expect=2; expect_problem='home' ;;
      disk_graph_low) test_graph_free=21474836479; expect=2; expect_problem='container' ;;
      disk_invalid) test_min=20oops; expect=2; expect_problem='disk' ;;
      disk_long) test_df_mode=long ;;
      disk_malformed) test_df_mode=malformed; expect=2; expect_problem='disk' ;;
      disk_failure) test_df_mode=fail; expect=2; expect_problem='disk' ;;
      fs_home_nfs) test_home_fs=nfs; expect=2; expect_problem='network' ;;
      fs_home_cifs) test_home_fs=cifs; expect=2; expect_problem='network' ;;
      fs_home_fuse) test_home_fs=fuse.sshfs; expect=2; expect_problem='network' ;;
      fs_home_ext4) test_home_fs=ext4 ;;
      fs_graph_nfs4) test_graph_fs=nfs4; expect=2; expect_problem='network' ;;
      fs_graph_tmpfs) test_graph_fs=tmpfs; expect=2; expect_problem='filesystem' ;;
      port_busy) test_busy=1; expect=2; expect_problem='8601' ;;
      port_substring) test_busy=1; test_busy_port=18601 ;;
      proc_free) rm "$test_dir/bin/ss" ;;
      proc_busy) rm "$test_dir/bin/ss"; printf '0: 0100007F:2199 00000000:0000 0A\n' >> "$test_dir/tree/proc/net/tcp"; expect=2; expect_problem='8601' ;;
      proc_busy6) rm "$test_dir/bin/ss"; printf '0: 00000000000000000000000000000000:2199 00000000000000000000000000000000:0000 0A\n' >> "$test_dir/tree/proc/net/tcp6"; expect=2; expect_problem='8601' ;;
      proc_other) rm "$test_dir/bin/ss"; printf '0: 0100007F:48A9 00000000:0000 0A\n' >> "$test_dir/tree/proc/net/tcp" ;;
      proc_bad) rm "$test_dir/bin/ss"; printf 'not a socket record\n' >> "$test_dir/tree/proc/net/tcp6"; expect=2; expect_problem='port' ;;
      podman_missing) rm "$test_dir/bin/podman"; expect_missing=podman ;;
      podman_old) test_podman_version=4.9.0; expect=2; expect_problem='podman' ;;
      gzip_missing) rm "$test_dir/bin/gzip"; expect_missing=gzip ;;
      ssh_missing) rm "$test_dir/bin/ssh-keygen"; expect_missing=openssh ;;
      curl_missing) rm "$test_dir/bin/curl"; expect_missing=curl ;;
      gpg_missing) rm "$test_dir/bin/gpg"; expect_missing=gnupg2 ;;
      dnf_missing) rm "$test_dir/bin/dnf"; expect=2; expect_problem='package manager' ;;
      coreutils_once) rm "$test_dir/bin/sha256sum" "$test_dir/bin/wc"; expect_missing=coreutils ;;
      sudo_missing) rm "$test_dir/bin/sudo" "$test_dir/bin/gzip"; expect=2; expect_problem='sudo'; expect_missing=gzip ;;
      linger_no_sudo) rm "$test_dir/bin/sudo"; test_linger=no; expect=2; expect_problem='sudo'; expect_fix=linger ;;
      no_tty) rm "$test_dir/bin/gzip"; test_yes=0; expect=2; expect_problem='terminal'; expect_missing=gzip ;;
      epel_missing) test_epel=missing; expect_fix=epel ;;
      rpm_missing) rm "$test_dir/bin/rpm"; expect=2; expect_problem='RPM' ;;
      selinux_permissive) test_selinux=Permissive ;;
      selinux_disabled) test_selinux=Disabled ;;
      selinux_unknown) test_selinux='untrusted output'; test_selinux_expected=unknown ;;
      injected_provider) test_provider="$test_dir/bin/podman-compose" ;;
    esac
    test_selinux_expected=${test_selinux_expected:-$test_selinux}
    : > "$test_dir/log"
    python3 -I -B "$root/scripts/fakes/prereq/filesystem-inventory.py" "$test_dir/tree" "$test_dir/state" "$test_dir/home" > "$test_dir/before"
    actual=0
    env -i LC_ALL="$test_locale" PATH="$test_dir/bin" FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" \
      FAKE_ARCH="$test_arch" FAKE_UID="$test_uid" FAKE_GRAPH_ROOT="$test_dir/home/graph" \
      FAKE_TIME_SYNC="$test_time" FAKE_LINGER="$test_linger" \
      FAKE_PODMAN_VERSION="$test_podman_version" \
      FAKE_SELINUX="$test_selinux" \
      FAKE_EPEL="$test_epel" \
      FAKE_COMPOSE_INSTALLED="$test_compose" \
      FAKE_HOME_FREE="$test_home_free" FAKE_GRAPH_FREE="$test_graph_free" FAKE_HOME_FS="$test_home_fs" FAKE_GRAPH_FS="$test_graph_fs" \
      FAKE_DF_MODE="$test_df_mode" \
      FAKE_PORT_BUSY="$test_busy" FAKE_BUSY_PORT="$test_busy_port" CORTEX_MIN_FREE_GIB="$test_min" \
      TEST_TOKEN='ip54-secret-token-marker' TEST_PASSWORD='ip54-secret-password-marker' \
      TEST_KEY='ip54-secret-key-marker' TEST_HEX='0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef' \
      PREREQ_COMPOSE="$test_provider" PREREQ_COMPOSE_VERIFIED=1 \
      CORTEX_YES="$test_yes" check_only="$test_check" \
      PREREQ_ROOT="$test_dir/tree" HOME="$test_dir/home" \
      "$test_dir/bin/sh" -c '. "$1"; say() { printf "%s\n" "$*"; }; warn() { printf "warning: %s\n" "$*" >&2; }; EX_MACHINE=2; EX_PREREQ=3; home_dir=$2; port=8601; rc=0; machine_look || rc=$?; printf "TEST_FIXES=%s\nTEST_MISSING=%s\nTEST_COMPOSE=%s\n" "$PREREQ_FIXES" "$PREREQ_MISSING" "$PREREQ_COMPOSE"; exit "$rc"' \
      sh "$include" "$test_dir/home" > "$test_dir/out" 2>&1 || actual=$?
    python3 -I -B "$root/scripts/fakes/prereq/filesystem-inventory.py" "$test_dir/tree" "$test_dir/state" "$test_dir/home" > "$test_dir/after"
    if ! cmp "$test_dir/before" "$test_dir/after"; then
      printf 'FAIL IP-02: machine look changed the fake tree in %s\n' "$test_case" >&2
      exit 1
    fi
    if grep -Eq 'ip54-secret-|0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef' "$test_dir/out" "$test_dir/log"; then
      printf 'FAIL IP-54: machine look leaked an environment marker in %s\n' "$test_case" >&2
      exit 1
    fi
    if [ "$actual" != "$expect" ]; then
      printf 'FAIL IP-machine: %s in %s returned %s (expected %s)\n' "$test_case" "$test_locale" "$actual" "$expect" >&2
      exit 1
    fi
    if [ -n "$expect_problem" ] && ! grep -qi "$expect_problem" "$test_dir/out"; then
      printf 'FAIL IP-machine: %s lacks its named cause\n' "$test_case" >&2
      exit 1
    fi
    if [ -n "$expect_fix" ] && ! grep -Eq "^TEST_FIXES=.*${expect_fix}" "$test_dir/out"; then
      printf 'FAIL IP-machine: %s was not planned for correction\n' "$test_case" >&2
      exit 1
    fi
    if [ -n "$expect_missing" ] && ! awk -v wanted="$expect_missing" '
      /^TEST_MISSING=/ { capturing = 1; sub(/^TEST_MISSING=/, "") }
      /^TEST_COMPOSE=/ { capturing = 0 }
      capturing && $0 == wanted { found = 1 }
      END { if (!found) exit 1 }
    ' "$test_dir/out"; then
      printf 'FAIL IP-15: %s was not listed as missing\n' "$expect_missing" >&2
      exit 1
    fi
    if [ "$test_case" = coreutils_once ] && [ "$(grep -c '^coreutils$' "$test_dir/out")" != 1 ]; then
      printf '%s\n' 'FAIL IP-15: a package shared by missing commands must be listed once' >&2
      exit 1
    fi
    if [ -n "$expect_warning" ] && ! grep -qi "warning:.*${expect_warning}" "$test_dir/out"; then
      printf 'FAIL IP-09: %s lacks its warning\n' "$test_case" >&2
      exit 1
    fi
    if [ "$test_case" = healthy ] && [ "$(grep -c '^problem:' "$test_dir/out" || :)" != 0 ]; then
      printf '%s\n' 'FAIL IP-01: an otherwise ready machine must report no problem' >&2
      exit 1
    fi
    if [ "$test_case" = healthy ] && ! grep -q '^TEST_MISSING=podman-compose (requalify)$' "$test_dir/out"; then
      printf '%s\n' 'FAIL IP-01: the installed provider must be listed for requalification' >&2
      exit 1
    fi
    if [ "$test_case" = injected_provider ] && ! grep -q '^TEST_COMPOSE=$' "$test_dir/out"; then
      printf '%s\n' 'FAIL IP-compose-trust: machine look retained an unverified injected path' >&2
      exit 1
    fi
    if [ "$test_case" = check_no_tty ] && grep -qi 'terminal' "$test_dir/out"; then
      printf '%s\n' 'FAIL IP-check: read-only --check must not require a terminal' >&2
      exit 1
    fi
    if [ "$(grep -c "^SELinux: ${test_selinux_expected}$" "$test_dir/out" || :)" != 1 ]; then
      printf 'FAIL IP-13: %s did not report SELinux exactly once\n' "$test_case" >&2
      exit 1
    fi
    test_selinux_expected=''
    if [ "$test_case" = multiple ] && [ "$(grep -c '^problem:' "$test_dir/out")" -lt 4 ]; then
      printf '%s\n' 'FAIL IP-14: a machine look must report all four problems' >&2
      exit 1
    fi
    if [ "$test_case" = disk_long ] && ! grep -q '^df <-P>' "$test_dir/log"; then
      printf '%s\n' 'FAIL IP-10: disk check did not request unwrapped POSIX output' >&2
      exit 1
    fi
    case $test_case in
      os|arch|multiple)
        if grep -q '^rpm ' "$test_dir/log"; then
          printf '%s\n' 'FAIL IP-03/05: EPEL was inspected before Rocky 10 and x86_64 were confirmed' >&2
          exit 1
        fi ;;
    esac
    if grep -Eq '^(dnf|sudo|usermod|systemctl|systemd-run|systemd-analyze|gpg|curl|wget|pip|pip3|podman-compose|setenforce|semanage|chcon|restorecon)( |$)' "$test_dir/log"; then
      printf '%s\n' 'FAIL IP-02: machine look tried a mutation or network tool' >&2
      exit 1
    fi
    if ! sh "$root/scripts/fakes/prereq/check-readonly-log.sh" "$test_dir/log"; then
      printf '%s\n' 'FAIL SAM-02: machine look requested a mutating loginctl or podman operation' >&2
      exit 1
    fi
  done
  printf 'ok IP-01/03/05/06/07/14 fake machine checks (%s)\n' "$test_locale"

}

prereq_test_range_cases() {
  # IP-35: allocate after every existing UID and GID range, including the
  # last unterminated record; malformed or exhausted data cannot be guessed.
  for range_case in occupied empty malformed exhausted; do
    if [ -n "$prereq_screen_case" ] && [ "$range_case" != "$prereq_screen_case" ]; then continue; fi
    case $range_case in
      occupied)
        printf 'alpha:100000:65536\nbeta:300000:65536' > "$test_dir/tree/etc/subuid"
        printf 'gamma:500000:65536\n' > "$test_dir/tree/etc/subgid"
        expected_range=565536-631071; expected_status=0 ;;
      empty)
        : > "$test_dir/tree/etc/subuid"; : > "$test_dir/tree/etc/subgid"
        expected_range=100000-165535; expected_status=0 ;;
      malformed)
        printf 'alpha:100000:bad\n' > "$test_dir/tree/etc/subuid"
        : > "$test_dir/tree/etc/subgid"; expected_range=''; expected_status=1 ;;
      exhausted)
        printf 'alpha:4294967000:296\n' > "$test_dir/tree/etc/subuid"
        : > "$test_dir/tree/etc/subgid"; expected_range=''; expected_status=1 ;;
    esac
    actual=0
    LC_ALL="$test_locale" PATH="$test_dir/bin" "$test_dir/bin/sh" -c \
      '. "$1"; prereq_subid_range "$2" "$3"' sh "$include" \
      "$test_dir/tree/etc/subuid" "$test_dir/tree/etc/subgid" > "$test_dir/range" 2>/dev/null || actual=$?
    if [ "$actual" != "$expected_status" ] || [ "$(cat "$test_dir/range")" != "$expected_range" ]; then
      printf 'FAIL IP-35: %s allocation returned status %s and range %s (%s)\n' \
        "$range_case" "$actual" "$(cat "$test_dir/range")" "$test_locale" >&2
      exit 1
    fi
  done
  printf 'ok IP-35 non-overlapping subordinate-ID allocation (%s)\n' "$test_locale"
}

prereq_test_repair_cases() {
  : > "$test_dir/tree/.fake-root"
  for subid_case in both uid gid stale sudo_fail noop noop_existing; do
    if [ -n "$prereq_screen_case" ] && [ "$subid_case" != "$prereq_screen_case" ]; then continue; fi
    printf 'other:100000:65536\n' > "$test_dir/tree/etc/subuid"
    printf 'other:200000:65536\n' > "$test_dir/tree/etc/subgid"
    : > "$test_dir/log"
    subid_uid=1 subid_gid=1 subid_range=265536-331071 subid_sudo_fail=0 subid_noop=0 expected_status=0
    case $subid_case in
      uid) subid_gid=0 ;;
      gid) subid_uid=0 ;;
      stale) subid_range=100000-165535; expected_status=3 ;;
      sudo_fail) subid_sudo_fail=1; expected_status=3 ;;
      noop) subid_noop=1; expected_status=3 ;;
      noop_existing) printf 'cortexuser:100000:65536\n' > "$test_dir/tree/etc/subuid"; subid_noop=1; subid_gid=0; expected_status=3 ;;
    esac
    actual=0
    env -i LC_ALL="$test_locale" PATH="$test_dir/bin" FAKE_BIN="$test_dir/bin" FAKE_ROOT="$test_dir/tree" \
      FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" FAKE_ALLOW_SUBIDS=1 \
      FAKE_SUDO_FAIL="$subid_sudo_fail" FAKE_USERMOD_NOOP="$subid_noop" PREREQ_ROOT="$test_dir/tree" \
      "$test_dir/bin/sh" -c '. "$1"; EX_PREREQ=3; prereq_fix_subids cortexuser "$2" "$3" "$4"' \
      sh "$include" "$subid_range" "$subid_uid" "$subid_gid" > "$test_dir/out" 2>&1 || actual=$?
    if [ "$actual" != "$expected_status" ]; then
      printf 'FAIL IP-35: %s repair returned %s, expected %s (%s)\n' "$subid_case" "$actual" "$expected_status" "$test_locale" >&2
      exit 1
    fi
    if [ "$subid_case" = stale ] && [ -s "$test_dir/log" ]; then
      printf '%s\n' 'FAIL IP-35: stale range caused a privileged call' >&2; exit 1
    fi
    if [ "$subid_case" = both ] && ! grep -Fxq 'sudo <usermod> <--add-subuids> <265536-331071> <--add-subgids> <265536-331071> <--> <cortexuser>' "$test_dir/log"; then
      printf '%s\n' 'FAIL IP-35: repair did not apply the planned range exactly' >&2; exit 1
    fi
  done
  printf 'ok IP-35 subordinate-ID repair checks the plan and result (%s)\n' "$test_locale"
}

prereq_test_subid_cases() {
  # IP-08: the complete account name and a sufficient numeric range are required.
  for test_case in valid exact_top overflow other prefix short bad_count malformed; do
    if [ -n "$prereq_screen_case" ] && [ "$test_case" != "$prereq_screen_case" ]; then continue; fi
    case $test_case in
      valid) printf 'cortexuser:100000:65536\n' > "$test_dir/subids"; expect=0 ;;
      exact_top) printf 'cortexuser:4294901760:65536\n' > "$test_dir/subids"; expect=0 ;;
      overflow) printf 'cortexuser:4294901761:65536\n' > "$test_dir/subids"; expect=1 ;;
      other) printf 'other:100000:65536\n' > "$test_dir/subids"; expect=1 ;;
      prefix) printf 'cortexuser-extra:100000:65536\n' > "$test_dir/subids"; expect=1 ;;
      short) printf 'cortexuser:100000:65535\n' > "$test_dir/subids"; expect=1 ;;
      bad_count) printf 'cortexuser:100000:65536oops\n' > "$test_dir/subids"; expect=1 ;;
      malformed) printf 'cortexuser:not-a-number:65536\n' > "$test_dir/subids"; expect=1 ;;
    esac
    actual=0
    LC_ALL="$test_locale" PATH="$test_dir/bin" "$test_dir/bin/sh" -c '. "$1"; prereq_subid_ok cortexuser "$2"' sh "$include" "$test_dir/subids" > "$test_dir/out" 2>&1 || actual=$?
    if [ "$actual" != "$expect" ]; then
      printf 'FAIL IP-08: %s in %s returned %s (expected %s)\n' "$test_case" "$test_locale" "$actual" "$expect" >&2
      exit 1
    fi
  done
  printf 'ok IP-08 subordinate ranges match the whole user (%s)\n' "$test_locale"

}

prereq_test_version_cases() {
  # Q-I5: only a complete numeric installed provider version can meet 1.5.0.
  for version_case in '1.5.0:0' '1.5:0' '1.6.0:0' '2.0.0:0' '1.4.9:1' '0.99.0:1' '1.5.0rc:1' '1.5.0.1:1' '1..5:1' 'bogus:1'; do
    if [ -n "$prereq_screen_case" ] && [ "$version_case" != "$prereq_screen_case" ]; then continue; fi
    test_version=${version_case%:*} expect=${version_case#*:} actual=0
    LC_ALL="$test_locale" PATH="$test_dir/bin" "$test_dir/bin/sh" -c '. "$1"; prereq_compose_version_ok "$2"' sh "$include" "$test_version" > "$test_dir/out" 2>&1 || actual=$?
    if [ "$actual" != "$expect" ]; then
      printf 'FAIL IP-compose-version: %s in %s returned %s (expected %s)\n' "$test_version" "$test_locale" "$actual" "$expect" >&2
      exit 1
    fi
  done
  printf 'ok IP-compose-version complete numeric 1.5.0 boundary (%s)\n' "$test_locale"

}

prereq_test_message_cases() {
  for test_message in $(cat "$test_dir/messages"); do
    if [ -n "$prereq_screen_case" ] && [ "$test_message" != "$prereq_screen_case" ]; then continue; fi
    if ! LC_ALL="$test_locale" sh -c '. "$1"; prereq_msg "$2" "secret-marker-unsafe"' sh "$include" "$test_message" > "$test_dir/out" 2>&1; then
      printf 'FAIL IP-53: message %s was not recognized\n' "$test_message" >&2
      exit 1
    fi
    if ! LC_ALL="$test_locale" awk '
      NR == 1 { if ($0 !~ /^problem: .+/) exit 1; cause = substr($0, 10); next }
      NR == 2 { if ($0 !~ /^fix: .+/ || substr($0, 6) == cause) exit 1; next }
      { exit 1 }
      END { if (NR != 2) exit 1 }
    ' "$test_dir/out" || grep -q 'secret-marker-unsafe' "$test_dir/out"; then
      printf 'FAIL IP-53/54: message %s lacks two safe distinct lines\n' "$test_message" >&2
      exit 1
    fi
  done
}

prereq_test_provider_cases() {
  # An injected path is not proof of a verified RPM-owned Compose provider.
  : > "$test_dir/log"
  actual=0
  LC_ALL="$test_locale" PATH="$test_dir/token-bin" FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" \
    PREREQ_COMPOSE="$test_dir/bin/podman-compose" PREREQ_COMPOSE_VERIFIED=1 \
    "$test_dir/token-bin/sh" -c '. "$1"; prereq_compose_path' sh "$include" > "$test_dir/out" 2>&1 || actual=$?
  if [ "$actual" != 1 ] || [ -s "$test_dir/out" ] || [ -s "$test_dir/log" ]; then
    printf 'FAIL IP-compose-trust: unverified provider was not silently refused (status %s)\n' "$actual" >&2
    exit 1
  fi
  printf 'ok IP-compose-trust unverified provider is refused (%s)\n' "$test_locale"

}

prereq_test_completion_cases() {
  # IP-50/51: the completion message names the token file but never reads it.
  : > "$test_dir/log"
  if ! LC_ALL="$test_locale" PATH="$test_dir/token-bin" FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" \
    "$test_dir/token-bin/sh" -c '. "$1"; say() { printf "%s\n" "$*"; }; EX_USAGE=1; say_done 0.2.1 https://localhost:8601 "$2" "$3"' \
    sh "$include" "$test_dir/token" "$test_dir/install.log" > "$test_dir/out" 2>&1; then
    printf '%s\n' 'FAIL IP-50: completion message failed' >&2
    exit 1
  fi
  if ! grep -qF '0.2.1' "$test_dir/out" || ! grep -qF 'https://localhost:8601' "$test_dir/out" ||
     ! grep -qF "$test_dir/token" "$test_dir/out" || ! grep -qF "$test_dir/install.log" "$test_dir/out" ||
     ! grep -qi 'next' "$test_dir/out" || grep -q 'ip50-token-contents' "$test_dir/out" || [ -s "$test_dir/log" ]; then
    printf '%s\n' 'FAIL IP-50: completion omitted a required value or read the token' >&2
    exit 1
  fi
  actual=0
  LC_ALL="$test_locale" PATH="$test_dir/token-bin" FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" \
    "$test_dir/token-bin/sh" -c '. "$1"; say() { printf "%s\n" "$*"; }; EX_USAGE=1; say_done "" https://localhost "$2" "$3"' \
    sh "$include" "$test_dir/token" "$test_dir/install.log" > "$test_dir/out" 2>&1 || actual=$?
  if [ "$actual" != 1 ] || ! grep -qi 'usage' "$test_dir/out"; then
    printf '%s\n' 'FAIL IP-51: missing completion input was not refused with usage' >&2
    exit 1
  fi
  printf 'ok IP-50/51 completion never opens the token (%s)\n' "$test_locale"

}

prereq_test_key_cases() {
  for key_case in valid short foreign multiple fail; do
    if [ -n "$prereq_screen_case" ] && [ "$key_case" != "$prereq_screen_case" ]; then continue; fi
    : > "$test_dir/log"
    actual=0; expected_status=1
    [ "$key_case" != valid ] || expected_status=0
    env -i LC_ALL="$test_locale" PATH="$test_dir/bin" FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" \
      FAKE_ALLOW_KEY_READ=1 FAKE_KEY_CASE="$key_case" "$test_dir/bin/sh" -c \
      '. "$1"; prereq_epel_key_ok "$2" "$3"' sh "$include" "$test_dir/state" "$test_dir/fake-key" \
      > "$test_dir/out" 2>&1 || actual=$?
    if [ "$actual" != "$expected_status" ]; then
      printf 'FAIL IP-compose-trust: %s key returned %s, expected %s (%s)\n' "$key_case" "$actual" "$expected_status" "$test_locale" >&2; exit 1
    fi
  done
  printf 'ok IP-compose-trust full primary fingerprint and sole certificate (%s)\n' "$test_locale"
}

prereq_test_signature_cases() {
  mkdir -p "$test_dir/empty-ring" "$test_dir/pinned-ring" "$test_dir/gpg-home"
  : > "$test_dir/pinned-ring/certificate.key"
  for signature_case in valid digest_only header_only unsigned bad wrong_key isolation_broken; do
    if [ -n "$prereq_screen_case" ] && [ "$signature_case" != "$prereq_screen_case" ]; then continue; fi
    : > "$test_dir/log"
    actual=0; expected_status=1
    [ "$signature_case" != valid ] || expected_status=0
    env -i LC_ALL="$test_locale" PATH="$test_dir/bin" FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" \
      FAKE_ALLOW_KEY_READ=1 FAKE_ALLOW_RPMKEYS=1 FAKE_SIGNATURE_CASE="$signature_case" \
      "$test_dir/bin/sh" -c '. "$1"; prereq_rpm_signature_ok "$2" "$3" "$4" "$5"' \
      sh "$include" "$test_dir/gpg-home" "$test_dir/empty-ring" "$test_dir/pinned-ring" "$test_dir/archive.rpm" \
      > "$test_dir/out" 2>&1 || actual=$?
    if [ "$actual" != "$expected_status" ]; then
      printf 'FAIL IP-compose-trust: %s signature returned %s, expected %s (%s)\n' "$signature_case" "$actual" "$expected_status" "$test_locale" >&2; exit 1
    fi
  done
  printf 'ok IP-compose-trust RPM signature and isolated keyring (%s)\n' "$test_locale"
}

prereq_test_payload_cases() {
  mkdir -p "$test_dir/provider-bin"
  printf '%s\n' '#!/bin/sh' 'exit 95' > "$test_dir/provider-bin/podman-compose"
  chmod 755 "$test_dir/provider-bin/podman-compose"
  ln -s "$test_dir/provider-bin/podman-compose" "$test_dir/provider-bin/link"
  for payload_case in valid epoch arch owner old mismatch noisy symlink ghost skip_digest missing_module; do
    if [ -n "$prereq_screen_case" ] && [ "$payload_case" != "$prereq_screen_case" ]; then continue; fi
    : > "$test_dir/log"
    actual=0; expected_status=1; provider="$test_dir/provider-bin/podman-compose"
    [ "$payload_case" != valid ] || expected_status=0
    [ "$payload_case" != symlink ] || provider="$test_dir/provider-bin/link"
    env -i LC_ALL="$test_locale" PATH="$test_dir/bin" FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" \
      FAKE_ALLOW_PAYLOAD=1 FAKE_PAYLOAD_CASE="$payload_case" FAKE_PROVIDER="$provider" "$test_dir/bin/sh" -c \
      '. "$1"; prereq_rpm_payload_ok "$2" "$3"' sh "$include" "$test_dir/archive.rpm" "$provider" \
      > "$test_dir/out" 2>&1 || actual=$?
    if [ "$actual" != "$expected_status" ]; then
      printf 'FAIL IP-compose-trust: %s installed payload returned %s, expected %s (%s)\n' "$payload_case" "$actual" "$expected_status" "$test_locale" >&2; exit 1
    fi
    if grep -q '^podman-compose ' "$test_dir/log"; then
      printf '%s\n' 'FAIL IP-compose-trust: an untrusted provider was executed' >&2; exit 1
    fi
  done
  rm "$test_dir/provider-bin/link"
  printf 'ok IP-compose-trust exact installed identity and package verification (%s)\n' "$test_locale"
}

prereq_test_fetch_cases() {
  if [ "$prereq_screen_kind" = fetch ]; then mkdir -p "$test_dir/pinned-ring"; : > "$test_dir/pinned-ring/certificate.key"; fi
  for fetch_case in valid metadata_signed metadata_bad_sig metadata_foreign_sig metadata_missing_sig metadata_key_wrong metadata_no_tool repomd_mismatch primary_mismatch package_mismatch wrong_epoch duplicate path_escape http_mirror fail_package; do
    if [ -n "$prereq_screen_case" ] && [ "$fetch_case" != "$prereq_screen_case" ]; then continue; fi
    mkdir -p "$test_dir/fetch-$fetch_case" "$test_dir/replies-$fetch_case"
    chmod 700 "$test_dir/fetch-$fetch_case"
    python3 "$root/scripts/fakes/prereq/repo-fixtures.py" "$test_dir/replies-$fetch_case" "$fetch_case"
    printf '%s\n' "$test_dir/replies-$fetch_case" > "$test_dir/state/fetch-fixtures"
    printf '%s\n' "$fetch_case" > "$test_dir/state/fetch-case"
    : > "$test_dir/log"
    actual=0; expected_status=1; metadata_check=0; key_case=valid
    [ "$fetch_case" != valid ] || expected_status=0
    case $fetch_case in metadata_*) metadata_check=1 ;; esac
    [ "$fetch_case" != metadata_signed ] || expected_status=0
    [ "$fetch_case" != metadata_key_wrong ] || key_case=foreign
    if [ "$fetch_case" = metadata_no_tool ]; then mv "$test_dir/bin/gpgv" "$test_dir/bin/gpgv.saved"; fi
    env -i LC_ALL="$test_locale" PATH="$test_dir/bin" FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" \
      FAKE_ALLOW_FETCH=1 FAKE_ALLOW_KEY_READ=1 FAKE_KEY_CASE="$key_case" FAKE_FETCH_FIXTURES="$test_dir/replies-$fetch_case" FAKE_FETCH_CASE="$fetch_case" \
      "$test_dir/bin/sh" -c '. "$1"; prereq_epel_fetch "$2" "$3" "$4" "$5" "$6"' sh "$include" \
      'https://mirrors.fedoraproject.org/metalink?repo=epel-z-10.2&arch=x86_64' \
      'podman-compose|0|1.5.0|1.el10_1|noarch' "$test_dir/fetch-$fetch_case" "$metadata_check" "$test_dir/pinned-ring/certificate.key" \
      > "$test_dir/out" 2>&1 || actual=$?
    if [ "$fetch_case" = metadata_no_tool ]; then mv "$test_dir/bin/gpgv.saved" "$test_dir/bin/gpgv"; fi
    if [ "$actual" != "$expected_status" ]; then
      printf 'FAIL IP-compose-trust: %s metadata fetch returned %s, expected %s (%s)\n' "$fetch_case" "$actual" "$expected_status" "$test_locale" >&2; exit 1
    fi
    if [ "$fetch_case" = metadata_no_tool ] && { ! grep -q 'provision trusted GnuPG' "$test_dir/out" || grep -q '^curl ' "$test_dir/log"; }; then
      printf '%s\n' 'FAIL W1-73: EPEL metadata verifier missing without an actionable refusal' >&2; exit 1
    fi
    if [ "$expected_status" = 0 ] && { [ ! -s "$test_dir/fetch-$fetch_case/compose.rpm" ] || [ ! -s "$test_dir/fetch-$fetch_case/receipt.txt" ]; }; then
      printf '%s\n' 'FAIL IP-compose-trust: verified acquisition lacks archive or receipt' >&2; exit 1
    fi
    if [ "$expected_status" != 0 ] && [ -e "$test_dir/fetch-$fetch_case/compose.rpm" ]; then
      printf '%s\n' 'FAIL IP-compose-trust: failed acquisition retained an unverified archive' >&2; exit 1
    fi
  done
  rm -rf "$test_dir"/fetch-* "$test_dir"/replies-*
  printf 'ok IP-compose-trust metadata-to-archive binding (%s)\n' "$test_locale"
}

prereq_test_qualify_cases() {
  mkdir -p "$test_dir/tree/usr/bin" "$test_dir/tree/etc/pki/rpm-gpg"
  printf 'cortexuser:100000:65536\n' > "$test_dir/tree/etc/subuid"
  printf 'cortexuser:100000:65536\n' > "$test_dir/tree/etc/subgid"
  printf 'ID=rocky\nVERSION_ID=10.2\n' > "$test_dir/tree/etc/os-release"
  printf '%s\n' '#!/bin/sh' 'exit 95' > "$test_dir/tree/usr/bin/podman-compose"
  chmod 755 "$test_dir/tree/usr/bin/podman-compose"
  : > "$test_dir/tree/etc/pki/rpm-gpg/RPM-GPG-KEY-EPEL-10"
  for qualify_case in valid fd3_probe foreign_key wrong_repo bad_signature bad_payload bad_metadata; do
    if [ -n "$prereq_screen_case" ] && [ "$qualify_case" != "$prereq_screen_case" ]; then continue; fi
    mkdir -p "$test_dir/qualify-replies"
    fetch_case=valid key_case=valid repo_case=metadata_off signature_case=valid payload_case=valid fd3_probe=0
    expected_status=3
    case $qualify_case in
      valid) expected_status=0 ;;
      fd3_probe) expected_status=0; fd3_probe=1 ;;
      foreign_key) key_case=foreign ;;
      wrong_repo) repo_case=testing ;;
      bad_signature) signature_case=digest_only ;;
      bad_payload) payload_case=mismatch ;;
      bad_metadata) fetch_case=package_mismatch ;;
    esac
    python3 "$root/scripts/fakes/prereq/repo-fixtures.py" "$test_dir/qualify-replies" "$fetch_case"
    printf '%s\n' "$test_dir/qualify-replies" > "$test_dir/state/fetch-fixtures"
    printf '%s\n' "$fetch_case" > "$test_dir/state/fetch-case"
    : > "$test_dir/log"
    actual=0
    env -i LC_ALL="$test_locale" PATH="$test_dir/bin" HOME="$test_dir/home" PREREQ_ROOT="$test_dir/tree" \
      FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" FAKE_ALLOW_KEY_READ=1 FAKE_KEY_CASE="$key_case" \
      FAKE_ALLOW_DNF_CONFIG=1 FAKE_REPO_CASE="$repo_case" FAKE_ALLOW_RPMKEYS=1 FAKE_SIGNATURE_CASE="$signature_case" \
      FAKE_ALLOW_PAYLOAD=1 FAKE_PAYLOAD_CASE="$payload_case" FAKE_PROVIDER="$test_dir/tree/usr/bin/podman-compose" FAKE_PROBE_FD3="$fd3_probe" \
      FAKE_GRAPH_ROOT="$test_dir/home/graph" CORTEX_YES=1 \
      "$test_dir/bin/sh" -c '. "$1"; say() { printf "%s\n" "$*"; }; warn() { printf "warning: %s\n" "$*" >&2; }; EX_PREREQ=3; EX_MACHINE=2; home_dir=$2; port=8601; PREREQ_COMPOSE=untrusted; rc=0; if [ "$3" = valid ]; then machine_look || exit $?; fi; prereq_requalify_compose || rc=$?; printf "RESULT=%s\n" "$PREREQ_COMPOSE"; prereq_compose_path || :; exit "$rc"' \
      sh "$include" "$test_dir/home" "$qualify_case" > "$test_dir/out" 2>&1 || actual=$?
    if [ "$actual" != "$expected_status" ]; then
      printf 'FAIL IP-compose-trust: %s requalification returned %s, expected %s (%s)\n' "$qualify_case" "$actual" "$expected_status" "$test_locale" >&2; exit 1
    fi
    if [ "$qualify_case" = valid ]; then
      if grep -q '^problem:' "$test_dir/out" || ! grep -Fxq 'Compose requalification: private pinned EPEL certificate and isolated RPM keyring verified.' "$test_dir/out"; then
        printf '%s\n' 'FAIL O: successful verified Compose requalification emitted a problem or omitted its proof' >&2
        cat "$test_dir/out" >&2
        exit 1
      fi
    fi
    if [ "$expected_status" != 0 ] && { ! grep -q '^problem:' "$test_dir/out" || ! grep -q '^fix:' "$test_dir/out" || grep -q '^Compose requalification: .* verified\.$' "$test_dir/out"; }; then
      printf 'FAIL O: %s must report a fix and never claim verification\n' "$qualify_case" >&2; exit 1
    fi
    if [ "$qualify_case" = valid ] || [ "$qualify_case" = fd3_probe ]; then
      if ! grep -Fxq "RESULT=$test_dir/tree/usr/bin/podman-compose" "$test_dir/out" ||
         ! grep -Fxq "$test_dir/tree/usr/bin/podman-compose" "$test_dir/out" ||
         ! grep -q '^Verifier rpmkeys: RPM version 4.19.1.1$' "$test_dir/out" ||
         ! grep -q '^Certificate fingerprint: 7D8D15CBFC4E62688591FB2633D98517E37ED158$' "$test_dir/out" ||
         ! grep -q '^RPM verification: .*Signature.*: OK$' "$test_dir/out"; then
        printf '%s\n' 'FAIL IP-compose-trust: verified provider path was not returned' >&2; exit 1
      fi
    elif ! grep -Fxq 'RESULT=' "$test_dir/out"; then
      printf 'FAIL IP-compose-trust: %s retained an unverified provider\n' "$qualify_case" >&2; exit 1
    fi
    if grep -Eq '^(sudo|podman-compose|pip|pip3|systemctl)( |$)|^dnf <install>' "$test_dir/log"; then
      printf '%s\n' 'FAIL IP-compose-trust: requalification mutated or ran the provider' >&2; exit 1
    fi
    rm -rf "$test_dir/qualify-replies"
  done
  printf 'ok IP-compose-trust end-to-end installed provider requalification (%s)\n' "$test_locale"
}

prereq_test_candidate_cases() {
  for candidate_case in valid dependency bootstrap_in_epel changed foreign_signature wrong_nevra; do
    if [ -n "$prereq_screen_case" ] && [ "$candidate_case" != "$prereq_screen_case" ]; then continue; fi
    candidate_stage="$test_dir/epel-candidate-$candidate_case"
    mkdir -m 700 "$candidate_stage" "$test_dir/candidate-replies"
    mkdir -m 700 "$candidate_stage/packages" "$candidate_stage/gpg" "$candidate_stage/empty" "$candidate_stage/pinned"
    candidate_name=podman-compose
    [ "$candidate_case" != dependency ] || candidate_name=epel-dependency
    [ "$candidate_case" != bootstrap_in_epel ] || candidate_name=epel-release
    python3 "$root/scripts/fakes/prereq/repo-fixtures.py" "$test_dir/candidate-replies" valid "$candidate_name"
    cp "$test_dir/candidate-replies/package" "$candidate_stage/packages/compose.rpm"
    cp "$test_dir/tree/etc/pki/rpm-gpg/RPM-GPG-KEY-EPEL-10" "$candidate_stage/pinned/certificate.key"
    printf '%s\n' "$test_dir/candidate-replies" > "$test_dir/state/fetch-fixtures"
    printf '%s\n' valid > "$test_dir/state/fetch-case"
    candidate_hash=$(sha256sum "$candidate_stage/packages/compose.rpm"); candidate_hash=${candidate_hash%% *}
    candidate_identity="$candidate_name|0|1.5.0|1.el10_1|noarch"; candidate_sig=valid; expected_status=0; actual=0
    case $candidate_case in
      changed) printf '%s\n' changed >> "$candidate_stage/packages/compose.rpm"; expected_status=1 ;;
      foreign_signature) candidate_sig=wrong_key; expected_status=1 ;;
      wrong_nevra) candidate_identity='podman-compose|1|1.5.0|1.el10_1|noarch'; expected_status=1 ;;
      bootstrap_in_epel) expected_status=1 ;;
    esac
    : > "$test_dir/log"
    env -i LC_ALL="$test_locale" PATH="$test_dir/bin" HOME="$test_dir/home" FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" \
      FAKE_ALLOW_KEY_READ=1 FAKE_ALLOW_RPMKEYS=1 FAKE_SIGNATURE_CASE="$candidate_sig" FAKE_ALLOW_PAYLOAD=1 FAKE_PROVIDER="$test_dir/tree/usr/bin/podman-compose" FAKE_EPEL_IDENTITY="$candidate_name|0|1.5.0|1.el10_1|noarch" \
      "$test_dir/bin/sh" -c '. "$1"; prereq_epel_candidate_ok "$2" "$3" "$4" "$5" "$6" 0' sh "$include" "$candidate_stage" "$candidate_stage/packages/compose.rpm" "$candidate_hash" "$candidate_identity" 'https://mirrors.fedoraproject.org/metalink?repo=epel-z-10.2&arch=x86_64' \
      > "$test_dir/out" 2>&1 || actual=$?
    if [ "$actual" != "$expected_status" ]; then printf 'FAIL W1-73 EPEL candidate: %s returned %s, expected %s (%s)\n' "$candidate_case" "$actual" "$expected_status" "$test_locale" >&2; cat "$test_dir/out" >&2; exit 1; fi
    rm -rf "$candidate_stage" "$test_dir/candidate-replies"
  done
  printf 'ok W1-73 EPEL candidate origin, identity, signer and hash (%s)\n' "$test_locale"
}

prereq_test_extract_cases() {
  for extract_case in valid duplicate symlink missing traversal changed foreign_origin foreign_key no_tool; do
    if [ -n "$prereq_screen_case" ] && [ "$extract_case" != "$prereq_screen_case" ]; then continue; fi
    extract_stage="$test_dir/extract-$extract_case"
    mkdir -m 700 "$extract_stage" "$extract_stage/packages"
    printf '%s\n' 'verified synthetic epel-release archive' > "$extract_stage/packages/epel-release.rpm"
    extract_hash=$(sha256sum "$extract_stage/packages/epel-release.rpm"); extract_hash=${extract_hash%% *}
    extract_fixture=$extract_case; extract_repo=extras; extract_key=valid; expected_status=3; actual=0
    case $extract_case in
      valid) expected_status=0 ;;
      changed) extract_fixture=valid; printf '%s\n' changed >> "$extract_stage/packages/epel-release.rpm" ;;
      foreign_origin) extract_fixture=valid; extract_repo=baseos ;;
      foreign_key) extract_fixture=valid; extract_key=foreign ;;
      no_tool) extract_fixture=valid; mv "$test_dir/bin/rpm2cpio" "$test_dir/bin/rpm2cpio.saved" ;;
    esac
    python3 "$root/scripts/fakes/prereq/epel-release-fixtures.py" "$test_dir/epel-release.cpio" "$extract_fixture"
    printf '%s\n' "$test_dir/epel-release.cpio" > "$test_dir/state/epel-cpio"
    printf '%s|%s|epel-release|0|10|8.el10|noarch|%s\n' "$extract_stage/packages/epel-release.rpm" "$extract_hash" "$extract_repo" > "$extract_stage/transaction.manifest"
    : > "$test_dir/log"
    env -i LC_ALL="$test_locale" PATH="$test_dir/bin" HOME="$test_dir/home" FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" \
      FAKE_ALLOW_EXTRACT=1 FAKE_EPEL_CPIO="$test_dir/epel-release.cpio" FAKE_ALLOW_KEY_READ=1 FAKE_KEY_CASE="$extract_key" \
      "$test_dir/bin/sh" -c '. "$1"; EX_PREREQ=3; PREREQ_ROCKY_PRE_STAGE=$2; PREREQ_ROCKY_VERIFIED_MANIFEST=$(cat "$2/transaction.manifest"); prereq_extract_epel_release "$2"' \
      sh "$include" "$extract_stage" > "$test_dir/out" 2>&1 || actual=$?
    if [ "$extract_case" = no_tool ]; then mv "$test_dir/bin/rpm2cpio.saved" "$test_dir/bin/rpm2cpio"; fi
    if [ "$actual" != "$expected_status" ]; then printf 'FAIL W1-73 EPEL bootstrap extraction: %s returned %s, expected %s (%s)\n' "$extract_case" "$actual" "$expected_status" "$test_locale" >&2; cat "$test_dir/out" >&2; exit 1; fi
    if [ "$extract_case" = valid ] && { [ ! -s "$extract_stage/epel-config/epel.repo" ] || [ ! -s "$extract_stage/epel-pinned/certificate.key" ]; }; then
      printf '%s\n' 'FAIL W1-73: verified EPEL config or certificate missing' >&2; exit 1
    fi
    if [ "$extract_case" = no_tool ] && ! grep -q 'restore rpm2cpio' "$test_dir/out"; then printf '%s\n' 'FAIL W1-73: missing RPM extractor has no actionable fix' >&2; exit 1; fi
    if grep -Eq '^(sudo|dnf|curl|gpgv) ' "$test_dir/log"; then printf '%s\n' 'FAIL W1-73: bootstrap extraction changed the machine or used the network' >&2; exit 1; fi
    rm -rf "$extract_stage"
  done
  printf 'ok W1-73 extract EPEL config only from verified Rocky Extras RPM (%s)\n' "$test_locale"
}

prereq_test_combine_cases() {
  for combine_case in valid duplicate changed collision swap_after_combine; do
    if [ -n "$prereq_screen_case" ] && [ "$combine_case" != "$prereq_screen_case" ]; then continue; fi
    if [ "$combine_case" = swap_after_combine ] && [ "${PREREQ_V01_SELECT:-}" = mixed ]; then continue; fi
    combine_stage="$test_dir/combine-$combine_case"
    mkdir -m 700 "$combine_stage" "$combine_stage/bootstrap" "$combine_stage/bootstrap/packages" "$combine_stage/mixed" "$combine_stage/mixed/packages"
    printf '%s\n' 'verified extras archive' > "$combine_stage/bootstrap/packages/two.rpm"
    printf '%s\n' 'verified Rocky archive' > "$combine_stage/mixed/packages/one.rpm"
    printf '%s\n' 'verified EPEL archive' > "$combine_stage/mixed/packages/compose.rpm"
    printf '%s\n' 'fingerprint=0123456789012345678901234567890123456789' > "$combine_stage/mixed/roots.receipt"
    combine_boot_hash=$(sha256sum "$combine_stage/bootstrap/packages/two.rpm"); combine_boot_hash=${combine_boot_hash%% *}
    combine_rocky_hash=$(sha256sum "$combine_stage/mixed/packages/one.rpm"); combine_rocky_hash=${combine_rocky_hash%% *}
    combine_epel_hash=$(sha256sum "$combine_stage/mixed/packages/compose.rpm"); combine_epel_hash=${combine_epel_hash%% *}
    printf '%s|%s|epel-release|0|10|8.el10|noarch|extras\n' "$combine_stage/bootstrap/packages/two.rpm" "$combine_boot_hash" > "$combine_stage/bootstrap/transaction.manifest"
    printf '%s|%s|gnupg2|0|2.4.5|1.el10|x86_64|baseos\n%s|%s|podman-compose|0|1.5.0|1.el10_1|noarch|epel\n' \
      "$combine_stage/mixed/packages/one.rpm" "$combine_rocky_hash" "$combine_stage/mixed/packages/compose.rpm" "$combine_epel_hash" > "$combine_stage/mixed/transaction.manifest"
    if [ "$combine_case" = duplicate ]; then
      cp "$combine_stage/mixed/packages/one.rpm" "$combine_stage/bootstrap/packages/one.rpm"
      printf '%s|%s|gnupg2|0|2.4.5|1.el10|x86_64|baseos\n' "$combine_stage/bootstrap/packages/one.rpm" "$combine_rocky_hash" >> "$combine_stage/bootstrap/transaction.manifest"
    fi
    if [ "$combine_case" = collision ]; then
      mv "$combine_stage/mixed/packages/one.rpm" "$combine_stage/mixed/packages/two.rpm"
      sed -i 's|/one.rpm|/two.rpm|' "$combine_stage/mixed/transaction.manifest"
    fi
    expected_status=3; actual=0
    case $combine_case in valid|duplicate|swap_after_combine) expected_status=0 ;; esac
    env -i LC_ALL="$test_locale" PATH="$test_dir/bin" FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" RUN_COMBINE_CASE="$combine_case" \
      "$test_dir/bin/sh" -c '. "$1"; EX_PREREQ=3; boot=$(cat "$2/bootstrap/transaction.manifest"); mixed=$(cat "$2/mixed/transaction.manifest")
        if [ "$RUN_COMBINE_CASE" = changed ]; then printf "%s\n" changed >> "$2/bootstrap/packages/two.rpm"; fi
        if [ "$RUN_COMBINE_CASE" = swap_after_combine ]; then
          python3() {
            command python3 "$@" || return $?
            victim=$3/packages/one.rpm; old=$(sha256sum "$victim"); old=${old%% *}
            printf "%s\n" attacker-replacement > "$victim"; changed=$(sha256sum "$victim"); changed=${changed%% *}
            sed "s/|$old|/|$changed|/" "$3/transaction.manifest" > "$3/swapped.manifest" && mv "$3/swapped.manifest" "$3/transaction.manifest"
            : > "$3/attack-performed"
          }
        fi
        if ! prereq_combine_bootstrap "$2" "$boot" "$mixed"; then [ "$RUN_COMBINE_CASE" = swap_after_combine ] && [ -f "$2/attack-performed" ] && exit 0; exit 3; fi
        if [ "$RUN_COMBINE_CASE" = swap_after_combine ]; then [ -f "$2/attack-performed" ] || exit 98; changed=$(sha256sum "$2/packages/one.rpm"); changed=${changed%% *}; case $PREREQ_ROCKY_VERIFIED_MANIFEST in *"|$changed|"*) printf "%s\n" "FAIL V-01: adopted post-combination tuple" >&2; exit 99 ;; esac; fi' sh "$include" "$combine_stage" > "$test_dir/out" 2>&1 || actual=$?
    if [ "$actual" != "$expected_status" ]; then printf 'FAIL W1-73 combined closures: %s returned %s, expected %s (%s)\n' "$combine_case" "$actual" "$expected_status" "$test_locale" >&2; cat "$test_dir/out" >&2; exit 1; fi
    if [ "$expected_status" = 0 ] && { [ "$(wc -l < "$combine_stage/transaction.manifest")" != 3 ] || ! grep -q '/two.rpm$' "$combine_stage/bootstrap.manifest"; }; then
      printf '%s\n' 'FAIL W1-73 combined closures: wrong archive union or bootstrap group' >&2; exit 1
    fi
    rm -rf "$combine_stage"
  done
  printf 'ok W1-73 combine verified closures and deduplicate dependencies (%s)\n' "$test_locale"
}

prereq_test_metadata_cases() {
  mv "$test_dir/bin/stat" "$test_dir/bin/stat.real"
  ln -s "$root/scripts/fakes/prereq/fake-command" "$test_dir/bin/stat"
  for metadata_case in valid wrong_hash primary_bad missing metadata_required metadata_signed metadata_bad_sig metadata_foreign_sig metadata_missing_sig assign_valid assign_wrong_hash assign_missing private_valid private_unowned private_assign_valid classify_missing; do
    if [ -n "$prereq_screen_case" ] && [ "$metadata_case" != "$prereq_screen_case" ]; then continue; fi
    mkdir -p "$test_dir/rocky-meta-$metadata_case/packages" "$test_dir/rocky-meta-$metadata_case/rocky-ring" "$test_dir/rocky-replies-$metadata_case" "$test_dir/rocky-key-$metadata_case"
    python3 "$root/scripts/fakes/prereq/rocky-fixtures.py" "$test_dir/rocky-key-$metadata_case"
    cp "$test_dir/rocky-key-$metadata_case/root1.asc" "$test_dir/rocky-meta-$metadata_case/rocky-ring/root-1.key"
    head -n 1 "$test_dir/rocky-key-$metadata_case/fingerprints" > "$test_dir/state/gpgv-signer"
    printf '%s\n' 'synthetic Rocky package one' > "$test_dir/rocky-meta-$metadata_case/packages/one.rpm"
    printf '%s\n' 'synthetic Rocky package two' > "$test_dir/rocky-meta-$metadata_case/packages/two.rpm"
    hash1=$(sha256sum "$test_dir/rocky-meta-$metadata_case/packages/one.rpm"); hash1=${hash1%% *}
    hash2=$(sha256sum "$test_dir/rocky-meta-$metadata_case/packages/two.rpm"); hash2=${hash2%% *}
    printf '%s|%s|gnupg2|0|2.4.5|1.el10|x86_64|baseos\n%s|%s|epel-release|0|10|8.el10|noarch|extras\n' \
      "$test_dir/rocky-meta-$metadata_case/packages/one.rpm" "$hash1" "$test_dir/rocky-meta-$metadata_case/packages/two.rpm" "$hash2" \
      > "$test_dir/rocky-meta-$metadata_case/transaction.manifest"
    fixture_case=$metadata_case; metadata_mode=verify; stat_case=valid
    case $metadata_case in
      assign_*) fixture_case=${metadata_case#assign_}; metadata_mode=assign ;;
      private_valid) fixture_case=valid; metadata_mode=verify_private; stat_case=private ;;
      private_unowned) fixture_case=valid; metadata_mode=verify_private ;;
      private_assign_valid) fixture_case=valid; metadata_mode=assign_private; stat_case=private ;;
      classify_missing) fixture_case=missing; metadata_mode=classify_private; stat_case=private ;;
    esac
    python3 "$root/scripts/fakes/prereq/rocky-repo-fixtures.py" "$test_dir/rocky-replies-$metadata_case" \
      "$test_dir/rocky-meta-$metadata_case/packages/one.rpm" "$test_dir/rocky-meta-$metadata_case/packages/two.rpm" "$fixture_case"
    metadata_manifest="$test_dir/rocky-meta-$metadata_case/transaction.manifest"
    case $metadata_mode in assign|assign_private|classify_private)
      cut -d '|' -f 1-7 "$metadata_manifest" > "$test_dir/rocky-meta-$metadata_case/candidates.manifest"
      metadata_manifest="$test_dir/rocky-meta-$metadata_case/candidates.manifest"
    ;; esac
    : > "$test_dir/rocky-replies-$metadata_case/metalink"
    printf '%s\n' "$test_dir/rocky-replies-$metadata_case" > "$test_dir/state/fetch-fixtures"
    printf '%s\n' "$metadata_case" > "$test_dir/state/fetch-case"
    : > "$test_dir/log"
    actual=0; expected_status=1; metadata_policy='baseos=0,extras=0'
    [ "$metadata_case" != valid ] || expected_status=0
    [ "$metadata_case" != assign_valid ] || expected_status=0
    case $metadata_case in private_valid|private_assign_valid|classify_missing) expected_status=0 ;; esac
    case $metadata_case in metadata_*) metadata_policy='baseos=1,extras=0' ;; esac
    [ "$metadata_case" != metadata_signed ] || expected_status=0
    if [ "$metadata_case" = metadata_required ]; then mv "$test_dir/bin/gpgv" "$test_dir/bin/gpgv.saved"; fi
    env -i LC_ALL="$test_locale" PATH="$test_dir/bin" FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" FAKE_ALLOW_ROOT_STAT=1 FAKE_STAT_CASE="$stat_case" \
      "$test_dir/bin/sh" -c '. "$1"; if [ "$4" = verify ]; then prereq_rocky_metadata_ok 10.2 "$2" "$3"; else prereq_rocky_metadata_ok 10.2 "$2" "$3" "$4"; fi' sh "$include" \
      "$metadata_manifest" "$metadata_policy" "$metadata_mode" > "$test_dir/out" 2>&1 || actual=$?
    if [ "$actual" != "$expected_status" ]; then
      printf 'FAIL IP-rocky-trust: %s metadata returned %s, expected %s (%s)\n' "$metadata_case" "$actual" "$expected_status" "$test_locale" >&2; exit 1
    fi
    if [ "$metadata_case" = classify_missing ] && ! grep -q '|epel-release|0|10|8.el10|noarch|unmatched$' "$test_dir/out"; then
      printf '%s\n' 'FAIL IP-rocky-trust: unmatched archive was silently assigned Rocky origin' >&2; exit 1
    fi
    if [ "$metadata_case" = metadata_required ] && grep -q '^curl ' "$test_dir/log"; then
      printf '%s\n' 'FAIL IP-rocky-trust: enabled metadata signatures were bypassed' >&2; exit 1
    fi
    if [ "$metadata_case" = metadata_required ] && ! grep -q 'provision trusted GnuPG' "$test_dir/out"; then
      printf '%s\n' 'FAIL W1-73: missing metadata verifier has no actionable fix' >&2; exit 1
    fi
    if { [ "$metadata_case" = assign_valid ] || [ "$metadata_case" = private_assign_valid ]; } && { ! grep -q '|gnupg2|0|2.4.5|1.el10|x86_64|baseos$' "$test_dir/out" ||
      ! grep -q '|epel-release|0|10|8.el10|noarch|extras$' "$test_dir/out"; }; then
      printf '%s\n' 'FAIL IP-rocky-trust: authenticated repository assignment is incomplete' >&2; exit 1
    fi
    if [ "$metadata_case" = metadata_required ]; then mv "$test_dir/bin/gpgv.saved" "$test_dir/bin/gpgv"; fi
  done
  rm -f "$test_dir/state/gpgv-signer"
  rm "$test_dir/bin/stat"
  mv "$test_dir/bin/stat.real" "$test_dir/bin/stat"
  rm -rf "$test_dir"/rocky-meta-* "$test_dir"/rocky-replies-* "$test_dir"/rocky-key-*
  printf 'ok IP-rocky-trust official metadata binds every bootstrap archive (%s)\n' "$test_locale"
}

prereq_test_pre_cases() {
  for pre_case in valid bad_metadata bad_signature unsafe_stage signed_without_tool; do
    if [ -n "$prereq_screen_case" ] && [ "$pre_case" != "$prereq_screen_case" ]; then continue; fi
    mkdir -p "$test_dir/pre-$pre_case" "$test_dir/pre-keys-$pre_case" "$test_dir/pre-replies-$pre_case"
    chmod 700 "$test_dir/pre-$pre_case"
    [ "$pre_case" != unsafe_stage ] || chmod 755 "$test_dir/pre-$pre_case"
    python3 "$root/scripts/fakes/prereq/rocky-fixtures.py" "$test_dir/pre-keys-$pre_case"
    printf '%s\n' 'synthetic downloaded archive' > "$test_dir/pre-replies-$pre_case/expected.rpm"
    fixture_case=deps signature_case=valid pre_repo_case=metadata_off expected_status=3 actual=0
    case $pre_case in valid) expected_status=0 ;; bad_metadata) fixture_case=deps_bad ;; bad_signature) signature_case=unsigned ;; signed_without_tool) pre_repo_case=valid; mv "$test_dir/bin/gpgv" "$test_dir/bin/gpgv.saved" ;; esac
    python3 "$root/scripts/fakes/prereq/rocky-repo-fixtures.py" "$test_dir/pre-replies-$pre_case" \
      "$test_dir/pre-replies-$pre_case/expected.rpm" "$test_dir/pre-replies-$pre_case/expected.rpm" "$fixture_case"
    : > "$test_dir/pre-replies-$pre_case/metalink"
    printf '%s\n' "$test_dir/pre-replies-$pre_case" > "$test_dir/state/fetch-fixtures"
    printf '%s\n' valid > "$test_dir/state/fetch-case"
    : > "$test_dir/log"
    env -i LC_ALL="$test_locale" PATH="$test_dir/bin" FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" FAKE_BIN="$test_dir/bin" \
      FAKE_ALLOW_ROOTS=1 FAKE_ALLOW_ROCKY_CONFIG=1 FAKE_ROCKY_CASE="$pre_repo_case" FAKE_ALLOW_ROCKY_DOWNLOAD=1 FAKE_INCLUDE_DEPS=1 \
      FAKE_ROOT_STAGE="$test_dir/pre-$pre_case" FAKE_ALLOW_ROCKY_ARCHIVES=1 FAKE_ALLOW_RPMKEYS=1 FAKE_SIGNATURE_CASE="$signature_case" \
      "$test_dir/bin/sh" -c '. "$1"; EX_PREREQ=3; PREREQ_MISSING=$(printf "gnupg2\npodman-compose (requalify)"); prereq_rocky_preverify "$2" 10.2 main "$3"' \
      sh "$include" "$test_dir/pre-$pre_case" "$test_dir/pre-keys-$pre_case/root1.asc" > "$test_dir/out" 2>&1 || actual=$?
    if [ "$pre_case" = signed_without_tool ]; then mv "$test_dir/bin/gpgv.saved" "$test_dir/bin/gpgv"; fi
    if [ "$actual" != "$expected_status" ]; then printf 'FAIL IP-W1-73: %s preverification returned %s, expected %s (%s)\n' "$pre_case" "$actual" "$expected_status" "$test_locale" >&2; exit 1; fi
    if [ "$pre_case" = valid ] && [ "$(wc -l < "$test_dir/pre-$pre_case/transaction.manifest")" != 3 ]; then printf '%s\n' 'FAIL IP-W1-73: dependency closure was not fully verified' >&2; exit 1; fi
    if [ "$pre_case" = signed_without_tool ] && { ! grep -q 'provision trusted GnuPG' "$test_dir/out" || grep -Eq '^(curl|dnf .*<download>) ' "$test_dir/log"; }; then printf '%s\n' 'FAIL W1-73: preverification hid the missing metadata verifier or fetched packages' >&2; exit 1; fi
    if grep -q '^sudo ' "$test_dir/log" || grep -q 'Prerequisite plan:' "$test_dir/out"; then printf '%s\n' 'FAIL IP-W1-73: preverification used sudo or printed a plan' >&2; exit 1; fi
  done
  rm -rf "$test_dir"/pre-* "$test_dir"/pre-keys-* "$test_dir"/pre-replies-*
  printf 'ok IP-W1-73 complete dependency closure verified before plan (%s)\n' "$test_locale"
}

prereq_test_nocurl_cases() {
  if [ "$prereq_screen_kind" = nocurl ]; then mkdir -p "$test_dir/pinned-ring"; : > "$test_dir/pinned-ring/certificate.key"; fi
  mkdir -m 700 "$test_dir/nocurl-bin" "$test_dir/nocurl-replies" "$test_dir/nocurl-epel" "$test_dir/nocurl-redirect" "$test_dir/nocurl-rocky" "$test_dir/nocurl-rocky/packages"
  for test_command in "$test_dir/bin"/*; do
    case ${test_command##*/} in curl|python3) ;; *) ln -s "$test_command" "$test_dir/nocurl-bin/${test_command##*/}" ;; esac
  done
  ln -s "$root/scripts/fakes/prereq/nocurl-python" "$test_dir/nocurl-bin/python3"
  printf '%s\n' 'synthetic downloaded archive' > "$test_dir/nocurl-rocky/packages/one.rpm"
  python3 "$root/scripts/fakes/prereq/rocky-repo-fixtures.py" "$test_dir/nocurl-replies" "$test_dir/nocurl-rocky/packages/one.rpm" "$test_dir/nocurl-rocky/packages/one.rpm" deps
  python3 "$root/scripts/fakes/prereq/repo-fixtures.py" "$test_dir/nocurl-replies" valid
  printf '%s\n' "$test_dir/nocurl-replies" > "$test_dir/state/fetch-fixtures"
  nocurl_hash=$(sha256sum "$test_dir/nocurl-rocky/packages/one.rpm"); nocurl_hash=${nocurl_hash%% *}
  printf '%s|%s|gnupg2|0|2.4.5|1.el10|x86_64\n' "$test_dir/nocurl-rocky/packages/one.rpm" "$nocurl_hash" > "$test_dir/nocurl-rocky/candidates.manifest"
  for nocurl_case in epel rocky redirect rocky_redirect; do
    if [ -n "$prereq_screen_case" ] && [ "$nocurl_case" != "$prereq_screen_case" ]; then continue; fi
    actual=0; nocurl_redirect=0; nocurl_expected=0; nocurl_stage="$test_dir/nocurl-epel"; : > "$test_dir/log"
    [ "$nocurl_case" != redirect ] || { nocurl_redirect=1; nocurl_expected=1; nocurl_stage="$test_dir/nocurl-redirect"; }
    [ "$nocurl_case" != rocky_redirect ] || { nocurl_redirect=1; nocurl_expected=1; }
    env -i LC_ALL="$test_locale" PATH="$test_dir/nocurl-bin" HOME="$test_dir/home" FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" \
      FAKE_REAL_PYTHON="$(command -v python3)" FAKE_PATCH_PY="$root/scripts/fakes/prereq/nocurl-patch.py" FAKE_HTTP_REDIRECT="$nocurl_redirect" RUN_NOCURL_CASE="$nocurl_case" \
      "$test_dir/nocurl-bin/sh" -c '. "$1"; case "$RUN_NOCURL_CASE" in epel|redirect)
          prereq_epel_fetch "https://mirrors.fedoraproject.org/metalink?repo=epel-z-10.2&arch=x86_64" "podman-compose|0|1.5.0|1.el10_1|noarch" "$2" 0 "$3"
        ;; *) prereq_rocky_metadata_ok 10.2 "$4/candidates.manifest" "baseos=0,appstream=0,crb=0" assign_private ;; esac' \
      sh "$include" "$nocurl_stage" "$test_dir/pinned-ring/certificate.key" "$test_dir/nocurl-rocky" > "$test_dir/out" 2>&1 || actual=$?
    if [ "$actual" != "$nocurl_expected" ] || grep -q '^curl ' "$test_dir/log"; then printf 'FAIL W1-73 missing curl: %s returned %s, expected %s (%s)\n' "$nocurl_case" "$actual" "$nocurl_expected" "$test_locale" >&2; cat "$test_dir/out" >&2; exit 1; fi
  done
  rm -rf "$test_dir/nocurl-bin" "$test_dir/nocurl-replies" "$test_dir/nocurl-epel" "$test_dir/nocurl-redirect" "$test_dir/nocurl-rocky"
  printf 'ok W1-73 metadata verification when curl is missing (%s)\n' "$test_locale"
}

prereq_test_flow_cases() {
  mv "$test_dir/bin/stat" "$test_dir/bin/stat.real"
  ln -s "$root/scripts/fakes/prereq/fake-command" "$test_dir/bin/stat"
  for flow_case in approved plan_output_failure repeat_package repeat_provider repeat_bootstrap fd3_probe plus_dependency missing_dependency partial_resume pty_yes pty_no pty_blank pty_eof repair noop bad_signature install_provider install_bad_signature bootstrap_provider bootstrap_bad_signature bootstrap_failure bootstrap_decline bootstrap_existing bootstrap_existing_packages bootstrap_existing_bad missing_curl bootstrap_missing_curl missing_gpg bootstrap_missing_gpg unsafe_home unsafe_home_install; do
    if [ -n "$prereq_screen_case" ] && [ "$flow_case" != "$prereq_screen_case" ]; then continue; fi
    mkdir -p "$test_dir/flow-replies" "$test_dir/flow-keys" "$test_dir/tree/var/cache"
    printf 'cortexuser:100000:65536\n' > "$test_dir/tree/etc/subuid"
    printf 'cortexuser:100000:65536\n' > "$test_dir/tree/etc/subgid"
    python3 "$root/scripts/fakes/prereq/rocky-fixtures.py" "$test_dir/flow-keys"
    cp "$test_dir/flow-keys/root1.asc" "$test_dir/tree/etc/pki/rpm-gpg/RPM-GPG-KEY-Rocky-10"
    printf '%s\n' 'synthetic downloaded archive' > "$test_dir/flow-replies/expected.rpm"
    flow_fixture=deps; case $flow_case in missing_curl|bootstrap_missing_curl) flow_fixture=deps_curl ;; esac
    python3 "$root/scripts/fakes/prereq/rocky-repo-fixtures.py" "$test_dir/flow-replies" "$test_dir/flow-replies/expected.rpm" "$test_dir/flow-replies/expected.rpm" "$flow_fixture"
    python3 "$root/scripts/fakes/prereq/repo-fixtures.py" "$test_dir/flow-replies" valid
    python3 "$root/scripts/fakes/prereq/epel-release-fixtures.py" "$test_dir/epel-release.cpio" valid
    printf '%s\n' "$test_dir/epel-release.cpio" > "$test_dir/state/epel-cpio"
    printf '%s\n' "$test_dir/flow-replies" > "$test_dir/state/fetch-fixtures"
    printf '%s\n' valid > "$test_dir/state/fetch-case"
    : > "$test_dir/log"; rm -f "$test_dir/state/rocky-installed" "$test_dir/state/bootstrap-installed" "$test_dir/state"/installed.*
    expected_status=0; flow_sig=valid; flow_epel_sig=valid; flow_fixes=''; flow_noop=0; flow_mixed=0; flow_boot=0; flow_boot_fail=0; flow_decline=0; flow_path="$test_dir/bin"; flow_create_curl=''; flow_create_gpg=''; flow_no_gpg=0; flow_main=gnupg2; actual=0
    flow_plus=0; [ "$flow_case" != plus_dependency ] || flow_plus=1
    flow_fd3=0; [ "$flow_case" != fd3_probe ] || flow_fd3=1
    flow_skip=''; case $flow_case in missing_dependency) flow_skip=dependency-two ;; partial_resume) flow_skip=gnupg2 ;; esac
    flow_missing=$(printf 'gnupg2\npodman-compose (requalify)')
    [ "$flow_case" != bad_signature ] || { expected_status=3; flow_sig=digest_only; flow_epel_sig=digest_only; }
    [ "$flow_case" != noop ] || { expected_status=3; flow_noop=1; }
    [ "$flow_case" != missing_dependency ] || expected_status=3
    [ "$flow_case" != plan_output_failure ] || expected_status=3
    case $flow_case in
      install_provider|install_bad_signature|repeat_provider) flow_mixed=1; flow_missing=$(printf 'gnupg2\npodman-compose') ;;
      pty_yes|pty_no|pty_blank|pty_eof) flow_mixed=1; flow_missing=$(printf 'gnupg2\npodman-compose') ;;
      unsafe_home_install) flow_mixed=1; flow_missing=$(printf 'gnupg2\npodman-compose') ;;
      bootstrap_provider|bootstrap_bad_signature|bootstrap_failure|bootstrap_decline|repeat_bootstrap) flow_mixed=1; flow_boot=1; flow_fixes=epel; flow_missing=$(printf 'gnupg2\npodman-compose') ;;
      bootstrap_existing|bootstrap_existing_bad) flow_boot=1; flow_fixes=epel; flow_missing='podman-compose (requalify)' ;;
      bootstrap_existing_packages) flow_boot=1; flow_fixes=epel; flow_missing=$(printf 'gnupg2\npodman-compose (requalify)') ;;
      missing_curl|bootstrap_missing_curl)
        flow_main=curl; flow_missing=$(printf 'curl\npodman-compose (requalify)'); flow_path="$test_dir/flow-nocurl-bin"; flow_create_curl="$flow_path/curl"
        if [ "$flow_case" = bootstrap_missing_curl ]; then flow_mixed=1; flow_boot=1; flow_fixes=epel; flow_missing=$(printf 'curl\npodman-compose'); fi
        mkdir -m 700 "$flow_path"
        for test_command in "$test_dir/bin"/*; do case ${test_command##*/} in curl|python3) ;; *) ln -s "$test_command" "$flow_path/${test_command##*/}" ;; esac; done
        ln -s "$root/scripts/fakes/prereq/nocurl-python" "$flow_path/python3" ;;
      missing_gpg|bootstrap_missing_gpg|partial_resume)
        flow_no_gpg=1; flow_path="$test_dir/flow-nogpg-bin"; flow_create_gpg=$flow_path
        if [ "$flow_case" = bootstrap_missing_gpg ]; then flow_mixed=1; flow_boot=1; flow_fixes=epel; flow_missing=$(printf 'gnupg2\npodman-compose'); fi
        mkdir -m 700 "$flow_path"
        for test_command in "$test_dir/bin"/*; do case ${test_command##*/} in gpg|gpgv) ;; *) ln -s "$test_command" "$flow_path/${test_command##*/}" ;; esac; done ;;
    esac
    [ "$flow_case" != install_bad_signature ] || { expected_status=3; flow_epel_sig=wrong_key; }
    [ "$flow_case" != bootstrap_bad_signature ] || { expected_status=3; flow_epel_sig=wrong_key; }
    [ "$flow_case" != bootstrap_existing_bad ] || { expected_status=3; flow_epel_sig=wrong_key; }
    [ "$flow_case" != bootstrap_failure ] || { expected_status=3; flow_boot_fail=1; }
    [ "$flow_case" != bootstrap_decline ] || { expected_status=2; flow_decline=1; }
    case $flow_case in unsafe_home|unsafe_home_install) expected_status=3; chmod 777 "$test_dir/home" ;; esac
    if [ "$flow_case" = repair ]; then
      flow_fixes='subuid subgid'
      printf 'other:100000:65536\n' > "$test_dir/tree/etc/subuid"
      printf 'other:200000:65536\n' > "$test_dir/tree/etc/subgid"
    fi
    sed "s|$test_dir/bin|$flow_path|g" "$include" > "$test_dir/flow-include.sh"
    env -i LC_ALL="$test_locale" PATH="$flow_path" HOME="$test_dir/home" PREREQ_ROOT="$test_dir/tree" CORTEX_YES=1 \
      FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" FAKE_BIN="$test_dir/bin" FAKE_PRIVATE_PARENT="$test_dir/home" FAKE_DOWNLOAD_PARENT="$test_dir/home" \
      FAKE_ALLOW_ROOTS=1 FAKE_ALLOW_ROOT_STAT=1 FAKE_STAT_CASE=mixed FAKE_REAL_STAT="$test_dir/bin/stat.real" FAKE_ALLOW_STAGE=1 FAKE_PROBE_FD3="$flow_fd3" \
      FAKE_ALLOW_ROCKY_CONFIG=1 FAKE_ROCKY_CASE=metadata_off FAKE_ALLOW_ROCKY_DOWNLOAD=1 FAKE_INCLUDE_DEPS=1 FAKE_PLUS_DEP="$flow_plus" FAKE_ALLOW_ROCKY_ARCHIVES=1 FAKE_EPEL_MIX="$flow_mixed" \
      FAKE_ALLOW_ROCKY_INSTALL=1 FAKE_ALLOW_KEY_READ=1 FAKE_ALLOW_DNF_CONFIG=1 FAKE_REPO_CASE=metadata_off FAKE_BOOTSTRAP_DOWNLOAD="$flow_boot" FAKE_TWO_PHASE="$flow_boot" FAKE_BOOT_FAIL="$flow_boot_fail" FAKE_PRIVATE_EPEL_PARENT="$test_dir/home" \
      FAKE_ALLOW_RPMKEYS=1 FAKE_SIGNATURE_CASE="$flow_sig" FAKE_EPEL_SIGNATURE_CASE="$flow_epel_sig" FAKE_ALLOW_PAYLOAD=1 FAKE_PROVIDER="$test_dir/tree/usr/bin/podman-compose" \
      TEST_TOKEN='ip54-secret-token-marker' TEST_PASSWORD='ip54-secret-password-marker' TEST_KEY='ip54-secret-key-marker' \
      FAKE_ROOT="$test_dir/tree" FAKE_ALLOW_SUBIDS=1 FAKE_INSTALL_NOOP="$flow_noop" FAKE_SKIP_INSTALL_PACKAGE="$flow_skip" FAKE_GRAPH_ROOT="$test_dir/home/graph" FAKE_MAIN_PACKAGE="$flow_main" FAKE_CREATE_CURL="$flow_create_curl" FAKE_CREATE_GPG="$flow_create_gpg" FAKE_REAL_PYTHON="$(command -v python3)" FAKE_PATCH_PY="$root/scripts/fakes/prereq/nocurl-patch.py" RUN_FIXES="$flow_fixes" RUN_MISSING="$flow_missing" RUN_DECLINE="$flow_decline" RUN_FAKE_NO_GPG="$flow_no_gpg" RUN_FLOW_CASE="$flow_case" RUN_RESUME_HELPER="$root/scripts/fakes/prereq/fresh-resume-child.sh" RUN_PTY_HELPER="$root/scripts/fakes/prereq/public-pty-test.py" \
      "$flow_path/sh" -c '
        RUN_INCLUDE=$1; . "$1"; say() { case $RUN_FLOW_CASE:$* in plan_output_failure:"  verifier_rpm="*) return 1 ;; esac; printf "%s\n" "$*"; }; warn() { :; }; EX_PREREQ=3; EX_MACHINE=2; home_dir=$HOME; port=8601
        case $RUN_FLOW_CASE in pty_yes|pty_no|pty_blank|pty_eof) exec "$FAKE_BIN/python3" -I "$RUN_PTY_HELPER" "$RUN_INCLUDE" "$PREREQ_ROOT" "$FAKE_BIN/sh" "$RUN_FLOW_CASE" ;; esac
        if [ "$RUN_FAKE_NO_GPG" = 1 ]; then
          prereq_pgp_fingerprints() { case $1 in */certificate.key) printf "%s\n" 7D8D15CBFC4E62688591FB2633D98517E37ED158 ;; *) "$FAKE_BIN/sh" -c '\''. "$1"; prereq_pgp_fingerprints "$2"'\'' sh "$RUN_INCLUDE" "$1" ;; esac; }
        fi
        FAKE_ROOT_STAGE="$PREREQ_ROOT/var/cache/cortex-prereq-1000-$$"; export FAKE_ROOT_STAGE
        PREREQ_MISSING=$RUN_MISSING; PREREQ_FIXES=$RUN_FIXES
        if [ "$RUN_DECLINE" = 1 ]; then prereq_confirm() { printf "%s\n" "DECLINED"; return 2; }; fi
        if [ "$RUN_FLOW_CASE" = partial_resume ]; then
          if prereq_run; then printf "%s\n" "FAIL SAM-03: incomplete transaction succeeded" >&2; exit 95; fi
          [ -f "$FAKE_STATE_DIR/installed.dependency-one" ] && [ ! -f "$FAKE_STATE_DIR/installed.gnupg2" ] || exit 96
          printf "%s\n" FRESH_RESUME >> "$FAKE_LOG"
          unset FAKE_SKIP_INSTALL_PACKAGE
          exec "$FAKE_BIN/sh" "$RUN_RESUME_HELPER" "$RUN_INCLUDE"
        fi
        prereq_run || exit $?
        case $RUN_FLOW_CASE in repeat_package|repeat_provider|repeat_bootstrap)
          printf "%s\n" SECOND_RUN >> "$FAKE_LOG"
          prereq_run || exit $?
          ;;
        esac' sh "$test_dir/flow-include.sh" > "$test_dir/out" 2>&1 || actual=$?
    case $flow_case in unsafe_home|unsafe_home_install) chmod 755 "$test_dir/home" ;; esac
    if grep -Eq 'ip54-secret-' "$test_dir/out" "$test_dir/log"; then
      printf 'FAIL IP-54: public prerequisite flow leaked a synthetic marker (%s)\n' "$flow_case" >&2; exit 1
    fi
    if [ "$actual" != "$expected_status" ]; then printf 'FAIL W1-73 entry: %s returned %s, expected %s (%s)\n' "$flow_case" "$actual" "$expected_status" "$test_locale" >&2; cat "$test_dir/out" >&2; tail -15 "$test_dir/log" >&2; exit 1; fi
    case $flow_case in repeat_package|repeat_provider|repeat_bootstrap)
      awk '/^SECOND_RUN$/ { seen++; next } seen && /^(sudo|dnf|curl|gpg|gpgv|rpmkeys|loginctl) / { bad=1 } END { exit (seen != 1 || bad) ? 1 : 0 }' "$test_dir/log" || { printf 'FAIL V-06: repeat run changed machine or reacquired packages (%s)\n' "$flow_case" >&2; exit 1; }
      ;;
    esac
    if [ "$flow_case" = repeat_package ] || [ "$flow_case" = repeat_provider ] || [ "$flow_case" = repeat_bootstrap ] || [ "$flow_case" = approved ] || [ "$flow_case" = fd3_probe ] || [ "$flow_case" = plus_dependency ] || [ "$flow_case" = partial_resume ] || [ "$flow_case" = repair ] || [ "$flow_case" = install_provider ] || [ "$flow_case" = bootstrap_provider ] || [ "$flow_case" = bootstrap_existing_packages ] || [ "$flow_case" = missing_curl ] || [ "$flow_case" = bootstrap_missing_curl ] || [ "$flow_case" = missing_gpg ] || [ "$flow_case" = bootstrap_missing_gpg ]; then
      [ -f "$test_dir/state/rocky-installed" ] && grep -q 'dependency-one' "$test_dir/out" && grep -q 'dependency-two' "$test_dir/out" || { printf '%s\n' 'FAIL W1-73 entry: missing verified dependency or install' >&2; exit 1; }
      grep -Fxq "Privileged commands use $flow_path/sudo and PATH=$flow_path; package and account changes use umask 022." "$test_dir/out" || { printf '%s\n' 'FAIL W1-73: package plan privilege context missing' >&2; exit 1; }
      awk '/^  sudo / { sub(/^  /, ""); print }' "$test_dir/out" > "$test_dir/planned-sudo"
      awk '/^sudo / { gsub(/[<>]/, ""); print }' "$test_dir/log" > "$test_dir/actual-sudo"
      cmp -s "$test_dir/planned-sudo" "$test_dir/actual-sudo" || { printf '%s\n' 'FAIL W1-73 entry: sudo plan mismatch' >&2; exit 1; }
    elif { [ "$flow_case" = bad_signature ] || [ "$flow_case" = install_bad_signature ] || [ "$flow_case" = bootstrap_bad_signature ] || [ "$flow_case" = bootstrap_existing_bad ]; } && { grep -Eq '^  sudo |^Prerequisite plan:' "$test_dir/out" || grep -q '^sudo ' "$test_dir/log"; }; then
      printf '%s\n' 'FAIL W1-73 entry: bad signature reached plan or mutation' >&2; exit 1
    fi
    if [ "$flow_case" = plan_output_failure ] && grep -q '^sudo ' "$test_dir/log"; then printf '%s\n' 'FAIL F-7: failed package plan output reached sudo' >&2; exit 1; fi
    if [ "$flow_case" = missing_dependency ] && { [ ! -f "$test_dir/state/installed.dependency-one" ] || [ -f "$test_dir/state/installed.dependency-two" ]; }; then printf '%s\n' 'FAIL SAM-03: partial package state was not represented' >&2; exit 1; fi
    case $flow_case in pty_yes|pty_no|pty_blank|pty_eof) grep -q "PASS V-09: $flow_case" "$test_dir/out" || { printf '%s\n' 'FAIL V-09: public PTY receipt missing' >&2; exit 1; } ;; esac
    if [ "$flow_case" = partial_resume ] && { ! grep -q '^FRESH_RESUME$' "$test_dir/log" || [ ! -f "$test_dir/state/installed.gnupg2" ]; }; then printf '%s\n' 'FAIL SAM-03: fresh resume did not install the missing package' >&2; exit 1; fi
    if [ "$flow_case" = bootstrap_provider ] && [ ! -f "$test_dir/state/bootstrap-installed" ]; then printf '%s\n' 'FAIL W1-73 entry: Extras bootstrap did not install first' >&2; exit 1; fi
    if [ "$flow_case" = plus_dependency ] && { ! grep -q '^rpmkeys .*dep+1\.rpm' "$test_dir/log" || ! grep -q '^sudo <dnf>.*dep+1\.rpm' "$test_dir/log"; }; then printf '%s\n' 'FAIL SAM-01: plus-bearing dependency was not verified and installed' >&2; exit 1; fi
    if [ "$flow_case" = bootstrap_existing ] && { [ ! -f "$test_dir/state/bootstrap-installed" ] || [ -f "$test_dir/state/rocky-installed" ] || grep -q '^sudo <dnf>.*compose.rpm' "$test_dir/log"; }; then printf '%s\n' 'FAIL W1-73 entry: existing Compose was reinstalled or bootstrap omitted' >&2; exit 1; fi
    if [ "$flow_case" = bootstrap_existing ] && ! grep -Eq '^dnf .*<--setopt=reposdir=[^>]+/epel-config>' "$test_dir/log"; then
      printf '%s\n' 'FAIL Q17e: verified EPEL extraction did not reach staged requalification' >&2; exit 1
    fi
    if [ "$flow_case" = bootstrap_existing ]; then printf 'ok Q17e verified extraction supplies staged EPEL requalification (%s)\n' "$test_locale"; fi
    if [ "$flow_case" = bootstrap_failure ] && [ -f "$test_dir/state/rocky-installed" ]; then printf '%s\n' 'FAIL W1-73 entry: provider installed after bootstrap failure' >&2; exit 1; fi
    if [ "$flow_case" = bootstrap_decline ] && { grep -q '^sudo ' "$test_dir/log" || [ "$(grep -c '^Prerequisite plan:' "$test_dir/out")" -ne 1 ] || [ -f "$test_dir/state/bootstrap-installed" ]; }; then printf '%s\n' 'FAIL W1-73 entry: decline changed the machine or repeated the plan' >&2; exit 1; fi
    if { [ "$flow_case" = unsafe_home ] || [ "$flow_case" = unsafe_home_install ]; } && { ! grep -q 'staging parent' "$test_dir/out" || grep -Eq '^(sudo|dnf|curl|gpg|rpmkeys) ' "$test_dir/log"; }; then printf '%s\n' 'FAIL W1-73 entry: unsafe staging parent was used' >&2; exit 1; fi
    for leftover in "$test_dir/home"/.cortex-prereq.*; do [ ! -e "$leftover" ] || { printf '%s\n' 'FAIL W1-73 entry: private stage remains' >&2; exit 1; }; done
    rm -rf "$test_dir/flow-replies" "$test_dir/flow-keys"
    case $flow_case in missing_curl|bootstrap_missing_curl|missing_gpg|bootstrap_missing_gpg|partial_resume) rm -rf "$flow_path" ;; esac
  done
  printf 'cortexuser:100000:65536\n' > "$test_dir/tree/etc/subuid"
  printf 'cortexuser:100000:65536\n' > "$test_dir/tree/etc/subgid"
  rm "$test_dir/bin/stat"; mv "$test_dir/bin/stat.real" "$test_dir/bin/stat"
  printf 'ok W1-73 public prerequisite flow (%s)\n' "$test_locale"
}

prereq_test_run_cases() {
  for run_case in approved inherited_boot no_tty bad_signature nothing unsafe_home; do
    if [ -n "$prereq_screen_case" ] && [ "$run_case" != "$prereq_screen_case" ]; then continue; fi
    mkdir -p "$test_dir/run-replies"
    python3 "$root/scripts/fakes/prereq/repo-fixtures.py" "$test_dir/run-replies" valid
    printf '%s\n' "$test_dir/run-replies" > "$test_dir/state/fetch-fixtures"
    printf '%s\n' valid > "$test_dir/state/fetch-case"
    : > "$test_dir/log"
    run_yes=1 signature_case=valid run_missing='podman-compose (requalify)' expected_status=0
    run_boot_stage='' run_boot_key_hash='' run_boot_config_hash=''
    case $run_case in
      inherited_boot)
        run_boot_stage="$test_dir/home/.cortex-prereq.inherited"
        run_boot_key_hash=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa
        run_boot_config_hash=bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb ;;
      no_tty) run_yes=0; expected_status=2 ;;
      bad_signature) signature_case=digest_only; expected_status=3 ;;
      nothing) run_missing='' ;;
      unsafe_home) chmod 777 "$test_dir/home"; expected_status=3 ;;
    esac
    actual=0
    env -i LC_ALL="$test_locale" PATH="$test_dir/bin" HOME="$test_dir/home" PREREQ_ROOT="$test_dir/tree" \
      FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" FAKE_ALLOW_KEY_READ=1 FAKE_KEY_CASE=valid \
      FAKE_ALLOW_DNF_CONFIG=1 FAKE_REPO_CASE=metadata_off FAKE_ALLOW_RPMKEYS=1 FAKE_SIGNATURE_CASE="$signature_case" \
      FAKE_ALLOW_PAYLOAD=1 FAKE_PAYLOAD_CASE=valid FAKE_PROVIDER="$test_dir/tree/usr/bin/podman-compose" \
      RUN_MISSING="$run_missing" RUN_CASE="$run_case" CORTEX_YES="$run_yes" \
      PREREQ_EPEL_BOOT_STAGE="$run_boot_stage" PREREQ_EPEL_BOOT_KEY_HASH="$run_boot_key_hash" PREREQ_EPEL_BOOT_CONFIG_HASH="$run_boot_config_hash" \
      "$test_dir/bin/sh" -c '
        . "$1"; say() { printf "%s\n" "$*"; printf "say <%s>\n" "$*" >> "$FAKE_LOG"; }
        EX_PREREQ=3; EX_MACHINE=2; PREREQ_MISSING=$RUN_MISSING; PREREQ_FIXES=""
        if [ -z "$RUN_MISSING" ]; then PREREQ_COMPOSE="$PREREQ_ROOT/usr/bin/podman-compose"; prereq_compose_path() { printf "%s\n" "$PREREQ_COMPOSE"; }; fi
        prereq_run
        run_status=$?
        if [ "$RUN_CASE" = inherited_boot ] && { [ -n "$PREREQ_MISSING" ] || [ "${PREREQ_EPEL_BOOT_STAGE+x}" = x ] || [ "${PREREQ_EPEL_BOOT_KEY_HASH+x}" = x ] || [ "${PREREQ_EPEL_BOOT_CONFIG_HASH+x}" = x ]; }; then
          printf "%s\n" "FAIL Q17e: inherited EPEL boot state survived or requalification did not finish" >&2; exit 96
        fi
        exit "$run_status"' sh "$include" > "$test_dir/out" 2>&1 || actual=$?
    if [ "$run_case" = unsafe_home ]; then chmod 755 "$test_dir/home"; fi
    if [ "$actual" != "$expected_status" ]; then
      printf 'FAIL IP-prereq-flow: %s returned %s, expected %s (%s)\n' "$run_case" "$actual" "$expected_status" "$test_locale" >&2
      cat "$test_dir/out" >&2
      tail -15 "$test_dir/log" >&2
      exit 1
    fi
    if [ "$run_case" = approved ] && ! awk '
      /^say <Prerequisite plan/ { plan = NR }
      /^(gpg|dnf|curl|rpmkeys) / && !first { first = NR }
      END { if (!plan || !first || first >= plan) exit 1 }
    ' "$test_dir/log"; then
      printf '%s\n' 'FAIL W1-73: provider verification must precede the plan' >&2; exit 1
    fi
    if [ "$run_case" = inherited_boot ] && { ! grep -Eq '^dnf .*<--cacheonly> <--assumeno> <config-manager> <--dump> <epel>$' "$test_dir/log" || grep -q '<--setopt=reposdir=' "$test_dir/log"; }; then
      printf '%s\n' 'FAIL Q17e: inherited EPEL state changed the machine repo requalification path' >&2; exit 1
    fi
    if [ "$run_case" = inherited_boot ]; then printf 'ok Q17e inherited boot state cleared; machine EPEL path used (%s)\n' "$test_locale"; fi
    if [ "$run_case" = no_tty ] && grep -Eq '^sudo ' "$test_dir/log"; then
      printf '%s\n' 'FAIL IP-26: no-terminal refusal changed the machine' >&2; exit 1
    fi
    if [ "$run_case" = unsafe_home ] && { ! grep -q 'staging parent' "$test_dir/out" || grep -Eq '^(sudo|dnf|curl|gpg|rpmkeys) ' "$test_dir/log"; }; then printf '%s\n' 'FAIL W1-73: unsafe requalification parent was used' >&2; exit 1; fi
    if [ "$run_case" = bad_signature ] && grep -q '^say <Prerequisite plan' "$test_dir/log"; then
      printf '%s\n' 'FAIL W1-73: a bad signature reached the plan' >&2; exit 1
    fi
    if [ "$run_case" = nothing ] && grep -Eq '^(gpg|dnf|curl|rpmkeys|sudo) ' "$test_dir/log"; then
      printf '%s\n' 'FAIL IP-20: ready prerequisites were changed' >&2; exit 1
    fi
    if [ "$run_yes" = 1 ] && grep -q 'Proceed with these changes?' "$test_dir/out"; then
      printf '%s\n' 'FAIL IP-24: --yes still prompted' >&2; exit 1
    fi
    rm -rf "$test_dir/run-replies"
  done
  printf 'ok IP-20/22/24/26 single consent and requalification flow (%s)\n' "$test_locale"
}

prereq_test_repairrun_cases() {
  for repair_case in subids linger linger_user both noop; do
    if [ -n "$prereq_screen_case" ] && [ "$repair_case" != "$prereq_screen_case" ]; then continue; fi
    mkdir -p "$test_dir/repair-replies"
    python3 "$root/scripts/fakes/prereq/repo-fixtures.py" "$test_dir/repair-replies" valid
    printf '%s\n' "$test_dir/repair-replies" > "$test_dir/state/fetch-fixtures"
    printf '%s\n' valid > "$test_dir/state/fetch-case"
    printf 'other:100000:65536\n' > "$test_dir/tree/etc/subuid"
    printf 'other:200000:65536\n' > "$test_dir/tree/etc/subgid"
    rm -f "$test_dir/state/linger"
    : > "$test_dir/log"
    repair_fixes='subuid subgid' repair_linger=yes repair_linger_mode=sudo repair_noop=0 expected_status=0
    case $repair_case in
      linger) repair_fixes=linger; repair_linger=no; printf 'cortexuser:100000:65536\n' > "$test_dir/tree/etc/subuid"; printf 'cortexuser:100000:65536\n' > "$test_dir/tree/etc/subgid" ;;
      linger_user) repair_fixes=linger; repair_linger=no; repair_linger_mode=user; printf 'cortexuser:100000:65536\n' > "$test_dir/tree/etc/subuid"; printf 'cortexuser:100000:65536\n' > "$test_dir/tree/etc/subgid" ;;
      both) repair_fixes='subuid subgid linger'; repair_linger=no ;;
      noop) repair_noop=1; expected_status=3 ;;
    esac
    actual=0
    env -i LC_ALL="$test_locale" PATH="$test_dir/bin" HOME="$test_dir/home" PREREQ_ROOT="$test_dir/tree" CORTEX_YES=1 \
      FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" FAKE_BIN="$test_dir/bin" FAKE_ROOT="$test_dir/tree" \
      FAKE_GRAPH_ROOT="$test_dir/home/graph" FAKE_ALLOW_KEY_READ=1 FAKE_KEY_CASE=valid FAKE_ALLOW_DNF_CONFIG=1 FAKE_REPO_CASE=metadata_off \
      FAKE_ALLOW_RPMKEYS=1 FAKE_SIGNATURE_CASE=valid FAKE_ALLOW_PAYLOAD=1 FAKE_PAYLOAD_CASE=valid FAKE_PROVIDER="$test_dir/tree/usr/bin/podman-compose" \
      FAKE_ALLOW_SUBIDS=1 FAKE_USERMOD_NOOP="$repair_noop" FAKE_ALLOW_LINGER=1 FAKE_LINGER="$repair_linger" FAKE_LINGER_MODE="$repair_linger_mode" RUN_FIXES="$repair_fixes" \
      "$test_dir/bin/sh" -c '
        . "$1"; say() { printf "%s\n" "$*"; printf "say <%s>\n" "$*" >> "$FAKE_LOG"; }; warn() { :; }
        EX_PREREQ=3; EX_MACHINE=2; home_dir=$2; port=8601; PREREQ_MISSING="podman-compose (requalify)"; PREREQ_FIXES=$RUN_FIXES
        prereq_run || exit $?; printf "%s\n" SPLIT >> "$FAKE_LOG"; prereq_run' sh "$include" "$test_dir/home" \
      > "$test_dir/out" 2>&1 || actual=$?
    if [ "$actual" != "$expected_status" ]; then
      printf 'FAIL IP-repair-flow: %s returned %s, expected %s (%s)\n' "$repair_case" "$actual" "$expected_status" "$test_locale" >&2; exit 1
    fi
    if [ "$expected_status" = 0 ] && ! awk '/^SPLIT$/ { after_split = 1; next } after_split && /^(sudo|dnf|gpg|curl|rpmkeys) / { bad = 1 } END { exit bad ? 1 : 0 }' "$test_dir/log"; then
      printf '%s\n' 'FAIL IP-37: second prerequisite run repeated a change or trust acquisition' >&2; exit 1
    fi
    if [ "$repair_case" = subids ] && ! grep -Fxq 'say <  sudo usermod --add-subuids 265536-331071 --add-subgids 265536-331071 -- cortexuser>' "$test_dir/log"; then
      printf '%s\n' 'FAIL IP-22: subordinate-ID command was not shown in the plan' >&2; exit 1
    fi
    if [ "$repair_case" = linger_user ]; then
      grep -q '^say <  sudo loginctl enable-linger ' "$test_dir/log" || { printf '%s\n' 'FAIL F-3: sudo fallback not disclosed' >&2; exit 1; }
      if grep -q '^sudo <loginctl>' "$test_dir/log"; then printf '%s\n' 'FAIL F-3: successful user linger invoked sudo' >&2; exit 1; fi
    fi
    rm -rf "$test_dir/repair-replies"
  done
  rm -f "$test_dir/state/linger"
  printf 'ok IP-22/34/35/36/37 repair plan, postcheck and second run (%s)\n' "$test_locale"
}

prereq_test_approved_cases() {
  mv "$test_dir/bin/stat" "$test_dir/bin/stat.real"
  ln -s "$root/scripts/fakes/prereq/fake-command" "$test_dir/bin/stat"
  for approved_case in valid swap manifest_swap; do
    if [ -n "$prereq_screen_case" ] && [ "$approved_case" != "$prereq_screen_case" ]; then continue; fi
    case $approved_case in valid) approved_num=301 ;; swap) approved_num=302 ;; manifest_swap) approved_num=303 ;; esac
    approved_stage="$test_dir/tree/var/cache/cortex-prereq-1000-$approved_num"
    approved_private="$test_dir/approved-private-$approved_case"
    mkdir -p "$approved_private" "$test_dir/approved-keys-$approved_case" "$test_dir/approved-replies-$approved_case"
    chmod 700 "$approved_private"
    python3 "$root/scripts/fakes/prereq/rocky-fixtures.py" "$test_dir/approved-keys-$approved_case"
    printf '%s\n' 'synthetic downloaded archive' > "$test_dir/approved-replies-$approved_case/expected.rpm"
    python3 "$root/scripts/fakes/prereq/rocky-repo-fixtures.py" "$test_dir/approved-replies-$approved_case" \
      "$test_dir/approved-replies-$approved_case/expected.rpm" "$test_dir/approved-replies-$approved_case/expected.rpm" deps
    : > "$test_dir/approved-replies-$approved_case/metalink"
    printf '%s\n' "$test_dir/approved-replies-$approved_case" > "$test_dir/state/fetch-fixtures"
    printf '%s\n' valid > "$test_dir/state/fetch-case"
    rm -f "$test_dir/state/rocky-installed" "$test_dir/state"/installed.*; : > "$test_dir/log"
    expected_status=3; actual=0; copy_swap=0
    [ "$approved_case" != valid ] || expected_status=0
    [ "$approved_case" != swap ] || copy_swap=1
    env -i LC_ALL="$test_locale" PATH="$test_dir/bin" PREREQ_ROOT="$test_dir/tree" FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" FAKE_BIN="$test_dir/bin" \
      FAKE_ALLOW_ROOTS=1 FAKE_ALLOW_ROOT_STAT=1 FAKE_STAT_CASE=mixed FAKE_PRIVATE_STAGE="$approved_private" FAKE_REAL_STAT="$test_dir/bin/stat.real" \
      FAKE_ALLOW_STAGE=1 FAKE_ROOT_STAGE="$approved_stage" FAKE_ALLOW_ROCKY_CONFIG=1 FAKE_ROCKY_CASE=metadata_off \
      FAKE_ALLOW_ROCKY_DOWNLOAD=1 FAKE_DOWNLOAD_STAGE="$approved_private" FAKE_INCLUDE_DEPS=1 FAKE_ALLOW_ROCKY_ARCHIVES=1 FAKE_ALLOW_RPMKEYS=1 FAKE_ALLOW_ROCKY_INSTALL=1 FAKE_COPY_SWAP="$copy_swap" \
      RUN_CASE="$approved_case" "$test_dir/bin/sh" -c '
        . "$1"; say() { printf "%s\n" "$*"; }; EX_PREREQ=3; PREREQ_MISSING=$(printf "gnupg2\npodman-compose (requalify)")
        prereq_rocky_preverify "$2" 10.2 main "$4" || exit $?
        prereq_rocky_verified_plan "$2" "$3" 10.2 "$4" || exit $?
        printf "%s\n" PLAN_DONE >> "$FAKE_LOG"
        if [ "$RUN_CASE" = manifest_swap ]; then printf "%s\n" altered >> "$2/root-transaction.manifest"; fi
        prereq_rocky_apply_verified "$2" "$3" 10.2 "$4"' \
      sh "$include" "$approved_private" "$approved_stage" "$test_dir/approved-keys-$approved_case/root1.asc" > "$test_dir/out" 2>&1 || actual=$?
    if [ "$actual" != "$expected_status" ]; then printf 'FAIL IP-W1-73: %s approved copy returned %s, expected %s (%s)\n' "$approved_case" "$actual" "$expected_status" "$test_locale" >&2; cat "$test_dir/out" >&2; tail -12 "$test_dir/log" >&2; exit 1; fi
    if ! awk '/^PLAN_DONE$/ { after = 1; next } !after && /^sudo / { bad = 1 } END { exit bad ? 1 : 0 }' "$test_dir/log"; then printf '%s\n' 'FAIL IP-W1-73: sudo ran before the plan' >&2; exit 1; fi
    if [ "$approved_case" = valid ]; then
      if [ ! -f "$test_dir/state/rocky-installed" ] || ! grep -q 'dependency-one' "$test_dir/out" || ! grep -q 'dependency-two' "$test_dir/out"; then printf '%s\n' 'FAIL IP-W1-73: plan omitted verified dependencies or install' >&2; exit 1; fi
      if ! grep -q 'verifier_rpm=RPM version 4.19.1.1' "$test_dir/out" || ! grep -q 'verifier_dnf=4.21.1' "$test_dir/out" || ! grep -q 'os=Rocky Linux 10.2 arch=x86_64' "$test_dir/out"; then printf '%s\n' 'FAIL W1-73: plan omitted native verifier or platform receipts' >&2; exit 1; fi
      if ! grep -q 'rocky_metadata=baseos repomd_sha256=' "$test_dir/out"; then printf '%s\n' 'FAIL W1-73: Rocky metadata receipt was not retained' >&2; exit 1; fi
      awk '/^  sudo / { sub(/^  /, ""); print }' "$test_dir/out" > "$test_dir/planned-sudo"
      awk '/^sudo / { gsub(/[<>]/, ""); print }' "$test_dir/log" > "$test_dir/actual-sudo"
      if ! cmp -s "$test_dir/planned-sudo" "$test_dir/actual-sudo"; then printf '%s\n' 'FAIL IP-22: planned and actual sudo commands differ' >&2; exit 1; fi
    elif [ -f "$test_dir/state/rocky-installed" ]; then printf '%s\n' 'FAIL IP-W1-73: changed archive or manifest was installed' >&2; exit 1; fi
    if [ -e "$approved_stage" ]; then printf '%s\n' 'FAIL IP-W1-73: root copy was not removed' >&2; exit 1; fi
  done
  rm "$test_dir/bin/stat"
  mv "$test_dir/bin/stat.real" "$test_dir/bin/stat"
  rm -rf "$test_dir"/approved-private-* "$test_dir"/approved-keys-* "$test_dir"/approved-replies-* "$test_dir/state/rocky-installed" "$test_dir/state"/installed.*
  printf 'ok IP-W1-73 exact plan and approved root copy (%s)\n' "$test_locale"
}

prereq_test_mixedapply_cases() {
  if [ -n "$prereq_screen_case" ]; then mkdir -p "$test_dir/tree/var/cache"; mv "$test_dir/bin/stat" "$test_dir/bin/stat.real"; ln -s "$root/scripts/fakes/prereq/fake-command" "$test_dir/bin/stat"; fi
  for mixed_apply_case in valid swap recheck_bad_signature recheck_bad_metadata; do
    if [ -n "$prereq_screen_case" ] && [ "$mixed_apply_case" != "$prereq_screen_case" ]; then continue; fi
    mixed_stage="$test_dir/mixed-apply-$mixed_apply_case"; mixed_root="$test_dir/tree/var/cache/cortex-prereq-1000-451"
    mkdir -m 700 "$mixed_stage" "$test_dir/mixed-apply-keys" "$test_dir/mixed-apply-replies"
    mkdir -p "$test_dir/tree/var/cache"
    python3 "$root/scripts/fakes/prereq/rocky-fixtures.py" "$test_dir/mixed-apply-keys"
    printf '%s\n' 'synthetic downloaded archive' > "$test_dir/mixed-apply-replies/expected.rpm"
    python3 "$root/scripts/fakes/prereq/rocky-repo-fixtures.py" "$test_dir/mixed-apply-replies" "$test_dir/mixed-apply-replies/expected.rpm" "$test_dir/mixed-apply-replies/expected.rpm" deps
    python3 "$root/scripts/fakes/prereq/repo-fixtures.py" "$test_dir/mixed-apply-replies" valid
    printf '%s\n' "$test_dir/mixed-apply-replies" > "$test_dir/state/fetch-fixtures"
    printf '%s\n' valid > "$test_dir/state/fetch-case"
    : > "$test_dir/log"; rm -f "$test_dir/state/rocky-installed" "$test_dir/state"/installed.*
    mixed_swap=0 expected_status=0 actual=0
    [ "$mixed_apply_case" != swap ] || { mixed_swap=1; expected_status=3; }
    case $mixed_apply_case in recheck_bad_signature|recheck_bad_metadata) expected_status=3 ;; esac
    env -i LC_ALL="$test_locale" PATH="$test_dir/bin" HOME="$test_dir/home" PREREQ_ROOT="$test_dir/tree" FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" FAKE_BIN="$test_dir/bin" \
      FAKE_ALLOW_ROOTS=1 FAKE_ALLOW_ROOT_STAT=1 FAKE_STAT_CASE=mixed FAKE_PRIVATE_STAGE="$mixed_stage" FAKE_REAL_STAT="$test_dir/bin/stat.real" \
      FAKE_ALLOW_STAGE=1 FAKE_ROOT_STAGE="$mixed_root" FAKE_DOWNLOAD_STAGE="$mixed_stage" FAKE_COPY_SWAP="$mixed_swap" \
      FAKE_ALLOW_ROCKY_CONFIG=1 FAKE_ROCKY_CASE=metadata_off FAKE_ALLOW_DNF_CONFIG=1 FAKE_REPO_CASE=metadata_off \
      FAKE_ALLOW_ROCKY_DOWNLOAD=1 FAKE_EPEL_MIX=1 FAKE_ALLOW_ROCKY_ARCHIVES=1 FAKE_ALLOW_ROCKY_INSTALL=1 \
      FAKE_ALLOW_RPMKEYS=1 FAKE_ALLOW_KEY_READ=1 FAKE_ALLOW_PAYLOAD=1 FAKE_PROVIDER="$test_dir/tree/usr/bin/podman-compose" RUN_MIXED_APPLY_CASE="$mixed_apply_case" \
      "$test_dir/bin/sh" -c '. "$1"; say() { printf "%s\n" "$*"; }; EX_PREREQ=3; PREREQ_MISSING=$(printf "gnupg2\npodman-compose")
        prereq_mixed_preverify "$2" 10.2 "$4" || exit $?
        prereq_rocky_verified_plan "$2" "$3" 10.2 "$4" || exit $?
        if [ "$RUN_MIXED_APPLY_CASE" = recheck_bad_signature ]; then FAKE_EPEL_SIGNATURE_CASE=wrong_key; export FAKE_EPEL_SIGNATURE_CASE; fi
        if [ "$RUN_MIXED_APPLY_CASE" = recheck_bad_metadata ]; then printf "%s\n" changed >> "$FAKE_STATE_DIR/../mixed-apply-replies/baseos-primary"; fi
        printf "%s\n" PLAN_DONE >> "$FAKE_LOG"
        prereq_rocky_apply_verified "$2" "$3" 10.2 "$4"' \
      sh "$include" "$mixed_stage" "$mixed_root" "$test_dir/mixed-apply-keys/root1.asc" > "$test_dir/out" 2>&1 || actual=$?
    if [ "$actual" != "$expected_status" ]; then printf 'FAIL W1-73 mixed apply: %s returned %s, expected %s (%s)\n' "$mixed_apply_case" "$actual" "$expected_status" "$test_locale" >&2; cat "$test_dir/out" >&2; tail -15 "$test_dir/log" >&2; exit 1; fi
    if [ "$mixed_apply_case" = valid ]; then
      if ! grep -Eiq 'epel.*community|community.*epel' "$test_dir/out" || ! grep -Eiq 'persist.*repositor|repositor.*persist' "$test_dir/out" || ! grep -Eiq 'persist.*key|key.*persist' "$test_dir/out"; then
        printf '%s\n' 'FAIL V-02: EPEL community trust and persistent repository/key effects not disclosed' >&2; exit 1
      fi
      [ -f "$test_dir/state/rocky-installed" ] || { printf '%s\n' 'FAIL W1-73 mixed apply: verified closure was not installed' >&2; exit 1; }
      grep -q 'epel_fingerprint=7D8D15CBFC4E62688591FB2633D98517E37ED158' "$test_dir/out" || { printf '%s\n' 'FAIL W1-73 mixed apply: pinned EPEL fingerprint missing from receipt' >&2; exit 1; }
      grep -q 'epel_repomd_sha256=' "$test_dir/out" || { printf '%s\n' 'FAIL W1-73 mixed apply: EPEL metadata receipt was not retained' >&2; exit 1; }
      awk '/^  sudo / { sub(/^  /, ""); print }' "$test_dir/out" > "$test_dir/planned-sudo"
      awk '/^sudo / { gsub(/[<>]/, ""); print }' "$test_dir/log" > "$test_dir/actual-sudo"
      cmp -s "$test_dir/planned-sudo" "$test_dir/actual-sudo" || { printf '%s\n' 'FAIL W1-73 mixed apply: sudo plan mismatch' >&2; exit 1; }
    elif [ -f "$test_dir/state/rocky-installed" ]; then printf '%s\n' 'FAIL W1-73 mixed apply: swapped archive installed' >&2; exit 1; fi
    [ ! -e "$mixed_root" ] || { printf '%s\n' 'FAIL W1-73 mixed apply: root stage remains' >&2; exit 1; }
    rm -rf "$mixed_stage" "$test_dir/mixed-apply-keys" "$test_dir/mixed-apply-replies"
  done
  if [ -n "$prereq_screen_case" ]; then rm "$test_dir/bin/stat"; mv "$test_dir/bin/stat.real" "$test_dir/bin/stat"; fi
}

prereq_test_bootapply_cases() {
  if [ -n "$prereq_screen_case" ]; then mkdir -p "$test_dir/tree/var/cache"; mv "$test_dir/bin/stat" "$test_dir/bin/stat.real"; ln -s "$root/scripts/fakes/prereq/fake-command" "$test_dir/bin/stat"; fi
  for boot_apply_case in valid first_failure changed_after_first; do
    if [ -n "$prereq_screen_case" ] && [ "$boot_apply_case" != "$prereq_screen_case" ]; then continue; fi
    boot_apply_stage="$test_dir/boot-apply-$boot_apply_case"; boot_apply_root="$test_dir/tree/var/cache/cortex-prereq-1000-452"
    mkdir -m 700 "$boot_apply_stage" "$boot_apply_stage/bootstrap" "$boot_apply_stage/mixed" "$test_dir/boot-apply-keys" "$test_dir/boot-apply-replies"
    python3 "$root/scripts/fakes/prereq/rocky-fixtures.py" "$test_dir/boot-apply-keys"
    printf '%s\n' 'synthetic downloaded archive' > "$test_dir/boot-apply-replies/expected.rpm"
    python3 "$root/scripts/fakes/prereq/rocky-repo-fixtures.py" "$test_dir/boot-apply-replies" "$test_dir/boot-apply-replies/expected.rpm" "$test_dir/boot-apply-replies/expected.rpm" deps
    python3 "$root/scripts/fakes/prereq/repo-fixtures.py" "$test_dir/boot-apply-replies" valid
    printf '%s\n' "$test_dir/boot-apply-replies" > "$test_dir/state/fetch-fixtures"
    printf '%s\n' valid > "$test_dir/state/fetch-case"
    : > "$test_dir/log"; rm -f "$test_dir/state/rocky-installed" "$test_dir/state/bootstrap-installed" "$test_dir/state"/installed.*
    boot_apply_fail=0 boot_apply_swap=0 expected_status=3 actual=0
    case $boot_apply_case in valid) expected_status=0 ;; first_failure) boot_apply_fail=1 ;; changed_after_first) boot_apply_swap=1 ;; esac
    env -i LC_ALL="$test_locale" PATH="$test_dir/bin" HOME="$test_dir/home" PREREQ_ROOT="$test_dir/tree" FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" FAKE_BIN="$test_dir/bin" \
      FAKE_ALLOW_ROOTS=1 FAKE_ALLOW_ROOT_STAT=1 FAKE_STAT_CASE=mixed FAKE_PRIVATE_STAGE="$boot_apply_stage" FAKE_REAL_STAT="$test_dir/bin/stat.real" \
      FAKE_ALLOW_STAGE=1 FAKE_ROOT_STAGE="$boot_apply_root" FAKE_DOWNLOAD_STAGE="$boot_apply_stage/mixed" \
      FAKE_ALLOW_ROCKY_CONFIG=1 FAKE_ROCKY_CASE=metadata_off FAKE_ALLOW_DNF_CONFIG=1 FAKE_REPO_CASE=metadata_off \
      FAKE_ALLOW_ROCKY_DOWNLOAD=1 FAKE_BOOTSTRAP_DOWNLOAD=1 FAKE_EPEL_MIX=1 FAKE_ALLOW_ROCKY_ARCHIVES=1 FAKE_ALLOW_ROCKY_INSTALL=1 FAKE_TWO_PHASE=1 FAKE_BOOT_FAIL="$boot_apply_fail" FAKE_BOOT_SWAP="$boot_apply_swap" \
      FAKE_ALLOW_RPMKEYS=1 FAKE_ALLOW_KEY_READ=1 FAKE_ALLOW_PAYLOAD=1 FAKE_PROVIDER="$test_dir/tree/usr/bin/podman-compose" \
      "$test_dir/bin/sh" -c '. "$1"; say() { printf "%s\n" "$*"; }; EX_PREREQ=3; PREREQ_MISSING=$(printf "gnupg2\npodman-compose")
        FAKE_DOWNLOAD_STAGE=$2/bootstrap; export FAKE_DOWNLOAD_STAGE
        prereq_rocky_preverify "$2/bootstrap" 10.2 extras "$4" || exit $?
        boot=$PREREQ_ROCKY_VERIFIED_MANIFEST; boot_sizes=$PREREQ_ROCKY_VERIFIED_SIZES; boot_policy=$PREREQ_ROCKY_POLICY
        FAKE_DOWNLOAD_STAGE=$2/mixed; export FAKE_DOWNLOAD_STAGE
        prereq_mixed_preverify "$2/mixed" 10.2 "$4" || exit $?
        mixed=$PREREQ_ROCKY_VERIFIED_MANIFEST; mixed_sizes=$PREREQ_ROCKY_VERIFIED_SIZES; mixed_policy=$PREREQ_ROCKY_POLICY
        prereq_combine_bootstrap "$2" "$boot" "$mixed" || exit $?
        PREREQ_RECHECK_BOOT_STAGE=$2/bootstrap PREREQ_RECHECK_BOOT_MANIFEST=$boot PREREQ_RECHECK_BOOT_SIZES=$boot_sizes PREREQ_RECHECK_BOOT_POLICY=$boot_policy
        PREREQ_RECHECK_MIXED_STAGE=$2/mixed PREREQ_RECHECK_MIXED_MANIFEST=$mixed PREREQ_RECHECK_MIXED_SIZES=$mixed_sizes PREREQ_RECHECK_MIXED_POLICY=$mixed_policy
        prereq_rocky_verified_plan "$2" "$3" 10.2 "$4" || exit $?
        printf "%s\n" PLAN_DONE >> "$FAKE_LOG"
        prereq_rocky_apply_verified "$2" "$3" 10.2 "$4"' \
      sh "$include" "$boot_apply_stage" "$boot_apply_root" "$test_dir/boot-apply-keys/root1.asc" > "$test_dir/out" 2>&1 || actual=$?
    if [ "$actual" != "$expected_status" ]; then printf 'FAIL W1-73 ordered bootstrap: %s returned %s, expected %s (%s)\n' "$boot_apply_case" "$actual" "$expected_status" "$test_locale" >&2; cat "$test_dir/out" >&2; tail -15 "$test_dir/log" >&2; exit 1; fi
    if [ "$boot_apply_case" = valid ]; then
      [ -f "$test_dir/state/bootstrap-installed" ] && [ -f "$test_dir/state/rocky-installed" ] || { printf '%s\n' 'FAIL W1-73 ordered bootstrap: both groups did not install' >&2; exit 1; }
      awk '/^  sudo / { sub(/^  /, ""); print }' "$test_dir/out" > "$test_dir/planned-sudo"
      awk '/^sudo / { gsub(/[<>]/, ""); print }' "$test_dir/log" > "$test_dir/actual-sudo"
      cmp -s "$test_dir/planned-sudo" "$test_dir/actual-sudo" || { printf '%s\n' 'FAIL W1-73 ordered bootstrap: sudo plan mismatch' >&2; exit 1; }
    elif [ -f "$test_dir/state/rocky-installed" ]; then printf '%s\n' 'FAIL W1-73 ordered bootstrap: second group ran after failure or swap' >&2; exit 1; fi
    [ ! -e "$boot_apply_root" ] || { printf '%s\n' 'FAIL W1-73 ordered bootstrap: root stage remains' >&2; exit 1; }
    rm -rf "$boot_apply_stage" "$test_dir/boot-apply-keys" "$test_dir/boot-apply-replies"
  done
  printf 'ok W1-73 ordered bootstrap and locked second group (%s)\n' "$test_locale"
  if [ -n "$prereq_screen_case" ]; then rm "$test_dir/bin/stat"; mv "$test_dir/bin/stat.real" "$test_dir/bin/stat"; fi
}

prereq_test_real_cases() {
  for pq_real_case in A B C D E F G L ORDER; do
    if [ -n "$prereq_screen_case" ] && [ "$pq_real_case" != "$prereq_screen_case" ]; then continue; fi
    case $pq_real_case in
      A) set -- Rocky102.test_A_real_rocky_dump_without_metalink ;;
      B) set -- Rocky102.test_B_real_rocky_section Rocky102.test_repo_headers_and_optional_fields_remain_strict Rocky102.test_epel_headers_remain_strict ;;
      C) set -- Rocky102.test_C_rocky_primary_observed_length Rocky102.test_C_limit_and_checksum_stay_enforced ;;
      D) set -- Rocky102.test_D_real_epel_dump_without_mirrorlist ;;
      E) set -- Rocky102.test_E_real_rpm_version_characters ;;
      F) set -- Rocky102.test_F_real_metalink_current_hash Rocky102.test_F_duplicate_current_digest_rejected Rocky102.test_F_alternate_does_not_rescue_wrong_or_missing_current_digest ;;
      G|L|ORDER) set -- ;;
    esac
    pq_real_test="$root/scripts/install-prereq-real-test.py"
    if [ "$pq_real_case" = G ]; then pq_real_test="$root/scripts/fakes/prereq/zstd-guards-test.py"; fi
    if [ "$pq_real_case" = L ]; then pq_real_test="$root/scripts/fakes/prereq/rocky-floor-test.py"; fi
    if [ "$pq_real_case" = ORDER ]; then pq_real_test="$root/scripts/fakes/prereq/locale-order-test.py"; fi
    if ! LC_ALL="$test_locale" PREREQ_TEST_SOURCE="$original_include" PREREQ_TEST_TMP="$scratch" \
      python3 -I -B "$pq_real_test" "$@" > "$test_dir/real-result" 2>&1; then
      printf 'FAIL IP-real-%s: captured prerequisite data (%s)\n' "$pq_real_case" "$test_locale" >&2
      cat "$test_dir/real-result" >&2
      exit 1
    fi
    if [ "$pq_real_case" = C ] && ! LC_ALL="$test_locale" PREREQ_TEST_TMP="$scratch" \
      python3 -I -B "$root/scripts/fakes/prereq/rocky-large-metadata-test.py" "$original_include" > "$test_dir/real-large-result" 2>&1; then
      printf 'FAIL IP-real-C: full-path expanded metadata (%s)\n' "$test_locale" >&2
      cat "$test_dir/real-large-result" >&2
      exit 1
    fi
  done
  printf 'ok IP-real captured prerequisite data (%s)\n' "$test_locale"
}

if [ -n "$prereq_screen_case" ]; then
  # These later cells use the fake provider tree prepared earlier by the full suite.
  case $prereq_screen_kind in candidate|flow|run|repairrun|approved|mixedapply|bootapply)
    mkdir -p "$test_dir/tree/usr/bin" "$test_dir/tree/etc/pki/rpm-gpg"
    printf 'ID=rocky\nVERSION_ID=10.2\n' > "$test_dir/tree/etc/os-release"
    printf '%s\n' '#!/bin/sh' 'exit 95' > "$test_dir/tree/usr/bin/podman-compose"
    chmod 755 "$test_dir/tree/usr/bin/podman-compose"
    : > "$test_dir/tree/etc/pki/rpm-gpg/RPM-GPG-KEY-EPEL-10"
    if [ "$prereq_screen_kind" = approved ]; then mkdir -p "$test_dir/tree/var/cache"; fi
    if [ "$prereq_screen_kind" = repairrun ]; then : > "$test_dir/tree/.fake-root"; fi
  ;; esac
  set -- ${PREREQ_TEST_LOCALES:-C C.utf8}
  test_locale=$1
  case $prereq_screen_kind in real) prereq_test_real_cases ;; os) prereq_test_os_cases ;; machine) prereq_test_machine_cases ;; range) prereq_test_range_cases ;; repair) prereq_test_repair_cases ;; subid) prereq_test_subid_cases ;; version) prereq_test_version_cases ;; message) prereq_test_message_cases ;; provider) prereq_test_provider_cases ;; completion) prereq_test_completion_cases ;; key) prereq_test_key_cases ;; signature) prereq_test_signature_cases ;; payload) prereq_test_payload_cases ;; fetch) prereq_test_fetch_cases ;; qualify) prereq_test_qualify_cases ;; candidate) prereq_test_candidate_cases ;; extract) prereq_test_extract_cases ;; combine) prereq_test_combine_cases ;; metadata) prereq_test_metadata_cases ;; pre) prereq_test_pre_cases ;; nocurl) prereq_test_nocurl_cases ;; flow) prereq_test_flow_cases ;; run) prereq_test_run_cases ;; repairrun) prereq_test_repairrun_cases ;; approved) prereq_test_approved_cases ;; mixedapply) prereq_test_mixedapply_cases ;; bootapply) prereq_test_bootapply_cases ;; esac
  printf 'SCREEN %s: survived\n' "$PREREQ_SCREEN_MUTANT"
  exit 0
fi

for test_locale in ${PREREQ_TEST_LOCALES:-C C.utf8}; do
  if ! LC_ALL=C grep -Fx -- "$test_locale" "$test_dir/locale.available" >/dev/null ||
     ! LC_ALL="$test_locale" locale charmap > "$test_dir/locale.charmap" 2> "$test_dir/locale.error" ||
     [ -s "$test_dir/locale.error" ]; then
    printf 'FAIL IP-53: locale matrix lost requested locale %s\n' "$test_locale" >&2
    exit 1
  fi
  prereq_test_real_cases

  prereq_test_range_cases

  prereq_test_repair_cases

  for linger_case in already user sudo fail noop; do
    rm -f "$test_dir/state/linger"
    : > "$test_dir/log"
    linger_state=no linger_mode=$linger_case expected_status=0
    case $linger_case in
      already) linger_state=yes ;;
      fail|noop) expected_status=3 ;;
    esac
    actual=0
    env -i LC_ALL="$test_locale" PATH="$test_dir/bin" FAKE_BIN="$test_dir/bin" \
      FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" FAKE_ALLOW_LINGER=1 \
      FAKE_LINGER="$linger_state" FAKE_LINGER_MODE="$linger_mode" \
      "$test_dir/bin/sh" -c '. "$1"; EX_PREREQ=3; prereq_fix_linger cortexuser' \
      sh "$include" > "$test_dir/out" 2>&1 || actual=$?
    if [ "$actual" != "$expected_status" ]; then
      printf 'FAIL IP-36: %s returned %s, expected %s (%s)\n' "$linger_case" "$actual" "$expected_status" "$test_locale" >&2; exit 1
    fi
    if [ "$linger_case" = already ] && grep -q '<enable-linger>' "$test_dir/log"; then
      printf '%s\n' 'FAIL IP-36: already enabled linger was changed' >&2; exit 1
    fi
    if [ "$linger_case" = sudo ] && ! awk '
      /^loginctl <enable-linger>/ && !user { user = NR }
      /^sudo <loginctl> <enable-linger>/ && !sudo { sudo = NR }
      /^loginctl <show-user>/ { lastshow = NR }
      END { if (!user || !sudo || user >= sudo || lastshow <= sudo) exit 1 }
    ' "$test_dir/log"; then
      printf '%s\n' 'FAIL IP-36: linger fallback order or recheck was wrong' >&2; exit 1
    fi
  done
  rm -f "$test_dir/state/linger"
  printf 'ok IP-36 linger repair and fallback (%s)\n' "$test_locale"
  prereq_test_key_cases

  prereq_test_signature_cases

  prereq_test_payload_cases

  for repo_case in valid metadata_off testing wrong_minor wrong_arch foreign gpg_off tls_off baseurl mirrorlist custom_ca duplicate; do
    : > "$test_dir/log"
    actual=0; expected_status=1
    [ "$repo_case" != valid ] || expected_status=0
    [ "$repo_case" != metadata_off ] || expected_status=0
    env -i LC_ALL="$test_locale" PATH="$test_dir/bin" FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" \
      FAKE_ALLOW_DNF_CONFIG=1 FAKE_REPO_CASE="$repo_case" "$test_dir/bin/sh" -c \
      '. "$1"; prereq_epel_repo_config_ok 10.2 || exit $?; printf "METADATA=%s\nURL=%s\n" "$PREREQ_EPEL_REPO_GPGCHECK" "$PREREQ_EPEL_METALINK"' sh "$include" > "$test_dir/out" 2>&1 || actual=$?
    if [ "$actual" != "$expected_status" ]; then
      printf 'FAIL IP-compose-trust: %s repo config returned %s, expected %s (%s)\n' "$repo_case" "$actual" "$expected_status" "$test_locale" >&2; exit 1
    fi
    if [ "$repo_case" = valid ] && ! grep -q '^METADATA=1$' "$test_dir/out"; then
      printf '%s\n' 'FAIL IP-compose-trust: enabled metadata signatures were not retained' >&2; exit 1
    fi
    if [ "$repo_case" = metadata_off ] && ! grep -q '^METADATA=0$' "$test_dir/out"; then
      printf '%s\n' 'FAIL IP-compose-trust: metadata signature setting was changed' >&2; exit 1
    fi
  done
  printf 'ok IP-compose-trust stable EPEL configuration (%s)\n' "$test_locale"
  for private_repo_case in valid foreign missing; do
    private_repo_stage="$test_dir/private-epel-$private_repo_case"
    mkdir -m 700 "$private_repo_stage" "$test_dir/private-epel-$private_repo_case/epel-config" "$test_dir/private-epel-$private_repo_case/pinned"
    printf '%s\n' '[epel]' > "$private_repo_stage/epel-config/epel.repo"
    printf '%s\n' 'synthetic pinned key' > "$private_repo_stage/pinned/certificate.key"
    [ "$private_repo_case" != missing ] || rm "$private_repo_stage/epel-config/epel.repo"
    private_repo_mode=valid expected_status=1 actual=0
    [ "$private_repo_case" != valid ] || expected_status=0
    [ "$private_repo_case" != foreign ] || private_repo_mode=foreign
    env -i LC_ALL="$test_locale" PATH="$test_dir/bin" PREREQ_ROOT="$test_dir/tree" FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" \
      FAKE_ALLOW_DNF_CONFIG=1 FAKE_REPO_CASE="$private_repo_mode" FAKE_PRIVATE_EPEL_STAGE="$private_repo_stage" \
      "$test_dir/bin/sh" -c '. "$1"; prereq_epel_repo_config_ok 10.2 "$2"' sh "$include" "$private_repo_stage" > "$test_dir/out" 2>&1 || actual=$?
    if [ "$actual" != "$expected_status" ]; then printf 'FAIL W1-73 private EPEL config: %s returned %s, expected %s (%s)\n' "$private_repo_case" "$actual" "$expected_status" "$test_locale" >&2; exit 1; fi
    rm -rf "$private_repo_stage"
  done
  printf 'ok W1-73 private EPEL repository config (%s)\n' "$test_locale"
  for rocky_case in valid metadata_off foreign gpg_off tls_off baseurl wrong_repo wrong_key; do
    for rocky_repo in baseos appstream crb extras; do
      actual=0; expected_status=1
      [ "$rocky_case" != valid ] || expected_status=0
      [ "$rocky_case" != metadata_off ] || expected_status=0
      env -i LC_ALL="$test_locale" PATH="$test_dir/bin" FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" \
        FAKE_ALLOW_ROCKY_CONFIG=1 FAKE_ROCKY_CASE="$rocky_case" "$test_dir/bin/sh" -c \
        '. "$1"; prereq_rocky_repo_ok 10.2 "$2" || exit $?; printf "METADATA=%s\n" "$PREREQ_ROCKY_REPO_GPGCHECK"' sh "$include" "$rocky_repo" > "$test_dir/out" 2>&1 || actual=$?
      if [ "$actual" != "$expected_status" ]; then
        printf 'FAIL IP-rocky-trust: %s %s returned %s, expected %s (%s)\n' "$rocky_case" "$rocky_repo" "$actual" "$expected_status" "$test_locale" >&2; exit 1
      fi
      if [ "$rocky_case" = valid ] && ! grep -q '^METADATA=1$' "$test_dir/out"; then
        printf '%s\n' 'FAIL IP-rocky-trust: enabled metadata signatures were not retained' >&2; exit 1
      fi
      if [ "$rocky_case" = metadata_off ] && ! grep -q '^METADATA=0$' "$test_dir/out"; then
        printf '%s\n' 'FAIL IP-rocky-trust: metadata signature setting was changed' >&2; exit 1
      fi
    done
  done
  printf 'ok IP-rocky-trust expected Rocky repository settings (%s)\n' "$test_locale"
  mkdir -p "$test_dir/rocky-roots"
  python3 "$root/scripts/fakes/prereq/rocky-fixtures.py" "$test_dir/rocky-roots"
  mv "$test_dir/bin/gpg" "$test_dir/bin/gpg.saved"
  for roots_case in valid rotated missing foreign modified ghost no_digest weak_hash malformed; do
    mkdir -p "$test_dir/roots-$roots_case"
    chmod 700 "$test_dir/roots-$roots_case"
    : > "$test_dir/log"
    actual=0; expected_status=1
    root1="$test_dir/rocky-roots/root1.asc"; root2=''
    case $roots_case in
      valid) expected_status=0 ;;
      rotated) root2="$test_dir/rocky-roots/root2.asc"; expected_status=0 ;;
      missing) root1="$test_dir/rocky-roots/missing.asc" ;;
      malformed) root1="$test_dir/rocky-roots/fingerprints" ;;
    esac
    env -i LC_ALL="$test_locale" PATH="$test_dir/bin" FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" \
      FAKE_ALLOW_ROOTS=1 FAKE_ROOT_CASE="$roots_case" "$test_dir/bin/sh" -c '
        . "$1"; shift; stage=$1; shift; if [ -n "$2" ]; then prereq_rocky_roots "$stage" "$1" "$2"; else prereq_rocky_roots "$stage" "$1"; fi' \
      sh "$include" "$test_dir/roots-$roots_case" "$root1" "$root2" > "$test_dir/out" 2>&1 || actual=$?
    if [ "$actual" != "$expected_status" ]; then
      printf 'FAIL IP-rocky-trust: %s root set returned %s, expected %s (%s)\n' "$roots_case" "$actual" "$expected_status" "$test_locale" >&2; exit 1
    fi
    if [ "$expected_status" = 0 ]; then
      expected_count=1; [ "$roots_case" != rotated ] || expected_count=2
      if [ "$(grep -c '^fingerprint=' "$test_dir/roots-$roots_case/roots.receipt")" != "$expected_count" ]; then
        printf '%s\n' 'FAIL IP-rocky-trust: root identity receipt is incomplete' >&2; exit 1
      fi
      awk -F= '/^fingerprint=/ { print $2 }' "$test_dir/roots-$roots_case/roots.receipt" > "$test_dir/actual-fingerprints"
      head -n "$expected_count" "$test_dir/rocky-roots/fingerprints" > "$test_dir/expected-fingerprints"
      if ! cmp -s "$test_dir/actual-fingerprints" "$test_dir/expected-fingerprints"; then
        printf '%s\n' 'FAIL IP-rocky-trust: root fingerprint receipt differs from the key bytes' >&2; exit 1
      fi
    fi
    if grep -Eq '^(gpg|gpgv|sudo|dnf|curl) ' "$test_dir/log"; then
      printf '%s\n' 'FAIL IP-rocky-trust: root inspection used GPG, network or mutation' >&2; exit 1
    fi
  done
  mv "$test_dir/bin/gpg.saved" "$test_dir/bin/gpg"
  rm -rf "$test_dir"/roots-* "$test_dir/rocky-roots"
  printf 'ok IP-rocky-trust pre-existing rotated root set without GPG (%s)\n' "$test_locale"
  mv "$test_dir/bin/stat" "$test_dir/bin/stat.real"
  ln -s "$root/scripts/fakes/prereq/fake-command" "$test_dir/bin/stat"
  for seal_case in valid rotated altered extra missing; do
    mkdir -p "$test_dir/seal-$seal_case/rocky-ring" "$test_dir/seal-private-$seal_case" "$test_dir/seal-keys-$seal_case"
    python3 "$root/scripts/fakes/prereq/rocky-fixtures.py" "$test_dir/seal-keys-$seal_case"
    cp "$test_dir/seal-keys-$seal_case/root1.asc" "$test_dir/seal-$seal_case/rocky-ring/root-1.key"
    root2=''; expected_status=1; actual=0
    case $seal_case in
      valid) expected_status=0 ;;
      rotated) root2="$test_dir/seal-keys-$seal_case/root2.asc"; cp "$root2" "$test_dir/seal-$seal_case/rocky-ring/root-2.key"; expected_status=0 ;;
      altered) printf '%s\n' changed >> "$test_dir/seal-$seal_case/rocky-ring/root-1.key" ;;
      extra) cp "$test_dir/seal-keys-$seal_case/root2.asc" "$test_dir/seal-$seal_case/rocky-ring/root-2.key" ;;
      missing) rm "$test_dir/seal-$seal_case/rocky-ring/root-1.key" ;;
    esac
    env -i LC_ALL="$test_locale" PATH="$test_dir/bin" FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" \
      FAKE_ALLOW_ROOTS=1 FAKE_ALLOW_ROOT_STAT=1 "$test_dir/bin/sh" -c '
        . "$1"; shift; sealed=$1; private=$2; shift 2
        if [ -n "$2" ]; then prereq_rocky_seal_ok "$sealed" "$private" "$1" "$2"; else prereq_rocky_seal_ok "$sealed" "$private" "$1"; fi' \
      sh "$include" "$test_dir/seal-$seal_case" "$test_dir/seal-private-$seal_case" "$test_dir/seal-keys-$seal_case/root1.asc" "$root2" \
      > "$test_dir/out" 2>&1 || actual=$?
    if [ "$actual" != "$expected_status" ]; then
      printf 'FAIL IP-rocky-trust: %s sealed roots returned %s, expected %s (%s)\n' "$seal_case" "$actual" "$expected_status" "$test_locale" >&2; exit 1
    fi
  done
  rm "$test_dir/bin/stat"
  mv "$test_dir/bin/stat.real" "$test_dir/bin/stat"
  rm -rf "$test_dir"/seal-* "$test_dir"/seal-private-* "$test_dir"/seal-keys-*
  printf 'ok IP-rocky-trust sealed root set matches OS-owned keys (%s)\n' "$test_locale"
  mv "$test_dir/bin/stat" "$test_dir/bin/stat.real"
  ln -s "$root/scripts/fakes/prereq/fake-command" "$test_dir/bin/stat"
  mkdir -p "$test_dir/tree/var/cache"
  for stage_case in valid existing fail_children; do
    stage="$test_dir/tree/var/cache/cortex-prereq-1000-$stage_case"
    # The production name contains digits only; use separate numeric fixtures.
    case $stage_case in valid) stage="$test_dir/tree/var/cache/cortex-prereq-1000-101" ;; existing) stage="$test_dir/tree/var/cache/cortex-prereq-1000-102" ;; fail_children) stage="$test_dir/tree/var/cache/cortex-prereq-1000-103" ;; esac
    mkdir -p "$test_dir/stage-private-$stage_case" "$test_dir/stage-keys-$stage_case"
    python3 "$root/scripts/fakes/prereq/rocky-fixtures.py" "$test_dir/stage-keys-$stage_case"
    expected_status=3 actual=0 stage_fail=0
    case $stage_case in
      valid) expected_status=0 ;;
      existing) mkdir "$stage"; printf '%s\n' preserve > "$stage/owner-file" ;;
      fail_children) stage_fail=children ;;
    esac
    : > "$test_dir/log"
    env -i LC_ALL="$test_locale" PATH="$test_dir/bin" PREREQ_ROOT="$test_dir/tree" FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" FAKE_BIN="$test_dir/bin" \
      FAKE_ALLOW_ROOTS=1 FAKE_ALLOW_ROOT_STAT=1 FAKE_ALLOW_STAGE=1 FAKE_ROOT_STAGE="$stage" FAKE_STAGE_FAIL="$stage_fail" \
      "$test_dir/bin/sh" -c '. "$1"; EX_PREREQ=3; prereq_prepare_root_stage "$2" "$3" "$4"' \
      sh "$include" "$stage" "$test_dir/stage-private-$stage_case" "$test_dir/stage-keys-$stage_case/root1.asc" > "$test_dir/out" 2>&1 || actual=$?
    if [ "$actual" != "$expected_status" ]; then
      printf 'FAIL IP-rocky-trust: %s stage returned %s, expected %s (%s)\n' "$stage_case" "$actual" "$expected_status" "$test_locale" >&2; exit 1
    fi
    if [ "$stage_case" = valid ] && [ ! -f "$stage/rocky-ring/root-1.key" ]; then printf '%s\n' 'FAIL IP-rocky-trust: stage lost its sealed root' >&2; exit 1; fi
    if [ "$stage_case" = existing ] && { [ ! -f "$stage/owner-file" ] || grep -q '^sudo <rm>' "$test_dir/log"; }; then printf '%s\n' 'FAIL IP-rocky-trust: existing stage was touched' >&2; exit 1; fi
    if [ "$stage_case" = fail_children ] && [ -e "$stage" ]; then printf '%s\n' 'FAIL IP-rocky-trust: incomplete newly created stage was not cleaned' >&2; exit 1; fi
    rm -rf "$stage"
  done
  rm "$test_dir/bin/stat"
  mv "$test_dir/bin/stat.real" "$test_dir/bin/stat"
  rm -rf "$test_dir"/stage-private-* "$test_dir"/stage-keys-*
  printf 'ok IP-rocky-trust safe root-owned stage preparation (%s)\n' "$test_locale"
  mv "$test_dir/bin/stat" "$test_dir/bin/stat.real"
  ln -s "$root/scripts/fakes/prereq/fake-command" "$test_dir/bin/stat"
  for candidate_case in valid plus private_plus private private_unowned unowned writable unknown empty; do
    mkdir -p "$test_dir/candidates-$candidate_case/packages"
    case $candidate_case in
      empty) ;;
      unknown) printf '%s\n' unknown > "$test_dir/candidates-$candidate_case/packages/unknown.rpm" ;;
      plus|private_plus) printf '%s\n' one > "$test_dir/candidates-$candidate_case/packages/dependency+one.rpm" ;;
      *) printf '%s\n' one > "$test_dir/candidates-$candidate_case/packages/one.rpm" ;;
    esac
    actual=0; expected_status=1; candidate_mode=root; stat_case=$candidate_case
    case $candidate_case in valid|plus) expected_status=0 ;; private|private_plus) candidate_mode=private; stat_case=private; expected_status=0 ;; private_unowned) candidate_mode=private; stat_case=valid ;; esac
    env -i LC_ALL="$test_locale" PATH="$test_dir/bin" FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" \
      FAKE_ALLOW_ROOT_STAT=1 FAKE_STAT_CASE="$stat_case" FAKE_ALLOW_ROCKY_ARCHIVES=1 \
      "$test_dir/bin/sh" -c '. "$1"; prereq_rocky_candidates "$2" "$3"' sh "$include" "$test_dir/candidates-$candidate_case" "$candidate_mode" \
      > "$test_dir/out" 2>&1 || actual=$?
    if [ "$actual" != "$expected_status" ]; then
      printf 'FAIL IP-rocky-trust: %s candidate inventory returned %s, expected %s (%s)\n' "$candidate_case" "$actual" "$expected_status" "$test_locale" >&2; exit 1
    fi
    if [ "$expected_status" = 0 ] && ! grep -q '|gnupg2|0|2.4.5|1.el10|x86_64$' "$test_dir/out"; then
      printf '%s\n' 'FAIL IP-rocky-trust: candidate identity missing' >&2; exit 1
    fi
  done
  rm "$test_dir/bin/stat"
  mv "$test_dir/bin/stat.real" "$test_dir/bin/stat"
  rm -rf "$test_dir"/candidates-*
  printf 'ok IP-rocky-trust root-owned candidate inventory (%s)\n' "$test_locale"
  mv "$test_dir/bin/stat" "$test_dir/bin/stat.real"
  ln -s "$root/scripts/fakes/prereq/fake-command" "$test_dir/bin/stat"
  for archives_case in valid plus private_plus private private_unowned rotated foreign unsigned digest_only checksum nevra origin swap missing unowned writable unlisted; do
    mkdir -p "$test_dir/archives-$archives_case/rocky-ring" "$test_dir/archives-$archives_case/empty" "$test_dir/archives-$archives_case/packages"
    chmod 700 "$test_dir/archives-$archives_case"
    : > "$test_dir/archives-$archives_case/rocky-ring/root-1.key"
    [ "$archives_case" != rotated ] || : > "$test_dir/archives-$archives_case/rocky-ring/root-2.key"
    archive_one=one.rpm
    case $archives_case in plus|private_plus) archive_one=dependency+one.rpm ;; esac
    printf '%s\n' 'synthetic Rocky package one' > "$test_dir/archives-$archives_case/packages/$archive_one"
    printf '%s\n' 'synthetic Rocky package two' > "$test_dir/archives-$archives_case/packages/two.rpm"
    hash1=$(sha256sum "$test_dir/archives-$archives_case/packages/$archive_one"); hash1=${hash1%% *}
    hash2=$(sha256sum "$test_dir/archives-$archives_case/packages/two.rpm"); hash2=${hash2%% *}
    archive_repo=extras; signature_case=valid; actual=0; expected_status=1; archive_mode=root; stat_case=$archives_case
    case $archives_case in
      valid|plus|rotated) expected_status=0 ;;
      private|private_plus) archive_mode=private; stat_case=private; expected_status=0 ;;
      private_unowned) archive_mode=private; stat_case=valid ;;
      foreign) signature_case=wrong_key ;;
      unsigned) signature_case=unsigned ;;
      digest_only) signature_case=digest_only ;;
      checksum) hash1=0000000000000000000000000000000000000000000000000000000000000000 ;;
      origin) archive_repo=baseos ;;
      swap) signature_case=swap ;;
      missing) rm "$test_dir/archives-$archives_case/packages/two.rpm" ;;
      unlisted) printf '%s\n' 'unverified extra archive' > "$test_dir/archives-$archives_case/packages/extra.rpm" ;;
    esac
    printf '%s|%s|gnupg2|0|2.4.5|1.el10|x86_64|baseos\n%s|%s|epel-release|0|10|8.el10|noarch|%s\n' \
      "$test_dir/archives-$archives_case/packages/$archive_one" "$hash1" "$test_dir/archives-$archives_case/packages/two.rpm" "$hash2" "$archive_repo" \
      > "$test_dir/archives-$archives_case/transaction.manifest"
    env -i LC_ALL="$test_locale" PATH="$test_dir/bin" FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" \
      FAKE_ALLOW_ROCKY_ARCHIVES=1 FAKE_ARCHIVE_CASE="$archives_case" FAKE_ALLOW_RPMKEYS=1 FAKE_ALLOW_ROOT_STAT=1 FAKE_STAT_CASE="$stat_case" FAKE_SIGNATURE_CASE="$signature_case" \
      "$test_dir/bin/sh" -c '. "$1"; prereq_rocky_archives_ok "$2" "$3" "$4"' sh "$include" \
      "$test_dir/archives-$archives_case" "$test_dir/archives-$archives_case/transaction.manifest" "$archive_mode" > "$test_dir/out" 2>&1 || actual=$?
    if [ "$actual" != "$expected_status" ]; then
      printf 'FAIL IP-rocky-trust: %s archive set returned %s, expected %s (%s)\n' "$archives_case" "$actual" "$expected_status" "$test_locale" >&2; exit 1
    fi
  done
  rm "$test_dir/bin/stat"
  mv "$test_dir/bin/stat.real" "$test_dir/bin/stat"
  rm -rf "$test_dir"/archives-*
  printf 'ok IP-rocky-trust every bootstrap archive and swap check (%s)\n' "$test_locale"
  prereq_test_metadata_cases
  mv "$test_dir/bin/stat" "$test_dir/bin/stat.real"
  ln -s "$root/scripts/fakes/prereq/fake-command" "$test_dir/bin/stat"
  for transaction_case in valid altered_root bad_signature bad_metadata extra swap sudo_fail; do
    mkdir -p "$test_dir/tx-$transaction_case/packages" "$test_dir/tx-$transaction_case/rocky-ring" "$test_dir/tx-$transaction_case/empty" \
      "$test_dir/tx-private-$transaction_case" "$test_dir/tx-keys-$transaction_case" "$test_dir/tx-replies-$transaction_case"
    python3 "$root/scripts/fakes/prereq/rocky-fixtures.py" "$test_dir/tx-keys-$transaction_case"
    cp "$test_dir/tx-keys-$transaction_case/root1.asc" "$test_dir/tx-$transaction_case/rocky-ring/root-1.key"
    printf '%s\n' 'synthetic Rocky package one' > "$test_dir/tx-$transaction_case/packages/one.rpm"
    printf '%s\n' 'synthetic Rocky package two' > "$test_dir/tx-$transaction_case/packages/two.rpm"
    hash1=$(sha256sum "$test_dir/tx-$transaction_case/packages/one.rpm"); hash1=${hash1%% *}
    hash2=$(sha256sum "$test_dir/tx-$transaction_case/packages/two.rpm"); hash2=${hash2%% *}
    printf '%s|%s|gnupg2|0|2.4.5|1.el10|x86_64|baseos\n%s|%s|epel-release|0|10|8.el10|noarch|extras\n' \
      "$test_dir/tx-$transaction_case/packages/one.rpm" "$hash1" "$test_dir/tx-$transaction_case/packages/two.rpm" "$hash2" \
      > "$test_dir/tx-$transaction_case/transaction.manifest"
    signature_case=valid fixture_case=valid sudo_fail=0 expected_status=3; actual=0
    case $transaction_case in
      valid) expected_status=0 ;;
      altered_root) printf '%s\n' altered >> "$test_dir/tx-$transaction_case/rocky-ring/root-1.key" ;;
      bad_signature) signature_case=unsigned ;;
      bad_metadata) fixture_case=wrong_hash ;;
      extra) printf '%s\n' extra > "$test_dir/tx-$transaction_case/packages/extra.rpm" ;;
      swap) signature_case=swap ;;
      sudo_fail) sudo_fail=1 ;;
    esac
    python3 "$root/scripts/fakes/prereq/rocky-repo-fixtures.py" "$test_dir/tx-replies-$transaction_case" \
      "$test_dir/tx-$transaction_case/packages/one.rpm" "$test_dir/tx-$transaction_case/packages/two.rpm" "$fixture_case"
    : > "$test_dir/tx-replies-$transaction_case/metalink"
    printf '%s\n' "$test_dir/tx-replies-$transaction_case" > "$test_dir/state/fetch-fixtures"
    printf '%s\n' valid > "$test_dir/state/fetch-case"
    rm -f "$test_dir/state/rocky-installed" "$test_dir/state"/installed.*; : > "$test_dir/log"
    env -i LC_ALL="$test_locale" PATH="$test_dir/bin" FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" FAKE_BIN="$test_dir/bin" \
      FAKE_ALLOW_ROOTS=1 FAKE_ALLOW_ROOT_STAT=1 FAKE_ALLOW_ROCKY_ARCHIVES=1 FAKE_ALLOW_RPMKEYS=1 FAKE_SIGNATURE_CASE="$signature_case" \
      FAKE_ALLOW_ROCKY_INSTALL=1 FAKE_ROOT_STAGE="$test_dir/tx-$transaction_case" FAKE_SUDO_FAIL="$sudo_fail" \
      "$test_dir/bin/sh" -c '. "$1"; say() { printf "%s\n" "$*"; }; EX_PREREQ=3; prereq_rocky_install "$2" "$3" 10.2 "baseos=0,extras=0" "$4"' \
      sh "$include" "$test_dir/tx-$transaction_case" "$test_dir/tx-private-$transaction_case" "$test_dir/tx-keys-$transaction_case/root1.asc" \
      > "$test_dir/out" 2>&1 || actual=$?
    if [ "$actual" != "$expected_status" ]; then
      printf 'FAIL IP-rocky-trust: %s transaction returned %s, expected %s (%s)\n' "$transaction_case" "$actual" "$expected_status" "$test_locale" >&2; exit 1
    fi
    if [ "$transaction_case" = valid ] && [ ! -f "$test_dir/state/rocky-installed" ]; then
      printf '%s\n' 'FAIL IP-rocky-trust: verified transaction was not installed' >&2; exit 1
    fi
    if [ "$transaction_case" != valid ] && [ -f "$test_dir/state/rocky-installed" ]; then
      printf '%s\n' 'FAIL IP-rocky-trust: unverified transaction was installed' >&2; exit 1
    fi
    if [ "$transaction_case" != valid ] && [ "$transaction_case" != sudo_fail ] && grep -q '^sudo ' "$test_dir/log"; then
      printf '%s\n' 'FAIL IP-rocky-trust: sudo ran before transaction verification' >&2; exit 1
    fi
  done
  rm "$test_dir/bin/stat"
  mv "$test_dir/bin/stat.real" "$test_dir/bin/stat"
  rm -rf "$test_dir"/tx-* "$test_dir/state/rocky-installed" "$test_dir/state"/installed.*
  printf 'ok IP-rocky-trust verify-before-install transaction (%s)\n' "$test_locale"
  for download_case in main extras mixed failure unknown; do
    mkdir -p "$test_dir/download-$download_case/packages"
    chmod 700 "$test_dir/download-$download_case"
    : > "$test_dir/log"
    download_mode=main download_fail=0 download_missing=$(printf 'gzip\ngnupg2\npodman-compose (requalify)') expected_status=0 actual=0
    case $download_case in
      extras) download_mode=extras ;;
      mixed) download_mode=mixed; download_missing=$(printf 'gzip\npodman-compose') ;;
      failure) download_fail=1; expected_status=3 ;;
      unknown) download_missing=$(printf 'gzip\n--nogpgcheck\npodman-compose'); expected_status=3 ;;
    esac
    env -i LC_ALL="$test_locale" PATH="$test_dir/bin" FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" FAKE_BIN="$test_dir/bin" \
      FAKE_ALLOW_ROOT_STAT=1 FAKE_ALLOW_ROCKY_DOWNLOAD=1 FAKE_ROOT_STAGE="$test_dir/download-$download_case" FAKE_DOWNLOAD_FAIL="$download_fail" \
      RUN_MISSING="$download_missing" "$test_dir/bin/sh" -c '. "$1"; EX_PREREQ=3; PREREQ_MISSING=$RUN_MISSING; prereq_rocky_download "$2" 10.2 "$3"' \
      sh "$include" "$test_dir/download-$download_case" "$download_mode" > "$test_dir/out" 2>&1 || actual=$?
    if [ "$actual" != "$expected_status" ]; then
      printf 'FAIL IP-rocky-trust: %s download returned %s, expected %s (%s)\n' "$download_case" "$actual" "$expected_status" "$test_locale" >&2; exit 1
    fi
    if [ "$expected_status" = 0 ] && [ ! -f "$test_dir/download-$download_case/packages/one.rpm" ]; then
      printf '%s\n' 'FAIL IP-rocky-trust: download produced no staged archive' >&2; exit 1
    fi
    if grep -Eq '^dnf .*<(install|reinstall|upgrade|update|--nogpgcheck)>' "$test_dir/log"; then
      printf '%s\n' 'FAIL IP-rocky-trust: download used a mutating or insecure DNF form' >&2; exit 1
    fi
    if grep -q '^sudo ' "$test_dir/log"; then printf '%s\n' 'FAIL IP-rocky-trust: private download used sudo' >&2; exit 1; fi
    if [ "$download_case" = main ] && grep -q '<podman>' "$test_dir/log"; then
      printf '%s\n' 'FAIL IP-21: already present Podman was added to the transaction' >&2; exit 1
    fi
    if [ "$download_case" = mixed ] && { ! grep -q '<--repo=epel>' "$test_dir/log" || ! grep -q '<podman-compose>' "$test_dir/log" || grep -q '<podman>' "$test_dir/log"; }; then
      printf '%s\n' 'FAIL W1-73: mixed download did not select the named Compose closure' >&2; exit 1
    fi
  done
  rm -rf "$test_dir"/download-*
  printf 'ok IP-rocky-trust download-only named transactions (%s)\n' "$test_locale"
  prereq_test_pre_cases
  prereq_test_approved_cases
  prereq_test_fetch_cases

  prereq_test_nocurl_cases
  prereq_test_qualify_cases

  prereq_test_candidate_cases
  for mixed_case in valid swap_after_verify bad_epel missing_compose bootstrap bootstrap_tampered; do
    if [ "$mixed_case" = swap_after_verify ] && [ "${PREREQ_V01_SELECT:-}" = combine ]; then continue; fi
    mixed_stage="$test_dir/mixed-$mixed_case"
    mkdir -m 700 "$mixed_stage" "$test_dir/mixed-keys" "$test_dir/mixed-replies"
    python3 "$root/scripts/fakes/prereq/rocky-fixtures.py" "$test_dir/mixed-keys"
    printf '%s\n' 'synthetic downloaded archive' > "$test_dir/mixed-replies/expected.rpm"
    python3 "$root/scripts/fakes/prereq/rocky-repo-fixtures.py" "$test_dir/mixed-replies" "$test_dir/mixed-replies/expected.rpm" "$test_dir/mixed-replies/expected.rpm" deps
    python3 "$root/scripts/fakes/prereq/repo-fixtures.py" "$test_dir/mixed-replies" valid
    mkdir -m 700 "$test_dir/mixed-boot" "$test_dir/mixed-boot/packages"
    printf '%s\n' 'verified synthetic epel-release archive' > "$test_dir/mixed-boot/packages/epel-release.rpm"
    mixed_boot_hash=$(sha256sum "$test_dir/mixed-boot/packages/epel-release.rpm"); mixed_boot_hash=${mixed_boot_hash%% *}
    printf '%s|%s|epel-release|0|10|8.el10|noarch|extras\n' "$test_dir/mixed-boot/packages/epel-release.rpm" "$mixed_boot_hash" > "$test_dir/mixed-boot/transaction.manifest"
    python3 "$root/scripts/fakes/prereq/epel-release-fixtures.py" "$test_dir/epel-release.cpio" valid
    printf '%s\n' "$test_dir/epel-release.cpio" > "$test_dir/state/epel-cpio"
    printf '%s\n' "$test_dir/mixed-replies" > "$test_dir/state/fetch-fixtures"
    printf '%s\n' valid > "$test_dir/state/fetch-case"
    mixed_sig=valid mixed_download=1 expected_status=3 actual=0
    case $mixed_case in valid|swap_after_verify|bootstrap) expected_status=0 ;; bad_epel) mixed_sig=wrong_key ;; missing_compose) mixed_download=0 ;; esac
    mixed_include=$include
    if [ "$mixed_case" = swap_after_verify ]; then
      sed 's/^prereq_rocky_archives_ok() {/prereq_test_archives_original() {/' "$include" > "$test_dir/mixed-swap-include.sh"
      mixed_include=$test_dir/mixed-swap-include.sh
    fi
    : > "$test_dir/log"
    env -i LC_ALL="$test_locale" PATH="$test_dir/bin" HOME="$test_dir/home" PREREQ_ROOT="$test_dir/tree" FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" \
      FAKE_ALLOW_ROOTS=1 FAKE_ALLOW_ROCKY_CONFIG=1 FAKE_ROCKY_CASE=metadata_off FAKE_ALLOW_DNF_CONFIG=1 FAKE_REPO_CASE=metadata_off \
      FAKE_ALLOW_ROCKY_DOWNLOAD=1 FAKE_EPEL_MIX="$mixed_download" FAKE_ROOT_STAGE="$mixed_stage" FAKE_ALLOW_ROCKY_ARCHIVES=1 \
      FAKE_ALLOW_RPMKEYS=1 FAKE_EPEL_SIGNATURE_CASE="$mixed_sig" FAKE_ALLOW_KEY_READ=1 FAKE_ALLOW_PAYLOAD=1 FAKE_PROVIDER="$test_dir/tree/usr/bin/podman-compose" \
      FAKE_ALLOW_EXTRACT=1 FAKE_EPEL_CPIO="$test_dir/epel-release.cpio" FAKE_PRIVATE_EPEL_STAGE="$mixed_stage" RUN_MIXED_CASE="$mixed_case" \
      "$test_dir/bin/sh" -c '. "$1"; EX_PREREQ=3; PREREQ_MISSING=$(printf "gnupg2\npodman-compose")
        if [ "$RUN_MIXED_CASE" = swap_after_verify ]; then
          prereq_rocky_archives_ok() {
            prereq_test_archives_original "$@" || return $?
            victim=$1/packages/one.rpm; old=$(sha256sum "$victim"); old=${old%% *}
            printf "%s\n" attacker-replacement > "$victim"; changed=$(sha256sum "$victim"); changed=${changed%% *}
            sed "s/|$old|/|$changed|/" "$1/transaction.manifest" > "$1/swapped.manifest" && mv "$1/swapped.manifest" "$1/transaction.manifest"
            ATTACK_HASH=$changed
            : > "$1/attack-performed"
          }
        fi
        case $RUN_MIXED_CASE in bootstrap*) PREREQ_ROCKY_PRE_STAGE=$4; PREREQ_ROCKY_VERIFIED_MANIFEST=$(cat "$4/transaction.manifest"); prereq_extract_epel_release "$4" || exit $? ;; esac
        if [ "$RUN_MIXED_CASE" = bootstrap_tampered ]; then printf "%s\n" changed >> "$4/epel-config/epel.repo"; fi
        if ! prereq_mixed_preverify "$2" 10.2 "$3"; then [ "$RUN_MIXED_CASE" = swap_after_verify ] && [ -f "$2/attack-performed" ] && exit 0; exit 3; fi
        if [ "$RUN_MIXED_CASE" = swap_after_verify ]; then [ -f "$2/attack-performed" ] || exit 98; case $PREREQ_ROCKY_VERIFIED_MANIFEST in *"|$ATTACK_HASH|"*) printf "%s\n" "FAIL V-01: adopted post-verification tuple" >&2; exit 99 ;; esac; fi' \
      sh "$mixed_include" "$mixed_stage" "$test_dir/mixed-keys/root1.asc" "$test_dir/mixed-boot" > "$test_dir/out" 2>&1 || actual=$?
    if [ "$actual" != "$expected_status" ]; then printf 'FAIL W1-73 mixed closure: %s returned %s, expected %s (%s)\n' "$mixed_case" "$actual" "$expected_status" "$test_locale" >&2; cat "$test_dir/out" >&2; tail -12 "$test_dir/log" >&2; exit 1; fi
    if { [ "$mixed_case" = valid ] || [ "$mixed_case" = bootstrap ]; } && { ! grep -q '|gnupg2|0|2.4.5|1.el10|x86_64|baseos$' "$mixed_stage/transaction.manifest" || ! grep -q '|podman-compose|0|1.5.0|1.el10_1|noarch|epel$' "$mixed_stage/transaction.manifest"; }; then
      printf '%s\n' 'FAIL W1-73 mixed closure: missing authenticated Rocky or EPEL archive' >&2; exit 1
    fi
    if grep -q '^sudo ' "$test_dir/log" || grep -q 'Prerequisite plan:' "$test_dir/out"; then printf '%s\n' 'FAIL W1-73 mixed closure: changed machine or printed plan before verification' >&2; exit 1; fi
    rm -rf "$mixed_stage" "$test_dir/mixed-keys" "$test_dir/mixed-replies" "$test_dir/mixed-boot"
  done
  printf 'ok W1-73 complete mixed Rocky/EPEL closure before plan (%s)\n' "$test_locale"
  prereq_test_extract_cases
  prereq_test_combine_cases
  mv "$test_dir/bin/stat" "$test_dir/bin/stat.real"
  ln -s "$root/scripts/fakes/prereq/fake-command" "$test_dir/bin/stat"
  prereq_test_mixedapply_cases
  prereq_test_bootapply_cases
  rm "$test_dir/bin/stat"; mv "$test_dir/bin/stat.real" "$test_dir/bin/stat"
  printf 'ok W1-73 mixed closure approved root copy and swap refusal (%s)\n' "$test_locale"
  mkdir -p "$test_dir/nogpg-bin" "$test_dir/nogpg-replies"
  for test_command in "$test_dir/bin"/*; do
    [ "${test_command##*/}" = gpg ] || ln -s "$test_command" "$test_dir/nogpg-bin/${test_command##*/}"
  done
  sed "s|$test_dir/bin|$test_dir/nogpg-bin|g" "$include" > "$test_dir/nogpg-include.sh"
  python3 "$root/scripts/fakes/prereq/repo-fixtures.py" "$test_dir/nogpg-replies" valid
  printf '%s\n' "$test_dir/nogpg-replies" > "$test_dir/state/fetch-fixtures"
  printf '%s\n' valid > "$test_dir/state/fetch-case"
  for nogpg_case in valid foreign; do
    : > "$test_dir/log"; actual=0; expected_status=0
    [ "$nogpg_case" != foreign ] || expected_status=3
    env -i LC_ALL="$test_locale" PATH="$test_dir/nogpg-bin" HOME="$test_dir/home" PREREQ_ROOT="$test_dir/tree" \
      FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" FAKE_ALLOW_DNF_CONFIG=1 FAKE_REPO_CASE=metadata_off \
      FAKE_ALLOW_RPMKEYS=1 FAKE_ALLOW_PAYLOAD=1 FAKE_PROVIDER="$test_dir/tree/usr/bin/podman-compose" RUN_FINGERPRINT="$nogpg_case" \
      "$test_dir/nogpg-bin/sh" -c '
        . "$1"; say() { printf "%s\n" "$*"; }; EX_PREREQ=3
        prereq_pgp_fingerprints() { if [ "$RUN_FINGERPRINT" = valid ]; then printf "%s\n" 7D8D15CBFC4E62688591FB2633D98517E37ED158; else printf "%s\n" 1111111111111111111111111111111111111111; fi; }
        prereq_requalify_compose' sh "$test_dir/nogpg-include.sh" > "$test_dir/out" 2>&1 || actual=$?
    if [ "$actual" != "$expected_status" ] || grep -q '^gpg ' "$test_dir/log"; then
      printf 'FAIL W1-73: no-gpg %s returned %s, expected %s (%s)\n' "$nogpg_case" "$actual" "$expected_status" "$test_locale" >&2; exit 1
    fi
  done
  rm -rf "$test_dir/nogpg-bin" "$test_dir/nogpg-replies"
  printf 'ok W1-73 installed-provider fingerprint without gpg (%s)\n' "$test_locale"
  # W1-73: exercise the public entry point, including acquisition, the actual
  # approval plan and cleanup, rather than only its transaction helpers.
  prereq_test_flow_cases
  prereq_test_run_cases
  prereq_test_repairrun_cases
  for plan_case in packages podman compose duplicate invalid; do
    case $plan_case in
      packages) plan_missing=$(printf 'gzip\nopenssh\npodman-compose (requalify)'); plan_packages='gzip openssh'; plan_compose=verify; expected_status=0 ;;
      podman) plan_missing=$(printf 'podman\npodman-compose (requalify)'); plan_packages=podman; plan_compose=verify; expected_status=0 ;;
      compose) plan_missing=$(printf 'curl\npodman-compose'); plan_packages=curl; plan_compose=install; expected_status=0 ;;
      duplicate) plan_missing=$(printf 'gzip\ngzip\npodman-compose (requalify)'); plan_packages=''; plan_compose=''; expected_status=1 ;;
      invalid) plan_missing=$(printf 'gzip\n--nogpgcheck\npodman-compose'); plan_packages=''; plan_compose=''; expected_status=1 ;;
    esac
    actual=0
    env -i LC_ALL="$test_locale" PATH="$test_dir/bin" RUN_MISSING="$plan_missing" "$test_dir/bin/sh" -c \
      '. "$1"; PREREQ_MISSING=$RUN_MISSING; prereq_parse_missing || exit $?; printf "PACKAGES=%s\nCOMPOSE=%s\n" "$PREREQ_PLAN_PACKAGES" "$PREREQ_PLAN_COMPOSE"' \
      sh "$include" > "$test_dir/out" 2>&1 || actual=$?
    if [ "$actual" != "$expected_status" ]; then
      printf 'FAIL IP-21/22: %s package plan returned %s, expected %s (%s)\n' "$plan_case" "$actual" "$expected_status" "$test_locale" >&2; exit 1
    fi
    if [ "$expected_status" = 0 ] && { ! grep -Fxq "PACKAGES=$plan_packages" "$test_dir/out" || ! grep -Fxq "COMPOSE=$plan_compose" "$test_dir/out"; }; then
      printf 'FAIL IP-21/22: %s selected the wrong package plan (%s)\n' "$plan_case" "$test_locale" >&2; exit 1
    fi
  done
  printf 'ok IP-21/22 package plan is exact and bounded (%s)\n' "$test_locale"
  if ! LC_ALL="$test_locale" sh -c '. "$1"; prereq_msg os_unsupported' sh "$include" > "$test_dir/out" 2>&1; then
    printf 'FAIL IP-53: os_unsupported was not reported in %s\n' "$test_locale" >&2
    exit 1
  fi
  if ! LC_ALL="$test_locale" awk '
    NR == 1 { if ($0 !~ /^problem: .+/ || $0 !~ /Rocky/) exit 1; next }
    NR == 2 { if ($0 !~ /^fix: .+/) exit 1; next }
    { exit 1 }
    END { if (NR != 2) exit 1 }
  ' "$test_dir/out"; then
    printf 'FAIL IP-53: expected a Rocky Linux problem and a separate fix in %s\n' "$test_locale" >&2
    exit 1
  fi
  printf 'ok IP-53 os_unsupported cause and fix (%s)\n' "$test_locale"

  prereq_test_message_cases

  actual=0
  LC_ALL="$test_locale" sh -c '. "$1"; prereq_msg not_a_message' sh "$include" > "$test_dir/out" 2>&1 || actual=$?
  if [ "$actual" != 1 ] || [ -s "$test_dir/out" ]; then
    printf '%s\n' 'FAIL IP-53: unknown message must fail silently' >&2
    exit 1
  fi
  printf 'ok IP-53/54 catalogue lines do not echo untrusted arguments (%s)\n' "$test_locale"

  prereq_test_os_cases

  prereq_test_subid_cases

  prereq_test_version_cases

  prereq_test_provider_cases

  prereq_test_completion_cases

  prereq_test_machine_cases

  # Exercise the real install.sh --check entry point against the same fake tree.
  printf 'ID=rocky\nVERSION_ID=10.2\n' > "$test_dir/tree/etc/os-release"
  printf 'cortexuser:100000:65536\n' > "$test_dir/tree/etc/subuid"
  printf 'cortexuser:100000:65536\n' > "$test_dir/tree/etc/subgid"
  : > "$test_dir/tree/sys/fs/cgroup/cgroup.controllers"
  : > "$test_dir/log"
  actual=0
  env -i LC_ALL="$test_locale" PATH="$test_dir/bin" FAKE_LOG="$test_dir/log" FAKE_STATE_DIR="$test_dir/state" \
    FAKE_GRAPH_ROOT="$test_dir/home/graph" PREREQ_ROOT="$test_dir/tree" HOME="$test_dir/home" \
    "$test_dir/bin/sh" "$test_dir/install-deploy/install.sh" --check --home "$test_dir/home" > "$test_dir/out" 2>&1 || actual=$?
  if [ "$actual" != 2 ] || ! grep -q 'origin and signature will be verified before use' "$test_dir/out" || grep -qi 'terminal' "$test_dir/out"; then
    printf 'FAIL IP-check: install.sh --check did not return the W1-69 finding in %s (status %s)\n' "$test_locale" "$actual" >&2
    exit 1
  fi
  if grep -Eq '^(dnf|sudo|usermod|systemctl|systemd-run|systemd-analyze|gpg|curl|wget|pip|pip3|podman-compose)( |$)' "$test_dir/log"; then
    printf '%s\n' 'FAIL IP-check: install.sh --check tried a mutation, key, provider or network tool' >&2
    exit 1
  fi
  if ! sh "$root/scripts/fakes/prereq/check-readonly-log.sh" "$test_dir/log"; then
    printf '%s\n' 'FAIL SAM-02: install.sh --check requested a mutating loginctl or podman operation' >&2
    exit 1
  fi
  printf 'ok IP-check install.sh reports unqualified provider read-only (%s)\n' "$test_locale"
  LC_ALL="$test_locale" TMPDIR=$scratch sh "$root/scripts/fakes/prereq/caller-trap-cell.sh" "$original_include" "$root"
  printf 'ok W1-109 caller traps unchanged (%s)\n' "$test_locale"
  case $test_locale in
    C) pq_test_completed_c=1 ;;
    C.utf8) pq_test_completed_utf8=1 ;;
  esac
done
if [ "$pq_test_completed_c" -ne 1 ] || [ "$pq_test_completed_utf8" -ne 1 ]; then
  printf '%s\n' 'FAIL IP-53: locale matrix did not complete C and C.utf8' >&2
  exit 1
fi
if [ ! -s "$test_dir/messages" ] || ! cmp -s "$test_dir/messages" "$test_dir/message-uses"; then
  printf '%s\n' 'FAIL IP-52: message table and call sites differ' >&2
  exit 1
fi
printf '%s\n' 'ok IP-52 every message is declared and used'
TMPDIR=$scratch python3 -I -B "$root/scripts/fakes/prereq/caller-trap-order-test.py"
TMPDIR=$scratch python3 -I -B "$root/scripts/fakes/prereq/caller-trap-mutant-test.py"
if [ -n "${PREREQ_JOINED_INSTALL:-}" ]; then
  printf '%s\n' 'FAIL IP-64: joined-installer suite is not implemented yet' >&2
  exit 1
fi
printf '%s\n' 'SKIP IP-64: joined release installer is not available in this lane'
