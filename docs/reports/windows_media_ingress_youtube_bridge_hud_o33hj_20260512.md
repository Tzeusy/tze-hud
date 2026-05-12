# Windows Media Ingress YouTube Bridge Report - hud-o33hj

Date: 2026-05-12
Issue: `hud-o33hj`
Change: `openspec/changes/windows-media-ingress-exemplar/`
Video ID: `O0FGCxkHM-U`
Approved zone: `media-pip`

## Bridge Path

The approved bridge path is:

```text
operator-visible-official-player-window-capture-to-media-ingress-open
```

The intended live validation shape is:

```bash
python3 .claude/skills/user-test/scripts/windows_media_ingress_exemplar.py youtube-bridge \
  --windows-host tzehouse-windows.parrot-hen.ts.net \
  --windows-user tzeus \
  --ssh-key ~/.ssh/ecdsa_home \
  --target tzehouse-windows.parrot-hen.ts.net:50051 \
  --evidence-json docs/reports/artifacts/windows_media_ingress_hud_o33hj/youtube-bridge-live.json
```

The bridge must launch the official YouTube embed/player sidecar for
`https://www.youtube.com/embed/O0FGCxkHM-U`, keeps the player/control surface
operator-visible, and feed sidecar-captured video frames through the dedicated
bridge agent's video-only `MediaIngressOpen` lane into `media-pip`.

The frame-capture adapter is now implemented as
`windows-visible-player-copyfromscreen-frame-capture`. It runs on Windows
locally or over SSH, finds the visible official-player browser window, captures
multiple in-memory screenshots with `System.Drawing.Graphics.CopyFromScreen`,
hashes/sample-checks those frames, and reports JSON evidence without writing
captured frame files. The helper still does not claim HUD pixel proof by itself:
after the adapter captures frames and `MediaIngressOpen` admits the bridge lane,
an exclusive validation window must attach pixel/readback or screenshot evidence
from `media-pip`.

## Evidence

Artifact directory:

```text
docs/reports/artifacts/windows_media_ingress_hud_o33hj/
```

Artifacts produced in this worker:

- `policy-review.json`: machine-readable approved boundary and prohibited paths.
- `youtube-bridge-dry-run.json`: bridge path evidence with `MediaIngressOpen` named as the only HUD entrypoint.
- `youtube_source_evidence.html`: generated official-player sidecar HTML.

The dry-run artifact intentionally records:

- `media_ingress_open_attempted: false`
- `media_ingress_open_admitted: false`
- `hud_runtime_receives_youtube_frames: false`

No live frame-ingress proof is claimed from this worker because another worker is
currently running the `hud-nfl7n` Windows release soak against the benchmark HUD.
Starting or replacing the media-ingress HUD task would risk disrupting that
runtime, so live validation was deferred.

## Local Verification

```bash
python3 -m py_compile \
  .claude/skills/user-test/scripts/windows_media_ingress_exemplar.py \
  .claude/skills/user-test/scripts/test_windows_media_ingress_exemplar.py

python3 .claude/skills/user-test/scripts/test_windows_media_ingress_exemplar.py

python3 .claude/skills/user-test/scripts/windows_media_ingress_exemplar.py policy-review \
  --evidence-json docs/reports/artifacts/windows_media_ingress_hud_o33hj/policy-review.json

python3 .claude/skills/user-test/scripts/windows_media_ingress_exemplar.py youtube-bridge \
  --dry-run \
  --media-ingress-dry-run \
  --output-dir docs/reports/artifacts/windows_media_ingress_hud_o33hj \
  --evidence-json docs/reports/artifacts/windows_media_ingress_hud_o33hj/youtube-bridge-dry-run.json
```

Results: all passed locally. The Python unit test covers the approved bridge
decision, official-player HTML shape, dry-run evidence, bridge parser defaults,
frame-capture fixture validation, cached-frame rejection, fixture-backed
adapter dry-run evidence, and approved-zone rejection.

## Boundary Check

This change does not introduce:

- `yt-dlp` or `youtube-dl`
- direct media URL extraction
- media download, cache, or offline copy
- audio routing into the HUD runtime
- browser/WebView hosting inside the compositor

The local/synthetic fallback proof remains available through:

```bash
python3 .claude/skills/user-test/scripts/windows_media_ingress_exemplar.py local-producer \
  --target tzehouse-windows.parrot-hen.ts.net:50051 \
  --agent-id windows-local-media-producer \
  --zone-name media-pip
```

## Remaining Live Step

Live Windows validation still needs an exclusive validation window after
`hud-nfl7n` releases the benchmark HUD. Run the `youtube-bridge` command without
`--media-ingress-dry-run` against a HUD launched with
`app/tze_hud_app/config/windows-media-ingress.toml`; it will capture visible
official-player frames before opening `MediaIngressOpen`. Attach pixel/readback
or screenshot evidence proving bridged video frames entered `media-pip` through
`MediaIngressOpen`.
