# Policy Wiring Execution Backlog (Coordinator-Ready, Corrected)

Date: 2026-04-09
Source issue: `hud-iq2x.2`
Coordinator source-of-truth artifact: `policy_wiring_execution_backlog.md`
Input reports:
- `docs/reconciliations/policy_wiring_direction_report.md`
- `docs/reconciliations/policy_wiring_direction_report_20260409.md`
- `docs/reconciliations/policy_wiring_human_signoff_report.md`
- `docs/reconciliations/policy_wiring_gen1_reconciliation.md`
- `docs/reconciliations/policy_wiring_final_reconciliation_20260409.md`

## Goal
Materialize the policy wiring direction output into a concrete, dependency-ordered backlog that is safe to execute against current repo reality.

This corrected backlog makes three changes to the initial plan:

1. Front-load the already-live lease/session capability seam before broader policy wiring.
2. Treat capability-escalation semantics as first-class work, not an enhancement.
3. Replace assumed event/frame implementation with a post-pilot scope decision gate.

## Current Coordinator Caveat

The earlier companion payload `docs/reconciliations/policy_wiring_execution_backlog.proposed_beads.json` reflects the older, less strict plan. Treat that JSON as a legacy/stale artifact and explicitly superseded by this backlog.

Tracker instantiation snapshot (2026-04-09):
- `PW-00` -> `hud-iq2x.5` (closed)
- `PW-01` -> `hud-iq2x.6` (closed)
- `PW-02` -> `hud-iq2x.7` (blocked pending PR review)
- `PW-02b` -> `hud-iq2x.8` (closed)
- `PW-03` -> `hud-jq5p` (in progress)
- `PW-04`..`PW-08` not yet instantiated

`hud-iq2x.5` through `hud-iq2x.8` now instantiate the governance-first front of this plan (lease scope, spec reconciliation, seam matrix, capability-escalation semantics). Mutation pilot and downstream telemetry/scope/reconciliation beads are still pending instantiation; the final signoff closure bead is now tracked separately as `hud-5fb1`.

This document remains the coordinator source of truth for remaining bead creation and dependency wiring.

## Corrected Low-Churn Execution Order

1. Fix the live lease capability-scope mismatch.
2. Reconcile v1 policy authority claims across specs.
3. Define the full runtime/policy/scene seam contract and ownership matrix.
4. Define capability-escalation policy semantics explicitly.
5. Implement mutation-path pilot wiring only.
6. Add telemetry and latency conformance gates for the pilot.
7. Decide whether event/frame policy wiring remains a v1 implementation goal or whether the v1 spec should shrink.
8. Reconcile implementation against doctrine/specs and publish signoff.

## Dependency Graph

- `discovered-from:hud-iq2x.4` -> `PW-00`
- `discovered-from:hud-iq2x.2` -> `PW-01`
- `PW-01` -> `PW-02`
- `PW-00` -> `PW-02b`
- `PW-01` -> `PW-02b`
- `PW-02` -> `PW-03`
- `PW-02b` -> `PW-03`
- `PW-03` -> `PW-04`
- `PW-04` -> `PW-05`
- `PW-01` -> `PW-06`
- `PW-02` -> `PW-06`
- `PW-02b` -> `PW-06`
- `PW-03` -> `PW-06`
- `PW-04` -> `PW-06`
- `PW-05` -> `PW-06`
- `PW-06` -> `PW-07`
- `discovered-from:hud-iq2x.4` -> `PW-08`

## Proposed Bead Set (for coordinator to create)

Instantiation note: `PW-00`, `PW-01`, `PW-02`, `PW-02b`, and `PW-03` now exist in tracker form. The sections below remain authoritative for acceptance criteria, citations, and dependency intent.

### PW-00 — Fix lease capability scope to session-granted subset
- Type/Priority: `bug` / `P1`
- Discovered from: `discovered-from:hud-iq2x.4`
- Spec citations:
  - `openspec/changes/v1-mvp-standards/specs/lease-governance/spec.md`
  - `openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md`
- RFC/doctrine citations:
  - `about/law-and-lore/rfcs/0008-lease-governance.md`
  - `about/heart-and-soul/security.md`
- Acceptance:
  - Lease grants MUST deny or clamp requested capabilities that exceed the session-granted authorization set.
  - Tests cover over-requested lease capabilities and prove the agent cannot expand lease scope beyond session grants.
  - Runtime behavior and spec language are aligned for lease capability scope.
- Notes:
  - This is a live authority seam and should not wait for broader policy-stack work.

### PW-01 — Reconcile v1 policy authority claims across specs
- Type/Priority: `task` / `P1`
- Discovered from: `discovered-from:hud-iq2x.2`
- Spec citations:
  - `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md`
  - `openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md`
  - `openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md`
- Acceptance:
  - Distinguish implemented v1 runtime authority from target policy wiring.
  - Remove or reclassify unwired MUST claims that currently overstate implementation.
  - Capture explicit handoff notes for downstream implementation beads.

### PW-02 — Define runtime-policy-scene seam contract and ownership matrix
- Type/Priority: `task` / `P1`
- Depends on: `PW-01`
- Spec-first marker: `REQUIRED before any policy wiring code`
- Spec citations:
  - `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md`
  - `openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md`
  - `openspec/changes/v1-mvp-standards/specs/lease-governance/spec.md`
- Doctrine boundary references:
  - `about/heart-and-soul/v1.md`
  - `about/heart-and-soul/security.md`
  - `about/heart-and-soul/presence.md`
- Acceptance:
  - Publish level-by-level (0-6) input-source mapping.
  - Publish explicit ownership matrix for runtime-owned mutable state, including budget, safe mode, contention, lease state, and attention state.
  - Explicitly account for all three policy surfaces: `tze_hud_runtime`, `tze_hud_policy`, and `tze_hud_scene::policy`.
  - State which abstractions are canonical, transitional, or scheduled for retirement.
  - Define `PolicyContext` construction contract and `ArbitrationOutcome` execution contract.

### PW-02b — Define capability-escalation policy source semantics
- Type/Priority: `task` / `P1`
- Depends on: `PW-00`, `PW-01`
- Spec-first marker: `REQUIRED before mutation-path wiring`
- Spec citations:
  - `openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md`
  - `openspec/changes/v1-mvp-standards/specs/lease-governance/spec.md`
  - `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md`
- Acceptance:
  - Source of truth for `CapabilityRequest` evaluation is explicit.
  - Session grants, lease grants, and mid-session escalation semantics are aligned.
  - Tests cover grant, deny, mixed-capability denial, and reconnect/resume cases.
  - Any remaining operator-approval or dynamic-policy assumptions are written down, not implied.

### PW-03 — Implement mutation-path pilot wiring via `tze_hud_policy`
- Type/Priority: `feature` / `P1`
- Tracker issue: `hud-jq5p`
- Depends on: `PW-02`, `PW-02b`
- Spec citations:
  - `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md`
  - `openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md`
  - `openspec/changes/v1-mvp-standards/specs/lease-governance/spec.md`
- Acceptance:
  - Runtime builds policy evaluation context snapshots for mutation path.
  - Runtime executes policy outcomes while retaining runtime ownership of mutable counters/state.
  - Existing safe-mode, freeze, lease, and capability behavior remains stable or is explicitly re-specified before merge.
  - Tests cover ordering plus reject/queue/shed/commit outcomes.
- Notes:
  - Do not expand to event/frame paths in this bead.

### PW-04 — Add policy decision telemetry + latency conformance harness
- Type/Priority: `task` / `P1`
- Depends on: `PW-03`
- Spec citations:
  - `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md`
  - `openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md`
  - `openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md`
- Doctrine references:
  - `about/heart-and-soul/validation.md`
- Acceptance:
  - Emit per-level decision counts/outcomes from the wired mutation path.
  - Add CI-visible benchmark/assertions for policy-path latency budgets.
  - Failure output includes actionable diagnostics: level, path, budget, and observed percentile.
  - Benchmark guidance is deterministic enough for CI use.

### PW-05 — Decide v1 scope for event/frame policy wiring after mutation pilot
- Type/Priority: `task` / `P1`
- Depends on: `PW-04`
- Spec citations:
  - `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md`
  - `openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md`
- Acceptance:
  - Use mutation-path telemetry and latency data to decide whether event/frame policy wiring remains a v1 implementation goal.
  - Produce one of two explicit outcomes:
    - event/frame implementation follow-on beads, or
    - spec-shrink/spec-clarification beads that remove v1 overcommitment.
  - No event/frame implementation may start before this decision closes.
- Notes:
  - This replaces the earlier assumption that event/frame implementation is automatically the next step.

### PW-06 — Reconcile implementation vs doctrine/spec after pilot and scope decision
- Type/Priority: `task` / `P1`
- Depends on: `PW-01`, `PW-02`, `PW-02b`, `PW-03`, `PW-04`, `PW-05`
- Spec-first marker: `reconcile before closure`
- Spec citations:
  - `about/heart-and-soul/v1.md`
  - `openspec/changes/v1-mvp-standards/specs/policy-arbitration/spec.md`
  - `openspec/changes/v1-mvp-standards/specs/runtime-kernel/spec.md`
  - `openspec/changes/v1-mvp-standards/specs/session-protocol/spec.md`
  - `openspec/changes/v1-mvp-standards/specs/lease-governance/spec.md`
- Acceptance:
  - Evidence map from code paths/tests to each retained MUST claim.
  - Any uncovered MUST produces explicit new bead(s), not TODO comments.
  - Contradictions from the direction report are closed, reduced, or consciously deferred with rationale.

### PW-07 — Publish human signoff report for corrected policy wiring program
- Type/Priority: `task` / `P1`
- Depends on: `PW-06`
- Discovered from: `discovered-from:hud-iq2x.4`
- Tracker instantiation status: now tracked as `hud-5fb1`
- Acceptance:
  - Produce concise signoff report under `docs/reconciliations/`.
  - Link all created/closed follow-on beads and unresolved risks.
  - State explicitly whether event/frame policy wiring remains in v1 scope.

### PW-08 — Patch stale policy direction artifacts and mark proposal-only status
- Type/Priority: `docs` / `P2`
- Discovered from: `discovered-from:hud-iq2x.4`
- Acceptance:
  - Remove stale prompt-file-missing claims from policy direction artifacts.
  - Mark older `PW-*` proposal payloads as superseded if they remain in-tree.
  - Leave future readers with one unambiguous coordinator source of truth.

## Coordinator Application Notes

- Create these as child/follow-on beads under epic `hud-iq2x`.
- Use explicit dependency edges exactly as listed above.
- Do not start `PW-03` until `PW-02` and `PW-02b` are complete.
- Do not pre-create event/frame implementation beads. Create them only if `PW-05` concludes they remain in v1 scope.
