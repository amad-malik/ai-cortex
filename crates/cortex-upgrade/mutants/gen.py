#!/usr/bin/env python3 -I
"""Candidate mutants for a region of a source file (written by hue for the R-5 rework of packet 17).

    python3 -I mutants/gen.py <file> <start-end>[,<start-end>...] <id-prefix> <tests-csv> [--first N]   > candidates.json

For every line in the given line ranges it proposes the single-fault edits that apply to it: `==` and `!=` swapped, `&&` and `||` swapped, a leading `!` of a
condition dropped, `<` `<=` `>` `>=` swapped, an `if <cond> {` made `if false {` and `if true {`, a standalone statement that ends in `?;` or a call of `sync_dir`,
`persist` or `remove_if_there` deleted, a `State::X` replaced by another state, a mode constant changed, a `.is_ok()` flipped. The `old` text of each candidate is
the line with as many lines of context before it as it takes to occur exactly ONCE in the file (the runner refuses an anchor that is not unique). Candidates
that do not compile, or that no test catches, show in the run; the person reading decides whether a survivor is equivalent or a gap.
"""
import json
import re
import sys

path, ranges, prefix, tests = sys.argv[1], sys.argv[2], sys.argv[3], sys.argv[4].split(",")
src = open(path).read()
lines = src.split("\n")
spans = [tuple(int(x) for x in r.split("-")) for r in ranges.split(",")]


def unique_anchor(i, new_line):
    """The smallest context (lines i-k..i) whose text occurs once in the file; returns (old, new) or None."""
    for k in range(0, 6):
        lo = max(0, i - k)
        old = "\n".join(lines[lo : i + 1])
        if src.count(old) == 1:
            new = "\n".join(lines[lo:i] + [new_line])
            return old, new
    return None


def candidates(i, line):
    s = line.strip()
    if not s or s.startswith("//") or s.startswith("///") or s.startswith("#["):
        return
    out = []
    ind = line[: len(line) - len(line.lstrip())]
    for a, b, what in (("==", "!=", "an equality is inverted"), ("!=", "==", "an inequality is inverted"), ("&&", "||", "an && becomes ||"), ("||", "&&", "an || becomes &&")):
        if a in line and not (a == "==" and "=>" in line) and "fn " not in line.split(a)[0]:
            for m in re.finditer(re.escape(a), line):
                j = m.start()
                if a == "!=" and line[j : j + 3] == "!==":
                    continue
                out.append((line[:j] + b + line[j + len(a) :], what + f" (at column {j})"))
                break
    m = re.search(r"\bif (!?)(.+) \{$", line)
    if m and "let " not in line:
        out.append((f"{ind}if false {{", "a condition is always false"))
        out.append((f"{ind}if true {{", "a condition is always true"))
        if m.group(1) == "!":
            out.append((line.replace("if !", "if ", 1), "a negation is dropped"))
    elif re.search(r"\b(&&|\|\|) !", line):
        out.append((re.sub(r"(&&|\|\|) !", r"\1 ", line, count=1), "a negation is dropped"))
    for a, b in ((">=", ">"), ("<=", "<"), (">", ">="), ("<", "<=")):
        if re.search(rf"[\w)\]] {re.escape(a)} [\w(&]", line) and "=>" not in line and "->" not in line:
            out.append((re.sub(rf"([\w)\]]) {re.escape(a)} ", rf"\1 {b} ", line, count=1), f"a comparison {a} becomes {b}"))
            break
    if re.fullmatch(r"\s*[\w:.&()\[\]\", ]+\)\?;", line) or re.match(r"\s*(sync_dir|persist|remove_if_there)\(.*\)\??;\s*$", line):
        if "let " not in line and "return" not in line:
            out.append((f"{ind}", "a statement is deleted"))
    if "State::" in line and "=>" not in line and "fn " not in line:
        for st in ("Planned", "BackedUp", "Replaced", "Restored"):
            if f"State::{st}" in line:
                other = {"Planned": "BackedUp", "BackedUp": "Replaced", "Replaced": "Restored", "Restored": "Planned"}[st]
                out.append((line.replace(f"State::{st}", f"State::{other}", 1), f"State::{st} becomes State::{other}"))
                break
    for const in ("0o600", "0o755", "0o644"):
        if const in line and "assert" not in line:
            out.append((line.replace(const, "0o666" if const != "0o600" else "0o644", 1), f"the mode {const} is changed"))
            break
    if ".is_ok()" in line:
        out.append((line.replace(".is_ok()", ".is_err()", 1), "is_ok becomes is_err"))
    if ".is_empty()" in line and "assert" not in line:
        out.append((line.replace(".is_empty()", ".is_some()" if False else ".is_empty() && false", 1), "an emptiness check never holds"))
    for new, what in out:
        yield new, what


res = []
n = 0
for (a, b) in spans:
    for i in range(a - 1, b):
        for new_line, what in candidates(i, lines[i]):
            anchored = unique_anchor(i, new_line)
            if anchored is None or anchored[0] == anchored[1]:
                continue
            old, new = anchored
            n += 1
            res.append({"id": f"{prefix}{n:03d}", "design": None, "file": path, "tests": tests, "fails": ["__collect__"], "what": f"line {i + 1}: {what}", "old": old, "new": new})
json.dump(res, sys.stdout, indent=1)
print(f"{len(res)} candidates", file=sys.stderr)
