## Context

tze_hud supports three display abstractions — zones (runtime-rendered, zero-geometry), widgets (parameterized SVG templates), and raw tiles (full compositor control). Zones and widgets cover the common publishing patterns; raw tiles are the escape hatch for custom layouts that no zone or widget supports.

The presence card is a custom agent identity tile: an expanded glass card showing an avatar, metadata eyebrow, bold agent name, live status line, compact time chip, and a small human dismiss affordance. This is genuinely custom layout — no zone type or widget type covers "agent identity display with live status updates and multi-agent stacking." It uses the raw tile API directly: the agent requests a lease, creates a tile with geometry, inserts a layered flat node tree, and mutates the status nodes periodically.

The component-shape-language epic is fully implemented, but raw tiles are styled directly via node properties (RGBA colors, font sizes, markdown content), not through design tokens or RenderingPolicy. This exemplar intentionally exercises the direct-styling path.

Existing spec references:
- Scene graph spec: tile CRUD, V1 node types, atomic batch mutations, namespace isolation
- Lease governance spec: lease state machine, `AutoRenew` policy, orphan handling, disconnection badge, grace period
- Session protocol spec: single bidirectional gRPC stream, heartbeat, ClientMessage/ServerMessage envelopes

## Goals / Non-Goals

**Goals:**
- Define the canonical reference for a raw-tile agent lifecycle: authenticate → lease → create tile → insert nodes → update content → renew lease → disconnect → orphan → cleanup
- Prove multi-agent coexistence: 3 agents each hold their own presence card tile on the same tab, vertically stacked without overlap
- Demonstrate lease lifecycle observability: active → orphaned (with disconnection badge) → expired (tile removed)
- Provide gRPC test sequences (MutationBatch payloads) that exercise CreateTile, layered AddNode assembly, and SetTileRoot (content updates)
- Provide a user-test scenario exercising the disconnect → staleness badge → cleanup pipeline visually

**Non-Goals:**
- Rich input handling beyond a single dismiss affordance
- Zone or widget publishing (this exemplar uses only the raw tile API)
- Design token integration (node properties are specified directly)
- Agent-to-agent communication (agents are independent; no coordination beyond layout offset calculation)
- True blur, rounded clipping, or compositor-level glass effects beyond layered translucent solids

## Decisions

### 1. Tile geometry: fixed 320x112, bottom-left corner, vertical stacking

**Choice**: Each agent's presence card is a 320x112 tile anchored to the bottom-left corner of the tab. Agent cards are stacked vertically with 12px gaps: agent 0 at `y = tab_height - 112 - 24`, agent 1 at `y = tab_height - 224 - 12 - 24`, agent 2 at `y = tab_height - 336 - 24 - 24`. The x-offset is 24px from the left edge.

**Rationale**: The redesign needs room for hierarchy and breathing space. 320x112 supports a premium raw-tile composition: accent rail, avatar plate, metadata eyebrow, stronger name line, longer status line, and a time chip without crowding. The bottom-left corner remains conventionally unused by primary content, and fixed spacing prevents overlap.

**Alternative considered**: Dynamic layout via querying tab dimensions and dividing space — over-engineering for an identity card. Corner anchoring is the natural fit.

### 2. Node tree: 13-node glass stack

**Choice**: Each presence card tile uses a flat 13-node stack: one `SolidColorNode` root plus 12 child nodes for sheen, accent rail, avatar plate, avatar image, eyebrow, name, status, chip background, chip text, dismiss background, dismiss label, and dismiss hit region. The visual direction is "glass HUD" expressed through layered translucent solids and cool typography because raw tiles do not offer blur or rounded clipping.

**Rationale**: Flat tree (depth=1) is still the simplest structure that achieves the layout. The root provides the dark glass slab, the child solids create the glass sheen and accent treatment, and the text nodes establish hierarchy without requiring any nested layout primitives.

> **Implementation note:** The glass effect is an approximation built from layered f32 RGBA fills: dark root slab, 2px top sheen, translucent accent plate, and a small chip backdrop. The orphan badge remains runtime-owned and sits outside the tile node tree.

**Alternative considered**: Nested layout nodes for flex-like behavior — unnecessary complexity; fixed positions are sufficient for the expanded card and keep the exemplar aligned with the v1 raw-node surface.

### 3. Lease policy: `AutoRenew` with 120s TTL

**Choice**: Each agent requests a lease with `ttl_ms = 120000` (2 minutes) and capabilities [create_tiles, modify_own_tiles]. The server-side lease state machine is configured with `AutoRenew` renewal policy. The runtime auto-renews at 75% TTL (90s) as long as the session is active.

> **Implementation note:** The LeaseRequest proto carries `ttl_ms`, `capabilities` (repeated string), and `lease_priority`. Renewal policy (`AutoRenew`) is a server-side / Rust-layer concern, not a LeaseRequest wire field.

**Rationale**: `AutoRenew` minimizes agent complexity -- the agent does not need to implement a renewal timer. 120s TTL provides a comfortable window: the 75% renewal fires at 90s, leaving 30s of margin before expiry. If the agent disconnects, the orphan grace period (30s default) handles reconnection within the TTL window.

**Alternative considered**: Manual renewal with explicit LeaseRequest at intervals -- adds unnecessary agent-side timer logic for a simple display card. One-shot -- inappropriate for a persistent presence card.

### 4. Content updates: `SetTileRoot` for status line and chip every 30s

**Choice**: Each agent submits a `MutationBatch` with a single `SetTileRoot` mutation every 30 seconds, providing the complete node tree with the status line updated to `Connected • last active ...` and the time chip updated to `NOW` / `30S` / `1M`. The visual layers, avatar treatment, eyebrow, name, and dismiss control remain unchanged in the rebuilt tree.

> **Implementation note:** There is no `ReplaceNode` variant in `SceneMutation`. To update the card, the agent rebuilds the full flat tree and submits it via `SetTileRoot`. For a 13-node flat tree this is still cheap enough for a 30-second cadence.

### 5. Human override: top-right dismiss button

**Choice**: The card uses `input_mode = Capture` and reserves a compact 24x24 dismiss control in the top-right corner. The visible affordance is a subtle glass square with a centered `X`, and an overlaid `HitRegionNode` dispatches `interaction_id = "dismiss-card"` back to the owning resident agent. When clicked, the agent closes its own session and the tile clears immediately.

**Rationale**: Presence doctrine requires that the human can always override. A small explicit dismiss control is enough to satisfy that requirement without turning the card into a full interactive widget.

**Rationale**: 30s update interval is low-overhead and keeps the card feeling alive. The dual expression of recency — natural-language status plus compact chip — improves scanability, especially in a multi-agent stack.

**Alternative considered**: Using individual `AddNode` mutations after a hypothetical remove -- unnecessary complexity; `SetTileRoot` handles full tree replacement atomically.

### 5. Z-order assignment: sequential per-agent, below ZONE_TILE_Z_MIN

**Choice**: Agent 0 gets z_order = 100, agent 1 gets z_order = 101, agent 2 gets z_order = 102. All values are well below ZONE_TILE_Z_MIN (0x8000_0000).

**Rationale**: Sequential z-order prevents ZOrderConflict rejections (tiles do not overlap spatially due to vertical stacking, but unique z-orders are good practice). Low values keep presence cards below all zone/widget tiles and most content tiles. The specific values (100-102) leave room for future tile types below and above.

**Alternative considered**: Same z-order for all cards (valid since they don't overlap spatially) — fragile; any future layout change that introduces overlap would break.

### 6. Disconnect scenario: agent 2 disconnects, badge appears, grace period expires

**Choice**: The test scenario disconnects agent 2 (top card in the stack). The runtime detects disconnect via heartbeat timeout (15s = 3 missed heartbeats at 5s interval), transitions the lease to ORPHANED, renders a disconnection badge on agent 2's tile within 1 frame, and starts the 30s grace period. If agent 2 does not reconnect, the lease transitions to EXPIRED and the tile is removed.

**Rationale**: Disconnecting the top card in the stack is visually unambiguous — the badge and eventual removal are clearly visible. The two remaining cards (agents 0 and 1) continue operating normally, proving that disconnection is isolated per-agent.

## Risks / Trade-offs

- **[Risk] Tab dimensions not known at tile creation time** → Mitigation: The exemplar assumes a minimum tab size of 800x600. Agents query tab dimensions from the SceneSnapshot received at session establishment and compute y-offsets accordingly. If the tab is smaller than expected, cards may be partially off-screen — acceptable for a reference exemplar.
- **[Risk] Avatar ResourceId requires prior upload** → Mitigation: The test sequence includes a resource upload step (UploadResource with a small PNG) before tile creation. The exemplar spec defines placeholder 32x32 PNGs for each agent (colored squares: blue, green, orange).
- **[Trade-off] Fixed positions vs. responsive layout** → Fixed positions are simpler and sufficient for a reference exemplar. A production presence system would query tab geometry and adapt. The exemplar prioritizes clarity over adaptability.
- **[Trade-off] Capture input mode vs. click-through tile** → Capture is necessary to host a real dismiss action, but it means the card area itself is no longer fully click-through. For this exemplar, that trade-off is acceptable because the card occupies a small bottom-left footprint and the dismiss affordance is doctrinally valuable.
- **[Trade-off] Flat node tree vs. nested layout** → Flat is simpler but less flexible. If presence cards needed more complex internal layout (e.g., progress bars, multiple status lines), nesting would be necessary. For v1, flat is correct.
