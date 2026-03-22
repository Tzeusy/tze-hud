# RFC 0010: Scene Events and Interruptions

**Status:** Draft
**Issue:** rig-fzg
**Date:** 2026-03-22
**Authors:** tze_hud architecture team
**Depends on:** RFC 0001 (Scene Contract), RFC 0002 (Runtime Kernel), RFC 0005 (Session Protocol), RFC 0007 (System Shell), RFC 0008 (Lease Governance), RFC 0009 (Policy Arbitration)

---

## Summary

This RFC defines the complete scene event taxonomy, interruption class semantics, and attention budget mechanics for tze_hud. Event references are scattered across RFC 0001 (scene topology), RFC 0005 (subscription categories, `SceneEvent` wire type), and RFC 0007 (override events, budget warning). This RFC is the single authoritative specification for:

- The complete `SceneEvent` oneof taxonomy delivered to agents over gRPC
- Interruption classes and their behavioral contracts
- Attention budget tracking, enforcement, and agent observability
- How these three concerns compose with the RFC 0009 policy arbitration stack

Contradictions between this RFC and prior RFC text are resolved here; those documents must be updated to align.

---

## Motivation

The existing RFCs establish that agents can subscribe to `scene_topology` events and receive `SceneEvent` messages on the gRPC session stream (RFC 0005 §7.1, §3.2). RFC 0007 defines override events as runtime-internal (not agent-facing). RFC 0009 §2.4–§2.5 defines interruption policy and attention budget as Steps 4 and 5 of the arbitration stack. But no single document specifies:

1. What `SceneEvent` actually contains — its complete oneof taxonomy.
2. How interruption classes are declared, defaulted, and enforced at the wire level.
3. What attention budget signals, if any, are agent-observable.
4. Where inter-agent coordination signals fit in the event model.

Without this specification:
- RFC 0001 references `SceneEvent` as imported from `scene_service.proto` but the proto is never fully defined.
- RFC 0005 §7.1 lists subscription categories that map to event types not formally specified.
- Implementors must infer the `SceneEvent` message structure from scattered references.
- The doctrine in `presence.md` §"Inter-agent events" ("tab switched, new agent joined, agent departed, user dismissed tile, scene entering degraded mode") has no implementation home.

---

## Design Requirements Satisfied

| ID | Requirement | Source |
|----|-------------|--------|
| DR-SE1 | Complete, typed `SceneEvent` taxonomy | RFC 0005 §3.2, §7.1 |
| DR-SE2 | Inter-agent coordination signals at scene level | presence.md §"Inter-agent events" |
| DR-SE3 | Interruption class declaration at wire level | privacy.md §"Interruption classes", RFC 0009 §2.4 |
| DR-SE4 | Attention budget signals observable by agents | attention.md §"Attention Budget", RFC 0009 §2.5 |
| DR-SE5 | Override events remain internal; agents receive downstream effects | RFC 0007 §7.3 |
| DR-SE6 | `SceneEvent` delivery semantics consistent with RFC 0005 traffic class | RFC 0005 §3.2 |
| DR-SE7 | Subscription categories fully cover the event taxonomy | RFC 0005 §7.1 |

---

## 1. SceneEvent Taxonomy

### 1.1 Overview

A `SceneEvent` is a server-to-client notification delivered over the gRPC session stream (RFC 0005, field 33 in `SessionMessage`). It represents a state change in the scene that an agent subscribed to a relevant category should be aware of. Events are never generated for state changes the agent itself caused via a committed `MutationBatch` — agents learn about their own mutations via `BatchCommitted` (RFC 0001 §3.2). Events are only generated for changes caused by other agents, the runtime, or the human operator.

`SceneEvent` messages have traffic class **State-stream** (RFC 0005 §3.2): they are delivered reliably and in order, and coalesced under backpressure using a coalesce key (the event's target object ID or category). An agent that falls behind receives a merged view rather than individual event history.

### 1.2 Event Categories and Subscription Mapping

Each `SceneEvent` variant belongs to exactly one subscription category (RFC 0005 §7.1). The runtime only delivers events in categories the agent has subscribed to and has capability for.

| Category (RFC 0005 §7.1) | Events in this category | Required capability |
|--------------------------|-------------------------|---------------------|
| `scene_topology` | `TileCreated`, `TileDeleted`, `TileUpdated`, `TabCreated`, `TabDeleted`, `TabRenamed`, `TabReordered`, `ActiveTabChanged`, `ZoneOccupancyChanged`, `AgentJoined`, `AgentDeparted` | `read_scene` |
| `lease_changes` | `LeaseGranted`, `LeaseRenewed`, `LeaseRevoked`, `LeaseExpired`, `LeaseSuspended`, `LeaseResumed` | *(always subscribed; cannot opt out)* |
| `degradation_notices` | `DegradationLevelChanged` | *(always subscribed; cannot opt out)* |
| `zone_events` | `ZoneOccupancyChanged` (own zones only), `ZoneEvicted` | `zone_publish:<zone>` |
| `focus_events` | `FocusGained`, `FocusLost` | `receive_input` |
| `input_events` | Pointer, touch, and key events (RFC 0004) | `receive_input` |
| `attention_events` | `AttentionBudgetWarning`, `AttentionBudgetRestored` | `read_scene` |
| `telemetry_frames` | `TelemetryFrame` (RFC 0005 §9) | `read_telemetry` |

**Note on `ZoneOccupancyChanged` dual routing:** This event appears in both `scene_topology` (all topology subscribers see zone occupancy changes) and `zone_events` (agents with publish permission see changes to their accessible zones). An agent subscribing to both categories receives the event once, not twice — the runtime deduplicates before delivery.

**New category: `attention_events`** — This category did not exist in RFC 0005 §7.1. It is added by this RFC. RFC 0005 must be updated to add `ATTENTION_EVENTS = 8` to the `SubscriptionCategory` enum. Required capability: `read_scene`. Agents that do not subscribe receive no attention budget signals; they observe only that their visible update rate has slowed (RFC 0009 §2.5).

### 1.3 Complete SceneEvent Oneof

```protobuf
// Scene-level events delivered to agents over the gRPC session stream.
// Traffic class: State-stream (coalesced under backpressure, per RFC 0005 §3.2).
// Agents receive only events in their subscribed categories (RFC 0005 §7.1).
// Events are NOT generated for mutations the receiving agent itself committed.
message SceneEvent {
  uint64 sequence = 1;       // Scene sequence number after which this event was generated.
                             // Agents can use this to determine ordering relative to mutations.
  uint64 timestamp_wall_us = 2;  // Wall-clock UTC (µs since epoch, RFC 0003 §3.1) when event was generated.

  oneof event {
    // ── Scene topology ──────────────────────────────────────────────────────
    TileCreatedEvent    tile_created    =  3;
    TileDeletedEvent    tile_deleted    =  4;
    TileUpdatedEvent    tile_updated    =  5;
    TabCreatedEvent     tab_created     =  6;
    TabDeletedEvent     tab_deleted     =  7;
    TabRenamedEvent     tab_renamed     =  8;
    TabReorderedEvent   tab_reordered   =  9;
    ActiveTabChangedEvent active_tab_changed = 10;
    ZoneOccupancyChangedEvent zone_occupancy_changed = 11;
    AgentJoinedEvent    agent_joined    = 12;
    AgentDepartedEvent  agent_departed  = 13;

    // ── Lease changes (always delivered, uncancelable) ─────────────────────
    LeaseGrantedEvent   lease_granted   = 20;
    LeaseRenewedEvent   lease_renewed   = 21;
    LeaseRevokedEvent   lease_revoked   = 22;
    LeaseExpiredEvent   lease_expired   = 23;
    LeaseSuspendedEvent lease_suspended = 24;
    LeaseResumedEvent   lease_resumed   = 25;

    // ── Degradation (always delivered, uncancelable) ───────────────────────
    DegradationLevelChangedEvent degradation_level_changed = 30;

    // ── Zone publish feedback ──────────────────────────────────────────────
    ZoneEvictedEvent    zone_evicted    = 31;

    // ── Attention budget signals ───────────────────────────────────────────
    AttentionBudgetWarningEvent   attention_budget_warning   = 40;
    AttentionBudgetRestoredEvent  attention_budget_restored  = 41;
  }
}
```

### 1.4 Scene Topology Events

```protobuf
// ── Tile lifecycle ───────────────────────────────────────────────────────────

// A new tile was created by another agent (not the receiver).
// Topology subscribers only see tiles in tabs they have read_scene access to.
message TileCreatedEvent {
  SceneId tile_id        = 1;
  SceneId tab_id         = 2;
  string  agent_namespace = 3;  // Namespace of the creating agent.
  Rect    bounds         = 4;
  uint32  z_order        = 5;
}

// A tile was deleted (by another agent, lease expiry, or runtime eviction).
message TileDeletedEvent {
  SceneId tile_id        = 1;
  SceneId tab_id         = 2;
  TileDeletionReason reason = 3;
}

enum TileDeletionReason {
  TILE_DELETION_REASON_UNSPECIFIED  = 0;
  TILE_DELETION_REASON_AGENT        = 1;  // Owning agent issued DeleteTile.
  TILE_DELETION_REASON_LEASE_EXPIRY = 2;  // Tile's TTL expired.
  TILE_DELETION_REASON_VIEWER       = 3;  // Viewer dismissed tile (RFC 0007 §4.1).
  TILE_DELETION_REASON_BUDGET       = 4;  // Lease revoked for budget violation (RFC 0008 §3).
  TILE_DELETION_REASON_SAFE_MODE    = 5;  // Session suspended on safe mode entry (RFC 0007 §5.2).
  TILE_DELETION_REASON_SESSION_END  = 6;  // Agent session closed and grace period expired.
}

// A tile's bounds, z-order, opacity, input mode, or sync group was updated.
// Only the fields that changed are populated; absent fields = not changed.
// (Coalesced under backpressure: multiple TileUpdatedEvents for the same
// tile_id in a backpressure window are merged into a single event carrying
// the latest value of each field.)
message TileUpdatedEvent {
  SceneId      tile_id      = 1;
  SceneId      tab_id       = 2;
  Rect         bounds       = 3;   // Absent (zero rect) = not changed.
  uint32       z_order      = 4;   // 0 = not changed (z_order is never 0 in practice).
  float        opacity      = 5;   // 0.0 = not changed; use has_opacity to distinguish.
  InputMode    input_mode   = 6;   // UNSPECIFIED = not changed.
  SceneId      sync_group   = 7;   // Zero = not changed.
}

// ── Tab lifecycle ────────────────────────────────────────────────────────────

message TabCreatedEvent {
  SceneId tab_id        = 1;
  string  name          = 2;
  uint32  display_order = 3;
}

message TabDeletedEvent {
  SceneId tab_id = 1;
}

message TabRenamedEvent {
  SceneId tab_id   = 1;
  string  new_name = 2;
}

message TabReorderedEvent {
  SceneId tab_id        = 1;
  uint32  display_order = 2;  // New display_order value.
}

// The active tab changed (another tab is now in focus).
message ActiveTabChangedEvent {
  SceneId new_active_tab_id = 1;
  SceneId old_active_tab_id = 2;  // Zero = no prior active tab (startup).
}

// ── Zone occupancy ───────────────────────────────────────────────────────────

// The occupancy of a zone instance changed.
// Delivered to:
//   - scene_topology subscribers: for all zone changes in visible tabs.
//   - zone_events subscribers: for zones the agent has zone_publish access to.
//
// "Occupancy" means the set of active publications in a zone and their content,
// per presence.md §"Zone anatomy". This event does not carry the full content
// payload — agents that need to react to zone content must query via scene
// snapshot or maintain their own publish state.
message ZoneOccupancyChangedEvent {
  string  zone_name           = 1;  // Zone instance name (e.g. "notification").
  SceneId tab_id              = 2;  // Tab this zone instance belongs to.
  ZoneOccupancyChangeKind kind = 3;
  string  publisher_namespace = 4;  // Namespace of agent that caused the change.
                                    // Empty if cleared by runtime policy.
}

enum ZoneOccupancyChangeKind {
  ZONE_OCCUPANCY_CHANGE_UNSPECIFIED = 0;
  ZONE_OCCUPANCY_CHANGE_PUBLISHED   = 1;  // Content was published (new or replace).
  ZONE_OCCUPANCY_CHANGE_CLEARED     = 2;  // Content was removed (TTL expiry, ClearZone, or eviction).
  ZONE_OCCUPANCY_CHANGE_STACKED     = 3;  // New item added to a Stack-policy zone.
  ZONE_OCCUPANCY_CHANGE_MERGED      = 4;  // Key-value update on a MergeByKey zone.
}

// ── Agent presence ───────────────────────────────────────────────────────────
// These are the "coarse-grained coordination signals" named in presence.md
// §"Inter-agent events". They are not agent-to-agent messages — they are
// scene-level announcements about the presence landscape.
//
// Visibility is topology-controlled: agents without read_scene capability
// do not receive these events. Agents with read_scene but without
// topology-read capability see only public structure (namespace, presence
// level) — they do not see lease metadata (see presence.md §"Visibility").

// Another agent has established a session and is now resident in the scene.
// Delivered to scene_topology subscribers only if the joining agent's
// presence is not hidden by topology visibility policy.
message AgentJoinedEvent {
  string        agent_namespace = 1;  // Joining agent's identity.
  PresenceLevel presence_level  = 2;  // Guest / Resident / Embodied.
}

// An agent's session has ended (graceful close, grace period expiry, or eviction).
message AgentDepartedEvent {
  string            agent_namespace = 1;
  AgentDepartReason reason          = 2;
}

enum AgentDepartReason {
  AGENT_DEPART_REASON_UNSPECIFIED    = 0;
  AGENT_DEPART_REASON_GRACEFUL_CLOSE = 1;  // Agent sent SessionClose.
  AGENT_DEPART_REASON_GRACE_EXPIRED  = 2;  // Disconnected; grace period elapsed without reconnect.
  AGENT_DEPART_REASON_EVICTED        = 3;  // Runtime evicted session (budget violation, admin action).
}
```

### 1.5 Lease Change Events

These events are delivered unconditionally to all active sessions — they are not filterable. An agent's own leases are always reported to that agent.

```protobuf
// A lease was granted following a LeaseRequest.
// Correlated with the LeaseResponse ack (RFC 0005 §3.2) via lease_id.
message LeaseGrantedEvent {
  SceneId lease_id  = 1;
  string  namespace = 2;
  uint64  ttl_ms    = 3;  // Granted TTL in milliseconds (0 = indefinite).
  uint32  priority  = 4;  // Assigned lease_priority (RFC 0008 §2.1).
}

// A lease was successfully renewed. The new TTL replaces the prior TTL.
message LeaseRenewedEvent {
  SceneId lease_id      = 1;
  uint64  new_expires_at_wall_us = 2;  // UTC µs; new absolute expiry wall time.
}

// A lease was revoked (viewer dismiss, budget violation, admin action).
// All tiles associated with this lease are simultaneously removed.
// The reason is the same reason code used in TileDeletedEvent.
message LeaseRevokedEvent {
  SceneId             lease_id  = 1;
  string              namespace = 2;
  LeaseRevocationReason reason  = 3;
}

enum LeaseRevocationReason {
  LEASE_REVOCATION_REASON_UNSPECIFIED   = 0;
  LEASE_REVOCATION_REASON_VIEWER        = 1;  // Viewer dismiss action (RFC 0007 §4.1).
  LEASE_REVOCATION_REASON_BUDGET        = 2;  // Budget policy: sustained throttle or critical limit (RFC 0008 §3).
  LEASE_REVOCATION_REASON_CAPABILITY    = 3;  // Required capability was revoked mid-session.
  LEASE_REVOCATION_REASON_ADMIN         = 4;  // Runtime administrator action.
}

// A lease reached its TTL without renewal.
message LeaseExpiredEvent {
  SceneId lease_id  = 1;
  string  namespace = 2;
}

// A lease was suspended on safe mode entry (RFC 0008 §3.4).
// All associated tiles stop rendering. The lease remains valid.
// TTL clock is paused for the duration of suspension (RFC 0008 §3.6).
message LeaseSuspendedEvent {
  SceneId lease_id = 1;
}

// A previously suspended lease is active again (safe mode exit, RFC 0008 §3.4).
// Tiles resume rendering. TTL clock resumes from where it paused.
message LeaseResumedEvent {
  SceneId lease_id              = 1;
  uint64  new_expires_at_wall_us = 2;  // Recalculated expiry (original TTL minus suspension duration).
}
```

### 1.6 Degradation Level Changed Event

This event is delivered unconditionally and cannot be unsubscribed. Agents that ignore it will not know they need to back off.

```protobuf
// The runtime's degradation level changed (RFC 0002 §6).
// Agents SHOULD reduce their publish rate and mutation complexity
// when the level increases. Shedding resumes automatically when
// the level drops — no agent action is required to resume rendering.
message DegradationLevelChangedEvent {
  uint32 new_level = 1;  // 0 = Normal; 5 = Emergency (RFC 0002 §6, RFC 0009 §2.7).
  uint32 old_level = 2;
  string reason    = 3;  // Human-readable; empty in normal transitions.
}
```

### 1.7 Zone Eviction Event

Delivered when a Replace-policy zone evicts the agent's current content in favor of a higher-priority publisher.

```protobuf
// The agent's content was evicted from a Replace-policy zone by a
// higher-priority publisher (RFC 0009 §2.6, ZoneEvictionDenied complement).
// This is the evicted agent's notification — the evicting agent never
// receives this event (its publish succeeded normally).
message ZoneEvictedEvent {
  string  zone_name          = 1;  // Zone instance name.
  SceneId tab_id             = 2;
  string  evicting_namespace = 3;  // Agent that displaced the receiver.
}
```

### 1.8 Attention Budget Events

These events are delivered only to agents subscribed to the `attention_events` category (capability: `read_scene`). They are advisory: the attention budget mechanism (RFC 0009 §2.5) operates regardless of whether the agent is subscribed. The events allow cooperative agents to voluntarily reduce their interrupt rate before they are coalesced into silence.

```protobuf
// The agent's interruption rate has reached the configured warning
// threshold (default: 80% of max_interruptions_per_agent_per_minute).
// The agent SHOULD reduce its publish rate for non-Silent interruptions.
// This is the agent-facing counterpart of the budget_warning badge
// shown to the viewer (RFC 0007 §3.5).
message AttentionBudgetWarningEvent {
  uint32 current_rate_per_min  = 1;  // Current 60-second rolling rate.
  uint32 max_rate_per_min      = 2;  // Configured limit for this session.
  bool   zone_budget_also_full = 3;  // True if at least one zone's budget is also near limit.
}

// The agent's interruption rate has dropped back below the warning
// threshold. The budget has refilled sufficiently to resume normal
// operation.
message AttentionBudgetRestoredEvent {
  uint32 current_rate_per_min = 1;  // Current rate at time of restoration.
  uint32 max_rate_per_min     = 2;  // Configured limit.
}
```

---

## 2. Interruption Classes

### 2.1 Definition

An **interruption class** declares how disruptive a content update may be. It is declared by the publishing agent and enforced by the runtime at Steps 4 and 5 of the arbitration stack (RFC 0009 §2.4–§2.5).

Every mutation that produces visible output on screen carries an effective interruption class. The class is determined in priority order:

1. **Per-publish override:** The agent explicitly sets `InterruptionClass` on the zone publish or tile creation.
2. **Zone default:** The zone's `default_interruption_class` from its `ZoneDefinitionProto` (RFC 0001 §7.1).
3. **System default:** `Normal` if no zone default is set.

The runtime always applies the **more restrictive** of the agent-declared class and the zone's **ceiling class** (the maximum class the zone permits for agent-supplied content, set in zone configuration). An agent cannot escalate interruption class beyond the zone's ceiling.

### 2.2 Class Definitions

| Class | Value | Behavioral Contract |
|-------|-------|---------------------|
| `Silent` | 0 | Updates existing content with no visual change to layout or attention state. Clock ticks, dashboard value refreshes, ambient slow-rotation. **Never counted against attention budget. Always passes quiet hours.** |
| `Gentle` | 1 | May produce a subtle visual indicator (badge, border glow) but does not reflow the screen, open new tiles, or produce sound. Blocked during quiet hours if `pass_through_class < Gentle`. |
| `Normal` | 2 | May create new tiles, show overlays, or trigger scene transitions. Standard agent activity level. Blocked during quiet hours if `pass_through_class < Normal`. |
| `Urgent` | 3 | May override quiet hours (configurable), grab viewport focus, play a sound, or expand to a larger screen area. Implies: the viewer will regret ignoring this. **Must be earned; see §2.3.** |
| `Critical` | 4 | Overrides everything. Bypasses quiet hours unconditionally. Bypasses attention budget unconditionally. Used for: fire alarm, security breach, system failure, smoke detector. **Never queued. Never coalesced.** |

**Urgency inflation is a protocol violation.** RFC 0009 §2.4 establishes that urgency must be earned. Agents that mark Normal-class content as Urgent will have their sessions flagged and eventually throttled or revoked by budget policy.

### 2.3 Earned Urgency Contract

`Urgent` is a promise to the viewer: "this requires your attention right now, and you will regret ignoring it." The runtime cannot automatically verify whether an agent is honoring this contract, but the attention budget (§3) provides a rate signal: an agent with a high `Urgent`-class publish rate is statistically likely to be inflating urgency. The runtime MAY record the per-class breakdown in telemetry and MAY apply stricter budget limits to sessions that disproportionately use `Urgent`.

**V1 scope:** The runtime tracks the per-agent urgent-class rate. When the rate exceeds a configurable threshold (`[privacy] max_urgent_per_agent_per_minute`, default: 4), the runtime logs a warning. Budget enforcement at the per-urgent-class level is a post-v1 feature.

### 2.4 Wire Declaration

```protobuf
// Interruption class declared by the agent on a zone publish or tile update.
// Used by the interruption gate (RFC 0009 §2.4) and attention budget (RFC 0009 §2.5).
enum InterruptionClass {
  INTERRUPTION_CLASS_UNSPECIFIED = 0;  // Use zone default or system default (Normal).
  INTERRUPTION_CLASS_SILENT      = 1;
  INTERRUPTION_CLASS_GENTLE      = 2;
  INTERRUPTION_CLASS_NORMAL      = 3;
  INTERRUPTION_CLASS_URGENT      = 4;
  INTERRUPTION_CLASS_CRITICAL    = 5;
}
```

**`ZonePublish` integration:** `InterruptionClass` is added as field 5 to `ZonePublish` in RFC 0005 §9. `UNSPECIFIED` (0) means "use zone default." RFC 0001's `ZoneDefinitionProto` must add `InterruptionClass default_interruption_class = 10` (the zone's default) and `InterruptionClass ceiling_interruption_class = 11` (the zone's maximum for agent-declared content). Both fields default to `UNSPECIFIED` (inheriting system default Normal and ceiling Critical respectively).

### 2.5 Quiet Hours Interaction

Quiet hours are a runtime policy configured in `[privacy.quiet_hours]` (RFC 0006 §7). The interruption gate evaluates:

```
if quiet_hours_active and class < pass_through_class:
    → Queue(QuietHours { window_end_us })
else:
    → pass to attention budget gate
```

`pass_through_class` is configurable. Default: `Urgent` (meaning only `Urgent` and `Critical` pass during quiet hours; `Normal`, `Gentle`, and `Silent`-that-is-layout-affecting are queued). `Silent` updates (no layout change) always pass regardless of this setting.

Queued mutations accumulate in a per-zone FIFO queue. On quiet hours end, the runtime dequeues and applies them in FIFO order. Queued zone publishes follow their zone's contention policy: a `LatestWins` zone receiving 10 queued publishes will only render the last one.

---

## 3. Attention Budget

### 3.1 What the Attention Budget Is

The attention budget is a runtime-enforced rate limit on non-silent interruptions. It is both a consumer protection (the viewer's finite attention cannot be saturated by a single misbehaving agent) and a quality signal (a high-interruption-rate agent is misconfigured or exploitative).

The budget is distinct from the resource budget (RFC 0008 §4), which limits compute, memory, and bandwidth. The attention budget limits *behavioral frequency* — how often an agent is allowed to demand viewer attention.

### 3.2 Budget Accounting

Two budget scopes are maintained simultaneously:

**Per-agent budget:**
- Counter: rolling count of non-silent interruptions in the last 60 seconds, per agent session.
- Configurable limit: `[privacy] max_interruptions_per_agent_per_minute` (default: 20).
- `Critical` interruptions are exempt from this count.

**Per-zone budget:**
- Counter: rolling count of non-silent interruptions in the last 60 seconds, per zone instance.
- Configurable limit: `[privacy] max_interruptions_per_zone_per_minute` (default: 10).
- Zones with `ContentionPolicy::Stack` have higher default limits (default: 30) because stacked notifications are additive; the viewer can dismiss any individual item.

A mutation exhausts the budget if *either* the per-agent or per-zone counter exceeds its limit.

**Warning threshold:** When either counter reaches 80% of its limit, the runtime emits `AttentionBudgetWarningEvent` to subscribed agents (§1.8).

### 3.3 When Budget Is Exhausted

When a mutation's interruption class would push the agent or zone over budget:

1. The mutation is not dropped. Its commit proceeds normally (zone occupancy updates, MutationBatch validation).
2. Its *visual presentation* is coalesced: it joins a per-zone coalesce queue. If another coalesce-pending mutation for the same agent+zone key is already queued, the newer replaces the older (latest-wins within the coalesce buffer).
3. The coalesced content is presented when the budget has refilled sufficiently (as time passes and the rolling window expires).
4. The agent is NOT notified of individual coalesced mutations — the agent observes only that its visible update rate has slowed.
5. If the agent is subscribed to `attention_events`, it will have already received `AttentionBudgetWarningEvent` before this point.

**`Critical` exemption:** `Critical`-class mutations are never coalesced. They are applied immediately regardless of budget state.

**`Silent` exemption:** Silent mutations carry zero interruption cost and do not decrement the budget. They are never coalesced by the attention budget (though they may be coalesced by the state-stream coalesce key mechanism under backpressure, per RFC 0005 §2.5).

### 3.4 Budget Refill

The budget refills continuously as time passes — there is no sharp reset. The rolling 60-second window means that older interruptions "age out" smoothly. An agent that publishes 20 Gentle interruptions at t=0 will regain full budget by t=60 regardless of what it does in between.

The refill is not accelerated by agent action (e.g., the agent cannot "refund" interruptions it wishes it hadn't sent).

### 3.5 Agent Visibility into Budget State

Agents can observe their attention budget state through two mechanisms:

1. **`AttentionBudgetWarningEvent`** (§1.8) — delivered when the budget reaches 80% of limit. Tells the agent its current rate and the configured limit.
2. **`AttentionBudgetRestoredEvent`** (§1.8) — delivered when the budget drops below the warning threshold after being exhausted.

Agents cannot query the exact current budget counter on demand. The warning/restored event pair is the designed observation channel. This is intentional: providing a real-time budget query API would encourage agents to probe and precisely optimize their interrupt rate, which is the opposite of the desired behavior (agents should default to the quietest class that conveys the information, not maximize to the budget ceiling).

### 3.6 Budget Configuration

All attention budget parameters are defined in `[privacy]` (RFC 0006 §7). They are hot-reloadable.

```toml
[privacy]
# Per-agent rolling interrupt budget (excludes Critical-class).
# Counted per session; each agent session has an independent counter.
max_interruptions_per_agent_per_minute = 20

# Per-zone rolling interrupt budget (excludes Critical-class).
# "notification" zone has a higher default because it uses Stack contention.
# Zone-specific overrides use zone_budget.<zone_name> notation.
max_interruptions_per_zone_per_minute = 10

# Warning threshold as a fraction of the limit. [0.0, 1.0].
# At this fraction, AttentionBudgetWarningEvent is emitted.
attention_budget_warning_threshold = 0.80

# Maximum Urgent-class publishes per agent per minute before a warning is logged.
# V1: warning only. Post-v1: can trigger budget enforcement.
max_urgent_per_agent_per_minute = 4
```

---

## 4. Protobuf Schema

### 4.1 events.proto (new file)

```protobuf
syntax = "proto3";

package tze_hud.events.v1;

import "scene.proto";   // SceneId, Rect, InputMode (tze_hud.scene.v1; RFC 0001 §7.1)

// ─── Interruption class ──────────────────────────────────────────────────────

enum InterruptionClass {
  INTERRUPTION_CLASS_UNSPECIFIED = 0;  // Inherit zone default (or Normal if no zone default).
  INTERRUPTION_CLASS_SILENT      = 1;
  INTERRUPTION_CLASS_GENTLE      = 2;
  INTERRUPTION_CLASS_NORMAL      = 3;
  INTERRUPTION_CLASS_URGENT      = 4;
  INTERRUPTION_CLASS_CRITICAL    = 5;
}

// ─── Presence level (shared enum; also defined in session.proto RFC 0005) ────
// Defined here to avoid a circular import (events.proto imported by session.proto).
// session.proto's PresenceLevel and this PresenceLevel must remain identical.
// If they diverge, this RFC's definition takes precedence.
enum PresenceLevel {
  PRESENCE_LEVEL_UNSPECIFIED = 0;
  PRESENCE_LEVEL_GUEST       = 1;
  PRESENCE_LEVEL_RESIDENT    = 2;
  PRESENCE_LEVEL_EMBODIED    = 3;  // Reserved; not active in v1 (RFC 0005 §1.2).
}

// ─── SceneEvent (main event envelope) ────────────────────────────────────────

message SceneEvent {
  uint64 sequence          = 1;
  uint64 timestamp_wall_us = 2;  // UTC µs since epoch (RFC 0003 §3.1).

  oneof event {
    // Scene topology (subscription: scene_topology)
    TileCreatedEvent          tile_created          =  3;
    TileDeletedEvent          tile_deleted          =  4;
    TileUpdatedEvent          tile_updated          =  5;
    TabCreatedEvent           tab_created           =  6;
    TabDeletedEvent           tab_deleted           =  7;
    TabRenamedEvent           tab_renamed           =  8;
    TabReorderedEvent         tab_reordered         =  9;
    ActiveTabChangedEvent     active_tab_changed    = 10;
    ZoneOccupancyChangedEvent zone_occupancy_changed = 11;
    AgentJoinedEvent          agent_joined          = 12;
    AgentDepartedEvent        agent_departed        = 13;

    // Lease changes (always delivered)
    LeaseGrantedEvent   lease_granted   = 20;
    LeaseRenewedEvent   lease_renewed   = 21;
    LeaseRevokedEvent   lease_revoked   = 22;
    LeaseExpiredEvent   lease_expired   = 23;
    LeaseSuspendedEvent lease_suspended = 24;
    LeaseResumedEvent   lease_resumed   = 25;

    // Degradation (always delivered)
    DegradationLevelChangedEvent degradation_level_changed = 30;

    // Zone publish feedback (subscription: zone_events)
    ZoneEvictedEvent zone_evicted = 31;

    // Attention budget (subscription: attention_events)
    AttentionBudgetWarningEvent  attention_budget_warning  = 40;
    AttentionBudgetRestoredEvent attention_budget_restored = 41;
  }
}

// ─── Scene topology events ───────────────────────────────────────────────────

message TileCreatedEvent {
  SceneId tile_id         = 1;
  SceneId tab_id          = 2;
  string  agent_namespace = 3;
  Rect    bounds          = 4;
  uint32  z_order         = 5;
}

enum TileDeletionReason {
  TILE_DELETION_REASON_UNSPECIFIED  = 0;
  TILE_DELETION_REASON_AGENT        = 1;
  TILE_DELETION_REASON_LEASE_EXPIRY = 2;
  TILE_DELETION_REASON_VIEWER       = 3;
  TILE_DELETION_REASON_BUDGET       = 4;
  TILE_DELETION_REASON_SAFE_MODE    = 5;
  TILE_DELETION_REASON_SESSION_END  = 6;
}

message TileDeletedEvent {
  SceneId           tile_id = 1;
  SceneId           tab_id  = 2;
  TileDeletionReason reason = 3;
}

message TileUpdatedEvent {
  SceneId   tile_id    = 1;
  SceneId   tab_id     = 2;
  Rect      bounds     = 3;
  uint32    z_order    = 4;
  float     opacity    = 5;
  InputMode input_mode = 6;
  SceneId   sync_group = 7;
}

message TabCreatedEvent {
  SceneId tab_id        = 1;
  string  name          = 2;
  uint32  display_order = 3;
}

message TabDeletedEvent {
  SceneId tab_id = 1;
}

message TabRenamedEvent {
  SceneId tab_id   = 1;
  string  new_name = 2;
}

message TabReorderedEvent {
  SceneId tab_id        = 1;
  uint32  display_order = 2;
}

message ActiveTabChangedEvent {
  SceneId new_active_tab_id = 1;
  SceneId old_active_tab_id = 2;
}

enum ZoneOccupancyChangeKind {
  ZONE_OCCUPANCY_CHANGE_UNSPECIFIED = 0;
  ZONE_OCCUPANCY_CHANGE_PUBLISHED   = 1;
  ZONE_OCCUPANCY_CHANGE_CLEARED     = 2;
  ZONE_OCCUPANCY_CHANGE_STACKED     = 3;
  ZONE_OCCUPANCY_CHANGE_MERGED      = 4;
}

message ZoneOccupancyChangedEvent {
  string                  zone_name           = 1;
  SceneId                 tab_id              = 2;
  ZoneOccupancyChangeKind kind                = 3;
  string                  publisher_namespace = 4;
}

message AgentJoinedEvent {
  string        agent_namespace = 1;
  PresenceLevel presence_level  = 2;
}

enum AgentDepartReason {
  AGENT_DEPART_REASON_UNSPECIFIED    = 0;
  AGENT_DEPART_REASON_GRACEFUL_CLOSE = 1;
  AGENT_DEPART_REASON_GRACE_EXPIRED  = 2;
  AGENT_DEPART_REASON_EVICTED        = 3;
}

message AgentDepartedEvent {
  string           agent_namespace = 1;
  AgentDepartReason reason         = 2;
}

// ─── Lease change events ─────────────────────────────────────────────────────

message LeaseGrantedEvent {
  SceneId lease_id  = 1;
  string  namespace = 2;
  uint64  ttl_ms    = 3;
  uint32  priority  = 4;
}

message LeaseRenewedEvent {
  SceneId lease_id               = 1;
  uint64  new_expires_at_wall_us = 2;
}

enum LeaseRevocationReason {
  LEASE_REVOCATION_REASON_UNSPECIFIED = 0;
  LEASE_REVOCATION_REASON_VIEWER      = 1;
  LEASE_REVOCATION_REASON_BUDGET      = 2;
  LEASE_REVOCATION_REASON_CAPABILITY  = 3;
  LEASE_REVOCATION_REASON_ADMIN       = 4;
}

message LeaseRevokedEvent {
  SceneId               lease_id  = 1;
  string                namespace = 2;
  LeaseRevocationReason reason    = 3;
}

message LeaseExpiredEvent {
  SceneId lease_id  = 1;
  string  namespace = 2;
}

message LeaseSuspendedEvent {
  SceneId lease_id = 1;
}

message LeaseResumedEvent {
  SceneId lease_id               = 1;
  uint64  new_expires_at_wall_us = 2;
}

// ─── Degradation ─────────────────────────────────────────────────────────────

message DegradationLevelChangedEvent {
  uint32 new_level = 1;
  uint32 old_level = 2;
  string reason    = 3;
}

// ─── Zone publish feedback ───────────────────────────────────────────────────

message ZoneEvictedEvent {
  string  zone_name          = 1;
  SceneId tab_id             = 2;
  string  evicting_namespace = 3;
}

// ─── Attention budget signals ─────────────────────────────────────────────────

message AttentionBudgetWarningEvent {
  uint32 current_rate_per_min  = 1;
  uint32 max_rate_per_min      = 2;
  bool   zone_budget_also_full = 3;
}

message AttentionBudgetRestoredEvent {
  uint32 current_rate_per_min = 1;
  uint32 max_rate_per_min     = 2;
}
```

### 4.2 Required Changes to Existing Protos

The following amendments are normative. Existing RFCs must be updated to align.

**RFC 0001 `scene_service.proto`** — Add to `ZoneDefinitionProto`:
```protobuf
import "events.proto";  // InterruptionClass

// In message ZoneDefinitionProto:
InterruptionClass default_interruption_class = 10;  // UNSPECIFIED = Normal
InterruptionClass ceiling_interruption_class = 11;  // UNSPECIFIED = Critical (no ceiling)
```

**RFC 0005 `session.proto`** — Add field 5 to `ZonePublish`:
```protobuf
import "events.proto";  // InterruptionClass

// In message ZonePublish:
InterruptionClass interruption_class = 5;  // UNSPECIFIED = use zone default
```

**RFC 0005 `session.proto`** — Add `ATTENTION_EVENTS` to `SubscriptionCategory` enum:
```protobuf
ATTENTION_EVENTS = 8;  // AttentionBudgetWarning, AttentionBudgetRestored
```

**RFC 0005 `session.proto`** — Update the `SceneEvent` import comment:
```protobuf
// SceneEvent is now defined in events.proto (RFC 0010 §4.1), not in scene_service.proto.
// scene_service.proto no longer defines SceneEvent. The import is updated accordingly.
import "events.proto";
```

---

## 5. Delivery Semantics

### 5.1 Coalesce Keys

`SceneEvent` messages are delivered with traffic class **State-stream** (RFC 0005 §3.2). Under backpressure, the runtime coalesces events with the same coalesce key. The coalesce key for each event type:

| Event | Coalesce key |
|-------|-------------|
| `TileCreatedEvent` | `(tile_id)` — unlikely to coalesce; creation is once |
| `TileDeletedEvent` | `(tile_id)` |
| `TileUpdatedEvent` | `(tile_id)` — multiple updates coalesce into latest state |
| `TabCreatedEvent` | `(tab_id)` |
| `TabDeletedEvent` | `(tab_id)` |
| `TabRenamedEvent` | `(tab_id)` |
| `TabReorderedEvent` | `(tab_id)` |
| `ActiveTabChangedEvent` | singleton — only one active tab; latest wins |
| `ZoneOccupancyChangedEvent` | `(zone_name, tab_id)` |
| `AgentJoinedEvent` | `(agent_namespace)` |
| `AgentDepartedEvent` | `(agent_namespace)` |
| `LeaseGrantedEvent` | `(lease_id)` |
| `LeaseRenewedEvent` | `(lease_id)` — latest renewal wins |
| `LeaseRevokedEvent` | `(lease_id)` |
| `LeaseExpiredEvent` | `(lease_id)` |
| `LeaseSuspendedEvent` | `(lease_id)` |
| `LeaseResumedEvent` | `(lease_id)` |
| `DegradationLevelChangedEvent` | singleton — latest level wins |
| `ZoneEvictedEvent` | `(zone_name, tab_id)` |
| `AttentionBudgetWarningEvent` | singleton per session |
| `AttentionBudgetRestoredEvent` | singleton per session |

**Lease and degradation events are never dropped**, even under extreme backpressure, because agents depend on them for correct behavior. The runtime uses HTTP/2 backpressure for these rather than coalescing.

### 5.2 Ordering Guarantees

- Events within a session are delivered in generation order (the order in which the runtime generated them, indexed by `SceneEvent.sequence`).
- An agent that misses events due to disconnection receives a full `SceneSnapshot` on reconnect (RFC 0005 §1.3, §6.5) and does not receive a replay of missed events.
- The `sequence` field in `SceneEvent` allows agents to detect gaps if they track sequences. A gap indicates that a reconnect+snapshot cycle is needed.

### 5.3 Self-Event Suppression

The runtime MUST NOT deliver a `SceneEvent` to the agent whose `MutationBatch` caused the state change. The agent already has ground truth from its `BatchCommitted` response. Receiving redundant events would force agents to implement de-duplication logic and waste bandwidth.

**Exception:** Lease and degradation events are always delivered even if the agent's own action triggered them. A lease revocation caused by the agent's own budget violation should still be delivered to the agent so it knows its lease is gone.

---

## 6. Relationship to Other RFCs

### 6.1 RFC 0001 — Scene Contract

`SceneEvent` is imported from `events.proto` (this RFC). RFC 0001's `scene_service.proto` previously implied ownership of `SceneEvent` by referencing it as an export. That ownership moves to this RFC. RFC 0001 §7.2 must be updated to note that `SceneEvent` is defined in RFC 0010.

The event taxonomy in §1.4 mirrors the mutation taxonomy in RFC 0001 §2 and §7.1. Each mutation type that changes observable scene state has a corresponding event type. This symmetry is intentional: an agent constructing a scene state mirror from events can verify completeness by checking that every `SceneMutation` variant maps to an event.

### 6.2 RFC 0005 — Session Protocol

RFC 0005 §7.1 defines subscription categories. This RFC adds one new category (`attention_events`, §1.2) and fully specifies the event types behind existing categories. RFC 0005 §9 must add `ATTENTION_EVENTS = 8` to `SubscriptionCategory`. The `ZonePublish` message (RFC 0005 §9) must add `interruption_class` as field 5 (§4.2).

`SceneEvent` (field 33 in `SessionMessage`) is now typed as `tze_hud.events.v1.SceneEvent` rather than an import from `scene_service.proto`. The field number and traffic class are unchanged.

### 6.3 RFC 0007 — System Shell

RFC 0007 §7.3 defines `OverrideEvent` as internal — not agent-accessible. This RFC preserves that constraint. The downstream effects of override events become visible to agents via:
- `TileDeletedEvent` with reason `TILE_DELETION_REASON_VIEWER` (for tile dismiss).
- `LeaseRevokedEvent` with reason `LEASE_REVOCATION_REASON_VIEWER` (for session-level dismiss).
- `LeaseSuspendedEvent` / `LeaseResumedEvent` (for safe mode entry/exit).

Agents learn what happened to their content; they do not learn the raw override command.

The `budget_warning` badge (RFC 0007 §3.5) shown to the viewer is the viewer-facing signal. `AttentionBudgetWarningEvent` (§1.8) is the agent-facing counterpart. They are triggered by the same threshold but are entirely separate delivery paths.

### 6.4 RFC 0008 — Lease Governance

RFC 0008 §3 defines the lease state machine. This RFC's lease events (§1.5) are the wire representation of state machine transitions:

| RFC 0008 Transition | This RFC Event |
|--------------------|---------------|
| `REQUESTED → ACTIVE` | `LeaseGrantedEvent` |
| TTL renewal | `LeaseRenewedEvent` |
| `ACTIVE → REVOKED (viewer_dismissed)` | `LeaseRevokedEvent(VIEWER)` |
| `ACTIVE → REVOKED (budget_policy)` | `LeaseRevokedEvent(BUDGET)` |
| `ACTIVE → SUSPENDED (safe_mode)` | `LeaseSuspendedEvent` |
| `SUSPENDED → ACTIVE (safe_mode_exit)` | `LeaseResumedEvent` |
| `ACTIVE → EXPIRED (TTL)` | `LeaseExpiredEvent` |
| `ORPHANED → ACTIVE (reconnect within grace)` | `LeaseGrantedEvent` (reclaim) |

### 6.5 RFC 0009 — Policy Arbitration

Steps 4 and 5 of the arbitration stack (RFC 0009 §2.4–§2.5) reference interruption classes and attention budget. This RFC provides their complete definitions (§2, §3). The `InterruptionClass` enum used in RFC 0009 §2.4's prose is formally defined in `events.proto` (§4.1).

RFC 0009 §2.5 states: "The agent is not notified. The runtime coalesces silently." This RFC refines that statement: agents subscribed to `attention_events` DO receive notification via `AttentionBudgetWarningEvent` before budget exhaustion. The no-notification claim applies only to individual coalesced mutations, not to the budget state transition. This is not a contradiction — it is a clarification that completes the attention governance model.

---

## 7. Subscription Category Summary (Updated)

This table supersedes RFC 0005 §7.1 for the complete category list.

| Category | Enum Value | Description | Minimum Capability | Opt-out? |
|----------|-----------|-------------|-------------------|----------|
| `scene_topology` | 1 | Tile, tab, zone, and agent presence changes | `read_scene` | Yes |
| `input_events` | 2 | Pointer, touch, key events routed to agent's tiles | `receive_input` | Yes |
| `focus_events` | 3 | Focus gained/lost on agent's tiles | `receive_input` | Yes |
| `degradation_notices` | 4 | Runtime degradation level changes | *(none required)* | **No** |
| `lease_changes` | 5 | Lease state transitions for agent's own leases | *(none required)* | **No** |
| `zone_events` | 6 | Zone occupancy changes for accessible zones; ZoneEvictedEvent | `zone_publish:<zone>` | Yes |
| `telemetry_frames` | 7 | Runtime performance telemetry samples | `read_telemetry` | Yes |
| `attention_events` | **8** | Attention budget warning/restored signals | `read_scene` | Yes |

`degradation_notices` (4) and `lease_changes` (5) are always active for all sessions and cannot be disabled (consistent with RFC 0005 §7.1).

---

## 8. Design Decisions and Rationale

### 8.1 Why `SceneEvent` is a single oneof rather than separate per-category streams

A single `SceneEvent` oneof on the session stream (RFC 0005 field 33) preserves the ordering guarantee: the agent sees all subscribed events in the global sequence order the runtime generated them. Separate streams per category would require the agent to merge streams and re-sort by sequence — complex and error-prone. The single stream with per-category filtering (at the subscription level) is simpler and preserves the ordering property.

### 8.2 Why agents do not receive self-events

Self-event suppression (§5.3) prevents a common agent implementation bug: an agent that listens to `scene_topology` changes and reacts to them would create a feedback loop if it received events caused by its own mutations. The `BatchCommitted` response provides authoritative confirmation of the agent's own mutations. Self-suppression enforces a clean write-read separation.

### 8.3 Why attention budget state is not queryable on demand

See §3.5. Providing a real-time budget query would encourage agents to "budget-game" — publishing as close to the ceiling as possible continuously. The warning/restored event pair rewards cooperative agents (those that voluntarily reduce rate before exhaustion) without giving exploitative agents precise information for gaming the limit.

### 8.4 Why inter-agent events show namespace but not lease details

`AgentJoinedEvent` and `AgentDepartedEvent` carry `agent_namespace` and `presence_level` only. They do not carry lease IDs, capability scopes, or tile IDs. This is consistent with presence.md §"Visibility": by default, agents see only the public structure of the scene. Topology-level detail (who holds what lease, where their tiles are) requires the `read_scene` capability and is provided via snapshot queries, not via join/depart event payloads.

### 8.5 Why `ZoneOccupancyChangedEvent` does not carry content

Zone content payloads can be large (text, image references) and may have complex privacy classification semantics. Delivering content in the event would require the runtime to re-evaluate the recipient's privacy access for each fan-out, increasing complexity and latency. Instead, the event signals that a zone's state has changed; agents that need the content query via snapshot. This is consistent with the state-stream coalesce model: a fast zone (many publishes per second) would otherwise flood the event bus.

---

## 9. Open Questions

1. **`AgentJoinedEvent` topology gating in v1:** In v1, topology visibility policy is binary (the `read_scene` capability gates `scene_topology` subscription). The full topology-read capability distinction (presence.md §"Visibility") is deferred to post-v1. In v1, any agent with `read_scene` receives join/depart events for all other agents visible in the scene. This is a known simplification.

2. **Zone-level attention budget per zone type vs. per instance:** The current design tracks attention budget per zone instance (e.g., per "notification" zone in the "Morning" tab). A deployment with many tabs and notification zones would maintain many independent counters. An alternative is to track per zone type (all "notification" zones share one counter). Decision deferred to implementation; the config key (`max_interruptions_per_zone_per_minute`) applies to the instance-level interpretation.

3. **`TileUpdatedEvent` field presence signaling:** `TileUpdatedEvent` uses zero-values to mean "not changed" (consistent with RFC 0001 `TilePatch`). This is ambiguous for `opacity`: 0.0 is both "not changed" and a valid value (fully transparent). The implementation must use a `has_opacity` wrapper field (proto3 optional) to distinguish these cases. RFC 0001's `TilePatch` has the same issue (noted in §7.1 comments). Both must be resolved together.

4. **Post-v1: agent-to-agent signaling plane:** presence.md §"Inter-agent events" states "Direct agent-to-agent communication is out of scope for the presence engine." This RFC provides the scene-level coordination signals (join/depart, zone occupancy, tab switch). Post-v1, if agents need richer coordination (e.g., explicit handoff protocols), this would be implemented as a separate signaling plane outside the scene event bus.

---

## 10. Related RFCs

| RFC | Relationship |
|-----|-------------|
| RFC 0001 (Scene Contract) | Defines scene objects referenced in events. `SceneEvent` ownership moves from `scene_service.proto` to `events.proto` (this RFC). |
| RFC 0002 (Runtime Kernel) | Degradation levels referenced in `DegradationLevelChangedEvent` are defined in RFC 0002 §6. |
| RFC 0003 (Timing Model) | `timestamp_wall_us` in `SceneEvent` follows RFC 0003 §3.1 UTC µs convention. |
| RFC 0004 (Input Model) | `input_events` and `focus_events` categories carry RFC 0004 event types. Those types are defined in RFC 0004, not this RFC. |
| RFC 0005 (Session Protocol) | Session stream delivery, subscription categories, and `ZonePublish` are defined in RFC 0005. This RFC adds one category and one field. |
| RFC 0006 (Configuration) | Attention budget configuration lives in `[privacy]` (RFC 0006 §7). Hot-reload semantics are RFC 0006 §9. |
| RFC 0007 (System Shell) | Override events are internal (RFC 0007 §7.3). Downstream effects surface as lease and tile events here. Budget warning badge (RFC 0007 §3.5) and `AttentionBudgetWarningEvent` share the same threshold. |
| RFC 0008 (Lease Governance) | Lease state machine transitions map to lease events in §1.5 of this RFC. |
| RFC 0009 (Policy Arbitration) | Interruption gate (Step 4) and attention budget gate (Step 5) are specified in RFC 0009. This RFC provides the formal definitions of `InterruptionClass` and attention budget semantics they reference. |
