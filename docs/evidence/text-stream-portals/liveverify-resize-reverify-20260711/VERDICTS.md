# VERDICTS — live OS-injection whole-portal resize RE-VERIFY post-#1129 (hud-8agm0)

**Date:** 2026-07-11
**Host:** autonomous `windows-vm.example` HUD testhost — single **1280×800** display
(bottom-right resize affordance is pointer-reachable).
**Binary:** FRESH cross-compiled `tze_hud.exe`, git **`82790cf7`**, sha256
`997cf67e…` (`logs/exe.sha256`) — **includes #1129** (`fix(portal): wire
whole-portal resize on the first-class surface path [hud-yno2r]`). Re-hashed
on-device == local build; running PID launched from it.
**Config:** DEFAULT. `tze_hud.toml` = `profile = "full-display"`, agent-alpha
registered; launched via the `TzeHudFullscreen` scheduled task
(`--window-mode fullscreen --config … --bind-all-interfaces`). **No
`TZE_HUD_RESIDENT_GRPC_PORTAL` bridge flag** at Machine or User env scope — the
prior bridge-routing run (hud-rw8eo) left no persistent bridge config; only
`TZE_HUD_PSK` + `TZE_HUD_MCP_RESIDENT_PRINCIPAL` are set. This run drives the
resident-gRPC first-class `PortalSurface` path (hud-rpm9s), agent-alpha.
**Injection:** real Windows OS pointer + keyboard via the interactive
scheduled-task `SendInput`/`mouse_event`/`SetCursorPos` path, with the injector
console HIDDEN (the hud-egn13 H1 fix) so events reach the HUD overlay.
**Methodology:** the proven hud-egn13 harness, **reused verbatim**
(`resize_injection_driver.py` here is byte-identical to
`../liveverify-resize-injection-20260711/resize_injection_driver.py`); only
`--outdir` differs.
**Authoritative evidence:** the gRPC `SceneSnapshot` — `tiles[].bounds` (whole
tile) and `nodes[].data.<Variant>.bounds` (transcript `TextMarkdown` node +
`portal-resize-bottom-right` hit node). `viewer_geometry_locked` is
`#[serde(skip)]` (element_store.rs:238), so lock state is **inferred** from its
consequences, never read directly.

## Headline

**#1129 is confirmed live.** Whole-portal **pointer** resize — inert (Δw=0) on
the pre-fix build (hud-egn13 finding P1) — now **applies end to end via real OS
pointer input**: the frame grows and every group member scales, and the resize
step re-wraps the transcript content and re-anchors the hit regions to the new
pane. This **closes the hud-rpmwt dynamic re-wrap CANNOT-VERIFY axis** for the
resize→re-wrap direction.

**But a new, distinct gap surfaced at the *republish* step (finding R1):** on the
live first-class-surface path the **outer** tile/frame geometry stays resized (no
snap-back), yet a subsequent adapter **republish reverts the inner republished
nodes** (transcript wrap width AND hit regions, incl. the resize band) back to
the agent's **declared attach-time geometry** — the opposite of the unit-tested
`adapter_republish_after_resize_keeps_transcript_wrapped_to_resized_pane`
contract. Same unit-green/live-red class as #1129. The #1109 **keyboard** chord
remains **injection-limited** (finding K1), Δw=0.

## The evidence in one table

Transcript `TextMarkdown` node width and the `portal-resize-bottom-right` hit
node (tile-local), across the run:

| Snapshot | transcript node w | resize band (local x,y,w,h) | frame w | output tile w |
|---|---|---|---|---|
| `00-baseline` | 391.0 | 830.0, 650.0, 34.0, 34.0 | 860.0 | 391.0 |
| `01-pointer-resized` (post-resize, **pre-republish**) | **463.7** | **984.4, 702.6, 40.3, 36.8** | **1020.0** | **463.7** |
| `02-pointer-republish` | 391.0 | 830.0, 650.0, 34.0, 34.0 | 1020.0 | 463.7 |
| `03-persist` (+settle) | 391.0 | 830.0, 650.0, 34.0, 34.0 | 1020.0 | 463.7 |
| `07-move-probe` | 391.0 | 830.0, 650.0, 34.0, 34.0 | 1020.0 (moved x 210→102.4) | 463.7 |

At the **resize** (`01`): the transcript node, the output tile, the frame, and
the resize band all track the new bounds (band re-anchored from the 860-frame
corner to the 1020-frame corner and rescaled 34→40.3/36.8). At the first
**republish** (`02`): the outer tile/frame hold, but the inner republished nodes
snap back to the declared 860-frame geometry — the resize band is now at local
`830,650` = the **interior** of the 1020-wide frame, no longer on its corner.

## Per-check verdicts (task hud-8agm0)

| # | Check | Verdict | Evidence |
|---|-------|---------|----------|
| M | Whole-portal **move** via OS pointer (focus-independent) | **PASS** | frame `x` `210→102.4` (Δx −107.6) for an injected `−110` header drag. `06-pre-move` → `07-move-probe`. Proves OS pointer drives the portal. |
| 1 | Pointer-band whole-portal **resize** → Δw>0, whole portal scales | **PASS ✅** | frame **860×680 → 1020×735** (Δw +160, Δh +55); members scaled (input_scroll +79.4/+44.3, output_scroll +72.7/+44.3, capture_backstop +160/+55); transcript node 391→463.7, resize band re-anchored + rescaled. `00-baseline` → `01-pointer-resized`. **#1129 works live.** |
| 2 | Post-resize adapter republish re-wraps/clips to the new bounds (hud-rpmwt dynamic) | **CANNOT-VERIFY axis CLOSED at resize; NEW gap at republish (R1)** | The dynamic re-wrap DOES occur — the resize itself re-wrapped the transcript node 391→463.7 (`01`), closing the motion-sweep CANNOT-VERIFY for resize→re-wrap. But the *republish-reconcile* sub-contract fails: on republish the transcript node reverts to 391 (`02`) while the output tile stays 463.7. Machine `check2 pass=false`. See R1. |
| 3a | No snap-back across settle + republish (**outer** geometry) | **PASS** | frame `210,60,1020,735` identical `02`→`03`; the whole-portal resize is NOT undone. |
| 3b | Hit regions (incl. resize band) re-anchored to the **new** frame after republish | **FAIL (R1)** | resize band re-anchors correctly at the resize (`01`: local 984.4,702.6) but reverts to the 860-frame corner (local 830,650) on republish (`02`), landing in the interior of the resized frame. Same root cause as check 2. |
| 4 | #1109 **keyboard** chord (Ctrl+Shift+Right) → grow + re-wrap | **INJECTION-LIMITED (K1)** | chord injects cleanly (`SendInput` ok, 6 repeats, no stderr) after composer-first OS-key-focus acquisition; **frame Δw=0** (`04-kbd-resized`). Distinct from the resize logic — pointer resize works on the same build. |

## Findings

### R1 (PRODUCT — report, not filed) — reconcile-on-republish does not hold live on the first-class-surface path
The runtime's viewer-geometry-lock contract (unit-tested at
`crates/tze_hud_runtime/src/windowed/portal.rs:6674`,
`adapter_republish_after_resize_keeps_transcript_wrapped_to_resized_pane`, and
the sibling `ctrl_shift_arrow_width_resize_then_republish_reconciles_to_narrower_pane`
at :5544) states: while a tile is viewer-geometry-locked, an adapter republish
carrying **stale attach-time bounds** MUST be reconciled to the resized pane, NOT
left at the stale width. The reconcile is unconditional-on-lock inside
`SceneGraph::set_tile_root_impl` (tiles.rs:452) and `add_node_to_tile_impl`
(tiles.rs:661) via `reconcile_locked_subtree_to_tile_bounds`
(`crates/tze_hud_scene/src/graph/node_tree.rs:70`), so any caller — including the
live resident-gRPC republish — should hit it.

Live, it does **not**: after the locked resize (which correctly scaled tile +
node tree at `01`), the exemplar's `publish_portal(include_tile_setup=False)`
re-declared the transcript root and hit nodes at their config (860-frame)
geometry, and the runtime **kept those stale bounds** (transcript 391, band
830,650) instead of reconciling them to the resized pane (463.7 / 1020-frame
corner). The outer tile/frame bounds stayed resized only because
`include_tile_setup=False` never re-sends them.

Because `viewer_geometry_locked` is serde-skip, the snapshot cannot say **which**
of two candidate root causes holds:
- (a) the first-class-surface resize scales the node tree but does **not take**
  the viewer-geometry-lock on the constituent panes (so there is no lock for
  republish to honor); or
- (b) the lock is taken at resize but **dropped/not effective** before the
  resident-gRPC republish reaches `set_tile_root`.

Either way the observable is unambiguous and is the same unit-green/live-red gap
class that #1129 itself fixed for the resize-apply path: the headless suite
exercises `lock + set_tile_root` directly and is green, while the live
first-class-surface republish path has no end-to-end coverage and reverts. Recommend
the runtime team (a) live-verify that the first-class-surface resize takes and
retains the viewer-geometry-lock on every constituent pane through a republish,
and (b) add a live-path (enqueue/resident-gRPC-ordered) integration test that
resizes, republishes at stale bounds, and asserts inner-node reconcile — mirroring
how #1129 added `enqueue_pointer_corner_drag_resizes_whole_portal_after_click_to_focus`.

### K1 (injection limitation, unchanged from hud-egn13) — overlay does not take OS keyboard focus from the injector
The #1109 chord injects cleanly but does not resize (frame Δw=0). Root cause is
unchanged: the topmost, taskbar-hidden, `WS_EX_NOREDIRECTIONBITMAP` overlay only
acquires **OS keyboard focus** via the `AttachThreadInput` foreground-lock
workaround (`focus_window_for_text_input`, hud-dwcr7) on text-input (composer)
focus; from an external injector session (foreground = the hidden injector, not
the HUD) `WindowEvent::KeyboardInput` is not reliably routed to the overlay. Now
that pointer resize is proven working on the **same build**, this isolates the
keyboard Δw=0 to the **keyboard-input-delivery** path, not the resize logic —
recorded injection-limited per task guidance, NOT asserted as a product bug. A
follow-up could `SetForegroundWindow(hud_hwnd)` from the injector before the
chord, or exercise the chord from a real operator session that already holds OS
keyboard focus.

## Caveats

- **Lock state is not directly observable** (`viewer_geometry_locked` is
  serde-skip). All lock claims are inferred from tile/node bounds consequences.
- **No screenshots committed.** GDI/`CopyFromScreen` capture does not composite
  the transparent Vulkan overlay on this software-GPU VM (same as hud-om69w /
  hud-egn13); the authoritative evidence is the gRPC `SceneSnapshot` bounds.
- **Move restore drift.** The header-drag move measured `−107.6` for an injected
  `−110` (drag-activation hysteresis) — expected, not a defect.

## Hygiene

Placeholders only: `windows-vm.example` (testhost), `proxmox-host.example`
(hypervisor), `agent-alpha` / `admin-user` (registered agent / interactive
injector account). No PSK, no real IPs/hostnames, no full-desktop frames. VM
left clean: lease released, zero tiles / zero portal_surfaces
(`snapshots/99-clean.json`, `tiles_remaining: 0`), injector scheduled tasks +
scripts removed, runtime left in the default `TzeHudFullscreen` config.
