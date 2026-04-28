# External Projection-Daemon MCP Facade

The MCP facade belongs to the projection daemon. It is not the runtime v1 MCP bridge and should not expose raw scene state, zone publishing shortcuts, PTY attachment, terminal capture, or process lifecycle controls.

## Required Boundary

- Accept only the cooperative operation contract from `operation-examples.md`.
- Authenticate callers through daemon-local authority, an MCP bearer token, OS-protected IPC, or another unguessable credential.
- Bind owner-scoped non-attach operations to `projection_id` plus `owner_token`; bind operator cleanup to separate explicit operator authority.
- Return bounded operation responses only: no unbounded transcript, unbounded inbox history, `owner_token` outside a successful `attach` response, owner-token verifier, or raw runtime scene graph.
- Emit audit records without transcript text, HUD input text, or owner tokens.

## Tool Shape

Either shape is acceptable if the payload schema remains the same:

- One dispatcher tool, for example `projection_operation(payload)`.
- Seven operation tools: `attach`, `publish_output`, `publish_status`, `get_pending_input`, `acknowledge_input`, `detach`, and `cleanup`.

The skill examples use operation JSON payloads so they work with either facade shape.

## Claude-Style MCP Configuration

Use `settings.template.json` as a starting point:

```json
{
  "mcpServers": {
    "hud-projection-daemon": {
      "type": "url",
      "url": "http://<PROJECTION_DAEMON_HOST>:<PORT>/mcp",
      "headers": {
        "Authorization": "Bearer ${HUD_PROJECTION_DAEMON_TOKEN}"
      }
    }
  }
}
```

Keep the server name explicit. Avoid names such as `tze-hud` that can be confused with the runtime v1 MCP zone publishing bridge.
