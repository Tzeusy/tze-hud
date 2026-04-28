## Why

LLM coding sessions can already use terminal, chat, and HUD surfaces, but an already-running session has no durable, low-context way to project its state onto the HUD and accept operator input from that surface. Cooperative HUD projection gives Codex, Claude, opencode, and similar agents a shared opt-in bridge to a text-stream portal without requiring PTY attachment, terminal emulation, or runtime ownership of external process lifecycles.

This is valuable now because the text-stream portal raw-tile pilot already proves the governed surface model, composer input, collapse/restore behavior, and transport-agnostic adapter boundary. The missing contract is the LLM-facing projection layer that keeps long-lived portal state outside token context while allowing the active agent session to remain the semantic owner.

## What Changes

- Introduce a cooperative projection protocol for already-running LLM sessions that opt in through a skill, command, or MCP server.
- Define a projection daemon as the owner of durable session state: HUD connection, portal lease, retained transcript window, pending inbox, lifecycle state, reconnect behavior, and privacy metadata.
- Define an LLM-facing control surface with low-token operations for attaching, publishing output/status, checking pending HUD input, acknowledging input, detaching, and cleaning up.
- Define how HUD-originated input is delivered to the active LLM session as bounded transactional submissions rather than raw terminal keystrokes.
- Define provider-neutral session identity and icon/profile metadata so Codex, Claude, opencode, and future LLM sessions can use the same projection contract.
- Reuse the existing text-stream portal content-layer surface for rendering, movement, expand/collapse, input, redaction, attention, and lease behavior.
- Explicitly exclude PTY attachment, terminal byte capture, process hosting, and automatic capture of arbitrary already-running terminal output.

## Capabilities

### New Capabilities

- `cooperative-hud-projection`: Cooperative opt-in projection protocol for already-running LLM sessions, covering session identity, daemon-retained state, LLM-facing operations, HUD input delivery, lifecycle, governance, and provider neutrality.

### Modified Capabilities

- `text-stream-portals`: Clarify that cooperative LLM-session projection is a concrete non-tmux adapter family that reuses the existing portal surface, bounded input, content-layer, traffic-class, and external-adapter isolation requirements without adding PTY or runtime process lifecycle semantics.

## Impact

- Adds a new OpenSpec capability for the projection daemon and LLM-facing skill/MCP contract.
- Adds delta requirements to `text-stream-portals` to bind cooperative projection to the existing portal semantics.
- Future implementation will likely touch:
  - a new `.claude/skills/hud-projection/` skill and mirrored tool skill surfaces,
  - optional MCP server or local daemon code for projection state and HUD communication,
  - user-test scripts for live Windows HUD projection flows,
  - docs and examples for Codex/Claude/opencode attachment workflows.
- Runtime core should not require new node types, portal-specific RPCs, PTY integration, or process lifecycle management for the first implementation.
