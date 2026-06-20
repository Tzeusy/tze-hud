# hud-i0jdz — TzeHouse fullscreen-vs-overlay composite harness: hang reproduced

- Date: 2026-06-20
- Bead: `hud-i0jdz` (follow-up from `hud-nfl7n`)
- Host: `tzehouse-windows.parrot-hen.ts.net`, exe `C:\tze_hud\tze_hud.exe` (main, 06-20 build)
- Harness: `scripts/ci/windows/windowed-fullscreen-overlay-perf.ps1` (fix `d0df9fd2`)
- GPU: freed (prod `TzeHudOverlay` stopped) before each run.

## Result: harness hangs in the FULLSCREEN benchmark pass — no report produced

The composite harness reaches `"[windowed-perf] Running fullscreen benchmark..."`
and then **never completes the fullscreen pass**, so
`windowed_fullscreen_vs_overlay_report.json` is never written (the exact
`hud-i0jdz` symptom).

Observations:
- The fullscreen `tze_hud.exe` benchmark child (PID 22600) ran for **7+ minutes**
  accumulating **~291 CPU-seconds** (~70% of one core) without exiting and
  without producing `fullscreen.json`.
- Its `fullscreen.stdout.log` and `fullscreen.stderr.log` were **empty** — the
  hang is **silent** (no error, no progress output).
- Only `windowed-benchmark.toml` was produced in the output dir; neither
  `fullscreen.json` nor the report.

## Root cause is NOT the harness wrapper

The harness launches each benchmark with `Start-Process -RedirectStandardOutput`
(which sets `CREATE_NO_WINDOW`). To rule that out, the fullscreen benchmark was
also launched **directly with no redirect** (`--window-mode fullscreen
--benchmark-frames 120 --benchmark-warmup-frames 30 --benchmark-emit ...`):

- Still **no emit** (`emit_exists=False`) within 90 s; process had to be killed.

So the fullscreen windowed **benchmark mode itself hangs** on TzeHouse — it
renders/burns CPU but the measured-frame counter never reaches the target and the
emit never fires — regardless of launch method. This is a focused
compositor/benchmark bug, not a harness-script defect.

## Unblock path

Per the bead's unblock condition ("…or a focused performance bug explains the
regression"), this is captured as a focused bug. `hud-i0jdz` (the rerun) stays
blocked until the benchmark hang is fixed; once it completes, the same harness +
the exclusive-GPU-window pattern proven this session can produce the
`composite_delta.p99_us <= 500` artifact.

### Suggested investigation for the fix

- Determine whether the benchmark frame counter advances on `RedrawRequested` /
  present-complete callbacks that never fire when the fullscreen window is not
  the foreground/visible surface in the session (the harness session may not own
  the foreground), causing the 600-frame target to never be reached.
- Add a benchmark watchdog: if N seconds elapse with < expected frames, emit a
  partial report + non-zero exit instead of hanging silently.

## Artifacts

- `harness-run.log` — harness progress (stops at "Running fullscreen benchmark...").
