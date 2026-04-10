# Text Stream Portals Backlog Materialization

Date: 2026-04-10
Source artifact: `docs/reconciliations/text_stream_portals_direction_report_20260410.md`
Scope: materialize the smallest honest bead graph that turns the approved text-stream-portals doctrine and spec package into an implementation-ready epic
Materialized epic: `hud-t98e`

## Purpose

Turn the approved text-stream-portals planning package into an execution-ready, low-churn beads graph.

This backlog assumes the repo reality established by the direction report and the final spec reconciliation:

1. Doctrine, RFC 0013, and the main v1 specs now agree on the product boundary: transport-agnostic text streams, content-layer only, raw-tile pilot first.
2. The repo still lacks a real pilot implementation, especially around transcript-scroll/input wiring, a resident raw-tile portal surface, and an honest first adapter path.
3. The first implementation tranche should prove the boundary with the narrowest viable pilot rather than expand into terminal semantics or chrome-hosted UI.

## Ordering Rules

1. Land the runtime scroll/local-input seam before attempting a transcript-like portal surface.
2. Build the resident raw-tile portal surface before wiring the first adapter proof.
3. Do not claim the portal pilot works until validation covers coalescing, privacy, override, and an operator-facing adapter flow.
4. End the epic with both a reconciliation bead and a human-readable report bead.

## Materialized Bead Set

### Phase A: Runtime seam

| Bead ID | Title | Type | Priority | Depends on | Why now |
|---|---|---|---|---|---|
| hud-t98e.1 | Wire runtime scroll/input support for transcript portal tiles | task | 1 | none | The pilot cannot honestly satisfy the local-first portal interaction contract until transcript-like raw tiles can scroll and acknowledge locally. |

### Phase B: Portal surface and first adapter

| Bead ID | Title | Type | Priority | Depends on | Why now |
|---|---|---|---|---|---|
| hud-t98e.2 | Add resident raw-tile text stream portal pilot surface | feature | 1 | hud-t98e.1 | This creates the actual governed portal shape without adding new node types or chrome affordances. |
| hud-t98e.3 | Add transport-agnostic bridge and tmux pilot adapter | feature | 1 | hud-t98e.2 | The first adapter proof should ride the approved raw-tile pilot rather than inventing its own surface or runtime boundary. |

### Phase C: Validation and terminal quality gates

| Bead ID | Title | Type | Priority | Depends on | Why now |
|---|---|---|---|---|---|
| hud-t98e.4 | Add validation and user-test coverage for text stream portals | task | 1 | hud-t98e.3 | The pilot needs durable evidence for bounded viewport, governance, and the first adapter-backed operator flow. |
| hud-t98e.5 | Reconcile spec-to-code (gen-1) for phase-0 text stream portals | task | 1 | hud-t98e.1, hud-t98e.2, hud-t98e.3, hud-t98e.4 | Mandatory terminal reconciliation bead. |
| hud-t98e.6 | Generate epic report for: phase-0 text stream portals | task | 1 | hud-t98e.1, hud-t98e.2, hud-t98e.3, hud-t98e.4, hud-t98e.5 | Human-readable closeout/report artifact. |

## Dependency Graph

1. `hud-t98e.1`
2. `hud-t98e.2`
3. `hud-t98e.3`
4. `hud-t98e.4`
5. `hud-t98e.5`
6. `hud-t98e.6`

No parallelism is recommended in the first tranche. The input seam, raw-tile portal surface, and first adapter are tightly coupled enough that parallel PRs would likely increase review churn more than they reduce wall-clock time.

## Coordinator-Ready Summary

- The first ready child bead is `hud-t98e.1`.
- `hud-t98e.2` and `hud-t98e.3` are the core implementation tranche that proves the portal boundary without terminal drift.
- `hud-t98e.4` is the proof bead that should establish both automated and operator-facing confidence before the epic is considered closeable.
- `hud-t98e.5` is the mandatory reconciliation bead that audits final spec coverage and spawns follow-up work if the pilot leaves gaps.
- `hud-t98e.6` is the human-readable closeout bead required by `/project-direction` for a non-trivial epic.

## Recorded Beads Payload

See `docs/reconciliations/text_stream_portals_backlog_materialization_20260410.proposed_beads.json`.
