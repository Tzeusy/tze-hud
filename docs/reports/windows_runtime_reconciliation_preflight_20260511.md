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
| Widget raster optimization | `hud-8qkr0`, `hud-9wljr.1`, `hud-9wljr.2`, `hud-eeejt`, `hud-vzvna` / PR #648; PR #648 TzeHouse upper estimates: 0.514 ms numeric/color changing, 0.788 ms text-changing | Landed; final release evidence remains part of soak closeout |
| Long-run widget publish response draining | `hud-qivb5` / PR #649 | Landed |
| Live frame/input metrics in soak artifact | `hud-wydpo` / PR #650 | Landed; live validation remains part of strict smoke and soak |
| Nonzero input-latency strict smoke | `hud-9wljr.3` / PR #652 | Implementation landed; blocked on TzeHouse reachability for live proof |
| Benchmark HUD process resource sampling | `hud-9wljr.4` / PR #651 | Implementation landed; blocked on TzeHouse reachability for live proof |
| Full 60-minute three-agent Windows soak | `hud-nfl7n` | Blocked on `hud-9m47l` |
| TzeHouse out-of-band recovery procedure | `docs/operations/tzehouse-windows-recovery.md`, `hud-9wljr.6` | Documented; current supported route is operator/manual recovery, then verification probes |
| Cooperative HUD projection completion | `docs/reports/cooperative_hud_projection_gen2_reconciliation_20260510.md`, `openspec/changes/archive/2026-05-10-cooperative-hud-projection/`, `hud-ggntn.7`, `hud-ggntn.10`, `hud-ggntn.11`, `hud-ggntn.12` | Landed and archived |
| Release tag / artifact with perf report | `openspec/changes/windows-first-performant-runtime/tasks.md` section 5.3 | Not ready; depends on successful `hud-nfl7n` |
| Archive `windows-first-performant-runtime` | `openspec/changes/windows-first-performant-runtime/tasks.md` section 5.4 | Not ready; depends on reconciliation and release decision |

## Current Blockers

`hud-9m47l` tracks the renewed TzeHouse outage. The latest refresh at
`2026-05-10T19:43:19Z` found `tzehouse-windows.parrot-hen.ts.net` still
`Online=false` in Tailscale, with `LastSeen=2026-05-10T17:06:32.1Z` and IP
`100.87.181.125`. Fresh probes timed out for Tailscale ping, non-interactive
SSH as `tzeus`, and TCP ports `22`, `50051`, and `9090`. This blocks the live
strict smoke and full soak.

The out-of-band recovery planning gap is closed by `hud-9wljr.6` and
`docs/operations/tzehouse-windows-recovery.md`. The repository still does not
define a safe Wake-on-LAN, Synology-mediated, router, or BIOS remote wake
procedure; the supported route for the current outage is manual/operator host
recovery followed by the documented Tailscale, SSH, TCP, MCP, and gRPC probes.

The former approval blocker `hud-egd2j` is closed. Branch protection for `main`
requires status checks but no pull-request reviews, and PRs #648, #650, #651,
and #652 have been merged.

## Completion Criteria Remaining

The serious soak/reconciliation cycle is complete only after all of the
following are true:

1. TzeHouse reachability is restored and verified through Tailscale, SSH, gRPC
   `:50051`, and MCP `:9090`.
2. A strict short smoke proves `live_metrics.ok=true`, nonzero input-latency
   triple samples, and resource samples with `process_count >= 1`.
3. `hud-nfl7n` produces a valid 60-minute three-agent soak artifact with
   accepted publish artifacts, frame-time metrics, input latency, resource
   samples, idle GPU budget evidence, transparent-overlay composite delta,
   memory drift, jitter/failure observations, and cleanup evidence.
4. `hud-iygbd` closes with a final requirement-to-evidence reconciliation and a
   release/archive decision.

## Prompt-to-Artifact Checklist

| Objective requirement | Required artifact or gate | Current evidence | Status |
|---|---|---|---|
| Run the serious soak cycle | 60-minute `hud-nfl7n` three-agent soak artifact under `docs/reports/` or `docs/evidence/` | `hud-nfl7n` acceptance criteria and dry-run notes prove the command shape, including `user-test-windows-tailnet`, 3600s, 1 rps, `main-progress`, required live metrics, resource sampling, and `--windows-process-command-match 'C:\tze_hud\benchmark.toml'` | Blocked by `hud-9m47l` |
| Prove host readiness before soak | Tailscale, non-interactive SSH, gRPC `:50051`, MCP `:9090`, widget/zone discovery, and gRPC smoke | `docs/operations/tzehouse-windows-recovery.md`; latest `hud-9m47l` notes record the `2026-05-10T19:43:19Z` Tailscale offline state plus Tailscale ping, SSH, and TCP probe timeouts | Incomplete |
| Prove strict smoke metrics | Short smoke with `live_metrics.ok=true`, nonzero input-latency buckets, and `process_count >= 1` resource samples | Implementations landed via PR #650/#651/#652; local unit/config/dry-run validation is recorded on `hud-nfl7n` | Awaiting live host |
| Prove release-quality soak metrics | Accepted publish counts, frame-time p50/p99/p99.9, input latency, resource samples, idle GPU, private-memory drift, transparent-overlay composite delta, jitter/failure observations, and cleanup evidence | `hud-nfl7n` acceptance criteria and benchmark launch docs enumerate the required evidence | Awaiting live host |
| Reconcile requirements to implementation | Final `hud-iygbd` closeout mapping every windows-first task/design budget to evidence or tracked gap | This preflight report maps current evidence and blockers; final closeout depends on `hud-nfl7n` | Incomplete |
| Use `/project-direction` for gaps | Focused Beads for any uncovered or failed requirement | Known soak/reconciliation gaps are scheduled as `hud-9m47l`, `hud-nfl7n`, and `hud-iygbd`; CI maintenance is tracked by deferred `hud-s4m8w`; Beads backup durability is tracked by deferred `hud-qdeh8`. If strict smoke or soak fails after recovery, create child beads under `hud-9wljr` with the failing artifact. | Satisfied for known gaps |
| Keep handoff durable | Pushed docs plus Beads notes/export state | Recovery, benchmark, and AGENTS handoff docs are pushed. Local `bd export --no-memories` includes current Beads notes, but `.beads/issues.jsonl` and `.beads/backup/` are ignored by git, `bd dolt remote list` reports no remote, and `bd backup sync` fails without a configured destination. The tracked-doc handoff is durable; Beads remote durability remains `hud-qdeh8`. | Partially satisfied; infrastructure gap scheduled |

## Direction

No new soak implementation or release-planning gap remains unscheduled.
Cooperative HUD projection section 6 has separate closeout evidence and does not
need new direction work. The TzeHouse recovery-procedure gap is documented and
closed as `hud-9wljr.6`; Beads remote durability is separately scheduled as
`hud-qdeh8` and does not unblock live soak evidence. If either live strict smoke
or the full soak fails after TzeHouse reachability is restored, create focused
child beads under `hud-9wljr` using the failing artifact as evidence, then rerun
`hud-iygbd`.
