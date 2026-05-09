# Cooperative HUD Projection Proof Retry

Date: 2026-05-09
Bead: `hud-ggntn.12`
Worker branch: `agent/hud-ggntn.12`

## Result

Visible Windows desktop capture is still blocked from the available SSH path.
The runtime accepted a live cooperative projection proof tile over resident
gRPC, but `System.Drawing.Graphics.CopyFromScreen` failed with `The handle is
invalid` and produced a fully transparent 1024x768 PNG.

To avoid treating OS desktop capture as the proof source, this pass added and
used a runtime-native readback artifact path:

```bash
cargo run -p render_artifacts --features headless \
  --bin cooperative-projection-readback -- \
  --output docs/evidence/cooperative-hud-projection/hud-ggntn.12-2026-05-09/readback \
  --width 1280 --height 720
```

That command renders the cooperative projection scene through
`HeadlessRuntime`, calls `read_pixels()`, and writes the PPM source frame plus
JSON metadata:

- `readback/cooperative-projection-readback.ppm`: raw RGB frame from runtime readback.
- `readback/cooperative-projection-readback.json`: metadata, telemetry, and sampled pixels.

The reviewable PNG derivative was packaged from the PPM with ImageMagick:

```bash
convert \
  docs/evidence/cooperative-hud-projection/hud-ggntn.12-2026-05-09/readback/cooperative-projection-readback.ppm \
  docs/evidence/cooperative-hud-projection/hud-ggntn.12-2026-05-09/readback/cooperative-projection-readback.png
```

## Live Windows Context

Connectivity and runtime checks passed:

- `ssh -i ~/.ssh/ecdsa_home hudbot@tzehouse-windows.parrot-hen.ts.net "whoami"` returned `tzehouse\hudbot`.
- `ssh -i ~/.ssh/ecdsa_home tzeus@tzehouse-windows.parrot-hen.ts.net "whoami"` returned `tzehouse\tzeus`.
- `query user` showed `tzeus` active on console session 2.
- `tze_hud.exe` was running in session 2.
- `Test-NetConnection 127.0.0.1 -Port 50051` passed.
- `Test-NetConnection 127.0.0.1 -Port 9090` passed.

The scheduled task PSK was extracted from `TzeHudOverlay` arguments and stripped
of carriage returns before use. The PSK is not stored in these artifacts.

## Evidence

Live gRPC proof:

- `logs/live-projection-proof-transcript.json`: resident gRPC session established as `agent-alpha`, lease granted, projection tile created at 3840x2160 display coordinates, and proof text rendered.
- `logs/post-cleanup-scene-snapshot.json`: interim snapshot showed the proof tile still present after the first release attempt timed out.
- `logs/final-cleanup-scene-snapshot.json`: final snapshot after explicit lease release showed `tiles=0`, no proof text, and no `agent-alpha` references.

Desktop capture blocker:

- `logs/windows-screenshot-command.json`: `CopyFromScreen` failure details.
- `screenshots/windows-desktop-capture.png`: transparent 1024x768 output from the failed desktop capture path.

Runtime-native proof:

- `readback/cooperative-projection-readback.json`: `tile_count=1`, `node_count=1`, `active_leases=1`, sampled background pixels `[63,63,89,255]`, and projection tile pixels `[56,69,89,255]`.
- `readback/cooperative-projection-readback.ppm`: readback source frame.
- `readback/cooperative-projection-readback.png`: reviewable image derivative.

## Cleanup

The first release request timed out waiting for a `LeaseResponse`, and the
follow-up snapshot confirmed the tile remained. A second explicit
`LeaseRelease` using the lease ID from the snapshot was acknowledged. Final
snapshot verification showed no remaining proof tile or projection lease output.

## Residual Blocker

The Windows interactive desktop may be active, but SSH-triggered
`CopyFromScreen` is not a reliable proof path for the visible overlay. Future
visible proof still needs either operator-assisted unlocked desktop capture or a
runtime/windowed capture API that reads the HUD surface directly from inside the
running Windows process.
