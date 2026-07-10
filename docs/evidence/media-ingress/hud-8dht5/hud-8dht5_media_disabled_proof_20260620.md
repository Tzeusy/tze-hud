# hud-8dht5 / hud-eqn3m — Authenticated MEDIA_DISABLED admission-rejection proof

- Date: 2026-06-20
- Beads: `hud-8dht5` (host-unreachable blocker), `hud-eqn3m` (prove disabled-start path)
- Host: `windows-host.example` (Tailscale `198.51.100.125`)
- Driver: `/user-test` skill, `windows_media_ingress_exemplar.py local-producer`

## Summary

The authenticated `MEDIA_DISABLED` admission rejection is proven. The original
`hud-eqn3m` defect — starting the media HUD with `operator_disabled=true` did not
bind gRPC `50052`, so the producer saw a transport/connection failure instead of
an authenticated admission rejection — does **not** reproduce on the current
`main` build. With `media_ingress.enabled=true` + `operator_disabled=true`, the
isolated media HUD **binds 50052** and returns a structured, authenticated
`MEDIA_DISABLED` rejection.

No production source change was required: the rejection logic
(`crates/tze_hud_protocol/src/session_server/media.rs:124`, `reject_code =
"MEDIA_DISABLED"` when `!enabled || operator_disabled`) is already in `main`.
Branch `origin/agent/hud-eqn3m` adds the regression test
(`crates/tze_hud_protocol/src/session_server/tests.rs`) plus this live proof.

## Reachability (clears the hud-8dht5 blocker)

- `tailscale ping` → pong 4 ms to `198.51.100.125`.
- TCP `22`, `9090`, `50051` OPEN; `50052` initially closed (prod HUD does not
  bind the media port) — exactly the pre-proof condition.

## Isolated HUD launch

- Production overlay (`TzeHudOverlay`, PID 30392) stopped to release the GPU
  lock + ports (this host is not production; downtime acceptable).
- Isolated task `TzeHud8dht5Media` launched:
  `tze_hud.exe --config C:\tze_hud\hud-8dht5\windows-media-ingress-operator-disabled.toml
  --window-mode overlay --bind-all-interfaces --grpc-port 50052 --mcp-port 9092 --psk <user-test PSK>`
- Bound listeners (`isolated-listeners.json`): gRPC `50052` + MCP `9092`, PID 38828.
- **PSK:** the isolated HUD was launched with the dedicated user-test PSK
  (`MCP_TEST_PSK`), and the producer authenticated with the same key — no
  dependence on the production scheduled-task PSK. (Note: pass the PSK
  **unquoted** through SSH→cmd→PowerShell; single quotes are not stripped by the
  Windows command-line parser and corrupt the value → AUTH_FAILED.)

## Producer result (`media-disabled-proof.json`)

```json
{
  "authenticated": true,
  "admitted": false,
  "reject_code": "MEDIA_DISABLED",
  "reject_reason": "media ingress is disabled by runtime configuration",
  "expected_reject_code": "MEDIA_DISABLED",
  "target": "windows-host.example:50052",
  "zone_name": "media-pip",
  "video_only": true
}
```

Producer exit code: `0` (the new `--expect-reject-code MEDIA_DISABLED` flag
asserts: session authenticates AND admission is rejected with exactly that code;
a clean admission or a different/absent rejection fails).

## Harness change

- `hud_grpc_client.py`: typed `MediaIngressRejected(RuntimeError)` carrying
  `reject_code` / `reject_reason`.
- `windows_media_ingress_exemplar.py`: `local-producer --expect-reject-code`
  for a deterministic pass/fail admission-rejection artifact.

## Artifacts

- `media-disabled-proof.json` — producer evidence (authenticated, rejected).
- `isolated-listeners.json` — bound gRPC/MCP listeners for the isolated HUD.
- `windows-media-ingress-operator-disabled.toml` (staged at `/tmp/hud-8dht5/`) —
  `enabled=true` + `operator_disabled=true`.
