# Specification: Media/WebRTC Post-v1 Bounded Ingress

## Purpose

Define the smallest admissible media/WebRTC capability slice after v1: one-way inbound visual media ingress into runtime-owned surfaces, with deterministic timing, explicit governance, and default-off safety.

This specification is normative for the contract envelope only. It does not replace downstream detailed specs for signaling shape, schema/snapshot deltas, zone contract details, runtime activation budgets, privacy/operator policy, compositor rendering semantics, or validation harness specifics.

Current downstream contract artifacts in this tranche:
1. Signaling shape decision (`WM-S2a`): `docs/reconciliations/webrtc_media_v1_signaling_shape_decision.md`
2. Protocol/schema + snapshot deltas (`WM-S2b`): `docs/reconciliations/webrtc_media_v1_protocol_schema_snapshot_deltas.md`
3. Runtime activation gate + budgets (`WM-S3`): `docs/reconciliations/webrtc_media_v1_runtime_activation_gate_budgets.md`

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
Media/WebRTC ingress MUST remain post-v1 and MUST NOT alter v1 doctrine defaults. The runtime MUST keep media ingress disabled unless explicit post-v1 activation criteria are met. Activation criteria MUST require all of: approved signaling contract, approved schema/snapshot deltas, approved zone transport contract, approved runtime budget gate, approved privacy/operator policy, approved compositor contract, and approved validation scenarios.
Scope: post-v1-contract-tranche

#### Scenario: v1 runtime remains media-disabled
- **WHEN** the runtime starts under v1 defaults
- **THEN** no media worker threads MUST be spawned and no live media ingress MUST be accepted

#### Scenario: activation denied when any prerequisite contract is missing
- **WHEN** post-v1 media ingress is requested but one prerequisite contract above is not approved
- **THEN** activation MUST be denied and ingress MUST remain disabled

---

### Requirement: Directional Transport Boundary
The first ingress slice MUST be strictly one-way visual ingress into the compositor. The runtime MUST NOT accept upstream outbound media, negotiated bidirectional AV channels, or audio channels in this slice. The slice MUST admit at most one active inbound media stream at a time.
Scope: post-v1-contract-tranche

#### Scenario: second concurrent stream is rejected
- **WHEN** one inbound media stream is already active and a second stream is requested
- **THEN** the second request MUST be rejected with a deterministic admission failure

#### Scenario: audio-bearing ingress is rejected
- **WHEN** an ingress request includes audio channel semantics
- **THEN** the runtime MUST reject the request and MUST NOT route audio to output

---

### Requirement: Timing Semantics for Presentation and Expiry
Ingress publications MUST declare presentation lifecycle timing in wall-clock terms (`present_at_wall_us`, `expires_at_wall_us`) and runtime scheduling MUST honor that timing against compositor presentation cadence. The runtime MUST NOT present frames before `present_at_wall_us` and MUST NOT present frames at or after `expires_at_wall_us`. Presentation MAY cease earlier if ingress is torn down due to lease revocation, budget breach, or operator/policy disable.

When ingress publication contracts include both relative expiry (`ttl_us`) and
absolute expiry (`expires_at_wall_us`), implementations MUST apply deterministic
normalization:
1. `expires_at_wall_us` is canonical for persisted snapshot/reconnect state.
2. `ttl_us` remains valid as relative input and maps to absolute expiry via:
- `present_at_wall_us + ttl_us` when `present_at_wall_us != 0`;
- otherwise `accepted_at_wall_us + ttl_us` at receiver admission time.
3. If both `ttl_us` and `expires_at_wall_us` are non-zero and resolve to
different expiry instants, publication MUST be rejected as invalid.
Scope: post-v1-contract-tranche

#### Scenario: scheduled ingress does not render early
- **WHEN** an ingress publication sets `present_at_wall_us` to 500ms in the future
- **THEN** the runtime MUST render no media frame for that publication before the declared presentation time
- **AND** first presentation MUST occur no later than one frame period after `present_at_wall_us`

#### Scenario: expired ingress is not rendered
- **WHEN** an ingress publication arrives with `expires_at_wall_us` in the past
- **THEN** the runtime MUST reject or immediately clear the publication and MUST render zero media frames for it

#### Scenario: ttl-only ingress derives canonical absolute expiry
- **WHEN** an ingress publication provides `ttl_us > 0` and `expires_at_wall_us = 0`
- **THEN** receiver MUST derive and persist effective absolute expiry using the deterministic mapping above

#### Scenario: conflicting ttl and absolute expiry is rejected
- **WHEN** an ingress publication provides both `ttl_us > 0` and `expires_at_wall_us > 0`, but ttl-derived expiry does not equal `expires_at_wall_us`
- **THEN** receiver MUST reject the publication as a malformed timing contract

### Requirement: Reconnect Snapshot Behavior For Scheduled Ingress
If an ingress publication has been accepted but its `present_at_wall_us` has not yet arrived when the session disconnects, that pending publication is runtime-local only and MUST NOT survive reconnect snapshot/resume. The reconnect snapshot MUST include only ingress publications that are already active at snapshot time. Clients that still want the scheduled ingress after resume MUST re-issue it after `SessionResumeResult`.
Scope: post-v1-contract-tranche

#### Scenario: scheduled ingress is not preserved across reconnect
- **WHEN** an ingress publication is accepted with `present_at_wall_us` in the future
- **AND** the session disconnects before the presentation time is reached
- **THEN** the reconnect snapshot MUST omit that pending publication
- **AND** the client MUST re-issue the publication after reconnect if it is still desired

---

### Requirement: Lease and Budget Coupling
Media ingress MUST be jointly governed by lease validity and runtime budget policy. An ingress request MUST be admitted only when the publisher holds the required lease scope and the runtime budget gate permits media ingress. Lease revocation or budget breach MUST trigger deterministic ingress teardown.
Scope: post-v1-contract-tranche

#### Scenario: ingress denied without lease authority
- **WHEN** an agent requests media ingress without an active lease/capability grant for the target surface
- **THEN** the runtime MUST deny admission and MUST return a structured authorization/budget error

#### Scenario: lease revocation tears down ingress
- **WHEN** the lease governing an active ingress stream is revoked
- **THEN** the stream MUST be torn down and removed from active presentation within one compositor frame

---

### Requirement: Zone and Layer Containment
Media ingress MUST be constrained to a fixed runtime-owned media zone class and
MUST NOT be routable to arbitrary zone types. The approved class for this slice
is exactly one zone type per tab with:
1. `accepted_media_types` including `VideoSurfaceRef`.
2. `transport_constraint = WebRtcRequired`.
3. `layer_attachment` fixed in configuration and not overridable by publisher
   payloads at runtime.

Publishers MUST target this zone by canonical `zone_name`; any other zone name
MUST be rejected for media ingress. The runtime MUST enforce that media
transport admission (`MediaIngressOpen`) and content publication (`ZonePublish`
with `VideoSurfaceRef`) bind to the same approved zone identity.

Reconnect semantics for this zone MUST follow snapshot-first contract limits:
1. reconnect snapshot persists only declarative active publication state for the
   approved zone (`VideoSurfaceRef` + publication metadata),
2. transport session internals are never snapshotted,
3. after resume, runtime MUST treat pre-disconnect transport as
   non-authoritative until stream-epoch reconciliation succeeds or a fresh open
   occurs.

If configuration declares a fixed media zone set (single approved media zone
name/class per tab), runtime MUST reject any attempt to open/publish media to a
zone outside that fixed set.
Scope: post-v1-contract-tranche

#### Scenario: non-media zone target is rejected
- **WHEN** an ingress publish targets a zone outside the approved media zone contract
- **THEN** the runtime MUST reject the publish and leave existing zone content unchanged

#### Scenario: layer attachment contract is enforced
- **WHEN** an ingress publish targets an approved media zone
- **THEN** the resulting visual surface MUST attach only to the layer class declared by the zone contract

#### Scenario: transport constraint mismatch is rejected
- **WHEN** media ingress open/publish targets a zone without `transport_constraint = WebRtcRequired`
- **THEN** runtime MUST reject admission as a zone transport contract violation

#### Scenario: reconnect restores publication metadata but not transport session
- **WHEN** a media zone has an active publication at disconnect and session resume succeeds
- **THEN** reconnect snapshot MUST restore only declarative media publication state for that zone
- **AND** the transport session MUST require stream-epoch reconciliation or fresh open before being treated as active

#### Scenario: fixed-zone restriction rejects alternate zone identity
- **WHEN** runtime is configured with a fixed approved media zone name per tab
- **AND** publisher attempts media ingress against a different zone name
- **THEN** runtime MUST reject the request and preserve current approved-zone state unchanged

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
