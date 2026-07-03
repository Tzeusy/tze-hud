# Tasks: portal-bottom-chat-composer

## 1. Spec delta (this change's deliverable)

- [x] 1.1 Author delta: Multi-Line Composer Wrap and Growth, Composer Submit-Key Contract, Pilot-Path Viewer History, Transcript Turn Separators
- [ ] 1.2 `openspec validate portal-bottom-chat-composer --strict` passes
- [ ] 1.3 Commit + push to main

## 2. Implementation (beads under hud-nx7yq — file, then implement)

- [x] 2.1 File bead: composer multi-line wrap + bounded upward growth + internal vertical scroll (runtime draft state + compositor composer box; interacts with #987 composer_input_strip and #983 caret-follow single-line fallback) — implemented (hud-nx7yq.1): `portal.composer.max_lines` token (default 6) selects profile; multi-line wraps the draft to the box width, grows the box UPWARD to `max_lines` (transcript yields by occlusion, portal outer geometry untouched), then scrolls vertically keeping the caret line visible; delete shrinks back. Single-line profile (`max_lines == 1`) preserves hud-zlfi4 horizontal caret-follow. Pure CPU core `composer_visible_line_count` / `composer_vertical_line_offset` / `composer_input_box` + wrap measurement `measure_composer_wrapped`. Growth is compositor-local (viewer layout), no adapter round trip. Live re-verify (3.1) pending.
- [ ] 2.2 File bead: submit-key routing — Enter submits, Ctrl+Enter/Shift+Enter newline, empty-draft no-op (runtime keyboard path)
- [ ] 2.3 File bead: pilot-path viewer history (route exemplar/raw-tile submissions through projection-authority echo, or equivalent kind-distinct append; prefer authority routing per design decision 3)
- [ ] 2.4 File bead: token-styled turn separators between transcript entries (compositor + tokens; minimal slice, attribution stays promotion-scoped)
- [ ] 2.5 Implement + merge the four beads (TDD, CI green each)

## 3. Closeout

- [ ] 3.1 Live re-verify on reference Windows overlay (wrap, growth, Ctrl+Enter, Enter-send, history bubbling, separators)
- [ ] 3.2 Annotate the superseded decision in `docs/reports/text-stream-refinement.md`
- [ ] 3.3 Sync + archive per hud-hpuzp convention once implementation lands
