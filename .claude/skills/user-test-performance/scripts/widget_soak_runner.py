#!/usr/bin/env python3
"""Run a three-agent live WidgetPublish soak and emit a summary artifact."""

from __future__ import annotations

import argparse
import datetime as dt
import json
import os
import subprocess
import time
from pathlib import Path
from typing import Any


DEFAULT_AGENTS = "agent-alpha,agent-beta,agent-gamma"
DEFAULT_TARGETS_FILE = "targets/publish_load_targets.toml"
DEFAULT_WIDGET = "main-progress"
DEFAULT_INSTANCE = "main-progress"
DEFAULT_DURATION_S = 3600.0
DEFAULT_RATE_RPS = 1.0
MAX_ARTIFACT_BYTES = 16 * 1024 * 1024


def utc_now_iso() -> str:
    return dt.datetime.now(dt.timezone.utc).replace(microsecond=0).isoformat()


def repo_root() -> Path:
    return Path(__file__).resolve().parents[4]


def parse_agents(raw: str) -> list[str]:
    agents = [part.strip() for part in raw.split(",") if part.strip()]
    if not agents:
        raise SystemExit("--agent-ids must contain at least one agent id")
    return agents


def binary_path(root: Path) -> Path:
    suffix = ".exe" if os.name == "nt" else ""
    return root / "target" / "release" / f"widget_publish_load_harness{suffix}"


def sample_windows_resources(args: argparse.Namespace, label: str) -> dict[str, Any]:
    if not args.sample_windows_resources:
        return {"enabled": False, "label": label}

    ssh_target = f"{args.win_user}@{args.win_host}"
    cmd = [
        "ssh",
        "-o",
        "BatchMode=yes",
    ]
    if args.ssh_identity:
        cmd.extend(["-i", args.ssh_identity])
    cmd.extend(
        [
            ssh_target,
            (
                "powershell -NoProfile -ExecutionPolicy Bypass -Command "
                "\"$p=Get-Process -Name 'tze_hud' -ErrorAction SilentlyContinue; "
                "$gpu=$null; "
                "try { $gpu=(nvidia-smi --query-gpu=utilization.gpu,memory.used "
                "--format=csv,noheader,nounits 2>$null | Select-Object -First 1) } catch {}; "
                "[pscustomobject]@{"
                f"label='{label}'; "
                "timestamp_utc=(Get-Date).ToUniversalTime().ToString('o'); "
                "process_count=($p | Measure-Object).Count; "
                "cpu_seconds_total=($p | Measure-Object CPU -Sum).Sum; "
                "working_set_bytes_total=($p | Measure-Object WorkingSet64 -Sum).Sum; "
                "private_memory_bytes_total=($p | Measure-Object PrivateMemorySize64 -Sum).Sum; "
                "gpu_csv=$gpu"
                "} | ConvertTo-Json -Compress\""
            ),
        ]
    )
    try:
        result = subprocess.run(cmd, check=False, capture_output=True, text=True, timeout=30)
        if result.returncode != 0:
            return {
                "enabled": True,
                "label": label,
                "ok": False,
                "returncode": result.returncode,
                "stderr": result.stderr.strip(),
            }
        parsed = json.loads(result.stdout)
        parsed["enabled"] = True
        parsed["ok"] = True
        return parsed
    except Exception as exc:  # noqa: BLE001
        return {"enabled": True, "label": label, "ok": False, "error": str(exc)}


def build_harness(root: Path, skip_build: bool) -> Path:
    bin_path = binary_path(root)
    if skip_build:
        if not bin_path.exists():
            raise SystemExit(f"--skip-build requested but binary is missing: {bin_path}")
        return bin_path

    subprocess.run(
        ["cargo", "build", "--release", "-p", "widget_publish_load_harness"],
        cwd=root,
        check=True,
    )
    if not bin_path.exists():
        raise SystemExit(f"expected harness binary not found after build: {bin_path}")
    return bin_path


def harness_command(
    *,
    bin_path: Path,
    args: argparse.Namespace,
    agent_id: str,
    output: Path,
    publish_count: int,
    param_step: float,
) -> list[str]:
    cmd = [
        str(bin_path),
        "--targets-file",
        args.targets_file,
        "--target-id",
        args.target_id,
        "--mode",
        "paced",
        "--duration-s",
        f"{args.duration_s:.3f}",
        "--publish-count",
        str(publish_count),
        "--target-rate-rps",
        f"{args.rate_rps:.6f}",
        "--widget-name",
        args.widget_name,
        "--instance-id",
        args.instance_id,
        "--payload-profile",
        "progress_soak",
        "--param-name",
        args.param_name,
        "--param-start",
        "0.0",
        "--param-step",
        f"{param_step:.9f}",
        "--transition-ms",
        str(args.transition_ms),
        "--ttl-us",
        str(args.ttl_us),
        "--timeout-s",
        str(args.timeout_s),
        "--agent-id",
        agent_id,
        "--output",
        str(output),
    ]
    if args.layer4_output_root:
        cmd.extend(["--layer4-output-root", args.layer4_output_root])
    if args.target_p99_rtt_us is not None:
        cmd.extend(["--target-p99-rtt-us", str(args.target_p99_rtt_us)])
    if args.target_throughput_rps is not None:
        cmd.extend(["--target-throughput-rps", str(args.target_throughput_rps)])
    if args.normalization_mapping_approved:
        cmd.append("--normalization-mapping-approved")
    return cmd


def load_agent_artifact(path: Path) -> dict[str, Any]:
    if not path.exists():
        return {"artifact_path": str(path), "artifact_missing": True}
    try:
        artifact_size = path.stat().st_size
        if artifact_size > MAX_ARTIFACT_BYTES:
            return {
                "artifact_path": str(path),
                "artifact_error": f"artifact exceeds {MAX_ARTIFACT_BYTES} byte limit",
                "artifact_size_bytes": artifact_size,
            }
        with path.open("r", encoding="utf-8") as f:
            data = json.load(f)
    except (json.JSONDecodeError, OSError) as exc:
        return {"artifact_path": str(path), "artifact_error": str(exc)}
    data["artifact_path"] = str(path)
    return data


def repo_relative_path(path: str | Path, root: Path) -> str:
    candidate = Path(path)
    if not candidate.is_absolute():
        return str(path)
    try:
        return candidate.relative_to(root).as_posix()
    except ValueError:
        return str(path)


def summarize(
    *,
    args: argparse.Namespace,
    root: Path,
    started_at: str,
    ended_at: str,
    output_root: Path,
    commands: dict[str, list[str]],
    agent_results: dict[str, dict[str, Any]],
    resource_samples: list[dict[str, Any]],
    dry_run: bool,
) -> dict[str, Any]:
    metrics_by_agent: dict[str, Any] = {}
    total_requests = 0
    total_success = 0
    total_errors = 0
    max_rtt_jitter_us = 0

    for agent_id, artifact in agent_results.items():
        metrics = artifact.get("metrics", {})
        total_requests += int(metrics.get("request_count", 0) or 0)
        total_success += int(metrics.get("success_count", 0) or 0)
        total_errors += int(metrics.get("error_count", 0) or 0)
        rtt_jitter_us = max(
            0,
            int(metrics.get("rtt_p99_us", 0) or 0) - int(metrics.get("rtt_p50_us", 0) or 0),
        )
        max_rtt_jitter_us = max(max_rtt_jitter_us, rtt_jitter_us)
        metrics_by_agent[agent_id] = {
            "request_count": metrics.get("request_count"),
            "success_count": metrics.get("success_count"),
            "error_count": metrics.get("error_count"),
            "throughput_rps": metrics.get("throughput_rps"),
            "rtt_p50_us": metrics.get("rtt_p50_us"),
            "rtt_p99_us": metrics.get("rtt_p99_us"),
            "rtt_jitter_us": rtt_jitter_us,
            "verdict": artifact.get("verdict"),
            "artifact_path": repo_relative_path(artifact.get("artifact_path"), root)
            if artifact.get("artifact_path")
            else None,
            "artifact_error": artifact.get("artifact_error"),
            "artifact_missing": artifact.get("artifact_missing"),
            "returncode": artifact.get("returncode"),
        }

    drift = None
    ok_samples = [s for s in resource_samples if s.get("ok")]
    if len(ok_samples) >= 2:
        first = ok_samples[0]
        last = ok_samples[-1]
        drift = {
            "working_set_bytes_delta": (last.get("working_set_bytes_total") or 0)
            - (first.get("working_set_bytes_total") or 0),
            "private_memory_bytes_delta": (last.get("private_memory_bytes_total") or 0)
            - (first.get("private_memory_bytes_total") or 0),
            "cpu_seconds_delta": (last.get("cpu_seconds_total") or 0)
            - (first.get("cpu_seconds_total") or 0),
        }

    return {
        "schema_version": 1,
        "kind": "widget_publish_three_agent_soak",
        "dry_run": dry_run,
        "started_at_utc": started_at,
        "ended_at_utc": ended_at,
        "target_id": args.target_id,
        "targets_file": args.targets_file,
        "widget_name": args.widget_name,
        "instance_id": args.instance_id,
        "duration_s": args.duration_s,
        "rate_rps_per_agent": args.rate_rps,
        "agents": list(agent_results.keys()),
        "aggregate": {
            "request_count": total_requests,
            "success_count": total_success,
            "error_count": total_errors,
            "max_rtt_jitter_us": max_rtt_jitter_us,
        },
        "metrics_by_agent": metrics_by_agent,
        "resource_samples": resource_samples,
        "resource_drift": drift,
        "commands": {
            agent_id: [repo_relative_path(part, root) for part in command]
            for agent_id, command in commands.items()
        },
        "output_root": repo_relative_path(output_root, root),
    }


def write_summary(output_root: Path, summary: dict[str, Any]) -> Path:
    path = output_root / "soak_summary.json"
    with path.open("w", encoding="utf-8") as f:
        json.dump(summary, f, indent=2, sort_keys=True)
        f.write("\n")
    return path


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target-id", default="user-test-windows-tailnet")
    parser.add_argument("--targets-file", default=DEFAULT_TARGETS_FILE)
    parser.add_argument("--agent-ids", default=DEFAULT_AGENTS)
    parser.add_argument("--widget-name", default=DEFAULT_WIDGET)
    parser.add_argument("--instance-id", default=DEFAULT_INSTANCE)
    parser.add_argument("--param-name", default="progress")
    parser.add_argument("--duration-s", type=float, default=DEFAULT_DURATION_S)
    parser.add_argument("--rate-rps", type=float, default=DEFAULT_RATE_RPS)
    parser.add_argument("--transition-ms", type=int, default=0)
    parser.add_argument("--ttl-us", type=int, default=65_000_000)
    parser.add_argument("--timeout-s", type=float, default=30.0)
    parser.add_argument("--target-p99-rtt-us", type=int, default=None)
    parser.add_argument("--target-throughput-rps", type=float, default=None)
    parser.add_argument("--normalization-mapping-approved", action="store_true")
    parser.add_argument("--output-root", default="")
    parser.add_argument("--layer4-output-root", default="")
    parser.add_argument("--skip-build", action="store_true")
    parser.add_argument("--dry-run", action="store_true")
    parser.add_argument("--sample-windows-resources", action="store_true")
    parser.add_argument("--resource-sample-interval-s", type=float, default=300.0)
    parser.add_argument("--win-user", default="hudbot")
    parser.add_argument("--win-host", default="tzehouse-windows.parrot-hen.ts.net")
    parser.add_argument("--ssh-identity", default="")
    args = parser.parse_args()

    if args.duration_s <= 0:
        raise SystemExit("--duration-s must be > 0")
    if args.rate_rps <= 0:
        raise SystemExit("--rate-rps must be > 0")
    if args.resource_sample_interval_s < 0:
        raise SystemExit("--resource-sample-interval-s must be >= 0")

    agents = parse_agents(args.agent_ids)
    root = repo_root()
    timestamp = dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    output_root = Path(args.output_root) if args.output_root else root / "benchmarks" / "soak" / timestamp
    if not output_root.is_absolute():
        output_root = root / output_root
    agent_dir = output_root / "agents"
    log_dir = output_root / "logs"
    agent_dir.mkdir(parents=True, exist_ok=True)
    log_dir.mkdir(parents=True, exist_ok=True)

    publish_count = max(1, int(round(args.duration_s * args.rate_rps)))
    param_step = 0.0 if publish_count <= 1 else 1.0 / float(publish_count - 1)
    planned_bin = binary_path(root)

    commands: dict[str, list[str]] = {}
    for agent_id in agents:
        commands[agent_id] = harness_command(
            bin_path=planned_bin,
            args=args,
            agent_id=agent_id,
            output=agent_dir / f"{agent_id}.json",
            publish_count=publish_count,
            param_step=param_step,
        )

    started_at = utc_now_iso()
    if args.dry_run:
        dry_results = {
            agent_id: {
                "artifact_path": str(agent_dir / f"{agent_id}.json"),
                "returncode": None,
            }
            for agent_id in agents
        }
        summary = summarize(
            args=args,
            root=root,
            started_at=started_at,
            ended_at=utc_now_iso(),
            output_root=output_root,
            commands=commands,
            agent_results=dry_results,
            resource_samples=[],
            dry_run=True,
        )
        path = write_summary(output_root, summary)
        print(f"dry-run summary: {path}")
        return 0

    bin_path = build_harness(root, args.skip_build)
    commands = {
        agent_id: harness_command(
            bin_path=bin_path,
            args=args,
            agent_id=agent_id,
            output=agent_dir / f"{agent_id}.json",
            publish_count=publish_count,
            param_step=param_step,
        )
        for agent_id in agents
    }

    resource_samples = [sample_windows_resources(args, "before")]
    processes: dict[str, subprocess.Popen[Any]] = {}
    log_handles: list[Any] = []
    interrupted = False
    launch_error = None
    try:
        for agent_id, cmd in commands.items():
            stdout = (log_dir / f"{agent_id}.stdout.log").open("w", encoding="utf-8")
            stderr = (log_dir / f"{agent_id}.stderr.log").open("w", encoding="utf-8")
            log_handles.extend([stdout, stderr])
            processes[agent_id] = subprocess.Popen(cmd, cwd=root, stdout=stdout, stderr=stderr)
            stdout.close()
            stderr.close()
            log_handles.remove(stdout)
            log_handles.remove(stderr)

        next_resource_sample = (
            time.monotonic() + args.resource_sample_interval_s
            if args.sample_windows_resources and args.resource_sample_interval_s > 0
            else None
        )
        resource_sample_index = 1
        while any(proc.poll() is None for proc in processes.values()):
            time.sleep(2.0)
            if next_resource_sample is not None and time.monotonic() >= next_resource_sample:
                resource_samples.append(
                    sample_windows_resources(args, f"during-{resource_sample_index}")
                )
                resource_sample_index += 1
                next_resource_sample += args.resource_sample_interval_s
    except KeyboardInterrupt:
        interrupted = True
        for proc in processes.values():
            if proc.poll() is None:
                proc.terminate()
    except Exception as exc:  # noqa: BLE001
        launch_error = str(exc)
        for proc in processes.values():
            if proc.poll() is None:
                proc.terminate()
    finally:
        for handle in log_handles:
            handle.close()
        for proc in processes.values():
            if proc.poll() is None:
                try:
                    proc.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    proc.wait(timeout=10)

    resource_samples.append(sample_windows_resources(args, "after"))
    agent_results: dict[str, dict[str, Any]] = {}
    exit_code = 0
    for agent_id in agents:
        proc = processes.get(agent_id)
        artifact = load_agent_artifact(agent_dir / f"{agent_id}.json")
        artifact["returncode"] = proc.returncode if proc is not None else None
        agent_results[agent_id] = artifact
        if proc is None:
            exit_code = exit_code or 1
        elif proc.returncode != 0:
            exit_code = proc.returncode or 1
        if artifact.get("artifact_error") or artifact.get("artifact_missing"):
            exit_code = exit_code or 1

    if launch_error is not None:
        exit_code = exit_code or 1
    if interrupted:
        exit_code = exit_code or 130

    summary = summarize(
        args=args,
        root=root,
        started_at=started_at,
        ended_at=utc_now_iso(),
        output_root=output_root,
        commands=commands,
        agent_results=agent_results,
        resource_samples=resource_samples,
        dry_run=False,
    )
    if launch_error is not None:
        summary["launch_error"] = launch_error
    if interrupted:
        summary["interrupted"] = True
    path = write_summary(output_root, summary)
    print(f"soak summary: {path}")
    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
