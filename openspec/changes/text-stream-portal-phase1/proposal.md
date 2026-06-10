## Why

The text stream portal is the project's chosen exemplar UX flow: the one interaction the owner wants polished end-to-end to a "works extremely well" bar before breadth work resumes. Phase 0 is complete and reconciled — epic `hud-t98e` shipped the raw-tile pilot, gen-2 reconciliation (PR #441) confirmed 13/13 normative requirement coverage in `openspec/specs/text-stream-portals/spec.md`, and live Windows evidence has accumulated across multiple runs: the 2026-04-27 four-tile exemplar validation (`docs/reports/hud-eq1m4-text-stream-portal-windows-validation-20260427.md`), the 2026-04-28 composer/caret evidence passes (`test_results/text-stream-portal-hud-0ojis-*-20260428.json`, checklist rows in `docs/exemplar-manual-review-checklist.md`), the 2026-05-09 rerun, the 2026-05-10 diagnostic-input validation (`hud-ozbwh`), and the 2026-05-11 live cooperative-projection portal transcript (`docs/evidence/external-agent-projection-authority/live-portal-transcript-20260511T125016Z.json`).

That evidence also shows where the pilot stops short of excellent:

- Markdown is approximated, not rendered. The compositor's `strip_markdown_v1` treats `#`-headed lines as bold and `*...*` as italic; everything else is stripped or shown literally, even though the scene contract names `TextMarkdownNode` content as CommonMark.
- Ellipsis overflow is approximated by visible-line-count truncation with an appended `…`, which can clip glyphs and shift layout under streaming append.
- Composer editing is adapter-echoed: every keystroke round-trips through the adapter before the draft updates, which caps editing feel at adapter latency rather than the runtime's local-feedback budget.
- Streaming behavior is proven for correctness (ordering, coalescing coherence) but has no normative cadence bar tied to the engineering-bar budgets.
- Portal visuals are raw-tile colors chosen ad hoc by the exemplar adapter, outside the design-token / component-profile system that governs every other visual surface.

RFC 0013 §7 deliberately gated anything beyond the raw-tile pilot behind evidence and a separate approval. This change defines Phase 1: the normative "works extremely well" bar for the exemplar flow, and the evidence-backed promotion gate the RFC requires. **This proposal explicitly seeks the RFC 0013 §7.2 promotion approval**, conditioned on the refreshed live-exemplar evidence plan defined in the delta spec's promotion requirements. Until that gate passes, all Phase-1 work remains expressible on the raw-tile pilot.

## What Changes

- Define a normative CommonMark rendering subset for portal text (headings, bold/italic, inline code, code blocks, lists, links-as-styled-text), styled through design tokens, with tables, images, and raw HTML explicitly excluded.
- Require markdown parsing to happen outside the per-frame stage pipeline: parse on content commit, cache styled runs, never re-parse on the render path.
- Replace the approximate line-count truncation with a normative overflow contract: word-boundary ellipsis with the ellipsis glyph included in measurement, no partially clipped glyphs, and layout stability under streaming append.
- Extend bounded one-shot reply submission into bounded, local-first composer draft editing: runtime-owned draft buffer with local echo, caret movement, selection, word-wise delete, and size-capped paste. No terminal passthrough, no IME composition (stays v1-reserved), no general-purpose rich-text editor.
- Define sustained streaming cadence behavior under representative LLM token-rate workloads: burst absorption, coalescing fairness across concurrent portals, and smoothness bounded by the locked Windows frame and latency budgets in `about/craft-and-care/engineering-bar.md`. *(Amended 2026-06-10:)* live cadence evidence additionally measures a transport RTT baseline and per-append publish-to-present timestamps, so runtime-added overhead is reported explicitly — end-to-end latency is RTT plus bounded runtime overhead, evidenced rather than inferred.
- *(Added by 2026-06-10 amendment)* Add viewer window management for expanded portals: pointer-driven resize affordances on the portal frame, focus-scoped resize hotkeys (Ctrl+`+`/`-`), token-defined min/max clamping within lease and scene budgets, local-first gesture feedback the adapter cannot override mid-gesture, and token-styled geometry-only scroll-position indicators.
- Move portal visual identity (frame, header, composer, transcript, divider) onto design tokens and a swappable component profile, including token-driven collapsed/expanded transitions.
- Add a promotion decision gate: the evidence (refreshed live exemplar runs across at least two adapter families) that justifies promoting from raw-tile assembly to a first-class portal surface, and the explicit boundary of what promotion does and does not change. *(Amended 2026-06-10:)* the gate additionally requires an agent-ergonomics demonstration — an LLM session driving the full portal lifecycle exclusively through the vendored skill surface, with zero scene-graph ceremony in its context.

## What Does Not Change

- All 13 existing Phase-0 requirements remain in force. Phase-1 requirements are additive or strictly-extending; none weakens the transport-agnostic boundary, content-layer placement, bounded viewport, coalescing coherence, governance, attention, or adapter-isolation contracts.
- Portal traffic stays on the existing primary bidirectional session stream. No portal-specific transport RPC.
- The portal remains a governed presence surface, subordinate to leases, redaction, safe mode, freeze, dismiss, and the attention model.

## Non-Goals

Carried over from RFC 0013 §1.2/§4.4 and kept prominent — Phase 1 does **not** admit:

- terminal emulation: no VT100/xterm compatibility, no ANSI cursor addressing, no alternate screen, no PTY hosting,
- full transcript history materialized in the scene graph (bounded viewport stands; retained history stays adapter-side),
- chrome-layer portal UI or shell-owned portal affordances,
- a dedicated portal transport or second long-lived portal stream outside the primary session stream,
- runtime ownership of external process lifecycles (tmux, LLM CLIs, projection daemons),
- IME-complete composition (remains v1-reserved per the input-model spec),
- markdown tables, images, raw HTML, or link navigation.

## Capabilities

### Modified Capabilities

- `text-stream-portals`: add Phase-1 markdown fidelity, overflow correctness, local-first composer draft editing, sustained streaming cadence, component-profile styling, and the RFC-0013 promotion evidence gate; extend the existing low-latency interaction and transcript interaction requirements with normative budgets and draft-editing semantics.

## Approval Ask

RFC 0013 §7.2 states that promotion beyond the raw-tile pilot "requires separate approval and does not happen automatically." This change is that approval request. Acceptance of this change approves:

1. implementing the Phase-1 requirements on the existing raw-tile pilot (no new node types required), and
2. promoting to a first-class portal surface **only after** the Promotion Evidence Gate requirement in this change's delta spec is satisfied by refreshed live exemplar runs, recorded under `docs/evidence/text-stream-portals/`.

Promotion, when its gate passes, permits a dedicated portal surface or node type and a portal component-type contract. It does not relax any non-goal above.

## Impact

- Affected specs:
  - `openspec/specs/text-stream-portals/spec.md` (this change's delta)
  - `openspec/specs/component-shape-language/spec.md` (follow-up delta at promotion time: portal component type contract and new canonical token keys; not modified by this change)
- Affected code (implementation phase):
  - `crates/tze_hud_compositor/src/text.rs` (markdown styled runs, overflow/ellipsis measurement)
  - `crates/tze_hud_compositor/src/renderer.rs`
  - `crates/tze_hud_scene/src/types.rs` (styled-run cache types if needed; no new node types pre-promotion)
  - `crates/tze_hud_runtime/src/windowed.rs` (composer draft buffer, paste cap enforcement)
  - `crates/tze_hud_runtime/` input/focus routing for draft editing events
  - `.claude/skills/user-test/scripts/text_stream_portal_exemplar.py` (new live phases: markdown, overflow, composer-edit, cadence, profile-swap)
  - `tests/integration/text_stream_portal_*.rs` (new coverage per requirement)
- Affected docs:
  - `about/legends-and-lore/rfcs/0013-text-stream-portals.md` gains an amendment note recording the Phase-1 contract and the answered §8 open question 1 (draft editing model) after acceptance
  - `docs/exemplar-manual-review-checklist.md` gains Phase-1 sign-off rows
- Risk: composer draft editing is the largest scope step — it introduces a bounded runtime-owned editing primitive. The delta spec bounds it tightly (plain text, byte-capped, no IME, no undo contract, no terminal passthrough) to avoid the "general inline editor" drift RFC 0013 §4.3 warns about.
