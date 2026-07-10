# Live-Verify Sweep — Motion Polish + Multi-Node Layout Regression (2026-07-11)

Host: `windows-host.example` (operator Windows console), overlay exe-direct
as `admin-user`, scene 3840×2160, MCP 9090 / gRPC 50051. Fresh build deployed
from current `main` (worktree base `1df2c512`, which includes **#1097** rpmwt
re-wrap+clip, **#1099** multi-node portal-part layout, **#1100** Wave-2
reconciliation). Deployed exe sha256 `11e6d77f…`, pid bound both ports, MCP
reachable (this build implements `tools/list`).

Method: resident gRPC scenario phases from `text_stream_portal_exemplar.py`
(baseline / streaming / profile-swap / scroll — all gRPC-driven, no injection) +
Windows OS pointer injection via the interactive scheduled-task path (for the
whole-portal resize attempts) + `TzeHudCapture` screendumps (CopyFromScreen,
5120×2880 3-monitor virtual desktop).

**Environment note (injection reachability):** the console is now a **3-monitor**
layout (primary 2560×1440 @ (0,0); two 2560×1440 satellites). The OS-input
injector scales scene→**primary-monitor** bounds and `SetCursorPos` in
primary-origin coordinates, so it can only address the primary monitor. The
overlay renders the 3840×2160 scene ~1:1 anchored near the primary's top-left,
so a full-size portal's bottom-right **22 px** resize handle falls off the
injector-reachable region (or at an unstable render offset when the overlay
spans monitors). Pointer-injected whole-portal resize was therefore not reliably
landable this session (three dead-center calibrated attempts hit the pane/minimize
region instead of the handle). This is a **tooling/environment limitation on the
current 3-monitor console**, not a product regression — the 2026-07-10 sweep
reached the handle under a different monitor arrangement.

| # | Item | Verdict | Evidence |
|---|------|---------|----------|
| 1 | Multi-node baseline render intact (#1099) | **PASS** | `01-baseline-portal-crop.png` (portal 1720×1360). Header drag-handles present; LEFT input/composer pane ("Exemplar Review Portal" + "type a reply — Enter to submit") cleanly separated from RIGHT transcript pane; GFM markdown renders readable (bold headings, mono code spans, lists); text wraps and is **bounded within the output pane** — no overflow past the frame; composer strip clean, not overpainted by transcript; container-part background consistent. |
| 2 | Per-part clip bands / no part paints over another (#1099) | **PASS** | Across `01`, `08`, `11`: input pane, output pane, header band, footer, dividers all stay in their own bands. Composer strip clean in every capture; no bleed between panes; header spans top without stomping content. |
| 3 | Transcript-only tail-ellipsis (#1099) | **PASS** | `11-profile-compact-crop.png` — last visible transcript line truncates cleanly with "…" ("Step + color-sweep…") inside the output pane; ellipsis confined to the transcript part (composer/header unaffected). |
| 4 | Bounds-aware wrap + clip live at a second (narrower) width | **PASS** | `08-after-cal-resize-crop.png` (portal 1400×900, narrower output pane). Same document re-rendered at the narrower pane **re-wraps to the narrower column** and stays clipped within the frame — no stale-wide-wrap overflow. Brackets the wide baseline (#1, 1720 px) → runtime is bounds-authoritative for wrap+clip live. |
| 5 | Whole-portal resize → re-wrap+clip live (hud-rpmwt / #1097) | **CANNOT-VERIFY (live, dynamic)** | The dynamic pointer-driven whole-portal resize (which takes the viewer-geometry-lock and drives the reconcile-on-republish fix) could not be exercised: the 22 px resize handle is not injector-reachable on the current 3-monitor console (see Environment note; `04-resize-injection-minimize-artifact-crop.png` shows a mis-hit minimize instead of resize). **Mitigations:** the fix is unit-covered RED-without/GREEN-with (`adapter_republish_after_resize_keeps_transcript_wrapped_to_resized_pane`, portal.rs) and #4 confirms live bounds-aware wrap+clip at two widths. Needs a single-primary-monitor console (or a gRPC/hotkey resize path) for a live dynamic repro. |
| 6 | Streaming fade reveal + follow-tail (motion polish) | **PASS** | `09-streaming-mid-crop.png` — captured mid-stream: the newly-revealed lower rows render as a distinct lighter **fade-reveal band** (the `StreamFadeRamp` mid-animation), footer reads "streaming • lines 1-108 / 120" (follow-tail advancing whole lines), content clipped within the output pane, scroll thumb visible bottom-right. Streaming ran to `scenario_complete` and settled with no flicker/tearing artifact. |
| 7 | Profile swap (reskin) live + chrome intact (regression) | **PASS** | `11-profile-compact-crop.png` vs the `01` grey standard — a distinct profile (warm/amber header accent, darker body, larger type) applies **live** via gRPC token republish; header/minimize/title/composer/panes all intact and correctly positioned; no part overpaint. Exemplar cycled compact→expanded→standard cleanly. |
| 8 | Scroll (smooth-scroll path) live (regression) | **PASS (offset path)** | `scroll.log` — 8 gRPC `set_scroll_offset_mutation` steps applied (scroll_y 0→160 px, viewport_start advanced 0→7) to `scenario_complete` with no error; runtime `ScrollSmoother` (tau 90 ms) animates displayed offset toward each target. Prior sweep already VERIFIED-FIXED the scroll observe-and-log / clipped-output behavior (hud-991cj). No fresh scrolled screendump (phase tears down quickly). |
| 9 | Cadence parameters sane (motion-polish tuning) | **PASS (by review + live render)** | `easing.rs` / `animation.rs`: SCROLL_SMOOTH_TAU_MS=90, snap ε=0.5 px; transitions IN=120 ms / OUT=80 ms; collapse/expand + fade = EaseInOut smoothstep; follow-tail = EaseOutQuad; all curves monotonic non-overshoot. Values read as quick-but-deliberate at 60 Hz and are unit-tested. **Note:** true smoothness/feel of eased transitions (120/80 ms) and scroll settle is **not observable from single-frame screendumps** (≪ capture latency) — the streaming fade band is the one motion actually caught mid-animation; deeper cadence "feel" tuning still wants human/high-speed observation. |
| 10 | Baseline chrome/header/composer/transcript correct (fresh regression) | **PASS** | Consolidated across #1–#3, #6–#7: chrome, header drag-handles, composer strip, transcript render, and per-part layout all correct on the fresh `main` build across four widths/profiles. |

## Cleanup
All exemplar leases released (each phase logged `lease released` / `scenario_complete`);
no ambient/zone publishes made; no tiles left painted; overlay confirmed **clean**
via a final live screendump (empty desktop; capture withheld from the commit —
full-desktop images are not committed per the environment-scrub policy) and the
per-phase `lease released` log lines above. Host probe artifacts removed
(`TzeHudBoundsProbe` task deleted; probe scripts + transient diagnostic-input
scripts deleted). PSK never written to any committed file.

## Discovered follow-ups
- **Live dynamic resize re-wrap (hud-rpmwt/#1097) needs a reachable resize path
  for autopilot repro.** On multi-monitor consoles the OS-input injector cannot
  land the 22 px whole-portal resize handle (addresses primary monitor only).
  Options: (a) add a gRPC/scene-mutation or keyboard-hotkey whole-portal resize
  that takes the viewer-geometry-lock (testable without pointer), or (b) pin the
  test host to a single primary monitor for live-verify sessions. Filed as a
  follow-up.
- **Motion cadence "feel" tuning is not screendump-verifiable.** Eased
  transitions (120/80 ms) and scroll settle (tau 90 ms) are sub-capture-latency;
  autopilot single-frame captures can confirm clean end-states and the streaming
  fade band but cannot tune easing feel. Needs human live observation or on-host
  high-frame-rate capture.
