## MODIFIED Requirements

### Requirement: DegradationNotice Delivery
The runtime SHALL send DegradationNotice to all active sessions when the runtime degradation level changes. Existing DegradationLevel numeric values 0 through 7 MUST remain unchanged; TEXTURE_QUALITY_REDUCED=8 and EMERGENCY_RENDERING=9 SHALL be appended. The exact mapping SHALL be Normal→NORMAL, Coalesce→COALESCING_MORE, ReduceTextureQuality→TEXTURE_QUALITY_REDUCED, DisableTransparency→RENDERING_SIMPLIFIED, ShedTiles→SHEDDING_TILES, and Emergency→EMERGENCY_RENDERING. DEGRADATION_LEVEL_UNSPECIFIED MUST NOT apply a runtime or compositor policy. Render-only transitions SHALL carry an empty affected_capabilities list. DegradationNotice SHALL be transactional, delivered through a bounded per-session never-drop queue, and SHALL apply producer backpressure when a live session queue is full. degradation_notices subscriptions SHALL be delivered unconditionally and SHALL NOT be filterable.
Source: RFC 0005 §3.4, §7.1
Scope: v1-mandatory

#### Scenario: Degradation notice always delivered
- **WHEN** the runtime enters Coalesce
- **THEN** all active sessions SHALL receive DegradationNotice(level=COALESCING_MORE) regardless of subscription configuration

#### Scenario: Exact appended mapping
- **WHEN** the runtime enters ReduceTextureQuality or Emergency
- **THEN** the notice SHALL use TEXTURE_QUALITY_REDUCED or EMERGENCY_RENDERING respectively without changing any existing enum number

#### Scenario: Queue full backpressure
- **WHEN** a live session's bounded degradation-notice queue is full
- **THEN** the publisher SHALL block until capacity is available or the session closes and SHALL NOT drop or overwrite a notice

#### Scenario: Unspecified level is fail-safe
- **WHEN** a receiver decodes DEGRADATION_LEVEL_UNSPECIFIED
- **THEN** it MUST NOT infer or apply any runtime degradation action

## ADDED Requirements

### Requirement: Degradation Current-State Ordering
After SessionEstablished or an accepted SessionResumeResult, the runtime SHALL send SceneSnapshot, then the current exact DegradationNotice, then ordinary incremental events. Subscription to future degradation transitions and capture of the current notice MUST be atomic so a transition cannot be lost between them. This ordering SHALL apply even when the current level is Normal.

#### Scenario: New session ordering
- **WHEN** a new session is established while the runtime is already at ShedTiles
- **THEN** it SHALL receive SessionEstablished, SceneSnapshot, DegradationNotice(level=SHEDDING_TILES), then any later incremental event

#### Scenario: Resume ordering
- **WHEN** a resume is accepted while the runtime is already at Emergency
- **THEN** it SHALL receive SessionResumeResult, SceneSnapshot, DegradationNotice(level=EMERGENCY_RENDERING), then any later incremental event
