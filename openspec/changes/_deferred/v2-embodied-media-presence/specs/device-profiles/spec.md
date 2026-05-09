## ADDED Requirements

### Requirement: Exercised Device Profiles

V2 SHALL convert mobile and glasses device profiles from schema-only declarations into exercised runtime profiles with explicit performance, interaction, and degradation contracts.

#### Scenario: constrained profile is validated as real target

- **WHEN** a mobile or glasses profile is selected in v2 validation
- **THEN** the runtime executes that profile with profile-specific budgets and assertions

### Requirement: Device-Aware Capability Negotiation

Capability negotiation SHALL account for device constraints and deployment shape. The runtime MUST be able to deny or degrade capabilities that are valid on desktop but invalid for constrained device profiles.

#### Scenario: constrained device denies unsupported media mode

- **WHEN** a constrained device profile cannot satisfy a requested media mode
- **THEN** negotiation fails or degrades explicitly instead of pretending parity with desktop

### Requirement: Upstream Composition Contract

V2 SHALL define upstream composition rules for device profiles that cannot host the full compositor locally. Upstream composition MUST preserve timing, authority, and operator visibility semantics.

#### Scenario: upstream-composed device preserves operator sovereignty

- **WHEN** a constrained device receives upstream-composed output
- **THEN** operator controls and runtime authority remain explicit and enforceable
