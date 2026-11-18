#!/bin/sh
# scripts/status-mount-proof.sh: vera's ENFORCING-TARGET receipt for A100 (option C, row 6f7b0dfb), on throwaway directories and a stand-in container.
#
#   CV2_DB=1 /path/to/workspace/spikes/cortex-v2-s0/admission/cv2-admit sh scripts/status-mount-proof.sh
#
# What vera asked for: effective UID/GIDs/domain; a repeated replacement of LAST_GOOD visible to the container; server writes denied; the private
# backup tree and its label untouched; the host publisher and a restore-style read still working. What it does:
#   - builds, under a fresh mktemp directory, the two directories install.sh makes (dump-status 0755 with LAST_GOOD 0644; dumps 0700 with a fake set);
#   - runs ONE stand-in container named cv2-s5-test-status from the pinned image that is already local (never pulled), with the server's own settings:
#     user 10001:10001, read-only root, cap-drop ALL, no-new-privileges, no network, no published port, and ONLY <home>/dump-status mounted, ":ro,z";
#   - a second, short-lived container named cv2-s5-test-status-ctl shows what the OLD mount (the whole tree, ":ro,z") did to the private tree's label:
#     the negative control for option C;
#   - removes both containers BY EXACT NAME, verifies it, and removes its own temporary directory by that exact path.
# It needs SELinux ENFORCING (otherwise it proves nothing) and never touches a v1 container, volume or port. The stand-in is NOT the cortex-server image:
# the server image is built by install.sh; what is proved here is the mount, the label and the user, which do not depend on what runs inside.
set -u

die() { echo "status-mount-proof: $*" >&2; exit 2; }
[ "${CV2_DB:-}" = 1 ] || die "run it as: CV2_DB=1 cv2-admit sh scripts/status-mount-proof.sh (it holds the same VM-wide reservation as the database proofs)"
command -v podman >/dev/null 2>&1 || die "podman is required"
command -v getenforce >/dev/null 2>&1 || die "getenforce is required: this receipt is only meaningful on an SELinux host"
[ "$(getenforce)" = Enforcing ] || die "SELinux is not Enforcing on this host; the receipt would prove nothing"

C=cv2-s5-test-status
CTL=cv2-s5-test-status-ctl
IMAGE='docker.io/pgvector/pgvector:0.8.2-pg18@sha256:42e7f6b4e1eceb02ff14e3e6bc6108bbe259abbe83879dc1845d0da1ddeb555d'
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo=$(CDPATH= cd -- "$here/.." && pwd)

for n in "$C" "$CTL"; do podman container exists "$n" && die "container $n already exists; remove it by exact name first"; done
podman image exists "$IMAGE" || die "the pinned image is not local; this proof does not pull"

echo "proof-under-test compose.yml $(sha256sum "$repo/deploy/compose.yml" | cut -d' ' -f1)"
echo "proof-under-test install.sh $(sha256sum "$repo/deploy/install.sh" | cut -d' ' -f1)"
echo "selinux $(getenforce); podman $(podman --version | cut -d' ' -f3); host user $(id -un) uid $(id -u)"

pass=0 fail=0
ok()  { pass=$((pass + 1)); echo "ok   $1"; }
bad() { fail=$((fail + 1)); echo "FAIL $1"; [ -z "${2:-}" ] || echo "       $2"; }
expect() { if [ "$2" = "$3" ]; then ok "$1"; else bad "$1" "expected [$2] got [$3]"; fi; }

T=$(mktemp -d "${TMPDIR:-/tmp}/cv2-s5-status-proof.XXXXXX") || die "no temporary directory"
volnames=""
teardown() {
  rc=$?
  trap - EXIT INT TERM
  echo
  echo "== teardown: removing by exact name"
  podman rm --force --volumes "$C" >/dev/null 2>&1 || true
  podman rm --force --volumes "$CTL" >/dev/null 2>&1 || true
  # the image declares an anonymous VOLUME: it belongs to the stand-in and goes with it; named here, removed by exact name if it is still there
  for v in $volnames; do podman volume rm "$v" >/dev/null 2>&1 || true; done
  gone=1
  for n in "$C" "$CTL"; do if podman container exists "$n"; then echo "STILL THERE: container $n"; gone=0; fi; done
  for v in $volnames; do if podman volume exists "$v"; then echo "STILL THERE: volume $v"; gone=0; fi; done
  rm -rf -- "$T"
  if [ -e "$T" ]; then echo "STILL THERE: $T"; gone=0; fi
  if [ "$gone" = 1 ]; then echo "REMOVAL VERIFIED: containers $C and $CTL and the temporary directory $T are gone"; else echo "REMOVAL NOT VERIFIED"; rc=1; fi
  exit "$rc"
}
trap teardown EXIT INT TERM

# ---- the two directories exactly as install.sh and cortex-dump leave them -----------------------------------------------------------------
home=$T/home
mkdir -p "$home/dump-status" "$home/dumps/pre-upgrade/set-1"
chmod 0755 "$home/dump-status"; chmod 0700 "$home/dumps"
printf '2026-11-06T00:00:00Z\n' > "$home/dump-status/LAST_GOOD"; chmod 0644 "$home/dump-status/LAST_GOOD"
printf -- '-- a synthetic dump\n' > "$home/dumps/pre-upgrade/set-1/dump.sql"
label_of() { ls -dZ "$1" | cut -d' ' -f1; }
status_before=$(label_of "$home/dump-status"); dumps_before=$(label_of "$home/dumps"); set_before=$(label_of "$home/dumps/pre-upgrade/set-1/dump.sql")
echo "labels before: dump-status $status_before; dumps $dumps_before; a dump file $set_before"

# ---- the stand-in container: the server's settings and ONLY the status directory ---------------------------------------------------------
echo "== start $C (user 10001:10001, read-only root, cap-drop ALL, no-new-privileges, --network none, no published port, only dump-status mounted :ro,z)"
podman run -d --rm --pull=never --name "$C" --label cortexv2=dev --network none --memory 256m --cpus 1 \
  --user 10001:10001 --read-only --cap-drop ALL --security-opt no-new-privileges \
  -v "$home/dump-status:/var/lib/cortex-dumps:ro,z" \
  "$IMAGE" sleep 600 >/dev/null || die "could not start $C"
x() { podman exec "$C" "$@"; }
volnames=$(podman inspect --format '{{range .Mounts}}{{if eq .Type "volume"}}{{.Name}} {{end}}{{end}}' "$C" | tr -d '\n')
echo "anonymous volume(s) of the stand-in image (not ours, removed with it): ${volnames:-none}"
sleep 2
podman container exists "$C" || die "$C is not running"

echo "== the effective identity and domain inside the container"
echo "   id: $(x id)"; echo "   groups: $(x id -G)"
dom=$(x cat /proc/self/attr/current 2>/dev/null | tr -d '\0')
echo "   domain: $dom"
expect "the process runs as uid 10001 gid 10001" "10001 10001" "$(x id -u) $(x id -g)"
case $dom in *container_t*) ok "the domain is a container domain ($dom)" ;; *) bad "the domain is a container domain" "got [$dom]" ;; esac

echo "== the server reads LAST_GOOD, and cannot write the directory"
expect "LAST_GOOD is readable and holds the host's text" "$(cat "$home/dump-status/LAST_GOOD")" "$(x cat /var/lib/cortex-dumps/LAST_GOOD 2>&1)"
if x sh -c ': > /var/lib/cortex-dumps/new-file' 2>/dev/null; then bad "creating a file in the status directory is denied" "it was created"; else ok "creating a file in the status directory is denied"; fi
if x sh -c 'echo x >> /var/lib/cortex-dumps/LAST_GOOD' 2>/dev/null; then bad "appending to LAST_GOOD is denied" "it was written"; else ok "appending to LAST_GOOD is denied"; fi
if x rm -f /var/lib/cortex-dumps/LAST_GOOD 2>/dev/null && ! [ -e "$home/dump-status/LAST_GOOD" ]; then bad "deleting LAST_GOOD is denied" "it was deleted"; else ok "deleting LAST_GOOD is denied"; fi
expect "LAST_GOOD is still there, unchanged" "2026-11-06T00:00:00Z" "$(cat "$home/dump-status/LAST_GOOD")"
case $(x sh -c 'grep " /var/lib/cortex-dumps " /proc/self/mountinfo' 2>/dev/null) in *" ro,"*|*" ro "*) ok "the mount is read-only in the container's own mount table" ;; *) bad "the mount is read-only in the container's own mount table" "$(x sh -c 'grep " /var/lib/cortex-dumps " /proc/self/mountinfo' 2>&1 | cut -c1-200)" ;; esac

echo "== only the status directory is mounted: no sign of the private tree"
# the image declares an anonymous VOLUME of its own (under the container storage, not under our home): it is not a mount of ours and is told apart by its source
mounts=$(podman inspect --format '{{range .Mounts}}{{.Source}}={{.Destination}}={{.RW}};{{end}}' "$C" | tr ';' '\n' | grep -v '^$')
ours=$(printf '%s\n' "$mounts" | grep -F "$T/" || true)
expect "exactly one mount of ours, the status directory, read-only" "$home/dump-status=/var/lib/cortex-dumps=false" "$ours"
expect "no other mount is on the destination of the status directory" "1" "$(printf '%s\n' "$mounts" | grep -c '=/var/lib/cortex-dumps=')"
expect "no mount source lies under the private tree" "0" "$(printf '%s\n' "$mounts" | grep -cF "$home/dumps" || true)"
expect "the directory holds LAST_GOOD alone" "LAST_GOOD" "$(x ls -A /var/lib/cortex-dumps)"
if x sh -c 'ls -d /var/lib/cortex-dumps/../dumps /var/lib/dumps /dumps 2>/dev/null' | grep -q .; then bad "no path to a dumps directory exists in the container" "$(x sh -c 'ls -d /var/lib/cortex-dumps/../dumps /var/lib/dumps /dumps 2>/dev/null')"; else ok "no path to a dumps directory exists in the container"; fi

echo "== a repeated replacement of LAST_GOOD, the way the publisher does it (a temporary file INSIDE the directory, then a rename), is visible without a restart"
for n in 1 2 3; do
  v="2026-11-06T0$n:00:00Z"
  printf '%s\n' "$v" > "$home/dump-status/.LAST_GOOD.tmp$$"; chmod 0644 "$home/dump-status/.LAST_GOOD.tmp$$"
  mv -f "$home/dump-status/.LAST_GOOD.tmp$$" "$home/dump-status/LAST_GOOD"
  expect "replacement $n is what the container reads" "$v" "$(x cat /var/lib/cortex-dumps/LAST_GOOD 2>&1)"
done
echo "   label of the replaced file on the host: $(label_of "$home/dump-status/LAST_GOOD")"
# the control: a file made OUTSIDE the directory and renamed in keeps its own label; the container may not be able to read it. That is why the temporary file is made inside.
printf '2026-11-06T09:00:00Z\n' > "$T/outside.tmp"; chmod 0644 "$T/outside.tmp"
mv -f "$T/outside.tmp" "$home/dump-status/LAST_GOOD"
outside_label=$(label_of "$home/dump-status/LAST_GOOD")
echo "   control (a file made outside and renamed in): label $outside_label; the container reads: $(x cat /var/lib/cortex-dumps/LAST_GOOD 2>&1 | head -1 | cut -c1-80)"
# restore the file the proper way, so that the rest of the receipt is about the real publisher
printf '2026-11-06T10:00:00Z\n' > "$home/dump-status/.LAST_GOOD.tmp$$"; chmod 0644 "$home/dump-status/.LAST_GOOD.tmp$$"; mv -f "$home/dump-status/.LAST_GOOD.tmp$$" "$home/dump-status/LAST_GOOD"
expect "the proper publisher path works again after the control" "2026-11-06T10:00:00Z" "$(x cat /var/lib/cortex-dumps/LAST_GOOD 2>&1)"

echo "== labels: only the status directory was relabeled; the private tree and its files were not"
status_after=$(label_of "$home/dump-status"); dumps_after=$(label_of "$home/dumps"); set_after=$(label_of "$home/dumps/pre-upgrade/set-1/dump.sql")
echo "labels after: dump-status $status_after; dumps $dumps_after; a dump file $set_after"
case $status_after in *container_file_t*) ok "dump-status carries a container file label ($status_after)" ;; *) bad "dump-status carries a container file label" "got [$status_after] (was [$status_before])" ;; esac
expect "the private dumps directory keeps its label" "$dumps_before" "$dumps_after"
expect "a dump file keeps its label" "$set_before" "$set_after"
expect "the host can still read a dump set (the restore side)" "-- a synthetic dump" "$(cat "$home/dumps/pre-upgrade/set-1/dump.sql")"
expect "the private tree is still 0700" "700" "$(stat -c %a "$home/dumps")"

# ---- the negative control: the OLD mount (the whole tree, :ro,z) relabels the private tree --------------------------------------------
echo "== control: what the whole-tree mount of increment 5a did to the private tree (this is what option C avoids)"
ctl_tree=$T/ctl; mkdir -p "$ctl_tree/dumps/set-1"; chmod 0700 "$ctl_tree/dumps"; printf 'x\n' > "$ctl_tree/dumps/set-1/dump.sql"
ctl_before=$(label_of "$ctl_tree/dumps/set-1/dump.sql")
podman run --rm --pull=never --name "$CTL" --label cortexv2=dev --network none --user 10001:10001 --read-only --cap-drop ALL --security-opt no-new-privileges \
  -v "$ctl_tree:/var/lib/cortex-dumps:ro,z" "$IMAGE" true >/dev/null 2>&1 || echo "   (the control container could not run: $?)"
ctl_after=$(label_of "$ctl_tree/dumps/set-1/dump.sql")
echo "   a dump file in the control tree: label before $ctl_before, after $ctl_after"
if [ "$ctl_before" != "$ctl_after" ]; then ok "control: the whole-tree mount relabels the private tree's files ($ctl_before -> $ctl_after)"; else bad "control: the whole-tree mount relabels the private tree's files" "the label did not change: the control proves nothing here"; fi

echo
echo "status-mount-proof: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
