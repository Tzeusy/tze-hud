# Tasks

## 1. Zone Definition Adjustment

- [ ] 1.1 Change `notification-area` default `max_depth` from 8 to 5 in `ZoneRegistry::with_defaults()` (`crates/tze_hud_scene/src/types.rs`)
- [ ] 1.2 Change `notification-area` default `auto_clear_ms` from `Some(5_000)` to `Some(8_000)` in `ZoneRegistry::with_defaults()`
- [ ] 1.3 Update existing tests that assert on `max_depth: 8` or `auto_clear_ms: 5000` for notification-area zones (search for `max_depth: 8` and `5_000` in zone-related tests)

## 2. Compositor Stack Rendering Fix

- [ ] 2.1 Remove the `break` statement after `ZoneContent::Notification` in `render_zone_content` (`crates/tze_hud_compositor/src/renderer.rs` ~line 716) so all Stack publications are iterated
- [ ] 2.2 Implement per-notification vertical slot layout: compute `slot_height = font_size_px + 2 * padding + 2 * border_width`, position each notification at `zone_y + slot_index * slot_height` with newest (last in reverse iteration) at slot 0
- [ ] 2.3 Add clipping: skip rendering notifications whose y_offset + slot_height exceeds zone_y + zone_height

## 3. Urgency-Tinted Backdrop Rendering

- [ ] 3.1 Replace hardcoded urgency color arrays in `render_zone_content` with token-resolved values: look up `color.notification.urgency.{low,normal,urgent,critical}` from the zone's effective rendering policy / token context, falling back to the spec defaults (#2A2A2A, #1A1A3A, #8B6914, #8B1A1A)
- [ ] 3.2 Render each notification's backdrop quad at 0.9 opacity (per-notification, not zone-wide)
- [ ] 3.3 Clamp out-of-range urgency values (>3) to 3 (critical) before color lookup

## 4. Border Rendering

- [ ] 4.1 Implement four-quad border rendering for each notification: top, right, bottom, left edges at 1px width using `color.border.default` token
- [ ] 4.2 Position borders at the inner edges of the backdrop quad (not outside)

## 5. Text Rendering Update

- [ ] 5.1 Update `TextItem::from_zone_notification` to use token-resolved `typography.body.size` (default 16px) instead of the zone-wide `font_size_px` fallback
- [ ] 5.2 Ensure text is left-aligned and inset by `spacing.padding.medium` (8px) + border_width (1px) from the backdrop edges
- [ ] 5.3 Verify text uses `color.text.primary` token for text color

## 6. TTL Auto-Dismiss with Fade-Out

- [ ] 6.1 Add per-publication animation state: extend `ZonePublishRecord` (or a parallel structure) with `opacity: f32`, `target_opacity: f32`, `fade_start: Option<Instant>`, `fade_duration_ms: u32`
- [ ] 6.2 In the compositor frame loop, check each Stack publication's age against its TTL; when expired, set `target_opacity = 0.0` and `fade_start = Some(now)` with `fade_duration_ms = 150`
- [ ] 6.3 Interpolate opacity linearly from 1.0 to 0.0 over the fade duration; apply opacity to the backdrop quad and text alpha
- [ ] 6.4 When fade completes (opacity reaches 0.0), remove the publication from `active_publishes`
- [ ] 6.5 After removal, reflow remaining notifications (recalculate slot positions)

## 7. Component Profile

- [ ] 7.1 Create `profiles/notification-stack-exemplar/profile.toml` with `name = "notification-stack-exemplar"`, `version = "1.0.0"`, `component_type = "notification"`, and `token_overrides` for the four urgency backdrop colors
- [ ] 7.2 Create `profiles/notification-stack-exemplar/zones/notification-area.toml` with rendering policy overrides: `backdrop_opacity = 0.9`, `font_size_px = 16.0`, `text_align = "start"`, `margin_horizontal = 8.0`, `margin_vertical = 8.0`
- [ ] 7.3 Verify profile passes OpaqueBackdrop readability validation at startup (backdrop present, opacity >= 0.8)

## 8. Integration Tests

- [ ] 8.1 Add integration test in `tests/integration/` that creates a SceneGraph with notification-area zone (max_depth=5), publishes 3 notifications from 3 different agent namespaces with urgency 0, 1, 3, and asserts all 3 are present in `active_publishes` in correct order
- [ ] 8.2 Add integration test that publishes 6 notifications and asserts only the 5 newest remain (oldest evicted)
- [ ] 8.3 Add integration test that publishes with `ttl_ms = 100`, advances time by 300ms, and asserts the notification is removed from `active_publishes`
- [ ] 8.4 Add integration test that publishes 3 notifications from different agents, verifies each has the correct urgency-specific backdrop color in the render output

## 9. User-Test Scenario Script

- [ ] 9.1 Create `.claude/skills/user-test/scripts/notification_exemplar.py` — a Python 3 script (stdlib only) that publishes notification bursts via MCP `publish_to_zone` to a deployed HUD
- [ ] 9.2 Implement Phase 1: publish 3 notifications (urgency 0, 1, 2) from agents alpha/beta/gamma, pause 2s
- [ ] 9.3 Implement Phase 2: publish 2 more (urgency 3, urgency 1), pause 2s for stack growth observation
- [ ] 9.4 Implement Phase 3: wait for first batch TTL expiry (~8s), pause 1s for visual confirmation of stack shrinkage
- [ ] 9.5 Implement Phase 4: publish 6 notifications rapidly to trigger max_depth eviction, pause 3s for inspection
- [ ] 9.6 Accept CLI args: `--url` (MCP endpoint), `--psk-env` (PSK env var name), `--ttl` (override default 8s TTL)
