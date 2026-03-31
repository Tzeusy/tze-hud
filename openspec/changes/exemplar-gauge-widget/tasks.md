## 1. Asset Bundle: widget.toml Manifest

- [ ] 1.1 Create production gauge widget bundle directory (location TBD — either `assets/widgets/gauge/` or alongside existing bundles)
- [ ] 1.2 Write `widget.toml` manifest with full parameter schema: `level` (f32, 0.0-1.0), `label` (string), `fill_color` (color, default [0,180,255,255]), `severity` (enum: info/warning/error)
- [ ] 1.3 Declare two layers in order: `background.svg` (zero bindings), `fill.svg` (four bindings)
- [ ] 1.4 Configure fill layer bindings: `level` -> `bar.height` (linear, 0.0-200.0), `fill_color` -> `bar.fill` (direct), `label` -> `label-text.text-content` (direct), `severity` -> `indicator.fill` (discrete with value_map matching canonical `color.severity.*` fallbacks)

## 2. Asset Bundle: background.svg

- [ ] 2.1 Create `background.svg` with 100x240 viewBox, outer frame `<rect>` using `fill="{{color.backdrop.default}}"`, `stroke="{{color.border.default}}"`, `data-role="backdrop"`
- [ ] 2.2 Add inner track `<rect id="track">` with a slightly lighter fill (e.g., `{{color.backdrop.default}}` with reduced opacity or a differentiated token) — no `data-role` attribute
- [ ] 2.3 Verify zero hardcoded hex colors in the SVG markup — all colors use `{{token.key}}` placeholders

## 3. Asset Bundle: fill.svg

- [ ] 3.1 Create `fill.svg` with 100x240 viewBox, `<clipPath id="track-clip">` constraining fill bar to the track area (x=30, y=10, width=40, height=200)
- [ ] 3.2 Add `<rect id="bar">` fill bar with `clip-path="url(#track-clip)"`, positioned at track bottom (y=210), height=0 (default), `fill="{{color.text.accent}}"` — grows upward via linear binding
- [ ] 3.3 Add `<text id="label-text">` with `fill="{{color.text.primary}}"`, `data-role="text"`, centered below the track, `font-family="sans-serif"`, `font-size="10"`
- [ ] 3.4 Add `<circle id="indicator">` severity indicator with `fill="{{color.severity.info}}"`, positioned top-right of the gauge
- [ ] 3.5 Verify zero hardcoded hex colors — all colors use `{{token.key}}` placeholders
- [ ] 3.6 Verify `data-role="text"` on label-text, no `data-role` on bar or indicator

## 4. Bundle Validation

- [ ] 4.1 Load the exemplar bundle through the widget bundle loader and verify it passes manifest parsing, SVG parse validation, token placeholder resolution, and binding resolution with zero errors
- [ ] 4.2 Verify token resolution produces correct concrete values with default canonical tokens (backdrop=#000000, border=#333333, accent=#4A9EFF, text=#FFFFFF, severity.info=#4A9EFF)
- [ ] 4.3 Verify the bundle registers as Widget Type `"gauge"` with four parameters, two layers, four bindings

## 5. Parameter Validation Tests

- [ ] 5.1 Test `level` clamping: publish `level=1.5` -> clamped to 1.0; publish `level=-0.5` -> clamped to 0.0
- [ ] 5.2 Test `level` NaN/infinity rejection: publish `level=NaN` -> `WIDGET_PARAMETER_INVALID_VALUE`
- [ ] 5.3 Test unknown parameter rejection: publish `{"temperature": 72}` -> `WIDGET_UNKNOWN_PARAMETER`
- [ ] 5.4 Test `severity` enum validation: publish `severity="critical"` -> `WIDGET_PARAMETER_INVALID_VALUE`
- [ ] 5.5 Test default values at startup: `level=0.0, label="", fill_color=[0,180,255,255], severity="info"`

## 6. Binding and Rendering Tests

- [ ] 6.1 Test linear binding: publish `level=0.5` -> bar height=100.0; `level=0.0` -> height=0.0; `level=1.0` -> height=200.0
- [ ] 6.2 Test direct color binding: publish `fill_color=[255,0,0,255]` -> bar fill becomes red
- [ ] 6.3 Test text-content binding: publish `label="CPU Load"` -> label-text content = "CPU Load"
- [ ] 6.4 Test discrete binding: publish `severity="warning"` -> indicator fill = "#FFB800"; `severity="error"` -> "#FF4444"

## 7. Interpolation Tests

- [ ] 7.1 Test f32 interpolation: publish `level=0.8` with `transition_ms=300` from `level=0.2` -> midpoint at 150ms is approximately 0.5
- [ ] 7.2 Test color interpolation: publish `fill_color=[255,0,0,255]` with `transition_ms=300` from `[0,0,255,255]` -> midpoint at 150ms is approximately `[128,0,128,255]`
- [ ] 7.3 Test string snap: publish `label="Memory"` with `transition_ms=500` -> label changes immediately at t=0
- [ ] 7.4 Test enum snap: publish `severity="error"` with `transition_ms=500` -> indicator changes immediately at t=0
- [ ] 7.5 Test `transition_ms=0` applies all parameters in next frame with single re-rasterize

## 8. Contention Tests

- [ ] 8.1 Test LatestWins: Agent A publishes `level=0.3`, Agent B publishes `level=0.7` -> gauge renders 0.7
- [ ] 8.2 Test parameter retention: Agent A publishes `level=0.5, label="CPU"`, Agent B publishes `level=0.8` -> gauge renders level=0.8, label="CPU"

## 9. Performance Validation

- [ ] 9.1 Benchmark re-rasterization at 512x512 after a `level` parameter change — must complete in < 2ms
- [ ] 9.2 Verify unchanged parameters reuse cached GPU texture (no re-rasterization triggered)

## 10. MCP Integration Test Fixtures

- [ ] 10.1 Create MCP test fixture: set level to 0.75 via `publish_to_widget`, verify fill bar at 75%
- [ ] 10.2 Create MCP test fixture: animate level from 0.0 to 0.9 with `transition_ms=300`, verify smooth animation
- [ ] 10.3 Create MCP test fixture: set all four parameters in one publish call, verify all render correctly
- [ ] 10.4 Create MCP test fixture: change severity through info -> warning -> error sequence, verify indicator color changes

## 11. User-Test Integration

- [ ] 11.1 Create user-test script for the five-step gauge publish sequence: (1) initial state level=0.0/info, (2) animate to 0.75/300ms, (3) change to warning, (4) animate to 0.95/300ms, (5) change to error + label="CRITICAL"
- [ ] 11.2 Verify user-test renders correctly on target display with token-driven colors matching operator configuration
