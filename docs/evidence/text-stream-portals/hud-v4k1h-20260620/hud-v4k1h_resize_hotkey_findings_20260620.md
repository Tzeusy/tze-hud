# hud-v4k1h / hud-sp8l7 — Ctrl +/- portal resize: live-injection findings

- Date: 2026-06-20
- Beads: `hud-v4k1h`, `hud-sp8l7`
- Host: `tzehouse-windows.parrot-hen.ts.net:50051`, user-test PSK + `agent-alpha`

## DEFINITIVE ROOT CAUSE (operator-confirmed reproduction, 2026-06-20)

Two operator-watched live runs (composer focus, then output-pane focus) BOTH
showed **no visible resize**. Since the runtime applies the new tile bounds
directly to the scene when the hotkey fires (`portal.rs:apply_portal_resize_hotkey`
writes `tile.bounds` + bumps `scene.version`), "no visible change" means the
hotkey **returned early — it never received the keystroke**.

The code already documents exactly why, at
`crates/tze_hud_runtime/src/windowed/lifecycle.rs:62-70`:

> Acquiring OS keyboard focus for a topmost, taskbar-hidden,
> `WS_EX_NOREDIRECTIONBITMAP` overlay is subject to the Windows
> foreground-activation lock: a bare `SetForegroundWindow()` from a process that
> is not already the foreground process silently fails. When it fails the
> overlay receives mouse input (routed via `SetCapture`, which does not require
> keyboard focus) but **NO `WindowEvent::KeyboardInput`** — typing and
> focus-scoped hotkeys (Ctrl+/-) are dead even though the composer is focused at
> the scene level (hud-dwcr7).

This matches the live behavior exactly: every **mouse**-routed interaction works
(click-focus → `input:focus-gained`, header drag, gRPC scroll), but every
**keyboard**-routed action (Ctrl+/- resize) is dead. The `from_key_code` fix
(PR #937) is present and correct — but the keystroke never reaches dispatch.

`focus_window_for_text_input` (lifecycle.rs:48) has an `AttachThreadInput` +
`SetForegroundWindow` + `SetFocus` workaround for the foreground lock. It is
INSUFFICIENT in live operation: with the current build the overlay still does
not receive `WindowEvent::KeyboardInput` for injected (or, by the same
mechanism, physically-typed) Ctrl+/- chords. **hud-v4k1h is a live-reproducing
symptom of the (closed) hud-dwcr7 keyboard-focus issue — the foreground-lock
workaround does not fully restore OS keyboard delivery to the overlay.**

### Fix direction (for the implementer)

- Verify `focus_window_for_text_input` actually wins foreground in live op
  (log `GetForegroundWindow()` == our HWND after the attach/detach).
- If `SetForegroundWindow` still loses, consider a low-level keyboard hook
  (`WH_KEYBOARD_LL`) that forwards Ctrl+/- (and composer text) to the overlay
  independent of OS foreground, or a dedicated always-foreground input shim.
- Add a runtime debug counter for `WindowEvent::KeyboardInput` received so this
  is observable without screen capture next time.

## Status: injection path exercised; visible/logged confirmation BLOCKED by observability limits (NOT closed)

### What is established (high confidence)

1. **The fix is present.** The documented root cause of `hud-v4k1h` — winit on
   Windows does not resolve the logical key to bare `=`/`-`/`+` under Ctrl — was
   fixed in PR #937 (`HotkeyResizeDir::from_key_code`, physical `KeyCode`
   `Equal`/`Minus`/numpad), wired into dispatch at
   `crates/tze_hud_runtime/src/windowed/keyboard.rs:283-284`. Commit `7ade0a08`
   is an ancestor of `main`; the deployed exe (built 2026-06-20 09:47 UTC) is
   newer, so it contains the fix. The "still doesn't work" operator report
   (2026-06-19) predates the fix.
2. **The injection harness works.** A new `resize-hotkey` phase + `chord`/
   `screenshot` injector actions were added to `text_stream_portal_exemplar.py`.
   Live run: `ok=true, returncode=0`. Composer focus was gained
   (`input:focus-gained`; a composer caret appears in the capture, confirming
   focus), and `Ctrl+Equal` ×6 / `Ctrl+Minus` ×12 were injected through the real
   Windows `SendInput` path with `wVk = VK_OEM_PLUS/VK_OEM_MINUS` (so the runtime
   sees the physical `Equal`/`Minus` KeyCode the resize dispatch resolves).

### Why the result could NOT be confirmed this session

The portal lives on the transparent overlay window, created with
`WS_EX_NOREDIRECTIONBITMAP` + Vulkan/DirectComposition. Two automated
observation paths both failed for structural reasons:

1. **GDI screenshot (`CopyFromScreen`) cannot capture the overlay.** The
   before/grow/shrink PNGs show only the desktop *behind* the overlay (VS Code) —
   not even the config's gauge/progress/status widgets appear. baseline→grow
   differed by 42 px (the composer caret); baseline→shrink was byte-identical.
   This is expected: `NOREDIRECTIONBITMAP`/DirectComposition surfaces bypass the
   GDI redirection bitmap that `CopyFromScreen`/BitBlt read. **The diff is
   therefore inconclusive about the resize, not evidence it failed.**
2. **Runtime debug log not captured.** Relaunching under
   `RUST_LOG=tze_hud_runtime::windowed=debug` with stdout/stderr redirected to a
   file yielded a 0-byte log — the overlay process does not surface tracing to
   the redirected handle in this launch mode. So the definitive
   `"portal resize: Ctrl hotkey consumed (resize applied)"` line could not be
   collected.

### Recommended next step (one of)

- **Operator visual confirm** (cheapest — operator is at the TzeHouse console):
  re-run the `resize-hotkey` phase and watch the portal grow on Ctrl+Equal /
  shrink on Ctrl+Minus.
- **DXGI Desktop Duplication / Windows.Graphics.Capture** screenshot instead of
  `CopyFromScreen` — these DO capture hardware/DirectComposition overlays; swap
  the `Capture-Screen` implementation to use one of them.
- **Agent-side key-forward check** — with `access_input_events`, subscribe to
  forwarded KeyDown events: a *consumed* resize hotkey is NOT forwarded to the
  agent, so absence of the injected `Ctrl+Equal` in the agent stream is positive
  evidence the resize fired.

## Harness changes (committed)

- `text_stream_portal_exemplar.py`: `chord` + `screenshot` injector actions,
  `Send-VkChord`/`Capture-Screen` PowerShell, `build_resize_hotkey_plan`,
  `run_resize_hotkey_phase`, `resize-hotkey` phase, `--resize-shot-prefix`.

## Artifacts

- `resize-hotkey-transcript.json`, `resize-hotkey-logged-transcript.json`
- `resize-{baseline,grow,shrink}.png` (desktop-only; demonstrates the GDI-capture
  limitation, not a resize failure)
- `hud-portal-debug.log` (0 bytes — capture limitation)
