## Context

The widget system spec defines three SVG parameter binding types — linear, direct, and discrete — but only linear and direct bindings have exemplars proving them end-to-end (the progress bar exercises linear f32-to-width and direct color bindings). Discrete bindings — the mapping type that converts enum string values into concrete SVG attribute values via a lookup table — have no exemplar. A status indicator is the canonical use case: a small colored dot that snaps between a fixed set of colors based on a categorical "online/away/busy/offline" parameter.

This exemplar is a specification artifact, not a code change. It defines the visual target, asset bundle structure, behavioral scenarios, and test fixtures that validate the discrete binding path of the SVG widget pipeline. It plugs into the existing `/user-test` cross-machine validation workflow.

### Current State

- The widget system spec defines discrete bindings: enum parameter -> lookup table -> SVG attribute value (spec.md §SVG Layer Parameter Bindings, "Discrete mapping applied" scenario).
- The component-shape-language spec defines `{{token.key}}` mustache placeholder resolution in SVG strings at bundle load time, including border/outline colors via `{{color.border.default}}`.
- The component-shape-language epic (hud-sc0a) is fully implemented: SVG token placeholders, design tokens, RenderingPolicy extensions, and component profiles are all live.
- No existing exemplar exercises discrete enum bindings. The progress-bar exemplar covers linear f32 bindings; this exemplar completes the binding-type coverage.

### Constraints

- The widget must be small (32x32 or 48x48) — it is an icon-sized indicator, not a full-panel visualization.
- Discrete binding snaps immediately: `transition_ms` on the publish call does NOT interpolate between enum values. Enum parameters have no interpolation path — only f32 and color parameters interpolate. The runtime must snap to the new enum value on the next frame.
- Token placeholders (`{{color.border.default}}`) are resolved at bundle load time, not at publish time. The border ring color is baked into the SVG at load, not parameterized.
- Test fixtures must be compatible with the existing MCP `publish_to_widget` tool surface.

## Goals / Non-Goals

**Goals:**
- Define the exact visual properties of a correctly rendered status indicator (circle shape, discrete color mapping, border ring, optional label text).
- Define the complete asset bundle: `widget.toml` manifest with enum + string parameter schema, `indicator.svg` with discrete color binding and text-content binding.
- Define behavioral scenarios covering all enum transitions (online/away/busy/offline) and label updates.
- Provide MCP test fixtures that exercise each enum value via `publish_to_widget`.
- Define a user-test scenario for cross-machine visual validation.

**Non-Goals:**
- Implementing any rendering code (that is widget-system task scope).
- Defining new parameter types or binding types (discrete enum binding already exists in the spec).
- Multi-indicator layouts or dashboard composition (this is a single standalone widget).
- Animated transitions between status values (enum snaps; animation is a non-goal by design).

## Decisions

### 1. Single SVG layer with circle + label, not multiple layers

**Decision:** The indicator uses a single `indicator.svg` file containing both the circle shape and the optional label `<text>` element. Two bindings target elements within this SVG: the discrete color binding on the circle's `fill`, and the text-content binding on the label element.

**Rationale:** Multiple SVG layers add compositor complexity (layer compositing, z-order) without value for a 32x32 widget. A single SVG keeps the exemplar focused on proving discrete bindings, not multi-layer composition.

### 2. Border ring uses {{color.border.default}} token placeholder, not a parameter

**Decision:** The circle's border ring (`stroke` attribute) uses `{{color.border.default}}` — a token placeholder resolved at bundle load time. It is not a runtime-parameterized attribute.

**Rationale:** The border ring is a visual-identity concern (visibility on any background), not agent-controlled data. Token resolution is the correct mechanism. This also exercises the token placeholder path within a widget SVG, proving both placeholder substitution and discrete parameter binding coexist in the same SVG file.

### 3. Default geometry is 48x48, not 32x32

**Decision:** `default_geometry` in the widget.toml is 48x48 pixels. Operators can override this in instance configuration.

**Rationale:** 32x32 is tight for a circle + label. At 48x48 the circle is 32px diameter with 16px vertical space for an optional label line. The SVG viewBox is designed for this aspect ratio. 32x32 remains a valid operator override for label-free deployments.

### 4. Status colors are hardcoded in the discrete binding, not token-derived

**Decision:** The discrete mapping table in the widget.toml binding uses literal hex colors: "online" -> "#00CC66", "away" -> "#FFB800", "busy" -> "#FF4444", "offline" -> "#666666". These are NOT derived from design tokens.

**Alternative considered:** Define custom non-canonical tokens (e.g., `color.status.online`, `color.status.away`) and use token placeholders in the discrete mapping values. This was rejected because: (a) the discrete mapping `values` in widget.toml are static TOML strings, not SVG content — token placeholders are resolved in SVG strings only, not in TOML manifest fields; (b) the colors are semantically tied to the widget's identity (green=online is universal UX), not to the operator's theme.

**Rationale:** Discrete binding values are widget-authored visual semantics. Token placeholders apply to SVG template content, not to widget.toml binding configuration.

### 5. LatestWins contention policy

**Decision:** The status indicator uses `LatestWins` contention. Only the most recent publish determines the indicator state.

**Rationale:** A status indicator represents a single entity's current state. Stack or MergeByKey makes no sense for a categorical "what is this thing's status right now?" widget.

## Risks / Trade-offs

- **[Risk] Label text may overflow at 48x48 viewBox.** Long labels (e.g., "Home Assistant Server") won't fit in ~48px width. -> Mitigation: The SVG label uses a small font size (10px) and the spec notes that labels exceeding the viewBox are clipped. The parameter schema sets `max_length: 16` for the label string.

- **[Risk] Discrete binding lookup table has no fallback for unknown enum values.** If the enum validation is bypassed somehow, the lookup returns no match. -> Mitigation: The widget parameter schema's `allowed_values` constraint rejects invalid enum values at publish time (WIDGET_PARAMETER_INVALID_VALUE). The discrete binding is never reached with an unknown value. The default parameter value ("offline") ensures the widget always has a valid initial state.

- **[Risk] Border ring visibility depends on operator's color.border.default token.** If an operator sets this to transparent, the ring disappears. -> Mitigation: The canonical fallback for `color.border.default` is `#333333` (dark gray). Operators who override tokens accept the visual consequences.
