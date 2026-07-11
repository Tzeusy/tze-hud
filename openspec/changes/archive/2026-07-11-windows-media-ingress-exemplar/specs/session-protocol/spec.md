## MODIFIED Requirements

### Requirement: Media Ingress Session Messages

The existing `MediaIngressOpen`, `MediaIngressClose`, SDP/ICE exchange, `MediaIngressOpenResult`, `MediaIngressState`, and `MediaIngressCloseNotice` messages SHALL be active for the Windows media ingress exemplar. These messages SHALL remain unavailable when the runtime's media enablement gate is false. The session protocol MUST amend the previously deferred/reserved media message allocation so only the one-stream Windows media ingress messages named by this requirement are active; other media/control messages remain deferred.

Source: `openspec/specs/session-protocol/spec.md`, `crates/tze_hud_protocol/proto/session.proto`
Scope: Active only for the accepted Windows-only one-stream media slice.

#### Scenario: valid media open admits one stream

- **WHEN** an authenticated resident/local producer session with `media_ingress` capability sends a valid `MediaIngressOpen` for the approved `media-pip` zone
- **THEN** the runtime MUST respond with `MediaIngressOpenResult.admitted = true`
- **AND** it MUST include a nonzero `stream_epoch` and assigned `surface_id`

#### Scenario: disabled gate rejects media messages

- **WHEN** media enablement is false
- **AND** a session sends `MediaIngressOpen`
- **THEN** the runtime MUST reject the request with deterministic `MEDIA_DISABLED` policy code or equivalent structured runtime error
- **AND** it MUST NOT create decode or transport workers

#### Scenario: still-deferred media messages remain unavailable

- **WHEN** a session sends a media message outside the one-stream Windows ingress set activated by this change
- **THEN** the runtime MUST reject it as deferred or unsupported
- **AND** the rejection MUST NOT mutate active media state
