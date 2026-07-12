# Proposal: mcp-standard-tool-invocation

## Why

The runtime MCP server (`crates/tze_hud_mcp`) implements the standard
`initialize` and `tools/list` JSON-RPC methods, but it dispatched tools as
**bare** JSON-RPC methods — the method name *is* the tool name
(`{"method": "publish_to_widget", ...}`). It never implemented the MCP-standard
`tools/call` method. Every spec-compliant MCP client — the Claude Code MCP
config, the MCP inspector, the official SDKs — invokes tools exclusively via
`tools/call` and received `-32601 "Method not found: tools/call"`, so the tze_hud
tool surface was unreachable from any standard client despite `tools/list`
faithfully advertising it (hud-09emd).

## What Changes

- **`tools/call` dispatch**: the MCP server SHALL implement the standard
  `tools/call` method. It unwraps `params.name` (the tool) and
  `params.arguments` (the tool's parameters) and delegates to the SAME tool
  dispatch table the bare method==tool-name path uses — no forked tool registry.
- **Spec-shaped results**: a successful `tools/call` returns an MCP result
  (`content` array with a text block carrying the tool's JSON result, plus
  `isError: false`), distinct from the raw tool JSON the bare path returns.
- **Tool-execution errors as `isError`**: a tool that runs but fails is reported
  as a result with `isError: true` and a text content block, so the calling model
  sees and can react to the failure — not a JSON-RPC protocol error.
- **Correct protocol errors**: an unknown tool NAME via `tools/call` is
  JSON-RPC Invalid Params (`-32602`), not Method Not Found (`-32601`) — the
  `tools/call` method itself exists. Missing/non-string `name` is Invalid Params.
- **Same capability gate**: `tools/call` runs after PSK authentication and
  through the identical Guest/Resident capability gate as the bare path — a
  Resident tool still requires `resident_mcp`.
- **Back-compat preserved**: the legacy bare method==tool-name dispatch remains
  supported unchanged, still returning the tool's raw JSON as `result`.

The `capabilities.tools` object already advertised by `initialize`
(`{ listChanged: false }`) covers both `tools/list` and `tools/call`; no
capability-advertisement change is required.

## Impact

- Spec: adds one requirement to `session-protocol` (introspection requirement
  unchanged). Additive — no behavior removed.
- Code: `crates/tze_hud_mcp/src/server.rs` `dispatch()` gains a `tools/call` arm
  and response-shaping helpers. Bare-method dispatch is byte-for-byte unchanged.
- Clients: standard MCP clients can now reach the tool surface; existing
  bare-method callers are unaffected.
