#!/usr/bin/python3 -I
"""G12 input matrix, layer L1 (SOURCE TIME), r2: nix. For one included installer file and one cell (caller environment variables unset, empty, valid or
hostile, the argument vector) it asks the S5-10 ORACLE (hue's, imported by path from the development tree, never copied, never unconfined) whether sourcing the
file runs, prints or changes anything.  The caller environment is given at LAUNCH of the leaf shell (the process environment of the oracle's namespace) and
the effective values are OBSERVED before anything else runs; the include context is the REAL prefix of install.sh up to its first include marker (set -eu,
umask 077, the assignments, the functions), replayed; the file run is compared with an EMPTY-file run of the same cell and the same context, so a varied cell
differs from the clean one on purpose and nothing is normalised; both raw outputs are kept.  The matrix is FINITE and is never claimed as proof for every
environment.  Synthetic values only: the canary is a shell builtin (echo) that prints CANARY; nothing here is destructive."""
import hashlib, importlib.util, json, os, re

NEEDED = ("oracle_tools", "_oracle_run", "oracle_baseline", "ORACLE_PRELUDE", "ORACLE_EPILOGUE", "ORACLE_SENTINEL_STATUS", "ORACLE_ENV")
MARKER = re.compile(rb"#\s*@include\s*$")


class OracleMissing(Exception):
    pass


def load_oracle(dev_tree):
    """hue's oracle module from the development tree at the candidate commit. Raises OracleMissing, or the module's OracleUnavailable, and never falls back."""
    path = os.path.join(dev_tree, "scripts", "s5_check.py")
    for link in (os.path.join(dev_tree, "scripts"), path):   # F4, in depth: the code that is about to be executed in this process is never reached through a symbolic link
        if os.path.islink(link):
            raise OracleMissing("%s is a symbolic link: the oracle is not loaded through a link" % link)
    if not os.path.isfile(path):
        raise OracleMissing("no scripts/s5_check.py in the development tree %s" % dev_tree)
    spec = importlib.util.spec_from_file_location("s5_check_for_matrix", path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    missing = [n for n in NEEDED if not hasattr(mod, n)]
    if missing:
        raise OracleMissing("the checker at %s has no oracle (missing %s)" % (path, ", ".join(missing)))
    mod.oracle_tools()
    mod.oracle_baseline()
    return mod


def sha256_file(path):
    with open(path, "rb") as fh:
        return hashlib.sha256(fh.read()).hexdigest()


def prefix_from_install(dev_install):
    """The include context: install.sh from its first line up to (not including) the first include marker. Only `here` is replaced (it needs cd and a command
    substitution, which the restricted oracle shell refuses): everything else is replayed exactly."""
    out, found = [], False
    for line in dev_install.splitlines(keepends=True):
        if MARKER.search(line):
            found = True
            break
        out.append(b"here='/oracle/here'\n" if line.startswith(b"here=$(") else line)
    if not found:
        raise OracleMissing("no '# @include' marker in install.sh")
    return b"".join(out)


def generate_cells(inv, untestable=()):
    """The cells, in a fixed order, a clean control first. A variable the leaf refuses to start with (untestable) still gets its single cells, which report
    UNTESTABLE, but is left out of the profiles."""
    cells = []

    def add(kind, set_, unset_, argv, why):
        cells.append({"id": "M%03d" % len(cells), "kind": kind, "set": dict(set_), "unset": list(unset_), "argv": list(argv), "why": why})

    add("control", {}, [], [], "clean control: no caller variable, no argument")
    for v in inv["variables"]:
        name = v["name"]
        add("unset", {}, [name], [], "%s unset" % name)
        add("empty", {name: ""}, [], [], "%s empty" % name)
        for val in v["valid"]:
            add("valid", {name: val}, [], [], "%s valid" % name)
        for val in v["hostile"]:
            add("hostile", {name: val}, [], [], "%s hostile" % name)
    for c in inv["interacting"]:
        add("interacting", c["set"], c.get("unset", []), [], c["why"])
    for argv in inv["argv"]:
        for profile in inv["profiles"]:
            if profile == "clean":
                vars_ = {}
            elif profile == "all_valid":
                vars_ = {v["name"]: v["valid"][0] for v in inv["variables"] if v["name"] not in untestable}
            else:
                vars_ = {v["name"]: (v["hostile"][0] if v["hostile"] else v["valid"][0]) for v in inv["variables"] if v["name"] not in untestable}
            add("argv", vars_, [], argv, "argv %r, profile %s" % (argv, profile))
    return cells


def quote(s):
    return "'" + s.replace("'", "'\\''") + "'"


def launch_env(oracle, cell):
    """The process environment of the leaf shell: the oracle's own fixed one, with the cell's variables set or removed."""
    env = dict(oracle.ORACLE_ENV)
    for name in cell["unset"]:
        env.pop(name, None)
    env.update(cell["set"])
    return env


def observations(cell):
    """Printed BEFORE anything else runs: the values the leaf shell really has for the cell's variables."""
    names = sorted(set(cell["set"]) | set(cell["unset"]))
    lines = ["printf 'OBS %%s=%%s\\n' %s \"${%s-<unset>}\"" % (quote(n), n) for n in names]
    lines.append("printf 'OBS ARGV=%s\\n' \"$#\"")
    return ("\n".join(lines) + "\n").encode()


def script_for(oracle, cell, prefix, body):
    first = oracle.ORACLE_PRELUDE.replace(b"set -x\n", b"")
    args = ("set --" + "".join(" " + quote(a) for a in cell["argv"]) + "\n").encode()
    return first + observations(cell) + args + prefix + b"set -x\n" + body + oracle.ORACLE_EPILOGUE


def took_launch_values(cell, out):
    """Every variable the cell sets must be OBSERVED with that value (a value may span lines; SHELLOPTS and BASHOPTS are lists that may hold more). A leaf that
    did not take a value cannot show that value's effect: the cell is UNTESTABLE, listed, and never an accept (a restricted bash ignores SHELLOPTS and BASHOPTS
    at startup by design)."""
    lines = out.decode("utf-8", "replace").splitlines()
    for name, value in cell["set"].items():
        start = next((i for i, l in enumerate(lines) if l.startswith("OBS %s=" % name)), None)
        seen = None
        if start is not None:
            parts = [lines[start].split("=", 1)[1]]
            for l in lines[start + 1:]:
                if l.startswith("OBS ") or l.startswith("+ "):
                    break
                parts.append(l)
            seen = "\n".join(parts)
        if seen is None or not (seen == value or (name in ("SHELLOPTS", "BASHOPTS") and value in seen)):
            return "the leaf did not take %s (asked %r, observed %r)" % (name, value[:30], (seen if seen is not None else "nothing")[:40])
    return ""


def run_cell(oracle, data, cell, prefix):
    """(outcome, detail, base output, file output): ACCEPT, REFUSE, or UNTESTABLE (the leaf cannot start with this context or did not take a value, so the cell proves nothing)."""
    saved = oracle.ORACLE_ENV
    oracle.ORACLE_ENV = launch_env(oracle, cell)
    try:
        base_out, base_rc, _ = oracle._oracle_run(script_for(oracle, cell, prefix, b""), None)
        text = base_out.decode("utf-8", "replace")
        if base_rc != oracle.ORACLE_SENTINEL_STATUS or "restricted" in text or "readonly variable" in text:
            why = next((l for l in text.splitlines() if l.strip() and not l.startswith("OBS ")), "status %s" % base_rc)
            return "UNTESTABLE", "the leaf cannot run this context: %s" % why.strip()[:90], base_out, b""
        missed = took_launch_values(cell, base_out)
        if missed:
            return "UNTESTABLE", missed, base_out, b""
        out, rc, note = oracle._oracle_run(script_for(oracle, cell, prefix, data), base_out)
    finally:
        oracle.ORACLE_ENV = saved
    if rc == oracle.ORACLE_SENTINEL_STATUS and out == base_out:
        return "ACCEPT", "", base_out, out
    if out and out != base_out[:len(out)]:
        words = " | ".join(l.strip() for l in out.decode("utf-8", "replace").splitlines() if l.strip() and l.strip() not in text)
        return "REFUSE", (words or "bash printed something that an empty file does not")[:120], base_out, out
    return "REFUSE", note or "bash ended with status %s where an empty file ends with %s" % (rc, oracle.ORACLE_SENTINEL_STATUS), base_out, out


def probe_untestable(oracle, inv, prefix):
    """The variables the leaf shell cannot start with or does not take, with the first valid value or the first hostile one (they are kept out of the profiles)."""
    names = set()
    for v in inv["variables"]:
        for value in [v["valid"][0]] + v["hostile"][:1]:
            cell = {"id": "probe", "kind": "probe", "set": {v["name"]: value}, "unset": [], "argv": [], "why": ""}
            if run_cell(oracle, b"", cell, prefix)[0] == "UNTESTABLE":
                names.add(v["name"])
    return names


def run_matrix(oracle, files, inv, prefix):
    """files: {name: bytes}. Returns (rows, counts, untestable names, raw); a row is (cell id, file, kind, why, outcome, detail); raw keeps both outputs of every run."""
    untestable = probe_untestable(oracle, inv, prefix)
    rows, raw, counts = [], [], {"ACCEPT": 0, "REFUSE": 0, "UNTESTABLE": 0}
    for cell in generate_cells(inv, untestable):
        for name in sorted(files):
            outcome, detail, base_out, out = run_cell(oracle, files[name], cell, prefix)
            counts[outcome] += 1
            rows.append((cell["id"], name, cell["kind"], cell["why"], outcome, detail))
            raw.append((cell["id"], name, outcome, base_out, out))
    return rows, counts, untestable, raw


def table(rows):
    clean = lambda t: t.replace("\t", " ").replace("\n", "\\n")
    return "".join("\t".join(clean(str(x)) for x in r) + "\n" for r in rows)


def raw_text(raw):
    """Every observation as the leaf printed it (no normalising): the empty-file run and the file run of each cell."""
    out = []
    for cid, name, outcome, base_out, run_out in raw:
        out.append("=== %s %s %s\n--- empty-file run\n%s\n--- file run\n%s\n" % (cid, name, outcome, base_out.decode("utf-8", "replace"), run_out.decode("utf-8", "replace")))
    return "".join(out)
