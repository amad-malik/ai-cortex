#!/bin/sh
# Build the MAC release set (0.2.2). No publication is performed here. This is the Mac recipe: a COPY-ADAPT of scripts/release.sh (hue, 15 Nov 2026; cox's ruling W1-281;
# release.sh and the Linux helper are not changed). WHAT DIFFERS FROM release.sh (scripts/mac-release-differences.txt lists every edit; scripts/mac-release-parity-check.py
# re-derives this file from release.sh by exactly those edits and fails on any other difference):
#   - no native program: there is no cargo build of the CLI; no systemd units and no backup scripts in the archive;
#   - no podman build: the arm64 server image is IMPORTED by its sha256 (--image-archive, --image-sha256), checked (image-check: one manifest, os linux, architecture arm64),
#     and BOUND to its source commit (--image-source-commit): every path the image build reads must be identical in that commit and in the head (image-binding);
#   - the archive is the Mac member list 3 (scripts/mac_release_manifest.py, which imports the Linux helper); the names are cortex-V-macos.tar.gz and cortex-server-V-aarch64.oci.tar;
#   - the installer is joined by the SAME join-installer from mac/install-mac.sh, mac/install-prereq.sh and mac/install-fetch.sh (copied once, digested, re-checked at the end);
#   - the notices and the licence (--notices, --license, each with its sha256) go BESIDE the signed list, as siblings of the release directory.
# Everything else is release.sh's: the arguments, the clean tree, the seams, the gates (CI, S5 on the layout, the roles proof, squawk), the staging directory and the checks at the end.
# Test-only command seams: CORTEX_RELEASE_CI, CORTEX_RELEASE_S5,
# CORTEX_RELEASE_PROOF, CORTEX_RELEASE_SQUAWK,
# CORTEX_RELEASE_GIT_SHA, CORTEX_RELEASE_CRATES_DIR and CORTEX_RELEASE_TOML.
# Real release runs do not set these variables, and a run that sets one is refused unless CORTEX_RELEASE_FIXTURE_ROOT names this very
# root (RB-3): a fixture says so, a releaser never does. An active seam is named on stderr and in install.meta (RELEASE_SEAMS=).
set -eu
# RB-11: bash (sh here) imports exported functions from the environment (BASH_FUNC_name%%); one named [ or echo would defeat or silence every
# guard below, so they are dropped first: the few the next line needs, then all the rest.
case ${BASH_VERSION-} in ?*) unset -f '[' test compgen builtin command type ;; esac
case ${BASH_VERSION-} in ?*) for release_fn in $(compgen -A function); do unset -f "$release_fn"; done ;; esac
# RB-4: an empty or relative PATH element resolves to the tree itself, so it is refused before any command is looked up by name. The
# three gates (ci, s5, proof) keep the caller's PATH (ci.sh needs cargo and its subcommands); every other tool is found on /usr/bin:/bin,
# and cargo and git are found ONCE, now, by absolute path.
case ":${PATH-}:" in
    *:[!/]*) echo 'mac-release: PATH has an empty or relative element' >&2; exit 2 ;;
esac
caller_path=$PATH
cargo_bin=$(command -v cargo || true)
git_bin=$(command -v git || true)
case $cargo_bin in /*) ;; *) cargo_bin= ;; esac
case $git_bin in /*) ;; *) git_bin= ;; esac
PATH=/usr/bin:/bin
export PATH
hash -r 2>/dev/null || :
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
seams=
for seam in CORTEX_RELEASE_CI CORTEX_RELEASE_S5 CORTEX_RELEASE_PROOF CORTEX_RELEASE_SQUAWK CORTEX_RELEASE_GIT_SHA CORTEX_RELEASE_CRATES_DIR CORTEX_RELEASE_TOML; do
    eval "seam_value=\${$seam:-}"
    [ -z "$seam_value" ] || seams=${seams:+$seams,}$seam
done
if [ -n "$seams" ]; then
    [ "${CORTEX_RELEASE_FIXTURE_ROOT:-}" = "$root" ] || { echo "mac-release: test seams are set outside a fixture ($seams); a real release sets none of them" >&2; exit 2; }
    echo "mac-release: TEST SEAMS ACTIVE (this is not a real release): $seams" >&2
fi
# One digest over the content of every file under a directory and the target text of every link (RB-2: what was checked is what ships, and nothing moved under the build).
# These are bounded, no-follow reads (the helper): a plain sha256sum of a path would follow a link and block on a FIFO placed there (cox's ruling on O-1, sam TAJ-SAM01's class).
# A digest that cannot be made prints its cause and is empty: it never equals a digest taken before, so the end comparisons refuse.
tree_digest() {
    python3 -I "$helper" tree-digest --dir "$1"
}
bin_members='docker-compose-render docker-compose-lint'
members_digest() {
    python3 -I "$helper" members-digest --dir "$1" $bin_members
}
[ "$#" -ge 1 ] || { echo 'mac-release: version required' >&2; exit 2; }
version=$1
shift
image_archive= image_sha256= image_source_commit= image_receipt= notices= notices_sha256= license_file= license_sha256=
notes_url="https://github.com/amad-malik/ai-cortex/releases/tag/v${version}-macos"
label=$version
trust_key_file=
trust_key_given=0
while [ "$#" -gt 0 ]; do
    case $1 in
        --trust-key-file)
            [ "$#" -ge 2 ] || { echo 'mac-release: --trust-key-file needs a value' >&2; exit 2; }
            [ "$trust_key_given" -eq 0 ] || { echo 'mac-release: --trust-key-file given twice' >&2; exit 2; }
            trust_key_given=1; trust_key_file=$2; shift 2 ;;
        --image-archive) [ "$#" -ge 2 ] || { echo 'mac-release: --image-archive needs a value' >&2; exit 2; }; [ -z "$image_archive" ] || { echo 'mac-release: --image-archive given twice' >&2; exit 2; }; image_archive=$2; shift 2 ;;
        --image-sha256) [ "$#" -ge 2 ] || { echo 'mac-release: --image-sha256 needs a value' >&2; exit 2; }; [ -z "$image_sha256" ] || { echo 'mac-release: --image-sha256 given twice' >&2; exit 2; }; image_sha256=$2; shift 2 ;;
        --image-source-commit) [ "$#" -ge 2 ] || { echo 'mac-release: --image-source-commit needs a value' >&2; exit 2; }; [ -z "$image_source_commit" ] || { echo 'mac-release: --image-source-commit given twice' >&2; exit 2; }; image_source_commit=$2; shift 2 ;;
        --image-receipt) [ "$#" -ge 2 ] || { echo 'mac-release: --image-receipt needs a value' >&2; exit 2; }; [ -z "$image_receipt" ] || { echo 'mac-release: --image-receipt given twice' >&2; exit 2; }; image_receipt=$2; shift 2 ;;
        --notices) [ "$#" -ge 2 ] || { echo 'mac-release: --notices needs a value' >&2; exit 2; }; [ -z "$notices" ] || { echo 'mac-release: --notices given twice' >&2; exit 2; }; notices=$2; shift 2 ;;
        --notices-sha256) [ "$#" -ge 2 ] || { echo 'mac-release: --notices-sha256 needs a value' >&2; exit 2; }; [ -z "$notices_sha256" ] || { echo 'mac-release: --notices-sha256 given twice' >&2; exit 2; }; notices_sha256=$2; shift 2 ;;
        --license) [ "$#" -ge 2 ] || { echo 'mac-release: --license needs a value' >&2; exit 2; }; [ -z "$license_file" ] || { echo 'mac-release: --license given twice' >&2; exit 2; }; license_file=$2; shift 2 ;;
        --license-sha256) [ "$#" -ge 2 ] || { echo 'mac-release: --license-sha256 needs a value' >&2; exit 2; }; [ -z "$license_sha256" ] || { echo 'mac-release: --license-sha256 given twice' >&2; exit 2; }; license_sha256=$2; shift 2 ;;
        --notes-url) [ "$#" -ge 2 ] || { echo 'mac-release: missing notes URL' >&2; exit 2; }; notes_url=$2; shift 2 ;;
        --label) [ "$#" -ge 2 ] || { echo 'mac-release: missing label' >&2; exit 2; }; label=$2; shift 2 ;;
        *) echo 'mac-release: invalid option' >&2; exit 2 ;;
    esac
done
# The approved public key file is an ARGUMENT only: no environment variable stands in for it (W1-84). Its path is absolute and has no newline.
if [ "$trust_key_given" -eq 1 ]; then
    nl=$(printf '\nx'); nl=${nl%x}
    case $trust_key_file in
        /*) ;;
        *) echo 'mac-release: --trust-key-file must be an absolute path without a newline' >&2; exit 2 ;;
    esac
    case $trust_key_file in
        *"$nl"*) echo 'mac-release: --trust-key-file must be an absolute path without a newline' >&2; exit 2 ;;
    esac
fi
valid_version() {
    python3 -I - "$1" <<'PY'
import re, sys
v = sys.argv[1]
m = re.fullmatch(r'(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?', v, re.ASCII)
if len(v.encode()) > 64 or v.endswith('.part') or not m:
    raise SystemExit(1)
if m.group(4) and any(p.isdigit() and len(p) > 1 and p[0] == '0' for p in m.group(4).split('.')):
    raise SystemExit(1)
PY
}
valid_version "$version" || { echo 'mac-release: invalid version' >&2; exit 2; }
# the IMPORTED server image (the Mac recipe never builds it): named, hashed and bound to a source commit
mnl=$(printf '\nx'); mnl=${mnl%x}
[ -n "$image_archive" ] && [ -n "$image_sha256" ] && [ -n "$image_source_commit" ] || { echo 'mac-release: --image-archive, --image-sha256 and --image-source-commit are required' >&2; exit 2; }
case $image_archive in /*) ;; *) echo 'mac-release: --image-archive must be an absolute path without a newline' >&2; exit 2 ;; esac
case $image_archive in *"$mnl"*) echo 'mac-release: --image-archive must be an absolute path without a newline' >&2; exit 2 ;; esac
case $image_sha256 in *[!0-9a-f]*|'') echo 'mac-release: --image-sha256 must be 64 lowercase hex' >&2; exit 2 ;; esac
[ "${#image_sha256}" -eq 64 ] || { echo 'mac-release: --image-sha256 must be 64 lowercase hex' >&2; exit 2; }
case $image_source_commit in *[!0-9a-f]*|'') echo 'mac-release: --image-source-commit must be 40 lowercase hex' >&2; exit 2 ;; esac
[ "${#image_source_commit}" -eq 40 ] || { echo 'mac-release: --image-source-commit must be 40 lowercase hex' >&2; exit 2; }
case $image_receipt in /*) ;; *) echo 'mac-release: --image-receipt must name an absolute receipt directory' >&2; exit 2 ;; esac
case $image_receipt in *"$mnl"*) echo 'mac-release: --image-receipt must not contain a newline' >&2; exit 2 ;; esac
# the notices and the licence go beside the signed list: each is a file and its sha256 together, or neither
for beside_pair in "notices:$notices:$notices_sha256" "license:$license_file:$license_sha256"; do
    beside_rest=${beside_pair#*:}; beside_file=${beside_rest%%:*}; beside_sha=${beside_rest#*:}
    if [ -n "$beside_file" ] || [ -n "$beside_sha" ]; then
        [ -n "$beside_file" ] && [ -n "$beside_sha" ] || { echo "mac-release: --${beside_pair%%:*} and its sha256 are given together" >&2; exit 2; }
        case $beside_file in /*) ;; *) echo "mac-release: --${beside_pair%%:*} must be an absolute path without a newline" >&2; exit 2 ;; esac
        case $beside_file in *"$mnl"*) echo "mac-release: --${beside_pair%%:*} must be an absolute path without a newline" >&2; exit 2 ;; esac
        case $beside_sha in *[!0-9a-f]*) echo "mac-release: --${beside_pair%%:*}-sha256 must be 64 lowercase hex" >&2; exit 2 ;; esac
        [ "${#beside_sha}" -eq 64 ] || { echo "mac-release: --${beside_pair%%:*}-sha256 must be 64 lowercase hex" >&2; exit 2; }
    fi
done
python3 -I - "$notes_url" "$label" <<'PYINPUT' || { echo 'mac-release: invalid notes URL or label' >&2; exit 2; }
import re, sys
url, label = sys.argv[1:]
if (not url.startswith('https://') or len(url.encode()) > 2000
        or any(ord(c) < 33 or ord(c) > 126 or c in '"\\<>`' for c in url)):
    raise SystemExit(1)
if not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9._-]{0,63}', label, re.ASCII):
    raise SystemExit(1)
PYINPUT
# RB-1: the release is labelled with a git commit, so it is built only from a tree that IS that commit (a run that names its SHA is a test).
git_checked=0 start_tree= object_format=
if [ -z "${CORTEX_RELEASE_GIT_SHA:-}" ]; then
    git_checked=1
    [ "$trust_key_given" -eq 1 ] || { echo 'mac-release: --trust-key-file is required for a real release' >&2; exit 2; }
    [ -n "$notices" ] && [ -n "$license_file" ] || { echo 'mac-release: --notices and --license (each with its sha256) are required for a real release' >&2; exit 2; }
    [ -n "$git_bin" ] || { echo 'mac-release: git not found on PATH' >&2; exit 1; }
    git_env() { env -i PATH=/usr/bin:/bin GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null "$git_bin" -C "$root" "$@"; }
    toplevel=$(git_env rev-parse --show-toplevel 2>/dev/null) || { echo 'mac-release: this directory is not in a git work tree' >&2; exit 1; }
    [ "$(CDPATH= cd -- "$toplevel" && pwd -P)" = "$(CDPATH= cd -- "$root" && pwd -P)" ] || { echo 'mac-release: this directory is not the top of its own git work tree' >&2; exit 1; }
    # RB-12: the same check runs three times (at the start, right after the layout copy, just before the move), so a change made while a gate or
    # the image build runs is caught at the next one; the HEAD is read once here and compared at the end; an IGNORED file under deploy/ is copied
    # into the layout without showing in git status, so it refuses too (ignored files elsewhere are not covered).
    tree_clean() {
        changes=$(git_env status --porcelain --untracked-files=all -- . ':(exclude)target' ':(exclude)dist' ':(exclude)tools/squawk/2.71.0/squawk-linux-musl-x64') || { echo 'mac-release: git status failed' >&2; exit 1; }
        [ -z "$changes" ] || { echo "mac-release: the work tree is not clean ($1); commit or remove every change before a release" >&2; exit 1; }
        ignored=$(git_env ls-files --others --ignored --exclude-standard -- deploy) || { echo 'mac-release: git ls-files failed' >&2; exit 1; }
        [ -z "$ignored" ] || { echo "mac-release: an ignored file under deploy/ would be shipped ($1): $(printf '%s\n' "$ignored" | head -n 1)" >&2; exit 1; }
    }
    tree_clean "at the start"
    start_head=$(git_env rev-parse HEAD) || { echo 'mac-release: cannot read the git HEAD' >&2; exit 1; }
    start_tree=$(git_env rev-parse 'HEAD^{tree}') || { echo 'mac-release: cannot read the git tree' >&2; exit 1; }
    object_format=$(git_env rev-parse --show-object-format) || { echo 'mac-release: cannot read the git object format' >&2; exit 1; }
fi
[ ! -e "dist/$version" ] && [ ! -L "dist/$version" ] || { echo 'mac-release: destination exists' >&2; exit 1; }
[ ! -e "dist/$version.part" ] && [ ! -L "dist/$version.part" ] || { echo 'mac-release: staging destination exists' >&2; exit 1; }
[ ! -e "dist/$version.install.sh.binding.json" ] && [ ! -L "dist/$version.install.sh.binding.json" ] || { echo 'mac-release: binding record destination exists' >&2; exit 1; }
[ ! -e "dist/$version.image-binding.json" ] && [ ! -L "dist/$version.image-binding.json" ] || { echo 'mac-release: image binding record destination exists' >&2; exit 1; }
[ ! -e "dist/$version.beside" ] && [ ! -L "dist/$version.beside" ] || { echo 'mac-release: the destination for the notices and the licence exists' >&2; exit 1; }
mkdir -p target
work=$(mktemp -d "$root/target/release-work.XXXXXXXX")
TMPDIR=$work
export TMPDIR
part_created=0
part="$root/dist/$version.part"
cleanup() {
    if [ "$part_created" -eq 1 ]; then rm -rf -- "$part"; fi
    rm -rf -- "$work"
}
trap cleanup EXIT
trap 'exit 1' HUP INT TERM
# The workspace version is the single release-version source (W1-52).
[ -n "$cargo_bin" ] || { echo 'mac-release: cargo not found on PATH' >&2; exit 1; }
"$cargo_bin" metadata --locked --no-deps --format-version 1 > "$work/metadata.json" || { echo 'mac-release: cargo metadata failed' >&2; exit 1; }
python3 -I - "$root" "$version" "$work/metadata.json" <<'PYVERSION' || { echo 'mac-release: workspace version check failed' >&2; exit 1; }
import json, pathlib, sys, tomllib
root, requested, metadata_path = sys.argv[1:]
root = pathlib.Path(root).resolve()
try:
    metadata = json.loads(pathlib.Path(metadata_path).read_text(encoding='utf-8'))
    workspace = tomllib.loads((root / 'Cargo.toml').read_text(encoding='utf-8'))
    version = workspace['workspace']['package']['version']
    if requested != version or version == '0.1.0':
        raise ValueError('requested version differs from workspace or is the development version')
    if pathlib.Path(metadata['workspace_root']).resolve() != root:
        raise ValueError('metadata workspace root differs')
    members = set(metadata['workspace_members'])
    packages = [p for p in metadata['packages'] if p['id'] in members]
    if not members or {p['id'] for p in packages} != members:
        raise ValueError('workspace members are missing from metadata')
    for package in packages:
        manifest = tomllib.loads(pathlib.Path(package['manifest_path']).read_text(encoding='utf-8'))
        if manifest['package']['version'] != {'workspace': True} or package['version'] != version:
            raise ValueError('workspace package does not inherit the release version')
except (OSError, ValueError, KeyError, TypeError, tomllib.TOMLDecodeError) as error:
    raise SystemExit('mac-release: ' + str(error))
PYVERSION
ci=${CORTEX_RELEASE_CI:-"$root/scripts/ci.sh"}
s5=${CORTEX_RELEASE_S5:-"$root/scripts/s5-check.sh"}
squawk=${CORTEX_RELEASE_SQUAWK:-"$root/tools/squawk/2.71.0/squawk-linux-musl-x64"}
helper="$root/scripts/release_manifest.py"
mhelper="$root/scripts/mac_release_manifest.py"
feature_digest=$(python3 -I "$helper" file-digest --path "$root/deploy/server-features") || { echo 'mac-release: missing server feature file' >&2; exit 1; }
python3 -I "$helper" server-features "$root/deploy/server-features" >/dev/null || { echo 'mac-release: invalid server feature file' >&2; exit 1; }
[ "$feature_digest" = "$(python3 -I "$helper" file-digest --path "$root/deploy/server-features")" ] || { echo 'mac-release: server feature file changed during validation' >&2; exit 1; }
# An exception is never silently ignored; every local integrity pin must pass.
exception_file="$root/scripts/release-migration-exceptions.json"
# The list is REQUIRED (vera's answer (d), sam EXC-SAM01, cox W1-107): missing or invalid data is a failure, never an implicit empty configuration. Only an explicit
# schema-2 empty list with its exact zero-entry record means no waivers, and that pair is verified below like any other. Refused before the CI gate.
if [ ! -e "$exception_file" ] && [ ! -L "$exception_file" ]; then
    echo 'mac-release: the migration exception list is missing: a release needs the reviewed list (an explicit empty list means no waivers)' >&2
    exit 1
fi
exception_enabled=0
if [ -e "$exception_file" ] || [ -L "$exception_file" ]; then
    python3 -I "$helper" verify-approval --root "$root" || { echo 'mac-release: migration exception approval is not verified' >&2; exit 1; }
    exception_enabled=1
fi
# RB-2 for the Mac inputs: mac/ is copied ONCE and must hold exactly the three installer files; the join reads copies named as the joiner expects; the notices and the
# licence are read once, checked against their sha256 and copied. One recheck at the end refuses a live change made under the release.
mkdir "$work/mac"
cp -R "$root/mac/." "$work/mac/" || { echo 'mac-release: cannot copy mac/' >&2; exit 1; }
mac_digest=$(tree_digest "$work/mac") || exit 1
[ "$(ls -A "$work/mac" | sort | tr '\n' ' ')" = 'install-fetch.sh install-mac.sh install-prereq.sh ' ] || { echo 'mac-release: mac/ must hold exactly install-mac.sh, install-prereq.sh and install-fetch.sh' >&2; exit 1; }
mkdir "$work/joinlayout"
cp -p -- "$work/mac/install-mac.sh" "$work/joinlayout/install.sh"
cp -p -- "$work/mac/install-prereq.sh" "$work/joinlayout/install-prereq.sh"
cp -p -- "$work/mac/install-fetch.sh" "$work/joinlayout/install-fetch.sh"
mkdir "$work/beside"
if [ -n "$notices" ]; then
    [ "$(python3 -I "$helper" file-digest --path "$notices")" = "$notices_sha256" ] || { echo 'mac-release: the notices file is not the one named (sha256)' >&2; exit 1; }
    cp -p -- "$notices" "$work/beside/THIRD-PARTY-NOTICES.txt"
    [ "$(python3 -I "$helper" file-digest --path "$work/beside/THIRD-PARTY-NOTICES.txt")" = "$notices_sha256" ] || { echo 'mac-release: the copied notices file differs' >&2; exit 1; }
fi
if [ -n "$license_file" ]; then
    [ "$(python3 -I "$helper" file-digest --path "$license_file")" = "$license_sha256" ] || { echo 'mac-release: the licence file is not the one named (sha256)' >&2; exit 1; }
    cp -p -- "$license_file" "$work/beside/LICENSE"
    [ "$(python3 -I "$helper" file-digest --path "$work/beside/LICENSE")" = "$license_sha256" ] || { echo 'mac-release: the copied licence file differs' >&2; exit 1; }
fi
# The anchor join (contract G12-BINDING-RECORD-SCHEMA-nix-r4.md): the key file, the fingerprint tool and the installer template are validated NOW, with nothing written,
# so a bad key or template fails before the first gate and not hours later at the join. The hash of the one buffer it read is kept for the two re-reads below.
key_start_hash=
if [ "$trust_key_given" -eq 1 ]; then
    key_start_hash=$(python3 -I "$helper" check-anchor --deploy "$work/joinlayout" --trust-key-file "$trust_key_file") || exit 1
fi
# Gate 1: CI must run without the text-only option.
PATH=$caller_path "$ci" > "$work/ci.log" 2>&1 || { echo 'mac-release: CI failed' >&2; exit 1; }
python3 -I "$root/scripts/release-ci-check.py" "$work/ci.log" "$root/scripts/release-ci-allowlist.json" || { echo 'mac-release: CI results are not approved' >&2; exit 1; }
# Gate 2: check the generated release layout, before a build is started.
mkdir "$work/layout"
cp -R "$root/deploy/." "$work/layout/"
# RB-2: what ships is this copy, taken once: S5 reads it, and the installer join, the db image, the toml and the archive read it (never the
# live tree again). The five bin scripts are copied now too. One recheck at the end refuses a live change made under the build.
deploy_digest=$(tree_digest "$work/layout") || exit 1
mkdir "$work/scripts"
for member in $bin_members; do
    cp -p -- "$root/scripts/$member" "$work/scripts/$member" || { echo "mac-release: bin script missing: $member" >&2; exit 1; }
done
scripts_digest=$(members_digest "$work/scripts") || exit 1
# X-1: the S5 checker (S5-23) reads crates/cortex-server/Cargo.toml beside the layout; the layout is not in the tree, so it is copied there.
[ -f "$root/crates/cortex-server/Cargo.toml" ] && [ ! -L "$root/crates/cortex-server/Cargo.toml" ] || { echo 'mac-release: crates/cortex-server/Cargo.toml is missing (the S5 gate reads it beside the layout)' >&2; exit 1; }
mkdir -p "$work/crates/cortex-server"
cp -p -- "$root/crates/cortex-server/Cargo.toml" "$work/crates/cortex-server/Cargo.toml"
[ "$git_checked" -eq 0 ] || tree_clean "after the CI gate"
python3 -I - "$work/layout/compose.yml" <<'PY'
import re, sys
p = sys.argv[1]
lines = open(p, encoding='utf-8').readlines()
out = []
service = None
skip_build = False
changed = False
services = set()
for line in lines:
    m = re.match(r'^  ([A-Za-z0-9_-]+):\s*(?:#.*)?$', line.rstrip('\n'))
    if m:
        service = m.group(1)
    if service in ('server', 'migrate'):
        if re.match(r'^    build:\s*', line):
            skip_build = True
            changed = changed or service == 'server'
            continue
        if skip_build:
            if line.strip() and len(line) - len(line.lstrip(' ')) <= 4:
                skip_build = False
            else:
                continue
        if re.match(r'^    image:\s*', line):
            continue
        if m and m.group(1) in ('server', 'migrate'):
            if service in services:
                raise SystemExit('mac-release: duplicate service definition')
            services.add(service)
            out.append(line)
            out.append('    image: ${CORTEX_SERVER_IMAGE:?}\n')
            continue
    out.append(line)
if not changed or services != {'server', 'migrate'}:
    raise SystemExit('mac-release: required server or migrate definition not found')
open(p, 'w', encoding='utf-8').writelines(out)
PY
# The verifier refuses a db-roles.sh or compose.yml over its cap, so the builder refuses them first (the bytes that ship, before S5, squawk and the image build).
python3 -I "$helper" check-staged-sizes --layout "$work/layout"
PATH=$caller_path "$s5" "$work/layout" || { echo 'mac-release: S5 check failed' >&2; exit 1; }
# Gate 3: consume the CI proof verdict; an explicit seam supplies the verdict in tests.
if [ -n "${CORTEX_RELEASE_PROOF:-}" ]; then
    proof=$(PATH=$caller_path "$CORTEX_RELEASE_PROOF") || { echo 'mac-release: roles proof failed' >&2; exit 1; }
else
    proof=$(sed -n 's/^roles-proof[[:space:]]\{1,\}NOT_RUN[[:space:]]\{1,\}needs a database: .*\. \(last live run matches the current scripts (roles-proof: [1-9][0-9]* passed, 0 failed)\)$/\1/p' "$work/ci.log")
fi
case $proof in
    'last live run matches the current scripts'|'last live run matches the current scripts ('*')') ;;
    *) echo 'mac-release: current live roles proof required' >&2; exit 1 ;;
esac
# Gate 4: every forward migration must pass squawk.
[ -x "$squawk" ] || { echo 'mac-release: squawk not found' >&2; exit 1; }
if [ -z "${CORTEX_RELEASE_SQUAWK:-}" ]; then
    # the same bounded, no-follow read as every other digest (cox O-1, the class of TAJ-SAM01): a plain sha256sum of this path followed a link and blocked on a FIFO that
    # passes [ -x ] with the execute bit, and the path is excluded from the clean checks. A digest that cannot be made prints its cause and refuses.
    actual=$(python3 -I "$helper" file-digest --path "$squawk") || { echo 'mac-release: squawk digest mismatch' >&2; exit 1; }
    [ "$actual" = cf542e159fb3a94e5fd31d8b5282a31334df8205c6d7993afe286e9133fdff3e ] || { echo 'mac-release: squawk digest mismatch' >&2; exit 1; }
fi
mkdir "$work/squawk-home"
python3 -I "$helper" migration-files "${CORTEX_RELEASE_CRATES_DIR:-$root/crates}" > "$work/migrations"
while IFS= read -r migration; do
    # First releases are expand-only. Fail closed on contract SQL even if squawk exits 0.
    allow_dynamic=0
    exception_mode=
    if [ "$exception_enabled" -eq 1 ] && exception_mode=$(python3 -I "$helper" exception-candidate --root "$root" --migration "$migration" 2>/dev/null); then
        # the waiver is set only for the three words the helper may print (hale X-1): any other answer, even with status 0, waives nothing
        case $exception_mode in
            baseline|clean|findings) allow_dynamic=1 ;;
            *) exception_mode= ;;
        esac
    else
        exception_mode=
    fi
    python3 -I "$helper" scan-migration --migration "$migration" --allow-dynamic "$allow_dynamic" || exit 1
    if [ -n "${CORTEX_RELEASE_SQUAWK:-}" ]; then
        if env -i PATH=/usr/bin:/bin HOME="$work/squawk-home" TMPDIR="$work" RELEASE_TEST_LOG="${RELEASE_TEST_LOG:-}" RELEASE_TEST_FAIL="${RELEASE_TEST_FAIL:-}" "$squawk" --config "$root/scripts/release-squawk.toml" --reporter json "$migration" > "$work/squawk.json"; then squawk_status=0; else squawk_status=$?; fi
    else
        if env -i PATH=/usr/bin:/bin HOME="$work/squawk-home" TMPDIR="$work" "$squawk" --config "$root/scripts/release-squawk.toml" --reporter json "$migration" > "$work/squawk.json"; then squawk_status=0; else squawk_status=$?; fi
    fi
    if [ "$exception_mode" = baseline ]; then
        [ "$squawk_status" -le 1 ] && python3 -I "$helper" exception-squawk --migration "$migration" --input "$work/squawk.json" && python3 -I "$helper" exception-candidate --root "$root" --migration "$migration" >/dev/null || { echo "mac-release: approved baseline findings differ: $migration" >&2; exit 1; }
    elif [ "$exception_mode" = findings ]; then
        # a matcher-2 entry with Squawk approvals: squawk must have reported (status 1) EXACTLY the approved (rule, reported line) pairs of this file's entry: an extra finding,
        # a missing (stale) approval, a moved line or a status 0 each refuse (the helper names the finding)
        [ "$squawk_status" -eq 1 ] && python3 -I "$helper" exception-squawk --root "$root" --migration "$migration" --input "$work/squawk.json" || { echo "mac-release: approved Squawk findings differ: $migration" >&2; exit 1; }
    else
        [ "$squawk_status" -eq 0 ] || { echo "mac-release: migration gate failed: $migration" >&2; exit 1; }
        # a matcher-2 exception without Squawk approvals waives the dynamic-SQL flag only: its Squawk report must be a clean, well-formed empty list
        if [ "$exception_mode" = clean ]; then
            python3 -I "$helper" squawk-clean --input "$work/squawk.json" || { echo "mac-release: migration gate failed: $migration" >&2; exit 1; }
        fi
    fi
done < "$work/migrations"
# All gates passed. Build only in a new, private staging directory.
[ ! -L "$root/dist" ] || { echo 'mac-release: dist must not be a link' >&2; exit 1; }
mkdir -p "$root/dist"
umask 077
mkdir "$part" || { echo 'mac-release: cannot create staging destination' >&2; exit 1; }
part_created=1
# G12: a real (git-checked) release also writes the binding record of the joined installer, from the very bytes the join read; a seam release has no commit to bind to
join_hash=
if [ "$git_checked" -eq 1 ]; then
    join_hash=$(python3 -I "$helper" join-installer --deploy "$work/joinlayout" --output "$work/install.sh" --record "$work/install.sh.binding.json" --commit "$start_head" --tree "$start_tree" --object-format "$object_format" --trust-key-file "$trust_key_file")
else
    if [ "$trust_key_given" -eq 1 ]; then
        join_hash=$(python3 -I "$helper" join-installer --deploy "$work/joinlayout" --output "$work/install.sh" --trust-key-file "$trust_key_file")
    else
        python3 -I "$helper" join-installer --deploy "$work/joinlayout" --output "$work/install.sh"
    fi
    echo 'mac-release: no binding record (a test-seam release has no commit to bind to)' >&2
fi
[ "$join_hash" = "$key_start_hash" ] || { echo 'mac-release: the trust key file changed during the release' >&2; exit 1; }
sh -n "$work/install.sh" || { echo 'mac-release: joined installer is invalid shell' >&2; exit 1; }
install -m 0755 "$work/install.sh" "$part/install.sh"
epoch=${SOURCE_DATE_EPOCH:-$(date -u +%s)}
case $epoch in *[!0-9]*|'') echo 'mac-release: invalid source date' >&2; exit 1 ;; esac
image_name="cortex-server-$version-aarch64.oci.tar"
archive_name="cortex-$version-macos.tar.gz"
[ "$feature_digest" = "$(python3 -I "$helper" file-digest --path "$root/deploy/server-features")" ] || { echo 'mac-release: server feature file changed during the release' >&2; exit 1; }
# THE IMPORTED SERVER IMAGE: checked before anything is copied (its sha256 is the one named, the archive holds ONE manifest, its config says linux and arm64), copied, and
# checked again on the copy. The recipe never builds it: building was a VM job (hue's receipts, build 2).
python3 -I "$mhelper" image-check --archive "$image_archive" --sha256 "$image_sha256" > "$work/image-check.json" || { echo 'mac-release: the server image is not acceptable' >&2; exit 1; }
cp -p -- "$image_archive" "$part/$image_name"
chmod 0600 "$part/$image_name"
[ "$(python3 -I "$mhelper" digest --path "$part/$image_name")" = "$image_sha256" ] || { echo 'mac-release: the copied server image differs from the one named' >&2; exit 1; }
receipt_hash=$(python3 -I "$mhelper" verify-receipt --receipt "$image_receipt" --image-sha256 "$image_sha256" --image-commit "$image_source_commit" --version "$version") || { echo 'mac-release: the image receipt is invalid' >&2; exit 1; }
printf 'mac-release: image receipt SHA256SUMS sha256 %s\n' "$receipt_hash"
image_digest=$(python3 -I "$helper" image-digest "$part/$image_name")
server_image="localhost/cortex-server@$image_digest"
# THE IMAGE BINDING (cox W1-281): the image was built from --image-source-commit; the set is cut from this head; every path the image build READS must be identical in both,
# or the release is refused. The record names BOTH commits and the object ids compared. A test-seam release has no commit to bind to and says so.
if [ "$git_checked" -eq 1 ]; then
    python3 -I "$mhelper" image-binding --root "$root" --image-commit "$image_source_commit" --version "$version" > "$work/image-binding.json" || { echo 'mac-release: the server image is not bound to this head (a build input differs, or the image commit is not an ancestor)' >&2; exit 1; }
else
    echo 'mac-release: no image binding record (a test-seam release has no commit to bind to)' >&2
fi
db_image=$(python3 -I "$helper" db-image "$work/layout/compose.yml")
python3 -I "$mhelper" archive --layout "$work/layout" --scripts "$work/scripts" --output "$work/release.tar" \
    --version "$version" --server-image "$server_image" --db-image "$db_image" --epoch "$epoch"
gzip -n -c "$work/release.tar" > "$part/$archive_name"
python3 -I "$mhelper" manifest --output-dir "$part" --layout "$work/layout" --version "$version" \
    --server-image "$server_image" --db-image "$db_image" --label "$label" --notes-url "$notes_url" \
    --toml "${CORTEX_RELEASE_TOML:-$work/layout/cortex.toml.example}" --seams "$seams" \
    --crates "${CORTEX_RELEASE_CRATES_DIR:-$root/crates}" --epoch "$epoch"
(
    cd "$part"
    sha256sum "$archive_name" "$image_name" install.sh manifest.json install.meta > SHA256SUMS
    sha256sum -c SHA256SUMS >/dev/null
)
python3 -I "$mhelper" verify-dist --output-dir "$part" --version "$version" || { echo 'mac-release: distribution verification failed' >&2; exit 1; }
# RB-2: nothing the release read has moved under it (the image was built from the live tree; the shipped files are the copies).
[ "$(tree_digest "$root/deploy")" = "$deploy_digest" ] || { echo 'mac-release: a deploy file changed during the release' >&2; exit 1; }
[ "$(members_digest "$root/scripts")" = "$scripts_digest" ] || { echo 'mac-release: a bin script changed during the release' >&2; exit 1; }
[ "$(tree_digest "$root/mac")" = "$mac_digest" ] || { echo 'mac-release: a mac installer file changed during the release' >&2; exit 1; }
if [ "$git_checked" -eq 1 ]; then
    tree_clean "at the end"
    [ "$(git_env rev-parse HEAD)" = "$start_head" ] || { echo 'mac-release: the git HEAD moved during the release' >&2; exit 1; }
fi
# The approved key file is an input like the others: read again, it must hash as it did at the start (the join read the same bytes: see above). The re-read is the
# SAME checked, bounded, no-follow read as the start and the join (trust-key-hash is read_trust_key): a plain sha256sum of the path would block on a FIFO placed there,
# lose the size bound and follow a link (sam TAJ-SAM01). A read that fails is a refusal.
if [ "$trust_key_given" -eq 1 ]; then
    key_end_hash=$(python3 -I "$helper" trust-key-hash --trust-key-file "$trust_key_file") || { echo 'mac-release: the trust key file changed during the release' >&2; exit 1; }
    [ "$key_end_hash" = "$key_start_hash" ] || { echo 'mac-release: the trust key file changed during the release' >&2; exit 1; }
fi
# -T prevents a racing destination directory from absorbing the staging tree.
mv -T -n -- "$part" "$root/dist/$version"
[ ! -e "$part" ] || { echo 'mac-release: destination appeared during build' >&2; exit 1; }
part_created=0
# G12: the record is published as a SIBLING of the release directory (not inside it: the exact inventory of verify-dist and the unpack stays; not in SHA256SUMS)
if [ "$git_checked" -eq 1 ]; then
    mv -T -n -- "$work/install.sh.binding.json" "$root/dist/$version.install.sh.binding.json" || { echo "mac-release: cannot publish the binding record; dist/$version is already published: remove it before running again" >&2; exit 1; }
    [ ! -e "$work/install.sh.binding.json" ] || { echo "mac-release: binding record destination appeared during build; dist/$version is already published: remove it before running again" >&2; exit 1; }
fi
# The Mac extras are SIBLINGS of the release directory (not inside it: the exact inventory of verify-dist stays; not in SHA256SUMS): the image binding (both commits and every
# build input compared) and the notices and the licence that go BESIDE the signed list.
if [ "$git_checked" -eq 1 ]; then
    mv -T -n -- "$work/image-binding.json" "$root/dist/$version.image-binding.json" || { echo "mac-release: cannot publish the image binding record; dist/$version is already published: remove it before running again" >&2; exit 1; }
fi
if [ -n "$(ls -A "$work/beside")" ]; then
    mv -T -n -- "$work/beside" "$root/dist/$version.beside" || { echo "mac-release: cannot publish the notices and the licence; dist/$version is already published: remove it before running again" >&2; exit 1; }
fi
printf 'mac-release: built dist/%s\n' "$version"
