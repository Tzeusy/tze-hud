> **Implementation prerequisites:** This exemplar requires the following compositor/runtime changes that are not yet landed:
> - `render_zone_content()` in `renderer.rs` breaks after the first publication match — Stack contention zones (like notification-area) only render the latest publication. Multi-publication iteration is required for vertical stacking to work.
> - Per-publication urgency backdrop colors are resolved from compile-time constants in `urgency_to_severity_color()`, not from the profile's scoped token map. Token override of `color.notification.urgency.*` has no effect at render time until the compositor reads from the resolved token map.

## ADDED Requirements

### Requirement: Notification Stack Vertical Layout
The compositor MUST render all active publications in a Stack-contention notification-area zone as individually-positioned notification items, stacked vertically within the zone geometry. Each notification MUST occupy a slot of height `font_size_px + 2 * spacing.padding.medium + 2 * stroke.border.width` pixels. Notifications MUST be ordered newest-at-top: the most recent publication renders at the top of the zone geometry (lowest y_offset), and older publications render below it. If the total slot height of all active publications exceeds the zone geometry height, the lowest (oldest visible) notifications MUST be clipped at the zone boundary.
Scope: v1-mandatory

#### Scenario: Single notification renders at top of zone
- **WHEN** a single `NotificationPayload` is published to the notification-area zone
- **THEN** the compositor MUST render the notification's backdrop quad starting at the zone's top-left corner (zone_x, zone_y), spanning the full zone width and one slot height

#### Scenario: Three notifications stack vertically newest-on-top
- **WHEN** three `NotificationPayload` publications exist in the notification-area zone (published in order A, B, C)
- **THEN** notification C (newest) MUST render at slot index 0 (top), B at slot index 1, and A at slot index 2 (bottom)
- **AND** each notification's y_offset MUST equal `zone_y + slot_index * slot_height`

#### Scenario: Notifications exceeding zone height are clipped
- **WHEN** 5 notifications are active and `5 * slot_height > zone_geometry.height`
- **THEN** the compositor MUST clip notifications that extend beyond `zone_y + zone_height` at the zone boundary

---

### Requirement: Urgency-Tinted Notification Backdrops
Each notification's backdrop quad MUST be tinted according to the publication's `NotificationPayload.urgency` field. The urgency-to-color mapping MUST use profile token overrides when a notification component profile is active, falling back to the following hardcoded defaults when no profile provides urgency tokens:

| Urgency value | Urgency level | Default backdrop color | Opacity |
|---|---|---|---|
| 0 | low | dark gray (#2A2A2A) | 0.9 |
| 1 | normal | dark blue (#1A1A3A) | 0.9 |
| 2 | urgent | amber (#8B6914) | 0.9 |
| 3 | critical | red (#8B1A1A) | 0.9 |

The backdrop MUST be rendered as an opaque quad at 90% opacity (0.9 alpha). When a component profile overrides `color.notification.urgency.low`, `color.notification.urgency.normal`, `color.notification.urgency.urgent`, or `color.notification.urgency.critical`, the profile values MUST take precedence over the defaults. Urgency values outside 0-3 MUST be clamped to 3 (critical).

**Implementation note — dynamic urgency color resolution:** At render time, the compositor resolves urgency backdrop colors by looking up `color.notification.urgency.{level}` (where level is `low`, `normal`, `urgent`, or `critical`) from the profile's scoped token map. The scoped token map is resolved at startup (profile overrides -> global -> canonical fallbacks) and stored alongside the profile. The `RenderingPolicy.backdrop` field is NOT used for per-notification urgency coloring; it provides the fallback color when no urgency-specific token is found (e.g., for non-notification content rendered in the same zone type).

This is distinct from the alert-banner urgency-to-severity token mapping. The notification-area zone MUST NOT use `color.severity.*` tokens for backdrop coloring.
Scope: v1-mandatory

#### Scenario: Low urgency notification renders dark gray backdrop
- **WHEN** a `NotificationPayload` with `urgency = 0` is published to the notification-area zone
- **THEN** the backdrop quad MUST be rendered with color #2A2A2A at 0.9 opacity (or the profile-overridden `color.notification.urgency.low` value)

#### Scenario: Normal urgency notification renders dark blue backdrop
- **WHEN** a `NotificationPayload` with `urgency = 1` is published to the notification-area zone
- **THEN** the backdrop quad MUST be rendered with color #1A1A3A at 0.9 opacity (or the profile-overridden `color.notification.urgency.normal` value)

#### Scenario: Urgent notification renders amber backdrop
- **WHEN** a `NotificationPayload` with `urgency = 2` is published to the notification-area zone
- **THEN** the backdrop quad MUST be rendered with color #8B6914 at 0.9 opacity (or the profile-overridden `color.notification.urgency.urgent` value)

#### Scenario: Critical notification renders red backdrop
- **WHEN** a `NotificationPayload` with `urgency = 3` is published to the notification-area zone
- **THEN** the backdrop quad MUST be rendered with color #8B1A1A at 0.9 opacity (or the profile-overridden `color.notification.urgency.critical` value)

#### Scenario: Out-of-range urgency clamped to critical
- **WHEN** a `NotificationPayload` with `urgency = 5` is published to the notification-area zone
- **THEN** the urgency MUST be clamped to 3 and the critical backdrop color MUST be used

#### Scenario: Notification-area does not use severity tokens
- **WHEN** a `NotificationPayload` with `urgency = 3` is published to the notification-area zone
- **THEN** the compositor MUST NOT resolve `color.severity.critical` for the backdrop color; it MUST use the notification-specific urgency token or default

---

### Requirement: Notification Border Rendering
Each notification MUST be rendered with a 1px border using the `color.border.default` design token (resolved from the zone's effective rendering policy or canonical token fallback). The border MUST be rendered as four thin quads (top, right, bottom, left edges) along the perimeter of the backdrop quad. The border color MUST be consistent across all urgency levels — it is not urgency-tinted.
Scope: v1-mandatory

#### Scenario: Notification has visible 1px border
- **WHEN** a notification is rendered in the notification-area zone
- **THEN** four border quads MUST be rendered: top edge (full width, 1px height), right edge (1px width, full height), bottom edge (full width, 1px height), left edge (1px width, full height)
- **AND** all four quads MUST use the color resolved from `color.border.default`

#### Scenario: Border renders inside backdrop bounds
- **WHEN** a notification backdrop quad spans (x, y, w, h)
- **THEN** the border quads MUST be positioned at the inner edges of the backdrop quad, not outside it

---

### Requirement: Notification Text Rendering
Notification text MUST be rendered using the `typography.body.family` font family, `typography.body.size` font size (16px canonical default), and `typography.body.weight` font weight, resolved from the zone's effective rendering policy (token-derived defaults, then profile overrides). Text MUST be left-aligned within the notification's content area (backdrop minus padding minus border). Text color MUST use `color.text.primary`. Padding MUST use `spacing.padding.medium` (8px canonical default) on all four sides between the border and the text content area.
Scope: v1-mandatory

#### Scenario: Notification text uses body typography tokens
- **WHEN** a notification with `text = "Doorbell rang"` is rendered
- **THEN** the text MUST be rendered at the font size resolved from `typography.body.size` (default 16px), using the font family from `typography.body.family`, with left alignment

#### Scenario: Text is inset by padding from border
- **WHEN** a notification backdrop spans (x, y, w, h) with 1px border and 8px padding
- **THEN** the text content area MUST start at (x + 1 + 8, y + 1 + 8) with width (w - 2 - 16) and height (h - 2 - 16)

#### Scenario: Long text clips within content area
- **WHEN** notification text exceeds the content area width
- **THEN** the text MUST be clipped at the content area boundary (no wrapping in v1)

---

### Requirement: Notification Stack Max Depth Eviction
The notification-area zone MUST enforce `max_depth = 5` for Stack contention. When a new notification is published and the number of active publications equals max_depth, the oldest publication (earliest arrival time) MUST be evicted before the new publication is added. The evicted notification MUST be removed immediately (no fade-out for eviction — fade-out applies only to TTL-based dismissal).
Scope: v1-mandatory

#### Scenario: Sixth notification evicts oldest
- **WHEN** 5 notifications (A, B, C, D, E) are active in the notification-area zone and notification F is published
- **THEN** notification A (oldest) MUST be evicted and notification F MUST be added
- **AND** the active publications MUST be [B, C, D, E, F] in arrival order

#### Scenario: Evicted notification has no fade-out
- **WHEN** a notification is evicted due to max_depth overflow
- **THEN** the notification MUST be removed from active_publishes immediately without triggering a fade-out transition

---

### Requirement: Notification TTL Auto-Dismiss with Fade-Out
Each notification MUST auto-dismiss after a per-publish TTL. The default TTL MUST be 8000ms (8 seconds). When a notification's TTL expires, the compositor MUST initiate a 150ms fade-out transition by interpolating the notification's opacity from 1.0 to 0.0 over 150ms. When the fade-out completes (opacity reaches 0.0), the publication MUST be removed from `active_publishes`. The per-publish TTL MAY be overridden via the `ttl_ms` field in the publish call (if provided); otherwise the zone's `auto_clear_ms` value (8000) is used.

The fade-out MUST be per-notification (each notification fades independently). Multiple notifications MAY fade simultaneously if their TTLs expire within 150ms of each other.

**Implementation note — per-publication animation state:** Per-notification fade-out requires a `PublicationAnimationState` per active publication, extending the zone-level `ZoneAnimationState` pattern from hud-sc0a (Extended RenderingPolicy). Fields: `opacity: f32` (current opacity), `target_opacity: f32` (0.0 when fading out), `transition_start: Instant` (when fade began), `duration_ms: u32` (150 for notification fade-out). This is distinct from the zone-level `ZoneAnimationState` which tracks zone-wide transitions; `PublicationAnimationState` tracks individual publication fade lifecycle within the zone.
Scope: v1-mandatory

#### Scenario: Notification auto-dismisses after 8 seconds
- **WHEN** a notification is published with no explicit TTL and the zone's `auto_clear_ms` is 8000
- **THEN** the notification MUST begin its fade-out transition 8000ms after publication time

#### Scenario: Fade-out interpolates opacity over 150ms
- **WHEN** a notification's TTL expires and fade-out begins
- **THEN** the notification's effective opacity MUST interpolate linearly from 1.0 to 0.0 over 150ms
- **AND** at 75ms into the fade-out, the effective opacity MUST be approximately 0.5

#### Scenario: Notification removed after fade-out completes
- **WHEN** a notification's fade-out transition completes (opacity = 0.0)
- **THEN** the publication MUST be removed from `active_publishes`
- **AND** remaining notifications MUST reflow to fill the vacated slot

#### Scenario: Custom TTL respected
- **WHEN** a notification is published with `ttl_ms = 3000`
- **THEN** the notification MUST begin fade-out at 3000ms, not the default 8000ms

#### Scenario: Multiple notifications fade simultaneously
- **WHEN** two notifications have TTLs that expire within 100ms of each other
- **THEN** both MUST fade independently and simultaneously, each with its own 150ms transition

**Implementation note — `transition_out_ms` vs per-notification fade:** `RenderingPolicy.transition_out_ms` (from hud-sc0a) controls zone-level fade — the entire zone fading when its content is cleared or replaced. Per-notification fade-out uses `PublicationAnimationState` (see above) and operates independently within the zone. The zone override TOML (`zones/notification-area.toml`) SHOULD NOT set `transition_out_ms` for per-notification auto-dismiss behavior; setting it would add an unintended zone-wide fade on top of the per-publication fades.

---

### Requirement: Multi-Agent Notification Publishing
The notification-area zone MUST accept publications from multiple agents simultaneously. Three or more agents MUST be able to publish notifications concurrently, and all publications MUST be correctly stacked according to arrival order. Each notification MUST render independently with its own urgency-tinted backdrop, regardless of which agent published it. The zone's `max_publishers` MUST be at least 3.
Scope: v1-mandatory

#### Scenario: Three agents publish simultaneously
- **WHEN** agent-alpha publishes a notification with urgency=0 (low), agent-beta publishes with urgency=2 (urgent), and agent-gamma publishes with urgency=3 (critical), in that order within the same frame interval
- **THEN** all three notifications MUST appear in the stack: agent-gamma's at top (newest), agent-beta's in middle, agent-alpha's at bottom (oldest)
- **AND** each notification MUST render with its own urgency-specific backdrop color

#### Scenario: Agents do not interfere with each other's notifications
- **WHEN** agent-alpha's notification TTL expires while agent-beta has an active notification
- **THEN** agent-alpha's notification MUST fade out independently without affecting agent-beta's notification

---

### Requirement: Notification Stack Exemplar Component Profile
An exemplar component profile named `notification-stack-exemplar` MUST be provided at `profiles/notification-stack-exemplar/`. The profile MUST declare `component_type = "notification"` and MUST include a `zones/notification-area.toml` with rendering policy overrides. The profile's `token_overrides` MUST define the four urgency backdrop color tokens:

```
color.notification.urgency.low = "#2A2A2A"
color.notification.urgency.normal = "#1A1A3A"
color.notification.urgency.urgent = "#8B6914"
color.notification.urgency.critical = "#8B1A1A"
```

The profile MUST pass OpaqueBackdrop readability validation (backdrop_opacity >= 0.8 for all urgency colors).
Scope: v1-mandatory

#### Scenario: Exemplar profile loads successfully
- **WHEN** the runtime starts with `[component_profiles] notification = "notification-stack-exemplar"` in configuration
- **THEN** the profile MUST load without errors and the notification-area zone MUST use the profile's token overrides for urgency backdrop colors

#### Scenario: Profile passes OpaqueBackdrop readability
- **WHEN** the `notification-stack-exemplar` profile is validated at startup
- **THEN** the OpaqueBackdrop readability check MUST pass (backdrop_opacity = 0.9 >= 0.8)

---

### Requirement: Notification Exemplar MCP Integration Test
An integration test MUST exercise the notification stack exemplar via MCP `publish_to_zone` calls. The test MUST simulate 3 agent identities publishing notifications with varying urgency levels (0-3) to the `notification-area` zone. The test MUST verify:
1. Stack ordering: newest notification is at the top of the stack
2. TTL auto-dismiss: notifications are removed from `active_publishes` after TTL + fade-out duration
3. Urgency styling: each notification's urgency maps to the correct backdrop color
4. Max depth eviction: publishing beyond max_depth=5 evicts the oldest notification
5. Multi-agent: publications from different agents coexist correctly in the stack
Scope: v1-mandatory

#### Scenario: MCP publish from 3 agents with mixed urgency
- **WHEN** the test publishes 3 notifications via MCP `publish_to_zone`:
  - agent "alpha" with `{"type":"notification","text":"System idle","icon":"","urgency":0}`
  - agent "beta" with `{"type":"notification","text":"Update available","icon":"update","urgency":1}`
  - agent "gamma" with `{"type":"notification","text":"Security alert","icon":"shield","urgency":3}`
- **THEN** the zone's `active_publishes` MUST contain all 3 records
- **AND** the newest (gamma) MUST be at the end of the publish list (rendered at top)

#### Scenario: Max depth eviction via MCP burst
- **WHEN** the test publishes 6 notifications sequentially via MCP `publish_to_zone`
- **THEN** `active_publishes` MUST contain exactly 5 records (the 2nd through 6th)
- **AND** the 1st notification MUST have been evicted

#### Scenario: TTL expiry removes notification from active publishes
- **WHEN** a notification is published with `ttl_ms = 100` (short TTL for testing) and 300ms elapses (100ms TTL + 150ms fade-out + 50ms margin)
- **THEN** the notification MUST no longer be present in `active_publishes`

---

### Requirement: Notification Exemplar User-Test Scenario
A user-test scenario script MUST exercise the notification stack on a live deployed HUD. The script MUST publish a burst of notifications from multiple simulated agents with mixed urgency levels and varying TTLs. The script MUST:
1. Publish 3 notifications in rapid succession (urgency 0, 1, 2) and pause 2 seconds for visual inspection
2. Publish 2 more notifications (urgency 3, urgency 1) to show stack growth
3. Wait for the first batch to auto-dismiss, verifying the stack shrinks visually
4. Publish 6 notifications rapidly to trigger max_depth eviction, then pause for inspection
Scope: v1-mandatory

#### Scenario: Burst publish with visual inspection pauses
- **WHEN** the user-test script runs against a deployed HUD with notification-area zone active
- **THEN** the script MUST complete all 4 phases without errors
- **AND** the operator MUST be able to visually confirm urgency-tinted backdrops, vertical stacking, and auto-dismiss behavior

#### Scenario: Script publishes via MCP publish_to_zone
- **WHEN** the user-test script sends a notification
- **THEN** it MUST use the MCP `publish_to_zone` endpoint with content type `notification` and the correct `NotificationPayload` JSON structure
