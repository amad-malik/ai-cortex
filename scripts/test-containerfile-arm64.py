#!/usr/bin/env python3
"""test-containerfile-arm64.py: cells and mutants for scripts/containerfile-arm64-check.py, and the variant under the REAL S5 checker.

Run:  python3 -I -B scripts/test-containerfile-arm64.py        (exit 0 only if every cell passes and every mutant is killed)

CELLS (the real files) must pass. MUTANTS: each one breaks exactly one guard in a temporary copy, and the check must FAIL (exit 1) with the message that names that guard;
a mutant whose edit did not change the file counts as a failure of the test, not as a kill. Nothing here writes into the repository: every copy is in a temporary directory.
S5 CELLS: the variant is placed as Containerfile (with its dockerignore as Containerfile.dockerignore) in a copy of deploy/, and scripts/s5-check.sh judges it by every S5 rule;
the S5 MUTANTS show that this judgement is not vacuous (a one-fault variant fails there).
"""
import difflib
import hashlib
import os
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
DEPLOY = os.path.join(ROOT, "deploy")
CHECK = os.path.join(HERE, "containerfile-arm64-check.py")
S5 = os.path.join(HERE, "s5-check.sh")
FILES = ["Containerfile", "Containerfile.dockerignore", "Containerfile.arm64", "Containerfile.arm64.dockerignore"]
AMD64 = "e372170ca8630f0f03e9b70fdd0bf4a3ce3426b0de7cdba615f06337389de176"
ARM64 = "ef3f509cbc7109e0fa77463bab23ba74a5d65c3335bb11759f13a13a0c43764d"
X86_SHA = "b1c8cb14e30889e7e2999e6b69e62d9e9b9ffc0f4df8180564d306d9680e6c55"
ARM_SHA = "9f608c142e0e38a78957d0661ba1dfaf0b6fb37184cfaa4b9a0aa57af6184282"

results = []   # (name, passed, detail)


def sha(p):
    with open(p, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()


def run_check(d, *extra):
    p = subprocess.run([sys.executable, "-I", "-B", CHECK, *extra, d], capture_output=True, text=True, timeout=120)
    return p.returncode, p.stdout + p.stderr


def fresh(tmp, name):
    d = os.path.join(tmp, name)
    os.makedirs(d)
    for f in FILES:
        shutil.copyfile(os.path.join(DEPLOY, f), os.path.join(d, f))
    return d


def rd(d, f):
    with open(os.path.join(d, f), "r", encoding="utf-8", newline="") as fh:
        return fh.read()


def wr(d, f, text):
    with open(os.path.join(d, f), "w", encoding="utf-8", newline="") as fh:
        fh.write(text)


def record(name, passed, detail=""):
    results.append((name, passed, detail))
    print(("ok    " if passed else "FAIL  ") + name + (("  :: " + detail) if (detail and not passed) else ""))


def mutant(tmp, name, edit, want, rc_want=1):
    """edit(dir) changes the copy and returns False if it could not; the check must exit rc_want and print `want`."""
    d = fresh(tmp, name)
    before = {f: (sha(os.path.join(d, f)) if os.path.isfile(os.path.join(d, f)) and not os.path.islink(os.path.join(d, f)) else None) for f in FILES}
    if edit(d) is False:
        return record(name, False, "the edit did not apply")
    after = {f: (sha(os.path.join(d, f)) if os.path.isfile(os.path.join(d, f)) and not os.path.islink(os.path.join(d, f)) else None) for f in FILES}
    islink = any(os.path.islink(os.path.join(d, f)) for f in FILES)
    if before == after and not islink:
        return record(name, False, "the edit changed no file: this is not a mutant")
    rc, out = run_check(d)
    record(name, rc == rc_want and want in out, f"rc={rc} (wanted {rc_want}); wanted text {want!r}; got: {out.strip().splitlines()[-3:]}")


def sub(f, old, new, count=None):
    def e(d):
        t = rd(d, f)
        if old not in t or (count is not None and t.count(old) != count):
            return False
        wr(d, f, t.replace(old, new))
    return e


def main():
    with tempfile.TemporaryDirectory(prefix="arm64-cf-test.") as tmp:
        # ---- CELLS on the real files
        rc, out = run_check(DEPLOY)
        record("C01 the real files pass", rc == 0 and "containerfile-arm64-check: OK" in out, out[-300:])
        hashes = {f: sha(os.path.join(DEPLOY, f)) for f in FILES}
        run_check(DEPLOY)
        record("C02 the check writes nothing (the four files are unchanged by a run)", hashes == {f: sha(os.path.join(DEPLOY, f)) for f in FILES})
        p = subprocess.run([sys.executable, "-I", "-B", CHECK, "--print", DEPLOY], capture_output=True, text=True, timeout=60)
        record("C03 --print renders exactly the variant on disk", p.returncode == 0 and p.stdout == rd(DEPLOY, "Containerfile.arm64"))
        x = rd(DEPLOY, "Containerfile")
        a = rd(DEPLOY, "Containerfile.arm64")
        record("C04 the variant holds the arm64 base twice and the amd64 base never", a.count(ARM64) == 2 and AMD64 not in a)
        record("C05 the x86_64 file still holds the amd64 base twice, the x86_64 rustup file and sha, and none of the arm64 pins", x.count(AMD64) == 2 and "/x86_64-unknown-linux-gnu/rustup-init" in x and X86_SHA in x and ARM64 not in x and ARM_SHA not in x)
        record("C06 the variant holds the aarch64 rustup file and sha and neither x86_64 one", "/aarch64-unknown-linux-gnu/rustup-init" in a and ARM_SHA in a and "x86_64-unknown-linux-gnu" not in a and X86_SHA not in a)
        ops = [o for o in difflib.SequenceMatcher(None, x.splitlines(), a.splitlines(), autojunk=False).get_opcodes() if o[0] != "equal"]
        removed = sum(o[2] - o[1] for o in ops)
        added = sum(o[4] - o[3] for o in ops)
        record("C07 the variant differs from the x86_64 file in exactly SIX hunks (header, jobs note, build FROM, sha ARG with the ENV line, rustup URL, run FROM): 7 lines out, 14 in", len(ops) == 6 and removed == 7 and added == 14, f"hunks={len(ops)} removed={removed} added={added}")
        rc, out = subprocess.run([sys.executable, "-I", "-B", CHECK, "a", "b"], capture_output=True, text=True, timeout=60).returncode, ""
        record("C08 two arguments are a usage error (exit 64)", rc == 64)

        # ---- MUTANTS of the check
        mutant(tmp, "MA01 both FROM digests back to the x86_64 base", sub("Containerfile.arm64", ARM64, AMD64, 2), "is not Containerfile with exactly the replacements")
        mutant(tmp, "MA02 only the build-stage FROM back to the x86_64 base", sub("Containerfile.arm64", "@sha256:" + ARM64 + " AS build", "@sha256:" + AMD64 + " AS build", 1), "is not Containerfile with exactly the replacements")
        mutant(tmp, "MA03 the variant fetches the x86_64 rustup-init", sub("Containerfile.arm64", "/aarch64-unknown-linux-gnu/rustup-init", "/x86_64-unknown-linux-gnu/rustup-init", 1), "is not Containerfile with exactly the replacements")
        mutant(tmp, "MA04 the variant checks the x86_64 rustup-init sha256", sub("Containerfile.arm64", ARM_SHA, X86_SHA, 1), "is not Containerfile with exactly the replacements")
        mutant(tmp, "MA05 the jobs cap is back to 2 in the variant", sub("Containerfile.arm64", "CARGO_BUILD_JOBS=4 CARGO_TARGET_DIR=", "CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=", 1), "is not Containerfile with exactly the replacements")
        mutant(tmp, "MA06 the variant drifts: cargo build without --release", sub("Containerfile.arm64", "cargo build --release --locked", "cargo build --locked", 1), "is not Containerfile with exactly the replacements")
        mutant(tmp, "MA07 the x86_64 file drifts (cargo build without --release) and the variant does not follow", sub("Containerfile", "cargo build --release --locked", "cargo build --locked", 1), "is not Containerfile with exactly the replacements")
        mutant(tmp, "MA08 the variant is missing", lambda d: os.remove(os.path.join(d, "Containerfile.arm64")), "Containerfile.arm64 does not exist")
        mutant(tmp, "MA09 the variant is a link to the x86_64 file", lambda d: (os.remove(os.path.join(d, "Containerfile.arm64")), os.symlink("Containerfile", os.path.join(d, "Containerfile.arm64"))), "is a symbolic link")
        mutant(tmp, "MA10 the variant's dockerignore has one more line", lambda d: wr(d, "Containerfile.arm64.dockerignore", rd(d, "Containerfile.arm64.dockerignore") + "extra\n"), "differs from Containerfile.dockerignore")
        mutant(tmp, "MA11 the variant's dockerignore is missing", lambda d: os.remove(os.path.join(d, "Containerfile.arm64.dockerignore")), "Containerfile.arm64.dockerignore does not exist")
        mutant(tmp, "MA12 the variant has CRLF line ends", lambda d: wr(d, "Containerfile.arm64", rd(d, "Containerfile.arm64").replace("\n", "\r\n")), "carriage return")
        mutant(tmp, "MA13 the x86_64 base digest changed (the replacement no longer applies)", sub("Containerfile", AMD64, "0" * 64, 2), "needs its old text 2 time(s)")
        mutant(tmp, "MA14 a digit lost from the variant's FROM digest", sub("Containerfile.arm64", "@sha256:" + ARM64 + " AS build", "@sha256:" + ARM64[:-1] + " AS build", 1), "is not Containerfile with exactly the replacements")
        mutant(tmp, "MA15 the x86_64 file is missing", lambda d: os.remove(os.path.join(d, "Containerfile")), "Containerfile does not exist")
        mutant(tmp, "MA16 the variant gains a step (an extra RUN line at the end)", lambda d: wr(d, "Containerfile.arm64", rd(d, "Containerfile.arm64") + "RUN true\n"), "is not Containerfile with exactly the replacements")
        mutant(tmp, "MA17 a comment of the variant changed (the check is strict on comments too)", sub("Containerfile.arm64", "THE ARM64 VARIANT differs from", "THE ARM64 VARIANT may differ from", 1), "is not Containerfile with exactly the replacements")
        mutant(tmp, "MA18 the x86_64 rustup sha256 changed (the replacement no longer applies)", sub("Containerfile", X86_SHA, "0" * 64, 1), "needs its old text 1 time(s)")
        mutant(tmp, "MA19 the x86_64 dockerignore changed and the variant's did not follow", lambda d: wr(d, "Containerfile.dockerignore", rd(d, "Containerfile.dockerignore") + "extra\n"), "differs from Containerfile.dockerignore")

        # ---- the variant under the REAL S5 checker: it is placed as Containerfile in a copy of deploy/
        def mini_repo(name):
            """<tmp>/<name>/deploy (a copy of deploy/) beside <tmp>/<name>/crates/cortex-server/Cargo.toml: where S5 looks for the feature list."""
            root = os.path.join(tmp, name)
            shutil.copytree(DEPLOY, os.path.join(root, "deploy"), symlinks=True)
            os.makedirs(os.path.join(root, "crates", "cortex-server"))
            shutil.copyfile(os.path.join(ROOT, "crates", "cortex-server", "Cargo.toml"), os.path.join(root, "crates", "cortex-server", "Cargo.toml"))
            return os.path.join(root, "deploy")

        def s5_layout(name, variant_edit=None):
            lay = mini_repo(name)
            shutil.copyfile(os.path.join(DEPLOY, "Containerfile.arm64"), os.path.join(lay, "Containerfile"))
            shutil.copyfile(os.path.join(DEPLOY, "Containerfile.arm64.dockerignore"), os.path.join(lay, "Containerfile.dockerignore"))
            if variant_edit is not None and variant_edit(lay) is False:
                return None
            return lay

        def s5(lay):
            q = subprocess.run(["sh", S5, lay], capture_output=True, text=True, timeout=300)
            return q.returncode, q.stdout + q.stderr

        control = mini_repo("s5-control")
        rc, out = s5(control)
        record("S01 control: the x86_64 deploy directory passes S5", rc == 0, out[-250:])
        lay = s5_layout("s5-variant")
        rc, out = s5(lay)
        record("S02 the ARM64 VARIANT, placed as Containerfile, passes every S5 rule", rc == 0, out[-250:])

        def s5_mut(name, edit, want):
            lay = s5_layout(name, edit)
            if lay is None:
                return record(name, False, "the edit did not apply")
            rc, out = s5(lay)
            record(name, rc != 0 and want in out, f"rc={rc}; wanted {want!r}; got {out.strip().splitlines()[-3:]}")

        s5_mut("SM01 the variant loses ARG CORTEX_GIT_SHA=unknown: S5 refuses it", lambda l: sub("Containerfile", "ARG CORTEX_GIT_SHA=unknown\n", "", 1)(l), "S5-20")
        s5_mut("SM02 the variant COPYs an env file into the image: S5 refuses it", lambda l: sub("Containerfile", "COPY xtask ./xtask\n", "COPY xtask ./xtask\nCOPY .env /src/.env\n", 1)(l), "S5-05")
        s5_mut("SM03 the variant builds with --all-features: S5 refuses it", lambda l: sub("Containerfile", '--features "$(cat server-features)"', "--all-features", 1)(l), "S5-23")

    failed = [r for r in results if not r[1]]
    cells = [r for r in results if r[0][0] in "CS" and not r[0].startswith(("SM",))]
    muts = [r for r in results if r[0].startswith(("MA", "SM"))]
    print(f"test-containerfile-arm64: {len(results) - len(failed)} of {len(results)} passed ({len(cells)} cells, {len(muts)} mutants; {len([m for m in muts if m[1]])} killed)")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
