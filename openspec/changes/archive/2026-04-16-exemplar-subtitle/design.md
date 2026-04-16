## Context

The component-shape-language change defines the design token system, RenderingPolicy extensions, component profiles, and readability enforcement that give tze_hud a coherent visual identity. The subtitle zone is the most-used zone type: every transcription agent, narration system, and conversational flow publishes through it. Yet there is no reference that says "this is what a correctly rendered subtitle looks like" and "these are the exact MCP calls that exercise every subtitle behavior."

The component-shape-language epic is **fully implemented** — design tokens, extended RenderingPolicy fields, component profiles, SVG `{{token.key}}` placeholders, zone readability enforcement (DualLayer for subtitle), token-driven zone rendering, and 8-direction text outline are all operational. This means the exemplar can ship as a concrete **component profile** (`profile.toml` + `zones/subtitle.toml`) with token-referenced rendering overrides, plus test fixtures that validate the rendering pipeline end-to-end. It plugs into the existing `/user-test` cross-machine validation workflow.

### Current State

- The subtitle zone type is defined in configuration with `contention_policy: latest_wins`, `accepted_media_types: [stream_text]`, and a basic geometry policy (bottom ~5-10% of screen, centered).
- The compositor renders subtitle stream-text with hardcoded colors (dark blue-gray backdrop, white text, no outline). There is no token indirection, no fade transitions, no configurable opacity.
- The component-shape-language implementation is complete: token-derived rendering policies, 8-direction outline, backdrop opacity, fade transitions are all wired into the compositor. But no exemplar validates the subtitle pipeline end-to-end or provides reference test fixtures.
- The `/user-test` skill has a single subtitle entry in `all-zones-test.json` that publishes a short string — it does not exercise multi-line, rapid replacement, streaming, or TTL expiry.

### Constraints

- This exemplar references existing specs; it does not redefine rendering behavior. The component-shape-language spec is authoritative for how tokens flow into RenderingPolicy fields.
- Test fixtures must be compatible with the existing `publish_zone_batch.py` script (JSON array of `{zone_name, content, ttl_us, namespace}` objects).
- The user-test workflow deploys over SSH/SCP to a Windows target and publishes via MCP HTTP. Fixtures must work within that constraint (no gRPC-only features).
- Streaming word-by-word reveal with breakpoints is a gRPC stream feature. The MCP `publish_to_zone` call sends stream-text as a single payload with breakpoint indices; the runtime handles progressive reveal. The MCP test can verify the initial publish but not the per-word timing.

## Goals / Non-Goals

**Goals:**
- Define the exact visual properties of a correctly rendered subtitle (token values, RenderingPolicy field values, outline technique, backdrop treatment, transitions).
- Define behavioral scenarios covering all subtitle interactions: single-line publish, multi-line overflow, rapid replacement, TTL auto-clear, streaming with breakpoints.
- Provide MCP test fixtures (JSON) that exercise each scenario via `publish_to_zone`.
- Define a user-test scenario that validates subtitle rendering end-to-end across machines.

**Non-Goals:**
- Implementing any rendering code (that is component-shape-language task scope).
- Defining new zone types or modifying the subtitle zone type schema.
- Testing gRPC streaming internals (MCP can verify publish; per-word timing is a unit test concern).
- Custom font or theme variations (the exemplar uses default tokens; profiles are component-shape-language scope).

## Decisions

### 1. Exemplar ships as a component profile referencing canonical tokens

**Decision:** The subtitle exemplar ships as a component profile directory (`exemplar-subtitle/`) with `profile.toml` and `zones/subtitle.toml`. The zone rendering override file references canonical tokens via `{{token.key}}` syntax rather than hardcoding values. This means the exemplar adapts to operator token overrides while providing production-quality defaults.

The effective rendering for default tokens:
- `text_color` = `#FFFFFF` (via `{{color.text.primary}}`)
- `outline_color` = `#000000` (via `{{color.outline.default}}`)
- `outline_width` = `2.0` (via `{{stroke.outline.width}}`)
- `backdrop_color` = `#000000` (via `{{color.backdrop.default}}`)
- `backdrop_opacity` = `0.6` (via `{{opacity.backdrop.default}}`)
- `font_family` = SystemSansSerif (via `{{typography.subtitle.family}}`)
- `font_size_px` = `28.0` (via `{{typography.subtitle.size}}`)
- `font_weight` = `600` (via `{{typography.subtitle.weight}}`)
- `text_align` = `"center"` (literal, not token-driven)
- `transition_in_ms` = `200` (literal)
- `transition_out_ms` = `150` (literal)

**Rationale:** Since component-shape-language is fully implemented, the exemplar should exercise the profile system rather than just the default path. A profile with token references validates both the profile loader and the token resolution pipeline. Operators who override `color.text.primary` in `[design_tokens]` get a subtitle that automatically matches their color scheme.

### 2. Test fixtures are JSON arrays compatible with publish_zone_batch.py

**Decision:** All test fixtures are JSON files in the same format as `all-zones-test.json` — arrays of `{zone_name, content, ttl_us, namespace}` objects. Each fixture file exercises one behavioral scenario. A master fixture chains all scenarios with inter-message delays.

**Rationale:** Reusing the existing batch publish infrastructure avoids new tooling. The delay between messages (for rapid replacement testing) is handled by the `--delay-ms` argument to `publish_zone_batch.py`.

**Note:** The streaming fixture extends the base publish schema with an optional `breakpoints` field (array of u32 byte offsets). The `publish_zone_batch.py` script MUST be updated to forward this field to the MCP `publish_to_zone` call when present.

### 3. Transition defaults are exemplar-recommended, not token-driven

**Decision:** `transition_in_ms` (200) and `transition_out_ms` (150) are not in the canonical token schema. They are RenderingPolicy field defaults that the exemplar recommends for the subtitle zone type definition in configuration. These values are set in the zone type config, not derived from tokens.

**Rationale:** Transitions are behavioral, not visual-identity. They belong in zone type configuration, not the design token system. The exemplar specifies recommended values; operators can override them in their zone type config.

### 4. Streaming breakpoints tested via single MCP publish with breakpoint indices

**Decision:** The MCP `publish_to_zone` call for stream-text sends the full text and breakpoint indices in one payload: `{"content": "The quick brown fox jumps over the lazy dog", "breakpoints": [3, 9, 15, 19, 25, 30, 34, 38]}`. The runtime's stream-text handler reveals words progressively at the compositor's frame rate. The MCP test verifies the publish succeeds and the final rendered text is correct; per-word reveal timing is validated by compositor unit tests.

**Rationale:** MCP is a transactional publish. The streaming reveal is a compositor-side rendering behavior. Testing the full word-by-word timing requires compositor frame inspection, not MCP round-trips.

## Risks / Trade-offs

- **[Risk] Exemplar defines transition defaults that the zone type config doesn't support yet.** The current zone type config schema may not have `transition_in_ms`/`transition_out_ms` fields. -> Mitigation: The component-shape-language tasks add these fields to RenderingPolicy. The exemplar documents the expected values; implementation tasks wire them in.

- **[Risk] Rapid replacement test depends on timing.** Publishing two subtitles in rapid succession (<100ms) tests "no flicker" — but verification is visual. -> Mitigation: The user-test scenario is human-observed. Automated verification uses compositor snapshot tests (existing pattern in the codebase).

- **[Risk] Multi-line overflow behavior depends on glyphon word-wrap + compositor ellipsis logic.** If glyphon doesn't support overflow ellipsis natively, the compositor must implement it. -> Mitigation: The spec scenario defines the expected behavior; the component-shape-language tasks include TextItem overflow handling.
