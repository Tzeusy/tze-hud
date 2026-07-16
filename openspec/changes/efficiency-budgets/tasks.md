## 1. Owner Gate and Measurement Contracts

- [ ] 1.1 Obtain explicit owner approval of the idle settling/window values, two-wakeup-per-second ceiling, constrained-envelope definition, canonical-flow vector, and five-percent byte/token regression threshold before implementation beads unblock.
- [ ] 1.2 Define versioned structured artifact schemas for quiescent counters, invalidation closures and work amplification, constrained-profile identity, and canonical-flow byte/token measurements, including fail-closed missing-field validation.

## 2. Idle and Change-Proportional Runtime Instrumentation

- [ ] 2.1 Instrument runtime-driven main/compositor wakeups, GPU queue submissions, surface acquisitions, and presents with source attribution that excludes benchmark sampling and external operating-system events.
- [ ] 2.2 Implement quiescence detection and the canonical 5-second-settle plus 60-second idle scenario for both overlay and headless paths, with tests enforcing zero GPU work and at most 120 runtime-driven wakeups.
- [ ] 2.3 Implement typed invalidation-closure accounting across layout, raster, texture upload, render encoding, and composition damage, including structured full-surface fallback reasons.
- [ ] 2.4 Add the canonical one-node change in a 50-tile scene and dependency-expansion scenarios, with diagnostic gates rejecting all unrelated out-of-closure work.

## 3. Constrained-Envelope Calibration Lane

- [ ] 3.1 Select and document a reproducible WARP or llvmpipe runner plus an enforceable two-logical-CPU process limit without claiming device qualification.
- [ ] 3.2 Run the existing versioned CPU/GPU/upload calibration vector and benchmark corpus under the constrained profile, recording complete runner, backend, adapter, limit, resolution, factor, and enforcement identity.
- [ ] 3.3 Gate constrained results against the reference lane's unchanged normalized ceilings and add fail-closed tests for missing constraints, missing identity, renderer fallback, and invalid calibration.

## 4. Canonical LLM Flow Calibration

- [ ] 4.1 Define versioned deterministic JSON-RPC fixtures for `publish_to_zone`, the attach/publish/long-poll/acknowledge portal turn, and `publish_to_widget`, including fixed canonical sentinels for dynamic secret-bearing fields.
- [ ] 4.2 Select and pin the authoritative tokenizer implementation, version, vocabulary fingerprint, and counting policy; add deterministic byte/token accounting for every request, response, operation, and flow total.
- [ ] 4.3 Add compatibility checks for tokenizer and fixture fingerprints plus repeated-run tests proving identical byte and token counts.
- [ ] 4.4 Generate initial candidate baselines, obtain explicit owner approval, and commit the approved counts and rationale as the comparison authority.
- [ ] 4.5 Implement the exact integer five-percent failure threshold, structured warning for smaller increases, improvement reporting, and fail-closed missing/unapproved baseline behavior.

## 5. Validation, Documentation, and Rollout

- [ ] 5.1 Integrate efficiency artifacts into Layer 3/Layer 4 outputs and CI summaries with actionable actual, budget, absolute-delta, percentage-delta, scenario, flow, and fingerprint diagnostics.
- [ ] 5.2 Run focused unit/integration tests, the constrained calibration lane, canonical-flow determinism checks, repository spec/documentation gates, and the full risk-scaled CI suite.
- [ ] 5.3 Update engineering-budget and validation-operation documentation with approved baselines, runner identity, invocation commands, and rollback/waiver rules.
