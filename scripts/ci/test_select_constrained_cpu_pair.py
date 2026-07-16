#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("select_constrained_cpu_pair.py")
SPEC = importlib.util.spec_from_file_location("select_constrained_cpu_pair", MODULE_PATH)
assert SPEC is not None
select_constrained_cpu_pair = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
sys.modules[SPEC.name] = select_constrained_cpu_pair
SPEC.loader.exec_module(select_constrained_cpu_pair)


class SelectConstrainedCpuPairTests(unittest.TestCase):
    def test_selects_exact_pair_from_nonzero_range(self) -> None:
        self.assertEqual(
            (8, 9), select_constrained_cpu_pair.select_cpu_pair("8-10,14")
        )

    def test_selects_across_sparse_singletons(self) -> None:
        self.assertEqual(
            (2, 7), select_constrained_cpu_pair.select_cpu_pair("2,7,11-12")
        )

    def test_rejects_fewer_than_two_cpus(self) -> None:
        with self.assertRaisesRegex(ValueError, "at least two"):
            select_constrained_cpu_pair.select_cpu_pair("5")

    def test_rejects_malformed_range(self) -> None:
        with self.assertRaisesRegex(ValueError, "invalid CPU-list range"):
            select_constrained_cpu_pair.select_cpu_pair("3-1")


if __name__ == "__main__":
    unittest.main()
