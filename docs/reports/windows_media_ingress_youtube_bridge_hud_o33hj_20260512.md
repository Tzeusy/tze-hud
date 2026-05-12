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
bridge agent's video-only `MediaIngressOpen` lane into `media-pip`. The current
helper intentionally fails before HUD auth when run without
`--media-ingress-dry-run`, because the Windows frame-capture adapter has not yet
landed and the local/synthetic producer must not be substituted for YouTube
frame proof.

## Evidence

Artifact directory:

```text
docs/reports/artifacts/windows_media_ingress_hud_o33hj/
```

Artifacts produced in this worker:

- `policy-review.json`: machine-readable approved boundary and prohibited paths.
- `youtube-bridge-dry-run.json`: bridge path evidence with `MediaIngressOpen` named as the only HUD entrypoint and the missing live adapter called out.
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
and approved-zone rejection.

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

## Blocked Live Step

Live Windows validation still needs both the Windows frame-capture adapter and an
exclusive validation window after `hud-nfl7n` releases the benchmark HUD. After
the adapter exists, run the `youtube-bridge` command without
`--media-ingress-dry-run` against a HUD launched with
`app/tze_hud_app/config/windows-media-ingress.toml`, then attach pixel/readback
or screenshot evidence proving bridged video frames entered `media-pip` through
`MediaIngressOpen`.
