## Context

The current cooperative HUD projection contract is intentionally narrow: an already-running LLM session opts into a daemon-owned projection, receives an owner token, publishes output/status, polls semantic HUD input, and materializes through an existing text-stream portal resident gRPC adapter. That work explicitly avoided PTY capture, process hosting, and runtime ownership of provider lifecycles.

The 2026-05-11 goal asks for the next layer: an external authority that can launch or attach multiple provider-neutral LLM sessions, authenticate them to the local Windows runtime, and give each a governed HUD presence surface. This belongs outside the compositor because it concerns provider process/session orchestration and runtime-facing policy routing, not pixels or frame timing.

## Goals / Non-Goals

Goals:

- Manage multiple provider-neutral LLM session records from one external authority.
- Support both launched and cooperatively attached session origins.
- Route each session to one bounded existing v1 HUD surface: zone, widget, or leased text-stream/raw-tile portal.
- Authenticate to the local Windows HUD runtime using authority-held target metadata while keeping secrets out of logs, audit records, docs, and scene state.
- Preserve runtime sovereignty over capabilities, leases, attention, privacy/redaction, TTLs, revocation, safe mode, and budgets.
- Prove the first slice with three concurrent sessions publishing distinct governed elements and cleaning up predictably.

Non-goals:

- No terminal capture, PTY injection, tmux control, stdin/stdout interception, or raw keystroke passthrough.
- No provider-specific RPC behavior in runtime core.
- No new compositor node type or agent-rendered chrome.
- No WebRTC/media/mobile/glasses/embodied presence.
- No notification-spam escalation model.
- No durable secret or transcript persistence in v1.

## Decisions

### 1. Authority owns session orchestration, runtime owns presentation

The external authority will hold provider-neutral session records and produce runtime-facing surface commands. The runtime still accepts or rejects those commands through existing MCP/gRPC/auth/capability/lease paths.

This keeps provider lifecycle concerns outside `tze_hud_runtime` and lets tests validate routing without requiring real Codex/Claude/opencode binaries.

### 2. Launch is supervised but not captured

For v1, a launched session means the authority records an intended provider command and environment hints and may supervise process identity in a wrapper. It MUST NOT claim transcript capture unless the provider cooperates through the projection operation contract. Attach remains the same cooperative opt-in flow as the archived projection contract.

### 3. Surface routing is declarative

Each managed session declares a desired surface class:

- `zone`: publish bounded semantic content to a named zone via the existing zone publish path.
- `widget`: publish bounded typed parameters to an existing widget instance via the existing widget publish path.
- `portal`: acquire or reuse a lease and materialize a text-stream/raw-tile portal through the existing resident adapter.

The authority can build a route plan and audit it, but runtime policy remains authoritative at execution time.

### 4. Attention and privacy fail closed

Every presence route carries content classification, lifecycle, TTL, and attention intent. Missing classification defaults to private. Attention defaults to ambient/gentle; higher-priority attention requests require explicit classification and rate-limit eligibility. Backlog size alone must not escalate interruptions.

### 5. Three-session demo is the acceptance slice

The first demo uses three provider-neutral sessions:

- status session routed to a status/notification zone;
- progress session routed to a widget instance;
- question/session transcript routed to a leased text-stream portal.

The demo must show concurrent session state, distinct route plans, cleanup/revocation, and reconnect bookkeeping. Live Windows evidence may reuse existing MCP/gRPC scripts and must record any host/runtime blocker as a bead rather than treating a proxy test as live validation.

## Risks / Trade-offs

- Real provider launch behavior can vary. Mitigation: first slice validates provider-neutral launch metadata and cooperative operation flow, not provider-specific CLI automation.
- Widget instances differ across deployments. Mitigation: live user-test must call `list_widgets` and adapt to available instances before publishing.
- Windows host reachability can block live validation. Mitigation: run bounded reachability probes and file a bead with exact blocker evidence if unavailable.
- Multi-surface routing could duplicate runtime policy. Mitigation: route plans are advisory; runtime remains the enforcement point.

## Migration Plan

No runtime migration is required. Add the new authority types and tests to `tze_hud_projection`, then optionally expose the vertical slice through the existing stdio authority if needed. Existing cooperative projection operations remain backward-compatible.
