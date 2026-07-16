#!/usr/bin/env python3
# /// script
# requires-python = ">=3.11"
# ///
"""Fail-closed gate for quiescent-runtime efficiency artifacts."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any


WAKEUP_CEILING = 120
ZERO_GPU_CEILING = 0


def _object(parent: dict[str, Any], field: str, failures: list[str]) -> dict[str, Any]:
    value = parent.get(field)
    if not isinstance(value, dict):
        failures.append(f"{field}: required object is missing or invalid")
        return {}
    return value


def _integer(
    parent: dict[str, Any], path: str, field: str, failures: list[str]
) -> int | None:
    value = parent.get(field)
    if not isinstance(value, int) or isinstance(value, bool) or value < 0:
        failures.append(f"{path}.{field}: required non-negative integer is missing or invalid")
        return None
    return value


def _nonempty_string(
    parent: dict[str, Any], path: str, field: str, failures: list[str]
) -> str | None:
    value = parent.get(field)
    if not isinstance(value, str) or not value.strip():
        failures.append(f"{path}.{field}: required non-empty string is missing or invalid")
        return None
    return value


def _budget(actual: int | None, ceiling: int) -> dict[str, Any]:
    return {
        "actual": actual,
        "ceiling": ceiling,
        "passed": actual is not None and actual <= ceiling,
    }


def validate_artifact(
    artifact: dict[str, Any], *, require_constrained: bool
) -> tuple[dict[str, Any], list[str]]:
    failures: list[str] = []

    if artifact.get("schema_version") != 1:
        failures.append("schema_version: expected 1")

    scenario = _object(artifact, "scenario", failures)
    if scenario.get("name") != "quiescent_static_scene" or scenario.get("version") != 1:
        failures.append("scenario: expected quiescent_static_scene version 1")

    runtime = _object(artifact, "runtime", failures)
    _nonempty_string(runtime, "runtime", "build", failures)
    window_mode = _nonempty_string(runtime, "runtime", "window_mode", failures)
    if window_mode not in {None, "headless", "overlay", "fullscreen"}:
        failures.append(f"runtime.window_mode: unsupported value {window_mode!r}")

    pacing = _object(artifact, "pacing", failures)
    if pacing.get("mode") != "event_driven" or pacing.get("requested_cadence_hz") is not None:
        failures.append(
            "pacing: quiescent evidence requires event_driven mode and null requested_cadence_hz"
        )

    renderer = _object(artifact, "renderer", failures)
    backend = _nonempty_string(renderer, "renderer", "backend", failures)
    adapter = _nonempty_string(renderer, "renderer", "adapter", failures)
    software = renderer.get("software")
    if not isinstance(software, bool):
        failures.append("renderer.software: required boolean is missing or invalid")

    viewport = _object(artifact, "viewport", failures)
    width = _integer(viewport, "viewport", "width", failures)
    height = _integer(viewport, "viewport", "height", failures)
    if width == 0 or height == 0:
        failures.append("viewport: dimensions must be non-zero")

    settling_ms = _integer(artifact, "artifact", "settling_duration_ms", failures)
    interval_ms = _integer(artifact, "artifact", "interval_duration_ms", failures)
    if settling_ms is not None and settling_ms < 5_000:
        failures.append("settling_duration_ms: must be at least 5000")
    if interval_ms is not None and interval_ms < 60_000:
        failures.append("interval_duration_ms: must be at least 60000")
    if artifact.get("status") != "complete":
        failures.append("status: measurement must be complete")

    wakeups = _object(artifact, "wakeups", failures)
    combined = _integer(wakeups, "wakeups", "combined_runtime_driven", failures)
    main_loop = _integer(wakeups, "wakeups", "main_loop", failures)
    compositor_loop = _integer(wakeups, "wakeups", "compositor_loop", failures)
    _integer(wakeups, "wakeups", "excluded_sampler", failures)
    _integer(wakeups, "wakeups", "excluded_operating_system", failures)
    sources = wakeups.get("sources")
    if not isinstance(sources, dict):
        failures.append("wakeups.sources: required object is missing or invalid")
        sources_total = None
    elif any(
        not isinstance(name, str)
        or not name
        or not isinstance(count, int)
        or isinstance(count, bool)
        or count < 0
        for name, count in sources.items()
    ):
        failures.append("wakeups.sources: keys and non-negative integer counts are required")
        sources_total = None
    else:
        sources_total = sum(sources.values())

    if combined is not None and main_loop is not None and compositor_loop is not None:
        if combined != main_loop + compositor_loop:
            failures.append("wakeups: combined count does not equal main_loop + compositor_loop")
    if combined is not None and sources_total is not None and combined != sources_total:
        failures.append("wakeups: combined count does not equal attributed source total")
    if combined is not None and combined > WAKEUP_CEILING:
        failures.append(
            f"runtime-driven wakeups {combined} exceed {WAKEUP_CEILING} over the measured interval"
        )

    gpu = _object(artifact, "gpu", failures)
    submissions = _integer(gpu, "gpu", "queue_submissions", failures)
    acquisitions = _integer(gpu, "gpu", "surface_acquisitions", failures)
    presents = _integer(gpu, "gpu", "presents", failures)
    if submissions is not None and submissions != 0:
        failures.append(f"GPU queue submissions must be zero, got {submissions}")
    if acquisitions is not None and acquisitions != 0:
        failures.append(f"surface acquisitions must be zero, got {acquisitions}")
    if presents is not None and presents != 0:
        failures.append(f"presents must be zero, got {presents}")

    constrained = artifact.get("constrained_profile")
    if require_constrained:
        if not isinstance(constrained, dict):
            failures.append("constrained_profile: required object is missing or invalid")
        else:
            operating_system = _nonempty_string(
                constrained, "constrained_profile", "operating_system", failures
            )
            _nonempty_string(constrained, "constrained_profile", "cpu_model", failures)
            _nonempty_string(
                constrained, "constrained_profile", "cpu_limit_enforcement", failures
            )
            logical_cpu_limit = _integer(
                constrained, "constrained_profile", "logical_cpu_limit", failures
            )
            if logical_cpu_limit is not None and logical_cpu_limit != 2:
                failures.append(
                    f"constrained_profile.logical_cpu_limit: expected 2, got {logical_cpu_limit}"
                )
            if software is not True:
                failures.append("renderer.software: constrained lane requires true")
            adapter_lower = adapter.lower() if adapter else ""
            backend_lower = backend.lower() if backend else ""
            if operating_system == "linux":
                if "llvmpipe" not in adapter_lower and "lavapipe" not in adapter_lower:
                    failures.append(
                        "renderer.adapter: Linux constrained lane must prove llvmpipe or lavapipe"
                    )
                if "vulkan" not in backend_lower:
                    failures.append(
                        "renderer.backend: Linux constrained lane must identify Vulkan"
                    )
            elif operating_system == "windows":
                if "warp" not in adapter_lower:
                    failures.append(
                        "renderer.adapter: Windows constrained lane must prove WARP"
                    )
            elif operating_system is not None:
                failures.append(
                    f"constrained_profile.operating_system: unsupported {operating_system!r}"
                )

    report = {
        "schema_version": 1,
        "status": "pass" if not failures else "fail",
        "scenario": artifact.get("scenario"),
        "runtime": artifact.get("runtime"),
        "pacing": artifact.get("pacing"),
        "renderer": artifact.get("renderer"),
        "viewport": artifact.get("viewport"),
        "settling_duration_ms": settling_ms,
        "interval_duration_ms": interval_ms,
        "budgets": {
            "runtime_wakeups": _budget(combined, WAKEUP_CEILING),
            "gpu_queue_submissions": _budget(submissions, ZERO_GPU_CEILING),
            "surface_acquisitions": _budget(acquisitions, ZERO_GPU_CEILING),
            "presents": _budget(presents, ZERO_GPU_CEILING),
        },
        "failures": failures,
    }
    return report, failures


def load_artifact(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise SystemExit(f"{path}: invalid efficiency artifact: {error}") from error
    if not isinstance(value, dict):
        raise SystemExit(f"{path}: efficiency artifact root must be an object")
    return value


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("artifact", type=Path)
    parser.add_argument("--report", type=Path)
    parser.add_argument("--require-constrained", action="store_true")
    args = parser.parse_args()

    artifact = load_artifact(args.artifact)
    report, failures = validate_artifact(
        artifact, require_constrained=args.require_constrained
    )
    rendered = json.dumps(report, indent=2, sort_keys=True) + "\n"
    if args.report:
        args.report.parent.mkdir(parents=True, exist_ok=True)
        args.report.write_text(rendered, encoding="utf-8")
    print(rendered, end="")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
