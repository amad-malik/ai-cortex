# Installer prerequisite messages and checks. This file is joined into install.sh.
# Caller contract: public installer entries leave the caller's EXIT, HUP, INT
# and TERM traps unchanged on every return, so errexit runs the installer's
# own cleanup. Private cleanup traps run only inside subshells;
# their internal _body functions are not caller entry points.

prereq_msg() {
  pq_msg_location=selected pq_msg_port=unknown
  case ${2-} in home|container) pq_msg_location=$2 ;; esac
  case ${2-} in ''|*[!0123456789]*) ;; *) [ "${#2}" -le 5 ] && pq_msg_port=$2 ;; esac
  case ${1-} in
    os_unsupported)
      case ${PREREQ_OS_REASON:-invalid_version} in
        distribution) pq_cause='this distribution has no trusted installer chain'; pq_fix='use Rocky Linux 10 on x86_64' ;;
        untrusted_major) pq_cause='no trusted repository chain is defined for this Rocky major version'; pq_fix='use Rocky Linux 10 on x86_64' ;;
        *) pq_cause='the Rocky Linux release is not a supported major.minor version'; pq_fix='use Rocky Linux 10 with a numeric minor version on x86_64' ;;
      esac
      ;;
    arch_unsupported)
      pq_cause='this machine is not identified as x86_64'
      pq_fix='use an x86_64 machine with Rocky Linux 10'
      ;;
    is_root)
      pq_cause='the installer cannot run as root or identify the current user'
      pq_fix='run the installer as a normal user for rootless containers'
      ;;
    no_cgroup_v2)
      pq_cause='cgroup v2 is not available on this machine'
      pq_fix='enable cgroup v2 and run the installer again'
      ;;
    no_subuid)
      pq_cause='the current user has no sufficient subordinate user-ID range'
      pq_fix='allow the installer to configure a non-overlapping subordinate user-ID range'
      ;;
    no_subgid)
      pq_cause='the current user has no sufficient subordinate group-ID range'
      pq_fix='allow the installer to configure a non-overlapping subordinate group-ID range'
      ;;
    subids_failed)
      pq_cause='the planned subordinate-ID repair did not complete safely'
      pq_fix='check the subordinate-ID files and sudo access, then review the plan again'
      ;;
    time_not_synced)
      pq_cause='the system clock is reported as unsynchronized'
      pq_fix='enable clock synchronization and run the installer again'
      ;;
    linger_off)
      pq_cause='user service lingering is not enabled or could not be confirmed'
      pq_fix='allow the installer to enable lingering for the current user'
      ;;
    disk_low)
      pq_cause="the $pq_msg_location filesystem has insufficient free disk space"
      pq_fix='provide the required free space on that filesystem and run the installer again'
      ;;
    disk_unknown)
      pq_cause="the $pq_msg_location filesystem free disk space could not be checked"
      pq_fix='check the disk path and the configured free-space threshold, then run the installer again'
      ;;
    fs_not_local)
      pq_cause="the $pq_msg_location filesystem is a network or FUSE filesystem"
      pq_fix='select a local XFS or ext4 filesystem and run the installer again'
      ;;
    fs_unsupported)
      pq_cause="the $pq_msg_location filesystem is not supported"
      pq_fix='select a local XFS or ext4 filesystem and run the installer again'
      ;;
    port_busy)
      pq_cause="the selected port $pq_msg_port is already listening"
      pq_fix='choose a free port and run the installer again'
      ;;
    port_unknown)
      pq_cause='the selected port could not be checked'
      pq_fix='make socket information available and run the installer again'
      ;;
    graph_unknown)
      pq_cause='the container storage filesystem could not be identified'
      pq_fix='check the rootless Podman storage configuration and run the installer again'
      ;;
    podman_old)
      pq_cause='the installed Podman version is older than 5 or could not be confirmed'
      pq_fix='update Podman through the distribution package manager, then run the installer again'
      ;;
    no_sudo)
      pq_cause='sudo is required for the planned prerequisites but is unavailable'
      pq_fix='install or configure sudo, then run the installer again as a normal user'
      ;;
    no_tty)
      pq_cause='a terminal is required to confirm the prerequisite plan'
      pq_fix='run again with --yes, or from a terminal'
      ;;
    consent_declined)
      pq_cause='the prerequisite plan was not approved'
      pq_fix='review the plan and approve it when ready to continue'
      ;;
    done_usage)
      pq_cause='completion summary usage requires a version, endpoint, token path and log path'
      pq_fix='provide all four values and print the summary again'
      ;;
    epel_needed)
      pq_cause='the EPEL release package is not installed or could not be confirmed'
      pq_fix='review the prerequisite plan for Rocky Extras and stable EPEL before continuing'
      ;;
    rpm_unknown)
      pq_cause='the RPM package database tool is unavailable'
      pq_fix='restore the Rocky Linux RPM tooling and run the installer again'
      ;;
    rpm_extract_unknown)
      pq_cause='the native RPM archive extraction tool is unavailable'
      pq_fix='restore rpm2cpio from the trusted Rocky Linux image and run the installer again'
      ;;
    metadata_verifier_missing)
      pq_cause='signed repository metadata cannot be verified with the available tools'
      pq_fix='provision trusted GnuPG and gpgv from the Rocky Linux image, then run the installer again'
      ;;
    home_unsafe)
      pq_cause='the private staging parent can be replaced or is not owned by the current user'
      pq_fix='use a user-owned home directory whose parents are not writable by other users'
      ;;
    package_manager_unknown)
      pq_cause='the Rocky Linux package manager is unavailable'
      pq_fix='restore DNF through the system administrator and run the installer again'
      ;;
    compose_missing)
      pq_cause='podman-compose is not installed or could not be confirmed'
      pq_fix='review the plan to install a verified stable EPEL provider before continuing'
      ;;
    compose_verify_failed)
      pq_cause='the installed Compose provider could not be verified against stable EPEL'
      pq_fix='check the repository, network, package signature and installed files, then review the plan again'
      ;;
    prereq_unavailable)
      pq_cause='the prerequisite plan cannot be completed safely'
      pq_fix='check the reported prerequisites and run the installer again'
      ;;
    rocky_install_failed)
      pq_cause='the Rocky prerequisite transaction could not be verified or installed'
      pq_fix='check the trusted Rocky roots, repositories, package files and sudo access, then retry'
      ;;
    *) return 1 ;;
  esac
  printf 'problem: %s\n' "$pq_cause" >&2 || return "${EX_PREREQ:-3}"
  printf 'fix: %s\n' "$pq_fix" >&2 || return "${EX_PREREQ:-3}"
}

prereq_info() {
  case ${1-} in
    selinux)
      case ${2-} in Enforcing|Permissive|Disabled) pq_info_selinux=$2 ;; *) pq_info_selinux=unknown ;; esac
      say "SELinux: $pq_info_selinux"
      ;;
    os_unproven)
      case ${2-} in 10.*) pq_info_minor=${2#10.}; case $pq_info_minor in ''|*[!0123456789]*) return 1 ;; esac ;; *) return 1 ;; esac
      warn "Rocky Linux $2 is UNPROVEN; the real-image check covered Rocky Linux 10.2" ;;
    time_unknown) warn 'clock synchronization could not be determined; check the system clock' ;;
    compose_pending) say 'podman-compose is installed; its origin and signature will be verified before use.' ;;
    done_version) say "Installed Cortex version: $2" ;;
    done_endpoint) say "Endpoint: $2" ;;
    done_token) say "Token file: $2" ;;
    done_next) say 'Next: use the token file to sign in to the endpoint.' ;;
    done_log) say "Install log: $2" ;;
    *) return 1 ;;
  esac
}

say_done() {
  if [ "$#" -lt 3 ] || [ "$#" -gt 4 ]; then prereq_msg done_usage; return "${EX_USAGE:-1}"; fi
  pq_done_log=${4:-${CORTEX_LOG:-}}
  pq_done_newline='
'
  for pq_done_value in "$1" "$2" "$3" "$pq_done_log"; do
    case $pq_done_value in ''|*"$pq_done_newline"*) prereq_msg done_usage; return "${EX_USAGE:-1}" ;; esac
  done
  prereq_info done_version "$1"
  prereq_info done_endpoint "$2"
  prereq_info done_token "$3"
  prereq_info done_next
  prereq_info done_log "$pq_done_log"
}

# The production installer always inspects the real root. Tests substitute
# this function only in an isolated copy; environment variables cannot select it.
prereq_system_root() { :; }

# Keep trust and privilege checks on system tools; restore the caller environment.
prereq_harden_environment() {
  PATH=/usr/bin:/usr/sbin
  LC_ALL=C
  export PATH LC_ALL
  hash -r
  unset LD_PRELOAD LD_LIBRARY_PATH LD_AUDIT PYTHONPATH PYTHONHOME PYTHONSTARTUP BASH_ENV ENV CDPATH OPENSSL_CONF OPENSSL_MODULES OPENSSL_ENGINES GCONV_PATH PREREQ_ROOT || return 1
}

prereq_secure_call() {
  [ "$#" -eq 1 ] || return 1
  set -- "$1" "${PATH+x}" "${PATH-}" "${LD_PRELOAD+x}" "${LD_PRELOAD-}" "${LD_LIBRARY_PATH+x}" "${LD_LIBRARY_PATH-}" "${LD_AUDIT+x}" "${LD_AUDIT-}" "${PYTHONPATH+x}" "${PYTHONPATH-}" "${PYTHONHOME+x}" "${PYTHONHOME-}" "${PYTHONSTARTUP+x}" "${PYTHONSTARTUP-}" "${BASH_ENV+x}" "${BASH_ENV-}" "${ENV+x}" "${ENV-}" "${CDPATH+x}" "${CDPATH-}" "${GCONV_PATH+x}" "${GCONV_PATH-}" "${PREREQ_ROOT+x}" "${PREREQ_ROOT-}" "${OPENSSL_CONF+x}" "${OPENSSL_CONF-}" "${OPENSSL_MODULES+x}" "${OPENSSL_MODULES-}" "${OPENSSL_ENGINES+x}" "${OPENSSL_ENGINES-}" "${LC_ALL+x}" "${LC_ALL-}"
  if prereq_harden_environment; then
    # Preserve ordinary errexit for a bare wrapper call. If the caller itself
    # uses if/||, POSIX shell semantics still suppress errexit in the body.
    "$1"
    pq_secure_status=$?
  else
    pq_secure_status=1
  fi
  shift
  if [ "$1" = x ]; then PATH=$2; export PATH; else unset PATH; fi
  shift 2
  if [ "$1" = x ]; then LD_PRELOAD=$2; export LD_PRELOAD; else unset LD_PRELOAD; fi
  shift 2
  if [ "$1" = x ]; then LD_LIBRARY_PATH=$2; export LD_LIBRARY_PATH; else unset LD_LIBRARY_PATH; fi
  shift 2
  if [ "$1" = x ]; then LD_AUDIT=$2; export LD_AUDIT; else unset LD_AUDIT; fi
  shift 2
  if [ "$1" = x ]; then PYTHONPATH=$2; export PYTHONPATH; else unset PYTHONPATH; fi
  shift 2
  if [ "$1" = x ]; then PYTHONHOME=$2; export PYTHONHOME; else unset PYTHONHOME; fi
  shift 2
  if [ "$1" = x ]; then PYTHONSTARTUP=$2; export PYTHONSTARTUP; else unset PYTHONSTARTUP; fi
  shift 2
  if [ "$1" = x ]; then BASH_ENV=$2; export BASH_ENV; else unset BASH_ENV; fi
  shift 2
  if [ "$1" = x ]; then ENV=$2; export ENV; else unset ENV; fi
  shift 2
  if [ "$1" = x ]; then CDPATH=$2; export CDPATH; else unset CDPATH; fi
  shift 2
  if [ "$1" = x ]; then GCONV_PATH=$2; export GCONV_PATH; else unset GCONV_PATH; fi
  shift 2
  if [ "$1" = x ]; then PREREQ_ROOT=$2; export PREREQ_ROOT; else unset PREREQ_ROOT; fi
  shift 2
  if [ "$1" = x ]; then OPENSSL_CONF=$2; export OPENSSL_CONF; else unset OPENSSL_CONF; fi
  shift 2
  if [ "$1" = x ]; then OPENSSL_MODULES=$2; export OPENSSL_MODULES; else unset OPENSSL_MODULES; fi
  shift 2
  if [ "$1" = x ]; then OPENSSL_ENGINES=$2; export OPENSSL_ENGINES; else unset OPENSSL_ENGINES; fi
  shift 2
  if [ "$1" = x ]; then LC_ALL=$2; export LC_ALL; else unset LC_ALL; fi
  shift 2
  hash -r
  return "$pq_secure_status"
}

# Read the whole machine rather than stopping after the first blocking finding.
machine_look() {
  prereq_secure_call machine_look_body
}

machine_look_body() {
  PREREQ_MISSING='' PREREQ_FIXES='' PREREQ_NEED_SUDO=0 PREREQ_COMPOSE=''
  pq_machine_block=0 pq_machine_supported=1
  if ! prereq_os_ok "$(prereq_system_root)/etc/os-release"; then
    prereq_msg os_unsupported || return "${EX_PREREQ:-3}"
    pq_machine_block=1
    pq_machine_supported=0
  elif [ "$PREREQ_OS_CONFIDENCE" = UNPROVEN ]; then
    prereq_info os_unproven "$pq_os_version" || return "${EX_PREREQ:-3}"
  fi
  pq_machine_arch=$(uname -m 2>/dev/null) || pq_machine_arch=''
  if [ "$pq_machine_arch" != x86_64 ]; then
    prereq_msg arch_unsupported || return "${EX_PREREQ:-3}"
    pq_machine_block=1
    pq_machine_supported=0
  fi
  pq_machine_uid=$(id -u 2>/dev/null) || pq_machine_uid=''
  case $pq_machine_uid in
    ''|*[!0123456789]*|0) prereq_msg is_root || return "${EX_PREREQ:-3}"; pq_machine_block=1 ;;
  esac
  pq_machine_user=$(id -un 2>/dev/null) || pq_machine_user=''
  if [ -z "$pq_machine_user" ]; then
    prereq_msg is_root || return "${EX_PREREQ:-3}"
    pq_machine_block=1
  else
    if ! prereq_subid_ok "$pq_machine_user" "$(prereq_system_root)/etc/subuid"; then
      prereq_msg no_subuid || return "${EX_PREREQ:-3}"
      PREREQ_FIXES="${PREREQ_FIXES:+$PREREQ_FIXES }subuid"
      PREREQ_NEED_SUDO=1
    fi
    if ! prereq_subid_ok "$pq_machine_user" "$(prereq_system_root)/etc/subgid"; then
      prereq_msg no_subgid || return "${EX_PREREQ:-3}"
      PREREQ_FIXES="${PREREQ_FIXES:+$PREREQ_FIXES }subgid"
      PREREQ_NEED_SUDO=1
    fi
  fi
  if [ ! -f "$(prereq_system_root)/sys/fs/cgroup/cgroup.controllers" ]; then
    prereq_msg no_cgroup_v2 || return "${EX_PREREQ:-3}"
    pq_machine_block=1
  fi
  if command -v podman >/dev/null 2>&1; then
    if ! prereq_podman_ok; then
      prereq_msg podman_old || return "${EX_PREREQ:-3}"
      pq_machine_block=1
    fi
  else
    prereq_missing_add podman
    PREREQ_NEED_SUDO=1
  fi
  if [ "$pq_machine_supported" -eq 1 ]; then
    if ! command -v dnf >/dev/null 2>&1; then
      prereq_msg package_manager_unknown || return "${EX_PREREQ:-3}"
      pq_machine_block=1
    fi
    if ! command -v rpm >/dev/null 2>&1; then
      prereq_msg rpm_unknown || return "${EX_PREREQ:-3}"
      pq_machine_block=1
    elif ! rpm -q --quiet epel-release >/dev/null 2>&1; then
      prereq_msg epel_needed || return "${EX_PREREQ:-3}"
      PREREQ_FIXES="${PREREQ_FIXES:+$PREREQ_FIXES }epel"
      PREREQ_NEED_SUDO=1
    fi
    if command -v rpm >/dev/null 2>&1; then
      if rpm -q --quiet podman-compose >/dev/null 2>&1; then
        prereq_info compose_pending || return "${EX_PREREQ:-3}"
        prereq_missing_add 'podman-compose (requalify)'
      else
        prereq_msg compose_missing || return "${EX_PREREQ:-3}"
        prereq_missing_add podman-compose
        PREREQ_NEED_SUDO=1
      fi
    fi
  fi
  for pq_command_package in \
    curl:curl tar:tar gzip:gzip python3:python3 ssh-keygen:openssh gpg:gnupg2 gpgv:gnupg2 rpmkeys:rpm \
    awk:gawk sed:sed grep:grep sha256sum:coreutils cat:coreutils chmod:coreutils \
    cp:coreutils cut:coreutils head:coreutils ln:coreutils mkdir:coreutils \
    mktemp:coreutils mv:coreutils readlink:coreutils rm:coreutils sort:coreutils \
    stat:coreutils tee:coreutils tr:coreutils wc:coreutils find:findutils; do
    pq_command=${pq_command_package%%:*}
    pq_package=${pq_command_package#*:}
    if ! command -v "$pq_command" >/dev/null 2>&1; then
      prereq_missing_add "$pq_package"
      PREREQ_NEED_SUDO=1
    fi
  done
  pq_machine_time=''
  if command -v timedatectl >/dev/null 2>&1; then
    pq_machine_time=$(timedatectl show -p NTPSynchronized --value 2>/dev/null) || pq_machine_time=''
  fi
  case $pq_machine_time in
    yes) ;;
    no) prereq_msg time_not_synced || return "${EX_PREREQ:-3}"; pq_machine_block=1 ;;
    *) prereq_info time_unknown || return "${EX_PREREQ:-3}";;
  esac
  pq_machine_selinux=''
  if command -v getenforce >/dev/null 2>&1; then
    pq_machine_selinux=$(getenforce 2>/dev/null) || pq_machine_selinux=''
  fi
  prereq_info selinux "$pq_machine_selinux" || return "${EX_PREREQ:-3}"
  if [ -n "$pq_machine_user" ]; then
    pq_machine_linger=''
    if command -v loginctl >/dev/null 2>&1; then
      pq_machine_linger=$(loginctl show-user "$pq_machine_user" -p Linger --value 2>/dev/null) || pq_machine_linger=''
    fi
    if [ "$pq_machine_linger" != yes ]; then
      prereq_msg linger_off || return "${EX_PREREQ:-3}"
      PREREQ_FIXES="${PREREQ_FIXES:+$PREREQ_FIXES }linger"
      PREREQ_NEED_SUDO=1
    fi
  fi
  pq_machine_min=${CORTEX_MIN_FREE_GIB:-20}
  case $pq_machine_min in
    ''|*[!0123456789]*) prereq_msg disk_unknown 'home' || return "${EX_PREREQ:-3}"; pq_machine_block=1; pq_machine_min='' ;;
  esac
  if [ -n "$pq_machine_min" ]; then
    while [ "${pq_machine_min#0}" != "$pq_machine_min" ]; do pq_machine_min=${pq_machine_min#0}; done
    [ -n "$pq_machine_min" ] || pq_machine_min=0
  fi
  if [ -n "$pq_machine_min" ] && [ "${#pq_machine_min}" -gt 9 ]; then
    prereq_msg disk_unknown 'home' || return "${EX_PREREQ:-3}"
    pq_machine_block=1
    pq_machine_min=''
  fi
  if [ -n "$pq_machine_min" ]; then
    pq_machine_min_bytes=$((pq_machine_min * 1073741824))
    pq_machine_home=$(prereq_existing_dir "${home_dir:-}") || pq_machine_home=''
    if [ -z "$pq_machine_home" ]; then
      pq_machine_block=1
    else
      pq_machine_disk_rc=0
      prereq_disk_ok "$pq_machine_home" home "$pq_machine_min_bytes" || pq_machine_disk_rc=$?
      [ "$pq_machine_disk_rc" -ne "${EX_PREREQ:-3}" ] || return "${EX_PREREQ:-3}"
      [ "$pq_machine_disk_rc" -eq 0 ] || pq_machine_block=1
    fi
    if command -v podman >/dev/null 2>&1; then
      pq_machine_graph=$(podman info --format '{{.Store.GraphRoot}}' 2>/dev/null) || pq_machine_graph=''
    else
      pq_machine_graph=${HOME:-}/.local/share/containers/storage
    fi
    case $pq_machine_graph in
      /*) pq_machine_graph=$(prereq_existing_dir "$pq_machine_graph") || pq_machine_graph='' ;;
      *) pq_machine_graph='' ;;
    esac
    if [ -z "$pq_machine_graph" ]; then
      prereq_msg graph_unknown || return "${EX_PREREQ:-3}"
      pq_machine_block=1
    else
      pq_machine_disk_rc=0
      prereq_disk_ok "$pq_machine_graph" container "$pq_machine_min_bytes" || pq_machine_disk_rc=$?
      [ "$pq_machine_disk_rc" -ne "${EX_PREREQ:-3}" ] || return "${EX_PREREQ:-3}"
      [ "$pq_machine_disk_rc" -eq 0 ] || pq_machine_block=1
    fi
  fi
  pq_machine_port_rc=0
  prereq_port_free "${port:-}" || pq_machine_port_rc=$?
  [ "$pq_machine_port_rc" -ne "${EX_PREREQ:-3}" ] || return "${EX_PREREQ:-3}"
  [ "$pq_machine_port_rc" -eq 0 ] || pq_machine_block=1
  if [ "$PREREQ_NEED_SUDO" -eq 1 ] && ! [ -x /usr/bin/sudo ]; then
    prereq_msg no_sudo || return "${EX_PREREQ:-3}"
    pq_machine_block=1
  fi
  if [ "${check_only:-0}" != 1 ] && [ -n "$PREREQ_MISSING$PREREQ_FIXES" ] && [ "${CORTEX_YES:-0}" != 1 ] && ! prereq_tty_available; then
    prereq_msg no_tty || return "${EX_PREREQ:-3}"
    pq_machine_block=1
  fi
  if [ "${check_only:-0}" = 1 ] && [ -n "$PREREQ_MISSING$PREREQ_FIXES" ]; then
    pq_machine_block=1
  fi
  [ "$pq_machine_block" -eq 0 ] || return "${EX_MACHINE:-2}"
  return 0
}

# os-release is untrusted data: recognize only literal values, without sourcing it.
prereq_os_ok() {
  PREREQ_OS_REASON=invalid_version PREREQ_OS_CONFIDENCE=''
  [ "$#" -eq 1 ] && [ -f "$1" ] && [ -r "$1" ] || return 1
  pq_os_id='' pq_os_version='' pq_os_id_seen=0 pq_os_version_seen=0
  while IFS= read -r pq_os_line || [ -n "$pq_os_line" ]; do
    case $pq_os_line in
      ID=*) pq_os_key=ID; pq_os_value=${pq_os_line#ID=} ;;
      VERSION_ID=*) pq_os_key=VERSION_ID; pq_os_value=${pq_os_line#VERSION_ID=} ;;
      *) continue ;;
    esac
    case $pq_os_value in
      \"*\") pq_os_value=${pq_os_value#\"}; pq_os_value=${pq_os_value%\"} ;;
      \'*\') pq_os_value=${pq_os_value#\'}; pq_os_value=${pq_os_value%\'} ;;
    esac
    case $pq_os_value in ''|*[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._-]*) return 1 ;; esac
    case $pq_os_key in
      ID) [ "$pq_os_id_seen" -eq 0 ] || return 1; pq_os_id_seen=1; pq_os_id=$pq_os_value ;;
      VERSION_ID) [ "$pq_os_version_seen" -eq 0 ] || return 1; pq_os_version_seen=1; pq_os_version=$pq_os_value ;;
    esac
  done < "$1"
  [ "$pq_os_id" = rocky ] || { PREREQ_OS_REASON=distribution; return 1; }
  case $pq_os_version in *.*) ;; *) return 1 ;; esac
  pq_os_major=${pq_os_version%%.*} pq_os_minor=${pq_os_version#*.}
  case $pq_os_major in ''|*[!0123456789]*) return 1 ;; esac
  case $pq_os_minor in ''|*[!0123456789]*) return 1 ;; esac
  if [ "$pq_os_major" != 10 ]; then
    case $pq_os_major in 1[123456789]|[23456789]?|???*) PREREQ_OS_REASON=untrusted_major ;; esac
    return 1
  fi
  pq_os_minor_normal=$pq_os_minor
  while :; do case $pq_os_minor_normal in 0*) pq_os_minor_normal=${pq_os_minor_normal#0} ;; *) break ;; esac; done
  case $pq_os_minor_normal in ''|1|2) PREREQ_OS_CONFIDENCE=SUPPORTED ;; *) PREREQ_OS_CONFIDENCE=UNPROVEN ;; esac
  PREREQ_OS_REASON=''
  return 0
}

# Repository operations accept the same Rocky 10.MINOR identity as os-release.
prereq_release_ok() {
  [ "$#" -eq 1 ] || return 1
  case $1 in
    10.*) case ${1#10.} in ''|*[!0123456789]*) return 1 ;; esac ;;
    *) return 1 ;;
  esac
}

# Require a complete account match and a valid range of at least 65536 IDs.
prereq_subid_ok() {
  [ "$#" -eq 2 ] && [ -f "$2" ] && [ -r "$2" ] || return 1
  case $1 in ''|*[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_.-]*) return 1 ;; esac
  LC_ALL=C awk -F: -v pq_user="$1" '
    NF == 3 && $1 == pq_user &&
    $2 ~ /^[0123456789]+$/ && $3 ~ /^[0123456789]+$/ &&
    $2 + 0 > 0 && $2 + 0 <= 4294967295 && $3 + 0 >= 65536 &&
    $2 + $3 <= 4294967296 { found = 1 }
    END { if (!found) exit 1 }
  ' "$2"
}

# One range above every recorded subordinate UID and GID avoids overlaps in
# either namespace. Refuse malformed or exhausted files instead of guessing.
prereq_subid_range() {
  [ "$#" -eq 2 ] && [ -f "$1" ] && [ -r "$1" ] && [ -f "$2" ] && [ -r "$2" ] || return 1
  LC_ALL=C awk -F: '
    BEGIN { highest = 99999 }
    /^([[:space:]]*|[[:space:]]*#.*)$/ { next }
    NF != 3 || $1 == "" || $2 !~ /^[0123456789]+$/ || $3 !~ /^[0123456789]+$/ { bad = 1; next }
    {
      start = $2 + 0; count = $3 + 0
      if (start < 1 || count < 1 || start > 4294967295 || start + count > 4294967296) { bad = 1; next }
      if (start + count - 1 > highest) highest = start + count - 1
    }
    END {
      first = highest + 1; last = first + 65535
      if (bad || last > 4294967295) exit 1
      printf "%.0f-%.0f\n", first, last
    }
  ' "$1" "$2"
}

prereq_subid_has_range() {
  [ "$#" -eq 3 ] || return 1
  LC_ALL=C awk -F: -v pq_user="$1" -v pq_expected="$2" '
    BEGIN { split(pq_expected, parts, "-") }
    NF == 3 && $1 == pq_user && $2 ~ /^[0123456789]+$/ && $3 ~ /^[0123456789]+$/ &&
      $2 + 0 == parts[1] + 0 && $3 + 0 == 65536 { found = 1 }
    END { if (!found) exit 1 }
  ' "$3"
}

prereq_fix_subids() {
  if [ "$#" -ne 4 ]; then prereq_msg subids_failed; return "${EX_PREREQ:-3}"; fi
  pq_fix_user=$1 pq_fix_range=$2 pq_fix_uid=$3 pq_fix_gid=$4
  case $pq_fix_user in ''|-*|*[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_.-]*) prereq_msg subids_failed; return "${EX_PREREQ:-3}" ;; esac
  case $pq_fix_uid:$pq_fix_gid in 1:0|0:1|1:1) ;; *) prereq_msg subids_failed; return "${EX_PREREQ:-3}" ;; esac
  pq_fix_current=$(prereq_subid_range "$(prereq_system_root)/etc/subuid" "$(prereq_system_root)/etc/subgid") || pq_fix_current=''
  if [ -z "$pq_fix_current" ] || [ "$pq_fix_current" != "$pq_fix_range" ]; then prereq_msg subids_failed; return "${EX_PREREQ:-3}"; fi
  set -- usermod
  [ "$pq_fix_uid" != 1 ] || set -- "$@" --add-subuids "$pq_fix_range"
  [ "$pq_fix_gid" != 1 ] || set -- "$@" --add-subgids "$pq_fix_range"
  if ! ( umask 022; /usr/bin/sudo "$@" -- "$pq_fix_user" ); then prereq_msg subids_failed; return "${EX_PREREQ:-3}"; fi
  if { [ "$pq_fix_uid" = 1 ] && ! prereq_subid_has_range "$pq_fix_user" "$pq_fix_range" "$(prereq_system_root)/etc/subuid"; } ||
     { [ "$pq_fix_gid" = 1 ] && ! prereq_subid_has_range "$pq_fix_user" "$pq_fix_range" "$(prereq_system_root)/etc/subgid"; }; then
    prereq_msg subids_failed
    return "${EX_PREREQ:-3}"
  fi
  return 0
}

prereq_fix_linger() {
  if [ "$#" -ne 1 ]; then prereq_msg linger_off; return "${EX_PREREQ:-3}"; fi
  pq_linger_user=$1
  case $pq_linger_user in ''|-*|*[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_.-]*) prereq_msg linger_off; return "${EX_PREREQ:-3}" ;; esac
  pq_linger_state=$(loginctl show-user "$pq_linger_user" -p Linger --value 2>/dev/null) || pq_linger_state=''
  [ "$pq_linger_state" != yes ] || return 0
  # The one approved plan lists the privileged fallback before either attempt.
  loginctl enable-linger "$pq_linger_user" || :
  pq_linger_state=$(loginctl show-user "$pq_linger_user" -p Linger --value 2>/dev/null) || pq_linger_state=''
  [ "$pq_linger_state" != yes ] || return 0
  if ! ( umask 022; /usr/bin/sudo loginctl enable-linger -- "$pq_linger_user" ); then prereq_msg linger_off; return "${EX_PREREQ:-3}"; fi
  pq_linger_state=$(loginctl show-user "$pq_linger_user" -p Linger --value 2>/dev/null) || pq_linger_state=''
  if [ "$pq_linger_state" != yes ]; then prereq_msg linger_off; return "${EX_PREREQ:-3}"; fi
  return 0
}

prereq_tty_available() {
  pq_tty=$(prereq_system_root)/dev/tty
  [ -c "$pq_tty" ] && ( : <> "$pq_tty" ) 2>/dev/null
}

prereq_confirm() {
  [ "${CORTEX_YES:-0}" != 1 ] || return 0
  if ! prereq_tty_available; then prereq_msg no_tty || return "${EX_PREREQ:-3}"; return "${EX_MACHINE:-2}"; fi
  say 'Proceed with these changes? [y/N]' || return "${EX_PREREQ:-3}"
  pq_confirm_answer=''
  IFS= read -r pq_confirm_answer < "$pq_tty" || pq_confirm_answer=''
  case $pq_confirm_answer in
    y|Y|yes|yEs|yeS|yES|Yes|YEs|YeS|YES) return 0 ;;
    *) prereq_msg consent_declined || return "${EX_PREREQ:-3}"; return "${EX_MACHINE:-2}" ;;
  esac
}

prereq_missing_add() {
  pq_missing_newline='
'
  case "$pq_missing_newline$PREREQ_MISSING$pq_missing_newline" in *"$pq_missing_newline$1$pq_missing_newline"*) return 0 ;; esac
  PREREQ_MISSING="${PREREQ_MISSING}${PREREQ_MISSING:+$pq_missing_newline}$1"
}

prereq_podman_ok() {
  pq_podman_version=$(podman --version 2>/dev/null) || return 1
  case $pq_podman_version in 'podman version '*) pq_podman_version=${pq_podman_version#podman version } ;; *) return 1 ;; esac
  pq_podman_major=${pq_podman_version%%.*}
  case $pq_podman_major in ''|*[!0123456789]*) return 1 ;; esac
  while [ "${pq_podman_major#0}" != "$pq_podman_major" ]; do pq_podman_major=${pq_podman_major#0}; done
  [ -n "$pq_podman_major" ] && [ "${#pq_podman_major}" -le 9 ] && [ "$pq_podman_major" -ge 5 ]
}

prereq_compose_version_ok() {
  case ${1-} in ''|*[!0123456789.]*|.*|*.|*..*) return 1 ;; *.*) ;; *) return 1 ;; esac
  pq_compose_major=${1%%.*}
  pq_compose_tail=${1#*.}
  pq_compose_minor=${pq_compose_tail%%.*}
  case $pq_compose_tail in
    *.*) pq_compose_patch=${pq_compose_tail#*.} ;;
    *) pq_compose_patch=0 ;;
  esac
  for pq_compose_part in "$pq_compose_major" "$pq_compose_minor" "$pq_compose_patch"; do
    case $pq_compose_part in ''|*[!0123456789]*) return 1 ;; esac
    [ "${#pq_compose_part}" -le 9 ] || return 1
  done
  while [ "${pq_compose_major#0}" != "$pq_compose_major" ]; do pq_compose_major=${pq_compose_major#0}; done
  while [ "${pq_compose_minor#0}" != "$pq_compose_minor" ]; do pq_compose_minor=${pq_compose_minor#0}; done
  [ -n "$pq_compose_major" ] || pq_compose_major=0
  [ -n "$pq_compose_minor" ] || pq_compose_minor=0
  [ "$pq_compose_major" -gt 1 ] || { [ "$pq_compose_major" -eq 1 ] && [ "$pq_compose_minor" -ge 5 ]; }
}

# Examine the full primary fingerprint of exactly one public certificate.
# The caller must supply a private, already copied key file and GPG home.
prereq_epel_key_ok() {
  [ "$#" -eq 2 ] || return 1
  if ! command -v gpg >/dev/null 2>&1; then
    pq_key_fingerprint=$(prereq_pgp_fingerprints "$2") || return 1
    [ "$pq_key_fingerprint" = 7D8D15CBFC4E62688591FB2633D98517E37ED158 ] || return 1
    pq_key_records=$(printf '%s\n' 'pub::::::::::' "fpr:::::::::$pq_key_fingerprint:") || return 1
    return 0
  fi
  pq_key_records=$(LC_ALL=C gpg --batch --no-options --homedir "$1" --with-colons --fingerprint --show-keys "$2" 2>/dev/null) || return 1
  printf '%s\n' "$pq_key_records" | LC_ALL=C awk -F: '
    $1 == "pub" { primary++; waiting = 1; next }
    $1 == "sub" { waiting = 0 }
    $1 == "fpr" && waiting { fingerprint = $10; waiting = 0; fingerprints++ }
    END {
      if (primary != 1 || fingerprints != 1 || fingerprint != "7D8D15CBFC4E62688591FB2633D98517E37ED158") exit 1
    }
  '
}

# Accept only the official stable metalink and DNF's effective secure options.
# This inspection is part of the consented phase, never machine_look.
prereq_epel_repo_config_ok() {
  [ "$#" -eq 1 ] || [ "$#" -eq 2 ] || return 1
  prereq_release_ok "$1" || return 1
  pq_repo_expected=epel-z-$1
  PREREQ_EPEL_METALINK='' PREREQ_EPEL_REPO_GPGCHECK=''
  pq_repo_key='file:///etc/pki/rpm-gpg/RPM-GPG-KEY-EPEL-10'
  if [ "$#" -eq 2 ]; then
    pq_repo_file=$2/epel-config/epel.repo pq_repo_key_file=$2/pinned/certificate.key
    prereq_private_path "$2" "$pq_repo_file" && prereq_private_path "$2" "$pq_repo_key_file" &&
      [ -f "$pq_repo_file" ] && [ -f "$pq_repo_key_file" ] || return 1
    pq_repo_key=file://$2/pinned/certificate.key
    pq_repo_config=$(LC_ALL=C dnf "--releasever=$1" --cacheonly --assumeno "--setopt=reposdir=$(prereq_system_root)/etc/yum.repos.d,$2/epel-config" "--setopt=epel.gpgkey=$pq_repo_key" config-manager --dump epel 2>/dev/null) || return 1
  else
    pq_repo_config=$(LC_ALL=C dnf "--releasever=$1" --cacheonly --assumeno config-manager --dump epel 2>/dev/null) || return 1
  fi
  pq_repo_selected=$(printf '%s\n' "$pq_repo_config" | LC_ALL=C awk -v expected="$pq_repo_expected" -v expected_key="$pq_repo_key" '
    function trim(s) { sub(/^[[:space:]]+/, "", s); sub(/[[:space:]]+$/, "", s); return s }
    function yes(s) { s = tolower(s); return s == "1" || s == "true" }
    function boolean(s) { s = tolower(s); return s == "0" || s == "1" || s == "true" || s == "false" }
    /^[=]+[[:space:]]+repo: epel[[:space:]]+[=]+$/ { sections++; next }
    /^[=]+.*[=]+$/ { bad = 1; next }
    /^[[:space:]]*$/ { next }
    /^\[[^]]+\]$/ { if ($0 != "[epel]" || ++headers > 1) bad = 1; next }
    {
      pos = index($0, "=")
      if (!pos) { bad = 1; next }
      key = trim(substr($0, 1, pos - 1)); value = trim(substr($0, pos + 1))
      if (key == "baseurl" || key == "metalink" || key == "mirrorlist" || key == "gpgcheck" ||
          key == "gpgkey" || key == "sslverify" || key == "sslcacert" || key == "repo_gpgcheck" ||
          key == "proxy" || key == "proxy_sslverify" || key == "proxy_sslcacert") {
        if (++seen[key] != 1) bad = 1
        values[key] = value
      }
    }
    END {
      if (sections != 1 || bad) exit 1
      split("baseurl metalink gpgcheck gpgkey sslverify sslcacert repo_gpgcheck proxy proxy_sslverify proxy_sslcacert", required, " ")
      for (i in required) if (seen[required[i]] != 1) exit 1
      if (values["baseurl"] != "" || values["mirrorlist"] != "" || values["sslcacert"] != "" ||
          values["proxy_sslcacert"] != "" || !yes(values["gpgcheck"]) || !yes(values["sslverify"]) ||
          !yes(values["proxy_sslverify"]) || !boolean(values["repo_gpgcheck"]) ||
          values["gpgkey"] != expected_key) exit 1
      prefix = "https://mirrors.fedoraproject.org/metalink?"
      url = values["metalink"]
      if (substr(url, 1, length(prefix)) != prefix) exit 1
      count = split(substr(url, length(prefix) + 1), params, "&")
      for (i = 1; i <= count; i++) {
        if (split(params[i], pair, "=") != 2 || ++query[pair[1]] != 1) exit 1
        if (pair[1] == "repo") { if (pair[2] != expected) exit 1 }
        else if (pair[1] == "arch") { if (pair[2] != "x86_64") exit 1 }
        else if (pair[1] == "infra" || pair[1] == "content") {
          if (pair[2] !~ /^[ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._]+$/) exit 1
        } else exit 1
      }
      if (query["repo"] != 1 || query["arch"] != 1) exit 1
      print url
      print yes(values["repo_gpgcheck"]) ? "1" : "0"
    }
  ') || return 1
  pq_repo_newline='
'
  PREREQ_EPEL_METALINK=${pq_repo_selected%"$pq_repo_newline"*}
  PREREQ_EPEL_REPO_GPGCHECK=${pq_repo_selected#*"$pq_repo_newline"}
  case $PREREQ_EPEL_REPO_GPGCHECK in 0|1) ;; *) PREREQ_EPEL_METALINK=''; return 1 ;; esac
}

# Validate effective Rocky settings without fetching metadata or importing keys.
prereq_rocky_repo_ok() {
  [ "$#" -eq 2 ] || return 1
  prereq_release_ok "$1" || return 1
  case $2 in baseos) pq_rocky_channel=BaseOS ;; appstream) pq_rocky_channel=AppStream ;; crb) pq_rocky_channel=CRB ;; extras) pq_rocky_channel=extras ;; *) return 1 ;; esac
  PREREQ_ROCKY_REPO_GPGCHECK=''
  pq_rocky_config=$(LC_ALL=C dnf "--releasever=$1" --cacheonly --assumeno config-manager --dump "$2" 2>/dev/null) || return 1
  pq_rocky_policy=$(printf '%s\n' "$pq_rocky_config" | LC_ALL=C awk -v section="$2" -v expected="$pq_rocky_channel-$1" '
    function trim(s) { sub(/^[[:space:]]+/, "", s); sub(/[[:space:]]+$/, "", s); return s }
    function yes(s) { s = tolower(s); return s == "1" || s == "true" }
    function boolean(s) { s = tolower(s); return s == "0" || s == "1" || s == "true" || s == "false" }
    /^[=]+.*[=]+$/ { if (index($0, "repo: " section " ")) sections++; else bad = 1; next }
    /^[[:space:]]*$/ { next }
    /^\[[^]]+\]$/ { if ($0 != "[" section "]" || ++headers > 1) bad = 1; next }
    {
      pos = index($0, "="); if (!pos) { bad = 1; next }
      key = trim(substr($0, 1, pos - 1)); value = trim(substr($0, pos + 1))
      if (key == "baseurl" || key == "metalink" || key == "mirrorlist" || key == "gpgcheck" ||
          key == "gpgkey" || key == "sslverify" || key == "sslcacert" || key == "repo_gpgcheck" ||
          key == "proxy" || key == "proxy_sslverify" || key == "proxy_sslcacert") {
        if (++seen[key] != 1) bad = 1
        values[key] = value
      }
    }
    END {
      if (sections != 1 || bad) exit 1
      split("baseurl mirrorlist gpgcheck gpgkey sslverify sslcacert repo_gpgcheck proxy proxy_sslverify proxy_sslcacert", required, " ")
      for (i in required) if (seen[required[i]] != 1) exit 1
      if (values["baseurl"] != "" || values["metalink"] != "" || values["sslcacert"] != "" ||
          values["proxy_sslcacert"] != "" || !yes(values["gpgcheck"]) || !yes(values["sslverify"]) ||
          !yes(values["proxy_sslverify"]) || !boolean(values["repo_gpgcheck"]) ||
          values["gpgkey"] != "file:///etc/pki/rpm-gpg/RPM-GPG-KEY-Rocky-10") exit 1
      prefix = "https://mirrors.rockylinux.org/mirrorlist?"
      url = values["mirrorlist"]
      if (substr(url, 1, length(prefix)) != prefix) exit 1
      count = split(substr(url, length(prefix) + 1), params, "&")
      for (i = 1; i <= count; i++) {
        if (split(params[i], pair, "=") != 2 || ++query[pair[1]] != 1) exit 1
        if (pair[1] == "repo") { if (pair[2] != expected) exit 1 }
        else if (pair[1] == "arch") { if (pair[2] != "x86_64") exit 1 }
        else exit 1
      }
      if (query["repo"] != 1 || query["arch"] != 1) exit 1
      print yes(values["repo_gpgcheck"]) ? "1" : "0"
    }
  ') || return 1
  case $pq_rocky_policy in 0|1) PREREQ_ROCKY_REPO_GPGCHECK=$pq_rocky_policy ;; *) return 1 ;; esac
}

# Derive v4 primary-key fingerprints from already trusted RPM-owned key bytes;
# this is identification only, never a substitute for signature verification.
prereq_pgp_fingerprints() {
  [ "$#" -eq 1 ] || return 1
  python3 -I - "$1" <<'PREREQ_PGP_PY'
import base64
import hashlib
from pathlib import Path
import re
import sys

try:
    text = Path(sys.argv[1]).read_text(encoding="ascii")
    blocks = re.findall(r"-----BEGIN PGP PUBLIC KEY BLOCK-----(.*?)-----END PGP PUBLIC KEY BLOCK-----", text, re.S)
    remainder = re.sub(r"-----BEGIN PGP PUBLIC KEY BLOCK-----.*?-----END PGP PUBLIC KEY BLOCK-----", "", text, flags=re.S)
    if not blocks or remainder.strip():
        raise ValueError("no public key")
    fingerprints = []
    for block in blocks:
        lines = [line.strip() for line in block.splitlines()]
        payload = "".join(line for line in lines if line and ":" not in line and not line.startswith("="))
        data = base64.b64decode(payload, validate=True)
        offset = 0
        while offset < len(data):
            header = data[offset]
            offset += 1
            if not header & 0x80:
                raise ValueError("invalid packet")
            if header & 0x40:
                tag = header & 0x3f
                first = data[offset]
                offset += 1
                if first < 192:
                    size = first
                elif first < 224:
                    size = ((first - 192) << 8) + data[offset] + 192
                    offset += 1
                elif first == 255:
                    size = int.from_bytes(data[offset:offset + 4], "big")
                    offset += 4
                else:
                    raise ValueError("partial packet")
            else:
                tag = (header >> 2) & 15
                width = (1, 2, 4, 0)[header & 3]
                if not width:
                    raise ValueError("indeterminate packet")
                size = int.from_bytes(data[offset:offset + width], "big")
                offset += width
            end = offset + size
            if end > len(data):
                raise ValueError("truncated packet")
            body = data[offset:end]
            offset = end
            if tag == 6:
                if len(body) < 6 or body[0] != 4 or len(body) > 65535:
                    raise ValueError("unsupported primary key")
                fingerprint = hashlib.sha1(b"\x99" + len(body).to_bytes(2, "big") + body, usedforsecurity=False).hexdigest().upper()
                if fingerprint in fingerprints:
                    raise ValueError("duplicate primary key")
                fingerprints.append(fingerprint)
    if not fingerprints:
        raise ValueError("no primary key")
    print("\n".join(fingerprints))
except (OSError, UnicodeError, ValueError, IndexError):
    raise SystemExit(1)
PREREQ_PGP_PY
}

# Bind one or more OS-provided roots to a private verifier, without GPG.
prereq_rocky_roots() {
  [ "$#" -ge 2 ] || return 1
  pq_roots_stage=$1; shift
  [ -d "$pq_roots_stage" ] && [ ! -L "$pq_roots_stage" ] || return 1
  pq_roots_mode=$(stat -c %a -- "$pq_roots_stage" 2>/dev/null) || return 1
  pq_roots_uid=$(stat -c %u -- "$pq_roots_stage" 2>/dev/null) || return 1
  pq_roots_self=$(id -u 2>/dev/null) || return 1
  [ "$pq_roots_mode" = 700 ] && [ "$pq_roots_uid" = "$pq_roots_self" ] || return 1
  mkdir -m 700 "$pq_roots_stage/rocky-ring" || return 1
  pq_roots_receipt=$pq_roots_stage/roots.receipt
  [ ! -e "$pq_roots_receipt" ] && [ ! -L "$pq_roots_receipt" ] || return 1
  : > "$pq_roots_receipt" || return 1
  pq_roots_count=0 pq_roots_seen='
'
  for pq_roots_file do
    case $pq_roots_file in /*) ;; *) return 1 ;; esac
    case $pq_roots_file in *[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_./-]*|*/../*|*/..|*//* ) return 1 ;; esac
    case $pq_roots_seen in *"
$pq_roots_file
"*) return 1 ;; esac
    pq_roots_seen="$pq_roots_seen$pq_roots_file
"
    [ -f "$pq_roots_file" ] && [ ! -L "$pq_roots_file" ] || return 1
    pq_roots_owner=$(LC_ALL=C rpm -qf --queryformat '%{NAME}\n' "$pq_roots_file" 2>/dev/null) || return 1
    pq_roots_package=rocky-gpg-keys
    [ "$pq_roots_owner" = "$pq_roots_package" ] || return 1
    pq_roots_algorithm=$(LC_ALL=C rpm -qf --queryformat '%{FILEDIGESTALGO}\n' "$pq_roots_file" 2>/dev/null) || return 1
    [ "$pq_roots_algorithm" = 8 ] || return 1
    pq_roots_flags=$(LC_ALL=C rpm -qf --queryformat '[%{FILENAMES}|%{FILEFLAGS}|%{FILEVERIFYFLAGS}|%{FILEUSERNAME}|%{FILEGROUPNAME}|%{FILEMODES}\n]' "$pq_roots_file" 2>/dev/null) || return 1
    printf '%s\n' "$pq_roots_flags" | LC_ALL=C awk -F'|' -v path="$pq_roots_file" '
      function has(value, bit) { return int(value / bit) % 2 == 1 }
      $1 == path {
        found++
        if (NF != 6 || $2 !~ /^[0123456789]+$/ || $3 !~ /^[0123456789]+$/ || $6 !~ /^[0123456789]+$/ ||
            has($2, 8) || has($2, 64) || !has($3, 1) || !has($3, 2) || !has($3, 8) || !has($3, 16) || !has($3, 64) ||
            $4 != "root" || $5 != "root" || int($6 / 4096) % 16 != 8 || has(int($6 / 8) % 8, 2) || has($6 % 8, 2)) bad = 1
      }
      END { if (found != 1 || bad) exit 1 }
    ' || return 1
    pq_roots_changes=$(LC_ALL=C rpm -Vf --noscripts "$pq_roots_file" 2>/dev/null) || return 1
    [ -z "$pq_roots_changes" ] || return 1
    pq_roots_count=$((pq_roots_count + 1))
    pq_roots_copy=$pq_roots_stage/rocky-ring/root-$pq_roots_count.key
    cp -- "$pq_roots_file" "$pq_roots_copy" || return 1
    pq_roots_original_hash=$(sha256sum -- "$pq_roots_file") || return 1
    pq_roots_copy_hash=$(sha256sum -- "$pq_roots_copy") || return 1
    pq_roots_original_hash=${pq_roots_original_hash%% *}
    pq_roots_copy_hash=${pq_roots_copy_hash%% *}
    [ "$pq_roots_original_hash" = "$pq_roots_copy_hash" ] || return 1
    pq_roots_fingerprints=$(prereq_pgp_fingerprints "$pq_roots_copy") || return 1
    printf 'root=%s\nsha256=%s\n' "$pq_roots_file" "$pq_roots_copy_hash" >> "$pq_roots_receipt" || return 1
    while IFS= read -r pq_roots_fingerprint; do
      [ -n "$pq_roots_fingerprint" ] || return 1
      printf 'fingerprint=%s\n' "$pq_roots_fingerprint" >> "$pq_roots_receipt" || return 1
    done <<PREREQ_ROCKY_FINGERPRINTS
$pq_roots_fingerprints
PREREQ_ROCKY_FINGERPRINTS
  done
  [ "$pq_roots_count" -gt 0 ]
}

# Root-owned non-writable ancestors prevent replacement between verification
# and the privileged transaction. The caller creates the stage after consent.
prereq_root_locked_path() {
  [ "$#" -eq 1 ] || return 1
  case $1 in /*) pq_locked_path=$1 ;; *) return 1 ;; esac
  case $pq_locked_path in *[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_./+~^-]*|*/../*|*/..|*//* ) return 1 ;; esac
  while :; do
    [ -e "$pq_locked_path" ] && [ ! -L "$pq_locked_path" ] || return 1
    pq_locked_mode=$(stat -c '%u|%a' -- "$pq_locked_path" 2>/dev/null) || return 1
    printf '%s\n' "$pq_locked_mode" | LC_ALL=C awk -F'|' '
      NF != 2 || $1 != "0" || $2 !~ /^[01234567]+$/ || length($2) < 3 || length($2) > 4 { exit 1 }
      { group = int($2 / 10) % 10; other = $2 % 10; if (group == 2 || group == 3 || group == 6 || group == 7 || other == 2 || other == 3 || other == 6 || other == 7) exit 1 }
    ' || return 1
    [ "$pq_locked_path" != / ] || break
    pq_locked_path=${pq_locked_path%/*}
    [ -n "$pq_locked_path" ] || pq_locked_path=/
  done
}

# A download/verification stage is owned by this user and inaccessible to
# other users. Its contents are never used for installation after approval.
prereq_private_path() {
  [ "$#" -eq 2 ] || return 1
  pq_private_stage=$1 pq_private_path=$2
  case $pq_private_stage in /*) ;; *) return 1 ;; esac
  case $pq_private_path in "$pq_private_stage"|"$pq_private_stage"/*) ;; *) return 1 ;; esac
  case $pq_private_path in *[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_./+~^-]*|*/../*|*/..|*//* ) return 1 ;; esac
  pq_private_uid=$(id -u 2>/dev/null) || return 1
  pq_private_stage_mode=$(stat -c %a -- "$pq_private_stage" 2>/dev/null) || return 1
  [ "$pq_private_stage_mode" = 700 ] || return 1
  while :; do
    [ -e "$pq_private_path" ] && [ ! -L "$pq_private_path" ] || return 1
    pq_private_info=$(stat -c '%u|%a' -- "$pq_private_path" 2>/dev/null) || return 1
    printf '%s\n' "$pq_private_info" | LC_ALL=C awk -F'|' -v uid="$pq_private_uid" '
      NF != 2 || $1 != uid || $2 !~ /^[01234567]+$/ || length($2) < 3 || length($2) > 4 { exit 1 }
      { group = int($2 / 10) % 10; other = $2 % 10; if (group == 2 || group == 3 || group == 6 || group == 7 || other == 2 || other == 3 || other == 6 || other == 7) exit 1 }
    ' || return 1
    [ "$pq_private_path" != "$pq_private_stage" ] || break
    pq_private_path=${pq_private_path%/*}
  done
}

prereq_stage_path_ok() {
  [ "$#" -eq 3 ] || return 1
  case $1 in
    root) prereq_root_locked_path "$3" ;;
    private) prereq_private_path "$2" "$3" ;;
    *) return 1 ;;
  esac
}

# Compare the root-owned verifier ring to the exact OS-owned roots after
# checking their RPM ownership and integrity in a private inspection stage.
prereq_rocky_seal_ok() {
  [ "$#" -ge 3 ] || return 1
  pq_seal_stage=$1 pq_seal_private=$2; shift 2
  pq_seal_ring=$pq_seal_stage/rocky-ring
  [ -d "$pq_seal_ring" ] && prereq_root_locked_path "$pq_seal_ring" || return 1
  prereq_rocky_roots "$pq_seal_private" "$@" || return 1
  pq_seal_count=0 pq_seal_seen='
'
  for pq_seal_original do
    prereq_root_locked_path "$pq_seal_original" || return 1
    pq_seal_count=$((pq_seal_count + 1))
    pq_seal_copy=$pq_seal_ring/root-$pq_seal_count.key
    [ -f "$pq_seal_copy" ] && [ ! -L "$pq_seal_copy" ] && prereq_root_locked_path "$pq_seal_copy" || return 1
    pq_seal_source_hash=$(sha256sum -- "$pq_seal_original") || return 1
    pq_seal_copy_hash=$(sha256sum -- "$pq_seal_copy") || return 1
    pq_seal_source_hash=${pq_seal_source_hash%% *}
    pq_seal_copy_hash=${pq_seal_copy_hash%% *}
    [ "$pq_seal_source_hash" = "$pq_seal_copy_hash" ] || return 1
    pq_seal_seen="$pq_seal_seen$pq_seal_copy
"
  done
  for pq_seal_found in "$pq_seal_ring"/* "$pq_seal_ring"/.[!.]* "$pq_seal_ring"/..?*; do
    if [ ! -e "$pq_seal_found" ] && [ ! -L "$pq_seal_found" ]; then continue; fi
    case $pq_seal_seen in *"
$pq_seal_found
"*) ;; *) return 1 ;; esac
  done
  [ "$pq_seal_count" -gt 0 ]
}

# Create only a new root-owned cache stage after consent. A collision never
# permits removal of a pre-existing path.
prereq_prepare_root_stage() {
  [ "$#" -ge 3 ] || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
  pq_stage_path=$1 pq_stage_private=$2; shift 2
  pq_stage_cache=$(prereq_system_root)/var/cache
  pq_stage_uid=$(id -u 2>/dev/null) || pq_stage_uid=''
  case $pq_stage_uid in ''|*[!0123456789]*) prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}" ;; esac
  case $pq_stage_path in "$pq_stage_cache"/cortex-prereq-"$pq_stage_uid"-*) ;; *) prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}" ;; esac
  pq_stage_suffix=${pq_stage_path#"$pq_stage_cache"/cortex-prereq-"$pq_stage_uid"-}
  case $pq_stage_suffix in ''|*[!0123456789]*) prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}" ;; esac
  if [ -e "$pq_stage_path" ] || [ -L "$pq_stage_path" ] || ! prereq_root_locked_path "$pq_stage_cache" || ! prereq_rocky_roots "$pq_stage_private" "$@"; then
    prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"
  fi
  if ! /usr/bin/sudo mkdir -m 755 -- "$pq_stage_path"; then prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; fi
  PREREQ_ROOT_STAGE=$pq_stage_path
  if ! /usr/bin/sudo mkdir -m 755 -- "$pq_stage_path/packages" "$pq_stage_path/rocky-ring" "$pq_stage_path/empty"; then
    if prereq_root_locked_path "$pq_stage_path"; then /usr/bin/sudo rm -rf -- "$pq_stage_path" || :; fi
    prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"
  fi
  pq_stage_count=0
  for pq_stage_root do
    pq_stage_count=$((pq_stage_count + 1))
    if ! /usr/bin/sudo install -m 0644 -- "$pq_stage_root" "$pq_stage_path/rocky-ring/root-$pq_stage_count.key"; then
      if prereq_root_locked_path "$pq_stage_path"; then /usr/bin/sudo rm -rf -- "$pq_stage_path" || :; fi
      prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"
    fi
  done
  if ! mkdir -m 700 "$pq_stage_private/seal" || ! prereq_rocky_seal_ok "$pq_stage_path" "$pq_stage_private/seal" "$@"; then
    if prereq_root_locked_path "$pq_stage_path"; then /usr/bin/sudo rm -rf -- "$pq_stage_path" || :; fi
    prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"
  fi
  PREREQ_ROOT_STAGE=$pq_stage_path
  return 0
}

# Inventory root-owned downloaded archives without trusting their claimed
# identity or installing them. Official metadata and signatures follow.
prereq_rocky_candidates() {
  [ "$#" -eq 1 ] || [ "$#" -eq 2 ] || return 1
  pq_candidates_stage=$1 pq_candidates_mode=${2:-root}
  pq_candidates_dir=$1/packages
  [ -d "$pq_candidates_dir" ] && prereq_stage_path_ok "$pq_candidates_mode" "$pq_candidates_stage" "$pq_candidates_dir" || return 1
  pq_candidates_count=0 pq_candidates_output='' pq_candidates_identities='
'
  for pq_candidate in "$pq_candidates_dir"/* "$pq_candidates_dir"/.[!.]* "$pq_candidates_dir"/..?*; do
    if [ ! -e "$pq_candidate" ] && [ ! -L "$pq_candidate" ]; then continue; fi
    case $pq_candidate in *.rpm) ;; *) return 1 ;; esac
    [ -f "$pq_candidate" ] && [ ! -L "$pq_candidate" ] && prereq_stage_path_ok "$pq_candidates_mode" "$pq_candidates_stage" "$pq_candidate" || return 1
    pq_candidate_hash=$(sha256sum -- "$pq_candidate") || return 1
    pq_candidate_hash=${pq_candidate_hash%% *}
    case $pq_candidate_hash in ''|*[!0123456789abcdef]*) return 1 ;; esac
    [ "${#pq_candidate_hash}" -eq 64 ] || return 1
    pq_candidate_identity=$(LC_ALL=C rpm -qp --queryformat '%{NAME}|%{EPOCHNUM}|%{VERSION}|%{RELEASE}|%{ARCH}\n' "$pq_candidate" 2>/dev/null) || return 1
    printf '%s\n' "$pq_candidate_identity" | LC_ALL=C awk -F'|' '
      NF != 5 || $1 !~ /^[ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_.+-]+$/ ||
      $2 !~ /^[0123456789]+$/ || $3 !~ /^[ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_.+~^]+$/ ||
      $4 !~ /^[ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_.+~^]+$/ || ($5 != "noarch" && $5 != "x86_64") { exit 1 }
      END { if (NR != 1) exit 1 }
    ' || return 1
    case $pq_candidates_identities in *"
$pq_candidate_identity
"*) return 1 ;; esac
    pq_candidates_identities="$pq_candidates_identities$pq_candidate_identity
"
    pq_candidates_count=$((pq_candidates_count + 1))
    pq_candidates_output="${pq_candidates_output}${pq_candidates_output:+
}$pq_candidate|$pq_candidate_hash|$pq_candidate_identity"
  done
  [ "$pq_candidates_count" -gt 0 ] || return 1
  # Manifest paths are ASCII; C byte order matches the Python manifest sort.
  printf '%s\n' "$pq_candidates_output" | LC_ALL=C sort -t '|' -k1,1
}

# The manifest must come from authenticated allowed Rocky metadata; this helper
# checks every selected exact archive with one isolated inherited root set.
prereq_rocky_archives_ok() {
  [ "$#" -ge 2 ] && [ "$#" -le 4 ] || return 1
  pq_archives_stage=$1 pq_archives_manifest=$2 pq_archives_mode=${3:-root}
  PREREQ_ARCHIVES_VERIFIED_MANIFEST='' PREREQ_ARCHIVES_VERIFIED_SIZES=''
  pq_archives_snapshot=${4-}
  if [ -z "$pq_archives_snapshot" ]; then pq_archives_snapshot=$(cat "$pq_archives_manifest") || return 1; fi
  [ -n "$pq_archives_snapshot" ] || return 1
  pq_archives_mixed=0
  if [ "$pq_archives_mode" = private_mixed ]; then pq_archives_mode=private; pq_archives_mixed=1; fi
  pq_archives_ring=$pq_archives_stage/rocky-ring
  [ "$pq_archives_manifest" = "$pq_archives_stage/transaction.manifest" ] || return 1
  [ -d "$pq_archives_ring" ] && [ -d "$pq_archives_stage/empty" ] && [ -f "$pq_archives_manifest" ] || return 1
  prereq_stage_path_ok "$pq_archives_mode" "$pq_archives_stage" "$pq_archives_ring" &&
    prereq_stage_path_ok "$pq_archives_mode" "$pq_archives_stage" "$pq_archives_stage/empty" &&
    prereq_stage_path_ok "$pq_archives_mode" "$pq_archives_stage" "$pq_archives_manifest" || return 1
  pq_archives_key_count=0
  for pq_archives_key in "$pq_archives_stage/empty"/*.key; do [ ! -e "$pq_archives_key" ] && [ ! -L "$pq_archives_key" ] || return 1; done
  for pq_archives_key in "$pq_archives_ring"/*.key; do
    [ -f "$pq_archives_key" ] && [ ! -L "$pq_archives_key" ] && prereq_stage_path_ok "$pq_archives_mode" "$pq_archives_stage" "$pq_archives_key" || return 1
    pq_archives_key_count=$((pq_archives_key_count + 1))
  done
  [ "$pq_archives_key_count" -gt 0 ] || return 1
  pq_archives_count=0 pq_archives_receipts='' pq_archives_sizes='' pq_archives_seen='
'
  while IFS='|' read -r pq_archive_file pq_archive_hash pq_archive_name pq_archive_epoch pq_archive_version pq_archive_release pq_archive_arch pq_archive_repo || [ -n "$pq_archive_file" ]; do
    case $pq_archive_file in "$pq_archives_stage"/packages/*) ;; *) return 1 ;; esac
    case $pq_archive_file in *[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_./+~^-]*|*/../*|*/..|*//* ) return 1 ;; esac
    case $pq_archives_seen in *"
$pq_archive_file
"*) return 1 ;; esac
    pq_archives_seen="$pq_archives_seen$pq_archive_file
"
    [ -f "$pq_archive_file" ] && [ ! -L "$pq_archive_file" ] && prereq_stage_path_ok "$pq_archives_mode" "$pq_archives_stage" "$pq_archive_file" || return 1
    pq_archive_size=$(wc -c < "$pq_archive_file") || return 1
    case $pq_archive_size in ''|*[!0123456789]*) return 1 ;; esac
    case $pq_archive_hash in *[!0123456789abcdef]*|'') return 1 ;; esac
    [ "${#pq_archive_hash}" -eq 64 ] || return 1
    case $pq_archive_name in ''|*[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_.+-]*) return 1 ;; esac
    case $pq_archive_epoch in ''|*[!0123456789]*) return 1 ;; esac
    case $pq_archive_version:$pq_archive_release in *[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_.+~^:-]*|:*|*:) return 1 ;; esac
    case $pq_archive_arch in noarch|x86_64) ;; *) return 1 ;; esac
    case $pq_archive_repo in baseos|appstream|crb|extras) ;; epel) [ "$pq_archives_mixed" = 1 ] || return 1 ;; *) return 1 ;; esac
    pq_archive_epel=epel-release
    if [ "$pq_archive_name" = "$pq_archive_epel" ] && [ "$pq_archive_repo" != extras ]; then return 1; fi
    pq_archive_actual_hash=$(sha256sum -- "$pq_archive_file") || return 1
    pq_archive_actual_hash=${pq_archive_actual_hash%% *}
    [ "$pq_archive_actual_hash" = "$pq_archive_hash" ] || return 1
    pq_archive_identity=$(LC_ALL=C rpm -qp --queryformat '%{NAME}|%{EPOCHNUM}|%{VERSION}|%{RELEASE}|%{ARCH}\n' "$pq_archive_file" 2>/dev/null) || return 1
    [ "$pq_archive_identity" = "$pq_archive_name|$pq_archive_epoch|$pq_archive_version|$pq_archive_release|$pq_archive_arch" ] || return 1
    if [ "$pq_archive_repo" = epel ]; then
      [ "$pq_archive_name" != "$pq_archive_epel" ] || return 1
      prereq_epel_candidate_ok "$pq_archives_stage" "$pq_archive_file" "$pq_archive_hash" "$pq_archive_identity" "$PREREQ_EPEL_METALINK" "$PREREQ_EPEL_REPO_GPGCHECK" >/dev/null || return 1
      [ "$(wc -c < "$pq_archive_file")" = "$pq_archive_size" ] || return 1
      pq_archives_sizes="${pq_archives_sizes}${pq_archives_sizes:+
}$pq_archive_file|$pq_archive_size"
      pq_archives_count=$((pq_archives_count + 1))
      pq_archives_receipts="${pq_archives_receipts}${pq_archives_receipts:+
}epel_archive=$pq_archive_identity repository=epel sha256=$pq_archive_hash signature=verified"
      while IFS= read -r pq_archive_receipt_line; do
        [ -z "$pq_archive_receipt_line" ] || pq_archives_receipts="$pq_archives_receipts
epel_$pq_archive_receipt_line"
      done < "$pq_epelcandidate_temp/receipt.txt"
      continue
    fi
    if LC_ALL=C rpmkeys --define '_keyring fs' --define "_keyringpath $pq_archives_stage/empty" \
        --define '_pkgverify_level all' --define '_pkgverify_flags 0' --define '__vsflags 0' \
        --checksig --verbose "$pq_archive_file" >/dev/null 2>&1; then return 1; fi
    pq_archive_verify=$(LC_ALL=C rpmkeys --define '_keyring fs' --define "_keyringpath $pq_archives_ring" \
      --define '_pkgverify_level all' --define '_pkgverify_flags 0' --define '__vsflags 0' \
      --checksig --verbose "$pq_archive_file" 2>/dev/null) || return 1
    printf '%s\n' "$pq_archive_verify" | LC_ALL=C awk '
      { line = tolower($0) }
      line ~ /signature/ && line ~ /: ok[[:space:]]*$/ { signature = 1 }
      line ~ /payload sha256 digest/ && line ~ /: ok[[:space:]]*$/ { digest = 1 }
      END { if (!signature || !digest) exit 1 }
    ' || return 1
    pq_archive_after=$(sha256sum -- "$pq_archive_file") || return 1
    pq_archive_after=${pq_archive_after%% *}
    [ "$pq_archive_after" = "$pq_archive_hash" ] || return 1
    [ "$(wc -c < "$pq_archive_file")" = "$pq_archive_size" ] || return 1
    pq_archives_sizes="${pq_archives_sizes}${pq_archives_sizes:+
}$pq_archive_file|$pq_archive_size"
    pq_archives_count=$((pq_archives_count + 1))
    pq_archives_receipts="${pq_archives_receipts}${pq_archives_receipts:+
}rocky_archive=$pq_archive_identity repository=$pq_archive_repo sha256=$pq_archive_hash signature=verified"
  done <<PREREQ_ARCHIVES_SNAPSHOT
$pq_archives_snapshot
PREREQ_ARCHIVES_SNAPSHOT
  [ "$pq_archives_count" -gt 0 ] || return 1
  prereq_stage_path_ok "$pq_archives_mode" "$pq_archives_stage" "$pq_archives_stage/packages" || return 1
  for pq_archives_found in "$pq_archives_stage/packages"/* "$pq_archives_stage/packages"/.[!.]* "$pq_archives_stage/packages"/..?*; do
    if [ ! -e "$pq_archives_found" ] && [ ! -L "$pq_archives_found" ]; then continue; fi
    case $pq_archives_seen in *"
$pq_archives_found
"*) ;; *) return 1 ;; esac
  done
  PREREQ_ARCHIVES_VERIFIED_MANIFEST=$pq_archives_snapshot PREREQ_ARCHIVES_VERIFIED_SIZES=$pq_archives_sizes
  printf '%s\n' "$pq_archives_receipts"
}

# Bind each archive to official HTTPS Rocky metadata and retain any configured
# metadata-signature check against the inherited Rocky root set.
prereq_metadata_policy_ready() {
  case ,${1-}, in
    *,*=1,*) command -v gpgv >/dev/null 2>&1 || { prereq_msg metadata_verifier_missing; return 1; } ;;
  esac
  return 0
}

prereq_rocky_metadata_ok() {
  [ "$#" -ge 3 ] && [ "$#" -le 5 ] || return 1
  pq_rocky_meta_mode=${4:-verify}
  pq_rocky_meta_mixed=0 pq_rocky_meta_snapshot=${5-}
  case $pq_rocky_meta_mode in
    verify|assign) pq_rocky_meta_path_mode=root ;;
    verify_private) pq_rocky_meta_path_mode=private; pq_rocky_meta_mode=verify ;;
    assign_private) pq_rocky_meta_path_mode=private; pq_rocky_meta_mode=assign ;;
    classify_private) pq_rocky_meta_path_mode=private; pq_rocky_meta_mode=classify ;;
    verify_mixed_private) pq_rocky_meta_path_mode=private; pq_rocky_meta_mode=verify; pq_rocky_meta_mixed=1 ;;
    *) return 1 ;;
  esac
  prereq_release_ok "$1" || return 1
  case $pq_rocky_meta_mode:$2 in verify:*/transaction.manifest|assign:*/candidates.manifest|classify:*/candidates.manifest) ;; *) return 1 ;; esac
  [ -f "$2" ] && [ ! -L "$2" ] && prereq_stage_path_ok "$pq_rocky_meta_path_mode" "${2%/*}" "$2" || return 1
  pq_rocky_meta_roots=''
  case ,$3, in
    *,*=1,*)
      command -v gpgv >/dev/null 2>&1 || { prereq_msg metadata_verifier_missing; return 1; }
      pq_rocky_meta_ring=${2%/*}/rocky-ring
      [ -d "$pq_rocky_meta_ring" ] && prereq_stage_path_ok "$pq_rocky_meta_path_mode" "${2%/*}" "$pq_rocky_meta_ring" || return 1
      for pq_rocky_meta_key in "$pq_rocky_meta_ring"/*.key; do
        [ -f "$pq_rocky_meta_key" ] && [ ! -L "$pq_rocky_meta_key" ] && prereq_stage_path_ok "$pq_rocky_meta_path_mode" "${2%/*}" "$pq_rocky_meta_key" || return 1
        pq_rocky_meta_key_fprs=$(prereq_pgp_fingerprints "$pq_rocky_meta_key") || return 1
        pq_rocky_meta_roots="${pq_rocky_meta_roots}${pq_rocky_meta_roots:+
}$pq_rocky_meta_key_fprs"
      done
      [ -n "$pq_rocky_meta_roots" ] || return 1 ;;
  esac
  python3 -I - "$1" "$2" "$3" "$pq_rocky_meta_roots" "$pq_rocky_meta_mode" "$pq_rocky_meta_snapshot" "$pq_rocky_meta_mixed" <<'PREREQ_ROCKY_METADATA_PY'
import base64
import gzip
import hashlib
import io
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import time
from urllib.parse import urljoin, urlsplit
import urllib.request
import xml.etree.ElementTree as ET

release, manifest, policy_text, root_fingerprints, mode, snapshot, mixed = sys.argv[1:]
channels = {"baseos": "BaseOS", "appstream": "AppStream", "crb": "CRB", "extras": "extras"}

def require(condition):
    if not condition:
        raise ValueError("unverified Rocky metadata")

def local(tag):
    return tag.rsplit("}", 1)[-1]

def one(items):
    require(len(items) == 1)
    return items[0]

def sha(data):
    return hashlib.sha256(data).hexdigest()

def valid_hash(value):
    return len(value) == 64 and all(c in "0123456789abcdef" for c in value)

def xml(data):
    require(b"<!DOCTYPE" not in data.upper() and b"<!ENTITY" not in data.upper())
    return ET.fromstring(data)

ROCKY_PRIMARY_LIMIT = 536870912
ROCKY_MIN_AVAILABLE_KIB = 1048576

def require_rocky_memory(minimum_kib):
    try:
        with open("/proc/meminfo", encoding="ascii") as source:
            rows = [line.split() for line in source if line.startswith("MemAvailable:")]
        if len(rows) != 1 or len(rows[0]) != 3 or rows[0][2] != "kB" or not rows[0][1].isascii() or not rows[0][1].isdigit():
            raise MemoryError("memory availability unavailable")
        if int(rows[0][1]) < minimum_kib:
            raise MemoryError("insufficient available memory")
    except (OSError, UnicodeError, ValueError):
        raise MemoryError("memory availability unavailable") from None

def verify_rocky_primary(packed, expected_hash):
    total = 0
    digest = hashlib.sha256()
    tail = b""
    with gzip.GzipFile(fileobj=io.BytesIO(packed)) as source:
        while True:
            chunk = source.read(min(65536, ROCKY_PRIMARY_LIMIT + 1 - total))
            if not chunk:
                break
            total += len(chunk)
            require(total <= ROCKY_PRIMARY_LIMIT)
            digest.update(chunk)
            scan = (tail + chunk).upper()
            require(b"<!DOCTYPE" not in scan and b"<!ENTITY" not in scan)
            tail = scan[-8:]
    require(digest.hexdigest() == expected_hash)

def iter_rocky_packages(packed):
    with gzip.GzipFile(fileobj=io.BytesIO(packed)) as source:
        root = None
        depth = 0
        for event, element in ET.iterparse(source, events=("start", "end")):
            if event == "start":
                depth += 1
                if depth == 1:
                    root = element
                continue
            if depth == 2:
                if local(element.tag) == "package" and element.get("type") == "rpm":
                    yield element
                element.clear()
                root.remove(element)
            depth -= 1

def get(url, limit):
    if shutil.which("curl"):
        result = subprocess.run(
            ["curl", "--disable", "--fail", "--silent", "--show-error", "--location",
             "--proto", "=https", "--proto-redir", "=https", "--max-time", "45",
             "--max-filesize", str(limit), "--", url],
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
            env={"PATH": os.environ.get("PATH", ""), "LC_ALL": "C"}, check=False,
        )
        require(result.returncode == 0 and len(result.stdout) <= limit)
        return result.stdout
    require(urlsplit(url).scheme == "https")
    class HttpsRedirect(urllib.request.HTTPRedirectHandler):
        def redirect_request(self, req, fp, code, msg, headers, newurl):
            require(urlsplit(newurl).scheme == "https")
            return super().redirect_request(req, fp, code, msg, headers, newurl)
    os.environ.pop("SSL_CERT_FILE", None)
    os.environ.pop("SSL_CERT_DIR", None)
    with urllib.request.build_opener(urllib.request.ProxyHandler({}), HttpsRedirect()).open(url, timeout=45) as response:
        chunks, total, deadline = [], 0, time.monotonic() + 45
        while True:
            require(time.monotonic() < deadline)
            chunk = response.read(min(65536, limit + 1 - total))
            if not chunk:
                break
            chunks.append(chunk)
            total += len(chunk)
            require(total <= limit)
    return b"".join(chunks)

def memory_file(name, data):
    descriptor = os.memfd_create(name, 0)
    view = memoryview(data)
    while view:
        written = os.write(descriptor, view)
        require(written > 0)
        view = view[written:]
    os.lseek(descriptor, 0, os.SEEK_SET)
    return descriptor

def verify_metadata_signature(signature, data):
    allowed = set(root_fingerprints.splitlines())
    require(allowed and all(len(value) == 40 and all(c in "0123456789ABCDEF" for c in value) for value in allowed))
    keyring = b""
    keys = sorted((Path(manifest).parent / "rocky-ring").glob("*.key"))
    require(keys)
    for key in keys:
        text = key.read_text(encoding="ascii")
        blocks = re.findall(r"-----BEGIN PGP PUBLIC KEY BLOCK-----(.*?)-----END PGP PUBLIC KEY BLOCK-----", text, re.S)
        require(blocks)
        for block in blocks:
            lines = (line.strip() for line in block.splitlines())
            payload = "".join(line for line in lines if line and ":" not in line and not line.startswith("="))
            keyring += base64.b64decode(payload, validate=True)
    descriptors = []
    try:
        for name, content in (("roots", keyring), ("signature", signature), ("repomd", data)):
            descriptors.append(memory_file(name, content))
        keyfd, sigfd, datafd = descriptors
        checked = subprocess.run(
            ["gpgv", "--keyring", "/proc/self/fd/" + str(keyfd), "--status-fd", "1",
             "/proc/self/fd/" + str(sigfd), "/proc/self/fd/" + str(datafd)],
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, pass_fds=tuple(descriptors),
            env={"PATH": os.environ.get("PATH", ""), "LC_ALL": "C"}, check=False,
        )
        require(checked.returncode == 0)
        statuses = [line.split() for line in checked.stdout.decode("ascii").splitlines()
                    if line.startswith("[GNUPG:] VALIDSIG ")]
        require(statuses)
        for status in statuses:
            require(len(status) >= 11)
            signer = status[2]
            primary_signer = status[11] if len(status) > 11 else signer
            require(len(signer) == 40 and primary_signer in allowed)
    finally:
        for descriptor in descriptors:
            os.close(descriptor)

try:
    policy = {}
    for part in policy_text.split(","):
        pair = part.split("=")
        require(len(pair) == 2 and pair[0] in channels and pair[0] not in policy and pair[1] in ("0", "1"))
        policy[pair[0]] = pair[1]
    records = []
    manifest_text = snapshot if snapshot else Path(manifest).read_text(encoding="ascii")
    for line in manifest_text.splitlines():
        row = line.split("|")
        require(len(row) == (7 if mode in ("assign", "classify") else 8) and valid_hash(row[1]))
        if mode == "verify":
            if mixed == "1" and row[7] == "epel":
                continue
            require(row[7] in channels and row[7] in policy)
        records.append(row)
    if not records and mixed == "1":
        print("rocky_metadata=no-rocky-archives")
        raise SystemExit(0)
    require(records and len({row[0] for row in records}) == len(records))
    receipts = []
    matches = {row[0]: [] for row in records}
    repos = sorted(policy) if mode in ("assign", "classify") else sorted({row[7] for row in records})
    for repo in repos:
        base = "https://dl.rockylinux.org/pub/rocky/" + release + "/" + channels[repo] + "/x86_64/os/"
        repomd = get(urljoin(base, "repodata/repomd.xml"), 4194304)
        if policy[repo] == "1":
            signature = get(urljoin(base, "repodata/repomd.xml.asc"), 1048576)
            verify_metadata_signature(signature, repomd)
        root = xml(repomd)
        primary = one([e for e in root if local(e.tag) == "data" and e.get("type") == "primary"])
        checksum = one([e for e in primary if local(e.tag) == "checksum" and e.get("type", "").lower() == "sha256"])
        open_checksum = one(list(e for e in primary if local(e.tag) == "open-checksum" and e.get("type", "").lower() == "sha256"))
        location = one([e for e in primary if local(e.tag) == "location"])
        href = location.get("href", "")
        require(href.startswith("repodata/") and href.endswith(".xml.gz") and "//" not in href)
        require(all(c.isascii() and (c.isalnum() or c in "._-/+") for c in href))
        require(all(part not in ("", ".", "..") for part in href.split("/")))
        packed_hash = (checksum.text or "").strip().lower()
        raw_hash = (open_checksum.text or "").strip().lower()
        require(valid_hash(packed_hash) and valid_hash(raw_hash))
        require_rocky_memory(ROCKY_MIN_AVAILABLE_KIB)
        packed = get(urljoin(base, href), 134217728)
        require(sha(packed) == packed_hash)
        # Check the entire expanded stream and its digest before parsing it.
        verify_rocky_primary(packed, raw_hash)
        eligible = [row for row in records if (mode in ("assign", "classify") and (row[2] != "epel-release" or repo == "extras")) or
                    (mode == "verify" and row[7] == repo)]
        found = {row[0]: 0 for row in eligible}
        for package in iter_rocky_packages(packed):
            children = {}
            for child in package:
                key = local(child.tag)
                require(key not in children)
                children[key] = child
            if "version" not in children or "name" not in children or "arch" not in children:
                continue
            version = children["version"]
            identity = [children["name"].text, version.get("epoch"), version.get("ver"), version.get("rel"), children["arch"].text]
            for row in eligible:
                if identity == row[2:7]:
                    check = children["checksum"]
                    if check.get("type", "").lower() == "sha256" and (check.text or "").strip().lower() == row[1]:
                        found[row[0]] += 1
                        matches[row[0]].append(repo)
                    elif mode == "verify":
                        raise ValueError("archive metadata mismatch")
        if mode in ("assign", "classify"):
            require(all(count <= 1 for count in found.values()))
        else:
            require(all(count == 1 for count in found.values()))
        receipts.append("rocky_metadata=" + repo + " repomd_sha256=" + sha(repomd) + " primary_sha256=" + packed_hash +
                        " metadata_signature=" + ("verified" if policy[repo] == "1" else "not-enabled"))
    if mode in ("assign", "classify"):
        if mode == "assign":
            require(all(matches[row[0]] for row in records))
        print("\n".join("|".join(row + [matches[row[0]][0] if matches[row[0]] else "unmatched"]) for row in records))
        print("\n".join(receipts), file=sys.stderr)
    else:
        print("\n".join(receipts))
except MemoryError:
    print("problem: Rocky metadata parsing needs at least 1 GiB currently available on a supported 4 GiB machine", file=sys.stderr)
    print("fix: free memory or use a machine with at least 4 GiB RAM, then retry", file=sys.stderr)
    raise SystemExit(1)
except (OSError, ValueError, KeyError, TypeError, AttributeError, EOFError, OverflowError, ET.ParseError, subprocess.SubprocessError):
    raise SystemExit(1)
PREREQ_ROCKY_METADATA_PY
}

# Install only the locked archives after the full OS-root, metadata and RPM
# signature chain passes; all online repositories are disabled for the install.
prereq_rocky_install() {
  [ "$#" -ge 5 ] || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
  pq_install_stage=$1 pq_install_private=$2 pq_install_release=$3 pq_install_policy=$4; shift 4
  pq_install_manifest=$pq_install_stage/transaction.manifest
  if ! prereq_rocky_seal_ok "$pq_install_stage" "$pq_install_private" "$@"; then
    prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"
  fi
  pq_install_candidates=$(prereq_rocky_candidates "$pq_install_stage") || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
  pq_install_manifest_candidates=$(LC_ALL=C awk -F'|' 'NF == 8 { print $1 "|" $2 "|" $3 "|" $4 "|" $5 "|" $6 "|" $7; next } { bad = 1 } END { if (bad || NR == 0) exit 1 }' "$pq_install_manifest") || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
  if [ "$pq_install_candidates" != "$pq_install_manifest_candidates" ]; then prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; fi
  pq_install_metadata=$(prereq_rocky_metadata_ok "$pq_install_release" "$pq_install_manifest" "$pq_install_policy") || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
  pq_install_archives=$(prereq_rocky_archives_ok "$pq_install_stage" "$pq_install_manifest") || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
  pq_install_roots=$(cat "$pq_install_private/roots.receipt") || { prereq_msg rocky_install_failed || return "${EX_PREREQ:-3}"; return "${EX_PREREQ:-3}"; }
  while IFS= read -r pq_install_line; do [ -z "$pq_install_line" ] || say "$pq_install_line" || return "${EX_PREREQ:-3}"; done <<PREREQ_ROCKY_ROOT_RECEIPT
$pq_install_roots
PREREQ_ROCKY_ROOT_RECEIPT
  say "$pq_install_metadata" || return "${EX_PREREQ:-3}"
  say "$pq_install_archives" || return "${EX_PREREQ:-3}"
  set --
  while IFS='|' read -r pq_install_file pq_install_rest || [ -n "$pq_install_file" ]; do
    [ -n "$pq_install_file" ] || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
    set -- "$@" "$pq_install_file"
  done < "$pq_install_manifest"
  [ "$#" -gt 0 ] || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
  say "Verified Rocky install: sudo dnf --releasever=$pq_install_release --disablerepo=* install -y $*" || return "${EX_PREREQ:-3}"
  if ! ( umask 022; /usr/bin/sudo dnf "--releasever=$pq_install_release" '--disablerepo=*' install -y "$@" ); then
    prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"
  fi
  return 0
}

# Acquire only the exact requested NEVRA through the official metalink's
# checksum chain. Signature and installed-payload checks remain separate.
prereq_epel_fetch() {
  [ "$#" -eq 5 ] || [ "$#" -eq 6 ] || return 1
  case ${6:-compose} in compose|dependency) ;; *) return 1 ;; esac
  case $4 in
    0) ;;
    1) command -v gpg >/dev/null 2>&1 && command -v gpgv >/dev/null 2>&1 || { prereq_msg metadata_verifier_missing; return 1; }
       mkdir -m 700 "$3/metadata-gpg-home" || return 1
       prereq_epel_key_ok "$3/metadata-gpg-home" "$5" || return 1 ;;
    *) return 1 ;;
  esac
  python3 -I - "$1" "$2" "$3" "$4" "$5" "${6:-compose}" <<'PREREQ_EPEL_PY'
import ctypes
import gzip
import hashlib
import http.client
import os
from pathlib import Path
import shutil
import signal
import stat
import subprocess
import sys
import time
from urllib.parse import parse_qsl, urljoin, urlsplit
import urllib.request
import xml.etree.ElementTree as ET

metalink_url, requested, directory, metadata_check, keyfile, selection = sys.argv[1:]
stage = Path(directory)
created = []
deadline = time.monotonic() + 180

def deadline_expired(_signum, _frame):
    raise TimeoutError("EPEL fetch deadline expired")

signal.signal(signal.SIGALRM, deadline_expired)
signal.alarm(180)

def require(condition):
    if not condition:
        raise ValueError("unverified repository data")

def local(tag):
    return tag.rsplit("}", 1)[-1]

def digest(data):
    return hashlib.sha256(data).hexdigest()

def sha256(value):
    require(len(value) == 64 and all(c in "0123456789abcdef" for c in value))
    return value

def xml(data):
    require(b"<!DOCTYPE" not in data.upper() and b"<!ENTITY" not in data.upper())
    return ET.fromstring(data)

def one(items):
    require(len(items) == 1)
    return items[0]

def href(value):
    require(value and not value.startswith("/") and "//" not in value)
    require(all(c.isascii() and (c.isalnum() or c in "._-/+") for c in value))
    require(all(part not in ("", ".", "..") for part in value.split("/")))
    return value

def trusted_zstd_library():
    # Verify each path component, including every symlink traversed, before
    # loading the resolved OS library; never use the dynamic search path.
    for candidate in ("/usr/lib64/libzstd.so.1", "/lib64/libzstd.so.1"):
        pending = candidate.split("/")[1:]
        current = "/"
        links = 0
        try:
            root = os.lstat("/")
            require(root.st_uid == 0 and stat.S_ISDIR(root.st_mode) and not (root.st_mode & 0o022))
            while pending:
                component = pending.pop(0)
                current = os.path.join(current, component)
                info = os.lstat(current)
                require(info.st_uid == 0)
                if stat.S_ISLNK(info.st_mode):
                    links += 1
                    require(links <= 40)
                    target = os.readlink(current)
                    target = target if os.path.isabs(target) else os.path.join(os.path.dirname(current), target)
                    combined = os.path.normpath(os.path.join(target, *pending))
                    require(combined.startswith("/"))
                    pending = combined.split("/")[1:]
                    current = "/"
                    continue
                require(not (info.st_mode & 0o022))
                if pending:
                    require(stat.S_ISDIR(info.st_mode))
                else:
                    require(stat.S_ISREG(info.st_mode))
            require(current.startswith("/usr/lib64/") or current.startswith("/lib64/"))
            return ctypes.CDLL(current)
        except FileNotFoundError:
            continue
    raise ValueError("safe OS zstd library unavailable")

def decode_zstd(compressed, open_size):
    # The decoder and its native binding are trusted verifier code; hashes
    # authenticate bytes but do not isolate native memory-safety faults.
    limit = 268435456
    require(isinstance(open_size, int) and 0 < open_size <= limit)
    require(compressed.startswith(b"\x28\xb5\x2f\xfd"))
    library = trusted_zstd_library()
    library.ZSTD_getFrameContentSize.argtypes = [ctypes.c_void_p, ctypes.c_size_t]
    library.ZSTD_getFrameContentSize.restype = ctypes.c_ulonglong
    library.ZSTD_findFrameCompressedSize.argtypes = [ctypes.c_void_p, ctypes.c_size_t]
    library.ZSTD_findFrameCompressedSize.restype = ctypes.c_size_t
    library.ZSTD_decompress.argtypes = [ctypes.c_void_p, ctypes.c_size_t, ctypes.c_void_p, ctypes.c_size_t]
    library.ZSTD_decompress.restype = ctypes.c_size_t
    library.ZSTD_isError.argtypes = [ctypes.c_size_t]
    library.ZSTD_isError.restype = ctypes.c_uint
    source = ctypes.create_string_buffer(compressed)
    declared = library.ZSTD_getFrameContentSize(source, len(compressed))
    unknown = (1 << 64) - 1
    error = (1 << 64) - 2
    require(declared != error and (declared == unknown or (declared <= limit and declared == open_size)))
    frame_size = library.ZSTD_findFrameCompressedSize(source, len(compressed))
    require(not library.ZSTD_isError(frame_size) and frame_size == len(compressed))
    output = ctypes.create_string_buffer(open_size)
    decoded = library.ZSTD_decompress(output, open_size, source, len(compressed))
    require(not library.ZSTD_isError(decoded) and decoded == open_size)
    return output.raw

def get(url, name, limit):
    require(time.monotonic() < deadline)
    timeout = min(45, max(1, int(deadline - time.monotonic())))
    target = stage / name
    with target.open("xb") as output:
        created.append(target)
        if shutil.which("curl"):
            result = subprocess.run(
                ["curl", "--disable", "--fail", "--silent", "--show-error", "--location",
                 "--proto", "=https", "--proto-redir", "=https", "--max-time", str(timeout),
                 "--max-filesize", str(limit), "--", url],
                stdout=output, stderr=subprocess.DEVNULL,
                env={"PATH": os.environ.get("PATH", ""), "LC_ALL": "C"}, timeout=timeout + 1, check=False,
            )
            require(result.returncode == 0)
        else:
            require(urlsplit(url).scheme == "https")
            class HttpsRedirect(urllib.request.HTTPRedirectHandler):
                def redirect_request(self, req, fp, code, msg, headers, newurl):
                    require(urlsplit(newurl).scheme == "https")
                    return super().redirect_request(req, fp, code, msg, headers, newurl)
            os.environ.pop("SSL_CERT_FILE", None)
            os.environ.pop("SSL_CERT_DIR", None)
            with urllib.request.build_opener(urllib.request.ProxyHandler({}), HttpsRedirect()).open(url, timeout=timeout) as response:
                total = 0
                while True:
                    require(time.monotonic() < deadline)
                    chunk = response.read(min(65536, limit + 1 - total))
                    if not chunk:
                        break
                    total += len(chunk)
                    require(total <= limit)
                    output.write(chunk)
    require(target.stat().st_size <= limit)
    return target.read_bytes()

try:
    st = stage.lstat()
    require(stage.is_absolute() and stat.S_ISDIR(st.st_mode) and not stage.is_symlink())
    require(st.st_uid == os.getuid() and stat.S_IMODE(st.st_mode) == 0o700)
    fields = requested.split("|")
    require(len(fields) == 5 and fields[1].isdigit())
    require((selection == "compose" and fields[0] == "podman-compose") or
            (selection == "dependency" and fields[0] != "epel-release"))
    require(fields[4] in ("noarch", "x86_64"))
    require(all(field and all(c.isascii() and (c.isalnum() or c in "._+-") for c in field) for field in fields))
    parsed = urlsplit(metalink_url)
    require(parsed.scheme == "https" and parsed.netloc == "mirrors.fedoraproject.org" and parsed.path == "/metalink" and not parsed.fragment)
    params = parse_qsl(parsed.query, keep_blank_values=True, strict_parsing=True)
    require(len(dict(params)) == len(params) and all(k in ("repo", "arch", "infra", "content") for k, _ in params))
    query = dict(params)
    repo = query.get("repo", "")
    minor = repo.removeprefix("epel-z-10.")
    require(query.get("arch") == "x86_64" and repo.startswith("epel-z-10.") and minor.isascii() and minor.isdigit())
    for attempt in range(2):
        checkpoint = len(created)
        try:
            metalink_data = get(metalink_url, "metalink.xml", 1048576)
            break
        except (OSError, ValueError, http.client.HTTPException, subprocess.SubprocessError):
            for path in reversed(created[checkpoint:]):
                try:
                    path.unlink()
                except OSError:
                    pass
            del created[checkpoint:]
            if attempt == 1:
                raise
    metalink = xml(metalink_data)
    record = one([e for e in metalink.iter() if local(e.tag) == "file" and e.get("name") == "repomd.xml"])
    verification = one([e for e in record if local(e.tag) == "verification"])
    hashes = [e.text.strip().lower() for e in verification if local(e.tag) == "hash" and e.get("type", "").lower() == "sha256" and e.text]
    expected_repomd = sha256(one(hashes))
    mirrors = []
    for item in record.iter():
        if local(item.tag) != "url" or not item.text:
            continue
        candidate = item.text.strip()
        parts = urlsplit(candidate)
        if (parts.scheme == "https" and parts.hostname and not parts.username and not parts.password and
                parts.port in (None, 443) and not parts.query and not parts.fragment and
                parts.path.endswith("/repodata/repomd.xml") and "%" not in parts.path):
            mirrors.append(candidate)
    attempted = 0
    failures = {}
    succeeded = False
    for repomd_url in mirrors[:8]:
        if time.monotonic() >= deadline:
            break
        attempted += 1
        phase = "index fetch"
        checkpoint = len(created)
        try:
            repomd = get(repomd_url, "repomd.xml", 4194304)
            phase = "index verification"
            require(digest(repomd) == expected_repomd)
            if metadata_check == "1":
                phase = "metadata signature"
                get(repomd_url + ".asc", "repomd.xml.asc", 1048576)
                home = stage / "metadata-gpg-home"
                keyring = stage / "metadata-keyring.gpg"
                require(not keyring.exists() and not keyring.is_symlink())
                created.append(keyring)
                clean_env = {"PATH": os.environ.get("PATH", ""), "LC_ALL": "C"}
                prepared = subprocess.run(
                    ["gpg", "--batch", "--no-options", "--no-tty", "--homedir", str(home),
                     "--dearmor", "--output", str(keyring), keyfile],
                    stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, env=clean_env, timeout=max(0.1, deadline - time.monotonic()), check=False,
                )
                require(prepared.returncode == 0 and keyring.is_file() and not keyring.is_symlink())
                checked = subprocess.run(
                    ["gpgv", "--homedir", str(home), "--keyring", str(keyring), "--status-fd", "1",
                     str(stage / "repomd.xml.asc"), str(stage / "repomd.xml")],
                    stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, env=clean_env, timeout=max(0.1, deadline - time.monotonic()), check=False,
                )
                require(checked.returncode == 0)
                statuses = [line.split() for line in checked.stdout.decode("ascii").splitlines()
                            if line.startswith("[GNUPG:] VALIDSIG ")]
                require(statuses)
                for status in statuses:
                    require(len(status) >= 11)
                    signer = status[2]
                    primary_signer = status[11] if len(status) > 11 else signer
                    require(len(signer) == 40 and primary_signer == "7D8D15CBFC4E62688591FB2633D98517E37ED158")
            root = xml(repomd)
            primary = one([e for e in root if local(e.tag) == "data" and e.get("type") == "primary"])
            checksum = one([e for e in primary if local(e.tag) == "checksum" and e.get("type", "").lower() == "sha256"])
            open_checksum = one(list(e for e in primary if local(e.tag) == "open-checksum" and e.get("type", "").lower() == "sha256"))
            location = one([e for e in primary if local(e.tag) == "location"])
            primary_href = href(location.get("href", ""))
            require(primary_href.endswith((".xml.gz", ".xml.zst")))
            base = repomd_url[:-len("repodata/repomd.xml")]
            primary_name = "primary.xml.zst" if primary_href.endswith(".xml.zst") else "primary.xml.gz"
            phase = "primary fetch"
            compressed = get(urljoin(base, primary_href), primary_name, 134217728)
            phase = "primary verification"
            require(digest(compressed) == sha256((checksum.text or "").strip().lower()))
            if primary_name == "primary.xml.zst":
                open_size = one([e for e in primary if local(e.tag) == ("open" + "-size")])
                size_text = (open_size.text or "").strip()
                require(size_text.isascii() and size_text.isdigit() and len(size_text) <= 9)
                raw = decode_zstd(compressed, int(size_text))
            else:
                with gzip.open(stage / "primary.xml.gz", "rb") as source:
                    raw = source.read(268435457)
            require(len(raw) <= 268435456 and digest(raw) == sha256((open_checksum.text or "").strip().lower()))
            packages = []
            for package in xml(raw):
                if local(package.tag) != "package" or package.get("type") != "rpm":
                    continue
                children = {}
                for child in package:
                    name = local(child.tag)
                    require(name not in children)
                    children[name] = child
                version = children.get("version")
                if version is None or "name" not in children or "arch" not in children:
                    continue
                identity = [children["name"].text, version.get("epoch"), version.get("ver"), version.get("rel"), children["arch"].text]
                if identity == fields:
                    packages.append(children)
            package = one(packages)
            package_hash = package["checksum"]
            require(package_hash.get("type", "").lower() == "sha256")
            expected_package = sha256((package_hash.text or "").strip().lower())
            package_href = href(package["location"].get("href", ""))
            require(package_href.startswith("Packages/") and package_href.endswith(".rpm"))
            package_url = urljoin(base, package_href)
            phase = "package fetch"
            archive = get(package_url, "compose.rpm", 134217728)
            phase = "package verification"
            require(digest(archive) == expected_package)
            receipt = stage / "receipt.txt"
            with receipt.open("x", encoding="utf-8") as output:
                created.append(receipt)
                output.write("repository=stable-epel\nmetalink=" + metalink_url + "\nrepomd=" + repomd_url +
                             "\nrepomd_sha256=" + expected_repomd + "\npackage=" + package_url +
                             "\npackage_sha256=" + expected_package + "\nmetadata_signature=" +
                             ("verified" if metadata_check == "1" else "not-enabled") + "\nnevra=" + requested + "\n")
        except (OSError, ValueError, KeyError, TypeError, AttributeError, ctypes.ArgumentError, EOFError, OverflowError, ET.ParseError, http.client.HTTPException, subprocess.SubprocessError):
            failures[phase] = failures.get(phase, 0) + 1
            for path in reversed(created[checkpoint:]):
                try:
                    path.unlink()
                except OSError:
                    pass
            del created[checkpoint:]
            continue
        succeeded = True
        break
    if not succeeded:
        reasons = ", ".join("%s=%d" % pair for pair in sorted(failures.items())) or "deadline or no eligible mirrors"
        print("EPEL mirrors failed: %d tried (%s)" % (attempted, reasons), file=sys.stderr)
        raise ValueError("all mirrors failed")
except (OSError, ValueError, KeyError, TypeError, AttributeError, ctypes.ArgumentError, EOFError, OverflowError, ET.ParseError, http.client.HTTPException, subprocess.SubprocessError):
    for path in reversed(created):
        try:
            path.unlink()
        except OSError:
            pass
    raise SystemExit(1)
PREREQ_EPEL_PY
}

# A negative control proves that this rpmkeys invocation does not accept a
# key from the system store. The caller supplies fresh private directories.
prereq_rpm_signature_ok() {
  [ "$#" -eq 4 ] || return 1
  pq_sig_home=$1 pq_sig_empty=$2 pq_sig_pinned=$3 pq_sig_archive=$4
  for pq_sig_dir in "$pq_sig_home" "$pq_sig_empty" "$pq_sig_pinned"; do
    case $pq_sig_dir in /*) ;; *) return 1 ;; esac
    case $pq_sig_dir in *[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_./-]*|*/../*|*/..|*//* ) return 1 ;; esac
    [ -d "$pq_sig_dir" ] && [ ! -L "$pq_sig_dir" ] || return 1
  done
  [ "$pq_sig_empty" != "$pq_sig_pinned" ] || return 1
  for pq_sig_key in "$pq_sig_empty"/*.key; do
    [ ! -e "$pq_sig_key" ] && [ ! -L "$pq_sig_key" ] || return 1
  done
  [ -f "$pq_sig_pinned/certificate.key" ] && [ ! -L "$pq_sig_pinned/certificate.key" ] || return 1
  for pq_sig_key in "$pq_sig_pinned"/*.key; do
    [ "$pq_sig_key" = "$pq_sig_pinned/certificate.key" ] || return 1
  done
  prereq_epel_key_ok "$pq_sig_home" "$pq_sig_pinned/certificate.key" || return 1
  if LC_ALL=C rpmkeys --define '_keyring fs' --define "_keyringpath $pq_sig_empty" \
      --define '_pkgverify_level all' --define '_pkgverify_flags 0' --define '__vsflags 0' \
      --checksig --verbose "$pq_sig_archive" >/dev/null 2>&1; then return 1; fi
  pq_sig_output=$(LC_ALL=C rpmkeys --define '_keyring fs' --define "_keyringpath $pq_sig_pinned" \
    --define '_pkgverify_level all' --define '_pkgverify_flags 0' --define '__vsflags 0' \
    --checksig --verbose "$pq_sig_archive" 2>/dev/null) || return 1
  printf '%s\n' "$pq_sig_output" | LC_ALL=C awk '
    { line = tolower($0) }
    line ~ /signature/ && line ~ /: ok[[:space:]]*$/ { signature = 1 }
    line ~ /payload sha256 digest/ && line ~ /: ok[[:space:]]*$/ { digest = 1 }
    END { if (!signature || !digest) exit 1 }
  '
}

# Bind a privately downloaded EPEL candidate to an independently authenticated
# stable-repository copy and verify the candidate itself under the pinned key.
prereq_epel_candidate_ok() {
  [ "$#" -eq 6 ] || return 1
  pq_epelcandidate_stage=$1 pq_epelcandidate_file=$2 pq_epelcandidate_hash=$3 pq_epelcandidate_identity=$4 pq_epelcandidate_url=$5 pq_epelcandidate_policy=$6
  case $pq_epelcandidate_file in "$pq_epelcandidate_stage"/packages/*.rpm) ;; *) return 1 ;; esac
  case $pq_epelcandidate_hash in ''|*[!0123456789abcdef]*) return 1 ;; esac
  [ "${#pq_epelcandidate_hash}" -eq 64 ] || return 1
  for pq_epelcandidate_path in "$pq_epelcandidate_file" "$pq_epelcandidate_stage/gpg" "$pq_epelcandidate_stage/empty" "$pq_epelcandidate_stage/pinned" "$pq_epelcandidate_stage/pinned/certificate.key"; do
    prereq_private_path "$pq_epelcandidate_stage" "$pq_epelcandidate_path" || return 1
  done
  [ -f "$pq_epelcandidate_file" ] && [ ! -L "$pq_epelcandidate_file" ] || return 1
  pq_epelcandidate_actual=$(sha256sum -- "$pq_epelcandidate_file") || return 1
  [ "${pq_epelcandidate_actual%% *}" = "$pq_epelcandidate_hash" ] || return 1
  pq_epelcandidate_actual=$(LC_ALL=C rpm -qp --queryformat '%{NAME}|%{EPOCHNUM}|%{VERSION}|%{RELEASE}|%{ARCH}\n' "$pq_epelcandidate_file" 2>/dev/null) || return 1
  [ "$pq_epelcandidate_actual" = "$pq_epelcandidate_identity" ] || return 1
  pq_epelcandidate_number=${pq_epelcandidate_number:-0}
  case $pq_epelcandidate_number in *[!0123456789]*|'') return 1 ;; esac
  [ "${#pq_epelcandidate_number}" -le 6 ] || return 1
  pq_epelcandidate_number=$((pq_epelcandidate_number + 1))
  pq_epelcandidate_temp=$pq_epelcandidate_stage/epel-candidate-$pq_epelcandidate_number
  mkdir -m 700 "$pq_epelcandidate_temp" || return 1
  prereq_epel_fetch "$pq_epelcandidate_url" "$pq_epelcandidate_identity" "$pq_epelcandidate_temp" "$pq_epelcandidate_policy" "$pq_epelcandidate_stage/pinned/certificate.key" dependency || return 1
  pq_epelcandidate_reference=$(sha256sum -- "$pq_epelcandidate_temp/compose.rpm") || return 1
  [ "${pq_epelcandidate_reference%% *}" = "$pq_epelcandidate_hash" ] || return 1
  prereq_rpm_signature_ok "$pq_epelcandidate_stage/gpg" "$pq_epelcandidate_stage/empty" "$pq_epelcandidate_stage/pinned" "$pq_epelcandidate_file" || return 1
  pq_epelcandidate_after=$(sha256sum -- "$pq_epelcandidate_file") || return 1
  [ "${pq_epelcandidate_after%% *}" = "$pq_epelcandidate_hash" ] || return 1
  printf '%s|%s|%s|epel\n' "$pq_epelcandidate_file" "$pq_epelcandidate_hash" "$pq_epelcandidate_identity"
}

# Compare identity and the package verification result after the archive has
# independently passed the repository and isolated-signature checks.
prereq_rpm_payload_ok() {
  [ "$#" -eq 2 ] || return 1
  pq_payload_archive=$1 pq_payload_path=$2
  case $pq_payload_path in /*) ;; *) return 1 ;; esac
  [ -f "$pq_payload_path" ] && [ -x "$pq_payload_path" ] && [ ! -L "$pq_payload_path" ] || return 1
  pq_payload_format='%{NAME}|%{EPOCHNUM}|%{VERSION}|%{RELEASE}|%{ARCH}\n'
  pq_payload_installed=$(LC_ALL=C rpm -q --queryformat "$pq_payload_format" podman-compose 2>/dev/null) || return 1
  pq_payload_archive_id=$(LC_ALL=C rpm -qp --queryformat "$pq_payload_format" "$pq_payload_archive" 2>/dev/null) || return 1
  pq_payload_owner=$(LC_ALL=C rpm -qf --queryformat "$pq_payload_format" "$pq_payload_path" 2>/dev/null) || return 1
  [ "$pq_payload_installed" = "$pq_payload_archive_id" ] && [ "$pq_payload_installed" = "$pq_payload_owner" ] || return 1
  printf '%s\n' "$pq_payload_installed" | LC_ALL=C awk -F'|' '
    NF != 5 || $1 != "podman-compose" || $2 !~ /^[0123456789]+$/ ||
    $3 !~ /^[0123456789.]+$/ || $4 !~ /^[ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._]+$/ ||
    ($5 != "noarch" && $5 != "x86_64") { exit 1 }
    END { if (NR != 1) exit 1 }
  ' || return 1
  pq_payload_version=${pq_payload_installed#*|}
  pq_payload_version=${pq_payload_version#*|}
  pq_payload_version=${pq_payload_version%%|*}
  prereq_compose_version_ok "$pq_payload_version" || return 1
  pq_payload_algorithm=$(LC_ALL=C rpm -qp --queryformat '%{FILEDIGESTALGO}\n' "$pq_payload_archive" 2>/dev/null) || return 1
  [ "$pq_payload_algorithm" = 8 ] || return 1
  pq_payload_manifest=$(LC_ALL=C rpm -qp --queryformat '[%{FILENAMES}|%{FILEFLAGS}|%{FILEVERIFYFLAGS}|%{FILEMODES}\n]' "$pq_payload_archive" 2>/dev/null) || return 1
  printf '%s\n' "$pq_payload_manifest" | LC_ALL=C awk -F'|' -v provider="$pq_payload_path" '
    function has(value, bit) { return int(value / bit) % 2 == 1 }
    NF != 4 || $2 !~ /^[0123456789]+$/ || $3 !~ /^[0123456789]+$/ || $4 !~ /^[0123456789]+$/ { bad = 1; next }
    $1 == provider || index($1, "/site-packages/podman_compose") {
      if (has($2, 8) || has($2, 64) || !has($3, 8) || !has($3, 16) || !has($3, 64)) bad = 1
      type = int($4 / 4096) % 16
      if (type == 8 && (!has($3, 1) || !has($3, 2))) bad = 1
      if (type != 8 && type != 4) bad = 1
      if ($1 == provider && type == 8) executable++
      if ($1 ~ /\/site-packages\/podman_compose\.py$/ && type == 8) module++
    }
    END { if (bad || executable != 1 || module != 1) exit 1 }
  ' || return 1
  pq_payload_changes=$(LC_ALL=C rpm -Vp --noscripts "$pq_payload_archive" 2>/dev/null) || return 1
  [ -z "$pq_payload_changes" ]
}

# Fail closed until the RPM archive, signer and installed payload are verified.
prereq_compose_path() {
  return 1
}

# The staging directory is later removed by path, so no other user may be
# able to replace any directory in the path leading to it.
prereq_home_safe() {
  case ${HOME:-} in /*) pq_home_path=$HOME ;; *) return 1 ;; esac
  case $pq_home_path in *[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_./-]*|*/../*|*/..|*//* ) return 1 ;; esac
  pq_home_uid=$(id -u 2>/dev/null) || return 1
  case $pq_home_uid in ''|*[!0123456789]*|0) return 1 ;; esac
  pq_home_first=1
  while :; do
    [ -d "$pq_home_path" ] && [ ! -L "$pq_home_path" ] || return 1
    pq_home_owner_mode=$(stat -c '%u|%a' -- "$pq_home_path" 2>/dev/null) || return 1
    printf '%s\n' "$pq_home_owner_mode" | LC_ALL=C awk -F'|' -v uid="$pq_home_uid" -v first="$pq_home_first" '
      NF != 2 || $1 !~ /^[0123456789]+$/ || $2 !~ /^[01234567]+$/ || length($2) < 3 || length($2) > 4 { exit 1 }
      { group = int($2 / 10) % 10; other = $2 % 10
        if ((first && $1 != uid) || ($1 != 0 && $1 != uid) || group == 2 || group == 3 || group == 6 || group == 7 || other == 2 || other == 3 || other == 6 || other == 7) exit 1 }
      END { if (NR != 1) exit 1 }
    ' || return 1
    [ "$pq_home_path" != / ] || break
    pq_home_path=${pq_home_path%/*}
    [ -n "$pq_home_path" ] || pq_home_path=/
    pq_home_first=0
  done
  return 0
}

prereq_requalify_error() {
  PREREQ_COMPOSE=''
  if [ -n "${pq_requalify_stage:-}" ]; then rm -rf -- "$pq_requalify_stage"; fi
  prereq_msg compose_verify_failed
  return "${EX_PREREQ:-3}"
}

# Verify the installed provider without replacing it or changing the machine.
prereq_requalify_compose_inner() {
  (
  prereq_requalify_compose_inner_body 3>&- >&2 || exit "$?"
  printf '%s\n' "$pq_requalify_provider" || exit "${EX_PREREQ:-3}"
  )
}

prereq_requalify_compose_inner_body() {
  prereq_harden_environment || return 1
  PREREQ_COMPOSE='' pq_requalify_stage=''
  trap 'if [ -n "$pq_requalify_stage" ]; then rm -rf -- "$pq_requalify_stage"; fi' EXIT
  trap 'exit 130' HUP INT TERM
  prereq_compose_path() { return 1; }
  case ${HOME:-} in /*) ;; *) prereq_requalify_error; return "${EX_PREREQ:-3}" ;; esac
  if ! prereq_home_safe; then prereq_msg home_unsafe || return "${EX_PREREQ:-3}"; return "${EX_PREREQ:-3}"; fi
  pq_requalify_stage=$(mktemp -d "$HOME/.cortex-prereq.XXXXXXXX") || { prereq_requalify_error; return "${EX_PREREQ:-3}"; }
  case $pq_requalify_stage in "$HOME"/.cortex-prereq.*) ;; *) pq_requalify_stage=''; prereq_requalify_error; return "${EX_PREREQ:-3}" ;; esac
  if ! mkdir -m 700 "$pq_requalify_stage/gpg" "$pq_requalify_stage/empty" "$pq_requalify_stage/pinned"; then
    prereq_requalify_error; return "${EX_PREREQ:-3}"
  fi
  if ! prereq_os_ok "$(prereq_system_root)/etc/os-release"; then
    prereq_requalify_error; return "${EX_PREREQ:-3}"
  fi
  pq_requalify_arch=$(uname -m 2>/dev/null) || pq_requalify_arch=''
  if [ "$pq_requalify_arch" != x86_64 ]; then
    prereq_msg arch_unsupported || return "${EX_PREREQ:-3}"
    return "${EX_PREREQ:-3}"
  fi
  if [ -n "${PREREQ_EPEL_BOOT_STAGE:-}" ]; then
    pq_requalify_boot=$PREREQ_EPEL_BOOT_STAGE
    pq_requalify_boot_key=$pq_requalify_boot/epel-pinned/certificate.key pq_requalify_boot_repo=$pq_requalify_boot/epel-config/epel.repo
    if ! prereq_private_path "$pq_requalify_boot" "$pq_requalify_boot_key" || ! prereq_private_path "$pq_requalify_boot" "$pq_requalify_boot_repo" ||
       ! mkdir -m 700 "$pq_requalify_stage/epel-config" || ! cp -- "$pq_requalify_boot_key" "$pq_requalify_stage/pinned/certificate.key" ||
       ! cp -- "$pq_requalify_boot_repo" "$pq_requalify_stage/epel-config/epel.repo"; then
      prereq_requalify_error; return "${EX_PREREQ:-3}"
    fi
    pq_requalify_key_hash=$(sha256sum -- "$pq_requalify_stage/pinned/certificate.key") || pq_requalify_key_hash=''
    pq_requalify_repo_hash=$(sha256sum -- "$pq_requalify_stage/epel-config/epel.repo") || pq_requalify_repo_hash=''
    if [ "${pq_requalify_key_hash%% *}" != "${PREREQ_EPEL_BOOT_KEY_HASH:-}" ] || [ "${pq_requalify_repo_hash%% *}" != "${PREREQ_EPEL_BOOT_CONFIG_HASH:-}" ] ||
       ! prereq_epel_key_ok "$pq_requalify_stage/gpg" "$pq_requalify_stage/pinned/certificate.key" ||
       ! HOME="$pq_requalify_stage" TMPDIR="$pq_requalify_stage" XDG_CACHE_HOME="$pq_requalify_stage" prereq_epel_repo_config_ok "$pq_os_version" "$pq_requalify_stage"; then
      prereq_requalify_error; return "${EX_PREREQ:-3}"
    fi
  elif ! cp -- "$(prereq_system_root)/etc/pki/rpm-gpg/RPM-GPG-KEY-EPEL-10" "$pq_requalify_stage/pinned/certificate.key" ||
       ! prereq_epel_key_ok "$pq_requalify_stage/gpg" "$pq_requalify_stage/pinned/certificate.key" ||
       ! HOME="$pq_requalify_stage" TMPDIR="$pq_requalify_stage" XDG_CACHE_HOME="$pq_requalify_stage" prereq_epel_repo_config_ok "$pq_os_version"; then
    prereq_requalify_error; return "${EX_PREREQ:-3}"
  fi
  pq_requalify_identity=$(LC_ALL=C rpm -q --queryformat '%{NAME}|%{EPOCHNUM}|%{VERSION}|%{RELEASE}|%{ARCH}\n' podman-compose 2>/dev/null) || pq_requalify_identity=''
  pq_requalify_provider=$(prereq_system_root)/usr/bin/podman-compose
  if [ -z "$pq_requalify_identity" ] ||
     ! prereq_epel_fetch "$PREREQ_EPEL_METALINK" "$pq_requalify_identity" "$pq_requalify_stage" "$PREREQ_EPEL_REPO_GPGCHECK" "$pq_requalify_stage/pinned/certificate.key" ||
     ! prereq_rpm_signature_ok "$pq_requalify_stage/gpg" "$pq_requalify_stage/empty" "$pq_requalify_stage/pinned" "$pq_requalify_stage/compose.rpm" ||
     ! prereq_rpm_payload_ok "$pq_requalify_stage/compose.rpm" "$pq_requalify_provider"; then
    prereq_requalify_error; return "${EX_PREREQ:-3}"
  fi
  pq_requalify_rpm=$(LC_ALL=C rpm --version 2>/dev/null) || { prereq_requalify_error; return "${EX_PREREQ:-3}"; }
  pq_requalify_rpmkeys=$(LC_ALL=C rpmkeys --version 2>/dev/null) || { prereq_requalify_error; return "${EX_PREREQ:-3}"; }
  if command -v gpg >/dev/null 2>&1; then
    pq_requalify_gpg=$(LC_ALL=C gpg --version 2>/dev/null) || { prereq_requalify_error; return "${EX_PREREQ:-3}"; }
  else
    pq_requalify_gpg='not installed; primary fingerprint derived locally'
  fi
  pq_requalify_newline='
'
  pq_requalify_rpm=${pq_requalify_rpm%%"$pq_requalify_newline"*}
  pq_requalify_rpmkeys=${pq_requalify_rpmkeys%%"$pq_requalify_newline"*}
  pq_requalify_gpg=${pq_requalify_gpg%%"$pq_requalify_newline"*}
  say 'Compose requalification: private pinned EPEL certificate and isolated RPM keyring verified.' || return "${EX_PREREQ:-3}"
  say "Verifier rpm: $pq_requalify_rpm" || return "${EX_PREREQ:-3}"
  say "Verifier rpmkeys: $pq_requalify_rpmkeys" || return "${EX_PREREQ:-3}"
  say "Verifier gpg: $pq_requalify_gpg" || return "${EX_PREREQ:-3}"
  say 'RPM verifier: private filesystem keyring, signatures and digests required, empty-keyring negative control passed.' || return "${EX_PREREQ:-3}"
  pq_requalify_fingerprints=$(LC_ALL=C awk -F: '$1 == "fpr" && length($10) == 40 && $10 ~ /^[0123456789ABCDEF]+$/ { print $10 }' <<PREREQ_KEYS
$pq_key_records
PREREQ_KEYS
  ) || { prereq_requalify_error; return "${EX_PREREQ:-3}"; }
  while IFS= read -r pq_requalify_line; do
    [ -z "$pq_requalify_line" ] || say "Certificate fingerprint: $pq_requalify_line" || return "${EX_PREREQ:-3}"
  done <<PREREQ_FINGERPRINTS
$pq_requalify_fingerprints
PREREQ_FINGERPRINTS
  while IFS= read -r pq_requalify_line; do
    case $pq_requalify_line in *Signature*|*signature*|*digest*) say "RPM verification: $pq_requalify_line" || return "${EX_PREREQ:-3}" ;; esac
  done <<PREREQ_RPM_OUTPUT
$pq_sig_output
PREREQ_RPM_OUTPUT
  while IFS= read -r pq_requalify_line; do say "$pq_requalify_line" || return "${EX_PREREQ:-3}"; done < "$pq_requalify_stage/receipt.txt" || return "${EX_PREREQ:-3}"
  say "Verified installed package: $pq_requalify_identity" || return "${EX_PREREQ:-3}"
  say "Verified absolute provider: $pq_requalify_provider" || return "${EX_PREREQ:-3}"
  if ! rm -rf -- "$pq_requalify_stage"; then prereq_requalify_error; return "${EX_PREREQ:-3}"; fi
  pq_requalify_stage=''
  return 0
}

prereq_requalify_compose() {
  prereq_secure_call prereq_requalify_compose_body
}

prereq_requalify_compose_body() {
  PREREQ_COMPOSE=''
  pq_requalify_result=$(prereq_requalify_compose_inner 3>&-) || {
    pq_requalify_status=$?
    prereq_compose_path() { return 1; }
    return "$pq_requalify_status"
  }
  case $pq_requalify_result in /*) ;; *) prereq_msg compose_verify_failed || return "${EX_PREREQ:-3}"; return "${EX_PREREQ:-3}" ;; esac
  pq_verified_path=$pq_requalify_result PREREQ_COMPOSE=$pq_requalify_result
  prereq_compose_path() {
    [ -n "${PREREQ_COMPOSE:-}" ] && [ "$PREREQ_COMPOSE" = "$pq_verified_path" ] &&
      [ -f "$PREREQ_COMPOSE" ] && [ -x "$PREREQ_COMPOSE" ] && [ ! -L "$PREREQ_COMPOSE" ] || return 1
    printf '%s\n' "$PREREQ_COMPOSE" || return "${EX_PREREQ:-3}"
  }
  return 0
}

# Translate only the exact package names emitted by machine_look into a plan.
prereq_parse_missing() {
  PREREQ_PLAN_PACKAGES='' PREREQ_PLAN_COMPOSE=''
  pq_plan_newline='
'
  pq_plan_seen=$pq_plan_newline
  while IFS= read -r pq_plan_item || [ -n "$pq_plan_item" ]; do
    [ -n "$pq_plan_item" ] || continue
    case $pq_plan_seen in *"$pq_plan_newline$pq_plan_item$pq_plan_newline"*) return 1 ;; esac
    pq_plan_seen="$pq_plan_seen$pq_plan_item$pq_plan_newline"
    case $pq_plan_item in
      'podman-compose (requalify)') [ -z "$PREREQ_PLAN_COMPOSE" ] || return 1; PREREQ_PLAN_COMPOSE=verify ;;
      podman-compose) [ -z "$PREREQ_PLAN_COMPOSE" ] || return 1; PREREQ_PLAN_COMPOSE=install ;;
      podman|curl|tar|gzip|python3|openssh|gnupg2|rpm|gawk|sed|grep|coreutils|findutils)
        PREREQ_PLAN_PACKAGES="${PREREQ_PLAN_PACKAGES}${PREREQ_PLAN_PACKAGES:+ }$pq_plan_item" ;;
      *) return 1 ;;
    esac
  done <<PREREQ_MISSING_ITEMS
${PREREQ_MISSING:-}
PREREQ_MISSING_ITEMS
  [ -n "$PREREQ_PLAN_COMPOSE" ]
}

# Resolve and download as the user into the private stage, before the plan;
# no privileged command or installation occurs during this phase.
prereq_rocky_download() {
  [ "$#" -eq 3 ] || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
  pq_download_stage=$1 pq_download_release=$2 pq_download_group=$3
  prereq_release_ok "$pq_download_release" || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
  if ! prereq_parse_missing || [ ! -d "$pq_download_stage/packages" ] || ! prereq_private_path "$pq_download_stage" "$pq_download_stage/packages"; then
    prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"
  fi
  for pq_download_existing in "$pq_download_stage/packages"/* "$pq_download_stage/packages"/.[!.]* "$pq_download_stage/packages"/..?*; do
    if [ -e "$pq_download_existing" ] || [ -L "$pq_download_existing" ]; then prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; fi
  done
  case $pq_download_group in
    main)
      set --
      for pq_download_package in $PREREQ_PLAN_PACKAGES; do set -- "$@" "$pq_download_package"; done
      [ "$#" -gt 0 ] || return 0
      if ! HOME="$pq_download_stage" TMPDIR="$pq_download_stage" XDG_CACHE_HOME="$pq_download_stage" dnf "--releasever=$pq_download_release" --assumeno \
          "--setopt=cachedir=$pq_download_stage/dnf-cache" "--setopt=logdir=$pq_download_stage/dnf-log" "--setopt=persistdir=$pq_download_stage/dnf-persist" --repo=baseos --repo=appstream --repo=crb \
          download --resolve "--destdir=$pq_download_stage/packages" "$@"; then
        prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"
      fi ;;
    mixed)
      [ "$PREREQ_PLAN_COMPOSE" = install ] || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
      set --
      for pq_download_package in $PREREQ_PLAN_PACKAGES; do set -- "$@" "$pq_download_package"; done
      set -- "$@" podman-compose
      if [ -n "${PREREQ_EPEL_PRIVATE_STAGE:-}" ]; then
        [ "$PREREQ_EPEL_PRIVATE_STAGE" = "$pq_download_stage" ] || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
        if ! HOME="$pq_download_stage" TMPDIR="$pq_download_stage" XDG_CACHE_HOME="$pq_download_stage" dnf "--releasever=$pq_download_release" --assumeno \
            "--setopt=cachedir=$pq_download_stage/dnf-cache" "--setopt=logdir=$pq_download_stage/dnf-log" "--setopt=persistdir=$pq_download_stage/dnf-persist" \
            "--setopt=reposdir=$(prereq_system_root)/etc/yum.repos.d,$pq_download_stage/epel-config" "--setopt=epel.gpgkey=file://$pq_download_stage/pinned/certificate.key" \
            --repo=baseos --repo=appstream --repo=crb --repo=epel download --resolve "--destdir=$pq_download_stage/packages" "$@"; then
          prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"
        fi
      elif ! HOME="$pq_download_stage" TMPDIR="$pq_download_stage" XDG_CACHE_HOME="$pq_download_stage" dnf "--releasever=$pq_download_release" --assumeno \
          "--setopt=cachedir=$pq_download_stage/dnf-cache" "--setopt=logdir=$pq_download_stage/dnf-log" "--setopt=persistdir=$pq_download_stage/dnf-persist" --repo=baseos --repo=appstream --repo=crb --repo=epel \
          download --resolve "--destdir=$pq_download_stage/packages" "$@"; then
        prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"
      fi ;;
    extras)
      if ! HOME="$pq_download_stage" TMPDIR="$pq_download_stage" XDG_CACHE_HOME="$pq_download_stage" dnf "--releasever=$pq_download_release" --assumeno \
          "--setopt=cachedir=$pq_download_stage/dnf-cache" "--setopt=logdir=$pq_download_stage/dnf-log" "--setopt=persistdir=$pq_download_stage/dnf-persist" --repo=extras --repo=baseos --repo=appstream --repo=crb \
          download --resolve "--destdir=$pq_download_stage/packages" epel-release; then
        prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"
      fi ;;
    *) prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}" ;;
  esac
  return 0
}

# Resolve and verify the complete Rocky closure before presenting the plan.
prereq_rocky_preverify() {
  [ "$#" -ge 4 ] || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
  pq_pre_stage=$1 pq_pre_release=$2 pq_pre_group=$3; shift 3
  PREREQ_ROCKY_VERIFIED_MANIFEST='' PREREQ_ROCKY_VERIFIED_SIZES='' PREREQ_ROCKY_PRE_STAGE='' PREREQ_ROCKY_POLICY='' PREREQ_ROCKY_METADATA_RECEIPT='' PREREQ_ROCKY_ARCHIVE_RECEIPT='' PREREQ_MIXED=0 PREREQ_BOOT_FILES='' PREREQ_BOOT_ONLY_OK=0
  case $pq_pre_group in main) pq_pre_repos='baseos appstream crb' ;; extras) pq_pre_repos='baseos appstream crb extras' ;; *) prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}" ;; esac
  if ! prereq_private_path "$pq_pre_stage" "$pq_pre_stage" ||
     ! mkdir -m 700 "$pq_pre_stage/packages" "$pq_pre_stage/empty" ||
     ! prereq_rocky_roots "$pq_pre_stage" "$@"; then
    prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"
  fi
  pq_pre_policy=''
  for pq_pre_repo in $pq_pre_repos; do
    if ! HOME="$pq_pre_stage" TMPDIR="$pq_pre_stage" XDG_CACHE_HOME="$pq_pre_stage" prereq_rocky_repo_ok "$pq_pre_release" "$pq_pre_repo"; then
      prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"
    fi
    pq_pre_policy="${pq_pre_policy}${pq_pre_policy:+,}$pq_pre_repo=$PREREQ_ROCKY_REPO_GPGCHECK"
  done
  prereq_metadata_policy_ready "$pq_pre_policy" || return "${EX_PREREQ:-3}"
  if ! prereq_rocky_download "$pq_pre_stage" "$pq_pre_release" "$pq_pre_group" ||
     ! prereq_rocky_candidates "$pq_pre_stage" private > "$pq_pre_stage/candidates.manifest"; then
    prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"
  fi
  pq_pre_snapshot=$(prereq_rocky_metadata_ok "$pq_pre_release" "$pq_pre_stage/candidates.manifest" "$pq_pre_policy" assign_private 2> "$pq_pre_stage/metadata.receipt") || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
  if [ -z "$pq_pre_snapshot" ] || ! printf '%s\n' "$pq_pre_snapshot" > "$pq_pre_stage/transaction.manifest" ||
     ! prereq_rocky_archives_ok "$pq_pre_stage" "$pq_pre_stage/transaction.manifest" private "$pq_pre_snapshot" > "$pq_pre_stage/rpm.receipt"; then
    prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"
  fi
  PREREQ_ROCKY_VERIFIED_MANIFEST=$PREREQ_ARCHIVES_VERIFIED_MANIFEST PREREQ_ROCKY_VERIFIED_SIZES=$PREREQ_ARCHIVES_VERIFIED_SIZES
  PREREQ_ROCKY_METADATA_RECEIPT=$(cat "$pq_pre_stage/metadata.receipt") || return "${EX_PREREQ:-3}"
  PREREQ_ROCKY_ARCHIVE_RECEIPT=$(cat "$pq_pre_stage/rpm.receipt") || return "${EX_PREREQ:-3}"
  [ -n "$PREREQ_ROCKY_VERIFIED_MANIFEST" ] || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
  PREREQ_ROCKY_PRE_STAGE=$pq_pre_stage PREREQ_ROCKY_POLICY=$pq_pre_policy
  return 0
}

# Read only the two required members from an already verified Rocky Extras RPM.
# The decompressor receives a hashed memory snapshot, never an elevated path.
prereq_extract_epel_release() {
  PREREQ_EPEL_BOOT_STAGE='' PREREQ_EPEL_BOOT_KEY_HASH='' PREREQ_EPEL_BOOT_CONFIG_HASH=''
  [ "$#" -eq 1 ] && [ "$1" = "${PREREQ_ROCKY_PRE_STAGE:-}" ] && [ -n "${PREREQ_ROCKY_VERIFIED_MANIFEST:-}" ] || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
  pq_extract_stage=$1 pq_extract_count=0 pq_extract_file='' pq_extract_hash='' pq_extract_name=epel-release
  prereq_private_path "$pq_extract_stage" "$pq_extract_stage/transaction.manifest" || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
  pq_extract_disk=$(cat "$pq_extract_stage/transaction.manifest") || pq_extract_disk=''
  [ "$pq_extract_disk" = "$PREREQ_ROCKY_VERIFIED_MANIFEST" ] || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
  while IFS='|' read -r pq_extract_row_file pq_extract_row_hash pq_extract_row_name pq_extract_row_epoch pq_extract_row_version pq_extract_row_release pq_extract_row_arch pq_extract_row_repo || [ -n "$pq_extract_row_file" ]; do
    if [ "$pq_extract_row_name" = "$pq_extract_name" ]; then
      [ "$pq_extract_row_repo" = extras ] || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
      pq_extract_file=$pq_extract_row_file pq_extract_hash=$pq_extract_row_hash pq_extract_count=$((pq_extract_count + 1))
    fi
  done <<PREREQ_EXTRACT_MANIFEST
$PREREQ_ROCKY_VERIFIED_MANIFEST
PREREQ_EXTRACT_MANIFEST
  case $pq_extract_file in "$pq_extract_stage"/packages/*.rpm) ;; *) prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}" ;; esac
  if ! command -v rpm2cpio >/dev/null 2>&1; then prereq_msg rpm_extract_unknown; return "${EX_PREREQ:-3}"; fi
  if [ "$pq_extract_count" -ne 1 ] || ! prereq_private_path "$pq_extract_stage" "$pq_extract_file"; then
    prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"
  fi
  if ! python3 -I - "$pq_extract_file" "$pq_extract_hash" "$pq_extract_stage" <<'PREREQ_EXTRACT_PY'
import hashlib
import os
from pathlib import Path
import selectors
import stat
import subprocess
import sys
import time

archive, expected, directory = sys.argv[1:]
process = None
descriptor = None
try:
    with open(archive, "rb") as source:
        data = source.read(134217729)
    if len(data) > 134217728 or hashlib.sha256(data).hexdigest() != expected:
        raise ValueError("archive changed")
    descriptor = os.memfd_create("verified-epel-release", 0)
    view = memoryview(data)
    while view:
        count = os.write(descriptor, view)
        if count <= 0:
            raise ValueError("snapshot failed")
        view = view[count:]
    os.lseek(descriptor, 0, os.SEEK_SET)
    process = subprocess.Popen(["rpm2cpio", "/proc/self/fd/" + str(descriptor)], stdin=subprocess.DEVNULL,
                               stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, pass_fds=(descriptor,),
                               env={"PATH": os.environ.get("PATH", ""), "LC_ALL": "C"})
    selector = selectors.DefaultSelector()
    selector.register(process.stdout, selectors.EVENT_READ)
    chunks, size, deadline = [], 0, time.monotonic() + 45
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise ValueError("decompression timed out")
        if not selector.select(remaining):
            raise ValueError("decompression timed out")
        chunk = os.read(process.stdout.fileno(), 65536)
        if not chunk:
            break
        size += len(chunk)
        if size > 67108864:
            raise ValueError("decompressed archive too large")
        chunks.append(chunk)
    if process.wait(timeout=max(0.1, deadline - time.monotonic())) != 0:
        raise ValueError("decompressor failed")
    data = b"".join(chunks)
    wanted = {"etc/pki/rpm-gpg/RPM-GPG-KEY-EPEL-10": "certificate.key", "etc/yum.repos.d/epel.repo": "epel.repo"}
    found, offset, trailer = {}, 0, False
    while offset < len(data):
        header = data[offset:offset + 110]
        if len(header) != 110 or header[:6] not in (b"070701", b"070702"):
            raise ValueError("invalid cpio header")
        fields = [int(header[6 + n * 8:14 + n * 8], 16) for n in range(13)]
        mode, links, length, namesize, check = fields[1], fields[4], fields[6], fields[11], fields[12]
        offset += 110
        if not 1 <= namesize <= 4096 or offset + namesize > len(data):
            raise ValueError("invalid cpio name")
        raw_name = data[offset:offset + namesize]
        if raw_name[-1:] != b"\0" or b"\0" in raw_name[:-1]:
            raise ValueError("invalid cpio name")
        name = raw_name[:-1].decode("utf-8")
        offset = (offset + namesize + 3) & ~3
        if offset + length > len(data):
            raise ValueError("truncated cpio member")
        content = data[offset:offset + length]
        offset = (offset + length + 3) & ~3
        if header[:6] == b"070702" and sum(content) & 0xffffffff != check:
            raise ValueError("invalid cpio checksum")
        if name == "TRAILER!!!":
            trailer = True
            break
        name = name.removeprefix("./")
        if name in wanted:
            if name in found or not stat.S_ISREG(mode) or links != 1 or not 0 < length <= 1048576:
                raise ValueError("invalid requested member")
            found[name] = content
    if not trailer or set(found) != set(wanted) or any(data[offset:]):
        raise ValueError("incomplete cpio archive")
    stage = Path(directory)
    for name in ("epel-config", "epel-pinned", "epel-gpg"):
        (stage / name).mkdir(mode=0o700)
    (stage / "epel-config" / "epel.repo").write_bytes(found["etc/yum.repos.d/epel.repo"])
    key_name = "etc/pki/rpm-gpg/RPM-GPG-KEY-EPEL-10"
    (stage / "epel-pinned" / "certificate.key").write_bytes(found[key_name])
except (OSError, ValueError, UnicodeError, IndexError, subprocess.SubprocessError):
    raise SystemExit(1)
finally:
    if process is not None and process.poll() is None:
        process.kill()
        process.wait()
    if descriptor is not None:
        os.close(descriptor)
PREREQ_EXTRACT_PY
  then
    prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"
  fi
  if ! prereq_epel_key_ok "$pq_extract_stage/epel-gpg" "$pq_extract_stage/epel-pinned/certificate.key"; then
    prereq_msg compose_verify_failed; return "${EX_PREREQ:-3}"
  fi
  pq_extract_key_hash=$(sha256sum -- "$pq_extract_stage/epel-pinned/certificate.key") || return "${EX_PREREQ:-3}"
  pq_extract_config_hash=$(sha256sum -- "$pq_extract_stage/epel-config/epel.repo") || return "${EX_PREREQ:-3}"
  PREREQ_EPEL_BOOT_STAGE=$pq_extract_stage PREREQ_EPEL_BOOT_KEY_HASH=${pq_extract_key_hash%% *} PREREQ_EPEL_BOOT_CONFIG_HASH=${pq_extract_config_hash%% *}
  return 0
}

# Verify a mixed closure against the Rocky roots or the pinned EPEL root,
# assigning EPEL only after a candidate fails to match Rocky metadata.
prereq_mixed_preverify() {
  [ "$#" -ge 3 ] || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
  pq_mixed_stage=$1 pq_mixed_release=$2; shift 2
  PREREQ_ROCKY_VERIFIED_MANIFEST='' PREREQ_ROCKY_VERIFIED_SIZES='' PREREQ_ROCKY_PRE_STAGE='' PREREQ_ROCKY_POLICY='' PREREQ_ROCKY_METADATA_RECEIPT='' PREREQ_ROCKY_ARCHIVE_RECEIPT='' PREREQ_MIXED=0 PREREQ_BOOT_FILES='' PREREQ_BOOT_ONLY_OK=0 PREREQ_EPEL_PRIVATE_STAGE='' pq_epelcandidate_number=0
  if ! prereq_private_path "$pq_mixed_stage" "$pq_mixed_stage" ||
     ! mkdir -m 700 "$pq_mixed_stage/packages" "$pq_mixed_stage/empty" "$pq_mixed_stage/gpg" "$pq_mixed_stage/pinned" ||
     ! prereq_rocky_roots "$pq_mixed_stage" "$@"; then
    prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"
  fi
  if [ -n "${PREREQ_EPEL_BOOT_STAGE:-}" ]; then
    pq_mixed_boot=$PREREQ_EPEL_BOOT_STAGE
    pq_mixed_boot_key=$pq_mixed_boot/epel-pinned/certificate.key pq_mixed_boot_repo=$pq_mixed_boot/epel-config/epel.repo
    if ! prereq_private_path "$pq_mixed_boot" "$pq_mixed_boot_key" || ! prereq_private_path "$pq_mixed_boot" "$pq_mixed_boot_repo" ||
       ! mkdir -m 700 "$pq_mixed_stage/epel-config" || ! cp -- "$pq_mixed_boot_key" "$pq_mixed_stage/pinned/certificate.key" ||
       ! cp -- "$pq_mixed_boot_repo" "$pq_mixed_stage/epel-config/epel.repo"; then
      prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"
    fi
    pq_mixed_key_hash=$(sha256sum -- "$pq_mixed_stage/pinned/certificate.key") || pq_mixed_key_hash=''
    pq_mixed_repo_hash=$(sha256sum -- "$pq_mixed_stage/epel-config/epel.repo") || pq_mixed_repo_hash=''
    if [ "${pq_mixed_key_hash%% *}" != "${PREREQ_EPEL_BOOT_KEY_HASH:-}" ] || [ "${pq_mixed_repo_hash%% *}" != "${PREREQ_EPEL_BOOT_CONFIG_HASH:-}" ]; then
      prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"
    fi
    PREREQ_EPEL_PRIVATE_STAGE=$pq_mixed_stage
  elif ! cp -- "$(prereq_system_root)/etc/pki/rpm-gpg/RPM-GPG-KEY-EPEL-10" "$pq_mixed_stage/pinned/certificate.key"; then
    prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"
  fi
  if ! prereq_epel_key_ok "$pq_mixed_stage/gpg" "$pq_mixed_stage/pinned/certificate.key"; then prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; fi
  if [ -n "$PREREQ_EPEL_PRIVATE_STAGE" ]; then
    HOME="$pq_mixed_stage" TMPDIR="$pq_mixed_stage" XDG_CACHE_HOME="$pq_mixed_stage" prereq_epel_repo_config_ok "$pq_mixed_release" "$pq_mixed_stage" || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
  else
    HOME="$pq_mixed_stage" TMPDIR="$pq_mixed_stage" XDG_CACHE_HOME="$pq_mixed_stage" prereq_epel_repo_config_ok "$pq_mixed_release" || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
  fi
  pq_mixed_policy=''
  for pq_mixed_repo in baseos appstream crb; do
    if ! HOME="$pq_mixed_stage" TMPDIR="$pq_mixed_stage" XDG_CACHE_HOME="$pq_mixed_stage" prereq_rocky_repo_ok "$pq_mixed_release" "$pq_mixed_repo"; then
      prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"
    fi
    pq_mixed_policy="${pq_mixed_policy}${pq_mixed_policy:+,}$pq_mixed_repo=$PREREQ_ROCKY_REPO_GPGCHECK"
  done
  prereq_metadata_policy_ready "$pq_mixed_policy" || return "${EX_PREREQ:-3}"
  if [ "$PREREQ_EPEL_REPO_GPGCHECK" = 1 ] && { ! command -v gpg >/dev/null 2>&1 || ! command -v gpgv >/dev/null 2>&1; }; then
    prereq_msg metadata_verifier_missing; return "${EX_PREREQ:-3}"
  fi
  if ! prereq_rocky_download "$pq_mixed_stage" "$pq_mixed_release" mixed ||
     ! prereq_rocky_candidates "$pq_mixed_stage" private > "$pq_mixed_stage/candidates.manifest"; then
    prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"
  fi
  pq_mixed_classified=$(prereq_rocky_metadata_ok "$pq_mixed_release" "$pq_mixed_stage/candidates.manifest" "$pq_mixed_policy" classify_private 2> "$pq_mixed_stage/metadata.receipt") || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
  pq_mixed_compose=0 pq_mixed_compose_name=podman-compose pq_mixed_release_name=epel-release
  pq_mixed_snapshot=''
  while IFS='|' read -r pq_mixed_file pq_mixed_hash pq_mixed_name pq_mixed_epoch pq_mixed_version pq_mixed_rel pq_mixed_arch pq_mixed_source || [ -n "$pq_mixed_file" ]; do
    if [ "$pq_mixed_source" = unmatched ]; then pq_mixed_source=epel; fi
    if [ "$pq_mixed_name" = "$pq_mixed_compose_name" ]; then
      [ "$pq_mixed_source" = epel ] && prereq_compose_version_ok "$pq_mixed_version" || { prereq_msg compose_verify_failed; return "${EX_PREREQ:-3}"; }
      pq_mixed_compose=$((pq_mixed_compose + 1))
    fi
    if [ "$pq_mixed_name" = "$pq_mixed_release_name" ] && [ "$pq_mixed_source" != extras ]; then prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; fi
    pq_mixed_snapshot="${pq_mixed_snapshot}${pq_mixed_snapshot:+
}$pq_mixed_file|$pq_mixed_hash|$pq_mixed_name|$pq_mixed_epoch|$pq_mixed_version|$pq_mixed_rel|$pq_mixed_arch|$pq_mixed_source"
  done <<PREREQ_MIXED_CLASSIFIED
$pq_mixed_classified
PREREQ_MIXED_CLASSIFIED
  if [ "$pq_mixed_compose" -ne 1 ] || ! printf '%s\n' "$pq_mixed_snapshot" > "$pq_mixed_stage/transaction.manifest" ||
     ! prereq_rocky_archives_ok "$pq_mixed_stage" "$pq_mixed_stage/transaction.manifest" private_mixed "$pq_mixed_snapshot" > "$pq_mixed_stage/rpm.receipt"; then
    prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"
  fi
  PREREQ_ROCKY_VERIFIED_MANIFEST=$PREREQ_ARCHIVES_VERIFIED_MANIFEST PREREQ_ROCKY_VERIFIED_SIZES=$PREREQ_ARCHIVES_VERIFIED_SIZES
  PREREQ_ROCKY_METADATA_RECEIPT=$(cat "$pq_mixed_stage/metadata.receipt") || return "${EX_PREREQ:-3}"
  PREREQ_ROCKY_ARCHIVE_RECEIPT=$(cat "$pq_mixed_stage/rpm.receipt") || return "${EX_PREREQ:-3}"
  [ -n "$PREREQ_ROCKY_VERIFIED_MANIFEST" ] || return "${EX_PREREQ:-3}"
  PREREQ_ROCKY_PRE_STAGE=$pq_mixed_stage PREREQ_ROCKY_POLICY=$pq_mixed_policy PREREQ_MIXED=1
  PREREQ_RECHECK_BOOT_STAGE='' PREREQ_RECHECK_BOOT_MANIFEST='' PREREQ_RECHECK_BOOT_SIZES='' PREREQ_RECHECK_BOOT_POLICY=''
  PREREQ_RECHECK_MIXED_STAGE=$pq_mixed_stage PREREQ_RECHECK_MIXED_MANIFEST=$PREREQ_ROCKY_VERIFIED_MANIFEST
  PREREQ_RECHECK_MIXED_SIZES=$PREREQ_ROCKY_VERIFIED_SIZES PREREQ_RECHECK_MIXED_POLICY=$pq_mixed_policy
  return 0
}

# Merge two already verified closures without changing their recorded bytes or
# duplicating an identical dependency. The bootstrap subset remains explicit.
prereq_combine_bootstrap() {
  [ "$#" -eq 3 ] || [ "$#" -eq 4 ] || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
  pq_combine_mode=${4:-install}
  case $pq_combine_mode in install|verify) ;; *) prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}" ;; esac
  pq_combine_stage=$1 pq_combine_boot=$1/bootstrap pq_combine_mixed=$1/mixed
  if ! prereq_private_path "$pq_combine_stage" "$pq_combine_stage" ||
     ! prereq_private_path "$pq_combine_boot" "$pq_combine_boot/transaction.manifest" ||
     ! prereq_private_path "$pq_combine_mixed" "$pq_combine_mixed/transaction.manifest" ||
     ! prereq_private_path "$pq_combine_mixed" "$pq_combine_mixed/roots.receipt"; then
    prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"
  fi
  if ! pq_combine_output=$(python3 -I - "$pq_combine_stage" "$2" "$3" "$pq_combine_mode" <<'PREREQ_COMBINE_PY'
import hashlib
import os
from pathlib import Path
import stat
import sys

stage = Path(sys.argv[1])
snapshots = (sys.argv[2], sys.argv[3])
selection = sys.argv[4]
try:
    output = stage / "packages"
    output.mkdir(mode=0o700)
    records, boot_paths, identities, names, sizes = [], [], {}, {}, {}
    found_release = found_compose = 0
    for group, snapshot in zip(("bootstrap", "mixed"), snapshots):
        source = stage / group
        if (source / "transaction.manifest").read_text(encoding="ascii").rstrip("\n") != snapshot:
            raise ValueError("manifest changed")
        for line in snapshot.splitlines():
            row = line.split("|")
            if len(row) != 8:
                raise ValueError("invalid manifest")
            filename, digest, name, epoch, version, release, arch, repo = row
            path = Path(filename)
            if path.parent != source / "packages" or not path.name.endswith(".rpm") or not all(c in "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_.+~^-" for c in path.name):
                raise ValueError("invalid archive path")
            if len(digest) != 64 or not all(c in "0123456789abcdef" for c in digest) or repo not in ("baseos", "appstream", "crb", "extras", "epel"):
                raise ValueError("invalid identity")
            if group == "bootstrap" and repo == "epel":
                raise ValueError("untrusted bootstrap origin")
            if selection == "verify" and repo == "epel":
                raise ValueError("provider already installed")
            if name == "epel-release":
                if group != "bootstrap" or repo != "extras":
                    raise ValueError("invalid release origin")
                found_release += 1
            if name == "podman-compose":
                if group != "mixed" or repo != "epel":
                    raise ValueError("invalid provider origin")
                found_compose += 1
            descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
            with os.fdopen(descriptor, "rb") as stream:
                if not stat.S_ISREG(os.fstat(stream.fileno()).st_mode):
                    raise ValueError("not a regular archive")
                data = stream.read(134217729)
            if len(data) > 134217728 or hashlib.sha256(data).hexdigest() != digest:
                raise ValueError("archive changed")
            identity = tuple(row[2:7])
            if identity in identities:
                if identities[identity] != (digest, repo):
                    raise ValueError("conflicting dependency")
                continue
            if path.name in names:
                raise ValueError("conflicting archive filename")
            identities[identity] = (digest, repo)
            names[path.name] = identity
            destination = output / path.name
            with destination.open("xb") as stream:
                stream.write(data)
            records.append([str(destination), *row[1:]])
            sizes[str(destination)] = len(data)
            if group == "bootstrap":
                boot_paths.append(str(destination))
    if found_release != 1 or found_compose != (1 if selection == "install" else 0) or not boot_paths:
        raise ValueError("incomplete closure")
    records.sort(key=lambda row: row[0])
    (stage / "transaction.manifest").write_text("\n".join("|".join(row) for row in records) + "\n", encoding="ascii")
    (stage / "bootstrap.manifest").write_text("\n".join(sorted(boot_paths)) + "\n", encoding="ascii")
    (stage / "roots.receipt").write_bytes((stage / "mixed" / "roots.receipt").read_bytes())
    print("\n".join("|".join(row) for row in records))
    print("__PREREQ_COMBINED_SIZES__")
    print("\n".join(path + "|" + str(sizes[path]) for path in sorted(sizes)))
    print("__PREREQ_COMBINED_BOOT__")
    print("\n".join(sorted(boot_paths)))
except (OSError, ValueError, UnicodeError):
    raise SystemExit(1)
PREREQ_COMBINE_PY
  ); then
    prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"
  fi
  pq_combine_sizes_separator='
__PREREQ_COMBINED_SIZES__
'
  pq_combine_boot_separator='
__PREREQ_COMBINED_BOOT__
'
  case $pq_combine_output in *"$pq_combine_sizes_separator"*"$pq_combine_boot_separator"*) ;; *) return "${EX_PREREQ:-3}" ;; esac
  PREREQ_ROCKY_VERIFIED_MANIFEST=${pq_combine_output%%"$pq_combine_sizes_separator"*}
  pq_combine_tail=${pq_combine_output#*"$pq_combine_sizes_separator"}
  PREREQ_ROCKY_VERIFIED_SIZES=${pq_combine_tail%%"$pq_combine_boot_separator"*}
  PREREQ_BOOT_FILES=${pq_combine_tail#*"$pq_combine_boot_separator"}
  [ -n "$PREREQ_ROCKY_VERIFIED_MANIFEST" ] && [ -n "$PREREQ_ROCKY_VERIFIED_SIZES" ] && [ -n "$PREREQ_BOOT_FILES" ] || return "${EX_PREREQ:-3}"
  PREREQ_ROCKY_PRE_STAGE=$pq_combine_stage PREREQ_MIXED=1 PREREQ_BOOT_ONLY_OK=0
  [ "$pq_combine_mode" != verify ] || PREREQ_BOOT_ONLY_OK=1
  return 0
}

# Render the real verified closure and every privileged command before the
# question. The manifest retained in memory is the post-copy comparison.
prereq_rocky_verified_plan() {
  [ "$#" -ge 4 ] || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
  pq_plan_private=$1 pq_plan_root=$2 pq_plan_release=$3; shift 3
  prereq_release_ok "$pq_plan_release" || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
  if [ "$pq_plan_private" != "${PREREQ_ROCKY_PRE_STAGE:-}" ] || [ -z "${PREREQ_ROCKY_VERIFIED_MANIFEST:-}" ] || [ -z "${PREREQ_ROCKY_VERIFIED_SIZES:-}" ] ||
     ! prereq_private_path "$pq_plan_private" "$pq_plan_private/transaction.manifest"; then
    prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"
  fi
  pq_plan_disk_manifest=$(cat "$pq_plan_private/transaction.manifest") || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
  if [ "$pq_plan_disk_manifest" != "$PREREQ_ROCKY_VERIFIED_MANIFEST" ]; then prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; fi
  pq_plan_packages='' pq_plan_copy_commands='' pq_plan_root_manifest='' pq_plan_install_args='' pq_plan_sizes='' pq_plan_boot_args='' pq_plan_boot_root='' pq_plan_epel=0 pq_plan_boot_seen='
'
  while IFS='|' read -r pq_plan_file pq_plan_hash pq_plan_name pq_plan_epoch pq_plan_version pq_plan_release_part pq_plan_arch pq_plan_repo || [ -n "$pq_plan_file" ]; do
    case $pq_plan_file in "$pq_plan_private"/packages/*.rpm) ;; *) prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}" ;; esac
    if ! prereq_private_path "$pq_plan_private" "$pq_plan_file"; then prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; fi
    pq_plan_current_hash=$(sha256sum -- "$pq_plan_file") || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
    pq_plan_current_hash=${pq_plan_current_hash%% *}
    [ "$pq_plan_current_hash" = "$pq_plan_hash" ] || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
    pq_plan_size=$(printf '%s\n' "$PREREQ_ROCKY_VERIFIED_SIZES" | LC_ALL=C awk -F'|' -v path="$pq_plan_file" '$1 == path { print $2; found++ } END { if (found != 1) exit 1 }') || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
    case $pq_plan_size in ''|*[!0123456789]*) prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}" ;; esac
    pq_plan_current_size=$(wc -c < "$pq_plan_file") || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
    [ "$pq_plan_current_size" = "$pq_plan_size" ] || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
    pq_plan_dest=$pq_plan_root/packages/${pq_plan_file##*/}
    [ "$pq_plan_repo" != epel ] || pq_plan_epel=1
    pq_plan_packages="${pq_plan_packages}${pq_plan_packages:+
}  package: $pq_plan_name|$pq_plan_epoch|$pq_plan_version|$pq_plan_release_part|$pq_plan_arch repository=$pq_plan_repo bytes=$pq_plan_size sha256=$pq_plan_hash verified=yes"
    pq_plan_copy_commands="${pq_plan_copy_commands}${pq_plan_copy_commands:+
}  copy: $pq_plan_file -> $pq_plan_dest
  sudo tee -- $pq_plan_dest"
    pq_plan_sizes="${pq_plan_sizes}${pq_plan_sizes:+
}$pq_plan_dest|$pq_plan_size"
    pq_plan_root_manifest="${pq_plan_root_manifest}${pq_plan_root_manifest:+
}$pq_plan_dest|$pq_plan_hash|$pq_plan_name|$pq_plan_epoch|$pq_plan_version|$pq_plan_release_part|$pq_plan_arch|$pq_plan_repo"
    case "
${PREREQ_BOOT_FILES:-}
" in
      *"
$pq_plan_file
"*)
        pq_plan_boot_args="${pq_plan_boot_args}${pq_plan_boot_args:+ }$pq_plan_dest"
        pq_plan_boot_root="${pq_plan_boot_root}${pq_plan_boot_root:+
}$pq_plan_dest"
        pq_plan_boot_seen="$pq_plan_boot_seen$pq_plan_file
" ;;
      *) pq_plan_install_args="${pq_plan_install_args}${pq_plan_install_args:+ }$pq_plan_dest" ;;
    esac
  done <<PREREQ_VERIFIED_PLAN
$PREREQ_ROCKY_VERIFIED_MANIFEST
PREREQ_VERIFIED_PLAN
  [ -n "$pq_plan_root_manifest" ] || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
  if [ -n "${PREREQ_BOOT_FILES:-}" ]; then
    while IFS= read -r pq_plan_boot_file || [ -n "$pq_plan_boot_file" ]; do
      case $pq_plan_boot_seen in *"
$pq_plan_boot_file
"*) ;; *) prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}" ;; esac
    done <<PREREQ_PLAN_BOOT_FILES
$PREREQ_BOOT_FILES
PREREQ_PLAN_BOOT_FILES
    [ -n "$pq_plan_boot_args" ] && { [ -n "$pq_plan_install_args" ] || [ "${PREREQ_BOOT_ONLY_OK:-0}" = 1 ]; } || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
  fi
  pq_plan_rpm=$(LC_ALL=C rpm --version 2>/dev/null) || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
  pq_plan_rpmkeys=$(LC_ALL=C rpmkeys --version 2>/dev/null) || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
  pq_plan_dnf=$(HOME="$pq_plan_private" TMPDIR="$pq_plan_private" XDG_CACHE_HOME="$pq_plan_private" LC_ALL=C dnf --version 2>/dev/null) || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
  pq_plan_newline='
'
  pq_plan_rpm=${pq_plan_rpm%%"$pq_plan_newline"*} pq_plan_rpmkeys=${pq_plan_rpmkeys%%"$pq_plan_newline"*} pq_plan_dnf=${pq_plan_dnf%%"$pq_plan_newline"*}
  for pq_plan_version in "$pq_plan_rpm" "$pq_plan_rpmkeys" "$pq_plan_dnf"; do
    case $pq_plan_version in ''|*[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._+\ -]*) prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}" ;; esac
  done
  pq_plan_manifest_file=$pq_plan_private/root-transaction.manifest
  [ ! -e "$pq_plan_manifest_file" ] && [ ! -L "$pq_plan_manifest_file" ] || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
  printf '%s\n' "$pq_plan_root_manifest" > "$pq_plan_manifest_file" || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
  PREREQ_ROCKY_ROOT_MANIFEST=$pq_plan_root_manifest PREREQ_ROCKY_PLAN_SIZES=$pq_plan_sizes PREREQ_BOOT_ROOT_FILES=$pq_plan_boot_root
  say 'Prerequisite plan: install the following verified packages and dependencies.' || return "${EX_PREREQ:-3}"
  say 'Privileged commands use /usr/bin/sudo and PATH=/usr/bin:/usr/sbin; package and account changes use umask 022.' || return "${EX_PREREQ:-3}"
  say "  os=Rocky Linux $pq_plan_release arch=x86_64" || return "${EX_PREREQ:-3}"
  say "  verifier_rpm=$pq_plan_rpm" || return "${EX_PREREQ:-3}"
  say "  verifier_rpmkeys=$pq_plan_rpmkeys" || return "${EX_PREREQ:-3}"
  say "  verifier_dnf=$pq_plan_dnf" || return "${EX_PREREQ:-3}"
  if [ "$pq_plan_epel" = 1 ] || [ -n "${PREREQ_EPEL_BOOT_STAGE:-}" ]; then
    say '  EPEL is a community repository; its packages are trusted using the pinned signing key.' || return "${EX_PREREQ:-3}"
    say '  CRB is used for verified downloads here; no manual crb enable is needed.' || return "${EX_PREREQ:-3}"
    if [ -n "${PREREQ_EPEL_BOOT_STAGE:-}" ]; then
      say '  Installing epel-release persistently configures the EPEL repository and places its signing key on the system.' || return "${EX_PREREQ:-3}"
    else
      say '  The existing EPEL repository configuration and signing key remain persistent on the system.' || return "${EX_PREREQ:-3}"
    fi
    say '  epel_fingerprint=7D8D15CBFC4E62688591FB2633D98517E37ED158' || return "${EX_PREREQ:-3}"
  fi
  while IFS= read -r pq_plan_line; do [ -z "$pq_plan_line" ] || { say "  $pq_plan_line" || return "${EX_PREREQ:-3}"; }; done <<PREREQ_METADATA_RECEIPT
${PREREQ_ROCKY_METADATA_RECEIPT:-}
PREREQ_METADATA_RECEIPT
  while IFS= read -r pq_plan_line; do [ -z "$pq_plan_line" ] || { say "  $pq_plan_line" || return "${EX_PREREQ:-3}"; }; done <<PREREQ_ARCHIVE_RECEIPT
${PREREQ_ROCKY_ARCHIVE_RECEIPT:-}
PREREQ_ARCHIVE_RECEIPT
  while IFS= read -r pq_plan_line; do say "  $pq_plan_line" || return "${EX_PREREQ:-3}"; done < "$pq_plan_private/roots.receipt" || return "${EX_PREREQ:-3}"
  while IFS= read -r pq_plan_line; do say "$pq_plan_line" || return "${EX_PREREQ:-3}"; done <<PREREQ_PACKAGE_LINES
$pq_plan_packages
PREREQ_PACKAGE_LINES
  say "  sudo mkdir -m 755 -- $pq_plan_root" || return "${EX_PREREQ:-3}"
  say "  sudo mkdir -m 755 -- $pq_plan_root/packages $pq_plan_root/rocky-ring $pq_plan_root/empty" || return "${EX_PREREQ:-3}"
  pq_plan_root_number=0
  for pq_plan_key do
    pq_plan_root_number=$((pq_plan_root_number + 1))
    say "  sudo install -m 0644 -- $pq_plan_key $pq_plan_root/rocky-ring/root-$pq_plan_root_number.key" || return "${EX_PREREQ:-3}"
  done
  while IFS= read -r pq_plan_line; do say "$pq_plan_line" || return "${EX_PREREQ:-3}"; done <<PREREQ_COPY_LINES
$pq_plan_copy_commands
PREREQ_COPY_LINES
  say "  sudo tee -- $pq_plan_root/transaction.manifest" || return "${EX_PREREQ:-3}"
  if [ -n "$pq_plan_boot_args" ]; then say "  sudo dnf --releasever=$pq_plan_release --disablerepo=* install -y $pq_plan_boot_args" || return "${EX_PREREQ:-3}"; fi
  if [ -n "$pq_plan_install_args" ]; then say "  sudo dnf --releasever=$pq_plan_release --disablerepo=* install -y $pq_plan_install_args" || return "${EX_PREREQ:-3}"; fi
  say "  sudo rm -rf -- $pq_plan_root" || return "${EX_PREREQ:-3}"
  return 0
}

# Re-run constrained metadata and signature verification on the retained
# pre-copy tuple. Root-owned copies are independently matched to the same hash.
prereq_recheck_closure() {
  [ "$#" -eq 4 ] || return 1
  pq_recheck_stage=$1 pq_recheck_manifest=$2 pq_recheck_sizes=$3 pq_recheck_policy=$4
  [ -n "$pq_recheck_manifest" ] && [ -n "$pq_recheck_sizes" ] && [ -n "$pq_recheck_policy" ] || return 1
  prereq_rocky_metadata_ok "$pq_locked_release" "$pq_recheck_stage/transaction.manifest" "$pq_recheck_policy" verify_mixed_private "$pq_recheck_manifest" >/dev/null || return 1
  prereq_rocky_archives_ok "$pq_recheck_stage" "$pq_recheck_stage/transaction.manifest" private_mixed "$pq_recheck_manifest" >/dev/null || return 1
  [ "$PREREQ_ARCHIVES_VERIFIED_MANIFEST" = "$pq_recheck_manifest" ] && [ "$PREREQ_ARCHIVES_VERIFIED_SIZES" = "$pq_recheck_sizes" ]
}

# Install a mixed closure only if its locked root-owned bytes still match the
# complete in-memory manifest that was authenticated before approval.
prereq_mixed_locked_install() {
  [ "$#" -ge 4 ] || return "${EX_PREREQ:-3}"
  pq_locked_stage=$1 pq_locked_private=$2 pq_locked_release=$3; shift 3
  if [ "${PREREQ_MIXED:-0}" != 1 ] || [ -z "${PREREQ_ROCKY_ROOT_MANIFEST:-}" ] ||
     ! prereq_rocky_seal_ok "$pq_locked_stage" "$pq_locked_private" "$@" ||
     ! prereq_root_locked_path "$pq_locked_stage/transaction.manifest"; then
    prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"
  fi
  pq_locked_manifest=$(cat "$pq_locked_stage/transaction.manifest") || return "${EX_PREREQ:-3}"
  [ "$pq_locked_manifest" = "$PREREQ_ROCKY_ROOT_MANIFEST" ] || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
  pq_locked_candidates=$(prereq_rocky_candidates "$pq_locked_stage") || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
  pq_locked_expected=$(printf '%s\n' "$PREREQ_ROCKY_ROOT_MANIFEST" | LC_ALL=C awk -F'|' 'NF == 8 { print $1 "|" $2 "|" $3 "|" $4 "|" $5 "|" $6 "|" $7; next } { bad = 1 } END { if (bad || NR == 0) exit 1 }') || return "${EX_PREREQ:-3}"
  [ "$pq_locked_candidates" = "$pq_locked_expected" ] || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
  if [ -n "${PREREQ_RECHECK_BOOT_STAGE:-}" ]; then
    prereq_recheck_closure "$PREREQ_RECHECK_BOOT_STAGE" "$PREREQ_RECHECK_BOOT_MANIFEST" "$PREREQ_RECHECK_BOOT_SIZES" "$PREREQ_RECHECK_BOOT_POLICY" || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
  fi
  if [ -n "${PREREQ_RECHECK_MIXED_STAGE:-}" ]; then
    prereq_recheck_closure "$PREREQ_RECHECK_MIXED_STAGE" "$PREREQ_RECHECK_MIXED_MANIFEST" "$PREREQ_RECHECK_MIXED_SIZES" "$PREREQ_RECHECK_MIXED_POLICY" || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
  elif [ -z "${PREREQ_RECHECK_BOOT_STAGE:-}" ]; then
    prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"
  fi
  if [ -n "${PREREQ_BOOT_ROOT_FILES:-}" ]; then
    set --
    while IFS= read -r pq_locked_boot_file || [ -n "$pq_locked_boot_file" ]; do
      [ -n "$pq_locked_boot_file" ] || return "${EX_PREREQ:-3}"
      set -- "$@" "$pq_locked_boot_file"
    done <<PREREQ_LOCKED_BOOT
$PREREQ_BOOT_ROOT_FILES
PREREQ_LOCKED_BOOT
    [ "$#" -gt 0 ] || return "${EX_PREREQ:-3}"
    if ! ( umask 022; /usr/bin/sudo dnf "--releasever=$pq_locked_release" '--disablerepo=*' install -y "$@" ); then
      prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"
    fi
    pq_locked_after=$(prereq_rocky_candidates "$pq_locked_stage") || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
    pq_locked_manifest_after=$(cat "$pq_locked_stage/transaction.manifest") || pq_locked_manifest_after=''
    if [ "$pq_locked_after" != "$pq_locked_expected" ] || [ "$pq_locked_manifest_after" != "$PREREQ_ROCKY_ROOT_MANIFEST" ] ||
       ! prereq_root_locked_path "$pq_locked_stage/transaction.manifest"; then
      prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"
    fi
  fi
  set --
  while IFS='|' read -r pq_locked_file pq_locked_rest || [ -n "$pq_locked_file" ]; do
    [ -n "$pq_locked_file" ] || return "${EX_PREREQ:-3}"
    case "
${PREREQ_BOOT_ROOT_FILES:-}
" in *"
$pq_locked_file
"*) ;; *) set -- "$@" "$pq_locked_file" ;; esac
  done <<PREREQ_LOCKED_MANIFEST
$PREREQ_ROCKY_ROOT_MANIFEST
PREREQ_LOCKED_MANIFEST
  if [ "$#" -eq 0 ] && [ -n "${PREREQ_BOOT_ROOT_FILES:-}" ] && [ "${PREREQ_BOOT_ONLY_OK:-0}" = 1 ]; then return 0; fi
  [ "$#" -gt 0 ] || return "${EX_PREREQ:-3}"
  if ! ( umask 022; /usr/bin/sudo dnf "--releasever=$pq_locked_release" '--disablerepo=*' install -y "$@" ); then
    prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"
  fi
  return 0
}

# Copy the preverified bytes to a root-owned stage only after approval, then
# compare them to the retained manifest before executing the planned install.
prereq_rocky_apply_verified() {
  [ "$#" -ge 4 ] || { prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"; }
  pq_apply_private=$1 pq_apply_root=$2 pq_apply_release=$3; shift 3
  if [ "$pq_apply_private" != "${PREREQ_ROCKY_PRE_STAGE:-}" ] || [ -z "${PREREQ_ROCKY_ROOT_MANIFEST:-}" ] ||
     ! mkdir -m 700 "$pq_apply_private/root-prepare" ||
     ! prereq_prepare_root_stage "$pq_apply_root" "$pq_apply_private/root-prepare" "$@"; then
    prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"
  fi
  pq_apply_status=0
  while IFS='|' read -r pq_apply_file pq_apply_hash pq_apply_name pq_apply_epoch pq_apply_version pq_apply_release_part pq_apply_arch pq_apply_repo || [ -n "$pq_apply_file" ]; do
    pq_apply_source=$pq_apply_private/packages/${pq_apply_file##*/}
    pq_apply_size=$(printf '%s\n' "$PREREQ_ROCKY_PLAN_SIZES" | LC_ALL=C awk -F'|' -v path="$pq_apply_file" '$1 == path { print $2; found++ } END { if (found != 1) exit 1 }') || { pq_apply_status=${EX_PREREQ:-3}; break; }
    case $pq_apply_size in ''|*[!0123456789]*) pq_apply_status=${EX_PREREQ:-3}; break ;; esac
    if ! prereq_private_path "$pq_apply_private" "$pq_apply_source" ||
       ! ( umask 022; head -c "$pq_apply_size" -- "$pq_apply_source" | /usr/bin/sudo tee -- "$pq_apply_file" >/dev/null ) ||
       ! prereq_root_locked_path "$pq_apply_file"; then pq_apply_status=${EX_PREREQ:-3}; break; fi
    pq_apply_hash_after=$(sha256sum -- "$pq_apply_file") || { pq_apply_status=${EX_PREREQ:-3}; break; }
    pq_apply_hash_after=${pq_apply_hash_after%% *}
    if [ "$pq_apply_hash_after" != "$pq_apply_hash" ]; then pq_apply_status=${EX_PREREQ:-3}; break; fi
  done <<PREREQ_APPROVED_MANIFEST
$PREREQ_ROCKY_ROOT_MANIFEST
PREREQ_APPROVED_MANIFEST
  if [ "$pq_apply_status" -eq 0 ]; then
    pq_apply_private_manifest=$(cat "$pq_apply_private/root-transaction.manifest") || pq_apply_private_manifest=''
    if [ "$pq_apply_private_manifest" != "$PREREQ_ROCKY_ROOT_MANIFEST" ] ||
       ! ( umask 022; printf '%s\n' "$PREREQ_ROCKY_ROOT_MANIFEST" | /usr/bin/sudo tee -- "$pq_apply_root/transaction.manifest" >/dev/null ) ||
       ! prereq_root_locked_path "$pq_apply_root/transaction.manifest"; then
      pq_apply_status=${EX_PREREQ:-3}
    else
      pq_apply_manifest_after=$(cat "$pq_apply_root/transaction.manifest") || pq_apply_manifest_after=''
      [ "$pq_apply_manifest_after" = "$PREREQ_ROCKY_ROOT_MANIFEST" ] || pq_apply_status=${EX_PREREQ:-3}
    fi
  fi
  if [ "$pq_apply_status" -eq 0 ]; then
    if ! mkdir -m 700 "$pq_apply_private/install-check"; then
      pq_apply_status=${EX_PREREQ:-3}
    elif [ "${PREREQ_MIXED:-0}" = 1 ]; then
      prereq_mixed_locked_install "$pq_apply_root" "$pq_apply_private/install-check" "$pq_apply_release" "$@" || pq_apply_status=${EX_PREREQ:-3}
    else
      prereq_rocky_install "$pq_apply_root" "$pq_apply_private/install-check" "$pq_apply_release" "$PREREQ_ROCKY_POLICY" "$@" || pq_apply_status=${EX_PREREQ:-3}
    fi
  fi
  if ! prereq_root_locked_path "$pq_apply_root" || ! /usr/bin/sudo rm -rf -- "$pq_apply_root"; then
    prereq_msg rocky_install_failed; return "${EX_PREREQ:-3}"
  fi
  PREREQ_ROOT_STAGE=''
  [ "$pq_apply_status" -eq 0 ] || { prereq_msg rocky_install_failed; return "$pq_apply_status"; }
  return 0
}

# Package acquisition and installation run in a subshell so its cleanup traps
# cannot change traps owned by the sourcing installer. Standard output returns
# only the verified provider; the plan remains visible on standard error.
prereq_run_packages_inner() {
  (
  prereq_run_packages_inner_body 3>&- >&2 || exit "$?"
  printf '%s\n' "$PREREQ_COMPOSE"
  )
}

prereq_run_packages_inner_body() {
  prereq_harden_environment || return 1
  pq_entry_stage='' PREREQ_ROOT_STAGE='' pq_requalify_stage='' PREREQ_EPEL_BOOT_STAGE=''
  trap 'if [ -n "$PREREQ_ROOT_STAGE" ] && prereq_root_locked_path "$PREREQ_ROOT_STAGE"; then /usr/bin/sudo rm -rf -- "$PREREQ_ROOT_STAGE" || :; fi; if [ -n "$pq_requalify_stage" ]; then rm -rf -- "$pq_requalify_stage"; fi; if [ -n "$pq_entry_stage" ]; then rm -rf -- "$pq_entry_stage"; fi' EXIT
  trap 'exit 130' HUP INT TERM
  if ! prereq_parse_missing; then
    prereq_msg prereq_unavailable || exit "${EX_PREREQ:-3}"; exit "${EX_PREREQ:-3}"
  fi
  if ! prereq_os_ok "$(prereq_system_root)/etc/os-release"; then
    prereq_msg os_unsupported || exit "${EX_PREREQ:-3}"; exit "${EX_PREREQ:-3}"
  fi
  pq_entry_arch=$(uname -m 2>/dev/null) || pq_entry_arch=''
  if [ "$pq_entry_arch" != x86_64 ]; then
    prereq_msg arch_unsupported || exit "${EX_PREREQ:-3}"; exit "${EX_MACHINE:-2}"
  fi
  if [ "$PREREQ_OS_CONFIDENCE" = UNPROVEN ]; then prereq_info os_unproven "$pq_os_version" || exit "${EX_PREREQ:-3}"; fi
  if ! [ -x /usr/bin/sudo ]; then prereq_msg no_sudo || exit "${EX_PREREQ:-3}"; exit "${EX_MACHINE:-2}"; fi
  pq_entry_compose_mode=$PREREQ_PLAN_COMPOSE pq_entry_packages=$PREREQ_PLAN_PACKAGES
  pq_entry_uid_fix=0 pq_entry_gid_fix=0 pq_entry_linger=0 pq_entry_epel=0 pq_entry_user='' pq_entry_range=''
  case ${PREREQ_FIXES:-} in *[!abcdefghijklmnopqrstuvwxyz\ ]*) prereq_msg prereq_unavailable || exit "${EX_PREREQ:-3}"; exit "${EX_PREREQ:-3}" ;; esac
  for pq_entry_fix in ${PREREQ_FIXES:-}; do
    case $pq_entry_fix in
      subuid) pq_entry_uid_fix=1 ;; subgid) pq_entry_gid_fix=1 ;; linger) pq_entry_linger=1 ;;
      epel) pq_entry_epel=1 ;;
      *) prereq_msg prereq_unavailable || exit "${EX_PREREQ:-3}"; exit "${EX_PREREQ:-3}" ;;
    esac
  done
  if [ "$pq_entry_uid_fix$pq_entry_gid_fix$pq_entry_linger" != 000 ]; then
    pq_entry_user=$(id -un 2>/dev/null) || pq_entry_user=''
    case $pq_entry_user in ''|-*|*[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_.-]*) prereq_msg prereq_unavailable || exit "${EX_PREREQ:-3}"; exit "${EX_PREREQ:-3}" ;; esac
  fi
  if [ "$pq_entry_uid_fix$pq_entry_gid_fix" != 00 ]; then
    pq_entry_range=$(prereq_subid_range "$(prereq_system_root)/etc/subuid" "$(prereq_system_root)/etc/subgid") || pq_entry_range=''
    [ -n "$pq_entry_range" ] || { prereq_msg subids_failed || exit "${EX_PREREQ:-3}"; exit "${EX_PREREQ:-3}"; }
  fi
  pq_entry_release=$pq_os_version
  case ${HOME:-} in /*) ;; *) prereq_msg prereq_unavailable || exit "${EX_PREREQ:-3}"; exit "${EX_PREREQ:-3}" ;; esac
  if ! prereq_home_safe; then prereq_msg home_unsafe || exit "${EX_PREREQ:-3}"; exit "${EX_PREREQ:-3}"; fi
  pq_entry_stage=$(mktemp -d "$HOME/.cortex-prereq.XXXXXXXX") || { prereq_msg prereq_unavailable || exit "${EX_PREREQ:-3}"; exit "${EX_PREREQ:-3}"; }
  case $pq_entry_stage in "$HOME"/.cortex-prereq.*) ;; *) pq_entry_stage=''; prereq_msg prereq_unavailable || exit "${EX_PREREQ:-3}"; exit "${EX_PREREQ:-3}" ;; esac
  if [ "$pq_entry_compose_mode" = verify ] && [ "$pq_entry_epel" = 0 ]; then
    prereq_requalify_compose || exit "$?"
    pq_entry_provider=$PREREQ_COMPOSE
  else
    pq_entry_provider=$(prereq_system_root)/usr/bin/podman-compose
  fi
  set --
  for pq_entry_root in "$(prereq_system_root)"/etc/pki/rpm-gpg/RPM-GPG-KEY-Rocky-*; do
    [ -f "$pq_entry_root" ] || continue
    set -- "$@" "$pq_entry_root"
  done
  [ "$#" -gt 0 ] || { prereq_msg rocky_install_failed || exit "${EX_PREREQ:-3}"; exit "${EX_PREREQ:-3}"; }
  if [ "$pq_entry_epel" = 1 ]; then
    if ! mkdir -m 700 "$pq_entry_stage/bootstrap" "$pq_entry_stage/mixed"; then prereq_msg rocky_install_failed || exit "${EX_PREREQ:-3}"; exit "${EX_PREREQ:-3}"; fi
    prereq_rocky_preverify "$pq_entry_stage/bootstrap" "$pq_entry_release" extras "$@" || exit "$?"
    pq_entry_boot_manifest=$PREREQ_ROCKY_VERIFIED_MANIFEST
    pq_entry_boot_sizes=$PREREQ_ROCKY_VERIFIED_SIZES pq_entry_boot_policy=$PREREQ_ROCKY_POLICY
    pq_entry_boot_metadata=$PREREQ_ROCKY_METADATA_RECEIPT pq_entry_boot_archives=$PREREQ_ROCKY_ARCHIVE_RECEIPT
    prereq_extract_epel_release "$pq_entry_stage/bootstrap" || exit "$?"
    if [ "$pq_entry_compose_mode" = verify ]; then
      prereq_requalify_compose || exit "$?"
      pq_entry_provider=$PREREQ_COMPOSE
      if [ -n "$pq_entry_packages" ]; then
        prereq_rocky_preverify "$pq_entry_stage/mixed" "$pq_entry_release" main "$@" || exit "$?"
        pq_entry_mixed_manifest=$PREREQ_ROCKY_VERIFIED_MANIFEST
        pq_entry_mixed_sizes=$PREREQ_ROCKY_VERIFIED_SIZES pq_entry_mixed_policy=$PREREQ_ROCKY_POLICY
      else
        : > "$pq_entry_stage/mixed/transaction.manifest" || exit "${EX_PREREQ:-3}"
        cp -- "$pq_entry_stage/bootstrap/roots.receipt" "$pq_entry_stage/mixed/roots.receipt" || exit "${EX_PREREQ:-3}"
        pq_entry_mixed_manifest='' pq_entry_mixed_sizes='' pq_entry_mixed_policy='' PREREQ_ROCKY_METADATA_RECEIPT='' PREREQ_ROCKY_ARCHIVE_RECEIPT=''
      fi
    else
      prereq_mixed_preverify "$pq_entry_stage/mixed" "$pq_entry_release" "$@" || exit "$?"
      pq_entry_mixed_manifest=$PREREQ_ROCKY_VERIFIED_MANIFEST
      pq_entry_mixed_sizes=$PREREQ_ROCKY_VERIFIED_SIZES pq_entry_mixed_policy=$PREREQ_ROCKY_POLICY
    fi
    prereq_combine_bootstrap "$pq_entry_stage" "$pq_entry_boot_manifest" "$pq_entry_mixed_manifest" "$pq_entry_compose_mode" || exit "$?"
    PREREQ_RECHECK_BOOT_STAGE=$pq_entry_stage/bootstrap PREREQ_RECHECK_BOOT_MANIFEST=$pq_entry_boot_manifest
    PREREQ_RECHECK_BOOT_SIZES=$pq_entry_boot_sizes PREREQ_RECHECK_BOOT_POLICY=$pq_entry_boot_policy
    PREREQ_RECHECK_MIXED_STAGE='' PREREQ_RECHECK_MIXED_MANIFEST=$pq_entry_mixed_manifest
    PREREQ_RECHECK_MIXED_SIZES=$pq_entry_mixed_sizes PREREQ_RECHECK_MIXED_POLICY=$pq_entry_mixed_policy
    [ -z "$pq_entry_mixed_manifest" ] || PREREQ_RECHECK_MIXED_STAGE=$pq_entry_stage/mixed
    PREREQ_ROCKY_METADATA_RECEIPT="$pq_entry_boot_metadata${PREREQ_ROCKY_METADATA_RECEIPT:+
$PREREQ_ROCKY_METADATA_RECEIPT}"
    PREREQ_ROCKY_ARCHIVE_RECEIPT="$pq_entry_boot_archives${PREREQ_ROCKY_ARCHIVE_RECEIPT:+
$PREREQ_ROCKY_ARCHIVE_RECEIPT}"
  elif [ "$pq_entry_compose_mode" = verify ]; then
    prereq_rocky_preverify "$pq_entry_stage" "$pq_entry_release" main "$@" || exit "$?"
  else
    prereq_mixed_preverify "$pq_entry_stage" "$pq_entry_release" "$@" || exit "$?"
  fi
  pq_entry_uid=$(id -u 2>/dev/null) || exit "${EX_PREREQ:-3}"
  case $pq_entry_uid in ''|*[!0123456789]*) exit "${EX_PREREQ:-3}" ;; esac
  pq_entry_root_stage=$(prereq_system_root)/var/cache/cortex-prereq-$pq_entry_uid-$$
  prereq_rocky_verified_plan "$pq_entry_stage" "$pq_entry_root_stage" "$pq_entry_release" "$@" || exit "$?"
  if [ "$pq_entry_uid_fix$pq_entry_gid_fix" != 00 ]; then
    pq_entry_command='  sudo usermod'
    [ "$pq_entry_uid_fix" != 1 ] || pq_entry_command="$pq_entry_command --add-subuids $pq_entry_range"
    [ "$pq_entry_gid_fix" != 1 ] || pq_entry_command="$pq_entry_command --add-subgids $pq_entry_range"
    say "$pq_entry_command -- $pq_entry_user" || exit "${EX_PREREQ:-3}"
  fi
  if [ "$pq_entry_linger" = 1 ]; then
    say "  loginctl enable-linger $pq_entry_user" || exit "${EX_PREREQ:-3}"
    say "  sudo loginctl enable-linger -- $pq_entry_user" || exit "${EX_PREREQ:-3}"
  fi
  prereq_confirm || exit "$?"
  prereq_rocky_apply_verified "$pq_entry_stage" "$pq_entry_root_stage" "$pq_entry_release" "$@" || exit "$?"
  pq_entry_compose_archive='' pq_entry_compose_hash='' pq_entry_compose_name=podman-compose
  while IFS='|' read -r pq_post_file pq_post_hash pq_post_name pq_post_epoch pq_post_version pq_post_release pq_post_arch pq_post_repo || [ -n "$pq_post_file" ]; do
    pq_post_expected=$pq_post_name'|'$pq_post_epoch'|'$pq_post_version'|'$pq_post_release'|'$pq_post_arch
    pq_post_installed=$(LC_ALL=C rpm -q --queryformat '%{NAME}|%{EPOCHNUM}|%{VERSION}|%{RELEASE}|%{ARCH}\n' "$pq_post_name" 2>/dev/null) || pq_post_installed=''
    if ! printf '%s\n' "$pq_post_installed" | LC_ALL=C grep -Fx -- "$pq_post_expected" >/dev/null; then
      prereq_msg rocky_install_failed || exit "${EX_PREREQ:-3}"; exit "${EX_PREREQ:-3}"
    fi
    if [ "$pq_post_name" = "$pq_entry_compose_name" ]; then pq_entry_compose_archive=$pq_post_file pq_entry_compose_hash=$pq_post_hash; fi
  done <<PREREQ_POST_MANIFEST
$PREREQ_ROCKY_VERIFIED_MANIFEST
PREREQ_POST_MANIFEST
  if [ "$pq_entry_uid_fix$pq_entry_gid_fix" != 00 ]; then
    prereq_fix_subids "$pq_entry_user" "$pq_entry_range" "$pq_entry_uid_fix" "$pq_entry_gid_fix" || exit "$?"
  fi
  if [ "$pq_entry_linger" = 1 ]; then prereq_fix_linger "$pq_entry_user" || exit "$?"; fi
  if [ "$pq_entry_compose_mode" = install ]; then
    if [ -z "$pq_entry_compose_archive" ] || ! prereq_private_path "$pq_entry_stage" "$pq_entry_compose_archive"; then
      prereq_msg compose_verify_failed || exit "${EX_PREREQ:-3}"; exit "${EX_PREREQ:-3}"
    fi
    pq_entry_compose_actual=$(sha256sum -- "$pq_entry_compose_archive") || pq_entry_compose_actual=''
    if [ "${pq_entry_compose_actual%% *}" != "$pq_entry_compose_hash" ] ||
       ! prereq_rpm_payload_ok "$pq_entry_compose_archive" "$pq_entry_provider"; then
      prereq_msg compose_verify_failed || exit "${EX_PREREQ:-3}"; exit "${EX_PREREQ:-3}"
    fi
    pq_entry_compose_actual=$(sha256sum -- "$pq_entry_compose_archive") || pq_entry_compose_actual=''
    if [ "${pq_entry_compose_actual%% *}" != "$pq_entry_compose_hash" ]; then prereq_msg compose_verify_failed || exit "${EX_PREREQ:-3}"; exit "${EX_PREREQ:-3}"; fi
    pq_verified_path=$pq_entry_provider
    prereq_compose_path() {
      [ -n "${PREREQ_COMPOSE:-}" ] && [ "$PREREQ_COMPOSE" = "$pq_verified_path" ] &&
        [ -f "$PREREQ_COMPOSE" ] && [ -x "$PREREQ_COMPOSE" ] && [ ! -L "$PREREQ_COMPOSE" ] || return 1
      printf '%s\n' "$PREREQ_COMPOSE"
    }
  fi
  pq_entry_requalify='podman-compose (requalify)'
  pq_entry_look_status=0
  machine_look || pq_entry_look_status=$?
  [ "$pq_entry_look_status" -ne "${EX_PREREQ:-3}" ] || exit "${EX_PREREQ:-3}"
  if [ "$pq_entry_look_status" -ne 0 ] || [ "$PREREQ_MISSING" != "$pq_entry_requalify" ] || [ -n "$PREREQ_FIXES" ]; then
    prereq_msg prereq_unavailable || exit "${EX_PREREQ:-3}"; exit "${EX_PREREQ:-3}"
  fi
  PREREQ_COMPOSE=$pq_entry_provider
  prereq_compose_path >/dev/null || { prereq_msg prereq_unavailable || exit "${EX_PREREQ:-3}"; exit "${EX_PREREQ:-3}"; }
  return 0
}

# Step 2 consumes the read-only findings from machine_look.
prereq_run() {
  # Only verified extraction inside this step may supply a boot repository.
  unset PREREQ_EPEL_BOOT_STAGE PREREQ_EPEL_BOOT_KEY_HASH PREREQ_EPEL_BOOT_CONFIG_HASH || return 1
  prereq_secure_call prereq_run_body
}

prereq_run_body() {
  if [ -z "${PREREQ_MISSING:-}${PREREQ_FIXES:-}" ]; then
    if ! prereq_compose_path >/dev/null; then prereq_msg prereq_unavailable || return "${EX_PREREQ:-3}"; return "${EX_PREREQ:-3}"; fi
    say 'Prerequisites are already ready.' || return "${EX_PREREQ:-3}"
    return 0
  fi
  pq_run_epel=0
  for pq_run_possible_fix in ${PREREQ_FIXES:-}; do [ "$pq_run_possible_fix" != epel ] || pq_run_epel=1; done
  if prereq_parse_missing && { [ -n "$PREREQ_PLAN_PACKAGES" ] || [ "$PREREQ_PLAN_COMPOSE" = install ] || [ "$pq_run_epel" = 1 ]; }; then
    pq_run_result=$(prereq_run_packages_inner 3>&-) || return "$?"
    case $pq_run_result in /*) ;; *) prereq_msg prereq_unavailable || return "${EX_PREREQ:-3}"; return "${EX_PREREQ:-3}" ;; esac
    pq_verified_path=$pq_run_result PREREQ_COMPOSE=$pq_run_result
    prereq_compose_path() {
      [ -n "${PREREQ_COMPOSE:-}" ] && [ "$PREREQ_COMPOSE" = "$pq_verified_path" ] &&
        [ -f "$PREREQ_COMPOSE" ] && [ -x "$PREREQ_COMPOSE" ] && [ ! -L "$PREREQ_COMPOSE" ] || return 1
      printf '%s\n' "$PREREQ_COMPOSE" || return "${EX_PREREQ:-3}"
    }
    PREREQ_MISSING='' PREREQ_FIXES=''
    return 0
  fi
  pq_run_requalify='podman-compose (requalify)'
  if [ "${PREREQ_MISSING:-}" != "$pq_run_requalify" ]; then
    prereq_msg prereq_unavailable || return "${EX_PREREQ:-3}"
    return "${EX_PREREQ:-3}"
  fi
  pq_run_uid=0 pq_run_gid=0 pq_run_linger=0 pq_run_user='' pq_run_range=''
  case ${PREREQ_FIXES:-} in *[!abcdefghijklmnopqrstuvwxyz\ ]*) prereq_msg prereq_unavailable || return "${EX_PREREQ:-3}"; return "${EX_PREREQ:-3}" ;; esac
  for pq_run_fix in ${PREREQ_FIXES:-}; do
    case $pq_run_fix in
      subuid) pq_run_uid=1 ;;
      subgid) pq_run_gid=1 ;;
      linger) pq_run_linger=1 ;;
      *) prereq_msg prereq_unavailable || return "${EX_PREREQ:-3}"; return "${EX_PREREQ:-3}" ;;
    esac
  done
  if [ "$pq_run_uid$pq_run_gid$pq_run_linger" != 000 ]; then
    pq_run_user=$(id -un 2>/dev/null) || pq_run_user=''
    case $pq_run_user in ''|-*|*[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_.-]*) prereq_msg prereq_unavailable || return "${EX_PREREQ:-3}"; return "${EX_PREREQ:-3}" ;; esac
  fi
  if [ "$pq_run_uid$pq_run_gid" != 00 ]; then
    pq_run_range=$(prereq_subid_range "$(prereq_system_root)/etc/subuid" "$(prereq_system_root)/etc/subgid") || pq_run_range=''
    if [ -z "$pq_run_range" ]; then prereq_msg subids_failed || return "${EX_PREREQ:-3}"; return "${EX_PREREQ:-3}"; fi
  fi
  prereq_requalify_compose || return "$?"
  pq_run_verified_path=$PREREQ_COMPOSE
  say 'Prerequisite plan: use the verified installed podman-compose and make the following machine changes.' || return "${EX_PREREQ:-3}"
  say 'Privileged commands use /usr/bin/sudo and PATH=/usr/bin:/usr/sbin; package and account changes use umask 022.' || return "${EX_PREREQ:-3}"
  if [ "$pq_run_uid$pq_run_gid" != 00 ]; then
    pq_run_command='  sudo usermod'
    [ "$pq_run_uid" != 1 ] || pq_run_command="$pq_run_command --add-subuids $pq_run_range"
    [ "$pq_run_gid" != 1 ] || pq_run_command="$pq_run_command --add-subgids $pq_run_range"
    say "$pq_run_command -- $pq_run_user" || return "${EX_PREREQ:-3}"
    if ! [ -x /usr/bin/sudo ]; then prereq_msg no_sudo || return "${EX_PREREQ:-3}"; return "${EX_MACHINE:-2}"; fi
  fi
  if [ "$pq_run_linger" = 1 ]; then
    say "  loginctl enable-linger $pq_run_user" || return "${EX_PREREQ:-3}"
    say "  sudo loginctl enable-linger -- $pq_run_user" || return "${EX_PREREQ:-3}"
    if ! [ -x /usr/bin/sudo ]; then prereq_msg no_sudo || return "${EX_PREREQ:-3}"; return "${EX_MACHINE:-2}"; fi
  fi
  prereq_confirm || return "$?"
  if [ "$pq_run_uid$pq_run_gid" != 00 ]; then
    prereq_fix_subids "$pq_run_user" "$pq_run_range" "$pq_run_uid" "$pq_run_gid" || return "$?"
  fi
  if [ "$pq_run_linger" = 1 ]; then prereq_fix_linger "$pq_run_user" || return "$?"; fi
  if [ "$pq_run_uid$pq_run_gid$pq_run_linger" != 000 ]; then
    pq_run_look_status=0
    machine_look || pq_run_look_status=$?
    [ "$pq_run_look_status" -ne "${EX_PREREQ:-3}" ] || return "${EX_PREREQ:-3}"
    if [ "$pq_run_look_status" -ne 0 ] || [ "$PREREQ_MISSING" != "$pq_run_requalify" ] || [ -n "$PREREQ_FIXES" ]; then
      prereq_msg prereq_unavailable || return "${EX_PREREQ:-3}"
      return "${EX_PREREQ:-3}"
    fi
  fi
  PREREQ_COMPOSE=$pq_run_verified_path
  prereq_compose_path >/dev/null || { prereq_msg prereq_unavailable || return "${EX_PREREQ:-3}"; return "${EX_PREREQ:-3}"; }
  PREREQ_MISSING='' PREREQ_FIXES=''
  return 0
}

# Resolve a not-yet-created install directory to its nearest existing parent.
prereq_existing_dir() {
  case ${1-} in /*) pq_existing=$1 ;; *) return 1 ;; esac
  while [ ! -d "$pq_existing" ]; do
    [ "$pq_existing" != / ] || return 1
    pq_existing=${pq_existing%/*}
    [ -n "$pq_existing" ] || pq_existing=/
  done
  printf '%s\n' "$pq_existing"
}

prereq_disk_ok() {
  [ "$#" -eq 3 ] || return 1
  pq_disk_label=$2 pq_disk_min=$3
  pq_disk_data=$(LC_ALL=C df -P -T -B1 -- "$1" 2>/dev/null | LC_ALL=C awk '
    NR == 2 && NF >= 7 && $5 ~ /^[0123456789]+$/ { print $2, $5; ok = 1 }
    END { if (NR != 2 || !ok) exit 1 }
  ') || { prereq_msg disk_unknown "$pq_disk_label" || return "${EX_PREREQ:-3}"; return 1; }
  pq_disk_fs=${pq_disk_data%% *}
  pq_disk_free=${pq_disk_data#* }
  case $pq_disk_free in ''|*[!0123456789]*) prereq_msg disk_unknown "$pq_disk_label" || return "${EX_PREREQ:-3}"; return 1 ;; esac
  case $pq_disk_fs in
    xfs|ext4) ;;
    nfs|nfs4|cifs|smb3|smbfs|fuse*) prereq_msg fs_not_local "$pq_disk_label" || return "${EX_PREREQ:-3}"; return 1 ;;
    *) prereq_msg fs_unsupported "$pq_disk_label" || return "${EX_PREREQ:-3}"; return 1 ;;
  esac
  [ "$pq_disk_free" -ge "$pq_disk_min" ] 2>/dev/null || { prereq_msg disk_low "$pq_disk_label" || return "${EX_PREREQ:-3}"; return 1; }
  return 0
}

# Parse the local endpoint as a whole, so 8601 cannot match 18601.
prereq_port_free() {
  case ${1-} in ''|*[!0123456789]*) prereq_msg port_unknown || return "${EX_PREREQ:-3}"; return 1 ;; esac
  pq_port=$1
  while [ "${pq_port#0}" != "$pq_port" ]; do pq_port=${pq_port#0}; done
  [ -n "$pq_port" ] && [ "${#pq_port}" -le 5 ] && [ "$pq_port" -ge 1 ] && [ "$pq_port" -le 65535 ] || { prereq_msg port_unknown || return "${EX_PREREQ:-3}"; return 1; }
  if command -v ss >/dev/null 2>&1 && pq_sockets=$(ss -H -ltn 2>/dev/null); then
    pq_port_rc=0
    printf '%s\n' "$pq_sockets" | LC_ALL=C awk -v pq_port="$pq_port" '
      NF > 0 {
        if (NF < 4 || $4 !~ /:/) { bad = 1; next }
        endpoint = $4; sub(/^.*:/, "", endpoint)
        if (endpoint !~ /^[0123456789]+$/) { bad = 1; next }
        if (endpoint == pq_port) found = 1
      }
      END { if (bad) exit 2; if (found) exit 0; exit 1 }
    ' || pq_port_rc=$?
    case $pq_port_rc in
      0) prereq_msg port_busy "$pq_port" || return "${EX_PREREQ:-3}"; return 1 ;;
      1) return 0 ;;
      *) prereq_msg port_unknown || return "${EX_PREREQ:-3}"; return 1 ;;
    esac
  fi
  pq_port_hex=$(printf '%04X' "$pq_port") || { prereq_msg port_unknown || return "${EX_PREREQ:-3}"; return 1; }
  for pq_socket_file in "$(prereq_system_root)/proc/net/tcp" "$(prereq_system_root)/proc/net/tcp6"; do
    [ -r "$pq_socket_file" ] || { prereq_msg port_unknown || return "${EX_PREREQ:-3}"; return 1; }
    pq_port_rc=0
    LC_ALL=C awk -v pq_port_hex="$pq_port_hex" '
      NR == 1 { if ($0 !~ /local_address/ || $0 !~ /st/) bad = 1; next }
      NF > 0 {
        n = split($2, addr, ":")
        if (NF < 4 || n != 2 || (length(addr[1]) != 8 && length(addr[1]) != 32) ||
            length(addr[2]) != 4 || addr[1] !~ /^[0123456789ABCDEFabcdef]+$/ ||
            addr[2] !~ /^[0123456789ABCDEFabcdef]+$/ || $4 !~ /^[0123456789ABCDEFabcdef][0123456789ABCDEFabcdef]$/) { bad = 1; next }
        if (toupper($4) == "0A" && toupper(addr[2]) == pq_port_hex) found = 1
      }
      END { if (NR == 0 || bad) exit 2; if (found) exit 0; exit 1 }
    ' "$pq_socket_file" || pq_port_rc=$?
    case $pq_port_rc in
      0) prereq_msg port_busy "$pq_port" || return "${EX_PREREQ:-3}"; return 1 ;;
      1) ;;
      *) prereq_msg port_unknown || return "${EX_PREREQ:-3}"; return 1 ;;
    esac
  done
  return 0
}
