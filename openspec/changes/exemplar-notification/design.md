## Context

The notification-area zone is a v1 built-in zone type with Stack contention (accumulates publications, auto-dismisses after TTL). The runtime already registers a `"notification-area"` zone at `x_pct=0.75, y_pct=0.02, width_pct=0.24, height_pct=0.30` with `ContentionPolicy::Stack { max_depth: 8 }` and `auto_clear_ms: Some(5_000)`. The `NotificationPayload` struct carries `text: String`, `icon: String`, and `urgency: u32` (0=low, 1=normal, 2=urgent, 3=critical).

The compositor currently has two rendering defects for notifications:
1. **Only renders the most-recent notification** — the `break` statement in `render_zone_content` after the first `ZoneContent::Notification` match means Stack publications beyond the first are invisible.
2. **Urgency colors are hardcoded** — the compositor uses inline `[r, g, b, a]` arrays instead of token-resolved values from the zone's effective `RenderingPolicy`.

The component-shape-language spec defines the notification component type contract: `OpaqueBackdrop` readability (backdrop_opacity >= 0.8), token-driven rendering (body typography, padding, border), and explicit separation from alert-banner urgency-to-severity color mapping. This exemplar exercises that contract.

A component profile (`notification-stack-exemplar`) will ship as the exemplar's visual identity, with `zones/notification-area.toml` overrides for urgency-specific backdrop colors and typography. This leverages the component profile system (component-shape-language) rather than hardcoding values.

## Goals / Non-Goals

**Goals:**
- Prove the notification-area zone renders a stack of up to 5 notifications with per-notification urgency-tinted backdrops, 1px borders, body-size left-aligned text, and 8px padding
- Fix the compositor to iterate all Stack publications (not just most-recent) with per-item vertical layout
- Demonstrate multi-agent publishing: 3 agents post simultaneously, all stack correctly
- Verify TTL auto-dismiss (8s default) with 150ms fade-out transition
- Verify max_depth=5 eviction (oldest removed when full)
- Ship a component profile (`notification-stack-exemplar`) with urgency-to-backdrop color overrides via token_overrides
- Provide an MCP-based integration test and a user-test scenario script

**Non-Goals:**
- Icon rendering (texture pipeline is not yet available per hud-lh3w; icon field is accepted but not rendered)
- Sound/haptic adjunct effects (defined in zone doctrine but not v1 renderer scope)
- Urgency-to-severity token mapping (that is alert-banner-specific per component-shape-language spec; notification-area uses its own backdrop color, not severity tokens)
- Profile switching at runtime (operator configures at startup)
- Drag-to-dismiss or swipe gestures (post-v1 input model)

## Decisions

### Per-notification vertical layout with dynamic slot sizing
**Decision:** Each notification occupies a fixed-height slot within the zone geometry. Slot height = `font_size_px + 2 * padding + 2 * border_width`. Notifications stack top-to-bottom within the zone, with newest at the top (index 0). The zone geometry's `height_pct` bounds the visible area; notifications exceeding the visible height are clipped.
**Rationale:** Fixed-height slots simplify layout and avoid text measurement in the compositor hot path. The zone geometry is pre-allocated; we subdivide it rather than dynamically growing.
**Alternative considered:** Variable-height notifications with text wrapping measurement. Rejected because it requires per-frame text layout which violates the frame budget. Long text is clipped to bounds.

### Urgency backdrop colors as profile token overrides (not hardcoded)
**Decision:** The exemplar ships a component profile `notification-stack-exemplar` with `token_overrides` in `profile.toml`:
```toml
[token_overrides]
"color.notification.urgency.low" = "#2A2A2A"       # dark gray
"color.notification.urgency.normal" = "#1A1A3A"    # dark blue
"color.notification.urgency.urgent" = "#8B6914"    # amber
"color.notification.urgency.critical" = "#8B1A1A"  # red
```
The compositor reads the publication's `urgency` field and looks up the corresponding token at render time. Non-canonical tokens are allowed per component-shape-language spec.
**Rationale:** Follows the component profile system — operators can swap profiles to change urgency colors without code changes. Avoids the anti-pattern of hardcoded color arrays in the renderer.
**Alternative considered:** Using the severity token mapping (color.severity.info/warning/critical). Rejected because the spec explicitly states notification-area does NOT use urgency-to-severity mapping (that is alert-banner-specific).

### Stack ordering: newest on top, oldest at bottom
**Decision:** `active_publishes` for Stack zones are stored oldest-first (current behavior). The renderer iterates in reverse order so newest renders at `y_offset=0` (top of zone). When max_depth is exceeded, the oldest (index 0) is evicted.
**Rationale:** Newest-on-top matches user expectation for notification stacks (Android, macOS, Windows). The existing Stack eviction logic already removes index 0 (oldest) when exceeding max_depth.

### Fade-out transition via ZoneAnimationState
**Decision:** Each notification publish record gets a per-record animation state tracking opacity. When TTL expires, the opacity target is set to 0.0 with a 150ms duration. When opacity reaches 0.0, the record is removed from `active_publishes`. This extends the `ZoneAnimationState` pattern (defined in component-shape-language spec) to per-publication granularity.
**Rationale:** The component-shape-language spec already defines `ZoneAnimationState` with `opacity`, `target_opacity`, `transition_start`, `duration_ms` fields. Extending it per-publication is a natural fit for Stack zones where each item has independent lifecycle.
**Alternative considered:** Immediate removal on TTL expiry. Rejected because it creates jarring visual pops, especially with multiple notifications dismissing in sequence.

### Default notification-area zone definition adjustments
**Decision:** Change `max_depth` from 8 to 5 and `auto_clear_ms` from 5000 to 8000 in `ZoneRegistry::with_defaults()`.
**Rationale:** 5 is the exemplar's showcase depth — sufficient to demonstrate stack behavior without overwhelming the zone geometry. 8s TTL gives the observer time to read notifications before auto-dismiss.

### Border rendering as four thin quads
**Decision:** Each notification's 1px border is rendered as four thin quads (top, right, bottom, left) along the edges of the backdrop quad. Border color comes from `color.border.default` token.
**Rationale:** Matches the component-shape-language spec's rendering notes for the notification component type. Four-quad borders are cheap (4 vertices each) and GPU-friendly.

## Risks / Trade-offs

- **[Frame budget with 5 stacked notifications]** Each notification adds 1 backdrop quad + 4 border quads + 1 text item = 6 draw elements. At max_depth=5, that is 30 elements per frame. This is well within the frame budget (compositor handles hundreds of tile quads already). --> No mitigation needed.
- **[TTL timer precision]** `auto_clear_ms` is checked per-frame (~16ms at 60fps). A notification with TTL=8000ms may persist 8000-8016ms. --> Acceptable for notification use case; not a real-time media clock.
- **[Multi-agent race on Stack insertion]** Two agents publishing simultaneously contend for `Arc<Mutex<SceneGraph>>`. The lock is held briefly during `publish_to_zone`. --> Already handled by existing mutex semantics; Stack ordering is insertion-order (arrival order per spec).
- **[Font metrics unavailable for slot height]** The slot height formula uses `font_size_px` directly without glyph metrics. Glyphs that exceed the font size (descenders, diacritics) may clip slightly. --> Acceptable for v1; text measurement is a post-v1 concern.
- **[Profile not configured by default]** If the operator does not configure `[component_profiles] notification = "notification-stack-exemplar"`, the runtime falls back to token-derived defaults (which still produce valid OpaqueBackdrop rendering, just without urgency-specific colors). --> This is by design; the profile is an optional enhancement.
