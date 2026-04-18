# RFC 0013: Text Stream Portals

**Status:** Implemented
**Issue:** hud-t98e
**Date:** 2026-04-10 (implemented 2026-04-17)
**Authors:** tze_hud architecture team
**Depends on:** RFC 0001 (Scene Contract), RFC 0004 (Input Model), RFC 0005 (Session Protocol), RFC 0007 (System Shell), RFC 0008 (Lease Governance), RFC 0009 (Policy Arbitration)

---

## Summary

This RFC defines **text stream portals**: governed on-screen surfaces for low-latency streaming text interaction. A text stream portal is not a terminal emulator, not a chat client embedded in chrome, and not a generic application host. It is a presence surface that lets the runtime display incremental text output, accept bounded text input, and enforce the same sovereignty, lease, privacy, and override rules that govern every other agent-visible surface.

The core boundary is **transport-agnostic text streams**. The runtime deals in input and output text streams plus session metadata. Adapters for tmux, chat transports, LLM sessions, or other text-producing systems live outside the runtime and speak this boundary. The runtime remains sovereign over pixels, focus, input routing, redaction, dismissal, and safe mode.

This RFC also defines a phased boundary:

- **Phase 0 / pilot:** text stream portals are proven using resident raw tiles plus external adapters. No tmux-specific logic enters the runtime. No terminal-emulator node is added.
- **Post-pilot:** if the pattern proves stable and governance requirements remain coherent, the project may define a first-class portal surface or node type. That promotion requires separate approval and does not happen automatically.

**Inline color runs (cross-reference):** `TextMarkdownNode.color_runs` (see RFC 0001 §TextMarkdownNode) enables adapter-side ANSI-to-runs conversion without a terminal-emulator surface. An adapter can strip ANSI escape sequences from `content` and map each color segment to a `TextColorRun` entry, letting the runtime compositor render colored spans natively. This feature is in scope for Phase 0 adapters; terminal emulation (PTY, cursor positioning, scrollback) remains out of scope.

---

## Motivation

tze_hud already names the CLI, the chat transcript, and the generated webpage as incomplete forms of model interaction. One missing middle ground is a live, governed text interaction surface that is fast enough for ongoing conversation but still shaped by the runtime's presence model.

This use case spans more than LLM chat:

- a person interacting with another person through a chat transport,
- a person interacting with an LLM through a resident session,
- an operator monitoring and replying into a live agent workflow,
- a local adapter bridging an existing text environment into a HUD presence surface.

The project should support this class of interaction without drifting into three failure modes:

1. **Terminal-host drift.** The runtime becomes responsible for PTY semantics, ANSI, scrollback, shell lifecycle, or arbitrary process hosting.
2. **Chrome drift.** Agent-specific portal affordances leak into the system shell, violating chrome sovereignty and agent exclusion.
3. **Chat-app drift.** The system's dominant interaction pattern becomes text scrolling rather than governed presence on a live screen.

This RFC resolves those pressures by defining a narrow contract that preserves the runtime's boundaries.

---

## Design Requirements Satisfied

| Requirement | This RFC |
|-------------|----------|
| Runtime sovereignty over pixels and input | Portal surfaces are runtime-rendered and runtime-governed |
| Local-first interaction | Reply/expand/scroll interactions reuse the existing local-feedback contract |
| Lease governance | Portal surfaces remain lease-bound or runtime-managed; no side channel bypass |
| Chrome isolation | Agent portal UI lives in content-layer territory, not the chrome layer |
| Adapter replaceability | Runtime depends on generic text-stream contract, not tmux or any single backend |

---

## 1. Conceptual Model

### 1.1 Definition

A **text stream portal** is a governed surface that presents:

- a stream of output text arriving incrementally over time,
- bounded input text authored by the viewer,
- session identity and status metadata,
- optional adjunct controls such as expand, collapse, interrupt, or acknowledge.

The portal exists to express live presence through text interaction, not to mirror an arbitrary terminal session byte-for-byte.

### 1.2 Non-goals

Text stream portals are explicitly **not**:

- a full terminal emulator,
- a PTY host,
- a shell or process manager,
- a chrome-layer agent tray,
- a generic chat app framework,
- a bypass around lease, privacy, or override controls.

### 1.3 Canonical Use Cases

Canonical uses include:

- resident LLM interaction surfaces,
- operator-facing conversation with external text systems,
- chat-style collaboration portals where the remote peer may be human or model.

The defining property is not who is on the other side of the stream. The defining property is that the runtime is presenting a low-latency text interaction surface under full governance.

---

## 2. Adapter Boundary

### 2.1 Runtime Contract

The runtime-facing contract is transport-agnostic. A portal source must expose:

- **Session identity:** stable session ID, display name, optional peer class, lifecycle state.
- **Output stream:** incremental text output units with ordering semantics and timestamps.
- **Input submission:** bounded user-authored text input submission.
- **Status metadata:** connected, disconnected, backpressured, idle, awaiting reply, etc.
- **Optional control operations:** interrupt, retry, clear unread, acknowledge.

The runtime core MUST NOT depend on:

- tmux window IDs,
- PTY escape sequences,
- terminal resize semantics,
- shell prompts,
- chat-provider-specific message schemas,
- application-specific transport APIs.

The portal contract is semantic, not a new session topology. For resident adapters, all portal-related control, publication, and lease traffic continues to ride the existing primary bidirectional session stream. Phase 0 does not introduce a second long-lived "portal stream" per agent.

If portal output units carry timing metadata for ordering, scheduling, unread windows, or expiry, those fields MUST follow the existing clock-domain rules:

- wall-clock scheduling or expiry uses `_wall_us`,
- monotonic latency or RTT measurement uses `_mono_us`,
- arrival timestamps remain advisory and do not override runtime presentation control.

### 2.2 External Adapters

Adapters live outside the runtime. Examples include:

- a tmux adapter that wraps panes/windows and converts their text I/O into portal stream events,
- a chat-platform adapter that maps remote thread messages into portal stream events,
- an LLM-session adapter that bridges a resident coding/chat agent into the portal contract.

Adapters are replaceable. The runtime contract must remain stable if tmux is replaced by any other text source.

### 2.3 Security Boundary

Adapters are authenticated actors, not privileged extensions of the compositor. Any adapter that can submit input, emit output, or request visibility on the screen must pass through the existing runtime authentication and capability model. No adapter gets implicit authority because it runs locally.

---

## 3. Surface Model

### 3.1 Layer Attachment

Text stream portals are **content-layer** surfaces. They MUST NOT be rendered in the chrome layer unless a future RFC explicitly creates a runtime-owned portal shell concept.

This follows two invariants:

1. agent-specific content must not pollute the system shell,
2. portal identity and transcript state are agent or adapter state, not chrome state.

### 3.2 States

A portal may present at least two states:

- **Collapsed:** summary card showing identity, status, unread/activity state, and affordance to expand.
- **Expanded:** transcript-focused surface showing recent output history plus reply/input affordance.

State transitions remain governed by the runtime's input and focus rules.

### 3.3 Privacy-Preserving Presentation

Portal identity, status, and transcript content are all subject to viewer-context policy. A collapsed portal card MUST NOT be treated as "safe metadata" by default. If the current viewer is not permitted to see the portal's identity or transcript:

- the portal preserves its geometry,
- visible content is replaced with a neutral redaction placeholder,
- transcript previews and activity details are suppressed,
- interactive affordances that would reveal private content are disabled.

### 3.4 Phase-0 Representation

The pilot representation uses resident raw tiles plus existing node types:

- `TextMarkdownNode` for transcript and summary text,
- `SolidColorNode` for card and transcript backgrounds,
- `HitRegionNode` for expand/collapse/reply/interrupt controls,
- optional `StaticImageNode` for identity/iconography.

This phase intentionally avoids a dedicated terminal node, transcript node, or byte-stream renderer.

Phase-0 transcript rendering is a **bounded viewport**, not an unbounded transcript store. Any transcript text materialized into scene nodes MUST remain within existing node-size and per-tile resource budgets. Full retained history, if any, belongs to adapter-side storage or another non-scene persistence layer. The raw tile only carries the currently visible or immediately scrollable window.

---

## 4. Interaction Model

### 4.1 Focus and Activation

Portals reuse RFC 0004 focus and command-input semantics. Expand, collapse, reply, and control affordances are `HitRegionNode` targets with local-first feedback. The runtime acknowledges pointer and command activation locally before any adapter response.

### 4.2 Scroll

Expanded transcript views require local-first scroll semantics. Scroll remains a runtime-owned local state, with adapters notified asynchronously of the resulting offset or viewport state if needed. In phase 0, a portal tile that supports transcript scrolling MUST use the existing runtime scroll contract for scrollable tiles; user scroll input remains authoritative over any adapter-driven attempt to reposition the viewport.

### 4.3 Reply Submission

Viewer-authored input is bounded text, not raw key-by-key terminal passthrough in the pilot phase. Submit/cancel actions are transactional interactions. In phase 0, this is expressed through existing interaction surfaces: hit-region activation, focus, command input, and agent-owned mutation handling. It does **not** imply a new portal-specific transport RPC, a new general-purpose inline text editor, an IME-complete composition surface, or a byte-by-byte terminal input path. A future RFC may define richer editing or composition behavior if needed.

### 4.4 No Terminal Semantics in Phase 0

The pilot phase does not promise:

- VT100/xterm compatibility,
- cursor-addressable text regions,
- alternate-screen handling,
- copy-mode parity,
- terminal mouse-report passthrough,
- IME-complete line editing.

If those semantics become necessary, they require a separate decision and likely a distinct node or surface class.

---

## 5. Timing and Message Classes

Portal traffic uses the existing message taxonomy:

- **Output transcript append/update:** state-stream, reliable and coalescible by viewport/history policy.
- **Typing / live status indicators:** ephemeral realtime, latest-wins.
- **Viewer reply submission:** transactional.
- **Interrupt / cancel controls:** transactional.

Arrival time is not presentation time. Output stream elements may carry timing metadata for ordering, unread calculation, or viewport policy, but the runtime still decides presentation.

State-stream coalescing MUST preserve a coherent transcript window. Intermediate render states may be skipped, but already-committed logical transcript units within the retained on-screen history window MUST NOT be lost merely because updates were coalesced. Coalescing may collapse multiple append operations into a newer complete window snapshot; it must not turn ordered transcript history into "latest line only."

---

## 6. Governance

### 6.1 Lease and Ownership

In the pilot phase, a portal is owned through the same lease and tile model as any other resident surface. The runtime may later define a runtime-managed portal abstraction, but it must still map onto explicit governance rather than bypass it.

### 6.2 Disconnect and Orphan Handling

When the owning resident session disconnects unexpectedly, the portal follows the normal lease/orphan path rather than inventing portal-specific failure semantics:

- the lease transitions to `ORPHANED`,
- the visible portal freezes at its last coherent state or runtime placeholder policy,
- a disconnection or staleness indicator may be shown by the runtime,
- the reconnect grace timer starts,
- successful reconnect reclaims the same portal surface,
- grace expiry removes the tile and frees resources.

External adapters do not define their own orphan timeline. The runtime's lease and failure contracts remain authoritative.

### 6.3 Privacy and Redaction

Portal transcripts are often privacy-sensitive. Redaction, viewer-class filtering, and attention/interruption policy apply exactly as they do for any other tile. A portal does not get special exemption because it is "just text."

### 6.4 Safe Mode, Freeze, Dismiss, and Override

Portal surfaces obey all human overrides:

- dismiss removes the tile,
- safe mode suspends the owning session and lease, replaces visible portal content with runtime-owned placeholders, and captures all input in chrome,
- freeze prevents visible scene mutation without leaking viewer intent; adapters are not explicitly told "the scene is frozen" and may observe only generic queue-pressure or dropped-mutation signals,
- revoke terminates the owning authority regardless of adapter wishes.

This distinction is load-bearing. Safe mode is not revocation. Freeze is not a portal-specific pause API. Both continue to obey the system-shell and lease-governance contracts already defined elsewhere.

### 6.5 Attention

A text stream portal may be ambient, but it must not become an attention weapon. Activity indicators, unread counts, and expansion behavior remain subordinate to the runtime's attention model. The portal is a presence surface, not an engagement funnel.

Typing indicators, unread counts, and transcript churn MUST default to ambient or gentle behavior. A portal must not self-escalate interruption class merely because a stream is active or a backlog is growing.

---

## 7. Phase Boundary

### 7.1 Phase 0: Raw-Tile Pilot

The first implementation phase proves:

- transport-agnostic stream boundary,
- content-layer portal behavior,
- low-latency transcript interaction,
- lease/privacy/override compliance,
- external adapter viability.

It does so without:

- new protobuf message types,
- new scene node types,
- new long-lived per-portal session streams,
- terminal-emulator responsibilities in the runtime,
- chrome-layer portal UI.

### 7.2 Promotion Criteria

Promotion to a first-class runtime portal surface or node type requires evidence that:

- the same layout/behavior pattern recurs across multiple adapters,
- raw-tile expression is creating repeated complexity,
- governance requirements are stable,
- the feature remains subordinate to the broader presence thesis rather than becoming the dominant product interaction mode,
- the system still does not need full terminal semantics.

Without that evidence, the raw-tile pilot remains the correct scope.

---

## 8. Open Questions

1. Should expanded portal input be modeled as bounded one-shot reply submission only, or does the product need an in-surface editable draft model?
2. Should transcript virtualization/windowing remain purely adapter-side in the pilot, or should the runtime own transcript history policies?
3. Which portal states deserve dedicated runtime badges versus ordinary tile-local affordances?
4. If multiple portal sources coexist for the same agent, is that one portal with multiple streams or multiple portals?

---

## 9. Rejected Alternatives

### 9.1 Tmux-Aware Runtime

Rejected because it hardcodes one adapter family into the core product surface and pressures the runtime toward PTY/process management.

### 9.2 Chrome Tray for Agent Portals

Rejected because the current shell contract excludes agent-specific chrome content and requires chrome rendering independence from agent state.

### 9.3 Terminal Emulator as First Step

Rejected because it solves the wrong problem first. Terminal semantics are much larger than the governed text interaction problem and would distort the runtime boundary prematurely.
