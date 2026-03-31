## Context

The widget system defines a complete pipeline for parameterized visual widgets: SVG asset bundles with parameter bindings, design token placeholder resolution, smooth interpolation, and contention governance. The component-shape-language change adds the design token system, SVG `{{token.key}}` placeholder resolution, and `data-role` readability conventions. Both are fully implemented.

A minimal gauge test fixture exists at `crates/tze_hud_widget/tests/fixtures/gauge/` — it validates bundle loading but uses hardcoded hex colors, lacks token placeholders, and has no readability attributes. This exemplar defines the production-quality evolution: a fully token-driven gauge widget that exercises every widget pipeline capability in a single visual.

## Goals / Non-Goals

**Goals:**

- Define the canonical reference widget that proves the SVG asset bundle pipeline works end-to-end with design tokens
- Exercise all four parameter binding types (linear, direct, discrete, text-content) in one widget
- Specify exact SVG markup with `{{token.key}}` placeholders for all colors, `data-role` attributes for readability convention compliance
- Define interpolation behavior: smooth transitions for f32/color parameters, immediate snap for string/enum
- Provide concrete MCP test fixtures for `publish_to_widget` exercising every parameter type and transition behavior
- Provide a user-test scenario for cross-machine visual validation

**Non-Goals:**

- Replacing the existing test fixture — the minimal fixture remains for unit test purposes; this exemplar is the production target
- Defining new widget system capabilities — this exemplar exercises existing capabilities only
- Circular gauge variant — v1 uses a vertical fill bar; circular variants are deferred
- Custom animations beyond parameter interpolation (no keyframes, no easing curves beyond linear)

## Decisions

### D1: Vertical fill bar geometry over circular gauge

**Choice:** The exemplar gauge uses a vertical rectangular fill bar, not a radial/circular gauge.

**Rationale:** A vertical bar exercises the linear binding type most clearly (parameter 0.0-1.0 maps directly to height 0-200px), requires only standard SVG rectangles (no arc paths or clip-path tricks), and rasterizes within the 2ms budget trivially. Circular gauges require `<path>` arc computation in the binding, which the v1 binding model does not support (linear maps to a single numeric attribute, not a path data string). The vertical bar is the simplest visual that proves the full pipeline.

**Alternatives considered:**
- *Circular arc gauge:* Would require computing SVG path `d` attributes from a parameter value, which the declarative binding model (linear/direct/discrete) does not support. The `d` attribute is a complex path string, not a single numeric value.
- *Horizontal bar:* Equivalent complexity but less natural for a "fill level" metaphor (vertical = fill up).

### D2: Token placeholders for all colors, hardcoded geometry

**Choice:** All color values in SVG layers use `{{token.key}}` placeholders (e.g., `{{token.color.backdrop.default}}`, `{{token.color.text.primary}}`, `{{token.color.severity.info}}`). Geometry values (viewBox, dimensions, positions) remain hardcoded.

**Rationale:** Color tokens demonstrate the design token pipeline with zero ambiguity — every color in the rendered gauge traces back to a token key. Geometry tokens exist in the canonical schema (`spacing.*`, `typography.*.size`) but the gauge's spatial layout is widget-specific, not theme-driven. Mixing token-driven colors with hardcoded layout is the intended v1 pattern: tokens handle visual consistency across widgets, layout is per-widget.

### D3: Severity indicator uses canonical color.severity.* tokens in discrete mapping

**Choice:** The discrete binding for the `severity` enum parameter maps to canonical token values rather than arbitrary hex colors. The `value_map` in `widget.toml` references the same colors that `color.severity.info`, `color.severity.warning`, and `color.severity.error` tokens resolve to at startup. The SVG indicator element itself uses `{{token.color.severity.info}}` as its default fill (resolved at bundle load time).

**Rationale:** This ensures the gauge's severity colors are always consistent with the operator's configured design tokens. If an operator overrides `color.severity.error` to a different red, both the zone-level severity rendering and the gauge widget's indicator use the same color. The discrete `value_map` in `widget.toml` must use concrete hex values (not token references — the binding value_map is resolved at render time, not bundle load time), so the exemplar documents that the value_map should match the canonical token fallback values or be updated when tokens are customized.

### D4: data-role attributes on backdrop and text elements

**Choice:** The background layer's frame rectangle uses `data-role="backdrop"` and the fill layer's label text uses `data-role="text"`. The fill bar and severity indicator do not use data-role (they are functional visual elements, not readability-contract elements).

**Rationale:** The readability convention spec requires `data-role="backdrop"` and `data-role="text"` on elements that participate in readability validation. The gauge's frame provides the backdrop (dark background behind all content), and the label provides the text. The fill bar and indicator circle are data-driven visuals, not text readability elements — they should not trigger readability validation.

### D5: Re-rasterization budget of 2ms for 512x512

**Choice:** The exemplar specifies that re-rasterization after any single parameter change completes in < 2ms at 512x512 resolution.

**Rationale:** This matches the widget-system spec's performance budget (spec.md line 251). The gauge SVG is intentionally simple (two layers, ~10 elements total) to stay well within this budget. resvg rasterizes simple SVGs of this complexity in < 1ms on modern hardware.

### D6: viewBox changed from 100x220 to 100x240

**Choice:** The exemplar gauge uses a 100x240 viewBox, whereas the existing test fixture at `crates/tze_hud_widget/tests/fixtures/gauge/` uses 100x220.

**Rationale:** The extra 20px of height accommodates the label text below the gauge track. The label element is positioned at approximately y=210 to y=230, below the track which ends at y=210. The existing 100x220 fixture had insufficient vertical space for the label text without clipping. The 100x240 viewBox provides 30px of clearance below the track for the label and any descenders.

## Risks / Trade-offs

- **[Risk] Discrete value_map drifts from design tokens** — The `value_map` in `widget.toml` uses concrete hex values that should match the canonical token fallbacks. If an operator overrides `color.severity.*` tokens but does not update the gauge's `value_map`, the indicator colors will be inconsistent with other severity-colored UI elements. **Mitigation:** Document this coupling in the exemplar spec. A future enhancement could support token references in value_map, but that is out of scope for v1.
- **[Risk] fill bar anchoring** — The fill bar's `y` attribute must be computed so the bar fills upward from the bottom of the track, not downward from the top. The linear binding maps `level` to `height`, but `y` must be adjusted inversely. **Mitigation:** Use a clip-path or position the bar at the track bottom and let height grow upward. The exemplar spec defines the exact SVG structure.
