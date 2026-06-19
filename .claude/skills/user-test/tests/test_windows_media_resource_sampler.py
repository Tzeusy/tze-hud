from __future__ import annotations

import sys
import unittest
from pathlib import Path


SCRIPT_DIR = Path(__file__).resolve().parents[1] / "scripts"
sys.path.insert(0, str(SCRIPT_DIR))

import windows_media_resource_sampler as sampler  # noqa: E402


class WindowsMediaResourceSamplerTests(unittest.TestCase):
    def test_remote_script_avoids_param_block_regression(self) -> None:
        script = sampler.build_remote_sample_script(grpc_port=50052, samples=2, interval_s=1)

        self.assertNotIn("param(", script)
        self.assertIn("$GrpcPort = 50052", script)
        self.assertIn("$Samples = 2", script)
        self.assertIn("$IntervalSeconds = 1", script)
        self.assertIn("ConvertTo-Json", script)

    def test_summarize_samples_reports_cpu_gpu_and_memory(self) -> None:
        raw = {
            "samples": [
                {
                    "elapsed_s": 0.0,
                    "listener_pid": 1234,
                    "cpu_seconds": 10.0,
                    "working_set_bytes": 1000,
                    "private_memory_bytes": 2000,
                    "gpu_3d_utilization_pct_sum": 3.0,
                    "nvidia_gpu_utilization_pct": 12.0,
                    "nvidia_gpu_memory_used_mb": 500.0,
                },
                {
                    "elapsed_s": 10.0,
                    "listener_pid": 1234,
                    "cpu_seconds": 12.0,
                    "working_set_bytes": 1100,
                    "private_memory_bytes": 2600,
                    "gpu_3d_utilization_pct_sum": 5.0,
                    "nvidia_gpu_utilization_pct": 16.0,
                    "nvidia_gpu_memory_used_mb": 520.0,
                },
            ],
            "logical_processors": 4,
            "errors": [],
        }

        summary = sampler.summarize_samples(raw)

        self.assertEqual(summary["sample_count"], 2)
        self.assertEqual(summary["valid_sample_count"], 2)
        self.assertEqual(summary["private_memory_drift_bytes"], 600)
        self.assertEqual(summary["working_set_drift_bytes"], 100)
        self.assertEqual(summary["cpu_percent"]["avg"], 5.0)
        self.assertEqual(summary["gpu_3d_utilization_pct_sum"]["max"], 5.0)
        self.assertEqual(summary["nvidia_gpu_utilization_pct"]["avg"], 14.0)
        self.assertEqual(summary["nvidia_gpu_memory_used_mb"]["max"], 520.0)

    def test_summarize_samples_rejects_missing_process_as_invalid(self) -> None:
        raw = {
            "samples": [
                {
                    "elapsed_s": 0.0,
                    "listener_pid": None,
                    "cpu_seconds": None,
                    "working_set_bytes": None,
                    "private_memory_bytes": None,
                    "gpu_3d_utilization_pct_sum": None,
                }
            ],
            "logical_processors": 8,
            "errors": [],
        }

        summary = sampler.summarize_samples(raw)

        self.assertEqual(summary["sample_count"], 1)
        self.assertEqual(summary["valid_sample_count"], 0)
        self.assertIsNone(summary["private_memory_drift_bytes"])
        self.assertEqual(summary["cpu_percent"]["count"], 0)


if __name__ == "__main__":
    unittest.main()
