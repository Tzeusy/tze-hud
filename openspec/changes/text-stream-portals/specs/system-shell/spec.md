## ADDED Requirements

### Requirement: Text Stream Portals Remain Outside Chrome

Text stream portal identity, transcript state, and interaction affordances SHALL remain outside the chrome layer unless a future shell-specific capability explicitly defines a runtime-owned portal shell surface. The current system shell MUST NOT expose agent-specific portal identities or transcript state.

#### Scenario: portal status does not become shell status metadata

- **WHEN** one or more text stream portals are active
- **THEN** the shell status area MAY expose aggregate system health only and SHALL NOT expose portal-specific identities, transcript previews, or agent-owned controls

#### Scenario: shell override still applies to portal tiles

- **WHEN** the viewer dismisses a portal tile or enters safe mode
- **THEN** the system shell SHALL override that portal surface under the same unconditional rules as any other content-layer tile
