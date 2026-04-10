## ADDED Requirements

### Requirement: Embodied Presence Level

V2 SHALL introduce an embodied presence level beyond guest and resident. Embodied presence MUST carry stronger identity, operator visibility, and transport/governance requirements than resident presence.

#### Scenario: embodied session is distinguishable

- **WHEN** an embodied-capable agent connects
- **THEN** the runtime exposes an embodied presence state that is distinct from guest and resident

### Requirement: Presence-Orchestrated Media Ownership

Live media ownership SHALL be bound to explicit presence/session state rather than unmanaged stream handles. The runtime MUST be able to revoke, suspend, or downgrade media behavior through the same sovereign control model that governs other presence.

#### Scenario: embodied media tears down on presence revocation

- **WHEN** an embodied session loses authority or is revoked
- **THEN** its media behavior is torn down or downgraded deterministically

### Requirement: Multi-Agent Orchestration Remains Explicit

Any v2 orchestration across multiple agents, feeds, or presence states MUST be explicit and policy-governed. The runtime MUST NOT infer cross-agent control or feed ownership from transport activity alone.

#### Scenario: media ownership conflict is policy-resolved

- **WHEN** multiple agents contend for a media-capable surface or orchestration role
- **THEN** the runtime resolves the conflict through declared policy rather than implicit last-writer wins
