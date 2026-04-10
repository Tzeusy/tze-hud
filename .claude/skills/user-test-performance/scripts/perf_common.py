#!/usr/bin/env python3
"""Common utilities for /user-test-performance benchmark workflows."""

from __future__ import annotations

import csv
import json
from pathlib import Path
from typing import Any, Dict, Iterable, Optional

RESULTS_CSV_COLUMNS = [
    "timestamp_utc",
    "benchmark_key",
    "target_id",
    "target_host",
    "network_scope",
    "transport",
    "mode",
    "widget_name",
    "payload_profile",
    "publish_count",
    "duration_s",
    "target_rate_rps",
    "request_count",
    "success_count",
    "error_count",
    "wall_duration_us",
    "throughput_rps",
    "rtt_p50_us",
    "rtt_p95_us",
    "rtt_p99_us",
    "rtt_max_us",
    "aggregate_send_time_us",
    "aggregate_ack_drain_time_us",
    "payload_bytes_out",
    "payload_bytes_in",
    "wire_bytes_out",
    "wire_bytes_in",
    "byte_accounting_mode",
    "calibration_status",
    "verdict",
    "threshold_comparisons_informational",
    "target_p99_rtt_us",
    "target_throughput_rps",
    "spec_id",
    "rfc_id",
    "budget_id",
    "threshold_id",
    "warnings",
    "artifact_path",
]

LEGACY_ALIAS_MAP = {
    "timestamp": "timestamp_utc",
    "benchmark_id": "benchmark_key",
    "p50_rtt_us": "rtt_p50_us",
    "p95_rtt_us": "rtt_p95_us",
    "p99_rtt_us": "rtt_p99_us",
}


def _string(value: Any) -> str:
    if value is None:
        return ""
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, (dict, list)):
        return json.dumps(value, separators=(",", ":"), sort_keys=True)
    return str(value)


def load_publish_artifact(path: Path) -> Dict[str, Any]:
    with path.open("r", encoding="utf-8") as handle:
        data = json.load(handle)
    if not isinstance(data, dict):
        raise ValueError(f"artifact must be a JSON object: {path}")
    return data


def artifact_to_row(artifact: Dict[str, Any], artifact_path: Path) -> Dict[str, str]:
    identity = artifact.get("identity", {})
    metrics = artifact.get("metrics", {})
    thresholds = artifact.get("thresholds", {})
    traceability = artifact.get("traceability", {})

    row = {
        "timestamp_utc": _string(artifact.get("timestamp_utc") or artifact.get("run_started_at_utc")),
        "benchmark_key": _string(artifact.get("benchmark_key")),
        "target_id": _string(identity.get("target_id")),
        "target_host": _string(identity.get("target_host")),
        "network_scope": _string(identity.get("network_scope")),
        "transport": _string(identity.get("transport")),
        "mode": _string(identity.get("mode")),
        "widget_name": _string(identity.get("widget_name")),
        "payload_profile": _string(identity.get("payload_profile")),
        "publish_count": _string(identity.get("publish_count")),
        "duration_s": _string(identity.get("duration_s")),
        "target_rate_rps": _string(identity.get("target_rate_rps")),
        "request_count": _string(metrics.get("request_count")),
        "success_count": _string(metrics.get("success_count")),
        "error_count": _string(metrics.get("error_count")),
        "wall_duration_us": _string(metrics.get("wall_duration_us")),
        "throughput_rps": _string(metrics.get("throughput_rps")),
        "rtt_p50_us": _string(metrics.get("rtt_p50_us")),
        "rtt_p95_us": _string(metrics.get("rtt_p95_us")),
        "rtt_p99_us": _string(metrics.get("rtt_p99_us")),
        "rtt_max_us": _string(metrics.get("rtt_max_us")),
        "aggregate_send_time_us": _string(metrics.get("aggregate_send_time_us")),
        "aggregate_ack_drain_time_us": _string(metrics.get("aggregate_ack_drain_time_us")),
        "payload_bytes_out": _string(metrics.get("payload_bytes_out")),
        "payload_bytes_in": _string(metrics.get("payload_bytes_in")),
        "wire_bytes_out": _string(metrics.get("wire_bytes_out")),
        "wire_bytes_in": _string(metrics.get("wire_bytes_in")),
        "byte_accounting_mode": _string(artifact.get("byte_accounting_mode")),
        "calibration_status": _string(artifact.get("calibration_status")),
        "verdict": _string(artifact.get("verdict")),
        "threshold_comparisons_informational": _string(
            artifact.get("threshold_comparisons_informational")
        ),
        "target_p99_rtt_us": _string(thresholds.get("target_p99_rtt_us")),
        "target_throughput_rps": _string(thresholds.get("target_throughput_rps")),
        "spec_id": _string(traceability.get("spec_id")),
        "rfc_id": _string(traceability.get("rfc_id")),
        "budget_id": _string(traceability.get("budget_id")),
        "threshold_id": _string(traceability.get("threshold_id")),
        "warnings": _string(artifact.get("warnings", [])),
        "artifact_path": str(artifact_path),
    }
    return row


def _normalize_row_keys(row: Dict[str, str]) -> Dict[str, str]:
    normalized: Dict[str, str] = {}
    for key, value in row.items():
        normalized[LEGACY_ALIAS_MAP.get(key, key)] = value
    return normalized


def migrate_results_csv(csv_path: Path) -> None:
    if not csv_path.exists():
        csv_path.parent.mkdir(parents=True, exist_ok=True)
        with csv_path.open("w", newline="", encoding="utf-8") as handle:
            writer = csv.DictWriter(handle, fieldnames=RESULTS_CSV_COLUMNS)
            writer.writeheader()
        return

    with csv_path.open("r", newline="", encoding="utf-8") as handle:
        reader = csv.DictReader(handle)
        existing_header = reader.fieldnames or []
        if existing_header == RESULTS_CSV_COLUMNS:
            return
        existing_rows = list(reader)

    normalized_rows = [_normalize_row_keys(r) for r in existing_rows]

    with csv_path.open("w", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(handle, fieldnames=RESULTS_CSV_COLUMNS)
        writer.writeheader()
        for row in normalized_rows:
            writer.writerow({column: row.get(column, "") for column in RESULTS_CSV_COLUMNS})


def append_result_row(csv_path: Path, row: Dict[str, str]) -> None:
    migrate_results_csv(csv_path)
    csv_path.parent.mkdir(parents=True, exist_ok=True)
    with csv_path.open("a", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(handle, fieldnames=RESULTS_CSV_COLUMNS)
        writer.writerow({column: row.get(column, "") for column in RESULTS_CSV_COLUMNS})


def append_artifact(csv_path: Path, artifact_path: Path) -> Dict[str, str]:
    artifact = load_publish_artifact(artifact_path)
    row = artifact_to_row(artifact, artifact_path)
    append_result_row(csv_path, row)
    return row


def _iter_rows(csv_path: Path) -> Iterable[Dict[str, str]]:
    if not csv_path.exists():
        return []
    with csv_path.open("r", newline="", encoding="utf-8") as handle:
        reader = csv.DictReader(handle)
        rows = [_normalize_row_keys(r) for r in reader]
    return rows


def find_latest_by_benchmark_key(
    csv_path: Path, benchmark_key: str, *, exclude_artifact_path: Optional[str] = None
) -> Optional[Dict[str, str]]:
    latest: Optional[Dict[str, str]] = None
    for row in _iter_rows(csv_path):
        if row.get("benchmark_key") != benchmark_key:
            continue
        if exclude_artifact_path and row.get("artifact_path") == exclude_artifact_path:
            continue
        if latest is None or row.get("timestamp_utc", "") >= latest.get("timestamp_utc", ""):
            latest = row
    return latest
