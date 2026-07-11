# YouTube frame-bridge cluster — scoping (NOT implemented)

- Date: 2026-06-20
- Beads: `hud-d82p7`, `hud-o33hj`, `hud-s0pit`, blocker `hud-t1900`
- Decision (operator, 2026-06-20): **scope the blockers, do not implement this session.**

## What the cluster wants

Bridge frames from an operator-visible official YouTube player (video
`O0FGCxkHM-U`) into the HUD `media-pip` zone **only** through `MediaIngressOpen`,
then capture live Windows HUD pixel/readback proof.

- `hud-o33hj` — implement the approved narrow Windows-only raw-frame bridge from
  the official player sidecar into media ingress (operator/maintainer approval
  recorded 2026-05-12).
- `hud-d82p7` — implement the Windows-side frame-capture adapter (PR #658 review
  found the lane still lacks valid non-dry-run frame proof).
- `hud-s0pit` — capture live Windows HUD pixel/readback proof once a benchmark
  HUD / exclusive validation window is available.

## Blockers (why it cannot close yet)

1. **Policy gate.** The user-test exemplar records
   `raw_youtube_frame_bridge = "blocked_pending_policy_approval"` and an explicit
   prohibited-paths list (`yt-dlp`, `youtube-dl`, `googlevideo.com`,
   `videoplayback`, `download`, `direct media url`). The HUD lane is currently
   limited to a **self-owned/local synthetic** source; the YouTube lane launches
   the official embedded player as *source evidence only* and does NOT bridge
   frames. Bridging real YouTube frames needs a separate policy review to clear
   the `blocked_pending_policy_approval` state.
2. **`hud-t1900` (Windows Chrome Error 153).** Windows Chrome browser state
   causes YouTube Error 153 even with a valid http origin; this blocks live
   validation of the official-player sidecar and must be fixed first.
3. **Real adapter code.** No Windows frame-capture adapter exists yet
   (`hud-d82p7`); this is genuine implementation work (capture the official
   player surface → feed raw frames through `MediaIngressOpen` into `media-pip`,
   preserving player/control visibility), not a validation rerun.
4. **Exclusive GPU window.** `hud-s0pit`'s pixel/readback proof needs an
   exclusive validation window (the media-ingress isolated-HUD pattern proven in
   `hud-8dht5`/`hud-gog64.8` this session is reusable here once the above clear).

## Recommended order when picked up

1. Clear the policy gate (review → approve compliant bridge, or confirm the
   2026-05-12 approval covers the concrete implementation).
2. Fix `hud-t1900` (Chrome Error 153) so the official player renders.
3. Implement `hud-o33hj`/`hud-d82p7` adapter (MediaIngressOpen-only path).
4. Run `hud-s0pit` live pixel proof using the isolated-media-HUD harness from
   `docs/evidence/media-ingress/hud-8dht5/` (operator-disabled → enabled config,
   alt ports, user-test PSK).

## Reusable assets proven this session

- Isolated media-HUD launch + admission proof: `start-disabled-media-hud.ps1`,
  operator-disabled/enabled TOMLs, `--expect-reject-code` exemplar flag.
- Resource sampling harness (`windows_media_resource_sampler.py`) for the live
  pixel-proof run's CPU/GPU/mem capture.
