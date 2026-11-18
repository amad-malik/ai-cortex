#!/usr/bin/env python3
"""Check real machine-screen cells against the unmodified and mutated include."""
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
with tempfile.TemporaryDirectory(prefix="machine-screen-", dir=scratch) as folder:
    for ident in ("IPM-04", "IPM-05", "IPM-13", "IPM-34", "IPM-35", "IPM-09", "IPM-10", "IPM-11", "IPM-12", "IPM-14", "IPM-15", "IPM-16", "IPM-17", "IPM-19", "IPM-22", "IPM-23", "IPM-25", "IPM-26", "IPM-27", "IPM-31", "IPM-32", "IPM-33", "IPM-36", "IPM-37", "IPM-38", "IPM-39", "IPM-40", "IPM-41", "IPM-42"):
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
