# mac/install-fetch.sh: the Mac COPY of deploy/install-fetch.sh (blob 27c39d81812771fcf9d8a530f68ad48dbb32813a at the Linux release head 530fbd1), kai row 7a79e791 / cox 3e7a28ef (the join step).
# THE ONLY DIFFERENCES, platform only (the table is in INSTALL-MAC-PIN-nemo-r3.md): the archive is cortex-V-macos.tar.gz and the server image cortex-server-V-aarch64.oci.tar (the Mac set holds NO native program);
# the closed member list is the Mac's list 3 (compose.yml, stack.env, db-roles.sh, cortex.toml.example, docker/compose.docker.yml, bin/docker-compose-render, bin/docker-compose-lint, initdb/): no cortex program, no systemd units, no Linux backup scripts;
# MEMBER_LIST is 3 in install.meta and members is 3 in manifest.json; the image check after a load is `image inspect` (Docker has no `image exists`).
# Release-fetch and trust functions for the joined installer (install lane 6c). Sourcing has no side effects. Functions only.
#
# STAGE 1: rel_fetch fills a stage that the CALLER reserved; rel_trust proves what is in it. STAGE 2: rel_unpack (a closed member-list 2, into a private
# candidate), rel_load (podman load, then publish and point current) and verify_release_dir (read-only). Stage 1 installs nothing; stage 2 never leaves
# a candidate behind on a failure, never replaces a release and never switches an existing current.
# The caller (install.sh, W1-85 step 3) reserves the stage with mktemp -d, name .cortex-install-stage.XXXXXXXX, mode 0700, in the nearest existing
# user-owned ancestor of the target home (W1-86), never creates the home before trust, and removes the stage on every exit.
#
# Inputs, never read from the release: rel_fetch reads the variables CORTEX_FROM (a directory) or CORTEX_BASE_URL (https only). rel_trust takes the TRUST
# ANCHOR AS ARGUMENTS and reads no variable for it (an environment must not decide whom to trust, W1-84): rel_trust VERSION STAGE KEYFILE PRINCIPAL FPR,
# the key file holding one line "<principal> ssh-ed25519 <key>" and FPR being the fingerprint written into install.sh. The real signing scheme and key
# are the CTO's decision (D56, C-57): the namespace and the key type below are the test scheme and change in one place.
# Exit codes: EX_FETCH (5) and EX_TRUST (4) from the caller, with the defaults shown. Every function runs its work in a subshell with a fixed PATH
# (hale F-1), a fixed locale and IFS, no inherited loader or interpreter variables and umask 077 (F-2); the caller's shell is not changed.
# rel_trust leaves REL_TRUSTED (the verified tuple: the stage, the version, name/size/hash of the five listed files and the install.meta values),
# read from the bytes it hashed and never re-learned from a file (W1-89 f). Tests point the rel_tool_* seams at fakes; production calls /usr/bin.

fetch_msg() {
    case ${1-} in
        trust_key)
            printf '%s\n' 'problem: the release signing key could not be confirmed' 'fix: check the signing fingerprint in the release instructions' >&2 ;;
        trust_signature)
            printf '%s\n' 'problem: the release checksum signature could not be verified' 'fix: obtain the signed release files from the trusted release location' >&2 ;;
        trust_checksum)
            printf '%s\n' 'problem: a release file is missing or does not match its checksum' 'fix: obtain a complete release from the trusted release location' >&2 ;;
        trust_stage)
            printf '%s\n' 'problem: the release version or the staging directory is not the one this run reserved' 'fix: run the installer again; it reserves a fresh staging directory every time' >&2 ;;
        fetch_download)
            printf '%s\n' 'problem: the release files could not be obtained' 'fix: check the release location and try again' >&2 ;;
        fetch_location)
            printf '%s\n' 'problem: no download location is configured' 'fix: use --from <dir> with the release files, or give --base-url an https location' >&2 ;;
        fetch_archive)
            printf '%s\n' 'problem: the release archive could not be safely unpacked' 'fix: obtain a complete release from the trusted release location' >&2 ;;
        fetch_home)
            printf '%s\n' 'problem: the install directory cannot be used' 'fix: give --home an absolute path (letters, digits and . _ / - only) under a directory you own' >&2 ;;
        fetch_present)
            printf '%s\n' 'problem: this release is already unpacked or another release is current' 'fix: use cortex upgrade to change releases, or remove the old release directory first' >&2 ;;
        fetch_verify)
            printf '%s\n' 'problem: the release directory could not be verified' 'fix: obtain a complete release and run the installer again' >&2 ;;
        fetch_image)
            printf '%s\n' 'problem: the server image could not be loaded' 'fix: check that rootless Podman is available and try again' >&2 ;;
        *) return 1 ;;
    esac
    return 0
}

# The tools the trust step needs, by absolute path. A test redefines these after sourcing; the environment cannot.
rel_tool_sha256sum() { /usr/bin/sha256sum "$@"; }   # portable: ok the Linux default of the seam: the Mac installer redefines it to Homebrew's gsha256sum (gnu_seams_arm) before any call
rel_tool_keygen() { /usr/bin/ssh-keygen "$@"; }
rel_tool_curl() { /usr/bin/curl "$@"; }
rel_tool_uid() { /usr/bin/id -u; }
# tar, podman and python3 read behaviour from their environment (vera V6S2-01): GNU tar takes TAR_OPTIONS as initial options (POSIXLY_CORRECT only changes where options may
# stand among operands, and every call here puts them first; it is NOT unset: bash run as sh sets it itself, and unsetting it would take the shell out of POSIX mode);
# podman can be pointed at a remote service, another configuration or another store (CONTAINER_HOST, CONTAINER_CONNECTION, CONTAINER_SSHKEY, CONTAINER_PASSPHRASE,
# CONTAINERS_CONF, CONTAINERS_CONF_OVERRIDE, CONTAINERS_REGISTRIES_CONF, CONTAINERS_STORAGE_CONF, PODMAN_CONNECTIONS_CONF, STORAGE_DRIVER, STORAGE_OPTS); python3 -I
# ignores every PYTHON* variable and the user site by itself. rel_env_clean removes them all, and the tar and podman seams REFUSE (status 1, the tool is not run) if one
# is still set when they are called. HOME, XDG_RUNTIME_DIR and the XDG directories stay: rootless podman needs them, and they select the user's own configuration and
# store, which is the same-account boundary of this model (the files /etc/containers and ~/.config/containers are read by podman and are not an environment input).
rel_tool_tar() { [ -z "${TAR_OPTIONS+x}" ] || return 1; /usr/bin/tar "$@"; }
rel_tool_podman() { [ -z "${CONTAINER_HOST+x}${CONTAINER_CONNECTION+x}${CONTAINER_SSHKEY+x}${CONTAINER_PASSPHRASE+x}${CONTAINERS_CONF+x}${CONTAINERS_CONF_OVERRIDE+x}${CONTAINERS_REGISTRIES_CONF+x}${CONTAINERS_STORAGE_CONF+x}${PODMAN_CONNECTIONS_CONF+x}${STORAGE_DRIVER+x}${STORAGE_OPTS+x}" ] || return 1; /usr/bin/podman "$@"; }
rel_tool_python() { "${MAC_PYTHON:-/usr/bin/python3}" -I "$@"; }   # the Mac installer names the python3 it judged usable (MAC_PYTHON): never the Command Line Tools stub
rel_tool_stat() { /usr/bin/stat "$@"; }
rel_tool_wc() { /usr/bin/wc "$@"; }
rel_tool_cat() { /usr/bin/cat "$@"; }
rel_tool_cp() { /usr/bin/cp "$@"; }
rel_tool_mkdir() { /usr/bin/mkdir "$@"; }
rel_tool_rm() { /usr/bin/rm "$@"; }
rel_tool_rmdir() { /usr/bin/rmdir "$@"; }
rel_tool_find() { /usr/bin/find "$@"; }
rel_tool_sort() { /usr/bin/sort "$@"; }
rel_tool_mktemp() { /usr/bin/mktemp "$@"; }
rel_tool_chmod() { /usr/bin/chmod "$@"; }
rel_tool_mv() { /usr/bin/mv "$@"; }
rel_tool_ln() { /usr/bin/ln "$@"; }

# Called FIRST inside every subshell function (never in the caller's shell): the fixed PATH of a merged-/usr Rocky 10, a fixed locale, a known IFS,
# globbing off, and none of the variables that make a child load somebody's code. rel_nl is the newline the functions split lines on.
rel_env_clean() {
    unset -f '.' ':' '[' alias bg bind break builtin caller cd command compgen complete compopt continue declare dirs disown echo enable eval exec exit export false fc fg getopts hash help history jobs kill let local logout mapfile popd printf pushd pwd read readarray readonly return set shift shopt source suspend test times trap true type typeset ulimit umask unalias unset wait 2>/dev/null || :
    PATH=/usr/bin:/usr/sbin
    export PATH
    hash -r 2>/dev/null || :
    LC_ALL=C
    export LC_ALL
    umask 077
    unset LD_PRELOAD LD_LIBRARY_PATH LD_AUDIT PYTHONPATH PYTHONHOME PYTHONSTARTUP BASH_ENV ENV CDPATH GCONV_PATH OPENSSL_CONF OPENSSL_MODULES OPENSSL_ENGINES 2>/dev/null || :
    unset TAR_OPTIONS CONTAINER_HOST CONTAINER_CONNECTION CONTAINER_SSHKEY CONTAINER_PASSPHRASE CONTAINERS_CONF CONTAINERS_CONF_OVERRIDE CONTAINERS_REGISTRIES_CONF CONTAINERS_STORAGE_CONF PODMAN_CONNECTIONS_CONF STORAGE_DRIVER STORAGE_OPTS 2>/dev/null || :
    rel_nl=$(printf '\nx')
    rel_nl=${rel_nl%x}
    rel_tab=$(printf '\tx')
    rel_tab=${rel_tab%x}
    IFS=" $rel_tab$rel_nl"
    set -f
}

# The engine's parse_version, and no looser rule: strict SemVer without build metadata, at most 64 bytes, not ending in .part, MAJOR.MINOR.PATCH
# without leading zeros (up to 19 digits), an optional -PRERELEASE of dot-separated identifiers (none empty, a numeric one without a leading zero).
# 0 for a version, else 1. The alphabets are spelled out: a range follows the locale.
rel_version_ok() (
    rel_env_clean
    rv=${1-}
    [ "${#rv}" -le 64 ] || return 1
    case $rv in ""|*.part|*[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789.-]*) return 1 ;; esac
    rv_core=${rv%%-*}
    rv_pre=
    if [ "$rv_core" != "$rv" ]; then
        rv_pre=${rv#*-}
        [ -n "$rv_pre" ] || return 1
    fi
    case $rv_core in .*|*.|*..*) return 1 ;; esac
    IFS=.
    set -- $rv_core
    [ "$#" -eq 3 ] || return 1
    for rv_n in "$1" "$2" "$3"; do
        case $rv_n in *[!0123456789]*|0?*) return 1 ;; esac
        [ "${#rv_n}" -le 19 ] || return 1
    done
    if [ -n "$rv_pre" ]; then
        case $rv_pre in .*|*.|*..*) return 1 ;; esac
        set -- $rv_pre
        for rv_n in "$@"; do
            case $rv_n in *[!0123456789]*|0) ;; 0*) return 1 ;; esac
        done
    fi
    return 0
)

# An image reference as the engine's stack.env reader takes it: at most 300 bytes of letters, digits and . _ : / @ -, name@sha256:<64 lower hex>,
# a name that is not empty and holds no second @.
rel_image_ok() (
    rel_env_clean
    ri=${1-}
    { [ -n "$ri" ] && [ "${#ri}" -le 300 ]; } || return 1
    case $ri in *[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._:/@-]*) return 1 ;; esac
    case $ri in *@sha256:*) ;; *) return 1 ;; esac
    ri_name=${ri%@sha256:*}
    ri_dig=${ri##*@sha256:}
    { [ -n "$ri_name" ] && [ "${#ri_dig}" -eq 64 ]; } || return 1
    case $ri_name in *@*) return 1 ;; esac
    case $ri_dig in *[!0123456789abcdef]*) return 1 ;; esac
    return 0
)

# The size limit of each release file, in bytes (the builder's own limits: install.meta 4 KiB, the listings 64 KiB).
rel_limit() (
    rel_env_clean
    case ${1-} in
        manifest.json|install.sh) printf '%s\n' 1048576 ;;
        install.meta) printf '%s\n' 4096 ;;
        SHA256SUMS|SHA256SUMS.sig) printf '%s\n' 65536 ;;
        *) printf '%s\n' 4294967296 ;;
    esac
)

# 0 only for the directory this run reserved: an absolute path of safe characters with no dot components, named .cortex-install-stage.<8 letters or
# digits>, a real directory (not a link), owned by the effective user, mode exactly 0700. It says nothing about what is inside.
rel_stage_ok() (
    rel_env_clean
    rs=${1-}
    case $rs in /*) ;; *) return 1 ;; esac
    case $rs in *[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._/@+-]*) return 1 ;; esac
    case $rs in */|*//*|*/./*|*/.|*/../*|*/..) return 1 ;; esac
    rs_base=${rs##*/}
    case $rs_base in .cortex-install-stage.????????) ;; *) return 1 ;; esac
    case ${rs_base#.cortex-install-stage.} in *[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789]*) return 1 ;; esac
    [ ! -L "$rs" ] || return 1
    [ -d "$rs" ] || return 1
    rs_me=$(rel_tool_uid) || return 1
    rs_info=$(rel_tool_stat -c '%u %a' -- "$rs") || return 1
    [ "$rs_info" = "$rs_me 700" ] || return 1
    return 0
)

# Removes the files rel_fetch may have written into the stage, by exact name. The directory is the caller's and stays.
rel_unstage() {
    for ru_name in "cortex-$2-macos.tar.gz" "cortex-server-$2-aarch64.oci.tar" install.sh manifest.json install.meta SHA256SUMS SHA256SUMS.sig; do
        rel_tool_rm -f -- "$1/$ru_name" || :
    done
}

# Fills the stage the caller reserved (empty, 0700, owned by the user, named as above) with the seven release files. No trust decision is made here and
# nothing is unpacked. A failure removes the files written, not the directory. Offline: CORTEX_FROM is a directory. Online: CORTEX_BASE_URL, https only.
rel_fetch() (
    rel_env_clean
    rel_version=${1-}
    rel_stage=${2-}
    rel_version_ok "$rel_version" || { fetch_msg fetch_download; return "${EX_FETCH:-5}"; }
    rel_stage_ok "$rel_stage" || { fetch_msg fetch_download; return "${EX_FETCH:-5}"; }
    rel_entries=$(rel_tool_find "$rel_stage" -mindepth 1 -maxdepth 1 -printf x) || { fetch_msg fetch_download; return "${EX_FETCH:-5}"; }
    [ -z "$rel_entries" ] || { fetch_msg fetch_download; return "${EX_FETCH:-5}"; }
    if [ -n "${CORTEX_FROM:-}" ]; then
        if [ -L "$CORTEX_FROM" ] || [ ! -d "$CORTEX_FROM" ]; then fetch_msg fetch_download; return "${EX_FETCH:-5}"; fi
    else
        rel_base=${CORTEX_BASE_URL:-}
        [ -n "$rel_base" ] || { fetch_msg fetch_location; return "${EX_FETCH:-5}"; }
        [ "${#rel_base}" -le 2048 ] || { fetch_msg fetch_download; return "${EX_FETCH:-5}"; }
        case $rel_base in https://*) ;; *) fetch_msg fetch_download; return "${EX_FETCH:-5}" ;; esac
        case $rel_base in *[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._~:/-]*|*..*) fetch_msg fetch_download; return "${EX_FETCH:-5}" ;; esac
        rel_host=${rel_base#https://}
        rel_host=${rel_host%%/*}
        [ -n "$rel_host" ] || { fetch_msg fetch_download; return "${EX_FETCH:-5}"; }
        rel_base=${rel_base%/}
    fi
    rel_keep=0
    trap '[ "$rel_keep" = 1 ] || rel_unstage "$rel_stage" "$rel_version"' 0
    for rel_name in "cortex-$rel_version-macos.tar.gz" "cortex-server-$rel_version-aarch64.oci.tar" install.sh manifest.json install.meta SHA256SUMS SHA256SUMS.sig; do
        rel_limit_bytes=$(rel_limit "$rel_name")
        if [ -n "${CORTEX_FROM:-}" ]; then
            rel_src=$CORTEX_FROM/$rel_name
            if [ -L "$rel_src" ] || [ ! -f "$rel_src" ] || [ ! -r "$rel_src" ]; then fetch_msg fetch_download; return "${EX_FETCH:-5}"; fi
            rel_size=$(rel_tool_wc -c < "$rel_src") || { fetch_msg fetch_download; return "${EX_FETCH:-5}"; }
            if [ "$rel_size" -gt "$rel_limit_bytes" ]; then fetch_msg fetch_download; return "${EX_FETCH:-5}"; fi
            rel_tool_cp -- "$rel_src" "$rel_stage/$rel_name" || { fetch_msg fetch_download; return "${EX_FETCH:-5}"; }
        else
            rel_blocks=$((rel_limit_bytes / 512 + 2))
            ( ulimit -f "$rel_blocks"; rel_tool_curl -q --fail --silent --show-error --location --proto '=https' --proto-redir '=https' --max-redirs 3 --speed-limit 1000000 --speed-time 30 --max-time 7200 --max-filesize "$rel_limit_bytes" --output "$rel_stage/$rel_name" "$rel_base/$rel_name" ) 2>/dev/null || { fetch_msg fetch_download; return "${EX_FETCH:-5}"; }
        fi
        if [ -L "$rel_stage/$rel_name" ] || [ ! -f "$rel_stage/$rel_name" ]; then fetch_msg fetch_download; return "${EX_FETCH:-5}"; fi
        rel_size=$(rel_tool_wc -c < "$rel_stage/$rel_name") || { fetch_msg fetch_download; return "${EX_FETCH:-5}"; }
        if [ "$rel_size" -gt "$rel_limit_bytes" ]; then fetch_msg fetch_download; return "${EX_FETCH:-5}"; fi
    done
    rel_keep=1
    return 0
)

# 0 only for a file with no NUL byte. A variable cannot hold one: the shell would drop it with a warning, and the bytes parsed would no longer be the bytes
# that were hashed or signed, so a file that holds one is refused before it is read into a variable.
rel_no_nul() (
    rel_env_clean
    rn_size=$(rel_tool_wc -c < "$1") || return 1
    { rn_text=$(rel_tool_cat -- "$1"; rn_rc=$?; printf x; exit "$rn_rc"); } 2>/dev/null || return 1
    [ "${#rn_text}" -eq "$((rn_size + 1))" ]
)

# Removes the work directory rel_trust_run made inside the stage, by exact names.
rel_trust_clean() {
    rel_tool_rm -f -- "$1/.trust-work/key.pub" "$1/.trust-work/signers" 2>/dev/null || :
    rel_tool_rmdir -- "$1/.trust-work" 2>/dev/null || :
}

# The proof, in a subshell. Order: the stage and the version; the signer inputs and the fingerprint of the key (against the one pinned by the caller);
# the signature of SHA256SUMS (read ONCE into a variable: the bytes verified are the bytes parsed); the listing in the builder's exact shape; the hash
# of each of the five files against the listing; install.meta (read once, hashed from the same bytes) as bounded data in the builder's exact shape.
# Success prints the verified tuple on stdout and nothing else; a refusal prints the two-line message on stderr and returns EX_TRUST.
rel_trust_run() (
    rel_env_clean
    rt_ver=${1-}
    rt_stage=${2-}
    rt_keyfile=${3-}
    rt_principal=${4-}
    rt_fpr=${5-}
    rel_version_ok "$rt_ver" || { fetch_msg trust_stage; return "${EX_TRUST:-4}"; }
    rel_stage_ok "$rt_stage" || { fetch_msg trust_stage; return "${EX_TRUST:-4}"; }
    rt_tar=cortex-$rt_ver-macos.tar.gz
    rt_img=cortex-server-$rt_ver-aarch64.oci.tar
    rt_listed="$rt_tar $rt_img install.sh manifest.json install.meta"
    rt_all="$rt_listed SHA256SUMS SHA256SUMS.sig"
    rt_entries=$(rel_tool_find "$rt_stage" -mindepth 1 -maxdepth 1 -printf x) || { fetch_msg trust_checksum; return "${EX_TRUST:-4}"; }
    [ "${#rt_entries}" -eq 7 ] || { fetch_msg trust_checksum; return "${EX_TRUST:-4}"; }
    for rt_f in $rt_all; do
        rt_p=$rt_stage/$rt_f
        if [ -L "$rt_p" ] || [ ! -f "$rt_p" ] || [ ! -r "$rt_p" ]; then fetch_msg trust_checksum; return "${EX_TRUST:-4}"; fi
        rt_links=$(rel_tool_stat -c '%h' -- "$rt_p") || { fetch_msg trust_checksum; return "${EX_TRUST:-4}"; }
        [ "$rt_links" = 1 ] || { fetch_msg trust_checksum; return "${EX_TRUST:-4}"; }
        rt_size=$(rel_tool_wc -c < "$rt_p") || { fetch_msg trust_checksum; return "${EX_TRUST:-4}"; }
        rt_max=$(rel_limit "$rt_f")
        if [ "$rt_size" -gt "$rt_max" ]; then fetch_msg trust_checksum; return "${EX_TRUST:-4}"; fi
    done
    { [ -n "$rt_principal" ] && [ "${#rt_principal}" -le 64 ]; } || { fetch_msg trust_key; return "${EX_TRUST:-4}"; }
    case $rt_principal in -*|*[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._@-]*) fetch_msg trust_key; return "${EX_TRUST:-4}" ;; esac
    case $rt_fpr in SHA256:*) ;; *) fetch_msg trust_key; return "${EX_TRUST:-4}" ;; esac
    rt_fpr_b=${rt_fpr#SHA256:}
    [ "${#rt_fpr_b}" -eq 43 ] || { fetch_msg trust_key; return "${EX_TRUST:-4}"; }
    case $rt_fpr_b in *[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/]*) fetch_msg trust_key; return "${EX_TRUST:-4}" ;; esac
    case $rt_keyfile in /*) ;; *) fetch_msg trust_key; return "${EX_TRUST:-4}" ;; esac
    if [ -L "$rt_keyfile" ] || [ ! -f "$rt_keyfile" ]; then fetch_msg trust_key; return "${EX_TRUST:-4}"; fi
    rt_ksize=$(rel_tool_wc -c < "$rt_keyfile") || { fetch_msg trust_key; return "${EX_TRUST:-4}"; }
    [ "$rt_ksize" -le 1024 ] || { fetch_msg trust_key; return "${EX_TRUST:-4}"; }
    rel_no_nul "$rt_keyfile" || { fetch_msg trust_key; return "${EX_TRUST:-4}"; }
    rt_ktext=$(rel_tool_cat -- "$rt_keyfile") || { fetch_msg trust_key; return "${EX_TRUST:-4}"; }
    set -- $rt_ktext
    [ "$#" -eq 3 ] || { fetch_msg trust_key; return "${EX_TRUST:-4}"; }
    [ "$rt_ktext" = "$1 $2 $3" ] || { fetch_msg trust_key; return "${EX_TRUST:-4}"; }
    [ "$1" = "$rt_principal" ] || { fetch_msg trust_key; return "${EX_TRUST:-4}"; }
    [ "$2" = ssh-ed25519 ] || { fetch_msg trust_key; return "${EX_TRUST:-4}"; }
    [ "${#3}" -eq 68 ] || { fetch_msg trust_key; return "${EX_TRUST:-4}"; }
    case $3 in AAAAC3NzaC1lZDI1NTE5AAAAI*) ;; *) fetch_msg trust_key; return "${EX_TRUST:-4}" ;; esac
    case $3 in *[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/]*) fetch_msg trust_key; return "${EX_TRUST:-4}" ;; esac
    rt_keyb64=$3
    rt_work=$rt_stage/.trust-work
    rel_tool_mkdir -- "$rt_work" || { fetch_msg trust_checksum; return "${EX_TRUST:-4}"; }
    trap 'rel_trust_clean "$rt_stage"' 0
    { printf 'ssh-ed25519 %s\n' "$rt_keyb64" > "$rt_work/key.pub"; } 2>/dev/null || { fetch_msg trust_key; return "${EX_TRUST:-4}"; }
    { printf '%s ssh-ed25519 %s\n' "$rt_principal" "$rt_keyb64" > "$rt_work/signers"; } 2>/dev/null || { fetch_msg trust_key; return "${EX_TRUST:-4}"; }
    rt_lf=$(rel_tool_keygen -l -f "$rt_work/key.pub") || { fetch_msg trust_key; return "${EX_TRUST:-4}"; }
    set -- $rt_lf
    { [ "${1-}" = 256 ] && [ "${2-}" = "$rt_fpr" ]; } || { fetch_msg trust_key; return "${EX_TRUST:-4}"; }
    rel_no_nul "$rt_stage/SHA256SUMS" || { fetch_msg trust_signature; return "${EX_TRUST:-4}"; }
    rt_sums=$(rel_tool_cat -- "$rt_stage/SHA256SUMS"; rt_rc=$?; printf x; exit "$rt_rc") || { fetch_msg trust_signature; return "${EX_TRUST:-4}"; }
    rt_sums=${rt_sums%x}
    rt_verdict=$(printf '%s' "$rt_sums" | rel_tool_keygen -Y verify -f "$rt_work/signers" -I "$rt_principal" -n cortex-release -s "$rt_stage/SHA256SUMS.sig" 2>&1) || { fetch_msg trust_signature; return "${EX_TRUST:-4}"; }
    rt_first=${rt_verdict%%"$rel_nl"*}
    [ "$rt_first" = "Good \"cortex-release\" signature for $rt_principal with ED25519 key $rt_fpr" ] || { fetch_msg trust_signature; return "${EX_TRUST:-4}"; }
    case $rt_sums in *"$rel_nl") ;; *) fetch_msg trust_checksum; return "${EX_TRUST:-4}" ;; esac
    rt_rest=$rt_sums
    rt_files=
    rt_meta=
    for rt_f in $rt_listed; do
        [ -n "$rt_rest" ] || { fetch_msg trust_checksum; return "${EX_TRUST:-4}"; }
        rt_line=${rt_rest%%"$rel_nl"*}
        rt_rest=${rt_rest#*"$rel_nl"}
        rt_want=${rt_line%% *}
        [ "${#rt_want}" -eq 64 ] || { fetch_msg trust_checksum; return "${EX_TRUST:-4}"; }
        case $rt_want in *[!0123456789abcdef]*) fetch_msg trust_checksum; return "${EX_TRUST:-4}" ;; esac
        [ "$rt_line" = "$rt_want  $rt_f" ] || { fetch_msg trust_checksum; return "${EX_TRUST:-4}"; }
        rt_p=$rt_stage/$rt_f
        if [ "$rt_f" = install.meta ]; then
            rel_no_nul "$rt_p" || { fetch_msg trust_checksum; return "${EX_TRUST:-4}"; }
            rt_meta=$(rel_tool_cat -- "$rt_p"; rt_rc=$?; printf x; exit "$rt_rc") || { fetch_msg trust_checksum; return "${EX_TRUST:-4}"; }
            rt_meta=${rt_meta%x}
            rt_got=$(printf '%s' "$rt_meta" | rel_tool_sha256sum) || { fetch_msg trust_checksum; return "${EX_TRUST:-4}"; }
            rt_size=${#rt_meta}
        else
            rt_got=$(rel_tool_sha256sum -- "$rt_p") || { fetch_msg trust_checksum; return "${EX_TRUST:-4}"; }
            rt_size=$(rel_tool_wc -c < "$rt_p") || { fetch_msg trust_checksum; return "${EX_TRUST:-4}"; }
        fi
        rt_got=${rt_got%% *}
        [ "$rt_got" = "$rt_want" ] || { fetch_msg trust_checksum; return "${EX_TRUST:-4}"; }
        rt_files="$rt_files${rel_nl}file $rt_f $rt_size $rt_got"
    done
    [ -z "$rt_rest" ] || { fetch_msg trust_checksum; return "${EX_TRUST:-4}"; }
    case $rt_meta in *"$rel_nl") ;; *) fetch_msg trust_checksum; return "${EX_TRUST:-4}" ;; esac
    rt_rest=$rt_meta
    rt_values=
    for rt_k in META_SCHEMA RELEASE_VERSION STACK_VERSION MEMBER_LIST CORTEX_SERVER_IMAGE CORTEX_DB_IMAGE ROLES_SCRIPT_DIGEST; do
        [ -n "$rt_rest" ] || { fetch_msg trust_checksum; return "${EX_TRUST:-4}"; }
        rt_line=${rt_rest%%"$rel_nl"*}
        rt_rest=${rt_rest#*"$rel_nl"}
        case $rt_line in "$rt_k"=*) ;; *) fetch_msg trust_checksum; return "${EX_TRUST:-4}" ;; esac
        rt_val=${rt_line#*=}
        { [ -n "$rt_val" ] && [ "${#rt_val}" -le 300 ]; } || { fetch_msg trust_checksum; return "${EX_TRUST:-4}"; }
        case $rt_val in *[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._:/@-]*) fetch_msg trust_checksum; return "${EX_TRUST:-4}" ;; esac
        case $rt_k in
            META_SCHEMA) [ "$rt_val" = 1 ] || { fetch_msg trust_checksum; return "${EX_TRUST:-4}"; } ;;
            RELEASE_VERSION|STACK_VERSION) [ "$rt_val" = "$rt_ver" ] || { fetch_msg trust_checksum; return "${EX_TRUST:-4}"; } ;;
            MEMBER_LIST) [ "$rt_val" = 3 ] || { fetch_msg trust_checksum; return "${EX_TRUST:-4}"; } ;;
            CORTEX_SERVER_IMAGE|CORTEX_DB_IMAGE) rel_image_ok "$rt_val" || { fetch_msg trust_checksum; return "${EX_TRUST:-4}"; } ;;
            ROLES_SCRIPT_DIGEST)
                [ "${#rt_val}" -eq 64 ] || { fetch_msg trust_checksum; return "${EX_TRUST:-4}"; }
                case $rt_val in *[!0123456789abcdef]*) fetch_msg trust_checksum; return "${EX_TRUST:-4}" ;; esac ;;
        esac
        if [ "$rt_k" != META_SCHEMA ]; then rt_values="$rt_values${rel_nl}meta $rt_k $rt_val"; fi
    done
    [ -z "$rt_rest" ] || { fetch_msg trust_checksum; return "${EX_TRUST:-4}"; }
    printf '%s\n' "v1${rel_nl}version $rt_ver${rel_nl}stage $rt_stage$rt_files$rt_values"
    return 0
)

# Unsets the temporaries of the wrappers that run in the caller's shell (rel_trust, and in stage 2 rel_unpack and rel_load) and returns the status it is
# given, so no wrapper leaves a name behind.
rel_finish() {
    unset rt_out ru_out rl_cand rl_rc
    return "${1:-1}"
}

# Proves the release in the stage (rel_fetch filled it), against the trust anchor given as the 3rd to 5th argument, and keeps the verified tuple in REL_TRUSTED. Runs in the caller's shell so the variable
# survives; every other name it uses is cleared. A failure leaves REL_TRUSTED empty and returns EX_TRUST with the message printed.
rel_trust() {
    REL_TRUSTED=
    rt_out=$(rel_trust_run "${1-}" "${2-}" "${3-}" "${4-}" "${5-}") || { rel_finish "$?"; return; }
    case $rt_out in v1*) ;; *) fetch_msg trust_checksum; rel_finish "${EX_TRUST:-4}"; return ;; esac
    REL_TRUSTED=$rt_out
    rel_finish 0
}

# The install directory as the engine's check_home takes it (absolute, letters, digits and . _ / - only, no empty, dot or dot-dot component), and not the root.
rel_home_ok() (
    rel_env_clean
    rh=${1-}
    case $rh in /?*) ;; *) return 1 ;; esac
    case $rh in *[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._/-]*) return 1 ;; esac
    case $rh in */|*//*|*/./*|*/.|*/../*|*/..) return 1 ;; esac
    return 0
)

# What the closed member list 2 allows for a name: "<type> <mode>" with the tar spelling of the mode, or nothing. Names are exact (the engine's unpacker).
rel_member_mode() (
    rel_env_clean
    case ${1-} in
        db-roles.sh|bin/docker-compose-render|bin/docker-compose-lint) printf '%s\n' '- rwxr-xr-x' ;;
        compose.yml|stack.env|cortex.toml.example|docker/compose.docker.yml) printf '%s\n' '- rw-r--r--' ;;
        initdb|bin|docker) printf '%s\n' 'd rwxr-xr-x' ;;
        initdb/?*)
            rm_name=${1#initdb/}
            [ "${#rm_name}" -le 64 ] || return 1
            case $rm_name in [ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789_-]*) ;; *) return 1 ;; esac
            case $rm_name in *[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._-]*) return 1 ;; esac
            printf '%s\n' '- rw-r--r--' ;;
        *) return 1 ;;
    esac
    return 0
)

# The rules rel_trust applied to an install.meta value, for the tuple: KEY VALUE VERSION. 0 or 1.
rel_meta_value_ok() (
    rel_env_clean
    rm_key=${1-}
    rm_val=${2-}
    rm_ver=${3-}
    { [ -n "$rm_val" ] && [ "${#rm_val}" -le 300 ]; } || return 1
    case $rm_val in *[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._:/@-]*) return 1 ;; esac
    case $rm_key in
        RELEASE_VERSION|STACK_VERSION) [ "$rm_val" = "$rm_ver" ] ;;
        MEMBER_LIST) [ "$rm_val" = 3 ] ;;
        CORTEX_SERVER_IMAGE|CORTEX_DB_IMAGE) rel_image_ok "$rm_val" ;;
        ROLES_SCRIPT_DIGEST)
            [ "${#rm_val}" -eq 64 ] || return 1
            case $rm_val in *[!0123456789abcdef]*) return 1 ;; esac
            return 0 ;;
        *) return 1 ;;
    esac
)

# The verified tuple (REL_TRUSTED) as rel_trust left it, checked again in full: v1, the version and the stage as asked, the five files in the builder's order
# with a size and a lower-case hex hash, the six install.meta values in order under the rules rel_trust applied. 0 for exactly that, else 1.
rel_tuple_ok() (
    rel_env_clean
    rk_ver=${1-}
    rk_stage=${2-}
    rk_rest=${REL_TRUSTED-}
    [ -n "$rk_rest" ] || return 1
    rk_rest=$rk_rest$rel_nl
    for rk_k in head1 head2 head3 f1 f2 f3 f4 f5 m1 m2 m3 m4 m5 m6; do
        [ -n "$rk_rest" ] || return 1
        rk_line=${rk_rest%%"$rel_nl"*}
        rk_rest=${rk_rest#*"$rel_nl"}
        case $rk_k in
            head1) [ "$rk_line" = v1 ] || return 1 ;;
            head2) [ "$rk_line" = "version $rk_ver" ] || return 1 ;;
            head3) [ "$rk_line" = "stage $rk_stage" ] || return 1 ;;
            f1|f2|f3|f4|f5)
                case $rk_k in
                    f1) rk_name=cortex-$rk_ver-macos.tar.gz ;;
                    f2) rk_name=cortex-server-$rk_ver-aarch64.oci.tar ;;
                    f3) rk_name=install.sh ;;
                    f4) rk_name=manifest.json ;;
                    f5) rk_name=install.meta ;;
                esac
                set -- $rk_line
                { [ "$#" -eq 4 ] && [ "$1" = file ] && [ "$2" = "$rk_name" ] && [ "$rk_line" = "$1 $2 $3 $4" ]; } || return 1
                case $3 in ""|*[!0123456789]*) return 1 ;; esac
                [ "${#3}" -le 19 ] || return 1
                [ "${#4}" -eq 64 ] || return 1
                case $4 in *[!0123456789abcdef]*) return 1 ;; esac ;;
            m1|m2|m3|m4|m5|m6)
                case $rk_k in
                    m1) rk_key=RELEASE_VERSION ;;
                    m2) rk_key=STACK_VERSION ;;
                    m3) rk_key=MEMBER_LIST ;;
                    m4) rk_key=CORTEX_SERVER_IMAGE ;;
                    m5) rk_key=CORTEX_DB_IMAGE ;;
                    m6) rk_key=ROLES_SCRIPT_DIGEST ;;
                esac
                set -- $rk_line
                { [ "$#" -eq 3 ] && [ "$1" = meta ] && [ "$2" = "$rk_key" ] && [ "$rk_line" = "$1 $2 $3" ]; } || return 1
                rel_meta_value_ok "$rk_key" "$3" "$rk_ver" || return 1 ;;
        esac
    done
    [ -z "$rk_rest" ]
)

# The size and hash of a file as the tuple holds them ("<size> <hash>"), or nothing. Call it only after rel_tuple_ok.
rel_tuple_file() (
    rel_env_clean
    IFS=$rel_nl
    for rk_line in ${REL_TRUSTED-}; do
        case $rk_line in "file ${1-} "*) printf '%s\n' "${rk_line#"file ${1-} "}"; return 0 ;; esac
    done
    return 1
)

# The value of an install.meta key as the tuple holds it, or nothing. Call it only after rel_tuple_ok.
rel_tuple_meta() (
    rel_env_clean
    IFS=$rel_nl
    for rk_line in ${REL_TRUSTED-}; do
        case $rk_line in "meta ${1-} "*) printf '%s\n' "${rk_line#"meta ${1-} "}"; return 0 ;; esac
    done
    return 1
)

# 0 only if the file NAME in the stage is, right now, the file the tuple describes: a regular file, not a link, its size and its sha256.
rel_file_is() (
    rel_env_clean
    rf_path=${1-}/${2-}
    rf_want=$(rel_tuple_file "${2-}") || return 1
    if [ -L "$rf_path" ] || [ ! -f "$rf_path" ] || [ ! -r "$rf_path" ]; then return 1; fi
    rf_size=$(rel_tool_wc -c < "$rf_path") || return 1
    [ "$rf_size" = "${rf_want%% *}" ] || return 1
    rf_got=$(rel_tool_sha256sum -- "$rf_path") || return 1
    [ "${rf_got%% *}" = "${rf_want#* }" ]
)

# The shell half of the engine's verify_release_dir, read-only: DIR is an absolute real directory (not a link) that holds exactly the member list 2 (names,
# types, modes), manifest.json and SHA256SUMS (0644, the builder's shape: 64 lower hex, two spaces, a name, one line per file, a final LF), every listed file
# hashes to its line and every file is listed, and stack.env is the builder's three lines (a SemVer and two pinned images). 0 for exactly that, else 1.
rel_check_dir() (
    rel_env_clean
    rc_dir=${1-}
    rel_home_ok "$rc_dir" || return 1
    if [ -L "$rc_dir" ] || [ ! -d "$rc_dir" ]; then return 1; fi
    rc_bad=$(rel_tool_find "$rc_dir" -mindepth 1 -path '*[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._/-]*' -printf x) || return 1
    [ -z "$rc_bad" ] || return 1
    rc_phys=$(rel_tool_find "$rc_dir" -mindepth 1 -printf x) || return 1
    rc_found=$(rel_tool_find "$rc_dir" -mindepth 1 -printf '%y %m %P\n') || return 1
    [ -n "$rc_found" ] || return 1
    rc_files=
    rc_dirs=
    rc_n=0
    IFS=$rel_nl
    for rc_line in $rc_found; do
        IFS=" "
        set -- $rc_line
        IFS=$rel_nl
        [ "$#" -ge 3 ] || return 1
        rc_type=$1
        rc_mode=$2
        shift 2
        rc_path=$*
        rc_n=$((rc_n + 1))
        case " $rc_files $rc_dirs " in *" $rc_path "*) return 1 ;; esac
        case $rc_path in manifest.json|SHA256SUMS) rc_want="- rw-r--r--" ;; *) rc_want=$(rel_member_mode "$rc_path") || return 1 ;; esac
        case $rc_type in
            d) [ "$rc_want $rc_mode" = "d rwxr-xr-x 755" ] || return 1; rc_dirs="$rc_dirs $rc_path" ;;
            f) [ "$rc_want $rc_mode" = "- rwxr-xr-x 755" ] || [ "$rc_want $rc_mode" = "- rw-r--r-- 644" ] || return 1; rc_files="$rc_files $rc_path" ;;
            *) return 1 ;;
        esac
    done
    [ "${#rc_phys}" -eq "$rc_n" ] || return 1
    IFS=" $rel_tab$rel_nl"
    for rc_need in initdb bin docker; do
        case " $rc_dirs " in *" $rc_need "*) ;; *) return 1 ;; esac
    done
    for rc_need in compose.yml stack.env db-roles.sh cortex.toml.example manifest.json SHA256SUMS docker/compose.docker.yml bin/docker-compose-render bin/docker-compose-lint; do
        case " $rc_files " in *" $rc_need "*) ;; *) return 1 ;; esac
    done
    rel_no_nul "$rc_dir/SHA256SUMS" || return 1
    rc_size=$(rel_tool_wc -c < "$rc_dir/SHA256SUMS") || return 1
    [ "$rc_size" -le 1048576 ] || return 1
    rc_sums=$(rel_tool_cat -- "$rc_dir/SHA256SUMS"; sc_rc=$?; printf x; exit "$sc_rc") || return 1
    rc_sums=${rc_sums%x}
    case $rc_sums in *"$rel_nl") ;; *) return 1 ;; esac
    rc_rest=$rc_sums
    rc_listed=
    while [ -n "$rc_rest" ]; do
        rc_line=${rc_rest%%"$rel_nl"*}
        rc_rest=${rc_rest#*"$rel_nl"}
        rc_hash=${rc_line%% *}
        [ "${#rc_hash}" -eq 64 ] || return 1
        case $rc_hash in *[!0123456789abcdef]*) return 1 ;; esac
        case $rc_line in "$rc_hash  "?*) ;; *) return 1 ;; esac
        rc_name=${rc_line#"$rc_hash  "}
        [ "$rc_name" != SHA256SUMS ] || return 1
        case " $rc_files " in *" $rc_name "*) ;; *) return 1 ;; esac
        case " $rc_listed " in *" $rc_name "*) return 1 ;; esac
        rc_listed="$rc_listed $rc_name"
        rc_got=$(rel_tool_sha256sum -- "$rc_dir/$rc_name") || return 1
        [ "${rc_got%% *}" = "$rc_hash" ] || return 1
    done
    for rc_name in $rc_files; do
        [ "$rc_name" = SHA256SUMS ] && continue
        case " $rc_listed " in *" $rc_name "*) ;; *) return 1 ;; esac
    done
    rc_size=$(rel_tool_wc -c < "$rc_dir/stack.env") || return 1
    [ "$rc_size" -le 8192 ] || return 1
    rel_no_nul "$rc_dir/stack.env" || return 1
    rc_env=$(rel_tool_cat -- "$rc_dir/stack.env"; sc_rc=$?; printf x; exit "$sc_rc") || return 1
    rc_env=${rc_env%x}
    case $rc_env in "CORTEX_VERSION="*"${rel_nl}CORTEX_SERVER_IMAGE="*"${rel_nl}CORTEX_DB_IMAGE="*"$rel_nl") ;; *) return 1 ;; esac
    rc_ver=${rc_env#CORTEX_VERSION=}
    rc_ver=${rc_ver%%"$rel_nl"*}
    rc_srv=${rc_env#*"${rel_nl}CORTEX_SERVER_IMAGE="}
    rc_srv=${rc_srv%%"$rel_nl"*}
    rc_db=${rc_env#*"${rel_nl}CORTEX_DB_IMAGE="}
    rc_db=${rc_db%%"$rel_nl"*}
    [ "$rc_env" = "CORTEX_VERSION=$rc_ver${rel_nl}CORTEX_SERVER_IMAGE=$rc_srv${rel_nl}CORTEX_DB_IMAGE=$rc_db$rel_nl" ] || return 1
    rel_version_ok "$rc_ver" || return 1
    rel_image_ok "$rc_srv" || return 1
    rel_image_ok "$rc_db" || return 1
    return 0
)

# The read-only verification the installer calls before it trusts a release directory (until the cortex binary can run the engine's own): rel_check_dir,
# then the manifest copy in the directory must hold a release of the stack.env version with member list 2 whose roles digest is db-roles.sh's.
# manifest.json is read by python3 -I (a bounded file); a missing python3 is a refusal: unknown is not success. It never repairs anything.
verify_release_dir() (
    rel_env_clean
    vd=${1-}
    rel_check_dir "$vd" || { fetch_msg fetch_verify; return "${EX_FETCH:-5}"; }
    vd_env=$(rel_tool_cat -- "$vd/stack.env") || { fetch_msg fetch_verify; return "${EX_FETCH:-5}"; }
    vd_ver=${vd_env#CORTEX_VERSION=}
    vd_ver=${vd_ver%%"$rel_nl"*}
    vd_roles=$(rel_tool_python -c 'import json, sys
d, v = sys.argv[1], sys.argv[2]
b = open(d + "/manifest.json", "rb").read(1048577)
if len(b) > 1048576:
    sys.exit(1)
m = json.loads(b.decode("utf-8"))
if m.get("schema") != 1:
    sys.exit(1)
r = [x for x in m["releases"] if x.get("version") == v]
if len(r) != 1:
    sys.exit(1)
c = r[0]["cortex"]
if c["members"] != 3:
    sys.exit(1)
g = c["roles_script_digest"]
if not g.startswith("sha256:"):
    sys.exit(1)
print(g[7:])' "$vd" "$vd_ver" 2>/dev/null) || { fetch_msg fetch_verify; return "${EX_FETCH:-5}"; }
    { [ "${#vd_roles}" -eq 64 ]; } || { fetch_msg fetch_verify; return "${EX_FETCH:-5}"; }
    case $vd_roles in *[!0123456789abcdef]*) fetch_msg fetch_verify; return "${EX_FETCH:-5}" ;; esac
    vd_got=$(rel_tool_sha256sum -- "$vd/db-roles.sh") || { fetch_msg fetch_verify; return "${EX_FETCH:-5}"; }
    [ "${vd_got%% *}" = "$vd_roles" ] || { fetch_msg fetch_verify; return "${EX_FETCH:-5}"; }
    return 0
)

# 0 only for a candidate directory this installer made: HOME/releases/.cortex-unpack.<8 letters or digits>, a real directory, owned by the user.
rel_cand_ok() (
    rel_env_clean
    rg_cand=${1-}
    rg_home=${2-}
    rel_home_ok "$rg_home" || return 1
    case $rg_cand in "$rg_home/releases/.cortex-unpack."????????) ;; *) return 1 ;; esac
    case ${rg_cand##*/.cortex-unpack.} in *[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789]*) return 1 ;; esac
    if [ -L "$rg_cand" ] || [ ! -d "$rg_cand" ]; then return 1; fi
    rg_me=$(rel_tool_uid) || return 1
    rg_owner=$(rel_tool_stat -c '%u' -- "$rg_cand") || return 1
    [ "$rg_owner" = "$rg_me" ]
)

# Removes a candidate directory, only if rel_cand_ok recognises it by name, place and owner (unknown means keep). A SUBSHELL that cleans its environment first (vera V6S2R1-01):
# rel_load calls it from the caller's shell after a failed load, and the native rm must not start with the caller's environment. Its status is the caller's to ignore.
rel_discard() (
    rel_env_clean
    if rel_cand_ok "${1-}" "${2-}"; then rel_tool_rm -rf -- "$1" || :; fi
)

# SCOPE of the candidate's checks (owner ruling CQ6S2-01): the SHA256SUMS this function generates INSIDE the candidate is LOCAL-CONSISTENCY evidence: it catches corruption
# and a half-changed tree between unpack and load. It is NOT protection against a coherent change by the SAME user in that window: the release's trust decision is the signed
# tuple at the stage, the candidate is private (0700) and the user's own account is the boundary of this model (W1-66, the R-4a limits).
# Unpacks the verified archive into a NEW private candidate under HOME/releases and prints its path (rel_unpack keeps it in REL_CANDIDATE). Order: the tuple
# and the three files used are checked against it; the home is checked and made (private, never an existing release overwritten); the archive's LISTING is read
# and must be the closed member list 2 (names, types, modes, no links, at most 256 entries, 1 GiB); only then is it extracted; the tree is checked again;
# stack.env and db-roles.sh must agree with the SIGNED install.meta; manifest.json and the generated SHA256SUMS are written; rel_check_dir verifies the result.
# Any failure removes the candidate. A leftover candidate of a lost run is never touched.
rel_unpack_run() (
    rel_env_clean
    ru_ver=${1-}
    ru_stage=${2-}
    ru_home=${3-}
    rel_version_ok "$ru_ver" || { fetch_msg trust_stage; return "${EX_TRUST:-4}"; }
    rel_stage_ok "$ru_stage" || { fetch_msg trust_stage; return "${EX_TRUST:-4}"; }
    rel_tuple_ok "$ru_ver" "$ru_stage" || { fetch_msg trust_checksum; return "${EX_TRUST:-4}"; }
    ru_tar=cortex-$ru_ver-macos.tar.gz
    rel_file_is "$ru_stage" "$ru_tar" || { fetch_msg trust_checksum; return "${EX_TRUST:-4}"; }
    rel_file_is "$ru_stage" manifest.json || { fetch_msg trust_checksum; return "${EX_TRUST:-4}"; }
    rel_home_ok "$ru_home" || { fetch_msg fetch_home; return "${EX_FETCH:-5}"; }
    ru_me=$(rel_tool_uid) || { fetch_msg fetch_home; return "${EX_FETCH:-5}"; }
    if [ -L "$ru_home" ] || { [ -e "$ru_home" ] && [ ! -d "$ru_home" ]; }; then fetch_msg fetch_home; return "${EX_FETCH:-5}"; fi
    if [ -d "$ru_home" ]; then
        ru_owner=$(rel_tool_stat -c '%u' -- "$ru_home") || { fetch_msg fetch_home; return "${EX_FETCH:-5}"; }
        [ "$ru_owner" = "$ru_me" ] || { fetch_msg fetch_home; return "${EX_FETCH:-5}"; }
    fi
    if [ -L "$ru_home/releases" ] || { [ -e "$ru_home/releases" ] && [ ! -d "$ru_home/releases" ]; }; then fetch_msg fetch_home; return "${EX_FETCH:-5}"; fi
    if [ -e "$ru_home/releases/$ru_ver" ] || [ -L "$ru_home/releases/$ru_ver" ]; then fetch_msg fetch_present; return "${EX_FETCH:-5}"; fi
    ru_listing=$(rel_tool_tar --numeric-owner -tvzf "$ru_stage/$ru_tar" 2>/dev/null) || { fetch_msg fetch_archive; return "${EX_FETCH:-5}"; }
    ru_count=0
    ru_total=0
    ru_seen=
    IFS=$rel_nl
    for ru_line in $ru_listing; do
        IFS=" "
        set -- $ru_line
        IFS=$rel_nl
        [ "$#" -ge 6 ] || { fetch_msg fetch_archive; return "${EX_FETCH:-5}"; }
        ru_perm=$1
        ru_size=$3
        shift 5
        ru_name=$*
        ru_name=${ru_name%/}
        ru_want=$(rel_member_mode "$ru_name") || { fetch_msg fetch_archive; return "${EX_FETCH:-5}"; }
        [ "$ru_want" = "${ru_perm%"${ru_perm#?}"} ${ru_perm#?}" ] || { fetch_msg fetch_archive; return "${EX_FETCH:-5}"; }
        case $ru_size in ""|*[!0123456789]*) fetch_msg fetch_archive; return "${EX_FETCH:-5}" ;; esac
        [ "${#ru_size}" -le 10 ] || { fetch_msg fetch_archive; return "${EX_FETCH:-5}"; }
        [ "$ru_size" -le 1073741824 ] || { fetch_msg fetch_archive; return "${EX_FETCH:-5}"; }
        ru_total=$((ru_total + ru_size))
        [ "$ru_total" -le 1073741824 ] || { fetch_msg fetch_archive; return "${EX_FETCH:-5}"; }
        ru_count=$((ru_count + 1))
        [ "$ru_count" -le 256 ] || { fetch_msg fetch_archive; return "${EX_FETCH:-5}"; }
        case " $ru_seen " in *" $ru_name "*) fetch_msg fetch_archive; return "${EX_FETCH:-5}" ;; esac
        ru_seen="$ru_seen $ru_name"
    done
    for ru_need in compose.yml stack.env db-roles.sh cortex.toml.example docker/compose.docker.yml bin/docker-compose-render bin/docker-compose-lint; do
        case " $ru_seen " in *" $ru_need "*) ;; *) fetch_msg fetch_archive; return "${EX_FETCH:-5}" ;; esac
    done
    case " $ru_seen " in *" initdb "*|*" initdb/"*) ;; *) fetch_msg fetch_archive; return "${EX_FETCH:-5}" ;; esac
    rel_tool_mkdir -p -- "$ru_home/releases" || { fetch_msg fetch_home; return "${EX_FETCH:-5}"; }
    ru_owner=$(rel_tool_stat -c '%u' -- "$ru_home/releases") || { fetch_msg fetch_home; return "${EX_FETCH:-5}"; }
    [ "$ru_owner" = "$ru_me" ] || { fetch_msg fetch_home; return "${EX_FETCH:-5}"; }
    ru_cand=$(rel_tool_mktemp -d -- "$ru_home/releases/.cortex-unpack.XXXXXXXX") || { fetch_msg fetch_archive; return "${EX_FETCH:-5}"; }
    ru_keep=0
    trap '[ "$ru_keep" = 1 ] || rel_discard "$ru_cand" "$ru_home"' 0
    rel_tool_tar --no-same-owner -p -xzf "$ru_stage/$ru_tar" -C "$ru_cand" 2>/dev/null || { fetch_msg fetch_archive; return "${EX_FETCH:-5}"; }
    for ru_dir in initdb bin docker; do
        [ ! -L "$ru_cand/$ru_dir" ] && [ -d "$ru_cand/$ru_dir" ] || { fetch_msg fetch_archive; return "${EX_FETCH:-5}"; }
        rel_tool_chmod 0755 -- "$ru_cand/$ru_dir" || { fetch_msg fetch_archive; return "${EX_FETCH:-5}"; }
    done
    ru_bad=$(rel_tool_find "$ru_cand" -mindepth 1 -path '*[!ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789._/-]*' -printf x) || { fetch_msg fetch_archive; return "${EX_FETCH:-5}"; }
    [ -z "$ru_bad" ] || { fetch_msg fetch_archive; return "${EX_FETCH:-5}"; }
    ru_phys=$(rel_tool_find "$ru_cand" -mindepth 1 -printf x) || { fetch_msg fetch_archive; return "${EX_FETCH:-5}"; }
    ru_found=$(rel_tool_find "$ru_cand" -mindepth 1 -printf '%y %m %P\n') || { fetch_msg fetch_archive; return "${EX_FETCH:-5}"; }
    ru_n=0
    IFS=$rel_nl
    for ru_line in $ru_found; do
        IFS=" "
        set -- $ru_line
        IFS=$rel_nl
        [ "$#" -ge 3 ] || { fetch_msg fetch_archive; return "${EX_FETCH:-5}"; }
        ru_type=$1
        ru_mode=$2
        shift 2
        ru_name=$*
        ru_n=$((ru_n + 1))
        ru_want=$(rel_member_mode "$ru_name") || { fetch_msg fetch_archive; return "${EX_FETCH:-5}"; }
        case "$ru_type $ru_want $ru_mode" in "d d rwxr-xr-x 755"|"f - rwxr-xr-x 755"|"f - rw-r--r-- 644") ;; *) fetch_msg fetch_archive; return "${EX_FETCH:-5}" ;; esac
        case " $ru_seen " in *" $ru_name "*) ;; *) case $ru_name in initdb|bin|docker) ;; *) fetch_msg fetch_archive; return "${EX_FETCH:-5}" ;; esac ;; esac
    done
    [ "${#ru_phys}" -eq "$ru_n" ] || { fetch_msg fetch_archive; return "${EX_FETCH:-5}"; }
    ru_env=$(rel_tool_cat -- "$ru_cand/stack.env"; sc_rc=$?; printf x; exit "$sc_rc") || { fetch_msg fetch_archive; return "${EX_FETCH:-5}"; }
    ru_env=${ru_env%x}
    ru_srv=$(rel_tuple_meta CORTEX_SERVER_IMAGE) || { fetch_msg trust_checksum; return "${EX_TRUST:-4}"; }
    ru_db=$(rel_tuple_meta CORTEX_DB_IMAGE) || { fetch_msg trust_checksum; return "${EX_TRUST:-4}"; }
    ru_roles=$(rel_tuple_meta ROLES_SCRIPT_DIGEST) || { fetch_msg trust_checksum; return "${EX_TRUST:-4}"; }
    [ "$ru_env" = "CORTEX_VERSION=$ru_ver${rel_nl}CORTEX_SERVER_IMAGE=$ru_srv${rel_nl}CORTEX_DB_IMAGE=$ru_db$rel_nl" ] || { fetch_msg trust_checksum; return "${EX_TRUST:-4}"; }
    ru_got=$(rel_tool_sha256sum -- "$ru_cand/db-roles.sh") || { fetch_msg trust_checksum; return "${EX_TRUST:-4}"; }
    [ "${ru_got%% *}" = "$ru_roles" ] || { fetch_msg trust_checksum; return "${EX_TRUST:-4}"; }
    rel_tool_cp -- "$ru_stage/manifest.json" "$ru_cand/manifest.json" || { fetch_msg fetch_archive; return "${EX_FETCH:-5}"; }
    rel_tool_chmod 0644 -- "$ru_cand/manifest.json" || { fetch_msg fetch_archive; return "${EX_FETCH:-5}"; }
    rel_file_is "$ru_cand" manifest.json || { fetch_msg trust_checksum; return "${EX_TRUST:-4}"; }
    ru_names=$(rel_tool_find "$ru_cand" -mindepth 1 -type f -printf '%P\n') || { fetch_msg fetch_archive; return "${EX_FETCH:-5}"; }
    ru_names=$(printf '%s\n' "$ru_names" | rel_tool_sort) || { fetch_msg fetch_archive; return "${EX_FETCH:-5}"; }
    ru_sums=
    IFS=$rel_nl
    for ru_name in $ru_names; do
        ru_got=$(rel_tool_sha256sum -- "$ru_cand/$ru_name") || { fetch_msg fetch_archive; return "${EX_FETCH:-5}"; }
        ru_sums="$ru_sums${ru_got%% *}  $ru_name$rel_nl"
    done
    printf '%s' "$ru_sums" > "$ru_cand/SHA256SUMS" || { fetch_msg fetch_archive; return "${EX_FETCH:-5}"; }
    rel_tool_chmod 0644 -- "$ru_cand/SHA256SUMS" || { fetch_msg fetch_archive; return "${EX_FETCH:-5}"; }
    rel_check_dir "$ru_cand" || { fetch_msg fetch_archive; return "${EX_FETCH:-5}"; }
    printf '%s\n' "$ru_cand"
    ru_keep=1
    return 0
)

# Unpacks the trusted archive into a private candidate (see rel_unpack_run) and keeps the candidate's path in REL_CANDIDATE for rel_load. Runs in the caller's
# shell so the variable survives. A failure leaves REL_CANDIDATE empty and returns EX_TRUST or EX_FETCH with the message printed.
rel_unpack() {
    REL_CANDIDATE=
    ru_out=$(rel_unpack_run "${1-}" "${2-}" "${3-}") || { rel_finish "$?"; return; }
    case $ru_out in /*) ;; *) fetch_msg fetch_archive; rel_finish "${EX_FETCH:-5}"; return ;; esac
    REL_CANDIDATE=$ru_out
    rel_finish 0
}

# Publishes the candidate rel_unpack made: the tuple and the candidate are checked again (a changed candidate or image is refused and the candidate removed),
# no current of any kind may exist and the release directory must not, the image is loaded with rootless podman and must then exist under its pinned name,
# and only then is the candidate renamed to HOME/releases/<version> and current made a link to it. Nothing is overwritten and no existing current is switched.
rel_load_run() (
    rel_env_clean
    rl_ver=${1-}
    rl_stage=${2-}
    rl_home=${3-}
    rl_cand=${4-}
    rel_version_ok "$rl_ver" || { fetch_msg trust_stage; return "${EX_TRUST:-4}"; }
    rel_stage_ok "$rl_stage" || { fetch_msg trust_stage; return "${EX_TRUST:-4}"; }
    rel_tuple_ok "$rl_ver" "$rl_stage" || { fetch_msg trust_checksum; return "${EX_TRUST:-4}"; }
    rel_cand_ok "$rl_cand" "$rl_home" || { fetch_msg fetch_archive; return "${EX_FETCH:-5}"; }
    rl_ok=0
    trap '[ "$rl_ok" = 1 ] || rel_discard "$rl_cand" "$rl_home"' 0
    if [ -e "$rl_home/current" ] || [ -L "$rl_home/current" ] || [ -e "$rl_home/releases/$rl_ver" ] || [ -L "$rl_home/releases/$rl_ver" ]; then fetch_msg fetch_present; return "${EX_FETCH:-5}"; fi
    rel_check_dir "$rl_cand" || { fetch_msg fetch_verify; return "${EX_FETCH:-5}"; }
    rl_image=cortex-server-$rl_ver-aarch64.oci.tar
    rel_file_is "$rl_stage" "$rl_image" || { fetch_msg trust_checksum; return "${EX_TRUST:-4}"; }
    rl_ref=$(rel_tuple_meta CORTEX_SERVER_IMAGE) || { fetch_msg trust_checksum; return "${EX_TRUST:-4}"; }
    rel_tool_podman load --input "$rl_stage/$rl_image" > /dev/null 2>&1 || { fetch_msg fetch_image; return "${EX_FETCH:-5}"; }
    rel_tool_podman image inspect "$rl_ref" > /dev/null 2>&1 || { fetch_msg fetch_image; return "${EX_FETCH:-5}"; }
    rel_tool_mv -T -- "$rl_cand" "$rl_home/releases/$rl_ver" || { fetch_msg fetch_present; return "${EX_FETCH:-5}"; }
    rl_ok=1
    rel_tool_ln -s -T -- "releases/$rl_ver" "$rl_home/current" || { fetch_msg fetch_present; return "${EX_FETCH:-5}"; }
    return 0
)

# Loads the image and publishes the release rel_unpack prepared (see rel_load_run); REL_CANDIDATE is consumed either way. Runs in the caller's shell.
rel_load() {
    rl_cand=${REL_CANDIDATE-}
    REL_CANDIDATE=
    rel_load_run "${1-}" "${2-}" "${3-}" "$rl_cand" && { rel_finish 0; return; }
    rl_rc=$?
    rel_discard "$rl_cand" "${3-}"
    rel_finish "$rl_rc"
}
