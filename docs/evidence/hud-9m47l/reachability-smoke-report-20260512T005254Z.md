# hud-9m47l TzeHouse Reachability Smoke

Run stamp: `20260512T005254Z`
Host: `tzehouse-windows.parrot-hen.ts.net`
Identity: `~/.ssh/ecdsa_home`

## Result

PASS. TzeHouse is reachable, both SSH users authenticate non-interactively, TCP
ports `22`, `50051`, and `9090` are open, and `TzeHudBenchmarkOverlay` is the
active HUD task for soak strict smoke.

Strict smoke should start with benchmark config, not production config:
`C:\tze_hud\benchmark.toml` via `TzeHudBenchmarkOverlay`.

## Commands

Reachability:

```bash
tailscale status --json | jq -r '.Peer[] | select(.DNSName=="tzehouse-windows.parrot-hen.ts.net.") | {HostName,DNSName,Online,LastSeen,TailscaleIPs}'
timeout 12 tailscale ping -c 1 tzehouse-windows.parrot-hen.ts.net
timeout 12 ssh -i ~/.ssh/ecdsa_home -o IdentitiesOnly=yes -o BatchMode=yes -o ConnectTimeout=8 tzeus@tzehouse-windows.parrot-hen.ts.net "whoami"
timeout 12 ssh -i ~/.ssh/ecdsa_home -o IdentitiesOnly=yes -o BatchMode=yes -o ConnectTimeout=8 hudbot@tzehouse-windows.parrot-hen.ts.net "whoami"
for port in 22 50051 9090; do timeout 6 bash -lc "cat < /dev/null > /dev/tcp/tzehouse-windows.parrot-hen.ts.net/$port" && echo "$port open"; done
```

Benchmark task install/start used a transient non-default PSK from
`TZE_HUD_PSK`/`MCP_TEST_PSK`; the value was not printed or written to this
artifact.

```bash
scripts/windows/install_benchmark_hud_task.ps1
ssh -i ~/.ssh/ecdsa_home -o IdentitiesOnly=yes -o BatchMode=yes -o ConnectTimeout=8 tzeus@tzehouse-windows.parrot-hen.ts.net 'schtasks /Run /TN TzeHudBenchmarkOverlay'
```

MCP smokes:

```bash
python3 .claude/skills/user-test/scripts/publish_zone_batch.py \
  --url http://tzehouse-windows.parrot-hen.ts.net:9090/mcp \
  --psk-env MCP_TEST_PSK \
  --messages-file docs/evidence/external-agent-projection-authority/replay-zone-messages.json \
  --list-zones

python3 .claude/skills/user-test/scripts/publish_widget_batch.py \
  --url http://tzehouse-windows.parrot-hen.ts.net:9090/mcp \
  --psk-env MCP_TEST_PSK \
  --messages-file docs/evidence/external-agent-projection-authority/replay-widget-messages.json \
  --list-widgets \
  --cleanup-on-exit
```

## Artifacts

- `docs/evidence/hud-9m47l/reachability-preflight-20260512T005254Z.txt`
  confirms Tailscale ping, non-interactive SSH for `tzeus` and `hudbot`, and
  TCP ports `22`, `50051`, and `9090`.
- `docs/evidence/hud-9m47l/remote-state-before-20260512T005254Z.json`
  confirms the previous listener was production-configured.
- `docs/evidence/hud-9m47l/benchmark-task-install-20260512T005254Z.txt`
  confirms `TzeHudBenchmarkOverlay` registration for
  `C:\tze_hud\benchmark.toml`.
- `docs/evidence/hud-9m47l/task-run-20260512T005254Z.txt` confirms the
  benchmark scheduled task run attempt succeeded.
- `docs/evidence/hud-9m47l/remote-state-after-20260512T005254Z.json`
  confirms `tze_hud.exe` is bound to `50051` and `9090` with benchmark config.
- `docs/evidence/hud-9m47l/tcp-after-benchmark-20260512T005254Z.txt`
  confirms ports `22`, `50051`, and `9090` are open after launch.
- `docs/evidence/hud-9m47l/mcp-zone-smoke-20260512T005254Z.json` confirms
  `/mcp` zone discovery and `status-bar` publish succeeded.
- `docs/evidence/hud-9m47l/mcp-widget-smoke-20260512T005254Z.json` confirms
  `/mcp` widget discovery, `main-progress` publish, and cleanup succeeded.
