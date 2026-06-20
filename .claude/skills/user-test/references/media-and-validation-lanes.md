# Media Ingress & Validation Lanes

Standalone lanes that run outside the core deploy/publish workflow. Referenced
from [../SKILL.md](../SKILL.md).

## Windows Media Ingress Exemplar

Use this lane only with `app/tze_hud_app/config/windows-media-ingress.toml`, which explicitly enables the one-stream `media-pip` surface and grants `windows-local-media-producer` the `media_ingress` capability.

HUD media-ingress proof uses a self-owned/local synthetic video source. It is video-only and does not route audio:

```bash
TZE_HUD_PSK="$PSK" python3 .claude/skills/user-test/scripts/windows_media_ingress_exemplar.py \
  local-producer \
  --target windows-host.example:50051 \
  --agent-id windows-local-media-producer \
  --zone-name media-pip \
  --source-label synthetic-color-bars \
  --hold-s 30 \
  --evidence-json build/windows-media-ingress/local-producer-evidence.json
```

YouTube source evidence is separate from HUD frame-ingress proof. Launch video ID `O0FGCxkHM-U` through the official embedded-player URL; do not bridge raw YouTube frames into the HUD runtime:

```bash
python3 .claude/skills/user-test/scripts/windows_media_ingress_exemplar.py \
  youtube-sidecar \
  --windows-host windows-host.example \
  --windows-user admin-user \
  --ssh-key ~/.ssh/hud-ssh-key \
  --evidence-json build/windows-media-ingress/youtube-source-evidence.json
```

Record the policy boundary before validation:

```bash
python3 .claude/skills/user-test/scripts/windows_media_ingress_exemplar.py \
  policy-review \
  --evidence-json build/windows-media-ingress/policy-review.json
```

The media exemplar must not introduce `yt-dlp`, direct YouTube media URL extraction, downloads, a browser/WebView node inside the compositor, raw YouTube frame bridging, or a YouTube audio route into the HUD runtime.

## D18 Validation Lane (SSH)

`scripts/d18_validation.sh` replaces the suspended GitHub Actions
`real-decode-windows.yml` lane (owner decision 2026-06-13, hud-1aswu.4: no
self-hosted runner exists or is planned). It runs the lane's substantive
checks against tzehouse-windows over SSH from the rig: connectivity gate,
GPU-lock respect (read-only — never removes a lock), GStreamer MSVC SDK
verification, hardware-decoder capability report, and the real-decode
harness step (activation-gated until `tze_hud_runtime::real_decode_windows`
lands — hud-ora8.1 phase 1).

```bash
.claude/skills/user-test/scripts/d18_validation.sh --allow-missing-sdk
```

Exit codes: `0` pass/gated, `1` check failed, `2` GPU busy (live interactive
session holds the lock), `3` SDK not installed (strict mode). The host
defaults to `tailscale ip -4 tzehouse-windows` because MagicDNS is not in
the rig resolver; override with `--win-host`.
