#!/usr/bin/env python3
"""make-mac-release.py (hue, 15 Nov 2026): derives scripts/mac-release.sh FROM scripts/release.sh by explicit, counted edits (cox W1-281: a copy-adapt; release.sh is not changed).
usage: python3 -I make-mac-release.py SCRIPTS_DIR      (reads SCRIPTS_DIR/release.sh, writes SCRIPTS_DIR/mac-release.sh)
Every edit names its old text and how many times it must occur; any other count stops the script. The edits ARE the documented difference between the two recipes."""
import re
import sys

d = sys.argv[1]
s = open(d + "/release.sh", encoding="utf-8").read()
EDITS = []


def rep(name, old, new, count=1):
    global s
    n = s.count(old)
    if n != count:
        raise SystemExit("edit '%s': the old text occurs %d time(s), expected %d" % (name, n, count))
    s = s.replace(old, new)
    EDITS.append((name, count))


rep("header", "#!/bin/sh\n# Build a local release. No publication is performed here.\n# Test-only command seams: CORTEX_RELEASE_CI, CORTEX_RELEASE_S5,\n# CORTEX_RELEASE_PROOF, CORTEX_RELEASE_SQUAWK, CORTEX_RELEASE_CLI,\n# CORTEX_RELEASE_GIT_SHA, CORTEX_RELEASE_CRATES_DIR and CORTEX_RELEASE_TOML.\n",
    """#!/bin/sh
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
""")
rep("tools comment", "# and cargo, podman and git are found ONCE, now, by absolute path.", "# and cargo and git are found ONCE, now, by absolute path.")
rep("podman lookup 1", "podman_bin=$(command -v podman || true)\n", "")
rep("podman lookup 2", "case $podman_bin in /*) ;; *) podman_bin= ;; esac\n", "")
rep("seam list: no CLI seam", " CORTEX_RELEASE_SQUAWK CORTEX_RELEASE_CLI CORTEX_RELEASE_GIT_SHA", " CORTEX_RELEASE_SQUAWK CORTEX_RELEASE_GIT_SHA")
rep("bin members", "bin_members='cortex-dump cortex-dump-status cortex-dump-alert cortex-restore restore-compare'", "bin_members='docker-compose-render docker-compose-lint'")
rep("variables", "min_upgrade_from=\nminimum_given=0\n", "image_archive= image_sha256= image_source_commit= image_receipt= notices= notices_sha256= license_file= license_sha256=\n")
rep("Mac release notes address", 'notes_url="https://github.com/amad-malik/ai-cortex/releases/tag/v${version}"', 'notes_url="https://github.com/amad-malik/ai-cortex/releases/tag/v${version}-macos"')
rep("options", "        --min-upgrade-from) [ \"$#\" -ge 2 ] || { echo 'release: missing minimum version' >&2; exit 2; }; min_upgrade_from=$2; minimum_given=1; shift 2 ;;\n",
    """        --image-archive) [ "$#" -ge 2 ] || { echo 'release: --image-archive needs a value' >&2; exit 2; }; [ -z "$image_archive" ] || { echo 'release: --image-archive given twice' >&2; exit 2; }; image_archive=$2; shift 2 ;;
        --image-sha256) [ "$#" -ge 2 ] || { echo 'release: --image-sha256 needs a value' >&2; exit 2; }; [ -z "$image_sha256" ] || { echo 'release: --image-sha256 given twice' >&2; exit 2; }; image_sha256=$2; shift 2 ;;
        --image-source-commit) [ "$#" -ge 2 ] || { echo 'release: --image-source-commit needs a value' >&2; exit 2; }; [ -z "$image_source_commit" ] || { echo 'release: --image-source-commit given twice' >&2; exit 2; }; image_source_commit=$2; shift 2 ;;
        --image-receipt) [ "$#" -ge 2 ] || { echo 'release: --image-receipt needs a value' >&2; exit 2; }; [ -z "$image_receipt" ] || { echo 'release: --image-receipt given twice' >&2; exit 2; }; image_receipt=$2; shift 2 ;;
        --notices) [ "$#" -ge 2 ] || { echo 'release: --notices needs a value' >&2; exit 2; }; [ -z "$notices" ] || { echo 'release: --notices given twice' >&2; exit 2; }; notices=$2; shift 2 ;;
        --notices-sha256) [ "$#" -ge 2 ] || { echo 'release: --notices-sha256 needs a value' >&2; exit 2; }; [ -z "$notices_sha256" ] || { echo 'release: --notices-sha256 given twice' >&2; exit 2; }; notices_sha256=$2; shift 2 ;;
        --license) [ "$#" -ge 2 ] || { echo 'release: --license needs a value' >&2; exit 2; }; [ -z "$license_file" ] || { echo 'release: --license given twice' >&2; exit 2; }; license_file=$2; shift 2 ;;
        --license-sha256) [ "$#" -ge 2 ] || { echo 'release: --license-sha256 needs a value' >&2; exit 2; }; [ -z "$license_sha256" ] || { echo 'release: --license-sha256 given twice' >&2; exit 2; }; license_sha256=$2; shift 2 ;;
""")
rep("minimum validation replaced by the image and beside arguments",
    "if [ \"$minimum_given\" -eq 1 ]; then\n    valid_version \"$min_upgrade_from\" || { echo 'release: invalid minimum version' >&2; exit 2; }\nfi\n",
    """# the IMPORTED server image (the Mac recipe never builds it): named, hashed and bound to a source commit
mnl=$(printf '\\nx'); mnl=${mnl%x}
[ -n "$image_archive" ] && [ -n "$image_sha256" ] && [ -n "$image_source_commit" ] || { echo 'release: --image-archive, --image-sha256 and --image-source-commit are required' >&2; exit 2; }
case $image_archive in /*) ;; *) echo 'release: --image-archive must be an absolute path without a newline' >&2; exit 2 ;; esac
case $image_archive in *"$mnl"*) echo 'release: --image-archive must be an absolute path without a newline' >&2; exit 2 ;; esac
case $image_sha256 in *[!0-9a-f]*|'') echo 'release: --image-sha256 must be 64 lowercase hex' >&2; exit 2 ;; esac
[ "${#image_sha256}" -eq 64 ] || { echo 'release: --image-sha256 must be 64 lowercase hex' >&2; exit 2; }
case $image_source_commit in *[!0-9a-f]*|'') echo 'release: --image-source-commit must be 40 lowercase hex' >&2; exit 2 ;; esac
[ "${#image_source_commit}" -eq 40 ] || { echo 'release: --image-source-commit must be 40 lowercase hex' >&2; exit 2; }
case $image_receipt in /*) ;; *) echo 'release: --image-receipt must name an absolute receipt directory' >&2; exit 2 ;; esac
case $image_receipt in *"$mnl"*) echo 'release: --image-receipt must not contain a newline' >&2; exit 2 ;; esac
# the notices and the licence go beside the signed list: each is a file and its sha256 together, or neither
for beside_pair in "notices:$notices:$notices_sha256" "license:$license_file:$license_sha256"; do
    beside_rest=${beside_pair#*:}; beside_file=${beside_rest%%:*}; beside_sha=${beside_rest#*:}
    if [ -n "$beside_file" ] || [ -n "$beside_sha" ]; then
        [ -n "$beside_file" ] && [ -n "$beside_sha" ] || { echo "release: --${beside_pair%%:*} and its sha256 are given together" >&2; exit 2; }
        case $beside_file in /*) ;; *) echo "release: --${beside_pair%%:*} must be an absolute path without a newline" >&2; exit 2 ;; esac
        case $beside_file in *"$mnl"*) echo "release: --${beside_pair%%:*} must be an absolute path without a newline" >&2; exit 2 ;; esac
        case $beside_sha in *[!0-9a-f]*) echo "release: --${beside_pair%%:*}-sha256 must be 64 lowercase hex" >&2; exit 2 ;; esac
        [ "${#beside_sha}" -eq 64 ] || { echo "release: --${beside_pair%%:*}-sha256 must be 64 lowercase hex" >&2; exit 2; }
    fi
done
""")
rep("a real release needs the notices and the licence",
    "    [ \"$trust_key_given\" -eq 1 ] || { echo 'release: --trust-key-file is required for a real release' >&2; exit 2; }\n",
    "    [ \"$trust_key_given\" -eq 1 ] || { echo 'release: --trust-key-file is required for a real release' >&2; exit 2; }\n    [ -n \"$notices\" ] && [ -n \"$license_file\" ] || { echo 'release: --notices and --license (each with its sha256) are required for a real release' >&2; exit 2; }\n")
rep("destination checks", "[ ! -e \"dist/$version.install.sh.binding.json\" ] && [ ! -L \"dist/$version.install.sh.binding.json\" ] || { echo 'release: binding record destination exists' >&2; exit 1; }\n",
    "[ ! -e \"dist/$version.install.sh.binding.json\" ] && [ ! -L \"dist/$version.install.sh.binding.json\" ] || { echo 'release: binding record destination exists' >&2; exit 1; }\n"
    "[ ! -e \"dist/$version.image-binding.json\" ] && [ ! -L \"dist/$version.image-binding.json\" ] || { echo 'release: image binding record destination exists' >&2; exit 1; }\n"
    "[ ! -e \"dist/$version.beside\" ] && [ ! -L \"dist/$version.beside\" ] || { echo 'release: the destination for the notices and the licence exists' >&2; exit 1; }\n")
rep("helpers", "helper=\"$root/scripts/release_manifest.py\"\n", "helper=\"$root/scripts/release_manifest.py\"\nmhelper=\"$root/scripts/mac_release_manifest.py\"\n")
rep("the Mac installer files, notices and licence are copied ONCE (before the key check, which reads the join layout)",
    "# The anchor join (contract G12-BINDING-RECORD-SCHEMA-nix-r4.md)",
    """# RB-2 for the Mac inputs: mac/ is copied ONCE and must hold exactly the three installer files; the join reads copies named as the joiner expects; the notices and the
# licence are read once, checked against their sha256 and copied. One recheck at the end refuses a live change made under the release.
mkdir "$work/mac"
cp -R "$root/mac/." "$work/mac/" || { echo 'release: cannot copy mac/' >&2; exit 1; }
mac_digest=$(tree_digest "$work/mac") || exit 1
[ "$(ls -A "$work/mac" | sort | tr '\\n' ' ')" = 'install-fetch.sh install-mac.sh install-prereq.sh ' ] || { echo 'release: mac/ must hold exactly install-mac.sh, install-prereq.sh and install-fetch.sh' >&2; exit 1; }
mkdir "$work/joinlayout"
cp -p -- "$work/mac/install-mac.sh" "$work/joinlayout/install.sh"
cp -p -- "$work/mac/install-prereq.sh" "$work/joinlayout/install-prereq.sh"
cp -p -- "$work/mac/install-fetch.sh" "$work/joinlayout/install-fetch.sh"
mkdir "$work/beside"
if [ -n "$notices" ]; then
    [ "$(python3 -I "$helper" file-digest --path "$notices")" = "$notices_sha256" ] || { echo 'release: the notices file is not the one named (sha256)' >&2; exit 1; }
    cp -p -- "$notices" "$work/beside/THIRD-PARTY-NOTICES.txt"
    [ "$(python3 -I "$helper" file-digest --path "$work/beside/THIRD-PARTY-NOTICES.txt")" = "$notices_sha256" ] || { echo 'release: the copied notices file differs' >&2; exit 1; }
fi
if [ -n "$license_file" ]; then
    [ "$(python3 -I "$helper" file-digest --path "$license_file")" = "$license_sha256" ] || { echo 'release: the licence file is not the one named (sha256)' >&2; exit 1; }
    cp -p -- "$license_file" "$work/beside/LICENSE"
    [ "$(python3 -I "$helper" file-digest --path "$work/beside/LICENSE")" = "$license_sha256" ] || { echo 'release: the copied licence file differs' >&2; exit 1; }
fi
# The anchor join (contract G12-BINDING-RECORD-SCHEMA-nix-r4.md)""")
rep("the anchor check reads the Mac join layout", "check-anchor --deploy \"$root/deploy\" --trust-key-file", "check-anchor --deploy \"$work/joinlayout\" --trust-key-file")
rep("the join reads the Mac layout (three call sites)", "join-installer --deploy \"$work/layout\"", "join-installer --deploy \"$work/joinlayout\"", 3)

# the build section: from the git sha to the distribution check
a = s.index("if [ -n \"${CORTEX_RELEASE_GIT_SHA:-}\" ]; then\n    git_sha=$CORTEX_RELEASE_GIT_SHA\n")
b = s.index("python3 -I \"$helper\" verify-dist --output-dir \"$part\" --version \"$version\" || { echo 'release: distribution verification failed' >&2; exit 1; }\n")
b_end = b + len("python3 -I \"$helper\" verify-dist --output-dir \"$part\" --version \"$version\" || { echo 'release: distribution verification failed' >&2; exit 1; }\n")
NEW_BUILD = """epoch=${SOURCE_DATE_EPOCH:-$(date -u +%s)}
case $epoch in *[!0-9]*|'') echo 'release: invalid source date' >&2; exit 1 ;; esac
image_name="cortex-server-$version-aarch64.oci.tar"
archive_name="cortex-$version-macos.tar.gz"
[ "$feature_digest" = "$(python3 -I "$helper" file-digest --path "$root/deploy/server-features")" ] || { echo 'release: server feature file changed during the release' >&2; exit 1; }
# THE IMPORTED SERVER IMAGE: checked before anything is copied (its sha256 is the one named, the archive holds ONE manifest, its config says linux and arm64), copied, and
# checked again on the copy. The recipe never builds it: building was a VM job (hue's receipts, build 2).
python3 -I "$mhelper" image-check --archive "$image_archive" --sha256 "$image_sha256" > "$work/image-check.json" || { echo 'release: the server image is not acceptable' >&2; exit 1; }
cp -p -- "$image_archive" "$part/$image_name"
chmod 0600 "$part/$image_name"
[ "$(python3 -I "$mhelper" digest --path "$part/$image_name")" = "$image_sha256" ] || { echo 'release: the copied server image differs from the one named' >&2; exit 1; }
receipt_hash=$(python3 -I "$mhelper" verify-receipt --receipt "$image_receipt" --image-sha256 "$image_sha256" --image-commit "$image_source_commit" --version "$version") || { echo 'release: the image receipt is invalid' >&2; exit 1; }
printf 'release: image receipt SHA256SUMS sha256 %s\\n' "$receipt_hash"
image_digest=$(python3 -I "$helper" image-digest "$part/$image_name")
server_image="localhost/cortex-server@$image_digest"
# THE IMAGE BINDING (cox W1-281): the image was built from --image-source-commit; the set is cut from this head; every path the image build READS must be identical in both,
# or the release is refused. The record names BOTH commits and the object ids compared. A test-seam release has no commit to bind to and says so.
if [ "$git_checked" -eq 1 ]; then
    python3 -I "$mhelper" image-binding --root "$root" --image-commit "$image_source_commit" --version "$version" > "$work/image-binding.json" || { echo 'release: the server image is not bound to this head (a build input differs, or the image commit is not an ancestor)' >&2; exit 1; }
else
    echo 'release: no image binding record (a test-seam release has no commit to bind to)' >&2
fi
db_image=$(python3 -I "$helper" db-image "$work/layout/compose.yml")
python3 -I "$mhelper" archive --layout "$work/layout" --scripts "$work/scripts" --output "$work/release.tar" \\
    --version "$version" --server-image "$server_image" --db-image "$db_image" --epoch "$epoch"
gzip -n -c "$work/release.tar" > "$part/$archive_name"
python3 -I "$mhelper" manifest --output-dir "$part" --layout "$work/layout" --version "$version" \\
    --server-image "$server_image" --db-image "$db_image" --label "$label" --notes-url "$notes_url" \\
    --toml "${CORTEX_RELEASE_TOML:-$work/layout/cortex.toml.example}" --seams "$seams" \\
    --crates "${CORTEX_RELEASE_CRATES_DIR:-$root/crates}" --epoch "$epoch"
(
    cd "$part"
    sha256sum "$archive_name" "$image_name" install.sh manifest.json install.meta > SHA256SUMS
    sha256sum -c SHA256SUMS >/dev/null
)
python3 -I "$mhelper" verify-dist --output-dir "$part" --version "$version" || { echo 'release: distribution verification failed' >&2; exit 1; }
"""
s = s[:a] + NEW_BUILD + s[b_end:]
EDITS.append(("the build section (git sha, CLI build, podman build and save, Linux archive, manifest and verify-dist) replaced by the image import, the binding and the Mac archive, manifest and verify-dist", 1))
rep("end check of the Mac installer files",
    "[ \"$(members_digest \"$root/scripts\")\" = \"$scripts_digest\" ] || { echo 'release: a bin script changed during the release' >&2; exit 1; }\n",
    "[ \"$(members_digest \"$root/scripts\")\" = \"$scripts_digest\" ] || { echo 'release: a bin script changed during the release' >&2; exit 1; }\n"
    "[ \"$(tree_digest \"$root/mac\")\" = \"$mac_digest\" ] || { echo 'release: a mac installer file changed during the release' >&2; exit 1; }\n")
rep("the image binding record and the notices and licence are published beside the release",
    "printf 'release: built dist/%s\\n' \"$version\"",
    """# The Mac extras are SIBLINGS of the release directory (not inside it: the exact inventory of verify-dist stays; not in SHA256SUMS): the image binding (both commits and every
# build input compared) and the notices and the licence that go BESIDE the signed list.
if [ "$git_checked" -eq 1 ]; then
    mv -T -n -- "$work/image-binding.json" "$root/dist/$version.image-binding.json" || { echo "release: cannot publish the image binding record; dist/$version is already published: remove it before running again" >&2; exit 1; }
fi
if [ -n "$(ls -A "$work/beside")" ]; then
    mv -T -n -- "$work/beside" "$root/dist/$version.beside" || { echo "release: cannot publish the notices and the licence; dist/$version is already published: remove it before running again" >&2; exit 1; }
fi
printf 'release: built dist/%s\\n' "$version\"""")
# the messages carry the Mac recipe's name
before = s.count("release: ")
s = re.sub(r"(?<![-A-Za-z])release: ", "mac-release: ", s)
EDITS.append(("every 'release: ' message prefix becomes 'mac-release: '", before))
open(d + "/mac-release.sh", "w", encoding="utf-8").write(s)
open(d + "/mac-release-differences.txt", "w", encoding="utf-8").write(
    "scripts/mac-release.sh is scripts/release.sh with exactly these edits (make-mac-release.py; name, number of occurrences of the old text):\n" + "".join("  %2d  %s\n" % (c, n) for n, c in EDITS))
print("mac-release.sh written: %d edits" % len(EDITS))
