#!/bin/sh
# audit-gate-test.sh: the cell of the audit exception (hue, 10 Nov 2026; cox row 8a0cff7a, kai's conditions (a) to (d) in his row ad064581).
#
# .cargo/audit.toml ignores ONE advisory (RUSTSEC-2026-0459, libsqlite3-sys 0.37.0) ONLY BECAUSE the crate is in no build. This cell keeps the exception from outliving its reason:
#   AG-1  the PLANTED case: a fixture workspace where libsqlite3-sys IS in the build graph: the proof of scripts/audit-gate.sh must FAIL there (exit 1, 'EXCEPTION VOID', naming the crate)
#   AG-2  a fixture where the crate is nowhere in Cargo.lock: the proof passes and says so
#   AG-3  a fixture with no Cargo.lock at all: cargo tree --locked cannot run, and a proof that cannot be made is NOT a pass (exit 1)
#   AG-4  the REAL tree: the proof passes ('nothing to print')
#   AG-5  .cargo/audit.toml is exactly one table with exactly one key and exactly one advisory id: no wildcard, nothing else (kai's condition (b))
#   AG-6  kai's line is in the file VERBATIM (an edit of it is a deliberate act that shows in a diff of this cell too)
#   AG-7  ci.sh runs the proof and this cell: the audit gate is 'sh scripts/audit-gate.sh' and a gate runs 'sh scripts/audit-gate-test.sh' (a test no gate runs protects nothing)
#   AG-8  the order inside the gate, with a FAKE cargo: the proof runs first with kai's exact command; cargo audit runs after a good proof and its status is the gate's; a void or unprovable proof stops the gate BEFORE cargo audit
#   AG-9 to AG-14  sam's AE-SAM-01 (cox 655dbb5e): kai's line voids the exception 'the moment that command prints a path OR THE SQLX FEATURES CHANGE'. The REVIEWED-BASIS GUARD compares EVERY workspace sqlx dependency
#      (version, default-features, the feature SET) with the basis kai reviewed (=0.9.0, default-features = false, runtime-tokio, postgres, uuid, tls-none) and fails EXCEPTION VOID on any difference: planted drifts with SQLite still
#      ABSENT from the graph (an extra feature, default features on, a subset, another version, a member adding a feature, no sqlx at all)
#   AG-15 the same four features in ANOTHER ORDER pass (the order makes no false difference);  AG-16 the control: the reviewed fixture and the REAL tree pass
#   AG-8g/h the guard runs between the proof and cargo audit and stops the gate before cargo audit (a drift, or a basis that cannot be read)
#   AG-17 to AG-26 (sam's AE-SAM-02, cox d722e57a): the guard must VALIDATE the cargo metadata (required fields and their TYPES) BEFORE it filters: alongside ONE approved sqlx record, a package with no dependencies array,
#      a dependency with no name, features as a MAPPING or holding a NON-string, packages of the wrong type, a boolean or a string of the wrong type, a dependency that is not an object, workspace_members of the wrong type must each
#      be REFUSED (exit 1, 'cannot be made', and the phrase of THAT check), never 'basis OK'; AG-17 the control: a valid, multiline record with unrelated extra fields (even a malformed field of ANOTHER dependency) passes
#   AG-8i the gate level: malformed metadata stops the gate before cargo audit
# Offline (cargo tree --locked --offline; the fixtures have path dependencies only). Run it like any test:  cv2-admit sh scripts/audit-gate-test.sh
set -u
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
root=$(dirname "$here")
gate="$here/audit-gate.sh"
fx="$here/audit-gate-fixtures"
pass=0 fail=0
ok()  { pass=$((pass + 1)); echo "ok   $1"; }
bad() { fail=$((fail + 1)); echo "FAIL $1"; [ -z "${2:-}" ] || echo "       $2"; }
mkdir -p "${TMPDIR:-$root/target}" || exit 2
T=$(mktemp -d "${TMPDIR:-$root/target}/audit-gate-test.XXXXXX") || exit 2
trap 'rm -rf "$T"' EXIT
first2() { printf '%s\n' "$1" | head -n 2 | tr '\n' '|' | cut -c1-220; }

# the reviewed-basis guard alone on a root: sets rc and out
basis() {
  if [ -f "$gate" ]; then out=$(sh "$gate" --basis-only --root "$1" 2>&1); rc=$?; else out="scripts/audit-gate.sh is missing"; rc=127; fi
}

# the proof alone on a root: sets rc and out
proof() {
  if [ -f "$gate" ]; then out=$(sh "$gate" --proof-only --root "$1" 2>&1); rc=$?; else out="scripts/audit-gate.sh is missing"; rc=127; fi
}

proof "$fx/in-graph"
if [ "$rc" -eq 1 ] && printf '%s\n' "$out" | grep -q 'EXCEPTION VOID' && printf '%s\n' "$out" | grep -q 'libsqlite3-sys'; then ok "AG-1 the planted case (the crate IS in the build graph): the proof FAILS, exit 1, EXCEPTION VOID, the crate named"; else bad "AG-1 the planted case (the crate IS in the build graph) must make the proof fail with EXCEPTION VOID" "rc=$rc: $(first2 "$out")"; fi

proof "$fx/absent"
if [ "$rc" -eq 0 ] && printf '%s\n' "$out" | grep -q 'not in Cargo.lock'; then ok "AG-2 the crate is nowhere in Cargo.lock: the proof passes and says so"; else bad "AG-2 a lock without the crate must pass the proof and say 'not in Cargo.lock'" "rc=$rc: $(first2 "$out")"; fi

proof "$fx/unlocked"
if [ "$rc" -eq 1 ] && printf '%s\n' "$out" | grep -q 'cannot be made'; then ok "AG-3 no Cargo.lock: cargo tree --locked cannot run, the proof FAILS (exit 1), it is not a pass"; else bad "AG-3 a proof that cannot be made must fail with exit 1 and say 'cannot be made'" "rc=$rc: $(first2 "$out")"; fi

proof "$root"
if [ "$rc" -eq 0 ] && printf '%s\n' "$out" | grep -q 'nothing to print'; then ok "AG-4 the REAL tree: the proof passes (cargo tree prints 'nothing to print': libsqlite3-sys is in no build)"; else bad "AG-4 the real tree must pass the proof with 'nothing to print'" "rc=$rc: $(first2 "$out")"; fi

if [ -f "$root/.cargo/audit.toml" ]; then
  r=$(python3 -I - "$root/.cargo/audit.toml" <<'PY' 2>&1
import sys, tomllib
d = tomllib.load(open(sys.argv[1], "rb"))
adv = d.get("advisories")
good = list(d) == ["advisories"] and isinstance(adv, dict) and list(adv) == ["ignore"] and adv["ignore"] == ["RUSTSEC-2026-0459"]
print("OK" if good else "BAD %r" % (d,))
PY
  )
else r="BAD .cargo/audit.toml is missing"; fi
if [ "$r" = OK ]; then ok "AG-5 .cargo/audit.toml is one table, one key, exactly [RUSTSEC-2026-0459]: no wildcard, nothing else"; else bad "AG-5 .cargo/audit.toml must be exactly [advisories] ignore = [\"RUSTSEC-2026-0459\"]" "$r"; fi

kai_line=$(cat <<'EOF'
kai (development lead), 10 Nov 2026 11:59Z: RUSTSEC-2026-0459 (libsqlite3-sys 0.37.0, a C SQLite vulnerability) is ignored by cargo audit for v0.2.1 ONLY BECAUSE the crate is a Cargo.lock entry of sqlx's optional sqlite driver and is in no build: cargo tree --locked --offline -i libsqlite3-sys --workspace --all-features --target all prints nothing to print; our sqlx has default-features = false and the features runtime-tokio, postgres, uuid, tls-none; cargo deny says advisories ok. The exception is VOID, and the release must be audited again, the moment that command prints a path or the sqlx features change. Review at the next sqlx bump (v0.2.002).
EOF
)
if [ -f "$root/.cargo/audit.toml" ] && grep -qxF -- "# $kai_line" "$root/.cargo/audit.toml"; then ok "AG-6 kai's line is in .cargo/audit.toml verbatim, as one comment line"; else bad "AG-6 kai's line must be in .cargo/audit.toml verbatim as the comment line '# kai (development lead), 10 Nov 2026 11:59Z: ...'"; fi

if grep -qF 'gate audit blocking "$admit" sh scripts/audit-gate.sh' "$root/scripts/ci.sh" && ! grep -qF 'gate audit blocking "$admit" cargo audit' "$root/scripts/ci.sh" && grep -qF 'gate audit-gate-test blocking "$admit" sh scripts/audit-gate-test.sh' "$root/scripts/ci.sh"; then ok "AG-7 ci.sh runs the proof (the audit gate is sh scripts/audit-gate.sh) and this cell (gate audit-gate-test)"; else bad "AG-7 ci.sh must run the audit gate as 'sh scripts/audit-gate.sh' and a gate 'audit-gate-test' for this cell, and no bare 'cargo audit' gate"; fi

# AG-9 to AG-16 (sam's AE-SAM-01): the REVIEWED-BASIS GUARD
drift() {  # drift ID FIXTURE DESCRIPTION: a planted drift must make the guard fail with EXCEPTION VOID, naming sqlx
  basis "$fx/$2"
  if [ "$rc" -eq 1 ] && printf '%s\n' "$out" | grep -q 'EXCEPTION VOID' && printf '%s\n' "$out" | grep -q 'sqlx'; then ok "$1 $3: the guard FAILS, exit 1, EXCEPTION VOID, sqlx named"; else bad "$1 $3 must make the guard fail with EXCEPTION VOID" "rc=$rc: $(first2 "$out")"; fi
}
drift AG-9 basis-extra-feature "the planted FEATURE DRIFT (a feature added that does not bring SQLite in)"
drift AG-10 basis-default-on "default features turned back on"
drift AG-11 basis-subset "a SUBSET of the reviewed features (tls-none missing)"
drift AG-12 basis-version "another sqlx version (=0.9.1)"
drift AG-13 basis-member-extra "a MEMBER adding a feature over the workspace entry (the second record)"
drift AG-14 basis-none "no workspace crate depends on sqlx (nothing to compare)"
basis "$fx/basis-reordered"
if [ "$rc" -eq 0 ] && printf '%s\n' "$out" | grep -q 'basis OK'; then ok "AG-15 the same four features in ANOTHER ORDER: the guard passes (the order makes no false difference)"; else bad "AG-15 the same feature set in another order must pass" "rc=$rc: $(first2 "$out")"; fi
basis "$fx/basis-ok"; rc1=$rc; out1=$out
basis "$root"
if [ "$rc1" -eq 0 ] && [ "$rc" -eq 0 ] && printf '%s\n' "$out1" | grep -q 'basis OK' && printf '%s\n' "$out" | grep -q 'basis OK'; then ok "AG-16 the CONTROL: the reviewed fixture and the REAL tree pass the guard ('basis OK')"; else bad "AG-16 the reviewed fixture and the real tree must pass the guard" "fixture rc=$rc1, real tree rc=$rc: $(first2 "$out")"; fi

# AG-8 the order inside the gate, with a fake cargo (no network, nothing is audited)
fake="$T/fake"; mkdir -p "$fake"
cat > "$fake/cargo" <<'EOF'
#!/bin/sh
echo "$*" >> "$FAKE_LOG"
case $1 in
  tree) case ${FAKE_TREE:-nothing} in
          nothing) echo 'warning: nothing to print.' >&2; exit 0 ;;
          path) echo 'libsqlite3-sys v0.37.0'; echo '`-- sqlx-sqlite v0.9.0'; exit 0 ;;
          absent) echo 'error: package ID specification `libsqlite3-sys` did not match any packages' >&2; exit 101 ;;
          *) echo 'error: something else broke' >&2; exit 101 ;;
        esac ;;
  metadata) case ${FAKE_META:-ok} in
              ok) printf '%s\n' '{"packages":[{"name":"app","dependencies":[{"name":"sqlx","req":"=0.9.0","uses_default_features":false,"features":["runtime-tokio","postgres","uuid","tls-none"]}]}]}'; exit 0 ;;
              drift) printf '%s\n' '{"packages":[{"name":"app","dependencies":[{"name":"sqlx","req":"=0.9.0","uses_default_features":false,"features":["runtime-tokio","postgres","uuid","tls-none","macros"]}]}]}'; exit 0 ;;
              malformed) printf '%s\n' '{"packages":[{"name":"app","dependencies":[{"name":"sqlx","req":"=0.9.0","uses_default_features":false,"features":["runtime-tokio","postgres","uuid","tls-none"]}]},{"name":"other"}]}'; exit 0 ;;
              *) echo 'error: metadata broke' >&2; exit 101 ;;
            esac ;;
  audit) exit "${FAKE_AUDIT_RC:-0}" ;;
esac
exit 99
EOF
chmod +x "$fake/cargo"
fakegate() {  # fakegate NAME TREE AUDIT_RC: the whole gate (no --proof-only) with the fake cargo; sets rc, out and $T/NAME.log
  : > "$T/$1.log"
  if [ -f "$gate" ]; then out=$(FAKE_LOG="$T/$1.log" FAKE_TREE=$2 FAKE_AUDIT_RC=$3 FAKE_META=${FAKE_META:-ok} PATH="$fake:$PATH" sh "$gate" --root "$fx/absent" ${4:-} 2>&1); rc=$?; else out="scripts/audit-gate.sh is missing"; rc=127; fi
}
want_tree='tree --locked --offline -i libsqlite3-sys --workspace --all-features --target all'
want_meta='metadata --locked --offline --no-deps --format-version 1'
fakegate g1 nothing 0
if [ "$rc" -eq 0 ] && [ "$(sed -n 1p "$T/g1.log")" = "$want_tree" ] && [ "$(sed -n 2p "$T/g1.log")" = "$want_meta" ] && [ "$(sed -n 3p "$T/g1.log")" = audit ] && [ "$(wc -l < "$T/g1.log")" -eq 3 ]; then ok "AG-8a a good proof, the basis guard, then cargo audit: kai's exact tree command first, cargo metadata second, then 'audit' with no flag, exit 0"; else bad "AG-8a the gate must run kai's exact tree command, then the guard's cargo metadata, then 'cargo audit', and exit 0" "rc=$rc log: $(tr '\n' '|' < "$T/g1.log" | cut -c1-200)"; fi
fakegate g2 nothing 1
if [ "$rc" -eq 1 ] && [ "$(sed -n 3p "$T/g2.log")" = audit ]; then ok "AG-8b a good proof and a failing cargo audit: the gate fails with cargo audit's status"; else bad "AG-8b cargo audit's failing status must be the gate's" "rc=$rc log: $(tr '\n' '|' < "$T/g2.log" | cut -c1-200)"; fi
fakegate g3 path 0
if [ "$rc" -eq 1 ] && printf '%s\n' "$out" | grep -q 'EXCEPTION VOID' && [ "$(wc -l < "$T/g3.log")" -eq 1 ]; then ok "AG-8c the crate in the graph: EXCEPTION VOID, exit 1, and cargo audit is NOT run"; else bad "AG-8c a void exception must stop the gate before cargo audit" "rc=$rc log: $(tr '\n' '|' < "$T/g3.log" | cut -c1-200)"; fi
fakegate g4 absent 0
if [ "$rc" -eq 0 ] && [ "$(sed -n 3p "$T/g4.log")" = audit ]; then ok "AG-8d the crate nowhere in the lock: the gate goes on to cargo audit"; else bad "AG-8d a lock without the crate must go on to cargo audit" "rc=$rc log: $(tr '\n' '|' < "$T/g4.log" | cut -c1-200)"; fi
fakegate g5 broken 0
if [ "$rc" -eq 1 ] && printf '%s\n' "$out" | grep -q 'cannot be made' && [ "$(wc -l < "$T/g5.log")" -eq 1 ]; then ok "AG-8e any other cargo failure: the proof cannot be made, exit 1, and cargo audit is NOT run"; else bad "AG-8e an unprovable proof must stop the gate before cargo audit" "rc=$rc log: $(tr '\n' '|' < "$T/g5.log" | cut -c1-200)"; fi

fakegate g6 nothing 0 --proof-only
if [ "$rc" -eq 0 ] && [ "$(wc -l < "$T/g6.log")" -eq 1 ] && [ "$(sed -n 1p "$T/g6.log")" = "$want_tree" ]; then ok "AG-8f --proof-only stops after the proof: cargo audit is NOT run (the cell's own fixtures rely on it)"; else bad "AG-8f --proof-only must stop after the proof and not run cargo audit" "rc=$rc log: $(tr '\n' '|' < "$T/g6.log" | cut -c1-200)"; fi

FAKE_META=drift fakegate g7 nothing 0
if [ "$rc" -eq 1 ] && printf '%s\n' "$out" | grep -q 'EXCEPTION VOID' && [ "$(wc -l < "$T/g7.log")" -eq 2 ] && [ "$(sed -n 2p "$T/g7.log")" = "$want_meta" ]; then ok "AG-8g a drifted sqlx basis: EXCEPTION VOID, exit 1, and cargo audit is NOT run (the guard sits between the proof and the audit)"; else bad "AG-8g a drifted basis must stop the gate before cargo audit" "rc=$rc log: $(tr '\n' '|' < "$T/g7.log" | cut -c1-200)"; fi
FAKE_META=broken fakegate g8 nothing 0
if [ "$rc" -eq 1 ] && printf '%s\n' "$out" | grep -q 'cannot be made' && [ "$(wc -l < "$T/g8.log")" -eq 2 ]; then ok "AG-8h a basis that cannot be read (cargo metadata fails): the check cannot be made, exit 1, and cargo audit is NOT run"; else bad "AG-8h an unreadable basis must stop the gate before cargo audit" "rc=$rc log: $(tr '\n' '|' < "$T/g8.log" | cut -c1-200)"; fi

# AG-17 to AG-26 (sam's AE-SAM-02): the guard VALIDATES the metadata before it filters; canned cargo metadata is fed straight to the helper, always ALONGSIDE one approved sqlx record
okrec='{"name":"sqlx","req":"=0.9.0","uses_default_features":false,"features":["runtime-tokio","postgres","uuid","tls-none"],"optional":false}'
canned() { out=$(printf '%s' "$1" | python3 -I -B "$here/audit-basis-check.py" 2>&1); rc=$?; }
mal() {  # mal ID JSON PHRASE DESCRIPTION: refused (exit 1, 'cannot be made') with the phrase of THIS check, never 'basis OK'
  canned "$2"
  if [ "$rc" -eq 1 ] && printf '%s\n' "$out" | grep -q 'cannot be made' && printf '%s\n' "$out" | grep -qF -- "$3" && ! printf '%s\n' "$out" | grep -q 'basis OK'; then ok "$1 $4: refused, exit 1, cannot be made, '$3'"; else bad "$1 $4 must be refused with 'cannot be made' and '$3', never 'basis OK'" "rc=$rc: $(first2 "$out")"; fi
}
canned '{
  "packages": [ {"name":"app","version":"0.0.1","extra":{"a":1},"dependencies":[ {"name":"serde","req":"^1","features":{"weird":1}}, '"$okrec"' ]} ],
  "workspace_members": ["app 0.0.1 (path+file:///x)"], "future_field": [1,2,3]
}'
if [ "$rc" -eq 0 ] && printf '%s\n' "$out" | grep -q 'basis OK'; then ok "AG-17 the CONTROL: valid multiline metadata with unrelated extra fields (even a malformed field of ANOTHER dependency) passes"; else bad "AG-17 valid metadata with unrelated extra fields must pass" "rc=$rc: $(first2 "$out")"; fi
mal AG-18 '{"packages":[{"name":"app","dependencies":['"$okrec"']},{"name":"other"}]}' "no 'dependencies' array" "a package with NO dependencies array beside an approved record"
mal AG-19 '{"packages":[{"name":"app","dependencies":['"$okrec"',{"req":"1"}]}]}' "dependency has no string 'name'" "a dependency with NO name beside an approved record"
mal AG-20 '{"packages":[{"name":"app","dependencies":[{"name":"sqlx","req":"=0.9.0","uses_default_features":false,"features":{"runtime-tokio":1,"postgres":1,"uuid":1,"tls-none":1}}]}]}' "'features' is not an array of strings" "features as a MAPPING (its keys are the reviewed set)"
mal AG-21 '{"packages":[{"name":"app","dependencies":[{"name":"sqlx","req":"=0.9.0","uses_default_features":false,"features":["runtime-tokio","postgres","uuid","tls-none",5]}]}]}' "'features' is not an array of strings" "a features array holding a NON-string"
mal AG-22 '{"packages":{"app":1}}' "'packages' is not an array" "packages of the wrong type"
mal AG-23 '{"packages":[{"name":"app","dependencies":[{"name":"sqlx","req":"=0.9.0","uses_default_features":"false","features":["runtime-tokio","postgres","uuid","tls-none"]}]}]}' "'uses_default_features' is not a boolean" "uses_default_features as a STRING"
mal AG-24 '{"packages":[{"name":"app","dependencies":[{"name":"sqlx","req":9,"uses_default_features":false,"features":["runtime-tokio","postgres","uuid","tls-none"]}]}]}' "'req' is not a string" "req as a NUMBER"
mal AG-25 '{"packages":[{"name":"app","dependencies":['"$okrec"',"sqlx"]}]}' "is not an object" "a dependency record that is not an object"
mal AG-26 '{"packages":[{"name":"app","dependencies":['"$okrec"']}],"workspace_members":5}' "'workspace_members' is not an array of strings" "workspace_members of the wrong type"

FAKE_META=malformed fakegate g9 nothing 0
if [ "$rc" -eq 1 ] && printf '%s\n' "$out" | grep -q 'cannot be made' && [ "$(wc -l < "$T/g9.log")" -eq 2 ]; then ok "AG-8i malformed metadata (an approved record beside a package with no dependencies array): cannot be made, exit 1, and cargo audit is NOT run"; else bad "AG-8i malformed metadata must stop the gate before cargo audit" "rc=$rc log: $(tr '\n' '|' < "$T/g9.log" | cut -c1-200)"; fi

echo "audit-gate-test: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
