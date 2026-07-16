## ADDED Requirements

### Requirement: Profile-Owned Operational Runtime Envelope
The selected display profile MUST freeze one operational runtime envelope at startup. The envelope MUST contain the profile identity, resident/embodied-session ceiling, aggregate leased-tile ceiling, aggregate logical agent-leased texture ceiling, per-agent update-rate ceiling, aggregate runtime-resident-memory ceiling, and named resident-memory sub-ceilings for resource/image residency, retained widget-asset sources, widget raster caches, and font residency. The resident-memory classes MUST be disjoint, and their sub-ceilings MUST fit within the aggregate runtime-resident-memory ceiling. The envelope MUST be immutable until restart.

Source: RFC 0006 §3.1; heart-and-soul/efficiency.md §Compute budget
Scope: v1-mandatory

#### Scenario: Selected custom profile freezes one envelope
- **WHEN** startup resolves a valid custom profile with lower admission and resident-memory values
- **THEN** configuration freeze MUST produce one immutable operational envelope containing exactly those resolved values and the custom profile identity

#### Scenario: Resident-memory sub-ceilings exceed aggregate
- **WHEN** the sum of resource/image, widget-asset-source, widget-raster-cache, and font-residency sub-ceilings exceeds the profile's aggregate runtime-resident-memory ceiling
- **THEN** configuration validation MUST reject startup with a structured profile-budget error identifying the aggregate and each sub-ceiling

#### Scenario: Hot reload attempts to change envelope
- **WHEN** a hot reload changes any operational-envelope field
- **THEN** the running envelope MUST remain unchanged and the runtime MUST report that a restart is required

### Requirement: Operational Budget Domain Separation
The profile's `max_texture_mb` MUST remain the aggregate logical ceiling for agent-leased texture resources. The profile's runtime-resident-memory budget MUST govern runtime-owned retained/decoded resource bytes, GPU image copies, retained widget-asset source bytes, widget-raster allocations, and font residency. Durable widget asset bytes governed by `[widget_runtime_assets]` MUST NOT debit resident memory until those bytes are loaded into a resident allocation. Configuration documentation and startup diagnostics MUST distinguish these domains.

Source: RFC 0006 §3.1-3.2; RFC 0011 §2.2a, §11, §13
Scope: v1-mandatory

#### Scenario: Durable asset stored but not resident
- **WHEN** a runtime widget SVG exists only in the durable asset store and has not been loaded or rasterized
- **THEN** its disk bytes MUST debit only the durable widget-asset budget and MUST NOT debit the operational resident-memory envelope

#### Scenario: Durable asset becomes resident
- **WHEN** the runtime loads and rasterizes a durable widget SVG
- **THEN** each resulting resident allocation MUST debit the matching runtime-resident-memory class while the on-disk bytes remain governed separately

#### Scenario: Durable widget registration retains a resident source copy
- **WHEN** runtime widget registration stores SVG bytes durably and also retains source bytes in protocol, scene, or renderer state
- **THEN** each distinct retained source allocation MUST debit the widget-asset-residency class while the durable file bytes remain outside the resident-memory envelope

## MODIFIED Requirements

### Requirement: Display Profile headless
The `headless` built-in profile MUST define the following budget values: `max_tiles = 256`, `max_texture_mb = 512`, `max_agents = 8`, `target_fps = 60`, `min_fps = 1`, `max_media_streams = 2`, `max_agent_update_hz = 60`. The headless profile MUST use an offscreen render target with no window. The `headless` profile MUST NOT be extendable via `[display_profile].extends`. The update-rate value is a per-agent state-stream admission ceiling; it MUST NOT be interpreted as compositor frame cadence or as permission to submit idle frames.

Source: RFC 0006 §3.4; RFC 0002 §7; heart-and-soul/efficiency.md §Compute budget
Scope: v1-mandatory

#### Scenario: headless profile budget values
- **WHEN** the runtime starts with `profile = "headless"`
- **THEN** the effective profile has max_tiles=256, max_texture_mb=512, max_agents=8, target_fps=60, min_fps=1, max_media_streams=2, max_agent_update_hz=60 and renders to an offscreen surface

#### Scenario: headless update-rate boundary
- **WHEN** a registered headless agent requests `max_update_hz = 60`
- **THEN** configuration validation accepts the request
- **AND** a request above 60 produces `CONFIG_AGENT_BUDGET_EXCEEDS_PROFILE`

#### Scenario: headless not extendable
- **WHEN** a config file sets `[display_profile] extends = "headless"`
- **THEN** startup fails with `CONFIG_HEADLESS_NOT_EXTENDABLE` structured error
