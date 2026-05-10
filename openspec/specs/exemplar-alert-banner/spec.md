# exemplar-alert-banner Specification

## Purpose
Defines the alert-banner exemplar component profile and visual contract for severity-colored, chrome-layer alert rendering using the component shape language.

## Requirements
### Requirement: Alert-Banner Exemplar Component Profile
The exemplar-alert-banner component profile MUST be a valid component profile directory containing a `profile.toml` manifest and a `zones/alert-banner.toml` rendering override. The profile MUST declare `component_type = "alert-banner"` and MUST pass OpaqueBackdrop readability validation at startup.

The `profile.toml` manifest MUST contain:
- `name = "exemplar-alert-banner"`
- `version = "1.0.0"`
- `component_type = "alert-banner"`
- `[token_overrides]` with severity color tokens matching canonical defaults: `color.severity.info = "#4A9EFF"`, `color.severity.warning = "#FFB800"`, `color.severity.error = "#FF4444"`, `color.severity.critical = "#FF0000"`
- `[token_overrides]` with heading typography tokens: `typography.heading.size = "24"`, `typography.heading.weight = "700"`
Scope: v1-mandatory

#### Scenario: Profile loads successfully at startup
- **WHEN** the exemplar-alert-banner profile directory is included in `[component_profile_bundles].paths` and the runtime starts
- **THEN** the profile MUST load without errors, pass OpaqueBackdrop readability validation, and register as the active profile for the `alert-banner` component type

#### Scenario: Profile declares correct component type
- **WHEN** the exemplar-alert-banner profile's `profile.toml` is parsed
- **THEN** the `component_type` field MUST be `"alert-banner"` and MUST match the v1 component type registry

#### Scenario: Zone override file uses registry name
- **WHEN** the exemplar-alert-banner profile contains `zones/alert-banner.toml`
- **THEN** the override MUST be matched to the `"alert-banner"` zone definition (registry name form with hyphen)

----

### Requirement: Alert-Banner Severity-Colored Backdrop Rendering
When a `NotificationPayload` is published to the alert-banner zone, the compositor MUST render a full-width backdrop quad whose color is determined by the urgency-to-severity token mapping. The backdrop MUST be rendered at 90% opacity (0.9 alpha). The mapping MUST follow the component-shape-language spec:

| Urgency value | Severity token | Default resolved color |
|---|---|---|
| 0 (low) | `color.severity.info` | #4A9EFF |
| 1 (normal) | `color.severity.info` | #4A9EFF |
| 2 (urgent) | `color.severity.warning` | #FFB800 |
| 3 (critical) | `color.severity.critical` | #FF0000 |

The backdrop MUST span the full width of the display with horizontal padding applied from the zone's `margin_horizontal` RenderingPolicy field.
Scope: v1-mandatory

#### Scenario: Urgency 0 renders info-blue backdrop
- **WHEN** a `NotificationPayload` with `urgency = 0` and `text = "System check complete"` is published to the alert-banner zone
- **THEN** the compositor MUST render a full-width backdrop quad with color resolved from `color.severity.info` (default #4A9EFF) at 0.9 alpha

#### Scenario: Urgency 1 renders info-blue backdrop
- **WHEN** a `NotificationPayload` with `urgency = 1` and `text = "Update available"` is published to the alert-banner zone
- **THEN** the compositor MUST render a full-width backdrop quad with color resolved from `color.severity.info` (default #4A9EFF) at 0.9 alpha

#### Scenario: Urgency 2 renders warning-amber backdrop
- **WHEN** a `NotificationPayload` with `urgency = 2` and `text = "Disk space low"` is published to the alert-banner zone
- **THEN** the compositor MUST render a full-width backdrop quad with color resolved from `color.severity.warning` (default #FFB800) at 0.9 alpha

#### Scenario: Urgency 3 renders critical-red backdrop
- **WHEN** a `NotificationPayload` with `urgency = 3` and `text = "Security breach detected"` is published to the alert-banner zone
- **THEN** the compositor MUST render a full-width backdrop quad with color resolved from `color.severity.critical` (default #FF0000) at 0.9 alpha

#### Scenario: Non-notification content uses default backdrop
- **WHEN** a `ZoneContent::StreamText` is published to the alert-banner zone
- **THEN** the compositor MUST use the zone's default `backdrop` color from the effective RenderingPolicy, not the severity color mapping

----

### Requirement: Alert-Banner Heading Typography
Alert-banner text MUST be rendered using the heading typography token set. The text MUST use `typography.heading.family` (default: system sans-serif), `typography.heading.size` (24px), and `typography.heading.weight` (700/bold). The text color MUST be `color.text.primary` (default: #FFFFFF white) for maximum contrast against all severity backdrop colors. Text MUST be left-aligned with horizontal padding matching the zone's `margin_horizontal`.
Scope: v1-mandatory

#### Scenario: Alert text renders with heading typography
- **WHEN** a `NotificationPayload` with `text = "Weather alert: severe storms"` is published to the alert-banner zone
- **THEN** the compositor MUST render the text at 24px font size, weight 700, using the system sans-serif font family, in white (#FFFFFF) color

#### Scenario: Text has horizontal padding
- **WHEN** an alert banner is rendered with `margin_horizontal` set in the effective RenderingPolicy
- **THEN** the text MUST be inset from the left edge of the backdrop quad by the `margin_horizontal` value

----

### Requirement: Alert-Banner Stack-by-Severity Ordering
When multiple alert-banner publications are simultaneously active, the compositor MUST stack them vertically within the alert-banner zone geometry. The stacking order MUST be by severity: critical alerts (urgency 3) at the top, warnings (urgency 2) below, info alerts (urgency 0-1) at the bottom. Within the same severity level, newer publications MUST appear above older ones. Each stacked banner MUST be a separate backdrop quad + text item. The total zone height MUST grow to accommodate all active banners.
Scope: v1-mandatory

#### Scenario: Critical stacks above warning
- **WHEN** a warning alert (urgency 2) is active and a critical alert (urgency 3) is published to the alert-banner zone
- **THEN** the critical alert MUST render above the warning alert, with the critical banner's backdrop quad positioned at a lower y-coordinate (closer to screen top)

#### Scenario: Warning stacks above info
- **WHEN** an info alert (urgency 1) is active and a warning alert (urgency 2) is published to the alert-banner zone
- **THEN** the warning alert MUST render above the info alert

#### Scenario: Three severity levels stack correctly
- **WHEN** info (urgency 1), warning (urgency 2), and critical (urgency 3) alerts are all simultaneously active in the alert-banner zone
- **THEN** from top to bottom the rendering order MUST be: critical (red #FF0000), warning (amber #FFB800), info (blue #4A9EFF)

#### Scenario: Same-severity banners ordered by recency
- **WHEN** two info alerts are simultaneously active with different publication times
- **THEN** the more recently published alert MUST render above the older alert within the info severity group

#### Scenario: Zone height grows with stacked banners
- **WHEN** three alert banners are simultaneously active
- **THEN** the alert-banner zone's effective height MUST be approximately 3x the single-banner height, pushing chrome-layer content below it further down

----

### Requirement: Alert-Banner Auto-Dismiss by Severity
Alert-banner publications MUST auto-dismiss after a severity-dependent duration. The auto-dismiss durations MUST be: critical (urgency 3) = 30 seconds, warning (urgency 2) = 15 seconds, info (urgency 0 or 1) = 8 seconds. Auto-dismiss MUST be implemented by setting the `expires_at` field on the `ZonePublishRecord` at publication time. Publishers MAY override the default auto-dismiss duration by providing an explicit `expires_at` value.
Scope: v1-mandatory

#### Scenario: Info alert auto-dismisses after 8 seconds
- **WHEN** a `NotificationPayload` with `urgency = 1` is published to the alert-banner zone without an explicit `expires_at`
- **THEN** the publication's `expires_at` MUST be set to the current time plus 8 seconds, and the banner MUST be removed from rendering after that time

#### Scenario: Warning alert auto-dismisses after 15 seconds
- **WHEN** a `NotificationPayload` with `urgency = 2` is published to the alert-banner zone without an explicit `expires_at`
- **THEN** the publication's `expires_at` MUST be set to the current time plus 15 seconds

#### Scenario: Critical alert auto-dismisses after 30 seconds
- **WHEN** a `NotificationPayload` with `urgency = 3` is published to the alert-banner zone without an explicit `expires_at`
- **THEN** the publication's `expires_at` MUST be set to the current time plus 30 seconds

#### Scenario: Publisher overrides auto-dismiss duration
- **WHEN** a `NotificationPayload` with `urgency = 2` is published to the alert-banner zone with an explicit `expires_at` of current time plus 60 seconds
- **THEN** the publication MUST use the publisher-provided `expires_at` (60 seconds), not the default warning duration (15 seconds)

----

### Requirement: Alert-Banner Chrome-Layer Positioning
The alert-banner zone MUST render in the chrome layer, above all agent content. The banner MUST span the full display width. The banner height MUST accommodate the heading text (24px font + vertical padding). When no alerts are active, the alert-banner zone MUST occupy zero vertical space (no empty chrome reservation).
Scope: v1-mandatory

#### Scenario: Banner renders above agent content
- **WHEN** an alert banner is active and agent tiles are rendered in the content layer
- **THEN** the alert-banner backdrop quad and text MUST render on top of all agent content (chrome layer z-order)

#### Scenario: Full-width rendering
- **WHEN** an alert banner is rendered on a 1920-pixel-wide display
- **THEN** the backdrop quad MUST span from x=0 to x=1920 (full display width), with text inset by `margin_horizontal`

#### Scenario: Zero height when no alerts active
- **WHEN** no alert-banner publications are active (all expired or none published)
- **THEN** the alert-banner zone MUST occupy zero vertical space and MUST NOT push content down

----

### Requirement: Alert-Banner Integration Test Suite
The exemplar MUST include integration tests that verify the complete alert-banner pipeline: publication, urgency-to-severity mapping, backdrop color rendering, stacking order, and auto-dismiss.
Scope: v1-mandatory

#### Scenario: Test urgency 0 backdrop color
- **WHEN** an integration test publishes a `NotificationPayload` with `urgency = 0` to the alert-banner zone and reads back the rendered frame pixels at the banner region
- **THEN** the backdrop pixel RGB values MUST match `color.severity.info` resolved color (#4A9EFF: R=74, G=158, B=255, within tolerance of +/-5 per channel)

#### Scenario: Test urgency 1 backdrop color
- **WHEN** an integration test publishes a `NotificationPayload` with `urgency = 1` to the alert-banner zone and reads back the rendered frame pixels at the banner region
- **THEN** the backdrop pixel RGB values MUST match `color.severity.info` resolved color (#4A9EFF: R=74, G=158, B=255, within tolerance of +/-5 per channel), proving that urgency 1 maps to the same severity token as urgency 0

#### Scenario: Test urgency 2 backdrop color
- **WHEN** an integration test publishes a `NotificationPayload` with `urgency = 2` to the alert-banner zone and reads back the rendered frame pixels
- **THEN** the backdrop pixel RGB values MUST match `color.severity.warning` resolved color (#FFB800: R=255, G=184, B=0, within tolerance of +/-5 per channel)

#### Scenario: Test urgency 3 backdrop color
- **WHEN** an integration test publishes a `NotificationPayload` with `urgency = 3` to the alert-banner zone and reads back the rendered frame pixels
- **THEN** the backdrop pixel RGB values MUST match `color.severity.critical` resolved color (#FF0000: R=255, G=0, B=0, within tolerance of +/-5 per channel)

#### Scenario: Test stacking order with three severity levels
- **WHEN** an integration test publishes info (urgency 1), warning (urgency 2), and critical (urgency 3) alerts simultaneously and reads back the rendered frame
- **THEN** the critical (red) backdrop MUST be at the top, warning (amber) in the middle, and info (blue) at the bottom

----

### Requirement: Alert-Banner User-Test Scenario
The exemplar MUST include a user-test scenario that publishes alerts at each urgency level in sequence for visual verification. The scenario publishes info, then warning, then critical alerts via MCP `publish_to_zone` and waits between publications to allow visual inspection.
Scope: v1-mandatory

#### Scenario: Sequential urgency-level publication
- **WHEN** the user-test scenario is executed
- **THEN** it MUST publish an info alert (urgency 1, text "Info: system nominal"), wait 3 seconds, publish a warning alert (urgency 2, text "Warning: disk space low"), wait 3 seconds, then publish a critical alert (urgency 3, text "CRITICAL: security breach detected")

#### Scenario: All three banners visible simultaneously
- **WHEN** all three alerts from the sequential publication are within their auto-dismiss windows
- **THEN** the display MUST show three stacked banners: critical (red) at top, warning (amber) in middle, info (blue) at bottom, each with white heading text
