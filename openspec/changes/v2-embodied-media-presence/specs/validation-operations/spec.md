## ADDED Requirements

### Requirement: Dual-Lane Media And Device Validation

V2 SHALL validate media and device behavior through both deterministic CI-friendly rehearsal lanes and higher-fidelity real-decode/device lanes. Capability claims are not release-ready unless both validation lanes exist for the relevant scope.

#### Scenario: real-decode lane is required for release readiness

- **WHEN** a v2 media capability seeks release signoff
- **THEN** both deterministic and real-decode/device validation evidence are available

### Requirement: V1 Validation Backlog Carries Forward

The deferred validation backlog from the archived v1 standards program SHALL be completed as part of the v2 validation program rather than left implicit. This includes: a standalone Layer 3 benchmark binary with JSON emission and split-latency reporting, the baseline 25-scene registry, record/replay trace infrastructure, soak/leak tolerance validation, a three-agent cross-spec integration run, and calibrated reference-hardware budget gates.

#### Scenario: v2 benchmark lane inherits the v1 telemetry contract

- **WHEN** a v2 validation runner emits benchmark evidence
- **THEN** it uses a standalone benchmark path that produces machine-readable per-frame telemetry and the split latency budgets inherited from v1

#### Scenario: baseline scene corpus remains mandatory

- **WHEN** v2 media or device scenes are added
- **THEN** the validation registry still includes the full v1 baseline scene corpus and extends it rather than replacing it

#### Scenario: regression evidence spans replay and endurance

- **WHEN** a multi-agent media, device-profile, or orchestration regression is investigated
- **THEN** record/replay traces and soak/leak evidence are available for the affected flow

#### Scenario: release evidence includes calibrated scale validation

- **WHEN** a v2 capability claims release readiness
- **THEN** the evidence includes a calibrated three-agent integration run and normalized budget validation on the designated reference hardware

### Requirement: Structured Operator And Failure Observability

The runtime SHALL emit structured signals for media admission, teardown, operator override, device-state transitions, and failure recovery. These signals MUST be sufficient for CI, review, and operational diagnosis without leaking raw media payloads.

#### Scenario: teardown is auditable without payload leakage

- **WHEN** media or embodied presence is disabled by policy or operator action
- **THEN** the runtime emits machine-readable evidence of the transition without exposing raw media content

### Requirement: Cross-Spec Conformance Audits Gate V2 Expansion

Before a v2 capability is declared release-ready, the repo SHALL complete cross-spec conformance audits for canonical capability vocabulary, MCP authority-surface enforcement, and protobuf/session-envelope field allocation parity. The MCP audit MUST cover lease-free guest operations, capability-gated guest publications, and resident-authority-gated tools. V2 media, device, or embodied behavior MUST NOT rely on unresolved authority-surface or wire-contract drift inherited from v1.

#### Scenario: capability vocabulary is canonical before expansion

- **WHEN** a v2 feature references capability names across config, runtime, protocol, or MCP surfaces
- **THEN** those names match the canonical vocabulary instead of retaining legacy aliases

#### Scenario: MCP authority split remains auditable

- **WHEN** a tool surface exposes lease-free guest operations, capability-gated guest publications, and resident operations
- **THEN** each path remains explicitly enforced by the documented authority contract rather than collapsing into an implicit or ad hoc privilege model

#### Scenario: session envelope drift is blocked before release

- **WHEN** v2 session or media messages are added or revised
- **THEN** a field-allocation audit confirms they remain consistent with the declared session-protocol envelope contract

### Requirement: Phased Release Gates

V2 SHALL use explicit phase gates for bounded ingress, embodied presence, device-profile execution, and broader AV/orchestration. Later phases MUST NOT be declared active until earlier phases have passing validation and reconciliation evidence.

#### Scenario: embodied AV is blocked behind earlier phase evidence

- **WHEN** bounded ingress or device-profile validation is incomplete
- **THEN** embodied or bidirectional AV phases remain blocked
