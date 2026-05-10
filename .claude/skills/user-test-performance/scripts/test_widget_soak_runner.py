#!/usr/bin/env python3
"""Focused tests for widget_soak_runner live-metrics artifact handling."""

from __future__ import annotations

import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).with_name("widget_soak_runner.py")
SPEC = importlib.util.spec_from_file_location("widget_soak_runner", SCRIPT)
assert SPEC is not None
widget_soak_runner = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(widget_soak_runner)


def write_json(path: Path, payload: dict) -> None:
    path.write_text(json.dumps(payload), encoding="utf-8")


class LiveMetricsArtifactTests(unittest.TestCase):
    def test_windowed_artifact_extracts_frame_and_input_percentiles(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "windowed.json"
            write_json(
                path,
                {
                    "schema": "tze_hud.windowed_compositor_benchmark.v1",
                    "benchmark": {"recorded_frames": 4},
                    "frame_time": {
                        "p50_us": 20,
                        "p99_us": 40,
                        "p99_9_us": 40,
                        "peak_us": 40,
                    },
                    "summary": {
                        "frame_time": {"samples": [10, 20, 30, 40]},
                        "input_to_local_ack": {"samples": [1, 2, 3]},
                        "input_to_scene_commit": {"samples": [10, 20, 30]},
                        "input_to_next_present": {"samples": [16, 32, 48]},
                    },
                },
            )

            result = widget_soak_runner.load_live_metrics_artifact(path)

            self.assertTrue(result["ok"], result)
            self.assertEqual(result["frame_time"]["p50_us"], 20)
            self.assertEqual(result["frame_time"]["p99_us"], 40)
            self.assertEqual(result["frame_time"]["p99_9_us"], 40)
            self.assertEqual(result["input_latency"]["input_to_local_ack"]["p99_us"], 3)
            self.assertEqual(result["input_latency"]["input_to_scene_commit"]["p95_us"], 30)
            self.assertEqual(result["input_latency"]["input_to_next_present"]["p50_us"], 32)

    def test_missing_input_triple_is_explicit_failure(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "frame_only.json"
            write_json(
                path,
                {
                    "schema": "tze_hud.windowed_compositor_benchmark.v1",
                    "frame_time": {"p50_us": 20, "p99_us": 40, "p99_9_us": 40},
                    "summary": {"frame_time": {"samples": [10, 20, 30, 40]}},
                },
            )

            result = widget_soak_runner.load_live_metrics_artifact(path)

            self.assertFalse(result["ok"], result)
            self.assertIn("input_to_local_ack.p99_us", result["missing_metrics"])
            self.assertIn("input_to_scene_commit.sample_count", result["missing_metrics"])
            self.assertIn("input_to_next_present.p50_us", result["missing_metrics"])

    def test_headless_benchmark_sessions_are_aggregated(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "benchmark.json"
            write_json(
                path,
                {
                    "calibration": {"factors": {"cpu": 1.0, "gpu": 1.0, "upload": 1.0}},
                    "sessions": [
                        {
                            "name": "steady_state_render",
                            "summary": {
                                "frame_time": {"samples": [100, 200]},
                                "input_to_local_ack": {"samples": [1, 2]},
                                "input_to_scene_commit": {"samples": [11, 12]},
                                "input_to_next_present": {"samples": [21, 22]},
                            },
                        },
                        {
                            "name": "high_mutation",
                            "summary": {
                                "frame_time": {"samples": [300, 400]},
                                "input_to_local_ack": {"samples": [3, 4]},
                                "input_to_scene_commit": {"samples": [13, 14]},
                                "input_to_next_present": {"samples": [23, 24]},
                            },
                        },
                    ],
                },
            )

            result = widget_soak_runner.load_live_metrics_artifact(path)

            self.assertTrue(result["ok"], result)
            self.assertEqual(result["frame_time"]["sample_count"], 4)
            self.assertEqual(result["frame_time"]["p99_9_us"], 400)
            self.assertEqual(result["input_latency"]["input_to_next_present"]["p99_us"], 24)
            self.assertEqual(result["sessions"], ["steady_state_render", "high_mutation"])


if __name__ == "__main__":
    unittest.main()
