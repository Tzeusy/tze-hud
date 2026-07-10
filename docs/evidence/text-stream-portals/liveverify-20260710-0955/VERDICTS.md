# Text Stream Portal — Live-Verify Sweep Verdicts (2026-07-10)

> **Retro-scrub 2026-07-11 [hud-ryawj]:** real host identifiers in this file were replaced with
> placeholders (`windows-host.example`, `admin-user`, etc.). Additionally, the full-screen
> (5120×2880) `CopyFromScreen` captures in this directory were deleted because they exposed the
> operator's desktop (taskbar + full desktop-icon inventory) behind the transparent overlay; the
> tight `*-crop.png` HUD crops and the `*.log` traces are retained as the evidence of record. Git
> history retains the removed blobs (history rewrite out of scope).

Host: tzehouse-windows (`windows-host.example`), overlay exe-direct
as `admin-user`, scene 3840×2160, portal 1720×1360 @ (2092,120). MCP 9090 / gRPC 50051.
Overlay build already deployed ~09:55 SGT (today's main). Resumed a sweep the
prior worker left partial (killed by usage limit ~10:26 SGT).

Method: exemplar `text_stream_portal_exemplar.py` resident gRPC scenario phases +
Windows OS pointer/wheel injection (interactive `admin-user` session, scheduled-task
dispatch) + `TzeHudCapture` screendumps (CopyFromScreen, 5120×2880 virtual
desktop). Keyboard/Unicode injection is unreliable on this host (SendInput
KEYEVENTF_UNICODE does not reliably reach the overlay); pointer + wheel injection
is reliable. Portal header drag hit-region is `accepts_pointer=False`, so a
header MOVE cannot be initiated by synthetic pointer injection (only a real
pointer or the runtime's own drag path). The bottom-right resize handle uses
`auto_capture=True` and IS pointer-injectable.

All portal leases released cleanly (TTL 600s); no ambient/zone publishes made;
overlay left clean (empty desktop confirmed post-run).

| # | Bead | Verdict | Evidence |
|---|------|---------|----------|
| 1 | hud-991cj | VERIFIED-FIXED | `diag-input.log` scroll:output — 3 wheel notches → runtime-translated local-first scroll, "no agent re-render", output stays clipped in transcript box. #1031 fix (scroll = observe-and-log; full body in node) eliminates the per-event re-render that caused flicker. |
| 2 | hud-pncm3 | CANNOT-VERIFY | Requires submitting long + multi-line composer entries (Enter / Shift+Enter) to render viewer-echo turns; keyboard injection unreliable on this host and there is no gRPC submit path (viewer-echo is runtime-owned from a real ComposerDraftSubmitEvent). Needs a human keyboard round. |
| 3 | hud-uyhpn | VERIFIED-FIXED | Owner live feel-confirmation round-6 ("drag is looking better") after #1022 scene-lock/present split. Fresh synthetic header-drag not achievable (header hit-region `accepts_pointer=False`); verdict rests on owner's live confirmation. |
| 4 | hud-2v8br | CANNOT-VERIFY | Tab traversal + focus ring is keyboard-driven; synthetic key injection unreliable on this host (same path that fails to register composer text). Tried: nothing reliable to inject Tab. Needs a human keyboard round. |
| 5 | hud-2zsbf | VERIFIED-FIXED | `2zsbf-portal-crop.png` — long markdown paste wraps within the composer (INPUT) pane, caret sits correctly at text end with no trailing-space offset, draft does NOT extend past the composer box; OUTPUT pane bounded. |
| 6 | hud-lyqun | VERIFIED-FIXED (resize/republish coherence) | Live pointer-injected resize: `lyqun-resize.log` portal-resize:start (1720×1360) → portal-resize:end (1320×1040), "all portal tiles committed to grouped position", panes re-partitioned (input 655px / output 658px). 35 grouped gRPC mutations on republish did NOT stomp/fracture members — header, input pane, composer, footer, dividers all remained aligned in the resized frame (`rpmwt-after-resize-crop.png`). The historical fracture (adapter republish stomps members) did not reproduce. CAVEAT: the chained header-drag half could not be pointer-injected (`accepts_pointer=False`); resize-coherence + prior programmatic group-move coherence (`window-mgmt.log`) cover the group-integrity mechanism. |
| 7 | hud-rpmwt | **STILL-BROKEN** | Live resize (1720×1360 → 1320×1040). Exemplar re-published re-wrapped body at the new output width (658px, 35 mutations), but the runtime rendered the OUTPUT-pane markdown at the STALE (wider ~1720) wrap and it overflows ~400px past the resized frame's right edge, unclipped (full-screen `rpmwt-after-resize.png` removed 2026-07-11 for privacy; the overflow is retained in crop `rpmwt-after-resize-crop.png`). Text does not re-wrap to the new pane width — the "node-bounds authority" follow-up the bead itself anticipated. |
| 8 | hud-acfvp | CANNOT-VERIFY | Wheel-scroll of INPUT-pane history requires populated history entries (from submits — keyboard-gated) before there is anything to scroll; no reliable submit injection and no seeded history. Wheel primitive itself works (proven on OUTPUT pane). Needs a human to submit entries (or a tail-seed hook) first. |
| 9 | hud-tlx5c | VERIFIED-FIXED | `tlx5c-compact-crop.png` (compact 680×520, title 16pt/body 14pt, INPUT/TRANSCRIPT panes) vs `tlx5c-expanded-crop.png` (expanded, larger fonts, full GFM markdown render). Reskin applies live; chrome intact in both profiles; distinct, coherent looks. |
| 10 | hud-o9ybl | VERIFIED-FIXED | `composer-edit.log` empty→typed→mid-delete→cleared with caret_x tracked through every state (0→45.6→118.56→109.44→45.6→36.48→0), no phantom chars; `o9ybl-composer-edit-typed.png` (removed 2026-07-11 for privacy — full-desktop capture; the `composer-edit.log` caret trace above is the retained record) showed the draft "Hello" rendered inside the composer box. |
| 11 | hud-sonj6 | VERIFIED-FIXED (short phase) | `cadence.log` complete 24-cycle run: runtime_overhead_ms {mean 0.002, p95 0.003, max 0.004}, over_budget_count 0/24 vs 16.6ms present budget, within_present_budget=true, all 24 presented, lease released. Prior failing baseline (p95 23.5ms, 8/24 over) resolved. (Short phase only; 60-min soak = hud-5kq8k, out of scope.) |

## Discovered follow-ups
- **hud-rpmwt STILL-BROKEN** — OUTPUT-pane transcript does not re-wrap to the new
  pane width after whole-portal resize; text renders at stale (wider) wrap and
  overflows the resized frame unclipped. Runtime does not honor re-published node
  bounds for text wrap (node-bounds-authority). New live repro captured today.

## Not fully live-verifiable with current tooling (needs a human keyboard round)
- pncm3 (viewer-echo submit), 2v8br (Tab focus), acfvp (INPUT-history scroll) —
  all gated on reliable keyboard/submit injection, which this host lacks.
- lyqun chained header-drag — header hit-region `accepts_pointer=False`.
