# RFC 0010: Scene Events and Interruptions

**Status:** Draft
**Issue:** rig-fzg
**Date:** 2026-03-23
**Authors:** tze_hud architecture team
**Depends on:** RFC 0001 (Scene Contract), RFC 0002 (Runtime Kernel), RFC 0004 (Input Model), RFC 0005 (Session Protocol), RFC 0006 (Configuration), RFC 0007 (System Shell), RFC 0008 (Lease Governance), RFC 0009 (Policy Arbitration)

---

## Summary

This RFC defines the complete event model for tze_hud: the event taxonomy, wire-level event structure, interruption classification, quiet hours enforcement, agent event emission, subscription model, event bus architecture, the `tab_switch_on_event` contract, and quantitative requirements for event processing. Event references are currently scattered across RFC 0001 (scene topology), RFC 0004 (input routing), RFC 0005 (subscription categories, `SceneEvent` wire type), RFC 0006 (`tab_switch_on_event`, `emit_scene_event` capability), RFC 0007 (override events, audit), and RFC 0009 (arbitration stack, attention level). This RFC is the single authoritative specification for:

- The three-category event taxonomy (input events, scene events, system events) with clear ownership boundaries
- The complete `SceneEvent` protobuf structure delivered to agents over gRPC
- Interruption classes and their behavioral contracts
- Quiet hours enforcement semantics
- Agent event emission protocol and rate limiting
- Event subscription model with wildcard and prefix matching
- Event bus architecture and delivery pipeline
- The `tab_switch_on_event` contract linking configuration to runtime behavior
- Quantitative requirements for event processing latency
- Relationship between scene events and RFC 0007 audit events

Contradictions between this RFC and prior RFC text are resolved here; those documents must be updated to align.

---

## Motivation

The existing RFCs establish that agents can subscribe to `scene_topology` events and receive `SceneEvent` messages on the gRPC session stream (RFC 0005 7.1, 3.2). RFC 0006 5.5 defines a preliminary scene-event taxonomy with agent-emittable named events (e.g., `doorbell.ring`). RFC 0007 defines override events as runtime-internal. RFC 0009 11.5 defines interruption policy and attention budget as Level 4 of the arbitration stack. But no single document specifies:

1. What `SceneEvent` actually contains -- its complete oneof taxonomy and wire structure.
2. How the three event categories (input, scene, system) relate to each other and which RFC owns which.
3. How agents emit their own semantic events and what rate limits apply.
4. How event subscriptions work at the wire level, including wildcard and prefix matching.
5. How events flow through the classification, filtering, and delivery pipeline.
6. How `tab_switch_on_event` (RFC 0006 5.4) translates a named event into a runtime tab switch.
7. What latency budgets the event pipeline must meet.
8. How RFC 0007's audit events (safe mode, freeze, override actions) flow through the same bus.

Without this specification:
- RFC 0006 5.5's scene-event taxonomy is a placeholder that cannot be implemented.
- The `emit_scene_event` capability (RFC 0006 6.3) has no wire-level protocol.
- Implementors must infer event structure from scattered references.
- The doctrine in presence.md "Inter-agent events" ("tab switched, new agent joined, agent departed, user dismissed tile, scene entering degraded mode") has no complete implementation home.

---

## Doctrine References

This RFC draws from the following doctrine passages:

**presence.md, "Inter-agent events":**
> "Agents can subscribe to a shared event bus for coarse-grained coordination signals: 'tab switched,' 'new agent joined,' 'agent departed,' 'user dismissed tile,' 'scene entering degraded mode.' These are scene-level events, not agent-to-agent messages."

**attention.md, "Attention Budget":**
> "Every screen has finite attention capacity. Interruptions are withdrawals from that budget. A screen that interrupts constantly -- even with accurate, useful information -- becomes noise."

**attention.md, "Quiet by default":**
> "No agent should interrupt by default. Interruption is opt-in by the viewer or explicitly configured. An unconfigured agent is a silent agent."

**privacy.md, "Interruption classes":**
> "Every agent interaction that changes the screen -- new tile, overlay, notification, tab switch -- carries an interruption class."

**privacy.md, "Quiet hours":**
> "During quiet hours, only urgent and critical interruptions are allowed. Normal and gentle updates are queued and delivered when quiet hours end. Silent updates continue (they are invisible by definition)."

**architecture.md, "Message classes":**
> Four traffic classes with different delivery semantics: Transactional (reliable, ordered, acknowledged), State-stream (reliable, ordered, coalesced), Ephemeral realtime (low-latency, droppable, latest-wins), Clocked media/cues (scheduled against a media or display clock).

**privacy.md, "Viewer context":**
> "The runtime maintains a viewer context that informs what the screen shows."

---

## Design Requirements Satisfied

| ID | Requirement | Source |
|----|-------------|--------|
| DR-SE1 | Complete, typed `SceneEvent` taxonomy | RFC 0005 3.2, 7.1 |
| DR-SE2 | Inter-agent coordination signals at scene level | presence.md "Inter-agent events" |
| DR-SE3 | Interruption class declaration at wire level | privacy.md "Interruption classes", RFC 0009 11.5 |
| DR-SE4 | Attention budget signals observable by agents | attention.md "Attention Budget", RFC 0009 11.5 |
| DR-SE5 | Override events remain internal; agents receive downstream effects | RFC 0007 7.3 |
| DR-SE6 | `SceneEvent` delivery semantics consistent with RFC 0005 traffic class | RFC 0005 3.2 |
| DR-SE7 | Subscription categories fully cover the event taxonomy | RFC 0005 7.1 |
| DR-SE8 | Event classification latency < 5us per event | This RFC |
| DR-SE9 | Event delivery to subscriber < 100us from emission | This RFC |
| DR-SE10 | Agent event emission rate limited and capability-gated | RFC 0006 6.3 |
| DR-SE11 | `tab_switch_on_event` contract fully specified | RFC 0006 5.4 |

---

## 1. Event Taxonomy

### 1.1 Three Categories

tze_hud events fall into three categories with clear ownership boundaries:

```
EVENT TAXONOMY

1. INPUT EVENTS           (owned by RFC 0004)
   Pointer, keyboard, touch, gesture events.
   Routed through hit-test and focus model.
   NOT part of this RFC -- RFC 0004 is authoritative.
   This RFC defines how input events interact with the scene event bus.

2. SCENE EVENTS           (this RFC's primary scope)
   Zone publications and occupancy changes.
   Tile lifecycle (created, deleted, updated, lease changed).
   Tab switches.
   Agent-emitted semantic events (e.g., doorbell.ring, timer.expired).
   Sync group commits.
   Agent presence (joined, departed).
   Scene snapshot available.

3. SYSTEM EVENTS          (runtime-only, limited agent visibility)
   Safe mode enter/exit.
   Freeze/unfreeze (not directly visible to agents; see RFC 0007).
   Viewer context change (not visible to agents; see privacy.md).
   Degradation level change.
   Lease state changes (suspend, resume, revoke).
   Budget warnings.
   Audit trail entries.
```

**Ownership rule:** Input events are defined by RFC 0004 and delivered via the `input_events` and `focus_events` subscription categories. Scene events and system events are defined by this RFC and delivered via the other subscription categories. When an input event triggers a scene-level side effect (e.g., a viewer pressing a dismiss button generates a tile deletion), the input event goes through RFC 0004's pipeline and the resulting scene change is emitted as a scene event through this RFC's pipeline. The two pipelines are separate.

### 1.2 Event Categories and Subscription Mapping

Each `SceneEvent` variant belongs to exactly one subscription category (RFC 0005 7.1). The runtime only delivers events in categories the agent has subscribed to and has capability for.

| Category (RFC 0005 7.1) | Events in this category | Required capability |
|--------------------------|-------------------------|---------------------|
| `scene_topology` | `TileCreated`, `TileDeleted`, `TileUpdated`, `TabCreated`, `TabDeleted`, `TabRenamed`, `TabReordered`, `ActiveTabChanged`, `ZoneOccupancyChanged`, `AgentJoined`, `AgentDeparted`, `SyncGroupCommitted`, `SceneSnapshotAvailable` | `read_scene_topology` |
| `lease_changes` | `LeaseGranted`, `LeaseRenewed`, `LeaseRevoked`, `LeaseExpired`, `LeaseSuspended`, `LeaseResumed` | *(always subscribed; cannot opt out)* |
| `degradation_notices` | `DegradationLevelChanged` | *(always subscribed; cannot opt out)* |
| `zone_events` | `ZoneOccupancyChanged` (own zones only), `ZoneEvicted` | `publish_zone:<zone>` |
| `focus_events` | `FocusGained`, `FocusLost` | `access_input_events` |
| `input_events` | Pointer, touch, and key events (RFC 0004) | `access_input_events` |
| `agent_events` | Agent-emitted semantic events (e.g., `agent.doorbell.ring`) | `subscribe_scene_events` |
| `telemetry_frames` | `TelemetryFrame` (RFC 0005 9) | `read_telemetry` |
| `attention_events` | `AttentionBudgetWarning`, `AttentionBudgetRestored` | `read_scene_topology` |

**Note on `ZoneOccupancyChanged` dual routing:** This event appears in both `scene_topology` (all topology subscribers see zone occupancy changes) and `zone_events` (agents with publish permission see changes to their accessible zones). An agent subscribing to both categories receives the event once, not twice -- the runtime deduplicates before delivery.

**New categories added by this RFC:**
- `agent_events` (enum value 9) -- Agent-emitted semantic events. RFC 0005 must be updated to add `AGENT_EVENTS = 9` to the `SubscriptionCategory` enum.
- `attention_events` (enum value 8) -- This category did not exist in RFC 0005 7.1. RFC 0005 must be updated to add `ATTENTION_EVENTS = 8` to the `SubscriptionCategory` enum. Required capability: `read_scene_topology`.

---

## 2. Event Structure

### 2.1 SceneEvent Envelope

Every scene event shares a common envelope structure that carries identity, classification, timing, and source information:

```protobuf
// Scene-level events delivered to agents over the gRPC session stream.
// Traffic class: State-stream (coalesced under backpressure, per RFC 0005 3.2)
// unless otherwise noted per event type.
// Agents receive only events in their subscribed categories (RFC 0005 7.1).
// Events are NOT generated for mutations the receiving agent itself committed
// (see 5.3 Self-Event Suppression).
message SceneEvent {
  bytes  event_id           = 1;  // Unique event ID (UUID v7, 16 bytes).
  string event_type         = 2;  // Namespaced event type string.
                                  // Scene events: "zone.published", "tile.created", "tab.switched"
                                  // Agent events: "agent.<namespace>.<event_name>"
                                  // System events: "system.safe_mode_entered", "system.degradation_changed"
  InterruptionClass interruption_class = 3;  // Effective interruption class after ceiling enforcement.
  uint64 timestamp_wall_us  = 4;  // Wall-clock UTC (us since epoch, RFC 0003 3.1).
  uint64 timestamp_mono_us  = 5;  // Monotonic clock (us, RFC 0003 3.1) for ordering/latency.
  bytes  source_lease_id    = 6;  // Lease that generated this event (empty for system events).
  string source_namespace   = 7;  // Namespace of the agent that caused this event.
                                  // Empty for runtime-generated system events.
  uint64 sequence           = 8;  // Scene sequence number. Agents can detect gaps for
                                  // reconnect+snapshot decisions.

  oneof payload {
    ZoneEventPayload       zone       = 10;
    TileEventPayload       tile       = 11;
    TabEventPayload        tab        = 12;
    AgentEventPayload      agent      = 13;
    SystemEventPayload     system     = 14;
    SyncGroupEventPayload  sync_group = 15;
  }
}
```

### 2.2 Event Type Naming Convention

Event types use a dotted namespace hierarchy consistent with RFC 0006 5.5:

- **Scene events:** `<object>.<action>` -- e.g., `zone.published`, `tile.created`, `tab.switched`
- **Agent events:** `agent.<namespace>.<event_name>` -- e.g., `agent.doorbell_agent.doorbell.ring`
- **System events:** `system.<action>` -- e.g., `system.safe_mode_entered`, `system.degradation_changed`

Both segments use lowercase letters, digits, and underscores. No whitespace, no uppercase. The `system.` and `scene.` prefixes are reserved for runtime-generated events; agents cannot emit events with these prefixes (RFC 0006 5.5).

---

## 3. Interruption Classification

### 3.1 Definition

An **interruption class** declares how disruptive a content update or event may be. It is declared by the publishing agent and enforced by the runtime at Level 4 (Attention) of the arbitration stack (RFC 0009 1.1).

Every event that produces visible output on screen or demands viewer attention carries an effective interruption class. The class is determined in priority order:

1. **Per-publish override:** The agent explicitly sets `InterruptionClass` on the zone publish, tile creation, or emitted event.
2. **Zone default:** The zone's `default_interruption_class` from its `ZoneDefinitionProto` (RFC 0001 7.1).
3. **System default:** `NORMAL` if no zone default is set.

The runtime always applies the **more restrictive** of the agent-declared class and the zone's **ceiling class** (the maximum class the zone permits for agent-supplied content, set in zone configuration). An agent cannot escalate interruption class beyond the zone's ceiling.

### 3.2 Class Definitions

```protobuf
enum InterruptionClass {
  CRITICAL = 0;    // Always delivered. Security alerts, safe mode, fire alarm.
                   // Bypasses quiet hours unconditionally.
                   // Bypasses attention budget unconditionally.
                   // Never queued. Never coalesced.
  HIGH     = 1;    // Delivered unless quiet hours (configurable).
                   // May override quiet hours if pass_through_class <= HIGH.
                   // May grab viewport focus, play sounds, expand screen area.
                   // Implies: the viewer will regret ignoring this.
                   // Doorbell, urgent notifications, security camera motion.
  NORMAL   = 2;    // Standard agent activity. Filtered by attention budget.
                   // May create new tiles, show overlays, trigger transitions.
                   // Blocked during quiet hours if pass_through_class > NORMAL.
                   // Zone updates, tile changes, tab switches.
  LOW      = 3;    // Batched/deferred. Background sync, telemetry, status refreshes.
                   // May produce subtle visual indicator (badge, border glow)
                   // but does not reflow screen or produce sound.
                   // Blocked during quiet hours.
  SILENT   = 4;    // Never interrupts. Available only via explicit query.
                   // Updates existing content with no visual disruption.
                   // Clock ticks, dashboard refreshes, ambient rotation.
                   // Never counted against attention budget.
                   // Always passes quiet hours (invisible by definition).
}
```

**Mapping to privacy.md classes:** The doctrine in privacy.md names five classes (Silent, Gentle, Normal, Urgent, Critical). This RFC's enum maps to them as follows:

| privacy.md name | This RFC enum | Notes |
|-----------------|---------------|-------|
| Critical | `CRITICAL` (0) | Identical semantics |
| Urgent | `HIGH` (1) | Renamed for consistency with RFC 0009 severity levels |
| Normal | `NORMAL` (2) | Identical semantics |
| Gentle | `LOW` (3) | Renamed; "gentle" and "low" describe the same behavioral contract |
| Silent | `SILENT` (4) | Identical semantics |

The enum ordering (CRITICAL=0 as highest priority) ensures that lower numeric values represent higher urgency, which simplifies comparison logic in the arbitration pipeline.

### 3.3 Earned Urgency Contract

`HIGH` is a promise to the viewer: "this requires your attention right now, and you will regret ignoring it." The runtime cannot automatically verify whether an agent is honoring this contract, but the attention budget (6) provides a rate signal: an agent with a high `HIGH`-class publish rate is statistically likely to be inflating urgency. The runtime MAY record the per-class breakdown in telemetry and MAY apply stricter budget limits to sessions that disproportionately use `HIGH`.

> **Doctrine (attention.md, "Earned urgency"):** "Urgency is a promise: 'this requires your attention right now.' Breaking that promise -- marking non-urgent content urgent -- degrades the entire interrupt system."

**V1 scope:** The runtime tracks the per-agent HIGH-class rate. When the rate exceeds a configurable threshold (`[privacy] max_urgent_per_agent_per_minute`, default: 4), the runtime logs a warning. Budget enforcement at the per-HIGH-class level is a post-v1 feature.

### 3.4 Wire Declaration

```protobuf
// Declared in events.proto (this RFC).
// Used by the interruption gate (RFC 0009 Level 4) and attention budget.
// Lower numeric value = higher urgency.
enum InterruptionClass {
  INTERRUPTION_CLASS_CRITICAL = 0;  // Always delivered.
  INTERRUPTION_CLASS_HIGH     = 1;  // Passes quiet hours if configured.
  INTERRUPTION_CLASS_NORMAL   = 2;  // Standard. Use zone default or system default.
  INTERRUPTION_CLASS_LOW      = 3;  // Batched/deferred.
  INTERRUPTION_CLASS_SILENT   = 4;  // Never interrupts.
}
```

**`ZonePublish` integration:** `InterruptionClass` is added as field 5 to `ZonePublish` in RFC 0005 9. Value 0 (`CRITICAL`) means highest urgency. To specify "use zone default," an agent omits the field entirely (proto3 default zero would mean CRITICAL, so `InterruptionClass` uses an explicit `UNSPECIFIED` sentinel at value 5):

```protobuf
enum InterruptionClass {
  INTERRUPTION_CLASS_CRITICAL    = 0;
  INTERRUPTION_CLASS_HIGH        = 1;
  INTERRUPTION_CLASS_NORMAL      = 2;
  INTERRUPTION_CLASS_LOW         = 3;
  INTERRUPTION_CLASS_SILENT      = 4;
  INTERRUPTION_CLASS_UNSPECIFIED = 5;  // Inherit zone default (or NORMAL if no zone default).
}
```

RFC 0001's `ZoneDefinitionProto` must add `InterruptionClass default_interruption_class = 10` (the zone's default) and `InterruptionClass ceiling_interruption_class = 11` (the zone's maximum for agent-declared content). Both fields default to `UNSPECIFIED` (inheriting system default NORMAL and ceiling CRITICAL respectively).

---

## 4. Quiet Hours Enforcement

### 4.1 Configuration Source

Quiet hours are configured in RFC 0006's `[attention]` section (mapped from `[privacy.quiet_hours]`):

```toml
[privacy.quiet_hours]
enabled = true
start = "22:00"
end = "07:00"
timezone = "local"
pass_through_class = "HIGH"  # HIGH and CRITICAL pass during quiet hours
```

The `pass_through_class` field determines the minimum interruption class that is delivered during quiet hours. Classes with urgency lower than this threshold are deferred.

### 4.2 Delivery Rules During Quiet Hours

During quiet hours, the interruption gate (RFC 0009 Level 4) evaluates each event's interruption class:

| Interruption Class | Quiet Hours Behavior |
|--------------------|---------------------|
| `CRITICAL` | **Always delivered.** Quiet hours never block CRITICAL events. Security alerts, safe mode, fire alarms reach the viewer immediately. |
| `HIGH` | **Deferred to queue, delivered on quiet hours exit.** Unless `pass_through_class <= HIGH`, in which case HIGH events pass through. Default configuration passes HIGH through. |
| `NORMAL` | **Deferred to queue, delivered on quiet hours exit.** Zone updates, tile changes, and standard agent activity are queued. |
| `LOW` | **Discarded.** LOW events are too stale by quiet hours exit to be useful. Telemetry, background sync, and status refreshes generated during quiet hours are dropped, not queued. |
| `SILENT` | **Unaffected.** Silent events are invisible by definition. They continue flowing regardless of quiet hours state. |

### 4.3 Queue Semantics

Quiet hours affect **delivery**, not **generation**. Events are still created, logged, and counted for telemetry purposes. The queue holds deferred events for delivery when quiet hours end.

- Queued events accumulate in a per-zone FIFO queue.
- On quiet hours exit, the runtime dequeues and delivers them in FIFO order.
- Queued zone publishes follow their zone's contention policy: a `LatestWins` zone receiving 10 queued publishes delivers only the last one.
- The queue has a configurable maximum depth per zone (default: 100). Events exceeding the queue depth are dropped (oldest first).
- `CRITICAL` events are never queued. They bypass the queue entirely.

### 4.4 Interaction with Attention Budget

Quiet hours and attention budget are independent deferral mechanisms evaluated at the same arbitration level (RFC 0009 Level 4). When both would queue an interruption, the longer deferral wins. If quiet hours end at 07:00 and the attention budget refills in 30 seconds, the interruption is queued until 07:00 (RFC 0009 2.2, Level 4 tie-breaking).

---

## 5. Agent Event Emission

### 5.1 Emission Protocol

Agents can emit scene events by sending an `EmitSceneEvent` message on the session stream. This is the wire-level implementation of the `emit_scene_event:<name>` capability defined in RFC 0006 6.3.

```protobuf
// Client -> Server: Agent emits a named scene event.
// Requires `emit_scene_event:<name>` capability for the specific event name.
// Added to SessionMessage oneof (RFC 0005) as field 45.
message EmitSceneEvent {
  string event_name                = 1;  // Dotted event name (e.g., "doorbell.ring").
                                         // Must match the <name> in the agent's
                                         // emit_scene_event:<name> capability grant.
  InterruptionClass interruption_class = 2;  // Requested interruption class.
                                              // Runtime may downgrade (see 5.3).
                                              // UNSPECIFIED = NORMAL.
  bytes  payload                   = 3;  // Optional opaque payload (max 4KB).
                                         // Interpretation is event-name-specific.
                                         // V1: payload is opaque bytes; post-v1 may
                                         // add structured payload schemas per event.
}

// Server -> Client: Result of EmitSceneEvent.
// Correlated by sequence number.
message EmitSceneEventResult {
  bool   accepted                  = 1;  // True if the event was accepted and will be delivered.
  InterruptionClass effective_class = 2;  // The class the runtime assigned (may differ from requested).
  RuntimeError error               = 3;  // Populated if accepted = false.
}
```

### 5.2 Capability Requirement

Agent event emission requires the `emit_scene_event:<name>` capability, where `<name>` is the exact dotted event name the agent wants to emit. This is a per-event-name grant, not a blanket permission.

```toml
# RFC 0006 agent configuration example:
[agents.registered.doorbell_agent]
capabilities = [
  "emit_scene_event:doorbell.ring",   # Can fire the doorbell event
  "emit_scene_event:doorbell.motion", # Can fire the motion event
]
```

**Reserved prefixes:** Agents cannot emit events with names starting with `system.` or `scene.`. Such capability grants are rejected at config load time (`CONFIG_RESERVED_EVENT_PREFIX`, RFC 0006 5.5).

### 5.3 Namespace Prefixing and Class Enforcement

Agent events are delivered to subscribers with the full namespace prefix: `agent.<namespace>.<event_name>`. This ensures event names are globally unique and attributable.

Example: An agent with namespace `doorbell_agent` emitting event `doorbell.ring` produces an event with `event_type = "agent.doorbell_agent.doorbell.ring"`.

**Interruption class enforcement:** An agent can request any interruption class up to HIGH. The runtime enforces the following rules:

- Agents cannot request `CRITICAL`. Only the runtime can emit CRITICAL events. An agent requesting CRITICAL is downgraded to HIGH.
- The runtime may further downgrade based on the agent's attention budget state. An agent that has exhausted its budget may have HIGH requests downgraded to NORMAL.
- The effective class is reported back in `EmitSceneEventResult.effective_class`.

### 5.4 Rate Limiting

Agent event emission is rate limited to prevent event spam:

- **Default rate limit:** 10 events/second per agent session.
- **Configurable:** `[privacy] max_agent_events_per_second` (per-agent, not per-event-name).
- Events exceeding the rate limit are rejected with `RuntimeError` code `AGENT_EVENT_RATE_EXCEEDED`.
- Rate limiting is a sliding window (1-second window, max count).
- CRITICAL events (which agents cannot emit) are exempt from this limit.

---

## 6. Attention Budget

### 6.1 What the Attention Budget Is

The attention budget is a runtime-enforced rate limit on non-silent interruptions. It is both a consumer protection (the viewer's finite attention cannot be saturated by a single misbehaving agent) and a quality signal (a high-interruption-rate agent is misconfigured or exploitative).

> **Doctrine (attention.md, "Attention Budget"):** "The runtime must treat attention budget as a real constraint: Not every update needs to be an interruption. Silent and gentle updates exist for a reason."

The budget is distinct from the resource budget (RFC 0008 4), which limits compute, memory, and bandwidth. The attention budget limits *behavioral frequency* -- how often an agent is allowed to demand viewer attention.

### 6.2 Budget Accounting

Two budget scopes are maintained simultaneously:

**Per-agent budget:**
- Counter: rolling count of non-silent interruptions in the last 60 seconds, per agent session.
- Configurable limit: `[privacy] max_interruptions_per_agent_per_minute` (default: 20).
- `CRITICAL` interruptions are exempt from this count.

**Per-zone budget:**
- Counter: rolling count of non-silent interruptions in the last 60 seconds, per zone instance.
- Configurable limit: `[privacy] max_interruptions_per_zone_per_minute` (default: 10).
- Zones with `ContentionPolicy::Stack` have higher default limits (default: 30) because stacked notifications are additive.

A mutation exhausts the budget if *either* the per-agent or per-zone counter exceeds its limit.

**Warning threshold:** When either counter reaches 80% of its limit, the runtime emits `AttentionBudgetWarningEvent` to subscribed agents.

### 6.3 When Budget Is Exhausted

When a mutation's interruption class would push the agent or zone over budget:

1. The mutation is not dropped. Its commit proceeds normally (zone occupancy updates, MutationBatch validation).
2. Its *visual presentation* is coalesced: it joins a per-zone coalesce queue. If another coalesce-pending mutation for the same agent+zone key is already queued, the newer replaces the older (latest-wins within the coalesce buffer).
3. The coalesced content is presented when the budget has refilled sufficiently.
4. The agent is NOT notified of individual coalesced mutations -- the agent observes only that its visible update rate has slowed.
5. If the agent is subscribed to `attention_events`, it will have already received `AttentionBudgetWarningEvent` before this point.

**`CRITICAL` exemption:** `CRITICAL`-class mutations are never coalesced. They are applied immediately regardless of budget state.

**`SILENT` exemption:** Silent mutations carry zero interruption cost and do not decrement the budget.

### 6.4 Budget Configuration

All attention budget parameters are defined in `[privacy]` (RFC 0006 7). They are hot-reloadable.

```toml
[privacy]
max_interruptions_per_agent_per_minute = 20
max_interruptions_per_zone_per_minute = 10
attention_budget_warning_threshold = 0.80
max_urgent_per_agent_per_minute = 4
max_agent_events_per_second = 10
```

---

## 7. Event Subscription Model

### 7.1 Subscription Categories

Agents subscribe to event categories via `SubscriptionChange` (RFC 0005 7.3). The complete category list (superseding RFC 0005 7.1):

```protobuf
enum SubscriptionCategory {
  SUBSCRIPTION_CATEGORY_UNSPECIFIED = 0;
  SCENE_TOPOLOGY                    = 1;  // Tile, tab, zone, agent presence, sync group events
  INPUT_EVENTS                      = 2;  // Pointer, touch, key events (RFC 0004)
  FOCUS_EVENTS                      = 3;  // Focus gained/lost on agent's tiles
  DEGRADATION_NOTICES               = 4;  // Runtime degradation level changes (cannot opt out)
  LEASE_CHANGES                     = 5;  // Lease state transitions (cannot opt out)
  ZONE_EVENTS                       = 6;  // Zone occupancy for accessible zones; ZoneEvicted
  TELEMETRY_FRAMES                  = 7;  // Runtime performance telemetry samples
  ATTENTION_EVENTS                  = 8;  // Attention budget warning/restored signals
  AGENT_EVENTS                      = 9;  // Agent-emitted semantic events
}
```

### 7.2 Subscription Filtering

Within each subscription category, agents can apply finer-grained filters using event type prefix matching. This is specified via an extended `SubscriptionChange` message:

```protobuf
message SubscriptionChange {
  repeated SubscriptionCategory add    = 1;
  repeated SubscriptionCategory remove = 2;
  // Fine-grained event type filters within subscribed categories.
  // If empty, all events in the subscribed categories are delivered.
  // If non-empty, only events matching at least one filter are delivered.
  repeated EventFilter filters         = 3;
}

message EventFilter {
  // Event type prefix to match. Supports:
  //   "zone.*"                    -- all zone events
  //   "zone.published"            -- specific zone event
  //   "tile.*"                    -- tile lifecycle events
  //   "tab.*"                     -- tab events
  //   "agent.<namespace>.*"       -- another agent's events (requires capability)
  //   "system.lease"              -- own lease state changes
  //   "system.degradation"        -- degradation events
  string event_type_prefix = 1;
}
```

**Wildcard semantics:** The `*` character matches any suffix. Only trailing wildcards are supported (no infix wildcards). A filter of `"zone.*"` matches `zone.published`, `zone.cleared`, etc. A filter of `"agent.doorbell_agent.*"` matches all events from the doorbell agent.

**Capability enforcement:** Subscription filters are checked against the agent's capabilities. An agent cannot subscribe to `agent.<namespace>.*` for another agent's events unless it holds `subscribe_scene_events` capability. An agent without `read_scene_topology` capability cannot subscribe to `scene_topology` events regardless of filter.

### 7.3 Subscription Limits

- Maximum subscriptions per agent: **32** (total across all categories and filters).
- Exceeding this limit causes the `SubscriptionChange` to be partially rejected: the first 32 are accepted, the remainder are denied in `SubscriptionChangeResult.denied_subscriptions`.

---

## 8. Event Bus Architecture

### 8.1 Pipeline Overview

Events flow through a four-stage pipeline:

```
EMISSION  -->  CLASSIFICATION  -->  POLICY FILTERING  -->  DELIVERY

  1. Source generates event          2. InterruptionClass       3. Arbitration stack      4. Fan-out to
     (runtime, agent, system)           assigned/enforced          Level 4 (Attention)       subscribed agents
                                        (< 5us)                    evaluates quiet hours,    (< 100us per
                                                                   attention budget           subscriber)
                                                                   (< 2us)
```

### 8.2 Stage 1: Emission

Events are generated by three sources:

1. **Runtime-generated scene events:** Tile lifecycle, tab switches, zone occupancy changes, agent presence, sync group commits, scene snapshots. Generated during the compositor's mutation intake and scene commit stages (RFC 0002 Stage 3-4).

2. **Agent-emitted events:** Generated when an agent sends `EmitSceneEvent` on the session stream. The runtime validates the agent's capability, applies rate limiting, and namespace-prefixes the event before it enters the pipeline.

3. **System events:** Generated by runtime subsystems. Safe mode entry/exit (RFC 0007), degradation changes (RFC 0002 6), lease state transitions (RFC 0008), viewer context changes (privacy.md), attention budget warnings (this RFC). System events always have `source_lease_id` empty and `source_namespace` empty.

### 8.3 Stage 2: Classification

Every event is assigned an effective `InterruptionClass`:

- **Scene events** receive their class from the mutation that triggered them, inheriting the zone's default class or the agent's explicit declaration.
- **Agent events** receive their class from the `EmitSceneEvent.interruption_class` field, subject to ceiling enforcement (5.3).
- **System events** have hardcoded classes:
  - Safe mode enter/exit: `CRITICAL`
  - Degradation change: `NORMAL`
  - Lease suspend/resume/revoke: `NORMAL` (the event is informational; the action already happened)
  - Budget warning/restored: `LOW`
  - Viewer context change: `SILENT` (not delivered to agents)

**Quantitative requirement:** Classification must complete in **< 5us per event** (DR-SE8).

### 8.4 Stage 3: Policy Filtering

The classified event passes through RFC 0009 Level 4 (Attention):

1. **Quiet hours check** (< 2us): If quiet hours are active and the event's class is below `pass_through_class`, the event is queued for deferred delivery (4.3).
2. **Attention budget check**: If the event's class is not SILENT and the agent's or zone's budget is exhausted, the event's associated mutation is coalesced (6.3). The event itself is still delivered to subscribers (events are informational; coalescing affects visual presentation, not event delivery).

**Distinction:** Policy filtering affects *mutation presentation*, not *event delivery*. An event that is "queued" by quiet hours means the associated visual change is deferred, but the event message itself may still be delivered to subscribed agents (unless the subscriber is also subject to quiet hours filtering on their delivery channel). This distinction matters for agents that are observers, not publishers.

### 8.5 Stage 4: Delivery

Events are fan-out delivered to all agents with matching subscriptions:

1. The runtime iterates over active sessions.
2. For each session, it checks: (a) subscription category match, (b) event filter match, (c) capability check, (d) self-event suppression (5.3 Self-Event Suppression).
3. Matching events are enqueued on the session's outbound gRPC stream.

**Delivery semantics by traffic class** (per architecture.md):

| Event type | Traffic class | Delivery semantics |
|-----------|--------------|-------------------|
| Scene events (topology, zone) | State-stream | Reliable, ordered, coalesced under backpressure using coalesce key |
| Agent semantic events | Transactional | Reliable, ordered, acknowledged (via EmitSceneEventResult) |
| System events (lease, degradation) | State-stream | Reliable, ordered, coalesced |
| Attention budget events | State-stream | Reliable, ordered, singleton coalesce key per session |

**Events are fire-and-forget from the subscriber's perspective:** No ack is required from subscribers. The runtime does not wait for subscriber acknowledgement before proceeding. If a subscriber's outbound buffer is full, events are coalesced per their coalesce key (5.1 Coalesce Keys).

### 8.6 Coalesce Keys

Under backpressure, the runtime coalesces events with the same coalesce key:

| Event | Coalesce key |
|-------|-------------|
| `TileCreatedEvent` | `(tile_id)` -- unlikely to coalesce; creation is once |
| `TileDeletedEvent` | `(tile_id)` |
| `TileUpdatedEvent` | `(tile_id)` -- multiple updates coalesce into latest state |
| `TabCreatedEvent` | `(tab_id)` |
| `TabDeletedEvent` | `(tab_id)` |
| `TabRenamedEvent` | `(tab_id)` |
| `TabReorderedEvent` | `(tab_id)` |
| `ActiveTabChangedEvent` | singleton -- only one active tab; latest wins |
| `ZoneOccupancyChangedEvent` | `(zone_name, tab_id)` |
| `AgentJoinedEvent` | `(agent_namespace)` |
| `AgentDepartedEvent` | `(agent_namespace)` |
| `SyncGroupCommittedEvent` | `(sync_group_id)` |
| `SceneSnapshotAvailableEvent` | singleton |
| `LeaseGrantedEvent` | `(lease_id)` |
| `LeaseRenewedEvent` | `(lease_id)` -- latest renewal wins |
| `LeaseRevokedEvent` | `(lease_id)` |
| `LeaseExpiredEvent` | `(lease_id)` |
| `LeaseSuspendedEvent` | `(lease_id)` |
| `LeaseResumedEvent` | `(lease_id)` |
| `DegradationLevelChangedEvent` | singleton -- latest level wins |
| `ZoneEvictedEvent` | `(zone_name, tab_id)` |
| `AgentEventPayload` | `(event_type)` -- latest value per event type |
| `AttentionBudgetWarningEvent` | singleton per session |
| `AttentionBudgetRestoredEvent` | singleton per session |

**Lease and degradation events are never dropped**, even under extreme backpressure, because agents depend on them for correct behavior.

---

## 9. tab_switch_on_event Contract

### 9.1 Configuration

The `tab_switch_on_event` field (RFC 0006 5.4) names a scene-level event that automatically activates the tab:

```toml
[[tabs]]
name = "Security"
tab_switch_on_event = "doorbell.ring"  # Auto-switch when doorbell fires
```

The value is a dotted event name following the naming convention in 2.2. It matches against the `event_name` field of agent-emitted events (before namespace prefixing) or the `event_type` field of scene events.

### 9.2 Matching Rules

When an event fires, the runtime checks all tabs' `tab_switch_on_event` values:

1. **Agent events:** The match is against the bare event name (e.g., `doorbell.ring`), not the fully qualified `agent.<namespace>.doorbell.ring`. This allows the tab configuration to be agent-independent -- any agent that can emit `doorbell.ring` can trigger the tab switch.

2. **Scene events:** The match is against the scene event type (e.g., `zone.published`, `tab.switched`). In practice, using scene events as tab switch triggers is uncommon because they represent state changes that have already happened.

3. **System events:** System events **cannot** trigger tab switches. The `system.*` prefix is excluded from `tab_switch_on_event` matching. Tab switches driven by system state (e.g., safe mode) are handled directly by the runtime (RFC 0007 5).

### 9.3 Attention Filtering

A `tab_switch_on_event`-triggered tab switch is subject to the same attention filtering as any other interruption:

1. The triggering event's `InterruptionClass` determines whether the tab switch is permitted under the current quiet hours and attention budget state.
2. If quiet hours block the event, the tab switch is deferred until quiet hours end.
3. If the attention budget is exhausted, the tab switch is coalesced (the latest matching event wins when the budget refills).

### 9.4 Generated Events

A successful `tab_switch_on_event` tab switch generates an `ActiveTabChangedEvent` in the `scene_topology` category. This event is delivered to all `scene_topology` subscribers. The `ActiveTabChangedEvent` carries the new and old active tab IDs but does not carry the event that triggered the switch -- subscribers that need causal information must correlate by timestamp.

---

## 10. Relationship to RFC 0007 Audit Events

### 10.1 Audit Events Are System Events

Shell-state audit events from RFC 0007 (safe mode entry/exit, freeze/unfreeze, override actions like dismiss-all, mute-all) are **system events** in this taxonomy. They flow through the same event bus but with restricted visibility.

### 10.2 Audit Event Visibility

Audit events have the following properties:

- **Always CRITICAL interruption class.** Audit events are never queued, deferred, or coalesced.
- **Always logged regardless of subscription.** The runtime logs all audit events to the audit trail (RFC 0007 6) independently of whether any agent is subscribed.
- **Limited agent visibility.** Most audit events are **not delivered to agents**. This is consistent with RFC 0007's design decision that override events are internal:
  - **Agents DO receive:** `LeaseSuspendedEvent`, `LeaseResumedEvent` (safe mode enter/exit effects on their leases), `DegradationLevelChangedEvent` (degradation state changes).
  - **Agents DO NOT receive:** The raw override command (freeze, dismiss, mute). Agents learn about the *effects* of overrides (lease revoked, tile deleted) but not the command itself. This preserves viewer privacy (RFC 0007 4.3: the viewer's decision to freeze the scene is viewer state that must not be exposed to agents).

### 10.3 Safe Mode Event Flow

When safe mode is entered (RFC 0007 5):

1. The runtime generates a `system.safe_mode_entered` audit event (logged, not delivered to agents).
2. All active leases are suspended. Each suspension generates a `LeaseSuspendedEvent` delivered to the affected agent.
3. When safe mode exits, a `system.safe_mode_exited` audit event is logged.
4. Suspended leases are resumed. Each generates a `LeaseResumedEvent`.

Agents experience safe mode as a lease suspension/resumption cycle. They do not see the safe mode event itself.

### 10.4 Freeze Event Flow

When the scene is frozen (RFC 0007 4.3):

1. The runtime generates a `system.freeze_entered` audit event (logged, not delivered to agents).
2. Agents are NOT informed that the scene is frozen. They continue submitting mutations, which are silently queued.
3. If the queue reaches 80% capacity, the runtime sends `MUTATION_QUEUE_PRESSURE` via `RuntimeError` in `MutationResult` (RFC 0007 freeze advisory model decision). This does not reveal the cause of pressure.
4. When the scene is unfrozen, queued mutations are applied and a `system.freeze_exited` audit event is logged.

---

## 11. Payload Messages

### 11.1 Zone Event Payload

```protobuf
message ZoneEventPayload {
  string  zone_name           = 1;  // Zone instance name (e.g., "notification").
  SceneId tab_id              = 2;  // Tab this zone instance belongs to.
  ZoneEventKind kind          = 3;
  string  publisher_namespace = 4;  // Namespace of agent that caused the change.
                                    // Empty if cleared by runtime policy.
}

enum ZoneEventKind {
  ZONE_EVENT_KIND_UNSPECIFIED = 0;
  ZONE_EVENT_KIND_PUBLISHED   = 1;  // Content was published (new or replace).
  ZONE_EVENT_KIND_CLEARED     = 2;  // Content was removed (TTL expiry, ClearZone, or eviction).
  ZONE_EVENT_KIND_STACKED     = 3;  // New item added to a Stack-policy zone.
  ZONE_EVENT_KIND_MERGED      = 4;  // Key-value update on a MergeByKey zone.
  ZONE_EVENT_KIND_EVICTED     = 5;  // Agent's content evicted by higher-priority publisher.
}
```

### 11.2 Tile Event Payload

```protobuf
message TileEventPayload {
  SceneId tile_id              = 1;
  SceneId tab_id               = 2;
  TileEventKind kind           = 3;
  string  agent_namespace      = 4;
  // Fields below are populated only for CREATED and UPDATED events.
  Rect    bounds               = 5;
  uint32  z_order              = 6;
  float   opacity              = 7;
  InputMode input_mode         = 8;
  SceneId sync_group           = 9;
  SceneId lease_id             = 10;
  TileDeletionReason deletion_reason = 11;  // Populated only for DELETED events.
}

enum TileEventKind {
  TILE_EVENT_KIND_UNSPECIFIED = 0;
  TILE_EVENT_KIND_CREATED     = 1;
  TILE_EVENT_KIND_DELETED     = 2;
  TILE_EVENT_KIND_UPDATED     = 3;  // Bounds, z-order, opacity, input mode, or sync group changed.
  TILE_EVENT_KIND_LEASE_CHANGED = 4;  // Lease state changed (e.g., renewed, suspended).
}

enum TileDeletionReason {
  TILE_DELETION_REASON_UNSPECIFIED  = 0;
  TILE_DELETION_REASON_AGENT        = 1;  // Owning agent issued DeleteTile.
  TILE_DELETION_REASON_LEASE_EXPIRY = 2;  // Tile's TTL expired.
  TILE_DELETION_REASON_VIEWER       = 3;  // Viewer dismissed tile (RFC 0007 4.1).
  TILE_DELETION_REASON_BUDGET       = 4;  // Lease revoked for budget violation (RFC 0008 3).
  TILE_DELETION_REASON_SAFE_MODE    = 5;  // Session suspended on safe mode entry (RFC 0007 5.2).
  TILE_DELETION_REASON_SESSION_END  = 6;  // Agent session closed and grace period expired.
}
```

### 11.3 Tab Event Payload

```protobuf
message TabEventPayload {
  SceneId tab_id         = 1;
  TabEventKind kind      = 2;
  string  name           = 3;  // Tab name (populated for CREATED, RENAMED).
  uint32  display_order  = 4;  // Display order (populated for CREATED, REORDERED).
  SceneId old_active_tab = 5;  // For SWITCHED: previous active tab (zero = startup).
  SceneId new_active_tab = 6;  // For SWITCHED: new active tab.
}

enum TabEventKind {
  TAB_EVENT_KIND_UNSPECIFIED = 0;
  TAB_EVENT_KIND_CREATED     = 1;
  TAB_EVENT_KIND_DELETED     = 2;
  TAB_EVENT_KIND_RENAMED     = 3;
  TAB_EVENT_KIND_REORDERED   = 4;
  TAB_EVENT_KIND_SWITCHED    = 5;  // Active tab changed.
}
```

### 11.4 Agent Event Payload

```protobuf
// Payload for agent-emitted semantic events.
// Delivered when an agent calls EmitSceneEvent.
message AgentEventPayload {
  string event_name      = 1;  // The bare event name (e.g., "doorbell.ring").
                               // The full event_type in SceneEvent is
                               // "agent.<namespace>.<event_name>".
  string agent_namespace = 2;  // Emitting agent's namespace.
  bytes  payload         = 3;  // Agent-supplied opaque payload (max 4KB).
  InterruptionClass requested_class  = 4;  // Class the agent requested.
  InterruptionClass effective_class  = 5;  // Class the runtime assigned.
}
```

### 11.5 System Event Payload

```protobuf
message SystemEventPayload {
  SystemEventKind kind     = 1;
  // Fields below are populated based on kind.
  uint32 degradation_level_new = 2;  // For DEGRADATION_CHANGED.
  uint32 degradation_level_old = 3;  // For DEGRADATION_CHANGED.
  string reason                = 4;  // Human-readable reason (optional).
  // Lease fields (for LEASE_* kinds).
  SceneId lease_id             = 5;
  string  lease_namespace      = 6;
  uint64  lease_ttl_ms         = 7;
  uint32  lease_priority       = 8;
  uint64  lease_expires_at_wall_us = 9;
  LeaseRevocationReason lease_revocation_reason = 10;
  // Attention budget fields (for BUDGET_* kinds).
  uint32 budget_current_rate   = 11;
  uint32 budget_max_rate       = 12;
  bool   budget_zone_also_full = 13;
}

enum SystemEventKind {
  SYSTEM_EVENT_KIND_UNSPECIFIED          = 0;
  SYSTEM_EVENT_KIND_SAFE_MODE_ENTERED    = 1;   // RFC 0007 5. Internal only; agents see LeaseSuspended.
  SYSTEM_EVENT_KIND_SAFE_MODE_EXITED     = 2;   // RFC 0007 5. Internal only; agents see LeaseResumed.
  SYSTEM_EVENT_KIND_FREEZE_ENTERED       = 3;   // RFC 0007 4.3. Internal only.
  SYSTEM_EVENT_KIND_FREEZE_EXITED        = 4;   // RFC 0007 4.3. Internal only.
  SYSTEM_EVENT_KIND_VIEWER_CONTEXT_CHANGED = 5; // privacy.md. Internal only.
  SYSTEM_EVENT_KIND_DEGRADATION_CHANGED  = 6;   // RFC 0002 6. Delivered to agents.
  SYSTEM_EVENT_KIND_LEASE_GRANTED        = 7;   // RFC 0008. Delivered to agents.
  SYSTEM_EVENT_KIND_LEASE_RENEWED        = 8;
  SYSTEM_EVENT_KIND_LEASE_REVOKED        = 9;
  SYSTEM_EVENT_KIND_LEASE_EXPIRED        = 10;
  SYSTEM_EVENT_KIND_LEASE_SUSPENDED      = 11;
  SYSTEM_EVENT_KIND_LEASE_RESUMED        = 12;
  SYSTEM_EVENT_KIND_BUDGET_WARNING       = 13;  // Delivered to attention_events subscribers.
  SYSTEM_EVENT_KIND_BUDGET_RESTORED      = 14;  // Delivered to attention_events subscribers.
}

enum LeaseRevocationReason {
  LEASE_REVOCATION_REASON_UNSPECIFIED  = 0;
  LEASE_REVOCATION_REASON_VIEWER       = 1;  // Viewer dismiss action (RFC 0007 4.1).
  LEASE_REVOCATION_REASON_BUDGET       = 2;  // Budget policy (RFC 0008 3).
  LEASE_REVOCATION_REASON_CAPABILITY   = 3;  // Required capability revoked mid-session.
  LEASE_REVOCATION_REASON_ADMIN        = 4;  // Runtime administrator action.
}
```

### 11.6 Sync Group Event Payload

```protobuf
// Emitted when all mutations in a sync group are committed atomically.
message SyncGroupEventPayload {
  SceneId sync_group_id      = 1;  // The sync group that committed.
  uint32  member_count       = 2;  // Number of tiles/zones in the group.
  string  committing_namespace = 3;  // Agent that triggered the commit.
}
```

---

## 12. Quantitative Requirements

| Metric | Target | Rationale |
|--------|--------|-----------|
| Event classification (Stage 2) | < 5us per event | Must not contribute meaningfully to frame budget (16.6ms at 60fps) |
| Quiet hours check | < 2us per event | Simple timestamp comparison; must not block pipeline |
| Event delivery to subscriber | < 100us from emission | Subscribers should observe scene changes within the same frame they occur |
| Max event rate (aggregate, all sources) | 1000 events/second | Above this rate, the system is misbehaving. Individual sources are limited more tightly. |
| Max agent event rate | 10 events/second per agent | Prevents event spam from individual agents |
| Max subscriptions per agent | 32 | Prevents subscription explosion; most agents need 3-5 categories |
| Event payload max size | 4KB | Agent event payloads are bounded to prevent memory abuse |
| Coalesce buffer per subscriber | 64 entries | Bounded coalesce buffer prevents unbounded memory growth under sustained backpressure |
| Quiet hours queue depth per zone | 100 entries | Bounded queue prevents memory growth during long quiet hours windows |
| Self-event suppression check | < 1us per event | Simple lease_id comparison |

---

## 13. Complete Protobuf Schema

### 13.1 events.proto

```protobuf
syntax = "proto3";

package tze_hud.events.v1;

import "scene.proto";   // SceneId, Rect, InputMode (tze_hud.scene.v1; RFC 0001 7.1)

// ── Interruption class ──────────────────────────────────────────────────────

enum InterruptionClass {
  INTERRUPTION_CLASS_CRITICAL    = 0;  // Always delivered. Bypasses quiet hours and budget.
  INTERRUPTION_CLASS_HIGH        = 1;  // Delivered unless quiet hours block it.
  INTERRUPTION_CLASS_NORMAL      = 2;  // Standard activity. Filtered by attention budget.
  INTERRUPTION_CLASS_LOW         = 3;  // Batched/deferred. Subtle indicators only.
  INTERRUPTION_CLASS_SILENT      = 4;  // Never interrupts. Query-only.
  INTERRUPTION_CLASS_UNSPECIFIED = 5;  // Inherit zone default (or NORMAL if no zone default).
}

// ── Presence level ──────────────────────────────────────────────────────────
// Shared enum; also defined in session.proto (RFC 0005).
// session.proto's PresenceLevel and this PresenceLevel must remain identical.

enum PresenceLevel {
  PRESENCE_LEVEL_UNSPECIFIED = 0;
  PRESENCE_LEVEL_GUEST       = 1;
  PRESENCE_LEVEL_RESIDENT    = 2;
  PRESENCE_LEVEL_EMBODIED    = 3;  // Reserved; not active in v1 (RFC 0005 1.2).
}

// ── SceneEvent (main event envelope) ────────────────────────────────────────

message SceneEvent {
  bytes  event_id            = 1;   // Unique event ID (UUID v7, 16 bytes).
  string event_type          = 2;   // Namespaced: "zone.published", "tile.created",
                                    //             "agent.doorbell_agent.doorbell.ring",
                                    //             "system.degradation_changed"
  InterruptionClass interruption_class = 3;  // Effective class after ceiling enforcement.
  uint64 timestamp_wall_us   = 4;   // UTC us since epoch (RFC 0003 3.1).
  uint64 timestamp_mono_us   = 5;   // Monotonic us (RFC 0003 3.1).
  bytes  source_lease_id     = 6;   // Lease that generated this (empty for system events).
  string source_namespace    = 7;   // Agent namespace (empty for runtime events).
  uint64 sequence            = 8;   // Scene sequence number for ordering/gap detection.

  oneof payload {
    ZoneEventPayload       zone       = 10;
    TileEventPayload       tile       = 11;
    TabEventPayload        tab        = 12;
    AgentEventPayload      agent      = 13;
    SystemEventPayload     system     = 14;
    SyncGroupEventPayload  sync_group = 15;
  }
}

// ── Zone events ─────────────────────────────────────────────────────────────

enum ZoneEventKind {
  ZONE_EVENT_KIND_UNSPECIFIED = 0;
  ZONE_EVENT_KIND_PUBLISHED   = 1;
  ZONE_EVENT_KIND_CLEARED     = 2;
  ZONE_EVENT_KIND_STACKED     = 3;
  ZONE_EVENT_KIND_MERGED      = 4;
  ZONE_EVENT_KIND_EVICTED     = 5;
}

message ZoneEventPayload {
  string        zone_name           = 1;
  SceneId       tab_id              = 2;
  ZoneEventKind kind                = 3;
  string        publisher_namespace = 4;  // Empty if cleared by runtime policy.
}

// ── Tile events ─────────────────────────────────────────────────────────────

enum TileEventKind {
  TILE_EVENT_KIND_UNSPECIFIED   = 0;
  TILE_EVENT_KIND_CREATED       = 1;
  TILE_EVENT_KIND_DELETED       = 2;
  TILE_EVENT_KIND_UPDATED       = 3;
  TILE_EVENT_KIND_LEASE_CHANGED = 4;
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

message TileEventPayload {
  SceneId            tile_id          = 1;
  SceneId            tab_id           = 2;
  TileEventKind      kind             = 3;
  string             agent_namespace  = 4;
  Rect               bounds           = 5;   // For CREATED, UPDATED.
  uint32             z_order          = 6;
  float              opacity          = 7;
  InputMode          input_mode       = 8;
  SceneId            sync_group       = 9;
  SceneId            lease_id         = 10;
  TileDeletionReason deletion_reason  = 11;  // For DELETED.
}

// ── Tab events ──────────────────────────────────────────────────────────────

enum TabEventKind {
  TAB_EVENT_KIND_UNSPECIFIED = 0;
  TAB_EVENT_KIND_CREATED     = 1;
  TAB_EVENT_KIND_DELETED     = 2;
  TAB_EVENT_KIND_RENAMED     = 3;
  TAB_EVENT_KIND_REORDERED   = 4;
  TAB_EVENT_KIND_SWITCHED    = 5;
}

message TabEventPayload {
  SceneId      tab_id          = 1;
  TabEventKind kind            = 2;
  string       name            = 3;   // For CREATED, RENAMED.
  uint32       display_order   = 4;   // For CREATED, REORDERED.
  SceneId      old_active_tab  = 5;   // For SWITCHED.
  SceneId      new_active_tab  = 6;   // For SWITCHED.
}

// ── Agent events ────────────────────────────────────────────────────────────

message AgentEventPayload {
  string            event_name       = 1;
  string            agent_namespace  = 2;
  bytes             payload          = 3;   // Max 4KB.
  InterruptionClass requested_class  = 4;
  InterruptionClass effective_class  = 5;
}

// ── System events ───────────────────────────────────────────────────────────

enum SystemEventKind {
  SYSTEM_EVENT_KIND_UNSPECIFIED            = 0;
  SYSTEM_EVENT_KIND_SAFE_MODE_ENTERED      = 1;
  SYSTEM_EVENT_KIND_SAFE_MODE_EXITED       = 2;
  SYSTEM_EVENT_KIND_FREEZE_ENTERED         = 3;
  SYSTEM_EVENT_KIND_FREEZE_EXITED          = 4;
  SYSTEM_EVENT_KIND_VIEWER_CONTEXT_CHANGED = 5;
  SYSTEM_EVENT_KIND_DEGRADATION_CHANGED    = 6;
  SYSTEM_EVENT_KIND_LEASE_GRANTED          = 7;
  SYSTEM_EVENT_KIND_LEASE_RENEWED          = 8;
  SYSTEM_EVENT_KIND_LEASE_REVOKED          = 9;
  SYSTEM_EVENT_KIND_LEASE_EXPIRED          = 10;
  SYSTEM_EVENT_KIND_LEASE_SUSPENDED        = 11;
  SYSTEM_EVENT_KIND_LEASE_RESUMED          = 12;
  SYSTEM_EVENT_KIND_BUDGET_WARNING         = 13;
  SYSTEM_EVENT_KIND_BUDGET_RESTORED        = 14;
}

enum LeaseRevocationReason {
  LEASE_REVOCATION_REASON_UNSPECIFIED = 0;
  LEASE_REVOCATION_REASON_VIEWER      = 1;
  LEASE_REVOCATION_REASON_BUDGET      = 2;
  LEASE_REVOCATION_REASON_CAPABILITY  = 3;
  LEASE_REVOCATION_REASON_ADMIN       = 4;
}

message SystemEventPayload {
  SystemEventKind kind                     = 1;
  uint32          degradation_level_new    = 2;
  uint32          degradation_level_old    = 3;
  string          reason                   = 4;
  SceneId         lease_id                 = 5;
  string          lease_namespace          = 6;
  uint64          lease_ttl_ms             = 7;
  uint32          lease_priority           = 8;
  uint64          lease_expires_at_wall_us = 9;
  LeaseRevocationReason lease_revocation_reason = 10;
  uint32          budget_current_rate      = 11;
  uint32          budget_max_rate          = 12;
  bool            budget_zone_also_full    = 13;
}

// ── Sync group events ───────────────────────────────────────────────────────

message SyncGroupEventPayload {
  SceneId sync_group_id        = 1;
  uint32  member_count         = 2;
  string  committing_namespace = 3;
}

// ── Agent event emission (client -> server) ─────────────────────────────────

message EmitSceneEvent {
  string            event_name        = 1;
  InterruptionClass interruption_class = 2;
  bytes             payload           = 3;  // Max 4KB.
}

message EmitSceneEventResult {
  bool              accepted          = 1;
  InterruptionClass effective_class   = 2;
  RuntimeError      error             = 3;
}

// ── Event subscription filter ───────────────────────────────────────────────

message EventFilter {
  string event_type_prefix = 1;  // Trailing wildcard: "zone.*", "agent.foo.*"
}
```

### 13.2 Required Changes to Existing Protos

The following amendments are normative. Existing RFCs must be updated to align.

**RFC 0001 `scene_service.proto`** -- Add to `ZoneDefinitionProto`:
```protobuf
import "events.proto";  // InterruptionClass

// In message ZoneDefinitionProto:
InterruptionClass default_interruption_class = 10;  // UNSPECIFIED = NORMAL
InterruptionClass ceiling_interruption_class = 11;  // UNSPECIFIED = CRITICAL (no ceiling)
```

**RFC 0005 `session.proto`** -- Add field 5 to `ZonePublish`:
```protobuf
import "events.proto";  // InterruptionClass

// In message ZonePublish:
InterruptionClass interruption_class = 5;  // UNSPECIFIED = use zone default
```

**RFC 0005 `session.proto`** -- Add to `SubscriptionCategory` enum:
```protobuf
ATTENTION_EVENTS = 8;   // AttentionBudgetWarning, AttentionBudgetRestored
AGENT_EVENTS     = 9;   // Agent-emitted semantic events
```

**RFC 0005 `session.proto`** -- Add to `SessionMessage` oneof (client -> server):
```protobuf
EmitSceneEvent      emit_scene_event      = 45;  // Agent emits a named event
```

**RFC 0005 `session.proto`** -- Add to `SessionMessage` oneof (server -> client):
```protobuf
EmitSceneEventResult emit_scene_event_result = 46;  // Ack for EmitSceneEvent
```

**RFC 0005 `session.proto`** -- Add `filters` field to `SubscriptionChange`:
```protobuf
// In message SubscriptionChange:
repeated EventFilter filters = 3;  // Fine-grained event type filters
```

**RFC 0005 `session.proto`** -- Update `SceneEvent` import:
```protobuf
// SceneEvent is now defined in events.proto (RFC 0010 13.1), not in scene_service.proto.
import "events.proto";
```

---

## 14. Delivery Semantics

### 14.1 Ordering Guarantees

- Events within a session are delivered in generation order (the order in which the runtime generated them, indexed by `SceneEvent.sequence`).
- An agent that misses events due to disconnection receives a full `SceneSnapshot` on reconnect (RFC 0005 1.3, 6.5) and does not receive a replay of missed events.
- The `sequence` field in `SceneEvent` allows agents to detect gaps. A gap indicates that a reconnect+snapshot cycle is needed.

### 14.2 Self-Event Suppression

The runtime MUST NOT deliver a `SceneEvent` to the agent whose `MutationBatch` caused the state change. The agent already has ground truth from its `BatchCommitted` response.

**Exception:** Lease and degradation events are always delivered even if the agent's own action triggered them. A lease revocation caused by the agent's own budget violation should still be delivered to the agent so it knows its lease is gone.

### 14.3 Agent Event Delivery

Agent-emitted events (agent payload) are delivered to all agents subscribed to `agent_events` that pass the capability check. The emitting agent does NOT receive its own event (self-event suppression applies).

---

## 15. Design Decisions and Rationale

### 15.1 Why a unified SceneEvent envelope rather than per-category message types

A single `SceneEvent` envelope with a payload oneof preserves the ordering guarantee: the agent sees all subscribed events in global sequence order. Per-category streams would require agents to merge and re-sort, which is complex and error-prone. The envelope also carries common fields (event_id, interruption_class, timestamps, source) uniformly, eliminating per-type boilerplate.

### 15.2 Why agents do not receive self-events

Self-event suppression (14.2) prevents feedback loops: an agent that listens to `scene_topology` and reacts to changes would loop if it received events from its own mutations. The `BatchCommitted` response provides authoritative confirmation.

### 15.3 Why attention budget state is not queryable on demand

Providing a real-time budget query would encourage agents to "budget-game" -- publishing as close to the ceiling as possible. The warning/restored event pair rewards cooperative agents without giving exploitative agents precise optimization data.

### 15.4 Why agent events use the bare event name for tab_switch_on_event matching

The `tab_switch_on_event` configuration should be agent-independent. A tab configured with `tab_switch_on_event = "doorbell.ring"` should switch regardless of which agent emits the event. Matching against the bare name (before namespace prefixing) achieves this.

### 15.5 Why system events have restricted visibility

System events like safe mode and freeze represent viewer actions. Exposing them to agents would leak viewer intent (privacy.md "Agent isolation"). Agents see the effects (lease suspension, mutation backpressure) but not the cause.

---

## 16. Open Questions

1. **Zone-level attention budget per zone type vs. per instance:** The current design tracks attention budget per zone instance. A deployment with many tabs would maintain many independent counters. An alternative is per zone type. Decision deferred to implementation.

2. **Post-v1: agent-to-agent signaling plane:** presence.md "Inter-agent events" states "Direct agent-to-agent communication is out of scope for the presence engine." This RFC provides scene-level coordination signals. If agents need richer coordination post-v1, it would be a separate signaling plane.

3. **EventFilter wildcard depth:** V1 supports only trailing wildcards at the segment boundary (`zone.*`). Post-v1 may support deeper patterns (`agent.*.doorbell.*`). The `EventFilter` message is designed to be extended.

4. **Attention budget defaults tuning:** The default values (20 per agent per minute, 10 per zone per minute) are starting points subject to tuning based on real deployments.

---

## 17. Subscription Category Summary (Updated)

This table supersedes RFC 0005 7.1 for the complete category list.

| Category | Enum Value | Description | Minimum Capability | Opt-out? |
|----------|-----------|-------------|-------------------|----------|
| `scene_topology` | 1 | Tile, tab, zone, agent presence, sync group changes | `read_scene_topology` | Yes |
| `input_events` | 2 | Pointer, touch, key events routed to agent's tiles | `access_input_events` | Yes |
| `focus_events` | 3 | Focus gained/lost on agent's tiles | `access_input_events` | Yes |
| `degradation_notices` | 4 | Runtime degradation level changes | *(none required)* | **No** |
| `lease_changes` | 5 | Lease state transitions for agent's own leases | *(none required)* | **No** |
| `zone_events` | 6 | Zone occupancy changes for accessible zones; ZoneEvicted | `publish_zone:<zone>` | Yes |
| `telemetry_frames` | 7 | Runtime performance telemetry samples | `read_telemetry` | Yes |
| `attention_events` | 8 | Attention budget warning/restored signals | `read_scene_topology` | Yes |
| `agent_events` | 9 | Agent-emitted semantic events | `subscribe_scene_events` | Yes |

`degradation_notices` (4) and `lease_changes` (5) are always active for all sessions and cannot be disabled.

---

## 18. Related RFCs

| RFC | Relationship |
|-----|-------------|
| RFC 0001 (Scene Contract) | Defines scene objects referenced in events. `SceneEvent` ownership moves from `scene_service.proto` to `events.proto` (this RFC). `ZoneDefinitionProto` gains `default_interruption_class` and `ceiling_interruption_class` fields. |
| RFC 0002 (Runtime Kernel) | Degradation levels referenced in `DegradationLevelChangedEvent` are defined in RFC 0002 6. Event emission integrates with the compositor pipeline stages (RFC 0002 3.2). |
| RFC 0003 (Timing Model) | `timestamp_wall_us` and `timestamp_mono_us` in `SceneEvent` follow RFC 0003 3.1 clock domain conventions. |
| RFC 0004 (Input Model) | `input_events` and `focus_events` categories carry RFC 0004 event types. Those types are defined in RFC 0004, not this RFC. This RFC defines how input events interact with the scene event bus (1.1). |
| RFC 0005 (Session Protocol) | Session stream delivery, subscription categories, and `ZonePublish` are defined in RFC 0005. This RFC adds two categories (`attention_events`, `agent_events`), two `SessionMessage` fields (`emit_scene_event`, `emit_scene_event_result`), and `EventFilter` to `SubscriptionChange`. |
| RFC 0006 (Configuration) | `tab_switch_on_event` (5.4), `emit_scene_event` capability (6.3), scene-event taxonomy (5.5), and attention budget configuration (`[privacy]` section) are defined in RFC 0006. This RFC is the authoritative specification that RFC 0006 5.5 defers to. |
| RFC 0007 (System Shell) | Override events are internal (RFC 0007 7.3). Downstream effects surface as lease and tile events here. Budget warning badge (RFC 0007 3.5) and `AttentionBudgetWarningEvent` share the same threshold. Safe mode and freeze audit events flow through this RFC's system event path with restricted visibility (10). |
| RFC 0008 (Lease Governance) | Lease state machine transitions map to system events with lease payload in this RFC. |
| RFC 0009 (Policy Arbitration) | Interruption gate (Level 4) and attention budget are specified in RFC 0009. This RFC provides the formal definitions of `InterruptionClass` and attention budget semantics they reference. |
