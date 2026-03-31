# Exemplar: Agent Presence Card

Domain: EXEMPLAR
Depends on: scene-graph, lease-governance, session-protocol

---

## ADDED Requirements

### Requirement: Presence Card Tile Geometry
Each agent presence card SHALL be a tile with fixed dimensions 200x80 logical pixels. The tile SHALL be anchored to the bottom-left corner of the active tab with a 16px margin from the left edge and 16px margin from the bottom edge. The tile opacity SHALL be 1.0. The tile input_mode SHALL be Passthrough (display-only, no input capture).

> **Implementation note:** `CreateTile` only carries `tab_id`, `namespace`, `lease_id`, `bounds`, and `z_order`. Opacity and input_mode must be set via separate `UpdateTileOpacity` and `UpdateTileInputMode` mutations in the same batch.

Source: proposal (visual requirements), scene-graph spec (tile CRUD, tile field invariants)

#### Scenario: Single agent presence card creation
- **WHEN** an agent with an ACTIVE lease submits a MutationBatch containing CreateTile with bounds {x: 16.0, y: tab_height - 96.0, width: 200.0, height: 80.0}, followed by UpdateTileOpacity (1.0) and UpdateTileInputMode (Passthrough)
- **THEN** the runtime MUST create the tile in the agent's namespace with the specified geometry, opacity, and input mode

#### Scenario: Tile input mode is Passthrough
- **WHEN** a pointer event lands on a presence card tile
- **THEN** the hit-test MUST skip the tile (Passthrough) and test the next tile in z-order

### Requirement: Multi-Agent Vertical Stacking
When multiple agents each create a presence card tile, the tiles SHALL be vertically stacked in the bottom-left corner with 8px vertical gaps between cards. Agent tiles MUST NOT overlap. The stacking order (bottom to top) SHALL be: agent 0 at y = tab_height - 96, agent 1 at y = tab_height - 184, agent 2 at y = tab_height - 272. Each agent SHALL use a unique z_order value (agent 0: 100, agent 1: 101, agent 2: 102), all below ZONE_TILE_Z_MIN (0x8000_0000).
Source: proposal (multi-agent coexistence), scene-graph spec (tile field invariants, z-order)

#### Scenario: Three agents create non-overlapping presence cards
- **WHEN** agents A, B, and C each create a presence card tile with the specified y-offsets and z_orders
- **THEN** three tiles MUST be visible in the bottom-left corner, vertically stacked with 8px gaps, and no BoundsOutOfRange validation error MUST occur. Unique z_order values are assigned as a best practice for deterministic rendering order, even though the runtime's ZOrderConflict validation does not apply to Passthrough tiles.

#### Scenario: Z-order values below zone minimum
- **WHEN** presence card tiles are created with z_order values 100, 101, 102
- **THEN** all tiles MUST be accepted (all values < ZONE_TILE_Z_MIN = 0x8000_0000) and MUST render below any zone or widget tiles

### Requirement: Presence Card Node Tree
Each presence card tile SHALL have a flat node tree (depth = 1) with exactly 3 child nodes under the tile root:
1. **SolidColorNode** — background fill with `Rgba { r: 0.08, g: 0.08, b: 0.08, a: 0.78 }`, bounds matching the full tile (0, 0, 200, 80)
2. **StaticImageNode** — agent avatar icon, 32x32 pixels, positioned at (8, 24) within the tile, fit mode Cover, referencing a pre-uploaded ResourceId
3. **TextMarkdownNode** — agent identity text positioned at (48, 8) within the tile, 144px wide, 64px tall, containing the agent name in bold followed by a newline and "Last active: now" status text. Font size 14px, color `Rgba { r: 0.94, g: 0.94, b: 0.94, a: 1.0 }`, left-aligned, overflow Ellipsis.

The nodes SHALL be added in the order listed (SolidColorNode first, then StaticImageNode, then TextMarkdownNode) so that the background renders behind the icon and text. Use either `SetTileRoot` with the complete tree, or individual `AddNode` mutations.
Source: proposal (visual requirements), scene-graph spec (V1 node types, AddNode)

#### Scenario: Node tree structure after creation
- **WHEN** an agent creates a presence card tile and adds 3 nodes (via SetTileRoot or AddNode)
- **THEN** the tile MUST have exactly 3 child nodes: a SolidColorNode (background), a StaticImageNode (avatar), and a TextMarkdownNode (identity text), in that insertion order

#### Scenario: Background node covers full tile
- **WHEN** the SolidColorNode is added with bounds (0, 0, 200, 80) and color `Rgba { r: 0.08, g: 0.08, b: 0.08, a: 0.78 }`
- **THEN** the node MUST render a semi-transparent dark rectangle covering the entire tile area

#### Scenario: Avatar image node with pre-uploaded resource
- **WHEN** the StaticImageNode references a ResourceId from a prior UploadResource call
- **THEN** the runtime MUST accept the node (resource exists) and render the image at (8, 24) with 32x32 size

#### Scenario: Text content with markdown bold
- **WHEN** the TextMarkdownNode contains "**AgentName**\nLast active: now"
- **THEN** the runtime MUST render "AgentName" in bold and "Last active: now" on a second line, both left-aligned, with overflow Ellipsis if text exceeds 144px width

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
Each agent SHALL update its presence card's TextMarkdownNode content every 30 seconds by submitting a MutationBatch with a single `SetTileRoot` mutation containing the complete updated node tree. The updated text content SHALL contain the agent name (bold) and an updated "Last active: Ns ago" timestamp reflecting the elapsed time since the agent's session started. The SolidColorNode and StaticImageNode SHALL be included unchanged in the rebuilt tree.

> **Implementation note:** There is no `ReplaceNode` variant in `SceneMutation`. To update a single node's content, the agent rebuilds the full node tree and submits it via `SetTileRoot`. For a 3-node flat tree this is trivially cheap.

Source: proposal (behavioral requirements), scene-graph spec (atomic batch mutations)

#### Scenario: Content update after 30 seconds
- **WHEN** 30 seconds have elapsed since the last content update
- **THEN** the agent MUST submit a MutationBatch with a SetTileRoot mutation containing the full node tree with the TextMarkdownNode updated to "**AgentName**\nLast active: 30s ago"

#### Scenario: Content update after 90 seconds
- **WHEN** 90 seconds have elapsed since session start
- **THEN** the TextMarkdownNode content MUST read "**AgentName**\nLast active: 1m ago" (human-friendly time format)

#### Scenario: Only text node is replaced
- **WHEN** a content update MutationBatch is submitted
- **THEN** the batch MUST contain exactly 1 mutation (SetTileRoot with the complete node tree); the SolidColorNode and StaticImageNode MUST be included unchanged in the rebuilt tree

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
- **THEN** agents A and B's tiles MUST remain at their original positions (y = tab_height - 96 and y = tab_height - 184 respectively); no automatic repositioning SHALL occur

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
4. MutationBatch: CreateTile (200x80, bottom-left corner) + UpdateTileOpacity(1.0) + UpdateTileInputMode(Passthrough) + SetTileRoot (3-node tree: SolidColorNode, StaticImageNode, TextMarkdownNode)
5. Wait 30s, then MutationBatch: SetTileRoot (updated node tree with new TextMarkdownNode content)
6. Repeat step 5 every 30s
7. SessionClose (expect_resume = false) or connection drop for disconnect test

The multi-agent test sequence SHALL run 3 instances of this sequence concurrently with different agent namespaces, avatar colors, y-offsets, and z_orders.
Source: proposal (test integration), session-protocol spec (ClientMessage/ServerMessage)

#### Scenario: Full single-agent lifecycle
- **WHEN** the test sequence executes steps 1-7 for a single agent
- **THEN** the agent MUST have an ACTIVE lease, a visible 200x80 tile in the bottom-left corner with avatar and identity text, and the text MUST update every 30 seconds

#### Scenario: Three-agent concurrent lifecycle
- **WHEN** three instances of the test sequence run concurrently with namespaces "agent-alpha", "agent-beta", "agent-gamma"
- **THEN** three non-overlapping presence card tiles MUST be visible in the bottom-left corner, each with its own avatar color and agent name

### Requirement: User-Test Scenario
The exemplar SHALL define a user-test scenario with the following steps:
1. Launch 3 agent sessions that each create a presence card
2. Visually verify: 3 stacked cards in bottom-left, each with colored avatar and agent name
3. Wait 30s, verify: "Last active" text updates on all 3 cards
4. Disconnect agent 2 (drop connection or close stream)
5. Verify within 1s (human-observable tolerance; the runtime requirement is 1 frame / 16.6ms): disconnection badge appears on agent 2's card
6. Wait 30s: agent 2's card is removed; agents 0 and 1 remain unchanged
7. Pass criteria: all visual states observed in sequence

Source: proposal (user-test integration)

#### Scenario: User-test visual verification sequence
- **WHEN** the user-test scenario executes all 7 steps
- **THEN** the tester MUST observe: 3 cards visible → content updates → disconnection badge → tile removal → 2 remaining cards
