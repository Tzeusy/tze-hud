# hud-j45j1 YouTube Official-Player Capture Fix

Date: 2026-05-12

## Scope

Worker G investigated the `hud-s0pit` blocker where the approved PR #659
official-player frame-capture adapter found Chrome for YouTube video
`O0FGCxkHM-U`, but `CopyFromScreen` returned five black/static samples with one
hash and `mean_rgb [0, 0, 0]`.

This worker did not stop or reconfigure the production HUD or benchmark
processes. Live checks used temporary `TzeHudYoutubeJ45J1*` scheduled tasks only,
then deleted those tasks. The production HUD remained PID `30228` on ports
`50051` and `9090`; temporary alternate HUD ports `50052` and `9091` were not
used.

## Root Cause

The adapter selected matching official-player windows by largest area. The live
desktop had multiple windows whose titles started with
`tze_hud YouTube source evidence O0FGCxkHM-U`, including a stale/offscreen
Chrome window at `left=-7`, `top=-1481`, `width=2575`, `height=1407`. Sorting by
area selected that stale window, which led to black `CopyFromScreen` samples.

After selecting and moving the on-screen player, the first live proof was
nonblank but static. The remaining issue was that playback was not guaranteed
before sampling.

## Fix

The capture adapter now:

- uses muted autoplay parameters on the official embed URL;
- ranks matching windows by visible primary-screen area before title exactness
  and total area;
- moves/focuses the selected player onto the primary working area before
  capture;
- sends a real center click to request official-player playback before sampling;
- records and validates `selected_window_visible_area` and
  `playback_click_sent`;
- accepts Windows `Set-Content` UTF-8 BOM evidence files for fixture validation.

No yt-dlp/youtube-dl path, direct media URL extraction, download/cache/offline
copy, audio route to HUD, or compositor browser/WebView surface was introduced.

## Live Evidence

Artifact directory:

```text
docs/reports/artifacts/hud-j45j1-youtube-capture-live-20260512T1121/
```

Key artifacts:

- `frame-capture-patched.json`
- `frame-capture-summary.json`
- `youtube-bridge-fixture-validated.json`
- `youtube_source_evidence.html`

The successful live frame-capture proof recorded:

- `captured_frame_count`: `5`
- `distinct_frame_hashes`: `2`
- `selected_window_visible_area`: `2956800`
- `playback_click_sent`: `true`
- selected window: `left=80`, `top=80`, `width=2400`, `height=1232`
- frame 0 `mean_rgb`: `[169.3, 150.14, 116.67]`
- frame 1-4 `mean_rgb`: `[151.19, 134.3, 104.25]`

## Validation Commands

Focused local gates:

```bash
python3 -m py_compile \
  .claude/skills/user-test/scripts/windows_media_ingress_exemplar.py \
  .claude/skills/user-test/scripts/test_windows_media_ingress_exemplar.py

python3 .claude/skills/user-test/scripts/test_windows_media_ingress_exemplar.py

python3 .claude/skills/user-test/tests/test_windows_media_ingress_exemplar.py
```

Live source-capture task used explicit SSH identity, `BatchMode`, and timeouts.
The patched generated PowerShell was run through an interactive scheduled task
because direct SSH PowerShell does not share the visible desktop context:

```bash
PYTHONPATH=.claude/skills/user-test/scripts python3 - <<'PY' | \
  timeout 20 ssh -o BatchMode=yes -o IdentitiesOnly=yes -o ConnectTimeout=8 \
    -i ~/.ssh/ecdsa_home tzeus@tzehouse-windows.parrot-hen.ts.net \
    "powershell -NoProfile -Command \"... create/run TzeHudYoutubeJ45J1PatchedCapture ...\""
import windows_media_ingress_exemplar as w
script = w.build_windows_frame_capture_powershell(
    video_id=w.YOUTUBE_VIDEO_ID,
    sample_count=5,
    sample_interval_s=2.0,
    settle_s=8.0,
)
...
PY
```

Fixture validation of the live JSON:

```bash
python3 .claude/skills/user-test/scripts/windows_media_ingress_exemplar.py \
  youtube-bridge \
  --dry-run \
  --media-ingress-dry-run \
  --frame-capture-fixture-json \
    docs/reports/artifacts/hud-j45j1-youtube-capture-live-20260512T1121/frame-capture-patched.json \
  --output-dir docs/reports/artifacts/hud-j45j1-youtube-capture-live-20260512T1121 \
  --evidence-json \
    docs/reports/artifacts/hud-j45j1-youtube-capture-live-20260512T1121/youtube-bridge-fixture-validated.json
```

## Result

The `hud-j45j1` unblock condition is satisfied for source capture: live Windows
capture from the controlled official YouTube player produced nonblank,
distinct frame evidence through `CopyFromScreen` without prohibited source
paths.

This does not claim HUD runtime ingress or pixel/readback proof. The validated
capture evidence should be applied to the stacked YouTube branch for PR #659;
the separate isolated media-ingress startup issue remains outside this worker's
scope.
