# RFC 0008: Lease and Resource Governance

**Status:** Draft
**Issue:** rig-lqp
**Date:** 2026-03-23
**Authors:** tze_hud architecture team
**Depends on:** RFC 0001 (Scene Contract), RFC 0002 (Runtime Kernel), RFC 0005 (Session Protocol), RFC 0007 (System Shell)

---

## Summary

This RFC consolidates lease lifecycle, resource governance, and budget enforcement into a single authoritative specification. Lease concepts are referenced throughout RFC 0001, RFC 0002, RFC 0005, and RFC 0007, but no single document defines the complete state machine, suspension semantics, zone interaction, assignment semantics, safe-mode behavior, or enforcement ladder. This RFC is that document.

All other RFCs defer to this document on lease lifecycle questions. Contradictions between this RFC and other RFCs are resolved here; those RFCs must be updated to align.

---

## Motivation

The doctrine in `presence.md` and `security.md` is clear: every resident or embodied agent holds leases with TTL, renewal semantics, capability scopes, resource budgets, and revocation semantics. The existing RFCs implement these concepts piecemeal:

- RFC 0001 SS3.3 defines lease checks within the mutation pipeline but does not define the lease state machine.
- RFC 0002 SS5 defines budget enforcement tiers and the degradation ladder but does not define lease lifecycle events.
- RFC 0005 SS1 defines session lifecycle with lease orphaning on disconnect but defers lease protocol details to `scene_service.proto`.
- RFC 0007 SS4.2/SS5.2 contained a contradiction: "Dismiss All" revoked leases simultaneously, but safe mode was explicitly designed to allow resume without re-establishing leases.

Without a canonical state machine:
- Implementers must reconcile lease behavior by reading four documents and resolving contradictions manually.
- The safe-mode revoke/suspend contradiction creates an ambiguous implementation target.
- `lease_priority` assignment and sort semantics are mentioned (RFC 0002 SS5.2, SS6.2) but never fully specified.
- Budget enforcement triggers (SS5 of RFC 0002) do not specify which lease states are valid targets for each enforcement action.
- Suspension vs. revocation semantics are conflated, making disconnect handling ambiguous.
- Zone publishing and lease interaction is undefined, leaving agents uncertain whether zone publishes require active leases.

---

## Design Requirements Satisfied

| ID | Requirement | Source |
|----|-------------|--------|
| DR-LG1 | Complete, unambiguous lease state machine | presence.md SS"Leases: presence requires governance" |
| DR-LG2 | Lease priority assignment and sort order | RFC 0002 SS5.2, SS6.2 |
| DR-LG3 | Safe-mode lease behavior: suspend (not revoke) | RFC 0007 SS5.2, security.md SS"Human override" |
| DR-LG4 | Resource budget definition and enforcement ladder | security.md SS"Resource governance", RFC 0002 SS5 |
| DR-LG5 | Degradation triggers and lease interaction | RFC 0002 SS6, failure.md SS"Degradation axes" |
| DR-LG6 | Lease protocol messages (request/response/events) | RFC 0001, RFC 0005 |
| DR-LG7 | Contradiction resolution: Dismiss All revoke vs. safe-mode suspend | RFC 0007 SS4.2 vs. SS5.2 |
| DR-LG8 | Lease-zone relationship and zone publish governance | presence.md SS"Zones: the LLM-first publishing surface" |
| DR-LG9 | Suspension semantics: preserves state, resumable | security.md SS"Human override" |
| DR-LG10 | Orphan/disconnect handling with grace periods | failure.md SS"Agent crashes", SS"Reconnection contract" |

---

## 1. Lease Concepts

### 1.1 What a Lease Is

A **lease** is a runtime-issued, time-bounded authorization that grants an agent:

- A **namespace** (identity boundary for tile ownership)
- **Mutation rights** over tiles in that namespace
- A **capability scope** (what operations are permitted -- see RFC 0005 SS7 for the capability vocabulary)
- A **resource budget** (texture memory, update rate, node count, concurrent tiles)
- A **TTL** with renewal semantics
- A **priority** governing shedding order under degradation
- **Revocation semantics** (how and when the runtime can reclaim the lease)

Leases are not tiles. A single lease may govern multiple tiles. Conversely, each tile's `lease_id` field references exactly one current lease. A lease is not a session -- a single session may hold multiple leases (up to `max_active_leases`).

> **Doctrine:** "Every resident or embodied agent receives: a namespace, one or more surface leases, capability scopes (what it can do), TTL and renewal semantics, resource budgets (memory, bandwidth, update rate), allowed z-order or overlay privileges, event subscriptions (what it can observe), revocation semantics (how and when the runtime can take it back)." -- presence.md SS"Leases: presence requires governance"

### 1.2 What Requires a Lease

| Operation | Lease Required |
|-----------|---------------|
| `CreateTile` mutation | Yes -- agent must hold a valid lease in the target namespace |
| `InsertNode`, `ReplaceNode`, `RemoveNode` | Yes -- lease must be `ACTIVE` for the tile's `lease_id` |
| `UpdateTileBounds`, `UpdateTileExpiry` | Yes -- same tile ownership check |
| `ZonePublish` (resident agent path) | Yes -- lease must be `ACTIVE` and include `publish_zone:<zone_type>` capability |
| `ZonePublish` (guest MCP path) | No -- guest uses `ZonePublishToken`; runtime holds the underlying tile |
| `CreateTab`, `RemoveTab` | No -- tab operations require `manage_tabs` capability, not a surface lease |
| Read operations (scene queries, hit-test) | No -- read access is governed by `read_scene_topology` capability, not leases |

### 1.3 Lease Identity

Each lease has:
- A `LeaseId` (UUIDv7, time-ordered, assigned by runtime at grant time; SceneId type per RFC 0001 SS1.1)
- A `namespace` (agent identity string, established at session auth)
- A `session_id` (parent session; lease is invalidated if session is revoked)
- A `granted_at_us` (UTC microseconds; RFC 0003 SS3.1 wall-clock domain)
- A `ttl_ms` (lease duration; 0 = indefinite, subject to session TTL)
- A `renewal_policy` (see SS1.4)
- A `capability_scope` (list of granted capabilities from the RFC 0005 SS7 vocabulary)
- A `resource_budget` (see SS6)
- A `lease_priority` (see SS2)

### 1.4 Renewal Policy

Each lease carries a `renewal_policy` that governs how TTL renewal is handled:

| Policy | Behavior |
|--------|----------|
| `MANUAL` (default) | Agent must explicitly send `LeaseRequest` with `operation = RENEW` before TTL expires. No runtime assistance. |
| `AUTO_RENEW` | Runtime automatically renews the lease at 75% TTL elapsed, provided the agent's session is `Active` and no budget violations are pending. The renewal uses the same TTL as the current grant. Agent receives a `LeaseResponse` with `result = GRANTED` on each auto-renewal. |
| `ONE_SHOT` | Lease expires at TTL without renewal option. The agent must request a new lease to continue. Suitable for transient operations (single notification, one-time overlay). |

Auto-renewal is a convenience -- the runtime acts on the agent's behalf. Auto-renewal can be disabled mid-lease by the runtime if:
- The agent enters budget warning state (SS6.3)
- The agent's session enters `Disconnecting` state
- Safe mode is entered (TTL clock paused per SS4.3; auto-renewal suspended)

---

## 2. Lease Priority

### 2.1 Assignment Rules

Every lease carries a `lease_priority: u8` field. Priority determines shedding order under the degradation ladder (RFC 0002 SS6).

| Value | Priority Class | Assignment Rule |
|-------|---------------|-----------------|
| 0 | System/Chrome | Runtime-internal only. Used for chrome layer tiles, safe-mode overlays, system indicators. Agents may **not** request priority 0; it is reserved for runtime-owned surfaces. Never shed under any degradation level. |
| 1 | High | Agent-assigned. Requires `lease:priority:1` capability. Granted to primary interactive agents, embodied sessions with media access, or agents operating in `exclusive` zones (RFC 0001 SS2.5). |
| 2 | Normal (default) | Agent-assigned. Any agent may request priority 2. This is the default if no priority is specified in the `LeaseRequest`. Standard resident agents receive this priority. |
| 3 | Low | Agent-assigned. Explicitly requested for background/ambient content: weather widgets, status dashboards, decorative overlays. May also be assigned by the runtime when downgrading an agent's requested priority due to capability constraints. |
| 4+ | Background/Speculative | Agent-assigned. Best-effort rendering. First to be shed under any degradation pressure. Suitable for pre-fetched content, speculative overlays, or content that the agent acknowledges may not be rendered. |

**Runtime override:** The runtime always grants the **effective** priority, which may differ from the agent's request:
- An agent requesting priority 1 without `lease:priority:1` capability receives priority 2.
- An agent requesting priority 0 always receives priority 2 (or whatever its capability ceiling permits).
- The runtime may downgrade priority at any time via `LeaseStateChange` (SS7.3) if policy conditions change.

### 2.2 Sort Semantics

When the compositor must order tiles for rendering priority or shedding decisions, it uses a two-key sort:

```
sort key = (lease_priority ASC, z_order DESC)
```

Numerically lower `lease_priority` values are higher priority (0 = highest). Within the same priority class, higher `z_order` wins (foreground tiles preserved over background tiles).

Tiles with the highest `lease_priority` values (least important) and lowest `z_order` values are shed first.

**Note on RFC 0002 phrasing:** RFC 0002 SS5.2 uses `DESC` for `lease_priority`, which means "sorted descending by value, so lower numeric values appear first" -- equivalent to `ASC` when the goal is "lower value = higher importance." This RFC uses the clearer formulation: sort `lease_priority` ascending (lower value = higher priority preserved first).

### 2.3 Priority Constraints

- Runtime-internal leases (chrome layer tiles, safe-mode overlays) use priority 0 and cannot be overridden by any agent action.
- A tile shed under the degradation ladder (RFC 0002 SS6.2, Level 4+) remains in the scene graph with its `lease_id` valid; the lease is not revoked -- only rendering is deferred. This is a rendering decision, not a governance decision.
- If a lease is **suspended** (see SS3), its tiles are not rendered regardless of priority.
- If a lease is **revoked**, its tiles are immediately removed from the scene graph.

---

## 3. Lease State Machine

### 3.1 States

```
REQUESTED --(granted)---> ACTIVE --(TTL expired)-------------------> EXPIRED
     |                      |                                       (terminal)
     |(denied)              |
     v                      |(viewer dismisses tile)
   DENIED                   +---(viewer_dismissed)------------------> REVOKED
   (terminal)               |                                       (terminal)
                            |(budget policy: throttle sustained 30s
                            |  or critical limit exceeded)
                            +---(budget_policy)---------------------> REVOKED
                            |
                            |(safe mode entry)
                            +---(safe_mode)-------------------------> SUSPENDED
                            |
                            |(agent disconnect, ungraceful or
                            |  graceful with expect_resume)
                            +---(disconnect)------------------------> ORPHANED
                            |
                            |(agent releases lease)
                            +---(released)--------------------------> RELEASED
                                                                    (terminal)

SUSPENDED --(safe_mode_exit)--> ACTIVE
SUSPENDED --(max_suspension_time exceeded)--> REVOKED

ORPHANED  --(agent reconnects within grace period)--> ACTIVE
ORPHANED  --(grace period expires)-------------------> EXPIRED

NOTE: Degradation Level 4-5 does NOT change lease state. Leases remain ACTIVE;
only the rendering pass is modified (see SS3.3, SS7.1).
```

### 3.2 State Descriptions

| State | Description | Agent Can Mutate | Tiles Rendered | Resources Held |
|-------|-------------|-----------------|----------------|----------------|
| `REQUESTED` | Lease request received; runtime evaluating | No | No (not yet created) | No |
| `ACTIVE` | Lease valid; agent holds mutation rights | Yes | Yes | Yes |
| `SUSPENDED` | Lease valid but mutations blocked; tiles frozen with staleness badge | No | Yes (frozen, stale-badged) | Yes (preserved) |
| `ORPHANED` | Session disconnected; within reconnect grace period | No | Yes (frozen; disconnection badge per RFC 0007 SS3.1) | Yes (preserved) |
| `REVOKED` | Runtime reclaimed lease; no recovery path | No | No (tiles removed) | No (freed) |
| `EXPIRED` | TTL elapsed without renewal | No | No (tiles removed) | No (freed) |
| `DENIED` | Lease request rejected | N/A | N/A | N/A |
| `RELEASED` | Agent voluntarily released lease | No | No (tiles removed) | No (freed) |

### 3.3 Transition Triggers

#### REQUESTED -> ACTIVE (grant)

The runtime evaluates:
1. Agent session is `Active` (RFC 0005 SS1.1).
2. Agent holds `create_tiles` capability (RFC 0005 SS7 vocabulary).
3. `agent.active_leases.len() < agent.max_active_leases` (SS9).
4. Requested capability scope is a subset of the agent's session-granted capabilities.
5. Requested `lease_priority` is within the agent's priority capability ceiling.
6. Requested `resource_budget` does not exceed the session's configured maximum (SS6.1).

If check (4) fails for any requested capability, the runtime denies the entire `LeaseRequest`; it does not clamp or partially grant the capability scope.

If all checks pass, the runtime assigns a `LeaseId` (SceneId, UUIDv7) and transitions to `ACTIVE`. Side effects:
- `LeaseResponse` with `result = GRANTED` sent to agent.
- `LeaseStateChange` emitted on `lease_changes` subscription category.
- Lease added to the runtime's lease registry.

**Latency requirement:** < 1ms from `LeaseRequest` receipt to `LeaseResponse` emission (local decision on compositor thread, no I/O).

#### REQUESTED -> DENIED

If any evaluation check fails. Terminal; the agent must submit a new `LeaseRequest` (possibly with reduced scope, priority, or budget).

Side effects:
- `LeaseResponse` with `result = DENIED` and populated `deny_reason` sent to agent.

#### ACTIVE -> SUSPENDED (safe mode)

Safe mode entry suspends leases without revoking them. See SS3.4 for the canonical resolution of the revoke/suspend contradiction.

On safe mode entry:
- All `ACTIVE` leases transition to `SUSPENDED`.
- Agent sessions are notified via `SessionSuspended` (RFC 0005 SS3.7) with `reason = "viewer_safe_mode"`.
- `LeaseSuspend` message sent for each lease (SS7.2).
- Agent mutations are rejected with `SAFE_MODE_ACTIVE` error (RFC 0005 SS3.5).
- TTL clock is paused (SS4.3).
- Auto-renewal is suspended.
- Staleness badge shown on all affected tiles (SS4.2).

**Latency requirement:** < 1 frame (16.6ms) from safe mode trigger to all leases in `SUSPENDED` state and staleness badge rendered.

#### SUSPENDED -> ACTIVE (safe mode exit)

On safe mode exit (explicit viewer action only -- RFC 0007 SS5.5):
- All `SUSPENDED` leases transition back to `ACTIVE`.
- Agent sessions receive `SessionResumed` (RFC 0005 SS3.7).
- `LeaseResume` message sent for each lease (SS7.2).
- Agent mutations are accepted again.
- Staleness badge cleared; tiles rendered from last-committed scene state.
- TTL clock resumes with adjusted effective elapsed time (SS4.3).
- Auto-renewal resumes if applicable.
- Any mutations queued by agents during suspension are applied.

**The agent does not re-request a lease after safe mode exit.** Lease identity, TTL (accounting for elapsed suspension time), and capability scope are preserved across the `ACTIVE -> SUSPENDED -> ACTIVE` cycle.

#### SUSPENDED -> REVOKED (max suspension time exceeded)

If a lease remains in `SUSPENDED` state beyond `max_suspension_time_ms` (configurable, default 300,000 ms / 5 minutes), it transitions to `REVOKED`. This prevents indefinite resource consumption by suspended leases.

Side effects:
- Tiles removed from scene graph.
- Resources freed.
- `LeaseResponse` with `result = REVOKED` and `revoke_reason = SUSPENSION_TIMEOUT` sent to agent.

#### ACTIVE -> ORPHANED (session disconnect)

When the agent's session transitions to `Closed` (RFC 0005 SS1.1) via ungraceful disconnect or graceful disconnect with `expect_resume = true`:
- The lease transitions to `ORPHANED`.
- The session reconnect grace period begins (`reconnect_grace_period_ms`, default 30,000 ms).
- Tiles are frozen at their last known state.
- The disconnection badge is shown per RFC 0007 SS3.1.
- Mutations are blocked (no session to receive them from).
- TTL clock continues running (unlike suspension, orphaned leases can expire).

If `expect_resume = false` in `SessionClose`, the runtime may accelerate cleanup (implementation-defined; grace period is a ceiling, not a floor).

#### ORPHANED -> ACTIVE (reconnect)

Agent reconnects before grace period expiry and presents a valid session token (RFC 0005 SS6.3). The lease is reclaimed; tiles resume their last-known state; the disconnection badge clears. See SS5 for full reconnect semantics.

#### ORPHANED -> EXPIRED (grace period elapsed)

Grace period expires without reconnect. Tiles are removed from the scene graph. Resources are freed. The agent must re-request a new lease and re-create its tiles.

#### ACTIVE -> EXPIRED (TTL elapsed)

When `effective_ttl_elapsed >= ttl_ms`:
1. Lease transitions to `EXPIRED`.
2. `LeaseResponse` with `result = EXPIRED` sent to the agent.
3. Tiles remain in scene graph for one additional frame (grace frame for visual continuity), then removed.
4. Resources freed after tile removal.

Agents should renew before expiry. Recommended practice: renew when `now > expires_at_us - (ttl_ms * 0.25)` (renew at 75% of TTL elapsed). For `AUTO_RENEW` leases, the runtime handles this automatically (SS1.4).

#### ACTIVE -> REVOKED (viewer dismisses tile)

When the viewer dismisses a specific tile (RFC 0007 SS4.1):
1. The tile's lease transitions to `REVOKED`.
2. The tile is removed from the scene graph.
3. `LeaseResponse` with `result = REVOKED` and `revoke_reason = VIEWER_DISMISSED` sent to agent.
4. No grace period; no reconnect path.

The agent may re-request a new lease. Viewer dismissal is not a permanent ban -- only a momentary choice (RFC 0007 SS4.1).

#### ACTIVE -> REVOKED (budget policy)

When budget enforcement reaches the Revocation tier (SS6.3):
1. All of the agent's leases transition to `REVOKED`.
2. `LeaseResponse` with `result = REVOKED` and `revoke_reason = BUDGET_POLICY` sent for each lease.
3. No grace period; session resumption window is bypassed (RFC 0002 SS5.2).
4. Agent-owned textures and node data are freed after the post-revocation delay (default 100ms).

#### ACTIVE -> RELEASED (agent releases)

Agent sends `LeaseRequest` with `operation = RELEASE`. Immediately terminal. Tiles are removed. Resources freed.

### 3.4 Canonical Resolution: Safe Mode -- Suspend, Not Revoke (DR-LG7)

**This section resolves the contradiction between RFC 0007 SS4.2 and SS5.2.**

RFC 0007 SS4.2 ("Dismiss All / Safe Mode") originally stated:
> "All active leases are revoked simultaneously."

RFC 0007 SS5.2 ("Safe Mode Behavior") states:
> "Safe mode does not terminate sessions by default. This is intentional: a viewer who accidentally entered safe mode should be able to resume without agents needing to reconnect and re-establish their leases."

These are contradictory. If leases are revoked (SS4.2), agents cannot resume without re-establishing leases (contradicting SS5.2). If leases survive as suspended (SS5.2), they are not revoked (contradicting SS4.2).

**Resolution (DR-LG7):** Safe mode **suspends** leases; it does not revoke them. RFC 0007 SS4.2's phrase "All active leases are revoked" is incorrect. The correct behavior is:

- On safe mode entry: all `ACTIVE` leases -> `SUSPENDED`.
- On safe mode exit: all `SUSPENDED` leases -> `ACTIVE` (TTL adjusted for suspension duration).
- **Rationale:** security.md SS"Human override" establishes that safe mode is a viewer override that must be quickly reversible. A viewer who accidentally triggers safe mode should not lose all agent state. Revocation is the appropriate response to *budget violations* and *malicious behavior*, not to the human pressing the emergency stop. The emergency stop is a *pause*, not a *purge*.

> **Doctrine:** "The human is always the ultimate authority. No agent, regardless of trust level or capability scope, can prevent the human from: [...] Entering a 'safe mode' that disconnects all agents." -- security.md SS"Human override"
>
> The word "disconnects" in the doctrine is interpreted as "disconnects from rendering and interaction," not "destroys lease state." The safe mode overlay replaces agent tiles (they are disconnected from the screen), but the leases are preserved.

RFC 0007 SS4.2 has been updated to read: "All active leases are suspended simultaneously" (corrected in RFC 0007 Round 3 review).

**Exception:** If the viewer explicitly selects "Disconnect Agents" (a separate control, distinct from "Dismiss All" / safe mode), the intent is revocation, not suspension. That control follows the per-tile dismiss flow (REVOKED path) applied to all agents.

---

## 4. Suspension Semantics

### 4.1 What Suspension Preserves

When a lease transitions to `SUSPENDED`, the following state is preserved in memory:

- **Tiles.** All tiles governed by the lease remain in the scene graph. Their geometry, z-order, and opacity are unchanged. They are not removed or re-created.
- **Node trees.** All nodes within each tile are preserved. Text content, image data, hit regions, and node hierarchy remain intact.
- **Zone publications.** All active zone publishes made under this lease remain in the zone registry. Content is marked stale but not cleared (see SS4.2).
- **Resource allocations.** Texture memory, uploaded images, and cached resources are not freed. The lease's resource budget remains reserved.
- **Lease metadata.** LeaseId, namespace, capability scope, resource budget, priority -- all preserved unchanged.

### 4.2 What Suspension Blocks

While a lease is `SUSPENDED`, the following operations are blocked:

- **New mutations.** Any `MutationBatch` targeting tiles under this lease is rejected with `SAFE_MODE_ACTIVE` error code (RFC 0005 SS3.5).
- **New zone publishes.** Zone publishes that require this lease's `publish_zone:*` capability are rejected. Existing publications remain visible but stale-badged.
- **Lease renewal.** `LeaseRequest` with `operation = RENEW` is accepted but has no effect (TTL clock is paused per SS4.3, so renewal is unnecessary -- but permitted for compatibility with agents that renew on a timer).
- **New lease requests.** `LeaseRequest` with `operation = REQUEST` for a new lease is denied with `DenyReason = SAFE_MODE_ACTIVE` while safe mode is active.

### 4.3 TTL Accounting During Suspension

When a lease is `SUSPENDED`, its TTL clock is paused. TTL advancement resumes when the lease transitions back to `ACTIVE`. This prevents a lease from expiring while the agent is unable to send renewals.

Formally: `effective_ttl_elapsed = sum of time spent in ACTIVE state`. The lease expires when `effective_ttl_elapsed >= ttl_ms`.

Implementation: the runtime records `suspended_at_us` on suspension entry and adds `(resumed_at_us - suspended_at_us)` to an accumulated `suspension_duration_us` on resume. The effective expiry is `granted_at_us + ttl_ms + suspension_duration_us`.

**Precision requirement:** TTL accounting during suspension must be accurate to +/- 100ms.

### 4.4 Suspension Visual Representation

When a lease is `SUSPENDED`:

- **Staleness badge.** Each tile governed by the lease displays a staleness badge (RFC 0007 SS3.2). The badge is rendered in the chrome layer, ensuring agents cannot occlude or interfere with it.
- **Zone publications.** Stale-badged. The zone's rendering policy appends a visual staleness indicator (reduced opacity, subtle "paused" icon) to the published content. Content remains visible but is visually marked as potentially stale.
- **Safe mode overlay.** During safe-mode suspension specifically, the safe mode overlay (RFC 0007 SS5.2) replaces normal tile rendering with neutral placeholders. The overlay displays "Session Paused" labels centered in the tile bounds.

### 4.5 Resume

When a lease transitions from `SUSPENDED` back to `ACTIVE`:

1. Staleness badges are cleared on all affected tiles (within 1 frame).
2. Zone publication staleness indicators are cleared.
3. Any mutations that agents submitted during suspension (which were rejected with `SAFE_MODE_ACTIVE`) must be re-submitted by the agent. The runtime does not queue rejected mutations.
4. The tile content is rendered from the last-committed scene state (preserved during suspension).
5. Auto-renewal resumes if the lease's `renewal_policy` is `AUTO_RENEW`.

**Latency requirement:** < 2 frames (33.2ms) from safe mode exit to full tile re-render with badges cleared.

### 4.6 Max Suspension Time

A lease may not remain `SUSPENDED` indefinitely. The configurable `max_suspension_time_ms` (default: 300,000 ms / 5 minutes) limits how long resources remain locked by suspended leases. If a lease is still `SUSPENDED` when this timer expires, it transitions to `REVOKED`.

This timer applies to all causes of suspension. If safe mode remains active beyond `max_suspension_time_ms`, suspended leases are progressively revoked (oldest first) to reclaim resources.

**Configuration parameter:** `max_suspension_time_ms` (SS10, config table).

---

## 5. Orphan/Disconnect Handling

### 5.1 Disconnect Detection

Agent disconnect is detected by:
- gRPC stream EOF or RST_STREAM
- Heartbeat timeout: missing `HeartbeatPing` after `heartbeat_missed_threshold x heartbeat_interval_ms` (default: `3 x 5000 ms = 15,000 ms`; RFC 0005 SS4)
- WebRTC ICE failure (embodied agents, post-v1)

### 5.2 Grace Period Entry

On disconnect detection:
1. All `ACTIVE` leases for the agent transition to `ORPHANED`.
2. The reconnect grace period timer starts (`reconnect_grace_period_ms`, default 30,000 ms).
3. Tiles are frozen at their last-committed scene state. No mutations can modify them.
4. The disconnection badge (RFC 0007 SS3.1) is shown on each affected tile within 1 frame.
5. TTL clock continues running during the grace period (unlike suspension). An orphaned lease can expire if its TTL elapses during the grace period.
6. Zone publications remain visible but their staleness is not visually indicated (the disconnection badge on tiles is sufficient signal).

### 5.3 During Grace Period

While a lease is `ORPHANED`:
- **Mutations blocked.** No session exists to send mutations from.
- **Tiles remain rendered.** Content is frozen at last known state. Disconnection badge displayed.
- **Resources held.** Texture memory, node data, and resource budget reservation are maintained.
- **Auto-renewal suspended.** No session to receive renewal confirmation.
- **TTL continues.** The lease can expire during the grace period, at which point it transitions to `EXPIRED` (not `ORPHANED -> EXPIRED` waits for grace period; if TTL expires first, the lease goes directly to `EXPIRED`).

### 5.4 Agent Reconnect During Grace Period

If the agent reconnects within the grace period (RFC 0005 SS6.3):
1. The agent presents its session token via `SessionResume`.
2. The runtime validates the token and accepts the resume.
3. All `ORPHANED` leases for the agent transition back to `ACTIVE`.
4. Disconnection badges are cleared on all affected tiles (within 1 frame).
5. The runtime sends a `SceneSnapshot` (RFC 0005 SS6.4) so the agent can synchronize state.
6. The agent may immediately submit mutations.
7. Auto-renewal resumes if applicable.

**Grace period precision:** +/- 100ms. The runtime must not prematurely expire the grace period.

### 5.5 Grace Period Expiry

If the agent does not reconnect before the grace period expires:
1. All `ORPHANED` leases transition to `EXPIRED`.
2. Tiles are removed from the scene graph.
3. Resources are freed (texture memory, node data).
4. The agent's session state is fully cleaned up.

If the agent reconnects after grace period expiry (RFC 0005 SS6.5):
1. The agent must authenticate as a new session (no resume).
2. No leases exist; the agent must request new leases.
3. The runtime sends a `SceneSnapshot` of the current scene topology so the agent can make informed lease requests.

### 5.6 Graceful Disconnect with expect_resume

When the agent sends `SessionClose` with `expect_resume = true` (RFC 0005 SS1.5):
- Leases transition to `ORPHANED` as with ungraceful disconnect.
- The full grace period applies.
- The agent is expected to reconnect and reclaim leases.

When the agent sends `SessionClose` with `expect_resume = false`:
- Leases transition to `ORPHANED`.
- The runtime **may** accelerate cleanup (implementation-defined; the grace period is a ceiling, not a floor). In practice, the runtime may immediately transition orphaned leases to `EXPIRED` if the agent explicitly signals it will not return.

---

## 6. Resource Budgets

### 6.1 Budget Dimensions

Every lease carries a `ResourceBudget` that governs its permitted resource consumption:

```rust
pub struct ResourceBudget {
    // Per-tile limits (enforced per tile at mutation time)
    pub texture_bytes_per_tile: u64,    // Max texture memory for a single tile's nodes
    pub max_nodes_per_tile: u32,        // Max node count in a tile's tree; [1, 64]
    pub update_rate_hz: f32,            // Max mutation rate for this lease (mutations/second)

    // Per-session aggregate limits
    pub max_tiles: u32,                 // Max concurrent tiles under this lease; [1, 64]
    pub texture_bytes_total: u64,       // Aggregate texture bytes across all lease tiles
    pub max_active_leases: u32,         // Max simultaneous leases per session; [1, 64]
    pub max_concurrent_streams: u32,    // Max simultaneous media streams (0 in v1; post-v1 only)
}
```

Default values match the platform profile defaults in RFC 0002 SS4.3. A requesting agent may request specific values; the runtime grants at or below the session's configured maximum.

**Relationship to RFC 0001 `ResourceBudget`:** RFC 0001 SS2.3 defines a per-tile `ResourceBudget` struct (`max_tiles`, `max_texture_bytes`, `max_update_rate_hz`, `max_nodes_per_tile`) carried on each `Tile`. This RFC's `ResourceBudget` is the lease-level envelope that governs aggregate session limits. The per-tile struct is a subset projection. Implementations must not conflate the two: the lease-level budget populates the per-tile defaults, but per-tile limits are checked independently at mutation time.

### 6.2 Budget Tracking

Budget usage is tracked in the scene graph per-agent and per-lease:

- **Texture memory:** Accumulated from all `StaticImageNode` data sizes across all tiles under the lease. Tracked incrementally on node creation/replacement/deletion.
- **Tile count:** Count of active tiles referencing this lease's `LeaseId`.
- **Node count per tile:** Count of nodes in each tile's tree.
- **Update rate:** Sliding window over the last 1 second, tracking `MutationBatch` arrivals attributed to this lease.

Budget tracking is aligned with the existing `ResourceBudget` struct in `tze_hud_scene::types`.

### 6.3 Overage Handling

Budget enforcement uses soft and hard limits to provide graduated feedback:

**Soft limit (warning) at 80%:**
When any budget dimension reaches 80% of its allocated maximum:
- A `BudgetWarning` event is sent to the agent via `LeaseStateChange` (SS7.3).
- A budget warning badge is rendered on affected tiles (border indicator per RFC 0007 SS3.5).
- The mutation is **accepted** -- soft limits do not reject work.
- The warning persists until usage drops below 80%.

**Hard limit (reject) at 100%:**
When a mutation would push any budget dimension to or beyond 100% of its allocated maximum:
- The mutation is **rejected** with a `BUDGET_EXCEEDED_*` error code (see SS6.4).
- The entire `MutationBatch` fails (atomic pipeline, RFC 0001 SS4).
- The agent must reduce usage or request a budget increase (post-v1 renegotiation).

### 6.4 Budget Enforcement Points

Budget checks occur at the following points in the mutation pipeline (RFC 0001 SS4):

| Stage | Check | Failure Action |
|-------|-------|---------------|
| Mutation Intake (Stage 3) | `update_rate_hz` -- sliding window | Batch rejected with `BUDGET_EXCEEDED_UPDATE_RATE` |
| Per-mutation Validation | `max_nodes_per_tile` -- for `InsertNode` / `ReplaceNode` | Mutation rejected with `BUDGET_EXCEEDED_NODE_COUNT` |
| Per-mutation Validation | `texture_bytes_per_tile` -- for node creation with texture content | Mutation rejected with `BUDGET_EXCEEDED_TEXTURE_BYTES` |
| Per-mutation Validation | `max_tiles` -- for `CreateTile` | Mutation rejected with `BUDGET_EXCEEDED_TILE_COUNT` |
| Per-mutation Validation | `texture_bytes_total` -- aggregate across all tiles | Mutation rejected with `BUDGET_EXCEEDED_TEXTURE_TOTAL` |
| Lease Request | `max_active_leases` -- checked before granting new lease | Lease denied with `LEASE_DENIED_MAX_LEASES` |

Budget checks are all-or-nothing within a batch (RFC 0001 SS4 -- atomic pipeline). Per-mutation budget enforcement overhead must be < 50us per check.

### 6.5 Three-Tier Enforcement Ladder

Per-agent budget enforcement operates as a three-tier ladder (authoritative definition from RFC 0002 SS5.2, reproduced and extended here):

| Tier | Trigger | Duration | Action |
|------|---------|----------|--------|
| **Warning** | Any per-lease budget at >= 80% (soft limit) | -- | Send `BudgetWarning` event; render warning badge on tiles |
| **Throttle** | Warning unresolved for 5 seconds | Until resolved | Coalesce updates more aggressively; reduce effective `update_rate_hz` by 50% |
| **Revocation** | Throttle sustained for 30 seconds, or critical limit exceeded | Immediate | Revoke all leases; terminate session (see SS6.6) |

Critical triggers that **bypass** the ladder and go directly to Revocation:
- Attempt to allocate texture memory exceeding the session hard maximum (`CriticalTextureOomAttempt`).
- Repeated invariant violations (> 10 in a session lifetime, `RepeatedInvariantViolations`).
- Protocol violations indicating malicious intent (e.g., forged session IDs).

### 6.6 Resource Cleanup on Revocation

When a session is revoked (budget policy or critical trigger), the compositor thread executes on the same frame tick:

1. Move `BudgetState` to `Revoked`.
2. Transition all session leases from `ACTIVE` -> `REVOKED` (state machine SS3.3).
3. Send `LeaseResponse` with `result = REVOKED` and `revoke_reason = BUDGET_POLICY` for all active leases.
4. Mark all agent-owned tiles for immediate removal from the scene graph.
5. **No grace period.** Unlike unexpected disconnects, policy-driven revocations bypass the session resumption window (RFC 0005 SS6.3).
6. After post-revocation delay (default 100ms, to allow `LeaseResponse` delivery), free all agent-owned textures and node data. Reference counts reach zero; resources are released.
7. Remove `AgentResourceState` from the compositor's per-agent table.

Post-revocation resource footprint must be zero (architecture.md SS"Resource lifecycle"). Verified by the `disconnect_reclaim_multiagent` test scene.

### 6.7 Budget Inheritance for Zone Publishes

Zone publications made under a lease count against the publisher's lease budget:

- **Texture bytes:** If a zone publish includes image content (e.g., a `StaticImage` published to an `ambient-background` zone), the image data size counts against the publisher's `texture_bytes_total`.
- **Update rate:** Each zone publish operation counts as one mutation for `update_rate_hz` tracking.
- **Node count:** Does not apply directly (zone tiles are runtime-owned), but the runtime may impose a per-zone node limit as part of the zone definition.

Guest zone publishes (via MCP `ZonePublishToken`) do not count against any agent's lease budget because guests do not hold leases (presence.md SS"Guest agents and zone leases"). The runtime absorbs the resource cost.

---

## 7. Lease-Zone Relationship

### 7.1 Zone Publish Requires Active Lease

For resident agents, publishing to a zone requires:
1. An `ACTIVE` lease (not `SUSPENDED`, `ORPHANED`, or any terminal state).
2. The lease must include the `publish_zone:<zone_type>` capability for the target zone's type. For example, publishing to a `subtitle` zone instance requires `publish_zone:subtitle` capability.
3. The zone instance must exist in the current active tab (or be a global zone).

If any condition is not met, the publish is rejected with a structured error:
- Missing lease: `LEASE_NOT_FOUND`
- Lease not active: `LEASE_NOT_ACTIVE`
- Missing capability: `CAPABILITY_NOT_GRANTED`
- Zone not found: `ZONE_NOT_FOUND`

### 7.2 Lease Suspension Freezes Zone Publications

When a lease is `SUSPENDED`:
- All active zone publications made under that lease remain in the zone registry.
- Content remains visible but is visually marked as stale (reduced opacity, staleness indicator per the zone's rendering policy).
- New zone publishes under the suspended lease are rejected with `SAFE_MODE_ACTIVE`.
- Other agents may still publish to the same zone (their leases are independent). Zone contention policy (RFC 0001 SS2.5) applies across leases normally.

### 7.3 Lease Revocation Clears Zone Publications

When a lease is `REVOKED` or `EXPIRED`:
- All active zone publications made under that lease are immediately cleared from the zone registry.
- The zone's contention policy governs what fills the vacated space: for `latest-wins` zones, nothing is shown until a new publish; for `stack` zones, the next item in the stack becomes visible; for `merge-by-key` zones, the key is removed.
- Resources associated with the cleared publications are freed.

### 7.4 Zone Contention Applies Across Leases

Zone contention policy (RFC 0001 SS2.5) operates across leases, not within a single lease:
- Two agents with separate leases publishing to a `latest-wins` zone compete under the zone's contention policy.
- Two tiles under the same lease publishing to the same zone also compete under the same policy.
- The contention policy is zone-defined, not lease-defined. Agents do not choose contention behavior.

### 7.5 Guest Agents and Zone Leases

Guest agents publishing via MCP `ZonePublishToken` do not hold leases and are not subject to lease-zone governance. Their publications are governed solely by the zone's timeout and cleanup policy (failure.md SS"Zone content during agent failure"). The `ZonePublishToken` serves as the authorization mechanism, replacing the lease's capability scope for this specific operation.

---

## 8. Lease Renewal

### 8.1 When to Renew

Agents with leases that have a finite TTL and `MANUAL` renewal policy should renew before expiry. Recommended practice: renew when `now > expires_at_us - (ttl_ms * 0.25)` (renew at 75% of TTL elapsed).

The runtime does not proactively warn agents about impending expiry -- it is the agent's responsibility. A lease that expires without renewal transitions to `EXPIRED`, and the agent must re-request a new lease and re-create its tiles.

For `AUTO_RENEW` leases, the runtime handles renewal automatically at 75% TTL elapsed (SS1.4).

### 8.2 Renewal Semantics

A `LeaseRequest` with `operation = RENEW` and a valid, non-expired `lease_id`:
- Resets the TTL clock (`granted_at_us = now`).
- May grant a different TTL than requested (runtime policy).
- Does **not** change the lease ID (continuity is preserved).
- Does **not** affect tiles, capability scope, or resource budget unless the agent also requests changes (not supported in v1 -- budget and scope changes require a new lease).

A `LeaseRequest` with `operation = RENEW` on an `EXPIRED` lease is rejected with `DenyReason = LEASE_ALREADY_EXPIRED`. The agent must request a new lease.

### 8.3 Indefinite Leases

`ttl_ms = 0` requests an indefinite lease. The runtime may grant indefinite leases to trusted sessions (embodied agents, long-running resident agents). Indefinite leases survive session reconnects within the grace period but are invalidated on policy-driven revocation or session termination.

Indefinite leases are always `MANUAL` renewal policy (no auto-renewal needed; TTL never elapses).

---

## 9. Lease Caps and Scene-Level Limits

| Dimension | Default Cap | Hard Max | Notes |
|-----------|-------------|----------|-------|
| `max_active_leases` per session | 8 | 64 | RFC 0002 SS4.3 |
| Leases per runtime (all agents) | 64 (total) | 64 | System-wide ceiling |
| `max_tiles` per lease | 8 | 64 | Configured per session |
| `max_nodes_per_tile` | 32 | 64 | RFC 0001 SS2.3 |
| `texture_bytes_per_tile` | 16 MiB | -- | Platform-dependent |
| `texture_bytes_total` per session | 256 MiB | 2 GiB | RFC 0002 SS4.3 |
| `update_rate_hz` per lease | 30 | 120 | RFC 0002 SS4.3 |

Requests exceeding the hard max are rejected with structured errors. Values between the default and hard max are negotiated at session establishment.

---

## 10. Wire Protocol

### 10.1 Transport

All lease messages flow over the primary session stream (RFC 0005 SS2.1), not as separate RPCs. This is consistent with the "one session stream per agent" architecture (architecture.md SS"Session model: one stream per agent"). Lease messages are multiplexed alongside mutations, events, heartbeats, and telemetry within the `SessionMessage` envelope.

### 10.2 Lease Messages

The following lease-specific messages are defined. `LeaseRequest` and `LeaseResponse` are carried in the existing `SessionMessage` oneof fields (RFC 0005 SS3.1, SS3.2, fields 21 and 31). `LeaseSuspend`, `LeaseResume`, and `LeaseStateChange` are new server-to-client messages defined here.

#### LeaseRequest (Client -> Server)

```protobuf
message LeaseRequest {
  enum Operation {
    OPERATION_UNSPECIFIED = 0;
    REQUEST               = 1;   // Request a new lease
    RENEW                 = 2;   // Renew an existing lease before expiry
    RELEASE               = 3;   // Voluntarily release a lease
    EXTEND                = 4;   // Request TTL extension mid-lease (may be denied)
  }

  enum RenewalPolicy {
    RENEWAL_POLICY_UNSPECIFIED = 0;
    MANUAL                     = 1;   // Agent must explicitly renew (default)
    AUTO_RENEW                 = 2;   // Runtime auto-renews at 75% TTL
    ONE_SHOT                   = 3;   // No renewal; expires at TTL
  }

  Operation        operation         = 1;   // Required
  SceneId          lease_id          = 2;   // Required for RENEW/RELEASE/EXTEND; nil for REQUEST
  string           namespace         = 3;   // Requested namespace; must match session auth identity
  uint32           ttl_ms            = 4;   // Requested TTL; 0 = indefinite (runtime may cap)
  uint32           lease_priority    = 5;   // Requested priority [0-4]; runtime grants at/below capability ceiling
  ResourceBudget   resource_budget   = 6;   // Requested budget; runtime grants at/below session maximum
  repeated string  capability_scope  = 7;   // Requested capabilities; subset of session-granted capabilities
  RenewalPolicy    renewal_policy    = 8;   // Requested renewal behavior (default: MANUAL)
}
```

#### LeaseResponse (Server -> Client)

```protobuf
message LeaseResponse {
  enum Result {
    RESULT_UNSPECIFIED            = 0;
    GRANTED                       = 1;   // New lease or renewal accepted
    DENIED                        = 2;   // Request refused; see deny_reason
    REVOKED                       = 3;   // Runtime reclaimed an active lease
    EXPIRED                       = 4;   // Lease TTL elapsed
    RELEASED                      = 5;   // Release acknowledged
    EXTENDED                      = 6;   // TTL extension granted
  }

  enum RevokeReason {
    REVOKE_REASON_UNSPECIFIED     = 0;
    VIEWER_DISMISSED              = 1;   // Human dismissed the tile (RFC 0007 SS4.1)
    BUDGET_POLICY                 = 2;   // Budget enforcement escalation (SS6.5)
    CRITICAL_VIOLATION            = 3;   // Protocol or invariant violation (SS6.5)
    SESSION_TERMINATED            = 4;   // Parent session revoked (implies lease revocation)
    RUNTIME_SHUTDOWN              = 5;   // Orderly runtime shutdown (RFC 0002 SS1.4)
    SUSPENSION_TIMEOUT            = 6;   // Lease exceeded max_suspension_time_ms while SUSPENDED
  }

  enum DenyReason {
    DENY_REASON_UNSPECIFIED       = 0;
    INSUFFICIENT_CAPABILITY       = 1;   // Agent lacks required capability scope
    MAX_LEASES_EXCEEDED           = 2;   // Agent at max_active_leases limit
    PRIORITY_NOT_PERMITTED        = 3;   // Requested priority above capability ceiling
    BUDGET_EXCEEDS_SESSION_MAX    = 4;   // Requested budget above session maximum
    SESSION_NOT_ACTIVE            = 5;   // Session not in Active state
    LEASE_ALREADY_EXPIRED         = 6;   // Renew/extend on an already-expired lease
    SAFE_MODE_ACTIVE              = 7;   // New lease requests rejected while in safe mode
    MAX_RUNTIME_LEASES_EXCEEDED   = 8;   // Runtime-wide lease limit reached (64)
  }

  SceneId        lease_id          = 1;   // Granted LeaseId; populated for GRANTED/EXTENDED
  Result         result            = 2;   // Required
  RevokeReason   revoke_reason     = 3;   // Populated if result = REVOKED
  DenyReason     deny_reason       = 4;   // Populated if result = DENIED
  uint32         granted_ttl_ms    = 5;   // Actual TTL granted (may differ from requested)
  uint32         granted_priority  = 6;   // Actual priority granted
  ResourceBudget granted_budget    = 7;   // Actual budget granted
  uint64         expires_at_us     = 8;   // UTC us; RFC 0003 SS3.1; 0 if TTL is indefinite
  string         message           = 9;   // Human-readable detail (for logs/debug)
}
```

#### LeaseSuspend (Server -> Client)

Sent when a lease transitions to `SUSPENDED`. Delivered on the `lease_changes` subscription category (unconditionally subscribed, RFC 0005 SS7.1).

```protobuf
message LeaseSuspend {
  SceneId  lease_id  = 1;   // Lease being suspended
  string   reason    = 2;   // "safe_mode", "degradation_emergency", etc.
  uint64   max_suspension_time_ms = 3;  // After this duration, lease will be auto-revoked
}
```

#### LeaseResume (Server -> Client)

Sent when a lease transitions from `SUSPENDED` back to `ACTIVE`. Delivered on the `lease_changes` subscription category.

```protobuf
message LeaseResume {
  SceneId  lease_id            = 1;   // Lease being resumed
  uint64   adjusted_expires_at_us = 2;  // New expiry time (adjusted for suspension duration)
  uint64   suspension_duration_us = 3;  // How long the lease was suspended
}
```

#### LeaseStateChange (Server -> Client)

General-purpose lease state notification. Delivered on the `lease_changes` subscription category.

```protobuf
message LeaseStateChange {
  SceneId    lease_id      = 1;   // Affected lease
  LeaseState previous_state = 2;  // State before transition
  LeaseState new_state     = 3;   // State after transition
  string     reason        = 4;   // Human-readable reason for the transition
  uint64     timestamp_wall_us = 5; // When the transition occurred (UTC us)
}
```

### 10.3 Lease State Enum

Used in `LeaseStateChange` and in `LeaseStateRecord` (telemetry/snapshot).

```protobuf
enum LeaseState {
  LEASE_STATE_UNSPECIFIED = 0;
  REQUESTED               = 1;
  ACTIVE                  = 2;
  SUSPENDED               = 3;
  ORPHANED                = 4;
  REVOKED                 = 5;
  EXPIRED                 = 6;
  DENIED                  = 7;
  RELEASED                = 8;
}
```

### 10.4 Resource Budget (protobuf)

```protobuf
message ResourceBudget {
  uint64 texture_bytes_per_tile  = 1;   // Max texture bytes for a single tile
  uint32 max_nodes_per_tile      = 2;   // Max nodes in tile tree [1, 64]
  float  update_rate_hz          = 3;   // Max mutations/second for this lease
  uint32 max_tiles               = 4;   // Max concurrent tiles [1, 64]
  uint64 texture_bytes_total     = 5;   // Aggregate texture bytes across all tiles
  uint32 max_active_leases       = 6;   // Max simultaneous leases per session [1, 64]
  uint32 max_concurrent_streams  = 7;   // Max media streams; 0 in v1 (post-v1 only)
}
```

### 10.5 Lease State Record (telemetry/snapshot)

```protobuf
message LeaseStateRecord {
  SceneId     lease_id       = 1;
  string      namespace      = 2;
  string      session_id     = 3;
  LeaseState  state          = 4;
  uint64      granted_at_us  = 5;   // UTC us; RFC 0003 SS3.1
  uint64      expires_at_us  = 6;   // UTC us; 0 = indefinite
  uint32      priority       = 7;
  ResourceBudget budget      = 8;
  repeated string capability_scope = 9;
  LeaseRequest.RenewalPolicy renewal_policy = 10;
  uint64      suspension_duration_us = 11;  // Accumulated time spent suspended
}
```

### 10.6 SessionMessage Field Allocation

The following fields are allocated (or reserved) in the `SessionMessage` oneof for lease-related messages. Existing allocations from RFC 0005:

| Field Number | Direction | Message |
|-------------|-----------|---------|
| 21 | Client -> Server | `LeaseRequest` |
| 31 | Server -> Client | `LeaseResponse` |

New allocations required by this RFC:

| Field Number | Direction | Message |
|-------------|-----------|---------|
| 47 | Server -> Client | `LeaseSuspend` |
| 48 | Server -> Client | `LeaseResume` |
| 49 | Server -> Client | `LeaseStateChange` |

These field numbers continue the server-to-client allocation block (fields 30-46 are allocated by RFC 0005; 45-46 are `SessionSuspended`/`SessionResumed`). See RFC 0005 SS9.2 for the field registry.

### 10.7 Proto Package and Imports

```protobuf
syntax = "proto3";
package tze.lease.v1;

import "scene.proto";  // SceneId

// All messages defined in SS10.2-10.5 above belong to this package.
// LeaseRequest and LeaseResponse are ALSO imported into session.proto
// (tze_hud.protocol.v1) for embedding in the SessionMessage oneof.
```

---

## 11. Configuration Parameters

| Parameter | Default | Description |
|-----------|---------|-------------|
| `reconnect_grace_period_ms` | 30,000 | How long orphaned leases are held after disconnect. Config file key: `reconnect_grace_secs` (RFC 0006 SS2.2). |
| `max_suspension_time_ms` | 300,000 (5 min) | Maximum time a lease may remain SUSPENDED before auto-revoke. |
| `post_revocation_delay_ms` | 100 | Delay between lease revocation event delivery and resource freeing. |
| `budget_warning_threshold_pct` | 80 | Percentage of budget usage that triggers a soft warning. |
| `budget_throttle_escalation_s` | 5 | Seconds after warning before throttle escalation. |
| `budget_revocation_escalation_s` | 30 | Seconds after throttle before revocation escalation. |
| `max_active_leases_per_session` | 8 | Default max leases per agent session. |
| `max_leases_per_runtime` | 64 | System-wide maximum leases across all agents. |
| `auto_renew_threshold_pct` | 75 | Percentage of TTL elapsed before auto-renewal triggers. |

---

## 12. Quantitative Requirements Summary

| Metric | Requirement | Notes |
|--------|-------------|-------|
| Lease grant latency | < 1ms from `LeaseRequest` receipt | On compositor thread; no I/O |
| Lease revocation latency | < 1 frame (16.6ms) | Revocation enqueued and processed within the current frame tick |
| Safe mode suspend latency | < 1 frame (16.6ms) | All leases transitioned to `SUSPENDED` and staleness badge rendered within one frame of safe mode entry |
| Safe mode resume latency | < 2 frames (33.2ms) | Placeholder removal and tile re-render within two frames of exit |
| Budget enforcement overhead | < 50us per mutation check | Per-mutation budget check must not dominate mutation pipeline |
| Post-revocation resource freedom | 0 bytes after 100ms + 1 frame | All textures and node data freed |
| `LeaseResponse` delivery | Within post-revocation delay (default 100ms) | Before resource free |
| Orphan grace period precision | +/- 100ms | Runtime must not prematurely expire the grace period |
| TTL accounting precision | +/- 100ms | Suspension time correctly subtracted from effective elapsed TTL |
| Max leases per runtime | 64 | System-wide ceiling across all agents |
| Max leases per agent | 8 (default), 64 (hard max) | Per-session, negotiated at establishment |
| Disconnection badge response | < 1 frame (16.6ms) | Badge rendered within one frame of disconnect detection |

---

## 13. Degradation and Lease Interaction

### 13.1 Degradation Ladder vs. Lease States

The degradation ladder (RFC 0002 SS6) affects rendering but does not change lease states.

| Degradation Level | Lease State Impact |
|-------------------|-------------------|
| Level 0 (Normal) | No change |
| Level 1 (Coalesce) | No change -- update coalescing is a transport-layer effect |
| Level 2 (Reduce Texture Quality) | No change -- texture downsampling applied in render pass |
| Level 3 (Disable Transparency) | No change -- alpha blending disabled in render pass |
| Level 4 (Shed Tiles) | No change -- tiles removed from render pass; leases remain `ACTIVE`; tiles remain in scene graph |
| Level 5 (Emergency) | No lease state change (leases remain `ACTIVE`); only the highest-priority tile is rendered; all other agent tiles are visually suppressed |

**Key principle:** Degradation is a rendering concern, not a governance concern. Leases are governance. The degradation ladder never invalidates a lease.

> **Doctrine:** "Graceful degradation is mentioned throughout this project's doctrine. [...] The runtime chooses the degradation level based on current resource pressure and tile priorities. It does not ask the user -- it acts, and the user can override." -- failure.md SS"Core principle"

### 13.2 Priority Shedding at Level 4 and 5

At Level 4 ("Shed Tiles"), the compositor sorts tiles by `(lease_priority ASC, z_order DESC)` and removes the lowest-priority group from the render pass (approximately 25% of active tiles per application of Level 4). "Removed" means: not encoded in the render pass for that frame. The tile remains in the scene graph; its lease is valid; its content is preserved.

At Level 5 ("Emergency"), only the highest-priority single tile (lowest `lease_priority` value, then highest `z_order`) plus the chrome layer is rendered. All others are visually suppressed, but their leases remain `ACTIVE`.

Agents are notified of degradation via `DegradationNotice` (RFC 0005 SS3.4) with the current level. Agents should reduce their update rate and content complexity in response.

### 13.3 Recovery

Recovery from Level 5 back to Normal requires `frame_time_p95 < 12ms` over a 30-frame window, progressing one level at a time (RFC 0002 SS6.3 hysteresis). At each recovery step, previously-shed tiles are re-included in the render pass in priority order.

---

## 14. Cross-RFC Resolution

This section explicitly states which contradictions and ambiguities across the RFC set are resolved by this document.

### 14.1 RFC 0007 SS4.2 -- Safe Mode: Suspend, Not Revoke

**Contradiction:** RFC 0007 SS4.2 said "All active leases are revoked simultaneously." RFC 0007 SS5.2 said agents should be able to "resume without [...] re-establishing their leases."

**Resolution:** Safe mode **suspends** leases (SS3.4). RFC 0007 SS4.2 has been corrected to "All active leases are suspended simultaneously" (RFC 0007 Round 3 review, rig-5vq.13). This RFC is authoritative.

### 14.2 RFC 0001 SS3.3 -- Lease Validity Definition

**Ambiguity:** RFC 0001 SS3.3 references `lease_registry.get(T.lease_id).is_valid()` without defining which lease states are "valid."

**Resolution:** A lease is valid for mutation purposes if and only if its state is `ACTIVE`. All other states (`REQUESTED`, `SUSPENDED`, `ORPHANED`, `REVOKED`, `EXPIRED`, `DENIED`, `RELEASED`) fail the validity check. See SS3.2 state table.

### 14.3 RFC 0002 SS5.2 -- Sort Direction Phrasing

**Ambiguity:** RFC 0002 SS5.2 uses "`(lease_priority DESC, z_order DESC)`" which is confusing because "DESC by lease_priority value" means "lower value first" which is actually ASC order for the value.

**Resolution:** This RFC uses the unambiguous formulation: `(lease_priority ASC, z_order DESC)` where lower `lease_priority` values (higher importance) sort first. No behavioral change; only clearer description. See SS2.2.

### 14.4 RFC 0002 SS6.2 -- Level 5 Degradation and Lease State

**Ambiguity:** RFC 0002 SS6.2 Level 5 states "tiles suspended (not revoked -- leases remain valid)" which could be confused with the `SUSPENDED` lease state.

**Resolution:** Degradation Level 5 is a **rendering-only** suspension. Leases remain `ACTIVE` -- agents may continue sending mutations. The lease state does not change. The `SUSPENDED` lease state is reserved for safe mode and similar governance actions. See SS13.1.

### 14.5 RFC 0005 SS3.2 -- Missing SessionSuspended/SessionResumed

**Gap:** RFC 0005 did not originally include `SessionSuspended` or `SessionResumed` messages, creating a protocol gap for safe mode behavior.

**Resolution:** These messages were added in RFC 0005 Round 12 (fields 45-46) to close the gap flagged in RFC 0007 SS8. This RFC defines the lease-level messages (`LeaseSuspend`, `LeaseResume`) that complement the session-level ones.

---

## 15. Open Questions

1. **Budget renegotiation mid-lease.** V1 does not support changing a lease's `resource_budget` or `capability_scope` after grant (requires `RELEASE` + `REQUEST`). A `RENEGOTIATE` operation is a post-v1 candidate. Relevant for long-lived embodied sessions that need to acquire media stream capabilities after initial auth.

2. **Multi-lease atomic operations.** An agent may want to atomically swap two tiles across two leases (release one, acquire another in the same scene update). The current model requires two separate `LeaseRequest` messages. A batch-lease operation is a post-v1 consideration.

3. **Lease inheritance across tabs.** Currently a lease governs tiles within any tab in the agent's namespace. Should leases be tab-scoped? Tab-scoped leases would simplify tab teardown but add complexity for agents that span multiple tabs (e.g., a clock widget visible in all tabs). Defer to post-v1 design.

4. **Grace period acceleration on explicit release.** RFC 0005 SS1.5 notes the runtime "may accelerate cleanup" when `expect_resume = false`. The exact acceleration semantics are implementation-defined. A future revision should specify the minimum observable grace period.

---

## 16. Diagrams

### 16.1 Lease State Machine

```
                             ┌──────────┐
                             │ REQUESTED│
                             └────┬─────┘
                           grant/ │ \denied
                                 │  \
                                 v   v
                           ┌──────┐ ┌──────┐
                      ┌────│ACTIVE│ │DENIED│
                      │    └──┬───┘ └──────┘
                      │       │
          ┌───────────┼───────┼───────────────┐
          │           │       │               │
     safe_mode    disconnect  │      TTL expired/
     entry        (ungraceful)│      viewer_dismissed/
          │           │       │      budget_policy/
          v           v       │      released
    ┌─────────┐ ┌─────────┐  │           │
    │SUSPENDED│ │ORPHANED │  │           v
    └────┬────┘ └────┬────┘  │    ┌────────────┐
         │           │       │    │  REVOKED/   │
    safe_mode   reconnect    │    │  EXPIRED/   │
    exit    (within grace)   │    │  RELEASED   │
         │           │       │    │ (terminal)  │
         v           v       │    └────────────┘
    ┌──────┐   ┌──────┐     │
    │ACTIVE│   │ACTIVE│     │
    └──────┘   └──────┘     │
         │                   │
    max_suspension      grace_period
    time exceeded       expires
         │                   │
         v                   v
    ┌──────┐           ┌───────┐
    │REVOKED│          │EXPIRED│
    └──────┘           └───────┘
```

### 16.2 Disconnect/Reconnect Timeline

```
Time ──────────────────────────────────────────────────────────────────►

Agent              Runtime                 Display
  │                  │                       │
  │──[disconnect]──► │                       │
  │                  │──[detect: ~15s]──►    │
  │                  │  lease → ORPHANED     │──[disconnection badge]
  │                  │  grace timer starts   │  tiles frozen
  │                  │                       │
  │  ... grace period (default 30s) ...      │
  │                  │                       │
  │──[reconnect]───► │                       │
  │  SessionResume   │  validate token       │
  │                  │  lease → ACTIVE       │──[badge cleared]
  │ ◄─SceneSnapshot─ │                       │  tiles unfrozen
  │                  │                       │
  │──[mutations]───► │  apply normally       │──[render updates]

         --- OR (grace period expires) ---

  │                  │  grace timer fires    │
  │                  │  lease → EXPIRED      │──[tiles removed]
  │                  │  resources freed      │  space reclaimed
```

### 16.3 Safe Mode Suspension Timeline

```
Time ──────────────────────────────────────────────────────────────────►

Agent              Runtime                 Display
  │                  │                       │
  │                  │◄─[viewer: safe mode]  │
  │                  │  leases → SUSPENDED   │
  │◄─SessionSuspend─ │  TTL clocks paused   │──[safe mode overlay]
  │◄─LeaseSuspend──  │                       │  tiles replaced
  │                  │                       │  with placeholders
  │                  │                       │
  │──[mutation]────► │  REJECT: SAFE_MODE   │
  │◄─MutationResult─ │  ACTIVE              │
  │                  │                       │
  │  ... safe mode active ...                │
  │                  │                       │
  │                  │◄─[viewer: resume]     │
  │                  │  leases → ACTIVE      │
  │◄─SessionResumed─ │  TTL clocks resume   │──[overlay removed]
  │◄─LeaseResume───  │                       │  tiles restored
  │                  │                       │
  │──[mutation]────► │  apply normally       │──[render updates]
```

---

## 17. Cross-RFC Errata

This section records the corrections to other RFCs that this document mandates. Each affected RFC must be updated to align with this document at its next revision pass.

### RFC 0007 SS4.2 -- Dismiss All / Safe Mode

**Original text:**
> "All active leases are revoked simultaneously."

**Corrected text:**
> "All active leases are suspended simultaneously. Sessions are not terminated; agents receive `SessionSuspended`. Leases transition to `SUSPENDED` state (RFC 0008 SS3.3) and are restored to `ACTIVE` on safe mode exit without requiring the agent to re-request a lease."

**Status:** Applied in RFC 0007 Round 3 review (rig-5vq.13).

### RFC 0001 SS3.3 -- Lease Check

**Clarification:** A lease is valid for mutation purposes if its state is `ACTIVE`. `SUSPENDED`, `ORPHANED`, `REVOKED`, `EXPIRED`, and `RELEASED` leases all fail the validity check.

**Status:** Cross-reference added in RFC 0001 Round 4 review.

### RFC 0002 SS5.2 -- Frame-Time Guardian Sort

**Clarification:** No behavioral change; only clearer description. The sort `(lease_priority ASC, z_order DESC)` means: tiles with lower `lease_priority` values (higher importance) are preserved; within the same priority class, tiles with higher `z_order` values (rendered on top) are preserved.

### RFC 0005 SS3.2 -- Server -> Client Messages

**Required addition:** `LeaseSuspend` (field 47), `LeaseResume` (field 48), and `LeaseStateChange` (field 49) must be added to the `SessionMessage` oneof in the server-to-client direction. These complement the existing `SessionSuspended` (field 45) and `SessionResumed` (field 46) with per-lease granularity.

---

## 18. Related RFCs

| RFC | Topic | Relationship |
|-----|-------|-------------|
| RFC 0001 (Scene Contract) | Scene data model, mutation pipeline, tile structure | Lease validity enforced in mutation pipeline SS3.3; `Tile.lease_id` references leases defined here. Per-tile `ResourceBudget` is a projection of the lease-level budget. |
| RFC 0002 (Runtime Kernel) | Budget enforcement tiers, degradation ladder, admission control | Budget enforcement ladder (SS5) and tile shedding priority (SS6.2) are authoritative inputs to this RFC. Sort semantics clarified here. |
| RFC 0003 (Timing Model) | Timestamp semantics | All `*_at_us` fields in lease messages use RFC 0003 SS3.1 UTC microsecond timestamps. |
| RFC 0005 (Session Protocol) | Session lifecycle, `LeaseRequest`/`LeaseResponse` wire format | Session lifecycle affects lease states per SS6.1; `SessionSuspended`/`SessionResumed` messages required by SS6.2. New lease messages (SS10.6) extend the `SessionMessage` oneof. |
| RFC 0006 (Configuration) | Capability name vocabulary | Capability names in lease scope use the canonical `snake_case` scheme from RFC 0006 SS6.3. |
| RFC 0007 (System Shell) | Safe mode, tile dismiss, override controls | SS3.4 resolves the revoke/suspend contradiction; SS17 errata applied. |

---

## 19. Review Record

| Round | Date | Reviewer | Focus | Changes |
|-------|------|----------|-------|---------|
| A1 | 2026-04-19 | hud-ora8.1.11 | Amendment: C13 capability dialog + 7-day remember | Added capability taxonomy for 8 C13 capabilities: `media-ingress`, `microphone-ingress`, `audio-emit`, `recording`, `cloud-relay`, `external-transcode`, `federated-send`, `agent-to-agent-media`. Added per-session operator dialog flow (first-use gate within `REQUESTED` state; session capability cache). Added 7-day per-agent-per-capability remember: `CapabilityRememberRecord` data model, 7-day expiry, operator-revocable at any time, persistent local store. Inserted capability dialog as evaluation step 0 before the existing 6-step `REQUESTED -> ACTIVE` sequence. Added role-based interaction with RFC 0009 A1: `owner`/`admin` may grant; `member`/`guest` cannot; dialog times out if no authorized operator present. Added 3 audit events (`capability_granted`, `capability_remembered`, `capability_revoked`) per C17. Added 4 new `DenyReason` values (9–12) and 1 new `RevokeReason` value (7). Added `capability_dialog_timeout_ms` config parameter (default 30 s). Cross-referenced identity-and-roles capability spec (hud-ora8.2.5). §15 open question 1 partially addressed (scope narrowing via revocation is now defined). Full amendment document: `about/legends-and-lore/rfcs/reviews/0008-amendment-c13-capability-dialog.md` (issue hud-ora8.1.11). |
