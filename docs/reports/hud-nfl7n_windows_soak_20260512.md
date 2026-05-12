# hud-nfl7n Windows Soak Evidence - 2026-05-12

Issue: `hud-nfl7n`
Host: `TzeHouse` (`tzehouse-windows.parrot-hen.ts.net`, `100.87.181.125`)
Runtime config: `C:\tze_hud\benchmark.toml` via `TzeHudBenchmarkOverlay`

## Verdict

Blocked for Windows release tag and `windows-first-performant-runtime` archive.

The 60-minute three-agent widget soak completed successfully: `agent-alpha`,
`agent-beta`, and `agent-gamma` each published `3600/3600` accepted
`main-progress` widget updates with zero errors. Live frame/input metrics were
present, resource sampling found the benchmark HUD process on every sample, and
private-memory drift was below the 5 MiB gate.

Release/archive is still blocked because the transparent-overlay composite
cost harness did not emit the fullscreen-vs-overlay report, idle GPU evidence
missed the locked budget, and the completed workload is the canonical
`widget_soak_runner.py` widget workload rather than a combined scene/widget/zone
resident soak with lease cleanup proof. The per-agent RTT p99 values are low for
this 1 rps workload, but each agent also recorded a single max RTT outlier near
3.8 seconds; that should be classified before treating the run as latency-clean.

## Reference Hardware

Reference identity follows `docs/reports/windows_perf_baseline_2026-05.md`:

| Surface | Value |
|---|---|
| Host | `TzeHouse` |
| OS | Windows 11 Pro `10.0.26200`, build `26200` |
| CPU | Intel i5-13600KF, 14 cores / 20 logical processors |
| GPU | NVIDIA GeForce RTX 3080 |
| RAM | 16 GiB |
| Display | 4096x2160 at 60 Hz |

## Source And Deployment

| Item | Value |
|---|---|
| Branch head before this report | `9339c181ac602064ca4f713b51a124235e434b9d` |
| Base `origin/main` | `97b1aec67eeefc09e00645a877901cfdb34f8547` |
| Windows app SHA-256 | `e9a3652deae1adbc6331f7bfe5788520e199b637349920a83483b5b42d706c88` |
| Local harness SHA-256 | `7c8bb6cdf7fca65f80c71a091447e99defbf6b802bd8b265a4df60eb850d6d04` |
| Remote executable backup | `C:\tze_hud\tze_hud.exe.pre-hud-nfl7n-20260512T011115Z` |

The deployed `C:\tze_hud\tze_hud.exe` initially rejected
`--benchmark-emit`, so the current Windows app was cross-built and deployed to
the benchmark host. The prior executable was backed up first. No PSK value was
printed or written to repo artifacts; authenticated commands loaded the
benchmark task's DPAPI-protected PSK into process memory only.

## Pre-Run Gates

Immediately before the run:

- Tailscale peer was online and `tailscale ping` returned `pong` in 7 ms.
- Non-interactive SSH succeeded for both `tzeus` and `hudbot` using
  `~/.ssh/ecdsa_home`.
- TCP ports `22`, `50051`, and `9090` were open.
- MCP `/mcp` returned HTTP 200 and authenticated widget/zone discovery/publish
  smokes succeeded with the benchmark PSK.
- gRPC authenticated and established a session; the generic self-test then
  denied a lease because its hardcoded `grpc-self-test` agent is unregistered.
  The soak used registered benchmark agents.

## Commands

Live metrics source:

```powershell
C:\tze_hud\tze_hud.exe `
  --config C:\tze_hud\benchmark.toml `
  --window-mode overlay `
  --grpc-port 0 `
  --mcp-port 0 `
  --benchmark-emit C:\tze_hud\perf\hud-nfl7n\windowed_live_metrics.json `
  --benchmark-frames 600 `
  --benchmark-warmup-frames 120
```

Soak:

```bash
python3 .claude/skills/user-test-performance/scripts/widget_soak_runner.py \
  --target-id user-test-windows-tailnet \
  --duration-s 3600 \
  --rate-rps 1 \
  --windows-live-metrics-path 'C:\tze_hud\perf\hud-nfl7n\windowed_live_metrics.json' \
  --sample-windows-resources \
  --windows-process-command-match 'C:\tze_hud\benchmark.toml' \
  --resource-sample-interval-s 300 \
  --win-user tzeus \
  --ssh-identity ~/.ssh/ecdsa_home \
  --skip-build \
  --output-root docs/reports/artifacts/hud-nfl7n-soak-20260512T012037Z
```

Overlay-composite attempt:

```powershell
C:\tze_hud\windowed-fullscreen-overlay-perf.ps1 `
  -ExePath C:\tze_hud\tze_hud.exe `
  -OutputDir C:\tze_hud\perf\hud-nfl7n\overlay-cost `
  -Frames 600 `
  -WarmupFrames 120 `
  -TargetDeltaUs 500 `
  -FailOnBudget
```

## Artifact Index

- `docs/reports/artifacts/hud-nfl7n-soak-20260512T012037Z/soak_summary.json`
- `docs/reports/artifacts/hud-nfl7n-soak-20260512T012037Z/agents/agent-alpha.json`
- `docs/reports/artifacts/hud-nfl7n-soak-20260512T012037Z/agents/agent-beta.json`
- `docs/reports/artifacts/hud-nfl7n-soak-20260512T012037Z/agents/agent-gamma.json`
- `docs/reports/artifacts/hud-nfl7n-soak-20260512T012037Z/live_metrics_source.json`
- `docs/reports/artifacts/hud-nfl7n-soak-20260512T012037Z/live_metrics_summary.json`
- `docs/reports/artifacts/hud-nfl7n-soak-20260512T012037Z/cleanup_widget_evidence.jsonl`
- `docs/reports/artifacts/hud-nfl7n-soak-20260512T012037Z/overlay-cost/overlay-cost-task.log`
- `docs/reports/artifacts/hud-nfl7n-soak-20260512T012037Z/follow_ups.json`

## Soak Results

| Metric | Result |
|---|---:|
| Start | `2026-05-12T01:20:37+00:00` |
| End | `2026-05-12T02:20:39+00:00` |
| Agents | 3 |
| Planned publishes | 10,800 |
| Accepted publishes | 10,800 |
| Errors | 0 |
| Per-agent return codes | 0 / 0 / 0 |
| `WIDGET_CAPABILITY_MISSING` | 0 |
| Private-memory drift | +1,634,304 bytes (+1.56 MiB) |
| Working-set drift | -255,520,768 bytes (-243.68 MiB) |
| Resource samples | 13, all `process_count=1` |

Per-agent summary:

| Agent | Requests | Success | Errors | RTT p50 | RTT p99 | Throughput |
|---|---:|---:|---:|---:|---:|---:|
| `agent-alpha` | 3600 | 3600 | 0 | 10,676 us | 57,690 us | 1.000 rps |
| `agent-beta` | 3600 | 3600 | 0 | 10,648 us | 53,925 us | 1.000 rps |
| `agent-gamma` | 3600 | 3600 | 0 | 10,488 us | 47,467 us | 1.000 rps |

Each agent artifact includes the harness's normal stream-terminal warning after
`SessionClose`: `stream closed while waiting for WidgetPublishResult acks`.
Because each agent reports `success_count == request_count == 3600` and
`error_count == 0`, no unexplained missing durable ack is indicated by this run.
Each agent also recorded one max RTT outlier near 3.8 seconds despite p99 staying
below 58 ms; this remains a jitter follow-up rather than accepted release
evidence.

## Live Metrics

Live metrics were loaded from the windowed compositor benchmark artifact:

| Metric | Sample Count | p50 | p95 | p99 | p99.9 |
|---|---:|---:|---:|---:|---:|
| Frame time | 600 | 1,718 us | n/a | 5,918 us | 8,396 us |
| `input_to_local_ack` | 600 | 4 us | 12 us | 17 us | n/a |
| `input_to_scene_commit` | 600 | 13,436 us | 15,239 us | 27,002 us | n/a |
| `input_to_next_present` | 600 | 15,683 us | 18,037 us | 28,632 us | n/a |

The summary reports `live_metrics.ok=true` and `missing_metrics=[]`.

## Resource Samples

The runner sampled the benchmark-config HUD process with
`--windows-process-command-match 'C:\tze_hud\benchmark.toml'`; every sample had
`process_count=1`.

GPU samples from `nvidia-smi` ranged from 14% to 54%, average 24.38%. The
post-soak sample was 16%. That is useful loaded/idle evidence, but it does not
meet the locked idle GPU target from the baseline report (`<= 0.5%`) and remains
a release blocker.

Private-memory drift passed the 5 MiB gate:

```text
start private bytes: 296,329,216
end private bytes:   297,963,520
delta:               +1,634,304 bytes (+1.56 MiB)
```

## Cleanup Evidence

`cleanup_widget_evidence.jsonl` proves `main-progress` was reset after the soak:

- Before clear: `progress=1.0`, `label=""`.
- Clear responses: four `clear_widget` requests returned `cleared=true` for
  agent namespaces plus `user-test`.
- After clear: `progress=0.0`, `label=""`.

This is widget stale-UI cleanup evidence. It is not a full scene lease cleanup
proof because the completed soak did not create leased scene tiles.

## Blocking Follow-Ups JSON

```json
[
  {
    "title": "Fix or rerun TzeHouse fullscreen-vs-overlay composite harness",
    "type": "bug",
    "priority": 1,
    "depends_on": "hud-nfl7n",
    "rationale": "The reference-host overlay composite harness failed before writing windowed_fullscreen_vs_overlay_report.json; hud-nfl7n still lacks the required < +0.5 ms p99 overlay delta.",
    "unblock_condition": "A TzeHouse artifact records fullscreen and overlay p99 frame times plus composite_delta.p99_us <= 500, or a focused performance bug explains the regression."
  },
  {
    "title": "Resolve idle GPU budget evidence for benchmark HUD",
    "type": "bug",
    "priority": 1,
    "depends_on": "hud-nfl7n",
    "rationale": "The completed soak captured process_count=1 resource samples, but nvidia-smi GPU utilization remained 14%-54% with a 16% post-soak sample, above the locked <=0.5% idle GPU target.",
    "unblock_condition": "A clean idle reference-host sample shows HUD idle GPU within budget, or a tracked fix recalibrates/remediates the idle GPU cost with comparable evidence."
  },
  {
    "title": "Close scene/zone/lease coverage gap for release soak",
    "type": "task",
    "priority": 1,
    "depends_on": "hud-nfl7n",
    "rationale": "The completed 60-minute run used the canonical widget_soak_runner.py path and proved widget publish/cleanup only; it did not concurrently exercise resident scene tiles, zone publishing, or lease cleanup.",
    "unblock_condition": "Either the release criterion is explicitly narrowed to the widget soak plus separate MCP smokes, or a comparable 60-minute resident workload covers scenes, widgets, zones, and lease cleanup."
  },
  {
    "title": "Classify 60-minute soak RTT max outliers",
    "type": "task",
    "priority": 2,
    "depends_on": "hud-nfl7n",
    "rationale": "The soak completed with p99 RTT under 58 ms and zero errors, but each agent recorded one max RTT near 3.8 seconds; classify whether these were shutdown/drain artifacts, transient host stalls, or material jitter.",
    "unblock_condition": "A follow-up analysis or rerun explains/removes the 3.8 second max RTT outliers and states whether they affect release latency confidence."
  }
]
```

## Release Decision

Do not create the Windows release tag yet. Do not archive
`windows-first-performant-runtime` yet.

The soak fixed the previous missing-artifact blocker and proves successful
three-agent widget publishing over 60 minutes, but release/archive remains
blocked on overlay composite cost, idle GPU budget evidence, scene/zone/lease
workload coverage, and RTT outlier classification.
