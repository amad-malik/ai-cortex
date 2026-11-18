# Installer prerequisite messages and checks. This file is joined into install.sh.

prereq_msg() {
  pq_msg_location=selected pq_msg_port=unknown
  case ${2-} in home|container) pq_msg_location=$2 ;; esac
  case ${2-} in ''|*[!0123456789]*) ;; *) [ "${#2}" -le 5 ] && pq_msg_port=$2 ;; esac
  case ${1-} in
    os_unsupported)
      pq_cause='this installer supports Rocky Linux 10 on x86_64 only'
      pq_fix='use Rocky Linux 10 on a supported machine; other systems follow in a later release'
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
    package_manager_unknown)
      pq_cause='the Rocky Linux package manager is unavailable'
      pq_fix='restore DNF through the system administrator and run the installer again'
      ;;
    compose_requalify)
      pq_cause='podman-compose is installed but its origin and signature are not proven'
      pq_fix='allow the installer to download and verify the exact package; this needs network access and one confirmation'
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
    *) return 1 ;;
  esac
  printf 'problem: %s\n' "$pq_cause" >&2
  printf 'fix: %s\n' "$pq_fix" >&2
}

prereq_info() {
  case ${1-} in
    selinux)
      case ${2-} in Enforcing|Permissive|Disabled) pq_info_selinux=$2 ;; *) pq_info_selinux=unknown ;; esac
      say "SELinux: $pq_info_selinux"
      ;;
    time_unknown) warn 'clock synchronization could not be determined; check the system clock' ;;
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

# Read the whole machine rather than stopping after the first blocking finding.
machine_look() {
  PREREQ_MISSING='' PREREQ_FIXES='' PREREQ_NEED_SUDO=0 PREREQ_COMPOSE=''
  pq_machine_block=0 pq_machine_supported=1
  if ! prereq_os_ok "${PREREQ_ROOT:-}/etc/os-release"; then
    prereq_msg os_unsupported
    pq_machine_block=1
    pq_machine_supported=0
  fi
  pq_machine_arch=$(uname -m 2>/dev/null) || pq_machine_arch=''
  if [ "$pq_machine_arch" != x86_64 ]; then
    prereq_msg arch_unsupported
    pq_machine_block=1
    pq_machine_supported=0
  fi
  pq_machine_uid=$(id -u 2>/dev/null) || pq_machine_uid=''
  case $pq_machine_uid in
    ''|*[!0123456789]*|0) prereq_msg is_root; pq_machine_block=1 ;;
  esac
  pq_machine_user=$(id -un 2>/dev/null) || pq_machine_user=''
  if [ -z "$pq_machine_user" ]; then
    prereq_msg is_root
    pq_machine_block=1
  else
    if ! prereq_subid_ok "$pq_machine_user" "${PREREQ_ROOT:-}/etc/subuid"; then
      prereq_msg no_subuid
      PREREQ_FIXES="${PREREQ_FIXES:+$PREREQ_FIXES }subuid"
      PREREQ_NEED_SUDO=1
    fi
    if ! prereq_subid_ok "$pq_machine_user" "${PREREQ_ROOT:-}/etc/subgid"; then
      prereq_msg no_subgid
      PREREQ_FIXES="${PREREQ_FIXES:+$PREREQ_FIXES }subgid"
      PREREQ_NEED_SUDO=1
    fi
  fi
  if [ ! -f "${PREREQ_ROOT:-}/sys/fs/cgroup/cgroup.controllers" ]; then
    prereq_msg no_cgroup_v2
    pq_machine_block=1
  fi
  if command -v podman >/dev/null 2>&1; then
    if ! prereq_podman_ok; then
      prereq_msg podman_old
      pq_machine_block=1
    fi
  else
    prereq_missing_add podman
    PREREQ_NEED_SUDO=1
  fi
  if [ "$pq_machine_supported" -eq 1 ]; then
    if ! command -v dnf >/dev/null 2>&1; then
      prereq_msg package_manager_unknown
      pq_machine_block=1
    fi
    if ! command -v rpm >/dev/null 2>&1; then
      prereq_msg rpm_unknown
      pq_machine_block=1
    elif ! rpm -q --quiet epel-release >/dev/null 2>&1; then
      prereq_msg epel_needed
      PREREQ_FIXES="${PREREQ_FIXES:+$PREREQ_FIXES }epel"
      PREREQ_NEED_SUDO=1
    fi
    if command -v rpm >/dev/null 2>&1; then
      if rpm -q --quiet podman-compose >/dev/null 2>&1; then
        prereq_msg compose_requalify
        prereq_missing_add 'podman-compose (requalify)'
      else
        prereq_msg compose_missing
        prereq_missing_add podman-compose
        PREREQ_NEED_SUDO=1
      fi
    fi
  fi
  for pq_command_package in \
    curl:curl tar:tar gzip:gzip python3:python3 ssh-keygen:openssh-clients gpg:gnupg2 gpgv:gnupg2 rpmkeys:rpm \
    awk:gawk sed:sed grep:grep sha256sum:coreutils cat:coreutils chmod:coreutils \
    cp:coreutils cut:coreutils head:coreutils ln:coreutils mkdir:coreutils \
    mktemp:coreutils mv:coreutils readlink:coreutils rm:coreutils sort:coreutils \
    stat:coreutils tr:coreutils wc:coreutils find:findutils; do
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
    no) prereq_msg time_not_synced; pq_machine_block=1 ;;
    *) prereq_info time_unknown ;;
  esac
  pq_machine_selinux=''
  if command -v getenforce >/dev/null 2>&1; then
    pq_machine_selinux=$(getenforce 2>/dev/null) || pq_machine_selinux=''
  fi
  prereq_info selinux "$pq_machine_selinux"
  if [ -n "$pq_machine_user" ]; then
    pq_machine_linger=''
    if command -v loginctl >/dev/null 2>&1; then
      pq_machine_linger=$(loginctl show-user "$pq_machine_user" -p Linger --value 2>/dev/null) || pq_machine_linger=''
    fi
    if [ "$pq_machine_linger" != yes ]; then
      prereq_msg linger_off
      PREREQ_FIXES="${PREREQ_FIXES:+$PREREQ_FIXES }linger"
    fi
  fi
  pq_machine_min=${CORTEX_MIN_FREE_GIB:-20}
  case $pq_machine_min in
    ''|*[!0123456789]*) prereq_msg disk_unknown 'home'; pq_machine_block=1; pq_machine_min='' ;;
  esac
  if [ -n "$pq_machine_min" ]; then
    while [ "${pq_machine_min#0}" != "$pq_machine_min" ]; do pq_machine_min=${pq_machine_min#0}; done
    [ -n "$pq_machine_min" ] || pq_machine_min=0
  fi
  if [ -n "$pq_machine_min" ] && [ "${#pq_machine_min}" -gt 9 ]; then
    prereq_msg disk_unknown 'home'
    pq_machine_block=1
    pq_machine_min=''
  fi
  if [ -n "$pq_machine_min" ]; then
    pq_machine_min_bytes=$((pq_machine_min * 1073741824))
    pq_machine_home=$(prereq_existing_dir "${home_dir:-}") || pq_machine_home=''
    if [ -z "$pq_machine_home" ] || ! prereq_disk_ok "$pq_machine_home" home "$pq_machine_min_bytes"; then
      pq_machine_block=1
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
      prereq_msg graph_unknown
      pq_machine_block=1
    elif ! prereq_disk_ok "$pq_machine_graph" container "$pq_machine_min_bytes"; then
      pq_machine_block=1
    fi
  fi
  if ! prereq_port_free "${port:-}"; then
    pq_machine_block=1
  fi
  if [ "$PREREQ_NEED_SUDO" -eq 1 ] && ! command -v sudo >/dev/null 2>&1; then
    prereq_msg no_sudo
    pq_machine_block=1
  fi
  if [ "${check_only:-0}" != 1 ] && [ -n "$PREREQ_MISSING$PREREQ_FIXES" ] && [ "${CORTEX_YES:-0}" != 1 ] && ! prereq_tty_available; then
    prereq_msg no_tty
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
  [ "$pq_os_id" = rocky ] || return 1
  case $pq_os_version in
    10) return 0 ;;
    10.*)
      pq_os_minor=${pq_os_version#10.}
      case $pq_os_minor in ''|*[!0123456789.]*|.*|*.|*..*) return 1 ;; esac
      return 0
      ;;
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
  case $pq_fix_user in ''|*[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_.-]*) prereq_msg subids_failed; return "${EX_PREREQ:-3}" ;; esac
  case $pq_fix_uid:$pq_fix_gid in 1:0|0:1|1:1) ;; *) prereq_msg subids_failed; return "${EX_PREREQ:-3}" ;; esac
  pq_fix_current=$(prereq_subid_range "${PREREQ_ROOT:-}/etc/subuid" "${PREREQ_ROOT:-}/etc/subgid") || pq_fix_current=''
  if [ -z "$pq_fix_current" ] || [ "$pq_fix_current" != "$pq_fix_range" ]; then prereq_msg subids_failed; return "${EX_PREREQ:-3}"; fi
  set -- usermod
  [ "$pq_fix_uid" != 1 ] || set -- "$@" --add-subuids "$pq_fix_range"
  [ "$pq_fix_gid" != 1 ] || set -- "$@" --add-subgids "$pq_fix_range"
  if ! sudo "$@" "$pq_fix_user"; then prereq_msg subids_failed; return "${EX_PREREQ:-3}"; fi
  if { [ "$pq_fix_uid" = 1 ] && ! prereq_subid_has_range "$pq_fix_user" "$pq_fix_range" "${PREREQ_ROOT:-}/etc/subuid"; } ||
     { [ "$pq_fix_gid" = 1 ] && ! prereq_subid_has_range "$pq_fix_user" "$pq_fix_range" "${PREREQ_ROOT:-}/etc/subgid"; }; then
    prereq_msg subids_failed
    return "${EX_PREREQ:-3}"
  fi
  return 0
}

prereq_fix_linger() {
  if [ "$#" -ne 1 ]; then prereq_msg linger_off; return "${EX_PREREQ:-3}"; fi
  pq_linger_user=$1
  case $pq_linger_user in ''|*[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_.-]*) prereq_msg linger_off; return "${EX_PREREQ:-3}" ;; esac
  pq_linger_state=$(loginctl show-user "$pq_linger_user" -p Linger --value 2>/dev/null) || pq_linger_state=''
  [ "$pq_linger_state" != yes ] || return 0
  if ! loginctl enable-linger "$pq_linger_user"; then
    if ! sudo loginctl enable-linger "$pq_linger_user"; then prereq_msg linger_off; return "${EX_PREREQ:-3}"; fi
  fi
  pq_linger_state=$(loginctl show-user "$pq_linger_user" -p Linger --value 2>/dev/null) || pq_linger_state=''
  if [ "$pq_linger_state" != yes ]; then prereq_msg linger_off; return "${EX_PREREQ:-3}"; fi
  return 0
}

prereq_tty_available() {
  pq_tty=${PREREQ_ROOT:-}/dev/tty
  [ -c "$pq_tty" ] && ( : <> "$pq_tty" ) 2>/dev/null
}

prereq_confirm() {
  [ "${CORTEX_YES:-0}" != 1 ] || return 0
  if ! prereq_tty_available; then prereq_msg no_tty; return "${EX_MACHINE:-2}"; fi
  say 'Proceed with these changes? [y/N]'
  pq_confirm_answer=''
  IFS= read -r pq_confirm_answer < "$pq_tty" || pq_confirm_answer=''
  case $pq_confirm_answer in
    y|Y|yes|yEs|yeS|yES|Yes|YEs|YeS|YES) return 0 ;;
    *) prereq_msg consent_declined; return "${EX_MACHINE:-2}" ;;
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
  [ "$#" -eq 1 ] || return 1
  case $1 in
    10) pq_repo_expected=epel-10 ;;
    10.*)
      pq_repo_minor=${1#10.}
      case $pq_repo_minor in ''|*[!0123456789]*) return 1 ;; esac
      pq_repo_expected=epel-z-$1 ;;
    *) return 1 ;;
  esac
  PREREQ_EPEL_METALINK='' PREREQ_EPEL_REPO_GPGCHECK=''
  pq_repo_config=$(LC_ALL=C dnf "--releasever=$1" --cacheonly --assumeno config-manager --dump epel 2>/dev/null) || return 1
  pq_repo_selected=$(printf '%s\n' "$pq_repo_config" | LC_ALL=C awk -v expected="$pq_repo_expected" '
    function trim(s) { sub(/^[[:space:]]+/, "", s); sub(/[[:space:]]+$/, "", s); return s }
    function yes(s) { s = tolower(s); return s == "1" || s == "true" }
    function boolean(s) { s = tolower(s); return s == "0" || s == "1" || s == "true" || s == "false" }
    /^[=]+[[:space:]]+repo: epel[[:space:]]+[=]+$/ { sections++; next }
    /^[=]+.*[=]+$/ { bad = 1; next }
    /^[[:space:]]*$/ { next }
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
      split("baseurl metalink mirrorlist gpgcheck gpgkey sslverify sslcacert repo_gpgcheck proxy proxy_sslverify proxy_sslcacert", required, " ")
      for (i in required) if (seen[required[i]] != 1) exit 1
      if (values["baseurl"] != "" || values["mirrorlist"] != "" || values["sslcacert"] != "" ||
          values["proxy_sslcacert"] != "" || !yes(values["gpgcheck"]) || !yes(values["sslverify"]) ||
          !yes(values["proxy_sslverify"]) || !boolean(values["repo_gpgcheck"]) ||
          values["gpgkey"] != "file:///etc/pki/rpm-gpg/RPM-GPG-KEY-EPEL-10") exit 1
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
  case $1 in
    10) ;;
    10.*) pq_rocky_minor=${1#10.}; case $pq_rocky_minor in ''|*[!0123456789]*) return 1 ;; esac ;;
    *) return 1 ;;
  esac
  case $2 in baseos) pq_rocky_channel=BaseOS ;; appstream) pq_rocky_channel=AppStream ;; crb) pq_rocky_channel=CRB ;; extras) pq_rocky_channel=extras ;; *) return 1 ;; esac
  PREREQ_ROCKY_REPO_GPGCHECK=''
  pq_rocky_config=$(LC_ALL=C dnf "--releasever=$1" --cacheonly --assumeno config-manager --dump "$2" 2>/dev/null) || return 1
  pq_rocky_policy=$(printf '%s\n' "$pq_rocky_config" | LC_ALL=C awk -v section="$2" -v expected="$pq_rocky_channel-$1" '
    function trim(s) { sub(/^[[:space:]]+/, "", s); sub(/[[:space:]]+$/, "", s); return s }
    function yes(s) { s = tolower(s); return s == "1" || s == "true" }
    function boolean(s) { s = tolower(s); return s == "0" || s == "1" || s == "true" || s == "false" }
    /^[=]+.*[=]+$/ { if (index($0, "repo: " section " ")) sections++; else bad = 1; next }
    /^[[:space:]]*$/ { next }
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
      split("baseurl metalink mirrorlist gpgcheck gpgkey sslverify sslcacert repo_gpgcheck proxy proxy_sslverify proxy_sslcacert", required, " ")
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
    if not blocks:
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
  case $pq_locked_path in *[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_./-]*|*/../*|*/..|*//* ) return 1 ;; esac
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

# The manifest must come from authenticated allowed Rocky metadata; this helper
# checks every selected exact archive with one isolated inherited root set.
prereq_rocky_archives_ok() {
  [ "$#" -eq 2 ] || return 1
  pq_archives_stage=$1 pq_archives_manifest=$2
  pq_archives_ring=$pq_archives_stage/rocky-ring
  [ "$pq_archives_manifest" = "$pq_archives_stage/transaction.manifest" ] || return 1
  [ -d "$pq_archives_ring" ] && [ -d "$pq_archives_stage/empty" ] && [ -f "$pq_archives_manifest" ] || return 1
  prereq_root_locked_path "$pq_archives_ring" && prereq_root_locked_path "$pq_archives_stage/empty" && prereq_root_locked_path "$pq_archives_manifest" || return 1
  pq_archives_key_count=0
  for pq_archives_key in "$pq_archives_stage/empty"/*.key; do [ ! -e "$pq_archives_key" ] && [ ! -L "$pq_archives_key" ] || return 1; done
  for pq_archives_key in "$pq_archives_ring"/*.key; do
    [ -f "$pq_archives_key" ] && [ ! -L "$pq_archives_key" ] && prereq_root_locked_path "$pq_archives_key" || return 1
    pq_archives_key_count=$((pq_archives_key_count + 1))
  done
  [ "$pq_archives_key_count" -gt 0 ] || return 1
  pq_archives_count=0 pq_archives_receipts='' pq_archives_seen='
'
  while IFS='|' read -r pq_archive_file pq_archive_hash pq_archive_name pq_archive_epoch pq_archive_version pq_archive_release pq_archive_arch pq_archive_repo || [ -n "$pq_archive_file" ]; do
    case $pq_archive_file in "$pq_archives_stage"/packages/*) ;; *) return 1 ;; esac
    case $pq_archive_file in *[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_./-]*|*/../*|*/..|*//* ) return 1 ;; esac
    case $pq_archives_seen in *"
$pq_archive_file
"*) return 1 ;; esac
    pq_archives_seen="$pq_archives_seen$pq_archive_file
"
    [ -f "$pq_archive_file" ] && [ ! -L "$pq_archive_file" ] && prereq_root_locked_path "$pq_archive_file" || return 1
    case $pq_archive_hash in *[!0123456789abcdef]*|'') return 1 ;; esac
    [ "${#pq_archive_hash}" -eq 64 ] || return 1
    case $pq_archive_name in ''|*[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_.+-]*) return 1 ;; esac
    case $pq_archive_epoch in ''|*[!0123456789]*) return 1 ;; esac
    case $pq_archive_version:$pq_archive_release in *[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_.+:-]*|:*|*:) return 1 ;; esac
    case $pq_archive_arch in noarch|x86_64) ;; *) return 1 ;; esac
    case $pq_archive_repo in baseos|appstream|crb|extras) ;; *) return 1 ;; esac
    pq_archive_epel=epel-release
    if [ "$pq_archive_name" = "$pq_archive_epel" ] && [ "$pq_archive_repo" != extras ]; then return 1; fi
    pq_archive_actual_hash=$(sha256sum -- "$pq_archive_file") || return 1
    pq_archive_actual_hash=${pq_archive_actual_hash%% *}
    [ "$pq_archive_actual_hash" = "$pq_archive_hash" ] || return 1
    pq_archive_identity=$(LC_ALL=C rpm -qp --queryformat '%{NAME}|%{EPOCHNUM}|%{VERSION}|%{RELEASE}|%{ARCH}\n' "$pq_archive_file" 2>/dev/null) || return 1
    [ "$pq_archive_identity" = "$pq_archive_name|$pq_archive_epoch|$pq_archive_version|$pq_archive_release|$pq_archive_arch" ] || return 1
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
    pq_archives_count=$((pq_archives_count + 1))
    pq_archives_receipts="${pq_archives_receipts}${pq_archives_receipts:+
}rocky_archive=$pq_archive_identity repository=$pq_archive_repo sha256=$pq_archive_hash signature=verified"
  done < "$pq_archives_manifest"
  [ "$pq_archives_count" -gt 0 ] || return 1
  printf '%s\n' "$pq_archives_receipts"
}

# Bind each selected archive to current official HTTPS Rocky metadata. If a
# repository requires metadata signatures, refuse until they can be checked.
prereq_rocky_metadata_ok() {
  [ "$#" -eq 3 ] || return 1
  case $1 in
    10) ;;
    10.*) pq_rocky_meta_minor=${1#10.}; case $pq_rocky_meta_minor in ''|*[!0123456789]*) return 1 ;; esac ;;
    *) return 1 ;;
  esac
  case $2 in */transaction.manifest) ;; *) return 1 ;; esac
  [ -f "$2" ] && [ ! -L "$2" ] && prereq_root_locked_path "$2" || return 1
  python3 -I - "$1" "$2" "$3" <<'PREREQ_ROCKY_METADATA_PY'
import gzip
import hashlib
import io
import os
from pathlib import Path
import subprocess
import sys
from urllib.parse import urljoin
import xml.etree.ElementTree as ET

release, manifest, policy_text = sys.argv[1:]
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

def get(url, limit):
    result = subprocess.run(
        ["curl", "--disable", "--fail", "--silent", "--show-error", "--location",
         "--proto", "=https", "--proto-redir", "=https", "--max-time", "45",
         "--max-filesize", str(limit), "--", url],
        stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
        env={"PATH": os.environ.get("PATH", ""), "LC_ALL": "C"}, check=False,
    )
    require(result.returncode == 0 and len(result.stdout) <= limit)
    return result.stdout

try:
    policy = {}
    for part in policy_text.split(","):
        pair = part.split("=")
        require(len(pair) == 2 and pair[0] in channels and pair[0] not in policy and pair[1] in ("0", "1"))
        policy[pair[0]] = pair[1]
    records = []
    for line in Path(manifest).read_text(encoding="ascii").splitlines():
        row = line.split("|")
        require(len(row) == 8 and row[7] in channels and valid_hash(row[1]))
        require(row[7] in policy and policy[row[7]] == "0")
        records.append(row)
    require(records and len({row[0] for row in records}) == len(records))
    receipts = []
    for repo in sorted({row[7] for row in records}):
        base = "https://dl.rockylinux.org/pub/rocky/" + release + "/" + channels[repo] + "/x86_64/os/"
        repomd = get(urljoin(base, "repodata/repomd.xml"), 4194304)
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
        packed = get(urljoin(base, href), 134217728)
        require(sha(packed) == packed_hash)
        with gzip.GzipFile(fileobj=io.BytesIO(packed)) as source:
            raw = source.read(268435457)
        require(len(raw) <= 268435456 and sha(raw) == raw_hash)
        found = {row[0]: 0 for row in records if row[7] == repo}
        for package in xml(raw):
            if local(package.tag) != "package" or package.get("type") != "rpm":
                continue
            children = {}
            for child in package:
                key = local(child.tag)
                require(key not in children)
                children[key] = child
            if "version" not in children or "name" not in children or "arch" not in children:
                continue
            version = children["version"]
            identity = [children["name"].text, version.get("epoch"), version.get("ver"), version.get("rel"), children["arch"].text]
            for row in records:
                if row[7] == repo and identity == row[2:7]:
                    check = children["checksum"]
                    require(check.get("type", "").lower() == "sha256" and (check.text or "").strip().lower() == row[1])
                    found[row[0]] += 1
        require(all(count == 1 for count in found.values()))
        receipts.append("rocky_metadata=" + repo + " repomd_sha256=" + sha(repomd) + " primary_sha256=" + packed_hash)
    print("\n".join(receipts))
except (OSError, ValueError, KeyError, TypeError, EOFError, OverflowError, ET.ParseError, subprocess.SubprocessError):
    raise SystemExit(1)
PREREQ_ROCKY_METADATA_PY
}

# Acquire only the exact requested NEVRA through the official metalink's
# checksum chain. Signature and installed-payload checks remain separate.
prereq_epel_fetch() {
  [ "$#" -eq 5 ] || return 1
  case $4 in
    0) ;;
    1) mkdir -m 700 "$3/metadata-gpg-home" || return 1
       prereq_epel_key_ok "$3/metadata-gpg-home" "$5" || return 1 ;;
    *) return 1 ;;
  esac
  python3 -I - "$1" "$2" "$3" "$4" "$5" <<'PREREQ_EPEL_PY'
import gzip
import hashlib
import os
from pathlib import Path
import stat
import subprocess
import sys
from urllib.parse import parse_qsl, urljoin, urlsplit
import xml.etree.ElementTree as ET

metalink_url, requested, directory, metadata_check, keyfile = sys.argv[1:]
stage = Path(directory)
created = []

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

def get(url, name, limit):
    target = stage / name
    with target.open("xb") as output:
        created.append(target)
        result = subprocess.run(
            ["curl", "--disable", "--fail", "--silent", "--show-error", "--location",
             "--proto", "=https", "--proto-redir", "=https", "--max-time", "45",
             "--max-filesize", str(limit), "--", url],
            stdout=output, stderr=subprocess.DEVNULL,
            env={"PATH": os.environ.get("PATH", ""), "LC_ALL": "C"}, check=False,
        )
    require(result.returncode == 0 and target.stat().st_size <= limit)
    return target.read_bytes()

try:
    st = stage.lstat()
    require(stage.is_absolute() and stat.S_ISDIR(st.st_mode) and not stage.is_symlink())
    require(st.st_uid == os.getuid() and stat.S_IMODE(st.st_mode) == 0o700)
    fields = requested.split("|")
    require(len(fields) == 5 and fields[0] == "podman-compose" and fields[1].isdigit())
    require(fields[4] in ("noarch", "x86_64"))
    require(all(field and all(c.isascii() and (c.isalnum() or c in "._+-") for c in field) for field in fields))
    parsed = urlsplit(metalink_url)
    require(parsed.scheme == "https" and parsed.netloc == "mirrors.fedoraproject.org" and parsed.path == "/metalink" and not parsed.fragment)
    params = parse_qsl(parsed.query, keep_blank_values=True, strict_parsing=True)
    require(len(dict(params)) == len(params) and all(k in ("repo", "arch", "infra", "content") for k, _ in params))
    query = dict(params)
    repo = query.get("repo", "")
    require(query.get("arch") == "x86_64" and (repo == "epel-10" or
            (repo.startswith("epel-z-10.") and repo.removeprefix("epel-z-10.").isdigit())))
    metalink = xml(get(metalink_url, "metalink.xml", 1048576))
    record = one([e for e in metalink.iter() if local(e.tag) == "file" and e.get("name") == "repomd.xml"])
    hashes = [e.text.strip().lower() for e in record.iter() if local(e.tag) == "hash" and e.get("type", "").lower() == "sha256" and e.text]
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
    repomd_url = one(mirrors[:1])
    repomd = get(repomd_url, "repomd.xml", 4194304)
    require(digest(repomd) == expected_repomd)
    if metadata_check == "1":
        get(repomd_url + ".asc", "repomd.xml.asc", 1048576)
        home = stage / "metadata-gpg-home"
        keyring = stage / "metadata-keyring.gpg"
        require(not keyring.exists() and not keyring.is_symlink())
        created.append(keyring)
        clean_env = {"PATH": os.environ.get("PATH", ""), "LC_ALL": "C"}
        prepared = subprocess.run(
            ["gpg", "--batch", "--no-options", "--no-tty", "--homedir", str(home),
             "--dearmor", "--output", str(keyring), keyfile],
            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, env=clean_env, check=False,
        )
        require(prepared.returncode == 0 and keyring.is_file() and not keyring.is_symlink())
        checked = subprocess.run(
            ["gpgv", "--homedir", str(home), "--keyring", str(keyring), "--status-fd", "1",
             str(stage / "repomd.xml.asc"), str(stage / "repomd.xml")],
            stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, env=clean_env, check=False,
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
    require(primary_href.endswith(".xml.gz"))
    base = repomd_url[:-len("repodata/repomd.xml")]
    compressed = get(urljoin(base, primary_href), "primary.xml.gz", 134217728)
    require(digest(compressed) == sha256((checksum.text or "").strip().lower()))
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
    archive = get(package_url, "compose.rpm", 134217728)
    require(digest(archive) == expected_package)
    receipt = stage / "receipt.txt"
    with receipt.open("x", encoding="utf-8") as output:
        created.append(receipt)
        output.write("repository=stable-epel\nmetalink=" + metalink_url + "\nrepomd=" + repomd_url +
                     "\nrepomd_sha256=" + expected_repomd + "\npackage=" + package_url +
                     "\npackage_sha256=" + expected_package + "\nmetadata_signature=" +
                     ("verified" if metadata_check == "1" else "not-enabled") + "\nnevra=" + requested + "\n")
except (OSError, ValueError, KeyError, TypeError, EOFError, OverflowError, ET.ParseError, subprocess.SubprocessError):
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

prereq_requalify_error() {
  PREREQ_COMPOSE=''
  if [ -n "${pq_requalify_stage:-}" ]; then rm -rf -- "$pq_requalify_stage"; fi
  prereq_msg compose_verify_failed
  return "${EX_PREREQ:-3}"
}

# Called only after approval; it never replaces or executes an installed provider.
prereq_requalify_compose() {
  PREREQ_COMPOSE='' pq_requalify_stage=''
  prereq_compose_path() { return 1; }
  case ${HOME:-} in /*) ;; *) prereq_requalify_error; return "${EX_PREREQ:-3}" ;; esac
  pq_requalify_stage=$(mktemp -d "$HOME/.cortex-prereq.XXXXXXXX") || { prereq_requalify_error; return "${EX_PREREQ:-3}"; }
  case $pq_requalify_stage in "$HOME"/.cortex-prereq.*) ;; *) pq_requalify_stage=''; prereq_requalify_error; return "${EX_PREREQ:-3}" ;; esac
  if ! mkdir -m 700 "$pq_requalify_stage/gpg" "$pq_requalify_stage/empty" "$pq_requalify_stage/pinned"; then
    prereq_requalify_error; return "${EX_PREREQ:-3}"
  fi
  if ! prereq_os_ok "${PREREQ_ROOT:-}/etc/os-release" ||
     ! cp -- "${PREREQ_ROOT:-}/etc/pki/rpm-gpg/RPM-GPG-KEY-EPEL-10" "$pq_requalify_stage/pinned/certificate.key" ||
     ! prereq_epel_key_ok "$pq_requalify_stage/gpg" "$pq_requalify_stage/pinned/certificate.key" ||
     ! prereq_epel_repo_config_ok "$pq_os_version"; then
    prereq_requalify_error; return "${EX_PREREQ:-3}"
  fi
  pq_requalify_identity=$(LC_ALL=C rpm -q --queryformat '%{NAME}|%{EPOCHNUM}|%{VERSION}|%{RELEASE}|%{ARCH}\n' podman-compose 2>/dev/null) || pq_requalify_identity=''
  pq_requalify_provider=${PREREQ_ROOT:-}/usr/bin/podman-compose
  if [ -z "$pq_requalify_identity" ] ||
     ! prereq_epel_fetch "$PREREQ_EPEL_METALINK" "$pq_requalify_identity" "$pq_requalify_stage" "$PREREQ_EPEL_REPO_GPGCHECK" "$pq_requalify_stage/pinned/certificate.key" ||
     ! prereq_rpm_signature_ok "$pq_requalify_stage/gpg" "$pq_requalify_stage/empty" "$pq_requalify_stage/pinned" "$pq_requalify_stage/compose.rpm" ||
     ! prereq_rpm_payload_ok "$pq_requalify_stage/compose.rpm" "$pq_requalify_provider"; then
    prereq_requalify_error; return "${EX_PREREQ:-3}"
  fi
  pq_requalify_rpm=$(LC_ALL=C rpm --version 2>/dev/null) || { prereq_requalify_error; return "${EX_PREREQ:-3}"; }
  pq_requalify_rpmkeys=$(LC_ALL=C rpmkeys --version 2>/dev/null) || { prereq_requalify_error; return "${EX_PREREQ:-3}"; }
  pq_requalify_gpg=$(LC_ALL=C gpg --version 2>/dev/null) || { prereq_requalify_error; return "${EX_PREREQ:-3}"; }
  pq_requalify_newline='
'
  pq_requalify_rpm=${pq_requalify_rpm%%"$pq_requalify_newline"*}
  pq_requalify_rpmkeys=${pq_requalify_rpmkeys%%"$pq_requalify_newline"*}
  pq_requalify_gpg=${pq_requalify_gpg%%"$pq_requalify_newline"*}
  say 'Compose requalification: private pinned EPEL certificate and isolated RPM keyring verified.'
  say "Verifier rpm: $pq_requalify_rpm"
  say "Verifier rpmkeys: $pq_requalify_rpmkeys"
  say "Verifier gpg: $pq_requalify_gpg"
  say 'RPM verifier: private filesystem keyring, signatures and digests required, empty-keyring negative control passed.'
  pq_requalify_fingerprints=$(printf '%s\n' "$pq_key_records" | LC_ALL=C awk -F: '$1 == "fpr" && length($10) == 40 && $10 ~ /^[0123456789ABCDEF]+$/ { print $10 }')
  while IFS= read -r pq_requalify_line; do
    [ -z "$pq_requalify_line" ] || say "Certificate fingerprint: $pq_requalify_line"
  done <<PREREQ_FINGERPRINTS
$pq_requalify_fingerprints
PREREQ_FINGERPRINTS
  while IFS= read -r pq_requalify_line; do
    case $pq_requalify_line in *Signature*|*signature*|*digest*) say "RPM verification: $pq_requalify_line" ;; esac
  done <<PREREQ_RPM_OUTPUT
$pq_sig_output
PREREQ_RPM_OUTPUT
  while IFS= read -r pq_requalify_line; do say "$pq_requalify_line"; done < "$pq_requalify_stage/receipt.txt"
  say "Verified installed package: $pq_requalify_identity"
  say "Verified absolute provider: $pq_requalify_provider"
  if ! rm -rf -- "$pq_requalify_stage"; then pq_requalify_stage=''; prereq_requalify_error; return "${EX_PREREQ:-3}"; fi
  pq_requalify_stage='' pq_verified_path=$pq_requalify_provider PREREQ_COMPOSE=$pq_requalify_provider
  prereq_compose_path() {
    [ -n "${PREREQ_COMPOSE:-}" ] && [ "$PREREQ_COMPOSE" = "$pq_verified_path" ] &&
      [ -f "$PREREQ_COMPOSE" ] && [ -x "$PREREQ_COMPOSE" ] && [ ! -L "$PREREQ_COMPOSE" ] || return 1
    printf '%s\n' "$PREREQ_COMPOSE"
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
      podman|curl|tar|gzip|python3|openssh-clients|gnupg2|rpm|gawk|sed|grep|coreutils|findutils)
        PREREQ_PLAN_PACKAGES="${PREREQ_PLAN_PACKAGES}${PREREQ_PLAN_PACKAGES:+ }$pq_plan_item" ;;
      *) return 1 ;;
    esac
  done <<PREREQ_MISSING_ITEMS
${PREREQ_MISSING:-}
PREREQ_MISSING_ITEMS
  [ -n "$PREREQ_PLAN_COMPOSE" ]
}

prereq_package_plan() {
  [ "$#" -eq 3 ] && prereq_parse_missing || return 1
  pq_package_release=$1 pq_package_stage=$2 pq_package_epel=$3
  case $pq_package_release in
    10) ;;
    10.*) pq_package_minor=${pq_package_release#10.}; case $pq_package_minor in ''|*[!0123456789]*) return 1 ;; esac ;;
    *) return 1 ;;
  esac
  case $pq_package_epel in 0|1) ;; *) return 1 ;; esac
  if [ "$PREREQ_PLAN_COMPOSE" = install ]; then
    case $pq_package_stage in /*) ;; *) return 1 ;; esac
    case $pq_package_stage in *[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_./-]*|*/../*|*/..|*//* ) return 1 ;; esac
  fi
  if [ -n "$PREREQ_PLAN_PACKAGES" ]; then
    say "  sudo dnf --releasever=$pq_package_release --repo=baseos --repo=appstream --repo=crb install -y $PREREQ_PLAN_PACKAGES"
  fi
  if [ "$pq_package_epel" = 1 ]; then
    say "  sudo dnf --releasever=$pq_package_release --repo=extras install -y epel-release"
  fi
  if [ "$PREREQ_PLAN_COMPOSE" = install ]; then
    say "  sudo dnf --releasever=$pq_package_release --repo=baseos --repo=appstream --repo=crb --repo=epel install -y $pq_package_stage/compose.rpm"
  fi
  return 0
}

# Step 2 consumes the read-only findings from machine_look.
prereq_run() {
  if [ -z "${PREREQ_MISSING:-}${PREREQ_FIXES:-}" ]; then
    if ! prereq_compose_path >/dev/null; then prereq_msg prereq_unavailable; return "${EX_PREREQ:-3}"; fi
    say 'Prerequisites are already ready.'
    return 0
  fi
  pq_run_requalify='podman-compose (requalify)'
  if [ "${PREREQ_MISSING:-}" != "$pq_run_requalify" ]; then
    prereq_msg prereq_unavailable
    return "${EX_PREREQ:-3}"
  fi
  pq_run_uid=0 pq_run_gid=0 pq_run_linger=0 pq_run_user='' pq_run_range=''
  case ${PREREQ_FIXES:-} in *[!abcdefghijklmnopqrstuvwxyz\ ]*) prereq_msg prereq_unavailable; return "${EX_PREREQ:-3}" ;; esac
  for pq_run_fix in ${PREREQ_FIXES:-}; do
    case $pq_run_fix in
      subuid) pq_run_uid=1 ;;
      subgid) pq_run_gid=1 ;;
      linger) pq_run_linger=1 ;;
      *) prereq_msg prereq_unavailable; return "${EX_PREREQ:-3}" ;;
    esac
  done
  if [ "$pq_run_uid$pq_run_gid$pq_run_linger" != 000 ]; then
    pq_run_user=$(id -un 2>/dev/null) || pq_run_user=''
    case $pq_run_user in ''|*[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_.-]*) prereq_msg prereq_unavailable; return "${EX_PREREQ:-3}" ;; esac
  fi
  if [ "$pq_run_uid$pq_run_gid" != 00 ]; then
    pq_run_range=$(prereq_subid_range "${PREREQ_ROOT:-}/etc/subuid" "${PREREQ_ROOT:-}/etc/subgid") || pq_run_range=''
    if [ -z "$pq_run_range" ]; then prereq_msg subids_failed; return "${EX_PREREQ:-3}"; fi
  fi
  say 'Prerequisite plan: verify the installed podman-compose using the stable EPEL community repository; network access is required.'
  if [ "$pq_run_uid$pq_run_gid" != 00 ]; then
    pq_run_command='  sudo usermod'
    [ "$pq_run_uid" != 1 ] || pq_run_command="$pq_run_command --add-subuids $pq_run_range"
    [ "$pq_run_gid" != 1 ] || pq_run_command="$pq_run_command --add-subgids $pq_run_range"
    say "$pq_run_command $pq_run_user"
    if ! command -v sudo >/dev/null 2>&1; then prereq_msg no_sudo; return "${EX_MACHINE:-2}"; fi
  fi
  if [ "$pq_run_linger" = 1 ]; then
    say "  loginctl enable-linger $pq_run_user"
    say '  If needed:'
    say "  sudo loginctl enable-linger $pq_run_user"
  fi
  prereq_confirm || return "$?"
  if [ "$pq_run_uid$pq_run_gid" != 00 ]; then
    prereq_fix_subids "$pq_run_user" "$pq_run_range" "$pq_run_uid" "$pq_run_gid" || return "$?"
  fi
  if [ "$pq_run_linger" = 1 ]; then prereq_fix_linger "$pq_run_user" || return "$?"; fi
  if [ "$pq_run_uid$pq_run_gid$pq_run_linger" != 000 ]; then
    if ! machine_look || [ "$PREREQ_MISSING" != "$pq_run_requalify" ] || [ -n "$PREREQ_FIXES" ]; then
      prereq_msg prereq_unavailable
      return "${EX_PREREQ:-3}"
    fi
  fi
  prereq_requalify_compose || return "$?"
  prereq_compose_path >/dev/null || { prereq_msg prereq_unavailable; return "${EX_PREREQ:-3}"; }
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
  ') || { prereq_msg disk_unknown "$pq_disk_label"; return 1; }
  pq_disk_fs=${pq_disk_data%% *}
  pq_disk_free=${pq_disk_data#* }
  case $pq_disk_free in ''|*[!0123456789]*) prereq_msg disk_unknown "$pq_disk_label"; return 1 ;; esac
  case $pq_disk_fs in
    xfs|ext4) ;;
    nfs|nfs4|cifs|smb3|smbfs|fuse*) prereq_msg fs_not_local "$pq_disk_label"; return 1 ;;
    *) prereq_msg fs_unsupported "$pq_disk_label"; return 1 ;;
  esac
  [ "$pq_disk_free" -ge "$pq_disk_min" ] 2>/dev/null || { prereq_msg disk_low "$pq_disk_label"; return 1; }
  return 0
}

# Parse the local endpoint as a whole, so 8601 cannot match 18601.
prereq_port_free() {
  case ${1-} in ''|*[!0123456789]*) prereq_msg port_unknown; return 1 ;; esac
  pq_port=$1
  while [ "${pq_port#0}" != "$pq_port" ]; do pq_port=${pq_port#0}; done
  [ -n "$pq_port" ] && [ "${#pq_port}" -le 5 ] && [ "$pq_port" -ge 1 ] && [ "$pq_port" -le 65535 ] || { prereq_msg port_unknown; return 1; }
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
      0) prereq_msg port_busy "$pq_port"; return 1 ;;
      1) return 0 ;;
      *) prereq_msg port_unknown; return 1 ;;
    esac
  fi
  pq_port_hex=$(printf '%04X' "$pq_port") || { prereq_msg port_unknown; return 1; }
  for pq_socket_file in "${PREREQ_ROOT:-}/proc/net/tcp" "${PREREQ_ROOT:-}/proc/net/tcp6"; do
    [ -r "$pq_socket_file" ] || { prereq_msg port_unknown; return 1; }
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
      0) prereq_msg port_busy "$pq_port"; return 1 ;;
      1) ;;
      *) prereq_msg port_unknown; return 1 ;;
    esac
  done
  return 0
}
