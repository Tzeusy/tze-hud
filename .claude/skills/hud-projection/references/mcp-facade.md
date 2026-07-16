# Projection Control Facade Requirements

The production projection ingress routes cooperative projection operations into the runtime's in-process `ProjectionAuthority`. It is **not** a standalone external daemon and **not** the runtime v1 MCP zone/widget publishing bridge.

**Current status:** The ingress is **fully wired**. All seven operations are served by the runtime MCP server's per-operation `portal_projection_<op>` tools (`crates/tze_hud_mcp/src/server.rs` ~643-683), which forward to the in-process authority over `portal_op_tx`. The tools are classified Resident and require `resident_mcp` (`crates/tze_hud_mcp/src/server.rs` ~218-232); an external session obtains that capability as the **resident principal** — the runtime mints `resident_mcp` only when the MCP bearer matches BOTH the configured `TZE_HUD_MCP_RESIDENT_PRINCIPAL` AND the PSK, each compared constant-time (`caller_context`, ~321-352; `with_resident_principal`, ~187). Wire it by setting `TZE_HUD_MCP_RESIDENT_PRINCIPAL` equal to the PSK (`crates/tze_hud_runtime/src/mcp.rs` ~111, env at ~71-73) and sending the PSK as the bearer. Published output renders on screen for both portal adapter families (exemplar gRPC and the in-process cooperative driver; cooperative render landed in PR #959). hud-bq0gl.1 (production ingress), hud-bq0gl.3 (operator input-return loop), and hud-nu65o (PSK-gated resident principal) are all closed. The boundary requirements below describe the contract this ingress satisfies.

## Required Boundary

The production ingress must:
- Accept only the cooperative operation contract from `operation-examples.md`.
- Authenticate callers through an MCP bearer token, OS-protected IPC, or another unguessable credential.
- Bind owner-scoped non-attach operations to `projection_id` plus `owner_token`; bind operator cleanup to separate explicit operator authority.
- Treat a matching-key attach replay as authenticated token rotation: return a fresh token, atomically replace the sole verifier, invalidate every prior token, and preserve the original expiry deadline. The idempotency key never substitutes for Resident MCP authorization.
- Return bounded operation responses only: no unbounded transcript, unbounded inbox history, `owner_token` outside a successful `attach` response, owner-token verifier, or raw runtime scene graph.
- Emit audit records without transcript text, HUD input text, or owner tokens.
- Route operations to the in-process `ProjectionAuthority` in `tze_hud_runtime`, not to a separate projection process.

The authority's retained transcript window and reconnect bookkeeping are
bounded, in-memory presentation state only. Durable transcript continuity is
adapter/client state, never runtime-core persistence. The preferred client
stores its private bounded authored tail under the XDG state hierarchy at
`tze_hud/portal-continuity/`, then replays the same `logical_unit_id` and
`coalesce_key` values after authenticated attach. That file must not contain
an owner token or any viewer-authored/pending HUD input.

## Component Harness (Development / Testing Only)

The repo ships a stdio component harness in `crates/tze_hud_projection` for local protocol development and unit testing:

```bash
cargo run -p tze_hud_projection --bin tze_hud_projection_authority -- --stdio --caller-identity codex-local
```

**This binary is not the production ingress.** It runs `ProjectionAuthority` in an isolated process with no connection to the live runtime, compositor, or display. It is useful for testing the operation contract and inspecting audit records, but output sent to it never reaches the screen.

Send one operation JSON object per stdin line. The process writes one JSON result per stdout line:

```json
{"response":{"request_id":"req-attach","projection_id":"codex-rig","accepted":true,"server_timestamp_wall_us":1777400000000000,"status_summary":"projection attached","owner_token":"<attach-only>","lifecycle_state":"attached","pending_remaining_count":0,"pending_remaining_bytes":0,"portal_update_ready":false,"coalesced_output_count":0},"audit_records":[{"timestamp_wall_us":1777400000000000,"operation":"attach","projection_id":"codex-rig","caller_identity":"codex-local","request_id":"req-attach","accepted":true,"reason":"attach accepted","category":"attach"}]}
```

The harness keeps projection state in memory only for the lifetime of that process. Restarting it purges transcript text, pending input text, owner tokens, and cached lease identity. Operator cleanup can be enabled with `--operator-authority-env HUD_PROJECTION_OPERATOR_AUTHORITY`; owner operations still require the owner token issued by `attach`.

When driving the harness with `portal_client.py`, the client-owned
`portal-continuity` tail can rebuild authored output after restart. This does
not make the harness or runtime durable: a restarted authority creates fresh
runtime state, and the client explicitly republishes its bounded tail.

## Tool Shape

The facade shipped as **seven per-operation tools** — `portal_projection_attach`, `portal_projection_publish` (for `publish_output`), `portal_projection_publish_status`, `portal_projection_get_pending_input`, `portal_projection_acknowledge_input`, `portal_projection_detach`, and `portal_projection_cleanup` — not a single dispatcher tool. The skill examples use operation JSON payloads that map directly onto these tools' params.

## Claude-Style MCP Configuration

The production MCP ingress is live; adapt `settings.template.json` to point at it (set `TZE_HUD_MCP_RESIDENT_PRINCIPAL` equal to the PSK on the runtime, and send the PSK as the bearer):

```json
{
  "mcpServers": {
    "tze-hud-runtime": {
      "type": "url",
      "url": "http://<TZE_HUD_RUNTIME_HOST>:<MCP_PORT>/mcp",
      "headers": {
        "Authorization": "Bearer ${TZE_HUD_PSK}"
      }
    }
  }
}
```

The endpoint is the **runtime's** MCP server (the same server that serves the v1
zone/widget publishing tools) — the projection facade is in-process, not a
separate daemon. The bearer is the runtime PSK; the runtime must also be started
with `TZE_HUD_MCP_RESIDENT_PRINCIPAL` set equal to that PSK so the caller is
minted `resident_mcp`. The projection facade and the zone-publishing bridge are
distinguished by which tools you call (`portal_projection_*` vs the zone/widget
tools), not by separate servers.

A standard MCP client configured this way (Claude Code, the MCP inspector, the
SDKs) invokes tools through the primary MCP-standard `tools/call` method —
`{"method": "tools/call", "params": {"name": "portal_projection_attach",
"arguments": { ... }}}` — and receives a spec-shaped result (a `content` array
with `isError`). The runtime retains a **legacy bare-method fallback** where the
JSON-RPC `method` is the tool name directly (e.g.
`{"method": "portal_projection_attach", "params": { ... }}`), returning the
tool's raw JSON result. Both dialects reach the same tool dispatch table, so
the operation object in `tools/call.params.arguments` is identical to the
legacy method's direct `params` object.
