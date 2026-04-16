## 1. Asset Bundle

- [ ] 1.1 Create exemplar bundle directory at `assets/widget_bundles/progress-bar/`
- [ ] 1.2 Create `widget.toml` manifest declaring `name = "progress-bar"`, `version = "1.0.0"`, `description = "Horizontal progress bar with configurable fill and label"`, `default_contention_policy = "LatestWins"`, parameter_schema with three parameters (progress: f32 min=0.0 max=1.0 default=0.0, label: string string_max_bytes=32 default="", fill_color: color default=[74,158,255,255]), layers array referencing track.svg (no bindings) and fill.svg (three bindings: linear progress->fill-bar.width attr_min=0 attr_max=296, direct fill_color->fill-bar.fill, text-content label->label-text.text-content). Note: no `default_geometry` in manifest; the loader assigns 100% relative geometry. Per-instance geometry is set via `geometry_override` in `[[tabs.widgets]]` config.
- [ ] 1.3 Create `track.svg` with `viewBox="0 0 300 20"` containing a single `<rect id="track-bg" x="0" y="0" width="300" height="20" rx="10" ry="10" fill="{{token.color.backdrop.default}}" fill-opacity="0.3"/>` — background track with rounded end-caps, token-driven color at 30% opacity
- [ ] 1.4 Create `fill.svg` with `viewBox="0 0 300 20"` containing: (a) `<rect id="fill-bar" x="2" y="2" width="0" height="16" rx="8" ry="8" fill="{{token.color.text.accent}}"/>` — dynamic fill bar with token-driven default color, 2px inset from track edges, (b) `<text id="label-text" x="150" y="10" text-anchor="middle" dominant-baseline="central" font-size="11" fill="{{token.color.text.primary}}"></text>` — centered label text with token-driven primary color
- [ ] 1.5 Add the bundle directory path to the default configuration's `[widget_bundle_paths]` array so the runtime discovers it at startup
- [ ] 1.6 Verify bundle loads at startup: run the runtime and confirm no `WIDGET_BUNDLE_*` errors in logs, `list_widgets` MCP tool returns "progress-bar" with correct parameter schema

## 2. Widget Instance Configuration

- [ ] 2.1 Add a `[[tabs.widgets]]` entry in the default configuration declaring an instance of type "progress-bar" on a suitable tab (e.g., "dashboard" or the default tab) with geometry `{ x = 10, y = 400, width = 300, height = 20 }` and `layer_attachment = "Content"`
- [ ] 2.2 Verify the widget instance appears in SceneSnapshot at session establishment via `list_widgets` output

## 3. MCP Test Fixtures

- [ ] 3.1 Create `progress-bar-step.json` in `.claude/skills/user-test/scripts/` containing the 7-step publish_to_widget sequence: (1) progress=0.0/label="" transition_ms=0, (2) progress=0.25/label="25%" transition_ms=200, (3) progress=0.5/label="50%" transition_ms=200, (4) progress=0.75/label="75%" transition_ms=200, (5) progress=1.0/label="100%" transition_ms=200, (6) fill_color=[0,200,83,255] transition_ms=300, (7) progress=0.0/label="" transition_ms=0. Each entry must have `widget_name="progress-bar"`.
- [ ] 3.2 Create `progress-bar-color-sweep.json`: publish sequence cycling fill_color through red [255,0,0,255], amber [255,184,0,255], green [0,200,83,255], and back to token default [74,158,255,255] — each with transition_ms=300 and progress=0.75/label="75%"
- [ ] 3.3 Verify all fixture JSON files parse correctly and contain valid `publish_to_widget` payloads by dry-loading them

## 4. User-Test Integration

- [ ] 4.1 Add a progress-bar-widget test scenario section to the user-test skill documentation (`.claude/skills/user-test/SKILL.md`) describing how to run the progress bar fixtures with human acceptance criteria: (a) thin horizontal bar with pill/capsule shape, (b) fill animates smoothly between steps, (c) label text centered and legible on the bar, (d) fill color matches token accent or overridden color, (e) reset returns bar to empty with no visual artifacts
- [ ] 4.2 Add `progress-bar-step.json` as a named test group in the user-test skill so it can be invoked with the existing cross-machine workflow

## 5. Validation

- [ ] 5.1 Run the runtime with the progress-bar bundle loaded and publish `progress-bar-step.json` via MCP — confirm the bar fills smoothly from 0% to 100% with correct labels at each step
- [ ] 5.2 Publish `progress-bar-color-sweep.json` — confirm the fill color transitions smoothly between red, amber, green, and accent blue
- [ ] 5.3 Verify re-rasterization performance: confirm parameter updates at 300x20 complete within the 2ms budget (no frame drops or visible jank during 200ms transitions)
- [ ] 5.4 Verify token override: change `color.text.accent` in `[design_tokens]` to a different color, restart, confirm the progress bar fill uses the overridden color
- [ ] 5.5 Run the full `/user-test` workflow deploying to the target machine and publishing `progress-bar-step.json` — verify all five human acceptance criteria pass
