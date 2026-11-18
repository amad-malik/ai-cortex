#!/usr/bin/env python3
"""Mutation checks for the W1-284 skip and signed-receipt gates."""
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MUTANTS = (
    ("unknown skip", "scripts/release-ci-check.py", "if permitted.get(gate) != reason:", "if gate in permitted and permitted.get(gate) != reason:", "test_ci_policy_022.py"),
    ("changed reason", "scripts/release-ci-check.py", "if permitted.get(gate) != reason:", "if gate not in permitted:", "test_ci_policy_022.py"),
    ("second Mac skip", "scripts/release-ci-check.py", "if permitted.get(gate) != reason:", "if permitted.get(gate) != reason and gate != 'install-mac-test':", "test_ci_policy_022.py"),
    ("zero exit with skips", "scripts/mac-presign-check.py", "rc != 77 or any", "rc not in (0, 77) or any", "test_mac_presign.py"),
    ("extra skipped cell", "scripts/mac-presign-check.py", "len(skips) != 1 or not skips[0]", "len(skips) < 1 or not skips[-1]", "test_mac_presign.py"),
    ("omitted required cell", "scripts/mac-presign-check.py", "or set(observed) != set(required) - {\"IT-M29\"}", "or False", "test_mac_presign.py"),
    ("inherited suite switch", "scripts/mac-presign-check.py", "if not k.startswith((\"INSTALL_MAC_\", \"MAC_SET_\"))", "if not k.startswith((\"MAC_SET_\",))", "test_mac_presign.py"),
    ("missing receipt", "scripts/mac-publish-core.sh", 'python3 -I -B "$helper" "$set_dir" "$key" "$version" "$receipt" "$receipt_sha" || { echo \'mac-publish: signed-set receipt gate failed\' >&2; exit 1; }', 'true', "test_mac_publish_core.py"),
    ("receipt from another set", "scripts/mac-postsign-check.py", "if not re.fullmatch(pattern, text):", "if False:", "test_mac_postsign.py"),
)
FILES = ("scripts/ci.sh", "scripts/release-ci-check.py", "scripts/release-ci-allowlist.json", "scripts/mac-publish-core.sh", "scripts/mac-postsign-check.py", "scripts/mac-presign-check.py", "scripts/release-fixtures/test_ci_policy_022.py", *("scripts/mac-release-fixtures/" + n for n in ("test_mac_publish_core.py", "test_mac_postsign.py", "test_mac_presign.py")))


def run_test(root, name):
    folder = "release-fixtures" if name == "test_ci_policy_022.py" else "mac-release-fixtures"
    return subprocess.run([sys.executable, "-I", "-B", str(root / "scripts" / folder / name)], stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=120)


def main():
    with tempfile.TemporaryDirectory(prefix="w1284-mutants-") as temp:
        root = Path(temp)
        for name in FILES:
            target = root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, target)
        for test in {item[-1] for item in MUTANTS}:
            result = run_test(root, test)
            if result.returncode != 0:
                print(f"baseline FAIL {test}: {result.stdout[-1000:]}")
                return 1
        for label, file, old, new, test in MUTANTS:
            path = root / file
            original = path.read_text()
            if original.count(old) != 1:
                print(f"FAIL {label}: mutation is not unique")
                return 1
            path.write_text(original.replace(old, new, 1))
            result = run_test(root, test)
            path.write_text(original)
            if result.returncode == 0:
                print(f"FAIL {label}: mutant survived")
                return 1
            print(f"ok {label}: mutant caught")
    print(f"W1-284 mutants: {len(MUTANTS)} caught, 0 survived")
    return 0


if __name__ == "__main__":
    sys.exit(main())
