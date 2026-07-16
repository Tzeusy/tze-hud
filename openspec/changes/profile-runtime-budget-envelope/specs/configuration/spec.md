## ADDED Requirements

### Requirement: Profile-Owned Operational Runtime Envelope
The selected display profile MUST freeze one operational runtime envelope at startup. The envelope MUST contain the profile identity, resident-session ceiling, aggregate leased-tile ceiling, aggregate agent-leased texture ceiling, per-agent update-rate ceiling, aggregate runtime-resident-memory ceiling, and named resident-memory sub-ceilings for the resource store, widget raster caches, and font cache. Runtime-owned resident-memory sub-ceilings MUST fit within the aggregate runtime-resident-memory ceiling. The envelope MUST be immutable until restart.

Source: RFC 0006 §3.1; heart-and-soul/efficiency.md §Compute budget
Scope: v1-mandatory

#### Scenario: Selected custom profile freezes one envelope
- **WHEN** startup resolves a valid custom profile with lower admission and resident-memory values
- **THEN** configuration freeze MUST produce one immutable operational envelope containing exactly those resolved values and the custom profile identity

#### Scenario: Resident-memory sub-ceilings exceed aggregate
- **WHEN** the sum of resource-store, widget-cache, and font-cache resident-memory sub-ceilings exceeds the profile's aggregate runtime-resident-memory ceiling
- **THEN** configuration validation MUST reject startup with a structured profile-budget error identifying the aggregate and each sub-ceiling

#### Scenario: Hot reload attempts to change envelope
- **WHEN** a hot reload changes any operational-envelope field
- **THEN** the running envelope MUST remain unchanged and the runtime MUST report that a restart is required

### Requirement: Operational Budget Domain Separation
The profile's `max_texture_mb` MUST remain the aggregate ceiling for agent-leased texture resources. The profile's runtime-resident-memory budget MUST govern runtime-owned decoded-resource, widget-raster, and font-cache allocations. Durable widget asset bytes governed by `[widget_runtime_assets]` MUST NOT debit resident memory until those bytes are loaded into a resident allocation. Configuration documentation and startup diagnostics MUST distinguish these domains.

Source: RFC 0006 §3.1-3.2; RFC 0011 §2.2a, §11, §13
Scope: v1-mandatory

#### Scenario: Durable asset stored but not resident
- **WHEN** a runtime widget SVG exists only in the durable asset store and has not been loaded or rasterized
- **THEN** its disk bytes MUST debit only the durable widget-asset budget and MUST NOT debit the operational resident-memory envelope

#### Scenario: Durable asset becomes resident
- **WHEN** the runtime loads and rasterizes a durable widget SVG
- **THEN** each resulting resident allocation MUST debit the matching runtime-resident-memory class while the on-disk bytes remain governed separately
