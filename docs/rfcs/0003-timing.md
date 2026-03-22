# RFC 0003: Timing Model

**Status:** Draft
**Issue:** rig-5vq.3
**Date:** 2026-03-22
**Authors:** tze_hud architecture team

---

## Review History

### Round 1 — Doctrinal Alignment Deep-Dive (rig-5vq.19)

**Reviewer:** Beads worker agent
**Date:** 2026-03-22
**Doctrine files reviewed:** architecture.md, presence.md, validation.md, v1.md, failure.md

#### Doctrinal Alignment: 4/5

The RFC correctly implements the core invariant ("arrival time is not presentation time"), all four message classes, sync group semantics, and the injectable clock architecture. Quantitative requirements are traceable to doctrine. The following doctrinal gaps were found and fixed in this round:

- **[MUST-FIX → FIXED]** `architecture.md §Time is a first-class API concept` mandates `sequence`, `priority`, and `coalesce_key` as required timing fields on realtime payloads. These were absent from `TimestampedPayload`. Added in §7.1 and documented in §3.2.
- **[MUST-FIX → FIXED]** `message_class` was a `string` field instead of a typed enum, losing proto-level type safety. Replaced with `MessageClass` enum.
- **[MUST-FIX → FIXED]** `validation.md §Other performance budgets` specifies "sync drift < 500μs". The RFC had no mention of this budget. Added `sync_drift_budget_us` to §4.2, `TimingConfig`, and `FrameTimingRecord`.
- **[MUST-FIX → FIXED]** `failure.md §Agent is slow` requires staleness indicators when data is stale beyond a threshold. The RFC had no staleness indicator specification. Added §4.7 "Staleness Indicators".
- **[SHOULD-FIX → FIXED]** `presence.md §Tabs and tiles` lists "latency class" as a tile property with timing significance. The RFC did not explain the relationship between `latency_class` and sync group membership. Added §2.1 clarification.
- **[CONSIDER]** `architecture.md §Time` mentions `effective_after` as a payload field. The RFC does not implement `effective_after` explicitly (it maps to `present_at`). This is acceptable for v1 but should be noted as a future field.

#### Technical Robustness: 4/5

The clock domain hierarchy is sound. The frame deadline model is complete and backed by RFC 0002 stage budgets. The following technical issues were found and fixed:

- **[MUST-FIX → FIXED]** `uint64` drift correction arithmetic can underflow when skew is negative (agent clock behind compositor). `corrected = agent_ts - skew` is unsigned subtraction. Fixed §4.4 with correct signed arithmetic and overflow guard.
- **[SHOULD-FIX → FIXED]** `SyncGroupId` was defined as a new wrapper `message { bytes id = 1; }` inconsistent with RFC 0001's `SceneId` pattern. Aligned to use `bytes` directly with a comment pointing to `scene.proto SceneId`.
- **[CONSIDER]** `AllOrDefer` with growing member sets (Open Question 3) is unresolved. The recommendation in §9 is sound — document and defer to implementation.

#### Cross-RFC Consistency: 3/5

- **[MUST-FIX → NOTED, pending RFC 0001 amendment]** RFC 0001 uses `_ms` timestamp units throughout; RFC 0003 establishes `_us` as authoritative. §7.2 documents the migration requirement. A formal RFC 0001 amendment is tracked separately.
- **[SHOULD-FIX → NOTED]** RFC 0002 `TelemetryRecord` has `stage_durations_us: [u64; 8]` as a fixed array. RFC 0003 `FrameTimingRecord` has named per-stage fields. These serve the same role; the named-field version in RFC 0003 is strictly more informative but they need to converge. RFC 0002 should adopt RFC 0003's named fields in a follow-on amendment.
- **[SHOULD-FIX → FIXED]** `SyncGroupId` type was incompatible between RFC 0001 (bare `SceneId`) and RFC 0003 (wrapper message). Resolved by aligning to `bytes` with `scene.proto SceneId` reference.

**No dimension below 3. Round 1 findings addressed. Ready for Round 2 (Technical Architecture Scrutiny).**

### Round 2 — Technical Architecture Scrutiny (rig-5vq.20)

**Reviewer:** Beads worker agent
**Date:** 2026-03-22
**Doctrine files reviewed:** architecture.md, validation.md, v1.md

#### Doctrinal Alignment: 4/5

No new doctrinal gaps. Round 1 fixes held. Score unchanged.

#### Technical Robustness: 4/5

- **[MUST-FIX → FIXED]** `ClockSync` RPC had messages defined in `timing.proto` but no `service` block and no home in any proto service. Added `ClockSyncService` to `timing.proto` §7.1 with a cross-reference note for RFC 0005.
- **[MUST-FIX → FIXED]** Pending queue flush behavior on session close was unspecified. Added normative language to §5.3.
- **[SHOULD-FIX → FIXED]** Expiry non-negotiability under load was stated only in Open Questions (recommendation), not as a normative requirement in §5.4. Promoted to normative text.
- **[SHOULD-FIX → FIXED]** Clock skew estimation window had no fast-convergence path after a sudden clock jump. Added jump-detection reset condition to §4.3.
- **[SHOULD-FIX → FIXED]** Explicit `DeleteTile` mid-deferral behavior was unspecified in §2.3. Added parity with expiry behavior.
- **[CONSIDER]** Frame quantization boundary condition noted in §3.3; no change required.

#### Cross-RFC Consistency: 4/5 (up from 3/5)

- **[MUST-FIX → FIXED]** `ClockSync` RPC was referenced in §4.5/§7.2 but absent from RFC 0005 session service. Added `ClockSyncService` to `timing.proto` and added cross-reference note in §7.2 clarifying service home.
- **[MUST-FIX → FIXED in RFC 0005]** `TimingHints.sync_group_id` was `string` in RFC 0005's inline definition but `bytes` in RFC 0003. Fixed the RFC 0005 inline `TimingHints` to `bytes`.
- **[SHOULD-FIX → FIXED in RFC 0005]** `session_open_at_us` clock reference not exposed in `SessionEstablished`; agents had no way to align their clocks at handshake without an extra RPC. Added `compositor_wallclock_us` and `estimated_skew_us` to `SessionEstablished`.
- **[CONSIDER]** `ZonePublish.ttl_ms` unit inconsistency deferred to RFC 0001 `_ms → _us` amendment sweep.

**No dimension below 3. Round 2 findings addressed. Ready for Round 3.**

---

## Summary

This RFC defines the Timing Model for tze_hud — the authoritative specification for how time flows through the compositor, how it is expressed in the API, and how the system behaves when timing constraints are violated. It covers clock domains, sync groups, timestamp semantics, drift rules, deadline behavior, and the protobuf schema for timing-related messages.

The Timing Model is a foundational contract. Every other subsystem that touches scheduling — scene mutations, frame delivery, expiry, sync group coordination — depends on the definitions here.

---

## Motivation

The core architectural rule is: **arrival time is not presentation time**. This is not a nuance — it is a load-bearing invariant. If the compositor treats arrival time as presentation time:

- A subtitle "highlight this word" arrives at an arbitrary network delay and is shown too early or too late relative to the audio.
- An agent's batch mutation arrives mid-frame and is applied in two halves, causing visible tearing.
- A sync group with two agents commits one member's update in frame N and the other in frame N+1, producing a visible flash of incoherent state.
- A tile with `expires_at` is shown for an undefined duration depending on network jitter.

Without a precise timing contract:

- Agents have no stable way to schedule content.
- The compositor cannot enforce frame-aligned commit semantics.
- Sync groups cannot guarantee atomicity.
- Expiry behavior is non-deterministic.

This RFC resolves all of these by specifying timing as a first-class concern with defined semantics at every layer.

---

## Design Requirements Satisfied

| Requirement | This RFC |
|-------------|----------|
| DR-V3: Structured telemetry | Timing fields defined in `TelemetryRecord`; sync drift is a tracked metric. |
| DR-V4: Deterministic test scenes | Injectable clock source for all timing paths; no wall-clock surprises in tests. |

---

## 1. Clock Domains

### 1.1 Overview

tze_hud recognizes four distinct clock domains. They are not interchangeable.

| Domain | Authority | Resolution | Primary Use |
|--------|-----------|------------|-------------|
| **Display clock** | vsync interrupt / presentation callback | ~16.6ms (frame) | Composition timing, frame numbering |
| **Monotonic system clock** | OS monotonic clock (`CLOCK_MONOTONIC` / `Instant`) | microseconds (μs) | Internal measurements, deadline tracking, telemetry |
| **Network clock** | Agent-supplied timestamps (UTC wall clock) | microseconds (μs) | `present_at`, `expires_at`, `created_at` in API |
| **Media clock** | GStreamer pipeline clock | nanoseconds (ns) | AV sync, subtitle/cue alignment (post-v1) |

**Primary clock for composition:** The display clock is master for all presentation timing decisions. The compositor does not attempt to render a frame until a vsync signal (or simulated vsync in headless mode) is received. Scene commits are quantized to frame boundaries. No mutation is ever applied "between frames."

```
Clock Domain Hierarchy

  ┌─────────────────────────────────────────────────────────────────┐
  │                     Display Clock (master)                       │
  │              vsync @ 60Hz → frame N, N+1, N+2 ...               │
  │                        resolution: ~16.6ms                       │
  └──────────────────────────┬──────────────────────────────────────┘
                             │ drives
             ┌───────────────┼───────────────┐
             ▼               ▼               ▼
  ┌──────────────┐  ┌──────────────┐  ┌─────────────────────────┐
  │  Monotonic   │  │  Network     │  │  Media Clock (post-v1)  │
  │  System      │  │  Clock       │  │  GStreamer pipeline      │
  │  Clock       │  │  (UTC, μs)   │  │  resolution: ns         │
  │  resolution: │  │  Agent-      │  │  Synchronized to        │
  │  μs          │  │  supplied    │  │  display clock via      │
  │  Internal    │  │  present_at, │  │  PTS/offset mapping     │
  │  deadlines,  │  │  expires_at  │  └─────────────────────────┘
  │  telemetry   │  └──────────────┘
  └──────────────┘
         ▲
         │ reference for
  ┌──────────────┐
  │  Frame       │
  │  Deadline    │
  │  Tracking    │
  └──────────────┘
```

### 1.2 Clock Domain Responsibilities

**Display clock.** Generated by vsync interrupts or the presentation callback (platform-specific). In headless mode, a synthetic clock fires at the configured frame rate (default 60Hz). The compositor threads use the display clock to: determine which frame is current, compute which pending mutations fall before the commit deadline, and evaluate `present_at` conditions.

The display clock does not run in UTC. It is an ordinal sequence of frame numbers plus a monotonic offset from an arbitrary epoch (runtime startup). Conversion to wall time uses the monotonic clock at the time of the most recent vsync.

**Monotonic system clock.** Used for all internal duration measurements: stage latencies, deadline calculations, agent heartbeat tracking, lease TTL expiry, and telemetry timestamps. Never exposed directly to agents. Never jumps backward.

**Network clock.** All timestamps in the public API (agent-supplied `present_at`, `expires_at`, `created_at`) are expressed in this domain: UTC microseconds since Unix epoch. The compositor maintains a clock-skew offset between the monotonic system clock and the network clock, computed from observed timestamps during agent sessions. This offset is used to interpret agent-supplied timestamps in terms of the local monotonic timeline.

**Media clock (post-v1).** GStreamer pipeline clock, aligned to media presentation timestamps (PTS). Post-v1 only. The compositor maps between media PTS and display frame numbers to schedule cue-type content (subtitles, word highlights, beat markers) against the current playback position. The mapping is maintained as a running linear regression between pipeline clock and display frame sequence. See §6 for the full media timing specification.

### 1.3 Synchronization Points

The three active clock domains (display, monotonic, network) have two synchronization points:

**vsync sync point (per-frame).** At the start of each frame, the compositor records: `frame_number`, `vsync_monotonic_us` (monotonic clock value at vsync), and `vsync_wallclock_us` (UTC wall clock value at vsync, sampled once and cached). This triple is the canonical sync point for that frame and is included in the `TelemetryRecord`.

**Agent session sync point (per-handshake).** During agent session establishment (gRPC handshake), the compositor records the `session_open_monotonic_us` and `session_open_wallclock_us`. The difference is the session's initial clock-skew estimate. Subsequent agent-supplied timestamps are validated against this estimate.

---

## 2. Sync Groups

### 2.1 Purpose

A sync group is a set of tiles whose mutations must be applied atomically in the same frame. No member of a sync group can show an updated state while another member still shows its previous state — the compositor either applies all pending mutations for all group members in a single frame commit, or defers all of them.

This is the mechanism by which multi-tile layouts, coordinated overlays, and multi-agent presentations remain visually coherent.

**Relationship to tile latency class (doctrine: presence.md §Tabs and tiles):** Tiles carry a `latency_class` property that governs how aggressively their mutations are prioritized during Stage 3 (Mutation Intake). A tile's `latency_class` and its `sync_group` membership are orthogonal:

- `latency_class` determines how quickly a tile's mutations are admitted under load (admission priority).
- `sync_group` determines when the admitted mutations are applied to the scene (commit atomicity).

A tile with `latency_class = Interactive` in a sync group gets prioritized admission but still waits for the sync group's commit policy before its mutation appears on screen. The RFC 0001 Scene Contract defines the `latency_class` values; the Timing RFC governs how they interact with sync group commit rules.

### 2.2 Sync Group Identity

A `SyncGroupId` is a `SceneId` (UUIDv7). Sync groups are named objects in the scene graph, not just tags on tiles. They have an explicit lifecycle: create, join, leave, and destroy.

```rust
pub struct SyncGroup {
    pub id: SceneId,                    // SyncGroupId
    pub name: Option<String>,           // Human-readable label; max 128 UTF-8 bytes
    pub owner_namespace: String,        // Namespace that created this group
    pub members: BTreeSet<SceneId>,     // TileIds currently in the group
    pub created_at_us: u64,             // UTC microseconds
    pub commit_policy: SyncCommitPolicy,
}

pub enum SyncCommitPolicy {
    /// All members must have a pending mutation before any can be applied.
    /// If a frame closes before all members have pending mutations, the group
    /// defers. Suitable for tightly coordinated layouts.
    AllOrDefer,

    /// Apply whatever subset of members have pending mutations this frame.
    /// Members without pending mutations are implicitly "unchanged" — they
    /// do not block the group. Suitable for loose coordination.
    AvailableMembers,
}
```

### 2.3 Semantics

**Creating a sync group.** Any agent with the `SYNC_GROUP_CREATE` capability grant can create a sync group. The creator becomes the `owner_namespace`. The group starts with zero members.

**Joining a sync group.** A tile joins via `UpdateTileSyncGroup` mutation. A tile may belong to at most one sync group at a time. Joining is immediate; the tile's next pending mutation is subject to sync group commit rules.

**Leaving a sync group.** A tile leaves via `UpdateTileSyncGroup { sync_group: None }`. After leaving, the tile's mutations are no longer subject to group commit constraints. Leaving is applied at the next scene commit.

**Destroying a sync group.** A sync group is destroyed when explicitly deleted or when its last member leaves. Destruction removes the group from the scene graph. Any tile that still holds a reference to the group ID is automatically released from it.

**Tile deletion mid-deferral.** An explicit `DeleteTile` mutation causes the tile to leave its sync group before deletion, identical to expiry behavior (see §5.4). The sync group's commit policy evaluates against the updated member set after the deletion. If the deleted tile was the only missing member in an `AllOrDefer` group, removing it unblocks the group: the remaining members' pending mutations may now commit in the same frame as the deletion.

**Cross-agent sync groups.** Multiple agents can place tiles into the same sync group. The group does not belong to any single agent's mutation batch — it is a scene-graph object. When Agent A and Agent B both have tiles in the same sync group, their mutations are held in a pending queue until the commit policy's condition is satisfied. The compositor evaluates this at Stage 4 (Scene Commit) of the frame pipeline.

### 2.4 Timing Contract

**Atomicity window:** one display frame (16.6ms at 60Hz). All pending mutations for all members of a sync group are applied in the same stage-4 execution. The compositor does not split sync group commits across frames.

**Deadline:** a sync group's pending mutations must arrive before the frame's mutation intake cutoff (end of Stage 3, see §5). Mutations arriving after the cutoff for a frame are held for the next frame.

**AllOrDefer policy:** if the policy is `AllOrDefer` and at least one member has no pending mutation at the time of Stage 4, the entire group is deferred to the next frame. This can cascade: if the group is still incomplete at the next frame, it defers again. Maximum deferral: configurable, default 3 frames (50ms at 60fps). If the group is still incomplete after max deferral, the available members' mutations are force-applied and the group transitions to `AvailableMembers` policy for that commit cycle only. A `sync_group_force_commit` event is emitted to telemetry.

**AvailableMembers policy:** mutations from members with pending work are applied. Members without pending work remain unchanged. No deferral.

```
Sync Group Commit Flow (AllOrDefer, cross-agent)

  Agent A ──────────────────────────────────────────────────────►
  (tile T1 in group G)
     │ mutation M_A arrives in frame N's intake window
     │ T1 pending: YES

  Agent B ──────────────────────────────────────────────────────►
  (tile T2 in group G)
     │ mutation M_B arrives in frame N's intake window
     │ T2 pending: YES

  Frame N pipeline:
    Stage 3 (Mutation Intake):
      T1 mutation queued for group G
      T2 mutation queued for group G
      Group G: [T1: PENDING, T2: PENDING] → all members ready

    Stage 4 (Scene Commit):
      Apply M_A to T1 ──┐
      Apply M_B to T2 ──┴──► atomic: both visible in frame N

  ─────────────────────────────────────────────────────────────
  Failure case: only Agent A's mutation arrives in frame N

  Frame N pipeline:
    Stage 3: T1 pending: YES, T2 pending: NO
    Stage 4: AllOrDefer → defer entire group to frame N+1

  Frame N+1 pipeline (if M_B arrives):
    Stage 3: T1 pending: YES (carried over), T2 pending: YES
    Stage 4: Apply M_A and M_B atomically → both visible in frame N+1

  If frame N+2 is also incomplete → defer again (max 3 frames)
  If frame N+3 is incomplete → force commit with warning in telemetry
```

### 2.5 Resource Governance

Sync groups are not a resource leak vector. The compositor enforces:

- Maximum sync groups per agent namespace: 16.
- Maximum tiles per sync group: 64 (same as the `max_tiles_per_tab` budget from RFC 0001).
- An agent cannot place another agent's tiles into a sync group (ownership check; the joining tile must be in the mutating agent's namespace).

---

## 3. Timestamp Semantics

### 3.1 The Invariant

**Arrival time is not presentation time.** Every payload in the API that has timing significance carries explicit timestamp fields. The compositor never infers "show this now" from "this just arrived."

```
Correct mental model:

  Agent                 Network               Compositor
    │                      │                      │
    │── MutationBatch ─────►│                      │
    │   present_at=T+500ms  │── MutationBatch ─────►│
    │                      │                      │
    │                      │             Store mutation;
    │                      │             do not apply yet
    │                      │                      │
    │                      │         ... frames pass ...
    │                      │                      │
    │                      │             Frame at T+500ms:
    │                      │             apply mutation now
    │                      │             Present!

Incorrect model (never do this):

  Agent                 Compositor
    │── mutation ─────────► apply immediately → tearing, incoherence
```

### 3.2 Timestamp and Ordering Fields

All timestamps in the public API use **UTC microseconds since Unix epoch** as a `uint64`. This is the network clock domain. Zero (`0`) always means "not set" — the runtime started after 2025-01-01T00:00:00Z, so zero is never a valid timestamp.

Doctrine (architecture.md §Time is a first-class API concept): every meaningful realtime payload carries `present_at`, `effective_after`, `expires_at`, `sequence`, `priority`, `coalesce_key`, `sync_group`. All six timing fields are specified here.

| Field | Type | Scope | Semantics |
|-------|------|-------|-----------|
| `present_at_us` | `uint64` | `MutationBatch`, `Tile`, `TextNode`, `StaticImageNode` | Do not apply this mutation/render this content before this time. If zero, apply at the earliest available frame. |
| `expires_at_us` | `uint64` | `Tile`, `UpdateTileExpiry` | Remove this tile from the scene at or after this time. If zero, no automatic expiry. |
| `created_at_us` | `uint64` | `Tab`, `SyncGroup`, `SyncGroupConfig` | Wall-clock time of creation. Set by the compositor on object creation; agent-supplied values are advisory only and may be overwritten. |
| `session_open_at_us` | `uint64` | Handshake | Session establishment wall-clock time. Used for clock-skew estimation. |
| `sequence` | `uint64` | `TimestampedPayload`, `MutationBatch` | Monotonically increasing per-source ordering number. Within a `coalesce_key`, only the payload with the highest sequence is presented. Payloads with sequence ≤ last-seen for their source are dropped with a warning event. |
| `priority` | `uint32` | `TimestampedPayload` | Shedding priority under load. Higher value = more important. Zero = normal/unset (not shed first). Used by admission control in Stage 3 when the compositor is degrading. |
| `coalesce_key` | `string` | `TimestampedPayload` | For `STATE_STREAM` class: payloads sharing the same non-empty key are coalesced to the latest sequence before frame commit. Empty = no coalescing. Cross-agent coalescing never happens — keys are scoped per agent session. |
| `sync_group` | `SyncGroupId` | `Tile`, `MutationBatch` | Sync group membership for this mutation. Tiles declared in a sync group have their mutations held until the group's commit policy is satisfied (see §2). |

**Precedence.** `present_at_us` can be set at both the batch level and the individual tile/node level:

1. Node-level `present_at_us` (if set) overrides tile-level.
2. Tile-level `present_at_us` (if set) overrides batch-level.
3. Batch-level `present_at_us` (if set) is the default for all tiles and nodes in the batch that do not set their own.

### 3.3 Timestamp Resolution

Timestamps are stored and compared at **microsecond (μs) resolution**. The display clock has millisecond-level granularity (16.6ms per frame at 60fps). Microsecond resolution in timestamps is used to preserve agent intent precisely, not because the compositor can act on sub-millisecond differences. In practice, timestamps are quantized to the nearest frame boundary during evaluation.

**Frame quantization:** a `present_at_us` timestamp T is "in scope" for frame F if:
```
T <= frame_F_vsync_us + frame_budget_us / 2
```
Where `frame_budget_us` is 16,667μs at 60fps. Timestamps within half a frame period after the vsync are treated as belonging to that frame. This prevents off-by-one frame errors when timestamps and vsync are very close.

### 3.4 Timezone Handling

All timestamps are UTC internally. The compositor never stores or computes local time. Timestamps in telemetry output are UTC ISO-8601 strings. Agent-supplied timestamps must be UTC. The protocol rejects any timestamp that could only be valid in a local timezone offset (a heuristic check — the runtime does not formally validate timezone semantics, but timestamps dramatically outside the expected range trigger clock-skew warnings).

### 3.5 Timestamp Validation

The compositor applies these validation rules to all agent-supplied timestamps:

| Condition | Action |
|-----------|--------|
| `present_at_us < session_open_at_us - 60_000_000` (> 60s in the past) | Reject: mutation too stale. Structured error `TIMESTAMP_TOO_OLD`. |
| `present_at_us > current_wallclock_us + max_future_schedule_us` | Reject: timestamp too far in future. Structured error `TIMESTAMP_TOO_FUTURE`. Default `max_future_schedule_us`: 300_000_000 (5 minutes). |
| `expires_at_us <= present_at_us` (expiry before or at presentation) | Reject: inconsistent timestamps. Structured error `TIMESTAMP_EXPIRY_BEFORE_PRESENT`. |
| `delivery_policy == DELIVERY_POLICY_DROP_IF_LATE` and `message_class != MESSAGE_CLASS_EPHEMERAL_REALTIME` | Reject: `DROP_IF_LATE` is only valid for the `EPHEMERAL_REALTIME` message class. Structured error `INVALID_DELIVERY_POLICY`. |
| Clock skew > 100ms (see §4.2) | Warning in telemetry; apply timestamps with skew correction. |
| Clock skew > 1s | Reject with structured error `CLOCK_SKEW_EXCESSIVE`. |

These limits are configurable per deployment. The defaults above apply to typical LAN/localhost agent deployments.

---

## 4. Drift Rules

### 4.1 Drift Concept

Clock drift is the divergence between the agent's clock (source of `present_at`, `expires_at`) and the compositor's monotonic clock (used to evaluate those timestamps). Some drift is expected. The system is designed to tolerate and correct moderate drift. Excessive drift is rejected.

### 4.2 Maximum Allowed Drift

Two distinct drift concepts apply. Do not conflate them:

- **Agent clock drift** — divergence between an agent's UTC wall clock and the compositor's clock. Tolerates network/NTP imprecision. Measured in milliseconds.
- **Sync group presentation skew** — the spread between mutation arrival times across members of a sync group in the same frame. Measured in microseconds. Doctrine (validation.md §Other performance budgets): `sync drift < 500μs`. This is the *telemetry budget*, not a hard rejection threshold — it tracks whether sync group members are arriving with low latency relative to each other.

| Clock pair | Default tolerance | Enforcement | Configuration key |
|-----------|------------------|-------------|-------------------|
| Agent network clock vs. compositor monotonic | **100ms** warning; **1s** rejection | Structured error `CLOCK_SKEW_EXCESSIVE` | `timing.max_agent_clock_drift_ms` |
| Sync group member arrival spread (telemetry budget) | **500μs** | Telemetry alert `sync_drift_high`; stale indicator if exceeded at presentation | `timing.sync_drift_budget_us` |
| Display clock drift from nominal frame rate (jitter) | **2ms** | Telemetry warning | `timing.max_vsync_jitter_ms` |
| (post-v1) Media clock vs. display clock | 10ms | Telemetry warning | `timing.max_media_drift_ms` |

**Sync drift tracking:** `sync_group_max_drift_us` in `FrameTimingRecord` (see §7.1) records the worst mutation-arrival spread across sync group members this frame. If this value exceeds `timing.sync_drift_budget_us` (default 500μs), a `sync_drift_high` telemetry event is emitted and the chrome layer staleness indicator is activated for the affected sync group's tiles.

### 4.3 Drift Detection

**Estimation window.** The compositor maintains a sliding window of the last 32 agent timestamps, recording `(agent_timestamp_us, compositor_monotonic_us)` pairs at the time each mutation batch arrives. The clock-skew estimate is the median of `(agent_ts - compositor_ts)` over the window. Median is used (not mean) to suppress outlier spikes from individual delayed messages.

**Update frequency.** The estimate is updated on every mutation batch arrival. The estimation window is bounded; the oldest sample is evicted when the window fills.

**Clock jump detection.** If consecutive samples show a skew change greater than **`timing.clock_jump_detection_ms`** (default: 50ms) between them — indicating a sudden NTP step correction or agent clock adjustment, not gradual drift — the compositor resets the estimation window to the current single sample rather than continuing to accumulate. The estimate is re-initialized from this sample as a single-point estimate until the window refills. This prevents systematic miscorrection during the convergence period after a clock step, where a 32-sample median weighted by stale values would apply the wrong correction for up to 32 subsequent mutations.

**Vsync jitter tracking.** Each frame's vsync arrival time is recorded. Jitter is computed as the standard deviation of inter-frame intervals over the last 120 frames (2 seconds at 60fps). High jitter triggers a telemetry warning and may indicate GPU or OS scheduling issues.

### 4.4 Drift Correction

When drift is within tolerance (≤ 100ms), the compositor applies an offset correction to the agent's timestamps before evaluation. The skew estimate (`int64`, signed) may be positive (agent clock ahead) or negative (agent clock behind).

**Correct signed arithmetic (avoids `uint64` underflow):**
```
// skew_us is int64: positive = agent ahead, negative = agent behind
corrected_present_at_us: i64 = (agent_present_at_us as i64) - skew_us
```

Implementors must cast the `uint64` agent timestamp to `i64` for the correction arithmetic, then validate that the result is positive before re-casting. A negative corrected timestamp means the agent timestamp is so far in the past (after correction) that it should be treated as `present_at_us = 0` (immediate). A `TIMESTAMP_TOO_OLD` rejection (§3.5) normally catches this before drift correction is applied.

This correction is transparent to the agent. The compositor applies it silently. Agents are notified of the current skew estimate via the session telemetry stream so they can self-correct if desired.

### 4.5 Drift Exceeds Tolerance

**> 100ms (default), ≤ 1s:** The compositor continues processing but emits a `CLOCK_SKEW_HIGH` warning in the session event stream and telemetry. The agent is expected to self-correct. Timestamps are applied with skew correction.

**> 1s:** The compositor rejects new mutation batches with `CLOCK_SKEW_EXCESSIVE`. The session remains open; the agent must re-synchronize. Re-synchronization: the agent requests a `ClockSync` RPC (see §7), receives the current compositor monotonic offset, and adjusts its timestamps accordingly. After three consecutive `ClockSync` failures to bring drift within tolerance, the session is terminated with a structured error.

### 4.6 Agent Clock vs Runtime Clock Reconciliation

The compositor does not attempt to slew its own clock to match an agent's clock. The compositor's monotonic clock is sovereign. The compositor maintains a per-session skew estimate and applies it to evaluate the agent's timestamps. This is one-directional: the compositor adapts its evaluation to the agent's observed skew, but the underlying compositor clock is never adjusted by agent communication.

The `ClockSync` RPC is designed for agents to align their clocks to the compositor, not the reverse.

### 4.7 Staleness Indicators (Doctrine: failure.md §Agent is slow)

Doctrine (failure.md): "If the agent's tiles depend on fresh data and the data is stale beyond a threshold, displays a staleness indicator."

The timing model defines two staleness conditions detected from temporal signals:

**Content staleness (no new mutations):** If a tile has not received any mutation within a configurable idle threshold (default: `timing.tile_stale_threshold_ms = 5000`, i.e., 5 seconds), and the tile's `message_class` is `STATE_STREAM` or `TRANSACTIONAL` (indicating it should be updated by an active agent), the compositor activates the chrome layer's staleness indicator for that tile. The threshold does not apply to tiles in `EPHEMERAL_REALTIME` class (silence is expected between events) or tiles without a registered agent session.

**Sync group staleness (arrival spread exceeds budget):** If `sync_group_max_drift_us` for a committed sync group exceeds `timing.sync_drift_budget_us` (default 500μs), the compositor activates the staleness indicator for the slow member's tiles for that frame (see §4.2).

The staleness indicator is a chrome-layer visual badge rendered by the runtime (see architecture.md §System shell). It does not affect tile content — the tile continues to render its last committed state. It is cleared when: (a) a new valid mutation arrives for the tile, or (b) the agent disconnects and the tile enters the grace period.

---

## 5. Deadline Behavior

### 5.1 Frame Deadline Model

Each display frame has a single **mutation intake cutoff**: the end of Stage 3 (Mutation Intake) in the frame pipeline (RFC 0002 §3.2). Mutations arriving at the compositor thread after Stage 3 closes are held in the pending queue and evaluated at Stage 3 of the next frame.

```
Frame Timeline (16.6ms at 60fps)

  0μs              500μs     1ms           5ms           13ms     16,667μs
  │                  │        │              │              │         │
  ▼                  ▼        ▼              ▼              ▼         ▼
  ┌──────────────────┐ ┌──────┐ ┌───────────┐ ┌────────────┐ ┌───────┐
  │ 1. Input Drain   │ │2.Lc  │ │ 3. Mut    │ │ 4. Scene   │ │  ...  │
  │    <500μs p99    │ │Fdbk  │ │    Intake │ │    Commit  │ │       │
  │                  │ │<500μs│ │    <1ms   │ │    <1ms    │ │       │
  └──────────────────┘ └──────┘ └─────┬─────┘ └────────────┘ └───────┘
                                      │
                               MUTATION INTAKE CUTOFF
                               Mutations after this point
                               deferred to next frame

  ◄─────────────────────────── 16.6ms total frame budget ──────────────►

  Stage timing budgets (p99):
    Stage 1: Input Drain         < 500μs
    Stage 2: Local Feedback      < 500μs
    Stage 3: Mutation Intake     < 1ms    ← cutoff point
    Stage 4: Scene Commit        < 1ms
    Stage 5: Layout Resolve      < 1ms
    Stage 6: Render Encode       < 4ms
    Stage 7: GPU Submit+Present  < 8ms
    Stage 8: Telemetry Emit      < 200μs  (non-blocking, telemetry thread)

  Key deadline: mutations must arrive before end of Stage 3 to be
  included in the current frame's commit.
```

### 5.2 Late Arrival Policy

A mutation is "late" if it arrives at the compositor thread after Stage 3 has closed for the current frame. The compositor never applies a late mutation mid-frame (no partial frame updates). Late policy:

**Default (defer):** The mutation is held in the pending queue. It is evaluated at Stage 3 of the next frame. The next frame's Stage 3 sees both the late mutation from frame N and any new mutations that arrived during frame N's pipeline. They are processed in FIFO order, then sorted by `present_at_us`.

**Drop policy (ephemeral realtime class only):** Mutations marked as `MessageClass::EphemeralRealtime` may be configured to drop on late arrival rather than defer. This prevents stale ephemeral state (hover positions, interim speech tokens) from applying one frame late. Drop is the correct behavior for content where "late" equals "wrong." Default: defer for all classes; agents opt into drop via `delivery_policy` field in the mutation batch.

**Clocked media/cues (post-v1):** A cue that misses its target frame is handled by the media timing engine (§6). In v1, this class is not active.

### 5.3 Presentation Deadline (`present_at`)

A mutation batch with `present_at_us = T` is held in the pending queue until the compositor evaluates a frame F where `frame_F_vsync_us >= T`. The compositor does not apply the mutation to frames where the vsync has not yet reached T.

**Presentation accuracy:** ±1 frame (±16.6ms at 60fps). The compositor guarantees that a mutation with `present_at_us = T` will be applied no earlier than T and no later than T + 16.6ms (one frame after the target). Accuracy depends on mutation arrival before the frame's intake cutoff; a mutation arriving late may shift by one additional frame.

**Pending queue:** held pending mutations are stored in a per-agent sorted queue, ordered by `present_at_us` ascending. The queue is drained in Stage 3: mutations whose `present_at_us <= current_frame_vsync_us + half_frame` are extracted and forwarded to Stage 4.

**Maximum pending queue depth:** 256 entries per agent. Mutations that would exceed this limit are rejected with `PENDING_QUEUE_FULL`. Agents with many deferred mutations are scheduling too aggressively; they should reduce their schedule horizon or increase their mutation rate.

**Session close flush.** The pending queue is session-scoped, not agent-scoped. When an agent session closes — gracefully (via `SessionClose`) or ungracefully (timeout, TCP reset, grace period expiry) — all entries in that session's pending queue are discarded. The compositor does not apply pending mutations from a closed session. A reconnecting agent starts with an empty queue and must retransmit any desired future-scheduled mutations under the new session. This ensures that budget accounting is clean: after session close, the session's pending queue entries no longer count against any resource budget.

### 5.4 Expiration Policy (`expires_at`)

A tile with `expires_at_us = T` is automatically removed from the scene at the first frame F where `frame_F_vsync_us >= T`.

**Check mechanism:** expiry evaluation happens at Stage 4 (Scene Commit), before applying new mutations. The compositor maintains an expiry heap (min-heap ordered by `expires_at_us`) of all tiles with active expiry. At Stage 4:

1. Pop all tiles from the heap where `expires_at_us <= current_frame_vsync_us`.
2. Apply an implicit `DeleteTile` mutation for each expired tile.
3. Then apply the frame's explicit mutation batches.

**Complexity:** O(expired_items) per frame, not O(total_items). The heap ensures only expired tiles are visited. Total tile count does not affect expiry evaluation cost.

**Expiry is non-negotiable under load.** Expiry evaluation at Stage 4 is never deferred by degradation. The compositor must evaluate the expiry heap and apply implicit `DeleteTile` mutations even during degradation level 4 (ShedTiles) or 5 (Emergency). Expiry semantics are a correctness contract, not a performance optimization. An agent that sets `expires_at_us` must be able to rely on tile removal within one frame (≤ 16.6ms at 60fps) of the expiry time, regardless of compositor load. RFC 0002's degradation machinery must not shed Stage 4 expiry work.

**Expiry notification:** when a tile expires, the compositor emits a `TileExpired { tile_id, expires_at_us, frame_number }` event to the owning agent's event subscription stream.

**Expiry and sync groups:** if a tile that is a member of a sync group expires, the tile is removed from the sync group first, then deleted. The sync group's commit policy evaluates against the updated member set.

### 5.5 Interaction Between `present_at` and `expires_at`

The compositor validates at mutation commit time that `expires_at_us > present_at_us` (see §3.5). If a tile's `present_at_us` is in the future and its `expires_at_us` is also in the future but before `present_at_us`, the mutation is rejected.

A tile whose `present_at_us` has not yet been reached is not rendered, but it is in the scene graph and consumes resource budget. Agents scheduling large numbers of future mutations should be aware that each pending tile counts against their resource budget from the moment it is committed, not from the moment it becomes visible.

---

## 6. Media Timing (Post-v1)

This section specifies the media timing model for reference. All content in this section is **explicitly deferred to post-v1**. v1 ships sync groups for scene-level coordination only; media clock integration is not part of the v1 delivery.

See `heart-and-soul/v1.md` §"V1 explicitly defers" for the authoritative v1 boundary.

### 6.1 GStreamer Pipeline Clock

When GStreamer media pipelines are active (post-v1), the compositor integrates a fourth clock domain: the GStreamer pipeline clock. The pipeline clock is a monotonically increasing nanosecond counter maintained by GStreamer, synchronized to the media source's presentation timestamps (PTS).

The media clock is not the master clock. The display clock remains master. The media clock is mapped to the display clock via a linear PTS-to-frame-number mapping maintained by the compositor's media integration layer.

**Mapping update:** On each vsync, the compositor queries the active pipeline's current PTS. The mapping is: `frame_number = (pts_ns - pts_offset_ns) / frame_duration_ns`. The mapping is updated as a Kalman filter to smooth jitter in PTS delivery.

### 6.2 AV Synchronization

Audio-video synchronization follows GStreamer's built-in clock synchronization. The compositor's role is to ensure that video frames (GPU textures) are submitted for presentation at the correct display frame, aligned to the pipeline's clock. The compositor does not modify audio timing — GStreamer owns audio output.

**Video frame submission:** each decoded video frame has a PTS. The compositor maps the PTS to a target display frame number and schedules the texture upload for that frame's Stage 6 (Render Encode). Video frames arriving late (after their target frame's Stage 6) are dropped; the previous frame is held (freeze) rather than showing a gap. Consecutive drops trigger a `video_decode_stall` event.

### 6.3 Word-Highlighting and Subtitle Timing

Subtitle and word-highlight cues are a subclass of the `ClockedMediaCue` message class. Each cue carries:
- `start_pts_us`: PTS at which the cue becomes active.
- `end_pts_us`: PTS at which the cue expires.
- `payload`: the cue content (text range, color, opacity, etc.).

The compositor maps `start_pts_us` and `end_pts_us` to display frame numbers using the media clock mapping. The cue's `present_at_us` (display clock domain) is computed automatically; agents publishing cued content do not need to compute display timestamps.

### 6.4 Sync Groups with Media Clock

Post-v1 sync groups may declare a `media_clock_binding`: a reference to an active GStreamer pipeline. Tiles in such a group are updated with cue-class timing rather than mutation-class timing. The sync group's atomicity guarantee extends across both the media plane (frame decode) and the scene plane (content updates).

---

## 7. Protobuf Schema

### 7.1 `timing.proto`

```protobuf
syntax = "proto3";
package tze_hud.timing.v1;

// ─── Timestamp ───────────────────────────────────────────────────────────────

/// A UTC microsecond timestamp. Zero means "not set."
/// The compositor validates that real timestamps are > 0 and within range.
message MicrosecondTimestamp {
  uint64 us = 1; // UTC microseconds since Unix epoch
}

// ─── Timestamped Payload ─────────────────────────────────────────────────────

/// Wraps any bytes payload with timing metadata.
/// Used for generic payload scheduling on the gRPC session stream.
/// Message class enum for typed discrimination.
/// Doctrine: architecture.md §Message classes — four traffic classes with different delivery semantics.
enum MessageClass {
  MESSAGE_CLASS_UNSPECIFIED      = 0;
  MESSAGE_CLASS_TRANSACTIONAL    = 1; // Reliable, ordered, acked. Never coalesced.
  MESSAGE_CLASS_STATE_STREAM     = 2; // Reliable, ordered, coalesced. Latest-wins per coalesce_key.
  MESSAGE_CLASS_EPHEMERAL_REALTIME = 3; // Low-latency, droppable, latest-wins per source.
  MESSAGE_CLASS_CLOCKED_MEDIA_CUE  = 4; // Scheduled against media/display clock. Post-v1 active.
}

/// Delivery policy for ephemeral_realtime class.
enum DeliveryPolicy {
  DELIVERY_POLICY_UNSPECIFIED = 0;
  DELIVERY_POLICY_DEFER       = 1; // Default: hold for next frame if late.
  DELIVERY_POLICY_DROP_IF_LATE = 2; // Ephemeral only: discard if late (stale = wrong).
}

message TimestampedPayload {
  bytes         payload          = 1; // Serialized inner message
  uint64        present_at_us    = 2; // 0 = immediate
  uint64        expires_at_us    = 3; // 0 = no expiry
  uint64        created_at_us    = 4; // Agent-assigned creation time; advisory only
  MessageClass  message_class    = 5; // Traffic class; governs delivery semantics
  DeliveryPolicy delivery_policy = 6; // Default DEFER; DROP_IF_LATE valid only for EPHEMERAL_REALTIME
  uint64        sequence         = 7; // Monotonic per-source ordering. Doctrine: architecture.md §Time is a first-class API concept.
  uint32        priority         = 8; // For shedding under load (higher = more important; 0 = unset/normal).
  string        coalesce_key     = 9; // For state-stream dedup. Empty = no coalescing. Doctrine: architecture.md §Time.
}

// ─── Sync Group ──────────────────────────────────────────────────────────────

// Cross-RFC consistency (RFC 0001): SyncGroupId is a SceneId (UUIDv7, 16 bytes),
// consistent with RFC 0001 §1.1. It is NOT a separate wrapper type. Both the Rust
// type alias `type SyncGroupId = SceneId` and the protobuf representation use the
// same 16-byte UUIDv7 encoding defined in RFC 0001. A `SyncGroupId` of all-zero
// bytes means "not set / not in a sync group" (consistent with RFC 0001 §10.1).
//
// Implementors: use the `SceneId` message type from scene.proto, not a local alias.
// The scene.proto SceneId message is:
//   message SceneId { bytes id = 1; }  // 16-byte UUIDv7

/// Configuration for a sync group.
message SyncGroupConfig {
  bytes  id              = 1; // SyncGroupId: 16-byte UUIDv7 (from scene.proto SceneId)
  string name            = 2; // Optional; max 128 UTF-8 bytes
  SyncCommitPolicy commit_policy = 3;
  uint32 max_defer_frames = 4; // Default 3; 0 = use default
  uint64 created_at_us   = 5; // Agent-supplied creation time (UTC μs); advisory — compositor may overwrite
}

enum SyncCommitPolicy {
  SYNC_COMMIT_POLICY_UNSPECIFIED     = 0;
  SYNC_COMMIT_POLICY_ALL_OR_DEFER    = 1; // Defer until all members have pending mutations
  SYNC_COMMIT_POLICY_AVAILABLE_MEMBERS = 2; // Apply available members; don't block
}

/// Mutation: create a sync group.
message CreateSyncGroupMutation {
  SyncGroupConfig config = 1;
}

/// Mutation: delete a sync group (tiles are removed from the group first).
message DeleteSyncGroupMutation {
  bytes id = 1; // SyncGroupId: 16-byte UUIDv7
}

/// Event: emitted when a sync group is force-committed after max deferral.
message SyncGroupForceCommitEvent {
  bytes  id                = 1; // SyncGroupId: 16-byte UUIDv7
  uint32 defer_frames_used = 2;
  uint64 frame_number      = 3;
}

// ─── Clock Sync ──────────────────────────────────────────────────────────────

/// Request from agent: ask compositor for its current clock.
message ClockSyncRequest {
  uint64 agent_timestamp_us = 1; // Agent's UTC microseconds at time of request
}

/// Response from compositor: provides clock reference for skew correction.
message ClockSyncResponse {
  uint64 compositor_monotonic_us  = 1; // Compositor monotonic clock at response time
  uint64 compositor_wallclock_us  = 2; // Compositor UTC wall clock at response time
  int64  estimated_skew_us        = 3; // Current skew estimate: agent_ts - compositor_ts
  bool   skew_within_tolerance    = 4;
  string warning                  = 5; // Non-empty if skew is in warning range
}

// ─── Frame Telemetry ─────────────────────────────────────────────────────────

/// Per-frame timing data, embedded in TelemetryRecord.
message FrameTimingRecord {
  uint64 frame_number             = 1;
  uint64 vsync_monotonic_us       = 2;  // Monotonic clock at vsync
  uint64 vsync_wallclock_us       = 3;  // UTC wall clock at vsync
  uint32 stage1_input_drain_us    = 4;  // Stage durations in microseconds
  uint32 stage2_local_feedback_us = 5;
  uint32 stage3_mutation_intake_us = 6;
  uint32 stage4_scene_commit_us   = 7;
  uint32 stage5_layout_resolve_us = 8;
  uint32 stage6_render_encode_us  = 9;
  uint32 stage7_gpu_submit_us     = 10;
  uint32 stage8_telemetry_emit_us = 11;
  uint32 total_frame_us           = 12;
  bool   over_budget              = 13; // total_frame_us > 16,667
  uint32 mutations_applied        = 14;
  uint32 mutations_deferred       = 15; // Held for next frame (present_at in future)
  uint32 mutations_dropped        = 16; // EphemeralRealtime drops
  uint32 tiles_expired            = 17; // Tiles removed by expires_at this frame
  uint32 sync_groups_deferred     = 18; // AllOrDefer groups that did not commit
  uint32 sync_groups_force_committed = 19;
  uint64 sync_group_max_drift_us  = 20; // Worst mutation-arrival spread within any single sync group this frame:
                                        // max over all committed sync groups of
                                        // (latest_member_arrival_us - earliest_member_arrival_us).
                                        // Zero if no sync group committed this frame.
                                        // Expressed in microseconds (monotonic clock domain).
  bool   sync_drift_budget_exceeded = 21; // True if sync_group_max_drift_us > timing.sync_drift_budget_us (default 500μs).
                                          // Doctrine: validation.md §Other performance budgets — "sync drift < 500μs".
}

// ─── Timing Config ───────────────────────────────────────────────────────────

/// Runtime timing configuration (loaded from TOML config; documented here for completeness).
message TimingConfig {
  uint32 target_fps                    = 1;  // Default 60
  uint32 max_agent_clock_drift_ms      = 2;  // Default 100
  uint32 max_vsync_jitter_ms           = 3;  // Default 2
  uint32 max_future_schedule_ms        = 4;  // Default 300,000 (5 minutes)
  uint32 sync_group_max_defer_frames   = 5;  // Default 3
  uint32 pending_queue_depth_per_agent = 6;  // Default 256
  uint32 sync_drift_budget_us          = 7;  // Default 500. Doctrine: validation.md §Other performance budgets ("sync drift < 500μs").
  uint32 tile_stale_threshold_ms       = 8;  // Default 5000. Staleness indicator activates if a STATE_STREAM/TRANSACTIONAL tile
                                             // receives no mutation for this many ms. Doctrine: failure.md §Agent is slow.
  // post-v1 fields:
  uint32 max_media_drift_ms            = 9;  // Default 10 (post-v1)
  // Round 2 addition (T-R4):
  uint32 clock_jump_detection_ms       = 10; // Default 50. If consecutive skew samples differ by more
                                             // than this value, the estimation window is reset to the
                                             // latest sample. Prevents miscorrection after NTP step
                                             // adjustments. See §4.3.
}

// ─── Clock Sync Service ───────────────────────────────────────────────────────
//
// ClockSync is a unary RPC available to resident agents for clock alignment.
// This service block documents the message contract. The preferred implementation
// adds ClockSync as a method on the SessionService in RFC 0005 (session.proto)
// rather than exposing a separate gRPC service endpoint, keeping all
// agent-runtime communication on one endpoint. See §7.2 for details.
//
// If a standalone service is required (e.g., for versioning), use this block.
service ClockSyncService {
  // Unary RPC: agent sends its current timestamp; compositor responds with
  // its clock reference and the current skew estimate. Agents SHOULD call
  // this once at session start (to avoid cold-start validation failures)
  // and after receiving CLOCK_SKEW_HIGH events. See §4.5.
  rpc ClockSync(ClockSyncRequest) returns (ClockSyncResponse);
}
```

### 7.2 Integration with `scene.proto`

**Timestamp unit migration (RFC 0001 → RFC 0003):** RFC 0001 used millisecond-resolution (`_ms`) timestamp fields throughout `scene.proto` and the Rust data model (e.g., `present_at_ms`, `expires_at_ms`, `committed_at_ms`). RFC 0003 establishes microsecond resolution (`_us`) as the authoritative standard, consistent with the architecture doctrine (CLAUDE.md: "μs resolution"). RFC 0001 must be updated in a follow-on amendment to rename all `_ms` timestamp fields to `_us` and change the units to UTC microseconds since Unix epoch. Until that amendment lands, `timing.proto` uses `_us` exclusively; implementors should treat the RFC 0001 `_ms` fields as pending migration.

The fields in `timing.proto` supplement the scene contract defined in RFC 0001. Key cross-references:

- `CreateSyncGroupMutation` and `DeleteSyncGroupMutation` are new variants in the `SceneMutation` oneof (RFC 0001 §8, field numbers **21 and 22** respectively). Field 20 is already occupied by `ClearZoneMutation` (RFC 0001 §8).
- `SyncGroupConfig` supplements `UpdateTileSyncGroupMutation` (RFC 0001 field 11): sync group creation is now an explicit, separate operation rather than an implicit side effect of assigning a tile to a group ID. `UpdateTileSyncGroupMutation` continues to handle tile membership changes; `CreateSyncGroupMutation` / `DeleteSyncGroupMutation` handle group lifecycle.
- `FrameTimingRecord` is embedded in `TelemetryRecord` (RFC 0002 §3.2, Stage 8).
- The `ClockSyncRequest`/`ClockSyncResponse` pair is a unary RPC. The **preferred implementation** adds a `ClockSync` method to the `SessionService` defined in RFC 0005 (session.proto). This keeps all agent-runtime communication on a single service endpoint. The `ClockSyncService` block in §7.1 documents the contract; RFC 0005 carries the normative `SessionService` definition. Do not create a second standalone gRPC service endpoint unless versioning requires it.

### 7.3 Wire Encoding Notes

1. All `uint64` timestamp fields use 0 to represent "not set." Zero is never a valid timestamp in this system.
2. `SyncGroupId.id` is exactly 16 bytes (UUIDv7) or all-zero bytes to represent absent. Agents must not send partially filled IDs.
3. `estimated_skew_us` in `ClockSyncResponse` is signed (`int64`) because skew can be positive (agent clock ahead) or negative (agent clock behind). A positive value means the agent's clock is ahead.
4. `delivery_policy` in `TimestampedPayload` is a protobuf enum (`DeliveryPolicy`). Implementations must treat unknown enum values as `DELIVERY_POLICY_DEFER`.

---

## 8. Timing in the Validation Architecture

### 8.1 Injectable Clock

All timing paths in the compositor use an injectable clock source, not direct calls to the OS clock. The `Clock` trait is injected at construction time:

```rust
pub trait Clock: Send + Sync + 'static {
    /// Current UTC microseconds since Unix epoch.
    fn now_us(&self) -> u64;
    /// Current value of the monotonic clock, in microseconds.
    fn monotonic_us(&self) -> u64;
}

pub struct SystemClock; // Production: uses std::time::SystemTime and Instant
pub struct SimulatedClock { /* atomic u64 cell, manually advanced */ } // Tests
```

This satisfies DR-V4 (deterministic test scenes): all timing-dependent behavior can be exercised with a `SimulatedClock` without wall-clock interference.

**Clock trait design constraint.** The `Clock` trait has no `advance` or `set_time` method. Time advancement is a concern of concrete test implementations only — `SimulatedClock::advance_by_us(delta: u64)` exists on the struct directly, not on the trait. Do not add mutation methods to the `Clock` trait: the trait is an observation interface; adding `advance` would require production implementations to carry dead no-ops and would violate single-responsibility.

### 8.2 Test Coverage Requirements

The following behaviors must be covered by Layer 0 (scene graph assertion) tests:

- `present_at_us` in the future: mutation is held in pending queue, not applied.
- `present_at_us` reached: mutation is applied at the correct frame.
- `expires_at_us` reached: tile is removed at the correct frame, expiry event emitted.
- `expires_at_us <= present_at_us`: mutation is rejected with `TIMESTAMP_EXPIRY_BEFORE_PRESENT`.
- `AllOrDefer` sync group with one member late: group defers; no partial apply.
- `AllOrDefer` sync group max deferral: force commit after N frames, event emitted.
- `AvailableMembers` sync group: applies available members, ignores absent.
- Clock skew > tolerance: rejection with structured error.
- Clock skew within tolerance: correction applied transparently.
- Expiry heap: O(expired_items) behavior verified with large tile sets.
- Pending queue flush on session close: queued entries are discarded on disconnect; not applied after reconnect.
- Expiry under degradation: expiry evaluation runs even when compositor is in ShedTiles or Emergency mode.
- Clock jump detection: consecutive samples differing by > `clock_jump_detection_ms` trigger window reset; subsequent corrections use single-point estimate.
- Tile deletion mid-deferral: deleting a tile that is the sole missing member of an `AllOrDefer` group unblocks the group in the same frame.

### 8.3 Chaos Test Requirements

The timing model must survive chaos injection (see `heart-and-soul/validation.md`):

- Clock discontinuities: simulated clock jumps forward by 10s — all pending queues drain correctly, expired tiles are removed, no crashes.
- Clock jumps backward: monotonic clock cannot go backward; the simulation must never do this. Network clock going backward (agent skew) triggers skew detection.
- Vsync jitter: vsync signals arriving at 14ms, 16ms, 20ms intervals — sync groups still commit atomically, expiry still fires at the correct UTC time.
- Pending queue saturation: 256+ mutations queued — 257th rejected with `PENDING_QUEUE_FULL`, no state corruption.
- Sync group thrash: rapid join/leave/create/delete of sync groups — no dangling tile references, no leaked group objects.

---

## 9. Open Questions

1. **`present_at_us` precision floor:** Should the API advertise that sub-frame precision (< 16.6ms) is silently rounded to the nearest frame, or should agents be told explicitly? Recommendation: document the frame quantization rule (§3.3) in the API and surface it as a structured warning in `ClockSyncResponse` if the agent is sending sub-frame precision timestamps.

2. **Sync group ownership transfer:** Can a sync group's `owner_namespace` be transferred? The current spec does not allow it. Implications: if the owner agent disconnects, the sync group must be destroyed or adopted. Recommendation: on owner disconnect, emit `SyncGroupOrphaned` event; all member tiles are released from the group; the group is destroyed after a 5s grace period.

3. **`AllOrDefer` with growing member sets:** if a sync group's membership changes while it has pending deferred mutations (e.g., a tile joins mid-deferral cycle), what is the correct behavior? Recommendation: the new member is not considered for the current deferral cycle; it joins the group's next evaluation epoch. This prevents membership churn from extending deferral indefinitely.

4. **Pending queue ordering for equal `present_at_us`:** When two mutations from the same agent have the same `present_at_us`, they are applied in FIFO arrival order. This is deterministic but may not match the agent's intent for simultaneous operations. Recommendation: document this and recommend agents use distinct `present_at_us` values or a single batch for simultaneous operations.

5. **Expiry precision under load:** If the compositor is shedding work (§5.2 of RFC 0002), expiry evaluation at Stage 4 still runs. However, a tile that was due to expire at frame N might not be visibly removed until frame N+1 if the compositor is overloaded. Recommendation: expiry evaluation is non-negotiable; it must run even under load. The compositor should not shed Stage 4 work. Document that expiry latency is at most one frame even under load.
