## 1. Component Profile Setup

- [ ] 1.1 Create exemplar profile directory `profiles/exemplar-alert-banner/` with `profile.toml` manifest: `name = "exemplar-alert-banner"`, `version = "1.0.0"`, `component_type = "alert-banner"`, `[token_overrides]` with `color.severity.info = "#4A9EFF"`, `color.severity.warning = "#FFB800"`, `color.severity.critical = "#FF0000"`, `typography.heading.size = "24"`, `typography.heading.weight = "700"`
- [ ] 1.2 Create `profiles/exemplar-alert-banner/zones/alert_banner.toml` rendering override with `backdrop_opacity = 0.9`, `margin_horizontal` for horizontal padding, and heading typography fields
- [ ] 1.3 Add `profiles/exemplar-alert-banner/` to `[component_profile_bundles].paths` in the default/example config
- [ ] 1.4 Verify profile loads at startup without errors and passes OpaqueBackdrop readability validation

## 2. Severity-Colored Backdrop Rendering

- [ ] 2.1 Refactor `render_zone_content()` in `tze_hud_compositor` to resolve alert-banner backdrop color from severity tokens (`color.severity.info`, `color.severity.warning`, `color.severity.critical`) via the design token system instead of hardcoded RGBA values
- [ ] 2.2 Implement urgency-to-severity token key mapping function: urgency 0,1 -> `color.severity.info`, urgency 2 -> `color.severity.warning`, urgency 3 -> `color.severity.critical`
- [ ] 2.3 Apply 0.9 alpha (90% opacity) to the resolved severity color for the backdrop quad
- [ ] 2.4 Ensure non-notification content (`ZoneContent::StreamText`, etc.) published to alert-banner falls back to the zone's default `backdrop` from effective RenderingPolicy

## 3. Heading Typography

- [ ] 3.1 Wire alert-banner text rendering to use `typography.heading.family` (SansSerif), `typography.heading.size` (24px), `typography.heading.weight` (700) from the effective RenderingPolicy
- [ ] 3.2 Set text color to `color.text.primary` (#FFFFFF white) for all alert-banner text
- [ ] 3.3 Apply horizontal padding (`margin_horizontal`) from the effective RenderingPolicy to inset text from the left edge of the backdrop quad

## 4. Stack-by-Severity Ordering

- [ ] 4.1 Implement severity-aware stacking in the compositor for alert-banner zone: sort active publications by severity (critical at top, warning middle, info bottom), then by recency within same severity
- [ ] 4.2 Render each stacked banner as a separate backdrop quad + text item, positioned vertically within the zone geometry
- [ ] 4.3 Dynamically adjust alert-banner zone height to accommodate all active stacked banners (single banner height x active count); when no banners are active, zone height MUST be zero

## 5. Auto-Dismiss by Severity

- [ ] 5.1 Implement default auto-dismiss duration mapping in the publishing path: urgency 0,1 -> 8s, urgency 2 -> 15s, urgency 3 -> 30s
- [ ] 5.2 Set `expires_at` on `ZonePublishRecord` at publication time using the severity-mapped duration when no explicit `expires_at` is provided by the publisher
- [ ] 5.3 Verify the compositor's existing TTL expiration logic correctly removes expired alert-banner publications from rendering

## 6. Integration Tests

- [ ] 6.1 Write integration test: publish urgency 0 to alert-banner zone, render headless frame, assert backdrop pixel RGB values match #4A9EFF (R=74, G=158, B=255) within +/-5 tolerance
- [ ] 6.2 Write integration test: publish urgency 2 to alert-banner zone, render headless frame, assert backdrop pixel RGB values match #FFB800 (R=255, G=184, B=0) within +/-5 tolerance
- [ ] 6.3 Write integration test: publish urgency 3 to alert-banner zone, render headless frame, assert backdrop pixel RGB values match #FF0000 (R=255, G=0, B=0) within +/-5 tolerance
- [ ] 6.4 Write integration test: publish info (urgency 1), warning (urgency 2), and critical (urgency 3) simultaneously, render headless frame, assert critical (red) backdrop is at top, warning (amber) in middle, info (blue) at bottom by sampling pixel colors at each vertical position
- [ ] 6.5 Write integration test: publish non-notification content (`StreamText`) to alert-banner zone, verify backdrop uses default RenderingPolicy color (not severity mapping)

## 7. User-Test Scenario

- [ ] 7.1 Create user-test script (Python or JSON scenario) that publishes info alert via MCP `publish_to_zone` with `urgency = 1`, text "Info: system nominal"
- [ ] 7.2 Add 3-second delay, then publish warning alert with `urgency = 2`, text "Warning: disk space low"
- [ ] 7.3 Add 3-second delay, then publish critical alert with `urgency = 3`, text "CRITICAL: security breach detected"
- [ ] 7.4 Document expected visual result: three stacked banners visible simultaneously -- critical (red) at top, warning (amber) in middle, info (blue) at bottom, all with white 24px bold heading text
- [ ] 7.5 Integrate user-test scenario into the existing user-test skill infrastructure (`.claude/skills/user-test/`)
