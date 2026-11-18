#!/usr/bin/env python3
"""Check real fetch and requalification screens against the unmodified and mutated include."""
import json
import os
from pathlib import Path
import subprocess
import tempfile

root = Path(__file__).resolve().parents[3]
scratch = Path(os.environ["TMPDIR"]).resolve()
if scratch == Path("/tmp") or Path("/tmp") in scratch.parents:
    raise SystemExit("private scratch outside /tmp required")
source = (root / "deploy/install-prereq.sh").read_text()
mutants = {m["id"]: m for m in json.loads((root / "scripts/install-prereq-mutants.json").read_text())}
with tempfile.TemporaryDirectory(prefix="fetch-screen-", dir=scratch) as folder:
    for ident in ("IPM-52", "IPM-53", "IPM-54", "IPM-55", "IPM-56", "IPM-73"):
        mutant = mutants[ident]
        if source.count(mutant["old"]) != 1:
            raise SystemExit(f"bad {ident} anchor")
        candidate = Path(folder) / "mutant.sh"
        candidate.write_text(source.replace(mutant["old"], mutant["new"], 1))
        for locale_order in ("C C.utf8", "C.utf8 C"):
            for label, path in (("baseline", root / "deploy/install-prereq.sh"), ("mutant", candidate)):
                env = dict(os.environ, PREREQ_TEST_SOURCE=str(path), PREREQ_SCREEN_MUTANT=ident, PREREQ_TEST_LOCALES=locale_order, PREREQ_TEST_TMP=folder)
                run = subprocess.run(["sh", str(root / "scripts/install-prereq-test.sh")], env=env, capture_output=True, text=True, check=False)
                output = run.stdout + run.stderr
                if label == "baseline":
                    if run.returncode != 0 or f"SCREEN {ident}: survived" not in output:
                        raise SystemExit(f"{ident} baseline screen failed ({locale_order}): {output}")
                else:
                    failures = [line for line in output.splitlines() if line.startswith("FAIL ")]
                    if run.returncode == 0 or not failures or not failures[0].startswith(mutant["fails"]):
                        raise SystemExit(f"{ident} wrong mutant failure ({locale_order}): {output}")
        print(f"PASS {ident}: real cell survives baseline and catches named fault in both locale orders", flush=True)
