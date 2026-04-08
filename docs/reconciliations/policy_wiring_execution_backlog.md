# Policy Wiring Execution Backlog (Coordinator-Ready)

Date: 2026-04-08
Source issue: `hud-iq2x.2`
Input report: `docs/reconciliations/policy_wiring_direction_report.md` (from `origin/main`, merged via PR #375)

## Goal
Materialize the policy wiring direction output into a concrete, dependency-ordered implementation backlog with explicit spec grounding and low-churn sequencing.

## Low-Churn Execution Order
1. Spec reconciliation and seam contract first (documentation/spec only).
2. Mutation-path pilot next (code), preserving runtime ownership boundaries.
3. Telemetry and conformance harness immediately after pilot to prove budgets and behavior.
4. Event/frame extension only after mutation path is stable and measurable.
5. Human review summary and reconciliation tail close the loop.

## Dependency Graph
- `PW-01` -> `PW-02`
- `PW-02` -> `PW-03`
- `PW-03` -> `PW-04`
- `PW-04` -> `PW-05`
- `PW-01` -> `PW-06`
- `PW-02` -> `PW-06`
- `PW-03` -> `PW-06`
- `PW-04` -> `PW-06`
- `PW-05` -> `PW-06`
- `PW-06` -> `PW-07`

## Proposed Bead Set (for coordinator to create)

### PW-01 — Reconcile v1 policy authority claims across specs
- Type/Priority: `task` / `P1`
- Depends on: `hud-iq2x.2`
- Spec citations:
  - `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md` (per-mutation/per-event/per-frame MUST claims)
  - `openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md` (runtime authority/budget constraints)
  - `openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md` (capability request policy semantics)
- Acceptance:
  - Distinguish implemented v1 runtime authority from target policy wiring.
  - Remove or reclassify unwired MUST claims that currently overstate implementation.
  - Add explicit spec-first handoff notes for downstream implementation beads.

### PW-02 — Define runtime-policy seam contract and ownership matrix
- Type/Priority: `task` / `P1`
- Depends on: `PW-01`
- Spec-first marker: `REQUIRED before any policy wiring code`
- Spec citations:
  - `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md`
  - `openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md`
  - Doctrine boundary references: `about/heart-and-soul/v1.md`, `about/heart-and-soul/architecture.md`
- Acceptance:
  - Publish level-by-level (0-6) input-source mapping.
  - Publish explicit runtime-owned mutable state matrix (budget, safe-mode, contention, lease state).
  - Define `PolicyContext` construction contract and `ArbitrationOutcome` execution contract.

### PW-03 — Implement mutation-path pilot wiring via `tze_hud_policy`
- Type/Priority: `feature` / `P1`
- Depends on: `PW-02`
- Spec citations:
  - `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md` (per-mutation ordering and short-circuit)
  - `openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md` (mutation/result semantics)
- Acceptance:
  - Runtime builds policy evaluation context snapshots for mutation path.
  - Runtime executes policy outcomes while retaining runtime ownership of mutable counters/state.
  - Safe-mode/freeze/capability behavior remains stable or is explicitly re-specified.
  - Add/adjust tests for policy ordering and rejection/queue/shed outcomes.

### PW-04 — Add policy decision telemetry + latency conformance harness
- Type/Priority: `task` / `P1`
- Depends on: `PW-03`
- Spec citations:
  - `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md` (per-mutation/per-frame budgets)
  - `openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md` (frame budget/degradation requirements)
  - `openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md` (runtime telemetry frame contract)
- Acceptance:
  - Emit per-level decision counts/outcomes from wired path.
  - Add CI-visible benchmark/assertions for policy path latency budgets.
  - Failure output includes actionable diagnostics (level, path, budget, observed percentile).

### PW-05 — Extend wiring to per-event and per-frame paths
- Type/Priority: `feature` / `P2`
- Depends on: `PW-04`
- Spec citations:
  - `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md` (per-event and per-frame pipelines)
  - `openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md` (stage guarantees and frame overlap)
- Acceptance:
  - Implement event/frame ordering and short-circuit semantics per reconciled spec.
  - Preserve one-frame local-override guarantees.
  - Add end-to-end tests for event/frame policy paths.

### PW-06 — Reconcile implementation vs doctrine/spec after wiring phases
- Type/Priority: `task` / `P1`
- Depends on: `PW-01`, `PW-02`, `PW-03`, `PW-04`, `PW-05`
- Spec-first marker: `reconcile before closure`
- Spec citations:
  - `about/heart-and-soul/v1.md`
  - `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md`
  - `openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md`
  - `openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md`
- Acceptance:
  - Evidence map from code paths/tests to each retained MUST claim.
  - Any uncovered MUST produces explicit new bead(s), not TODO comments.
  - Contradictions from direction report are closed or consciously deferred.

### PW-07 — Publish human signoff report for policy wiring program
- Type/Priority: `task` / `P1`
- Depends on: `PW-06`
- Spec citation or marker: `report references reconciled specs + implemented evidence`
- Acceptance:
  - Produce concise signoff report under `docs/reconciliations/`.
  - Link all created/closed follow-on beads and unresolved risks.
  - Include explicit do-not-do-yet list for post-v1 deferrals.

## Coordinator Application Notes
- Create these as child/follow-on beads under epic `hud-iq2x`.
- Use explicit dependency edges exactly as listed in the graph above.
- Keep `hud-iq2x.3`/`hud-iq2x.4` as report/reconciliation tails if they still map; otherwise supersede with `PW-06`/`PW-07` and link via `discovered-from`.
- Do not start `PW-03` until `PW-01` + `PW-02` are complete (spec-first guardrail).
