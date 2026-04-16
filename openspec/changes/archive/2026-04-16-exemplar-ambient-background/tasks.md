## 1. MCP StaticImage Content Type Dispatch

- [ ] 1.1 Add `"static_image"` arm to `parse_zone_content()` in `crates/tze_hud_mcp/src/tools.rs` — parse `{"type": "static_image", "resource_id": "<hex>"}` into `ZoneContent::StaticImage(ResourceId)`
- [ ] 1.2 Return `invalid_params` error when `resource_id` is missing or empty in a `static_image` content object
- [ ] 1.3 Update the unknown-type error message to include `static_image` in the list of valid content types
- [ ] 1.4 Add unit test: `test_publish_to_zone_static_image` — publish `static_image` content, assert `ZoneContent::StaticImage` in active publications
- [ ] 1.5 Add unit test: `test_publish_to_zone_static_image_missing_resource_id` — assert `invalid_params` error

## 2. Compositor Ambient Background Rendering Tests

- [ ] 2.1 Add integration test: `test_ambient_background_solid_color_renders` — publish `SolidColor(dark_blue)` to `ambient-background` zone, render frame, assert pixels at full-screen coordinates match the published color
- [ ] 2.2 Add integration test: `test_ambient_background_static_image_placeholder` — publish `StaticImage(resource_id)` to `ambient-background` zone, render frame, assert placeholder warm-gray pixels appear at full-screen coordinates
- [ ] 2.3 Add integration test: `test_ambient_background_replacement_contention` — publish red then blue to `ambient-background`, render frame, assert only blue pixels appear and active publication count is 1
- [ ] 2.4 Add integration test: `test_ambient_background_z_order_behind_content` — publish solid color to `ambient-background` AND create a content-layer tile with a different color, render frame, assert tile pixels occlude background and background pixels visible outside tile bounds

## 3. Ambient Background TTL and Persistence

- [ ] 3.1 Add unit test: `test_ambient_background_zero_ttl_persists` — publish with `ttl_us: 0`, verify publication persists in zone state (no auto-clear)
- [ ] 3.2 Add unit test: `test_ambient_background_no_publication_empty` — verify zone with no publications has empty active_publishes list

## 4. User-Test Scenario Integration

- [ ] 4.1 Add ambient-background test scenario to user-test publish script or MCP batch file: publish dark blue solid color, then replace with warm amber
- [ ] 4.2 Add ambient-background rapid replacement scenario: publish 10 different colors in sequence, verify final color via `list_zones` or occupancy check
- [ ] 4.3 Add ambient-background static image scenario: publish a `static_image` content type (using a test resource_id), verify acceptance

## 5. Documentation and Exemplar Completeness

- [ ] 5.1 Verify `ambient-background` zone in `ZoneRegistry::with_defaults()` matches spec expectations (ContentionPolicy::Replace, LayerAttachment::Background, full-screen geometry, accepted media types)
- [ ] 5.2 Verify `ambient-background` component type in component-shape-language spec aligns with exemplar (ReadabilityTechnique::None, zero required tokens)
