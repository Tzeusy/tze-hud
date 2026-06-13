# hud-7oafx Idle GPU Classification - 2026-05-12

Issue: `hud-7oafx`
Host: `TzeHouse` (`tzehouse-windows.parrot-hen.ts.net`)

## Verdict

Resolved as a release blocker by recalibrating the evidence against the current
locked engineering bar.

The release-blocking idle GPU gate is no longer the original proposed
`<= 0.5%` device-utilization target from
`openspec/changes/windows-first-performant-runtime/design.md`. The current
locked bar in `about/craft-and-care/engineering-bar.md` is:

- Idle GPU, overlay mode, no agents: `<= 4.0%` Windows GPU engine sum
  regression ceiling.
- `<= 0.5%` remains aspirational after cleaner sampling and tuning.

The committed TzeHouse idle overlay sample in
`docs/reports/windows_perf_baseline_2026-05.md` measured `3.791%` Windows GPU
engine utilization sum with no benchmark client running. That is below the
locked `<= 4.0%` release ceiling and should be treated as pass-level idle GPU
evidence for the current Windows-first release gate.

## Inputs Reviewed

- `docs/reports/windows_perf_baseline_2026-05.md`
- `docs/reports/hud-nfl7n_windows_soak_20260512.md`
- `docs/reports/artifacts/hud-nfl7n-soak-20260512T012037Z/soak_summary.json`
- `docs/reports/artifacts/hud-nfl7n-soak-20260512T012037Z/follow_ups.json`
- `docs/reports/hud-i0jdz_tzehouse_rerun_blocked_20260512.md`
- `docs/reports/artifacts/hud-i0jdz-rerun-blocked-20260512T030529Z/lane_status.json`
- `.claude/skills/user-test-performance/scripts/widget_soak_runner.py`
- `about/craft-and-care/engineering-bar.md`

## Evidence Classification

The `hud-nfl7n` soak samples prove the resource sampler found the
benchmark-config HUD process on every sample (`process_count=1`). They do not,
by themselves, prove idle GPU budget conformance:

- `widget_soak_runner.py` records `gpu_csv` from
  `nvidia-smi --query-gpu=utilization.gpu,memory.used`, which is a whole-device
  snapshot, not per-process GPU attribution.
- The sampled `hud-nfl7n` window spans the active 60-minute three-agent widget
  workload at 1 rps per agent, so the `before`/`during-*` values are loaded
  evidence.
- The `after` value was still a whole-device `nvidia-smi` reading and cannot
  distinguish compositor idle cost from desktop/DWM/background GPU activity.

The idle budget should instead be judged against the committed idle overlay
resource sample from the baseline report:

| Metric | Evidence | Current Gate | Result |
|---|---:|---:|---|
| Idle CPU | `0.000%` total processor capacity | `<= 1%` of one core | Pass |
| Idle GPU | `3.791%` Windows GPU engine sum | `<= 4.0%` engine sum | Pass |
| Aspirational idle GPU | `3.791%` Windows GPU engine sum | `<= 0.5%` after cleaner sampling | Not met; non-blocking tuning target |

## Windows Lane Safety

No live Windows rerun or GPU workload was started for this classification.

The latest committed lane-status artifact from `hud-i0jdz` shows TzeHouse was
reachable, but the GPU lane was occupied by a benchmark-config `tze_hud.exe`
process on canonical ports and `C:\ProgramData\tze_hud\gpu.lock` pointed at a
dead PID. Starting another benchmark or clearing the lock would have violated
the dispatch constraint and could have invalidated the other worker's evidence.

## Release Impact

`hud-7oafx` can be closed once this branch is merged. It does not require a new
live GPU sample unless the project wants to pursue the aspirational `<= 0.5%`
target as a separate tuning task.

Windows release/archive remains blocked by the other explicit follow-ups from
`hud-nfl7n`, including the transparent-overlay composite-cost artifact and the
scene/zone/lease workload coverage decision. Idle GPU should no longer be listed
as a hard release blocker under the current engineering bar.

## Validation

Read-only validation commands:

```bash
jq '.resource_samples | map({
  label: .label,
  process_count: .process_count,
  process_ids: .process_ids,
  gpu_csv: .gpu_csv
})' \
  docs/reports/artifacts/hud-nfl7n-soak-20260512T012037Z/soak_summary.json

jq '.gpu_lane' \
  docs/reports/artifacts/hud-i0jdz-rerun-blocked-20260512T030529Z/lane_status.json

rg -n "Idle GPU|nvidia-smi|gpu_csv|process_count" \
  docs about .claude/skills/user-test-performance/scripts
```

No secrets were read or written, and no Beads lifecycle state was mutated.
