## Why

The widget system spec defines a complete SVG-based rendering pipeline — asset bundles, parameter schemas, linear/direct/discrete bindings, token placeholder resolution, compositor rasterization, and parameter interpolation — but no exemplar proves it works end-to-end for a common UI pattern beyond the gauge. A horizontal progress bar is the simplest non-trivial widget: a fill bar whose width tracks a `[0.0, 1.0]` value, with token-driven colors and optional label text. It exercises the critical subset of the widget pipeline (linear f32 binding to SVG width, direct color binding, text-content binding, `{{token}}` placeholders, transition_ms interpolation) without the complexity of circular geometry or multi-indicator layouts.

A polished progress bar exemplar serves three purposes: (1) it proves the SVG asset bundle format, parameter binding resolution, and token placeholder substitution work together correctly for a width-based linear mapping, (2) it defines concrete `publish_to_widget` MCP call sequences that exercise progress interpolation, label updates, and color overrides, and (3) it provides ready-made user-test fixtures for cross-machine visual validation.

## What Changes

- Define a **progress bar widget exemplar specification** covering the complete visual contract: thin horizontal bar (300x20 default geometry), background track with rounded end-caps in `{{color.backdrop.default}}` at 30% opacity, fill bar in `{{color.text.accent}}` (or configurable `fill_color`) with rounded end-caps via SVG `rx`/`ry`, optional centered label text.
- Define the **asset bundle** structure: `widget.toml` manifest with parameter_schema (progress: f32, label: string, fill_color: color), layers (track.svg, fill.svg), and bindings (linear mapping for fill width, direct mapping for fill color, text-content binding for label).
- Define **behavioral test scenarios**: smooth progress interpolation with transition_ms=200, immediate label snap, re-rasterization under 2ms budget at default size.
- Define **MCP test fixtures** — concrete `publish_to_widget` call sequences stepping progress through 0.0 -> 0.25 -> 0.50 -> 0.75 -> 1.0 with transition_ms, label updates, and fill_color overrides.
- Define a **user-test scenario** where an agent publishes progress updates and the visual result is confirmed.

## Capabilities

### New Capabilities
- `exemplar-progress-bar`: Production-quality horizontal progress bar widget exemplar defining the visual contract, asset bundle structure, parameter bindings, behavioral scenarios, MCP test fixtures, and user-test integration — the canonical proof that the SVG widget pipeline handles width-based linear bindings.

### Modified Capabilities

(none — this exemplar defines test/validation artifacts for existing widget-system and component-shape-language capabilities, it does not change spec-level requirements)

## Impact

- **Asset bundles**: New exemplar bundle directory with `widget.toml`, `track.svg`, `fill.svg` serving as a reference implementation for widget authors.
- **Test fixtures**: New JSON test message files for progress-bar-specific scenarios.
- **User-test workflow**: Progress-bar-specific test scenario added to the existing cross-machine validation workflow.
- **Implementer guidance**: Concrete rendering targets and acceptance criteria for the widget compositor SVG pipeline (linear width binding, token resolution, interpolation, text-content binding).
- **No runtime code changes**: This exemplar defines the target — implementation is driven by the widget-system tasks (bundle loading, parameter binding, compositor rasterization).
