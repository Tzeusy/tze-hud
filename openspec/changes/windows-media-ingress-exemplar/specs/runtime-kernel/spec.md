## MODIFIED Requirements

### Requirement: Media Worker Pool

The media worker/decode path SHALL remain disabled by default, but MAY be spawned on Windows when the `windows-media-ingress-exemplar` activation gate passes. Spawned media workers MUST remain outside the compositor frame loop, MUST communicate via bounded channels, and MUST be torn down on operator disable, safe mode, lease revocation, expiry, or budget breach.

Source: `about/heart-and-soul/architecture.md`, `openspec/specs/runtime-kernel/spec.md`, `openspec/specs/media-webrtc-bounded-ingress/spec.md`
Scope: Active only for the accepted Windows-only one-stream media slice.

#### Scenario: worker pool is gated

- **WHEN** media enablement is false
- **THEN** no media worker pool or decode pipeline MUST be spawned

#### Scenario: worker teardown is frame-bounded

- **WHEN** an admitted media stream is revoked or disabled
- **THEN** presentation MUST stop within one compositor frame
- **AND** worker teardown MUST release media resources deterministically

### Requirement: Decoded Frame Upload Contract

Decoded media frames for the Windows media slice MUST cross into the compositor through runtime-owned `VideoFrame`/surface upload APIs. The implementation MUST specify build features and platform dependencies for real decode paths, and MUST provide deterministic placeholder behavior when decode support or first-frame data is absent.

Source: `crates/tze_hud_compositor/src/video_surface.rs`, `crates/tze_hud_runtime/src/gst_decode_pipeline.rs`, `openspec/specs/runtime-kernel/spec.md`
Scope: Active only for the accepted Windows-only one-stream media slice.

#### Scenario: decoded frame replaces placeholder

- **WHEN** an admitted stream produces a decoded RGBA frame for its assigned surface
- **THEN** the compositor MUST upload that frame into a runtime-owned texture
- **AND** the approved media zone MUST render the latest accepted frame instead of the placeholder

#### Scenario: decode dependency is absent

- **WHEN** the runtime is built or launched without required media decode dependencies
- **THEN** media ingress MUST remain disabled or fail admission with a structured dependency reason
- **AND** the compositor MUST remain stable and continue rendering a deterministic placeholder or empty state
