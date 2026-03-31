## Context

The widget system defines a four-level ontology (type, instance, publication, occupancy) with SVG asset bundles, parameter bindings, and design token placeholder resolution. The progress bar exemplar is the second widget exemplar (alongside gauges referenced in the widget-system spec scenarios) and the first to exercise **width-based linear binding** — the fill bar width scales linearly with the `progress` parameter rather than using height or rotation.

The component-shape-language spec defines the SVG token placeholder resolution mechanism: `{{token.key}}` patterns in raw SVG are substituted at bundle load time. The widget-system spec defines linear, direct, and text-content binding types. This exemplar combines all three binding types in a single widget with minimal complexity.

Existing infrastructure:
- Widget bundle loading from `widget.toml` + SVG files (widget-system spec)
- Parameter schema validation: f32 with min/max, string with max_length, color with RGBA
- Linear binding: maps f32 `[min, max]` to `[attr_min, attr_max]` on SVG attributes
- Direct binding: color parameter value set as-is on SVG fill/stroke attributes
- Text-content binding: string parameter replaces text node content inside `<text>`/`<tspan>` elements
- Token placeholder substitution: `{{token.key}}` resolved at load time
- Parameter interpolation: linear interpolation for f32, component-wise for color, immediate snap for string

## Goals / Non-Goals

**Goals:**
- Define a complete, production-quality widget asset bundle (widget.toml, track.svg, fill.svg) that is self-contained and exemplary for widget authors.
- Exercise the three binding types (linear, direct, text-content) in a single widget.
- Validate the token placeholder pipeline with `{{color.backdrop.default}}`, `{{color.text.accent}}`, `{{color.text.primary}}`.
- Provide concrete MCP `publish_to_widget` test sequences that step through progress values with smooth interpolation.
- Serve as a user-test fixture for visual validation of widget rendering.

**Non-Goals:**
- Vertical progress bar variant (horizontal only in this exemplar).
- Indeterminate/spinner mode (only determinate progress from 0.0 to 1.0).
- Accessibility labels or ARIA roles (widgets are display-only, input_mode=Passthrough).
- Custom track height or end-cap radius as parameters (these are baked into the SVG geometry; configurable via token overrides or authoring new SVG files post-v1).
- Animated shimmer or pulse effects (pure parameter-driven rendering only).

## Decisions

### SVG layer split: track.svg + fill.svg

**Decision:** Two SVG layers — track.svg for the static background track, fill.svg for the dynamic fill bar and label.

**Rationale:** Separating static from dynamic minimizes re-rasterization scope. When only `progress` or `fill_color` changes, only fill.svg needs re-rasterization. The track is static after token resolution and only re-rasterizes on geometry resize. This matches the layer-per-concern pattern recommended by the widget-system spec.

**Alternative considered:** Single SVG with all elements. Rejected because any parameter change would re-rasterize the entire SVG including the static track, wasting the <2ms budget.

### Fill bar width via linear binding on SVG `width` attribute

**Decision:** The fill bar uses a `<rect>` element with `id="fill-bar"` whose `width` attribute is bound linearly to the `progress` parameter: `[0.0, 1.0]` maps to `[0, 296]` (inner width accounting for 2px padding on each side of a 300px bar).

**Rationale:** Linear binding on `width` is the simplest way to express "fill proportional to value." The widget-system spec already defines linear mapping with `attr_min`/`attr_max`. The 296px inner width comes from: 300px total width - 2px left padding - 2px right padding.

**Alternative considered:** SVG `clipPath` or `viewBox` manipulation. Rejected as more complex and no better visually; `width` directly controls the rectangle extent.

### Rounded end-caps via SVG rx/ry on rect elements

**Decision:** Both track and fill bar `<rect>` elements use `rx`/`ry` for rounded end-caps. The fill bar uses rx=8/ry=8 (half its 16px height, producing semicircle end-caps) while the track uses rx=10/ry=10 (half the 20px track height). The radii are proportional to their respective heights, producing visually complementary curves rather than identical curvature.

**Rationale:** SVG `rx`/`ry` attributes on `<rect>` produce rounded corners natively without path commands. Setting the radius to half the element height produces a half-circle end-cap, giving a capsule/pill shape. Each rect uses its own proportional radius: 10px for the 20px track, 8px for the 16px fill bar.

### Label as text-content binding

**Decision:** The label text uses a `<text>` element with `id="label-text"` centered on the fill.svg layer. The `label` parameter uses a text-content binding to replace the text node content.

**Rationale:** Text-content binding is the spec-defined mechanism for string-to-text-node mapping. Centering uses `text-anchor="middle"` and `dominant-baseline="central"` at `x="150" y="10"` (center of the 300x20 viewport).

### Default fill color from token, overridable via parameter

**Decision:** The fill bar's `fill` attribute in the SVG template uses `{{color.text.accent}}` as the initial value. The `fill_color` parameter (type: color) has a direct binding that overrides this attribute at runtime.

**Rationale:** Token-driven default means the progress bar inherits the deployment's accent color out of the box. The `fill_color` parameter allows per-publish overrides without modifying the bundle — an agent can publish `fill_color=[255, 0, 0, 255]` for a red error progress bar.

**Ordering:** Token substitution happens at bundle load time (resolving `{{color.text.accent}}` to e.g. `#4A9EFF`). Parameter binding resolution happens at registration time (validating the binding target exists). Parameter publication happens at runtime (applying the new color). This means the token value is the effective default until a publication overrides it.

### Transition behavior

**Decision:** Progress changes use transition_ms=200 for smooth interpolation. Label changes snap immediately. Color changes interpolate component-wise.

**Rationale:** This follows the widget-system spec exactly: f32 parameters use linear interpolation, string parameters snap at t=0, color parameters use component-wise sRGB interpolation. The 200ms default is a UX choice for the test fixtures — callers can use any transition_ms value.

## Risks / Trade-offs

**[Risk] Fill bar at progress=0.0 with rx/ry still shows rounded end-caps as a tiny sliver** -- Mitigation: When width=0, the fill rect collapses to nothing (width=0 rect is not rendered by SVG). At very small widths (< 2*rx), the rounded corners merge and the bar appears as an ellipse rather than a pill, which is acceptable visual behavior.

**[Risk] Label text overflows bar at small widths** -- Mitigation: The label is placed on the fill.svg layer which spans the full 300x20 viewport, not clipped to the fill bar rect. The label sits on top of both track and fill visually. Long labels are the caller's responsibility — the max_length=32 constraint limits text to short strings like "75%", "Loading...", or "3/10".

**[Risk] Token resolution failure blocks bundle loading** -- Mitigation: All referenced tokens (`color.backdrop.default`, `color.text.accent`, `color.text.primary`) are canonical tokens with hardcoded fallbacks. They will always resolve unless the operator provides an invalid override, which is caught at startup with `WIDGET_BUNDLE_UNRESOLVED_TOKEN`.
