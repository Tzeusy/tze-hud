#!/usr/bin/env python3
"""Run gRPC widget publish benchmarking via the Rust publish-load harness."""

from __future__ import annotations

import argparse
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path

import perf_common

REPO_ROOT = Path(__file__).resolve().parents[4]
DEFAULT_RESULTS_CSV = REPO_ROOT / "test_results" / "benchmark_history" / "results.csv"
DEFAULT_ARTIFACT_DIR = REPO_ROOT / "test_results" / "publish_load"


def _float(value: str) -> float:
    parsed = float(value)
    if parsed <= 0:
        raise argparse.ArgumentTypeError("value must be > 0")
    return parsed


def _timestamp_tag() -> str:
    return datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")


def default_output_path(target_id: str, mode: str) -> Path:
    filename = f"{_timestamp_tag()}_{target_id}_{mode}.json"
    return DEFAULT_ARTIFACT_DIR / filename


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target-id", required=True)
    parser.add_argument(
        "--targets-file",
        default=str(REPO_ROOT / "targets" / "publish_load_targets.toml"),
    )
    parser.add_argument("--mode", choices=["burst", "paced"], default="burst")
    parser.add_argument("--publish-count", type=int)
    parser.add_argument("--duration-s", type=_float)
    parser.add_argument("--target-rate-rps", type=_float)
    parser.add_argument("--widget-name", default="gauge")
    parser.add_argument("--instance-id", default="publish-load-harness")
    parser.add_argument("--payload-profile", default="gauge_default")
    parser.add_argument("--param-name", default="value")
    parser.add_argument("--param-start", type=float, default=0.0)
    parser.add_argument("--param-step", type=float, default=1.0)
    parser.add_argument("--transition-ms", type=int, default=0)
    parser.add_argument("--ttl-us", type=int, default=0)
    parser.add_argument("--timeout-s", type=_float, default=30.0)
    parser.add_argument("--agent-id", default="widget-publish-load-harness")
    parser.add_argument("--psk")
    parser.add_argument("--normalization-mapping-approved", action="store_true")
    parser.add_argument("--target-p99-rtt-us", type=int)
    parser.add_argument("--target-throughput-rps", type=float)
    parser.add_argument("--output-json")
    parser.add_argument("--results-csv", default=str(DEFAULT_RESULTS_CSV))
    parser.add_argument("--cargo-profile", choices=["release", "debug"], default="release")
    parser.add_argument("--no-history-compare", action="store_true")
    return parser


def run_harness(args: argparse.Namespace, artifact_path: Path) -> int:
    cmd = ["cargo", "run", "-p", "widget_publish_load_harness"]
    if args.cargo_profile == "release":
        cmd.append("--release")
    cmd.extend(
        [
            "--",
            "--target-id",
            args.target_id,
            "--targets-file",
            args.targets_file,
            "--mode",
            args.mode,
            "--widget-name",
            args.widget_name,
            "--instance-id",
            args.instance_id,
            "--payload-profile",
            args.payload_profile,
            "--param-name",
            args.param_name,
            "--param-start",
            str(args.param_start),
            "--param-step",
            str(args.param_step),
            "--transition-ms",
            str(args.transition_ms),
            "--ttl-us",
            str(args.ttl_us),
            "--timeout-s",
            str(args.timeout_s),
            "--output",
            str(artifact_path),
            "--agent-id",
            args.agent_id,
        ]
    )

    passthrough_fields = [
        ("publish_count", "--publish-count"),
        ("duration_s", "--duration-s"),
        ("target_rate_rps", "--target-rate-rps"),
        ("psk", "--psk"),
        ("target_p99_rtt_us", "--target-p99-rtt-us"),
        ("target_throughput_rps", "--target-throughput-rps"),
    ]
    for field, flag in passthrough_fields:
        value = getattr(args, field)
        if value is not None:
            cmd.extend([flag, str(value)])

    if args.normalization_mapping_approved:
        cmd.append("--normalization-mapping-approved")

    print("[user-test-performance] running Rust harness:")
    print(" ", " ".join(_redacted_command_for_log(cmd)))
    proc = subprocess.run(cmd, cwd=REPO_ROOT)
    return proc.returncode


def _redacted_command_for_log(cmd: list[str]) -> list[str]:
    redacted: list[str] = []
    i = 0
    while i < len(cmd):
        token = cmd[i]
        if token == "--psk" and (i + 1) < len(cmd):
            redacted.extend([token, "<redacted>"])
            i += 2
            continue
        redacted.append(token)
        i += 1
    return redacted


def _parse_float(field: str, row: dict) -> float | None:
    value = row.get(field, "")
    if value == "":
        return None
    try:
        return float(value)
    except ValueError:
        return None


def print_history_comparison(current: dict, previous: dict | None) -> None:
    print("[user-test-performance] benchmark key:", current.get("benchmark_key", ""))
    if previous is None:
        print("[user-test-performance] no historical baseline for this benchmark key")
        return

    current_thr = _parse_float("throughput_rps", current)
    previous_thr = _parse_float("throughput_rps", previous)
    current_p99 = _parse_float("rtt_p99_us", current)
    previous_p99 = _parse_float("rtt_p99_us", previous)

    print("[user-test-performance] historical comparison:")
    if current_thr is not None and previous_thr is not None:
        print(f"  throughput_rps: {previous_thr:.3f} -> {current_thr:.3f} ({current_thr - previous_thr:+.3f})")
    if current_p99 is not None and previous_p99 is not None:
        print(f"  rtt_p99_us: {previous_p99:.3f} -> {current_p99:.3f} ({current_p99 - previous_p99:+.3f})")


def main() -> int:
    parser = build_parser()
    args = parser.parse_args()

    artifact_path = Path(args.output_json) if args.output_json else default_output_path(args.target_id, args.mode)
    artifact_path.parent.mkdir(parents=True, exist_ok=True)
    csv_path = Path(args.results_csv)

    code = run_harness(args, artifact_path)
    if code != 0:
        return code

    if not artifact_path.exists():
        print(f"artifact missing after harness run: {artifact_path}", file=sys.stderr)
        return 2

    row = perf_common.append_artifact(csv_path, artifact_path)
    previous = None
    if not args.no_history_compare:
        previous = perf_common.find_latest_by_benchmark_key(
            csv_path,
            row.get("benchmark_key", ""),
            exclude_artifact_path=row.get("artifact_path"),
        )

    print(f"[user-test-performance] artifact: {artifact_path}")
    print(f"[user-test-performance] results csv: {csv_path}")
    print_history_comparison(row, previous)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
