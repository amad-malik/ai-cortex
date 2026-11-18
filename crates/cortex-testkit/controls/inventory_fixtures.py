"""K40 (a), r10: the capture inventory's own fixtures, run on the inventory UNDER TEST.

Each fixture is a few lines of shell appended to a copy of the crate's controls.sh (with the crate's
supervise.sh and controls/ beside it, so the copy alone PASSES). A BAD fixture must make the
inventory FAIL (exit 1); a GOOD one must leave it PASSING (exit 0). The baseline copy, with nothing
appended, must PASS. Standard library only; run as `python3 -I`. The shell is never executed.

Usage: python3 -I inventory_fixtures.py INVENTORY_PY CRATE_DIR
"""

import os
import shutil
import subprocess
import sys
import tempfile

BAD = [   # (name, text): each must FAIL the inventory
    ('assignment', 'x=$(foo)'),
    ('test', '[ "$(state c)" = absent ] && echo gone'),
    ('then-position', 'if [ -n "$left" ]; then st=$(state c); fi'),
    ('arithmetic (mike: took=$(( $(up_ms) - t0 )))', 't=$(( $(up_ms) - t0 ))'),
    ('arithmetic, two captures (r9 K21)', 'ms=$(( $(date -d "$t1" +%s%3N) - $(date -d "$t0" +%s%3N) ))'),
    ('an effect in text (vera: an unchecked rescue)', 'echo "follow-up: $(sig KILL "$ID" member,leader)" >> notes'),
    ('an effect nested in a pure text capture', 'echo "x: $(echo $(sig KILL f))"'),
    ('a for list', 'for i in $(seq 1 3); do :; done'),
    ('printf -v', 'printf -v v \'%s\' "$(ts)"'),
    ('an append to a non-accumulator', 'foo="$foo $(ts)"'),
    ("a capture as a command's argument", 'remove_exact "$(cat f)" n'),
    ('a pipeline decides (r9 wait_gone)', "while tr '\\0' ' ' < /proc/1/cmdline | grep -q x; do sleep 0.1; done"),
    ('a pipeline, |&', 'foo |& grep -q x && echo y'),
    ('a process substitution', 'while read -r l; do :; done < <(ls)'),
    ('an unpinned here-string (r9 K35)', 'IFS=: read -r t p q <<<"$w"'),
    ('a capture after a multi-line single-quoted string', "awk '\n{ print }' f; r=$(sig KILL f)"),
    ('a capture continued across lines', 'echo "a $(echo b\nsig KILL f)"'),
    ('a rescue gated on = present', '[ "$st" = present ] && remove_exact "$C" n'),
    ("verdict's argument 4 decides INVALID (vera)", 'verdict n 0 "measured" "$(head -1 invalid-reason)"'),
    ('a compound effect in output text (vera)', 'echo "report: $(echo noted; sig KILL "$ID" member,leader)"'),
    ('a compound past a PURE first word (mike)', 'echo "note $(echo ok; sig KILL f)"'),
    ('an append that grows only by its capture (mike)', 'bad="$bad$(ts)"'),
    ("verdict's status argument (mike)", 'verdict n "$(echo 0)" "measured"'),
    ('a compound: &&', 'echo "a $(echo ok && sig KILL f)"'),
    ('a compound: ||', 'echo "a $(echo ok || sig KILL f)"'),
    ('a compound: a pipeline into an effect', 'echo "a $(echo ok | sig KILL f)"'),
    ("verdict's name argument", 'verdict "$(echo n)" 0 "measured"'),
    ('a command runner in text (wcap)', 'echo "x $(wcap r sig KILL f)"'),
    ('awk can run a command', 'echo "x $(awk \'BEGIN { system("true") }\' /dev/null)"'),
    ('a write inside a text capture', 'echo "x $(echo y > f)"'),
    ("a capture as an output's redirection target", 'echo "x" > "$(echo f)"'),
    ('a PURE name redefined as a function', 'head() { sig KILL f; }'),
    # r12 (kai: DEFAULT-DENY): seven r10/r11 GOOD lines whose capture bodies the controls do not use. Under an
    # allowlist of exact bodies each is an UNREVIEWED TEXT capture: kept, now BAD; each has a corpus-form GOOD partner
    ('unreviewed body (r12; r11 GOOD: an accumulator)', 'bad="$bad x($(head -1 f))"'),
    ("unreviewed body (r12; r11 GOOD: a verdict's text)", 'verdict n 0 "$(tr \'\\n\' \' \' < f)"'),
    ('unreviewed body (r12; r11 GOOD: a PURE pipeline)', 'echo "$(sha256sum f | cut -c1-16)"'),
    ('unreviewed body (r12; r11 GOOD: a quoted bar, and stderr to /dev/null)', 'echo "$(grep -oE \'a|b\' f 2>/dev/null | head -1)"'),
    ("unreviewed body (r12; r11 GOOD: verdict's argument 3, a plain argument 4)", 'verdict n $? "x $(head -1 f)" "$ir"'),
    ('unreviewed body (r12; r11 GOOD: an input redirect in the body, an output redirect after)', 'echo "$(tr \'\\n\' \' \' < f)" >> "$OUT/notes"'),
    ('unreviewed body (r12; r11 GOOD: an append with literal text)', 'fu="$fu $n: $(tr \'\\n\' \' \' < f);"'),
    # r12: vera's four forms (r11 recheck) and mike's three that are not among hers (F1B-R10-LEXICAL-LIMIT)
    ('a >& write inside a text body (vera; mike)', 'echo "x $(echo y >& f)"'),
    ('printf with two spaces before -v (vera; mike)', 'printf  -v bad \'%s\' "$(echo failure)"'),
    ('an attached redirection filename capture (vera)', 'echo x>f"$(echo g)"'),
    ('a PURE name redefined with function and no parentheses (vera; mike)', 'function head { sig KILL f; }\necho "$(head)"'),
    ("a redirect before verdict's name shifts its arguments (mike)", 'verdict >notes n "$(echo 0)" "measured"'),
    ('a <> redirection target (mike)', 'echo "record" <> "$(head -1 target-path)"'),
    ('a process substitution inside a text capture (mike)', 'echo "note $(head -1 <(sig KILL f))"'),
    # r12: the two forms K40 does not model, refused by default (UNMODELLED) rather than left as limits
    ('a backtick command substitution', 'echo "x `sig KILL f`"'),
    ('eval', 'eval "sig KILL f"'),
    # r12: an ALLOWLISTED body (ts) in a shape that is not allowed: the shapes are default-deny too
    ('ts in printf with two spaces before -v', 'printf  -v bad \'%s\' "$(ts)"'),
    ("ts after a redirect before verdict's name", 'verdict >notes "$n" $? "$(ts)"'),
    ("ts in verdict's argument 4", 'verdict "$n" $? "measured" "$(ts)"'),
    ('ts as a <> redirection target', 'echo "x" <> "$(ts)"'),
    ('ts in text with an unreviewed > redirect', 'echo "$(ts) x" > "$f"'),
    # r13 (vera's F1B-R12-K40-TS-CALLEE; mike's first retained input): the trusted helper ts calls date; redefining
    # that direct callee turns an allowlisted ts capture into an effect
    ("date, ts's callee, redefined (vera)", 'date() { : > "$OUT/date-effect"; }\necho "$(ts) note"'),
    ("date, ts's callee, redefined (mike)", 'date() { printf \'%s\\n\' changed > "$OUT/marker"; }\necho "$(ts) note"'),
]
GOOD = [   # (name, text): each must leave the inventory PASSING
    ('a timestamp in text', 'echo "$(ts) note" >> notes'),
    ('a case pattern', 'case $x in a|b) : ;; esac'),
    ('an or-list', '[ -n "$a" ] || [ -n "$b" ]'),
    ('single-quoted text', "echo 'literal $(sig KILL f) | x'"),
    ('arithmetic without a capture', 'echo "${x:-?} $(( 1 + 2 ))"; n=$(( n + 1 ))'),
    ('a multi-line single-quoted program', "awk '\n$1 == g { print $2 }' f"),
    ('a pipe inside text', 'echo "a | b"'),
    # r12: the corpus forms (exact allowlisted bodies, in the allowed shapes), the partners of the lines above
    ('an accumulator (controls.sh K28)', 'bad="$bad real(rc $r, members \'$(echo $x)\')"'),
    ("a verdict's text (controls.sh K27)", 'verdict "$n" $? "$(tr \'\\n\' \' \' < "$OUT/$n.stdout")"'),
    ('a PURE pipeline (controls.sh header line)', 'echo "# controls, supervisor $SUP sha256 $(sha256sum "$SUP" | cut -c1-16)"'),
    ('stderr to /dev/null in a pipeline (controls.sh observe_fault_case)', 'verdict "$name" $? "logged \'$(grep -oE "$want" "$OUT/$name-stopped.stderr" 2>/dev/null | head -1)\'" "$inv"'),
    ("verdict's argument 3, a plain argument 4 (controls.sh K40)", 'verdict "$n" $? "(b) $(head -1 "$OUT/$n.stdout")" "$ir"'),
    ('an output redirect after the text, to the notes file (controls.sh)', 'echo "$(ts) $n: note" >> "$OUT/$n.notes"'),
    ('an append with literal text (controls.sh K24)', 'fu="$fu $n: $(tr \'\\n\' \' \' < "$OUT/$n.guard");"'),
    ('partner: a text body with no write', 'echo "x $(ts)" >> "$f"'),
    ('partner: echo, not printf -v', 'echo "$(ts) failure" >> "$OUT/$n.notes"'),
    ('partner: a quoted argument and a separate redirect', 'echo "x $(ts)" >> "$4"'),
    ('partner: a function that does not redefine a PURE name', 'note() { echo "$(ts) x"; }'),
    ("partner: verdict's name first", 'verdict "$n" $? "measured $(head -1 "$OUT/$n.stdout")"'),
    ('partner: >> to a named file', 'echo "record $(ts)" >> "$OUT/$n.notes"'),
    ('partner: a file argument, not a process substitution', 'echo "note $(head -1 "$OUT/$n.stdout")"'),
    ('partner: a backtick inside single quotes is text', "echo 'a `literal` in single quotes'"),
    ('partner: the word eval inside text', 'echo "$(ts) evaluate" >> "$OUT/$n.notes"'),
    ('partner: the healthy ts, date not redefined', 'echo "$(ts) note"'),
]


def run(inv, root, extra):
    with tempfile.TemporaryDirectory(prefix="cv2-k40-") as d:
        shutil.copy2(os.path.join(root, "supervise.sh"), d)
        shutil.copytree(os.path.join(root, "controls"), os.path.join(d, "controls"),
                        ignore=shutil.ignore_patterns("__pycache__"))
        if extra:
            with open(os.path.join(d, "controls", "controls.sh"), "a") as f:
                f.write("\n" + extra + "\n")
        p = subprocess.run([sys.executable, "-I", inv, d], capture_output=True, text=True, timeout=60)
        return p.returncode, p.stdout.split("\n", 1)[0][:200]


def main():
    inv, root = os.path.abspath(sys.argv[1]), os.path.abspath(sys.argv[2])
    wrong = []
    rc, head = run(inv, root, "")
    if rc != 0:
        wrong.append(f"the baseline copy did not PASS (rc {rc}: {head})")
    for name, text in BAD:
        rc, head = run(inv, root, text)
        if rc != 1:
            wrong.append(f"BAD '{name}' was not refused (rc {rc})")
    for name, text in GOOD:
        rc, head = run(inv, root, text)
        if rc != 0:
            wrong.append(f"GOOD '{name}' was refused (rc {rc}: {head})")
    total = 1 + len(BAD) + len(GOOD)
    if wrong:
        print(f"FAIL: {total - len(wrong)} of {total} fixtures as expected ({len(BAD)} bad, {len(GOOD)} good, the baseline); " + "; ".join(wrong))
        return 1
    print(f"PASS: {total} of {total} fixtures as expected: the baseline passes, {len(BAD)} bad refused, {len(GOOD)} good passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
