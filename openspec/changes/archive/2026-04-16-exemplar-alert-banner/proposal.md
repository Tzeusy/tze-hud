## Why

The component-shape-language spec defines the urgency-to-severity token mapping, chrome-layer rendering, and stacking semantics for alert-banner zones, but no concrete exemplar exists to prove these contracts work end-to-end. Without a polished reference implementation exercising the full alert-banner pipeline -- from `NotificationPayload` publication through urgency mapping to severity-colored backdrop rendering -- there is no way to validate that the token system, stacking order, and auto-dismiss behavior integrate correctly. An exemplar closes this gap and provides a visual regression anchor for future changes.

## What Changes

- Define a **polished alert-banner zone exemplar** that exercises the full alert-banner component type contract: severity-colored backdrop via urgency-to-severity token mapping, heading typography, chrome-layer positioning, stack-by-severity ordering, and configurable auto-dismiss timers.
- Add **integration tests** that publish alerts at each urgency level and verify backdrop colors match the resolved severity tokens (`color.severity.info=#4A9EFF`, `color.severity.warning=#FFB800`, `color.severity.critical=#FF0000`).
- Add **stacking-order tests** verifying critical alerts render above warnings above info banners.
- Define a **user-test scenario** that publishes info, warning, and critical alerts in sequence for visual verification of the complete alert-banner pipeline.

## Capabilities

### New Capabilities
- `exemplar-alert-banner`: Exemplar definition for the alert-banner zone type, proving the urgency-to-severity token mapping, chrome-layer rendering, stack-by-severity ordering, auto-dismiss timers, and end-to-end integration testing.

### Modified Capabilities
<!-- No existing spec requirements are changing. The exemplar exercises existing contracts from component-shape-language. -->

## Impact

- **Compositor (`tze_hud_compositor`)**: `render_zone_content()` urgency-to-color mapping must be refactored from hardcoded RGBA values to token-resolved severity colors (`color.severity.info`, `color.severity.warning`, `color.severity.critical`). This replaces the current inline match arms with token lookups.
- **Configuration (`tze_hud_config`)**: Default design token values for `color.severity.info=#4A9EFF`, `color.severity.warning=#FFB800`, `color.severity.critical=#FF0000` must be present in the canonical fallback table.
- **Scene types (`tze_hud_scene`)**: No structural changes -- `NotificationPayload.urgency: u32` and `ZoneContent::Notification` already exist.
- **Test infrastructure**: New integration tests in `tze_hud_compositor` for alert-banner backdrop color verification and stacking-order validation.
- **User-test scripts**: New alert-banner user-test scenario publishing alerts at each urgency level via MCP `publish_to_zone`.
