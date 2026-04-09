# Specification: Media/WebRTC Post-v1 Bounded Ingress

## Purpose

Define the smallest admissible media/WebRTC capability slice after v1: one-way inbound visual media ingress into runtime-owned surfaces, with deterministic timing, explicit governance, and default-off safety.

This specification is normative for the contract envelope only. It does not replace downstream detailed specs for signaling shape, schema/snapshot deltas, zone contract details, runtime activation budgets, privacy/operator policy, compositor rendering semantics, or validation harness specifics.

---

## Bounded Slice Definition

The bounded ingress slice is all of the following, and nothing broader:

1. Exactly one inbound visual media stream at a time.
2. Runtime-owned rendering target (zone-managed surface), not agent-owned pixel loops.
3. One-way ingress only (producer -> runtime compositor).
4. Default-off activation, gated by capability, lease, and budget policy.
5. No v1 behavior change unless post-v1 media activation gates are explicitly enabled.

---

## Explicit Non-Goals

The bounded ingress slice MUST NOT include:

1. Bidirectional AV session semantics.
2. Audio capture, routing, mixing, or playback policy.
3. Multi-feed compositing or adaptive bitrate orchestration.
4. Embodied presence negotiation and session choreography.
5. Browser/remoting transport expansion (e.g., WebTransport promotion).

---

## Deferred Items

The following remain deferred beyond this slice and require separate capability contracts:

1. Bidirectional WebRTC call/session state machines.
2. Media-plane audio policy and household-aware routing.
3. Multi-stream scheduling, priority arbitration, and dynamic feed layouts.
4. Cross-device/mobile media profile negotiation.
5. Clocked subtitle/cue synchronization against live media clocks.

---

## Requirements

### Requirement: Post-v1 Activation Boundary
Media/WebRTC ingress SHALL remain post-v1 and SHALL NOT alter v1 doctrine defaults. The runtime MUST keep media ingress disabled unless explicit post-v1 activation criteria are met. Activation criteria MUST require all of: approved signaling contract, approved schema/snapshot deltas, approved zone transport contract, approved runtime budget gate, approved privacy/operator policy, approved compositor contract, and approved validation scenarios.
Scope: post-v1-contract-tranche

#### Scenario: v1 runtime remains media-disabled
- **WHEN** the runtime starts under v1 defaults
- **THEN** no media worker threads SHALL be spawned and no live media ingress SHALL be accepted

#### Scenario: activation denied when any prerequisite contract is missing
- **WHEN** post-v1 media ingress is requested but one prerequisite contract above is not approved
- **THEN** activation MUST be denied and ingress MUST remain disabled

---

### Requirement: Directional Transport Boundary
The first ingress slice SHALL be strictly one-way visual ingress into the compositor. The runtime MUST NOT accept upstream outbound media, negotiated bidirectional AV channels, or audio channels in this slice. The slice MUST admit at most one active inbound media stream at a time.
Scope: post-v1-contract-tranche

#### Scenario: second concurrent stream is rejected
- **WHEN** one inbound media stream is already active and a second stream is requested
- **THEN** the second request MUST be rejected with a deterministic admission failure

#### Scenario: audio-bearing ingress is rejected
- **WHEN** an ingress request includes audio channel semantics
- **THEN** the runtime MUST reject the request and MUST NOT route audio to output

---

### Requirement: Timing Semantics for Presentation and Expiry
Ingress publications MUST declare presentation lifecycle timing in wall-clock terms (`present_at_wall_us`, `expires_at_wall_us`) and runtime scheduling MUST honor that timing against compositor presentation cadence. The runtime MUST NOT present frames before `present_at_wall_us` and MUST stop presenting at or after `expires_at_wall_us`.
Scope: post-v1-contract-tranche

#### Scenario: scheduled ingress does not render early
- **WHEN** an ingress publication sets `present_at_wall_us` to 500ms in the future
- **THEN** the runtime MUST render no media frame for that publication before the declared presentation time
- **AND** first presentation MUST occur no later than two compositor frames after `present_at_wall_us`

#### Scenario: expired ingress is not rendered
- **WHEN** an ingress publication arrives with `expires_at_wall_us` in the past
- **THEN** the runtime MUST reject or immediately clear the publication and MUST render zero media frames for it

---

### Requirement: Lease and Budget Coupling
Media ingress MUST be jointly governed by lease validity and runtime budget policy. An ingress request MUST be admitted only when the publisher holds the required lease scope and the runtime budget gate permits media ingress. Lease revocation or budget breach MUST trigger deterministic ingress teardown.
Scope: post-v1-contract-tranche

#### Scenario: ingress denied without lease authority
- **WHEN** an agent requests media ingress without an active lease/capability grant for the target surface
- **THEN** the runtime MUST deny admission and MUST return a structured authorization/budget error

#### Scenario: lease revocation tears down ingress
- **WHEN** the lease governing an active ingress stream is revoked
- **THEN** the stream MUST be torn down and removed from active presentation within 100ms

---

### Requirement: Zone and Layer Containment
Media ingress MUST be constrained to an approved media zone contract and declared layer attachment semantics. Ingress MUST NOT be routable to arbitrary zones or bypass runtime-owned layering rules.
Scope: post-v1-contract-tranche

#### Scenario: non-media zone target is rejected
- **WHEN** an ingress publish targets a zone outside the approved media zone contract
- **THEN** the runtime MUST reject the publish and leave existing zone content unchanged

#### Scenario: layer attachment contract is enforced
- **WHEN** an ingress publish targets an approved media zone
- **THEN** the resulting visual surface MUST attach only to the layer class declared by the zone contract

---

### Requirement: Privacy and Operator Safety Assumptions
Every ingress publication MUST carry content classification and MUST be processed through privacy/viewer/operator policy before presentation. Operator disable controls MUST immediately override ingress regardless of publisher intent.
Scope: post-v1-contract-tranche

#### Scenario: missing classification is rejected
- **WHEN** an ingress publication omits required content classification metadata
- **THEN** the runtime MUST reject the publication and report a structured policy error

#### Scenario: operator disable wins immediately
- **WHEN** an operator triggers media disable while ingress is active
- **THEN** media presentation MUST cease within one compositor frame and ingress MUST remain disabled until explicitly re-enabled

---

### Requirement: Measurable Validation Readiness
No implementation work for media ingress MAY be treated as merge-ready unless validation scenarios prove this bounded contract end-to-end. Validation MUST include at least: single-stream admission limits, timing-window compliance, lease-revocation teardown, policy-gated rejection paths, and operator disable behavior.
Scope: post-v1-contract-tranche

#### Scenario: acceptance suite proves bounded ingress invariants
- **WHEN** the bounded ingress validation suite executes
- **THEN** all required invariants above MUST produce machine-verifiable pass/fail outcomes

#### Scenario: failed invariant blocks readiness
- **WHEN** any required bounded-ingress invariant fails
- **THEN** media ingress implementation readiness MUST be rejected until the failure is resolved
