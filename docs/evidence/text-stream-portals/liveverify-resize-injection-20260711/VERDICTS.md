# VERDICTS — live OS-injection whole-portal resize (§6b.7 / hud-egn13)

**Date:** 2026-07-11
**Host:** autonomous `windows-vm.example` HUD testhost — single **1280×800** display
(so the bottom-right resize affordance is pointer-reachable; the 3-monitor
injector-unreachability blocker from the 2026-07-11 motion sweep does not apply).
**Binary:** current-`main` `tze_hud.exe`, sha256 `af6b215c…` (see `logs/exe.sha256`)
— reused as deployed by the hud-om69w run, verified intact + MCP/gRPC up.
**Transport:** resident-gRPC first-class `PortalSurface` (hud-rpm9s), agent-alpha.
**Injection:** real Windows OS pointer + keyboard events via the interactive
scheduled-task `SendInput`/`mouse_event`/`SetCursorPos` path.
**Authoritative evidence:** the gRPC `SceneSnapshot` — `tiles[].bounds` (whole
tile) and `nodes[].data.TextMarkdown.bounds` (wrapped transcript). The
`viewer_geometry_locked` set is `#[serde(skip)]` (element_store.rs:238) so it is
**not** observable in a snapshot; its *consequence* (a post-resize republish that
re-wraps to the new bounds instead of snapping back) is the intended proof.

## Headline

Real OS input **does** reach and drive the HUD end to end once a test-harness
blocker is removed (see finding H1) — the whole-portal **move** (header drag)
applies correctly and is verified via gRPC. But the whole-portal **resize** —
both pointer-drag and the #1109 keyboard chord — **does not apply** via real OS
input. The pointer-resize failure is a **genuine product gap** invisible to the
headless test suite (finding P1); the keyboard-resize failure is
**injection-limited** on this host (finding K1).

## Per-check verdicts

| # | Check | Verdict | Evidence |
|---|-------|---------|----------|
| M | Whole-portal **move** via OS pointer (focus-independent drag handle) | **PASS** | frame `x,y` `210,60` → `102.4,96` for an injected `(-110,+40)` drag (measured `-107.6,+36`). `snapshots/06-pre-move.json` → `07-move-probe.json`. |
| 1 | Whole-portal **pointer resize** via the bottom-right affordance → tile bounds grow | **FAIL — product gap (P1)** | correct aim `(1066,736)` inside the 8px BottomRight band; injection `ok:true`; **frame Δw=0, Δh=0**, all members unchanged. `00-baseline.json` → `01-pointer-resized.json`. NOT injection-limited (M proves OS pointer drives the portal). |
| 2 | Post-resize adapter republish re-wraps/clips to the new bounds (hud-rpmwt) | **BLOCKED by #1** | transcript `TextMarkdown` width `391` → `391` (no resize to react to). Observability is in place (width captured); there is simply no resize to re-wrap. `02-pointer-republish.json`. **The CANNOT-VERIFY from the 2026-07-11 motion sweep therefore remains open** — it is gated on #1. |
| 3 | #1109 **keyboard** whole-portal resize (Ctrl+Shift+Right) → same grow + re-wrap | **INJECTION-LIMITED (K1)** | chord injects cleanly (`SendInput` ok, 6 repeats, no warnings) after composer-first OS-keyboard-focus acquisition; **frame Δw=0**. `04-kbd-resized.json`. |
| 4 | Resize geometry persists (no snap-back), hit regions stay aligned | **PARTIAL / N/A for resized geometry** | Declared geometry is stable across settle + republish (frame `210,60,860,680` identical in `02`/`03`) and the resize-band node stays anchored to the frame corner (local `830,650,34,34`). But because #1 never resized, there is **no resized geometry to test for snap-back** — the snap-back guarantee itself is unverified here (gated on #1). |

## Findings

### H1 (methodology, ROOT-CAUSED + FIXED) — the injector's own console ate the input
The interactive scheduled-task launches a **visible `powershell.exe` console**
that becomes the foreground/topmost window. With it visible, `WindowFromPoint`
over **every** portal coordinate (`640,400`, `1066,736`, `856,427`) returns the
injector's own console (hwnd `1638868`, rect `47,0,1280,752`) — the synthetic
pointer/keyboard events land on the console, never the transparent HUD overlay.
The exemplar's prior "success" (hud-ofe76) only ever asserted `ok:true` =
"`SendInput` returned", never "the portal reacted", so this was never caught.
**Fix (in this harness): hide the console (`ShowWindow(SW_HIDE)`) at the top of
the injected script.** After the fix the move probe drives the portal end to end
(finding M). Coordinate mapping is correct: the interactive session reports
`PrimaryScreen.Bounds = 1280×800` = the gRPC scene area (scale 1.0). *(The
`1024×768` / `foreground=0` seen from a **direct-SSH** shell are the classic
non-interactive-session defaults, not the interactive desktop the HUD renders on
— the injector correctly uses the interactive scheduled-task principal.)*

### P1 (PRODUCT — report, not filed) — OS pointer resize never starts the gesture
Whole-portal **pointer** resize cannot be initiated by real OS pointer input on
the reference text-stream portal, even with correct aim, a captured frame corner,
and a focused scrollable pane. Root cause, narrowed in-code:

- The overlay pointer-down dispatch runs **click-to-focus first**
  (`process_with_focus`, `windowed/lifecycle.rs:1147`) and only **then** the
  resize handler (`apply_portal_resize_pointer_event`, `:1389`).
- The resize handler gates on a **scrollable** focused tile
  (`scene.tile_scroll_config(focused_tile_id)?`, `windowed/portal.rs:1028`) and
  resolves the whole group against the **frame** anchor rect (`resolve_portal_group`).
- But the resize corner lies on the **frame** tile (non-scrollable). The
  click-to-focus on that same pointer-down moves/clears focus off the scrollable
  pane the gate requires, so the gate returns `None` and the gesture never starts.
- The unit/integration tests (e.g. `pointer_affordance_resize_scales_whole_portal_from_frame`,
  `portal.rs:5935`) **pre-set** the FocusManager to the composer and call
  `apply_portal_resize_pointer_event` **directly**, bypassing the live
  click-to-focus step — so they are green while the live OS path is dead.
- Compounding rect inconsistency: the overlay **capture-affordance** hit-test
  (`cursor_over_focused_portal_affordance` → `focused_portal_tile_rect`,
  `windowed/hittest.rs`) keys off the focused **scrollable sub-pane's own** rect,
  while the resize **commit** keys off the **frame** rect. When the scrollable
  pane does not reach the frame's bottom-right corner (the normal header + panes
  + footer layout), the two never agree on where the affordance is.

Verified negative across three live configurations: exemplar portal focusing the
output pane; exemplar portal focusing the composer; and a synthetic portal whose
scrollable tile fills the frame (corner-coincident) — all `Δw=Δh=0`.

**This is exactly the class of gap live OS-injection exists to surface: headless
tests call the commit path directly; the real OS-pointer → gesture path has no
end-to-end coverage.** Recommend the runtime team either (a) evaluate the resize
affordance before click-to-focus mutates focus on the initiating down, or (b)
let the resize gate resolve the portal group from the tile under the pointer
(the frame) rather than requiring the *focused* tile to be scrollable.

### K1 (injection limitation) — overlay does not take OS keyboard focus from the injector
The #1109 chord injects cleanly, but the topmost, taskbar-hidden,
`WS_EX_NOREDIRECTIONBITMAP` overlay only acquires **OS keyboard focus** via the
`AttachThreadInput` foreground-lock workaround in `focus_window_for_text_input`
(`windowed/lifecycle.rs`, hud-dwcr7), which fires on **text-input (composer)**
focus. Driving that from an external injector session — where the foreground
process is the (hidden) injector, not the HUD — does not reliably route
`WindowEvent::KeyboardInput` to the overlay. We reproduced the real keyboard-viewer
path (focus composer → OS key focus acquired → click transcript to a non-composer
surface → chord) but the chord still did not land (frame `Δw=0`). Recorded
injection-limited per task guidance; **not** asserted as a product bug — a real
operator whose overlay already holds OS keyboard focus is untested here. A
follow-up could `SetForegroundWindow(hud_hwnd)` from the injector before the chord.

## Caveats

- **No screenshots committed.** GDI/`CopyFromScreen` capture does not composite
  the transparent Vulkan overlay on this software-GPU Proxmox VM (same as
  hud-om69w §caveat), and the resize under test never applied, so a frame would
  add nothing over the authoritative gRPC bounds. All verdicts are gRPC-sourced.
- **Move restore drift.** The header-drag move does not land pixel-exact
  (measured `-107.6,+36` for an injected `-110,+40`) due to the drag-activation
  hysteresis threshold — expected, not a defect.

## Hygiene

Placeholders only: `windows-vm.example` (testhost), `proxmox-host.example`
(hypervisor), `agent-alpha` / `admin-user` (registered agent / SSH user). No PSK,
no real IPs/hostnames/usernames, no full-desktop frames. VM left clean: lease
released, zero tiles / zero portal_surfaces (`snapshots/99-clean.json`), injector
scheduled tasks + scripts removed.
