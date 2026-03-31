# Exemplar: Progress Bar Widget

Domain: PRESENCE
Source: Proposal (exemplar-progress-bar change)
Depends on: widget-system, component-shape-language

---

## ADDED Requirements

### Requirement: Progress Bar Asset Bundle Structure
The progress bar exemplar MUST be delivered as a widget asset bundle directory named `progress-bar` containing exactly three files: `widget.toml` (manifest), `track.svg` (background track layer), and `fill.svg` (dynamic fill bar and label layer). The bundle MUST conform to the Widget Asset Bundle Format requirement from widget-system. The `widget.toml` MUST declare: `name = "progress-bar"`, `version = "1.0.0"`, `description = "Horizontal progress bar with configurable fill and label"`, a `parameter_schema` with three parameters, a `layers` array referencing both SVG files with bindings, and `default_geometry = { x = 0, y = 0, width = 300, height = 20 }`.
Scope: v1-mandatory

#### Scenario: Bundle loads successfully with default tokens
- **WHEN** the runtime scans the `progress-bar` bundle directory containing `widget.toml`, `track.svg`, and `fill.svg`, and the design token map contains all canonical fallback values
- **THEN** the runtime MUST register a Widget Type named "progress-bar" with three parameters, two SVG layers, and default geometry 300x20

#### Scenario: Bundle rejected when SVG file missing
- **WHEN** the `progress-bar` bundle directory contains `widget.toml` and `track.svg` but `fill.svg` is absent
- **THEN** the runtime MUST reject the bundle with error code WIDGET_BUNDLE_MISSING_SVG

---

### Requirement: Progress Bar Parameter Schema
The progress bar widget MUST declare exactly three parameters in its `parameter_schema`:

1. `progress` — type: f32, min: 0.0, max: 1.0, default: 0.0. Represents the fill level as a fraction of the total bar width.
2. `label` — type: string, max_length: 32, default: "" (empty string). Represents optional text centered on the bar (e.g., "75%", "Loading...", or empty for no label).
3. `fill_color` — type: color, default: [74, 158, 255, 255] (the RGBA equivalent of the canonical `color.text.accent` fallback `#4A9EFF`). Represents the fill bar color, overriding the token-derived default when published.

The runtime MUST validate all publications against this schema using the standard Widget Parameter Schema requirement from widget-system.
Scope: v1-mandatory

#### Scenario: Publish valid progress value
- **WHEN** an agent publishes `{ "progress": 0.75 }` to the progress-bar widget
- **THEN** the runtime MUST accept the publication and the fill bar MUST render at 75% of the inner track width

#### Scenario: Progress value clamped to range
- **WHEN** an agent publishes `{ "progress": 1.5 }` to the progress-bar widget
- **THEN** the runtime MUST clamp the value to 1.0 and render a fully filled bar

#### Scenario: Label published as text-content
- **WHEN** an agent publishes `{ "label": "75%" }` to the progress-bar widget
- **THEN** the label text "75%" MUST appear centered on the bar, rendered via the text-content binding on the `<text>` element

#### Scenario: Empty label shows no text
- **WHEN** the progress-bar widget has label="" (default or explicitly published)
- **THEN** no visible text MUST appear on the bar (the `<text>` element contains an empty string)

#### Scenario: Fill color overrides token default
- **WHEN** an agent publishes `{ "fill_color": [255, 0, 0, 255] }` to the progress-bar widget
- **THEN** the fill bar MUST render in red (#FF0000FF) instead of the token-derived accent color

---

### Requirement: Progress Bar SVG Track Layer
The `track.svg` file MUST define the background track — a static rounded rectangle that forms the visual boundary of the progress bar. The SVG MUST have:
- `viewBox="0 0 300 20"` matching the default geometry.
- A `<rect>` element with `id="track-bg"`, `x="0"`, `y="0"`, `width="300"`, `height="20"`, `rx="10"`, `ry="10"` for rounded end-caps, and `fill` set to `{{color.backdrop.default}}` with `fill-opacity="0.3"` (30% opacity for a muted background).
- No parameter bindings — the track layer is fully static after token resolution.

Token placeholders MUST be resolved at bundle load time per the SVG Token Placeholder Resolution requirement from component-shape-language. The resolved SVG MUST parse successfully.
Scope: v1-mandatory

#### Scenario: Track renders with token-resolved backdrop color
- **WHEN** the design token `color.backdrop.default` resolves to `"#000000"` (canonical fallback)
- **THEN** `track.svg` MUST render a rounded rectangle with fill `#000000` at 30% opacity

#### Scenario: Track renders with operator-overridden backdrop color
- **WHEN** the operator overrides `color.backdrop.default` to `"#1A1A2E"` in `[design_tokens]`
- **THEN** `track.svg` MUST render a rounded rectangle with fill `#1A1A2E` at 30% opacity

#### Scenario: Track has rounded end-caps
- **WHEN** `track.svg` is rasterized at default 300x20 geometry
- **THEN** the track rectangle MUST display rounded end-caps with a 10px radius, producing a pill/capsule shape

---

### Requirement: Progress Bar SVG Fill Layer
The `fill.svg` file MUST define the dynamic fill bar and optional label text. The SVG MUST have `viewBox="0 0 300 20"` and contain:

1. A `<rect>` element with `id="fill-bar"`, `x="2"`, `y="2"`, `width="0"` (default, overridden by binding), `height="16"`, `rx="8"`, `ry="8"` for rounded end-caps proportional to the fill bar height, and `fill="{{color.text.accent}}"` (token-resolved default, overridden by fill_color binding).
2. A `<text>` element with `id="label-text"`, positioned at `x="150"`, `y="10"` (center of viewport), with `text-anchor="middle"`, `dominant-baseline="central"`, `font-size="11"`, `fill="{{color.text.primary}}"`, and text content bound to the `label` parameter.

The fill bar rect MUST have 2px inset from the track edges (x=2, y=2, max_width=296, height=16) so it sits visually inside the track.
Scope: v1-mandatory

#### Scenario: Fill bar width tracks progress linearly
- **WHEN** the `progress` parameter is published as 0.5
- **THEN** the fill bar `<rect>` `width` attribute MUST be set to 148 (linear interpolation: 0 + (296 - 0) * 0.5 = 148)

#### Scenario: Fill bar at zero progress
- **WHEN** the `progress` parameter is 0.0 (default)
- **THEN** the fill bar `<rect>` `width` attribute MUST be 0, rendering no visible fill

#### Scenario: Fill bar at full progress
- **WHEN** the `progress` parameter is 1.0
- **THEN** the fill bar `<rect>` `width` attribute MUST be 296, filling the entire inner track area

#### Scenario: Fill color uses token-resolved default
- **WHEN** no `fill_color` parameter has been published and `color.text.accent` resolves to `"#4A9EFF"`
- **THEN** the fill bar MUST render with fill `#4A9EFF`

#### Scenario: Label text renders centered
- **WHEN** the `label` parameter is published as `"Loading..."`
- **THEN** the text "Loading..." MUST appear horizontally and vertically centered on the bar using the `<text>` element at x=150, y=10

#### Scenario: Label text uses primary color token
- **WHEN** the design token `color.text.primary` resolves to `"#FFFFFF"`
- **THEN** the label text MUST render in white (#FFFFFF)

---

### Requirement: Progress Bar Parameter Bindings
The `widget.toml` MUST declare the following parameter bindings in its `layers` section:

**Layer 0 (track.svg):** No parameter bindings. This layer is static.

**Layer 1 (fill.svg):**
1. Linear binding: `progress` parameter maps to `fill-bar` element's `width` attribute with `attr_min = 0` and `attr_max = 296`.
2. Direct binding: `fill_color` parameter maps to `fill-bar` element's `fill` attribute.
3. Text-content binding: `label` parameter maps to `label-text` element's `text-content` synthetic target.

All bindings MUST resolve at widget type registration time per the SVG Layer Parameter Bindings requirement from widget-system. The runtime MUST reject the bundle with WIDGET_BINDING_UNRESOLVABLE if any binding references a nonexistent parameter name or SVG element ID.
Scope: v1-mandatory

#### Scenario: Linear binding resolves at registration
- **WHEN** the runtime registers the progress-bar widget type and validates the linear binding from `progress` to `fill-bar.width`
- **THEN** the binding MUST resolve successfully because `progress` is declared in the parameter schema and `fill-bar` exists in `fill.svg`

#### Scenario: Direct color binding resolves at registration
- **WHEN** the runtime validates the direct binding from `fill_color` to `fill-bar.fill`
- **THEN** the binding MUST resolve successfully because `fill_color` is a color parameter and `fill` is a valid SVG attribute

#### Scenario: Text-content binding resolves at registration
- **WHEN** the runtime validates the text-content binding from `label` to `label-text.text-content`
- **THEN** the binding MUST resolve successfully because `label` is a string parameter and `label-text` is a `<text>` element

#### Scenario: Binding to missing element rejected
- **WHEN** the `fill.svg` is modified to remove the `id="fill-bar"` element but the binding still references it
- **THEN** the runtime MUST reject the bundle with error code WIDGET_BINDING_UNRESOLVABLE

---

### Requirement: Progress Bar Interpolation Behavior
When a `publish_to_widget` call includes `transition_ms > 0`, the progress bar MUST interpolate parameters according to the Widget Parameter Interpolation requirement from widget-system:

- `progress` (f32): Linear interpolation from old value to new value over the specified duration. The fill bar width MUST animate smoothly through intermediate values.
- `label` (string): Snaps to the new value immediately at t=0. No character-by-character interpolation.
- `fill_color` (color): Component-wise linear interpolation in sRGB space over the specified duration.

The recommended transition_ms for progress updates is 200ms, providing visually smooth fill animation without perceivable delay.
Scope: v1-mandatory

#### Scenario: Smooth progress animation
- **WHEN** an agent publishes `{ "progress": 1.0 }` with `transition_ms=200` to a widget currently at `progress=0.0`
- **THEN** the compositor MUST render intermediate fill bar widths (e.g., width=148 at 100ms) using linear interpolation over 200ms

#### Scenario: Label snaps during transition
- **WHEN** an agent publishes `{ "progress": 0.75, "label": "75%" }` with `transition_ms=200` to a widget currently at `label="50%"`
- **THEN** the label MUST display "75%" immediately at the start of the transition, while the fill bar animates over 200ms

#### Scenario: Color interpolates during transition
- **WHEN** an agent publishes `{ "fill_color": [255, 0, 0, 255] }` with `transition_ms=300` to a widget currently at `fill_color=[74, 158, 255, 255]`
- **THEN** the fill bar color MUST interpolate component-wise in sRGB space over 300ms

#### Scenario: Zero transition applies immediately
- **WHEN** an agent publishes `{ "progress": 0.5 }` with `transition_ms=0`
- **THEN** the fill bar MUST jump to width=148 in the next frame with a single re-rasterize

---

### Requirement: Progress Bar Rendering Performance
Re-rasterization of the progress bar widget MUST complete in less than 2ms at the default 300x20 geometry on reference hardware (single core at 3GHz equivalent, integrated GPU), per the Widget Compositor Rendering requirement from widget-system. The progress bar's minimal SVG complexity (two rects and one text element) MUST fit well within this budget.
Scope: v1-mandatory

#### Scenario: Re-rasterization under budget
- **WHEN** the `progress` parameter changes and the fill.svg layer is re-rasterized at 300x20 pixels
- **THEN** the rasterization MUST complete in less than 2ms on reference hardware

#### Scenario: Cached texture reused when unchanged
- **WHEN** a frame is rendered and no progress-bar parameters have changed since the last rasterization
- **THEN** the compositor MUST reuse the cached GPU texture without re-rasterizing

---

### Requirement: Progress Bar MCP Test Fixtures
The exemplar MUST define a sequence of `publish_to_widget` MCP tool calls that exercise the full progress bar behavior. The test fixture sequence MUST include:

1. Initial publish: `{ "progress": 0.0, "label": "" }` with `transition_ms=0` — empty bar, no label.
2. Quarter progress: `{ "progress": 0.25, "label": "25%" }` with `transition_ms=200` — animated fill to 25%.
3. Half progress: `{ "progress": 0.5, "label": "50%" }` with `transition_ms=200` — animated fill to 50%.
4. Three-quarter progress: `{ "progress": 0.75, "label": "75%" }` with `transition_ms=200` — animated fill to 75%.
5. Full progress: `{ "progress": 1.0, "label": "100%" }` with `transition_ms=200` — animated fill to 100%.
6. Color override: `{ "fill_color": [0, 200, 83, 255] }` with `transition_ms=300` — fill color changes to green.
7. Reset: `{ "progress": 0.0, "label": "" }` with `transition_ms=0` — immediate reset to empty.

Each step MUST be a valid `publish_to_widget` call with `widget_name="progress-bar"`.
Scope: v1-mandatory

#### Scenario: Full test fixture sequence executes without errors
- **WHEN** the test fixture sequence is executed against a runtime with the progress-bar widget registered and instantiated on the active tab
- **THEN** every `publish_to_widget` call MUST return success and the widget MUST render the expected visual state at each step

#### Scenario: Transition produces visible animation between steps
- **WHEN** steps 2 through 5 are published in sequence with a 1-second delay between each
- **THEN** each transition MUST produce a visible smooth fill animation over 200ms followed by a static state until the next publish

---

### Requirement: Progress Bar User-Test Integration
The exemplar MUST define a user-test scenario compatible with the existing cross-machine validation workflow. The user-test scenario MUST:

1. Verify the progress-bar widget is registered via `list_widgets`.
2. Publish the test fixture sequence (steps 1-7 from the MCP Test Fixtures requirement) with appropriate pauses between steps for visual confirmation.
3. At each step, prompt the tester to confirm the expected visual state: fill bar width, label text, fill color.
4. Report pass/fail per step.

The user-test scenario name MUST be `progress-bar-widget`.
Scope: v1-mandatory

#### Scenario: User-test discovers registered widget
- **WHEN** the user-test scenario calls `list_widgets`
- **THEN** the response MUST include a widget type named "progress-bar" with three parameters (progress, label, fill_color)

#### Scenario: User-test publishes progress sequence
- **WHEN** the user-test scenario publishes progress values 0.0, 0.25, 0.5, 0.75, 1.0 in sequence
- **THEN** each publish MUST succeed and the tester MUST see the fill bar animate to the corresponding width with the label updating at each step
