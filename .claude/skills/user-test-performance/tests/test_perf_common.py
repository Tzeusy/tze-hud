from __future__ import annotations

import csv
import json
import tempfile
import unittest
from pathlib import Path

import sys

SCRIPT_DIR = Path(__file__).resolve().parents[1] / "scripts"
sys.path.insert(0, str(SCRIPT_DIR))

import perf_common  # noqa: E402


class PerfCommonTests(unittest.TestCase):
    def test_migrate_legacy_header_maps_aliases(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            csv_path = Path(td) / "results.csv"
            with csv_path.open("w", newline="", encoding="utf-8") as handle:
                writer = csv.DictWriter(handle, fieldnames=["timestamp", "benchmark_id", "throughput_rps"])
                writer.writeheader()
                writer.writerow(
                    {
                        "timestamp": "2026-04-10T00:00:00Z",
                        "benchmark_id": "k1",
                        "throughput_rps": "123.45",
                    }
                )

            perf_common.migrate_results_csv(csv_path)

            with csv_path.open("r", newline="", encoding="utf-8") as handle:
                reader = csv.DictReader(handle)
                self.assertEqual(reader.fieldnames, perf_common.RESULTS_CSV_COLUMNS)
                rows = list(reader)

            self.assertEqual(rows[0]["timestamp_utc"], "2026-04-10T00:00:00Z")
            self.assertEqual(rows[0]["benchmark_key"], "k1")
            self.assertEqual(rows[0]["throughput_rps"], "123.45")

    def test_append_artifact_and_find_previous(self) -> None:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            csv_path = root / "results.csv"
            base = {
                "benchmark_key": "bench-1",
                "identity": {
                    "target_id": "user-test-windows-tailnet",
                    "target_host": "host",
                    "network_scope": "tailnet",
                    "transport": "grpc",
                    "mode": "burst",
                    "widget_name": "gauge",
                    "payload_profile": "p",
                    "publish_count": 100,
                    "duration_s": None,
                    "target_rate_rps": None,
                },
                "metrics": {
                    "request_count": 100,
                    "success_count": 100,
                    "error_count": 0,
                    "wall_duration_us": 1000,
                    "throughput_rps": 100.0,
                    "rtt_p50_us": 10,
                    "rtt_p95_us": 20,
                    "rtt_p99_us": 30,
                    "rtt_max_us": 40,
                    "aggregate_send_time_us": 100,
                    "aggregate_ack_drain_time_us": 200,
                    "payload_bytes_out": 1000,
                    "payload_bytes_in": 1200,
                    "wire_bytes_out": None,
                    "wire_bytes_in": None,
                },
                "byte_accounting_mode": "payload_only",
                "thresholds": {"target_p99_rtt_us": None, "target_throughput_rps": None},
                "traceability": {"spec_id": "s", "rfc_id": "r", "budget_id": None, "threshold_id": None},
                "calibration_status": "uncalibrated",
                "verdict": "uncalibrated",
                "threshold_comparisons_informational": True,
                "warnings": [],
            }

            artifact_a = root / "a.json"
            artifact_b = root / "b.json"

            payload_a = dict(base)
            payload_a["timestamp_utc"] = "2026-04-10T00:00:00Z"
            payload_b = dict(base)
            payload_b["timestamp_utc"] = "2026-04-10T00:01:00Z"
            payload_b["metrics"] = dict(base["metrics"])
            payload_b["metrics"]["throughput_rps"] = 150.0

            artifact_a.write_text(json.dumps(payload_a), encoding="utf-8")
            artifact_b.write_text(json.dumps(payload_b), encoding="utf-8")

            row_a = perf_common.append_artifact(csv_path, artifact_a)
            row_b = perf_common.append_artifact(csv_path, artifact_b)

            previous = perf_common.find_latest_by_benchmark_key(
                csv_path,
                row_b["benchmark_key"],
                exclude_artifact_path=row_b["artifact_path"],
            )

            self.assertIsNotNone(previous)
            assert previous is not None
            self.assertEqual(previous["artifact_path"], row_a["artifact_path"])


if __name__ == "__main__":
    unittest.main()
