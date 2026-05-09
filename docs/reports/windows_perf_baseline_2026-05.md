# Windows Performance Baseline — May 2026

Issue: `hud-1753c`
Change source: `openspec/changes/windows-first-performant-runtime/tasks.md` §2
Run date: 2026-05-09

## Reference Hardware

Canonical Windows target: `tzehouse-windows.parrot-hen.ts.net`

| Field | Value |
|---|---|
| OS | Microsoft Windows 11 Pro, version 10.0.26200, build 26200, 64-bit |
| CPU | 13th Gen Intel(R) Core(TM) i5-13600KF, 14 cores / 20 logical processors |
| GPU | NVIDIA GeForce RTX 3080, driver 32.0.15.9636 |
| RAM | 16 GiB |
| Display mode observed | 4096 x 2160 via `Win32_VideoController`; runtime overlay log observed 3840 x 2160 surface at 1.5 scale |
| Runtime launch mode | Scheduled task `TzeHudOverlay`, overlay mode, Vulkan backend, premultiplied alpha |

## Artifacts

Raw artifacts are under `docs/reports/artifacts/windows_perf_baseline_2026-05/`:

| Artifact | Purpose |
|---|---|
| `windows_benchmark_300frames.json` | Windows headless Layer-3 benchmark output, 300 frames per scenario |
| `windows_benchmark_stdout.txt` | Remote benchmark transcript |
| `widget_rasterize_criterion.txt` | Windows Criterion output for widget SVG rasterization |
| `widget_publish_load_harness_1000_burst.json` | gRPC widget publish-load attempt against deployed HUD |
| `mcp_results.csv` | MCP publish attempts against deployed HUD |
| `idle_resource_sample.json` | 10-second overlay-mode process/GPU resource sample |

## Headless Frame And Mutation Baseline

Command shape: cross-compiled `benchmark.exe`, run on the Windows target with `--emit C:/tze_hud/windows_perf_benchmark_hud1753c.json --frames 300`. The live HUD was stopped during this headless run and restarted afterwards.

Calibration factors:

| Factor | Value |
|---|---:|
| CPU | 0.7576 |
| GPU fill/composition | 0.3247 |
| Upload | 0.2680 |
| Scene ops/sec | 704,688 |
| Upload tile ops/sec | 18,658 |
| GPU calibration FPS | 1,539.8 |

Frame results:

| Scenario | FPS | frame p50 | frame p99 | frame p99.9 | peak | input_to_scene_commit p99 | input_to_next_present p99 |
|---|---:|---:|---:|---:|---:|---:|---:|
| steady_state_render | 1617.9 | 0.494 ms | 1.979 ms | 2.679 ms | 2.741 ms | 0.002 ms | 1.979 ms |
| high_mutation | 1516.1 | 0.509 ms | 2.139 ms | 2.962 ms | 3.182 ms | 0.001 ms | 2.139 ms |

Validation verdict was `partial`: 6 pass, 0 fail, 1 uncalibrated. `input_to_local_ack` had no samples because this benchmark path does not inject input events.

## Widget Raster Cost

Command shape: cross-compiled `widget_rasterize` Criterion bench, run on the Windows target with sample size 20, 1s warmup, 3s measurement.

| Benchmark | Mean | 95% range |
|---|---:|---:|
| gauge_512x512/cold | 3.679 ms | 2.840-4.282 ms |
| gauge_512x512/warm | 4.775 ms | 4.622-4.902 ms |
| gauge_128x128/warm | 2.326 ms | 2.200-2.437 ms |

This misses the proposed Windows target of <= 1 ms p99 for 512x512 re-rasterization and also exceeds the older v1 2 ms target.

## Live HUD Transport Attempts

The deployed HUD was reachable on ports 50051 and 9090 after starting the `TzeHudOverlay` scheduled task.

`widget_publish_load_harness` reached the gRPC session service but all 1000 publishes were rejected:

| Metric | Value |
|---|---:|
| Requests | 1000 |
| Success | 0 |
| Errors | 1000 |
| p50 RTT | 45.151 ms |
| p95 RTT | 49.657 ms |
| p99 RTT | 49.674 ms |
| Rejection | `WIDGET_CAPABILITY_MISSING: publish_widget:main-progress` |

A temporary attempt to add a benchmark-agent capability to `C:/tze_hud/tze_hud.toml` caused the scheduled task not to rebind HUD ports, so the original config was restored. This means the transport harness is present, but the deployed reference config is not benchmark-ready for gRPC widget publishing.

MCP zone publish attempts were also not accepted:

| Target | Result |
|---|---|
| `status-bar`, 100 publishes | 100/100 rejected with zone media type mismatch |
| `subtitle`, 100 publishes | 100/100 failed with remote close/no response |

These MCP results are diagnostic only and are not treated as successful throughput baselines.

## Idle Resource Sample

Overlay-mode sample, 10 seconds, no benchmark workload:

| Metric | Value |
|---|---:|
| `tze_hud.exe` CPU over sample | 0.0% of total logical CPU |
| Working set | 270,348,288 bytes |
| NVIDIA GPU utilization | 38% device |
| NVIDIA memory used | 5,319 MiB / 10,240 MiB |

GPU utilization is global device utilization from `nvidia-smi`, not per-process attribution, so it is not a clean HUD-only idle metric. It is still far above the proposed <= 0.5% device-utilization target and needs isolated measurement.

## Overlay Composite Cost

No existing harness currently emits a fullscreen-vs-overlay frame-time delta for the windowed compositor. The headless benchmark gives GPU composition cost, and runtime logs confirm overlay mode uses Vulkan with premultiplied alpha, but the proposed `<= +0.5 ms p99` transparent-overlay composite budget is not directly measurable yet from the available tooling.

## Multi-Agent Soak

A 60-minute, three-agent Windows soak was not run in this baseline pass. The current blocking issues are:

1. The reference config grants `agent-alpha`, `agent-beta`, and `agent-gamma` tile/input capabilities only, not widget publish capabilities.
2. The gRPC widget publish-load path cannot establish a successful live widget workload under the deployed config.
3. There is no single Windows soak runner that emits leak/jitter/resource drift metrics in the report shape required by this bead.

## Gap Analysis Against `design.md` §1

| Property | Proposed target | Baseline status |
|---|---:|---|
| Frame time p99 | <= 8.3 ms | PASS in headless steady/high-mutation paths: 1.979 ms / 2.139 ms |
| Frame time p99.9 | <= 16.6 ms | PASS in headless steady/high-mutation paths: 2.679 ms / 2.962 ms |
| input_to_local_ack p99 | <= 2 ms | NOT MEASURED; no input samples in benchmark |
| input_to_scene_commit p99 | <= 25 ms | PASS in headless paths: <= 0.002 ms |
| input_to_next_present p99 | <= 16.6 ms | PASS in headless paths: <= 2.139 ms |
| Widget SVG re-rasterization | <= 1 ms p99 | FAIL; Criterion means 3.679-4.775 ms for 512x512 |
| Transparent-overlay composite delta | <= +0.5 ms p99 | NOT MEASURED; no fullscreen/overlay delta harness |
| Idle CPU | <= 1% single core | PASS in 10s overlay sample: 0.0% total CPU |
| Idle GPU | <= 0.5% device | UNKNOWN/LIKELY FAIL; global `nvidia-smi` reported 38% |
| Memory growth over 60-min soak | <= 5 MB drift | NOT MEASURED; soak not run |

## Top Three Gaps

1. **Widget rasterization is over budget.** The 512x512 path is roughly 3.7-4.8 ms mean on reference hardware, missing both the new 1 ms target and the older 2 ms target. Suspected causes: repeated SVG parse/rasterize cost in `resvg`/`tiny-skia`, text binding work, and missing warm-path parse/cache separation.
2. **Windowed overlay performance is not benchmarkable yet.** There is no current harness that compares fullscreen vs transparent overlay p99 frame cost. Suspected cause: windowed telemetry is collected internally but not exported as a bounded benchmark artifact.
3. **Reference live-workload config is not benchmark-ready.** gRPC widget publishing is blocked by missing `publish_widget:main-progress` capability, MCP zone publishes failed, and no three-agent soak runner emits the required metrics. Suspected cause: user-test deployment config is optimized for manual demos, not performance harness identity/capability setup.

## Notes

`examples/benchmark` had drifted from the current `HeadlessRuntime` API and did not compile with `--features headless` at the start of this work. The harness was updated so the Windows benchmark binary can be built and run again.
