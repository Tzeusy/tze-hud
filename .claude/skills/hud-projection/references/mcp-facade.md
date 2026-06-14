# Projection Control Facade Requirements

The production projection ingress routes cooperative projection operations into the runtime's in-process `ProjectionAuthority`. It is **not** a standalone external daemon and **not** the runtime v1 MCP zone/widget publishing bridge.

**Current status:** The ingress is **partially wired**. The OUTPUT operations `attach` and `publish_output` are served by the runtime MCP server's portal-projection facade tools `portal_projection_attach` and `portal_projection_publish` (`crates/tze_hud_mcp/src/server.rs` ~556-565), which forward to the in-process authority over `portal_op_tx`. However, those two tools are classified Resident and are rejected with `CAPABILITY_REQUIRED` unless the caller holds `resident_mcp` (`crates/tze_hud_mcp/src/server.rs` ~198-199, ~396), and the runtime's HTTP MCP transport mints only bearer/guest contexts with no capabilities (`crates/tze_hud_runtime/src/mcp.rs` ~256-260) — so a normal external session cannot reach them yet. The resident-capable ingress that closes this gap is tracked by hud-bq0gl.1. The input-return + lifecycle operations (`publish_status`, `get_pending_input`, `acknowledge_input`, `detach`, `cleanup`) have **no MCP method at all yet** (tracked by hud-bq0gl.1 for production ingress and hud-bq0gl.3 for the operator input-return loop). The boundary requirements below apply to the full facade as it completes.

## Required Boundary

The production ingress must:
- Accept only the cooperative operation contract from `operation-examples.md`.
- Authenticate callers through an MCP bearer token, OS-protected IPC, or another unguessable credential.
- Bind owner-scoped non-attach operations to `projection_id` plus `owner_token`; bind operator cleanup to separate explicit operator authority.
- Return bounded operation responses only: no unbounded transcript, unbounded inbox history, `owner_token` outside a successful `attach` response, owner-token verifier, or raw runtime scene graph.
- Emit audit records without transcript text, HUD input text, or owner tokens.
- Route operations to the in-process `ProjectionAuthority` in `tze_hud_runtime`, not to a separate projection process.

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

## Tool Shape

Either shape is acceptable if the payload schema remains the same:

- One dispatcher tool, for example `projection_operation(payload)`.
- Seven operation tools: `attach`, `publish_output`, `publish_status`, `get_pending_input`, `acknowledge_input`, `detach`, and `cleanup`.

The skill examples use operation JSON payloads so they work with either facade shape.

## Claude-Style MCP Configuration (Future)

When the production MCP ingress ships (hud-bq0gl.1), adapt `settings.template.json` to point at it:

```json
{
  "mcpServers": {
    "hud-projection-daemon": {
      "type": "url",
      "url": "http://<PROJECTION_INGRESS_HOST>:<PORT>/mcp",
      "headers": {
        "Authorization": "Bearer ${HUD_PROJECTION_INGRESS_TOKEN}"
      }
    }
  }
}
```

Keep the server name explicit (e.g. `hud-projection-daemon`). Avoid names such as `tze-hud` that can be confused with the runtime v1 MCP zone publishing bridge.
