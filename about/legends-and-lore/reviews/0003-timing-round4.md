# Review: RFC 0003 Timing Model — Round 4 (Final Hardening and Quantitative Verification)

**Issue:** rig-5vq.22
**Reviewer:** Beads worker agent
**Date:** 2026-03-22
**Round:** 4 of 4 — Final Hardening and Quantitative Verification
**Doctrine files consulted:** architecture.md, validation.md, v1.md

---

## Methodology

Focus: shipping readiness. Is every requirement testable? Is every protobuf message
fully specified? Are all platform targets addressed? Are all open questions resolved?
Would an LLM worker be able to implement this RFC without ambiguity?

The RFC as it exists after Round 3 fixes (including the rig-fim correctness pass,
rig-6k5 ID type unification, and rig-9se AllOrDefer normative spec pass) is the
base for this review.

---

## Scores

| Dimension | Round 1 | Round 2 | Round 3 | Round 4 |
|-----------|---------|---------|---------|---------|
| Doctrinal Alignment | 4/5 | 4/5 | 4/5 | **5/5** |
| Technical Robustness | 4/5 | 4/5 | 4/5 | **5/5** |
| Cross-RFC Consistency | 3/5 | 4/5 | 4/5 | **5/5** |

All dimensions ≥ 4. RFC hardened and ready for implementation.

---

## Doctrinal Alignment: 5/5

All doctrinal commitments held and strengthened. The RFC fully implements:

- `architecture.md §Time is a first-class API concept`: `present_at`, `effective_after`, `expires_at`, `sequence`, `priority`, `coalesce_key`, `sync_group` all specified with quantitative semantics.
- `architecture.md §Message classes`: `MessageClass` enum with four typed values; `DeliveryPolicy` constraint (`DROP_IF_LATE` only on `EPHEMERAL_REALTIME`).
- `validation.md §DR-V4 (deterministic test scenes)`: injectable `Clock` trait with `SystemClock` and `SimulatedClock` implementations. `Clock::advance` excluded from trait to prevent production dead code.
- `validation.md §Other performance budgets ("sync drift < 500μs")`: `sync_drift_budget_us` in `TimingConfig` (default 500μs); measured per-frame as `sync_group_max_drift_us`; triggers `sync_drift_budget_exceeded` flag and staleness indicator.
- `failure.md §Agent is slow`: staleness indicators on both content staleness (5s no mutations) and sync group drift (>500μs arrival spread).
- `validation.md §Layer 3 split latency budgets`: all three split metrics (`input_to_local_ack`, `input_to_scene_commit`, `input_to_next_present`) with quantitative budgets, not just a single "input to pixel" conflation.

Round 4 promotions completed the doctrinal commitment surface: all five open
questions resolved to normative text or explicit post-v1 deferrals. No remaining
vagueness detectable. All requirements are quantitative with units.

Score upgraded from 4/5 to 5/5.

---

## Technical Robustness: 5/5

The clock domain hierarchy, frame deadline model, drift correction arithmetic,
expiry heap, and sync group commit policies were all sound coming into this round.
The following issues were found and fixed:

### [MUST-FIX → FIXED] R4-T1: `max_future_schedule_ms` unit inconsistency creates silent implementation trap

**Location:** §3.5 (timestamp validation table), `TimingConfig` proto (§7.1 field 4)

**Problem:** The timestamp validation rule in §3.5 was:
```
present_at_us > current_wallclock_us + max_future_schedule_us
```
with "Default `max_future_schedule_us`: 300_000_000 (5 minutes)" in the prose. But
`TimingConfig` defined the field as `max_future_schedule_ms = 4  // Default 300,000 (5 minutes)`.

The field name uses `_ms` while the comparison is against a `_us` field. An implementor
reading only the proto would use `max_future_schedule_ms * 1_000` to get microseconds —
but this implicit conversion was not documented anywhere. If they instead used the
field directly, the effective limit would be 300 milliseconds (not 5 minutes), silently
rejecting all non-trivial future scheduling.

This is a correctness trap: the field unit in the proto does not match the unit used
in the comparison, and the discrepancy is not called out.

**Fix applied:** Renamed `max_future_schedule_ms` → `max_future_schedule_us` in
`TimingConfig` proto (field 4), with default 300,000,000 (microseconds, 5 minutes).
Updated §3.5 prose to reference `timing.max_future_schedule_us` with explicit unit
annotation. Updated §8.2 test coverage to verify the comparison uses microseconds
directly with no conversion.

**Rationale:** RFC 0003 §3.1 mandates that scheduling fields use `_us` units. The
`max_future_schedule_us` field is directly compared against `present_at_us` — it
must be in the same unit. The `_ms` fields in `TimingConfig` (drift thresholds,
jitter budgets) are coarse thresholds not used in direct timestamp comparisons;
the `_ms` suffix is accurate for those fields. Only this one field was mislabeled.

---

### [MUST-FIX → FIXED] R4-T2: Open Questions 2, 3, 4 contained required normative behaviors left as questions

**Location:** §9 Open Questions (pre-Round 4)

**Problem:** Three open questions had concrete, unambiguous recommendations that
an implementor needs to build a correct system:

- **OQ2** (sync group ownership transfer): described what happens when an owner
  disconnects, recommended `SyncGroupOrphaned` event, 5s grace period, member
  release — but left as a question with no proto definition.
- **OQ3** (AllOrDefer with growing member sets): gave a clear recommendation
  (new members excluded from current epoch) but left it as a question.
- **OQ4** (equal `present_at_us`): stated FIFO as correct behavior but buried it
  in the question.

Leaving these as open questions means an implementor making independent decisions
could produce incompatible behavior. The recommendations were unambiguous and
complete enough to be normative.

**Fix applied:**
- §2.3 now has "Owner namespace disconnect" section: non-transferable ownership;
  `SyncGroupOrphanedEvent` emitted to all subscribers; all member tiles released
  from group; group destroyed after 5-second grace period; reconnecting owner can
  cancel destruction within grace period.
- §2.3 now has "Joining during an active deferral cycle" section: new member
  excluded from current epoch; evaluated starting from next epoch; prevents
  membership churn from extending deferral.
- §5.3 now has "Equal `present_at_us` ordering" section: FIFO arrival order is
  normative; agents should use single batches or distinct timestamps.
- `SyncGroupOrphanedEvent` proto message added to §7.1.
- All three promoted behaviors added to §8.2 test coverage requirements.

---

### [SHOULD-FIX → FIXED] R4-T3: `TimingHints.sync_group_id` uses raw `bytes` while all other SyncGroupId fields use `SceneId`

**Location:** `TimingHints` proto message (§7.1)

**Problem:** Every other field carrying a `SyncGroupId` in `timing.proto` uses the
typed `SceneId` message from `scene.proto`: `SyncGroupConfig.id`, `DeleteSyncGroupMutation.sync_group_id`, `SyncGroupForceCommitEvent.id`, and now `SyncGroupOrphanedEvent.id`. Only `TimingHints.sync_group_id` used raw `bytes`, with a comment explaining it was "16-byte UUIDv7" — effectively an inline untyped `SceneId`.

The inconsistency creates a type system gap: generated code has `bytes` in one place
and `SceneId` in every other, requiring a special-case conversion at the call site
where `TimingHints` is used. The §7.3 note tried to explain this, but the explanation
is harder to follow than fixing the type.

**Fix applied:** Changed `bytes sync_group_id` → `SceneId sync_group_id` in
`TimingHints`. Updated the comment to clarify the RFC 0001 §1.1 encoding. Updated
§7.3 wire encoding note to list all SyncGroupId usages uniformly.

---

### [SHOULD-FIX → FIXED] R4-T4: `TimingConfig` fields had no validation range specification

**Location:** §7.1 (TimingConfig proto), §9 (Open Questions)

**Problem:** The Round 3 review noted that RFC 0006 lacks a `[timing]` section,
but the underlying issue is that `TimingConfig` itself specifies defaults but
not valid ranges. An implementor building the config layer has no guidance on:
- What is the minimum valid `target_fps`? (1? 15? 30?)
- What is the maximum `pending_queue_depth_per_agent`? (Unbounded means OOM risk)
- Can `sync_drift_budget_us` be 0? (If so, every sync group fires `sync_drift_high`)

**Fix applied:** Added §10 "Configuration Integration (RFC 0006 Pending Amendment)"
with a table of all `TimingConfig` fields, their defaults, and valid ranges. This
serves as the authoritative reference for implementors building the configuration
layer and as a pre-spec for the pending RFC 0006 `[timing]` amendment.

---

## Cross-RFC Consistency: 5/5

No new cross-RFC inconsistencies found.

The `TimingHints.sync_group_id` → `SceneId` change aligns with RFC 0001's
type authority and eliminates the last inconsistency in SyncGroupId representation
across `timing.proto`.

The `max_future_schedule_us` rename applies the `_us` convention that RFC 0003
itself mandates in §3.1, maintaining internal naming consistency.

All previous round fixes held:
- `ClockSync` RPC on `SessionService` (RFC 0005) — verified correct.
- `TimingHints` naming convention (`_wall_us`) — verified correct.
- `DeleteSyncGroupMutation.sync_group_id` field name — verified correct.
- `CreateSyncGroupMutation` wire format — verified correct.
- `ZonePublish.ttl_us` (RFC 0005 Round 11) — verified correct.

Score maintained at 5/5.

---

## Quantitative Verification

### Algorithm Correctness: VERIFIED

| Algorithm | Correctness Assessment |
|-----------|----------------------|
| Frame quantization: `T <= frame_F_vsync_us` | Correct. Strict no-earlier-than; no half-frame slop (fixed in rig-fim). |
| Drift correction: `corrected = (agent_ts as i64) - skew_us` | Correct. Signed arithmetic prevents uint64 underflow. |
| Drift estimation: 32-sample median | Correct. Median suppresses outlier spikes; window reset on jump detection. |
| Force-commit: `deferred_frames_count >= max_defer_frames(G)` | Correct. Counter increments only on incomplete deferrals (not idle). |
| Expiry heap drain: `expires_at_us <= current_frame_vsync_us` | Correct. O(expired_items). Non-negotiable under load. |
| Pending queue drain: `present_at_us <= current_frame_vsync_us` | Correct. Consistent with frame quantization rule. |

### Drift Bounds: REALISTIC

| Budget | Value | Achievability at 60fps |
|--------|-------|------------------------|
| p99 frame time | < 16.6ms | Achievable on reference hardware (no media decode in v1). |
| input_to_local_ack | p99 < 4ms | Substantial headroom (stages 1+2 ≤ 1ms combined). |
| input_to_scene_commit | p99 < 50ms | Achievable for localhost agents. Remote agents: document limitation. |
| input_to_next_present | p99 < 33ms | Within two frames at 60Hz. Achievable. |
| Sync drift | < 500μs | Tight but correct. Two localhost gRPC agents: 100–500μs spread typical. |
| Agent clock drift | 100ms warn / 1s reject | Appropriate for LAN/localhost. |
| Future schedule window | 5 minutes | Conservative. Correct unit (μs) after R4-T1 fix. |
| Max defer frames | 3 (default, ≈50ms) | Appropriate for typical network jitter. |
| Pending queue depth | 256 per agent | Adequate for normal use; prevents queue-based OOM. |

### Testability: VERIFIED

All behaviors in §8.2 and §8.3 are testable with the `SimulatedClock`:

- Every temporal path depends only on `Clock::now_us()` / `Clock::monotonic_us()`.
- `SimulatedClock::advance_by_us()` advances time without wall-clock interference.
- All state machine transitions are deterministic given a controlled clock.
- Chaos tests (clock jumps, queue saturation, sync group thrash) cover the known failure modes.

No behavioral gap was found where a test would require a physical clock.

---

## Platform Assessment

| Platform | Assessment |
|----------|------------|
| Linux (X11/Wayland) | `CLOCK_MONOTONIC` for monotonic; `CLOCK_REALTIME` for UTC. Frame timing via DRM vsync events. No gaps. |
| Windows (Win32/D3D12) | `QueryPerformanceCounter` for monotonic; `GetSystemTimePreciseAsFileTime` for UTC. DXGI vsync. No gaps. |
| macOS (Cocoa/Metal) | `mach_absolute_time` for monotonic; `clock_gettime(CLOCK_REALTIME)` for UTC. `CVDisplayLink` for vsync. No gaps. |
| Headless (CI) | `tokio::time::interval` synthetic vsync (RFC 0002 §7). `SimulatedClock` or system clock. No physical display required. |

All four platforms are addressed without platform-specific timing contracts — the
`Clock` trait abstraction handles the OS differences cleanly.

---

## Implementor Completeness Check

An implementor with only RFC 0003 can build:

- [x] All four clock domains with correct authority and resolution.
- [x] Injectable `Clock` trait with production and test implementations.
- [x] `SyncGroup` Rust struct with `deferred_frames_count` tracking.
- [x] `AllOrDefer` and `AvailableMembers` commit policies with force-commit state machine.
- [x] Drift estimation, correction, and rejection logic.
- [x] Pending queue with session-scoped flush.
- [x] Expiry heap with O(expired_items) drain.
- [x] `TimestampedPayload` with all six timing fields.
- [x] `TimingConfig` with valid ranges (§10).
- [x] All protobuf messages and the `ClockSyncService` / `SessionService` RPC.
- [x] All error codes: `TIMESTAMP_TOO_OLD`, `TIMESTAMP_TOO_FUTURE`, `TIMESTAMP_EXPIRY_BEFORE_PRESENT`, `INVALID_DELIVERY_POLICY`, `CLOCK_SKEW_EXCESSIVE`, `PENDING_QUEUE_FULL`.
- [x] All telemetry fields in `FrameTimingRecord`.

No "TBD", no unresolved design choice, no ambiguous behavior remains.

---

## Changes Applied

### RFC 0003 (about/legends-and-lore/rfcs/0003-timing.md)

- §Review History: Added Round 4 summary.
- §2.3: Added "Joining during an active deferral cycle" (OQ3 → normative).
- §2.3: Added "Owner namespace disconnect" (OQ2 → normative) with `SyncGroupOrphanedEvent` reference.
- §5.3: Added "Equal `present_at_us` ordering" (OQ4 → normative).
- §7.1 `timing.proto`: Renamed `max_future_schedule_ms` → `max_future_schedule_us` (field 4, default 300,000,000); changed `TimingHints.sync_group_id` from `bytes` to `SceneId`; added `SyncGroupOrphanedEvent` message.
- §7.3: Updated wire encoding note for uniform `SceneId` usage and `max_future_schedule_us` units.
- §8.2: Added six new test coverage requirements covering the promoted OQ behaviors and unit consistency.
- §9: Promoted OQ2, OQ3, OQ4 to resolved status with section references; clarified OQ1 and OQ5 status.
- §10 (new): "Configuration Integration" with `[timing]` field table, defaults, and validation ranges.
