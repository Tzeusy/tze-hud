# hud-i1nrq Media-Ingress HudSession Timeout Diagnosis

Date: 2026-05-12

## Scope

Worker J diagnosed the `hud-s0pit` rerun failure where the isolated
media-ingress HUD appeared to bind alternate ports `50052/9091`, but the
`youtube-bridge` live path timed out waiting for `session_established`.

The investigation was non-disruptive: it did not stop or restart the production
HUD, did not clear the GPU lock, did not mutate Beads state, and did not write
secrets to tracked files.

## Starting Evidence

Committed evidence on main shows:

- `docs/reports/hud-s0pit_youtube_bridge_rerun_blocked_20260512.md`
- `docs/reports/artifacts/hud-s0pit-rerun-youtube-bridge-live-20260512T034408Z/isolated-media-hud-launch.json`
- `docs/reports/artifacts/hud-s0pit-rerun-youtube-bridge-live-20260512T034408Z/youtube-bridge-live.stderr`

The isolated launch evidence recorded:

- process path: `C:\tze_hud\hud-s0pit-rerun\tze_hud.exe`
- command line: `--grpc-port 50052 --mcp-port 9091`
- local listeners on `0.0.0.0:50052` and `0.0.0.0:9091`

The client failure was:

```text
TimeoutError: Timed out waiting for session_established
```

That failure shape means the Python client did not receive either
`SessionEstablished` or `SessionError` from `HudSession`.

## Live Probes

Connectivity to the Windows host was healthy:

```bash
timeout 10s tailscale ping --c 1 tzehouse-windows.parrot-hen.ts.net
timeout 12s ssh -o BatchMode=yes -o IdentitiesOnly=yes -o ConnectTimeout=8 \
  -i ~/.ssh/ecdsa_home hudbot@tzehouse-windows.parrot-hen.ts.net "whoami"
timeout 12s ssh -o BatchMode=yes -o IdentitiesOnly=yes -o ConnectTimeout=8 \
  -i ~/.ssh/ecdsa_home tzeus@tzehouse-windows.parrot-hen.ts.net "whoami"
```

Current production HUD state was left intact:

- process: `C:\tze_hud\tze_hud.exe`, PID `49856`
- listeners: `50051` and `9090`
- worker-host socket probes to `50051` and `9090` succeeded
- worker-host socket probes to `50052` and `9091` timed out because no isolated
  HUD was running during this diagnosis

Evidence path:

```text
docs/reports/artifacts/hud-i1nrq-media-ingress-session-timeout-20260512/firewall-probes.json
```

## Root Cause

Windows Defender Firewall has explicit inbound Block rules for the exact
isolated executable paths used by the failed validation attempts:

```text
C:\tze_hud\hud-s0pit\tze_hud.exe
C:\tze_hud\hud-s0pit-rerun\tze_hud.exe
```

Each path has both TCP and UDP Block rules on Private/Public profiles. The
canonical production executable path has Allow rules:

```text
C:\tze_hud\tze_hud.exe
```

This explains the observed split:

- Windows-local `Get-NetTCPConnection` could see the isolated process listening
  on `50052/9091`.
- Remote gRPC/MCP clients could still time out because the inbound firewall
  blocked the isolated executable path.
- The production HUD on `C:\tze_hud\tze_hud.exe` remained reachable on
  `50051/9090` because that executable path is allowed.

The timeout was therefore not caused by `HudSession` auth/config code and does
not yet indicate a `MediaIngressOpen` admission bug.

## Focused Fix Options

Use one of these before rerunning `hud-s0pit`:

1. Remove the stale inbound Block rules for the isolated validation paths, then
   add explicit temporary Allow rules for the validation executable path and
   alternate ports before launch.
2. Run the isolated validation build from a firewall-allowed executable path,
   while still passing the isolated config and alternate `--grpc-port` /
   `--mcp-port` arguments.

Do not rely on adding an Allow rule while the path-specific Block rules remain
present; Windows Firewall block rules can still win over an allow rule.

## Cleanup

No cleanup was required from this diagnosis. The worker did not create remote
tasks, copy binaries, stop HUD processes, clear locks, or modify firewall state.

## Result

Blocked handoff. The root cause is identified with focused remediation, but the
runtime was not relaunched through the corrected firewall state in this worker
turn. Coordinator can merge this diagnostic evidence, but should not close
`hud-i1nrq` until a rerun proves `session_established` and `MediaIngressOpen` on
the isolated media-ingress lane.
