## 1. Component Profile

- [ ] 1.1 Create exemplar profile directory at a suitable location (e.g., `assets/profiles/exemplar-subtitle/`) with `profile.toml` declaring `name = "exemplar-subtitle"`, `version = "1.0.0"`, `component_type = "subtitle"`, and `description = "Production-quality subtitle exemplar — the flagship zone reference"`
- [ ] 1.2 Create `zones/subtitle.toml` in the profile directory with all rendering override fields referencing canonical tokens: `text_color = "{{color.text.primary}}"`, `outline_color = "{{color.outline.default}}"`, `outline_width = "{{stroke.outline.width}}"`, `backdrop_color = "{{color.backdrop.default}}"`, `backdrop_opacity = "{{opacity.backdrop.default}}"`, `font_family = "{{typography.subtitle.family}}"`, `font_size_px = "{{typography.subtitle.size}}"`, `font_weight = "{{typography.subtitle.weight}}"`, `text_align = "center"`, `transition_in_ms = 200`, `transition_out_ms = 150`, `margin_vertical = "{{spacing.padding.medium}}"`
- [ ] 1.3 Add the exemplar profile path to the default configuration's `[component_profile_bundles]` paths array so it is discovered at startup
- [ ] 1.4 Set `[component_profiles]` `subtitle = "exemplar-subtitle"` in the default configuration to activate the profile
- [ ] 1.5 Verify profile loads at startup: run the runtime and confirm no `PROFILE_READABILITY_VIOLATION`, `PROFILE_UNKNOWN_COMPONENT_TYPE`, or `PROFILE_UNRESOLVED_TOKEN` errors in logs

## 2. MCP Test Fixtures

- [ ] 2.1 Create `subtitle-single-line.json` in `.claude/skills/user-test/scripts/`: single publish `{"zone_name": "subtitle", "content": "Hello world — exemplar subtitle test", "ttl_us": 10000000, "namespace": "exemplar-test"}`
- [ ] 2.2 Create `subtitle-multiline.json`: publish with long text that forces word-wrap — `"This is a much longer subtitle message designed to test word wrapping behavior across multiple lines. The compositor should wrap this text cleanly within the zone bounds and truncate with ellipsis if it exceeds the vertical space available."`, `ttl_us: 10000000`
- [ ] 2.3 Create `subtitle-rapid-replace.json`: three publishes in sequence — `"First subtitle — should be replaced immediately"`, `"Second subtitle — also replaced"`, `"Third subtitle — this one stays"` — each with `ttl_us: 5000000`, intended for `--delay-ms 100`
- [ ] 2.4 Create `subtitle-ttl-expiry.json`: single publish with short TTL — `"This subtitle expires in 3 seconds"`, `ttl_us: 3000000`
- [ ] 2.5 Create `subtitle-streaming.json`: stream-text publish with breakpoints — `{"zone_name": "subtitle", "content": "The quick brown fox jumps over the lazy dog", "breakpoints": [3, 9, 15, 19, 25, 30, 34, 38], "ttl_us": 10000000, "namespace": "exemplar-test"}`
- [ ] 2.6 Create `subtitle-full-sequence.json`: all scenarios combined in order with comments indicating each group — single line, multi-line, rapid replacement (x3), TTL expiry, streaming. Each with appropriate `ttl_us` for the scenario.

## 3. User-Test Integration

- [ ] 3.1 Add a subtitle-specific test scenario section to the user-test skill documentation (`.claude/skills/user-test/SKILL.md`) describing how to use the subtitle fixtures with human acceptance criteria: (1) white text with black outline on semi-transparent backdrop, (2) centered at bottom of screen, (3) multi-line wraps cleanly, (4) rapid replacement has no flicker, (5) content fades out after TTL, (6) streaming reveals word-by-word
- [ ] 3.2 Verify `subtitle-full-sequence.json` works with `publish_zone_batch.py` by running a dry parse: load the JSON and confirm all messages have valid `zone_name`, `content`, and `namespace` fields
- [ ] 3.3 Add the `subtitle-full-sequence.json` as a named test group in the user-test skill so it can be invoked alongside the existing `all-zones-test.json`

## 4. Validation

- [ ] 4.1 Run the runtime with the exemplar-subtitle profile active and publish `subtitle-single-line.json` via MCP — confirm the subtitle renders with white-on-black-outline text, semi-transparent backdrop, centered at bottom
- [ ] 4.2 Publish `subtitle-rapid-replace.json` with `--delay-ms 100` — confirm only the third message remains visible, no flicker between transitions
- [ ] 4.3 Publish `subtitle-ttl-expiry.json` — confirm the subtitle disappears after 3 seconds with a visible fade-out
- [ ] 4.4 Publish `subtitle-multiline.json` — confirm word-wrap renders correctly and backdrop sizes to contain all visible text
- [ ] 4.5 Publish `subtitle-streaming.json` — confirm the text appears (word-by-word reveal if gRPC, full text if MCP publish)
- [ ] 4.6 Run the full `/user-test` workflow deploying to the Windows target and publishing `subtitle-full-sequence.json` — verify all six human acceptance criteria pass
