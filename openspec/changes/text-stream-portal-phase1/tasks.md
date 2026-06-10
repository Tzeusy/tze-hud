# Tasks — Text Stream Portal Phase-1

No implementation begins until this change is reviewed and accepted. Acceptance approves Phase-1 implementation on the raw-tile pilot immediately; promotion work (section 7) additionally waits on the Promotion Evidence Gate passing.

## 1. Contract and review

- [ ] 1.1 Validate this OpenSpec change with `openspec validate text-stream-portal-phase1 --strict`
- [ ] 1.2 Review doctrine alignment against `about/heart-and-soul/v1.md`, `vision.md`, and CLAUDE.md core rules (frame loop, arrival vs presentation, local feedback first, modular visual identity, four message classes)
- [ ] 1.3 Confirm the final scope keeps terminal emulation, scene-graph transcript history, chrome portal UI, dedicated portal transport, and runtime process ownership excluded
- [ ] 1.4 After acceptance, add an RFC 0013 amendment note recording the Phase-1 contract and the resolution of §8 open question 1 (in-surface editable draft model, bounded)
- [ ] 1.5 Confirm the bounded draft-editing primitive is recorded as a deliberate, bounded extension relative to the v1 interactive-primitive scope in `about/heart-and-soul/v1.md`

## 2. Markdown rendering subset

- [ ] 2.1 Implement subset parsing (headings, strong/emphasis, inline code, code blocks, lists, links-as-styled-text) into a cached styled-run representation keyed by content identity, parsed at content commit outside the per-frame pipeline
- [ ] 2.2 Replace `strip_markdown_v1` consumption in the portal text path with styled-run rendering; excluded constructs (tables, images, raw HTML, blockquotes, footnotes, strikethrough, task lists, autolinks) render as literal source text
- [ ] 2.3 Resolve all subset styling from design tokens at startup; add profile-scoped token overrides for any missing keys (code background, link treatment) without touching canonical keys pre-promotion
- [ ] 2.4 Verify: integration tests for each subset construct, each excluded construct degrading to literal text, link non-navigability, and node-budget compliance for markdown-heavy windows
- [ ] 2.5 Verify: headless benchmark proving zero per-frame parse cost for unchanged content and stage-budget compliance (Stages 3–5 < 1 ms each) when a 65535-byte payload commits mid-stream

## 3. Overflow and ellipsis correctness

- [ ] 3.1 Implement measured word-boundary ellipsis truncation with the ellipsis glyph included in shaped-width measurement, with grapheme-cluster fallback for unbroken tokens
- [ ] 3.2 Enforce whole-line vertical visibility (no partially clipped glyph rows) and whole-line follow-tail advancement
- [ ] 3.3 Implement append stability for scrolled-back viewports: appends beyond the viewport cause no reflow or truncation-point change in visible lines
- [ ] 3.4 Verify: integration tests for word-boundary truncation, grapheme fallback, no-clipped-glyph invariants (property-based across random content/widths), and scrolled-back append stability
- [ ] 3.5 Verify: layout-resolve stage stays < 1 ms with styled-run caching under transcript-sized content

## 4. Composer draft editing

- [ ] 4.1 Implement the runtime-owned bounded plain-text draft buffer attached to focused composer regions, with local rendering of text, caret, and selection within the input-to-local-ack budget
- [ ] 4.2 Implement editing operations: caret movement (character/word/line), keyboard and pointer selection, backspace/delete (character and word-wise), paste with UTF-8-boundary truncation at the cap and visible at-capacity feedback
- [ ] 4.3 Implement coalescible draft-state notifications (state-stream) and transactional submission/cancel delivering exactly the local buffer at submit time
- [ ] 4.4 Enforce exclusions: no IME composition, no undo/redo, no rich text, no multi-caret, no interpretation of editing keystrokes as terminal/provider input
- [ ] 4.5 Wire governance: draft suspends under safe mode with chrome input capture; draft content obeys portal redaction policy
- [ ] 4.6 Update the cooperative projection adapter and exemplar adapter to consume draft-state notifications instead of per-keystroke republish of composer text nodes
- [ ] 4.7 Verify: integration tests for local echo independence from adapter latency, word-wise delete, coalesced notifications, oversized-paste truncation and non-forwarding, submit-content fidelity, safe-mode suspension, and keystroke non-passthrough across both adapter families
- [ ] 4.8 Verify: live exemplar `composer-edit` phase on the reference Windows host (extends the 2026-04-28 composer/caret evidence) meeting the input-to-local-ack Windows lane budget

## 5. Sustained streaming cadence

- [ ] 5.1 Implement work-conserving coalescing with cross-portal fairness (no unbounded divergence between equal-rate portals)
- [ ] 5.2 Add a cadence harness generating the normative workloads (sustained ≥ 200 scalars/s in ≥ 10 increments/s for ≥ 60 s; bursts ≥ 4096 bytes in 250 ms) against headless and live targets
- [ ] 5.3 Verify: headless benchmark holding frame budgets (`high_mutation` p99 ≤ 8.3 ms / p99.9 ≤ 16.6 ms Windows lane), input budgets under concurrent typing/scroll, per-stage budgets, and the 1000 events/s aggregate ceiling during sustained streams and bursts
- [ ] 5.4 Verify: dual-portal fairness test under equal sustained rates; retained-window coherence under burst per the existing coalescing requirement
- [ ] 5.5 Verify: 60-minute streaming soak within the ≤ 5 MiB memory-drift budget, recorded with the reference hardware tag
- [ ] 5.6 Verify: live exemplar `cadence` phase on the reference Windows host with reference-tagged artifacts
- [ ] 5.7 *(amendment)* Verify: live cadence phase records a transport RTT baseline and per-append publish-to-present timestamps; evidence artifact reports runtime-added overhead separately from RTT, within the `high_mutation` input-to-next-present budget for presented appends

## 6. Portal component profile styling

- [ ] 6.1 Expose the runtime's resolved token set to the exemplar adapter publish path and remove all literal visual values from exemplar portal publishes
- [ ] 6.2 Define the portal part inventory (frame, header, composer, transcript body, divider, collapsed card) and the token mapping each part consumes
- [ ] 6.3 Implement collapsed/expanded transitions on existing zone-transition mechanics with token-derived treatment, redaction-safe at every frame
- [ ] 6.4 Verify: integration tests for profile-swap reskin without adapter logic changes, token-propagation on republish, and no-redacted-flash during transitions under a restricted viewer
- [ ] 6.5 Verify: live exemplar `profile-swap` phase demonstrating an operator-visible reskin on the reference Windows host

## 6b. Window management (amendment 2026-06-10)

- [ ] 6b.1 Implement pointer-driven resize affordances on the portal frame (corner/edge capture regions, content layer) with local-first geometry feedback during the gesture
- [ ] 6b.2 Implement focus-scoped resize hotkeys (Ctrl+`+`/Ctrl+`=` grow, Ctrl+`-` shrink) with token-defined step; unfocused portals never consume them; chrome/shell-reserved shortcuts and safe-mode capture take precedence
- [ ] 6b.3 Implement min/max clamping (token-defined legible minimum; lease-bounds and scene-budget maximum) and pane re-layout under the overflow contract at every intermediate geometry
- [ ] 6b.4 Deliver geometry changes to the owning adapter as coalescible state-stream snapshots; gesture remains authoritative over adapter publishes until gesture end
- [ ] 6b.5 Implement token-styled, geometry-only scroll-position indicators for overflowing transcript/composer panes, redaction-safe
- [ ] 6b.6 Verify: integration tests for local-first resize, focus-scoped hotkey routing (focused/unfocused), bounds clamping without clipped glyphs, mid-gesture adapter-override rejection, and indicator presence under redaction
- [ ] 6b.7 Verify: live exemplar `window-mgmt` phase (pointer resize + hotkey resize via OS input injection, following the `diagnostic-input` pattern) on the reference Windows host

## 7. Promotion evidence gate and promotion

- [ ] 7.1 Extend `text_stream_portal_exemplar.py` with `markdown`, `overflow`, `composer-edit`, `window-mgmt`, `cadence` (with RTT-overhead reporting), and `profile-swap` phases alongside the existing phases
- [ ] 7.2 Run the full extended exemplar live against the exemplar script adapter and the cooperative projection adapter on the reference Windows host; record reference-tagged artifacts under `docs/evidence/text-stream-portals/`
- [ ] 7.3 Record raw-tile complexity observations (tile counts, mutation batch shapes, workaround inventory) and governance confirmation (redaction, safe mode, freeze, orphan path) in the evidence package
- [ ] 7.3b *(amendment)* Run the agent-ergonomics demonstration: an LLM session drives attach → stream output → poll/acknowledge input → detach exclusively through the vendored skill surface (cooperative projection contract), zero scene-graph mutations authored in the LLM's context; record ceremony metrics (operation count, glue outside the skill) alongside the complexity observations
- [ ] 7.4 Assess the package against every RFC 0013 §7.2 criterion; record the pass/fail decision and rationale in `docs/reports/`
- [ ] 7.5 If the gate passes: author the follow-up component-shape-language delta (`text-portal` component type, canonical token keys) and the first-class portal surface design as separate reviewed work items under this change's promotion approval
- [ ] 7.6 If the gate fails: file beads for the gaps, keep the raw-tile pilot authoritative, and leave Phase-1 behavioral requirements in force on raw tiles
- [ ] 7.7 Update `docs/exemplar-manual-review-checklist.md` with Phase-1 sign-off rows and record manual review outcomes

## 8. Closeout

- [ ] 8.1 Reconcile implementation against every ADDED and MODIFIED requirement in this change before archive/sync
- [ ] 8.2 Create follow-up beads for any deferred items discovered during implementation (token canonicalization, first-class surface schema, adapter migrations)
- [ ] 8.3 Create the epic and child beads mirroring sections 2–7 with dependency edges (verification children depend on implementation children; section 7 depends on sections 2–6; terminal reconciliation depends on all) and verify with `bd show`, `bd dep tree`, and `bd ready`
