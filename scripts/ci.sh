#!/bin/sh
# CI gates for cortex-v2 (S5). Run from anywhere; it works from the repository root.
#
#   scripts/ci.sh [--text-only] [--strict]      run the gates and print one line each
#   scripts/ci.sh --selftest-gates              check the gate logic itself (blocking vs report-only), no cargo
#
#   --text-only  run only what starts no build: the S5 checker, its self-test, shell syntax. Wave 0 uses this.
#   --strict     a gate that is NOT_RUN counts as a failure. Default: it is named in the table and never shown green.
#
# A gate command may exit 77 to say NOT_RUN, but ONLY with a last output line of the exact form 'NOT_RUN: <reason>'; a 77 without it is a FAILURE (cox W1-106 A2).
# The TEST gate (scripts/test-gate/test-gate.sh) runs EVERY workspace test: the DB-free driver, then the database tests one package at a time under supervise.sh, each through
# its own admission. Start ci.sh itself NOT under an outer cv2-admit: nested, the database drivers refuse (the VM-wide one-database lock would be skipped) and the test gate is NOT_RUN.
#
# BLOCKING: fmt, clippy, tests, cargo deny (advisories, bans, sources), cargo audit, xtask fitness, the S5 checks, shell syntax,
#           and the secret scan once a scanner is installed.
# REPORT-ONLY (D94, CTO 5 Nov: licensing is not a gate until the January commercial release): cargo deny licenses and
#           xtask licences. Their output is kept in the log; their exit status is ignored. The register is the work list.
# Everything that runs goes through scripts/cv2-admit-shim: admitted on build-host-1, a plain call anywhere else.
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
admit="$root/scripts/cv2-admit-shim"
log_dir=${CI_LOG_DIR:-"$root/target/ci"}
deploy_dir=${CI_DEPLOY_DIR:-"$root/deploy"}

mode=run text_only=0 strict=0
for a in "$@"; do
  case $a in
    --text-only) text_only=1 ;;
    --strict) strict=1 ;;
    --selftest-gates) mode=selftest ;;
    -h|--help) sed -n '2,14p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "ci: unknown argument $a" >&2; exit 2 ;;
  esac
done
mkdir -p "$log_dir"

results="" failed=0 notrun=0
record() { results="$results$1|$2|$3
"; }
# gate <name> <blocking|report> <command...>
gate() {
  name=$1 kind=$2; shift 2
  out="$log_dir/$name.log"
  if "$@" >"$out" 2>&1; then
    record "$name" PASS ""
  else
    rc=$?
    # cox W1-106 (A2): exit 77 means NOT_RUN, but ONLY with a last output line of the exact form 'NOT_RUN: <reason>'; a 77 without it, or the text with another exit
    # code, is a failure, so a stray 77 or a stray line can never turn a failure into a skip
    reason=$(tail -n 1 "$out" | sed -n 's/^NOT_RUN: \(.*[^[:space:]].*\)$/\1/p')
    if [ "$rc" -eq 77 ] && [ -n "$reason" ]; then
      skip "$name" "$reason"
    elif [ "$kind" = report ]; then
      record "$name" REPORT "command failed; kept in $out"
    else
      record "$name" FAIL "see $out"; failed=1
    fi
  fi
}
skip() { record "$1" NOT_RUN "$2"; notrun=$((notrun + 1)); }
table() {
  printf '%s' "$results" | while IFS='|' read -r n s note; do
    [ -n "$n" ] && printf '%-16s %-8s %s\n' "$n" "$s" "$note"
  done
}

# verdict <failed> <notrun> <strict>: the last line. A gate that did not run is never reported as a plain pass.
verdict() {
  if [ "$1" -ne 0 ]; then echo "ci: FAIL"; return 1; fi
  if [ "$2" -ne 0 ] && [ "$3" -eq 1 ]; then echo "ci: FAIL (--strict and a gate is NOT_RUN)"; return 1; fi
  if [ "$2" -ne 0 ]; then echo "ci: PASS-WITH-NOT-RUN ($2 gates did not run; see NOT_RUN above)"; return 0; fi
  echo "ci: PASS"
}

# pick_proof <dir>: the transcript whose status decides = the NEWEST roles-proof-run*.txt by modification time, except those
# named *-RED.txt (intentional runs against a broken script). A newer failing run therefore decides, not the older green one
# (nix N32). Prints nothing when there is none.
pick_proof() { ls -t "$1"/roles-proof-run*.txt 2>/dev/null | grep -v -- '-RED\.txt$' | head -1 || true; }

# proof_status <transcript> <db-roles.sh sha256> <roles script sha256> <initdb-args sha256>: is the last LIVE roles proof for the CURRENT bytes? (nix N27)
proof_status() {
  [ -f "$1" ] || { echo "no live result recorded"; return 0; }
  res=$(grep -E '^roles-proof: [0-9]+ passed, [0-9]+ failed' "$1" | tail -1)
  [ -n "$res" ] || { echo "the last transcript has no result line"; return 0; }
  case $res in *", 0 failed"*) ;; *) echo "last live run FAILED ($res)"; return 0 ;; esac
  exits=$(grep -ciE '^[[:space:]]*#[[:space:]]*exit[[:space:]]*:' "$1" || true)   # any spelling of an exit line (hale N-1)
  [ "$exits" -le 1 ] || { echo "last live run FAILED ($res; more than one exit line)"; return 0; }       # the real flow writes one (hale N-1, N34)
  grep -qxF '# exit: 0' "$1" || { echo "last live run FAILED ($res; exit line is not 0)"; return 0; }   # the run's own status (N32)
  passed=${res#roles-proof: }; passed=${passed%% passed*}
  [ "$passed" -gt 0 ] 2>/dev/null || { echo "last live run ran nothing ($res)"; return 0; }            # N30
  for pair in "db-roles.sh $2" "00-cortex-roles.sh $3" "initdb-args $4"; do
    grep -qxF "proof-under-test $pair" "$1" || { echo "STALE: ${pair%% *} changed since the last live run ($res)"; return 0; }
  done
  echo "last live run matches the current scripts ($res)"
}

if [ "$mode" = selftest ]; then
  # The gate logic must: pass a passing command, fail a failing blocking one, and only REPORT a failing report-only one.
  gate st-pass blocking true
  gate st-block blocking false
  gate st-report report false
  # cox W1-106 (A2): exit 77 is NOT_RUN only with a last line 'NOT_RUN: <reason>'; no line, a line that is not last, an empty reason, or the line with another exit code is a FAILURE
  gate st-notrun blocking sh -c 'echo working; echo "NOT_RUN: no podman here"; exit 77'
  gate st-77none blocking sh -c 'exit 77'
  gate st-77notlast blocking sh -c 'echo "NOT_RUN: a reason"; echo "and then something else"; exit 77'
  gate st-77empty blocking sh -c 'echo "NOT_RUN:    "; exit 77'
  gate st-1text blocking sh -c 'echo "NOT_RUN: pretending"; exit 1'
  gate st-77tab blocking sh -c 'printf "NOT_RUN: \t\n"; exit 77'
  gate st-77mixed blocking sh -c 'printf "NOT_RUN:  \t \t\n"; exit 77'
  want="st-pass|PASS|
st-block|FAIL|see $log_dir/st-block.log
st-report|REPORT|command failed; kept in $log_dir/st-report.log
st-notrun|NOT_RUN|no podman here
st-77none|FAIL|see $log_dir/st-77none.log
st-77notlast|FAIL|see $log_dir/st-77notlast.log
st-77empty|FAIL|see $log_dir/st-77empty.log
st-1text|FAIL|see $log_dir/st-1text.log
st-77tab|FAIL|see $log_dir/st-77tab.log
st-77mixed|FAIL|see $log_dir/st-77mixed.log
"
  # The verdict line must never read as a plain pass when a gate did not run (nix N2).
  v1=$(verdict 0 0 0) || true; v2=$(verdict 0 3 0) || true; v3=$(verdict 1 0 0) || true; v4=$(verdict 0 3 1) || true
  want_v="ci: PASS|ci: PASS-WITH-NOT-RUN (3 gates did not run; see NOT_RUN above)|ci: FAIL|ci: FAIL (--strict and a gate is NOT_RUN)"
  # The proof status must say whether the last live result is for the CURRENT scripts (nix N27).
  pt=$(mktemp -d)
  printf 'proof-under-test db-roles.sh AAA\nproof-under-test 00-cortex-roles.sh BBB\nproof-under-test initdb-args CCC\nroles-proof: 90 passed, 0 failed\n# exit: 0\n' > "$pt/ok.txt"
  printf 'proof-under-test db-roles.sh AAA\nproof-under-test 00-cortex-roles.sh BBB\nproof-under-test initdb-args CCC\nroles-proof: 88 passed, 2 failed\n# exit: 1\n' > "$pt/bad.txt"
  s1=$(proof_status "$pt/ok.txt" AAA BBB CCC) || true
  s2=$(proof_status "$pt/ok.txt" XXX BBB CCC) || true
  s3=$(proof_status "$pt/ok.txt" AAA BBB ZZZ) || true
  s4=$(proof_status "$pt/bad.txt" AAA BBB CCC) || true
  s5=$(proof_status "$pt/none.txt" AAA BBB CCC) || true
  # nix N30: a run that ran nothing; N31: the hash lines only inside comment lines; N32: a run whose own exit line is not 0
  hashes='proof-under-test db-roles.sh AAA\nproof-under-test 00-cortex-roles.sh BBB\nproof-under-test initdb-args CCC\n'
  printf "${hashes}roles-proof: 0 passed, 0 failed\n# exit: 0\n" > "$pt/zero.txt"
  printf '# note: proof-under-test db-roles.sh AAA\n# note: proof-under-test 00-cortex-roles.sh BBB\n  proof-under-test initdb-args CCC\nroles-proof: 90 passed, 0 failed\n# exit: 0\n' > "$pt/commented.txt"
  printf "${hashes}roles-proof: 90 passed, 0 failed\n# exit: 1\n" > "$pt/exit1.txt"
  printf "${hashes}roles-proof: 90 passed, 0 failed\n" > "$pt/noexit.txt"
  s6=$(proof_status "$pt/zero.txt" AAA BBB CCC) || true
  s7=$(proof_status "$pt/commented.txt" AAA BBB CCC) || true
  s8=$(proof_status "$pt/exit1.txt" AAA BBB CCC) || true
  s9=$(proof_status "$pt/noexit.txt" AAA BBB CCC) || true
  # hale N-1 (N34): the real flow writes ONE exit line; two of them, in any order, are not a transcript to trust
  printf "${hashes}roles-proof: 90 passed, 0 failed\n# exit: 0\n# exit: 1\n" > "$pt/exit01.txt"
  printf "${hashes}roles-proof: 90 passed, 0 failed\n# exit: 1\n# exit: 0\n" > "$pt/exit10.txt"
  printf "${hashes}roles-proof: 90 passed, 0 failed\n# exit: 0\n# exit: 0\n" > "$pt/exit00.txt"
  printf "${hashes}roles-proof: 90 passed, 0 failed\n# exit: 0\n# exit:1\n" > "$pt/exit0b.txt"
  printf "${hashes}roles-proof: 90 passed, 0 failed\n# exit: 0\n# EXIT: 1\n" > "$pt/exit0c.txt"
  printf "${hashes}roles-proof: 90 passed, 0 failed\n# exit: 0\n#exit : 1\n" > "$pt/exit0d.txt"
  s14=$(proof_status "$pt/exit0b.txt" AAA BBB CCC) || true
  s15=$(proof_status "$pt/exit0c.txt" AAA BBB CCC) || true
  s16=$(proof_status "$pt/exit0d.txt" AAA BBB CCC) || true
  s11=$(proof_status "$pt/exit01.txt" AAA BBB CCC) || true
  s12=$(proof_status "$pt/exit10.txt" AAA BBB CCC) || true
  s13=$(proof_status "$pt/exit00.txt" AAA BBB CCC) || true
  # nix N32: the LAST run decides, a deliberate RED run is skipped, and nothing is nothing
  mkdir "$pt/runs" "$pt/empty"
  cp "$pt/ok.txt" "$pt/runs/roles-proof-run9.txt"; cp "$pt/bad.txt" "$pt/runs/roles-proof-run10.txt"; cp "$pt/bad.txt" "$pt/runs/roles-proof-run11-RED.txt"
  touch -d '2026-11-01 10:00:00' "$pt/runs/roles-proof-run9.txt"; touch -d '2026-11-01 11:00:00' "$pt/runs/roles-proof-run10.txt"; touch -d '2026-11-01 12:00:00' "$pt/runs/roles-proof-run11-RED.txt"
  p1=$(pick_proof "$pt/runs" | sed "s|$pt/||")
  s10=$(proof_status "$pt/$p1" AAA BBB CCC) || true
  touch -d '2026-11-01 13:00:00' "$pt/runs/roles-proof-run9.txt"
  p2=$(pick_proof "$pt/runs" | sed "s|$pt/||")
  p3=$(pick_proof "$pt/empty")
  rm -rf "$pt"
  want_s="last live run matches the current scripts (roles-proof: 90 passed, 0 failed)|STALE: db-roles.sh changed since the last live run (roles-proof: 90 passed, 0 failed)|STALE: initdb-args changed since the last live run (roles-proof: 90 passed, 0 failed)|last live run FAILED (roles-proof: 88 passed, 2 failed)|no live result recorded"
  want_s2="last live run ran nothing (roles-proof: 0 passed, 0 failed)|STALE: db-roles.sh changed since the last live run (roles-proof: 90 passed, 0 failed)|last live run FAILED (roles-proof: 90 passed, 0 failed; exit line is not 0)|last live run FAILED (roles-proof: 90 passed, 0 failed; exit line is not 0)|last live run FAILED (roles-proof: 88 passed, 2 failed)"
  want_p="runs/roles-proof-run10.txt|runs/roles-proof-run9.txt|"
  one="last live run FAILED (roles-proof: 90 passed, 0 failed; more than one exit line)"
  want_s3="$one|$one|$one|$one|$one|$one"
  if [ "$results" = "$want" ] && [ "$failed" -eq 1 ] && [ "$notrun" -eq 1 ] && [ "$v1|$v2|$v3|$v4" = "$want_v" ] && [ "$s1|$s2|$s3|$s4|$s5" = "$want_s" ] \
     && [ "$s6|$s7|$s8|$s9|$s10" = "$want_s2" ] && [ "$p1|$p2|$p3" = "$want_p" ] && [ "$s11|$s12|$s13|$s14|$s15|$s16" = "$want_s3" ]; then echo "ci: gate logic ok (10 gate cases, 4 verdict cases, 16 proof-status cases, 3 selection cases)"; exit 0; fi
  echo "ci: GATE LOGIC WRONG" >&2; table >&2; echo "verdicts: $v1|$v2|$v3|$v4" >&2; echo "proof status: $s1|$s2|$s3|$s4|$s5" >&2; echo "more: $s6|$s7|$s8|$s9|$s10" >&2; echo "picked: $p1|$p2|$p3" >&2; echo "exit lines: $s11|$s12|$s13|$s14|$s15|$s16" >&2; exit 1
fi

# ---- always: nothing heavy ---------------------------------------------------------------------------------------
gate shell-syntax blocking sh -c 'for f in scripts/ci.sh scripts/audit-gate.sh scripts/audit-gate-test.sh scripts/s5-check.sh scripts/install-test.sh scripts/install-fetch-test.sh deploy/install-fetch.sh scripts/shim-test.sh scripts/roles-proof.sh scripts/install-witness.sh scripts/witness-test.sh scripts/shell-mutants-test.sh scripts/verify-units.sh scripts/verify-units-test.sh scripts/cv2-admit-shim scripts/test-gate/test-gate.sh scripts/test-gate/test-gate-test.sh scripts/test-gate/fakes/cargo scripts/test-gate/fakes/admit scripts/test-gate/fakes/supervise deploy/install.sh deploy/initdb/*.sh mac/install-mac.sh mac/install-prereq.sh mac/install-fetch.sh scripts/install-mac-test.sh; do sh -n "$f" || exit 1; done'
gate s5-selftest blocking "$admit" sh scripts/s5-check.sh --self-test
gate s5-deploy blocking "$admit" sh scripts/s5-check.sh "$deploy_dir"
gate s5-never-scripts blocking "$admit" sh scripts/s5-check.sh --never-list scripts
gate gate-logic blocking sh scripts/ci.sh --selftest-gates
gate install-test blocking "$admit" sh scripts/install-test.sh
gate install-prereq-test blocking "$admit" sh scripts/install-prereq-test.sh
skip prerequisite-IP64 "joined prerequisite suite is not implemented (IP-64)"
gate install-prereq-mutants blocking "$admit" env PREREQ_MUTANT_FAST=1 python3 -I -B scripts/install_prereq_mutants.py
gate shim-test blocking "$admit" sh scripts/shim-test.sh
gate witness-test blocking "$admit" sh scripts/witness-test.sh
gate gate-runner-test blocking "$admit" sh scripts/install-gate/gate-runner-test.sh
gate witness-mutants blocking "$admit" python3 -I -B scripts/witness_mutants.py
gate install-mutants blocking sh scripts/ci-fanout.sh --shards 10 --spec scripts/install-mutants.json -- "$admit" python3 -I -B scripts/install_mutants.py
gate test-gate-test blocking "$admit" sh scripts/test-gate/test-gate-test.sh
gate test-gate-mutants blocking "$admit" python3 -I -B scripts/shell_mutants.py scripts/test-gate/test-gate-mutants.json
gate install-fetch-test blocking "$admit" sh scripts/install-fetch-test.sh
# THE MAC GATES (the Mac installer, kai 7a79e791): they run here, on Linux, against FAKE Mac tools (a fake docker, GNU wrappers, a fake BSD stat): a green is "built and run on Linux stand-ins", NEVER "proved on a Mac" (the Mac kit and
# the CTO's own run are what show a real Mac). Each prints its own limit. The compose lane's and the port lane's suites run only where their lanes are merged into this tree: otherwise NOT_RUN with the reason, never green by omission.
gate install-mac-test blocking "$admit" python3 -I -B scripts/mac-presign-check.py
gate mac-release-test blocking "$admit" env TMPDIR="$(CDPATH= cd -- "$log_dir" && pwd)" sh scripts/mac-release-test.sh
gate release-oci-test blocking "$admit" python3 -I -B scripts/release-fixtures/test_oci_descriptor.py
gate release-ci-test blocking "$admit" python3 -I -B scripts/release-fixtures/test_ci_allowlist.py
gate release-ci-policy-test blocking "$admit" python3 -I -B scripts/release-fixtures/test_ci_policy_022.py
gate mac-presign-check-test blocking "$admit" python3 -I -B scripts/mac-release-fixtures/test_mac_presign.py
gate mac-postsign-check-test blocking "$admit" python3 -I -B scripts/mac-release-fixtures/test_mac_postsign.py
gate mac-publish-core-test blocking "$admit" python3 -I -B scripts/mac-release-fixtures/test_mac_publish_core.py
gate release-w1284-mutants blocking "$admit" python3 -I -B scripts/release-fixtures/test_w1284_mutants.py
gate mac-publish-syntax blocking bash -n scripts/mac-publish-core.sh
gate mac-receipt-test blocking "$admit" python3 -I -B scripts/mac-release-fixtures/test_build_receipt.py
gate mac-notes-url-test blocking "$admit" env TMPDIR="$(CDPATH= cd -- "$log_dir" && pwd)" python3 -I -B scripts/mac-release-fixtures/test_mac_notes_url.py
gate install-mac-mutants blocking sh scripts/ci-fanout.sh --shards 6 --spec scripts/install-mac-mutants.json -- "$admit" python3 -I -B scripts/install_mac_mutants.py
gate mac-port-scan blocking "$admit" python3 -I scripts/mac-port-scan --strict mac/install-mac.sh mac/install-prereq.sh mac/install-fetch.sh scripts/install-mac-test.sh
if [ -f scripts/mac-port-test.sh ]; then gate mac-port-test blocking "$admit" sh scripts/mac-port-test.sh; else skip mac-port-test "the mac-port lane (scripts/mac-port-test.sh) is not merged into this tree"; fi
if [ -f scripts/docker-compose-test.sh ]; then gate docker-compose-test blocking "$admit" sh scripts/docker-compose-test.sh; else skip docker-compose-test "the mac-docker lane (scripts/docker-compose-test.sh) is not merged into this tree"; fi
gate install-fetch-mutants blocking env INSTALL_FETCH_NAMED=1 INSTALL_FETCH_TIMEOUT=2400 sh scripts/ci-fanout.sh --shards 6 --spec scripts/install-fetch-mutants.json -- "$admit" python3 -I -B scripts/install_fetch_mutants.py
gate ci-mutants blocking "$admit" python3 -I -B scripts/shell_mutants.py scripts/ci-mutants.json
gate shell-mutants-test blocking "$admit" sh scripts/shell-mutants-test.sh
gate verify-units-test blocking "$admit" sh scripts/verify-units-test.sh
gate verify-units-mutants blocking "$admit" python3 -I -B scripts/shell_mutants.py scripts/verify-units-mutants.json

# ---- builds: skipped in text-only mode ---------------------------------------------------------------------------
if [ "$text_only" -eq 1 ]; then
  for g in fmt clippy test deny-advisories deny-licences audit audit-gate-test fitness xtask-licences; do skip "$g" "text-only: this gate starts a build"; done
else
  gate fmt blocking "$admit" cargo fmt --all -- --check
  gate clippy blocking "$admit" cargo clippy --workspace --all-targets -- -D warnings
  # cox W1-103 (2): EVERY workspace test runs. One script: the DB-free driver (one admission, bounded), the database tests classed by their own panic and run one package at a
  # time under supervise.sh, each through its OWN admission (so NOT under an outer one: a nested call skips the VM-wide database lock and the script refuses with NOT_RUN, exit 77).
  gate test blocking sh scripts/test-gate/test-gate.sh
  gate deny-advisories blocking "$admit" cargo deny check advisories bans sources
  gate deny-licences report "$admit" cargo deny check licenses
  # kai's decision (A), 10 Nov 2026 (cox 8a0cff7a): .cargo/audit.toml ignores ONE advisory only while its crate is in no build. The audit gate is the PROOF of that, then cargo audit (scripts/audit-gate.sh); the cell
  # that makes the proof fail on a planted case, and checks the file and this wiring, is its own gate: a test no gate runs protects nothing.
  gate audit blocking "$admit" sh scripts/audit-gate.sh
  gate audit-gate-test blocking "$admit" sh scripts/audit-gate-test.sh
  gate fitness blocking "$admit" cargo run -p xtask -- fitness
  gate xtask-licences report "$admit" cargo run -p xtask -- licences
fi

# ---- secret scan: never green by omission (LAYOUT section 6) -----------------------------------------------------
# the roles proof needs a real database: it is run by hand and named here so the table never looks complete without it (nix N22)
proof_dir=${CI_PROOF_DIR:-/path/to/docs/mvp/S5-evidence}
proof_file=$(pick_proof "$proof_dir")
pstat=$(proof_status "$proof_file" "$(sha256sum deploy/db-roles.sh | cut -d' ' -f1)" "$(sha256sum deploy/initdb/00-cortex-roles.sh | cut -d' ' -f1)" \
  "$(python3 -I -B -c "import sys, yaml; print(yaml.safe_load(open(sys.argv[1]))['services']['db']['environment']['POSTGRES_INITDB_ARGS'], end='')" deploy/compose.yml | sha256sum | cut -d' ' -f1)")
skip witness-live "needs a running stack (Wave 2, G-S5-2): install.sh on a throwaway stack, then witness_container on the server and the database. No live result recorded."
skip roles-proof "needs a database: run by hand, CV2_DB=1 cv2-admit sh scripts/roles-proof.sh. $pstat"

gate secret-scan blocking "$admit" sh scripts/ci-secret-scan.sh
gate secret-scan-pin-test blocking "$admit" python3 -I -B scripts/release-fixtures/test_gitleaks_pin.py

table
verdict "$failed" "$notrun" "$strict"
