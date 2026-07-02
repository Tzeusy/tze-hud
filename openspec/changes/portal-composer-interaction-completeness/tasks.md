# Tasks — Portal Composer Interaction Completeness

This change is **spec ahead of code**: the two deltas describe interaction behavior the
composer does not yet implement. The change stays OPEN until the implementation lands and is
verified, then it is synced + archived. Acceptance of this change authorizes the implementation
beads below.

## 1. Contract and review

- [x] 1.1 Validate this change with `openspec validate portal-composer-interaction-completeness --strict`
- [x] 1.2 Confirm doctrine alignment: "one scene model, two profiles" (pointer-less Mobile Presence Node must be operable), "local feedback first", and focus reuse of runtime-owned rules (CLAUDE.md, RFC 0004, RFC 0013 §4.3)
- [x] 1.3 Confirm the deltas add no new transport/input plane and do not change focus scoping, latency budgets, draft bounds, or IME status (v1-reserved)

## 2. Implement — pointer-independent focus (`hud-v0cal`) — DONE (PR #980)

- [x] 2.1 Wire a keyboard focus-traversal path (Tab/Shift+Tab) into `dispatch_key_down_event_inner` BEFORE composer/agent routing, driving the existing `FocusManager::navigate_next`/`navigate_prev`. Landed INPUT-PATH only (no renderer touch) so no promotion coordination was needed; shared `InputProcessor::apply_focus_transition_side_effects` keeps the keyboard and pointer click-to-focus paths in sync.
- [x] 2.2 Focus scoping intact: traversal runs after safe-mode capture + shell-reserved + resize hotkey and before composer/agent; Ctrl/Alt/Meta chords excluded (Win/Cmd+Tab reach the OS); the consumed Tab key-down AND its matching key-up are swallowed so Tab is never raw input.
- [x] 2.3 Test: composer acquires focus + edits with no pointer events (regression test added); existing pointer click-to-focus test still passes after the shared-helper extraction.

## 3. Implement — horizontal caret-follow (`hud-zlfi4`)

- [x] 3.1 Compute a per-composer horizontal scroll offset at render time so the active caret cluster stays within `[text_margin, width - text_margin]`; shift draft `pixel_x` and the selection-run x by the same offset. Coordinate with the promotion epic (renderer ownership). Landed as a render-time recompute: `Compositor::prime_composer_scroll_offset` measures caret x against the composer font (mutable rasterizer) BEFORE `collect_text_items`; `composer_scroll_offset` (pure) clamps it to standard chat-input semantics; `collect_composer_text_item` shifts draft `pixel_x` by `-offset` while pinning the clip to the region interior (selection run is byte-anchored to `pixel_x` so it scrolls with the text). Draft now lays out on one unwrapped line (widened layout width) so overflow scrolls + clips instead of wrapping.
- [x] 3.2 The offset is local presentation state and obeys the same redaction/safe-mode/focus rules; no adapter round trip. The offset lives on the compositor, is recomputed per frame from runtime-owned draft state, and is only ever applied while the composer overlay itself renders (which already carries the redaction/safe-mode/focus gating); no adapter traffic is involved.
- [x] 3.3 Test: caret stays visible as the draft grows past the box width, and when the caret moves back left. Covered by 9 CPU-only unit tests over the pure `composer_scroll_offset` core (fits→no-scroll, typing-past-width, Home→0, End→tail, mid-text-visible, left-sweep-monotonic-reveal, delete→no-dead-space, bounded-offset, degenerate/narrow-box). Full GPU end-to-end rendering remains for the live-verify pass (§5).

## 4. Already-covered compliance (`hud-hxhnt`) — no delta

- [ ] 4.1 Pointer caret-placement (real x→byte hit-test, not a linear byte-fraction guess) and pointer-drag range-selection satisfy the EXISTING "Local-First Composer Draft Editing" requirement ("selection (keyboard and pointer)"). Track as a compliance fix under `hud-hxhnt`; this change does not respec it.

## 5. Reconcile and close

- [ ] 5.1 After 2.x/3.x land and are verified, sync the deltas to `openspec/specs/text-stream-portals/spec.md` and archive (`openspec archive portal-composer-interaction-completeness`)
- [ ] 5.2 Close `hud-v0cal` and `hud-zlfi4` on merge of their implementations
