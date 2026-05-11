## ADDED Requirements

### Requirement: Windows Media Ingress Configuration

Windows media ingress SHALL be disabled by default and SHALL require explicit machine-readable configuration before any media admission, transport, decode, or worker startup occurs. Configuration SHALL define the approved media zone identity, geometry source, maximum active stream count, default content classification, operator-disable state, and canonical `media_ingress` capability grants for authenticated resident/local producers.

Source: `about/heart-and-soul/security.md`, `about/heart-and-soul/privacy.md`, `openspec/specs/configuration/spec.md`, `openspec/specs/media-webrtc-bounded-ingress/spec.md`
Scope: Active only for the accepted Windows-only one-stream media slice.

#### Scenario: default configuration disables media

- **WHEN** the runtime starts without explicit Windows media ingress configuration
- **THEN** media ingress MUST be disabled
- **AND** no media transport or decode worker MUST be spawned

#### Scenario: approved media zone is configured

- **WHEN** Windows media ingress is enabled
- **THEN** the configuration MUST name `media-pip` as the approved media zone for this change
- **AND** it MUST set `max_active_streams = 1`
- **AND** it MUST define fixed content-layer geometry owned by runtime configuration

#### Scenario: producer lacks media capability

- **WHEN** an authenticated session without `media_ingress` requests media admission
- **THEN** admission MUST be denied before transport or decode startup
- **AND** the denial MUST identify a capability failure

#### Scenario: operator disable persists

- **WHEN** the operator disables media ingress in runtime state
- **THEN** new media admissions MUST fail until an explicit re-enable action occurs
- **AND** prior streams MUST NOT auto-resume after re-enable
