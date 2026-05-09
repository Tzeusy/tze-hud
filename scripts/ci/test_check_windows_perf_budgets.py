#!/usr/bin/env python3
from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path


MODULE_PATH = Path(__file__).with_name("check_windows_perf_budgets.py")
SPEC = importlib.util.spec_from_file_location("check_windows_perf_budgets", MODULE_PATH)
assert SPEC is not None
check_windows_perf_budgets = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
sys.modules[SPEC.name] = check_windows_perf_budgets
SPEC.loader.exec_module(check_windows_perf_budgets)


class WindowsPerfBudgetCheckerTests(unittest.TestCase):
    def test_load_json_reports_missing_file_without_traceback(self) -> None:
        missing = Path(tempfile.gettempdir()) / "tze_hud_missing_benchmark.json"

        with self.assertRaises(SystemExit) as raised:
            check_windows_perf_budgets.load_json(missing)

        self.assertIn("unable to read", str(raised.exception))

    def test_hardware_factors_rejects_malformed_calibration(self) -> None:
        with self.assertRaises(SystemExit) as raised:
            check_windows_perf_budgets.hardware_factors({"calibration": []})

        self.assertIn("'calibration' must be an object", str(raised.exception))

    def test_samples_rejects_malformed_bucket(self) -> None:
        with self.assertRaises(ValueError) as raised:
            check_windows_perf_budgets.samples({"frame_time": []}, "frame_time")

        self.assertEqual("frame_time: expected object", str(raised.exception))


if __name__ == "__main__":
    unittest.main()
