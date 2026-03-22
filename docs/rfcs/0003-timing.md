# RFC 0003: Timing Model

**Status:** Draft
**Issue:** rig-5vq.3
**Date:** 2026-03-22
**Authors:** tze_hud architecture team

---

## Review History

### Feature Addition — Relative Scheduling Primitives (rig-ohm)

**Author:** Beads worker agent
**Date:** 2026-03-22
**Issue:** rig-ohm

#### Changes Applied

- **[FEATURE → ADDED]** §3.2: Added `after_us`, `frames_from_now`, and `next_frame` to the Timestamp and Ordering Fields table. Added normative note block defining the `oneof schedule` wire encoding, mutual exclusivity, conversion semantics, and validation rules for relative scheduling fields.
- **[FEATURE → ADDED]** §5.3: Added §5.3.1 "Relative Scheduling Primitives" specifying how the compositor converts `after_us`, `frames_from_now`, and `next_frame` to absolute `present_at_us` at Stage 3 intake. Defines the monotonic-clock conversion path, frame-number calculation, and pending queue behavior after conversion.
- **[FEATURE → ADDED]** §7.1: Updated `TimestampedPayload` to use `oneof schedule` for `present_at_us`, `after_us`, `frames_from_now`, and `next_frame`. Updated `TimingHints` similarly. Added `RELATIVE_SCHEDULE_CONFLICT` error code and `RelativeScheduleConfig` to `TimingConfig`. Added `relative_scheduling` section to §7.3 wire encoding notes.
- **[FEATURE → ADDED]** §8.2: Added test coverage requirements for all three relative scheduling primitives, including conversion accuracy, frame boundary behavior, error rejection, and pending queue ordering after conversion.

**Doctrinal basis:** architecture.md §Time is a first-class API concept — "payloads carry timing semantics." The relative primitives are wire-level convenience sugar only; they are converted to absolute timestamps at Stage 3 intake and never enter the scene graph, telemetry, or stored state. This is consistent with the internal timing model already specified in §3 and §5.

---

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
- **[CONSIDER → SUBSEQUENTLY FIXED in rig-fim]** Frame quantization boundary condition noted in §3.3; at the time of Round 2 this was flagged as a boundary condition only. Post-Round 2 analysis (rig-fim) identified this as a correctness bug: the half-frame tolerance allowed mutations to present before their declared `present_at_us`, violating the no-earlier-than guarantee in §5.3. Fixed in rig-fim correctness pass.

#### Cross-RFC Consistency: 4/5 (up from 3/5)

- **[MUST-FIX → FIXED]** `ClockSync` RPC was referenced in §4.5/§7.2 but absent from RFC 0005 session service. Added `rpc ClockSync` to `SessionService` in RFC 0005 §9. Added `ClockSyncService` to `timing.proto` §7.1 with a cross-reference note in §7.2 clarifying that the RPC is hosted on `SessionService` (not `ClockSyncService`) to keep all agent-runtime communication on one endpoint.
- **[MUST-FIX → FIXED in RFC 0005]** `TimingHints.sync_group_id` was `string` in RFC 0005's inline definition but `bytes` in RFC 0003. Fixed the RFC 0005 inline `TimingHints` to `bytes`.
- **[SHOULD-FIX → FIXED in RFC 0005]** `session_open_at_us` clock reference not exposed in `SessionEstablished`; agents had no way to align their clocks at handshake without an extra RPC. Added `compositor_wallclock_us` and `estimated_skew_us` to `SessionEstablished`.
- **[CONSIDER]** `ZonePublish.ttl_ms` unit inconsistency deferred to RFC 0001 `_ms → _us` amendment sweep.

**No dimension below 3. Round 2 findings addressed. Ready for Round 3.**

---

### Correctness Fix — Frame Quantization (rig-fim)

**Reviewer:** Beads worker agent
**Date:** 2026-03-22

#### Changes Applied

- **[MUST-FIX → FIXED]** §3.3 used a half-frame tolerance in the quantization rule (`T <= frame_F_vsync_us + frame_budget_us / 2`), which contradicted §5.3's normative "no earlier than T" guarantee. A mutation with `present_at_us = V + 8ms` would be applied at frame F (vsync at V), presenting 8ms before the declared time. Fixed by replacing the half-frame rule with a strict vsync boundary: `T <= frame_F_vsync_us`. Added explanatory note on the expected one-frame delay for timestamps just after a vsync.
- **[MUST-FIX → FIXED]** §5.3 pending queue drain condition used `present_at_us <= current_frame_vsync_us + half_frame`, propagating the same early-presentation bug into the implementation contract. Fixed to `present_at_us <= current_frame_vsync_us` to match the corrected §3.3 rule and the no-earlier-than guarantee.

---

### Cross-RFC Pass — ID Type Unification (rig-6k5)

**Reviewer:** Beads worker agent
**Date:** 2026-03-22

#### Changes Applied

- **[MUST-FIX → FIXED]** §7.1 proto comment contained an incorrect `SceneId` definition: `message SceneId { bytes id = 1; }`. The authoritative definition in RFC 0001 §7.1 is `message SceneId { bytes bytes = 1; }`. Comment corrected.
- **[MUST-FIX → FIXED]** `SyncGroupConfig.id` changed `bytes id` → `SceneId id` (sync groups are scene objects with SceneId per RFC 0001 §1.1). Requires `import "scene.proto"` in `timing.proto`.
- **[MUST-FIX → FIXED]** `DeleteSyncGroupMutation.id` changed `bytes id` → `SceneId id`.
- **[MUST-FIX → FIXED]** `SyncGroupForceCommitEvent.id` changed `bytes id` → `SceneId id`.
- **[FIXED]** Added `import "scene.proto"` to the `timing.proto` header.

---

### Normative Spec Pass — AllOrDefer Force-Commit (rig-9se)

**Reviewer:** Beads worker agent
**Date:** 2026-03-22

#### Changes Applied

- **[MUST-FIX → FIXED]** `max_defer_frames` was referenced only implicitly in §2.4 prose ("Maximum deferral: configurable, default 3 frames"). Added a normative paragraph to §2.3 naming `SyncGroupConfig.max_defer_frames` explicitly and documenting its semantics, default, and the meaning of 0.
- **[MUST-FIX → FIXED]** §2.4 referenced `SyncGroupForceCommitEvent` and described the force-commit trigger in a single sentence but provided no normative state machine. Added §2.4.1 "Force-Commit Semantics" specifying: trigger condition, member partitioning (present-and-ready vs absent), mutation disposition (present applied, absent discarded), `deferred_frames_count` increment rule, temporary policy transition, and `SyncGroupForceCommitEvent` contract.
- **[MUST-FIX → FIXED]** The prose in §2.4 stated "transitions to `AvailableMembers` policy for that commit cycle only" but the proto and Rust model had no mechanism to track this. Clarified in §2.4.1 that this is not stored as an enum state: it is the natural result of the force-commit procedure, with `deferred_frames_count` as the sole required tracking field.
- **[MUST-FIX → FIXED]** `SyncGroupForceCommitEvent` proto had only 3 fields (`id`, `defer_frames_used`, `frame_number`). Added `present_member_ids`, `absent_member_ids`, and `mutations_discarded` to make the event actionable for consumers diagnosing split-brain state.
- **[MUST-FIX → FIXED]** `SyncGroup` Rust struct had no field to track deferral state. Added `deferred_frames_count: u32` with a normative doc comment specifying increment/reset rules.
- **[MUST-FIX → FIXED]** §8.2 (Test Coverage) had only a single line about force-commit. Expanded to cover: mutation disposition (present applied, absent discarded), policy recovery after force-commit, counter increment rule, custom `max_defer_frames` values (including 0 = runtime default).

---

### Round 3 — Cross-RFC Consistency and Integration (rig-5vq.21)

**Reviewer:** Beads worker agent
**Date:** 2026-03-22
**Doctrine files reviewed:** architecture.md, validation.md

#### Doctrinal Alignment: 4/5

No new doctrinal gaps found. Round 1 and 2 fixes held. The RFC correctly implements all architecture.md timing mandates, all four message classes, the injectable clock, and the sync drift budget from validation.md. Score unchanged from prior rounds.

#### Technical Robustness: 4/5

No new technical gaps found. Force-commit state machine, frame quantization, drift correction, and pending queue semantics are all sound. Score unchanged from prior rounds.

#### Cross-RFC Consistency: 4/5 (up from 3/5 in Round 1, maintained from Round 2)

The following issues were identified during this round's systematic cross-RFC pass:

- **[MUST-FIX → FIXED]** `FrameTimingRecord.vsync_monotonic_us`/`vsync_wallclock_us` and `ClockSyncResponse.compositor_monotonic_us`/`compositor_wallclock_us` used the old generic `_us` suffix for clock-domain fields. RFC 0005 Round 6 (rig-77n) established the explicit `_wall_us`/`_mono_us` convention to encode clock domain in every field name. RFC 0005 §7.1 even includes a note: "if RFC 0003 still uses the old _us suffix, treat [the RFC 0005 definition] as the intended final form; RFC 0003 §7.1 should be updated to match." Fixed: renamed `vsync_monotonic_us` → `vsync_mono_us`, `vsync_wallclock_us` → `vsync_wall_us`, `compositor_monotonic_us` → `compositor_mono_us`, `compositor_wallclock_us` → `compositor_wall_us`. Updated corresponding prose in §1.3 and §4.3. The internal `session_open_monotonic_us`/`session_open_wallclock_us` names in §1.3 prose are also updated.

- **[MUST-FIX → FIXED]** RFC 0005 §9.1 import graph claims `timing.proto` exports `TimingHints`, but `timing.proto` (RFC 0003) defined no `TimingHints` message — RFC 0005 relied on an inline definition. Added a canonical `TimingHints` message to §7.1 using the `_wall_us` naming convention (matching RFC 0005's inline definition), so the import reference is now satisfied. The inline definition in RFC 0005 correctly notes that RFC 0003 is authoritative.

- **[MUST-FIX → FIXED]** `DeleteSyncGroupMutation.id` field name mismatch: RFC 0003 used `id = 1` while RFC 0001 uses `sync_group_id = 1`. Wire-format compatible (same field number) but generated-code name clash. Fixed: aligned RFC 0003 to `sync_group_id = 1`, matching RFC 0001 (the scene-contract authority).

- **[MUST-FIX → FIXED]** `CreateSyncGroupMutation` structural mismatch: RFC 0001 defined `{ SceneId id = 1; bytes config = 2; }` (opaque bytes, id as separate field) while RFC 0003 defined `{ SyncGroupConfig config = 1; }` (typed message, id embedded in config at field 1). These are wire-incompatible (different field count, different field-1 type). Fixed: RFC 0001 §7.1 is updated to match RFC 0003's canonical definition — `{ SyncGroupConfig config = 1; }` where `SyncGroupConfig.id` carries the group id. Updated RFC 0001 §7.1 and the Rust enum variant comment. Cross-reference note added to both RFCs.

- **[MUST-FIX → FIXED in RFC 0005]** `ZonePublish.ttl_ms` in RFC 0005 contradicts RFC 0003's mandate that all timing fields use `_us` units. RFC 0005 §8.2 MCP tool description also references `auto_clear_ms` while RFC 0001 uses `auto_clear_us`. Fixed in RFC 0005 §9: `ttl_ms` renamed to `ttl_us` with unit annotation; `auto_clear_ms` prose references updated to `auto_clear_us`. `heartbeat_interval_ms` is kept as-is — it is a session keepalive interval, not a UTC timestamp or scheduling field, and its `_ms` suffix is accurate (the value is in milliseconds).

- **[CONSIDER]** RFC 0006 has no `[timing]` section, despite RFC 0003's `TimingConfig` defining ten configurable parameters (`target_fps`, `max_agent_clock_drift_ms`, `sync_group_max_defer_frames`, etc.). Until RFC 0006 is amended, these parameters live only in the protobuf definition. Recommendation: a follow-on RFC 0006 amendment should add `[timing]` with all `TimingConfig` fields, validation rules, and TOML examples. Not addressed in this round — out of scope for a timing RFC review.

**No dimension below 3. Round 3 findings addressed. Ready for Round 4 (Final Hardening).**

---

### Override State Interaction — Freeze and Safe Mode Timing Behavior (rig-zts)

**Reviewer:** Beads worker agent
**Date:** 2026-03-22
**Issue:** rig-zts — RFC 0003: Define timer behavior during freeze and safe mode

#### Changes Applied

- **[MUST-FIX → FIXED]** Added §5.6 "Override State Interaction" specifying normative behavior for all six timing-sensitive constructs (present_at, expires_at, sync group deferral counters, staleness timers, clock-skew estimation window, and headless virtual clock) during freeze and safe mode. Grounded in RFC 0007 §4.3 (freeze queue semantics), RFC 0007 §5 (safe mode session suspension), and architecture.md §Policy arbitration (human override at position 1).
- **[MUST-FIX → FIXED]** §2.4 "Timing Contract" — added a freeze cross-reference: `deferred_frames_count` does not increment while freeze is active. See §5.6.1.
- **[MUST-FIX → FIXED]** §4.3 "Drift Detection" — added a safe mode cross-reference: the clock-skew estimation window is frozen during `SessionSuspended`; on `SessionResumed`, the window is reset to empty. See §5.6.2.
- **[MUST-FIX → FIXED]** §4.7 "Staleness Indicators" — added freeze and safe mode exceptions: staleness timers are suspended during freeze; tile staleness is suppressed while a session's `SessionSuspended` is active. See §5.6.1 and §5.6.2.
- **[MUST-FIX → FIXED]** §5.3 "Presentation Deadline (present_at)" — added freeze behavior: the pending queue drain condition is not evaluated during freeze. On unfreeze, mutations whose `present_at_us` has passed are applied immediately (same-frame flush). See §5.6.1.
- **[MUST-FIX → FIXED]** §5.4 "Expiration Policy (expires_at)" — added freeze behavior: expiry heap evaluation is suspended during freeze. On unfreeze, all tiles whose `expires_at_us <= current_frame_vsync_us` are expired immediately in the first post-unfreeze Stage 4. See §5.6.1.
- **[MUST-FIX → FIXED]** §8.2 "Test Coverage Requirements" — added seven new test cases covering override-state timing behavior.
- **[SHOULD-FIX → FIXED]** Added `FrameTimingRecord.frozen` and `FrameTimingRecord.safe_mode_active` boolean fields to §7.1 proto, allowing telemetry consumers to identify frames affected by override states without needing to cross-reference `ChromeState`.

---

### Round 4 — Final Hardening and Quantitative Verification (rig-5vq.22)

**Reviewer:** Beads worker agent
**Date:** 2026-03-22
**Doctrine files reviewed:** architecture.md, validation.md, v1.md

#### Doctrinal Alignment: 5/5

All prior doctrinal commitments held. The RFC fully implements:
- "Arrival time is not presentation time" — enforced at every layer through §3, §5.
- All four message classes with typed `MessageClass` enum and `DeliveryPolicy` constraints.
- Injectable clock satisfying DR-V4.
- Sync drift budget (500μs) from validation.md, with telemetry measurement and alerting.
- Staleness indicators from failure.md §Agent is slow.
- Three split latency metrics from validation.md §Layer 3.
- `TimingConfig` fully parameterized and defaults documented.

Round 4 promotions improved doctrinal completeness: all five open questions resolved to normative text or explicit post-v1 deferrals. No remaining vagueness.

Score upgraded to 5/5 — all doctrinal commitments are implemented, quantitative, and unambiguous.

#### Technical Robustness: 5/5

The following MUST-FIX and SHOULD-FIX items were found and fixed:

- **[MUST-FIX → FIXED]** R4-T1: `max_future_schedule_ms` in `TimingConfig` used `_ms` suffix but is directly compared against `present_at_us` in §3.5 — an implicit unit conversion not documented anywhere. Renamed to `max_future_schedule_us` with default 300_000_000 (microseconds). Updated §3.5 prose to reference `timing.max_future_schedule_us` explicitly with unit annotation. Eliminates a silent implementation trap.

- **[MUST-FIX → FIXED]** R4-T2: Open Questions 2, 3, 4 contained concrete, unambiguous behaviors that an implementor needs but were left as questions. Promoted to normative text: §2.3 now specifies owner-namespace disconnect behavior (orphan event, 5s grace period, member release); §2.3 specifies join-during-deferral epoch boundary; §5.3 specifies FIFO ordering for equal `present_at_us`. Added `SyncGroupOrphanedEvent` proto to §7.1 and corresponding test coverage to §8.2.

- **[SHOULD-FIX → FIXED]** R4-T3: `TimingHints.sync_group_id` used raw `bytes` while all other SyncGroupId fields in `timing.proto` use `SceneId`. Changed to `SceneId` for type system consistency. Updated §7.3 wire encoding note.

- **[SHOULD-FIX → FIXED]** R4-T4: `TimingConfig` parameters had no specification of valid ranges, leaving configuration validation undefined for implementors. Added §10 "Configuration Integration" with a validation table for all `TimingConfig` fields, documenting expected `[timing]` TOML section pending RFC 0006 amendment.

Score upgraded to 5/5 — all identified technical gaps addressed; the RFC is production-ready with no remaining ambiguities.

#### Cross-RFC Consistency: 5/5

No new cross-RFC inconsistencies found. All changes from Round 3 hold. The `TimingHints.sync_group_id` → `SceneId` change aligns with RFC 0001's type authority for all `SceneId` usages. The `max_future_schedule_us` rename uses the `_us` convention consistently with RFC 0003's own mandate.

Score maintained at 5/5.

**All scores ≥ 4. All open questions resolved. RFC hardened and ready for implementation.**

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

**vsync sync point (per-frame).** At the start of each frame, the compositor records: `frame_number`, `vsync_mono_us` (monotonic clock value at vsync), and `vsync_wall_us` (UTC wall clock value at vsync, sampled once and cached). This triple is the canonical sync point for that frame and is included in the `TelemetryRecord`.

**Agent session sync point (per-handshake).** During agent session establishment (gRPC handshake), the compositor records the `session_open_mono_us` and `session_open_wall_us`. The difference is the session's initial clock-skew estimate. Subsequent agent-supplied timestamps are validated against this estimate.

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
    /// For AllOrDefer groups: counts consecutive frames during which the group
    /// was deferred because at least one member had a pending mutation but at
    /// least one other member was absent. Incremented at Stage 4 on each
    /// incomplete-deferral frame; reset to 0 on normal commit or force-commit.
    /// Has no meaning for AvailableMembers groups (always 0).
    /// When this value reaches max_defer_frames(G), the Stage 4 evaluation
    /// for the current frame triggers a force-commit (see §2.4.1).
    pub deferred_frames_count: u32,
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

**Joining during an active deferral cycle.** If a tile joins an `AllOrDefer` group that currently has pending deferred mutations (i.e., `deferred_frames_count > 0`), the joining tile is NOT considered for the current deferral cycle. The new member is evaluated starting from the **next evaluation epoch** — the first Stage 4 evaluation after the current deferral cycle either commits or force-commits. This prevents a newly joining tile from immediately blocking an already-in-progress deferral (which would extend deferral time unexpectedly). Concretely: when the compositor evaluates whether the group is complete for the current frame, it uses the member set as it was at the START of the current deferral cycle, not the current member set. The joining tile is recorded in the group but contributes to commit-policy evaluation only from the next epoch onward.

**Leaving a sync group.** A tile leaves via `UpdateTileSyncGroup { sync_group: None }`. After leaving, the tile's mutations are no longer subject to group commit constraints. Leaving is applied at the next scene commit.

**Destroying a sync group.** A sync group is destroyed when explicitly deleted or when its last member leaves. Destruction removes the group from the scene graph. Any tile that still holds a reference to the group ID is automatically released from it.

**Owner namespace disconnect.** A sync group's `owner_namespace` cannot be transferred. When the agent session owning the namespace that created a sync group closes (gracefully or ungracefully), the compositor treats the group as orphaned:

1. Emit a `SyncGroupOrphanedEvent` (see §7.1 proto) to all agents with active event subscriptions.
2. Release all member tiles from the group (identical to each member calling `UpdateTileSyncGroup { sync_group: None }`). The tiles themselves are not deleted; only their group membership is cleared.
3. Destroy the sync group after a **5-second grace period**. If the owner namespace's session reconnects within the grace period (within the session reconnection window defined in RFC 0005 §6.3) and re-creates the group, the in-flight grace-period destruction is cancelled and the new group ID takes over. Tiles that were released during the orphan phase must be explicitly re-joined by the reconnecting agent.

This behavior ensures that cross-agent sync groups do not deadlock waiting for a member that will never return. The `SyncGroupOrphanedEvent` gives participating agents visibility to clean up their own deferred mutations.

**Tile deletion mid-deferral.** An explicit `DeleteTile` mutation causes the tile to leave its sync group before deletion, identical to expiry behavior (see §5.4). The sync group's commit policy evaluates against the updated member set after the deletion. If the deleted tile was the only missing member in an `AllOrDefer` group, removing it unblocks the group: the remaining members' pending mutations may now commit in the same frame as the deletion.

**Cross-agent sync groups.** Multiple agents can place tiles into the same sync group. The group does not belong to any single agent's mutation batch — it is a scene-graph object. When Agent A and Agent B both have tiles in the same sync group, their mutations are held in a pending queue until the commit policy's condition is satisfied. The compositor evaluates this at Stage 4 (Scene Commit) of the frame pipeline.

**`max_defer_frames` (AllOrDefer only).** For `AllOrDefer` groups, `SyncGroupConfig.max_defer_frames` is the maximum number of consecutive frames the compositor will defer a group before triggering a force-commit. The default is 3 frames (≈50ms at 60fps). Agents SHOULD set this to the smallest value that tolerates their expected network jitter. The value 0 means "use the runtime default" (`timing.sync_group_max_defer_frames` in `TimingConfig`). See §2.4 and §2.4.1 for the timing contract and normative force-commit semantics.

### 2.4 Timing Contract

**Atomicity window:** one display frame (16.6ms at 60Hz). All pending mutations for all members of a sync group are applied in the same stage-4 execution. The compositor does not split sync group commits across frames.

**Deadline:** a sync group's pending mutations must arrive before the frame's mutation intake cutoff (end of Stage 3, see §5). Mutations arriving after the cutoff for a frame are held for the next frame.

**AllOrDefer policy:** if the policy is `AllOrDefer` and at least one member has no pending mutation at the time of Stage 4, the entire group is deferred to the next frame. This can cascade: if the group is still incomplete at the next frame, it defers again. Maximum deferral is `max_defer_frames` (configured in `SyncGroupConfig.max_defer_frames`; default 3 frames = 50ms at 60fps; 0 means use the runtime default `timing.sync_group_max_defer_frames`). If the group is still incomplete after `max_defer_frames` consecutive deferrals, a force-commit is triggered. See §2.4.1 for the full force-commit state machine and its normative contract.

**AvailableMembers policy:** mutations from members with pending work are applied. Members without pending work remain unchanged. No deferral.

**Freeze interaction.** While freeze is active (RFC 0007 §4.3), `deferred_frames_count` is NOT incremented. The deferral counter counts actual frames where the group was incomplete and a scene commit was blocked — frames where the scene is frozen are not missed-commit frames; they are not counted at all. On unfreeze, the group resumes from its pre-freeze `deferred_frames_count`. See §5.6.1 for the full normative freeze/safe mode override interaction.

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

### 2.4.1 Force-Commit Semantics

A force-commit is triggered when an `AllOrDefer` sync group has been deferred for `max_defer_frames` consecutive frames and is still not complete. This subsection specifies the force-commit state machine normatively.

#### Trigger Condition

At Stage 4 of frame F, an `AllOrDefer` group G triggers a force-commit if and only if **all** of the following hold:

1. `G.commit_policy == AllOrDefer`
2. `G.deferred_frames_count >= max_defer_frames(G)` (i.e., the group has already been deferred the maximum allowed number of times)
3. At least one member of G has a pending mutation (the group is non-empty of work)
4. At least one member of G has **no** pending mutation (the group is still incomplete)

Where `max_defer_frames(G)` is `G.config.max_defer_frames` if non-zero, else `timing.sync_group_max_defer_frames` (the runtime-wide default, default value 3).

Note: a group that is deferred but has **no** pending mutations for any member does not trigger force-commit (no work is ready; the group is simply idle). The deferral counter is only incremented when at least one member has a pending mutation but the group is blocked waiting for other members.

#### Force-Commit Procedure

When a force-commit is triggered at Stage 4 of frame F:

1. **Partition members.** Classify each member tile of G as either:
   - **Present-and-ready**: has a pending mutation in the current frame's intake queue.
   - **Absent**: has no pending mutation.

2. **Apply present-and-ready mutations.** All pending mutations from present-and-ready members are applied atomically in frame F's Stage 4, exactly as they would be in a normal `AvailableMembers` commit. The scene is updated for these tiles.

3. **Discard absent-member deferred state.** Any pending mutations that were held over from previous deferral cycles for absent members are **discarded** (not applied). The absent members retain their last committed scene state. Absent members do NOT carry their deferred mutations into frame F+1; the deferral queue for the group is cleared entirely after the force-commit.

   **Rationale:** Carrying over absent-member mutations would create a split-brain state — the present members' mutations were just applied in frame F, so applying absent members' mutations one frame later would produce an incoherent transition. The force-commit is an escape hatch from a stuck state, not a guaranteed delivery mechanism.

4. **Reset deferral counter.** `G.deferred_frames_count` is reset to 0.

5. **Emit `SyncGroupForceCommitEvent`.** The compositor emits this telemetry event (see §7.1) at Stage 8 of frame F, containing: the group ID, the number of deferral frames used, the frame number, the IDs of present-and-ready members, and the IDs of absent members.

6. **Temporary policy transition.** The group's commit policy transitions to `AvailableMembers` **for the current frame's commit cycle only** (i.e., the force-commit is itself an `AvailableMembers`-style operation). The group's configured policy remains `AllOrDefer`; this is not a permanent policy change. At Stage 4 of frame F+1 and beyond, the group returns to evaluating with `AllOrDefer` semantics.

   Implementation note: this transition is not stored as a new enum value. It is the natural result of the force-commit procedure — present-and-ready mutations are applied and absent mutations are discarded — without changing `SyncGroup.commit_policy`. No separate "forced-commit mode" field is required; the `deferred_frames_count` reaching the threshold and then being reset to 0 is the complete state machine.

#### State Machine Diagram

```
AllOrDefer group G with max_defer_frames = N

  Stage 4, frame F:
  ┌─────────────────────────────────────────────────────────────┐
  │ Is G complete? (all members have pending mutations)         │
  └───────────────────────────────┬─────────────────────────────┘
                                  │
             ┌────────────────────┴───────────────────┐
             │ YES                                    │ NO
             ▼                                        ▼
  ┌────────────────────────┐           ┌──────────────────────────────┐
  │ Apply all mutations    │           │ G.deferred_frames_count >= N? │
  │ atomically.            │           └───────────────────┬──────────┘
  │ Reset deferred_frames  │                               │
  │ counter to 0.          │          ┌────────────────────┴──────────┐
  └────────────────────────┘          │ YES (force-commit)            │ NO
                                      ▼                               ▼
                           ┌────────────────────────┐   ┌────────────────────────┐
                           │ Apply present-and-ready│   │ Defer entire group to  │
                           │ mutations.             │   │ frame F+1.             │
                           │ Discard absent-member  │   │ Increment              │
                           │ deferred mutations.    │   │ deferred_frames_count. │
                           │ Emit ForceCommitEvent. │   └────────────────────────┘
                           │ Reset counter to 0.    │
                           └────────────────────────┘
```

#### Deferred_frames_count Increment Rule

`deferred_frames_count` is incremented at Stage 4 **only when** the group is deferred because it is incomplete (condition: at least one member has a pending mutation and at least one member is absent). It is NOT incremented when:

- The group is complete and commits normally (counter is reset to 0 instead).
- The group has no pending mutations for any member (idle; counter is unchanged).
- A force-commit fires (counter is reset to 0 instead).

This ensures the counter accurately tracks "how many consecutive frames have been lost to incomplete group state."

#### Group Returns to Normal AllOrDefer Evaluation

After a force-commit, `G.deferred_frames_count == 0` and `G.commit_policy` is still `AllOrDefer`. Frame F+1 evaluates the group from scratch:

- If all members have pending mutations → commit normally.
- If the group is again incomplete → begin a new deferral cycle (counter starts at 0, increments toward `max_defer_frames` again).

The force-commit does not "punish" the group or permanently weaken its atomicity guarantee. It is a bounded recovery mechanism, not a policy downgrade.

#### `SyncGroupForceCommitEvent` Contract

The event (see §7.1 for the proto) is emitted at telemetry Stage 8 of the frame in which the force-commit fires. Guaranteed fields:

| Field | Meaning |
|-------|---------|
| `id` | The `SyncGroupId` of the group that was force-committed. |
| `defer_frames_used` | Equal to `max_defer_frames(G)` at the time of force-commit. |
| `frame_number` | The display frame number in which the force-commit occurred. |
| `present_member_ids` | All `TileId`s (as `SceneId`s) that had pending mutations and were applied. |
| `absent_member_ids` | All `TileId`s that had no pending mutations; their deferred state was discarded. |
| `mutations_discarded` | Count of pending mutations from absent members that were discarded. |

Consumers of this event MUST NOT assume that all group members' mutations were applied. The `absent_member_ids` field identifies tiles whose state may be out of sync with the intended coordinated update.

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
| `present_at_us` | `uint64` | `MutationBatch`, `Tile`, `TextNode`, `StaticImageNode` | Do not apply this mutation/render this content before this time. If zero, apply at the earliest available frame. Part of `oneof schedule` — mutually exclusive with `after_us`, `frames_from_now`, and `next_frame`. |
| `after_us` | `uint64` | `MutationBatch`, `TimestampedPayload` | Relative scheduling sugar: apply this mutation N microseconds from the compositor's monotonic clock at Stage 3 intake. The compositor converts to an absolute `present_at_us` at intake time (see §5.3.1). Zero means "immediately" (same as `present_at_us = 0`). Part of `oneof schedule` — mutually exclusive with `present_at_us`, `frames_from_now`, and `next_frame`. |
| `frames_from_now` | `uint32` | `MutationBatch`, `TimestampedPayload` | Relative scheduling sugar: apply this mutation N display frames from the current frame at Stage 3 intake. The compositor converts to a target frame number and absolute `present_at_us` at intake time (see §5.3.1). Zero means "this frame" (earliest available frame, same as `present_at_us = 0`). Part of `oneof schedule` — mutually exclusive with `present_at_us`, `after_us`, and `next_frame`. |
| `next_frame` | `bool` | `MutationBatch`, `TimestampedPayload` | Relative scheduling sugar for `frames_from_now = 1`: apply this mutation on the next display frame after intake, guaranteed not to be the current frame. Useful when a mutation must not appear in the same frame as co-batch mutations targeting the current frame. Part of `oneof schedule` — mutually exclusive with `present_at_us`, `after_us`, and `frames_from_now`. |
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

**Relative scheduling fields (`after_us`, `frames_from_now`, `next_frame`) follow the same precedence hierarchy.** Whichever `oneof schedule` variant is set at the most specific level (node → tile → batch) governs the scheduling for that mutation.

**`oneof schedule` — Mutual Exclusivity.** The scheduling fields `present_at_us`, `after_us`, `frames_from_now`, and `next_frame` are mutually exclusive. Setting more than one in the same message is a validation error; the compositor rejects the batch with `RELATIVE_SCHEDULE_CONFLICT`. Agents must set exactly one of these fields (or none — omitting all is equivalent to `present_at_us = 0`, i.e., immediate).

**Wire-protocol only.** The relative scheduling fields (`after_us`, `frames_from_now`, `next_frame`) are wire-protocol convenience sugar. They are never stored in the scene graph, telemetry records, or internal state. The compositor converts them to absolute `present_at_us` values at Stage 3 intake before the mutation enters the pending queue. See §5.3.1 for the conversion semantics.

### 3.3 Timestamp Resolution

Timestamps are stored and compared at **microsecond (μs) resolution**. The display clock has millisecond-level granularity (16.6ms per frame at 60fps). Microsecond resolution in timestamps is used to preserve agent intent precisely, not because the compositor can act on sub-millisecond differences. In practice, timestamps are quantized to the nearest frame boundary during evaluation.

**Frame quantization:** a `present_at_us` timestamp T is "in scope" for frame F if:
```
T <= frame_F_vsync_us
```
Where `frame_F_vsync_us` is the vsync time of frame F. A timestamp must not exceed the frame's vsync time to be applied at that frame. This is a strict no-earlier-than rule: content is never presented before its declared `present_at_us`. A timestamp that falls between two vsync times is held until the next frame whose vsync is at or after T.

**Note on off-by-one frame delays.** A mutation with `present_at_us` just after vsync F will wait until vsync F+1. This is intentional: the no-earlier-than guarantee is more valuable than saving one frame of latency. Agents that want minimal latency should set `present_at_us` to the earliest acceptable frame, not a future frame.

### 3.4 Timezone Handling

All timestamps are UTC internally. The compositor never stores or computes local time. Timestamps in telemetry output are UTC ISO-8601 strings. Agent-supplied timestamps must be UTC. The protocol rejects any timestamp that could only be valid in a local timezone offset (a heuristic check — the runtime does not formally validate timezone semantics, but timestamps dramatically outside the expected range trigger clock-skew warnings).

### 3.5 Timestamp Validation

The compositor applies these validation rules to all agent-supplied timestamps:

| Condition | Action |
|-----------|--------|
| `present_at_us < session_open_at_us - 60_000_000` (> 60s in the past) | Reject: mutation too stale. Structured error `TIMESTAMP_TOO_OLD`. |
| `present_at_us > current_wallclock_us + timing.max_future_schedule_us` | Reject: timestamp too far in future. Structured error `TIMESTAMP_TOO_FUTURE`. Default `timing.max_future_schedule_us`: 300_000_000 μs (5 minutes). Unit: microseconds — the comparison is direct against `present_at_us` with no conversion needed. |
| Relative field (`after_us` or `frames_from_now`) converted to `present_at_us > current_wallclock_us + timing.max_future_schedule_us` | Reject: converted timestamp too far in future. Same structured error `TIMESTAMP_TOO_FUTURE`. The threshold is applied to the converted value, not the raw relative value. |
| More than one `oneof schedule` variant set in the same message | Reject: mutually exclusive scheduling fields. Structured error `RELATIVE_SCHEDULE_CONFLICT`. See §7.2 for the error code registration in RFC 0005. |
| `expires_at_us <= present_at_us` (expiry before or at presentation) | Reject: inconsistent timestamps. Structured error `TIMESTAMP_EXPIRY_BEFORE_PRESENT`. For relative scheduling, `present_at_us` here is the **converted** value computed at Stage 3 intake. |
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

**Estimation window.** The compositor maintains a sliding window of the last 32 agent timestamps, recording `(agent_timestamp_us, compositor_mono_us)` pairs at the time each mutation batch arrives. The clock-skew estimate is the median of `(agent_ts - compositor_ts)` over the window. Median is used (not mean) to suppress outlier spikes from individual delayed messages.

**Update frequency.** The estimate is updated on every mutation batch arrival. The estimation window is bounded; the oldest sample is evicted when the window fills.

**Clock jump detection.** If consecutive samples show a skew change greater than **`timing.clock_jump_detection_ms`** (default: 50ms) between them — indicating a sudden NTP step correction or agent clock adjustment, not gradual drift — the compositor resets the estimation window to the current single sample rather than continuing to accumulate. The estimate is re-initialized from this sample as a single-point estimate until the window refills. This prevents systematic miscorrection during the convergence period after a clock step, where a 32-sample median weighted by stale values would apply the wrong correction for up to 32 subsequent mutations.

**Safe mode interaction.** During safe mode, all agent sessions are suspended (`SessionSuspended` with reason `safe_mode`, RFC 0007 §5.1). No mutation batches arrive, so no new `(agent_ts, compositor_ts)` samples are added to the estimation window. On safe mode exit, when sessions receive `SessionResumed`, the estimation window is **reset to empty** rather than resumed with stale samples. Stale samples collected before a potentially long safe mode period (seconds to minutes) represent a different clock state and would bias the skew estimate. The window is re-initialized from fresh samples submitted by the agent after resumption. See §5.6.2 for the full normative safe mode timing interaction.

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

**Freeze and safe mode exceptions.** Two override states affect staleness indicator behavior:

- **During freeze (RFC 0007 §4.3):** The `tile_stale_threshold_ms` timer is suspended for all tiles. Since agents' mutations are queued and not applied during freeze, the absence of new mutations does not indicate agent slowness — it indicates the compositor is frozen. Staleness indicators MUST NOT be activated for tiles solely because they received no mutation during a freeze interval. The timer resumes on unfreeze; time elapsed during freeze does not count toward the staleness threshold. (Example: a tile has been idle for 4,800ms. The scene freezes for 2 seconds, then unfreezes. The tile is not stale until 200ms of additional unfreeze time passes — not 200ms after unfreeze.)
- **During safe mode (RFC 0007 §5):** Sessions are suspended (`SessionSuspended`). A session's tiles MUST NOT show staleness indicators while that session's `SessionSuspended` is active. The tile is in a known-suspended state, not a stale state. Staleness indicators are re-evaluated fresh on `SessionResumed` (timer reset to 0 at the moment of resumption).

See §5.6 for the unified override state interaction specification.

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

**Pending queue:** held pending mutations are stored in a per-agent sorted queue, ordered by `present_at_us` ascending. The queue is drained in Stage 3: mutations whose `present_at_us <= current_frame_vsync_us` are extracted and forwarded to Stage 4. This drain condition enforces the no-earlier-than guarantee directly: a mutation is released only once the frame's vsync has reached or passed the mutation's declared presentation time.

**Freeze interaction.** While freeze is active (RFC 0007 §4.3), the pending queue drain condition (`present_at_us <= current_frame_vsync_us`) is NOT evaluated. Mutations continue to accumulate in the queue (up to the class-aware overflow rules in RFC 0007 §4.3), but no mutations are extracted and no scene commits occur. On unfreeze, the drain runs immediately in the next available Stage 3: all mutations whose `present_at_us <= current_frame_vsync_us` are extracted and applied in the first post-unfreeze frame. Mutations whose `present_at_us` had already passed when unfreeze occurs are treated as immediately applicable — their window has elapsed, but the compositor must still apply them rather than drop them (they are not "late" in the `DELIVERY_POLICY_DROP_IF_LATE` sense; that policy applies only to `EPHEMERAL_REALTIME` class). See §5.6.1 for the full normative freeze behavior.

**Maximum pending queue depth:** 256 entries per agent. Mutations that would exceed this limit are rejected with `PENDING_QUEUE_FULL`. Agents with many deferred mutations are scheduling too aggressively; they should reduce their schedule horizon or increase their mutation rate.

**Equal `present_at_us` ordering.** When two or more mutations from the same agent have identical `present_at_us` values, they are applied in **FIFO arrival order** within the same frame. This ordering is deterministic and reproducible. Agents that intend simultaneous mutations SHOULD include all of them in a single `MutationBatch` (which is applied atomically per batch) or use distinct `present_at_us` values. Relying on FIFO ordering across separate batches is possible but fragile — network reordering can change the effective order.

**Session close flush.** The pending queue is session-scoped, not agent-scoped. When an agent session closes — gracefully (via `SessionClose`) or ungracefully (timeout, TCP reset, grace period expiry) — all entries in that session's pending queue are discarded. The compositor does not apply pending mutations from a closed session. A reconnecting agent starts with an empty queue and must retransmit any desired future-scheduled mutations under the new session. This ensures that budget accounting is clean: after session close, the session's pending queue entries no longer count against any resource budget.

### 5.3.1 Relative Scheduling Primitives

The relative scheduling fields (`after_us`, `frames_from_now`, `next_frame`) are converted to absolute `present_at_us` values by the compositor at **Stage 3 intake**, before the mutation enters the pending queue. From that point forward, the mutation is treated identically to one submitted with an explicit `present_at_us`. The relative fields are never stored, logged, or emitted in telemetry — they exist only on the wire.

#### `after_us` Conversion

When a mutation batch carries `after_us = N` (N ≥ 0):

1. At Stage 3 intake, the compositor reads its monotonic clock: `intake_mono_us = clock.monotonic_us()`.
2. Compute target monotonic time: `target_mono_us = intake_mono_us + N`.
3. Convert to wall-clock domain using the current per-session clock-skew estimate (§4.4):
   `present_at_us = (target_mono_us as i64 + skew_us) as u64`
   where `skew_us` is the current signed skew estimate (positive = agent ahead; the inversion converts from monotonic to wall time).
4. If `N == 0`, the result is equivalent to `present_at_us = current_wallclock_us`, meaning "apply at the earliest available frame."

**Precision note.** The skew estimate introduces up to `max_agent_clock_drift_ms` (default 100ms) of error in the wall-clock conversion. This is acceptable because `after_us` is intended for local agents on the same machine; clock skew for localhost agents is expected to be near zero. Remote agents with substantial skew SHOULD use `present_at_us` directly after performing `ClockSync`.

**Validation.** The converted `present_at_us` is validated under the same rules as a directly supplied `present_at_us` (§3.5). An `after_us` value exceeding `timing.max_future_schedule_us` after conversion is rejected with `TIMESTAMP_TOO_FUTURE`.

#### `frames_from_now` Conversion

When a mutation batch carries `frames_from_now = N` (N ≥ 0):

1. At Stage 3 intake, the compositor reads the current frame number: `current_frame = frame_state.current_frame_number`.
2. Compute target frame: `target_frame = current_frame + N`.
3. Estimate the vsync time for the target frame:
   `target_vsync_us = current_vsync_mono_us + (N * frame_duration_us)`
   where `frame_duration_us` is `1_000_000 / target_fps` (default: 16,667 μs at 60fps).
4. Convert to wall-clock domain: `present_at_us = target_vsync_us_as_wall`.
5. The converted `present_at_us` is then treated identically to an explicit `present_at_us` and enters the pending queue.

If `N == 0`, `target_frame = current_frame` and `present_at_us` is set to `0` (immediate: apply this frame if intake deadline permits, otherwise next frame per §5.2).

**Frame boundary note.** If Stage 3 intake occurs very close to the frame cutoff (within the last microseconds), a `frames_from_now = 0` mutation may miss the current frame's commit window and be deferred to the next frame. This is expected behavior — the no-earlier-than guarantee applies from the target frame onward; missing the current frame by one is not a violation.

**Frame rate changes.** If `target_fps` changes between the time of conversion and the target frame (e.g., dynamic refresh rate), the estimated vsync time may not align exactly with the actual vsync. The mutation is still applied at the first frame whose vsync time meets or exceeds the stored `present_at_us`. Frame rate changes do not cause mutations to be applied earlier than intended; they may cause them to be applied one frame later if the new frame duration is longer.

#### `next_frame` Conversion

`next_frame = true` is exactly equivalent to `frames_from_now = 1`. The compositor converts it identically: the mutation is scheduled to apply on the display frame immediately following the current frame at intake. An agent that sets `next_frame = true` is guaranteed that the mutation will not be applied in the same frame as mutations arriving in the current frame's Stage 3 window.

Use case: an agent submitting two related mutations — one for the current frame and one that must appear one frame later — can pack both in a single batch and use `next_frame` on the deferred mutation to prevent it from coalescing into the current frame's commit.

**`next_frame = false` is treated as "field not set"** — it does not select any scheduling variant. The `oneof schedule` semantics apply: only one variant may be active; if `next_frame = false` is the only schedule field, the mutation uses no scheduling override (equivalent to omitting the oneof, which means `present_at_us = 0`).

#### Interaction with `expires_at_us`

For relative-scheduled mutations, `expires_at_us` is validated against the **converted** `present_at_us`, not the raw relative field. The validation `expires_at_us > present_at_us` (§3.5) is applied after conversion.

#### Interaction with Sync Groups

Relative scheduling converts to absolute `present_at_us` before sync group evaluation. The sync group's commit policy evaluates against the absolute `present_at_us` of each pending mutation exactly as it would for an explicitly absolute-scheduled mutation. There is no special behavior for sync group members that used relative scheduling.

#### `after_us` and `frames_from_now` in `TimingHints` (MutationBatch)

`TimingHints` is embedded in `MutationBatch` (RFC 0005 §9) and carries batch-level scheduling. The `after_us`, `frames_from_now`, and `next_frame` fields in `TimingHints` follow the same conversion semantics as in `TimestampedPayload`. They apply as batch-level defaults, subject to the precedence rules in §3.2 (node-level overrides tile-level overrides batch-level).

### 5.4 Expiration Policy (`expires_at`)

A tile with `expires_at_us = T` is automatically removed from the scene at the first frame F where `frame_F_vsync_us >= T`.

**Check mechanism:** expiry evaluation happens at Stage 4 (Scene Commit), before applying new mutations. The compositor maintains an expiry heap (min-heap ordered by `expires_at_us`) of all tiles with active expiry. At Stage 4:

1. Pop all tiles from the heap where `expires_at_us <= current_frame_vsync_us`.
2. Apply an implicit `DeleteTile` mutation for each expired tile.
3. Then apply the frame's explicit mutation batches.

**Complexity:** O(expired_items) per frame, not O(total_items). The heap ensures only expired tiles are visited. Total tile count does not affect expiry evaluation cost.

**Expiry is non-negotiable under load.** Expiry evaluation at Stage 4 is never deferred by degradation. The compositor must evaluate the expiry heap and apply implicit `DeleteTile` mutations even during degradation level 4 (ShedTiles) or 5 (Emergency). Expiry semantics are a correctness contract, not a performance optimization. An agent that sets `expires_at_us` must be able to rely on tile removal within one frame (≤ 16.6ms at 60fps) of the expiry time, regardless of compositor load. RFC 0002's degradation machinery must not shed Stage 4 expiry work.

**Freeze interaction.** While freeze is active (RFC 0007 §4.3), expiry heap evaluation is **suspended**. Stage 4 does not run during freeze, so no expired tiles are checked or removed. When unfreeze occurs, the expiry heap is drained immediately at Stage 4 of the first post-unfreeze frame: all tiles whose `expires_at_us <= current_frame_vsync_us` are removed in that frame. This means a tile whose `expires_at_us` passed during the freeze interval is expired on the first post-unfreeze frame — not gradually over subsequent frames. The one-frame accuracy guarantee (≤ 16.6ms) applies only to unfreeze time; the viewer's decision to freeze the scene is a human override (architecture.md §Policy arbitration, priority 1) that temporarily suspends all timing evaluation. See §5.6.1 for the full normative freeze behavior.

**Safe mode interaction.** During safe mode (RFC 0007 §5), expiry evaluation continues normally. Unlike freeze, safe mode does not pause the compositor's frame loop — it suspends agent sessions. The frame pipeline still runs. Tiles with `expires_at_us` continue to be evaluated each frame and expire on schedule. This is intentional: expiry is a scene management contract set by the agent before safe mode; the compositor honors it regardless of session suspension state. See §5.6.2.

**Expiry notification:** when a tile expires, the compositor emits a `TileExpired { tile_id, expires_at_us, frame_number }` event to the owning agent's event subscription stream.

**Expiry and sync groups:** if a tile that is a member of a sync group expires, the tile is removed from the sync group first, then deleted. The sync group's commit policy evaluates against the updated member set.

### 5.5 Interaction Between `present_at` and `expires_at`

The compositor validates at mutation commit time that `expires_at_us > present_at_us` (see §3.5). If a tile's `present_at_us` is in the future and its `expires_at_us` is also in the future but before `present_at_us`, the mutation is rejected.

A tile whose `present_at_us` has not yet been reached is not rendered, but it is in the scene graph and consumes resource budget. Agents scheduling large numbers of future mutations should be aware that each pending tile counts against their resource budget from the moment it is committed, not from the moment it becomes visible.

### 5.6 Override State Interaction

RFC 0007 (System Shell) defines two human override states — **Freeze** (§4.3) and **Safe Mode** (§5) — that affect the timing model in distinct ways. This section is the normative specification for all timing-sensitive interactions with both override states. Individual sections above contain forward references to this section; this section is the authoritative source.

**Doctrinal grounding:** architecture.md §Policy arbitration establishes "Human override always wins" at priority 1. Timing evaluation is priority-2 runtime behavior. When a human override suspends or pauses timing machinery, that is correct and expected behavior — not a timer violation.

#### 5.6.1 Freeze Behavior

Freeze is entered when the viewer toggles the freeze shortcut (RFC 0007 §4.3). The compositor continues the frame loop and rendering, but scene commits are suppressed — mutations are queued but not applied.

**Timing evaluation during freeze:**

| Timing construct | Behavior during freeze |
|-----------------|----------------------|
| `present_at_us` pending queue drain | **Suspended.** Drain condition is not evaluated; no mutations are extracted from the queue. Mutations continue to accumulate (subject to RFC 0007 §4.3 class-aware overflow rules). |
| `expires_at_us` expiry heap | **Suspended.** Stage 4 does not run; no expiry heap evaluation occurs; no tiles are expired. |
| Sync group `deferred_frames_count` | **Not incremented.** Frozen frames are not "missed-commit" frames. The counter tracks actual incomplete-deferral events during live scene commits only. |
| `tile_stale_threshold_ms` timer | **Suspended** for all tiles. Elapsed freeze time does not count toward the staleness threshold. |
| Clock-skew estimation window | **Unaffected.** Agent mutation batches continue to arrive and are queued (not committed); each arrival contributes a `(agent_ts, compositor_ts)` sample to the estimation window as normal. The compositor still needs an accurate skew estimate for when unfreeze occurs. |
| Vsync jitter tracking | **Unaffected.** Vsync interrupts continue; jitter is tracked for monitoring purposes. |

**On unfreeze:**

All timing machinery resumes immediately in the first available frame after unfreeze:

1. **Pending queue flush.** The drain condition runs against all queued mutations. Mutations whose `present_at_us` has already passed (`present_at_us <= current_frame_vsync_us`) are applied in the first post-unfreeze Stage 4. The compositor applies them in `present_at_us` order (FIFO for equal values); there is no special "freeze catch-up" ordering. Ephemeral realtime mutations with `delivery_policy = DROP_IF_LATE` whose `present_at_us` is in the past are dropped (consistent with their drop-if-late semantics — being suspended in a frozen queue is equivalent to late arrival for these messages).

2. **Expiry flush.** All tiles whose `expires_at_us <= current_frame_vsync_us` are expired immediately in Stage 4 of the first post-unfreeze frame. This may result in several tiles expiring at once; each generates a `TileExpired` event as normal. The one-frame accuracy guarantee (≤ 16.6ms) for expiry applies from the moment of unfreeze, not from the original `expires_at_us`.

3. **Staleness timers resume.** All per-tile staleness timers resume from their pre-freeze values (not reset to zero). Time elapsed during freeze is not added to the timer.

4. **Sync group deferral counters resume.** Groups resume from their pre-freeze `deferred_frames_count`. The first post-unfreeze Stage 4 evaluates all `AllOrDefer` groups normally.

**Ephemeral realtime mutations during freeze:** The `DROP_IF_LATE` delivery policy applies at unfreeze time. An ephemeral mutation that arrived during freeze and whose `present_at_us` is now in the past is dropped on unfreeze. This is the correct behavior: ephemeral content (hover positions, interim speech tokens) frozen in the queue has already become stale. Agents should expect that ephemeral state submitted during a freeze period will not be applied.

**Freeze + safe mode interaction:** If safe mode is triggered while freeze is active, the freeze queue is discarded and freeze is cancelled as part of safe mode entry (RFC 0007 §5.6). From the timing model's perspective, the freeze queue is purged, all suspended timing constructs are abandoned, and safe mode behavior (§5.6.2) applies. On safe mode exit, all timing machinery starts fresh (empty pending queues, reset staleness timers, reset estimation windows).

#### 5.6.2 Safe Mode Behavior

Safe mode is entered when the viewer triggers the emergency stop (RFC 0007 §5). The compositor's frame loop continues; the scene graph remains intact. All agent gRPC sessions receive `SessionSuspended` with reason `safe_mode`. All new mutation batches are rejected with `SAFE_MODE_ACTIVE` until safe mode exits.

**Timing evaluation during safe mode:**

| Timing construct | Behavior during safe mode |
|-----------------|--------------------------|
| `present_at_us` pending queue drain | **Runs normally.** The frame pipeline runs; Stage 3 evaluates the pending queue each frame. Mutations from the pre-safe-mode queue (submitted before `SessionSuspended`) continue to drain and are applied when their `present_at_us` is reached. New mutations cannot be submitted (rejected with `SAFE_MODE_ACTIVE`). |
| `expires_at_us` expiry heap | **Runs normally.** Expiry evaluation is unaffected by safe mode. Tiles expire on schedule. `TileExpired` events are emitted but are not delivered to suspended sessions. |
| Sync group `deferred_frames_count` | **Increments normally** (for groups with pending mutations from the pre-safe-mode queue). However, since new mutations cannot arrive, groups that were mid-deferral when safe mode began will eventually force-commit (once `deferred_frames_count` reaches `max_defer_frames`). This is correct behavior — the force-commit escape hatch applies regardless of why mutations stopped arriving. |
| `tile_stale_threshold_ms` timer | **Suppressed** for sessions whose `SessionSuspended` is active. A tile is not "stale" because its agent is in a known-suspended state. The staleness indicator MUST NOT be shown for tiles owned by suspended sessions. |
| Clock-skew estimation window (per session) | **Frozen.** No new mutation batches arrive for suspended sessions; the window does not update. On `SessionResumed`, the per-session estimation window is **reset to empty**. Stale samples from before safe mode may represent a different clock state (especially after a long safe mode) and must not bias post-resumption corrections. The window re-initializes from fresh samples after resumption. |
| Vsync jitter tracking | **Unaffected.** The frame loop continues. |

**On safe mode exit (`SessionResumed`):**

1. Each session's clock-skew estimation window is reset to empty.
2. Each session's staleness timer is reset to 0 (the resumed session is not immediately stale).
3. All other timing machinery resumes normally. Pending queue draining, expiry heap evaluation, and sync group evaluation were not paused and need no special catch-up.
4. Any mutations that were queued before `SessionSuspended` and survived in the pending queue (their `present_at_us` has not yet elapsed) are still eligible for application. Mutations whose `present_at_us` passed during safe mode are applied immediately in the first post-resumption Stage 3 drain (same as the freeze flush rule for non-ephemeral mutations).

**Safe mode does not interrupt expiry.** This is a deliberate asymmetry with freeze. Freeze stops the compositor from committing any scene changes; safe mode does not. The compositor continues to run its full frame pipeline during safe mode, including expiry evaluation. An agent cannot rely on safe mode to "pause" their tiles' expiry timers — safe mode is a viewer sovereignty action, not a scene-pause. Agents that need tiles to survive long safe mode periods must either set long `expires_at_us` values or update them before safe mode entry.

#### 5.6.3 Headless Mode and Virtual Clock

In headless mode with `SimulatedClock`, the clock is advanced manually by tests. There is no real viewer, so freeze and safe mode are not triggered by human input. However, tests that model freeze/safe mode scenarios should advance the clock through override-state boundaries to verify the behaviors specified in §5.6.1 and §5.6.2.

Specifically, tests should cover:
- Freeze-then-advance-clock: set the clock to T, enter freeze, advance clock to T+100ms, unfreeze, then verify that tiles expired at T+10ms are expired in the first post-unfreeze frame and not before.
- Safe mode during pre-scheduled mutations: enter safe mode with pending `present_at_us` mutations; verify they are applied (not discarded) when their time arrives, even though the session is suspended.

See §8.2 for the full test coverage matrix.

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

import "scene.proto";  // SceneId (tze_hud.scene.v1) — RFC 0001 §7.1

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

  // Schedule oneof: exactly one of these fields may be set per message.
  // Setting more than one is a validation error (RELATIVE_SCHEDULE_CONFLICT).
  // Omitting all is equivalent to present_at_us = 0 (apply at earliest available frame).
  //
  // present_at_us: absolute UTC µs; 0 = immediate. Network clock domain (§1.1, §3.2).
  // after_us: relative µs from compositor monotonic clock at Stage 3 intake (§5.3.1).
  //   Converted to present_at_us at intake; never stored after conversion.
  // frames_from_now: relative frame count from current frame at intake (§5.3.1).
  //   Converted to present_at_us at intake; 0 = this frame (same as present_at_us=0).
  // next_frame: sugar for frames_from_now=1 (§5.3.1). False = field not set.
  oneof schedule {
    uint64 present_at_us   = 2;  // Absolute UTC µs; 0 = immediate
    uint64 after_us        = 20; // Relative: N µs from compositor monotonic clock at intake
    uint32 frames_from_now = 21; // Relative: N display frames from current frame at intake
    bool   next_frame      = 22; // Sugar: frames_from_now = 1 (next display frame)
  }

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
// Implementors: use the `SceneId` message type from scene.proto (imported above).
// The authoritative scene.proto SceneId message (RFC 0001 §7.1) is:
//   message SceneId { bytes bytes = 1; }  // 16-byte UUIDv7 little-endian

/// Configuration for a sync group.
message SyncGroupConfig {
  SceneId id             = 1; // SyncGroupId (RFC 0001 §1.1): 16-byte UUIDv7
  string  name           = 2; // Optional; max 128 UTF-8 bytes
  SyncCommitPolicy commit_policy = 3;
  uint32  max_defer_frames = 4; // Default 3; 0 = use default
  uint64  created_at_us  = 5; // Agent-supplied creation time (UTC μs); advisory — compositor may overwrite
}

enum SyncCommitPolicy {
  SYNC_COMMIT_POLICY_UNSPECIFIED     = 0;
  SYNC_COMMIT_POLICY_ALL_OR_DEFER    = 1; // Defer until all members have pending mutations
  SYNC_COMMIT_POLICY_AVAILABLE_MEMBERS = 2; // Apply available members; don't block
}

/// Mutation: create a sync group.
/// The group ID is embedded in `config.id` (SyncGroupConfig field 1).
/// RFC 0001 §7.1 scene.proto uses this same definition (canonical cross-reference).
/// Prior RFC 0001 versions had a redundant top-level `id` field; that was an early draft
/// artifact. RFC 0001 §7.1 has been updated to match this canonical form.
message CreateSyncGroupMutation {
  SyncGroupConfig config = 1;
}

/// Mutation: delete a sync group (tiles are removed from the group first).
/// Field name `sync_group_id` matches RFC 0001 §7.1 scene.proto definition (canonical).
message DeleteSyncGroupMutation {
  SceneId sync_group_id = 1; // SyncGroupId (RFC 0001 §1.1): 16-byte UUIDv7
}

/// Event: emitted when a sync group is force-committed after max deferral.
/// See §2.4.1 for the full normative contract.
///
/// Emitted at telemetry Stage 8 of the frame in which the force-commit fired.
/// Consumers MUST NOT assume all members' mutations were applied; check
/// absent_member_ids for tiles whose deferred mutations were discarded.
message SyncGroupForceCommitEvent {
  SceneId         id                   = 1; // SyncGroupId (RFC 0001 §1.1): 16-byte UUIDv7
  uint32          defer_frames_used    = 2; // Equals max_defer_frames(G) at trigger time
  uint64          frame_number         = 3; // Display frame number of the force-commit
  repeated SceneId present_member_ids  = 4; // TileIds whose pending mutations WERE applied
  repeated SceneId absent_member_ids   = 5; // TileIds with no pending mutation; deferred state discarded
  uint32          mutations_discarded  = 6; // Count of discarded pending mutations from absent members
}

/// Event: emitted when a sync group becomes orphaned due to its owner namespace's session closing.
/// See §2.3 "Owner namespace disconnect" for the full normative contract.
///
/// Emitted to all agents with active event subscriptions. The group begins a 5-second
/// grace period before destruction. All member tiles have already been released from
/// the group when this event fires.
message SyncGroupOrphanedEvent {
  SceneId id           = 1; // SyncGroupId (RFC 0001 §1.1): 16-byte UUIDv7
  uint64  frame_number = 2; // Display frame number when orphan was detected
  string  owner_namespace = 3; // The namespace whose session closed
}

// ─── Timing Hints ────────────────────────────────────────────────────────────

/// Per-message timing metadata. Embedded in MutationBatch (RFC 0005 §9 `session.proto`)
/// and any message that carries scheduling semantics.
///
/// Clock-domain convention: `_wall_us` fields use the network clock (UTC µs since Unix
/// epoch). This matches `present_at_us` and `expires_at_us` semantics in §3.2.
/// `sync_group_id` is a SceneId (RFC 0001 §1.1: 16-byte UUIDv7). An all-zero SceneId
/// means "not set / not in a sync group", consistent with RFC 0001 §10.1.
///
/// RFC 0005 §9.1 imports `TimingHints` from this file; the inline definition in RFC 0005
/// §9 must match this definition exactly. RFC 0003 is authoritative.
///
/// Schedule oneof: sets the batch-level scheduling for all mutations in a MutationBatch.
/// Exactly one variant may be set. Setting more than one is a validation error
/// (RELATIVE_SCHEDULE_CONFLICT). Subject to the precedence rules in §3.2 (node-level
/// overrides tile-level overrides batch-level).
message TimingHints {
  // Schedule oneof: batch-level presentation scheduling. Converted to present_at_us at
  // Stage 3 intake for relative variants (§5.3.1). Omitting all = present immediately.
  oneof schedule {
    uint64 present_at_wall_us = 1;  // Absolute: wall-clock UTC µs; 0 = present immediately
    uint64 after_us           = 10; // Relative: N µs from compositor monotonic clock at intake
    uint32 frames_from_now    = 11; // Relative: N display frames from current frame at intake; 0 = this frame
    bool   next_frame         = 12; // Sugar: frames_from_now = 1 (next display frame); false = not set
  }
  uint64  expires_at_wall_us = 2;  // Wall-clock UTC µs; 0 = no expiry. Domain: network clock (§1.1).
  SceneId sync_group_id      = 3;  // SyncGroupId (RFC 0001 §1.1): 16-byte UUIDv7; all-zero = not in a group.
}

// ─── Clock Sync ──────────────────────────────────────────────────────────────

/// Request from agent: ask compositor for its current clock.
message ClockSyncRequest {
  uint64 agent_timestamp_us = 1; // Agent's UTC microseconds at time of request
}

/// Response from compositor: provides clock reference for skew correction.
/// Clock-domain convention (RFC 0005 Round 6, rig-77n): `_wall_us` = UTC wall clock;
/// `_mono_us` = monotonic system clock.
message ClockSyncResponse {
  uint64 compositor_mono_us       = 1; // Compositor monotonic clock at response time (µs since arbitrary epoch)
  uint64 compositor_wall_us       = 2; // Compositor UTC wall clock at response time (µs since Unix epoch)
  int64  estimated_skew_us        = 3; // Current skew estimate: agent_ts - compositor_ts (signed; no suffix — delta, not timestamp)
  bool   skew_within_tolerance    = 4;
  string warning                  = 5; // Non-empty if skew is in warning range
}

// ─── Frame Telemetry ─────────────────────────────────────────────────────────

/// Per-frame timing data, embedded in TelemetryRecord.
// Clock-domain convention (RFC 0005 Round 6, rig-77n): `_wall_us` = UTC wall clock;
// `_mono_us` = monotonic system clock.
message FrameTimingRecord {
  uint64 frame_number             = 1;
  uint64 vsync_mono_us            = 2;  // Monotonic clock at vsync (µs since arbitrary epoch)
  uint64 vsync_wall_us            = 3;  // UTC wall clock at vsync (µs since Unix epoch)
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
  bool   frozen            = 22;          // True if the compositor was in freeze state during this frame (RFC 0007 §4.3).
                                          // When true: no pending queue drain, no expiry evaluation, no scene commit.
                                          // mutations_applied, tiles_expired, and sync_groups_deferred will be 0.
  bool   safe_mode_active  = 23;          // True if the compositor was in safe mode during this frame (RFC 0007 §5).
                                          // When true: scene pipeline ran normally (expiry, drain), but agent sessions
                                          // are suspended. Consumers should interpret this frame's data accordingly.
}

// ─── Timing Config ───────────────────────────────────────────────────────────

/// Runtime timing configuration (loaded from TOML config; documented here for completeness).
message TimingConfig {
  uint32 target_fps                    = 1;  // Default 60
  uint32 max_agent_clock_drift_ms      = 2;  // Default 100
  uint32 max_vsync_jitter_ms           = 3;  // Default 2
  uint32 max_future_schedule_us        = 4;  // Default 300,000,000 (5 minutes as microseconds).
                                             // Unit: microseconds. This field is directly compared
                                             // against present_at_us (§3.5), so no unit conversion
                                             // is needed at evaluation time.
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

**Relative scheduling error code.** The `RELATIVE_SCHEDULE_CONFLICT` structured error is emitted when a `MutationBatch` or `TimestampedPayload` sets more than one variant in `oneof schedule`. This error code must be added to `RuntimeError.ErrorCode` in RFC 0005 `session.proto`. The string form is `"RELATIVE_SCHEDULE_CONFLICT"`; the enum form should be added as the next available code after `TIMESTAMP_EXPIRY_BEFORE_PRESENT` (currently code 17). Until RFC 0005 is amended to add the enum value, implementations MUST use the string form `error_code = "RELATIVE_SCHEDULE_CONFLICT"` and set `error_code_enum = ERROR_CODE_UNKNOWN` (code 1).

The fields in `timing.proto` supplement the scene contract defined in RFC 0001. Key cross-references:

- `CreateSyncGroupMutation` and `DeleteSyncGroupMutation` are new variants in the `SceneMutation` oneof (RFC 0001 §8, field numbers **21 and 22** respectively). Field 20 is already occupied by `ClearZoneMutation` (RFC 0001 §8).
- `SyncGroupConfig` supplements `UpdateTileSyncGroupMutation` (RFC 0001 field 11): sync group creation is now an explicit, separate operation rather than an implicit side effect of assigning a tile to a group ID. `UpdateTileSyncGroupMutation` continues to handle tile membership changes; `CreateSyncGroupMutation` / `DeleteSyncGroupMutation` handle group lifecycle.
- `FrameTimingRecord` is embedded in `TelemetryRecord` (RFC 0002 §3.2, Stage 8).
- The `ClockSyncRequest`/`ClockSyncResponse` pair is a unary RPC. The **preferred implementation** adds a `ClockSync` method to the `SessionService` defined in RFC 0005 (session.proto). This keeps all agent-runtime communication on a single service endpoint. The `ClockSyncService` block in §7.1 documents the contract; RFC 0005 carries the normative `SessionService` definition. Do not create a second standalone gRPC service endpoint unless versioning requires it.

### 7.3 Wire Encoding Notes

1. All `uint64` timestamp fields use 0 to represent "not set." Zero is never a valid timestamp in this system.
2. `SyncGroupId` is a `SceneId` (RFC 0001 §1.1: 16-byte UUIDv7) in all contexts: `SyncGroupConfig.id`, `TimingHints.sync_group_id`, `DeleteSyncGroupMutation.sync_group_id`, `SyncGroupForceCommitEvent.id`, and `SyncGroupOrphanedEvent.id`. An all-zero `SceneId` means "not set / not in a sync group" (consistent with RFC 0001 §10.1). Agents must not send partially filled IDs.
3. `estimated_skew_us` in `ClockSyncResponse` is signed (`int64`) because skew can be positive (agent clock ahead) or negative (agent clock behind). A positive value means the agent's clock is ahead. It carries no `_wall_us` or `_mono_us` suffix because it is a signed delta, not an absolute timestamp (see RFC 0005 §2.4).
4. `delivery_policy` in `TimestampedPayload` is a protobuf enum (`DeliveryPolicy`). Implementations must treat unknown enum values as `DELIVERY_POLICY_DEFER`.
5. **Relative scheduling oneof encoding.** In `TimestampedPayload` and `TimingHints`, the `oneof schedule` uses standard protobuf3 oneof semantics. Only one variant may be set per message. Setting multiple variants via manual byte manipulation is undefined behavior — implementations MUST reject such messages with `RELATIVE_SCHEDULE_CONFLICT`. The `next_frame = false` case is indistinguishable from "field not set" in proto3 (false is the default for bool). Implementations MUST NOT treat `next_frame = false` as an active scheduling choice; they must treat it as "oneof not set." Only `next_frame = true` activates the `next_frame` variant.
6. **`after_us` field number alignment.** In `TimestampedPayload`, `after_us = 20` and `frames_from_now = 21` use high field numbers to avoid conflicts with any future additions to fields 10–19 of the message. These numbers match the issue specification (rig-ohm) and MUST NOT be renumbered without a wire-incompatible protocol version bump. In `TimingHints`, the corresponding fields are `after_us = 10` and `frames_from_now = 11` — lower numbers are safe because `TimingHints` has no pre-existing fields in that range.

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
- `AllOrDefer` sync group max deferral: force commit after `max_defer_frames` consecutive incomplete frames; `SyncGroupForceCommitEvent` emitted with correct `present_member_ids`, `absent_member_ids`, and `mutations_discarded`.
- `AllOrDefer` force-commit mutation disposition: present-and-ready mutations applied; absent-member deferred mutations discarded (NOT carried to frame F+1).
- `AllOrDefer` force-commit policy recovery: after force-commit, group re-evaluates with `AllOrDefer` semantics on the next frame; `deferred_frames_count` is 0.
- `AllOrDefer` `deferred_frames_count` increment rule: counter increments only when ≥1 member is pending and ≥1 member is absent; not incremented when group is idle (no pending mutations for any member).
- `AllOrDefer` custom `max_defer_frames`: a group configured with `max_defer_frames=1` force-commits after 1 deferral frame; a group with `max_defer_frames=0` uses the runtime default.
- `AvailableMembers` sync group: applies available members, ignores absent.
- Clock skew > tolerance: rejection with structured error.
- Clock skew within tolerance: correction applied transparently.
- Expiry heap: O(expired_items) behavior verified with large tile sets.
- Pending queue flush on session close: queued entries are discarded on disconnect; not applied after reconnect.
- Expiry under degradation: expiry evaluation runs even when compositor is in ShedTiles or Emergency mode.
- Clock jump detection: consecutive samples differing by > `clock_jump_detection_ms` trigger window reset; subsequent corrections use single-point estimate.
- Tile deletion mid-deferral: deleting a tile that is the sole missing member of an `AllOrDefer` group unblocks the group in the same frame.
- Owner namespace disconnect: closing the owning agent session emits `SyncGroupOrphanedEvent`, releases all member tiles from the group, and destroys the group after the 5-second grace period; member tiles are not deleted.
- Join during active deferral: a tile joining an `AllOrDefer` group with `deferred_frames_count > 0` is not evaluated as a required member until the next evaluation epoch; the joining tile does not extend the current deferral cycle.
- Equal `present_at_us` ordering: two mutations with the same `present_at_us` are applied in FIFO arrival order; if packed in a single `MutationBatch`, they are applied atomically.
- `max_future_schedule_us` rejection: a mutation with `present_at_us > current_wallclock_us + timing.max_future_schedule_us` is rejected with `TIMESTAMP_TOO_FUTURE`; the comparison uses microsecond units directly without conversion.
- `after_us` basic conversion: a mutation with `after_us = 500_000` (500ms) is converted at Stage 3 intake to `present_at_us ≈ current_wallclock_us + 500_000`; the converted mutation enters the pending queue and is applied at the correct frame.
- `after_us = 0`: treated as `present_at_us = 0`; mutation is applied at the earliest available frame (this frame if intake deadline permits, else next frame).
- `after_us` monotonic conversion accuracy: using a `SimulatedClock`, verify that `present_at_us` after conversion equals `(monotonic_us_at_intake + after_us)` mapped through the current skew estimate; the pending queue drain condition fires at the correct simulated frame.
- `frames_from_now` basic conversion: a mutation with `frames_from_now = 3` at intake during frame N is converted to `present_at_us = vsync_mono_us_at_N + 3 * frame_duration_us` (wall-clock domain); the mutation is applied at frame N+3 and not before.
- `frames_from_now = 0`: converted to `present_at_us = 0`; mutation is applied at the earliest available frame, identical to omitting the schedule field.
- `frames_from_now = 1` vs `next_frame = true`: both converted to the same `present_at_us` (vsync of frame N+1); applied in the same frame; conversion produces identical pending queue entries.
- `next_frame = false` is treated as "oneof not set": a message with only `next_frame = false` set is treated as having no schedule variant, equivalent to `present_at_us = 0` (immediate). It does NOT select the `next_frame` variant.
- Relative scheduling and `expires_at_us`: `expires_at_us` is validated against the **converted** `present_at_us`; `TIMESTAMP_EXPIRY_BEFORE_PRESENT` fires when `expires_at_us <= converted_present_at_us`, not against the raw `after_us` or `frames_from_now` value.
- `RELATIVE_SCHEDULE_CONFLICT` rejection: a mutation with two or more `oneof schedule` variants set (e.g., both `present_at_us` and `after_us`) is rejected with `RELATIVE_SCHEDULE_CONFLICT`; no partial processing occurs.
- Relative scheduling and sync groups: a sync group mutation scheduled with `after_us` is evaluated by sync group commit policy against the absolute `present_at_us` after conversion; sync group atomicity is not affected by whether scheduling was relative or absolute.
- Relative scheduling telemetry: `FrameTimingRecord` records the converted `present_at_us` value in all telemetry fields; the raw `after_us` / `frames_from_now` values do not appear in any telemetry record, scene graph state, or stored mutation.
- `after_us` `TIMESTAMP_TOO_FUTURE` rejection: `after_us` exceeding `timing.max_future_schedule_us` after conversion triggers `TIMESTAMP_TOO_FUTURE`; the threshold is applied to the converted `present_at_us`, not to the raw `after_us` value.
- `frames_from_now` frame rate change: if `target_fps` changes between conversion and target frame, the mutation is applied at the first frame whose actual vsync time meets or exceeds the stored `present_at_us`; the mutation is not applied earlier than the original intended frame.

**Override state interaction (§5.6):**

- Freeze suspends `present_at_us` queue drain: a mutation with `present_at_us` in the past queued during freeze is NOT applied until after unfreeze; on unfreeze it is applied in the first post-unfreeze Stage 3.
- Freeze suspends expiry heap: a tile whose `expires_at_us` passes during freeze is NOT expired until the first post-unfreeze Stage 4; multiple tiles expired simultaneously in the first post-unfreeze frame.
- Freeze does not increment `deferred_frames_count`: an `AllOrDefer` group held mid-deferral before freeze retains its pre-freeze counter value after unfreeze.
- Freeze suspends staleness timer: a tile that has been idle for 4,800ms (threshold: 5,000ms) before a 2-second freeze is not stale until 200ms after unfreeze (not 200ms total post-freeze elapsed time).
- Safe mode: expiry heap continues to run; tiles expire on schedule during safe mode (not blocked by session suspension).
- Safe mode: staleness indicators are suppressed for tiles owned by suspended sessions; indicator is cleared on `SessionResumed` and staleness timer resets to 0.
- Safe mode: clock-skew estimation window is reset to empty on `SessionResumed`; first post-resumption skew estimate is derived from fresh samples only.

### 8.3 Chaos Test Requirements

The timing model must survive chaos injection (see `heart-and-soul/validation.md`):

- Clock discontinuities: simulated clock jumps forward by 10s — all pending queues drain correctly, expired tiles are removed, no crashes.
- Clock jumps backward: monotonic clock cannot go backward; the simulation must never do this. Network clock going backward (agent skew) triggers skew detection.
- Vsync jitter: vsync signals arriving at 14ms, 16ms, 20ms intervals — sync groups still commit atomically, expiry still fires at the correct UTC time.
- Pending queue saturation: 256+ mutations queued — 257th rejected with `PENDING_QUEUE_FULL`, no state corruption.
- Sync group thrash: rapid join/leave/create/delete of sync groups — no dangling tile references, no leaked group objects.

---

## 9. Open Questions

1. **`present_at_us` precision floor:** §3.3 documents the frame quantization rule. Implementors MAY optionally surface a `sub_frame_precision_warning` in `ClockSyncResponse` if the agent is consistently sending timestamps with sub-frame (< 16.6ms) differences between sequential mutations. This telemetry optimization is a post-v1 candidate; v1 silently quantizes to frame boundaries as specified in §3.3.

**(Resolved — promoted to normative text in Round 4)**

2. ~~**Sync group ownership transfer**~~ — Resolved in §2.3 "Owner namespace disconnect": non-transferable; orphan on owner session close; `SyncGroupOrphanedEvent` emitted; 5-second grace period; members released. See §7.1 proto definition of `SyncGroupOrphanedEvent`.

3. ~~**`AllOrDefer` with growing member sets**~~ — Resolved in §2.3 "Joining during an active deferral cycle": new members are excluded from the current deferral epoch; they join the next evaluation epoch after the deferral cycle completes or force-commits.

4. ~~**Pending queue ordering for equal `present_at_us`**~~ — Resolved in §5.3 "Equal `present_at_us` ordering": FIFO arrival order is normative; agents should use single batches or distinct timestamps for simultaneous intent.

5. ~~**Expiry precision under load**~~ — Resolved in §5.4 (Round 2, T-R3): expiry evaluation is non-negotiable at Stage 4 even under degradation. Expiry latency is at most one frame (≤ 16.6ms at 60fps) even under load.

---

## 10. Configuration Integration (RFC 0006 Pending Amendment)

`TimingConfig` (§7.1) defines ten runtime-configurable timing parameters. These parameters are currently only specified in the protobuf definition here. RFC 0006 (Configuration) does not yet have a `[timing]` section; this is a known gap tracked for a follow-on amendment.

Until that amendment lands:

- All `TimingConfig` fields have defaults documented in §7.1 (e.g., `target_fps = 60`, `max_agent_clock_drift_ms = 100`, `sync_drift_budget_us = 500`, `max_future_schedule_us = 300_000_000`).
- Implementors SHOULD expose these as a `[timing]` TOML section following RFC 0006's config schema conventions (§1.2 of RFC 0006).
- The expected `[timing]` fields and their validation rules:

| Field | Type | Default | Validation |
|-------|------|---------|------------|
| `target_fps` | u32 | 60 | 1–240 |
| `max_agent_clock_drift_ms` | u32 | 100 | 1–10_000 |
| `max_vsync_jitter_ms` | u32 | 2 | 0–100 |
| `max_future_schedule_us` | u32 | 300_000_000 | 1_000_000–3_600_000_000 (1s–1h) |
| `sync_group_max_defer_frames` | u32 | 3 | 1–60 |
| `pending_queue_depth_per_agent` | u32 | 256 | 16–4096 |
| `sync_drift_budget_us` | u32 | 500 | 1–100_000 |
| `tile_stale_threshold_ms` | u32 | 5000 | 500–300_000 |
| `clock_jump_detection_ms` | u32 | 50 | 10–10_000 |
| `max_media_drift_ms` | u32 | 10 | (post-v1) |

This table is normative for validation purposes once the RFC 0006 amendment lands. Until then it serves as the authoritative reference for implementors building the configuration layer.
