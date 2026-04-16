## 1. Asset Bundle

- [ ] 1.1 Create the `status-indicator/` bundle directory under the widget assets path (alongside the existing gauge fixture)
- [ ] 1.2 Create `widget.toml` manifest with name="status-indicator", version="1.0.0", parameter_schema (status: enum with enum_allowed_values=["online","away","busy","offline"] default="offline"; label: string with string_max_bytes=16 default=""), single layer referencing `indicator.svg`, discrete binding on `indicator-fill` fill attribute with value_map (online="#00CC66", away="#FFB800", busy="#FF4444", offline="#666666"), direct text-content binding on `label-text`, default_contention_policy="LatestWins". Note: no `default_geometry` in manifest; the loader assigns 100% relative geometry. Per-instance geometry (e.g., 48x48) is set via `geometry_override` in `[[tabs.widgets]]` config.
- [ ] 1.3 Create `indicator.svg` with viewBox="0 0 48 48": circle element (id="indicator-fill", cx=24, cy=20, r=14, fill="#666666", stroke="{{token.color.border.default}}", stroke-width="1.5"), text element (id="label-text", x=24, y=44, text-anchor="middle", font-size="10", fill="{{token.color.text.secondary}}", empty default content). No scripts, no external refs, no animation.
- [ ] 1.4 Verify bundle loads: add the status-indicator bundle to the test fixture scan path and confirm the widget bundle loader registers it without error (widget type "status-indicator" with 2 parameters and 1 layer with 2 bindings)

## 2. Discrete Binding Verification

- [ ] 2.1 Write a unit test: load the status-indicator bundle, publish status="online" and assert the `indicator-fill` element's fill attribute is "#00CC66"
- [ ] 2.2 Write a unit test: publish status="away" and assert fill is "#FFB800"
- [ ] 2.3 Write a unit test: publish status="busy" and assert fill is "#FF4444"
- [ ] 2.4 Write a unit test: publish status="offline" and assert fill is "#666666"
- [ ] 2.5 Write a unit test: publish status="do-not-disturb" (not in enum_allowed_values) and assert WIDGET_PARAMETER_INVALID_VALUE rejection
- [ ] 2.6 Write a unit test: publish status="online" with transition_ms=500, then status="busy" — assert the fill snaps to #FF4444 with no interpolation frame (discrete enum binding ignores transition_ms)

## 3. Text-Content Binding Verification

- [ ] 3.1 Write a unit test: publish label="Butler" and assert the `label-text` element's text content is "Butler"
- [ ] 3.2 Write a unit test: publish label="" and assert the `label-text` element's text content is empty
- [ ] 3.3 Write a unit test: publish a label exceeding string_max_bytes=16 and assert WIDGET_PARAMETER_INVALID_VALUE rejection

## 4. Token Placeholder Resolution Verification

- [ ] 4.1 Write a unit test: load the bundle with default tokens, assert the circle's stroke attribute resolved to "#333333" (color.border.default fallback)
- [ ] 4.2 Write a unit test: load the bundle with default tokens, assert the label's fill attribute resolved to "#B0B0B0" (color.text.secondary fallback)
- [ ] 4.3 Write a unit test: load the bundle with a custom design_tokens override `color.border.default = "#FF0000"`, assert the circle's stroke resolved to "#FF0000"

## 5. Contention and Partial Update Verification

- [ ] 5.1 Write a unit test: Agent A publishes status="online" label="A", Agent B publishes status="busy" label="B" — assert occupancy reflects B's values (LatestWins)
- [ ] 5.2 Write a unit test: publish status="online" label="Butler", then publish only status="away" — assert occupancy is status="away" label="Butler" (partial update retains unpublished parameters)

## 6. MCP Test Fixtures

- [ ] 6.1 Create `enum-cycle-test.json` fixture: four sequential `publish_to_widget` calls cycling status through "online", "away", "busy", "offline" with 1-second delays
- [ ] 6.2 Create `label-update-test.json` fixture: three sequential publishes setting label to "Butler", "Codex", "" with status="online"
- [ ] 6.3 Create `validation-test.json` fixture: publish with status="do-not-disturb" expecting WIDGET_PARAMETER_INVALID_VALUE error response

## 7. User-Test Integration

- [ ] 7.1 Add a status-indicator entry to the user-test widget test sequence: deploy the bundle, cycle through all four enum values with labels, require human visual confirmation of color changes
- [ ] 7.2 Document the expected visual for each status value in the user-test fixture comments (online=green dot, away=yellow dot, busy=red dot, offline=gray dot, with configurable label text below)

## 8. Performance Verification

- [ ] 8.1 Write a benchmark or assertion: re-rasterize the 48x48 indicator SVG after a status parameter change and assert completion under 2ms
