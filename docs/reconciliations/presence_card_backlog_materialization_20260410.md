# Presence Card Backlog Materialization

Date: 2026-04-10
Source artifact: `docs/reconciliations/presence_card_direction_report_20260410.md`
Scope: materialize the smallest honest bead graph that closes the Presence Card live-proof gap
Materialized epic: `hud-sx7q`

## Purpose

Turn the Presence Card direction work into an execution-ready, low-churn beads graph.

This backlog deliberately assumes the current code reality:

1. Raw tile, lease, resource, coexistence, and disconnect behaviors already have substantial automated coverage.
2. The remaining critical gap is the live resident `/user-test` path and the missing manual-review closure.
3. The first tranche should repair doc drift and tooling gaps before any attempt to declare the exemplar finished.

## Ordering Rules

1. Reconcile stale Presence Card docs before writing implementation beads against them.
2. Extend the resident gRPC helper before writing the live scenario script.
3. Do not mark Presence Card complete until the live Windows scenario is executed and the manual checklist is updated.
4. End the epic with both a reconciliation bead and a human-readable report bead.

## Materialized Bead Set

### Phase A: Truth and tooling prep

| Bead ID | Title | Type | Priority | Depends on | Why now |
|---|---|---|---|---|---|
| hud-sx7q.1 | Reconcile Presence Card coverage and review artifacts | task | 1 | none | Current docs still describe already-landed work as blocked or partial. |
| hud-sx7q.2 | Extend gRPC user-test helper for Presence Card resident flows | task | 1 | hud-sx7q.1 | The live scenario depends on upload + richer tile mutation support. |

### Phase B: Live scenario materialization

| Bead ID | Title | Type | Priority | Depends on | Why now |
|---|---|---|---|---|---|
| hud-sx7q.3 | Add Presence Card live scenario and `/user-test` integration | feature | 1 | hud-sx7q.2 | This is the missing execution surface that turns tests into operator proof. |
| hud-sx7q.4 | Execute Presence Card live validation and close manual review | task | 1 | hud-sx7q.3 | The exemplar is not complete until the Windows/manual path exists and is used. |

### Phase C: Terminal quality gates

| Bead ID | Title | Type | Priority | Depends on | Notes |
|---|---|---|---|---|---|
| hud-sx7q.5 | Reconcile spec-to-code (gen-1) for Presence Card live user-test flow | task | 1 | hud-sx7q.1, hud-sx7q.2, hud-sx7q.3, hud-sx7q.4 | Mandatory terminal reconciliation bead. |
| hud-sx7q.6 | Generate epic report for: Presence Card live user-test flow | task | 1 | hud-sx7q.1, hud-sx7q.2, hud-sx7q.3, hud-sx7q.4, hud-sx7q.5 | Human-readable execution/report artifact. |

## Dependency Graph

1. `hud-sx7q.1`
2. `hud-sx7q.2`
3. `hud-sx7q.3`
4. `hud-sx7q.4`
5. `hud-sx7q.5`
6. `hud-sx7q.6`

No parallelism is recommended in this tranche. The surfaces are small but tightly coupled, and premature splitting would create review overhead without reducing risk.

## Coordinator-Ready Summary

- The first ready child bead is `hud-sx7q.1`.
- `hud-sx7q.2` and `hud-sx7q.3` are the real implementation tranche.
- `hud-sx7q.4` is the manual/live validation gate and should not be skipped.
- `hud-sx7q.5` is the mandatory reconciliation bead that audits final spec coverage and creates follow-up beads if live validation exposes new gaps.
- `hud-sx7q.6` is the human-readable closeout bead.

## Recorded Beads Payload

See `docs/reconciliations/presence_card_backlog_materialization_20260410.proposed_beads.json`.

## Closeout Path

- Reconciliation artifact (`hud-sx7q.5`):
  `docs/reconciliations/presence_card_live_user_test_reconciliation_gen1_20260416.md`
- Epic report (`hud-sx7q.6`):
  `docs/reconciliations/presence_card_live_user_test_epic_report_20260416.md`
