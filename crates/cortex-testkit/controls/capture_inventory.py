"""K40 (r9; r10 strengthened BY CONSTRUCTION: mike's timing finding, vera's F1B-R9-OWNED-RESCUE-CAPTURE):
an inventory of EVERY Bash command substitution in cortex-testkit, each judged by its POSITION;
r12: its TEXT class is an exact allowlist (default-deny).

Standard library only; run as `python3 -I`. Nothing is executed.

The failure model (accepted at r7 to r9): a command substitution whose pipe cannot be made yields
EMPTY TEXT WITH ASSIGNMENT STATUS 0, although its command never ran. r9 classified captures by
their command's name, which does not say how the result is consumed (mike: an `up_ms` capture fed
an elapsed time; vera: `exists`, `sig` and `date` captures fed rescues, effects and timing). r10
judges each capture by WHERE it stands, so that the rule holds by construction:

THE SHELL FILES are discovered (every `*.sh`, and every file whose first line is a shell shebang,
under the crate; target/, .git/ and __pycache__/ skipped), not listed.

PRODUCTION (`supervise.sh`): its captures are PINNED, each to one reviewed body and class (mike's
finite production disposition at r8, and the two witnessed queries). Any other capture, or a pinned
one missing, is a FAIL.

THE FAKES (`controls/fake-podman/*`): fault injectors by design: FAKE.

THE CONTROLS (every other shell file): a capture may stand ONLY
  - as the WITNESSED body of `wcap` (controls.sh) or of the observer's outer capture (observer.sh),
    whose witness is checked outside the capture;
  - as the ANCHORED inner listing of `obs_listing` (an empty listing fails its own-row anchor);
  - as TEXT, which is an ALLOWLIST (r12, kai: DEFAULT-DENY, after r10 and r11 showed that a deny-list
    of syntax can always be extended by one more Bash form): a capture is TEXT only if its body is
    EXACTLY one of TEXT_BODIES (the ten bodies of the controls' 22 TEXT occurrences) AND it stands in
    one of three exact command shapes, judged by whole words (text_shape):
        echo "TEXT" [>> TARGET]          verdict NAME $? "TEXT" ["WHY"]          VAR="$VAR<literal>…"
    Any other quoted capture is UNCLASSIFIED: a FAIL. A new TEXT capture is a REVIEWED ADDITION to
    TEXT_BODIES, never a widened pattern. The command names these rely on (TRUSTED: every command
    of an allowlisted body, verdict, and, r13, `date`, the direct callee of the pinned ts; the judge
    checks this itself) may not be redefined: no function definition of one by either form
    (`NAME()`, `function NAME`, with or without parentheses), except ts at its pinned body and
    verdict once in controls.sh, and no alias, unalias, enable or expand_aliases in a control
    (REDEFINED: a FAIL).
A capture anywhere else (an assignment, a test, a case word, a for list, a command's argument, and
EVERY capture inside an arithmetic expansion, which r9 skipped: mike's `took=$(( $(up_ms) - t0 ))`)
is in a DECISION position: a FAIL. Every value a decision uses comes instead from the capture-free
helpers of controls/consumers.sh, and every effect runs as a direct command.

QUOTES ARE READ FILE-WIDE: each line is scanned in the state it starts in, so the text after a
multi-line single-quoted string (an awk or python program) is code, and a line inside one is not.

Also: no rescue may be gated on `= present` (kept from r9).

THE SAME MODEL WITHOUT A CAPTURE (r10): a PIPE that cannot be made fails the same way, so in the
controls no decision may read a pipeline either. Outside quotes and captures, a control file may
hold no pipeline (`|` or `|&` with a blank beside it) and no process substitution (`<(`, `>(`): a
FAIL (r9's wait_gone read `tr … | grep -q` failing as "the helper has exited"). A here-string
(`<<<`) is allowed only at its pinned sites (HERE), where one that cannot be made means its command
never runs and returns status 1, and the caller fails closed (the observer's two awk programs: an
untrusted listing; self_pgid_into: no anchor group, so no listing is trusted). Any other is a FAIL.

A capture or arithmetic expansion CONTINUED across lines is not modelled, so in a control it is a
FAIL (CONTINUED); so is a backtick command substitution or eval (r12: UNMODELLED). The judge's own
rules are exercised by controls/inventory_fixtures.py (K40 (a)).

NOT CLAIMED: this is a lexical judge, not a Bash parser. Here-documents are not modelled (the
controls use none). A case pattern's `|` is told from a pipeline only by having no blank beside it.
Command resolution by PATH is not modelled (the controls do not change their own PATH). The production supervisor's pipelines are not judged here (its captures are
pinned; its one process substitution, the group line's read, fails closed to NO CUSTODY, 73). It
does not prove what a TEXT string says.

Usage: python3 -I capture_inventory.py CRATE_DIR [--list]
"""

import os
import re
import sys

SKIP = {"target", ".git", "__pycache__"}
# r12 (kai: DEFAULT-DENY). TEXT is an ALLOWLIST, not a deny-list of syntax: a control capture is TEXT only if
# its body is EXACTLY one of these bodies (the ones the controls' 22 TEXT occurrences use) AND it stands in
# one of the three exact command shapes of text_shape(). Anything else is UNCLASSIFIED: a FAIL. A new TEXT
# capture is a REVIEWED ADDITION to this list.
TEXT_BODIES = (
    "ts",
    'grep -oE "$want" "$OUT/$name-stopped.stderr" 2>/dev/null | head -1',
    'sha256sum "$SUP" | cut -c1-16',
    'sha256sum "$FAKE_DIR/podman" | cut -c1-16',
    "tr '\\n' ' ' < \"$OUT/$n.guard\"",
    "tr '\\n' ' ' < \"$OUT/$n.stdout\"",
    "tr '\\n' ' ' < \"$OUT/$n.fixtures\"",
    "echo $x",
    'head -1 "$OUT/$n.stdout"',
    "grep '^mismatches' \"$OUT/$n.stdout\" | cut -c1-3000",
)
# the command names the allowlist and its shapes rely on: no control may define one as a function, except ts
# at its pinned body and verdict once in controls.sh; and no control may alias, unalias or enable a command
# r13 (vera's F1B-R12-K40-TS-CALLEE, mike's first retained input): `date`, the direct callee of the pinned ts, is
# trusted too. main() checks that every command of every allowlisted body, and of ts's pinned body, is TRUSTED,
# so a new allowlist entry cannot leave its commands unprotected.
TRUSTED = {"ts", "grep", "head", "sha256sum", "cut", "tr", "echo", "verdict", "date"}
TS_DEF = "ts() { date -u +%FT%T.%3NZ; }"
# supervise.sh, pinned: (class, regex on the normalised capture body)
PRODUCTION = [
    ("SETUP",     r'^date -u \+%Y%m%dT%H%M%SZ$'),
    ("SETUP",     r"^head -c 64 /dev/urandom \| base64 \| tr -dc 'A-Za-z0-9' \| head -c 24$"),
    ("TEXT",      r'^date -u \+%FT%T\.%3NZ$'),
    ("SETUP",     r'^cd "\$\(dirname "\$0"\)" && pwd$'),
    ("SETUP",     r'^mktemp -d "\$\{TMPDIR:-/tmp\}/cv2-sup\.XXXXXX"$'),
    ("WITNESSED", r"^run_capped \"\$dl\" 20 podman ps -a .*; printf 'list-rc=%s\\n' \"\$\?\"$"),
    ("WITNESSED", r"^run_capped \"\$dl\" 20 podman volume ls .*; printf 'list-rc=%s\\n' \"\$\?\"$"),
    ("WITNESSED", r"^jobs -rp 2>/dev/null; printf 'jobs-rc=%s\\n' \"\$\?\"$"),
    ("POSITIVE",  r"^helper_probe$"),
    ("POSITIVE",  r"^grep '\^final ' \"\$STATUS_FILE\" 2>/dev/null \| tail -1$"),
    ("POSITIVE",  r"^sed -n 's/\.\* rc=\\\(\[0-9\]\[0-9\]\*\\\) \.\*/\\1/p' <<<\"\$line\"$"),
    ("TEXT",      r'^run_capped "\$START_DL" "\$START_S" podman run -d '),
    ("TEXT",      r'^run_capped "\$READY_DL" 5 podman exec "\$NAME" postgres --version 2>/dev/null$'),
]
SUBST = re.compile(r"\$\((?!\()")
# r10: the reviewed here-strings of the controls: (file, regex on the raw line, how many)
HERE_OK = [
    ("controls/observer.sh", r'<<<"\$out"$', 2),
    ("controls/consumers.sh", r'read -ra __f <<<"\$__s"$', 1),
]


def skeleton(text):
    """(skeleton, starts): the file with every quoted string, capture, arithmetic expansion and
    comment blanked (newlines kept), so that what is left is the shell's own control text, read
    file-wide (a quote may span lines); and, for each line, the state it STARTS in: "code", "sq"
    or "dq" (inside a quoted string begun on an earlier line), or "cap" or "arith" (inside a
    capture or an arithmetic expansion continued from an earlier line)."""
    out, stack, i, n, starts = [], ["code"], 0, len(text), ["code"]

    def kind():
        t = stack[-1]
        return t if isinstance(t, str) else t[0]

    def emit(ch, keep, state=None):
        out.append(ch if (keep or ch == "\n") else " ")
        if ch == "\n":
            starts.append(state or kind())

    while i < n:
        ch, top = text[i], stack[-1]
        if top == "dq":
            if ch == "\\":
                emit(ch, False); emit(text[i + 1] if i + 1 < n else "", False); i += 2; continue
            if ch == '"':
                stack.pop(); emit(ch, False); i += 1; continue
            if text.startswith("$((", i):
                stack.append(["arith", 0]); out.append("   "); i += 3; continue
            if text.startswith("$(", i):
                stack.append(["cap", 0]); emit(ch, False); emit("(", False); i += 2; continue
            emit(ch, False); i += 1; continue
        if isinstance(top, list) and top[0] == "arith":
            if ch == "(":
                top[1] += 1
            elif ch == ")":
                if top[1] == 0 and i + 1 < n and text[i + 1] == ")":
                    stack.pop(); out.append("  "); i += 2; continue
                top[1] -= 1
            emit(ch, False); i += 1; continue
        # code, or code inside a capture
        keep = top == "code"
        if ch == "\\":
            emit(ch, keep); emit(text[i + 1] if i + 1 < n else "", keep); i += 2; continue
        if ch == "'":
            j = text.find("'", i + 1)
            j = n - 1 if j < 0 else j
            for c in text[i:j + 1]:
                emit(c, False, "sq")
            i = j + 1; continue
        if ch == '"':
            stack.append("dq"); emit(ch, False); i += 1; continue
        if ch == "#" and (i == 0 or text[i - 1] in " \t;\n"):
            j = text.find("\n", i)
            j = n if j < 0 else j
            for c in text[i:j]:
                emit(c, False)
            i = j; continue
        if text.startswith("$((", i):
            stack.append(["arith", 0]); out.append("   "); i += 3; continue
        if text.startswith("$(", i):
            stack.append(["cap", 0]); emit(ch, False); emit("(", False); i += 2; continue
        if isinstance(top, list) and top[0] == "cap":
            if ch == "(":
                top[1] += 1
            elif ch == ")":
                if top[1] == 0:
                    stack.pop(); emit(ch, False); i += 1; continue
                top[1] -= 1
        emit(ch, keep); i += 1
    return "".join(out), starts


PIPE = re.compile(r"(?<!\|)\|(?!\|)")


def strip_comment(line, q=None):   # q: the quote the line starts inside (r10), or None
    out = []
    for i, ch in enumerate(line):
        if q:
            if ch == q:
                q = None
        elif ch in "'\"":
            q = ch
        elif ch == "#" and (i == 0 or line[i - 1] in " \t;"):
            break
        out.append(ch)
    return "".join(out)


def captures_in(text):
    """Every $( … ) anywhere in text (nested ones too, and inside arithmetic): their bodies."""
    out, i, n = [], 0, len(text)
    while i < n:
        if text.startswith("$((", i):
            i += 3
            continue
        if text.startswith("$(", i):
            depth, j, q2 = 1, i + 2, None
            while j < n and depth:
                c = text[j]
                if q2:
                    if c == q2:
                        q2 = None
                elif c in "'\"":
                    q2 = c
                elif c == "(":
                    depth += 1
                elif c == ")":
                    depth -= 1
                j += 1
            body = text[i + 2:j - 1]
            out.append(body.strip())
            out.extend(captures_in(body))
            i = j
            continue
        i += 1
    return out


def scan_line(code, dq=False):
    """For each OUTERMOST $( … ) in one line: (start, body, in_double_quotes, command_start, in_arith).
    r10: a capture INSIDE an arithmetic expansion is reported too (in_arith), never skipped; dq is
    True for a line that starts inside a double-quoted string begun on an earlier line."""
    res, i, sq, cmd_start, n = [], 0, False, 0, len(code)
    while i < n:
        ch = code[i]
        if sq:
            if ch == "'":
                sq = False
            i += 1
            continue
        if ch == "\\":
            i += 2
            continue
        if ch == "'" and not dq:
            sq = True
        elif ch == '"':
            dq = not dq
        elif code.startswith("$((", i):
            depth, j = 2, i + 3          # arithmetic expansion: to its matching ))
            while j < n and depth:
                depth += code[j] == "("
                depth -= code[j] == ")"
                j += 1
            for body in captures_in(code[i + 3:j - 2]):   # r10: every capture in it decides a number
                res.append((i, body, dq, cmd_start, True))
            i = j
            continue
        elif code.startswith("$(", i):
            depth, j, q2 = 1, i + 2, None
            while j < n and depth:
                c = code[j]
                if q2:
                    if c == q2:
                        q2 = None
                elif c in "'\"":
                    q2 = c
                elif c == "(":
                    depth += 1
                elif c == ")":
                    depth -= 1
                j += 1
            res.append((i, code[i + 2:j - 1].strip(), dq, cmd_start, False))
            i = j
            continue
        elif not dq and ch in ";|&{(":
            cmd_start = i + 1
        elif not dq and (i == 0 or code[i - 1] in " \t;") and re.match(r"(then|do|else)\s", code[i:]):
            cmd_start = i + code[i:].index(" ") + 1
        i += 1
    return res


def first_word(body):
    m = re.match(r"[^\s;|&]+", body)
    return m.group(0) if m else ""


def skip_expansion(s, i):
    """s[i] is the '$' of $( , $(( or ${: the index just past its balanced end."""
    n = len(s)
    if s.startswith("$((", i):
        depth, j = 2, i + 3
        while j < n and depth:
            depth += s[j] == "("
            depth -= s[j] == ")"
            j += 1
        return j
    op, cl = ("(", ")") if s.startswith("$(", i) else ("{", "}")
    depth, j, q = 1, i + 2, None
    while j < n and depth:
        c = s[j]
        if c == "\\" and q != "'":
            j += 2
            continue
        if q:
            if c == q:
                q = None
        elif c in "'\"":
            q = c
        elif c == op:
            depth += 1
        elif c == cl:
            depth -= 1
        j += 1
    return j


def simple_command(line, i):
    """r12: the words of the simple command that starts at line[i], as (word, start, end), to the first unquoted
    ; & | newline, unbalanced ) or closing-brace word. Quotes and expansions are kept inside their word, and a
    redirection stays part of its word, so that no shape can mistake it for an argument."""
    toks, n, j = [], len(line), i
    while j < n:
        while j < n and line[j] in " \t":
            j += 1
        if j >= n or line[j] in ";&|\n)" or (line[j] == "}" and (j + 1 == n or line[j + 1] in " \t;")):
            break
        k, q = j, None
        while k < n:
            c = line[k]
            if q == "'":
                q = None if c == "'" else q
                k += 1
                continue
            if c == "\\":
                k += 2
                continue
            if c == "$" and k + 1 < n and line[k + 1] in "({":
                k = skip_expansion(line, k)
                continue
            if q == '"':
                q = None if c == '"' else q
                k += 1
                continue
            if c in "'\"":
                q = c
                k += 1
                continue
            if c in " \t;&|\n)":
                break
            k += 1
        toks.append((line[j:k], j, k))
        j = k
    return toks


def commands_of(body):
    """r13: the command word of each simple command of body (split at ; & | and newlines outside quotes)."""
    names, i, n = [], 0, len(body)
    while i < n:
        toks = simple_command(body, i)
        if not toks:
            i += 1
            continue
        names.append(toks[0][0])
        i = toks[-1][2]
    return names


def one_dq(word):
    """word is exactly one double-quoted string."""
    if len(word) < 2 or word[0] != '"':
        return False
    i, n = 1, len(word)
    while i < n:
        c = word[i]
        if c == "\\":
            i += 2
            continue
        if c == "$" and i + 1 < n and word[i + 1] in "({":
            i = skip_expansion(word, i)
            continue
        if c == '"':
            return i == n - 1
        i += 1
    return False


def text_shape(line, cmd_start, start):
    """r12: the exact command shapes a TEXT capture may stand in, by whole words:
       echo "TEXT" [>> TARGET]                    TARGET one double-quoted string or a plain word, no capture
       verdict NAME $? "TEXT" ["WHY"]             NAME "$var" or a plain word; WHY one string, no capture
       VAR="$VAR<literal>…"                       VAR one of bad fu cl hold, alone, growing by literal text
    The capture must lie inside TEXT (or the appended string). Anything else: None."""
    toks = simple_command(line, cmd_start)
    if not toks:
        return None

    def holds(t):
        return t[1] < start < t[2]

    def plain(w):
        return re.fullmatch(r"[A-Za-z0-9_./-]+", w) is not None

    w = [t[0] for t in toks]
    if w[0] == "echo" and len(w) in (2, 4) and one_dq(w[1]) and holds(toks[1]):
        if len(w) == 2 or (w[2] == ">>" and ((one_dq(w[3]) and "$(" not in w[3]) or plain(w[3]))):
            return "echo"
    if (w[0] == "verdict" and len(w) in (4, 5) and w[2] == "$?" and one_dq(w[3]) and holds(toks[3])
            and (re.fullmatch(r'"\$[A-Za-z_]\w*"', w[1]) or plain(w[1]))
            and (len(w) == 4 or (one_dq(w[4]) and "$(" not in w[4]))):
        return "verdict"
    m = re.fullmatch(r'(bad|fu|cl|hold)="\$([A-Za-z_]\w*)(.*)', w[0], re.S)
    if (len(w) == 1 and m and m.group(1) == m.group(2) and one_dq(w[0][len(m.group(1)) + 1:])
            and holds(toks[0]) and m.group(3)[:1] not in ("", "$", '"', "\\", "`")):
        return "append"
    return None


def has_backtick(code, dq=False):
    """r12: a backtick outside single quotes (in code or in a double-quoted string): a command substitution
    this judge does not model."""
    sq, i, n = False, 0, len(code)
    while i < n:
        c = code[i]
        if sq:
            sq = c != "'"
        elif c == "\\":
            i += 2
            continue
        elif c == "'" and not dq:
            sq = True
        elif c == '"':
            dq = not dq
        elif c == "`":
            return True
        i += 1
    return False


def control_class(rel, line, body, dq, cmd_start, arith=False, start=None):
    if arith:
        return "DECISION"   # r10 (mike: `took=$(( $(up_ms) - t0 ))`): a capture inside arithmetic
    if rel == "controls/controls.sh" and re.match(r'^"\$@"; printf \'\\nwcap-rc=%s\' "\$\?"$', body):
        return "WITNESSED"
    if rel == "controls/observer.sh" and re.match(r"^obs_listing; printf 'obs-rc=%s\\n' \"\$\?\"$", body):
        return "WITNESSED"
    if rel == "controls/observer.sh" and body.startswith('"$OBS_PS" -e -o pgid=,pid=,stat='):
        return "ANCHORED"
    if dq:
        # r12: DEFAULT-DENY: an allowlisted body in an allowed shape is TEXT; any other quoted capture is
        # UNCLASSIFIED (a new TEXT capture is a reviewed addition to TEXT_BODIES)
        if body in TEXT_BODIES and start is not None and text_shape(line, cmd_start, start):
            return "TEXT"
        return "UNCLASSIFIED"
    return "DECISION"


def shell_files(root):
    for d, dirs, files in os.walk(root):
        dirs[:] = sorted(x for x in dirs if x not in SKIP)
        for f in sorted(files):
            p = os.path.join(d, f)
            if not os.path.isfile(p) or os.path.islink(p):
                continue
            rel = os.path.relpath(p, root)
            if f.endswith(".sh"):
                yield rel
                continue
            try:
                with open(p, errors="replace") as fh:
                    first = fh.readline()
            except OSError:
                continue
            if re.match(r"#!.*\b(ba|da|k|z)?sh\b", first):
                yield rel


def main():
    root = sys.argv[1]
    listing = "--list" in sys.argv
    counts, problems, rows, pinned_seen = {}, [], [], set()
    here_seen = {}
    ts_defs = verdict_defs = 0
    files = list(shell_files(root))
    for rel in files:
        text = open(os.path.join(root, rel), errors="replace").read()
        sk_text, starts = skeleton(text)
        control = rel != "supervise.sh" and not rel.startswith("controls/fake-podman/")
        if control:
            raws = text.split("\n")
            for n, sk in enumerate(sk_text.split("\n"), 1):
                if starts[n - 1] in ("cap", "arith"):   # r10: not modelled, so not allowed
                    counts["CONTINUED"] = counts.get("CONTINUED", 0) + 1
                    rows.append(f"{rel}:{n} CONTINUED {' '.join(raws[n - 1].split())[:100]}")
                    problems.append(f"{rel}:{n} a capture or arithmetic expansion continued across lines in a control: {' '.join(raws[n - 1].split())[:100]}")
                raw = raws[n - 1] if n <= len(raws) else ""
                # r12: the trusted command names (TRUSTED) are never redefined: a function definition, by
                # either form (`NAME()` or `function NAME`, with or without parentheses), or any alias,
                # unalias or enable, found in the code text (quotes, captures and comments blanked)
                defs = [m.group(1) for m in re.finditer(r"\bfunction\s+([A-Za-z_][\w-]*)", sk)]
                defs += [m.group(1) for m in re.finditer(r"(?<![\w$-])([A-Za-z_][\w-]*)\s*\(\s*\)", sk)]
                for name in defs:
                    if name not in TRUSTED:
                        continue
                    if name == "ts" and strip_comment(raw).strip() == TS_DEF:
                        ts_defs += 1
                        continue
                    if name == "verdict" and rel == "controls/controls.sh" and re.match(r"verdict\(\) \{", raw):
                        verdict_defs += 1
                        continue
                    counts["REDEFINED"] = counts.get("REDEFINED", 0) + 1
                    rows.append(f"{rel}:{n} REDEFINED {' '.join(raw.split())[:100]}")
                    problems.append(f"{rel}:{n} a trusted command name redefined: {' '.join(raw.split())[:100]}")
                for m in re.finditer(r"(?:^|[;&|({]|\b(?:then|do|else))\s*(alias|unalias|enable)\b(?!\s*\))|\b(expand_aliases)\b", sk):
                    counts["REDEFINED"] = counts.get("REDEFINED", 0) + 1
                    rows.append(f"{rel}:{n} REDEFINED {' '.join(raw.split())[:100]}")
                    problems.append(f"{rel}:{n} command resolution changed ({m.group(1) or m.group(2)}): {' '.join(raw.split())[:100]}")
                pipes = [m for m in PIPE.finditer(sk)
                         if (sk[m.start() - 1] if m.start() else " ") in " \t"
                         or (sk[m.end()] if m.end() < len(sk) else " ") in " \t&"]
                if pipes:   # one row per line
                    counts["PIPE"] = counts.get("PIPE", 0) + 1
                    rows.append(f"{rel}:{n} PIPE {' '.join(raw.split())[:100]}")
                    problems.append(f"{rel}:{n} a pipeline in a control (a pipe that cannot be made decides): {' '.join(raw.split())[:100]}")
                if "<(" in sk or ">(" in sk:
                    counts["PROCSUB"] = counts.get("PROCSUB", 0) + 1
                    rows.append(f"{rel}:{n} PROCSUB {' '.join(raw.split())[:100]}")
                    problems.append(f"{rel}:{n} a process substitution in a control: {' '.join(raw.split())[:100]}")
                for _ in re.finditer("<<<", sk):
                    ok = [k for k, (f, rx, c) in enumerate(HERE_OK) if f == rel and re.search(rx, raw.rstrip())]
                    cls = "HERE" if ok else "UNPINNED-HERE"
                    if ok:
                        here_seen[ok[0]] = here_seen.get(ok[0], 0) + 1
                    else:
                        problems.append(f"{rel}:{n} an unpinned here-string in a control: {' '.join(raw.split())[:100]}")
                    counts[cls] = counts.get(cls, 0) + 1
                    rows.append(f"{rel}:{n} {cls} {' '.join(raw.split())[:100]}")
        for n, raw in enumerate(text.split("\n"), 1):
            st0 = starts[n - 1]
            if st0 == "sq":   # r10: the rest of a single-quoted string begun on an earlier line is not code
                k = raw.find("'")
                raw = "" if k < 0 else " " * (k + 1) + raw[k + 1:]
                st0 = "code"
            code = strip_comment(raw, '"' if st0 == "dq" else None)
            # r12: what this judge does not model is refused by default, not left as a limit
            if control and (has_backtick(code, st0 == "dq")
                            or re.search(r"(?:^|[;&|({]|\b(?:then|do|else))\s*eval\b", sk_text.split("\n")[n - 1])):
                counts["UNMODELLED"] = counts.get("UNMODELLED", 0) + 1
                rows.append(f"{rel}:{n} UNMODELLED {' '.join(raw.split())[:100]}")
                problems.append(f"{rel}:{n} a backtick substitution or eval in a control (not modelled, so refused): {' '.join(raw.split())[:100]}")
            for start, body, dq, cs, arith in scan_line(code, st0 == "dq"):
                short = " ".join(body.split())
                if rel == "supervise.sh":
                    cls = "UNPINNED"
                    for i, (c, rx) in enumerate(PRODUCTION):
                        if re.search(rx, short):
                            cls = c
                            pinned_seen.add(i)
                            break
                elif rel.startswith("controls/fake-podman/"):
                    cls = "FAKE"
                else:
                    cls = control_class(rel, code, body, dq, cs, arith, start)
                counts[cls] = counts.get(cls, 0) + 1
                rows.append(f"{rel}:{n} {cls} {short[:100]}")
                if cls in ("DECISION", "UNPINNED", "UNCLASSIFIED"):
                    problems.append(f"{rel}:{n} a capture in a {cls} position: $({short[:100]})")
            if re.search(r'"\$(st|at_exit)" = present \]', code) and ("remove_exact" in code or "then" in code):
                problems.append(f"{rel}:{n} a rescue gated on '= present' (an empty state token skips it)")
    if "controls/controls.sh" in files and (ts_defs, verdict_defs) != (1, 1):
        problems.append(f"controls/controls.sh: ts defined {ts_defs} times at its pinned body and verdict {verdict_defs} times, want 1 and 1")
    for k, (f, rx, c) in enumerate(HERE_OK):
        if f in files and here_seen.get(k, 0) != c:
            problems.append(f"{f}: the pinned here-string /{rx}/ was seen {here_seen.get(k, 0)} times, want {c}")
    if "supervise.sh" in files:
        for i, (c, rx) in enumerate(PRODUCTION):
            if i not in pinned_seen:
                problems.append(f"supervise.sh: the pinned {c} capture /{rx}/ was not found")
    # r13: the allowlist's own commands, and the pinned ts's callees, are all protected (one hop)
    used = {c for b in TEXT_BODIES for c in commands_of(b)}
    used |= set(commands_of(TS_DEF[TS_DEF.index("{") + 1:TS_DEF.rindex("}")]))
    for c in sorted(used - TRUSTED):
        problems.append(f"the judge itself: the allowlisted command {c} is not TRUSTED (it could be redefined unseen)")
    word = "PASS" if not problems else "FAIL"
    other = ("PIPE", "PROCSUB", "HERE", "UNPINNED-HERE", "CONTINUED", "REDEFINED", "UNMODELLED")
    caps = {k: v for k, v in counts.items() if k not in other}
    rest = {k: counts.get(k, 0) for k in other}
    print(f"{word}: {len(files)} shell files ({', '.join(files)}); {sum(caps.values())} captures: "
          + ", ".join(f"{k} {v}" for k, v in sorted(caps.items()))
          + "; in the controls: " + ", ".join(f"{k} {v}" for k, v in rest.items()))
    if listing:
        print("\n".join(rows))
    if problems:
        print("mismatches: " + "; ".join(problems))
    return 0 if not problems else 1


if __name__ == "__main__":
    sys.exit(main())
