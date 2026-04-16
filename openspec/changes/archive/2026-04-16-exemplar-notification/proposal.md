## Why

The notification-area zone is one of six v1 built-in zone types, but it has no end-to-end exemplar demonstrating its visual and behavioral contract. The current renderer renders only the most-recent notification (breaks on line 716: `break` after first item), ignoring Stack contention semantics where multiple notifications should accumulate and render independently. The urgency-tinted backdrop colors are hardcoded in the compositor rather than driven by token-resolved rendering policy. Before v1 release, a polished notification stack exemplar proves the notification-area zone renders beautifully with urgency-driven styling, multi-agent stacking, TTL auto-dismiss, and max_depth eviction — and exposes the rendering gaps that must be fixed.

## What Changes

- **Notification stack exemplar**: A self-contained test/demo scenario that exercises the notification-area zone with 3 simulated agents publishing notifications at varying urgency levels (0-3), verifying correct vertical stacking, TTL auto-dismiss (8s default), fade-out transition (150ms), and max_depth=5 eviction
- **Urgency-tinted backdrop rendering**: Each notification's backdrop color must be urgency-driven — low=dark gray, normal=dark blue, urgent=amber, critical=red — rendered as opaque quads at 90% opacity with 1px border (per component-shape-language spec)
- **Stack contention verification**: Notifications from multiple agents accumulate correctly (newest on top), oldest auto-evicted when stack exceeds max_depth=5
- **MCP integration test**: `publish_to_zone` calls from 3 simulated agent identities with varying urgency levels, verifying stack ordering, TTL auto-dismiss, urgency styling, and max depth eviction
- **User-test scenario**: Agent publishes a burst of notifications with mixed urgency, verifying they stack, render with correct urgency styling, and auto-dismiss in order

## Capabilities

### New Capabilities
- `exemplar-notification`: Defines the notification stack exemplar — visual requirements (urgency-tinted backdrops, border, body font, padding), behavioral requirements (stack contention, TTL auto-dismiss, fade-out, max_depth eviction), multi-agent test scenarios, and user-test integration

### Modified Capabilities

_(none — this exemplar consumes existing notification-area zone, MCP publish_to_zone, and component-shape-language specs as-is; any rendering gaps discovered are tracked as implementation issues, not spec changes)_

## Impact

- **New test code**: Integration test in `tests/integration/` exercising multi-agent notification publishing via MCP
- **Compositor changes**: Fix the notification renderer to iterate all Stack publications (not just most-recent), render per-notification urgency-tinted backdrops with border, apply per-publish TTL auto-dismiss and fade-out transition
- **Zone definition adjustment**: Default notification-area `max_depth` changes from 8 to 5, `auto_clear_ms` from 5000 to 8000 to match exemplar TTL
- **Existing specs**: No changes — component-shape-language and scene-graph specs already define the contracts this exemplar exercises
- **User-test script**: New or extended publish script exercising the notification burst scenario
