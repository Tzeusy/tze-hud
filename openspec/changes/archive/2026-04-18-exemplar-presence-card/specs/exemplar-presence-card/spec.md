# Exemplar: Agent Presence Card

Domain: EXEMPLAR
Depends on: scene-graph, lease-governance, session-protocol

---

## ADDED Requirements

### Requirement: Presence Card Tile Geometry
Each agent presence card SHALL be a tile with fixed dimensions 320x112 logical pixels. The tile SHALL be anchored to the bottom-left corner of the active tab with a 24px margin from the left edge and 24px margin from the bottom edge. The tile opacity SHALL be 1.0. The tile input_mode SHALL be Capture so the card can host a small human-dismiss affordance in the top-right corner.

> **Implementation note:** `CreateTile` only carries `tab_id`, `namespace`, `lease_id`, `bounds`, and `z_order`. Opacity and input_mode must be set via separate `UpdateTileOpacity` and `UpdateTileInputMode` mutations in the same batch.

Source: proposal (visual requirements), scene-graph spec (tile CRUD, tile field invariants)

#### Scenario: Single agent presence card creation
- **WHEN** an agent with an ACTIVE lease submits a MutationBatch containing CreateTile with bounds {x: 24.0, y: tab_height - 136.0, width: 320.0, height: 112.0}, followed by UpdateTileOpacity (1.0) and UpdateTileInputMode (Capture)
- **THEN** the runtime MUST create the tile in the agent's namespace with the specified geometry, opacity, and input mode

#### Scenario: Dismiss affordance is interactive
- **WHEN** a pointer event lands on the dismiss hit region in the presence card's top-right corner
- **THEN** the runtime MUST dispatch a click with interaction_id `dismiss-card` to the owning agent session so the human can clear the card directly

### Requirement: Multi-Agent Vertical Stacking
When multiple agents each create a presence card tile, the tiles SHALL be vertically stacked in the bottom-left corner with 12px vertical gaps between cards. Agent tiles MUST NOT overlap. The stacking order (bottom to top) SHALL be: agent 0 at y = tab_height - 136, agent 1 at y = tab_height - 260, agent 2 at y = tab_height - 384. Each agent SHALL use a unique z_order value (agent 0: 100, agent 1: 101, agent 2: 102), all below ZONE_TILE_Z_MIN (0x8000_0000).
Source: proposal (multi-agent coexistence), scene-graph spec (tile field invariants, z-order)

#### Scenario: Three agents create non-overlapping presence cards
- **WHEN** agents A, B, and C each create a presence card tile with the specified y-offsets and z_orders
- **THEN** three tiles MUST be visible in the bottom-left corner, vertically stacked with 12px gaps, and no BoundsOutOfRange validation error MUST occur. Unique z_order values are assigned as a best practice for deterministic rendering order.

#### Scenario: Z-order values below zone minimum
- **WHEN** presence card tiles are created with z_order values 100, 101, 102
- **THEN** all tiles MUST be accepted (all values < ZONE_TILE_Z_MIN = 0x8000_0000) and MUST render below any zone or widget tiles

### Requirement: Presence Card Node Tree
Each presence card tile SHALL have a flat node tree (depth = 1) with a `SolidColorNode` root and exactly 12 child nodes under that root:
1. **SolidColorNode root** — background fill with `Rgba { r: 0.10, g: 0.14, b: 0.19, a: 0.72 }`, bounds matching the full tile `(0, 0, 320, 112)`
2. **SolidColorNode sheen** — 2px top highlight with `Rgba { r: 0.92, g: 0.96, b: 1.0, a: 0.16 }`, bounds `(0, 0, 320, 2)`
3. **SolidColorNode accent rail** — agent-tinted rail at `(0, 18, 4, 76)` with alpha `0.78`
4. **SolidColorNode avatar plate** — agent-tinted translucent plate at `(24, 28, 56, 56)` with alpha `0.22`
5. **StaticImageNode** — agent avatar icon, referencing a pre-uploaded `ResourceId`, displayed at `(34, 38, 36, 36)` with fit mode `Cover`
6. **TextMarkdownNode eyebrow** — uppercase `"RESIDENT AGENT"` metadata label at `(96, 18, 152, 12)`, font size `11px`, color `Rgba { r: 0.72, g: 0.80, b: 0.90, a: 0.82 }`
7. **TextMarkdownNode name** — bold agent name at `(96, 34, 152, 26)`, font size `20px`, color `Rgba { r: 0.97, g: 0.99, b: 1.0, a: 1.0 }`
8. **TextMarkdownNode status** — `"Connected • last active now"` style status line at `(96, 68, 148, 18)`, font size `13px`, color `Rgba { r: 0.82, g: 0.88, b: 0.94, a: 0.92 }`
9. **SolidColorNode time-chip background** — bounds `(224, 20, 44, 22)`, color `Rgba { r: 0.86, g: 0.92, b: 1.0, a: 0.12 }`
10. **TextMarkdownNode time-chip text** — compact `"NOW"`/`"30S"`/`"1M"` label at `(224, 21, 44, 22)`, font size `10px`, centered, color `Rgba { r: 0.96, g: 0.98, b: 1.0, a: 0.96 }`
11. **SolidColorNode dismiss background** — bounds `(280, 18, 24, 24)`, subtle glass tint for the clear affordance
12. **TextMarkdownNode dismiss label** — centered `"X"` at `(280, 18, 24, 24)`, font size `12px`, bright white tint
13. **HitRegionNode dismiss target** — bounds `(280, 18, 24, 24)`, interaction_id `dismiss-card`, accepts_focus `true`, accepts_pointer `true`

The nodes SHALL be added in the order listed so that the background renders behind the glass layers, avatar treatment, typography, and time chip. The runtime-owned orphan badge occupies separate chrome above the tile and MUST NOT require extra card nodes.
Source: proposal (visual requirements), scene-graph spec (V1 node types, AddNode)

#### Scenario: Node tree structure after creation
- **WHEN** an agent creates a presence card tile and adds the full glass card tree
- **THEN** the tile MUST have a `SolidColorNode` root with exactly 12 child nodes in the specified insertion order

#### Scenario: Background node covers full tile
- **WHEN** the root `SolidColorNode` is added with bounds `(0, 0, 320, 112)` and color `Rgba { r: 0.10, g: 0.14, b: 0.19, a: 0.72 }`
- **THEN** the node MUST render a semi-transparent dark rectangle covering the entire tile area

#### Scenario: Avatar image node with pre-uploaded resource
- **WHEN** the `StaticImageNode` references a `ResourceId` from a prior `UploadResource` call
- **THEN** the runtime MUST accept the node (resource exists) and render the image at `(34, 38, 36, 36)` inside the tinted avatar plate, visually centered with equal inset

#### Scenario: Text content with markdown bold
- **WHEN** the status node contains `"Connected • last active now"` and the time-chip node contains `"NOW"`
- **THEN** the runtime MUST render a two-tier hierarchy with the bold agent name, status line, and compact chip label, all using overflow `Ellipsis` where applicable

### Requirement: Lease Lifecycle for Presence Cards
Each agent SHALL request a lease with ttl_ms 120000 (2 minutes) and capabilities including create_tiles and modify_own_tiles. The server-side lease state machine SHALL be configured with `AutoRenew` renewal policy. The runtime SHALL auto-renew the lease at 75% TTL (90 seconds elapsed). The agent MUST hold the lease for the entire lifetime of the presence card. Tile creation and node mutations MUST be rejected if the lease is not ACTIVE.

> **Implementation note:** The LeaseRequest proto has fields: `ttl_ms`, `capabilities` (repeated string), and `lease_priority`. Renewal policy (`AutoRenew`) is a server-side / Rust-layer concern, not a wire field. LeaseResponse has `granted: bool`, not an enum result.
Source: proposal (lease governance), lease-governance spec (lease state machine, auto-renewal)

#### Scenario: Lease request and grant
- **WHEN** an agent sends a LeaseRequest with ttl_ms 120000 and capabilities [create_tiles, modify_own_tiles]
- **THEN** the runtime MUST grant the lease (transition REQUESTED -> ACTIVE) and return a LeaseResponse with granted = true, the assigned LeaseId, and the effective TTL

#### Scenario: Auto-renewal at 75% TTL
- **WHEN** 90 seconds have elapsed since lease grant (75% of 120s TTL)
- **THEN** the runtime MUST auto-renew the lease and send a LeaseResponse with granted = true, resetting the TTL expiry

#### Scenario: Tile mutation with active lease succeeds
- **WHEN** an agent with an ACTIVE lease submits a MutationBatch containing CreateTile
- **THEN** the mutation MUST pass the lease check and proceed to subsequent validation checks

#### Scenario: Tile mutation with expired lease rejected
- **WHEN** an agent submits a MutationBatch but its lease has transitioned to EXPIRED
- **THEN** the runtime MUST reject the batch with LeaseExpired error

### Requirement: Periodic Content Update
Each agent SHALL update its presence card content every 30 seconds by submitting a `MutationBatch` with a single `SetTileRoot` mutation containing the complete updated node tree. The updated text content SHALL keep the eyebrow, name, visual layering, and dismiss affordance unchanged while updating the status line to `"Connected • last active Ns ago"` and the time-chip label to compact values such as `"30S"` or `"1M"`.

> **Implementation note:** There is no `ReplaceNode` variant in `SceneMutation`. To update the status line and time chip, the agent rebuilds the full 13-node tree and submits it via `SetTileRoot`. For this small flat tree the overhead is acceptable.

Source: proposal (behavioral requirements), scene-graph spec (atomic batch mutations)

#### Scenario: Content update after 30 seconds
- **WHEN** 30 seconds have elapsed since the last content update
- **THEN** the agent MUST submit a `MutationBatch` with a `SetTileRoot` mutation containing the full node tree with the status node updated to `"Connected • last active 30s ago"` and the chip node updated to `"30S"`

#### Scenario: Content update after 90 seconds
- **WHEN** 90 seconds have elapsed since session start
- **THEN** the status node content MUST read `"Connected • last active 1m ago"` and the chip node MUST read `"1M"`

#### Scenario: Only text node is replaced
- **WHEN** a content update MutationBatch is submitted
- **THEN** the batch MUST contain exactly 1 `SetTileRoot` mutation with the complete node tree; the glass background, accent treatment, avatar plate, avatar image, eyebrow, and name nodes MUST be included unchanged in the rebuilt tree

### Requirement: Agent Disconnect and Orphan Handling
When an agent disconnects (gRPC stream close or heartbeat timeout after 15 seconds), the runtime SHALL transition all the agent's ACTIVE leases to ORPHANED. The presence card tile SHALL be frozen at its last committed state. A disconnection badge SHALL appear on the tile within 1 frame (16.6ms). The reconnect grace period (30 seconds) SHALL start. If the agent does not reconnect within the grace period, leases SHALL transition to EXPIRED and the tile SHALL be removed from the scene graph.
Source: proposal (disconnect test), lease-governance spec (orphan handling grace period), failure.md

#### Scenario: Disconnect detection via heartbeat timeout
- **WHEN** an agent stops sending heartbeats and 15 seconds elapse (3 missed heartbeats at 5s interval)
- **THEN** the runtime MUST detect the disconnect and transition the agent's lease from ACTIVE to ORPHANED

#### Scenario: Disconnection badge appears within 1 frame
- **WHEN** a lease transitions to ORPHANED
- **THEN** a disconnection badge MUST appear on the agent's presence card tile within 1 frame (16.6ms), visually indicating the tile is orphaned

#### Scenario: Tile frozen during orphan state
- **WHEN** a presence card tile's lease is ORPHANED
- **THEN** the tile MUST remain rendered at its last committed state (background, avatar, and text are unchanged), and no mutations from the disconnected agent SHALL be accepted

#### Scenario: Reconnect within grace period reclaims lease
- **WHEN** a disconnected agent reconnects within 30 seconds
- **THEN** the ORPHANED lease MUST transition back to ACTIVE, the disconnection badge MUST clear within 1 frame, and the agent MUST be able to resume content updates

#### Scenario: Grace period expiry removes tile
- **WHEN** 30 seconds elapse without the agent reconnecting
- **THEN** the ORPHANED lease MUST transition to EXPIRED and the presence card tile (with all its nodes) MUST be removed from the scene graph

### Requirement: Multi-Agent Isolation During Disconnect
When one agent disconnects, the remaining agents' presence cards SHALL continue operating normally. Lease transitions for the disconnected agent MUST NOT affect other agents' leases, tiles, or content updates. The remaining agents' presence cards MUST remain at their assigned positions without repositioning.
Source: proposal (multi-agent coexistence), scene-graph spec (namespace isolation)

#### Scenario: Two agents continue after third disconnects
- **WHEN** agent C disconnects while agents A and B remain connected
- **THEN** agents A and B MUST continue submitting content updates successfully, their tiles MUST remain at their original y-positions, and their leases MUST remain ACTIVE

#### Scenario: Disconnected agent's tile removal does not shift others
- **WHEN** agent C's grace period expires and its tile is removed
- **THEN** agents A and B's tiles MUST remain at their original positions (y = tab_height - 136 and y = tab_height - 260 respectively); no automatic repositioning SHALL occur

### Requirement: Resource Upload for Avatar Icons
Each agent SHALL upload a 32x32 PNG avatar image via the resource upload mechanism before creating its presence card tile. The runtime SHALL return a ResourceId (BLAKE3 content hash). The StaticImageNode SHALL reference this ResourceId. The exemplar defines three placeholder avatar colors: agent 0 = solid blue (RGB 66, 133, 244), agent 1 = solid green (RGB 52, 168, 83), agent 2 = solid orange (RGB 251, 188, 4).
Source: proposal (visual requirements), scene-graph spec (ResourceId identity, content deduplication)

#### Scenario: Avatar resource upload and reference
- **WHEN** an agent uploads a 32x32 PNG image
- **THEN** the runtime MUST return a ResourceId (BLAKE3 hash) and the agent MUST use this ResourceId in the StaticImageNode

#### Scenario: Duplicate avatar upload returns same ResourceId
- **WHEN** two agents upload identical PNG images
- **THEN** both MUST receive the same ResourceId (content deduplication)

### Requirement: gRPC Test Sequence
The exemplar SHALL define a complete gRPC test sequence for one agent that exercises the full presence card lifecycle:
1. SessionInit (authenticate, receive SessionEstablished with SceneSnapshot)
2. LeaseRequest (ttl_ms=120000, capabilities: create_tiles, modify_own_tiles; `AutoRenew` is server-side)
3. UploadResource (32x32 PNG avatar)
4. MutationBatch: CreateTile (320x112, bottom-left corner) + UpdateTileOpacity(1.0) + UpdateTileInputMode(Capture) + SetTileRoot/AddNode assembly for the 13-node interactive glass card tree
5. Wait 30s, then MutationBatch: SetTileRoot (updated node tree with new status-line and time-chip content)
6. Repeat step 5 every 30s
7. SessionClose (expect_resume = false) or connection drop for disconnect test

The multi-agent test sequence SHALL run 3 instances of this sequence concurrently with different agent namespaces, avatar colors, y-offsets, and z_orders.
Source: proposal (test integration), session-protocol spec (ClientMessage/ServerMessage)

#### Scenario: Full single-agent lifecycle
- **WHEN** the test sequence executes steps 1-7 for a single agent
- **THEN** the agent MUST have an ACTIVE lease, a visible 320x112 tile in the bottom-left corner with the glass-card hierarchy, and the status text MUST update every 30 seconds

#### Scenario: Three-agent concurrent lifecycle
- **WHEN** three instances of the test sequence run concurrently with namespaces "agent-alpha", "agent-beta", "agent-gamma"
- **THEN** three non-overlapping presence card tiles MUST be visible in the bottom-left corner, each with its own avatar color and agent name

> **Live proof status (2026-04-10):** automated integration coverage exists; resident `/user-test` execution evidence is still required for production-like live validation.

### Requirement: User-Test Scenario
The exemplar SHALL define a user-test scenario with the following steps:
1. Launch 3 agent sessions that each create a presence card
2. Visually verify: 3 stacked glass cards in bottom-left, each with colored accent/avatar treatment, metadata eyebrow, bold name, and time chip
3. Wait 30s, verify: status lines and time chips update on all 3 cards
4. Disconnect agent 2 (drop connection or close stream)
5. Verify within 1s (human-observable tolerance; the runtime requirement is 1 frame / 16.6ms): disconnection badge appears on agent 2's card
6. Wait 30s: agent 2's card is removed; agents 0 and 1 remain unchanged
7. Pass criteria: all visual states observed in sequence

Source: proposal (user-test integration)

#### Scenario: User-test visual verification sequence
- **WHEN** the user-test scenario executes all 7 steps
- **THEN** the tester MUST observe: 3 cards visible → content updates → disconnection badge → tile removal → 2 remaining cards

> **Live proof status (2026-04-10):** scenario contract is specified, but closure still requires resident `/user-test` integration and a completed manual-review checklist verdict.
