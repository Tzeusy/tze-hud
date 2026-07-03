# text-stream-portals Delta: bottom-chat composer + visible viewer history

## ADDED Requirements

### Requirement: Multi-Line Composer Wrap and Growth

When a portal composer's draft exceeds the visible composer width, the composer SHALL wrap the draft to multiple lines within the composer width rather than extending a single line horizontally. The composer box SHALL grow upward to accommodate wrapped lines up to a token-bounded maximum line count, with the transcript pane yielding the space; past the maximum, the composer SHALL scroll vertically within its bounded box, keeping the active caret visible. The composer SHALL remain pinned to the bottom edge of the portal surface. Wrap, growth, caret, and selection rendering are local-first from runtime-owned draft state per the Local-First Composer Draft Editing contract; growth and shrink of the composer box MUST NOT require an adapter round trip. Single-line horizontal caret-follow remains the defined behavior only for a composer profile whose bounded height is one line. This requirement supersedes the "No bottom-chat-style input" decision in `docs/reports/text-stream-refinement.md` (owner direction 2026-07-03).

Source: owner live direction 2026-07-03 (hud-nx7yq), `docs/reports/text-stream-refinement.md` §input (superseded), RFC 0013 §4.3
Scope: v1

#### Scenario: long draft wraps instead of scrolling horizontally

- **WHEN** the viewer types a draft wider than the composer box in a multi-line composer profile
- **THEN** the draft SHALL wrap to the next line within the composer width
- **AND** the earlier text SHALL remain visible rather than sliding out of view to the left

#### Scenario: composer grows upward bounded

- **WHEN** wrapped draft lines exceed the composer's current height
- **THEN** the composer box SHALL grow upward up to the token-defined maximum line count, the transcript pane yielding the space locally
- **AND** past the maximum the composer SHALL scroll vertically keeping the caret visible

#### Scenario: shrinking the draft shrinks the composer

- **WHEN** the viewer deletes draft content such that fewer wrapped lines are needed
- **THEN** the composer box SHALL shrink back toward its single-line height and the transcript pane SHALL reclaim the space

### Requirement: Composer Submit-Key Contract

While a portal composer holds focus, Enter SHALL submit the draft as a bounded transactional reply, and Ctrl+Enter and Shift+Enter SHALL insert a newline into the draft at the caret. Newline insertion is a local draft edit under the Local-First Composer Draft Editing contract. The submit and newline keys are focus-scoped to the composer: chrome- and shell-reserved shortcuts take precedence and safe-mode capture overrides them. An empty or whitespace-only draft SHALL NOT submit.

Source: owner live direction 2026-07-03 (hud-nx7yq), §Low-Latency Text Interaction (viewer submit is transactional)
Scope: v1

#### Scenario: enter submits, ctrl+enter inserts newline

- **WHEN** the viewer presses Ctrl+Enter (or Shift+Enter) in a focused composer with a non-empty draft
- **THEN** a newline SHALL be inserted at the caret with local-first echo and no submission SHALL occur
- **AND** a subsequent plain Enter SHALL submit the full multi-line draft transactionally

#### Scenario: empty draft does not submit

- **WHEN** the viewer presses Enter in a focused composer whose draft is empty or whitespace-only
- **THEN** no submission SHALL occur and the composer SHALL remain focused

### Requirement: Pilot-Path Viewer History

An accepted composer submission SHALL become visible as a viewer-authored entry in the portal's retained transcript on EVERY portal surface path, including the Phase-0 raw-tile pilot — the viewer's submitted words MUST NOT silently disappear from the surface on submit. Paths that use the projection authority satisfy this via the existing Viewer Reply Echo requirement; the raw-tile pilot SHALL either route submissions through the same echo mechanism or append an equivalent kind-distinct viewer entry to its visible transcript window at submit time. All Viewer Reply Echo constraints apply unchanged: runtime-authored at submit time, kind-distinct, never counted as unread, never escalating attention, redacting like transcript content, and absent for rejected submissions.

Source: §Viewer Reply Echo, owner live observation 2026-07-03 ("whenever I press Enter my text disappears"), `crates/tze_hud_projection/src/authority.rs` (append_viewer_echo)
Scope: v1

#### Scenario: pilot portal shows the submitted reply

- **WHEN** the viewer submits a reply on a raw-tile pilot portal and the submission is accepted
- **THEN** the submitted text SHALL appear in the visible transcript as a viewer-authored entry without waiting for the adapter to echo it back

#### Scenario: rejected submission still not echoed on the pilot

- **WHEN** a pilot-path submission is rejected
- **THEN** no viewer entry SHALL be appended and the existing rejection feedback applies

### Requirement: Transcript Turn Separators

Adjacent transcript entries SHALL be visually separated by a subtle, token-styled divider or border so the history reads as discrete conversational turns. Separator treatment (line, spacing, or bordered grouping) resolves from design tokens via the portal's component profile and MUST NOT be hardcoded in the compositor. Separators are geometry-only presentation: they SHALL remain present under redaction without revealing content, and they SHALL NOT raise interruption class. Viewer-authored entries remain kind-distinct per the Viewer Reply Echo requirement; full per-turn attribution and the multi-node turn model remain governed by the promotion-era transcript presentation (portal-chat-grade-affordances / hud-g1ena) — this requirement mandates only the minimal visible separation between entries.

Source: owner live direction 2026-07-03 ("a nice mini border between entries"), `openspec/changes/portal-chat-grade-affordances/` (turn model, promotion-scoped)
Scope: v1

#### Scenario: entries render with token-styled separation

- **WHEN** the transcript contains consecutive entries
- **THEN** a token-styled separator SHALL render between adjacent entries
- **AND** the separator's treatment SHALL resolve from design tokens rather than hardcoded values

#### Scenario: separators persist under redaction

- **WHEN** the portal's transcript is redacted for the current viewer
- **THEN** entry separation MAY remain visible as geometry
- **AND** the separators SHALL NOT reveal entry content or counts beyond what the redaction policy permits

## MODIFIED Requirements

### Requirement: Phase-1 Markdown Rendering Subset

Text stream portal surfaces SHALL render a defined CommonMark subset for transcript content: ATX headings (levels 1–6), strong emphasis, emphasis, inline code, fenced and indented code blocks, ordered and unordered lists (including nesting), links rendered as styled non-navigable text, and thematic breaks rendered as a token-styled horizontal divider. All subset styling (heading scale, emphasis weight/style, code font family and background, list indentation, link treatment, divider color and thickness) MUST resolve from design tokens rather than hardcoded values. The thematic-break divider treatment is shared with the Transcript Turn Separators requirement: the runtime encodes entry boundaries as thematic breaks when lowering the retained transcript, so adapter-authored thematic breaks and runtime-authored entry separators render with the same token-styled divider — an adapter can draw a divider line but MUST NOT thereby gain any additional semantics (a divider is content-free geometry and never implies a viewer turn, attention change, or attribution). Constructs outside the subset — tables, images, raw HTML, blockquotes, footnotes, strikethrough, task lists, and autolinks — MUST NOT be parsed in Phase 1 and SHALL render as literal source text rather than being silently dropped. Link destinations MUST NOT be navigable, fetched, or previewed; only the link text is styled. Rendered markdown content remains subject to the existing bounded-viewport, node-size, and per-tile resource budget rules.

Source: RFC 0001 §TextMarkdownNode, RFC 0013 §3.4, `about/heart-and-soul/vision.md` (visual identity is modular), PR #994 (thematic-break divider, hud-nx7yq.4)

#### Scenario: subset constructs render with token-driven styling

- **WHEN** a portal transcript update contains a heading, bold and italic spans, inline code, a fenced code block, a nested list, and a link
- **THEN** each construct SHALL render with the visual treatment resolved from the active design tokens
- **AND** no construct's color, font, size, or background SHALL come from a hardcoded compositor value

#### Scenario: thematic break renders as a token-styled divider

- **WHEN** a portal transcript contains a thematic break line (for example `---`), whether authored by the adapter or inserted by the runtime as an entry separator
- **THEN** the surface SHALL render a horizontal divider whose color and thickness resolve from the divider design tokens
- **AND** the divider SHALL carry no semantics beyond visual separation

#### Scenario: excluded constructs degrade to literal text

- **WHEN** a portal transcript update contains a markdown table, an image reference, or raw HTML
- **THEN** the surface SHALL render the construct's literal source text
- **AND** no transcript content SHALL be silently dropped

#### Scenario: links are styled text only

- **WHEN** a transcript update contains `[release notes](https://example.com)`
- **THEN** the link text SHALL render with the token-defined link treatment
- **AND** activating, hovering, or rendering the link SHALL NOT trigger navigation, fetching, or preview of the destination

#### Scenario: markdown rendering respects node budgets

- **WHEN** a markdown-heavy transcript window is rendered into scene nodes
- **THEN** the materialized content SHALL stay within the existing text-node size limits and per-tile resource budgets required by the Bounded Transcript Viewport requirement
