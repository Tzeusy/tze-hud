# Tasks: mcp-standard-tool-invocation

## 1. Spec delta (this change's deliverable)

- [x] 1.1 Author delta: MCP Standard Tool Invocation requirement (session-protocol)
- [x] 1.2 `openspec validate mcp-standard-tool-invocation --strict` passes

## 2. Implementation (bead hud-09emd — this PR)

- [x] 2.1 `dispatch()`: add `tools/call` arm — unwrap `params.name` /
      `params.arguments`, rewrite to the bare form, flag response for spec-shaping.
- [x] 2.2 Delegate to the same `invoke_tool` dispatch table + capability gate.
- [x] 2.3 Spec-shape the response: `tools_call_success_result` (content/isError:false)
      and `tools_call_error_result` (content/isError:true for tool-execution errors).
- [x] 2.4 Unknown tool via `tools/call` → Invalid Params (-32602), not -32601;
      bare-method unknown method stays -32601.
- [x] 2.5 Keep bare method==tool-name dispatch unchanged (raw JSON result).
- [x] 2.6 Tests: happy-path delegation + spec shape, unknown-tool, missing-name,
      tool-execution `isError`, bare-method back-compat.

## 3. Docs

- [x] 3.1 Confirm skill docs (hud-projection / th-hud-publish) do not assert a
      bare-method-only dialect (they describe the MCP tool surface abstractly; the
      bare-method transport keeps working, so no correction is needed).
