# exemplar-gauge-widget Specification

## Purpose
Defines the gauge widget exemplar bundle, parameter schema, SVG layer bindings, and token-driven vertical fill rendering used by the widget publishing path.

## Requirements
### Requirement: Gauge Widget Visual Contract
The exemplar gauge widget MUST be a vertical fill gauge consisting of exactly two SVG layers: a static background layer (`background.svg`) providing the frame and track, and a dynamic fill layer (`fill.svg`) containing the fill bar, label text, and severity indicator. The gauge MUST use a 100x240 viewBox for both layers. The background layer MUST contain a rounded outer frame rectangle and an inner track rectangle. The fill layer MUST contain a fill bar rectangle (bound to `level`), a text label element (bound to `label`), and a severity indicator circle (bound to `severity`). The fill bar MUST fill upward from the bottom of the track: the bar is positioned at the track bottom with `height` driven by the `level` parameter via linear binding, and a `clipPath` constraining the fill to the track area. All visible colors in both SVG layers MUST use `{{token.key}}` design token placeholders — no hardcoded hex color values in the SVG markup.
Scope: v1-mandatory

#### Scenario: Background layer structure
- **WHEN** the gauge widget bundle is loaded and token placeholders are resolved
- **THEN** `background.svg` MUST contain a `<rect>` outer frame with `fill="{{token.color.backdrop.default}}"`, `stroke="{{token.color.border.default}}"`, `data-role="backdrop"`, and an inner track `<rect>` with `fill="{{token.color.backdrop.default}}"` and `fill-opacity="0.3"` (track uses `{{token.color.backdrop.default}}` at 30% opacity for a lighter appearance)

#### Scenario: Fill layer structure
- **WHEN** the gauge widget bundle is loaded and token placeholders are resolved
- **THEN** `fill.svg` MUST contain a `<rect id="bar">` for the fill bar with `fill="{{token.color.text.accent}}"`, a `<text id="label-text">` with `fill="{{token.color.text.primary}}"`, `stroke="{{token.color.outline.default}}"`, `stroke-width="{{token.stroke.outline.width}}"`, and `data-role="text"`, and a `<circle id="indicator">` with `fill="{{token.color.severity.info}}"`

#### Scenario: Fill bar grows upward from track bottom
- **WHEN** the `level` parameter changes from 0.0 to 1.0
- **THEN** the fill bar MUST grow upward from the bottom of the track (y=210 minus height) to fill the full track height (200px), constrained by a clipPath to the track area

#### Scenario: All colors are token-driven
- **WHEN** the gauge SVG layers are inspected before token resolution
- **THEN** every `fill`, `stroke`, and color-bearing attribute MUST use a `{{token.key}}` placeholder — there MUST be zero hardcoded hex color values in the raw SVG markup

---

### Requirement: Gauge Widget Parameter Schema
The gauge widget MUST declare exactly four parameters in its `widget.toml` parameter schema. The `level` parameter MUST be type `f32` with default `0.0` and constraints `f32_min=0.0`, `f32_max=1.0`. The `label` parameter MUST be type `string` with default `""`. The `fill_color` parameter MUST be type `color` with default `[74, 158, 255, 255]` (matching the `color.text.accent` canonical fallback `#4A9EFF`). The `severity` parameter MUST be type `enum` with default `"info"` and `enum_allowed_values` `["info", "warning", "error"]`. The parameter order in the manifest MUST be: level, label, fill_color, severity.
Scope: v1-mandatory

#### Scenario: Level parameter validated and clamped
- **WHEN** an agent publishes `level=1.5` to the gauge widget
- **THEN** the runtime MUST clamp the value to `1.0` and accept the publication

#### Scenario: Level parameter rejects NaN
- **WHEN** an agent publishes `level=NaN` to the gauge widget
- **THEN** the runtime MUST reject the publication with error code `WIDGET_PARAMETER_INVALID_VALUE`

#### Scenario: Unknown parameter rejected
- **WHEN** an agent publishes `{"temperature": 72}` to the gauge widget
- **THEN** the runtime MUST reject the publication with error code `WIDGET_UNKNOWN_PARAMETER`

#### Scenario: Severity enum validated
- **WHEN** an agent publishes `severity="critical"` to the gauge widget (not in allowed values)
- **THEN** the runtime MUST reject the publication with error code `WIDGET_PARAMETER_INVALID_VALUE`

#### Scenario: Default parameter values at startup
- **WHEN** the gauge widget instance is created with no initial parameter overrides
- **THEN** the effective parameters MUST be `level=0.0`, `label=""`, `fill_color=[74,158,255,255]`, `severity="info"`

---

### Requirement: Gauge Widget Binding Configuration
The gauge widget MUST declare exactly four parameter bindings on the fill layer. The `level` binding MUST map parameter `level` to SVG element `bar`, attribute `height`, with `linear` mapping from `attr_min=0.0` to `attr_max=200.0`. The `fill_color` binding MUST map parameter `fill_color` to SVG element `bar`, attribute `fill`, with `direct` mapping. The `label` binding MUST map parameter `label` to SVG element `label-text`, attribute `text-content`, with `direct` mapping. The `severity` binding MUST map parameter `severity` to SVG element `indicator`, attribute `fill`, with `discrete` mapping and a `value_map` of `info="#4A9EFF"`, `warning="#FFB800"`, `error="#FF4444"` (matching canonical `color.severity.*` token fallback values). The background layer MUST have zero bindings (it is static).
Scope: v1-mandatory

#### Scenario: Linear binding maps level to bar height
- **WHEN** `level=0.5` is published
- **THEN** the `height` attribute of SVG element `bar` MUST be set to `100.0` (linear interpolation: 0.5 * 200.0)

#### Scenario: Direct color binding sets fill
- **WHEN** `fill_color=[255, 0, 0, 255]` is published
- **THEN** the `fill` attribute of SVG element `bar` MUST be set to the SVG color representation of red (e.g., `rgb(255,0,0)` or `#FF0000`)

#### Scenario: Text-content binding sets label
- **WHEN** `label="CPU Load"` is published
- **THEN** the text content of SVG element `label-text` MUST be replaced with `"CPU Load"`

#### Scenario: Discrete binding maps severity to indicator color
- **WHEN** `severity="warning"` is published
- **THEN** the `fill` attribute of SVG element `indicator` MUST be set to `"#FFB800"`

#### Scenario: Level at zero renders empty track
- **WHEN** `level=0.0` is published
- **THEN** the `height` attribute of SVG element `bar` MUST be `0.0` and the fill bar MUST be visually invisible (zero height)

#### Scenario: Level at one renders full track
- **WHEN** `level=1.0` is published
- **THEN** the `height` attribute of SVG element `bar` MUST be `200.0` and the fill bar MUST fill the entire track

---

### Requirement: Gauge Widget Interpolation Behavior
When a gauge widget publication includes `transition_ms > 0`, the compositor MUST interpolate `level` (f32) and `fill_color` (color) parameters smoothly over the transition duration. The `level` parameter MUST use linear interpolation: `value(t) = old + (new - old) * (elapsed_ms / transition_ms)`. The `fill_color` parameter MUST use component-wise linear interpolation in sRGB space (each of R, G, B, A interpolated independently). The `label` (string) parameter MUST snap to the new value immediately at `t=0`. The `severity` (enum) parameter MUST snap to the new value immediately at `t=0`. The recommended default transition duration for the gauge exemplar is `transition_ms=300` for smooth level and color changes. When `transition_ms=0`, all parameters MUST be applied immediately with a single re-rasterize.
Scope: v1-mandatory

#### Scenario: Level animates smoothly over 300ms
- **WHEN** an agent publishes `level=0.8` with `transition_ms=300` to a gauge currently at `level=0.2`
- **THEN** at `t=150ms` the effective level MUST be approximately `0.5` (linear interpolation midpoint) and the fill bar height MUST reflect this intermediate value

#### Scenario: Color interpolates component-wise
- **WHEN** an agent publishes `fill_color=[255, 0, 0, 255]` with `transition_ms=300` to a gauge currently at `fill_color=[0, 0, 255, 255]`
- **THEN** at `t=150ms` the effective fill color MUST be approximately `[128, 0, 128, 255]` (midpoint of each sRGB component)

#### Scenario: Label snaps immediately during transition
- **WHEN** an agent publishes `label="Memory"` with `transition_ms=500` to a gauge currently at `label="CPU"`
- **THEN** the label text MUST display `"Memory"` immediately at `t=0`, not interpolate character-by-character

#### Scenario: Severity snaps immediately during transition
- **WHEN** an agent publishes `severity="error"` with `transition_ms=500` to a gauge currently at `severity="info"`
- **THEN** the indicator color MUST change to `"#FF4444"` immediately at `t=0`

#### Scenario: Zero transition applies all parameters instantly
- **WHEN** an agent publishes `level=0.9, label="Disk", severity="warning"` with `transition_ms=0`
- **THEN** all three parameters MUST be applied in the next frame with a single re-rasterize

---

### Requirement: Gauge Widget Contention Policy
The gauge widget instance MUST use `LatestWins` contention policy by default. When two agents publish to the same gauge widget instance, only the most recent publication's parameter values MUST be rendered. The previous publication MUST be fully replaced. There is no parameter-level merge between publications — the entire parameter set from the latest publication wins, with unpublished parameters retaining their previous values (or schema defaults if no prior publication exists).
Scope: v1-mandatory

#### Scenario: Latest publication wins
- **WHEN** Agent A publishes `level=0.3` and then Agent B publishes `level=0.7` to the same gauge instance
- **THEN** the gauge MUST render `level=0.7` (Agent B's publication replaces Agent A's)

#### Scenario: Unpublished parameters retained
- **WHEN** Agent A publishes `level=0.5, label="CPU"` and then Agent B publishes `level=0.8` (without label)
- **THEN** the gauge MUST render `level=0.8` from Agent B's publication, and `label` MUST retain `"CPU"` from Agent A's publication

---

### Requirement: Gauge Widget Readability Conventions
The gauge widget SVG layers MUST follow the widget SVG readability conventions. The background layer's outer frame rectangle MUST include `data-role="backdrop"` to identify it as the backdrop element for readability validation. The fill layer's label text element MUST include `data-role="text"` to identify it as a text element for readability validation. Elements that are functional visuals (fill bar, severity indicator, track) MUST NOT use `data-role` attributes — they are not subject to readability checks. When the gauge widget is used in a component profile with a text-bearing component type, the `data-role` attributes enable the readability validator to verify that backdrop and text elements meet the component type's readability technique requirements.
Scope: v1-mandatory

#### Scenario: Backdrop element identified
- **WHEN** the readability validator scans the gauge background SVG
- **THEN** it MUST find exactly one element with `data-role="backdrop"` (the outer frame rectangle)

#### Scenario: Text element identified
- **WHEN** the readability validator scans the gauge fill SVG
- **THEN** it MUST find exactly one element with `data-role="text"` (the label text element)

#### Scenario: Functional visuals excluded from readability
- **WHEN** the readability validator scans both gauge SVG layers
- **THEN** the fill bar (`id="bar"`), severity indicator (`id="indicator"`), and track rectangle (`id="track"`) MUST NOT have `data-role` attributes

---

### Requirement: Gauge Widget Performance Budget
Re-rasterization of the gauge widget after any single parameter change MUST complete in less than 2ms at 512x512 pixel resolution on reference hardware. The gauge SVG is intentionally minimal (two layers, approximately 10 elements total) to remain well within this budget. The compositor MUST cache the rasterized GPU texture and reuse it when no parameters have changed — unchanged frames MUST NOT trigger re-rasterization.
Scope: v1-mandatory

#### Scenario: Re-rasterization within budget
- **WHEN** a 512x512 pixel gauge widget is re-rasterized after a `level` parameter change
- **THEN** the re-rasterization MUST complete in less than 2ms on reference hardware

#### Scenario: Unchanged parameters reuse cache
- **WHEN** a frame is rendered and no gauge parameters have changed since the last rasterization
- **THEN** the compositor MUST reuse the cached GPU texture without re-rasterizing

---

### Requirement: Gauge Widget Asset Bundle Structure
The gauge widget exemplar asset bundle MUST be a directory containing exactly three files: `widget.toml` (manifest with full parameter schema, layers, and bindings), `background.svg` (static background frame and track layer), and `fill.svg` (dynamic fill bar, label text, and severity indicator layer with parameter bindings). The `widget.toml` MUST declare `name = "gauge"`, `version = "1.0.0"`, and a human-readable `description`. The manifest MUST list exactly two layers in order: `background.svg` (no bindings) then `fill.svg` (four bindings). The bundle MUST pass all validation checks: manifest parsing, SVG parse validation, token placeholder resolution, and binding resolution.
Scope: v1-mandatory

#### Scenario: Bundle loads successfully
- **WHEN** the runtime loads the exemplar gauge widget bundle directory
- **THEN** the runtime MUST register a Widget Type named `"gauge"` with four parameters, two layers, and four bindings

#### Scenario: Token placeholders resolved at load time
- **WHEN** the runtime loads the gauge SVG files
- **THEN** all `{{token.key}}` placeholders MUST be resolved to concrete color values from the design token map before SVG parsing

#### Scenario: Bundle passes all validation
- **WHEN** the gauge bundle is validated during startup
- **THEN** it MUST pass manifest validation, SVG parse validation, token resolution (no `WIDGET_BUNDLE_UNRESOLVED_TOKEN`), and binding resolution (no `WIDGET_BINDING_UNRESOLVABLE`)

---

### Requirement: Gauge Widget MCP Test Scenarios
The gauge widget exemplar MUST define concrete MCP `publish_to_widget` test scenarios that exercise all four parameter types, animated transitions, and contention behavior. The following test sequences MUST be specified as reference MCP calls.
Scope: v1-mandatory

#### Scenario: Set level via MCP
- **WHEN** an agent calls `publish_to_widget` with `widget_name="gauge"`, `params={"level": 0.75}`
- **THEN** the gauge fill bar MUST render at 75% of the track height

#### Scenario: Animate level with transition
- **WHEN** an agent calls `publish_to_widget` with `widget_name="gauge"`, `params={"level": 0.9}`, `transition_ms=300`
- **THEN** the gauge fill bar MUST animate smoothly from its current level to 90% over 300ms

#### Scenario: Set all four parameters
- **WHEN** an agent calls `publish_to_widget` with `widget_name="gauge"`, `params={"level": 0.65, "label": "CPU", "fill_color": [255, 165, 0, 255], "severity": "warning"}`
- **THEN** the gauge MUST render with 65% fill, "CPU" label, orange fill color, and yellow warning indicator

#### Scenario: Change severity to error
- **WHEN** an agent calls `publish_to_widget` with `widget_name="gauge"`, `params={"severity": "error"}`
- **THEN** the severity indicator MUST change to `#FF4444` (error red) immediately

#### Scenario: Rapid successive publishes
- **WHEN** an agent publishes `level=0.3` then immediately publishes `level=0.8` with `transition_ms=300`
- **THEN** the gauge MUST animate from 0.3 to 0.8 (the second publish interrupts any in-progress transition from the first)

---

### Requirement: Gauge Widget User-Test Integration
The gauge widget exemplar MUST define a user-test scenario compatible with the cross-machine validation workflow. The test sequence MUST: (1) publish initial gauge parameters via MCP (`level=0.0, label="System", severity="info"`), (2) animate level from 0.0 to 0.75 with `transition_ms=300`, (3) change severity to `"warning"`, (4) animate level from 0.75 to 0.95 with `transition_ms=300`, (5) change severity to `"error"` and label to `"CRITICAL"`. Each step MUST be visually verifiable on the target display.
Scope: v1-mandatory

#### Scenario: User-test gauge sequence
- **WHEN** the user-test script executes the five-step gauge publish sequence
- **THEN** the gauge MUST visually progress from empty/info through 75%/warning to 95%/error with smooth fill animation and immediate severity/label snaps

#### Scenario: User-test verifies token-driven colors
- **WHEN** the user-test gauge renders on the target display
- **THEN** the background frame color MUST match `color.backdrop.default`, the label text color MUST match `color.text.primary`, and the severity indicator colors MUST match the configured `color.severity.*` tokens

---

### Requirement: Gauge Widget SVG Markup Specification
The exemplar gauge MUST define exact SVG markup for both layers. The `background.svg` MUST use the following token placeholders: `{{token.color.backdrop.default}}` for the frame fill, `{{token.color.border.default}}` for the frame stroke, and a track fill using `{{token.color.backdrop.default}}` with `fill-opacity="0.3"` (track uses the backdrop token at 30% opacity for a lighter appearance). The `fill.svg` MUST use: `{{token.color.text.accent}}` for the default fill bar color (overridden at runtime by the `fill_color` direct binding), `{{token.color.text.primary}}` for the label text fill with `stroke="{{token.color.outline.default}}"` and `stroke-width="{{token.stroke.outline.width}}"` for DualLayer readability, and `{{token.color.severity.info}}` for the default indicator fill (overridden at runtime by the `severity` discrete binding). Note: The stroke attributes on the label text element make the gauge compatible with DualLayer readability profiles even though it ships as a global bundle. The fill layer MUST define a `<clipPath>` element constraining the fill bar to the track area so the bar grows upward naturally.
Scope: v1-mandatory

#### Scenario: background.svg token resolution
- **WHEN** `background.svg` is loaded with default canonical tokens
- **THEN** the frame fill MUST resolve to `#000000` (color.backdrop.default fallback), the frame stroke MUST resolve to `#333333` (color.border.default fallback)

#### Scenario: fill.svg token resolution
- **WHEN** `fill.svg` is loaded with default canonical tokens
- **THEN** the default bar fill MUST resolve to `#4A9EFF` (color.text.accent fallback), the label text fill MUST resolve to `#FFFFFF` (color.text.primary fallback), the indicator fill MUST resolve to `#4A9EFF` (color.severity.info fallback)

#### Scenario: Operator token override changes gauge colors
- **WHEN** an operator overrides `color.text.primary = "#00FF00"` in `[design_tokens]`
- **THEN** the gauge label text MUST render in green (`#00FF00`) after token resolution at startup

#### Scenario: ClipPath constrains fill bar to track
- **WHEN** `level=1.0` is published
- **THEN** the fill bar MUST fill exactly the track area and MUST NOT overflow beyond the track boundaries
