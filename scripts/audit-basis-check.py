#!/usr/bin/env python3
"""audit-basis-check.py: the REVIEWED-BASIS GUARD of the audit exception (hue, 10 Nov 2026; sam's AE-SAM-01 on the audit pin, cox row 655dbb5e).

kai's line in .cargo/audit.toml says the exception for RUSTSEC-2026-0459 is VOID 'the moment that command prints a path OR THE SQLX FEATURES CHANGE'. scripts/audit-gate.sh proves the first half
(the crate is in no build); this checks the second: it reads the JSON of  cargo metadata --locked --offline --no-deps --format-version 1  on stdin and compares EVERY sqlx dependency of EVERY workspace
crate (each member's own record, so a member that adds a feature over the workspace entry is seen too, and dev and build records as well) with the basis kai reviewed:
    version requirement  =0.9.0
    default-features     false
    features             exactly the SET runtime-tokio, postgres, uuid, tls-none      (a subset, a superset or a prefix is a difference; the ORDER is not)
FIRST the metadata is VALIDATED (see validate(): required fields and their types, a missing array is not an empty one, a dependency without a name is not skipped, features must be an array of strings): malformed or partial
input exits 1 with 'cannot be made' and the phrase of the failing check, never 'basis OK'; unrelated extra fields are allowed. THEN it exits 0 when every record equals the basis, and 1 with 'EXCEPTION VOID' on any
difference or on no sqlx dependency at all (nothing to compare).

WHAT IT DOES NOT COVER (kai owns any wider policy; nothing is widened here): the 'optional', 'target', 'kind', 'rename' and 'source' of a sqlx dependency; any crate other than sqlx; a lock file change. A transitive change that
enables sqlite through ANOTHER crate is covered by the graph proof of audit-gate.sh; a lock change that adds ANOTHER vulnerable crate is the audit's own job.
"""
import json
import sys

BASIS_REQ = "=0.9.0"
BASIS_DEFAULT_FEATURES = False
BASIS_FEATURES = frozenset(["runtime-tokio", "postgres", "uuid", "tls-none"])
BASIS_TEXT = "version =0.9.0, default-features false, features postgres, runtime-tokio, tls-none, uuid"


class Malformed(Exception):
    pass


def need(ok, what):
    if not ok:
        raise Malformed(what)


def validate(d):
    """Refuse malformed or partial cargo metadata BEFORE anything is filtered (sam's AE-SAM-02, cox row d722e57a): the required fields of the packages and of every dependency and their TYPES.
    A missing 'dependencies' array is NOT an empty one, a dependency without a name is NOT skipped, 'features' must be an ARRAY OF STRINGS (a mapping or a non-string is refused), 'uses_default_features' a boolean,
    'req' a string; 'workspace_members', when present, an array of strings. UNRELATED extra fields are allowed (Cargo adds fields over time), and so is anything else about a dependency that is not sqlx."""
    need(isinstance(d, dict), "the top level is not an object")
    need(isinstance(d.get("packages"), list), "'packages' is not an array")
    if "workspace_members" in d:
        need(isinstance(d["workspace_members"], list) and all(isinstance(x, str) for x in d["workspace_members"]), "'workspace_members' is not an array of strings")
    for i, p in enumerate(d["packages"]):
        need(isinstance(p, dict), "package %d is not an object" % i)
        need(isinstance(p.get("name"), str), "package %d has no string 'name'" % i)
        need(isinstance(p.get("dependencies"), list), "package %s has no 'dependencies' array" % p["name"])
        for j, dep in enumerate(p["dependencies"]):
            need(isinstance(dep, dict), "dependency %d of package %s is not an object" % (j, p["name"]))
            need(isinstance(dep.get("name"), str), "a dependency has no string 'name' (dependency %d of package %s)" % (j, p["name"]))
            if dep["name"] == "sqlx":
                need(isinstance(dep.get("req"), str), "the sqlx dependency of package %s: 'req' is not a string" % p["name"])
                need(isinstance(dep.get("uses_default_features"), bool), "the sqlx dependency of package %s: 'uses_default_features' is not a boolean" % p["name"])
                need(isinstance(dep.get("features"), list) and all(isinstance(x, str) for x in dep["features"]), "the sqlx dependency of package %s: 'features' is not an array of strings" % p["name"])


def main():
    try:
        d = json.load(sys.stdin)
        validate(d)
        packages = d["packages"]
    except Malformed as e:
        print("audit-gate: the reviewed-basis check cannot be made (the cargo metadata is malformed: %s): not a pass" % e)
        return 1
    except (ValueError, KeyError, TypeError) as e:
        print("audit-gate: the reviewed-basis check cannot be made (the input is not cargo metadata JSON: %s)" % type(e).__name__)
        return 1
    records = [(p["name"], dep) for p in packages for dep in p.get("dependencies", []) if dep.get("name") == "sqlx"]
    if not records:
        print("audit-gate: EXCEPTION VOID: no workspace crate depends on sqlx, so the reviewed basis (%s) cannot be compared; the exception was approved for that basis only" % BASIS_TEXT)
        return 1
    bad = []
    for crate, dep in records:
        features = frozenset(dep.get("features") or [])
        if dep.get("req") != BASIS_REQ or dep.get("uses_default_features") is not BASIS_DEFAULT_FEATURES or features != BASIS_FEATURES:
            bad.append("  %s: version %s, default-features %s, features %s" % (crate, dep.get("req"), str(dep.get("uses_default_features")).lower(), " ".join(sorted(features)) or "none"))
    if bad:
        print("audit-gate: EXCEPTION VOID: the sqlx dependency settings differ from the basis kai reviewed (%s) in %d of %d record(s); the exception no longer holds and the release must be audited again:" % (BASIS_TEXT, len(bad), len(records)))
        for line in bad[:12]:
            print("audit-gate:" + line)
        return 1
    print("audit-gate: basis OK: all %d sqlx dependency record(s) of the %d workspace crate(s) are exactly the reviewed basis (%s)" % (len(records), len(packages), BASIS_TEXT))
    return 0


if __name__ == "__main__":
    sys.exit(main())
