# Windows Host Reachability Restoration Attempt - hud-6imos

Issue: `hud-6imos`
Attempted at: 2026-05-09T17:19:45Z
Local report date: 2026-05-10 Asia/Singapore (UTC+08:00)
Reference host: `TzeHouse` (`tzehouse-windows.parrot-hen.ts.net`, tailnet `100.87.181.125`)

## Verdict

Blocked awaiting operator or privileged host-side recovery.

The reference Windows host still resolves in tailnet DNS, but Tailscale reports
the Windows node offline and all direct probes from this worker time out. Both
required SSH identities (`hudbot` and `tzeus`) fail before authentication because
port 22 is unreachable. HUD gRPC (`50051`) and MCP (`9090`) also time out, so no
gRPC or MCP validation was attempted.

Raw evidence:

- `docs/reports/artifacts/hud-6imos-windows-reachability-20260510/reachability_probe_20260509T171945Z.json`

## Safety Context

PR #641 and PR #642 were both open with merge state status `BLOCKED` during this
attempt, matching the artifact's `merge_state_status` field. To avoid disrupting
their review workers or live validation state, the
probe stayed read-only: no HUD process was killed, launched, restarted, or
published to.

## Probe Results

| Check | Result |
|---|---|
| DNS | `tzehouse-windows.parrot-hen.ts.net` resolves to `100.87.181.125` |
| Tailscale status | Node present as `TzeHouse`, Windows, `Online: false`, `LastSeen: 2026-05-09T16:05:09.1Z` |
| Tailscale ping | timed out against `100.87.181.125` |
| ICMP ping | 3 transmitted, 0 received |
| TCP 22 | timed out |
| TCP 50051 | timed out |
| TCP 9090 | timed out |
| SSH `hudbot` | timed out connecting to port 22 |
| SSH `tzeus` | timed out connecting to port 22 |

## Local Recovery Assessment

The adjacent `tzehouse-synology.parrot-hen.ts.net` node is reachable on SSH port
22, which indicates the local tailnet path is generally working. However, this
worker does not have key access to that host as `hudbot`, `tzeus`, `tze`,
`tzeusy`, or `admin`, and the repository docs do not define a Wake-on-LAN or
Synology-mediated recovery command for the Windows machine. Without SSH to the
Windows host, a documented out-of-band wake path, or privileged access to an
adjacent machine, this session cannot safely restore reachability.

## Required Operator Action

Restore the Windows node itself before unblocking the dependent validation and
soak work:

1. Confirm `TzeHouse` is powered on and connected to the network.
2. Confirm Tailscale is running and the node is online in the tailnet.
3. Confirm Windows OpenSSH is running and accepts the existing `~/.ssh/ecdsa_home`
   key for both `hudbot` and `tzeus`.
4. Confirm the HUD runtime is started with gRPC on `50051` and MCP on `9090`.
5. Re-run the non-interactive probes in the artifact before starting soak or
   live validation work.

## Blocked Work

This continues to block:

- `hud-ok1y0` - Windows perf soak and release closeout
- `hud-eeejt` - retained widget raster benchmark on Windows reference hardware
- `hud-hfcoe` - diagnostic-input live transcript capture
