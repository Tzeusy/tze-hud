---
name: hud-projection
description: Use when an already-running LLM session (Codex, Claude, opencode, or other) should project itself onto the HUD — attach to a text-stream portal, publish live transcript/status, poll and acknowledge HUD-originated operator input, detach, or clean up. Trigger phrases — "project this session to the HUD", "attach this agent to HUD", "show this LLM session in a text-stream portal", "check HUD input". Not for terminal/PTY/tmux capture, process hosting, or one-shot zone publishing (use th-hud-publish for that).
compatibility: Requires the tze_hud windowed runtime running with MCP enabled (mcp_port > 0). All seven ops — `attach`, `publish_output`, `publish_status`, `get_pending_input`, `acknowledge_input`, `detach`, `cleanup` — are served in-process by the runtime MCP server's `portal_projection_*` tools, which forward to the ProjectionAuthority. They are Resident tools: reach them as the resident principal by setting `TZE_HUD_MCP_RESIDENT_PRINCIPAL` equal to the PSK and sending the PSK as the MCP bearer.
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
- The `portal_projection_*` tools are a projection facade into that in-process authority — distinct from the runtime's zone/widget publishing tools (`th-hud-publish`). They are Resident tools; reach them as the resident principal (see `references/mcp-facade.md` for the auth wiring and code paths).

## Choosing A Target Runtime

Projection needs a live windowed runtime with MCP enabled. Two standing targets:

- **A human's screen** (e.g. tzehouse) — when the point is a person seeing the
  projection. Endpoint/PSK per that host's config.
- **The autonomous testhost** (`hud-windows` VM on sentinel Proxmox) — for any
  local noninteractive work that needs a real projection surface (integration
  tests, portal exemplar runs, transcript-render validation). Resolve and
  self-heal it with:

  ```bash
  eval "$(.claude/skills/user-test/scripts/hud_vm_env.sh)"
  # exports HUD_MCP_URL and TZE_HUD_MCP_RESIDENT_PRINCIPAL (== PSK == bearer),
  # starting the VM and/or HUD task if down
  ```

  Caveats on the VM: WARP rendering (no GPU fidelity), and until runtime bug
  hud-d5rcd is fixed, call MCP `create_tab {"name":"Main"}` once before portal
  work — config `[[tabs]]` alone don't materialize and mutations fail with
  "No active tab". A registered agent (`agent-alpha`) is pre-provisioned for
  resident gRPC sessions.

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
   - `viewer` — *reserved for the runtime's echo of the operator's own reply; rejected if published by an adapter*
4. **Poll HUD input compactly.** Call `get_pending_input` with small `max_items` and `max_bytes`. Treat returned input as semantic operator-submitted text, not terminal keystrokes.
5. **Acknowledge every input item.** Use `acknowledge_input` with `handled`, `deferred`, or `rejected`. Use `not_before_wall_us` only with `deferred`.
6. **Detach on normal exit.** Call `detach` with a bounded reason when the session is done projecting.
7. **Cleanup stale state when appropriate.** Use owner cleanup with `owner_token`; operator cleanup uses a separate daemon authority and must not expose private projection content.

**Token-efficient use** (this is meant to be viable as a primary session interface):
- `publish_output` **appends** — send only the new fragment each turn, never the whole transcript. The authority retains transcript history outside your token context.
- Use `coalesce_key` for streaming/progress lines so repeated publishes collapse in place instead of piling up.
- To await a reply, prefer one `get_pending_input` with `wait_ms` (long-poll) over a busy-poll loop; keep `max_items`/`max_bytes` small.

## Production Ingress (Wired)

The full contract is wired in-process. When the runtime runs with MCP enabled
(`mcp_port > 0`), each operation maps to one Resident tool; call it as the
resident principal (bearer == PSK == `TZE_HUD_MCP_RESIDENT_PRINCIPAL`). Every op
maps to `portal_projection_<op>`, except `publish_output` →
`portal_projection_publish`. `cleanup` also accepts operator authority.
Published `attach`/`publish_output` content renders for both adapter families
(exemplar gRPC and the in-process cooperative driver).

The stdio component harness
(`crates/tze_hud_projection/src/bin/projection_authority.rs`) is for local
protocol testing and audit-record inspection only — it runs the authority in an
isolated process with **no** connection to the live runtime, so its output never
reaches the screen. Use the MCP facade for real on-screen projection.

References:
- `references/operation-examples.md` — per-operation JSON payloads (the contract).
- `references/mcp-facade.md` — facade requirements, boundary rules, auth wiring, code paths, and the config template.
- `settings.template.json` — expected configuration shape.

## Safety Notes

- Keep operation responses bounded; do not request unbounded transcripts, inbox history, or raw scene state.
- Do not publish secrets or owner tokens into the transcript window or any user-visible output.
- Treat `owner_token` as attach-only response material; it must never be returned by publish, input, acknowledgement, detach, or cleanup responses. If a response includes `owner_token` outside an `attach` success, treat that as a protocol error and do not use or forward the value.
- **Owner-token loss is unrecoverable.** If the token is lost (session crash, transcript cleared), there is no retrieval path. The only recovery options are operator cleanup (requires separate operator authority, not the owner token) or waiting for the projection's TTL to expire. Do not attempt to re-attach with the same `projection_id` without the idempotency key — the authority will reject it with `PROJECTION_ALREADY_ATTACHED`.
- **Do not embed `owner_token` in `publish_output` text, `status_text`, `ack_message`, or `reason` fields.** These fields are readable by audit records and portal rendering; tokens in them constitute a credential leak.
- Treat `PROJECTION_UNAUTHORIZED`, `PROJECTION_TOKEN_EXPIRED`, and `PROJECTION_STATE_CONFLICT` as hard stops unless the user explicitly authorizes reattach or operator cleanup.
- If the runtime restarts, prior transcript text, pending input text, owner tokens, and cached lease identity are gone. Attach again and receive a fresh owner token — the old token is permanently invalid after a restart.
