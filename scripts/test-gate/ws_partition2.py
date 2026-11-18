#!/usr/bin/env python3
"""cox's partition check for a workspace GREEN receipt (delta 4, row 366c2480), version 2 (bob, for cox's row 5c9229e1).

CHANGES against ws_partition.py (sha256 78431c32eab5f91d, mira's tool, unchanged elsewhere):
  1. Every line of every input file is stripped of ANSI escape sequences BEFORE any match, so coloured and plain text of the same run
     give identical results (the old patterns silently ignored a line whose final `ok` was wrapped in colour codes).
  2. A trybuild fixture line (`test tests/ui/<name>.rs [should pass] ... ok`, or `[should fail to compile]`) is a CHILD of its parent
     test: it is not an inventory test and not a partition. `check` prints a `harness children` section naming each ok child; a child
     that is not `ok` (FAILED, or ignored) FAILS the gate ("difference - harness child not ok").
  3. A real test that ran and is not in the inventory still FAILS, in plain and in coloured text.

  ws_partition.py derive  BUILD_JSON RUN_TXT            -> DB-only TSV on stdout (tests that failed
                                                            only because CV2_PG_PORT is unset)
  ws_partition.py skips   DBONLY_TSV                    -> one `--skip NAME` per DB-only test name
  ws_partition.py check   BUILD_JSON LIST_TXT RUN_TXT DBONLY_TSV IGNORED_TSV
                                                        -> prints the table; exit 1 if the partition
                                                           inventory = DB-free green + DB-only +
                                                           declared-ignored has ANY difference, or
                                                           anything failed

BUILD_JSON  cargo test --workspace --no-run --message-format=json   (package + target per binary)
LIST_TXT    cargo test --workspace -- --list                         (every test, none run)
RUN_TXT     cargo test --workspace --no-fail-fast -- --exact --skip ... (the DB-free run)
DBONLY_TSV  package<TAB>target<TAB>test   (declared: needs a database; NOT run by this receipt)
IGNORED_TSV package<TAB>target<TAB>test<TAB>reason   (declared #[ignore]d: run by its parent test)
"""
import json
import os
import re
import sys


def load_map(build_json):
    m = {}
    for line in open(build_json):
        try:
            j = json.loads(line)
        except ValueError:
            continue
        if (j.get("reason") != "compiler-artifact" or not j.get("profile", {}).get("test")
                or not j.get("executable")):
            continue
        pid = j["package_id"]
        frag = pid.split("#")[-1]
        pkg = frag.split("@")[0] if "@" in frag else pid.split("#")[0].rstrip("/").split("/")[-1]
        t = j["target"]
        m[os.path.basename(j["executable"])] = (pkg, "+".join(t["kind"]), t["name"])
    return m


RUNNING = re.compile(r"^\s+Running .*\(target/debug/deps/([^)]+)\)\s*$")
DOCTESTS = re.compile(r"^\s+Doc-tests (\S+)\s*$")
LISTED = re.compile(r"^(.+): test$")
RAN = re.compile(r"^test (.+) \.\.\. (ok|FAILED|ignored.*)$")
# trybuild prints one line per fixture while its parent test runs: `test tests/ui/<name>.rs [should pass] ... ok`
CHILD = re.compile(r"^tests/ui/\S+\.rs(?: \[[^\]]+\])?$")
# CSI sequences (colours, cursor, erase), OSC sequences (titles, hyperlinks) ended by BEL or ST, and two-character escapes
ANSI = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[@-Z\\-_]")


def strip_ansi(s):
    return ANSI.sub("", s)
BLOCK = re.compile(r"^---- (.+) stdout ----$")


def sections(path, m):
    """Yield (key, line) with key = (package, kind, target) for the section the line is in."""
    cur = None
    for line in open(path, errors="replace"):
        line = strip_ansi(line.rstrip("\n"))
        r = RUNNING.match(line)
        if r:
            cur = m.get(r.group(1), ("?", "?", r.group(1)))
            continue
        d = DOCTESTS.match(line)
        if d:
            cur = ("(doc)", "doctest", d.group(1))
            continue
        yield cur, line


def tests_listed(path, m):
    out = set()
    for key, line in sections(path, m):
        t = LISTED.match(line)
        if key and t:
            out.add((key[0], key[2], t.group(1)))
    return out


def tests_ran(path, m):
    """The tests that ran, harness children (trybuild fixtures) excluded."""
    out = {}
    for key, line in sections(path, m):
        t = RAN.match(line)
        if key and t and not CHILD.match(t.group(1)):
            out[(key[0], key[2], t.group(1))] = "ignored" if t.group(2).startswith("ignored") else t.group(2)
    return out


def children_ran(path, m):
    """The harness children (trybuild fixture lines) in the run, in file order: (package, target, name, status)."""
    out = []
    for key, line in sections(path, m):
        t = RAN.match(line)
        if key and t and CHILD.match(t.group(1)):
            out.append((key[0], key[2], t.group(1), "ignored" if t.group(2).startswith("ignored") else t.group(2)))
    return out


def derive(build_json, run_txt):
    m = load_map(build_json)
    rows = set()
    cur_test = None
    failed = tests_ran(run_txt, m)
    # failure blocks: the text after `---- name stdout ----` until the next block or `failures:`
    block, text = None, []
    def flush():
        if block and "CV2_PG_PORT is unset" in "\n".join(text):
            key = block[0]
            for (pkg, tgt, name), st in failed.items():
                if tgt == key[2] and name == block[1] and st == "FAILED":
                    rows.add((pkg, tgt, name))
    for key, line in sections(run_txt, m):
        b = BLOCK.match(line)
        if b:
            flush()
            block, text = (key, b.group(1)), []
        elif line.startswith("failures:"):
            flush()
            block, text = None, []
        elif block:
            text.append(line)
    flush()
    for r in sorted(rows):
        print("\t".join(r))


def load_dbonly(path):
    return {tuple(l.rstrip("\n").split("\t")) for l in open(path) if l.strip() and not l.startswith("#")}


def skips(dbonly_tsv):
    for name in sorted({t[2] for t in load_dbonly(dbonly_tsv)}):
        print("--skip")
        print(name)


def load_ignored(path):
    out = {}
    for l in open(path):
        if l.strip() and not l.startswith("#"):
            f = l.rstrip("\n").split("\t")
            out[tuple(f[:3])] = f[3] if len(f) > 3 else ""
    return out


def check(build_json, list_txt, run_txt, dbonly_tsv, ignored_tsv):
    m = load_map(build_json)
    inv = tests_listed(list_txt, m)
    ran = tests_ran(run_txt, m)
    kids = children_ran(run_txt, m)
    dbonly = load_dbonly(dbonly_tsv)
    declared_ignored = load_ignored(ignored_tsv)
    green = {t for t, s in ran.items() if s == "ok"}
    failed = sorted(t for t, s in ran.items() if s == "FAILED")
    ignored_ran = {t for t, s in ran.items() if s == "ignored"}
    undeclared_ignored = sorted(ignored_ran - set(declared_ignored))
    unaccounted = sorted(inv - green - dbonly - set(declared_ignored))
    ghost = sorted((green | set(failed) | ignored_ran) - inv)
    overlap = sorted(green & dbonly)
    stale = sorted((dbonly | set(declared_ignored)) - inv)
    ignored = undeclared_ignored
    print("PARTITION CHECK (inventory = DB-free green + DB-only)")
    print(f"  inventory (listed, none run) : {len(inv)}")
    print(f"  DB-free, ran and passed      : {len(green)}")
    print(f"  DB-only (declared, NOT run)  : {len(dbonly)}")
    print(f"  ran and FAILED               : {len(failed)}")
    print(f"  declared #[ignore] (parent runs it): {len(declared_ignored)}")
    for t, why in sorted(declared_ignored.items()):
        print("      ", " :: ".join(t), "-", why)
    print(f"  sum green + DB-only + declared-ignored : {len(green | dbonly | set(declared_ignored))}")
    kids_bad = sorted(k for k in kids if k[3] != "ok")
    diffs = {"inventory but neither green nor DB-only": unaccounted,
             "ran but not in the inventory": ghost,
             "green AND declared DB-only": overlap,
             "declared DB-only but not in the inventory": stale,
             "FAILED": failed, "ignored but not declared": ignored,
             "harness child not ok": kids_bad}
    bad = False
    for name, items in diffs.items():
        print(f"  difference - {name}: {len(items)}")
        for t in items:
            print("      ", " :: ".join(t))
        bad = bad or bool(items)
    per = {}
    for p, tg, _ in inv:
        per.setdefault((p, tg), [0, 0, 0])[0] += 1
    for p, tg, n in green:
        per.setdefault((p, tg), [0, 0, 0])[1] += 1
    for p, tg, n in dbonly:
        per.setdefault((p, tg), [0, 0, 0])[2] += 1
    print()
    print("  %-18s %-34s %6s %8s %8s" % ("package", "target", "listed", "DB-free", "DB-only"))
    for (p, tg), (a, g, d) in sorted(per.items()):
        print("  %-18s %-34s %6d %8d %8d" % (p, tg, a, g, d))
    kids_ok = sorted({(p, tg, n) for p, tg, n, s in kids if s == "ok"})
    print()
    print(f"  harness children (trybuild fixtures a parent test prints; not in the inventory, not partitions): {len(kids_ok)} ok")
    for t in kids_ok:
        print("      ", " :: ".join(t))
    print("\n  RESULT:", "PARTITION EMPTY, NOTHING FAILED" if not bad else "NOT EMPTY or FAILED (see above)")
    return 1 if bad else 0


if __name__ == "__main__":
    cmd = sys.argv[1]
    if cmd == "derive":
        derive(sys.argv[2], sys.argv[3])
    elif cmd == "skips":
        skips(sys.argv[2])
    elif cmd == "check":
        sys.exit(check(*sys.argv[2:7]))
    else:
        sys.exit(__doc__)
