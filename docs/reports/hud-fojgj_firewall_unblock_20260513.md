# hud-fojgj Firewall Unblock

Date: 2026-05-13

## Scope

`hud-fojgj` authorized the Windows firewall/runtime mutation needed after
`hud-i1nrq` identified path-specific inbound firewall blocks for isolated
media-ingress validation executables.

This pass changed only Windows Defender Firewall rules for the isolated
validation executable paths. It did not stop or restart the production HUD,
copy binaries, clear the GPU lock, or change tracked secrets.

## Preflight

Connectivity checks passed for the Windows host:

- `tailscale ping --c 1 tzehouse-windows.parrot-hen.ts.net`
- SSH as `hudbot` with `~/.ssh/ecdsa_home`
- SSH as `tzeus` with `~/.ssh/ecdsa_home`

The production HUD remained active:

- executable: `C:\tze_hud\tze_hud.exe`
- PID observed: `49856`
- command line: `--config C:\tze_hud\benchmark.toml --window-mode overlay --grpc-port 50051 --mcp-port 9090`

## Firewall Change

The authorized unblock command removed inbound rules scoped to these isolated
validation executable paths, then added inbound TCP Allow rules for ports
`50052` and `9091`:

- `C:\tze_hud\hud-s0pit\tze_hud.exe`
- `C:\tze_hud\hud-s0pit-rerun\tze_hud.exe`

The `netsh` deletion phase returned `No rules match the specified criteria`,
which means the exact stale Block rules were no longer present by that point.
The add phase returned `Ok.` for both validation paths.

## Verification

Post-change firewall inventory for both isolated validation paths showed inbound
Allow rules on Private/Public profiles and no Block rules:

```text
C:\tze_hud\hud-s0pit\tze_hud.exe
  Allow TCP Any, Private/Public
  Allow UDP Any, Private/Public

C:\tze_hud\hud-s0pit-rerun\tze_hud.exe
  Allow TCP Any, Private/Public
  Allow UDP Any, Private/Public
```

The production HUD process was still the only observed `tze_hud.exe` process.

## Result

`hud-fojgj` is unblocked. The dependent media-ingress rerun should proceed via
`hud-i1nrq` / `hud-s0pit`; the next validation still needs to respect the GPU
single-HUD guard, because firewall reachability and GPU ownership are separate
gates.
