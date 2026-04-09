## ADDED Requirements

### Requirement: Dual-Lane Media And Device Validation

V2 SHALL validate media and device behavior through both deterministic CI-friendly rehearsal lanes and higher-fidelity real-decode/device lanes. Capability claims are not release-ready unless both validation lanes exist for the relevant scope.
Source: `about/heart-and-soul/validation.md`, `openspec/specs/media-webrtc-bounded-ingress/spec.md`
Scope: post-v1

#### Scenario: real-decode lane is required for release readiness

- **WHEN** a v2 media capability seeks release signoff
- **THEN** both deterministic and real-decode/device validation evidence are available

### Requirement: Structured Operator And Failure Observability

The runtime SHALL emit structured signals for media admission, teardown, operator override, device-state transitions, and failure recovery. These signals MUST be sufficient for CI, review, and operational diagnosis without leaking raw media payloads.
Source: `about/heart-and-soul/validation.md`, `about/heart-and-soul/privacy.md`, `openspec/specs/media-webrtc-privacy-operator-policy/spec.md`
Scope: post-v1

#### Scenario: teardown is auditable without payload leakage

- **WHEN** media or embodied presence is disabled by policy or operator action
- **THEN** the runtime emits machine-readable evidence of the transition without exposing raw media content

### Requirement: Phased Release Gates

V2 SHALL use explicit phase gates for bounded ingress, embodied presence, device-profile execution, and broader AV/orchestration. Later phases MUST NOT be declared active until earlier phases have passing validation and reconciliation evidence.
Source: `about/heart-and-soul/v1.md`, `about/heart-and-soul/validation.md`, `openspec/specs/media-webrtc-bounded-ingress/spec.md`
Scope: post-v1

#### Scenario: embodied AV is blocked behind earlier phase evidence

- **WHEN** bounded ingress or device-profile validation is incomplete
- **THEN** embodied or bidirectional AV phases remain blocked
