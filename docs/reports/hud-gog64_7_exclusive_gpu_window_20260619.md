# hud-gog64.7 Exclusive GPU Window Validation

Date: 2026-06-19
Host: TzeHouse (`tzehouse-windows.parrot-hen.ts.net`)
Worker branch: `agent/hud-gog64.7`

## Result

Pass. The operator-approved exclusive Windows GPU window recorded on
`hud-gog64.7` was used for a short interruption of the production HUD. The
isolated media-ingress HUD launched with
`app/tze_hud_app/config/windows-media-ingress.toml` semantics and bound gRPC on
`0.0.0.0:50052` without the prior GPU-lock failure. Production `TzeHudOverlay`
was restored and verified on `0.0.0.0:50051` and `0.0.0.0:9090`.

Evidence directory:

```text
docs/reports/artifacts/hud-gog64.7-exclusive-gpu-window-20260619T111955Z/
```

## Interruption And Restoration Plan

Preconditions checked before interruption:

- Tailscale ping to `tzehouse-windows.parrot-hen.ts.net` succeeded.
- Non-interactive SSH as `tzeus` with `~/.ssh/ecdsa_home` succeeded.
- Production HUD was listening on `50051/9090` as PID `38228`.
- The GPU lock belonged to the production HUD PID.
- Alternate gRPC port `50052` was free.
- MCP port `9091` was already occupied by a local listener, so the isolated
  validation used `9092` instead.
- A non-default PSK was recovered from the existing scheduled task XML without
  printing or storing the value.

Execution plan used:

1. Copy a staged `windows-media-ingress.toml` to `C:\tze_hud\hud-gog64_7\`,
   changing only bundle/profile paths to absolute `C:/tze_hud/...` paths.
2. Stop only the production HUD PID that owned `50051`.
3. Register and start temporary task `TzeHudGog647Media`:
   `C:\tze_hud\tze_hud.exe --config C:\tze_hud\hud-gog64_7\windows-media-ingress.toml --window-mode overlay --bind-all-interfaces --grpc-port 50052 --mcp-port 9092 --psk <redacted>`.
4. Verify `50052` bound.
5. Stop/delete the temporary task and restart `TzeHudOverlay` in a `finally`
   block.
6. Verify production `50051/9090` and MCP `list_zones` after restore.
7. Remove the temporary `C:\tze_hud\hud-gog64_7` directory.

## Evidence Summary

`baseline-production-state.json`:

- production PID: `38228`
- production listeners: `0.0.0.0:50051`, `0.0.0.0:9090`
- GPU lock: `SESSION_TYPE=interactive`, `PID=38228`

`exclusive-window-bind-and-restore.json`:

- `recover-psk`: pass, value omitted
- `baseline-check`: pass
- `stop-production`: pass, `50051_closed=True`
- `start-isolated-media-hud`: pass, `grpc_50052_bound=True`
- isolated media HUD PID: `28592`
- isolated listeners: `0.0.0.0:50052`, `0.0.0.0:9092`
- isolated GPU lock: `SESSION_TYPE=interactive`, `PID=28592`
- `restore-production`: pass, `grpc_50051=True`, `mcp_9090=True`
- restored production PID: `26628`

`post-restore-tcp-probes.txt`:

- `22:open`
- `50051:open`
- `9090:open`
- `50052:closed_or_timeout`
- `9092:closed_or_timeout`

`post-restore-mcp-list-zones.json`:

- MCP `list_zones` returned `count=6` after production restore.

`remote-cleanup.json`:

- `C:\tze_hud\hud-gog64_7` removed.

## Scope Boundary

This bead proves the exclusive-window scheduling/restoration path and the
media-ingress HUD's ability to bind gRPC without GPU-lock failure. It does not
run the authenticated local producer, prove frames enter `media-pip`, or perform
the 10-minute record-only soak. Those remain owned by `hud-gog64.5` and
`hud-gog64.8`.

## Follow-Up

The baseline state included a pre-existing local-only listener on
`127.0.0.1:9091` owned by PID `22384`. It did not block this validation because
`9092` was available, but future media-ingress reruns that assume `9091` should
either clear or avoid that listener.
