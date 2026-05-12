# hud-s0pit YouTube Bridge Live Validation Blocker

Date: 2026-05-12

## Scope

Worker A validated the stacked PR code by merging `origin/agent/hud-d82p7`
onto `agent/hud-s0pit` after fetching both `origin/agent/hud-o33hj` and
`origin/agent/hud-d82p7`. The validation target was the approved
official-player sidecar path for YouTube video `O0FGCxkHM-U`, entering the HUD
only through `MediaIngressOpen` into `media-pip`.

## Branch And Binary

- Worker branch: `agent/hud-s0pit`
- Stacked source branch merged for validation: `origin/agent/hud-d82p7`
- PR stack: PR #659 (`agent/hud-d82p7`) on PR #658 (`agent/hud-o33hj`)
- Built binary: `target/x86_64-pc-windows-gnu/release/tze_hud.exe`
- SHA-256: `49fa3367528ed49349d292ffe848f70dad1ec05eff1d0d62c224be8f54902754`
- Remote isolated path used during validation: `C:\tze_hud\hud-s0pit\`

## Active Windows State

The existing production HUD was left intact:

- Existing task: `TzeHudOverlay`
- Existing config: `C:\tze_hud\tze_hud.toml`
- Existing ports: `50051` and `9090`
- Existing process observed before validation: `tze_hud.exe` PID `30228`

To avoid disrupting that process, validation used a separate scheduled task:

- Temporary task: `TzeHudHudS0pit`
- Temporary config: `C:\tze_hud\hud-s0pit\windows-media-ingress.toml`
- Temporary ports: `50052` and `9091`
- Temporary process: `tze_hud.exe` PID `55904`

The temporary config was copied from
`app/tze_hud_app/config/windows-media-ingress.toml`, with bundle paths adjusted
on the Windows host to point at the existing `C:\tze_hud\widget_bundles` and
`C:\tze_hud\profiles` directories.

## Commands

Connectivity:

```bash
timeout 12 ssh -o BatchMode=yes -o IdentitiesOnly=yes -o ConnectTimeout=8 \
  -i ~/.ssh/ecdsa_home hudbot@tzehouse-windows.parrot-hen.ts.net whoami

timeout 12 ssh -o BatchMode=yes -o IdentitiesOnly=yes -o ConnectTimeout=8 \
  -i ~/.ssh/ecdsa_home tzeus@tzehouse-windows.parrot-hen.ts.net whoami
```

Build:

```bash
cargo build --release -p tze_hud_app --target x86_64-pc-windows-gnu
sha256sum target/x86_64-pc-windows-gnu/release/tze_hud.exe
```

Deploy isolated binary/config:

```bash
ssh -o BatchMode=yes -o IdentitiesOnly=yes -o ConnectTimeout=8 \
  -i ~/.ssh/ecdsa_home hudbot@tzehouse-windows.parrot-hen.ts.net \
  "powershell -NoProfile -Command \"New-Item -ItemType Directory -Force -Path C:\\tze_hud\\hud-s0pit | Out-Null\""

scp -o BatchMode=yes -o IdentitiesOnly=yes -o ConnectTimeout=8 \
  -i ~/.ssh/ecdsa_home \
  target/x86_64-pc-windows-gnu/release/tze_hud.exe \
  hudbot@tzehouse-windows.parrot-hen.ts.net:C:/tze_hud/hud-s0pit/tze_hud.exe

scp -o BatchMode=yes -o IdentitiesOnly=yes -o ConnectTimeout=8 \
  -i ~/.ssh/ecdsa_home \
  app/tze_hud_app/config/windows-media-ingress.toml \
  hudbot@tzehouse-windows.parrot-hen.ts.net:C:/tze_hud/hud-s0pit/windows-media-ingress.toml
```

Launch isolated HUD task, using the existing HUD PSK only in process memory:

```bash
PSK=$(ssh -o BatchMode=yes -o IdentitiesOnly=yes -o ConnectTimeout=8 \
  -i ~/.ssh/ecdsa_home tzeus@tzehouse-windows.parrot-hen.ts.net \
  'schtasks /Query /TN TzeHudOverlay /V /FO LIST' |
  sed -n 's/.*--psk \([^ ]*\).*/\1/p' | tr -d '\r' | head -n1)

ssh -o BatchMode=yes -o IdentitiesOnly=yes -o ConnectTimeout=8 \
  -i ~/.ssh/ecdsa_home tzeus@tzehouse-windows.parrot-hen.ts.net \
  "schtasks /Create /F /TN TzeHudHudS0pit /SC ONCE /ST 23:59 /IT /RL HIGHEST /TR \"C:\\tze_hud\\hud-s0pit\\tze_hud.exe --config C:\\tze_hud\\hud-s0pit\\windows-media-ingress.toml --window-mode overlay --grpc-port 50052 --mcp-port 9091 --psk <redacted>\""

ssh -o BatchMode=yes -o IdentitiesOnly=yes -o ConnectTimeout=8 \
  -i ~/.ssh/ecdsa_home tzeus@tzehouse-windows.parrot-hen.ts.net \
  "schtasks /Run /TN TzeHudHudS0pit"
```

Live bridge attempt:

```bash
TZE_HUD_PSK="$PSK" python3 .claude/skills/user-test/scripts/windows_media_ingress_exemplar.py youtube-bridge \
  --target tzehouse-windows.parrot-hen.ts.net:50052 \
  --windows-host tzehouse-windows.parrot-hen.ts.net \
  --windows-user tzeus \
  --ssh-key ~/.ssh/ecdsa_home \
  --output-dir docs/reports/artifacts/hud-s0pit-youtube-bridge-live-20260512T1043 \
  --capture-settle-s 8 \
  --capture-frame-samples 5 \
  --capture-frame-interval-s 2 \
  --hold-s 5 \
  --timeout-s 10 \
  --evidence-json docs/reports/artifacts/hud-s0pit-youtube-bridge-live-20260512T1043/youtube-bridge-live.json
```

Additional media-open check:

```bash
TZE_HUD_PSK="$PSK" python3 .claude/skills/user-test/scripts/windows_media_ingress_exemplar.py local-producer \
  --target tzehouse-windows.parrot-hen.ts.net:50052 \
  --agent-id windows-local-media-producer \
  --zone-name media-pip \
  --source-label synthetic-color-bars \
  --hold-s 2 \
  --timeout-s 10 \
  --evidence-json docs/reports/artifacts/hud-s0pit-youtube-bridge-live-20260512T1043/local-producer-media-ingress-open.json
```

## Evidence

Artifacts:

- `docs/reports/artifacts/hud-s0pit-youtube-bridge-live-20260512T1043/youtube_source_evidence.html`
- `docs/reports/artifacts/hud-s0pit-youtube-bridge-live-20260512T1043/frame-capture-live.json`
- `docs/reports/artifacts/hud-s0pit-youtube-bridge-live-20260512T1043/frame-capture-live-2.json`
- `docs/reports/artifacts/hud-s0pit-youtube-bridge-live-20260512T1043/sidecar-capture.log`
- `docs/reports/artifacts/hud-s0pit-youtube-bridge-live-20260512T1043/sidecar-capture-2.log`
- `docs/reports/artifacts/hud-s0pit-youtube-bridge-live-20260512T1043/diagnostic-dry-run/youtube_source_evidence.html`

The sidecar/capture scheduled task did find the controlled official-player
Chrome window:

- title: `tze_hud YouTube source evidence O0FGCxkHM-U - Google Chrome`
- capture API: `System.Drawing.Graphics.CopyFromScreen`
- prohibited paths recorded as unused: download/extraction, cache/offline copy,
  audio route to HUD, saved frame files

However both live frame-capture attempts were invalid proof:

- `captured_frame_count`: `5`
- `distinct_frame_hashes`: `1`
- every sampled frame hash:
  `ae26e335303a6a5e0e44e4995e1c08fc506ecc05f096a8f7773bb5f3314e3052`
- every sampled frame `mean_rgb`: `[0, 0, 0]`
- selected window bounds: `left=-7`, `top=-1481`, `width=2575`,
  `height=1407`

The PR #659 validator correctly rejects this evidence as blank/static, so it
cannot satisfy the required nonblank/distinct frame proof.

## Blockers

This is blocked, not a direct merge candidate.

1. Direct SSH `youtube-bridge` frame capture cannot see the interactive
   sidecar window. The first live attempt failed before `MediaIngressOpen`
   with `No visible official YouTube player window found for O0FGCxkHM-U`.
2. Running launch and capture inside an interactive scheduled task can see the
   official-player Chrome window, but `CopyFromScreen` captures black/static
   pixels from the selected offscreen window.
3. The isolated HUD process listened on `50052/9091`, but resident gRPC timed
   out waiting for `session_established`, and MCP on `9091/mcp` timed out.
4. PR #659 does not yet produce HUD pixel/readback proof that captured YouTube
   frames actually enter `media-pip`; its live evidence path still marks
   `hud_runtime_receives_youtube_frames` false when `MediaIngressOpen` is
   admitted.

## Cleanup

Cleanup performed:

- Stopped temporary HUD PID `55904`
- Deleted scheduled tasks:
  - `TzeHudHudS0pit`
  - `TzeHudYoutubeSidecarS0pit`
  - `TzeHudYoutubeCaptureS0pit`
  - `TzeHudYoutubeCaptureS0pit2`
- Removed remote validation directory `C:\tze_hud\hud-s0pit`
- Verified temporary ports `50052` and `9091` were no longer listening
- Left the original `TzeHudOverlay` process and ports `50051`/`9090` intact

## Result

Blocked. The useful partial result is that the stack was fetched and merged
into this worker branch, the Windows host was reachable, an isolated media
config task could be launched without replacing the production HUD, and the
current capture path produced concrete invalid-frame evidence. The missing
condition is valid nonblank/distinct official-player frame evidence plus HUD
pixel/readback proof that those frames entered `media-pip` through
`MediaIngressOpen`.
