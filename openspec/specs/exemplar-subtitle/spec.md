# exemplar-subtitle Specification

## Purpose
Defines the subtitle exemplar zone contract for readable bottom-of-screen text, dual-layer readability, backdrop/outline policy, and transition behavior.

## Requirements
### Requirement: Subtitle Visual Contract
The subtitle zone exemplar MUST render with the following visual properties when using default canonical design tokens (no component profile active). These properties constitute the reference rendering target for all subtitle implementations.

**Text rendering:**
- Text color: `#FFFFFF` (from token `color.text.primary`)
- Font family: system sans-serif (from token `typography.subtitle.family` = `"system-ui"`)
- Font size: 28px (from token `typography.subtitle.size`)
- Font weight: 600 (from token `typography.subtitle.weight`)
- Text alignment: centered horizontally within the zone geometry
- Word wrap: enabled (glyphon `Wrap::Word`); text MUST wrap at zone boundary minus horizontal margins
- Overflow: when wrapped text exceeds the zone's vertical bounds, the last visible line MUST be truncated with ellipsis (`...`)

**Text outline (DualLayer readability):**
- Outline color: `#000000` (from token `color.outline.default`)
- Outline width: 2px (from token `stroke.outline.width`)
- Technique: 8-direction multi-pass rendering — text rendered at pixel offsets [(-2,0), (2,0), (0,-2), (0,2), (-2,-2), (2,-2), (-2,2), (2,2)] in outline color, then fill text rendered on top in text color
- Ref: component-shape-language spec, Extended RenderingPolicy requirement, "Text outline rendered via multi-pass" scenario

**Backdrop:**
- Backdrop color: `#000000` (from token `color.backdrop.default`)
- Backdrop opacity: 0.6 (60%) (from token `opacity.backdrop.default`)
- Effective backdrop RGBA: `Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.6 }`
- Backdrop geometry: spans the full zone width minus margins, full zone height minus vertical margins
- Ref: component-shape-language spec, Default Zone Rendering with Tokens requirement, subtitle zone defaults

**Zone geometry:**
- Position: bottom of screen, ~5-10% from bottom edge, centered horizontally
- Vertical margin: 8px (from token `spacing.padding.medium` applied to `margin_vertical`)
- Zone geometry is resolved by the runtime's geometry policy for the subtitle zone type; agents do not specify coordinates
- Ref: about/heart-and-soul/presence.md, "Example zones" section

**Transitions:**
- Fade-in: 200ms opacity ramp (0.0 to 1.0) when content is published
- Fade-out: 150ms opacity ramp (1.0 to 0.0) when content is cleared or replaced
- Ref: component-shape-language spec, Extended RenderingPolicy requirement, `transition_in_ms`/`transition_out_ms` fields

**Readability enforcement:**
- The effective RenderingPolicy MUST pass DualLayer readability validation: backdrop present with opacity >= 0.3, outline present with width >= 1.0
- Ref: component-shape-language spec, Zone Readability Enforcement requirement, DualLayer checks
Scope: v1-mandatory

#### Scenario: Default subtitle renders with token-derived white-on-black-outline text
- **WHEN** the runtime starts with default canonical tokens (no `[design_tokens]` overrides, no component profile active) and an agent publishes `"Hello world"` to the subtitle zone
- **THEN** the compositor MUST render the text in white (`#FFFFFF`) with a 2px black (`#000000`) 8-direction outline over a 60%-opacity black backdrop, using system sans-serif font at 28px, centered horizontally

#### Scenario: Custom token overrides change subtitle appearance
- **WHEN** `[design_tokens]` contains `"color.text.primary" = "#00FF00"` and `"typography.subtitle.size" = "36"` and an agent publishes to the subtitle zone
- **THEN** the subtitle text MUST render in green (`#00FF00`) at 36px font size, with all other properties at their canonical defaults

#### Scenario: DualLayer readability passes for default subtitle
- **WHEN** the runtime constructs the effective RenderingPolicy for the subtitle zone using default canonical tokens
- **THEN** the DualLayer readability check MUST pass: `backdrop` = `Some(Rgba::BLACK)`, `backdrop_opacity` = `Some(0.6)` (>= 0.3), `outline_color` = `Some(Rgba::BLACK)`, `outline_width` = `Some(2.0)` (>= 1.0)

---

### Requirement: Subtitle Contention Policy — Latest Wins
The subtitle zone MUST use `latest_wins` contention policy. When a new publication arrives while an existing subtitle is displayed, the new content MUST replace the old content immediately (after any configured fade-out/fade-in transition). There MUST be no queue, no merge, and no visible gap between consecutive publishes when transitions are configured — the fade-out of the old content and fade-in of the new content MUST overlap or be instantaneous if transitions are zero.
Scope: v1-mandatory

#### Scenario: New subtitle replaces existing subtitle
- **WHEN** an agent publishes `"First message"` to the subtitle zone and then publishes `"Second message"` while the first is still displayed
- **THEN** the subtitle zone MUST display only `"Second message"` — the first message is replaced, not queued

#### Scenario: Rapid replacement produces no flicker
- **WHEN** an agent publishes `"Message A"` to the subtitle zone and publishes `"Message B"` within 50ms
- **THEN** the transition from A to B MUST be visually clean — no frame where neither A nor B is visible (no blank/flicker frame between replacements)

**Note — Transition interrupt semantics:** When a new publish arrives during an active transition_out_ms fade-out, the fade-out MUST be cancelled immediately and the new content MUST begin its transition_in_ms fade-in from the current composite opacity (not from zero). This prevents blank frames during rapid replacement.

#### Scenario: Different agents — latest wins regardless of source
- **WHEN** agent "transcriber" publishes `"transcription text"` and agent "narrator" publishes `"narration text"` 100ms later
- **THEN** the subtitle zone MUST display only `"narration text"` — latest-wins is source-agnostic

---

### Requirement: Subtitle Auto-Clear After TTL
When a subtitle publication includes an `expires_at_wall_us` timestamp (or the zone type defines a default TTL), the subtitle content MUST auto-clear after the TTL expires. Auto-clear MUST trigger the configured fade-out transition before removing content. After the fade-out completes, the zone MUST display nothing (empty occupancy) until the next publication. The exemplar default TTL is 5 seconds (5,000,000 microseconds).
Scope: v1-mandatory

#### Scenario: Subtitle auto-clears after TTL
- **WHEN** an agent publishes `"Temporary message"` with `ttl_us = 5000000` (5 seconds)
- **THEN** the subtitle MUST display for 5 seconds, then fade out over 150ms, then the zone MUST show no content

#### Scenario: New publish resets TTL
- **WHEN** an agent publishes `"First"` with `ttl_us = 5000000` and publishes `"Second"` with `ttl_us = 5000000` after 3 seconds
- **THEN** `"Second"` MUST display for a full 5 seconds from its publish time (not 2 seconds remaining from the first publish)

#### Scenario: No TTL means content persists until replaced
- **WHEN** an agent publishes `"Persistent message"` with no `ttl_us` and no `expires_at_wall_us`
- **THEN** the subtitle MUST remain displayed indefinitely until replaced by another publish or cleared by `ClearZone`

---

### Requirement: Subtitle Streaming Word-by-Word Reveal
The subtitle zone MUST support `stream_text` content with breakpoint indices. Breakpoints identify byte offsets in the text where the compositor MAY pause progressive reveal (typically word boundaries). The compositor MUST reveal text progressively from the first character, pausing briefly at each breakpoint to create a word-by-word reveal effect. The reveal rate MUST be governed by the compositor's frame timing, not by agent publish cadence. Once all text is revealed, the subtitle displays the full text until TTL expiry or replacement.
Scope: v1-mandatory

#### Scenario: Stream-text with breakpoints reveals word-by-word
- **WHEN** an agent publishes stream-text `"The quick brown fox"` with breakpoints at byte offsets `[3, 9, 15]` (after "The", "quick", "brown")
- **THEN** the compositor MUST reveal the text progressively: first "The", then "The quick", then "The quick brown", then "The quick brown fox"

#### Scenario: Stream-text without breakpoints reveals all at once
- **WHEN** an agent publishes stream-text `"Instant display"` with an empty breakpoints array
- **THEN** the compositor MUST display the full text immediately (no progressive reveal)

#### Scenario: Replacement during streaming cancels reveal
- **WHEN** an agent publishes stream-text `"Long streaming message"` with breakpoints and a new publish arrives before reveal completes
- **THEN** the compositor MUST cancel the in-progress reveal and display the new content (latest-wins applies during streaming)

---

### Requirement: Subtitle Multi-Line Overflow Handling
When subtitle text is too long to fit on a single line within the zone geometry, the compositor MUST word-wrap the text. When wrapped text exceeds the zone's vertical bounds (typically 2-3 lines for a 5-10% height zone), the compositor MUST truncate the last visible line with an ellipsis character (`...`) to indicate overflow. The backdrop MUST size to contain all visible text lines (including the truncated line).
Scope: v1-mandatory

#### Scenario: Long text wraps to multiple lines
- **WHEN** an agent publishes `"This is a much longer subtitle that will definitely need to wrap across multiple lines on any reasonable display size"` to the subtitle zone
- **THEN** the compositor MUST word-wrap the text within the zone width and display it across multiple lines, with the backdrop sized to contain all visible lines

#### Scenario: Excessive text truncated with ellipsis
- **WHEN** an agent publishes text that wraps to more lines than the zone's vertical bounds allow
- **THEN** the last visible line MUST end with `...` and any text beyond the zone bounds MUST NOT render

---

### Requirement: Subtitle MCP Test Fixtures
The exemplar MUST define a set of MCP `publish_to_zone` call sequences as JSON fixture files compatible with `publish_zone_batch.py`. Each fixture exercises a specific subtitle behavior. All fixtures MUST use `zone_name: "subtitle"` and `namespace: "exemplar-test"`.

**Required fixture files:**

1. **`subtitle-single-line.json`** — Single short subtitle publish. Verifies basic rendering.
2. **`subtitle-multiline.json`** — Long text that forces word-wrap. Verifies multi-line rendering and backdrop sizing.
3. **`subtitle-rapid-replace.json`** — Three subtitles published in rapid succession (intended for use with `--delay-ms 100`). Verifies latest-wins contention with no flicker.
4. **`subtitle-ttl-expiry.json`** — Subtitle with 3-second TTL. Verifies auto-clear after timeout.
5. **`subtitle-streaming.json`** — Stream-text with breakpoint indices. Verifies word-by-word reveal.
6. **`subtitle-full-sequence.json`** — All scenarios in order with appropriate inter-message delays. Used by the user-test workflow for end-to-end validation.
Scope: v1-mandatory

#### Scenario: Single-line fixture publishes correctly
- **WHEN** `subtitle-single-line.json` is loaded and published via `publish_zone_batch.py`
- **THEN** the MCP call MUST succeed and the subtitle zone MUST render the text with default token-derived styling

#### Scenario: Rapid replacement fixture exercises contention
- **WHEN** `subtitle-rapid-replace.json` is published with `--delay-ms 100`
- **THEN** three subtitles MUST publish in rapid succession and only the last MUST remain visible after all publishes complete

#### Scenario: Full sequence fixture is self-contained
- **WHEN** `subtitle-full-sequence.json` is published via `publish_zone_batch.py` with `--delay-ms 4000` (to allow TTL expiry between groups)
- **THEN** each scenario group MUST execute in order: single line, multi-line, rapid replacement, TTL expiry, streaming

---

### Requirement: Subtitle User-Test Scenario
The exemplar MUST define a user-test scenario compatible with the `/user-test` skill. The scenario MUST deploy the HUD application to the Windows target, publish the subtitle full-sequence fixture via MCP, and provide human-verifiable acceptance criteria for each rendered message. The scenario serves as the acceptance test for subtitle rendering quality.

**Acceptance criteria (human-verified):**
1. White text with visible black outline on semi-transparent dark backdrop
2. Text centered horizontally near bottom of screen
3. Multi-line text wraps cleanly within backdrop bounds
4. Rapid replacement transitions are smooth (no blank frames)
5. Content disappears after TTL (fade-out visible)
6. Streaming text reveals word-by-word (if observable at human time scale)
Scope: v1-mandatory

#### Scenario: User-test publishes subtitle sequence and human verifies
- **WHEN** the `/user-test` skill deploys the HUD and publishes `subtitle-full-sequence.json` via MCP
- **THEN** a human observer MUST be able to verify all six acceptance criteria listed above

#### Scenario: User-test subtitle messages use exemplar namespace
- **WHEN** the subtitle user-test scenario executes
- **THEN** all published messages MUST use `namespace: "exemplar-test"` to distinguish from other zone test traffic
