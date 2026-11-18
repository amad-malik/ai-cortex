#!/usr/bin/env python3
"""The documentation renderer for Cortex v2 (piece 13; cox WAVE1-RULINGS W1-28 and W1-29).

ONE renderer, Python 3 standard library only, no network, deterministic: the same contract gives
the same bytes. The contract is the committed copy of the pinned S3 JSON twin in tools/contract/.
Run it through tools/build-docs.sh, which adds the checksum, the CLI capture and the staleness
check.

Subcommands (each prints to stdout; a refusal is a non-zero exit with one line per problem on
stderr):
  check CONTRACT              the contract checks: every operation names its errors (x-errors,
                              an explicit [] allowed) and has an error response; its auth
                              (security, an explicit [] allowed); and its CLI (x-cli, '-' = none)
  render CONTRACT             prints the API reference page to stdout (tools/build-docs.sh --write
                              saves it as docs/generated/API-REFERENCE.md)
  cli-list CONTRACT           the cortex subcommands the contract names, one per line
  capture CONTRACT CORTEX_BIN the CLI reference page (docs/generated/CLI-REFERENCE.md), from
                              `cortex <sub> --help`; a subcommand the binary does not answer is a
                              refusal, and so is no binary (tools/build-docs.sh decides to SKIP the
                              capture when CORTEX_BIN is unset, and does not call this then)
  inventory CONTRACT FILE     the route inventory diffed both ways with FILE ("METHOD PATH" lines;
                              the hook for sam's SLOT-03: mounted inventory equals S3)

The page also renders three contract sections beside the operations: the route rules (x-rules, in
R-number order), the label grammars (x-label-grammar, with the fields each is checked against) and
the notes (x-notes, in contract order). INTERNAL_NOTES lists the x-notes keys that are internal to
the team and are SKIPPED on the page: "borrowed-code" (the D94 licensing note; cox, 6 Nov 18:56Z).
Add a key there to keep another internal note off the client page.
"""

import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path

# Every HTTP verb OpenAPI 3.1 allows on a path item, so none is skipped (hale F-4).
METHODS = ("get", "put", "post", "delete", "options", "head", "patch", "trace")
# The other fields a path item may have; anything else is refused by `check` (fail closed).
PATH_ITEM_FIELDS = ("summary", "description", "servers", "parameters", "$ref")
# x-notes keys that are internal to the team and are not rendered (see the module docstring).
INTERNAL_NOTES = ("borrowed-code",)
# The capture's environment: fixed, so the same binary prints the same bytes.
CAPTURE_ENV = {"LC_ALL": "C", "NO_COLOR": "1", "COLUMNS": "100", "TERM": "dumb"}


def load(path):
    return json.loads(Path(path).read_text(encoding="utf-8"))


def operations(c):
    """(path, method, operation) in the contract's own order."""
    for path, item in c.get("paths", {}).items():
        for method, o in item.items():
            if method in METHODS and isinstance(o, dict):
                yield path, method, o


def _name(path, method, o):
    return o.get("operationId", f"{method.upper()} {path}")


def _is_error_status(code):
    return code == "default" or (len(code) == 3 and code[0] in "45")


# ---- the contract checks (S3-DOC-PACK-SPEC section 5, items 2 and 3; x-cli for item 4) ----


def check(c):
    """Every problem, one line each; empty means the contract passes."""
    problems = []
    ops = list(operations(c))
    if not ops:
        problems.append("the contract has no operations")
    for path, item in c.get("paths", {}).items():
        for key in item:
            if key not in METHODS and key not in PATH_ITEM_FIELDS:
                problems.append(f"{path}: unknown path-item key {key!r} (not a verb or an OpenAPI field)")
    for path, method, o in ops:
        name = _name(path, method, o)
        if "x-errors" not in o:
            problems.append(f"{name}: no x-errors entry (write [] when the route has no code of its own)")
        if not any(_is_error_status(k) for k in o.get("responses", {})):
            problems.append(f"{name}: no error response (4xx, 5xx or default)")
        if "security" not in o:
            problems.append(f"{name}: no security requirement (write [] for a route that needs no token)")
        if "x-cli" not in o:
            problems.append(f"{name}: no x-cli entry (write - for a route with no CLI command)")
    return problems


# ---- the CLI (item 4) ----


def parse_cli(s):
    """The cortex subcommands an x-cli entry names: words up to the first placeholder or option,
    alternatives 'a|b' within those words expanded, ' | ' separating commands, and a trailing
    ' (...)' comment dropped. '-' names none."""
    s = (s or "").strip()
    if s in ("", "-"):
        return []
    out = []
    for alt in s.split(" | "):
        # A trailing " (...)" comment ends its own command only (hale N-1).
        cut = alt.find(" (")
        if cut >= 0:
            alt = alt[:cut]
        words = alt.split()
        if not words or words[0] != "cortex":
            continue
        sub = []
        for w in words[1:]:
            if w[0] in "<[-.":
                break
            sub.append(w)
        combos = [[]]
        for w in sub:
            combos = [c + [a] for c in combos for a in w.split("|") if a]
        out.extend(" ".join(c) for c in combos if c)
    return out


def cli_commands(c):
    """Every subcommand the contract names, once, in first-seen order."""
    seen = []
    for _, _, o in operations(c):
        for sub in parse_cli(o.get("x-cli", "")):
            if sub not in seen:
                seen.append(sub)
    return seen


def _help(cortex_bin, words):
    env = {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), **CAPTURE_ENV}
    try:
        r = subprocess.run([cortex_bin, *words, "--help"], capture_output=True, text=True,
                           timeout=30, env=env, check=False)
    except (OSError, subprocess.TimeoutExpired) as e:
        return None, f"cannot run: {type(e).__name__}"
    if r.returncode != 0:
        return None, f"exit {r.returncode}"
    if not r.stdout.strip():
        return None, "empty answer"
    return r.stdout, None


def capture(c, cortex_bin):
    """(page text, problems). No binary is a problem here; the skip is build-docs.sh's decision."""
    if not cortex_bin:
        return "", ["CORTEX_BIN is not set: the CLI capture (check 4) cannot pass without the cortex binary"]
    p = Path(cortex_bin)
    if not (p.is_file() and os.access(p, os.X_OK)):
        return "", [f"CORTEX_BIN is not an executable file: {cortex_bin}"]
    problems = []
    parts = [
        "# Cortex CLI reference (generated)",
        "",
        "Captured from `cortex <command> --help` for every command the contract names in `x-cli`. "
        "Do not edit: this page is a build product; run `tools/build-docs.sh --write`.",
        "",
    ]
    for words in [[]] + [sub.split() for sub in cli_commands(c)]:
        title = " ".join(["cortex", *words])
        text, why = _help(cortex_bin, words)
        if text is None:
            problems.append(f"{title}: missing from the capture ({why})")
            continue
        parts += [f"## {title}", "", "```text", text.rstrip("\n"), "```", ""]
    return "\n".join(parts), problems


# ---- the route inventory (item 1; sam's SLOT-03) ----


def inventory(c):
    return sorted(f"{m.upper()} {path}" for path, m, _ in operations(c))


def inventory_diff(c, lines):
    mounted = []
    problems = []
    for raw in lines:
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line in mounted:
            problems.append(f"{line}: listed twice in the mounted inventory")
        mounted.append(line)
    want = inventory(c)
    for x in sorted(set(mounted) - set(want)):
        problems.append(f"{x}: mounted but not in the contract")
    for x in sorted(set(want) - set(mounted)):
        problems.append(f"{x}: in the contract but not mounted")
    return problems


# ---- the page (S3-DOC-PACK-SPEC section 3) ----


def cell(v):
    """A Markdown table cell: one line, '|' escaped."""
    return " ".join(str(v).split()).replace("|", "\\|")


def _ref_name(ref):
    return ref.rsplit("/", 1)[-1]


def _resolve(c, obj):
    """An object or the component a local $ref names."""
    ref = obj.get("$ref") if isinstance(obj, dict) else None
    if not ref:
        return obj
    node = c
    for part in ref.lstrip("#/").split("/"):
        node = node.get(part, {}) if isinstance(node, dict) else {}
    return node


def _schema_text(s):
    if not isinstance(s, dict):
        return ""
    if "$ref" in s:
        return f"`{_ref_name(s['$ref'])}`"
    bits = []
    t = s.get("type")
    if isinstance(t, list):
        bits.append(" or ".join(t))
    elif t:
        bits.append(t)
    if s.get("format"):
        bits.append(s["format"])
    if "enum" in s:
        bits.append("one of " + ", ".join(f"`{e}`" for e in s["enum"]))
    if "minimum" in s and "maximum" in s:
        bits.append(f"{s['minimum']} to {s['maximum']}")
    elif "maximum" in s:
        bits.append(f"at most {s['maximum']}")
    elif "minimum" in s:
        bits.append(f"at least {s['minimum']}")
    if "pattern" in s:
        bits.append(f"pattern `{s['pattern']}`")
    if "x-max-bytes" in s:
        bits.append(f"at most {s['x-max-bytes']} bytes")
    if "default" in s:
        bits.append(f"default `{json.dumps(s['default'])}`")
    return ", ".join(bits)


def _first_sentence(text):
    text = " ".join(str(text).split())
    for stop in (". ", ": "):
        i = text.find(stop)
        if i > 0:
            return text[: i + 1].rstrip(":") if stop == ". " else text[:i]
    return text


def _content(c, body):
    out = []
    for ctype, media in (body or {}).get("content", {}).items():
        schema = _schema_text(_resolve(c, media).get("schema", {})) if isinstance(media, dict) else ""
        out.append(f"`{ctype}` {schema}".strip())
    return "; ".join(out)


def _operation(c, path, method, o):
    name = _name(path, method, o)
    lines = [f"### `{name}`: {method.upper()} `{path}`", ""]
    summary = o.get("summary", "")
    lines += [f"**Task:** {_first_sentence(summary)}", ""]
    if summary and _first_sentence(summary) != " ".join(summary.split()):
        lines += [f"> {' '.join(summary.split())}", ""]

    scope = o.get("x-scope", "")
    holders = c.get("x-scopes", {}).get(scope)
    who = f"scope `{scope}` (class {o.get('x-class', '?')})"
    if isinstance(holders, list) and len(holders) > 1:
        who += f": {holders[1]}"
    auth = "a worker token" if o.get("security") else "no token (security: [])"
    target = o.get("x-target")
    lines += [f"**Who may call:** {who}; auth: {auth}; target rule `{target}`."]
    if o.get("x-target-note"):
        lines += [f"Target note: {' '.join(str(o['x-target-note']).split())}"]
    lines += [""]

    lines += ["**Request:**", ""]
    params = [_resolve(c, p) for p in o.get("parameters", [])]
    if params:
        lines += ["| Name | In | Required | Schema | Description |", "|---|---|---|---|---|"]
        for p in params:
            lines.append(
                f"| `{cell(p.get('name', ''))}` | {cell(p.get('in', ''))} | "
                f"{'yes' if p.get('required') else 'no'} | {cell(_schema_text(p.get('schema', {})))} | "
                f"{cell(p.get('description') or p.get('schema', {}).get('description', ''))} |"
            )
        lines += [""]
    else:
        lines += ["No parameters.", ""]
    if o.get("requestBody"):
        body = _resolve(c, o["requestBody"])
        cap = f" (at most {o['x-max-body']} bytes)" if "x-max-body" in o else ""
        lines += [f"Body{cap}: {_content(c, body)}", ""]

    lines += ["**Response:**", ""]
    for code, r in o.get("responses", {}).items():
        if _is_error_status(code):
            continue
        r = _resolve(c, r)
        headers = ", ".join(f"`{h}`" for h in r.get("headers", {}))
        content = _content(c, r)
        bits = [b for b in (content, f"headers {headers}" if headers else "") if b]
        lines.append(f"- `{code}`: {'; '.join(bits) if bits else 'no body'}")
    if o.get("x-order"):
        lines.append(f"- Order: {o['x-order']}")
    lines += [""]

    lines += ["**Failures:**", ""]
    own = o.get("x-errors", [])
    lines.append("- This route's own codes: " + (", ".join(f"`{e}`" for e in own) if own else "none"))
    shared = [f"`{code}` {_ref_name(r['$ref'])}" if isinstance(r, dict) and "$ref" in r else f"`{code}`"
              for code, r in o.get("responses", {}).items() if _is_error_status(code)]
    lines.append("- Responses: " + ", ".join(shared))
    lines += [""]

    if o.get("x-mutates"):
        idem = ("a write: send `Idempotency-Key`; the same key with the same body replays the first "
                "answer for 24 hours, and the same key with another body is refused (R-11, R-19).")
    else:
        idem = "not a write: safe to retry."
    lines += [f"**Idempotency:** {idem}", ""]

    cli = o.get("x-cli", "")
    lines += [f"**CLI:** {'none' if cli.strip() in ('', '-') else '`' + cli + '`'}", ""]
    return lines


def _codes_table(title, codes):
    lines = [f"### {title}", ""]
    if not codes:
        return lines + ["None.", ""]
    lines += ["| Code | Status |", "|---|---|"]
    for code, status in codes.items():
        if isinstance(status, list):
            status = ", ".join(str(s) for s in status)
        lines.append(f"| `{cell(code)}` | {cell(status)} |")
    return lines + [""]


def _rule_order(key):
    """'R-10 name' sorts by its number; a key that is not 'R-<n> ...' sorts after them, in file order."""
    head = str(key).split(" ", 1)[0]
    return (0, int(head[2:])) if head.startswith("R-") and head[2:].isdigit() else (1, 0)


def _rules(c):
    """The route rules (x-rules): one heading per rule, its full text, in R-number order."""
    rules = c.get("x-rules", {})
    lines = ["## Route rules", ""]
    if not rules:
        return lines + ["None.", ""]
    for key in sorted(rules, key=_rule_order):
        lines += [f"### {cell(key)}", "", " ".join(str(rules[key]).split()), ""]
    return lines


def _where(c, ptr):
    """Where in the contract a pattern sits, as Markdown: `Schema.field` (`Schema.field[]` for the
    items of a list), `operationId` parameter `name`, else the JSON pointer."""
    if ptr[:2] == ["components", "schemas"] and len(ptr) >= 5 and ptr[3] == "properties":
        if ptr[5:] == []:
            return f"`{ptr[2]}.{ptr[4]}`"
        if ptr[5:] == ["items"]:
            return f"`{ptr[2]}.{ptr[4]}[]`"
        return None
    if ptr[:2] == ["components", "parameters"] and ptr[3:] == ["schema"]:
        return f"parameter `{ptr[2]}`"
    if ptr[0] == "paths" and len(ptr) == 6 and ptr[3] == "parameters" and ptr[5] == "schema":
        o = c["paths"][ptr[1]][ptr[2]]
        return f"`{_name(ptr[1], ptr[2], o)}` parameter `{o['parameters'][ptr[4]].get('name', '?')}`"
    return None


def _pattern_sites(c, pattern):
    """Every place the contract checks a value against this exact pattern, in contract order.

    The contract does not link a grammar to its fields; "Applies to" is found by pattern equality, so
    two grammars with the same pattern would each list the other's fields (the four of r2.7 differ).
    An entry with no usable pattern has no sites (None would match every dict without a pattern)."""
    if not isinstance(pattern, str) or not pattern:
        return []
    found = []

    def walk(node, ptr):
        if isinstance(node, dict):
            if node.get("pattern") == pattern:
                found.append(_where(c, ptr) or "`" + "/".join(str(x) for x in ptr) + "`")
            for k, v in node.items():
                walk(v, ptr + [k])
        elif isinstance(node, list):
            for i, v in enumerate(node):
                walk(v, ptr + [i])

    for top in ("paths", "components"):
        walk(c.get(top, {}), [top])
    return list(dict.fromkeys(found))


def _grammars(c):
    """The label grammars (x-label-grammar): every one, with the fields it is checked against."""
    g = c.get("x-label-grammar", {})
    lines = ["## Label grammars", ""]
    if not g:
        return lines + ["None.", ""]
    lines += ["| Grammar | Pattern | At most (characters) | Source | Applies to |", "|---|---|---|---|---|"]
    for key, v in g.items():
        sites = _pattern_sites(c, v.get("pattern"))
        lines.append(f"| `{cell(key)}` | `{cell(v.get('pattern', ''))}` | {cell(v.get('maxLength', ''))} | "
                     f"{cell(v.get('source', ''))} | {cell(', '.join(sites)) if sites else 'no field in this contract carries this pattern'} |")
    return lines + [""]


def _notes(c):
    """The contract notes (x-notes) except the internal ones: one heading per note, its full text,
    in contract order."""
    notes = {k: v for k, v in c.get("x-notes", {}).items() if k not in INTERNAL_NOTES}
    lines = ["## Notes", ""]
    if not notes:
        return lines + ["None.", ""]
    for key, body in notes.items():
        lines += [f"### {cell(key)}", "", " ".join(str(body).split()), ""]
    return lines


def render(c, source_name, source_sha):
    ops = list(operations(c))
    info = c.get("info", {})
    lines = [
        "# Cortex API reference (generated)",
        "",
        f"Generated from `{source_name}` (sha256 `{source_sha}`) by `tools/render_reference.py`. "
        "Do not edit: this page is a build product. Correct the contract and run "
        "`tools/build-docs.sh --write`.",
        "",
        f"{cell(info.get('title', ''))} {cell(info.get('version', ''))}: {len(ops)} operations on "
        f"{len(c.get('paths', {}))} paths.",
        "",
    ]
    lines += _rules(c) + _grammars(c) + _notes(c) + ["## Operations", ""]
    for path, method, o in ops:
        lines += _operation(c, path, method, o)
    ec = c.get("x-error-codes", {})
    lines += ["## Errors", ""]
    if ec.get("rule"):
        lines += [" ".join(str(ec["rule"]).split()), ""]
    lines += _codes_table("Platform and chain codes", ec.get("platform_and_chain", {}))
    lines += _codes_table("Piece codes", ec.get("pieces", {}))
    lines += _codes_table("Deferred codes", ec.get("deferred", {}))
    lines += _codes_table("Retired in r2", ec.get("retired_in_r2", {}))
    # cox W1-34 (S3 r2.7): a code several pieces declare has ONE status, title and fix.
    shared = c.get("x-shared-codes", {})
    lines += ["### Codes shared by several pieces", ""]
    if shared.get("rule"):
        lines += [" ".join(str(shared["rule"]).split()), ""]
    codes = shared.get("codes", {})
    if codes:
        lines += ["| Code | Status | Title | Fix |", "|---|---|---|---|"]
        for code, v in codes.items():
            lines.append(f"| `{cell(code)}` | {cell(v.get('status', ''))} | {cell(v.get('title', ''))} | "
                         f"{cell(v.get('fix', ''))} |")
        lines += [""]
    else:
        lines += ["None.", ""]
    host = c.get("x-host-commands", {})
    if host:
        lines += ["## Host commands (not HTTP)", "", "| Command | What it does |", "|---|---|"]
        for cmd, what in host.items():
            lines.append(f"| `{cell(cmd)}` | {cell(what)} |")
        lines += [""]
    return "\n".join(lines)


# ---- the command line ----


def _refuse(problems):
    for p in problems:
        print(p, file=sys.stderr)
    return 1


def main(argv):
    # The same bytes under any locale (hale N-2).
    sys.stdout.reconfigure(encoding="utf-8", newline="\n")
    if len(argv) >= 2 and argv[0] in ("check", "render", "cli-list"):
        path = Path(argv[1])
        c = load(path)
        if argv[0] == "check":
            problems = check(c)
            return _refuse(problems) if problems else 0
        if argv[0] == "render":
            sha = hashlib.sha256(path.read_bytes()).hexdigest()
            sys.stdout.write(render(c, path.name, sha))
            return 0
        sys.stdout.write("".join(f"{s}\n" for s in cli_commands(c)))
        return 0
    if len(argv) == 3 and argv[0] == "capture":
        text, problems = capture(load(argv[1]), argv[2])
        if problems:
            return _refuse(problems)
        sys.stdout.write(text)
        return 0
    if len(argv) == 3 and argv[0] == "inventory":
        try:
            lines = Path(argv[2]).read_text(encoding="utf-8").splitlines()
        except (OSError, UnicodeDecodeError) as e:
            return _refuse([f"cannot read the inventory file {argv[2]}: {type(e).__name__}"])
        problems = inventory_diff(load(argv[1]), lines)
        return _refuse(problems) if problems else 0
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
