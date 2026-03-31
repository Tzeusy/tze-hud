## Context

The ambient-background zone is one of six built-in v1 zone types. It is already registered in `ZoneRegistry::with_defaults()` with:
- `ContentionPolicy::Replace` / `max_publishers: 1` (functionally latest-wins)
- `LayerAttachment::Background` (lowest z-order, behind all content and chrome)
- Full-screen `Relative { x_pct: 0.0, y_pct: 0.0, width_pct: 1.0, height_pct: 1.0 }` geometry
- Accepts `ZoneMediaType::SolidColor`, `ZoneMediaType::StaticImage`, `ZoneMediaType::VideoSurfaceRef`
- Component type: `ambient-background` with `ReadabilityTechnique::None`, zero required tokens

The compositor already renders `ZoneContent::SolidColor` as a filled quad at zone geometry. `ZoneContent::StaticImage` rendering is placeholder (warm-gray quad). The MCP `publish_to_zone` tool dispatches `solid_color` but not `static_image` content type objects.

The component-shape-language epic is fully implemented. Since `ambient-background` has no token-driven rendering policy fields (no text, no backdrop, no outline), a component profile is unnecessary — the exemplar exercises the zone directly.

## Goals / Non-Goals

**Goals:**
- Prove end-to-end ambient-background zone publishing works for SolidColor content via MCP and direct API
- Prove Replace contention policy correctly replaces background on each new publish
- Prove background-layer z-ordering renders behind all content-layer tiles
- Add `static_image` content type dispatch to MCP `publish_to_zone`
- Provide user-test scenarios for agent-driven ambient background control
- Document overlay/HUD mode behavior: background layer is fully transparent (passthrough) by default when no content is published

**Non-Goals:**
- GPU texture pipeline for StaticImage (remains placeholder warm-gray quad in v1 vertical slice; full texture upload is a separate follow-up)
- Animated transitions / opacity crossfade between backgrounds (deferred per v1 compositing scope)
- VideoSurfaceRef content (post-v1)
- Component profile authoring (no token-driven fields to override for this zone type)
- Gallery/cycling content (post-v1 extension of StaticImage)

## Decisions

### 1. Use existing `ContentionPolicy::Replace` as-is

**Choice**: The zone registry defines `ContentionPolicy::Replace` with `max_publishers: 1`. This is functionally identical to "latest-wins" for a single-occupant zone — each new publish evicts the previous one.

**Rationale**: The doctrine describes ambient-background as "latest-wins" contention. `ContentionPolicy::Replace` implements this semantic exactly when `max_publishers: 1`. No code change needed.

**Alternative considered**: Adding a dedicated `LatestWins` variant — unnecessary; `Replace` already expresses the intent and the renderer already handles it.

### 2. MCP `static_image` content type dispatch: ResourceId from string

**Choice**: Add `"static_image"` as a recognized content type in `parse_zone_content()` in `crates/tze_hud_mcp/src/tools.rs`. The JSON shape: `{"type": "static_image", "resource_id": "<hash-string>"}`. The `resource_id` field is parsed as a hex-encoded SHA-256 content-address string and converted to `ResourceId`.

**Rationale**: Follows the same pattern as `solid_color` dispatch. Agents must upload resources separately (via the resource store, when available) and then reference them by content-address. This keeps publishing lightweight — no binary blobs in JSON-RPC.

**Alternative considered**: Embedding base64 image data in the publish call — violates the MCP payload size contract and the content-addressed resource model.

### 3. Tests validate renderer output, not just scene state

**Choice**: Integration tests publish to the ambient-background zone, run a render frame, and assert pixel output — specifically that the background color appears at full-screen coordinates and renders behind any content-layer tiles.

**Rationale**: Scene-state-only tests miss renderer bugs. The existing test infrastructure (`make_compositor_and_surface`) supports pixel inspection. The exemplar's value is proving the rendering path, not just the data model.

### 4. TTL semantics: 0 = persistent until replaced

**Choice**: Publishing with `ttl_us: 0` (or omitting TTL) means the background persists until explicitly replaced by another publish. This aligns with the zone's `auto_clear_ms: None` default.

**Rationale**: Ambient backgrounds are not ephemeral. They should persist until intentionally changed. The MCP handler already treats `ttl_us: 0` as "use zone default" which for ambient-background is no auto-clear.

## Risks / Trade-offs

- **[Risk] StaticImage renders as placeholder quad** → Mitigation: Accepted for v1 vertical slice. The exemplar spec calls out that StaticImage rendering is visually incomplete but structurally correct (resource reference flows through, geometry is correct, pixel output is a warm-gray stand-in). Full texture upload is tracked separately.
- **[Risk] No crossfade transition between backgrounds** → Mitigation: v1 compositing scope explicitly defers animated opacity transitions. The exemplar documents this as a known limitation. Replacement is instant (frame-boundary swap).
- **[Trade-off] No overlay transparency test** → In overlay/HUD mode the background layer is transparent by default (passthrough to desktop). Testing this requires a real windowed compositor with `SetWindowCompositionAttribute` (Windows) or equivalent. The exemplar specifies the behavior but defers visual validation to the user-test scenario on a real display.
