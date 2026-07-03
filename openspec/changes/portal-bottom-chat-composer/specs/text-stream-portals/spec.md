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
