#!/bin/sh
# F-7 r4: real failing writes in conditional calls must not be ignored.
set -eu
source_file=${1:?include required}
scratch=${TMPDIR:?scratch required}
case $scratch in /tmp|/tmp/*|/var/tmp|/var/tmp/*) exit 2 ;; esac
work=$(mktemp -d "$scratch/output-failure.XXXXXXXX")
trap 'rm -rf -- "$work"' EXIT HUP INT TERM
mkdir -p "$work/home" "$work/tree/sys/fs/cgroup" "$work/tree/etc/pki/rpm-gpg"
: > "$work/tree/sys/fs/cgroup/cgroup.controllers"
: > "$work/tree/etc/pki/rpm-gpg/RPM-GPG-KEY-EPEL-10"
run_case() {
  kind=$1 failure=$2 expected=$3
  actual=0
  FAKE_WORK=$work FAKE_KIND=$kind FAKE_FAILURE=$failure HOME="$work/home" sh -c '
    . "$1"
    EX_PREREQ=3 EX_MACHINE=2 CORTEX_YES=1 check_only=0 home_dir=$HOME port=8601
    prereq_system_root() { printf "%s" "$FAKE_WORK/tree"; }
    say() {
      case $FAKE_KIND:$FAKE_FAILURE:$* in
        machine:yes:SELinux:*) printf "%s\n" "$*" > /dev/full ;;
        requalify:yes:*) printf "%s\n" "$*" > /dev/full ;;
        *) printf "%s\n" "$*" ;;
      esac
    }
    warn() { printf "%s\n" "$*"; }
    prereq_os_ok() { pq_os_version=10.2; return 0; }
    prereq_subid_ok() { return 0; }
    prereq_existing_dir() { printf "%s\n" "$HOME"; }
    prereq_disk_ok() { return 0; }
    prereq_port_free() { return 0; }
    uname() { printf "%s\n" x86_64; }
    id() { case $1 in -u) printf "%s\n" 1000 ;; -un) printf "%s\n" cortexuser ;; *) return 1 ;; esac; }
    podman() { case $1 in --version) printf "%s\n" "podman version 5.8.2" ;; info) printf "%s\n" "$HOME" ;; *) return 1 ;; esac; }
    dnf() { return 0; }
    rpm() { case $1 in -q) case ${2-} in --quiet) return 0 ;; *) printf "%s\n" "podman-compose|0|1.5.0|1.el10|noarch" ;; esac ;; --version) printf "%s\n" "RPM version fake" ;; *) return 1 ;; esac; }
    rpmkeys() { printf "%s\n" "RPM version fake"; }
    gpg() { printf "%s\n" "gpg fake"; }
    timedatectl() { printf "%s\n" yes; }
    getenforce() { printf "%s\n" Enforcing; }
    loginctl() { printf "%s\n" yes; }
    prereq_home_safe() { return 0; }
    prereq_epel_key_ok() { pq_key_records="fpr:::::::::7D8D15CBFC4E62688591FB2633D98517E37ED158:"; }
    prereq_epel_repo_config_ok() { PREREQ_EPEL_METALINK=fake PREREQ_EPEL_REPO_GPGCHECK=1; }
    prereq_epel_fetch() { printf "%s\n" "fixture receipt" > "$pq_requalify_stage/receipt.txt"; }
    prereq_rpm_signature_ok() { pq_sig_output="Signature: OK"; }
    prereq_rpm_payload_ok() { return 0; }
    status=0
    case $FAKE_KIND in
      machine) machine_look || status=$? ;;
      requalify) prereq_requalify_compose || status=$? ;;
      *) exit 90 ;;
    esac
    printf "status=%s provider=%s\n" "$status" "${PREREQ_COMPOSE:-}"
    exit "$status"
  ' sh "$source_file" > "$work/out" 2>&1 || actual=$?
  if [ "$actual" -ne "$expected" ]; then
    printf 'FAIL F-7 r4: %s/%s returned %s, expected %s\n' "$kind" "$failure" "$actual" "$expected" >&2
    cat "$work/out" >&2
    return 1
  fi
  printf 'PASS F-7 r4: %s/%s returned %s\n' "$kind" "$failure" "$actual"
}
failures=0
run_case machine no 0 || failures=$((failures + 1))
run_case machine yes 3 || failures=$((failures + 1))
run_case requalify no 0 || failures=$((failures + 1))
run_case requalify yes 3 || failures=$((failures + 1))
[ "$failures" -eq 0 ]
