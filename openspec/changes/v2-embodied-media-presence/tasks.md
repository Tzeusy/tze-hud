## 1. Media Plane Foundation

- [ ] 1.1 Promote bounded media ingress from contract-only to an implementation-ready v2 capability
- [ ] 1.2 Land session-envelope media signaling and schema/snapshot parity
- [ ] 1.3 Implement runtime activation gate, operator policy, and telemetry for bounded ingress
- [ ] 1.4 Implement compositor `VideoSurfaceRef` render path and deterministic teardown/fallback states
- [ ] 1.5 Validate bounded ingress across headless synthetic and real-decode lanes

## 2. Embodied Presence

- [ ] 2.1 Define embodied presence level, session identity, and operator visibility contract
- [ ] 2.2 Bind media admission to embodied-capable sessions and explicit authority scope
- [ ] 2.3 Specify reconnect, reclaim, and failure behavior for embodied sessions with media intent
- [ ] 2.4 Add operator controls and audit surfaces for embodied presence state

## 3. Device Profile Execution

- [ ] 3.1 Convert mobile profile from schema-only to exercised runtime profile
- [ ] 3.2 Define glasses/companion-display upstream composition and degradation rules
- [ ] 3.3 Add capability negotiation for constrained-device profiles
- [ ] 3.4 Validate profile-specific performance, privacy, and operator behavior

## 4. Broader AV and Orchestration

- [ ] 4.1 Define admission criteria for bidirectional AV beyond bounded ingress
- [ ] 4.2 Define audio routing/mixing policy and household-aware output rules
- [ ] 4.3 Define multi-feed orchestration, layout, and priority contracts
- [ ] 4.4 Add embodied/media orchestration rules for coordinated agent presence

## 5. Validation and Operations

- [ ] 5.1 Extend validation framework with media/device rehearsal scenes and CI-visible verdicts
- [ ] 5.2 Define runner strategy and calibration requirements for real-decode and device-profile lanes
- [ ] 5.3 Add structured observability for media admission, teardown, operator actions, and device-state transitions
- [ ] 5.4 Define release-readiness gates for v2 media/device/embodied capability claims

## 6. Planning and Reconciliation

- [x] 6.1 Produce an end-to-end v2 execution plan with phase gates, dependencies, and explicit non-goals (`execution-plan.md`)
- [x] 6.2 Reconcile v2 specs against doctrine, RFCs, and the current bounded-ingress tranche (`reconciliation.md`)
- [x] 6.3 Generate a beads graph for the approved v2 program after signoff (`beads-graph.md`)
- [x] 6.4 Reconcile the full v2 planning package (proposal, design, specs, tasks, execution plan, beads graph) before execution-readiness claims (`final-reconciliation.md`)
