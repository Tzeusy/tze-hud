# Windows Benchmark Config And Launch Path

Issue: `hud-l7x8f`
Date: 2026-05-09

## Purpose

`app/tze_hud_app/config/production.toml` remains the production-safe default.
Live Windows widget benchmarks and soak runs now use the separate
`app/tze_hud_app/config/benchmark.toml` file so benchmark-only agent grants do
not broaden the default operator config.

## Deployment Shape

Copy these files to the Windows reference host:

| Local file | Windows path |
|---|---|
| `target/x86_64-pc-windows-gnu/release/tze_hud.exe` | `C:\tze_hud\tze_hud.exe` |
| `app/tze_hud_app/config/benchmark.toml` | `C:\tze_hud\benchmark.toml` |
| `scripts/windows/install_benchmark_hud_task.ps1` | `C:\tze_hud\install_benchmark_hud_task.ps1` |
| `widget_bundles/` and `profiles/` | beside the exe under `C:\tze_hud\` |

Register the benchmark task from the Windows host:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass `
  -File C:\tze_hud\install_benchmark_hud_task.ps1 `
  -BaseDir C:\tze_hud `
  -Psk $env:TZE_HUD_PSK
```

The installer refuses missing PSKs and the default development PSK. It writes a
DPAPI-protected `benchmark_hud.psk.dpapi` for the current task user and the
generated runner passes the key to `tze_hud.exe` through `TZE_HUD_PSK`, not a
command-line argument. The runner stops only existing `tze_hud.exe` processes
whose command line already references the benchmark config and benchmark ports,
so the production `TzeHudOverlay` task is left unchanged.

Launch:

```powershell
schtasks /Run /TN TzeHudBenchmarkOverlay
```

## Registered Benchmark Agents

`benchmark.toml` registers:

| Agent | Purpose | Key grants |
|---|---|---|
| `widget-publish-load-harness` | canonical Rust gRPC publish-load harness | `publish_widget:main-progress`, `read_telemetry` |
| `agent-alpha` / `agent-beta` / `agent-gamma` | three-agent live soak | tile/input grants, `publish_widget:main-*`, `publish_zone:subtitle`, `publish_zone:notification-area`, `publish_zone:status-bar`, `read_telemetry` |

Unregistered agents still receive guest policy.

## Soak Runner

The 60-minute soak entry point is:

```bash
python3 .claude/skills/user-test-performance/scripts/widget_soak_runner.py \
  --target-id user-test-windows-tailnet \
  --duration-s 3600 \
  --rate-rps 1 \
  --windows-live-metrics-path 'C:\tze_hud\perf\<bead-id>\windowed_live_metrics.json' \
  --sample-windows-resources \
  --ssh-identity ~/.ssh/ecdsa_home
```

Artifacts are written under `benchmarks/soak/<timestamp>/`:

| Artifact | Contents |
|---|---|
| `agents/<agent>.json` | per-agent Rust publish-load artifact |
| `logs/<agent>.stdout.log` / `stderr.log` | harness transcripts |
| `live_metrics_source.json` / `live_metrics_summary.json` | copied windowed/runtime telemetry source and normalized frame/input latency metrics |
| `soak_summary.json` | aggregate request counts, success/error counts, RTT jitter, live frame-time p50/p99/p99.9, the three input latency buckets, optional resource drift |

Real soak runs fail closed unless live metrics are supplied with either
`--windows-live-metrics-path` or `--live-metrics-artifact`. The metrics source
must contain frame-time p50/p99/p99.9 and non-empty
`input_to_local_ack`, `input_to_scene_commit`, and `input_to_next_present`
buckets. Use `--allow-missing-live-metrics` only for an explicitly informational
run that cannot satisfy the release closeout gate.

For strict release evidence, pair the resident soak with a bounded
`--benchmark-emit C:\tze_hud\perf\<bead-id>\windowed_live_metrics.json` run and
pass that artifact to the soak summary as the live metrics source. The windowed
benchmark exercises the real windowed compositor and a synthetic pointer probe
through the same `FrameTelemetry.input_to_local_ack_us`,
`input_to_scene_commit_us`, and `input_to_next_present_us` fields used by
runtime telemetry. Treat those input samples as comparable windowed benchmark
input-path evidence, not human desktop interaction.

## MCP Compatibility

The `/user-test-performance` target registry now uses
`http://tzehouse-windows.parrot-hen.ts.net:9090/mcp` for MCP HTTP publishes. The
bare `:9090` URL can return an empty/non-JSON response on the deployed runtime.
