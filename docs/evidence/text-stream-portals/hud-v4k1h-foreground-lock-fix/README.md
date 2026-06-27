# hud-v4k1h — OS keyboard-focus root cause + fix (Windows foreground-lock timeout)

**Date:** 2026-06-27 · **Reference HW:** `tzehouse-windows` · **Status:** fix proven
on-device; handed off for the consolidated hud-v4k1h PR.

## TL;DR for the hud-v4k1h worker

The reason **no keyboard input reaches the overlay at all** on `tzehouse-windows`
(typing, `Ctrl`+`=`/`-` resize, `Tab`, scroll all dead) is the Windows
**foreground-activation lock timeout** being set to `2147483647` (≈ infinite) on
that machine. With the lock at MAX, `SetForegroundWindow()` in
`focus_window_for_text_input` silently fails (even with the existing
`AttachThreadInput` workaround), so the overlay never acquires OS keyboard focus
and the OS delivers **zero `WindowEvent::KeyboardInput`**.

> The projection-path `accepts_pointer: true` change (in-app click-to-focus,
> `crates/tze_hud_projection/src/resident_grpc.rs`) is **necessary but
> INSUFFICIENT on its own** — it only governs the in-app `NodeHit` focus once a
> pointer event arrives. Without lifting the OS foreground lock, the OS delivers
> no keystrokes for any handler to receive. **Both fixes are required.**

The fix is already present (uncommitted) in the `agent-hud-v4k1h-focus`
worktree's `lifecycle.rs`, and as `lifecycle-focus-fix.diff` here. Please
incorporate it into the consolidated hud-v4k1h PR.

## Root cause (proven on-device)

`FOCUS-DIAG` captured live (instrumentation on the focus path):

```
FOCUS-DIAG(hud-v4k1h): hwnd=0x1f0f6a fg_before=0x1f0f6a attached=false
  prev_lock_timeout=2147483647 got_timeout=true lock_cleared=true
  SetForegroundWindow=true fg_after=0x1f0f6a fg_after_is_ours=true
```

`prev_lock_timeout=2147483647` is the smoking gun: `SPI_GETFOREGROUNDLOCKTIMEOUT`
was MAX. After momentarily clearing it, `SetForegroundWindow` returns `true` and
the overlay becomes foreground.

## The fix

`crates/tze_hud_runtime/src/windowed/lifecycle.rs` → `focus_window_for_text_input`:
bracket the existing `SetForegroundWindow`/`SetFocus` with
`SPI_SETFOREGROUNDLOCKTIMEOUT = 0` (save the previous value via
`SPI_GETFOREGROUNDLOCKTIMEOUT`, set 0 with `fWinIni = 0` so the change is
runtime-only — not persisted, no `WM_SETTINGCHANGE` broadcast — then restore).
Keep `AttachThreadInput`. See `lifecycle-focus-fix.diff`.

```rust
// after AttachThreadInput(.., TRUE):
let mut prev: u32 = 0;
let got = SystemParametersInfoW(SPI_GETFOREGROUNDLOCKTIMEOUT, 0,
    Some(&mut prev as *mut u32 as *mut c_void), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)).is_ok();
let _ = SystemParametersInfoW(SPI_SETFOREGROUNDLOCKTIMEOUT, 0,
    Some(core::ptr::null_mut()), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)); // 0 == no lock
let sfw = SetForegroundWindow(hwnd).as_bool();
let _ = SetFocus(hwnd);
if got { let _ = SystemParametersInfoW(SPI_SETFOREGROUNDLOCKTIMEOUT, 0,
    Some(prev as usize as *mut c_void), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)); }
// ... AttachThreadInput(.., FALSE)
```

The patch also leaves a **failure-only** `FOCUS-WARN` diag line (silent on
success) as durable observability for this thrice-recurring fragile path
(hud-dwcr7, hud-v4k1h).

## On-device evidence (after the fix)

Real `WindowEvent::KeyboardInput` now reaches the overlay + composer. Operator
keypresses, captured live (`KEY-DIAG`, physical key codes — impossible from the
harness Unicode typer, i.e. genuine hardware keystrokes):

```
KEY-DIAG: phys=Code(ControlLeft) ... ctrl=true
KEY-DIAG: phys=Code(Equal) logical=Character("=") state=Pressed ctrl=true   <- Ctrl+= now arrives
KEY-DIAG: phys=Code(KeyA)/KeyS/KeyD ... text=Some(..)                       <- composer typing works
```

Full proof: `focus-fix-proof.txt`. Before the fix (prior main `aa67a6e5`,
2026-06-22), the diag log had **zero** keyboard events and the operator
confirmed typing/resize/scroll all dead.

## Also surfaced live (separate bugs — NOT part of this focus fix)

With keys now flowing, two handler-level bugs are exposed (deferred to the
portal-component promotion epic; see linked beads on hud-v4k1h):

1. **Group-aware resize:** `Ctrl`+`=` resizes only the single focused tile (the
   input pane), not the whole portal — because the runtime has **no portal-group
   concept**; a portal is N independent tiles and even drag-move is grouped
   client-side. Operator report: "the input window floats to the top-left."
2. **Tab traversal:** portal interactive affordances are built with
   `accepts_focus=false`, so the focus cycle has no stops → "Tab doesn't work."

## Harness addition

`exemplar-harness.diff` adds real virtual-key chord injection (`Ctrl`+`=`/`-`,
`Tab`/`Shift`+`Tab`) to `text_stream_portal_exemplar.py` — the documented
injector gap ("a Windows injector that produces target OEM key events").
**Caveat:** synthetic `SendInput` only lands on an **unlocked, quiescent**
desktop (a locked/secure desktop returns `ERROR_ACCESS_DENIED`, and other
foreground windows intercept); the authoritative focused-portal resize check
remains a real operator keypress.
