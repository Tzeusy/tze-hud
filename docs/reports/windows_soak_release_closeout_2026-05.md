# Windows Soak And Release Closeout - May 2026

Issue: `hud-ok1y0`
OpenSpec source: `openspec/changes/windows-first-performant-runtime/tasks.md` section 5
Latest attempt: 2026-05-10T02:47:24Z to 2026-05-10T03:34:43Z
Reference host: `TzeHouse` (`tzehouse-windows.parrot-hen.ts.net`, tailnet `100.87.181.125`)

## Verdict

Blocked. Do not tag a Windows release from this attempt.

The Windows reference host is reachable and the benchmark HUD can be launched with the dedicated `C:\tze_hud\benchmark.toml` configuration. A short three-agent smoke run against `main-progress` succeeded before the long run. The canonical 60-minute three-agent soak path did not produce valid per-agent publish artifacts: after 47 minutes and 19 seconds of runtime, all three `widget_publish_load_harness` child processes were still alive, no per-agent JSON had been written, and the parent run had to be interrupted to preserve host state.

This is a release blocker for task 5.1/5.3 because the soak did not prove accepted publish counts, RTT percentiles, frame-time behavior under load, stale UI cleanup, or release readiness. No Windows release tag or artifact was prepared.

Primary evidence:

- `docs/reports/artifacts/windows_soak_release_closeout_2026-05/soak_20260510T024723Z/soak_summary.json`
- `docs/reports/artifacts/windows_soak_release_closeout_2026-05/soak_20260510T024723Z/logs/`

## Reference Hardware

| Surface | Value |
|---|---|
| Host | `TzeHouse` (`tzehouse-windows.parrot-hen.ts.net`) |
| CPU | 13th Gen Intel(R) Core(TM) i5-13600KF, 14 cores / 20 logical processors |
| GPU | NVIDIA GeForce RTX 3080, driver `32.0.15.9636` |
| RAM | 16 GiB physical memory |
| Display | 4096x2160 at 60 Hz, active console session |
| OS | Microsoft Windows 11 Pro, version `10.0.26200`, build `26200` |
| Runtime for attempt | `C:\tze_hud\tze_hud.exe`, overlay mode, gRPC `50051`, MCP `9090`, config `C:\tze_hud\benchmark.toml` |

## Host And Launch Notes

The earlier reachability blocker is resolved. SSH, gRPC, and MCP were reachable before this run. The worker deployed `app/tze_hud_app/config/benchmark.toml` and `scripts/windows/install_benchmark_hud_task.ps1` to `C:\tze_hud\`, registered `TzeHudBenchmarkOverlay`, and launched one benchmark-config `tze_hud.exe` process.

Two launch-script defects were found and fixed locally during setup:

- The generated benchmark runner now trims the DPAPI-protected PSK file before `ConvertTo-SecureString`; the untrimmed CRLF caused `Input string was not in a correct format`.
- The generated benchmark runner now uses `Start-Process` splatting instead of line-continuation backticks; the generated file had lost the backticks, so PowerShell parsed `-ArgumentList` as a command.

After the blocked soak attempt, the worker stopped the benchmark task and restored the production `TzeHudOverlay` task. Final observed process was the production HUD with `C:\tze_hud\tze_hud.toml` on ports `50051`/`9090`.

## Soak Command

The parent runner command was:

```bash
python3 .claude/skills/user-test-performance/scripts/widget_soak_runner.py \
  --target-id user-test-windows-tailnet \
  --duration-s 3600 \
  --rate-rps 1 \
  --sample-windows-resources \
  --resource-sample-interval-s 300 \
  --win-user tzeus \
  --ssh-identity ~/.ssh/ecdsa_home \
  --skip-build \
  --output-root docs/reports/artifacts/windows_soak_release_closeout_2026-05/soak_20260510T024723Z
```

Each child harness targeted `main-progress` as `agent-alpha`, `agent-beta`, and `agent-gamma` with 3,600 planned publishes at 1 rps per agent.

## Observed Results

| Metric | Result |
|---|---:|
| Planned duration | 3,600 s |
| Actual parent-run duration before interruption | 2,839 s |
| Planned agents | 3 |
| Per-agent artifacts | Missing for all three agents |
| Aggregate request/success/error counts | 0 / 0 / 0 in summary because child artifacts were missing |
| Child process return codes | `-15` for all three after parent interruption |
| Resource samples captured | 11 (`before`, `during-1`..`during-9`, `after`) |
| Private memory drift | +47.65 MiB |
| Working set drift | -222.89 MiB |
| CPU seconds delta | 2,631.19 CPU-s over 2,839 wall-s, about 92.7% of one core |
| GPU utilization samples | min 12%, avg 33.7%, max 44% from `nvidia-smi` |

The private-memory drift exceeds the locked manual/reference-host budget of <= 5 MiB for a completed three-agent 60-minute soak. Because the publish artifacts are missing and the run was interrupted, treat this as a blocker signal requiring a rerun after the harness drain issue is fixed, not as a final calibrated leak measurement.

## Required Metrics Matrix

| Required closeout metric | Status |
|---|---|
| Frame-time p50/p99/p99.9 under 60-minute load | Not measured in this soak artifact; current live soak path does not collect frame telemetry while resident publishes run |
| Input latency triple under live load | Not measured in this soak artifact; no accepted per-agent publish artifacts were emitted |
| Idle/loaded CPU/GPU | Partially measured; loaded samples were captured during the blocked run, but no idle sample was captured in the same artifact |
| Memory drift over 60 minutes | Blocker signal: +47.65 MiB private drift over 47.3 minutes, but the run was interrupted and not release-valid |
| Failure/jitter observations | Blocker: long-run child harnesses did not emit artifacts; summary has no RTT/jitter metrics |
| Stale UI / lease cleanup | Not proven; parent interruption terminated child processes before normal closeout artifacts |
| Transparent-overlay composite cost | Not measured in this attempt; use the windowed fullscreen-vs-overlay harness after the soak path is fixed |

For comparison only, the locked baseline report `docs/reports/windows_perf_baseline_2026-05.md` still records the latest valid headless frame-time/input metrics on this reference host. Those baseline metrics are not a substitute for the missing loaded soak metrics.

## Release Decision

Do not tag a Windows release. The blocker preventing release is: the canonical three-agent Windows soak path does not complete with auditable per-agent publish artifacts and does not collect the required frame/input metrics under live load.

Coordinator follow-up work should be created under `hud-9wljr` for:

1. Fix `widget_publish_load_harness` long-run behavior so it drains `WidgetPublishResult` responses concurrently while sending, or otherwise enforces an overall drain deadline that still writes partial diagnostic artifacts.
2. Extend the Windows soak artifact to include frame-time p50/p99/p99.9 and the input latency triple under live resident load, not only publish RTT and process resource samples.
3. Rerun the full 60-minute three-agent benchmark-config soak on `TzeHouse` and gate release tagging on accepted publish artifacts, <= 5 MiB memory drift, and no material jitter/failure observations.

## Prior Recovery Evidence

Earlier host recovery evidence remains relevant:

- `docs/reports/artifacts/windows_soak_release_closeout_2026-05/reachability_probe_20260509T171102Z.json`
- `docs/reports/artifacts/windows_soak_release_closeout_2026-05/reachability_recovery_20260510T022109Z.json`
