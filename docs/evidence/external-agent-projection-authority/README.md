# External Agent Projection Authority Evidence

This directory contains the headless demo artifact and replay fixtures for the 2026-05-11 external agent projection authority vertical slice.

## Files

- `three-session-demo-plan-20260511.json`: Redacted authority route-plan artifact for three provider-neutral sessions routed to widget, portal, and zone surfaces. The portal route is explicitly `portal_surface=text_stream_raw_tile` with `materialization=resident_raw_tile`, matching the existing text-stream portal raw-tile pilot. Its lifecycle checks cover revoke isolation, expiry cleanup, reconnect fresh-lease behavior, and provider-process supervision with stdio capture disabled.
- `replay-zone-messages.json`: Zone publish payload extracted from the demo plan for `publish_zone_batch.py`.
- `replay-widget-messages.json`: Widget publish payload extracted from the demo plan for `publish_widget_batch.py`.
- `live-replay.sh`: Non-interactive replay harness for the live Windows `/user-test` path.
- `watch-live-replay.sh`: Bounded reachability watcher that polls Tailscale and runs `live-replay.sh` as soon as Windows responds.
- `live-replay-blocked-watch-20260511T025838Z.txt`: Bounded 10-poll reachability watch showing the live replay remained blocked before SSH/MCP/gRPC.
- `live-replay-blocked-20260511T042949Z.txt`: Latest direct live replay attempt after runtime-auth material hardening; still blocked at Tailscale reachability before SSH/MCP/gRPC.
- `live-replay-blocked-20260511T053140Z.txt`: Latest direct live replay attempt after the bounded watcher landed; still blocked at Tailscale reachability before SSH/MCP/gRPC.
- `live-replay-blocked-watch-20260511T055927Z.txt`: Latest bounded 3-poll reachability watch; still blocked at Tailscale reachability before SSH/MCP/gRPC.

## Live Replay

After `tzehouse-windows.parrot-hen.ts.net` is reachable, verify SSH, start `TzeHudOverlay` if needed, and use the MCP `/mcp` endpoint:

```bash
bash docs/evidence/external-agent-projection-authority/live-replay.sh
```

The harness checks Tailscale reachability, SSH for `hudbot` and `tzeus`, MCP `:9090`, gRPC `:50051`, starts `TzeHudOverlay` if SSH works but ports are down, publishes the zone/widget replay payloads, and runs the text-stream portal composer smoke. It expects `TZE_HUD_PSK` to be set locally, but accepts `MCP_TEST_PSK` as a fallback for the MCP publish scripts and exports the same value for the portal smoke. It never writes the value into artifacts.

If the host is reachable but the existing scheduled task starts without a non-default PSK, opt into task recreation before launch:

```bash
RECREATE_TASK_ON_START=1 \
  bash docs/evidence/external-agent-projection-authority/live-replay.sh
```

This recreates `TzeHudOverlay` over SSH with `schtasks /Create /IT /RL HIGHEST`, the local PSK value from `TZE_HUD_PSK` or `MCP_TEST_PSK`, and the same overlay/gRPC/MCP arguments used by the recovery runbook, then starts the task. The PSK value is transmitted only over the SSH session and is not written to repo evidence files.

For a bounded wait-and-run loop:

```bash
MAX_POLLS=30 POLL_INTERVAL_S=60 \
  bash docs/evidence/external-agent-projection-authority/watch-live-replay.sh
```

The watcher exits `20` if the host never becomes reachable. If Tailscale ping succeeds, it runs `live-replay.sh`, writes a timestamped replay log, and returns the replay exit code.

The demo plan's lifecycle checks are headless evidence only; live replay must still verify runtime acceptance, visual behavior, and cleanup against the Windows HUD.

Latest blocker evidence: `live-replay-blocked-watch-20260511T055927Z.txt`
shows a 3-poll window where the watcher exited `20` after Windows remained
offline in Tailscale and `100.87.181.125` timed out/no reply before SSH, MCP,
or gRPC could run.
The longer reachability watch in `live-replay-blocked-watch-20260511T025838Z.txt`
shows a 10-poll window where Windows stayed offline in Tailscale and ports
`22`, `50051`, and `9090` stayed `closed_or_timeout`.

## Local Checks

```bash
bash -n docs/evidence/external-agent-projection-authority/live-replay.sh
bash -n docs/evidence/external-agent-projection-authority/watch-live-replay.sh
jq -e '.zone_messages == input' \
  docs/evidence/external-agent-projection-authority/three-session-demo-plan-20260511.json \
  docs/evidence/external-agent-projection-authority/replay-zone-messages.json
jq -e '.widget_messages == input' \
  docs/evidence/external-agent-projection-authority/three-session-demo-plan-20260511.json \
  docs/evidence/external-agent-projection-authority/replay-widget-messages.json
jq -e '(.route_plans | length) == 3
  and (.zone_messages | length) == 1
  and (.widget_messages | length) == 1
  and (.portal_routes | length) == 1
  and (.portal_routes[0].portal_surface == "text_stream_raw_tile")
  and (.portal_routes[0].materialization == "resident_raw_tile")
  and ([.lifecycle_checks[].accepted] | all)
  and any(.lifecycle_checks[]; .check == "provider_process_supervision"
    and .stdio_capture == "disabled")' \
  docs/evidence/external-agent-projection-authority/three-session-demo-plan-20260511.json
rg -n 'owner[_]token|operator[-]secret|terminal[_]capture|raw[_]keystroke|p[t]y' \
  docs/evidence/external-agent-projection-authority || true
```
