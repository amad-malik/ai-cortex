"""K31 (r7, mike's recogniser finding): run THIS revision's recogniser fixtures against the shell
recogniser of a given copy of signal_sites.py (this revision's, or an older one for a RED-first run).

Standard library only; run as `python3 -I`. The module under test is loaded from its path and only
its `shell_sites(path)` function is called, on a temporary file of fixture lines.

Usage: python3 -I recogniser_fixtures.py SIGNAL_SITES_PY
"""

import importlib.util
import os
import sys

sys.dont_write_bytecode = True
HERE = os.path.dirname(os.path.abspath(__file__))


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def main():
    ours = load("signal_sites_ours", os.path.join(HERE, "signal_sites.py"))
    under_test = load("signal_sites_under_test", sys.argv[1])
    problems = ours.fixtures(under_test.shell_sites)
    print(("PASS" if not problems else "FAIL") + f": {len(ours.SHELL_FIXTURES)} recogniser fixtures on "
          f"{sys.argv[1]}" + ("; " + "; ".join(problems) if problems else ""))
    return 0 if not problems else 1


if __name__ == "__main__":
    sys.exit(main())
