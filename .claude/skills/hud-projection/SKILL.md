---
name: hud-projection
description: Use when an already-running Codex, Claude, opencode, or other LLM session should cooperatively project itself into tze_hud, publish status/output, poll HUD-originated input, acknowledge input, detach, or clean up through an external projection daemon. Do not use for terminal capture, PTY attachment, tmux scraping, process hosting, or direct runtime v1 MCP zone publishing.
compatibility: Requires an external projection-daemon MCP server, CLI, or local IPC surface that implements the cooperative HUD projection operation contract.
metadata:
  owner: tze
  authors:
    - tze
    - OpenAI Codex
  status: active
  last_reviewed: "2026-04-29"
---

# HUD Projection

Use this skill to opt an already-running LLM session into a governed tze_hud text-stream portal.

Hard boundaries:
- This is cooperative opt-in. The current session intentionally calls projection operations.
- This is not PTY, tmux, shell, stdin/stdout, or terminal byte-stream capture.
- The daemon owns projection state outside the LLM token context and outside runtime core: HUD connection metadata, advisory portal lease identity, bounded transcript/window state, pending HUD input, acknowledgement state, lifecycle state, unread state, privacy classification, and reconnect bookkeeping.
- If MCP is available, it is the external projection daemon's MCP facade, not the runtime v1 MCP bridge.

## Source Of Truth

When changing behavior or resolving ambiguity, read:
- `openspec/changes/cooperative-hud-projection/specs/cooperative-hud-projection/spec.md`
- `openspec/changes/cooperative-hud-projection/design.md`
- `openspec/changes/cooperative-hud-projection/reconciliation.md`

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
2. **Store the owner token out of transcript text.** A successful attach returns `owner_token`; no other response should return it. Use it for later owner-scoped operations, but do not paste it into user-visible HUD output or logs.
3. **Publish intentionally.** Call `publish_output` for assistant-visible transcript/status fragments and `publish_status` for lifecycle updates such as `active`, `waiting_for_input`, `blocked`, or `detached`.
4. **Poll HUD input compactly.** Call `get_pending_input` with small `max_items` and `max_bytes`. Treat returned input as semantic operator-submitted text, not terminal keystrokes.
5. **Acknowledge every input item.** Use `acknowledge_input` with `handled`, `deferred`, or `rejected`. Use `not_before_wall_us` only with `deferred`.
6. **Detach on normal exit.** Call `detach` with a bounded reason when the session is done projecting.
7. **Cleanup stale state when appropriate.** Use owner cleanup with `owner_token`; operator cleanup uses a separate daemon authority and must not expose private projection content.

## MCP Facade

If projection MCP tools are connected, call the external daemon's MCP facade. The facade may expose one dispatcher tool such as `projection_operation` or one tool per operation. The JSON payloads in `references/operation-examples.md` remain the contract either way.

To configure an external daemon MCP server for Claude-style clients, adapt `settings.template.json`. The server name should make the boundary clear, for example `hud-projection-daemon`, so agents do not confuse it with runtime v1 MCP zone tools.

See `references/mcp-facade.md` for facade requirements and a minimal configuration shape.

## Safety Notes

- Keep operation responses bounded; do not ask the daemon for unbounded transcripts, inbox history, or raw scene state.
- Do not publish secrets or owner tokens into the transcript window.
- Treat `owner_token` as attach-only response material; it must never be returned by publish, input, acknowledgement, detach, or cleanup responses.
- Treat `PROJECTION_UNAUTHORIZED`, `PROJECTION_TOKEN_EXPIRED`, and `PROJECTION_STATE_CONFLICT` as hard stops unless the user explicitly authorizes reattach or operator cleanup.
- If the daemon restarts, prior transcript text, pending input text, owner tokens, and cached lease identity are gone. Attach again and receive a fresh owner token.
