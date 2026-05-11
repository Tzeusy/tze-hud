## ADDED Requirements

### Requirement: External Multi-Session Projection Supervisor
Cooperative HUD projection SHALL allow an external agent projection authority to supervise multiple launched or attached provider-neutral sessions while preserving the existing cooperative operation contract. The supervisor SHALL keep provider lifecycle orchestration outside runtime core, SHALL route only through existing v1 HUD surfaces, and SHALL NOT broaden the authority of a cooperative projection owner token beyond its owning projection ID.

#### Scenario: multiple provider-neutral sessions share the same contract
- **WHEN** the external authority manages Codex, Claude, and opencode session records concurrently
- **THEN** each session SHALL use the same provider-neutral projection identity, owner binding, audit, privacy, attention, and cleanup semantics
- **AND** provider kind SHALL NOT change authorization, input delivery, or runtime policy behavior

#### Scenario: owner token cannot route another session
- **WHEN** session A presents its owner token while attempting to publish, poll, or cleanup session B
- **THEN** the projection authority SHALL reject the operation with a stable authorization error
- **AND** it SHALL NOT expose session B transcript, pending input, lifecycle metadata, or route state

### Requirement: Cross-Surface Projection Presence
A cooperative projection MAY be represented by a zone, widget, or leased text-stream/raw-tile portal when supervised by an external authority. Zone and widget projection routes SHALL use the existing runtime zone/widget publish contracts; portal routes SHALL use the existing resident session lease and text-stream portal adapter contracts. All routes remain subject to runtime sovereignty and MUST carry content classification and attention intent.

#### Scenario: zone projection uses existing publish contract
- **WHEN** a projected session routes status text to a named zone
- **THEN** the runtime-facing command SHALL be an existing zone publish request with bounded content, TTL, classification, and agent identity
- **AND** no projection-specific compositor primitive SHALL be required

#### Scenario: widget projection uses existing widget contract
- **WHEN** a projected session routes progress to a widget instance
- **THEN** the runtime-facing command SHALL be an existing widget publish request with bounded typed parameters, TTL, classification, and agent identity
- **AND** runtime widget capability checks SHALL remain authoritative

#### Scenario: portal projection keeps lease governance
- **WHEN** a projected session routes transcript or questions to a text-stream/raw-tile portal
- **THEN** the route SHALL acquire or reuse a lease only within the authenticated session capability set
- **AND** detach, revocation, expiry, or cleanup SHALL remove stale projected tiles through existing lease cleanup behavior
