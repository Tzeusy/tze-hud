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

    # ── scene_lock_misses / invariant_violations zero-baseline gate ─────────

    @staticmethod
    def _artifact_with_summary(summary_overrides: dict) -> dict:
        """Build a minimal valid benchmark artifact for both required sessions.

        Latency buckets carry one well-under-budget sample so the percentile
        checks pass; counter values default to their healthy baseline (0) and
        are merged with ``summary_overrides``.
        """
        bucket = {"samples": [1]}
        base_summary = {
            "frame_time": bucket,
            "input_to_local_ack": bucket,
            "input_to_scene_commit": bucket,
            "input_to_next_present": bucket,
            "lease_violations": 0,
            "budget_overruns": 0,
            "sync_drift_violations": 0,
            "invariant_violations": 0,
            "scene_lock_misses": 0,
        }
        base_summary.update(summary_overrides)
        return {
            "calibration": {"factors": {"cpu": 0.854, "gpu": 0.338, "upload": 0.215}},
            "sessions": [
                {"name": name, "summary": dict(base_summary)}
                for name in check_windows_perf_budgets.REQUIRED_SESSIONS
            ],
        }

    def test_scene_lock_misses_in_zero_counters(self) -> None:
        self.assertIn("scene_lock_misses", check_windows_perf_budgets.ZERO_COUNTERS)
        self.assertIn("invariant_violations", check_windows_perf_budgets.ZERO_COUNTERS)

    def test_healthy_counters_pass(self) -> None:
        artifact = self._artifact_with_summary({})
        _results, failures = check_windows_perf_budgets.validate_benchmark(artifact)
        self.assertEqual(failures, [])

    def test_nonzero_scene_lock_misses_fails(self) -> None:
        artifact = self._artifact_with_summary({"scene_lock_misses": 3})
        _results, failures = check_windows_perf_budgets.validate_benchmark(artifact)
        self.assertTrue(
            any("scene_lock_misses" in f for f in failures),
            f"expected a scene_lock_misses failure, got {failures!r}",
        )

    def test_nonzero_invariant_violations_fails(self) -> None:
        artifact = self._artifact_with_summary({"invariant_violations": 1})
        _results, failures = check_windows_perf_budgets.validate_benchmark(artifact)
        self.assertTrue(
            any("invariant_violations" in f for f in failures),
            f"expected an invariant_violations failure, got {failures!r}",
        )

    def test_missing_scene_lock_misses_field_fails(self) -> None:
        # A summary that omits the counter entirely must not silently pass.
        artifact = self._artifact_with_summary({})
        for session in artifact["sessions"]:
            del session["summary"]["scene_lock_misses"]
        _results, failures = check_windows_perf_budgets.validate_benchmark(artifact)
        self.assertTrue(
            any("scene_lock_misses" in f for f in failures),
            f"expected a scene_lock_misses failure for missing field, got {failures!r}",
        )

    def test_baseline_counter_ceiling_enforced(self) -> None:
        # Patch a non-zero ceiling in to exercise the BASELINE_COUNTERS path.
        original = dict(check_windows_perf_budgets.BASELINE_COUNTERS)
        check_windows_perf_budgets.BASELINE_COUNTERS["scene_lock_misses"] = 2
        try:
            # scene_lock_misses is also in ZERO_COUNTERS; remove it there so the
            # ceiling path is what we observe for this targeted unit test.
            zero = check_windows_perf_budgets.ZERO_COUNTERS
            check_windows_perf_budgets.ZERO_COUNTERS = tuple(
                c for c in zero if c != "scene_lock_misses"
            )
            under = self._artifact_with_summary({"scene_lock_misses": 2})
            _r, failures_under = check_windows_perf_budgets.validate_benchmark(under)
            self.assertEqual(failures_under, [])

            over = self._artifact_with_summary({"scene_lock_misses": 3})
            _r, failures_over = check_windows_perf_budgets.validate_benchmark(over)
            self.assertTrue(any("scene_lock_misses" in f for f in failures_over))
        finally:
            check_windows_perf_budgets.BASELINE_COUNTERS.clear()
            check_windows_perf_budgets.BASELINE_COUNTERS.update(original)
            check_windows_perf_budgets.ZERO_COUNTERS = zero


if __name__ == "__main__":
    unittest.main()
