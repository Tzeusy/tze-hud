# Tasks — Text Stream Portal Phase-1

No implementation begins until this change is reviewed and accepted. Acceptance approves Phase-1 implementation on the raw-tile pilot immediately; promotion work (section 7) additionally waits on the Promotion Evidence Gate passing.

## 1. Contract and review

- [x] 1.1 Validate this OpenSpec change with `openspec validate text-stream-portal-phase1 --strict`
  <!-- Validated on acceptance (#662); `openspec validate text-stream-portal-phase1 --strict` passes as of 2026-06-13 -->
- [x] 1.2 Review doctrine alignment against `about/heart-and-soul/v1.md`, `vision.md`, and CLAUDE.md core rules (frame loop, arrival vs presentation, local feedback first, modular visual identity, four message classes)
  <!-- Doctrine review completed during acceptance of #662; exclusion list confirmed in proposal/design -->
- [x] 1.3 Confirm the final scope keeps terminal emulation, scene-graph transcript history, chrome portal UI, dedicated portal transport, and runtime process ownership excluded
  <!-- Scope exclusions held in proposal.md/design.md; confirmed and held in all implementation PRs -->
- [x] 1.4 After acceptance, add an RFC 0013 amendment note recording the Phase-1 contract and the resolution of §8 open question 1 (in-surface editable draft model, bounded)
  <!-- `docs: amend RFC 0013 with Phase-1 contract note [hud-5jbra.1]` (direct merge) -->
- [x] 1.5 Confirm the bounded draft-editing primitive is recorded as a deliberate, bounded extension relative to the v1 interactive-primitive scope in `about/heart-and-soul/v1.md`
  <!-- `docs(v1): amend touch posture, macOS lane criterion, introspection scope; acknowledge text-stream portals [hud-w0jfp.1]` (#712) -->

## 2. Markdown rendering subset

- [x] 2.1 Implement subset parsing (headings, strong/emphasis, inline code, code blocks, lists, links-as-styled-text) into a cached styled-run representation keyed by content identity, parsed at content commit outside the per-frame pipeline
  <!-- `feat: Phase-1 markdown subset parse-on-commit cache [hud-5jbra.2]` (#664) — BLAKE3-keyed MarkdownCache, parse_markdown_subset(), StyledSpan; `perf: move/instrument markdown priming off the render-frame hot path [hud-gpqde]` (#681); `perf: move markdown priming off the render thread (commit-time prime + atomic swap) [hud-380dl]` (#696) -->
- [x] 2.2 Replace `strip_markdown_v1` consumption in the portal text path with styled-run rendering; excluded constructs (tables, images, raw HTML, blockquotes, footnotes, strikethrough, task lists, autolinks) render as literal source text
  <!-- `fix: replace lossy strip_markdown_v1 cache-miss fallback with observable guard / non-lossy prime [hud-xcp9b]` (#700); `fix(compositor): preserve color_runs when markdown cache path is taken [hud-bylxu]` (#673); `fix(compositor): wire heading_scale, code_background, bold_weight tokens; preserve ordered-list ordinals [hud-f8jb0]` (#731) -->
- [x] 2.3 Resolve all subset styling from design tokens at startup; add profile-scoped token overrides for any missing keys (code background, link treatment) without touching canonical keys pre-promotion
  <!-- `feat: consume full portal part-token inventory + production token plumbing [hud-1uh1l]` (#680); `fix: portal token fallback diagnostic + single-source default palette [hud-dcynv]` (#701); `fix(compositor): wire heading_scale, code_background, bold_weight tokens [hud-f8jb0]` (#731) -->
- [x] 2.4 Verify: integration tests for each subset construct, each excluded construct degrading to literal text, link non-navigability, and node-budget compliance for markdown-heavy windows
  <!-- `feat: Phase-1 markdown subset parse-on-commit cache [hud-5jbra.2]` (#664) — 452-line integration test suite (tests/integration/text_stream_portal_markdown.rs) covering all subset constructs, excluded-construct literal fallback, and link treatment -->
- [x] 2.5 Verify: headless benchmark proving zero per-frame parse cost for unchanged content and stage-budget compliance (Stages 3–5 < 1 ms each) when a 65535-byte payload commits mid-stream
  <!-- `perf: move/instrument markdown priming off the render-frame hot path [hud-gpqde]` (#681); `perf: move markdown priming off the render thread (commit-time prime + atomic swap) [hud-380dl]` (#696); `perf: TextItem::text -> Arc<str> to avoid per-frame plain_text clone [hud-yvufj]` (#699) — zero per-frame parse cost enforced by commit-time prime + atomic swap; stage-budget benchmarks included -->

## 3. Overflow and ellipsis correctness

- [x] 3.1 Implement measured word-boundary ellipsis truncation with the ellipsis glyph included in shaped-width measurement, with grapheme-cluster fallback for unbroken tokens
  <!-- `feat: Phase-1 overflow and ellipsis contract [hud-5jbra.3]` (#665); `fix: grapheme fallback is reachable for long unbroken tokens [hud-wq6qp]` (#674); `perf: make truncate_line_to_ellipsis sub-quadratic [hud-alq0x]` (#677); `fix: derive logical run offsets for RTL ellipsis truncation [hud-u7nyn]` (#676) -->
- [x] 3.2 Enforce whole-line vertical visibility (no partially clipped glyph rows) and whole-line follow-tail advancement
  <!-- `feat: follow-tail scroll-anchor + tail-anchored overflow for streaming transcripts [hud-pvoc1]` (#678); `feat: wire follow-tail/tail-anchored overflow into the render path [hud-5d4km]` (#683) -->
- [x] 3.3 Implement append stability for scrolled-back viewports: appends beyond the viewport cause no reflow or truncation-point change in visible lines
  <!-- `feat: follow-tail scroll-anchor + tail-anchored overflow for streaming transcripts [hud-pvoc1]` (#678) -->
- [x] 3.4 Verify: integration tests for word-boundary truncation, grapheme fallback, no-clipped-glyph invariants (property-based across random content/widths), and scrolled-back append stability
  <!-- `test: property-based tests for truncate_tail_anchored [hud-347b4]` (#682); `fix: tail-anchored overflow component defects (line-count, resume-to-AtTail, head-trim anchor) [hud-o55ye]` (#684); `test(compositor): add would-fail-pre-fix regression tests for #708 (a)/(b) via extracted call-site functions [hud-dhb4s]` (#745); `fix: add is_finite() guards to truncate_for_ellipsis [hud-xuimm]` (#697) -->
- [x] 3.5 Verify: layout-resolve stage stays < 1 ms with styled-run caching under transcript-sized content
  <!-- `perf: make find_paren_close and find_backtick_close sub-quadratic [hud-xq0uo]` (#685); `perf: make truncate_line_to_ellipsis sub-quadratic [hud-alq0x]` (#677); `perf: move/instrument markdown priming off the render-frame hot path [hud-gpqde]` (#681); `perf: move markdown priming off the render thread [hud-380dl]` (#696) — sub-quadratic paths + commit-time caching enforce the budget; perf-assert CI lane added (#720) -->

## 4. Composer draft editing

- [x] 4.1 Implement the runtime-owned bounded plain-text draft buffer attached to focused composer regions, with local rendering of text, caret, and selection within the input-to-local-ack budget
  <!-- `feat: runtime-owned composer draft buffer with coalesced adapter notifications [hud-5jbra.4]` (#667); `feat: composer draft rendering — token-driven draft/caret/at-capacity visual [hud-2zyt9]` (#689); `feat: composer echo local render — draft/caret/at-capacity without adapter round-trip [hud-r3ax6]` (#707) -->
- [x] 4.2 Implement editing operations: caret movement (character/word/line), keyboard and pointer selection, backspace/delete (character and word-wise), paste with UTF-8-boundary truncation at the cap and visible at-capacity feedback
  <!-- `feat: wire ComposerDraft into runtime input routing + coalesced delivery with flush guarantee [hud-qwqxy]` (#686); `feat: wire ComposerDraftManager into the runtime input loop [hud-odxjl]` (#687); `fix(hud-083az): consume focused-composer multiline paste; wire pointer selection` (#733); `feat: runtime clipboard-injection API for composer draft (inject_composer_paste MCP tool) [hud-k1uun]` (#748); `fix(compositor): safe char-boundary caret in composer echo [hud-le2fd]` (#722) -->
- [x] 4.3 Implement coalescible draft-state notifications (state-stream) and transactional submission/cancel delivering exactly the local buffer at submit time
  <!-- `feat: wire ComposerDraft into runtime input routing + coalesced delivery with flush guarantee [hud-qwqxy]` (#686); `feat: deliver composer DraftNotificationBatch to the adapter proto bridge [hud-ygbcy]` (#688) -->
- [x] 4.4 Enforce exclusions: no IME composition, no undo/redo, no rich text, no multi-caret, no interpretation of editing keystrokes as terminal/provider input
  <!-- Enforced in `feat: runtime-owned composer draft buffer [hud-5jbra.4]` (#667) and `feat: wire ComposerDraftManager [hud-odxjl]` (#687) — plain-text-only buffer, no IME path wired -->
- [x] 4.5 Wire governance: draft suspends under safe mode with chrome input capture; draft content obeys portal redaction policy
  <!-- `feat: suspend composer draft input on safe-mode enter/exit [hud-8k2ah]` (#695); `feat: portal shortcut precedence + safe-mode capture + gesture-authority enforcement [hud-38236]` (#692) -->
- [x] 4.6 Update the cooperative projection adapter and exemplar adapter to consume draft-state notifications instead of per-keystroke republish of composer text nodes
  <!-- `refactor(exemplar): migrate text-stream portal exemplar off adapter-owned echo [hud-0iq62]` (#735) -->
- [x] 4.7 Verify: integration tests for local echo independence from adapter latency, word-wise delete, coalesced notifications, oversized-paste truncation and non-forwarding, submit-content fidelity, safe-mode suspension, and keystroke non-passthrough across both adapter families
  <!-- `fix(compositor): safe char-boundary caret in composer echo; retarget tests at production code [hud-le2fd]` (#722); `test(hud-28j7v): inject clock into drain_inner; add regression guard for notify_tile_content_appended wiring` (#729); `fix(hud-083az): consume focused-composer multiline paste; wire pointer selection` (#733); `fix(security): window-mgmt hardening — safe-mode fail-closed test [hud-an467]` (#739) -->
- [ ] 4.8 Verify: live exemplar `composer-edit` phase on the reference Windows host (extends the 2026-04-28 composer/caret evidence) meeting the input-to-local-ack Windows lane budget
  <!-- Pending: requires live Windows exemplar run -->

## 5. Sustained streaming cadence

- [x] 5.1 Implement work-conserving coalescing with cross-portal fairness (no unbounded divergence between equal-rate portals)
  <!-- `feat: portal cadence coalescing with cross-portal fairness [hud-5jbra.5]` (#668) -->
- [x] 5.2 Add a cadence harness generating the normative workloads (sustained ≥ 200 scalars/s in ≥ 10 increments/s for ≥ 60 s; bursts ≥ 4096 bytes in 250 ms) against headless and live targets
  <!-- `feat: wire PortalCadenceCoalescer into streaming presentation path + arrival-to-present measurement [hud-zmt1a]` (#679); `feat: production portal driver — host adapter + drain cadence coalescer into present path [hud-6rkc8]` (#690); `fix(cadence): correct stale-sequence on coalesce-key in-place path; add wait-hint API [hud-endkj]` (#725) -->
- [x] 5.3 Verify: headless benchmark holding frame budgets (`high_mutation` p99 ≤ 8.3 ms / p99.9 ≤ 16.6 ms Windows lane), input budgets under concurrent typing/scroll, per-stage budgets, and the 1000 events/s aggregate ceiling during sustained streams and bursts
  <!-- `ci+test: add perf-assert CI lane; gate all straggler wall-clock asserts [hud-94vm5]` (#720); perf-assert CI lane enforces frame-budget and stage-budget assertions on every merge -->
- [x] 5.4 Verify: dual-portal fairness test under equal sustained rates; retained-window coherence under burst per the existing coalescing requirement
  <!-- `feat: portal cadence coalescing with cross-portal fairness [hud-5jbra.5]` (#668) — dual-portal fairness included in coalescer tests -->
- [ ] 5.5 Verify: 60-minute streaming soak within the ≤ 5 MiB memory-drift budget, recorded with the reference hardware tag
  <!-- Pending: 60-minute soak not yet run on reference hardware -->
- [ ] 5.6 Verify: live exemplar `cadence` phase on the reference Windows host with reference-tagged artifacts
  <!-- Pending: live exemplar not yet run -->
- [ ] 5.7 *(amendment)* Verify: live cadence phase records a transport RTT baseline and per-append publish-to-present timestamps; evidence artifact reports runtime-added overhead separately from RTT, within the `high_mutation` input-to-next-present budget for presented appends
  <!-- Pending: arrival-to-present measurement instrumentation landed (#679), but live evidence not yet recorded -->

## 6. Portal component profile styling

- [x] 6.1 Expose the runtime's resolved token set to the exemplar adapter publish path and remove all literal visual values from exemplar portal publishes
  <!-- `feat: portal component profile styling with token-driven adapter [hud-5jbra.6]` (#669); `feat: consume full portal part-token inventory + production token plumbing [hud-1uh1l]` (#680); `fix: portal token fallback diagnostic + single-source default palette [hud-dcynv]` (#701) -->
- [x] 6.2 Define the portal part inventory (frame, header, composer, transcript body, divider, collapsed card) and the token mapping each part consumes
  <!-- `feat: consume full portal part-token inventory + production token plumbing [hud-1uh1l]` (#680) -->
- [x] 6.3 Implement collapsed/expanded transitions on existing zone-transition mechanics with token-derived treatment, redaction-safe at every frame
  <!-- `feat: portal window management — resize affordances, hotkeys, geometry snapshots, scroll indicators [hud-5jbra.9]` (#672) — geometry snapshots and collapsed card transitions via zone-transition mechanics; `feat: wire portal resize/affordance/scroll-indicator into the compositor + Ctrl-gated hotkeys [hud-2ps6p]` (#691) -->
- [x] 6.4 Verify: integration tests for profile-swap reskin without adapter logic changes, token-propagation on republish, and no-redacted-flash during transitions under a restricted viewer
  <!-- `fix(security): window-mgmt hardening — safe-mode fail-closed test, token-resolved PortalWindowTokens [hud-an467]` (#739); `test(compositor): add at-capacity token distinctness + override propagation test [hud-2axdq]` (#737) -->
- [ ] 6.5 Verify: live exemplar `profile-swap` phase demonstrating an operator-visible reskin on the reference Windows host
  <!-- Pending: live exemplar not yet run -->

## 6b. Window management (amendment 2026-06-10)

- [x] 6b.1 Implement pointer-driven resize affordances on the portal frame (corner/edge capture regions, content layer) with local-first geometry feedback during the gesture
  <!-- `feat: portal window management — resize affordances, hotkeys, geometry snapshots, scroll indicators [hud-5jbra.9]` (#672); `feat: wire portal resize/affordance/scroll-indicator into the compositor + Ctrl-gated hotkeys [hud-2ps6p]` (#691); `feat: wire pointer-affordance portal resize into windowed.rs [hud-o0st9]` (#705); `feat: mid-drag re-truncation under the overflow contract [hud-ghhxa]` (#694) -->
- [x] 6b.2 Implement focus-scoped resize hotkeys (Ctrl+`+`/Ctrl+`=` grow, Ctrl+`-` shrink) with token-defined step; unfocused portals never consume them; chrome/shell-reserved shortcuts and safe-mode capture take precedence
  <!-- `feat: wire portal resize/affordance/scroll-indicator into the compositor + Ctrl-gated hotkeys [hud-2ps6p]` (#691); `feat: portal shortcut precedence + safe-mode capture + gesture-authority enforcement [hud-38236]` (#692) -->
- [x] 6b.3 Implement min/max clamping (token-defined legible minimum; lease-bounds and scene-budget maximum) and pane re-layout under the overflow contract at every intermediate geometry
  <!-- `feat: wire portal resize/affordance/scroll-indicator into the compositor + Ctrl-gated hotkeys [hud-2ps6p]` (#691); `feat: mid-drag re-truncation under the overflow contract [hud-ghhxa]` (#694) -->
- [x] 6b.4 Deliver geometry changes to the owning adapter as coalescible state-stream snapshots; gesture remains authoritative over adapter publishes until gesture end
  <!-- `feat(6b.4): wire resize geometry into ProjectionAuthority producer [hud-npq6g]` (#730) -->
- [x] 6b.5 Implement token-styled, geometry-only scroll-position indicators for overflowing transcript/composer panes, redaction-safe
  <!-- `feat: wire portal resize/affordance/scroll-indicator into the compositor + Ctrl-gated hotkeys [hud-2ps6p]` (#691) -->
- [x] 6b.6 Verify: integration tests for local-first resize, focus-scoped hotkey routing (focused/unfocused), bounds clamping without clipped glyphs, mid-gesture adapter-override rejection, and indicator presence under redaction
  <!-- `fix(security): window-mgmt hardening — safe-mode fail-closed test, token-resolved PortalWindowTokens, comment fix [hud-an467]` (#739) -->
- [ ] 6b.7 Verify: live exemplar `window-mgmt` phase (pointer resize + hotkey resize via OS input injection, following the `diagnostic-input` pattern) on the reference Windows host
  <!-- Pending: live exemplar not yet run -->

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
