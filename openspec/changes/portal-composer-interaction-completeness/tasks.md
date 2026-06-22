# Tasks — Portal Composer Interaction Completeness

This change is **spec ahead of code**: the two deltas describe interaction behavior the
composer does not yet implement. The change stays OPEN until the implementation lands and is
verified, then it is synced + archived. Acceptance of this change authorizes the implementation
beads below.

## 1. Contract and review

- [x] 1.1 Validate this change with `openspec validate portal-composer-interaction-completeness --strict`
- [ ] 1.2 Confirm doctrine alignment: "one scene model, two profiles" (pointer-less Mobile Presence Node must be operable), "local feedback first", and focus reuse of runtime-owned rules (CLAUDE.md, RFC 0004, RFC 0013 §4.3)
- [ ] 1.3 Confirm the deltas add no new transport/input plane and do not change focus scoping, latency budgets, draft bounds, or IME status (v1-reserved)

## 2. Implement — pointer-independent focus (`hud-v0cal`)

- [ ] 2.1 Wire a keyboard focus-traversal path (focus-advance/retreat key + token-defined focus chord) into the windowed key-down path BEFORE agent dispatch, routing to the runtime focus manager to focus the composer hit region. Coordinate with the promotion epic (`hud-g1ena`, composer render-path ownership).
- [ ] 2.2 Keep focus scoping intact: traversal respects chrome/shell precedence and safe-mode capture; an unfocused portal still does not consume composer keystrokes.
- [ ] 2.3 Test: composer becomes focusable and editable with no pointer events (glasses/no-pointer path).

## 3. Implement — horizontal caret-follow (`hud-zlfi4`)

- [ ] 3.1 Compute a per-composer horizontal scroll offset at render time so the active caret cluster stays within `[text_margin, width - text_margin]`; shift draft `pixel_x` and the selection-run x by the same offset. Coordinate with the promotion epic (renderer ownership).
- [ ] 3.2 The offset is local presentation state and obeys the same redaction/safe-mode/focus rules; no adapter round trip.
- [ ] 3.3 Test: caret stays visible as the draft grows past the box width, and when the caret moves back left.

## 4. Already-covered compliance (`hud-hxhnt`) — no delta

- [ ] 4.1 Pointer caret-placement (real x→byte hit-test, not a linear byte-fraction guess) and pointer-drag range-selection satisfy the EXISTING "Local-First Composer Draft Editing" requirement ("selection (keyboard and pointer)"). Track as a compliance fix under `hud-hxhnt`; this change does not respec it.

## 5. Reconcile and close

- [ ] 5.1 After 2.x/3.x land and are verified, sync the deltas to `openspec/specs/text-stream-portals/spec.md` and archive (`openspec archive portal-composer-interaction-completeness`)
- [ ] 5.2 Close `hud-v0cal` and `hud-zlfi4` on merge of their implementations
