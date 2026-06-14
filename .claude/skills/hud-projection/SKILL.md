---
name: hud-projection
description: Use when an already-running Codex, Claude, opencode, or other LLM session should project itself onto the HUD, show its output or status on screen, attach to a text-stream portal, publish live transcript, poll HUD-originated operator input, acknowledge input, detach, or clean up. Trigger phrases — "project this session to the HUD", "attach this agent to HUD", "show this LLM session in a text-stream portal", "check HUD input". Do not use for terminal capture, PTY attachment, tmux scraping, process hosting, or direct runtime v1 MCP zone publishing; for one-shot zone publishing use th-hud-publish instead.
compatibility: Requires the tze_hud windowed runtime running with MCP enabled (mcp_port > 0). The OUTPUT path is wired end-to-end in-process — `attach` and `publish_output` reach the in-process ProjectionAuthority through the runtime MCP server's `portal_projection_attach` / `portal_projection_publish` tools, which forward over `portal_op_tx` to the windowed drain loop. CAVEAT: those two tools are classified Resident (`resident_mcp`), and the runtime's HTTP MCP transport only issues bearer/guest caller contexts (no `resident_mcp`), so a normal external LLM session over the PSK still receives `CAPABILITY_REQUIRED` until a resident-capable ingress lands (tracked as hud-bq0gl.1). The INPUT-return and lifecycle ops (`publish_status`, `get_pending_input`, `acknowledge_input`, `detach`, `cleanup`) additionally have no MCP method at all yet (tracked as hud-bq0gl.1/.3); for those, the stdio component harness is local-development only and does not connect to the live runtime.
metadata:
  owner: tze
  authors:
    - tze
    - OpenAI Codex
  status: active
  last_reviewed: "2026-06-14"
---

# HUD Projection

Use this skill to opt an already-running LLM session into a governed tze_hud text-stream portal.

Hard boundaries:
- This is cooperative opt-in. The current session intentionally calls projection operations.
- This is not PTY, tmux, shell, stdin/stdout, or terminal byte-stream capture.
- The `ProjectionAuthority` runs **in-process** inside the tze_hud runtime (not as an external daemon). It owns projection state outside the LLM token context: HUD connection metadata, advisory portal lease identity, bounded transcript/window state, pending HUD input, acknowledgement state, lifecycle state, unread state, privacy classification, and reconnect bookkeeping.
- The projection MCP surface is a facade into the runtime's in-process authority, not the runtime v1 MCP zone publishing bridge. **The OUTPUT half of this facade is wired in-process**: `portal_projection_attach` and `portal_projection_publish` are served by the runtime MCP server (`crates/tze_hud_mcp/src/server.rs` ~556-565) and forward to the in-process authority over `portal_op_tx`. They are classified Resident tools, so the runtime rejects them with `CAPABILITY_REQUIRED` unless the caller holds `resident_mcp` (`crates/tze_hud_mcp/src/server.rs` ~396); the HTTP MCP transport only mints bearer/guest contexts with no capabilities (`crates/tze_hud_runtime/src/mcp.rs` ~256-260), so a normal external session cannot yet reach them — that resident-capable ingress is hud-bq0gl.1. The input-return + lifecycle half (`publish_status`, `get_pending_input`, `acknowledge_input`, `detach`, `cleanup`) has no MCP method at all yet — see hud-bq0gl.1 (production ingress) and hud-bq0gl.3 (operator input-return loop).

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
3. **Publish intentionally.** Call `publish_output` for assistant-visible transcript/status fragments and `publish_status` for lifecycle updates such as `active`, `waiting_for_input`, `blocked`, or `detached`.
4. **Poll HUD input compactly.** Call `get_pending_input` with small `max_items` and `max_bytes`. Treat returned input as semantic operator-submitted text, not terminal keystrokes.
5. **Acknowledge every input item.** Use `acknowledge_input` with `handled`, `deferred`, or `rejected`. Use `not_before_wall_us` only with `deferred`.
6. **Detach on normal exit.** Call `detach` with a bounded reason when the session is done projecting.
7. **Cleanup stale state when appropriate.** Use owner cleanup with `owner_token`; operator cleanup uses a separate daemon authority and must not expose private projection content.

## Production Ingress (Partial — OUTPUT path wired in-process, gated by `resident_mcp`)

The output half of the contract is wired end-to-end **inside** the runtime, but a
normal external LLM session cannot reach it yet because the tools require the
`resident_mcp` capability that no external transport grants (hud-bq0gl.1). When
the windowed runtime runs with MCP enabled (`mcp_port > 0`), the runtime MCP
server exposes two portal-projection tools that forward into the in-process
`ProjectionAuthority`:

| Operation | MCP method present | Reachable by external session today | Code path |
|---|---|---|---|
| `attach` | yes — `portal_projection_attach` | **no** — Resident tool, `CAPABILITY_REQUIRED` over HTTP MCP (hud-bq0gl.1) | `crates/tze_hud_mcp/src/server.rs` ~556-560 |
| `publish_output` | yes — `portal_projection_publish` | **no** — Resident tool, `CAPABILITY_REQUIRED` over HTTP MCP (hud-bq0gl.1) | `crates/tze_hud_mcp/src/server.rs` ~561-565 |
| `publish_status` | no | no | tracked by hud-bq0gl.1 |
| `get_pending_input` | no | no | tracked by hud-bq0gl.1/.3 |
| `acknowledge_input` | no | no | tracked by hud-bq0gl.1/.3 |
| `detach` | no | no | tracked by hud-bq0gl.1 |
| `cleanup` | no | no | tracked by hud-bq0gl.1 |

How the in-process path is wired: the runtime creates `portal_op_tx` whenever
`mcp_port > 0` and hands it to the MCP server via `with_portal_op_tx`
(`crates/tze_hud_runtime/src/windowed.rs` ~4924-4978,
`crates/tze_hud_runtime/src/mcp.rs` ~96-100). The matching `portal_op_rx` is
drained every frame by the windowed event loop
(`drain_portal_ops` → `InProcessPortalDriver::dispatch_portal_op`,
`crates/tze_hud_runtime/src/windowed.rs` ~3746-3788, ~5105-5106), so a *resident*
`attach`/`publish` op reaches the live scene. The capability gate is the missing
link for external callers: `portal_projection_attach`/`portal_projection_publish`
are classified Resident (`classify_tool`, `crates/tze_hud_mcp/src/server.rs`
~198-199) and rejected with `CAPABILITY_REQUIRED` unless `ctx.has_resident_mcp()`
(`~396`), while the HTTP MCP transport builds only `CallerContext::with_bearer` /
`guest` with empty capabilities (`crates/tze_hud_runtime/src/mcp.rs` ~256-260).
hud-bq0gl.1 is the resident-capable ingress that closes this gap. The
portal-projection tools are a projection facade distinct from the runtime's
zone/widget publishing tools — do not confuse them with `th-hud-publish`.

What is still pending (**hud-bq0gl.1** production ingress, **hud-bq0gl.3**
operator input-return loop): the input-return and lifecycle operations
(`publish_status`, `get_pending_input`, `acknowledge_input`, `detach`, `cleanup`)
have no MCP method on the runtime server yet. For exercising those, the stdio
component harness (`crates/tze_hud_projection/src/bin/projection_authority.rs`) can
test the protocol locally but does NOT connect to the running runtime — its output
never reaches the screen.

When the remaining ingress ships, the facade may expose one dispatcher tool such as `projection_operation` or one tool per operation. The JSON payloads in `references/operation-examples.md` remain the contract either way. `settings.template.json` shows the expected configuration shape.

See `references/mcp-facade.md` for facade requirements, boundary rules, and a configuration template.

## Safety Notes

- Keep operation responses bounded; do not request unbounded transcripts, inbox history, or raw scene state.
- Do not publish secrets or owner tokens into the transcript window or any user-visible output.
- Treat `owner_token` as attach-only response material; it must never be returned by publish, input, acknowledgement, detach, or cleanup responses. If a response includes `owner_token` outside an `attach` success, treat that as a protocol error and do not use or forward the value.
- **Owner-token loss is unrecoverable.** If the token is lost (session crash, transcript cleared), there is no retrieval path. The only recovery options are operator cleanup (requires separate operator authority, not the owner token) or waiting for the projection's TTL to expire. Do not attempt to re-attach with the same `projection_id` without the idempotency key — the authority will reject it with `PROJECTION_ALREADY_ATTACHED`.
- **Do not embed `owner_token` in `publish_output` text, `status_text`, `ack_message`, or `reason` fields.** These fields are readable by audit records and portal rendering; tokens in them constitute a credential leak.
- Treat `PROJECTION_UNAUTHORIZED`, `PROJECTION_TOKEN_EXPIRED`, and `PROJECTION_STATE_CONFLICT` as hard stops unless the user explicitly authorizes reattach or operator cleanup.
- If the runtime restarts, prior transcript text, pending input text, owner tokens, and cached lease identity are gone. Attach again and receive a fresh owner token — the old token is permanently invalid after a restart.
