# hud-i0jdz TzeHouse Rerun Blocked - 2026-05-12

Issue: `hud-i0jdz`

Branch: `agent/hud-i0jdz-rerun`

Base commit: `d0df9fd238e7`

Artifact: `docs/reports/artifacts/hud-i0jdz-rerun-blocked-20260512T030529Z/lane_status.json`

## Status

Blocked before starting the fullscreen-vs-overlay rerun.

The hardened harness is present on this branch, and the local harness contract
test still passes. TzeHouse is reachable, but the Windows GPU lane is not clean:
a benchmark-config `tze_hud.exe` process is already running on the canonical
benchmark ports, while `C:\ProgramData\tze_hud\gpu.lock` points at a dead PID.

Starting `scripts/ci/windows/windowed-fullscreen-overlay-perf.ps1` in that state
would create a second compositor on the same GPU lane, which would make
`composite_delta.p99_us` evidence unreliable. Per dispatch constraints, this
worker did not stop, restart, or replace the existing runtime.

## Observed State

- Tailscale ping succeeded for `tzehouse-windows.parrot-hen.ts.net`.
- Non-interactive SSH succeeded with `~/.ssh/ecdsa_home` as `tzeus`.
- TCP ports `22`, `50051`, and `9090` were open.
- `TzeHudBenchmarkOverlay` exists and last returned `0`.
- Running process:
  - PID `30228`
  - command line: `"C:\tze_hud\tze_hud.exe" --config C:\tze_hud\benchmark.toml --window-mode overlay --grpc-port 50051 --mcp-port 9090`
  - start time: `2026-05-12T01:19:57.5089475Z`
- GPU lock:
  - `SESSION_TYPE=interactive`
  - `PID=55904`
  - `STARTED_AT=2026-05-12T02:43:27Z`
  - PID `55904` was not alive during the check.

## Validation

```bash
python3 -m unittest scripts/ci/test_windowed_overlay_perf_script.py -v
```

Result: passed, 2 tests.

## Resume Condition

Resume the rerun only after the coordinator confirms the benchmark/GPU lane is
free or explicitly authorizes cleanup of the existing benchmark-config
`tze_hud.exe` process and stale GPU lock. Then run the hardened harness:

```powershell
C:\tze_hud\windowed-fullscreen-overlay-perf.ps1 `
  -ExePath C:\tze_hud\tze_hud.exe `
  -OutputDir C:\tze_hud\perf\hud-i0jdz\overlay-cost `
  -Frames 600 `
  -WarmupFrames 120 `
  -TargetDeltaUs 500 `
  -FailOnBudget
```

Copy the full output directory into
`docs/reports/artifacts/hud-i0jdz-overlay-cost-<timestamp>/`. The closure
artifact still needs `windowed_fullscreen_vs_overlay_report.json` with
fullscreen and overlay p99 frame times and `composite_delta.p99_us <= 500`.
