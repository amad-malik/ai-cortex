#!/bin/sh
# W1-109: the real bare prerequisite entries preserve the installer's traps.
set -eu
source_file=$1 root=$2 scratch=${TMPDIR:?}
case $scratch in /tmp|/tmp/*|/var/tmp|/var/tmp/*) exit 2 ;; esac
work=$(mktemp -d "$scratch/caller-trap.XXXXXXXX")
trap 'rm -rf -- "$work"' EXIT HUP INT TERM
mkdir -p "$work/bin" "$work/state" "$work/home/graph" "$work/tree/etc/pki/rpm-gpg" \
  "$work/tree/sys/fs/cgroup" "$work/tree/proc/net" "$work/tree/usr/bin" "$work/tree/var/cache" "$work/replies" "$work/keys"
for safe in sh awk cat chmod cp cut dirname find grep gzip head ln mkdir mktemp mv python3 readlink rm sed sha256sum sort ssh-keygen stat tar tee tr wc; do
  ln -s "$(command -v "$safe")" "$work/bin/$safe"
done
for fake in id uname df ss timedatectl loginctl getenforce podman podman-compose dnf sudo usermod systemctl systemd-run systemd-analyze rpm rpmkeys rpm2cpio gpg gpgv curl wget pip pip3 setenforce semanage chcon restorecon; do
  ln -s "$root/scripts/fakes/prereq/fake-command" "$work/bin/$fake"
done
sed -e "s|/usr/bin:/usr/sbin|$work/bin|g" -e "s|/usr/bin/sudo|$work/bin/sudo|g" \
  -e 's|^prereq_system_root() { :; }$|prereq_system_root() { printf "%s" "${PREREQ_ROOT:-}"; }|' \
  -e 's#GCONV_PATH PREREQ_ROOT || return 1#GCONV_PATH || return 1#' "$source_file" > "$work/include.sh"
grep -Fxq 'prereq_system_root() { printf "%s" "${PREREQ_ROOT:-}"; }' "$work/include.sh" || { printf '%s\n' 'FAIL W1-109: isolated root substitution failed' >&2; exit 1; }
grep -Fxq "  PATH=$work/bin" "$work/include.sh" || { printf '%s\n' 'FAIL W1-109: isolated PATH substitution failed' >&2; exit 1; }
! grep -Fq '/usr/bin/sudo' "$work/include.sh" || { printf '%s\n' 'FAIL W1-109: unsafe sudo remains' >&2; exit 1; }
printf 'ID=rocky\nVERSION_ID=10.2\n' > "$work/tree/etc/os-release"
printf 'cortexuser:100000:65536\n' > "$work/tree/etc/subuid"
printf 'cortexuser:100000:65536\n' > "$work/tree/etc/subgid"
: > "$work/tree/sys/fs/cgroup/cgroup.controllers"
printf 'sl local_address rem_address st\n' > "$work/tree/proc/net/tcp"
printf 'sl local_address rem_address st\n' > "$work/tree/proc/net/tcp6"
printf '#!/bin/sh\nexit 95\n' > "$work/tree/usr/bin/podman-compose"
chmod 755 "$work/tree/usr/bin/podman-compose"
: > "$work/tree/etc/pki/rpm-gpg/RPM-GPG-KEY-EPEL-10"
python3 "$root/scripts/fakes/prereq/repo-fixtures.py" "$work/replies" valid
python3 "$root/scripts/fakes/prereq/rocky-fixtures.py" "$work/keys"
cp "$work/keys/root1.asc" "$work/tree/etc/pki/rpm-gpg/RPM-GPG-KEY-Rocky-10"
printf '%s\n' 'synthetic downloaded archive' > "$work/replies/expected.rpm"
python3 "$root/scripts/fakes/prereq/rocky-repo-fixtures.py" "$work/replies" "$work/replies/expected.rpm" "$work/replies/expected.rpm" deps
printf '%s\n' "$work/replies" > "$work/state/fetch-fixtures"
printf '%s\n' valid > "$work/state/fetch-case"
cat > "$work/caller.sh" <<'CALLER'
. "$1"
say() { [ "$TRAP_CASE" != machine_fault ] && [ "$TRAP_CASE" != run_fault ] || return 1; printf '%s\n' "$*"; }
warn() { :; }
caller_exit() { trap -p EXIT HUP INT TERM > "$TRAP_AFTER"; printf '%s\n' sentinel > "$TRAP_MARKER"; }
trap caller_exit EXIT
trap ':' HUP INT TERM
trap -p EXIT HUP INT TERM > "$TRAP_BEFORE"
EX_MACHINE=2 EX_PREREQ=3 home_dir=$HOME port=8601
set -e
case $TRAP_CASE in
  machine_*) machine_look ;;
  run_package_failure) PREREQ_MISSING=gzip PREREQ_FIXES=''; prereq_run ;;
  run_package_success)
    FAKE_ROOT_STAGE="$PREREQ_ROOT/var/cache/cortex-prereq-1000-$$"; export FAKE_ROOT_STAGE
    PREREQ_MISSING=$(printf 'gnupg2\npodman-compose (requalify)') PREREQ_FIXES=''; prereq_run ;;
  run_*) PREREQ_MISSING='podman-compose (requalify)' PREREQ_FIXES=''; prereq_run ;;
esac
CALLER
for cell in machine_ok machine_root machine_refusal machine_fault run_approved run_refusal run_bad_signature run_fault run_package_failure run_package_success; do
  uid=1000 check=0 yes=1 signature=valid expected=0
  case $cell in
    machine_root) uid=0; expected=2 ;;
    machine_refusal) check=1; expected=2 ;;
    machine_fault|run_bad_signature|run_fault|run_package_failure) expected=3 ;;
    run_refusal) yes=0; expected=2 ;;
  esac
  [ "$cell" != run_bad_signature ] || signature=digest_only
  allow_install=0
  if [ "$cell" = run_package_success ]; then
    allow_install=1
    mv "$work/bin/stat" "$work/bin/stat.real"
    ln -s "$root/scripts/fakes/prereq/fake-command" "$work/bin/stat"
    printf 'ID=rocky\nVERSION_ID=10.2\n' > "$work/tree/etc/os-release"
  fi
  if [ "$cell" = run_package_failure ]; then printf 'ID=almalinux\nVERSION_ID=10\n' > "$work/tree/etc/os-release"; fi
  before="$work/before-$cell" after="$work/after-$cell" marker="$work/marker-$cell"
  status=0
  env -i LC_ALL="${LC_ALL:-C}" PATH="$work/bin" HOME="$work/home" PREREQ_ROOT="$work/tree" \
    FAKE_LOG="$work/log" FAKE_STATE_DIR="$work/state" FAKE_UID="$uid" FAKE_GRAPH_ROOT="$work/home/graph" \
    FAKE_ALLOW_KEY_READ=1 FAKE_KEY_CASE=valid FAKE_ALLOW_DNF_CONFIG=1 FAKE_REPO_CASE=metadata_off \
    FAKE_ALLOW_RPMKEYS=1 FAKE_SIGNATURE_CASE="$signature" FAKE_ALLOW_PAYLOAD=1 FAKE_PROVIDER="$work/tree/usr/bin/podman-compose" \
    FAKE_BIN="$work/bin" FAKE_PRIVATE_PARENT="$work/home" FAKE_DOWNLOAD_PARENT="$work/home" FAKE_REAL_STAT="$work/bin/stat.real" \
    FAKE_ALLOW_ROOTS="$allow_install" FAKE_ALLOW_ROOT_STAT="$allow_install" FAKE_STAT_CASE=mixed FAKE_ALLOW_STAGE="$allow_install" \
    FAKE_ALLOW_ROCKY_CONFIG="$allow_install" FAKE_ROCKY_CASE=metadata_off FAKE_ALLOW_ROCKY_DOWNLOAD="$allow_install" FAKE_INCLUDE_DEPS=1 \
    FAKE_ALLOW_ROCKY_ARCHIVES="$allow_install" FAKE_ALLOW_ROCKY_INSTALL="$allow_install" \
    CORTEX_YES="$yes" check_only="$check" TRAP_CASE="$cell" TRAP_BEFORE="$before" TRAP_AFTER="$after" TRAP_MARKER="$marker" \
    "$work/bin/sh" "$work/caller.sh" "$work/include.sh" > "$work/out-$cell" 2>&1 || status=$?
  if [ "$cell" = run_package_success ]; then rm "$work/bin/stat"; mv "$work/bin/stat.real" "$work/bin/stat"; fi
  case $cell in machine_*) entry=machine_look ;; *) entry=prereq_run ;; esac
  if [ "$status" != "$expected" ] || [ ! -s "$marker" ] || [ ! -s "$before" ] || [ ! -s "$after" ] || ! cmp -s "$before" "$after"; then
    printf 'FAIL W1-109: %s %s status=%s expected=%s or caller traps changed\n' "$entry" "$cell" "$status" "$expected" >&2
    cat "$work/out-$cell" >&2
    exit 1
  fi
done
printf '%s\n' 'PASS W1-109: bare prerequisite entries preserve caller EXIT, HUP, INT and TERM traps'
