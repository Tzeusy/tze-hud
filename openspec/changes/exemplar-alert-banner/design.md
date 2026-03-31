## Context

The alert-banner zone type is one of six v1 component types defined in the component-shape-language spec. It is the only zone type whose backdrop color is dynamically determined by publication content -- specifically, the `urgency` field on `NotificationPayload` maps to severity design tokens that resolve to specific colors.

### Current Implementation State

The component-shape-language epic is fully implemented. The design token system (`DesignTokenMap`, token resolution, canonical fallbacks), extended `RenderingPolicy` (font_family, font_weight, text_color, backdrop_opacity, outline_color, outline_width, etc.), component profiles (profile.toml + zones/ overrides + widgets/ bundles), urgency-to-severity token mapping, and zone readability enforcement (OpaqueBackdrop validation) are all operational.

The compositor's `render_zone_content()` has urgency-aware background coloring for notifications that maps through severity tokens. The `NotificationPayload` struct carries `urgency: u32` (0-3). The alert-banner zone type exists in the zone registry, accepts `ZoneContent::Notification`, and is positioned in the chrome layer.

This exemplar ships as a **component profile** with a `zones/alert-banner.toml` rendering override that wires the specific severity token values, heading typography, and backdrop opacity to produce a polished reference implementation.

### Constraints

- The wgpu rect pipeline supports flat RGBA colors -- no gradients or rounded corners in v1.
- Zone stacking within a zone type must use the existing contention policy infrastructure.
- Auto-dismiss requires timer integration with the zone publication TTL mechanism.
- The exemplar profile must pass OpaqueBackdrop readability validation at startup (backdrop_opacity >= 0.8).

## Goals / Non-Goals

**Goals:**
- Prove the urgency-to-severity token mapping end-to-end: `NotificationPayload.urgency` -> severity token key -> resolved RGBA color -> backdrop quad.
- Ship as a component profile (`exemplar-alert-banner/`) with `zones/alert-banner.toml` that demonstrates profile-based zone rendering customization.
- Demonstrate chrome-layer full-width rendering with correct typography (heading tokens: 24px, weight 700, white text).
- Validate stack-by-severity ordering: critical banners above warning above info.
- Define configurable auto-dismiss durations per severity level (critical: 30s, warning: 15s, info: 8s).
- Provide integration tests and a user-test scenario exercising all urgency levels.

**Non-Goals:**
- Animated transition effects (fade-in/fade-out uses the broader transition_in_ms/transition_out_ms fields; not demonstrated here).
- Custom severity levels beyond the four defined urgency values (0-3).
- Alert acknowledgement / user interaction (post-v1).
- Sound/haptic adjuncts (noted in zone definition but not part of this visual exemplar).
- Runtime theme switching of severity colors.
- Widget SVG bundles (alert-banner uses text-only rendering, not parameterized SVG visuals).

## Decisions

### 1. Exemplar ships as a component profile directory

**Decision:** The exemplar is a component profile directory (`exemplar-alert-banner/`) containing `profile.toml` declaring `component_type = "alert-banner"` and a `zones/alert-banner.toml` rendering override. The profile overrides severity token values to the spec canonical defaults (info=#4A9EFF, warning=#FFB800, error=#FF4444, critical=#FF0000) and sets heading typography via token_overrides.

**Rationale:** Component profiles are the v1 mechanism for packaging visual identity. Shipping the exemplar as a profile proves the profile system works for alert-banner and provides a concrete reference for profile authors. The `zones/` override demonstrates how profile-scoped rendering customization interacts with the token-derived defaults.

**Alternative considered:** Implementing directly in compositor code without a profile. Rejected because the point of the exemplar is to prove the profile-based authoring workflow.

### 2. Stack-by-severity uses z-ordered sub-regions within the alert-banner zone geometry

**Decision:** When multiple alert banners are simultaneously active, the compositor stacks them vertically within the alert-banner zone's geometry allocation. Critical banners render at the top (highest visual priority), warnings below, info at the bottom. Each banner is a separate backdrop quad + text item. The total zone height grows to accommodate stacked banners.

**Rationale:** This follows the spec's "contention: stack by severity" definition. Vertical stacking within the zone geometry is the simplest approach that respects chrome-layer positioning (full-width bar that pushes content down -- more banners = more push-down).

**Alternative considered:** Replacing lower-severity banners when a higher-severity one arrives. Rejected because the spec explicitly says "stack by severity," not "replace by severity."

### 3. Auto-dismiss uses zone publication TTL

**Decision:** Auto-dismiss is implemented by setting the `expires_at` field on `ZonePublishRecord` at publication time. The per-severity durations (critical: 30s, warning: 15s, info: 8s) are applied by the publishing path (MCP tool or gRPC endpoint) based on the notification's urgency. The compositor's existing TTL expiration logic handles removal.

**Rationale:** The zone publication system already supports `expires_at` for TTL-based expiration. Using this existing mechanism avoids adding a parallel timer system. The durations are per-severity defaults that can be overridden by the publisher if needed.

### 4. Backdrop opacity fixed at 90%

**Decision:** Alert-banner backdrops render at 90% opacity (0.9 alpha) regardless of severity level. This matches the spec's OpaqueBackdrop readability requirement (>= 0.8) and the `opacity.backdrop.opaque` token default.

**Rationale:** 90% opacity provides strong contrast for heading text while maintaining slight transparency to indicate overlay status. Consistent opacity across severity levels keeps the visual language clean.

### 5. Typography uses heading tokens

**Decision:** Alert-banner text uses `typography.heading.family` (system sans-serif), `typography.heading.size` (24px), and `typography.heading.weight` (700/bold). Text color is always `color.text.primary` (white, #FFFFFF) for maximum contrast against all severity backdrop colors.

**Rationale:** The spec's V1 Component Type Definitions for alert-banner list these exact required tokens. White text on colored backdrops (blue, amber, red) meets WCAG contrast requirements at all severity levels.

### 6. Zone rendering override file uses registry name form

**Decision:** The zones override file is named `alert-banner.toml` (hyphen form, matching the zone registry name `"alert-banner"`), not `alert_banner.toml` (config constant form with underscore). Per the Zone Name Reconciliation table in the component-shape-language spec, zone override files MUST use the registry name.

**Rationale:** The spec explicitly defines that zone override files in `zones/` subdirectory are matched by registry name. The zone registry name is `"alert-banner"` (hyphen); the config constant `"alert_banner"` (underscore) is a separate identifier. Using the wrong form would trigger `PROFILE_ZONE_OVERRIDE_MISMATCH`.

## Risks / Trade-offs

- **[Stacking implementation]** -> The stack-by-severity contention policy may not yet have compositor-level rendering support for multiple simultaneous banners within a single zone. Mitigation: the tasks include explicit stacking implementation and testing as discrete steps.
- **[Auto-dismiss timer precision]** -> Zone publication TTL uses `Instant` comparison which may have platform-dependent precision. Mitigation: durations are in the seconds range (8-30s) where millisecond precision is irrelevant.
- **[Profile path configuration]** -> The exemplar profile directory must be included in the `[component_profile_bundles].paths` config for the runtime to discover it. Mitigation: the tasks include a configuration step and document the required config entry.
