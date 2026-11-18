#!/usr/bin/env python3
"""containerfile-arm64-check.py: proves that deploy/Containerfile.arm64 is deploy/Containerfile and nothing else but the arm64 pins.

THE RULE (D173, kai's ruling 14 Nov: a VARIANT, not build arguments; the x86_64 file, the release head and the released image stay byte-identical):
the arm64 Containerfile is BUILT from the x86_64 one by the exact replacements in REPLACEMENTS below, each with the number of times it must apply.
The check renders the expected variant from the x86_64 file and compares it, byte for byte, with the file on disk. Any other difference, in either file, fails:
  - an edit to the x86_64 file that the variant does not follow (a changed cargo line, a new step, a new base) makes the rendering differ from the variant;
  - an edit to the variant that is not one of the replacements makes it differ from the rendering;
  - a replacement that no longer applies (its old text is gone or occurs another number of times) is reported by name.
It also checks that the two pins are real (the arm64 base and the aarch64 rustup-init sha256 are 64 hex, and not the x86_64 ones), that the variant is a regular file and not
a link, and that Containerfile.arm64.dockerignore is byte-identical to Containerfile.dockerignore (the build tool reads <Containerfile name>.dockerignore: measured with podman 5.8.2).

WHERE THE PINS COME FROM (read from the publishers, 14 Nov 2026):
  arm64 base   quay.io/rockylinux/rockylinux index sha256:827d37bc128288ccf160ee318bb3cb92d591164cb217e92f8bc61e3982ae1834 (registry tag 10.2.20260525.0): its amd64 child is
               EXACTLY the x86_64 pin e372170c..., its linux/arm64/v8 child is ef3f509c... (both manifests created 2026-05-26: the same Rocky build);
  aarch64 sha  https://static.rust-lang.org/rustup/archive/1.29.3/aarch64-unknown-linux-gnu/rustup-init.sha256 (the same publisher's x86_64 listing equals the pin already in
               the released Containerfile: b1c8cb14...).

usage:  python3 -I -B scripts/containerfile-arm64-check.py [DEPLOY_DIR]           check (exit 0 = every check ok, 1 = a check failed, 64 = usage)
        python3 -I -B scripts/containerfile-arm64-check.py --print [DEPLOY_DIR]    print the rendered variant to stdout (how the file was written)
"""
import difflib
import os
import re
import sys

X86 = "Containerfile"
ARM = "Containerfile.arm64"
X86_IGN = "Containerfile.dockerignore"
ARM_IGN = "Containerfile.arm64.dockerignore"

AMD64_BASE = "sha256:e372170ca8630f0f03e9b70fdd0bf4a3ce3426b0de7cdba615f06337389de176"
ARM64_BASE = "sha256:ef3f509cbc7109e0fa77463bab23ba74a5d65c3335bb11759f13a13a0c43764d"
X86_RUSTUP_SHA = "b1c8cb14e30889e7e2999e6b69e62d9e9b9ffc0f4df8180564d306d9680e6c55"
ARM_RUSTUP_SHA = "9f608c142e0e38a78957d0661ba1dfaf0b6fb37184cfaa4b9a0aa57af6184282"
BASE_REPO = "quay.io/rockylinux/rockylinux@"

HEADER_OLD = "# cortex-server image (S5). Built from the repository root: podman build -f deploy/Containerfile .\n"
HEADER_NEW = ("# cortex-server image (S5) for linux/arm64 (D173: the Mac's native server image). THE ARM64 VARIANT of deploy/Containerfile.\n"
              "# Built from the repository root ON AN ARM64 MACHINE: podman build -f deploy/Containerfile.arm64 .\n")
BASES_OLD = "# Every base is pinned by digest. The build runs through cv2-admit on the shared VM (CARGO_BUILD_JOBS=2, its own target directory).\n"
BASES_NEW = (
    "# Every base is pinned by digest. The build runs on a VM of its own, not through cv2-admit: CARGO_BUILD_JOBS is lifted from 2 to 4 here (it changes the build time, not the program).\n"
    "#\n"
    "# THE ARM64 VARIANT differs from deploy/Containerfile ONLY in the lines named below; scripts/containerfile-arm64-check.py builds this file from the x86_64 one by exactly\n"
    "# these replacements and fails on any other difference:\n"
    "#   - the two FROM digests: the linux/arm64/v8 manifest of the SAME Rocky build as the x86_64 pin (registry index sha256:827d37bc128288ccf160ee318bb3cb92d591164cb217e92f8bc61e3982ae1834, tag 10.2.20260525.0);\n"
    "#   - the rustup-init file: aarch64-unknown-linux-gnu, with its own sha256 (the publisher's rustup-init.sha256 listing for 1.29.3);\n"
    "#   - CARGO_BUILD_JOBS: 4 (the ENV line).\n")

# (name, old text, new text, how many times old must occur in the x86_64 file)
REPLACEMENTS = [
    ("header: the file name and the arm64 build line", HEADER_OLD, HEADER_NEW, 1),
    ("header: the jobs cap and the variant note", BASES_OLD, BASES_NEW, 1),
    ("the two FROM digests (build stage and run stage)", BASE_REPO + AMD64_BASE, BASE_REPO + ARM64_BASE, 2),
    ("CARGO_BUILD_JOBS 2 to 4", "CARGO_BUILD_JOBS=2 CARGO_TARGET_DIR=", "CARGO_BUILD_JOBS=4 CARGO_TARGET_DIR=", 1),
    ("the rustup-init sha256", "ARG RUSTUP_INIT_SHA256=" + X86_RUSTUP_SHA, "ARG RUSTUP_INIT_SHA256=" + ARM_RUSTUP_SHA, 1),
    ("the rustup-init file (aarch64)", "/x86_64-unknown-linux-gnu/rustup-init", "/aarch64-unknown-linux-gnu/rustup-init", 1),
]


def render(x86_text):
    """The expected variant. Returns (text, problems): problems name each replacement whose old text does not occur the stated number of times."""
    text, problems = x86_text, []
    for name, old, new, count in REPLACEMENTS:
        found = text.count(old)
        if found != count:
            problems.append(f"the replacement '{name}' needs its old text {count} time(s) in the x86_64 file, found {found}")
            continue
        text = text.replace(old, new)
    return text, problems


def read_regular(path):
    """The bytes of a regular file that is not a link, or (None, why)."""
    if os.path.islink(path):
        return None, "is a symbolic link (it must be a regular file)"
    if not os.path.isfile(path):
        return None, "does not exist (or is not a regular file)"
    with open(path, "rb") as f:
        return f.read(), None


def check(d):
    """Returns a list of (ok, text) lines."""
    out = []

    def ok(t): out.append((True, t))
    def bad(t): out.append((False, t))

    x86_b, why = read_regular(os.path.join(d, X86))
    if why: bad(f"{X86} {why}")
    arm_b, why2 = read_regular(os.path.join(d, ARM))
    if why2: bad(f"{ARM} {why2}")
    xi_b, why3 = read_regular(os.path.join(d, X86_IGN))
    if why3: bad(f"{X86_IGN} {why3}")
    ai_b, why4 = read_regular(os.path.join(d, ARM_IGN))
    if why4: bad(f"{ARM_IGN} {why4}")

    # the pins themselves: shape, and not the x86_64 ones
    if not (re.fullmatch(r"sha256:[0-9a-f]{64}", ARM64_BASE) and re.fullmatch(r"sha256:[0-9a-f]{64}", AMD64_BASE) and ARM64_BASE != AMD64_BASE):
        bad("the arm64 base pin is not a sha256 digest different from the x86_64 one")
    else:
        ok("the arm64 base pin is a sha256 digest and not the x86_64 one")
    if not (re.fullmatch(r"[0-9a-f]{64}", ARM_RUSTUP_SHA) and re.fullmatch(r"[0-9a-f]{64}", X86_RUSTUP_SHA) and ARM_RUSTUP_SHA != X86_RUSTUP_SHA):
        bad("the aarch64 rustup-init sha256 is not 64 hex different from the x86_64 one")
    else:
        ok("the aarch64 rustup-init sha256 is 64 hex and not the x86_64 one")

    if x86_b is not None and arm_b is not None:
        try:
            x86_t, arm_t = x86_b.decode("utf-8"), arm_b.decode("utf-8")
        except UnicodeDecodeError:
            bad("a Containerfile is not UTF-8")
            return out
        if "\r" in x86_t or "\r" in arm_t:
            bad("a Containerfile has a carriage return")
        want, problems = render(x86_t)
        for p in problems:
            bad(p)
        if not problems:
            if want == arm_t:
                ok(f"{ARM} is {X86} with exactly the {len(REPLACEMENTS)} replacements and nothing else")
            else:
                bad(f"{ARM} is not {X86} with exactly the replacements; first differences (expected -, found +):")
                for ln in list(difflib.unified_diff(want.splitlines(), arm_t.splitlines(), "expected", ARM, lineterm="", n=0))[:14]:
                    bad("      " + ln[:200])
    if xi_b is not None and ai_b is not None:
        if xi_b == ai_b:
            ok(f"{ARM_IGN} is byte-identical to {X86_IGN} (the build context is the same)")
        else:
            bad(f"{ARM_IGN} differs from {X86_IGN}: the arm64 build would see another context")
    return out


def main(argv):
    args = argv[1:]
    printing = False
    if args and args[0] == "--print":
        printing, args = True, args[1:]
    if len(args) > 1:
        print(__doc__.split("usage:")[1].strip().splitlines()[0], file=sys.stderr)
        return 64
    d = args[0] if args else os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "deploy")
    if printing:
        b, why = read_regular(os.path.join(d, X86))
        if why:
            print(f"{X86} {why}", file=sys.stderr)
            return 1
        text, problems = render(b.decode("utf-8"))
        if problems:
            for p in problems:
                print(p, file=sys.stderr)
            return 1
        sys.stdout.write(text)
        return 0
    results = check(d)
    for good, text in results:
        print(("ok    " if good else "FAIL  ") + text)
    fails = sum(1 for good, _ in results if not good)
    if fails:
        print(f"containerfile-arm64-check: FAILED ({fails} problem line(s))")
        return 1
    print(f"containerfile-arm64-check: OK ({len(results)} checks)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
