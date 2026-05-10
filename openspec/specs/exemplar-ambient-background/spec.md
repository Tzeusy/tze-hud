# exemplar-ambient-background Specification

## Purpose
Defines the ambient-background exemplar zone contract for full-display background-layer content, latest-wins publication behavior, and transparent/clear fallback rendering.

## Requirements
### Requirement: Ambient Background Zone Visual Contract

The ambient-background zone MUST render as a full-screen surface in the background layer, behind all content-layer tiles and chrome-layer zones. When no content is published to the zone, the background layer MUST be fully transparent (passthrough to underlying desktop in overlay/HUD mode) or the runtime's default clear color (in windowed mode).

The zone accepts two v1 content types:
- `SolidColor(Rgba)`: A full-screen single-color fill at the zone's geometry (100% width, 100% height).
- `StaticImage(ResourceId)`: A content-addressed image reference scaled to fill the display. In v1 vertical slice, this renders as a placeholder quad (warm-gray) because the GPU texture upload pipeline is deferred.

The zone has `ReadabilityTechnique::None` — no text rendering, no backdrop, no outline validation. No component profile tokens are required.

Scope: v1-mandatory

#### Scenario: Solid color background fills the display

- **WHEN** an agent publishes `ZoneContent::SolidColor(Rgba { r: 0.1, g: 0.1, b: 0.3, a: 1.0 })` to the `ambient-background` zone
- **THEN** the compositor MUST render a full-screen quad at the zone's geometry with the specified RGBA color in the background layer pass

#### Scenario: No publication yields transparent/clear background

- **WHEN** no content has been published to the `ambient-background` zone
- **THEN** the background layer MUST render as the runtime's clear color (or fully transparent in overlay mode) with no zone-specific visual output

#### Scenario: StaticImage renders placeholder in v1

- **WHEN** an agent publishes `ZoneContent::StaticImage(resource_id)` to the `ambient-background` zone
- **THEN** the compositor MUST render a full-screen quad at the zone's geometry with a placeholder color (warm-gray) in the background layer pass
- **AND** the `resource_id` MUST be preserved in the zone's active publication record for future GPU texture pipeline use

---

### Requirement: Ambient Background Latest-Wins Contention

The ambient-background zone MUST use `ContentionPolicy::Replace` with `max_publishers: 1`. Each new publication MUST immediately replace the current background content. The displaced content is discarded — no queue, no history, no eviction notification.

This is the "latest-wins" semantic: only the most recently published content is rendered.

Scope: v1-mandatory

#### Scenario: New solid color replaces previous solid color

- **WHEN** an agent publishes `SolidColor(Rgba { r: 1.0, g: 0.0, b: 0.0, a: 1.0 })` (red) to `ambient-background`
- **AND** subsequently publishes `SolidColor(Rgba { r: 0.0, g: 0.0, b: 1.0, a: 1.0 })` (blue)
- **THEN** the zone's active publication count MUST be 1
- **AND** the rendered background MUST be blue (the second publish)

#### Scenario: Static image replaces solid color

- **WHEN** an agent publishes `SolidColor(Rgba { r: 0.0, g: 0.2, b: 0.4, a: 1.0 })` to `ambient-background`
- **AND** subsequently publishes `StaticImage(resource_id)` to the same zone
- **THEN** the zone's active publication count MUST be 1
- **AND** the rendered content MUST reflect the StaticImage publication (placeholder quad in v1)

#### Scenario: Rapid replacement under contention

- **WHEN** an agent publishes N solid color backgrounds in rapid succession (N >= 10 within a single frame interval)
- **THEN** after the frame renders, only the last-published color MUST be visible
- **AND** the zone's active publication list MUST contain exactly 1 record

---

### Requirement: Background Layer Z-Order

The ambient-background zone MUST render in the compositor's background layer pass, which executes before the content layer pass and the chrome layer pass. All content-layer tiles (agent tiles, zone tiles) and chrome-layer zones (status-bar, alert-banner) MUST render on top of the ambient-background, occluding it where they overlap.

Scope: v1-mandatory

#### Scenario: Content tile renders on top of background

- **WHEN** an agent publishes `SolidColor(Rgba { r: 0.0, g: 0.0, b: 0.5, a: 1.0 })` to `ambient-background`
- **AND** a content-layer tile with `SolidColor(Rgba { r: 1.0, g: 0.0, b: 0.0, a: 1.0 })` exists at the center of the display
- **THEN** the pixel at the tile's position MUST be red (tile occludes background)
- **AND** the pixel outside the tile's bounds MUST be dark blue (background visible)

#### Scenario: Chrome zone renders on top of background

- **WHEN** an agent publishes a solid color to `ambient-background`
- **AND** a status-bar zone has active publications in the chrome layer
- **THEN** the status-bar pixels MUST occlude the background at the status-bar's geometry

---

### Requirement: MCP StaticImage Content Type Dispatch

The MCP `publish_to_zone` handler MUST accept `static_image` as a content type in the `content` parameter object. The JSON shape MUST be:

```json
{"type": "static_image", "resource_id": "<hex-encoded-sha256>"}
```

The handler MUST parse the `resource_id` field and construct a `ZoneContent::StaticImage(ResourceId)`. If `resource_id` is missing or empty, the handler MUST return an `invalid_params` error.

Scope: v1-mandatory

#### Scenario: Publish static image via MCP

- **WHEN** an MCP caller sends `publish_to_zone` with `{"zone_name": "ambient-background", "content": {"type": "static_image", "resource_id": "abc123def456"}}`
- **THEN** the zone's active publication MUST contain `ZoneContent::StaticImage(ResourceId)` with the corresponding resource ID

#### Scenario: Missing resource_id returns error

- **WHEN** an MCP caller sends `publish_to_zone` with `{"zone_name": "ambient-background", "content": {"type": "static_image"}}`
- **THEN** the handler MUST return an `invalid_params` error indicating that `resource_id` is required

#### Scenario: Solid color via MCP to ambient-background

- **WHEN** an MCP caller sends `publish_to_zone` with `{"zone_name": "ambient-background", "content": {"type": "solid_color", "r": 0.05, "g": 0.05, "b": 0.15, "a": 1.0}}`
- **THEN** the zone's active publication MUST contain `ZoneContent::SolidColor(Rgba { r: 0.05, g: 0.05, b: 0.15, a: 1.0 })`

---

### Requirement: Ambient Background TTL Semantics

Publishing to the ambient-background zone with `ttl_us: 0` (or omitting TTL) MUST result in persistent content that remains until explicitly replaced by a subsequent publish. The zone's `auto_clear_ms` is `None`, meaning no automatic expiry.

A non-zero TTL MUST cause the content to expire after the specified duration, at which point the background reverts to the default state (transparent/clear).

Scope: v1-mandatory

#### Scenario: Zero TTL persists until replaced

- **WHEN** an agent publishes `SolidColor(dark_blue)` with `ttl_us: 0` to `ambient-background`
- **AND** 60 seconds elapse with no further publishes
- **THEN** the background MUST still render dark_blue

#### Scenario: Non-zero TTL expires

- **WHEN** an agent publishes `SolidColor(warm_amber)` with a non-zero TTL to `ambient-background`
- **AND** the TTL elapses
- **THEN** the zone's active publication list MUST be empty
- **AND** the background MUST revert to the default clear/transparent state

---

### Requirement: Ambient Background User-Test Scenarios

The exemplar MUST include user-test scenarios exercisable via the `user-test` skill or equivalent MCP-based test harness, proving the ambient-background zone works as an agent-controlled display surface.

Scope: v1-mandatory

#### Scenario: Agent sets solid color background

- **WHEN** an agent publishes `{"type": "solid_color", "r": 0.05, "g": 0.05, "b": 0.2, "a": 1.0}` (dark blue) to `ambient-background` via MCP `publish_to_zone`
- **THEN** the HUD display background MUST change to dark blue
- **AND** the zone occupancy query MUST show 1 active publication with SolidColor content

#### Scenario: Agent replaces background with warm amber

- **WHEN** an agent publishes dark blue to `ambient-background`
- **AND** subsequently publishes `{"type": "solid_color", "r": 0.9, "g": 0.6, "b": 0.2, "a": 1.0}` (warm amber)
- **THEN** the HUD display background MUST change to warm amber
- **AND** the previous dark blue MUST no longer be visible

#### Scenario: Agent publishes static image background

- **WHEN** an agent publishes `{"type": "static_image", "resource_id": "<valid-resource-id>"}` to `ambient-background` via MCP
- **THEN** the zone MUST accept the publication
- **AND** the rendered output MUST reflect the StaticImage content (placeholder in v1)

#### Scenario: Rapid replacement stress test

- **WHEN** an agent publishes 10 different solid colors to `ambient-background` in rapid succession via MCP
- **THEN** only the last-published color MUST be visible after rendering
- **AND** the zone occupancy MUST show exactly 1 active publication
