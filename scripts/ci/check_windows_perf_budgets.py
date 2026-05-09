#!/usr/bin/env python3
"""Validate Windows-first benchmark artifacts against locked budgets.

The input is the JSON emitted by:

    cargo run -p benchmark --features headless -- --emit <path>

Budgets are expressed in reference-hardware microseconds and normalized with
the calibration factors already present in the benchmark artifact.
"""

from __future__ import annotations

import argparse
import json
import math
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any


REFERENCE_HARDWARE_TAG = (
    "TzeHouse / Intel Core i5-13600KF / NVIDIA RTX 3080 driver 32.0.15.9636 / "
    "16 GiB RAM / Windows 11 Pro 10.0.26200 build 26200 / 4096x2160@60Hz"
)

# From docs/reports/windows_perf_baseline_2026-05.md, layer3_benchmark_600.json.
# The benchmark artifact factors are relative to the harness' internal reference
# constants, so Windows-locked TzeHouse budgets scale by current/reference factor.
REFERENCE_FACTORS = {
    "cpu": 0.854,
    "gpu": 0.338,
    "upload": 0.215,
}

REQUIRED_SESSIONS = ("steady_state_render", "high_mutation")


@dataclass(frozen=True)
class Budget:
    metric: str
    bucket: str
    percentile: float
    reference_budget_us: int
    factor: str


WINDOWS_HEADLESS_BUDGETS = (
    Budget("frame_time_p99", "frame_time", 99.0, 8_300, "gpu"),
    Budget("frame_time_p99_9", "frame_time", 99.9, 16_600, "gpu"),
    Budget("input_to_local_ack_p99", "input_to_local_ack", 99.0, 2_000, "cpu"),
    Budget(
        "input_to_scene_commit_p99",
        "input_to_scene_commit",
        99.0,
        25_000,
        "cpu",
    ),
    Budget(
        "input_to_next_present_p99",
        "input_to_next_present",
        99.0,
        16_600,
        "gpu",
    ),
)

ZERO_COUNTERS = ("lease_violations", "budget_overruns", "sync_drift_violations")


def nearest_rank(values: list[int], percentile: float) -> int:
    if not values:
        raise ValueError("no samples")
    sorted_values = sorted(values)
    rank = math.ceil((percentile / 100.0) * len(sorted_values))
    index = max(rank - 1, 0)
    return sorted_values[min(index, len(sorted_values) - 1)]


def load_json(path: Path) -> dict[str, Any]:
    try:
        with path.open("r", encoding="utf-8") as handle:
            payload = json.load(handle)
    except json.JSONDecodeError as exc:
        raise SystemExit(f"{path}: invalid JSON: {exc}") from exc
    if not isinstance(payload, dict):
        raise SystemExit(f"{path}: expected object root")
    return payload


def hardware_factors(artifact: dict[str, Any]) -> dict[str, float]:
    factors = artifact.get("calibration", {}).get("factors", {})
    missing = [
        key
        for key in ("cpu", "gpu", "upload")
        if not isinstance(factors.get(key), (int, float))
    ]
    if missing:
        raise SystemExit(
            "benchmark artifact is missing required calibration factors: "
            + ", ".join(missing)
        )
    return {key: float(factors[key]) for key in ("cpu", "gpu", "upload")}


def session_by_name(artifact: dict[str, Any]) -> dict[str, dict[str, Any]]:
    sessions = artifact.get("sessions", [])
    if not isinstance(sessions, list):
        raise SystemExit("benchmark artifact field 'sessions' must be a list")
    by_name = {}
    for session in sessions:
        name = session.get("name") if isinstance(session, dict) else None
        if isinstance(name, str):
            by_name[name] = session
    missing = [name for name in REQUIRED_SESSIONS if name not in by_name]
    if missing:
        raise SystemExit("benchmark artifact is missing sessions: " + ", ".join(missing))
    return by_name


def samples(summary: dict[str, Any], bucket: str) -> list[int]:
    raw_samples = summary.get(bucket, {}).get("samples", [])
    if not isinstance(raw_samples, list) or not raw_samples:
        raise ValueError(f"{bucket}: no samples")
    out = []
    for value in raw_samples:
        if not isinstance(value, int) or value < 0:
            raise ValueError(f"{bucket}: invalid sample {value!r}")
        out.append(value)
    return out


def validate_benchmark(artifact: dict[str, Any]) -> tuple[list[dict[str, Any]], list[str]]:
    factors = hardware_factors(artifact)
    sessions = session_by_name(artifact)
    results: list[dict[str, Any]] = []
    failures: list[str] = []

    for session_name in REQUIRED_SESSIONS:
        summary = sessions[session_name].get("summary", {})
        if not isinstance(summary, dict):
            failures.append(f"{session_name}: missing summary")
            continue

        for budget in WINDOWS_HEADLESS_BUDGETS:
            factor = factors[budget.factor]
            reference_factor = REFERENCE_FACTORS[budget.factor]
            effective_budget = max(
                1, math.ceil(budget.reference_budget_us * (factor / reference_factor))
            )
            try:
                observed = nearest_rank(samples(summary, budget.bucket), budget.percentile)
            except ValueError as exc:
                failures.append(f"{session_name}.{budget.metric}: {exc}")
                continue

            passed = observed <= effective_budget
            result = {
                "session": session_name,
                "metric": budget.metric,
                "percentile": budget.percentile,
                "observed_us": observed,
                "reference_budget_us": budget.reference_budget_us,
                "reference_hardware_factor": reference_factor,
                "hardware_factor": factor,
                "effective_budget_us": effective_budget,
                "pass": passed,
            }
            results.append(result)
            if not passed:
                failures.append(
                    f"{session_name}.{budget.metric}: observed {observed}us exceeds "
                    f"effective budget {effective_budget}us "
                    f"(TzeHouse budget {budget.reference_budget_us}us, "
                    f"{budget.factor} factor {factor:.4f}, "
                    f"TzeHouse factor {reference_factor:.4f})"
                )

        for counter in ZERO_COUNTERS:
            observed = summary.get(counter)
            passed = observed == 0
            results.append(
                {
                    "session": session_name,
                    "metric": counter,
                    "observed": observed,
                    "budget": 0,
                    "pass": passed,
                }
            )
            if not passed:
                failures.append(f"{session_name}.{counter}: expected 0, observed {observed!r}")

    return results, failures


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--benchmark-json", type=Path, required=True)
    parser.add_argument("--output-json", type=Path)
    args = parser.parse_args()

    artifact = load_json(args.benchmark_json)
    results, failures = validate_benchmark(artifact)
    report = {
        "schema": "tze_hud.windows_perf_budget_gate.v1",
        "reference_hardware": REFERENCE_HARDWARE_TAG,
        "source_artifact": str(args.benchmark_json),
        "results": results,
        "failures": failures,
        "verdict": "fail" if failures else "pass",
    }

    if args.output_json:
        args.output_json.parent.mkdir(parents=True, exist_ok=True)
        args.output_json.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")

    for result in results:
        status = "PASS" if result["pass"] else "FAIL"
        if "observed_us" in result:
            print(
                f"{status} {result['session']}.{result['metric']}: "
                f"{result['observed_us']}us <= {result['effective_budget_us']}us"
            )
        else:
            print(
                f"{status} {result['session']}.{result['metric']}: "
                f"{result['observed']} == {result['budget']}"
            )

    if failures:
        print("\nWindows performance budget failures:", file=sys.stderr)
        for failure in failures:
            print(f"  - {failure}", file=sys.stderr)
        return 1

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
