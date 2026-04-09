# V2 Embodied/Media Execution Plan

Date: 2026-04-09  
Issue: `hud-8cy3.2`  
Change: `openspec/changes/v2-embodied-media-presence`

## Objective

Deliver a dependency-ordered v2 program that activates media and embodied capabilities without violating v1 boundaries, governance doctrine, or validation discipline.

## Program Constraints

1. Do not widen v1 promises from `about/heart-and-soul/v1.md`.
2. Preserve runtime sovereignty for timing, admission, and revocation.
3. Keep bounded ingress as the first admissible media slice.
4. Use evidence-gated progression between phases; no phase skipping.

## Phase Plan

### Phase 1: Bounded Media Activation

Scope:
- `tasks.md` 1.1 to 1.5
- `tasks.md` 5.1 to 5.3 (minimum telemetry/validation lanes required for go/no-go)

Exit gates:
1. Bounded ingress admission is capability + lease + operator + privacy + budget gated.
2. Compositor render and teardown behavior is deterministic for media surfaces.
3. Validation lane includes both synthetic/headless and at least one real-decode proof path.
4. Operator override and revoke behavior is validated and observable.

Primary risks:
- Decode path instability across environments.
- Scope creep from bounded ingress into bidirectional AV.

### Phase 2: Embodied Presence

Scope:
- `tasks.md` 2.1 to 2.4

Entry prerequisites:
1. Phase 1 exit gates passed.
2. Embodied identity/authority semantics agreed in spec and protocol contract surfaces.

Exit gates:
1. Embodied state is explicit and distinguishable from guest/resident.
2. Media behavior is bound to embodied authority and tears down on revocation.
3. Reconnect/reclaim/failure semantics are deterministic and test-covered.
4. Operator visibility + audit surfaces are available for embodied transitions.

Primary risks:
- Authority-model drift between session state and lease enforcement.
- Privacy/operator controls lagging behind embodied capability surfaces.

### Phase 3: Device Profile Execution

Scope:
- `tasks.md` 3.1 to 3.4
- `tasks.md` 5.2 (runner and calibration requirements for device lanes)

Entry prerequisites:
1. Phase 2 exit gates passed.
2. Device capability-negotiation contract is stable enough for exercised profiles.

Exit gates:
1. Mobile profile executes as an exercised runtime profile (not schema-only).
2. Glasses/companion composition and degradation behavior is explicit and tested.
3. Profile-specific privacy/perf/operator assertions are measured and published.
4. Runner strategy for device-representative lanes is documented and reproducible.

Primary risks:
- Validation blind spots caused by non-representative runners.
- Degradation-policy mismatch between desktop assumptions and constrained devices.

### Phase 4: Broader AV and Orchestration

Scope:
- `tasks.md` 4.1 to 4.4
- `tasks.md` 5.4 (release-readiness gates for broader claims)

Entry prerequisites:
1. Phases 1 through 3 complete with evidence.
2. Bidirectional AV admission criteria approved by operator/privacy/governance owners.

Exit gates:
1. Bidirectional AV admission policy is explicit; no implicit escalation from ingress.
2. Audio routing and household-aware policy contracts are defined and validated.
3. Multi-feed orchestration conflict resolution is policy-driven, not implicit.
4. Release-readiness criteria for v2 claims are documented and passing.

Primary risks:
- Operational complexity outpacing policy/observability maturity.
- Contention behavior becoming non-deterministic in multi-feed scenarios.

## Dependency and Sequencing Rules

1. Phase 1 is a hard prerequisite for all other phases.
2. Phase 2 must complete before Phase 4 claims embodied media orchestration.
3. Phase 3 can run partially in parallel with late Phase 2 docs/tests, but release signoff remains serialized: 1 -> 2 -> 3 -> 4.
4. Validation and observability tasks are not "last mile"; each phase must land its minimum evidence before advancing.

## Non-Goals for This Planning Bead

1. No new wire-level schemas or runtime implementation in this bead.
2. No bidirectional AV policy finalization.
3. No claim that mobile/glasses execution is already shipped.
4. No reopening of v1 scope boundaries.

## Evidence Package Required Per Phase

For each phase, produce:
1. Spec deltas with `Source` and `Scope` tags.
2. Task completion evidence tied to test/validation outputs.
3. Risk status update (open, mitigated, accepted with rationale).
4. Reconciliation note against doctrine and adjacent capability contracts.

## Handoff to Next Beads

1. `hud-8cy3.3` should transform this phase plan into a dependency-ordered bead graph.
2. `hud-8cy3.4` should verify coherence across proposal/design/spec/tasks/evidence and flag any scope drift before execution begins.
