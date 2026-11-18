#!/bin/sh
# Test of scripts/install_mutants.py (the mutation DRIVER of the install lane): when a BASELINE is not green the driver must NAME the failing cell(s), not only the last 8 lines of the suite's output
# (finn's smoke run 04:21Z: one of eight concurrent baselines failed with '193 passed, 1 failed' and no receipt held the cell: the failing line was earlier than the last 8 lines; cox 3bce6367, kai 7e00cb44).
# The driver finds its suite and its deploy/ by its OWN path, so the cells run a COPY of it beside a FAKE suite and a FAKE deploy/ in a temporary directory: the driver is not changed for the test, no hook is added, and
# scripts/install-test.sh, scripts/install-mutants.json and deploy/ are not touched. The suite's failure marker is read from its bad() (install-test.sh): 'FAIL <name>' and ONE indented detail line.
# The SUITE-MUTANT LAYOUT (cox a17af225): the cells of the suite read the repository's crates/ through $here/.. (IT-147, IT-148), so the layout the driver gives a suite mutant must give them the lane's crates/ too (IMD-5, IMD-6).
# The TRANSIENT DIRECTORIES beside the suite (cox 34176243, hue's smoke finding 4): the real run deals the 29 suite ids to EVERY shard of ci-fanout, so a suite mutant copies the SHARED scripts/ while the other shards' install-test.sh create and
# remove their own scripts/.provider.XXXXXX (a fake provider with a symlink under it): a copy that meets a half-built or vanishing one fails ('Too many levels of symbolic links'). The copy ignores .provider.* and .install-mutant-* (IMD-7 .. IMD-10).
# NAMED MUTANTS of the driver (IMD-1 .. IMD-10) are run by this test itself: each must make a cell FAIL; the exactly-once check of each old text comes first.
# Run:  scripts/cv2-admit-shim sh scripts/install-mutants-driver-test.sh
set -eu
here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
driver=${INSTALL_MUTANTS_PY:-"$here/install_mutants.py"}
T=$(mktemp -d)
trap 'rm -rf "$T"' EXIT
pass=0 fail=0
ok() { echo "ok   $1"; pass=$((pass + 1)); }
bad() { echo "FAIL $1"; [ -z "${2:-}" ] || echo "       $2"; fail=$((fail + 1)); }

# a fake suite and a fake deploy/ in $T/d, with a COPY of the driver (argument 1 names the driver file to copy; argument 2 the kind of suite)
# kinds: red1 (ONE failed cell, early in the output, 12 ok lines after it: outside the last 8 lines), red2 (TWO failed cells), layoutred (green when INSTALL_SH names an installer, red in the layout run of a SUITE mutant, which
# runs the suite with no INSTALL_SH), green (no failure), crash (a non-zero exit with no FAIL line), copyread (a suite whose cells read a normal file and a normal hidden directory beside it, and a sentinel a suite mutant flips: it judges what the layout COPIES), cratesread (a suite whose first cell reads crates/marker.txt through $here/.. and whose IT-77 reads a sentinel a suite mutant flips)
mkworld() {
  rm -rf "$T/d"; mkdir -p "$T/d/scripts" "$T/d/deploy"
  cp "$1" "$T/d/scripts/install_mutants.py"
  echo '#!/bin/sh' > "$T/d/deploy/install.sh"
  echo '[]' > "$T/d/scripts/install-mutants.json"
  case $2 in
    red1) printf '%s\n' '#!/bin/sh' 'echo "ok   IT-01 first"' 'echo "FAIL IT-77 the fake cell"' 'echo "       fake detail of IT-77"' 'i=0; while [ "$i" -lt 12 ]; do i=$((i + 1)); echo "ok   IT-9$i filler"; done' 'echo "install-test: 13 passed, 1 failed"' 'exit 1' > "$T/d/scripts/install-test.sh" ;;
    red2) printf '%s\n' '#!/bin/sh' 'echo "FAIL IT-41 the first fake cell"' 'echo "       detail of IT-41"' 'echo "ok   IT-42 between"' 'echo "FAIL IT-43 the second fake cell"' 'echo "       detail of IT-43"' 'i=0; while [ "$i" -lt 12 ]; do i=$((i + 1)); echo "ok   IT-9$i filler"; done' 'echo "install-test: 13 passed, 2 failed"' 'exit 1' > "$T/d/scripts/install-test.sh" ;;
    layoutred) printf '%s\n' '#!/bin/sh' 'if [ -n "${INSTALL_SH:-}" ]; then echo "ok   IT-01 a"; echo "install-test: 1 passed, 0 failed"; exit 0; fi' 'echo "ok   IT-01 a"' 'echo "FAIL IT-77 the fake cell"' 'echo "       fake detail of IT-77"' 'i=0; while [ "$i" -lt 12 ]; do i=$((i + 1)); echo "ok   IT-9$i filler"; done' 'echo "install-test: 13 passed, 1 failed"' 'exit 1' > "$T/d/scripts/install-test.sh" ;;
    copyread) printf '%s\n' '#!/bin/sh' 'here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)' 'sentinel=good' 'n=0; f=0' \
      'if [ "$(cat "$here/keepme.txt" 2>/dev/null)" = kept ]; then echo "ok   IT-01 a normal file beside the suite is copied"; n=$((n + 1)); else echo "FAIL IT-01 a normal file beside the suite is copied"; echo "       keepme.txt is not there"; f=$((f + 1)); fi' \
      'if [ "$(cat "$here/.keepdir/file" 2>/dev/null)" = kept ]; then echo "ok   IT-02 a normal hidden directory beside the suite is copied"; n=$((n + 1)); else echo "FAIL IT-02 a normal hidden directory beside the suite is copied"; echo "       .keepdir/file is not there"; f=$((f + 1)); fi' \
      'if [ "$sentinel" = good ]; then echo "ok   IT-77 the sentinel"; n=$((n + 1)); else echo "FAIL IT-77 the sentinel"; echo "       the sentinel is not good"; f=$((f + 1)); fi' \
      'echo "install-test: $n passed, $f failed"; [ "$f" -eq 0 ]' > "$T/d/scripts/install-test.sh" ;;
    cratesread) printf '%s\n' '#!/bin/sh' 'here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)' 'sentinel=good' 'n=0; f=0' \
      'if [ "$(cat "$here/../crates/marker.txt" 2>/dev/null)" = "crates-ok" ]; then echo "ok   IT-01 reads crates"; n=$((n + 1)); else echo "FAIL IT-01 reads crates"; echo "       crates/marker.txt is not there"; f=$((f + 1)); fi' \
      'if [ "$sentinel" = good ]; then echo "ok   IT-77 the sentinel"; n=$((n + 1)); else echo "FAIL IT-77 the sentinel"; echo "       the sentinel is not good"; f=$((f + 1)); fi' \
      'echo "install-test: $n passed, $f failed"; [ "$f" -eq 0 ]' > "$T/d/scripts/install-test.sh" ;;
    green) printf '%s\n' '#!/bin/sh' 'echo "ok   IT-01 a"' 'echo "ok   IT-02 b"' 'echo "ok   IT-03 c"' 'echo "install-test: 3 passed, 0 failed"' 'exit 0' > "$T/d/scripts/install-test.sh" ;;
    crash) printf '%s\n' '#!/bin/sh' 'echo "ok   IT-01 a"' 'echo "Killed"' 'exit 137' > "$T/d/scripts/install-test.sh" ;;
  esac
}
# drive: run the copied driver (no arguments: the baseline, then every mutant of the spec); sets rc and $T/out
drive() { python3 -I -B "$T/d/scripts/install_mutants.py" > "$T/out" 2>&1 && rc=0 || rc=$?; }

# ---- the cells: each returns a list of what is wrong (empty = pass) for the driver copy in $T/d ---------------------------------------------------------------------------------------------------------
cell_baseline_names_the_cell() {
  w=""
  mkworld "$1" red1; drive
  [ "$rc" = 2 ] || w="$w [a failing baseline: rc=$rc, want 2]"
  grep -q '^BASELINE NOT GREEN' "$T/out" || w="$w [the line BASELINE NOT GREEN is gone]"
  grep -qx 'install-test: 13 passed, 1 failed' "$T/out" || w="$w [the suite's last lines are no longer printed (what the driver prints today stays)]"
  grep -qx 'FAIL IT-77 the fake cell' "$T/out" || w="$w [the failing cell is NOT named in the output: the line 'FAIL IT-77 the fake cell' is missing]"
  grep -qx 'the baseline failed 1 cell(s):' "$T/out" || w="$w [the count line 'the baseline failed 1 cell(s):' is missing]"
  grep -qx '       fake detail of IT-77' "$T/out" || w="$w [the detail line of the failing cell is missing]"
  printf '%s' "$w"
}
cell_two_cells_and_the_count() {
  w=""
  mkworld "$1" red2; drive
  grep -qx 'FAIL IT-41 the first fake cell' "$T/out" && grep -qx 'FAIL IT-43 the second fake cell' "$T/out" || w="$w [not every failing cell is named]"
  grep -qx '       detail of IT-41' "$T/out" && grep -qx '       detail of IT-43' "$T/out" || w="$w [a detail line is missing]"
  grep -qx 'the baseline failed 2 cell(s):' "$T/out" || w="$w [the count of failed cells is missing or wrong (want 'the baseline failed 2 cell(s):')]"
  ! grep -q 'IT-42' "$T/out" || w="$w [a PASSING cell (IT-42) is printed with the failed ones]"
  printf '%s' "$w"
}
cell_suite_mutant_layout_baseline() {
  w=""
  # a SUITE mutant makes the driver run the unmutated suite once in its own layout first: a failure there must name the cell too
  mkworld "$1" layoutred
  printf '[{"id":"X","what":"a suite mutant","file":"scripts/install-test.sh","old":"fake detail","new":"fake detai2","fails":["IT-77"]}]\n' > "$T/d/scripts/install-mutants.json"
  drive
  [ "$rc" = 2 ] || w="$w [a failing LAYOUT baseline: rc=$rc, want 2]"
  grep -q 'fails in the layout used for suite mutants' "$T/out" || w="$w [the layout baseline did not fail where the test expects it]"
  grep -qx 'FAIL IT-77 the fake cell' "$T/out" || w="$w [the failing cell of the layout baseline is NOT named]"
  printf '%s' "$w"
}
cell_crash_says_so() {
  w=""
  mkworld "$1" crash; drive
  [ "$rc" = 2 ] || w="$w [a baseline that crashes: rc=$rc, want 2]"
  grep -q 'holds no failed cell' "$T/out" || w="$w [a suite that stopped without a FAIL line is not said to hold no failed cell]"
  printf '%s' "$w"
}
cell_passing_output_unchanged() {
  w=""
  mkworld "$1" green; drive
  printf '%s\n' 'baseline: the unmutated install.sh passes (3 checks)' 'install-mutants: 0 mutants, 0 not caught or broken' > "$T/expect"
  { [ "$rc" = 0 ] && cmp -s "$T/out" "$T/expect"; } || w="$w [the output of a PASSING baseline changed: rc=$rc: $(tr '\n' '|' < "$T/out" | cut -c1-160)]"
  printf '%s' "$w"
}

cell_suite_mutant_sees_crates() {
  w=""
  # a lane whose suite cells READ crates/ (as IT-147 and IT-148 do): the layout of a suite mutant must give them the lane's crates/, so the unmutated suite is green there and the mutant reaches its verdict (IT-77 alone fails)
  mkworld "$1" cratesread; mkdir -p "$T/d/crates"; printf 'crates-ok' > "$T/d/crates/marker.txt"
  printf '[{"id":"X","what":"a suite mutant","file":"scripts/install-test.sh","old":"sentinel=good","new":"sentinel=evil","fails":["IT-77"]}]\n' > "$T/d/scripts/install-mutants.json"
  drive
  [ "$rc" = 0 ] || w="$w [a suite mutant in a lane whose cells read crates/ did not reach a verdict: rc=$rc: $(grep -m1 -E 'BASELINE|^X ' "$T/out" | cut -c1-110)]"
  grep -Eq '^X +OK \(IT-77\)' "$T/out" || w="$w [the suite mutant was not judged OK by IT-77 alone: $(grep '^X ' "$T/out" | head -1 | cut -c1-100)]"
  grep -qx 'install-mutants: 1 mutants, 0 not caught or broken' "$T/out" || w="$w [the closing line is missing]"
  [ "$(cat "$T/d/crates/marker.txt" 2>/dev/null)" = "crates-ok" ] || w="$w [the lane's own crates/marker.txt was damaged or removed by the run (the temporary layout must not be followed when it is removed)]"
  [ -z "$(find "$T/d" -maxdepth 1 -name '.install-mutant-*' | head -1)" ] || w="$w [a temporary layout directory was left behind]"
  printf '%s' "$w"
}
cell_layout_without_crates_fails_closed() {
  w=""
  # the control: a lane that has NO crates/ must not be made green by the layout: the suite fails closed and its failing cell is NAMED
  mkworld "$1" cratesread
  printf '[{"id":"X","what":"a suite mutant","file":"scripts/install-test.sh","old":"sentinel=good","new":"sentinel=evil","fails":["IT-77"]}]\n' > "$T/d/scripts/install-mutants.json"
  drive
  [ "$rc" = 2 ] || w="$w [a lane without crates/: rc=$rc, want 2]"
  grep -q '^BASELINE NOT GREEN' "$T/out" || w="$w [a lane without crates/ was judged green]"
  grep -qx 'FAIL IT-01 reads crates' "$T/out" || w="$w [the failing cell of a lane without crates/ is NOT named]"
  printf '%s' "$w"
}

cell_suite_mutant_survives_transient_dirs() {
  w=""
  # a transient directory of ANOTHER shard beside the lane's scripts/ (a half-built fake provider: a symlink LOOP and a DANGLING link): the copy of scripts/ must ignore it, the suite mutant must reach its verdict, the directory must be left exactly as it was
  for tname in .provider.AbC123 .install-mutant-Zed789; do
    mkworld "$1" copyread; mkdir -p "$T/d/crates"
    printf 'kept' > "$T/d/scripts/keepme.txt"; mkdir "$T/d/scripts/.keepdir"; printf 'kept' > "$T/d/scripts/.keepdir/file"
    mkdir "$T/d/scripts/$tname"; ln -s loop "$T/d/scripts/$tname/loop"; ln -s /nonexistent-nemo-link-target "$T/d/scripts/$tname/dangling"
    printf '[{"id":"X","what":"a suite mutant","file":"scripts/install-test.sh","old":"sentinel=good","new":"sentinel=evil","fails":["IT-77"]}]\n' > "$T/d/scripts/install-mutants.json"
    drive
    [ "$rc" = 0 ] || w="$w [$tname: a suite mutant beside a transient directory did not reach a verdict: rc=$rc: $(grep -m1 -E 'BASELINE|Error|Traceback|^X ' "$T/out" | cut -c1-120)]"
    grep -Eq '^X +OK \(IT-77\)' "$T/out" || w="$w [$tname: the suite mutant was not judged OK by IT-77 alone (the normal file and the normal hidden directory must still be copied): $(grep '^X ' "$T/out" | head -1 | cut -c1-110)]"
    { [ -L "$T/d/scripts/$tname/loop" ] && [ -L "$T/d/scripts/$tname/dangling" ]; } || w="$w [$tname: the other shard's directory was changed by the run]"
  done
  printf '%s' "$w"
}

all_cells() { # all_cells <driver file>: prints the problems of every cell (empty = all pass)
  p=""
  for c in cell_baseline_names_the_cell cell_two_cells_and_the_count cell_suite_mutant_layout_baseline cell_crash_says_so cell_passing_output_unchanged cell_suite_mutant_sees_crates cell_layout_without_crates_fails_closed cell_suite_mutant_survives_transient_dirs; do
    r=$($c "$1"); [ -z "$r" ] || p="$p <$c:$r>"
  done
  printf '%s' "$p"
}

# ---- the cells against the driver under test ---------------------------------------------------------------------------------------------------------------------------------------------------------------
res=$(all_cells "$driver")
if [ -z "$res" ]; then ok "IT-D01 a failing baseline names its failing cell(s) with their detail, in the baseline and in the suite-mutant layout; a crash is said to name no cell; the passing output is unchanged; a suite mutant of a lane whose cells read crates/ reaches its verdict in the driver's layout, and a lane WITHOUT crates/ still fails closed with the named cell; a suite mutant beside another shard's transient .provider.* or .install-mutant-* directory (a symlink loop and a dangling link) reaches its verdict while a normal file and a normal hidden directory are still copied"; else bad "IT-D01 the driver names the failing cell" "$res"; fi

# ---- the NAMED MUTANTS of the driver: the exactly-once check of each old text FIRST, then each must make the cells fail ---------------------------------------------------------------------------------------
# IMD-1 the failure marker is never matched (nothing is printed); IMD-2 the detail line is dropped; IMD-3 the baseline site prints nothing; IMD-4 the layout-baseline site prints nothing; IMD-5 the layout of a suite mutant is not given crates/; IMD-6 it looks for crates/ under another name;
# IMD-7 the copy of scripts/ ignores nothing; IMD-8 its .provider.* pattern is misspelt; IMD-9 its .install-mutant-* pattern is misspelt; IMD-10 it ignores every hidden entry
cat > "$T/mutants.txt" <<'EOF'
IMD-1|        if line.startswith("FAIL "):|        if line.startswith("FAIL-NEVER "):
IMD-2|            while j < len(lines) and lines[j].startswith("       "):|            while j < len(lines) and lines[j].startswith("NEVER"):
IMD-3|        print_failed_cells(out)|        pass
IMD-4|            print_failed_cells(lout)|            pass
IMD-5|        os.symlink(crates_src, os.path.join(d, "crates"))|        pass
IMD-6|crates_src = os.path.join(os.path.dirname(HERE), "crates")|crates_src = os.path.join(os.path.dirname(HERE), "crates2")
IMD-7|, ignore=shutil.ignore_patterns(".provider.*", ".install-mutant-*"))|)
IMD-8|".provider.*"|".providers.*"
IMD-9|".install-mutant-*"|".install-mutants-*"
IMD-10|ignore=shutil.ignore_patterns(".provider.*", ".install-mutant-*")|ignore=shutil.ignore_patterns(".*")
EOF
nm=0; nkill=0
while IFS='|' read -r mid old new; do
  nm=$((nm + 1))
  n=$(python3 -I -c 'import sys; print(open(sys.argv[1], encoding="utf-8").read().count(sys.argv[2]))' "$driver" "$old")
  if [ "$n" != 1 ]; then bad "IT-D02 $mid" "NOT APPLIED: the old text occurs $n times in the driver, want exactly 1"; continue; fi
  python3 -I -c 'import sys; t = open(sys.argv[1], encoding="utf-8").read(); open(sys.argv[2], "w", encoding="utf-8").write(t.replace(sys.argv[3], sys.argv[4]))' "$driver" "$T/mutant.py" "$old" "$new"
  python3 -I -c 'import ast, sys; ast.parse(open(sys.argv[1], encoding="utf-8").read())' "$T/mutant.py" 2>/dev/null || { bad "IT-D02 $mid" "BROKEN: the mutant is a syntax error (it proves nothing)"; continue; }
  mres=$(all_cells "$T/mutant.py")
  if [ -n "$mres" ]; then nkill=$((nkill + 1)); echo "       $mid killed by: $(printf '%s' "$mres" | grep -o '<cell_[a-z_]*' | tr -d '<' | tr '\n' ' ')"; else bad "IT-D02 $mid" "SURVIVED: the cells pass although the driver does not print the failing cell"; fi
done < "$T/mutants.txt"
[ "$nkill" = "$nm" ] && ok "IT-D02 the named mutants of the driver (IMD-1 .. IMD-10: no marker match, no detail line, no print at the baseline site, none at the layout-baseline site, no crates/ in the suite-mutant layout, crates/ looked for under another name, nothing ignored, each ignore pattern misspelt, every hidden entry ignored): each applies exactly once and each is killed ($nkill of $nm)"

echo "install-mutants-driver-test: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
