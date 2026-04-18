#!/usr/bin/env python3
"""
MCP publish_to_zone stress test with performance telemetry.

Exercises the MCP publish_to_zone endpoint at varying load levels while
collecting latency percentiles and host resource telemetry via SSH.

Connection defaults:
  MCP URL : http://tzehouse-windows.parrot-hen.ts.net:9090
  PSK env : MCP_TEST_PSK  (default token: tze-hud-key)
  SSH host: tzeus@tzehouse-windows.parrot-hen.ts.net
  SSH key : ~/.ssh/ecdsa_home
"""

from __future__ import annotations

import argparse
import json
import math
import os
import subprocess
import sys
import threading
import time
import urllib.error
import urllib.request
import uuid
from dataclasses import asdict, dataclass, field
from typing import Any

# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

DEFAULT_MCP_URL = "http://tzehouse-windows.parrot-hen.ts.net:9090"
DEFAULT_PSK_ENV = "MCP_TEST_PSK"
DEFAULT_PSK_FALLBACK = "tze-hud-key"
DEFAULT_SSH_USER = "tzeus"
DEFAULT_SSH_HOST = "tzehouse-windows.parrot-hen.ts.net"
DEFAULT_SSH_KEY = os.path.expanduser("~/.ssh/ecdsa_home")
DEFAULT_REPORT_FILE = "stress_report.json"
DEFAULT_NAMESPACE = "stress-test"
DEFAULT_TTL_US = 30_000_000  # 30 seconds

# All 6 default zones with appropriate sample content
ZONES: list[dict[str, Any]] = [
    {
        "zone_name": "subtitle",
        "content": "Stress test running -- subtitle zone",
        "merge_key": "stress-subtitle",
    },
    {
        "zone_name": "status-bar",
        "content": "Stress test active",
        "merge_key": "stress-status",
    },
    {
        "zone_name": "notification-area",
        "content": "Stress test notification",
        "merge_key": "stress-notification",
    },
    {
        "zone_name": "alert-banner",
        "content": "Stress test alert banner",
        "merge_key": "stress-alert",
    },
    {
        "zone_name": "pip",
        "content": "PIP stress test content",
        "merge_key": "stress-pip",
    },
    {
        "zone_name": "ambient-background",
        "content": "Ambient background stress content",
        "merge_key": "stress-ambient",
    },
]

# Load profiles: (name, rate_per_sec, duration_sec)
PROFILES: list[tuple[str, float, int]] = [
    ("idle", 1, 15),
    ("low", 5, 15),
    ("medium", 20, 15),
    ("high", 50, 15),
    ("burst", 100, 10),
]

# ---------------------------------------------------------------------------
# RPC helper (same pattern as publish_zone_batch.py)
# ---------------------------------------------------------------------------


def rpc_call(
    url: str,
    token: str,
    method: str,
    params: dict[str, Any],
    request_id: int,
    timeout: float = 20.0,
) -> dict[str, Any]:
    body = json.dumps(
        {
            "jsonrpc": "2.0",
            "id": request_id,
            "method": method,
            "params": params,
        }
    ).encode("utf-8")
    req = urllib.request.Request(
        url=url,
        data=body,
        headers={
            "Content-Type": "application/json",
            "Authorization": f"Bearer {token}",
        },
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=timeout) as resp:
        payload = resp.read().decode("utf-8")
    return json.loads(payload)


# ---------------------------------------------------------------------------
# Telemetry: background SSH sampling thread
# ---------------------------------------------------------------------------


@dataclass
class TelemetrySample:
    """One per-second snapshot from the remote host."""

    wall_ts: float               # time.time() when sample was taken
    cpu_total_sec: float | None  # raw Get-Process .CPU (cumulative seconds)
    cpu_pct: float | None        # delta CPU% (computed from prev; None for first)
    ws_mb: float | None          # working set MB
    private_mb: float | None     # private memory MB
    gpu_util_pct: float | None
    gpu_mem_mb: float | None


def _parse_proc_line(line: str) -> tuple[float | None, float | None, float | None]:
    """Parse 'cpu_sec ws_mb priv_mb' from PowerShell output."""
    parts = line.strip().split()
    if len(parts) >= 3:
        try:
            return float(parts[0]), float(parts[1]), float(parts[2])
        except ValueError:
            pass
    return None, None, None


def _parse_nvidia_line(line: str) -> tuple[float | None, float | None]:
    """Parse 'util_pct, mem_mb' from nvidia-smi output."""
    parts = [p.strip() for p in line.split(",")]
    if len(parts) >= 2:
        try:
            return float(parts[0]), float(parts[1])
        except ValueError:
            pass
    return None, None


class TelemetryThread:
    """
    Background thread that SSH-samples host metrics every 1 second.

    Maintains a single persistent SSH connection running a remote loop that
    emits one line per second containing process CPU seconds, working set MB,
    private memory MB, GPU utilization %, and GPU memory MB.

    Usage::

        thr = TelemetryThread(user, host, key)
        ok = thr.start()          # False if SSH connection fails
        # ... run load profile ...
        thr.stop()                # join(timeout=5), kill SSH if needed
        samples = thr.samples     # list[TelemetrySample]
        avg_cpu = thr.avg_cpu_pct # float | None

    If SSH fails at start(), returns False and the profile continues with
    telemetry_status="incomplete".
    """

    # Remote script emits one line per second:
    #   "<cpu_sec> <ws_mb> <priv_mb>|<gpu_util>,<gpu_mem>"
    # The '|' separates process stats from GPU stats for unambiguous parsing.
    _REMOTE_LOOP = (
        "while true; do "
        "CPU=$(powershell -NoProfile -NonInteractive -Command \""
        "try { $p = Get-Process tze_hud -ErrorAction Stop | Select-Object -First 1; "
        "Write-Output \\\"$($p.CPU) "
        "$([math]::Round($p.WorkingSet64/1MB,2)) "
        "$([math]::Round($p.PrivateMemorySize64/1MB,2))\\\" "
        "} catch { Write-Output 'none none none' }"
        "\"); "
        "GPU=$(nvidia-smi --query-gpu=utilization.gpu,memory.used "
        "--format=csv,noheader,nounits 2>/dev/null || echo '0,0'); "
        "echo \"${CPU}|${GPU}\"; "
        "sleep 1; "
        "done"
    )

    def __init__(self, user: str, host: str, key: str) -> None:
        self._user = user
        self._host = host
        self._key = key
        self._stop_event = threading.Event()
        self._thread: threading.Thread | None = None
        self._proc: subprocess.Popen | None = None  # type: ignore[type-arg]
        self.samples: list[TelemetrySample] = []
        self._lock = threading.Lock()

    def start(self) -> bool:
        """
        Spawn the background SSH process and start the sampling thread.

        Returns True on success; False if SSH fails to produce output within
        12 seconds (auth error, unreachable host, etc.). On False the caller
        should set telemetry_status="incomplete" and continue the profile.
        """
        try:
            self._proc = subprocess.Popen(
                [
                    "ssh",
                    "-i", self._key,
                    "-o", "BatchMode=yes",
                    "-o", "StrictHostKeyChecking=no",
                    "-o", "ConnectTimeout=8",
                    f"{self._user}@{self._host}",
                    self._REMOTE_LOOP,
                ],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
        except Exception as exc:
            print(
                f"WARNING: telemetry SSH launch failed: {exc}",
                file=sys.stderr,
            )
            return False

        # Probe: wait up to 12s for the first output line to confirm connection.
        first_line_event = threading.Event()
        first_line: list[str] = []

        def _read_first() -> None:
            assert self._proc is not None
            assert self._proc.stdout is not None
            line = self._proc.stdout.readline()
            first_line.append(line)
            first_line_event.set()

        probe = threading.Thread(target=_read_first, daemon=True)
        probe.start()
        got_first = first_line_event.wait(timeout=12.0)

        if not got_first or not first_line or not first_line[0].strip():
            stderr_data = ""
            if self._proc.stderr:
                try:
                    stderr_data = self._proc.stderr.read(512)
                except Exception:
                    pass
            print(
                f"WARNING: telemetry SSH produced no output within 12s "
                f"(stderr: {stderr_data.strip()!r}). "
                "Profile will continue without telemetry.",
                file=sys.stderr,
            )
            try:
                self._proc.kill()
            except Exception:
                pass
            return False

        # Seed samples with the first line already read.
        first_sample = self._parse_line(first_line[0], prev_sample=None)
        with self._lock:
            self.samples.append(first_sample)

        self._thread = threading.Thread(target=self._run, daemon=True)
        self._thread.start()
        return True

    def _parse_line(
        self,
        line: str,
        prev_sample: TelemetrySample | None,
    ) -> TelemetrySample:
        """Parse one output line from the remote loop into a TelemetrySample."""
        wall_ts = time.time()
        cpu_total_sec: float | None = None
        ws_mb: float | None = None
        private_mb: float | None = None
        gpu_util: float | None = None
        gpu_mem: float | None = None

        line = line.strip()
        if "|" in line:
            proc_part, gpu_part = line.split("|", 1)
        else:
            proc_part = line
            gpu_part = ""

        cpu_total_sec, ws_mb, private_mb = _parse_proc_line(proc_part)
        if gpu_part:
            gpu_util, gpu_mem = _parse_nvidia_line(gpu_part)

        # Instantaneous CPU% = delta_cpu_seconds / delta_wall_seconds * 100
        cpu_pct: float | None = None
        if (
            prev_sample is not None
            and prev_sample.cpu_total_sec is not None
            and cpu_total_sec is not None
        ):
            dt_wall = wall_ts - prev_sample.wall_ts
            dt_cpu = cpu_total_sec - prev_sample.cpu_total_sec
            if dt_wall > 0 and dt_cpu >= 0:
                cpu_pct = (dt_cpu / dt_wall) * 100.0

        return TelemetrySample(
            wall_ts=wall_ts,
            cpu_total_sec=cpu_total_sec,
            cpu_pct=cpu_pct,
            ws_mb=ws_mb,
            private_mb=private_mb,
            gpu_util_pct=gpu_util,
            gpu_mem_mb=gpu_mem,
        )

    def _run(self) -> None:
        """Thread body: read lines from SSH stdout, build sample list."""
        assert self._proc is not None
        assert self._proc.stdout is not None

        while not self._stop_event.is_set():
            try:
                line = self._proc.stdout.readline()
            except Exception:
                break
            if not line:
                # SSH process exited unexpectedly
                break
            with self._lock:
                prev = self.samples[-1] if self.samples else None
                sample = self._parse_line(line, prev_sample=prev)
                self.samples.append(sample)

    def stop(self) -> None:
        """
        Signal the thread to stop and join with a 5-second timeout.

        If the thread does not stop within 5 seconds (e.g. blocked on
        readline), kill the SSH subprocess to unblock it.
        """
        self._stop_event.set()

        if self._thread is not None:
            self._thread.join(timeout=5.0)
            if self._thread.is_alive():
                if self._proc is not None:
                    try:
                        self._proc.kill()
                    except Exception:
                        pass
                self._thread.join(timeout=1.0)
        elif self._proc is not None:
            # start() probing failed but proc was launched; clean up.
            try:
                self._proc.kill()
            except Exception:
                pass

    @property
    def avg_cpu_pct(self) -> float | None:
        """
        Average CPU% across all samples with valid per-sample cpu_pct.

        Averages individual delta-based cpu_pct values, which correctly handles
        counter resets (process restarts) and gaps in data collection.
        Returns None if no samples with valid cpu_pct exist.
        """
        with self._lock:
            pcts = [s.cpu_pct for s in self.samples if s.cpu_pct is not None and not math.isnan(s.cpu_pct)]
        if not pcts:
            return None
        return sum(pcts) / len(pcts)

    def to_dict_list(self) -> list[dict[str, Any]]:
        """Serialize samples to a list of dicts for the JSON report."""
        with self._lock:
            return [
                {
                    "wall_ts": s.wall_ts,
                    "cpu_pct": s.cpu_pct,
                    "ws_mb": s.ws_mb,
                    "private_mb": s.private_mb,
                    "gpu_util_pct": s.gpu_util_pct,
                    "gpu_mem_mb": s.gpu_mem_mb,
                }
                for s in self.samples
            ]


# ---------------------------------------------------------------------------
# Telemetry: single-shot SSH-based host metrics (kept for backward compat)
# ---------------------------------------------------------------------------


def _ssh_cmd(
    user: str,
    host: str,
    key: str,
    remote_cmd: str,
    timeout: float = 10.0,
) -> str:
    """Run a command on the remote host via SSH and return stdout."""
    result = subprocess.run(
        [
            "ssh",
            "-i", key,
            "-o", "BatchMode=yes",
            "-o", "StrictHostKeyChecking=no",
            "-o", "ConnectTimeout=8",
            f"{user}@{host}",
            remote_cmd,
        ],
        capture_output=True,
        text=True,
        timeout=timeout,
    )
    return result.stdout.strip()


def collect_host_metrics(
    user: str,
    host: str,
    key: str,
) -> dict[str, Any]:
    """
    Collect CPU, memory, and GPU metrics from the Windows host.

    Returns a dict with keys: cpu_percent, mem_percent, gpu_util_percent,
    gpu_mem_percent. Values are floats or None on failure.
    """
    metrics: dict[str, Any] = {
        "cpu_percent": None,
        "mem_percent": None,
        "gpu_util_percent": None,
        "gpu_mem_percent": None,
        "error": None,
    }

    # CPU + memory via PowerShell (works on Windows with OpenSSH)
    ps_cpu_mem = (
        "powershell -NoProfile -NonInteractive -Command \""
        "$cpu = (Get-CimInstance Win32_Processor | "
        "Measure-Object -Property LoadPercentage -Average).Average; "
        "$os = Get-CimInstance Win32_OperatingSystem; "
        "$memPct = [math]::Round(100 * ($os.TotalVisibleMemorySize - "
        "$os.FreePhysicalMemory) / $os.TotalVisibleMemorySize, 1); "
        "Write-Output \\\"$cpu $memPct\\\"\""
    )

    # GPU via nvidia-smi (outputs: utilization.gpu, utilization.memory)
    nvidia_cmd = (
        "nvidia-smi --query-gpu=utilization.gpu,utilization.memory "
        "--format=csv,noheader,nounits"
    )

    try:
        cpu_mem_out = _ssh_cmd(user, host, key, ps_cpu_mem)
        parts = cpu_mem_out.split()
        if len(parts) >= 2:
            metrics["cpu_percent"] = float(parts[0])
            metrics["mem_percent"] = float(parts[1])
    except Exception as exc:
        metrics["error"] = f"cpu/mem SSH error: {exc}"

    try:
        gpu_out = _ssh_cmd(user, host, key, nvidia_cmd)
        if gpu_out:
            gpu_parts = [p.strip() for p in gpu_out.split(",")]
            if len(gpu_parts) >= 2:
                metrics["gpu_util_percent"] = float(gpu_parts[0])
                metrics["gpu_mem_percent"] = float(gpu_parts[1])
    except Exception as exc:
        existing = metrics.get("error")
        metrics["error"] = (
            f"{existing}; gpu SSH error: {exc}" if existing else f"gpu SSH error: {exc}"
        )

    return metrics


# ---------------------------------------------------------------------------
# Latency statistics
# ---------------------------------------------------------------------------


def percentile(sorted_data: list[float], p: float) -> float:
    """Compute percentile p (0-100) from a pre-sorted list."""
    if not sorted_data:
        return float("nan")
    idx = (p / 100.0) * (len(sorted_data) - 1)
    lo = int(idx)
    hi = lo + 1
    if hi >= len(sorted_data):
        return sorted_data[-1]
    frac = idx - lo
    return sorted_data[lo] + frac * (sorted_data[hi] - sorted_data[lo])


@dataclass
class ProfileResult:
    profile_name: str
    rate_per_sec: float
    duration_sec: int
    total_requests: int = 0
    success_count: int = 0
    error_count: int = 0
    latencies_ms: list = field(default_factory=list)
    host_metrics_start: dict = field(default_factory=dict)
    host_metrics_end: dict = field(default_factory=dict)
    start_ts: float = 0.0
    end_ts: float = 0.0
    # Background telemetry time-series
    telemetry_samples: list = field(default_factory=list)  # list[dict] serialized
    telemetry_avg_cpu_pct: float | None = None
    telemetry_status: str = "ok"  # "ok" | "incomplete"

    @property
    def error_rate(self) -> float:
        if self.total_requests == 0:
            return float("nan")
        return self.error_count / self.total_requests

    def latency_stats(self) -> dict[str, float]:
        if not self.latencies_ms:
            return {k: float("nan") for k in ("p50", "p95", "p99", "max", "mean")}
        s = sorted(self.latencies_ms)
        return {
            "p50": percentile(s, 50),
            "p95": percentile(s, 95),
            "p99": percentile(s, 99),
            "max": s[-1],
            "mean": sum(s) / len(s),
        }

    def to_dict(self) -> dict[str, Any]:
        d = asdict(self)
        d["error_rate"] = self.error_rate
        d["latency_stats_ms"] = self.latency_stats()
        # Drop raw latency list from report to keep file compact
        d.pop("latencies_ms", None)
        return d


# ---------------------------------------------------------------------------
# Preflight gate
# ---------------------------------------------------------------------------


def preflight_check(url: str, token: str) -> bool:
    """
    Send a single list_zones call to verify the MCP endpoint is reachable.

    Returns True if the endpoint responds with a valid JSON-RPC result.
    Returns False (after printing to stderr) on any failure: connection
    refused, DNS error, HTTP non-200, JSON-RPC error, or timeout.
    """
    try:
        resp = rpc_call(url, token, "list_zones", {}, request_id=0, timeout=5.0)
    except urllib.error.URLError as exc:
        print(
            f"MCP endpoint unreachable at {url}: {exc.reason}",
            file=sys.stderr,
        )
        return False
    except OSError as exc:
        print(
            f"MCP endpoint unreachable at {url}: {exc}",
            file=sys.stderr,
        )
        return False
    except Exception as exc:  # noqa: BLE001
        print(
            f"MCP endpoint unreachable at {url}: {exc}",
            file=sys.stderr,
        )
        return False

    if "error" in resp:
        err = resp["error"]
        print(
            f"MCP endpoint unreachable at {url}: JSON-RPC error {err}",
            file=sys.stderr,
        )
        return False

    return True


# ---------------------------------------------------------------------------
# Load driver
# ---------------------------------------------------------------------------


class RateController:
    """Emits ticks at a fixed rate using token-bucket logic."""

    def __init__(self, rate_per_sec: float) -> None:
        self._interval = 1.0 / rate_per_sec
        self._next_tick = time.monotonic()

    def wait_for_next(self) -> None:
        now = time.monotonic()
        wait = self._next_tick - now
        if wait > 0:
            time.sleep(wait)
        self._next_tick += self._interval


def run_profile(
    profile_name: str,
    rate_per_sec: float,
    duration_sec: int,
    url: str,
    token: str,
    ssh_user: str,
    ssh_host: str,
    ssh_key: str,
    verbose: bool = False,
) -> ProfileResult:
    result = ProfileResult(
        profile_name=profile_name,
        rate_per_sec=rate_per_sec,
        duration_sec=duration_sec,
    )

    # --- Start background telemetry thread ---
    telem = TelemetryThread(ssh_user, ssh_host, ssh_key)
    print(f"  [{profile_name}] Starting background telemetry thread...", flush=True)
    telem_ok = telem.start()
    if not telem_ok:
        result.telemetry_status = "incomplete"
        print(
            f"  [{profile_name}] WARNING: telemetry unavailable; "
            "profile continues without host metrics.",
            file=sys.stderr,
            flush=True,
        )

    result.start_ts = time.time()
    deadline = time.monotonic() + duration_sec
    controller = RateController(rate_per_sec)
    req_id_counter = 1
    zone_cycle = 0

    print(
        f"  [{profile_name}] Running {rate_per_sec}/s for {duration_sec}s...",
        flush=True,
    )

    while time.monotonic() < deadline:
        controller.wait_for_next()
        if time.monotonic() >= deadline:
            break

        zone = ZONES[zone_cycle % len(ZONES)]
        zone_cycle += 1
        req_id = req_id_counter
        req_id_counter += 1

        params: dict[str, Any] = {
            "zone_name": zone["zone_name"],
            "content": f"{zone['content']} #{req_id}",
            "namespace": DEFAULT_NAMESPACE,
            "ttl_us": DEFAULT_TTL_US,
            "merge_key": zone["merge_key"],
        }

        t0 = time.monotonic()
        try:
            rpc_call(url, token, "publish_to_zone", params, req_id, timeout=5.0)
            latency_ms = (time.monotonic() - t0) * 1000.0
            result.latencies_ms.append(latency_ms)
            result.success_count += 1
            if verbose:
                print(
                    f"    req={req_id} zone={zone['zone_name']}"
                    f" lat={latency_ms:.1f}ms ok"
                )
        except Exception as exc:
            result.error_count += 1
            if verbose:
                latency_ms = (time.monotonic() - t0) * 1000.0
                print(
                    f"    req={req_id} zone={zone['zone_name']}"
                    f" lat={latency_ms:.1f}ms ERR: {exc}"
                )
        finally:
            result.total_requests += 1

    result.end_ts = time.time()

    # --- Stop background telemetry thread ---
    print(f"  [{profile_name}] Stopping telemetry thread...", flush=True)
    telem.stop()

    if telem_ok:
        result.telemetry_samples = telem.to_dict_list()
        result.telemetry_avg_cpu_pct = telem.avg_cpu_pct

    stats = result.latency_stats()
    avg_cpu_str = (
        f"{result.telemetry_avg_cpu_pct:.1f}%"
        if result.telemetry_avg_cpu_pct is not None
        else "n/a"
    )
    n_samples = len(result.telemetry_samples)
    print(
        f"  [{profile_name}] Done: "
        f"total={result.total_requests} "
        f"ok={result.success_count} "
        f"err={result.error_count} "
        f"p50={stats['p50']:.1f}ms "
        f"p99={stats['p99']:.1f}ms "
        f"avg_cpu={avg_cpu_str} "
        f"telem_samples={n_samples}",
        flush=True,
    )

    return result


# ---------------------------------------------------------------------------
# Summary table
# ---------------------------------------------------------------------------


def _fmt(v: Any, decimals: int = 1) -> str:
    if v is None or (isinstance(v, float) and math.isnan(v)):
        return "n/a"
    return f"{v:.{decimals}f}"


def print_summary_table(results: list[ProfileResult]) -> None:
    header = (
        f"{'Profile':<12} {'Rate/s':>6} {'Total':>7} {'Errors':>7} "
        f"{'ErrRate':>8} {'p50ms':>7} {'p95ms':>7} {'p99ms':>7} {'maxms':>7} "
        f"{'AvgCPU%':>8} {'Samples':>8}"
    )
    sep = "-" * len(header)
    print()
    print(sep)
    print("  MCP Zone Publish Stress Test -- Results")
    print(sep)
    print(header)
    print(sep)

    for r in results:
        stats = r.latency_stats()
        err_pct = (
            "n/a"
            if math.isnan(r.error_rate)
            else f"{r.error_rate * 100:.1f}%"
        )
        avg_cpu = _fmt(r.telemetry_avg_cpu_pct)
        n_samples = len(r.telemetry_samples)

        print(
            f"{r.profile_name:<12} {r.rate_per_sec:>6.0f} {r.total_requests:>7} "
            f"{r.error_count:>7} {err_pct:>8} "
            f"{_fmt(stats['p50']):>7} {_fmt(stats['p95']):>7} "
            f"{_fmt(stats['p99']):>7} {_fmt(stats['max']):>7} "
            f"{avg_cpu:>8} {n_samples:>8}"
        )

    print(sep)
    print()


# ---------------------------------------------------------------------------
# Entry point
# ---------------------------------------------------------------------------


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="MCP publish_to_zone stress test with performance telemetry",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__,
    )
    parser.add_argument(
        "--url",
        default=DEFAULT_MCP_URL,
        help=f"MCP HTTP URL (default: {DEFAULT_MCP_URL})",
    )
    parser.add_argument(
        "--psk-env",
        default=DEFAULT_PSK_ENV,
        help=f"Env var containing the PSK (default: {DEFAULT_PSK_ENV})",
    )
    parser.add_argument(
        "--ssh-user",
        default=DEFAULT_SSH_USER,
        help=f"SSH username for host metrics (default: {DEFAULT_SSH_USER})",
    )
    parser.add_argument(
        "--ssh-host",
        default=DEFAULT_SSH_HOST,
        help=f"SSH hostname for host metrics (default: {DEFAULT_SSH_HOST})",
    )
    parser.add_argument(
        "--ssh-key",
        default=DEFAULT_SSH_KEY,
        help=f"SSH private key path (default: {DEFAULT_SSH_KEY})",
    )
    parser.add_argument(
        "--report",
        default=DEFAULT_REPORT_FILE,
        help=f"Path to write JSON report (default: {DEFAULT_REPORT_FILE})",
    )
    parser.add_argument(
        "--profiles",
        nargs="+",
        choices=[p[0] for p in PROFILES] + ["all"],
        default=["all"],
        help="Which load profiles to run (default: all)",
    )
    parser.add_argument(
        "--verbose",
        action="store_true",
        help="Print each request result",
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()

    # Resolve token -- no hardcoded credentials in logic
    token = os.environ.get(args.psk_env, "").strip()
    if not token:
        token = DEFAULT_PSK_FALLBACK
        print(
            f"WARNING: {args.psk_env} not set, using built-in default token.",
            file=sys.stderr,
        )

    # Determine which profiles to run
    selected_names = (
        {p[0] for p in PROFILES} if "all" in args.profiles else set(args.profiles)
    )
    profiles_to_run = [p for p in PROFILES if p[0] in selected_names]

    print("MCP Stress Test")
    print(f"  Target  : {args.url}")
    print(f"  SSH     : {args.ssh_user}@{args.ssh_host} (key: {args.ssh_key})")
    print(f"  Report  : {args.report}")
    print(f"  Zones   : {len(ZONES)}")
    print(f"  Profiles: {[p[0] for p in profiles_to_run]}")
    print()

    # Preflight: verify MCP endpoint is reachable before running any baselines
    # or load profiles. Fails fast with exit code 1 on any connectivity error.
    if not preflight_check(args.url, token):
        sys.exit(1)

    results: list[ProfileResult] = []
    run_id = str(uuid.uuid4())

    for name, rate, dur in profiles_to_run:
        print(f"--- Profile: {name} ({rate}/s, {dur}s) ---", flush=True)
        result = run_profile(
            profile_name=name,
            rate_per_sec=rate,
            duration_sec=dur,
            url=args.url,
            token=token,
            ssh_user=args.ssh_user,
            ssh_host=args.ssh_host,
            ssh_key=args.ssh_key,
            verbose=args.verbose,
        )
        results.append(result)
        # Small cooldown between profiles to let the host settle
        if (name, rate, dur) != profiles_to_run[-1]:
            print("  (cooling down 3s...)", flush=True)
            time.sleep(3)

    print_summary_table(results)

    # Write JSON report
    report = {
        "run_id": run_id,
        "mcp_url": args.url,
        "ssh_host": f"{args.ssh_user}@{args.ssh_host}",
        "zones_tested": [z["zone_name"] for z in ZONES],
        "profiles": [r.to_dict() for r in results],
    }
    with open(args.report, "w", encoding="utf-8") as f:
        json.dump(report, f, indent=2, ensure_ascii=True)
    print(f"Report written to: {args.report}", flush=True)

    # Exit non-zero if any profile had errors
    return 1 if any(r.error_count > 0 for r in results) else 0


if __name__ == "__main__":
    raise SystemExit(main())
