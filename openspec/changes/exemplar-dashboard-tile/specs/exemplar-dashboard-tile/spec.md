## ADDED Requirements

### Requirement: Dashboard Tile Composition
The exemplar dashboard tile SHALL be a single agent-owned tile of 400x300 logical pixels positioned at (50, 50) within the active tab's content layer with z_order = 100 and opacity = 1.0. The tile SHALL contain exactly 6 nodes arranged as a flat tree (all children of the root) composited in tree order (painter's model, first child rendered first):

1. SolidColorNode — background fill, Rgba { r: 0.07, g: 0.07, b: 0.07, a: 0.90 }, bounds covering the full tile (0, 0, 400, 300)
2. StaticImageNode — agent icon, 48x48, positioned at (16, 16), fit mode = Contain, referencing a previously-uploaded PNG resource
3. TextMarkdownNode — header, content = agent name in bold markdown (e.g., `"**Dashboard Agent**"`), font_size_px = 18.0, color = Rgba { r: 1.0, g: 1.0, b: 1.0, a: 1.0 }, positioned at (76, 20, 308, 32), alignment = Left, overflow = Ellipsis
4. TextMarkdownNode — body, content = markdown-formatted live info (e.g., stats, uptime, timestamp), font_size_px = 14.0, color = Rgba { r: 0.78, g: 0.78, b: 0.78, a: 1.0 }, positioned at (16, 72, 368, 180), alignment = Left, overflow = Ellipsis
5. HitRegionNode — "Refresh" button, bounds (16, 256, 176, 36), interaction_id = "refresh-button", accepts_focus = true, accepts_pointer = true, auto_capture = true, release_on_up = true, cursor_style = Pointer, tooltip = "Refresh dashboard content"
6. HitRegionNode — "Dismiss" button, bounds (208, 256, 176, 36), interaction_id = "dismiss-button", accepts_focus = true, accepts_pointer = true, auto_capture = true, release_on_up = true, cursor_style = Pointer, tooltip = "Dismiss this tile"

> **Note:** Tile-level scrolling via `ScrollConfig` is a separate post-v1 feature. The body node uses `Ellipsis` overflow; content that exceeds the node bounds is truncated with an ellipsis indicator.

Source: scene-graph spec (V1 Node Types, Scene Graph Hierarchy, Tile Field Invariants); architecture.md (Intra-tile compositing)
Scope: v1-mandatory

#### Scenario: All four node types present in tile
- **WHEN** the dashboard tile is created via atomic batch
- **THEN** the tile SHALL contain exactly 6 nodes: 1 SolidColorNode, 1 StaticImageNode, 2 TextMarkdownNodes, and 2 HitRegionNodes, arranged in the specified tree order

#### Scenario: Background node covers full tile bounds
- **WHEN** the SolidColorNode is rendered
- **THEN** it SHALL fill the entire tile area (0, 0, 400, 300) with Rgba { r: 0.07, g: 0.07, b: 0.07, a: 0.90 }, providing a dark semi-transparent background

#### Scenario: Icon image references uploaded resource
- **WHEN** the StaticImageNode is created
- **THEN** it SHALL reference a valid ResourceId from a previously-uploaded PNG image, and the runtime SHALL render the image at 48x48 within the node's bounds

#### Scenario: Painter's model compositing order
- **WHEN** the tile's nodes are composited
- **THEN** the SolidColorNode (background) SHALL be rendered first, the StaticImageNode (icon) second, the TextMarkdownNodes (header, body) third and fourth, and the HitRegionNodes (buttons) last, with each subsequent node painting over previous nodes in overlapping regions

---

### Requirement: Atomic Tile Creation Batch
The entire dashboard tile (CreateTile + SetTileRoot + 6x InsertNode) SHALL be submitted as a single MutationBatch. The batch SHALL be validated and committed atomically: if any mutation fails, the entire batch SHALL be rejected with no partial application. The user SHALL never see a partially-constructed dashboard tile.

Source: scene-graph spec (Atomic Batch Mutations, Transaction Validation Pipeline); presence.md (Scene mutations are atomic)
Scope: v1-mandatory

#### Scenario: Successful atomic tile creation
- **WHEN** the agent submits a MutationBatch containing CreateTile, SetTileRoot, and 6 InsertNode mutations with valid parameters
- **THEN** the entire batch SHALL be committed atomically and the dashboard tile SHALL appear fully constructed in one frame

#### Scenario: Partial failure rejects entire batch
- **WHEN** the agent submits the tile creation batch but one InsertNode has invalid bounds (e.g., width = 0)
- **THEN** the entire batch SHALL be rejected, no tile SHALL appear in the scene, and the rejection SHALL include the failing mutation_index and error code

#### Scenario: Batch does not exceed mutation limits
- **WHEN** the tile creation batch is submitted
- **THEN** the batch SHALL contain fewer than 1000 mutations (8 mutations: 1 CreateTile + 1 SetTileRoot + 6 InsertNode) and SHALL pass the batch size check

---

### Requirement: Resource Upload Before Tile Creation
Before submitting the tile creation batch, the agent SHALL upload the icon PNG image via the resource upload path. The runtime SHALL return a ResourceId (BLAKE3 content hash of the image bytes). The StaticImageNode in the tile creation batch SHALL reference this ResourceId.

Source: scene-graph spec (ResourceId Identity, Content deduplication)
Scope: v1-mandatory

#### Scenario: Resource uploaded and referenced
- **WHEN** the agent uploads a 48x48 PNG image
- **THEN** the runtime SHALL return a ResourceId (BLAKE3 hash) and the agent SHALL use this ResourceId in the StaticImageNode's resource_id field

#### Scenario: Unknown resource rejected
- **WHEN** the agent submits the tile creation batch with a StaticImageNode referencing a ResourceId that was not previously uploaded
- **THEN** the batch SHALL be rejected with `ResourceNotFound` at the InsertNode mutation for the StaticImageNode

---

### Requirement: Lease Request With AUTO_RENEW
The agent SHALL request a lease before creating the dashboard tile. The LeaseRequest SHALL specify: ttl_ms = 60000, renewal_policy = AUTO_RENEW, capability_scope including `create_tiles` and `modify_own_tiles`, resource_budget with max_tiles >= 1 and max_nodes_per_tile >= 6 and appropriate texture_bytes_total for the icon image. The runtime SHALL respond with LeaseResponse result = GRANTED.

Source: lease-governance spec (Lease State Machine, Auto-Renewal Policy, Operations Requiring a Lease, Lease Identity)
Scope: v1-mandatory

#### Scenario: Lease granted with requested parameters
- **WHEN** the agent sends a LeaseRequest with ttl_ms = 60000, renewal_policy = AUTO_RENEW, and valid capabilities
- **THEN** the runtime SHALL respond with LeaseResponse result = GRANTED, a UUIDv7 LeaseId, the granted TTL, and the allocated resource budget

#### Scenario: Tile creation requires active lease
- **WHEN** the agent attempts to submit the tile creation MutationBatch without a prior ACTIVE lease
- **THEN** the batch SHALL be rejected with `LeaseNotFound` or `LeaseNotActive`

#### Scenario: Lease auto-renews at 75% TTL
- **WHEN** the lease has been active for 45 seconds (75% of 60-second TTL) and the agent session is still connected
- **THEN** the runtime SHALL auto-renew the lease and send LeaseResponse with result = GRANTED and an updated expiry

---

### Requirement: Periodic Content Update
The agent SHALL update the body TextMarkdownNode content every 5 seconds by submitting a MutationBatch containing a single ReplaceNode mutation. The updated content SHALL be valid CommonMark markdown (e.g., including a timestamp, connection uptime, or simulated metrics). The update SHALL require the existing ACTIVE lease.

Source: scene-graph spec (V1 Node Types — TextMarkdownNode content limit); lease-governance spec (Operations Requiring a Lease)
Scope: v1-mandatory

#### Scenario: Successful content update
- **WHEN** the agent submits a ReplaceNode mutation targeting the body TextMarkdownNode with new markdown content
- **THEN** the runtime SHALL accept the batch, replace the node content, and re-render the text in the next frame

#### Scenario: Content update with expired lease rejected
- **WHEN** the agent's lease has expired and it attempts to submit a content update MutationBatch
- **THEN** the batch SHALL be rejected with `LeaseExpired`

#### Scenario: Content does not exceed TextMarkdownNode limit
- **WHEN** the agent updates the body TextMarkdownNode
- **THEN** the content SHALL be fewer than 65535 UTF-8 bytes

---

### Requirement: HitRegionNode Local Feedback
The runtime SHALL provide immediate local visual feedback on the dashboard tile's HitRegionNodes (Refresh and Dismiss buttons) without waiting for agent response. The runtime SHALL update pressed state on PointerDownEvent within p99 < 4ms. The runtime SHALL update hovered state on PointerEnter/PointerLeave within p99 < 4ms. The runtime SHALL render a focus ring on the focused HitRegionNode.

Source: input-model spec (Runtime-Owned Local State Updates, Local Feedback Latency — input_to_local_ack, Local Feedback Defaults and Customization, Focus Ring Visual Indication)
Scope: v1-mandatory

#### Scenario: Button pressed state on pointer down
- **WHEN** a PointerDownEvent lands on the "Refresh" HitRegionNode
- **THEN** the runtime SHALL set pressed = true on that node within p99 < 4ms and apply the default press visual (multiply by 0.85 darkening) without waiting for the agent

#### Scenario: Button hovered state on pointer enter
- **WHEN** the pointer enters the bounds of the "Dismiss" HitRegionNode
- **THEN** the runtime SHALL set hovered = true and apply the default hover visual (0.1 white overlay) within p99 < 4ms

#### Scenario: Focus ring on focused button
- **WHEN** focus transfers to the "Refresh" HitRegionNode via Tab key or click
- **THEN** the runtime SHALL render a 2px focus ring at the node's bounds in the chrome layer

#### Scenario: Pressed state cleared on pointer up
- **WHEN** a PointerUpEvent occurs while the "Refresh" button has pressed = true and release_on_up = true
- **THEN** the runtime SHALL clear pressed = false and release pointer capture

---

### Requirement: Agent Callback on Button Activation
When a HitRegionNode in the dashboard tile is activated (via PointerDown + PointerUp click sequence or CommandInputEvent with action = ACTIVATE), the runtime SHALL dispatch the event to the owning agent via the gRPC session stream's EventBatch. The event SHALL include the node's interaction_id ("refresh-button" or "dismiss-button"), tile_id, node_id, coordinates, device_id, and timestamp_mono_us.

Source: input-model spec (HitRegionNode Primitive, Event Dispatch Flow, Event Routing Resolution, Pointer Event Types, Command Input Model)
Scope: v1-mandatory

#### Scenario: Click on Refresh button dispatches event to agent
- **WHEN** the user clicks (PointerDown + PointerUp) on the "Refresh" HitRegionNode
- **THEN** the agent SHALL receive a ClickEvent in the EventBatch with interaction_id = "refresh-button", the correct tile_id and node_id, and coordinates relative to the node

#### Scenario: ACTIVATE command on focused Dismiss button
- **WHEN** the "Dismiss" HitRegionNode has focus and the user triggers ACTIVATE (e.g., Enter key)
- **THEN** the agent SHALL receive a CommandInputEvent with action = ACTIVATE, interaction_id = "dismiss-button", and source = KEYBOARD

#### Scenario: Agent handles refresh callback
- **WHEN** the agent receives a ClickEvent or CommandInputEvent(ACTIVATE) with interaction_id = "refresh-button"
- **THEN** the agent SHALL submit a content update MutationBatch to refresh the body TextMarkdownNode

#### Scenario: Agent handles dismiss callback
- **WHEN** the agent receives a ClickEvent or CommandInputEvent(ACTIVATE) with interaction_id = "dismiss-button"
- **THEN** the agent SHALL release its lease (LeaseRelease) and the tile SHALL be removed from the scene

---

### Requirement: Focus Cycling Between Buttons
The dashboard tile's two HitRegionNodes (Refresh and Dismiss) SHALL participate in focus cycling. Pressing Tab (NAVIGATE_NEXT) SHALL cycle focus between the two buttons in tree order: Refresh first, then Dismiss, then wrap. Pressing Shift+Tab (NAVIGATE_PREV) SHALL cycle in reverse. Focus cycles through the tile's two HitRegionNodes (Refresh then Dismiss). Cross-tile Tab navigation moves focus to the next tile's first focusable HitRegionNode.

Source: input-model spec (Focus Cycling, Focus Tree Structure, Click-to-Focus Acquisition, Pointer-Free Navigation)
Scope: v1-mandatory

#### Scenario: Tab key cycles through buttons
- **WHEN** the "Refresh" HitRegionNode has focus and the user presses Tab
- **THEN** focus SHALL transfer to the "Dismiss" HitRegionNode, dispatching FocusLostEvent to Refresh and FocusGainedEvent(source=TAB_KEY) to the agent for Dismiss

#### Scenario: Tab key wraps from last to first
- **WHEN** the "Dismiss" HitRegionNode has focus and the user presses Tab
- **THEN** focus SHALL wrap to the "Refresh" HitRegionNode (or advance to the next tile in the cross-tile focus cycle if other tiles exist)

#### Scenario: All buttons reachable without pointer
- **WHEN** the display has no pointer device (pointer-free profile)
- **THEN** both HitRegionNodes SHALL be reachable via NAVIGATE_NEXT/NAVIGATE_PREV commands, and ACTIVATE SHALL trigger the same agent callback as a pointer click

---

### Requirement: Lease Orphan Handling on Disconnect
When the agent disconnects unexpectedly, the dashboard tile's lease SHALL transition from ACTIVE to ORPHANED. The runtime SHALL freeze the tile at its last known state and render a disconnection badge within 1 frame. If the agent reconnects within the grace period (default 30 seconds), the lease SHALL transition back to ACTIVE and the badge SHALL clear. If the grace period expires, the lease SHALL transition to EXPIRED and the tile SHALL be removed.

Source: lease-governance spec (Orphan Handling Grace Period, Grace Period Precision)
Scope: v1-mandatory

#### Scenario: Disconnection triggers orphan state and badge
- **WHEN** the agent's gRPC stream disconnects unexpectedly
- **THEN** the lease SHALL transition to ORPHANED, the tile SHALL be frozen at its last state, and a disconnection badge SHALL appear within 1 frame

#### Scenario: Reconnection within grace period restores tile
- **WHEN** the agent reconnects within 30 seconds of disconnection
- **THEN** the lease SHALL transition back to ACTIVE, the disconnection badge SHALL clear within 1 frame, and the agent can immediately submit mutations

#### Scenario: Grace period expiry removes tile
- **WHEN** the agent fails to reconnect within 30 seconds
- **THEN** the lease SHALL transition to EXPIRED and the dashboard tile (including all nodes) SHALL be removed from the scene graph

---

### Requirement: Lease Expiry Without Renewal Removes Tile
If the agent's lease expires (e.g., because auto-renewal was disabled due to a budget warning, or because the agent stopped renewing a MANUAL lease), the lease SHALL transition to EXPIRED and the dashboard tile SHALL be removed from the scene graph. All resources (icon texture) associated with the tile SHALL be freed.

Source: lease-governance spec (Lease State Machine, Post-Revocation Resource Cleanup, Three-Tier Budget Enforcement Ladder)
Scope: v1-mandatory

#### Scenario: Lease expiry removes tile
- **WHEN** the lease's TTL elapses without renewal
- **THEN** the lease SHALL transition to EXPIRED and the dashboard tile and all its nodes SHALL be removed

#### Scenario: Resources freed after expiry
- **WHEN** the lease expires and the tile is removed
- **THEN** the icon image resource reference count SHALL drop and, if no other tile references it, the resource SHALL be freed

---

### Requirement: Z-Order Compositing at Content Layer
The dashboard tile SHALL render at z_order = 100 in the content layer. It SHALL appear below any zone tiles (z_order >= 0x8000_0000) and below any widget tiles (z_order >= 0x9000_0000). It SHALL appear above any agent tiles with z_order < 100 and below any agent tiles with z_order > 100. The chrome layer (tab bar, system indicators, disconnection badges) SHALL always render above the dashboard tile.

Source: scene-graph spec (Tile Field Invariants, Zone Layer Attachment, Hit-Testing Contract); architecture.md (Compositing model, Layer stack)
Scope: v1-mandatory

#### Scenario: Dashboard tile below zone tiles
- **WHEN** the scene has the dashboard tile at z_order = 100 and a zone tile at z_order = 0x8000_0001
- **THEN** the zone tile SHALL render above the dashboard tile

#### Scenario: Chrome layer above dashboard tile
- **WHEN** the dashboard tile is visible and a chrome element (e.g., tab bar) overlaps its bounds
- **THEN** the chrome element SHALL render above the dashboard tile

#### Scenario: Hit-test respects z-order
- **WHEN** the dashboard tile at z_order = 100 overlaps with another agent tile at z_order = 200
- **THEN** a pointer event in the overlap region SHALL hit the z_order = 200 tile, not the dashboard tile

---

### Requirement: Headless Test Coverage
All exemplar behaviors SHALL be testable in a headless environment (no display server, no physical GPU). Hit-testing, node composition, lease state transitions, mutation validation, and event dispatch SHALL be exercisable via Layer 0 tests with injected synthetic events.

Source: input-model spec (Headless Testability); scene-graph spec (Transaction Validation Performance)
Scope: v1-mandatory

#### Scenario: Headless tile creation test
- **WHEN** a Layer 0 test creates the dashboard tile via injected MutationBatch
- **THEN** the scene graph SHALL contain the tile with all 6 nodes in the correct tree order, verifiable without a GPU

#### Scenario: Headless input test
- **WHEN** a Layer 0 test injects a synthetic PointerDownEvent at coordinates within the "Refresh" HitRegionNode bounds
- **THEN** the hit-test SHALL return a NodeHit for the Refresh node with interaction_id = "refresh-button"

#### Scenario: Headless lease expiry test
- **WHEN** a Layer 0 test simulates time advancement past the lease TTL without renewal
- **THEN** the lease SHALL transition to EXPIRED and the tile SHALL be removed from the scene graph

---

### Requirement: Full Lifecycle User-Test Scenario
The exemplar SHALL define a user-test scenario covering the complete lifecycle: (1) agent establishes gRPC session, (2) agent requests lease with AUTO_RENEW, (3) agent uploads icon resource, (4) agent submits atomic tile creation batch, (5) tile appears with all nodes rendered, (6) agent periodically updates body content, (7) user clicks "Refresh" button and agent receives callback, (8) user clicks "Dismiss" button and agent releases lease, (9) tile disappears cleanly.

Source: all referenced specs combined
Scope: v1-mandatory

#### Scenario: End-to-end lifecycle completes successfully
- **WHEN** the full lifecycle is executed in order: session connect, lease request, resource upload, tile creation batch, content update, Refresh click, Dismiss click
- **THEN** each step SHALL succeed, the agent SHALL receive the expected events and responses at each step, and the tile SHALL be cleanly removed on Dismiss

#### Scenario: Disconnect during lifecycle triggers orphan path
- **WHEN** the agent's session disconnects unexpectedly after tile creation but before Dismiss
- **THEN** the tile SHALL enter orphan state with a disconnection badge, and SHALL be cleaned up after the grace period expires

#### Scenario: Namespace isolation during lifecycle
- **WHEN** the dashboard agent creates its tile
- **THEN** no other agent session SHALL be able to mutate or delete the dashboard tile, and attempts to do so SHALL be rejected with `CapabilityMissing` or `LeaseNotFound`
