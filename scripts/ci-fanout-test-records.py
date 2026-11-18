# ci-fanout-test-records.py <scripts dir> <temp dir>: for cell FO-32 of scripts/ci-fanout-test.sh. Reads the result print lines out of the two real drivers' SOURCE, checks their shape, and renders one
# record per real spec id (the first 12) in each driver's own format into <temp dir>/real-<driver>-<k>.records, with the matching id list in real-<driver>-<k>.spec. Prints PROBLEMS and exits 1 on a drift.
import json
import re
import sys

here, tmp = sys.argv[1], sys.argv[2]
problems = []
rendered = {}
for name, drv, spec, want in (("im", "install_mutants.py", "install-mutants.json", 1), ("fm", "install_fetch_mutants.py", "install-fetch-mutants.json", 2)):
    with open("%s/%s" % (here, drv), encoding="utf-8") as f:
        lines = [l.strip() for l in f if "{m['id']:" in l and l.strip().startswith('print(f"')]
    if len(lines) != want:
        problems.append("%s: %d result print lines, expected %d" % (drv, len(lines), want))
    templates = []
    for l in lines:
        m = re.fullmatch(r"print\(f\"\{m\['id'\]:(\d+)\} \{verdict\}  \[\{m\['what'\]\}\]\"\)", l)
        if not m:
            problems.append("%s: a result print line has a different shape: %s" % (drv, l))
            continue
        templates.append("%%-%ds %%s  [%%s]" % int(m.group(1)))
    with open("%s/%s" % (here, spec), encoding="utf-8") as f:
        ids = [e["id"] for e in json.load(f)][:12]
    for k, tpl in enumerate(templates):
        with open("%s/real-%s-%d.records" % (tmp, name, k), "w", encoding="utf-8") as f:
            for i in ids:
                f.write(tpl % (i, "OK (IT-1, IT-2)", "what the mutant does") + "\n")
        with open("%s/real-%s-%d.spec" % (tmp, name, k), "w", encoding="utf-8") as f:
            json.dump([{"id": i} for i in ids], f)
        rendered["%s-%d" % (name, k)] = len(ids)
if problems:
    print("PROBLEMS: " + "; ".join(problems))
    sys.exit(1)
print("rendered " + ", ".join("%s=%d" % kv for kv in sorted(rendered.items())))
