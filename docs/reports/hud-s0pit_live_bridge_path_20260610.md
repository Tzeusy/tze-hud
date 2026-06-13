# hud-s0pit live bridge-path validation, 2026-06-10

## Scope

This run used an exclusive Windows HUD window on `tzehouse-windows.parrot-hen.ts.net` to validate the approved YouTube bridge path for video `O0FGCxkHM-U`.

The validation branch was `agent/hud-3ervz`, updated through `6f3492d6`. The isolated HUD was deployed to `C:\tze_hud\hud-s0pit-rerun` and launched on gRPC `50052` / MCP `9091` so the production `TzeHudOverlay` could be restored afterward.

## Result

Pass for bridge-path admission:

- The official YouTube player sidecar launched through an operator-visible Chrome window.
- The Windows frame-capture adapter captured 5 official-player frames with 4 distinct hashes.
- The selected player window was visible on the primary display with area `2956800`.
- `MediaIngressOpen` admitted the `media-pip` lane on the isolated HUD.
- The admitted stream used `VIDEO_H264_BASELINE`, `stream_epoch=1`, and was held for 20 seconds.
- The bridge stayed video-only and used no `yt-dlp`, `youtube-dl`, direct media URL extraction, download/cache/offline copy, audio route, or compositor browser/WebView surface.

Not complete for final hud-s0pit closeout:

- The evidence still records `hud_runtime_receives_youtube_frames=false`.
- `live_pixel_proof_required=true` remains accurate.
- This run proves official-player frames are available and the HUD admits the bridge lane through `MediaIngressOpen`; it does not prove final HUD pixel/readback delivery.

## Evidence

Primary artifact directory:

`docs/reports/artifacts/hud-s0pit-live-bridge-20260610T032102Z/`

Key files:

- `youtube-bridge-live-attempt5.json`: successful run evidence.
- `pre-exclusive-state.json`: production HUD state before the exclusive window.
- `isolated-media-hud-launch.json`: isolated HUD process and port binding.
- `final-restored-state.json`: production HUD restored on ports `50051` and `9090`.
- `windows-media-ingress.toml`: isolated validation config used for the run.

## Cleanup

The isolated `TzeHudS0pitRerun` task was ended and deleted. `C:\tze_hud\hud-s0pit-rerun` was removed. `TzeHudOverlay` was restarted from its stored scheduled task and verified as `C:\tze_hud\tze_hud.exe` on ports `50051` and `9090`.

No PSK or secret material was written to tracked files; command lines in artifacts are redacted.
