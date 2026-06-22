# Tasks — Portal Resize Key-Up Fallback

This change documents already-shipped behavior (PR #967, bead `hud-vznqm`) as a normative
requirement, closing the code-ahead-of-spec gap found by the 2026-06-21 reconciliation sweep
(`hud-y3pho`). Tasks are verification/reconciliation, not new implementation.

## 1. Contract and review

- [x] 1.1 Validate this change with `openspec validate portal-resize-keyup-fallback --strict`
- [x] 1.2 Confirm doctrine alignment: "local feedback first" and focus-scoped input routing (CLAUDE.md, RFC 0004); resize stays viewer-authoritative
- [x] 1.3 Confirm the delta adds no new transport/RPC/input plane and no change to focus-scoping, clamping, pointer resize, or the scroll indicator

## 2. Verify implementation satisfies the requirement (PR #967)

- [x] 2.1 "apply resize on key release as fallback" — `portal_resize_key_code` + the key-up handler in `crates/tze_hud_runtime/src/windowed/keyboard.rs` (`dispatch_key_up_event_inner`) resize when the chord arrives release-only (`HotkeyResizeDir::from_key(...).or_else(from_key_code(...))`)
- [x] 2.2 "dedup consumed press/release pairs" — `consumed_portal_resize_keydowns` (`crates/tze_hud_runtime/src/windowed/mod.rs`) records key-DOWN intercepts so the key-UP fallback swallows the already-consumed chord, keeping a normal down/up pair to exactly one resize
- [x] 2.3 Locked by the key-up fallback regression test in `crates/tze_hud_runtime/src/windowed/portal.rs` (`ctrl_resize_keyup_fallback_resizes_when_live_windows_omits_keydown`)

## 3. Reconcile and close

- [x] 3.1 Sync the delta into `openspec/specs/text-stream-portals/spec.md` and archive (`openspec archive portal-resize-keyup-fallback`) — code already shipped, so this is a direct code-ahead-of-spec reconciliation
- [x] 3.2 Close `hud-y3pho`
