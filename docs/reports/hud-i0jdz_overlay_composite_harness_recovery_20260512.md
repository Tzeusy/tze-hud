# hud-i0jdz Overlay Composite Harness Recovery - 2026-05-12

Issue: `hud-i0jdz`

## Status

Blocked on coordinated TzeHouse access for the live rerun.

Worker D did not restart, reconfigure, or benchmark the shared TzeHouse HUD
because Worker A owns concurrent live Windows media validation. The local
harness failure was investigated and the harness was patched so the next live
run can produce actionable artifacts.

## Root Cause

The previous overlay-composite attempt wrote only:

```text
[windowed-perf] Running fullscreen benchmark...
exception=fullscreen benchmark failed with exit code
```

No `fullscreen.json`, `overlay.json`, or
`windowed_fullscreen_vs_overlay_report.json` was copied into
`docs/reports/artifacts/hud-nfl7n-soak-20260512T012037Z/overlay-cost/`.

The app binary is built with `#![windows_subsystem = "windows"]`. Launching it
from PowerShell with `& $ExePath @args` and then reading `$LASTEXITCODE` is not
a reliable harness boundary for this GUI-subsystem benchmark path: the wrapper
can observe an empty exit-code value before it has a useful per-mode artifact or
diagnostic log. The script now uses `Start-Process -Wait -PassThru` and captures
per-mode stdout/stderr before checking exit code and artifact existence.

## Local Fix

Patched `scripts/ci/windows/windowed-fullscreen-overlay-perf.ps1` to:

- launch each mode with `Start-Process -Wait -PassThru`
- read `$process.ExitCode` instead of `$LASTEXITCODE`
- write `logs/fullscreen.stdout.log`, `logs/fullscreen.stderr.log`,
  `logs/overlay.stdout.log`, and `logs/overlay.stderr.log`
- include log paths in nonzero-exit and missing-artifact errors

Added `scripts/ci/test_windowed_overlay_perf_script.py` to pin the harness
contract.

## Required Live Rerun

Run only after the coordinator confirms Worker A has released the TzeHouse
runtime/GPU lane.

```powershell
C:\tze_hud\windowed-fullscreen-overlay-perf.ps1 `
  -ExePath C:\tze_hud\tze_hud.exe `
  -OutputDir C:\tze_hud\perf\hud-i0jdz\overlay-cost `
  -Frames 600 `
  -WarmupFrames 120 `
  -TargetDeltaUs 500 `
  -FailOnBudget
```

Then copy the output directory into a repo artifact path, for example:

```text
docs/reports/artifacts/hud-i0jdz-overlay-cost-<timestamp>/
```

The unblock artifact must include:

- `windowed_fullscreen_vs_overlay_report.json`
- `fullscreen.json`
- `overlay.json`
- `logs/fullscreen.stdout.log`
- `logs/fullscreen.stderr.log`
- `logs/overlay.stdout.log`
- `logs/overlay.stderr.log`

`hud-i0jdz` can close only if
`windowed_fullscreen_vs_overlay_report.json` records both fullscreen and overlay
p99 frame times and `composite_delta.p99_us <= 500`. If the rerun produces a
valid report with `composite_delta.p99_us > 500`, keep this bead blocked and
open a focused compositor performance bug with the report attached.
