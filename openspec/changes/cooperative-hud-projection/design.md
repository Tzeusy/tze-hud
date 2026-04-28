## Context

The existing text-stream portal capability defines a governed content-layer surface for incremental text output, bounded input submission, session identity, status metadata, collapse/expand behavior, and adapter isolation. The live portal exemplar already exercises composer input, bounded transcript rendering, movement, resize, and minimized-icon behavior through resident raw tiles on the primary `HudSession` stream.

The proposed workflow is different from PTY or tmux attachment. A user may already be in a Codex, Claude, opencode, or similar session and then ask that session to project itself to the HUD. Because the runtime must not become a terminal host and because arbitrary already-running processes cannot be reliably hijacked without prior PTY ownership, the attachment model must be cooperative: the active LLM session opts in, uses a skill or MCP server, and exchanges compact projection operations with a long-lived local daemon.

The projection daemon is the durable owner of transport and state. It keeps the HUD connection, portal lease, transcript retention, pending input inbox, lifecycle metadata, and reconnect behavior outside the LLM token context. The LLM session remains the semantic owner: it decides what to publish, when to consume HUD input, and when to detach.

## Goals / Non-Goals

**Goals:**

- Let already-running LLM sessions opt into a HUD text-stream portal without being launched by a wrapper.
- Keep long-lived transcript and inbox state outside token context.
- Provide a provider-neutral contract usable by Codex, Claude, opencode, and future agent CLIs.
- Support bidirectional interaction through bounded HUD input submissions and LLM-controlled acknowledgement.
- Reuse the existing text-stream portal surface, lease governance, attention model, redaction rules, and content-layer movement/collapse behavior.
- Keep the LLM-facing operation set small enough to use as a skill or MCP server without massive token drain.

**Non-Goals:**

- PTY attachment to arbitrary already-running terminal processes.
- Terminal emulation, ANSI screen modeling, cursor-addressed regions, or raw keyboard passthrough.
- Runtime ownership of Codex, Claude, opencode, shell, tmux, or process lifecycle.
- A chrome-layer session tray or shell-owned portal control.
- Automatic capture of all CLI output. Cooperative sessions publish intentional projection updates.
- Full retained transcript mirroring into scene nodes or LLM context.

## Decisions

### 1. Attachment model: cooperative opt-in, not OS/process capture

**Decision:** `/hud-projection` attaches an already-running LLM session by registering a logical projection with a daemon. The agent then cooperates through explicit operations.

**Rationale:** This matches the user's workflow while staying honest about technical boundaries. Without prior PTY ownership, reliable stdin/stdout capture is not generally available. Cooperative attachment still works across providers because it depends on a shared protocol rather than process internals.

**Alternative considered:** Attach to the terminal process after launch. Rejected as brittle, provider- and OS-specific, and contrary to the no-PTY scope decision.

### 2. State ownership: daemon-retained, token-light session contract

**Decision:** The projection daemon stores retained transcript, visible window, pending input, lifecycle, unread state, and HUD lease metadata. The LLM-facing contract exposes compact operations and summaries, not a full transcript feed.

**Rationale:** The projection must survive long-lived sessions without turning the LLM context into a state store. The daemon can reconnect, coalesce, redact, and replay bounded visible windows without requiring the model to keep history in tokens.

**Alternative considered:** Let each LLM carry the portal transcript in prompt state. Rejected because it creates persistent token drain and makes reconnection/history behavior model-dependent.

### 3. Transport split: skill/MCP control inward, resident gRPC outward

**Decision:** The LLM-facing side may be a skill, local CLI, or MCP server with semantic operations. The HUD-facing side uses the existing resident gRPC session and text-stream portal raw-tile path.

**Rationale:** MCP is suitable for low-frequency semantic operations, but the architecture explicitly keeps high-rate deltas off MCP. The daemon is the boundary: it can accept sparse LLM operations and publish efficient state-stream updates to the HUD.

**Alternative considered:** Drive the HUD directly from the LLM through MCP-only zone publishes. Rejected because it bypasses the portal lease/input model and would be unsuitable for ongoing transcript churn.

### 4. Input delivery: inbox with transactional acknowledgement

**Decision:** HUD submissions become ordered pending inbox items in the daemon. The LLM session fetches or is notified of pending items, handles them in its normal reasoning loop, then acknowledges each item with handled/deferred/rejected status.

**Rationale:** This preserves human intent and avoids pretending that HUD text can be injected into an arbitrary already-running model prompt. It also gives the daemon enough information to show pending, delivered, handled, or stale states.

**Alternative considered:** Automatically write HUD submissions into the terminal or current prompt. Rejected because that is PTY input, not cooperative projection.

### 5. Projection identity: provider-neutral session records

**Decision:** Projection sessions use a stable projection ID, provider kind, display name, optional icon/profile hint, repository/workspace hints, lifecycle state, and content classification.

**Rationale:** The HUD needs enough identity for multiple movable icons, but the contract must not become Codex-specific. Provider-specific details belong in optional metadata.

**Alternative considered:** Define separate Codex/Claude/opencode session schemas. Rejected because it would fragment the surface and duplicate governance behavior.

### 6. Rendering: reuse existing portal, no new node type

**Decision:** The first implementation reuses text-stream portal raw-tile composition. Collapsed icons, expanded transcript, composer input, movement, resize, and restore behavior are portal surface concerns, not a new compositor primitive.

**Rationale:** The existing portal capability already covers the right behavior and has live refinement underway. A new runtime node would be premature before projection-specific usage proves repeated raw-tile friction.

**Alternative considered:** Create a dedicated projected-session node. Rejected as premature and likely to churn while the projection contract is still being validated.

## Risks / Trade-offs

- **[Risk] Cooperative projection can miss unreported session output.** → Mitigation: document that v1 is intentional projection, not terminal capture; provide low-friction publish/status helpers and a later optional wrapper/tmux adapter path if needed.
- **[Risk] LLMs may poll too often or include too much transcript in tool calls.** → Mitigation: make operations bounded by default, expose compact pending counts/summaries, and reject oversized payloads.
- **[Risk] Multiple projected sessions can become distracting.** → Mitigation: require ambient attention defaults, no auto-escalation on backlog growth, and privacy classification on identity/status metadata.
- **[Risk] Daemon lifecycle bugs could leave stale portal tiles.** → Mitigation: reuse lease/orphan/cleanup rules, add explicit detach and heartbeat semantics, and validate cleanup in user-test.
- **[Risk] Skill and MCP variants diverge.** → Mitigation: define one normative projection operation schema and treat skills as packaging surfaces for the same contract.

## Migration Plan

No runtime migration is required for the spec phase. Implementation should proceed in layers:

1. Define the projection operation schema and local daemon storage model.
2. Implement a minimal daemon that can create one projected session and publish a portal using existing resident gRPC credentials.
3. Add a skill/MCP wrapper that lets an already-running LLM attach, publish, poll input, acknowledge, and detach.
4. Add live user-test coverage on the Windows HUD for attach → output → HUD input → acknowledgement → collapse/restore → detach cleanup.
5. Broaden to multiple simultaneous sessions and provider-specific icon/profile hints after the single-session flow is stable.

Rollback is operational: stop the daemon, release leases, and remove the skill/MCP server configuration. No persistent runtime schema migration is expected in v1.

## Open Questions

1. Should pending HUD input be pushed to the LLM through a notification/prompt mechanism where available, or should the first contract require explicit polling?
2. Should projection daemon state persist across host restarts, or only across transient HUD/session reconnects?
3. What is the minimum provider metadata needed for useful icons without leaking private project/session details?
4. Should the first skill package live beside `th-hud-publish`, or should projection be a separate installable skill to keep the simple publish path small?
