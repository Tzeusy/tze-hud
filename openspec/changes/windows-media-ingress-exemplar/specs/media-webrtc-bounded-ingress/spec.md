## MODIFIED Requirements

### Requirement: Post-v1 Activation Boundary

Media/WebRTC ingress MUST remain disabled under default runtime configuration, but the bounded ingress contract is reactivated for the `windows-media-ingress-exemplar` change when all Windows-specific activation criteria are met. Activation criteria MUST require all of: explicit Windows media enablement, approved signaling/schema behavior, approved media zone contract, runtime budget gate, privacy/operator policy, compositor `VideoSurfaceRef` rendering contract, and validation scenarios.

Source: `about/heart-and-soul/v1.md`, `openspec/specs/media-webrtc-bounded-ingress/spec.md`, `openspec/changes/windows-first-performant-runtime/specs/windows-runtime-scope/spec.md`
Scope: Active only for the accepted Windows-only one-stream media slice; all other media/WebRTC work remains deferred.

#### Scenario: default runtime remains media-disabled

- **WHEN** the runtime starts without the explicit Windows media ingress enablement state
- **THEN** no media worker threads MUST be spawned and no live media ingress MUST be accepted

#### Scenario: Windows exemplar activates after prerequisites

- **WHEN** the Windows runtime has explicit media enablement, an approved media zone, required capability grants, privacy/operator approval, and budget headroom
- **THEN** exactly one inbound video-only stream MAY be admitted

### Requirement: Directional Transport Boundary

The first active ingress slice SHALL be strictly one-way visual ingress into the compositor. The runtime MUST NOT accept upstream outbound media, negotiated bidirectional AV channels, or audio channels in this slice. The slice MUST admit at most one active inbound media stream globally unless a later OpenSpec change changes the stream limit.

Source: `openspec/specs/media-webrtc-bounded-ingress/spec.md`
Scope: Active only for the accepted Windows-only one-stream media slice.

#### Scenario: second concurrent stream is rejected

- **WHEN** one inbound media stream is already active
- **AND** a second stream is requested
- **THEN** the second request MUST be rejected with a deterministic admission failure

#### Scenario: audio-bearing ingress is rejected

- **WHEN** an ingress request includes audio channel semantics or an active audio track
- **THEN** the runtime MUST reject the request with a deterministic `AUDIO_UNSUPPORTED` admission reason
- **AND** it MUST NOT route audio to output

### Requirement: Timing, Lease, and Budget Bounds

An admitted media stream MUST remain governed by presentation timing, lease lifetime, and runtime budget limits. Missing presentation time MUST present on the next eligible compositor frame; expired presentation MUST be dropped; lease revocation, expiry, operator disable, safe mode, or budget breach MUST stop presentation within one compositor frame.

Source: `openspec/specs/media-webrtc-bounded-ingress/spec.md`
Scope: Active only for the accepted Windows-only one-stream media slice.

#### Scenario: expired media frame is dropped

- **WHEN** a media frame arrives after its expiry time
- **THEN** the frame MUST NOT replace the currently presented surface content
- **AND** the runtime MUST record a dropped-frame reason

#### Scenario: lease revocation stops media

- **WHEN** the lease governing an active media stream is revoked
- **THEN** the runtime MUST close the stream
- **AND** the compositor MUST stop presenting the surface within one compositor frame

#### Scenario: budget breach tears down media

- **WHEN** decode, upload, or presentation exceeds the configured media budget policy
- **THEN** the runtime MUST stop accepting frames for that stream
- **AND** it MUST emit a media state or close notice with a budget-related reason

### Requirement: Reconnect and Snapshot Behavior

Reconnect behavior MUST be explicit. A reconnecting producer MUST NOT inherit an active surface implicitly; it MUST perform a fresh admission flow or present an authoritative stream snapshot allowed by the current lease and policy state.

Source: `openspec/specs/media-webrtc-bounded-ingress/spec.md`
Scope: Active only for the accepted Windows-only one-stream media slice.

#### Scenario: reconnect requires current admission

- **WHEN** a producer disconnects and later reconnects
- **THEN** the runtime MUST require current enablement, capability, privacy, operator, stream-count, and budget gates before presenting media again
- **AND** stale surface content MUST NOT auto-resume without a fresh admitted stream epoch
