## Context

The v1 MVP specifications define the complete raw tile API surface: four node types, atomic batch mutations, lease governance state machine, z-order compositing, and input capture with local feedback. These specs are individually well-defined but have never been exercised together in a single concrete scenario. This exemplar proves the APIs compose correctly by defining a polished, interactive dashboard tile that a resident agent creates, populates, interacts with, and eventually releases.

This exemplar uses the **raw tile API exclusively** — no zones, no widgets, no design tokens. The agent directly creates tiles, inserts nodes, manages leases, and handles input events over the gRPC session stream. This is the full-control path described in `about/heart-and-soul/presence.md` under "Relationship to raw tiles."

The component-shape-language system (zones, widgets, design tokens, component profiles) is assumed fully implemented but is irrelevant to this exemplar. The dashboard tile demonstrates what an agent does when it needs custom layout that no zone or widget covers.

## Goals / Non-Goals

**Goals:**

- Prove all four v1 node types (SolidColorNode, TextMarkdownNode, StaticImageNode, HitRegionNode) compose correctly in a single tile via intra-tile tree-order compositing (painter's model).
- Prove the full gRPC session lifecycle: connect, authenticate, request lease, create tile + nodes atomically, receive input events, update content, release/disconnect.
- Prove lease governance integration: TTL with AUTO_RENEW, orphan handling on disconnect, disconnection badge rendering, and lease expiry leading to tile cleanup.
- Prove input capture: HitRegionNode local feedback (pressed/hovered in < 4ms), focus semantics, event routing to the owning agent, and agent callback on ACTIVATE.
- Provide a concrete, testable scenario for headless CI (Layer 0 unit tests + integration tests without GPU).
- Serve as a developer reference for building interactive tiles with the raw API.

**Non-Goals:**

- Zone publishing, widget parameterization, or design token usage (this is raw tile API only).
- Multi-tile coordination or cross-tile gestures.
- Media plane (WebRTC) integration — no video surfaces or audio.
- Scroll behavior — the dashboard tile is a fixed-size content panel.
- Cross-agent interaction or orchestrator patterns.
- Performance benchmarking (covered by existing spec performance requirements).
- Mobile display profile adaptation (desktop profile only for the exemplar).

## Decisions

### Decision 1: Single tile with flat node tree (no nesting)

The exemplar uses a single tile with a flat list of child nodes under the tile root. All four node types are direct children of the root, composited in tree order:

1. SolidColorNode (background — dark semi-transparent, e.g., `Rgba { r: 0.07, g: 0.07, b: 0.07, a: 0.90 }`)
2. StaticImageNode (agent icon/logo — top-left corner, 48x48)
3. TextMarkdownNode (header — agent name, bold, positioned top alongside icon)
4. TextMarkdownNode (body — markdown-formatted live stats/info, positioned below header)
5. HitRegionNode ("Refresh" button — bottom-left, accepts_focus + accepts_pointer + auto_capture)
6. HitRegionNode ("Dismiss" button — bottom-right, accepts_focus + accepts_pointer + auto_capture)

**Rationale:** A flat tree is the simplest structure that still exercises all node types and the painter's model compositing. Nested subtrees would add complexity without proving additional API surface for the exemplar's purpose. The spec allows up to 64 nodes per tile; 6 is well within budget and easy to reason about.

**Alternative considered:** Nested tree (e.g., a "button group" parent node containing both HitRegionNodes). Rejected because grouping nodes is a layout convenience that the compositor does not optimize differently in v1 — tree order is tree order regardless of depth.

### Decision 2: Atomic batch for initial tile creation

The entire tile (CreateTile + 6x InsertNode) MUST be submitted as a single MutationBatch. The user MUST never see a partially-constructed tile.

**Rationale:** This directly validates the atomic batch mutation requirement from the scene-graph spec. If any node fails insertion (e.g., bounds validation), the entire batch is rejected and no tile appears. This is the doctrinal "agents never expose intermediate state" principle from `presence.md`.

### Decision 3: AUTO_RENEW lease with 60-second TTL

The exemplar agent requests a lease with `ttl_ms = 60000` and `renewal_policy = AUTO_RENEW`. The runtime auto-renews at 75% TTL (45 seconds). This keeps the tile alive indefinitely while the agent is connected, with a clean expiry path on disconnect.

**Rationale:** AUTO_RENEW is the natural policy for a persistent dashboard tile. MANUAL would require the agent to implement renewal logic (extra complexity for the exemplar). ONE_SHOT would cause the tile to disappear after 60 seconds, which is wrong for a dashboard. The 60-second TTL is long enough to be realistic but short enough that orphan expiry is observable in tests.

**Alternative considered:** Indefinite TTL (`ttl_ms = 0`). Rejected because it doesn't exercise the TTL/renewal machinery, which is a key validation target.

### Decision 4: interaction_id-based button callbacks

Each HitRegionNode carries a distinct `interaction_id` string: `"refresh-button"` and `"dismiss-button"`. When the agent receives a ClickEvent or CommandInputEvent(ACTIVATE), it matches on `interaction_id` to determine which button was activated.

**Rationale:** `interaction_id` is the spec-defined mechanism for agent-side event disambiguation. Using it in the exemplar validates the field flows correctly through hit-testing, event routing, and serialization.

### Decision 5: Periodic content update via MutationBatch

The agent periodically (every 5 seconds) updates the body TextMarkdownNode content with fresh markdown (e.g., live stats, timestamp, connection uptime). This is done via a MutationBatch containing a single ReplaceNode mutation targeting the body node.

**Rationale:** This validates that an agent can update node content within an existing tile using the atomic mutation path, and that the compositor re-renders the updated text correctly. The 5-second interval is low-frequency enough to avoid hitting rate limits but frequent enough to be observable.

### Decision 6: Tile geometry: 400x300 at content layer

The tile is 400x300 logical pixels, positioned at (50, 50) within the tab's content layer. Z-order is set to a low value in the agent-owned range (e.g., `z_order = 100`), well below `ZONE_TILE_Z_MIN` (0x8000_0000). Opacity is 1.0 (the SolidColorNode background handles the visual transparency via its alpha channel).

**Rationale:** 400x300 is a reasonable dashboard card size. Placing it at (50, 50) keeps it visible and avoids edge effects. The low z-order ensures it doesn't conflict with higher-priority tiles. Using tile opacity = 1.0 with a semi-transparent background node is the correct pattern: the tile is fully opaque to the compositor (participates normally in z-order), but the background node's alpha blends with whatever is behind it in overlay mode.

### Decision 7: Resource upload for icon image

Before the tile creation batch, the agent uploads a PNG image resource via the resource upload path. The resulting `ResourceId` (BLAKE3 content hash) is referenced by the StaticImageNode. This validates the resource upload + reference workflow.

**Rationale:** StaticImageNode requires a ResourceId. Skipping the upload would leave this node type untested. The icon is a small PNG (48x48, < 10KB) — minimal resource budget impact.

## Risks / Trade-offs

**[Risk] Exemplar becomes stale as specs evolve** — The exemplar references specific field names, message types, and behaviors from the current v1 specs. If specs change, the exemplar must be updated.
Mitigation: The exemplar is a change artifact, not a living document. It will be archived after implementation. The implementation tests themselves are the living validation.

**[Risk] Flat node tree doesn't exercise deeper compositing** — A flat tree validates painter's-model ordering but not deeply nested node trees.
Mitigation: Acceptable for the exemplar scope. Deeper nesting is a scene-graph unit test concern, not an exemplar concern.

**[Trade-off] Fixed geometry vs. responsive layout** — The exemplar uses hardcoded pixel positions (400x300 at (50,50)). Real agents would compute geometry based on the display profile.
Mitigation: The exemplar's purpose is to validate the API, not to demonstrate responsive design. Display profile adaptation is a separate concern.

**[Trade-off] Single tile vs. multi-tile** — The exemplar creates only one tile. Multi-tile coordination (z-order stacking, lease budgeting across tiles) is not exercised.
Mitigation: Multi-tile scenarios are a separate exemplar concern. This exemplar focuses on depth (all node types, full lifecycle) rather than breadth (many tiles).
