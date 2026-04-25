## ADDED Requirements

### Requirement: Validation Operations Carry-Forward

The validation framework SHALL carry forward the deferred v1 validation operations backlog as standalone baseline validation work, independent of any v2 media, device, or embodied-presence program. The carry-forward scope includes a standalone Layer 3 benchmark path with JSON emission and split-latency reporting, the baseline 25-scene registry, record/replay trace infrastructure, soak/leak tolerance validation, a three-agent cross-spec integration run, and calibrated reference-hardware budget gates.

#### Scenario: Benchmark lane inherits the v1 telemetry contract

- **WHEN** validation benchmark evidence is emitted
- **THEN** it MUST use a standalone benchmark path that produces machine-readable per-frame telemetry and reports the v1 split latency budgets separately

#### Scenario: Baseline scene corpus remains mandatory

- **WHEN** new validation scenes are added for later capabilities
- **THEN** the validation registry MUST still include the full v1 baseline scene corpus and extend it rather than replacing it

#### Scenario: Regression evidence spans replay and endurance

- **WHEN** a multi-agent, timing, or contention regression is investigated
- **THEN** record/replay traces and soak/leak evidence MUST be available for the affected flow

#### Scenario: Baseline release evidence includes calibrated scale validation

- **WHEN** a capability claims validation-framework release readiness
- **THEN** the evidence MUST include a calibrated three-agent integration run and normalized budget validation on the designated reference hardware

### Requirement: Cross-Spec Conformance Audits

Before a capability expansion relies on validation-framework evidence, the repo SHALL complete cross-spec conformance audits for canonical capability vocabulary, MCP authority-surface enforcement, and protobuf/session-envelope field allocation parity. The MCP audit MUST cover lease-free guest operations, capability-gated guest publications, and resident-authority-gated operations. Capability expansion MUST NOT rely on unresolved authority-surface or wire-contract drift inherited from earlier specs.

#### Scenario: Capability vocabulary is canonical before expansion

- **WHEN** a feature references capability names across config, runtime, protocol, MCP, or spec prose
- **THEN** those names MUST match the canonical vocabulary instead of retaining legacy aliases

#### Scenario: MCP authority split remains auditable

- **WHEN** a tool surface exposes lease-free guest operations, capability-gated guest publications, and resident operations
- **THEN** each path MUST remain explicitly enforced by the documented authority contract rather than collapsing into an implicit or ad hoc privilege model

#### Scenario: Session envelope drift is blocked before expansion

- **WHEN** session protocol messages are added or revised
- **THEN** a field-allocation audit MUST confirm they remain consistent with the declared session-protocol envelope contract

### Requirement: Canonical Validation Framework Reconciliation

The validation operations carry-forward SHALL be reconciled against `openspec/specs/validation-framework/spec.md` before archive. Existing canonical requirements that already cover archived v1 obligations SHALL be preserved rather than duplicated, and any missing carry-forward obligations SHALL be promoted with stable requirement names and scenarios.

#### Scenario: Archive avoids duplicate validation requirements

- **WHEN** this change is archived into canonical specs
- **THEN** requirements already present in `openspec/specs/validation-framework/spec.md` MUST be merged by intent rather than duplicated under new names

#### Scenario: Archived v1 backlog remains traceable

- **WHEN** reviewers inspect the archived canonical validation-framework spec
- **THEN** the standalone Layer 3 benchmark path, scene registry, record/replay, soak/leak, three-agent integration, calibrated budgets, and cross-spec audit obligations MUST remain traceable to the v1 carry-forward backlog
