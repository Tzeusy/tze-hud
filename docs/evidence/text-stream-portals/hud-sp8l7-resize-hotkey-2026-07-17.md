# Portal resize hotkeys: current TzeHouse capture blocked (hud-sp8l7)

**Verdict: BLOCKED.** This capture does not prove that `Ctrl`+`=` visibly grows, or that `Ctrl`+`-` visibly shrinks, the focused portal. It must not be used as a passing live confirmation.

## Provenance

- Source revision built and deployed: `6f84b54c0bc8c539816310eb3a1d032bbf681dcc`.
- Windows release executable SHA-256: `23239a6e0cfa4e7ff776d8c3251d5f1716ef72bbef35c225073f4d03e4154a2b` (25,164,308 bytes).
- The remote executable reported `tze_hud 0.1.0 (6f84b54c)` and its SHA-256 matched the local build before the capture.
- The resolver/self-heal route, resident gRPC, and MCP authentication gate all passed before the run. No target identity, credential, or raw desktop image is retained here.

## What ran

The tracked [`resize-hotkey` plan](../../../.claude/skills/user-test/scripts/text_stream_portal_exemplar.py#L1193-L1225) focuses the output pane, captures a baseline, sends six `Ctrl`+`=`, captures a grow image, sends twelve `Ctrl`+`-`, and captures a shrink image.

The first attempt used the default 12-second diagnostic-input deadline and timed out after its baseline capture; that timeout is not a product result. A second run used the harness's supported 45-second deadline and completed its scheduled-task step with `ok=true`, exit code `0`, and a 7.437-second duration. The transcript also recorded `input:pointer-down-unhandled`; this only establishes a pointer checkpoint, not delivery of the resize chords.

| Capture | Pixels | Bytes | SHA-256 |
| --- | ---: | ---: | --- |
| Baseline | 2560 × 1440 | 213,816 | `9f2006f872cbe98b98898a849bf6cbea3934233c654877d4b2a372af570976d5` |
| Grow | 2560 × 1440 | 213,816 | `9f2006f872cbe98b98898a849bf6cbea3934233c654877d4b2a372af570976d5` |
| Shrink | 2560 × 1440 | 213,816 | `9f2006f872cbe98b98898a849bf6cbea3934233c654877d4b2a372af570976d5` |

All three captures were byte-identical. They show no visible geometry delta, so the acceptance criterion is unproven. Full-screen captures are deliberately not committed: they were used only for this equality/foreground diagnosis, then removed from local staging and the target.

## Why this capture cannot validate the runtime

The current exemplar creates its interactive injector by launching `powershell.exe` as a scheduled task ([launcher](../../../.claude/skills/user-test/scripts/text_stream_portal_exemplar.py#L1455-L1457)). It does not hide that console before synthetic input. The existing re-verification driver documents the resulting foreground-window hazard and its required `ShowWindow(..., SW_HIDE)` precondition ([driver](liveverify-resize-reverify-20260711/resize_injection_driver.py#L92-L105)). The full-screen capture from this run likewise showed the interactive task console in the foreground.

Consequently, scheduled-task `ok=true` only proves that the injector script returned successfully. It does not prove that the transparent overlay received the injected `Ctrl` chords. With a foreground-stealing console and byte-identical screenshots, this result is a validation-harness blocker, not evidence of a new runtime regression.

## Existing work and follow-up

There is no separate open Beads issue whose title, description, or notes track this console-steal condition. The prior OS-injection evidence bead `hud-egn13` is closed through PR #1127 / merge `dc6cc6800737db30bb3a38f8430cccdf085047a2b`; the current hidden-console driver was carried forward by closed `hud-8agm0`, PR #1132 / merge `b937fa6d305c0c6e80fc200058c7aee838e070b0`. `hud-sp8l7` remains in progress; its `gh-pr:923` reference is historical failure evidence only.

Coordinator recommendation (not created by this worker): create and link a P1 harness bug under `hud-sp8l7` to make the exemplar hide its interactive injector console before `SendInput`, or run the same capture from a verified foreground operator session. Resume this bead only when baseline/grow/shrink captures visibly differ and the geometry change is attributable to the delivered hotkeys.
