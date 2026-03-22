# RFC 0008: Lease and Resource Governance

**Status:** Draft
**Issue:** rig-lqp
**Date:** 2026-03-22
**Authors:** tze_hud architecture team
**Depends on:** RFC 0001 (Scene Contract), RFC 0002 (Runtime Kernel), RFC 0005 (Session Protocol), RFC 0007 (System Shell)

---

## Summary

This RFC consolidates lease lifecycle, resource governance, and budget enforcement into a single authoritative specification. Lease concepts are referenced throughout RFC 0001, RFC 0002, RFC 0005, and RFC 0007, but no single document defines the complete state machine, assignment semantics, safe-mode behavior, or enforcement ladder. This RFC is that document.

All other RFCs defer to this document on lease lifecycle questions. Contradictions between this RFC and other RFCs are resolved here; those RFCs must be updated to align.

---

## Motivation

The doctrine in `presence.md` and `security.md` is clear: every resident or embodied agent holds leases with TTL, renewal semantics, capability scopes, resource budgets, and revocation semantics. The existing RFCs implement these concepts piecemeal:

- RFC 0001 §3.3 defines lease checks within the mutation pipeline but does not define the lease state machine.
- RFC 0002 §5 defines budget enforcement tiers and the degradation ladder but does not define lease lifecycle events.
- RFC 0005 §1 defines session lifecycle with lease orphaning on disconnect but defers lease protocol details to `scene_service.proto`.
- RFC 0007 §4.2/§5.2 contain a contradiction: "Dismiss All" revokes leases simultaneously, but safe mode is explicitly designed to allow resume without re-establishing leases.

Without a canonical state machine:
- Implementers must reconcile lease behavior by reading four documents and resolving contradictions manually.
- The safe-mode revoke/suspend contradiction creates an ambiguous implementation target.
- `lease_priority` assignment and sort semantics are mentioned (RFC 0002 §5.2, §6.2) but never fully specified.
- Budget enforcement triggers (§5 of RFC 0002) do not specify which lease states are valid targets for each enforcement action.

---

## Design Requirements Satisfied

| ID | Requirement | Source |
|----|-------------|--------|
| DR-LG1 | Complete, unambiguous lease state machine | presence.md §"Leases: presence requires governance" |
| DR-LG2 | Lease priority assignment and sort order | RFC 0002 §5.2, §6.2 |
| DR-LG3 | Safe-mode lease behavior: suspend (not revoke) | RFC 0007 §5.2, security.md §"Human override" |
| DR-LG4 | Resource budget definition and enforcement ladder | security.md §"Resource governance", RFC 0002 §5 |
| DR-LG5 | Degradation triggers and lease interaction | RFC 0002 §6, failure.md §"Degradation axes" |
| DR-LG6 | Lease protocol messages (request/response/events) | RFC 0001, RFC 0005 |
| DR-LG7 | Contradiction resolution: Dismiss All revoke vs. safe-mode suspend | RFC 0007 §4.2 vs. §5.2 |

---

## 1. Lease Concepts

### 1.1 What a Lease Is

A **lease** is a runtime-issued, time-bounded authorization that grants an agent:

- A **namespace** (identity boundary for tile ownership)
- **Mutation rights** over tiles in that namespace
- A **capability scope** (what operations are permitted — e.g., `CREATE_TILE`, `WRITE_SCENE`, `zone:publish:subtitle`)
- A **resource budget** (texture memory, update rate, node count, concurrent leases)
- A **TTL** with renewal semantics
- **Revocation semantics** (how and when the runtime can reclaim the lease)

Leases are not tiles. A single lease may govern multiple tiles. Conversely, each tile's `lease_id` field references exactly one current lease.

### 1.2 What Requires a Lease

| Operation | Lease Required |
|-----------|---------------|
| `CreateTile` mutation | Yes — agent must hold a valid lease in the target namespace |
| `InsertNode`, `ReplaceNode`, `RemoveNode` | Yes — lease must be valid for the tile's `lease_id` |
| `UpdateTileBounds`, `UpdateTileExpiry` | Yes — same tile ownership check |
| `ZonePublish` (resident agent path) | Yes — lease must include `zone:publish:<zone_name>` capability |
| `ZonePublish` (guest MCP path) | No — guest uses `ZonePublishToken`; runtime holds the underlying tile |
| `CreateTab`, `RemoveTab` | No — tab operations require `MANAGE_TABS` capability, not a surface lease |
| Read operations (scene queries, hit-test) | No — read access is governed by `READ_SCENE` capability, not leases |

### 1.3 Lease Identity

Each lease has:
- A `LeaseId` (UUIDv7, time-ordered, assigned by runtime at grant time)
- A `namespace` (agent identity string, established at session auth)
- A `session_id` (parent session; lease is invalidated if session is revoked)
- A `granted_at_us` (UTC microseconds)
- A `ttl_ms` (lease duration; 0 = indefinite, subject to session TTL)
- A `capability_scope` (list of granted capabilities)
- A `resource_budget` (see §4)
- A `lease_priority` (see §2)

---

## 2. Lease Priority

### 2.1 Assignment

Every lease carries a `lease_priority: u8` field. The convention is:

| Value | Priority Class | Typical Use |
|-------|---------------|-------------|
| 0 | Critical | Chrome layer, safety overlays (runtime-internal only; agents may not request) |
| 1 | High | Primary interactive agent, embodied sessions with media |
| 2 | Normal (default) | Standard resident agents |
| 3 | Low | Background ambient content (weather widgets, status dashboards) |
| 4 | Speculative | Best-effort; first to be shed under pressure |

Agents may **request** a priority at lease-request time. The runtime **grants** the priority at or below what the agent's capability scope allows. An agent without an explicit priority capability receives priority 2 (Normal) regardless of what it requests.

The capability `lease:priority:<N>` permits requesting priority N or lower (numerically equal or higher value). A runtime administrator configures which agents receive elevated priority capabilities.

### 2.2 Sort Semantics

When the compositor must order tiles for rendering priority or shedding decisions, it uses a two-key sort:

```
sort key = (lease_priority ASC, z_order DESC)
```

Numerically lower `lease_priority` values are higher priority (0 = highest). Within the same priority class, higher `z_order` wins (foreground tiles preserved over background tiles).

This is consistent with RFC 0002 §5.2:
> "Sort tiles by priority using a two-key tuple `(lease_priority DESC, z_order DESC)` — lease priority is the primary sort key; z-order is the tiebreaker. Tiles with lower lease priority (numerically higher values, per the convention where 0 = highest) and lower z-order are shed first."

**Note on RFC 0002 phrasing:** RFC 0002's `DESC` for `lease_priority` means "sorted descending by value, so lower numeric values appear first" — equivalent to `ASC` when the goal is "lower value = higher importance." This RFC uses the clearer formulation: sort `lease_priority` ascending (lower value = higher priority preserved first).

### 2.3 Priority Constraints

- Runtime-internal leases (chrome layer tiles, safe-mode overlays) use priority 0 and cannot be overridden.
- A tile shed under the degradation ladder (RFC 0002 §6.2, Level 4+) remains in the scene graph with its `lease_id` valid; the lease is not revoked — only rendering is deferred.
- If a lease is **suspended** (see §3), its tiles are not rendered regardless of priority.
- If a lease is **revoked**, its tiles are immediately removed from the scene graph.

---

## 3. Lease State Machine

### 3.1 States

```
REQUESTED ──(granted)──► ACTIVE ──(TTL expired)──────────────────► EXPIRED
     │                      │
     │(denied)              │(viewer dismisses tile)
     ▼                      ├──(viewer_dismissed)──────────────────► REVOKED
  DENIED                    │
  (terminal)                │(budget policy: throttle sustained 30s
                            │  or critical limit exceeded)
                            ├──(budget_policy)────────────────────► REVOKED
                            │
                            │(safe mode entry)
                            ├──(safe_mode)────────────────────────► SUSPENDED
                            │
                            │(degradation Level 5)
                            ├──(degradation_emergency)───────────► SUSPENDED
                            │
                            │(agent disconnect, within grace period)
                            ├──(disconnect)───────────────────────► ORPHANED
                            │
                            │(agent releases lease)
                            └──(released)─────────────────────────► RELEASED
                                                                  (terminal)

SUSPENDED ──(safe_mode_exit / degradation recovery)──────────────► ACTIVE
ORPHANED  ──(agent reconnects within grace period)───────────────► ACTIVE
ORPHANED  ──(grace period expires)───────────────────────────────► EXPIRED
REVOKED   (terminal; no recovery)
EXPIRED   (terminal; agent may re-request a new lease)
```

### 3.2 State Descriptions

| State | Description | Agent Can Mutate | Tiles Rendered |
|-------|-------------|-----------------|----------------|
| `REQUESTED` | Lease request received; runtime evaluating | No | No (not yet created) |
| `ACTIVE` | Lease valid; agent holds mutation rights | Yes | Yes |
| `SUSPENDED` | Lease valid but mutations blocked; tiles frozen or replaced | No | No (placeholders shown, see §3.5) |
| `ORPHANED` | Session disconnected; within reconnect grace period | No | Yes (frozen at last state; disconnection badge shown per RFC 0007 §3.1) |
| `REVOKED` | Runtime reclaimed lease; no recovery path | No | No (tiles removed) |
| `EXPIRED` | TTL elapsed without renewal | No | No (tiles removed) |
| `DENIED` | Lease request rejected | N/A | N/A |
| `RELEASED` | Agent voluntarily released lease | No | No (tiles removed) |

### 3.3 Transition Triggers

#### REQUESTED → ACTIVE (grant)

The runtime evaluates:
1. Agent session is `Active` (RFC 0005 §1.1).
2. Agent holds `CREATE_TILE` capability.
3. `agent.active_leases.len() < agent.max_active_leases` (RFC 0002 §4.3).
4. Requested capability scope is within the agent's granted capability set.
5. Requested `lease_priority` is within the agent's priority capability.

If all checks pass, the runtime assigns a `LeaseId` and transitions to `ACTIVE`.

#### REQUESTED → DENIED

If any evaluation check fails. Terminal; the agent must submit a new `LeaseRequest` (possibly with reduced scope or priority).

#### ACTIVE → ORPHANED (session disconnect)

When the agent's session transitions to `Closed` (RFC 0005 §1.1) via ungraceful disconnect:
- The lease transitions to `ORPHANED`.
- The session reconnect grace period begins (`reconnect_grace_period_ms`, default 30,000 ms).
- Tiles are frozen at their last known state.
- The disconnection badge is shown per RFC 0007 §3.1.

If `expect_resume = false` in `SessionClose`, the runtime may accelerate cleanup (implementation-defined; grace period is a ceiling, not a floor).

#### ORPHANED → ACTIVE (reconnect)

Agent reconnects before grace period expiry and presents a valid session token (RFC 0005 §4.2). The lease is reclaimed; tiles resume their last-known state; the disconnection badge clears.

#### ORPHANED → EXPIRED (grace period elapsed)

Grace period expires without reconnect. Tiles are removed from the scene graph. The agent must re-request a new lease and re-create its tiles.

#### ACTIVE → SUSPENDED (safe mode)

Safe mode entry suspends leases without revoking them. See §3.4 for the canonical resolution of the revoke/suspend contradiction.

On safe mode entry:
- All `ACTIVE` leases transition to `SUSPENDED`.
- Agent sessions are notified via `SessionSuspended` with `reason = SAFE_MODE`.
- Agent mutations are rejected with `SAFE_MODE_ACTIVE` error.
- Tiles are replaced with neutral placeholders (RFC 0007 §5.2).

#### SUSPENDED → ACTIVE (safe mode exit)

On safe mode exit (explicit viewer action only — RFC 0007 §5.5):
- All `SUSPENDED` leases transition back to `ACTIVE`.
- Agent sessions receive `SessionResumed`.
- Agent mutations are accepted again.
- Tile placeholders are replaced with the last-committed tile state.

**The agent does not re-request a lease after safe mode exit.** Lease identity, TTL (accounting for elapsed suspension time — see §3.6), and capability scope are preserved across the `ACTIVE → SUSPENDED → ACTIVE` cycle.

#### ACTIVE → SUSPENDED (degradation emergency)

At degradation Level 5 (RFC 0002 §6.2), all non-highest-priority tiles enter a rendering-suspended state. Leases remain `ACTIVE` — this is a rendering suspension, not a lease suspension.

**Clarification:** RFC 0002 §6.2 Level 5 uses the phrase "tiles suspended (not revoked — leases remain valid)." This is a *rendering suspension only* — not a `SUSPENDED` lease state. The tiles are not rendered, but the lease remains `ACTIVE` and agents may continue sending mutations (which are queued or applied to scene state, even if not rendered). This RFC formalizes that distinction: the lease state does not change at degradation Level 5; only the rendering pass is modified.

#### ACTIVE → REVOKED (viewer dismisses tile)

When the viewer dismisses a specific tile (RFC 0007 §3.3):
1. The tile's lease transitions to `REVOKED`.
2. The tile is removed from the scene graph.
3. `LeaseResponse` with `reason = VIEWER_DISMISSED` is sent to the agent.
4. No grace period; no reconnect path.

The agent may re-request a new lease. Viewer dismissal is not a permanent ban — only a momentary choice (RFC 0007 §3.3).

#### ACTIVE → REVOKED (budget policy)

When budget enforcement reaches the Revocation tier (RFC 0002 §5.2):
1. All of the agent's leases transition to `REVOKED`.
2. `LeaseRevocationEvent` is sent for each lease.
3. No grace period; session resumption window is bypassed (RFC 0002 §5.2).
4. Agent-owned textures and node data are freed after the post-revocation delay (default 100ms).

#### ACTIVE → EXPIRED (TTL elapsed)

When `granted_at_us + ttl_ms < now`:
1. Lease transitions to `EXPIRED`.
2. `LeaseResponse` with `reason = EXPIRED` is sent to the agent.
3. Tiles remain in scene graph for one additional frame (grace frame for visual continuity), then removed.

Agents should renew before expiry. Renewal is a new `LeaseRequest` with the existing `LeaseId` (see §5).

#### ACTIVE → RELEASED (agent releases)

Agent sends `LeaseRequest` with `operation = RELEASE`. Immediately terminal. Tiles are removed.

### 3.4 Canonical Resolution: Safe Mode — Suspend, Not Revoke

**This section resolves the contradiction between RFC 0007 §4.2 and §5.2.**

RFC 0007 §4.2 ("Dismiss All / Safe Mode") states:
> "All active leases are revoked simultaneously."

RFC 0007 §5.2 ("Safe Mode Behavior") states:
> "Safe mode does not terminate sessions by default. This is intentional: a viewer who accidentally entered safe mode should be able to resume without agents needing to reconnect and re-establish their leases."

These are contradictory. If leases are revoked (§4.2), agents cannot resume without re-establishing leases (contradicting §5.2). If leases survive as suspended (§5.2), they are not revoked (contradicting §4.2).

**Resolution (DR-LG3):** Safe mode **suspends** leases; it does not revoke them. RFC 0007 §4.2's phrase "All active leases are revoked" is incorrect. The correct behavior is:

- On safe mode entry: all `ACTIVE` leases → `SUSPENDED`.
- On safe mode exit: all `SUSPENDED` leases → `ACTIVE` (TTL adjusted for suspension duration).
- **Rationale:** security.md §"Human override" establishes that safe mode is a viewer override that must be quickly reversible. A viewer who accidentally triggers safe mode should not lose all agent state. Revocation is the appropriate response to *budget violations* and *malicious behavior*, not to the human pressing the emergency stop. The emergency stop is a *pause*, not a *purge*.

RFC 0007 §4.2 must be updated to read: "All active leases are suspended simultaneously" (or equivalent phrasing). This RFC is authoritative on this point.

**Exception:** If the viewer explicitly selects "Disconnect Agents" (a separate control, distinct from "Dismiss All" / safe mode) the intent is revocation, not suspension. That control follows the per-tile dismiss flow (§3.3 REVOKED path) applied to all agents.

### 3.5 Suspended Lease Visual Representation

When a lease is `SUSPENDED` (safe-mode path):
- The runtime replaces the agent's tiles with neutral placeholders.
- Placeholder appearance: per RFC 0007 §5.2 — a subtle neutral pattern with "Session Paused" label centered in the tile bounds.
- Placeholder tiles are rendered by the runtime at the original tile's position and bounds.
- The agent's tile content is preserved in scene memory (not freed).
- On resume, placeholder tiles are replaced by re-rendering the agent's tiles from their last-committed scene state.

### 3.6 TTL Accounting During Suspension

When a lease is `SUSPENDED`, its TTL clock is paused. TTL advancement resumes when the lease transitions back to `ACTIVE`. This prevents a lease from expiring while the agent is unable to send renewals.

Formally: `effective_ttl_elapsed = sum of time spent in ACTIVE state`. The lease expires when `effective_ttl_elapsed >= ttl_ms`.

---

## 4. Resource Budgets

### 4.1 Budget Dimensions

Every lease carries a `ResourceBudget` that governs its permitted resource consumption:

```rust
pub struct ResourceBudget {
    // Per-tile limits (enforced per tile at mutation time)
    pub texture_bytes_per_tile: u64,    // Max texture memory for a single tile's nodes
    pub max_nodes_per_tile: u8,         // Max node count in a tile's tree; [1, 64]
    pub update_rate_hz: f32,            // Max mutation rate for this lease (mutations/second)

    // Per-session aggregate limits
    pub max_tiles: u16,                 // Max concurrent tiles under this lease; [1, 64]
    pub texture_bytes_total: u64,       // Aggregate texture bytes across all lease tiles
    pub max_active_leases: u8,          // Max simultaneous leases per session; [1, 64]
    pub max_concurrent_streams: u8,     // Max simultaneous media streams (0 in v1; post-v1 only)
}
```

Default values match the platform profile defaults in RFC 0002 §4.3. A requesting agent may request specific values; the runtime grants at or below the session's configured maximum.

### 4.2 Budget Enforcement Points

Budget checks occur at the following points in the mutation pipeline (RFC 0001 §4):

| Stage | Check | Failure Action |
|-------|-------|---------------|
| Mutation Intake (Stage 3) | `update_rate_hz` — sliding window | Batch rejected with `BUDGET_EXCEEDED_UPDATE_RATE` |
| Per-mutation Validation | `max_nodes_per_tile` — for `InsertNode` / `ReplaceNode` | Mutation rejected with `BUDGET_EXCEEDED_NODE_COUNT` |
| Per-mutation Validation | `texture_bytes_per_tile` — for node creation with texture content | Mutation rejected with `BUDGET_EXCEEDED_TEXTURE_BYTES` |
| Per-mutation Validation | `max_tiles` — for `CreateTile` | Mutation rejected with `BUDGET_EXCEEDED_TILE_COUNT` |
| Per-mutation Validation | `texture_bytes_total` — aggregate across all tiles | Mutation rejected with `BUDGET_EXCEEDED_TEXTURE_TOTAL` |
| Lease Request | `max_active_leases` — checked before granting new lease | Lease denied with `LEASE_DENIED_MAX_LEASES` |

Budget checks are all-or-nothing within a batch (RFC 0001 §4 — atomic pipeline).

### 4.3 Three-Tier Enforcement Ladder

Per-agent budget enforcement operates as a three-tier ladder (authoritative definition from RFC 0002 §5.2, reproduced here for completeness):

| Tier | Trigger | Duration | Action |
|------|---------|----------|--------|
| **Warning** | Any per-lease budget limit exceeded | — | Send `BudgetWarning` event to agent |
| **Throttle** | Warning unresolved for 5 seconds | Until resolved | Coalesce updates more aggressively; reduce effective `update_rate_hz` by 50% |
| **Revocation** | Throttle sustained for 30 seconds, or critical limit exceeded | Immediate | Revoke all leases; terminate session (see §4.4) |

Critical triggers that **bypass** the ladder and go directly to Revocation:
- Attempt to allocate texture memory exceeding the session hard maximum.
- Repeated invariant violations (> 10 in a session lifetime).
- Protocol violations indicating malicious intent (e.g., forged session IDs).

### 4.4 Resource Cleanup on Revocation

When a session is revoked (budget policy or critical trigger), the compositor thread executes on the same frame tick:

1. Move `BudgetState` to `Revoked`.
2. Transition all session leases from `ACTIVE` → `REVOKED` (state machine §3.3).
3. Enqueue `LeaseRevocationEvent` for all active leases.
4. Mark all agent-owned tiles as orphaned (frozen at last state; disconnection badge shown per RFC 0007 §3.1).
5. **No grace period.** Unlike unexpected disconnects, policy-driven revocations bypass the session resumption window (RFC 0005 §4.2).
6. After post-revocation delay (default 100ms, to allow `LeaseRevocationEvent` delivery), free all agent-owned textures and node data. Reference counts reach zero; resources are released.
7. Remove `AgentResourceState` from the compositor's per-agent table.

Post-revocation resource footprint must be zero (architecture.md §"Resource lifecycle"). Verified by the `disconnect_reclaim_multiagent` test scene.

---

## 5. Lease Protocol Messages

### 5.1 LeaseRequest (Agent → Runtime)

```protobuf
message LeaseRequest {
  enum Operation {
    OPERATION_UNSPECIFIED = 0;
    REQUEST    = 1;   // Request a new lease
    RENEW      = 2;   // Renew an existing lease before expiry
    RELEASE    = 3;   // Voluntarily release a lease
    EXTEND     = 4;   // Request TTL extension mid-lease (may be denied)
  }

  Operation      operation         = 1;   // Required
  string         lease_id          = 2;   // Required for RENEW/RELEASE/EXTEND; empty for REQUEST
  string         namespace         = 3;   // Requested namespace; must match session auth identity
  uint32         ttl_ms            = 4;   // Requested TTL; 0 = indefinite (runtime may cap)
  uint32         lease_priority    = 5;   // Requested priority [0–4]; runtime grants at/below capability ceiling
  ResourceBudget resource_budget   = 6;   // Requested budget; runtime grants at/below session maximum
  repeated string capability_scope = 7;   // Requested capabilities; subset of session-granted capabilities
}
```

### 5.2 LeaseResponse (Runtime → Agent)

```protobuf
message LeaseResponse {
  enum Result {
    RESULT_UNSPECIFIED            = 0;
    GRANTED                       = 1;   // New lease or renewal accepted
    DENIED                        = 2;   // Request refused; see reason
    REVOKED                       = 3;   // Runtime reclaimed an active lease
    EXPIRED                       = 4;   // Lease TTL elapsed
    RELEASED                      = 5;   // Release acknowledged
    EXTENDED                      = 6;   // TTL extension granted
  }

  enum RevokeReason {
    REVOKE_REASON_UNSPECIFIED     = 0;
    VIEWER_DISMISSED              = 1;   // Human dismissed the tile (RFC 0007 §3.3)
    BUDGET_POLICY                 = 2;   // Budget enforcement escalation (§4.3)
    CRITICAL_VIOLATION            = 3;   // Protocol or invariant violation (§4.3)
    SESSION_TERMINATED            = 4;   // Parent session revoked (implies lease revocation)
    RUNTIME_SHUTDOWN              = 5;   // Orderly runtime shutdown (RFC 0002 §3.1)
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
  }

  string         lease_id          = 1;   // Granted LeaseId; populated for GRANTED/EXTENDED
  Result         result            = 2;   // Required
  RevokeReason   revoke_reason     = 3;   // Populated if result = REVOKED
  DenyReason     deny_reason       = 4;   // Populated if result = DENIED
  uint32         granted_ttl_ms    = 5;   // Actual TTL granted (may differ from requested)
  uint32         granted_priority  = 6;   // Actual priority granted
  ResourceBudget granted_budget    = 7;   // Actual budget granted
  uint64         expires_at_us     = 8;   // UTC μs; 0 if TTL is indefinite
  string         message           = 9;   // Human-readable detail (for logs/debug)
}
```

### 5.3 LeaseRevocationEvent

Delivered as a `LeaseResponse` with `result = REVOKED` in the `lease_changes` subscription category (RFC 0005 §7.1). This message is always delivered — `lease_changes` is not filterable.

### 5.4 Subscription Category

`lease_changes` is delivered unconditionally to all active sessions (RFC 0005 §7.1). Agents cannot opt out. This ensures agents always know the current state of their leases, even under degradation or safe mode.

---

## 6. Lease Lifecycle and Session Lifecycle Interaction

### 6.1 Session State → Lease State Mapping

| Session State (RFC 0005 §1.1) | Effect on Agent Leases |
|-------------------------------|----------------------|
| `Connecting` | No leases exist |
| `Handshaking` | No leases exist |
| `Active` | Leases may be in any state (`ACTIVE`, `SUSPENDED`, `ORPHANED`) |
| `Disconnecting` (graceful) | Leases transition to `ORPHANED`; grace period starts |
| `Closed` (from Active) | Leases remain `ORPHANED` until grace period expires or agent reconnects |
| `Closed` (from Handshaking) | No leases; nothing to reclaim |
| `Resuming` | Leases remain `ORPHANED`; pending reclaim |
| `Active` (after resume) | Leases reclaimed back to `ACTIVE`; tiles resume from frozen state |

### 6.2 Safe Mode and Session Interaction

During safe mode (RFC 0007 §5):
- Sessions remain in `Active` state (network connections maintained).
- Leases transition to `SUSPENDED`.
- Mutations are rejected with `SAFE_MODE_ACTIVE`.
- `LeaseRequest` for new leases is rejected with `DenyReason = SAFE_MODE_ACTIVE`.
- Renewals are accepted (to prevent leases from expiring during suspension, see §3.6).
- On safe mode exit, leases return to `ACTIVE` and sessions resume normally.

### 6.3 New Lease Requests During Suspension

An agent's existing leases are `SUSPENDED` during safe mode, but its session is `Active`. The agent may still send `LeaseRequest` messages. The runtime's behavior:

- `RENEW` on a `SUSPENDED` lease: **accepted** (prevents expiry; TTL accounting paused per §3.6, so renewal is not necessary — but it is permitted and acknowledged without effect).
- `RELEASE` on a `SUSPENDED` lease: **accepted** (agent may voluntarily release a suspended lease; lease transitions to `RELEASED`; tile placeholder is removed).
- `REQUEST` for a new lease: **denied** with `DenyReason = SAFE_MODE_ACTIVE`. New surface territory cannot be established while safe mode is active.
- `EXTEND` on a `SUSPENDED` lease: **accepted** (same rationale as `RENEW`).

---

## 7. Degradation and Lease Interaction

### 7.1 Degradation Ladder vs. Lease States

The degradation ladder (RFC 0002 §6) affects rendering but does not change lease states, except at Level 5 Emergency.

| Degradation Level | Lease State Impact |
|-------------------|-------------------|
| Level 0 (Normal) | No change |
| Level 1 (Coalesce) | No change — update coalescing is a transport-layer effect |
| Level 2 (Reduce Texture Quality) | No change — texture downsampling applied in render pass |
| Level 3 (Disable Transparency) | No change — alpha blending disabled in render pass |
| Level 4 (Shed Tiles) | No change — tiles removed from render pass; leases remain `ACTIVE`; tiles remain in scene graph |
| Level 5 (Emergency) | No lease state change (leases remain `ACTIVE`); only the highest-priority tile is rendered; all other agent tiles are visually suppressed |

**Key principle:** Degradation is a rendering concern, not a governance concern. Leases are governance. The degradation ladder never invalidates a lease.

### 7.2 Priority Shedding at Level 4 and 5

At Level 4 ("Shed Tiles"), the compositor sorts tiles by `(lease_priority ASC, z_order DESC)` and removes the lowest-priority group from the render pass (approximately 25% of active tiles per application of Level 4). "Removed" means: not encoded in the render pass for that frame. The tile remains in the scene graph; its lease is valid; its content is preserved.

At Level 5 ("Emergency"), only the highest-priority single tile (lowest `lease_priority` value, then highest `z_order`) plus the chrome layer is rendered. All others are visually suppressed, but their leases remain `ACTIVE`.

Agents are notified of degradation via `DegradationNotice` (RFC 0005 §3.4) with the current level. Agents should reduce their update rate and content complexity in response.

### 7.3 Recovery

Recovery from Level 5 back to Normal requires `frame_time_p95 < 12ms` over a 30-frame window, progressing one level at a time (RFC 0002 §6.3 hysteresis). At each recovery step, previously-shed tiles are re-included in the render pass in priority order.

---

## 8. Lease Renewal

### 8.1 When to Renew

Agents with leases that have a finite TTL should renew before expiry. Recommended practice: renew when `now > expires_at_us - (ttl_ms * 0.25)` (renew at 75% of TTL elapsed).

The runtime does not proactively warn agents about impending expiry — it is the agent's responsibility. A lease that expires without renewal transitions to `EXPIRED`, and the agent must re-request a new lease and re-create its tiles.

### 8.2 Renewal Semantics

A `LeaseRequest` with `operation = RENEW` and a valid, non-expired `lease_id`:
- Resets the TTL clock (`granted_at_us = now`).
- May grant a different TTL than requested (runtime policy).
- Does **not** change the lease ID (continuity is preserved).
- Does **not** affect tiles, capability scope, or resource budget unless the agent also requests changes (not supported in v1 — budget and scope changes require a new lease).

A `LeaseRequest` with `operation = RENEW` on an `EXPIRED` lease is rejected with `DenyReason = LEASE_ALREADY_EXPIRED`. The agent must request a new lease.

### 8.3 Indefinite Leases

`ttl_ms = 0` requests an indefinite lease. The runtime may grant indefinite leases to trusted sessions (embodied agents, long-running resident agents). Indefinite leases survive session reconnects within the grace period but are invalidated on policy-driven revocation or session termination.

---

## 9. Lease Caps and Scene-Level Limits

| Dimension | Default Cap | Hard Max | Notes |
|-----------|-------------|----------|-------|
| `max_active_leases` per session | 8 | 64 | RFC 0002 §4.3 |
| `max_tiles` per lease | 8 | 64 | Configured per session |
| `max_nodes_per_tile` | 32 | 64 | RFC 0001 §2.3 |
| `texture_bytes_per_tile` | 16 MiB | — | Platform-dependent |
| `texture_bytes_total` per session | 256 MiB | 2 GiB | RFC 0002 §4.3 |
| `update_rate_hz` per lease | 30 | 120 | RFC 0002 §4.3 |

Requests exceeding the hard max are rejected with structured errors. Values between the default and hard max are negotiated at session establishment.

---

## 10. Protobuf Schema

```protobuf
syntax = "proto3";
package tze.lease.v1;

// ─── Lease Request / Response ────────────────────────────────────────────────

message LeaseRequest {
  enum Operation {
    OPERATION_UNSPECIFIED = 0;
    REQUEST               = 1;
    RENEW                 = 2;
    RELEASE               = 3;
    EXTEND                = 4;
  }

  Operation        operation        = 1;
  string           lease_id         = 2;
  string           namespace        = 3;
  uint32           ttl_ms           = 4;
  uint32           lease_priority   = 5;
  ResourceBudget   resource_budget  = 6;
  repeated string  capability_scope = 7;
}

message LeaseResponse {
  enum Result {
    RESULT_UNSPECIFIED = 0;
    GRANTED            = 1;
    DENIED             = 2;
    REVOKED            = 3;
    EXPIRED            = 4;
    RELEASED           = 5;
    EXTENDED           = 6;
  }

  enum RevokeReason {
    REVOKE_REASON_UNSPECIFIED = 0;
    VIEWER_DISMISSED          = 1;
    BUDGET_POLICY             = 2;
    CRITICAL_VIOLATION        = 3;
    SESSION_TERMINATED        = 4;
    RUNTIME_SHUTDOWN          = 5;
  }

  enum DenyReason {
    DENY_REASON_UNSPECIFIED   = 0;
    INSUFFICIENT_CAPABILITY   = 1;
    MAX_LEASES_EXCEEDED       = 2;
    PRIORITY_NOT_PERMITTED    = 3;
    BUDGET_EXCEEDS_SESSION_MAX = 4;
    SESSION_NOT_ACTIVE        = 5;
    LEASE_ALREADY_EXPIRED     = 6;
    SAFE_MODE_ACTIVE          = 7;
  }

  string         lease_id       = 1;
  Result         result         = 2;
  RevokeReason   revoke_reason  = 3;
  DenyReason     deny_reason    = 4;
  uint32         granted_ttl_ms = 5;
  uint32         granted_priority = 6;
  ResourceBudget granted_budget = 7;
  uint64         expires_at_us  = 8;   // UTC μs; RFC 0003 §3.1; 0 = indefinite
  string         message        = 9;   // Human-readable debug detail
}

// ─── Resource Budget ─────────────────────────────────────────────────────────

message ResourceBudget {
  uint64 texture_bytes_per_tile  = 1;   // Max texture bytes for a single tile
  uint32 max_nodes_per_tile      = 2;   // Max nodes in tile tree [1, 64]
  float  update_rate_hz          = 3;   // Max mutations/second for this lease
  uint32 max_tiles               = 4;   // Max concurrent tiles [1, 64]
  uint64 texture_bytes_total     = 5;   // Aggregate texture bytes across all tiles
  uint32 max_active_leases       = 6;   // Max simultaneous leases per session [1, 64]
  uint32 max_concurrent_streams  = 7;   // Max media streams; 0 in v1 (post-v1 only)
}

// ─── Lease State (for telemetry / scene snapshot) ───────────────────────────

message LeaseStateRecord {
  string      lease_id       = 1;
  string      namespace      = 2;
  string      session_id     = 3;
  LeaseState  state          = 4;
  uint64      granted_at_us  = 5;   // UTC μs; RFC 0003 §3.1
  uint64      expires_at_us  = 6;   // UTC μs; 0 = indefinite
  uint32      priority       = 7;
  ResourceBudget budget      = 8;
  repeated string capability_scope = 9;
}

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

---

## 11. Cross-RFC Errata

This section records the corrections to other RFCs that this document mandates. Each affected RFC must be updated to align with this document at its next revision pass.

### RFC 0007 §4.2 — Dismiss All / Safe Mode

**Current text:**
> "All active leases are revoked simultaneously."

**Corrected text:**
> "All active leases are suspended simultaneously. Sessions are not terminated; agents receive `SessionSuspended`. Leases transition to `SUSPENDED` state (RFC 0008 §3.3) and are restored to `ACTIVE` on safe mode exit without requiring the agent to re-request a lease."

**Rationale:** See §3.4 (DR-LG7). The intent of the action is to pause agents, not destroy their state. The corrected behavior is consistent with RFC 0007 §5.2 ("does not terminate sessions by default"), security.md ("safe mode as last resort, quickly reversible"), and the principle that revocation is for misbehavior, not for emergency stops.

### RFC 0001 §3.3 — Lease Check

**Current text:** References lease validity check (`lease_registry.get(T.lease_id).is_valid()`) without specifying which lease states are "valid."

**Clarification:** A lease is valid for mutation purposes if its state is `ACTIVE`. `SUSPENDED`, `ORPHANED`, `REVOKED`, `EXPIRED`, and `RELEASED` leases all fail the validity check.

### RFC 0002 §5.2 — Frame-Time Guardian

**Current text:** Uses "`(lease_priority DESC, z_order DESC)`" sort — see §2.2 of this RFC for the unambiguous formulation.

**Clarification:** No behavioral change; only the description. The sort means: tiles with lower `lease_priority` values (higher importance) are preserved; within the same priority class, tiles with higher `z_order` values (rendered on top) are preserved.

### RFC 0005 §3.2 — Server → Client Messages

**Current text:** Does not include `SessionSuspended` or `SessionResumed` in the message table (noted as a protocol gap in RFC 0007 §8).

**Required addition:** Add `SessionSuspended` and `SessionResumed` to the `SessionMessage` oneof and to the §3.2 message table:

| Message | Traffic Class | Description |
|---------|--------------|-------------|
| `SessionSuspended` | Transactional | Runtime has suspended the session (safe mode entry); all mutations rejected until `SessionResumed` |
| `SessionResumed` | Transactional | Session resumed after suspension; mutations accepted again |

---

## 12. Quantitative Requirements Summary

| Metric | Requirement | Notes |
|--------|-------------|-------|
| Lease grant latency | < 1ms from `LeaseRequest` receipt | On compositor thread; no I/O |
| Lease revocation latency | < 1 frame (≤ 16.6ms) | Revocation enqueued and processed within the current frame tick |
| Safe mode suspend latency | < 1 frame | All leases transitioned to `SUSPENDED` within one frame of safe mode entry |
| Safe mode resume latency | < 2 frames | Placeholder removal and tile re-render within two frames of exit |
| Post-revocation resource freedom | 0 bytes after 100ms + 1 frame | All textures and node data freed |
| `LeaseRevocationEvent` delivery | Within post-revocation delay (default 100ms) | Before resource free |
| Orphan grace period | 30,000ms default (configurable) | `reconnect_grace_period_ms` in RFC 0005 §8 |
| TTL accounting precision | ±1ms | Suspension time correctly subtracted from effective elapsed TTL |

---

## 13. Open Questions

1. **Budget renegotiation mid-lease.** V1 does not support changing a lease's `resource_budget` or `capability_scope` after grant (requires `RELEASE` + `REQUEST`). A `RENEGOTIATE` operation is a post-v1 candidate. Relevant for long-lived embodied sessions that need to acquire media stream capabilities after initial auth.

2. **Multi-lease atomic operations.** An agent may want to atomically swap two tiles across two leases (release one, acquire another in the same scene update). The current model requires two separate `LeaseRequest` messages. A batch-lease operation is a post-v1 consideration.

3. **Lease inheritance across tabs.** Currently a lease governs tiles within any tab in the agent's namespace. Should leases be tab-scoped? Tab-scoped leases would simplify tab teardown but add complexity for agents that span multiple tabs (e.g., a clock widget visible in all tabs). Defer to post-v1 design.

4. **Grace period acceleration on explicit release.** RFC 0005 §1.3 notes the runtime "may accelerate cleanup" when `expect_resume = false`. The exact acceleration semantics are implementation-defined. A future revision should specify the minimum observable grace period.

---

## 14. Related RFCs

| RFC | Topic | Relationship |
|-----|-------|-------------|
| RFC 0001 (Scene Contract) | Scene data model, mutation pipeline, tile structure | Lease validity enforced in mutation pipeline §3.3; `Tile.lease_id` references leases defined here |
| RFC 0002 (Runtime Kernel) | Budget enforcement tiers, degradation ladder, admission control | Budget enforcement ladder (§5) and tile shedding priority (§6.2) are authoritative inputs to this RFC |
| RFC 0003 (Timing Model) | Timestamp semantics | All `*_at_us` fields in lease messages use RFC 0003 §3.1 UTC microsecond timestamps |
| RFC 0005 (Session Protocol) | Session lifecycle, `LeaseRequest`/`LeaseResponse` wire format | Session lifecycle affects lease states per §6.1; `SessionSuspended`/`SessionResumed` messages required by §6.2 |
| RFC 0007 (System Shell) | Safe mode, tile dismiss, override controls | §3.4 resolves the revoke/suspend contradiction; §4.2 errata required |

---

## 15. Review Record

*No reviews yet. This RFC is in initial draft.*
