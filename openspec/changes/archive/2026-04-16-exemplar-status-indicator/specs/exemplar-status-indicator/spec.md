# Exemplar Status Indicator Specification

Domain: PRESENCE
Source: Proposal (exemplar-status-indicator change)
Depends on: widget-system, component-shape-language

---

## ADDED Requirements

### Requirement: Status Indicator Visual Contract
The status indicator widget SHALL render as a filled circle with a border ring and an optional text label below. The circle MUST be centered horizontally within the widget's viewport. The default viewport SHALL be 48x48 pixels (configurable per widget instance). The circle diameter MUST be 32 pixels at default viewport size. The border ring MUST use a 1.5px stroke in the color resolved from the `{{token.color.border.default}}` design token placeholder at bundle load time. The fill color MUST be determined by the discrete binding on the `status` enum parameter. The optional label text MUST be rendered below the circle, centered horizontally, at 10px font size in the color resolved from `{{token.color.text.secondary}}`.
Scope: v1-mandatory

#### Scenario: Default render with no publication
- **WHEN** the status indicator widget instance is created and no agent has published to it
- **THEN** the widget MUST render with the default parameter values: status="offline" (fill #666666), label="" (no label text visible), border ring in the color resolved from token `color.border.default` (default: `#333333`)

#### Scenario: Online status renders green
- **WHEN** an agent publishes status="online" to the status indicator widget
- **THEN** the circle fill MUST be #00CC66

#### Scenario: Away status renders yellow
- **WHEN** an agent publishes status="away" to the status indicator widget
- **THEN** the circle fill MUST be #FFB800

#### Scenario: Busy status renders red
- **WHEN** an agent publishes status="busy" to the status indicator widget
- **THEN** the circle fill MUST be #FF4444

#### Scenario: Offline status renders gray
- **WHEN** an agent publishes status="offline" to the status indicator widget
- **THEN** the circle fill MUST be #666666

#### Scenario: Label text displayed below circle
- **WHEN** an agent publishes status="online" and label="Butler" to the status indicator widget
- **THEN** the widget MUST render the circle with fill #00CC66 and the text "Butler" centered below the circle in the color resolved from token `color.text.secondary` (default: `#B0B0B0`)

#### Scenario: Empty label renders no text
- **WHEN** an agent publishes status="online" and label="" to the status indicator widget
- **THEN** the widget MUST render only the filled circle and border ring with no text element visible

---

### Requirement: Status Indicator Parameter Schema
The status indicator widget type MUST declare exactly two parameters in its `widget.toml` parameter_schema. The `status` parameter MUST be of type `enum` with `enum_allowed_values: ["online", "away", "busy", "offline"]` and `default: "offline"`. The `label` parameter MUST be of type `string` with `default: ""` and `string_max_bytes: 16`. The runtime MUST validate all publications against this schema per the widget-system parameter validation requirements.
Scope: v1-mandatory

#### Scenario: Valid enum publish accepted
- **WHEN** an agent publishes `{ "status": "online" }` to the status indicator widget
- **THEN** the runtime MUST accept the publication and apply the discrete color mapping

#### Scenario: Invalid enum value rejected
- **WHEN** an agent publishes `{ "status": "do-not-disturb" }` to the status indicator widget
- **THEN** the runtime MUST reject the publication with error code WIDGET_PARAMETER_INVALID_VALUE

#### Scenario: Label exceeding string_max_bytes rejected
- **WHEN** an agent publishes `{ "label": "This label is way too long for the widget" }` to the status indicator widget
- **THEN** the runtime MUST reject the publication with error code WIDGET_PARAMETER_INVALID_VALUE

#### Scenario: Partial publish retains previous values
- **WHEN** an agent publishes `{ "status": "online", "label": "Butler" }` followed by `{ "status": "away" }`
- **THEN** after the second publish the widget MUST render status="away" (fill #FFB800) and label="Butler" (retained from the previous publication)

---

### Requirement: Status Indicator Asset Bundle Structure
The status indicator asset bundle MUST contain exactly two files: `widget.toml` (manifest) and `indicator.svg` (single SVG layer). The `widget.toml` MUST declare: name `"status-indicator"`, version `"1.0.0"`, description `"Enum-driven status indicator with discrete color mapping"`, parameter_schema with the status enum and label string parameters, a single layer entry referencing `indicator.svg`, default_contention_policy of `"LatestWins"`, and bindings for the discrete color mapping and text-content label. Note: The bundle manifest has no `default_geometry` field; the loader always assigns 100% relative geometry. Per-instance geometry (e.g., 48x48) is configured via the `geometry_override` field in `[[tabs.widgets]]` config entries.
Scope: v1-mandatory

#### Scenario: Bundle loads successfully
- **WHEN** the runtime scans the status-indicator bundle directory containing a valid `widget.toml` and `indicator.svg`
- **THEN** the runtime MUST register a Widget Type named "status-indicator" with two parameters (status: enum, label: string) and one SVG layer with two bindings

#### Scenario: Discrete binding resolved at registration
- **WHEN** the widget type is registered and the discrete binding maps status enum values to fill colors
- **THEN** the runtime MUST validate that all enum values in enum_allowed_values have corresponding entries in the discrete mapping lookup table

#### Scenario: Token placeholders resolved in SVG at load time
- **WHEN** the bundle's `indicator.svg` contains `stroke="{{token.color.border.default}}"` and `fill="{{token.color.text.secondary}}"`
- **THEN** the runtime MUST substitute these placeholders with the resolved token values (fallback: `#333333` and `#B0B0B0` respectively) before SVG parsing

---

### Requirement: Status Indicator Discrete Color Binding
The status indicator MUST use a discrete binding mapping the `status` enum parameter to the `fill` attribute of the SVG element with id `indicator-fill`. The discrete mapping lookup table MUST contain exactly four entries: `"online"` maps to `"#00CC66"`, `"away"` maps to `"#FFB800"`, `"busy"` maps to `"#FF4444"`, `"offline"` maps to `"#666666"`. The binding MUST snap immediately — publishing a new enum value MUST update the fill color on the next compositor frame with no interpolation. The `transition_ms` parameter on `publish_to_widget` MUST have no effect on enum parameter transitions (enum parameters have no interpolation path).
Scope: v1-mandatory

#### Scenario: Discrete snap on status change
- **WHEN** the widget is rendering status="online" (fill #00CC66) and an agent publishes status="busy" with transition_ms=500
- **THEN** the widget MUST snap the fill to #FF4444 on the next frame — transition_ms MUST NOT cause a color interpolation between green and red

#### Scenario: Rapid enum cycling renders correct final state
- **WHEN** an agent publishes status="online", then status="away", then status="busy" in rapid succession (within one frame interval)
- **THEN** the widget MUST render the final value status="busy" (fill #FF4444) — intermediate values need not be rendered

#### Scenario: Lookup table covers all allowed values
- **WHEN** the widget type is loaded and the discrete binding is resolved
- **THEN** the binding's lookup table MUST have exactly one entry for each value in the status parameter's enum_allowed_values — no missing entries, no extra entries

---

### Requirement: Status Indicator Text-Content Binding
The status indicator MUST use a direct text-content binding mapping the `label` string parameter to the text content of the SVG element with id `label-text`. The binding target_attribute MUST be `text-content` (the synthetic binding target that replaces the element's character data, not an XML attribute). When the label value is an empty string, the `<text>` element MUST contain no character data (effectively invisible). The label text MUST NOT be truncated or ellipsized by the binding — string_max_bytes enforcement in the parameter schema ensures values fit.
Scope: v1-mandatory

#### Scenario: Label text updated via text-content binding
- **WHEN** an agent publishes `{ "label": "Butler" }` to the status indicator widget
- **THEN** the SVG element with id `label-text` MUST have its text node content replaced with "Butler"

#### Scenario: Empty label clears text content
- **WHEN** an agent publishes `{ "label": "" }` to the status indicator widget
- **THEN** the SVG element with id `label-text` MUST have its text node content set to the empty string

---

### Requirement: Status Indicator Contention Policy
The status indicator widget MUST use LatestWins contention policy. When multiple agents publish to the same status indicator instance, only the most recent publication's parameter values SHALL determine the widget occupancy. The displaced agent's publication SHALL be fully replaced — no partial merging of parameters across publications.
Scope: v1-mandatory

#### Scenario: Second agent wins
- **WHEN** Agent A publishes `{ "status": "online", "label": "AgentA" }` and then Agent B publishes `{ "status": "busy", "label": "AgentB" }` to the same instance
- **THEN** the widget occupancy MUST reflect status="busy", label="AgentB" — Agent A's values are fully displaced

#### Scenario: Single agent updates own publication
- **WHEN** Agent A publishes `{ "status": "online" }` then later publishes `{ "status": "away" }`
- **THEN** the widget occupancy MUST reflect status="away" with label retained from the first publication (partial update semantics within the winning publication)

---

### Requirement: Status Indicator Performance Budget
Re-rasterization of the status indicator SVG at 48x48 pixels MUST complete in under 2ms on reference hardware. This budget is trivially met: a 48x48 SVG with a single circle and optional text is orders of magnitude below the 2ms budget for 512x512 widgets defined in the widget system spec. The exemplar states this budget explicitly for completeness.
Scope: v1-mandatory

#### Scenario: Re-rasterization under budget
- **WHEN** the status parameter changes from "online" to "busy" and the compositor re-rasterizes the 48x48 SVG
- **THEN** the rasterization MUST complete in under 2ms on reference hardware

---

### Requirement: Status Indicator MCP Test Fixtures
The exemplar MUST define MCP test fixtures exercising all status indicator behaviors via `publish_to_widget`. The fixtures MUST include: (1) a cycle test publishing each enum value in sequence ("online", "away", "busy", "offline") with 1-second delays between publishes, verifying each discrete color renders correctly; (2) a label test publishing status="online" with label="Butler", then label="Codex", then label="" to verify text-content binding updates; (3) a contention test publishing from two namespaces to verify LatestWins; (4) a validation test publishing an invalid enum value to verify WIDGET_PARAMETER_INVALID_VALUE rejection.
Scope: v1-mandatory

#### Scenario: Enum cycle test exercises all four colors
- **WHEN** the MCP test fixture publishes `publish_to_widget(widget_name="status-indicator", params={"status": "online"})` followed by "away", "busy", "offline" with 1-second delays
- **THEN** the widget MUST render #00CC66, then #FFB800, then #FF4444, then #666666 in sequence — each snap is visually distinct with no intermediate colors

#### Scenario: Label update test exercises text-content binding
- **WHEN** the MCP test fixture publishes label="Butler" then label="Codex" then label=""
- **THEN** the label text MUST update to "Butler", then "Codex", then disappear — each change is immediate

#### Scenario: Invalid enum publish returns error
- **WHEN** the MCP test fixture publishes `{ "status": "do-not-disturb" }` to the status indicator
- **THEN** the MCP tool MUST return an error response with code WIDGET_PARAMETER_INVALID_VALUE

---

### Requirement: Status Indicator User-Test Integration
The exemplar MUST define a user-test scenario compatible with the `/user-test` cross-machine validation workflow. The test MUST: (1) deploy the status-indicator asset bundle to the target machine, (2) publish a sequence of status changes via MCP `publish_to_widget`, (3) require human visual confirmation that the indicator circle changes color correctly for each enum value, and (4) verify the label text appears and disappears as expected.
Scope: v1-mandatory

#### Scenario: Cross-machine visual validation
- **WHEN** an operator runs the status indicator user-test from the development machine targeting a remote display
- **THEN** the status indicator widget MUST be visible on the remote display, cycling through green (online), yellow (away), red (busy), and gray (offline) with label text appearing and clearing as published

---

### Requirement: Status Indicator SVG Template
The `indicator.svg` file MUST define a valid SVG document with viewBox `"0 0 48 48"` containing: (1) a `<circle>` element with id `indicator-fill`, cx="24", cy="20", r="14", fill="#666666" (default offline color, overridden by discrete binding at runtime), stroke=`"{{token.color.border.default}}"`, stroke-width="1.5"; (2) a `<text>` element with id `label-text`, x="24", y="44", text-anchor="middle", font-size="10", fill=`"{{token.color.text.secondary}}"`, containing empty default text content. The SVG MUST NOT contain any scripting, external references, or animation elements. Token placeholders MUST be limited to `{{token.color.border.default}}` (circle stroke) and `{{token.color.text.secondary}}` (label fill).
Scope: v1-mandatory

#### Scenario: SVG structure valid for resvg
- **WHEN** the runtime loads `indicator.svg` after token placeholder resolution
- **THEN** the resolved SVG MUST parse successfully with resvg and contain exactly two addressable elements: circle `indicator-fill` and text `label-text`

#### Scenario: Token placeholders resolve to canonical fallbacks
- **WHEN** no design token overrides are configured (empty `[design_tokens]` section)
- **THEN** `{{token.color.border.default}}` MUST resolve to `#333333` and `{{token.color.text.secondary}}` MUST resolve to `#B0B0B0` in the SVG before parsing

#### Scenario: SVG has no external dependencies
- **WHEN** the runtime loads `indicator.svg`
- **THEN** the SVG MUST NOT reference external fonts, images, stylesheets, or scripts — it MUST be fully self-contained
