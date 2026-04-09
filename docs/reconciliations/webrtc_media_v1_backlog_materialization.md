# WebRTC/Media V1 Backlog Materialization (Corrected)

Date: 2026-04-09
Source issue: `hud-nn9d.2`
WM-S0 inventory artifact: `docs/reconciliations/webrtc_media_v1_seam_inventory.md`
Inputs:
- `docs/reconciliations/webrtc_media_v1_direction_report.md`
- `docs/reconciliations/webrtc_media_v1_human_signoff_report.md`
- `docs/reconciliations/webrtc_media_v1_reconciliation_gen1.md`

## Purpose

Materialize follow-on backlog from the WebRTC/media direction work into a stricter, lower-churn plan that matches actual repo seams.

This corrected backlog changes the original plan in four important ways:

1. Add an explicit repo-seam inventory before writing the first media capability spec.
2. Split protocol work into signaling-shape and schema/snapshot tasks instead of treating them as one bead.
3. Move privacy/operator controls and compositor contract definition into the contract tranche.
4. Delay creation of implementation beads until the corrected spec tranche receives renewed human review and reconciliation.

## Current Coordinator Caveat

The companion payload `docs/reconciliations/webrtc_media_v1_backlog_materialization.proposed_beads.json` reflects the earlier, more optimistic plan. Treat that JSON as stale until it is regenerated from this corrected backlog.

The current `hud-nn9d.*` beads closed the direction/signoff/reconciliation pass. They did not instantiate the corrected `WM-*` implementation program below.

## Ordering Rules

1. No runtime media implementation bead starts before media contracts, validation strategy, and privacy/operator controls are written and reviewed.
2. Contract beads are serialized where scope is still unsettled.
3. Implementation beads are not created until the corrected contract tranche receives renewed human review and reconciliation.
4. Deferred scope stays explicit as blocked follow-on beads, not TODOs.

## Proposed Bead Set (Coordinator-Apply)

### Phase A: Contract and seam inventory tranche (create now)

| Local ID | Title | Type | Priority | Depends on | Why now |
|---|---|---|---|---|---|
| WM-S0 | Inventory repo seams required for first media slice | task | 1 | discovered-from:hud-nn9d.4 | The original plan understated the protocol/schema/config/compositor surface area. |
| WM-S1 | Spec: media/WebRTC post-v1 capability contract (bounded ingress slice) | task | 1 | WM-S0 | Capability scope must be written only after all required seams are enumerated. |
| WM-S2a | Spec decision: media signaling shape (`Session` delta vs separate media RPC) | task | 1 | WM-S1 | The repo does not yet have a settled media signaling contract. |
| WM-S2b | Spec: protocol/schema and snapshot deltas for bounded media ingress | task | 1 | WM-S1, WM-S2a | Covers proto fields, snapshot semantics, and backward compatibility. |
| WM-S2c | Spec: media zone contract (zone identity, transport constraint, layer attachment, reconnect semantics) | task | 1 | WM-S1, WM-S2b | “Fixed zone” is too vague without an explicit zone/transport contract. |
| WM-S3 | Spec: runtime media activation gate and budgets | task | 1 | WM-S1, WM-S2a, WM-S2b | Prevent ad hoc media-thread or decode enablement. |
| WM-S3b | Spec: privacy, operator controls, and enablement policy for media ingress | task | 1 | WM-S1, WM-S2c | Household-screen privacy and operator control are admission criteria, not cleanup work. |
| WM-S3c | Spec: compositor contract for `VideoSurfaceRef` rendering | task | 1 | WM-S1, WM-S2c, WM-S3 | Rendering and degradation semantics need a contract before implementation exists. |
| WM-S4 | Spec: validation-framework media rehearsal scenarios and pass/fail thresholds | task | 1 | WM-S1, WM-S2b, WM-S2c, WM-S3, WM-S3b, WM-S3c | Defines measurable acceptance before implementation beads are even created. |
| WM-S5 | Docs/spec alignment: architecture-v1 phased contract wording for media deferment | task | 2 | WM-S1 | Removes doctrine ambiguity called out in the direction work. |
| WM-S6 | Docs alignment: README media/WebRTC claims vs v1/post-v1 scope | task | 2 | WM-S1, WM-S5 | Closes public-claim drift around active v1 media support. |

### Phase B: Renewed review gates (create now)

| Local ID | Title | Type | Priority | Depends on | Notes |
|---|---|---|---|---|---|
| WM-G1 | Human review: corrected media contract tranche signoff | task | 1 | WM-S1, WM-S2a, WM-S2b, WM-S2c, WM-S3, WM-S3b, WM-S3c, WM-S4, WM-S5, WM-S6 | Required because the original signoff happened before these corrected seams were spelled out. |
| WM-G2 | Reconciliation: corrected media contract + backlog coverage | task | 1 | WM-G1 | Verifies that the corrected tranche closes the protocol/schema/config/compositor gaps before implementation begins. |

### Phase C: Implementation tranche (do not create until WM-G1 and WM-G2 are closed)

| Local ID | Title | Type | Priority | Depends on | Notes |
|---|---|---|---|---|---|
| WM-I1 | Implement media signaling/schema path for bounded ingress slice | feature | 1 | WM-G2 | Applies the approved signaling-shape and schema/snapshot spec. |
| WM-I2 | Implement runtime activation gate (`off` by default) plus approved operator controls | feature | 1 | WM-G2, WM-I1 | Includes explicit enablement policy and default-off behavior. |
| WM-I3 | Implement compositor `VideoSurfaceRef` render path for the approved media zone contract | feature | 1 | WM-G2, WM-I2 | Must match the approved zone identity, transport, and degradation contract. |
| WM-I4 | Implement validation scenes, benchmarks, and privacy/operator tests for bounded media ingress | task | 1 | WM-G2, WM-I3 | Makes readiness evidence explicit in CI/integration flows. |

### Phase D: Deferred scope markers (create as blocked/deferred only after Phase C is real)

| Local ID | Title | Type | Priority | Depends on | Deferred reason |
|---|---|---|---|---|---|
| WM-D1 | Deferred: bidirectional AV/WebRTC session negotiation and embodied presence | feature | 3 | WM-I4 | Explicitly out of scope for the bounded ingress tranche. |
| WM-D2 | Deferred: audio routing/mixing policy engine | feature | 3 | WM-I4 | Premature until bounded video ingress is stable and measured. |
| WM-D3 | Deferred: multi-feed compositing and adaptive bitrate orchestration | feature | 4 | WM-I4 | High complexity and intentionally outside the bounded tranche. |

## Corrected Dependency Graph

1. `WM-S0`
2. `WM-S1`
3. `WM-S2a`
4. `WM-S2b`, `WM-S2c`
5. `WM-S3`, `WM-S3b`, `WM-S3c`
6. `WM-S4`, `WM-S5`, `WM-S6`
7. `WM-G1`
8. `WM-G2`
9. Only then create `WM-I1` -> `WM-I2` -> `WM-I3` -> `WM-I4`
10. `WM-D1`, `WM-D2`, `WM-D3` remain blocked/deferred

## Coordinator-Ready Bead Create Payloads (Do Not Execute Here)

Use these as direct `bd create` inputs from coordinator context. Replace local IDs in dependency arguments with created bead IDs.

```bash
bd create "Inventory repo seams required for first media slice" \
  --type task --priority 1 \
  --description "Inventory all repo surfaces required for a bounded post-v1 media ingress slice: protocol, types.proto schema, snapshots, zone transport semantics, config/profile surfaces, compositor contract, validation strategy, privacy/operator controls, and default-off activation gates. Convert each seam into downstream spec work or mark it explicitly deferred." \
  --deps discovered-from:hud-nn9d.4 --json

bd create "Spec: media/WebRTC post-v1 capability contract (bounded ingress slice)" \
  --type task --priority 1 \
  --description "Author normative capability spec for the smallest admissible post-v1 media slice after seam inventory is complete. Include timing model, transport boundaries, lease/budget coupling, privacy assumptions, explicit non-goals, and measurable acceptance scenarios." \
  --deps blocks:<WM-S0-ID> --json

bd create "Spec decision: media signaling shape (Session delta vs separate media RPC)" \
  --type task --priority 1 \
  --description "Resolve whether bounded media ingress extends Session messaging or introduces a separate media RPC. Document compatibility, downgrade behavior, and why the chosen shape is lower-churn for this repo." \
  --deps blocks:<WM-S1-ID> --json

bd create "Spec: protocol/schema and snapshot deltas for bounded media ingress" \
  --type task --priority 1 \
  --description "Define proto fields/messages, zone snapshot semantics, reconnect behavior, and backward-compatible schema changes required for bounded media ingress." \
  --deps blocks:<WM-S1-ID> --deps blocks:<WM-S2a-ID> --json

bd create "Spec: media zone contract (zone identity, transport constraint, layer attachment, reconnect semantics)" \
  --type task --priority 1 \
  --description "Define the exact approved media zone or zone class for bounded ingress, including transport constraint representation, layer attachment, reconnect/snapshot behavior, and any fixed-zone restrictions." \
  --deps blocks:<WM-S1-ID> --deps blocks:<WM-S2b-ID> --json

bd create "Spec: runtime media activation gate and budgets" \
  --type task --priority 1 \
  --description "Define runtime media worker activation criteria, default-off behavior, degradation coupling, quantitative budgets, and prerequisites for enabling the bounded ingress path." \
  --deps blocks:<WM-S1-ID> --deps blocks:<WM-S2a-ID> --deps blocks:<WM-S2b-ID> --json

bd create "Spec: privacy, operator controls, and enablement policy for media ingress" \
  --type task --priority 1 \
  --description "Define viewer/privacy constraints, human/operator overrides, observability requirements, and explicit enablement policy for bounded media ingress on a household-facing display." \
  --deps blocks:<WM-S1-ID> --deps blocks:<WM-S2c-ID> --json

bd create "Spec: compositor contract for VideoSurfaceRef rendering" \
  --type task --priority 1 \
  --description "Define texture ownership, present-time semantics, degradation/fallback states, and non-audio render behavior for VideoSurfaceRef within the approved media zone contract." \
  --deps blocks:<WM-S1-ID> --deps blocks:<WM-S2c-ID> --deps blocks:<WM-S3-ID> --json

bd create "Spec: validation-framework media rehearsal scenarios and pass/fail thresholds" \
  --type task --priority 1 \
  --description "Specify validation scenes and benchmark thresholds for bounded media ingress, including headless-vs-real-decode strategy, privacy/operator control tests, and CI-visible pass/fail outputs." \
  --deps blocks:<WM-S1-ID> --deps blocks:<WM-S2b-ID> --deps blocks:<WM-S2c-ID> --deps blocks:<WM-S3-ID> --deps blocks:<WM-S3b-ID> --deps blocks:<WM-S3c-ID> --json

bd create "Docs/spec alignment: architecture-v1 phased contract wording for media deferment" \
  --type task --priority 2 \
  --description "Align architecture/v1 wording so media vision and v1 deferment are explicitly phased and non-contradictory." \
  --deps blocks:<WM-S1-ID> --json

bd create "Docs alignment: README media/WebRTC claims vs v1/post-v1 scope" \
  --type task --priority 2 \
  --description "Align README claims about protocol planes and media so v1 boundaries remain explicit while preserving post-v1 direction." \
  --deps blocks:<WM-S1-ID> --deps blocks:<WM-S5-ID> --json

bd create "Human review: corrected media contract tranche signoff" \
  --type task --priority 1 \
  --description "Produce a refreshed human-readable signoff report for the corrected media contract tranche after the missing protocol/schema/config/compositor seams are specified." \
  --deps blocks:<WM-S1-ID> --deps blocks:<WM-S2a-ID> --deps blocks:<WM-S2b-ID> --deps blocks:<WM-S2c-ID> --deps blocks:<WM-S3-ID> --deps blocks:<WM-S3b-ID> --deps blocks:<WM-S3c-ID> --deps blocks:<WM-S4-ID> --deps blocks:<WM-S5-ID> --deps blocks:<WM-S6-ID> --json

bd create "Reconciliation: corrected media contract + backlog coverage" \
  --type task --priority 1 \
  --description "Verify that the corrected media contract tranche closes the missing protocol/schema/config/compositor seams and that no implementation beads are created on hidden assumptions." \
  --deps blocks:<WM-G1-ID> --json
```

## Implementation Guardrail

Do not create `WM-I*` beads until `WM-G1` and `WM-G2` are closed. The original backlog's implementation tranche was too optimistic about how narrow the bounded media slice really is.

When Phase C is eventually created, each implementation bead should cite the exact corrected spec beads above and should not merge new scope back into the tranche.

## Acceptance Traceability

- Target 1 (explicit dependencies): Satisfied by the corrected phase tables and create payload dependency wiring.
- Target 2 (spec-writing first): Satisfied by `WM-S0` through `WM-S6` plus renewed review gates before any implementation beads exist.
- Target 3 (low-churn + deferred markers): Satisfied by the seam inventory, split protocol work, early privacy/operator requirements, and explicit `WM-D*` deferred beads.
