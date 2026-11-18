#!/usr/bin/env python3
"""W1-109 must not precede existing locale cells in the full suite."""
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[3]
SUITE = ROOT / 'scripts/install-prereq-test.sh'

# RED on f7c1e22, actual C-first diagnostic receipts:
# IPM-04: FAIL W1-109: machine_look machine_ok status=2 expected=0 or caller traps changed
# IPM-05: FAIL W1-109: machine_look machine_root status=0 expected=2 or caller traps changed
# IPM-06: FAIL W1-109: machine_look machine_ok status=2 expected=0 or caller traps changed

class CallerTrapPlacement(unittest.TestCase):
    def test_cell_after_existing_machine_checks_in_each_locale(self):
        suite = SUITE.read_text()
        call = 'LC_ALL="$test_locale" TMPDIR=$scratch sh "$root/scripts/fakes/prereq/caller-trap-cell.sh" "$original_include" "$root"'
        self.assertEqual(suite.count(call), 1, 'caller trap cell must run once in each locale loop')
        loop = suite.index('for test_locale in ${PREREQ_TEST_LOCALES:-C C.utf8}; do')
        old_cell = suite.index('  prereq_test_machine_cases\n', loop)
        done = suite.index('  case $test_locale in\n', old_cell)
        self.assertLess(old_cell, suite.index(call))
        self.assertLess(suite.index(call), done)

    def test_trap_mutants_run_after_all_locale_cells(self):
        suite = SUITE.read_text()
        call = 'TMPDIR=$scratch python3 -I -B "$root/scripts/fakes/prereq/caller-trap-mutant-test.py"'
        self.assertEqual(suite.count(call), 1)
        end = suite.index('if [ "$pq_test_completed_c" -ne 1 ]')
        self.assertGreater(suite.index(call), end)
        self.assertLess(suite.index(call), suite.index("'SKIP IP-64:"))

if __name__ == '__main__':
    unittest.main()
