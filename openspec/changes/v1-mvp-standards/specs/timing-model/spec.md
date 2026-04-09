# Timing Model Specification

Domain: FOUNDATION
Source RFC: 0003 (Timing Model)

---

## ADDED Requirements

### Requirement: Clock Domain Separation
tze_hud MUST recognize four distinct clock domains: Display clock (vsync-driven, ~16.6ms resolution, master for composition), Monotonic system clock (OS monotonic, microsecond resolution, for internal measurements), Network clock (agent-supplied UTC microseconds, for present_at/expires_at), and Media clock (GStreamer pipeline, nanoseconds, post-v1 only). These domains MUST NOT be interchangeable.
Source: RFC 0003 §1.1
Scope: v1-mandatory

#### Scenario: Display clock as master
- **WHEN** the compositor makes presentation timing decisions
- **THEN** the display clock (vsync signal) MUST be the master reference; no mutation is ever applied "between frames"

#### Scenario: Monotonic clock for internal use
- **WHEN** the compositor measures stage latencies, deadline calculations, or lease TTL
- **THEN** it MUST use the monotonic system clock, which MUST never jump backward

### Requirement: Clock Domain Naming Convention
All timestamp fields MUST encode their clock domain in the field name suffix: `_wall_us` for UTC wall clock (network clock domain) and `_mono_us` for monotonic system clock. Fields MUST NOT mix wall and mono in the same field. Fields without a clock-domain suffix are deltas or frame-relative values, not timestamps.
Source: RFC 0003 §1.3, Review History (RFC 0005 Round 6)
Scope: v1-mandatory

#### Scenario: Field naming enforcement
- **WHEN** a new timestamp field is added to any proto or Rust struct
- **THEN** it MUST use `_wall_us` for UTC wall clock or `_mono_us` for monotonic clock; generic `_us` suffix without domain indicator MUST NOT be used for timestamps

### Requirement: Vsync Sync Point
At the start of each frame, the compositor MUST record a canonical sync point triple: frame_number, vsync_mono_us (monotonic clock at vsync), and vsync_wall_us (UTC wall clock at vsync, sampled once and cached). This triple MUST be included in the TelemetryRecord for each frame.
Source: RFC 0003 §1.3
Scope: v1-mandatory

#### Scenario: Per-frame sync point
- **WHEN** a new frame begins at vsync
- **THEN** the compositor MUST record (frame_number, vsync_mono_us, vsync_wall_us) and include it in the FrameTimingRecord

### Requirement: Session Clock Sync Point
During agent session establishment (gRPC handshake), the compositor MUST record session_open_mono_us and session_open_wall_us. The difference MUST be the session's initial clock-skew estimate.
Source: RFC 0003 §1.3
Scope: v1-mandatory

#### Scenario: Session handshake sync
- **WHEN** an agent establishes a gRPC session
- **THEN** the compositor MUST record session_open_mono_us and session_open_wall_us and compute the initial clock-skew estimate for that session

### Requirement: Arrival Time Is Not Presentation Time
Every payload with timing significance MUST carry explicit timestamp fields. The compositor MUST never infer "show this now" from "this just arrived." Mutations with present_at_wall_us in the future MUST be held in a pending queue until the target frame. Mutations without present_at_wall_us (or present_at_wall_us = 0) SHALL be applied at the earliest available frame.
Source: RFC 0003 §3.1
Scope: v1-mandatory

#### Scenario: Future-scheduled mutation
- **WHEN** an agent submits a MutationBatch with present_at_wall_us = T+500ms
- **THEN** the compositor MUST store the mutation and NOT apply it until a frame whose vsync_wall_us >= T+500ms

#### Scenario: Immediate mutation
- **WHEN** an agent submits a MutationBatch with present_at_wall_us = 0
- **THEN** the compositor MUST apply the mutation at the earliest available frame (current frame if before Stage 3 cutoff, otherwise next frame)

### Requirement: Timestamp Resolution and Format
All timestamps in the public API MUST use UTC microseconds since Unix epoch as uint64 (network clock domain). Zero (0) MUST always mean "not set." Microsecond resolution MUST be used to preserve agent intent; in practice, timestamps are quantized to frame boundaries during evaluation.
Source: RFC 0003 §3.2, §3.3
Scope: v1-mandatory

#### Scenario: Zero timestamp semantics
- **WHEN** a timestamp field contains value 0
- **THEN** the runtime MUST interpret it as "not set" (e.g., present_at_wall_us=0 means immediate, expires_at_wall_us=0 means no expiry)

### Requirement: Timing Fields on Payloads
All meaningful realtime payloads MUST carry the following timing fields as applicable: present_at_wall_us (or relative scheduling alternative), expires_at_wall_us, created_at_wall_us, sequence (monotonic per-source ordering), priority (shedding priority under load), coalesce_key (for state-stream dedup), and sync_group. The oneof schedule (present_at_wall_us, after_us, frames_from_now, next_frame) MUST be mutually exclusive; setting more than one MUST be rejected with RELATIVE_SCHEDULE_CONFLICT.
Source: RFC 0003 §3.2
Scope: v1-mandatory

#### Scenario: Timing hints on MutationBatch
- **WHEN** an agent submits a MutationBatch
- **THEN** it MAY set present_at_wall_us (or a relative scheduling field), expires_at_wall_us, sequence, priority, coalesce_key, and sync_group; the compositor MUST interpret and enforce each field's semantics

#### Scenario: Mutual exclusivity of schedule fields
- **WHEN** an agent sets both present_at_wall_us and after_us in the same message
- **THEN** the compositor MUST reject the batch with RELATIVE_SCHEDULE_CONFLICT

### Requirement: Timestamp Precedence
present_at_wall_us (or its relative scheduling equivalent) MUST follow a precedence hierarchy: node-level overrides tile-level, tile-level overrides batch-level, batch-level is the default for items that do not set their own.
Source: RFC 0003 §3.2
Scope: v1-mandatory

#### Scenario: Node-level override
- **WHEN** a TextMarkdownNode has present_at_wall_us = T1 and its parent tile has present_at_wall_us = T2
- **THEN** the node MUST use T1 for its presentation scheduling, ignoring T2

### Requirement: Frame Quantization
A present_at_wall_us timestamp T is "in scope" for frame F if and only if T <= frame_F_vsync_wall_us. This is a strict no-earlier-than rule: content MUST never be presented before its declared present_at_wall_us. A timestamp falling between two vsync times MUST be held until the next frame whose vsync is at or after T.
Source: RFC 0003 §3.3
Scope: v1-mandatory

#### Scenario: No-earlier-than guarantee
- **WHEN** a mutation has present_at_wall_us = V + 1ms (where V is vsync time of frame F)
- **THEN** the mutation MUST NOT be applied at frame F; it MUST wait until frame F+1 whose frame_F_vsync_wall_us >= present_at_wall_us

#### Scenario: Presentation accuracy
- **WHEN** a mutation has present_at_wall_us = T
- **THEN** it MUST be applied no earlier than T and no later than T + one frame period (16.6ms at 60fps)

### Requirement: Timestamp Validation
The compositor MUST apply validation rules to all agent-supplied timestamps: present_at_wall_us more than 60 seconds in the past MUST be rejected with TIMESTAMP_TOO_OLD; present_at_wall_us more than max_future_schedule_us (default 300,000,000 us = 5 minutes) in the future MUST be rejected with TIMESTAMP_TOO_FUTURE; expires_at_wall_us <= present_at_wall_us MUST be rejected with TIMESTAMP_EXPIRY_BEFORE_PRESENT; DROP_IF_LATE with non-EPHEMERAL_REALTIME class MUST be rejected with INVALID_DELIVERY_POLICY; clock skew > 1s MUST be rejected with CLOCK_SKEW_EXCESSIVE.
Source: RFC 0003 §3.5
Scope: v1-mandatory

#### Scenario: Stale timestamp rejection
- **WHEN** an agent submits a mutation with present_at_wall_us more than 60 seconds before session_open_wall_us
- **THEN** the compositor MUST reject with TIMESTAMP_TOO_OLD

#### Scenario: Future timestamp rejection
- **WHEN** an agent submits a mutation with present_at_wall_us more than 5 minutes in the future
- **THEN** the compositor MUST reject with TIMESTAMP_TOO_FUTURE

#### Scenario: Expiry before presentation
- **WHEN** an agent submits a tile with expires_at_wall_us <= present_at_wall_us
- **THEN** the compositor MUST reject with TIMESTAMP_EXPIRY_BEFORE_PRESENT

### Requirement: Sync Group Membership and Lifecycle
Sync groups MUST be named scene-graph objects with SceneId (UUIDv7) identity, explicit lifecycle (create, join, leave, destroy), and an owner_namespace. Creating a sync group MUST require manage_sync_groups capability. A tile MUST belong to at most one sync group at a time. Joining is via UpdateTileSyncGroup; leaving is via setting sync_group to None. Destruction occurs on explicit delete or when the last member leaves.
Source: RFC 0003 §2.2, §2.3
Scope: v1-mandatory

#### Scenario: Create sync group
- **WHEN** an agent with manage_sync_groups capability submits CreateSyncGroup
- **THEN** the group MUST be created with the specified id, name, and commit_policy

#### Scenario: Tile single group membership
- **WHEN** a tile already belongs to sync group A and is assigned to sync group B
- **THEN** the tile MUST leave group A and join group B

#### Scenario: Auto-destroy on empty
- **WHEN** the last member tile leaves a sync group
- **THEN** the sync group MUST be destroyed and removed from the scene graph

### Requirement: Sync Group Commit Policies
Two commit policies MUST be supported: AllOrDefer (all members must have pending mutations before any can be applied; if incomplete at frame close, the group defers) and AvailableMembers (apply whatever subset of members have pending mutations; no deferral).
Source: RFC 0003 §2.2, §2.3
Scope: v1-mandatory

#### Scenario: AllOrDefer complete
- **WHEN** all members of an AllOrDefer group have pending mutations in frame N's intake window
- **THEN** all mutations MUST be applied atomically in frame N's Stage 4

#### Scenario: AllOrDefer incomplete
- **WHEN** only some members of an AllOrDefer group have pending mutations
- **THEN** the entire group MUST be deferred to the next frame

#### Scenario: AvailableMembers partial
- **WHEN** only some members of an AvailableMembers group have pending mutations
- **THEN** available members' mutations MUST be applied; absent members remain unchanged

### Requirement: AllOrDefer Force-Commit
An AllOrDefer sync group that has been deferred for max_defer_frames (default 3, configurable via SyncGroupConfig) consecutive frames MUST trigger a force-commit. Force-commit MUST: partition members into present-and-ready vs absent, apply present-and-ready mutations atomically, discard absent-member deferred mutations (not carry them forward), reset deferred_frames_count to 0, and emit SyncGroupForceCommitEvent. The deferred_frames_count MUST only increment when at least one member has a pending mutation and at least one is absent.
Source: RFC 0003 §2.4, §2.4.1
Scope: v1-mandatory

#### Scenario: Force-commit after max deferral
- **WHEN** an AllOrDefer group is incomplete for 3 consecutive frames (default max_defer_frames)
- **THEN** the compositor MUST force-commit: apply present members' mutations, discard absent members' deferred mutations, and emit SyncGroupForceCommitEvent

#### Scenario: Deferred_frames_count not incremented when idle
- **WHEN** an AllOrDefer group has no pending mutations for any member
- **THEN** deferred_frames_count MUST NOT be incremented

#### Scenario: Post-force-commit recovery
- **WHEN** a force-commit has fired
- **THEN** the group MUST resume AllOrDefer evaluation from deferred_frames_count = 0 on the next frame

### Requirement: Sync Group Owner Disconnect
When the agent session owning a sync group's namespace closes, the compositor MUST treat the group as orphaned: emit SyncGroupOrphanedEvent, release all member tiles from the group, and destroy the group after a 5-second grace period. If the owner reconnects within the grace period, the destruction MUST be cancelled.
Source: RFC 0003 §2.3
Scope: v1-mandatory

#### Scenario: Owner disconnect orphan
- **WHEN** the agent that created sync group G disconnects
- **THEN** the compositor MUST emit SyncGroupOrphanedEvent, release all member tiles, and destroy G after 5 seconds

#### Scenario: Owner reconnect within grace
- **WHEN** the owner reconnects within 5 seconds of disconnect
- **THEN** the pending group destruction MUST be cancelled

### Requirement: Sync Group Resource Governance
Maximum sync groups per agent namespace MUST be 16. Maximum tiles per sync group MUST be 64. An agent MUST NOT place another agent's tiles into a sync group (ownership check required).
Source: RFC 0003 §2.5
Scope: v1-mandatory

#### Scenario: Sync group limit
- **WHEN** an agent has 16 sync groups and attempts to create a 17th
- **THEN** the compositor MUST reject the creation

### Requirement: Sync Drift Budget
Sync group member arrival spread MUST be tracked as a telemetry budget: sync_group_max_drift_us in FrameTimingRecord records the worst mutation-arrival spread within any committed sync group. If this value exceeds sync_drift_budget_us (default 500us, from validation.md), the compositor MUST emit a sync_drift_high telemetry alert and activate the staleness indicator for the affected tiles.
Source: RFC 0003 §4.2
Scope: v1-mandatory

#### Scenario: Sync drift within budget
- **WHEN** all sync group members' mutations arrive within 500us of each other
- **THEN** sync_drift_budget_exceeded MUST be false in FrameTimingRecord

#### Scenario: Sync drift exceeded
- **WHEN** sync group members' mutations arrive with 800us spread
- **THEN** sync_drift_budget_exceeded MUST be true and the staleness indicator MUST be activated for the slow member's tiles

### Requirement: Clock Drift Detection and Correction
The compositor MUST maintain a sliding window of the last 32 agent timestamps for clock-skew estimation, using the median of (agent_ts - compositor_ts). When drift is within tolerance (<=100ms), the compositor MUST apply a signed offset correction transparently. Clock jump detection: if consecutive samples differ by more than clock_jump_detection_ms (default 50ms), the estimation window MUST be reset to the current single sample.
Source: RFC 0003 §4.3, §4.4
Scope: v1-mandatory

#### Scenario: Drift correction applied
- **WHEN** an agent's clock is 50ms ahead of the compositor
- **THEN** the compositor MUST apply a -50ms correction to the agent's timestamps transparently

#### Scenario: Clock jump resets window
- **WHEN** consecutive skew samples differ by more than 50ms
- **THEN** the compositor MUST reset the estimation window to the current single sample

### Requirement: Clock Drift Enforcement
Drift > 100ms MUST produce a CLOCK_SKEW_HIGH warning in the session event stream and telemetry; timestamps MUST still be applied with correction. Drift > 1s MUST reject new mutation batches with CLOCK_SKEW_EXCESSIVE. After three consecutive ClockSync failures to bring drift within tolerance, the session MUST be terminated.
Source: RFC 0003 §4.5
Scope: v1-mandatory

#### Scenario: High drift warning
- **WHEN** agent clock drift is 200ms
- **THEN** the compositor MUST emit CLOCK_SKEW_HIGH warning and continue processing with correction

#### Scenario: Excessive drift rejection
- **WHEN** agent clock drift exceeds 1 second
- **THEN** the compositor MUST reject new mutation batches with CLOCK_SKEW_EXCESSIVE

### Requirement: Presentation Deadline
Mutations with present_at_wall_us = T MUST be held in a per-agent pending queue (sorted by present_at_wall_us ascending, max depth 256 per agent) until frame F where frame_F_vsync_wall_us >= T. The drain condition MUST enforce the no-earlier-than guarantee directly. Late arrivals (after Stage 3 closes) MUST be deferred to the next frame by default. EphemeralRealtime class with DROP_IF_LATE delivery_policy MAY be dropped instead of deferred.
Source: RFC 0003 §5.1, §5.2, §5.3
Scope: v1-mandatory

#### Scenario: Pending queue drain
- **WHEN** a frame's vsync_wall_us >= a mutation's present_at_wall_us
- **THEN** the mutation MUST be extracted from the pending queue and forwarded to Stage 4

#### Scenario: Pending queue full
- **WHEN** an agent's pending queue has 256 entries and a new mutation would exceed this
- **THEN** the compositor MUST reject the mutation with PENDING_QUEUE_FULL

#### Scenario: DROP_IF_LATE ephemeral
- **WHEN** an EPHEMERAL_REALTIME mutation with DROP_IF_LATE arrives after Stage 3 closes
- **THEN** the compositor MUST discard it rather than deferring to the next frame

### Requirement: Expiration Policy
A tile with expires_at_wall_us = T MUST be automatically removed at the first frame F where frame_F_vsync_wall_us >= T. Expiry evaluation MUST happen at Stage 4 using a min-heap (O(expired_items) per frame). Expiry MUST be non-negotiable under load: it MUST run even during degradation Level 4 or 5. Expiry and sync groups: an expiring tile MUST be removed from its sync group before deletion.
Source: RFC 0003 §5.4
Scope: v1-mandatory

#### Scenario: Tile expiry
- **WHEN** a tile's expires_at_wall_us has been reached
- **THEN** the compositor MUST remove the tile from the scene at Stage 4 and emit a TileExpired event

#### Scenario: Expiry during degradation
- **WHEN** the compositor is at degradation Level 5 (Emergency)
- **THEN** expiry evaluation MUST still run at Stage 4; expired tiles MUST be removed

#### Scenario: Expiry heap efficiency
- **WHEN** 1000 tiles exist but only 2 have expired
- **THEN** the expiry evaluation MUST touch only the 2 expired tiles (O(expired_items))

### Requirement: Relative Scheduling Primitives
The compositor MUST support three relative scheduling fields as wire-level convenience sugar: after_us (N microseconds from compositor monotonic clock at Stage 3 intake), frames_from_now (N display frames from current frame), and next_frame (sugar for frames_from_now = 1). These MUST be converted to absolute present_at_wall_us at Stage 3 intake and MUST never be stored in the scene graph, telemetry, or internal state.
Source: RFC 0003 §5.3.1
Scope: v1-mandatory

#### Scenario: after_us conversion
- **WHEN** an agent submits a mutation with after_us = 500,000 (500ms)
- **THEN** the compositor MUST compute target_mono_us = monotonic_us_at_intake + 500,000, convert to wall-clock via the session clock-skew estimate (present_at_wall_us = target_mono_us + skew_us), and enter the mutation into the pending queue at Stage 3 intake

#### Scenario: frames_from_now conversion
- **WHEN** an agent submits a mutation with frames_from_now = 3 at frame N
- **THEN** the compositor MUST convert it to present_at_wall_us corresponding to frame N+3 vsync and apply at that frame

#### Scenario: next_frame equivalence
- **WHEN** an agent submits next_frame = true
- **THEN** it MUST be treated identically to frames_from_now = 1; the mutation MUST NOT be applied in the current frame

#### Scenario: Relative fields never stored
- **WHEN** a relative scheduling field is converted to present_at_wall_us
- **THEN** the raw relative value MUST NOT appear in any scene graph state, telemetry record, or stored mutation

### Requirement: Session Close Pending Queue Flush
When an agent session closes (gracefully or ungracefully), all entries in that session's pending queue MUST be discarded. The compositor MUST NOT apply pending mutations from a closed session. A reconnecting agent starts with an empty queue and MUST retransmit any desired future-scheduled mutations.
Source: RFC 0003 §5.3
Scope: v1-mandatory

#### Scenario: Pending queue discarded on disconnect
- **WHEN** an agent session closes with 10 pending mutations
- **THEN** all 10 pending mutations MUST be discarded and MUST NOT be applied after reconnect

### Requirement: Staleness Indicators
The compositor MUST detect and indicate two staleness conditions: content staleness (no mutation within tile_stale_threshold_ms, default 5000ms, for STATE_STREAM/TRANSACTIONAL tiles with a registered agent session) and sync group staleness (arrival spread exceeds sync_drift_budget_us). Staleness MUST be indicated via a chrome-layer visual badge that does not affect tile content. The indicator MUST be cleared when a new valid mutation arrives or the agent disconnects.
Source: RFC 0003 §4.7
Scope: v1-mandatory

#### Scenario: Content staleness
- **WHEN** a STATE_STREAM tile receives no mutation for 5 seconds
- **THEN** the compositor MUST activate the staleness indicator badge for that tile

#### Scenario: Staleness cleared on mutation
- **WHEN** a stale tile receives a new valid mutation
- **THEN** the staleness indicator MUST be cleared immediately

### Requirement: Injectable Clock
All timing paths in the compositor MUST use an injectable clock source (Clock trait with now_us() and monotonic_us() methods), not direct OS clock calls. A SystemClock (production) and SimulatedClock (tests, manually advanced) MUST be provided. The Clock trait MUST NOT have advance or set_time methods; time advancement is a concern of test implementations only.
Source: RFC 0003 §8.1
Scope: v1-mandatory

#### Scenario: Deterministic test timing
- **WHEN** a test uses SimulatedClock and advances time by 100ms
- **THEN** all timing-dependent behavior (present_at evaluation, expiry, staleness) MUST respond to the simulated time without wall-clock interference

#### Scenario: Clock trait is observation-only
- **WHEN** the Clock trait is used in production code
- **THEN** it MUST only provide now_us() and monotonic_us(); no mutation methods

### Requirement: Freeze Override Timing Behavior
During freeze (RFC 0007 section 4.3): present_at pending queue drain MUST be suspended; expires_at expiry heap evaluation MUST be suspended; sync group deferred_frames_count MUST NOT increment; tile_stale_threshold_ms timer MUST be suspended; clock-skew estimation window MUST continue updating. On unfreeze: all queued mutations whose present_at_wall_us has passed MUST be applied immediately; all expired tiles MUST be expired immediately; staleness timers MUST resume from pre-freeze values; ephemeral DROP_IF_LATE mutations whose present_at_wall_us has passed MUST be dropped.
Source: RFC 0003 §5.6.1
Scope: v1-mandatory

#### Scenario: Freeze suspends expiry
- **WHEN** the scene is frozen and a tile's expires_at_wall_us passes
- **THEN** the tile MUST NOT be expired during freeze; it MUST be expired in the first post-unfreeze Stage 4

#### Scenario: Freeze suspends staleness timer
- **WHEN** a tile has been idle for 4800ms and a 2-second freeze occurs
- **THEN** the tile MUST NOT become stale until 200ms after unfreeze (not immediately after unfreeze)

#### Scenario: Unfreeze pending queue flush
- **WHEN** the scene unfreezes and mutations in the queue have present_at_wall_us in the past
- **THEN** all such mutations MUST be applied in the first post-unfreeze Stage 3/4

### Requirement: Safe Mode Timing Behavior
During safe mode (RFC 0007 section 5): present_at pending queue drain MUST run normally; expires_at evaluation MUST run normally; staleness indicators MUST be suppressed for suspended sessions; clock-skew estimation window MUST be frozen (no new samples). On safe mode exit (SessionResumed): the estimation window MUST be reset to empty; staleness timers MUST be reset to 0.
Source: RFC 0003 §5.6.2
Scope: v1-mandatory

#### Scenario: Safe mode expiry continues
- **WHEN** safe mode is active and a tile's expires_at_wall_us is reached
- **THEN** the tile MUST be expired on schedule; safe mode does not pause expiry

#### Scenario: Safe mode staleness suppressed
- **WHEN** safe mode is active and a suspended session's tile has received no mutations
- **THEN** the staleness indicator MUST NOT be shown; the tile is in a known-suspended state

#### Scenario: Estimation window reset on resume
- **WHEN** safe mode exits and sessions receive SessionResumed
- **THEN** each session's clock-skew estimation window MUST be reset to empty; stale samples MUST NOT bias post-resumption corrections

### Requirement: Headless Virtual Clock
In headless mode, the compositor MUST run at a configurable frame rate (default 60fps) driven by a synthetic clock (tokio::time::interval or SimulatedClock). The same frame pipeline and timing semantics MUST apply; only the vsync source differs.
Source: RFC 0003 §5.6.3, RFC 0002 §8.4
Scope: v1-mandatory

#### Scenario: Headless timing equivalence
- **WHEN** the compositor runs in headless mode
- **THEN** all timing behavior (present_at evaluation, expiry, sync groups) MUST be identical to windowed mode

### Requirement: Message Class Typed Enum
The four message classes (Transactional, StateStream, EphemeralRealtime, ClockedMediaCue) MUST be represented as a typed MessageClass enum in the proto schema, not a string field. Each class has distinct delivery semantics: Transactional (reliable, ordered, acked, never coalesced), StateStream (reliable, ordered, coalesced per coalesce_key), EphemeralRealtime (low-latency, droppable, latest-wins), ClockedMediaCue (scheduled against media/display clock, post-v1).
Source: RFC 0003 §7.1
Scope: v1-mandatory

#### Scenario: Message class enforcement
- **WHEN** an agent submits a payload with message_class = EPHEMERAL_REALTIME and delivery_policy = DROP_IF_LATE
- **THEN** the compositor MUST accept it

#### Scenario: Invalid delivery policy
- **WHEN** an agent submits delivery_policy = DROP_IF_LATE with message_class = TRANSACTIONAL
- **THEN** the compositor MUST reject with INVALID_DELIVERY_POLICY

### Requirement: ClockSync RPC
The compositor MUST provide a ClockSync unary RPC for agents to align their clocks. The request carries agent_timestamp_wall_us; the response provides compositor_mono_us, compositor_wall_us, estimated_skew_us (signed int64), skew_within_tolerance, and optional warning. Agents SHOULD call ClockSync at session start and after receiving CLOCK_SKEW_HIGH events.
Source: RFC 0003 §7.1, §4.5
Scope: v1-mandatory

#### Scenario: ClockSync at session start
- **WHEN** an agent calls ClockSync immediately after session establishment
- **THEN** the compositor MUST return its current monotonic and wall clock values and the estimated skew

### Requirement: Timing Configuration
All timing parameters MUST be configurable via TimingConfig with documented defaults and validation ranges: target_fps (default 60, range 1-240), max_agent_clock_drift_ms (default 100, range 1-10000), max_vsync_jitter_ms (default 2, range 0-100), max_future_schedule_us (default 300000000, range 1000000-3600000000), sync_group_max_defer_frames (default 3, range 1-60), pending_queue_depth_per_agent (default 256, range 16-4096), sync_drift_budget_us (default 500, range 1-100000), tile_stale_threshold_ms (default 5000, range 500-300000), clock_jump_detection_ms (default 50, range 10-10000).
Source: RFC 0003 §7.1, §10
Scope: v1-mandatory

#### Scenario: Configuration validation
- **WHEN** a config sets target_fps = 0
- **THEN** the runtime MUST reject the configuration (outside valid range 1-240)

#### Scenario: Default behavior
- **WHEN** no [timing] section is provided in config
- **THEN** the runtime MUST use all documented defaults (target_fps=60, sync_drift_budget_us=500, etc.)

### Requirement: Media Clock Integration (Deferred)
Media clock integration (GStreamer pipeline clock, AV synchronization, PTS-to-frame mapping, word-highlighting/subtitle timing, sync groups with media_clock_binding) is explicitly deferred to post-v1. V1 MUST ship sync groups for scene-level coordination only.
Source: RFC 0003 §6
Scope: post-v1

#### Scenario: No media clock in v1
- **WHEN** the v1 runtime starts
- **THEN** no GStreamer pipeline clock SHALL be created; ClockedMediaCue message class SHALL be defined in the proto but not actively processed

### Requirement: Clocked Media/Cue Message Class (Deferred)
The ClockedMediaCue message class (scheduled against media/display clock for AV sync, subtitles, word-highlighting) is defined in the proto enum but MUST NOT be actively processed in v1.
Source: RFC 0003 §6, §7.1
Scope: post-v1

#### Scenario: ClockedMediaCue not processed
- **WHEN** an agent sends a payload with message_class = CLOCKED_MEDIA_CUE in v1
- **THEN** the runtime MAY accept or reject it; it MUST NOT attempt media-clock scheduling
