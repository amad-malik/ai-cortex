#!/usr/bin/env bash
# tools/build-docs.sh: the documentation build for Cortex v2 (piece 13; cox WAVE1-RULINGS W1-28,
# W1-29). One renderer (tools/render_reference.py, Python 3 standard library), no network, no Node.
#
#   tools/build-docs.sh               check: the contract pin, the contract checks, the CLI capture
#                                     (SKIPPED without CORTEX_BIN), the route inventory (SKIPPED
#                                     without --inventory) and that docs/generated equals a fresh
#                                     render. A run with a skipped check ends PASS-WITH-NOT-RUN and
#                                     names what did not run
#   tools/build-docs.sh --write       the same checks, then write docs/generated (the CLI page only
#                                     when the capture ran)
#   --inventory FILE                  diff the mounted route inventory ("METHOD PATH" lines) with
#                                     the contract, both ways (sam's SLOT-03)
#   --strict                          a skipped check (inventory or CLI capture) is a failure
#
# CORTEX_BIN names the cortex binary for the CLI capture (S3-DOC-PACK-SPEC section 5, item 4).
# Unset or empty, the capture is SKIPPED: 0.2.001 has no v2 cortex binary, and a red docs build
# for that reason says nothing about the documentation. Set, it is a claim: a value that is not an
# executable file, or a command that does not answer --help, fails the build.
# An EMPTY CORTEX_BIN is read as unset, so a CI wrapper that expands an empty variable gets a green
# PASS-WITH-NOT-RUN: use --strict in CI.
# The work dir is under the repo's target/, never /tmp, and is removed on exit.
set -euo pipefail
ROOT=$(cd "$(dirname "$0")/.." && pwd)
cd "$ROOT"
WRITE=0 STRICT=0 INVENTORY=""
while [ $# -gt 0 ]; do
  case "$1" in
    --write) WRITE=1 ;;
    --strict) STRICT=1 ;;
    --inventory) INVENTORY=${2:?--inventory needs a file}; shift ;;
    *) echo "build-docs: unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done
fail() { echo "build-docs: FAIL: $*" >&2; exit 1; }
PY=(python3 -I tools/render_reference.py)

# 1. The contract copy matches its pin; tools/contract/SHA256SUMS names exactly one file.
[ "$(wc -l < tools/contract/SHA256SUMS)" -eq 1 ] || fail "tools/contract/SHA256SUMS must name exactly one contract"
NAME=$(awk '{print $2}' tools/contract/SHA256SUMS)
(cd tools/contract && sha256sum -c --quiet SHA256SUMS) > /dev/null 2>&1 \
  || fail "tools/contract/$NAME does not match its pin (tools/contract/SHA256SUMS)"
CONTRACT=tools/contract/$NAME

# 2. Items 2 and 3: every route names its errors and its auth (and its CLI).
"${PY[@]}" check "$CONTRACT" || fail "the contract checks refused it (above)"

# 3. Render and capture into the work dir.
mkdir -p target
WORK=$(mktemp -d "$ROOT/target/docs-build.XXXXXX")
trap 'rm -rf -- "$WORK"' EXIT
"${PY[@]}" render "$CONTRACT" > "$WORK/API-REFERENCE.md"
CAPTURE_RAN=0
if [ -n "${CORTEX_BIN:-}" ]; then
  "${PY[@]}" capture "$CONTRACT" "$CORTEX_BIN" > "$WORK/CLI-REFERENCE.md" \
    || fail "the CLI capture (item 4): CORTEX_BIN must be an executable file and every command the contract names must answer --help"
  CAPTURE_RAN=1
elif [ "$STRICT" = 1 ]; then
  fail "--strict: CORTEX_BIN is not set, so the CLI capture (item 4) did not run"
else
  echo "build-docs: SKIPPED: the CLI capture (item 4) needs CORTEX_BIN (the cortex binary); this is not a full pass"
fi

# 4. Item 1: the mounted route inventory equals the contract (sam's SLOT-03).
if [ -n "$INVENTORY" ]; then
  "${PY[@]}" inventory "$CONTRACT" "$INVENTORY" || fail "the route inventory check refused it (above)"
  echo "build-docs: route inventory: equal to the contract"
elif [ "$STRICT" = 1 ]; then
  fail "--strict: no --inventory file, so the route inventory check (SLOT-03) did not run"
else
  echo "build-docs: SKIPPED: the route inventory check (item 1) needs --inventory FILE; this is not a full pass"
fi

# 5. The committed pages equal the fresh render, or --write replaces them.
if [ "$WRITE" = 1 ]; then
  mkdir -p docs/generated
  cp "$WORK/API-REFERENCE.md" docs/generated/
  if [ "$CAPTURE_RAN" = 1 ]; then
    cp "$WORK/CLI-REFERENCE.md" docs/generated/
    echo "build-docs: wrote docs/generated/API-REFERENCE.md and docs/generated/CLI-REFERENCE.md"
  else
    echo "build-docs: wrote docs/generated/API-REFERENCE.md (the CLI page needs the capture: not written, not compared)"
  fi
else
  PAGES=(API-REFERENCE.md)
  [ "$CAPTURE_RAN" = 1 ] && PAGES+=(CLI-REFERENCE.md)
  for f in "${PAGES[@]}"; do
    cmp -s "$WORK/$f" "docs/generated/$f" \
      || fail "docs/generated/$f is stale or missing: run tools/build-docs.sh --write and commit it"
  done
  echo "build-docs: docs/generated is current"
fi
# One of fixed sentences (hale F-1); a run with a skipped check is not a full pass and says which.
NOT_RUN=""
[ -n "$INVENTORY" ] || NOT_RUN="the route inventory check (item 1)"
if [ "$CAPTURE_RAN" = 0 ]; then
  NOT_RUN="${NOT_RUN:+$NOT_RUN and }the CLI capture (item 4)"
fi
if [ -z "$NOT_RUN" ]; then
  echo "build-docs: PASS"
else
  echo "build-docs: PASS-WITH-NOT-RUN: $NOT_RUN did not run"
fi
