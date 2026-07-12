# session-protocol Delta: MCP standard tool invocation (tools/call)

## ADDED Requirements

### Requirement: MCP Standard Tool Invocation
The MCP server SHALL implement the standard `tools/call` JSON-RPC method so that spec-compliant MCP clients (which invoke every tool through `tools/call`, never by using the tool name as the JSON-RPC method) can reach the tool surface. The `tools/call` method SHALL be dispatched after PSK authentication and SHALL pass through the same Guest/Resident capability gate as bare-method invocation — a Resident-class tool invoked via `tools/call` still requires the `resident_mcp` capability.

A `tools/call` request carries `params.name` (the tool to invoke) and an optional `params.arguments` object (the tool's parameters; an omitted `arguments` is treated as an empty object). The server SHALL delegate to the SAME tool dispatch table used by the legacy bare method==tool-name path — there SHALL NOT be a forked or duplicate tool registry.

On success `tools/call` SHALL return an MCP-shaped result: a `content` array containing at least one `text` block carrying the tool's result, together with `isError: false`. A tool that executes but fails SHALL be reported as a result with `isError: true` and a `text` content block describing the failure — NOT a JSON-RPC error — so the calling model can observe and react to the error. An unknown tool NAME supplied via `tools/call` SHALL return a JSON-RPC Invalid Params error (`-32602`), NOT Method Not Found (`-32601`), because the `tools/call` method itself is implemented; a missing or non-string `name` SHALL likewise return Invalid Params.

The legacy bare method==tool-name dispatch SHALL remain supported unchanged for back-compat, returning the tool's raw JSON result directly as the JSON-RPC `result` (not wrapped in the `content`/`isError` envelope). The `capabilities.tools` object advertised by `initialize` covers both `tools/list` and `tools/call`.

Source: crates/tze_hud_mcp/src/server.rs; hud-09emd
Scope: v1-mandatory

#### Scenario: tools/call invokes the same tool as the bare method path
- **WHEN** an authenticated MCP client sends `tools/call` with `params.name` set to a registered tool and `params.arguments` set to that tool's parameters
- **THEN** the runtime SHALL execute the identical tool that the bare method==tool-name path would run with those parameters
- **AND** the response SHALL be an MCP result with a `content` array carrying the tool's result as a `text` block and `isError: false`

#### Scenario: unknown tool via tools/call is Invalid Params
- **WHEN** an authenticated MCP client sends `tools/call` with a `params.name` that is not a registered tool
- **THEN** the runtime SHALL return a JSON-RPC error with code `-32602` (Invalid Params)
- **AND** it SHALL NOT return `-32601` (Method Not Found), because the `tools/call` method itself is implemented

#### Scenario: tool execution failure via tools/call is an isError result
- **WHEN** a tool invoked through `tools/call` runs but fails (e.g. it targets a resource that does not exist)
- **THEN** the runtime SHALL return a result with `isError: true` and a `text` content block describing the failure
- **AND** it SHALL NOT surface the failure as a JSON-RPC error object

#### Scenario: bare method dispatch remains supported
- **WHEN** an authenticated MCP client sends a request whose JSON-RPC `method` is a tool name directly (the legacy dialect)
- **THEN** the runtime SHALL execute that tool and return its raw JSON result as the JSON-RPC `result`
- **AND** the result SHALL NOT be wrapped in the `tools/call` `content`/`isError` envelope
