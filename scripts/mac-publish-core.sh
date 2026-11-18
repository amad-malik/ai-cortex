#!/bin/bash
# Mac publication core: invoke ONLY through the reviewed go gate. The inner
# uploader is separately reviewed and its hash and arguments are named by the go.
# The outer gate's GATE_EXACT_LINE is a procedure aid, not a security boundary.
set -eu
unset -f '[' test compgen builtin command type 2>/dev/null || :
for fn in $(compgen -A function); do unset -f "$fn"; done
PATH=/usr/bin:/bin
export PATH
unset BASH_ENV ENV
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
[ "$#" -eq 21 ] || { echo 'mac-publish: wrong argument count' >&2; exit 2; }
repo_id=$1 version=$2 target=$3 set_dir=$4 key=$5 receipt=$6 receipt_sha=$7 inner=$8 inner_sha=$9
source_commit=${10}
shift 10
[ "$#" -eq 11 ] || exit 2
[[ $repo_id =~ ^[1-9][0-9]{0,14}$ && $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ && $target =~ ^[0-9a-f]{40}$ && $source_commit =~ ^[0-9a-f]{40}$ && $inner_sha =~ ^[0-9a-f]{64}$ ]] || { echo 'mac-publish: invalid binding' >&2; exit 2; }
tag="v${version}-macos"
inner_line="PUSH PRERELEASE REPO-ID $repo_id $tag $target"
expected="$inner_line SOURCE $source_commit"
[ "${GATE_EXACT_LINE:-}" = "$expected" ] || { echo 'mac-publish: verified go line does not match' >&2; exit 1; }
[ "$1" = apply-core ] && [ "$2" = amad-malik/ai-cortex ] && [ "$3" = "$tag" ] && [ "$4" = "$target" ] && [ "${10}" = "$repo_id" ] || { echo 'mac-publish: inner publication arguments differ' >&2; exit 1; }
root=$(CDPATH= cd -- "$here/.." && pwd)
git_env() { env -i PATH=/usr/bin:/bin HOME=/nonexistent GIT_CONFIG_NOSYSTEM=1 git -C "$root" "$@"; }
check_source() {
  head=$(git_env rev-parse HEAD) || { echo 'mac-publish: cannot read source head' >&2; exit 1; }
  state=$(git_env status --porcelain --untracked-files=all) || { echo 'mac-publish: cannot read source status' >&2; exit 1; }
  [ "$head" = "$source_commit" ] && [ -z "$state" ] || { echo 'mac-publish: source head differs or tree is not clean' >&2; exit 1; }
  for file in "$here/install-mac-release-check.sh" "$here/install-mac-test.sh"; do
    [ -f "$file" ] && [ ! -L "$file" ] || { echo 'mac-publish: signed-set suite is missing or linked' >&2; exit 1; }
  done
}
check_source
[ -f "$inner" ] && [ ! -L "$inner" ] || { echo 'mac-publish: uploader is not a regular file' >&2; exit 1; }
actual=$(/usr/bin/sha256sum -- "$inner"); [ "${actual%% *}" = "$inner_sha" ] || { echo 'mac-publish: uploader hash differs' >&2; exit 1; }
helper="$here/mac-postsign-check.py"
[ -f "$helper" ] && [ ! -L "$helper" ] || { echo 'mac-publish: receipt checker is missing' >&2; exit 1; }
actual=$(/usr/bin/sha256sum -- "$helper")
[ "${actual%% *}" = 031ac60fec2ad5d034081721b4a0e95eb512f11761f9e3469f347842a78748a7 ] || { echo 'mac-publish: receipt checker hash differs' >&2; exit 1; }
check_list() {
  python3 -I - "$version" "$set_dir" "$5" <<'PY'
import hashlib, os, re, stat, sys
from pathlib import Path
version, directory, list_name = sys.argv[1:]
root = Path(directory)
if not root.is_absolute() or root.is_symlink() or not root.is_dir() or root.resolve() != root:
    raise SystemExit('mac-publish: invalid set directory')
expected = {f'cortex-{version}-macos.tar.gz', f'cortex-server-{version}-aarch64.oci.tar', 'install.sh', 'manifest.json', 'install.meta', 'SHA256SUMS', 'SHA256SUMS.sig', 'THIRD-PARTY-NOTICES.txt', 'LICENSE'}
seen = set()
try:
    list_path = Path(list_name)
    if not list_path.is_absolute() or list_path.is_symlink() or not list_path.is_file() or list_path.stat().st_size > 65536:
        raise ValueError('asset list must be a bounded regular file')
    lines = list_path.read_text(encoding='utf-8').splitlines()
    for line in lines:
        m = re.fullmatch(r'([0-9a-f]{64})  (/.+)', line)
        if not m: raise ValueError('bad asset list line')
        p = Path(m.group(2)); st = p.lstat()
        if not stat.S_ISREG(st.st_mode) or st.st_nlink != 1 or p.name in seen: raise ValueError('asset is not a unique regular file')
        if p.name in expected - {'THIRD-PARTY-NOTICES.txt', 'LICENSE'} and p.parent != root: raise ValueError('signed asset is outside set')
        seen.add(p.name)
        h = hashlib.sha256()
        with p.open('rb') as src:
            for block in iter(lambda: src.read(1024 * 1024), b''): h.update(block)
        if h.hexdigest() != m.group(1): raise ValueError('asset digest mismatch')
    if seen != expected: raise ValueError('asset inventory differs')
except (OSError, UnicodeError, ValueError) as exc:
    raise SystemExit('mac-publish: ' + str(exc))
PY
}
check_list "$@"
python3 -I -B "$helper" "$set_dir" "$key" "$version" "$receipt" "$receipt_sha" || { echo 'mac-publish: signed-set receipt gate failed' >&2; exit 1; }
check_list "$@"
check_source
actual=$(/usr/bin/sha256sum -- "$inner"); [ "${actual%% *}" = "$inner_sha" ] || { echo 'mac-publish: uploader changed' >&2; exit 1; }
# The checked core must repeat its own preflight and read-back. A partial remote
# create is reported by that core; this wrapper never retries or cleans it up.
GATE_EXACT_LINE="$inner_line" exec bash "$inner" "$@"
