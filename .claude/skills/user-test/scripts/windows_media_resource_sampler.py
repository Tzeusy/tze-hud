#!/usr/bin/env python3
# /// script
# requires-python = ">=3.10"
# ///
"""Collect Windows HUD process CPU/GPU/memory samples for media-ingress soaks."""

from __future__ import annotations

import argparse
import base64
import json
import subprocess
import sys
from pathlib import Path
from typing import Any


def encode_powershell_command(script: str) -> str:
    """Encode a PowerShell command for -EncodedCommand."""
    return base64.b64encode(script.encode("utf-16le")).decode("ascii")


def build_remote_sample_script(*, grpc_port: int, samples: int, interval_s: int) -> str:
    """Return a self-contained PowerShell sampler.

    This deliberately avoids a top-level ``param(...)`` block. A prior artifact
    sampler failed before execution when PowerShell parsed the copied script as
    ordinary command text rather than a script param block.
    """
    if grpc_port <= 0:
        raise ValueError("grpc_port must be positive")
    if samples <= 0:
        raise ValueError("samples must be positive")
    if interval_s < 0:
        raise ValueError("interval_s must be non-negative")

    return f"""
$ErrorActionPreference = 'SilentlyContinue'
$ProgressPreference = 'SilentlyContinue'
$GrpcPort = {grpc_port}
$Samples = {samples}
$IntervalSeconds = {interval_s}

function Read-Gpu3dUtilization {{
    try {{
        $gpuSamples = (Get-Counter '\\GPU Engine(*)\\Utilization Percentage' -ErrorAction Stop).CounterSamples |
            Where-Object {{ $_.InstanceName -like '*engtype_3D*' }}
        if ($gpuSamples) {{
            return [double](($gpuSamples | Measure-Object CookedValue -Sum).Sum)
        }}
    }} catch {{
        $script:errors += "gpu counter unavailable: $($_.Exception.Message)"
    }}
    return $null
}}

function Read-NvidiaSmi {{
    $reading = [ordered]@{{
        utilization_pct = $null
        memory_used_mb = $null
        raw = $null
        error = $null
    }}
    try {{
        $line = (nvidia-smi --query-gpu=utilization.gpu,memory.used --format=csv,noheader,nounits 2>$null |
            Select-Object -First 1)
        if ($line) {{
            $reading.raw = [string]$line
            $parts = ([string]$line).Split(',')
            if ($parts.Count -ge 2) {{
                $reading.utilization_pct = [double]($parts[0].Trim())
                $reading.memory_used_mb = [double]($parts[1].Trim())
            }}
        }}
    }} catch {{
        $reading.error = [string]$_.Exception.Message
    }}
    return $reading
}}

function Read-GpuLockLines {{
    $lockPath = 'C:\\ProgramData\\tze_hud\\gpu.lock'
    if (Test-Path $lockPath) {{
        return @(Get-Content -Path $lockPath | ForEach-Object {{
            $line = [string]$_
            if ($line -match '^DESCRIPTION=') {{
                'DESCRIPTION=<redacted-description>'
            }} else {{
                $line
            }}
        }})
    }}
    return @('gpu_lock=absent')
}}

$errors = @()
$result = [ordered]@{{
    started_at_utc = (Get-Date).ToUniversalTime().ToString('o')
    grpc_port = $GrpcPort
    samples_requested = $Samples
    interval_seconds = $IntervalSeconds
    logical_processors = (Get-CimInstance Win32_ComputerSystem).NumberOfLogicalProcessors
    samples = @()
    errors = @()
}}

$start = Get-Date
for ($i = 0; $i -lt $Samples; $i++) {{
    $listener = Get-NetTCPConnection -State Listen -LocalPort $GrpcPort -ErrorAction SilentlyContinue |
        Select-Object -First 1
    $owner = if ($listener) {{ $listener.OwningProcess }} else {{ $null }}
    $proc = if ($owner) {{ Get-Process -Id $owner -ErrorAction SilentlyContinue }} else {{ $null }}
    $nvidia = Read-NvidiaSmi
    $result.samples += [ordered]@{{
        index = $i
        at_utc = (Get-Date).ToUniversalTime().ToString('o')
        elapsed_s = [math]::Round(((Get-Date) - $start).TotalSeconds, 3)
        listener_pid = $owner
        process_name = if ($proc) {{ $proc.ProcessName }} else {{ $null }}
        cpu_seconds = if ($proc) {{ [double]$proc.CPU }} else {{ $null }}
        working_set_bytes = if ($proc) {{ [int64]$proc.WorkingSet64 }} else {{ $null }}
        private_memory_bytes = if ($proc) {{ [int64]$proc.PrivateMemorySize64 }} else {{ $null }}
        gpu_3d_utilization_pct_sum = Read-Gpu3dUtilization
        nvidia_gpu_utilization_pct = $nvidia.utilization_pct
        nvidia_gpu_memory_used_mb = $nvidia.memory_used_mb
        nvidia_smi_raw = $nvidia.raw
        nvidia_smi_error = $nvidia.error
        gpu_lock = Read-GpuLockLines
    }}
    if ($i -lt ($Samples - 1)) {{
        Start-Sleep -Seconds $IntervalSeconds
    }}
}}

$result.errors = $errors
$result.finished_at_utc = (Get-Date).ToUniversalTime().ToString('o')
$result | ConvertTo-Json -Depth 8 -Compress
""".strip()


def _numbers(samples: list[dict[str, Any]], key: str) -> list[float]:
    values: list[float] = []
    for sample in samples:
        value = sample.get(key)
        if isinstance(value, (int, float)):
            values.append(float(value))
    return values


def _stats(values: list[float]) -> dict[str, Any]:
    if not values:
        return {"count": 0, "min": None, "avg": None, "max": None}
    return {
        "count": len(values),
        "min": round(min(values), 3),
        "avg": round(sum(values) / len(values), 3),
        "max": round(max(values), 3),
    }


def _is_valid_sample(sample: dict[str, Any]) -> bool:
    has_process = (
        sample.get("listener_pid") is not None
        and isinstance(sample.get("cpu_seconds"), (int, float))
        and isinstance(sample.get("working_set_bytes"), (int, float))
        and isinstance(sample.get("private_memory_bytes"), (int, float))
    )
    has_gpu = isinstance(sample.get("gpu_3d_utilization_pct_sum"), (int, float)) or isinstance(
        sample.get("nvidia_gpu_utilization_pct"),
        (int, float),
    )
    return has_process and has_gpu


def _cpu_percent_samples(
    valid_samples: list[dict[str, Any]],
    logical_processors: int,
) -> list[float]:
    if logical_processors <= 0:
        return []
    values: list[float] = []
    for previous, current in zip(valid_samples, valid_samples[1:]):
        previous_cpu = previous.get("cpu_seconds")
        current_cpu = current.get("cpu_seconds")
        previous_elapsed = previous.get("elapsed_s")
        current_elapsed = current.get("elapsed_s")
        if not all(
            isinstance(value, (int, float))
            for value in (previous_cpu, current_cpu, previous_elapsed, current_elapsed)
        ):
            continue
        elapsed_delta = float(current_elapsed) - float(previous_elapsed)
        cpu_delta = float(current_cpu) - float(previous_cpu)
        if elapsed_delta <= 0 or cpu_delta < 0:
            continue
        values.append((cpu_delta / elapsed_delta / logical_processors) * 100.0)
    return values


def summarize_samples(raw: dict[str, Any]) -> dict[str, Any]:
    samples = [sample for sample in raw.get("samples", []) if isinstance(sample, dict)]
    valid_samples = [sample for sample in samples if _is_valid_sample(sample)]
    logical_processors = int(raw.get("logical_processors") or 0)

    first_valid = valid_samples[0] if valid_samples else None
    last_valid = valid_samples[-1] if valid_samples else None

    def drift_bytes(key: str) -> int | None:
        if not first_valid or not last_valid:
            return None
        first = first_valid.get(key)
        last = last_valid.get(key)
        if not isinstance(first, (int, float)) or not isinstance(last, (int, float)):
            return None
        return int(last) - int(first)

    return {
        "sample_count": len(samples),
        "valid_sample_count": len(valid_samples),
        "logical_processors": logical_processors,
        "started_at_utc": raw.get("started_at_utc"),
        "finished_at_utc": raw.get("finished_at_utc"),
        "grpc_port": raw.get("grpc_port"),
        "private_memory_drift_bytes": drift_bytes("private_memory_bytes"),
        "working_set_drift_bytes": drift_bytes("working_set_bytes"),
        "cpu_percent": _stats(_cpu_percent_samples(valid_samples, logical_processors)),
        "gpu_3d_utilization_pct_sum": _stats(
            _numbers(valid_samples, "gpu_3d_utilization_pct_sum")
        ),
        "nvidia_gpu_utilization_pct": _stats(_numbers(valid_samples, "nvidia_gpu_utilization_pct")),
        "nvidia_gpu_memory_used_mb": _stats(_numbers(valid_samples, "nvidia_gpu_memory_used_mb")),
        "first_valid_sample": first_valid,
        "last_valid_sample": last_valid,
        "errors": raw.get("errors", []),
    }


def build_ssh_command(args: argparse.Namespace) -> list[str]:
    cmd = [
        "ssh",
        "-o",
        "BatchMode=yes",
        "-o",
        "IdentitiesOnly=yes",
        "-o",
        f"ConnectTimeout={args.connect_timeout_s}",
    ]
    if args.ssh_key:
        cmd.extend(["-i", args.ssh_key])
    cmd.extend(
        [
            f"{args.win_user}@{args.win_host}",
            "powershell",
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            "-",
        ]
    )
    return cmd


def compute_sampler_timeout_s(*, connect_timeout_s: int, samples: int, interval_s: int) -> int:
    sample_window_s = samples * max(1, interval_s)
    command_overhead_s = samples * 10 + 120
    return connect_timeout_s + max(10, sample_window_s + command_overhead_s)


def collect_samples(args: argparse.Namespace) -> dict[str, Any]:
    script = build_remote_sample_script(
        grpc_port=args.grpc_port,
        samples=args.samples,
        interval_s=args.interval_s,
    )
    cmd = build_ssh_command(args)
    timeout_s = compute_sampler_timeout_s(
        connect_timeout_s=args.connect_timeout_s,
        samples=args.samples,
        interval_s=args.interval_s,
    )
    result = subprocess.run(
        cmd,
        input=script + "\n",
        capture_output=True,
        text=True,
        timeout=timeout_s,
    )
    if result.returncode != 0:
        raise RuntimeError(
            "remote sampler failed "
            f"status={result.returncode} stdout={result.stdout!r} stderr={result.stderr!r}"
        )
    try:
        raw = json.loads(result.stdout)
    except json.JSONDecodeError as exc:
        raise RuntimeError(f"remote sampler emitted invalid JSON: {exc}: {result.stdout!r}") from exc
    if not isinstance(raw, dict):
        raise RuntimeError("remote sampler JSON root must be an object")
    return raw


def write_json(path: Path, payload: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--win-host", default="windows-host.example")
    parser.add_argument("--win-user", default="admin-user")
    parser.add_argument("--ssh-key", default="")
    parser.add_argument("--connect-timeout-s", type=int, default=8)
    parser.add_argument("--grpc-port", type=int, default=50052)
    parser.add_argument("--samples", type=int, default=21)
    parser.add_argument("--interval-s", type=int, default=30)
    parser.add_argument("--raw-output", type=Path, required=True)
    parser.add_argument("--summary-output", type=Path, required=True)
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    raw = collect_samples(args)
    summary = summarize_samples(raw)
    write_json(args.raw_output, raw)
    write_json(args.summary_output, summary)
    print(json.dumps(summary, sort_keys=True))
    return 0 if summary["valid_sample_count"] > 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
