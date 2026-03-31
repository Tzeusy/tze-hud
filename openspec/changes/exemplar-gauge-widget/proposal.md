## Why

The widget system spec defines a complete SVG asset bundle pipeline — parameterized visual templates with four binding types, design token integration, smooth interpolation, and contention governance — but there is no production-quality reference widget that exercises the full pipeline end-to-end. The existing gauge test fixture (`crates/tze_hud_widget/tests/fixtures/gauge/`) validates bundle loading mechanics but uses hardcoded colors, lacks `{{token.key}}` placeholders, omits `data-role` readability attributes, and defines no interpolation or behavioral contract. Without a canonical exemplar, implementers have no concrete target for the compositor's SVG rendering pipeline, and testers have no fixture to validate that design tokens, parameter interpolation, and contention resolution work together correctly.

A polished gauge widget exemplar — the reference parameterized visual — proves the full pipeline: token-resolved SVG layers, all four parameter binding types (linear, direct, discrete, text-content), smooth `transition_ms` interpolation for f32 and color parameters, immediate snap for string and enum parameters, and `LatestWins` contention by default. This is the widget system's equivalent of the subtitle exemplar for zones.

## What Changes

- Define the **exemplar-gauge-widget** specification: a production-quality vertical fill gauge widget bundle that exercises every widget system capability in a single, polished visual
- Specify the **visual contract**: background track layer (static frame with token-resolved colors), fill layer (dynamic bar + label text + severity indicator, all with `{{token.key}}` placeholders and `data-role` attributes)
- Specify **four parameter binding types** in one widget: `level` (f32, linear binding to fill height), `label` (string, text-content binding), `fill_color` (color, direct binding to fill), `severity` (enum, discrete binding to indicator color via `color.severity.*` tokens)
- Specify **interpolation behavior**: `transition_ms=300` default for level/fill_color changes (linear f32 interpolation, component-wise sRGB color interpolation), immediate snap for label/severity changes
- Specify **parameter validation**: level clamped to [0.0, 1.0], unknown params rejected with `WIDGET_UNKNOWN_PARAMETER`, NaN/infinity rejected
- Specify **contention policy**: `LatestWins` by default (most recent publication wins)
- Specify **performance budget**: re-rasterization < 2ms for 512x512 on reference hardware
- Define **MCP test scenarios**: `publish_to_widget` calls exercising all four parameter types, animated transitions, severity changes
- Define **user-test integration**: agent publishes gauge parameters via MCP, verify visual renders correctly on the cross-machine validation workflow

## Capabilities

### New Capabilities
- `exemplar-gauge-widget`: Production-quality gauge widget exemplar defining the visual contract, behavioral scenarios, asset bundle specification, MCP test fixtures, and user-test integration — the flagship widget exemplar proving the full SVG asset bundle pipeline with design tokens

### Modified Capabilities

## Impact

- **Asset bundle**: New production-quality gauge widget bundle directory with `widget.toml` manifest, `background.svg`, and `fill.svg` — replaces the minimal test fixture as the reference gauge implementation
- **Test fixtures**: The existing `crates/tze_hud_widget/tests/fixtures/gauge/` remains as-is for unit test purposes; the exemplar defines the production target that exercises token placeholders and readability conventions the fixture does not
- **User-test workflow**: Gauge-specific test scenario added to the cross-machine validation skill
- **No new Rust code changes**: This exemplar defines the target specification — the widget asset bundle loader, compositor SVG renderer, and MCP `publish_to_widget` tool are defined by the widget-system and component-shape-language changes
