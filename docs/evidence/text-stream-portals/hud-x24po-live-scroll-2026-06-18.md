# hud-x24po live output-scroll sign-off attempt (2026-06-18)

Issue: `hud-x24po`
Original fix: `hud-l6zd0`, PR #914, merge `0783204904871472b7907a29a1afa8a81ad28704`

## Environment

- Host: `tzehouse-windows.parrot-hen.ts.net`
- gRPC target: `tzehouse-windows.parrot-hen.ts.net:50051`
- Worktree commit under test: `0783204904871472b7907a29a1afa8a81ad28704`
- Local and remote `tze_hud.exe` SHA-256: `83ba6fada6ab5cc1ca6e98d75c0588fb4505250cbf14bc3f569df77014f9c957`
- Transcript: `test_results/hud-x24po-text-stream-portal-scroll-20260618T143136Z.json`

The live scheduled-task PSK was recovered from `TzeHudOverlay` for the run and is not recorded here.

## Live Run Result

The text-stream portal exemplar connected to the live HUD and completed `baseline,scroll` against a `3840x2160` scene.

Observed transcript checkpoints:

- `scene:display-area`: `scene_width=3840`, `scene_height=2160`
- `portal:size`: `portal_w=1280`, `portal_h=960`
- `scroll:mount`: `visible_lines=14`, `total_lines=80`
- `scroll:offset`: `scroll_y=40`, `80`, `120`, `160`
- `scroll:append`: appended lines while preserving `scroll_y=160`
- `scroll` completed with `tail_start=71`, `total_lines=85`
- `cleanup:lease-release` completed
- `cleanup_errors=[]`

This proves the merged build accepts the live portal run, advances the output scroll window through multiple offsets, preserves the scroll offset during append, returns to the tail, and cleans up the lease.

## Remaining Gap

The required visual proof that no black or flickering rectangles appear above or below the output viewport is still not complete. An SSH-triggered Windows screenshot attempt failed with:

`Exception calling "CopyFromScreen" with "3" argument(s): "The handle is invalid"`

The captured PNG from that path was `1024x768` and fully transparent, so it is not usable as visual evidence. A manual/operator visual confirmation or runtime-native screenshot/readback path is still required before closing `hud-x24po`.
