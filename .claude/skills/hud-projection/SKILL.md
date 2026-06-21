---
name: hud-projection
description: Use when an already-running Codex, Claude, opencode, or other LLM session should project itself onto the HUD, show its output or status on screen, attach to a text-stream portal, publish live transcript, poll HUD-originated operator input, acknowledge input, detach, or clean up. Trigger phrases — "project this session to the HUD", "attach this agent to HUD", "show this LLM session in a text-stream portal", "check HUD input". Do not use for terminal capture, PTY attachment, tmux scraping, process hosting, or direct runtime v1 MCP zone publishing; for one-shot zone publishing use th-hud-publish instead.
compatibility: Requires the tze_hud windowed runtime running with MCP enabled (mcp_port > 0). The FULL operation contract is wired end-to-end in-process: all seven ops — `attach`, `publish_output`, `publish_status`, `get_pending_input`, `acknowledge_input`, `detach`, `cleanup` — are served by the runtime MCP server's `portal_projection_*` tools (`crates/tze_hud_mcp/src/server.rs` ~643-683), which forward over `portal_op_tx` to the windowed drain loop and the in-process ProjectionAuthority. These are Resident tools (`resident_mcp`); an external LLM session reaches them by presenting the resident principal — set `TZE_HUD_MCP_RESIDENT_PRINCIPAL` equal to the runtime PSK and send that value as the MCP bearer (the runtime constant-time-matches bearer==principal==PSK and mints `resident_mcp`; hud-nu65o). Published output renders on screen for BOTH portal adapter families — the exemplar gRPC adapter and the in-process cooperative driver (the cooperative render path landed in PR #959). The stale "partial / CAPABILITY_REQUIRED / no MCP method yet" framing is obsolete: hud-bq0gl.1 (production ingress) and hud-bq0gl.3 (input-return loop) are closed.
metadata:
  owner: tze
  authors:
    - tze
    - OpenAI Codex
  status: active
  last_reviewed: "2026-06-21"
---

# HUD Projection

Use this skill to opt an already-running LLM session into a governed tze_hud text-stream portal.

Hard boundaries:
- This is cooperative opt-in. The current session intentionally calls projection operations.
- This is not PTY, tmux, shell, stdin/stdout, or terminal byte-stream capture.
- The `ProjectionAuthority` runs **in-process** inside the tze_hud runtime (not as an external daemon). It owns projection state outside the LLM token context: HUD connection metadata, advisory portal lease identity, bounded transcript/window state, pending HUD input, acknowledgement state, lifecycle state, unread state, privacy classification, and reconnect bookkeeping.
- The projection MCP surface is a facade into the runtime's in-process authority, not the runtime v1 MCP zone publishing bridge. **The full facade is wired in-process**: all seven `portal_projection_*` tools are served by the runtime MCP server (`crates/tze_hud_mcp/src/server.rs` ~643-683) and forward to the in-process authority over `portal_op_tx`. They are classified Resident tools, so the runtime rejects them with `CAPABILITY_REQUIRED` unless the caller holds `resident_mcp` (`crates/tze_hud_mcp/src/server.rs` ~218-232, ~321-352). An external session obtains `resident_mcp` by authenticating as the **resident principal**: the runtime mints the capability only when the MCP bearer matches BOTH the configured `TZE_HUD_MCP_RESIDENT_PRINCIPAL` AND the PSK, each compared constant-time (so it can never silently grant `resident_mcp` to every authenticated caller). Set `TZE_HUD_MCP_RESIDENT_PRINCIPAL` equal to the PSK and send the PSK as the bearer. The portal-projection tools are a projection facade distinct from the runtime's zone/widget publishing tools — do not confuse them with `th-hud-publish`.

## Source Of Truth

When changing behavior or resolving ambiguity, read:
- `openspec/specs/cooperative-hud-projection/spec.md` — canonical spec (synced from the concluded change; the change directory is now archived under `openspec/changes/archive/2026-05-10-cooperative-hud-projection/`)

## Use When

- The user asks to "project this session to the HUD", "attach this agent to HUD", "show this LLM session in a text-stream portal", or "check HUD input".
- A Codex, Claude, opencode, or other provider session needs to publish explicit output/status to the HUD.
- The session needs to poll operator-submitted HUD input and acknowledge each input item as handled, deferred, or rejected.
- The session needs to detach or clean up its projection.

Do not use this skill for one-shot zone publishing; use `th-hud-publish` for that.

## Operation Contract

All requests include:
- `operation`
- `projection_id`
- `request_id`
- `client_timestamp_wall_us`

Owner-scoped operations after `attach` also include `owner_token`. Operator cleanup is the only non-attach operation that may instead use separate daemon authority.

The normative operations are:
- `attach`
- `publish_output`
- `publish_status`
- `get_pending_input`
- `acknowledge_input`
- `detach`
- `cleanup`

Read `references/operation-examples.md` for compact JSON examples of every operation, including Codex, Claude, and opencode attach examples.

## Workflow

1. **Attach once.** Choose a stable `projection_id`, set `provider_kind` to `codex`, `claude`, `opencode`, or `other`, and include a human-readable `display_name`. Default missing or uncertain classification to `private`.
2. **Store the owner token securely and immediately.** A successful attach returns `owner_token`; no other operation response will ever return it. Store it in a tool-call result or session variable, never in transcript text, assistant-visible output, or log lines. If it is lost before detach, you must treat the projection as unrecoverable and wait for operator cleanup or TTL expiry. Do not request the token again — there is no retrieval path.
3. **Publish intentionally.** Call `publish_output` for assistant-visible transcript/status fragments and `publish_status` for lifecycle updates such as `active`, `degraded`, or `detached`.

   **Accepted `lifecycle_state` values** (snake_case strings; any other value is rejected):
   - `attached` — session is attached but not yet actively working
   - `active` — session is running / producing output
   - `degraded` — session is blocked, slow, or in a degraded state
   - `hud_unavailable` — session cannot reach the HUD
   - `detached` — session has detached cleanly
   - `cleanup_pending` — projection is pending removal
   - `expired` — projection TTL has elapsed

   **Accepted `output_kind` values** (snake_case strings; defaults to `assistant` when omitted; any other value is rejected):
   - `assistant` *(default)* — normal assistant message / transcript fragment
   - `tool` — tool call or tool result
   - `status` — status or progress update
   - `error` — error output
   - `other` — any other kind
4. **Poll HUD input compactly.** Call `get_pending_input` with small `max_items` and `max_bytes`. Treat returned input as semantic operator-submitted text, not terminal keystrokes.
5. **Acknowledge every input item.** Use `acknowledge_input` with `handled`, `deferred`, or `rejected`. Use `not_before_wall_us` only with `deferred`.
6. **Detach on normal exit.** Call `detach` with a bounded reason when the session is done projecting.
7. **Cleanup stale state when appropriate.** Use owner cleanup with `owner_token`; operator cleanup uses a separate daemon authority and must not expose private projection content.

## Production Ingress (Wired — full contract, reachable via the resident principal)

The full operation contract is wired end-to-end **inside** the runtime. When the
windowed runtime runs with MCP enabled (`mcp_port > 0`), the runtime MCP server
exposes one tool per operation (`portal_projection_<op>`), each forwarding into
the in-process `ProjectionAuthority` over `portal_op_tx`. All are Resident tools,
reachable by a caller that holds `resident_mcp`:

| Operation | MCP tool | Reachable by external session | Code path |
|---|---|---|---|
| `attach` | `portal_projection_attach` | yes — as the resident principal | `crates/tze_hud_mcp/src/server.rs` ~643-647 |
| `publish_output` | `portal_projection_publish` | yes — as the resident principal | `crates/tze_hud_mcp/src/server.rs` ~648-652 |
| `publish_status` | `portal_projection_publish_status` | yes — as the resident principal | `crates/tze_hud_mcp/src/server.rs` ~653-660 |
| `get_pending_input` | `portal_projection_get_pending_input` | yes — as the resident principal | `crates/tze_hud_mcp/src/server.rs` ~661-668 |
| `acknowledge_input` | `portal_projection_acknowledge_input` | yes — as the resident principal | `crates/tze_hud_mcp/src/server.rs` ~669-676 |
| `detach` | `portal_projection_detach` | yes — as the resident principal | `crates/tze_hud_mcp/src/server.rs` ~677-681 |
| `cleanup` | `portal_projection_cleanup` | yes — as the resident principal (operator cleanup also accepted) | `crates/tze_hud_mcp/src/server.rs` ~682-683 |

How the path is wired:

1. **Op channel.** The runtime creates `portal_op_tx` whenever `mcp_port > 0` and
   hands it to the MCP server; the matching `portal_op_rx` is drained every frame
   by the windowed event loop (`drain_portal_ops` →
   `InProcessPortalDriver::dispatch_portal_op` → the per-frame `drain`), so a
   *resident* op reaches the live scene.
2. **Resident capability.** The tools are classified Resident (`classify_tool`,
   `crates/tze_hud_mcp/src/server.rs` ~218-232) and rejected with
   `CAPABILITY_REQUIRED` unless `ctx.has_resident_mcp()`. The single auditable
   place that mints `resident_mcp` (`caller_context`, ~321-352) grants it only
   when the request bearer matches BOTH the configured resident principal AND the
   PSK, each compared constant-time (`with_resident_principal`, ~187;
   `has_resident_mcp`, ~110). This is `hud-nu65o` — it can never silently grant
   `resident_mcp` to every authenticated caller.
3. **Runtime wiring.** The runtime passes the principal through from config:
   `McpConfig::with_psk(..).with_resident_principal(config.resident_principal)`
   (`crates/tze_hud_runtime/src/mcp.rs` ~111), surfaced operationally via the
   `TZE_HUD_MCP_RESIDENT_PRINCIPAL` env var (~71-73). Tests
   `mcp_http_resident_principal_reaches_resident_tool` and
   `mcp_http_no_resident_principal_still_gates_resident_tool`
   (`crates/tze_hud_runtime/src/mcp.rs` ~576, ~612) prove an external HTTP MCP
   caller reaches a Resident tool with the principal and is still gated without it.

To reach the facade as an external session: configure the runtime with
`TZE_HUD_MCP_RESIDENT_PRINCIPAL` set equal to the PSK, then send the PSK as your
MCP bearer. Published `attach`/`publish_output` content renders on screen for
both portal adapter families (the exemplar gRPC adapter and the in-process
cooperative driver; the cooperative render landed in PR #959).

The stdio component harness
(`crates/tze_hud_projection/src/bin/projection_authority.rs`) remains useful for
local protocol testing and audit-record inspection, but it runs the authority in
an isolated process with **no** connection to the live runtime — its output never
reaches the screen. Use the MCP facade above for real on-screen projection.

The JSON payloads in `references/operation-examples.md` are the per-operation
contract. `settings.template.json` shows the expected configuration shape.

See `references/mcp-facade.md` for facade requirements, boundary rules, and a configuration template.

## Safety Notes

- Keep operation responses bounded; do not request unbounded transcripts, inbox history, or raw scene state.
- Do not publish secrets or owner tokens into the transcript window or any user-visible output.
- Treat `owner_token` as attach-only response material; it must never be returned by publish, input, acknowledgement, detach, or cleanup responses. If a response includes `owner_token` outside an `attach` success, treat that as a protocol error and do not use or forward the value.
- **Owner-token loss is unrecoverable.** If the token is lost (session crash, transcript cleared), there is no retrieval path. The only recovery options are operator cleanup (requires separate operator authority, not the owner token) or waiting for the projection's TTL to expire. Do not attempt to re-attach with the same `projection_id` without the idempotency key — the authority will reject it with `PROJECTION_ALREADY_ATTACHED`.
- **Do not embed `owner_token` in `publish_output` text, `status_text`, `ack_message`, or `reason` fields.** These fields are readable by audit records and portal rendering; tokens in them constitute a credential leak.
- Treat `PROJECTION_UNAUTHORIZED`, `PROJECTION_TOKEN_EXPIRED`, and `PROJECTION_STATE_CONFLICT` as hard stops unless the user explicitly authorizes reattach or operator cleanup.
- If the runtime restarts, prior transcript text, pending input text, owner tokens, and cached lease identity are gone. Attach again and receive a fresh owner token — the old token is permanently invalid after a restart.
