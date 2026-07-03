# Tasks: portal-bottom-chat-composer

## 1. Spec delta (this change's deliverable)

- [x] 1.1 Author delta: Multi-Line Composer Wrap and Growth, Composer Submit-Key Contract, Pilot-Path Viewer History, Transcript Turn Separators
- [ ] 1.2 `openspec validate portal-bottom-chat-composer --strict` passes
- [ ] 1.3 Commit + push to main

## 2. Implementation (beads under hud-nx7yq — file, then implement)

- [ ] 2.1 File bead: composer multi-line wrap + bounded upward growth + internal vertical scroll (runtime draft state + compositor composer box; interacts with #987 composer_input_strip and #983 caret-follow single-line fallback)
- [ ] 2.2 File bead: submit-key routing — Enter submits, Ctrl+Enter/Shift+Enter newline, empty-draft no-op (runtime keyboard path)
- [ ] 2.3 File bead: pilot-path viewer history (route exemplar/raw-tile submissions through projection-authority echo, or equivalent kind-distinct append; prefer authority routing per design decision 3)
- [ ] 2.4 File bead: token-styled turn separators between transcript entries (compositor + tokens; minimal slice, attribution stays promotion-scoped)
- [ ] 2.5 Implement + merge the four beads (TDD, CI green each)

## 3. Closeout

- [ ] 3.1 Live re-verify on reference Windows overlay (wrap, growth, Ctrl+Enter, Enter-send, history bubbling, separators)
- [ ] 3.2 Annotate the superseded decision in `docs/reports/text-stream-refinement.md`
- [ ] 3.3 Sync + archive per hud-hpuzp convention once implementation lands
