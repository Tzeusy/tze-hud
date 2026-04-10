#!/usr/bin/env python3
"""Compare historical benchmark rows and flag regressions."""

from __future__ import annotations

import argparse
import csv
import datetime as dt
import json
import os
from typing import Any


_SCRIPT_DIR = os.path.dirname(os.path.abspath(__file__))
_REFERENCE_DIR = os.path.normpath(os.path.join(_SCRIPT_DIR, "..", "reference"))
DEFAULT_RESULTS_CSV = os.path.join(_REFERENCE_DIR, "results.csv")


def parse_iso_utc(value: str) -> dt.datetime:
    text = (value or "").strip()
    if not text:
        return dt.datetime.fromtimestamp(0, tz=dt.timezone.utc)
    normalized = text.replace("Z", "+00:00")
    parsed = dt.datetime.fromisoformat(normalized)
    if parsed.tzinfo is None:
        parsed = parsed.replace(tzinfo=dt.timezone.utc)
    return parsed.astimezone(dt.timezone.utc)


def as_float(value: str | None) -> float | None:
    if value is None:
        return None
    text = str(value).strip()
    if text == "":
        return None
    try:
        return float(text)
    except ValueError:
        return None


def load_rows(csv_path: str) -> list[dict[str, Any]]:
    if not os.path.exists(csv_path):
        raise FileNotFoundError(f"results csv not found: {csv_path}")
    with open(csv_path, "r", encoding="utf-8", newline="") as f:
        reader = csv.DictReader(f)
        rows = [dict(row) for row in reader]

    for row in rows:
        row["_recorded_at"] = parse_iso_utc(row.get("recorded_at_utc", ""))
    rows.sort(key=lambda r: r["_recorded_at"])
    return rows


def apply_filter(rows: list[dict[str, Any]], args: argparse.Namespace) -> list[dict[str, Any]]:
    filtered = rows
    if args.primary_key:
        filtered = [r for r in filtered if (r.get("primary_key", "") == args.primary_key)]
    if args.benchmark_name:
        filtered = [r for r in filtered if (r.get("benchmark_name", "") == args.benchmark_name)]
    if args.target_id:
        filtered = [r for r in filtered if (r.get("target_id", "") == args.target_id)]
    if args.transport:
        filtered = [r for r in filtered if (r.get("transport", "") == args.transport)]
    if args.mode:
        filtered = [r for r in filtered if (r.get("mode", "") == args.mode)]
    return filtered


def choose_candidate(rows: list[dict[str, Any]], args: argparse.Namespace) -> dict[str, Any]:
    if args.candidate_recorded_at:
        ts = parse_iso_utc(args.candidate_recorded_at)
        for row in rows:
            if row["_recorded_at"] == ts:
                return row
        raise ValueError(f"candidate row not found for timestamp {args.candidate_recorded_at}")
    return rows[-1]


def choose_baseline(
    rows: list[dict[str, Any]],
    candidate: dict[str, Any],
    args: argparse.Namespace,
) -> dict[str, Any]:
    if args.baseline_recorded_at:
        ts = parse_iso_utc(args.baseline_recorded_at)
        for row in rows:
            if row["_recorded_at"] == ts:
                return row
        raise ValueError(f"baseline row not found for timestamp {args.baseline_recorded_at}")

    candidate_ts = candidate["_recorded_at"]
    pool = [r for r in rows if r["_recorded_at"] < candidate_ts]

    if args.baseline_primary_key:
        pool = [r for r in pool if r.get("primary_key", "") == args.baseline_primary_key]
    elif candidate.get("primary_key"):
        same_pk = [r for r in pool if r.get("primary_key", "") == candidate.get("primary_key")]
        if same_pk:
            pool = same_pk
        else:
            pool = [
                r
                for r in pool
                if r.get("benchmark_name", "") == candidate.get("benchmark_name", "")
                and r.get("target_id", "") == candidate.get("target_id", "")
                and r.get("transport", "") == candidate.get("transport", "")
                and r.get("mode", "") == candidate.get("mode", "")
            ]

    if not pool:
        raise ValueError("no baseline row found prior to candidate")
    return pool[-1]


def metric_delta(candidate_val: float | None, baseline_val: float | None, prefer: str) -> dict[str, Any]:
    if candidate_val is None or baseline_val is None:
        return {"candidate": candidate_val, "baseline": baseline_val, "delta": None, "delta_pct": None, "status": "n/a"}

    delta = candidate_val - baseline_val
    delta_pct = (delta / baseline_val * 100.0) if baseline_val != 0 else None
    if abs(delta) < 1e-9:
        status = "no_change"
    elif prefer == "lower":
        status = "improved" if delta < 0 else "regressed"
    else:
        status = "improved" if delta > 0 else "regressed"
    return {
        "candidate": round(candidate_val, 4),
        "baseline": round(baseline_val, 4),
        "delta": round(delta, 4),
        "delta_pct": round(delta_pct, 4) if delta_pct is not None else None,
        "status": status,
    }


def evaluate_thresholds(row: dict[str, Any]) -> list[dict[str, Any]]:
    checks: list[dict[str, Any]] = []
    e2e = as_float(row.get("e2e_latency_ms"))
    p95 = as_float(row.get("p95_ms"))
    p99 = as_float(row.get("p99_ms"))
    throughput = as_float(row.get("throughput_rps"))
    error_count = as_float(row.get("error_count"))
    count = as_float(row.get("count"))
    error_rate = (error_count / count) if (error_count is not None and count and count > 0) else None

    def append_max(name: str, actual: float | None, expected: float | None) -> None:
        if expected is None:
            return
        checks.append(
            {
                "name": name,
                "actual": round(actual, 4) if actual is not None else None,
                "expected": expected,
                "type": "max",
                "status": "pass" if (actual is not None and actual <= expected) else "fail",
            }
        )

    def append_min(name: str, actual: float | None, expected: float | None) -> None:
        if expected is None:
            return
        checks.append(
            {
                "name": name,
                "actual": round(actual, 4) if actual is not None else None,
                "expected": expected,
                "type": "min",
                "status": "pass" if (actual is not None and actual >= expected) else "fail",
            }
        )

    append_max("e2e_latency_ms", e2e, as_float(row.get("expected_e2e_ms_max")))
    append_max("p95_ms", p95, as_float(row.get("expected_p95_ms_max")))
    append_max("p99_ms", p99, as_float(row.get("expected_p99_ms_max")))
    append_min("throughput_rps", throughput, as_float(row.get("expected_throughput_rps_min")))
    append_max("error_rate", error_rate, as_float(row.get("expected_error_rate_max")))
    return checks


def main() -> int:
    parser = argparse.ArgumentParser(description="Compare benchmark rows and highlight regressions")
    parser.add_argument("--results-csv", default=DEFAULT_RESULTS_CSV, help="Results CSV path")
    parser.add_argument("--primary-key", default="", help="Filter rows by primary key")
    parser.add_argument("--benchmark-name", default="", help="Filter rows by benchmark name")
    parser.add_argument("--target-id", default="", help="Filter rows by target id")
    parser.add_argument("--transport", default="", help="Filter rows by transport")
    parser.add_argument("--mode", default="", help="Filter rows by mode")
    parser.add_argument("--candidate-recorded-at", default="", help="Candidate row timestamp (UTC ISO8601)")
    parser.add_argument("--baseline-recorded-at", default="", help="Baseline row timestamp (UTC ISO8601)")
    parser.add_argument("--baseline-primary-key", default="", help="Force baseline primary key")
    parser.add_argument("--fail-on-regression", action="store_true", help="Exit non-zero when key metrics regress")
    args = parser.parse_args()

    rows = load_rows(args.results_csv)
    rows = apply_filter(rows, args)
    if len(rows) < 2:
        raise SystemExit("need at least two matching rows to compare")

    candidate = choose_candidate(rows, args)
    baseline = choose_baseline(rows, candidate, args)

    metric_config = {
        "e2e_latency_ms": "lower",
        "p95_ms": "lower",
        "p99_ms": "lower",
        "throughput_rps": "higher",
        "send_rps": "higher",
        "end_to_end_rps": "higher",
        "error_count": "lower",
        "success_count": "higher",
        "bytes_out": "lower",
        "bytes_in": "lower",
    }
    metrics: dict[str, Any] = {}
    regressions: list[str] = []
    for key, prefer in metric_config.items():
        diff = metric_delta(as_float(candidate.get(key)), as_float(baseline.get(key)), prefer)
        metrics[key] = diff
        if diff["status"] == "regressed":
            regressions.append(key)

    threshold_checks = evaluate_thresholds(candidate)
    threshold_failures = [c["name"] for c in threshold_checks if c["status"] == "fail"]

    output = {
        "candidate": {
            "recorded_at_utc": candidate.get("recorded_at_utc"),
            "benchmark_name": candidate.get("benchmark_name"),
            "primary_key": candidate.get("primary_key"),
            "target_id": candidate.get("target_id"),
            "transport": candidate.get("transport"),
            "mode": candidate.get("mode"),
        },
        "baseline": {
            "recorded_at_utc": baseline.get("recorded_at_utc"),
            "benchmark_name": baseline.get("benchmark_name"),
            "primary_key": baseline.get("primary_key"),
            "target_id": baseline.get("target_id"),
            "transport": baseline.get("transport"),
            "mode": baseline.get("mode"),
        },
        "metrics": metrics,
        "threshold_checks": threshold_checks,
        "summary": {
            "regressions": regressions,
            "threshold_failures": threshold_failures,
        },
    }

    print(json.dumps(output, ensure_ascii=True, indent=2))

    if threshold_failures:
        return 2
    if args.fail_on_regression and regressions:
        return 3
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
