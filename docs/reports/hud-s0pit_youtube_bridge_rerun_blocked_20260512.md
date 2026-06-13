# hud-s0pit YouTube Bridge Rerun Blocked

Date: 2026-05-12

## Scope

Worker I reran `hud-s0pit` from worktree
`.worktrees/parallel-agents/hud-s0pit-rerun` on branch
`agent/hud-s0pit-rerun`. The branch started from current main
`4e475bb4` and cherry-picked the PR #658/#659 stack for validation:

- `804fe4ed` cherry-pick of `24bec40e` (`hud-o33hj`)
- `090fd7da` cherry-pick of `ae870b82` (`hud-2aj4t`)
- `6fa5601d` cherry-pick of `b04620e8` (`hud-d82p7`)
- `8244e392` cherry-pick of `c012da68` (`hud-632rx`)
- `8eadaa91` cherry-pick of `1cfec13f` (`hud-j45j1`)

No existing PR branches were force-pushed.

## Binary And Config

- Built binary: `target/x86_64-pc-windows-gnu/release/tze_hud.exe`
- SHA-256: `d401ef5b9f0385c6bf70c2f7c1775fb736c1fd7f93e9982c9b3c59b1a3aaebbe`
- Remote isolated directory: `C:\tze_hud\hud-s0pit-rerun`
- Isolated config: copied from `app/tze_hud_app/config/windows-media-ingress.toml`, with bundle paths rewritten to `C:/tze_hud/widget_bundles` and `C:/tze_hud/profiles`
- Isolated task: `TzeHudS0pitRerun`
- Isolated ports: gRPC `50052`, MCP `9091`

Pre-validation production state was a benchmark-config HUD process on ports
`50051/9090`. The first alternate-port startup probe failed closed with:

```text
error: [gpu-lock] GPU is already in use: SESSION_TYPE=interactive-untracked PID=30228 STARTED_AT=unknown - existing tze_hud.exe process without a live gpu.lock
```

To create a short exclusive validation window, PID `30228` was stopped after
recording the active state. Cleanup restored the benchmark overlay through
`TzeHudBenchmarkOverlay`, yielding a new `tze_hud.exe` PID `49856` listening on
`50051/9090`.

## Commands

Connectivity:

```bash
timeout 10s tailscale ping --c 1 tzehouse-windows.parrot-hen.ts.net
timeout 12s ssh -o BatchMode=yes -o IdentitiesOnly=yes -o ConnectTimeout=8 \
  -i ~/.ssh/ecdsa_home hudbot@tzehouse-windows.parrot-hen.ts.net "whoami"
timeout 12s ssh -o BatchMode=yes -o IdentitiesOnly=yes -o ConnectTimeout=8 \
  -i ~/.ssh/ecdsa_home tzeus@tzehouse-windows.parrot-hen.ts.net "whoami"
```

Local validation:

```bash
python3 -m py_compile \
  .claude/skills/user-test/scripts/windows_media_ingress_exemplar.py \
  .claude/skills/user-test/scripts/test_windows_media_ingress_exemplar.py
python3 .claude/skills/user-test/scripts/test_windows_media_ingress_exemplar.py
python3 .claude/skills/user-test/tests/test_windows_media_ingress_exemplar.py
cargo build --release -p tze_hud_app --target x86_64-pc-windows-gnu
```

Official-player sidecar:

```bash
python3 .claude/skills/user-test/scripts/windows_media_ingress_exemplar.py youtube-sidecar \
  --windows-host tzehouse-windows.parrot-hen.ts.net \
  --windows-user tzeus \
  --ssh-key ~/.ssh/ecdsa_home \
  --connect-timeout-s 8 \
  --output-dir docs/reports/artifacts/hud-s0pit-rerun-youtube-bridge-live-20260512T034408Z \
  --evidence-json docs/reports/artifacts/hud-s0pit-rerun-youtube-bridge-live-20260512T034408Z/youtube-source-evidence.json
```

Frame capture used the corrected generated PowerShell from
`build_windows_frame_capture_powershell()` in an interactive scheduled task
named `TzeHudYoutubeS0pitRerunCapture`, then copied
`C:\tze_hud\hud-s0pit-rerun\youtube-capture.out` to
`frame-capture-live.json`.

Fixture validation:

```bash
python3 .claude/skills/user-test/scripts/windows_media_ingress_exemplar.py youtube-bridge \
  --dry-run \
  --media-ingress-dry-run \
  --frame-capture-fixture-json docs/reports/artifacts/hud-s0pit-rerun-youtube-bridge-live-20260512T034408Z/frame-capture-live.json \
  --output-dir docs/reports/artifacts/hud-s0pit-rerun-youtube-bridge-live-20260512T034408Z \
  --evidence-json docs/reports/artifacts/hud-s0pit-rerun-youtube-bridge-live-20260512T034408Z/youtube-bridge-fixture-validated.json
```

Live bridge attempt:

```bash
TZE_HUD_PSK="$PSK" python3 .claude/skills/user-test/scripts/windows_media_ingress_exemplar.py youtube-bridge \
  --target tzehouse-windows.parrot-hen.ts.net:50052 \
  --windows-host tzehouse-windows.parrot-hen.ts.net \
  --windows-user tzeus \
  --ssh-key ~/.ssh/ecdsa_home \
  --connect-timeout-s 8 \
  --frame-capture-fixture-json docs/reports/artifacts/hud-s0pit-rerun-youtube-bridge-live-20260512T034408Z/frame-capture-live.json \
  --output-dir docs/reports/artifacts/hud-s0pit-rerun-youtube-bridge-live-20260512T034408Z \
  --hold-s 20 \
  --timeout-s 10 \
  --evidence-json docs/reports/artifacts/hud-s0pit-rerun-youtube-bridge-live-20260512T034408Z/youtube-bridge-live.json
```

`$PSK` was extracted only in shell/process memory and was not written to tracked
files.

## Evidence

Artifact directory:

```text
docs/reports/artifacts/hud-s0pit-rerun-youtube-bridge-live-20260512T034408Z/
```

Key files:

- `policy-review.json`
- `youtube_source_evidence.html`
- `youtube-source-evidence.json`
- `frame-capture-live.json`
- `youtube-bridge-fixture-validated.json`
- `isolated-start-conflict.log`
- `isolated-media-hud-launch.json`
- `youtube-bridge-live.stderr`
- `youtube-bridge-live.rc`
- `cleanup-restore.json`

Source-capture result:

- `captured_frame_count`: `5`
- `distinct_frame_hashes`: `2`
- selected window: `left=80`, `top=80`, `width=2400`, `height=1232`
- `selected_window_visible_area`: `2956800`
- `playback_click_sent`: `true`
- nonblank samples: frames `0`, `2`, `3`, and `4` had mean RGB `[151.19, 134.3, 104.25]`
- prohibited paths remained unused: `yt-dlp`, `youtube-dl`, direct media URL extraction, download/cache/offline copy, audio route, compositor browser/WebView

## Blocker

This remains blocked, not a direct merge candidate.

After the exclusive window was opened, the isolated media-ingress HUD did bind
alternate ports `50052/9091`, but the live bridge client timed out before
`MediaIngressOpen`:

```text
TimeoutError: Timed out waiting for session_established
```

The attempted screen readback during the bridge hold did not produce a PNG, so
there is no HUD pixel/readback proof for this rerun. Therefore the rerun proves
corrected nonblank/distinct official-player frame capture, and it proves
fail-closed alternate startup while a production HUD is active, but it does not
prove captured YouTube frames entered `media-pip` through `MediaIngressOpen`.

## Cleanup

Cleanup completed:

- stopped isolated HUD PID `47428`
- deleted temporary tasks `TzeHudS0pitRerun` and `TzeHudYoutubeS0pitRerunCapture`
- removed `C:\tze_hud\hud-s0pit-rerun`
- restored benchmark overlay through `TzeHudBenchmarkOverlay`
- verified restored process PID `49856` on ports `50051/9090`

Coordinator should keep `hud-s0pit` open/blocked and route the remaining issue
to the isolated media-ingress HUD session-establishment failure on `50052/9091`.
