# hud-i0jdz TzeHouse fullscreen-vs-overlay rerun — 2026-07-16

## Outcome

The canonical live reference-host rerun did not reach the overlay pass and did
not evaluate the locked `composite_delta.p99_us <= 500` budget. The fullscreen
benchmark presented one warmup frame, then the no-progress watchdog fired after
30 seconds:

- `warmup_frames_seen`: 1 of 120
- `measured_frames_seen`: 0 of 600
- `recorded_frames`: 0
- `watchdog_abort.reason`: `no-progress timeout`

No `overlay.json` or `windowed_fullscreen_vs_overlay_report.json` was produced.
This is a failed measurement, not a performance pass, and the budget was not
weakened or bypassed.

Independent production-call-site review found a second precondition for the
eventual rerun. The harness currently forwards its default explicit dimensions
(`--width 1920 --height 1080`) to both modes. Fullscreen ignores those configured
dimensions and uses the monitor, while the explicit flags disable overlay
auto-sizing. This run's fullscreen artifact reports `3840x2160`; an unchanged
post-fix rerun would therefore compare unequal pixel workloads. No invalid delta
was emitted here because overlay never started. Before a future delta is
accepted, the harness must make both effective surfaces equal and fail closed if
the emitted dimensions differ. The separate focused P1 proposal is recorded in
`follow_ups.json`.

## Traceable run identity

- Assigned source HEAD: `09f69c6d2d82b065297602d44b4e50e97957f7a9`
- Cross-compiled release executable SHA-256:
  `6f039ffb677ebcea317a4486263afa956ccc9bf7ba97c456b4419da4725bfedc`
- Executable size: 24,927,358 bytes
- Harness SHA-256:
  `b61959fef1c5031884dc5f30696d9541a6cd6ea46320b47d17d75ea6061d9306`
- Reference tag: TzeHouse (`windows-host.example`), active console session
- Remote artifact root:
  `C:\tze_hud\perf\hud-i0jdz\run-20260716T131837Z\overlay-cost`

The executable and harness were copied with the file-user SSH role and verified
remotely by SHA-256 before the production HUD was stopped. Both SSH role gates
and MCP `initialize` returned successfully before the run.

The harness ran through a temporary interactive scheduled task so both
fullscreen and overlay child processes inherited the unlocked console desktop.
This removes the non-interactive SSH window-station/foreground hypothesis from
the earlier 2026-06-20 failure.

Canonical harness arguments:

```powershell
powershell -NoProfile -NonInteractive -ExecutionPolicy Bypass `
  -File C:\tze_hud\perf\hud-i0jdz\run-20260716T131837Z\windowed-fullscreen-overlay-perf.ps1 `
  -ExePath C:\tze_hud\perf\hud-i0jdz\run-20260716T131837Z\tze_hud.exe `
  -OutputDir C:\tze_hud\perf\hud-i0jdz\run-20260716T131837Z\overlay-cost `
  -Frames 600 `
  -WarmupFrames 120 `
  -TargetDeltaUs 500 `
  -FailOnBudget
```

## Root cause

The failure is a deterministic conflict between benchmark mode and the
production idle-render gate in `crates/tze_hud_runtime/src/windowed/mod.rs`:

1. `composite_tiles_v1` is seeded as a static scene.
2. The first compositor pass renders because the last-rendered version and
   geometry epoch start at `u64::MAX`.
3. After that present, the cached version/epoch match the unchanged scene;
   there is no in-flight animation or focused composer, so `dirty` is false.
4. The idle branch skips build/encode/present, which is correct for production
   idle efficiency.
5. `WindowedBenchmarkRunState::record()` is nested inside the successful
   present branch. With no later present, benchmark progress remains at one
   warmup frame until the watchdog aborts.

The live `fullscreen.json` is exact causal evidence for that path: one warmup
frame landed, then no other frame reached `record()`. The watchdog added for
the earlier focused bug converts the infinite hang into a diagnostic exit, but
does not make the benchmark workload advance.

The correct follow-up is to keep the normal-runtime idle gate intact while
making benchmark mode deterministically produce the requested sample frames.
That behavior change needs a red-green regression test before implementation.
The structured follow-up proposals are in `follow_ups.json`; the runtime fix and
the equal-surface harness guard remain separate focused changes.

## Cleanup and production restoration

The exclusive GPU lane was released before handoff:

- temporary benchmark scheduled task removed
- no staged `tze_hud.exe` process remains
- a one-off controller PowerShell wrapper that stalled while serializing its
  final report was killed only after restoration; it was in the Services
  session and was not the HUD
- `TzeHudOverlay` is `Running`
- restored HUD PID `34996` runs in Console session 1
- PID `34996` owns `0.0.0.0:50051` and `0.0.0.0:9090`
- `gpu.lock` names PID `34996`
- both SSH roles pass after restoration
- MCP `initialize` returns HTTP 200 after restoration

Machine-readable run and restoration state is in `lane_state.json`.

## Files

- `fullscreen.json` — watchdog diagnostic emitted by the runtime
- `logs/fullscreen.stdout.log` — runtime startup output
- `logs/fullscreen.stderr.log` — non-zero benchmark exit diagnostic
- `windowed-benchmark.toml` — generated runtime configuration (content-preserving
  copy; Windows line endings normalized)
- `lane_state.json` — run identity, causal classification, and restoration proof
- `follow_ups.json` — structured focused bug proposals

The current app maps any benchmark watchdog failure to the generic stderr text
`windowed benchmark artifact write failed`. In this run the partial artifact did
write successfully; its committed contents match the remote JSON semantically
(the copy normalized Windows line endings). Treat the stderr line as the
expected non-zero watchdog exit, not as evidence that `fullscreen.json` failed
to write.

## Independent restoration revalidation

At `2026-07-16T13:51:14Z`, the PR reviewer re-ran read-only live gates. The
staged executable and harness still matched the recorded SHA-256 values; the
committed fullscreen JSON, config, and per-mode logs matched the retained remote
artifacts semantically; overlay and combined-report files were still absent.
The same restored production PID `34996` was still running in the interactive
desktop session, owned both listeners, and matched the live GPU lock. No staged
HUD process or temporary benchmark task remained, both SSH roles passed, and MCP
`initialize` returned HTTP 200. The live CPU/GPU/OS profile matched the TzeHouse
reference identity. No secret-bearing task XML or private target identity was
copied into this evidence tree.
