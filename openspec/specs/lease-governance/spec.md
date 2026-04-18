# lease-governance Specification

## Purpose

Defines how the runtime governs agent screen territory through a lease system: the full lease state machine (REQUESTED → ACTIVE → SUSPENDED/ORPHANED/EXPIRED/REVOKED/RELEASED/DENIED), lease identity and TTL accounting, priority assignment and sort semantics, auto-renewal policies, safe mode suspension and resume, orphan handling and reconnect grace periods, resource budget enforcement (soft 80% warning, hard 100% reject, three-tier enforcement ladder), zone publication requirements, degradation ladder interaction, and wire protocol field allocations. This is the AUTHORITY capability for all agent access to compositor resources — every mutation path, lease lifecycle event, and budget enforcement decision is governed by the contracts defined here.

Source RFC: 0008 (Lease and Resource Governance)

---

## Requirements

### Requirement: Lease State Machine
The runtime MUST implement the following lease state machine with states: REQUESTED, ACTIVE, SUSPENDED, ORPHANED, REVOKED, EXPIRED, DENIED, RELEASED. Valid transitions MUST be: REQUESTED->ACTIVE (granted), REQUESTED->DENIED (denied), ACTIVE->SUSPENDED (safe mode), ACTIVE->ORPHANED (disconnect), ACTIVE->EXPIRED (TTL elapsed), ACTIVE->REVOKED (viewer dismiss, budget policy), ACTIVE->RELEASED (agent releases), SUSPENDED->ACTIVE (safe mode exit), SUSPENDED->REVOKED (max suspension time exceeded), ORPHANED->ACTIVE (reconnect within grace), ORPHANED->EXPIRED (grace period elapsed). DENIED, REVOKED, EXPIRED, and RELEASED are terminal states. Lease grants MUST always be a subset of the session's currently granted capabilities at request time.
Source: RFC 0008 §3.1
Scope: v1-mandatory

#### Scenario: Lease granted
- **WHEN** an agent sends a `LeaseRequest` with valid capabilities and budget within limits
- **THEN** the lease transitions from REQUESTED to ACTIVE and the agent receives `LeaseResponse` with `result = GRANTED`

#### Scenario: Lease denied
- **WHEN** an agent sends a `LeaseRequest` requesting capabilities beyond its session grants
- **THEN** the lease transitions to DENIED and the agent receives `LeaseResponse` with `result = DENIED` and populated `deny_reason`

#### Scenario: Escalation required before broader lease scope
- **WHEN** an agent is authorized for a capability by policy but does not yet hold it in the current session grant set
- **THEN** a `LeaseRequest` using that capability MUST be denied until the agent first obtains it via `CapabilityRequest`

#### Scenario: Invalid transition rejected
- **WHEN** a lease is in EXPIRED state
- **THEN** no further transitions are possible (terminal state)

---

### Requirement: Lease Identity
Each lease MUST have: a `LeaseId` (UUIDv7, time-ordered, SceneId type), a `namespace` (agent identity string), a `session_id` (parent session), `granted_at_wall_us` (UTC microseconds), `ttl_ms` (0 = indefinite), `renewal_policy`, `capability_scope`, `resource_budget`, and `lease_priority`.
Source: RFC 0008 §1.3
Scope: v1-mandatory

#### Scenario: Lease identity fields populated
- **WHEN** a lease is granted
- **THEN** the `LeaseResponse` includes a UUIDv7 LeaseId, the granted TTL, priority, budget, and expiry timestamp

---

### Requirement: Operations Requiring a Lease
`CreateTile`, `InsertNode`, `ReplaceNode`, `RemoveNode`, `UpdateTileBounds`, `UpdateTileExpiry` MUST require an ACTIVE lease in the target namespace. `ZonePublish` via resident agent path MUST require an ACTIVE lease with `publish_zone:<zone_type>` capability. `ZonePublish` via guest MCP path MUST NOT require a lease. `CreateTab`, `RemoveTab`, read operations, and scene queries MUST NOT require a lease.
Source: RFC 0008 §1.2
Scope: v1-mandatory

#### Scenario: Tile mutation without active lease
- **WHEN** an agent attempts `InsertNode` but its lease is in SUSPENDED state
- **THEN** the mutation is rejected with `SAFE_MODE_ACTIVE` error

#### Scenario: Guest zone publish without lease
- **WHEN** a guest agent publishes to a zone via MCP `ZonePublishToken`
- **THEN** the publish succeeds without requiring a lease

---

### Requirement: Priority Assignment
Every lease MUST carry a `lease_priority: u8` field. Priority 0 MUST be reserved for system/chrome (runtime-internal only; agents MUST NOT request priority 0). Priority 1 MUST require the `lease:priority:1` capability (from the canonical capability vocabulary per RFC 0006 §6.3 and RFC 0009 §8.1). Priority 2 MUST be the default. An agent requesting priority 0 MUST receive priority 2. An agent requesting priority 1 without the capability MUST receive priority 2. The runtime MAY downgrade priority at any time via `LeaseStateChange`. Note: RFC 0008 §2.1 uses the older name `lease_priority_high` for this capability; the canonical name is `lease:priority:1` per the RFC 0009 §8.1 capability registry and RFC 0006 §6.3 vocabulary.
Source: RFC 0008 §2.1, RFC 0009 §8.1, RFC 0006 §6.3
Scope: v1-mandatory

#### Scenario: Priority 0 request downgraded
- **WHEN** an agent requests `lease_priority = 0`
- **THEN** the lease is granted with priority 2 (or the agent's capability ceiling)

#### Scenario: Priority 1 without capability
- **WHEN** an agent requests `lease_priority = 1` without `lease:priority:1` capability
- **THEN** the lease is granted with priority 2

---

### Requirement: Priority Sort Semantics
The compositor MUST sort tiles for rendering priority and shedding decisions using the key `(lease_priority ASC, z_order DESC)`. Numerically lower `lease_priority` values MUST be higher priority (0 = highest). Within the same priority class, higher `z_order` MUST win. Tiles with the highest `lease_priority` values and lowest `z_order` values MUST be shed first.
Source: RFC 0008 §2.2
Scope: v1-mandatory

#### Scenario: Shedding order
- **WHEN** the degradation ladder requires tile shedding
- **THEN** tiles with the highest lease_priority values (least important) and lowest z_order values are shed first

---

### Requirement: Auto-Renewal Policy
Each lease MUST support three renewal policies: MANUAL (default: agent must explicitly send `LeaseRequest` with `operation = RENEW` before TTL expires), AUTO_RENEW (runtime auto-renews at 75% TTL elapsed if session is Active and no budget violations pending; agent receives `LeaseResponse` with `result = GRANTED` on each auto-renewal), ONE_SHOT (expires at TTL without renewal option; agent must request a new lease to continue). "Auto-renewal disabled" means the 75%-elapsed renewal timer is NOT armed for that lease; no renewal event is generated. The lease continues to age toward expiry at its current TTL. Auto-renewal MUST be disabled (timer not armed) when: (1) the agent enters budget warning state — resuming if warning clears before TTL expires; (2) the session enters `Disconnecting` state; (3) safe mode is entered (TTL clock paused per RFC 0008 §4.3; timer is also paused and resumes with the TTL clock on safe mode exit). ONE_SHOT leases are not subject to auto-renewal; however, their TTL clock IS paused during suspension, so they do not expire during safe mode (a ONE_SHOT lease suspended during safe mode still transitions normally to ACTIVE on safe mode exit with remaining TTL adjusted).
Source: RFC 0008 §1.4, §4.3
Scope: v1-mandatory

#### Scenario: Auto-renewal at 75% TTL
- **WHEN** a lease with AUTO_RENEW policy reaches 75% of its TTL
- **THEN** the runtime automatically renews the lease and sends `LeaseResponse` with `result = GRANTED`

#### Scenario: Auto-renewal disabled during budget warning
- **WHEN** a lease with AUTO_RENEW policy enters budget warning state
- **THEN** the auto-renewal timer is disarmed; the lease continues aging toward expiry and the agent must renew manually or resolve the budget warning before TTL elapses

#### Scenario: ONE_SHOT lease expires
- **WHEN** a ONE_SHOT lease reaches its TTL
- **THEN** the lease transitions to EXPIRED without renewal option

#### Scenario: ONE_SHOT lease suspended during safe mode
- **WHEN** a ONE_SHOT lease is active when safe mode is entered
- **THEN** the TTL clock pauses during suspension; on safe mode exit the lease transitions back to ACTIVE with adjusted expiry and can serve its full remaining TTL

---

### Requirement: Safe Mode Suspends Leases
On safe mode entry, all ACTIVE leases MUST transition to SUSPENDED (NOT REVOKED). Agent sessions MUST receive `SessionSuspended` with `reason = "viewer_safe_mode"`. `LeaseSuspend` MUST be sent for each lease. Mutations MUST be rejected with `SAFE_MODE_ACTIVE`. TTL clock MUST be paused. Auto-renewal MUST be suspended. This suspension MUST complete within 1 frame (16.6ms).
Source: RFC 0008 §3.3, §3.4
Scope: v1-mandatory

#### Scenario: Safe mode suspends all leases
- **WHEN** the viewer triggers safe mode
- **THEN** all ACTIVE leases transition to SUSPENDED within one frame, agents receive `SessionSuspended`, and mutations are rejected

#### Scenario: TTL paused during suspension
- **WHEN** a lease is SUSPENDED for 2 minutes and then resumed
- **THEN** the effective TTL does not count the 2 minutes of suspension time

---

### Requirement: Safe Mode Resume
On safe mode exit, all SUSPENDED leases MUST transition back to ACTIVE. Agents MUST receive `SessionResumed`. `LeaseResume` MUST be sent for each lease with `adjusted_expires_at_wall_us` and `suspension_duration_us`. Staleness badges MUST clear within 1 frame. Tiles MUST render from last-committed scene state. Resume MUST complete within 2 frames (33.2ms). The agent MUST NOT re-request a lease.
Source: RFC 0008 §3.3, §4.5
Scope: v1-mandatory

#### Scenario: Resume preserves lease identity
- **WHEN** safe mode is exited
- **THEN** all previously SUSPENDED leases return to ACTIVE with the same LeaseId, capability scope, and budget

---

### Requirement: Max Suspension Time
A lease MUST NOT remain SUSPENDED indefinitely. After `max_suspension_time_ms` (default: 300,000ms / 5 minutes), the lease MUST transition to REVOKED. If safe mode remains active beyond this duration, suspended leases MUST be progressively revoked (oldest first).
Source: RFC 0008 §4.6
Scope: v1-mandatory

#### Scenario: Suspension timeout
- **WHEN** a lease is SUSPENDED for more than 5 minutes (default)
- **THEN** the lease transitions to REVOKED with `revoke_reason = SUSPENSION_TIMEOUT`

---

### Requirement: Suspension Preserves State
When a lease is SUSPENDED, all tiles, node trees, zone publications, resource allocations, and lease metadata MUST be preserved in memory. No resources MUST be freed.
Source: RFC 0008 §4.1
Scope: v1-mandatory

#### Scenario: Tile content preserved during suspension
- **WHEN** a lease transitions from ACTIVE to SUSPENDED and back to ACTIVE
- **THEN** all tile content, node trees, and geometry remain intact without agent re-submission

---

### Requirement: Orphan Handling Grace Period
On agent disconnect, all ACTIVE leases MUST transition to ORPHANED. The reconnect grace period (default 30,000ms) MUST start. Tiles MUST be frozen at last known state. Disconnection badge MUST appear within 1 frame. TTL clock MUST continue running during the grace period. If the agent reconnects within the grace period, leases MUST transition back to ACTIVE and badges MUST clear within 1 frame.

Note: Disconnect *detection* uses the heartbeat timeout (default 15,000ms = 3 × 5,000ms per RFC 0005 §4). The reconnect grace *window* is a separate 30,000ms period that starts after detection. These are two distinct timers: detection (15s) determines when orphaning begins; grace (30s) determines how long the agent has to reclaim before leases expire.
Source: RFC 0008 §5.1, §5.2, §5.4
Scope: v1-mandatory

#### Scenario: Agent reconnects within grace period
- **WHEN** an agent disconnects and reconnects within 30 seconds
- **THEN** ORPHANED leases transition to ACTIVE, disconnection badges clear, and the agent can immediately submit mutations

#### Scenario: Grace period expires
- **WHEN** an agent fails to reconnect within the grace period
- **THEN** all ORPHANED leases transition to EXPIRED, tiles are removed, and resources are freed

---

### Requirement: Grace Period Precision
The grace period MUST be accurate to +/- 100ms. The runtime MUST NOT prematurely expire the grace period.
Source: RFC 0008 §5.4, §12
Scope: v1-mandatory

#### Scenario: Grace period not premature
- **WHEN** the reconnect grace period is 30,000ms
- **THEN** the agent can still reconnect at 29,950ms and successfully reclaim leases

---

### Requirement: Resource Budget Schema
Every lease MUST carry a `ResourceBudget` with dimensions: `max_nodes_per_tile` (range [1, 64]), `update_rate_hz` (mutations/second), `max_tiles` (range [1, 64]), `texture_bytes_total`, `max_active_leases` (range [1, 64]), `max_concurrent_streams` (0 in v1). Default values MUST match the platform profile defaults.
Source: RFC 0008 §6.1
Scope: v1-mandatory

#### Scenario: Budget dimensions enforced
- **WHEN** a lease is granted with `max_tiles = 8` and `max_nodes_per_tile = 32`
- **THEN** the agent can create up to 8 tiles, each with up to 32 nodes

#### Scenario: max_concurrent_streams zero in v1
- **WHEN** a lease is granted in v1
- **THEN** `max_concurrent_streams` is 0 (media streams deferred to post-v1)

---

### Requirement: Budget Soft Warning at 80%
When any budget dimension reaches 80% of its allocated maximum, the runtime MUST send a `BudgetWarning` event to the agent via `LeaseStateChange`. A budget warning badge MUST be rendered on affected tiles. The mutation MUST be accepted (soft limits do not reject work).
Source: RFC 0008 §6.3
Scope: v1-mandatory

#### Scenario: Texture budget at 80%
- **WHEN** an agent's texture usage reaches 80% of `texture_bytes_total`
- **THEN** a `BudgetWarning` event is sent, an amber border appears on tiles, and the mutation is accepted

---

### Requirement: Budget Hard Limit at 100%
When a mutation would push any budget dimension to or beyond 100% of its maximum, the mutation MUST be rejected with a `BUDGET_EXCEEDED_*` error code. The entire `MutationBatch` MUST fail atomically.
Source: RFC 0008 §6.3
Scope: v1-mandatory

#### Scenario: Tile count at 100%
- **WHEN** an agent at `max_tiles = 8` attempts to create a 9th tile
- **THEN** the entire `MutationBatch` is rejected with `BUDGET_EXCEEDED_TILE_COUNT`

---

### Requirement: Three-Tier Budget Enforcement Ladder
Budget enforcement MUST follow a three-tier ladder: (1) Warning at >= 80% (send `BudgetWarning`, render badge), (2) Throttle after warning unresolved for 5 seconds (reduce effective `update_rate_hz` by 50%), (3) Revocation after throttle sustained for 30 seconds or critical limit exceeded (revoke all leases, terminate session). Critical triggers that bypass the ladder: `CriticalTextureOomAttempt`, `RepeatedInvariantViolations` (> 10 in session lifetime), protocol violations indicating malicious intent.
Source: RFC 0008 §6.5
Scope: v1-mandatory

#### Scenario: Throttle after 5 seconds
- **WHEN** a budget warning persists for 5 seconds without resolution
- **THEN** the agent's effective update_rate_hz is reduced by 50%

#### Scenario: Revocation after 30 seconds throttle
- **WHEN** throttle persists for 30 seconds
- **THEN** all agent leases are revoked and the session is terminated

#### Scenario: Critical bypass
- **WHEN** an agent attempts to allocate texture memory exceeding the hard maximum
- **THEN** all leases are immediately revoked without going through warning/throttle

---

### Requirement: Budget Enforcement Latency
Per-mutation budget check overhead MUST be < 50us. Budget checks MUST be all-or-nothing within a batch (atomic pipeline).
Source: RFC 0008 §6.4, §12
Scope: v1-mandatory

#### Scenario: Budget check within latency budget
- **WHEN** a `MutationBatch` with 64 mutations is evaluated
- **THEN** each per-mutation budget check completes within 50us

---

### Requirement: Zone Publish Requires Active Lease
For resident agents, publishing to a zone MUST require: (1) an ACTIVE lease (not SUSPENDED, ORPHANED, or terminal), (2) the `publish_zone:<zone_type>` capability for the target zone, (3) the zone instance exists in the current active tab. Missing lease MUST produce `LEASE_NOT_FOUND`, inactive lease `LEASE_NOT_ACTIVE`, missing capability `CAPABILITY_NOT_GRANTED`, missing zone `ZONE_NOT_FOUND`.
Source: RFC 0008 §7.1
Scope: v1-mandatory

#### Scenario: Zone publish with suspended lease
- **WHEN** a resident agent attempts to publish to a zone while its lease is SUSPENDED
- **THEN** the publish is rejected with `LEASE_NOT_ACTIVE`

#### Scenario: Zone publish without capability
- **WHEN** an agent holds an ACTIVE lease but lacks `publish_zone:subtitle` and publishes to subtitle
- **THEN** the publish is rejected with `CAPABILITY_NOT_GRANTED`

---

### Requirement: Lease Suspension Freezes Zone Publications
When a lease is SUSPENDED, existing zone publications MUST remain visible but stale-badged. New zone publishes under the suspended lease MUST be rejected with `SAFE_MODE_ACTIVE`. Other agents' zone publishes MUST continue independently.
Source: RFC 0008 §7.2
Scope: v1-mandatory

#### Scenario: Stale zone content during suspension
- **WHEN** a lease is SUSPENDED
- **THEN** existing zone publications remain visible with a staleness indicator but no new publishes are accepted

---

### Requirement: Lease Revocation Clears Zone Publications
When a lease is REVOKED or EXPIRED, all zone publications made under that lease MUST be immediately cleared from the zone registry. The zone's contention policy MUST govern what fills the vacated space.
Source: RFC 0008 §7.3
Scope: v1-mandatory

#### Scenario: Zone cleared on revocation
- **WHEN** a lease is REVOKED
- **THEN** all zone publications from that lease are cleared and the zone's contention policy determines the replacement

---

### Requirement: Lease Grant Latency
Lease grant latency MUST be < 1ms from `LeaseRequest` receipt to `LeaseResponse` emission. This is a local decision on the compositor thread with no I/O.
Source: RFC 0008 §3.3, §12
Scope: v1-mandatory

#### Scenario: Sub-millisecond lease grant
- **WHEN** an agent sends a valid `LeaseRequest`
- **THEN** the `LeaseResponse` with `result = GRANTED` is emitted within 1ms

---

### Requirement: Post-Revocation Resource Cleanup
On budget-driven revocation, the compositor MUST: (1) transition all session leases to REVOKED, (2) send `LeaseResponse` with `revoke_reason = BUDGET_POLICY`, (3) mark tiles for removal, (4) bypass the grace period entirely, (5) free all resources after a 100ms delay (to allow LeaseResponse delivery). Post-revocation resource footprint MUST be zero.
Source: RFC 0008 §6.6
Scope: v1-mandatory

#### Scenario: Zero resource footprint after revocation
- **WHEN** an agent's leases are revoked due to budget policy
- **THEN** after the 100ms delay plus one frame, all textures and node data are freed and resource footprint is zero

---

### Requirement: Degradation Does Not Change Lease State
The degradation ladder (Levels 0-5) MUST NOT change lease states. Leases MUST remain ACTIVE during all degradation levels. At Level 4 (Shed Tiles), tiles are removed from the render pass but remain in the scene graph with valid leases. At Level 5 (Emergency), only the highest-priority tile plus chrome is rendered; all others are visually suppressed but leases remain ACTIVE.
Source: RFC 0008 §13.1
Scope: v1-mandatory

#### Scenario: Lease active during tile shedding
- **WHEN** the degradation ladder reaches Level 4 and a tile is shed from the render pass
- **THEN** the tile's lease remains ACTIVE, the agent can still submit mutations, and the tile remains in the scene graph

---

### Requirement: Tile Shedding Order
At degradation Level 4, the compositor MUST sort tiles by `(lease_priority ASC, z_order DESC)` and remove approximately 25% of active tiles per application. Shed tiles remain in the scene graph; their leases are not revoked. When degradation recovers, previously shed tiles MUST resume rendering in priority order without agent re-publish.
Source: RFC 0008 §13.2
Scope: v1-mandatory

#### Scenario: Shed tiles resume on recovery
- **WHEN** degradation recovers from Level 4 to Level 0
- **THEN** previously shed tiles resume rendering with their last committed content without any agent action

---

### Requirement: Degradation Trigger Threshold
The degradation trigger for tile shedding MUST be `frame_time_p95 > 14ms` over a 10-frame window. Recovery MUST require `frame_time_p95 < 12ms` over a 30-frame window, progressing one level at a time.
Source: RFC 0008 §13.3, RFC 0002 §6
Scope: v1-mandatory

#### Scenario: Degradation entry
- **WHEN** `frame_time_p95 > 14ms` is sustained over a 10-frame window
- **THEN** the degradation ladder advances and tile shedding begins

---

### Requirement: TTL Accounting Precision
TTL accounting during suspension MUST be accurate to +/- 100ms. The effective lease expiry MUST be calculated as `granted_at_wall_us + (ttl_ms * 1000) + suspension_duration_us`.
Source: RFC 0008 §4.3, §12
Scope: v1-mandatory

#### Scenario: TTL correctly adjusted after suspension
- **WHEN** a lease with `ttl_ms = 60000` is suspended for 10,000ms and then resumed
- **THEN** the effective expiry is extended by 10,000ms (within +/- 100ms accuracy)

---

### Requirement: Lease Caps
The system MUST enforce: max 64 leases per runtime (all agents), max 8 default (64 hard max) leases per session, max 64 tiles per lease, max 64 nodes per tile. Requests exceeding hard maximums MUST be rejected with structured errors.
Source: RFC 0008 §9
Scope: v1-mandatory

#### Scenario: Runtime-wide lease limit
- **WHEN** 64 leases are active across all agents and a new lease is requested
- **THEN** the request is denied with `MAX_RUNTIME_LEASES_EXCEEDED`

---

### Requirement: Wire Protocol Integration
Lease messages MUST flow over the primary session stream within the `SessionMessage` envelope. `LeaseRequest` (client->server) MUST use field 21. `LeaseResponse` (server->client) MUST use field 31. `LeaseSuspend` MUST use field 47, `LeaseResume` field 48, `LeaseStateChange` field 49.
Source: RFC 0008 §10.1, §10.6
Scope: v1-mandatory

#### Scenario: Lease messages on session stream
- **WHEN** an agent sends a `LeaseRequest`
- **THEN** it is carried as field 21 of the `SessionMessage` oneof on the existing session stream

---

### Requirement: Budget Renegotiation
V1 MUST NOT support changing a lease's `resource_budget` or `capability_scope` after grant. Changing budget or scope MUST require releasing the lease and requesting a new one.
Source: RFC 0008 §15
Scope: v1-reserved

#### Scenario: Budget change requires new lease
- **WHEN** an agent needs more texture memory than its current lease allows
- **THEN** it must release the current lease and request a new one with a larger budget

---

### Requirement: Multi-Lease Atomic Operations
Atomic operations spanning multiple leases (e.g., swap tiles across two leases) are deferred to post-v1. Implementations MUST NOT support atomic multi-lease operations in v1; agents MUST use separate `LeaseRequest` messages for each lease.
Source: RFC 0008 §15
Scope: post-v1

#### Scenario: Deferred atomic multi-lease ops
- **WHEN** an agent needs to atomically swap tiles across two leases
- **THEN** the runtime MUST NOT support atomic cross-lease operations; the agent must use two separate `LeaseRequest` messages (non-atomic)

---

### Requirement: Grace Period Acceleration
When `expect_resume = false` in `SessionClose`, the runtime MAY accelerate cleanup (implementation-defined). The grace period is a ceiling, not a floor. Implementations MUST treat the grace period as a maximum bound; early cleanup on explicit close is permitted but MUST NOT be applied when `expect_resume` is absent or true.
Source: RFC 0008 §5.6
Scope: v1-reserved

#### Scenario: Explicit no-resume disconnect
- **WHEN** an agent sends `SessionClose` with `expect_resume = false`
- **THEN** the runtime MAY immediately transition orphaned leases to EXPIRED; it MUST NOT apply early cleanup when `expect_resume` is absent or true
