## MODIFIED Requirements

### Requirement: Degradation Ladder
The runtime MUST implement a 6-level degradation ladder: Level 0 Normal, Level 1 Coalesce (reduce outbound SceneEvent frequency for state-stream tiles by a configurable ratio), Level 2 ReduceTextureQuality (scale down large textures by a configurable factor), Level 3 DisableTransparency (force semi-transparent tiles to opaque, skip alpha-blend and transition work), Level 4 ShedTiles (remove lowest-priority tiles from the render pass sorted by lease_priority ASC, z_order DESC), Level 5 Emergency (render only chrome layer + highest-priority single tile). `tze_hud_runtime::DegradationController` MUST be the sole production owner of frame history and level transitions; the scene-side tracker MUST NOT execute in the production frame path.
Source: RFC 0002 §6.2
Scope: v1-mandatory

#### Scenario: Level 1 coalescing
- **WHEN** the cadence-derived entry condition is satisfied while the runtime is at Normal
- **THEN** the runtime MUST transition to Level 1 and reduce outbound state-stream notification frequency without coalescing inbound MutationBatch messages

#### Scenario: Level 4 tile shedding
- **WHEN** the runtime reaches Level 4
- **THEN** tiles MUST be sorted by (lease_priority ASC, z_order DESC) and the lowest-priority quartile removed from the render pass while remaining in the scene graph

#### Scenario: Level 5 emergency
- **WHEN** the runtime reaches Level 5
- **THEN** only the chrome layer and the single highest-priority tile MUST be rendered; all other tiles MUST be visually suppressed

### Requirement: Degradation Trigger
At startup the runtime MUST freeze one immutable degradation envelope from the validated effective Windows presentation cadence. The effective cadence SHALL be the resolved `target_fps`, capped by monitor refresh when known; measured presentation intervals MUST NOT mutate the envelope. Let `P=floor(1_000_000/effective_fps)` microseconds. The entry threshold SHALL be `min(14000, ceil(21*P/25))`, the entry duration SHALL be `10*P`, and entry SHALL require at least 10 presented samples. Evaluation MUST consume the authoritative `degradation_work_time_us` measured from active runtime/compositor work before Stage 3 through successful Stage 7 completion. Nearest-rank p95 MUST exceed the entry threshold over the elapsed window before advancing exactly one level; samples MUST reset after every transition.
Source: RFC 0002 §6.1
Scope: v1-mandatory

#### Scenario: 60 Hz calibration
- **WHEN** the effective cadence is 60 Hz
- **THEN** the immutable entry threshold MUST equal 14000 microseconds and the entry duration MUST be approximately 166 milliseconds with at least 10 presented samples

#### Scenario: Transient spike tolerance
- **WHEN** fewer than 10 samples exist or the elapsed entry duration is incomplete
- **THEN** the runtime MUST NOT trigger degradation regardless of an individual sample

#### Scenario: Sustained overbudget
- **WHEN** nearest-rank p95 exceeds the cadence-derived entry threshold for the full entry duration with at least 10 presented samples
- **THEN** the runtime MUST advance one degradation level for the next frame and clear the sample window

### Requirement: Degradation Hysteresis
The recovery threshold SHALL be `min(12000, ceil(18*P/25))`, the recovery duration SHALL be `30*P`, and active-frame recovery SHALL require at least 30 presented samples whose nearest-rank p95 is below the threshold. Recovery MUST move one level at a time and reset samples after every transition. A runtime that is provably quiescent under the canonical no-render predicate MUST recover at most one level per complete recovery duration without submitting synthetic frames; any presentation-relevant work MUST clear quiescence. The entry/recovery thresholds MUST retain a positive hysteresis band.
Source: RFC 0002 §6.3
Scope: v1-mandatory

#### Scenario: Recovery threshold
- **WHEN** the runtime is at Level 2 and active-frame p95 remains below the cadence-derived recovery threshold for the full recovery duration with at least 30 samples
- **THEN** the runtime MUST recover one level to Level 1 and clear the sample window

#### Scenario: Quiescent recovery
- **WHEN** the runtime is degraded and the canonical scheduler predicate proves no presentation-relevant work or deadline is pending for one complete recovery duration
- **THEN** it MUST recover exactly one level without encoding, submitting, presenting, or emitting a synthetic frame telemetry record

#### Scenario: Full recovery time at 60 Hz
- **WHEN** the runtime is at Level 5 and remains continuously clean or quiescent at 60 Hz
- **THEN** reaching Normal MUST require five successive 500-millisecond recovery durations

## ADDED Requirements

### Requirement: Atomic Degradation Render Policy
A transition selected from frame N MUST apply no earlier than frame N+1, while telemetry for frame N MUST report the policy actually applied to N. For every built frame the runtime MUST atomically snapshot policy level, stable tile SceneId, lease priority, z-order, scene version, and geometry epoch under the scene lock. Suppression MUST use stable SceneIds and MUST be recomputed after relevant scene or geometry changes. Degradation MUST NOT mutate scene objects or lease state, and chrome plus human override controls MUST remain renderable at every level.

#### Scenario: Atomic shedding snapshot
- **WHEN** a tile is created, removed, re-leased, or reordered after a shedding snapshot
- **THEN** the runtime MUST recompute suppression from a new locked snapshot before rendering that changed scene

#### Scenario: Recovery restores rendering only
- **WHEN** the controller recovers below a shedding level
- **THEN** previously suppressed tiles MUST render again with their scene and lease state unchanged
