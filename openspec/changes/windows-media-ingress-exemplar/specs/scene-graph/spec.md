## MODIFIED Requirements

### Requirement: VideoSurfaceRef and WebRtcRequired

`VideoSurfaceRef` and the `WebRtcRequired` transport constraint SHALL be supported only for the approved Windows media ingress zone `media-pip` when explicit media enablement is active. Outside that approved zone or without media enablement, implementations SHALL treat `VideoSurfaceRef` as unsupported and SHALL reject media publication deterministically.

Source: `openspec/specs/scene-graph/spec.md`, `openspec/specs/media-webrtc-bounded-ingress/spec.md`
Scope: Active only for the accepted Windows-only one-stream media slice.

#### Scenario: approved media zone renders video surface

- **WHEN** a media ingress stream is admitted for the approved media zone
- **THEN** the scene graph MUST contain a `ZoneContent::VideoSurfaceRef` publication for the runtime-assigned surface identity
- **AND** the compositor MUST render that surface within the zone geometry

#### Scenario: non-approved zone rejects video surface

- **WHEN** an agent attempts to publish `VideoSurfaceRef` to any zone other than `media-pip`
- **THEN** the scene graph MUST reject the publish without changing existing zone occupancy

#### Scenario: existing default zones are not implicit media zones

- **WHEN** existing default zones such as `pip` or `ambient-background` accept ordinary content
- **THEN** they MUST NOT accept `VideoSurfaceRef` unless explicitly configured as the approved media zone by a later accepted change
