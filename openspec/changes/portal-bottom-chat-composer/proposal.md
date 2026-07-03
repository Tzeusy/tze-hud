# Proposal: portal-bottom-chat-composer

## Why

Live operator testing on the reference Windows overlay (2026-07-03, rounds 1–2) produced explicit owner direction that the text-stream portal should read and behave like a chat surface: submitted text must not vanish on Enter but bubble up into the visible history with a subtle separator, the composer must wrap to multiple lines instead of horizontally scrolling a single line off-screen, Ctrl+Enter must insert a newline while Enter sends, and the input belongs pinned at the bottom. This REVERSES the prior design decision in `docs/reports/text-stream-refinement.md` ("No bottom-chat-style input", line ~180) — a reversal our engineering notes flagged as requiring a scoped OpenSpec change before implementation. The building blocks exist but don't reach the live pilot path: §Viewer Reply Echo is specified and implemented in the projection-authority path, yet the raw-tile exemplar session shows nothing on submit; the composer is single-line by contract (§Local-First Composer Draft Editing + the caret-follow scenario).

## What Changes

- **Multi-line composer**: the composer wraps the draft to multiple lines within the composer width, growing upward to a token-bounded height (the transcript pane yields the space); horizontal caret-follow remains only as the degenerate single-line-profile behavior.
- **Submit-key contract**: Enter submits the draft; Ctrl+Enter (and Shift+Enter as the conventional alias) inserts a newline. Both are focus-scoped to the composer.
- **Viewer history on the pilot path**: an accepted submission SHALL appear in the visible transcript as a viewer-authored entry on ALL portal paths, including the Phase-0/raw-tile pilot (either by extending the echo contract to the pilot surface or by the pilot migrating onto the projection-authority echo) — the viewer's words never silently disappear.
- **Turn separators**: adjacent transcript entries are visually separated by a subtle token-styled divider/border; viewer entries are kind-distinct per §Viewer Reply Echo.
- **Supersession note**: `docs/reports/text-stream-refinement.md`'s "No bottom-chat-style input" decision is superseded by owner direction 2026-07-03 (recorded in this change).

## Capabilities

### New Capabilities

(none)

### Modified Capabilities

- `text-stream-portals`: MODIFIES §Local-First Composer Draft Editing (multi-line wrap, bounded growth, submit-key contract) and ADDS a requirement for pilot-path viewer history + turn separators consistent with §Viewer Reply Echo and §Ambient Portal Attention Defaults.

## Impact

- **Spec**: delta on `text-stream-portals`.
- **Code (implementation beads under hud-nx7yq)**: runtime composer draft state + keyboard handling (Enter/Ctrl+Enter routing), compositor composer strip → multi-line box (interacts with #987's `composer_input_strip` and #983's caret-follow), exemplar/pilot echo path (`resident_grpc.rs` or exemplar migration to `portal_projection_*`), separator rendering tokens.
- **Interactions**: turn separators overlap the promotion-era turn model (hud-g1ena / portal-chat-grade-affordances) — this change specifies the minimal pilot-visible separator; full turn attribution stays with promotion.
- **Non-goals**: rich text, IME (v1-reserved), message editing/deletion, threading.
