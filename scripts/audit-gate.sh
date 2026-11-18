#!/bin/sh
# audit-gate.sh: the AUDIT GATE of scripts/ci.sh (hue, 10 Nov 2026; cox row 8a0cff7a, kai's decision (A) of 11:59Z and his conditions (a) to (d); sam's AE-SAM-01, cox row 655dbb5e):
# the PROOF that the one ignored advisory still does not apply, the REVIEWED-BASIS GUARD, THEN cargo audit.
#   sh scripts/audit-gate.sh [--proof-only | --basis-only] [--root DIR]      (DIR defaults to the repository root; the two options stop after one step: scripts/audit-gate-test.sh uses them on fixtures)
# .cargo/audit.toml ignores RUSTSEC-2026-0459 (crate libsqlite3-sys 0.37.0) ONLY BECAUSE the crate is in no build and the sqlx settings are the reviewed ones (kai's line, verbatim, is the comment of that file).
# STEP 1, the proof, is kai's command:
#     cargo tree --locked --offline -i libsqlite3-sys --workspace --all-features --target all
# It must print 'nothing to print' (the crate is in Cargo.lock only through an optional driver that no workspace crate enables) or the crate must be nowhere in Cargo.lock. If it prints a PATH (the crate is in a
# build) the exception is VOID: this gate FAILS, exit 1, BEFORE cargo audit runs, and the release must be audited again without the exception. Any other failure of that command (no lock file, a lock that needs an
# update, cargo itself) is a proof that CANNOT BE MADE and is not a pass: exit 1.
# STEP 2, the guard (scripts/audit-basis-check.py), reads  cargo metadata --locked --offline --no-deps --format-version 1  and compares every workspace sqlx dependency (version, default-features, the feature SET)
# with the basis kai reviewed (=0.9.0, default-features = false, runtime-tokio, postgres, uuid, tls-none): any difference is EXCEPTION VOID, exit 1, BEFORE cargo audit. It does not widen kai's policy; what it does not cover is
# written in that file. A guard that cannot be run is not a pass either.
# STEP 3 is cargo audit itself, which reads .cargo/audit.toml at the root.
# Exit: 0 every step held (and cargo audit passed); 1 a step failed or could not be made, or cargo audit failed (cargo audit's own status); 2 a usage error.
set -u
root= proof_only=0 basis_only=0
while [ "$#" -gt 0 ]; do
  case $1 in
    --proof-only) proof_only=1; shift ;;
    --basis-only) basis_only=1; shift ;;
    --root) [ "$#" -ge 2 ] || { echo "audit-gate: --root needs a directory" >&2; exit 2; }; root=$2; shift 2 ;;
    *) echo "audit-gate: unknown argument $1" >&2; exit 2 ;;
  esac
done
[ "$proof_only" -eq 0 ] || [ "$basis_only" -eq 0 ] || { echo "audit-gate: --proof-only and --basis-only exclude each other" >&2; exit 2; }
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
[ -n "$root" ] || root=$(CDPATH= cd -- "$here/.." && pwd)
cd "$root" || { echo "audit-gate: cannot enter $root" >&2; exit 2; }
if [ "$basis_only" -eq 0 ]; then
  out=$(cargo tree --locked --offline -i libsqlite3-sys --workspace --all-features --target all 2>&1); rc=$?
  if [ "$rc" -eq 0 ] && printf '%s\n' "$out" | grep -q '^libsqlite3-sys v'; then
    echo "audit-gate: EXCEPTION VOID: libsqlite3-sys IS in a build of $root (RUSTSEC-2026-0459 is ignored only while it is in none); the release must be audited again without the exception in .cargo/audit.toml. cargo tree printed:"
    printf '%s\n' "$out" | head -n 12 | sed 's/^/audit-gate:   /'
    exit 1
  elif [ "$rc" -eq 0 ] && printf '%s\n' "$out" | grep -q 'nothing to print'; then
    echo "audit-gate: proof OK: cargo tree prints 'nothing to print': libsqlite3-sys is in no build, so the exception for RUSTSEC-2026-0459 holds"
  elif [ "$rc" -ne 0 ] && printf '%s\n' "$out" | grep -q 'did not match any packages'; then
    echo "audit-gate: proof OK: libsqlite3-sys is not in Cargo.lock (the exception for RUSTSEC-2026-0459 is moot and can be deleted)"
  else
    echo "audit-gate: the proof cannot be made (cargo tree exited $rc and printed neither a path nor 'nothing to print'): not a pass. Output:"
    printf '%s\n' "$out" | head -n 12 | sed 's/^/audit-gate:   /'
    exit 1
  fi
fi
[ "$proof_only" -eq 0 ] || exit 0
meta=$(cargo metadata --locked --offline --no-deps --format-version 1 2>&1); mrc=$?
if [ "$mrc" -ne 0 ]; then
  echo "audit-gate: the reviewed-basis check cannot be made (cargo metadata exited $mrc): not a pass. Output:"
  printf '%s\n' "$meta" | head -n 12 | sed 's/^/audit-gate:   /'
  exit 1
fi
printf '%s\n' "$meta" | python3 -I -B "$here/audit-basis-check.py" || exit 1
[ "$basis_only" -eq 0 ] || exit 0
cargo audit
