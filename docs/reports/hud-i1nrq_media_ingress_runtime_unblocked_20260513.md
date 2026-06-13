# hud-i1nrq Media Ingress Runtime Unblocked

Date: 2026-05-13

## Scope

This retry resumed `hud-i1nrq` after TzeHouse briefly returned to the tailnet.
The worker reused branch `agent/hud-i1nrq`, which already contained current
`main` plus the PR #659 YouTube bridge stack.

The previous interrupted exclusive-window handoff had left no HUD process
running. On recovery, `TzeHudBenchmarkOverlay` was still registered and was
started first to restore the host to a known benchmark state:

```text
PID 51712
C:\tze_hud\tze_hud.exe
--config C:\tze_hud\benchmark.toml --window-mode overlay --grpc-port 50051 --mcp-port 9090
```

## Isolated Runtime Proof

The corrected retry used a unique temporary task instead of deleting a possibly
absent `TzeHudS0pitRerun` task:

```text
Task: TzeHudI1nrqRetry012749
Executable: C:\tze_hud\hud-s0pit-rerun\tze_hud.exe
Config: C:\tze_hud\hud-s0pit-rerun\windows-media-ingress.toml
Ports: 50052 / 9091
```

The isolated runtime started successfully:

- stopped benchmark HUD PID `51712`
- started isolated HUD PID `27312`
- verified listeners on `0.0.0.0:50052` and `0.0.0.0:9091`

The local producer then established `HudSession` and completed
`MediaIngressOpen` against the isolated runtime:

```text
namespace=windows-local-media-producer
caps=['media_ingress', 'publish_zone:media-pip', 'read_telemetry']
scene display area: 3840x2160
stream_epoch=1
selected_codec=VIDEO_H264_BASELINE
close_reason=AGENT_CLOSED
```

This satisfies the `hud-i1nrq` unblock condition: the isolated
media-ingress runtime on alternate ports accepted `HudSession` and admitted the
approved `media-pip` stream via `MediaIngressOpen`.

## Remaining YouTube Bridge Blocker

The subsequent `youtube-bridge` command did not reach its own
`MediaIngressOpen` call. It failed before frame capture completed:

```text
The command line is too long.
```

Root cause: the direct SSH `powershell -EncodedCommand <large script>` frame
capture transport exceeds the Windows/OpenSSH command-line length limit. The
next YouTube bridge worker should copy the generated frame-capture script to
Windows and run it via an interactive scheduled task, or otherwise use a
chunked/stdin/file transport. The path must continue to use the
operator-visible official-player window and must not use `yt-dlp`,
`youtube-dl`, direct media URL extraction, download/cache/offline copy, audio
routing, or a compositor browser/WebView surface.

## Host Stability

After the YouTube bridge failure, TzeHouse became unreachable again:

```text
tailscale ping tzehouse-windows.parrot-hen.ts.net -> timed out/no reply
ssh tzeus@tzehouse-windows.parrot-hen.ts.net -> port 22 timeout
```

Because SSH timed out, this worker could not verify cleanup, delete the
temporary isolated task, or restore `TzeHudBenchmarkOverlay`. Follow the
existing TzeHouse recovery runbook before any next live attempt; first verify or
restore the benchmark HUD on `50051/9090`, then run the YouTube frame-capture
transport fix.

## Evidence

Artifact directory:

```text
docs/reports/artifacts/hud-i1nrq-youtube-bridge-retry-20260513T012749Z/
```

Key files:

- `isolated-media-hud-launch.json`
- `local-producer-media-ingress-open.json`
- `local-producer.stdout`
- `youtube-bridge-live.stderr`
- `youtube-bridge-live.rc`
- `retry-summary.json`

No PSK or secret material was written to tracked files.
