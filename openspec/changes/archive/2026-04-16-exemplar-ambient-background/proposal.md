## Why

The ambient-background zone type is defined in doctrine and registered in the zone registry, but no polished exemplar exists that exercises it end-to-end: publishing solid colors, static images, verifying latest-wins contention replacement, confirming background-layer z-order (behind all tiles), and validating the overlay/HUD transparency passthrough default. Without an exemplar, the ambient-background zone is specified but unproven — its rendering path, contention semantics, and MCP publishing surface remain untested as a cohesive capability.

## What Changes

- **Exemplar spec**: A self-contained exemplar specification defining the ambient-background zone's visual contract (full-screen, lowest z-order), accepted content types (SolidColor Rgba, StaticImage ResourceId), contention behavior (latest-wins/replace: new publish replaces current background immediately), and overlay-mode transparency default.
- **MCP content type gap**: The MCP `publish_to_zone` handler currently supports `solid_color` content dispatch but lacks `static_image` content type dispatch — this exemplar identifies that as a required implementation item.
- **Rendering validation**: Tests that confirm the compositor renders the ambient-background zone in the background layer pass, behind all content-layer tiles and chrome-layer zones.
- **User-test integration**: A test scenario exercising the full agent workflow: publish solid color, replace with image, verify rapid replacement (contention), inspect occupancy.

## Capabilities

### New Capabilities

- `exemplar-ambient-background`: Defines the exemplar contract for the ambient-background zone — visual requirements, behavioral requirements, content types, contention semantics, rendering expectations, and test/user-test scenarios.

### Modified Capabilities

_(none — existing zone registry, contention policy, and rendering pipeline are unchanged; this exemplar exercises them as-is)_

## Impact

- **Existing code**: Renderer already handles `ZoneContent::SolidColor` for zone backgrounds. `StaticImage` zone content rendering is placeholder (warm-gray quad). The MCP `publish_to_zone` tool does not yet dispatch `static_image` content type objects.
- **Tests**: New integration tests in compositor/renderer exercising ambient-background zone publish → render cycle for both SolidColor and StaticImage content.
- **User-test**: New test scenario in the user-test skill exercising ambient-background zone publishing via MCP.
- **Dependencies**: None — all types (`ZoneContent::SolidColor`, `ZoneContent::StaticImage`, `ResourceId`, `ContentionPolicy::Replace`, `LayerAttachment::Background`) already exist in `tze_hud_scene`.
