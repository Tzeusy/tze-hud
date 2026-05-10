# Windows Runtime Reconciliation Preflight - 2026-05-11

Issue: `hud-iygbd`
Epic: `hud-9wljr`
OpenSpec source: `openspec/changes/windows-first-performant-runtime/`

## Verdict

Blocked, but the remaining blockers are now explicit and scheduled in Beads.

This preflight does not close the Windows-first runtime reconciliation. It records
the current requirement-to-evidence map before the final live strict smoke and
60-minute soak can resume. The final reconciliation still depends on:

- `hud-9m47l` - restore TzeHouse reachability for live strict smoke and soak.
- `hud-egd2j` - obtain eligible non-author approvals for PRs #648, #650, #651,
  and #652.
- `hud-nfl7n` - rerun the full 60-minute three-agent Windows soak after the
  strict-smoke blockers are cleared.

## Scope

The active direction remains Windows-first. The delta spec narrows current work
to the native Windows HUD runtime, defers media/mobile/embodied scope, and
requires reference-hardware budget calibration before budgets are enforced.
The parallel cooperative HUD projection lane in section 6 of the task list is
already closed and archived; it is not part of the remaining Windows soak
blocker set.

## Requirement Map

| Requirement / task family | Evidence | Status |
|---|---|---|
| Windows-first active runtime scope | `about/heart-and-soul/v1.md`, `openspec/changes/windows-first-performant-runtime/proposal.md`, `hud-5qspb` / PR #628 | Landed |
| Deferred media and multi-device contracts | `v2.md`, `mobile.md`, `media-doctrine.md`, deferred media specs, `hud-5qspb` / PR #628 | Landed |
| Reference hardware baseline | `docs/reports/windows_perf_baseline_2026-05.md`, `hud-1753c` / PR #630 | Landed |
| Locked Windows budgets and CI gate | `about/craft-and-care/engineering-bar.md`, `scripts/ci/check_windows_perf_budgets.py`, `hud-1vgkk` / PR #637 | Landed |
| Benchmark-ready Windows config and launch path | `app/tze_hud_app/config/benchmark.toml`, `scripts/windows/install_benchmark_hud_task.ps1`, `hud-l7x8f` / PR #635 | Landed |
| Overlay performance harness | `hud-3atp0` / PR #633 | Landed, but final reference-host overlay delta remains part of release evidence |
| Widget raster optimization | `hud-8qkr0`, `hud-9wljr.1`, `hud-9wljr.2`, `hud-eeejt`, open `hud-vzvna` / PR #648 | Blocked on eligible approval |
| Long-run widget publish response draining | `hud-qivb5` / PR #649 | Landed |
| Live frame/input metrics in soak artifact | `hud-wydpo` / PR #650 | Blocked on eligible approval |
| Nonzero input-latency strict smoke | `hud-9wljr.3` / PR #652 | Blocked on eligible approval and TzeHouse reachability |
| Benchmark HUD process resource sampling | `hud-9wljr.4` / PR #651 | Blocked on eligible approval and TzeHouse reachability |
| Full 60-minute three-agent Windows soak | `hud-nfl7n` | Blocked on `hud-9m47l`, `hud-egd2j`, `hud-wydpo`, `hud-9wljr.3`, and `hud-9wljr.4` |
| Cooperative HUD projection completion | `docs/reports/cooperative_hud_projection_gen2_reconciliation_20260510.md`, `openspec/changes/archive/2026-05-10-cooperative-hud-projection/`, `hud-ggntn.7`, `hud-ggntn.10`, `hud-ggntn.11`, `hud-ggntn.12` | Landed and archived |
| Release tag / artifact with perf report | `openspec/changes/windows-first-performant-runtime/tasks.md` section 5.3 | Not ready; depends on successful `hud-nfl7n` |
| Archive `windows-first-performant-runtime` | `openspec/changes/windows-first-performant-runtime/tasks.md` section 5.4 | Not ready; depends on reconciliation and release decision |

## Current Blockers

`hud-9m47l` tracks the renewed TzeHouse outage. Fresh probes timed out for
Tailscale ping, non-interactive SSH as `tzeus`, and TCP ports `22`, `50051`, and
`9090`. This blocks the live strict smoke and full soak.

`hud-egd2j` tracks the approval gate. PRs #648, #650, #651, and #652 are open
and mergeable/CLEAN, but `reviewDecision` is empty. The authenticated GitHub
identity is the PR author, so this session cannot self-approve.

## Completion Criteria Remaining

The serious soak/reconciliation cycle is complete only after all of the
following are true:

1. PR #648, #650, #651, and #652 are approved, mergeable, and integrated in the
   correct order.
2. TzeHouse reachability is restored and verified through Tailscale, SSH, gRPC
   `:50051`, and MCP `:9090`.
3. A strict short smoke proves `live_metrics.ok=true`, nonzero input-latency
   triple samples, and resource samples with `process_count >= 1`.
4. `hud-nfl7n` produces a valid 60-minute three-agent soak artifact with
   accepted publish artifacts, frame-time metrics, input latency, resource
   samples, memory drift, jitter/failure observations, and cleanup evidence.
5. `hud-iygbd` closes with a final requirement-to-evidence reconciliation and a
   release/archive decision.

## Direction

No new implementation gap was discovered in this preflight beyond the blockers
already scheduled by `hud-9m47l` and `hud-egd2j`. Cooperative HUD projection
section 6 has separate closeout evidence and does not need new direction work.
If either live strict smoke or the full soak fails after those blockers clear,
create focused child beads under `hud-9wljr` using the failing artifact as
evidence, then rerun `hud-iygbd`.
