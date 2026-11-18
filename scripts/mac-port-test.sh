#!/bin/sh
# mac-port-test.sh: the tests of M-1a TOOLING, the static portability report and the bash-compat runner (bob; cox rows e6aed96f, 5f3cd001 and
# b911a172, kai rows c12581a0 and efcb10fe). Plan: mvp/pieces/Mac-M1a-tooling-PLAN-r0.md.
#
#   scripts/mac-port-test.sh [--shell sh|posix] [PS-nn ...]
#
#   --shell sh      run the shell tools under `sh` (default; /bin/sh is bash in POSIX mode on this VM)
#   --shell posix   run the shell tools under `bash --posix`
#   no ids          run every cell; with ids, only those
#
# The first line this script prints is the LIMIT of the tooling (kai's condition, row efcb10fe), verbatim. NOTHING here is real: no bash 3.2 is built or
# run, no installer file is executed, no systemd tool is called. The scanner only READS the files it is given; the runner runs only the fixtures
# of scripts/mac-port-fixtures/. Run it through cv2-admit: `cv2-admit scripts/mac-port-test.sh`. Each cell builds its own directory under
# target/ps-test; the run removes those directories, and only those, by their exact names at the end.
# ENVIRONMENT HYGIENE (hale MP-1): an exported shell function would replace a command of this script (PS-26 runs it nested with exported printf, echo and [). Run it with sh, as the first line says:
# run as bash (not in POSIX mode) an exported unset or set would defeat the block below. Every function the environment brings is unset FIRST: bash exports one as
# BASH_FUNC_<name>%% (the form of every bash since the Shellshock fix, Apple's 3.2.57 included; a bash older than that exports <name>=() {, which this block does not undo:
# ASK-A-MAC, and no such bash is on a supported Mac). The listing must WORK: if env or sed failed, every exported function would stay in place, so it refuses.
unset -f unset command env sed 2> /dev/null || :
# a PATH with an empty or a relative element would let a command be found in the working directory, and the block below looks env and sed up through the PATH: so this comes
# FIRST (hale MP-2, M2-1), with case and command printf only (nothing is looked up). ":$PATH:" makes an empty element "::", which this pattern also matches (a colon followed by
# anything but a slash), so one pattern covers both.
case ":$PATH:" in
  *:[!/]*) command printf '%s: problem: PATH has an empty or a relative element, so a command could be found in the working directory; fix: run it with a PATH of absolute directories only (no empty element, no . and no ~)\n' mac-port-test >&2; exit 2 ;;
esac
_pass=1
while :; do
  _fe=$(command env) || { command printf '%s: problem: cannot list the environment to remove the exported shell functions (env failed); fix: run it with a PATH that has a working env and sed\n' mac-port-test >&2; exit 2; }
  _fnames=$(command printf '%s\n' "$_fe" | command sed -n -e 's/^BASH_FUNC_\(.*\)%%=.*/\1/p') || { command printf '%s: problem: cannot list the environment to remove the exported shell functions (sed failed); fix: run it with a PATH that has a working env and sed\n' mac-port-test >&2; exit 2; }
  case $_pass in 2) break ;; esac
  set -f
  for _fn in $_fnames; do
    unset -f "$_fn" 2> /dev/null || :
  done
  set +f
  _pass=2
done
# the second listing is what is LEFT: an entry this shell could not import (bash --posix refuses a function named [) cannot be unset, stays in the environment, and every child
# shell would import it (sam M1A-FU-SAM01): refused. (case, not [, in this block: a [ that was imported must not run before it is removed)
case $_fnames in
  '') ;;
  *)
    set -f
    command printf '%s: problem: the environment holds exported shell functions that this shell cannot remove (%s), and a child shell would import them; fix: unset them in the calling shell (unset -f <name>) or start it with sh\n' mac-port-test "$(command printf '%s ' $_fnames)" >&2
    set +f
    exit 2
    ;;
esac
unset _fn _fe _fnames _pass
set -eu

LIMIT="catches the known 3.2 differences; NOT the real /bin/bash of a Mac, Apple's 3.2.57 with its own patches; proves nothing about what is not listed"
printf '%s\n' "$LIMIT"

here=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
scan=$here/mac-port-scan
runner=$here/mac-port-run
fx=$here/mac-port-fixtures
rules=$fx/rules.txt
deploy=$here/../deploy
base=${CORTEX_PS_TMP:-"$here/../target/ps-test"}

sut=sh
want=""
while [ $# -gt 0 ]; do
  case $1 in
    --shell)
      shift
      case ${1:-} in
        sh) sut=sh ;;
        posix) sut="bash --posix" ;;
        *) echo "mac-port-test: --shell takes sh or posix" >&2; exit 2 ;;
      esac
      ;;
    PS-*) want="$want $1" ;;
    -h | --help) sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
    *) echo "mac-port-test: unknown argument $1" >&2; exit 2 ;;
  esac
  shift
done
LC_ALL=C
export LC_ALL
mkdir -p "$base"
base=$(CDPATH= cd -P -- "$base" && pwd -P)
made=""

# ---- helpers ---------------------------------------------------------------------------------------------

need() { # need <message> <test args...>: a failed check names itself and fails the cell
  _m=$1
  shift
  if "$@"; then return 0; fi
  printf '    CHECK FAILED: %s\n' "$_m" >&2
  return 1
}
run() { # run <name> <command...>: status in $rc, output in $T/<name>.out and .err
  _n=$1
  shift
  rc=0
  "$@" > "$T/$_n.out" 2> "$T/$_n.err" || rc=$?
}
new_test() {
  T=$(mktemp -d "$base/t.XXXXXX")
  made="$made $T"
  mkdir -p "$T/w" "$T/tmp"
}
cleanup_made() { # remove the directories THIS run made, by their exact names, shape-checked
  for _d in $made; do
    case $_d in "$base"/t.??????) rm -rf -- "$_d" ;; esac # s5-never: ok the exact directories this run made, shape-checked
  done
}
sc() { # sc <name> <args...>: the scanner, run with the cwd of the cell's empty work directory
  _n=$1
  shift
  rc=0
  (cd "$T/w" && python3 -I "$scan" "$@") > "$T/$_n.out" 2> "$T/$_n.err" || rc=$?
}
# lines <name>: the lines reported by the last scan <name> for a rule, as "<line>:<rule>" (text mode), sorted
reported() { # reported <out file>: "line:rule" pairs of finding lines (not NOTE lines), sorted numerically
  sed -n 's/^[^ ]*:\([0-9][0-9]*\): \(error\|warn\|info\) \(MP-[0-9][0-9]*\) .*/\1:\3/p' "$1" | sort -t: -k1,1n -k2,2
}
hdr() { printf '#!/bin/bash\n'; }

# ---- the rules: one bad and one good fixture for EVERY rule -----------------------------------------------

ps_01() { # every rule has a known-bad line that is reported (its own rule, on its own line, and no other rule) and a known-good line that reports nothing
  new_test
  mkdir -p "$T/fx"
  python3 -I - "$rules" "$T/fx" > "$T/gen.out" <<'PY' || return 1
import sys
rows = []
for line in open(sys.argv[1]):
    line = line.rstrip('\n')
    if not line.strip() or line.startswith('#'):
        continue
    f = line.split(' ||| ')
    assert len(f) == 3, 'a row needs three fields: %r' % line
    rows.append(f)
ids = [r[0] for r in rows]
assert len(set(ids)) == len(ids), 'a rule id occurs twice'
for rid, bad, good in rows:
    for kind, text in (('bad', bad), ('good', good)):
        open('%s/%s-%s.sh' % (sys.argv[2], kind, rid), 'w').write('#!/bin/bash\n# fixture %s %s\n%s\n' % (rid, kind, text.replace('<NL>', '\n')))
print(len(rows))
PY
  nrules=$(cat "$T/gen.out")
  need "the fixture table has at least 60 rules (has $nrules)" test "$nrules" -ge 60 || return 1
  sc a --format json "$T"/fx/bad-*.sh "$T"/fx/good-*.sh
  need "the scan of all the fixtures exits 0 (a report; got $rc): $(head -n 2 "$T/a.err" | tr '\n' '|')" test "$rc" -eq 0 || return 1
  python3 -I - "$T/a.out" "$T/fx" "$rules" > "$T/check.out" 2>&1 <<'PY' || { cat "$T/check.out" >&2; return 1; }
import json, os, sys
d = json.load(open(sys.argv[1]))
fx = sys.argv[2]
by = {}
for f in d['findings']:
    by.setdefault(os.path.basename(f['file']), []).append(f)
bad_rows = {}
for line in open(sys.argv[3]):
    line = line.rstrip('\n')
    if not line.strip() or line.startswith('#'):
        continue
    rid, bad, good = line.split(' ||| ')
    bad_rows[rid] = bad.replace('<NL>', '\n')
problems = []
for rid, bad in sorted(bad_rows.items()):
    want_line = 2 + len(bad.split('\n'))
    got = by.get('bad-%s.sh' % rid, [])
    rules = sorted(set(f['rule'] for f in got))
    if rules != [rid]:
        problems.append('bad-%s.sh: reported %s, wanted exactly [%s]' % (rid, rules, rid))
    elif not any(f['rule'] == rid and f['line'] == want_line for f in got):
        problems.append('bad-%s.sh: %s reported on lines %s, wanted line %d' % (rid, rid, [f['line'] for f in got], want_line))
    for f in got:
        if not f.get('replacement') or not f.get('message'):
            problems.append('bad-%s.sh: a finding without a message or a replacement' % rid)
    good = by.get('good-%s.sh' % rid, [])
    if good:
        problems.append('good-%s.sh: reported %s, wanted nothing' % (rid, [(f['rule'], f['line']) for f in good]))
print('\n'.join(problems[:20]))
sys.exit(1 if problems else 0)
PY
}

ps_02() { # the lexer: comments, single quotes, message text and quoted here-documents are blanked; $(...), backticks and expansions inside them are scanned
  new_test
  cat > "$T/w/lex.sh" <<'LEX'
#!/bin/bash
# sed -i in a comment
echo 'sed -i x f'
echo "sed -i in a message"
cat <<'EOF'
sed -i in a quoted heredoc
EOF
cat <<EOF2
sed -i in plain text but $(sed -i x f) is code
EOF2
echo "$(sed -i x f)"
echo "a # not a comment"; sed -i x f
x=`sed -i x f`
echo 'it'"'"'s'; sed -i x f
sed -i x f # trailing comment
echo "line one
sed -i in a multi-line message
line three"; sed -i x f
echo "it's"; sed -i x f
LEX
  sc a lex.sh
  need "exit 0 (got $rc)" test "$rc" -eq 0 || return 1
  got=$(reported "$T/a.out" | tr '\n' ' ')
  want="9:MP-001 11:MP-001 12:MP-001 13:MP-001 14:MP-001 15:MP-001 18:MP-001 19:MP-001 "
  need "only the code lines are reported: got [$got] wanted [$want]" test "$got" = "$want" || return 1
}

ps_03() { # the construct is found where a command is: after separators, keywords, wrappers; the option forms; and not where it is only an argument
  new_test
  cat > "$T/w/bad.sh" <<'BAD'
#!/bin/bash
sed -i x f
sed -i '' x f
sed --in-place x f
sed -ni 'p' f
if sed -i x f; then :; fi
a | sed -i x f
A=1 sed -i x f
find . -exec sed -i x {} +
xargs sed -i x
sudo sed -i x f
( sed -i x f )
{ sed -i x f; }
a && sed -i x f
command sed -i x f
BAD
  cat > "$T/w/good.sh" <<'GOOD'
#!/bin/bash
sed -i.bak x f
sed -n 'p' f
grep -i x f
echo sed -i
ls sed -i
sed 's/-i/x/' f
command -v sed
type sed
printf '%s\n' sed -i
x=sed
GOOD
  sc a bad.sh
  got=$(reported "$T/a.out" | tr '\n' ' ')
  want="2:MP-001 3:MP-001 4:MP-001 5:MP-001 6:MP-001 7:MP-001 8:MP-001 9:MP-001 10:MP-001 11:MP-001 12:MP-001 13:MP-001 14:MP-001 15:MP-001 "
  need "every command position is found: got [$got]" test "$got" = "$want" || return 1
  sc b good.sh
  need "nothing is reported on the good file: $(reported "$T/b.out" | tr '\n' ' ')" test -z "$(reported "$T/b.out")" || return 1
  # other options of other rules: attached and long forms
  cat > "$T/w/more.sh" <<'MORE'
#!/bin/bash
date --date=x +%s
stat -c%a f
readlink -e f
sort -n -V f
date -u +%FT%TZ
date +%s
date -j -f %s 5 +%s
stat -f %a f
readlink f
sort -n f
mktemp "$d/x.XXXXXX"
mktemp -d "${TMPDIR:-/tmp}/x.XXXXXX"
mktemp
MORE
  sc c more.sh
  got=$(reported "$T/c.out" | tr '\n' ' ')
  want="2:MP-004 3:MP-006 4:MP-007 5:MP-016 14:MP-013 "
  need "attached and long option forms are found, the portable spellings are not: got [$got] wanted [$want]" test "$got" = "$want" || return 1
}

ps_04() { # the allow marker: only in a real comment, only with a reason; it is listed as a NOTE with its reason, never silently
  new_test
  cat > "$T/w/allow.sh" <<'ALLOW'
#!/bin/bash
sed -i x f # portable: ok the Linux-only branch of the installer
sed -i y f
sed -i z f # portable: ok
sed -i q f; echo "# portable: ok not a comment"
ALLOW
  sc a allow.sh
  got=$(reported "$T/a.out" | tr '\n' ' ')
  want="3:MP-001 4:MP-001 4:MP-900 5:MP-001 "
  need "line 2 is allowed; line 3 is a finding; line 4 stays a finding AND the marker without a reason is MP-900; a marker inside a string is no marker: got [$got]" test "$got" = "$want" || return 1
  need "the allowed line is a NOTE with its reason" grep -q '^NOTE [^ ]*:2: MP-001 allowed: the Linux-only branch of the installer$' "$T/a.out" || return 1
  need "the summary counts one allowed line" grep -q ', 1 allowed, in 1 file(s)$' "$T/a.out" || return 1
  sc b --format json allow.sh
  need "the JSON lists the allowed line with its reason" python3 -I -c '
import json, sys
d = json.load(open(sys.argv[1]))
a = d["allowed"]
assert len(a) == 1 and a[0]["line"] == 2 and a[0]["rule"] == "MP-001" and a[0]["reason"] == "the Linux-only branch of the installer", a
assert d["summary"]["allowed"] == 1
' "$T/b.out" || return 1
}

ps_05() { # the warning rule: an array expansion under set -u is an error before bash 4.4; the safe spelling, "$@" and a quoted here-document are not reported
  new_test
  mk() { # mk <name> <line 2> <line 3>
    printf '#!/bin/bash\n%s\n%s\n' "$2" "$3" > "$T/w/$1.sh"
  }
  mk a 'set -u' 'echo "${arr[@]}"'
  mk b 'set -eu' 'echo "${arr[@]}"'
  mk c 'set -o nounset' 'echo "${arr[@]}"'
  mk d 'set -e' 'echo "${arr[@]}"'
  mk e 'set -u' 'echo ${arr[@]+"${arr[@]}"}'
  mk f 'set -u' 'echo "$@"'
  mk i 'set -u; echo "${arr[@]}"' ':'
  mk j 'set -u' 'echo "${arr[*]}"'
  printf '#!/bin/bash\nset -u\ncat <<'"'"'EOF'"'"'\n"${arr[@]}"\nEOF\n' > "$T/w/g.sh"
  for n in a b c; do
    sc "w$n" "$n.sh"
    need "$n.sh: MP-070 is reported on line 3 as a warning" grep -q '^[^ ]*:3: warn MP-070 ' "$T/w$n.out" || return 1
  done
  for n in d e f g; do
    sc "w$n" "$n.sh"
    need "$n.sh: nothing is reported ($(reported "$T/w$n.out" | tr '\n' ' '))" test -z "$(reported "$T/w$n.out")" || return 1
  done
  sc wi i.sh
  need "i.sh: an array expansion on the same line as set -u, after it, is reported on line 2" grep -q '^[^ ]*:2: warn MP-070 ' "$T/wi.out" || return 1
  sc wj j.sh
  need "j.sh: \"\${arr[*]}\" is an array expansion too (line 3)" grep -q '^[^ ]*:3: warn MP-070 ' "$T/wj.out" || return 1
}

# ---- the report -------------------------------------------------------------------------------------------

ps_06() { # text and JSON formats, counts, order, and the exit codes (0 a report, 1 only with --strict on an error, 2 a refusal)
  new_test
  printf '#!/bin/bash\nsed -i x f\nxargs -r rm\necho -e "a"\n' > "$T/w/r1.sh"
  printf '#!/bin/bash\nxargs -r rm\necho -e "a"\n' > "$T/w/r2.sh"
  sc a r1.sh
  need "a report exits 0 even with findings (got $rc)" test "$rc" -eq 0 || return 1
  need "the first line says what the report is and is not" sh -c "head -n 1 '$T/a.out' | grep -q '^mac-port-scan: REPORT from documentation.*not a proof'" || return 1
  need "the finding lines are file:line: severity rule message; use: replacement, in line order" sh -c "grep -E '^r1.sh:[0-9]+: (error|warn|info) MP-[0-9]+ .*; use: .+' '$T/a.out' | cut -d: -f2 | tr '\n' ' ' | grep -qx '2 3 4 '" || return 1
  need "the severities are error, info, warn for these three" sh -c "grep -E '^r1.sh:' '$T/a.out' | awk '{print \$2}' | tr '\n' ' ' | grep -qx 'error info warn '" || return 1
  need "the last line is the summary with the three counts" sh -c "tail -n 1 '$T/a.out' | grep -qx 'mac-port-scan: 3 finding(s) (1 error, 1 warn, 1 info), 0 allowed, in 1 file(s)'" || return 1
  sc b --strict r1.sh
  need "--strict exits 1 on an error-level finding (got $rc)" test "$rc" -eq 1 || return 1
  sc c --strict r2.sh
  need "--strict exits 0 when there is only warn and info (got $rc)" test "$rc" -eq 0 || return 1
  sc d --format json r1.sh r2.sh
  need "the JSON has limit, findings (file, line, rule, severity, message, replacement), allowed and a summary that agrees, files in the order given" python3 -I -c '
import json, sys
d = json.load(open(sys.argv[1]))
assert set(d) == {"limit", "findings", "allowed", "summary"}, sorted(d)
f = d["findings"]
assert [x["file"].split("/")[-1] for x in f] == ["r1.sh"] * 3 + ["r2.sh"] * 2, f
assert [x["line"] for x in f] == [2, 3, 4, 2, 3], f
assert all(set(x) == {"file", "line", "rule", "severity", "message", "replacement"} for x in f)
s = d["summary"]
assert s == {"findings": 5, "error": 1, "warn": 2, "info": 2, "allowed": 0, "files": 2}, s
assert "not a proof" in d["limit"]
' "$T/d.out" || return 1
  sc e --bogus r1.sh
  need "an unknown option: exit 2 with a problem and a fix, no traceback, and it is named as an option the tool does not know (not tried as a file)" sh -c "[ $rc -eq 2 ] && grep -q 'problem:' '$T/e.err' && grep -q 'fix:' '$T/e.err' && ! grep -qi traceback '$T/e.err' && grep -q 'an option this tool does not know: --bogus' '$T/e.err'" || return 1
  sc f missing.sh
  need "a missing file: exit 2 naming it" sh -c "[ $rc -eq 2 ] && grep -q 'missing.sh' '$T/f.err' && ! grep -qi traceback '$T/f.err'" || return 1
  sc g --format xml r1.sh
  need "an unknown format: exit 2" test "$rc" -eq 2 || return 1
  mkdir "$T/w/adir"
  sc h adir
  need "a directory as a file: exit 2, and the message says it is a directory (not a read error)" sh -c "[ $rc -eq 2 ] && grep -q 'adir is a directory' '$T/h.err' && grep -q 'fix:' '$T/h.err'" || return 1
  sc i --help
  need "--help: exit 0 and the usage" sh -c "[ $rc -eq 0 ] && grep -q 'mac-port-scan' '$T/i.out'" || return 1
}

ps_07() { # --list-rules is the rule table, and it is the fixture table: the same ids, valid severities, the planned severities for the notable ones
  new_test
  sc a --list-rules
  need "--list-rules exits 0 (got $rc)" test "$rc" -eq 0 || return 1
  python3 -I - "$T/a.out" "$rules" > "$T/check.out" 2>&1 <<'PY' || { cat "$T/check.out" >&2; return 1; }
import sys
listed = {}
for line in open(sys.argv[1]):
    p = line.split(None, 2)
    assert len(p) == 3, 'a line needs id, severity and a title: %r' % line
    assert p[1] in ('error', 'warn', 'info'), p
    listed[p[0]] = p[1]
table = set()
for line in open(sys.argv[2]):
    line = line.rstrip('\n')
    if line.strip() and not line.startswith('#'):
        table.add(line.split(' ||| ')[0])
table.add('MP-900')
miss = sorted(set(listed) ^ table)
assert not miss, 'the rule list and the fixture table differ on %s (MP-900 is the marker rule and needs no row)' % miss
for r in ('MP-007', 'MP-008', 'MP-030', 'MP-031'):
    assert listed[r] == 'info', (r, listed[r])
for r in ('MP-032', 'MP-070'):
    assert listed[r] == 'warn', (r, listed[r])
for r in ('MP-001', 'MP-035', 'MP-050', 'MP-900'):
    assert listed[r] == 'error', (r, listed[r])
PY
}

ps_08() { # what is only a word, a name or a check is not an invocation
  new_test
  cat > "$T/w/fp.sh" <<'FP'
#!/bin/bash
command -v flock >/dev/null
type timeout
which sha256sum
echo "run sha256sum next"
printf '%s\n' systemctl
cmd=readlink
# stat -c %a is Linux
grep -q 'timeout' f
name=tac
sha256sum_tool=1
echo "$sed_i"
my_flock() { :; }
mapfile_note=1
sed -i x f
FP
  sc a fp.sh
  need "the scan exits 0 (got $rc)" test "$rc" -eq 0 || return 1
  need "only the one real invocation at the end (the positive control) is reported: got [$(reported "$T/a.out" | tr '\n' ' ')]" test "$(reported "$T/a.out" | tr '\n' ' ')" = "15:MP-001 " || return 1
}

ps_09() { # a smoke report on the real installer files: it runs, the lines are in order, the counts agree with the lines, the files are untouched; no count is asserted
  new_test
  [ -f "$deploy/install.sh" ] && [ -f "$deploy/db-roles.sh" ] || { echo "    the installer files are not in this checkout" >&2; return 1; }
  before=$(cksum "$deploy/install.sh" "$deploy/db-roles.sh" | tr '\n' ' ')
  sc a --format json "$deploy/install.sh" "$deploy/db-roles.sh"
  need "the scan exits 0 (got $rc): $(head -n 2 "$T/a.err" | tr '\n' '|')" test "$rc" -eq 0 || return 1
  python3 -I - "$T/a.out" > "$T/check.out" 2>&1 <<'PY' || { cat "$T/check.out" >&2; return 1; }
import json, sys
d = json.load(open(sys.argv[1]))
f = d['findings']
order = []
for x in f:
    if x['file'] not in order:
        order.append(x['file'])
assert [o.split('/')[-1] for o in order] == [n for n in ('install.sh', 'db-roles.sh') if n in [o.split('/')[-1] for o in order]], order
for name in order:
    lines = [x['line'] for x in f if x['file'] == name]
    assert lines == sorted(lines), 'lines out of order in %s' % name
s = d['summary']
assert s['findings'] == len(f)
for sev in ('error', 'warn', 'info'):
    assert s[sev] == sum(1 for x in f if x['severity'] == sev), sev
assert s['files'] == 2
PY
  sc b --strict "$deploy/install.sh" "$deploy/db-roles.sh"
  haserr=$(python3 -I -c 'import json,sys; print(1 if any(x["severity"]=="error" for x in json.load(open(sys.argv[1]))["findings"]) else 0)' "$T/a.out")
  need "--strict exits 1 exactly when there is an error-level finding (error findings: $haserr, exit $rc)" test "$rc" -eq "$haserr" || return 1
  after=$(cksum "$deploy/install.sh" "$deploy/db-roles.sh" | tr '\n' ' ')
  need "the installer files are byte-identical after the scans" test "$before" = "$after" || return 1
}

ps_10() { # the scanner only READS: it executes nothing, writes nothing, changes nothing (an AST check of its source, and a before and after of the files and the directory)
  new_test
  python3 -I - "$scan" > "$T/ast.out" 2>&1 <<'PY' || { cat "$T/ast.out" >&2; return 1; }
import ast, sys
tree = ast.parse(open(sys.argv[1]).read())
allowed = {'sys', 're', 'json', 'os'}
bad = []
for n in ast.walk(tree):
    if isinstance(n, ast.Import):
        bad += [a.name for a in n.names if a.name.split('.')[0] not in allowed]
    if isinstance(n, ast.ImportFrom) and (n.module or '').split('.')[0] not in allowed:
        bad.append(n.module)
    if isinstance(n, ast.Attribute) and n.attr in ('system', 'popen', 'execv', 'execvp', 'execve', 'spawnl', 'remove', 'unlink', 'rename', 'rmdir', 'removedirs', 'makedirs', 'mkdir', 'chmod', 'chown', 'kill', 'killpg', 'fork', 'write_text', 'write_bytes'):
        bad.append('attribute ' + n.attr)
    if isinstance(n, ast.Name) and n.id in ('eval', 'exec', 'compile', '__import__'):
        bad.append('name ' + n.id)
    if isinstance(n, ast.Call) and isinstance(n.func, ast.Name) and n.func.id == 'open':
        mode = n.args[1].value if len(n.args) > 1 and isinstance(n.args[1], ast.Constant) else 'r'
        for kw in n.keywords:
            if kw.arg == 'mode' and isinstance(kw.value, ast.Constant):
                mode = kw.value.value
        if any(c in str(mode) for c in 'wax+'):
            bad.append('open with mode ' + str(mode))
assert not bad, 'the scanner must only read: %s' % sorted(set(bad))
PY
  cat > "$T/w/danger.sh" <<'DANGER'
#!/bin/bash
echo CANARY > canary-must-not-exist
touch canary-must-not-exist-2
sed -i x f
DANGER
  b1=$(cksum "$T/w/danger.sh")
  b2=$(ls -A "$T/w" | tr '\n' ' ')
  sc a danger.sh
  need "the scan of a file whose lines would leave canary files, if anything ran them, exits 0 (got $rc)" test "$rc" -eq 0 || return 1
  need "the file is byte-identical and nothing was created beside it" test "$b1|$b2" = "$(cksum "$T/w/danger.sh")|$(ls -A "$T/w" | tr '\n' ' ')" || { ls -A "$T/w" >&2; return 1; }
  need "and the sed -i in it is reported" grep -q ' MP-001 ' "$T/a.out" || return 1
}

# ---- the runner -------------------------------------------------------------------------------------------

mkrun() { # mkrun <name> <args...>: the runner, TMPDIR private to the cell
  _n=$1
  shift
  rc=0
  env TMPDIR="$T/tmp" $sut "$runner" "$@" > "$T/$_n.out" 2> "$T/$_n.err" || rc=$?
}
ps_11() { # the runner: the LIMIT first, the factual line second, three shapes that really differ, in a child sh as well
  new_test
  mkrun a "$fx/probe-shape.sh" one two
  need "the program's arguments reach it in every shape" sh -c "grep -c '^args=one two\$' '$T/a.out' | grep -qx 3" || return 1
  need "the runner exits 0 when every shape passes (got $rc): $(head -n 3 "$T/a.err" | tr '\n' '|')" test "$rc" -eq 0 || return 1
  need "the first line is the LIMIT, verbatim" test "$(head -n 1 "$T/a.out")" = "$LIMIT" || return 1
  need "the second line is the factual line about BASH_COMPAT=32" sh -c "sed -n 2p '$T/a.out' | grep -q 'BASH_COMPAT=32 is bash [0-9].* acting like 3.2 where its manual documents a switch; it does not remove bash 4+ features'" || return 1
  python3 -I - "$T/a.out" "$T/tmp" > "$T/check.out" 2>&1 <<'PY' || { cat "$T/check.out" >&2; return 1; }
import re, sys
text = open(sys.argv[1]).read()
parts = re.split(r'^== shape (\S+) ==$', text, flags=re.M)
assert len(parts) == 7, 'three shapes expected, got %d' % ((len(parts) - 1) // 2)
got = {}
for i in range(1, 7, 2):
    kv = dict(l.split('=', 1) for l in parts[i + 1].splitlines() if re.match(r'^[a-z_]+=', l))
    # the shim sh is FIRST in PATH in every shape (on a host whose /bin/sh is bash in POSIX mode a missing shim would not change the other fields)
    assert re.fullmatch(re.escape(sys.argv[2]) + r'/mac-port-run\.[A-Za-z0-9]{6}/sh', kv.pop('child_sh', '')), (parts[i], 'the child sh is not the shim')
    got[parts[i]] = kv
want = {
    'posix': {'shape': 'posix', 'args': 'one two', 'self_posix': 'yes', 'self_compat': 'none', 'child_posix': 'yes', 'child_compat': 'none'},
    'compat32': {'shape': 'compat32', 'args': 'one two', 'self_posix': 'no', 'self_compat': '32', 'child_posix': 'no', 'child_compat': '32'},
    'posix+compat32': {'shape': 'posix+compat32', 'args': 'one two', 'self_posix': 'yes', 'self_compat': '32', 'child_posix': 'yes', 'child_compat': '32'},
}
assert got == want, 'shapes differ from what they must be:\n got  %s\n want %s' % (got, want)
PY
  # a hostile caller environment (BASH_COMPAT, POSIXLY_CORRECT, SHELLOPTS) must not change a shape
  run h env TMPDIR="$T/tmp" BASH_COMPAT=44 POSIXLY_CORRECT=y SHELLOPTS=posix $sut "$runner" "$fx/probe-shape.sh" one two
  need "with a hostile caller environment the runner still exits 0 (got $rc)" test "$rc" -eq 0 || return 1
  tail -n +3 "$T/a.out" | sed 's/mac-port-run\.[A-Za-z0-9]*/mac-port-run.X/' > "$T/a.body"
  tail -n +3 "$T/h.out" | sed 's/mac-port-run\.[A-Za-z0-9]*/mac-port-run.X/' > "$T/h.body"
  need "and prints exactly what it printed with a clean environment" cmp -s "$T/a.body" "$T/h.body" || { diff "$T/a.body" "$T/h.body" >&2; return 1; }
  # the caller's startup files and exported functions are inputs too: BASH_ENV and ENV name a file a non-interactive bash runs first, BASHOPTS sets shopt
  # options, an exported function named sh would replace the shim, GLOBIGNORE and CDPATH change what a word means (sam M1A-SAM-01)
  printf 'printf "startup:%%s\n" "${MAC_PORT_SHAPE:-none}" >> "%s/canary.log"\n' "$T" > "$T/startup.sh"
  : > "$T/canary.log"
  run h2 env TMPDIR="$T/tmp" BASH_ENV="$T/startup.sh" ENV="$T/startup.sh" BASHOPTS=xpg_echo GLOBIGNORE='*' CDPATH=/ 'BASH_FUNC_sh%%=() { echo hijacked; }' $sut "$runner" "$fx/probe-shape.sh" one two
  need "with BASH_ENV, ENV, BASHOPTS, GLOBIGNORE, CDPATH and an exported function sh in the caller's environment the runner still exits 0 (got $rc)" test "$rc" -eq 0 || return 1
  need "no startup file of the caller ran, in any shape or in the version query ($(tr '\n' ' ' < "$T/canary.log"))" test ! -s "$T/canary.log" || return 1
  tail -n +3 "$T/h2.out" | sed 's/mac-port-run\.[A-Za-z0-9]*/mac-port-run.X/' > "$T/h2.body"
  need "and the output is exactly the clean one" cmp -s "$T/a.body" "$T/h2.body" || { diff "$T/a.body" "$T/h2.body" >&2; return 1; }
  need "each shape has an ok line and the last line counts the shapes" sh -c "grep -c '^mac-port-run: shape .*: ok\$' '$T/a.out' | grep -qx 3 && tail -n 1 '$T/a.out' | grep -qx 'mac-port-run: 3 shape(s) run, 0 failed'" || return 1
}

ps_12() { # a failing shape fails the run and is named, the other shapes still run; refusals; nothing is left behind
  new_test
  mkrun a "$fx/probe-fail-compat.sh"
  need "a program that fails under BASH_COMPAT=32 makes the runner exit 1 (got $rc)" test "$rc" -eq 1 || return 1
  need "all three shapes ran" sh -c "grep -c '^== shape ' '$T/a.out' | grep -qx 3" || return 1
  need "posix is ok; compat32 and posix+compat32 FAILED with the program's exit code 7" sh -c "grep -qx 'mac-port-run: shape posix: ok' '$T/a.out' && grep -qx 'mac-port-run: shape compat32: FAILED (exit 7)' '$T/a.out' && grep -qx 'mac-port-run: shape posix+compat32: FAILED (exit 7)' '$T/a.out'" || return 1
  need "the last line says three shapes, two failed" sh -c "tail -n 1 '$T/a.out' | grep -qx 'mac-port-run: 3 shape(s) run, 2 failed'" || return 1
  need "the program's own standard error is in the output" grep -qx 'probe-fail-compat: failing under BASH_COMPAT=32' "$T/a.out" || return 1
  need "no shim directory is left in the temporary directory ($(ls -A "$T/tmp" | tr '\n' ' '))" test -z "$(ls -A "$T/tmp")" || return 1
  # refusals: only its own fixtures are run (cox condition 2)
  printf '#!/bin/bash\necho should-never-run > "%s/ran"\n' "$T" > "$T/outside.sh"
  mkrun b "$T/outside.sh"
  need "a script outside the fixtures directory is refused: exit 2, problem and fix, and it did not run" sh -c "[ $rc -eq 2 ] && grep -q 'problem:' '$T/b.err' && grep -q 'fix:' '$T/b.err' && [ ! -e '$T/ran' ]" || return 1
  mkrun c "$deploy/install.sh"
  need "deploy/install.sh is refused: exit 2, and the message says the runner runs only its own fixtures" sh -c "[ $rc -eq 2 ] && grep -qi 'only its own fixtures' '$T/c.err'" || return 1
  mkrun d "$fx/../mac-port-test.sh"
  need "a file beside the fixtures, not in them, is refused too (exit $rc)" test "$rc" -eq 2 || return 1
  mkrun e "$fx/no-such-probe.sh"
  need "a missing program: exit 2 and the message says it does not exist" sh -c "[ $rc -eq 2 ] && grep -q 'does not exist' '$T/e.err'" || return 1
  mkrun f
  need "no arguments: exit 2 and a usage line" sh -c "[ $rc -eq 2 ] && grep -q 'usage' '$T/f.err'" || return 1
  need "nothing printed a traceback or ran the refused programs" sh -c "[ ! -e '$T/ran' ]" || return 1
}

ps_13() { # the LIMIT is verbatim in the runner's output, the scanner's --help and this script; no systemd tool, no kill by name or group in the new shell files
  new_test
  sc a --help
  need "the scanner's --help carries the LIMIT verbatim" grep -qF "$LIMIT" "$T/a.out" || return 1
  need "the runner's source carries the LIMIT verbatim" grep -qF "$LIMIT" "$runner" || return 1
  need "this script carries it as its first printed line" grep -qF "LIMIT=\"$LIMIT\"" "$here/mac-port-test.sh" || return 1
  sysd='system''ctl|login''ctl|journal''ctl|systemd''-run|systemd''-analyze'
  need "the runner names no systemd tool" test -z "$(grep -E "$sysd" "$runner" || true)" || return 1
  killw='pk''ill|kill''all|kill -''9|kill -''KILL'
  need "the runner kills nothing by name, group or signal 9" test -z "$(grep -E "$killw" "$runner" || true)" || return 1
}

ps_14() { # syntax under sh and bash --posix, python parses, modes and shebangs, the scanner runs with -I
  new_test
  for f in "$runner" "$here/mac-port-test.sh" "$fx/probe-shape.sh" "$fx/probe-fail-compat.sh"; do
    need "$(basename "$f") parses under bash --posix" bash --posix -n "$f" || return 1
  done
  for f in "$runner" "$here/mac-port-test.sh"; do
    need "$(basename "$f") parses under sh" sh -n "$f" || return 1
    need "$(basename "$f") starts with #!/bin/sh" test "$(head -n 1 "$f")" = "#!/bin/sh" || return 1
  done
  need "the scanner compiles" python3 -I -c 'import ast,sys; ast.parse(open(sys.argv[1]).read())' "$scan" || return 1
  need "the scanner starts with #!/usr/bin/env python3" test "$(head -n 1 "$scan")" = "#!/usr/bin/env python3" || return 1
  for f in "$scan" "$runner" "$here/mac-port-test.sh" "$fx/probe-shape.sh" "$fx/probe-fail-compat.sh"; do
    need "$(basename "$f") is mode 0755" test "$(stat -c %a "$f")" = 755 || return 1
  done
  need "the runner uses no bashism (no [[, no arrays, no local, no <(...))" test -z "$(grep -nE '\[\[|\]\]|<\(|(^|[;&|({])[[:space:]]*local[[:space:]]|^[[:space:]]*function[[:space:]]|\+=\(' "$runner" || true)" || return 1
}

ps_15() { # more forms of the rules (attached options, clusters, long forms, the portable spellings): exactly the expected rules on every line
  new_test
  python3 -I - "$fx/forms.txt" "$T/w/forms.sh" > "$T/gen.out" <<'PY' || return 1
import sys
rows = []
for line in open(sys.argv[1]):
    line = line.rstrip('\n')
    if not line.strip() or line.startswith('#'):
        continue
    exp, text = line.split(' ||| ', 1)
    rows.append((exp, text))
open(sys.argv[2], 'w').write('#!/bin/bash\n# forms\n' + '\n'.join(t for e, t in rows) + '\n')
open(sys.argv[2] + '.expect', 'w').write('\n'.join(e for e, t in rows) + '\n')
print(len(rows))
PY
  nrows=$(cat "$T/gen.out")
  need "the forms table has at least 120 rows (has $nrows)" test "$nrows" -ge 120 || return 1
  sc a --format json forms.sh
  need "the scan exits 0 (got $rc): $(head -n 2 "$T/a.err" | tr '\n' '|')" test "$rc" -eq 0 || return 1
  python3 -I - "$T/a.out" "$T/w/forms.sh.expect" "$T/w/forms.sh" > "$T/check.out" 2>&1 <<'PY' || { cat "$T/check.out" >&2; return 1; }
import json, sys
d = json.load(open(sys.argv[1]))
got = {}
for f in d['findings']:
    got.setdefault(f['line'], set()).add(f['rule'])
exp = [l for l in open(sys.argv[2]).read().split('\n') if l]
src = open(sys.argv[3]).read().split('\n')
problems = []
for n, e in enumerate(exp, 3):
    want = set() if e == 'OK' else set(e.split(','))
    if got.get(n, set()) != want:
        problems.append('line %d  %s  wanted %s, got %s' % (n, src[n - 1], sorted(want) or 'nothing', sorted(got.get(n, set())) or 'nothing'))
print('\n'.join(problems[:25]))
sys.exit(1 if problems else 0)
PY
}

ps_16() { # structure: wrappers, find -exec, here-documents with a tab-stripped delimiter, a here-string, an arithmetic shift, a continued line
  new_test
  {
    printf '#!/bin/bash\n'
    printf 'env FOO=1 sed -i x f\n'
    printf 'timeout 5 sed -i x f\n'
    printf 'nohup sed -i x f\n'
    printf 'nice -n 5 sed -i x f\n'
    printf 'exec sed -i x f\n'
    printf 'time sed -i x f\n'
    printf 'find . -execdir sed -i x {} \;\n'
    printf 'find . -ok sed -i x {} \;\n'
    printf 'command -p sed -i x f\n'
    printf 'x=$((1<<3)); sed -i x f\n'
    printf 'sed \\\n'
    printf '  -i x f\n'
    printf 'cat <<-EOF3\n'
    printf '\tsed -i in a tab-indented heredoc\n'
    printf '\tEOF3\n'
    printf 'cat <<< "$(sed -i x f)"\n'
    printf "cat <<< 'sed -i x f'\n"
    printf 'sed -i x f 2>&1\n'
    printf 'sed -i x f >/dev/null 2>&1 &\n'
    printf 'echo ok\n'
    printf 'sed -i x f\n'
    printf "cat <<'EOF4'\n"
    printf '${v,,} &>> x\n'
    printf 'EOF4\n'
    printf 'cat <<EOF5\n'
    printf '${v,,} but not &>> this\n'
    printf 'EOF5\n'
    printf 'find . -okdir sed -i x {} \\;\n'
    printf 'if :; then sed -i x f; fi\n'
    printf 'while :; do sed -i x f; done\n'
    printf '! sed -i x f\n'
    printf 'until sed -i x f; do :; done\n'
    printf 'x || { sed -i x f; }\n'
    printf 'if :; then :; elif sed -i x f; then :; else sed -i x f; fi\n'
    printf 'sudo -u root sed -i x f\n'
    printf 'doas sed -i x f\n'
    printf 'xargs -n 1 sed -i x\n'
    printf 'timeout -s TERM 5 sed -i x f\n'
    printf 'env -u X sed -i x f\n'
    printf 'stdbuf -o0 sed -i x f\n'
    printf 'if :; then :\n'
    printf 'elif sed -i x f; then :\n'
    printf 'fi\n'
    printf 'if :; then :\n'
    printf 'else sed -i x f\n'
    printf 'fi\n'
    printf 'while sed -i x f; do :; done\n'
    printf 'sed \\\n'
    printf '%s\n' '-i x f'
  } > "$T/w/struct.sh"
  sc a struct.sh
  need "exit 0 (got $rc)" test "$rc" -eq 0 || return 1
  got=$(reported "$T/a.out" | tr '\n' ' ')
  want="2:MP-001 3:MP-001 3:MP-011 4:MP-001 5:MP-001 6:MP-001 7:MP-001 8:MP-001 9:MP-001 10:MP-001 11:MP-001 12:MP-001 17:MP-001 19:MP-001 20:MP-001 22:MP-001 27:MP-052 29:MP-001 30:MP-001 31:MP-001 32:MP-001 33:MP-001 34:MP-001 35:MP-001 36:MP-001 37:MP-001 38:MP-001 39:MP-001 39:MP-011 40:MP-001 41:MP-001 43:MP-001 46:MP-001 48:MP-001 49:MP-001 "
  need "found where a command is, nothing in the bodies: got [$got] wanted [$want]" test "$got" = "$want" || return 1
}

ps_17() { # the runner takes its fixtures directory from where IT is: a link inside it, a sibling directory and a path with .. are refused; a plain fixture runs
  new_test
  L=$T/lane/scripts
  mkdir -p "$L/mac-port-fixtures" "$L/mac-port-fixtures-extra"
  cp "$runner" "$L/mac-port-run"
  chmod 0755 "$L/mac-port-run"
  cp "$fx/probe-shape.sh" "$L/mac-port-fixtures/probe-shape.sh"
  cp "$fx/probe-shape.sh" "$L/mac-port-fixtures-extra/probe-shape.sh"
  ln -s "$fx/probe-shape.sh" "$L/mac-port-fixtures/linked.sh"
  rc=0
  env TMPDIR="$T/tmp" $sut "$L/mac-port-run" "$L/mac-port-fixtures/probe-shape.sh" > "$T/a.out" 2> "$T/a.err" || rc=$?
  need "a plain fixture in the copied layout runs (got $rc): $(head -n 2 "$T/a.err" | tr '\n' '|')" test "$rc" -eq 0 || return 1
  rc=0
  env TMPDIR="$T/tmp" $sut "$L/mac-port-run" "$L/mac-port-fixtures/linked.sh" > "$T/b.out" 2> "$T/b.err" || rc=$?
  need "a symbolic link inside the fixtures directory is refused: exit 2 and the message says so (got $rc)" sh -c "[ $rc -eq 2 ] && grep -qi 'symbolic link' '$T/b.err'" || return 1
  rc=0
  env TMPDIR="$T/tmp" $sut "$L/mac-port-run" "$L/mac-port-fixtures-extra/probe-shape.sh" > "$T/c.out" 2> "$T/c.err" || rc=$?
  need "a directory whose name only starts like the fixtures directory is refused (got $rc)" test "$rc" -eq 2 || return 1
  rc=0
  env TMPDIR="$T/tmp" $sut "$L/mac-port-run" "$L/mac-port-fixtures/../mac-port-fixtures-extra/probe-shape.sh" > "$T/d.out" 2> "$T/d.err" || rc=$?
  need "a path with .. into a sibling directory is refused (got $rc)" test "$rc" -eq 2 || return 1
  need "none of the three refused programs printed anything of its own" sh -c "! grep -q '^shape=' '$T/b.out' '$T/c.out' '$T/d.out'" || return 1
}

ps_18() { # the lexer survives what is cut off and what is hostile: no exception, the same answer twice, nothing slow (the scanner is imported, not run, so that 400 cuts cost one process)
  new_test
  python3 -I - "$scan" "$fx/forms.txt" "$fx/rules.txt" > "$T/fuzz.out" 2>&1 <<'PY' || { cat "$T/fuzz.out" >&2; return 1; }
import importlib.machinery, importlib.util, sys, time
loader = importlib.machinery.SourceFileLoader('mac_port_scan', sys.argv[1])
spec = importlib.util.spec_from_loader('mac_port_scan', loader)
m = importlib.util.module_from_spec(spec)
loader.exec_module(m)
src = ''
for path in sys.argv[2:]:
    for line in open(path):
        line = line.rstrip('\n')
        if line.strip() and not line.startswith('#'):
            for field in line.split(' ||| ')[1:]:
                src += field.replace('<NL>', '\n') + '\n'
assert len(src) > 5000, len(src)
n = 0
t0 = time.time()
step = max(1, len(src) // 300)
for cut in list(range(0, len(src), step)) + [len(src)]:
    part = src[:cut]
    a = m.scan_text(part)
    b = m.scan_text(part)
    assert a == b, 'not deterministic at %d' % cut
    n += 1
frags = ['"', "'", '$(', '${', '`', '<<', '<<EOF', 'cat <<EOF\nabc', '$((', '((', "x=$'", '\\', 'a | ', '&&', ';;&', 'echo "$(', "echo '$(", '{ ', '! ', 'case x in a) ', '}', ')', '))', '`a`b`', '"$(echo "$(echo ")")")"']
for frag in frags:
    for tail in ('', '\n', ' sed -i x f', '\nsed -i x f\n'):
        m.scan_text(frag + tail)
        n += 1
def nest(n):
    text = 'sed -i x f'
    for _ in range(n):
        text = "sh -c '" + text.replace("'", "'\\''") + "'"
    return text
assert [r for _, r in m.scan_text(nest(3))[0]] == ['MP-001'], 'three levels of sh -c are scanned'
assert m.scan_text(nest(4))[0] == [], 'a fourth level is beyond the limit and is not scanned'
dt = time.time() - t0
assert dt < 20, 'too slow: %.1f s for %d scans' % (dt, n)
print('%d scans, %.1f s' % (n, dt))
PY
}

ps_19() { # command substitutions inside the WORD of a parameter expansion are code: unquoted, double-quoted, nested, in an unquoted here-document; the literal twins are quiet (sam M1A-SAM-02)
  new_test
  cat > "$T/w/param.sh" <<'PARAM'
#!/bin/bash
echo ${v:-$(sed -i x f)}
echo "${v:-$(sed -i x f)}"
echo ${v:-$(stat -c %s f)}
echo "${v:-`sed -i x f`}"
echo ${v:-${w:-$(sed -i x f)}}
echo ${v:-'$(sed -i x f)'}
echo "${v:-'$(sed -i x f)'}"
echo '${v:-$(sed -i x f)}'
echo ${v:-$(echo hi)}
cat <<EOF
${v:-$(sed -i x f)}
EOF
cat <<'EOF'
${v:-$(sed -i x f)}
EOF
echo ${v:-"$(sed -i x f)"}
echo ${v:+$(sed -i x f)}
echo ${#v}
echo ${v//$(sed -i x f)/y}
echo ${v:-"'$(sed -i x f)'"}
echo ${v:-$((1<<3))}
sed -i x f
PARAM
  sc a param.sh
  need "exit 0 (got $rc)" test "$rc" -eq 0 || return 1
  got=$(reported "$T/a.out" | tr '\n' ' ')
  want="2:MP-001 3:MP-001 4:MP-006 5:MP-001 6:MP-001 8:MP-001 12:MP-001 17:MP-001 18:MP-001 20:MP-001 21:MP-001 23:MP-001 "
  need "the executable twins are reported, the literal ones are not: got [$got] wanted [$want]" test "$got" = "$want" || return 1
}

ps_20() { # command substitutions inside ARITHMETIC are code too: $(( )), (( )), a C-style for, in double quotes, in an unquoted here-document, inside a parameter word; the literal twins are quiet
  new_test
  cat > "$T/w/arith.sh" <<'ARITH'
#!/bin/bash
echo $(( $(date -d x +%s) + 1 ))
echo "$(( $(stat -c %s f) + 1 ))"
(( n = $(sed -i x f; echo 1) ))
x=$(( 1 << 3 ))
echo '$(( $(sed -i x f) ))'
cat <<EOF
$(( $(sed -i x f) ))
EOF
cat <<'EOF'
$(( $(sed -i x f) ))
EOF
echo ${v:-$(( $(sed -i x f) ))}
for ((i = $(date -d x +%s); i < 3; i++)); do :; done
sed -i x f
ARITH
  sc a arith.sh
  need "exit 0 (got $rc)" test "$rc" -eq 0 || return 1
  got=$(reported "$T/a.out" | tr '\n' ' ')
  want="2:MP-004 3:MP-006 4:MP-001 8:MP-001 13:MP-001 14:MP-004 15:MP-001 "
  need "the executable twins are reported, the literal ones and the plain arithmetic are not: got [$got] wanted [$want]" test "$got" = "$want" || return 1
}

ps_21() { # a fixed command inside a script handed to another shell is code: sh -c, bash -lc, eval, trap, behind a wrapper; reported on the line of the outer command; nothing built at run time
  new_test
  cat > "$T/w/nested.sh" <<'NESTED'
#!/bin/bash
sh -c 'sed -i x f'
bash -c "stat -c %a f"
eval "sed -i x f"
trap 'sed -i x f' EXIT
sh -c 'echo hi'
bash -lc 'readlink -f x'
xargs sh -c 'sed -i x "$@"' _
sudo bash -c 'sha256sum f'
sh -c "$script"
eval "$cmd"
trap 'rm -f "$tmp"' EXIT
trap - EXIT
dash -ec 'timeout 5 true; flock x y'
sh -c 'sh -c "sed -i x f"'
echo sh -c 'sed -i x f'
sh script.sh
sh script.sh -c 'sed -i x f'
sh -o pipefail -c 'sed -i x f'
sh -c 'echo x # portable: ok'
sh -c 'sed -i x f # portable: ok the Linux-only branch'
NESTED
  sc a nested.sh
  need "exit 0 (got $rc)" test "$rc" -eq 0 || return 1
  got=$(reported "$T/a.out" | tr '\n' ' ')
  want="2:MP-001 3:MP-006 4:MP-001 5:MP-001 7:MP-007 8:MP-001 9:MP-010 14:MP-011 14:MP-012 15:MP-001 19:MP-001 "
  need "the strings run by another shell are scanned, the rest is not (the args of a script file are not the shell's options; the marker error of a nested string is not repeated): got [$got] wanted [$want]" test "$got" = "$want" || return 1
  need "a marker with a reason INSIDE a nested string allows its finding and is listed as a NOTE with its reason on the line of the outer command (never silently)" grep -q '^NOTE [^ ]*:21: MP-001 allowed: the Linux-only branch$' "$T/a.out" || return 1
}
ps_22() { # hale MP-1: an exported shell function must not replace a command of the runner: an exported env (the clean room would be built by the function) and an exported [ (a link out of the fixtures directory would run)
  new_test
  L=$T/lane/scripts
  mkdir -p "$L/mac-port-fixtures" "$T/outside"
  cp "$runner" "$L/mac-port-run"
  chmod 0755 "$L/mac-port-run"
  cp "$fx/probe-shape.sh" "$L/mac-port-fixtures/probe-shape.sh"
  printf '#!/bin/sh\necho OUTSIDE-FILE-RAN-AS-A-FIXTURE\n: > "%s/canary.ran"\n' "$T" > "$T/outside/canary.sh"
  chmod 0755 "$T/outside/canary.sh"
  ln -s "$T/outside/canary.sh" "$L/mac-port-fixtures/escape.sh"
  # the control: no hostile function in the environment: all three shapes run
  rc=0
  env TMPDIR="$T/tmp" $sut "$L/mac-port-run" "$L/mac-port-fixtures/probe-shape.sh" > "$T/k.out" 2> "$T/k.err" || rc=$?
  need "without a hostile function the runner exits 0 and all three shapes ran (got $rc)" sh -c "[ $rc -eq 0 ] && [ \"\$(grep -c '^shape=' '$T/k.out')\" -eq 3 ]" || return 1
  # an exported function named env that prints a canary and returns 0
  rc=0
  env TMPDIR="$T/tmp" 'BASH_FUNC_env%%=() { echo HOSTILE-env-RAN; return 0; }' $sut "$L/mac-port-run" "$L/mac-port-fixtures/probe-shape.sh" > "$T/c1.out" 2> "$T/c1.err" || rc=$?
  need "an exported env never ran ($(grep -h HOSTILE "$T/c1.out" "$T/c1.err" | head -n 1))" sh -c "! grep -q HOSTILE-env-RAN '$T/c1.out' '$T/c1.err'" || return 1
  need "with an exported env the runner still exits 0 and all three shapes ran (got $rc)" sh -c "[ $rc -eq 0 ] && [ \"\$(grep -c '^shape=' '$T/c1.out')\" -eq 3 ]" || return 1
  need "with an exported env the factual line still names the bash version" grep -Eq '^factual: BASH_COMPAT=32 is bash [0-9]' "$T/c1.out" || return 1
  # an exported function named command (the block lists the environment with `command env`: it must be gone before that)
  rc=0
  env TMPDIR="$T/tmp" 'BASH_FUNC_command%%=() { builtin echo HOSTILE-command-RAN; return 0; }' $sut "$L/mac-port-run" "$L/mac-port-fixtures/probe-shape.sh" > "$T/c3.out" 2> "$T/c3.err" || rc=$?
  need "an exported command never ran ($(grep -h HOSTILE "$T/c3.out" "$T/c3.err" | head -n 1))" sh -c "! grep -q HOSTILE-command-RAN '$T/c3.out' '$T/c3.err'" || return 1
  need "with an exported command the runner still exits 0 and all three shapes ran (got $rc)" sh -c "[ $rc -eq 0 ] && [ \"\$(grep -c '^shape=' '$T/c3.out')\" -eq 3 ]" || return 1
  # an exported [ that answers true to [ ! -L x ] and passes everything else to the real builtin, with a link that points OUTSIDE the fixtures directory
  rc=0
  env TMPDIR="$T/tmp" 'BASH_FUNC_[%%=() { case "$*" in "! -L "*) return 0 ;; esac; builtin [ "$@"; }' $sut "$L/mac-port-run" "$L/mac-port-fixtures/escape.sh" > "$T/c2.out" 2> "$T/c2.err" || rc=$?
  need "with an exported [ the link out of the fixtures directory is still refused: exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  case $sut in
    sh) need "under sh the [ is imported and removed: the runner itself refuses the link, and says so" grep -qi 'symbolic link' "$T/c2.err" || return 1 ;;
  esac # (under bash --posix the shell refuses to import a function named [ and the block refuses the entry up front, see PS-27)
  need "and the file outside the fixtures directory did not run" sh -c "[ ! -e '$T/canary.ran' ] && ! grep -q OUTSIDE-FILE-RAN '$T/c2.out' '$T/c2.err'" || return 1
}
ps_23() { # hale MP-1, the listing must work: the runner lists the exported functions with env and sed; a listing that FAILS (or prints and then fails) must refuse: exit 2, problem and fix, no shape run, no private directory made
  new_test
  L=$T/lane/scripts
  mkdir -p "$L/mac-port-fixtures"
  cp "$runner" "$L/mac-port-run"
  chmod 0755 "$L/mac-port-run"
  cp "$fx/probe-shape.sh" "$L/mac-port-fixtures/probe-shape.sh"
  for t in sed env; do
    mkdir -p "$T/h-$t"
    cat > "$T/h-$t/$t" <<'FAKEHYG'
#!/bin/sh
case ${FAKE_HYG_MODE:-} in
  fail) exit 1 ;;
  partial) "$FAKE_HYG_REAL" "$@"; exit 1 ;;
  first-fail | first-partial)
    n=$(cat "$FAKE_HYG_COUNT" 2> /dev/null || echo 0)
    n=$((n + 1))
    echo "$n" > "$FAKE_HYG_COUNT"
    if [ "$n" -eq 1 ]; then
      if [ "$FAKE_HYG_MODE" = first-partial ]; then "$FAKE_HYG_REAL" "$@"; fi
      exit 1
    fi ;;
esac
exec "$FAKE_HYG_REAL" "$@"
FAKEHYG
    chmod 0755 "$T/h-$t/$t"
    real=$(command -v "$t")
    for m in fail partial first-fail first-partial; do
      tmpd=$T/tmp-$t$m
      mkdir -p "$tmpd"
      rc=0
      env PATH="$T/h-$t:$PATH" TMPDIR="$tmpd" FAKE_HYG_MODE="$m" FAKE_HYG_COUNT="$T/cnt-r$t$m" FAKE_HYG_REAL="$real" 'BASH_FUNC_zzhost%%=() { :; }' $sut "$L/mac-port-run" "$L/mac-port-fixtures/probe-shape.sh" > "$T/i$t$m.out" 2> "$T/i$t$m.err" || rc=$?
      need "a $t that $m: the runner refuses with exit 2 (got $rc)" test "$rc" -eq 2 || return 1
      need "a $t that $m: the message has problem and fix and says it cannot list the environment ($(head -n 2 "$T/i$t$m.err" | tr '\n' '|'))" sh -c "grep -q 'problem:' '$T/i$t$m.err' && grep -q 'fix:' '$T/i$t$m.err' && grep -qi 'environment' '$T/i$t$m.err'" || return 1
      need "a $t that $m: no shape ran and nothing was printed as a result" sh -c "! grep -q '^shape=' '$T/i$t$m.out' && ! grep -q 'mac-port-run: shape' '$T/i$t$m.out'" || return 1
      need "a $t that $m: no private directory was made ($(ls -A "$tmpd" | tr '\n' ' '))" test -z "$(ls -A "$tmpd")" || return 1
    done
    # the control: the stand-in that delegates changes nothing
    rc=0
    env PATH="$T/h-$t:$PATH" TMPDIR="$T/tmp" FAKE_HYG_MODE="" FAKE_HYG_REAL="$real" 'BASH_FUNC_zzhost%%=() { :; }' $sut "$L/mac-port-run" "$L/mac-port-fixtures/probe-shape.sh" > "$T/c$t.out" 2> "$T/c$t.err" || rc=$?
    need "a $t that works (the stand-in delegates): the runner exits 0 and all three shapes ran (got $rc): $(head -n 2 "$T/c$t.err" | tr '\n' '|')" sh -c "[ $rc -eq 0 ] && [ \"\$(grep -c '^shape=' '$T/c$t.out')\" -eq 3 ]" || return 1
  done
}
ps_24() { # hale MP-2: a PATH with an empty or a relative element is refused (a fixture's commands could resolve from the working directory): exit 2, problem and fix, nothing run, nothing made
  new_test
  L=$T/lane/scripts
  mkdir -p "$L/mac-port-fixtures"
  cp "$runner" "$L/mac-port-run"
  chmod 0755 "$L/mac-port-run"
  cp "$fx/probe-shape.sh" "$L/mac-port-fixtures/probe-shape.sh"
  n=0
  for pv in "$PATH:" ":$PATH" "$PATH::/usr/bin" "bin:$PATH" ".:$PATH" "$PATH:./x" "~/bin:$PATH"; do
    n=$((n + 1))
    tmpd=$T/tmp-p$n
    mkdir -p "$tmpd"
    rc=0
    env PATH="$pv" TMPDIR="$tmpd" $sut "$L/mac-port-run" "$L/mac-port-fixtures/probe-shape.sh" > "$T/p$n.out" 2> "$T/p$n.err" || rc=$?
    need "PATH variant $n is refused with exit 2 (got $rc)" test "$rc" -eq 2 || return 1
    need "PATH variant $n: the message names the PATH, with problem and fix ($(head -n 1 "$T/p$n.err"))" sh -c "grep -q 'problem:' '$T/p$n.err' && grep -q 'fix:' '$T/p$n.err' && grep -q 'PATH' '$T/p$n.err'" || return 1
    need "PATH variant $n: no shape ran and no private directory was made ($(ls -A "$tmpd" | tr '\n' ' '))" sh -c "! grep -q '^shape=' '$T/p$n.out' && [ -z \"\$(ls -A '$tmpd')\" ]" || return 1
  done
  # the control: an all-absolute PATH (the caller's own, as it is) runs
  rc=0
  env TMPDIR="$T/tmp" $sut "$L/mac-port-run" "$L/mac-port-fixtures/probe-shape.sh" > "$T/pk.out" 2> "$T/pk.err" || rc=$?
  need "the caller's absolute PATH runs all three shapes (got $rc)" sh -c "[ $rc -eq 0 ] && [ \"\$(grep -c '^shape=' '$T/pk.out')\" -eq 3 ]" || return 1
}
ps_25() { # hale MP-3: a closing terminal (SIGHUP) removes the runner's private directory too, and the runner exits 129
  new_test
  L=$T/lane/scripts
  mkdir -p "$L/mac-port-fixtures"
  cp "$runner" "$L/mac-port-run"
  chmod 0755 "$L/mac-port-run"
  # a fixture that records its pid next to the private directory (TMPDIR is one of the four variables the clean room keeps) and waits; exec, so that the pid IS the waiting process
  cat > "$L/mac-port-fixtures/sleeper.sh" <<'SLEEPER'
#!/bin/bash
echo $$ > "$TMPDIR/sleeper.pid"
exec sleep 20
SLEEPER
  chmod 0755 "$L/mac-port-fixtures/sleeper.sh"
  # started with SIGHUP at its default (a caller that ignores it, nohup for one, cannot be trapped: python resets it before it execs)
  python3 -I -c 'import os, signal, sys; signal.signal(signal.SIGHUP, signal.SIG_DFL); os.execvp(sys.argv[1], sys.argv[1:])' env TMPDIR="$T/tmp" $sut "$L/mac-port-run" "$L/mac-port-fixtures/sleeper.sh" > "$T/h.out" 2> "$T/h.err" &
  rp=$!
  i=0
  while [ ! -s "$T/tmp/sleeper.pid" ] && [ "$i" -lt 100 ]; do
    sleep 0.1
    i=$((i + 1))
  done
  need "the fixture started and recorded its pid" test -s "$T/tmp/sleeper.pid" || { kill "$rp" 2> /dev/null || true; return 1; }
  sp=$(cat "$T/tmp/sleeper.pid")
  kill -HUP "$rp" 2> /dev/null || true # what a closing terminal does: the whole foreground group gets it, the runner and the fixture
  kill -HUP "$sp" 2> /dev/null || true
  rc=0
  { wait "$rp" || rc=$?; } 2> /dev/null # the shell's own "Hangup" report of the job is not output of the cell
  if kill -0 "$sp" 2> /dev/null; then kill "$sp" 2> /dev/null || true; fi
  need "the runner exits 129 after SIGHUP (got $rc)" test "$rc" -eq 129 || return 1
  need "and its private directory is gone ($(ls -A "$T/tmp" | tr '\n' ' '))" test -z "$(ls -A "$T/tmp" | grep '^mac-port-run[.]' || true)" || return 1
}
ps_26() { # hale MP-1, the test script itself: exported functions must not replace its commands either (a printf, echo and cd that leave a canary; not a [: bash --posix refuses to import a function named [, and its child shells would still import the exported variable): the first line is still the LIMIT, the cell passes, no canary
  new_test
  : > "$T/canary26"
  rc=0
  env CORTEX_PS_TMP="$T/nested" CANARY="$T/canary26" 'BASH_FUNC_printf%%=() { builtin echo printf >> "$CANARY"; builtin printf "$@"; }' 'BASH_FUNC_echo%%=() { builtin echo echo >> "$CANARY"; builtin echo "$@"; }' 'BASH_FUNC_cd%%=() { builtin echo cd >> "$CANARY"; builtin cd "$@"; }' $sut "$here/mac-port-test.sh" PS-17 > "$T/n.out" 2> "$T/n.err" || rc=$?
  need "the nested test script exits 0 and its cell passes (got $rc): $(tail -n 2 "$T/n.err" | tr '\n' '|')" sh -c "[ $rc -eq 0 ] && grep -qx 'PS-17 PASS' '$T/n.out'" || return 1
  need "its first line is still the LIMIT" test "$(head -n 1 "$T/n.out")" = "$LIMIT" || return 1
  need "no exported function ran ($(tr '\n' ' ' < "$T/canary26"))" test ! -s "$T/canary26" || return 1
  # the listing must work in the test script too: an env or a sed that fails, or prints and then fails, refuses (exit 2, problem and fix) before anything is printed
  for t in sed env; do
    mkdir -p "$T/h-$t"
    cat > "$T/h-$t/$t" <<'FAKEHYG'
#!/bin/sh
case ${FAKE_HYG_MODE:-} in
  fail) exit 1 ;;
  partial) "$FAKE_HYG_REAL" "$@"; exit 1 ;;
  first-fail | first-partial)
    n=$(cat "$FAKE_HYG_COUNT" 2> /dev/null || echo 0)
    n=$((n + 1))
    echo "$n" > "$FAKE_HYG_COUNT"
    if [ "$n" -eq 1 ]; then
      if [ "$FAKE_HYG_MODE" = first-partial ]; then "$FAKE_HYG_REAL" "$@"; fi
      exit 1
    fi ;;
esac
exec "$FAKE_HYG_REAL" "$@"
FAKEHYG
    chmod 0755 "$T/h-$t/$t"
    real=$(command -v "$t")
    for m in fail partial first-fail first-partial; do
      rc=0
      env PATH="$T/h-$t:$PATH" FAKE_HYG_MODE="$m" FAKE_HYG_COUNT="$T/cnt-t$t$m" FAKE_HYG_REAL="$real" 'BASH_FUNC_zzhost%%=() { :; }' CORTEX_PS_TMP="$T/nested-$t$m" $sut "$here/mac-port-test.sh" PS-17 > "$T/m$t$m.out" 2> "$T/m$t$m.err" || rc=$?
      need "a $t that $m: the test script refuses with exit 2 (got $rc)" test "$rc" -eq 2 || return 1
      need "a $t that $m: the message has problem and fix and says it cannot list the environment ($(head -n 1 "$T/m$t$m.err"))" sh -c "grep -q 'problem:' '$T/m$t$m.err' && grep -q 'fix:' '$T/m$t$m.err' && grep -qi 'environment' '$T/m$t$m.err'" || return 1
      need "a $t that $m: nothing was printed on standard output (the LIMIT comes after the block)" test ! -s "$T/m$t$m.out" || return 1
    done
  done
}
ps_27() { # sam M1A-FU-SAM01: an exported function entry that this shell could not import cannot be unset and stays in the environment for child shells: it must be refused (exit 2, problem and fix) before any child shell, in the runner and in the test script
  new_test
  L=$T/lane/scripts
  mkdir -p "$L/mac-port-fixtures"
  cp "$runner" "$L/mac-port-run"
  chmod 0755 "$L/mac-port-run"
  cp "$fx/probe-shape.sh" "$L/mac-port-fixtures/probe-shape.sh"
  : > "$T/canary27"
  # (1) an entry that NO shell can import (a name with a slash): refused by the runner and by the test script, in both modes
  rc=0
  env TMPDIR="$T/tmp" 'BASH_FUNC_a/b%%=() { :; }' $sut "$L/mac-port-run" "$L/mac-port-fixtures/probe-shape.sh" > "$T/u1.out" 2> "$T/u1.err" || rc=$?
  need "the runner refuses an exported entry that no shell can import: exit 2 (got $rc)" test "$rc" -eq 2 || return 1
  need "and the message has problem and fix and names the entry ($(head -n 2 "$T/u1.err" | tr '\n' '|'))" sh -c "grep -q 'problem:' '$T/u1.err' && grep -q 'fix:' '$T/u1.err' && grep 'problem:' '$T/u1.err' | grep -qF 'cannot remove' && grep 'problem:' '$T/u1.err' | grep -qF 'a/b'" || return 1
  need "and no shape ran" sh -c "! grep -q '^shape=' '$T/u1.out'" || return 1
  rc=0
  env CORTEX_PS_TMP="$T/nested1" 'BASH_FUNC_a/b%%=() { :; }' $sut "$here/mac-port-test.sh" PS-17 > "$T/u2.out" 2> "$T/u2.err" || rc=$?
  need "the test script refuses it too: exit 2 (got $rc), problem and fix, the entry named" sh -c "[ $rc -eq 2 ] && grep -q 'problem:' '$T/u2.err' && grep -q 'fix:' '$T/u2.err' && grep 'problem:' '$T/u2.err' | grep -qF 'cannot remove' && grep 'problem:' '$T/u2.err' | grep -qF 'a/b'" || return 1
  need "and it printed nothing on standard output (the LIMIT comes after the block)" test ! -s "$T/u2.out" || return 1
  # (2) a [ that leaves a canary: under sh it is imported and removed (the valid control: everything runs, no canary); under bash --posix the shell REFUSES to import a function named [, so the entry would reach every child shell: it is refused, no canary
  rc=0
  env TMPDIR="$T/tmp" CANARY="$T/canary27" 'BASH_FUNC_[%%=() { builtin echo "[" >> "$CANARY"; builtin [ "$@"; }' $sut "$L/mac-port-run" "$L/mac-port-fixtures/probe-shape.sh" > "$T/u3.out" 2> "$T/u3.err" || rc=$?
  case $sut in
    sh) need "the runner under sh imports and removes the [: exit 0 and all three shapes ran (got $rc)" sh -c "[ $rc -eq 0 ] && [ \"\$(grep -c '^shape=' '$T/u3.out')\" -eq 3 ]" || return 1 ;;
    *) need "the runner under bash --posix refuses the [ it could not import: exit 2 (got $rc), problem and fix, the entry named" sh -c "[ $rc -eq 2 ] && grep -q 'problem:' '$T/u3.err' && grep -q 'fix:' '$T/u3.err' && grep 'problem:' '$T/u3.err' | grep -qF 'cannot remove' && grep 'problem:' '$T/u3.err' | grep -qF '['" || return 1 ;;
  esac
  need "no [ ran in the runner or in a child ($(tr '\n' ' ' < "$T/canary27"))" test ! -s "$T/canary27" || return 1
  rc=0
  env CORTEX_PS_TMP="$T/nested2" CANARY="$T/canary27" 'BASH_FUNC_[%%=() { builtin echo "[" >> "$CANARY"; builtin [ "$@"; }' $sut "$here/mac-port-test.sh" PS-17 > "$T/u4.out" 2> "$T/u4.err" || rc=$?
  case $sut in
    sh) need "the test script under sh imports and removes the [: exit 0 and its cell passes, its child shells included (got $rc)" sh -c "[ $rc -eq 0 ] && grep -qx 'PS-17 PASS' '$T/u4.out'" || return 1 ;;
    *) need "the test script under bash --posix refuses the [ it could not import: exit 2 (got $rc), problem and fix, the entry named, nothing on standard output" sh -c "[ $rc -eq 2 ] && grep -q 'problem:' '$T/u4.err' && grep -q 'fix:' '$T/u4.err' && grep 'problem:' '$T/u4.err' | grep -qF 'cannot remove' && grep 'problem:' '$T/u4.err' | grep -qF '[' && [ ! -s '$T/u4.out' ]" || return 1 ;;
  esac
  need "no [ ran in the nested test script or in one of its child shells ($(tr '\n' ' ' < "$T/canary27"))" test ! -s "$T/canary27" || return 1
}
ps_28() { # hale M2-1: the PATH refusal comes BEFORE anything is looked up through the PATH: with a . in the PATH a hostile env and sed in the working directory must not run, in the runner and in the test script
  new_test
  L=$T/lane/scripts
  mkdir -p "$L/mac-port-fixtures" "$T/cwd"
  cp "$runner" "$L/mac-port-run"
  chmod 0755 "$L/mac-port-run"
  cp "$fx/probe-shape.sh" "$L/mac-port-fixtures/probe-shape.sh"
  for t in env sed; do
    printf '#!/bin/sh\necho %s-RAN >> "$CANARY"\nexit 1\n' "$t" > "$T/cwd/$t"
    chmod 0755 "$T/cwd/$t"
  done
  : > "$T/canary28"
  rc=0
  (cd "$T/cwd" && env PATH=".:$PATH" CANARY="$T/canary28" TMPDIR="$T/tmp" $sut "$L/mac-port-run" "$L/mac-port-fixtures/probe-shape.sh") > "$T/r.out" 2> "$T/r.err" || rc=$?
  need "the runner refuses a PATH with a . in it: exit 2, problem and fix naming the PATH (got $rc: $(head -n 1 "$T/r.err"))" sh -c "[ $rc -eq 2 ] && grep -q 'problem:' '$T/r.err' && grep -q 'fix:' '$T/r.err' && grep 'problem:' '$T/r.err' | grep -q 'PATH'" || return 1
  need "and the env and sed in the working directory never ran ($(tr '\n' ' ' < "$T/canary28"))" test ! -s "$T/canary28" || return 1
  need "and no shape ran" sh -c "! grep -q '^shape=' '$T/r.out'" || return 1
  rc=0
  (cd "$T/cwd" && env PATH=".:$PATH" CANARY="$T/canary28" CORTEX_PS_TMP="$T/nested28" $sut "$here/mac-port-test.sh" PS-17) > "$T/t.out" 2> "$T/t.err" || rc=$?
  need "the test script refuses it too: exit 2, problem and fix naming the PATH (got $rc: $(head -n 1 "$T/t.err"))" sh -c "[ $rc -eq 2 ] && grep -q 'problem:' '$T/t.err' && grep -q 'fix:' '$T/t.err' && grep 'problem:' '$T/t.err' | grep -q 'PATH'" || return 1
  need "and the env and sed in the working directory never ran there either ($(tr '\n' ' ' < "$T/canary28"))" test ! -s "$T/canary28" || return 1
  need "and it printed nothing on standard output (the LIMIT comes after the block)" test ! -s "$T/t.out" || return 1
  # the refusal itself is printed by the builtin (command printf), not by an exported printf function that is not removed yet
  : > "$T/canary28"
  rc=0
  (cd "$T/cwd" && env PATH=".:$PATH" CANARY="$T/canary28" TMPDIR="$T/tmp" 'BASH_FUNC_printf%%=() { builtin echo printf-RAN >> "$CANARY"; builtin printf "$@"; }' $sut "$L/mac-port-run" "$L/mac-port-fixtures/probe-shape.sh") > "$T/pr.out" 2> "$T/pr.err" || rc=$?
  need "with an exported printf too the runner refuses the PATH (exit 2, got $rc) and the exported printf never ran ($(tr '\n' ' ' < "$T/canary28"))" sh -c "[ $rc -eq 2 ] && grep 'problem:' '$T/pr.err' | grep -q 'PATH' && [ ! -s '$T/canary28' ]" || return 1
  rc=0
  (cd "$T/cwd" && env PATH=".:$PATH" CANARY="$T/canary28" CORTEX_PS_TMP="$T/nested28b" 'BASH_FUNC_printf%%=() { builtin echo printf-RAN >> "$CANARY"; builtin printf "$@"; }' $sut "$here/mac-port-test.sh" PS-17) > "$T/pt.out" 2> "$T/pt.err" || rc=$?
  need "and so does the test script (exit 2, got $rc) without the exported printf running ($(tr '\n' ' ' < "$T/canary28"))" sh -c "[ $rc -eq 2 ] && grep 'problem:' '$T/pt.err' | grep -q 'PATH' && [ ! -s '$T/canary28' ]" || return 1
  # the test script refuses an EMPTY element too (a trailing colon, a leading colon, a double colon), not only a relative one
  n=0
  for pv in "$PATH:" ":$PATH" "$PATH::/usr/bin"; do
    n=$((n + 1))
    rc=0
    env PATH="$pv" CORTEX_PS_TMP="$T/nested28c$n" $sut "$here/mac-port-test.sh" PS-17 > "$T/e$n.out" 2> "$T/e$n.err" || rc=$?
    need "the test script refuses a PATH with an empty element (variant $n): exit 2 with problem and fix (got $rc)" sh -c "[ $rc -eq 2 ] && grep -q 'problem:' '$T/e$n.err' && grep -q 'fix:' '$T/e$n.err'" || return 1
  done
}

# ---- run -------------------------------------------------------------------------------------------------

all_ids="PS-01 PS-02 PS-03 PS-04 PS-05 PS-06 PS-07 PS-08 PS-09 PS-10 PS-11 PS-12 PS-13 PS-14 PS-15 PS-16 PS-17 PS-18 PS-19 PS-20 PS-21 PS-22 PS-23 PS-24 PS-25 PS-26 PS-27 PS-28"
[ -n "$want" ] || want=$all_ids
trap cleanup_made EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
pass=0
fail=0
failed=""
for id in $want; do
  fn=$(printf '%s' "$id" | tr 'A-Z-' 'a-z_')
  if ! command -v "$fn" > /dev/null 2>&1; then
    echo "$id MISSING (no such cell)"
    fail=$((fail + 1))
    failed="$failed $id"
    continue
  fi
  if "$fn"; then
    echo "$id PASS"
    pass=$((pass + 1))
  else
    echo "$id FAIL"
    fail=$((fail + 1))
    failed="$failed $id"
  fi
done
if [ "$fail" -eq 0 ]; then
  echo "SUMMARY under '$sut': $pass passed, 0 failed"
  exit 0
fi
echo "SUMMARY under '$sut': $pass passed, $fail failed ($failed )"
exit 1
