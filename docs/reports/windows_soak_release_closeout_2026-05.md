# Windows Soak And Release Closeout - May 2026

Issue: `hud-ok1y0`
OpenSpec source: `openspec/changes/windows-first-performant-runtime/tasks.md` section 5
Attempted at: 2026-05-09T17:11:02Z
Reference host: `TzeHouse` (`tzehouse-windows.parrot-hen.ts.net`, tailnet `100.87.181.125`)

## Verdict

Blocked before soak execution.

The 60-minute three-agent Windows soak was not started because the reference
Windows host was unavailable from this worker. Both required SSH identities
timed out on port 22, and direct TCP probes to SSH, gRPC, and MCP ports timed
out. Per the issue constraint, no benchmark HUD task was launched, no resident
agents were started, and no release artifact or tag was prepared.

Raw probe evidence:

- `docs/reports/artifacts/windows_soak_release_closeout_2026-05/reachability_probe_20260509T171102Z.json`

## Recovery Update - 2026-05-10

Issue `hud-6imos` was rechecked after the operator reported that the Windows
host was back online. The host is reachable over Tailscale, non-interactive SSH
succeeds for both `hudbot` and `tzeus`, and the `TzeHudOverlay` scheduled task
was started successfully. The running `tze_hud.exe` process listens on gRPC
port `50051` and MCP port `9090`.

MCP validation used `http://tzehouse-windows.parrot-hen.ts.net:9090/mcp` and
passed for all six configured zones. Widget discovery returned the
`main-progress` and `main-status` instances; publish and cleanup passed for
both. The older `main-gauge` fixture was not a valid deployed-instance target
for this runtime config.

Recovery evidence:

- `docs/reports/artifacts/windows_soak_release_closeout_2026-05/reachability_recovery_20260510T022109Z.json`

## Safety Gate

The live-run safety constraint was checked before attempting any soak workload.
PR #641 (`feat: add portal diagnostic input injector [hud-t95gs]`) was open at
the time of the attempt, with merge state `DIRTY`. Because the host itself was
unreachable, no further check for active diagnostic-input or text-stream portal
validation could be performed, and no live HUD state was changed.

## Commands

```bash
ssh -o ConnectTimeout=10 -o ServerAliveInterval=5 -o ServerAliveCountMax=1 \
  -o BatchMode=yes -o IdentitiesOnly=yes -o StrictHostKeyChecking=no \
  -i ~/.ssh/ecdsa_home \
  hudbot@tzehouse-windows.parrot-hen.ts.net "whoami"
```

Result: exit 255, `ssh: connect to host tzehouse-windows.parrot-hen.ts.net port 22: Connection timed out`.

```bash
ssh -o ConnectTimeout=10 -o ServerAliveInterval=5 -o ServerAliveCountMax=1 \
  -o BatchMode=yes -o IdentitiesOnly=yes -o StrictHostKeyChecking=no \
  -i ~/.ssh/ecdsa_home \
  tzeus@tzehouse-windows.parrot-hen.ts.net "whoami"
```

Result: exit 255, `ssh: connect to host tzehouse-windows.parrot-hen.ts.net port 22: Connection timed out`.

```bash
nc -vz -w 5 tzehouse-windows.parrot-hen.ts.net 22
nc -vz -w 5 tzehouse-windows.parrot-hen.ts.net 50051
nc -vz -w 5 tzehouse-windows.parrot-hen.ts.net 9090
```

Result: all three probes timed out against `100.87.181.125`.

## Required Metrics

No new soak metrics exist from this attempt:

| Required closeout metric | Status |
|---|---|
| Frame-time p50/p99/p99.9 under 60-minute three-agent load | Not measured; host unavailable |
| Input latency triple under live load | Not measured; host unavailable |
| Idle/loaded CPU/GPU | Not measured; host unavailable |
| Memory drift over 60 minutes | Not measured; host unavailable |
| Transparent-overlay composite cost | Not measured in this attempt; use the windowed overlay harness once the host is reachable |
| Stale UI / lease cleanup | Not measured; no resident agents started |

## Release Decision

Do not tag a Windows release from this attempt. The release gate still needs a
successful reference-host soak artifact that includes the metrics above and
demonstrates no material jitter, leak, stale UI, or cleanup regression.
