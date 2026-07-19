# hud-76clo: SceneSnapshot Ctrl resize validation — blocked by Windows key delivery

Date: 2026-07-17

This record captures the live result for the text-stream portal Ctrl resize
validation path. It answers one narrow question: did focused Windows Ctrl
hotkeys produce an authoritative portal geometry change? The answer on the
current interactive overlay is **no**; this is a redacted blocked result, not
an acceptance claim.

## Contract exercised

The validation driver focuses the composer surface, injects `Ctrl+Equal` and
`Ctrl+Minus` through the existing hidden-console interactive-task path, then
uses a fresh resident-protocol `SceneSnapshot` observer after each stage. It
retains only the target frame rectangle. The producer is
`build_resize_hotkey_snapshot_plan` and the observer/assertion boundary is
`capture_scene_snapshot_portal_bounds` / `assert_scene_snapshot_resize_bounds`
in `.claude/skills/user-test/scripts/text_stream_portal_exemplar.py`.

The runtime accepts the chord only when the focused scene tile has a scroll
configuration; `apply_portal_resize_hotkey` documents and enforces that
contract in `crates/tze_hud_runtime/src/windowed/portal.rs`. The composer is a
valid scrollable portal surface, and keyboard routing intercepts Ctrl resize
before composer draft routing in `crates/tze_hud_runtime/src/windowed/keyboard.rs`.

## Redacted protocol evidence

| Stage | Frame bounds from transient SceneSnapshot | Delta from previous stage |
|---|---:|---:|
| baseline | `x=2092, y=120, w=1720, h=1360` | — |
| Ctrl+Equal | `x=2092, y=120, w=1720, h=1360` | `dw=0, dh=0` |
| Ctrl+Minus | `x=2092, y=120, w=1720, h=1360` | `dw=0, dh=0` |

The live protocol did confirm `input:focus-gained` for the composer. Across
the chord stages it emitted no `input:key-down`, `input:character`, or
composer-draft events, while each interactive injector task completed. Since
the geometry stayed unchanged, the absence of downstream input events is
evidence that the overlay did not receive the injected Windows keyboard stream;
it is not evidence of a resize implementation failure.

No raw SceneSnapshot, host/user identity, credential, task output, or desktop
capture is retained here. The Ctrl resize plan includes no screenshot action,
and no GDI capture was generated or used as evidence.

## Condition to resume

Re-run the unchanged protocol assertion only after the overlay owns the actual
Windows keyboard input queue on the interactive desktop. The two acceptable
ways to establish that condition are:

1. Bring the `tze_hud` HWND to foreground and keyboard focus from the
   injector's interactive session before the chord (including the documented
   foreground-lock/input-queue workaround when Windows requires it).
2. Have a real operator session already holding overlay keyboard focus issue
   the Ctrl hotkeys.

This condition follows the Windows focus contract in
`focus_window_for_text_input` (`crates/tze_hud_runtime/src/windowed/lifecycle.rs`)
and is consistent with the existing K1 live-evidence record in
`docs/evidence/text-stream-portals/liveverify-resize-reverify-20260711/VERDICTS.md`.
Until then, `hud-76clo` remains open and blocked; no Beads state was changed.
