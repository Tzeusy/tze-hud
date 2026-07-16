## ADDED Requirements

### Requirement: Profile-Derived Admission Defaults
The runtime MUST derive resident-session and per-session resource admission from the frozen operational envelope. The effective default for each per-session dimension MUST be the smaller of the canonical runtime default and the selected-profile ceiling. A validated registered-agent override MUST replace the canonical default for that agent, then remain capped by the selected profile and the absolute runtime hard maximum. `profile.max_agents` MUST cap concurrent resident sessions; guest session capacity MUST remain a separate control-plane safety limit.

Source: RFC 0002 §4.2-4.3; RFC 0006 §3.1-3.2
Scope: v1-mandatory

#### Scenario: Tight profile lowers session defaults
- **WHEN** a selected profile sets `max_tiles`, `max_texture_mb`, or `max_agent_update_hz` below the canonical per-session default and a resident agent has no explicit override
- **THEN** the admitted session and every lease it receives MUST use the lower profile-derived value

#### Scenario: Registered override is below profile
- **WHEN** a registered agent has a validated resource override below the selected-profile ceiling
- **THEN** admission and every lease for that session MUST use the registered override, subject to the absolute runtime hard maximum

#### Scenario: Resident session ceiling reached
- **WHEN** the number of active resident sessions equals `profile.max_agents`
- **THEN** the next resident admission MUST be rejected with a structured resource-exhausted error naming the selected profile and resident-session ceiling

### Requirement: Aggregate Profile Admission Enforcement
Per-session checks MUST NOT be the only profile enforcement. Before admitting a resident session, granting a lease, or committing a resource-increasing mutation, the runtime MUST verify that the proposed aggregate leased-tile and agent-leased texture usage remains within the frozen operational envelope. Aggregate denial MUST be atomic and MUST NOT change session, lease, scene, or accounting state.

Source: RFC 0002 §3.2 Stage 4, §4.3; RFC 0006 §3.1
Scope: v1-mandatory

#### Scenario: Individually valid agents exceed aggregate tile ceiling
- **WHEN** each resident agent remains within its own tile budget but a proposed mutation would make total leased tiles exceed the profile aggregate
- **THEN** the complete mutation batch MUST be rejected and aggregate usage MUST remain unchanged

#### Scenario: Shared texture has distinct logical and physical charges
- **WHEN** two agents reference the same decoded texture allocation
- **THEN** each agent MUST receive the full logical texture charge while the process-resident ledger MUST charge each actual allocation only once

### Requirement: Operational Envelope Accounting Snapshot
The production runtime MUST expose a machine-readable accounting snapshot containing profile identity, every admission ceiling, every resident-memory class ceiling, current usage, and denial/eviction counters. The snapshot MUST come from the same frozen envelope and accounting ledgers used for enforcement; a reconstructed documentation-only view is not sufficient.

Source: RFC 0002 §3.2 Stage 8; heart-and-soul/validation.md DR-V3
Scope: v1-mandatory

#### Scenario: Startup snapshot traces every consumer
- **WHEN** the runtime completes startup
- **THEN** its accounting snapshot MUST identify the selected profile and report the effective limits used by session admission, lease defaults, aggregate scene resources, resource-store residency, widget caches, and font cache
