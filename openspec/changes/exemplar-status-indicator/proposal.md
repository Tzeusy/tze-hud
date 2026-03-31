## Why

The widget system spec defines discrete enum-to-attribute bindings as a first-class mapping type, but no exemplar proves the discrete binding path works end-to-end. The progress-bar exemplar exercises linear f32 bindings; this exemplar exercises the complementary discrete enum binding — the mapping type that converts categorical values into concrete visual properties without interpolation. A status indicator is the simplest non-trivial widget for this: a small colored dot whose fill snaps between exactly four colors based on an enum parameter, with an optional label. It proves that enum parameters, discrete binding lookup tables, immediate-snap transitions (no interpolation), and token-driven fallback colors work together correctly through the SVG widget pipeline.

## What Changes

- Define a **status indicator widget exemplar specification** covering the complete visual contract: small icon-sized widget (48x48 default, 32x32 optional), circle indicator shape, discrete color mapping from enum values ("online" -> #00CC66, "away" -> #FFB800, "busy" -> #FF4444, "offline" -> #666666), optional label text below the indicator, and a subtle border ring using `{{color.border.default}}` for visibility on any background.
- Define the **asset bundle** structure: `widget.toml` manifest with parameter_schema (status: enum with allowed_values, label: string), single SVG layer `indicator.svg` containing the circle shape with discrete color binding on fill and text-content binding for the label.
- Define **behavioral test scenarios**: enum value snaps immediately (no interpolation — discrete mapping has no transition_ms effect on the enum parameter itself), LatestWins contention policy, re-rasterization under 2ms at 48x48 (trivially met at this small size).
- Define **MCP test fixtures** — concrete `publish_to_widget` call sequences cycling through each enum value ("online", "away", "busy", "offline") and verifying each renders the correct discrete-mapped color.
- Define a **user-test scenario** where an agent publishes status changes and the visual result is confirmed on a real display.

## Capabilities

### New Capabilities
- `exemplar-status-indicator`: Production-quality enum-driven status indicator widget exemplar defining the visual contract, asset bundle structure, discrete parameter bindings, behavioral scenarios, MCP test fixtures, and user-test integration — the canonical proof that the SVG widget pipeline handles discrete enum-to-color mappings.

### Modified Capabilities

(none — this exemplar defines test/validation artifacts for existing widget-system and component-shape-language capabilities, it does not change spec-level requirements)

## Impact

- **Asset bundles**: New exemplar bundle directory with `widget.toml` and `indicator.svg` serving as a reference for discrete-binding widget authoring.
- **Test fixtures**: New JSON test message files for status-indicator-specific scenarios (enum cycling, label updates).
- **User-test workflow**: Status-indicator-specific test scenario added to the existing cross-machine validation workflow.
- **Implementer guidance**: Concrete rendering targets and acceptance criteria for the widget compositor's discrete binding path (enum lookup table, immediate snap, no interpolation).
- **No runtime code changes**: This exemplar defines the target — implementation is driven by the widget-system tasks (bundle loading, discrete binding resolution, compositor rasterization).
